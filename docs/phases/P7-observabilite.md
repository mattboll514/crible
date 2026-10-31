# P7 — Observabilité

**But** : rendre Crible vérifiable sans lire le code. Cette phase couvre les analytics sans contenu, le journal de debug, le debug dans la réponse, `explain_ids`, les stats et l'éval.

**Jalon J7** :
- `crible eval` sur l'échantillon Legifacile produit un rapport complet : fidélité, qualité, latence, pour les trois modes ;
- `explain_ids` sur un document absent des résultats dit pourquoi ;
- `crible stats` montre le trafic de la démo ;
- INV-13 et INV-17 sont verts.

---

### CORRIGER-ARRET-SOUS-CHARGE
- **Origine** : ajoutée par le propriétaire le 2026-10-31, hors plan initial. Elle vient du rapport de J6 (`docs/jalons/J6-rapport.md`, §7 et §8) et de l'idée du 2026-10-31 notée dans `docs/suivi.md` (`NETTOYER-P6`, `serve_stops_on_sigint`). Les tests `serve_answers_the_probes_and_stops_on_sigterm` et `serve_stops_on_sigint` (`crates/crible/tests/serve_cli.rs`, échec à la ligne 260, `expect("the reply is readable")`, erreur `ConnectionReset` / « Connection reset by peer ») ont chacun échoué une fois sous la charge de la suite complète, et sont verts 5 fois sur 5 lancés seuls. Le rapport de J6 proposait de rendre `try_get` tolérant à une connexion réinitialisée, en réessayant.
- **Objectif** : supprimer l'échec intermittent des tests d'arrêt de `crible serve` sous charge, en corrigeant sa **cause**, sans masquer un défaut du serveur.
- **Dépend de** : DEMARRER-SERVEUR-AXUM
- **À lire** : `adr/0099-cycle-de-vie-du-serveur-demarrage-health-ready-arret-propre.md` (cycle de vie du serveur, `/health`, `/ready`, arrêt propre) ; `adr/0104-arret-du-serveur-et-jobs-d-ecriture.md` (arrêt du serveur et jobs d'écriture) ; `crates/crible/tests/serve_cli.rs` ; `docs/jalons/J6-rapport.md` §7.
- **Livrables** :
  1. un **diagnostic, avant toute correction, écrit dans le compte rendu de `@dev`**, qui dit d'où vient la connexion réinitialisée :
     - soit du test : requête envoyée pendant le démarrage, ou après le signal d'arrêt, la réinitialisation étant alors admissible ;
     - soit du serveur : connexion acceptée puis fermée sans réponse avant l'arrêt, ce qui contredirait l'arrêt propre d'ADR-0099.
  2. la correction, à l'endroit de la cause :
     - côté test, `try_get` traite une connexion réinitialisée comme « pas encore prêt » ou « déjà arrêté », selon la phase du test, sans affaiblir aucune assertion ; c'est la piste du rapport de J6 ;
     - côté serveur, corriger l'arrêt.
  3. si le diagnostic révèle un comportement que la conception ne tranche pas, une `QUESTION-CONCEPTION`.
- **Critères d'acceptation** :
  - environ 50 exécutions consécutives vertes de `cargo test -p crible --test serve_cli`, dont une partie sous charge, par exemple en parallèle d'un `cargo test` d'une autre crate ;
  - les assertions sont inchangées : `/health` et `/ready` rendent toujours `200` avant l'arrêt, et le processus s'arrête avec le code attendu ;
  - aucun test n'est désactivé ni marqué `#[ignore]`.
- **Invariants** : aucun.

### AGREGER-ANALYTICS
- **Objectif** : les analytics agrégées.
- **Dépend de** : ROUTE-RECHERCHE, FILE-D-ECRITURE-ET-JOBS
- **À lire** : `06-observabilite.md` (analytics) ; `adr/0117-analytics-stockage-et-calcul-schema-histogrammes-percentile-et-formes-de-filtre.md` (schéma, seaux, percentile, formes de filtre) ; `adr/0118-analytics-execution-file-pertes-vidage-purge-operations-et-preuve-inv-17.md` (file, pertes, vidage, purge, preuve d'INV-17).
- **Livrables** :
  - `SearchEvent` et `OpEvent`, deux types **sans champ** pour le texte, les valeurs de filtre ou les ids, à la liste fermée de champs d'ADR-0117 décision 5 (un événement par cible en multi-index) ;
  - le calcul de `filter_shape` à partir de l'AST (champs et opérateurs seulement), selon la règle de canonicalisation d'ADR-0117 décision 6, rangé dans la table `filter_shapes` ;
  - un agrégateur par minute, avec des histogrammes à 64 seaux et un histogramme de pool en log2, aux bornes d'ADR-0117 décisions 2 et 3, dans `analytics.sqlite` au schéma d'ADR-0117 décision 1 ;
  - un vidage asynchrone toutes les 10 s et à l'arrêt, par la file bornée et le compteur `analytics_dropped` d'ADR-0118 décisions 1 à 3 ;
  - les lignes journalières par index, écrites au premier vidage où la ligne du jour UTC manque (ADR-0118 décision 5) ;
  - la purge selon la rétention, à l'ouverture puis toutes les 24 h (ADR-0118 décision 4).
- **Critères d'acceptation** :
  - un test vérifie que les histogrammes s'additionnent (un p95 calculé sur deux minutes fusionnées égale le p95 calculé sur les événements bruts, à la **résolution d'un seau** près, c'est-à-dire la largeur relative de 25 % fixée par ADR-0117 décision 4) ;
  - le vidage ne bloque jamais une requête (**INV-17**, file saturée artificiellement), prouvé par le test `inv_17_*` sous la feature `failpoints` décrit par ADR-0118 décision 7.
- **Invariants** : INV-17.

### TESTER-ANALYTICS-SANS-CONTENU
- **Objectif** : prouver INV-13.
- **Dépend de** : AGREGER-ANALYTICS
- **Livrables** : un test d'intégration qui lance 100 requêtes avec des textes et des valeurs de filtre uniques (marqueurs aléatoires), force le vidage, puis cherche chaque marqueur dans le fichier `analytics.sqlite` (octets bruts) et dans les logs `info`.
- **Critères d'acceptation** : aucun marqueur n'est trouvé.
- **Invariants** : INV-13.

### JOURNAL-DE-DEBUG
- **Objectif** : un journal complet, activé à la demande.
- **Dépend de** : AGREGER-ANALYTICS
- **À lire** : `06-observabilite.md` (journal de debug) ; `adr/0118-analytics-execution-file-pertes-vidage-purge-operations-et-preuve-inv-17.md` (décisions 7 et 8 : preuve d'INV-17 et rejeu de MES-16).
- **Livrables** :
  - `debug.sqlite` ;
  - un enregistrement si `debug: true` et que le journal est activé, et selon `sample_rate` ;
  - une écriture asynchrone ;
  - la purge selon la durée de vie (`ttl_days`) ;
  - `GET /v1/debug/queries`, réservé à l'admin, avec ses filtres ;
  - le test `inv_17_*` du journal de debug, sur le modèle d'ADR-0118 décision 7 (barrière sur l'écrivain sous la feature `failpoints`, file pleine, réponses identiques, pertes comptées) ;
  - le rejeu de MES-16 par `@mesureur` avec les analytics et le journal de debug actifs, après le `@dev` de la tâche (ADR-0118 décision 8) : rapport `docs/mesures/MES-16-charge-concurrente-analytics-et-journal.md`, données brutes JSON à côté, protocole d'ADR-0113 décision 5.
- **Critères d'acceptation** :
  - désactivé par défaut : aucun fichier écrit ;
  - la purge supprime bien les entrées expirées ;
  - un rôle `read` reçoit `403` (ADR-0095) ;
  - `GET /v1/debug/queries` figure dans la spécification OpenAPI, et son exception nommée est retirée du test des routes de PUBLIER-OPENAPI dans le même changement (ADR-0111, complément du 2026-10-30) ;
  - le test `inv_17_*` du journal est vert, et le rapport de rejeu de MES-16 existe et tient le critère d'ADR-0113 (chaque recherche sous 3 × le p99 du repos, `5xx` interdit).

### REPONSE-DEBUG
- **Objectif** : `debug: true` dans la réponse.
- **Dépend de** : ROUTE-RECHERCHE
- **À lire** : `05-api.md` (réponse, objet `debug`) ; `adr/0119-objet-debug-de-la-reponse-de-recherche-forme-rangs-temps-et-capture.md` (forme de l'objet, rangs, temps, capture) ; `adr/0120-fin-de-took-ms-attribution-de-l-ouverture-des-lecteurs-et-rendu-hors-chronometre.md` (fin de `took_ms` à la fin de `fetch`, ouverture des lecteurs rattachée à la clé `fetch`).
- **Livrables** : le plan et les temps par étape aux clés de `03-recherche.md` (ADR-0119 décisions 4 et 5), et pour chaque résultat le rang et le score dans chaque branche ainsi que le rang fusionné (ADR-0119 décisions 1 à 3) ; en multi-index, aussi le rang dans son index, `target_rank` (ADR-0119 décision 9), et la capture `DebugCapture { top_n }` construite seulement si `debug` est demandé (ADR-0119 décision 8).
- **Critères d'acceptation** :
  - la somme des étapes reste cohérente avec le total (écart inférieur à 10 %) : la somme est le chemin critique `encode + filter + max(dense, lexical) + fusion + group + fetch`, `validate` exclu, en `dense` et `lexical` sur la branche unique et en `hybride` sur le chemin critique (ADR-0119 décision 6) ; le total s'arrête à la fin de `fetch`, avant le rendu JSON, et l'ouverture des lecteurs appartient à la clé `fetch`, dans tous les modes (ADR-0120 décisions 1 et 2) ;
  - les rangs affichés correspondent à ceux d'une exécution branche par branche, prise après le filtre et avant `dedup`/`group_by` pour les branches et avant regroupement pour `fused_rank` (ADR-0119 décisions 1 et 2).
- **Invariants** : aucun.

### EXPLIQUER-DOCUMENTS
- **Objectif** : `explain_ids`.
- **Dépend de** : REPONSE-DEBUG
- **À lire** : `06-observabilite.md` (explain) ; `05-api.md` (format).
- **Livrables** : pour chaque document demandé :
  - s'il existe et s'il est vivant ;
  - s'il est dans le pool, et sinon les clauses qui l'excluent (chaque clause est évaluée séparément) ;
  - son rang dense exact (scan exact du pool), son rang binaire et sa présence dans les N candidats ;
  - son rang et son score BM25, avec l'explication tantivy ;
  - son rang fusionné, et s'il a été absorbé par un regroupement.
- **Critères d'acceptation** :
  - un test par cas : exclu par le filtre, mort, hors des N candidats binaires, rang lexical seul, absorbé par un regroupement ;
  - le cas « rang 70 en dense, rang 2 en lexical » de l'échantillon est expliqué.

### ROUTE-STATS
- **Objectif** : lire les analytics.
- **Dépend de** : AGREGER-ANALYTICS
- **À lire** : `06-observabilite.md` (lecture).
- **Livrables** :
  - `GET /v1/stats` : comptes, erreurs par code, p50/p95/p99 par étape, taux de requêtes sans résultat, index et formes de filtre les plus fréquents, état journalier ;
  - la commande `crible stats`, qui affiche des tableaux.
- **Critères d'acceptation** :
  - les chiffres affichés pour un trafic de test connu sont exacts ;
  - une clé `read` ou `write` reçoit `403` sur `/v1/stats`, réservé au rôle `admin` (ADR-0095) ;
  - `GET /v1/stats` figure dans la spécification OpenAPI, et son exception nommée est retirée du test des routes de PUBLIER-OPENAPI dans le même changement (ADR-0111, complément du 2026-10-30).

### COMMANDE-EVAL
- **Objectif** : l'instrument de validation du propriétaire.
- **Dépend de** : REPONSE-DEBUG
- **À lire** : `06-observabilite.md` (éval).
- **Livrables** :
  - `crible eval` (contre une instance via l'API, ou en local sur un dossier de données) ;
  - les mesures : fidélité contre `exact`, qualité contre `expected`, latences, comparaison `--baseline` ;
  - un rapport JSON et un rapport Markdown dans `docs/mesures/` ;
  - `docs/jalons/J7-demo.md`.
- **Critères d'acceptation** :
  - le rapport sur l'échantillon est lisible sans contexte ;
  - une régression injectée (N réduit) est bien détectée par `--baseline`.
