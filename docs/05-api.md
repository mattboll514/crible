# 05 — API REST v1

## Conventions

- **Format** : base `/v1`, JSON en UTF-8, noms en anglais et en `snake_case`.
- **Routes de liste** : une route de liste rend un objet à une clé, le nom pluriel de la ressource (ADR-0103) : `{ "jobs": [ … ] }` pour `GET /v1/jobs`, `{ "indexes": [ … ] }` pour `GET /v1/indexes`.
- **Authentification** : en-tête `Authorization: Bearer <clé>`. Les clés sont déclarées dans `crible.toml` sous forme de hash SHA-256, avec un nom et un rôle :

  | Rôle | Droits |
  |---|---|
  | `read` | recherche, lecture de documents, liste des index |
  | `write` | `read` + documents, lots, jobs |
  | `admin` | `write` + administration des index, compaction, export, instantanés, stats, journal de debug |

  Un haché vaut 64 caractères hexadécimaux minuscules : c'est le SHA-256 des octets UTF-8 de la clé telle qu'elle est transmise dans l'en-tête. `crible key generate` affiche la clé et son haché (ADR-0100).

  L'en-tête est analysé ainsi : schéma `Bearer` insensible à la casse, un seul espace, puis la clé, non vide et sans espace. Un en-tête absent, mal formé, vide ou présent plusieurs fois reçoit `401`, avec l'en-tête de réponse `WWW-Authenticate: Bearer` et le même corps qu'une clé inconnue : aucun indice ne distingue ces cas. Un rôle insuffisant reçoit `403`, sans `WWW-Authenticate` (ADR-0100).

- **Traçabilité** : chaque réponse porte l'en-tête `x-request-id`, et chaque réponse de recherche donne la `generation` lue. Un `x-request-id` entrant est repris s'il fait au plus 128 octets et ne contient que des caractères `[A-Za-z0-9._:-]` ; sinon, et s'il est absent, le serveur génère un UUID v4 et ne journalise jamais la valeur refusée. La valeur retenue est renvoyée sur **toutes** les réponses, y compris `404`, `405`, `413` et `500`, portée dans le champ `request_id` du corps d'erreur et dans chaque ligne de log de la requête (ADR-0098).
- **Mutations asynchrones** : elles renvoient `202 Accepted` avec un objet `job`.
  - Avec `?wait=true`, l'appel attend la fin du job (au plus `wait_timeout_ms`, 60 s par défaut). Un job terminé dans le délai est rendu **terminé**, en `200`, avec son objet `job` complet (`state` à `succeeded` ou `failed`, et son `error` le cas échéant). À l'échéance de `wait_timeout_ms`, l'appel rend `202 Accepted` avec le job **dans son état courant** (`queued` ou `running`), sans erreur, et le client suit l'avancement par `GET /v1/jobs/{job_id}` : jamais `504`, jamais un code `timeout`, le job n'est pas annulé (ADR-0067).
- **Dates** : ISO 8601 pour les horodatages, `YYYY-MM-DD` pour les champs de type `date`.
- **Ids** : seuls les ids externes apparaissent (**INV-01**).

## Erreurs

Format unique :

<!-- openapi: Error -->
```json
{ "error": { "code": "invalid_filter", "message": "unknown field 'juridicton' (did you mean 'juridiction'?)",
             "details": { "path": "$.filter.juridicton" }, "request_id": "…" } }
```

Toute réponse d'erreur du serveur suit ce format et porte `x-request-id` (ADR-0098, ADR-0101). Seule exception : le `503` de `/ready`, dont le corps reste celui d'ADR-0099. `message` est le message de l'erreur ; pour `internal`, il est générique et fixe (« internal error; see the server log for this request_id »), et un message ne contient jamais d'iid (**INV-01**) ni de chemin absolu du serveur. `details` vaut `{}`, sauf une clé documentée ici pour ce code : `path` seul pour `invalid_filter`, aujourd'hui `{"path": "$.filter.<champ>"}` ; et, pour un `invalid_request` qui porte une `reason`, `path` accompagné de cette `reason`, aujourd'hui `{"path": "$.mode", "reason": "lexical_branch_disabled"}` pour une branche lexicale désactivée (ADR-0145). Le `path` d'un `invalid_request` n'est publié **qu'avec** une `reason` : un refus qui nomme seulement un chemin dans son message — champ inconnu, mode inconnu, `targets` vide ou dupliqué — garde `details: {}`. `reason` est un **vocabulaire fermé**, une valeur par cas documenté ici ; une valeur nouvelle est documentée dans ce fichier dans le même changement que le code qui la produit, et demande son ADR (ADR-0101, décision 3 ; ADR-0145, décisions 2 et 3). Une route inconnue et une méthode non permise sur une route connue rendent toutes deux `not_found` (404), avec le message `no route for <METHOD> <path>` ; il n'y a pas de `405`. Un rejet d'extracteur (JSON mal formé, type de contenu absent ou faux, corps illisible) rend `invalid_request` (400) ; un corps au-delà de la limite rend `payload_too_large` (413). Chaque réponse d'erreur est journalisée avec son code, son statut et son `request_id` ; le `message` n'est journalisé que pour `internal` (ADR-0101). Toute réponse `overloaded` porte `Retry-After: 1`, quelle qu'en soit la cause — file d'écriture, limite de recherches concurrentes, arrêt en cours, verrou SQLite, branche lexicale —, par une seule règle posée au rendu de l'erreur ; le `503` de `/ready` n'est pas un `overloaded` et n'en porte pas (ADR-0103).

| Code | HTTP | Cas |
|---|---|---|
| `invalid_request` | 400 | Corps invalide, paramètre hors bornes ; `fusion.alpha` hors de `[0, 1]` ou non fini, à la création de l'index, au `PATCH` de `search.fusion` et dans une requête (ADR-0090) ; destination d'export déjà occupée, c'est-à-dire un dossier `exchange/outbox/<index>-g<G>/` existant (ADR-0061) ; instantané absent, incomplet ou altéré, destination d'instantané déjà occupée, ou `index.json` de la cible divergent à la restauration (ADR-0064, ADR-0065) ; recherche `lexical` ou `hybrid` sur un index dont `lexical.enabled` vaut `false`, y compris quand cet index est une cible d'une recherche multi-index : message « lexical branch is disabled for this index » et `details` valant `{"path": "$.mode", "reason": "lexical_branch_disabled"}`, sur la route simple comme sur la route multi-index (ADR-0089, ADR-0145) ; `directory` d'un lot hors de l'inbox, lien symbolique ou entrée qui n'est ni un fichier ni un dossier (ADR-0107) ; `doc_id` présent à la fois dans `documents` et dans `deletions` d'un lot en ligne (ADR-0040, ADR-0107) ; corps de lot en ligne sans aucun document ni aucune suppression (ADR-0107) ; corps non vide sur `compact`, `exports` ou `snapshots` (ADR-0107) ; combinaison de `query` et `vector` hors de la règle du mode demandé, ou `query` sans `vector` sur un index à modèle `external` (ADR-0109) ; nom de champ déclaré `returned: false` fourni dans `return.fields` (ADR-0109) |
| `invalid_filter` | 400 | Champ inconnu, opérateur interdit pour ce type, valeur mal typée |
| `unauthorized` | 401 | Clé absente, mal formée ou inconnue ; réponse avec `WWW-Authenticate: Bearer` et un corps identique dans les trois cas (ADR-0100) |
| `forbidden` | 403 | Rôle insuffisant ; sans `WWW-Authenticate` (ADR-0100) |
| `unknown_index` | 404 | Index inexistant |
| `not_found` | 404 | Document, job ou lot introuvable ; route inconnue ou méthode non permise sur une route connue (ADR-0101) ; `doc_id` inconnu ou retiré dans un patch de métadonnées (ADR-0060) ou dans `GET /documents/{doc_id}`, et `directory` absent dans `POST /batches` (ADR-0107) |
| `immutable_setting` | 409 | Tentative de modifier un réglage figé |
| `batch_conflict` | 409 | Même `batch_id`, manifeste différent ; rendu par le job d'un lot en ligne ou déposé, jamais par la soumission (ADR-0107) |
| `payload_too_large` | 413 | Corps au-delà des limites |
| `incompatible_model` | 422 | Modèle du lot ou des index d'une recherche incompatible |
| `acceptance_failed` | 422 | Recette refusée |
| `overloaded` | 503 | File d'écriture ou limite de recherches concurrentes atteinte, verrou SQLite non obtenu dans le délai de 1 000 ms (ADR-0038), verrou de présence d'un lot en ligne déjà tenu par une autre ingestion du même `batch_id` (ADR-0056), verrou d'écrivain `WRITER_LOCK` d'un index déjà tenu par un autre processus : toute commande d'écriture de la CLI sur un index servi par une instance en service est refusée sans attente, avec un message qui nomme l'index et dit qu'un autre processus l'écrit, jamais un chemin du serveur (ADR-0101, ADR-0116), course de génération pendant un export : l'index a commité entre l'instantané et la première lecture, après trois tentatives (ADR-0061) ; index lexical non certifié, en retard ou illisible, ouvert en lecture seule : la branche lexicale est refusée, le mode `dense` reste servi, et la prochaine ouverture en écriture répare (ADR-0087, ADR-0088, ADR-0089) — le message nomme l'état constaté : « behind » (`M > Gt`), « no lexical index is committed in this epoch although passages exist (generation G); open the index for writing to rebuild it » (aucun commit tantivy alors que des passages existent : l'ouverture en écriture reconstruit, elle ne rattrape pas), « ahead » (`Gt > G`), « not certified » (colonne M absente, avec ou sans commit tantivy) ou « unreadable » (lexical illisible, `M > G`) (`Retry-After: 1`, ADR-0103). Une chaîne d'analyse enregistrée différente de celle d'`index.json`, à l'ouverture en lecture seule : « the lexical index was built with another analysis chain; open the index for writing to rebuild it » (ADR-0106). Une époque qui n'a jamais porté de passage (`M = 0`, aucun commit tantivy) n'est pas refusée : `lexical` et `hybrid` rendent une réponse vide (ADR-0089). Registre d'index ou état publié pas encore chargé au démarrage : les index existent sur disque mais ne sont pas encore ouverts, donc une liste vide ou un `404` y serait faux ; toute route qui les lit rend `503 overloaded` avec le message fixe « the server is starting » (ADR-0146) |
| `timeout` | 504 | Requête trop longue |
| `internal` | 500 | Erreur inattendue (journalisée avec le `request_id`) ; panne disque pendant un instantané ou une restauration (ADR-0065) |

Ces codes servent aussi de catégories d'erreurs dans les analytics.

## Endpoints

