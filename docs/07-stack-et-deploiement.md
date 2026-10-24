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
    crible/                  # binaire `crible` (CLI) : serve, index, attach, search, compact, export, snapshot, restore, inspect, eval, stats, key
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
| HTTP | `axum` 0.8, `tokio`, `tower`, `tower-http` | Comme NextPlaid |
| OpenAPI | `utoipa`, `utoipa-swagger-ui` | Spécification générée depuis le code |
| Instantanés sans verrou | `arc-swap` | |
| Calcul parallèle | `rayon` | Pool dédié, séparé de tokio |
| Encodage | `ort` (2.x, `load-dynamic`), `tokenizers` | ONNX Runtime chargé dynamiquement, comme NextPlaid |
| Vecteurs | `memmap2`, `half` | Fichiers plats little-endian |
| Noyaux SIMD | à trancher (crate dédiée ou intrinsèques NEON/AVX2 avec repli scalaire) | Micro-benchmark dans `NOYAUX-SIMD` |
| Bitmaps | `roaring` | |
| SQLite | `rusqlite` (`bundled`) | Mode WAL |
| Compression du texte | `zstd` | |
| Lexical | `tantivy` 0.27 | Analyse française intégrée |
| Lots | `arrow`, `parquet` (arrow-rs) | |
| Hash | `blake3` | Empreintes, checksums, `passage_hash` |
| Erreurs | `thiserror` (bibliothèques), `anyhow` (binaire) | |
| Logs | `tracing`, `tracing-subscriber` (JSON) | |
| Config | `serde`, `serde_json`, `toml` | Plus les surcharges par variables d'environnement `CRIBLE_*` (ADR-0027) |
| CLI | `clap` | |
| Tests | `proptest`, `criterion`, `tempfile` | |

**Côté Python** : `uv`, `pyarrow`, `numpy`, `httpx`, `pytest`, `blake3` (ADR-0034). Les outils d'export ONNX restent dans `tools/` et ne sont pas des dépendances du paquet : ils forment le groupe `uv` séparé `export` de `tools/pyproject.toml` — `onnx`, `onnxruntime`, `torch`, `transformers`, `tokenizers`, `huggingface-hub`, plus `onnxscript` si l'exportateur l'exige (ADR-0050). `optimum` n'en fait plus partie : aucune version publiée ne permet `transformers` 5 (ADR-0050). Ce groupe n'est pas installé par un `uv sync` par défaut, donc la CI ne l'installe pas ; il s'installe avec `uv sync --group export`. `torch` vient de l'index CPU `pytorch-cpu`, ce qui évite la pile CUDA sous Linux (plusieurs Go). Les bornes de version font foi dans `tools/pyproject.toml`.

Un second groupe, `ci`, sert au seul job `rust-onnx` (ADR-0057). Il contient ONNX Runtime, en version exacte (`onnxruntime==1.31.0`), et **pas** `torch` : c'est le runtime d'exécution dont les tests Rust ont besoin, pas la chaîne d'export. Il peut aussi recevoir `tokenizers` si c'est la seule dépendance qui manque aux tests de `tools/bench/mes04/`. Il s'installe avec `uv sync --group ci`, et `ORT_DYLIB_PATH` désigne la bibliothèque de la roue (`onnxruntime/capi/libonnxruntime.so.1.31.0`), jamais un chemin de poste de développement. La feature `download-binaries` d'`ort` n'est pas utilisée : elle ferait entrer un binaire non contrôlé dans le build, hors du mode `load-dynamic` retenu ci-dessus. La version d'`onnxruntime` du groupe `ci` et celle du champ `export` de `model.json` doivent rester alignées.

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
sha256 = "…"                  # généré par `crible key generate`

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
writer_queue = 16
default_timeout_ms = 2000
query_cache_entries = 1024    # cache LRU des requêtes, par instance, jamais persisté (ADR-0051)
```

Chaque clé peut être surchargée par une variable d'environnement, par exemple `CRIBLE_SERVER__BIND`.

`crible-encode` ne règle ni `intra_op_num_threads` ni le niveau d'optimisation de la session ONNX Runtime : les **défauts de la bibliothèque** sont conservés, MES-05 ayant montré qu'ils sont les plus rapides dans les deux précisions (ADR-0052). Un réglage n'aurait de sens que pour brider la consommation, et se déciderait en conteneur, en P8.

## Docker

**Une seule image serveur, CPU, multi-architecture** (`linux/arm64` pour le Mac mini, `linux/amd64`). La construction suit le modèle de NextPlaid :
- étapes `cargo-chef` (cache des dépendances) puis build en release ;
- image d'exécution `debian:bookworm-slim` avec la bibliothèque ONNX Runtime CPU de l'architecture visée ;
- `ORT_DYLIB_PATH` positionné ;
- utilisateur non-root ;
- port 8080 ;
- `HEALTHCHECK` sur `/health` ;
- arrêt propre sur SIGTERM : on arrête d'accepter, on termine les jobs d'écriture en cours, on vide les analytics, puis on sort.

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

- **Sur chaque push et chaque PR** : `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, puis `uv run pytest` pour `clients/python`.
- **Sur chaque push et chaque PR, job `rust-onnx`** (ADR-0057) : un job séparé, sur `ubuntu-latest`, qui prouve le chemin ONNX. Il a un `timeout-minutes` explicite, resserré après le premier run à froid. Dans l'ordre : toolchain Rust et `uv` ; génération de `synth-small` comme le job Rust ; `uv sync --group ci` et `ORT_DYLIB_PATH` vers la bibliothèque de la roue ; restauration du cache, sinon téléchargement des fichiers depuis le dépôt de modèle Hugging Face, épinglé sur un sha de commit et vérifié par blake3 contre un manifeste versionné, puis dépôt dans `data/models/mdenseon/` ; enfin la commande ci-dessous. Le job Rust actuel reste inchangé : `cargo test --locked`, sans ONNX Runtime.

  ```
  cargo test --locked --workspace -- --include-ignored --skip inv_12_the_legifacile_sample_passes_the_acceptance_check
  ```

  Le cache GitHub Actions est indexé sur les hash du manifeste et n'est jamais une source de confiance : le contenu restauré est vérifié comme un téléchargement neuf. Un échec de téléchargement, un hash faux, un fichier absent ou une révision introuvable fait échouer le job, avec l'URL, le fichier et les hash attendu et obtenu ; aucun repli silencieux (`continue-on-error`, `|| true`) n'est admis. Le test de l'échantillon Legifacile est le seul exclu, par son nom, parce que ses données dérivent de la production (ADR-0032).
- **Sur un tag `v*`** : build de l'image multi-arch avec buildx et publication sur GHCR.
- Les **jeux de données lourds** ne tournent pas en CI : ils se génèrent sur la machine cible, et les mesures se font là, puis sont consignées dans `docs/mesures/`. Un **artefact de modèle déjà exporté** n'est pas un jeu de données : le job `rust-onnx` le télécharge et le met en cache, sans jamais ré-exporter le modèle (ADR-0049, ADR-0050, ADR-0057).
