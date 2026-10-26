# MES-14 — Coût des filtres d'intervalle sur colonne de valeurs, selon la taille du domaine

Ce fichier porte deux campagnes, dans l'ordre où elles ont été faites :

- **2026-10-22** (§ 1 à § 9), tâche `EVALUER-FILTRES` : coût de référence de
  `Pool::evaluate`, avant toute optimisation ;
- **2026-10-26** (§ 10), tâche `INSERER-POOL-EN-ORDRE` : rejeu de la même matrice
  avant et après le remplacement de `RoaringBitmap::insert` par une construction en
  ordre croissant des iid dans `scan_column` (`RoaringBitmap::append` au moment du
  relevé ; `from_sorted_iter` depuis, voir le § 10).

La campagne du 2026-10-22 n'a pas été réécrite ; le § 10 la reprend comme
référence « avant » et donne les écarts.

## 1. Campagne du 2026-10-22

- Tâche : `EVALUER-FILTRES` (phase P1), mesure demandée par le propriétaire avant la
  validation du jalon J1.
- Références : `docs/08-invariants-et-tests.md` (MES-14), `docs/03-recherche.md`
  (« Filtres → pool », « Budget de latence indicatif »), ADR-0018 (colonnes de
  valeurs, domaine réduit), ADR-0035 (valeurs nulles),
  `crates/crible-search/src/pool.rs` (`Pool::evaluate`, `evaluate_and`,
  `scan_column`).
- Commit : `b595386` (branche `phase/P1-stockage-et-recherche-exacte`). Aucun code
  de production n'a été modifié pendant la campagne : seul un banc a été ajouté
  (`crates/crible-search/benches/pool_filter.rs`), plus un déclencheur `[[bench]]`
  dans `crates/crible-search/Cargo.toml`. Aucune dépendance nouvelle.
- Machine : MacBook Air Apple M3 (8 cœurs : 4 performance, 4 efficacité), 24 Go de
  RAM (25 769 803 776 octets), macOS 15.7.7 (24G720). Mesure **native**, aucun
  conteneur. Le banc est compilé et lancé par `cargo bench` (profil `bench`,
  optimisations actives).
- Charge ambiante : la machine n'était **pas au repos**. La moyenne de charge est
  passée de 3,27 à 3,79 sur 8 cœurs entre le début (14:21:52Z) et la fin
  (14:53:45Z) de la campagne (`data/mes14/raw/conditions-start.txt` et
  `conditions-end.txt`). Le runtime du modèle local qui sert cet agent
  (`/Applications/Ollama.app`) tournait, comme lors de MES-15. La campagne
  elle-même n'a lancé aucun autre travail lourd. Les p95 et p99 restent proches
  des p50, sauf un cas (§ 4.2), donc l'effet de cette charge est faible sur ces
  chiffres, sans être mesuré.
- Jeux de données : `synth-small` attaché par `crible attach` (l1 puis l2 puis l3,
  état de référence de J1), et deux fixtures à l'échelle écrites par le banc. Les
  tailles de fichiers sont décimales (Mo = 10⁶ octets) ; les mémoires de ce
  paragraphe sont en Mio (1 Mio = 1 048 576 octets).

| Point | Passages vivants | Origine | `records.sqlite` | Somme de contrôle |
|---|---|---|---|---|
| `synth-small` | 9 750 | `crible attach` l1+l2+l3 | 4,25 Mo | jeu : `blake3:2d3d8265a8df350d816bccb5732fe338921d275cae5d091a96d0497646e58805` (voir note) ; `records.sqlite` : SHA-256 `f341df0d21e0ad13b4233016b224bdf6f5c869826b2a12db57b73163066ad229` |
| `fixture-1m` | 1 000 000 | banc `pool_filter prepare` | 213,8 Mo | SHA-256 `5ce1be32488c69560c86dfaa7bdad9443c6680211bfa40745ea083cbb142d4f4` |
| `fixture-8m58` | 8 580 000 | banc `pool_filter prepare` | 1 849,3 Mo | SHA-256 `d53184ac6821267a07cdf030aa15370579f9695d729b1d5434b304e809d68c66` |

  Note : la somme du jeu `synth-small` est celle que `tools/gen_synth.py` calcule
  (`gen_synth.dataset_checksum`) et qu'on retrouve dans `tools/README.md` ; elle
  couvre `index.json`, les trois manifestes de lot, leurs fichiers et les fichiers
  de référence du jeu, pas l'index attaché. Les SHA-256 portent sur le
  `records.sqlite` **mesuré**, relevés hors banc par `shasum -a 256`.

- Configuration mesurée : `Pool::evaluate` seul, filtre déjà validé et analysé
  hors de la boucle chronométrée. 1 000 évaluations par cas, 200 évaluations de
  préchauffage écartées, par point. Structures entièrement en RAM (cache chaud).
  Mémoire des structures de filtre relevée par `FilterIndex::memory()` : 0,6 Mio à
  9 750 passages, 43,2 Mio à 1 M, 369,3 Mio à 8,58 M. Construction des structures
  depuis `records.sqlite`, hors boucle : 0,03 s, 2,33 s, 20,56 s.

## 2. Protocole

Le banc est `crates/crible-search/benches/pool_filter.rs` (`harness = false`). Le
pilote complet est `tools/bench/mes14/run.sh`, qui enchaîne compilation, fixtures,
campagne, second tour de répétabilité, sommes de contrôle et fusion. Commandes
exactes :

```sh
# compilation du banc avant la campagne : aucune mesure ne paie rustc
cargo bench -p crible-search --bench pool_filter --no-run

# fixtures à l'échelle : N passages vivants, schéma et distributions de synth-small
cargo bench -p crible-search --bench pool_filter -- prepare \
    --index "$PWD/data/mes14/fixture-8m58" --passages 8580000

# un point : 1 000 évaluations par cas, 200 de préchauffage
cargo bench -p crible-search --bench pool_filter -- measure \
    --index "$PWD/data/mes14/fixture-8m58" --out "$PWD/data/mes14/raw" \
    --label fixture-8m58 --repeats 1000 --warmup 200 \
    --commit b595386 --machine "Apple M3 (8 cores) / 24 GB / macOS 15.7.7" \
    --date 2026-10-22

# fusion en un fichier de données brutes, à côté de ce rapport
python3 tools/bench/mes14/merge.py \
    docs/mesures/MES-14-cout-filtres-intervalles.2026-10-22.json \
    data/mes14/raw/synth-small.json data/mes14/raw/fixture-1m.json \
    data/mes14/raw/fixture-8m58.json data/mes14/raw/round2-*.json
```

- **Ce que chaque cas mesure** : un appel complet à `Pool::evaluate`. Les cas
  `keyword seul` (bitmap) servent de référence. Les cas `… seul, domaine =
  vivants` donnent à la clause d'intervalle le domaine de tous les vivants. Les cas
  `ET` placent une clause `keyword` devant : `evaluate_and` évalue d'abord les
  clauses en bitmap, puis la clause d'intervalle sur `présent ∩ vivants ∩ keyword`
  (ADR-0018). Deux tailles de domaine réduit sont ainsi couvertes : `partie=corps`
  (environ 18 % des passages) et `categorie=cat-03` (6,25 %).
- **Deux contrôles** : `annee >= 5000` lit les mêmes identifiants que
  `annee 2000..2010` et n'en garde aucun ; `annee >= 1900` les garde tous. Leur
  écart borne le coût d'insertion dans la bitmap de résultat.
