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

Toute réponse d'erreur du serveur suit ce format et porte `x-request-id` (ADR-0098, ADR-0101). Seule exception : le `503` de `/ready`, dont le corps reste celui d'ADR-0099. `message` est le message de l'erreur ; pour `internal`, il est générique et fixe (« internal error; see the server log for this request_id »), et un message ne contient jamais d'iid (**INV-01**) ni de chemin absolu du serveur. `details` vaut `{}`, sauf une clé documentée ici pour ce code — aujourd'hui `path` pour `invalid_filter` seulement. Une route inconnue et une méthode non permise sur une route connue rendent toutes deux `not_found` (404), avec le message `no route for <METHOD> <path>` ; il n'y a pas de `405`. Un rejet d'extracteur (JSON mal formé, type de contenu absent ou faux, corps illisible) rend `invalid_request` (400) ; un corps au-delà de la limite rend `payload_too_large` (413). Chaque réponse d'erreur est journalisée avec son code, son statut et son `request_id` ; le `message` n'est journalisé que pour `internal` (ADR-0101). Toute réponse `overloaded` porte `Retry-After: 1`, quelle qu'en soit la cause — file d'écriture, limite de recherches concurrentes, arrêt en cours, verrou SQLite, branche lexicale —, par une seule règle posée au rendu de l'erreur ; le `503` de `/ready` n'est pas un `overloaded` et n'en porte pas (ADR-0103).

| Code | HTTP | Cas |
|---|---|---|
| `invalid_request` | 400 | Corps invalide, paramètre hors bornes ; `fusion.alpha` hors de `[0, 1]` ou non fini, à la création de l'index, au `PATCH` de `search.fusion` et dans une requête (ADR-0090) ; destination d'export déjà occupée, c'est-à-dire un dossier `exchange/outbox/<index>-g<G>/` existant (ADR-0061) ; instantané absent, incomplet ou altéré, destination d'instantané déjà occupée, ou `index.json` de la cible divergent à la restauration (ADR-0064, ADR-0065) ; recherche `lexical` ou `hybrid` sur un index dont `lexical.enabled` vaut `false`, y compris quand cet index est une cible d'une recherche multi-index : message « lexical branch is disabled for this index » (ADR-0089) ; `directory` d'un lot hors de l'inbox, lien symbolique ou entrée qui n'est ni un fichier ni un dossier (ADR-0107) ; `doc_id` présent à la fois dans `documents` et dans `deletions` d'un lot en ligne (ADR-0040, ADR-0107) ; corps de lot en ligne sans aucun document ni aucune suppression (ADR-0107) ; corps non vide sur `compact`, `exports` ou `snapshots` (ADR-0107) ; combinaison de `query` et `vector` hors de la règle du mode demandé, ou `query` sans `vector` sur un index à modèle `external` (ADR-0109) ; nom de champ déclaré `returned: false` fourni dans `return.fields` (ADR-0109) |
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
| `overloaded` | 503 | File d'écriture ou limite de recherches concurrentes atteinte, verrou SQLite non obtenu dans le délai de 1 000 ms (ADR-0038), verrou de présence d'un lot en ligne déjà tenu par une autre ingestion du même `batch_id` (ADR-0056), course de génération pendant un export : l'index a commité entre l'instantané et la première lecture, après trois tentatives (ADR-0061) ; index lexical non certifié, en retard ou illisible, ouvert en lecture seule : la branche lexicale est refusée, le mode `dense` reste servi, et la prochaine ouverture en écriture répare (ADR-0087, ADR-0088, ADR-0089) — le message nomme l'état constaté : « behind » (`M > Gt`), « no lexical index is committed in this epoch although passages exist (generation G); open the index for writing to rebuild it » (aucun commit tantivy alors que des passages existent : l'ouverture en écriture reconstruit, elle ne rattrape pas), « ahead » (`Gt > G`), « not certified » (colonne M absente, avec ou sans commit tantivy) ou « unreadable » (lexical illisible, `M > G`) (`Retry-After: 1`, ADR-0103). Une chaîne d'analyse enregistrée différente de celle d'`index.json`, à l'ouverture en lecture seule : « the lexical index was built with another analysis chain; open the index for writing to rebuild it » (ADR-0106). Une époque qui n'a jamais porté de passage (`M = 0`, aucun commit tantivy) n'est pas refusée : `lexical` et `hybrid` rendent une réponse vide (ADR-0089) |
| `timeout` | 504 | Requête trop longue |
| `internal` | 500 | Erreur inattendue (journalisée avec le `request_id`) ; panne disque pendant un instantané ou une restauration (ADR-0065) |