**Démarrage : `503 overloaded` jusqu'à la publication.** Le serveur se lie au port avant de charger les index, et c'est décidé (ADR-0099). Tant que le registre n'est pas chargé, toute route qui le lit rend `503 overloaded` avec le message fixe « the server is starting », `details: {}`, `x-request-id` et `Retry-After: 1` ; aucun code d'erreur ni aucune valeur de `details` n'est ajouté (ADR-0146). Ce sont les quatorze routes du registre — `GET /v1/indexes`, `POST /v1/indexes`, `GET`, `PATCH` et `DELETE /v1/indexes/{index}`, `POST` et `PATCH /v1/indexes/{index}/documents`, `GET /v1/indexes/{index}/documents/{doc_id}`, `POST /v1/indexes/{index}/batches`, `POST /v1/indexes/{index}/search`, `POST /v1/search`, `POST /v1/indexes/{index}/compact`, `POST /v1/indexes/{index}/exports`, `POST /v1/indexes/{index}/snapshots` — et les quatre routes de l'état publié : `GET /v1/jobs`, `GET /v1/jobs/{job_id}`, `GET /v1/stats` et `GET /v1/debug/queries`, dont `enabled: false` reste réservé au journal réellement désactivé par la configuration publiée (ADR-0122). `/health` et `/ready` ne changent pas : `/health` rend `200` dès la liaison, `/ready` rend `503` jusqu'à la publication du dernier instantané (ADR-0099). Le refus naît à l'accès au registre ou à l'état publié, dans le gestionnaire de chaque route, jamais dans une couche : il garde donc l'ordre propre à chaque route. En `POST /v1/search`, une faute de forme ou de bornes du corps reste `400 invalid_request` avant le `503`, dans l'ordre d'ADR-0110, qui examine la forme, les bornes, le regroupement et les entrées du mode — étape (a), voir « Recherche multi-index » — avant toute lecture de l'index ; les fautes qui demandent la configuration ou les schémas publiés — les entrées d'un modèle `external`, branche lexicale, `invalid_filter`, `return.fields`, `incompatible_model` — rendent `503` d'abord. `POST /v1/indexes` examine lui aussi son corps avant l'accès au registre, donc une faute de forme y reste `400` ; ailleurs, la route qui lit l'index avant d'examiner le corps rend `503` d'abord, comme `POST /v1/indexes/{index}/search` et `PATCH /v1/indexes/{index}`. Un `401` ou un `403` passe avant ce `503` (ADR-0100). Un registre empoisonné reste `500 internal`, avec son message distinct. Une instance **chargée** qui ne sert aucun index n'est pas concernée : elle est prête, et rend `200 {"indexes":[]}` sur `GET /v1/indexes` et `200 {"jobs":[]}` sur `GET /v1/jobs` ; un `404 unknown_index` rendu après chargement reste celui d'un index réellement inconnu.

| Méthode et chemin | Rôle | Description |
|---|---|---|
| `POST /v1/indexes` | admin | Créer un index à chaud (config complète) → `201` avec le corps de `GET /v1/indexes/{index}` ; nom déjà pris → `400` (ADR-0105) |
| `GET /v1/indexes` | read | Lister les index : nom, modèle, génération, documents, passages vivants, triés par nom (ADR-0105) |
| `GET /v1/indexes/{index}` | read | Config + état : époque, génération, documents, passages vivants et morts, ratio de morts, nombre de segments, tailles, dernier lot (ADR-0105) |
| `PATCH /v1/indexes/{index}` | admin | Modifier les réglages modifiables → `202` avec un job `rebuild_lexical` ou `patch_config` ; un réglage figé → `409` ; une clé inconnue → `400` (ADR-0105) |
| `DELETE /v1/indexes/{index}?confirm={index}` | admin | Supprimer l'index → `202` avec un job `delete_index` ; `confirm` absent ou différent du nom → `400` (ADR-0105) |
| `POST /v1/indexes/{index}/documents` | write | Lot en ligne : `batch_id` obligatoire, documents et passages (texte, ou vecteurs fournis), `deletions` → `202` avec un job `online_batch` ; un corps sans document ni suppression rend `400` ; erreurs d'attache dans le job (ADR-0107) |
| `PATCH /v1/indexes/{index}/documents` | write | Mettre à jour des métadonnées de documents → job ; `not_found` porté par le job (ADR-0107) |
| `GET /v1/indexes/{index}/documents/{doc_id}` | read | Document, ses passages et son état, lus dans l'instantané publié (`?text=true` pour inclure le texte) (ADR-0107) |
| `POST /v1/indexes/{index}/batches` | write | Attacher un lot déposé dans `exchange/inbox/<nom>` ; corps `{ "directory": "<nom>" }`, un seul composant de chemin, sans lien symbolique ; le job revalide le dossier avant de le lire → job (ADR-0107) |
| `GET /v1/jobs/{job_id}` · `GET /v1/jobs` | write | Suivre un job ; lister les jobs (filtres `index`, `state`) ; le membre `durations` d'une attache est gardé en mémoire et vaut `null` quand il n'y a rien à rendre (ADR-0149) |
| `POST /v1/indexes/{index}/search` | read | Recherche dans un index |
| `POST /v1/search` | read | Recherche multi-index |
| `POST /v1/indexes/{index}/compact` | admin | Lancer une compaction ; sans corps → `202` avec un job `compact` (ADR-0107) |
| `POST /v1/indexes/{index}/exports` | admin | Exporter vers `exchange/outbox` ; sans corps → `202` avec un job `export` (ADR-0107) |
| `POST /v1/indexes/{index}/snapshots` | admin | Instantané vers `exchange/snapshots` ; sans corps → `202` avec un job `snapshot` ; l'écrivain est suspendu pendant la copie, les lectures continuent (ADR-0017, ADR-0065, ADR-0107) |
| `GET /v1/stats` | admin | Analytics agrégées ; paramètres `from`, `to`, `index`, `operation`, `group_by`, réponse fermée (voir « Statistiques » ; ADR-0124) |
| `GET /v1/debug/queries` | admin | Journal de debug, s'il est activé ; `200` avec `enabled: false` et `queries: []` quand il est désactivé (ADR-0121, ADR-0122) |
| `GET /health` · `GET /ready` | — | En vie · prêt (modèles chargés, chaque index a publié son instantané, aucune reprise en cours). `/health` rend `200` dès la liaison au port ; `/ready` rend `503` jusqu'à la publication du dernier instantané, puis `200` ; une instance sans index est prête (ADR-0099) |
| `GET /openapi.json` · `GET /docs` | — | Spécification OpenAPI · Swagger UI |

La spécification est **OpenAPI 3.1** et **générée depuis le code** ; elle est servie par `GET /openapi.json`, et `GET /docs` sert Swagger UI. Aucun fichier de spécification n'est versionné. Le schéma de sécurité `bearerAuth` (HTTP Bearer, clé d'API d'ADR-0100) décrit l'authentification des routes à rôle ; `/health` et `/ready` la neutralisent dans la spécification par `security: [{}]`, une exigence vide que `utoipa` écrit à la place de `security: []`, et restent sans authentification. `/openapi.json` et `/docs` ne figurent pas dans la spécification et restent servis sans clé ; `/docs` répond par une redirection `303` vers `/docs/`. Ce tableau est la **référence** de la liste des routes : un test compare, dans les deux sens, l'ensemble des couples (méthode, chemin sans paramètre de requête) de la spécification à celui du tableau. Sont exceptées par leur nom `/openapi.json` et `GET /docs`, ressources statiques hors spécification, et toute ligne du tableau affectée à une tâche d'une phase ultérieure, tant que cette tâche n'est pas faite. `GET /v1/debug/queries` est déclarée dans la spécification par JOURNAL-DE-DEBUG, et son exception a quitté le test dans le même changement (ADR-0122, décision 9). `GET /v1/stats` est déclarée par ROUTE-STATS, et son exception nommée quitte le test dans le même changement (ADR-0124, décision 16). Toute route ajoutée au code est ajoutée ici dans le même changement (ADR-0111, complément du 2026-10-30).

**Deux ajouts décidés par ADR-0130, livrés en P9.** L'interface statique `/ui`, servie sans clé hors de l'espace `/v1` et hors de la spécification, et `GET /v1/indexes/{index}/documents`, liste paginée des documents au rôle `read`, sont décidées par ADR-0130 (décisions 2, 4 et 6). Ni l'une ni l'autre ne figure encore dans le tableau ci-dessus : la tâche qui les livre y ajoute sa ligne **dans le même changement** que l'opération ou l'exception correspondante dans la spécification et dans `openapi_routes.rs` (ADR-0111) — `SERVIR-UI-STATIQUE` pour `/ui`, exception nommée comme `/docs`, et `ROUTE-LISTER-DOCUMENTS` pour la liste des documents. Aucune exception n'est ajoutée au test des routes avant ces tâches.

Dans un `PATCH /v1/indexes/{index}`, une clé **inconnue** est refusée avec `invalid_request` (400), à tous les niveaux du corps, y compris dans les objets imbriqués (`search`, `lexical`, `ingest`, `compaction`). Le message nomme la clé et son chemin JSON. `immutable_setting` (409) reste réservé aux clés connues mais figées (ADR-0031, INV-16). Les clés connues de `lexical` sont `enabled`, `language`, `positions` et `identifiers` (ADR-0148) ; elles sont acceptées au `PATCH` sous réserve de leur domaine, et un changement de `identifiers` reconstruit le lexical comme celui de `language` ou de `positions`.

### Administration des index

Un index se crée, se règle et se supprime par l'API **sans redémarrer l'instance** (ADR-0105). Les trois routes de mutation sont réservées à `admin`, les deux `GET` sont ouverts à `read` (`:11-13`).

**`POST /v1/indexes`** prend la configuration complète d'un index, dans la forme d'`index.json` (`docs/04`), et rend `201` avec le **corps de `GET /v1/indexes/{index}`**. La validation est celle de la création d'index de la CLI. Un nom déjà présent, dans le registre ou sur disque, rend `400 invalid_request` avec le message « index already exists » : il n'y a pas de code dédié. Le modèle nommé par `model` est lu sous `<data>/models/<nom>/model.json` avant toute écriture ; un modèle absent ou illisible rend `400 invalid_request`, et le message nomme le modèle. La session du modèle est chargée si elle ne l'est pas déjà, et reste chargée jusqu'au redémarrage. Une seule création a lieu à la fois par instance.

**`GET /v1/indexes`** rend l'enveloppe d'ADR-0103 :

<!-- openapi: IndexList -->
```json
{ "indexes": [ { "name": "decisions", "model": "mDenseOn-fr", "generation": 1288,
                 "documents": 412000, "live_passages": 8580000 } ] }
```

- triés par nom.

**`GET /v1/indexes/{index}`** rend la configuration matérialisée et l'état de la **génération publiée** :

<!-- openapi: IndexDetail -->
```json
{ "name": "decisions",
  "config": {
    "name": "decisions", "model": "mDenseOn-fr", "reference_precision": "fp16",
    "schema": {
      "document_fields": { "juridiction": { "type": "keyword" },
                           "numero": { "type": "keyword" },
                           "titre": { "type": "keyword", "filterable": false } },
      "passage_fields": { "partie": { "type": "keyword" } }
    },
    "cluster_by": ["juridiction"],
    "lexical": { "enabled": true, "language": "fr", "positions": false },
    "search": { "scan": "binary", "rescore_candidates": 5000, "exact_threshold": 5000,
                "candidates_per_branch": 100, "fusion": { "method": "rrf" } },
    "ingest": { "acceptance_sample": 200, "acceptance_min_cosine": 0.999 },
    "compaction": { "dead_ratio": 0.10 }
  },
  "state": { "epoch": 4, "generation": 1288,
             "documents": 412000, "live_passages": 8580000,
             "dead_passages": 12400, "dead_ratio": 0.00144,
             "segments": 37,
             "sizes": { "disk_bytes": 21474836480, "filter_memory_bytes": 41943040, "codes_memory_bytes": 1287000000 },
             "last_batch": { "batch_id": "legifacile-2026-10-19", "applied_at": "2026-10-19T08:12:03Z", "generation": 1288 } } }
```

- les comptes sont ceux de l'instantané publié, jamais le résultat d'une lecture de SQLite ;
- `segments` est un **nombre** ; la liste des segments n'est pas exposée, parce que leurs bornes d'iid sortiraient du moteur (**INV-01**) ;
- `sizes.disk_bytes` est la taille du dossier de l'index sur disque, `sizes.filter_memory_bytes` et `sizes.codes_memory_bytes` sont les estimations mémoire de l'état ;
- `last_batch` est la dernière ligne de `batches`, et vaut `null` quand aucun lot n'a été attaché ;
- `config.lexical.identifiers` est **absent** de la réponse quand il vaut `"none"` : la configuration rendue est la configuration matérialisée, et la clé n'est écrite que pour `"v1"` (ADR-0148, décision 6). Un index où le réglage vaut `"v1"` le rend donc dans `config.lexical`, à côté de `enabled`, `language` et `positions`. Un client doit lire l'absence comme `"none"`.

