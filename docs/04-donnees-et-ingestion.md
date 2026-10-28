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
    "rescore_candidates": 5000,
    "exact_threshold": 5000,
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
| `model` (son empreinte est copiée dans `index.json`) | `lexical.enabled`, `lexical.language` (domaine d'ADR-0086), `lexical.positions` : déclenchent un job de reconstruction du lexical |
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

`lexical.language` a lui aussi un **domaine**, contrôlé à la création de l'index comme à tout `PATCH` (ADR-0054), mais sous forme de **table** et non d'intervalle (ADR-0086) :

| Réglage | Domaine | Défaut |
|---|---|---|
| `lexical.language` | dix-huit codes ISO 639-1 en minuscules : `ar`, `da`, `de`, `el`, `en`, `es`, `fi`, `fr`, `hu`, `it`, `nl`, `no`, `pt`, `ro`, `ru`, `sv`, `ta`, `tr` | `"fr"` (exemple plus haut) |

Aucune autre valeur n'est acceptée, et aucune normalisation n'est faite en silence : `"FR"`, `"Fr"`, `" fr"`, `"français"`, `"French"` et `""` sont refusés avec `invalid_request` (400). La valeur stockée dans `index.json` est donc toujours la forme canonique. La table d'ADR-0086 nomme pour chaque code la variante tantivy, la présence du stemmer et celle d'une liste de mots vides : `ar`, `el`, `ro`, `ta` et `tr` n'en ont pas, et leur chaîne d'analyse omet `StopWordFilter`. Un `PATCH` accepté sur ce réglage déclenche un job de reconstruction du lexical, comme les autres réglages de `lexical.*`.

Parmi les réglages de `search`, `exact_threshold` est **optionnelle** dans `index.json` (ADR-0074). Absente à la création de l'index, elle prend la valeur de `rescore_candidates` à ce moment, et cette valeur est écrite dans la configuration stockée : elle se lit ensuite comme une valeur explicite. Un `PATCH` ultérieur de `rescore_candidates` ne déplace pas le seuil ; seul un `PATCH` de `exact_threshold` le change. Ce réglage n'a pas de domaine au sens d'`ingest.*` : tout entier de `0` à `u32::MAX` est accepté, sans nouvelle erreur.

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

Les durées par étape ne sont pas persistées (ADR-0042). Chaque étape de l'attache — `validation`, `segment`, `transaction`, `lexical`, `manifest`, `snapshot` — est mesurée, mais ces durées sont de la **télémétrie d'appel** : elles sont rendues au seul appelant d'origine de l'attache et restent disponibles pour la journalisation et les mesures. Aucune n'entre dans `batches.report`, et aucune écriture SQLite n'a lieu après le point de commit (ADR-0016).

## Mode en ligne (sans producteur)

`POST /v1/indexes/{index}/documents` accepte des documents avec leurs passages (ADR-0055) :
- **sans vecteurs** : le serveur encode les passages lui-même en fp32 sur CPU. C'est lent, donc réservé aux petits volumes ;
- **avec vecteurs** : mode « j'apporte mes vecteurs ».

Le serveur construit ensuite un lot interne et suit exactement le même chemin qu'un lot produit ailleurs.

**Document en ligne.** Un document porte `doc_id`, un `doc_hash` facultatif, ses `fields` et ses `passages` (`text`, `fields`, et un `vector` facultatif par passage). **Un seul type** décrit ce document : il sert au corps HTTP de la route et à la commande de la CLI, pour qu'aucun format parallèle ne soit maintenu. La règle vaut **par forme d'entrée** : l'entrée d'un patch de métadonnées est un second type, plus petit (`doc_id` et `fields` seulement), partagé de la même façon par la CLI et la route, et un patch ne porte jamais de `passages` (ADR-0060).

**Fichier d'entrée de `crible ingest`.** `crible ingest --data … --models … --index … documents.jsonl` lit un objet document par ligne, au format ci-dessus. `--models` est obligatoire, comme pour `crible attach` : le modèle vit hors du volume de données. L'enveloppe est **implicite** : le mode vaut `upsert` et le fichier ne porte pas de suppressions ; une suppression passe par un lot déposé.

**Lot interne.** Le lot est écrit dans `<data>/tmp/online-<batch_id>/`, **hors du dossier de l'index**, avec `std::fs`, et supprimé par un garde qui s'exécute en succès comme en échec. Il n'est jamais un dossier `tmp-*` du dossier de l'index : la reprise efface ces dossiers (INV-04), et une ouverture concurrente ferait disparaître le lot sous l'attache en cours.

**Verrou de présence.** Un lot en ligne est vivant tant qu'un verrou exclusif est tenu sur `<data>/tmp/online-<batch_id>.lock` (ADR-0056). L'ingestion ouvre ce fichier en lecture-écriture avec création, y prend le verrou avec `try_lock` **avant** de créer le dossier du lot, et le tient jusqu'**après** `attach`. Si le verrou est déjà tenu, l'ingestion échoue en `overloaded` (503) sans rien créer, ni dossier ni ligne dans `batches` : le message nomme le fichier de verrou et invite à réessayer. Le garde supprime le dossier, puis le verrou. `<data>/tmp/` étant commun à tous les index d'une racine, la présence du verrou est la seule preuve qu'un lot n'est pas un résidu : sans verrou pris, le nettoyage ne supprime rien (ADR-0056).

**Identité et manifeste.**

- `batch_id` = `online-<blake3 des octets du fichier>` pour la CLI. L'identité vient du **contenu**, comme le digest d'ADR-0041 : rejouer le même fichier ne fait rien (INV-10). Deux fichiers d'octets différents qui portent les mêmes documents donnent deux lots, dont le second est entièrement `unchanged`. La route HTTP prend son `batch_id` dans le corps de la requête ; l'idempotence reste celle de la section « Sémantique d'attache ».
- Le manifeste du lot interne est **déterministe**, pour qu'un rejeu ne produise pas de `409 batch_conflict` :

| Champ | Valeur |
|---|---|
| `producer.name` | `crible-online` |
| `producer.version` | `1` — version du **format du lot en ligne**, pas du binaire |
| `producer.host` | `crible-online` |
| `chunker` | `{ "name": "none", "version": "1" }` |
| `template` | `{ "name": "none", "version": "1" }` |
| `created_at` | `1970-01-01T00:00:00Z`, sentinelle documentée |

`producer.version` porte la version du format, parce que la version du binaire change à chaque mise à jour de Crible : un rejeu après mise à jour produirait un autre manifeste, donc un `409`. Il doit être incrémenté **seulement** avec un changement de format du lot en ligne. `created_at` est une sentinelle parce qu'aucun contrôle ni rapport ne la lit, et que le mtime n'est pas stable d'une copie à l'autre.

**Mode.** Le lot en ligne n'accepte que **`upsert`**, pour la CLI comme pour la route. `replace_all` est refusé en `invalid_request`, avec un message qui nomme le mode : 10 000 passages au plus par requête (`docs/05-api.md`) le rendent inutilisable, et `replace_all` reste le mode du lot déposé.

**`doc_hash`.** Le `doc_hash` du lot interne est **toujours** rempli : le document le fournit, ou il vaut le blake3 du JSON canonique de ses passages (règle et forme dans `docs/05-api.md`, ADR-0055).

## Patch de métadonnées

Un patch modifie les **champs d'un document** sans ré-encodage : il ne touche ni le texte, ni les vecteurs, ni les iid, ni le lexical, et il ne change pas le `doc_hash` (ADR-0060). Les champs de document sont hors du hash (ADR-0055) ; un patch ne peut donc pas se confondre avec un remplacement de passages.

**Entrée.** Une entrée de patch est un objet `{ "doc_id": …, "fields": { … } }`. Un seul type décrit cette entrée, et il sert aux deux chemins (ADR-0060) :

- `crible patch --data <dir> --index <nom> <fichier>.jsonl` lit un objet par ligne, le fichier étant passé en argument positionnel. Le fichier est du JSONL, et il n'y a pas de `--models` : un patch n'encode rien ;
- `PATCH /v1/indexes/{index}/documents` reçoit `{ "documents": [ … ] }`, dont chaque élément a exactement la forme ci-dessus.

**Règles.**

- **Fusion** : chaque clé de `fields` remplace la valeur du champ de même nom ; une clé absente garde sa valeur. Un `fields: {}` est un no-op, compté `unchanged`. La comparaison se fait sur le JSON canonique des champs (ADR-0019).
- **`null`** est une **valeur posée**, jamais un retrait : il n'existe aucun moyen de retirer une clé de `fields` par un patch (ADR-0035).
- **Champs acceptés** : seuls les champs déclarés dans `document_fields`. Une clé inconnue, un champ de passage, `doc_id` ou `doc_hash` sont refusés en `invalid_request`, avec un message qui nomme le champ, le niveau attendu, le chemin JSON et, pour un fichier, le numéro de ligne ; la valeur n'apparaît jamais dans le message (ADR-0030, ADR-0060).
- Un champ `filterable: false` ou `returned: false` **accepte** un patch : la valeur est stockée, elle ne participe simplement ni aux filtres ni au payload du document. Un champ ajouté par un `PATCH` de configuration est nullable, donc le patch peut y poser `null`.
- **Valeur mal typée** : refus en `invalid_request`, avant toute écriture, avec le type attendu dans le message ; jamais `invalid_filter`, qui appartient à la grammaire des filtres.
- **Document inconnu ou retiré** : refus **atomique** en `not_found`. Aucun document du fichier n'est appliqué, et le message nomme le `doc_id` fautif et son numéro de ligne. Le moteur ne distingue pas un `doc_id` jamais vu d'un `doc_id` retiré, puisque la ligne `documents` est supprimée au commit (ADR-0041). Un patch ne **crée** jamais de document : la création exige des passages.
- Un `doc_id` dupliqué dans le fichier est refusé en `invalid_request`.
- **Limites** : 32 Mo et 10 000 entrées, pour le corps HTTP comme pour le fichier de la CLI, refus en `payload_too_large`.
- **Atomicité** : le fichier passe entièrement ou pas du tout (**INV-03**).

**Effet et rapport.** Le patch écrit `documents` et `meta.generation` dans la transaction du commit de génération, pose `updated_generation = G+1` sur chaque ligne patchée, et laisse `doc_hash` inchangé (ADR-0060, `docs/02`). Il n'écrit ni segment, ni ligne `batches`. Le rapport donne `{ updated, unchanged, generation }`, par document : `generation` est celle qui est visible après l'appel. Un patch sans effet ne crée aucune génération, ne laisse aucun manifeste, et rend `updated: 0` avec la génération courante. Le rapport n'est pas persisté : la CLI le rend sur sa sortie standard, et le job le porte en mémoire pour sa durée de vie (ADR-0060).

**Un patch n'est pas durable.** Une attache ultérieure du même document avec un `doc_hash` identique réécrit les champs de document s'ils diffèrent (`:169`) : la valeur patchée est alors écrasée sans erreur. C'est la conséquence acceptée du fait que les champs de document ne font pas partie du `doc_hash`.

Un patch passe la même file d'écriture que les autres écritures de l'index. Faire tourner `crible patch` sur un index servi par un serveur reste une règle d'exploitation : `busy_timeout` n'assure que la vivacité (ADR-0038), pas l'exclusion entre deux processus.

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
| Contenu | Lot `replace_all` des données vivantes à la génération G | Copie cohérente de l'époque : `VACUUM INTO` de SQLite, `segment.json` et `vectors.f16`/`f32` de chaque segment, `index.json`, `manifests/<G>.json`, `lexical/` s'il existe ; un `snapshot.json` atteste la copie (ADR-0064) |
| Destination | `exchange/outbox/<index>-g<G>/`, en parties d'au plus 1 M passages | `exchange/snapshots/<index>-e<E>-g<G>/` |
| Restauration | Attacher le lot dans un index **vide, de même nom et de même configuration** que l'index source (ADR-0061) | CLI, hors ligne : installé comme nouvelle époque E+1, génération laissée à G, puis `CURRENT_EPOCH` (ADR-0065) |
| Avantage | Portable, indépendant de la version de Crible, lisible avec DuckDB | Restauration rapide, sans reconstruction |
| Pendant l'opération | L'écrivain continue | L'écrivain est suspendu le temps de la copie ; en CLI, la prise est hors ligne, le serveur étant arrêté (ADR-0065) |

Le manifeste d'un lot exporté suit le format public (`docs/format-lot-v1.md`, section « Lot d'export ») et porte les règles suivantes (ADR-0061) :

