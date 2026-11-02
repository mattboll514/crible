# Mesure — latence avant/après du cache des lecteurs denses à 8,58 M passages

Rapport attaché à la tâche `REUTILISER-LECTEURS-DENSES` (phase P7, livrable 2). Ce n'est pas une mesure numérotée `MES-xx` de `docs/08-invariants-et-tests.md` : elle répond au critère de la fiche (« si le cache est fait : la latence avant/après est mesurée ») et à la décision 8 d'ADR-0126.

## Campagne du 2026-11-01

- Tâche : `REUTILISER-LECTEURS-DENSES`
- Commit : `9ae150d457fb683d9cd4b5878f285e9e0b5b6873` (branche `tache/REUTILISER-LECTEURS-DENSES`). « Avant » est l'arbre de ce commit, extrait par `git archive` dans `data/mes-lecteurs-denses/avant/src` et compilé à part. « Après » est l'arbre de travail, dont le cache n'est pas commité (`crates/crible-store/src/readers.rs` ajouté, `crates/crible-server/src/{jobs,search,state}.rs` et `crates/crible-search/src/dense/exact.rs` modifiés, entre autres).
- Machine : MacBook Air M3, modèle `Mac15,12`, CPU `Apple M3`, 25 769 803 776 octets de RAM (24 Gio), macOS 15.7.7 (24G720), **natif** (pas de Docker), `cargo build --release`.
- Charge tierce : moyennes `2,63 / 4,61 / 4,76` à 16 h 33 (début de la série « pleine pool »), `3,35 / 2,68 / 2,33` à 18 h 22 (début de la série filtrée) et `2,99 / 2,99 / 2,87` à 18 h 50 (fin). Aucune autre campagne n'a tourné pendant les mesures.
- Mémoire : `vm_stat` en cours de campagne donne 22 Gio utilisés sur 24, dont 5,9 Gio de compresseur, et environ 1 Gio libre ; le navigateur Arc en occupe 6,9 Gio. Le fichier de vecteurs de 13,2 Go **ne tient donc pas dans le cache de pages**. C'est une réserve de premier ordre, chiffrée plus bas : chaque relecture intégrale repart du SSD.
- Espace disque : 66 GiB libres avant la campagne, 64 GiB après. Le dossier de construction « avant » pèse 1,1 Gio et n'a pas été supprimé (seuil de 5 Gio non atteint).

### Note du 2026-11-01 (relecture, tour 2)

L'arbre « après » mesuré est celui qui a été commité ensuite sous `ebe0d6d`
(« feat(store): cache dense segment readers per segment and payload pool per
epoch »), sur la base de `9ae150d`. Il est donc antérieur aux corrections du
tour 2 de la relecture de cette tâche, qui ne touchent pas le chemin d'une
requête.

Deux écarts au protocole d'ADR-0126, décision 8, sont acceptés par
l'orchestrateur, et la ligne `REUTILISER-LECTEURS-DENSES` du tableau « Tâches »
de `docs/suivi.md` les nomme : la série « avant » mesure 100 requêtes au lieu de
300 (8 à 13 s par requête), et le cache de pages n'est pas atteint (13,2 Go de
vecteurs pour 24 Gio de mémoire). Le rejeu à 300 requêtes sur un cache de pages
chaud est renvoyé au conteneur, en P8.

### Jeu de données

Point unique, celui sur lequel porte la décision : index **`synth-large`**, `data/synth-large/indexes/synth-large`.

| Élément | Valeur |
|---|---|
| Segment | 1 (`seg-large-0001`, `iid_start` 0, `batch_id` `large-0001`) |
| Passages déclarés | 8 600 000 |
| Passages vivants | 7 740 000 |
| Dimensions, précision | 768, fp16 |
| Taille de `vectors.f16` | 13 209 600 000 octets |
| Somme de contrôle déclarée | `blake3:9087fe59d7036cde0bf26fa9f8498deaca699efb0549ff64f17ddcd8d160711a` |
| Codes binaires | `codes.bin`, 825 600 000 octets, présent avant la campagne (mtime du 2026-11-01 14 h 16, inchangé ensuite) |
| Génération servie | 2, époque 1 |

### Configuration

