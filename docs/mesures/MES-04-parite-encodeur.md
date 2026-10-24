# MES-04 — Parité de l'encodeur : INT8 et fp32 contre PyTorch, et rappel@10 de l'INT8

## Campagne du 2026-10-24

- Tâche : `MESURER-PARITE-ENCODEUR` (phase P2, fiche
  `docs/phases/P2-encodeur-et-recette.md`). Mesure demandée avant le jalon J2.
- Références : `docs/08-invariants-et-tests.md` (MES-04, cible de fidélité de
  0,99), `docs/04-donnees-et-ingestion.md` (« Le modèle », usage des précisions,
  contrôle 9), `docs/03-recherche.md` (« Le pipeline d'une requête », « Réglages
  de précision »), ADR-0050 (référence sous transformers 5.14.1),
  ADR-0051 (cache des requêtes, sessions INT8 et fp32), Q-06.
- Commit : `2fd3c29b64a9f5ce02d83cbdd9df49a5ce6d540a`, branche
  `tache/MESURER-PARITE-ENCODEUR` (fusion de `ENCODER-REQUETES`). **Aucun code de
  production n'a été modifié pendant la campagne.** Seuls ont été ajoutés le
  pilote `crates/crible-encode/examples/measure_queries.rs`, les scripts
  `tools/bench/mes04/` et leur entrée pytest dans `tools/pyproject.toml`.
- Machine : MacBook Air Apple M3 (8 cœurs : 4 performance, 4 efficacité), 24 Go
  de RAM (25 769 803 776 octets), macOS 15.7.7 (24G720), APFS. Mesure **native**.
  `cargo build --release` et l'exemple compilé en `--release`. La machine n'était
  pas au repos (moyenne de charge 2,95 à 4,45 sur 8 cœurs ; Arc, opencode et
  Ollama actifs), mais cette mesure est **déterministe** : elle ne dépend pas de
  la charge. Aucun autre travail lourd n'a été lancé par la campagne.
- Jeu de données : `data/legifacile-sample/`, 515 Mio, six lots et
  **257 626 passages** (205 261 documents) dont les vecteurs fp16 sont les
  vecteurs de production recopiés de pgvector. `queries.jsonl` porte 300 requêtes
  réelles, SHA-256
  `fdaa2794b027307cb9ac2e69c2dcb3a0f25ac5ff59b830b9c19ec359310391c6`. Les six
  `manifest.json` ont pour SHA-256 `d768a17c…`, `28c2e08a…`, `d25400c3…`,
  `6ea65a13…`, `f9a5785d…` et `50e03a34…` (préfixes, valeurs complètes dans les
  fichiers).
- Modèle : `data/models/mdenseon/`, révision
  `a5fdb000f7a21da96c3bddde3a782ef777316df3`, `model.json` SHA-256
  `0429f351f13008f172bb9711ea3f71bc82948e1f4b498bc16ab0a4c3ed44f2f3`.
  `model.onnx` fp32 (1,1 Gio, blake3 `7b6eb680…`), `model.int8.onnx` (293 Mio,
  blake3 `0534b4ee…`), `tokenizer.json` (blake3 `8a801317…`). Export sous
  transformers 5.14.1, torch 2.14.1 et onnxruntime 1.31.0 (ADR-0050).
- Configuration : prompt `query: `, 300 requêtes de 8 à 92 tokens (moyenne
  26,17), aucune troncature ; cache des requêtes **désactivé** (capacité 0) ;
  recherche exacte sur tout le pool de chaque structure ; aucune déduplication,
  car le chemin de recherche actuel n'en applique pas (`docs/03-recherche.md`
  décrit `dedup: "content"` comme une option, et `crible-search` ne
  l'implémente pas).

## 1. Protocole

Le harnais est l'exemple Rust
`crates/crible-encode/examples/measure_queries.rs`. Il appelle
`crible_encode::QueryEncoder::encode_query`, donc le prompt, le tokenizer, la
troncature et la normalisation sont ceux du moteur, pas une réplique. La
référence PyTorch est produite par `tools/bench/mes04/torch_reference.py`, avec
le même `tokenizer.json`, le même prompt, le pooling CLS et la normalisation L2
que `tools/export_onnx.py`.

