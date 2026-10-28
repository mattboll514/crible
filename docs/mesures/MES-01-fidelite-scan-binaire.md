# MES-01 — Rappel du scan binaire et du rescoring contre l'exact, selon N

## Campagne du 2026-10-27

- Tâche : `MESURER-FIDELITE-BINAIRE` (phase P4, fiche
  `docs/phases/P4-recherche-rapide.md`). Mesure demandée avant le choix du `N`
  par défaut.
- Références : `docs/08-invariants-et-tests.md` (MES-01, cible de fidélité de
  0,99), **ADR-0076** (définition des métriques, protocole, forçage de la
  stratégie, jeux de requêtes), ADR-0074 (planification et `exact_threshold`),
  ADR-0004 (`codes.bin`, bit `x_i > 0`), ADR-0052 (requêtes fp32), ADR-0072
  (produit scalaire partagé entre EXACT et le rescoring), Q-01.
- Commit : `9ca3b0782637ca3ad38fe4112197ea1e8ece3f87`, branche
  `tache/MESURER-FIDELITE-BINAIRE`. **Aucun code de production n'a été modifié
  pendant la campagne.** L'exemple `crates/crible/examples/fidelity.rs`, les
  scripts `tools/bench/mes01/` et leur entrée pytest dans `tools/pyproject.toml`
  sont **neufs et non commités au moment de la mesure** : la campagne porte donc
  sur `9ca3b07` plus ce harnais, sans changement dans `crates/*/src/`.
- Index de mesure : six index neufs sous `data/mes01/indexes/`, construits par
  `crible index create` puis `crible attach` depuis les lots de l'échantillon,
  avec `search.exact_threshold = 0`. `search.rescore_candidates` vaut `1000` à
  la création, puis la campagne le réécrit à chaque `N` mesuré (ADR-0076,
  décision 4). Les lots et les index de l'échantillon n'ont pas été touchés.
  Pools mesurés : `articles` 50 000, `decisions` 49 998, `dole` 7 628, `jorf`
  50 000, `sarde` 50 000, `titres` 50 000 passages ; aucun passage mort.
- Machine : MacBook Air Apple M3 (`Mac15,12`, 8 cœurs : 4 performance,
  4 efficacité), 24 Go de RAM (25 769 803 776 octets), macOS 15.7.7 (24G720),
  APFS. Mesure **native**, binaire et exemple compilés en `--release`. La
  machine n'était pas au repos (moyenne de charge 2,73 à 3,30 sur 8 cœurs), mais
  cette mesure est **déterministe** : deux exécutions identiques rendent le même
  fichier, octet pour octet (contrôle au § 1). `ScanPool::new(0)`, donc un
  worker par cœur disponible, la convention de la CLI.
- Jeu de données : `data/legifacile-sample/`, 515 Mio, six lots et
  **257 626 passages** (205 261 documents), vecteurs `fp16` de production
  recopiés de pgvector. `queries.jsonl` porte 300 requêtes, SHA-256
  `fdaa2794b027307cb9ac2e69c2dcb3a0f25ac5ff59b830b9c19ec359310391c6`. Les six
  `manifest.json` ont pour SHA-256 `d768a17c…`, `28c2e08a…`, `d25400c3…`,
  `6ea65a13…`, `f9a5785d…` et `50e03a34…` (préfixes ; valeurs complètes dans les
  fichiers).
- Modèle : `data/models/mdenseon/`, révision
  `a5fdb000f7a21da96c3bddde3a782ef777316df3`, graphe fp32 blake3
  `7b6eb680…`. Les 300 requêtes sont encodées **en fp32** par
  `QueryEncoder` (prompt `query: `, ADR-0052), une fois, dans
  `data/mes01/vectors-fp32.f32` (300 × 768 `f32`, SHA-256 `6e13f8003dc6ba45…`).
  Les deux stratégies relisent donc le même vecteur : la mesure ne compare que
  le scan et le rescoring.

## 1. Protocole

Définitions (ADR-0076, décision 1), appliquées telles quelles par
`tools/bench/mes01/mes01_metrics.py` :

- **recall@10** = moyenne sur les requêtes de |top-10 BINARY ∩ top-10 EXACT| / 10,
  au niveau du **passage**, la clé étant `doc_id#ord` ;
