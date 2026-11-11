# ADR-0146 — Routes de l'API pendant le chargement au démarrage

- **Statut** : Accepté (accepté par anticipation par le propriétaire le 2026-11-11, confirmé par le propriétaire le 2026-11-11)
- **Date** : 2026-11-11
- **Décideurs** : propriétaire
- **Proposé par** : agent `@greffier`, tâche `T06-LISTE-INDEX-AU-DEMARRAGE` (v1.1), sur la base du verdict de `@architecte` du 2026-11-11 et du ticket 06 du projet Legifacile
- **Remplace** : — (complète ADR-0099 et ADR-0105 sans modifier leur corps)

## Contexte

### Le ticket et ce qu'il constate

Le ticket 06 de Legifacile (`../legifacile/tickets-crible/06-demarrage-index-vides.md`) rapporte un fait mesuré sur l'image 1.0.0 : pendant la reprise, `/health` rend `200`, `/ready` rend `503`, et `GET /v1/indexes` rend `200 {"indexes":[]}`. Un client qui ne consulte que cette route conclut à une base vide, puis peut relancer une création, une attache ou une alerte.

**Le serveur se lie au port avant le chargement**, et c'est décidé : ADR-0099 ouvre le service sur `/health` dès la liaison, pendant que les modèles se chargent et que chaque index s'ouvre en écriture avec sa reprise (`docs/02-architecture.md:144-159`). `/ready` reste `503` jusqu'à la publication du dernier instantané. La fenêtre entre la liaison et cette publication est donc normale, et elle peut durer : la reprise d'un index reconstruit le lexical et les `codes.bin` manquants (ADR-0088, ADR-0069, ADR-0142).

### L'inventaire, mesuré dans l'arbre le 2026-11-11

Pendant cette fenêtre, la publication n'a pas eu lieu : `loaded` vaut `None` (`crates/crible-server/src/state.rs:993`). Deux groupes de routes se comportent alors mal.

**Routes qui lisent le registre d'index**, toutes avec `loaded == None` :

| Route | Réponse actuelle | Où |
|---|---|---|
| `GET /v1/indexes` | `200 {"indexes":[]}` | `indexes.rs:139`, `state.rs:730` |
| `GET /v1/indexes/{index}` | `404 unknown_index` | `indexes.rs:210`, `state.rs:718` |
| `POST /v1/indexes` | `500 internal` | `state.rs:819-823` |
| `PATCH /v1/indexes/{index}` | `404 unknown_index` | `indexes.rs:290` |
| `DELETE /v1/indexes/{index}` | `404 unknown_index` | `indexes.rs:412` |
| `POST /v1/indexes/{index}/documents` | `404 unknown_index` | `documents.rs:147` |
| `PATCH /v1/indexes/{index}/documents` | `404 unknown_index` | `documents.rs:282` |
| `GET /v1/indexes/{index}/documents/{doc_id}` | `404 unknown_index` | `documents.rs:528` |
| `POST /v1/indexes/{index}/batches` | `404 unknown_index` | `batches.rs:119` |
| `POST /v1/indexes/{index}/search` | `404 unknown_index` | `search.rs:181` |
| `POST /v1/search` | `404 not_found` nommant l'index | `search_multi.rs:547` |
| `POST /v1/indexes/{index}/compact` | `404 unknown_index` | `maintenance.rs:115` |
| `POST /v1/indexes/{index}/exports` | `404 unknown_index` | `maintenance.rs:168` |
| `POST /v1/indexes/{index}/snapshots` | `404 unknown_index` | `maintenance.rs:234` |

Chacune de ces réponses est **fausse** pendant le chargement : les index existent sur disque, ils ne sont pas encore ouverts. Un `404 unknown_index` dit qu'un index n'existe pas, et un `200` avec une liste vide dit qu'il n'y en a aucun.

**Routes qui lisent l'état publié au démarrage** :

| Route | Réponse actuelle | Où |
|---|---|---|
| `GET /v1/jobs` | `200 {"jobs":[]}` | `http.rs:211` |
| `GET /v1/jobs/{job_id}` | `404 not_found` | `http.rs:239` |
| `GET /v1/stats` | `500 internal` | `http.rs:460` |
| `GET /v1/debug/queries` | `200` avec `enabled: false` | `http.rs:290-307` |

