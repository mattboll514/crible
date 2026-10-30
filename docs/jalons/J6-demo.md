# Démo J6 — serveur, API et charge concurrente

Commandes exactes du jalon J6, à jouer depuis la racine du dépôt. La démo crée
les six index de l'échantillon Legifacile, attache leurs lots, cherche en simple
et en multi-index, patche un document, compacte puis exporte. Elle se joue deux
fois : une fois via Swagger UI, une fois via le client Python. INV-02 sous
charge est prouvé ailleurs, par MES-16.

Références : `docs/phases/P6-serveur-et-api.md:5-9` (jalon J6), ADR-0113
(décisions 3 et 8), ADR-0111 (`/docs`, `/openapi.json`), ADR-0107 (routes de
lots et d'administration), ADR-0112 (client Python), ADR-0032 et ADR-0057
(modèle ONNX réel et ONNX Runtime).

L'état propre ne touche qu'un dossier nommé, gitignoré : `data/j6-demo`. Aucun
`rm -rf data`, et les index existants de `data/indexes/` ne sont pas touchés :
ceux de la démo vivent sous `data/j6-demo/data/indexes/`.

## Prérequis

- `data/legifacile-sample/` (gitignoré) : les six lots réels et `configs/` ;
- `data/models/mdenseon/` : `model.json`, `tokenizer.json`, `model.onnx` et
  `model.int8.onnx` du ré-export d'ADR-0050 ;
- ONNX Runtime, chargé dynamiquement par `crible-encode` : `ORT_DYLIB_PATH`
  nomme la bibliothèque. Le groupe `export` de `tools/` fournit celle de la
  roue Python (1.31.0) ; on affiche son chemin sans le figer dans ce document.

## Préambule — état propre, binaire, environnement

```sh
rm -rf data/j6-demo
mkdir -p data/j6-demo/data data/j6-demo/exchange/inbox data/j6-demo/exchange/outbox
cargo build --release
cd tools && uv sync --inexact && cd ..
export ORT_DYLIB_PATH="$(cd tools && uv run --no-sync python -c 'import onnxruntime, pathlib; print(next((pathlib.Path(onnxruntime.__file__).parent / "capi").glob("libonnxruntime*")))')"
```

Attendu : `cargo build --release` se termine par `Finished` ; la dernière
commande n'affiche rien et `ORT_DYLIB_PATH` nomme `libonnxruntime…` (`.dylib`
sur macOS, `.so` sur Linux).

`uv sync --inexact` est la seule mise à jour de l'environnement de `tools/` : une
synchronisation exacte retirerait le groupe `export` (ADR-0113, décision 7).

## Étape 1 — configuration de l'instance et clés

Trois clés, une par rôle d'ADR-0095. `crible key generate` affiche la clé et son
haché : la clé va dans la variable d'environnement du client, le haché dans
`crible.toml`.

```sh
./target/release/crible key generate
./target/release/crible key generate
./target/release/crible key generate
```

Attendu, trois fois : deux lignes, `key: …` puis `sha256: …` (64 caractères
hexadécimaux minuscules). Noter la première paire comme clé `admin`, la deuxième
comme clé `read`, la troisième comme clé `write`.

Écrire `data/j6-demo/crible.toml`, en remplaçant les trois `sha256` par ceux
affichés. Les chemins sont **relatifs à la racine du dépôt**, celle d'où le
serveur est lancé.

```sh
cat > data/j6-demo/crible.toml <<'TOML'
[server]
bind = "127.0.0.1:8080"
search_threads = 0
max_concurrent_searches = 0

[paths]
data = "data/j6-demo/data"
models = "data/models"
exchange = "data/j6-demo/exchange"

[[auth.keys]]
name = "j6-admin"
role = "admin"
sha256 = "<sha256 de la clé admin>"

[[auth.keys]]
name = "j6-read"
role = "read"
sha256 = "<sha256 de la clé read>"

[[auth.keys]]
name = "j6-write"
role = "write"
sha256 = "<sha256 de la clé write>"

[analytics]
retention_days = 90

[debug_journal]
enabled = false
sample_rate = 0.0
ttl_days = 7

[encoder]
query_precision = "fp32"

[limits]
writer_queue = 16
default_timeout_ms = 2000
wait_timeout_ms = 60000
query_cache_entries = 1024
TOML
```

