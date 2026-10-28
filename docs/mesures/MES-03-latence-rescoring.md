# MES-03 — Latence du rescoring fp16, à froid et à chaud

## Campagne du 2026-10-27

- **Tâche** : `MESURER-LATENCE-DENSE` (P4), mesure MES-03.
- **Commit** : `5542ac095e4404cf2a4ff294309fbf95dfe50df2`, branche `tache/MESURER-LATENCE-DENSE`.
  **Aucun code de production n'a été modifié pendant la campagne.** L'exemple
  `crates/crible/examples/dense_latency.rs`, les scripts `tools/bench/mes02/` et leur entrée pytest
  dans `tools/pyproject.toml` sont **neufs et non commités au moment de la mesure** : la campagne porte
  donc sur `5542ac0` plus ce harnais, sans changement dans `crates/*/src/`.
- **Machine** : MacBook Air M3, 8 cœurs, 24 Go, macOS 15.7.7 (24G720), APFS, **natif**, pas de conteneur.
  Charge au départ `3.28 3.53 3.59` ; `mediaanalysisd`, Lightroom, `mds_stores` et l'agent tournaient.
  La pression mémoire a fait partir des pages au swap pendant la campagne (voir MES-09) : les chiffres
  « à chaud » varient d'une exécution à l'autre, et chaque écart est déclaré ci-dessous.
- **Jeu de données** : `synth-large`, copie de mesure `data/mes02/indexes/synth-large`, génération 2,
  époque 1, **7 740 000 vivants**, 860 000 morts, un segment `seg-large-0001` de 8 600 000 vecteurs
  `f16` (`vectors.f16`, 13 209 600 000 octets). `records.sqlite` : 3,7 Go.
- **Requêtes** : les 300 vecteurs d'ADR-0077 (même matrice que MES-02, SHA-256
  `03cb2a2b3bb7951892179ee8d612b0970626f0f041fe6b3e18c2f0393452948b`).
- **Configuration** : copie de mesure avec `search.exact_threshold = 0` et `search.scan = "binary"` ;
  `candidates_per_branch = 100` (k) ; 8 fils de scan. Les candidats sont ceux du scan binaire du pool
  complet, `max(N, k)` d'entre eux, comme en production (ADR-0074).

### Protocole

Le protocole est celui d'**ADR-0077**. N ∈ {1000, 2000, 5000} ; la série chaude vient d'appeler le
vrai `ExactSearcher::rescore`, et deux répliques déclarées le décomposent : le produit scalaire seul
(`kernels::dot` sur la ligne `f16` mappée, via `FlatVectors`) et la lecture des clés seule (même
requête SQLite `SELECT doc_id, ord FROM passages WHERE iid = ?1`, sur une connexion en lecture seule).
Le froid a deux séries, parce que le moteur ne peut pas être refroidi de la même façon que ses vecteurs :

- **série A — vecteurs seuls**, conforme à ADR-0077 : une copie du fichier de vecteurs écrite avec
  `F_NOCACHE` (`copy-nocache`), ouverte en mmap, et `mincore` page par page avant chaque requête pour
  prouver la résidence. Le rescoring moteur n'est pas mesuré sur cette copie : seule la réplique du
  produit scalaire l'est, ce qui isole le coût des lectures fp16.
- **série B — moteur entier**, après déplacement du cache : lecture de fichiers non liés (13,5 Go pour
  N = 1000, 19,5 Go pour N = 2000 et N = 5000 ; liste dans `data/mes02/raw/displace-sources.txt`),
  résidence vérifiée par `mincore` à la première requête. Cette série existe parce que
  `SegmentReader::open` vérifie le sceau blake3 du fichier de vecteurs entier : ouvrir le searcher relit
  12,3 Gio et réchauffe le fichier, si bien qu'une copie non cachée ne peut pas être utilisée par le
  moteur lui-même.
  Le fichier `data/mes02/raw/displace-sources.txt` est **engendré par `run.sh`** à chaque campagne : il
  contient une seule ligne, la suite des `--displace-source <chemin>` (neuf lots Parquet de
  `data/synth-large` et dix segments de `data/mes15`) destinée à être étendue **non quotée** dans la
  commande, sous la forme `$(cat data/mes02/raw/displace-sources.txt)`. La recette le régénère donc sur
  un arbre propre ; aucune étape manuelle ne le crée.
- **série secondaire — après 10 minutes d'inactivité**, comme ADR-0077 le demande, déclarée comme
  borne basse. Elle ne l'est pas devenue : voir la conclusion.

Les documents produits depuis la correction portent le **plan** dense (`plan: "binary"`) : c'est la
porte d'ADR-0077, décision 1, et elle est donc prouvée sur ces séries. Le plan ne dépend que du pool
(7 740 000 vivants), de N et de k ; avec `exact_threshold = 0`, `plan()` ne peut rendre que `binary`
(`dense::planner.rs`), et MES-02 le vérifie par `compare` sur ce même pool ; les documents plus anciens
n'enregistrent pas le champ (écart 6).

Commandes (recette complète : `tools/bench/mes02/run.sh`) :

```
H=./target/release/examples/dense_latency
I="--data data/mes02 --index synth-large --vectors data/mes02/queries.f32"
$H rescore $I --dot-only --key-only --out data/mes02/raw/rescore-warm.json
$H rescore $I --skip-engine --key-only --out data/mes02/raw/key-only.json
$H rescore $I --skip-engine --key-only --passes 1 --out data/mes02/raw/key-only-isolated.json

mkdir -p data/mes02/cold
df -k data/mes02/cold            # 13 000 000 Kio libres exigés avant la copie (ADR-0077, décision 2)
$H copy-nocache --src …/vectors.f16 --dst data/mes02/cold/vectors-N.f16
$H rescore $I --cold --dot-only --dot-vectors-file data/mes02/cold/vectors-N.f16 \
    --ns N --passes 1 --out data/mes02/raw/rescore-cold-vectors-N.json
rm -f data/mes02/cold/vectors-N.f16
# Série B : 13.5 Go de déplacement pour N = 1000, 19.5 Go pour N = 2000 et 5000.
# La liste vient de run.sh, qui l'écrit : une ligne de drapeaux `--displace-source`.
$H rescore $I --cold --ns N --passes 1 \
    --displace-gb 13.5 $(cat data/mes02/raw/displace-sources.txt) \
    --out data/mes02/raw/rescore-cold-engine-N.json

sleep 600
$H rescore $I --cold --passes 1 --out data/mes02/raw/rescore-idle.json
```

Série chaude : 300 requêtes × 3 passages = 900 échantillons, comme la série des clés seules à trois
passages. Séries froides, série des clés seules à un passage et série secondaire : 300 requêtes × 1
passage, parce que la première requête réchauffe le fichier et que répéter ne mesurerait plus le froid.
Percentiles par **rang le plus proche** (ADR-0077) ; `mes02_analyze.py` les recalcule et refuse un
désaccord avec le résumé du harnais.

Écarts de protocole, déclarés :

1. **L'ordre des mesures biaise la décomposition.** Dans une même exécution, le rescoring moteur passe
   avant les répliques : il paie les défauts de page, et les répliques relisent ce qu'il vient de
   charger. Le produit scalaire « chaud » (0,36 ms) et la lecture des clés « chaude » (4,0 ms) sont donc
   des bornes **basses**. Deux contrôles mesurent la même lecture des clés **sans le moteur devant
   elle** (ordre d'exécution seulement : aucune mesure de résidence n'est faite dans ces séries, elles
   ne sont pas « froides ») : `warm-key-only-checked`, la série versionnée à trois passages, donne
   **50,4 ms** en p50 à N = 1000, et `key-only-first`, un passage, **60,8 ms**. Les deux commandes sont
   identiques à `--passes` près (`--skip-engine --key-only`).
2. **Les trois points d'une exécution `key-only` partagent le même `records.sqlite` de 3,5 Gio
   (3,76 Go).** Dans `warm-key-only-checked`, N = 1000 paie les lectures aléatoires (50,4 ms) et les
   points suivants réutilisent des pages déjà chargées : 49,1 ms à N = 2000, 28,7 ms à N = 5000. La
   série n'est donc pas monotone en N, et seul le premier point est une borne comparable au moteur.
3. **La série B ne descend pas à zéro de résidence** : à N = 1000, 0 page sur 1 052 à la première
   requête ; à N = 2000, 15 pages sur 2 102 ; à N = 5000, **56 %** des pages étaient déjà résidentes,
   malgré 19,5 Go de déplacement. La série N = 5000 de la série B est donc **contaminée** et n'entre
   dans aucune conclusion ; elle reste dans le tableau et dans l'artefact.
4. **La série secondaire n'est pas une borne basse** : après 10 minutes sans toucher l'index, 72 % des
   pages de la première requête à N = 1000 sont résidentes (86 % à N = 5000), parce que l'ouverture du
   searcher vérifie le sceau et relit le fichier de vecteurs entier. Le protocole prévoyait cette série
   comme borne basse ; elle mesure en réalité un état intermédiaire.
5. **La série A n'a pas de chiffre moteur.** Les documents `rescore-cold-vectors-*` portent une série
   `ExactSearcher::rescore` : elle mesure le fichier de l'index, pas la copie non cachée, et vaut
   172,9 ms à N = 1000. Elle n'entre dans aucun tableau ; seuls les produits scalaires sur la copie
   comptent.
6. **Champs enregistrés après la campagne.** `plan` et `displace_gb` sont écrits par le harnais depuis
   la correction, donc présents dans `key-only.json` et `rescore-cold-vectors-1000.json` (rejoués,
   voir ci-dessous), absents des documents plus anciens ; la porte d'ADR-0077, décision 1, y est
   établie par construction (`exact_threshold = 0`, pool 7,74 M) et par les exécutions `compare` de
   MES-02. Le déplacement de la série B n'est enregistré dans aucun document de la campagne : la
   recette le fixe à 13,5 Go (N = 1000) et 19,5 Go (N = 2000 et 5000).
7. **Les fractions de résidence de la campagne gardent un dénominateur non dédupliqué.** Le harnais
   dédupliquait les pages au numérateur (`mincore` sur une suite ordonnée et dédoublonnée) mais divisait
   par `pages.len()`, doublons compris : la fraction publiée **sous-estime** donc légèrement la
   résidence. Le harnais déduplique les deux côtés depuis la correction ; les comptes dédupliqués ont
   été relevés sans nouvelle copie froide, par une exécution `--cold --skip-engine` sur la copie de
   mesure : **1 052** pages à N = 1000 (inchangé), **2 102** à N = 2000 au lieu de 2 103 et **5 288** à
   N = 5000 au lieu de 5 305. Les trois deltas sont donc de **0, 1 et 17 pages**, soit 0 %, 0,05 % et
   0,32 % du dénominateur. Sur la fraction de la première requête, l'effet maximal est celui de
   N = 5000 : 2 978 / 5 305 = 0,56136 devient 2 978 / 5 288 = 0,56316, soit **+0,0018 (0,18 point)** ;
   à N = 2000, 15 / 2 103 = 0,007133 devient 15 / 2 102 = 0,007136 (+0,000003). Les fractions du
   tableau — celles de la campagne — gardent donc le dénominateur non dédupliqué, sous-estimées d'au
   plus 0,18 point (N = 5000).

### Rejeu de la recette

La campagne entière n'a pas été rejouée (environ 105 minutes). Après la revue, trois étapes l'ont été,
avec les commandes corrigées, pour prouver que `tools/bench/mes02/run.sh` tourne sans intervention :

- **`key-only.json`**, régénéré par la commande que la recette porte désormais
  (`rescore --skip-engine --key-only`, trois passages). Le document porte `plan: "binary"` et
  900 durées de clés ; le moteur y est absent (900 `null`), ce qui est le comportement attendu de
  `--skip-engine`.
- **`rescore-cold-vectors-1000.json`**, rejoué à partir d'un `data/mes02/cold/` **absent** : la recette
  le crée (`mkdir -p`), vérifie la place disque (`df -k`, 13 000 000 Kio), copie 13 209 600 000 octets
  avec `F_NOCACHE`, mesure, puis supprime la copie. Aucune intervention manuelle. Le produit scalaire
  donne **85,0 ms** en p50, contre 93,3 ms pendant la campagne (**−8,9 %**), avec la même résidence
  (0 page sur 1 052 à la première requête). L'artefact versionné porte le rejeu. Ce rejeu avait
  enregistré `--vectors` en **chemin absolu**, seul document sur dix-huit dans ce cas : l'analyseur
  normalise désormais ce champ en chemin relatif à la racine du dépôt (et refuse un chemin hors du
  dépôt). La recette passe elle aussi des chemins relatifs au harnais — `--data`, `--index`,
  `--vectors`, `--out`, `--src`, `--dst` et les sources de déplacement — et ne garde `$ROOT` que pour
  les binaires (`$HARNESS`, `target/release/crible`) et les appels lancés depuis `tools/`. L'artefact
  reste propre parce que l'analyseur normalise de toute façon chaque chemin enregistré.
