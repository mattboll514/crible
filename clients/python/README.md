# `crible` (Python)

Paquet Python de Crible : `BatchWriter` écrit un lot au format public v1
(`docs/format-lot-v1.md`) en Parquet + `manifest.json`, et `Client` /
`AsyncClient` sont le client HTTP de l'API REST `/v1` (ADR-0112).

## Dépendances

`pyarrow`, `numpy`, `blake3`, `httpx` et `pytest` (dev), voir `docs/07`. Le
paquet est géré par `uv` :

```sh
uv sync
uv run pytest
```

## Utilisation

```python
from crible import BatchWriter

writer = BatchWriter(
    index_config,      # index.json de l'index cible (mapping)
    out_dir,           # dossier parent ; le lot va dans out_dir/<batch_id>/
    batch_id,          # nom du dossier du lot
    "upsert",          # ou "replace_all"
    model,             # ModelConfig résolu (content de models/<name>/model.json)
    {"name": "decouper_juri", "version": "3", "params": {"max_tokens": 1024}},
    {"name": "juri_titre_corps", "version": "1"},
)
writer.add_document(doc_id, doc_hash, fields, passages)
writer.delete(doc_id)
directory = writer.close()
```

`index_config` est lu pour `name`, `model`, `reference_precision` et `schema` ;
les autres réglages (`lexical`, `search`, `ingest`, `compaction`) sont acceptés
et ignorés.

`model` est la configuration résolue du modèle, **déjà en mémoire** : le writer
ne lit pas `models/<name>/model.json` (il n'est produit qu'en P2). Il en dérive
le membre `model` du manifeste, prend `vector_dtype` dans
`index_config.reference_precision` et laisse `model.runtime` absent. Pour un
modèle `onnx`, `hf_id`, `revision`, `pooling` et `prompts.document` sont requis ;
pour un modèle `external`, seuls `dims` et `normalize` sont lus.

Un **passage** est un mapping `{"text": str, "vector": séquence de nombres}` plus
les champs de passage déclarés. Le writer assigne `doc_id` et `ord` : les fournir
est refusé. Un document arrive avec **tous** ses passages et au moins un.

Le mot-clé facultatif `created_at` fixe l'horodatage du manifeste, une chaîne
RFC 3339 :

```python
writer = BatchWriter(..., created_at="2026-01-01T00:00:00Z")
```

Sans lui, le writer écrit l'heure UTC courante à `close()`. Un producteur qui
doit produire des fichiers identiques au bit près, comme le générateur
`tools/gen_synth.py`, passe une valeur dérivée de ses seuls paramètres.

Le mot-clé facultatif `host` fixe `producer.host`, l'identifiant de la machine
ou du producteur qui a produit le lot (ADR-0036). Il vaut par défaut
`socket.gethostname()` et une chaîne vide est refusée :

```python
writer = BatchWriter(..., host="gen_synth")
```

`host` et `producer.name` sont deux choses différentes : `name` nomme le
programme, `host` dit d'où le lot est sorti.

## Fichiers écrits

```
<out_dir>/<batch_id>/
  manifest.json
  documents.parquet          # toujours, même vide
  passages-0001.parquet…     # ≤ 1 000 000 passages par partie, toujours ≥ 1
  deletions.parquet          # si et seulement si le lot supprime un document
```

`manifest.json` porte les comptes, la liste des fichiers et un checksum
`blake3:<hex>` par fichier, calculé sur les octets écrits. Un lot vide reste
valide : `documents.parquet` et une partie `passages` vide sont écrits.

## Écriture atomique

Le lot n'est jamais écrit sous son nom final. Le writer crée
`<out_dir>/.<batch_id>.tmp-<suffixe>`, y écrit tous les fichiers, synchronise
chaque fichier puis ce dossier, renomme le dossier en `<out_dir>/<batch_id>/`,
puis synchronise `<out_dir>` pour rendre le renommage durable. Un lecteur ne
voit donc jamais un lot partiel : un dossier de lot visible est complet et
valide. `fsync` ne garantit pas forcément l'écriture physique : sur macOS, seul
`F_FULLFSYNC` va jusqu'au support, et le writer utilise `fsync`. La garantie
vaut donc contre un arrêt du processus ou du noyau, pas strictement contre une
coupure de courant.

Une erreur dans `add_document`, `delete` ou `close` supprime ce dossier
temporaire, et lui seul : la cible et les autres entrées d'`out_dir` restent
intactes. Le writer refuse ensuite tout nouvel appel, et le même `batch_id` peut
être réécrit par un nouveau writer. Un dossier cible déjà présent est toujours
refusé à la construction ; s'il apparaît pendant l'écriture, le renommage échoue
avec `BatchError` et rien n'est écrasé.

Synchroniser un dossier n'est pas accepté par toutes les plateformes : ces errno
de non-support (`EINVAL`, `ENOTSUP`, `EACCES`, `EBADF`, `EPERM`, `ENOTDIR`) sont
ignorés et seuls les fichiers sont alors synchronisés. Toute autre erreur de
synchronisation est une panne réelle : `close()` lève `BatchError` et le lot
n'est pas installé.

## Contrôles locaux

Le writer applique avant d'écrire le manifeste les contrôles que le moteur
répète à l'attache (contrôles 5 à 7 de `docs/04`) :

- schéma : champs inconnus, types de valeurs, collision avec une colonne
  réservée (`doc_id`, `doc_hash`, `ord`, `text`, `vector`) ;
