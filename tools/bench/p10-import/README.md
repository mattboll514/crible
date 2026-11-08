# Harnais de la campagne d'import — `IMPORTER-BASE-COMPLETE` (P10)

Harnais des trois passes de la tâche `IMPORTER-BASE-COMPLETE` — import, mesures, instantanés —
conforme au protocole d'ADR-0139 (décision 5). Il ne modifie ni le code de production, ni un ADR, ni un
document de conception, et ne supprime rien.

Les arborescences `data/p10-import/` et `data/p10-exchange/` qu'il produit ont été mises à la
corbeille le 2026-11-08, à la validation de J10 : une reprise les recrée, `prepare.sh` pour
l'échange et les scripts pour les sorties brutes.

| Fichier | Rôle |
|---|---|
| `prepare.sh` | contrôle d'espace, image `crible:p10`, volumes `crible-p10-data` et `crible-p10-models`, arborescence `data/p10-exchange/` (à recréer, voir l'introduction), clé d'API, conteneur `crible-p10-server` (port 18090) |
| `start.sh` | démarre `crible-p10-server-a2` (ou le crée avec les six montages), échantillonne `memory.current`, `anon`, `file` et le RSS toutes les 5 s pendant l'ouverture des index, imprime le délai jusqu'à `/ready` |
| `import.py` | client HTTP : `create` (six index), `attach` (un lot, `?wait=true` puis suivi du job, `--sample-container` pour la mémoire), `wait` (reprise du suivi), `state` et `collect` (`GET /v1/indexes`) |
| `attach-all.sh` | attache les lots dans l'ordre croissant, relève `df` sur l'hôte et dans la VM avant et après chaque attache, s'arrête sous 40 Go libres sur l'hôte |
| `summarize.py` | relit les sorties brutes et imprime les tableaux du rapport (durées, comptes, espace) |
| `run-measured.py` | lance une commande CLI hors ligne (`compact`, `rebuild-lexical`) dans un conteneur nommé, échantillonne la mémoire et `df`, écrit un JSON de résultat |
| `mes09.py` | MES-09 : les trois états d'ADR-0077 sur le serveur, et le déplacement des dossiers d'index vers `/data/staging-mes09/` |
| `mes09-per-index.sh` | une série MES-09 par index : arrêt, déplacement des cinq autres, démarrage, mesure, remise en place (même en cas d'échec) |
| `snapshot.py` | les six instantanés hors ligne : durée, `df` de l'hôte, contrôle de `snapshot.json` (fichiers listés, fichiers présents, tailles) ; `--summarize-only` réécrit `summary.json` depuis les JSON par index |
| `blake3-check.py` | recalcule le blake3 de chaque fichier d'un instantané et le compare à `snapshot.json` ; se lance depuis `tools/` par `uv run --no-sync` |

## Ordre d'exécution

```sh
sh tools/bench/p10-import/prepare.sh
sh tools/bench/p10-import/start.sh                            # reprise : mêmes volumes
python3 tools/bench/p10-import/import.py create
sh tools/bench/p10-import/attach-all.sh dole:dole-full-v1      # un lot par commande
sh tools/bench/p10-import/attach-all.sh sarde:sarde-full-v1    # (moins de 30 min chacune)
python3 tools/bench/p10-import/import.py attach \
    --index decisions --lot decisions-full-v1 \
    --sample-container crible-p10-server-a2                   # mémoire toutes les 5 s
python3 tools/bench/p10-import/import.py collect
python3 tools/bench/p10-import/summarize.py

# passe B : mesures, serveur arrêté pour la CLI
python3 tools/bench/p10-import/run-measured.py --name p10-mes07-decisions \
    --key mes07/decisions -- compact --data /data --index decisions
python3 tools/bench/p10-import/run-measured.py --name p10-rebuild2-decisions \
    --key rebuild2/decisions --interval 1 -- rebuild-lexical --data /data --index decisions
sh tools/bench/p10-import/start.sh
python3 tools/bench/p10-import/mes09.py series --label instance --target decisions
sh tools/bench/p10-import/mes09-per-index.sh decisions

# passe C : instantanés, serveur arrêté (ADR-0064, décision 3)
python3 tools/bench/p10-import/snapshot.py dole sarde titres jorf articles decisions
# restauration de contrôle du plus petit, dans un dossier jetable (`data/p10-exchange/` à recréer si absente)
docker run --rm -v /tmp/p10-restore-check:/restore \
    -v "$PWD/data/p10-exchange/snapshots/dole-e1-g1":/snapshot:ro \
    crible:p10 restore --data /restore --snapshot /snapshot
```

L'ordre complet est `prepare`, `start`, `import`, puis `mes07`, `mes09`, `rebuild`, `snapshot` :
chaque étape de mesure arrête le serveur avant la CLI et le redémarre après, sauf `snapshot`, qui
laisse le serveur arrêté.

`crible inspect` se lance **serveur arrêté** : sur un volume ouvert en écriture, il attend le verrou
et bloque. `start.sh` crée le conteneur sous le nom `crible-p10-server-a2` pour laisser intact
`crible-p10-server`, le conteneur de la première séance ; `CONTAINER=…` change le nom. Les deux
occupent le port 18090 : quand `crible-p10-server-a2` doit être créé et que `crible-p10-server`
tourne encore, `start.sh` l'**arrête** d'abord (`docker stop`, jamais `docker rm`), ce qui rend
l'ordre `prepare.sh` puis `start.sh` valable depuis un arbre neuf. `attach-all.sh` vise
`crible-p10-server` par défaut, parce que c'est le conteneur de `prepare.sh` ; après une reprise par
`start.sh`, passer `CONTAINER=crible-p10-server-a2`.

`attach-all.sh` accepte un ou plusieurs couples `index:lot` en argument ; sans
argument il attache les six lots, ce qui dépasse la limite de 30 minutes par
commande de la campagne. Chaque lot est monté **directement** sous
`/exchange/inbox/<lot>` en lecture seule par `prepare.sh` (ADR-0107 : `directory`
est un seul composant de chemin).

## Sorties brutes

Toutes sous `data/p10-import/`, gitignoré et à recréer par le harnais (mise à la corbeille le 2026-11-08) :

| Chemin | Contenu |
|---|---|
| `build.log`, `build-time.txt` | construction de l'image et sa durée |
| `df-host-prepare.txt`, `df-host-start.txt`, `df-host-per-lot.txt` | `df -h /` et `df -H /` sur l'hôte, datés |
| `df-vm-start.txt`, `df-vm-per-lot.txt`, `du-indexes-*.txt` | `df` et `du` dans la VM |
| `key-generate.txt`, `token.txt` | clé d'API (`token.txt` en mode 600) |
| `index-create/` | réponse de `POST /v1/indexes` par index |
| `jobs/*-submit.json`, `jobs/*-final.json` | réponse de `POST /batches` et état final du job |
| `polls/*.jsonl` | chaque `GET /v1/jobs/{id}`, horodaté côté client |
| `index-state/*.json`, `index-list.json` | réponses de `GET /v1/indexes` |
| `inspect-*.txt` | sortie de `crible inspect`, serveur arrêté (ADR-0116) |
| `server.log` | journal du serveur, horodaté par Docker (`docker logs -t`) |
| `oom-articles.txt`, `memory-after-restart.txt`, `docker-stats-after-restart.txt` | constat de l'OOM et mémoire de la VM |
| `memory/<index>.jsonl` | échantillons de mémoire de l'attache (`--sample-container`), un point par 5 s |
| `memory-open-<conteneur>-<horodatage>.jsonl`, `open-<conteneur>-<horodatage>.txt` | mémoire et délai jusqu'à `/ready` à chaque démarrage |
| `lexical-meta.txt` | documents vivants et opstamp des index lexicaux, lus dans `meta.json` |
| `searches-after-recovery.txt` | recherches de contrôle après la reprise lexicale |
| `mes07/<index>.json`, `mes07/<index>.samples.jsonl` | compaction mesurée : durée, `df`, mémoire, sortie de la CLI |
| `mes09/<label>-*.json` | série MES-09 : trois états, mémoire brute et `memory.stat` |
| `rebuild2/<index>.json` | reconstruction lexicale mesurée, échantillonnage à 1 s |
| `snapshots/<index>.json`, `snapshots/summary.json` | instantané : durée, taille, `df`, verdict de complétude |
| `snapshots/restore-check.txt` | restauration de contrôle de `dole` et recherche lexicale |

`summarize.py` ne lit que ces fichiers ; il n'extrapole rien et imprime `—`
pour une figure absente. Une durée marquée `*` vient des bornes d'étape des
sondes de job (précision de 10 s), faute de ligne `attach durations` : le job
a été interrompu avant de l'écrire.
