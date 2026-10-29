# 05 — API REST v1

## Conventions

- **Format** : base `/v1`, JSON en UTF-8, noms en anglais et en `snake_case`.
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

```json
{ "error": { "code": "invalid_filter", "message": "unknown field 'juridicton' (did you mean 'juridiction'?)",
             "details": { "path": "$.filter.juridicton" }, "request_id": "…" } }
```

Toute réponse d'erreur du serveur suit ce format et porte `x-request-id` (ADR-0098, ADR-0101). Seule exception : le `503` de `/ready`, dont le corps reste celui d'ADR-0099. `message` est le message de l'erreur ; pour `internal`, il est générique et fixe (« internal error; see the server log for this request_id »), et un message ne contient jamais d'iid (**INV-01**) ni de chemin absolu du serveur. `details` vaut `{}`, sauf une clé documentée ici pour ce code — aujourd'hui `path` pour `invalid_filter` seulement. Une route inconnue et une méthode non permise sur une route connue rendent toutes deux `not_found` (404), avec le message `no route for <METHOD> <path>` ; il n'y a pas de `405`. Un rejet d'extracteur (JSON mal formé, type de contenu absent ou faux, corps illisible) rend `invalid_request` (400) ; un corps au-delà de la limite rend `payload_too_large` (413). Chaque réponse d'erreur est journalisée avec son code, son statut et son `request_id` ; le `message` n'est journalisé que pour `internal` (ADR-0101). Toute réponse `overloaded` porte `Retry-After: 1`, quelle qu'en soit la cause — file d'écriture, limite de recherches concurrentes, arrêt en cours, verrou SQLite, branche lexicale —, par une seule règle posée au rendu de l'erreur ; le `503` de `/ready` n'est pas un `overloaded` et n'en porte pas (ADR-0103).

| Code | HTTP | Cas |
|---|---|---|
| `invalid_request` | 400 | Corps invalide, paramètre hors bornes ; `fusion.alpha` hors de `[0, 1]` ou non fini, à la création de l'index, au `PATCH` de `search.fusion` et dans une requête (ADR-0090) ; destination d'export déjà occupée, c'est-à-dire un dossier `exchange/outbox/<index>-g<G>/` existant (ADR-0061) ; instantané absent, incomplet ou altéré, destination d'instantané déjà occupée, ou `index.json` de la cible divergent à la restauration (ADR-0064, ADR-0065) ; recherche `lexical` ou `hybrid` sur un index dont `lexical.enabled` vaut `false`, y compris quand cet index est une cible d'une recherche multi-index : message « lexical branch is disabled for this index » (ADR-0089) |
| `invalid_filter` | 400 | Champ inconnu, opérateur interdit pour ce type, valeur mal typée |
| `unauthorized` | 401 | Clé absente, mal formée ou inconnue ; réponse avec `WWW-Authenticate: Bearer` et un corps identique dans les trois cas (ADR-0100) |
| `forbidden` | 403 | Rôle insuffisant ; sans `WWW-Authenticate` (ADR-0100) |
| `unknown_index` | 404 | Index inexistant |
| `not_found` | 404 | Document, job ou lot introuvable ; route inconnue ou méthode non permise sur une route connue (ADR-0101) ; `doc_id` inconnu ou retiré dans un patch de métadonnées (ADR-0060) |
| `immutable_setting` | 409 | Tentative de modifier un réglage figé |
| `batch_conflict` | 409 | Même `batch_id`, manifeste différent |
| `payload_too_large` | 413 | Corps au-delà des limites |
| `incompatible_model` | 422 | Modèle du lot ou des index d'une recherche incompatible |
| `acceptance_failed` | 422 | Recette refusée |
| `overloaded` | 503 | File d'écriture ou limite de recherches concurrentes atteinte, verrou SQLite non obtenu dans le délai de 1 000 ms (ADR-0038), verrou de présence d'un lot en ligne déjà tenu par une autre ingestion du même `batch_id` (ADR-0056), course de génération pendant un export : l'index a commité entre l'instantané et la première lecture, après trois tentatives (ADR-0061) ; index lexical non certifié, en retard ou illisible, ouvert en lecture seule : la branche lexicale est refusée, le mode `dense` reste servi, et la prochaine ouverture en écriture répare (ADR-0087, ADR-0088, ADR-0089) — le message nomme l'état constaté : « behind » (`M > Gt`), « no lexical index is committed in this epoch although passages exist (generation G); open the index for writing to rebuild it » (aucun commit tantivy alors que des passages existent : l'ouverture en écriture reconstruit, elle ne rattrape pas), « ahead » (`Gt > G`), « not certified » (colonne M absente, avec ou sans commit tantivy) ou « unreadable » (lexical illisible, `M > G`) (`Retry-After: 1`, ADR-0103). Une époque qui n'a jamais porté de passage (`M = 0`, aucun commit tantivy) n'est pas refusée : `lexical` et `hybrid` rendent une réponse vide (ADR-0089) |
| `timeout` | 504 | Requête trop longue |
| `internal` | 500 | Erreur inattendue (journalisée avec le `request_id`) ; panne disque pendant un instantané ou une restauration (ADR-0065) |