- **`rss-model-cold-1` et `-2`** (MES-09), rejoués pour que le document enregistre `passes: 0` quand
  `--skip-pass` ne fait aucune passe.

Ce qui est reproductible : les commandes, les fichiers produits, les champs de contrôle, l'ordre de
grandeur et la résidence. L'analyseur régénère les artefacts de façon déterministe depuis les documents
bruts ; les latences ne sont pas reproductibles à l'octet (le rejeu a donné 85,0 ms contre 93,3 ms pour
la série A). Elles varient de quelques pour cent à un facteur deux d'une exécution à l'autre sur cette
machine chargée (voir les écarts). La place disque, que la recette vérifie désormais par `df -k` avant
chaque copie (ADR-0077, décision 2), valait 77,7 Gio libres après le nettoyage des copies froides
(75,7 Gio à la dernière lecture : APFS en rend une partie avec du retard).

### Résultats

Millisecondes, p50 / p95 / p99. « moteur » est `ExactSearcher::rescore` ; « dot » la réplique du seul
produit scalaire ; « clés » la réplique de la seule lecture `doc_id`/`ord`. La colonne « résidence »
est la fraction moyenne des pages de vecteurs déjà résidentes au moment de la requête, mesurée par
`mincore`.

| Série | N | n | moteur | dot seul | clés seules | résidence (1re requête / moyenne) |
|---|---|---|---|---|---|---|
| chaud | 1000 | 900 | **29,757** / 90,522 / 101,514 | 0,360 / 1,281 / 1,387 | 4,013 / 5,102 / 6,365 | — (fichier mmap déjà chaud) |
| chaud | 2000 | 900 | **41,120** / 101,941 / 107,531 | 0,797 / 1,498 / 1,541 | 8,099 / 10,373 / 13,508 | — |
| chaud | 5000 | 900 | **61,075** / 186,893 / 198,765 | 2,039 / 3,226 / 3,387 | 20,060 / 24,279 / 26,384 | — |
| clés seules, moteur absent, 3 passages (`warm-key-only-checked`) | 1000 | 900 | — | — | **50,360** / 78,478 / 96,874 | — (non mesurée) |
| clés seules, moteur absent, 3 passages | 2000 | 900 | — | — | 49,097 / 94,716 / 114,952 | — |
| clés seules, moteur absent, 3 passages | 5000 | 900 | — | — | 28,700 / 62,911 / 72,863 | — |
| clés seules, moteur absent, 1 passage (`key-only-first`) | 1000 | 300 | — | — | 60,833 / 88,378 / 114,917 | — (non mesurée) |
| clés seules, moteur absent, 1 passage | 2000 | 300 | — | — | 77,887 / 108,566 / 129,475 | — |
| clés seules, moteur absent, 1 passage | 5000 | 300 | — | — | 79,326 / 155,447 / 162,573 | — |
| froid A : vecteurs seuls (`F_NOCACHE`) | 1000 | 300 | — | **84,971** / 116,734 / 123,838 | — | 0,00 / 0,18 |
| froid A : vecteurs seuls | 2000 | 300 | — | **156,358** / 246,343 / 270,628 | — | 0,00 / 0,33 |
| froid A : vecteurs seuls | 5000 | 300 | — | **177,535** / 464,141 / 515,621 | — | 0,00 / 0,57 |
| froid B : moteur entier | 1000 | 300 | **179,031** / 213,825 / 233,293 | — | — | 0,00 / 0,20 |
| froid B : moteur entier | 2000 | 300 | **288,056** / 394,448 / 435,337 | — | — | 0,01 / 0,35 |
| froid B : moteur entier, contaminé | 5000 | 300 | 258,271 / 596,938 / 660,259 | — | — | **0,56** / 0,84 |
| 10 min d'inactivité | 1000 | 300 | 117,447 / 133,932 / 154,312 | — | — | 0,72 / 0,70 |
| 10 min d'inactivité | 2000 | 300 | 159,098 / 200,636 / 209,292 | — | — | 0,76 / 0,82 |
| 10 min d'inactivité | 5000 | 300 | 245,684 / 336,887 / 348,247 | — | — | 0,86 / 0,88 |

Le détail par requête est dans `MES-03-latence-rescoring.2026-10-27.json` (durées brutes de chaque
série, dans l'ordre des requêtes, et fraction de pages résidentes requête par requête) et dans
`data/mes02/raw/`.

### Conclusion

**Le budget de `docs/03-recherche.md` n'est pas tenu.** À N = 1000, « < 1 ms à chaud » devient
**29,8 ms** pour le moteur (30 fois le budget) et « ~10 ms à froid » devient **179,0 ms** pour le moteur
entier à froid (série B, 18 fois le budget). Le **seul produit scalaire**, mesuré à froid sur des pages
réellement absentes (série A), coûte **85,0 ms** (8,5 fois le budget) ; à chaud il coûte 0,36 ms et tient
le budget, mais 2,04 ms à N = 5000, où il ne le tient plus. Les deux chiffres froids ne s'additionnent
pas : 85,0 ms est la réplique du calcul sur une copie non cachée, 179,0 ms est le moteur entier après
déplacement du cache, clés et tri compris.

Ce que la mesure établit :

- **Le coût du rescoring n'est pas le calcul, c'est la mémoire.** À N = 1000, le produit scalaire
  chaud coûte 0,36 ms et la lecture des clés 4,0 ms, pour un total moteur de 29,8 ms : 85 % du temps
  n'est ni du calcul ni la lecture telle qu'elle est mesurée à chaud. Les deux contrôles sans moteur
  devant elle montrent que la même lecture, seule, coûte **50,4 ms** (trois passages) et **60,8 ms**
  (un passage), soit plus que le moteur entier : c'est donc bien l'état du cache qui décide, pas
  l'arithmétique. Les deux lectures aléatoires en jeu sont les lignes `f16` (1 536 octets par candidat,
  768 × 2, lues à la granularité de page, 16 Kio sur Apple Silicon, dans un fichier de 12,3 Gio —
  13 209 600 000 octets) et `records.sqlite` (3,5 Gio, 3,76 Go), toutes deux hors RAM par construction
  (`docs/02`).
- **Le froid change d'ordre de grandeur pour les vecteurs et sextuple le moteur** à N = 1000 :
  0,36 → 85,0 ms pour les seuls produits scalaires (**×236**), 29,8 → 179,0 ms pour le moteur entier
  (**×6,0**). À N = 5000, le produit scalaire froid coûte 177,5 ms contre 2,04 ms à chaud (**×87**) :
  cinq fois plus de candidats ne coûtent que 2,1 fois plus de lectures froides (85,0 → 177,5 ms), parce
  que le débit des lectures aléatoires sature bien avant le calcul.
- **Le coût croît avec N, moins vite que N.** Moteur chaud : 29,8 → 61,1 ms de N = 1000 à N = 5000
  (+105 % pour ×5). Vecteurs froids : 85,0 → 177,5 ms (+109 %). Une partie du coût est fixe (le premier
  accès de chaque requête) et une partie seulement est proportionnelle aux candidats.
- **Les clés sont un coût caché du rescoring.** La réplique lit une ligne de `records.sqlite` par
  candidat ; à chaud sur les mêmes 1000 identifiants, elle prend 4,0 ms, et 50,4 à 60,8 ms quand elle
  passe sans le moteur devant elle. Les 1000 clés d'une requête sont donc 1 000 lectures aléatoires dans
  3,5 Gio, soit un ordre de grandeur au-dessus des 1 000 produits scalaires.

Ce que la mesure **ne** permet pas d'affirmer :

- la latence en conteneur (P8) ni sur le Mac mini ;
- la part exacte du produit scalaire et de la lecture des clés **dans le chiffre moteur** : les répliques
  sont mesurées après le moteur, donc à cache chaud, et l'ordre inverse n'a pas été mesuré. Un profil
  (échantillonnage) serait nécessaire pour attribuer les 25 ms inexpliquées à N = 1000 ;
- le froid d'un disque de serveur : les copies `F_NOCACHE` et le déplacement de cache mesurent APFS sur
  un SSD interne, avec 24 Go de RAM et d'autres applications actives. La résidence obtenue est
  hétérogène (0 % à 56 % selon N dans la série B), et la série N = 5000 est écartée pour cette raison ;
- l'effet d'un N supérieur à 5000 : la mesure s'arrête à 5000, borne du protocole.

**Proposition, à valider par le propriétaire** : le budget « < 1 ms » est hors d'atteinte tant que la
clé d'un candidat se lit dans `records.sqlite`. Deux pistes, qui demandent chacune un ADR avant tout
code : (a) garder en RAM, à côté des codes, la clé `doc_id#ord` de chaque vivant (ordre de grandeur :
10 à 20 octets par passage, soit 100 à 160 Mo à 8 M), ce qui supprime les 1 000 lectures aléatoires du
rescoring ; (b) n'en garder que `(doc_id, ord)` sous forme d'une colonne `u32`/`u32` indexée par iid,
la chaîne n'étant construite que pour les k résultats. La seconde est moins gourmande et suffit au
classement, qui compare le texte de la clé. La première étape reste de profiler le rescoring pour
confirmer la part des deux lectures.

## Profil du rescoring (ADR-0079, décision 1) — 2026-10-27

- **Tâche** : `REDUIRE-COUT-DES-CLES` (P4), étape 1 : le profil par échantillonnage exigé par
  ADR-0079, décision 1, **avant tout changement de code**. **Aucun code de production n'a été
  modifié** : rien dans `crates/*/src`, rien dans les tests. Le harnais
  `crates/crible/examples/rescore_profile.rs`, la recette `tools/bench/mes03/run.sh` et l'analyseur
  `tools/bench/mes03/mes03_profile_analyze.py` sont neufs ; ils ne sont pas encore commités.
- **Commit** : `1b1f28300159865b95d60bf95a93e2d97bd381e8`, branche `tache/REDUIRE-COUT-DES-CLES`
  (arbre identique à `1b1f283`, `UNIFIER-PARCOURS-SCAN` inclus).
- **Machine** : MacBook Air M3, 8 cœurs, 24 Go, macOS 15.7.7 (24G720), APFS, **natif**, pas de
  conteneur. Charge au départ `3.20 4.50 4.69`, à la fin `3.46 3.92 4.38` ; `pages free`
  4 297 au départ (67 Mio) et 65 555 à la fin (1,0 Gio) ; `mediaanalysisd`, Lightroom et
  `mds_stores` tournaient (charge de fond de MES-09). Place disque : 41,8 Gio libres avant, 41,7 Gio
  après, aucune copie écrite par cette campagne.
- **Jeu de données** : la copie de mesure `data/mes02/indexes/synth-large`, **lecture seule**,
  génération 2, époque 1, **7 740 000 vivants**, un segment `seg-large-0001` de 8 600 000 vecteurs
  `f16` (13 209 600 000 octets), `records.sqlite` de 3,76 Go. Configuration `exact_threshold = 0`
  et `scan = "binary"`, `candidates_per_branch = 100` (k), `rescore_candidates` sans effet ici
  puisque la campagne fixe N par point.
- **Requêtes** : les 300 vecteurs de `data/mes02/queries.f32` (même matrice que MES-02 et MES-03,
  SHA-256 `03cb2a2b3bb7951892179ee8d612b0970626f0f041fe6b3e18c2f0393452948b`), 768 composantes.

### Protocole

1. **Binaire avec symboles, sans toucher au manifeste** : `CARGO_PROFILE_RELEASE_DEBUG=true cargo
   build --release --example rescore_profile`. La variable d'environnement de Cargo remplace
   `debug = false` du profil `release` (`Cargo.toml` n'a aucune section `[profile]` et n'a pas été
   modifiée). Le binaire porte ses informations de debug ; `sample` nomme donc les fonctions Rust
   démanchées à côté de `sqlite3_step`, `pread` et `_platform_memmove`.
2. **Harnais** : `rescore_profile` construit le pool de production (`Pool::evaluate` avec le filtre
   vide, soit les 7 740 000 vivants), garde les `max(N, k)` candidats du **scan du moteur**
   (`dense::binary::top_k`, aucun réplica : ce sont les candidats que le rescoring reçoit
   réellement), puis répète `ExactSearcher::rescore`, **et rien d'autre**, pendant 30 s en
   parcourant les 300 requêtes dans l'ordre. La boucle est séquentielle, comme en production
   (`rescore.rs`, « the loop is sequential ») : le profileur ne lit qu'une pile à la fois et
   l'attribution est sans ambiguïté.
   Le harnais annonce sa première itération par un fichier `--ready-file` ; les itérations des deux
   premières secondes ne sont pas comptées dans ses durées (échauffement), et l'ouverture du
   searcher, qui vérifie le sceau blake3 des 12,3 Gio de vecteurs, tombe donc hors de
   l'échantillonnage.
