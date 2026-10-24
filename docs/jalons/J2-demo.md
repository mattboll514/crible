# Démo J2 — encodeur, recette et recherche texte

Commandes exactes du jalon J2, à jouer depuis la racine du dépôt. Elles partent
d'un état propre, créent l'index `dole` sur l'échantillon réel de Legifacile,
montrent qu'un lot dont les vecteurs ont été encodés sans le prompt est refusé
par la recette alors que le lot correct est accepté, montrent qu'un passage trop
long est refusé, puis cherchent en texte. Chaque refus est vérifié sans effet :
l'état de l'index ne bouge pas.

L'état propre ne touche que deux dossiers nommés, tous deux gitignorés :
`data/indexes/dole` (l'index de la démo) et `data/j2-demo` (les deux lots
fautifs construits pour elle). Aucun `rm -rf data`, et les autres index de
`data/indexes/` ne sont pas touchés.

## Prérequis

- `data/legifacile-sample/` (gitignoré) : les six lots réels et
  `configs/dole.json` ;
- `data/models/mdenseon/` : `model.json`, `tokenizer.json`, `model.onnx` et
  `model.int8.onnx` du ré-export d'ADR-0050 ;
- ONNX Runtime, chargé dynamiquement par `crible-encode` : `ORT_DYLIB_PATH`
  nomme le fichier de la bibliothèque. Le groupe `export` de `tools/` fournit
  celui de la roue Python (1.31.0), et l'on affiche son chemin sans le figer :

```sh
cd tools
uv sync --group export
uv run python -c 'import onnxruntime, pathlib; print(next((pathlib.Path(onnxruntime.__file__).parent / "capi").glob("libonnxruntime*")))'
cd ..
```

Ce chemin est propre à la machine ; il n'est pas recopié ici. Le groupe
`export` fournit aussi `onnxruntime` et `tokenizers` au constructeur des lots.

## Préambule — état propre et binaire

```sh
rm -rf data/indexes/dole data/j2-demo
cargo build --release
cd tools && uv sync --group export && cd ..
export ORT_DYLIB_PATH="$(cd tools && uv run python -c 'import onnxruntime, pathlib; print(next((pathlib.Path(onnxruntime.__file__).parent / "capi").glob("libonnxruntime*")))')"
```

Attendu : `cargo build --release` se termine par `Finished` ; la dernière
commande n'affiche rien et `ORT_DYLIB_PATH` nomme `libonnxruntime…` (`.dylib`
sur macOS, `.so` sur Linux).

## Étape 1 — construire les deux lots fautifs

Le lot correct de la démo est le lot `dole` de l'échantillon lui-même. Les deux
lots fautifs sont construits par `tools/demo_j2_batches.py`, jamais à la main :

- `j2-wrong-prompt` reprend 20 passages de 5 documents du lot `dole` (les
  premiers par `doc_id`) et ré-encode leurs **vecteurs** sans le prompt
  `document: `, puis les stocke en fp16 comme l'index ;
- `j2-too-long` porte un document dont l'unique passage fait 9 197 tokens avec
  le prompt, au-delà des 8 192 du modèle.

Les deux lots ont un manifeste, un schéma et des sommes de contrôle valides :
le moteur les refuse sur leur contenu, pas sur leur forme. Le script vérifie les
hash blake3 du modèle, fixe `created_at` et `host`, et ne supprime rien.

```sh
cd tools
uv run python demo_j2_batches.py --sample ../data/legifacile-sample --models ../data/models --out ../data/j2-demo
cd ..
```

Attendu, dans l'ordre (les cosinus viennent du modèle épinglé ; la construction
prend une trentaine de secondes) :

```text
j2-wrong-prompt: 5 documents, 20 passages -> ../data/j2-demo/j2-wrong-prompt
  cosine with the 'document: ' prompt: min 0.7323, median 0.8166; the acceptance threshold is 0.999
j2-too-long: 1 document, 1 passage, 9197 tokens with the prompt (max_tokens 8192) -> ../data/j2-demo/j2-too-long
```

