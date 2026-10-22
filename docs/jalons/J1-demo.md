# Démo J1 — stockage, lots et recherche exacte

Commandes exactes du jalon J1, à jouer depuis la racine du dépôt. Elles créent
un index sur le jeu synthétique `synth-small`, attachent les trois lots
(création, modification, suppression), affichent l'état, puis cherchent avec un
vecteur fourni et un filtre. Le rejeu part d'un `data/` sans `synth-small` ni
`indexes/synth-small` (tous deux gitignorés) :
`rm -rf data/synth-small data/indexes/synth-small`. Les autres index de
`data/indexes/` ne sont pas touchés.

## Préparation du jeu

`synth-small` est produit par `tools/gen_synth.py` : 2 000 documents et 10 000
passages, 768 dimensions, puis deux lots de mise à jour. `--overwrite` remplace
les seuls fichiers et dossiers que le générateur écrit.

`gen_synth.py` n'écrit pas `expected.jsonl` : ce fichier figure dans la liste
des fichiers racine qu'il gère (`SMALL_FILES`), donc `--overwrite` le supprime,
et seul `tools/reference_topk.py` le recalcule. La préparation enchaîne donc les
deux commandes, dans l'ordre de `.github/workflows/ci.yml`. Sans la seconde, la
comparaison des clés de la section « Recherche avec un vecteur fourni » serait
impossible.

```sh
cd tools
uv sync
uv run python gen_synth.py --out ../data/synth-small --overwrite
uv run python reference_topk.py --dataset ../data/synth-small
cd ..
```

Attendu, dans l'ordre :

```text
../data/synth-small: 1950 documents, 9750 live passages after l1+l2+l3, seed 20261021
../data/synth-small/expected.jsonl: reference over 1950 documents, 9750 live passages
dataset checksum: blake3:a9854f1e3f14b8c6e9afb5bd484a85d0cdc453d6812670d7f092e31760112b1f
wrote ../data/synth-small/expected.jsonl
```

La somme ne dépend ni de la machine, ni de la date, ni de la bibliothèque BLAS :
la référence est calculée exactement (ADR-0045).

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
`epochs/1/` (`records.sqlite`, `segments/` et `manifests/0.json`).

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

Attendu :

```text
index synth-small
  epoch: 1
  generation: 3
  documents: 1950
  passages: 9750 live, 2250 dead
  dead ratio: 0.1875
  segments: 2
    seg-l1: 10000 vectors
    seg-l2: 2000 vectors
  bitmaps: 671737 bytes (live 8208, keywords 88835, booleans 45446, columns 529248)
```

Le lot `l3` ne crée aucun passage : il ne supprime que des documents et ne
réécrit que des métadonnées. `inspect` ouvre le store, donc il exécute la reprise
(suppression des `tmp-*` et des segments orphelins, réécriture d'un manifeste
absent) et prend le verrou d'écriture SQLite : il demande un montage
inscriptible, comme `attach` et `search`.

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
`generation: 3`, `took_ms` (variable : quelques dizaines de millisecondes). Les
clés `doc_id#ord` sont, dans l'ordre, celles de la première ligne de
`data/synth-small/expected.jsonl` :

```text
synth-01163#3, synth-00407#0, synth-01703#2, synth-02087#4, synth-00087#1,
synth-00807#4, synth-01766#2, synth-00134#3, synth-00359#0, synth-00379#4
```

Les scores diffèrent de la référence de moins de 5 × 10⁻⁷, pour un écart maximal
de 4,77 × 10⁻⁷ ici (`synth-00087#1` : `0.6897367239` contre `0.6897372007`), donc
dans la tolérance de `1e-5` de la fiche `RECHERCHER-DENSE-EXACT`. L'écart est
déterministe : la référence est arrondie une seule fois en f32 (ADR-0045), alors
que le moteur somme en f32 dans l'ordre des composantes
(`crates/crible-search/src/dense/exact.rs`).

## Recherche avec un filtre

```sh
./target/release/crible search --data data --index synth-small \
    --vector-file /tmp/crible-j1-query.json \
    --filter '{"categorie": "cat-03"}' --k 5
```

Attendu : 5 résultats, tous d'un document dont le champ `categorie` vaut
`cat-03`, `pool_size: 585` (bien inférieur à 9 750) et des scores cosinus
strictement décroissants :

```text
synth-01603#4 -0.0417295
synth-00739#1 -0.0426792
synth-00675#4 -0.0461998
synth-00322#0 -0.0462879
synth-00003#1 -0.0468779
```

## Erreurs lisibles

Chaque échec écrit `<code>: <message>` sur la sortie d'erreur, avec le nom de
code de `docs/05-api.md` :

```sh
./target/release/crible index create --data data --config data/synth-small/index.json
```

Attendu : sortie non nulle et message suivant.

```text
invalid_request: an index already exists at data/indexes/synth-small: data/indexes/synth-small/CURRENT_EPOCH is present, and creating an index never overwrites one; use `crible attach`, `crible search` or `crible inspect` on it, or create another index under another name
```

Un arbre d'index incomplet — `epochs/` ou un `records.sqlite` à la racine de
l'index, sans `CURRENT_EPOCH` — produit un autre message, qui dit qu'un arbre
partiel est présent, le nomme, et propose de le retirer. Un `--data` jetable,
sous `data/tmp-partial-index` (gitignoré), reproduit cet état sans toucher à
l'index réel :

```sh
rm -rf data/tmp-partial-index
mkdir -p data/tmp-partial-index/indexes/synth-small/epochs
./target/release/crible index create --data data/tmp-partial-index --config data/synth-small/index.json
```

Attendu : sortie non nulle et message suivant.

```text
invalid_request: a partial index tree is present at data/tmp-partial-index/indexes/synth-small: data/tmp-partial-index/indexes/synth-small/epochs exists, but the index has no CURRENT_EPOCH pointer, so no creation can reopen it, and creating an index never overwrites one; remove it to create a new index
```

Nettoyage :

```sh
rm -rf data/tmp-partial-index
```

```sh
./target/release/crible search --data data --index synth-small --vector-file missing.npy
```

Attendu : `invalid_request: cannot read the vector file missing.npy: No such file or directory (os error 2)`.
