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
   au-dessus** du coût d'un commit de génération mesuré par MES-15 sur le même
   index (3,1 s), ce qui est attendu : la compaction réécrit tous les passages.
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