- **Une plage à deux bornes coûte deux parcours.** `{"annee":{"gte":…,"lte":…}}`
  s'analyse en un `And` de deux conditions (`Filter::parse`), et `evaluate_and`
  restreint le second parcours au résultat du premier. La colonne « iid lus » des
  tableaux cumule donc les deux domaines ; elle est **dérivée** : les tailles des
  domaines viennent des mêmes `records.sqlite`, lues en SQL (`json_extract`) hors
  banc, pas de la boucle chronométrée.
- **Préchauffage** : 200 évaluations du même filtre avant chaque série de 1 000,
  écartées. Le parcours lit des structures déjà en RAM : il n'y a pas de variable
  « cache froid » à contrôler pour la boucle elle-même.
- **Répétitions et percentiles** : 1 000 évaluations chronométrées par cas, une
  paire `Instant` par évaluation. p50, p95 et p99 par **rang le plus proche**, sans
  interpolation : sur 1 000 échantillons, p95 est la 950ᵉ valeur et p99 la 990ᵉ.
  C'est la convention de MES-15, reprise ici.
- **Second tour** : les trois points rejoués intégralement le même jour, même banc
  (§ 6).
- **Fixtures à l'échelle** : ce sont des **dispositifs de mesure**, pas des index
  produits par `crible attach`. `prepare` écrit les tables de `records.sqlite` par
  `GenerationStore::create`, puis insère 1 M ou 8,58 M de passages et leurs
  documents avec le schéma et les distributions de `synth-small` (§ un document
  pour cinq passages, 5 % de nuls sur `annee`, `score`, `publie`,
  `date_publication`, 10 % sur `source`, `partie`, `date_effet`, 16 catégories,
  5 parties, années 1990-2025, dates 1990-2025). Tirage déterministe (splitmix64
  sur l'iid), donc rejouable. Aucun fichier de vecteurs, aucune ligne dans
  `segments` : `Pool::evaluate` ne lit que les structures de filtre. Les structures
  sont construites par `FilterIndex::from_records`, la fonction du chargement d'un
  index ; l'équivalence avec la dérivation `after_commit` d'une attache est celle
  des tests de `CONSTRUIRE-BITMAPS`, pas re-mesurée ici.

## 3. Résultats — `synth-small`, 9 750 passages vivants

En microsecondes, sauf mention. Rappel : la colonne « iid lus » cumule les deux
bornes d'une plage.

| Cas | iid lus (clauses cumulées) | pool | p50 | p95 | p99 | ns/iid lu |
|---|---|---|---|---|---|---|
| `keyword` seul, `categorie=cat-03` | — | 585 | 0,3 µs | 0,4 µs | 0,4 µs | — |
| `keyword` seul, `partie=corps` | — | 1 704 | 1,0 µs | 1,0 µs | 1,0 µs | — |
| `integer` seul, `annee 2000..2010`, domaine = vivants | 15 840 (9 225 + 6 615) | 2 880 | 173,7 µs | 181,1 µs | 187,8 µs | 11,0 |
| `date` seul, `date_publication 2000-01-01..2003-12-31`, domaine = vivants | 15 800 (9 250 + 6 550) | 975 | 144,5 µs | 154,2 µs | 161,1 µs | 9,1 |
| `float` seul, `score > 0.6`, domaine = vivants | 9 350 | 3 655 | 93,6 µs | 97,0 µs | 103,3 µs | 10,0 |
| `partie=corps` ET `annee 2000..2010` | 2 787 (1 618 + 1 169) | 495 | 38,0 µs | 38,4 µs | 43,1 µs | 13,6 |
| `partie=corps` ET `date_publication 2000-01-01..2003-12-31` | 2 750 (1 612 + 1 138) | 155 | 33,3 µs | 33,9 µs | 38,8 µs | 12,1 |
| `categorie=cat-03` ET `annee 2000..2010` | 935 (560 + 375) | 185 | 12,8 µs | 13,0 µs | 15,8 µs | 13,6 |
| `categorie=cat-03` ET `date_publication 2000-01-01..2003-12-31` | 950 (560 + 390) | 55 | 12,0 µs | 12,5 µs | 14,0 µs | 12,6 |
| contrôle `annee >= 5000`, mêmes iid lus, aucun gardé | 9 225 | 0 | 51,8 µs | 52,2 µs | 56,8 µs | 5,6 |
| contrôle `annee >= 1900`, mêmes iid lus, tous gardés | 9 225 | 9 225 | 120,9 µs | 143,8 µs | 149,0 µs | 13,1 |

Sur ce jeu, tout tient sous 1 ms, mais une clause d'intervalle seule coûte déjà
0,09 à 0,17 ms, soit 100 à 500 fois une clause en bitmap (0,3 à 1,0 µs). Ces
chiffres ne disent rien de l'échelle de Legifacile : 9 750 passages, c'est 880 fois
moins que la cible.

## 4. Résultats à l'échelle

### 4.1 `fixture-1m`, 1 000 000 de passages vivants

| Cas | iid lus (clauses cumulées) | pool | p50 | p95 | p99 | ns/iid lu |
|---|---|---|---|---|---|---|
| `keyword` seul, `categorie=cat-03` | — | 62 500 | 24,3 µs | 25,4 µs | 28,1 µs | — |
| `keyword` seul, `partie=corps` | — | 179 611 | 7,1 µs | 7,3 µs | 7,7 µs | — |
| `integer` seul, `annee 2000..2010`, domaine = vivants | 1 635 075 (949 295 + 685 780) | 289 700 | 17,3 ms | 17,9 ms | 18,3 ms | 10,6 |
| `date` seul, `date_publication 2000-01-01..2003-12-31`, domaine = vivants | 1 107 900 (949 845 + 158 055) | 105 620 | 9,8 ms | 9,9 ms | 10,2 ms | 8,8 |
| `float` seul, `score > 0.6`, domaine = vivants | 950 595 | 382 320 | 8,5 ms | 8,8 ms | 9,7 ms | 8,9 |
| `partie=corps` ET `annee 2000..2010` | 293 620 (170 499 + 123 121) | 51 971 | 4,2 ms | 4,4 ms | 4,7 ms | 14,4 |
| `partie=corps` ET `date_publication 2000-01-01..2003-12-31` | 198 741 (170 350 + 28 391) | 18 924 | 2,1 ms | 2,2 ms | 2,2 ms | 10,6 |
| `categorie=cat-03` ET `annee 2000..2010` | 101 445 (59 240 + 42 205) | 17 780 | 1,7 ms | 1,7 ms | 1,8 ms | 16,6 |
| `categorie=cat-03` ET `date_publication 2000-01-01..2003-12-31` | 69 040 (59 270 + 9 770) | 6 460 | 804,5 µs | 830,0 µs | 892,2 µs | 11,7 |
| contrôle `annee >= 5000`, aucun gardé | 949 295 | 0 | 6,1 ms | 6,3 ms | 6,8 ms | 6,5 |
| contrôle `annee >= 1900`, tous gardés | 949 295 | 949 295 | 10,7 ms | 11,2 ms | 13,9 ms | 11,3 |

À 1 M, un seul cas d'intervalle passe sous 1 ms : la plage de dates derrière
`categorie=cat-03` (0,80 ms, 69 040 iid lus). La plage d'années derrière le même
`keyword` coûte 1,7 ms.

### 4.2 `fixture-8m58`, 8 580 000 de passages vivants (échelle Legifacile)

| Cas | iid lus (clauses cumulées) | pool | p50 | p95 | p99 | ns/iid lu |
|---|---|---|---|---|---|---|
| `keyword` seul, `categorie=cat-03` | — | 536 250 | 209,6 µs | 219,0 µs | 237,2 µs | — |
| `keyword` seul, `partie=corps` | — | 1 541 689 | 50,9 µs | 52,2 µs | 57,3 µs | — |
| `integer` seul, `annee 2000..2010`, domaine = vivants | 14 034 695 (8 148 075 + 5 886 620) | 2 492 350 | 194,4 ms | 216,9 ms | 238,8 ms | 13,9 |
| `date` seul, `date_publication 2000-01-01..2003-12-31`, domaine = vivants | 9 513 200 (8 153 090 + 1 360 110) | 907 955 | 114,7 ms | 116,1 ms | 121,0 ms | 12,1 |
| `float` seul, `score > 0.6`, domaine = vivants | 8 150 880 | 3 262 150 | 101,2 ms | 101,6 ms | 101,9 ms | 12,4 |
| `partie=corps` ET `annee 2000..2010` | 2 520 766 (1 463 650 + 1 057 116) | 447 324 | 49,2 ms | 50,1 ms | 72,6 ms | 19,5 |
| `partie=corps` ET `date_publication 2000-01-01..2003-12-31` | 1 709 435 (1 464 826 + 244 609) | 163 230 | 24,9 ms | 25,3 ms | 26,2 ms | 14,5 |
| `categorie=cat-03` ET `annee 2000..2010` | 876 185 (509 230 + 366 955) | 154 800 | 21,1 ms | 22,0 ms | 23,3 ms | 24,1 |
| `categorie=cat-03` ET `date_publication 2000-01-01..2003-12-31` | 595 090 (509 750 + 85 340) | 57 290 | 9,1 ms | 9,3 ms | 9,7 ms | 15,3 |
| contrôle `annee >= 5000`, aucun gardé | 8 148 075 | 0 | 72,1 ms | 72,5 ms | 73,3 ms | 8,9 |
| contrôle `annee >= 1900`, tous gardés | 8 148 075 | 8 148 075 | 126,9 ms | 127,3 ms | 128,0 ms | 15,6 |

Le coût est **linéaire dans le nombre d'iid lus**, entre 8,9 et 24 ns par iid à
cette échelle. Ce que le domaine réduit achète est donc exactement sa réduction :
`categorie=cat-03` divise le coût de la plage par environ 9 pour `annee`
(194,4 ms → 21,1 ms) et par 12,6 pour les dates (114,7 ms → 9,1 ms). Aucune de ces
deux combinaisons n'atteint 1 ms.

Deux écarts entre cas ne sont pas expliqués par le seul nombre d'iid lus, et la
matrice ne les isole pas :

- `keyword` seul : `categorie=cat-03` (536 250 dans le pool) coûte 209,6 µs quand
  `partie=corps` (1 541 689 dans le pool) coûte 50,9 µs. Le pool n'ordonne donc pas
  le coût d'une clause en bitmap ; l'explication la plus probable est la forme des
  conteneurs de la bitmap de valeur (tableaux denses contre tableaux creux) dans
  `roaring`, ce qui n'a pas été vérifié par une réplique.
- Coût par iid lu : 13,9 ns pour `annee` et 12,1 pour `date` sur un domaine plein,
  mais 24,1 et 15,3 derrière `categorie=cat-03`, pour un travail de même nature. Le
  parcours d'un domaine creux fait des accès épars dans la colonne (i64 : 65,5 Mio
  à 8,58 M passages, i32 : 32,7 Mio), donc hors cache, là où le domaine plein
  parcourt la colonne en ordre croissant. C'est une interprétation tirée du code
  (`scan_column`) et des tailles de colonne, pas une décomposition mesurée.

