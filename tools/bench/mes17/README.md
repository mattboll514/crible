# MES-17, seconde campagne : attribuer la mémoire anonyme retenue

Harnais de la seconde campagne de MES-17, décidée par
[ADR-0157](../../../docs/adr/0157-protocole-de-la-seconde-campagne-de-mes-17-attribuer-la-memoire-anonyme-retenue.md)
(accepté le 2026-11-21), dont la règle de lecture est complétée par
[ADR-0158](../../../docs/adr/0158-sens-de-l-effet-dans-la-regle-de-lecture-de-la-seconde-campagne-de-mes-17.md)
(accepté le 2026-11-22) : un effet est une **baisse** de l'anonyme retenu d'au
moins la dispersion entre les deux processus de V0. Il rejoue sept états sur un
conteneur jetable et un volume jetable, et attribue la mémoire anonyme retenue par
des variantes de réglage de l'allocateur de glibc. **Il ne corrige rien** : aucune
variable n'entre dans l'image ni dans `crible.toml`.

La campagne du 2026-11-22 compte **cinq parcours**, et non sept : V0 deux fois,
puis V1, V2 et V3. Aucune variante n'a fait baisser l'anonyme retenu d'au moins
45,3 MiB (la dispersion de V0), donc **V4 n'a pas tourné** (ADR-0158, décision 2).
Le protocole prévu reste décrit ci-dessous, V4 compris.

La première campagne et sa méthode sont dans
[`docs/mesures/MES-17-classes-de-memoire.md`](../../../docs/mesures/MES-17-classes-de-memoire.md).

## Prérequis

- Docker Desktop, VM `aarch64` de 10 420 109 312 octets et 8 vCPU (ADR-0135,
  décision 5 : aucune limite `--memory`, la RAM de la VM fait la borne) ;
- l'image `crible:t05-suite-mes17`, construite par l'orchestrateur depuis le
  commit de la branche de tâche — **ne pas la reconstruire** ;
- l'image `postgres:17` présente localement, utilisée comme sonde jetable
  (`--pull=never`), l'image mesurée n'ayant pas de shell ;
- l'échantillon `data/legifacile-sample` (six lots, 300 requêtes) et le modèle
  `data/models/mdenseon` ;
