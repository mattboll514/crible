# P9 — Tableau de bord

**But** : un tableau de bord **visuel en lecture seule**, intégré à `crible serve` et servi sous `/ui` dans la même image, qui montre les données et les analytics de Crible. Trois fonctions : **parcourir les données** (index, configuration, comptes, documents et passages, génération courante, nombre de segments), **tester une recherche** (bac à sable dense, lexical et hybride, mono et multi-index, panneaux `debug` et `explain`) et **voir les analytics** de `GET /v1/stats`. Écrit en HTML, CSS et JavaScript simples, sans framework, sans étape de build et sans dépendance nouvelle ; `cargo build` suffit. Le style est **très épuré** (référence [anara.com](https://anara.com/) : beaucoup de blanc, typographie soignée, peu de couleurs) et l'interface est en **anglais**. Le détail visuel est figé par la maquette validée par le propriétaire, première tâche de la phase (ADR-0130, décisions 1, 2, 3, 10 et 14).

**Jalon J9** (ADR-0130, décision 13) :
- le binaire de P8 sert `/ui` : la page se charge sans clé, sans requête vers un hôte externe (onglet réseau), avec la CSP ;
- avec une clé `read` : parcourir les index, voir config, comptes, époque, génération, nombre de segments ; lister les documents ; ouvrir un document et lire ses passages ;
- bac à sable : une recherche `dense`, une `lexical`, une `hybride` avec filtre et `k`, en mono et multi-index ; les clés de résultat sont identiques à celles de la même requête en `curl` ; les panneaux `debug` et `explain` s'affichent ;
- analytics avec une clé `admin` : seaux, p50/p95/p99 par étape, erreurs par code, formes de filtre ; avec une clé `read`, la page l'annonce et n'affiche aucune donnée ;
- aucun `iid`, aucun `segment_id`, aucun `iid_start` à l'écran ni dans le DOM (test et inspection) ;
- `cargo build` suffit à produire la page ; la CI ne gagne aucune étape ; `cargo fmt`, `clippy -D warnings` et `cargo test` verts ; `openapi_routes` vert avec l'exception `/ui`.

---

### MAQUETTE-TABLEAU-DE-BORD

- **Objectif** : figer avec le propriétaire la maquette des quatre vues du tableau de bord et les règles visuelles, avant toute implémentation.
- **Dépend de** : J8 validé. La tâche se fait **avec le propriétaire**, qui valide la maquette ; aucune tâche du plan ne la précède (ADR-0130, décision 13).
- **À lire** : `adr/0130-tableau-de-bord-integre.md` (décisions 1, 2, 3, 7, 10, 13 et 14), `01-vision-et-perimetre.md` (`:41` et le tableau du hors-périmètre). Référence de style : https://anara.com/.
- **Livrables** :
  - une maquette des **quatre vues** prévues par la décision 13 : index, document, recherche, analytics ;
  - les règles visuelles retenues, écrites : couleurs, typographie, espacements, densité ; l'interface est en **anglais** (décision 10) ;
  - une maquette compatible avec le service d'ADR-0130 : aucun script ni style en ligne, aucun hôte externe, police système ou fichier embarqué (décision 3) ;
  - la validation du propriétaire, consignée.
- **Critères d'acceptation** :
  - le propriétaire a **explicitement validé** la maquette ;
  - les vues qui affichent des données couvrent les informations de la décision 8 — dont la génération courante et le nombre de segments — et **ni** tailles de pool, **ni** historique des générations, **ni** liste des segments, **ni** journal de debug ;
  - la maquette ne montre aucun `iid`, aucun `segment_id` et aucun `iid_start` (décision 7) ;
  - chaque vue se réalise sans script ni style en ligne et sans hôte externe (décision 3) ;
  - les règles visuelles suffisent à écrire les pages sans nouvelle question de style.
- **Note** : aucun document ne fixe où la maquette est conservée. La tâche tranche l'emplacement avec le propriétaire et l'indique dans son compte rendu.
- **Invariants / mesures** : INV-01 (aucun id interne à l'écran) ; aucune mesure MES-xx.
- **Hors périmètre** : écrire le HTML, le CSS ou le JavaScript embarqué (`SERVIR-UI-STATIQUE`) ; choisir une bibliothèque de graphiques, écartée par la décision 11 ; une interface en français (décision 10) ; une page d'administration (décision 1).

### SERVIR-UI-STATIQUE

- **Objectif** : embarquer les actifs du tableau de bord dans le binaire et les servir sous `/ui`, avec leurs en-têtes, sans clé et sans dépendance nouvelle.
- **Dépend de** : MAQUETTE-TABLEAU-DE-BORD.
- **À lire** : ADR-0130 (décisions 2 à 5, 7, 9 à 12), ADR-0111 (décisions 4 et 5), `05-api.md` (`:78-80`), `07-stack-et-deploiement.md` (« Organisation du dépôt », « Dépendances Rust », « Docker », « CI »), `08-invariants-et-tests.md` (INV-01), `crates/crible-server/src/http.rs` (`:87-99`), `crates/crible-server/tests/openapi_serving.rs` (`:105-140`).
- **Livrables** :
  - `crates/crible-server/src/ui/` : les actifs embarqués (HTML, CSS, JS, favicon `.svg` éventuel) et leur module, par `include_bytes!` et `include_str!` (décision 3) ;
  - un **handler axum** écrit à la main, qui sert `/ui` et chaque actif : type MIME écrit pour `.html`, `.css`, `.js` et `.svg`, `charset=utf-8`, `Cache-Control: no-cache` et `ETag` (décision 3) ;
  - sur chaque réponse `/ui`, les en-têtes de la décision 3 : `Content-Security-Policy` avec la valeur exacte de l'ADR, `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer` ;
  - le montage dans `crates/crible-server/src/http.rs`, **sans clé d'API** et **sans clé de configuration** (décisions 4 et 9) ; un chemin inconnu sous `/ui` rend `404 not_found` au format d'ADR-0101 ;
  - `/ui` ajouté aux **exceptions nommées** de `crates/crible-server/tests/openapi_routes.rs`, comme `/docs`, et **aucun** chemin `/ui` dans la spécification (décision 12, point 5) ;
  - `crates/crible-server/tests/ui_serving.rs`, sur le précédent d'`openapi_serving.rs` ;
  - le montage de la page de la maquette, sans encore ses appels d'API : les pages sont les tâches suivantes.
- **Critères d'acceptation** :
  - `GET /ui` et **chaque actif** rendent `200` sans clé, avec le `Content-Type` attendu et `charset=utf-8`, la `Content-Security-Policy`, `X-Content-Type-Options` et `Referrer-Policy` (décision 12, point 1) ;
  - `GET /ui/inconnu` rend `404 not_found` au format d'ADR-0101 (point 2) ;
  - le **balayage des actifs embarqués** ne trouve ni `http://`, ni `https://`, ni `//cdn`, ni `<script>` en ligne, ni `<style>` en ligne, ni `@import`, ni `url(http` (point 3) ;
  - chaque `href` et chaque `src` d'`index.html` correspond à un fichier embarqué, et aucun fichier embarqué n'est orphelin (point 4) ;
  - `openapi_routes` est vert avec `/ui` excepté, et la spécification ne porte aucun chemin `/ui` (point 5) ;
  - aucune dépendance ni feature n'est ajoutée : ni `fs` de `tower-http`, ni `rust-embed`, ni `include_dir`, ni bibliothèque de graphiques (décision 3 ; `07` inchangé) ;
  - `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` et `cargo test` passent.
- **Invariants / mesures** : INV-01, par un test `inv_01_*` étendu au balayage des actifs si l'un d'eux peut porter un id (décision 12, point 6) ; aucune mesure MES-xx.
- **Hors périmètre** : les vues elles-mêmes (`PAGE-DONNEES`, `PAGE-RECHERCHE`, `PAGE-ANALYTICS`) ; une couche CORS (décision 4) ; une authentification des fichiers statiques ; une clé `[ui] enabled` (décision 9) ; un routeur côté client, un bundler, Node ; le service d'un dossier lu sur le disque ; une exception de plus que `/ui` dans le test des routes.

### ROUTE-LISTER-DOCUMENTS

- **Objectif** : ajouter `GET /v1/indexes/{index}/documents` paginé, rôle `read`, déclaré dans `docs/05`, la spécification et `openapi_routes.rs` dans le même changement.
- **Dépend de** : MAQUETTE-TABLEAU-DE-BORD ; ADR-0130 `Accepté` le 2026-11-02.
- **À lire** : ADR-0130 (décisions 5, 6 et 8), ADR-0103 (complément du 2026-10-29 : forme d'une liste), ADR-0107 (lecture d'un document), ADR-0102 (limites), ADR-0111 (complément du 2026-10-30 : tableau, spécification et `openapi_routes.rs` dans le même changement), ADR-0122 (décision 9, bornes d'un `limit` de lecture), `05-api.md` (table des routes, « Lecture d'un document »), `04-donnees-et-ingestion.md` (champ `returned`), `08-invariants-et-tests.md` (INV-01).
- **Livrables** :
  - la route et son gestionnaire dans `crates/crible-server/src/http.rs`, à côté de `GET /v1/indexes/{index}/documents/{doc_id}` ;
  - les paramètres `limit` (de 1 à 1 000, 100 par défaut) et `after` (curseur **exclusif** sur `doc_id`) ; un paramètre inconnu ou hors bornes rend `400 invalid_request` en nommant le paramètre, **selon la règle du journal de debug** (ADR-0122, décision 9), seule autre route de lecture paginée ;
  - le corps `{ "documents": [ … ] }`, même forme de liste qu'`IndexList` (ADR-0103) ;
  - les seuls champs **`returned`** du schéma ; **aucun texte par défaut** ; aucun `iid`, aucune clé de passage interne (décisions 6 et 8) ;
  - l'ordre croissant des `doc_id`, **stable et documenté** (décision 6, « À surveiller ») ;
  - la **ligne du tableau de `docs/05`**, l'**opération OpenAPI** et l'exemple `<!-- openapi: DocumentList -->` validé par le test des exemples, **dans le même changement** (ADR-0111 ; ADR-0130, décision 6). `docs/05` n'ajoute aucune ligne à ce tableau avant cette tâche, et n'ajoute aucune exception au test ;
  - la section « Lecture d'un document » de `docs/05` complétée par la liste paginée ;
  - un test de la route dans `crates/crible-server/tests/`.
- **Critères d'acceptation** :
  - une liste paginée rend les documents par `doc_id` croissant ; `after` exclut le curseur, et la page suivante reprend exactement là où la précédente s'arrête ;
  - `limit` hors de `[1, 1 000]`, `after` inconnu et paramètre inconnu rendent `400 invalid_request` nommant le paramètre ; un index inconnu rend `404 unknown_index` ; un index sans document rend `200` avec une liste vide ;
  - une clé `read` suffit ; une clé absente ou inconnue rend `401` ;
  - la réponse ne porte ni texte, ni vecteur, ni `iid`, ni clé de passage interne ;
  - un document renvoyé par la liste s'ouvre par la route de document ;
  - le test `openapi_routes` compare le tableau de `docs/05` et la spécification dans les deux sens, et reste vert ;
  - `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` et `cargo test` passent.
- **Invariants / mesures** : INV-01, par un test `inv_01_*` sur la réponse de la route ; aucune mesure MES-xx.
- **Hors périmètre** : toute recherche ou tout filtre dans la liste (les seuls paramètres sont `limit` et `after`) ; le texte par défaut, qui reste à la route de document avec `?text=true` ; un tri par un autre champ que `doc_id` ; la modification d'ADR-0103, d'ADR-0107 ou d'ADR-0111.

### PAGE-DONNEES

- **Objectif** : la vue qui parcourt les index et les documents, avec une clé `read`.
- **Dépend de** : SERVIR-UI-STATIQUE, ROUTE-LISTER-DOCUMENTS.
- **À lire** : ADR-0130 (décisions 1, 4, 5, 7 et 8), `05-api.md` (« Administration des index », « Lecture d'un document » et la liste paginée), `02-architecture.md` (« Identifiants »), `08-invariants-et-tests.md` (INV-01).
- **Livrables** :
  - la vue **liste des index**, depuis `GET /v1/indexes` ;
  - la vue d'un **index**, depuis `GET /v1/indexes/{index}` : configuration, comptes, époque, **génération courante**, **nombre de segments** ; ni historique des générations, ni liste des segments, ni tailles de pool (décision 8) ;
  - la **liste paginée des documents**, depuis `GET /v1/indexes/{index}/documents`, avec un bouton de page suivante qui porte `after` ;
  - la vue d'un **document** et de ses passages, depuis `GET /v1/indexes/{index}/documents/{doc_id}`, avec `?text=true` sur demande ;
  - le champ de **clé d'API** en `sessionStorage`, jamais dans l'URL, jamais dans `localStorage` (décision 4) ; un `401` et un `403` sont annoncés à l'écran, avec le code et le message du corps d'ADR-0101.
- **Critères d'acceptation** :
  - avec une clé `read`, la vue liste les index, ouvre un index, liste les documents, ouvre un document et lit ses passages (critère 2 de J9) ;
  - aucun `iid`, `segment_id` ni `iid_start` n'apparaît à l'écran ni dans le DOM (critère 5 de J9) ;
  - la page n'appelle pas `/v1/stats` et n'affiche aucune donnée d'analytics ;
  - la pagination avance par `after` et s'arrête proprement à la fin, sans doublon ni trou ;
  - la clé n'apparaît ni dans l'URL, ni dans `localStorage`, ni dans un log du navigateur contrôlé par la page ;
  - `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` et `cargo test` passent.
- **Invariants / mesures** : INV-01 ; aucune mesure MES-xx.
- **Hors périmètre** : toute action d'administration — création, `PATCH`, suppression, attache de lot, compaction, export, instantané (décision 1) ; le journal de debug (décision 7) ; les tailles de pool (décision 8) ; l'historique des générations et la liste des segments.

### PAGE-RECHERCHE

- **Objectif** : le bac à sable de recherche, mono et multi-index, avec ses panneaux de diagnostic.
- **Dépend de** : SERVIR-UI-STATIQUE.
- **À lire** : ADR-0130 (décisions 1, 4, 5, 7 et 10), `05-api.md` (« Recherche : requête », « Recherche : réponse », `POST /v1/search`), `03-recherche.md` (modes et fusion), ADR-0119 (`debug`), ADR-0123 (`explain_ids`), ADR-0121 et ADR-0122 (journal de debug), `08-invariants-et-tests.md` (INV-01, INV-13).
- **Livrables** :
  - un formulaire avec les champs de `docs/05` : `mode` (`dense`, `lexical`, `hybrid`), `query`, `vector` quand le mode l'exige, `filter` en JSON, `k`, `fusion`, `group_by`, `passages_per_document`, `dedup` et `return` ;
  - la cible **mono-index** (`POST /v1/indexes/{index}/search`) et **multi-index** (`POST /v1/search`, un `filter` par cible) ;
  - l'affichage des résultats avec les **mêmes clés que l'API** : `doc_id`, `passage`, `key`, `score`, `score_type`, champs et texte ;
  - les panneaux **`debug`** (plan, temps, rangs) et **`explain`** (`explain_ids`, forme d'ADR-0123), rendus tels que l'API les donne, sans recalcul ;
  - un **avertissement avant la case `debug`** : `debug: true` écrit la requête et le filtre dans le journal de debug quand celui-ci est activé (décision 7 ; ADR-0121, décision 4 ; ADR-0122, décisions 1 et 2).
- **Critères d'acceptation** :
  - une recherche `dense`, une `lexical` et une `hybride` avec filtre et `k`, en mono et multi-index, rendent les **mêmes clés de résultat** que la même requête en `curl` (critère 3 de J9) ;
  - les panneaux `debug` et `explain` s'affichent et ne portent aucun `iid` (critère 5 de J9) ;
  - l'avertissement du journal de debug est visible **avant** la case à cocher ;
  - la page n'appelle pas `GET /v1/debug/queries` (décision 7) ;
  - un filtre invalide affiche `invalid_filter` avec son `details.path`, sans casser la page ;
  - le texte et le filtre saisis par l'utilisateur peuvent s'afficher : INV-13 ne vise que `analytics.sqlite` (décision 7) ;
  - `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` et `cargo test` passent.
- **Invariants / mesures** : INV-01 (aucun id interne dans le DOM, `debug` et `explain` compris) ; INV-13 rappelé pour le seul contenu affichable ; aucune mesure MES-xx.
- **Hors périmètre** : une page de journal de debug (décision 7) ; l'enregistrement de requêtes favorites ; l'export d'un résultat ; toute écriture par l'interface (décision 1) ; un réglage de la configuration de l'index.

### PAGE-ANALYTICS

- **Objectif** : la vue des chiffres de `GET /v1/stats`, réservée à une clé `admin`, avec un message explicite pour une clé `read`.
- **Dépend de** : SERVIR-UI-STATIQUE.
- **À lire** : ADR-0130 (décisions 5, 8, 11 et 12), ADR-0124 (décisions 1 à 11 et 16), `05-api.md` (« Statistiques »), `06-observabilite.md` (« Lecture » `:68-91`), `08-invariants-et-tests.md` (INV-13).
- **Livrables** :
  - les seaux et les totaux de `/v1/stats` : `count`, `errors`, `zero_results_rate`, `latency_ms` (p50, p95 et p99 par étape), `top_filter_shapes`, `top_indexes`, `indexes` et `analytics_dropped` ;
  - les paramètres `from`, `to`, `index`, `operation` et `group_by` ;
  - des **SVG écrits à la main**, construits en JavaScript depuis le JSON, sans bibliothèque (décision 11) ;
  - pour une clé `read`, un message explicite sur le `403` qui dit qu'une clé `admin` est nécessaire : **aucun graphique vide, aucun zéro trompeur** (décision 5) ;
  - aucune taille de pool, écartée de la route comme de la page (décision 8 ; ADR-0124, décision 4 inchangée).
- **Critères d'acceptation** :
  - avec une clé `admin` : seaux, p50/p95/p99 par étape, erreurs par code et formes de filtre s'affichent (critère 4 de J9) ;
  - avec une clé `read` : la page annonce le `403` et n'affiche **aucune** donnée (critère 4 de J9) ;
  - les chiffres affichés sont ceux du corps de `/v1/stats` pour la même fenêtre, sans recalcul de percentile dans la page ;
  - aucune bibliothèque de graphiques et aucun hôte externe : les SVG sont construits dans le document (décisions 3 et 11) ;
  - la page n'affiche aucun texte de requête, aucune valeur de filtre, aucun id de résultat (INV-13) ;
  - `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` et `cargo test` passent.
- **Invariants / mesures** : INV-13 — la page ne rend que les membres de `/v1/stats`, dont les formes de filtre ne portent que des champs et des opérateurs. Aucun test de navigateur n'est ajouté (décision 12) : la vérification est le balayage des actifs et l'inspection à la démo de J9. Aucune mesure MES-xx.
- **Hors périmètre** : les percentiles de taille de pool (décision 8) ; `GET /v1/debug/queries` (décision 7) ; un changement du rôle de `/v1/stats` (ADR-0124, décision 16 inchangée) ; toute modification d'ADR-0124.

### EPROUVER-UI

- **Objectif** : prouver ce qui est testable sans navigateur, et consigner honnêtement ce qui ne l'est pas.
- **Dépend de** : SERVIR-UI-STATIQUE, ROUTE-LISTER-DOCUMENTS, PAGE-DONNEES, PAGE-RECHERCHE, PAGE-ANALYTICS.
- **À lire** : ADR-0130 (décisions 7 et 12, critères de J9 à la décision 13), les fiches des cinq tâches précédentes, `08-invariants-et-tests.md` (INV-01), `crates/crible-server/tests/openapi_serving.rs` (`:105-140`), `05-api.md` (routes appelées par les pages).
- **Livrables** :
  - les tests de `crates/crible-server/tests/ui_serving.rs`, complets : en-têtes, MIME et `charset`, `404`, balayage des actifs, cohérence des actifs, exception `/ui` (décision 12, points 1 à 5) ;
  - un **parcours de bout en bout sur `synth-small`**, par l'API seule — les mêmes appels que la page émet : liste des index, liste des documents, document, recherche `dense`, `lexical` et `hybride`, `debug` et `explain`, `/v1/stats` avec une clé `admin` et avec une clé `read` ;
  - un relevé **indicatif** de la taille des actifs embarqués (ADR-0130, « À surveiller ») ;
  - une section « **Non testable sans navigateur** » dans le compte rendu : CSP appliquée par le navigateur, rendu visuel, graphiques SVG, `sessionStorage`, chaque point dit **non vérifié** et jamais « vérifié » ;
  - l'inspection du DOM faite à la main à la démo de J9, consignée.
- **Critères d'acceptation** :
  - les six points de la décision 12 sont couverts par un test **nommé** ;
  - aucun `iid`, aucun `segment_id` et aucun `iid_start` dans un actif ni dans une réponse utilisée par la page (critère 5 de J9) ;
  - `cargo build` suffit à produire la page ; la CI ne gagne aucune étape ; `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` et `openapi_routes` sont verts (critère 6 de J9) ;
  - le parcours de bout en bout passe sur `synth-small` avec une clé `read` et une clé `admin` ;
  - ce qui n'est pas testable sans navigateur est listé comme tel, sans être présenté comme vérifié.
- **Invariants / mesures** : INV-01 ; aucune mesure MES-xx.
- **Hors périmètre** : un testeur JavaScript, un navigateur piloté, Node ; toute étape de CI nouvelle ; une modification des pages pour faire passer un test ; une affirmation sur le rendu qui ne vient pas de la démo de J9.

## Hors périmètre de P9

ADR-0130, décision 14, et renvoi de ces points aux « Idées » du suivi :

- le **contenu visuel exact** : la maquette validée par le propriétaire le fige (`MAQUETTE-TABLEAU-DE-BORD`) ;
- l'**historique des générations** : seule la génération courante est affichée (décision 8) ;
- la **liste des segments** : seul leur nombre est affiché, une liste rouvrirait INV-01 par `iid_start` (décision 8) ;
- la **page du journal de debug** : `GET /v1/debug/queries` n'est pas exposé par l'interface (décision 7) ;
- les **percentiles de taille de pool** : retirés de la page, comme de la route (décision 8 ; ADR-0124, décision 4) ;
- toute **dépendance nouvelle**, Rust ou JavaScript (décision 3 et 11) ;
- toute **action d'administration** par l'interface : création, `PATCH`, suppression, attache, compaction, export, instantané (décision 1) ;
- toute **modification d'ADR-0124** : sa décision 16 (analytics `admin`) et sa décision 4 (pas de percentiles de pool) restent en vigueur.