Ces codes servent aussi de catégories d'erreurs dans les analytics.

## Endpoints

| Méthode et chemin | Rôle | Description |
|---|---|---|
| `POST /v1/indexes` | admin | Créer un index (config complète) → `201` |
| `GET /v1/indexes` | read | Lister les index : nom, modèle, comptes, génération |
| `GET /v1/indexes/{index}` | read | Config + état : époque, génération, documents, passages vivants et morts, ratio de morts, segments, tailles, dernier lot |
| `PATCH /v1/indexes/{index}` | admin | Modifier les réglages modifiables ; un réglage figé → `409` ; une clé inconnue → `400` |
| `DELETE /v1/indexes/{index}?confirm={index}` | admin | Supprimer l'index (job) |
| `POST /v1/indexes/{index}/documents` | write | Lot en ligne : documents et passages (texte, ou vecteurs fournis), suppressions → job |
| `PATCH /v1/indexes/{index}/documents` | write | Mettre à jour des métadonnées de documents → job |
| `GET /v1/indexes/{index}/documents/{doc_id}` | read | Document, ses passages et son état (`?text=true` pour inclure le texte) |
| `POST /v1/indexes/{index}/batches` | write | Attacher un lot déposé dans `exchange/inbox/<dossier>` → job |
| `GET /v1/jobs/{job_id}` · `GET /v1/jobs` | write | Suivre un job ; lister les jobs (filtres `index`, `state`) |
| `POST /v1/indexes/{index}/search` | read | Recherche dans un index |
| `POST /v1/search` | read | Recherche multi-index |
| `POST /v1/indexes/{index}/compact` | admin | Lancer une compaction → job |
| `POST /v1/indexes/{index}/exports` | admin | Exporter vers `exchange/outbox` → job |
| `POST /v1/indexes/{index}/snapshots` | admin | Instantané vers `exchange/snapshots` → job ; l'écrivain est suspendu pendant la copie, les lectures continuent (ADR-0017, ADR-0065) |
| `GET /v1/stats` | admin | Analytics agrégées (voir `06-observabilite.md`) |
| `GET /v1/debug/queries` | admin | Journal de debug, s'il est activé |
| `GET /health` · `GET /ready` | — | En vie · prêt (modèles chargés, chaque index a publié son instantané, aucune reprise en cours). `/health` rend `200` dès la liaison au port ; `/ready` rend `503` jusqu'à la publication du dernier instantané, puis `200` ; une instance sans index est prête (ADR-0099) |
| `GET /openapi.json` · `GET /docs` | — | Spécification OpenAPI · Swagger UI |

Dans un `PATCH /v1/indexes/{index}`, une clé **inconnue** est refusée avec `invalid_request` (400), à tous les niveaux du corps, y compris dans les objets imbriqués (`search`, `lexical`, `ingest`, `compaction`). Le message nomme la clé et son chemin JSON. `immutable_setting` (409) reste réservé aux clés connues mais figées (ADR-0031, INV-16).

### Lot en ligne

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
- Limite : 10 000 passages par requête.
- **Mode** : `upsert` seulement (ADR-0055). `replace_all` est refusé avec `invalid_request`, et le message nomme le mode : la limite de 10 000 passages le rend inutilisable en ligne. `replace_all` reste le mode d'un lot déposé.
- **Manifeste du lot interne** : déterministe, `producer` vaut `crible-online` ; ses valeurs sont fixées par ADR-0055 et décrites dans `04-donnees-et-ingestion.md` (mode en ligne) et `format-lot-v1.md` (lot interne du mode en ligne).

### Patch de métadonnées

`PATCH /v1/indexes/{index}/documents` met à jour les **champs d'un document** sans ré-encodage (ADR-0060).