Attendu : le fichier est créé sans message. Une clé ou un haché faux ne se voit
qu'à la première requête (`401`), un haché non hexadécimal fait refuser le
démarrage.

## Étape 2 — les lots dans l'inbox

Les six lots de l'échantillon sont déposés dans `exchange/inbox/<nom>`. Un lien
symbolique y est refusé (ADR-0107), donc on copie ; `-l` fait des liens durs et
évite d'écrire 514 Mo de plus sur le disque.

```sh
for batch in data/legifacile-sample/batches/*; do
    cp -Rl "$batch" data/j6-demo/exchange/inbox/
done
ls data/j6-demo/exchange/inbox
```

Attendu : les six noms `legifacile-sample-articles-v1`, `-decisions-v1`,
`-dole-v1`, `-jorf-v1`, `-sarde-v1`, `-titres-v1`.

## Étape 3 — démarrer le serveur

Dans un terminal dédié, depuis la racine du dépôt, avec `ORT_DYLIB_PATH` posé
(étape « Préambule ») :

```sh
./target/release/crible serve --config data/j6-demo/crible.toml
```

Attendu : un journal JSON sur la sortie d'erreur, dont une ligne
`"the server is bound"` avec `127.0.0.1:8080`, puis
`"level":"INFO"` pour les six index ouverts ; aucun index n'existe encore, donc
le démarrage ne charge que le modèle.

Dans un second terminal, on vérifie que l'instance est prête :

```sh
curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:8080/ready
```

Attendu : `200`. Tant que les modèles ne sont pas chargés, la même commande rend
`503` (ADR-0099).

## Étape 4 — le scénario via Swagger UI

Ouvrir `http://127.0.0.1:8080/docs` dans un navigateur. Le document est servi par
`/docs`, la spécification par `/openapi.json` (ADR-0111). Cliquer sur
**Authorize**, saisir la clé **admin**, valider : elle couvre les trois rôles, donc
aucune opération des étapes suivantes ne recevra `403`.

Chaque opération ci-dessous se joue avec **Try it out** puis **Execute**. Les
corps sont exactement ceux qui suivent.

1. **Créer les six index** — `POST /v1/indexes`, une fois par fichier de
   `data/legifacile-sample/configs/` (`articles.json`, `decisions.json`,
   `dole.json`, `jorf.json`, `sarde.json`, `titres.json`). Coller le contenu du
   fichier comme corps. Attendu : `201` avec la configuration matérialisée et
   l'état `{ "generation": 0, "documents": 0, "live_passages": 0 }`.
2. **Attacher les six lots** — `POST /v1/indexes/{index}/batches`, corps
   `{ "directory": "legifacile-sample-<index>-v1" }`, par exemple :

   ```json
   { "directory": "legifacile-sample-dole-v1" }
   ```

   Attendu : `202` avec un job `attach_batch`. L'attache dure de 12 s (`titres`)
   à 150 s (`articles`) sur le Mac de la rédaction, parce que la recette
   ré-encode 200 passages en fp32 ; suivre `GET /v1/jobs/{id}` jusqu'à
   `"state": "succeeded"` et `"generation": 1`.
3. **Chercher en simple** — `POST /v1/indexes/dole/search` :

   ```json
   { "query": "Quel dossier législatif correspond à la loi de 2024 pour contrôler l'immigration et améliorer l'intégration ?",
     "mode": "hybrid", "k": 3 }
   ```

   Attendu : `200`, `pool_size: 7628`, `generation: 1`, et
   `JORFDOLE000047079143#0` en tête. `took_ms` inclut l'encodage de la requête.
