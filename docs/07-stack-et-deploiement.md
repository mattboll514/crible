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
    crible-server/           # axum : routes, auth, limites, erreurs, écrivain et jobs, analytics, journal de debug, OpenAPI, actifs du tableau de bord (`src/ui/`, ADR-0130)
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
| Client HTTP de `crible eval` | `hyper` (features `client`, `http1`), `hyper-util` (features `client-legacy`, `http1`), `http-body-util` | Module privé de la crate `crible` seulement, runtime tokio `current_thread` créé par `eval` ; HTTP/1.1 en clair, sans TLS, sans `reqwest` ni `ureq` ; les trois crates sont déjà dans `Cargo.lock` parce qu'`axum` les tire, l'activation de `client-legacy` y ajoute `want`, `try-lock` et `futures-channel` (ADR-0131) |
| OpenAPI | `utoipa` (`=6.0.0`, feature `axum_extras`), `utoipa-swagger-ui` (`=10.0.1`, feature `vendored` avec l'intégration `axum`) | Spécification générée depuis le code, en OpenAPI 3.1.0, servie par `/openapi.json` ; `docs/05` reste la référence de la liste des routes (ADR-0015, ADR-0111). `vendored` embarque Swagger UI : le build ne télécharge rien (ADR-0111) |
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
| CLI | `clap` | `crible stats` rend ses tableaux ASCII à la main, sans crate de tableau (ADR-0124, décision 14) |
| Tests | `proptest`, `criterion`, `tempfile`, `jsonschema` (`=0.60.2`, `default-features = false`) | `jsonschema` est une dépendance de développement de `crible-server` seulement, absente du binaire : elle valide les exemples de `docs/05` contre la spécification générée, sans résolution réseau ni fichier de `$ref` distant (ADR-0111) |

**Le tableau de bord n'ajoute aucune dépendance (ADR-0130, décision 3).** Ses actifs — HTML, CSS, JS et un favicon SVG éventuel — vivent dans `crates/crible-server/src/ui/`, sont embarqués par `include_bytes!` et `include_str!`, et servis sous `/ui` par un handler axum écrit à la main. Rien n'est ajouté au tableau ci-dessus : ni la feature `fs` de `tower-http`, ni `rust-embed`, ni `include_dir`, ni une bibliothèque de graphiques. Le type MIME, l'`ETag` et les en-têtes de sécurité sont écrits à la main. `cargo build` reste la seule étape de production, et le build ne télécharge rien, ne lit aucun dossier à l'exécution et ne dépend pas du répertoire courant.

**Côté Python** : `uv`, `pyarrow`, `numpy`, `httpx`, `pytest`, `blake3` (ADR-0034). Les bornes du paquet client font foi dans `clients/python/pyproject.toml` : `httpx` y est borné par `>=0.27,<1` (ADR-0112). Les outils d'export ONNX restent dans `tools/` et ne sont pas des dépendances du paquet : ils forment le groupe `uv` séparé `export` de `tools/pyproject.toml` — `onnx`, `onnxruntime`, `torch`, `transformers`, `tokenizers`, `huggingface-hub`, plus `onnxscript` si l'exportateur l'exige (ADR-0050). `optimum` n'en fait plus partie : aucune version publiée ne permet `transformers` 5 (ADR-0050). Ce groupe n'est pas installé par un `uv sync` par défaut, donc la CI ne l'installe pas ; il s'installe avec `uv sync --group export`. `torch` vient de l'index CPU `pytorch-cpu`, ce qui évite la pile CUDA sous Linux (plusieurs Go). Les bornes de version font foi dans `tools/pyproject.toml`.

Un second groupe, `ci`, sert aux jobs `rust-onnx` et `tools` (ADR-0057). Il contient ONNX Runtime, en version exacte (`onnxruntime==1.31.0`), et **pas** `torch` : c'est le runtime d'exécution dont les tests Rust ont besoin, pas la chaîne d'export. Il peut aussi recevoir `tokenizers` si c'est la seule dépendance qui manque aux tests de `tools/bench/mes04/`. Il s'installe avec `uv sync --group ci`, et `ORT_DYLIB_PATH` désigne la bibliothèque de la roue (`onnxruntime/capi/libonnxruntime.so.1.31.0`), jamais un chemin de poste de développement. La feature `download-binaries` d'`ort` n'est pas utilisée : elle ferait entrer un binaire non contrôlé dans le build, hors du mode `load-dynamic` retenu ci-dessus. La version d'`onnxruntime` du groupe `ci` et celle du champ `export` de `model.json` doivent rester alignées.

**Télémétrie d'ONNX Runtime (ADR-0128).** La bibliothèque ONNX Runtime 1.31.0 embarque le client de télémétrie Microsoft 1DS, actif par défaut : il crée un identifiant d'appareil et envoie des données d'usage vers un service tiers. Crible le coupe partout où il charge ORT, sans opt-in. Trois mécanismes, complémentaires : le binaire `crible` force `ORT_DISABLE_TELEMETRY=1` au tout début de `main`, avant `Cli::parse()` et avant tout fil ; l'`EnvironmentBuilder` de `crible-encode` reçoit `with_telemetry(false)`, seconde barrière sans garantie de sûreté ; un `.cargo/config.toml` porte une section `[env]` avec `ORT_DISABLE_TELEMETRY = { value = "1", force = true }`, qui couvre les binaires de test chargeant ORT dans leur propre processus. `crible-encode` ne pose jamais la variable : une bibliothèque appelée après le démarrage de tokio ou de rayon ferait la course avec des `getenv` concurrents. Aucune clé de configuration ni variable lue par Crible ne rouvre la télémétrie ; un nouvel ADR serait nécessaire.

## Environnement de développement (Mac)

- **Rust en natif** pour le développement et les tests : `cargo build`, `cargo test`. Docker ne sert qu'à l'image de déploiement.
- **ONNX Runtime pour `ort` en local** : la bibliothèque dynamique, installée par Homebrew ou téléchargée, est indiquée par `ORT_DYLIB_PATH`. La procédure est à documenter dans `CHARGER-MODELE`. Cette bibliothèque embarque la même télémétrie 1DS que celle du groupe `ci` : elle est coupée par les mécanismes d'ADR-0128 (variable forcée au début de `main`, `[env]` de `.cargo/config.toml` pour les binaires de test), donc un `cargo test` ou un `crible search` lancé depuis la racine du dépôt n'envoie rien à un tiers. Un binaire lancé hors du dépôt doit recevoir `ORT_DISABLE_TELEMETRY=1`.
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
sample_rate = 0.0            # fraction des requêtes SANS `debug: true`, dans [0, 1] ; `debug: true` est toujours enregistré (ADR-0122)
ttl_days = 7                 # 0 refusé au démarrage (ADR-0122)
max_rows = 500000            # 0 refusé au démarrage ; minimum 1 ; surcharge CRIBLE_DEBUG_JOURNAL__MAX_ROWS (ADR-0127)
max_bytes = 2147483648       # 2 Gio ; taille UTILISÉE de debug.sqlite ; 0 refusé, minimum 1 Mio ; surcharge CRIBLE_DEBUG_JOURNAL__MAX_BYTES (ADR-0127)

[encoder]
query_precision = "fp32"      # "fp32" | "int8" : précision des requêtes (ADR-0052)

[limits]
writer_queue = 16             # jobs en attente par index, le job en cours exclu ; au-delà, 503 overloaded (ADR-0103)
default_timeout_ms = 2000
wait_timeout_ms = 60000       # attente au plus de ?wait=true, puis 202 avec le job en cours (ADR-0067) ; borne aussi l'attente des jobs à l'arrêt (ADR-0104)
query_cache_entries = 1024    # cache LRU des requêtes, par instance, jamais persisté (ADR-0051)
```

**Surcharges par l'environnement (ADR-0097).** Chaque clé de `crible.toml` peut être surchargée par une variable d'environnement : `CRIBLE_` suivi du chemin de la clé en majuscules, les sections séparées par `__`. Exemples : `CRIBLE_SERVER__BIND`, `CRIBLE_LIMITS__QUERY_CACHE_ENTRIES`. La valeur est analysée comme un scalaire du type de la clé visée. Le serveur refuse de démarrer, avec un message qui nomme la variable et un code de sortie non nul, si la clé est inconnue, si la valeur ne s'analyse pas dans ce type, ou si la variable vise un tableau de tables (`[[auth.keys]]` ne se surcharge pas par l'environnement en v1). Même raisonnement qu'ADR-0031 : une faute de frappe ne doit pas passer sans effet.

Les variables `CRIBLE_*` documentées **hors configuration** sont exclues de ce contrôle, et la liste est fermée : `CRIBLE_FAILPOINT` (ADR-0068), `CRIBLE_BIN`, `CRIBLE_PYTHON`, `CRIBLE_EXPORT_REPRO`, `CRIBLE_CRASH_CHILD`, `CRIBLE_CRASH_BATCH` et `CRIBLE_CRASH_INDEX` (outils et tests), `CRIBLE_EVAL_API_KEY` (clé `read` de `crible eval --url`, ADR-0131) et les trois points d'observation des tests sous `failpoints`, `CRIBLE_TEST_BARRIER`, `CRIBLE_TEST_ENCODER` et `CRIBLE_TEST_SEGMENT_READERS` (ADR-0097, complément du 2026-11-03). Toute autre variable `CRIBLE_*` inconnue fait échouer le démarrage.

**Commande.** `crible serve [--config <chemin>]`, chemin par défaut `/etc/crible/crible.toml`. Un fichier absent ou illisible fait échouer le démarrage, avec un code de sortie non nul : le serveur ne démarre jamais sur des valeurs par défaut implicites. Les chemins `data`, `models` et `exchange` viennent de la section `[paths]` ; aucune option de ligne de commande ne les remplace.

`crible-encode` ne règle ni `intra_op_num_threads` ni le niveau d'optimisation de la session ONNX Runtime : les **défauts de la bibliothèque** sont conservés, MES-05 ayant montré qu'ils sont les plus rapides dans les deux précisions (ADR-0052). Un réglage n'aurait de sens que pour brider la consommation, et se déciderait en conteneur, en P8.

## Docker

**Une seule image serveur, CPU, multi-architecture** (`linux/arm64` pour les deux Mac — machine de développement et Mac mini —, `linux/amd64` pour la cible future ; ADR-0129). La construction suit le modèle de NextPlaid :
- étapes `cargo-chef` (cache des dépendances) puis build en release ;
- image d'exécution `debian:bookworm-slim` avec la bibliothèque ONNX Runtime CPU de l'architecture visée ;
- `ORT_DYLIB_PATH` positionné ;
- `ENV ORT_DISABLE_TELEMETRY=1` dans l'image (ADR-0128) : la télémétrie du client 1DS d'ONNX Runtime reste coupée même si le binaire est lancé hors de `main` de Crible, et l'environnement du conteneur dit la politique ;
- les actifs du tableau de bord sont **dans le binaire** (ADR-0130, décision 3) : l'image ne copie aucun dossier d'interface et n'ajoute aucune étape de build pour la servir ; `crible serve` sert `/ui` dans la même image et sur le même port ;
- utilisateur non-root ;
- port 8080 ;
- `HEALTHCHECK` sur `/health` ;
- arrêt propre sur SIGTERM et SIGINT (ADR-0099) : `/ready` passe à `503`, le serveur arrête d'accepter, il draine les requêtes en vol pendant au plus `[limits].default_timeout_ms` + 1 000 ms, il vide ensuite la file des analytics quand elles existent — écriture comprise, bornée à 2 s, un échec étant journalisé en `warn` sans reprise (ADR-0118, décision 3) —, puis la file du journal de debug quand il est activé — écriture comprise, bornée à 2 s, un échec étant journalisé en `warn` sans reprise (ADR-0122, décision 7) —, puis il sort en journalisant les requêtes coupées ; l'attente des jobs d'écriture est bornée par `[limits].wait_timeout_ms` (60 000 ms par défaut), comptée depuis le signal et en parallèle du drainage des requêtes : toute nouvelle soumission de job rend `503 overloaded` (« the server is shutting down », `Retry-After: 1`), les jobs acceptés avant le signal se terminent, et à l'échéance le processus sort en journalisant les jobs abandonnés, qui passent à `failed` au démarrage suivant (ADR-0104). Un second signal pendant le drainage fait sortir immédiatement avec un code non nul. Conséquence : le délai d'arrêt du conteneur (`stop_grace_period`, réglé en P8) doit être **au-dessus** de `wait_timeout_ms`, faute de quoi l'orchestrateur tue le processus avant la fin de l'attente.

Pas d'image CUDA : les producteurs sont en Python (ADR-0013).

**Volumes** :

| Montage | Type | Pourquoi |
|---|---|---|
| `/data` | volume nommé | Le mmap à travers le partage de fichiers macOS → VM est lent |
| `/models` | volume nommé | Modèles ONNX et tokenizers, vérifiés par hash au démarrage |
| `/exchange` | dossier monté depuis l'hôte | Lots (`inbox`), exports (`outbox`), instantanés : lectures et écritures séquentielles |
| `/etc/crible/crible.toml` | fichier monté, en lecture seule | Configuration |

**Montée et retour arrière.** Une instance se met à jour en épinglant l'image par sa version ou son empreinte, jamais par `:cpu`, étiquette mobile (ADR-0133, décision 4), après avoir pris un instantané de chaque index et gardé la configuration et l'image en place ; les contrôles sont la version rendue, les comptes de chaque index et `crible eval` sur le jeu de référence. Le retour arrière repart de l'image précédente et, quand la note de version le demande, des instantanés et de la configuration d'avant. Les commandes sont dans `docker/README.md`, sections « Monter de version » et « Revenir en arrière » ; la politique de version de la ligne 1.x qui encadre ces gestes est ADR-0143, et chaque mineure a sa note de version dans `docs/versions/<version>.md` (ADR-0143, décision 4).

**Compose d'exemple** (Legifacile) : le service `crible` et le MCP de Legifacile partagent un réseau interne. Crible n'est **jamais exposé publiquement** : seul le MCP lui parle.

**Contraintes propres au Mac** (ADR-0129) :
- la RAM disponible est celle de la VM Docker, pas celle de la machine : environ **8 Go** sur le Mac mini M1 16 Go aujourd'hui ; sur la machine de développement (MacBook Air M3, 24 Gio), la RAM allouée à Docker Desktop est indiquée dans chaque rapport de mesure. À 8,58 M passages, environ 13,2 Go de vecteurs fp16 sont attendus ;
- pas d'accès au GPU Apple ;
- le SIMD NEON est disponible ;
- un `replace_all` complet d'un grand index (`decisions`, `articles`, `jorf`) ne se fait pas sur le Mac mini : il se fait sur la machine de développement, puis l'index rejoint la machine de service par instantané et restauration (ADR-0064, ADR-0065), comme le fait déjà la campagne de mesures (ADR-0141) ; les lots quotidiens, petits, s'attachent directement sur le Mac mini (ADR-0142, décision 3).

## CI (GitHub Actions)

- **Sur chaque push et chaque PR** : `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo doc --no-deps --workspace --locked` avec `RUSTDOCFLAGS=-D warnings` sur cette seule étape (ADR-0114 : les liens intra-doc sont vérifiés à chaque push et chaque PR ; sans cette étape, ils n'étaient vus qu'en fin de phase, et il a fallu les réparer trois phases de suite, en P4, P5 et P6), `cargo test`, puis `cargo clippy -p crible-store --all-targets --locked --features failpoints -- -D warnings` et `cargo test -p crible-store --locked --features failpoints` (ADR-0068 : les tests de panne et de concurrence de `crible-store` — INV-02, INV-03, INV-04, INV-14 et les tests de panne d'`attacher_lot` — n'existent qu'avec la feature `failpoints`, et la CI ne les construisait jamais), puis `cargo clippy -p crible-server --all-targets --locked --features failpoints -- -D warnings` et `cargo test -p crible-server --locked --features failpoints` (complément du 2026-10-29 à ADR-0068 : le serveur prouve la reprise d'ADR-0108 par le point de panne `after_epoch_switch` de la compaction, avec sa propre feature `failpoints`, éteinte par défaut), puis `uv run pytest` pour `clients/python` : `addopts` exclut le marqueur `server`, donc ce job reste sans binaire Rust (ADR-0112). Le workflow entier déclare aussi `CARGO_PROFILE_DEV_DEBUG: 0` dans son `env:`, donc pour **tous** les jobs : la CI ne garde pas les informations de débogage, et `target/` reste plus petit (complément du 2026-11-01 à ADR-0068). Le job `Rust` commence en outre par une étape qui supprime des SDK préinstallés que le projet n'emploie pas (.NET, Android, GHC, CodeQL, suppression tolérante si un dossier manque) et qui affiche `df -h` au début et à la fin du job : elle libère de l'espace et rend le diagnostic vérifiable, après l'échec du run `41343484760` par `SIGBUS` du `ld` sur quatre binaires de test, sans échec de test, dans un `target/` supposé plein.
- **Sur chaque push et chaque PR, job `rust-onnx`** (ADR-0057) : un job séparé, sur `ubuntu-latest`, qui prouve le chemin ONNX. Il a un `timeout-minutes` explicite, resserré après le premier run à froid. Dans l'ordre : toolchain Rust et `uv` ; génération de `synth-small` comme le job Rust ; `uv sync --group ci` et `ORT_DYLIB_PATH` vers la bibliothèque de la roue ; restauration du cache, sinon téléchargement des fichiers depuis le dépôt de modèle Hugging Face, épinglé sur un sha de commit et vérifié par blake3 contre un manifeste versionné, puis dépôt dans `data/models/mdenseon/` ; enfin la commande ci-dessous. Le job Rust reste sans ONNX Runtime : `cargo test --locked`, puis les quatre étapes `failpoints` d'ADR-0068 — deux pour `crible-store`, deux pour `crible-server` (complément du 2026-10-29) —, puis l'étape « Test the Python client against the binary », qui exporte `CRIBLE_BIN` vers `target/debug/crible` et lance `uv run pytest -m server` (ADR-0112) ; rien d'autre pour ce job.

  ```
  cargo test --locked --workspace -- --include-ignored --skip inv_12_the_legifacile_sample_passes_the_acceptance_check --skip inv_09_legifacile_sample_queries_return_the_same_results_after_compaction --skip inv_08_legifacile_sample_queries_return_the_same_results_after_export_and_import --skip inv_12_the_reference_batch_acceptance_statistics_are_unchanged --skip api_and_cli_return_the_same_results_on_the_legifacile_sample --skip inv_03_a_docker_stop_during_an_attach_leaves_one_complete_generation --skip inv_04_a_docker_stop_during_an_attach_leaves_the_last_committed_generation
  ```

  Le cache GitHub Actions est indexé sur les hash du manifeste et n'est jamais une source de confiance : le contenu restauré est vérifié comme un téléchargement neuf. Un téléchargement fait au plus **quatre essais** par fichier, avec des attentes de 5, 15 et 45 s, un `Retry-After` respecté jusqu'à 60 s et refusé au-delà, et une **échéance globale de 600 s** pour l'artefact, bien sous le `timeout-minutes` du job ; seules les causes extérieures au contenu sont reprises (`URLError`, `TimeoutError`, autres `OSError` de lecture, `IncompleteRead`, HTTP 429 et 5xx), tandis qu'un hash faux, un autre 4xx, un manifeste invalide ou un cache divergent échouent du premier coup (ADR-0059). Un échec de téléchargement, un hash faux, un fichier absent ou une révision introuvable fait échouer le job, avec l'URL, le fichier et les hash attendu et obtenu ; aucun repli silencieux (`continue-on-error`, `|| true`) n'est admis. Les tests locaux exclus sont les **sept exclus**, nommés un par un, pour deux motifs. D'abord les cinq tests de l'échantillon Legifacile : `inv_12_the_legifacile_sample_passes_the_acceptance_check`, `inv_09_legifacile_sample_queries_return_the_same_results_after_compaction`, `inv_08_legifacile_sample_queries_return_the_same_results_after_export_and_import`, `inv_12_the_reference_batch_acceptance_statistics_are_unchanged` et `api_and_cli_return_the_same_results_on_the_legifacile_sample` ; leurs données dérivent de la production (ADR-0032), les quatre premiers passent par la recette et le cinquième encode ses requêtes, donc tous demandent ONNX Runtime et le vrai modèle, et ils restent locaux (ADR-0057, ADR-0062, ADR-0063, ADR-0085, ADR-0109). Ensuite les deux tests de l'arrêt propre en conteneur, `inv_03_a_docker_stop_during_an_attach_leaves_one_complete_generation` et `inv_04_a_docker_stop_during_an_attach_leaves_the_last_committed_generation` : ils demandent Docker et une image construite localement (`CRIBLE_DOCKER_IMAGE`, `crible:dev` par défaut), que le runner n'a pas (ADR-0132). La ligne `--skip` de chaque test arrive dans `.github/workflows/ci.yml` avec le test qui l'accompagne (ADR-0109, ADR-0132).
- **`.github/workflows/model-cache.yml`, seul écrivain du cache** (ADR-0059) : un workflow séparé, un seul job, **aucun test lancé**. Il tourne sur `schedule` le lundi et le jeudi à 03:17 UTC (`17 3 * * 1,4`), sur `push` filtré par `paths` quand `ci/model-artifact.toml` ou le fichier du workflow change, et sur `workflow_dispatch`. Il calcule la même clé que `rust-onnx`, restaure `data/models/mdenseon`, vérifie les blake3 et ne télécharge que ce qui manque, puis enregistre le cache seulement sur un défaut de cache. Le job `rust-onnx` ne fait plus qu'en **restaurer** : un seul écrivain évite la course sur une clé identique et une entrée de cache par branche. Deux passages par semaine gardent le cache de `main` sous l'éviction de sept jours.
- **`.github/workflows/publish-image.yml`, publication de l'image** (ADR-0133) : un workflow séparé, déclenché **sur un tag `v*` et sur lui seul** (ni branche, ni pull request). Le tag doit valoir `v` suivi de `[workspace.package] version` de `Cargo.toml`, sinon le workflow échoue **avant toute construction** ; la version publiée est le tag sans son `v` (`v1.0.0` publie `1.0.0`). Une **pré-version** Cargo (`1.0.0-rc.1`, tag `v1.0.0-rc.1`) reste admise et publie son étiquette de version, mais **ne déplace jamais `:cpu`** : elle sert d'essai du workflow avant la première version stable. Deux **runners natifs** construisent chacun une architecture (`ubuntu-24.04-arm` pour `linux/arm64`, `ubuntu-latest` pour `linux/amd64`), chacun avec sa portée de cache `type=gha`, et poussent le résultat **par empreinte** ; un job `merge` assemble la liste de manifestes avec `docker buildx imagetools create` et pose les étiquettes. Aucun QEMU. `:cpu` est l'étiquette **mobile de la dernière version stable publiée** : elle n'est déplacée que si le tag poussé est le plus haut tag stable `vX.Y.Z` du dépôt (tri des tags git, `fetch-depth: 0` sur le job qui décide), donc un correctif d'une version antérieure publie son étiquette et laisse `:cpu` en place. Le groupe de `concurrency` est fixe et `cancel-in-progress` vaut `false` : les publications sont sérialisées, et seules les empreintes orphelines d'une **exécution annulée** sont évitées. Avec `fail-fast: false`, si une architecture échoue après que l'autre a poussé son empreinte, le job `merge` ne tourne pas et cette empreinte reste sans étiquette dans le registre ; c'est le cas d'échec d'une seule architecture, qu'`cancel-in-progress: false` ne couvre pas. C'est la garde de version qui empêche `:cpu` de reculer, pas l'ordre des exécutions. Seul secret, `GITHUB_TOKEN` ; `contents: read` au niveau du workflow, `packages: write` sur les deux jobs qui publient seulement. Aucun agent ne crée ni ne pousse de tag : le premier est poussé par le propriétaire (ADR-0133).
- **Aucune étape de build front (ADR-0130).** Le tableau de bord est du HTML, du CSS et du JavaScript embarqués dans le binaire : la CI ne gagne ni Node, ni bundler, ni transpilation, et ses tests sont des tests Rust dans `crates/crible-server/tests/`. La CI ne gagne donc aucune étape pour P9.
- Les **jeux de données lourds** ne tournent pas en CI : ils se génèrent sur la machine cible, et les mesures se font là, puis sont consignées dans `docs/mesures/`. Un **artefact de modèle déjà exporté** n'est pas un jeu de données : le job `rust-onnx` le télécharge et le met en cache, sans jamais ré-exporter le modèle (ADR-0049, ADR-0050, ADR-0057).
