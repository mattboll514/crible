# Mesure — les deux routes de lecture en conteneur (index et document)

Rapport de la passe B de la tâche `MESURER-EN-CONTENEUR-MAC` (phase P8), point 4 de la fiche. Ce n'est
pas une mesure numérotée `MES-xx` de `docs/08-invariants-et-tests.md` : la fiche demande de mesurer
deux routes pour trancher une question d'architecture, et lui interdit d'ouvrir un numéro `MES-xx`
pour cela. Le protocole suit la décision 6 d'ADR-0135.

## Campagne du 2026-11-03

- **Tâche** : `MESURER-EN-CONTENEUR-MAC`, passe B.
- **Commit** : `3fb33439aa3349a31593b8281920af7ce51938bb`, branche
  `tache/MESURER-EN-CONTENEUR-MAC`. **Aucun code de production n'a été modifié.** Les harnais sont
  `crates/crible-store/examples/server_costs.rs` (nouveau, modes `walk` et `document`) et
  `tools/bench/mes-p8-conteneur/routes_client.py` (client HTTP).
- **Machine** : MacBook Air M3, 8 cœurs, 24 Go, macOS 15.7.7 (24G720), **en conteneur** : Docker
  Desktop 28.5.1, VM Linux `aarch64` de **12 529 836 032 octets (11,67 Gio)**, 8 vCPU, page de
  4 096 octets. Le serveur tourne dans l'image `crible:dev` avec le volume nommé
  `crible-mes-p8-data` sur `/data`. Aucune limite `--memory`. La charge de l'hôte n'est pas relevée :
  limite déclarée.
- **Configuration du serveur** : `data/mes-p8-conteneur/staging/crible.toml`, `search_threads = 0`,
  `max_concurrent_searches = 0`, `query_precision = "fp32"`, une clé de rôle `read`. Le serveur ouvre
  **tous** les index du dossier de données : `synth-large` et `synth-large-9seg`. Le premier n'est pas
  interrogé, mais il occupe sa projection et ses pages : déclaré.
- **Index interrogé** : `synth-large-9seg`, **8 segments** produits par `crible compact`
  (13 min 47 s), 7 740 000 passages vivants, génération 3, époque 2, dossier de
  **15 997 011 750 octets** en 31 fichiers. Il est la copie de la campagne de la passe A, préparée
  mais non mesurée ; c'est ici qu'elle sert.
- **Espace disque** : 92 GiB libres avant la campagne. Rien de volumineux n'a été écrit hors de
  `data/` et du volume.

### Les deux routes mesurées

| Route | Ce qu'elle paie | Timer dans le moteur |
|---|---|---|
| `GET /v1/indexes/{index}` | l'état de l'index, dont `sizes.disk_bytes`, qui parcourt le dossier (`read_dir`, `symlink_metadata`, une `len()` par fichier) | aucun : `directory_size` est `pub(crate)` |
| `GET /v1/indexes/{index}/documents/{doc_id}` | `ReadPool::new(&path, 1)` **par requête** (`crates/crible-server/src/documents.rs:543`), puis `read_document` et `passages_of_document` | aucun |

### Protocole

Trois séries, dans l'ordre, sur le même serveur :

1. **Client HTTP** (`routes_client.py`) : une passe de chauffe de 300 recherches denses, **écartée**,
   qui sert aussi à recueillir les identifiants (premier résultat de chaque requête) ; puis **trois
   passes mesurées** de **300 appels par route**, chaque route dans sa propre boucle, dans le même
   ordre à chaque passe. Client nommé : Python 3.13, `http.client.HTTPConnection`, une connexion
   persistante par passe, un appel à la fois, `time.perf_counter_ns` autour de l'appel, aucun
   réessai. Les percentiles sont recalculés depuis les durées par appel, sans filtre.
2. **Coût de `sizes.disk_bytes` isolé** : `server_costs walk --directory /data/indexes/synth-large-9seg
   --rounds 100`, même parcours récursif, même ordre d'appels, dans un conteneur à `ENTRYPOINT`
   remplacé, sur le volume, serveur en marche.