- **recouvrement@10** = même intersection après projection des deux top-10 sur
  `doc_id` dédoublonnés, divisée par le nombre de `doc_id` **distincts** du
  top-10 EXACT.

La référence EXACT est le **même pipeline** en `exact: true`, sur le même pool,
avec le même vecteur de requête. `k = 10` : sous BINARY le scan garde
`max(N, k)` candidats et les rescore tous, donc le top-10 avec `k = 10` est
exactement le top-10 de la branche réglée sur `candidates_per_branch = 100`, que
la CLI demande. Contrôle fait sur `Q01`, index `articles`, `N = 2000` : la
commande livrée `crible search --vector-file … --k 10` et l'exemple rendent les
mêmes clés **et** les mêmes scores, dans le même ordre (pool 50 000). Ce
contrôle et celui du déterminisme sont rejoués par
`tools/bench/mes01/mes01_controls.py` (étape 4 bis ci-dessous) ; leurs résultats,
y compris les deux SHA-256 identiques du contrôle de déterminisme, sont dans le
bloc `controls` des deux artefacts datés.

Commandes exactes, depuis la racine du dépôt :

```sh
export ORT_DYLIB_PATH=$(cd tools && uv run python -c \
  'import onnxruntime, pathlib; print(next((pathlib.Path(onnxruntime.__file__).parent / "capi").glob("libonnxruntime*.dylib")))')

# 1. listes de requêtes (texte, campagne principale, campagne filtrée)
(cd tools && uv run --no-sync python bench/mes01/mes01_prepare.py)

# 2. vecteurs de requête fp32, par l'encodeur du moteur
cargo build --release
cargo build --release --example measure_queries -p crible-encode
cargo build --release --example fidelity -p crible
./target/release/examples/measure_queries vectors \
    --model data/models/mdenseon --queries data/mes01/query-texts.jsonl \
    --out-dir data/mes01

# 3. index de mesure (exact_threshold = 0, rescore_candidates = 1000 à la création)
(cd tools && uv run --no-sync python bench/mes01/mes01_build_indexes.py)

# 4. moteur : EXACT puis BINARY pour N ∈ {200, 500, 1000, 2000, 5000}
(cd tools && uv run --no-sync python bench/mes01/mes01_campaign.py)

# 4 bis. déterminisme de l'exemple et équivalence avec `crible search`
(cd tools && uv run --no-sync python bench/mes01/mes01_controls.py)

# 5. agrégats et artefacts datés
(cd tools && uv run --no-sync python bench/mes01/mes01_analyze.py \
    --commit 9ca3b0782637ca3ad38fe4112197ea1e8ece3f87 --date 2026-10-27 \
    --machine "MacBook Air M3 (Mac15,12), 8 cores, 24 GB, macOS 15.7.7 (24G720), native" \
    --mesures ../docs/mesures)
```

- **Préchauffage** : sans objet, la mesure n'est pas une latence. Ce qui est
  chauffé, c'est le cache de pages : la première passe lit 76 Mo de vecteurs et
  4,8 Mo de codes par index, les suivantes les trouvent en cache. Le chiffre ne
  dépend pas de cet état, puisque la recherche est déterministe.
- **Répétitions** : une passe complète par `N` et par structure, soit
  36 exécutions pour la campagne principale et 12 pour la filtrée. Le
  déterminisme a été contrôlé en rejouant `articles`, `N = 2000` : fichier de
  sortie identique (`a == b` sur le JSON complet).
- **Contrôle de la stratégie (ADR-0076, décision 4)** : chaque exécution BINARY
  est refusée si le plan d'une seule requête ne rend pas
  `dense_strategy = "binary"` et `rescore_candidates = max(N, k)`. Résultat :
  **2 815 exécutions-requête BINARY** contrôlées, toutes en `"binary"` avec le
  `N` demandé ; **563 exécutions-requête EXACT**, toutes en `"exact"`. Aucune
  valeur n'a été écartée. Ces deux comptes sont ceux du bloc `strategy_control`
  de l'artefact daté (1 500 + 1 315 et 300 + 263).
