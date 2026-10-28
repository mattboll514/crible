# Outils hors ligne de Crible

Ce dossier contient les outils qui ne font pas partie du moteur : la génération
des jeux synthétiques et la référence exacte de la phase P1.

| Fichier | Rôle |
|---|---|
| `gen_synth.py` | Génère `synth-small` en trois lots et son jeu de requêtes ; option `--large` pour le jeu de latence. |
| `reference_topk.py` | Rejoue l'attache des lots et calcule le top-k exact en numpy ; écrit `expected.jsonl`. |
| `export_onnx.py` | Exporte l'encodeur mDenseOn en ONNX (fp32 et INT8), copie `tokenizer.json` et écrit `model.json`. |
| `reference_vectors.py` | Écrit les 20 textes et vecteurs de référence du test Rust de `CHARGER-MODELE`. |
| `fake_encoder.py` | Génère le petit modèle ONNX factice versionné, utilisé par les tests Rust qui n'ont pas le modèle réel. |
| `demo_j2_batches.py` | Construit les deux lots fautifs de la démo J2 (vecteurs sans prompt, passage trop long) à partir de l'échantillon Legifacile. |
| `fetch_model_artifact.py` | Télécharge et vérifie l'artefact de modèle de la CI (ADR-0057), ou affiche la clé de cache. |
| `tests/` | Tests pytest des outils. |
| `bench/mes00/` | Campagne de mesure de la référence pgvector (MES-00), indépendante de P1. |
| `bench/mes04/` | Campagne de mesure de la parité de l'encodeur (MES-04) : préparation des requêtes, référence PyTorch, comparaison ONNX et analyse. |
| `bench/mes15/` | Campagne de mesure du coût d'un commit de génération (MES-15) : `run.sh` (pilote complet) et `merge.py` (fusion des données brutes). |
| `bench/accelere-recette/` | Campagne de ventilation de l'étape `validation` d'un attache (`ACCELERER-RECETTE`, MES-08) : `run.sh` (chargement fp32, postes, encadrement) et `merge.py` (tableaux du rapport). |

Les identifiants, les options et les clés JSON sont en anglais ; ce README est
en français, comme les documents de `docs/`.

## Installation

`tools/` est un projet uv. Il dépend du paquet `crible` de `clients/python` par
un chemin relatif, ce qui apporte `pyarrow`, et il déclare `blake3` pour la
somme de contrôle (ADR-0034).

L'export ONNX a ses propres dépendances, réunies dans le groupe de dépendances
`export` : `torch.onnx.export` écrit le graphe et `onnxruntime.quantization` le
quantifie (ADR-0050), plus les paquets que le script importe directement
(`transformers` épinglé, `onnx`, `onnxscript` — exigé par l'exportateur
*dynamo* —, `tokenizers`, `huggingface-hub`). `optimum` ne fait plus partie du
groupe : aucune de ses versions publiées n'accepte `transformers` 5 (`optimum-onnx`
0.1.0 plafonne à `< 4.58`, ADR-0050). Un `uv sync` ordinaire **n'installe pas**
ce groupe : la CI et les tests lancent `uv sync` puis `uv run pytest` sans
PyTorch, et c'est volontaire, car les tests de `tools/` ne chargent aucun modèle.

```sh
cd tools
uv sync                     # outils courants : jeux synthétiques, référence
uv sync --group export      # ajoute PyTorch, transformers 5.14.1 et ONNX Runtime
```

Les roues de `torch` viennent de l'index CPU-only de PyTorch
(`https://download.pytorch.org/whl/cpu`, déclaré en `[[tool.uv.index]]` dans
`pyproject.toml`) : Crible ne fait que du CPU, et la roue PyPI de Linux
embarque la grappe CUDA, soit plusieurs Go inutiles. `transformers` est
**épinglé à `5.14.1`** (borne exacte, ADR-0050) : c'est la version sous laquelle
ont été calculés les vecteurs de production de Legifacile. La relever demande
une nouvelle mesure de fidélité (critère (ii) plus bas).

### Groupe `ci` et artefact de modèle (ADR-0057)

Le job CI `rust-onnx` exécute les tests Rust qui chargent ONNX Runtime, et le
job `tools` lance les tests MES-04 qui demandent `tokenizers`. Les deux
installent le groupe `ci`, qui contient `onnxruntime==1.31.0` et `tokenizers`,
**sans** `torch` : c'est le runtime d'exécution, pas la chaîne d'export.

