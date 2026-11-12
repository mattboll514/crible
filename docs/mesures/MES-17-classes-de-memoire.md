# MES-17 — Quelle classe de mémoire d'un conteneur Crible grandit, à quelle étape, et si la suppression d'un index ou le redémarrage la rend

## Campagne du 2026-11-12

- **Tâche** : `T05-MEMOIRE-NON-RENDUE` (v1.1). Mesure MES-17 d'`docs/08-invariants-et-tests.md`, protocole d'ADR-0151, décision 3, erratum du 2026-11-11 compris.
- **Commit** : `0bc657cd18ba00b3649fd39a471fae82fbb37146` (`0bc657c`), branche `tache/T05-MEMOIRE-NON-RENDUE`. Aucun code de production n'a été modifié : les seuls fichiers écrits sont ce rapport et les données brutes sous `data/mes17/` (gitignoré).
- **Machine** : MacBook Air M3 (8 cœurs, 24 Go), macOS 15.7.7 (24G720). **En conteneur** : Docker Desktop 28.5.1, VM `aarch64` de **10 420 109 312 octets (9,71 Gio)**, 8 vCPU, **aucune limite `--memory`** posée sur le conteneur (ADR-0135, décision 5, la RAM de la VM fait la borne), taille de page **4 096 octets** relevée dans le conteneur (ADR-0135, décision 7).
- **Image** : `crible:t05-mes17`, `sha256:1603d69c0bd6acecae7f1ae740a2acd6357479105aca41d95328fe09ca3604a5`, construite par l'orchestrateur depuis `0bc657c` et non reconstruite ici. Elle n'a pas de shell : les lectures de `/proc` et du cgroup passent par un conteneur jetable `postgres:17` (voir le protocole).
- **Jeu de données** : l'échantillon Legifacile, `data/legifacile-sample`, **six lots réels, 205 261 documents et 257 626 passages**, 515 Mo, décrit par son `README.md`. Empreintes relevées : `queries.jsonl` `sha256:fdaa2794b027307cb9ac2e69c2dcb3a0f25ac5ff59b830b9c19ec359310391c6` ; manifestes des six lots dans `data/mes17/raw/checksums.json`. Modèle : `data/models/mdenseon` (`model.onnx` fp32 de 1 227 955 436 octets, `tokenizer.json` de 34 363 287 octets), monté en lecture seule.
- **Configuration** : `data/mes17/crible.toml`, copie du modèle `docker/crible.example.toml` avec une clé `admin`, `[encoder].query_precision = "fp32"`, `/data` sur le volume nommé dédié `crible-mes17-data`, `/exchange` sur `data/mes17/exchange` (dossier de l'hôte, comme `docker/README.md` le prescrit), `search_threads = 0` et `max_concurrent_searches = 0` (les valeurs par défaut).
- **Index jetable** : `mes17`, créé pour la mesure par `POST /v1/indexes` puis supprimé par `DELETE /v1/indexes/mes17?confirm=mes17`. Aucun autre index, aucun autre volume, aucun autre conteneur n'a été touché ; `crible-p10-data`, `crible-mes-p8-data` et l'instance de développement `legifacile-dev-crible-1` sont restés intacts, et `.env.prod` n'a pas été ouvert.

### Protocole

Les six états d'ADR-0151, décision 3, dans l'ordre, sur le même processus. Chaque état est lu **trois fois**, à trois secondes d'intervalle, et la dispersion des trois lectures est rapportée : c'est le maximum moins le minimum sur les classes du tableau.

À chaque lecture, un conteneur `postgres:17` jetable partage les espaces de noms du serveur et lit, en une seule passe :

- `/proc/1/status` (`VmRSS`, `RssAnon`, `RssFile`, `RssShmem`, `VmHWM`) et `/proc/1/smaps_rollup` (`Rss`, `Pss`, `Anonymous`, `Private_Dirty`), par `docker run --rm --pid=container:crible-mes17 --cap-add SYS_PTRACE` ;
- `memory.current` et `memory.stat` du cgroup du conteneur, par `--cgroupns=host` et le chemin vérifié `/sys/fs/cgroup/docker/<id complet>/memory.current` ;
- les lignes `memory` du journal, par `docker logs` après chaque état (JSON `tracing`).

Une lecture ratée n'est jamais remplacée par un zéro : la sonde réessaie trois fois et s'arrête en erreur. Les scripts de la campagne sont `data/mes17/prepare_lots.py` et `data/mes17/measure.py` ; les sorties brutes sont sous `data/mes17/raw/` (`states/`, `logs/`, `queries/`, `attach/`, `memory-lines.json`, `summary.json`, `summary-run2.json`).

Séquence exécutée :

```sh
# volume et conteneur dédiés, une seule fois
docker volume create crible-mes17-data
docker run --rm -u 0 -v crible-mes17-data:/data postgres:17 sh -c 'chown -R 1000:1000 /data'
docker run -d --name crible-mes17 -p 18017:8080 --stop-timeout 65 \
  -v crible-mes17-data:/data -v "$PWD/data/models:/models:ro" \
  -v "$PWD/data/mes17/exchange:/exchange" \
  -v "$PWD/data/mes17/crible.toml:/etc/crible/crible.toml:ro" crible:t05-mes17

cd data/mes17
python3 measure.py state a-fresh        # (a) serveur neuf, aucun index
python3 measure.py create               # POST /v1/indexes : charge la session ONNX
python3 measure.py state b-index-open   # (b) index ouvert, aucune requête
python3 measure.py queries              # 300 requêtes de queries.jsonl, mode hybrid, k = 10
python3 measure.py state c-after-queries# (c) après la passe de requêtes
python3 measure.py attach               # six lots, 257 626 passages, un job par lot
python3 measure.py state d-after-attach # (d) après la campagne d'attache
python3 measure.py delete               # DELETE de l'index jetable, job delete_index
python3 measure.py state e-after-delete # (e) après le DELETE
python3 measure.py restart              # docker restart -t 65
python3 measure.py state f-after-restart# (f) après redémarrage
```

Aucun préchauffage séparé : l'état (a) est le serveur tel qu'il démarre, et l'état (b) suit immédiatement la création de l'index, donc le chargement du modèle. Les pages de fichiers ne sont ni vidées ni réchauffées à la main : la classe `file` est rapportée telle qu'elle se présente à chaque état, et son évolution s'explique par les fichiers touchés.

Une **campagne complémentaire déclarée** (états g à k) a été ajoutée après le redémarrage, pour deux raisons que le protocole laisse ouvertes : reproduire la croissance anonyme de l'attache sur un second processus, et mesurer la passe de requêtes **sur un index plein**, alors que l'ordre imposé des six états la place sur un index encore vide (voir « Écarts au protocole »). Elle rejoue création, attaché des six lots, 300 requêtes, puis `DELETE`, sur le même volume et le même conteneur, avec les mêmes sondes.

### Résultats

MiB = 1 048 576 octets. `current` = `memory.current` du cgroup, `anon` et `file` = entrées du même nom de `memory.stat`, « non ventilé » = `current − anon − file` (slab, piles noyau, tables de pages), `RssAnon`, `RssFile` et `Rss` = `/proc/1/status`, `PSS` = `smaps_rollup`. Chaque valeur est la **médiane des trois lectures** ; « disp. » est le maximum moins le minimum sur toutes les classes de la ligne, en MiB.

**Campagne principale (processus P1, conteneur `crible-mes17`, six états d'ADR-0151)**

| État | `current` | `anon` | `file` | non ventilé | `RssAnon` | `RssFile` | `Rss` | `PSS` | `VmHWM` | disp. |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| (a) serveur neuf, aucun index | 8,8 | 7,5 | 0,1 | 1,2 | 7,2 | 11,9 | 19,2 | 19,1 | 19,2 | 0,1 |
| (b) index ouvert | 2 712,8 | 1 501,4 | 1 204,1 | 7,3 | 1 500,9 | 29,1 | 1 530,1 | 1 530,2 | 2 063,3 | 0,0 |
| (c) après 300 requêtes | 2 715,2 | 1 503,7 | 1 204,2 | 7,3 | 1 503,2 | 30,1 | 1 533,3 | 1 533,4 | 2 063,3 | 0,2 |
| (d) après la campagne d'attache | 4 253,5 | 2 659,3 | 1 560,8 | 33,4 | 2 658,9 | 35,9 | 2 694,8 | 2 694,7 | 2 712,0 | 0,0 |
| (e) après le `DELETE` | 3 416,0 | 2 598,4 | 806,9 | 10,7 | 2 598,0 | 34,9 | 2 633,0 | 2 632,8 | 2 712,0 | 0,0 |
| (f) après redémarrage | 10,1 | 7,5 | 1,6 | 1,0 | 7,0 | 12,0 | 19,0 | 19,1 | 19,0 | 0,2 |

Écarts d'un état au suivant, en MiB, sur la médiane :

| Transition | `current` | `anon` | `file` | `RssAnon` | `Rss` |
|---|---:|---:|---:|---:|---:|
| (a) → (b) création de l'index, chargement de la session | +2 704,0 | **+1 493,9** | **+1 204,0** | +1 493,7 | +1 510,9 |
| (b) → (c) 300 requêtes sur index vide | +2,4 | +2,3 | +0,1 | +2,3 | +3,2 |
| (c) → (d) six lots attachés | +1 538,3 | **+1 155,6** | **+356,6** | +1 155,7 | +1 161,5 |
| (d) → (e) `DELETE` | −837,5 | **−60,9** | **−753,9** | −60,9 | −61,8 |
| (e) → (f) redémarrage | −3 405,9 | −2 590,9 | −805,3 | −2 591,0 | −2 614,0 |

**Campagne complémentaire (processus P2, même conteneur après le redémarrage de (f), états déclarés en plus des six)**

| État | `current` | `anon` | `file` | non ventilé | `RssAnon` | `RssFile` | `Rss` | `VmHWM` | disp. |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| (g) avant la création, après redémarrage | 10,2 | 7,5 | 1,6 | 1,1 | 7,0 | 12,2 | 19,2 | 19,2 | 0,1 |
| (h) index ouvert | 2 667,2 | 1 454,1 | 1 206,0 | 7,1 | 1 453,7 | 29,2 | 1 482,8 | 2 063,0 | 0,0 |
| (i) après les six lots attachés | 5 398,0 | 2 968,9 | 2 395,1 | 34,0 | 2 968,3 | 35,6 | 3 004,0 | 3 004,8 | 0,0 |
| (j) après 300 requêtes, index plein | 5 417,5 | 2 987,5 | 2 395,3 | 34,7 | 2 986,8 | **438,7** | 3 425,6 | 3 425,6 | 0,0 |
| (k) après le `DELETE` | 4 576,5 | 2 923,8 | 1 641,3 | 11,4 | 2 923,2 | 34,8 | 2 958,0 | 3 425,6 | 0,0 |

| Transition | `current` | `anon` | `file` | `RssFile` |
|---|---:|---:|---:|---:|
| (g) → (h) chargement de la session (2<sup>e</sup> processus) | +2 657,0 | **+1 446,6** | +1 204,4 | +17,0 |
| (h) → (i) six lots attachés | +2 730,8 | **+1 514,6** | +1 189,1 | +6,4 |
| (i) → (j) 300 requêtes sur index plein (`pool_size` = 257 626) | +19,5 | +18,5 | +0,2 | **+403,1** |
| (j) → (k) `DELETE` | −841,0 | **−63,7** | −754,0 | −403,9 |

Les deux campagnes donnent le même classement : la **mémoire anonyme** est la classe qui grandit à l'attache, le **cache de fichiers** celle qui grandit à la lecture des fichiers du modèle et des segments, et le `DELETE` ne rend que la seconde. Les volumes absolus diffèrent d'une campagne à l'autre (session 1 493,9 contre 1 446,6 MiB ; attache +1 155,6 contre +1 514,6 MiB d'anonyme), ce qui interdit de lire ces chiffres comme une constante : c'est la classe et le sens de la variation qui se répètent.

**Session ONNX, isolée par différence au chargement.** Aucun compteur par session n'existe (ADR-0151, décision 1). La différence (a) → (b) vaut **1 493,9 MiB d'anonyme et 1 204,0 MiB de `file`**, et la seconde campagne donne 1 446,6 et 1 204,4 MiB. La part `file` s'explique exactement : `model.onnx` (1 171,1 MiB) plus `tokenizer.json` (32,8 MiB) font 1 203,9 MiB, soit la valeur lue à 0,1 MiB près ; ce sont des pages de cache récupérables, pas de la mémoire possédée. La part anonyme, elle, est la session elle-même, avec l'index vide créé au même appel (négligeable : un `index.json` et un SQLite vide). Le pic `VmHWM` de 2 063 MiB atteint pendant le chargement montre en outre un transitoire d'environ 560 MiB au-dessus du régime, non retenu. **Aucune ligne `session_load` n'apparaît pendant les 300 requêtes encodées**, ni sur l'index vide ni sur l'index plein : le processus ne charge qu'une session pour le modèle `mdenseon` fp32, celle du document comme celle de la requête, ce que la ligne unique de chaque processus confirme.

**Ce que dit le journal, lot par lot (campagne principale puis complémentaire).** Anonyme après chaque `job_end`, en MiB, et gain depuis le lot précédent :

| Lot | passages | P1 `RssAnon` | gain | P2 `RssAnon` | gain |
|---|---:|---:|---:|---:|---:|
| articles | 50 000 | 2 238,2 | — | 2 185,6 | — |
| jorf | 50 000 | 2 406,9 | +168,7 | 2 416,9 | +231,3 |
| decisions | 49 998 | 2 495,1 | +88,2 | 2 451,0 | +34,1 |
| titres | 50 000 | 2 528,5 | +33,4 | 2 820,8 | +369,8 |
| sarde | 50 000 | 2 599,9 | +71,4 | 2 915,3 | +94,5 |
| dole | 7 628 | 2 658,9 | +59,0 | 2 968,3 | +53,0 |

Le gain par lot varie de +33 à +370 MiB et n'est **pas proportionnel au nombre de passages** : `dole`, avec 7 628 passages, coûte +59,0 MiB, autant qu'un lot de 50 000 passages. Aucun plateau ne se dessine sur six lots, et la croissance ne retombe pas entre deux lots. L'attache ajoute donc de l'anonyme par job autant que par volume de données, ce qui oriente vers des tampons et de la rétention d'allocateur plutôt que vers les données indexées : celles-ci restent en `mmap`, et c'est la classe `file` qui les porte (voir la conclusion).

**Le journal et les lectures directes concordent.** À la ligne `session_load` de P1 (`07:44:37`), le journal donne `rss_anon_bytes` = 1 500,9 MiB et `cgroup_current_bytes` = 2 712,6 MiB ; l'état (b), lu une seconde plus tard, donne 1 500,9 et 2 712,8 MiB. Au dernier `job_end` d'attache de P1, le journal donne 2 658,9 et 4 254,3 MiB, l'état (d) 2 658,9 et 4 253,5 MiB. L'écart reste sous 1 MiB tant que la lecture suit la ligne de près. Il s'ouvre en revanche **après** le `DELETE` : `delete_after_removal` lit 4 230,3 MiB de `current`, l'état (e) 3 416,0 MiB, parce que les 754 MiB de cache de fichiers libérés par la suppression de l'index sont récupérés par le noyau quelques secondes après l'appel. De même, l'anonyme baisse de 60,9 MiB (P1) et 63,7 MiB (P2) entre la ligne de suppression et l'état lu cinq secondes plus tard : la ligne mesure l'instant du `unlink`, pas le repos qui suit. Une ligne `memory` se lit donc comme un point de passage, jamais comme un état stable.

**Contrôle des quatre `step`.** Les quatre valeurs de l'ADR apparaissent aux étapes attendues, dans les deux processus, et aucune ne manque :

| `step` | Attendu | Constaté | Verdict |
|---|---|---|---|
| `session_load` | au chargement de la session ONNX, `index` nommant l'index déclencheur | P1 `07:44:37` (`index: mes17`, création de l'index), P2 `07:57:42` (`index: mes17`) ; une seule fois par processus | présent |
| `job_end` | à la fin de chaque job, quel qu'il soit | 7 lignes en P1 et 7 en P2 : six `attach_batch` puis un `delete_index`, chacune avec `job_id`, `kind` et `index` | présent |
| `delete_before_rename` | dans `delete_index`, avant le renommage vers la quarantaine | P1 `07:56:19.469`, P2 `08:07:53.280`, même `job_id` que la paire, 66 ms et 13 ms avant l'étape suivante | présent |
| `delete_after_removal` | dans `delete_index`, après suppression du dossier en quarantaine | P1 `07:56:19.535`, P2 `08:07:53.293` | présent |