3. **Profileur** : `/usr/bin/sample <pid> 20 -file data/mes03/sample-rescore-N.txt`, 20 secondes à
   1 ms, sur le processus du harnais. `xctrace` est inutilisable sur cette machine (il exige Xcode,
   seul Command Line Tools est installé) ; `samply` n'est pas installé ; `dtrace` exige `sudo`.
   La recette lance le harnais et attend explicitement sa fin (`wait`), donc aucune commande ne
   reste en arrière-plan après elle.
4. **Attribution** : `mes03_profile_analyze.py` reconstruit l'arbre des piles par préfixe textuel de
   l'indentation (la géométrie de `sample` : une cellule de deux caractères par niveau), calcule les
   échantillons **propres** de chaque trame (compte moins la somme des enfants), puis classe chaque
   échantillon propre par la **trame marqueur la plus profonde** de sa pile : une faute de page prise
   dans le b-tree SQLite compte comme lecture de clé, pas comme « noyau ». Les millisecondes valent
   la part de la trame multipliée par le p50 du harnais dans la même exécution.

Commandes (recette complète : `tools/bench/mes03/run.sh`, à lancer depuis la racine du dépôt) :

```
CARGO_PROFILE_RELEASE_DEBUG=true cargo build --release --example rescore_profile

H=./target/release/examples/rescore_profile
D="--data data/mes02 --index synth-large --vectors data/mes02/queries.f32"

# Ordre des candidats que le scan remet au rescoring (ADR-0079, décision 2).
$H order $D --ns 1000 --out data/mes03/order-1000.json
$H order $D --ns 5000 --out data/mes03/order-5000.json

# Profil : 30 s de rescoring, 20 s d'échantillonnage après la première itération.
$H run $D --ns 1000 --seconds 30 --out data/mes03/profile-run-1000.json \
    --ready-file data/mes03/ready.flag &
PID=$!
while [ ! -f data/mes03/ready.flag ]; do sleep 0.2; done
/usr/bin/sample "$PID" 20 -file data/mes03/sample-rescore-1000.txt
wait "$PID"
python3 tools/bench/mes03/mes03_profile_analyze.py \
    --sample data/mes03/sample-rescore-1000.txt \
    --run data/mes03/profile-run-1000.json \
    --out data/mes03/profile-1000.json
# Même bloc à N = 5000 : profile-run-5000.json, sample-rescore-5000.txt, profile-5000.json.
```

Le harnais et l'analyseur sont couverts par `cd tools && uv run --no-sync pytest bench/mes03`
(8 tests). Les profils bruts sont sous `data/mes03/` (gitignoré) ; le résumé versionné est
`MES-03-profil-rescoring.2026-10-27.json`.

### Résultats

Durées du harnais pendant l'échantillonnage (millisecondes, p50 / p95, rang le plus proche comme
ADR-0077) et volume d'échantillons. Un échantillon vaut 0,0100 ms à N = 1000 et 0,0470 ms à
N = 5000.

| N | itérations utiles | candidats | p50 | p95 | échantillons | dont hors boucle |
|---|---|---|---|---|---|---|
| 1000 | 186 | 1000 | **156,496** | 178,243 | 15 716 | 5 (0,03 %) |
| 5000 | 38 | 5000 | **739,434** | 802,181 | 15 729 | 5 (0,03 %) |

Ventilation, par poste, en part de la boucle et en millisecondes au p50 :

| Poste | N = 1000 | N = 5000 |
|---|---|---|
| lecture de la clé : pages (b-tree, `pread`) | 9 125 · **58,06 %** · **90,864 ms** | 9 942 · **63,21 %** · **467,382 ms** |
| lecture de la clé : préparation du `SELECT` | 129 · 0,82 % · 1,284 ms | 132 · 0,84 % · 6,205 ms |
| lecture de la clé : verrous (`fcntl`, mutex) | 96 · 0,61 % · 0,956 ms | 96 · 0,61 % · 4,513 ms |
| lecture de la clé : `reset` et `finalize` | 8 · 0,05 % · 0,080 ms | 10 · 0,06 % · 0,470 ms |
| lecture de la clé : décodage de ligne et reste | 70 · 0,45 % · 0,697 ms | 55 · 0,35 % · 2,586 ms |
| **lecture de la clé, total** | 9 447 · **60,11 %** · **94,071 ms** | 10 258 · **65,22 %** · **482,242 ms** |
| copie et décodage du vecteur `f16` | 6 220 · **39,58 %** · **61,937 ms** | 5 438 · **34,57 %** · **255,645 ms** |
| produit scalaire (`kernels::dot`) | 33 · 0,21 % · 0,329 ms | 17 · 0,11 % · 0,799 ms |
| tri | 1 · 0,01 % · 0,010 ms | 1 · 0,01 % · 0,047 ms |
| reste du rescoring (`reader`, `Scored`, boucle) | 10 · 0,06 % · 0,100 ms | 10 · 0,06 % · 0,470 ms |
| hors du rescoring (pilote, sortie du `main`) | 5 · 0,03 % · 0,050 ms | 5 · 0,03 % · 0,235 ms |

Trois postes demandés n'ont pas de ligne à eux, et c'est un résultat :

- **l'emprunt de connexion ne coûte presque rien de mesurable.** `ReadPool::checkout` apparaît
  **une seule fois** dans les piles de N = 5000 (0,006 %, 0,047 ms, sous le mutex du pool) et
  **jamais** à N = 1000 : le pool de lecture a autant de connexions que de fils de scan (8,
  `exact.rs:103`), une connexion est donc toujours libre et l'emprunt se réduit à un mutex et à un
  `Vec::pop`, le plus souvent en ligne dans `read_passage_key` ;
- **l'allocation de `String` pèse 0,12 % (0,189 ms) et 0,15 % (1,081 ms)**, mesurée à part
  (`key_of` hors `read_passage_key`) ;
- **le tri pèse un échantillon**, soit 0,01 ms et 0,05 ms : les 1 000 ou 5 000 scores sont triés par
  `sort_unstable_by` sur un `Vec` contigu, et la comparaison ne lit deux clés qu'en cas d'égalité
  exacte de score, rare sur des vecteurs aléatoires.

Deux contrôles internes de cohérence :

- **le profil est linéaire en N dans l'état mesuré** : 0,1565 ms par candidat à N = 1000 et
  0,1479 ms à N = 5000 (pages de clé : 0,0909 et 0,0935 ms ; vecteurs : 0,0619 et 0,0511 ms). Le
  produit scalaire seul coûte 0,33 µs par candidat à N = 1000, en accord avec les 0,36 ms de la
  réplique de MES-03 ; à N = 5000, ses 17 échantillons (0,61 à 0,99 ms à une déviation) ne
  permettent pas de retrouver les 2,04 ms de la campagne, et la réplique de MES-03 reste la
  meilleure estimation du noyau seul ;
- **le coût de la clé est un coût de lecture de pages**, pas d'appel : 9 074 des 9 125 échantillons
  « pages » à N = 1000 sont dans `pread` lui-même (99,4 %), c'est-à-dire dans l'attente du disque
  pour une page absente de `records.sqlite`, et non dans le parcours des nœuds du b-tree.

### Réponse à ADR-0079, décision 4 : par lot (a) ou nombre de lignes lues ?

**Le coût est dans le nombre de lignes lues, pas dans les N allers-retours ni dans les N emprunts de
connexion.** Les trois postes qui disparaîtraient avec une requête groupée — préparation du
`SELECT`, verrous (`fcntl` du verrou de mémoire partagée, mutex), `reset`/`finalize` — valent
ensemble **2,320 ms, soit 1,48 %** à N = 1000 et **11,188 ms, soit 1,51 %** à N = 5000, et l'emprunt
de connexion vaut 0 échantillon à N = 1000 et 1 échantillon (0,047 ms) à N = 5000. Les lectures de
pages, elles, valent 58,06 % et 63,21 %.

Le rapport par candidat le confirme : la préparation et les verrous coûtent le même nombre de
microsecondes par candidat aux deux N (0,00069 et 0,00069 échantillon par candidat pour la
préparation), c'est-à-dire un coût **par appel**, alors que les pages pèsent 0,0909 et 0,0935 ms par
candidat, c'est-à-dire un coût **par ligne**. Une requête unique pour les N iid supprimerait donc au
mieux **1,5 %** du rescoring, et laisserait intactes les 60 % qui viennent du nombre de lignes.

**Conséquence pour la décision 4 :** la condition que l'ADR pose pour (a) — « si le profil montre que
le coût est dans les N allers-retours et les N emprunts de connexion plutôt que dans le nombre de
lignes lues » — **n'est pas remplie**. (a) reste sans risque et sans effet sur les bits, mais le
profil ne le justifie pas comme premier levier ; c'est (b), la lecture des clés après le tri par
score, qui attaque les 58 à 63 % mesurés ici. Si (a) est gardé comme repli, sa borne haute est
désormais chiffrée : 2,3 ms à N = 1000 et 11,2 ms à N = 5000 dans l'état de cette campagne.

### Ordre des candidats remis au rescoring (ADR-0079, décision 2)

Le scan unique d'`UNIFIER-PARCOURS-SCAN` rend **les candidats par distance de Hamming croissante**,
et, à distance égale, par iid croissant (`Candidate::cmp`, `binary.rs`). Sur 300 requêtes :

| N | paires adjacentes | inversions de distance | inversions d'iid | pas moyen d'iid |
|---|---|---|---|---|
| 1000 | 299 700 | **0** | 5 222 (1,74 %) | +1 602 |
| 5000 | 1 499 700 | **0** | 7 011 (0,47 %) | +433 |

**L'ordre constaté n'est donc pas l'ordre croissant des iid** que demande ADR-0079, décision 2 : il
est trié par distance, et l'iid ne monte que par bouffées lorsque plusieurs candidats partagent une
distance. Les 5 222 et 7 011 inversions sont les frontières entre deux distances. Un tri par iid
reste donc nécessaire avant la boucle ; il coûte un tri de `u32` sur 1 000 ou 5 000 éléments, à
comparer au tri final déjà mesuré à un échantillon.

### Copie du vecteur `f16` et accès aléatoires (décision 2, option « ligne empruntée au mmap »)

Le poste « copie et décodage du vecteur `f16` » vaut **61,937 ms à N = 1000** (39,58 %) et
**255,645 ms à N = 5000** (34,57 %), et son coût par candidat est stable (0,0619 et 0,0511 ms). Or
les échantillons propres sont dans `SegmentReader::vector_f16` lui-même (6 219 sur 6 220 à
N = 1000) : la boucle de copie et d'élargissement y est en ligne, et `_platform_memmove` n'apparaît
que 12 fois (0,08 %). Autrement dit, **le temps n'est pas dans l'instruction de copie mais dans le
service des pages** que cette copie touche : 1 536 octets lus au hasard dans un fichier de 12,3 Gio,
une page de 16 Kio par candidat absent.

Ce que cela dit de l'option (2b), « emprunter la ligne au mmap au lieu de la copier » : elle supprime
l'allocation et la copie de 1 536 octets par candidat, dont le profil montre qu'elles ne sont pas le
poste. La borne haute de son gain est donc le coût des instructions elles-mêmes, non mesurable ici et
sans rapport avec les 62 à 256 ms observées ; la faute de page, elle, reste due à l'accès aléatoire,
que (2b) ne change pas. C'est un constat, pas une conclusion sur (2b), qui se mesure séparément comme
ADR-0079, décision 2, l'exige.

Ce que cela dit de l'accès aléatoire aux vecteurs, que `GROUPER-LECTURES-VECTEURS` (ADR-0080)
traitera : **c'est le deuxième poste, et il ne cède pas** — 61,9 ms à N = 1000, 255,6 ms à N = 5000,
soit ×4,1 pour ×5 candidats, et 34 à 40 % de la boucle. La machine ne pouvait pas le servir depuis
le cache : 67 Mio de pages libres au départ du profil, pour 12,3 Gio de vecteurs.

### Réserves

1. **L'état de la machine n'est pas celui de la campagne du 2026-10-27.** Le p50 du même
   `ExactSearcher::rescore` vaut ici **156,5 ms** à N = 1000 et **739,4 ms** à N = 5000, contre
   **29,757 ms** et **61,075 ms** pendant la campagne (facteurs 5,3 et 12,1), à code identique pour
   le rescoring (`1b1f283` ne touche pas `rescore.rs`) et pour le même index. La cause visible est
   le cache de pages : 4 297 pages libres (67 Mio) au départ, 65 555 (1,0 Gio) à la fin, avec la
   pression mémoire et le swap décrits par MES-09. **Les parts mesurées décrivent donc l'état
   froid** ; elles répondent à la question posée (où passe le temps que la campagne n'attribuait
   pas), mais elles ne se substituent pas au p50 de la campagne et ne disent pas ce que (b)
   rapportera à chaud. Un profil à chaud demanderait une machine dont le cache tient les 12,3 Gio
   de vecteurs et les 3,76 Go de `records.sqlite`.