```sh
cd tools
uv sync --group ci
```

`fetch_model_artifact.py` télécharge les fichiers du vrai modèle depuis le dépôt
Hugging Face public épinglé dans `ci/model-artifact.toml`, vérifie leur blake3 et
les place dans `data/models/mdenseon/`. Un fichier déjà présent (cache GitHub
Actions) est vérifié comme un téléchargement neuf ; un écart échoue en nommant la
clé de cache à purger. Aucun jeton n'est nécessaire.

```sh
uv run python fetch_model_artifact.py cache-key   # clé du cache Actions
uv run python fetch_model_artifact.py             # télécharge et vérifie
```

### ONNX Runtime sur Mac, pour les tests Rust

`crible-encode` charge ONNX Runtime **dynamiquement** (`ort` en `load-dynamic`,
`docs/07-stack-et-deploiement.md`) : la bibliothèque n'est pas liée à la
compilation, et `ORT_DYLIB_PATH` doit nommer son fichier à l'exécution. Les tests
Rust qui chargent un modèle sont **ignorés** par défaut, parce que la CI n'a ni
modèle ni ONNX Runtime ; `--ignored` les lance.

Deux provenances :

- par Homebrew, `brew install onnxruntime`, puis `ORT_DYLIB_PATH` vaut
  `$(brew --prefix onnxruntime)/lib/libonnxruntime.dylib` (sur Intel,
  `/usr/local` remplace `/opt/homebrew`) ;
- la roue Python du groupe `export`, déjà installée par
  `uv sync --group export`. Afficher le chemin, puis l'exporter :

```sh
cd tools
uv run python -c 'import onnxruntime, pathlib; print(next((pathlib.Path(onnxruntime.__file__).parent / "capi").glob("libonnxruntime*.dylib")))'
export ORT_DYLIB_PATH=/chemin/absolu/affiché
```

Lancer ensuite les tests depuis la racine du dépôt. Sur les cinq tests de
`crible-encode`, deux tournent sans modèle (les hash) et trois chargent le
modèle ; c'est l'option `--ignored` qui lance ces trois-là.

```sh
cargo test -p crible-encode                  # 2 tests, sans modèle
ORT_DYLIB_PATH=/chemin/absolu/libonnxruntime.dylib \
  cargo test -p crible-encode -- --ignored   # 3 tests, avec le modèle
```

Version mesurée le 2026-10-23 sur un Mac arm64 : ONNX Runtime **1.31.0**, celle
de la roue Python et du champ `export` de `model.json`.

## Générer `synth-small`

Depuis `tools/` :

```sh
uv run python gen_synth.py --out ../data/synth-small
uv run python reference_topk.py --dataset ../data/synth-small
```

La première commande écrit `index.json`, les trois lots et `queries.jsonl`. La
seconde écrit `expected.jsonl` et affiche la somme de contrôle du jeu complet.

`data/` est gitignoré : ces fichiers sont générés, jamais commités. Seule la
somme de contrôle est versionnée, dans ce README.

### Régénérer dans le même dossier

Le générateur **refuse** un dossier non vide et affiche quoi faire. Pour
remplacer une génération existante, passer `--overwrite` :

```sh
uv run python gen_synth.py --out ../data/synth-small --overwrite
uv run python reference_topk.py --dataset ../data/synth-small
```

`--overwrite` ne supprime que ce que le mode courant écrit : `index.json` et
`queries.jsonl`/`expected.jsonl` pour `synth-small`, `index.json` seul pour
`--large`, plus ses sous-dossiers de lots (`l1`, `l2`, `l3`, ou `large-0001`).
Tout autre fichier du dossier est laissé intact ; pour repartir d'un dossier
vraiment propre, le supprimer à la main.

### Contenu

- 768 dimensions, vecteurs en fp16, 16 grappes ; le champ de document
  `categorie` nomme la grappe et sert de `cluster_by`.
- Champs de document : `annee` (integer), `categorie` (keyword),
  `date_publication` (date), `publie` (bool), `score` (float), `source`
  (keyword).
- Champs de passage : `date_effet` (date), `partie` (keyword), `poids` (float),
  `principal` (bool), `rang` (integer).
