# 07 — Stack technique et déploiement

## Organisation du dépôt

```
crible/
  Cargo.toml                 # workspace Rust
  rust-toolchain.toml        # toolchain stable épinglée
  crates/
    crible-core/             # types partagés : ids, config d'index, schéma, filtres (AST), erreurs et codes, manifeste et format de lot
    crible-store/            # SQLite, segments (fp16, codes), générations, bitmaps et colonnes, lecture des lots, attache, compaction, export, instantanés, écriture tantivy
    crible-encode/           # ONNX Runtime (ort) + tokenizers : chargement des modèles, pooling, prompts, cache LRU des requêtes
    crible-search/           # pool, stratégies dense (exact, binaire + rescoring), noyaux SIMD, BM25 limité au pool, fusion, regroupement, multi-index, explain
    crible-server/           # axum : routes, auth, limites, erreurs, écrivain et jobs, analytics, journal de debug, OpenAPI
    crible/                  # binaire `crible` (CLI) : serve, index, attach, ingest, patch, search, compact, rebuild-lexical, export, snapshot, restore, inspect, eval, stats, key
  clients/python/            # paquet `crible` : BatchWriter (pyarrow) + client HTTP (uv)
  tools/                     # scripts Python : export ONNX, jeux synthétiques, référence numpy
  docker/                    # Dockerfile, compose d'exemple, crible.example.toml
  docs/                      # conception, plan, suivi, jalons, mesures
  data/                      # (gitignoré) données générées, fixtures volumineuses
```

**Arêtes internes entre crates.** `crible-store` déclare une dépendance vers `crible-encode`, pour ses **types** seulement (compteur de tokens, encodeur) : le contrôle 8 (longueurs, INV-11), le contrôle 9 (recette fp32, INV-12) et l'encodage en ligne de documents s'exécutent ainsi dans `crible-store::attach` et `crible-store::online`, avec le modèle chargé et fourni par l'appelant (ADR-0053). L'arête est **unidirectionnelle** : `crible-encode` ne dépend jamais de `crible-store`. Toute autre arête interne qui engage la conception demande son ADR.

## Dépendances Rust

Les versions sont à épingler dans `INITIALISER-WORKSPACE-RUST`, en prenant la dernière version stable compatible au moment de la tâche.

| Rôle | Crate | Remarque |
|---|---|---|
| HTTP | `axum` 0.8, `tokio`, `tower`, `tower-http` | Comme NextPlaid. Features : `tokio` (`rt-multi-thread`, `macros`, `signal`, `net`, `time`, `sync`) et `tower-http` (`trace`, `limit` pour la taille de corps, `catch-panic` pour une panique rattrapée : ADR-0102) ; `tokio` (`io-util`) et `tower` (`util`) sont des dépendances de développement seulement (complément du 2026-10-29 à ADR-0096) ; la feature `load-shed` de `tower` n'est pas activée et l'échéance de `timeout_ms` passe par `tokio::time::timeout`, pas par la couche `timeout` de `tower-http` (ADR-0102) ; les threads tokio restent aux défauts de la bibliothèque (ADR-0096) |
| OpenAPI | `utoipa`, `utoipa-swagger-ui` | Spécification générée depuis le code |
| Instantanés sans verrou | `arc-swap` | |
| Calcul parallèle | `rayon` | Pool dédié, séparé de tokio |
| Encodage | `ort` (2.x, `load-dynamic`), `tokenizers` | ONNX Runtime chargé dynamiquement, comme NextPlaid |
| Vecteurs | `memmap2`, `half` | Fichiers plats little-endian |
| Noyaux SIMD | `std::arch` en code maison (`crible-search::kernels`) : NEON sur `aarch64`, AVX2 sur `x86_64`, repli scalaire, détection à l'exécution (ADR-0070) | Micro-benchmark `criterion` dans `benches/kernels.rs`, chiffres consignés dans ADR-0070 |
| Bitmaps | `roaring` | |
| SQLite | `rusqlite` (`bundled`) | Mode WAL |
| Compression du texte | `zstd` | |
| Lexical | `tantivy` 0.27 | Analyse française intégrée |
| Lots | `arrow`, `parquet` (arrow-rs) | |
| Hash | `blake3` | Empreintes, checksums, `passage_hash` |
| Authentification | `sha2`, `subtle`, `getrandom` | SHA-256 du jeton reçu, comparaison en temps constant (`ConstantTimeEq` sur 32 octets), aléa de `crible key generate` ; l'hexadécimal est écrit et lu à la main, sans crate d'encodage (ADR-0100) |
| Erreurs | `thiserror` (bibliothèques), `anyhow` (binaire) | |
| Logs | `tracing`, `tracing-subscriber` (JSON) | Feature `env-filter` : `RUST_LOG` est honoré, avec `info` par défaut quand il est absent (ADR-0096). Quel que soit `RUST_LOG`, aucun log ne contient de texte de requête ni de valeur de filtre (INV-13) |
| Identifiant de requête | `uuid` (feature `v4`) | Génère `x-request-id` quand l'en-tête entrant est absent ou hors bornes (ADR-0098) |
| Config | `serde`, `serde_json`, `toml` | Plus les surcharges par variables d'environnement `CRIBLE_*` (ADR-0097) |
| CLI | `clap` | |
| Tests | `proptest`, `criterion`, `tempfile` | |

