# MES-06 — Branche lexicale : taille de l'index, durée de construction et latence BM25 filtrée

## Campagne du 2026-10-28

- Tâche : `MESURER-LEXICAL` (phase P5, fiche
  `docs/phases/P5-lexical-et-hybride.md:95-103`). Protocole fixé par **ADR-0091**,
  dont les six décisions sont suivies ici ; les écarts sont tous déclarés au § 1.3.
- Références : `docs/08-invariants-et-tests.md:71` (MES-06), `docs/03-recherche.md`
  (branche lexicale, `:84-95`, collecteur et block-WAND `:91`, tableau de budget
  `:145-155`), `docs/02-architecture.md:221` (tantivy, « à mesurer »), ADR-0007
  (tantivy pour BM25, attend ce chiffre), ADR-0047 (cible de 60 s au démarrage),
  ADR-0087 (pic RSS du commit, nombre de segments, budget de l'`IndexWriter`),
  ADR-0088 (durée de la reconstruction), ADR-0077 et ADR-0084 (percentiles et
  froid), ADR-0091 (protocole).
- Commit : `3614092a196aad0128157b5d42139122b9c6e55b`, branche
  `tache/MESURER-LEXICAL`. **Aucun code de production n'a été modifié pendant la
  campagne.** Les ajouts sont le harnais
  `crates/crible/examples/lexical_latency.rs`, les scripts et l'agrégateur de
  `tools/bench/mes06/`, leur entrée pytest dans `tools/pyproject.toml`, et la
  dépendance de test `tantivy` de `crates/crible/Cargo.toml` (déjà dans
  `docs/07`, déjà dépendance de `crible-store` et `crible-search`). Aucun
  changement de l'API publique.
- Machine : MacBook Air Apple M3 (8 cœurs : 4 performance, 4 efficacité), 24 Go
  de RAM (25 769 803 776 octets), macOS 15.7.7 (24G720), APFS. Mesure **native**,
  aucun conteneur. Binaire `cargo build --release`.
- Charge ambiante : la machine **n'était pas au repos**. La moyenne de charge a
  varié de 2,5 à 3,1 sur 8 cœurs pendant la campagne (Arc, opencode et Ollama
  actifs) ; aucun autre travail lourd n'a été lancé par la campagne. Les chiffres
  sont donc **natifs et indicatifs** : seule une mesure sur le Mac mini en
  conteneur serait contraignante (ADR-0081, ADR-0091 décision 4).
- Jeux de données :
  - `data/legifacile-sample/` : **257 626 passages**, 205 261 documents, six
    structures (50 000 passages par structure sauf `dole`, 7 628). Requêtes :
    `queries.jsonl`, 300 lignes, SHA-256
    `fdaa2794b027307cb9ac2e69c2dcb3a0f25ac5ff59b830b9c19ec359310391c6`.
    Les index de mesure sont construits par `crible index create` puis
    `crible attach` depuis les lots du jeu ; `records.sqlite` mesuré, SHA-256 :

    | Index | `records.sqlite` (SHA-256) |
    |---|---|
    | `articles` | `d390a099fdb0cb01134c402b1ed5609c57d678d14ff4acee3cad1f14927bc6f9` |
    | `decisions` | `4237650c45c62aff6ef31a5ecadeadb122c0579efeedefab6fd9a877c77c03b7` |
    | `jorf` | `0e24ce5e0bb780babfd109ec5ccf5b1716b217c9880f47a2945f4906e92447bf` |
    | `sarde` | `7f85b7d114f3509c93edcf84de80dbeb285671f0a1ee1a7335b3005e17cde4c6` |
    | `dole` | `f742e42cf43d855815f4cf97aec1aa3b87fc7261d381eac8447f67fba111ee6f` |
    | `titres` | `62d3f04ff7a44fece5caf934d76dfadce6a5c6ddbc0a91b44803c8dc1d17b232` |

    Chaque attache a écrit la génération 1, sans passage mort, avec une recette
    (ré-encodage ONNX fp32) dont le cosinus minimal a été de 0,999975 ou mieux.
  - `data/synth-large/indexes/synth-large` : **7 740 000 passages vivants** sur
    8 600 000 (`docs/mesures/MES-07-duree-compaction.md`), texte
    `large passage <n>`, `records.sqlite` SHA-256
    `d07fd173b6199074c93d37089561c02987276e35520d2940e6b3220a7d9ef1f5`.
    Le jeu est **réservé à la latence** (`docs/08:55`) : il ne sert ni à la taille
    ni à la durée de construction.
- Configuration mesurée : celle des index, `lexical.enabled = true`,
  `lexical.language = "fr"`, `lexical.positions = false` en campagne principale
  (`docs/03:88`), `true` en série secondaire. Aucun réglage de tantivy, de
  l'`IndexWriter` ni du pool n'a été touché : `WRITER_MEMORY_BUDGET_IN_BYTES`
  reste à 50 MiB, `WRITER_THREADS` à 1, `KEY_READ_CONNECTIONS` à 1.

## 1. Protocole

### 1.1 Harnais

`crates/crible/examples/lexical_latency.rs`, hors production, avec les modes
`probe`, `tokens`, `size`, `latency`, `wand`, `rebuild` et `copy-nocache`.
Il appelle l'**API publique** pour ce qu'il mesure : `Pool::evaluate` construit
chaque pool depuis un vrai filtre, et `crible_search::lexical::search` exécute la
branche. Deux modes lisent des structures que l'API n'expose pas, et le disent :

- `tokens` lit `records.sqlite` et les textes stockés pour compter les **jetons
  analysés** par structure, base de l'extrapolation (ADR-0091, décision 1) ;
- `wand` réplique le collecteur de `crates/crible-search/src/lexical.rs:527-600`
  pour compter les candidats visités, ce que la fonction publique ne rend pas.
  La réplique est **ancrée au moteur** : le mode compare ses clés, son ensemble
  de clés et son multiensemble de scores à ceux de `lexical::search` (§ 6).

L'agrégateur `tools/bench/mes06/mes06_analyze.py` relit les documents bruts,
**recalcule chaque percentile** depuis les durées par requête et **refuse** un
document dont le résumé ne correspond pas ; il écrit l'artefact daté.

### 1.2 Commandes

```sh
cargo build --release --example lexical_latency
cargo build --release --bin crible

# 1. Index de mesure de l'échantillon (create + attach, ORT_DYLIB_PATH du venv tools)
sh tools/bench/mes06/run.sh prepare

# 2. Taille et jetons analysés, positions = false
sh tools/bench/mes06/run.sh size

# 3. Filtres réels proches de 1 000, 10 000 et 50 000 passages
sh tools/bench/mes06/run.sh probe

# 4. Campagne réelle de latence (positions = false, k = 10 et 100, 3 passes)
sh tools/bench/mes06/run.sh latency

# 5. Reconstruction : false, puis true sur les mêmes données, puis retour à false
sh tools/bench/mes06/run.sh rebuild

# 6. synth-large : clone APFS, reconstruction, latence de plomberie, block-WAND
sh tools/bench/mes06/run.sh synth

# 7. La série froide, sur un point
sh tools/bench/mes06/run.sh cold

# 8. Agrégation et artefact daté
sh tools/bench/mes06/run.sh analyze
```

Chaque point de latence suit un **préchauffage** d'une passe complète, non
chronométrée, puis **3 passes** mesurées ; une **seule requête à la fois** ; le
nombre de threads rayon reste celui par défaut. Les percentiles sont au **rang le
plus proche** (`ceil(p × n)`), sans interpolation, et **`n` est écrit sur chaque
ligne**. La série froide est la seule à ne pas préchauffer (`--no-warmup`), ce que
le § 5.3 déclare.

Les filtres de pool sont **gelés** depuis la sortie de `probe` dans
`tools/bench/mes06/filters.json` (campagne principale),
`filters-secondary.json` (un seul point, positions `true`),
`filters-synth.json` (quatre points de `synth-large`),
`filters-wand-synth.json` (le plus grand pool) et `filters-cold.json` (le même,
pour la copie froide). Un filtre est une **vraie valeur** du jeu, jamais un
identifiant fabriqué : les tailles obtenues sont écrites à côté de la cible.

### 1.3 Écarts au protocole

1. **Positions `true`.** La configuration de mesure est éditée dans
   `data/mes06/indexes/<structure>/index.json` puis appliquée par le job
   `rebuild_lexical` : P5 n'expose aucun `PATCH` de configuration. Aucune donnée
   n'est modifiée en place hors de `data/mes06/` ; l'état `false` est restauré et
   sa taille retrouvée à l'octet près (§ 3).
2. **Requêtes de `synth-large`.** ADR-0091 décision 3 prévoit les seize mots du
   vocabulaire de `tools/gen_synth.py:134-151` (`df` ≈ 1/16). Ce vocabulaire
   n'alimente que la **petite** variante : la variante `--large` écrit
   `large passage <n>` (`tools/gen_synth.py:739`, déjà relevé par MES-07). Une
   première campagne avec ces seize mots a rendu **0 résultat** à tous les pools
   (mesuré, puis remplacé) : elle ne mesurait rien. Les requêtes de plomberie
   retenues sont donc `large`, `passage`, `large passage`, `42`, `1234`,
   `7740000`. Les deux premiers termes ont `df` = **100 %**, soit un pire cas
   **plus lourd** que le 1/16 prévu : la borne est conservatrice, jamais
   optimiste. La latence réelle à grand pool reste renvoyée à P9.
3. **Tailles de pool non atteignables.** Les filtres réels disponibles ne donnent
   pas toujours la cible. Tailles réellement obtenues, écrites dans chaque
   tableau : `sarde` **863** au maximum (limite de 64 clauses d'ADR-0029) ;
   `dole` **4 428** et **7 623** (tout le corpus fait 7 628 passages) ; `titres`
   **6 499** au minimum pour la cible de 1 000 (×6,5) ; `jorf` **1 905** (×1,9) ;
   `articles` **12 247** pour la cible de 10 000 (×1,2). Aucune valeur n'est
   extrapolée depuis ces écarts.
4. **`n` par structure.** Les 300 requêtes se répartissent en 200 `decisions`,
   63 `articles`, 12 `jorf`, 9 `sarde`, 8 `dole`, 8 `titres` : après 3 passes,
   `n` vaut 600, 189, 36, 27, 24 et 24. Les lignes « regroupées » du § 4.3
   rétablissent un `n` de 900, 873 et 849 sur l'ensemble des structures.
5. **Reconstruction mesurée en processus.** `rebuild` appelle
   `crible_store::generation::rebuild_lexical` depuis le harnais (API publique),
   pas la commande `crible rebuild-lexical`, pour éviter un processus silencieux.
   Le pic RSS est **échantillonné** par `ps` toutes les 200 ms, plus une mesure
   finale : c'est un **pic échantillonné**, pas une valeur exacte. À partir du
   second passage, le mode imprime aussi une ligne de progression toutes les 10 s.
6. **« 331 jetons sur `articles` » (ADR-0091, contexte).** Le harnais compte
   **120,07 jetons analysés** par passage d'`articles`. L'écart (×2,76) vient
   très probablement de la différence entre jetons du tokenizer et jetons après
   analyse (décomposition, mots vides, racinisation) ; la valeur de l'ADR n'est
   pas reproductible par le harnais. L'extrapolation suit la décision 1, qui
   demande les **jetons indexés** : elle porte donc sur les jetons analysés
   mesurés, et non sur 331.
7. **Froid.** Une seule série, sur un point, obtenue par une copie lue et écrite
   avec `F_NOCACHE` (`copy-nocache`, précédent ADR-0084), puis **une passe sans
   préchauffage** dans un processus neuf. `n = 6`. C'est une **borne**, pas une
   courbe : macOS ne purge pas son cache de pages sans `root`, et aucune purge
   n'a été tentée.
8. **Groupe d'égalité à la frontière du top-k** (optionnel, décision 5) : pas de
   point dédié. Il est observé indirectement par l'ancrage de la réplique
   (§ 6.2) : scores identiques 205 fois sur 205, ensemble de clés identique
   200 fois sur 205, ordre identique 174 fois sur 205 — l'écart est exactement
   l'ambiguïté d'égalité qu'`lexical::search` tranche par la clé.
9. **Pic RSS d'attache** (optionnel, décision 5) : mesuré par
   `/usr/bin/time -l` autour de `crible attach`, donc **maximum resident set
   size** du processus (§ 3.3), pas un échantillonnage.

## 2. Taille de l'index

### 2.1 Mesure, par structure

`lexical.positions = false` (défaut) puis `true`, sur les mêmes données.

| Structure | Passages vivants | Taille `false` (octets) | o/passage | Taille `true` (octets) | o/passage | Rapport `true/false` | Segments |
|---|---:|---:|---:|---:|---:|---:|---:|
| `articles` | 50 000 | 7 225 066 | 144,50 | 14 993 583 | 299,87 | 2,075 | 1 |
| `decisions` | 49 998 | 8 161 450 | 163,24 | 18 649 697 | 373,01 | 2,285 | 1 |
| `jorf` | 50 000 | 7 951 854 | 159,04 | 16 803 750 | 336,07 | 2,113 | 1 |
| `sarde` | 50 000 | 2 874 455 | 57,49 | 5 349 296 | 106,99 | 1,861 | 1 |
| `dole` | 7 628 | 1 844 795 | 241,85 | 4 256 798 | 558,05 | 2,307 | 1 |
| `titres` | 50 000 | 1 757 128 | 35,14 | 2 451 898 | 49,04 | 1,395 | 1 |
| **Total** | **257 626** | **29 814 748** | **115,73** | **62 505 022** | **242,62** | **2,096** | 6 |

Dix fichiers par index, un segment, aucun passage mort. Les positions doublent
presque le volume (×2,096 en moyenne), mais le surcoût dépend fortement de la
structure : ×1,40 sur `titres` (passages de 135 caractères) contre ×2,31 sur
`dole` (2 443 caractères). Les positions ajoutent une entrée par jeton indexé :
plus le passage est long, plus le surcoût est proportionnel.

### 2.2 Extrapolation aux 8,58 M passages

Base : **25 672 524 jetons analysés** pour **257 626 passages**, soit 99,65
jetons par passage en moyenne pondérée. L'extrapolation est linéaire en jetons
(ADR-0091, décision 1) et donnée en fourchette : le minimum et le maximum sont
les taux **par structure**, le central est le taux de l'échantillon entier
(total sur total), qui **est** le contrôle par le ratio de passages
(8 580 000 / 257 626 = **33,30**).

| Grandeur | Bas (structure la plus légère) | Central (ratio de passages) | Haut (structure la plus lourde) | Écart |
|---|---:|---:|---:|---:|
| Jetons analysés | 128,6 M (`titres`, 14,99/passage) | **855,0 M** (99,65/passage) | 2,00 G (`dole`, 233,45/passage) | ×15,6 |
| Taille, positions `false` | 0,30 Go (`titres`, 35,14 o/passage) | **0,99 Go** (115,73 o/passage) | 2,08 Go (`dole`, 241,85 o/passage) | ×6,9 |
| Taille, positions `true` | 0,42 Go (`titres`, 49,04 o/passage) | **2,08 Go** (242,62 o/passage) | 4,79 Go (`dole`, 558,05 o/passage) | ×11,4 |

Ce que la mesure permet d'affirmer : à l'échelle de Legifacile, l'index lexical
occupe **de l'ordre de 1 Go sans positions et 2 Go avec**, dans une fourchette de
0,3 à 2,1 Go et de 0,4 à 4,8 Go selon la composition réelle par structure. Elle
ne permet pas de choisir un chiffre unique : la composition de production par
structure est inconnue, et l'écart entre les bornes (×6,9 et ×11,4) est trop
grand pour être ignoré. Si un dimensionnement doit être arrêté avant P9, la
composition par structure devient une donnée à obtenir (ADR-0091, « À
surveiller »).

La mesure ne dit rien non plus de la place prise par les **fichiers `.store`**
(texte stocké par tantivy) : ils pèsent 201 328 octets sur `articles` (2,8 % de
l'index), donc la taille mesurée est bien celle des structures d'indexation, pas
une copie du corpus.

## 3. Durée de construction et empreinte du commit

### 3.1 Reconstruction, par structure

Job `rebuild_lexical` sur l'état publié, en processus, un thread d'écriture,
budget de 50 MiB.

| Structure | Positions | Durée (s) | Pic RSS (Mio) | Segments | Octets |
|---|---|---:|---:|---:|---:|
| `articles` | false | 2,274 | 132,0 | 1 | 7 225 066 |
| `articles` | true | 2,354 | 147,7 | 1 | 14 993 583 |
| `decisions` | false | 2,992 | 112,7 | 1 | 8 161 450 |
| `decisions` | true | 3,101 | 132,1 | 1 | 18 649 697 |
| `jorf` | false | 2,549 | 106,5 | 1 | 7 951 854 |
| `jorf` | true | 2,648 | 127,0 | 1 | 16 803 750 |
| `sarde` | false | 0,853 | 80,0 | 1 | 2 874 455 |
| `sarde` | true | 0,864 | 83,3 | 1 | 5 349 296 |
| `dole` | false | 0,738 | 57,3 | 1 | 1 844 795 |
| `dole` | true | 0,748 | 60,7 | 1 | 4 256 798 |
| `titres` | false | 0,366 | 68,0 | 1 | 1 757 128 |
| `titres` | true | 0,379 | 66,0 | 1 | 2 451 898 |
| **Total** | false | **9,771** | — | 6 | 29 814 748 |
| **Total** | true | **10,093** | — | 6 | 62 505 022 |

Deux contrôles de reproductibilité : les reconstructions de retour à `false`
redonnent **exactement** les octets mesurés avant la série `true` (7 225 066,
8 161 450, 7 951 854, 2 874 455, 1 844 795, 1 757 128), et l'ordre des durées
suit celui des jetons (2,99 s pour `decisions` à 8,11 M jetons contre 0,37 s pour
`titres` à 0,75 M). Les positions coûtent **+3,3 %** de durée (10,093 s contre
9,771 s) : le texte est analysé une fois de plus par champ de position, pas
ré-analysé.

### 3.2 Extrapolation aux 8,58 M passages, contre les 60 s d'ADR-0047

Débit mesuré : **2,63 M jetons analysés par seconde** (25 672 524 jetons en
9,771 s), soit 3,806 × 10⁻⁷ s par jeton, ou 37,9 µs par passage de l'échantillon.

| Composition supposée | Jetons à 8,58 M | Durée reconstruite |
|---|---:|---:|
| Toute en `titres` (14,99 j/passage) | 128,6 M | **48,9 s** |
| Moyenne de l'échantillon (99,65 j/passage) | 855,0 M | **325,4 s** (5 min 25 s) |
| Toute en `dole` (233,45 j/passage) | 2,00 G | **762,3 s** (12 min 42 s) |

La cible de 60 s d'ADR-0047 n'est tenue que si le corpus entier ressemble à
`titres` : le seuil d'équilibre est **18,4 jetons analysés par passage**
(157,6 M jetons). Avec les positions, le débit tombe à 2,54 M jetons/s et le
seuil à 17,8 jetons par passage. À la composition moyenne de l'échantillon, la
reconstruction prendrait **environ 5,4 minutes, soit 5,4 fois la cible**.

Un **contrôle indépendant** existe sur `synth-large` : 7 740 000 passages de
texte court, 119,6 Mo d'index, reconstruits en **14,549 s** avec 9 segments et un
pic RSS échantillonné de 1 470,5 Mio. Son débit (≈ 23 M jetons en 14,5 s si l'on
compte trois jetons par passage, soit 1,6 M jetons/s) est du même ordre que celui
de l'échantillon (2,6 M jetons/s) :
la linéarité en jetons n'est donc pas contredite par le seul point à grande
échelle disponible. Ce point ne vaut pas pour Legifacile (texte synthétique,
`docs/08:55`), il confirme seulement l'ordre de grandeur et le comportement des
segments.

`synth-large` donne aussi la seule observation de **plusieurs segments** :
119 569 302 octets en **9 segments** (58 fichiers), soit ≈ 13,3 Mo par segment
pour un budget d'écriture de 50 MiB. Le nombre de segments croît donc avec le
volume (≈ 1 segment par 860 000 passages), et le job ne les fusionne pas : le
`IndexWriter` est abandonné juste après le `commit`, sans attendre ni déclencher
de fusion. À 8,58 M passages, il faut attendre **une dizaine de segments** pour
un index lexical de 1 à 2 Go.

### 3.3 Pic RSS du commit, segments et jugement sur le budget de l'`IndexWriter`

- **Pic RSS de la reconstruction**, échantillonné : 57,3 à 147,7 Mio sur
  l'échantillon (maximum sur `articles` avec positions), **1 470,5 Mio** sur
  `synth-large` à 7,74 M passages.
- **Segments** : 1 par index d'échantillon, 9 pour `synth-large`.
- **Jugement sur le budget de 50 MiB et un thread (ADR-0087).** Le budget
  **n'est pas structurant** à cette échelle. Il produit 9 segments sur 7,74 M
  passages, sans tempête de fusions et sans coût de commit visible : 14,5 s pour
  indexer 7,74 M passages, contre 10,1 s pour les 257 626 passages de
  l'échantillon. Le pic RSS du processus (1,47 Gio) ne mesure pas ce budget — le
  budget borne la mémoire d'indexation d'un thread, pas le processus, qui porte
  aussi les pages de `records.sqlite` lues pendant le parcours. Aucun ADR n'est
  donc justifié par ce point ; en revanche, le nombre de segments produits par
  une reconstruction à 8,58 M passages (une dizaine) est à confronter à la
  latence de requête en P9, car chaque segment paie son propre parcours.
- **Pic RSS d'attache** (optionnel, mesure par `/usr/bin/time -l`, donc un
  maximum et non un échantillon) : 2 452 à 2 554 Mo (2 339 à 2 435 Mio) sur les
  six attaches, pour des durées de 13,2 s (`titres`) à 154,3 s (`articles`).
  **Hypothèse, non mesurée** : ce pic serait porté par la **session ONNX fp32 de
  la recette**, que `crible attach` ouvre pour ré-encoder son échantillon de
  contrôle. L'argument est une corrélation — le pic ne varie que de 4 % entre six
  structures dont le texte va de 135 à 2 443 caractères par passage — et la
  campagne n'a **aucun témoin sans recette** : l'attribution demanderait une
  attache sans ré-encodage, qui n'a pas été faite. Ce chiffre ne sert donc pas de
  mesure lexicale, et il ne prouve rien sur le coût mémoire du seul commit.

## 4. Latence BM25 filtrée, campagne réelle sur l'échantillon

### 4.1 Conditions

300 requêtes réelles de `queries.jsonl`, chacune rejouée sur l'index de sa
structure ; pools construits par `Pool::evaluate` depuis des filtres réels ;
`k = 10` principal, `k = 100` secondaire ; préchauffage puis 3 passes ; une seule
requête à la fois. La construction du pool est chronométrée **à part**
(`pool_millis`, 0,0052 à 0,1255 ms sur l'échantillon) et **n'entre pas** dans les
temps publiés, qui couvrent la recherche lexicale et la lecture des clés des
seuls candidats retenus. En millisecondes.

### 4.2 Par structure

| Structure | Pool (cible) | Taille réelle | `k` | p50 | p95 | p99 | `n` |
|---|---|---:|---:|---:|---:|---:|---:|
| `articles` | 1 000 | 1 027 | 10 | 0,405 | 0,837 | 1,012 | 189 |
| `articles` | 1 000 | 1 027 | 100 | 0,694 | 1,163 | 1,227 | 189 |
| `articles` | 10 000 | 12 247 | 10 | 0,315 | 0,709 | 0,844 | 189 |
| `articles` | 10 000 | 12 247 | 100 | 0,740 | 1,072 | 1,135 | 189 |
| `articles` | 50 000 | 50 000 | 10 | 0,248 | 0,412 | 0,458 | 189 |
| `articles` | 50 000 | 50 000 | 100 | 0,671 | 0,977 | 1,043 | 189 |
| `decisions` | 1 000 | 997 | 10 | 0,470 | 0,849 | 1,006 | 600 |
| `decisions` | 1 000 | 997 | 100 | 0,767 | 1,142 | 1,438 | 600 |
| `decisions` | 10 000 | 9 690 | 10 | 0,347 | 0,695 | 0,828 | 600 |
| `decisions` | 10 000 | 9 690 | 100 | 0,700 | 1,095 | 1,503 | 600 |
| `decisions` | 50 000 | 49 998 | 10 | 0,349 | 0,725 | 1,207 | 600 |
| `decisions` | 50 000 | 49 998 | 100 | 0,705 | 1,085 | 1,463 | 600 |
| `jorf` | 1 000 | 1 905 | 10 | 0,271 | 0,859 | 0,865 | 36 |
| `jorf` | 1 000 | 1 905 | 100 | 0,513 | 1,141 | 1,145 | 36 |
| `jorf` | 10 000 | 9 877 | 10 | 0,195 | 0,695 | 0,697 | 36 |
| `jorf` | 10 000 | 9 877 | 100 | 0,519 | 0,984 | 0,989 | 36 |
| `jorf` | 50 000 | 50 000 | 10 | 0,182 | 0,601 | 0,628 | 36 |
| `jorf` | 50 000 | 50 000 | 100 | 0,517 | 0,944 | 0,967 | 36 |
| `sarde` | 1 000 | 863 | 10 | 0,162 | 0,391 | 0,402 | 27 |
| `sarde` | 1 000 | 863 | 100 | 0,415 | 0,911 | 0,920 | 27 |
| `dole` | 1 000 | 4 428 | 10 | 0,079 | 0,156 | 0,163 | 24 |
| `dole` | 1 000 | 4 428 | 100 | 0,328 | 0,393 | 0,403 | 24 |
| `dole` | 10 000 | 7 623 | 10 | 0,066 | 0,138 | 0,142 | 24 |
| `dole` | 10 000 | 7 623 | 100 | 0,321 | 0,385 | 0,387 | 24 |
| `titres` | 1 000 | 6 499 | 10 | 0,151 | 0,348 | 0,356 | 24 |
| `titres` | 1 000 | 6 499 | 100 | 0,437 | 0,590 | 0,595 | 24 |
| `titres` | 10 000 | 11 946 | 10 | 0,212 | 0,376 | 0,386 | 24 |
| `titres` | 10 000 | 11 946 | 100 | 0,460 | 0,615 | 0,621 | 24 |
| `titres` | 50 000 | 50 000 | 10 | 0,145 | 0,323 | 0,323 | 24 |
| `titres` | 50 000 | 50 000 | 100 | 0,432 | 0,574 | 0,578 | 24 |

**Le pool ne commande pas le temps.** Pour une même structure, la p95 varie de
façon non monotone et souvent **à la baisse** quand le pool grandit : `articles`
passe de 0,837 ms (pool 1 027) à 0,412 ms (pool 50 000) à `k = 10`. Un plus grand
pool ne coûte donc rien de plus, et un petit pool ne protège de rien : le coût
suit la **longueur des listes d'inversion parcourues**, pas le nombre de passages
retenus. C'est le constat du § 6, chiffré.

### 4.3 Points regroupés sur les 300 requêtes

Les mêmes échantillons, réunis par taille de pool cible et par `k`, pour donner
une figure de la branche et non celle de la structure la plus favorable.

| Pool (cible) | `k` | `n` | p50 | p95 | p99 |
|---|---:|---:|---:|---:|---:|
| 1 000 | 10 | 900 | 0,426 | 0,839 | 1,006 |
| 1 000 | 100 | 900 | 0,733 | 1,142 | 1,269 |
| 10 000 | 10 | 873 | 0,330 | 0,695 | 0,839 |
| 10 000 | 100 | 873 | 0,694 | 1,080 | 1,492 |
| 50 000 | 10 | 849 | 0,318 | 0,654 | 1,206 |
| 50 000 | 100 | 849 | 0,687 | 1,064 | 1,456 |

La p95 reste **sous 1,2 ms** partout, et `k = 100` coûte **1,7 à 2,2 fois**
`k = 10` en p50 : la lecture des clés et le tri des 100 résultats pèsent plus que
le parcours lui-même à cette échelle.

### 4.4 Série secondaire, positions `true`

Un seul point de pool par structure, le plus grand atteignable, sur les mêmes
données et les mêmes requêtes.

| Structure | Pool (taille) | `k` | p50 `false` | p50 `true` | p95 `false` | p95 `true` |
|---|---:|---:|---:|---:|---:|---:|
| `articles` | 50 000 | 10 | 0,248 | 0,250 | 0,412 | 0,425 |
| `articles` | 50 000 | 100 | 0,671 | 0,672 | 0,977 | 0,980 |
| `decisions` | 49 998 | 10 | 0,349 | 0,353 | 0,725 | 0,728 |
| `decisions` | 49 998 | 100 | 0,705 | 0,708 | 1,085 | 1,091 |
| `jorf` | 50 000 | 10 | 0,182 | 0,184 | 0,601 | 0,603 |
| `jorf` | 50 000 | 100 | 0,517 | 0,588 | 0,944 | 0,965 |
| `sarde` | 863 | 10 | 0,162 | 0,162 | 0,391 | 0,389 |
| `sarde` | 863 | 100 | 0,415 | 0,423 | 0,911 | 0,915 |
| `dole` | 7 623 | 10 | 0,066 | 0,072 | 0,138 | 0,149 |
| `dole` | 7 623 | 100 | 0,321 | 0,337 | 0,385 | 0,400 |
| `titres` | 50 000 | 10 | 0,145 | 0,164 | 0,323 | 0,333 |
| `titres` | 50 000 | 100 | 0,432 | 0,434 | 0,574 | 0,574 |

Les positions ne changent pas l'ordre de grandeur de la latence : l'écart de p95
va de −0,5 % (`sarde`, `k = 10`) à +8,0 % (`dole`, `k = 10`), sans direction
systématique, et reste dans le bruit d'une machine chargée. **Positions `false` reste le bon
défaut pour la latence** ; `true` se paie en taille, pas en temps de requête, à
cette échelle.

## 5. Campagne de plomberie sur `synth-large`

**Pire cas synthétique, à confirmer en P9.** Ces chiffres ne valent pas pour
Legifacile : le texte n'est pas du français (`large passage <n>`) et les requêtes
n'ont aucune valeur de production. Ils bornent un comportement, rien de plus.

### 5.1 Pools et latence chaude

Les pools sont des valeurs de `categorie` (une seule, puis unions **non
adjacentes**), donc réparties dans le corpus ; le plus grand pool est l'index
entier.

| Pool (cible) | Taille réelle | `k` | p50 | p95 | p99 | `n` |
|---|---:|---:|---:|---:|---:|---:|
| 537 000 | 537 500 | 10 | 0,362 | 2 104,003 | 2 104,003 | 18 |
| 537 000 | 537 500 | 100 | 0,371 | 2 099,600 | 2 099,600 | 18 |
| 1 000 000 | 1 075 000 | 10 | 0,442 | 3 326,794 | 3 326,794 | 18 |
| 1 000 000 | 1 075 000 | 100 | 0,438 | 3 377,452 | 3 377,452 | 18 |
| 2 150 000 | 2 150 000 | 10 | 0,255 | 6 007,675 | 6 007,675 | 18 |
| 2 150 000 | 2 150 000 | 100 | 0,250 | 6 207,242 | 6 207,242 | 18 |
| 7 740 000 | 7 740 000 | 10 | 0,569 | **21 617,472** | 21 617,472 | 18 |
| 7 740 000 | 7 740 000 | 100 | 0,619 | **21 665,288** | 21 665,288 | 18 |

Les 18 échantillons mêlent six requêtes : trois portent sur `large`, `passage`
ou les deux (`df` = 100 %) et trois sur des nombres (`df` faible). Le p50 est donc
celui des requêtes numériques (moins d'une milliseconde), et le p95 celui des
requêtes à liste pleine. C'est **volontairement** le pire cas : une requête dont
tous les passages portent le terme. La lecture honnête est double : le **coût du
parcours** est constant — la liste d'inversion fait 7 740 000 documents et elle
est parcourue entièrement quel que soit le pool —, mais le **coût total croît
avec le pool** (2,10 s à 537 500, 21,62 s à 7 740 000), parce que la lecture des
clés et le tri final portent sur les candidats du pool (§ 5.2). `k` ne déplace
pas ce total de façon significative. Un mot courant d'un corpus réel a un `df`
bien plus faible ; c'est pourquoi la latence réelle à grand pool reste renvoyée
à P9.

### 5.2 Évolution avec la taille du pool

2,10 s (537 k) → 3,33 s (1,075 M) → 6,01 s (2,15 M) → 21,62 s (7,74 M). La
croissance est **sous-linéaire en pool** (×10,3 de temps pour ×14,4 de pool),
parce que, pour une requête à mot, la liste parcourue est la même dans tous les
cas (`df` = 100 % de 7,74 M) : ce qui augmente avec le pool, c'est le nombre de
candidats **dans le pool**, donc la lecture des clés et le tri final. Le parcours,
lui, est constant.

### 5.3 Série froide

Copie des fichiers par le harnais avec `F_NOCACHE` en lecture et en écriture
(17 913 150 342 octets ; la commande écrit son compte dans
`data/mes06/raw/copy-nocache.txt`, 20,5 s au rejeu de la correction, 25,0 s lors
de la campagne — le nombre d'octets est identique, la copie porte sur le même
arbre), puis **une passe sans préchauffage** dans un processus neuf, sur le plus
grand pool, `k = 10`.

| Point | `k` | p50 | p95 | p99 | `n` |
|---|---:|---:|---:|---:|---:|
| Froid, pool 7 740 000 | 10 | 1,315 | 22 840,205 | 22 840,205 | 6 |

La borne froide est **+5,7 %** au-dessus de la p95 chaude (22 840 ms contre
21 617 ms) pour la requête à liste pleine, et 2,3 fois la p50 chaude pour les
requêtes numériques (1,315 ms contre 0,569 ms). Le froid ne change donc pas la
conclusion : la branche lexicale coûte ce que coûte le parcours des listes, pas
la lecture disque, dont la part reste sous 6 % même sans cache.

## 6. Constat block-WAND

### 6.1 Candidats visités contre taille du pool

`docs/03:91` se demande si l'élagage block-WAND s'applique au collecteur qui
filtre sur le pool. La réponse est **non** : le collecteur maison est piloté par
`Weight::for_each`, pas par `Weight::for_each_pruning` (tantivy 0.27,
`src/collector/mod.rs:173-205`). Le nombre de candidats visités est donc le
nombre de **documents correspondants**, indépendamment du pool et de `k`.

| Index | Pool (taille) | `k` | Correspondants (min–max) | Visités | Ordre identique | Clés identiques | Scores identiques |
|---|---|---:|---:|---:|---:|---:|---:|
| `articles` | 1 027 | 10 | 3 135 – 46 241 | 3 135 – 46 241 | 18/20 | 20/20 | 20/20 |
| `articles` | 12 247 | 10 | 3 135 – 46 241 | 3 135 – 46 241 | 17/20 | 19/20 | 20/20 |
| `articles` | 50 000 | 10 | 3 135 – 46 241 | 3 135 – 46 241 | 20/20 | 20/20 | 20/20 |
| `decisions` | 997 | 10 | 15 160 – 45 884 | 15 160 – 45 884 | 18/20 | 20/20 | 20/20 |
| `decisions` | 9 690 | 10 | 15 160 – 45 884 | 15 160 – 45 884 | 19/20 | 20/20 | 20/20 |
| `decisions` | 49 998 | 10 | 15 160 – 45 884 | 15 160 – 45 884 | 19/20 | 19/20 | 20/20 |
| `jorf` | 1 905 | 10 | 1 389 – 40 781 | 1 389 – 40 781 | 10/12 | 11/12 | 12/12 |
| `jorf` | 9 877 | 10 | 1 389 – 40 781 | 1 389 – 40 781 | 10/12 | 12/12 | 12/12 |
| `jorf` | 50 000 | 10 | 1 389 – 40 781 | 1 389 – 40 781 | 9/12 | 12/12 | 12/12 |
| `sarde` | 863 | 10 | 606 – 25 546 | 606 – 25 546 | 7/9 | 9/9 | 9/9 |
| `dole` | 4 428 | 10 | 1 702 – 7 426 | 1 702 – 7 426 | 8/8 | 8/8 | 8/8 |
| `dole` | 7 623 | 10 | 1 702 – 7 426 | 1 702 – 7 426 | 8/8 | 8/8 | 8/8 |
| `titres` | 6 499 | 10 | 1 001 – 21 022 | 1 001 – 21 022 | 4/8 | 8/8 | 8/8 |
| `titres` | 11 946 | 10 | 1 001 – 21 022 | 1 001 – 21 022 | 4/8 | 7/8 | 8/8 |
| `titres` | 50 000 | 10 | 1 001 – 21 022 | 1 001 – 21 022 | 3/8 | 7/8 | 8/8 |
| `synth-large` | 7 740 000 | 10 | 7 740 000 | **7 740 000** | 3/3 | 3/3 | 3/3 |

Deux exemples suffisent : sur `articles`, une requête dont 46 241 passages
correspondent en visite **46 241** pour un pool de 1 027, soit 45 fois le pool ;
sur `synth-large`, chaque requête visite **7 740 000** candidats pour n'en garder
10. Le nombre de visites est **strictement égal** au nombre de correspondants sur
les 205 comparaisons de l'échantillon.

Ce que cela implique : la branche lexicale paie le **parcours complet des listes
d'inversion** à chaque requête, et le filtrage par pool ne réduit ce coût qu'en
supprimant le travail **après** le parcours (lecture de clé, tas de `k`). C'est
cohérent avec les latences du § 4 (indépendantes du pool) et avec le § 5
(croissance sous-linéaire en pool).

### 6.2 Ancrage de la réplique

La réplique du collecteur rend, sur les 205 comparaisons de l'échantillon et les
3 de `synth-large` : **scores identiques 208/208**, **ensemble de clés identique
203/208**, **ordre identique 177/208**. Les écarts sont exactement l'ambiguïté
d'égalité que `lexical::search` tranche au second passage (`crates/crible-search/
src/lexical.rs:37-47`) : quand le score du k-ième est ex æquo, le moteur relit
toutes les clés au score frontière et les départage par la clé, là où la réplique
s'arrête au premier passage. Comme le compteur de visites est indépendant de cet
ordre, l'ancrage est suffisant pour le chiffre publié, et il est déclaré.

## 7. Seuil d'alerte

### 7.1 Budget restant, par soustraction

Budget de bout en bout : **698 ms p95** (`docs/suivi.md:31`). Chaque terme est
cité de son rapport, avec sa ligne.

| Terme soustrait | Valeur | Source |
|---|---:|---|
| Encodage fp32, p95 | 55,763 ms | `docs/mesures/MES-05-latence-encodage.md:85` (passe 1 ; passe 2 : 56,361 ms) |
| Filtre bitmap `keyword`, p95 (`partie=corps`) | 0,0522 ms | `docs/mesures/MES-14-cout-filtres-intervalles.md:182` (p50 50,9 µs, p95 52,2 µs) |
| Scan binaire, pool entier, p95 | 44,132 ms | `docs/mesures/MES-02-latence-scan-binaire.md:124` |
| Rescoring livré, groupe, N = 1000, p95 | 16,05 ms | `docs/mesures/MES-03-latence-rescoring.md:1414` (14,65 à 16,05 ms) |
| Payload, k = 10 | 1,0 ms | `docs/03-recherche.md:155` |
| **Somme soustraite** | **116,997 ms** | |
| **Reste pour la branche lexicale** | **581,003 ms** | |

Le terme « filtre » est le filtre **bitmap `keyword`**, au **p95** : c'est le cas
que le moteur paie sur une clause de pool, et non un filtre d'intervalle, dont le
coût dépend du domaine. Le reste vaut **581,003 ms**, contre les « environ
560 ms » attendus par ADR-0091 : la différence vient des valeurs réellement
publiées par MES-02, MES-03 et MES-05, légèrement supérieures aux estimations du
tableau de `docs/03`. Ce reste **n'est valable que pour les rapports de cette
date** : toute nouvelle mesure d'un des cinq termes le déplace.

**Ligne de sensibilité.** Si le filtre de la requête était le pire cas
d'intervalle de MES-14 — `integer`, `annee 2000..2010`, domaine plein, p95
**216,9 ms** (`docs/mesures/MES-14-cout-filtres-intervalles.md:183`) — la somme
soustraite passerait à **333,845 ms** et le reste à **364,155 ms**. Les verdicts
du § 7.2 ne changent pas : sur les **51 points** mesurés (42 points de
l'échantillon, 8 points chauds et 1 point froid de `synth-large`), **aucun** ne
passe de « sous le reste » à « ALERTE », ni l'inverse. L'alerte est donc
insensible à ce choix de terme, et le chiffre principal reste 581,003 ms.

### 7.2 Verdicts

| Campagne | p95 retenue | Contre 581,003 ms | Verdict |
|---|---:|---:|---|
| Échantillon, `k = 10`, tous pools | 0,859 ms (`jorf`, pool 1 905) | 0,15 % | **sous le reste** |
| Échantillon, `k = 100`, tous pools | 1,163 ms (`articles`, pool 1 027) | 0,20 % | **sous le reste** |
| Échantillon, points regroupés, `k = 10` | 0,839 ms (pool 1 000) | 0,14 % | **sous le reste** |
| `synth-large`, 537 500 passages | 2 104,003 ms | 3,6 fois | **ALERTE (pire cas synthétique, à confirmer en P9)** |
| `synth-large`, 7 740 000 passages | 21 617,472 ms | 37,2 fois | **ALERTE (pire cas synthétique, à confirmer en P9)** |

Sur l'échantillon réel, **aucune alerte** : la branche lexicale consomme moins de
0,2 % du budget restant. L'alerte ne se déclenche que sur la campagne de
plomberie, dont le `df` de 100 % est un pire cas qui n'a pas de contrepartie en
production.

## 8. Conclusion

### 8.1 Ce que la mesure permet d'affirmer

1. **Taille** : l'index lexical de Legifacile occupe de l'ordre de **1 Go sans
   positions** et **2 Go avec**, extrapolé depuis 29,8 Mo et 62,5 Mo pour 257 626
   passages. La fourchette (0,3 à 2,1 Go ; 0,4 à 4,8 Go) dépend de la composition
   par structure, inconnue. Les positions multiplient le volume par 2,096 en
   moyenne (1,40 à 2,31 selon la longueur des passages).
2. **Durée de construction** : 2,63 M jetons analysés par seconde, soit 37,9 µs
   par passage de l'échantillon. Extrapolée, la reconstruction prend **325 s**
   (5 min 25 s) à la composition moyenne, entre 49 s et 762 s selon la
   composition. Les positions ajoutent 3,3 %.
3. **Latence** : la branche BM25 filtrée coûte **moins de 1,2 ms p95** sur
   l'échantillon, à tous les pools jusqu'à 50 000 passages, pour `k` = 10 et
   `k` = 100. Elle est **indépendante de la taille du pool** et **croît avec la
   longueur des listes d'inversion**.
4. **Block-WAND** : l'élagage **ne s'applique pas**. Le collecteur visite tous les
   documents correspondants (7 740 000 sur `synth-large` pour `k` = 10), quel que
   soit le pool.
5. **Commit** : 1 segment par index d'échantillon, 9 segments pour 7,74 M
   passages et 119,6 Mo ; pic RSS échantillonné de 57 à 148 Mio sur l'échantillon,
   1 470,5 Mio sur `synth-large`. Le budget de 50 MiB et un thread **n'est pas
   structurant** à cette échelle.
6. **Froid** : la borne froide dépasse la p95 chaude de 5,7 % ; le disque n'est
   pas le facteur limitant de la branche.

### 8.2 Ce que la mesure ne permet pas d'affirmer

- La **latence réelle à grand pool** sur Legifacile : `synth-large` ne donne
  qu'un pire cas synthétique. La mesure est renvoyée à P9 avec MES-07.
- La **composition de production par structure**, donc un chiffre unique de
  taille et de durée.
- Le comportement **sous concurrence** (`KEY_READ_CONNECTIONS = 1`) et pendant
  une **compaction**, hors périmètre (ADR-0091, décision 5).
- Le coût du **groupe d'égalité** comme point dédié : il n'est observé
  qu'indirectement (§ 6.2).

### 8.3 Comparaison aux cibles et propositions à valider par le propriétaire

1. **Cible de 60 s d'ADR-0047 : non tenue à composition moyenne.** La
   reconstruction extrapolée vaut 325 s (5 min 25 s) contre 60 s, soit 5,4 fois
   la cible ; elle ne la tient que si le corpus entier a le profil de `titres`
   (18,4 jetons analysés par passage au plus). La confirmation viendra de la
   première occurrence réelle en P9 (ADR-0088). **Proposition** : réviser la
   cible de 60 s, ou l'ordre des remèdes au temps de démarrage, dans un ADR à
   rédiger — la mesure ne tranche pas entre relever la cible et paralléliser
   l'écriture (`WRITER_THREADS` vaut 1 par ADR-0087).
2. **Alerte sur le pire cas synthétique.** À `df` = 100 %, la branche coûte
   21,6 s à 7,74 M passages, soit 37 fois le reste du budget ; dès 537 500
   passages elle dépasse déjà le reste. **Proposition, à ne pas coder** : si P9
   confirme qu'un terme courant dépasse le reste du budget à l'échelle de
   Legifacile, le repli d'ADR-0007 (index inversé maison par segment) devient
   l'option à instruire, avec un nouvel ADR. Rien n'est proposé avant cette
   confirmation : le pire cas mesuré n'a pas de contrepartie en production.
3. **Segments.** Le job ne fusionne pas les segments qu'il produit (le
   `IndexWriter` est abandonné après le `commit`). Une dizaine de segments est à
   attendre à 8,58 M passages. **Proposition** : vérifier en P9 que ce nombre ne
   pèse pas sur la latence de requête avant d'écrire un ADR sur la politique de
   fusion.
4. **Composition par structure.** L'écart entre les bornes de taille (×6,9 à
   ×11,4) est trop grand pour un dimensionnement. **Proposition** : obtenir la
   composition de production par structure avant P9 si une décision de
   dimensionnement doit être prise d'ici là.

## 9. Fichiers et reproductibilité

- Harnais : `crates/crible/examples/lexical_latency.rs` (modes `probe`,
  `tokens`, `size`, `latency`, `wand`, `rebuild`, `copy-nocache`).
- Scripts : `tools/bench/mes06/run.sh` (les huit étapes, dans l'ordre de la
  campagne), `tools/bench/mes06/filters*.json` (filtres gelés depuis `probe`),
  `tools/bench/mes06/mes06_analyze.py` (agrégation et artefact),
  `tools/bench/mes06/tests/test_mes06_analyze.py` (18 tests).
- Données brutes : `data/mes06/raw/` (gitignoré) — `attach-*.txt`,
  `tokens-*.json`, `size-*.json`, `probe-*.json`, `latency-*.json`,
  `rebuild-*.json`, `wand-*.json`, `copy-nocache.txt`, `analyze.txt`.
- Artefact daté : `docs/mesures/MES-06-lexical-bm25-filtre.2026-10-28.json`
  (chemins relatifs, aucun extrait de `.env.prod`). Il porte **tous** les
  chiffres publiés : tailles, jetons, latences et leurs verdicts d'alerte point
  par point (`verdicts`, 51 entrées), la reconstruction de `synth-large`
  (`synth_large.rebuild` : 14,549 s, 9 segments, 1 470,5 Mio, 119 569 302 octets)
  et le pic RSS d'attache (`attach`, 2 452 à 2 554 Mo), pour que rien de ce qui
  est affirmé ne dépende d'un répertoire gitignoré.
- `data/mes06/` pèse **17 Go** à la fin de la campagne : six index d'échantillon
  et le clone de `synth-large`. Le clone froid a été supprimé après la mesure.

Reproduction : `sh tools/bench/mes06/run.sh prepare` puis les étapes du § 1.2,
dans l'ordre. Un second passage exige de supprimer `data/mes06/indexes/` :
`index create` refuse une cible existante, et la série `true` édite
`index.json` en place.