- `ord` contigus à partir de 0, assignés par le writer ;
- vecteurs finis, de `dims` dimensions, en fp16 ou fp32 little-endian ;
- norme L2 à `1 ± 1e-2` quand `normalize` est vrai ;
- comptes : au moins un passage par document, au moins un fichier `documents` et
  un fichier `passages`, `deletions.parquet` si et seulement si des suppressions,
  jamais en mode `replace_all`.

Une erreur lève `BatchError` et le manifeste n'est pas écrit.

## Client HTTP

`crible.Client(base_url, api_key, *, timeout=60.0)` et `crible.AsyncClient`
exposent les mêmes méthodes, les secondes déclarées `async`. Les deux sont
utilisables en gestionnaire de contexte, exposent `close()` et partagent un cœur
unique : construction de la requête, encodage des vecteurs, lecture de la
réponse et correspondance des erreurs (ADR-0112).

```python
from crible import Client

with Client("http://127.0.0.1:8080", api_key) as client:
    client.create_index(index_config)                  # 201, config + état
    client.list_indexes()                              # {"indexes": [...]}
    client.get_index("decisions")                      # config + état
    client.search("decisions", vector=query_vector, k=10)
    client.search_multi([{"index": "articles"},
                         {"index": "decisions", "filter": {"juridiction": "ce"}}],
                        query="responsabilité du fait des choses")
    client.get_document("decisions", "LEGIARTI…", text=True)
    job = client.attach_batch("decisions", "legifacile-2026-10-19")
    client.wait(job)                                   # attend la fin du job
    client.upsert_documents("decisions", "manual-1", documents, deletions=["D0"])
    client.patch_documents("decisions", [{"doc_id": "D1", "fields": {"juridiction": "cass"}}])
    client.job(job["id"])
    client.compact("decisions")
    client.export("decisions")
```

- Les méthodes rendent le **JSON de la réponse en `dict`** : `docs/05-api.md` en
  est la référence, et un champ ajouté par le serveur ne casse aucun appelant.
- Les options de `search` et de `search_multi` reprennent les noms de champs de
  `docs/05` (`query`, `vector`, `mode`, `k`, `fusion`, `group_by`,
  `passages_per_document`, `dedup`, `return`, `exact`, `timeout_ms`). Seules les
  options fournies, non `None`, sont envoyées : `k=None` laisse le défaut du
  serveur s'appliquer.
- `vector` accepte un tableau NumPy ou une séquence de flottants ; le client
  l'encode en **base64 de composantes fp32 little-endian**, la forme de l'API.
  Une chaîne est prise pour un vecteur déjà encodé. La même règle s'applique au
  `vector` de chaque passage d'un lot en ligne.
- Les méthodes qui créent un job (`attach_batch`, `upsert_documents`,
  `patch_documents`, `compact`, `export`) le rendent sans attendre. `wait(job)`
  interroge `GET /v1/jobs/{id}` toutes les `poll_interval` secondes (0,5 s par
  défaut) et rend le job terminé ; un job `failed` lève l'exception typée de son
  `error.code`, et l'échéance de `wait` (600 s par défaut) lève l'exception
  standard `TimeoutError`, sans annuler le job.
- Le client **ne réessaie jamais**, même sur `503 overloaded` : l'appelant lit
  `retry_after`.

### Erreurs

Chaque code de `docs/05-api.md` a sa classe dans `crible.errors` :
`InvalidRequestError`, `InvalidFilterError`, `UnauthorizedError`,
`ForbiddenError`, `UnknownIndexError`, `NotFoundError`,
`ImmutableSettingError`, `BatchConflictError`, `PayloadTooLargeError`,
`IncompatibleModelError`, `AcceptanceFailedError`, `OverloadedError`,
`RequestTimeoutError` (le code `timeout`, nommé ainsi pour ne pas masquer
`TimeoutError`) et `InternalError`. Toutes héritent de `CribleError`, qui porte
`code`, `status`, `message`, `details`, `request_id` et `retry_after`.

Un code inconnu du client lève la base `CribleError`, `code` conservé : un
serveur plus récent ne casse pas un client plus ancien. Une réponse d'erreur
sans corps au format d'ADR-0101 lève la base avec `code = None` et le statut, et
une panne de transport lève `CribleTransportError`, l'exception d'origine en
`__cause__`.

```python
from crible import OverloadedError

try:
    client.search("decisions", query="contrat")
except OverloadedError as error:
    if error.retry_after is not None:
        time.sleep(error.retry_after)
```

## Tests contre le binaire

Les tests qui lancent `crible serve` portent le marqueur pytest `server`. Ils
échouent, sans sauter, quand `CRIBLE_BIN` est absent : un test sans son
prérequis est rouge, pas vert (ADR-0112). `addopts` exclut le marqueur par
défaut, donc `uv run pytest` reste vert sans binaire Rust ; le job Rust de la CI
lance `uv run pytest -m server` avec le binaire de `target/debug/crible` :

```sh
cargo build -p crible
CRIBLE_BIN=$PWD/../../target/debug/crible uv run pytest -m server
```

La fixture `tests/conftest.py` lance `crible serve` sur un `crible.toml`
temporaire — dossier de données temporaire, deux modèles `external`, clés de
test des trois rôles, port libre —, attend `/ready` et arrête le serveur par
`SIGTERM` (ADR-0099). Les données sont fabriquées par `BatchWriter` : aucun test
`server` ne dépend de `data/` ni d'ONNX Runtime.
