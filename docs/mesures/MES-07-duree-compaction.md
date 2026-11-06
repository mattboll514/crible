# MES-07 — Durée d'une compaction à l'échelle de Legifacile

## Campagne du 2026-10-25

- Tâche : `MESURER-CYCLE-DE-VIE` (phase P3), passage B.
- Commit : `46c2dd1` (branche `tache/MESURER-CYCLE-DE-VIE`). **Aucun code de
  production n'a été modifié pendant la campagne.** Le banc
  `crates/crible-store/benches/compact_cost.rs` et les deux scripts
  `tools/bench/mes07/run.sh` et `tools/bench/mes08/make_batches.py` sont neufs et
  non commités au moment de la mesure ; le protocole de compaction mesuré est
  celui de `46c2dd1`.
- Machine : MacBook Apple M3 (8 cœurs : 4 performance, 4 efficacité), 24 Go de RAM,
  macOS 15.7.7 (24G720), APFS, 104 Gio libres après la campagne. Mesure
  **native**, aucun conteneur. Le banc est un binaire de
  `crates/crible-store/benches/`, compilé par `cargo bench` (profil `bench`,
  optimisations actives).
- Charge ambiante : la machine n'était pas au repos, comme lors du passage A.
  Charge moyenne 3,20 au début, 2,41 à la fin (8 cœurs) ;
  `mediaanalysisd` a occupé 65 à 79 % d'un cœur et `mds_stores` 42 à 62 % pendant
  toute la campagne (`data/mes07/raw/conditions-start.txt` et
  `conditions-end.txt`). C'est **moins** que le passage A (5,08 à 5,53), mais
  l'effet de cette charge n'est pas isolé : une répétition sur cinq porte une
  valeur aberrante (voir « Écarts au protocole »).
- Jeux de données :
  - `data/mes07/indexes/articles` : l'échantillon Legifacile, structure
    `articles` — 50 000 passages, 42 417 documents, textes réels (1 167 octets en
    moyenne, médiane 655), vecteurs fp16 768 dimensions, un segment, modèle
    `mdenseon` (`onnx`). 4 242 documents supprimés par une attache de suppressions
    réelle, soit 5 110 passages morts : **44 890 passages vivants, dead ratio
    0,1022**. `records.sqlite` :
    sha256 `b579a29cf1aac8f7b9a3ce30116a5d43044408ce8a7c8e98828ae97ca48fc169`,
    58 847 232 octets.
    Index : 130 Mo.
  - `data/synth-large/indexes/synth-large` : `synth-large`,
    8 600 000 passages (`tools/gen_synth.py --large`, graine 20261021), un
    passage par document, vecteurs fp16 768 dimensions, texte court
    (« large passage <n> »), modèle `external`. 860 000 documents supprimés par
    une attache de suppressions réelle : **7 740 000 passages vivants, dead ratio
    0,1000**, un segment source. `records.sqlite` :
    sha256 `d07fd173b6199074c93d37089561c02987276e35520d2940e6b3220a7d9ef1f5`,
    3 758 346 240 octets. Index : 16 Go (12,9 Go de vecteurs, 3,76 Go de
    `records.sqlite`).
- Configuration mesurée : celle des index (aucun réglage changé). La compaction
  est lancée à la main (`compact`) et non par `dead_ratio` : le seuil est déjà
  atteint (0,10) et le résultat est le même. Segments de sortie d'au plus
  1 000 000 de passages (`MAX_SEGMENT_PASSAGES`).

### Protocole

```sh
# campagne complète : préparation idempotente, puis 5 répétitions par index
sh tools/bench/mes07/run.sh
```

`run.sh` construit ce qui manque, mesure, puis fusionne les rapports bruts dans
`docs/mesures/MES-07-duree-compaction.2026-10-25.json`.

Ce que fait une répétition, dans un processus unique :

1. clone APFS (`cp -cR`) de l'index de base dans `data/mes07/work/rep-<i>` : la
   compaction remplace l'époque, donc chaque répétition part du même état ;
