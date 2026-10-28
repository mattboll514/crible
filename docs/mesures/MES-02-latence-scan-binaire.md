# MES-02 — Latence du scan binaire selon la taille du pool

## Campagne du 2026-10-27

- **Tâche** : `MESURER-LATENCE-DENSE` (P4), mesure MES-02.
- **Commit** : `5542ac095e4404cf2a4ff294309fbf95dfe50df2`, branche `tache/MESURER-LATENCE-DENSE`.
  **Aucun code de production n'a été modifié pendant la campagne.** L'exemple
  `crates/crible/examples/dense_latency.rs`, les scripts `tools/bench/mes02/` et leur entrée pytest
  dans `tools/pyproject.toml` sont **neufs et non commités au moment de la mesure** : la campagne porte
  donc sur `5542ac0` plus ce harnais, sans changement dans `crates/*/src/`.
- **Machine** : MacBook Air M3, 8 cœurs, 24 Go, macOS 15.7.7 (24G720), APFS, **natif**, pas de conteneur.
  Charge au départ : `3.28 3.53 3.59` (`data/mes02/raw/conditions-start.txt`) ; `mediaanalysisd` occupait
  environ un cœur, Lightroom, `mds_stores` et la session de l'agent tournaient aussi. La machine n'était
  donc **pas** au repos : les chiffres sont ceux d'un poste de travail actif, pas d'une machine dédiée.
- **Jeu de données** : `synth-large` (8 600 000 passages, 768 dimensions, vecteurs aléatoires normalisés),
  génération 2, époque 1. La copie de mesure `data/mes02/indexes/synth-large` (17 Go, gitignorée) porte
  **7 740 000 passages vivants** et 860 000 morts, répartis sur un seul segment `seg-large-0001`.
  Codes binaires : 825 600 000 octets. Bitmaps et colonnes : 361 661 936 octets
  (`live` 1 082 408, `keywords` 16 579 528, `columns` 344 000 000).
- **Requêtes** : 300 vecteurs `f32` de 768 composantes, graine 20261021, flux 7 du générateur,
  loi normale centrée réduite normalisée sur la sphère unité, distincts, SHA-256 de la matrice
  `03cb2a2b3bb7951892179ee8d612b0970626f0f041fe6b3e18c2f0393452948b`
  (`data/mes02/queries.json`, écrit par `tools/bench/mes02/mes02_queries.py`).
- **Configuration** : copie de mesure avec `search.exact_threshold = 0` et `search.scan = "binary"`,
  donc **BINARY sur chaque point** ; `candidates_per_branch = 100` (k) ; `rescore_candidates = 1000` ;
  8 fils de scan (`ScanPool`). Le mode `scan` du harnais ne mesure **que le scan**, sans rescoring ni
  payload.

### Protocole

Le protocole est celui d'**ADR-0077**. Les pools sont construits par le harnais, pas par un filtre :
`contiguous` prend une fenêtre de `size` identifiants vivants consécutifs à partir du tiers de la liste
des vivants ; `dispersed` tire `size` positions uniformément dans toute la liste des vivants, avec la
graine 2026, puis les trie. `full` est le bitmap des vivants entier.

Commandes, depuis la racine du dépôt (la recette complète est `tools/bench/mes02/run.sh`) :

```
cargo build --release --example dense_latency
H=./target/release/examples/dense_latency
I="--data data/mes02 --index synth-large --vectors data/mes02/queries.f32"
$H scan $I --out data/mes02/raw/scan-main.json
$H scan $I --sizes all --out data/mes02/raw/scan-all.json
$H scan $I --force-traversal dense  --out data/mes02/raw/scan-forced-dense.json
$H scan $I --force-traversal sparse --out data/mes02/raw/scan-forced-sparse.json
$H exact $I --out data/mes02/raw/exact.json
$H compare $I --filter '{"categorie":"cat-01"}' --out data/mes02/raw/compare-cat01.json
$H compare $I --out data/mes02/raw/compare-all.json
```

