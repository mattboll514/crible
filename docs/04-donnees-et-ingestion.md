# 04 — Données et ingestion

## Configuration d'un index

Exemple : l'index `decisions` de Legifacile. Les champs sont indicatifs ; le schéma définitif sort de `DEFINIR-INDEX-LEGIFACILE`.

```json
{
  "name": "decisions",
  "model": "mdenseon",
  "reference_precision": "fp16",
  "schema": {
    "document_fields": {
      "juridiction":   { "type": "keyword" },
      "formation":     { "type": "keyword" },
      "date_decision": { "type": "date" },
      "numero":        { "type": "keyword" },
      "titre":         { "type": "keyword", "filterable": false }
    },
    "passage_fields": {
      "partie": { "type": "keyword" }
    }
  },
  "cluster_by": ["juridiction"],
  "lexical":    { "enabled": true, "language": "fr", "positions": false },
  "search": {
    "scan": "binary",
    "rescore_candidates": 1000,
    "exact_threshold": 1000,
    "candidates_per_branch": 100,
    "fusion": { "method": "rrf" }
  },
  "ingest":     { "acceptance_sample": 200, "acceptance_min_cosine": 0.999 },
  "compaction": { "dead_ratio": 0.10 }
}
```

**Types de champ** : `keyword`, `integer`, `float`, `date` (`YYYY-MM-DD`), `bool`.
- Chaque champ a deux options : `filterable` et `returned`, toutes deux à `true` par défaut.
- Tous les champs acceptent null.
- Les champs multivalués sont hors v1 (Q-03).
- Un même nom de champ ne peut pas être déclaré à la fois dans `document_fields` et dans `passage_fields` (ADR-0030). La validation le refuse avec `invalid_request` (400), et le message nomme le champ et les deux niveaux. La règle s'applique à la création de l'index et à tout ajout de champ par `PATCH`.

### Ce qui est figé, ce qui est réglable