- Chaque type est déclaré aux deux niveaux, et chaque champ sauf `categorie`
  contient des nulls.
- Les textes sont du texte aléatoire : la fiche met les textes réalistes hors
  périmètre.

| Lot | Contenu | Documents | Passages | Suppressions |
|---|---|---:|---:|---:|
| `l1` | création | 2 000 | 10 000 | 0 |
| `l2` | 300 documents modifiés, 100 nouveaux | 400 | 2 000 | 0 |
| `l3` | 150 suppressions, 50 mises à jour de métadonnées seules | 50 | 250 | 150 |

Une mise à jour de métadonnées seules renvoie le **même `doc_hash`** et **tous
les passages** du document, conformément à `docs/04-donnees-et-ingestion.md`
(« `doc_hash` identique » : passages ignorés, champs de document mis à jour).
`BatchWriter.add_document` refuse d'ailleurs une liste de passages vide. Seuls
les champs de document changent. Les 150 suppressions passent par `delete`.

### État de référence

**`expected.jsonl` décrit l'index après l'attache des trois lots `l1`, `l2` puis
`l3`**, et non l'état après `l1`. C'est l'état que J1 compare.

Après les trois attachements :

- **1 950 documents vivants** (2 000 + 100 − 150) ;
- **9 750 passages vivants** (10 000 + 500 − 750) ;
- 150 documents en pierres tombales au sens de l'attache, retirés de la
  référence.

La référence rejoue la sémantique du tableau de `docs/04` : document inconnu
ajouté, `doc_hash` identique → passages conservés et champs de document mis à
jour, `doc_hash` différent → passages remplacés, `deletions.parquet` → document
retiré.

### Déterminisme

Deux générations avec la même graine écrivent des fichiers **identiques au bit
près**. Le générateur ne lit ni l'horloge ni l'environnement :

- chaque tirage vient d'un flux nommé `SeedSequence([graine, numéro])` ;
- l'ordre des fichiers, des lignes et des clés JSON est fixé ;
- `created_at` est fixé par lot, dérivé de la graine seule, via le paramètre
  `created_at` de `BatchWriter` (par défaut, `BatchWriter` garde l'heure
  courante : son comportement ne change pas pour les autres producteurs) ;
- `producer.host` vaut `"gen_synth"`, une valeur stable passée au paramètre
  `host` de `BatchWriter` (ADR-0036 ; par défaut, `BatchWriter` garde le nom
  d'hôte de la machine).

La somme de contrôle ci-dessous ne dépend donc ni de la machine, ni de la date
de génération : elle est la même en CI et chez un autre développeur. Un test la
régénère et la compare à la valeur consignée ici.

Cette égalité entre machines vaut aussi pour `expected.jsonl`, que la somme
couvre. Elle n'a pas toujours été vraie : tant que `reference_topk.py` calculait
ses scores par un produit matriciel confié à BLAS, l'ordre de sommation variait
d'une bibliothèque à l'autre et la somme avec lui (ADR-0045). La référence
calcule désormais ses scores exactement, sans BLAS, donc ses octets sont les
mêmes partout.

### Somme de contrôle

Somme blake3 des artefacts connus de `data/synth-small` (les fichiers écrits par
le générateur et ceux listés par chaque manifeste de lot), triés par chemin
relatif, chaque chemin et son contenu entrant dans le calcul (voir
`gen_synth.dataset_checksum`). Un fichier étranger, comme un `.DS_Store`, ne
change pas la somme.

```
synth-small : blake3:357de46a0e288ae3f5c85f1241558494d12cf092d1d8c4f348c0fa289aaaeee2
```

Elle couvre `index.json`, les trois lots, `queries.jsonl` et `expected.jsonl`.
`reference_topk.py` la réaffiche après avoir écrit `expected.jsonl`.

Cette somme a changé le **2026-10-28** (ADR-0078, décisions 2 et 3) : le gabarit
de `index.json` passe `search.rescore_candidates` et `search.exact_threshold` de
1 000 à 5 000, le seuil suivant `N` à la création de l'index. `scan` reste
`none`, donc aucune stratégie de test ne change. Le générateur n'écrit rien
d'autre différemment. Les rapports datés antérieurs (`docs/jalons/J1-*`,
MES-13, MES-14) citent l'ancienne somme `blake3:a9854f1e…` et restent tels
quels.

