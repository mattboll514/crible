# ADR-0130 — Tableau de bord intégré (phase P9)

- **Statut** : Proposé
- **Date** : 2026-11-02
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, sur demande du propriétaire du 2026-11-02 (conversation avec l'orchestrateur), à partir de l'analyse de `@architecte` du 2026-11-02 ; travail hors tâche, aucune tâche du plan ne le porte
- **Remplace** : — (ne remplace aucun ADR ; **dépend d'ADR-0129**, `Proposé`, qui réserve P9 au tableau de bord et renvoie son contenu au présent ADR. Les deux ADR s'acceptent ensemble : ADR-0130 sans ADR-0129 laisserait le tableau de bord dans une phase P9 qui est encore la migration de Legifacile, et ADR-0129 sans ADR-0130 laisserait P9 sans contenu ni jalon)

## Contexte

### Ce qui est demandé, et pourquoi un ADR

Le propriétaire veut un **tableau de bord visuel** pour regarder les données et les analytics de Crible : parcourir les index et les documents, essayer une recherche, lire les statistiques d'usage. Le style est arrêté : **très épuré**, avec pour référence le site https://anara.com/ — beaucoup de blanc, typographie soignée, peu de couleurs. Le détail visuel sera figé par une maquette que le propriétaire valide, première tâche de la phase.

Cette fonctionnalité n'existe dans aucun document de conception. Pire, `docs/01-vision-et-perimetre.md:54` la classe **hors v1**, avec la justification « Les stats passent par l'API et la CLI ». L'ajout est donc un **écart de périmètre**, décidé par le propriétaire le 2026-11-02 : la ligne quitte le tableau du hors-périmètre. Un ADR accepté peut ajouter une fonctionnalité ; il ne peut pas laisser cette ligne en place.

La place de la fonctionnalité vient d'ADR-0129. Sa décision 5 réserve **P9** au tableau de bord, avec un fichier `phases/P9-tableau-de-bord.md` et le jalon **J9**, et renvoie explicitement le contenu et les critères au présent ADR. Sa décision 11 range parmi « ce qui n'est pas décidé ici » le contenu et les critères du tableau de bord. Rien de ce qui suit n'est exécutable avant que les deux ADR soient acceptés : une tâche de P9 dépendrait sinon d'un ADR `Proposé`, ce que `docs/09-plan-de-developpement.md:5 bis` et `docs/adr/README.md:23` interdisent.

### L'état des documents avant cet ADR

| Élément | Contenu actuel | Référence |
|---|---|---|
| Périmètre | « Interface web, tableau de bord » **hors v1** | `docs/01:54` |
| Interfaces | API REST, CLI, client Python | `docs/01:44-48` |
| Phases | `P0 → … → P8 → P9` ; P9 = migration de Legifacile | `docs/09:5`, `:79`, `:84` |
| Routes servies | `/health`, `/ready`, `/v1/indexes`, `/v1/indexes/{index}`, `/v1/indexes/{index}/documents/{doc_id}`, recherches, jobs, `/v1/stats`, `/v1/debug/queries`, `/openapi.json`, `/docs` | `crates/crible-server/src/http.rs:92-93`, `:136-148`, `:153`, `:158`, `:168-169` ; `docs/05:60-79` |
| Routes de liste des documents | **aucune** | — |
| Statique déjà servi | Swagger UI sous `/docs`, sans clé | ADR-0111, décision 4 |
| `tower-http` | features `trace`, `limit`, `catch-panic` ; **pas** `fs` | `docs/07:34`, `Cargo.toml:34-39`, ADR-0096 |
| Analytics | `GET /v1/stats`, rôle **`admin`** | ADR-0124, décision 16 |
| Journal de debug | `GET /v1/debug/queries`, rôle `admin`, court, désactivé par défaut ; `debug: true` **écrit toujours** quand le journal est activé | ADR-0121, ADR-0122 |
| Confidentialité | INV-01 : aucun iid hors du moteur ; INV-13 : analytics sans contenu | `docs/08:9`, `:21` |

### Ce que l'API permet déjà, et ce qui manque

L'analyse de `@architecte` confronte les quatre fonctions demandées aux routes réellement servies.

| Fonction | Route | Verdict |
|---|---|---|
| index, config, comptes, tailles, époque, génération courante, nombre de segments, dernier lot | `GET /v1/indexes`, `GET /v1/indexes/{index}` (`docs/05:61-62`, corps `IndexList` et `IndexDetail`) | `CONFORME` |
| document et ses passages | `GET /v1/indexes/{index}/documents/{doc_id}` (`docs/05:67`) | `CONFORME`, mais **seulement si le `doc_id` est connu** |
| parcourir la liste des documents | — | `ECART` : aucune route de liste, ni dans `docs/05`, ni dans `http.rs` |
| historique des générations | — | `NON-COUVERT` |
| liste des segments | — | `ECART` avec `docs/05:129` et INV-01 |
| bac à sable (`mode`, `filter`, `k`, `debug`, `explain_ids`) | `POST /v1/indexes/{index}/search`, `POST /v1/search` (`docs/05:70-71`) | `CONFORME` |
| volumes, latences, erreurs, par index et période | `GET /v1/stats` (`docs/05:75`, ADR-0124, décisions 1 et 2) | `CONFORME` pour la forme |
| tailles de pool | — | `ECART` : `docs/06:82` et ADR-0124, décision 4 excluent explicitement les percentiles de pool |
| analytics avec une clé `read` | `GET /v1/stats` est `admin` (ADR-0124, décision 16) | `ECART` avec « protégées par les clés d'API existantes » |

Le tableau de bord ne peut pas lire ce que l'API ne rend pas. Il faut donc trancher trois choses : la route de liste des documents, le niveau de clé des analytics, et le sort des informations qui existent sur disque mais ne sont pas exposées (générations, segments, tailles de pool).

### Le service de fichiers statiques

`docs/07:34` nomme `tower-http` sans la feature `fs`, et `Cargo.toml:34-39` n'active que `trace`, `limit` et `catch-panic` ; ADR-0096 énumère les features consignées, et `fs` n'y figure pas. `rust-embed` n'est pas dans `docs/07`. Servir un dossier depuis le disque ou embarquer des fichiers par une crate nouvelle demande donc un ADR. Le seul précédent de statique servi est Swagger UI, embarqué par la feature `vendored` d'`utoipa-swagger-ui` (ADR-0111).

### L'authentification dans un navigateur

Le transport est acquis : une seule forme, `Authorization: Bearer <clé>` (`docs/05:6`, ADR-0100, décision 6) ; les clés sont stockées hachées, la clé en clair n'existe qu'à la génération (`docs/05:10`, ADR-0100, décisions 2 et 3) ; ni la clé ni l'en-tête n'entrent dans un log (ADR-0100, décision 9, INV-13). Ce qui n'est tranché nulle part : où le navigateur garde la clé saisie, à quel rôle chaque page a droit, et si les fichiers statiques sont servis sans clé. `docs/` ne traite pas CORS du tout.

### La confidentialité

INV-01 interdit qu'un `iid` sorte du moteur. Le tableau de bord ne peut afficher que ce que l'API rend : l'`iid`, le `segment_id` — qui encode le premier iid d'un segment compacté (`c3-0000000000`, `docs/02:108`, ADR-0115) — et le `iid_start` du manifeste (`docs/02:90`) sont interdits à l'écran et dans le DOM.

`debug` et `explain` ne rendent aucun iid (`docs/05:340-350`, ADR-0119, décision 2, ADR-0123, décision 9) : ils sont affichables tels quels. INV-13 vise `analytics.sqlite`, donc le bac à sable peut afficher le texte et le filtre **de l'utilisateur**. En revanche, `debug: true` **écrit toujours** la requête et le filtre dans `debug.sqlite` quand le journal est activé (ADR-0121, décision 4 ; ADR-0122, décisions 1 et 2). L'interface doit le dire avant la case à cocher.

Le journal de debug est le seul endroit où apparaît le texte des requêtes d'autrui (`docs/06:104-106`, ADR-0121, décision 5). L'exposer dans la page serait une seconde décision, pas une conséquence du tableau de bord.

### L'activation, les graphiques et les tests

Trois questions restent ouvertes. Faut-il une clé de configuration pour activer le tableau de bord ? La règle 12 des `AGENTS.md` répond d'avance : « pas de configuration pour une valeur fixe », et `/docs` est servi sans condition (`crates/crible-server/src/http.rs:88-90`). Faut-il une bibliothèque de graphiques ? La CSP stricte interdit le code distant, et le précédent `vendored` d'ADR-0111 a coûté cher pour Swagger UI. Enfin, comment tester une interface sans ajouter Node à la CI, alors que `docs/07` décrit une CI sans étape de build front ?

### La question

Comment ajouter un tableau de bord visuel, intégré au serveur, sans dépendance nouvelle, sans étape de build, sans élargir la surface d'authentification ni de confidentialité, et sans ajouter à l'API plus que ce que les trois fonctions demandent ?

## Décision

**Un tableau de bord visuel en lecture seule est intégré à `crible serve`, servi sous `/ui` dans la même image, en HTML/CSS/JS sans framework ni build, en anglais, avec trois fonctions : parcourir les données, tester une recherche, voir les analytics.** Il est servi sans clé de configuration et sans clé d'API ; l'API qu'il appelle reste fermée. Une seule route d'API est ajoutée, la liste paginée des documents. Aucune dépendance Rust ni JavaScript n'est ajoutée.

### Décision 1 — objet, place et périmètre

Le tableau de bord sert à **visualiser les données et les analytics** de Crible. Il est placé en **phase P9**, après P8 et avant P10, conformément à ADR-0129 (décision 5). Son jalon est **J9**.

La ligne « Interface web, tableau de bord » est **retirée** du hors-périmètre de `docs/01:54`, et les « Interfaces » de `docs/01:44-48` mentionnent le tableau de bord intégré. C'est un écart de périmètre assumé, décidé par le propriétaire le 2026-11-02.

Le tableau de bord est **en lecture seule**. Il n'expose aucune action d'administration : ni création, ni `PATCH`, ni suppression d'index, ni attache de lot, ni compaction, ni export, ni instantané. Ces gestes restent à l'API et à la CLI, avec une clé `admin` ou `write`.

Il porte **trois fonctions** :

1. **Parcourir les données** : index, configuration, comptes, documents et passages, génération courante, nombre de segments ;
2. **Tester une recherche** : bac à sable dense, lexical et hybride, avec filtre, `k`, mono et multi-index, et les panneaux `debug` et `explain` ;
3. **Voir les analytics** : les chiffres de `GET /v1/stats`.

### Décision 2 — intégré au serveur, sous `/ui`

Les pages sont **servies par `crible serve` lui-même**, sous `/ui`, dans la **même image Docker**. Il n'y a pas d'application séparée, pas de second processus, pas de reverse proxy dédié, pas de port supplémentaire.

Le préfixe `/ui` suit le précédent de `/docs` (ADR-0111, décision 4) : une ressource statique servie par l'instance, hors de l'espace `/v1` de l'API.

### Décision 3 — service des fichiers : `include_bytes!` / `include_str!` et un handler axum

Les actifs (HTML, CSS, JS, favicon éventuel) sont **embarqués dans le binaire** par `include_bytes!` et `include_str!`, puis servis par un **handler axum** écrit à la main. Les fichiers vivent sous `crates/crible-server/src/ui/`.

Cette voie n'ajoute **aucune dépendance et aucune feature** : ni la feature `fs` de `tower-http`, ni `rust-embed`, ni `include_dir`. Le type MIME est écrit à la main pour les quatre extensions retenues : `.html`, `.css`, `.js`, `.svg`. Le build ne télécharge rien, ne lit aucun dossier à l'exécution et ne dépend pas du répertoire courant.

En-têtes fixés par cet ADR, sur chaque réponse `/ui` :

- `Content-Type` avec `charset=utf-8` ;
- `Cache-Control: no-cache` et `ETag` (le contenu est embarqué, donc stable pour une version du binaire) ;
- `Content-Security-Policy` stricte : `default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'` ;
- `X-Content-Type-Options: nosniff` ;
- `Referrer-Policy: no-referrer`.

Cette CSP n'est tenable que si le document ne porte **aucun script ni style en ligne** : seulement des `<link rel="stylesheet" href="/ui/…">` et des `<script src="/ui/…">`. Aucun hôte externe n'est appelé : ni CDN, ni police distante, ni image distante. Une police est soit une police système, soit un fichier embarqué.

### Décision 4 — authentification du navigateur

La clé d'API est **saisie par l'utilisateur** dans la page et gardée en **`sessionStorage`** ; elle est effacée à la fermeture de l'onglet. Elle n'apparaît **jamais** dans l'URL, dans un fragment, dans un log ou dans `localStorage`.

Les **fichiers statiques sont servis sans clé**, comme `/docs` (`docs/05:80`, ADR-0111, décision 4) : la page ne porte aucune donnée. L'API reste fermée ; chaque appel du JavaScript porte `Authorization: Bearer <clé>` et reçoit `401` ou `403` comme tout client.

Le tableau de bord et l'API sont servis par la **même origine**. Aucune couche CORS n'est ajoutée, et l'absence d'`Access-Control-Allow-Origin` est le défaut sûr. Ajouter CORS serait une décision, pas une commodité.

### Décision 5 — rôles : `read` pour parcourir et chercher, `admin` pour les analytics

Parcourir les données et tester une recherche fonctionnent avec une clé **`read`** (`docs/05:11-13`) : `GET /v1/indexes`, `GET /v1/indexes/{index}`, `GET /v1/indexes/{index}/documents`, `GET /v1/indexes/{index}/documents/{doc_id}`, `POST /v1/indexes/{index}/search`, `POST /v1/search`.

La page analytics exige une clé **`admin`**, conformément à ADR-0124, décision 16, **inchangée**. Avec une clé `read`, la page **l'annonce** et n'affiche aucune donnée : aucun graphique vide, aucun zéro trompeur, mais un message explicite sur le `403` et l'indication qu'une clé `admin` est nécessaire.

### Décision 6 — nouvelle route `GET /v1/indexes/{index}/documents`

Une route de liste est ajoutée, parce que sans elle le parcours des données se limite aux `doc_id` déjà connus :

```
GET /v1/indexes/{index}/documents?limit=&after=
```

- **Rôle** : `read`.
- **Corps** : `{ "documents": [ … ] }`, même forme de liste qu'`IndexList` (ADR-0103, complément du 2026-10-29 : un objet à une clé, le nom pluriel de la ressource).
- **Pagination** : par `doc_id`, curseur `after` exclusif, `limit` borné par la même règle que les autres routes de lecture. L'ordre est celui des `doc_id`, stable et documenté.
- **Champs** : ceux que l'index déclare `returned`. **Aucun texte par défaut** ; le texte s'obtient par la route de document, avec `?text=true`.
- **Aucun id interne** : ni `iid`, ni clé de passage interne dans la liste.

La route est **déclarée dans `docs/05`, dans la spécification OpenAPI et dans `openapi_routes.rs` dans le même changement** (ADR-0111, complément du 2026-10-30). Son exception nommée dans le test des routes ne peut donc pas exister : la ligne du tableau et l'opération OpenAPI arrivent ensemble.

### Décision 7 — confidentialité de l'écran

Aucun `iid`, aucun `segment_id` et aucun `iid_start` n'apparaît **à l'écran ni dans le DOM**. Les objets rendus par l'API n'en portent pas ; le tableau de bord n'en fabrique pas et ne lit pas les fichiers qui en contiennent.

Le panneau `debug` **prévient, sur la page et avant la case à cocher**, que `debug: true` écrit la requête et le filtre dans le journal de debug quand celui-ci est activé (ADR-0121, décision 4 ; ADR-0122, décisions 1 et 2).

La page **ne lit pas le journal de debug** : `GET /v1/debug/queries` n'est pas exposé par le tableau de bord. Une page « journal » serait une seconde décision.

### Décision 8 — périmètre des informations affichées

- **Génération** : seule la **génération courante** est affichée, celle que rend déjà `GET /v1/indexes/{index}`. L'historique des générations n'est pas exposé en P9, et aucune route n'est ajoutée pour lui.
- **Segments** : seul le **nombre** est affiché, déjà rendu par `IndexDetail`. Aucune route de liste des segments n'est ajoutée : une époque compactée encode `iid_start` dans le `segment_id` (`docs/02:108`, ADR-0115), donc une liste rouvrirait INV-01 et `docs/05:129` pour un gain nul.
- **Tailles de pool** : **retirées** de la page. `docs/06:82` et ADR-0124, décision 4 excluent les percentiles de pool de la réponse de `/v1/stats`, et cette décision reste inchangée.

### Décision 9 — activation : toujours servi, sans clé de configuration

Le tableau de bord est **toujours servi**, sans clé de configuration, comme `/docs`. La règle 12 des `AGENTS.md` — « pas de configuration pour une valeur fixe » — s'applique : la surface ajoutée est de quelques kilo-octets statiques, et Crible n'est jamais exposé publiquement (`docs/07`, « Compose d'exemple »).

Aucune clé `[ui] enabled`, aucune surcharge `CRIBLE_*`, aucun état de plus à décrire ni à tester.

### Décision 10 — HTML/CSS/JS sans framework ni étape de build, interface en anglais

Le tableau de bord est écrit en **HTML, CSS et JavaScript simples**, sans framework, sans transpilation, sans bundler, sans Node. Le binaire suffit à le produire : `cargo build` est la seule étape. La CI ne gagne **aucune étape**.

L'**interface est en anglais**, comme l'API et le code (`AGENTS.md`, règle 8). Les documents de `docs/` restent en français ; l'ADR ne change pas cette règle, il l'applique à une interface utilisateur.

### Décision 11 — graphiques : SVG écrit à la main

Les graphiques sont des **SVG écrits à la main**, construits en JavaScript depuis le JSON de `GET /v1/stats`. Aucune bibliothèque de graphiques, ni embarquée, ni distante. Les barres et les courbes suffisent à rendre `buckets`, `totals`, `latency_ms` et `errors`.

Cette voie ne coûte aucune dépendance, aucune licence à auditer, aucun octet vendu dans le dépôt, et elle est compatible avec la CSP `script-src 'self'` de la décision 3.

### Décision 12 — tests en Rust seulement

Les tests du tableau de bord vivent dans `crates/crible-server/tests/`, sur le précédent de `openapi_serving.rs:105-140`. Aucun testeur JavaScript, aucun Node, aucun runner ajouté.

1. `GET /ui` et **chaque actif** : `200`, `Content-Type` attendu avec `charset=utf-8`, `Content-Security-Policy`, `X-Content-Type-Options`, `Referrer-Policy`, et **sans clé**.
2. `GET /ui/inconnu` : `404 not_found`, au format d'ADR-0101.
3. **Balayage des actifs embarqués** : aucune occurrence de `http://`, `https://`, `//cdn`, `<script>` en ligne, `<style>` en ligne, `@import`, `url(http`. C'est la preuve mécanique de « aucun hôte externe » et de la CSP.
4. **Cohérence des actifs** : chaque `href` et chaque `src` de `index.html` correspond à un fichier embarqué, et aucun fichier embarqué n'est orphelin.
5. **`openapi_routes.rs`** : `/ui` ajouté aux exceptions nommées, comme `/docs` (`docs/05:80`, ADR-0111, décision 5), et **aucun chemin `/ui` dans la spécification**.
6. **`inv_01_*`** étendu si un actif peut contenir un id : un balayage suffit, aucun actif n'est généré.

Ce qui n'est pas testable sans navigateur est consigné dans la fiche de la tâche `EPROUVER-UI`, sans être présenté comme vérifié.

### Décision 13 — tâches de P9 et critères de J9

Les fiches sont écrites à l'application, après acceptation d'ADR-0129 et d'ADR-0130.

| # | Tâche | Objectif | Dépend de |
|---|---|---|---|
| 1 | `MAQUETTE-TABLEAU-DE-BORD` | Figer la maquette des quatre vues et les règles visuelles (référence anara.com : blanc, typographie, peu de couleurs), validée par le propriétaire | — |
| 2 | `SERVIR-UI-STATIQUE` | Embarquer les actifs, servir `/ui` avec MIME, cache et en-têtes de sécurité, sans clé et sans dépendance | 1 |
| 3 | `ROUTE-LISTER-DOCUMENTS` | Ajouter `GET /v1/indexes/{index}/documents` paginé, rôle `read`, déclaré dans `docs/05`, la spécification et `openapi_routes.rs` | 1, ADR-0130 accepté |
| 4 | `PAGE-DONNEES` | Page index et documents : liste, config, comptes, époque, génération, nombre de segments, document et passages | 2, 3 |
| 5 | `PAGE-RECHERCHE` | Bac à sable : mode, requête, filtre JSON, `k`, fusion, regroupement ; panneaux `debug` et `explain` | 2 |
| 6 | `PAGE-ANALYTICS` | Volumes, latences, erreurs, formes de filtre par index et période ; message explicite sur `403` avec une clé `read` | 2 |
| 7 | `EPROUVER-UI` | Balayages d'actifs, en-têtes, exception OpenAPI, parcours de bout en bout sur `synth-small` ; consigner ce qui n'est pas testable sans navigateur | 2-6 |

**Critères de J9.**

1. Le binaire de P8 sert `/ui` : la page se charge sans clé, sans requête vers un hôte externe (onglet réseau), avec la CSP.
2. Avec une clé `read` : parcourir les index, voir config, comptes, époque, génération, nombre de segments ; lister les documents ; ouvrir un document et lire ses passages.
3. Bac à sable : une recherche `dense`, une `lexical`, une `hybride` avec filtre et `k`, en mono et multi-index ; les clés de résultat sont identiques à celles de la même requête en `curl` ; les panneaux `debug` et `explain` s'affichent.
4. Analytics avec une clé `admin` : seaux, p50/p95/p99 par étape, erreurs par code, formes de filtre ; avec une clé `read`, la page l'annonce et n'affiche aucune donnée.
5. Aucun `iid`, aucun `segment_id`, aucun `iid_start` à l'écran ni dans le DOM (test et inspection).
6. `cargo build` suffit à produire la page ; la CI ne gagne aucune étape ; `cargo fmt`, `clippy -D warnings` et `cargo test` verts ; `openapi_routes` vert avec l'exception `/ui`.

### Décision 14 — ce qui n'est pas décidé ici

- Le **contenu visuel exact** : la maquette, validée par le propriétaire, fige les quatre vues et les règles de style. Cet ADR fixe le cadre, pas le pixel.
- Les **fiches de P9** : elles sont écrites à l'application. Le code relève des tâches, pas de l'ADR.
- L'**historique des générations**, la **liste des segments**, la **page du journal de debug** et les **percentiles de taille de pool** : hors périmètre de P9, à noter en « Idées » du suivi. Chacun demanderait une route ou une décision de plus.
- Toute **dépendance nouvelle**, Rust ou JavaScript.
- Toute **action d'administration** par l'interface.
- Toute **modification d'ADR-0124** : sa décision 16 (analytics `admin`) et sa décision 4 (pas de percentiles de pool) restent en vigueur.

## Application

Rien n'est appliqué tant que le propriétaire n'a pas accepté cet ADR, et rien ne l'est avant l'acceptation d'ADR-0129, dont celui-ci dépend. À l'acceptation des deux, `@greffier` met à jour les documents suivants, en citant `ADR-0130`.

**Documents de conception**

- `docs/01-vision-et-perimetre.md` : `:44-48` (interfaces), `:54` (retirer la ligne du hors-périmètre), `:59` (J9 → J10, par ADR-0129).
- `docs/02-architecture.md` : `:43` (P9 → P10, par ADR-0129) ; le cas échéant, un paragraphe sur `/ui` dans l'organisation du dépôt.
- `docs/05-api.md` : `:60-79` (ligne `/ui` et nouvelle ligne de liste des documents), `:80` (exception nommée `/ui` dans le test des routes), section « Lecture d'un document » (`:198` et suivantes) pour la liste paginée.
- `docs/06-observabilite.md` : `:64-88` (les tailles de pool sont explicitement écartées de l'interface).
- `docs/07-stack-et-deploiement.md` : `:29-50` (aucune dépendance nouvelle), `:34` (mention de l'interface embarquée), « Docker » (actifs dans l'image), « CI » (« aucun Node, aucune étape de build »).
- `docs/08-invariants-et-tests.md` : `:57`, `:71-73`, `:86` (P9 → P10, par ADR-0129) ; INV-01 rappelé pour l'interface.
- `docs/09-plan-de-developpement.md` : `:5`, `:79`, `:84` (par ADR-0129).
- `docs/README.md` : `:33-45` (table des phases, par ADR-0129).

**Fiches de phase**

- `docs/phases/P9-tableau-de-bord.md` : à créer, fiches de la décision 13 et critères de J9.
- `docs/phases/P8-docker-et-deploiement.md` : `:147` (retirer « une interface web » de la liste du hors-périmètre).
- `docs/phases/P9-migration-legifacile.md` : renommage et scission, par ADR-0129.

**Index et suivi**

- `docs/adr/README.md` : ligne d'index d'ADR-0130 et statut.
- `docs/suivi.md` : `:18-21` (jalons J9 et J10), `:34`, table des tâches `:140-149` (nouvelle phase P9), à l'application.

**Code, hors de cet ADR** : le montage `/ui` dans `crates/crible-server/src/http.rs:87-99`, le module d'actifs et le handler, la route de liste des documents dans `crates/crible-server/src/http.rs:136-148`, un test `crates/crible-server/tests/ui_serving.rs` et l'exception `/ui` dans `openapi_routes.rs`. Ces changements appartiennent aux tâches de la décision 13.

## Conséquences

- **Positives** :
  - les données et les analytics de Crible deviennent consultables sans `curl` ni `jq` ;
  - la page se construit avec `cargo build` seul : aucune chaîne Node, aucun bundler, aucune étape de CI ;
  - aucune dépendance Rust ni JavaScript n'est ajoutée : ni la feature `fs` de `tower-http`, ni `rust-embed`, ni bibliothèque de graphiques ;
  - la CSP stricte, l'absence d'hôte externe et le balayage mécanique des actifs donnent une garantie vérifiable, pas une promesse ;
  - la clé reste en `sessionStorage`, jamais dans l'URL ni dans un log, et les fichiers statiques sont servis sans clé : la surface d'authentification ne s'élargit pas ;
  - les analytics restent `admin` (ADR-0124, décision 16) : une clé `read` ne voit aucune statistique ;
  - l'interface ne lit pas le journal de debug et n'affiche aucun id interne : INV-01 et la confidentialité du journal sont tenues ;
  - une seule route d'API est ajoutée, et elle est déclarée dans `docs/05`, la spécification et le test des routes dans le même changement.
- **Négatives / coûts** :
  - le serveur porte quelques kilo-octets de plus et un handler de statique à maintenir ;
  - le type MIME, l'`ETag` et les en-têtes de sécurité sont écrits à la main : c'est du code qui n'existerait pas avec `ServeDir` ;
  - l'interface est en anglais, ce qui ne convient pas à un utilisateur francophone non technique ;
  - le tableau de bord n'affiche ni l'historique des générations, ni la liste des segments, ni les tailles de pool, ni le journal de debug : ces manques se verront à l'usage ;
  - une nouvelle route publique (`documents`) élargit l'API et doit être tenue dans `docs/05`, la spécification et le test des routes ;
  - les tests ne couvrent pas le comportement JavaScript : une régression d'affichage ne sera vue qu'à la main, à la démo de J9 ;
  - la maquette doit être validée avant toute implémentation, ce qui ajoute une tâche au début de la phase.
- **À surveiller** :
  - **la taille des actifs** et le temps de chargement de la page : un actif qui grossit sans mesure est un coût permanent dans le binaire ;
  - **la lisibilité de la CSP** : toute nouvelle fonction qui exigerait un style ou un script en ligne doit passer par un ADR, jamais par un assouplissement discret ;
  - **la cohérence des résultats** entre la page et `curl` : le critère 3 de J9 le vérifie, et une divergence signalerait un bug de la page, pas du moteur ;
  - **le rôle des analytics** : si une clé `read` devait un jour lire `/v1/stats`, ce serait un nouvel ADR remplaçant la décision 16 d'ADR-0124, pas un contournement ;
  - **la pagination par `doc_id`** : un index dont les `doc_id` ne sont pas ordonnés de façon stable rendrait la liste imprévisible ; l'ordre est à documenter et à tester ;
  - **l'usage réel** : si le tableau de bord n'est pas utilisé après P9, il reste une surface à maintenir et la question de le retirer se posera ;
  - **la dépendance à ADR-0129** : les deux ADR s'acceptent ensemble ; en accepter un seul laisserait le plan dans un état incohérent.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| **Application séparée** (un second binaire, un serveur de fichiers, un reverse proxy) | Elle ajoute un processus, un port, une image ou un service de plus, et un second chemin d'accès aux données. Le tableau de bord est une vue du serveur, pas un service autonome ; `/docs` montre déjà que le serveur peut servir du statique. |
| **Framework avec étape de build** (Svelte, React, Vue) | Une chaîne Node, un bundler, une étape de CI et des dépendances à auditer pour quatre pages en lecture seule. La CI devrait changer, et le binaire dépendrait d'un artefact construit à part. |
| **Interface en français** | L'API, le code et les messages d'erreur sont en anglais (`AGENTS.md`, règle 8). Une interface française créerait deux vocabulaires pour les mêmes objets et un travail de traduction à maintenir. |
| **Actions d'administration dans l'interface** (créer, `PATCH`, supprimer, attacher, compacter) | Le propriétaire a fixé la lecture seule. Les gestes d'écriture ont des conséquences durables et des rôles propres (`write`, `admin`) ; les mettre derrière un bouton dans un navigateur élargirait le risque sans besoin exprimé. |
| **Ouvrir `GET /v1/stats` au rôle `read`** | Remplacerait la décision 16 d'ADR-0124 et exposerait les analytics — volumes, latences, formes de filtre — à toute clé de lecture. Le propriétaire a retenu la clé `admin` et un message explicite pour `read`. |
| **Lire les documents par `doc_id` seulement, sans route de liste** | Le parcours des données se limiterait aux identifiants déjà connus : on ne peut pas découvrir un document. La route de liste est le seul ajout d'API que les trois fonctions exigent. |
| **`tower-http` `ServeDir` avec la feature `fs`** | Ajoute une feature absente d'ADR-0096 et de `docs/07`, monte un dossier dans l'image et lit le disque à chaque requête. Le contenu embarqué est plus simple et ne peut pas manquer à l'exécution. |
| **`rust-embed`** | Une crate nouvelle à déclarer, à auditer et à versionner, une macro de build et du poids, pour un gain nul sur quatre fichiers. ADR-0096 et `docs/07` devraient être modifiés. |
| **Clé de configuration `[ui] enabled`** | Une clé de plus à valider, à surcharger et à tester, pour une valeur qui ne change pas ; la règle 12 des `AGENTS.md` l'interdit et `/docs` est déjà servi sans condition. |
| **Bibliothèque de graphiques** (Chart.js, D3, ou un fichier vendu) | Une dépendance JavaScript, une licence, une revue et du poids ; le précédent `vendored` d'ADR-0111 a montré le coût pour Swagger UI. Des SVG écrits à la main suffisent aux barres et aux courbes de `/v1/stats`. |
| **Clé gardée dans un cookie** | Crée une surface CSRF que l'en-tête `Authorization` n'a pas, et impose une politique de cookie (attributs, expiration, portée) qu'aucun document ne prévoit. |
| **Clé gardée dans `localStorage`** | Survit à la fermeture de l'onglet et laisse la clé sur un poste partagé. `sessionStorage` borne la durée de vie à la session. |
| **Route proxy sous `/ui/api`** | Ferait un second chemin d'accès aux données, avec sa propre validation, ses propres erreurs et ses propres rôles à tenir : deux APIs pour un même moteur. |
| **`<canvas>` écrit à la main** | Faisable sans dépendance, mais plus de code que SVG pour des barres et une sparkline, et moins accessible. |
| **Page du journal de debug dans le tableau de bord** | Le journal contient le texte des requêtes d'autrui (ADR-0121, décision 5). L'exposer est une décision de confidentialité distincte, pas une conséquence du tableau de bord. |
| **Liste des segments et historique des générations** | La liste des segments rouvrirait INV-01 (`iid_start` dans `segment_id`, ADR-0115) et `docs/05:129` ; l'historique demanderait une route et une lecture sur le chemin d'une requête. Le nombre de segments et la génération courante suffisent à P9. |
| **Ajouter les percentiles de pool à `/v1/stats`** | Rouvrirait la décision 4 d'ADR-0124 et la réserve du seau 0 des échecs (`docs/06:64`). Le propriétaire a retenu de les retirer de la page. |

## Références

**Documents de conception**

- `docs/00-glossaire.md`
- `docs/01-vision-et-perimetre.md` `:44-48`, `:54`, `:59-66`
- `docs/02-architecture.md` `:43`, `:56-82`, `:90`, `:108`
- `docs/03-recherche.md` `:140`, `:144`
- `docs/05-api.md` `:6`, `:8-13`, `:60-80`, `:198`, `:229-243`, `:340-350`
- `docs/06-observabilite.md` `:17`, `:24`, `:26`, `:42`, `:47`, `:64-106`
- `docs/07-stack-et-deploiement.md` `:29-50`, `:97-102`, « Docker », « CI », « Compose d'exemple »
- `docs/08-invariants-et-tests.md` `:9`, `:21`, `:57`, `:71-73`, `:86`
- `docs/09-plan-de-developpement.md` `:5`, `:79`, `:84`, `:88`
- `docs/phases/P8-docker-et-deploiement.md` `:147`
- `docs/phases/P9-migration-legifacile.md`
- `docs/README.md` `:33-45`
- `docs/suivi.md` `:18-21`, `:34`, `:140-149`

**ADR**

- ADR-0012, ADR-0015 (API REST v1), ADR-0021, ADR-0031
- ADR-0064, ADR-0065 (instantané et restauration)
- ADR-0078 (N = 5000), ADR-0086 (analyseur), ADR-0093, ADR-0094
- ADR-0095 (rôles), ADR-0096 (dépendances et features du serveur), ADR-0097 (configuration), ADR-0098, ADR-0099
- ADR-0100 (clés d'API), ADR-0101 (corps d'erreur), ADR-0102 (limites), ADR-0103 (jobs et forme des listes), ADR-0105 (administration des index)
- ADR-0107 (écriture de documents), ADR-0109, ADR-0110 (recherche multi-index), ADR-0111 (OpenAPI et Swagger UI), ADR-0112 (client Python)
- ADR-0115 (`segment_id` d'une époque compactée), ADR-0116 (lecture sans écriture)
- ADR-0117, ADR-0118 (analytics), ADR-0119 (`debug`), ADR-0121, ADR-0122 (journal de debug), ADR-0123 (`explain_ids`)
- ADR-0124 (route `/v1/stats`, décisions 4 et 16), ADR-0127, ADR-0128
- ADR-0129 (replanification de la fin du plan) — **dépendance** : les deux ADR s'acceptent ensemble

**Code**

- `crates/crible-server/src/http.rs` `:87-99`, `:136-148`
- `crates/crible-server/tests/openapi_routes.rs`, `crates/crible-server/tests/openapi_serving.rs` `:105-140`
- `Cargo.toml` `:34-39`

**Lien externe**

- Référence de style : https://anara.com/ (sobriété, blanc, typographie soignée, peu de couleurs) ; le détail est figé par la maquette validée par le propriétaire.
