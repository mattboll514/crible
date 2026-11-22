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

## Troisième campagne, 2026-11-22 : essai de `malloc_trim(0)` (ADR-0159)

- **Tâche** : `T05-ESSAI-MALLOC-TRIM` (v1.1), suite du ticket 05 de Legifacile. Troisième campagne de
  MES-17, décidée par ADR-0159 (accepté le 2026-11-22). L'appel se fait dans le processus du serveur,
  par `gdb -p`, depuis un conteneur annexe jetable. **La campagne ne corrige rien** : `malloc_trim`
  n'entre ni dans le code, ni dans l'image, ni dans `crible.toml` (ADR-0159, décision 7).
- **Commit** : `d266d633fe9997016eb76139d0b512ca021d24bb`, branche `tache/T05-ESSAI-MALLOC-TRIM`.
  Aucun code de production n'a été modifié : les seuls fichiers écrits sont cette section, le harnais
  `tools/bench/mes17/` (dont `gdb.Dockerfile`) et les données brutes sous
  `data/mes17/raw/2026-11-22/trim-p1/`, `trim-p2/` et `trim-selftest/` (gitignorées).
- **Machine** : MacBook Air M3 (`Mac15,12`, 8 cœurs, 24 Go), macOS 15.7.7. **En conteneur** : Docker
  Desktop 28.5.1 (build `e180ab8`), VM `aarch64` de **10 420 109 312 octets (9,71 Gio)**, 8 vCPU,
  **aucune limite `--memory`** (ADR-0135, décision 5), noyau `6.10.14-linuxkit`, taille de page
  **4 096 octets** lue par `getconf PAGESIZE` dans la sonde. Même machine et même VM que les deux
  campagnes précédentes.
- **Disque** : relevé par parcours (`df -h .`, `df_start` de `run.json` et `df_end` de `summary.json`) :
  **93 Gio** libres au début de `trim-p1` et **98 Gio** à sa fin, **98 Gio** au début de `trim-p2` et
  **99 Gio** à sa fin. Toutes ces valeurs sont au-dessus du plancher de 15 Gio d'ADR-0159, décision 6.
- **Image mesurée** : `crible:t05-suite-mes17`,
  `sha256:dc800f534ff50589038b0ca5d7a4fd9336a3259c45b70fdb00408b84366a8080`, **non reconstruite**
  (ADR-0159, contexte) ; elle n'a pas de shell, comme lors de la seconde campagne.
- **Image annexe** : `crible-mes17-gdb:2026-11-22`,
  `sha256:dcc33b255269a3929a03d0885e3a6b28ed51d4cd1c19326218f453fe364825b7`, construite depuis
  `tools/bench/mes17/gdb.Dockerfile` : base `debian:bookworm-slim` épinglée par digest
  (`sha256:734a72b3d6142a71993b4bc580ca01d8ca687f77a2c422d96a961d3376b179bc`, même Debian que l'étage
  `runtime` de `docker/Dockerfile`), paquet `gdb` **13.1-3** et rien d'autre, glibc
  `2.36-9+deb12u14`. Le téléchargement de la base et du paquet est celui qu'autorise ADR-0159,
  décision 2 ; l'image est ensuite utilisée en `--pull=never`.
- **Jeu de données** : l'échantillon Legifacile de la première campagne, `data/legifacile-sample`,
  six lots réels, 205 261 documents et 257 626 passages ; `queries.jsonl` présent mais **aucune
  requête n'est lancée** (les états (d) et (e) ne sont pas rejoués).
- **Configuration** : `data/mes17/crible.toml`, identique à la seconde campagne ; volume nommé
  jetable `crible-mes17-data`, recréé pour cette campagne et supprimé après. Aucune variable
  `GLIBC_TUNABLES` : les deux parcours sont sans variante (`--variant V0`, variable absente, vérifié
  par `docker inspect` et `null` dans `run.json`).
- **Exploitation** : `legifacile-dev-crible-1` était **arrêté** (`Exited (0)`) avant le premier
  parcours et l'est resté après le dernier, vérifié au début et à la fin ; il n'a jamais été touché.
  Les deux conteneurs de base de données au repos (`poc-presidio-db-1`, `legifacile-dev-postgres-1`)
  n'ont pas été touchés : 13,4 puis 13,8 MiB et 226,2 puis 226,3 MiB de mémoire, relevés par
  `docker stats --no-stream` au début et à la fin. Seuls `crible-mes17`, `crible-mes17-gdb`,
  `crible-mes17-selftest`, le volume `crible-mes17-data`, les sondes `postgres:17 --rm` et le port
  18017 ont servi. La production n'a été ni lue ni approchée (ADR-0032).

### Protocole

**Deux processus, un parcours chacun** (ADR-0159, décision 4), sans variante, sur le même volume et
un conteneur neuf par processus :

| Étape | Description |
|---|---|
| (a) | processus neuf, aucun index |
| (b) | index créé, avec la ligne `session_load` |
| (c) | six lots attachés |
| trim | premier `malloc_trim(0)`, valeur de retour notée |
| (c′) | relevé après le premier trim |
| trim | second appel, contrôle d'idempotence |
| (c″) | relevé après le second appel |
| `DELETE` | suppression de l'index |
| (f) | relevé après le `DELETE` |
| trim | `malloc_trim(0)` après le `DELETE` |
| (f′) | relevé après ce trim |

**Aucun trim entre (b) et (c)** : il perturberait le tas avant l'attache et détruirait la référence
(c)−(b). Les états (d) et (e) d'ADR-0157 ne sont pas rejoués.

**Relevés** : trois lectures à 3 secondes d'intervalle par état, mêmes sources et même sonde que la
seconde campagne ; le chiffre qui fait foi reste l'anonyme du cgroup (`anon` de `memory.stat`) lu par
`read_once`, et les écarts sont calculés sur les octets avant arrondi. Le harnais `measure.py` a reçu
trois états de plus (`c1-after-trim`, `c2-after-trim2`, `f1-after-trim3`), les sous-commandes `trim` et
`trim-selftest`, et l'option `--campaign trim` pour `init`, `summary` et `journal-check`. Les deux
ajouts de sortie (`df_end` dans `summary.json`, clés `session_load_expected` et
`session_load_conforme` de `journal-check.json`) sont **conditionnés à `--campaign trim`**. Cinq
champs, eux, restent **inconditionnels** : `campaign` dans tout `summary.json`, et `annex_image`,
`annex_image_id`, `annex_base_digest`, `trim_timeout_s` dans tout `run.json` (`null` hors troisième
campagne). Aucun relevé, aucun état et aucune transition ne change : la seconde campagne se rejoue
avec les mêmes chiffres, et ses fichiers de sortie gagnent seulement ces cinq champs.

**Appel au repos seulement** : chaque `trim` commence par `GET /v1/jobs?state=queued` et
`?state=running`, exige deux listes vides, et l'affiche (`at rest: queued=0 running=0`) avant
d'appeler. Le contrôle porte sur la **file de jobs** : cette campagne ne lance aucune requête cliente,
donc aucune requête n'était en vol de ce côté. Aucun job n'était en cours aux six appels.
`trims.json` écrira désormais les compteurs (`at_rest`) ; les fichiers de **cette** campagne ne les
portent pas, le champ ayant été ajouté après coup, et la preuve du contrôle y est le code : un
compteur non nul aurait arrêté le parcours par `fail()`, jamais laissé passer un trim.