## Requêtes et résultats attendus

`queries.jsonl` contient 200 requêtes, une par ligne :

```json
{"query_id": 0, "k": 10, "vector": [0.097…, …], "filter": null}
```

- `vector` : 768 flottants, de norme 1 (float32 arrondi au plus proche) ;
- `filter` : `null` ou un filtre de la grammaire de `docs/05-api.md` ;
- les 200 requêtes varient les vecteurs (grappe, mélange de grappes, aléatoire)
  et les filtres (égalité, `ne`, `in`, `nin`, intervalles d'entier, de flottant
  et de date, booléen, `exists`, `$and`, `$or`, `$not`, champ de passage) ;
- elles incluent `{"source": {"ne": "journal"}}`, son complément
  `{"$not": {"source": "journal"}}` (résultats différents sur les nulls,
  ADR-0035), et `{"source": {"exists": true}}` / `{"source": {"exists": false}}`.

`expected.jsonl` contient une ligne par requête, dans le même ordre :

```json
{"query_id": 0, "k": 10, "pool_size": 9750,
 "results": [{"key": "synth-01163#3", "doc_id": "synth-01163", "ord": 3,
              "score": 0.6926815509796143}]}
```

Le score est le cosinus exact. Les vecteurs du lot sont décodés de fp16 vers
f32, la requête est normalisée en f32, puis le produit scalaire est calculé
**exactement, sans BLAS** (ADR-0045) : chaque produit de deux composantes f32
est formé en f64, où il est exact, la ligne est sommée par `math.fsum`, et le
résultat reçoit un seul arrondi final en f32. La norme de la requête suit la
même règle. L'ordre de sommation n'a donc plus d'effet : le fichier est le même
sur toute machine, quelle que soit la bibliothèque BLAS. Le moteur, lui, somme
en f32 scalaire dans l'ordre des composantes. Les résultats sont triés par score
décroissant, avec un **départage stable par clé** `doc_id#ord`, comme l'exige
J1. J1 compare les clés et leur ordre, avec une égalité des scores à `1e-5`
près.

### Quasi-égalités de scores

Mesure sur les 200 requêtes de `expected.jsonl` : nombre de requêtes dont deux
résultats **adjacents** diffèrent de moins de 1e-6, puis de moins de 1e-5. Elle
se recalcule directement depuis `expected.jsonl`.

| Calcul | < 1e-6 | < 1e-5 |
|---|---:|---:|
| exact, arrondi une fois en f32 | 4 | 55 |

L'arrondi f32 rend certains écarts exactement nuls : l'ordre est déterministe
pour la référence grâce au départage par clé. Ces deux décomptes décrivent le
jeu de données, pas la plateforme : la référence étant calculée exactement, ils
sont les mêmes sur toute machine (ADR-0045). Le moteur, lui, somme en f32 dans
l'ordre des composantes ; sur une quasi-égalité, une égalité exacte de la
référence peut donc ne pas l'être côté moteur. J1 compare les clés, puis les
scores à `1e-5` près.

### Sous-ensemble de la grammaire

`reference_topk.py` accepte le ET implicite d'un objet, `$and`, `$or`, `$not`,
le raccourci scalaire pour `eq`, et les opérateurs `eq`, `ne`, `lt`, `lte`,
`gt`, `gte`, `in`, `nin`, `exists`. Le jeu de requêtes n'utilise que ce
sous-ensemble.

Il valide aussi chaque filtre contre le schéma de `index.json`, comme le
moteur : un champ inconnu, un champ marqué `filterable: false` et un opérateur
que le type du champ n'accepte pas sont refusés, avec un message qui nomme le
champ et l'opérateur. La table type → opérateurs est celle de
`docs/05-api.md` : `keyword` accepte `eq`, `ne`, `in`, `nin`, `exists` ;
`integer`, `float` et `date` acceptent en plus `lt`, `lte`, `gt` et `gte` ;
`bool` n'accepte que `eq` et `exists`. Les 200 requêtes générées passent cette
validation, et un test le vérifie.

Une valeur nulle ne satisfait jamais une comparaison, `ne` ou `nin` : seul
`exists` regarde la présence (`exists: true` = présent et non null,
`exists: false` = absent ou null). `$not` complète sur **tous les passages
vivants**, nulls compris, donc `{"f": {"ne": "a"}}` et
`{"$not": {"f": {"eq": "a"}}}` ne sont pas équivalents. C'est la règle SQL de
l'ADR-0035, désormais écrite dans `docs/03-recherche.md` et `docs/05-api.md`.