```sh
# depuis tools/
uv sync --group export                                # déjà installé
uv run python bench/mes04/prepare.py                  # 300 textes, champs réduits

# depuis la racine
cargo build --release --example measure_queries -p crible-encode
export ORT_DYLIB_PATH=$(cd tools && uv run python -c \
  'import onnxruntime, pathlib; print(next((pathlib.Path(onnxruntime.__file__).parent / "capi").glob("libonnxruntime*.dylib")))')
./target/release/examples/measure_queries vectors \
    --model data/models/mdenseon --queries data/mes04/query-texts.jsonl \
    --out-dir data/mes04
(cd tools && uv run python bench/mes04/torch_reference.py)
(cd tools && uv run python bench/mes04/fidelity.py)   # distribution de Q-06
(cd tools && uv run python bench/mes04/analyze.py \
    --commit 2fd3c29b64a9f5ce02d83cbdd9df49a5ce6d540a --date 2026-10-24 \
    --machine "MacBook Air M3, 8 cores, 24 GB, macOS 15.7.7, native" \
    --fidelity data/mes04/fidelity.json \
    --out docs/mesures/MES-04-parite-encodeur.2026-10-24.json)
```

`bench/mes04/fidelity.py` rejoue la recette de `tools/export_onnx.py`
(`measure_fidelity`) : 100 passages par structure, prompt `document: `, graphe
fp32 contre les vecteurs fp16 du lot. Il sert à mesurer dans cette campagne la
distribution sur laquelle repose la proposition de Q-06, au lieu de la citer de
la campagne précédente.

- **Vecteurs PyTorch** : `transformers==5.14.1`, `torch==2.14.1` (CPU), modèle
  chargé depuis le cache Hugging Face en `local_files_only`. Les 300 requêtes
  passent une par une, sans troncature, en `torch.inference_mode()`.
- **Vecteurs Crible** : `QueryEncoder` avec une capacité de cache nulle, en INT8
  puis en fp32. Le harnais vérifie que les deux fichiers font 300 × 768 `f32`.
- **Cosinus** : calculés en `float64` entre les vecteurs normalisés, par requête.
  Les percentiles sont au **rang le plus proche** (`ceil(p / 100 × n)`) : sur 300
  valeurs, p1 est la 3e plus petite, sans interpolation.
- **Recherche exacte** : les vecteurs fp16 du lot sont convertis en `float32`
  comme le fait le moteur, puis notés par produit scalaire contre le vecteur de
  requête. Le top-10 est trié par score décroissant, puis par clé
  `doc_id#ord` croissante (l'ordre de `ExactSearcher` et de
  `tools/reference_topk.py`). Aucune déduplication.
- **Contrôle de la réplique numpy** : pour la requête `Q01`, le top-10 rendu par
  `crible search --query` sur l'index `articles` (attaché depuis le lot, INT8)
  est identique, dans le même ordre, au top-10 numpy des mêmes vecteurs.
- **Contrôle du chemin Rust** : sur les cinq premières requêtes, le cosinus entre
  le vecteur Rust et le vecteur `onnxruntime` Python vaut 0,99999997 à
  1,00000003, en INT8 comme en fp32. Un écart du harnais serait donc visible ;
  il ne l'est pas.

## 2. Résultats — cosinus contre PyTorch fp32

300 requêtes réelles, prompt `query: `, aucune troncature.

| Comparaison | Minimum | p1 | p5 | Médiane | Moyenne |
|---|---:|---:|---:|---:|---:|
| INT8 contre PyTorch fp32 | **0,920358** | 0,923824 | 0,935325 | 0,949328 | 0,948764 |
| fp32 ONNX contre PyTorch fp32 | **0,999999999989** | 0,999999999995 | 0,999999999997 | 0,999999999998 | 0,999999999998 |
| INT8 contre fp32 ONNX (moteur) | 0,920358 | 0,923825 | 0,935325 | 0,949328 | 0,948764 |

Le fp32 franchit le seuil de 0,9999 de la tâche `EXPORTER-MODELE-ONNX` avec une
marge de sept ordres de grandeur : la chaîne Crible (tokenizer, prompt, pooling,
normalisation) est fidèle au modèle PyTorch. L'INT8, lui, plafonne à 0,9668 sur
la meilleure requête et descend à 0,9204.

## 3. Résultats — rappel@10 de l'INT8 contre le fp32

Top-10 exact de chaque précision sur les passages de la structure de la requête.
« Parfait » compte les requêtes dont le top-10 INT8 est identique au top-10 fp32.