Elles mentent de la même façon : la table des jobs est ouverte et purgée au chargement (ADR-0103, décision 5), les analytics aussi (ADR-0118), et `enabled: false` décrit une configuration qui n'est pas encore publiée.

### Ce que la conception et les ADR disent, et ne disent pas

**Rien ne décrit ces réponses avant chargement.** Le tableau des routes de `docs/05-api.md:58-78` décrit l'état servi ; les deux sondes y sont les seules à porter un état transitoire (`:77`). ADR-0099 ne parle que de `/health` et `/ready`. ADR-0105, décisions 1 et 2, décrit un registre mutable et une création synchrone, jamais un registre non encore chargé. `docs/02-architecture.md:144-159` décrit la reprise, pas ce que l'API répond pendant qu'elle dure. Le seul comportement d'état non chargé testé est celui des sondes (`crates/crible-server/tests/server.rs:311`, `the_probe_bodies_distinguish_alive_from_ready`).

Le `200` d'une liste vide n'est pas un cas documenté : c'est une valeur de repli que le code applique avant chargement par symétrie avec le registre empoisonné, alors que la justification écrite ne vaut que pour celui-ci — un registre empoisonné ne doit pas répondre `200` sans index alors que l'instance les sert encore (`state.rs:158-171`).

**C'est donc une correction de comportement**, au sens d'ADR-0143, décision 1 : la conception dit qu'une instance se lie d'abord et se déclare prête ensuite (ADR-0099), et le comportement actuel la contredit. ADR-0143, décision 1, « Conséquences pour les tickets de la v1.1 », tranche déjà ce point pour `T06` (`docs/adr/0143-politique-de-version-de-la-ligne-1-x-et-procedure-de-montee-et-de-retour-arriere.md:65`) : remplacer `200 {"indexes":[]}` par `503` pendant le démarrage n'est pas le changement d'un cas documenté, c'est une correction autorisée en mineure, et elle demande son ADR — celui-ci — qui dit pourquoi.

**Le code d'erreur existe déjà.** `overloaded` (`503`) est le code d'une ressource momentanément indisponible qu'il faut réessayer : file d'écriture pleine, limite de recherches concurrentes, verrou SQLite, verrou d'écrivain, branche lexicale indisponible (`docs/05-api.md:50`, ADR-0087, décision 3), et drainage d'arrêt avec le message « the server is shutting down » (ADR-0104). Le `Retry-After: 1` est posé une seule fois, au rendu de l'erreur (`crates/crible-server/src/error.rs:104-124`, ADR-0103, décision 9). Aucun code nouveau, aucune valeur nouvelle de `details`.

### Ce qui reste à vérifier

Le comportement exact du client Python devant un `503` sur `GET /v1/indexes` : `OverloadedError` existe et porte `retry_after` (ADR-0112), donc rien à écrire côté client, mais aucun test `server` ne couvre aujourd'hui cette route sous `503`.

## Décision

**Tant que le registre n'est pas chargé, toute route qui en dépend rend `503 overloaded` avec le message fixe « the server is starting », et le `503` naît du registre lui-même, jamais d'une couche globale.** `/health` et `/ready` ne changent pas.

### 1. Routes du registre : `503 overloaded`

Tant que la publication n'a pas eu lieu (`loaded == None`), les quatorze routes de la première table du contexte rendent `503 overloaded` :

- message fixe, en anglais et sans variable : **« the server is starting »** ;
- corps d'ADR-0101 : `{"error": {"code": "overloaded", "message": "the server is starting", "details": {}, "request_id": "<x-request-id>"}}`, `details` à `{}`, aucune clé nouvelle ;
- `x-request-id` (ADR-0098) et `Retry-After: 1` par le rendu commun de l'erreur (ADR-0103, décision 9), sans code dédié dans les gestionnaires.

Le message ne nomme ni index, ni chemin, ni durée, et ne dit pas combien d'index restent à ouvrir : la fenêtre est bornée par le chargement, pas par un compte annoncé.

### 2. Routes de l'état publié : la même règle

`GET /v1/jobs`, `GET /v1/jobs/{job_id}`, `GET /v1/stats` et `GET /v1/debug/queries` suivent exactement la même règle, avec le même code, le même message et le même corps. Elles lisent un état ouvert au chargement — la table des jobs, les analytics, la configuration du journal — et une réponse qui affirme « aucun job », « aucun journal » ou qui échoue en `500` décrit un état qui n'existe pas encore.