- Mode `dense`, `k = 10`, `debug: true`, `timeout_ms = 30000` sur chaque requête (la valeur par défaut de 2 000 ms rendrait un `504` avant le code, où l'ouverture coûte plus de 8 s).
- `rescore_candidates = 1000`, `candidates_per_branch = 100`, `exact_threshold = 1000`, lus de `index.json` ; `search.scan = "none"` (voir « Stratégie dense »).
- Modèle `synthetic-external` : les requêtes portent un **vecteur** fp32 base64 de 768 dimensions, jamais un texte. Aucun encodeur, donc aucune variable `ORT_DYLIB_PATH`.
- Serveur : `search_threads = 0`, `max_concurrent_searches = 0`, `query_cache_entries = 0`, journal de debug désactivé, une seule clé `read`. Un client séquentiel, une connexion, aucune requête concurrente.
- 400 vecteurs de requête déterministes (graine `20261101`), normalisés, écrits par le harnais.

### Protocole

Harnais : `tools/bench/ouvertures-denses/latence.py`, ajouté pour cette campagne. Il démarre le serveur, attend `/ready`, envoie les requêtes une par une, lit `took_ms`, `debug.timings_ms`, `debug.plan` et `pool_size`, puis arrête le serveur par `SIGTERM` et vérifie son code de sortie. Aucun code de `crates/*/src/` n'est modifié.

Construction des deux binaires (depuis la racine du dépôt ; `git archive` ne contient que des fichiers suivis) :

```
mkdir -p data/mes-lecteurs-denses/avant/src
git archive 9ae150d | tar -x -C data/mes-lecteurs-denses/avant/src
cd data/mes-lecteurs-denses/avant/src
CARGO_TARGET_DIR=/Users/mattboll/MattCorp/crible/data/mes-lecteurs-denses/avant/target \
  cargo build --release --offline -p crible --bins --example dense_latency
cd ../../../..
cargo build --release --offline -p crible --bins --example dense_latency
```

Requêtes et séries :

```
python3 tools/bench/ouvertures-denses/latence.py queries \
  --out data/mes-lecteurs-denses/latence/queries.jsonl --count 400 --dims 768 --seed 20261101

# Série A, pool entier (secondaire) :
python3 tools/bench/ouvertures-denses/latence.py campaign \
  --binary data/mes-lecteurs-denses/avant/target/release/crible \
  --config data/mes-lecteurs-denses/latence/crible.toml --port 18080 \
  --queries data/mes-lecteurs-denses/latence/queries.jsonl \
  --out data/mes-lecteurs-denses/latence/latence-avant.json --requests 100 --warmup 3
python3 tools/bench/ouvertures-denses/latence.py campaign \
  --binary ./target/release/crible \
  --config data/mes-lecteurs-denses/latence/crible.toml --port 18080 \
  --queries data/mes-lecteurs-denses/latence/queries.jsonl \
  --out data/mes-lecteurs-denses/latence/latence-apres-1sur3.json --requests 100 --warmup 5

# Série B, pool filtré (preuve principale) : {"categorie":{"eq":"cat-00"}}, pool 430 000
python3 tools/bench/ouvertures-denses/latence.py campaign \
  --binary data/mes-lecteurs-denses/avant/target/release/crible \
  --config data/mes-lecteurs-denses/latence/crible.toml --port 18080 \
  --queries data/mes-lecteurs-denses/latence/queries.jsonl \
  --out data/mes-lecteurs-denses/latence/filtre-avant-1sur3.json \
  --requests 100 --warmup 10 --offset 0 --filter '{"categorie":{"eq":"cat-00"}}'
# puis trois passes « après » (offsets 0, 100, 200, 100 requêtes mesurées chacune),
# fusionnées par `latence.py merge` dans filtre-apres.json (n = 300).
```

- **Deux séries, et pourquoi.** Au pool entier, la stratégie est **exacte** et le scan exact lit 11,9 Go de vecteurs par requête. Son coût change d'un facteur deux pendant la campagne avec l'état du cache de pages (5 445 ms en p50 sur la série « avant », 12 290 ms sur la série « après »), ce qui noierait le gain du cache dans `total`. La série filtrée ramène le pool à 430 000 passages (environ 660 Mo lus par scan) et isole la dépense qui change : l'ouverture des lecteurs. Les deux séries gardent la même index, le même mode, le même `k`, le même N et les mêmes vecteurs ; la série filtrée est la preuve principale, la série au pool entier le contrôle secondaire.
- **Préchauffage** : 3 à 10 requêtes par passe, non comptées, pour absorber la première lecture des fichiers. **Aucun froid contrôlé** : pas de `F_NOCACHE`, pas de copie non cachée ; ADR-0084 renvoie le froid à P8.
- **Cache chaud, au sens de la machine** : le fichier de vecteurs ne reste pas résident (réserve mémoire ci-dessus). Chaque ouverture relit donc les 13,2 Go depuis le SSD, et c'est cette lecture qui est mesurée. Les deux versions la paient dans les mêmes conditions, la version « avant » à chaque requête.
- **Percentiles** : rang le plus proche (ADR-0117, décision 4), la même fonction que MES-02. Avec `n = 100`, le p99 est le maximum de la série : c'est le rang le plus proche, pas un lissage.
- **`n`** : 300 requêtes mesurées pour « après » (3 passes de 100, offsets 0, 100 et 200). Pour « avant », 100 requêtes mesurées : à 12,10 s de `total` en p50, 300 requêtes demanderaient environ 60,5 minutes, au-delà du seuil de 60 minutes que la consigne autorise, et la consigne admet 100 dans ce cas.
- **Aucune requête perdue** : 0 erreur HTTP dans toutes les séries, aucun `504`.
- **Contrôle par le harnais** : `dense_latency open --data data/synth-large --index synth-large --opens 2`, sur les deux binaires. Le harnais jette le searcher entre deux ouvertures : il paie donc l'ouverture entière à chaque appel, dans les deux versions, ce qui reproduit le chemin d'une requête « avant ».

### Résultats

**Série B, pool filtré (430 000 passages), preuve principale** — millisecondes, `n = 300` pour « après », 100 pour « avant » :

| Série | `total` p50 | p95 | p99 | `fetch` p50 | p95 | p99 |
|---|---|---|---|---|---|---|
| Avant (`9ae150d`) | 12 095,658 | 12 947,697 | 13 322,368 | 10 845,140 | 11 052,328 | 11 175,421 |
| Après (arbre de travail) | **67,884** | **74,598** | **77,834** | **3,333** | **4,816** | **5,383** |
| Écart p50 | −12 027,774 ms (−99,44 %) | | | −10 841,807 ms (−99,97 %) | | |

`dense` en p50 : 1 247,223 ms avant, 64,216 ms après. `filter` en p50 : 0,274 ms contre 0,267 ms. `encode`, `fusion` et `group` restent sous 0,01 ms dans les deux séries. Minimum et maximum de `total` : 10 939,619 et 14 577,136 ms avant ; 63,265 et 89,158 ms après.

**Série A, pool entier (7 740 000 passages), contrôle secondaire** — millisecondes, `n = 100` de chaque côté :

| Série | `total` p50 | p95 | p99 | `fetch` p50 | p95 | p99 | `dense` p50 |
|---|---|---|---|---|---|---|---|
| Avant (`9ae150d`) | 13 740,399 | 15 233,135 | 15 283,751 | 8 274,044 | 8 350,879 | 8 375,282 | 5 445,378 |
| Après (arbre de travail) | 12 296,079 | 12 647,001 | 12 926,619 | **5,902** | **8,249** | **9,112** | 12 290,079 |

`fetch` perd 8 268,142 ms en p50 (−99,93 %), la dépense que la décision 7 d'ADR-0126 rend à sa définition. `total` ne perd que 1 444,320 ms, parce que le scan exact a doublé entre les deux séries : c'est l'état du cache de pages, pas un effet du cache des lecteurs. `total` au pool entier n'est donc **pas** une preuve de gain ; `fetch` l'est.

**Démarrage du serveur jusqu'à `/ready`** — 3 mesures de chaque côté, alternées (`avant`, `après`, `avant`, …) pour que la dérive de la machine les frappe également :

| Série | Mesures (ms) | p50 |
|---|---|---|
| Avant | 26 056,0 · 25 681,2 · 25 322,6 | 25 681,2 |
| Après | 31 601,9 · 31 503,6 · 31 038,4 | **31 503,6** |

Le démarrage augmente de **5 822,4 ms en p50** (+22,7 %), et reste à 31,5 s, sous le budget de 60 s d'ADR-0047 (MES-13). Les journaux du serveur séparent les deux phases : « index opened for writing » à environ 22 s (avant) et 20 s (après), puis « models and indexes loaded » 3,4 s plus tard côté « avant » et 10,7 s plus tard côté « après ». La différence de 7,3 s de cette seconde phase est le remplissage des lecteurs, c'est-à-dire la vérification blake3 des 13,2 Go une fois par processus.

**Critère des 10 % de REPONSE-DEBUG**, chemin critique (`encode + filter + max(dense, lexical) + fusion + group + fetch`, `validate` exclu) contre `timings_ms.total`, écart relatif maximal observé sur la série « après » :

| Série | `n` | Écart p50 | Écart maximal |
|---|---|---|---|
| Après, pool filtré | 300 | 0,089 % | **0,130 %** |
| Après, pool entier | 100 | 0,002 % | 0,014 % |
| Avant, pool filtré | 100 | 0,086 % | 0,130 % |
| Avant, pool entier | 100 | 0,109 % | 0,147 % |

Le critère de 10 % est vert avec une marge de deux ordres de grandeur, dans les deux versions : la sortie de l'ouverture de `fetch` ne le dégrade pas, elle ne fait que supprimer une dépense qui était correctement mesurée.

**Contrôle par le harnais** — `DenseSearcher::open`, 2 ouvertures mesurées après la première :

| Binaire | Première (ms) | Série (ms) | p50 (ms) |
|---|---|---|---|
| Avant (`9ae150d`) | 8 213,547 | 8 225,445 · 8 247,505 | 8 225,445 |
| Après (arbre de travail) | 10 140,226 | 9 750,185 · 8 821,386 | 8 821,386 |

Le harnais paie la même ouverture dans les deux versions, parce qu'il jette le searcher : c'est le comportement d'une requête « avant », et cela confirme que le coût d'une ouverture par processus est inchangé par le cache.

### Stratégie dense

`debug.plan.dense_strategy` vaut **`exact`** dans toutes les séries, avec `rescore_candidates: 1000`, `pool_size` 7 740 000 (pool entier) ou 430 000 (filtre), génération 2.

L'index **a** ses codes binaires (`codes.bin`, 825 600 000 octets, présent avant la campagne), mais son `index.json` porte `"search": { "scan": "none", ... }`. `plan` (`crates/crible-search/src/dense/planner.rs:118`) rend `Strategy::Exact` dès que `scan` vaut `None`, quel que soit le pool et quels que soient les codes. Ce n'est donc ni un manque de codes ni une reconstruction : le serveur ne reconstruit rien, `codes.bin` n'a pas été réécrit (mtime inchangé), et la branche binaire n'est tout simplement pas demandée par la configuration de cet index.

Conséquence pour cette mesure : le gain du cache est chiffré sous la stratégie exacte. Le chemin binaire, qui aurait un scan de quelques dizaines de millisecondes et rendrait le gain d'ouverture encore plus visible dans `total`, n'est pas exercé ici. Le passer demanderait de changer `search.scan` de l'index, ce qui est une décision et une modification de données hors du périmètre de cette session.

### Lecture

Le cache des lecteurs denses supprime la dépense qu'ADR-0126 lui confie. Sur le même index, le même mode, le même `k`, le même N et les mêmes vecteurs, `fetch` passe de **10 845,140 ms** à **3,333 ms** en p50 sur le pool filtré, et de **8 274,044 ms** à **5,902 ms** sur le pool entier. Le `total` suit sur le pool filtré : **12 096 ms → 67,9 ms** en p50, soit un facteur 178. Le gain n'est pas un déplacement de clé : `filter`, `encode`, `fusion` et `group` ne bougent pas, et le critère des 10 % reste vert.

Le coût est celui qu'ADR-0126, décision 4, annonce : le démarrage et la fin de compaction paient l'ouverture de tous les segments. Il est mesuré à **+5,8 s en p50** (25,7 s → 31,5 s), sous le budget de 60 s d'ADR-0047. Une attache ne paierait que son segment neuf ; ce cas n'est pas mesuré ici.

Aucune recommandation n'est nécessaire pour la tâche : les chiffres confirment la décision. Deux points restent des propositions à valider par le propriétaire, et non des conclusions de cette mesure : mesurer aussi la stratégie `binary` (elle demande de changer `search.scan` de l'index, donc un ADR ou une décision explicite), et rejouer le démarrage sur une machine où le fichier de vecteurs tient dans le cache de pages, pour situer le +5,8 s par rapport aux 8,3 s de la mesure du livrable 1.

