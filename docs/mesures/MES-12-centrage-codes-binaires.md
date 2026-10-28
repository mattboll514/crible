# MES-12 — Codes binaires : signe brut contre signe après centrage

## Campagne du 2026-10-27

- Tâche : `MESURER-FIDELITE-BINAIRE` (phase P4, fiche
  `docs/phases/P4-recherche-rapide.md`). Même campagne que MES-01 : mêmes
  index, mêmes requêtes, même commit.
- Références : `docs/08-invariants-et-tests.md` (MES-12), `docs/03-recherche.md`
  (« Branche dense », le centrage « à étudier (MES-12) »), **ADR-0076**
  (protocole de centrage, décision 3), ADR-0004 (bit `x_i > 0`), ADR-0069
  décision 6 (format de `codes.bin`), Q-01.
- Commit : `9ca3b0782637ca3ad38fe4112197ea1e8ece3f87`, branche
  `tache/MESURER-FIDELITE-BINAIRE`. **Aucun code de production n'a été modifié
  et aucun format n'a changé** : le centrage est mesuré hors moteur, par
  `tools/bench/mes01/mes01_centering.py`, qui recalcule les codes en RAM depuis les
  vecteurs `fp16`. Ce script, l'exemple `crates/crible/examples/fidelity.rs` et
  les autres scripts `tools/bench/mes01/` sont **neufs et non commités au moment
  de la mesure** (même remarque que MES-01).
- Machine : MacBook Air Apple M3 (`Mac15,12`, 8 cœurs), 24 Go de RAM,
  macOS 15.7.7 (24G720), APFS, mesure native.
- Jeu de données : `data/legifacile-sample/`, 257 626 passages, six index de
  mesure sous `data/mes01/indexes/` (cf. MES-01, campagne du 2026-10-27).
  `queries.jsonl` SHA-256
  `fdaa2794b027307cb9ac2e69c2dcb3a0f25ac5ff59b830b9c19ec359310391c6`.
- Configuration : `N ∈ {200, 500, 1000, 2000, 5000}`, `k = 10`, vecteurs de
  requête fp32, `search.exact_threshold = 0` ; campagne principale (300
  requêtes, sans filtre) et variante filtrée synthétique (`etat = VIGUEUR` pour
  `articles`, `partie = motifs` pour `decisions`, 263 requêtes).

## 1. Protocole

Trois variantes de codes, sur les mêmes requêtes et la même référence EXACT que
MES-01 (ADR-0076, décision 3) :

- **`raw`** — signe brut, bit `i` vaut 1 quand `x_i > 0`. Ce n'est pas une
  variante de la mesure : c'est le **contrôle du harnais** ;
- **`a`** — la moyenne de **tous les passages vivants de l'index** est
  soustraite aux vecteurs des documents **et** au vecteur de requête, puis le
  signe est pris, **sans renormalisation**. Variante principale ;
- **`c`** — documents centrés, requête non centrée. Rapportée à titre
  indicatif.

Ce que le centrage change : **les codes seulement**. Le rescoring garde les
vecteurs `fp16` stockés et la requête brute, exactement comme le moteur, et
classe par cosinus décroissant puis clé `doc_id#ord` croissante (ADR-0072). La
comparaison avec EXACT reste donc celle du scan : une variante gagne si ses
codes amènent dans les `N` candidats les passages que l'exact place dans son
top-10.

La moyenne est calculée en `float64` sur les 768 colonnes de chaque index ; sa
norme, notée ci-dessous, dit l'ampleur du décalage : 0,7642 à 0,8271 selon la
structure, pour des vecteurs de norme 1. Ce n'est donc pas une correction
marginale des composantes proches de zéro, mais un déplacement de tout le
nuage.

**Contrôle préalable obligatoire**, exécuté avant tout chiffre de centrage :

1. **Codes identiques au bit près.** Le harnais reprend les `vectors.f16` de
   chaque segment, en extrait le signe, et compare les mots `u64` au `codes.bin`
   écrit par le moteur : **4 291 488 mots** comparés, soit **274 655 232 bits**,
   sur les huit blocs structure × campagne. Écart : **0 mot**.