Aucun défaut d'instrumentation n'est à signaler sur cette campagne. Deux précisions, qui ne sont pas des défauts. D'abord, la suppression de l'index dans l'essai écarté (ci-dessous) n'a pas eu lieu : les deux étapes de `delete_index` n'ont donc été observées que sur les deux suppressions de la campagne retenue. Ensuite, une **création** d'index n'émet qu'une ligne `session_load` et aucun `job_end`, puisqu'une création n'est pas un job : c'est ce que la décision 2 d'ADR-0151 décrit, et cela laisse la création et le chargement de session indistinguables dans le journal, ce que la mesure a dû séparer par différence d'états.

### Écarts au protocole, et ce qu'ils changent

1. **Lots dérivés.** Les six lots de l'échantillon ne partagent pas le même schéma (champs de document et champ de passage `partie` différents), et un index n'a qu'un schéma. Chaque lot a donc été recopié sous `data/mes17/exchange/inbox/mes17-<lot>/` avec, en plus, une colonne nulle pour chaque champ que sa structure ne porte pas (`partie` pour cinq lots, les champs des autres structures). Le texte, les `doc_id`, les `doc_hash`, l'`ord` et les **octets des vecteurs fp16 sont copiés tels quels** ; les comptes sont recomptés (identiques : 205 261 documents, 257 626 passages) et les sommes de contrôle `blake3` des manifestes recalculées. Le champ `index` du manifeste vaut `mes17`, sans quoi l'attache refuse le lot. Conséquence à retenir : dans cet index, un filtre sur un champ qu'une structure ne porte pas voit `null` ; la mesure ne porte aucune requête filtrée, donc cela ne change aucun chiffre.
2. **La passe de requêtes de l'état (c) tourne sur un index vide**, parce que l'ordre imposé place les requêtes avant l'attache. Les 300 requêtes rendent toutes `200` avec `pool_size: 0` ; elles exercent l'encodage et la plomberie, pas le scan dense ni le rescoring. Leur effet mesuré, +2,4 MiB, ne dit donc rien de la mémoire d'une recherche sur un index peuplé. La campagne complémentaire comble ce trou : sur l'index plein, la passe ajoute +18,5 MiB d'anonyme et **+403,1 MiB de `RssFile`** (dont 404,0 MiB de pages `mmap` de segments), latence murale p50 64,5 ms, p95 122,3 ms, p99 175,2 ms pour 300 requêtes séquentielles.
3. **Harnais sous `data/mes17/`, non sous `tools/bench/`.** La consigne de la session n'autorisait l'écriture que du rapport et des données brutes ; les scripts sont donc dans la zone gitignorée, avec les sorties. Ils sont cités ici pour la reproduction ; les déplacer sous `tools/bench/mes17/` est une proposition, pas un fait accompli.
4. **Sondes par un conteneur `postgres:17`.** L'image mesurée n'a pas de shell ; les lectures passent par un conteneur jetable partageant les espaces de noms de pid et de cgroup, avec `SYS_PTRACE`. Aucune image n'a été téléchargée. La première lecture de `smaps_rollup` d'un espace de noms fraîchement attaché a été refusée une fois ; la sonde réessaie jusqu'à trois fois et échoue franchement plutôt que de rendre un zéro.
5. **Essai écarté.** Un premier essai a échoué : les manifestes dérivés nommaient encore l'index d'origine, et les six jobs d'attache sont partis en `failed` (`invalid_request`, message « batch index … does not match index 'mes17' »). Aucune mesure de cet essai n'est retenue ; son journal est gardé sous `data/mes17/raw/logs/run1-journal.jsonl` et ses lectures sous `data/mes17/raw/states-run1/`. Il a une valeur de contrôle : ces six jobs en échec ont chacun émis leur `job_end`, ce qui montre que l'étape couvre aussi les jobs qui échouent. Le conteneur et le volume ont ensuite été détruits et recréés, pour que la campagne retenue parte d'un volume neuf.
6. **Le volume n'a pas été recréé entre les deux campagnes.** Seule la suppression de l'index a remis `/data` dans un état sans index. L'écart (f) → (g) de 1,3 MiB de `current` vient des fichiers d'exploitation laissés par la campagne précédente (`jobs.sqlite`, `analytics.sqlite`), pas d'un index.