## 5. Ce que coûte une clause d'intervalle, et ce qui la borne

Les deux contrôles donnent la borne basse et la borne haute du parcours à domaine
plein, sur les mêmes 8 148 075 iid lus :

| Contrôle, à 8,58 M | p50 | ns/iid lu |
|---|---|---|
| aucun gardé, aucun `insert` dans le résultat | 72,1 ms | 8,9 |
| plage `2000..2010`, 2 492 350 gardés | 194,4 ms | 13,9 (sur 14 034 695 lectures) |
| tous gardés | 126,9 ms | 15,6 |

Le parcours seul coûte **8,9 ns par iid**. Tout garder y ajoute **6,7 ns par iid
gardé** (126,9 contre 72,5 ms, soit 54,4 ms pour 8 148 075 insertions) : quand la
bitmap de résultat se remplit dense et en ordre croissant, l'insertion reste bon
marché. La même opération coûte **27,9 ns par iid gardé** quand la plage n'en garde
que 30 % : 14 034 695 lectures × 8,9 ns = 124,9 ms, mesurés 194,4 ms, donc 69,5 ms
pour 2 492 350 insertions. Le prix d'une insertion dépend de la densité du
résultat, pas du nombre de lectures.

`scan_column` insère chaque iid gardé par `RoaringBitmap::insert`, qui, dans
`roaring` 0.11.5, fait un `binary_search` puis un `Vec::insert` sur un conteneur de
type tableau (`array_store/mod.rs`, `ArrayStore::insert`), et non un ajout en fin :
le coût croît donc avec la longueur du conteneur, ce qui va dans le sens de l'écart
mesuré. C'est une **interprétation** appuyée sur la lecture de la dépendance, pas
une réplique ; elle n'a pas été mesurée isolément et n'appelle aucune modification
du code dans cette session.

Règle dérivée, à prendre comme un ordre de grandeur : à 8,58 M passages vivants et
cache chaud, **1 ms d'intervalle ≈ 40 000 à 110 000 iid lus** (1 ms divisé par 9 à
24 ns). Pour tenir sous 1 ms, il faut donc qu'une clause en bitmap ait déjà réduit
le domaine à quelques dizaines de milliers de passages, c'est-à-dire une clause
5 à 13 fois plus sélective que `categorie=cat-03` sur ce jeu (509 230 iid lus).

## 6. Répétabilité

Second tour intégral, même banc, même jour, mêmes fixtures. Écart des p50 :

| Point | Écart maximal des p50 entre les deux tours | Cas concerné |
|---|---|---|
| `synth-small` | +13,2 % | `date` seul |
| `fixture-1m` | +1,0 % | `integer` seul |
| `fixture-8m58` | −1,1 % | `partie=corps` ET `annee` |

Sur `synth-small`, les durées sont de quelques dizaines de microsecondes et
portent donc la dispersion de l'ordonnanceur (jusqu'à +13,2 % ; un p99 à 429,0 µs
au second tour sur `annee` seul, contre 187,8 µs au premier). À 1 M et 8,58 M, les
deux tours se suivent à 1,1 % près, ce qui rend les comparaisons entre cas solides.

## 7. Conclusion

Ce que la mesure permet d'affirmer :

