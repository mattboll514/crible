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

**Côté Python** : `uv`, `pyarrow`, `numpy`, `httpx`, `pytest`, `blake3` (ADR-0034). Les outils d'export ONNX (`optimum`, `onnxruntime`) restent dans `tools/` et ne sont pas des dépendances du paquet.

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

[limits]
writer_queue = 16
default_timeout_ms = 2000
```

Chaque clé peut être surchargée par une variable d'environnement, par exemple `CRIBLE_SERVER__BIND`.

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
- **Sur un tag `v*`** : build de l'image multi-arch avec buildx et publication sur GHCR.
- Les jeux de données lourds ne tournent pas en CI : les mesures se font sur la machine cible et sont consignées dans `docs/mesures/`.