Ces codes servent aussi de catégories d'erreurs dans les analytics.

## Endpoints

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
| `GET /v1/jobs/{job_id}` · `GET /v1/jobs` | write | Suivre un job ; lister les jobs (filtres `index`, `state`) |
| `POST /v1/indexes/{index}/search` | read | Recherche dans un index |
| `POST /v1/search` | read | Recherche multi-index |
| `POST /v1/indexes/{index}/compact` | admin | Lancer une compaction ; sans corps → `202` avec un job `compact` (ADR-0107) |
| `POST /v1/indexes/{index}/exports` | admin | Exporter vers `exchange/outbox` ; sans corps → `202` avec un job `export` (ADR-0107) |
| `POST /v1/indexes/{index}/snapshots` | admin | Instantané vers `exchange/snapshots` ; sans corps → `202` avec un job `snapshot` ; l'écrivain est suspendu pendant la copie, les lectures continuent (ADR-0017, ADR-0065, ADR-0107) |
| `GET /v1/stats` | admin | Analytics agrégées (voir `06-observabilite.md`) |
| `GET /v1/debug/queries` | admin | Journal de debug, s'il est activé |
| `GET /health` · `GET /ready` | — | En vie · prêt (modèles chargés, chaque index a publié son instantané, aucune reprise en cours). `/health` rend `200` dès la liaison au port ; `/ready` rend `503` jusqu'à la publication du dernier instantané, puis `200` ; une instance sans index est prête (ADR-0099) |
| `GET /openapi.json` · `GET /docs` | — | Spécification OpenAPI · Swagger UI |

La spécification est **OpenAPI 3.1** et **générée depuis le code** ; elle est servie par `GET /openapi.json`, et `GET /docs` sert Swagger UI. Aucun fichier de spécification n'est versionné. Le schéma de sécurité `bearerAuth` (HTTP Bearer, clé d'API d'ADR-0100) décrit l'authentification des routes à rôle ; `/health` et `/ready` la neutralisent dans la spécification par `security: [{}]`, une exigence vide que `utoipa` écrit à la place de `security: []`, et restent sans authentification. `/openapi.json` et `/docs` ne figurent pas dans la spécification et restent servis sans clé ; `/docs` répond par une redirection `303` vers `/docs/`. Ce tableau est la **référence** de la liste des routes : un test compare, dans les deux sens, l'ensemble des couples (méthode, chemin sans paramètre de requête) de la spécification à celui du tableau. Sont exceptées par leur nom `/openapi.json` et `GET /docs`, ressources statiques hors spécification, et toute ligne du tableau affectée à une tâche d'une phase ultérieure, tant que cette tâche n'est pas faite : en P6, `GET /v1/stats` (`ROUTE-STATS`) et `GET /v1/debug/queries` (`JOURNAL-DE-DEBUG`), chacun cité dans le test avec la tâche qui le livre. La tâche qui monte la route la déclare dans la spécification et retire son exception du test dans le même changement. Toute route ajoutée au code est ajoutée ici dans le même changement (ADR-0111, complément du 2026-10-30).

Dans un `PATCH /v1/indexes/{index}`, une clé **inconnue** est refusée avec `invalid_request` (400), à tous les niveaux du corps, y compris dans les objets imbriqués (`search`, `lexical`, `ingest`, `compaction`). Le message nomme la clé et son chemin JSON. `immutable_setting` (409) reste réservé aux clés connues mais figées (ADR-0031, INV-16).

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
- `last_batch` est la dernière ligne de `batches`, et vaut `null` quand aucun lot n'a été attaché.