**`PATCH /v1/indexes/{index}`** valide le patch **avant de le mettre en file** : clé inconnue → `400` (ADR-0031), réglage figé → `409 immutable_setting` nommant le champ (INV-16), valeur hors domaine → `400`. Le changement est ensuite exécuté par l'écrivain de l'index, dans l'ordre d'arrivée de sa file, comme un job : `202` avec le job, `?wait=true` honoré. Le `kind` du job vaut `rebuild_lexical` quand le job reconstruit l'index lexical (`lexical.enabled` vaut `true` après le patch, et l'une de `lexical.enabled`, `lexical.language`, `lexical.positions`, `lexical.identifiers` a changé), et `patch_config` sinon. Le job écrit `index.json`, reconstruit ce qui en dérive, puis publie ensemble la nouvelle configuration et le nouvel instantané : une requête voit l'ancienne configuration avec l'ancien instantané, ou la nouvelle avec le nouveau. `search.*` vaut pour les requêtes qui suivent la fin du job ; `ingest.*`, `compaction.*` et `cluster_by` valent pour le prochain job ou la prochaine compaction. Passer `lexical.enabled` à `false` ne reconstruit rien (les recherches `lexical` et `hybrid` sont refusées, ADR-0089) ; le repasser à `true` reconstruit. Le résultat du job vaut `null`.

**`DELETE /v1/indexes/{index}?confirm={index}`** exige un `confirm` égal au nom ; sinon la réponse est `400 invalid_request`, et le message nomme le paramètre `confirm`. La mutation rend `202` avec un job `delete_index` mis dans la file de l'index : les jobs acceptés avant lui se terminent d'abord. Le job retire l'index du registre — les nouvelles requêtes reçoivent `404 unknown_index`, les requêtes en vol finissent sur l'instantané qu'elles tiennent —, renomme le dossier en `<data>/tmp/deleted-<nom>-<job_id>` puis le supprime. Les jobs restés derrière lui échouent en `unknown_index`, et l'historique des jobs de l'index reste dans `jobs.sqlite` (ADR-0103). Un `POST` du même nom est refusé tant que l'index est au registre, accepté après le renommage.

### Lot en ligne

<!-- openapi: OnlineBatchRequest -->
```json
{
  "batch_id": "manual-2026-10-19-001",
  "mode": "upsert",
  "documents": [
    { "doc_id": "D1", "doc_hash": "optional", "fields": { "juridiction": "cass" },
      "passages": [ { "text": "…", "fields": { "partie": "motifs" } } ] }
  ],
  "deletions": ["D0"]
}
```

- Si `doc_hash` est absent, il vaut le blake3 du **JSON canonique des passages** (ADR-0055) : pour chaque passage, `ord`, `text` et `fields`, clés triées, dans la forme canonique d'ADR-0019 ; le digest est rendu `blake3:<hex>` (ADR-0034, ADR-0041). Cette forme couvre le texte, les champs et le découpage, donc un `doc_hash` identique signifie bien que rien n'a changé **dans les passages**. Les champs de **document** restent hors du hash : un patch de métadonnées ne change pas le `doc_hash` (ADR-0060). Le lot interne remplit toujours la colonne `doc_hash`, qui est non nullable.
- Si un passage porte un `vector`, il est encodé en base64 (fp32 little-endian) ; sinon, le serveur encode le texte.
- **`batch_id`** : obligatoire, pris dans le corps de la requête et passé à `crible-store::online` par le même constructeur que la CLI (ADR-0055, ADR-0107). Un rejeu du même `batch_id` avec le même contenu ne fait rien (INV-10) ; un contenu différent rend `batch_conflict` dans le job.
- **`deletions`** : liste de `doc_id` à retirer. Le lot interne les écrit dans un `deletions.parquet` conforme au format de lot v1, et l'attache les applique comme pour tout lot : la ligne `documents` tombe dans la transaction du commit (ADR-0041, ADR-0107). Un `doc_id` présent à la fois dans `documents` et dans `deletions` rend `400 invalid_request` (ADR-0040). Un corps qui ne porte que des `deletions` est un lot sans passage : son rapport porte `acceptance: null`, et ses fichiers `documents` et `passages` sont vides.
- **Corps vide** : un corps qui ne porte **ni** `documents` **ni** `deletions` rend `400 invalid_request`, avant la mise en file, avec un message qui nomme les deux champs attendus ; c'est le refus du fichier vide du chemin par fichier, appliqué au corps (ADR-0107).
- **Erreurs d'attache** : `batch_conflict`, `acceptance_failed` et `incompatible_model` sont détectés pendant le job. La soumission rend `202` et le job passe à `failed` avec le `error.code` correspondant (ADR-0103, ADR-0107). Seule la forme du corps est refusée avant la mise en file (`400 invalid_request`, `413 payload_too_large`).
- Limite : 10 000 entrées par requête, passages décrits et `doc_id` de `deletions` confondus (ADR-0107).
- **Mode** : `upsert` seulement (ADR-0055). `replace_all` est refusé avec `invalid_request`, et le message nomme le mode : la limite de 10 000 entrées le rend inutilisable en ligne. `replace_all` reste le mode d'un lot déposé. Le contrôle du `mode` précède celui du corps vide : un corps qui cumule les deux fautes, par exemple `{"batch_id":"x","mode":"replace_all"}`, rend le `400` qui nomme le mode (ADR-0115).
- **Manifeste du lot interne** : déterministe, `producer` vaut `crible-online` ; ses valeurs sont fixées par ADR-0055 et décrites dans `04-donnees-et-ingestion.md` (mode en ligne) et `format-lot-v1.md` (lot interne du mode en ligne).

### Lot déposé

`POST /v1/indexes/{index}/batches` attache un lot déjà écrit dans `<exchange>/inbox/<nom>/`.

<!-- openapi: DepositedBatchRequest -->
```json
{ "directory": "legifacile-decisions-2026-10-19" }
```

- `<nom>` est **un seul composant de chemin** : ni vide, ni `.`, ni `..`, ni `/`, ni `\` (`path_component`). Le chemin `inbox/<nom>` est canonicalisé et doit rester sous la racine canonique de l'inbox.
- `<nom>` ne doit pas être un **lien symbolique**, et aucun lien symbolique n'est admis **dans le dossier du lot, à toute profondeur**, y compris vers un autre lot de l'inbox ; toute entrée qui n'est ni un fichier ni un dossier fait aussi refuser le lot. Un refus rend `400 invalid_request`, et le message nomme le champ `directory`, jamais un chemin absolu du serveur (ADR-0101).
- Un dossier absent rend `404 not_found`. Sinon la réponse est `202` avec un job `attach_batch`. Le job **revalide le dossier du lot juste avant de le lire**, dans l'écrivain, par la même règle que ci-dessus : chemin canonique sous l'inbox, aucun lien symbolique à aucune profondeur, aucune entrée spéciale. Il n'ouvre pas les fichiers avec `O_NOFOLLOW` — la crate `libc` n'est pas dans `docs/07` —, et la fenêtre résiduelle entre cette revalidation et la lecture est assumée : l'inbox est sous le contrôle de l'exploitant, et le serveur seul la lit (ADR-0107).

### Patch de métadonnées

`PATCH /v1/indexes/{index}/documents` met à jour les **champs d'un document** sans ré-encodage (ADR-0060).

<!-- openapi: MetadataPatchRequest -->
```json
{ "documents": [ { "doc_id": "D1", "fields": { "juridiction": "cass" } } ] }
```

- **Fusion** : chaque clé de `fields` remplace la valeur du champ de même nom, les clés absentes gardent leur valeur. Un `fields: {}` est un no-op, compté `unchanged`.
- **`null`** est une valeur posée ; il ne retire jamais la clé.
- **Champs** : seuls les champs de document du schéma sont acceptés. Une clé inconnue, un champ de passage, `doc_id` ou `doc_hash` donnent `invalid_request`, avec un message qui nomme le champ, le niveau attendu (`document_fields`) et le chemin JSON — jamais la valeur. Un champ `filterable: false` ou `returned: false` accepte la valeur, qui reste invisible aux filtres ou au payload du document.
- **Valeur mal typée** : `invalid_request`, avec le type attendu ; jamais `invalid_filter`, qui appartient à la grammaire des filtres.
- **Document inconnu ou retiré** : `not_found`, et aucun document du corps n'est appliqué. Un patch ne crée jamais de document. Le refus est porté par le **job** : la soumission rend `202`, et le job passe à `failed` avec `error.code` à `not_found` (ADR-0103, ADR-0107).
- **Limites** : 32 Mo et 10 000 documents (voir « Limites par défaut »), sinon `payload_too_large`.
- **Rapport** : `{ updated, unchanged, generation }`, par document. Il n'est pas persisté (voir « Jobs »).
- **CLI** : `crible patch --data <dir> --index <nom> <fichier>.jsonl` lit le même objet, un par ligne, le fichier étant passé en argument positionnel, sans `--models` : un patch n'encode rien.
- **Effet** : `doc_hash` inchangé, `updated_generation = G+1` sur les lignes patchées, aucun segment, aucune ligne `batches` ; aucune génération n'est créée si aucun document ne change (`docs/02`, ADR-0060).
- **Non-durabilité** : une attache ultérieure du même document avec un `doc_hash` identique réécrit les champs de document et écrase la valeur patchée (`docs/04`).
- Le patch passe la même file d'écriture que les autres écritures de l'index. Lancer `crible patch` sur un index servi par un serveur reste une règle d'exploitation : `busy_timeout` n'assure que la vivacité (ADR-0038), pas l'exclusion entre processus.

### Lecture d'un document

`GET /v1/indexes/{index}/documents/{doc_id}` rend un document, ses champs et ses passages vivants (ADR-0107). `?text=true` ajoute le texte des passages.

<!-- openapi: DocumentDetail -->
```json
{ "doc_id": "LEGIARTI…", "generation": 1288, "doc_hash": "blake3:…",
  "fields": { "num": "1240", "fonds": "LEGI" },
  "passages": [ { "ord": 0, "fields": { "partie": "motifs" }, "text": "…" } ] }
