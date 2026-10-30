# ADR-0108 — Cohérence de l'époque après une compaction en échec

- **Statut** : Accepté (accepté par anticipation par le propriétaire le 2026-10-29, confirmé par le propriétaire le 2026-10-30)
- **Date** : 2026-10-29
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, tâche ROUTES-DOCUMENTS-ET-LOTS (phase P6), sur demande de l'orchestrateur et les verdicts de `@architecte` du 2026-10-29
- **Remplace** : — (ne remplace aucune décision : précise la reprise d'`docs/02` et le protocole de compaction, sans réécrire ADR-0016 ni ADR-0065)

## Contexte

`POST /v1/indexes/{index}/compact` rend la compaction atteignable depuis l'API (`docs/phases/P6-serveur-et-api.md:91`). Or un serveur ne rend pas le magasin à chaque commande : l'écrivain d'un index le garde en mémoire entre deux jobs, et les jobs d'un même index s'exécutent dans l'ordre de sa file (ADR-0103, décision 8 ; ADR-0105, décision 4). Ce changement de contexte rend visible un défaut écrit dans `docs/suivi.md`, section « Idées hors périmètre » : l'entrée du 2026-10-25 (`COMPACTER-INDEX`, tour 2) sur `adopt_epoch`, et l'entrée du 2026-10-29 (`FILE-D-ECRITURE-ET-JOBS`, point 10b), qui la déclare atteignable dès que `POST /compact` existe.

### Le protocole et sa fenêtre

La compaction suit six étapes (`docs/02-architecture.md:189-199`). L'étape 5 écrit `CURRENT_EPOCH ← E+1`, « c'est le point de bascule » ; l'étape 6 publie l'instantané, relance l'écrivain et récupère l'époque E. Entre les deux, `compact` appelle `adopt_epoch` (`crates/crible-store/src/compact.rs:346-351`), qui ouvre l'état de référence de E+1, construit et publie son instantané, puis fait basculer le magasin.

Si `adopt_epoch` échoue après `write_current_epoch`, deux vérités coexistent :

- sur disque, `CURRENT_EPOCH` nomme **E+1**, époque complète, vérifiée et portant la génération G+1 ;
- dans le magasin en mémoire, l'époque reste **E**, avec `meta.epoch = E` et la génération G.

`docs/02:88` dit déjà qu'`meta.epoch` garde une copie de `CURRENT_EPOCH`, « vérifiée à l'ouverture : un désaccord est une erreur interne ». Le désaccord n'est donc pas un état inconnu : c'est un état que l'ouverture refuse. Ce que rien ne vérifiait, c'est un désaccord **apparu après** l'ouverture.

### Pourquoi c'est grave

`GenerationStore::commit` (`crates/crible-store/src/generation.rs:848`) écrit la génération suivante dans `records.sqlite` de l'époque que le magasin tient, c'est-à-dire dans **E**. Ces générations ne sont vues par aucun lecteur après une réouverture, qui ouvre E+1 : elles sont **perdues**, alors que leur transaction a été commitée et que le client a reçu une génération. Un patch, une attache ou un lot en ligne suivant la compaction en échec produirait exactement cela.

`compact` porte déjà une garde, ajoutée avec la tâche `COMPACTER-INDEX` (`crates/crible-store/src/compact.rs:238-250`) : il lit `CURRENT_EPOCH` et refuse en `internal` si le pointeur ne nomme pas l'époque du magasin, parce qu'une seconde compaction calculerait E+1 — l'époque que le pointeur nomme — et pourrait la détruire. Cette garde protège la compaction, pas les autres commits.

En CLI, le défaut est latent : un processus par commande, et la réouverture suivante repart de `CURRENT_EPOCH`. Dans le serveur, il devient atteignable par une requête, sans redémarrage.

### Ce que la reprise fait, et ce qu'elle ne fait pas

La reprise (`docs/02:139-159`) lit `CURRENT_EPOCH`, ouvre l'état de référence de l'époque nommée, supprime les `segments/tmp-*` et les segments absents de la table `segments`, supprime les `tmp-*` du dossier de l'index et de `manifests/`, reconstruit le manifeste manquant, reconstruit les dérivés et publie l'instantané. Une **réouverture** du magasin depuis le disque est donc exactement le remède : elle repart de E+1 et publie l'état commité.

La reprise ne nettoie **pas** `epochs/tmp-*` : `docs/02:143` l'exclut explicitement, un dossier de transit restant « inerte » et se supprimant à la main ou par l'opération suivante qui vise ce numéro d'époque. La demande d'origine citait ce nettoyage ; le présent ADR ne le décide pas, et la décision 2 n'en a pas besoin : dans le cas visé, E+1 est complète et vérifiée avant la bascule, et aucun dossier de transit n'est en jeu.

## Décision

**Décision 1 — garde avant tout commit.** Avant tout commit — attache, lot en ligne, patch, compaction suivante —, `GenerationStore` vérifie que `CURRENT_EPOCH` nomme **son** époque. S'il en diffère, il refuse en **`internal`**, sans rien écrire, et la cause est journalisée : le pointeur, l'époque du magasin et l'index sont nommés. Le coût est **une lecture de fichier par commit** ; il reste très inférieur à celui du commit lui-même, mesuré de 106,4 ms à 641,3 ms selon le nombre de passages (MES-15, § 4).

La garde est celle que `compact` porte déjà avant sa première écriture (`crates/crible-store/src/compact.rs:238-250`), étendue à tous les chemins d'écriture. Elle ne remplace pas le refus d'écrire dans une époque que le magasin ne tient pas : elle s'ajoute aux contrôles de génération existants.

**Décision 2 — remise en état après une compaction en échec.** Quand un job `compact` échoue, **l'écrivain de l'index abandonne son magasin en mémoire et rouvre l'index depuis le disque** par la reprise (`docs/02:139-159` : époque nommée par `CURRENT_EPOCH`, nettoyage des `segments/tmp-*`, des `tmp-*` du dossier de l'index et des segments orphelins, reconstruction du manifeste), **puis republie l'instantané avant le job suivant**. Le remède vaut pour tout échec de compaction, pas seulement pour celui d'`adopt_epoch` : un échec avant la bascule laisse le pointeur sur E et la réouverture rend le même état.

