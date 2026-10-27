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