Le cosinus de 0,73 est loin du seuil de 0,999 : la recette doit refuser ce lot.

## Étape 2 — créer l'index

```sh
./target/release/crible index create --data data --config data/legifacile-sample/configs/dole.json
```

Attendu : `index 'dole' created at data/indexes/dole (epoch 1, generation 0)`.

## Étape 3 — le lot corrompu est refusé par la recette

```sh
./target/release/crible attach --data data --models data/models --index dole data/j2-demo/j2-wrong-prompt
```

Attendu : **code de sortie 1** et ce message sur la sortie d'erreur, sans aucun
texte de passage :

```text
acceptance_failed: batch 'j2-wrong-prompt' is refused: the acceptance check re-encoded 20 passage(s) in fp32 and got min cosine 0.732275, below acceptance_min_cosine = 0.999 (INV-12); p1 = 0.732275, p50 = 0.813774
```

Le refus est celui de la recette (contrôle 9, INV-12), pas d'un format : le lot
aurait été accepté si ses vecteurs avaient porté le prompt. La médiane du script
et le `p50` de la recette n'ont pas la même définition (interpolation d'un côté,
rang le plus proche de l'autre) ; seul le minimum déclenche le refus, et il est
le même. Preuve que le lot refusé n'a rien laissé derrière lui :

```sh
./target/release/crible inspect --data data --index dole
```

Attendu : `generation: 0`, `documents: 0`, `passages: 0 live, 0 dead`,
`segments: 0`.

## Étape 4 — le lot correct est accepté

```sh
./target/release/crible attach --data data --models data/models --index dole data/legifacile-sample/batches/legifacile-sample-dole-v1
```

