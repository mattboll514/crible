# ADR-0087 — Reprise lexicale à l'ouverture en écriture et comportement en lecture seule

- **Statut** : Accepté (décidé le 2026-10-28, tel que proposé)
- **Date** : 2026-10-28 (proposition et décision)
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, à la demande de l'orchestrateur, sur une `QUESTION-CONCEPTION` de `@dev` dans la tâche `INDEXER-TANTIVY-A-L-ATTACHE` (phase P5)
- **Remplace** : —

## Contexte

La tâche `INDEXER-TANTIVY-A-L-ATTACHE` (`docs/phases/P5-lexical-et-hybride.md:27-40`) branche tantivy sur le protocole d'attache. `@dev` a livré l'étape 4 de `docs/02-architecture.md:118` — un index tantivy par époque dans `epochs/<E>/lexical/`, ajout et suppression des passages, commit dont la charge utile est la génération `G+1`, lecteur rechargé dans l'instantané — et la reprise de `docs/02:139-141` : charge utile du dernier commit tantivy `Gt` comparée à la génération SQLite `G`, rattrapage si `Gt < G`, reconstruction complète si `Gt > G`, la reconstruction supprimant d'abord le dossier `lexical/`.

Il a fait tourner cette reprise **aussi à l'ouverture en lecture seule** (`crible search`), comme la réécriture du manifeste manquant. Le code du corps commun aux deux ouvertures appelle `recover_lexical` sans distinguer le mode (`crates/crible-store/src/generation.rs:695`).

**Verdicts de `@architecte`.**

- **Point 1, reprise exécutée à la lecture : ECART.** Trois décisions acceptées la refusent :
  - ADR-0069:114 tranche que « tout futur fichier dérivé doit suivre la même règle : reconstruit à l'ouverture en écriture, en RAM seulement en lecture » ;
  - ADR-0056:44 (décision 4) : « une lecture ne modifie plus le disque » ;
  - `docs/02:151` : un seul écrivain par index, les lectures n'étant jamais bloquées (INV-02).

  `lexical/` est un fichier dérivé (`docs/02:73,80`). Deux dégâts concrets en découlent. L'`IndexWriter` de tantivy prend le **verrou du répertoire** : un `crible search` qui commite fait échouer l'attache du serveur sur le même index, ou l'inverse. Et la reconstruction supprime des fichiers sous les `mmap` d'un serveur vivant, ce qui atteint INV-02 et INV-03 (`docs/08:10-11`).

  La correction dans la tâche est acquise : **la reprise lexicale ne s'exécute qu'à l'ouverture en écriture**. Ce qui reste **NON-COUVERT**, et que cet ADR tranche, est ce que fait une ouverture en lecture seule quand `Gt ≠ G`.

- **Point 2, durée de l'étape 4 : ECART.** `@dev` compte la durée du commit tantivy dans `snapshot` (étape 6). `docs/04-donnees-et-ingestion.md:229` dit « Chaque étape de l'attache — `validation`, `segment`, `transaction`, `manifest`, `snapshot` — est mesurée », et `docs/02:118` fait de tantivy l'étape 4, distincte des étapes 5 et 6. La durée de l'étape 4 n'est donc pas attribuable.

- **Point 3, budget de l'`IndexWriter` : NON-COUVERT mais gardé.** Le budget (50 MiB, un thread) est une constante du code, ce qu'autorise AGENTS.md règle 12 (pas de configuration pour une valeur fixe). Aucune décision ne le fixe pour autant.

- **Réécriture du manifeste manquant à la lecture (`ensure_manifest`).** Elle est appliquée par les deux ouvertures (`crates/crible-store/src/generation.rs:699`, `docs/02:85`) et **aucun ADR ne la décide**. `@architecte` juge qu'elle ne justifie pas d'étendre la pratique à tantivy : c'est un petit JSON, sans verrou exclusif et sans suppression de fichiers sous `mmap`.

## Décision

