# Rapport d'import — base Legifacile complète (passes A, B et C)

Rapport de la **passe A** de la tâche `IMPORTER-BASE-COMPLETE` (phase P10), selon le protocole
d'ADR-0139, décision 5. Ce n'est pas une mesure numérotée `MES-xx` de
`docs/08-invariants-et-tests.md` : la campagne donne lieu à un **rapport d'import daté**, puis à des
sections datées de MES-07 et de MES-09 (ADR-0139, décision 5, point 8).

Le rapport couvre les trois passes de la tâche, toutes le 2026-11-06 :

- **passe A, l'import** (08:50 à 13:46 UTC) : contrôle d'espace, construction de l'image, création
  des six index, attache des six lots, contrôle de chaque index ;
- **passe B, les mesures** (13:50 à 14:16 UTC) : `crible compact` sur `decisions` et `jorf`,
  MES-07 et MES-09 à l'échelle réelle, `rebuild-lexical` sur les six index ; les chiffres détaillés
  sont dans les sections datées de [MES-07](MES-07-duree-compaction.md) et de
  [MES-09](MES-09-ram-reelle.md), avec leurs JSON
  ([2026-11-06](MES-07-duree-compaction.2026-11-06.json),
  [2026-11-06](MES-09-ram-reelle.2026-11-06.conteneur.json)), et le présent rapport en donne la
  synthèse et la section « Reconstruction lexicale » ;
- **passe C, les instantanés** (14:16 à 14:32 UTC) : six instantanés hors ligne et une restauration
  de contrôle.

La campagne s'est déroulée en **deux séances**, le même jour :

1. de 08:50 à 09:18 UTC, sur le commit `ec5ec4ff447ab2c2b3dcf6b4a071f46e1ae150ca` : cinq lots
   attachés, puis **OOM de la VM** pendant l'étape lexicale d'`articles` (section « Incident ») ;
2. de 13:26 à 13:46 UTC, sur le commit `e059fdf9758092f47a19e858593c316770a3958a` : la tâche hors
   plan `CORRIGER-MEMOIRE-APRES-COMMIT` (commit `aa2fe41`) a supprimé le cache de `serde_json::Value`
   par document qui gonflait la mémoire du delta de commit. Image reconstruite, serveur relancé sur
   les mêmes volumes, `decisions` attaché, les six index contrôlés.

## Synthèse

