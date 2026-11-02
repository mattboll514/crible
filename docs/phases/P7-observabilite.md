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
- **À lire** : `06-observabilite.md` (journal de debug) ; `adr/0121-journal-de-debug-stockage-debug-sqlite-schema-payload-et-protections.md` (fichier, schéma, payload, protections) ; `adr/0122-journal-de-debug-execution-activation-capture-file-vidage-purge-route-et-preuves.md` (activation, capture, file, vidage, purge, route, preuves) ; `adr/0118-analytics-execution-file-pertes-vidage-purge-operations-et-preuve-inv-17.md` (décisions 7 et 8 : preuve d'INV-17 et rejeu de MES-16).
- **Livrables** :
  - `debug.sqlite`, au schéma d'ADR-0121 ;
  - l'enregistrement selon les **deux voies** d'ADR-0122 décision 1 : une requête `debug: true` est toujours enregistrée quand le journal est activé, `sample_rate` ne vise que les autres requêtes ; la capture `DebugCapture { top_n = 20 }` d'ADR-0119 décision 8 est construite si `debug` ou si la requête est échantillonnée, et une par cible en multi-index (ADR-0122 décision 3) ;
  - une écriture asynchrone par file bornée, les pertes étant comptées dans `debug_journal_dropped` (ADR-0122 décisions 6 et 7) ;
  - la purge selon la durée de vie (`ttl_days`), à l'ouverture puis toutes les 24 h (ADR-0122 décision 8) ;
  - `GET /v1/debug/queries`, réservé à l'admin, avec ses filtres et sa pagination keyset (ADR-0122 décision 9) ;
  - le test `inv_17_*` du journal de debug, sur le modèle d'ADR-0118 décision 7 (barrière sur l'écrivain sous la feature `failpoints`, file pleine, réponses identiques, pertes comptées), nommé `inv_17_debug_journal_never_blocks` (ADR-0122 décision 11) ;
  - le rejeu de MES-16 par `@mesureur` avec les analytics et le journal de debug actifs, après le `@dev` de la tâche (ADR-0118 décision 8) : rapport `docs/mesures/MES-16-charge-concurrente-analytics-et-journal.md`, données brutes JSON à côté, protocole d'ADR-0113 décision 5 (ADR-0122 décision 12).
- **Critères d'acceptation** :
  - désactivé par défaut : aucun fichier écrit ;
  - avec le journal activé et `sample_rate = 0`, une requête `debug: true` produit une ligne et une requête sans `debug` n'en produit aucune (ADR-0122 décision 1) ;
  - la purge supprime bien les entrées expirées ;
  - un rôle `read` reçoit `403` (ADR-0095) ;
  - `GET /v1/debug/queries` figure dans la spécification OpenAPI, et son exception nommée est retirée du test des routes de PUBLIER-OPENAPI dans le même changement (ADR-0111, complément du 2026-10-30) ;
  - le test `inv_17_*` du journal est vert, et le rapport de rejeu de MES-16 existe et tient le critère d'ADR-0113 (chaque recherche sous 3 × le p99 du repos, `5xx` interdit).
- **Invariants** : INV-17.