```json
{ "documents": [ { "doc_id": "D1", "fields": { "juridiction": "cass" } } ] }
```

- **Fusion** : chaque clé de `fields` remplace la valeur du champ de même nom, les clés absentes gardent leur valeur. Un `fields: {}` est un no-op, compté `unchanged`.
- **`null`** est une valeur posée ; il ne retire jamais la clé.
- **Champs** : seuls les champs de document du schéma sont acceptés. Une clé inconnue, un champ de passage, `doc_id` ou `doc_hash` donnent `invalid_request`, avec un message qui nomme le champ, le niveau attendu (`document_fields`) et le chemin JSON — jamais la valeur. Un champ `filterable: false` ou `returned: false` accepte la valeur, qui reste invisible aux filtres ou au payload du document.
- **Valeur mal typée** : `invalid_request`, avec le type attendu ; jamais `invalid_filter`, qui appartient à la grammaire des filtres.
- **Document inconnu ou retiré** : `not_found`, et aucun document du corps n'est appliqué. Un patch ne crée jamais de document.
- **Limites** : 32 Mo et 10 000 documents (voir « Limites par défaut »), sinon `payload_too_large`.
- **Rapport** : `{ updated, unchanged, generation }`, par document. Il n'est pas persisté (voir « Jobs »).
- **CLI** : `crible patch --data <dir> --index <nom> <fichier>.jsonl` lit le même objet, un par ligne, le fichier étant passé en argument positionnel, sans `--models` : un patch n'encode rien.
- **Effet** : `doc_hash` inchangé, `updated_generation = G+1` sur les lignes patchées, aucun segment, aucune ligne `batches` ; aucune génération n'est créée si aucun document ne change (`docs/02`, ADR-0060).
- **Non-durabilité** : une attache ultérieure du même document avec un `doc_hash` identique réécrit les champs de document et écrase la valeur patchée (`docs/04`).
- Le patch passe la même file d'écriture que les autres écritures de l'index. Lancer `crible patch` sur un index servi par un serveur reste une règle d'exploitation : `busy_timeout` n'assure que la vivacité (ADR-0038), pas l'exclusion entre processus.

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

```json
{ "fonds": "LEGI", "date_debut": { "lte": "2020-01-01" },
  "$or": [ { "date_fin": { "gt": "2020-01-01" } }, { "date_fin": { "exists": false } } ] }
```
Ce premier filtre sélectionne les articles du fonds LEGI en vigueur au 1er janvier 2020.

```json
{ "juridiction": { "in": ["cass", "ce"] }, "partie": { "nin": ["moyens", "faits"] } }
```
Ce second filtre garde les décisions de la Cour de cassation et du Conseil d'État, sans les passages « moyens » ni « faits ».

## Recherche : requête

`POST /v1/indexes/{index}/search`

| Champ | Type | Défaut | Rôle |
|---|---|---|---|
| `query` | string | — | Texte de la requête, encodé par le serveur. Il faut `query` ou `vector` |
| `vector` | base64 fp32 LE | — | Vecteur fourni (obligatoire pour un modèle `external`) |
| `mode` | `dense` · `lexical` · `hybrid` | `hybrid` si le lexical est activé, sinon `dense` | Branches à exécuter |
| `filter` | objet | aucun | Voir la grammaire ci-dessus |
| `k` | 1 à 1000 | 10 | Nombre de résultats ; sous `group_by: "document"`, nombre de documents distincts (ADR-0075) |
| `fusion` | `{method: "rrf"}` ou `{method: "relative_score", alpha}` | config de l'index | Fusion hybride ; `alpha ∈ [0, 1]`, bornes comprises et valeur finie, refusé en `invalid_request` hors de ce domaine (ADR-0090) |
| `group_by` | `"document"` ou null | null | Regroupement par document |
| `passages_per_document` | 1 à 5 | 1 | Passages renvoyés par document regroupé ; il est validé même sans `group_by`, où il est sans effet (ADR-0075) |
| `dedup` | `"content"` ou null | null | Fusion des passages identiques |
| `return` | `{fields: [...] ou "all", text: bool}` | `{fields: "all", text: true}` | Contenu renvoyé ; `"all"` vaut tous les champs **disponibles pour ce résultat** (ADR-0043) |
| `exact` | bool | false | Force le calcul exact (éval, debug) |
| `debug` | bool | false | Ajoute plan, temps et rangs ; journalise si le journal est activé |
| `explain_ids` | liste de `doc_id` ou de clés `doc_id#ord` | [] | Diagnostic de documents précis |
| `timeout_ms` | 1 à 30000 | 2000 | Délai maximal |