Les campagnes `scan-main` et `scan-all` valent 300 requêtes × 3 passages = **900 échantillons** par
point, sur les tailles 1 k, 10 k, 100 k et 1 M vivants en pool contigu et dispersé, puis sur le pool
complet (7,74 M), **à N = 1000, 2000 et 5000**. Les campagnes forcées (`scan-forced-dense`,
`scan-forced-sparse`) ne portent que sur **N = 1000**, comme le tableau des parcours forcés le montre.
Les percentiles sont ceux d'ADR-0077 : **rang le plus proche**, `ceil(ratio × n)`, sans interpolation.
Aucun passage n'est écarté : `tools/bench/mes02/mes02_analyze.py` recalcule chaque percentile à partir
des durées par requête et **refuse** un chiffre qui diffère du résumé écrit par le harnais.

**Le scan n'a pas d'état froid**, et le rapport l'écrit comme ADR-0077, décision 2, l'exige : ses codes
binaires sont chargés en RAM à l'ouverture et parcourus en entier à chaque requête (ADR-0069,
décision 3), si bien qu'aucune lecture de disque ne s'y intercale. La campagne ne mesure donc aucun
froid pour le scan, et les deux séries froides d'ADR-0077 ne portent que sur le rescoring (MES-03).

Deux écarts au protocole d'ADR-0077, déclarés :

- **Pas de passage d'échauffement séparé**, alors que la décision 4 du protocole écrit « après
  échauffement ». L'écart entre les trois passages d'un même point n'est pas négligeable. Il est donné
  sur les **45 points de la campagne**, séries automatiques et forcées séparées. Sur les **27 points
  automatiques** (`scan-main` et `scan-all`), les p50 par passage s'écartent de 0,0 % à **19,1 %**. Les
  plus grands écarts relatifs y sont sur les petits pools et sur 1 M dispersé — 1 k contigu à N = 5000 :
  48,08 / 47,54 / 56,62 µs, 19,1 % pour 9,1 µs ; 1 k contigu à N = 1000 : 11,1 % (6,0 µs) ; 1 k
  dispersé à N = 5000 : 9,0 % (4,4 µs) ; 1 M dispersé à N = 1000 : 11,42 / 11,70 / 12,31 ms, 7,8 %
  (0,89 ms). Les pools de 10 k à 100 k restent sous **3,2 %** (10 k contigu, N = 2000) et sous
  **27,2 µs** (100 k dispersé, N = 2000 : 1 737,96 / 1 765,12 / 1 749,00 µs) ; 1 M contigu sous 2,2 % (119 µs) ; le pool complet va de 0,6 % à
  4,9 %, soit **1,91 ms** à N = 1000 (38 856,67 / 40 343,50 / 40 763,75 µs). Sur les **18 points forcés**
  (`scan-forced-dense` et `scan-forced-sparse`, tous à N = 1000), l'étendue va de 0,2 % à **19,1 %**, et
  c'est là que se trouve le plus grand écart absolu de la campagne : pool complet, clairsemé forcé,
  32 688,54 / 37 617,88 / 38 939,29 µs, soit **6,25 ms** ; puis 7,8 % (pool complet, dense forcé,
  3,49 ms), 7,0 % (100 k dispersé, clairsemé forcé, 87,8 µs) et 6,9 % (1 M dispersé, dense forcé,
  1,98 ms). Le premier passage n'est le plus lent que sur **6 points sur 27** dans les séries
  automatiques et **2 sur 18** dans les forcées, soit **8 sur 45** : rien ne prouve qu'un échauffement
  écarté aurait changé ces chiffres, et les 900 échantillons incluent ce premier passage sans le
  retirer. L'écart reste un écart au protocole : il est mesuré, borné, et non rattrapé par un rejeu
  complet de la campagne.
- **La machine n'était pas au repos** (charge 3,3, autres applications actives). L'écart entre deux
  exécutions du même point est visible : le pool complet mesure 40,0 ms en parcours automatique et
  46,5 ms en parcours dense forcé, soit **16,1 % de dispersion inter-exécutions**
  (46,469 / 40,037 = 1,161). C'est le ratio de référence retenu partout ci-dessous.

Contrôles :

- **La réplique du harnais rend le même résultat que le moteur.** Le harnais ne peut pas construire un
  `Pool` arbitraire par l'API publique ; ADR-0077, décision 3, autorise donc une réplique du scan
  (précédent MES-13). Le mode `compare` ancre la réplique au moteur : sur un pool de 537 500 vivants
  (filtre `categorie = cat-01`) et sur les 7 740 000 vivants, les dix premiers résultats ont les mêmes
  clés et les mêmes scores, `plan binary` dans les deux cas. Le filtre `cat-01` est le seul rempli de
  `synth-large` ; ses identifiants internes sont entrelacés (`iid % 16`), donc ce pool est **dispersé**,
  et il ne sert qu'au contrôle, jamais de point de mesure. Les documents bruts de ce contrôle sont
  gitignorés ; leur **résumé est versionné** dans l'artefact, clé `controls` :
  `MES-02-latence-scan-binaire.2026-10-27.json` donne, pour `compare-cat01` et `compare-all`, le pool
  (537 500 et 7 740 000), le plan (`binary`), `identical: true` et la taille du haut de liste (10).