1. **Une clause d'intervalle coûte, à l'échelle de Legifacile, 101 à 194 ms sur un
   domaine plein** (p50, cache chaud) : 101,2 ms pour `score > 0.6` sur 8,15 M iid
   lus, 114,7 ms pour une plage de dates, 194,4 ms pour une plage d'années à deux
   bornes. C'est **100 à 200 fois** le budget de 1 ms de `docs/03-recherche.md`.
2. **Un domaine réduit par une clause en bitmap ne suffit pas à tenir ce budget** :
   `categorie=cat-03` ramène le coût à 9,1 ms (dates) et 21,1 ms (années), soit
   encore **9 à 21 fois** le budget. `partie=corps` donne 24,9 et 49,2 ms.
3. **Les clauses en bitmap tiennent le budget** : 50,9 µs (`partie=corps`,
   1,54 M dans le pool) et 209,6 µs (`categorie=cat-03`, 536 250 dans le pool) à
   8,58 M passages vivants, soit 4,8 et 19,6 fois sous 1 ms.
4. **Le coût d'un filtre d'intervalle est linéaire dans le nombre d'iid lus** :
   8,8 à 10,6 ns par iid à 1 M, 8,9 à 24,1 ns à 8,58 M selon la densité du domaine
   et la part gardée. La loi tient sur trois tailles mesurées (9 750, 1 M, 8,58 M),
   sans extrapolation.
5. **Sur `synth-small` seul, le budget est respecté** (0,09 à 0,17 ms pour un
   domaine plein) : c'est le premier chiffre demandé par la fiche, et il ne préjuge
   rien de Legifacile, où les mêmes filtres coûtent 700 à 1 100 fois plus.

Ce que la mesure ne permet pas d'affirmer :

- Rien sur les index à 8,58 M produits par `crible attach` : les points à l'échelle
  sont des fixtures écrites par le banc (§ 2). Ce qui est mesuré est bien
  `Pool::evaluate` sur des structures construites par `FilterIndex::from_records`,
  mais ces fixtures n'ont ni pierres tombales, ni segments, ni vecteurs, et leur
  distribution est celle de `synth-small`, pas celle de Legifacile (longueur des
  plages de dates, cardinalité réelle des champs).
- Rien sur `synth-large` de `tools/gen_synth.py --large` : ce jeu **ne peut pas**
  servir MES-14. Son schéma déclare les champs `integer`, `float` et `date`, mais
  ses lignes ne portent que `categorie` ; toutes les colonnes de valeurs y seraient
  vides, et la mesure ne verrait que des parcours de zéro iid. C'est pourquoi les
  fixtures ont été écrites par le banc, sans dépendance nouvelle.
- Rien sur un domaine froid : les structures de filtre vivent en RAM à chaque
  requête, donc le cache de pages n'entre pas dans la boucle mesurée. La
  construction des structures depuis `records.sqlite` coûte 20,56 s à 8,58 M, mais
  c'est un coût de chargement, pas de requête.
- Rien sur les autres étapes de la requête : ni le scan dense, ni la branche
  lexicale, ni la fusion. Ce sont MES-02, MES-06 et MES-00.
- Rien de fin sur l'origine de l'écart entre types et entre densités de domaine
  (§ 4.2) : la matrice ne l'isole pas, et l'interprétation du § 5 n'est pas une
  réplique.

**Comparaison au budget.** `docs/03-recherche.md` donne « Filtre (bitmaps) < 1 ms »
avec MES-14 pour mesure. Le budget est **tenu pour les clauses en bitmap**, y
compris à 8,58 M passages vivants (209,6 µs au pire cas mesuré). Il est **dépassé
pour les clauses d'intervalle** dès que le domaine dépasse quelques dizaines de
milliers de passages : à 1 M, la plage de dates derrière `categorie=cat-03` est le
seul cas **d'intervalle** sous 1 ms (804,5 µs) ; à 8,58 M, aucun cas d'intervalle
ne l'est (9,1 ms au mieux). ADR-0018 l'avait annoncé (« une clause d'intervalle seule
parcourt toute la colonne ») et renvoyait la décision à cette mesure.

**Recommandations, à valider par le propriétaire** (aucune n'a été appliquée) :

1. **Séparer la ligne de budget en deux dans `docs/03-recherche.md`** : garder
   « Filtre (bitmaps) < 1 ms », validé par cette mesure, et donner à la clause
   d'intervalle sa propre ligne, avec la valeur mesurée (101 à 194 ms à domaine
   plein, 9,1 à 49,2 ms sous un domaine réduit de 1/5 à 1/16) ou une cible
   distincte. Une cible de 1 ms pour un intervalle n'est pas atteignable avec la
   structure d'ADR-0018 à 8,58 M passages ; l'écrire noir sur blanc évite qu'une
   revue la suppose tenue.
2. **Ouvrir un ADR pour un index de plage**, ce qu'ADR-0018 prévoyait (« si c'est
   trop lent, ajouter des bitmaps par tranche (par année), avec un nouvel ADR »).
   Forme proposée : une bitmap par année pour les champs `date`, une bitmap par
   tranche pour les champs `integer`, gardées dans `FilterIndex` à côté de la
   colonne ; une plage devient une union de quelques bitmaps au lieu d'un parcours
   de colonne. Le gain attendu n'est **pas** mesuré ici : il se déduit seulement du
   fait qu'une clause en bitmap coûte 51 à 210 µs à cette échelle. À chiffrer par
   une nouvelle campagne MES-14, après implémentation.
3. **Si aucun index de plage n'entre en v1** : décider explicitement que les
   filtres d'intervalle sur un domaine large dépassent 1 ms à l'échelle de
   Legifacile, et le porter dans les budgets de latence de bout en bout (le pool se
   calcule avant le scan dense ; 10 à 50 ms de filtre changent le budget d'une
   requête). La règle d'exploitation dérivée du § 5 — 1 ms ≈ 40 000 à 110 000 iid
   lus — permet de dire, requête par requête, si le filtre tiendra.
4. **Insertion dans la bitmap de résultat, tâche à part** : `scan_column` insère
   chaque iid gardé par `RoaringBitmap::insert`, dont le coût croît avec la taille
   du conteneur. Une construction en ordre croissant (ajout en fin, ou
   `insert_range` par bloc contigu) supprimerait ce terme. Le calcul du § 5 situe
   cette part à ~13 ms des 21,1 ms du cas `categorie=cat-03` ET `annee`
   (876 185 lectures × 8,9 ns = 7,8 ms de parcours, 13,3 ms pour 154 800
   insertions) — calcul, pas mesure. Proposition : une tâche dédiée, avec un test
   d'égalité des bitmaps et un rejeu de MES-14 ; ce n'est ni un seuil ni un réglage,
   donc aucune modification n'est faite ici.
5. **Aucun seuil `N` ni `exact_threshold`** ne se déduit de cette mesure : elle ne
   porte que sur l'évaluation du pool.

## 8. Données brutes

`docs/mesures/MES-14-cout-filtres-intervalles.2026-10-22.json` : les six points
(trois de la campagne, trois du second tour), 1 000 échantillons chronométrés par
cas et par point, les tailles de pool et de domaine, la mémoire des structures, la
durée de construction, la somme SHA-256 du `records.sqlite` de chaque point et la
somme blake3 du jeu `synth-small`. Les fixtures et les relevés intermédiaires
restent dans `data/mes14/` (gitignoré, 1,9 Gio) : `run.log`, les conditions de
charge, `checksum-records.json`, `checksum-synth-small.txt` et un JSON par point
dans `raw/`.