2. **Top-10 BINARY identique au moteur.** Pour chaque requête et chaque `N`, le
   harnais rejoue le scan et le rescoring et compare la liste ordonnée des clés
   à celle de l'exécution BINARY du moteur : **2 815 comparaisons**
   (563 requêtes × 5 valeurs de `N`), comptées par structure dans
   `control_comparisons` (1 500 sur la campagne principale, 1 315 sur la
   filtrée). Écart : **0 requête**, dans les deux campagnes. Le harnais
   **s'arrête au premier écart** et n'écrit aucun chiffre de centrage ; l'analyse
   refuse elle aussi d'écrire un artefact si un contrôle est en échec
   (ADR-0076, décision 3).
3. **Rappels identiques.** La variante `raw` du harnais rend exactement les
   rappels de MES-01 (0,8953 / 0,9527 / 0,9757 / 0,9887 / 0,9977 sur la
   campagne principale) : le calcul des métriques ne dépend pas du chemin qui a
   produit le top-10.

Les trois contrôles passent : les chiffres du § 3 sont donc retenus.

Commandes :

```sh
# depuis la racine, après mes01_build_indexes.py, mes01_campaign.py et
# mes01_controls.py de MES-01
(cd tools && uv run --no-sync python bench/mes01/mes01_centering.py)
(cd tools && uv run --no-sync python bench/mes01/mes01_analyze.py \
    --commit 9ca3b0782637ca3ad38fe4112197ea1e8ece3f87 --date 2026-10-27 \
    --machine "MacBook Air M3 (Mac15,12), 8 cores, 24 GB, macOS 15.7.7 (24G720), native" \
    --mesures ../docs/mesures)
(cd tools && uv run --no-sync pytest bench/mes01/tests)   # 30 tests
```