4. **Chercher en multi-index** — `POST /v1/search`, avec `dole` et `jorf` :

   ```json
   { "targets": [ { "index": "dole" }, { "index": "jorf" } ],
     "query": "Quel dossier législatif correspond à la loi de 2024 pour contrôler l'immigration et améliorer l'intégration ?",
     "mode": "hybrid", "k": 5 }
   ```

   Attendu : `200`, cinq résultats dont `JORFDOLE000047079143` (dole) et
   `JORFARTI000026179331` (jorf), scores `rrf` de l'ordre de 0,016.
5. **Patcher un document** — `PATCH /v1/indexes/dole/documents` :

   ```json
   { "documents": [ { "doc_id": "JORFDOLE000047079143", "fields": { "annee": 2023 } } ] }
   ```

   Attendu : `202` avec un job `patch_documents` ; le job rend
   `{ "updated": 1, "unchanged": 0, "generation": 2 }`. `GET
   /v1/indexes/dole/documents/JORFDOLE000047079143` rend alors
   `"fields": { "annee": 2023, "type": "LOI_PUBLIEE" }`.
6. **Compacter** — `POST /v1/indexes/dole/compact`, **sans corps** (un corps non
   vide rend `400`). Attendu : `202` avec un job `compact` ; il rend
   `{ "from_epoch": 1, "to_epoch": 2, "generation": 3, "live_passages": 7628,
   "removed_passages": 0 }`.
7. **Exporter** — `POST /v1/indexes/dole/exports`, **sans corps**. Attendu :
   `202` avec un job `export` ; il rend
   `{ "directory": "outbox/dole-g3", "generation": 3, "documents": 3593,
   "passages": 7628 }`, et `ls data/j6-demo/exchange/outbox` montre `dole-g3`.

Les générations 2, 3 et 3 suivent l'ordre des étapes : l'attache publie la 1,
le patch la 2, la compaction la 3, et l'export écrit la génération publiée. Un
patch qui ne change rien (`annee` déjà à la valeur demandée) compte `unchanged`
et ne crée pas de génération.

## Étape 5 — le scénario via le client Python

Le même scénario, en un script, depuis un second terminal. Le client est celui
de `clients/python` (ADR-0112), installé dans `tools/.venv` par la dépendance de
chemin ; aucune dépendance nouvelle n'est nécessaire.

```sh
cd tools
export CRIBLE_ADMIN_KEY='<clé admin>'
export CRIBLE_READ_KEY='<clé read>'
export CRIBLE_WRITE_KEY='<clé write>'
uv run --no-sync python - <<'PY'
import asyncio, json
from pathlib import Path

from crible import AsyncClient

URL = "http://127.0.0.1:8080"
SAMPLE = Path("../data/legifacile-sample")
ADMIN, READ, WRITE = (
    __import__("os").environ["CRIBLE_ADMIN_KEY"],
    __import__("os").environ["CRIBLE_READ_KEY"],
    __import__("os").environ["CRIBLE_WRITE_KEY"],
)
QUESTION = (
    "Quel dossier législatif correspond à la loi de 2024 pour contrôler "
    "l'immigration et améliorer l'intégration ?"
)


async def main() -> None:
    async with AsyncClient(URL, ADMIN) as admin, \
               AsyncClient(URL, WRITE) as writer, \
               AsyncClient(URL, READ) as reader:
        for path in sorted((SAMPLE / "configs").glob("*.json")):
            config = json.loads(path.read_text(encoding="utf-8"))
            await admin.create_index(config)
            print(f"index {config['name']} created")
        for config in sorted((SAMPLE / "configs").glob("*.json")):
            index = json.loads(config.read_text(encoding="utf-8"))["name"]
            job = await writer.attach_batch(index, f"legifacile-sample-{index}-v1")
            job = await writer.wait(job, timeout=1800, poll_interval=1.0)
            print(f"attached {index}: generation {job['generation']}")
        answer = await reader.search("dole", query=QUESTION, mode="hybrid", k=3)
        print("dole:", answer["results"][0]["key"], "pool", answer["pool_size"])
        multi = await reader.search_multi(
            [{"index": "dole"}, {"index": "jorf"}], query=QUESTION, mode="hybrid", k=5
        )
        print("multi:", [result["doc_id"] for result in multi["results"]])
        job = await writer.patch_documents(
            "dole", [{"doc_id": "JORFDOLE000047079143", "fields": {"annee": 2023}}]
        )
        job = await writer.wait(job, timeout=600, poll_interval=0.5)
        print("patch:", job["result"]["report"])
        document = await reader.get_document("dole", "JORFDOLE000047079143")
        print("fields:", document["fields"])
        for name, starter in (("compact", admin.compact), ("export", admin.export)):
            job = await starter("dole")
            job = await admin.wait(job, timeout=1800, poll_interval=0.5)
            print(f"{name}:", job["result"]["report"])
        listing = await reader.list_indexes()
        for entry in listing["indexes"]:
            print(entry["name"], entry["generation"], entry["live_passages"])


asyncio.run(main())
PY
cd ..
```

