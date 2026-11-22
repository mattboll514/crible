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

## Campagne du 2026-11-22

- **Tâche** : `T05-SUITE-ATTRIBUER-ANONYME` (v1.1). Seconde campagne de MES-17, protocole
  d'ADR-0157 (accepté le 2026-11-21) complété par ADR-0158 (accepté le 2026-11-22). La règle de
  lecture appliquée est celle d'ADR-0157, décision 5, **dans le sens fixé par ADR-0158** : un effet
  est une **baisse** de l'anonyme retenu d'au moins la dispersion entre les deux processus de V0.
- **Commit** : `31e98041c974f649ceef641286b62b72cd1d043f`, branche
  `tache/T05-SUITE-ATTRIBUER-ANONYME`. Aucun code de production n'a été modifié : les seuls fichiers
  écrits sont cette section, le harnais `tools/bench/mes17/` et les données brutes sous
  `data/mes17/raw/2026-11-22/` (gitignoré).
- **Machine** : MacBook Air M3 (`Mac15,12`, 8 cœurs, 24 Go), macOS 15.7.7 (24G720). **En
  conteneur** : Docker Desktop 28.5.1, VM `aarch64` de **10 420 109 312 octets (9,71 Gio)**, 8 vCPU,
  **aucune limite `--memory`** (ADR-0135, décision 5), noyau `6.10.14-linuxkit`, **glibc
  2.36-9+deb12u14**, taille de page **4 096 octets** relevée par `getconf PAGESIZE` dans la sonde
  (ADR-0135, décision 7).
- **Disque** : **104 Gio libres** au début et à la fin de la campagne (`df -h .`), au-dessus du
  plancher de 15 Gio d'ADR-0157, décision 9.
- **Image** : `crible:t05-suite-mes17`,
  `sha256:dc800f534ff50589038b0ca5d7a4fd9336a3259c45b70fdb00408b84366a8080`, construite par
  l'orchestrateur le 2026-11-22 depuis `31e9804` et non reconstruite ici (journal
  `data/mes17/build-t05-suite.log`). Elle n'a pas de shell : les lectures passent par un conteneur
  `postgres:17` jetable et `--pull=never`.
- **Jeu de données** : l'échantillon Legifacile, `data/legifacile-sample`, six lots réels,
  **205 261 documents et 257 626 passages**, 515 Mo ; `queries.jsonl`
  `sha256:fdaa2794b027307cb9ac2e69c2dcb3a0f25ac5ff59b830b9c19ec359310391c6` ; modèle `mdenseon`
  (`data/models`), monté en lecture seule.
- **Configuration** : `data/mes17/crible.toml`, copie du modèle `docker/crible.example.toml` avec une
  clé `admin`, `[encoder].query_precision = "fp32"`, `/data` sur le volume nommé jetable
  `crible-mes17-data`, `/exchange` sur `data/mes17/exchange`, `search_threads = 0` et
  `max_concurrent_searches = 0` (valeurs par défaut). Aucun réglage d'allocateur dans l'image ni dans
  la configuration : la variante n'existe que dans l'environnement du conteneur, posé par le harnais.
- **Exploitation** : l'instance de développement de Legifacile, `legifacile-dev-crible-1`, était
  **arrêtée** par le propriétaire pendant toute la campagne, et n'a jamais été démarrée. La
  production n'a été ni lue ni approchée (ADR-0032). Aucun autre conteneur, volume ou réseau n'a été
  touché : seuls `crible-mes17`, `crible-mes17-data`, les sondes `postgres:17 --rm` et le port 18017
  ont servi.

### Protocole

Les **sept états** d'ADR-0157, décision 2, dans cet ordre, sur le même conteneur et le même volume :
(a) processus neuf, aucun index ; (b) index jetable créé ; (c) six lots attachés ; (d) 300 requêtes
de `queries.jsonl`, mode `hybrid`, `k = 10`, sur l'index plein ; (e) redémarrage avec l'index
présent ; (f) `DELETE` de l'index ; (g) redémarrage sans index.

**Cinq parcours, pas sept.** ADR-0158, décision 2, arrête la campagne à cinq exécutions du parcours :
V0 deux fois (contrôle), V1, V2 et V3 une fois. **V4 ne tourne pas**, puisque aucune variante n'a
d'effet au sens d'ADR-0158 (voir « Lecture »). Chaque parcours part d'un conteneur neuf ; le volume
est réutilisé, l'index ayant été supprimé à l'état (f). Un parcours supplémentaire dure environ
28 minutes de VM, avec l'instance de développement de Legifacile arrêtée : c'est ce coût qu'ADR-0158
épargne.