## Jeu de latence `--large`

```sh
uv run python gen_synth.py --large --out ../data/synth-large
```

Écrit un lot `large-0001` de 8,6 M passages (environ 13 Go en fp16), réparti en
parties de 1 M de passages au plus, avec des vecteurs aléatoires normalisés et
un passage par document. Il déclare son propre chunker (`passages_per_document`
= 1) au lieu de réutiliser celui de `synth-small`. Le jeu ne sert qu'à la
latence : il n'a ni requêtes ni résultats attendus. `--large-passages N` permet
une taille réduite, utilisée par les tests pour vérifier le découpage en parties
sans écrire le jeu complet.

## Exporter mDenseOn en ONNX

Depuis `tools/`, avec le groupe `export` installé :

```sh
uv run python export_onnx.py --out ../data/models/mdenseon --overwrite \
  --fidelity-sample ../data/legifacile-sample
```

Le script télécharge l'instantané épinglé de `lightonai/mDenseOn` dans le cache
Hugging Face, exporte le transformateur avec `torch.onnx.export` sous
`transformers==5.14.1` (ADR-0050), quantifie les poids en INT8 avec
`onnxruntime.quantization`, puis écrit quatre fichiers dans le dossier de
sortie :

| Fichier | Contenu |
|---|---|
| `model.onnx` | export fp32, axes `batch` et `sequence_length` dynamiques ; entrées `input_ids` et `attention_mask`, sortie `last_hidden_state` |
| `model.int8.onnx` | même graphe, poids quantifiés en INT8 par quantification dynamique d'ONNX Runtime ; graphe optionnel, réservé aux comparaisons, au repli et au diagnostic (ADR-0052) : les requêtes sont encodées en fp32 par défaut |
| `tokenizer.json` | tokenizer rapide de la révision épinglée, copié tel quel |
| `model.json` | hash blake3 des trois fichiers, révision, dimensions, pooling, normalisation, prompts, `max_tokens`, et champ `export` (versions de `transformers`, `torch` et `onnxruntime` de l'exécution, hors empreinte — ADR-0050) |

Le script écrit un cinquième fichier, `parity.json`, qui n'est pas un fichier de
modèle : c'est le compte rendu de la comparaison PyTorch contre ONNX de cette
exécution (date, révision, cosinus minimum et moyen, longueurs de tokens, et
sous `fidelity` la mesure contre les vecteurs de production quand
`--fidelity-sample` est donné). Le moteur ne lit que les quatre fichiers décrits
par `docs/04-donnees-et-ingestion.md` et **ignore tout autre fichier** du
dossier du modèle, `CHARGER-MODELE` compris.

Le graphe **n'applique aucun prompt et ne regroupe pas** : il rend le dernier
état caché, et Crible fait lui-même le pooling CLS puis la normalisation L2
(`docs/04-donnees-et-ingestion.md`). Les deux prompts `query: ` et
`document: ` sont donc ajoutés par le producteur et par le moteur, jamais par
ONNX.

`--overwrite` autorise l'écriture dans un dossier non vide. Le `model.json` et
le `parity.json` de l'export précédent sont remplacés une fois les nouveaux
fichiers lourds en place : ces deux fichiers décrivent une seule exécution, et
un ancien `parity.json` à côté d'un modèle neuf donnerait des chiffres faux. Un
export qui échoue avant ce point (téléchargement, export, quantification) laisse
donc l'export précédent **intact**, manifeste compris. `--skip-parity` saute la
comparaison PyTorch contre ONNX et l'exécution à 8 192 tokens (utile pour
réexporter rapidement) et laisse le dossier sans `parity.json` ; il est refusé
avec `--fidelity-sample`, puisque la mesure de fidélité est consignée dans ce
rapport.