**`PATCH /v1/indexes/{index}`** valide le patch **avant de le mettre en file** : clé inconnue → `400` (ADR-0031), réglage figé → `409 immutable_setting` nommant le champ (INV-16), valeur hors domaine → `400`. Le changement est ensuite exécuté par l'écrivain de l'index, dans l'ordre d'arrivée de sa file, comme un job : `202` avec le job, `?wait=true` honoré. Le `kind` du job vaut `rebuild_lexical` quand le job reconstruit l'index lexical (`lexical.enabled` vaut `true` après le patch et l'une de `lexical.enabled`, `lexical.language`, `lexical.positions` a changé), et `patch_config` sinon. Le job écrit `index.json`, reconstruit ce qui en dérive, puis publie ensemble la nouvelle configuration et le nouvel instantané : une requête voit l'ancienne configuration avec l'ancien instantané, ou la nouvelle avec le nouveau. `search.*` vaut pour les requêtes qui suivent la fin du job ; `ingest.*`, `compaction.*` et `cluster_by` valent pour le prochain job ou la prochaine compaction. Passer `lexical.enabled` à `false` ne reconstruit rien (les recherches `lexical` et `hybrid` sont refusées, ADR-0089) ; le repasser à `true` reconstruit. Le résultat du job vaut `null`.

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
- **Mode** : `upsert` seulement (ADR-0055). `replace_all` est refusé avec `invalid_request`, et le message nomme le mode : la limite de 10 000 entrées le rend inutilisable en ligne. `replace_all` reste le mode d'un lot déposé.
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
| `debug` | bool | false | Ajoute plan, temps et rangs ; journalise si le journal est activé |
| `explain_ids` | liste de `doc_id` ou de clés `doc_id#ord` | [] | Diagnostic de documents précis |
| `timeout_ms` | 1 à 30000 | 2000 | Délai maximal |

**Entrées selon le mode** (ADR-0109). `dense` prend **exactement un** de `query` ou `vector` : les deux ensemble, ou aucun des deux, rendent `invalid_request` (400). `lexical` exige `query` et **refuse** `vector` (400). `hybrid` exige `query` — la branche lexicale note le texte — et accepte un `vector` facultatif, qui sert alors la branche dense **à la place** de l'encodage de `query`. Sur un index à modèle `external`, qui n'encode pas, `dense` exige `vector` et `hybrid` exige `query` **et** `vector` ; un `query` sans `vector` y est refusé en `invalid_request` (400), avec un message qui nomme le modèle. La CLI suit la même règle : `--query` et `--vector-file` ensemble sont acceptés en `hybrid`.

Dans `return.fields`, un nom déclaré `returned: false` dans le schéma est refusé en `invalid_request` (400) : le message nomme le champ, son chemin `$.return.fields[i]` et le fait qu'il est déclaré `returned: false` (ADR-0109). `"all"` n'est pas concerné : il ne rend que les champs `returned`.

Un champ **inconnu** du corps est refusé en `invalid_request` (400) : c'est le cas d'un nom absent du tableau ci-dessus, d'un nom absent de `fusion` ou de `return`, et de tout objet imbriqué, et le message nomme le **chemin JSON** du champ, comme pour un `PATCH` de configuration (ADR-0031, ADR-0109). Dans `filter`, une clause hors de la grammaire reste `invalid_filter` (400), avec `details.path` : son code ne change pas. En P6, `debug` et `explain_ids` ne sont pas encore acceptés : ils sont refusés en `invalid_request`, avec un message qui dit que le champ n'est pas encore pris en charge ; P7 les ouvre (`REPONSE-DEBUG`, `EXPLIQUER-DOCUMENTS`, ADR-0109). Aucun champ n'est ignoré en silence.

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

`took_ms` est la durée totale du traitement, **de la prise du jeton de concurrence à la fin du calcul** : elle couvre l'encodage de la requête ou sa lecture dans le cache, la planification, les deux branches, la fusion, le regroupement et la lecture des payloads (ADR-0109). C'est la même durée que celle de `crible search`, qui mesure depuis l'entrée de sa commande.

Un résultat dont le document a été supprimé après la prise du snapshot garde son texte et ses champs de passage ; ses champs de document sont absents du résultat, sans marqueur et sans erreur (ADR-0043). C'est pourquoi `return.fields: "all"` signifie « tous les champs disponibles pour ce résultat ». Avec `group_by: "document"`, la réponse garde cette forme : une **liste plate** d'entrées de niveau passage, sans objet par document, et `doc_id` suffit à rattacher une entrée à son document (ADR-0075). `k` compte alors des **documents distincts**, et la liste porte au plus `k × passages_per_document` entrées. Les entrées d'un même document sont consécutives ; les documents sont rangés selon leur meilleur passage (score décroissant, puis clé croissante), et les passages d'un document par score décroissant puis clé croissante. Quand les candidats sur-échantillonnés sont épuisés avant `k` documents, la réponse en contient moins, sans erreur. Avec `dedup: "content"`, la fusion s'applique avant le regroupement. Aucun champ nouveau n'est ajouté aux résultats en P4.

