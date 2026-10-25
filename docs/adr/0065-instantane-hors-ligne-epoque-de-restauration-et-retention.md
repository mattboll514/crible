# ADR-0065 — Instantané hors ligne, époque de restauration, cible et rétention

- **Statut** : Accepté (décision du propriétaire le 2026-10-25 ; acceptation anticipée du propriétaire, à confirmer au point de fin de l'enchaînement de P3)
- **Date** : 2026-10-25 (proposition et acceptation)
- **Décideurs** : propriétaire
- **Décision** : accepté le 2026-10-25 par le propriétaire, tel que proposé, lors d'une acceptation anticipée de sa part, à confirmer au point de fin de l'enchaînement de P3. Les six décisions sont retenues telles quelles : `crible snapshot` hors ligne en P3 (1), restauration installée comme époque E+1, génération laissée à G (2), index cible créé s'il est absent et contrôlé par son `index.json` s'il existe, retour arrière permis (3), époque remplacée conservée sur disque (4), protocole d'écriture de l'époque inchangé et `epochs/tmp-*` laissés hors du nettoyage de la reprise (5), forme des commandes, rapport et codes d'erreur (6). Les modifications à appliquer sont celles de la section « Documents à modifier après acceptation ».
- **Proposé par** : agent greffier, sur demande de l'orchestrateur, pour la tâche `CREER-ET-RESTAURER-INSTANTANE` (phase P3), à la suite d'une `QUESTION-CONCEPTION` de `@dev` avant implémentation et des verdicts de `@architecte` du 2026-10-25
- **Remplace** : —

## Contexte

### Ce que la fiche et la conception demandent

La fiche `CREER-ET-RESTAURER-INSTANTANE` (`docs/phases/P3-cycle-de-vie.md:121-130`) demande deux commandes :

- `crible snapshot` : « l'écrivain est suspendu pendant la copie » ; on fait un `VACUUM INTO`, puis on copie les segments, `index.json`, le manifeste et le dossier lexical s'il existe ;
- `crible restore`, **hors ligne** : « installe l'instantané comme nouvelle époque et met à jour `CURRENT_EPOCH` ».

Ses critères sont « une restauration redonne la génération et des résultats identiques » et « un instantané incomplet est refusé ».

`docs/04-donnees-et-ingestion.md:298-304` reprend ces deux points : la restauration est « CLI : installé comme nouvelle époque, puis `CURRENT_EPOCH` », et, pendant l'opération, « l'écrivain est suspendu le temps de la copie ». L'avantage annoncé de l'instantané est une « restauration rapide, sans reconstruction ».

### Les précédents

- **ADR-0017** : en v1, la compaction suspend l'écrivain en tenant le `&mut GenerationStore` ; les écritures attendent dans la file, au-delà de sa capacité le serveur répond `503`, et **les lectures continuent** sur l'ancienne époque. La compaction de la CLI fonctionne déjà ainsi (`crates/crible-store/src/compact.rs`).
- **`crible patch`** sur un index servi est une **règle d'exploitation** : `busy_timeout` n'assure que la vivacité (ADR-0038), pas l'exclusion entre deux processus (`docs/04:282`). C'est le précédent direct d'une commande CLI qui écrit dans l'index d'un serveur.
- **L'export**, lui, est en ligne (ADR-0061) : il ne fait que lire un `Snapshot` et une transaction de lecture, sans bloquer l'écrivain. Un instantané n'est pas dans ce cas : il fige une époque entière, SQLite comprise, pour la copier.
- **La question du verrou d'écrivain inter-processus** est déjà ouverte et renvoyée à P6 (`docs/suivi.md:341`, « Idées hors périmètre »). Cet ADR ne la tranche pas.

### Ce que rien ne tranche

`@architecte` a rendu, le 2026-10-25, les verdicts suivants.

| Point | Sujet | Verdict |
|---|---|---|
| B3 | Instantané en ligne ou hors ligne en P3 | **ÉCART** |
| B4 | Époque et génération d'une restauration | **NON-COUVERT** |
| B5 | Index cible : absent, existant, à une génération supérieure | **NON-COUVERT** |
| B6 | Sort de l'époque remplacée | **ÉCART au regard d'INV-14** |
| C3 | Protocole d'écriture de l'époque, et résidus `epochs/tmp-*` | **NON-COUVERT** |
| — | Forme des commandes, rapport et codes d'erreur | non tranché |

### Les faits qui pèsent

- **Rien n'écrit que l'époque et la génération avancent ensemble.** La compaction incrémente les deux (`docs/02:179`) ; un patch crée une génération sans changer d'époque (ADR-0060). Le couple (époque, génération) n'est donc pas monotone dans ses deux composantes, et rien n'interdit qu'une opération change l'une sans l'autre. C'est ce qui laisse B4 sans réponse.
- **`meta.epoch` doit valoir `CURRENT_EPOCH`** (`docs/02:84`) : un désaccord est une erreur interne. Une restauration doit donc écrire les deux, et l'époque de l'instantané ne peut pas être reprise telle quelle si l'index cible porte déjà ce numéro.
- **La compaction écrit dans `epochs/tmp-<E+1>/`**, fsynce, renomme, puis bascule `CURRENT_EPOCH` en dernier (`compact.rs:285-312` ; `docs/02:173-179`). Elle supprime au passage son propre dossier de transit et une époque E+1 abandonnée avant d'écrire (`compact.rs:295-298`).
- **La reprise ne nettoie pas `epochs/`.** `docs/02:132-133` supprime `segments/tmp-*`, les `tmp-*` de la racine de l'index et ceux de `manifests/` ; un `epochs/tmp-*` laissé par une compaction interrompue n'est pas dans cette liste, et `reclaim_epochs` l'ignore explicitement (`compact.rs:201-209`).
- **`reclaim_epochs` supprime toute époque que `CURRENT_EPOCH` ne nomme pas et qu'aucun instantané en mémoire ne tient** (`compact.rs:170-190`). Après une bascule de restauration, l'époque remplacée est exactement dans ce cas.
- **INV-14** (`docs/08:22`) : « Rien de ce qui a coûté du GPU n'est détruit sans copie valide », avec pour exemple « la compaction garde l'époque E jusqu'à la bascule ». Une fois qu'une écriture a suivi la prise, l'époque remplacée contient des passages qu'aucune copie ne porte.
- **`manifests/<G>.json` porte `epoch` et `generation`** (`docs/02:85`) : recopié tel quel, il nommerait l'époque de la source.
- **Une restauration n'encode rien** : aucune recette, aucun modèle. L'encodeur est chargé paresseusement au premier usage (ADR-0051), et `docs/04:303` annonce justement une restauration « sans reconstruction ».
- **Les iid sont renumérotés à chaque compaction** (`docs/02:52`), pas à chaque copie : une époque copiée en entier garde ses iid, qui restent denses et cohérents avec ses segments et sa base.

## Décisions

**Les six décisions ci-dessous sont acceptées telles quelles : instantané hors ligne en P3, restauration en époque E+1 à génération inchangée, index cible créé ou contrôlé, époque remplacée conservée, protocole d'écriture de l'époque inchangé, forme des commandes, rapport et codes d'erreur.**

Les six points ont été soumis au propriétaire, chacun avec la recommandation de `@architecte`.

### 1. En P3, `crible snapshot` est hors ligne (B3, écart)

**Proposition.** En P3, `crible snapshot` est hors ligne, comme `crible restore` : l'opérateur arrête le serveur. Le cœur de la commande prend `&mut GenerationStore`, ce qui suspend l'écrivain au sens d'ADR-0017 — aucune écriture ne peut être commitée pendant la copie — et la future route `POST /v1/indexes/{index}/snapshots` (`docs/05-api.md:66`, P6, nature de job `snapshot` en `:251`) réutilisera la même fonction.

**Pourquoi.** Un processus ne peut pas suspendre l'écrivain d'un autre processus : `&mut` n'a de sens qu'à l'intérieur d'une instance. La CLI ne peut donc pas tenir la promesse « l'écrivain est suspendu le temps de la copie » face à un serveur en marche, et le projet a déjà tranché ce cas pour `crible patch` (`docs/04:282`). La phrase de `docs/04:304` reste vraie pour le serveur, et se lit « hors ligne » pour la CLI ; l'ADR le dit explicitement au lieu de laisser deux lectures.

**Portée de la suspension.** Pendant la copie, les écritures sont retenues et les lectures continuent (INV-02, ADR-0017). En CLI, il n'y a ni file ni lectures concurrentes : la règle d'exploitation est d'arrêter le serveur. Aucun verrou inter-processus n'est ajouté.

### 2. La restauration installe l'instantané comme époque E+1, la génération reste G (B4)

**Proposition.** La restauration installe l'instantané comme **époque E+1** de l'index cible, avec `next_epoch` comme la compaction. Elle réécrit `meta.epoch` dans la base copiée et `manifests/<G>.json` avec cette époque, puis bascule `CURRENT_EPOCH`. Elle **laisse la génération à G** : une restauration n'incrémente pas la génération, ce qui satisfait le critère « redonne la génération ».

**Précisions.** L'époque est copiée en entier, donc les iid de l'instantané sont conservés et restent denses : aucun renumérotage, et les résultats sont identiques sans reconstruction. Les `segment.json` ne sont pas réécrits — leur `generation` et leur `batch_id` disent la provenance du segment, comme pour une compaction (`compact.rs`, section « Identifiers of a compacted epoch »). Seuls `meta.epoch`, le manifeste de G et `CURRENT_EPOCH` changent.

**Cas d'un index cible absent.** Il n'y a pas d'époque E à dépasser : l'index est créé avec l'époque initiale d'une création (`INITIAL_EPOCH = 1`, `crates/crible/src/commands.rs:67`) et la génération G de l'instantané. La règle « E+1 » ne vaut que pour un index qui a déjà une époque.

**Pourquoi ne pas garder E_source.** Deux arbres différents porteraient le même numéro d'époque dans un même index — `epochs/<E_source>/` peut déjà exister chez la cible, entre autres époques conservées — et ni `CURRENT_EPOCH` ni `meta.epoch` ne permettraient de les distinguer. Le numéro d'époque est local à l'index ; il n'a pas à suivre la source.

### 3. Index cible : créé s'il est absent, contrôlé s'il existe, retour arrière permis (B5)

**Proposition.** Le nom de l'index cible est celui de `snapshot.json` : la commande n'a pas d'option `--index`.

- **Si l'index n'existe pas**, il est créé depuis l'`index.json` de l'instantané.
- **S'il existe**, son `index.json` doit être **identique** ; sinon la restauration est refusée en `invalid_request`, avec un message qui nomme le réglage divergent.
- **S'il existe à une génération supérieure à G**, la restauration est **permise** : c'est le retour arrière voulu. L'époque remplacée est conservée (décision 4), donc rien n'est détruit, et le rapport nomme l'époque et la génération remplacées.

Rien ne compare les générations : une cible à une génération inférieure à G suit le même chemin, et seul le rapport diffère.

**Pourquoi.** La comparaison d'`index.json` est ce qui garantit que l'époque restaurée est interprétée avec le schéma et les réglages sous lesquels elle a été écrite : les structures de filtrage se construisent depuis le schéma de l'`index.json` (`GenerationStore::create`), et un réglage réglable divergent — seuil exact, N — changerait le comportement des requêtes sans que rien ne le dise. Refuser en nommant le réglage vaut mieux que restaurer un index qui ne se comporte plus comme sa copie.

**Conséquence assumée.** Un index retuné après la prise doit être remis dans sa configuration d'origine, ou restauré sous un autre nom d'index, avant de pouvoir recevoir l'instantané.

### 4. L'époque remplacée n'est pas supprimée (B6, écart au regard d'INV-14)

**Proposition.** L'époque remplacée reste sur disque. Elle est nommée dans le rapport de la commande, et se supprime **à la main**, comme les résidus `.tmp-*` de l'export.

**Pourquoi.** Dès qu'une écriture a suivi la prise de l'instantané, l'époque remplacée contient des passages qui ne sont dans aucune copie valide : la détruire violerait INV-14. Or la restauration ne peut pas prouver qu'aucune écriture n'a suivi sans une règle de plus, et une copie peut de surcroît vivre sur un autre volume (`exchange/` est un dossier monté, `docs/02:77`).

**Conséquence assumée.** Chaque restauration laisse une époque de plus sur disque ; l'opérateur la supprime quand il est sûr de n'en plus avoir besoin. Le disque croît silencieusement si personne ne le fait.

### 5. Protocole d'écriture de l'époque et résidus `epochs/tmp-*` (C3)

**Proposition.** La compaction suit déjà le protocole voulu et **rien n'y change** : copie dans `epochs/tmp-<N>/`, fsync, renommage vers `epochs/<N>/`, puis `CURRENT_EPOCH` en dernier. Une panne avant la bascule laisse l'époque courante intacte et utilisable. La restauration suit le même protocole ; elle supprime, avant d'écrire, un dossier de transit ou une époque E+1 qu'un résidu occuperait, comme la compaction le fait (`compact.rs:295-298`).

Les résidus `epochs/tmp-*` ne sont pas nettoyés par la reprise : `docs/02:132-133` ne couvre que les `tmp-*` de la racine de l'index et de `manifests/`. Aucun nettoyage nouveau n'est codé ; un résidu du même numéro est de plus supprimé par l'opération suivante qui vise ce numéro, sinon il se supprime à la main. Le point était déjà ouvert pour la compaction, il le reste.

**Pourquoi.** Ajouter `epochs/` au nettoyage de la reprise changerait le protocole de reprise pour un résidu inerte, alors que `TESTER-REPRISE-APRES-PANNE` fige ce protocole. Un dossier sans `CURRENT_EPOCH` est déjà refusé par l'ouverture d'un index neuf (`crates/crible-store/src/generation.rs:156-197`) : ce comportement sera **prouvé par un test**, sans règle nouvelle.

### 6. Forme des commandes, rapport et codes d'erreur

**Proposition.**

```
crible snapshot --data <dir> --exchange <dir> --index <nom>
crible restore  --data <dir> --snapshot <dir>
```

Rapport de `crible snapshot` : destination, époque E, génération G, nombre de documents, nombre de passages, octets copiés.
Rapport de `crible restore` : nom de l'index, époque remplacée (ou « aucune »), nouvelle époque, génération, nombre de documents, nombre de passages.

Ni l'une ni l'autre ne prend `--models` : aucune n'encode. Codes : `invalid_request` (400) pour un instantané absent, incomplet ou altéré, une destination occupée, ou une configuration divergente ; `internal` (500) pour une panne disque.

**Pourquoi.** Les deux commandes sont des opérations de fichier sur une racine de données ; le modèle n'entre ni dans la copie ni dans la restauration, et l'exiger ferait croire à une recette qui n'a pas lieu. Les codes reprennent la table de `docs/05:31-44` : `invalid_request` pour ce que Crible ne peut pas exécuter, `internal` pour ce qui n'est la faute de personne.

## Conséquences

- **Positives** :
  - aucune forme de verrou inter-processus n'est inventée : la commande a la même forme que `crible patch` et `crible compact`, et le serveur réutilisera le même cœur en P6 ;
  - la restauration ne touche à rien avant d'avoir contrôlé l'instantané et la configuration : une panne avant la bascule laisse l'index cible intact (INV-03, INV-04) ;
  - aucune recette, aucun modèle, aucun ré-encodage : la restauration est rapide et rend des résultats identiques, génération comprise ;
  - l'époque remplacée restant sur disque, INV-14 est tenu et un retour arrière manuel reste possible ;
  - l'époque restaurée étant un dossier neuf, elle n'a pas de `cache/` : aucun cache estampillé d'une génération antérieure ne peut être réutilisé à tort.
- **Négatives / coûts** :
  - **espace disque** : pendant une restauration, l'époque remplacée et l'époque installée coexistent, en plus de l'instantané ; la compaction demande déjà deux fois l'espace (`docs/02:181`) ;
  - **disponibilité** : une prise d'instantané suspend l'écriture pendant toute la copie ; à l'échelle de Legifacile, l'époque pèse plus de 13 Go de vecteurs (`docs/02:195`). La durée n'est pas mesurée, et cet ADR n'avance aucun chiffre ;
  - **règle d'exploitation répétée** : la CLI ne peut pas être pointée sur un index servi ; rien ne le détecte, et ADR-0038 n'exclut pas deux écrivains ;
  - **nettoyage manuel** : époques remplacées et `epochs/tmp-*` s'accumulent si personne ne les supprime ;
  - **couple (époque, génération) non monotone** : une restauration change l'époque sans changer la génération. Tout code ou outil qui suppose que les deux avancent ensemble doit être relu ;
  - **historique `batches`** : l'époque restaurée porte l'historique de l'instantané, donc les lots attachés entre G et la génération remplacée disparaissent de l'état vivant — ils restent dans l'époque remplacée, sur disque. INV-10 (« même lot, même manifeste : rien ne se fait ») ne couvre plus ces lots : les rattacher les appliquerait de nouveau au lieu de ne rien faire ;
  - une cause de plus pour `invalid_request`, et une ligne de plus dans la table de `docs/05`.
- **À surveiller** :
  - **la durée** des deux commandes, non mesurée : avant la route de P6 et avant la première prise à l'échelle de Legifacile, une mesure est nécessaire ;
  - **la perte de l'historique après G** : si le retour arrière devient courant, il faudra décider si l'époque remplacée doit rester consultable, ou si son historique doit être rejoué ;
  - **un second processus** vivant sur la même racine pendant un instantané CLI : rien ne l'empêche, et le résultat serait une copie incohérente. La règle écrite dans `docs/04` est la seule garde ;
  - **la compatibilité de format** entre l'instantané et le binaire n'est pas tranchée ici : une époque d'un autre `format_version` échoue à l'ouverture, après installation. Si le cas se présente, il demandera un ADR ;
  - **le disque** : deux époques et les résidus non supprimés doivent rester visibles dans les rapports, sans quoi la croissance sera découverte trop tard.

## Alternatives écartées

| Décision | Alternative | Pourquoi écartée |
|---|---|---|
| 1 | (b) Un verrou d'écrivain inter-processus | La question est déjà ouverte et renvoyée à P6 (`docs/suivi.md:341`). Un fichier de verrou serait un format sur disque de plus, avec sa reprise et ses cas de verrou périmé, pour une commande de sauvegarde |
| 1 | (c) `BEGIN EXCLUSIVE` sur SQLite | Ne suspend pas l'écrivain d'un autre processus, et entre en conflit avec une connexion WAL ouverte : il bloquerait ou échouerait au lieu de garantir une copie cohérente |
| 2 | Garder `E_source` comme numéro d'époque | Deux arbres différents porteraient le même numéro dans un même index ; `epochs/<E_source>/` peut déjà exister chez la cible, et `CURRENT_EPOCH` avec `meta.epoch` ne les distingueraient pas |
| 3 | Restaurer dans un index dont l'`index.json` diffère | L'époque serait interprétée avec un autre schéma ou d'autres réglages que ceux sous lesquels elle a été écrite ; le comportement des requêtes changerait en silence |
| 4 | `reclaim_epochs` après la bascule | Supprime toute époque que le pointeur ne nomme pas et qu'aucun instantané ne tient : c'est exactement l'époque remplacée, qui contient des passages absents de toute copie dès qu'une écriture a suivi la prise. Violation d'INV-14 |
| 5 | Nettoyer `epochs/tmp-*` à la reprise | Étend le protocole de reprise, que `TESTER-REPRISE-APRES-PANNE` fige, pour un résidu inerte ; le point est déjà ouvert pour la compaction et reste hors de P3 |
| 6 | Ajouter `--models` aux commandes | Aucun encodage n'a lieu : ni la copie, ni la restauration ne chargent le modèle (ADR-0051). L'option ferait croire à une recette qui n'existe pas |
| 6 | Un code d'erreur nouveau pour un instantané altéré | La table de `docs/05:31-44` est volontairement courte ; `invalid_request` dit déjà « Crible ne peut pas exécuter cette demande », et `internal` est réservé à la panne inattendue |

## Documents à modifier après acceptation

| Document | Modification, selon les décisions retenues |
|---|---|
| `docs/04-donnees-et-ingestion.md` | section « Export, instantanés, restauration » : instantané hors ligne en P3 (1), époque E+1 et génération inchangée (2), cible et comparaison d'`index.json` (3), époque remplacée conservée (4), forme des commandes, rapport et codes (6) |
| `docs/02-architecture.md` | restauration : époque installée, `CURRENT_EPOCH` en dernier, génération inchangée (1, 2, 5) ; reprise : confirmer que les `epochs/tmp-*` restent hors de son périmètre (5) |
| `docs/05-api.md` | route `POST /v1/indexes/{index}/snapshots` de P6 : comportement et suspension de l'écrivain (1) ; lignes `invalid_request` et `internal` (6) |
| `docs/07-stack-et-deploiement.md` | seulement si les options des deux commandes y sont décrites ; la liste des binaires (`:15`) les nomme déjà |
| `docs/phases/P3-cycle-de-vie.md` | seulement si le propriétaire veut préciser dans la fiche que la génération est inchangée et que l'époque remplacée est conservée (2, 4) |

## Références

- Fiche `CREER-ET-RESTAURER-INSTANTANE` : `docs/phases/P3-cycle-de-vie.md:121-130`
- `docs/04-donnees-et-ingestion.md` : export et instantanés (`:296-314`), règle d'exploitation de `crible patch` (`:282`)
- `docs/02-architecture.md` : époque et iid (`:45-53`), organisation sur disque et `exchange/` (`:55-80`), `CURRENT_EPOCH` et `meta.epoch` (`:84`), manifeste (`:85`), point de commit (`:105-127`), reprise (`:129-142`), concurrence (`:144-149`), compaction (`:155-181`), mémoire et tailles (`:188-199`)
- `docs/08-invariants-et-tests.md` : INV-02, INV-03, INV-04, INV-10, INV-14 (`:22`), INV-15 (`:23`)
- `docs/05-api.md` : codes d'erreur (`:31-44`), route d'instantané (`:66`), nature de job `snapshot` (`:251`)
- `docs/suivi.md:341` : rétention des époques intra-processus, verrou d'écrivain à trancher avec P6
- `crates/crible-store/src/compact.rs` : protocole de l'époque (`:285-312`), `reclaim_epochs` (`:170-190`), résidus de transit (`:201-209`)
- `crates/crible-store/src/generation.rs:156-197` : refus d'un index partiel sans `CURRENT_EPOCH` ; `crates/crible/src/commands.rs:67` : `INITIAL_EPOCH`
- ADR-0017 (la compaction suspend les écritures), ADR-0038 (`busy_timeout` assure la vivacité), ADR-0051 (chargement paresseux de l'encodeur), ADR-0060 (génération sans changement d'époque), ADR-0061 (export en ligne, refus d'écraser), ADR-0064 (format et contenu de l'instantané)