### Conclusion

**Quelle classe grandit, à quelle étape.**

- **La mémoire anonyme possédée (`anon` du cgroup, `RssAnon` du processus) est la classe qui grandit à l'attache**, et c'est la seule qui ne se rend pas. Six lots, 257 626 passages : **+1 155,6 MiB** sur le premier processus et **+1 514,6 MiB** sur le second. La croissance est régulière d'un lot au suivant, sans plateau sur six lots, et son increment n'est pas proportionnel au volume : +33 à +370 MiB par lot, `dole` (7 628 passages) coûtant autant qu'un lot de 50 000. Cette absence de proportionnalité, alors que l'index sur disque ne fait que 790 Mo (753,8 MiB) et que ses vecteurs sont en `mmap`, désigne des tampons et de la rétention d'allocateur plutôt que des données indexées.
- **La session ONNX est anonyme, payée une fois par processus** : 1 493,9 MiB (P1) et 1 446,6 MiB (P2) au premier chargement, plus 1 204,0 MiB de cache de fichiers du modèle, récupérable. Les 300 requêtes encodées ne provoquent aucun second chargement.
- **Le cache de fichiers (`file`, `RssFile`) grandit à l'attache et à la recherche, et c'est la classe récupérable.** À l'attache, +356,6 MiB (P1) et +1 189,1 MiB (P2) de cache des fichiers écrits et des lots lus ; à la passe de requêtes sur index plein, +403,1 MiB de pages de segments mappées. Aucun de ces chiffres n'est une fuite : le noyau les réclame sous pression (règle de mémoire de `docs/02`).
- **Le `DELETE` ne rend que le cache de fichiers** : −753,9 MiB (P1) et −754,0 MiB (P2) de `file`, plus les 403,9 MiB de segments mappés de P2, contre **−60,9 et −63,7 MiB d'anonyme**, dont l'essentiel arrive quelques secondes après les deux lignes de `delete_index`. L'index supprimé laisse donc derrière lui de l'ordre de **1,1 à 1,5 GiB d'anonyme** que ni le `unlink` ni la purge de quarantaine ne rendent.
- **Le redémarrage rend tout** : (f) revient à 10,1 MiB de `current` et 7,0 MiB de `RssAnon`, soit le niveau du serveur neuf à 1,3 MiB près. La mémoire retenue est donc de la durée de vie du processus, jamais de l'état sur disque : aucune trace de la campagne ne subsiste après redémarrage.
- **La ventilation est complète à 1 % près** : « non ventilé » vaut 1 à 7 MiB au repos et 33 à 34 MiB après une attache, dont 27,1 MiB de `slab` et 5,7 MiB de tables de pages. `memory.stat` explique donc `memory.current`, et l'écart entre `memory.current` et `RssAnon` est bien le cache de fichiers et le slab, jamais un trou inexpliqué.

