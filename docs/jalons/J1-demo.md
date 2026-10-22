# Démo J1 — stockage, lots et recherche exacte

Commandes exactes du jalon J1, à jouer depuis la racine du dépôt. Elles créent
un index sur le jeu synthétique `synth-small`, attachent les trois lots
(création, modification, suppression), affichent l'état, puis cherchent avec un
vecteur fourni et un filtre.

## Préparation du jeu

`synth-small` est produit par `tools/gen_synth.py` : 2 000 documents et 10 000
passages, 768 dimensions, puis deux lots de mise à jour. `--overwrite` remplace
les seuls fichiers et dossiers que le générateur écrit.

```sh
cd tools
uv sync
uv run python gen_synth.py --out ../data/synth-small --overwrite
cd ..
```

La phase P1 n'embarque pas encore d'encodeur. `synth-small` déclare donc un
modèle `external`, dont la configuration est le
`models/<name>/model.json` de `docs/04-donnees-et-ingestion.md`. Les modèles
vivent sur leur propre volume, frère de `data/` (`docs/02-architecture.md`,
`[paths] models = "/models"` de `docs/07-stack-et-deploiement.md`) : la CLI le
désigne par `--models`. Elle est écrite une fois, à la main :

```sh
mkdir -p models/synthetic-external
cat > models/synthetic-external/model.json <<'JSON'
{ "name": "synthetic-external", "kind": "external", "dims": 768, "normalize": true }
JSON
```

## Construction et création de l'index

```sh
cargo build --release
./target/release/crible index create --data data --config data/synth-small/index.json
```

Attendu : `index 'synth-small' created at data/indexes/synth-small (epoch 1, generation 0)`.
Le dossier `data/indexes/synth-small/` contient `index.json`, `CURRENT_EPOCH` et
`epochs/1/` (base SQLite, segments et manifeste de la génération 0).

## Attache des trois lots

```sh
./target/release/crible attach --data data --models models --index synth-small data/synth-small/l1
./target/release/crible attach --data data --models models --index synth-small data/synth-small/l2
./target/release/crible attach --data data --models models --index synth-small data/synth-small/l3
```

Attendu, dans l'ordre :

```text
batch 'l1' attached to index 'synth-small': generation 1 (added 2000, replaced 0, unchanged 0, metadata_updated 0, deleted 0, ignored 0)
batch 'l2' attached to index 'synth-small': generation 2 (added 100, replaced 300, unchanged 0, metadata_updated 0, deleted 0, ignored 0)
batch 'l3' attached to index 'synth-small': generation 3 (added 0, replaced 0, unchanged 0, metadata_updated 50, deleted 150, ignored 0)
```

## État de l'instance

```sh
./target/release/crible inspect --data data
```

Attendu : un bloc `index synth-small` avec `epoch: 1`, `generation: 3`,
`documents: 1950`, `passages: 9750 live, 2250 dead`, `segments: 2` (le lot `l3`
ne crée aucun passage : il ne supprime que des documents et ne réécrit que des
métadonnées), puis la mémoire estimée des bitmaps. `inspect` ouvre le store, donc
il exécute la reprise (suppression des `tmp-*` et des segments orphelins,
réécriture d'un manifeste absent) et prend le verrou d'écriture SQLite : il
demande un montage inscriptible, comme `attach` et `search`.

## Recherche avec un vecteur fourni

La première requête de `queries.jsonl` n'a pas de filtre et demande `k = 10`.
Son vecteur est extrait dans un fichier JSON :

```sh
cd tools
uv run python - <<'PY'
import json
from pathlib import Path

query = json.loads(
    Path("../data/synth-small/queries.jsonl").read_text(encoding="utf-8").splitlines()[0]
)
Path("/tmp/crible-j1-query.json").write_text(
    json.dumps(query["vector"]), encoding="utf-8"
)
PY
cd ..
./target/release/crible search --data data --index synth-small \
    --vector-file /tmp/crible-j1-query.json --k 10
```

Attendu : un objet JSON `results` (10 résultats), `pool_size: 9750`,
`generation: 3`, `took_ms`. Les clés `doc_id#ord` sont celles de la première
ligne de `data/synth-small/expected.jsonl`, aux quasi-égalités de score près
décrites dans la fiche `RECHERCHER-DENSE-EXACT`.

## Recherche avec un filtre

```sh
./target/release/crible search --data data --index synth-small \
    --vector-file /tmp/crible-j1-query.json \
    --filter '{"categorie": "cat-03"}' --k 5
```

Attendu : 5 résultats au plus, tous d'un document dont le champ `categorie`
vaut `cat-03`, un `pool_size` bien inférieur à 9 750, et des scores cosinus
décroissants.

## Erreurs lisibles

Chaque échec écrit `<code>: <message>` sur la sortie d'erreur, avec le nom de
code de `docs/05-api.md` :

```sh
./target/release/crible index create --data data --config data/synth-small/index.json
```

Attendu : sortie non nulle et message
`invalid_request: an index already exists at data/indexes/synth-small: … is present, and creating an index never overwrites one; remove it to create a new index`.

```sh
./target/release/crible search --data data --index synth-small --vector-file missing.npy
```

Attendu : `invalid_request: cannot read the vector file missing.npy: No such file or directory (os error 2)`.