2. **Résolution.** 1 ms d'intervalle, 20 s, 15 716 et 15 729 échantillons : un échantillon vaut
   0,0100 ms et 0,0470 ms, et un poste de 5 échantillons porte une incertitude de Poisson de ±2. Les
   trois grands postes sont stables à mieux que 1 % relatif ; le tri et le « hors boucle » sont au
   niveau du bruit et sont cités pour ne rien exclure en silence.
3. **Chaque requête n'a été servie qu'une fois.** Le budget de 30 s a permis 186 appels à N = 1000
   et 38 à N = 5000, tous sur des requêtes distinctes : la boucle n'a pas bouclé, aucune page n'a
   donc été réutilisée d'une itération à l'autre, et l'état mesuré est **un premier passage** sur ces
   requêtes, jamais un état stationnaire. C'est proche du premier passage d'une campagne (ADR-0077),
   à ceci près que la campagne sert 300 requêtes et que ce profil en sert 186 et 38.
4. **Seule la boucle de rescoring est échantillonnée** : les ensembles de candidats sont construits
   avant, par le scan du moteur, hors du champ du profil. Le profil ne dit rien du scan, ni de la
   fusion, ni de l'étape de payload.
5. **`sample` mesure du temps mural**, attente dans `pread` comprise : c'est bien ce que le budget de
   `docs/03-recherche.md` mesure, mais cela ne sépare pas le temps CPU du temps d'attente. Les
   échantillons dans `pread` (9 074 à N = 1000) sont précisément de l'attente.
6. **Le total des échantillons attribués égale le total du fil principal** (15 716 et 15 729, reste
   nul) : aucun échantillon n'est écarté ni filtré.

### Conclusion

**Ce que la campagne n'attribuait à aucun poste est, dans l'état de cache de ce profil, une lecture
aléatoire de pages : 60,1 % pour les clés et 39,6 % pour les vecteurs.** À N = 1000, sur 156,5 ms
mesurées ici, 90,9 ms sont des `pread` de pages de `records.sqlite` (58,06 %) et 61,9 ms des copies
de lignes `f16` qui prennent une faute de page (39,58 %) ; le produit scalaire vaut 0,33 ms, le tri
0,01 ms, l'allocation de `String` 0,19 ms, l'emprunt de connexion 0,05 ms au plus.
À N = 5000, les mêmes postes valent 467,4 ms (63,21 %) et 255,6 ms (34,57 %) pour un p50 de 739,4 ms.

Trois conséquences, dans l'ordre d'ADR-0079 :

