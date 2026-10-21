# `crible` (Python)

Paquet Python de Crible : `BatchWriter` écrit un lot au format public v1
(`docs/format-lot-v1.md`) en Parquet + `manifest.json`. Le client HTTP arrive en
phase P6.

## Dépendances

`pyarrow`, `numpy`, `blake3` et `pytest` (dev), voir `docs/07`. Le paquet est
géré par `uv` :

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