Avec `debug: true`, la réponse ajoute :

<!-- openapi: none — le bloc `debug` arrive avec `REPONSE-DEBUG` (P7) ; aucun composant ne le décrit en P6. -->
```json
{ "debug": {
  "plan": { "mode": "hybrid", "dense_strategy": "binary", "rescore_candidates": 5000,
            "candidates_per_branch": 100, "fusion": "rrf", "group_by": null },
  "timings_ms": { "validate": 0.1, "encode": 23.4, "filter": 0.6, "dense": 9.8,
                  "lexical": 6.1, "fusion": 0.1, "group": 0.0, "fetch": 1.2, "total": 41.7 },
  "per_result": [ { "key": "LEGIARTI…#0", "dense_rank": 70, "dense_score": 0.61,
                    "lexical_rank": 2, "lexical_score": 14.2, "fused_rank": 1 } ]
} }
```

`dense_strategy` vaut `"exact"` ou `"binary"` (ADR-0074) : `"exact"` pour un pool de taille inférieure ou égale à `exact_threshold`, pour `exact: true` et pour `search.scan: none` ; `"binary"` sinon. Sous `"binary"`, `rescore_candidates` est le nombre **effectif** de candidats rescorés : il vaut `max(N, candidates_per_branch)`, où `candidates_per_branch` est le nombre de résultats demandés par la branche dense — sur-échantillonné quand la requête regroupe (ADR-0075) —, donc `candidates_per_branch` quand ce nombre dépasse N.

Avec `explain_ids`, chaque document demandé reçoit un diagnostic :

<!-- openapi: none — le bloc `explain` arrive avec `EXPLIQUER-DOCUMENTS` (P7) ; aucun composant ne le décrit en P6. -->
```json
{ "explain": [ {
  "id": "LEGIARTI…", "exists": true, "alive": true,
  "in_pool": false, "excluded_by": ["$.filter.date_debut"],
  "dense":   { "exact_rank": 70, "score": 0.61, "in_binary_candidates": true, "binary_rank": 233 },
  "lexical": { "rank": 2, "score": 14.2, "explanation": { "…": "détail tantivy par terme" } },
  "fused_rank": null, "grouped_under": null
} ] }
```

Le calcul de `exact_rank` est coûteux, car il impose un scan exact du pool. C'est réservé au debug.

Les deux booléens ont une définition unique (ADR-0041) : `exists` vaut « la ligne `documents` est présente » et `alive` vaut « le document a au moins un passage vivant ». Ils restent distincts ; `exists: false` signifie que le `doc_id` n'a jamais existé ou a été supprimé.

## Recherche multi-index

`POST /v1/search`

<!-- openapi: SearchMultiRequest -->
```json
{ "query": "responsabilité du fait des choses", "mode": "hybrid", "k": 10,
  "targets": [ { "index": "articles",  "filter": { "fonds": "LEGI" } },
               { "index": "decisions", "filter": { "partie": { "nin": ["moyens", "faits"] } } } ],
  "group_by": "document" }
```