| Structure | Requêtes | Passages | Cosinus INT8 min | Cosinus INT8 p50 | Rappel@10 moyen | Parfait | Accord document@10 |
|---|---:|---:|---:|---:|---:|---:|---:|
| `articles` | 63 | 50 000 | 0,933518 | 0,954352 | 0,8032 | 6 | 0,8190 |
| `decisions` | 200 | 49 998 | 0,920358 | 0,947701 | 0,7795 | 20 | 0,7890 |
| `dole` | 8 | 7 628 | 0,934644 | 0,946545 | 0,8875 | 2 | 0,9250 |
| `jorf` | 12 | 50 000 | 0,937211 | 0,949688 | 0,7750 | 1 | 0,8083 |
| `sarde` | 9 | 50 000 | 0,936521 | 0,950902 | 0,8889 | 2 | 0,8889 |
| `titres` | 8 | 50 000 | 0,937042 | 0,951178 | 0,8375 | 0 | 0,8375 |
| **Global** | **300** | **257 626** | **0,920358** | **0,949328** | **0,7920** | **31** | **0,8040** |

Le rappel@10 moyen est de **0,792** : 20,8 % des passages du top-10 fp32
disparaissent du top-10 INT8. Aucune structure n'atteint 0,90. Dans cette
comparaison, le fp32 est sa propre référence, donc son rappel vaut 1,0 par
construction ; le cosinus 0,999999999989 ci-dessus mesure sa fidélité à PyTorch,
pas son rappel.

## 4. Résultats — attendus du jeu de requêtes

145 requêtes ont `expected_in_sample: true`. Le top-10 est comparé aux documents
attendus (`src_id`), au niveau document.

| Précision | Requêtes | `hit@10` | `recall@10` |
|---|---:|---:|---:|
| INT8 | 145 | 0,758621 (110 / 145) | 0,758621 |
| fp32 | 145 | 0,779310 (113 / 145) | 0,779310 |

L'écart est de trois requêtes sur 145 (2,1 points). Cette métrique binaire est
peu sensible : elle ne voit pas les 20,8 % de churn du top-10, parce que les
attendus restent souvent dans le voisinage. Elle ne compense pas le rappel@10
contre fp32, mais elle montre que la dégradation INT8 ne se traduit pas par une
perte équivalente sur ce jeu d'attendus.

## 5. Conclusion

Ce que la mesure permet d'affirmer :

1. **La chaîne fp32 du moteur est fidèle à PyTorch** : minimum
   0,999999999989 sur 300 requêtes réelles, médiane 0,999999999998. Le ré-export
   et le chemin Rust ne sont pas en cause.
2. **Le graphe INT8 dégrade fortement les vecteurs de requête** : cosinus moyen
   0,948764, minimum 0,920358, et **rappel@10 de 0,7920** contre le top-10 fp32.
   Le défaut est confirmé hors du harnais Rust : `onnxruntime` en Python donne
   0,948 à 0,963 sur les dix premières requêtes, et le cosinus Rust/Python vaut
   1,0. Ce n'est donc ni un bug de chargement, ni un écart de tokenizer.
3. **La règle de fidélité de 0,99 de `docs/08` est manquée** si on l'applique à
   la précision de requête : 0,7920 contre 0,99. Cette règle vise le scan
   binaire, pas l'encodage, mais elle donne l'ordre de grandeur : l'INT8 des
   requêtes n'est pas une approximation acceptable.
4. **L'écart d'attendus est faible mais non nul, au détriment de l'INT8** :
   110 / 145 en INT8 contre 113 / 145 en fp32. Sur 145 requêtes, trois requêtes
   d'écart restent dans le bruit ; le rappel@10 de 0,7920 est le signal robuste.

**Recommandation, à valider par le propriétaire** : passer
`[encoder].query_precision` à `"fp32"` pour les requêtes, c'est-à-dire
abandonner le défaut INT8 de `docs/04` et d'ADR-0051. Conséquences à trancher
dans un ADR :

- la session fp32 pèse plus d'1 Go et devrait être chargée dès la première
  requête, ce qui contredit le chargement paresseux d'ADR-0051 et le budget de
  `docs/02` ;
- la latence fp32 est mesurée dans MES-05 (p50 21 ms, p95 56 ms sur le Mac M3) ;
- une alternative non mesurée est une **requantification** du graphe (statique,
  par canal, ou avec un jeu de calibration de requêtes) suivie d'une nouvelle
  MES-04. Aucun réglage de quantification n'a été essayé ici : la campagne
  mesure le graphe livré, elle ne l'améliore pas.

### Valeur proposée pour Q-06 (`acceptance_min_cosine`)