- `chunker` et `template` valent `{ "name": "none", "version": "1" }` : l'export ne découpe ni ne recompose rien, il recopie des entrées encodeur déjà stockées. La provenance du découpage d'origine n'est pas transportée ; elle reste dans l'index source (`batches.manifest`, `segments.batch_id`).
- `producer.version` est la version du binaire Crible qui a écrit le lot (`CARGO_PKG_VERSION`), et `producer.name` vaut `crible` avec `producer.host` à `crible-export`.
- `created_at` est la date réelle de l'export, lue dans SQLite. Le manifeste d'export **n'a pas à être déterministe** : deux exports de la même génération sont deux lots distincts, et les attacher tous deux au même index cible donne `409 batch_conflict`.
- `index` vaut le nom de l'index source ; c'est aussi le nom attendu à l'attache, d'où la restauration dans un index de même nom.
- Un dossier `exchange/outbox/<index>-g<G>/` déjà présent est refusé : l'export n'écrase jamais un export existant, qu'une attache peut être en train de lire. Le refus est rendu en `invalid_request` (400).
- L'export écrit sous un dossier temporaire `exchange/outbox/.tmp-<batch_id>-<pid>-<n>-<nanos>` puis le renomme. Une panne laisse donc un résidu `.tmp-*` que ni la reprise (`02-architecture.md`) ni une commande ne supprime : il se nettoie à la main.
- Une course de génération (l'index commite entre l'instantané et la première lecture) relance l'export sur un instantané neuf, au plus trois fois, puis rend `overloaded` (503). Ce nombre de tentatives est une mitigation interne, hors contrat.