- `data/mes17/crible.toml` (configuration de l'instance jetable) et
  `data/mes17/raw/key.txt` (clé d'administration, lue à l'exécution) ;
- les lots dérivés dans `data/mes17/exchange/inbox/`, produits par
  `prepare_lots.py` (voir plus bas) ;
- Python 3 pour `measure.py`, **bibliothèque standard seulement**.

## Commandes

Toutes les commandes se lancent depuis la racine du dépôt.

```sh
# volume jetable, une seule fois par campagne
docker volume create crible-mes17-data
docker run --rm -u 0 -v crible-mes17-data:/data --pull=never postgres:17 \
  sh -c 'chown -R 1000:1000 /data'

# un parcours = un conteneur neuf. La variante est posée ICI et nulle part ailleurs.
python3 tools/bench/mes17/measure.py up --variant V0
RUN=data/mes17/raw/2026-11-22/V0-p1
python3 tools/bench/mes17/measure.py init --run "$RUN" --variant V0

# preuve que glibc a pris le réglage : chargeur dynamique seul, aucun serveur
# V0 : une référence pour la campagne
python3 tools/bench/mes17/measure.py tunables --out data/mes17/raw/2026-11-22 \
  --variant V0 --name tunables-baseline
# V1 à V3 : une sortie dans le dossier du parcours
python3 tools/bench/mes17/measure.py tunables --out data/mes17/raw/2026-11-22/V1 \
  --variant V1
# V2 seulement : preuve à valeurs non nulles, la valeur 0 étant indistinguable
# d'un tunable non posé (voir « Variantes »)
python3 tools/bench/mes17/measure.py tunables --out data/mes17/raw/2026-11-22/V2 \
  --variant V2 --tunables glibc.malloc.trim_threshold=4096:glibc.malloc.top_pad=12345 \
  --name tunables-proof

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

Chaque état est lu trois fois à trois secondes d'intervalle ; le résumé donne la
médiane et la dispersion (maximum moins minimum). Une lecture ratée n'est jamais
remplacée par un zéro : la sonde réessaie trois fois et échoue franchement.

## Variantes (ADR-0157, décision 4)

| Variante | `GLIBC_TUNABLES` |
|---|---|
| V0 | aucun (contrôle) |
| V1 | `glibc.malloc.arena_max=1` |
| V2 | `glibc.malloc.trim_threshold=0:glibc.malloc.top_pad=0` |
| V3 | `glibc.malloc.mmap_threshold=131072` |
| V4 | combinaison des variantes qui ont eu un effet, passée par `--tunables` |

`up` écrit la valeur exacte dans l'environnement du conteneur, et `init` la relit
par `docker inspect` pour la porter dans `run.json`. Un parcours par variante ;
V0 et V4 en comptent deux chacun quand V4 tourne. En 2026-11-22, V4 n'a pas
tourné : aucune variante n'a eu d'effet au sens d'ADR-0158, et la campagne s'est
arrêtée à cinq parcours.

`measure.py tunables --out <dossier> --variant Vn` lance le chargeur dynamique de
l'image (`/lib/ld-linux-aarch64.so.1 --list-tunables`) avec la variable de la
variante, sans démarrer le serveur, et écrit `<dossier>/tunables.txt` et
`tunables.json` (variante, valeur exacte, image, identifiant d'image, commande).
`--tunables` remplace la valeur de la variante : c'est ainsi que V2 reçoit sa
preuve à valeurs non nulles (`--name tunables-proof`), la valeur `0` s'affichant
comme un tunable non posé.

## Ce que lit la sonde

`/proc/1/status` (`VmRSS`, `RssAnon`, `RssFile`, `RssShmem`, `VmHWM`, `VmSwap`),
`/proc/1/smaps_rollup`, `/proc/1/smaps` complet, `memory.current`,
`memory.swap.current` et `memory.stat` du cgroup, `/proc/meminfo` de la VM
(`MemAvailable`, `SwapFree`) et les lignes `memory` du journal.

`/proc/1/smaps` est volumineux : il est lu en entier et analysé, et **seul
l'agrégat est écrit** (ADR-0157, « Conséquences »).

**Règle de comptage des arènes de glibc**, telle qu'implémentée dans
`parse_smaps` : un groupe de régions anonymes consécutives (sans chemin) dont les
`Size` s'additionnent à exactement **65 536 kB** (64 Mio) compte pour **une**
arène, et son `Rss` est la somme des `Rss` du groupe. C'est la signature d'une
arène de glibc, souvent vue comme une région `rw-p` suivie d'une région `---p`.
Un groupe dont les tailles ne font pas 65 536 kB n'est pas une arène ; il est
rapporté à part. Le résumé donne le nombre d'arènes et leur `Rss` total.

## Garde mémoire

`measure.py state` lit `MemAvailable` et `SwapFree` de la VM dans la même sonde et
s'arrête si `MemAvailable` passe sous 1 Gio ou si `SwapFree` baisse de plus de
256 Mio depuis la ligne de base écrite par `init` dans `run.json`. La VM a 4 Gio
de swap : une mémoire anonyme passée en swap cacherait la rétention mesurée.

## Lots dérivés

`prepare_lots.py` recopie les six lots de l'échantillon sous
`data/mes17/exchange/inbox/mes17-<lot>/`, avec une colonne nulle pour chaque
champ que la structure du lot ne porte pas, le texte, les ids, l'`ord` et les
octets des vecteurs fp16 copiés tels quels, les comptes recomptés et les sommes
`blake3` des manifestes recalculées. Il demande `pyarrow` et `blake3`, outils de
développement de la première campagne, comme
`tools/bench/mes02/make_multiseg_batch.py` ; le mesurage, lui, reste en
bibliothèque standard. Le script n'est pas exécuté par `measure.py`.

## Données brutes

Sous `data/mes17/raw/<date>/<parcours>/` (gitignoré) : `run.json`, `states/`,
`logs/`, `queries/`, `attach/`, `delete.json`, `restarts.json`, `summary.json`,
`journal-check.json`, `memory-lines.json`, `tunables.txt` et `tunables.json`
(plus `tunables-proof.txt` pour V2). Le contrôle du journal vérifie les
quatre `step` d'ADR-0151, un `job_end` par lot attaché (six) et deux
`session_load` (états b et e).