`model.json` est écrit avec les clés triées, donc octet pour octet stable, et
**en dernier**, par un fichier temporaire renommé : il n'existe que si tous les
autres fichiers sont écrits. La configuration du modèle est vérifiée avant
l'export (dimensions, `max_position_embeddings`, prompts de
`config_sentence_transformers.json`, troncature du tokenizer) : un désaccord
arrête l'export au lieu de produire un `model.json` faux. Avec la parité, la
limite de 8 192 tokens et le cosinus sont en outre prouvés avant l'écriture ;
avec `--skip-parity`, `max_tokens` reste la valeur déclarée par le modèle, sans
exécution à cette longueur ni `parity.json` pour l'attester. Les erreurs réseau,
Hugging Face, exportateur et ONNX Runtime donnent une ligne `error:` (aplatie si
le message en tenait plusieurs) et un code de sortie non nul, sans trace brute.

### Révision et hash mesurés

Révision épinglée : `a5fdb000f7a21da96c3bddde3a782ef777316df3`
(`lightonai/mDenseOn`, dernier commit du dépôt au 2026-07-30). Ré-export mesuré
le **2026-10-23**, sur un Mac arm64 (8 cœurs, 24 Go), avec Python 3.13,
`torch` 2.14.1, `transformers` 5.14.1, `onnxscript` 0.7.2, `onnx` 1.23.2 et
`onnxruntime` 1.31.0. Ce sont les versions que `model.json` porte dans son champ
`export` :

```
model.onnx         blake3:7b6eb68099dbeac6826e282dd575aec766dfbe1bedac143bae094b12f82094a2
model.int8.onnx    blake3:0534b4ee028cfc4a6a7ca9dcca7a7c908110372033dab33a4f75fbe37d5941a5
tokenizer.json     blake3:8a801317cfb1eb744734392dc0aeef36d6d39660a0a884f06a0338e9bb5357db
```

Deux exports complets dans deux dossiers différents, à révision et versions
égales, rendent **les mêmes trois hash** (vérifié par
`CRIBLE_EXPORT_REPRO=1 uv run pytest tests/test_reexport_transformers5.py`).
`torch.onnx.export` seul n'y suffisait pas : il inscrit une trace de pile par
nœud, et cette trace nomme un module généré par un compteur qui change à chaque
exécution (`<eval_with_key>.68`, `.145`…). Le script retire ces métadonnées de
débogage avant d'écrire le graphe (`clear_node_metadata`) ; le graphe exécuté
est identique. La longueur et les identifiants du tracé sont fixés
(`TRACE_SEQUENCE_LENGTH`, `TRACE_SEED`), car ils changent les octets du graphe
sans changer ses axes. Les poids sont en ligne : `model.onnx` est un seul
fichier, lu tel quel par l'empreinte du modèle.

### `max_tokens`

`max_tokens` vaut **8192**, la valeur de `max_position_embeddings` du modèle et
de la troncature de `tokenizer.json`. Le script la vérifie en exécutant le
graphe exporté sur exactement 8192 tokens, avec la valeur relue dans le
`model.json` en cours d'écriture : l'exécution rend la forme attendue
`[1, 8192, 768]`, en 55 s sur le Mac de mesure. Cette vérification appartient
au rapport de parité : `--skip-parity` la saute, et `max_tokens` reste alors la
valeur déclarée par le modèle, sans exécution à cette longueur.

Aucune troncature n'est silencieuse (**INV-11**) : `encode_tokens` mesure d'abord
la longueur réelle de l'entrée, sans troncature, et **refuse** un texte qui
dépasse la limite au lieu d'en perdre la fin. C'est vrai des deux mesures
ci-dessous et de la vérification à 8 192 tokens ; un passage trop long arrête
l'export avec une ligne `error:`.

### Parité PyTorch contre ONNX fp32

Critère (i) de la fiche : sur 100 textes, le cosinus entre les vecteurs PyTorch
sous `transformers` 5.14.1 et ONNX fp32 vaut au moins 0,9999. Le corpus est
français, déterministe et versionné avec le script (`comparison_texts`) : il ne
contient aucune requête de production. Il va de textes d'une vingtaine de tokens
à un texte de **8 088 tokens**, donc proche de la limite, qui vérifie aussi
qu'aucune troncature n'intervient.

| Précision | Textes | Cosinus minimum | Cosinus moyen |
|---|---:|---:|---:|
| fp32 | 100 | 0,9999999998854414 | 0,9999999999971361 |