**Ce que la mesure ne permet pas d'affirmer.**

- **Rien sur l'OOM d'attache du ticket** : il n'est pas archivé, il portait sur un import de production et il demande sa propre reproduction. Les volumes mesurés ici (maximum 5 417,5 MiB, soit 5,3 GiB, de `current` pour 9,71 Gio de VM) restent loin d'une limite.
- **Rien sur les volumes de production.** L'échantillon fait 257 626 passages, là où Legifacile en compte 9,6 M ; les chiffres disent la classe et le sens, jamais l'échelle. La campagne de MES-09 (`docs/mesures/MES-09-ram-reelle.md`, vers 422-446) reste le point de comparaison déclaré, avec son delta anonyme de 10,55 GiB après attache.
- **Rien sur la nature exacte de l'anonyme retenu** : la mesure ne distingue pas une arène d'allocateur d'une structure vivante. Un profil de tas ou des compteurs d'allocateur le diraient ; ils ne sont pas dans la campagne.
- **Rien sur le coût d'un redémarrage avec un index à recharger** : l'état (f) est un redémarrage sans index, donc il ne dit pas si la parade « redémarrer » coûte un rechargement complet de 1,45 GiB de session plus l'index. La campagne complémentaire montre seulement que ce rechargement se reproduit à l'identique (1 446,6 MiB).
- **Rien sur une éventuelle stabilisation** : six lots ne suffisent pas à dire si la croissance s'arrête, plafonne ou continue indéfiniment. Un septième lot n'a pas été attaché.