| Point | Résultat mesuré |
|---|---|
| Index | **six**, 6 660 239 documents, **9 600 105 passages**, 0 mort |
| Comptes | **égaux à pgvector** sur les six index (section « Comptes par index ») |
| Plus longue des attaches abouties | `decisions`, **631 s** (dont 437 s jusqu'au commit, 194 s après) ; `articles` a duré 739 s mais a été tuée par l'OOM |
| Après le commit | 87 s pour `jorf`, 194 s pour `decisions` : sous la borne de `CORRIGER-BLOCAGE-APRES-COMMIT` (≤ 92 s et ≤ 227 s, `docs/adr/0139:16`) |
| RAM de la VM | **11,67 Gio** (12 529 836 032 o), inchangée pendant les trois passes |
| Pic mémoire à l'attache | `decisions`, **10,99 GiB d'`anon`** et 11,09 GiB de `memory.current`, **marge 0,58 GiB** |
| MES-07 (compaction) | `decisions` **371,0 s**, `jorf` **150,5 s**, seconde passe de `jorf` **142,1 s** ; `anon` ≤ 2,64 GiB |
| MES-09 (RAM de service) | instance **3,54 GiB d'`anon`**, plancher 1,43 GiB ; **minimum 4 Gio, confortable 12 Gio** |
| Reconstruction lexicale | 1, 5, 13, **71**, **127**, **185** s pour **1 022 144 487 jetons** : **trois index dépassent la cible de 60 s** |
| Instantanés | six, **23,73 GiB (25,48 Go)**, tous complets, 0,4 à 62,0 s |
| Espace | hôte **141 Go libres (131 GiB)** après la copie des index lexicaux et la construction de l'exemple, **144 Go (134 GiB)** à la fin des instantanés ; VM 137 G libres, `/data` 51 G |
| Incidents corrigés | OOM de la VM sur `articles` → `CORRIGER-MEMOIRE-APRES-COMMIT` (`aa2fe41`) ; blocage après le commit → `CORRIGER-BLOCAGE-APRES-COMMIT` |

Deux **tâches hors plan** ont corrigé les deux défauts que la campagne a révélés, sans changer la
conception :

1. **`CORRIGER-BLOCAGE-APRES-COMMIT`** : l'attache rendait la main trop tard après le point de
   commit. Après correction, le temps après le commit reste sous la borne — 87 s pour `jorf`,
   194 s pour `decisions` ; c'est cette correction qui rend le critère d'ADR-0139 vérifiable.
2. **`CORRIGER-MEMOIRE-APRES-COMMIT`** (`aa2fe41`) : le cache de `serde_json::Value` par document
   gonflait le delta de commit et a tué la VM sur `articles`. Après correction, `decisions`, plus
   gros lot de la campagne, passe avec 0,58 GiB de marge au lieu d'un OOM.

Les deux incidents restants — la rétention mémoire après une attache et les fusions `tantivy`
annulées — sont mesurés et **non corrigés** : ils figurent dans « Points ouverts ».

## Campagne du 2026-11-06

- **Tâche** : `IMPORTER-BASE-COMPLETE`, passe A (import). Branche `tache/IMPORTER-BASE-COMPLETE`,
  commits `ec5ec4f` (séance 1) et `e059fdf` (séance 2). **Aucun code de production n'a été modifié
  par la mesure** : seuls le harnais `tools/bench/p10-import/` et ce rapport sont nouveaux.
- **Protocole** : ADR-0139, décision 5 (préalable, conteneur et volumes, ordre des index, attache,
  contrôle par index, rapports, interdits). Les quatre points tranchés par l'ADR — contrôle
  d'espace, MES-07, MES-09, reconstruction lexicale — ne concernent que la suite de la campagne.
- **Machine** : MacBook Air M3 (`Mac15,12`), 8 cœurs (4 performance, 4 efficacité), **24 Go**
  (25 769 803 776 octets), macOS 15.7.7 (24G720).
- **VM Docker** : Docker Desktop 28.5.1, **8 vCPU**, **12 529 836 032 octets de RAM (11,67 Gio)**,
  `memory.max` du conteneur à `max` (aucune limite `--memory` : la limite liante est celle de la
  VM), disque de VM plafonné à **200 Gio** (`Docker.raw`, fichier épars de 214 748 364 800 octets),
  `df /data` dans la VM : **197 G** de taille.
- **Images** : le même nom, `crible:p10`, construit deux fois depuis `docker/Dockerfile`.
  `sha256:1b0c66abc0af…` le 2026-11-06T08:53:03Z, en **128 s**, depuis `ec5ec4f` (séance 1) ;
  `sha256:ff08d62b74d2…` le 2026-11-06T13:28:31Z, en **127 s**, depuis `e059fdf` (séance 2). Les
  cinq index attachés en séance 1 et la génération 1 d'`articles` ont été écrits par la **première**
  image ; `decisions` par la **seconde**.
- **Instance** : deux conteneurs successifs, mêmes volumes, même configuration.
  `crible-p10-server` (séance 1, image 1) est **arrêté** (`docker stop`, code 0) et conservé ;
  `crible-p10-server-a2` (séance 2, image 2) est **en marche** à la fin de la passe. Port `18090`
  sur l'hôte vers `8080`, `--stop-timeout 65`, `USER crible` (uid 1000), sans limite mémoire.
  Configuration générée par `prepare.sh` dans `data/p10-exchange/crible.toml` :
  `search_threads = 0`, `max_concurrent_searches = 0`, `query_precision = "fp32"`,
  `wait_timeout_ms = 60000`, une clé de rôle `admin`.
- **Volumes** : `crible-p10-data` sur `/data` (**volume nommé neuf**, ADR-0139 décision 5 point 2) ;
  `crible-p10-models` sur `/models`, alimenté depuis `data/models/mdenseon` (1,5 Gio) ;
  `/exchange` monté depuis `data/p10-exchange/` (gitignoré). Chaque lot est monté **directement**
  sous `/exchange/inbox/<lot>`, en lecture seule : `dole-full-v1`, `sarde-full-v1`,
  `titres-full-v2`, `jorf-full-v1`, `articles-full-v1`, `decisions-full-v1`.
- **Configuration des index** : les six fichiers de `docs/legifacile/configs/`, créés par
  `POST /v1/indexes` (ADR-0105). `reference_precision` vaut `fp16`, `lexical.enabled` vaut `true`,
  `ingest.acceptance_sample` vaut 200 et `acceptance_min_cosine` 0,999 pour les six.

## Contrôle d'espace disque

L'espace est mesuré sur l'**hôte**, la contrainte liante d'ADR-0139 (décision 1), en Go décimaux
(`df -H /`) avec les Gio (`df -h /`) publiés en regard.

- **Départ de la séance 1** (2026-11-06T08:50:21Z) : **151 Go libres (141 GiB)**, au-dessus du seuil
  de 130 Go.
- **Départ de la séance 2** (2026-11-06T13:26:22Z) : **185 Go libres (172 GiB)** ; le propriétaire a
  vidé la corbeille entre les deux séances.
- **Arrêt** : le seuil de 40 Go libres sur l'hôte n'a jamais été approché. Le minimum atteint est
  **123 Go (115 GiB)**, juste après l'OOM de la VM.

| Étape | Libre sur l'hôte | Libre dans la VM (`/data`) | `/data/indexes` |
|---|---:|---:|---:|
| `prepare.sh`, avant le conteneur (08:50) | 151 Go (141 GiB) | — | — |
| avant `dole`, conteneur démarré | 147 Go (137 GiB) | 144 G | 892 K |
| après `dole` | 147 Go (136 GiB) | 144 G | 42 M |
| après `sarde` | 146 Go (136 GiB) | 144 G | 687 M |
| après `titres` | 141 Go (132 GiB) | 140 G | 4,6 G |
| après `jorf` | 135 Go (125 GiB) | 134 G | 11 G |
| après `articles` (OOM) | 123 Go (115 GiB) | 129 G | 18 G |
| fin de la séance 1, conteneur arrêté | 128 Go (119 GiB) | 129 G | 15 G |
| reprise de la séance 2 (13:26) | 185 Go (172 GiB) | 147 G | 15 G |
| avant `decisions` (13:30) | 181 Go (169 GiB) | 147 G | 15 G |
| après `decisions` (13:41) | 166 Go (155 GiB) | 133 G | 25 G |
| fin de la passe A, serveur relancé (13:46) | 170 Go (159 GiB) | 137 G | 25 G |

Le premier bloc est le relevé de `prepare.sh` (151 Go) ; les suivants sont les blocs datés de
`data/p10-import/df-host-per-lot.txt` et `df-host-*.txt`, relevés **avant** et **après** chaque
attache. Le conteneur lui-même consomme 4 Go d'hôte entre `prepare.sh` et la première attache
(fichier de la VM). L'écart entre 123 Go après l'OOM et 128 Go après l'arrêt du conteneur n'est pas
expliqué : il n'est pas attribué, et il reste dans les chiffres bruts. Même remarque pour le
passage de 166 Go (juste après `decisions`) à 170 Go (après redémarrage) : la VM repasse de 54 G à
51 G d'occupation, alors que `du /data/indexes` ne change pas (25 G) ; l'espace rendu n'était donc
pas dans les index (voir « Contrôle par index »).

Le disque de la VM (197 G) n'est jamais devenu contraignant : il lui restait 133 G au plus fort de
la campagne.

## Comptes par index

Les comptes sont ceux de l'instantané publié (`GET /v1/indexes/{index}`) et de `crible inspect`,
comparés aux comptes de pgvector relevés par `EXPORTER-BASE-LEGIFACILE` (`docs/suivi.md`, 2026-11-05
et 2026-11-06).

| Index | Lot | Documents pgvector | Documents importés | Passages pgvector | Passages importés | Écart |
|---|---|---:|---:|---:|---:|---|
| `dole` | `dole-full-v1` | 3 600 | 3 600 | 7 653 | 7 653 | aucun |
| `sarde` | `sarde-full-v1` | 210 965 | 210 965 | 214 578 | 214 578 | aucun |
| `titres` | `titres-full-v2` | 1 477 883 | 1 477 883 | 1 477 883 | 1 477 883 | aucun |
| `jorf` | `jorf-full-v1` | 1 636 145 | 1 636 145 | 1 833 500 | 1 833 500 | aucun |
| `articles` | `articles-full-v1` | 2 135 023 | 2 135 023 | 2 476 715 | 2 476 715 | aucun |
| `decisions` | `decisions-full-v1` | 1 196 623 | 1 196 623 | 3 589 776 | 3 589 776 | aucun |
| **total** | | **6 660 239** | **6 660 239** | **9 600 105** | **9 600 105** | **aucun** |

Les six index sont **égaux à pgvector, document par document et passage par passage**, sans aucun
écart : le critère « les comptes par index sont égaux à ceux de pgvector, ou les écarts sont
expliqués » (`docs/phases/P10-validation-a-l-echelle-reelle.md:122`) est tenu, et le total de
l'instance atteint **6 660 239 documents** et **9 600 105 passages**.

**L'écart de 12 % au chiffre de `docs/02:230`.** La conception chiffre **8,58 M passages**
(`docs/02-architecture.md:230`, repris par `docs/01:9` et `:66`) ; l'export de pgvector en compte
**9 600 105**, soit **+1 020 105 passages, +11,89 %** — l'« environ 12 % » d'ADR-0139. Cet écart
n'est **pas** une perte ni un gain de l'import : chaque index importé est **exactement** égal à
pgvector, et l'écart existe donc déjà entre le chiffre de conception et le corpus réellement exporté.
Le fait mesuré s'arrête là : comptes égaux à pgvector, écart déjà présent entre la conception et
l'export. Deux attributions restent des **hypothèses, non mesurées** : que les deux plus gros index
(`decisions` 3 589 776 passages, 37,4 % du total ; `articles` 2 476 715, 25,8 %) portent l'essentiel
de l'écart, et que les versions de découpeur déclarées par les lots (`decouper` v1, `decouper_juri`
v3) changent le nombre de passages d'un même document. Aucun comptage par version de découpeur n'a
été fait. Le chiffre de 8,58 M date d'avant l'export ; le corriger dans
`docs/02:230` est une décision du propriétaire, non bloquante (ADR-0139, décision 5, point 5).