**Harnais versionné** : `tools/bench/mes17/measure.py` (bibliothèque standard seule) et
`prepare_lots.py`, avec le mode d'emploi `tools/bench/mes17/README.md`. Un parcours se rejoue ainsi,
depuis la racine du dépôt :

```sh
docker volume create crible-mes17-data
docker run --rm -u 0 -v crible-mes17-data:/data --pull=never postgres:17 \
  sh -c 'chown -R 1000:1000 /data'
python3 tools/bench/mes17/measure.py up --variant V0        # variante posée ici, et nulle part ailleurs
RUN=data/mes17/raw/<date>/<parcours>
python3 tools/bench/mes17/measure.py init --run "$RUN" --variant V0
python3 tools/bench/mes17/measure.py state a-fresh --run "$RUN"
python3 tools/bench/mes17/measure.py create --run "$RUN"
python3 tools/bench/mes17/measure.py state b-index-open --run "$RUN"
for lot in mes17-legifacile-sample-articles-v1 mes17-legifacile-sample-jorf-v1 \
           mes17-legifacile-sample-decisions-v1 mes17-legifacile-sample-titres-v1 \
           mes17-legifacile-sample-sarde-v1 mes17-legifacile-sample-dole-v1; do
  python3 tools/bench/mes17/measure.py attach --run "$RUN" --lot "$lot"
done
python3 tools/bench/mes17/measure.py state c-after-attach --run "$RUN"
python3 tools/bench/mes17/measure.py queries --run "$RUN"
python3 tools/bench/mes17/measure.py state d-after-queries --run "$RUN"
python3 tools/bench/mes17/measure.py restart --run "$RUN"
python3 tools/bench/mes17/measure.py state e-after-restart-index --run "$RUN"
python3 tools/bench/mes17/measure.py delete --run "$RUN"
python3 tools/bench/mes17/measure.py state f-after-delete --run "$RUN"
python3 tools/bench/mes17/measure.py restart --run "$RUN"
python3 tools/bench/mes17/measure.py state g-after-restart --run "$RUN"
python3 tools/bench/mes17/measure.py summary --run "$RUN"
python3 tools/bench/mes17/measure.py journal-check --run "$RUN"
python3 tools/bench/mes17/measure.py down
```

`up` est le **seul** endroit qui pose la variante, par `docker run -e GLIBC_TUNABLES=…` (ADR-0157,
décision 4) ; `init` relit la valeur exacte par `docker inspect` et l'écrit dans `run.json`. L'attache
est découpée en une commande par lot pour tenir la borne d'exécution ; les six appels, leur ordre et
le rapport de campagne sont ceux d'un appel unique.