**Appel borné et échouant franchement** : `docker run --rm --pull=never --name crible-mes17-gdb
--pid container:crible-mes17 --cap-add SYS_PTRACE --entrypoint gdb crible-mes17-gdb:2026-11-22 -p 1
-batch -ex 'set sysroot /proc/1/root' -ex 'file /proc/1/root/proc/1/exe' -ex 'call (int)
malloc_trim(0)'`, sous un délai borné de **120 s** (`TRIM_TIMEOUT`). Un blocage détruit l'annexe et
**déclare le parcours en échec** ; aucune valeur n'est substituée. Aucun appel n'a bloqué : 0,3 à
0,6 s chacun. **Aucune option n'élargit les privilèges au-delà de `--pid=container:` et
`--cap-add SYS_PTRACE`**, comme ADR-0159, décision 1 l'autorise : `--rm`, `--name` et `--entrypoint`
sont mécaniques (conteneur jetable nommé, entrée `gdb`), `--pull=never` relève de la décision 2, et
les deux `-ex` sont des **arguments `gdb`**, non des options Docker.

**Auto-test du harnais, avant la campagne** (ADR-0159, décision 3) : un conteneur de l'image annexe
lui-même, **sans Crible**, un `awk` qui alloue 40 000 blocs de 4 KiB (environ 156 MiB de tas), les
libère, puis dort. La sonde lit l'anonyme avant et après l'appel. Résultat :
`malloc_trim(0)` rend **1**, l'anonyme passe de **160,1 à 6,9 MiB** (−160 616 448 octets), l'appel
dure **0,3 s** et le délai borné s'applique. L'auto-test a donc montré les trois points exigés :
l'annexe atteint le processus visé, `gdb` rend la valeur de retour, la mesure d'anonyme suit
(`data/mes17/raw/2026-11-22/trim-selftest/selftest.json`).

### Résultats

MiB = 1 048 576 octets. Chaque valeur est la **médiane des trois lectures** ; « disp. » est la plus
grande dispersion des trois lectures sur `current`, `anon`, `file`, `RssAnon` et `Rss` des arènes.
Les écarts sont calculés sur les octets, avant arrondi. Les comparaisons sont internes à cette
campagne (ADR-0157, `:101-102`) : le chiffre de 45,3 MiB du 2026-11-22 n'est pas réutilisé.

**Processus 1 (`trim-p1`)**

| État | `current` | `anon` | `file` | `RssAnon` | `VmSwap` | arènes | `Rss` arènes | disp. |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| (a) neuf | 9,1 | 7,8 | 0,1 | 7,4 | 0,0 | 16 | 0,4 | 0,1 |
| (b) index créé | 2 711,0 | 1 499,6 | 1 204,1 | 1 499,4 | 0,0 | 34 | 741,5 | 0,1 |
| (c) six lots | 5 141,0 | 2 714,4 | 2 393,1 | 2 714,0 | 0,0 | 57 | 1 601,8 | 0,0 |
| (c′) après 1er trim | 4 612,8 | 2 066,9 | 2 393,1 | 2 066,4 | 0,0 | 57 | 990,8 | 0,0 |
| (c″) après 2e appel | 4 595,3 | 2 042,4 | 2 393,1 | 2 042,1 | 0,0 | 57 | 966,3 | 0,0 |
| (f) `DELETE` | 3 801,7 | 2 025,4 | 1 639,1 | 2 025,1 | 0,0 | 56 | 949,1 | 0,0 |
| (f′) après 3e trim | 3 683,9 | 1 925,2 | 1 639,1 | 1 925,0 | 0,0 | 56 | 849,0 | 0,0 |

**Processus 2 (`trim-p2`)**

| État | `current` | `anon` | `file` | `RssAnon` | `VmSwap` | arènes | `Rss` arènes | disp. |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| (a) neuf | 9,0 | 7,8 | 0,1 | 7,4 | 0,0 | 16 | 0,4 | 0,5 |
| (b) index créé | 2 711,1 | 1 499,8 | 1 204,0 | 1 499,3 | 0,0 | 35 | 741,5 | 0,1 |
| (c) six lots | 4 870,3 | 2 441,7 | 2 393,0 | 2 441,4 | 0,0 | 53 | 1 362,3 | 0,0 |
| (c′) après 1er trim | 4 562,4 | 2 063,1 | 2 393,0 | 2 062,8 | 0,0 | 53 | 987,3 | 0,0 |
| (c″) après 2e appel | 4 560,3 | 2 055,7 | 2 393,0 | 2 055,4 | 0,0 | 53 | 979,9 | 0,0 |
| (f) `DELETE` | 3 784,0 | 2 055,8 | 1 639,1 | 2 055,4 | 0,0 | 53 | 980,0 | 0,0 |
| (f′) après 3e trim | 3 692,6 | 1 960,7 | 1 639,1 | 1 960,4 | 0,0 | 53 | 884,9 | 0,0 |

**Transitions en anonyme, par processus (MiB)**

| Processus | (a)→(b) | **(b)→(c)** | **(c)→(c′)** | (c′)→(c″) | (c″)→(f) | **(f)→(f′)** |
|---|---:|---:|---:|---:|---:|---:|
| `trim-p1` | +1 491,8 | **+1 214,8** | **−647,5** | −24,5 | −17,0 | **−100,2** |
| `trim-p2` | +1 492,0 | **+941,9** | **−378,6** | −7,4 | +0,1 | **−95,1** |

Octets exacts : `trim-p1` +1 273 872 384 sur (c)−(b), −679 026 688 sur (c)−(c′),
−25 612 288 sur (c′)−(c″), −105 041 920 sur (f)−(f′) ; `trim-p2` +987 648 000, −397 021 184,
−7 737 344, −99 737 600.

**Valeurs de retour de `gdb`** — trois appels par processus, dans l'ordre :

| Appel | `trim-p1` | `trim-p2` | Durée |
|---|---:|---:|---|
| après (c) | **1** | **1** | 0,6 s / 0,3 s |
| second appel (idempotence) | **1** | **1** | 0,3 s / 0,3 s |
| après le `DELETE` | **1** | **1** | 0,3 s / 0,3 s |

**Dispersion intra-état** : la plus grande dispersion des trois lectures vaut **0,46 MiB**
(processus 2, état (a), sur `current`) ; sur l'anonyme, elle vaut **0,39 MiB** à ce même état (a) et
**0,04 MiB ou moins** dans tous les autres états. Les trois lectures sont donc stables.

**Garde mémoire** : jamais déclenchée. Le minimum de `MemAvailable` de la VM est **6 517 864 kB**
(`trim-p1`, état (c)), très au-dessus de 1 Gio. Aucun échange : `VmSwap` vaut 0 à tous les états des
deux processus, et `SwapFree` reste à **4 178 772 kB** du début à la fin.

**Contrôle du journal** : les deux parcours sont conformes. Les quatre `step` d'ADR-0151
apparaissent ; chaque parcours porte 7 lignes `job_end` (six `attach_batch`, un `delete_index`), soit
**un `job_end` par lot attaché** (6/6) ; `delete_before_rename` et `delete_after_removal` partagent le
`job_id` du `DELETE` ; `session_load` apparaît **une seule fois**, à l'état (b) : aucun redémarrage
n'a été rejoué, comme le prévoit ADR-0159, décision 4.