Le classement du harnais reproduit trois choix du moteur, testés par pytest :
distance de Hamming croissante puis `iid` croissant pour les `N` candidats
(`binary.rs`) ; cosinus en `float64` arrondi une fois en `f32`, puis clé
textuelle croissante (`exact.rs`, ADR-0072) ; normalisation de la requête en
`f32` terme à terme (`dense::normalise`, recalculée à l'identique).

## 2. Rappel de la référence

Les rappels de MES-01, rappelés pour lire les écarts (campagne principale,
300 requêtes) :

| Variante | N=200 | N=500 | N=1000 | N=2000 | N=5000 |
|---|---:|---:|---:|---:|---:|
| `raw` (moteur et harnais) | 0,8953 | 0,9527 | 0,9757 | 0,9887 | 0,9977 |

## 3. Résultats

### 3.1 recall@10 global, par variante et par `N`

**Campagne principale (300 requêtes)**

| Variante | N=200 | N=500 | N=1000 | N=2000 | N=5000 |
|---|---:|---:|---:|---:|---:|
| `raw` | 0,8953 | 0,9527 | 0,9757 | 0,9887 | 0,9977 |
| `a` (docs + requête centrés) | 0,8940 | 0,9553 | 0,9810 | 0,9953 | 0,9987 |
| `c` (docs centrés seuls) | 0,9300 | 0,9743 | 0,9903 | 0,9970 | 0,9983 |

**Variante filtrée synthétique (263 requêtes)**

| Variante | N=200 | N=500 | N=1000 | N=2000 | N=5000 |
|---|---:|---:|---:|---:|---:|
| `raw` | 0,8612 | 0,9445 | 0,9760 | 0,9924 | 0,9989 |
| `a` | 0,8536 | 0,9422 | 0,9776 | 0,9924 | 0,9985 |
| `c` | 0,9186 | 0,9776 | 0,9913 | 0,9973 | 1,0000 |

Requêtes parfaites sur 300 (campagne principale) : `raw` 145 / 209 / 241 / 274 /
296, `a` 157 / 217 / 257 / 287 / 296, `c` 174 / 244 / 276 / 292 / 295.

### 3.2 recouvrement@10 global, par variante et par `N`

| Variante | N=200 | N=500 | N=1000 | N=2000 | N=5000 |
|---|---:|---:|---:|---:|---:|
| `raw` (principale) | 0,8992 | 0,9549 | 0,9772 | 0,9897 | 0,9983 |
| `a` (principale) | 0,8961 | 0,9563 | 0,9810 | 0,9951 | 0,9986 |
| `c` (principale) | 0,9333 | 0,9764 | 0,9916 | 0,9980 | 0,9990 |
| `raw` (filtrée) | 0,8683 | 0,9473 | 0,9768 | 0,9931 | 0,9989 |
| `a` (filtrée) | 0,8577 | 0,9452 | 0,9790 | 0,9935 | 0,9989 |
| `c` (filtrée) | 0,9246 | 0,9789 | 0,9923 | 0,9984 | 1,0000 |

### 3.3 recall@10 par structure, variantes `a` et `c`

Le signe brut `raw` est rappelé au `N` retenu (5000) pour situer chaque
structure ; ses autres valeurs sont dans MES-01, § 2.

**Campagne principale**

| Structure | Pool | `raw` N=5000 | `a` N=200 | `a` N=500 | `a` N=1000 | `a` N=2000 | `a` N=5000 | `c` N=200 | `c` N=500 | `c` N=1000 | `c` N=2000 | `c` N=5000 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| `articles` | 50 000 | 1,0000 | 0,9048 | 0,9683 | 0,9889 | 1,0000 | 1,0000 | 0,9397 | 0,9841 | 0,9952 | 0,9984 | 0,9984 |
| `decisions` | 49 998 | 0,9965 | 0,8820 | 0,9450 | 0,9755 | 0,9930 | 0,9980 | 0,9175 | 0,9675 | 0,9870 | 0,9960 | 0,9980 |
| `dole` | 7 628 | 1,0000 | 0,9750 | 1,0000 | 1,0000 | 1,0000 | 1,0000 | 1,0000 | 1,0000 | 1,0000 | 1,0000 | 1,0000 |
| `jorf` | 50 000 | 1,0000 | 0,9250 | 0,9833 | 1,0000 | 1,0000 | 1,0000 | 0,9833 | 0,9917 | 1,0000 | 1,0000 | 1,0000 |
| `sarde` | 50 000 | 1,0000 | 0,9778 | 1,0000 | 1,0000 | 1,0000 | 1,0000 | 0,9778 | 0,9889 | 1,0000 | 1,0000 | 1,0000 |
| `titres` | 50 000 | 1,0000 | 0,8875 | 0,9750 | 0,9875 | 1,0000 | 1,0000 | 0,9625 | 1,0000 | 1,0000 | 1,0000 | 1,0000 |

**Variante filtrée synthétique**

| Structure | Pool | `raw` N=5000 | `a` N=200 | `a` N=500 | `a` N=1000 | `a` N=2000 | `a` N=5000 | `c` N=200 | `c` N=500 | `c` N=1000 | `c` N=2000 | `c` N=5000 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| `articles` | 17 494 | 1,0000 | 0,9000 | 0,9683 | 0,9921 | 0,9968 | 1,0000 | 0,9540 | 0,9937 | 0,9968 | 0,9968 | 1,0000 |
| `decisions` | 14 638 | 0,9985 | 0,8390 | 0,9340 | 0,9730 | 0,9910 | 0,9980 | 0,9075 | 0,9725 | 0,9895 | 0,9975 | 1,0000 |

## 4. Conclusion

Ce que la mesure permet d'affirmer :

1. **Le centrage principal `(a)` améliore le rappel, mais peu au N qui compte.**
   À N = 5000, il gagne **+0,0010** sur la campagne principale (0,9987 contre
   0,9977) et **perd 0,0004** sur la filtrée (0,9985 contre 0,9989). À
   N = 2000, le gain est net : **+0,0066** (0,9953 contre 0,9887) et la structure
   `decisions` passe de 0,9835 à 0,9930. À N = 200, en revanche, il ne fait pas
   mieux que le signe brut (0,8940 contre 0,8953) et il fait moins bien sur la
   filtrée (0,8536 contre 0,8612).
2. **Le centrage asymétrique `(c)` fait mieux que `(a)` partout, sauf au
   recall@10 de N = 5000 sur la campagne principale.** Il gagne **+0,0083 à
   N = 2000** (0,9970) et **+0,0146 à N = 1000** (0,9903) sur la campagne
   principale ; sur la filtrée, il atteint 0,9913 à N = 1000 et **1,0000** à
   N = 5000 (263 requêtes sur 263 parfaites). C'est un résultat cohérent avec
   l'intuition d'ADR-0076 : recentrer la requête sur la moyenne des documents
   alors que les scores de rescoring restent calculés sans centrage déplace les
   codes dans un sens qui ne correspond plus au classement final.
3. **Aucune variante ne dispense d'un `N` d'au moins 2000 pour une marge sûre.**
   Le meilleur chiffre à N = 1000 est **0,9913** (campagne filtrée, `(c)`),
   d'intervalle à 95 % de la moyenne [0,9870 ; 0,9955] ; sur la campagne
   principale, `(c)` donne 0,9903, d'intervalle [0,9864 ; 0,9943]. Les deux
   chevauchent 0,99, donc aucun `N = 1000` ne garantit la cible. À N = 2000,
   `(c)` donne 0,9970 [0,9949 ; 0,9991] sur la principale et 0,9973
   [0,9951 ; 0,9996] sur la filtrée, au-dessus de 0,99.
4. **Le centrage n'est pas un correctif gratuit.** Il demande de changer la
   fabrication de `codes.bin`, donc un ADR qui remplace ADR-0069 décision 6,
   alors que les codes sont un fichier dérivé, versionné par segment et
   reconstructible : adopter le centrage change ce que le moteur reconstruit et
   ce qu'un harnais doit reproduire, pour un gain qui n'apparaît qu'aux petits
   `N`.

**Recommandation, à valider par le propriétaire** :

- **garder le signe brut** (aucun centrage) pour `N = 5000`, le `N` que MES-01
  recommande : à ce `N`, le gain de `(a)` est de 0,0010 sur la campagne
  principale et négatif sur la filtrée, ce qui ne justifie pas de remplacer
  ADR-0069 décision 6 ;
- **ne pas retenir `(a)`** comme réglage par défaut : il ne dépasse pas le signe
  brut « de façon nette » au `N` retenu, la condition qu'ADR-0076 met à un ADR
  de remplacement ;
- **examiner `(c)` seulement si le coût de N = 5000 est refusé.** Le scoring
  asymétrique de `docs/03-recherche.md` (documents centrés, requête non
  centrée) atteindrait la cible dès N = 2000. Ce serait un levier à décrire puis
  à faire valider par un ADR avant tout code, et il ne serait pas non plus
  gratuit : il faut décider si les codes de `codes.bin` sont centrés côté
  documents, et comment le harnais de recette et `crible eval` le vérifient.

Limites à retenir :

- **les pools mesurés font au plus 50 000 passages**, contre **8,58 M** en
  production ; le comportement du centrage à cette échelle n'est pas mesuré et
  la confirmation est renvoyée à **P9**. Aucune extrapolation n'est faite ;
- la moyenne de centrage est celle des passages **vivants de l'index**, pas du
  pool : un filtre très sélectif ne la déplace pas, et c'est ce que
  l'implémentation devrait faire aussi (ADR-0076, décision 3). La variante
  filtrée le vérifie indirectement, puisque `(a)` y perd parfois du terrain ;
- les variantes `(b)` et `(d)` (moyennes distinctes pour documents et requête)
  n'ont pas été mesurées, comme le prévoit ADR-0076 ;
- aucune renormalisation après centrage n'a été faite : elle changerait le
  produit scalaire et sortirait du protocole.

## 5. Données brutes

`docs/mesures/MES-12-centrage-codes-binaires.2026-10-27.json` : les contrôles par
structure (`control_ok`, `codes_mismatched_words` nul, `control_comparisons` — de
40 à 1 000 comparaisons de top-10 selon la structure, 2 815 au total —), les
tables ci-dessus, la norme de la moyenne par structure, les **48 exécutions**
avec le bloc `strategy_control` (2 815 exécutions BINARY et 563 EXACT, dont
1 500 et 300 pour la campagne principale) et le bloc `controls` (déterminisme,
équivalence CLI), et les 563 lignes par requête (recall et recouvrement pour
chaque variante et chaque `N`). Les rangs de Hamming du résidu sont dans
l'artefact MES-01 (`residuals`). Aucun texte de requête, aucun `doc_id`, aucun
`iid` n'y figure.
Les fichiers de travail (`centering.json`, codes recalculés, `analysis.json`)
restent dans `data/mes01/` (gitignoré).

## 6. Constats hors mesure

- Le harnais importe `numpy` seulement (groupe par défaut de `tools/`) : il ne
  demande ni ONNX Runtime ni le modèle, et se rejoue donc sur n'importe quelle
  machine après la campagne.
- Le contrôle au bit près compare le signe **strict** (`> 0`) : une composante
  nulle ne pose pas de bit, comme ADR-0004 et ADR-0069 décision 6 le fixent.
  C'est un test unitaire du harnais, en plus du contrôle sur données réelles.
- Les codes recalculés de la variante `(a)` ne sont écrits nulle part : la
  mesure ne laisse aucun artefact susceptible d'être pris pour un format.