### Réserves

- **Cache chaud non atteint.** La machine a 24 Gio de RAM, dont 22 utilisés en cours de campagne (compresseur 5,9 Gio, Arc 6,9 Gio). Le fichier de 13,2 Go n'y tient pas : chaque ouverture le relit depuis le SSD. Les valeurs absolues d'ouverture sont donc plus hautes que celles du livrable 1 (8 263 ms en p50, cache chaud) et ne sont pas stables : 8 274 ms en p50 sur la série au pool entier, à 16 h 33, puis 10 845 ms sur la série filtrée, à 18 h 23. La comparaison avant/après reste valable, parce que les deux versions lisent les mêmes octets dans les mêmes conditions ; les valeurs absolues ne valent pas pour une machine au repos avec 13,2 Go résidents.
- **Série filtrée = preuve principale.** Le pool de 430 000 passages n'est pas la latence de service au pool entier. Sur le pool entier, `total` est dominé par le scan exact et ne montre qu'une partie du gain (−1 444 ms) ; seul `fetch` y est propre (−8 268 ms).
- **Stratégie exacte seulement.** `scan = "none"` dans la configuration de l'index : la branche binaire n'est pas mesurée, alors que les codes existent (voir « Stratégie dense »).
- **Un seul segment, un seul mode, une seule génération (2).** Ni multi-segments, ni `lexical`, ni `hybride`, ni attache, ni compaction, ni concurrence : une requête à la fois, `max_concurrent_searches = 0`. La contention du pool de charges utiles qu'ADR-0126 signale comme coût n'est pas mesurée.
- **`n = 100` pour « avant ».** Le p99 y est le maximum de la série (rang 100) ; les p50 et p95 sont stables, mais le p99 avant n'a pas la même résolution que celui de 300 requêtes après.
- **Pas de froid contrôlé.** ADR-0084 renvoie le froid à P8, en conteneur ; aucun `F_NOCACHE` n'est utilisé.
- **Machine de développement, native**, avec une charge tierce de 2 à 5 et un navigateur qui occupe 6,9 Gio. Aucune extrapolation au conteneur de P8 n'est faite.
- **Pas de comparaison à une cible de service** : la fiche ne fixe pas de budget de latence de requête pour cette tâche ; la seule cible chiffrée est le démarrage de 60 s d'ADR-0047, et il est tenu.

### Sorties brutes

Sous `data/mes-lecteurs-denses/latence/` :

- `queries.jsonl` (400 vecteurs, graine 20261101) et `crible.toml` (configuration du serveur, sans secret versionné) ;
- `latence-avant.json`, `latence-apres-1sur3.json` : série au pool entier, `n = 100` chacun ;
- `filtre-avant-1sur3.json`, `filtre-apres-1sur3.json`, `filtre-apres-2sur3.json`, `filtre-apres-3sur3.json`, `filtre-apres.json` (fusion, `n = 300`) : série au pool filtré ;
- `startup-avant-{1,2,3}.json`, `startup-apres-{1,2,3}.json` : démarrages, avec leurs journaux `.log` ;
- `harnais-open-avant.json`, `harnais-open-apres.json` : contrôle `dense_latency open` ;
- `uptime-campagne-avant.txt`, `uptime-campagne-apres.txt`, `machine.txt` ;
- chaque campagne a son journal serveur `*.server.log` et chaque démarrage son journal `*.log`.

Le dossier comporte aussi la clé de mesure (`key.txt`, non versionné, sous `data/`) ; elle n'est citée par aucune valeur dans ce rapport.