## Durées d'attache

Chaque lot est soumis par `POST /v1/indexes/{index}/batches?wait=true` ; l'appel rend **202** à
l'échéance de 60 s, puis le job est suivi par `GET /v1/jobs/{id}` toutes les 10 s (ADR-0067,
ADR-0134). Le **point de commit** est la fin de l'étape `transaction` du protocole ; le temps
« après le commit » est la somme des étapes `lexical`, `manifest` et `snapshot`. Les horodatages
bruts sont dans `data/p10-import/polls/` et `data/p10-import/server.log`.

| Index | Total du job | Jusqu'au commit | Après le commit | Attente `?wait=true` | Durée murale de la commande |
|---|---:|---:|---:|---:|---:|
| `dole` | 165 s | 165 s | 1 s | 60 s | 170 s |
| `sarde` | 69 s | 63 s | 6 s | 60 s | 70 s |
| `titres` | 101 s | 76 s | 25 s | 60 s | 110 s |
| `jorf` | 290 s | 203 s | 87 s | 60 s | 300 s |
| `articles` | 739 s* | 341 s* | 398 s* (interrompu) | 60 s | 743 s |
| `decisions` | 631 s | 437 s | 194 s | 60 s | 645 s |

`*` : bornes d'étape des sondes de job, précision de 10 s, **uniquement pour `articles`**, dont le
processus a été tué avant d'écrire sa ligne `attach durations`. Les cinq autres lignes viennent du
journal du serveur, relevées après coup avec `docker logs -t` et conservées dans
`data/p10-import/attach-durations-all.log` : `decisions` y donne `validation_ms` 282 765,
`segment_ms` 45 296, `transaction_ms` 109 129, `lexical_ms` 173 065, `manifest_ms` 4 et
`snapshot_ms` 20 867, soit 631 126 ms au total. La durée murale comprend les 60 s d'attente de
l'API, les sondes et les relevés `df` ; elle n'est pas une durée du job.

Détail par étape, en millisecondes, tel que le serveur le journalise :

| Index | `validation` | `segment` | `transaction` | `lexical` | `manifest` | `snapshot` |
|---|---:|---:|---:|---:|---:|---:|
| `dole` | 164 272 | 145 | 210 | 640 | 0 | 22 |
| `sarde` | 56 815 | 2 891 | 3 016 | 4 900 | 1 | 1 330 |
| `titres` | 36 039 | 17 724 | 21 970 | 10 993 | 1 | 14 075 |
| `jorf` | 142 165 | 23 421 | 37 904 | 66 852 | 1 | 19 862 |
| `articles` | ≈ 231 000* | ≈ 30 000* | ≈ 80 000* | ≥ 398 000 (interrompu)* | — | — |
| `decisions` | 282 765 | 45 296 | 109 129 | 173 065 | 4 | 20 867 |

Trois faits relevés, sans interprétation ajoutée :

- **Le premier lot paie la session fp32.** La `validation` de `dole` vaut 164,3 s pour 7 653
  passages, contre 56,8 s pour les 214 578 de `sarde` : la différence est le chargement de la
  session ONNX fp32 de la recette d'acceptation, faite au premier usage, et non un coût proportionnel
  au lot. Les validations suivantes varient de 36 à 142 s sans suivre la taille du lot.
- **L'étape lexicale suit les jetons plus que les passages** : 11,0 s pour `titres` (1 477 883
  passages, 23,2 M jetons), 66,9 s pour `jorf` (1 833 500 passages, 177,7 M jetons), 173,1 s pour
  `decisions` (3 589 776 passages, 494,6 M jetons), soit 0,47, 0,38 et 0,35 µs par jeton. Pour
  `articles` (2 476 715 passages), les fichiers de l'index lexical sont écrits à 09:10-09:11, mais
  l'étape n'est pas rendue : elle est tuée à 09:16:42.