**Côté Python** : `uv`, `pyarrow`, `numpy`, `httpx`, `pytest`, `blake3` (ADR-0034). Les outils d'export ONNX restent dans `tools/` et ne sont pas des dépendances du paquet : ils forment le groupe `uv` séparé `export` de `tools/pyproject.toml` — `onnx`, `onnxruntime`, `torch`, `transformers`, `tokenizers`, `huggingface-hub`, plus `onnxscript` si l'exportateur l'exige (ADR-0050). `optimum` n'en fait plus partie : aucune version publiée ne permet `transformers` 5 (ADR-0050). Ce groupe n'est pas installé par un `uv sync` par défaut, donc la CI ne l'installe pas ; il s'installe avec `uv sync --group export`. `torch` vient de l'index CPU `pytorch-cpu`, ce qui évite la pile CUDA sous Linux (plusieurs Go). Les bornes de version font foi dans `tools/pyproject.toml`.

Un second groupe, `ci`, sert aux jobs `rust-onnx` et `tools` (ADR-0057). Il contient ONNX Runtime, en version exacte (`onnxruntime==1.31.0`), et **pas** `torch` : c'est le runtime d'exécution dont les tests Rust ont besoin, pas la chaîne d'export. Il peut aussi recevoir `tokenizers` si c'est la seule dépendance qui manque aux tests de `tools/bench/mes04/`. Il s'installe avec `uv sync --group ci`, et `ORT_DYLIB_PATH` désigne la bibliothèque de la roue (`onnxruntime/capi/libonnxruntime.so.1.31.0`), jamais un chemin de poste de développement. La feature `download-binaries` d'`ort` n'est pas utilisée : elle ferait entrer un binaire non contrôlé dans le build, hors du mode `load-dynamic` retenu ci-dessus. La version d'`onnxruntime` du groupe `ci` et celle du champ `export` de `model.json` doivent rester alignées.

## Environnement de développement (Mac)

- **Rust en natif** pour le développement et les tests : `cargo build`, `cargo test`. Docker ne sert qu'à l'image de déploiement.
- **ONNX Runtime pour `ort` en local** : la bibliothèque dynamique, installée par Homebrew ou téléchargée, est indiquée par `ORT_DYLIB_PATH`. La procédure est à documenter dans `CHARGER-MODELE`.
- **Python** : `uv sync` dans `clients/python/` et `tools/`.
- **Données volumineuses** (jeux synthétiques, échantillons Legifacile, modèles) : dans `data/`, gitignoré.

## Configuration de l'instance : `crible.toml`

```toml
[server]
bind = "0.0.0.0:8080"
search_threads = 0            # 0 = nombre de cœurs
max_concurrent_searches = 0   # 0 = 2 × cœurs

[paths]
data = "/data"
models = "/models"
exchange = "/exchange"

[[auth.keys]]
name = "legifacile-mcp"
role = "read"
sha256 = "…"                  # 64 caractères hexadécimaux minuscules, généré par `crible key generate` (ADR-0100)

[[auth.keys]]
name = "legifacile-sync"
role = "write"
sha256 = "…"

[analytics]
retention_days = 90

[debug_journal]
enabled = false
sample_rate = 0.0
ttl_days = 7

[encoder]
query_precision = "fp32"      # "fp32" | "int8" : précision des requêtes (ADR-0052)

[limits]
writer_queue = 16             # jobs en attente par index, le job en cours exclu ; au-delà, 503 overloaded (ADR-0103)
default_timeout_ms = 2000
wait_timeout_ms = 60000       # attente au plus de ?wait=true, puis 202 avec le job en cours (ADR-0067) ; borne aussi l'attente des jobs à l'arrêt (ADR-0104)
query_cache_entries = 1024    # cache LRU des requêtes, par instance, jamais persisté (ADR-0051)
```

**Surcharges par l'environnement (ADR-0097).** Chaque clé de `crible.toml` peut être surchargée par une variable d'environnement : `CRIBLE_` suivi du chemin de la clé en majuscules, les sections séparées par `__`. Exemples : `CRIBLE_SERVER__BIND`, `CRIBLE_LIMITS__QUERY_CACHE_ENTRIES`. La valeur est analysée comme un scalaire du type de la clé visée. Le serveur refuse de démarrer, avec un message qui nomme la variable et un code de sortie non nul, si la clé est inconnue, si la valeur ne s'analyse pas dans ce type, ou si la variable vise un tableau de tables (`[[auth.keys]]` ne se surcharge pas par l'environnement en v1). Même raisonnement qu'ADR-0031 : une faute de frappe ne doit pas passer sans effet.