Attendu, dans cet ordre (durées du Mac de la rédaction, sans critère) :

```text
index articles created
…
index titres created
attached articles: generation 1
…
attached titres: generation 1
dole: JORFDOLE000047079143#0 pool 7628
multi: ['JORFDOLE000047079143', 'JORFARTI000026179331', 'JORFDOLE000051903428', 'JORFARTI000037598356', 'JORFDOLE000047079143']
patch: {'generation': 2, 'unchanged': 0, 'updated': 1}
fields: {'annee': 2023, 'type': 'LOI_PUBLIEE'}
compact: {'from_epoch': 1, 'generation': 3, 'live_passages': 7628, 'removed_passages': 0, 'to_epoch': 2}
export: {'directory': 'outbox/dole-g3', 'documents': 3593, 'generation': 3, 'passages': 7628}
articles 1 50000
decisions 1 49998
dole 3 7628
jorf 1 50000
sarde 1 50000
titres 1 50000
```

L'attache des six lots prend environ neuf minutes au total, dont 150 s pour
`articles` ; un `TimeoutError` du script signifie que le serveur ne répond plus,
pas que le lot est refusé (le client ne réessaie jamais, ADR-0112).

## Étape 6 — état de l'instance

```sh
curl -s -H "Authorization: Bearer <clé read>" http://127.0.0.1:8080/v1/indexes
ls data/j6-demo/exchange/outbox
```

Attendu : les six index triés par nom, `dole` à la génération 3 après le patch
et la compaction ; l'export `dole-g3` dans l'outbox. `GET
/v1/indexes/dole` rend la configuration matérialisée et l'état de la génération
publiée (deuxième époque après la compaction).

## Ce que la démo prouve, et ce qu'elle renvoie

La démo montre, par les deux voies, les cinq points du jalon qui la concernent
(`docs/phases/P6-serveur-et-api.md:6`) : création des index de l'échantillon,
attache des lots déposés, recherche simple et multi-index, patch de métadonnées,
compaction, export. Elle ne prouve pas les deux autres points :

- **INV-02 sous charge** : c'est MES-16, `docs/mesures/MES-16-charge-concurrente.md`
  (rapport écrit par `@mesureur`, protocole d'ADR-0113) ; le harnais est
  `tools/bench/mes16/mes16_charge.py` ;
- **INV-16 et la spécification OpenAPI** : ce sont les tests du serveur et de
  `openapi.rs` ; `/openapi.json` est servi par l'instance de la démo, ce qui
  permet de le comparer à `docs/05-api.md`.

La démo ne donne aucune mesure de latence : `took_ms` et les durées citées
servent de repère, pas de chiffre (règle 7 d'`AGENTS.md`).

## Nettoyage

```sh
rm -rf data/j6-demo
```

Seul ce dossier est retiré : il porte la configuration, les index de la démo,
les liens durs de l'inbox et l'export. `data/legifacile-sample/` et
`data/models/` restent en place.