- **Le temps après le commit reste sous la borne de `CORRIGER-BLOCAGE-APRES-COMMIT`** : 87 s pour
  `jorf` (borne ≤ 92 s), 194 s pour `decisions` (borne ≤ 227 s, `docs/adr/0139:16`).

## Contrôle par index

`crible inspect` a été lancé **serveur arrêté**, en lecture seule (ADR-0116), sur le volume
`crible-p10-data` ; `GET /v1/indexes/{index}` a été relevé serveur en marche. Les deux sources
concordent. **`crible inspect` se bloque si le serveur tient le volume ouvert en écriture** : lancé
pendant la séance 2 sans arrêter le serveur, il est resté bloqué plus de 120 s sans rien afficher ;
le processus a été arrêté par un `kill -TERM` depuis un conteneur auxiliaire partageant son espace de
PID, sans effet sur le serveur. Aucun message ne dit ce qu'il attendait : l'attente du verrou
d'écriture est l'explication la plus simple, elle n'est pas prouvée.

| Index | Époque | Génération | Documents | Passages vivants | Morts | Segments | Segment unique | `sizes.disk_bytes` |
|---|---:|---:|---:|---:|---:|---:|---|---:|
| `dole` | 1 | 1 | 3 600 | 7 653 | 0 | 1 | `seg-dole-full-v1` | 28 628 834 |
| `sarde` | 1 | 1 | 210 965 | 214 578 | 0 | 1 | `seg-sarde-full-v1` | 520 940 919 |
| `titres` | 1 | 1 | 1 477 883 | 1 477 883 | 0 | 1 | `seg-titres-full-v2` | 3 320 251 868 |
| `jorf` | 1 | 1 | 1 636 145 | 1 833 500 | 0 | 1 | `seg-jorf-full-v1` | 4 859 655 373 |
| `articles` | 1 | 1 | 2 135 023 | 2 476 715 | 0 | 1 | `seg-articles-full-v1` | 7 411 464 325 |
| `decisions` | 1 | 1 | 1 196 623 | 3 589 776 | 0 | 1 | `seg-decisions-full-v1` | 10 293 631 290 |

Ces relevés sont ceux de la **passe A**, juste après l'attache. Après les compactions de la passe B,
`decisions` est en époque 2 (génération 2, 4 segments, 10 252 383 506 octets) et `jorf` en époque 3
(génération 3, 2 segments, 4 964 150 023 octets) ; la section « Instantanés » donne les tailles
copiées, et « État laissé » l'état final.

Chaque index n'a **qu'un segment source**, comme attendu d'un lot `replace_all` unique
(`MES-07:164-166`) ; le ratio de morts vaut **0** partout. Les six index totalisent
**26 434 572 609 octets** de `sizes.disk_bytes`, soit 24,6 GiB ; `du` donne 25 G pour
`/data/indexes`. Le volume `/data` pèse 51 G avec les fichiers de Docker dans la VM.

**Un écart de mesure, conservé.** Le `disk_bytes` de `decisions` relevé **juste après l'attache**
valait **13,31 GiB** ; après l'arrêt du serveur, le même relevé donne **10 293 631 290 octets
(9,59 GiB)**, et `du` confirme 9,6 G (5,5 G de segments, 3,7 G de `records.sqlite`, 446 M de
lexical). Les 4,0 Go (3,72 GiB) de différence se retrouvent dans le `df` de l'hôte (166 → 170 Go
libres) et, à l'arrondi du gigaoctet, dans celui de la VM (54 → 51 G utilisés,
`data/p10-import/df-vm-after-decisions.txt` et `df-vm-final-passA.txt`), alors que `du /data/indexes`
ne bouge pas. L'attache occupait donc un espace de travail hors des index, libéré à l'arrêt ; le
répertoire `tmp` du volume est à 4,0 K après le redémarrage. La nature exacte de cet espace n'est
pas établie, et elle n'est pas attribuée. Le relevé de 13,31 GiB n'a pas de fichier brut conservé : `index-state/
decisions.json` a été écrasé par le relevé suivant, et seul l'affichage de la commande en garde la
trace.

## Mémoire pendant l'attache de `decisions`

Le conteneur n'a **aucune limite** (`memory.max` vaut `max`) : la limite liante est celle de la VM,
**12 529 836 032 octets (11,67 Gio)**. Un échantillonneur (`import.py attach --sample-container`)
relève `memory.current`, `anon`, `file` (`memory.stat`) et le RSS du processus toutes les **5 s**,
horodaté côté client ; 126 points, aucune erreur, dans `data/p10-import/memory/decisions.jsonl`.
`memory.current` inclut le cache de pages, récupérable ; `anon` est la mémoire réellement occupée.

| Étape | Points | `anon` max | `memory.current` max | `file` à l'`anon` max |
|---|---:|---:|---:|---:|
| avant l'attache (six index ouverts) | 12 | 3,35 GiB | 10,44 GiB | 2,76 GiB |
| `validation` | 45 | 3,78 GiB | 10,49 GiB | 6,59 GiB |
| `segment` | 8 | 3,78 GiB | 10,46 GiB | 6,48 GiB |
| `transaction` | 21 | 10,85 GiB | 10,99 GiB | 0,04 GiB |
| `lexical` | 40 | **10,99 GiB** | **11,09 GiB** | 0,00 GiB |

Le pic tombe **à 633 s**, en fin d'étape `lexical` : `anon` **10,99 GiB**, `memory.current`
**11,09 GiB**, cache de pages ramené à **0**. La montée est nette à l'étape `transaction` : `anon`
passe de 3,78 GiB à 10,30 GiB en **65 s** (points de 334 s à 400 s,
`data/p10-import/memory-decisions-ramp.txt`), et le noyau récupère le cache au fur et à mesure
(6,50 GiB → 0,37 GiB).

La marge restante est de **0,58 GiB (5,0 %)** sous la limite de la VM. L'attache a donc réussi de
justesse : la même étape, sur `articles` et avec l'image d'avant la correction, avait tué le
conteneur. Après le job, `anon` reste à **10,55 GiB** (le delta de commit est retenu) ; il retombe à
**3,57 GiB** au redémarrage du serveur, pour six index ouverts.

## Incident — OOM de la VM pendant l'attache de `articles`