Les variables `CRIBLE_*` documentées **hors configuration** sont exclues de ce contrôle, et la liste est fermée : `CRIBLE_FAILPOINT` (ADR-0068), `CRIBLE_BIN`, `CRIBLE_PYTHON`, `CRIBLE_EXPORT_REPRO`, `CRIBLE_CRASH_CHILD`, `CRIBLE_CRASH_BATCH` et `CRIBLE_CRASH_INDEX` (outils et tests). Toute autre variable `CRIBLE_*` inconnue fait échouer le démarrage.

**Commande.** `crible serve [--config <chemin>]`, chemin par défaut `/etc/crible/crible.toml`. Un fichier absent ou illisible fait échouer le démarrage, avec un code de sortie non nul : le serveur ne démarre jamais sur des valeurs par défaut implicites. Les chemins `data`, `models` et `exchange` viennent de la section `[paths]` ; aucune option de ligne de commande ne les remplace.

`crible-encode` ne règle ni `intra_op_num_threads` ni le niveau d'optimisation de la session ONNX Runtime : les **défauts de la bibliothèque** sont conservés, MES-05 ayant montré qu'ils sont les plus rapides dans les deux précisions (ADR-0052). Un réglage n'aurait de sens que pour brider la consommation, et se déciderait en conteneur, en P8.

## Docker

**Une seule image serveur, CPU, multi-architecture** (`linux/arm64` pour le Mac mini, `linux/amd64`). La construction suit le modèle de NextPlaid :
- étapes `cargo-chef` (cache des dépendances) puis build en release ;
- image d'exécution `debian:bookworm-slim` avec la bibliothèque ONNX Runtime CPU de l'architecture visée ;
- `ORT_DYLIB_PATH` positionné ;
- utilisateur non-root ;
- port 8080 ;
- `HEALTHCHECK` sur `/health` ;
- arrêt propre sur SIGTERM et SIGINT (ADR-0099) : `/ready` passe à `503`, le serveur arrête d'accepter, il draine les requêtes en vol pendant au plus `[limits].default_timeout_ms` + 1 000 ms, il vide les analytics quand elles existent (P7), puis il sort en journalisant les requêtes coupées ; l'attente des jobs d'écriture est bornée par `[limits].wait_timeout_ms` (60 000 ms par défaut), comptée depuis le signal et en parallèle du drainage des requêtes : toute nouvelle soumission de job rend `503 overloaded` (« the server is shutting down », `Retry-After: 1`), les jobs acceptés avant le signal se terminent, et à l'échéance le processus sort en journalisant les jobs abandonnés, qui passent à `failed` au démarrage suivant (ADR-0104). Un second signal pendant le drainage fait sortir immédiatement avec un code non nul. Conséquence : le délai d'arrêt du conteneur (`stop_grace_period`, réglé en P8) doit être **au-dessus** de `wait_timeout_ms`, faute de quoi l'orchestrateur tue le processus avant la fin de l'attente.

Pas d'image CUDA : les producteurs sont en Python (ADR-0013).

**Volumes** :

| Montage | Type | Pourquoi |
|---|---|---|
| `/data` | volume nommé | Le mmap à travers le partage de fichiers macOS → VM est lent |
| `/models` | volume nommé | Modèles ONNX et tokenizers, vérifiés par hash au démarrage |
| `/exchange` | dossier monté depuis l'hôte | Lots (`inbox`), exports (`outbox`), instantanés : lectures et écritures séquentielles |
| `/etc/crible/crible.toml` | fichier monté, en lecture seule | Configuration |

**Compose d'exemple** (Legifacile) : le service `crible` et le MCP de Legifacile partagent un réseau interne. Crible n'est **jamais exposé publiquement** : seul le MCP lui parle.