3. **Part de l'ouverture du `ReadPool` isolée** : `server_costs document --records
   /data/indexes/synth-large-9seg/epochs/2/records.sqlite --docs /data/staging/docs.json --rounds 3`,
   sur les **mêmes 300 identifiants** que la route, en deux temps par identifiant : un `ReadPool::new`
   puis les deux lectures (ce que fait la route), et les deux lectures sur un pool ouvert une seule
   fois et vérifié (le témoin). 900 appels par temps.

```sh
docker run -d --name crible-mes-p8-server -p 18080:8080 --stop-timeout 65 \
    -v crible-mes-p8-data:/data \
    -v "$PWD/data/mes-p8-conteneur/staging/crible.toml:/etc/crible/crible.toml:ro" crible:dev
python3 tools/bench/mes-p8-conteneur/routes_client.py --base http://127.0.0.1:18080 \
    --index synth-large-9seg --vectors data/mes02/queries.f32 \
    --token-file data/mes-p8-conteneur/staging/token.txt \
    --out data/mes-p8-conteneur/raw/routes-conteneur-9seg.json \
    --docs-out data/mes-p8-conteneur/staging/docs.json --passes 3
docker run --rm -v crible-mes-p8-data:/data --entrypoint server_costs \
    crible-mes-p8-harness:dev walk --directory /data/indexes/synth-large-9seg --rounds 100
docker run --rm -v crible-mes-p8-data:/data --entrypoint server_costs \
    crible-mes-p8-harness:dev document \
    --records /data/indexes/synth-large-9seg/epochs/2/records.sqlite \
    --docs /data/staging/docs.json --rounds 3