- **Cache froid / chaud** : les index viennent d'être construits ; la première
  ouverture en écriture de chaque index matérialise ou réécrit les `codes.bin`
  (ADR-0069, décision 1), les recherches suivantes les relisent. Les tailles
  observées sont exactement `count × 12 × 8` octets (4 800 000 pour 50 000
  passages, 732 288 pour `dole`).
- **Pas d'arrondi caché** : les moyennes sont des moyennes arithmétiques sur
  toutes les requêtes de la ligne ; les valeurs par requête sont dans le JSON
  brut. Les intervalles à 95 % du § 4 sont des intervalles de moyenne
  (`1,96 × écart-type / √n`), donnés pour situer le point estimé par rapport à
  la cible, jamais pour la remplacer.

## 2. Résultats — campagne principale, sans filtre

300 requêtes, 257 626 passages au total. Campagne principale d'ADR-0076,
décision 5 : pool maximal, donc pire cas de l'échantillon.

**recall@10 (passage), par structure et `N`**

| Structure | Pool | Requêtes | N=200 | N=500 | N=1000 | N=2000 | N=5000 |
|---|---:|---:|---:|---:|---:|---:|---:|
| `articles` | 50 000 | 63 | 0,9111 | 0,9714 | 0,9825 | 1,0000 | 1,0000 |
| `decisions` | 49 998 | 200 | 0,8810 | 0,9410 | 0,9705 | 0,9835 | 0,9965 |
| `dole` | 7 628 | 8 | 1,0000 | 1,0000 | 1,0000 | 1,0000 | 1,0000 |
| `jorf` | 50 000 | 12 | 0,9417 | 0,9667 | 0,9750 | 0,9917 | 1,0000 |
| `sarde` | 50 000 | 9 | 0,9667 | 0,9889 | 1,0000 | 1,0000 | 1,0000 |
| `titres` | 50 000 | 8 | 0,8750 | 0,9875 | 1,0000 | 1,0000 | 1,0000 |
| **Global** | **257 626** | **300** | **0,8953** | **0,9527** | **0,9757** | **0,9887** | **0,9977** |

**recouvrement@10 (document), par structure et `N`** — rapporté sans cible.

| Structure | N=200 | N=500 | N=1000 | N=2000 | N=5000 |
|---|---:|---:|---:|---:|---:|
| `articles` | 0,9158 | 0,9757 | 0,9855 | 1,0000 | 1,0000 |
| `decisions` | 0,8834 | 0,9415 | 0,9709 | 0,9845 | 0,9975 |
| `dole` | 1,0000 | 1,0000 | 1,0000 | 1,0000 | 1,0000 |
| `jorf` | 0,9731 | 0,9917 | 0,9917 | 1,0000 | 1,0000 |
| `sarde` | 0,9667 | 0,9889 | 1,0000 | 1,0000 | 1,0000 |
| `titres` | 0,8750 | 0,9875 | 1,0000 | 1,0000 | 1,0000 |
| **Global** | **0,8992** | **0,9549** | **0,9772** | **0,9897** | **0,9983** |

Requêtes parfaites (recall@10 = 1,0) sur 300 : 145, 209, 241, 274, 296 pour
N = 200, 500, 1000, 2000, 5000. Le minimum par requête vaut 0,3, 0,6, 0,7, 0,7
puis 0,8.

## 3. Résultats — variante filtrée synthétique

Filtres déclarés comme **synthétiques** (ADR-0076, décision 5), pris dans les
champs de l'échantillon : `{"etat": "VIGUEUR"}` pour `articles` (17 494 passages
retenus) et `{"partie": "motifs"}` pour `decisions` (14 638 passages retenus).
263 requêtes (63 `articles`, 200 `decisions`) ; les 37 requêtes des quatre
autres structures ne portent pas de champ filtrable déclaré et ne participent
pas à cette campagne. Le jeu de référence n'est pas modifié.

**recall@10 (passage)**

| Structure | Pool | Requêtes | N=200 | N=500 | N=1000 | N=2000 | N=5000 |
|---|---:|---:|---:|---:|---:|---:|---:|
| `articles` | 17 494 | 63 | 0,8905 | 0,9667 | 0,9873 | 0,9937 | 1,0000 |
| `decisions` | 14 638 | 200 | 0,8520 | 0,9375 | 0,9725 | 0,9920 | 0,9985 |
| **Global** | **32 132** | **263** | **0,8612** | **0,9445** | **0,9760** | **0,9924** | **0,9989** |