**Preuve que glibc a pris le réglage.** Avant chaque parcours, le chargeur de l'image
`/lib/ld-linux-aarch64.so.1` est lancé avec la même variable et `--list-tunables` (aucun shell
nécessaire) ; la sortie est conservée dans le dossier du parcours. La commande est
`python3 tools/bench/mes17/measure.py tunables --out <dossier> --variant Vn` : elle prend le
chargeur pour point d'entrée, ne démarre aucun serveur, et écrit `tunables.txt` et `tunables.json`
(valeur exacte, image et identifiant d'image, commande). `--tunables` remplace la valeur de la
variante et `--name` choisit le nom des fichiers, ce qui produit la preuve V2 (écart 7). Pendant la
campagne, la sortie venait de la même commande passée à la main ; la sous-commande a été ajoutée au
harnais ensuite, puis rejouée une fois par variante dans
`data/mes17/raw/2026-11-22/tunables-check/` : les sorties sont identiques, octet pour octet, à
`tunables-baseline.txt` et aux `tunables.txt` de V1, V2 et V3, et la preuve V2 à
`V2/tunables-proof.txt`. Valeurs effectives lues :

| Variante | `GLIBC_TUNABLES` | Valeur effective |
|---|---|---|
| V0 | aucun | — (référence `tunables-baseline.txt`) |
| V1 | `glibc.malloc.arena_max=1` | `glibc.malloc.arena_max: 0x1` |
| V2 | `glibc.malloc.trim_threshold=0:glibc.malloc.top_pad=0` | `trim_threshold: 0x0`, `top_pad: 0x0` (voir écart 7) |
| V3 | `glibc.malloc.mmap_threshold=131072` | `glibc.malloc.mmap_threshold: 0x20000` |

`/proc/1/environ` du serveur est lu par la même sonde à chaque état, et seule la valeur de
`GLIBC_TUNABLES` en est conservée. V1 et V3 y figurent entières ; V2 n'y montre que son premier
élément, glibc écrivant un NUL au premier `:` en tokenisant la valeur en place (écart 8).

**Relevés.** Chaque état est lu **trois fois, à 3 secondes d'intervalle** ; le rapport donne la
**médiane** et la **dispersion** (maximum moins minimum) par classe. Sources, lues en une passe par la
sonde : `/proc/1/status` (`VmRSS`, `RssAnon`, `RssFile`, `RssShmem`, `VmHWM`, `VmSwap`),
`/proc/1/smaps_rollup`, `/proc/1/smaps` complet, `memory.current`, `memory.swap.current` et
`memory.stat` du cgroup, `/proc/meminfo` de la VM, et les lignes `memory` du journal du serveur.
`/proc/1/smaps` est volumineux : il est lu en entier et analysé, et **seul l'agrégat est conservé**
(ADR-0157, « Conséquences »). Une lecture ratée n'est jamais remplacée par un zéro : la sonde
réessaie trois fois et échoue franchement, et `read_once` échoue dès qu'un champ de
`STATUS_FIELDS`, `STAT_FIELDS` ou `MEMINFO_FIELDS` manque.

**Capacité de la sonde.** Elle est lancée avec `--pid container:… --cgroupns=host` et la capacité
`SYS_PTRACE`, seules conditions pour lire `/proc/1` du conteneur mesuré et son cgroup. Aucun
attachement `gdb`, `strace` ni `ptrace` n'est fait : l'interprétation retenue est que cette capacité
sert la lecture, pas le débogage, donc elle est hors de l'exclusion d'ADR-0157, décision 6
(déclaration demandée par `docs/suivi.md`).

**Relevés déclarés en plus** du protocole d'ADR-0157, lus dans les mêmes fichiers par la même sonde :
`VmSwap` de `/proc/1/status`, `memory.swap.current` du cgroup, et `MemAvailable` et `SwapFree` de
`/proc/meminfo` de la VM. La VM a environ 4 Gio de swap : une mémoire anonyme passée en swap
cacherait la rétention mesurée. **Aucun échange n'est intervenu** : `VmSwap`, `memory.swap.current`,
`zswapped` et `swapcached` valent 0 dans toutes les lectures, et `SwapFree` reste à
4 178 772 kB du début à la fin.

**Garde mémoire**, appliquée à chaque lecture : arrêt si `MemAvailable` de la VM passe sous 1 Gio, ou
si `SwapFree` baisse de plus de 256 Mio depuis la ligne de base de `init`. Elle ne s'est jamais
déclenchée, et échoue si la valeur ou la ligne de base manque ; le minimum de la campagne est
**6 774 704 kB** (V0-p1, état (d)), très au-dessus de 1 Gio.

**Règle de comptage des arènes de glibc**, telle qu'implémentée dans le harnais : un groupe de
régions anonymes **consécutives** (sans chemin) dont les `Size` s'additionnent à **exactement
65 536 kB** (64 Mio) compte pour **une** arène, et son `Rss` est la somme des `Rss` du groupe. C'est
la signature d'une arène de glibc, souvent vue comme une région `rw-p` suivie d'une région `---p`.
Un groupe dont les tailles ne font pas 65 536 kB n'est pas une arène et est rapporté à part. Le
rapport donne le nombre d'arènes et leur `Rss` total. Contrôle de la règle : les 52 groupes de V0-p1
à l'état (c) somment tous à 65 536 kB, mais leurs découpages varient : 47 groupes se lisent `rw-p`
puis `---p` (un seul est `rw-p` 65 228 kB puis `---p` 308 kB ; 23 sont `rw-p` 132 kB puis `---p`
65 404 kB), et 5 groupes sont une seule région `rw-p` de 65 536 kB.

**Contrôle du journal.** Pour chaque parcours, le rapport vérifie la présence des **quatre `step`**
d'ADR-0151, décision 2 (`job_end`, `delete_before_rename`, `delete_after_removal`,
`session_load`), **un `job_end` par lot attaché** (six) et **deux `session_load`**, aux états (b) et
(e). Une ligne manquante serait un défaut du rapport, pas une observation (ADR-0151, erratum).

### Résultats