Pendant l'attache de `articles` (2 476 715 passages), le conteneur a été **tué par l'OOM killer de
la VM** :

- `docker inspect` : `OOMKilled: true`, `ExitCode: 137`, `FinishedAt: 2026-11-06T09:16:42.897482293Z` ;
- le job était à l'étape **`lexical`**, ouverte à `09:10:05Z` et jamais rendue ;
- **le point de commit était passé** : l'index `articles` porte la génération 1, 2 135 023 documents
  et 2 476 715 passages vivants, un segment de 2 476 715 vecteurs, 0 mort. La réouverture en
  lecture seule (INV-04) donne donc **la dernière génération commitée, complète** ;
- le job est journalisé `failed`, `generation: null`, avec « the job was interrupted by a server
  restart ». C'est le **redémarrage de clôture de la séance 1** (`09:17:44Z`) qui a écrit ce
  message : l'enregistrement du job n'avait jamais été finalisé, la cause réelle est l'OOM.

**Cause établie, et corrigée.** Le diagnostic natif de la tâche hors plan
`CORRIGER-MEMOIRE-APRES-COMMIT` a trouvé un cache d'un `serde_json::Value` par document, gardé pour
tout le lot dans `FilterIndex::insert_created_passages`. Le correctif (commit `aa2fe41`, fusionné
dans la branche de phase puis avancé sur la branche de tâche) ramène, en natif, le pic RSS
d'`articles` de 8,24 à 6,54 Go et celui de `jorf` de 5,34 à 3,80 Go. **Le pic de l'OOM lui-même
n'est pas mesuré** : aucun échantillonneur ne tournait pendant la séance 1.

**L'index lexical d'`articles` n'a pas été perdu.** Les fichiers de
`epochs/1/lexical/` datent de **09:10-09:11**, et son `meta.json` déclare **2 476 715 documents
vivants** (8 segments, opstamp 2 476 739) : la branche lexicale est **complète et durable** avant
l'OOM. C'est cohérent avec la réouverture : la reprise lexicale d'ADR-0088 n'a **rien** eu à
rattraper ni à reconstruire (voir la section suivante).

Conformément à ADR-0139, décision 3, la séance 1 s'est arrêtée là : aucun lot de `decisions` n'a été
soumis, aucun job en cours n'a été tué, et aucun réglage de Docker Desktop n'a été changé. La
reprise de la séance 2 a attaché `decisions` sans toucher à la VM.

## Reprise lexicale à la réouverture (séance 2)

Le serveur a été relancé sur les mêmes volumes, avec l'image reconstruite, à `13:29:00Z` :

- **41,9 s** jusqu'à la première réponse 200 de `/ready`, avec les six index ouverts (dont
  `decisions` encore vide). Le journal du conteneur donne le détail : `the server is bound` à
  13:29:00.000, `models and indexes loaded` (`indexes: 6, encoders: 1`) à 13:29:38.516, première
  réponse 200 de `/ready` à 13:29:41.917 — lignes conservées dans
  `data/p10-import/startup-a2-1329.log`. La durée de chargement du seul modèle n'est pas publiée par
  le serveur et n'est donc pas mesurée ici ;
- **aucun fichier lexical réécrit** : les `mtimes` de `epochs/1/lexical/` d'`articles` restent au
  09:10-09:11, et le répertoire lui-même est daté du 09:11. La reprise n'a donc ni rattrapé ni
  reconstruit : l'état lexical était déjà `Gt = G`, et l'hypothèse d'un lexical inachevé est
  **infirmée par les fichiers** ;
- **la branche lexicale est servie** : `POST /v1/indexes/articles/search` en mode `lexical` rend
  HTTP 200 avec trois résultats en **86 ms** (étape `lexical` 81,9 ms), puis 107 ms au second essai ;
  `decisions` rend aussi 200 en 77 ms après son attache ;
- `crible inspect` (serveur arrêté) confirme : `articles`, époque 1, génération 1, 2 135 023
  documents, 2 476 715 passages vivants, 0 mort, 1 segment de 2 476 715 vecteurs.

Après l'attache de `decisions`, le serveur a été arrêté (`docker stop`, code 0) pour l'inspection et
les relevés d'espace, puis **relancé à `13:45:01Z`** : **53 s** jusqu'à `/ready` avec six index
peuplés, `anon` maximal pendant l'ouverture **3,57 GiB**, `memory.current` 10,92 GiB dont 7,29 GiB
de cache de pages. Ce second démarrage conserve ses bruts
(`data/p10-import/memory-open-crible-p10-server-a2-*.jsonl`) ; ceux du premier ont été écrasés par
le script, qui tronquait son journal — corrigé depuis (`start.sh` horodate le nom des fichiers).

## Reconstruction lexicale (passe B, décision 4 d'ADR-0139)

`crible rebuild-lexical` a été lancé sur **chacun des six index**, **serveur arrêté**, en conteneur,
avec `tools/bench/p10-import/run-measured.py --interval 1` : durée, mémoire du conteneur et `df`
toutes les secondes. La cible est de **60 s par index** (ADR-0047 `:66`). La commande ne publie ni
compte de jetons ni compte de passages. Les passages sont ceux de l'état de l'index, et les **jetons
analysés** sont lus dans les index tantivy par
`crates/crible-store/examples/lexical_tokens.rs` (`cargo run --release -p crible-store --example
lexical_tokens -- <lexical-dir>`) : l'exemple lit le total que tantivy tient pour sa statistique BM25
(`Bm25StatisticsProvider::total_num_tokens`), en temps constant, sans toucher au code de production.
Sortie conservée dans `data/p10-import/lexical-tokens.txt`.