**recouvrement@10 (document)**

| Structure | N=200 | N=500 | N=1000 | N=2000 | N=5000 |
|---|---:|---:|---:|---:|---:|
| `articles` | 0,9020 | 0,9753 | 0,9887 | 0,9951 | 1,0000 |
| `decisions` | 0,8577 | 0,9385 | 0,9730 | 0,9924 | 0,9985 |
| **Global** | **0,8683** | **0,9473** | **0,9768** | **0,9931** | **0,9989** |

Requêtes parfaites sur 263 : 96, 167, 214, 244, 260. Minimum par requête :
0,1, 0,6, 0,6, 0,8 puis 0,9.

## 4. Conclusion

Ce que la mesure permet d'affirmer :

1. **La cible de 0,99 est atteinte à N = 5000, dans les deux campagnes et dans
   toutes les structures.** recall@10 global = **0,9977** (campagne principale)
   et **0,9989** (filtrée) ; la borne basse de l'intervalle à 95 % de la moyenne
   vaut **0,9953** et **0,9976**, au-dessus de 0,99 (la borne haute, 1,0001 par
   l'approximation normale, est plafonnée à 1). La campagne **filtrée** atteint
   aussi la cible dès **N = 2000** (0,9924, intervalle [0,9890 ; 0,9958] : le
   point estimé l'atteint, l'intervalle ne le garantit pas), mais la campagne
   principale non (0,9887). Aucun `N = 1000` ne la garantit : le meilleur chiffre
   à ce `N` est 0,9913 (variante `(c)` de MES-12, campagne filtrée), d'intervalle
   [0,9870 ; 0,9955] ; sur la campagne principale, la même variante donne 0,9903,
   d'intervalle [0,9864 ; 0,9943]. Les deux chevauchent 0,99.
2. **N = 2000 manque la cible sur la campagne principale** : 0,9887, intervalle
   [0,9840 ; 0,9933] qui chevauche 0,99. La structure `decisions` tombe à
   **0,9835**, sous la cible, et c'est elle qui pèse (200 requêtes sur 300). La
   campagne filtrée passe à 0,9924 avec ce même N, mais une seule campagne ne
   suffit pas à retenir la valeur.
3. **N = 1000 et en dessous sont sous la cible en moyenne globale** : 0,9757,
   0,9527 et 0,8953. À N = 1000, trois structures restent sous 0,99 : `articles`
   0,9825, `decisions` 0,9705 et `jorf` 0,9750. `sarde` et `titres` valent 1,0000,
   mais ne portent que 9 et 8 requêtes : une moyenne parfaite sur huit requêtes
   ne prouve rien de la structure.
4. **Aucune structure n'est sous 0,99 au N recommandé**, dans les deux
   campagnes : la plus faible est `decisions` à 0,9965 (principale) et 0,9985
   (filtrée). La règle de signalement d'ADR-0076 n'a donc rien à expliquer au N
   retenu ; elle s'applique en revanche à N = 2000 (0,9835).
5. **Le résidu à N = 5000 est un effet de rang de Hamming, pas un bruit.**
   Quatre requêtes de la campagne principale ne sont pas parfaites (Q33, J031,
   J040, J136, toutes `decisions`, recall 0,8 à 0,9). Le harnais a calculé le
   rang de Hamming des dix passages du top-10 exact de chacune : un ou deux de
   ces passages ont un rang supérieur à 5000 (5 554 pour Q33 ; 5 050 et 9 561
   pour J031 ; 7 731 et 15 996 pour J040 ; 5 074 et 5 643 pour J136 ; rang 0 =
   passage le plus proche). Le scan ne peut pas les garder, et aucun rescoring
   ne les récupère. La perte restante vient donc du classement binaire lui-même.
   Ces rangs sont dans le bloc `residuals` de l'artefact daté, requête par
   requête, sans `doc_id`. Le bloc `residuals.filtered` est vide : les rangs de
   la campagne filtrée ne sont dans aucun artefact commité, et la campagne n'a
   pas été rejouée ; les trois passages manquants à `N = 5000` y restent donc
   sans rang publié.
6. **Le coût de N = 5000 se lit ailleurs.** Cette mesure ne dit rien de la
   latence : 5 000 candidats rescored par requête contre 1 000 est un facteur 5
   sur l'étape de rescoring, que MES-02 et MES-03 doivent chiffrer avant que le
   défaut soit figé. Le rapport ne tranche pas ce compromis.

**Recommandation, à valider par le propriétaire** : fixer
`search.rescore_candidates` à **5000** comme `N` par défaut, sans centrage des
codes (voir MES-12). C'est la plus petite valeur mesurée qui atteint la cible
dans **les deux campagnes** et dans chaque structure ; la filtrée l'atteint dès
2000, mais la plus grande et la plus lourde (200 des 300 requêtes) non. La marge
est de +0,0077 sur la campagne principale. Si MES-02 ou MES-03 montrent que
5 000 candidats coûtent trop cher à l'échelle de Legifacile, la variante à
examiner est le scoring asymétrique (centrage des documents sans centrage de la
requête, MES-12 § 4), qui atteint 0,9970 à N = 2000 : cette piste demande un ADR
et n'est pas codée.

Limites à retenir :

- **les pools mesurés font au plus 50 000 passages**, contre **8,58 M** en
  production. À N = 5000, on rescore 10 % du plus grand pool mesuré. Rien ici ne
  prouve que 5 000 candidats suffisent à 8,58 M : la distribution des rangs de
  Hamming y est inconnue, et la confirmation est renvoyée à **P9**
  (`legifacile-full`). Aucune extrapolation n'est faite ;
- 200 des 300 requêtes portent sur `decisions` : la moyenne globale est dominée
  par cette structure, et c'est pourquoi le tableau par structure est donné ;
- le jeu de requêtes n'est pas un journal d'utilisateurs (`README` de
  l'échantillon, « Limites ») ; il mesure la recherche de sujets, pas la
  compréhension d'une phrase ;
- `synth-large` n'a pas été utilisé, comme le veut ADR-0076 ;
- la variante filtrée est synthétique : elle rapproche la mesure de la
  production (ADR-0004), elle ne la représente pas.

## 5. Données brutes

`docs/mesures/MES-01-fidelite-scan-binaire.2026-10-27.json` : les tables
ci-dessus, les 300 lignes par requête (recall et recouvrement pour chaque `N`),
les **48 exécutions** (36 principale, 12 filtrée) avec leur stratégie et leurs
tailles de pool, le bloc `strategy_control` qui compte les **2 815 plans BINARY
et 563 plans EXACT** acceptés par les contrôles, le bloc `controls`
(déterminisme, équivalence CLI), le bloc `residuals` (rangs de Hamming des
passages manquants), le SHA-256 de `queries.jsonl` et la commande de rejeu.
Aucun texte de requête, aucun `doc_id`, aucun `iid` n'y figure : seulement des
identifiants opaques (`Q01`, `J031`…) et des agrégats. Les fichiers de travail
(matrice de vecteurs, sorties du moteur par exécution, `centering.json`,
`analysis.json`) restent dans `data/mes01/` (gitignoré).

## 6. Constats hors mesure

- `crible search` ne publie pas le plan dense : `docs/05-api.md` le réserve à la
  sortie de debug de P7, et le commentaire de `planner.rs` le dit. La
  vérification `dense_strategy = "binary"` demandée par ADR-0076 passe donc par
  l'exemple `crates/crible/examples/fidelity.rs`, qui appelle
  `crible_search::dense::plan` et écrit le plan à côté des résultats. Le jour où
  la CLI exposera le plan, l'exemple pourra le relire de la réponse.
- Le rescoring est le même pour les deux stratégies (ADR-0072) : les scores
  EXACT et BINARY d'un même passage sont donc comparables au bit près. C'est ce
  qui rend la comparaison de top-10 légitime sans marge de tolérance.
- `crible search` ouvre l'index en écriture : cette ouverture matérialise ou
  réécrit les `codes.bin` de chaque segment (ADR-0069, décision 1). Les index de
  mesure portent donc ces fichiers, et le contrôle au bit près de MES-12 les
  compare aux codes recalculés depuis les vecteurs `fp16`.
