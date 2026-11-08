# Recette de la mesure en conteneur (P8, ADR-0135)

Cette recette exécute la **passe A** de la tâche `MESURER-EN-CONTENEUR-MAC` : MES-02 (scan binaire),
MES-03 (rescoring fp16, à chaud et à froid) et MES-09 (RAM réelle), dans un conteneur Docker sur la
machine de développement. La passe B (MES-05, MES-08, les deux routes, la recommandation de RAM)
réutilise la même image, le même volume et la même recette : ses sous-commandes et ses écarts sont
décrits plus bas, dans « Passage B : ce qui a été fait ».

Le protocole qui fait foi est **ADR-0135**, complété par ADR-0077 (latence dense), ADR-0080
(accès aux vecteurs `f16`, décision 7), ADR-0084 (quel froid fait foi) et ADR-0081 (parcours
clairsemé unique). Les rapports sont dans `docs/mesures/`, sections « Campagne en conteneur
(P8, ADR-0135) — 2026-11-03 ».

## Ce que la recette crée

| Objet | Nom | Taille observée |
|---|---|---|
| Image moteur | `crible:dev` | 178 Mo |
| Image harnais | `crible-mes-p8-harness:dev` | 194 Mo |
| Volume nommé | `crible-mes-p8-data`, monté sur `/data` | 35,36 Go |
| Copie de mesure | `/data/indexes/synth-large` | 17 Go |
| Copie à 8 segments | `/data/indexes/synth-large-9seg` | 15 Go |
| Modèle | `/data/models/mdenseon` | 1,5 Go |
| Requêtes | `/data/queries.f32` | 900 Ko |
| Documents bruts | `/data/raw`, recopiés dans `data/mes-p8-conteneur/raw/` | 968 Ko |

Aucun autre volume, aucune autre image et aucun conteneur ne sont touchés. Tous les conteneurs de
mesure tournent avec `--rm` : la campagne n'en laisse aucun derrière elle. L'image de base
`debian:bookworm-slim` servait déjà de base à l'image `crible:dev` ; elle n'est pas propre à cette
recette.

Le volume nommé `crible-mes-p8-data` et l'image du harnais `crible-mes-p8-harness:dev` sont **à
retirer par le propriétaire** (`docs/suivi.md`, « Données de mesure conservées ») : le tableau
ci-dessus garde leurs tailles observées, et `prepare.sh` les recrée.

`data/mes-p8-conteneur/` (gitignoré) contient les documents bruts (`raw/`), le contrôle natif au même
harnais (`native/raw/`) et les fichiers d'environnement (`staging/`).

## Prérequis

- Docker Desktop avec une VM Linux `aarch64` ; la campagne a tourné sur une VM de
  **12 529 836 032 octets (11,67 Gio)** de RAM et **8 vCPU**, page de **4 096 octets** ;
- environ **40 Gio libres sur l'hôte** et **35 Go dans la VM** ;
- `data/synth-large/indexes/synth-large` (la copie de mesure de MES-02) et `data/models/mdenseon`
  sur l'hôte ;
- `cargo build --release` n'est pas lancé sur l'hôte : le binaire est compilé dans l'image.

## Étapes

```sh
# 1. Images, volume, copies, requêtes. Idempotent : refuse d'écraser.
sh tools/bench/mes-p8-conteneur/prepare.sh

# 2. Environnement, pour les rapports.
sh tools/bench/mes-p8-conteneur/run.sh env

# 3. Mesures, une sous-commande à la fois, en avant-plan.
sh tools/bench/mes-p8-conteneur/run.sh mes02
sh tools/bench/mes-p8-conteneur/run.sh mes03-warm
sh tools/bench/mes-p8-conteneur/run.sh mes03-cold
sh tools/bench/mes-p8-conteneur/run.sh mes09

# 4. Documents bruts hors du volume.
sh tools/bench/mes-p8-conteneur/run.sh collect

# 5. Artefacts versionnés, percentiles recalculés et vérifiés.
(cd tools && uv run --no-sync python bench/mes-p8-conteneur/aggregate.py --date 2026-11-03)
```

Le harnais est `crates/crible/examples/dense_latency.rs`, compilé dans
`Dockerfile.harness` par greffe sur `crible:dev` (même glibc, même bibliothèque). Il reçoit cinq
ajouts Linux, tous sous `#[cfg(target_os = "linux")]` et sans effet sur macOS :