Si la réouverture échoue, l'index **reste servi en lecture** sur son dernier instantané et les **jobs suivants de cet index échouent en `internal`** jusqu'au redémarrage. Le journal porte une ligne `error` **une fois**, pas une par job refusé.

**Décision 3 — la CLI ne change pas.** `crible compact` garde son comportement : un processus par commande, et la réouverture est implicite pour la commande suivante. La garde de la décision 1 vaut pour elle aussi, sur le même chemin de code.

Ce qui n'est pas décidé ici : le délai de reprise d'un index après un échec de compaction, qui se mesure comme toute ouverture ; la conduite à tenir si l'époque E+1 nommée par le pointeur est elle-même illisible à la réouverture — c'est le cas d'un index cassé, déjà couvert par le refus d'ouverture et non par le présent ADR ; le nettoyage de `epochs/tmp-*`, laissé à `docs/02:143` ; la récupération de l'époque E remplacée, qui reste sur disque jusqu'à la compaction suivante.

## Conséquences

- **Positives** :
  - une génération commitée dans l'époque que le pointeur ne nomme plus devient impossible : le seul chemin qui écrivait là est fermé, et le refus est explicite plutôt qu'une perte silencieuse ;
  - la même garde couvre les quatre chemins d'écriture d'un index, sans mécanisme propre à la compaction ;
  - le serveur se remet seul d'une compaction en échec, sans redémarrage : la réouverture est le chemin déjà éprouvé par la reprise et par la CLI ;
  - après une réouverture, l'état servi est celui du pointeur, donc l'époque complète E+1 : le travail de la compaction n'est pas jeté ;
  - un index qui ne peut pas rouvrir reste **lisible** : la panne coûte les écritures, pas les recherches (**INV-02**).
- **Négatives / coûts** :
  - une lecture de fichier et une comparaison à chaque commit, dont le coût doit rester marginal devant le commit mesuré par MES-15 (106,4 ms à 641,3 ms selon le nombre de passages) ;
  - une compaction en échec paie une réouverture complète — reconstruction des dérivés, publication d'un instantané — avant de rendre le service d'écriture ;
  - si la réouverture échoue, l'écriture de l'index est indisponible jusqu'au redémarrage, et le client reçoit `internal` sur des jobs qu'un redémarrage aurait réparés ;
  - l'époque E remplacée reste sur disque jusqu'à la compaction suivante, qui la récupère : le disque porte deux époques entre-temps ;
  - la ligne `error` unique doit être distinguée d'une erreur de job ordinaire : un exploitant qui ne lit que les jobs ne voit que `internal`.

- **À surveiller** :
  - un test qui provoque l'échec d'`adopt_epoch` après la bascule, puis vérifie que le job `compact` est `failed`, que le job suivant (attache ou patch) réussit sur E+1, et qu'aucune génération n'a été écrite dans E ;
  - un test unitaire de la garde : un magasin dont `CURRENT_EPOCH` nomme une autre époque refuse `commit` en `internal`, n'écrit aucune ligne et journalise le pointeur et l'époque ;
  - un test de réouverture : après l'échec, l'instantané publié est celui de E+1, et les lectures qui le tenaient avant ont fini sur le leur ;
  - un test de réouverture impossible : l'index sert encore en lecture sur son dernier instantané, les jobs suivants rendent `internal`, et le journal ne porte qu'une ligne `error` ;
  - un test CLI qui déplace `CURRENT_EPOCH` sous un magasin ouvert et vérifie le refus en `internal` ;
  - la durée d'une réouverture après compaction en échec sur un index de 8,58 M passages : ouverture, reconstruction des dérivés et publication, à comparer au temps de démarrage d'ADR-0047.