**Pistes que les chiffres désignent, à valider par le propriétaire.** Aucune n'est décidée ici, et aucune ne peut l'être sans un ADR distinct (ADR-0151, décision 4).

1. **Attribuer l'anonyme retenu avant de le corriger.** La classe est identifiée, la cause ne l'est pas. Une ligne `memory` enrichie de compteurs d'allocateur, ou un profil de tas sur une attache, séparerait la rétention d'allocateur d'une structure vivante. C'est la seule étape qui dise si les pistes suivantes visent juste.
2. **Si c'est l'allocateur** : un `malloc_trim(0)` après la fin d'un job, sur glibc, ou le choix d'un allocateur qui rende ses arènes, sont les deux gestes que la littérature désigne. Aucun n'est dans `docs/07` : `libc` demanderait son ADR (ADR-0080, décision 3), et un changement d'allocateur aussi.
3. **`madvise(MADV_DONTNEED)` ne viserait pas la bonne classe.** Les chiffres le disent : les `mmap` de segments sont en `file`, rendus de 754 MiB par le `DELETE` et de 403,9 MiB par la suppression de l'index de P2 ; c'est l'anonyme qui reste. `memmap2` expose déjà `Advice::DontNeed`, mais l'appliquer aux segments ne rendrait pas le GiB anonyme mesuré.
4. **Le déchargement de session est un autre levier, d'un autre ordre de grandeur** : 1,45 GiB d'anonyme par session, non rendus avant redémarrage (ADR-0105, décision 2, choix assumé). La mesure le chiffre sans le juger ; le rouvrir demande un ADR.
5. **Deux propositions de protocole, pour une prochaine campagne de MES-17** : placer la passe de requêtes **après** la campagne d'attache, pour qu'elle porte sur un index plein — l'ordre actuel la mesure sur un index vide, `pool_size` à 0 ; et ajouter un septième état « redémarrage avec l'index encore présent », qui dirait ce que coûte la parade et si le rechargement rend la même empreinte. Ces deux changements touchent la décision 3 d'ADR-0151 et ne s'appliquent pas d'eux-mêmes.