```

- le document est lu dans la **base de l'époque de l'instantané publié**, jamais dans l'état SQLite courant ; `generation` est celle de l'instantané ;
- seuls les champs **`returned`** du schéma sont rendus, à la même règle que les résultats de recherche ; les passages sont les passages **vivants**, triés par `ord` croissant ;
- `text` n'apparaît qu'avec `?text=true` ; aucun vecteur, aucun `iid` (**INV-01**) ;
- un document inconnu ou retiré rend `404 not_found` ;
- rôle `read`.

**Liste paginée des documents (ADR-0130, décision 6 ; livrée en P9).** `GET /v1/indexes/{index}/documents?limit=&after=` rend les documents d'un index par `doc_id` croissant, curseur `after` exclusif, rôle `read`, dans un corps `{ "documents": [ … ] }`. Seuls les champs `returned` sont rendus, **sans texte par défaut**, et la liste ne porte aucun id interne. Sa ligne dans le tableau des routes et son opération OpenAPI arrivent avec la tâche `ROUTE-LISTER-DOCUMENTS`, dans le même changement (ADR-0111).

## Grammaire des filtres

- **Un objet est un ET implicite.** Chaque clé est soit un nom de champ, soit `$and`, `$or` ou `$not`.
- **La valeur d'un champ** est soit un scalaire (qui vaut `eq`), soit un objet d'opérateurs.

| Type | Opérateurs |
|---|---|
| `keyword` | `eq`, `ne`, `in`, `nin`, `exists` |
| `integer`, `float`, `date` | `eq`, `ne`, `lt`, `lte`, `gt`, `gte`, `in`, `exists` |
| `bool` | `eq`, `exists` |

- **Valeurs nulles** (ADR-0035) :
  - les comparaisons (`eq`, `ne`, `in`, `nin`, `lt`, `lte`, `gt`, `gte`) ne sont **jamais** satisfaites par un null ;
  - `exists: true` vaut « présent et non null » ; `exists: false` vaut « absent ou null » ;
  - `$not` est le complément sur les vivants. Il inclut donc les passages dont le champ est null : `{"$not": {"f": {"eq": "a"}}}` et `{"f": {"ne": "a"}}` ne sont **pas** équivalents.

- **Limites** (ADR-0029) :
  - `in` et `nin` : 1 000 valeurs au plus ;
  - profondeur : 8 au plus. On compte les **niveaux d'objets et de tableaux** ; la racine vaut 1 ; un objet d'opérateurs de champ compte pour un niveau ; un tableau `$and` ou `$or` compte pour un niveau, et chacun de ses éléments pour le niveau suivant. Neuf `$not` imbriqués sont refusés, quatre sont acceptés ;
  - nombre de clauses : 64 au plus. Une **clause** est une **condition de champ**, c'est-à-dire une entrée dont la clé est un nom de champ ; `$and`, `$or` et `$not` ne comptent pas. 64 conditions sont acceptées, la 65e donne `invalid_filter`. Exemple : `{ "a": 1, "$or": [ { "b": 2 }, { "c": 3 } ] }` compte 3 clauses et une profondeur de 3 ;
  - un dépassement donne `invalid_filter`, avec le chemin JSON de l'élément fautif.
- **Validation** contre le schéma de l'index :
  - le message d'erreur nomme ce qui était attendu ;
  - un champ marqué `filterable: false` donne `invalid_filter`.

**Ce que `filterable` promet, et ce qu'il ne promet pas.** `filterable` dit qu'un champ est stocké pour le filtre et accepté par la validation contre le schéma de cet index ; il ne promet pas qu'un filtre rende un résultat non vide. Un champ déclaré mais toujours nul dans un index donne un pool vide sous `in`, et non une erreur : aucune comparaison n'est satisfaite par un null (ADR-0035) ; `exists: false` le sélectionne, et un `$not` de comparaison aussi, puisqu'il complète sur les vivants, nulls compris (voir « Valeurs nulles » plus haut). Un champ `filterable: false` et un champ inconnu rendent le même `invalid_filter`, avec un message qui nomme le champ et `details.path`.

Le filtre vaut à l'identique dans les trois modes : les branches dense et lexicale partagent le même pool filtré (`03-recherche.md`, « Fusion (mode `hybrid`) » ; INV-05), donc `dense`, `lexical` et `hybrid` acceptent et appliquent un champ déclaré de la même façon, et aucun mode ne refuse un champ déclaré. Réserver un champ à un usage — un mode de recherche, un corpus, un client — est un choix de l'appelant, que le schéma ne porte pas et que la configuration d'index n'exprime pas.

Pour diagnostiquer un filtre accepté qui rend un pool vide, lire `pool_size` de la réponse — `targets[].pool_size` en multi-index —, et, pour un document précis, `in_pool` d'`explain_ids`, qui dit si l'id demandé est dans le pool filtré.

Exemples tirés de Legifacile :

<!-- openapi: Filter -->
```json
{ "fonds": "LEGI", "date_debut": { "lte": "2020-01-01" },
  "$or": [ { "date_fin": { "gt": "2020-01-01" } }, { "date_fin": { "exists": false } } ] }
```
Ce premier filtre sélectionne les articles du fonds LEGI en vigueur au 1er janvier 2020.

<!-- openapi: Filter -->
```json
{ "juridiction": { "in": ["cass", "ce"] }, "partie": { "nin": ["moyens", "faits"] } }
```
Ce second filtre garde les décisions de la Cour de cassation et du Conseil d'État, sans les passages « moyens » ni « faits ».

## Recherche : requête

`POST /v1/indexes/{index}/search`

| Champ | Type | Défaut | Rôle |
|---|---|---|---|
| `query` | string | — | Texte de la requête, encodé par le serveur ; exigé par `lexical` et `hybrid`, facultatif en `dense` (ADR-0109) |
| `vector` | base64 fp32 LE | — | Vecteur fourni ; remplace l'encodage de `query` dans la branche dense, obligatoire pour un modèle `external` (ADR-0109) |
| `mode` | `dense` · `lexical` · `hybrid` | `hybrid` si le lexical est activé, sinon `dense` | Branches à exécuter |
| `filter` | objet | aucun | Voir la grammaire ci-dessus |
| `k` | 1 à 1000 | 10 | Nombre de résultats ; sous `group_by: "document"`, nombre de documents distincts (ADR-0075) |
| `fusion` | `{method: "rrf"}` ou `{method: "relative_score", alpha}` | config de l'index | Fusion hybride ; `alpha ∈ [0, 1]`, bornes comprises et valeur finie, refusé en `invalid_request` hors de ce domaine (ADR-0090) |
| `group_by` | `"document"` ou null | null | Regroupement par document |
| `passages_per_document` | 1 à 5 | 1 | Passages renvoyés par document regroupé ; il est validé même sans `group_by`, où il est sans effet (ADR-0075) |
| `dedup` | `"content"` ou null | null | Fusion des passages identiques |
| `return` | `{fields: [...] ou "all", text: bool}` | `{fields: "all", text: true}` | Contenu renvoyé ; `"all"` vaut tous les champs **disponibles pour ce résultat** (ADR-0043) ; un nom déclaré `returned: false` est refusé (ADR-0109) |
| `exact` | bool | false | Force le calcul exact (éval, debug) |
| `debug` | bool | false | Ajoute plan, temps et rangs (ADR-0119) ; alimente le journal de debug quand il est activé : **toujours** pour `debug: true`, par échantillonnage sinon (ADR-0122, décisions 1 et 2) |
| `explain_ids` | liste de `doc_id` ou de clés `doc_id#ord` ; en multi-index, objets `{ "index", "id" }` | [] | Diagnostic de documents précis, 100 ids au plus (ADR-0123) |
| `timeout_ms` | 1 à 30000 | 2000 | Délai maximal |

**Ce que `query` couvre.** `query` ne porte que sur le **texte des passages** : la branche lexicale analyse ce texte, et la branche dense compare le vecteur du passage au vecteur de la requête. `doc_id` et `doc_hash` ne sont indexés dans aucune branche : un document tantivy est un passage à deux champs, `iid` et `text` (`03-recherche.md`, « Branche lexicale (tantivy) »), et le vecteur d'un passage ne décrit que ce texte. En `lexical`, la requête est un sac de termes liés par un **OU**. Un identifiant que l'analyse laisse en **un seul jeton**, par exemple `A42XYZ`, ne retrouve un passage que si le texte de ce passage porte ce jeton ; un identifiant **ponctué**, comme `A-42` ou `10-13.625`, est découpé en plusieurs jetons par l'analyseur, OUés comme les autres termes, donc un passage qui n'en porte qu'un seul est rendu, souvent très loin dans le classement (ADR-0094) ; et dans une requête à plusieurs termes, un identifiant ne pèse que comme ses termes, pas davantage. En `dense` et en `hybrid`, l'identifiant est encodé comme un texte quelconque, et le score ne mesure qu'une similarité avec le texte du passage : rien ne garantit que le document qui porte cet identifiant soit retrouvé, et aucune branche ne compare l'identifiant à un `doc_id`. Les champs de document n'entrent pas dans ce texte non plus : le texte indexé est l'entrée encodeur composée par le producteur, et le moteur n'y ajoute rien (`04-donnees-et-ingestion.md`, « Le découpage : qui fait quoi » ; ADR-0010, ADR-0020). Un champ de document n'est donc cherchable comme texte que si le gabarit du producteur le recopie dans le passage.

Pour lire un document connu, employer `GET /v1/indexes/{index}/documents/{doc_id}` ; pour savoir pourquoi un document attendu ne sort pas d'une recherche, employer `explain_ids`. Avec `mode: "lexical"` et `explain_ids` sur un identifiant que le texte des passages ne contient pas, la réponse ne porte aucun résultat et l'explication dit que le document existe et qu'il est dans le pool :

<!-- openapi: SearchExplain -->
```json
{ "explain": [
  { "id": "A-42", "doc_id": "A-42", "exists": true, "alive": true, "in_pool": true } ] }
```

Aucune clause de filtre ne l'exclut, d'où l'absence d'`excluded_by`. Aucun objet de branche ne le décrit : la branche dense n'a pas tourné en mode `lexical`, la branche lexicale ne le classe pas faute d'un terme analysé de la requête dans son texte, et `fused` est absent parce que le passage n'entre dans aucune liste fusionnée. Un membre sans valeur est omis, jamais rendu `null` (ADR-0119, décision 3 ; ADR-0123) ; la forme complète de `explain` est décrite dans « Recherche : réponse ».

**Entrées selon le mode** (ADR-0109). `dense` prend **exactement un** de `query` ou `vector` : les deux ensemble, ou aucun des deux, rendent `invalid_request` (400). `lexical` exige `query` et **refuse** `vector` (400). `hybrid` exige `query` — la branche lexicale note le texte — et accepte un `vector` facultatif, qui sert alors la branche dense **à la place** de l'encodage de `query`. Sur un index à modèle `external`, qui n'encode pas, `dense` exige `vector` et `hybrid` exige `query` **et** `vector` ; un `query` sans `vector` y est refusé en `invalid_request` (400), avec un message qui nomme le modèle. La CLI suit la même règle : `--query` et `--vector-file` ensemble sont acceptés en `hybrid`.

Dans `return.fields`, un nom déclaré `returned: false` dans le schéma est refusé en `invalid_request` (400) : le message nomme le champ, son chemin `$.return.fields[i]` et le fait qu'il est déclaré `returned: false` (ADR-0109). `"all"` n'est pas concerné : il ne rend que les champs `returned`.

Un champ **inconnu** du corps est refusé en `invalid_request` (400) : c'est le cas d'un nom absent du tableau ci-dessus, d'un nom absent de `fusion` ou de `return`, et de tout objet imbriqué, et le message nomme le **chemin JSON** du champ, comme pour un `PATCH` de configuration (ADR-0031, ADR-0109). Dans `filter`, une clause hors de la grammaire reste `invalid_filter` (400), avec `details.path` : son code ne change pas. `debug` est accepté depuis P7 (ADR-0119). `explain_ids` l'est depuis `EXPLIQUER-DOCUMENTS` (ADR-0123), avec la forme, les bornes et les codes d'erreur décrits ci-dessous. Aucun champ n'est ignoré en silence.

## Recherche : réponse

<!-- openapi: SearchResponse -->
```json
{
  "results": [
    { "doc_id": "LEGIARTI…", "passage": 0, "key": "LEGIARTI…#0",
      "score": 0.0312, "score_type": "rrf",
      "fields": { "num": "1240", "fonds": "LEGI" },
      "text": "Code civil > … > Article 1240\n\nTout fait quelconque de l'homme…" }
  ],
  "pool_size": 412337,
  "generation": 1287,
  "took_ms": 41.7,
  "truncated_query": false,
  "request_id": "…"
}
```

`took_ms` est la durée totale du traitement, **de la prise du jeton de concurrence à la fin de `fetch`** : elle couvre l'encodage de la requête ou sa lecture dans le cache, la planification, l'ouverture des lecteurs, les deux branches, la fusion, le regroupement et la lecture des payloads (ADR-0109). Elle **s'arrête avant le rendu JSON** des résultats, qui n'est pas du travail de recherche (ADR-0120, décision 1). C'est la même durée que celle de `crible search`, qui mesure depuis l'entrée de sa commande et arrête son total avant de construire sa réponse. Avec `debug: true`, `timings_ms.total` vaut cette même valeur : le total est arrêté une seule fois et sert à la réponse comme aux analytics (ADR-0119, décisions 5 et 7). L'échéance de `timeout_ms` et le jeton de concurrence, eux, couvrent toujours tout le travail de la requête, rendu compris (ADR-0120, décision 1).