### REPONSE-DEBUG
- **Objectif** : `debug: true` dans la réponse.
- **Dépend de** : ROUTE-RECHERCHE
- **À lire** : `05-api.md` (réponse, objet `debug`) ; `adr/0119-objet-debug-de-la-reponse-de-recherche-forme-rangs-temps-et-capture.md` (forme de l'objet, rangs, temps, capture) ; `adr/0120-fin-de-took-ms-attribution-de-l-ouverture-des-lecteurs-et-rendu-hors-chronometre.md` (fin de `took_ms` à la fin de `fetch`, ouverture des lecteurs rattachée à la clé `fetch`).
- **Livrables** : le plan et les temps par étape aux clés de `03-recherche.md` (ADR-0119 décisions 4 et 5), et pour chaque résultat le rang et le score dans chaque branche ainsi que le rang fusionné (ADR-0119 décisions 1 à 3) ; en multi-index, aussi le rang dans son index, `target_rank` (ADR-0119 décision 9), et la capture `DebugCapture { top_n }` construite seulement si `debug` est demandé (ADR-0119 décision 8), ou si la requête est échantillonnée pour le journal (ADR-0122 décision 3).
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
  - s'il est dans le pool, et sinon les clauses qui l'excluent (chaque clause est évaluée séparément, sur une feuille du filtre et par son chemin JSON complet, ADR-0123 décision 7) ;
  - son rang dense exact (scan exact du pool), son rang binaire et sa présence dans les N candidats (ADR-0123 décision 5) ;
  - son rang et son score BM25, avec la liste fermée `terms` terme par terme (ADR-0123 décision 6) ;
  - son rang fusionné, et s'il a été absorbé par un regroupement.
- **Critères d'acceptation** :
  - un test par cas : exclu par le filtre, mort, hors des N candidats binaires, rang lexical seul, absorbé par un regroupement ;
  - le cas « rang 70 en dense, rang 2 en lexical » est expliqué sur une fixture synthétique qui reproduit ce cas, verte en CI (ADR-0123 décision 11).
- **Note du 2026-11-01** (ADR-0123, décision 11) : ce cas n'existe pas sur `data/legifacile-sample`, où MES-10 mesure l'article 1240 à rang dense 1 et lexical 5 ; le prouver sur l'échantillon demanderait un sixième test local exclu de la CI, hors de la liste fermée de `docs/07:148-150`. La fixture synthétique fixe le cas — rang dense exact 70, rang lexical 2 — sans modèle ni données de production.

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
- **Note du 2026-11-01** (ADR-0125, décision 1) : le mode « contre une instance via l'API » (`:118`) est **reporté**. `docs/07:29-50` ne porte aucun client HTTP : ce mode demanderait une dépendance et un ADR, à proposer au propriétaire avec P8 ou P9 (instance déployée). En P7, `crible eval` fonctionne **en local seulement**, sur un dossier de données ouvert en lecture (ADR-0116), y compris à côté d'un serveur en service. **Renvoi du 2026-11-02** : le mode API est placé en P8 par le propriétaire (« ajoute la tâche en P8 »), dans la fiche `EVALUER-PAR-API` (`docs/phases/P8-docker-et-deploiement.md`), après `DEFINIR-VOLUMES-ET-CONFIG`.
- **Note du 2026-11-01** (ADR-0125, décisions 8 et 14) : la « régression injectée (N réduit) » (`:124`) vaut `--rescore-candidates 20` avec `k = 10`, et les tests d'`eval` tournent en CI sur `synth-small` avec des requêtes à `vector`. Le rapport de l'échantillon est produit **en local par `@dev`** dans `docs/mesures/`, et la démo de J7 le rejoue. Aucun nouveau test local n'est exclu de la CI (ADR-0062).

### REUTILISER-LECTEURS-DENSES
- **Origine** : hors plan initial, décision du propriétaire du 2026-11-01 sur recommandation de l'orchestrateur, en application de la décision 3 d'ADR-0120 (cache du `DenseSearcher` par génération retenu en principe). Le constat vient de REPONSE-DEBUG : chaque recherche dense ouvre ses lecteurs de segments (`DenseSearcher::open`), pour 0,6 ms à 4 000 passages et 2,3 ms à 20 000 passages, un coût qui croît avec le corpus ; `crates/crible-search/src/dense/exact.rs:148-153` dit pourtant les lecteurs « kept for every later query ». Ce coût n'est pas mesuré à 8,58 M passages.
- **Objectif** : supprimer l'ouverture des lecteurs denses du chemin de chaque requête, si elle compte à l'échelle de Legifacile.
- **Dépend de** : REPONSE-DEBUG
- **À lire** : `adr/0119-objet-debug-de-la-reponse-de-recherche-forme-rangs-temps-et-capture.md` (objet `debug`) ; `adr/0120-fin-de-took-ms-attribution-de-l-ouverture-des-lecteurs-et-rendu-hors-chronometre.md` (décisions 2 et 3 : attribution provisoire à `fetch`, cache cible) ; les ADR sur l'accès aux vecteurs et sur les générations ou époques : `adr/0080-acces-aux-vecteurs-fp16-au-rescoring.md`, `adr/0084-quel-froid-fait-foi-pour-le-rescoring-jusqua-p8.md`, `adr/0006-segments-pierres-tombales-compaction.md`, `adr/0017-compaction-suspend-les-ecritures.md`, `adr/0065-instantane-hors-ligne-epoque-de-restauration-et-retention.md` (rétention), `adr/0105-administration-des-index-par-l-api.md` (`Published`, configuration et instantané publiés ensemble), `adr/0108-coherence-de-l-epoque-apres-une-compaction-en-echec.md` ; `03-recherche.md:14` (clé `fetch`) ; `02-architecture.md` (générations et publication de l'instantané).
- **Livrables** :
  1. **d'abord une mesure**, par `@mesureur`, du coût d'ouverture des lecteurs denses à 8,58 M passages (index `synth-large` de `data/`, `--release`), p50 et p95 sur une série, consignée dans un court rapport sous `docs/mesures/` ;
  2. si ce coût est **négligeable** (p95 sous 1 ms), la tâche se ferme **sans code**, avec le rapport comme preuve et la décision 2 d'ADR-0120 maintenue ;
  3. sinon, les lecteurs denses sont ouverts **une fois par génération publiée** et réutilisés par les requêtes de cette génération, puis libérés quand la génération n'est plus servie ; un ADR, à rédiger au démarrage si la conception ne tranche pas, fixe la durée de vie, l'interaction avec la compaction, la rétention des époques et la mémoire ; la clé `fetch` retrouve son sens de `03-recherche.md:14`, la décision 2 d'ADR-0120 devient sans objet et `docs/03` est mis à jour.
