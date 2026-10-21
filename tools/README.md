# Outils hors ligne de Crible

Ce dossier contient les outils qui ne font pas partie du moteur : la génération
des jeux synthétiques et la référence exacte de la phase P1.

| Fichier | Rôle |
|---|---|
| `gen_synth.py` | Génère `synth-small` en trois lots et son jeu de requêtes ; option `--large` pour le jeu de latence. |
| `reference_topk.py` | Rejoue l'attache des lots et calcule le top-k exact en numpy ; écrit `expected.jsonl`. |
| `tests/` | Tests pytest des deux outils. |
| `bench/mes00/` | Campagne de mesure de la référence pgvector (MES-00), indépendante de P1. |

Les identifiants, les options et les clés JSON sont en anglais ; ce README est
en français, comme les documents de `docs/`.

## Installation

`tools/` est un projet uv. Il dépend du paquet `crible` de `clients/python` par
un chemin relatif, ce qui apporte `pyarrow`, et il déclare `blake3` pour la
somme de contrôle (ADR-0034).

```sh
cd tools
uv sync
```

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

### Somme de contrôle

Somme blake3 des artefacts connus de `data/synth-small` (les fichiers écrits par
le générateur et ceux listés par chaque manifeste de lot), triés par chemin
relatif, chaque chemin et son contenu entrant dans le calcul (voir
`gen_synth.dataset_checksum`). Un fichier étranger, comme un `.DS_Store`, ne
change pas la somme.

```
synth-small : blake3:2d3d8265a8df350d816bccb5732fe338921d275cae5d091a96d0497646e58805
```

Elle couvre `index.json`, les trois lots, `queries.jsonl` et `expected.jsonl`.
`reference_topk.py` la réaffiche après avoir écrit `expected.jsonl`.

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

Le score est le cosinus exact, calculé **en f32 comme le moteur** : les vecteurs
du lot sont décodés de fp16 vers f32, la requête est normalisée en f32, puis le
produit scalaire est fait en f32. Les résultats sont triés par score
décroissant, avec un **départage stable par clé** `doc_id#ord`, comme l'exige
J1. J1 compare les clés et leur ordre, avec une égalité des scores à `1e-5`
près.

### Quasi-égalités de scores

Mesure sur les 200 requêtes de `expected.jsonl` : nombre de requêtes dont deux
résultats **adjacents** diffèrent de moins de 1e-6, puis de moins de 1e-5. Elle
se recalcule directement depuis `expected.jsonl`.

| Calcul | < 1e-6 | < 1e-5 |
|---|---:|---:|
| f32, référence actuelle | 5 | 55 |

L'arrondi f32 rend certains écarts exactement nuls : l'ordre est déterministe
pour la référence grâce au départage par clé. Le produit scalaire passe par
BLAS, dont l'ordre de sommation peut différer de la boucle scalaire du moteur ;
sur une quasi-égalité, une égalité exacte de la référence peut donc ne pas
l'être côté moteur. J1 compare les clés, puis les scores à `1e-5` près.

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

## Tests

```sh
cd tools && uv run pytest
```

Les tests couvrent les deux critères d'acceptation de la tâche :

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