**1. La reprise lexicale n'a lieu qu'à l'ouverture en écriture.** À l'ouverture en lecture seule, le moteur ne compare plus `Gt` à `G`, n'ouvre aucun `IndexWriter` tantivy, ne commite rien, ne supprime rien sous `lexical/`, et n'écrit rien dans l'époque. La règle d'ADR-0069:114 s'applique à `lexical/` telle quelle : dérivé, reconstruit à l'ouverture en écriture, en RAM seulement en lecture. Un lecteur tantivy n'est ouvert que si la charge utile du dernier commit égale `G`.

**2. Quand `Gt ≠ G` et que l'ouverture est en lecture seule, l'instantané n'a pas de lecteur lexical, et la branche lexicale est refusée explicitement.** Le mode `dense` continue de servir la génération `G`, inchangé. Les modes `lexical` et `hybride` échouent, sans dégradation silencieuse vers la branche dense : une réponse `hybrid` réduite au dense porterait un `score_type: rrf` faux et un classement que le client n'a pas demandé. Le refus appartient à la branche lexicale, livrée par la tâche `RECHERCHER-BM25-DANS-LE-POOL` (`docs/phases/P5-lexical-et-hybride.md:53`) ; l'instantané se contente d'exposer l'absence de lecteur. Aucun invariant nouveau : INV-04 reste tenu par la prochaine ouverture en écriture, qui répare avant de publier son instantané.

**3. Code d'erreur du refus : `overloaded` (503).** C'est le code existant le plus juste de `docs/05-api.md:42` : il désigne déjà une ressource momentanément indisponible qu'il faut réessayer — file d'écriture pleine, verrou SQLite non obtenu (ADR-0038), verrou de présence d'un lot en ligne (ADR-0056), course de génération pendant un export (ADR-0061). L'état est bien temporaire : la prochaine ouverture en écriture — le démarrage du serveur en P4 et P5, ou une attache — répare l'index, et la requête réussit ensuite. Le message nomme l'état sans identifiant interne (« lexical index behind generation Gt of G; open the index for writing to repair it ») : la génération est un état visible, aucun `iid` n'y figure (INV-01).

`internal` (500) est **écarté** : il signale une erreur inattendue. Or une charge utile lexicale en retard sur SQLite est un état **attendu** après un arrêt entre l'étape 3 et l'étape 4, que la conception décrit (`docs/02:139-141`) ; il polluerait les analytics et accuserait le moteur d'une faute qu'il n'a pas commise. `invalid_request` (400) est écarté aussi : la même requête réussit après un redémarrage du serveur, rien dans la requête n'est fautif.

Si le propriétaire préfère un code dédié (`lexical_not_ready`, 503), il demande une ligne dans `docs/05` et un ADR : **ce n'est pas décidé ici**, comme pour la contention d'un lot en ligne (ADR-0056:50).

**4. La réécriture du manifeste manquant à la lecture reste hors périmètre.** Le présent ADR ne la change pas : elle garde son comportement actuel, aux deux ouvertures. Elle est signalée en « À surveiller ». Conséquence de rédaction : elle ne peut pas être citée comme précédent pour un autre fichier dérivé, tant qu'un ADR ne l'a pas tranchée.

**5. La durée de l'étape 4 devient une étape mesurée à part.** `AttachDurations` (`crates/crible-store/src/attach.rs`) reçoit un champ `lexical`, dans l'ordre du protocole : `validation`, `segment`, `transaction`, `lexical`, `manifest`, `snapshot`. C'est de la **télémétrie d'appel seulement** : les durées ne sont pas persistées (ADR-0042), aucune forme persistée ne change, `batches.report` reste identique. Le champ couvre l'ajout, la suppression, le commit tantivy et le rechargement du lecteur.

Ce que cet ADR ne décide pas : le code d'erreur d'une recherche lexicale sur un index dont `lexical.enabled` vaut `false` ; le budget de l'`IndexWriter` (point 3) ; le sort du dossier `lexical/` pendant une compaction (tâche `RECONSTRUIRE-LEXICAL`) ; une sous-commande de réparation explicite, absente de `docs/07`.