- **La CLI publique donne le même résultat que le moteur.** `crible search --vector-file` sur la même
  première requête rend les mêmes dix clés et les mêmes scores, avec et sans filtre, sur 537 500 et
  7 740 000 passages. Le même artefact, clé `controls.cli`, enregistre `same_keys_as_engine: true` et
  `max_score_delta: 0.0` pour `cli-cat01` et `cli-all`.

### Résultats

Les durées brutes par requête, passage par passage, sont dans
`MES-02-latence-scan-binaire.2026-10-27.json` ; les documents du harnais sont dans `data/mes02/raw/`.

Scan seul, en millisecondes, 900 échantillons par point, 8 fils.

| Pool | Forme | Parcours | p50 | p95 | p99 | max |
|---|---|---|---|---|---|---|
| 1 000 | contigu | dense | 0,055 | 0,089 | 0,118 | 0,206 |
| 1 000 | dispersé | clairsemé | 0,063 | 0,097 | 0,130 | 0,267 |
| 10 000 | contigu | dense | 0,375 | 0,431 | 0,462 | 0,509 |
| 10 000 | dispersé | clairsemé | 0,400 | 0,467 | 0,508 | 1,123 |
| 100 000 | contigu | dense | 0,978 | 1,061 | 1,085 | 1,230 |
| 100 000 | dispersé | clairsemé | 1,261 | 1,399 | 1,524 | 4,043 |
| 1 000 000 | contigu | dense | 4,942 | 5,370 | 5,617 | 8,899 |
| 1 000 000 | dispersé | clairsemé | 11,785 | 14,081 | 19,353 | 24,716 |
| 7 740 000 | complet | dense | 40,037 | 44,132 | 45,743 | 52,590 |

Parcours automatique : la règle d'ADR-0071 (dense dès que la densité du pool atteint 50 %) choisit
**dense** sur les pools contigus et sur le pool complet, **clairsemé** sur les pools dispersés.

Effet de N, p50 en millisecondes (les tailles de pool croissent plus vite que N : le tas de candidats
sature dès que `pool > N`).

| Pool | Forme | N = 1000 | N = 2000 | N = 5000 | 1000 → 5000 |
|---|---|---|---|---|---|
| 1 000 | contigu | 0,055 | 0,058 | 0,049 | bruit |
| 1 000 | dispersé | 0,063 | 0,060 | 0,051 | bruit |
| 10 000 | contigu | 0,375 | 0,408 | 0,402 | +7 % |
| 10 000 | dispersé | 0,400 | 0,437 | 0,438 | +10 % |
| 100 000 | contigu | 0,978 | 1,389 | 2,303 | +135 % |
| 100 000 | dispersé | 1,261 | 1,752 | 2,791 | +121 % |
| 1 000 000 | contigu | 4,942 | 5,500 | 7,155 | +45 % |
| 1 000 000 | dispersé | 11,785 | 12,415 | 14,159 | +20 % |
| 7 740 000 | complet | 40,037 | 42,134 | 44,578 | +11 % |

Parcours forcés, N = 1000, p50 / p95 / p99 en millisecondes. Le parcours forcé est celui d'ADR-0071
appliqué contre la règle, pour chiffrer ce que la règle évite.

| Pool | Forme | dense forcé | clairsemé forcé |
|---|---|---|---|
| 1 000 | contigu | 0,055 / 0,089 / 0,121 | 0,056 / 0,103 / 0,135 |
| 1 000 | dispersé | **17,452** / 19,127 / 20,016 | 0,051 / 0,090 / 0,113 |
| 10 000 | contigu | 0,375 / 0,433 / 0,464 | 0,376 / 0,433 / 0,463 |
| 10 000 | dispersé | **21,495** / 24,760 / 29,171 | 0,404 / 0,464 / 0,505 |
| 100 000 | contigu | 0,984 / 1,072 / 1,126 | 0,991 / 1,064 / 1,088 |
| 100 000 | dispersé | **27,612** / 32,633 / 37,264 | 1,276 / 1,506 / 1,651 |
| 1 000 000 | contigu | 5,338 / 5,935 / 6,151 | 4,709 / 4,994 / 5,258 |
| 1 000 000 | dispersé | **29,215** / 34,144 / 35,431 | 11,421 / 12,916 / 13,754 |
| 7 740 000 | complet | 46,469 / 52,125 / 53,958 | 37,495 / 41,721 / 43,474 |