Un résultat dont le document a été supprimé après la prise du snapshot garde son texte et ses champs de passage ; ses champs de document sont absents du résultat, sans marqueur et sans erreur (ADR-0043). C'est pourquoi `return.fields: "all"` signifie « tous les champs disponibles pour ce résultat ». Avec `group_by: "document"`, la réponse garde cette forme : une **liste plate** d'entrées de niveau passage, sans objet par document, et `doc_id` suffit à rattacher une entrée à son document (ADR-0075). `k` compte alors des **documents distincts**, et la liste porte au plus `k × passages_per_document` entrées. Les entrées d'un même document sont consécutives ; les documents sont rangés selon leur meilleur passage (score décroissant, puis clé croissante), et les passages d'un document par score décroissant puis clé croissante. Quand les candidats sur-échantillonnés sont épuisés avant `k` documents, la réponse en contient moins, sans erreur. Avec `dedup: "content"`, la fusion s'applique avant le regroupement. Aucun champ nouveau n'est ajouté aux résultats en P4.

Avec `debug: true`, la réponse ajoute un membre `debug` (ADR-0119) :

<!-- openapi: SearchDebug -->
```json
{
  "plan": { "mode": "hybrid", "dense_strategy": "binary", "rescore_candidates": 5000,
            "candidates_per_branch": 100, "fusion": "rrf", "group_by": null },
  "timings_ms": { "validate": 0.1, "encode": 30.0, "filter": 0.6, "dense": 9.8,
                  "lexical": 6.1, "fusion": 0.1, "group": 0.0, "fetch": 1.2, "total": 41.7 },
  "per_result": [ { "key": "LEGIARTI…#0", "dense_rank": 70, "dense_score": 0.61,
                    "lexical_rank": 2, "lexical_score": 14.2, "fused_rank": 1 } ]
}
```

`plan` est un objet fermé qui porte `mode`, `dense_strategy`, `rescore_candidates`, `candidates_per_branch`, `fusion` et `group_by`, plus `alpha` seulement quand `fusion` vaut `"relative_score"` (ADR-0119, décision 4). `mode` est le mode effectif ; `group_by` vaut `"document"` ou `null`. `candidates_per_branch` est le nombre **effectif** de résultats demandés par branche, sur-échantillonné quand la requête regroupe (ADR-0075). `dense_strategy` vaut `"exact"` ou `"binary"` (ADR-0074) : `"exact"` pour un pool de taille inférieure ou égale à `exact_threshold`, pour `exact: true` et pour `search.scan: none` ; `"binary"` sinon. Sous `"binary"`, `rescore_candidates` est le nombre **effectif** de candidats rescorés : il vaut `max(N, candidates_per_branch)`, donc `candidates_per_branch` quand ce nombre dépasse N. `dense_strategy` et `rescore_candidates` sont **absents** si aucune branche dense n'a tourné, par exemple en mode `lexical`.

`timings_ms` porte toujours les neuf clés de `03-recherche.md`, en millisecondes, `f64` arrondi à trois décimales ; une étape que le mode n'a pas exécutée vaut `0.0` (ADR-0119, décision 5). `total` est la valeur de `took_ms` : le chronomètre part de la prise du jeton et s'arrête à la fin de `fetch`, avant le rendu JSON (ADR-0109 ; ADR-0120, décision 1). La somme de cohérence n'est pas la somme des neuf clés : les branches dense et lexicale tournent en parallèle, et le chemin critique est `encode + filter + max(dense, lexical) + fusion + group + fetch`, `validate` exclu. Dans l'exemple, 30,0 + 0,6 + max(9,8 ; 6,1) + 0,1 + 0,0 + 1,2 = 41,7 ms, égal au total ; ajouter la branche lexicale donnerait 47,8 ms et compterait deux fois un temps parallèle (ADR-0119, décision 6).

`per_result` porte une entrée par résultat, dans l'ordre de `results`. `key` est la clé externe `doc_id#ord`, jamais un `iid`. `dense_rank` et `lexical_rank` sont les rangs dans la liste de sortie de chaque branche — `candidates_per_branch` entrées, après le filtre et avant `dedup` et `group_by`, à partir de 1, l'égalité étant départagée par la clé (ADR-0079 ; ADR-0119, décision 1). `dense_score` et `lexical_score` sont les scores de la branche, sans normalisation : cosinus pour le dense, BM25 pour le lexical. `fused_rank` est le rang dans la sortie de la fusion avant regroupement ; sous `group_by`, toutes les entrées d'un document portent le rang de son meilleur passage (ADR-0119, décision 2).

Un membre de branche qui n'a pas de valeur est **omis**, jamais rendu `null` ni `0` (ADR-0119, décision 3) : `dense_rank` et `dense_score` sont absents en mode `lexical`, `lexical_rank` et `lexical_score` en mode `dense`, et les membres d'une branche qui n'a pas trouvé le résultat le sont aussi. Un score nul est un score réel, un rang ne vaut jamais 0, et l'absence se lit à l'absence du membre.

`debug` ne change ni les résultats, ni leur ordre, ni `k`, ni l'échéance, ni `took_ms` : il ajoute seulement ce membre à une réponse `200`. Une réponse d'erreur, `504` compris, n'en porte jamais (ADR-0119, décision 10).

Avec `explain_ids`, chaque document demandé reçoit un diagnostic (ADR-0123) : `explain` est un tableau **frère** de `results`, avec une entrée par id demandé, dans l'ordre de la requête.

<!-- openapi: SearchExplain -->
```json
{ "explain": [
  { "id": "LEGIARTI…", "doc_id": "LEGIARTI…", "exists": true, "alive": true,
    "in_pool": false, "excluded_by": ["$.filter.date_debut"] },
  { "id": "LEGIARTI…#2", "doc_id": "LEGIARTI…", "exists": true, "alive": true, "in_pool": true,
    "dense":   { "key": "LEGIARTI…#2", "exact_rank": 70, "binary_rank": 233,
                 "in_binary_candidates": true },
    "lexical": { "key": "LEGIARTI…#2", "rank": 2, "score": 14.2,
                 "terms": [ { "term": "responsabilité", "tf": 3, "score": 9.8 },
                            { "term": "chose", "tf": 2, "score": 4.4 } ] },
    "fused":   { "rank": 1 } }
] }
```

- `id` est l'identifiant **tel que demandé**, `doc_id` ou clé `doc_id#ord` ; `doc_id` est toujours le document. Un id est une **clé** quand il se termine par `#` suivi d'un entier décimal valide comme `ord`, l'analyse coupant au **dernier** `#` ; tout autre id est un `doc_id`, même s'il contient `#` (`A#x` est un `doc_id`, pas une erreur). En conséquence, un document dont le `doc_id` se termine par `#<entier>` s'explique **passage par passage** (`A#3#0`), et le seul id refusé est l'id vide, ou celui dont la partie document est vide ou trop longue (ADR-0123, complément du 2026-11-01).
- `exists` et `alive` gardent leur définition unique (ADR-0041) : la ligne `documents` est présente, et le document a au moins un passage vivant. Ils restent distincts ; `exists: false` signifie que le `doc_id` n'a jamais existé ou a été supprimé. `alive` est **absent** quand `exists` est faux.
- `in_pool` dit si l'id demandé est dans le pool filtré : pour une **clé**, le passage lui-même ; pour un `doc_id`, au moins un de ses passages. Pour une clé, tous les membres décrivent ce passage, jamais son document ni un passage frère. `excluded_by` n'est présent que hors du pool : c'est la liste des **feuilles** du filtre qui échouent (ADR-0123, décision 7), chacune par son chemin JSON complet (`$.filter.$or[1].fonds`), dans l'ordre du filtre ; un nœud `$and`, `$or` ou `$not` n'y figure jamais. Pour un `doc_id`, c'est l'**union**, dans l'ordre du filtre, des feuilles qui échouent pour **au moins un** de ses passages vivants : elle n'est donc jamais vide quand le document a des passages vivants et que `in_pool` est faux, **sauf sous un `$not` de tête** — un document exclu par `$not: { "fonds": "A" }` satisfait la condition niée, donc aucune feuille n'échoue pour lui et la liste est vide —, et elle est absente quand il n'a aucun passage vivant, ou quand cette liste est vide, puisque tout membre sans valeur est omis (ADR-0123, décision 7 et complément du 2026-11-01).
- Suivent les objets de branche — `dense` `{key, exact_rank, binary_rank, in_binary_candidates}`, `lexical` `{key, rank, score, terms}`, `fused` `{rank}` — puis `grouped_under`. Une branche qui n'a pas tourné n'a pas d'objet ; `fused` reste présent en mode `dense` ou `lexical`, avec le rang de la branche unique (ADR-0119, décision 2). Quand l'id demandé est un `doc_id`, chaque branche décrit le passage **le mieux classé** du document dans cette branche et le nomme par sa `key`, les trois `key` pouvant différer.
- `grouped_under` est la clé du passage rendu dans `results` qui a absorbé celui-ci, par `dedup: "content"` ou par `group_by: "document"` ; sous `dedup`, on cherche d'abord le passage rendu de **même empreinte de contenu**, puis, sous regroupement ou déduplication par document, le passage rendu du **même document** ; il est absent quand le passage n'a pas été absorbé (ADR-0123, décision 5 et complément du 2026-11-01).
- Tous les rangs partent de 1 et **ne sont pas sur la même échelle** que `per_result.dense_rank` et `per_result.lexical_rank` de `debug`, qui comptent les `candidates_per_branch` entrées de la branche (ADR-0119 ; ADR-0123, décision 5). `exact_rank` est le rang sur **tout le pool filtré** au score dense exact, indépendant de `k`, l'égalité départagée par la clé (ADR-0079). `binary_rank` est le rang dans l'ordre exact de la **sélection binaire sur le pool filtré**, indépendant de `k` ; `in_binary_candidates` vaut vrai si et seulement si le passage fait partie des candidats retenus, c'est-à-dire si son rang binaire est au plus `max(N, candidates_per_branch)` ; sous stratégie exacte, ces deux membres sont absents. `lexical.rank` est le rang BM25 parmi les passages du pool qui ont un score, indépendant de `k`. `fused.rank` est le rang dans la liste fusionnée **avant regroupement**, comme `fused_rank` d'ADR-0119, et il est absent si le passage n'y figure pas.
- `terms` est fermé : un élément par **terme analysé** de la requête présent dans le passage, trié par `score` décroissant, et la somme des `score` égale `lexical.score` à `1e-4` près. Aucune `Explanation` tantivy brute n'est rendue (ADR-0123, décision 6).
- Un membre sans valeur est **omis**, jamais `null` ni `0`, comme pour `debug` (ADR-0119, décision 3).
- **Bornes** (ADR-0123, décision 3) : au plus **100 ids**, sinon `400 invalid_request` sur `$.explain_ids` ; les doublons sont fusionnés en gardant la première occurrence ; un id vide ou une clé mal formée rend `400 invalid_request` en nommant `$.explain_ids[i]`, une clé mal formée étant un id dont la partie document d'une clé est vide ou trop longue — un id qui contient `#` sans suffixe numérique est un `doc_id`, pas une clé fautive ; un `doc_id` inconnu rend `exists: false`, sans erreur.
- **Coût** (ADR-0123, décision 4) : `exact_rank` impose un scan exact du pool filtré, sous toute stratégie dense et seulement quand `explain_ids` est présent. Les deux seuls travaux liés à `explain_ids` compris dans `took_ms` sont des copies en mémoire faites avant l'arrêt du total : la **capture des rangs avant regroupement**, nécessaire à `fused.rank` et à `grouped_under`, et la **capture des survivants** du regroupement, leurs `k` clés et leur `doc_id`, sans lecture d'empreinte, que la route multi-index extrait avant sa fusion (ADR-0123, erratum du 2026-11-01) ; tout le reste — scans exacts, rangs binaires et lexicaux, passages absorbés, empreintes de contenu — se calcule **après `fetch`**, hors de `took_ms` et de `timings_ms`, mais **sous l'échéance** : une explication qui ne finit pas dans `timeout_ms` rend le même `504 timeout` qu'une recherche lente. Sous `explain_ids`, `lexical.rank` demande de scorer tout le pool marqué par la requête, coût non mesuré à 8,58 M passages (ADR-0123, complément du 2026-11-01).
- `explain_ids` ne change ni les résultats, ni leur ordre, ni `k`, ne dépend pas de `debug: true`, n'entre ni dans le journal de debug ni dans les analytics, et ne rend aucun `iid` (ADR-0121 ; ADR-0123, décisions 8 et 9).