MiB = 1 048 576 octets. `current` = `memory.current` du cgroup ; `anon` et `file` = entrées du même
nom de `memory.stat` ; `RssAnon` = `/proc/1/status` ; « arènes » = nombre et `Rss` total des arènes
au sens de la règle ci-dessus ; « disp. » = maximum des dispersions des trois lectures sur `current`,
`anon`, `file`, `RssAnon` et `Rss` des arènes. Chaque valeur est la **médiane des trois lectures**.
Les écarts sont calculés **sur les octets, avant arrondi**. Les comparaisons sont internes à cette
campagne, jamais avec les chiffres du 2026-11-12 (ADR-0157, décision 1).

**V0, premier processus (P1)**

| État | `current` | `anon` | `file` | `RssAnon` | `VmSwap` | arènes | `Rss` arènes | disp. |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| (a) neuf | 17,3 | 7,8 | 8,2 | 7,4 | 0,0 | 16 | 0,5 | 0,0 |
| (b) index créé | 2 780,7 | 1 543,7 | 1 229,5 | 1 543,4 | 0,0 | 34 | 785,4 | 0,4 |
| (c) six lots | 4 948,1 | 2 494,8 | 2 419,3 | 2 494,4 | 0,0 | 52 | 1 350,1 | 0,0 |
| (d) 300 requêtes | 4 967,0 | 2 512,8 | 2 419,3 | 2 512,2 | 0,0 | 52 | 1 367,7 | 0,0 |
| (e) redémarrage, index présent | 2 724,3 | 1 513,1 | 1 203,9 | 1 512,8 | 0,0 | 34 | 754,8 | 0,4 |
| (f) `DELETE` | 2 724,4 | 1 513,3 | 1 204,0 | 1 512,8 | 0,0 | 33 | 754,8 | 0,0 |
| (g) redémarrage, sans index | 9,3 | 8,2 | 0,1 | 7,7 | 0,0 | 16 | 0,4 | 0,4 |

**V0, second processus (P2)**

| État | `current` | `anon` | `file` | `RssAnon` | `VmSwap` | arènes | `Rss` arènes | disp. |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| (a) neuf | 8,9 | 7,8 | 0,1 | 7,5 | 0,0 | 16 | 0,4 | 0,1 |
| (b) index créé | 2 700,6 | 1 489,3 | 1 204,0 | 1 488,9 | 0,0 | 34 | 731,4 | 0,0 |
| (c) six lots | 4 911,8 | 2 485,7 | 2 393,0 | 2 485,3 | 0,0 | 52 | 1 308,8 | 0,0 |
| (d) 300 requêtes | 4 931,3 | 2 504,5 | 2 393,0 | 2 503,8 | 0,0 | 52 | 1 327,1 | 0,0 |
| (e) redémarrage, index présent | 2 703,6 | 1 492,5 | 1 203,9 | 1 492,3 | 0,0 | 32 | 734,6 | 0,4 |
| (f) `DELETE` | 2 704,2 | 1 493,1 | 1 204,0 | 1 492,7 | 0,0 | 32 | 734,7 | 0,0 |
| (g) redémarrage, sans index | 8,9 | 7,8 | 0,1 | 7,5 | 0,0 | 16 | 0,4 | 0,4 |

**V1, `glibc.malloc.arena_max=1`**

| État | `current` | `anon` | `file` | `RssAnon` | `VmSwap` | arènes | `Rss` arènes | disp. |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| (a) neuf | 8,4 | 7,3 | 0,1 | 7,1 | 0,0 | 0 | 0,0 | 0,1 |
| (b) index créé | 2 520,4 | 1 309,6 | 1 204,0 | 1 309,4 | 0,0 | 0 | 0,0 | 0,0 |
| (c) six lots | 4 706,8 | 2 276,2 | 2 393,0 | 2 275,9 | 0,0 | 0 | 0,0 | 0,0 |
| (d) 300 requêtes | 4 708,0 | 2 276,6 | 2 393,0 | 2 276,2 | 0,0 | 0 | 0,0 | 0,0 |
| (e) redémarrage, index présent | 2 724,0 | 1 513,0 | 1 203,9 | 1 512,8 | 0,0 | 0 | 0,0 | 0,2 |
| (f) `DELETE` | 2 724,2 | 1 513,2 | 1 204,0 | 1 513,0 | 0,0 | 0 | 0,0 | 0,0 |
| (g) redémarrage, sans index | 8,4 | 7,4 | 0,1 | 7,1 | 0,0 | 0 | 0,0 | 0,6 |

**V2, `glibc.malloc.trim_threshold=0:glibc.malloc.top_pad=0`**