## 9. Constats hors mesure

- `tools/gen_synth.py --large` ne produit aucune valeur pour les champs `integer`,
  `float` et `date` : `build_large` n'écrit que `categorie` et le texte. Le jeu
  reste valable pour la latence dense, pas pour MES-14. À savoir avant de le
  proposer comme point à l'échelle d'une mesure de filtre.
- `docs/08-invariants-et-tests.md` décrit `synth-small` comme « 10 000 passages » ;
  l'état de référence après l1 à l3 en compte 9 750 vivants sur 12 000 lignes (150
  documents supprimés par l3). La fiche `EVALUER-FILTRES` et ce rapport emploient
  bien 9 750.
- Le banc relève « iid lus » comme le domaine de la **première** clause d'intervalle
  (`présent ∩ vivants ∩ keyword`). Pour une plage à deux bornes, la seconde clause
  parcourt le résultat de la première ; le tableau cumule les deux, et cette
  valeur vient d'un comptage SQL, pas de la boucle mesurée. Un futur banc qui
  instrumenterait `scan_column` donnerait la même grandeur sans dérivation.
- Le fichier du banc a été remis en forme par `cargo fmt` après la campagne : mise
  en forme seulement, aucune ligne de logique ni aucun paramètre n'ont changé. La
  campagne a tourné sur la version d'avant, dont le binaire a été écrasé par la
  reconstruction ; l'identité du code de mesure repose donc sur la nature du
  changement, pas sur une comparaison de binaires.

## 10. Campagne du 2026-10-26 — rejeu avant/après `INSERER-POOL-EN-ORDRE`

- Tâche : `INSERER-POOL-EN-ORDRE` (phase P4), mesure demandée par sa fiche et par
  l'« À surveiller » d'ADR-0046.
- Références : `docs/phases/P4-recherche-rapide.md` (`INSERER-POOL-EN-ORDRE`),
  `docs/08-invariants-et-tests.md` (MES-14), ADR-0018, ADR-0046,
  `crates/crible-search/src/pool.rs` (`scan_column`), § 4.2 et § 5 de ce fichier.
- Code : commit `4c7dd6c`, branche `tache/INSERER-POOL-EN-ORDRE`. Le relevé
  « avant » tourne sur l'arbre à ce commit ; le relevé « après » sur le même arbre
  avec `crates/crible-search/src/pool.rs` modifié non commité (SHA-256 du fichier
  mesuré : `0aa8117cfec84f3653f3c0bd217478cc97de1cc6bd31ae81385f7299d8eeb690`).
  C'est le **seul** fichier de production modifié entre les deux relevés. Aucune
  dépendance nouvelle, aucun changement de format.
- Ce fichier de production a changé depuis, sur demande de `@relecteur` : `scan_column`
  rend `Result<RoaringBitmap, CribleError>` et construit le résultat par
  `RoaringBitmap::from_sorted_iter(kept)`, avec une erreur `Internal` si l'ordre des
  iid n'est pas croissant, au lieu d'un `append` dont le résultat était ignoré
  (SHA-256 courant : `3ab91e57fbc356d91520551f2f1a3fba4f6290ae3659531825f38a1e566af378`).
  `from_sorted_iter` fait la même construction par ajout en fin de conteneur, et
  `scan.iter()` rend les iid en ordre croissant par contrat : la branche d'erreur
  n'est jamais prise. Les chiffres du § 10 restent donc ceux du relevé
  `0aa8117c…`, **sans remesure**.
- Machine : la même que la campagne du 2026-10-22 — MacBook Air Apple M3
  (`Mac15,12`, 8 cœurs : 4 performance, 4 efficacité), 24 Go de RAM, macOS 15.7.7
  (24G720), mesure **native**, aucun conteneur, profil `bench`.
- Conditions ambiantes : secteur, batterie à 100 %, aucun avertissement thermique.
  « Avant » : charge de 3,78 à 2,51 sur 8 cœurs (20:13:58Z → 20:46:39Z). « Après » :
  de 2,90 à 2,40 (20:57:18Z → 21:24:16Z). Lightroom et Arc tournaient pendant les
  deux relevés, `mediaanalysisd` et `mds_stores` seulement en fin de relevé
  « avant ». Aucun build pendant les mesures : le binaire de banc était compilé
  avant, une fois par relevé.
- Fixtures : **les mêmes**, vérifiées par SHA-256 avant chaque relevé et identiques
  entre l'avant et l'après.

| Point | Vivants | `records.sqlite` SHA-256 | Identique au § 1 |
|---|---|---|---|
| `synth-small` | 9 750 | `75ea8430b0624c2788f18ca4c74c0c25ebbc2a12b3a6018934639510908bf6c8` | **non** (voir note) |
| `fixture-1m` | 1 000 000 | `5ce1be32488c69560c86dfaa7bdad9443c6680211bfa40745ea083cbb142d4f4` | oui |
| `fixture-8m58` | 8 580 000 | `d53184ac6821267a07cdf030aa15370579f9695d729b1d5434b304e809d68c66` | oui |

  Note : l'index `synth-small` a été réattaché le 2026-10-26 à 11:17, donc sa somme
  de `records.sqlite` n'est plus celle du § 1 (`f341df0d…`, 4 247 552 octets ; il en
  fait 3 559 424). Le contenu utile à MES-14 est inchangé et l'équivalence a été
  vérifiée, pas supposée : même somme blake3 du jeu
  (`blake3:2d3d8265…e58805`), 9 750 passages vivants, **les 11 tailles de pool et
  les 11 tailles de domaine du banc identiques** au 2026-10-22, et les neuf tailles
  de domaine dérivées en SQL redonnent exactement les valeurs du § 3. Seule la
  mémoire des structures diffère (571 461 octets contre 671 737).

  Les 11 tailles de pool et les 11 tailles de domaine du banc sont **identiques
  avant et après**, cas par cas, sur les trois points : la colonne « pool » des
  tableaux du § 10 vaut donc pour les deux relevés, et la colonne « iid lus » aussi.
  C'est le contrôle de correction de la mesure — une optimisation qui changerait le
  résultat changerait ces nombres — et `tools/bench/mes14/compact.py` échoue
  désormais bruyamment si un rejeu futur les fait diverger, au lieu de produire un
  tableau crédible.

### 10.1 Protocole

Identique au § 2, à deux exceptions près : deux relevés au lieu d'un, et `--out`
redirigé. `tools/bench/mes14/run.sh` n'a **pas** été utilisé : il écrit le fichier
de données brutes dans `docs/mesures/` et ne sait pas produire deux relevés.

```sh
cargo bench -p crible-search --bench pool_filter --no-run   # avant chaque relevé

RAW="$PWD/data/mes14/raw-2026-10-26-avant"                   # puis "-apres"
M="Apple M3 (8 cores) / 24 GB / macOS 15.7.7"
for fx in synth-small fixture-1m fixture-8m58; do
  case $fx in synth-small) INDEX="$PWD/data/indexes/synth-small" ;;
               *)          INDEX="$PWD/data/mes14/$fx" ;; esac
  cargo bench -p crible-search --bench pool_filter -- measure \
    --index "$INDEX" --out "$RAW" --label "$fx" \
    --repeats 1000 --warmup 200 --commit 4c7dd6c --machine "$M" --date 2026-10-26
  cargo bench -p crible-search --bench pool_filter -- measure \
    --index "$INDEX" --out "$RAW" --label "round2-$fx" --round 2 \
    --repeats 1000 --warmup 200 --commit 4c7dd6c --machine "$M" --date 2026-10-26
done
```