### Règles de l'instantané (ADR-0064, ADR-0065)

`crible snapshot --data <dir> --exchange <dir> --index <nom>` n'encode rien et ne prend donc pas `--models`.

- **Contenu copié** : le résultat de `VACUUM INTO` de `records.sqlite`, posé à la racine de l'époque sous le nom `records.sqlite` ; pour chaque segment de l'époque, `segment.json` et `vectors.f16` ou `vectors.f32` ; `index.json` ; `manifests/<G>.json` ; `lexical/` s'il existe. Le `VACUUM INTO` rend un fichier unique et cohérent, sans `-wal` ni `-shm` : la copie n'a pas à embarquer les fichiers annexes de WAL.
- **Exclusions** : `codes.bin`, `cache/` et les autres fichiers dérivés, reconstructibles (`02-architecture.md`), ainsi que `CURRENT_EPOCH`, que la restauration pose elle-même. `codes.bin` est dérivé des vecteurs : le copier ajouterait des octets sans rien rendre possible. `CURRENT_EPOCH`, lui, nomme l'époque de la **source** ; l'installer ferait pointer l'index cible vers une époque qui peut déjà exister chez lui.
- **« Le manifeste » de la fiche** désigne `manifests/<G>.json` de l'époque copiée. Seul le manifeste de la génération G est copié ; ceux des générations antérieures restent l'histoire de l'index.
- **`snapshot.json`** : un JSON versionné (`format_version: 1`, INV-15), **sans horodatage**, écrit **après** la copie, par fichier temporaire, fsync, renommage puis fsync du dossier. Il porte le nom de l'index, l'époque source E, la génération G et, pour **chaque fichier copié** sauf lui-même, son chemin relatif, sa taille en octets et son blake3 sous la forme `blake3:<hex>`, la liste étant triée par chemin. Il **atteste une copie** : il ne décrit pas une génération et ne remplace ni `manifests/<G>.json`, ni `meta.generation`, qui restent dérivés de SQLite (ADR-0016). Sa présence à la racine est le signal que la copie est allée au bout.
- **Refus à la restauration** : la restauration refuse en `invalid_request`, sans rien écrire, un instantané sans `snapshot.json`, dont le `format_version` est inconnu, auquel il manque un fichier listé, dont un fichier listé a une taille ou un blake3 différents, ou qui contient un fichier **non listé**. Le refus d'un fichier en trop est strict : une note ajoutée à la main, ou un `.DS_Store` déposé par macOS, rend l'instantané inutilisable.
- **Destination occupée** : un dossier `exchange/snapshots/<index>-e<E>-g<G>/` déjà présent est refusé en `invalid_request` (400), avec le dossier nommé ; un instantané n'écrase jamais un instantané existant, qu'une restauration ou un opérateur peut être en train de lire. C'est la règle de l'export (ADR-0061).
- **Transit** : l'instantané s'écrit sous `exchange/snapshots/.tmp-<index>-e<E>-g<G>-<pid>-<n>-<nanos>/`, puis le dossier est renommé ; le nom temporaire garantit que deux prises simultanées n'écrivent jamais au même endroit. Une panne laisse un résidu `.tmp-*` que ni la reprise ni une commande ne supprime : il se nettoie à la main, comme celui de l'export.
- **Hors ligne en P3** : l'opérateur arrête le serveur, comme pour `crible restore`. Le cœur de la commande prend `&mut GenerationStore`, ce qui suspend l'écrivain au sens d'ADR-0017 — les écritures sont retenues, les lectures continuent — quand il tourne dans le serveur ; un processus ne peut pas suspendre l'écrivain d'un autre, et aucun verrou inter-processus n'est ajouté : c'est la règle d'exploitation déjà écrite pour `crible patch`, `busy_timeout` n'assurant que la vivacité (ADR-0038). La future route `POST /v1/indexes/{index}/snapshots` (P6) réutilisera le même cœur.
- **Rapport** : destination, époque E, génération G, nombre de documents, nombre de passages, octets copiés.