**Proposer 0,999**, la valeur déjà présente dans les configs provisoires.

La distribution mesurée qui l'appuie est celle de la recette de fidélité de
`REEXPORTER-MODELE-TRANSFORMERS-5`, **rejouée dans cette campagne** par
`tools/bench/mes04/fidelity.py` (même révision, même graphe fp32 blake3
`7b6eb680…`) : 600 passages, 100 par structure, prompt `document: `, ONNX fp32
contre les vecteurs fp16 de production — **minimum 0,9999604, p1 0,9999935,
médiane 0,9999983**. Ces trois valeurs sont identiques à celles de
`data/models/mdenseon/parity.json` (campagne du 2026-10-23) au chiffre près : la
recette est reproductible. Le bloc `document_fidelity` du JSON brut porte cette
distribution. L'écart au pire cas est `1 − 0,9999604 = 3,96e-5`, soit 25 fois la
tolérance `1 − 0,999 = 1e-3`. Le seuil refuse encore les vecteurs du mauvais
export (0,983 à 0,990, ADR-0050) et un lot encodé sans le prompt.

Ce que cette valeur **couvre** : l'arrondi fp16 des vecteurs de production tels
qu'ils sont stockés et relus, et la chaîne documentaire complète (gabarit,
tokenizer, révision, export) sur les six structures de l'échantillon.

Ce que la mesure **ne couvre pas** :

- **aucun encodage MPS ou CUDA n'a été exécuté ici** ; la machine n'a pas de GPU
  exploitable par cette campagne. La provenance des vecteurs de l'échantillon
  (pgvector `halfvec`, produit par le pipeline Legifacile en natif sur MPS et en
  campagne sur CUDA) est documentée, pas réexécutée ;
- aucun graphe ONNX fp16 n'est mesuré : la référence est fp32 CPU contre des
  vecteurs fp16 déjà produits ;
- aucun autre GPU, aucune autre révision de transformers, aucun document de
  plus de 8 192 tokens ;
- la distribution ne dit rien d'un lot dont les vecteurs viendraient d'un
  matériel ou d'une bibliothèque différents : 0,999 est un défaut
  **conservateur**, pas une borne prouvée pour tout GPU.

Un seuil de 0,9995 passerait encore le minimum observé (marge 4,6e-4), mais la
mesure ne justifie pas de le resserrer : elle ne contient aucun cas MPS ou CUDA
contrôlé qui permettrait de borner le pire cas.

## 6. Données brutes

`docs/mesures/MES-04-parite-encodeur.2026-10-24.json` : les cosinus par requête
(INT8 et fp32), les rappels@10 par requête et par structure, les attendus, les
longueurs de tokens, l'identité du modèle, le SHA-256 de `queries.jsonl` et le
bloc `document_fidelity` (600 passages) qui appuie Q-06. Les fichiers de travail
(vecteurs `.f32`, textes de requête, `analysis.json`, `fidelity.json`) restent
dans `data/mes04/` (gitignoré). Aucun texte de requête n'est présent dans le
JSON versionné : seulement des identifiants opaques (`Q01`, `J008`…) et des
agrégats.

## 7. Constats hors mesure

- `crible search --query` encode toujours en INT8 et n'expose aucun réglage de
  précision ; le CLI n'a pas encore de chargeur `crible.toml` (ADR-0051). La
  mesure fp32 passe donc par l'exemple, pas par la commande.
- Le jeu de requêtes est déséquilibré : 200 requêtes sur `decisions` sur 300.
  La moyenne globale est donc dominée par `decisions` ; le tableau par structure
  donne les cinq autres.
- L'index de chaque structure a été attaché pour vérifier la réplique numpy et
  pour la démo J2, sous `data/indexes/` ; aucun index de l'échantillon n'existait
  avant la campagne. Commandes, une fois par structure (`articles`, `decisions`,
  `dole`, `jorf`, `sarde`, `titres`) :

  ```sh
  ./target/release/crible index create --data data \
      --config data/legifacile-sample/configs/<structure>.json
  ./target/release/crible attach --data data --models data/models \
      --index <structure> \
      data/legifacile-sample/batches/legifacile-sample-<structure>-v1
  ```

  L'attache a accepté les six lots (recette fp32 comprise).
- La déduplication par contenu n'existe pas dans le chemin de recherche ; elle
  n'a donc pas été appliquée. Si elle est ajoutée, MES-04 devra être rejouée :
  elle changerait les top-10 comparés.