## Recherche : réponse

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

Un résultat dont le document a été supprimé après la prise du snapshot garde son texte et ses champs de passage ; ses champs de document sont absents du résultat, sans marqueur et sans erreur (ADR-0043). C'est pourquoi `return.fields: "all"` signifie « tous les champs disponibles pour ce résultat ». Avec `group_by: "document"`, la réponse garde cette forme : une **liste plate** d'entrées de niveau passage, sans objet par document, et `doc_id` suffit à rattacher une entrée à son document (ADR-0075). `k` compte alors des **documents distincts**, et la liste porte au plus `k × passages_per_document` entrées. Les entrées d'un même document sont consécutives ; les documents sont rangés selon leur meilleur passage (score décroissant, puis clé croissante), et les passages d'un document par score décroissant puis clé croissante. Quand les candidats sur-échantillonnés sont épuisés avant `k` documents, la réponse en contient moins, sans erreur. Avec `dedup: "content"`, la fusion s'applique avant le regroupement. Aucun champ nouveau n'est ajouté aux résultats en P4.

Avec `debug: true`, la réponse ajoute :

```json
"debug": {
  "plan": { "mode": "hybrid", "dense_strategy": "binary", "rescore_candidates": 5000,
            "candidates_per_branch": 100, "fusion": "rrf", "group_by": null },
  "timings_ms": { "validate": 0.1, "encode": 23.4, "filter": 0.6, "dense": 9.8,
                  "lexical": 6.1, "fusion": 0.1, "group": 0.0, "fetch": 1.2, "total": 41.7 },
  "per_result": [ { "key": "LEGIARTI…#0", "dense_rank": 70, "dense_score": 0.61,
                    "lexical_rank": 2, "lexical_score": 14.2, "fused_rank": 1 } ]
}
```

`dense_strategy` vaut `"exact"` ou `"binary"` (ADR-0074) : `"exact"` pour un pool de taille inférieure ou égale à `exact_threshold`, pour `exact: true` et pour `search.scan: none` ; `"binary"` sinon. Sous `"binary"`, `rescore_candidates` est le nombre **effectif** de candidats rescorés : il vaut `max(N, candidates_per_branch)`, où `candidates_per_branch` est le nombre de résultats demandés par la branche dense — sur-échantillonné quand la requête regroupe (ADR-0075) —, donc `candidates_per_branch` quand ce nombre dépasse N.

Avec `explain_ids`, chaque document demandé reçoit un diagnostic :

```json
"explain": [ {
  "id": "LEGIARTI…", "exists": true, "alive": true,
  "in_pool": false, "excluded_by": ["$.filter.date_debut"],
  "dense":   { "exact_rank": 70, "score": 0.61, "in_binary_candidates": true, "binary_rank": 233 },
  "lexical": { "rank": 2, "score": 14.2, "explanation": { "…": "détail tantivy par terme" } },
  "fused_rank": null, "grouped_under": null
} ]
```

Le calcul de `exact_rank` est coûteux, car il impose un scan exact du pool. C'est réservé au debug.

Les deux booléens ont une définition unique (ADR-0041) : `exists` vaut « la ligne `documents` est présente » et `alive` vaut « le document a au moins un passage vivant ». Ils restent distincts ; `exists: false` signifie que le `doc_id` n'a jamais existé ou a été supprimé.

## Recherche multi-index

`POST /v1/search`

```json
{ "query": "responsabilité du fait des choses", "mode": "hybrid", "k": 10,
  "targets": [ { "index": "articles",  "filter": { "fonds": "LEGI" } },
               { "index": "decisions", "filter": { "partie": { "nin": ["moyens", "faits"] } } } ],
  "group_by": "document" }
```

- Chaque résultat porte `index`. En debug, il porte aussi `target_rank`, son rang dans son index.
- Les `explain_ids` prennent la forme `{ "index": "...", "id": "..." }`.
- Si une cible d'une recherche `lexical` ou `hybrid` a `lexical.enabled` à `false`, la requête entière est refusée en `invalid_request` (400), avec le message « lexical branch is disabled for this index » et le nom de l'index fautif ; aucune cible n'est servie en dense à sa place (ADR-0089). Le mode `dense` reste disponible sur les mêmes cibles.