### Règles de la restauration (ADR-0065)

`crible restore --data <dir> --snapshot <dir>` est **hors ligne**. Aucune des deux commandes de sauvegarde ne prend `--models`, et aucune ne charge le modèle : aucune recette n'a lieu.

1. **Contrôler l'instantané** : les cinq refus ci-dessus, avant toute écriture.
2. **Contrôler la cible** : le nom de l'index cible est celui de `snapshot.json`, la commande n'a pas d'option `--index`. Un index **absent** est créé depuis l'`index.json` de l'instantané, avec l'époque initiale d'une création et la génération G de l'instantané. Un index **existant** doit porter un `index.json` **identique** ; sinon la restauration est refusée en `invalid_request`, avec un message qui nomme le réglage divergent. Une cible à une génération **supérieure** à G est acceptée : c'est le retour arrière voulu.
3. **Écrire l'époque E+1** : la copie va dans `epochs/tmp-<E+1>/`, puis fsync et renommage vers `epochs/<E+1>/` — le protocole de la compaction. Un dossier de transit ou une époque E+1 qu'un résidu occuperait est supprimé avant l'écriture ; `meta.epoch` de la base copiée et `manifests/<G>.json` reçoivent le numéro E+1.
4. **Basculer** : `CURRENT_EPOCH ← E+1` en dernier. Une panne avant cette écriture laisse l'index cible intact (INV-03, INV-04).