L'écart au cosinus parfait vaut 1,1·10⁻¹⁰ au pire : très au-dessus de la barre
de 0,9999. Le fichier INT8 ne sert qu'aux comparaisons, au repli et au
diagnostic (ADR-0052) : les requêtes sont encodées en fp32 par défaut. Il
n'entre pas dans l'empreinte du modèle et n'est pas comparé ici, ce que fera
MES-04 sur les requêtes réelles. Les nombres ci-dessus sont écrits par le script dans
`parity.json`, à côté du modèle, avec la révision et la date.

La comparaison porte sur le **vecteur regroupé et normalisé**, le seul que
Crible consomme. Après pooling CLS et normalisation, l'écart de cosinus tombe
sous 10⁻¹⁰ alors que l'état caché brut diffère de quelques 10⁻⁴ : c'est
l'arithmétique flottante du graphe exporté qui diffère, pas l'usage qui en est
fait.

### Fidélité au modèle de production

Critère (ii), celui qu'aucun export précédent ne mesurait (ADR-0050) : le graphe
ONNX fp32, appliqué aux textes des passages avec le prompt `document: `, doit
retrouver les **vecteurs de production** de l'échantillon Legifacile, à un
cosinus d'au moins **0,999** — le seuil de recette de `docs/04` (INV-12).

Le tirage est **déterministe** : 100 passages par structure, soit 600, tirés par
l'ordre du digest BLAKE3 de leur clé `doc_id#ord`. Il ne dépend donc ni de
l'ordre des lignes ni du découpage en fichiers Parquet.

Avant de mesurer, le script vérifie la **provenance** du lot, lue dans le
`manifest.json` de chaque structure : `model.hf_id`, `model.revision`,
`model.dims` et `model.prompt_document` doivent être ceux du modèle de référence.
Un échantillon un jour ré-encodé par l'ONNX de Crible — ou par une autre
révision — rendrait le critère auto-référentiel, exactement l'aveuglement que
dénonce l'ADR-0050 ; il est refusé. La déclaration commune retenue est consignée
dans `parity.json`, sous `fidelity.manifest`.

| Passages | Minimum | p1 | Médiane | Seuil |
|---:|---:|---:|---:|---:|
| 600 | 0,9999603983 | 0,9999935039 | 0,9999983370 | 0,999 |

L'ancien graphe exporté sous `transformers` 4.57.6, lui, plafonnait à
0,983–0,990 (`data/legifacile-sample/README.md`, « Limite connue ») : c'est cet
écart qui a motivé le ré-export et l'ADR-0050.

Les deux mesures se reproduisent avec une seule commande, depuis `tools/`, avec
le groupe `export` installé et `data/legifacile-sample/` présent :

```sh
uv run python export_onnx.py --out ../data/models/mdenseon --overwrite \
  --fidelity-sample ../data/legifacile-sample
```

Elle réécrit `model.json` et `parity.json` ; ce dernier porte la parité sous
`min_cosine` / `mean_cosine` et la fidélité sous `fidelity`. Les deux mesures se
relisent aussi sans réexporter, par les tests qui les portent :

```sh
uv run pytest tests/test_reexport_transformers5.py -q
```

La parité seule se reproduit en sautant la fidélité (`--fidelity-sample` absent),
et l'export reproductible en deux dossiers par
`CRIBLE_EXPORT_REPRO=1 uv run pytest tests/test_reexport_transformers5.py`.

### Note sur le tokenizer

`tokenizer_config.json` de la révision épinglée nomme la classe
`TokenizersBackend`, qui n'existe que dans `transformers` 5.x. Le script lit
donc `tokenizer.json` avec la bibliothèque `tokenizers`, qui est aussi celle que
Crible utilise côté Rust : les identifiants sont exactement ceux du fichier
exporté.

## Vecteurs de référence de `CHARGER-MODELE`

Le critère d'acceptation de `CHARGER-MODELE` (phase P2) compare le vecteur rendu
par `crible_encode::Model` à celui de `tools/export_onnx.py` en fp32, à `1e-4`
près. La référence est une petite fixture versionnée, dans
`crates/crible-encode/tests/fixtures/mdenseon/` :

| Fichier | Contenu |
|---|---|
| `texts.txt` | les 20 textes de référence, un par ligne, en UTF-8 |
| `vectors.f32` | 20 × 768 flottants fp32 little-endian, dans le même ordre |