## Conséquences

- **Positives** :
  - une lecture ne modifie plus le disque, pour `lexical/` comme pour les segments (ADR-0056:44, ADR-0069:114) ;
  - plus de verrou de répertoire tantivy pris par une commande de lecture : `crible search` ne peut plus faire échouer l'attache du serveur, ni l'inverse (`docs/02:151`) ;
  - plus de suppression de fichiers sous les `mmap` d'un serveur vivant : INV-02 et INV-03 restent hors d'atteinte de la CLI ;
  - la reprise devient une propriété de l'écrivain, au même endroit que les autres reconstructions dérivées, donc un seul chemin à tester (INV-04) ;
  - la durée de tantivy est attribuable, ce qui rend MES-06 lisible : on saura si le commit ou l'analyse domine.
- **Négatives / coûts** :
  - un index laissé en retard par un arrêt entre l'étape 3 et l'étape 4 refuse `lexical` et `hybrid` à la CLI tant qu'aucune ouverture en écriture n'a eu lieu ; la commande le dit, mais elle ne répare pas ;
  - `crible search --mode hybrid` perd son intérêt sur une copie de mesure ouverte en lecture seule après un incident, alors qu'un serveur démarré réparerait ;
  - `overloaded` (503) couvre un cas de plus, dans un code déjà chargé (`docs/05:42`) ;
  - un champ de plus à tenir dans `AttachDurations` et dans ses tests, sans changement de la forme persistée ;
  - une règle de plus à respecter par tout futur fichier dérivé, et un écart documenté de plus entre la lettre d'ADR-0069 et le manifeste (point 4).