- **Génération inchangée** : la restauration laisse la génération à **G** ; elle ne l'incrémente pas, et c'est ce qui satisfait « une restauration redonne la génération ». L'époque est copiée en entier, donc les iid sont conservés et restent denses : aucun renumérotage, et les résultats sont identiques sans reconstruction. Les `segment.json` ne sont pas réécrits : leur `generation` et leur `batch_id` disent la provenance du segment, comme pour une compaction.
- **Époque remplacée conservée** : elle reste sur disque, le rapport la nomme, et elle se supprime **à la main**. Dès qu'une écriture a suivi la prise, elle contient des passages qu'aucune copie valide ne porte : `reclaim_epochs` la détruirait, ce qui violerait INV-14. Le disque croît donc si personne ne nettoie.
- **Couple (époque, génération) non monotone** : une restauration change l'époque sans changer la génération. Tout code ou outil qui suppose que les deux avancent ensemble doit être relu.
- **Historique `batches`** : l'époque restaurée porte l'historique de l'instantané. Les lots attachés entre G et la génération remplacée ne sont plus dans l'état vivant — ils restent dans l'époque remplacée — et INV-10 ne les couvre plus : les rattacher les appliquerait de nouveau au lieu de ne rien faire.
- **Rapport** : nom de l'index, époque remplacée (ou « aucune »), nouvelle époque, génération, nombre de documents, nombre de passages.
- **Codes** : `invalid_request` (400) pour un instantané absent, incomplet ou altéré, une destination occupée ou une configuration divergente ; `internal` (500) pour une panne disque.

## Le découpage : qui fait quoi

| Producteur (métier) | Crible (moteur) |
|---|---|
| Découpe le texte : unités, taille, chevauchement, parties des décisions | Modélise documents → passages ; le document est l'unité de mise à jour |
| Compose le gabarit et l'en-tête | Stocke l'entrée encodeur exacte (`text`) |
| Encode les passages | Vérifie les longueurs et fait la recette |
| Déclare `chunker` et `template` dans le manifeste | Trace le découpeur de chaque lot ; regroupe par document ; dédoublonne par contenu |

Plus tard, le découpeur générique de Legifacile (`decouper` : 1 024 tokens, 15 % de chevauchement) pourra devenir le découpeur par défaut du moteur, pour les projets qui n'ont pas de pipeline à eux. Ce n'est pas prévu en v1.