Attendu : **code de sortie 0**, la recette ré-encode 200 passages en fp32 et
les retrouve à 0,99999 et plus (la durée, environ deux minutes sur un Mac
arm64, n'est pas un critère) :

```text
batch 'legifacile-sample-dole-v1' attached to index 'dole': generation 1 (added 3593, replaced 0, unchanged 0, metadata_updated 0, deleted 0, ignored 0)
  acceptance: 200 passage(s) re-encoded, min cosine 0.999991, p1 0.999994, p50 0.999998
```

## Étape 5 — le passage trop long est refusé

```sh
./target/release/crible attach --data data --models data/models --index dole data/j2-demo/j2-too-long
```

Attendu : **code de sortie 1**. Le contrôle 8 (INV-11) passe avant la recette et
avant toute écriture, et nomme les clés fautives, jamais le texte :

```text
invalid_request: batch 'j2-too-long' is refused: 1 passage(s) exceed max_tokens = 8192 tokens, prompt included (INV-11); the first 1 keys, in the order of the batch rows, are: J2-TOO-LONG#0
```

## Étape 6 — état de l'instance

```sh
./target/release/crible inspect --data data --index dole
```

Attendu : les deux refus n'ont pas bougé l'index, qui reste à la génération
acceptée :

```text
index dole
  epoch: 1
  generation: 1
  documents: 3593
  passages: 7628 live, 0 dead
  dead ratio: 0.0000
  segments: 1
    seg-legifacile-sample-dole-v1: 7628 vectors
  bitmaps: 100381 bytes (live 8208, keywords 22941, booleans 0, columns 69232)
```

## Étape 7 — recherche en texte

La requête est `Q29` de `data/legifacile-sample/queries.jsonl`, dont l'attendu
`JORFDOLE000047079143` est dans l'échantillon. Le format `text` est le rapport
lisible de `crible search` : clé, score, champs et un extrait de texte ; le
format par défaut reste le JSON de `docs/05-api.md`, que les scripts parsent.

```sh
./target/release/crible search --data data --index dole --models data/models \
    --query "Quel dossier législatif correspond à la loi de 2024 pour contrôler l'immigration et améliorer l'intégration ?" \
    --k 3 --format text
```

Attendu : **code de sortie 0** ; le pool est l'ensemble des passages vivants,
le premier résultat est le dossier attendu, chaque résultat porte ses champs et
un extrait de texte de 160 caractères au plus, suivi d'une ellipse quand il est
coupé.

```text
index 'dole': 7628 passages in the pool, generation 1, 3 result(s), … ms
1. JORFDOLE000047079143#0  score … (cosine)
   fields: {"annee":2024,"type":"LOI_PUBLIEE"}
   text: LOI n° 2024-42 du 26 janvier 2024 pour contrôler l'immigration, améliorer l'intégration
2. JORFDOLE000051903428#0  score … (cosine)
   fields: {"annee":2025,"type":"ORDONNANCE_PUBLIEE"}
   text: Ordonnance n° 2025-646 du 16 juillet 2025 portant extension et adaptation des dispositions de la loi n° 2024-42 du 26 janvier 2024 pour contrôler l'immigration,…
3. JORFDOLE000047079143#3  score … (cosine)
   fields: {"annee":2024,"type":"LOI_PUBLIEE"}
   text: La loi d’orientation et du programmation du ministère de l’intérieur3 a acté une hausse du budget pour permettre d’atteindre l’objectif de 3 000 places de réten…
```

**Les scores ne sont pas consignés ici, volontairement.** La précision des
requêtes n'est pas exposée par la CLI : elle vaut `int8` par défaut aujourd'hui
(ADR-0051) et passera à `fp32` dans la tâche `NETTOYER-P2` (ADR-0052). Sur cette
requête, le premier résultat est le même aux deux précisions, vérifié hors CLI
avec le modèle épinglé ; seule la valeur du score change, par exemple 0,598 en
`int8` et 0,624 en `fp32` pour `JORFDOLE000047079143#0`. `took_ms` varie aussi :
il inclut le chargement de la session ONNX à la première requête.

La même recherche au format JSON, celui de `docs/05-api.md` :

```sh
./target/release/crible search --data data --index dole --models data/models \
    --query "Quel dossier législatif correspond à la loi sur l'immigration de 2024 ?" \
    --k 1
```

Attendu : un objet JSON avec `results` (ici un résultat), `pool_size: 7628`,
`generation: 1`, `took_ms` et `truncated_query: false`. Chaque résultat porte
`doc_id`, `passage`, `key`, `score`, `score_type: "cosine"`, `fields` et le
`text` complet (non tronqué). Le premier résultat est encore
`JORFDOLE000047079143#0`.

Aucun texte de requête ne part dans les journaux : la sortie standard porte le
résultat, la sortie d'erreur seulement `<code>: <message>` en cas d'échec, et
le journal (`WARN` sur la sortie d'erreur) ne reçoit ni requête ni valeur de
filtre.

## Ce que la démo prouve, et ce qu'elle renvoie

Les trois premiers points du jalon sont montrés de bout en bout : `crible
search --query` fonctionne sur l'échantillon Legifacile ; le lot corrompu
(mauvais prompt) est refusé et le lot correct accepté ; le passage trop long est
refusé. Les deux derniers points sont consignés ailleurs :

- **MES-04 et MES-05** : `docs/mesures/MES-04-parite-encodeur.md` (parité
  encodeur, rappel@10) et `docs/mesures/MES-05-latence-encodage.md` (latence
  d'encodage) ;
- **seuil de recette (Q-06)** : `acceptance_min_cosine = 0,999`, accepté par le
  propriétaire le 2026-10-24 (ADR-0052) et appliqué aux configurations d'index.

## Nettoyage

```sh
rm -rf data/indexes/dole data/j2-demo
```

Seuls ces deux dossiers sont retirés. Le second contient les lots fautifs ; le
premier, l'index de la démo. `data/legifacile-sample/` et `data/models/` restent
en place.