- **Documents mis à jour par cet ADR** :

| Document | Modification |
|---|---|
| `docs/02-architecture.md:161-172` | « Concurrence » : un bullet sur la garde `CURRENT_EPOCH` avant tout commit |
| `docs/02-architecture.md:189-199` | cycle de la compaction : une phrase sur l'échec après la bascule et la réouverture par l'écrivain |
| `docs/adr/README.md` | ligne d'index de cet ADR |

`docs/04-donnees-et-ingestion.md` n'est pas modifié : sa section « Compaction » n'existe pas, et les échecs de compaction n'y sont pas décrits (`:318-367` traite l'export, l'instantané et la restauration). `docs/adr/0016` ni `docs/adr/0065` ne sont modifiés : la garde renforce le point de commit unique sans le déplacer, et la réouverture ne change ni le contenu ni le protocole de restauration.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Marquer l'index indisponible sans réouverture | Le serveur continuerait à servir des lectures sur E et refuserait les écritures jusqu'au redémarrage : une compaction en échec arrêterait l'ingestion d'un index sain sur disque, alors que la reprise sait repartir de E+1. |
| Rejouer la compaction | Elle partirait d'un magasin sur E alors que le pointeur nomme E+1 : c'est l'état que la garde existante de `compact` refuse, et une seconde compaction pourrait détruire l'époque que le pointeur nomme. Le travail de E+1 est déjà fait. |
| La garde seule, sans réouverture | Sûre, mais l'index ne pourrait plus écrire avant un redémarrage : la garde empêche la perte sans rendre le service, et `POST /compact` deviendrait une route qui casse un index jusqu'au prochain redémarrage. |
| Réécrire `CURRENT_EPOCH` vers E pour revenir en arrière | Ajoute une écriture sur le chemin d'erreur, jette une époque complète et vérifiée, et laisse E+1 à supprimer ; si la réécriture échoue, l'état est pire que le désaccord détecté. La génération G+1 de E+1 est commitée : la garder est cohérent. |
| Rouvrir le magasin sans republier l'instantané | Les lecteurs garderaient l'instantané de E alors que les commits iraient dans E+1 : passages et générations mélangés, contraire à **INV-02** et **INV-03**. La publication avant le job suivant fait partie du remède. |
| Vérifier `CURRENT_EPOCH` à chaque requête de lecture | Une lecture tient son `Arc<Snapshot>` et ne commit rien : la garde ne la protège de rien, et une lecture de fichier par requête coûterait sans contrepartie. |
| Laisser `compact` seul porter la garde | L'attache, le lot en ligne et le patch écriraient dans E après l'échec : c'est précisément la perte de générations que l'ADR ferme. |

## Références

- `docs/suivi.md`, « Idées hors périmètre » : entrée du 2026-10-25 (`COMPACTER-INDEX`, tour 2) et entrée du 2026-10-29 (`FILE-D-ECRITURE-ET-JOBS`, point 10b).
- `docs/02-architecture.md:88` (`CURRENT_EPOCH`, copie dans `meta.epoch` vérifiée à l'ouverture), `:115-137` (générations, point de commit), `:139-159` (reprise au démarrage), `:161-172` (concurrence), `:174-203` (compaction, six étapes).
- `docs/mesures/MES-15-cout-commit-generation.md` §4 — coût d'un commit, de 106,4 ms à 641,3 ms selon le nombre de passages.
- `docs/phases/P6-serveur-et-api.md:84-97` — fiche `ROUTES-DOCUMENTS-ET-LOTS`, livrable `POST /compact`.
- `crates/crible-store/src/generation.rs:848` (`commit`), `:1054` (`adopt_epoch`), `:2005` (`write_current_epoch`).
- `crates/crible-store/src/compact.rs:238-250` (garde de cohérence avant compaction), `:346-351` (bascule puis `adopt_epoch`).
- `docs/adr/README.md` — index des décisions.
- ADR-0016 (point de commit SQLite), ADR-0017 (la compaction suspend les écritures), ADR-0047 (ordre des remèdes au temps de démarrage), ADR-0065 (restauration hors ligne), ADR-0099 (démarrage, reprise), ADR-0103 (file d'écriture et états des jobs), ADR-0105 (l'écrivain exécute les jobs), ADR-0107 (`POST /compact`, `exports` et `snapshots`).