- **État des tâches** : `INDEXER-TANTIVY-A-L-ATTACHE` reste **`BLOQUE`** jusqu'à la décision du propriétaire. `docs/suivi.md`, section « Écarts à la conception », doit citer ADR-0087 pour le point 1 et pour le point 2 ; ces deux lignes sont tenues par l'orchestrateur.
- **Documents à modifier à l'acceptation** :
  - `docs/02-architecture.md:139-141` : écrire que la comparaison `Gt`/`G` et la reprise n'ont lieu qu'à l'ouverture en **écriture**, et décrire le comportement en lecture seule (pas de lecteur lexical, branche refusée en `overloaded`) ;
  - `docs/02:80` : préciser que la reconstruction des fichiers dérivés se fait à l'ouverture en écriture, en RAM en lecture seule — la phrase vaut alors pour `lexical/` sans exception ;
  - `docs/04-donnees-et-ingestion.md:229` : ajouter `lexical` à la liste des étapes mesurées, entre `transaction` et `manifest` ;
  - `docs/05-api.md:42` : ajouter le cas « index lexical en retard sur la génération, ouverture en lecture seule » à `overloaded` ;
  - `docs/03-recherche.md:93` : mentionner, parmi les cas de reconstruction, que la reprise d'un retard n'a pas lieu à la lecture ;
  - `docs/phases/P5-lexical-et-hybride.md` : la fiche `INDEXER-TANTIVY-A-L-ATTACHE` est corrigée dans la tâche (reprise à l'ouverture en écriture, champ `lexical`) ; la fiche `RECHERCHER-BM25-DANS-LE-POOL` (`:53-...`) doit porter le refus en `overloaded` et son critère d'acceptation ;
  - `docs/08-invariants-et-tests.md` : **aucune modification nécessaire** ; INV-02, INV-03 et INV-04 ne changent pas, le refus n'est pas un invariant mais un test de la branche lexicale.
- **À surveiller** :
  - `ensure_manifest` : une lecture écrit encore un petit JSON dans `manifests/` (`docs/02:85`), ce qui contredit à la lettre « une lecture ne modifie plus le disque » (ADR-0056:44). Le coût est faible — pas de verrou exclusif, pas de suppression, fichier dérivé de la génération —, mais si un déploiement ouvre un jour un index sur un support réellement non inscriptible, ou si un contrôle d'intégrité exige qu'une lecture ne change aucun octet, un ADR devra aligner la pratique ;
  - le budget de l'`IndexWriter` (50 MiB, un thread, constante du code) : **MES-06** doit mesurer le pic RSS du commit et le **nombre de segments produits**, en plus de la taille de l'index et de la durée de construction. Un ADR n'est justifié que si la mesure montre que ce budget est structurant — par exemple s'il faut le régler par index pour tenir la cible de mémoire ;
  - un index laissé en retard de façon durable : si l'exploitation finit par garder des index ouverts en lecture seule sans serveur écrivain, le refus deviendra une gêne récurrente et il faudra une commande de réparation décidée, pas un contournement ;
  - `Gt > G` signalant une incohérence : la reconstruction complète reste la réponse, mais sa fréquence est un signal — un `Gt` en avance sur SQLite répété indiquerait que la charge utile de commit n'est pas écrite au bon moment.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| (a) La lecture ne répare pas et refuse la branche lexicale — **retenue** | Cohérente avec ADR-0069:114 et ADR-0056:44, coût nul à la lecture, aucun verrou ni suppression sous `mmap` |
| (b) Reconstruction en RAM à la lecture | Respecte la lettre d'ADR-0069, mais coûte des minutes et des gigaoctets à 8,58 M passages pour une commande CLI, à chaque invocation, pour un état que la prochaine ouverture en écriture répare seule. Un index tantivy complet en `RamDirectory` n'est pas une vue de lecture |
| (c) Statu quo : écrire le disque à la lecture | Contredit ADR-0069:114, ADR-0056:44 et `docs/02:151` ; le verrou de l'`IndexWriter` fait échouer l'attache du serveur, et la reconstruction supprime des fichiers sous des `mmap` vivants (INV-02, INV-03) |
| (d) Réparer à la lecture en prenant le verrou du répertoire | Même écriture disque que (c) avec un échec en `overloaded` au lieu d'un commit en course : une lecture bloquerait l'écrivain légitime, ce que `docs/02:151` interdit |
| (e) Dégradation silencieuse : `hybrid` servi par le seul dense | Rend un classement et un `score_type` qui ne correspondent pas à la requête, sans le dire ; le client ne peut pas distinguer un index à réparer d'un index sans branche lexicale |
| (f) Répondre `internal` au refus | Fait passer un état attendu (`docs/02:139-141`) pour une panne du moteur, et fausse les analytics |
| (g) Répondre `invalid_request` au refus | Accuse la requête, alors que la même requête réussit après un redémarrage du serveur |
| (h) Trancher `ensure_manifest` dans le même ADR | Élargit le périmètre à une pratique qu'aucun ADR ne porte, pour un fichier sans verrou ni suppression ; mieux vaut la signaler que la légitimer par contagion |

## Références

- `docs/02-architecture.md:80,85,118,133-145,151-153` — fichiers dérivés, manifeste, étapes de l'attache, reprise au démarrage, concurrence.
- `docs/03-recherche.md:78-93` — branche lexicale, reconstruction.
- `docs/04-donnees-et-ingestion.md:229` — étapes mesurées, télémétrie d'appel.
- `docs/05-api.md:29-44` — codes d'erreur, `overloaded`.
- `docs/08-invariants-et-tests.md:10-13` — INV-02, INV-03, INV-04.
- `docs/phases/P5-lexical-et-hybride.md:27-40,53-...` — `INDEXER-TANTIVY-A-L-ATTACHE`, `RECHERCHER-BM25-DANS-LE-POOL`.
- ADR-0007 (tantivy pour BM25), ADR-0016 (point de commit), ADR-0042 (durées non persistées), ADR-0056:44 (une lecture ne modifie plus le disque), ADR-0069:114 (règle des fichiers dérivés), ADR-0086 (`lexical.language`).
- `crates/crible-store/src/generation.rs:641-709` (ouverture et reprise, `recover_lexical`, `ensure_manifest`), `crates/crible-store/src/attach.rs:232-262` (`AttachDurations`).