- `--evict` : `madvise(MADV_PAGEOUT)` sur les pages projetées, puis
  `posix_fadvise(POSIX_FADV_DONTNEED)` sur le descripteur (décision 1 d'ADR-0135) ;
- résidence par `mincore`, publiée par point ;
- `vmmap_summary` lit `/proc/self/status` (`VmRSS`, `RssAnon`, `RssFile`, `RssShmem`) ;
- lecture du cgroup v2 (`memory.current`, `memory.stat`, `memory.max`) ;
- `--engine-limit` : diagnostic qui limite la consommation du rescoring à un résultat tout en lisant
  toutes les lignes, pour séparer la lecture des vecteurs de la lecture des clés.

**Aucun code de production n'est modifié.** Le harnais est un exemple, hors du chemin du serveur.

## Durées observées

| Étape | Durée |
|---|---|
| `prepare.sh`, copies comprises | environ 30 min |
| dont `crible compact` de la copie à 8 segments | 13 min 47 s, silencieux |
| `mes02` | environ 7 min, échauffements compris |
| `mes03-warm` | 27 min, dont 18 min pour la série chaude complète |
| `mes03-cold`, 9 séries | environ 27 min au total, 2 à 7 min par série |
| `mes09` | 6 min |
| `aggregate.py` | moins de 5 s |

Aucune commande ne dépasse 30 min. En revanche, `crible compact` et la série chaude n'impriment rien
pendant plus de 3 min : c'est un écart au protocole de conduite de la tâche, déclaré dans les rapports.

## Écarts et limites, à reprendre tels quels

- **La copie à 8 segments n'est pas mesurée en passe A.** Elle est prête ; le cas « nombre de
  segments réel » de la décision 2 attend une invocation `scan` sur `synth-large-9seg`.
- **Le froid ne vaut pour le moteur que partiellement.** `MADV_PAGEOUT` ne rend que les pages déjà
  fautées ; la résidence du moteur vaut 0,47 à 0,71 et la série s'appelle « froid partiel ». Les
  séries « dot » et « clés » ont une résidence de vecteurs nulle, mais `--evict` ne vise pas
  `records.sqlite` : le froid des clés ne vient que de la purge globale et du conteneur neuf.
- **`drop_caches` est global à la VM.** Il a réussi neuf fois, mais ne sert que de contre-contrôle.
- **Pas de `vmmap` sous Linux** : la mémoire possédée se lit au RSS anonyme, pas à la
  `Physical footprint` de macOS.
- **Pas de contrôle natif du froid au même harnais** : le contrôle natif rejoué porte sur la série
  chaude. Le froid ne se compare qu'à la dernière campagne publiée.
- **La charge de l'hôte n'est pas relevée**, contrairement aux campagnes natives.
- **L'écart entre la série « moteur » du harnais et ses répliques n'est pas expliqué** : 22,8 ms en
  natif pour 1 000 lignes lues seules (`--engine-limit 1`) contre 1,1 ms pour le produit scalaire
  seul. Aucun chiffre « moteur » ne doit servir de budget tant que cet écart n'est pas instruit.

## Passage A-bis : diagnostic de l'écart du moteur (2026-11-03)

L'écart déclaré ci-dessus a été instruit, en **natif seulement**, avec `ab-engine.sh`
(`head`, `replay`, `limit`, `profile`, `aggregate`). Verdict : **artefact du harnais**, pas une
régression.

- `dense_latency` passe `limit = candidates.len()` à `DenseSearcher::rescore` ; la production passe
  `k = candidates_per_branch` (100 ici). La différence, 1 000 à 5 000 lectures de clés aléatoires
  dans `records.sqlite` que le chemin livré ne fait pas, explique tout l'écart entre les deux
  harnais : ramené à `--engine-limit 100`, `dense_latency` revient au niveau de `rescore_ab`
  (10 192 µs contre 12 613 µs, soit −19,2 %, à N = 1000 ; 75 172 contre 76 698, soit −2,0 %, à
  N = 5000).
- Au commit du rejeu (`07a82d773db3245e6fc680b48baabe61fbd4ac21`, extrait par `git archive`, jamais
  copié hors de l'arbre), la série « moteur » est **plus lente** qu'à la tête : 45 184 µs contre
  41 590 à N = 1000, 726 902 contre 398 763 à N = 5000.
- Le profil (`sample`, 7 634 échantillons du fil principal) porte 86,6 % du temps du moteur sur
  `key_of` → `read_passage_key` → `sqlite3BtreeTableMoveto` → `pread`, et 0,2 % sur
  `kernels::neon::dot`.
- Les 0,79 / 2,17 ms du rejeu publié supposent les pages des candidats résidentes ; la résidence
  mesurée aujourd'hui sur ces pages est de 0,456 en moyenne (0,282 à 0,838), avec 355 Mio de pages
  libres. La première passe du rejeu publié coûtait d'ailleurs 15,07 ms, du même ordre que les
  12,6 ms d'aujourd'hui.

Sorties brutes : `data/mes-p8-conteneur/ab/raw/` (gitignoré). Artefact versionné :
`docs/mesures/MES-03-diagnostic-moteur.2026-11-03.json`. Les dossiers de construction
(`data/mes-p8-conteneur/ab/07a82d7`, `target-07a82d7`) et le cache de candidats sont supprimés après
la mesure ; la recette les reconstruit.

## Ce que la passe B réutilise

- les images `crible:dev` et `crible-mes-p8-harness:dev`, reconstruites depuis le même commit ;
- le volume `crible-mes-p8-data` et ses copies, sans nouvelle copie de 17 Go ;
- `queries.f32` et `mdenseon`, déjà dans le volume ;
- `run.sh env` et `run.sh collect`, tels quels ;
- `aggregate.py` : les blocs MES-05 et MES-08 s'ajoutent à `PUBLISHED` et aux écrivains, sans toucher
  aux trois artefacts existants ;
- la règle de comparaison de la décision 4 (dernière campagne publiée, seuil de 10 % au-delà de sa
  dispersion) et le seuil de 5 ms au p95 de la décision 6 pour les deux routes.

## Passage B : ce qui a été fait (2026-11-03)

La passe B est exécutée. Ce que la recette porte de plus :

- `Dockerfile.harness` construit et greffe **quatre** exemples, pas un : `dense_latency`
  (crible), `measure_queries` et `acceptance_sweep` (crible-encode), `server_costs` (crible-store).
  Un crate sous `tools/` ou `data/` ne peut pas entrer dans le contexte de construction
  (`.dockerignore`), d'où le portage du balayage de MES-08 dans `examples/acceptance_sweep.rs` ;
- sous-commandes ajoutées à `run.sh` : `mes03-limit` (chemin livré, `--engine-limit 100`, quatre
  passes à chaud dont une écartée, une passe froide par conteneur neuf), `mes05` (deux exécutions par
  précision), `mes08` (deux invocations entrelacées de deux tours, cadrage B = 1 avant et après,
  test de linéarité), `routes` (serveur détaché, client HTTP, deux harnais d'isolation, arrêt
  explicite du serveur) ;
- `routes_client.py` : client HTTP nommé (Python 3.13, `http.client`, une connexion persistante,
  `perf_counter_ns`), 300 appels par route et par passe, trois passes après une chauffe écartée ;
- `aggregate-b.py` : quatre artefacts de la passe B, percentiles recalculés depuis les durées par
  appel au **rang le plus proche** (ADR-0077), et refus d'une série dont le nombre d'appels, le
  minimum, le maximum ou la moyenne s'écarte de ceux du client ou du harnais ; les percentiles
  interpolés des harnais restent visibles dans chaque bloc, sous `harness_summary` ;
- la copie à 8 segments `synth-large-9seg` est mesurée (routes) ; `data/staging/` du volume porte
  `query-texts.jsonl`, `sample-daily-articles-1000.json`, `docs.json` et `crible.toml` ; les deux
  entrées sont déposées par `prepare.sh`, étape 6 bis, pour que la recette se rejoue sans copie
  manuelle ;
- `data/mes-p8-conteneur/staging/crible.toml` (gitignoré) porte la configuration du serveur de la
  campagne, avec une clé de rôle `read` : le jeton vit dans
  `data/mes-p8-conteneur/staging/token.txt`, jamais dans un fichier suivi.

Durées observées en passe B, pour dimensionner une reprise : 2 min 13 s à 13 min 15 s par point de
`mes03-limit` à chaud, 2 min 30 s à 3 min 30 s à froid, 50 s par exécution d'encodage, 15 min par
invocation de balayage, 1 min 40 s pour la linéarité, 2 min pour l'ouverture du serveur (deux index
ouverts), 1 min pour la campagne des routes.

## Nettoyage

Rien n'est supprimé automatiquement. Pour libérer la place après validation :

```sh
docker volume rm crible-mes-p8-data
docker image rm crible-mes-p8-harness:dev
rm -rf data/mes-p8-conteneur
```

`crible:dev` n'est pas propre à cette recette et n'est pas à supprimer sans vérifier les autres
campagnes qui l'utilisent.