1 000 évaluations chronométrées par cas, 200 de préchauffage écartées, cache chaud,
percentiles par rang le plus proche. Deux tours intégraux par relevé, donc quatre
passages par point au total.

**Bande de bruit du jour.** Écart maximal entre le premier et le second tour d'un
même relevé, cas par cas ; la plus grande des deux valeurs (avant ou après) est
retenue comme bande de bruit pour ce cas. C'est elle qui décide si un écart
avant/après est lisible.

| Fixture | Bande de bruit p50 | Bande de bruit p95 |
|---|---|---|
| `synth-small` | jusqu'à 24,1 % | jusqu'à 27,1 % |
| `fixture-1m` | jusqu'à 2,3 % | jusqu'à 11,7 % |
| `fixture-8m58` | jusqu'à 3,4 % | jusqu'à 13,0 % |

À `synth-small`, les durées sont de quelques dizaines de microsecondes et portent
la dispersion de l'ordonnanceur : un écart de 10 % n'y prouve rien. À 1 M et
8,58 M, un écart de p50 de plus de 2 à 4 % est lisible, et un écart de p95 doit
dépasser 12 à 13 % pour être distingué du bruit.

### 10.2 `fixture-8m58`, 8 580 000 passages vivants

En millisecondes, sauf mention. « iid lus » cumule les deux bornes d'une plage
(§ 2) ; « ns/iid lu » et « ns/gardé » sont `p50 / iid lus` et `p50 / pool` du
relevé « après », la colonne d'écart donne la même grandeur avant → après.

| Cas | pool | iid lus | p50 avant | p50 après | Δ p50 | p95 avant | p95 après | Δ p95 | ns/iid lu | ns/gardé | bruit p50 / p95 |
|---|---|---|---|---|---|---|---|---|---|---|---|
| `keyword` seul, `categorie=cat-03` | 536 250 | — | 211,9 µs | 211,0 µs | −0,4 % | 228,1 µs | 217,8 µs | −4,5 % | — | — | 0,8 / 3,5 % |
| `keyword` seul, `partie=corps` | 1 541 689 | — | 51,7 µs | 52,2 µs | +1,0 % | 56,7 µs | 53,9 µs | −4,9 % | — | — | 1,8 / 7,1 % |
| `integer` seul, `annee 2000..2010` | 2 492 350 | 14 034 695 | 194,93 | 160,62 | −17,6 % | 205,89 | 168,21 | −18,3 % | 13,9 → 11,4 | 78,2 → 64,4 | 0,5 / 0,8 % |
| `date` seul, `date_publication 2000..2003` | 907 955 | 9 513 200 | 115,16 | 95,54 | −17,0 % | 121,55 | 100,75 | −17,1 % | 12,1 → 10,0 | 126,8 → 105,2 | 0,9 / 1,6 % |
| `float` seul, `score > 0.6` | 3 262 150 | 8 150 880 | 101,80 | 86,00 | −15,5 % | 116,12 | 91,16 | −21,5 % | 12,5 → 10,6 | 31,2 → 26,4 | 0,6 / 7,6 % |
| `partie=corps` ET `annee` | 447 324 | 2 520 766 | 49,17 | 33,49 | −31,9 % | 53,26 | 34,16 | −35,9 % | 19,5 → 13,3 | 109,9 → 74,9 | 0,7 / 4,0 % |
| `partie=corps` ET `date` | 163 230 | 1 709 435 | 24,86 | 19,31 | −22,3 % | 26,32 | 20,24 | −23,1 % | 14,5 → 11,3 | 152,3 → 118,3 | 1,2 / 3,8 % |
| `cat-03` ET `annee` | 154 800 | 876 185 | 22,04 | 16,43 | −25,5 % | 23,94 | 18,20 | −24,0 % | 25,2 → 18,8 | 142,4 → 106,1 | 3,4 / 5,7 % |
| `cat-03` ET `date` | 57 290 | 595 090 | 9,42 | 7,77 | −17,6 % | 11,73 | 8,66 | −26,2 % | 15,8 → 13,1 | 164,5 → 135,6 | 1,8 / 13,0 % |
| contrôle `annee >= 5000`, aucun gardé | 0 | 8 148 075 | 72,24 | 71,67 | −0,8 % | 76,70 | 76,51 | −0,2 % | 8,9 → 8,8 | — | 0,4 / 1,9 % |
| contrôle `annee >= 1900`, tous gardés | 8 148 075 | 8 148 075 | 127,08 | 100,24 | −21,1 % | 142,16 | 105,03 | −26,1 % | 15,6 → 12,3 | 15,6 → 12,3 | 0,1 / 7,3 % |

Les deux cas `keyword` et le contrôle « aucun gardé » ne bougent pas (−0,8 % à
+1,0 %, dans le bruit) : c'est attendu, aucun ne passe par l'insertion, et cela
confirme que le parcours de la colonne n'a pas été touché. Les huit cas qui
passent par l'insertion — les sept cas d'intervalle et le contrôle « tous
gardés » — gagnent 15,5 à 31,9 % en p50 et 17,1 à 35,9 % en p95, tous au-delà du
bruit ; le contrôle « aucun gardé », qui n'insère rien, est exclu de cette plage.
La campagne dure 14 min 56 s « avant » et 12 min 13 s « après » pour
`fixture-8m58`.

### 10.3 `fixture-1m`, 1 000 000 de passages vivants

| Cas | pool | iid lus | p50 avant | p50 après | Δ p50 | p95 avant | p95 après | Δ p95 | ns/iid lu | ns/gardé | bruit p50 / p95 |
|---|---|---|---|---|---|---|---|---|---|---|---|
| `keyword` seul, `categorie=cat-03` | 62 500 | — | 24,4 µs | 24,3 µs | −0,2 % | 28,5 µs | 24,5 µs | −14,2 % | — | — | 0,3 / 11,7 % |
| `keyword` seul, `partie=corps` | 179 611 | — | 7,1 µs | 7,2 µs | +1,8 % | 8,2 µs | 7,4 µs | −9,6 % | — | — | 2,3 / 11,2 % |
| `integer` seul, `annee 2000..2010` | 289 700 | 1 635 075 | 17,50 | 15,18 | −13,3 % | 18,95 | 16,40 | −13,4 % | 10,7 → 9,3 | 60,4 → 52,4 | 1,0 / 0,8 % |
| `date` seul, `date_publication 2000..2003` | 105 620 | 1 107 900 | 9,75 | 8,67 | −11,1 % | 10,17 | 9,43 | −7,3 % | 8,8 → 7,8 | 92,3 → 82,1 | 0,2 / 1,7 % |
| `float` seul, `score > 0.6` | 382 320 | 950 595 | 8,50 | 7,61 | −10,4 % | 8,71 | 8,25 | −5,3 % | 8,9 → 8,0 | 22,2 → 19,9 | 0,4 / 1,3 % |
| `partie=corps` ET `annee` | 51 971 | 293 620 | 4,27 | 3,39 | −20,7 % | 4,42 | 3,42 | −22,7 % | 14,5 → 11,5 | 82,2 → 65,2 | 0,4 / 6,7 % |
| `partie=corps` ET `date` | 18 924 | 198 741 | 2,10 | 1,82 | −13,5 % | 2,29 | 1,87 | −18,3 % | 10,6 → 9,2 | 111,2 → 96,2 | 0,7 / 4,4 % |
| `cat-03` ET `annee` | 17 780 | 101 445 | 1,70 | 1,27 | −25,3 % | 1,91 | 1,34 | −29,8 % | 16,8 → 12,5 | 95,7 → 71,5 | 0,7 / 9,4 % |
| `cat-03` ET `date` | 6 460 | 69 040 | 808,0 µs | 699,1 µs | −13,5 % | 878,9 µs | 711,2 µs | −19,1 % | 11,7 → 10,1 | 125,1 → 108,2 | 1,3 / 6,5 % |
| contrôle `annee >= 5000`, aucun gardé | 0 | 949 295 | 6,15 | 6,14 | −0,2 % | 6,29 | 6,45 | +2,4 % | 6,5 → 6,5 | — | 0,0 / 5,9 % |
| contrôle `annee >= 1900`, tous gardés | 949 295 | 949 295 | 10,71 | 8,44 | −21,2 % | 11,04 | 9,73 | −11,9 % | 11,3 → 8,9 | 11,3 → 8,9 | 0,2 / 3,2 % |