## Recherche multi-index

`POST /v1/search`

<!-- openapi: SearchMultiRequest -->
```json
{ "query": "responsabilité du fait des choses", "mode": "hybrid", "k": 10,
  "targets": [ { "index": "articles",  "filter": { "fonds": "LEGI" } },
               { "index": "decisions", "filter": { "partie": { "nin": ["moyens", "faits"] } } } ],
  "group_by": "document" }
```

- **Corps** (ADR-0110). Tous les champs de « Recherche : requête » sont **communs, au premier niveau**, avec la même sémantique et les mêmes bornes : `query`, `vector`, `mode`, `k`, `fusion`, `group_by`, `passages_per_document`, `dedup`, `return`, `exact`, `debug`, `timeout_ms`. `explain_ids` prend la forme `{ "index", "id" }` (ADR-0123), et non plus le refus de P6. Seul `filter` est propre à une cible : `targets: [{ "index": "...", "filter": {...} }]`, `index` obligatoire, `filter` facultatif (absent = aucun filtre).
- **Champ inconnu** (ADR-0110). La règle de la route simple s'étend à ce corps, **à tous les niveaux**, objets de `targets[i]` compris : `400 invalid_request`, message nommant le **chemin JSON** du champ (`$.targets[0].filtre`). Dans un filtre, une clause hors grammaire reste `invalid_filter`, avec `details.path` préfixé par `$.targets[i].filter`.
- **`fusion`** (ADR-0110). Absent, chaque index applique sa fusion configurée. Présent, la valeur s'applique à **toutes** les cibles.
- **`mode` par défaut** (ADR-0110). `hybrid` si **toutes** les cibles ont le lexical activé, sinon `dense`. Un `mode` explicite `lexical` ou `hybrid` avec une cible sans lexical reste refusé en entier, avec le message « lexical branch is disabled for this index » et le nom de l'index fautif ; le corps porte le même `details` que la route simple, `{"path": "$.mode", "reason": "lexical_branch_disabled"}`, sans clé `details.index` ; aucune cible n'est servie en dense à sa place (ADR-0089, ADR-0145). Le mode `dense` reste disponible sur les mêmes cibles.
- **Modèle commun** (ADR-0110). En `dense` et en `hybrid`, toutes les cibles doivent avoir la même empreinte de modèle (ADR-0028), modèles `external` compris. Un `external` et un `onnx`, ou deux modèles différents, rendent `422 incompatible_model`, et le message nomme **deux index d'empreintes différentes** : la première cible et la première qui diffère. En `lexical`, le modèle n'est pas comparé. Les règles d'entrée d'ADR-0109 s'appliquent au modèle commun : sur des cibles `external`, `dense` exige `vector`, `hybrid` exige `query` **et** `vector`.
- **`targets`** (ADR-0110). Absent ou vide : `400 invalid_request`. Un même index deux fois : `400 invalid_request`, message nommant `$.targets[i].index` ; il compterait deux fois dans le RRF. Un index inconnu : `404 not_found` nommant l'index, **aucun résultat partiel**. Aucune borne propre sur le nombre de cibles : les doublons et les inconnus étant refusés, il est borné par le nombre d'index enregistrés et la taille de corps (ADR-0102).
- **`return.fields`** (ADR-0110). Sur des schémas hétérogènes, une liste de noms est acceptée si chaque nom est déclaré par au moins une cible ; un nom déclaré par aucune cible est refusé en `400 invalid_request`, avec `$.return.fields[i]`. Un nom déclaré `returned: false` dans une cible est refusé en `400`, et le message nomme le champ, `$.return.fields[i]`, l'index et `returned: false` (ADR-0109). Chaque résultat porte ceux des champs demandés que son index déclare et qui sont disponibles pour lui (ADR-0043) ; `"all"` rend tous les champs `returned` de l'index du résultat.
- **`explain_ids`** (ADR-0123). Chaque élément du corps est un objet `{ "index", "id" }`, `index` devant être l'une des cibles de `targets` ; un index hors des cibles rend `400 invalid_request`, avec le chemin `$.explain_ids[i].index`. La réponse est une **liste plate** `explain`, dans l'ordre de la requête, dont chaque entrée porte `index` en plus des membres de la route simple ; les rangs d'une entrée sont ceux de son index, et `fused.rank` est la fusion **de l'index**, avant son regroupement. La borne de 100 ids vaut pour la requête entière, toutes cibles confondues ; les doublons se jugent sur le couple `index` et `id`. L'explication se calcule après `fetch`, hors de `took_ms`, sous l'échéance.
- **Ordre des validations** (ADR-0110, première faute rendue) : (a) forme du corps, champs inconnus, bornes, entrées selon le mode, `targets` vide et doublons, en `400` ; (b) existence de chaque index, dans l'ordre de `targets`, en `404` ; (c) pour chaque cible, dans l'ordre, lexical désactivé (`400`), filtre contre son schéma (`invalid_filter`), puis `return.fields` contre l'ensemble des cibles (`400`) ; (d) modèle commun, en `422` ; (e) prise du jeton de concurrence, en `503 overloaded` ; (f) disponibilité lexicale d'une cible pendant l'exécution, en `503 overloaded` (ADR-0088, ADR-0089).
- **Exécution** (ADR-0110). **Un seul** jeton de concurrence pour toute la requête, même sémaphore et même `[server].max_concurrent_searches` que la route simple, et **une seule** échéance `timeout_ms`, encodage compris (ADR-0102, ADR-0109) ; au-delà, `504 timeout`, le jeton restant tenu jusqu'à la fin réelle du calcul. La requête est encodée **une fois**, par le même chemin et le même cache d'instance que la route simple (ADR-0051), et le vecteur sert toutes les cibles. Les cibles s'exécutent **l'une après l'autre**, dans l'ordre de `targets` : par cible, pool, branches, fusion, `dedup` puis `group_by` (ADR-0075), puis top-`k` de l'index, comme sur la route simple. Jamais de `dedup` ni de regroupement entre index : un même `passage_hash` présent dans deux index reste deux entrées.
- **Fusion entre index** (ADR-0110). L'unité est le passage, ou le document sous `group_by: "document"`, ses entrées restant consécutives dans leur ordre de l'index. En `dense`, les unités sont triées par `score` cosinus décroissant (score du meilleur passage pour un document), puis par nom d'`index` croissant, puis par clé croissante, et `score_type` vaut `cosine` ; le résultat égale le tri global des top-k de chaque index. En `lexical` et en `hybrid`, le score est un RRF sur les rangs de chaque index, `1 / (60 + rang)`, où `rang` est le rang de l'unité dans la liste finale de son index, à partir de 1 ; le tri est par ce score décroissant, puis `index` croissant, puis clé croissante, et `score_type` vaut `rrf`, y compris avec une seule cible. Toutes les entrées d'un document regroupé portent le score de son document. La troncature finale porte sur `k` unités : `k` résultats, ou `k` documents distincts (paires `index`, `doc_id`) et au plus `k × passages_per_document` entrées.

La réponse garde la forme plate de la route simple, et chaque entrée porte `index` en plus de `doc_id`, `passage`, `key`, `score`, `score_type`, `fields` et `text`. Au lieu de `pool_size` et `generation` scalaires, elle porte `targets: [{ "index", "generation", "pool_size" }]`, dans l'ordre de la requête (ADR-0110) :

<!-- openapi: SearchMultiResponse -->
```json
{
  "results": [
    { "index": "articles", "doc_id": "LEGIARTI…", "passage": 0, "key": "LEGIARTI…#0",
      "score": 0.0312, "score_type": "rrf", "fields": { "num": "1240" }, "text": "…" }
  ],
  "targets": [ { "index": "articles", "generation": 1287, "pool_size": 412337 },
               { "index": "decisions", "generation": 88, "pool_size": 51204 } ],
  "took_ms": 41.7,
  "truncated_query": false,
  "request_id": "…"
}
```

`took_ms` couvre la requête entière, **de la prise du jeton à la fin de `fetch`**, avant le rendu JSON, encodage compris (ADR-0109 ; ADR-0120, décision 1). `truncated_query` a la même définition que sur la route simple et n'a qu'une valeur, puisque l'encodage est unique ; il vaut `false` si rien n'est encodé.

Avec `debug: true`, la réponse ajoute un membre `debug` (ADR-0119) :

<!-- openapi: SearchMultiDebug -->
```json
{
  "targets": [
    { "index": "articles",
      "plan": { "mode": "hybrid", "dense_strategy": "binary", "rescore_candidates": 5000,
                "candidates_per_branch": 100, "fusion": "rrf", "group_by": "document" },
      "timings_ms": { "validate": 0.0, "encode": 0.0, "filter": 0.6, "dense": 9.8,
                      "lexical": 6.1, "fusion": 0.1, "group": 0.2, "fetch": 1.2, "total": 11.9 } }
  ],
  "timings_ms": { "validate": 0.1, "encode": 30.0, "filter": 0.6, "dense": 9.8,
                  "lexical": 6.1, "fusion": 0.1, "group": 0.2, "fetch": 1.2, "total": 41.9 },
  "per_result": [ { "index": "articles", "key": "LEGIARTI…#0", "dense_rank": 70,
                    "dense_score": 0.61, "lexical_rank": 2, "lexical_score": 14.2,
                    "fused_rank": 1, "target_rank": 1 } ]
}
```

`targets` porte une entrée par cible, dans l'ordre de la requête, avec le `plan` et le `timings_ms` de cette cible (ADR-0119, décision 9). Les deux étapes `validate` et `encode` y valent `0.0`, parce que la validation et l'encodage ont lieu **une fois** pour la requête et appartiennent au `timings_ms` global ; le `total` d'une cible est la durée de son travail, du filtre à la fin de `fetch`, avant le rendu JSON (ADR-0120, décision 1), et c'est à lui que s'applique la règle de cohérence de la route simple (ADR-0119, décision 6). Le `timings_ms` global porte les neuf clés : `validate` et `encode` mesurés une fois, les six autres étapes **sommées** sur les cibles séquentielles, la fusion entre index comprise dans `fusion`. Son `total` vaut `took_ms` ; il n'est pas un chemin critique et n'est pas soumis au test d'écart de 10 %.

`per_result` porte une entrée par résultat, dans l'ordre de `results`, avec `index` et `key` en plus des membres de la route simple. Les rangs et scores de branche gardent la définition de la route simple, prise dans l'index de l'entrée. `fused_rank` est le rang dans la fusion **de l'index**, avant son regroupement. `target_rank` est le rang de l'unité dans la liste finale de son index, à partir de 1 : en `lexical` et en `hybride`, c'est exactement le rang du `1 / (60 + rang)` de la fusion entre index (ADR-0110) ; en `dense`, c'est le rang dans la liste triée par cosinus de l'index. Sous `group_by`, toutes les entrées d'un document portent le rang de son meilleur passage. Le rang dans la fusion **entre index** n'a pas de membre : c'est la position de l'entrée dans `results`.

Les membres absents suivent la même règle qu'en simple : on les omet, jamais `null`, jamais `0` (ADR-0119, décision 3). `explain_ids` prend la forme `{ "index": "...", "id": "..." }`, et la réponse une liste plate dont chaque entrée porte `index` (ADR-0110 ; ADR-0123).

## Jobs