Le paramètre `enabled: false` de `GET /v1/debug/queries` reste réservé au journal **réellement désactivé** par la configuration publiée (ADR-0122).

### 3. Le `503` naît à l'étape « existence de l'index », jamais dans une couche

Le refus est prononcé à l'accès au registre : lecture d'un index, lecture de la liste, création d'un index. Concrètement, `AppState::index` et `AppState::indexes` rendent `overloaded` quand `loaded` vaut `None` au lieu de `Ok(None)` et `Ok(Vec::new())`, et `AppState::create_index` rend `overloaded` au lieu de `internal`.

La règle est **locale** et n'est pas un middleware global, pour deux raisons écrites :

- l'**ordre des validations** d'ADR-0110 est conservé : une forme de corps invalide rend `400 invalid_request` avant que l'existence de l'index ne soit examinée (`docs/05-api.md:398`, ADR-0110, ordre (a) puis (b)). Un `503` posé en couche passerait avant ce `400` et changerait un cas documenté ;
- l'**authentification passe avant**, comme aujourd'hui : un `401` ou un `403` reste rendu sur une instance en cours de démarrage (ADR-0100).

### 4. `/health` et `/ready` inchangés

ADR-0099 reste la seule règle des sondes : `/health` rend `200` dès la liaison, `/ready` rend `503` jusqu'à la publication du dernier instantané puis `200`, et une instance sans index est prête.

**Le `503` de ces routes ne se décide jamais sur `is_ready()`.** Pendant le drainage d'arrêt, `/ready` passe à `503` alors que les lectures continuent d'être servies (ADR-0104) : une règle fondée sur `is_ready()` refuserait les lectures à l'arrêt et changerait l'arrêt décidé. La règle porte sur `loaded`, qui reste publié jusqu'à la sortie du processus.

### 5. Registre empoisonné : `500 internal` inchangé

Un registre empoisonné — un verrou de lecture empoisonné — reste `500 internal`, avec son message distinct (« the index registry is poisoned »), et il est une **branche distincte** dans le code, nommée et testée pour elle-même. La justification écrite de `state.rs:158-171` reste vraie : un registre empoisonné alors que l'instance sert encore ses index est une panne, pas un démarrage.

### 6. Chargé et vide : les `200` restent

Une instance **chargée** qui ne sert aucun index rend `200 {"indexes":[]}` sur `GET /v1/indexes` et `200 {"jobs":[]}` sur `GET /v1/jobs` : elle est prête (ADR-0099), et une liste vide y est vraie. De même, un `404 unknown_index` rendu **après** chargement reste celui d'un index réellement inconnu.

C'est ce cas qui borne le correctif : la modification ne change que l'état « pas encore chargé » et ne touche ni la forme des corps, ni les routes, ni les codes d'un index inconnu.

### 7. Documentation dans le même changement que le code