**Espace disque et nettoyage.** `df -h .` relevait 56 Gio libres avant la campagne et 56 Gio après ; les lots dérivés et les sorties brutes occupent 439 Mo sous `data/mes17/`, l'échantillon d'origine 515 Mo, inchangé. Le conteneur `crible-mes17` a été arrêté puis supprimé, le volume `crible-mes17-data` supprimé, et aucun conteneur `postgres:17` de sonde ne subsiste ; `docker ps -a --filter name=crible-mes17` et `docker volume ls --filter name=crible-mes17` ne rendent plus rien. Les autres volumes et conteneurs de la machine n'ont pas été touchés, et la production n'a été ni lue ni approchée (ADR-0032).

### Données brutes

- `data/mes17/raw/states/*.json` — les trois lectures de chaque état (campagne principale et campagne complémentaire, suffixe `-run2`).
- `data/mes17/raw/logs/` — le journal de chaque état, la ligne `memory` extraite dans `data/mes17/raw/memory-lines.json`, et le journal de l'essai écarté.
- `data/mes17/raw/queries/pass.json`, `pass-run2.json` — les 300 requêtes, état, `took_ms`, latence murale et `pool_size`.
- `data/mes17/raw/attach/campaign.json`, `campaign-run2.json`, `delete.json`, `delete-run2.json` — les objets `job` des campagnes.
- `data/mes17/raw/summary.json`, `summary-run2.json` — les médianes et dispersions par état, telles que les tableaux les citent.
- `data/mes17/raw/checksums.json`, `derived-lots.json`, `prepare-lots.log` — l'empreinte du jeu et la description des lots dérivés.
