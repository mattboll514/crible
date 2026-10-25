# Format de lot v1

Spécification de référence du format d'échange public de Crible. Elle dérive de
`04-donnees-et-ingestion.md`, sections « Format de lot v1 », « Sémantique
d'attache » et « Contrôles et recette », et de l'ADR-0012. Le code Python du
producteur (`BatchWriter`) et le lecteur Rust de la phase P1 suivent ce
document.

Le format est **versionné**. La version 1 est la seule reconnue par le build
actuel. Toute évolution incompatible incrémente `format_version`.

## Dossier d'un lot

Un lot est un dossier `<batch_id>/` qui contient :

```
manifest.json
documents.parquet      (ou documents-0001.parquet, …)
passages.parquet       (ou passages-0001.parquet, …)
deletions.parquet      (facultatif)
```

Les noms de fichiers sont exacts :

- `documents.parquet`, ou `documents-NNNN.parquet` avec au moins quatre
  chiffres ;
- `passages.parquet`, ou `passages-NNNN.parquet` avec au moins quatre
  chiffres ;
- `deletions.parquet`, sans variante numérotée.

Un même rôle peut donc être réparti sur plusieurs fichiers pour les gros lots.
Les parties d'un rôle se parcourent dans l'ordre croissant de la **valeur
numérique** de leur suffixe (`passages-0001.parquet`, puis
`passages-0002.parquet`, …), quel que soit le nombre de chiffres écrits. Deux
parties d'un même rôle qui portent la même valeur numérique, par exemple
`passages-0001.parquet` et `passages-00001.parquet`, font rejeter le lot avec
`400 invalid_request` (ADR-0040). Les lignes des passages d'un même document
arrivent dans l'ordre croissant des `ord` (ADR-0039). Tout autre nom de fichier
dans `manifest.json` est refusé. P0 ne refuse pas le mélange des deux formes
pour un même rôle (`documents.parquet` et `documents-0001.parquet` dans le même
manifeste) : le producteur choisit une forme par rôle, mais cette règle n'est
pas encore contrôlée (ADR-0040).

