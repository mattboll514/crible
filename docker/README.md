# Image Docker du serveur Crible

Une seule image serveur, CPU, multi-architecture : `linux/arm64` pour les deux
Mac et `linux/amd64` pour la cible future (ADR-0129). La construction suit le
modèle de NextPlaid : `cargo-chef` met les dépendances en cache, la compilation
se fait en release, et l'image d'exécution est `debian:bookworm-slim`
(`docs/07-stack-et-deploiement.md`, section « Docker »).

## Ce que contient l'image

| Élément | Valeur |
|---|---|
| Binaire | `/usr/local/bin/crible`, lancé par `ENTRYPOINT ["crible"]` et `CMD ["serve"]` |
| Configuration | `/etc/crible/crible.toml`, à monter en lecture seule |
| ONNX Runtime | bibliothèque CPU 1.31.0 dans `/opt/ort`, désignée par `ORT_DYLIB_PATH` |
| Télémétrie | `ORT_DISABLE_TELEMETRY=1` dans l'environnement de l'image (ADR-0128) |
| Utilisateur | `crible`, uid 1000, non privilégié |
| Port | 8080, `EXPOSE 8080` |
| Dossiers | `/data`, `/models`, `/exchange`, `/etc/crible` |
| Contrôle | `HEALTHCHECK` sur `GET /health` toutes les 15 s |
| Tableau de bord | dans le binaire (ADR-0130), aucun dossier d'interface dans l'image |

La version d'ONNX Runtime est la même que celle du groupe `ci` de `docs/07`. Le
`Dockerfile` l'épingle (`ORT_VERSION`) et vérifie chaque archive par son
SHA-256, par architecture, avant de l'extraire : le téléchargement vient des
versions officielles du dépôt `microsoft/onnxruntime`, jamais d'un binaire non
contrôlé.

## Construire

Architecture de la machine (arm64 sur les Mac) :

```
docker buildx build --platform linux/arm64 --load \
  -t crible:dev -f docker/Dockerfile .
```

Les deux architectures. Le pilote `docker` de Docker Desktop refuse une
construction multi-plateforme : il faut un constructeur `docker-container`, ou
activer le magasin d'images containerd.

```
docker buildx create --name crible-builder --driver docker-container \
  --platform linux/arm64,linux/amd64

docker buildx build --builder crible-builder \
  --platform linux/arm64,linux/amd64 -t crible:multi \
  -f docker/Dockerfile --output type=oci,dest=/tmp/crible-multi.tar .
```

Une image multi-plateforme ne se charge pas par `--load` : elle s'exporte en
OCI, se pousse vers un registre (`--push`), ou se construit sans sortie pour
seulement prouver la réussite. La compilation `amd64` passe par l'émulation
QEMU sur un Mac arm64 : compter plusieurs minutes.

Le contexte de construction est filtré par `.dockerignore` à la racine. Il ne
contient **aucun** fichier `.env*` (`.env.prod` porte les accès de production,
ADR-0032), ni `data/`, `models/`, `exchange/`, `target/`, `.git/` ou `.claude/`.
L'image est autosuffisante : elle ne reçoit ni données ni modèle.

## Lancer

Préparer les volumes nommés, puis monter la configuration et les données :

```
docker volume create crible-data
docker volume create crible-models

docker run -d --name crible -p 8080:8080 \
  -v crible-data:/data \
  -v crible-models:/models \
  -v "$PWD/exchange:/exchange" \
  -v "$PWD/crible.toml:/etc/crible/crible.toml:ro" \
  crible:dev
```

`/data` et `/models` sont des volumes nommés : un mmap à travers le partage de
fichiers de macOS est lent. `/exchange` est monté depuis l'hôte : les lots, les
exports et les instantanés s'y lisent et s'y écrivent de façon séquentielle.

Les chemins de `[paths]` de `crible.toml` désignent `/data`, `/models` et
`/exchange`. Le serveur refuse de démarrer sur des valeurs par défaut
implicites : sans configuration montée, `crible serve` échoue et le conteneur
sort avec un code non nul.

## Vérifier avec un modèle et un index de test

Les données de développement vivent dans `data/`, gitignoré. Le plus petit
montage qui répond `200` sur `/ready` est un index à modèle externe
(`data/indexes/synth-small`, 36 Mo) et le modèle `data/models/synthetic-external`
(4 Ko). L'index `data/indexes/articles` avec le modèle `data/models/mdenseon`
charge en plus une vraie session ONNX Runtime, ce qui prouve la bibliothèque de
l'image.

Un index doit pouvoir être ouvert en écriture : recopier l'index dans le volume
`/data` plutôt que de le monter en lecture seule.

```
docker volume create crible-test-data
docker run --rm -u 0 -v crible-test-data:/data debian:bookworm-slim \
  mkdir -p /data/indexes
docker create --name crible-test -p 18080:8080 \
  -v crible-test-data:/data \
  -v "$PWD/data/models:/models:ro" \
  -v "$PWD/test.toml:/etc/crible/crible.toml:ro" \
  crible:dev
docker cp data/indexes/synth-small crible-test:/data/indexes/synth-small
docker run --rm -u 0 -v crible-test-data:/data debian:bookworm-slim \
  chown -R 1000:1000 /data
docker start crible-test

curl -sS -o /dev/null -w '%{http_code}\n' http://127.0.0.1:18080/ready   # 200
curl -sS http://127.0.0.1:18080/ready                                    # {"status":"ready"}
```

`/health` répond `200` dès la liaison du port ; `/ready` répond `503` jusqu'à ce
que les modèles soient chargés et que chaque index soit ouvert (ADR-0099). Le
`HEALTHCHECK` interroge `/health`, une condition de vie et non de démarrage.

Arrêter le conteneur par `SIGTERM` (`docker stop`) : le serveur retire `/ready`,
draine les requêtes en vol et attend les jobs d'écriture au plus
`[limits].wait_timeout_ms` (ADR-0099, ADR-0104). Le délai d'arrêt doit rester
au-dessus de cette valeur.

## Taille de l'image

Mesurée le 2026-11-02 avec `docker image ls`, binaire release, ONNX Runtime
1.31.0 :

| Architecture | Taille |
|---|---|
| `linux/arm64` | 177 Mo |
| `linux/amd64` | 162 Mo |

`docker image inspect` montre `ORT_DISABLE_TELEMETRY=1` dans `Config.Env` sur les
deux architectures, et `Config.User` vaut `crible`.