| État | `current` | `anon` | `file` | `RssAnon` | `VmSwap` | arènes | `Rss` arènes | disp. |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| (a) neuf | 9,0 | 7,8 | 0,1 | 7,4 | 0,0 | 16 | 0,5 | 0,7 |
| (b) index créé | 2 525,3 | 1 314,3 | 1 204,0 | 1 313,9 | 0,0 | 19 | 269,2 | 0,1 |
| (c) six lots | 4 796,3 | 2 367,9 | 2 393,0 | 2 368,1 | 0,0 | 42 | 964,6 | 0,0 |
| (d) 300 requêtes | 4 804,5 | 2 375,1 | 2 393,0 | 2 375,0 | 0,0 | 42 | 967,3 | 0,0 |
| (e) redémarrage, index présent | 2 553,6 | 1 342,7 | 1 203,9 | 1 342,5 | 0,0 | 20 | 269,2 | 0,7 |
| (f) `DELETE` | 2 530,8 | 1 320,0 | 1 204,0 | 1 319,8 | 0,0 | 19 | 269,2 | 0,0 |
| (g) redémarrage, sans index | 9,4 | 8,2 | 0,1 | 7,9 | 0,0 | 16 | 0,5 | 0,5 |

**V3, `glibc.malloc.mmap_threshold=131072`**

| État | `current` | `anon` | `file` | `RssAnon` | `VmSwap` | arènes | `Rss` arènes | disp. |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| (a) neuf | 9,0 | 7,9 | 0,1 | 7,7 | 0,0 | 16 | 0,4 | 0,1 |
| (b) index créé | 2 540,5 | 1 329,4 | 1 204,0 | 1 329,1 | 0,0 | 18 | 141,3 | 0,1 |
| (c) six lots | 4 787,5 | 2 359,7 | 2 393,0 | 2 359,8 | 0,0 | 41 | 812,7 | 0,0 |
| (d) 300 requêtes | 4 796,2 | 2 367,6 | 2 393,0 | 2 367,5 | 0,0 | 41 | 818,0 | 0,0 |
| (e) redémarrage, index présent | 2 553,6 | 1 342,7 | 1 203,9 | 1 342,4 | 0,0 | 18 | 141,2 | 0,6 |
| (f) `DELETE` | 2 530,8 | 1 320,0 | 1 204,0 | 1 319,6 | 0,0 | 18 | 141,3 | 0,0 |
| (g) redémarrage, sans index | 9,0 | 7,9 | 0,1 | 7,6 | 0,0 | 16 | 0,5 | 0,4 |

**Transitions en anonyme, par parcours (MiB)**

| Parcours | (a)→(b) | **(b)→(c)** | (c)→(d) | (d)→(e) | (e)→(f) | (f)→(g) |
|---|---:|---:|---:|---:|---:|---:|
| V0-p1 | +1 535,9 | **+951,1** | +18,0 | −999,7 | +0,2 | −1 505,1 |
| V0-p2 | +1 481,5 | **+996,4** | +18,7 | −1 011,9 | +0,5 | −1 485,3 |
| V1 | +1 302,3 | **+966,6** | +0,3 | −763,5 | +0,2 | −1 505,9 |
| V2 | +1 306,5 | **+1 053,6** | +7,3 | −1 032,4 | −22,7 | −1 311,8 |
| V3 | +1 321,6 | **+1 030,3** | +7,9 | −1 024,9 | −22,7 | −1 312,1 |

Trois lectures stables : la dispersion des trois lectures ne dépasse jamais 0,7 MiB dans les cinq
parcours, et 0,4 MiB sur l'anonyme retenu. La dispersion entre les deux processus de V0, elle, vaut
**45,3 MiB** sur (c)−(b) (951,1 contre 996,4), soit **45,37 MiB** sur les octets exacts
(47 575 040 octets).

**Lecture, selon ADR-0158.** Le seuil est calculé sur les octets exacts : moyenne de V0 **973,74 MiB**,
seuil 973,74 − 45,37 = **928,37 MiB**. Une variante n'a d'effet que si son (c)−(b) est inférieur ou
égal à cette valeur ; les écarts du tableau portent sur les valeurs arrondies à 0,1 MiB. ADR-0158
(`docs/adr/0158-sens-de-l-effet-dans-la-regle-de-lecture-de-la-seconde-campagne-de-mes-17.md:55`,
`:97`) donne `973,75`, `45,3` et `928,45` MiB sur ces valeurs arrondies : la règle est la même, et
l'écart de 0,08 MiB ne change aucune lecture.