- **Critères d'acceptation** :
  - la mesure existe ;
  - si le cache est fait : aucune ouverture de lecteur dense sur le chemin d'une requête d'une génération déjà servie (test) ;
  - si le cache est fait : INV-02 vert (lectures pendant les écritures) ;
  - si le cache est fait : aucun lecteur d'une génération retirée n'est gardé au-delà de sa rétention ;
  - si le cache est fait : le critère des 10 % de REPONSE-DEBUG est toujours vert ;
  - si le cache est fait : la latence avant/après est mesurée.
- **Invariants** : INV-02 (si le cache est fait).
- **Note du 2026-11-01 (application d'ADR-0126)** : le livrable 3 est précisé sans changer son but. Les lecteurs sont mis en cache **par segment**, clé `segment_id`, et le pool de charges utiles **par époque** ; « une fois par génération publiée » se lit donc comme « une fois par segment et par processus, publiés avec la génération ». Le sceau blake3 reste vérifié à chaque ouverture de lecteur, donc une fois par segment et par processus, pendant le remplissage. Le remplissage a lieu **avant** la publication de la génération, hors du verrou d'écriture : le critère « aucune ouverture sur le chemin d'une requête d'une génération déjà servie » se lit alors comme « aucune génération n'est servie sans ses lecteurs », et le cas de la première requête d'une génération non remplie n'existe pas. Le critère de rétention porte sur les **instantanés** : un lecteur lâché ne tient aucune `EpochLease`. Le critère de latence est prouvé par le **serveur** en `--release` sur `synth-large`, `k` et N fixes, au moins 300 requêtes à cache chaud, `debug: true`, p50/p95/p99 de `total` et de `fetch` avant/après, plus le temps de démarrage ; `crible eval` n'est pas l'outil de cette preuve (ADR-0125, décision 9), et le harnais reproductible est `crates/crible/examples/dense_latency.rs`.

### COUPER-TELEMETRIE-ONNX
- **Origine** : hors plan, 2026-11-02, préparation de J7. La série locale `cargo test --release -- --include-ignored` s'est bloquée deux fois sur deux sur `api_and_cli_return_the_same_results_on_the_legifacile_sample` : un `crible search` fini tourne indéfiniment dans `exit()`, sur le client de télémétrie 1DS embarqué par ONNX Runtime 1.31.0. Le verdict de `@architecte` est `NON-COUVERT` : la conception ne prévoit ni n'interdit cette télémétrie, qui envoie des données d'usage à un tiers, production comprise.
- **Objectif** : couper la télémétrie d'ONNX Runtime partout où Crible le charge, pour débloquer la sortie de processus et supprimer l'envoi à un tiers, sans changer les résultats de recherche.
- **Dépend de** : —
- **À lire** : `adr/0128-couper-la-telemetrie-d-onnx-runtime.md` (décisions 1 à 10) ; `adr/0057-onnx-runtime-et-vrai-modele-mdenseon-en-integration-continue.md` (job `rust-onnx`, `--include-ignored`) ; `docs/07-stack-et-deploiement.md:54` et `:59` (chargement d'ORT), `:116-123` (image Docker) ; `docs/06-observabilite.md:8` et `:36-40` (INV-13) ; la fiche `CONSTRUIRE-IMAGE-DOCKER` de `docs/phases/P8-docker-et-deploiement.md` ; `tools/README.md`.
- **Livrables** :
  1. `ORT_DISABLE_TELEMETRY=1` posé au tout début de `main` (`crates/crible/src/main.rs`, avant `Cli::parse()`), valeur forcée à `1`, avec un commentaire `// SAFETY:` (aucun fil n'existe encore) et un test ;
  2. `with_telemetry(false)` sur l'`EnvironmentBuilder` de `crates/crible-encode/src/model.rs`, sans effet sur la sûreté, et aucune pose de variable dans `crible-encode` ;
  3. `.cargo/config.toml`, section `[env]`, `ORT_DISABLE_TELEMETRY = { value = "1", force = true }`, pour les binaires de test qui chargent ORT dans leur processus ;
  4. un test `#[ignore]` qui lance un enfant `crible search` avec `HOME`, `XDG_DATA_HOME` et `XDG_CONFIG_HOME` temporaires, retire de son environnement `ORT_DISABLE_TELEMETRY`, `TF_BUILD`, `GITHUB_ACTIONS`, `GITLAB_CI`, `CI` et `ORT_RUNNING_UNIT_TESTS`, et vérifie l'exit `0` et l'absence de tout `.onnxruntime` ou `onnxruntime.db` sous ces dossiers ; il tourne dans le job `rust-onnx` (ADR-0057) ;
  5. la vérification que `docs/07`, `docs/06`, la fiche P8 et `tools/README.md` sont à jour selon ADR-0128 (appliqués par `@greffier` à l'acceptation : la tâche vérifie, ne réécrit pas).
- **Critères d'acceptation** :
  1. aucun fichier du client 1DS créé sous un `HOME` temporaire par `crible search` (test `#[ignore]` vert en local et dans `rust-onnx`) ;
  2. `ORT_DISABLE_TELEMETRY=1` posée au début de `main`, avec `// SAFETY:` et un test ;
  3. `with_telemetry(false)` sur l'environnement ORT ;
  4. `.cargo/config.toml` `[env]` en place ;
  5. résultats de recherche inchangés et `api_and_cli_return_the_same_results_on_the_legifacile_sample` vert en local sans variable posée à la main ;
  6. `docs/07`, `docs/06`, fiche P8 et `tools/README.md` à jour selon ADR-0128 (déjà appliqués par `@greffier` à l'acceptation : la tâche vérifie, ne réécrit pas).

  `cargo fmt`, `clippy -D warnings` et `cargo test` verts.
- **Invariants** : aucun (pas d'INV-18 : décision 9 d'ADR-0128).
- **Note du 2026-11-02 (application d'ADR-0128)** : l'ADR est `Accepté` et sa décision 10 est appliquée par `@greffier`. Les documents cités sont à jour : dans `docs/07-stack-et-deploiement.md`, la politique ORT est au paragraphe du **:56** (elle était attendue près de `:54`), la note de l'environnement de développement Mac au **:61** (près de `:59`), et la section Docker va du **:116** au **:126**, avec `ENV ORT_DISABLE_TELEMETRY=1` dans la liste ; `docs/06-observabilite.md:13` porte la phrase sur l'absence de télémétrie tierce ; la fiche `CONSTRUIRE-IMAGE-DOCKER` porte le livrable et le critère `docker image inspect` ; `tools/README.md` porte la note documentaire sur la roue ORT. Le livrable 5 et le critère 6 de cette fiche restent une **vérification** : la tâche ne réécrit aucun de ces documents, seul le code change.
- **Hors périmètre** : `clients/python/` (HTTP seulement) et tout code dans `tools/` ; la réouverture de la télémétrie ; la mesure d'un gain de latence à la sortie.