- **Corps** (ADR-0110). Tous les champs de « Recherche : requête » sont **communs, au premier niveau**, avec la même sémantique et les mêmes bornes : `query`, `vector`, `mode`, `k`, `fusion`, `group_by`, `passages_per_document`, `dedup`, `return`, `exact`, `timeout_ms`. `debug` et `explain_ids` sont refusés en P6, comme sur la route simple (ADR-0109). Seul `filter` est propre à une cible : `targets: [{ "index": "...", "filter": {...} }]`, `index` obligatoire, `filter` facultatif (absent = aucun filtre).
- **Champ inconnu** (ADR-0110). La règle de la route simple s'étend à ce corps, **à tous les niveaux**, objets de `targets[i]` compris : `400 invalid_request`, message nommant le **chemin JSON** du champ (`$.targets[0].filtre`). Dans un filtre, une clause hors grammaire reste `invalid_filter`, avec `details.path` préfixé par `$.targets[i].filter`.
- **`fusion`** (ADR-0110). Absent, chaque index applique sa fusion configurée. Présent, la valeur s'applique à **toutes** les cibles.
- **`mode` par défaut** (ADR-0110). `hybrid` si **toutes** les cibles ont le lexical activé, sinon `dense`. Un `mode` explicite `lexical` ou `hybrid` avec une cible sans lexical reste refusé en entier, avec le message « lexical branch is disabled for this index » et le nom de l'index fautif ; aucune cible n'est servie en dense à sa place (ADR-0089). Le mode `dense` reste disponible sur les mêmes cibles.
- **Modèle commun** (ADR-0110). En `dense` et en `hybrid`, toutes les cibles doivent avoir la même empreinte de modèle (ADR-0028), modèles `external` compris. Un `external` et un `onnx`, ou deux modèles différents, rendent `422 incompatible_model`, et le message nomme **deux index d'empreintes différentes** : la première cible et la première qui diffère. En `lexical`, le modèle n'est pas comparé. Les règles d'entrée d'ADR-0109 s'appliquent au modèle commun : sur des cibles `external`, `dense` exige `vector`, `hybrid` exige `query` **et** `vector`.
- **`targets`** (ADR-0110). Absent ou vide : `400 invalid_request`. Un même index deux fois : `400 invalid_request`, message nommant `$.targets[i].index` ; il compterait deux fois dans le RRF. Un index inconnu : `404 not_found` nommant l'index, **aucun résultat partiel**. Aucune borne propre sur le nombre de cibles : les doublons et les inconnus étant refusés, il est borné par le nombre d'index enregistrés et la taille de corps (ADR-0102).
- **`return.fields`** (ADR-0110). Sur des schémas hétérogènes, une liste de noms est acceptée si chaque nom est déclaré par au moins une cible ; un nom déclaré par aucune cible est refusé en `400 invalid_request`, avec `$.return.fields[i]`. Un nom déclaré `returned: false` dans une cible est refusé en `400`, et le message nomme le champ, `$.return.fields[i]`, l'index et `returned: false` (ADR-0109). Chaque résultat porte ceux des champs demandés que son index déclare et qui sont disponibles pour lui (ADR-0043) ; `"all"` rend tous les champs `returned` de l'index du résultat.
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

`took_ms` couvre la requête entière, **de la prise du jeton à la fin du calcul**, encodage compris (ADR-0109). `truncated_query` a la même définition que sur la route simple et n'a qu'une valeur, puisque l'encodage est unique ; il vaut `false` si rien n'est encodé. En debug, un résultat portera aussi `target_rank`, son rang dans son index, et les `explain_ids` prendront la forme `{ "index": "...", "id": "..." }` (P7, ADR-0110).

## Jobs

<!-- openapi: Job -->
```json
{ "id": "job_…", "kind": "attach_batch", "index": "decisions", "state": "succeeded",
  "created_at": "…", "started_at": "…", "finished_at": "…", "generation": 1288,
  "progress": { "step": "commit", "done": 932, "total": 932 },
  "result": { "report": { "added": 421, "replaced": 428, "unchanged": 1230,
                          "metadata_updated": 137, "deleted": 221, "ignored": 3,
                          "acceptance": { "sample": 200, "min_cosine": 0.99971, "p1": 0.9998, "p50": 0.99995 } } },
  "error": null }
```

