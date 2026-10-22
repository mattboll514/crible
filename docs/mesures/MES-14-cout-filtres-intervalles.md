# MES-14 — Coût des filtres d'intervalle sur colonne de valeurs, selon la taille du domaine

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