- `docs/05-api.md:50` (ligne `overloaded`) : ajouter la cause « registre d'index ou état publié pas encore chargé au démarrage » avec le message fixe.
- `docs/05-api.md:56-58` (sous « Endpoints ») : un paragraphe qui pose la règle — quatorze routes du registre et quatre routes de l'état publié rendent `503 overloaded` jusqu'à la publication, `/health` et `/ready` exceptés — et qui écrit le cas « chargé et vide ».
- `docs/02-architecture.md:173` (« Registre d'index mutable ») : une phrase disant que le registre n'est publié qu'à la fin du chargement, et que les routes qui le lisent rendent `503 overloaded` d'ici là.
- OpenAPI (ADR-0111) : la réponse `503` avec le composant `Error` sur chaque route concernée qui ne la porte pas encore, et sa description reprenant le message fixe.
- Client Python : rien. `OverloadedError` existe et porte `retry_after` (ADR-0112).
- Note de version de la mineure (ADR-0143, décision 4) : une ligne dans les **corrections de comportement**, avec le renvoi à cet ADR.

### 8. Tests

- un état non chargé se construit **sans barrière**, par `AppState::new()`, exactement comme `the_probe_bodies_distinguish_alive_from_ready` (`crates/crible-server/tests/server.rs:311`) : c'est le même état que pendant un chargement réel, sans dépendre d'un ordonnancement ;
- **un test par famille de routes** au moins : administration des index, documents, lots, recherche simple, recherche multi-index, maintenance, jobs, statistiques, journal de debug. Chacun vérifie le code `overloaded`, le message fixe, `details: {}`, `x-request-id`, `Retry-After: 1` et l'absence de l'ancienne réponse fausse ;
- un test **« chargé et vide »** qui borne le correctif : une instance chargée sans index rend `200 {"indexes":[]}` et `200 {"jobs":[]}` ;
- un test du **registre empoisonné** qui garde `500 internal`, distinct du démarrage ;
- un test de l'**ordre** : un corps invalide sur une route d'écriture rend `400` avant le `503` (ADR-0110), et une requête sans clé rend `401` avant le `503` (ADR-0100).

### Ce qui n'est pas décidé ici

- La durée du chargement et son amélioration : elle relève de la reprise (`docs/02`), d'ADR-0088 et d'ADR-0142, pas de cet ADR.
- Le corps de `/ready` et la liste de ses conditions : ADR-0099, inchangé.
- Un éventuel code d'erreur `unavailable` ou `starting` : il faudrait une `2.0.0` (ADR-0143, décision 1).
- La forme d'une réponse qui annoncerait la progression du chargement : ce serait un ajout compatible, et il demanderait son propre ADR.

## Conséquences

- **Positives** :
  - une seule règle remplace quatorze réponses fausses et quatre réponses trompeuses : un client qui interroge l'API pendant la reprise reçoit « réessaie », jamais « il n'y a rien » ;
  - `Retry-After: 1` donne au client la conduite à tenir sans qu'il ait à connaître `/ready` ;
  - aucun code d'erreur, aucun champ, aucune route ne change : le correctif tient dans une mineure (ADR-0143, décision 1) ;
  - l'ordre des validations d'ADR-0110 et l'antériorité de l'authentification sont préservés, donc les cas documentés ne bougent pas ;
  - le cas « chargé et vide » reste vrai et testé : une instance sans index continue de se déclarer prête et de rendre des listes vides.
- **Négatives / coûts** :
  - quatre routes qui rendaient `200` avec une liste vide rendent maintenant `503` pendant une fenêtre : un client qui tolérait la liste vide doit désormais réessayer, ce que la note de version signale ;
  - chaque route concernée gagne une réponse `503` dans la spécification, donc un test de plus (ADR-0111) ;
  - la règle est répétée à chaque famille de routes dans les tests, ce qui alourdit la suite de tests du serveur d'une dizaine de cas courts.
- **À surveiller** :
  - un client qui reçoit `503` sur `GET /v1/indexes` et le traite comme une panne plutôt que comme une attente : le symptôme serait une alerte au redémarrage, et la réponse serait la documentation de la note de version ;
  - une route nouvelle qui lit le registre sans passer par `AppState::index` ou `AppState::indexes` : elle échapperait à la règle, et le test par famille est là pour l'attraper ;
  - la durée du démarrage sur les six index de production (ADR-0142) : plus elle grandit, plus la fenêtre de `503` est visible côté client.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| **`200 {"indexes":[],"loading":true}`** | Ne règle que la liste des index et laisse les treize autres routes fausses ; garde un `200` pour une ressource absente, donc un client qui lit `indexes` sans regarder `loading` conclut encore à une base vide ; et change la forme d'un corps de route, ce qui est un ajout de contrat à décider, pas une correction. |
| **Lire la liste des index sur disque et la rendre sans leur état** | `<data>/indexes/*` devient une seconde source de vérité à côté du registre, alors que la reprise de `docs/02-architecture.md:144-159` décide que l'état servi vient de l'ouverture en écriture ; la route annoncerait des index que l'instance n'ouvre pas encore, et un index en quarantaine ou à moitié créé y apparaîtrait. |
| **Un code dédié `unavailable` ou `starting`** | C'est un code d'erreur nouveau, donc une rupture pour tout client qui compare `code` à la liste de `docs/05-api.md:38-52` ; ADR-0143, décision 1, l'interdit dans une mineure. `overloaded` dit déjà « réessaie », et le message fixe dit pourquoi. |
| **Un `503` global, en couche ou sur `is_ready()`** | Une couche passerait avant l'authentification et avant la validation du corps : un `401` deviendrait `503` et un `400` documenté deviendrait `503`, ce qui casse l'ordre d'ADR-0110. Sur `is_ready()`, la règle refuserait les lectures pendant le drainage d'arrêt, alors qu'ADR-0104 les laisse continuer. |
| **Laisser `GET /v1/jobs`, `GET /v1/stats` et `GET /v1/debug/queries` hors de la règle** | Ces routes mentent aussi pendant la fenêtre : `{"jobs":[]}` affirme qu'aucun job n'existe alors que la table n'est pas encore ouverte, `500 internal` sur `/v1/stats` annonce une panne qui n'en est pas une, et `enabled: false` décrit une configuration non publiée. Une règle unique vaut mieux que deux régimes voisins. |
| **Attendre la fin du chargement avant de se lier au port** | C'est l'inverse d'ADR-0099 : le conteneur n'aurait aucune sonde à interroger pendant la reprise, et l'orchestrateur ne distinguerait plus « démarrage long » de « processus mort ». La fenêtre est assumée ; c'est sa réponse qui change. |

## Références

- Ticket : `../legifacile/tickets-crible/06-demarrage-index-vides.md` (lecture seule, projet Legifacile).
- Suivi : `docs/suivi.md:167` (`T06-LISTE-INDEX-AU-DEMARRAGE`, classe « correction de comportement »), `:616` et `:629` (ordre de traitement de la v1.1), `:618-638` (audit et décisions attendues de la v1.1).
- Conception : `docs/05-api.md:35` (corps d'erreur commun), `:38-52` (liste close des codes), `:50` (`overloaded`), `:56-78` (tableau des routes, référence d'ADR-0111), `:77` (`/health`, `/ready`), `:398` (ordre des validations d'ADR-0110) ; `docs/02-architecture.md:144-159` (reprise au démarrage), `:173` (registre d'index mutable).
- ADR : ADR-0069 (`codes.bin` reconstruits à l'ouverture), ADR-0087 (décision 3, `overloaded` pour une ressource momentanément indisponible), ADR-0098 (`x-request-id`), ADR-0099 (liaison au port d'abord, `/health`, `/ready`, instance sans index prête), ADR-0100 (authentification avant la route), ADR-0101 (corps d'erreur, `details: {}`), ADR-0103 (décision 9, `Retry-After: 1` au rendu, décision 5, table des jobs ouverte au chargement), ADR-0104 (drainage d'arrêt, lectures continues), ADR-0105 (décisions 1 et 2, registre mutable et création synchrone), ADR-0110 (ordre des validations), ADR-0111 (spécification OpenAPI), ADR-0112 (client Python, `OverloadedError`), ADR-0118 (analytics ouvertes au chargement), ADR-0122 (journal de debug, `enabled: false`), ADR-0142 (durées de reconstruction lexicale), ADR-0143 (décision 1, correction de comportement ; décision 4, note de version).
- Code : `crates/crible-server/src/state.rs:158-171` (registre empoisonné, justification), `:716-733` (`AppState::index` et `indexes`, replis avant chargement), `:819-823` (`create_index`, `internal` avant chargement), `:900` (`AppState::load`), `:993` (publication de `loaded`) ; `crates/crible-server/src/indexes.rs:139`, `:210`, `:290`, `:412` ; `crates/crible-server/src/documents.rs:147`, `:282`, `:528` ; `crates/crible-server/src/batches.rs:119` ; `crates/crible-server/src/search.rs:181` ; `crates/crible-server/src/search_multi.rs:547` ; `crates/crible-server/src/maintenance.rs:115`, `:168`, `:234` ; `crates/crible-server/src/http.rs:211`, `:239`, `:290-307`, `:460` ; `crates/crible-server/src/error.rs:104-124` (`Retry-After: 1` au rendu).
- Tests : `crates/crible-server/tests/server.rs:311` (`the_probe_bodies_distinguish_alive_from_ready`, seul état non chargé testé aujourd'hui), `crates/crible-server/tests/errors.rs:570` (le `503` de `/ready` n'est pas un `overloaded`).
- Application : `docs/05-api.md`, `docs/02-architecture.md` et la spécification OpenAPI, dans le même changement que le code de la tâche `T06-LISTE-INDEX-AU-DEMARRAGE` ; note de version de la mineure.