| Index | Passages | Jetons analysés | Durée | Cible 60 s | `anon` max | `memory.current` max | `lexical/` après |
|---|---:|---:|---:|---|---:|---:|---:|
| `dole` | 7 653 | 1 788 111 | **1 s** | tenue | — | — | 1 887 989 o |
| `sarde` | 214 578 | 14 123 942 | **5 s** | tenue | 0,08 GiB | 0,15 GiB | 16 403 035 o |
| `titres` | 1 477 883 | 23 186 520 | **13 s** | tenue | 0,70 GiB | 0,75 GiB | 46 620 715 o |
| `jorf` | 1 833 500 | 177 669 539 | **71 s** | **dépassée (×1,18)** | 0,51 GiB | 2,43 GiB | 286 619 972 o |
| `articles` | 2 476 715 | 310 806 138 | **127 s** | **dépassée (×2,12)** | 0,74 GiB | 3,42 GiB | 320 448 060 o |
| `decisions` | 3 589 776 | 494 570 237 | **185 s** | **dépassée (×3,08)** | 0,73 GiB | 3,74 GiB | 454 656 393 o |
| **somme** | 9 600 105 | **1 022 144 487** | **402 s** | **non comparable** | | | |

**Jetons par passage** : 233,6 pour `dole`, 65,8 pour `sarde`, 15,7 pour `titres` (des intitulés
courts), 96,9 pour `jorf`, 125,5 pour `articles`, 137,8 pour `decisions` ; **106,5 en moyenne** sur
les 9 600 105 passages. Le coût en temps suit le nombre de jetons mieux que le nombre de passages :
`titres` (23,2 M jetons) prend 13 s, `jorf` (177,7 M) 71 s, `decisions` (494,6 M) 185 s.

- **La somme n'est pas comparable à 60 s** : la cible est un budget de démarrage **par index**, pas un
  budget d'instance (ADR-0139, décision 4).
- **Trois index dépassent la cible** : `jorf` (71 s), `articles` (127 s) et `decisions` (185 s). Le
  rapport le constate, sans rien coder ni changer : la décision du propriétaire du 2026-10-29 prévoit
  un ADR ultérieur (relever la cible dans ce cas de reprise, ou paralléliser l'écriture), et
  `WRITER_THREADS` reste à 1 (ADR-0087).
- **Le coût par passage croît avec l'index** : 8,8 µs pour `titres` (1,48 M), 38,7 µs pour `jorf`
  (1,83 M), 51,3 µs pour `articles` (2,48 M), 51,5 µs pour `decisions` (3,59 M) — la mesure ne
  sépare pas ce qui vient du texte, du tokenizer ou de l'écriture tantivy.
- **La mémoire possédée reste sous 0,75 GiB** pour tous les index : cette reconstruction paie en
  temps d'écriture, pas en mémoire.
- **La génération n'est pas incrémentée** : la commande réécrit la branche lexicale de l'époque
  courante (`« rebuilt from the reference state at generation 1 / 3 / 2 »`).
- **Vérification de service** : après reconstruction, `POST /v1/indexes/{index}/search` en mode
  `lexical` rend **HTTP 200 avec trois résultats sur les six index** (dole 20,8 ms, sarde 6,5 ms,
  titres 50,4 ms, articles 65,4 ms, jorf 64,1 ms, decisions 63,7 ms), et `meta.json` de chaque index
  lexical déclare exactement le nombre de passages vivants de l'index (7 653 à 3 589 776).
- **Repères** : les trois petits index ont été reconstruits deux fois (la première passe, à
  échantillonnage 5 s, a donné 1, 6 et 16 s) ; le tableau retient la seconde passe. L'espace rendu
  par la reconstruction est négligeable : le `df` de la VM reste entre 50,2 et 50,6 GiB utilisés
  pendant les six passes.

## Instantanés (passe C, ADR-0064 et ADR-0065)

Les six instantanés ont été pris **hors ligne**, serveur arrêté (ADR-0064, décision 3), par
`crible snapshot --data /data --exchange /exchange --index <index>` dans un conteneur neuf sur le
même volume, avec `data/p10-exchange/` monté sur `/exchange`. La destination est
`exchange/snapshots/<index>-e<E>-g<G>/` et **six destinations neuves** ont été écrites : aucune
n'a été écrasée. Le harnais est `tools/bench/p10-import/snapshot.py`, qui relève la durée, le `df`
de l'hôte avant et après, puis contrôle la copie.

| Index | Durée | Taille | Octets | Époque | Génération | Fichiers listés | Complet |
|---|---:|---:|---:|---:|---:|---:|---|
| `dole` | **0,4 s** | 0,03 GiB | 27 826 610 | 1 | 1 | 15 | oui |
| `sarde` | **2,9 s** | 0,46 GiB | 498 833 036 | 1 | 1 | 21 | oui |
| `titres` | **15,3 s** | 2,95 GiB | 3 167 968 879 | 1 | 1 | 39 | oui |
| `jorf` | **24,4 s** | 4,42 GiB | 4 744 561 825 | 3 | 3 | 75 | oui |
| `articles` | **44,7 s** | 6,66 GiB | 7 156 322 218 | 1 | 1 | 57 | oui |
| `decisions` | **62,0 s** | 9,21 GiB | 9 884 596 466 | 2 | 2 | 75 | oui |
| **total** | **149,7 s** | **23,73 GiB** | **25 480 109 034** | | | | **6 / 6** |

**Le total attendu d'environ 31 Go est dépassé de 5,5 Go en sens inverse : la mesure donne
25,48 Go (23,73 GiB), soit 18 % de moins.** L'écart n'est pas une erreur de copie : l'instantané
exclut `cache/` (ADR-0064, décision 4) et copie `records.sqlite` par `VACUUM INTO`, qui rend une base
compactée — c'est pourquoi `decisions` donne 9,88 Go d'instantané pour 10,25 Go d'époque. Le chiffre
de 31 Go était une estimation, pas une mesure.

**Ces instantanés représentent l'état compacté, pas l'état brut de l'import** (ADR-0139,
conséquences « Négatives ») : `decisions` est copié à l'époque 2, génération 2, avec quatre segments
issus de `crible compact`, et `jorf` à l'époque 3, génération 3, avec deux segments ; les quatre
autres sont copiés à l'époque 1 de l'import. Restaurer `decisions` ne rend donc pas l'index tel qu'il
est sorti de l'attache, mais le même contenu, sans mort et re-segmenté.