- **la décision 4 est tranchée négativement pour (a)** : le coût est dans les lignes lues (60 à 65 %
  du temps), pas dans les N allers-retours ni les N emprunts (1,5 % pour les appels, un échantillon
  pour l'emprunt). (a) n'est pas le premier levier ; sa borne haute est de 2,3 ms à N = 1000 et
  11,2 ms à N = 5000 ;
- **la décision 2 reste nécessaire** : le parcours unique rend les candidats par distance de Hamming,
  avec 0 inversion de distance et 5 222 inversions d'iid sur 299 700 paires à N = 1000, donc pas dans
  l'ordre croissant des iid ;
- **la décision 2, option « ligne empruntée au mmap », ne peut pas expliquer le poste vecteur** : le
  temps est dans la faute de page, pas dans la copie (`_platform_memmove`, 0,08 %). Le premier poste
  des vecteurs relève de `GROUPER-LECTURES-VECTEURS` (ADR-0080), que ce profil confirme comme
  prioritaire : les deux lectures de pages pèsent 97,6 % de la boucle à N = 1000, ce qui réalise le
  premier point de la rubrique « À surveiller » d'ADR-0079, et (b) seul ne ramènera pas le rescoring
  sous le budget de `docs/03-recherche.md`.

**Proposition, à valider par le propriétaire** : poursuivre `REDUIRE-COUT-DES-CLES` avec les deux
mesures neutres d'ADR-0079, décision 2 (ordre des iid, puis ligne empruntée), mesurées séparément et
ensemble, puis (b) ; laisser (a) en repli, avec les bornes chiffrées ci-dessus ; et faire du profil à
chaud, sur une machine dont le cache tient l'index, une condition du rejeu de MES-03 qu'exige la
décision 6, faute de quoi le gain de (b) ne sera attribuable qu'à l'état froid.

## Profil du rescoring à chaud (ADR-0079, décision 1) — 2026-10-27

Complément de la section précédente, même tâche `REDUIRE-COUT-DES-CLES` (P4). Le profil froid
attribue le temps du rescoring dans un état où le cache ne tenait rien : son p50 valait 5,3 et 12,1
fois celui de la campagne. Or les 25,4 ms qu'ADR-0079 demande d'attribuer sont celles d'un rescoring
**à chaud**, et c'est l'état chaud qui décide de l'option (a) : plus le total baisse, plus la part du
coût fixe par candidat monte. Cette section mesure donc le même rescoring après des passes
d'échauffement non mesurées sur les mêmes requêtes, dans les conditions de la série chaude
d'ADR-0077.

**Aucun code de production n'a été modifié** : rien dans `crates/*/src`, rien dans les tests. Trois
fichiers sont neufs ou étendus, tous hors production : le harnais
`crates/crible/examples/rescore_profile.rs` gagne `--warmup-passes P` et un compte de candidats
distincts, la recette `tools/bench/mes03/run-warm.sh` est neuve, et
`tools/bench/mes03/mes03_profile_artifact.py` assemble désormais l'artefact versionné à partir de
toutes les séries présentes sous `data/mes03/`.

- **Commit** : `1b1f28300159865b95d60bf95a93e2d97bd381e8`, branche `tache/REDUIRE-COUT-DES-CLES`.
- **Machine** : MacBook Air M3, 8 cœurs, **24 Go**, macOS 15.7.7 (24G720), APFS, **natif**, pas de
  conteneur. Charge au départ `2.27 2.92 3.34`, à la fin `2.46 2.45 2.81` ; `pages free` 47 973 au
  départ (750 Mio) et 59 151 à la fin (924 Mio) ; **swap occupé 4 892,94 Mio au départ et
  4 884,94 Mio à la fin** sur 6 144 Mio alloués ; `mediaanalysisd`, Lightroom et `mds_stores`
  tournaient, comme pendant le profil froid. Place disque : 45 233 560 Kio libres avant,
  45 009 696 Kio après, aucune copie écrite par cette campagne.
- **Jeu de données et requêtes** : identiques à la section précédente — copie de mesure
  `data/mes02/indexes/synth-large` en **lecture seule**, génération 2, époque 1, 7 740 000 vivants,
  `records.sqlite` de 3 758 346 240 octets (**page SQLite de 4 096 octets**, relevée dans l'en-tête
  du fichier), segment de 13 209 600 000 octets, requêtes `data/mes02/queries.f32`.

### Protocole

1. **Échauffement déclaré, puis mesure.** Le harnais exécute `P` passes complètes sur les mêmes
   requêtes, sans les mesurer, et n'annonce sa première itération mesurée par `--ready-file`
   qu'**après** ces passes : `/usr/bin/sample` ne voit donc jamais l'échauffement. Le p50 de chaque
   passe d'échauffement est enregistré (`warmup_pass_ms`) : il dit à quel état l'échauffement est
   arrivé, au lieu de le supposer.
2. **Trois séries, deux états.**
   - *chaud, campagne (300 requêtes)* : les 300 requêtes d'ADR-0077, 4 passes d'échauffement à
     N = 1000 et 2 à N = 5000 (une passe de 300 requêtes à N = 5000 dure 2 à 10 minutes) ; c'est la
     série comparable aux 29,757 et 61,075 ms de la campagne ;
   - *chaud, résidente (40 requêtes)* : les 40 premières requêtes, 6 passes d'échauffement. C'est le
     plus grand sous-ensemble dont les pages tiennent effectivement dans le cache de cette machine
     (0,76 Gio, tableau ci-dessous) : c'est le seul point du profil où **aucun** poste n'est gonflé
     par un défaut de page, et il est déclaré comme tel.
3. **Profileur et attribution** : inchangés (`/usr/bin/sample` 20 s à 1 ms, attributions par
   échantillons propres et trame marqueur la plus profonde). La valeur d'un échantillon suit le p50
   de sa propre série : 0,0033 ms (N = 1000), 0,0349 ms (N = 5000) et 0,0003 ms (série résidente).
4. **Durée de boucle** : 36 s, pour que la fenêtre échantillonnée de 20 s tombe entièrement dans la
   boucle mesurée. Le harnais sert 657 itérations à N = 1000 (2,2 passes sur les 300 requêtes), 61 à
   N = 5000 (les 61 premières requêtes) et 3 973 sur les 40 requêtes de la série résidente.

Commande (recette complète : `tools/bench/mes03/run-warm.sh`, premier plan de bout en bout) :

```
CARGO_PROFILE_RELEASE_DEBUG=true cargo build --release --example rescore_profile
sh tools/bench/mes03/run-warm.sh
```

Le script enchaîne les trois séries, attend chaque harnais (`wait`), analyse chaque rapport
d'échantillonnage et réécrit l'artefact versionné. Les profils bruts sont sous `data/mes03/`
(gitignoré) ; les tests `cd tools && uv run --no-sync pytest bench/mes03` passent (17 tests, dont 9
neufs pour l'assembleur d'artefact et le report des passes d'échauffement).

### Ce que le cache doit tenir

Le harnais prend ses candidats du vrai `dense::binary::top_k` ; il compte désormais les candidats
**distincts** de chaque série (`distinct_iids`), c'est-à-dire les pages que la boucle retouche à
chaque passe. Une ligne de vecteur vaut 1 536 octets et occupe une page de 16 Kio ; une ligne de
`records.sqlite` occupe une page de 4 Kio.

| Série | candidats distincts | pages de vecteurs | pages de lignes | total |
|---|---|---|---|---|
| 300 requêtes, N = 1000 | 294 449 | 4,49 Gio | ≥ 1,12 Gio | **≥ 5,62 Gio** |
| 300 requêtes, N = 5000 | 1 364 934 | 20,83 Gio | ≥ 5,21 Gio | **≥ 26,03 Gio** |
| 40 requêtes, N = 1000 | 39 902 | 0,61 Gio | ≥ 0,15 Gio | **≥ 0,76 Gio** |

La machine a 24 Go de RAM, dont 4,89 Gio de swap occupés par d'autres processus. Deux conséquences
encadrent toute la suite : **à N = 5000, l'ensemble de travail dépasse la RAM** (26,03 Gio), donc
aucune passe d'échauffement ne peut le rendre résident ; à N = 1000, 5,62 Gio de deux fichiers se
disputent le cache, et c'est le noyau qui arbitre, pas le protocole.

### Résultats

| Série | N | requêtes | passes d'échauffement (p50 de chaque passe) | itérations | p50 | p95 | échantillons |
|---|---|---|---|---|---|---|---|
| chaud, campagne | 1000 | 300 | 4 (150,2 · 75,7 · 54,9 · 50,2) | 657 | **51,373** | 57,864 | 15 675 |
| chaud, campagne | 5000 | 300 | 2 (637,5 · 500,8) | 61 | **548,169** | 659,657 | 15 720 |
| chaud, résidente | 1000 | 40 | 6 (136,9 · 4,9 · 6,9 · 4,8 · 4,7 · 4,6) | 3 973 | **4,774** | 45,063 | 15 637 |

Les passes d'échauffement disent déjà l'essentiel : à N = 1000 sur 300 requêtes, la première passe
coûte 150,2 ms et la quatrième 50,2 ms — l'échauffement converge vers un plateau, il ne devient pas
chaud. Sur 40 requêtes, la deuxième passe tombe à 4,9 ms et y reste. À N = 5000, deux passes ne
suffisent pas à sortir de 500 ms.

Ventilation, en nombre d'échantillons · part de la boucle · millisecondes au p50 de la série :

| Poste | chaud, 300 q, N = 1000 | chaud, 300 q, N = 5000 | chaud, résidente, 40 q, N = 1000 |
|---|---|---|---|
| lecture de la clé : pages (b-tree, `pread`) | 14 587 · **93,06 %** · **47,807 ms** | 9 268 · **58,96 %** · **323,183 ms** | 10 721 · **68,56 %** · **3,273 ms** |
| lecture de la clé : préparation du `SELECT` | 286 · 1,83 % · 0,937 ms | 150 · 0,95 % · 5,231 ms | 1 509 · 9,65 % · 0,461 ms |
| lecture de la clé : verrous (`fcntl`, mutex) | 318 · 2,03 % · 1,042 ms | 124 · 0,79 % · 4,324 ms | 1 448 · 9,26 % · 0,442 ms |
| lecture de la clé : `reset` et `finalize` | 19 · 0,12 % · 0,062 ms | 15 · 0,10 % · 0,523 ms | 125 · 0,80 % · 0,038 ms |
| lecture de la clé : décodage de ligne et reste | 228 · 1,46 % · 0,747 ms | 84 · 0,53 % · 2,929 ms | 799 · 5,11 % · 0,244 ms |
| lecture de la clé : `String` et formatage | 46 · 0,29 % · 0,151 ms | 21 · 0,13 % · 0,732 ms | 233 · 1,49 % · 0,071 ms |
| **lecture de la clé, total** | 15 484 · **98,78 %** · **50,746 ms** | 9 662 · **61,46 %** · **337,006 ms** | 14 835 · **94,87 %** · **4,529 ms** |
| copie et décodage du vecteur `f16` | 109 · 0,70 % · 0,357 ms | 6 002 · **38,18 %** · 209,295 ms | 461 · 2,95 % · 0,141 ms |
| produit scalaire (`kernels::dot`) | 53 · 0,34 % · 0,174 ms | 34 · 0,22 % · 1,186 ms | 201 · 1,29 % · 0,061 ms |
| tri | 6 · 0,04 % · 0,020 ms | 3 · 0,02 % · 0,105 ms | 30 · 0,19 % · 0,009 ms |
| reste du rescoring | 10 · 0,06 % · 0,033 ms | 16 · 0,10 % · 0,558 ms | 59 · 0,38 % · 0,018 ms |
| hors du rescoring | 13 · 0,08 % · 0,043 ms | 3 · 0,02 % · 0,105 ms | 51 · 0,33 % · 0,016 ms |

Le reste non attribué est **nul** dans les trois séries (15 675, 15 720 et 15 637 échantillons
attribués), et `ReadPool::checkout` n'apparaît dans aucune des trois : l'emprunt de connexion reste
sous le seuil de l'échantillonneur, comme à froid.

L'artefact versionné `docs/mesures/MES-03-profil-rescoring.2026-10-27.json` porte désormais les cinq
points (deux froids, trois chauds) ; chaque point déclare son `state`, sa série, ses passes
d'échauffement avec leur p50 (`warmup_passes`, `warmup_pass_ms`), les candidats distincts
(`distinct_iids`) et l'ordre des candidats de sa série.

### Écart avec les 29,8 et 61,1 ms de la campagne

| N | p50 de la campagne (ADR-0077) | p50 mesuré ici | rapport | coût par candidat, campagne | ici |
|---|---|---|---|---|---|
| 1000 | 29,757 ms | 51,373 ms | **×1,73** | 29,8 µs | 51,4 µs |
| 5000 | 61,075 ms | 548,169 ms | **×8,98** | 12,2 µs | 109,6 µs |

À N = 1000, l'écart reste dans le même ordre de grandeur, mais **la composition est inversée**, et
c'est le résultat le plus utile de ce complément :

- ici, les **vecteurs** ont gagné le cache : 0,357 ms, soit 173 fois moins que les 61,937 ms du profil
  froid, et 0,357 µs par candidat pour une copie de 1 536 octets ;
- ici, les **lignes** ont perdu : 47,807 ms de `pread`, à comparer aux **50,4 ms** de la série
  `warm-key-only-checked` de la campagne, mesure de la même lecture sans le moteur devant elle, et
  aux 90,864 ms du profil froid. La lecture des clés est donc restée au niveau « hors cache » de la
  campagne (écart 5 %), alors que la lecture des vecteurs a chuté d'un facteur 173.

Autrement dit, les deux ensembles de travail (≥ 5,62 Gio) se disputent les 24 Go, et **le noyau donne
le cache à l'un ou à l'autre** : ici aux vecteurs, pendant la campagne aux lignes. C'est ce qui
explique l'écart de 1,73 sans invoquer une différence de code — le rescoring est identique, et la
série `warm-key-only-checked` de la campagne retrouve à 5 % près le coût des pages de lignes mesuré
ici.

C'est aussi ce qui identifie **les 25,4 ms non attribuées de la campagne** : sur ses 29,757 ms,
4,013 ms de clés et 0,360 ms de produits scalaires ont été mesurés *après* le moteur, donc sur des
pages chargées par lui, et il reste 25,4 ms pour l'autre lecture aléatoire, celle des vecteurs
`f16`. La campagne elle-même relève que ses répliques, mesurées après le moteur, sont des bornes
basses (`:89-96`). Cette attribution est une **inférence**, pas une mesure : la campagne n'a pas de
profil, et sa réplique de clés à 4,013 ms prouve seulement que les lignes étaient résidentes à ce
moment-là. Le profil d'aujourd'hui, lui, mesure : 93,06 % du temps est dans les pages de lignes, et
0,70 % dans les vecteurs.

À N = 5000, l'écart de ×8,98 n'est pas une différence d'état de cache à reproduire : l'ensemble de
travail y vaut **26,03 Gio pour 24 Go de RAM**, donc aucun protocole ne peut le rendre résident, et
la campagne a mesuré là un état mixte (p95 186,893 = 3,1 fois son p50) que sa propre série B déclare
« contaminée » à 56 % de résidence (`:101-104`). Le point à N = 5000 sert donc de borne
semi-résidente, pas de reproduction.

### Réponse à ADR-0079, décision 4, à chaud

La condition de l'ADR est : le coût est-il dans les N allers-retours et les N emprunts de connexion
plutôt que dans le nombre de lignes lues ? Les trois postes qu'une requête groupée supprimerait —
préparation du `SELECT`, verrous, `reset`/`finalize` — valent :

| Série | préparation + verrous + `reset` | part de la boucle | millisecondes | part du p50 de la campagne |
|---|---|---|---|---|
| chaud, 300 q, N = 1000 | 623 échantillons | **3,98 %** | **2,041 ms** | 6,9 % de 29,757 ms |
| chaud, 300 q, N = 5000 | 289 échantillons | **1,84 %** | **10,078 ms** | 16,5 % de 61,075 ms |
| chaud, résidente, N = 1000 | 3 082 échantillons | **19,71 %** | **0,941 ms** | 3,2 % de 29,757 ms |

**La condition n'est pas remplie à chaud non plus.** Par candidat, ce coût fixe vaut **0,94 à
2,04 µs**, alors qu'une page de ligne coûte 47,8 µs (300 requêtes) et 3,27 µs (série résidente) :
même dans l'état le plus favorable à (a), la lecture des pages pèse 3,5 fois plus, et la lecture des
clés reste 94,87 % de la boucle.

Ce que la mesure confirme, en revanche, c'est l'intuition qui motivait ce complément : **la part du
coût fixe double quand le cache tient** — 1,48 % à froid (section précédente), 3,98 % à chaud sur
les mêmes 300 requêtes, **19,71 %** quand tout est résident. Mais la hausse relative s'accompagne
d'une baisse absolue : dans l'état où cette part atteint 19,71 %, le p50 vaut 4,774 ms, soit **6,2
fois moins** que les 29,757 ms de la campagne. (a) ne redevient donc jamais le premier levier : sa
borne haute est de **0,9 à 2,0 ms** à N = 1000 et **4,7 à 10,1 ms** à N = 5000 (4,7 ms en
extrapolant le coût résident de 0,94 µs par candidat à 5 000 candidats, ce qui est une
extrapolation, déclarée comme telle).

Un fait neuf, qui appartient à la suite de la tâche : **même résidente, une ligne de clé coûte
3,27 µs** contre 0,14 µs pour une ligne de vecteur. À ce prix-là, ce n'est plus la page qui coûte,
c'est l'appel système `pread`, la descente du b-tree et le décodage, payés **par ligne**. C'est
pourquoi (b), qui divise le nombre de lignes lues par 20 à 50, reste le premier levier ; et c'est
aussi ce qui rend (c) et (d), qui suppriment l'appel système, plus intéressantes après le profil
qu'avant.

### Ce que représente la copie `f16` à chaud

| Série | copie et décodage (part) | par candidat | dont `_platform_memmove` |
|---|---|---|---|
| chaud, 300 q, N = 1000 | 0,357 ms (0,70 %) | 0,357 µs | 21 échantillons, 0,069 ms |
| chaud, résidente, N = 1000 | 0,141 ms (2,95 %) | 0,141 µs | 97 échantillons, 0,030 ms |
| chaud, 300 q, N = 5000 | 209,295 ms (38,18 %) | 41,88 µs | 11 échantillons, 0,384 ms |

Quand les pages des vecteurs sont résidentes, **copier et décoder un vecteur coûte 0,14 à 0,36 µs**,
et l'instruction de copie elle-même 0,03 à 0,07 ms pour 1 000 candidats. C'est la borne haute de
l'option « emprunter la ligne au mmap » (ADR-0079, décision 2, deuxième mesure) : **moins de 0,1 ms
par 1 000 candidats à chaud**, contre 61,9 ms à froid. Les 209,295 ms de la série N = 5000 ne
contredisent pas ce chiffre : 99,8 % de ce poste y est du service de pages (0,384 ms seulement dans
`_platform_memmove`), c'est-à-dire l'ensemble de travail de 20,83 Gio que la RAM ne peut pas tenir.

### Réserves

1. **Le p50 à N = 5000 n'est pas reproduit** (548,169 contre 61,075 ms, ×8,98) : l'ensemble de
   travail y vaut 26,03 Gio pour 24 Go de RAM, et 4,89 Gio de swap sont déjà occupés au départ. Le
   point est une borne semi-résidente déclarée, pas une reproduction de la campagne.
2. **Le p50 à N = 1000 est du même ordre mais de composition inversée** (×1,73) : avec les mêmes 300
   requêtes, le cache est allé aux vecteurs ici, aux lignes pendant la campagne. Aucun réglage du
   protocole ne fixe cet arbitrage ; le harnais ne mesure ni `mincore` ni la résidence des deux
   fichiers, et cette mesure manque pour dater précisément l'état des deux ensembles.
3. **La série résidente ne décrit pas les conditions de la campagne** : 40 requêtes au lieu de 300,
   pour que 0,76 Gio tiennent. Elle sert à isoler les coûts qui ne dépendent pas des défauts de page ;
   elle ne remplace pas le p50 de la campagne.
4. **Résolution** : 1 ms, 20 s. Un échantillon vaut 0,0033 ms (N = 1000), 0,0349 ms (N = 5000) et
   0,0003 ms (série résidente) ; les postes à quelques échantillons (tri, reste, hors boucle) sont
   cités pour ne rien exclure en silence, avec une incertitude de Poisson de ±2 échantillons.
5. **Le p95 de la série résidente vaut 9,4 fois son p50** (45,063 contre 4,774 ms) : même résidente,
   la boucle prend des défauts de page quand les autres processus reprennent la main. Le p50 reste
   l'estimation de l'état résident, le p95 en est la dérive.
6. **`sample` mesure du temps mural**, attente dans `pread` comprise : c'est bien ce que le budget de
   `docs/03-recherche.md` mesure, mais cela ne sépare pas le temps CPU du temps d'attente.
7. **Seule la boucle de rescoring est échantillonnée** : ni le scan, ni la fusion, ni l'étape de
   payload ne sont dans le champ.
8. **Le harnais ouvre le searcher avant l'échauffement**, donc la vérification du sceau blake3
   (12,3 Gio) est hors champ, comme dans le profil froid.

### Conclusion

**Les 25,4 ms non attribuées par la campagne sont du service de pages, et non des allers-retours :
c'est la lecture aléatoire qui perd l'arbitrage du cache.** Aujourd'hui, ce sont les lignes de
`records.sqlite` : 47,807 ms sur 51,373 ms (93,06 %) à N = 1000 pour 300 requêtes, alors que les
vecteurs, eux, sont servis par le cache (0,357 ms, soit 173 fois moins qu'à froid). Quand tout tient
en cache — 40 requêtes, 0,76 Gio — le même rescoring coûte **4,774 ms** pour 1 000 candidats, dont
4,529 ms de clés (3,273 ms rien que dans `pread`, 0,941 ms de préparation, verrous et `reset`,
0,244 ms de décodage, 0,071 ms de `String`), 0,141 ms de copie et décodage de vecteurs, 0,061 ms de
produit scalaire et 0,009 ms de tri.

Trois conséquences, dans l'ordre d'ADR-0079 :

- **la décision 4 reste tranchée négativement pour (a)** : à chaud, la préparation, les verrous et
  le `reset` valent 3,98 % (2,041 ms) à N = 1000 et 1,84 % (10,078 ms) à N = 5000, et 19,71 %
  (0,941 ms) seulement dans l'état résident, où le p50 est déjà 6,2 fois sous celui de la campagne.
  Le coût est dans les lignes lues à chaque état mesuré : 68,6 à 93,1 % de la boucle ;
- **la décision 2, option « ligne empruntée au mmap », est bornée à chaud** : 0,03 à 0,07 ms par
  1 000 candidats pour l'instruction de copie elle-même, 0,14 à 0,36 µs par candidat pour la copie et
  le décodage quand les pages sont là. Elle ne peut pas expliquer les 25,4 ms, et la mesure de la
  décision 2 doit être lue à cette aune ;
- **le fait neuf est le coût par ligne d'une clé résidente** : 3,27 µs d'appel système et de b-tree
  contre 0,14 µs pour un vecteur. (b) divise le nombre de lignes lues par 20 à 50 et reste le premier
  levier ; (c) et (d), qui suppriment l'appel système, sont plus intéressantes après ce profil
  qu'avant, et relèvent d'un arbitrage de mémoire que le propriétaire seul tranche.

**Proposition, à valider par le propriétaire** : poursuivre `REDUIRE-COUT-DES-CLES` avec (b) en
premier, garder (a) en repli avec les bornes chiffrées ci-dessus (0,9 à 2,0 ms à N = 1000, 4,7 à
10,1 ms à N = 5000), et exiger du rejeu de MES-03 qu'ADR-0079, décision 6, prévoit — en plus du
protocole d'ADR-0077 — que **l'état de pages des deux fichiers soit déclaré**, puisque ce sont
5,62 Gio à N = 1000 et 26,03 Gio à N = 5000 qui se disputent 24 Go et que c'est cet arbitrage, non
le code, qui décide du chiffre.

## Rejeu après REDUIRE-COUT-DES-CLES (ADR-0079) — 2026-10-28

- **Tâche** : `REDUIRE-COUT-DES-CLES` (P4), étape « mesures après le code ». Elle couvre les
  mesures neutres de la décision 2 (ordre des `iid`, ligne `f16` empruntée, les deux ensemble), le
  rejeu de MES-03 de la décision 6 (`base` contre `final`, N ∈ {1000, 2000, 5000}), le contrôle
  top-10 du harnais de MES-01 et le contrôle d'égalité des réponses. La sous-section « Confirmation
  de la variante livrée (ADR-0083) » ajoute, le même jour, la mesure de (b) seule, devenue la
  variante livrée ; c'est son gain qui fait foi pour la fiche.
- **Commit** : `1b1f28300159865b95d60bf95a93e2d97bd381e8`, branche `tache/REDUIRE-COUT-DES-CLES`.
  Les changements mesurés sont ceux de `@dev`, **non commités** : (2a), (2b) et (b), dans l'arbre de
  travail, sous forme de patches relatifs à `HEAD` (`data/reduire-cles/patches/stage1-2a.patch`,
  `only-2b.patch`, `stage2-2a-2b.patch`, `stage3-final.patch`). **La mesure n'a modifié ni
  `crates/*/src`, ni les tests, ni les patches** ; elle a ajouté deux exemples non suivis
  (`crates/crible/examples/rescore_ab.rs`, `rescore_check.rs`) et quatre fichiers dans
  `tools/bench/mes03/` (`ab_round.py`, `residency.py`, `mes01_top10_check.py`,
  `mes03_ab_analyze.py`, plus leurs tests). (a), les clés par lot, **n'est pas mesurée** : le profil
  du 2026-10-27 a tranché sa condition négativement, et la fiche la laisse en repli. La
  sous-section de confirmation est mesurée depuis `31c3a7a`, commit de documentation seul dont le
  code est identique à `1b1f283`.
- **Machine** : MacBook Air M3, 8 cœurs, **24 Go**, macOS 15.7.7 (24G720), APFS, **natif**, pas de
  conteneur. Au départ de la première série (23 h 40) : charge `3.11 5.06 6.90`, **pages libres
  8 637 (135 Mio)**, **swap occupé 4 877,56 Mio sur 6 144**, 40 Gio libres sur le disque. À la fin
  (01 h 57) : charge `3.87 3.31 2.83`, **pages libres 3 756 (59 Mio)**, **swap 4 845,56 Mio**,
  41 Gio libres. `mediaanalysisd`, Lightroom et `mds_stores` tournaient, comme pendant les deux
  profils. La machine est donc dans l'état de pression mémoire décrit par MES-09, et **aucun chiffre
  absolu de cette section ne se compare directement à ceux de la campagne du 2026-10-27** (voir
  « Écarts au protocole », point 1).
- **Jeu de données** : la copie de mesure `data/mes02/indexes/synth-large`, **lecture seule**,
  génération 2, époque 1, **7 740 000 vivants**, un segment `seg-large-0001` de 8 600 000 vecteurs
  `f16` (`vectors.f16`, 13 209 600 000 octets), `records.sqlite` de 3 758 346 240 octets.
  Configuration de la copie : `exact_threshold = 0`, `scan = "binary"`, `candidates_per_branch = 100`,
  `rescore_candidates = 1000` dans `index.json` (valeur non utilisée : le harnais fixe N).
- **Requêtes** : les 300 vecteurs de `data/mes02/queries.f32`, même matrice que MES-02, MES-03 et les
  deux profils (SHA-256 `03cb2a2b3bb7951892179ee8d612b0970626f0f041fe6b3e18c2f0393452948b`).

### Protocole

1. **Cinq variantes construites hors de l'arbre de travail**, comme l'A/B d'ADR-0082 :

   ```
   for v in base 2a 2b 2a-2b final; do
       mkdir -p data/reduire-cles/src/$v
       git archive HEAD | tar -x -C data/reduire-cles/src/$v
   done
   (cd data/reduire-cles/src/2a    && patch -p1 < ../../patches/stage1-2a.patch)
   (cd data/reduire-cles/src/2b    && patch -p1 < ../../patches/only-2b.patch)
   (cd data/reduire-cles/src/2a-2b && patch -p1 < ../../patches/stage2-2a-2b.patch)
   (cd data/reduire-cles/src/final && patch -p1 < ../../patches/stage3-final.patch)
   # harnais copiés dans les variantes concernées (voir « Écarts », point 4)
   for v in base 2a 2b 2a-2b; do
       (cd data/reduire-cles/src/$v && CARGO_TARGET_DIR=../target-$v \
           cargo build --release --example dense_latency --example rescore_ab)
   done
   (cd data/reduire-cles/src/base && CARGO_TARGET_DIR=../target-base \
       cargo build --release --example fidelity --example rescore_check)
   (cd data/reduire-cles/src/final && CARGO_TARGET_DIR=../target-final \
       cargo build --release --example dense_latency --example fidelity \
       --example rescore_ab --example rescore_check)
   ```

   Chaque variante a son `CARGO_TARGET_DIR` ; les binaires sont copiés dans
   `data/reduire-cles/bin/<variante>/` et les répertoires `target` supprimés après la campagne.
   Les empreintes SHA-256 des binaires mesurés sont dans l'artefact versionné, bloc `binaries`.

2. **Harnais de latence `rescore_ab measure`**, neuf, non suivi avant cette campagne. Il construit le
   pool de production (`Pool::evaluate` avec le filtre vide, les 7 740 000 vivants) et les
   **ensembles de candidats du moteur lui-même** (`dense::binary::top_k`, `max(N, k)`), puis
   chronomètre **uniquement** `ExactSearcher::rescore`, appelé comme la production l'appelle
   (`planner::candidates`, `rescore(…, k)` avec `k = 100`). Un cycle de 300 requêtes est une
   **passe** ; le document porte chaque durée, les percentiles par passe et les percentiles de la
   série, par **rang le plus proche** (ADR-0077).

   ```
   B=data/reduire-cles/bin
   for n in 1000 5000; do
     $B/base/rescore_ab measure --data data/mes02 --index synth-large \
        --vectors data/mes02/queries.f32 --ns $n --passes 3 \
        --out data/reduire-cles/raw/ab-r1-base-$n.json
   done
   ```

   La série est **chaude** au sens d'ADR-0077 : 300 requêtes, 3 passes, aucun échauffement séparé,
   et chaque passe est reportée. Chaque point a **deux invocations** (deux rondes), avec **l'ordre
   des variantes alterné** entre les rondes :

   ```
   python3 tools/bench/mes03/ab_round.py --round 1 --order base,2a,2b,2a-2b \
       --ns 1000,5000 --passes 3 --tag ab \
       --vectors-file data/mes02/indexes/synth-large/epochs/1/segments/seg-large-0001/vectors.f16 \
       --records-file data/mes02/indexes/synth-large/epochs/1/records.sqlite
   python3 tools/bench/mes03/ab_round.py --round 2 --order 2a-2b,2b,2a,base \
       --ns 1000,5000 --passes 3 --tag ab \
       --vectors-file …/vectors.f16 --records-file …/records.sqlite
   ```

   Le rejeu de MES-03 suit le même protocole, avec `base` et `final` et N ∈ {1000, 2000, 5000} :

   ```
   python3 tools/bench/mes03/ab_round.py --round 1 --order base,final \
       --ns 1000,2000,5000 --passes 3 --tag rep --vectors-file … --records-file …
   python3 tools/bench/mes03/ab_round.py --round 2 --order final,base \
       --ns 1000,2000,5000 --passes 3 --tag rep --vectors-file … --records-file …
   ```

   Six passes par point, trois dans chaque ordre. Les bruts sont sous `data/reduire-cles/raw/`
   (`ab-r<ronde>-<variante>-<N>.json`, `rep-r<ronde>-<variante>-<N>.json`) ; gitignoré.

3. **État des pages des deux fichiers**, déclaré avant et après **chaque** invocation, comme la
   proposition de la section précédente le demande. `tools/bench/mes03/residency.py` mappe le
   fichier en lecture seule et interroge `mincore` page par page ; la fraction est **celle du
   fichier entier** (806 250 pages de 16 Kio pour `vectors.f16`, 917 565 pages de 4 Kio pour
   `records.sqlite`), et non celle des seules pages que la requête touche. Les fractions
   « avant » datent d'**avant** l'ouverture du searcher ; or celle-ci vérifie le sceau blake3 des
   12,3 Gio de vecteurs, donc relit le fichier entier avant la première requête.

4. **Verdict.** La règle est celle qu'ADR-0082, décision 3, écrit pour un A/B de latence : on
   compare le **p50 combiné** des passes de la variante à celui de `base` ; l'écart ne compte que
   s'il **dépasse l'amplitude de ses propres écarts d'une passe à l'autre**, mesurée entre les deux
   rondes au même indice de passe (ordre alterné). Dans cette amplitude, le point est un **match
   nul** ; au-delà, c'est un gain ou une perte selon le signe. Rien n'est filtré : les six passes de
   chaque point entrent dans le p50, y compris la première, froide.

5. **Analyse** :

   ```
   python3 tools/bench/mes03/mes03_ab_analyze.py --raw data/reduire-cles/raw \
       --out docs/mesures/MES-03-rejeu-reduire-cles.2026-10-28.json
   ```

   Les tests du harnais passent : `cd tools && uv run --no-sync pytest bench/mes03` (24 tests pour
   cette section, 27 depuis la sous-section de confirmation, dont 7 neufs pour l'analyse A/B).

### Contrôles

1. **Égalité `final` / `base`, au bit près** (ADR-0079, décision 6 ; ADR-0072, décision 3).
   `rescore_check` appelle la production (`dense::search`, `k = 100`) sur une copie en mémoire de la
   configuration où `rescore_candidates` vaut tour à tour 1000, 2000 et 5000, et écrit pour chaque
   requête la clé, `doc_id`, `ord`, le score et l'ordre. Les deux documents sont comparés champ à
   champ, le score par **son motif binaire** (`struct.pack('<f')`), pas par une tolérance.

   ```
   for v in base final; do
     data/reduire-cles/bin/$v/rescore_check --data data/mes02 --index synth-large \
       --vectors data/mes02/queries.f32 --ns 1000,2000,5000 --k 100 \
       --out data/reduire-cles/raw/check-$v-mes02-k100.json
   done
   ```

   Résultat : **0 différence** sur 300 requêtes × 3 valeurs de N × 100 résultats, soit
   **90 000 entrées** comparées, clé, score, ordre, `doc_id` et `ord` compris. Les deux variantes
   passent par `dense::search` avec `k = 100` et rendent donc 100 résultats par requête, si bien
   que la comparaison porte sur les 100 premiers dans les deux cas. La différence est en amont :
   `final` ne lit les clés que des 100 premiers candidats et des égalités qui les chevauchent
   (aucune sur ces vecteurs synthétiques). Le « rendait N » ne vaut que pour le harnais de latence
   `rescore_ab`, qui chronomètre `rescore` seul et compte `scored.len()` : c'est là que `base`
   rendait N scores et `final` cent.

2. **Top-10 du harnais de MES-01** (0 écart exigé par la fiche et par ADR-0079, décision 6).
   Ce qui est comparé : le top-10 rendu par `examples/fidelity` avec la variante `final`, `k = 10`,
   contre les **fichiers mêmes que la campagne MES-01 a enregistrés** le 2026-10-27 au commit
   `9ca3b07` (`data/mes01/engine/main/<structure>/n<N>.json`), sur une **copie** des six index de
   mesure (`data/reduire-cles/mes01-copy/`, `rescore_candidates` réécrit dans la copie). Clé, motif
   binaire du score, ordre, `doc_id` et `ord` sont comparés requête par requête.

   ```
   python3 tools/bench/mes03/mes01_top10_check.py --variant final \
       --out data/reduire-cles/raw/mes01-final.json
   python3 tools/bench/mes03/mes01_top10_check.py --variant base --ns 1000,5000 \
       --out data/reduire-cles/raw/mes01-base.json
   ```

   Résultat : **0 écart** sur les **30 points** (6 structures × 5 valeurs de N), soit
   300 réponses-requête par N et **1 500 au total** ; tous les plans enregistrés valent `binary` avec
   le `rescore_candidates` demandé. `base` donne aussi 0 écart aux deux N mesurés : la référence
   MES-01 est donc atteinte par `HEAD` comme par `final`, et le zéro n'est pas un effet de la
   variante seule.

3. **Porte de stratégie** : les `rescore_check` et les `fidelity` portent `dense_strategy = "binary"`
   pour toutes les requêtes (vérifié dans les documents), la copie portant `exact_threshold = 0`.
   Le harnais de latence, lui, ne passe pas par le planificateur : il prend les candidats du scan du
   moteur et chronomètre le rescoring, comme la campagne du 2026-10-27.

### Écarts au protocole

1. **L'état de la machine n'est pas celui de la campagne, ni celui du profil à chaud.** `base`
   mesure **64,7 ms** à N = 1000 et **539,5 ms** à N = 5000, contre 29,757 et 61,075 ms pendant la
   campagne (**×2,2** et **×8,8**) et 51,373 et 548,169 ms pour le profil à chaud. Le swap est
   occupé à 4,85 Gio, les pages libres tombent à 135 Mio au départ, et `records.sqlite` n'est
   résident qu'à **1,7 à 42 %** pendant toute la campagne (tableau ci-dessous). Les chiffres
   absolus de cette section décrivent cet état-là ; seuls les **écarts entre variantes mesurés dans
   les mêmes rondes alternées** sont comparables entre eux.
2. **Le harnais mesure la limite de consommation de la production, pas la liste entière.** `base`
   n'a pas de paramètre de limite (`rescore(query, &candidates)`) : il lit les N clés, comme la
   production de `HEAD`. `final` reçoit `k = 100` (`planner::candidates`), lit les clés des 100
   meilleurs candidats et du groupe d'égalité qui les chevauche, et rend **exactement 100**
   résultats par requête sur ces 300 requêtes. La colonne `base` du rejeu est donc comparable à la
   méthode de la campagne ; la colonne `final` mesure la production d'après la tâche, et non plus
   « le rescoring avec N clés ». C'est le changement sous test, pas un biais.
3. **Trois passes, sans échauffement séparé.** ADR-0077 demande trois passes après échauffement ;
   la campagne du 2026-10-27 n'avait pas d'échauffement déclaré non plus. Ici, chaque invocation
   commence par une passe froide, dont le p50 vaut 2 à 3 fois celui des suivantes ; elle est
   reportée et entre dans le p50 combiné. La dispersion élevée de `final` (89,6 % à N = 1000) est
   exactement cet effet.
4. **Deux copies du harnais de latence.** `rescore_ab.rs` appelle `rescore` avec la limite de
   consommation, qui n'existe qu'à partir de (b). Pour `base`, `2a`, `2b` et `2a-2b`, la copie
   placée dans leur arbre appelle la forme à deux arguments de leur API et écrit `"limit": null` ;
   c'est la **seule** différence entre les deux sources. Le harnais de profilage non suivi
   (`rescore_profile.rs`) n'a pas servi à cette campagne : il passe `candidates.len()` comme
   limite, ce qui aurait annulé l'effet de (b) sur `final`.
5. **La résidence est celle du fichier entier**, pas celle des pages touchées par les requêtes
   (la campagne, elle, calculait la résidence des pages des candidats). Elle répond à la question
   posée — l'arbitrage du cache entre deux fichiers —, elle ne dit pas quelle page de clé a été
   servie.
6. **Aucune série froide dans cette section.** Le rejeu demandé est celui de la série chaude
   d'ADR-0077 ; le froid (`F_NOCACHE`, `mincore` par requête, série 10 min) reste celui de la
   campagne et du profil, et appartient à P8 en conteneur.
7. **Le contrôle MES-01 rejoue la campagne principale sans filtre, six structures, `k = 10` ;** il
   ne rejoue ni la variante filtrée, ni les métriques agrégées (elles ne changent pas si les top-10
   sont identiques), ni `EXACT`, dont les fichiers enregistrés servent de référence indirecte par
   les chiffres publiés.

### Résultats

**Mesures neutres d'ADR-0079, décision 2** — millisecondes, p50 / p95 combinés sur 6 passes de
300 requêtes (1 800 échantillons par point), dispersion propre et verdict contre `base` :

| Variante | N | p50 | p95 | écart vs `base` | dispersion propre | verdict |
|---|---|---:|---:|---:|---:|---|
| `base` | 1000 | 64,8 | 154,1 | — | 21,8 % | référence |
| `2a` | 1000 | 69,1 | 151,8 | **+6,6 %** | 4,7 % | **perte** |
| `2b` | 1000 | 63,6 | 163,6 | −1,8 % | 23,1 % | match nul |
| `2a-2b` | 1000 | 69,8 | 148,6 | **+7,7 %** | 5,8 % | **perte** |
| `base` | 5000 | 516,8 | 674,4 | — | 11,1 % | référence |
| `2a` | 5000 | 569,4 | 748,6 | **+10,2 %** | 4,2 % | **perte** |
| `2b` | 5000 | 509,9 | 637,6 | −1,3 % | 1,1 % | gain marginal |
| `2a-2b` | 5000 | 567,5 | 736,6 | **+9,8 %** | 3,5 % | **perte** |

p50 de chaque passe, dans l'ordre des rondes (r1 puis r2) :

| Variante | N | passes r1 | passes r2 |
|---|---|---|---|
| `base` | 1000 | 146,5 · 65,9 · 54,2 | 132,8 · 63,0 · 57,2 |
| `2a` | 1000 | 140,2 · 66,8 · 57,7 | 142,0 · 70,0 · 58,8 |
| `2b` | 1000 | 133,0 · 61,1 · 54,0 | 147,2 · 66,6 · 55,8 |
| `2a-2b` | 1000 | 135,4 · 69,7 · 65,7 | 137,7 · 67,9 · 61,7 |
| `base` | 5000 | 619,8 · 485,2 · 490,8 | 565,5 · 488,5 · 515,6 |
| `2a` | 5000 | 608,5 · 556,0 · 559,0 | 631,8 · 556,4 · 544,8 |
| `2b` | 5000 | 555,6 · 481,1 · 502,1 | 560,4 · 479,8 · 507,4 |
| `2a-2b` | 5000 | 631,5 · 547,1 · 542,5 | 648,4 · 566,4 · 553,4 |

**Rejeu de MES-03** — `base` contre `final`, 6 passes par point, deux ordres. Sous la règle de
bruit d'ADR-0082 (dispersion propre de `final` de 89,6 %, 86,4 % et 11,4 %), le verdict vaut
**`nul`** à N = 1000, **`gain`** à N = 2000 et à N = 5000 : la première passe froide élargit la
dispersion au point d'y absorber l'écart des deux plus petits N.

| N | base p50 / p95 | final p50 / p95 | gain p50 | gain p95 |
|---:|---:|---:|---:|---:|
| 1000 | 64,7 / 142,8 | **9,8 / 61,7** | **−84,9 %** | **−56,8 %** |
| 2000 | 100,4 / 286,4 | **11,9 / 82,1** | **−88,2 %** | **−71,3 %** |
| 5000 | 539,5 / 639,4 | **170,7 / 230,0** | **−68,4 %** | **−64,0 %** |

| N | passes de `base` (r1 · r2) | passes de `final` (r1 · r2) |
|---:|---|---|
| 1000 | 133,1 · 62,7 · 54,4 | 133,9 · 64,8 · 57,7 | 50,1 · 9,7 · 0,8 | 58,8 · 9,8 · 0,8 |
| 2000 | 217,9 · 98,2 · 79,0 | 238,3 · 101,0 · 71,6 | 74,9 · 12,0 · 1,2 | 64,8 · 11,8 · 1,2 |
| 5000 | 542,2 · 486,1 · 532,3 | 552,0 · 584,3 · 509,5 | 165,4 · 175,6 · 177,1 | 154,9 · 156,8 · 171,6 |

**État des pages des deux fichiers**, fraction résidente du fichier entier avant → après chaque
invocation (r1 puis r2) :

| Série | N | `vectors.f16` avant → après | `records.sqlite` avant → après |
|---|---:|---|---|
| `base` | 1000 | 0,654 → 0,606 · 0,767 → 0,583 | 0,253 → 0,304 · 0,018 → 0,290 |
| `base` | 2000 | 0,606 → 0,571 · 0,583 → 0,547 | 0,304 → 0,402 · 0,290 → 0,416 |
| `base` | 5000 | 0,571 → 0,664 · 0,547 → 0,656 | 0,402 → 0,261 · 0,416 → 0,263 |
| `final` | 1000 | 0,664 → 0,706 · 0,745 → 0,656 | 0,261 → 0,122 · 0,017 → 0,122 |
| `final` | 2000 | 0,706 → 0,741 · 0,656 → 0,693 | 0,122 → 0,122 · 0,122 → 0,122 |
| `final` | 5000 | 0,741 → 0,745 · 0,693 → 0,767 | 0,122 → 0,017 · 0,122 → 0,018 |

Sur les mesures neutres, la résidence est du même ordre : `vectors.f16` entre 0,53 et 0,69,
`records.sqlite` entre 0,12 et 0,31 selon la position dans la ronde.

L'artefact versionné `MES-03-rejeu-reduire-cles.2026-10-28.json` (47 358 octets) porte les tables
ci-dessus, chaque passe, les six passes de chaque point, la dispersion, les verdicts, les fractions
de résidence par invocation, les empreintes des binaires et le résumé des contrôles. Les documents
bruts (durées par requête, résidences) restent sous `data/reduire-cles/raw/`.

### Confirmation de la variante livrée (ADR-0083) — 2026-10-28

- **Ce qui est mesuré** : la variante **livrée** par la tâche après le retrait de (2a) et (2b),
  c'est-à-dire **(b) seule** — lecture des clés après le tri par score, pour les `k` résultats et
  les égalités de score à la frontière. ADR-0083, décision 3, demande sa mesure de confirmation
  contre `base` et contre `final`, à N ∈ {1000, 2000, 5000}, passes alternées, règle de bruit
  d'ADR-0082, résidence déclarée par série, et demande que **le gain chiffré de la tâche soit
  celui de cette variante**, pas celui de `final`.
- **Construction** : `b-only` est construit **hors de l'arbre de travail**, depuis
  `git archive 31c3a7a` puis `patch -p1 < data/reduire-cles/patches/stage4-b-only.patch`, avec les
  trois exemples non suivis copiés (`rescore_ab.rs`, `rescore_check.rs`, `rescore_profile.rs`) et
  un `CARGO_TARGET_DIR` propre (`data/reduire-cles/target-b-only`). Le patch est **strictement
  équivalent à l'arbre de travail** : `git diff -- crates/` et le fichier de patch sont identiques.
  Vérification faite aussi sur la source : `diff -r crates/crible-search/src
  data/reduire-cles/src/b-only/crates/crible-search/src` ne rend aucune différence. Après le
  formatage de `rescore_check.rs` (voir plus bas), l'exemple a été recopié et recompilé pour que
  le binaire mesuré corresponde au fichier versionné.
- **Binaires mesurés** (SHA-256) : `rescore_ab`
  `078b5f09b66eafc776931e5b27be547a3991a07f67de4e9f4aa9de99d3b6f6ae`, `dense_latency`
  `8b62b17e8c3b7858b21b6aa3df3768ec536c14e7fa0927649cf1795dc99c4e3c`, `fidelity`
  `e92e67f29cb3880cff27c2cecbfe78ddc21ef9affbb8c037385208b7421fbcb9`, `rescore_check`
  `0c746a6a48fcc7abd12a3582b96f7f3e3e3ed86e0a64d7d3f105068fc8a3e906`. Les binaires `base` et
  `final` du rejeu précédent sont réutilisés tels quels : `HEAD` a changé (`31c3a7a` au-dessus de
  `1b1f283`) mais **le code est identique**, et leurs empreintes sont inchangées dans l'artefact.
- **Protocole** : deux rondes, ordre alterné, tag `conf` : `base,final,b-only` (r1) puis
  `b-only,final,base` (r2), trois passes de 300 requêtes par point, six passes par point, aucune
  filtrée. Le harnais et la règle de verdict sont ceux de la section précédente : `rescore_ab`,
  p50 combiné contre la référence, écart compté seulement au-delà de l'amplitude des écarts de la
  variante entre les deux rondes au même indice de passe.
- **Machine** : au début (02 h 08) charge `13,08 8,18 5,20`, **pages libres 120 369 (1,9 Gio)**,
  **swap occupé 4 845,56 Mio sur 6 144**, 40 Gio libres ; à la fin (02 h 57) charge
  `2,27 2,38 2,72`, **pages libres 55 281 (864 Mio)**, **swap 4 838,50 Mio**, 40 Gio libres. Même
  état de pression mémoire que le rejeu, avec un pic de charge au démarrage de la ronde 1 (le
  `cargo build` venait de finir). **Aucun chiffre absolu de cette sous-section ne se compare à la
  campagne du 2026-10-27** ; seuls les écarts mesurés dans les mêmes rondes alternées se
  comparent entre eux.

| N | `base` p50 / p95 | `b-only` p50 / p95 | `b-only` vs `base` | dispersion | verdict |
|---:|---:|---:|---:|---:|---|
| 1000 | 65,2 / 148,8 | **10,0 / 62,6** | **−84,7 % / −58,0 %** | 3,5 % | **gain** |
| 2000 | 96,6 / 265,2 | **11,5 / 81,5** | **−88,1 % / −69,3 %** | 65,0 % | **gain** |
| 5000 | 562,1 / 636,9 | **165,2 / 217,6** | **−70,6 % / −65,8 %** | 20,0 % | **gain** |

| N | `final` p50 / p95 | `b-only` vs `final` | dispersion | verdict |
|---:|---:|---:|---:|---|
| 1000 | 10,1 / 61,5 | −1,7 % / +1,7 % | 3,5 % | match nul |
| 2000 | 11,0 / 84,8 | +4,1 % / −4,0 % | 65,0 % | match nul |
| 5000 | 168,4 / 235,4 | −1,9 % / −7,6 % | 20,0 % | match nul |

Les deux mesures retirées ne se voient donc pas : `b-only` et `final` rendent des p50 qui
diffèrent de 0,2 à 3,2 ms (0,2 ms à N = 1000, 0,5 ms à N = 2000, 3,2 ms à N = 5000), toujours
**dans la dispersion** de `b-only`. Dans les mêmes rondes,
`final` contre `base` vaut −84,5 %, −88,6 % et −70,0 % en p50 (verdict `gain`, `nul`, `gain` ; au
N = 2000 la règle de bruit rend `nul` parce que la dispersion propre de `final` atteint 156,7 %,
effet de sa première passe froide). **Le gain de `final` était celui de (b) : le retrait de (2a)
et (2b) ne coûte rien de mesurable.**

p50 de chaque passe, dans l'ordre des rondes (r1 puis r2) :

| Variante | N | passes r1 | passes r2 |
|---|---|---:|---|
| `base` | 1000 | 141,1 · 66,9 · 56,9 | 133,8 · 61,3 · 53,6 |
| `final` | 1000 | 52,5 · 10,1 · 0,8 | 57,7 · 10,2 · 0,8 |
| `b-only` | 1000 | 56,5 · 9,8 · 0,9 | 56,6 · 10,1 · 0,9 |
| `base` | 2000 | 203,7 · 95,7 · 72,3 | 224,9 · 97,3 · 75,7 |
| `final` | 2000 | 75,6 · 11,9 · 1,2 | 59,2 · 10,4 · 1,3 |
| `b-only` | 2000 | 59,3 · 12,6 · 1,4 | 66,2 · 10,5 · 1,4 |
| `base` | 5000 | 586,6 · 540,7 · 486,4 | 590,4 · 595,1 · 451,5 |
| `final` | 5000 | 183,7 · 160,2 · 166,7 | 172,0 · 173,8 · 164,3 |
| `b-only` | 5000 | 149,1 · 153,7 · 165,5 | 182,2 · 170,2 · 173,2 |

**État des pages des deux fichiers**, fraction résidente du fichier entier avant → après chaque
invocation (r1 puis r2) :

| Série | N | `vectors.f16` avant → après | `records.sqlite` avant → après |
|---|---:|---|---|
| `base` | 1000 | 0,000 → 0,555 · 0,766 → 0,594 | 0,000 → 0,286 · 0,017 → 0,295 |
| `final` | 1000 | 0,655 → 0,682 · 0,731 → 0,660 | 0,265 → 0,122 · 0,015 → 0,122 |
| `b-only` | 1000 | 0,763 → 0,656 · 0,754 → 0,680 | 0,017 → 0,122 · 0,017 → 0,122 |
| `base` | 2000 | 0,555 → 0,567 · 0,594 → 0,554 | 0,286 → 0,421 · 0,295 → 0,409 |
| `final` | 2000 | 0,682 → 0,696 · 0,660 → 0,701 | 0,122 → 0,122 · 0,122 → 0,122 |
| `b-only` | 2000 | 0,656 → 0,700 · 0,680 → 0,708 | 0,122 → 0,119 · 0,122 → 0,122 |
| `base` | 5000 | 0,567 → 0,655 · 0,554 → 0,663 | 0,421 → 0,265 · 0,409 → 0,285 |
| `final` | 5000 | 0,696 → 0,763 · 0,701 → 0,766 | 0,122 → 0,017 · 0,122 → 0,017 |
| `b-only` | 5000 | 0,700 → 0,754 · 0,708 → 0,731 | 0,119 → 0,017 · 0,122 → 0,015 |

La première invocation de la ronde 1 (`base`, N = 1000) part de **deux fichiers absents des pages**
(0,000 pour les vecteurs comme pour `records.sqlite`) : la vérification du sceau blake3 relit les
12,3 Gio, puis l'arbitrage du cache laisse les vecteurs à 0,555 et `records.sqlite` à 0,286. Les séries qui suivent retrouvent
les fractions du rejeu : vecteurs entre 0,55 et 0,77, `records.sqlite` entre 0,015 et 0,42 selon la
position dans la ronde. Les séries `b-only` et `final` laissent `records.sqlite` à
**0,015–0,017** après elles : les clés lues sont peu nombreuses, donc peu de pages restent
chaudes.

### Contrôles rejoués sur la variante livrée

1. **Égalité au bit près, `b-only` contre `base`** : `rescore_check`, `k = 100`,
   N ∈ {1000, 2000, 5000}, sur `data/mes02/indexes/synth-large`, même protocole que pour `final`
   (clé, `doc_id`, `ord`, score comparé par son motif binaire, ordre). Les trois documents
   `check-base`, `check-final` et `check-b-only` portent le **même SHA-256**
   (`113df8905edc713a82052460e5e1c9126a6fd1eb8c5e20fe645ce8e4f044ec2f`) : **0 différence sur
   90 000 entrées**, et les trois variantes sont identiques octet pour octet. `b-only` rend
   exactement 100 résultats par requête, comme `final`.
2. **Top-10 du harnais de MES-01** : `fidelity`, `k = 10`, 6 structures × 5 valeurs de N × 300
   requêtes, contre les fichiers enregistrés par la campagne du 2026-10-27 au commit `9ca3b07`,
   sur la copie des index. **0 écart sur 30 points, soit 1 500 réponses-requête** ; tous les plans
   valent `binary`. Le retrait de (2a) et (2b) ne déplace donc aucun résultat, y compris quand le
   score s'égale à la frontière du top-10.
3. **INV-06 dans l'arbre de travail** : `cargo test inv_06` passe, avec
   `inv_06_ties_are_broken_by_ascending_key` (égalités de score tranchées par la clé textuelle
   croissante, vecteurs dupliqués),
   `inv_06_synth_small_200_queries_forced_exact_match_the_p1_reference` et
   `inv_06_synth_small_200_queries_match_the_numpy_reference`. Les tests de la tâche
   (`reduire_cout_des_cles.rs`) ne sont pas filtrés par `inv_06` ; ils passent avec la suite
   complète de `@dev`, hors de cette mesure.
4. **Formatage** : `crates/crible/examples/rescore_check.rs` était signalé par `cargo fmt --check`
   (l. 86) ; il a été reformaté avec `rustfmt --edition 2024 --config-path rustfmt.toml` sur ce
   seul fichier, et `cargo fmt --check` est vert sur tout le dépôt. Aucun autre fichier n'a été
   touché par cette correction.

**Ce que cette sous-section ne permet pas d'affirmer** : la latence en conteneur ou sur le
Mac mini ; le comportement sur un jeu réel où les égalités de score à la frontière dépasseraient
largement `k` — les 300 requêtes synthétiques n'en produisent aucune, et le cas n'est couvert que
par le test `inv_06_ties_are_broken_by_ascending_key` ; et le budget de `docs/03-recherche.md:144`,
atteint par la troisième passe seule (0,9 ms à N = 1000) et non par le p50 combiné (10,0 ms).

### Conclusion

**Le critère de la fiche est tenu, et par la variante livrée : le rescoring de `b-only`, (b) seule,
est plus rapide que `base` à N = 1000 et à N = 5000, de 84,7 % et 70,6 % en p50, de 58,0 % et
65,8 % en p95, dans les deux ordres de ronde** (sous-section « Confirmation de la variante livrée
(ADR-0083) »). À N = 2000, le gain vaut 88,1 % en p50 et 69,3 % en p95. `b-only` ne se distingue
pas de `final` (−1,7 %, +4,1 % et −1,9 %, trois matchs nuls) : **le gain du rejeu était bien celui
de (b), et le retrait de (2a) et (2b) ne coûte rien de mesurable.** Ce que la mesure établit, dans
l'ordre d'ADR-0079 :

- **(b), les clés après le tri par score, fait tout le gain.** Il ne reste plus que 100 lectures de
  clés par requête au lieu de 1 000 et 5 000 : `b-only` et `final` rendent exactement 100 résultats
  sur les 300 requêtes, à chaque N, et la troisième passe tombe à **0,9 ms** à N = 1000 et 1,4 ms à
  N = 2000, quand `base` ne descend jamais sous 53,6 et 72,3 ms. À N = 5000, `b-only` plafonne à
  149,1–182,2 ms parce qu'il lit encore 5 000 lignes `f16` : la lecture des clés n'est plus le
  poste, l'accès aux vecteurs le devient, ce qui est le sujet d'ADR-0080 et non de cette tâche ;
- **(2a), le parcours par `iid` croissant, est une perte**, de 6,6 % à N = 1000 et 10,2 % à
  N = 5000, au-delà de sa propre dispersion (4,7 % et 4,2 %). La perte est reproductible dans les
  deux ordres de ronde, et `2a-2b` la reproduit à l'identique (+7,7 % et +9,8 %) : elle est portée
  par le tri, pas par la variante. Le gain de localité attendu n'apparaît pas ; la campagne
  n'attribue pas la cause (aucun profil de `2a` n'a été fait), et **le parcours par `iid` ne peut
  pas être décrit comme neutre ou gratuit** ;
- **(2b), la ligne `f16` empruntée au mmap, ne se distingue pas du bruit** : match nul à N = 1000
  (−1,8 %, pour une dispersion de 23,1 %) et gain marginal à N = 5000 (−1,3 %, soit 6,9 ms sur
  516,8, pour une dispersion de 1,1 %). Rapporté à la borne du profil à chaud — la copie et le
  décodage valent 0,14 à 0,36 µs par candidat, soit **moins de 0,1 ms par 1 000 candidats**, et
  l'instruction de copie elle-même 0,03 à 0,07 ms —, un gain de 6,9 ms pour 5 000 candidats n'est
  pas dans l'ordre de grandeur de ce que la copie coûte. **Verdict net : le gain de (2b) seul ne
  justifie pas l'`unsafe` qu'il introduit.** L'emprunt supprime une allocation et une copie par
  candidat, mais la page est de toute façon touchée par le noyau, et le profil l'avait borné ;
- **l'égalité est préservée** : 0 différence sur 90 000 entrées entre `b-only` et `base` (clés,
  scores au motif binaire, ordre, `doc_id`, `ord` ; les trois documents `base`, `final` et
  `b-only` ont le même SHA-256) et 0 écart sur les 1 500 réponses-requête du top-10 de MES-01,
  contre les fichiers enregistrés par sa campagne. INV-06 n'est donc pas seulement vert : les
  variantes rendent la même réponse sur les jeux mesurés, et `inv_06_ties_are_broken_by_ascending_key`
  couvre le cas des égalités de score que ces jeux ne produisent pas.

Ce que la mesure **ne** permet pas d'affirmer :

- la latence en conteneur, ni sur le Mac mini : les chiffres sont natifs, sur une machine à
  4,85 Gio de swap occupé, avec 0,015 à 42 % de `records.sqlite` résident selon la série ;
- ce que `b-only` gagnerait sur un jeu réel où les égalités de score à la frontière dépasseraient
  largement `k` : les 300 requêtes synthétiques n'en produisent aucune, et le cas n'est couvert que
  par le test `inv_06_ties_are_broken_by_ascending_key` ;
- la tenue du budget de `docs/03-recherche.md:144` : à N = 1000, « < 1 ms à chaud » n'est atteint que
  par la troisième passe de `b-only` (0,9 ms), pas par le p50 combiné (10,0 ms), et la machine n'est
  pas celle de P8.

**Suite donnée, hors de cette mesure** : ADR-0083 (accepté par anticipation le 2026-10-28) décide
que **(b) seule est livrée** et que (2a) et (2b) sont retirées sans trace. La présente section
en porte la mesure de confirmation. Elle ne juge pas le code livré, qui appartient à la relecture
et au suivi de la tâche.