| Figé à la création (changer = recréer l'index et ré-encoder) | Réglable ensuite (`PATCH`) |
|---|---|
| `name` | `search.*` |
| `model` (son empreinte est copiée dans `index.json`) | `lexical.enabled`, `lexical.language`, `lexical.positions` : déclenchent un job de reconstruction du lexical |
| `reference_precision` | `cluster_by` : pris en compte à la prochaine compaction |
| Type et niveau des champs existants | `filterable` et `returned` : les bitmaps sont reconstruites |
| | Ajout d'un champ (nullable) |
| | `ingest.*`, `compaction.*` |

Toute tentative de modifier un réglage figé renvoie `409 immutable_setting` (**INV-16**).

Deux réglages de `ingest.*` ont un **domaine**, contrôlé à la création de l'index comme à tout `PATCH` (ADR-0054) :

| Réglage | Domaine | Défaut |
|---|---|---|
| `acceptance_sample` | `≥ 1` | `200` |
| `acceptance_min_cosine` | `∈ ]0, 1]` | `0,999` (ADR-0052) |

Une valeur hors de ces bornes est refusée avec `invalid_request` (400) : la configuration est invalide en elle-même, quelle que soit la tâche qui la lit, et c'est la validation de la configuration qui la refuse, pas l'attache. `ingest.*` reste réglable par `PATCH` : aucune clé n'est retirée, et la valeur par défaut ne change pas.

## Le modèle

Fichier `models/<name>/model.json` :

```json
{
  "name": "mdenseon",
  "kind": "onnx",
  "hf_id": "lightonai/mDenseOn",
  "revision": "<sha du commit HF>",
  "dims": 768,
  "pooling": "cls",
  "normalize": true,
  "max_tokens": 8192,
  "export": {
    "transformers": "<version>",
    "torch": "<version>",
    "onnxruntime": "<version>"
  },
  "prompts": { "query": "query: ", "document": "document: " },
  "files": {
    "fp32":      { "path": "model.onnx",       "blake3": "…" },
    "int8":      { "path": "model.int8.onnx",  "blake3": "…" },
    "tokenizer": { "path": "tokenizer.json",   "blake3": "…" }
  }
}
```

- **Empreinte** : blake3 du JSON canonique de `hf_id`, `revision`, `dims`, `pooling`, `normalize`, `prompts`, et des hash des fichiers `fp32` et `tokenizer`. Le fichier INT8 n'entre pas dans l'empreinte, car il ne change pas les vecteurs de référence (ADR-0028) ; il sert aux comparaisons, au repli et au diagnostic (ADR-0052).
- **`max_tokens`** : la longueur maximale réellement supportée par l'export ONNX, à vérifier dans `EXPORTER-MODELE-ONNX`.
- **`export`** : versions de `transformers`, `torch` et `onnxruntime` qui ont produit le fichier fp32. Ce champ est **hors de l'empreinte** (ADR-0028, ADR-0050) : il sert au diagnostic, jamais à refuser un lot. La fidélité du modèle se mesure par la recette (contrôle 9, INV-12).
- **Usage des précisions** :
  - les **requêtes** sont encodées en fp32 par défaut (réglable par `[encoder].query_precision` : `fp32` ou `int8`, ADR-0052) ; `int8` ne garde que 79,2 % du top-10 exact du fp32 (MES-04) et sert aux comparaisons, au repli et au diagnostic ;
  - la **recette** et l'**encodage en ligne de documents** se font en fp32.
  - Le registre de sessions est indexé par `(empreinte, précision)` et chaque session est chargée à son premier usage : le fichier INT8 hors empreinte ne peut donc pas se confondre avec le graphe fp32 du même modèle (ADR-0051).
- **`kind: "external"`** : pour un projet qui apporte ses propres vecteurs. Le modèle n'a alors que `dims` et `normalize`, sans encodeur. Il n'y a pas de recette, et chaque requête doit fournir un `vector`.

## Modèle documents → passages

- **Document** : `doc_id`, `doc_hash`, champs de document.
  - Le `doc_hash` est opaque et fourni par le producteur.
  - **Le producteur doit le changer dès que les passages changent** : leur texte, leurs champs ou leur découpage.
- **Passage** : `doc_id`, `ord`, `text` (l'entrée encodeur, sans prompt), champs de passage, `vector`.
- **`passage_hash`** : blake3 de `text`, calculé par Crible. Il sert à `dedup: content`.
- **Règles** :
  - un document présent dans un lot arrive avec **tous** ses passages ;
  - les `ord` sont contigus et commencent à 0 ; les lignes d'un même document arrivent dans l'ordre croissant des `ord`, et cet ordre se poursuit d'une partie à l'autre (ADR-0039) ;
  - un document a au moins un passage ; pour le retirer, on passe par `deletions`.

## Format de lot v1

Un lot est un dossier `<batch_id>/` :

```
manifest.json
documents.parquet      (ou documents-0001.parquet, … pour les gros lots)
passages.parquet       (ou passages-0001.parquet, …)
deletions.parquet      (optionnel)
```

`manifest.json` :

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

Colonnes Parquet :

| Fichier | Colonnes |
|---|---|
| `documents` | `doc_id` (utf8, non null), `doc_hash` (utf8, non null), puis une colonne par champ de document : keyword → utf8, integer → int64, float → float64, date → date32, bool → bool |
| `passages` | `doc_id` (utf8), `ord` (uint32), `text` (utf8), une colonne par champ de passage, `vector` |
| `deletions` | `doc_id` (utf8) |

La colonne `vector` est un `fixed_size_binary` de `dims × 2` octets en fp16 little-endian (`dims × 4` en fp32). Ce format reste lisible tel quel par pyarrow, arrow-rs et DuckDB.

**Modes** :
- `upsert` (défaut) : les documents listés sont créés, remplacés ou mis à jour, et les autres ne changent pas.
- `replace_all` : le lot est le **contenu complet** de l'index. Tout document existant absent du lot est supprimé. Le fichier `deletions` est interdit dans ce mode. C'est ce qui sert à l'import et à la reconstruction.

## Sémantique d'attache

| Situation du document | Effet |
|---|---|
| `doc_id` inconnu | Ajout de ses passages |
| Connu, `doc_hash` identique | Passages ignorés ; champs de document mis à jour s'ils diffèrent |
| Connu, `doc_hash` différent | Anciens passages → pierres tombales ; nouveaux ajoutés ; champs mis à jour |
| Dans `deletions` | Pierres tombales sur tous ses passages ; document retiré (ligne `documents` supprimée) |
| `doc_id` à la fois dans `documents` et `deletions` | Le lot est rejeté |
| Suppression d'un `doc_id` inconnu | Ignorée, comptée dans le rapport |

- **Document retiré** (ADR-0041) : la ligne `documents` est supprimée dans la transaction du commit de génération, qu'il soit nommé dans `deletions` ou absent d'un lot `replace_all`. C'est la même transaction que les pierres tombales, le segment, la ligne `batches` et `meta.generation` ; il n'y a pas de seconde écriture après le point de commit (ADR-0016). Les pierres tombales restent posées sur les passages : c'est elles qui portent l'état de suppression jusqu'à la compaction. Un document ré-ajouté ensuite est un document neuf : il n'a ni `doc_hash` ni champs hérités.
- **Atomicité** : le lot passe entièrement ou pas du tout (**INV-03**).
- **Idempotence** :
  - un `batch_id` déjà appliqué ne fait rien, et renvoie le rapport **persisté** de l'attache d'origine — six comptes, statistiques de recette et génération —, sans durées (**INV-10**, ADR-0042) ;
  - le même `batch_id` avec un autre manifeste (`manifest_blake3` différent) renvoie `409 batch_conflict`.
  - `manifest_blake3` est le digest blake3 des octets de `manifest.json` **tels qu'ils ont été lus**, calculé avant toute désérialisation et jamais recalculé depuis une structure en mémoire. La forme est `blake3:<hex>`, `<hex>` en hexadécimal minuscule, comme les checksums de fichiers du lot (ADR-0041). Un manifeste re-sérialisé à l'identique sur le fond mais différent sur la forme (espaces, ordre des clés, échappements) donne donc un `manifest_blake3` différent, et `409 batch_conflict` s'il a déjà été appliqué. L'idempotence sert à rejouer le même dossier octet pour octet, pas à rattraper un manifeste réécrit ; ce n'est pas un contrôle d'intégrité du manifeste, mais une identification de contenu.

## Contrôles et recette, dans l'ordre

1. `format` et `format_version` sont reconnus.
2. L'index existe et le mode est valide.
3. Le modèle du lot est compatible avec celui de l'index : chaque champ (`hf_id`, `revision`, `dims`, `pooling`, `normalize`, `prompt_document`) est comparé à la configuration résolue du modèle (`ModelConfig`). Sinon : `422 incompatible_model`.
4. Les checksums des fichiers correspondent au manifeste.
5. Le schéma Parquet est conforme à celui de l'index : colonnes présentes et types corrects.
6. Les comptes correspondent au manifeste ; les `ord` sont contigus ; les lignes d'un même document arrivent dans l'ordre croissant des `ord`, d'une partie à l'autre comprise (ADR-0039) ; les parties d'un rôle se parcourent dans l'ordre croissant de la valeur numérique de leur suffixe, et deux parties d'un même rôle de même valeur numérique font rejeter le lot ; un `doc_id` est unique dans `documents`, toutes parties confondues, et dans `deletions` (ADR-0040) ; chaque document a au moins un passage.
7. Les vecteurs sont finis, et leur norme est proche de 1 (|‖v‖ − 1| ≤ 1e-2) si `normalize` est vrai.
8. **Longueur** : le nombre de tokens, prompt compris, ne dépasse pas `max_tokens`. Sinon, le lot est rejeté (**INV-11**, aucune troncature silencieuse).
9. **Recette** :
   - on tire un échantillon de `acceptance_sample` passages, avec une graine dérivée du `batch_id` ;
   - on les ré-encode en ONNX fp32, avec `prompt_document` ;
   - on calcule le cosinus avec les vecteurs du lot ;
   - si le minimum est sous `acceptance_min_cosine` (0,999 : valeur fixée par ADR-0052, elle n'est plus provisoire ; domaine `]0, 1]`, ADR-0054), le lot est refusé : `422 acceptance_failed` (**INV-12**). Le rapport donne le minimum, le p1 et le p50.

   C'est la généralisation de la recette de Legifacile, qui a déjà détecté un écart de version de transformers (cosinus 0,988).

   Un lot **sans aucun passage** — seulement des `deletions` — n'a rien à tirer au sort : son rapport porte `acceptance: null`, et c'est un fait du lot, pas un réglage (ADR-0054). `acceptance: null` n'a donc pas d'autre signification : `acceptance_sample` ne peut pas valoir `0`, et un lot qui contient des passages ne peut pas produire un rapport sans statistiques de recette.

**Le rapport d'attache** donne six comptes **par document et mutuellement exclusifs** (ADR-0041), puis :

| Clé | Cas compté |
|---|---|
| `added` | `doc_id` inconnu, passages ajoutés |
| `replaced` | `doc_id` connu, `doc_hash` différent : anciens passages en pierres tombales, nouveaux ajoutés |
| `unchanged` | `doc_id` connu, `doc_hash` identique, champs de document identiques |
| `metadata_updated` | `doc_id` connu, `doc_hash` identique, champs de document différents |
| `deleted` | `doc_id` connu nommé dans `deletions`, ou absent d'un lot `replace_all` |
| `ignored` | suppression d'un `doc_id` inconnu |

Un document compte dans exactement une de ces six catégories, donc la somme des six vaut le nombre de documents traités. Les comptes de passages ne sont pas repris dans le rapport : ce sont ceux du manifeste du lot (`counts.passages`).

Le rapport **persisté** donne aussi :
- les statistiques de la recette ;
- la génération obtenue.

Les durées par étape ne sont pas persistées (ADR-0042). Chaque étape de l'attache — `validation`, `segment`, `transaction`, `manifest`, `snapshot` — est mesurée, mais ces durées sont de la **télémétrie d'appel** : elles sont rendues au seul appelant d'origine de l'attache et restent disponibles pour la journalisation et les mesures. Aucune n'entre dans `batches.report`, et aucune écriture n'a lieu après le point de commit (ADR-0016).

## Mode en ligne (sans producteur)

`POST /v1/indexes/{index}/documents` accepte des documents avec leurs passages :
- **sans vecteurs** : le serveur encode les passages lui-même en fp32 sur CPU. C'est lent, donc réservé aux petits volumes ;
- **avec vecteurs** : mode « j'apporte mes vecteurs ».

Le serveur construit ensuite un lot interne et suit exactement le même chemin qu'un lot produit ailleurs.

## Producteurs

- **Paquet Python `crible`** (`clients/python/`) :
  - `BatchWriter` (pyarrow) écrit le manifeste et les fichiers Parquet ;
  - il applique localement les mêmes contrôles (schéma, `ord`, normes, comptes) et calcule les checksums ;
  - le client HTTP arrive en phase 6.
- **Producteur Legifacile**, dans le dépôt de Legifacile :
  - il réutilise le compositeur et le découpeur existants, et sort des lots grâce à `BatchWriter` ;
  - le quotidien tourne en natif sur le Mac (MPS), les campagnes sur OVH (CUDA) ;
  - pour la migration, il réutilise les vecteurs déjà dans pgvector, sans GPU. Les textes sont recomposés, puis vérifiés contre le sha256 `src_hash` existant.
- **Piste qualité, côté producteur** : répéter un en-tête court en tête de chaque passage (« Code civil, art. 1240 », « Cass. civ. 2e, 12/03/2020 — motifs »), puis le mesurer.

## Export, instantanés, restauration

| | Export | Instantané |
|---|---|---|
| Contenu | Lot `replace_all` des données vivantes à la génération G | Copie cohérente de l'époque : `VACUUM INTO` de SQLite, segments, tantivy, `index.json`, manifeste |
| Destination | `exchange/outbox/<index>-g<G>/`, en parties d'au plus 1 M passages | `exchange/snapshots/<index>-e<E>-g<G>/` |
| Restauration | Attacher le lot dans un index vide | CLI : installé comme nouvelle époque, puis `CURRENT_EPOCH` |
| Avantage | Portable, indépendant de la version de Crible, lisible avec DuckDB | Restauration rapide, sans reconstruction |
| Pendant l'opération | L'écrivain continue | L'écrivain est suspendu le temps de la copie |

## Le découpage : qui fait quoi

| Producteur (métier) | Crible (moteur) |
|---|---|
| Découpe le texte : unités, taille, chevauchement, parties des décisions | Modélise documents → passages ; le document est l'unité de mise à jour |
| Compose le gabarit et l'en-tête | Stocke l'entrée encodeur exacte (`text`) |
| Encode les passages | Vérifie les longueurs et fait la recette |
| Déclare `chunker` et `template` dans le manifeste | Trace le découpeur de chaque lot ; regroupe par document ; dédoublonne par contenu |

Plus tard, le découpeur générique de Legifacile (`decouper` : 1 024 tokens, 15 % de chevauchement) pourra devenir le découpeur par défaut du moteur, pour les projets qui n'ont pas de pipeline à eux. Ce n'est pas prévu en v1.