**Contrôle de complétude.** `snapshot.json` d'ADR-0064 est présent partout, en `format_version: 1`,
avec l'index, l'époque et la génération de l'index source. Le contrôle compare la liste du
`snapshot.json` aux fichiers réellement présents : **aucun fichier manquant, aucun fichier non
listé, aucune taille différente**, sur les six (15 à 75 fichiers par instantané). Le **blake3** de
chaque fichier a été recalculé sur l'hôte avec `tools/bench/p10-import/blake3-check.py`, lancé depuis
`tools/` par `uv run --no-sync python` (le module `blake3` vient de `tools/pyproject.toml`) :
**282 fichiers, tous conformes** à `snapshot.json` — 15, 21, 39, 75, 57 et 75. Le détail est dans
`data/p10-import/snapshots/blake3-check.txt`. La CLI refait ce contrôle à la restauration, et la
restauration de contrôle ci-dessous l'exerce une seconde fois.

`df` de l'hôte, de 169 Go libres au départ à 144 Go à la fin (−25 Go, cohérent avec 25,48 Go
d'instantanés) : `169 → 166 → 161 → 154 → 144 Go`. **Le seuil d'arrêt de 40 Go libres n'a jamais été
approché.**

### Restauration de contrôle — `dole`

Le plus petit instantané a été restauré dans un dossier de données jetable (ADR-0065), puis ouvert
et interrogé :

```
crible restore --data /restore --snapshot /snapshot
index 'dole' restored from /snapshot: no epoch replaced, new epoch 1, generation 1,
  3600 document(s), 7653 passage(s)
```

- `crible inspect` sur la copie restaurée : époque 1, génération 1, **3 600 documents, 7 653
  passages vivants, 0 mort, 1 segment** (`seg-dole-full-v1`), identiques à l'index d'origine.
- Recherche `lexical` servie : « responsabilité contrat », 3 résultats en **54,1 ms**, premier
  résultat `JORFDOLE000017758191#7` (score 7,45, champ `type` = `LOI_PUBLIEE`).
- La restauration a donc accepté la copie, ce qui suppose le contrôle de `snapshot.json` réussi,
  blake3 compris. Les cinq autres instantanés n'ont pas été restaurés : le protocole ne demande
  qu'une restauration de contrôle, et les gros coûteraient 10 à 25 Go d'espace.

Le dossier jetable a été écrit dans `/tmp/p10-restore-check` (Docker Desktop n'ouvre pas `~/.Trash`
aux montages) puis **déplacé** vers `~/.Trash/p10-restore-check` : rien n'a été supprimé.

## Points ouverts — décisions du propriétaire

Ces points sont **mesurés et non tranchés**. Aucun n'a été corrigé pendant la campagne.

1. **La reconstruction lexicale dépasse la cible de 60 s pour trois index** : `jorf` 71 s (×1,18),
   `articles` 127 s (×2,12), `decisions` 185 s (×3,08). La décision du propriétaire du 2026-10-29
   prévoit un ADR si le dépassement est retenu comme normal à cette échelle — relever la cible ou
   paralléliser l'écriture (`WRITER_THREADS` reste à 1, ADR-0087). Rien n'est codé.
2. **Marge mémoire de 0,58 GiB à l'attache de `decisions`**, sur une VM de 12 Gio : 10,99 GiB
   d'`anon` pour 11,67 Gio. La proposition de **16 Gio** n'est pas mesurée en 16 Gio ; c'est un
   réglage que seul le propriétaire change (ADR-0139, décision 3).
3. **Rétention mémoire après une attache** : `anon` reste à 10,55 GiB après le job et ne retombe à
   3,57 GiB qu'au redémarrage. Ce n'est pas une fuite démontrée ; c'est un fait mesuré deux fois.
4. **Fusions `tantivy` annulées** pendant les deux compactions de `jorf` (`OpenWriteError(NotFound)`
   sur `epochs/tmp-2` puis `tmp-3`). Aucune perte de document, mais l'index lexical garde 10
   segments.
5. **`crible inspect` se bloque** sur un volume ouvert en écriture par le serveur : plus de 120 s
   sans message. Contournement appliqué : arrêter le serveur, ou tuer le processus par un conteneur
   auxiliaire `--pid=container:`.
6. **552,7 MiB de surcoût d'ouverture simultanée** (544,9 MiB en état (b)) : à état égal, l'instance
   à six index possède 2 152,8 MiB d'anonyme de plus que `dole` seul en (a), alors que la somme des
   écarts mesurés index par index vaut 1 600,1 MiB. Non expliqué par la campagne, conservé tel quel.
7. **Écart de 12 % avec `docs/02:230`** : la conception annonce 8 580 000 passages, la base importée
   en compte **9 600 105** (+11,89 %). L'écart existe déjà entre la conception et pgvector ; il n'est
   pas créé par l'import. Corriger la conception ou l'expliquer est une décision du propriétaire.

## État laissé

- **Serveur arrêté** (`docker stop`, code 0). Aucun conteneur de la campagne ne tourne, et la tâche
  rend l'instance prête à servir : `sh tools/bench/p10-import/start.sh` suffit à la relancer.
- **Conteneurs conservés**, tous arrêtés, aucun supprimé : `crible-p10-server` (image de la séance 1),
  `crible-p10-server-a2` (image `sha256:ff08d62b…`, commit `e059fdf`), les trois compactions
  (`p10-mes07-*`), les six reconstructions (`p10-rebuild*`) et les six instantanés
  (`p10-snapshot-*`).
- **Volumes conservés** : `crible-p10-data` (51 G dans la VM, `/data/indexes` 25 G) et
  `crible-p10-models` (1,5 G, `mdenseon`).
- **Index** : six index complets, 6 660 239 documents et 9 600 105 passages, 0 mort ; un seul
  segment source à l'import, puis `decisions` en époque 2, génération 2, 4 segments et `jorf` en
  époque 3, génération 3, 2 segments après les compactions de la passe B ; les quatre autres en
  époque 1, génération 1, 1 segment.
- **Exchange** : `data/p10-exchange/` conservé, `crible.toml`, `inbox/` (six lots montés en lecture
  seule) et **`snapshots/` avec les six instantanés, 24 G**. Les six lots de `data/legifacile-full/`
  n'ont pas été touchés.
- **Restauration de contrôle** : `~/.Trash/p10-restore-check` (27 Mo), déplacée là après la
  vérification ; rien n'a été supprimé.