Les 20 textes sont les premiers de `comparison_texts()`, le corpus français
déterministe de la parité. Les vecteurs sont calculés par le graphe ONNX fp32
avec le prompt `document: `, puis pooling CLS et normalisation L2, exactement
comme `onnx_vectors` : c'est ce que le moteur doit retrouver.

Pour régénérer la fixture, depuis `tools/`, avec le groupe `export` installé et
`data/models/mdenseon/` présent :

```sh
uv run python reference_vectors.py
```

Le script relit `data/models/mdenseon/model.onnx`, il ne l'exporte pas ; un
modèle ré-exporté se régénère d'abord par la commande de la section
« Exporter mDenseOn en ONNX ». Le script affiche le blake3 des deux fichiers.
Mesure du 2026-10-23, avec
`onnxruntime` 1.31.0 :

```
texts.txt    blake3:5dd96a6a618520a2c6adeb0638f0470564d3fe754820c2a2f504796caf61eeba
vectors.f32  blake3:b2ce1dc7e03523690272b81b412d8dc8dfd9b82ed5ba743adbedf47ed2f33eae
```

Le test Rust `matches_the_python_fp32_vectors_within_1e_4` est **ignoré** par
défaut : il lui faut le dossier du modèle et `ORT_DYLIB_PATH`. Depuis la racine
du dépôt, avec la bibliothèque de la section « ONNX Runtime sur Mac » :

```sh
ORT_DYLIB_PATH=/chemin/absolu/libonnxruntime.dylib \
  cargo test -p crible-encode -p crible-core -- --ignored --nocapture
```

Les quatre tests ignorés sont ceux qui lisent le modèle : le `model.json` réel,
le chargement séquentiel, le chargement concurrent, et la parité 1e-4. Sans le
modèle ou la bibliothèque, `--ignored` échoue avec la commande d'export, au lieu
de passer sans rien vérifier.

## Tests

```sh
cd tools && uv run pytest
```

Les tests de `gen_synth.py` couvrent les deux critères d'acceptation de la
tâche P1 :

- deux générations avec la même graine donnent des fichiers identiques au bit
  près ;
- le jeu contient tous les types de champs aux deux niveaux, des nulls, des
  champs de document et de passage, et des vecteurs en grappes.

Ils vérifient aussi les comptes des lots, l'état rejoué contre les Parquet
(passages d'un document modifié par `l2`, passages et champs de document d'une
mise à jour de métadonnées `l3`), la couverture des filtres, la sémantique des
nulls de l'ADR-0035 (`ne` contre `$not` + `eq`, `exists`), le top-k exact contre
une boucle naïve sur un échantillon stratifié (une requête par famille de
filtre), le découpage en parties du jeu `--large`, le refus d'un dossier non
vide et le périmètre de `--overwrite`, et l'égalité entre la somme régénérée et
celle consignée plus haut.

Les tests de `export_onnx.py` tournent **hors ligne, sans modèle et sans
PyTorch** : ils vérifient la forme de `model.json` attendue par
`CHARGER-MODELE`, les digestes blake3 (contre des valeurs de référence), la
stabilité octet pour octet et l'écriture atomique du fichier, le corpus de
parité (100 textes déterministes, dont un proche de la limite), le pooling CLS,
la normalisation, le cosinus, l'application du prompt et la limite de troncature,
le contrôle des prompts du `config_sentence_transformers.json` de l'instantané,
le refus d'un dossier non vide, le nettoyage du manifeste et du rapport une fois
les fichiers lourds réécrits, la sortie en
erreur sans trace brute, et l'absence d'import lourd à l'import du module.

Ceux de `REEXPORTER-MODELE-TRANSFORMERS-5` ajoutent le contrat de l'ADR-0050 :
épinglage de `transformers` et absence d'`optimum` dans le groupe `export`,
champ `export` de `model.json` (hors empreinte) et réécriture atomique, refus
d'une troncature silencieuse (une entrée trop longue lève, et rien n'est coupé),
tirage déterministe des 600 passages, provenance du lot vérifiée (révision,
prompt), seuil de fidélité de 0,999 (y compris le cas qui doit échouer : vecteurs
encodés sans le prompt), échec en une ligne quand l'exportateur ou le quantifieur
refuse le modèle, et reproductibilité des deux graphes
(`CRIBLE_EXPORT_REPRO=1`). L'export lui-même se lance à la main, et ses nombres
mesurés sont consignés plus haut.