Le `batch_id` est aussi le nom du dossier : c'est un segment de chemin unique.
Il ne doit être ni vide, ni `.`, ni `..`, et ne doit contenir ni `/`, ni `\`, ni
caractère de contrôle.

## `manifest.json`

Exemple, repris de `04-donnees-et-ingestion.md` :

```json
{
  "format": "crible-batch",
  "format_version": 1,
  "batch_id": "legifacile-decisions-2026-10-19",
  "index": "decisions",
  "mode": "upsert",
  "created_at": "2026-10-19T05:42:10Z",
  "producer": { "name": "legifacile-sync", "version": "…", "host": "macmini" },
  "model": {
    "hf_id": "lightonai/mDenseOn", "revision": "…", "dims": 768,
    "pooling": "cls", "normalize": true, "prompt_document": "document: ",
    "vector_dtype": "fp16",
    "runtime": { "torch": "…", "transformers": "…", "device": "mps" }
  },
  "chunker":  { "name": "decouper_juri", "version": "3", "params": { "max_tokens": 1024, "overlap": 0.15 } },
  "template": { "name": "juri_titre_corps", "version": "1" },
  "counts":   { "documents": 428, "passages": 932, "deletions": 221 },
  "files":    { "documents.parquet": "blake3:…", "passages.parquet": "blake3:…", "deletions.parquet": "blake3:…" }
}
```

### Champs

| Champ | Type | Obligatoire | Règle |
|---|---|---|---|
| `format` | chaîne | oui | Vaut exactement `crible-batch`. |
| `format_version` | entier | oui | Vaut `1`. |
| `batch_id` | chaîne | oui | Segment de chemin unique, non vide. |
| `index` | chaîne | oui | Nom de l'index cible ; doit correspondre à l'index d'attache. |
| `mode` | chaîne | non | `upsert` (défaut) ou `replace_all`. |
| `created_at` | chaîne | oui | Date RFC 3339 en UTC. P0 ne vérifie que sa non-vacuité, pas la syntaxe. |
| `producer.name` | chaîne | oui | Nom du programme producteur, non vide. |
| `producer.version` | chaîne | oui | Version du programme. |
| `producer.host` | chaîne | oui | Identifiant de la machine, ou du producteur, qui a produit le lot (ADR-0036). |
| `model` | objet | oui | Modèle d'encodage, voir ci-dessous. |
| `chunker` | objet | oui | Découpeur : `name`, `version`, `params` (objet JSON libre). |
| `template` | objet | oui | Gabarit : `name`, `version`. |
| `counts` | objet | oui | `documents`, `passages`, `deletions` (entiers positifs ou nuls). |
| `files` | objet | oui | Nom de fichier Parquet → checksum. |

Les clés inconnues des objets du manifeste sont refusées. Les objets
`model.runtime` et `chunker.params` sont des dictionnaires libres, dont les
clés appartiennent au producteur. Les compteurs sont des entiers non signés
sur 64 bits.

### Modèle

| Champ | Type | Obligatoire | Règle |
|---|---|---|---|
| `hf_id` | chaîne | pour un index ONNX | Dépôt Hugging Face de l'encodeur. |
| `revision` | chaîne | pour un index ONNX | Commit du dépôt. |
| `dims` | entier | oui | Nombre de dimensions, strictement positif. |
| `pooling` | chaîne | pour un index ONNX | Pooling de l'export (`cls`, `mean`, …). |
| `normalize` | booléen | oui | Vecteurs normalés L2 par le producteur. |
| `prompt_document` | chaîne | pour un index ONNX | Prompt préfixé avant l'encodage des passages. |
| `vector_dtype` | chaîne | oui | `fp16` ou `fp32`. |
| `runtime` | objet | non | Versions libres du runtime, valeurs chaînes. |

Pour un index `onnx`, les champs `hf_id`, `revision`, `dims`, `pooling`,
`normalize`, `prompt_document` et `vector_dtype` doivent correspondre à la
configuration du modèle de l'index. Un écart est refusé avec
`422 incompatible_model`, et le message nomme le champ.

Pour un index `external`, seuls `dims`, `normalize` et `vector_dtype` sont
comparés : les champs d'encodeur sont ignorés.

`vector_dtype` doit correspondre à `reference_precision` de l'index : `fp16`
avec `fp16`, `fp32` avec `fp32`. La précision de référence est figée à la
création de l'index (ADR-0005).

### Modes

- `upsert` (défaut) : les documents listés sont créés, remplacés ou mis à jour ;
  les autres documents de l'index ne changent pas. Un fichier `deletions` est
  autorisé.
- `replace_all` : le lot est le contenu complet de l'index. Tout document
  existant absent du lot est supprimé. Le fichier `deletions` est **interdit**
  dans ce mode, et `counts.deletions` vaut zéro.

### Fichiers et checksums

`files` associe chaque fichier Parquet à un checksum de la forme
`blake3:<hex>`, où `<hex>` est le digest blake3 des octets du fichier en
hexadécimal minuscule. En P0, la validation vérifie le préfixe `blake3:` et un
digest non vide. L'hexadécimal strict, la longueur et la valeur du digest
contre les octets du fichier sont vérifiés au contrôle 4, à l'attache (P1).

La liste doit contenir :

- au moins un fichier `documents…` ;
- au moins un fichier `passages…` ;
- `deletions.parquet` si et seulement si `counts.deletions > 0`, et jamais en
  mode `replace_all`.

Le contrôle de la valeur des checksums contre les fichiers eux-mêmes est le
contrôle 4 de `04`, exécuté à l'attache (phase P1). La validation du manifeste
vérifie seulement le préfixe, la présence d'un digest et la cohérence de la
liste.

### Comptes

`counts` déclare le nombre de lignes de chaque rôle :

- `counts.documents` est le nombre de lignes des fichiers `documents…` ;
- `counts.passages` est le nombre de lignes des fichiers `passages…` ;
- `counts.deletions` est le nombre de lignes de `deletions.parquet`.

Règles vérifiables sans lire le Parquet :

- un lot sans document n'a aucun passage (`documents == 0` implique
  `passages == 0`) ;
- chaque document a au moins un passage, donc `passages >= documents` ;
- `deletions == 0` interdit le fichier `deletions.parquet` ;
- `deletions > 0` exige le fichier `deletions.parquet`.

La correspondance exacte entre `counts` et le nombre de lignes réellement
lues est le contrôle 6 de `04`, exécuté à l'attache (phase P1).

### Lot interne du mode en ligne

Le mode en ligne (ADR-0055) construit un **lot interne** qui suit ce format et
le même chemin d'attache. Deux règles lui sont propres, pour qu'un rejeu du même
contenu ne produise pas de `409 batch_conflict` :

- le mode est toujours `upsert` ; `replace_all` est refusé en ligne avec
  `invalid_request` ;
- le manifeste est **déterministe** :

| Champ | Valeur |
|---|---|
| `producer.name` | `crible-online` |
| `producer.version` | `1` — version du format du lot en ligne, pas du binaire |
| `producer.host` | `crible-online` |
| `chunker` | `{ "name": "none", "version": "1" }` |
| `template` | `{ "name": "none", "version": "1" }` |
| `created_at` | `1970-01-01T00:00:00Z`, sentinelle documentée |

`producer.version` est incrémenté seulement avec un changement de format du lot
en ligne : la version du binaire, elle, change à chaque mise à jour de Crible et
casserait l'idempotence (INV-10). `created_at` est une sentinelle parce
qu'aucun contrôle ne lit sa valeur et que le mtime n'est pas stable d'une copie
à l'autre.

Le dossier du lot interne est écrit dans `<data>/tmp/online-<batch_id>/`, hors
du dossier de l'index, et supprimé en succès comme en échec. Pour la CLI,
`batch_id` vaut `online-<blake3 des octets du fichier>` ; la route HTTP le prend
dans le corps de la requête. La colonne `doc_hash` est toujours remplie : elle
vaut le `doc_hash` fourni, ou le blake3 du JSON canonique des passages (règle
dans `05-api.md`).

Le dossier `<data>/tmp/` est commun à tous les index d'une racine. Un dossier
`online-*` n'est donc un résidu de crash que si son **verrou de présence**
`<data>/tmp/online-<batch_id>.lock`, son voisin et dérivé de la même valeur, peut
être pris (ADR-0056). L'ingestion prend ce verrou exclusif avant de créer le
dossier du lot et le tient jusqu'après `attach` ; une seconde ingestion du même
`batch_id` échoue en `overloaded` (503) sans rien créer. Le nettoyage
n'interprète jamais l'absence de `.lock` comme la preuve d'un résidu : il ouvre
le fichier en lecture-écriture avec création, tente le verrou, et ne supprime le
dossier puis le verrou qu'en cas de succès. Toute autre erreur de verrou, comme
sur un montage réseau qui ne partage pas les verrous, ne supprime rien et laisse
le dossier en place.

### Lot d'export

`crible export` (et `POST /v1/indexes/{index}/exports`) écrit un lot
`replace_all` des données vivantes de l'index à la génération G, dans
`exchange/outbox/<index>-g<G>/`, en parties d'au plus un million de passages,
sans aucun iid (ADR-0061). C'est un lot de ce format, `format_version` 1, avec
les valeurs de manifeste suivantes :

| Champ | Valeur |
|---|---|
| `batch_id` | `<index>-g<G>`, identique au nom du dossier |
| `producer.name` | `crible` |
| `producer.version` | `CARGO_PKG_VERSION` — version du binaire qui a écrit le lot |
| `producer.host` | `crible-export` |
| `chunker` | `{ "name": "none", "version": "1" }` |
| `template` | `{ "name": "none", "version": "1" }` |
| `created_at` | date RFC 3339 de l'export |
| `index` | nom de l'index source |

`none`/`1` veut dire ici « aucun découpage ni recomposition par le producteur de
ce lot » : l'export recopie des entrées encodeur déjà stockées et n'a pas de
producteur qui découpe. Le manifeste décrit donc le lot exporté, pas l'histoire
des documents ; la provenance du découpage d'origine reste dans l'index source
(`batches.manifest`, `segments.batch_id`) et n'est pas transportée. C'est une
perte assumée de l'export, et elle ne change ni INV-08, ni la recette, ni
INV-10 : aucun contrôle ne lit `chunker` ni `template`.

Contrairement au lot du mode en ligne, le manifeste d'export **n'a pas à être
déterministe** : `created_at` et `producer.version` changent d'un export à
l'autre. Deux exports de la même génération sont donc deux lots distincts ; les
attacher tous deux au même index cible donne `409 batch_conflict`.
Ré-attacher **le même dossier** reste un no-op (INV-10) : même `batch_id`, même
manifeste.

`index` vaut le nom de l'index source, et l'attache exige ce nom : la
restauration se fait dans un index vide de même nom et de même configuration.
Un dossier de destination déjà présent n'est jamais écrasé ; l'export le refuse
en `invalid_request`. L'écriture passe par un dossier temporaire
`exchange/outbox/.tmp-*`, dont une panne peut laisser un résidu que rien ne
supprime automatiquement.

## Colonnes Parquet

Tous les fichiers sont au format Parquet. Les colonnes de schéma sont
nommées comme les champs déclarés dans l'index. L'ordre des colonnes est
libre ; les noms et les types font foi.

### `documents.parquet`

| Colonne | Type Parquet | Nullable | Remarque |
|---|---|---|---|
| `doc_id` | `BYTE_ARRAY` / UTF-8 (`utf8`) | non | Identifiant externe du document, non vide, unique dans le lot toutes parties confondues (ADR-0040). |
| `doc_hash` | `BYTE_ARRAY` / UTF-8 (`utf8`) | non | Opaque, fourni par le producteur. |
| *(un champ de document)* | selon `type` | oui | Voir la table ci-dessous. |

Correspondance des types de champ de document :

| Type du schéma | Type Parquet |
|---|---|
| `keyword` | `BYTE_ARRAY` / UTF-8 (`utf8`) |
| `integer` | `INT64` |
| `float` | `DOUBLE` (`float64`) |
| `date` | `DATE` (`date32`, jours depuis 1970-01-01) |
| `bool` | `BOOLEAN` |

Tous les champs déclarés sont nullables, même si le producteur n'émet jamais
de null. Un champ absent du schéma de l'index est refusé.

### `passages.parquet`

| Colonne | Type Parquet | Nullable | Remarque |
|---|---|---|---|
| `doc_id` | `BYTE_ARRAY` / UTF-8 (`utf8`) | non | Document du passage. |
| `ord` | `UINT_32` (`uint32`) | non | Rang du passage dans le document, contigu et à partir de 0. Les lignes d'un même document arrivent dans l'ordre croissant des `ord` (ADR-0039). |
| `text` | `BYTE_ARRAY` / UTF-8 (`utf8`) | non | Entrée encodeur exacte, sans prompt. |
| *(un champ de passage)* | selon `type` | oui | Même correspondance que les champs de document. |
| `vector` | `FIXED_SIZE_BINARY` | non | Voir ci-dessous. |

### `deletions.parquet`

| Colonne | Type Parquet | Nullable | Remarque |
|---|---|---|---|
| `doc_id` | `BYTE_ARRAY` / UTF-8 (`utf8`) | non | Document à supprimer, unique dans le lot (ADR-0040). |

Un `doc_id` apparaît **au plus une fois dans `documents`**, toutes parties
confondues, et au plus une fois dans `deletions` (ADR-0040). Un doublon dans
l'un ou l'autre rôle fait rejeter le lot avec `400 invalid_request`. Un `doc_id`
présent à la fois dans `documents` et dans `deletions` fait rejeter le lot.

## Vecteurs

La colonne `vector` est un `FIXED_SIZE_BINARY` de :

- `dims × 2` octets en `fp16` ;
- `dims × 4` octets en `fp32`.

Les composantes sont écrites en **little-endian**, dans l'ordre des dimensions,
sans en-tête ni compression spécifique. Un `fp16` est le motif binaire IEEE 754
demi-précision sur 16 bits. Ce choix reste lisible tel quel par pyarrow,
arrow-rs et DuckDB.

Le nombre d'octets doit correspondre exactement à `dims` et à `vector_dtype`.
Les vecteurs doivent être finis. Si `normalize` vaut vrai, leur norme L2 doit
être proche de 1 (`|‖v‖ − 1| ≤ 1e-2`), lors du contrôle 7 de `04`.

## Contrôles à l'attache

L'ordre de `04-donnees-et-ingestion.md` fait foi. La phase P0 couvre les
contrôles qui ne demandent pas de lire le Parquet :

1. `format` et `format_version` sont reconnus.
2. L'index existe (le nom du manifeste correspond à l'index cible) et le mode
   est valide.
3. Le modèle du lot est compatible avec celui de l'index : chaque champ est
   comparé à la configuration résolue du modèle (`ModelConfig`), sinon
   `422 incompatible_model`.
4. La forme des checksums et la liste des fichiers sont cohérentes (la
   vérification des valeurs est faite en P1).

La phase P1 ajoute :

5. Le schéma Parquet est conforme au schéma de l'index.
6. Les comptes correspondent aux lignes lues, les `ord` sont contigus et chaque
   document a au moins un passage. Les lignes d'un même document arrivent dans
   l'ordre croissant des `ord`, et cet ordre se poursuit d'une partie à l'autre
   (ADR-0039). Les parties d'un rôle se parcourent dans l'ordre croissant de la
   valeur numérique de leur suffixe, sans doublon de valeur. Un `doc_id`
   apparaît au plus une fois dans `documents` et au plus une fois dans
   `deletions` (ADR-0040).
7. Les vecteurs sont finis et normalés quand `normalize` est vrai.

La phase P2 ajoute les contrôles 8 (longueur en tokens) et 9 (recette).

## Erreurs

| Situation | Code |
|---|---|
| Format ou version inconnus, mode inconnu, lot destiné à un autre index | `400 invalid_request` |
| Liste de fichiers incohérente, fichier inconnu, checksum mal formé, comptes incohérents | `400 invalid_request` |
| Deux parties d'un même rôle de même valeur numérique, `doc_id` dupliqué dans `documents` ou dans `deletions` (ADR-0040) | `400 invalid_request` |
| `replace_all` avec un fichier `deletions` | `400 invalid_request` |
| Modèle du lot incompatible avec celui de l'index | `422 incompatible_model` |

Un `batch_id` déjà appliqué avec le même manifeste ne fait rien et renvoie le
rapport d'origine (INV-10). Le même `batch_id` avec un autre manifeste renvoie
`409 batch_conflict`.

## Références

`04-donnees-et-ingestion.md` ; `05-api.md` (lot en ligne) ; ADR-0005 (précision
de référence figée) ; ADR-0012 (format de lot public) ; ADR-0027 (`serde_json`) ;
ADR-0039 (ordre des lignes dans un lot v1) ; ADR-0040 (unicité des documents et
des parties dans un lot v1) ; ADR-0055 (lot interne du mode en ligne) ; ADR-0061 (manifeste d'un lot
exporté) ;
`08-invariants-et-tests.md` (INV-03, INV-10, INV-11, INV-12).