- **Espace** : hôte **141 Go libres (131 GiB)** au moment du compte rendu — 144 Go (134 GiB) à la
  fin des instantanés, moins la copie des six index lexicaux (1,0 G) et la construction de
  l'exemple en release ; VM **137 G libres** sur 197 G.
- **Clé d'API** : `data/p10-import/token.txt` (mode 600), absente de git.
- **Aucune donnée de mesure supprimée** : ni `rm`, ni `docker volume rm`, ni `docker system prune`,
  ni vidage de corbeille. Le harnais emploie `docker run --rm` pour ses conteneurs éphémères
  (`mes09.py`, `snapshot.py`) et `rmdir` sur `/data/staging-mes09`, un dossier vide qu'il a créé :
  aucun index, aucun instantané, aucun lot n'a été retiré.

## Recommandations — à valider par le propriétaire

1. **Garder 12 Gio pour servir et n'agrandir qu'avant un nouvel import.** MES-09 mesure 3,54 GiB
   d'anonyme en régime, d'où un minimum de 4 Gio et un confortable de 12 Gio : la VM actuelle suffit
   au service. C'est l'attache qui manque de marge (0,58 GiB) ; une valeur de **16 Gio** reste la
   proposition pour un import, non mesurée en 16 Gio. Point ouvert n° 2.
2. **Redémarrer le serveur avant toute mesure de mémoire et avant une reconstruction lourde.** Le
   delta de commit retient ~7 GiB après un job ; un redémarrage les rend et ramène `anon` à
   3,57 GiB. La passe B a appliqué cette règle à chaque série.
3. **Réserver 2 fois l'index compacté et 1 fois l'index instantané.** Mesuré : +12,2 GiB pour la
   compaction de `decisions` (9,6 GiB), +5,1 GiB pour `jorf` ; 25,48 Go pour les six instantanés.
4. **Ne pas rejouer l'attache d'`articles`** : elle est commitée, complète, et son lexical est
   durable ; la reconstruction de la passe B est une preuve, pas une réparation.
5. **La CLI ne publie pas le compte de jetons.** `lexical_tokens` le donne ; si la mesure devient
   récurrente, l'exposer dans `crible rebuild-lexical` demanderait une modification de production et
   donc un ADR. Aujourd'hui, l'exemple suffit et ne change rien au moteur.
6. **Faire relire le harnais** : `start.sh` recrée le conteneur avec les six montages et doit suivre
   un changement de nom de lot ; `snapshot.py` et `mes09.py` écrivent dans `data/p10-exchange/` et
   `data/p10-import/`, tous deux gitignorés.

## Fichiers

Versionnés, à commiter par `@git` (17 fichiers) :

- `tools/bench/p10-import/prepare.sh` — contrôle d'espace, image, volumes, configuration, serveur ;
- `tools/bench/p10-import/start.sh` — démarrage (ou recréation) du serveur, échantillonnage de la
  mémoire pendant l'ouverture des index, délai jusqu'à `/ready` ;
- `tools/bench/p10-import/import.py` — client HTTP de création, d'attache et de suivi, avec
  l'échantillonneur mémoire (`--sample-container`) ;
- `tools/bench/p10-import/attach-all.sh` — ordre des lots, `df` avant et après, seuil d'arrêt ;
- `tools/bench/p10-import/summarize.py` — tableaux du rapport depuis les sorties brutes ;
- `tools/bench/p10-import/run-measured.py` — compaction et reconstruction lexicale mesurées hors
  ligne, `df` et mémoire échantillonnés ;
- `tools/bench/p10-import/mes09.py` — les trois états d'ADR-0077 sur le serveur, et le déplacement
  des dossiers d'index vers `/data/staging-mes09/` ;
- `tools/bench/p10-import/mes09-per-index.sh` — une série MES-09 par index, avec remise en place
  garantie ;
- `tools/bench/p10-import/snapshot.py` — les six instantanés, durée, `df` et contrôle de complétude ;
- `tools/bench/p10-import/blake3-check.py` — recalcul des blake3 d'un instantané contre
  `snapshot.json` (lancé par `uv run --no-sync` depuis `tools/`) ;
- `tools/bench/p10-import/README.md` — mode d'emploi, ordre `prepare`, `start`, `import`, `mes07`,
  `mes09`, `rebuild`, `snapshot`, et liste des sorties ;
- `crates/crible-store/examples/lexical_tokens.rs` — compte des jetons analysés d'un index lexical,
  lu dans les statistiques BM25 de tantivy (ADR-0139, décision 4) ;
- `docs/mesures/import-legifacile-2026-11-06.md` — ce rapport ;
- `docs/mesures/MES-07-duree-compaction.md` — section datée du 2026-11-06 ;
- `docs/mesures/MES-07-duree-compaction.2026-11-06.json` — la campagne MES-07 en JSON ;
- `docs/mesures/MES-09-ram-reelle.md` — section datée du 2026-11-06 ;
- `docs/mesures/MES-09-ram-reelle.2026-11-06.conteneur.json` — la campagne MES-09 en JSON.

Bruts, gitignorés, sous `data/p10-import/` : `df-host-*.txt`, `df-vm-*.txt`, `du-indexes-*.txt`,
`build.log`, `build-time.txt`, `build-passA2.log`, `build-time-passA2.txt`, `image-before-rebuild.txt`,
`key-generate.txt`, `token.txt`, `vm-info.txt`, `index-create/`, `jobs/`, `polls/`, `index-state/`,
`index-list.json`, `inspect-*.txt`, `server.log`, `oom-articles.txt`, `memory-after-restart.txt`,
`memory-after-restart-six.txt`, `memory-before-decisions.txt`, `memory-final-passA.txt`,
`memory/decisions.jsonl`, `memory-decisions-summary.txt`, `memory-decisions-ramp.txt`,
`memory-decisions-table.txt`, `memory-open-crible-p10-server-a2-*.jsonl`, `open-*.txt`,
`lexical-meta.txt`, `searches-after-recovery.txt`, `decisions-steps.txt`,
`docker-stats-after-restart.txt`, `attach-durations-all.log`, `startup-a2-1329.log`,
`lexical-tokens.txt`, `lexical/` (copie des six index lexicaux, 1,0 G), `mes07/`, `mes09/`,
`rebuild/`, `rebuild2/`, `snapshots/`.