| Variante | (c)−(b) | Écart à la moyenne de V0 | Effet (baisse ≥ 45,3 MiB) | Lecture ADR-0157, décision 5 |
|---|---:|---:|---|---|
| V1 `arena_max=1` | +966,6 | −7,1 | **non** | rétention par les arènes : hypothèse non soutenue |
| V2 `trim_threshold=0:top_pad=0` | +1 053,6 | +79,8 | **non** (hausse) | observation, aucune cause confirmée |
| V3 `mmap_threshold=131072` | +1 030,3 | +56,5 | **non** (hausse) | observation, aucune cause confirmée |

Aucune variante ne fait baisser l'anonyme retenu d'au moins 45,3 MiB : la ligne « Aucune variante n'a
d'effet » de la table d'ADR-0157, décision 5, s'applique — structure vivante ou fragmentation —, et
V4 ne tourne pas (ADR-0158, décision 2).

**Contrôle du journal.** Les cinq parcours sont conformes : les quatre `step` apparaissent ; chaque
parcours porte 7 lignes `job_end` (six `attach_batch`, un `delete_index`), **un `job_end` par lot
attaché** (6/6) ; `delete_before_rename` et `delete_after_removal` partagent le `job_id` du `DELETE` ;
`session_load` apparaît **deux fois**, une en (b) (création de l'index) et une en (e) (redémarrage
avec l'index présent), sans nouvelle ligne en (g). Aucun défaut d'instrumentation n'est à signaler.

**Durées** (aucune conclusion de latence n'en est tirée : les variantes changent l'allocateur, donc le
temps, et la campagne ne mesure pas la latence ; ADR-0157, décision 5).

| Parcours | Attache des six lots | Articles / jorf / decisions / titres / sarde / dole | 300 requêtes | Création | `DELETE` | Redémarrages (e) puis (g) |
|---|---:|---|---:|---:|---:|---:|
| V0-p1 | 491,3 s | 72,3 / 93,4 / 108,4 / 15,1 / 36,2 / 165,9 | 17,7 s | 3,0 s | 3,0 s | 4,4 s puis 0,3 s |
| V0-p2 | 521,9 s | 72,4 / 99,6 / 123,6 / 18,1 / 36,2 / 172,0 | 18,5 s | 2,2 s | 3,0 s | 4,4 s puis 0,3 s |
| V1 | 615,0 s | 93,4 / 123,6 / 150,8 / 21,1 / 48,2 / 177,9 | 18,4 s | 2,4 s | 3,0 s | 4,4 s puis 0,3 s |
| V2 | 527,6 s | 72,3 / 96,5 / 126,7 / 18,1 / 39,2 / 174,8 | 18,6 s | 2,2 s | 3,0 s | 4,4 s puis 0,3 s |
| V3 | 545,8 s | 72,3 / 114,6 / 129,8 / 18,1 / 36,2 / 174,8 | 19,7 s | 2,2 s | 3,0 s | 4,4 s puis 0,3 s |

Un relevé d'état dure environ 7 secondes (trois lectures et deux pauses de 3 secondes). Les 300
requêtes rendent toutes `200`, avec `pool_size` = 257 626 ; les temps muraux par requête sont dans
`queries/pass.json` de chaque parcours.

### Écarts au protocole, et ce qu'ils changent

1. **Lots dérivés réutilisés.** Les six lots de `data/mes17/exchange/inbox/` viennent de la première
   campagne et n'ont pas été re-dérivés, pour ne pas réécrire des fichiers de cette campagne.
   Intégrité vérifiée hors mesure (index `mes17`, 205 261 documents, 257 626 passages, six
   manifestes) et par l'attache elle-même, qui revalide les sommes `blake3` : 6/6 jobs réussis dans
   chaque parcours. `prepare_lots.py` est versionné pour rejouer la dérivation ; il demande `pyarrow`
   et `blake3`, outils de développement de la première campagne, comme
   `tools/bench/mes02/make_multiseg_batch.py`. Le mesurage, lui, reste en bibliothèque standard.
2. **Pas de préchauffage séparé.** L'état (a) est le serveur tel qu'il démarre, et (b) suit
   immédiatement la création de l'index, donc le chargement de la session, comme dans la campagne du
   2026-11-12.
3. **Taille de page lue par `getconf`.** `/proc/1/smaps_rollup` de ce noyau n'expose pas
   `KernelPageSize` ; la sonde lit `getconf PAGESIZE` (4 096 octets) et une valeur illisible fait
   échouer la lecture, plutôt que de prendre un défaut.
4. **`smaps` non conservé.** `/proc/1/smaps` est lu en entier à chaque lecture, mais seul l'agrégat
   (nombre d'arènes, `Rss`, groupes, régions sans chemin) est écrit, comme ADR-0157 le prévoit.
5. **Attache en six commandes.** Chaque lot est attaché par une commande distincte, pour tenir la
   borne de 30 minutes par commande ; les six appels, leur ordre et le rapport sont ceux d'un appel
   unique.
6. **Volume réutilisé.** Le volume n'est pas recréé entre deux parcours : l'index est supprimé à
   l'état (f), et les fichiers d'exploitation (`jobs.sqlite`, `analytics.sqlite`) restent. C'est
   l'écart 6 de la campagne du 2026-11-12, et il explique l'écart de 0 à 8 MiB entre les états (a) et
   (g) des parcours.
7. **Preuve V2 indistinguable sur la valeur.** `--list-tunables` affiche `0x0` pour un tunable non
   posé comme pour la valeur `0` : la preuve de V2 passe par une sonde à valeurs non nulles
   (`trim_threshold=4096:top_pad=12345` rend `0x1000` et `0x3039`), qui montre que le parseur traite
   une chaîne à deux éléments, et par la valeur exacte conservée dans `run.json`.
8. **`/proc/1/environ` de V2 tronqué au premier `:`.** glibc tokenise la valeur en place et écrit un
   NUL au séparateur ; la sonde ne voit que `glibc.malloc.trim_threshold=0`. `docker inspect` et
   `run.json` gardent la chaîne complète. V1 et V3 s'y lisent entièrement.
9. **`df -h` brut dans les données.** La ligne complète de `df -h` est conservée dans `run.json`
   (gitignoré) ; aucun chemin absolu de la machine ne figure dans les fichiers versionnés.
10. **Harnais étendu pendant la campagne.** La sonde lit `/proc/1/environ` (preuve de la variable) et
    `attach` accepte `--lot`/`--reset` (borne d'exécution). Après la campagne, la sous-commande
    `tunables` a été ajoutée pour rendre la preuve reproductible (voir « Protocole »). Aucune autre
    modification n'a été faite : ni le code de production, ni l'image, ni `crible.toml`.

### Conclusion

**La lecture.** Les trois variantes de réglage de l'allocateur ne font pas baisser l'anonyme retenu
d'au moins la dispersion de V0 (45,3 MiB) : V1 le laisse à −7,1 MiB de la moyenne, V2 le fait monter
de 79,8 MiB et V3 de 56,5 MiB. La ligne « Aucune variante n'a d'effet » d'ADR-0157, décision 5,
s'applique : la rétention relève d'une **structure vivante ou d'une fragmentation**, sans que la
campagne départage les deux.

**Trois réserves obligatoires (ADR-0158, décision 3).**

- **V2 ne teste pas la fragmentation que `malloc_trim(0)` rendrait.** Sur glibc 2.36,
  `trim_threshold=0` ne rend que le haut du tas, c'est-à-dire l'arène principale, et rétrécit le tas
  des autres arènes ; `malloc_trim(0)` rend en plus, depuis glibc 2.8, les pages libres entières au
  milieu des arènes, par `madvise`. V2 ne ferme donc pas la ligne « fragmentation ». C'est une
  **correction déclarée** de l'équivalence inexacte de la recommandation 2 de la campagne du
  2026-11-12 : cette recommandation est citée ici, la section du 2026-11-12 n'est pas modifiée.
- **L'état (e) est un fait, hors de la règle de lecture.** Le redémarrage avec l'index présent rend la
  rétention : l'anonyme passe de 2 276,6 à 2 375,1 MiB en (d) à 1 342,7 à 1 513,0 MiB en (e) selon la
  variante, soit une baisse de 763,5 à 1 032,4 MiB. La rétention n'est donc pas nécessaire pour
  servir l'index. Le fait est consigné sans conclusion sur sa cause.
- **La baisse absolue de l'anonyme en (b) et en (c) sous les trois variantes est une observation.**
  Le chargement de la session coûte 1 309,6 à 1 329,4 MiB en (b) sous V1 à V3, contre une moyenne de
  V0 de 1 516,5 MiB (baisse de 187 à 207 MiB), et l'état (c) vaut 2 276,2 à 2 367,9 MiB contre une
  moyenne de V0 de 2 490,25 MiB (baisse de 122 à 214 MiB). La règle de lecture porte sur (c)−(b), pas
  sur les niveaux absolus : ces baisses ne sont la confirmation d'aucune cause.

**Les arènes ne portent pas la rétention.** V1 supprime les arènes (0 région de 65 536 kB à tous les
états, contre 16 à 52 sous V0) et garde la même rétention que V0 : +966,6 MiB contre +951,1 et
+996,4 MiB, soit −7,1 MiB de la moyenne, très en dessous du seuil. Et si V2 et V3 font baisser le
`Rss` des arènes à l'état (c) (964,6 et 812,7 MiB contre 1 350,1 et 1 308,8 MiB sous V0), leur
rétention ne baisse pas. Le nombre et le `Rss` des arènes ne suivent donc pas l'anonyme retenu.

**Ce que la campagne ne permet pas d'affirmer.**

- **Elle ne départage pas une structure vivante d'une fragmentation.** Les réglages d'allocateur ne
  font pas baisser la rétention ; V2 ne teste pas la fragmentation que `malloc_trim(0)` rendrait, et
  aucun profil ni compteur d'allocateur n'est dans la campagne (ADR-0157, décision 6).
- **Rien sur la production ni sur l'OOM non archivé de Legifacile.** L'échantillon fait 257 626
  passages, là où Legifacile en compte 9,6 M ; les chiffres disent la classe et le sens, jamais
  l'échelle. L'OOM d'attache du ticket 05 n'est pas archivé et demande sa propre reproduction.
- **Rien sur la latence.** Les variantes changent l'allocateur, donc le temps ; la campagne ne mesure
  pas la latence et aucune phrase sur ce sujet ne serait une affirmation mesurée.
- **Rien sur une stabilisation.** Six lots ne disent pas si la croissance s'arrête ou continue ; la
  campagne ne change pas ce point de la campagne du 2026-11-12.
- **Rien sur les autres leviers** : déchargement de session, autre allocateur, `madvise` sur les
  segments ne sont ni mesurés ni décidés ici (ADR-0151, décision 4 ; ADR-0105, décision 2).

**Recommandations, sans décision.** Le propriétaire choisit la suite, et toute campagne ou correction
nouvelle demande un **nouvel ADR** (ADR-0158, décision 4 ; ADR-0157, décisions 5 à 7).

1. **Profil de tas** (heaptrack, massif ou équivalent) sur une attache : il séparerait une structure
   vivante d'une fragmentation. Il demande un outil externe et un accès au processus, absents de
   `docs/07`, et ADR-0157, décision 6, l'exclut de la campagne réglée : c'est un ADR nouveau et une
   dépendance ou un outil à décider.
2. **Compteurs d'allocateur** (`mallinfo2`) : ils diraient la part libre dans les arènes. Ils
   demandent `libc`, absente de `docs/07` (ADR-0080, décision 3), et la forme de la ligne `memory`
   est fixée par ADR-0151, décision 2 : un ADR nouveau, et une mesure de la ligne.
3. **Essai de `malloc_trim(0)`** : il testerait la fragmentation que V2 ne rend pas. L'appeler sur un
   processus en service est un geste d'exploitation exclu par ADR-0157, décision 6 (`gdb -p`,
   `ptrace`) ; l'appeler depuis le code demanderait `libc`, donc un ADR (ADR-0080, décision 3), et
   une mesure avant toute correction.
4. **Examen des structures gardées après une attache** : une piste déjà écrite dans `docs/`
   consiste à retirer les textes de `CommitDelta.created` (axe (ii) du diagnostic de
   `CORRIGER-MEMOIRE-APRES-COMMIT`). ADR-0142 la range dans les alternatives écartées en v1 et la
   reporte en v2 : `@architecte` l'a jugée NON-COUVERTE par ADR-0066, elle demande un ADR propre, et
   le gain d'environ 5 Go sur `decisions` est prédit, pas mesuré. Aucun code n'a été lu et aucune
   conclusion n'en est tirée ici.

### Données brutes

- `data/mes17/raw/2026-11-22/V0-p1/` et `V0-p2/` — les deux processus de contrôle.
- `data/mes17/raw/2026-11-22/V1/`, `V2/`, `V3/` — un parcours par variante, avec `run.json`
  (variante, `GLIBC_TUNABLES` exact, image, VM, garde), `tunables.txt` (preuve `--list-tunables`),
  `tunables-proof.txt` pour V2, `states/*.json` (trois lectures par état), `logs/*.jsonl`,
  `queries/pass.json`, `attach/campaign.json`, `delete.json`, `restarts.json`, `summary.json`,
  `journal-check.json`, `memory-lines.json`.
- `data/mes17/raw/2026-11-22/tunables-baseline.txt` — la sortie de référence, sans variable.
- `data/mes17/raw/2026-11-22/` occupe 6,9 Mio ; l'image `crible:t05-suite-mes17` est conservée, le
  volume `crible-mes17-data` a été supprimé après la campagne, et aucun conteneur `crible-mes17` ni
  aucune sonde ne subsiste.