EXACT au point de bascule (pool égal à N, k = 100), en microsecondes, 900 échantillons. C'est la
colonne « EXACT » demandée par ADR-0077 pour situer le seuil d'ADR-0074 ; les scores viennent de
`ExactSearcher::score_slice` répliqué, sans lecture de clé, et aucun ex æquo de score n'a été
rencontré. Trois limites de cette colonne, déclarées : elle est **une réplique non ancrée** — le mode
`compare` n'ancre que le scan et le rescoring, faute d'un moyen public de forcer EXACT sur un pool
arbitraire ;
elle est **pessimiste**, parce que chaque ligne `f16` est copiée dans un tampon réutilisé alors que le
moteur passe la tranche mmap directement au produit scalaire ; et son pool est **toujours contigu**
(`Shape::Contiguous`), ce qui la rend optimiste sur ce point. Les trois effets vont en sens contraire et
ne sont pas chiffrés séparément : ces chiffres situent un ordre de grandeur, ils ne notent pas EXACT.

| Pool | k | p50 | p95 | p99 | max |
|---|---|---|---|---|---|
| 1 000 | 100 | 74 | 127 | 170 | 1 707 |
| 2 000 | 100 | 137 | 213 | 245 | 2 685 |
| 5 000 | 100 | 266 | 344 | 390 | 3 893 |

### Conclusion

**Le budget de `docs/03-recherche.md` est tenu sur le pool complet et manqué sur un pool dispersé de
1 M.** Le tableau de budget donne « 20 à 80 ms » pour 8,6 M passages et « quelques ms » pour un pool de
1 M :

- à **7 740 000 vivants**, le scan mesure **40,0 ms en p50 et 44,1 ms en p95**, dans la fourchette
  20 à 80 ms, plutôt vers le bas ;
- à **1 M contigus**, il mesure **4,9 ms en p50 et 5,4 ms en p95**, ce qui tient « quelques ms » ;
- à **1 M dispersés**, il mesure **11,8 ms en p50 et 14,1 ms en p95**, soit **2,4 fois** le pool contigu
  et le double de « quelques ms ». Le budget n'est tenu que si les passages d'un pool de 1 M sont
  contigus dans l'espace des iid, ce que rien ne garantit après un filtre.

Ce que la mesure établit :

- **Le coût suit la taille du pool, pas le nombre de candidats.** À N = 1000, passer de 1 k à 1 M
  multiplie le p50 par 90 en contigu (0,055 → 4,942 ms) et par 187 en dispersé (0,063 → 11,785 ms) ;
  passer de N = 1000 à N = 5000 ne coûte que +11 % sur le pool complet et +45 % sur 1 M contigu. Le tas
  de candidats pèse donc peu devant le parcours, sauf quand `pool / N` devient petit (100 k : +135 %).
- **La dispersion coûte cher, et de plus en plus.** Le rapport dispersé / contigu vaut 1,15 à 1 k,
  1,07 à 10 k, 1,29 à 100 k et **2,4 à 1 M** : le parcours clairsemé saute d'un bloc de 64 codes à
  l'autre, et la part des sauts croît avec l'étalement du pool.
- **Le parcours dense sur un pool dispersé est un piège de facteur 340.** À 1 k dispersé, dense forcé
  coûte 17,5 ms contre 0,051 ms en clairsemé ; à 1 M, 29,2 contre 11,4 ms. La règle d'ADR-0071 évite
  correctement ce cas.