**Contraintes propres au Mac** :
- la RAM disponible est celle de la VM Docker (~8 Go aujourd'hui), pas les 16 Go de la machine ;
- pas d'accès au GPU Apple ;
- le SIMD NEON est disponible.

## CI (GitHub Actions)

- **Sur chaque push et chaque PR** : `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, puis `cargo clippy -p crible-store --all-targets --locked --features failpoints -- -D warnings` et `cargo test -p crible-store --locked --features failpoints` (ADR-0068 : les tests de panne et de concurrence de `crible-store` — INV-02, INV-03, INV-04, INV-14 et les tests de panne d'`attacher_lot` — n'existent qu'avec la feature `failpoints`, et la CI ne les construisait jamais), puis `cargo clippy -p crible-server --all-targets --locked --features failpoints -- -D warnings` et `cargo test -p crible-server --locked --features failpoints` (complément du 2026-10-29 à ADR-0068 : le serveur prouve la reprise d'ADR-0108 par le point de panne `after_epoch_switch` de la compaction, avec sa propre feature `failpoints`, éteinte par défaut), puis `uv run pytest` pour `clients/python`.
- **Sur chaque push et chaque PR, job `rust-onnx`** (ADR-0057) : un job séparé, sur `ubuntu-latest`, qui prouve le chemin ONNX. Il a un `timeout-minutes` explicite, resserré après le premier run à froid. Dans l'ordre : toolchain Rust et `uv` ; génération de `synth-small` comme le job Rust ; `uv sync --group ci` et `ORT_DYLIB_PATH` vers la bibliothèque de la roue ; restauration du cache, sinon téléchargement des fichiers depuis le dépôt de modèle Hugging Face, épinglé sur un sha de commit et vérifié par blake3 contre un manifeste versionné, puis dépôt dans `data/models/mdenseon/` ; enfin la commande ci-dessous. Le job Rust reste sans ONNX Runtime : `cargo test --locked`, puis les quatre étapes `failpoints` d'ADR-0068 — deux pour `crible-store`, deux pour `crible-server` (complément du 2026-10-29) —, rien d'autre.

  ```
  cargo test --locked --workspace -- --include-ignored --skip inv_12_the_legifacile_sample_passes_the_acceptance_check --skip inv_09_legifacile_sample_queries_return_the_same_results_after_compaction --skip inv_08_legifacile_sample_queries_return_the_same_results_after_export_and_import --skip inv_12_the_reference_batch_acceptance_statistics_are_unchanged
  ```

  Le cache GitHub Actions est indexé sur les hash du manifeste et n'est jamais une source de confiance : le contenu restauré est vérifié comme un téléchargement neuf. Un téléchargement fait au plus **quatre essais** par fichier, avec des attentes de 5, 15 et 45 s, un `Retry-After` respecté jusqu'à 60 s et refusé au-delà, et une **échéance globale de 600 s** pour l'artefact, bien sous le `timeout-minutes` du job ; seules les causes extérieures au contenu sont reprises (`URLError`, `TimeoutError`, autres `OSError` de lecture, `IncompleteRead`, HTTP 429 et 5xx), tandis qu'un hash faux, un autre 4xx, un manifeste invalide ou un cache divergent échouent du premier coup (ADR-0059). Un échec de téléchargement, un hash faux, un fichier absent ou une révision introuvable fait échouer le job, avec l'URL, le fichier et les hash attendu et obtenu ; aucun repli silencieux (`continue-on-error`, `|| true`) n'est admis. Les tests de l'échantillon Legifacile sont les **quatre exclus**, nommés un par un : `inv_12_the_legifacile_sample_passes_the_acceptance_check`, `inv_09_legifacile_sample_queries_return_the_same_results_after_compaction`, `inv_08_legifacile_sample_queries_return_the_same_results_after_export_and_import` et `inv_12_the_reference_batch_acceptance_statistics_are_unchanged`. Leurs données dérivent de la production (ADR-0032), et les quatre passent par la recette, qui demande en plus ONNX Runtime et le vrai modèle ; ils restent locaux (ADR-0057, ADR-0062, ADR-0063, ADR-0085).
- **`.github/workflows/model-cache.yml`, seul écrivain du cache** (ADR-0059) : un workflow séparé, un seul job, **aucun test lancé**. Il tourne sur `schedule` le lundi et le jeudi à 03:17 UTC (`17 3 * * 1,4`), sur `push` filtré par `paths` quand `ci/model-artifact.toml` ou le fichier du workflow change, et sur `workflow_dispatch`. Il calcule la même clé que `rust-onnx`, restaure `data/models/mdenseon`, vérifie les blake3 et ne télécharge que ce qui manque, puis enregistre le cache seulement sur un défaut de cache. Le job `rust-onnx` ne fait plus qu'en **restaurer** : un seul écrivain évite la course sur une clé identique et une entrée de cache par branche. Deux passages par semaine gardent le cache de `main` sous l'éviction de sept jours.
- **Sur un tag `v*`** : build de l'image multi-arch avec buildx et publication sur GHCR.
- Les **jeux de données lourds** ne tournent pas en CI : ils se génèrent sur la machine cible, et les mesures se font là, puis sont consignées dans `docs/mesures/`. Un **artefact de modèle déjà exporté** n'est pas un jeu de données : le job `rust-onnx` le télécharge et le met en cache, sans jamais ré-exporter le modèle (ADR-0049, ADR-0050, ADR-0057).