**Durées** (aucune conclusion de latence n'en est tirée) : le parcours 1 va de 11:32:17 à 11:42:28
(10 min 11 s) et le parcours 2 de 11:42:31 à 11:52:27 (9 min 56 s). Attache des six lots :
**533,9 s** et **533,8 s** (articles 75,5 / 75,4 s ; jorf 111,6 / 99,5 s ; decisions 123,7 / 132,7 s ;
titres 15,1 / 18,1 s ; sarde 39,2 / 36,2 s ; dole 168,8 / 171,9 s). Création de l'index : 11,7 s puis
2,2 s ; `DELETE` : 3,0 s dans les deux cas ; un relevé d'état dure environ 7 secondes.

### Lecture (ADR-0159, décision 5)

**Le seuil est la dispersion entre les deux processus de cette campagne sur (c)−(b)**, calculée sur
les octets : moyenne **1 078,38 MiB**, dispersion **286 224 384 octets, soit 272,96 MiB** (1 214,86
contre 941,89). C'est un seuil large : la référence (c)−(b) diffère de 273 MiB entre deux processus
de la même image, du même jeu et de la même machine.

**L'effet se lit comme une baisse.** Les deux processus donnent une baisse (c)−(c′) **au-dessus du
seuil** :

| Processus | (c)−(b) | (c)−(c′) | Baisse ≥ seuil (272,96) ? | Part de la rétention rendue |
|---|---:|---:|---|---:|
| `trim-p1` | +1 214,86 | −647,57 | **oui** | 53,3 % |
| `trim-p2` | +941,89 | −378,63 | **oui** | 40,2 % |

Par la première ligne de lecture d'ADR-0159, décision 5, **cette part de l'anonyme retenu est
attribuée à une fragmentation que `malloc_trim(0)` rend** : des pages libres à l'intérieur des arènes,
que `trim_threshold=0` ne rend pas (réserve technique d'ADR-0158, `:60-66`). Le premier appel rend
`1` dans les deux processus, ce qui confirme que des pages ont effectivement été rendues.

**Ce que la rétention n'est pas, et ce qui reste.** L'observation de la seconde campagne est
reprise : les arènes ne portent pas la rétention. À l'état (c), les 57 et 53 arènes des deux
processus portent 1 601,8 et 1 362,3 MiB de `Rss`, alors que l'anonyme atteint 2 714,4 et 2 441,7
MiB ; au premier trim, le `Rss` des arènes baisse de 611,0 et 375,0 MiB, du même ordre que la baisse
de l'anonyme. Après les trois appels, il **reste** 542,9 MiB (`trim-p1`) et 555,9 MiB (`trim-p2`)
d'anonyme au-dessus de l'état (b) en (c″), et 425,6 et 460,9 MiB en (f′).

**Ce reste n'est classable par aucune ligne d'ADR-0159, décision 5.** La dernière ligne de cette
décision — « après un premier appel rendant `1` et un second rendant `0`, ce qui reste est une
structure vivante ou une fragmentation non rendable » — suppose un second appel rendant `0` ; son
antécédent est **faux ici** : le second appel rend `1` sur les deux processus, et la baisse associée
(24,43 et 7,38 MiB) est sous le seuil. La lecture est donc **restreinte** : les 542,9 et 555,9 MiB
restants en (c″), et 425,6 et 460,9 MiB en (f′), sont une **observation** que la décision 5 ne classe
pas. La campagne ne dit pas s'ils relèvent d'une structure vivante ou d'une fragmentation non
rendable.

**Le second appel n'a pas rendu `0`.** Le contrôle d'idempotence d'ADR-0159, décision 3, attendait
`0` et aucune baisse supplémentaire ; les deux processus rendent **`1`**, avec une baisse de 24,43 et
7,38 MiB. Cette baisse est **sous le seuil** : c'est une **observation**, pas un effet. La campagne
consigne le fait sans l'interpréter.

**(f)−(f′)** : le trim après le `DELETE` rend `1` et fait baisser l'anonyme de 100,18 et 95,12 MiB,
alors que le `DELETE` lui-même n'en rend que −17,0 et +0,1 MiB. ADR-0159 ne donne pas de ligne de
lecture pour (f)−(f′) — la règle porte sur (c)−(c′) — : c'est une **observation**, pas un effet.

**Ce que cette campagne ne conclut pas.** Ni « fuite », ni « pas de fuite » : une part est rendue par
`malloc_trim(0)` et une autre reste, sans que la campagne dise laquelle est une structure vivante.
Rien sur l'échelle de production — l'échantillon fait 257 626 passages, là où Legifacile en compte
9,6 M —, rien sur la latence, rien sur le coût d'un `malloc_trim` périodique, rien sur l'OOM
d'attache du ticket 05.

### Réserves

1. **Deux `-ex` s'ajoutent à la commande d'ADR-0159, décision 1.** Sans `set sysroot /proc/1/root` et
   `file /proc/1/root/proc/1/exe`, `gdb` 13.1 refuse de lire les fichiers du conteneur mesuré
   (`target:/usr/bin/... : Operation not permitted`) et ne connaît aucun symbole `malloc_trim` :
   la forme schématique de l'ADR ne rend alors aucune valeur. Les deux `-ex` font lire les symboles
   depuis la racine du conteneur mesuré, à travers `/proc/1/root`, ce qui ne demande aucune capacité
   au-delà de `SYS_PTRACE`, et **aucune option n'élargit les privilèges au-delà de `--pid=container:`
   et `--cap-add SYS_PTRACE`** : `--rm`, `--name` et `--entrypoint` sont mécaniques, `--pull=never`
   relève de la décision 2, et les deux `-ex` sont des arguments `gdb` (voir « Protocole »). La
   déviation est déclarée ici et dans `tools/bench/mes17/README.md` ; la
   commande exacte de chaque appel est conservée dans `trims.json`.
2. **Le seuil de cette campagne est large.** La dispersion entre processus sur (c)−(b) vaut
   272,96 MiB, contre 45,3 MiB pour V0 le 2026-11-22 : la référence de l'attache est bien moins
   reproductible d'un processus à l'autre que lors de la seconde campagne. Les deux baisses (c)−(c′)
   dépassent ce seuil, mais la marge du processus 2 n'est que de 105,7 MiB.
3. **La rétention (c)−(b) est plus haute que celle de la seconde campagne** : 1 214,86 et 941,89 MiB
   ici, contre 951,1 et 996,4 MiB pour V0 alors. C'est une **observation** : les comparaisons sont
   internes à la campagne (ADR-0157, `:101-102`), et aucun chiffre de la seconde campagne n'est
   réutilisé, ni comme valeur, ni comme seuil.
4. **Le contrôle d'idempotence n'a pas reproduit la valeur attendue** (deux `1` au lieu de `0`). La
   baisse associée est sous le seuil et reste une observation ; la campagne ne dit pas pourquoi le
   second appel rend encore des pages.
5. **Deux processus seulement.** Deux parcours ne donnent pas de distribution : la dispersion est un
   écart entre deux valeurs, pas un écart type, comme dans les deux campagnes précédentes.
6. **Écarts d'exécution déjà consignés** et reconduits : attache en six commandes (borne de 30 min
   par commande), volume réutilisé entre les deux parcours (l'index est supprimé à l'état (f)), lots
   dérivés de la première campagne non re-dérivés, `smaps` non conservé, taille de page lue par
   `getconf`, `df -h` brut conservé dans les données brutes — `df_start` dans `run.json` et `df_end`
   dans `summary.json`, tous deux gitignorés — et aucun chemin absolu de la machine dans les fichiers
   versionnés.
7. **Harnais étendu pendant la campagne** : états `c1-after-trim`, `c2-after-trim2` et
   `f1-after-trim3` ; sous-commandes `trim` et `trim-selftest` ; option `--campaign trim` ; clés
   `session_load_expected` et `session_load_conforme` de `journal-check.json` et `df_end` de
   `summary.json`, conditionnées à cette option ; et cinq champs inconditionnels — `campaign` dans
   `summary.json`, `annex_image`, `annex_image_id`, `annex_base_digest` et `trim_timeout_s` dans
   `run.json` — qui ne changent aucun chiffre. Aucune autre modification : ni le code de production,
   ni l'image, ni `crible.toml`.

### Recommandations, sans décision

Le propriétaire choisit la suite, et **toute correction demande un ADR distinct** (ADR-0159,
décision 7). Aucune n'est décidée ici.

1. **Si une correction est envisagée** — un `malloc_trim(0)` appelé depuis le code demande `libc`,
   absente de `docs/07` (ADR-0080, décision 3) —, elle demande un ADR de contrat et de dépendance,
   **et sa propre mesure de latence et de coût**. La campagne actuelle ne mesure ni l'un ni l'autre :
   les 0,3 à 0,6 s d'un appel par `gdb` mesurent l'outil, pas une correction en service.
2. **Si la part restante doit être attribuée** — 542,9 et 555,9 MiB au-dessus de (b) en (c″) —, le
   moyen suivant est un compteur d'allocateur (`mallinfo2`) ou un profil de tas. L'un et l'autre
   restent exclus par ADR-0157, décision 6, et demandent un ADR propre et une dépendance ou un outil
   externe.
3. **Rien n'est proposé sur la production** : l'échelle, la latence et l'OOM du ticket 05 demandent
   leurs propres reproductions.

### Données brutes

- `data/mes17/raw/2026-11-22/trim-p1/` et `trim-p2/` — un processus chacun, avec `run.json` (image
  mesurée et son identifiant, image annexe et son identifiant, digest de la base, VM, garde, `df`),
  `states/*.json` (trois lectures par état), `logs/*.jsonl`, `attach/campaign.json`, `create.json`,
  `delete.json`, `trims.json` (les trois appels : commande exacte, valeur de retour, durée, sortie de
  `gdb`), `summary.json`, `journal-check.json`, `memory-lines.json`.
- `data/mes17/raw/2026-11-22/trim-selftest/selftest.json` — l'auto-test de l'annexe, avant la
  campagne.
- Les images `crible:t05-suite-mes17` et `crible-mes17-gdb:2026-11-22` sont conservées ; le volume
  `crible-mes17-data` a été supprimé après la campagne, et aucun conteneur `crible-mes17*` ne
  subsiste.

## Campagne de contrôle d'ADR-0160, 2026-11-22

- **Tâche** : `T05-CORRIGER-MALLOC-TRIM` (v1.1), correction du ticket 05 de Legifacile. Campagne de
  contrôle d'ADR-0160, décision 6, accepté le 2026-11-22 : c'est la mesure qui précède la fusion du
  `malloc_trim(0)` appelé par le serveur à la fin de chaque job.
- **Commit** : `3f334c940246380199158c97ec18ee868b4828d7`, branche
  `tache/T05-CORRIGER-MALLOC-TRIM`. Aucun code de production n'a été modifié : les seuls fichiers
  écrits sont cette section, le harnais `tools/bench/mes17/` et les données brutes sous
  `data/mes17/raw/2026-11-22/ctl-ref-p1/`, `ctl-ref-p2/`, `ctl-fix-p1/` et `ctl-fix-p2/`
  (gitignorées).
- **Machine** : MacBook Air M3 (`Mac15,12`, 8 cœurs, 24 Go), macOS 15.7.7. **En conteneur** : Docker
  Desktop 28.5.1, VM `aarch64` de **10 420 109 312 octets (9,71 Gio)**, 8 vCPU, **aucune limite
  `--memory`** (ADR-0135, décision 5), noyau `6.10.14-linuxkit`, taille de page **4 096 octets** lue
  par `getconf PAGESIZE` dans la sonde. Même machine et même VM que les deux campagnes précédentes.
- **Disque** : **72 Gio** libres avant la campagne, **74 Gio** au démarrage du premier parcours et
  **73 Gio** à la fin du dernier (`df -h .`, `df_start` de `run.json`, `df_end` de `summary.json`),
  tous au-dessus du plancher de 15 Gio.
- **Images** : la référence est `crible:t05-suite-mes17`,
  `sha256:dc800f534ff50589038b0ca5d7a4fd9336a3259c45b70fdb00408b84366a8080`, construite par
  l'orchestrateur depuis `31e9804` et **non reconstruite**. La correction est `crible:t05-trim-fix`,
  `sha256:bcfad74627a38dd55838c58c83761b5a80f991a399cbd17954a48e622ff4c363`, construite depuis
  l'arbre propre (`3f334c9`) par `docker buildx build --platform linux/arm64 --load -t
  crible:t05-trim-fix -f docker/Dockerfile .`, le cache de `cargo chef` servant les étages de
  dépendances (journal `data/mes17/build-t05-trim-fix.log`, 7 min 45 s dont 17 s de compilation du
  workspace). L'orchestrateur a vérifié qu'aucun fichier de `crates/`, `Cargo.toml`, `Cargo.lock`,
  `rust-toolchain.toml` ni `docker/` n'a changé entre `31e9804` et `07b3a05` : la correction est donc
  la seule différence entre les deux images.
- **Réglage de journal** : `RUST_LOG=info,crible_server::trim=debug`, **le même sur les deux
  images**, posé par `docker run -e` et relu par `docker inspect` dans les quatre `run.json`. Il
  laisse toutes les autres lignes au défaut `info` de l'image (ADR-0096) et n'allume que la cible de
  la ligne `trim` ; l'image de référence ne contient pas ce module et n'émet donc aucune ligne
  `trim`, ce que les relevés confirment.
- **Jeu de données** : l'échantillon Legifacile de la première campagne, `data/legifacile-sample`,
  six lots réels, **205 261 documents et 257 626 passages** ; `queries.jsonl`, 300 requêtes ; modèle
  `mdenseon` monté en lecture seule.
- **Configuration** : `data/mes17/crible.toml`, identique aux deux campagnes précédentes, sans
  variante `GLIBC_TUNABLES` (`null` dans les quatre `run.json`).
- **Exploitation** : `legifacile-dev-crible-1` était **arrêté** (`Exited (0)`) au début et à la fin de
  la campagne, et n'a jamais été touché. Les deux conteneurs de base de données au repos
  (`poc-presidio-db-1`, `legifacile-dev-postgres-1`) n'ont pas été touchés : 13,4 puis 13,4 MiB et
  226,3 puis 226,3 MiB de mémoire, relevés par `docker stats --no-stream` au début et à la fin.
  Seuls `crible-mes17`, le volume `crible-mes17-data`, les sondes `postgres:17 --rm` et le port
  18017 ont servi. La production n'a été ni lue ni approchée (ADR-0032).

### Protocole

**Deux images, deux processus chacune**, sur le même jeu, la même configuration et la même machine
(ADR-0160, décision 6). Chaque parcours part d'un conteneur neuf et d'un **volume neuf** :

| Étape | Description |
|---|---|
| (a) | processus neuf, aucun index |
| (b) | index créé, avec la ligne `session_load` |
| (c) | six lots attachés, pendant une charge de 300 requêtes hybrides `k = 10` |

Les états (d), (e), (f) et (g) d'ADR-0157 ne sont pas rejoués, et la campagne s'arrête à (c). Un
relevé d'état garde la forme des campagnes précédentes : **trois lectures à 3 secondes
d'intervalle**, mêmes sources, même sonde, et le chiffre qui fait foi reste l'anonyme du cgroup
(`anon` de `memory.stat`) lu par `read_once` ; les écarts sont calculés sur les octets avant arrondi.

**Volume neuf par processus, et non volume réutilisé.** La troisième campagne supprimait l'index à
l'état (f), ce qui lui permettait de garder le volume entre deux parcours. La présente campagne
s'arrête à (c) : l'index n'est pas supprimé, et le volume est donc détruit puis recréé entre deux
processus, de sorte que l'état (a) n'ait aucun index dans les quatre parcours. Les fichiers
d'exploitation (`jobs.sqlite`, `analytics.sqlite`) repartent avec lui ; c'est un écart déclaré avec
les deux campagnes précédentes. Le protocole est **identique sur les quatre parcours**, donc il ne
biaise pas la lecture de (b)→(c) ; il déplace des niveaux absolus, il n'en compare pas.

**Charge de requêtes.** Les 300 requêtes de `queries.jsonl`, une fois chacune, en mode `hybrid`,
`k = 10`, sur l'index `mes17` — le seul que l'instance connaisse et celui que la campagne attache.
Elles sont lancées **par le harnais**, en 8 fils Python, à raison d'une toutes les 1,80 s sur une
fenêtre de **540 s** ouverte au premier `POST batches` de l'attache. Aucune requête ne tourne en
arrière-plan du shell. Chaque requête enregistre son début, en secondes epoch et en RFC 3339, et sa
fin, en secondes epoch seulement, dans `queries/load.json` et `queries/load-rows.jsonl` ;
`trim-lines` compte ensuite, d'après l'horodatage de chaque ligne `trim` du journal, combien de
requêtes étaient **en vol à cet instant**.

**La fenêtre de 540 s est plus courte que l'attache, écart déclaré.** L'attache dure 546 à 636 s
selon le parcours : la sixième fin de job tombe donc **hors charge** dans les quatre parcours — la
dernière requête se termine 4,3 s (correction, processus 2), 9,4 s (référence, processus 1), 13,6 s
(référence, processus 2) et 96,3 s (correction, processus 1) **avant** ce `job_end`. **Aucune des 20
fins de job de la fenêtre n'a eu de requête en vol**, et aucun des six trims non plus : les six
trims sont mesurés, la sixième fin de job tombe hors charge, et la campagne ne l'affirme pas
couverte.

Trois points de cette charge sont des **choix déclarés**, que ni ADR-0160 ni la consigne ne fixent :

1. **La fenêtre de 540 s** visait la durée de l'attache des six lots, mesurée entre 546 s et 636 s
   dans cette campagne : elle est trop courte de 6 à 96 s. L'écart est déclaré ci-dessus et laissé
   tel quel, la campagne n'est pas rejouée. Pour un passage futur, le harnais est corrigé : prendre
   `--load-window` au moins égal à la durée de l'attache (640 s ici) ; `trim-lines` publie le
   recouvrement mesuré, il ne le suppose pas.
2. **Le débit d'une requête toutes les 1,80 s** avec 8 fils donne une charge répartie, pas continue :
   le temps cumulé en vol vaut 182,8 s sur 538,7 s dans le premier parcours, soit un rapport cyclique
   de 34 %. Avec 300 requêtes pour 540 s, aucune répartition ne peut couvrir la phase en continu.
3. **Les percentiles portent sur les requêtes réussies** (`200`), et les refus sont comptés à part :
   quelques recherches dépassent l'échéance de 2 000 ms de `crible.toml` et sont refusées en `504`
   `timeout`. Compter un refus à 2 000 ms comme une latence de recherche confondrait l'échéance avec
   le temps de recherche ; l'exclure sans le dire serait pire. Le nombre de refus est donc publié
   pour chaque image, à côté des percentiles.

Le harnais `tools/bench/mes17/measure.py` a reçu pour cela : l'option `--campaign control` (`up`,
`init`, `summary`, `journal-check`, `trim-lines`), l'option `--image-role` d'`init`, les options
`--load-window` et `--load-threads` d'`attach`, les sous-commandes `load` et `trim-lines`, et
l'enregistrement incrémental des requêtes. `trim-lines` écrit `trim-lines.json` (lignes `trim`,
compte des `trimmed = 1`, durées, recouvrement avec la charge). Aucun relevé, aucun état et aucune
transition des campagnes précédentes ne change : `--campaign second` et `--campaign trim` gardent
leurs états et leurs transitions ; les clés ajoutées aux `run.json` sont **additives**, sauf le
renommage de `commit` et `branch` en `harness_commit` et `harness_branch`, qui distingue le commit
de l'arbre du harnais de celui de l'image.

### Résultats

MiB = 1 048 576 octets. Chaque valeur est la **médiane des trois lectures** ; « disp. » est la plus
grande dispersion des trois lectures sur `current`, `anon`, `file`, `RssAnon` et `Rss` des arènes.
Les écarts sont calculés sur les octets, avant arrondi. Les comparaisons sont internes à cette
campagne (ADR-0157, `:101-102`) : ni 45,3 MiB ni 272,96 MiB ne sont réutilisés.

**Référence, processus 1 (`ctl-ref-p1`, `crible:t05-suite-mes17`)**

| État | `current` | `anon` | `file` | `RssAnon` | `VmSwap` | arènes | `Rss` arènes | disp. |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| (a) neuf | 9,0 | 7,8 | 0,1 | 7,4 | 0,0 | 16 | 0,4 | 0,1 |
| (b) index créé | 2 704,5 | 1 493,1 | 1 204,1 | 1 492,7 | 0,0 | 33 | 734,7 | 0,1 |
| (c) six lots | 5 200,5 | 2 769,7 | 2 394,2 | 2 769,2 | 0,0 | 60 | 1 711,2 | 0,0 |

**Référence, processus 2 (`ctl-ref-p2`)**

| État | `current` | `anon` | `file` | `RssAnon` | `VmSwap` | arènes | `Rss` arènes | disp. |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| (a) neuf | 9,4 | 8,2 | 0,1 | 7,8 | 0,0 | 17 | 0,4 | 0,5 |
| (b) index créé | 2 697,7 | 1 486,4 | 1 204,1 | 1 486,2 | 0,0 | 34 | 727,9 | 0,1 |
| (c) six lots | 5 182,0 | 2 752,7 | 2 394,2 | 2 752,4 | 0,0 | 58 | 1 694,6 | 0,0 |

**Correction, processus 1 (`ctl-fix-p1`, `crible:t05-trim-fix`)**

| État | `current` | `anon` | `file` | `RssAnon` | `VmSwap` | arènes | `Rss` arènes | disp. |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| (a) neuf | 9,0 | 7,8 | 0,1 | 7,4 | 0,0 | 16 | 0,4 | 0,1 |
| (b) index créé | 2 704,1 | 1 492,7 | 1 204,1 | 1 492,4 | 0,0 | 33 | 734,6 | 0,1 |
| (c) six lots | 4 748,0 | 2 113,0 | 2 394,2 | 2 112,7 | 0,0 | 63 | 1 058,5 | 0,0 |

**Correction, processus 2 (`ctl-fix-p2`)**

| État | `current` | `anon` | `file` | `RssAnon` | `VmSwap` | arènes | `Rss` arènes | disp. |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| (a) neuf | 9,0 | 7,8 | 0,1 | 7,4 | 0,0 | 16 | 0,4 | 0,1 |
| (b) index créé | 2 717,6 | 1 506,2 | 1 204,1 | 1 505,9 | 0,0 | 35 | 748,2 | 0,1 |
| (c) six lots | 4 727,7 | 2 140,1 | 2 394,2 | 2 139,8 | 0,0 | 65 | 1 085,7 | 0,0 |

**Transitions en anonyme, par processus (MiB)**

| Parcours | Image | (a)→(b) | **(b)→(c)** | (b)→(c), octets exacts |
|---|---|---:|---:|---:|
| `ctl-ref-p1` | référence | +1 485,3 | **+1 276,6** | 1 338 589 184 |
| `ctl-ref-p2` | référence | +1 478,2 | **+1 266,4** | 1 327 894 528 |
| `ctl-fix-p1` | correction | +1 484,9 | **+620,3** | 650 412 032 |
| `ctl-fix-p2` | correction | +1 498,4 | **+633,9** | 664 686 592 |

**Dispersion intra-état** : sur l'anonyme, la plus grande dispersion des trois lectures vaut
**0,42 MiB** (référence, processus 2, état (a)) et 0,04 MiB ou moins dans tous les autres états ; sur
les cinq champs du tableau, 0,5 MiB à ce même état. Les trois lectures sont stables.

**Garde mémoire** : jamais déclenchée. Le minimum de `MemAvailable` de la VM vaut **6 438 096 kB**
(référence, processus 1, état (c)) et 6 464 888, 6 886 772 et 6 919 736 kB dans les trois autres
parcours, tous très au-dessus de 1 Gio. Aucun échange : `VmSwap` vaut 0 à tous les états des quatre
parcours, et `SwapFree` reste à **4 178 260 kB** du début à la fin.

**Contrôle du journal** : les quatre parcours sont conformes. Chaque parcours porte 6 lignes
`job_end` (six `attach_batch`), **un `job_end` par lot attaché** (6/6), et **une seule** ligne
`session_load`, à l'état (b) : aucun redémarrage n'est rejoué. Les deux `step` de suppression
d'ADR-0151 sont absents, comme le prévoit une campagne qui s'arrête à (c) : `journal-check
--campaign control` vérifie `job_end` et `session_load` seulement, et le déclare dans
`journal-check.json` (`required_steps`).

**Durées** (aucune conclusion de latence n'en est tirée) : le relevé va de la première lecture (a) à
la dernière lecture (c) en **9 min 36 s**, **9 min 41 s**, **11 min 4 s** et **9 min 32 s**. Attache
des six lots : **549 s**, **555 s**, **636 s** et **546 s** (articles 75,5 / 81,4 / 105,6 / 75,4 s ;
jorf 108,6 / 120,8 / 147,8 / 105,5 s ; decisions 126,7 / 126,8 / 129,5 / 129,7 s ; titres 18,1 /
18,2 / 18,1 / 18,1 s ; sarde 39,2 / 36,2 / 39,1 / 39,2 s ; dole 180,8 / 171,9 / 196,0 / 177,9 s).
Création de l'index : 2,3 / 2,5 / 2,3 / 2,8 s. Un relevé d'état dure environ 7 secondes.

### Lecture : seuil, gain, et critère d'INV-02

**Le seuil est la dispersion entre les deux processus de référence de cette campagne sur
(c)−(b)**, calculée sur les octets : **10 694 656 octets, soit 10,20 MiB** (1 276,58 contre
1 266,36 MiB). C'est la lecture qu'en fait l'orchestrateur d'ADR-0157, décision 5 : les deux
processus de référence tiennent le rôle de V0. **Aucun chiffre antérieur n'est repris comme
seuil** ; les 272,96 MiB de la troisième campagne ne sont cités que pour situer l'ordre de grandeur,
et l'étroitesse des 10,20 MiB est une propriété de cette campagne-ci, non un resserrement de la
règle.

**Le sens est la baisse** (ADR-0158). Moyennes des deux processus, sur les octets : référence
**1 271,48 MiB**, correction **627,09 MiB**.

| Lecture | Référence | Correction | Gain (baisse de (c)−(b)) |
|---|---:|---:|---:|
| moyenne des deux processus | 1 271,48 MiB | 627,09 MiB | **644,39 MiB** (675 692 544 octets) |
| processus 1 | 1 276,58 MiB | 620,28 MiB | **656,30 MiB** |
| processus 2 | 1 266,36 MiB | 633,89 MiB | **632,47 MiB** |

Le gain vaut **63,2 fois le seuil** de cette campagne, et **chacun** des deux processus de la
correction dépasse le seuil à lui seul. La part de l'anonyme retenu par l'attache que la correction
rend vaut **50,7 %** (644,39 sur 1 271,48 MiB). Les deux comparaisons sont internes à cette
campagne. L'état (c) de la correction vaut 2 113,0 et 2 140,1 MiB, contre 2 769,7 et 2 752,7 MiB pour
la référence, avec des états (b) comparables (1 492,7 et 1 506,2 contre 1 493,1 et 1 486,4 MiB) : la
baisse porte donc bien sur la croissance de l'attache, pas sur le chargement de l'index.

**Lignes `trim`.** La correction émet **6 lignes `trim` par processus**, une par `job_end` d'attache,
toutes de niveau `DEBUG`, cible `crible_server::trim`, et **toutes avec `trimmed = 1`** : aucun appel
n'a été sauté, et chaque appel a rendu des pages. L'image de référence n'émet **aucune** ligne
`trim`, comme attendu. Les durées sont données **dans l'ordre chronologique** des six fins de job,
c'est-à-dire l'ordre des lots ; `trim-lines.json` les range, lui, par ordre croissant :

| Parcours | `duration_us`, ordre chronologique | médiane | maximum |
|---|---|---:|---:|
| `ctl-fix-p1` | 29 394 / 10 065 / 6 820 / 5 308 / 8 186 / 12 419 | 9 125,5 | 29 394 |
| `ctl-fix-p2` | 7 609 / 8 792 / 8 072 / 7 602 / 8 468 / 11 332 | 8 270,0 | 11 332 |

Chaque ligne suit son `job_end` de 6,8 à 31,2 ms à l'horloge du journal : par exemple
`13:14:01.596` pour le `job_end` et `13:14:01.627` pour le `trim` du premier lot du premier
processus, soit 31,2 ms. **Le coût en service d'un appel est donc de 5,3 à 29,4 ms**, mesuré dans le
processus du serveur, là où les 0,3 à 0,6 s de la troisième campagne mesuraient `gdb` et non la
correction (ADR-0160, « Conséquences »).

**Latence des 300 requêtes, et critère d'INV-02** (`docs/08:10` : aucune requête au-delà du p99 de
référence × 3). Percentiles sur les requêtes réussies, refus comptés à part :

| Parcours | Réussies | Refusées (`504 timeout`) | p50 | p95 | p99 |
|---|---:|---:|---:|---:|---:|
| `ctl-ref-p1` | 296 | 4 | 415,3 ms | 1 500,3 ms | **1 731,2 ms** |
| `ctl-ref-p2` | 298 | 2 | 543,5 ms | 1 530,9 ms | **1 821,9 ms** |
| `ctl-fix-p1` | 283 | 17 | 499,5 ms | 1 549,0 ms | **1 950,6 ms** |
| `ctl-fix-p2` | 295 | 5 | 439,9 ms | 1 459,3 ms | **1 658,6 ms** |

Le p99 de référence le plus haut vaut 1 821,9 ms ; trois fois cette valeur vaut **5 465,7 ms**. Le
p99 de la correction le plus haut vaut 1 950,6 ms, soit **1,07 fois le p99 de référence** et 36 % de
la borne. La plus lente des requêtes **réussies** vaut 2 012,2 ms (`ctl-fix-p1`), et la plus lente
des requêtes refusées 2 052,0 ms (`ctl-fix-p1`, `Q53`) : la forme **par requête** de la même borne
d'ADR-0160, décision 6 — trois fois le p99 de l'image de référence sous attache — tient donc aussi,
aucune requête ne dépassant 5 465,7 ms. (La borne d'ADR-0113, décision 1, vaut 3 × le p99 de la
phase de repos et n'est pas mesurée ici.) Les p50 et p95 des deux images
se recouvrent.

**Le critère d'INV-02 est satisfait sur la charge concurrente de l'attache**, et sur elle seule :
aucune requête n'a traversé un trim (réserve 1), donc l'effet d'un trim sur une requête en vol
**n'est pas observé** ; il est seulement **borné** par les durées mesurées, 5,3 à 29,4 ms.

**Ce que la campagne ne conclut pas.** Rien sur l'échelle de Legifacile (l'échantillon fait
257 626 passages pour 9,6 M), rien sur le pic pendant une attache, rien sur l'OOM du ticket 05, rien
sur les 627,09 MiB de (c)−(b) qui restent dans la correction — le trim les laisse en place, comme
ADR-0160, décision 8, l'annonce.

### Réserves

1. **Aucune requête n'était en vol à l'instant exact d'un `trim`.** Le recouvrement est mesuré, pas
   supposé : sur les six `job_end` de chaque parcours, **aucun** n'avait de requête en vol dans les
   quatre parcours. Le plus proche est à **5,3 ms** — une requête qui **se termine** juste avant un
   `job_end`, dans la référence processus 1 (`12:58:00.933` contre `12:58:00.938`) — mais cette
   distance est celle d'un `job_end`, pas d'un `trim` : pour les six trims de la correction, la
   requête la plus proche est à **41,8 ms**. De plus, la fenêtre de 540 s est plus courte que
   l'attache, si bien que le sixième `job_end` tombe hors charge dans les quatre parcours. Le zéro
   est un **résultat mesuré**, pas une conséquence de requêtes courtes : les requêtes durent
   415,3 à 543,5 ms en p50 selon le parcours (minimum 38,1 ms, 0,63 s en moyenne sur les 1 172
   requêtes réussies), et le rapport cyclique de la charge vaut 34 %. Le recouvrement nul porte sur
   les **20 fins de job de la fenêtre**, référence et correction confondues, cinq par parcours :
   avec ce rapport cyclique, un zéro dû au hasard aurait un ordre de grandeur de 0,66²⁰ ≈ **0,02 %**,
   donc il est **très improbable par hasard**. Ce calcul compare deux horloges : `started_epoch` et
   `ended_epoch` viennent de l'horloge de l'hôte, l'horodatage des lignes `job_end` et `trim` de
   celle du conteneur. **Leur écart n'a pas été contrôlé pendant la campagne** : aucune donnée brute
   ne le mesure (`wall_ms − took_ms` borne la surcharge du client, pas le décalage d'horloges), les
   distances voisines de 5,3 ms et 41,8 ms ne font que plaider pour un alignement approximatif. Cela
   suggère une cause systématique, côté serveur ou côté harnais, que cette campagne **n'identifie
   pas** et sur laquelle elle **ne conclut pas**.
   L'effet d'un trim sur une requête en vol reste donc **non observé**, seulement **borné** par les
   durées mesurées des trims, 5,3 à 29,4 ms. Un protocole qui voudrait garantir le recouvrement
   demanderait une charge calée sur les fins de job, ce que 300 requêtes réparties sur
   neuf minutes ne permettent pas ; c'est une
   proposition, pas une décision.
2. **Les refus `504` diffèrent d'un parcours à l'autre** : 4 et 2 pour la référence, 17 et 5 pour la
   correction. Ils ne sont pas attribués au trim. Dans `ctl-fix-p1`, les 17 refus se répartissent en
   **5 avant `13:14:01`** (`13:13:37` à `13:13:51`), **7 de `13:14:09` à `13:14:47`** et **5 de
   `13:20:54` à `13:21:12`** ; **aucun ne recouvre un trim**. Les deux groupes encadrant les 17
   refus sont donc le **groupe de 5 avant le premier trim** (`13:13:37` à `13:13:51`) et le **groupe
   de 7 après lui** (`13:14:09` à `13:14:47`) ; ensemble ils couvrent 70 s autour du trim de
   `13:14:01`, et le **groupe de 5** de `13:20:54` à `13:21:12` reste à part. Cette attache est la
   plus lente des quatre (636 s contre 546 à 555 s) ; `ctl-fix-p2`, à 5 refus, est proche de la
   référence. Les refus de toute la campagne
   durent **2 005,6 à 2 071,6 ms**, tous sous la borne de 5 465,7 ms : les inclure dans les
   percentiles ne changerait pas le critère d'INV-02. La lecture retenue est celle d'une variance de
   la machine hôte pendant cette campagne, sans preuve. Un contrôle à l'échelle réelle dirait s'il
   en est autrement.
3. **Le seuil de cette campagne est étroit** (10,20 MiB ; les 45,3 MiB de V0 et les 272,96 MiB de la
   troisième campagne ne sont cités que pour situer l'ordre de grandeur, aucun n'est repris comme
   seuil). C'est une propriété de cette campagne : les deux processus de référence
   donnent 1 276,58 et 1 266,36 MiB, à 0,8 % l'un de l'autre. Un seuil étroit rend le gain plus
   facile à dépasser ; il ne change pas la lecture, puisque chacun des deux processus de la
   correction dépasse le seuil à lui seul.
4. **Les niveaux absolus de l'état (c) diffèrent de la troisième campagne** : 2 769,7 et 2 752,7 MiB
   pour la référence ici, contre 2 714,4 et 2 441,7 MiB alors. C'est une **observation** : les
   comparaisons sont internes à la campagne, et aucun chiffre de la troisième campagne n'est
   réutilisé, ni comme valeur, ni comme seuil.
5. **Les arènes ne suivent pas l'anonyme.** À l'état (c), les 63 et 65 arènes de la correction
   portent 1 058,5 et 1 085,7 MiB de `Rss`, contre 1 711,2 et 1 694,6 MiB pour les 60 et 58 arènes de
   la référence : la correction a **plus** d'arènes et **moins** de `Rss` d'arènes, alors que son
   anonyme est plus bas de 630 MiB environ. Le nombre et le `Rss` des arènes ne sont donc pas une
   mesure de la rétention, ce que les deux campagnes précédentes avaient déjà observé.
6. **Deux processus seulement.** Deux parcours ne donnent pas de distribution : la dispersion est un
   écart entre deux valeurs, pas un écart type, comme dans les campagnes précédentes.
7. **Le volume est recréé entre deux processus**, écart déclaré avec les deux campagnes précédentes
   (voir « Protocole ») : l'index n'étant pas supprimé à (c), le volume est détruit pour que l'état
   (a) n'ait aucun index. Les niveaux de (a) en sont comparables d'un parcours à l'autre, et les
   transitions (b)→(c) ne dépendent pas du volume.
8. **Un premier essai du premier parcours a été abandonné.** Le harnais échouait quand une requête
   refusée n'avait pas de `pool_size`, au lieu d'enregistrer le refus ; les 300 lignes de cet essai
   ont été perdues et le parcours rejoué. La vérification a été remplacée par un comptage des refus
   et par un enregistrement incrémental des requêtes (`queries/load-rows.jsonl`) ; les **quatre
   parcours retenus** ont tous tourné avec la version corrigée, et le parcours abandonné n'entre dans
   aucun chiffre de cette section.
9. **`pool_size` vaut 0 sur les premières requêtes** de chaque parcours : l'index existe à l'état (b)
   mais aucun passage n'y est encore visible, et la charge démarre avec la première attache. Le
   maximum vaut 249 998, soit les 257 626 passages moins les 7 628 du dernier lot, encore en cours
   d'attache à la fin de la charge. Ce n'est pas un défaut de mesure : le pool est bien celui de la
   génération visible à cet instant.
10. **Écarts d'exécution déjà consignés** et reconduits : attache des six lots en une seule commande
    (546 à 636 s, sous la borne de 30 minutes), lots dérivés de la première campagne non re-dérivés,
    `smaps` non conservé, taille de page lue par `getconf`, `df -h` brut gardé dans les données brutes
    (gitignorées) et aucun chemin absolu de la machine dans les fichiers versionnés.
11. **Harnais étendu pendant la campagne** : `--campaign control` (dont le défaut de `RUST_LOG`
    d'`up`), `--image-role` d'`init`, `--load-window` et `--load-threads` d'`attach`,
    sous-commandes `load` et `trim-lines`, enregistrement incrémental des requêtes, et
    `job_end_overlap` ajouté à `trim-lines` après les trois premiers parcours — cet ajout ne lit que
    des fichiers déjà écrits et ne change aucun chiffre. Aucune autre modification : ni le code de
    production, ni l'image, ni `crible.toml`.
12. **Deux champs des `run.json` de référence de cette campagne sont faux, et la donnée brute n'est
    pas corrigée.** Ils portent `commit: 3f334c9`, le commit de l'arbre du harnais au moment du
    relevé, et `trim_in_server: true`, déduit à tort de la campagne et non de l'image. Or l'image de
    référence vient de `31e9804` et ne contient pas le module `trim`. Le harnais est corrigé pour
    les passages futurs : `init --campaign control` exige `--image-role reference|correction`,
    `trim_in_server` en découle, `image_role` et `image_commit` sont enregistrés, et le commit de
    l'arbre passe sous `harness_commit`. La présente campagne n'est pas rejouée ; c'est
    `image_id` (`dc800f534ff5`) qui identifie l'image mesurée.

### Recommandations, sans décision

Le propriétaire choisit la suite. Aucune de ces propositions n'est décidée ici, et aucune ne modifie
ADR-0160.

1. **La fusion de la correction** : le gain de 644,39 MiB vaut 63,2 fois le seuil de la campagne, les
   deux processus dépassent le seuil à eux seuls et chaque fin de job a rendu des pages
   (`trimmed = 1`, 6 sur 6). Le critère d'INV-02 est satisfait **sur la charge concurrente de
   l'attache** ; l'effet d'un trim sur une requête en vol n'est **pas observé**, il est seulement
   borné par 5,3 à 29,4 ms (réserve 1). ADR-0160, décision 6, n'arrête la tâche que si le gain manque
   le seuil ou si le critère de latence échoue : ni l'un ni l'autre n'est arrivé. La décision de
   fusionner reste celle du propriétaire, sur ce rapport.
2. **Un suivi du nombre de refus `504` à l'échelle réelle** : 17 refus sur 300 dans un parcours de la
   correction, contre 4 et 2 pour la référence, sans attribution possible dans cette campagne. Une
   mesure sur `decisions` dirait s'il s'agit de la machine ou d'un effet de la correction ; elle
   reste un choix du propriétaire (ADR-0160, décision 7).
3. **Un protocole de charge calé sur les fins de job**, si le recouvrement exact doit être garanti :
   il demanderait de déclencher une fin de job pendant une charge courte et dense, donc un ADR de
   protocole. La présente campagne déclare le recouvrement mesuré (0 sur les 20 fins de job de la
   fenêtre) et s'en tient là.
4. **Les 627,09 MiB qui restent** dans (c)−(b) de la correction ne sont pas attribués : c'est le
   reste déjà renvoyé à la v2 par ADR-0160, décision 7, et il ne demande rien avant la 1.1.0.

### Données brutes

- `data/mes17/raw/2026-11-22/ctl-ref-p1/`, `ctl-ref-p2/`, `ctl-fix-p1/`, `ctl-fix-p2/` — un processus
  chacun, avec `run.json` (campagne, image et identifiant, `RUST_LOG` relu, variante `null`, VM,
  garde, `df_start` ; pour la référence, `commit` et `trim_in_server` sont faux, voir réserve 12),
  `states/*.json` (trois lectures par état), `logs/*.jsonl`,
  `attach/campaign.json`, `create.json`, `queries/load.json` (percentiles, refus, 300 lignes),
  `queries/load-rows.jsonl` (les mêmes lignes, écrites à l'arrivée), `summary.json` (médianes,
  dispersions, transitions, `df_end`), `journal-check.json`, `memory-lines.json` et
  `trim-lines.json` (lignes `trim`, `trimmed`, durées, recouvrement avec la charge).
- `data/mes17/build-t05-trim-fix.log` — la construction de l'image de correction, cache servi.
- Les images `crible:t05-suite-mes17`, `crible:t05-trim-fix` et `crible-mes17-gdb:2026-11-22` sont
  conservées ; le volume `crible-mes17-data` a été supprimé après la campagne, et aucun conteneur
  `crible-mes17*` ne subsiste.