<!-- openapi: Job -->
```json
{ "id": "job_…", "kind": "attach_batch", "index": "decisions", "state": "succeeded",
  "created_at": "…", "started_at": "…", "finished_at": "…", "generation": 1288,
  "progress": { "step": "snapshot", "done": 932, "total": 932 },
  "durations": { "validation_ms": 1234, "segment_ms": 5678, "transaction_ms": 90,
                 "lexical_ms": 0, "manifest_ms": 12, "snapshot_ms": 34 },
  "result": { "report": { "added": 421, "replaced": 428, "unchanged": 1230,
                          "metadata_updated": 137, "deleted": 221, "ignored": 3,
                          "acceptance": { "sample": 200, "min_cosine": 0.99971, "p1": 0.9998, "p50": 0.99995 } } },
  "error": null }
```

- Le rapport donne six comptes **par document et mutuellement exclusifs** : `added`, `replaced`, `unchanged`, `metadata_updated`, `deleted`, `ignored` (ADR-0041, et `docs/04-donnees-et-ingestion.md` pour la définition de chaque clé). Leur somme vaut le nombre de documents traités.

Un lot **sans passage**, qui ne porte que des `deletions`, n'a ni ajout ni recette : son rapport ne compte que des suppressions. Deux `doc_id` inconnus de l'index y donnent ce job :

<!-- openapi: Job -->
```json
{ "id": "job_…", "kind": "online_batch", "index": "decisions", "state": "succeeded",
  "created_at": "…", "started_at": "…", "finished_at": "…", "generation": 1289,
  "progress": { "step": "snapshot", "done": 0, "total": 0 },
  "durations": { "validation_ms": 3, "segment_ms": 1, "transaction_ms": 2,
                 "lexical_ms": 0, "manifest_ms": 1, "snapshot_ms": 1 },
  "result": { "report": { "generation": 1289, "added": 0, "replaced": 0, "unchanged": 0,
                          "metadata_updated": 0, "deleted": 0, "ignored": 2,
                          "acceptance": null } },
  "error": null }
```