```

Le serveur est **arrêté et supprimé** à la fin (`docker stop` puis `docker rm`), sans redémarrage
d'un quelconque service.

Écarts et limites, déclarés :

- **La décision 6 d'ADR-0135 n'est tenue qu'approximativement pour ces deux routes** : la passe
  d'échauffement écartée est une passe de **recherches** (`POST …/search`), pas une passe d'appels
  aux deux routes mesurées. Les routes de lecture sont donc mesurées dès leur premier appel ; la
  chauffe a seulement fait entrer dans le cache les pages de codes binaires du scan, pas celles de
  `sizes` ni celles des documents. Aucun point n'est retiré des séries mesurées, et le premier appel
  de chaque route est dans le p99 et le maximum.
- **Le coût de `sizes.disk_bytes` est mesuré par une réplique** du parcours de `directory_size`,
  parce que la fonction est `pub(crate)` : c'est le même ensemble d'appels système, pas le même
  appel de fonction. La réplique a été vérifiée sur le nombre de fichiers (31) et le total
  d'octets (15 997 011 750), identiques à ce que la route renvoie.
- **L'ouverture du `ReadPool` n'est pas mesurée seule** : `ReadPool::new` n'ouvre rien (il range un
  chemin), donc la part cherchée est la **différence** entre les deux temps du témoin.
- **Un appel de la route document a duré 2,77 ms**, et un de la réplique `fresh` 12,07 ms : ces
  points sont **conservés** et visibles dans les p99 et les maxima. Aucun n'est retiré.
- **Un seul index multi-segments** (8 segments) est mesuré ; les 1- et 8-segments ne sont pas
  comparés ici.

### Résultats

Documents bruts : `data/mes-p8-conteneur/raw/routes-conteneur-9seg.json`, `routes-smoke.json`,
`b-walk-9seg.json`, `b-readpool-9seg.json` (gitignorés) ; artefact versionné :
`docs/mesures/routes-en-conteneur-2026-11-03.json`, écrit par
`tools/bench/mes-p8-conteneur/aggregate-b.py`, qui recalcule chaque percentile depuis les durées par
appel et **refuse** une série dont le nombre d'appels, le minimum, le maximum ou la moyenne s'écarte
de ceux du client ou du harnais.

Tous les percentiles de ce rapport et de l'artefact sont pris au **rang le plus proche** (ADR-0077,
décision 4), jamais par interpolation ; l'artefact a été régénéré depuis les sorties brutes le
2026-11-03, après alignement de l'agrégateur sur cette méthode, et les deux concordent. Le client
écrivait ses propres percentiles par interpolation : ils restent dans les sorties brutes, et ce
rapport ne cite que les chiffres recalculés.

**Les deux routes**, 900 appels chacune, trois passes, en millisecondes :

| Route | p50 | p95 | p99 | max | corps |
|---|---:|---:|---:|---:|---:|
| `GET /v1/indexes/synth-large-9seg` | **0,401** | **0,491** | 1,143 | 10,720 | 1 495 octets |
| `GET /v1/indexes/synth-large-9seg/documents/{doc_id}` | **0,402** | **0,494** | 0,630 | 2,769 | — |

Par passe, p50 / p95 : index 0,395 / 0,504, 0,399 / 0,487, 0,411 / 0,521 ; document 0,408 / 0,473,
0,387 / 0,482, 0,414 / 0,505. Les deux routes se valent au dixième de milliseconde.

**Contexte, non ciblé** : la recherche dense de la même passe (`POST …/search`, k = 10) vaut
**50,114 ms** en p50 et 74,341 ms en p95. C'est elle qui fixe le coût d'une requête, pas les deux
routes mesurées.

**Coût de `sizes.disk_bytes`**, 100 parcours du dossier de l'index :

| Grandeur | Valeur |
|---|---|
| Fichiers parcourus | 31 |
| Octets additionnés | 15 997 011 750 |
| p50 | **0,0483 ms** |
| p95 | 0,104 ms |
| min – max | 0,0466 – 0,216 ms |

Soit **12,0 %** du p50 de la route (`0,0483 / 0,4011`). Le reste est le reste du corps de la route,
la sérialisation et la pile HTTP.

**Part de l'ouverture du `ReadPool`**, 300 identifiants × 3 tours, deux temps :

| Temps | p50 | p95 | p99 | max |
|---|---:|---:|---:|---:|
| `ReadPool::new` seul | **0,083 µs** | 0,084 µs | 0,209 µs | 0,667 µs |
| lectures sur un pool **neuf** (ce que fait la route) | **66,167 µs** | 82,083 µs | 221,416 µs | 12 068,916 µs |
| lectures sur un pool **réutilisé** (témoin) | **20,583 µs** | 32,833 µs | 56,292 µs | 873,459 µs |

`ReadPool::new` range un chemin : **83 ns**, il n'ouvre rien. Ce que la route paie en plus du témoin
est donc l'ouverture de la connexion et la préparation des requêtes SQLite **au premier usage** :
**45,6 µs** en p50 (66,167 − 20,583), soit **11,3 %** du p50 de la route (0,0456 / 0,4015). Les 900
lectures ont rapporté 900 passages.

### Lecture

- **Les deux routes sont dix fois sous le seuil de 5 ms en p95** : 0,491 ms et 0,494 ms, soit 9,8 et
  9,9 % du seuil, sur un index de 8 segments et 16 Go. Le seuil d'ADR-0135 est tenu avec une marge
  qui n'appelle aucun travail d'optimisation.
- **Ce qui domine une route n'est pas ce qu'on croit** : `sizes.disk_bytes` coûte 0,05 ms et
  l'ouverture du `ReadPool` 0,046 ms ; à elles deux ces deux parts font 0,09 ms sur 0,40 ms, et le
  reste est la pile HTTP et la sérialisation. Les diviser par deux ne changerait pas l'ordre de
  grandeur de la route.
- **Un `ReadPool` par époque ne se justifie pas par la latence, dans cette mesure.** La part qu'il
  économiserait est de **45,6 µs par requête** (11,3 % du p50, 0,9 % du seuil de 5 ms en p95). La
  proposition — si elle est retenue pour d'autres raisons, telles que la pression sur les
  descripteurs de fichiers ou la lisibilité — relève d'une **tâche séparée** : la fiche le dit, et ce
  rapport ne la tranche pas.
- **La queue est réelle et conservée** : 10,72 ms sur la route d'index (une passe sur 900 appels) et
  2,77 ms sur celle des documents. Aucune n'approche le seuil en p95, mais un p99 de 1,143 ms montre
  qu'un parcours de dossier peut être préempté.

### Réserves

Ce que la mesure **ne** permet pas d'affirmer :

- le coût de ces routes sur la cible : le Mac mini M1 n'a pas été mesuré ;
- le coût de `sizes.disk_bytes` sur un index à un segment, ou sur un dossier de plusieurs centaines
  de fichiers : seul un index à 8 segments et 31 fichiers a été parcouru ;
- l'effet d'une limite `--memory` ou d'un cache plus petit : aucune limite n'était posée, et la VM
  faisait 11,67 Gio ;
- la part exacte de la pile HTTP dans les 0,40 ms : elle n'est pas isolée, seules les deux parts
  internes le sont ;
- le régime réel des identifiants : les 300 documents interrogés sont les premiers résultats d'une
  recherche dense sur 300 requêtes, pas un tirage uniforme du corpus.