- **Deux inversions du parcours, une seule dans la bande de dispersion.** Sur le pool complet,
  clairsemé forcé (37,495 ms) est plus rapide que dense forcé (46,469 ms), soit **−19,3 %**
  (37,495 / 46,469 = 0,807), alors que la règle d'ADR-0071 choisit dense à 100 % de densité. À
  **1 M contigu**, le même ordre s'inverse : clairsemé forcé 4,709 ms contre dense forcé 5,338 ms
  (**−11,8 %**, 4,709 / 5,338 = 0,882), et 4,942 ms en dense automatique. ADR-0077, section « À
  surveiller », nomme exactement ce second cas — « si le parcours clairsemé dépasse le dense sur un
  pool contigu de 1 M » : la condition est remplie littéralement. Les deux écarts ne se situent pas au
  même niveau devant la **dispersion inter-exécutions du même pool, 16,1 %** (dense forcé 46,469 contre
  dense automatique 40,037 ; 46,469 / 40,037 = 1,161) : **−11,8 % reste dans cette dispersion** (11,8
  < 16,1), alors que **−19,3 % la dépasse** (19,3 > 16,1). Ce second écart vient d'une **exécution
  unique** de chaque parcours, sans alternance, et la mesure ne tranche donc pas. Elle signale que la
  règle « dense dès 50 % » n'est démontrée ni sur le pool complet ni sur un pool contigu de 1 M, et que
  ces deux cas méritent des exécutions alternées (dense, clairsemé, dense, clairsemé).
- **Sous quelques milliers de passages, EXACT est bien moins cher que BINARY suivi du rescoring.**
  EXACT à pool = 1000 coûte 74 µs en p50, contre 55 µs de scan plus un rescoring de 29,8 ms (MES-03)
  avec les mêmes 1000 candidats. Cela conforte le seuil d'ADR-0074 (1000) ; la mesure ne dit rien du
  point de croisement au-delà de 5 000, faute de point EXACT plus grand.

Ce que la mesure **ne** permet pas d'affirmer :

- la tenue du budget dans un conteneur (P8) ni sur le Mac mini : les chiffres sont natifs, sur un
  MacBook Air chargé ;
- l'effet d'un modèle réel sur le scan : `synth-large` a des vecteurs aléatoires, et seuls les codes
  binaires sont parcourus, donc la loi des vecteurs ne change pas le coût ;
- la généralité du rapport dispersé / contigu : `synth-large` est un seul segment de 8,6 M
  identifiants ; un index découpé en plusieurs segments ou compacté (identifiants renumérotés et
  denses) donnerait d'autres pools contigus.

**Proposition, à valider par le propriétaire** : instruire par un ADR les deux cas où le parcours
clairsemé devance le dense — le pool complet de 7,74 M et le pool contigu de 1 M — que la règle
d'ADR-0071 tranche aujourd'hui en faveur du dense. La mesure penche du côté du clairsemé : **−11,8 %**
sur 1 M contigu reste dans la dispersion inter-exécutions de 16,1 %, mais **−19,3 %** sur le pool
complet la dépasse. Les deux écarts viennent d'une exécution unique de chaque parcours : il faut soit
une série alternée (dense, clairsemé, dense, clairsemé) sur ces deux pools, soit laisser la règle en
l'état en la déclarant non mesurée sur ces points. Le second cas est celui qu'ADR-0077 a explicitement
mis sous surveillance ; ne pas l'instruire laisserait cette surveillance ouverte sans réponse.

## Complément du 2026-10-27 — parcours dense et clairsemé, passes alternées

La proposition ci-dessus a été instruite le même jour par une campagne dédiée, hors plan, sur la
même copie de mesure : `docs/mesures/MES-02-parcours-dense-clairseme.md` (artefact
`MES-02-parcours-dense-clairseme.2026-10-27.json`, mode `cross` du harnais `dense_latency`, recette
`tools/bench/mes02/run-parcours.sh`). Elle exécute les deux parcours **en alternance** dans une même
paire, après une passe d'échauffement séparée, sur des pools construits à densité donnée (10, 25,
40, 50, 60, 75 et 90 % de plage d'iid, à 1 M et 7,74 M d'iid) et sur les deux pools de cette campagne,
à N = 5000 et, sur ces deux pools, à N = 1000.

Résultat : **le clairsemé n'est jamais plus lent : dix-sept gains et un match nul** sur les 18 points ; l'inversion du pool
complet est confirmée (−19,2 % à N = 5000, −21,7 % à N = 1000), et celle de 1 M contigu se réduit à un
match nul à N = 1000 puis à −5,6 % à N = 5000. La conclusion sur le seuil d'ADR-0071, et la
proposition qui en découle, sont dans ce rapport dédié ; ce complément ne les reprend pas.