2. `GenerationStore::open` du clone (non chronométré) ;
3. **répliques** des deux lectures par passage, sur le clone, avant la
   compaction :
   - `passage_read` : `RecordStore::read_passage` pour **chaque** iid vivant —
     même requête SQLite et même décompression zstd que `records::passage_by_iid`,
     que la compaction exécute sur sa connexion d'écriture ;
   - `segment_open` : `SegmentReader::open` de chaque segment source, qui relit le
     fichier de vecteurs en flux pour son checksum blake3 ;
   - `vector_read` : `SegmentReader::vector` pour **chaque** iid vivant ;
   - `compress_new` / `compress_old` : compression des textes d'un échantillon de
     passages vivants (un iid sur 97, 463 textes pour `articles`, 20 000 pour
     `synth-large`), une fois avec un `zstd::bulk::Compressor` réutilisé (ce que
     fait la production depuis ADR-0048), une fois avec
     `zstd::stream::encode_all` par texte (ce qu'elle faisait avant) ;
4. `compact()` réel, chronométré, sur le même clone ;
5. relevé de l'état après, du nombre de segments neufs et de la taille de
   l'époque E+1, puis suppression du clone.

Chaque répétition relit tout l'index avant de compacter : le cache est donc
**chaud** pour la compaction, sauf pour la première répétition du premier point.
Aucun préchauffage séparé n'est fait. Les répliques sont des **copies** des
lectures de production, pas les appels de production : le rapport les étiquette
comme telles, et la durée de la compaction, elle, est celle du code réel.

Percentiles par rang le plus proche sur 5 répétitions ; sur 5 échantillons, p95 et
p99 valent le maximum.

### Résultats

Compaction complète, 5 répétitions, cache chaud, en millisecondes. `vivants` est
le nombre de passages copiés, `morts` la part de pierres tombales de la source.

| Index | Vivants | Morts | Segments source → neufs | p50 | p95 | p99 | min | max |
|---|---|---|---|---|---|---|---|---|
| `articles` (textes réels) | 44 890 | 10,22 % | 1 → 1 | **2 214** | 2 563 | 2 563 | 2 079 | 2 563 |
| `synth-large` (8,6 M) | 7 740 000 | 10,00 % | 1 → 8 | **377 924** | 398 867 | 398 867 | 366 184 | 398 867 |

Décomposition, p50 en millisecondes ; « reste » est
`compaction − passage_read − segment_open − vector_read` et porte l'ordre de
tri, les écritures de segments, les insertions SQLite (passages **et**
documents), la transaction, la vérification des checksums de E+1 et la bascule.

| Index | Compaction | `passage_read` | `segment_open` (source) | `vector_read` | Reste |
|---|---|---|---|---|---|
| `articles` | 2 214 | 651 (29,4 %) | 48 (2,2 %) | 95 (4,3 %) | **1 420 (64,2 %)** |
| `synth-large` | 377 924 | 36 115 (9,6 %) | 7 976 (2,1 %) | 16 796 (4,4 %) | **317 037 (83,9 %)** |

Coût unitaire par passage vivant, dérivé des p50 ci-dessus, en microsecondes :

| Index | Compaction | `passage_read` | `vector_read` | Reste | Octets de texte par passage |
|---|---|---|---|---|---|
| `articles` | 49,3 | 14,5 | 2,1 | 31,6 | 1 167 (moyenne) |
| `synth-large` | 48,8 | 4,7 | 2,2 | 41,0 | ~19 |

Compression des textes de l'échantillon, p50 en millisecondes, pour l'échantillon
entier (pas par passage) :

| Index | Textes | Contexte réutilisé (production) | Un contexte par texte (avant) | Gain |
|---|---|---|---|---|
| `articles` | 463 | 2,9 | 19,3 | **16,4 ms pour 463 textes**, soit 35 µs par texte |
| `synth-large` | 20 000 | 3,1 | 120,1 | **117 ms pour 20 000 textes**, soit 5,9 µs par texte |

Sortie de la compaction :

| Index | Époque E+1 | Segments neufs | Taille de E+1 |
|---|---|---|---|
| `articles` | 120 851 956 octets | 1 | 121 Mo |
| `synth-large` | 15 253 970 556 octets | 8 | 15,25 Go |

### Extrapolation à l'échelle de Legifacile

Deux extrapolations, toutes deux **linéaires en nombre de passages vivants**, et
signalées comme telles.

1. **Loi directe, depuis le point à 8,6 M.** `synth-large` compacte 7 740 000
   passages vivants en 377,9 s, soit 48,8 µs par passage. Legifacile complet
   compte 8 580 000 passages ; au seuil de compaction par défaut (10 % de morts)
   il en reste 7 722 000 vivants, soit **0,23 % de moins** que le point mesuré :
   **≈ 377 s**. Compaction à zéro mort : 8 580 000 vivants, extrapolation
   linéaire ×1,1085 → **≈ 419 s**.
2. **Loi par passage, depuis l'échantillon réel.** `articles` compacte
   44 890 passages à textes réels en 49,3 µs par passage. Appliqué à 8 580 000
   passages : 49,3 µs × 8,58 M = **≈ 423 s**.

Les deux lois donnent 419 à 423 s pour le même contenu, à 1 % près. Ce n'est pas
une preuve de linéarité parfaite : le coût par ligne grandit avec la taille de
`records.sqlite` (MES-13 mesure +16 % par ligne entre 1 M et 8,58 M pour
l'ouverture), donc l'extrapolation depuis l'échantillon est plutôt un **minorant**,
et le point à 8,6 M reste la référence.

### Écarts au protocole

- **Valeurs aberrantes, conservées.** Une répétition sur cinq est sortie très
  au-dessus des autres sur `passage_read` : 2 126 ms contre 507 à 741 ms pour
  `articles`, et 63 982 ms contre 35 893 à 37 049 ms pour `synth-large`. Aucune
  valeur n'est retirée ; p95 et p99 valent le maximum sur 5 échantillons, et les
  p50 sont stables (compaction de 366 à 399 s à 8,6 M, soit ±4,4 %). La cause
  n'est pas isolée : la charge ambiante de `mediaanalysisd` et `mds_stores` est le
  suspect, elle n'est pas prouvée.
- **Compression `old` bimodale** sur `synth-large` : 117 à 120 ms pour trois
  répétitions, 662 ms pour deux. Le p50 (120 ms) est retenu ; l'écart n'est pas
  expliqué.
- **Un seul segment en entrée** pour les deux index : `synth-large` a été attaché
  en un lot, donc la compaction écrit 8 segments et en relit 1. Le coût de
  `segment_open` porte sur un fichier de 12,9 Go.
- **`articles` n'est qu'une des six structures** de l'échantillon : un index
  Crible porte un schéma, donc les 257 626 passages de l'échantillon ne peuvent
  pas tenir dans un seul index. `articles` est la plus grosse structure
  (50 000 passages) et sa densité (1,18 passage par document) est proche de celle
  de l'échantillon (1,2551, MES-13).
- **La machine n'était pas au repos** (voir plus haut) ; la mesure est faite dans
  ces conditions et le signale, comme le passage A.

### Conclusion

Ce que la mesure permet d'affirmer :

1. **À l'échelle de Legifacile, une compaction dure environ 6 min 20 s**
   (377,9 s p50 mesurés sur 7,74 M passages vivants à 8,6 M passages) ; à zéro
   mort, environ 7 min (419 s extrapolés). C'est **deux ordres de grandeur
   au-dessus** du coût d'un commit de génération mesuré par MES-15 sur un index de
   taille comparable (`data/mes15/i-8580000-s10` contre `data/synth-large` ici,
   3,1 s), ce qui est attendu : la compaction réécrit tous les passages.
2. **`passage_by_iid` coûte 9,6 % de la compaction à 8,6 M** (36,1 s) **et 29,4 %
   sur l'échantillon réel** (0,65 s sur 2,21 s). L'écart vient de la taille des
   textes : 1 167 octets en moyenne sur l'échantillon contre ~19 sur
   `synth-large`, et c'est la décompression et l'analyse des champs qui coûtent,
   pas la requête SQLite. Une lecture par passage reste chère : 4,7 à 14,5 µs
   pièce.
3. **La relecture des vecteurs coûte 4,4 %** (16,8 s pour 7,74 M vecteurs, soit
   2,2 µs pièce), **et le checksum des segments source 2,1 %** (8,0 s pour
   12,9 Go, soit 1,6 Go/s). La compaction relit aussi E+1 pour le vérifier : ce
   second passage est dans le « reste », et il coûte du même ordre (8 s estimés
   par la même loi, non mesurés séparément).
4. **Le poste dominant est l'écriture** : 64 % sur l'échantillon, 84 % à 8,6 M.
   Elle porte 7,74 M insertions de passages **et** 7,74 M insertions de documents,
   plus l'écriture de 15,25 Go de segments et la transaction SQLite.
5. **Le gain du compresseur zstd réutilisé est confirmé** sur les textes de la
   compaction : 35 µs gagnées par texte réel (1 167 octets), 5,9 µs par texte
   court. Rapporté aux 7,72 M passages vivants de Legifacile, par la loi linéaire
   en nombre de textes (extrapolation), la compression coûterait **≈ 49 s avec le
   compresseur réutilisé contre ≈ 322 s avant**, soit **≈ 273 s (4 min 33 s)
   gagnées** sur une compaction de textes réels. Sur `synth-large`, où les textes
   font 19 octets, le même gain ne vaut que ≈ 44 s : c'est la taille des textes,
   pas le nombre de passages, qui porte ce poste.

Ce que la mesure ne permet **pas** d'affirmer :

- rien sur une compaction concurrente de lectures (INV-02) ni sur la
  récupération d'une panne en cours de compaction (INV-14) : ce sont des
  invariants, testés ailleurs ;
- rien sur un index à plusieurs centaines de segments en entrée : les deux index
  mesurés n'en ont qu'un, et `segment_open` est linéaire en octets, pas en
  nombre de fichiers ;
- rien sur le disque froid : toutes les répétitions tournent à cache chaud, et
  le premier accès d'une copie neuve n'est pas mesuré ici (MES-13 le chiffre à
  +1,3 % sur 8,58 M).

Cible ou budget : **il n'en existe aucun** pour MES-07 dans `docs/08`, `docs/03`
ni `docs/04` (le seul délai écrit est `wait_timeout_ms` = 60 s, qui borne une
attache côté client, pas une compaction). La mesure ne peut donc pas conclure à
un dépassement.

Recommandations, **à valider par le propriétaire** :

1. **Lire les passages vivants en un balayage ordonné plutôt qu'en une requête
   par iid.** C'est le seul poste que la mesure isole et qui soit franchement
   évitable : 36 s à 8,6 M avec des textes courts, et sans doute davantage avec
   des textes réels. Une lecture `SELECT … WHERE iid IN (…)` par blocs, ou un
   parcours ordonné de `passages`, remplacerait 7,74 M allers-retours. Le gain
   attendu est borné par les 9,6 à 29,4 % mesurés ; il ne demande aucun
   changement de format.
2. **Ne pas ouvrir de chantier sur la relecture des vecteurs** : 4,4 % à 8,6 M,
   et le protocole de `compact.rs` l'assume déjà (le checksum de E+1 est le
   contrôle d'intégrité qui autorise la bascule). La sampler changerait le
   protocole et demande une décision, pas une optimisation.
3. **Le poste d'écriture (64 à 84 %) n'est pas isolé** par cette campagne : une
   mesure de suivi qui sépare les insertions de passages, les insertions de
   documents, l'écriture des segments et la vérification dirait où va le temps.
   À proposer seulement si une compaction de 6 min gêne l'exploitation.

### Corrections du 2026-10-26 (NETTOYER-P3)

Texte seulement : aucune mesure n'a été rejouée.

- **Conclusion, point 1** : avant « le coût d'un commit de génération mesuré par
  MES-15 sur le même index (3,1 s) ». Après : « sur un index de taille comparable
  (`data/mes15/i-8580000-s10` contre `data/synth-large` ici, 3,1 s) ». Les deux
  index ne sont pas les mêmes fichiers : `synth-large` sert à cette campagne,
  `i-8580000-s10` à MES-15.

## Campagne du 2026-11-06, échelle réelle Legifacile, conteneur

- **Tâche** : `IMPORTER-BASE-COMPLETE` (phase P10), passe B, mesure MES-07. Branche
  `tache/IMPORTER-BASE-COMPLETE`, commit `e059fdf9758092f47a19e858593c316770a3958a`. **Aucun code de
  production n'a été modifié.** Le harnais `tools/bench/p10-import/` (`run-measured.py`) est neuf et
  non commité au moment de la mesure.
- **Protocole** : ADR-0139, décision 2. `crible compact` lancé **à la main**, jamais par `dead_ratio`,
  **en place** sur l'index importé, avant tout instantané. Une compaction par index, sans clone et
  sans copie : la seule option compatible avec la règle de non-suppression et avec le budget disque.
  Une seconde compaction en place sur `jorf` est un **cas distinct** (entrée multi-segments), pas une
  répétition.
- **Machine** : MacBook Air M3 (8 cœurs), 24 Go, macOS 15.7.7 (24G720). **En conteneur** : Docker
  Desktop 28.5.1, VM `aarch64` de **12 529 836 032 octets (11,67 Gio)** et 8 vCPU, volume nommé
  `crible-p10-data` sur `/data`, **aucune limite `--memory`** (`memory.max = max`).
- **Image** : `crible:p10`, `sha256:ff08d62b74d285acfccdc8f7b452ea77030786a65a831024698158b53b5f50b6`,
  construite depuis `e059fdf` (127 s). Sert les deux compactions comme l'instance.
- **Index mesurés** : `decisions` (3 589 776 passages vivants) et `jorf` (1 833 500), importés par
  `replace_all` avec l'image `sha256:1b0c66abc0af690b72c9c1e9fa1dafbdadb64b44912c66dd0c452c4be4460208`.
  **Le ratio de morts vaut 0 pour les deux** : aucun document n'a été supprimé depuis l'import, et la
  compaction n'a **rien retiré**. C'est le fait que la mesure déclare d'emblée.
- **Serveur arrêté pendant la mesure** (ADR-0116 : un seul écrivain par index ; la CLI n'écrit pas sur
  un index servi). Un conteneur neuf par compaction, sur le même volume, un seul index ouvert.

### Protocole

```sh
# decisions puis jorf : une compaction en place par index
python3 tools/bench/p10-import/run-measured.py --name p10-mes07-decisions \
    --key mes07/decisions -- compact --data /data --index decisions
python3 tools/bench/p10-import/run-measured.py --name p10-mes07-jorf \
    --key mes07/jorf -- compact --data /data --index jorf
# cas distinct : seconde compaction en place, entrée multi-segments
python3 tools/bench/p10-import/run-measured.py --name p10-mes07-jorf-second \
    --key mes07/jorf-second -- compact --data /data --index jorf
```

`run-measured.py` relève `memory.current`, `anon` et `file` du cgroup et le RSS de pid 1 **toutes les
5 s**, `df` sur l'hôte et dans la VM toutes les 60 s, et écrit un JSON par compaction sous
`data/p10-import/mes07/` (gitignoré). Artefact versionné :
`MES-07-duree-compaction.2026-11-06.json`.

Une compaction par index, **cache chaud** (l'index vient d'être écrit ou lu par la campagne), sans
préchauffage séparé. Aucune répétition n'a été faite : il y a **un point par index**, et le cas
multi-segments pour `jorf`.

### Résultats

| Index | Cas | Vivants | Morts | Segments source → neufs | Durée | µs / passage |
|---|---|---:|---:|---|---:|---:|
| `decisions` | 1re compaction, 1 segment en entrée | 3 589 776 | 0 | 1 → 4 | **371,0 s** | 103,3 |
| `jorf` | 1re compaction, 1 segment en entrée | 1 833 500 | 0 | 1 → 2 | **150,5 s** | 82,1 |
| `jorf` | 2e compaction, **2 segments en entrée** (cas distinct) | 1 833 500 | 0 | 2 → 2 | **142,1 s** | 77,5 |

Segments de sortie : `decisions` écrit `c2-0000000000` (1 000 000 vecteurs), `c2-0001000000`
(1 000 000), `c2-0002000000` (1 000 000) et `c2-0003000000` (589 776) ; `jorf` écrit
`c2-0000000000` (1 000 000) et `c2-0001000000` (833 500) à la première passe, puis `c3-0000000000`
(1 000 000) et `c3-0001000000` (833 500) à la seconde. Les segments d'entrée de l'import (`seg-decisions-full-v1`, `seg-jorf-full-v1`) disparaissent
avec l'époque 1 : la compaction écrit une **époque neuve** et le protocole retire l'ancienne, sans
réutiliser un seul segment (`docs/02:208`).

| Index | Cas | Époque avant → après | Génération | Octets écrits (époque neuve) | dont segments | dont `records.sqlite` |
|---|---|---|---:|---:|---:|---:|
| `decisions` | 1re | 1 → **2** | 1 → **2** | 10 252 383 506 | 5 858 535 761 | 3 939 201 024 |
| `jorf` | 1re | 1 → **2** | 1 → **2** | 4 964 150 023 | — | — |
| `jorf` | 2e | 2 → **3** | 2 → **3** | 4 964 150 023 | — | — |

L'époque neuve de `jorf` fait **exactement** les mêmes octets aux deux passes (4 964 150 023), et son
index lexical est reconstruit à chaque passe (455 Mo pour `decisions`, 287 Mo pour `jorf`), ce qui
explique qu'il figure dans les octets écrits.

Espace temporaire, mesuré et non extrapolé :

| Index | Cas | VM `/data` avant | Pic pendant | Hôte libre, série |
|---|---|---:|---:|---|
| `decisions` | 1re | 51,0 G | **63,2 GiB utilisés (+12,2)** | 170 → 156 → 170 Go |
| `jorf` | 1re | 51,0 G | **56,1 GiB utilisés (+5,1)** | 170 → 164 → 170 Go |
| `jorf` | 2e | 51,0 G | **56,2 GiB utilisés (+5,2)** | 170 → 163 → 170 Go |

Le pic vaut l'époque neuve **plus** l'ancienne, encore présente pendant l'écriture, puis l'espace
revient au niveau de départ dès la bascule. Le seuil d'arrêt de 40 Go libres sur l'hôte (ADR-0139,
décision 1) n'a jamais été approché : le minimum de la campagne est **156 Go**.

Mémoire du conteneur : `anon` est la mémoire possédée, `memory.current` inclut le cache de pages que
la compaction remplit en relisant l'index.

| Index | Cas | `anon` max | `memory.current` max | `VmHWM` max |
|---|---|---:|---:|---:|
| `decisions` | 1re | **2,64 GiB** | 10,99 GiB | 7,48 GiB |
| `jorf` | 1re | **2,52 GiB** | 10,90 GiB | 5,23 GiB |
| `jorf` | 2e | **2,47 GiB** | 7,99 GiB | 5,20 GiB |

Aucun OOM, aucune approche de la limite : `anon` reste sous **2,7 GiB**, et le `memory.current`
élevé est du cache de pages récupérable, que la VM évince d'elle-même.

### Incident — avertissement `tantivy` sur les deux compactions de `jorf`

Les deux compactions de `jorf` journalisent un `WARN` de `tantivy::indexer::segment_updater` : une
fusion de segments lexicaux est annulée parce que le répertoire temporaire de l'époque
(`epochs/tmp-2`, puis `epochs/tmp-3`) a disparu avant la fin de la fusion —
`OpenWriteError(NotFound)` sur `.tmpXXXXXX.fast`. La compaction avait déjà basculé l'époque.

Conséquence mesurée : l'index lexical de `jorf` garde **10 segments**, là où les index de taille
comparable en comptent 1 à 10 selon leur dernière fusion, mais **tous les passages sont là** — `meta.json` déclare 1 833 500 documents vivants, et une
recherche `lexical` est servie (63 ms). Aucune perte, aucun document manquant. L'avertissement n'est
donc **pas** un échec de la compaction ; il touche une fusion d'optimisation, après le commit. La
compaction de `decisions` ne l'a pas produit.

### Comparaison avec la campagne du 2026-10-25 — non directement comparable

Les deux campagnes ne mesurent pas la même chose :

- **Données** : 8,6 M passages **synthétiques** à texte de 19 octets, **10 % de morts** et un segment
  de 12,9 Go d'un côté ; des passages **réels** de 1,8 à 3,6 M, texte de 0,4 à 1,2 ko, **0 mort**, et
  un `records.sqlite` de 1,9 à 3,9 Go de l'autre. Une compaction à zéro mort recopie tous les
  passages ; la campagne de P3 en recopiait 90 %.
- **Machine** : **natif** (APFS, banc `cargo bench`, cinq répétitions sur clone `cp -cR`) contre
  **conteneur** (VM Linux de 11,67 Gio, un processus, une passe en place, aucun clone).
- **Ce qui reste comparable** : l'ordre de grandeur du coût unitaire. La campagne de P3 mesure
  **48,8 µs par passage** sur `synth-large` et 49,3 µs sur des textes réels ; ici la compaction coûte
  **77,5 à 103,3 µs par passage**, soit 1,6 à 2,1 fois plus. L'écart n'est pas attribué : il mêle des
  textes 20 à 60 fois plus longs, un `records.sqlite` plus gros, une écriture dans une VM et un seul
  passage au lieu d'une moyenne de cinq.

### Conclusion

- **Une compaction à l'échelle réelle dure 6 min 11 s sur `decisions`** (3,59 M passages) et
  **2 min 31 s sur `jorf`** (1,83 M), en conteneur, à zéro mort, cache chaud.
- **La seconde compaction de `jorf` coûte 142,1 s contre 150,5 s**, avec deux segments en entrée au
  lieu d'un : la différence de 8,4 s (5,6 %) est dans le bruit d'un point unique, et l'entrée
  multi-segments ne change pas l'ordre de grandeur. C'est le **cas distinct** qu'ADR-0139 annonçait,
  pas une répétition.
- **La mémoire possédée reste modeste** : 2,47 à 2,64 GiB d'anonyme, moins que la passe A pendant une
  attache (10,55 GiB). La compaction paie en **espace disque** (+5 à +12 GiB), pas en mémoire.
- **Aucune cible de durée n'existe** pour MES-07 dans `docs/03`, `docs/04` ou `docs/08` : la mesure ne
  peut pas conclure à un dépassement.

Ce que la mesure **ne** permet pas d'affirmer :

- le comportement d'une compaction à plusieurs centaines de segments en entrée ;
- le coût d'une compaction concurrente de lectures (INV-02), qui n'est pas le protocole retenu ;
- une distribution : il y a **un point par index**, sans répétition, et p50/p95/p99 n'ont pas de sens
  sur un point.

Recommandations, **à valider par le propriétaire** :

1. **Réserver 2 fois l'index compacté avant une compaction** : mesuré, +12,2 GiB pour `decisions`
   (9,6 GiB) et +5,1 GiB pour `jorf` (4,6 GiB). La réservation d'ADR-0129 (+28 Go) couvre les deux.
2. **Ne pas conclure sur les 10 segments lexicaux de `jorf`** : aucune mesure de latence lexicale
   multi-segments n'existe ici ; la recherche est servie en 63 ms, ce qui suffit à écarter un défaut
   bloquant, pas à établir un coût.