- Le rapport donne six comptes **par document et mutuellement exclusifs** : `added`, `replaced`, `unchanged`, `metadata_updated`, `deleted`, `ignored` (ADR-0041, et `docs/04-donnees-et-ingestion.md` pour la définition de chaque clé). Leur somme vaut le nombre de documents traités.
- `progress.done` et `progress.total` comptent les **passages** traités pour `attach_batch` et `online_batch`, comme `counts.passages` du manifeste, et les **documents** pour `patch_documents` ; ils ne se comparent pas aux comptes du rapport.
- `result.report` est le rapport **persisté** de l'attache : il ne porte pas de durées. Les durées par étape sont de la télémétrie d'appel (ADR-0042) : le serveur les **journalise une fois, à la fin du job**, au niveau `info`, avec `job_id`, `kind` et `index`, et **aucune réponse de l'API ne les rend**, `?wait=true` compris ; elles ne sont ni persistées ni portées par le job (ADR-0107). La CLI continue de les afficher à son appelant. Pour `patch_documents`, le rapport `{ updated, unchanged, generation }` n'est **pas** persisté : le job le porte en mémoire, pour sa durée de vie, et un job relu plus tard n'en donne rien (ADR-0060).
- `kind` vaut `attach_batch`, `online_batch`, `patch_documents`, `patch_config`, `compact`, `export`, `snapshot`, `rebuild_lexical` ou `delete_index`. `patch_config` est le patch d'un réglage de l'index qui ne reconstruit pas le lexical, et `rebuild_lexical` celui qui le reconstruit (ADR-0105).
- `state` vaut `queued`, `running`, `succeeded` ou `failed`.
- **Redémarrage** : un job resté `queued` ou `running` quand le serveur s'arrête passe à `failed` au démarrage suivant, avec `finished_at` à l'heure du démarrage, `error.code` à `internal` et le message fixe « the job was interrupted by a server restart ». Aucun job n'est rejoué : le client resoumet, et une attache est idempotente par `batch_id` (ADR-0103).
- **Erreur d'un job** : `{ "code", "message", "details" }`, sans `request_id` — un job n'est pas une requête —, avec les règles du corps d'erreur d'ADR-0101 : message générique et fixe pour `internal`, cause journalisée avec le `job_id` (ADR-0103).
- **Rétention** : les jobs terminés depuis plus de 30 jours sont purgés, de la table et de la mémoire, au démarrage et à la fin de chaque job ; les jobs en cours ne sont jamais purgés. La valeur est fixe, sans clé de configuration. Un job inconnu ou purgé rend `404 not_found` (ADR-0103).
- **Liste** : `GET /v1/jobs` rend tous les jobs retenus, dans un objet à une seule clé, `{ "jobs": [ … ] }` (ADR-0103), filtrables par `index` et par `state`, triés par `created_at` décroissant puis `id`, sans pagination en v1 ; une valeur de `state` hors des quatre états rend `invalid_request` (400). Aucune isolation par clé : toute clé `write` ou `admin` voit les jobs de l'instance (ADR-0103).
- **Arrêt du serveur** : dès le début du drainage, toute nouvelle soumission rend `503 overloaded` avec le message « the server is shutting down » ; les jobs acceptés avant le signal se terminent, et l'attente est bornée par `wait_timeout_ms` comptée depuis le signal, en parallèle du drainage des requêtes. À l'échéance, le processus sort et les jobs non terminés passent à `failed` au démarrage suivant (ADR-0104).

## Limites par défaut (configurables)

| Limite | Défaut |
|---|---|
| `k` | 1000 au plus |
| Longueur de requête | `max_tokens` du modèle ; au-delà, la requête est tronquée et `truncated_query: true` |
| Corps de requête (lot en ligne) | 32 Mio, 10 000 entrées : passages décrits et `doc_id` de `deletions` confondus (ADR-0107) |
| Corps d'un patch de métadonnées | 32 Mio, 10 000 documents (mêmes bornes pour le fichier de `crible patch`) |
| File d'écriture par index | 16 jobs en attente, puis `503 overloaded` avec `Retry-After: 1` (ADR-0103) |
| Recherches concurrentes | 2 × le nombre de cœurs (`[server].max_concurrent_searches`, 0 = 2 × cœurs), puis `503 overloaded` avec `Retry-After: 1` |
| `timeout_ms` | 2 000 par défaut (`[limits].default_timeout_ms`), 30 000 au plus ; à l'échéance, `504 timeout` |
| `wait_timeout_ms` | 60 000 par défaut : attente au plus de `?wait=true`, puis `202` avec le job dans son état courant (ADR-0067) |
| Cache des requêtes | 1 024 entrées par instance, jamais persisté (ADR-0051) |

Toute requête dont le corps dépasse **32 Mio (33 554 432 octets)** est refusée en `payload_too_large` (413) : cette borne est une constante du code, non configurable, et les « 32 Mo » des sections « Lot en ligne » et « Patch de métadonnées » s'entendent en Mio (ADR-0102). Les recherches concurrentes sont limitées par un sémaphore pris **sans attente** : au-delà, la réponse est `503 overloaded` immédiate avec `Retry-After: 1` ; la limite ne s'applique qu'aux routes de recherche, jamais à `/health`, `/ready` ni aux écritures (ADR-0102). À l'échéance de `timeout_ms`, la réponse est `504 timeout` ; le calcul n'est pas annulé et garde son jeton de concurrence jusqu'à sa fin réelle. L'échéance part de la prise du jeton et couvre tout le pipeline — encodage de la requête ou lecture dans le cache, planification, branches, fusion, regroupement et lecture des payloads —, l'authentification et le décodage du corps restant hors du chronomètre (ADR-0109). `timeout_ms` ne s'applique pas aux écritures : `?wait=true` rend `202` à l'échéance de `wait_timeout_ms` (ADR-0067, ADR-0102).