## Jobs

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
- `result.report` est le rapport **persisté** de l'attache : il ne porte pas de durées. Les durées par étape sont de la télémétrie d'appel, rendues au seul appelant d'origine de l'attache (ADR-0042) ; un job relu plus tard n'en donne aucune. Pour `patch_documents`, le rapport `{ updated, unchanged, generation }` n'est **pas** persisté : le job le porte en mémoire, pour sa durée de vie, et un job relu plus tard n'en donne rien (ADR-0060).
- `kind` vaut `attach_batch`, `online_batch`, `patch_documents`, `compact`, `export`, `snapshot`, `rebuild_lexical` ou `delete_index`.
- `state` vaut `queued`, `running`, `succeeded` ou `failed`.
- **Redémarrage** : un job resté `queued` ou `running` quand le serveur s'arrête passe à `failed` au démarrage suivant, avec `finished_at` à l'heure du démarrage, `error.code` à `internal` et le message fixe « the job was interrupted by a server restart ». Aucun job n'est rejoué : le client resoumet, et une attache est idempotente par `batch_id` (ADR-0103).
- **Erreur d'un job** : `{ "code", "message", "details" }`, sans `request_id` — un job n'est pas une requête —, avec les règles du corps d'erreur d'ADR-0101 : message générique et fixe pour `internal`, cause journalisée avec le `job_id` (ADR-0103).
- **Rétention** : les jobs terminés depuis plus de 30 jours sont purgés, de la table et de la mémoire, au démarrage et à la fin de chaque job ; les jobs en cours ne sont jamais purgés. La valeur est fixe, sans clé de configuration. Un job inconnu ou purgé rend `404 not_found` (ADR-0103).
- **Liste** : `GET /v1/jobs` rend tous les jobs retenus, filtrables par `index` et par `state`, triés par `created_at` décroissant puis `id`, sans pagination en v1 ; une valeur de `state` hors des quatre états rend `invalid_request` (400). Aucune isolation par clé : toute clé `write` ou `admin` voit les jobs de l'instance (ADR-0103).
- **Arrêt du serveur** : dès le début du drainage, toute nouvelle soumission rend `503 overloaded` avec le message « the server is shutting down » ; les jobs acceptés avant le signal se terminent, et l'attente est bornée par `wait_timeout_ms` comptée depuis le signal, en parallèle du drainage des requêtes. À l'échéance, le processus sort et les jobs non terminés passent à `failed` au démarrage suivant (ADR-0104).

## Limites par défaut (configurables)

| Limite | Défaut |
|---|---|
| `k` | 1000 au plus |
| Longueur de requête | `max_tokens` du modèle ; au-delà, la requête est tronquée et `truncated_query: true` |
| Corps de requête (lot en ligne) | 32 Mio, 10 000 passages |
| Corps d'un patch de métadonnées | 32 Mio, 10 000 documents (mêmes bornes pour le fichier de `crible patch`) |
| File d'écriture par index | 16 jobs en attente, puis `503 overloaded` avec `Retry-After: 1` (ADR-0103) |
| Recherches concurrentes | 2 × le nombre de cœurs (`[server].max_concurrent_searches`, 0 = 2 × cœurs), puis `503 overloaded` avec `Retry-After: 1` |
| `timeout_ms` | 2 000 par défaut (`[limits].default_timeout_ms`), 30 000 au plus ; à l'échéance, `504 timeout` |
| `wait_timeout_ms` | 60 000 par défaut : attente au plus de `?wait=true`, puis `202` avec le job dans son état courant (ADR-0067) |
| Cache des requêtes | 1 024 entrées par instance, jamais persisté (ADR-0051) |

Toute requête dont le corps dépasse **32 Mio (33 554 432 octets)** est refusée en `payload_too_large` (413) : cette borne est une constante du code, non configurable, et les « 32 Mo » des sections « Lot en ligne » et « Patch de métadonnées » s'entendent en Mio (ADR-0102). Les recherches concurrentes sont limitées par un sémaphore pris **sans attente** : au-delà, la réponse est `503 overloaded` immédiate avec `Retry-After: 1` ; la limite ne s'applique qu'aux routes de recherche, jamais à `/health`, `/ready` ni aux écritures (ADR-0102). À l'échéance de `timeout_ms`, la réponse est `504 timeout` ; le calcul n'est pas annulé et garde son jeton de concurrence jusqu'à sa fin réelle. `timeout_ms` ne s'applique pas aux écritures : `?wait=true` rend `202` à l'échéance de `wait_timeout_ms` (ADR-0067, ADR-0102).