- `ignored` compte les `doc_id` de `deletions` que l'index ne connaît pas ; c'est le seul compte qu'une suppression de document inconnu peut produire (`04-donnees-et-ingestion.md`, « Contrôles et recette, dans l'ordre », tableau du rapport d'attache). Le rapport ne dit pas **pourquoi** une suppression est ignorée : sa forme est celle des six compteurs (ADR-0041), sans compteur par cause ; INV-01 ne vise que l'`iid`, jamais le `doc_id`.
- `acceptance: null` veut dire « aucun passage à recetter », jamais « recette échouée » : le lot ne porte aucun passage, il n'y a rien à tirer au sort (`04-donnees-et-ingestion.md`, « Contrôles et recette, dans l'ordre », étape 9 ; ADR-0054). Une recette qui refuse un lot fait échouer le job, avec `error.code` à `acceptance_failed` (même étape 9 ; ci-dessus, « Erreurs d'attache »).
- Le corps d'un tel lot est décrit dans « Lot en ligne », qui donne aussi le sens d'`acceptance: null` pour un lot sans passage ; les définitions d'`acceptance: null` et d'`ignored` sont dans `04-donnees-et-ingestion.md`, « Contrôles et recette, dans l'ordre ». Son rapport suit la règle des six comptes ci-dessus, et `progress` compte des passages, d'où `done` et `total` à `0`.
- `progress` n'est publié que par `attach_batch`, `online_batch` et `patch_documents` ; les autres `kind` gardent `progress: null`, membre toujours présent. `progress.done` et `progress.total` comptent les **passages** traités pour `attach_batch` et `online_batch`, comme `counts.passages` du manifeste, et les **documents** pour `patch_documents` ; ils ne se comparent pas aux comptes du rapport. `progress.step` prend les noms d'étapes du protocole : `validation`, `segment`, `transaction`, `lexical`, `manifest`, `snapshot` pour `attach_batch` et `online_batch`, et `transaction`, `manifest`, `snapshot` pour `patch_documents` ; une étape que le protocole saute pour un job donné n'est pas publiée. `done` avance aux **frontières d'étape**, pas passage par passage : `total` est connu dès la lecture du manifeste ou du corps du patch, `step` prend son nom au début de chaque étape, et `done` vaut `total` à la fin d'un job réussi — l'exemple ci-dessus, `succeeded`, est à cette dernière étape. Un job réussi qui n'a joué aucune étape, comme le rejeu d'un lot déjà attaché, garde `progress: null` : un client ne doit donc pas supposer `done = total` sur tout job réussi (ADR-0134). Un job échoué garde la dernière progression publiée, éventuellement avec `done < total`, ou `null` si l'échec précède la première publication. Pendant l'encodage d'un `online_batch`, avant que le lot interne et son manifeste existent, `progress` reste `null` ; il suit ensuite les étapes de l'attache, en passages. Pendant le job, `progress` est lu **en mémoire** ; la table `jobs` n'est écrite qu'aux transitions d'état et porte alors la progression de l'instant (ADR-0103, décision 3 ; ADR-0134).
- `result.report` est le rapport **persisté** de l'attache : il ne porte pas de durées. Les durées par étape sont de la télémétrie d'appel (ADR-0042) : le serveur les rend dans le membre `durations` de l'objet `Job`, gardé **en mémoire** et **jamais persisté**, et les **journalise une fois, à la fin du job**, au niveau `info`, avec `job_id`, `kind` et `index` (ADR-0149). La CLI continue de les afficher à son appelant. Pour `patch_documents`, le rapport `{ updated, unchanged, generation }` n'est **pas** persisté : le job le porte en mémoire, pour sa durée de vie, et un job relu plus tard n'en donne rien (ADR-0060).
- `durations` est un membre **toujours présent** de l'objet `Job`, `null` quand il n'y a rien à rendre. Quand il porte un objet, celui-ci a exactement six clés, les noms de la ligne de journal `attach durations`, en millisecondes entières : `validation_ms`, `segment_ms`, `transaction_ms`, `lexical_ms`, `manifest_ms`, `snapshot_ms` ; une étape que le protocole n'a pas jouée vaut `0`, jamais `null`. Il porte exactement les durées que la ligne de journal publie, quand elle est publiée, et vaut `null` sinon : pour un job `queued` ou `running`, pour un job `failed`, pour un rejeu d'un `batch_id` déjà appliqué, pour un job d'une autre nature que `attach_batch` et `online_batch`, et pour un job relu après un redémarrage, le membre n'étant jamais écrit dans `jobs.sqlite`. Il est rendu par `GET /v1/jobs/{job_id}`, par `GET /v1/jobs` et par la réponse de `?wait=true` des deux routes d'attache, qui rendent le même objet `Job`, et il est lisible par toute clé `write` ou `admin`, sans restriction à la clé qui a soumis ; `batches.report` et `jobs.sqlite` ne changent pas (ADR-0149).
- `kind` vaut `attach_batch`, `online_batch`, `patch_documents`, `patch_config`, `compact`, `export`, `snapshot`, `rebuild_lexical` ou `delete_index`. `patch_config` est le patch d'un réglage de l'index qui ne reconstruit pas le lexical, et `rebuild_lexical` celui qui le reconstruit (ADR-0105).
- `state` vaut `queued`, `running`, `succeeded` ou `failed`.
- **Redémarrage** : un job resté `queued` ou `running` quand le serveur s'arrête passe à `failed` au démarrage suivant, avec `finished_at` à l'heure du démarrage, `error.code` à `internal` et le message fixe « the job was interrupted by a server restart ». Aucun job n'est rejoué : le client resoumet, et une attache est idempotente par `batch_id` (ADR-0103).
- **Erreur d'un job** : `{ "code", "message", "details" }`, sans `request_id` — un job n'est pas une requête —, avec les règles du corps d'erreur d'ADR-0101 : message générique et fixe pour `internal`, cause journalisée avec le `job_id` (ADR-0103).
- **Rétention** : les jobs terminés depuis plus de 30 jours sont purgés, de la table et de la mémoire, au démarrage et à la fin de chaque job ; les jobs en cours ne sont jamais purgés. La valeur est fixe, sans clé de configuration. Un job inconnu ou purgé rend `404 not_found` (ADR-0103).
- **Liste** : `GET /v1/jobs` rend tous les jobs retenus, dans un objet à une seule clé, `{ "jobs": [ … ] }` (ADR-0103), filtrables par `index` et par `state`, triés par `created_at` décroissant puis `id`, sans pagination en v1 ; une valeur de `state` hors des quatre états rend `invalid_request` (400). Aucune isolation par clé : toute clé `write` ou `admin` voit les jobs de l'instance (ADR-0103).
- **Arrêt du serveur** : dès le début du drainage, toute nouvelle soumission rend `503 overloaded` avec le message « the server is shutting down » ; les jobs acceptés avant le signal se terminent, et l'attente est bornée par `wait_timeout_ms` comptée depuis le signal, en parallèle du drainage des requêtes. À l'échéance, le processus sort et les jobs non terminés passent à `failed` au démarrage suivant (ADR-0104).

## Journal de debug

`GET /v1/debug/queries` lit le journal de debug, **réservé au rôle `admin`** : une clé `read` ou `write` reçoit `403 forbidden` (ADR-0095 ; ADR-0122, décision 9). Le journal est **désactivé par défaut** ; quand il l'est, la route rend `200` avec `enabled: false` et `queries: []`, sans ouvrir `debug.sqlite` (ADR-0121 ; ADR-0122, décision 9).

| Paramètre | Type | Défaut | Rôle |
|---|---|---|---|
| `index` | chaîne | — | nom exact pour une ligne d'index simple ; appartenance à `payload.targets[]` pour une ligne multi-index |
| `min_total_ms` | nombre | — | garde les lignes dont `total_ms` est supérieur ou égal à la valeur ; une ligne à `total_ms` `NULL` est exclue |
| `from` | chaîne ISO 8601 | — | borne basse incluse sur `created_at` |
| `to` | chaîne ISO 8601 | — | borne haute incluse sur `created_at` |
| `limit` | entier | 100 | nombre de lignes, de 1 à 1 000 |
| `before_id` | entier | — | pagination par clé : ne rend que les lignes d'`id` strictement inférieur |

<!-- openapi: DebugQueryList -->
```json
{ "enabled": true, "dropped": 0,
  "capped": false, "rows": 1, "bytes": 65536, "trimmed": 0,
  "queries": [
    { "id": 42, "created_at": "2026-10-31T10:00:00.123Z", "request_id": "…",
      "index": "decisions", "client": "legifacile-mcp", "mode": "hybrid",
      "status": "ok", "total_ms": 41.7,
      "payload": { "request": { "query": "responsabilité du fait des choses",
                                "filter": { "fonds": "LEGI" } },
                   "generation": 1287 } } ] }
```

- Note du 2026-11-02 : les quatre membres `capped`, `rows`, `bytes` et `trimmed`, annoncés dans la prose ci-dessous, ont été rétablis dans cet exemple par `PLAFONNER-JOURNAL-DE-DEBUG` (P8, ADR-0127), en même temps que leur implémentation et le composant `DebugQueryList` qui les déclare.
- L'ordre est `created_at DESC, id DESC`, un ordre total : deux lignes du même vidage partagent `created_at`, et `id` les départage. Le client reprend le dernier `id` reçu pour la page suivante ; il n'y a pas d'`OFFSET` (ADR-0122, décision 9).
- `payload` est rendu comme un **objet JSON**, jamais comme une chaîne ; sa forme est celle d'ADR-0121, décision 4. Un `payload` illisible rend `internal`.
- En multi-index, `index` vaut `null` et `payload.targets[]` porte les cibles, avec leur filtre, leur génération et leur capture (ADR-0122, décision 4).
- `dropped` est le compteur de pertes de file du processus, `debug_journal_dropped` (ADR-0122, décision 6).
- `capped`, `rows`, `bytes` et `trimmed` décrivent le plafond de taille de `debug.sqlite` (ADR-0127, décisions 3 et 6) : `capped` vaut vrai quand le dernier contrôle a trouvé le journal au plafond, et reste vrai jusqu'au redémarrage ; `rows` est le nombre de lignes du journal ; `bytes` est sa taille **utilisée**, `(page_count - freelist_count) × page_size` ; `trimmed` est le nombre de lignes supprimées par le plafond depuis le démarrage. Ce sont des compteurs de processus, jamais persistés. Journal désactivé, ils valent `false`, `0`, `0` et `0`. La taille physique du fichier n'est pas exposée : elle s'obtient avec les outils du système.
- L'objet reste **fermé** (ADR-0111) : ces quatre membres existent toujours, `enabled`, `dropped` et `queries` compris, que le journal soit activé ou non (ADR-0127, décision 6).
- Un paramètre **inconnu** ou **invalide** (`limit` hors de `[1, 1 000]`, `before_id` négatif, date qui n'est pas ISO 8601, `min_total_ms` négatif ou non numérique) rend `400 invalid_request`, avec un message qui nomme le paramètre (ADR-0109 ; ADR-0122, décision 9).
- La route est déclarée dans la spécification OpenAPI et son exception nommée est retirée du test des routes dans le même changement (ADR-0111, complément du 2026-10-30).

## Statistiques

`GET /v1/stats` lit les analytics agrégées, **réservé au rôle `admin`** : une clé `read` ou `write` reçoit `403 forbidden` (ADR-0095 ; ADR-0124, décision 16). Les chiffres portent sur ce qui est **vidé** dans `analytics.sqlite` : une requête de la dernière seconde peut manquer, et le décalage est borné par l'intervalle de vidage de 10 s (ADR-0118, décision 3 ; ADR-0124, décision 11).

| Paramètre | Type | Défaut | Rôle |
|---|---|---|---|
| `from` | chaîne ISO 8601 | plus ancienne ligne conservée | borne basse **incluse**, normalisée en UTC |
| `to` | chaîne ISO 8601 | plus récente ligne conservée | borne haute **incluse**, normalisée en UTC |
| `index` | chaîne | — | filtre d'égalité sur le nom d'index ; porte sur les noms présents dans les analytics, jamais sur les index vivants |
| `operation` | chaîne | — | filtre d'égalité sur une opération de la liste fermée de `06-observabilite.md` |
| `group_by` | `minute` · `hour` · `day` | `hour` | largeur des seaux |

<!-- openapi: StatsResponse -->
```json
{
  "from": "2026-10-31T00:00:00Z",
  "to": "2026-10-31T23:59:00Z",
  "group_by": "hour",
  "buckets": [
    { "start": "2026-10-31T08:00:00Z",
      "count": 4120,
      "errors": { "invalid_filter": 3, "timeout": 1 },
      "zero_results_rate": 0.0612,
      "latency_ms": {
        "validate": { "p50": 0.1, "p95": 0.14, "p99": 0.17 },
        "encode":   { "p50": 30.0, "p95": 47.5, "p99": 59.4 },
        "filter":   { "p50": 0.6, "p95": 1.0, "p99": 1.25 },
        "dense":    { "p50": 9.8, "p95": 15.3, "p99": 19.1 },
        "lexical":  { "p50": 6.1, "p95": 9.5, "p99": 11.9 },
        "fusion":   { "p50": 0.1, "p95": 0.13, "p99": 0.16 },
        "group":    { "p50": 0.1, "p95": 0.13, "p99": 0.16 },
        "fetch":    { "p50": 1.2, "p95": 1.5, "p99": 1.9 },
        "total":    { "p50": 41.7, "p95": 59.4, "p99": 74.3 }
      } }
  ],
  "totals": {
    "count": 4120,
    "errors": { "invalid_filter": 3, "timeout": 1 },
    "zero_results_rate": 0.0612,
    "latency_ms": {
      "validate": { "p50": 0.1, "p95": 0.14, "p99": 0.17 },
      "encode":   { "p50": 30.0, "p95": 47.5, "p99": 59.4 },
      "filter":   { "p50": 0.6, "p95": 1.0, "p99": 1.25 },
      "dense":    { "p50": 9.8, "p95": 15.3, "p99": 19.1 },
      "lexical":  { "p50": 6.1, "p95": 9.5, "p99": 11.9 },
      "fusion":   { "p50": 0.1, "p95": 0.13, "p99": 0.16 },
      "group":    { "p50": 0.1, "p95": 0.13, "p99": 0.16 },
      "fetch":    { "p50": 1.2, "p95": 1.5, "p99": 1.9 },
      "total":    { "p50": 41.7, "p95": 59.4, "p99": 74.3 }
    }
  },
  "top_indexes": [
    { "index": "articles", "count": 3800 },
    { "index": "decisions", "count": 320 }
  ],
  "top_filter_shapes": [
    { "shape": "date_debut:lte,fonds:eq", "count": 2140 }
  ],
  "indexes": [
    { "day": "2026-10-31", "index": "articles", "documents": 412000,
      "live_passages": 8580000, "dead_passages": 12400, "dead_ratio": 0.00144,
      "disk_bytes": 21474836480, "generation": 1288, "batches": 932 }
  ],
  "analytics_dropped": 0
}
```

- La réponse est un **objet à membres fixes** : `from`, `to`, `group_by`, `buckets`, `totals`, `top_indexes`, `top_filter_shapes`, `indexes`, `analytics_dropped`. Un membre **sans valeur est omis**, jamais `null` ni `0` inventé (ADR-0119, décision 3 ; ADR-0124, décision 1). `from` et `to` sont les échos **normalisés** des bornes appliquées, en UTC, et `group_by` vaut `minute`, `hour` ou `day`.
- `buckets` est **chronologique** et ne porte que les seaux qui ont au moins un événement : `start` est le début du seau tronqué à la minute, à l'heure ou au jour, `count` la somme des événements du seau. `errors` est un objet **code → nombre**, les seuls codes présents, et il est **omis** quand le seau ne porte aucune erreur. `totals` a la même forme qu'un seau, sur toute la fenêtre, **sans `start`**.
- `zero_results_rate` est la somme des `zero_results` divisée par la somme des `count` des événements de recherche (`search` et `search_multi`) de statut `ok`, par seau et sur la fenêtre ; il est **omis** quand le dénominateur est nul. Une recherche multi-index **pèse une fois par cible**, parce qu'elle produit un événement par cible (ADR-0117, décision 5 ; ADR-0124, décision 5).
- `latency_ms` porte, pour **chaque étape de `StageDurations`** (`validate`, `encode`, `filter`, `dense`, `lexical`, `fusion`, `group`, `fetch`, `total`) qui a au moins un échantillon dans le seau, `p50`, `p95` et `p99` en millisecondes : la valeur est la **borne haute** du seau d'histogramme qui contient le rang le plus proche, donc exacte sur la somme des seaux et sur-estimée à la résolution d'un seau (ADR-0117, décision 4 ; ADR-0124, décision 4). Une étape sans échantillon est **omise** : une branche qui n'a pas tourné n'a pas de percentile. Les percentiles de taille de pool ne sont pas rendus.
- `top_filter_shapes` rend **au plus 10** formes `{ "shape", "count" }` sur la fenêtre, par `count` décroissant, l'égalité étant départagée par l'ordre lexicographique croissant de la forme. La forme ne contient que des noms de champs et d'opérateurs, jamais une valeur (INV-13 ; ADR-0117, décision 6 ; ADR-0124, décision 6). Le classement porte sur les **recherches** de la fenêtre **quel que soit `operation`** demandé : la table `filter_shapes` ne porte pas de colonne d'opération, seules les recherches y écrivent une forme, et `?operation=compact` ne restreint donc pas ce classement (ADR-0124, note du 2026-11-01). `top_indexes` rend **au plus 10** `{ "index", "count" }`, par `count` décroissant, départagé par le nom croissant (ADR-0124, décision 7). Les deux bornes sont des constantes, sans clé de configuration.
- `indexes` porte **une entrée par index**, avec la **dernière** ligne de `day_indexes` de la fenêtre : `day`, `index`, `documents`, `live_passages`, `dead_passages`, `dead_ratio` (dérivé, jamais stocké), `disk_bytes`, `generation`, `batches`. Les jours absents ne sont **pas comblés** : un trou dit la vérité (ADR-0118, décision 5 ; ADR-0124, décision 8).
- `analytics_dropped` est le compteur de pertes de file **du processus serveur depuis son démarrage**, par processus et jamais persisté (ADR-0118, décision 2 ; ADR-0124, décision 9). `debug_journal_dropped` n'est pas ici : il reste rendu par `GET /v1/debug/queries`.
- Un paramètre **inconnu ou invalide** rend `400 invalid_request` avec un message qui **nomme le paramètre** : `group_by` hors de `minute`, `hour` et `day`, `operation` hors de la liste fermée, date qui n'est pas ISO 8601, `from` postérieur à `to`. Aucune borne de fenêtre ne s'ajoute à la rétention (ADR-0124, décision 2).
- Le filtre `index` porte sur les noms **présents dans `analytics.sqlite`**, sans contrôle contre les index vivants : l'historique d'un index supprimé reste lisible, et un nom sans aucun événement rend `200` avec une réponse vide, membres omis et listes vides, jamais `404`. Un nom invalide au sens des règles de nommage des index rend `400 invalid_request` nommant le paramètre (ADR-0124, décision 2).
- Aucun autre filtre ni aucune répartition n'est offert : ni `status`, ni `client`, ni `mode` (ADR-0124, décision 3).
- La route est déclarée dans la spécification OpenAPI, avec des composants fermés, et son exception nommée est retirée du test des routes dans le même changement (ADR-0111, complément du 2026-10-30 ; ADR-0124, décision 16).

## Limites par défaut (configurables)

| Limite | Défaut |
|---|---|
| `k` | 1000 au plus |
| Longueur de requête | `max_tokens` du modèle ; au-delà, la requête est tronquée et `truncated_query: true` |
| Corps de requête (lot en ligne) | 32 Mio, 10 000 entrées : passages décrits et `doc_id` de `deletions` confondus (ADR-0107) |
| Corps d'un patch de métadonnées | 32 Mio, 10 000 documents (mêmes bornes pour le fichier de `crible patch`) |
| File d'écriture par index | 16 jobs en attente, puis `503 overloaded` avec `Retry-After: 1` (ADR-0103) |
| Recherches concurrentes | 2 × le nombre de cœurs (`[server].max_concurrent_searches`, 0 = 2 × cœurs), puis `503 overloaded` avec `Retry-After: 1` |
| `timeout_ms` | 2 000 par défaut (`[limits].default_timeout_ms`), 30 000 au plus ; `[limits].default_timeout_ms` est lui-même borné de 1 à 30 000, et une valeur hors bornes empêche le serveur de démarrer (ADR-0147) ; à l'échéance, `504 timeout` |
| `wait_timeout_ms` | 60 000 par défaut : attente au plus de `?wait=true`, puis `202` avec le job dans son état courant (ADR-0067) |
| Cache des requêtes | 1 024 entrées par instance, jamais persisté (ADR-0051) |

Toute requête dont le corps dépasse **32 Mio (33 554 432 octets)** est refusée en `payload_too_large` (413) : cette borne est une constante du code, non configurable, et les « 32 Mo » des sections « Lot en ligne » et « Patch de métadonnées » s'entendent en Mio (ADR-0102). Les recherches concurrentes sont limitées par un sémaphore pris **sans attente** : au-delà, la réponse est `503 overloaded` immédiate avec `Retry-After: 1` ; la limite ne s'applique qu'aux routes de recherche, jamais à `/health`, `/ready` ni aux écritures (ADR-0102). À l'échéance de `timeout_ms`, la réponse est `504 timeout` ; le calcul n'est pas annulé et garde son jeton de concurrence jusqu'à sa fin réelle. L'échéance part de la prise du jeton et couvre tout le pipeline — encodage de la requête ou lecture dans le cache, planification, branches, fusion, regroupement et lecture des payloads —, l'authentification et le décodage du corps restant hors du chronomètre (ADR-0109). `timeout_ms` ne s'applique pas aux écritures : `?wait=true` rend `202` à l'échéance de `wait_timeout_ms` (ADR-0067, ADR-0102).