Même lecture qu'à 8,58 M : les clauses en bitmap ne bougent pas en p50 (−0,2 % et
+1,8 %, dans le bruit), le contrôle « aucun gardé » non plus, et les cas
d'intervalle gagnent 10,4 à 25,3 % en p50. La p95 est moins nette : celle des deux
`keyword` « gagne » 14,2 et 9,6 % alors que leur p50 ne bouge pas, ce qui mesure la
dispersion de ce point (jusqu'à 11,7 % sur la p95) et non le changement. À 1 M,
seul le p50 porte une comparaison fine.

### 10.4 `synth-small`, 9 750 passages vivants

| Cas | pool | iid lus | p50 avant | p50 après | Δ p50 | p95 avant | p95 après | Δ p95 | ns/iid lu | ns/gardé | bruit p50 / p95 |
|---|---|---|---|---|---|---|---|---|---|---|---|
| `keyword` seul, `cat-03` / `partie=corps` | 585 / 1 704 | — | 0,4 / 1,0 µs | 0,4 / 1,0 µs | 0,0 % | 0,5 / 1,0 µs | 0,5 / 1,0 µs | 0,0 % | — | — | 20 / 27 % |
| `integer` seul, `annee 2000..2010` | 2 880 | 15 840 | 189,4 µs | 138,0 µs | −27,1 % | 206,7 µs | 151,7 µs | −26,6 % | 12,0 → 8,7 | 65,8 → 47,9 | 6,5 / 8,4 % |
| `date` seul, `date_publication 2000..2003` | 975 | 15 800 | 139,3 µs | 116,9 µs | −16,1 % | 160,8 µs | 120,7 µs | −25,0 % | 8,8 → 7,4 | 142,9 → 119,9 | 12,3 / 13,4 % |
| `float` seul, `score > 0.6` | 3 655 | 9 350 | 75,2 µs | 60,7 µs | −19,3 % | 77,9 µs | 63,3 µs | −18,7 % | 8,0 → 6,5 | 20,6 → 16,6 | 6,2 / 19,0 % |
| `partie=corps` ET `annee` | 495 | 2 787 | 40,7 µs | 31,8 µs | −22,0 % | 41,2 µs | 32,5 µs | −21,1 % | 14,6 → 11,4 | 82,2 → 64,1 | 0,8 / 5,9 % |
| `partie=corps` ET `date` | 155 | 2 750 | 31,3 µs | 26,9 µs | −14,1 % | 32,0 µs | 27,2 µs | −15,0 % | 11,4 → 9,8 | 202,2 → 173,7 | 2,5 / 15,9 % |
| `cat-03` ET `annee` | 185 | 935 | 14,0 µs | 10,7 µs | −23,3 % | 14,2 µs | 10,8 µs | −24,0 % | 14,9 → 11,5 | 75,5 → 57,9 | 4,8 / 6,7 % |
| `cat-03` ET `date` | 55 | 950 | 10,8 µs | 9,5 µs | −12,3 % | 11,0 µs | 9,7 µs | −11,8 % | 11,4 → 10,0 | 197,0 → 172,7 | 5,4 / 23,2 % |
| contrôle `annee >= 5000` / `>= 1900` | 0 / 9 225 | 9 225 | 49,6 / 104,9 µs | 45,5 / 80,2 µs | −8,2 / −23,5 % | 50,1 / 111,4 µs | 45,6 / 83,8 µs | −9,0 / −24,8 % | 5,4 → 4,9 | — / 11,4 → 8,7 | 5,3 / 18,0 % |

Sur ce jeu, l'écart du contrôle « aucun gardé » (−8,2 %) est dans la dispersion du
point (5,3 % de bruit p50, 6,2 % sur `float`), et ce cas ne passe par aucune
insertion : c'est du bruit, pas un effet du changement. Les cas d'intervalle
gagnent 12,3 à 27,1 %, mais `date` seul (−16,1 %) et `cat-03` ET `date` (−12,3 %)
ne dépassent leur bande de bruit que de peu. À cette taille, seule la direction
générale est exploitable, comme au § 6.

### 10.5 Ce que la mesure confirme et corrige du § 5

Le § 5 décomposait le coût d'une clause d'intervalle en deux termes : un parcours
à 8,9 ns par iid lu, plus une insertion dont le prix dépend de la densité du
résultat. La paire de contrôles mesure le second terme sans hypothèse : elle lit
**le même ensemble** de 8 148 075 iid et ne diffère que par ce qu'elle garde.

| Contrôles, à 8,58 M | avant | après | écart |
|---|---|---|---|
| aucun gardé, 0 insertion | 72,24 ms | 71,67 ms | −0,8 % |
| tous gardés, 8 148 075 insertions | 127,08 ms | 100,24 ms | −21,1 % |
| parcours, ns par iid lu | 8,87 | 8,80 | inchangé |
| terme d'insertion, ms | 54,84 | 28,56 | −26,28 ms |
| terme d'insertion, ns par iid gardé | 6,73 | 3,51 | −48 % |

À 1 M, la même paire donne 6,48 → 6,47 ns par iid lu et 4,81 → 2,42 ns par iid
gardé, soit −50 % du terme d'insertion. Le parcours n'a donc pas bougé, et
l'insertion coûte désormais deux fois moins, sans disparaître : le résultat se
construit toujours par un ajout en fin de conteneur, un iid gardé à la fois.

**Ce que la mesure confirme.** Le terme d'insertion existe et pèse lourd sur un
domaine plein : 54,84 ms des 127,08 ms du contrôle « tous gardés », et
l'essentiel des 34,3 ms gagnés sur `annee 2000..2010` (194,93 → 160,62 ms). Cette
décomposition se referme presque : la première borne insère 5 886 620 iid et la
seconde 2 492 350 ; au prix du contrôle dense (6,73 ns puis 3,51 ns par iid gardé),
elles coûtaient 39,6 + 16,8 ms et coûtent 20,7 + 8,7 ms, soit **27,0 ms gagnées**
contre 34,3 ms observées. Les 7,3 ms d'écart s'expliquent par la seconde borne, qui
ne garde que 42 % de son domaine et payait donc plus cher par insertion que le
contrôle dense. L'ordre de grandeur du § 5 (« environ 70 ms des 194,4 ms ») est donc
**confirmé** : 54,84 ms mesurés pour la première borne seule.

**Ce que la mesure corrige.** Le § 5 attribuait au cas `cat-03` ET `annee` 13,3 ms
d'insertion pour 154 800 iid gardés, soit **86 ns par insertion**, et expliquait
l'écart entre domaine plein et domaine creux par la longueur des conteneurs. Le
rejeu de ce cas donne 22,04 → 16,43 ms, soit **10,8 ns gagnés par iid inséré**
(521 755 insertions comptées sur les deux bornes, pas 154 800 : la première borne
en insère déjà 366 955). Deux corrections en découlent :

1. **Le nombre d'insertions était sous-compté.** Une plage à deux bornes insère
   deux fois : la première borne garde 366 955 iid, la seconde 154 800. Le § 5
   rapportait au seul pool final un coût payé par les deux bornes, ce qui gonfle le
   prix unitaire d'un facteur 3,4.
2. **L'écart entre domaine plein et domaine creux n'est pas, pour l'essentiel, un
   coût d'insertion.** Après la tâche, `cat-03` ET `annee` coûte encore 18,8 ns par
   iid lu, insertion comprise, contre 8,8 pour le parcours dense, alors que son
   insertion est devenue du même ordre que celle du contrôle dense. Le surplus vient
   donc du **parcours épars de la colonne** (accès hors cache), que la tâche ne touche
   pas et que sa fiche met hors périmètre. En retirant des 16,43 ms les 1,83 ms
   d'insertion estimés, il reste 14,6 ms de parcours pour 876 185 iid lus, soit
   **16,7 ns par iid lu** contre 8,8 en ordre croissant : c'est là que se loge le
   surplus. Les autres cas à domaine réduit retombent entre 8,0 et 10,6 ns par iid lu
   une fois l'insertion retirée ; c'est le `annee` derrière `cat-03` qui porte le cas
   extrême, et c'est précisément celui sur lequel le § 5 avait bâti son
   interprétation. Le § 5 le soupçonnait déjà sans pouvoir l'isoler ; la mesure le
   sépare maintenant.

**Sur le micro-contrôle de `@dev`** (4,0–4,3 ns par gardé avec `append` contre
12,3–12,6 avec `insert`, 8,1 M iid, 30 % gardés) : la direction et l'ordre de
grandeur du rapport (≈ 2 à 4) sont confirmés, mais ses valeurs absolues ne se
transposent pas. Sur les fixtures, le terme d'insertion vaut 6,73 → 3,51 ns par
iid gardé quand le résultat est dense et ordonné, et la variation observée sur un
résultat creux vaut 10,8 ns par iid inséré. Un banc isolé sur une plage contiguë
de 8,1 M iid ne voit ni la dispersion des conteneurs ni le parcours de colonne du
cas réel ; ses chiffres servent à prouver la direction, pas à remplacer la mesure
de bout en bout.

### 10.6 Réponse à l'« À surveiller » d'ADR-0046

ADR-0046 demande de vérifier que le gain n'est pas nul, faute de quoi la part
d'insertion était mal calculée. **Le gain n'est pas nul** : 15,5 à 31,9 % en p50 sur
les huit cas qui passent par l'insertion à 8,58 M, 10,4 à 25,3 % à 1 M, tous
au-delà de la bande de bruit du jour. La part d'insertion était donc réelle, mais
**mal répartie** : le total du domaine plein reste du bon ordre (54,8 ms mesurés sur
la première borne seule, contre « environ 70 ms » calculés pour les deux), alors que
le prix unitaire par insertion était surestimé d'un facteur 3 à 6 sur un domaine
réduit. Le § 5 est corrigé en ce point, et l'affirmation « le prix d'une insertion
dépend de la densité du résultat » est ramenée à ce qu'elle vaut : le prix d'un
**iid lu** dépend de la densité du domaine, celui d'un iid **gardé** dépend peu de
la densité du résultat.

Les deux autres points d'« À surveiller » ne sont pas instruits par cette campagne :
la latence de bout en bout sur l'index réel reste à `COMPARER-AVEC-PGVECTOR`, et
aucune mesure de bout en bout n'est faite ici.

### 10.7 Comparaison au budget

Aucun cas d'intervalle n'atteint 1 ms à 8,58 M passages : le meilleur est
`cat-03` ET `date` à **7,77 ms**, le pire `annee 2000..2010` à **160,62 ms**. Le
gain de 15 à 32 % ne change donc pas la conclusion d'ADR-0046 : le budget de
1 ms reste tenu par les clauses en bitmap (211,0 µs au pire cas mesuré) et dépassé
par les clauses d'intervalle dès que le domaine dépasse quelques dizaines de
milliers de passages. La décision de reporter l'index de plage à P9 n'est pas
rouverte par cette mesure.

La règle d'exploitation du § 5, « 1 ms ≈ 40 000 à 110 000 iid lus », découle du coût
par iid lu. Ce coût, mesuré sur ce relevé, passe de 8,9 à 25,2 ns avant (22,044 ms
pour 876 185 iid sur `cat-03` ET `annee`, tableau § 10.2) à 8,8 à 18,8 ns après, ce
qui donnerait **1 ms ≈ 53 000 à 114 000 iid lus** sur les cas mesurés ici, contre
40 000 à 112 000 avant. La proposition de mise à jour de la ligne de budget de
`docs/03` et de la valeur d'ADR-0046 est donc à valider par le propriétaire ; elle
n'est pas appliquée, et aucun seuil (`N`, `exact_threshold`) n'en découle.

### 10.8 Ce que la mesure ne permet pas d'affirmer

- Rien de plus que le § 7 sur l'échelle réelle : les points à 1 M et 8,58 M restent
  des fixtures écrites par le banc, sans pierres tombales, sans segments et sans
  vecteurs, de distribution `synth-small`.
- Rien sur l'ordre de grandeur à attendre sur l'index de Legifacile : la part
  d'insertion dépend du nombre d'iid gardés par requête, donc de la sélectivité
  réelle des filtres, qui n'est pas celle des fixtures.
- Rien sur les autres étapes de la requête : le gain de 15 à 32 % porte sur le seul
  calcul du pool, pas sur la latence de bout en bout.
- La séparation « parcours / insertion » des cas à domaine réduit (§ 10.5) reste une
  dérivation : elle soustrait aux p50 mesurés un coût de parcours pris sur un
  domaine plein. Un banc qui instrumenterait `scan_column` donnerait la même
  grandeur sans dérivation.

### 10.9 Données brutes

`docs/mesures/MES-14-cout-filtres-intervalles.2026-10-26.json` (44 ko) : les six
points (trois « après », premier et second tour), sans les 1 000 échantillons par
cas, plus les blocs `comparison`, `noise`, `controls` et `controls_1m`. Il est
produit par `tools/bench/mes14/compact.py` à partir des deux répertoires de relevés
cités ci-dessous.

Les relevés complets, échantillons compris, restent dans `data/mes14/`
(gitignoré) : `raw-2026-10-26-avant/` et `raw-2026-10-26-apres/`, chacun avec les six
JSON de mesure, les conditions de charge, les sommes SHA-256 des `records.sqlite` et
un `README.md` de campagne.
