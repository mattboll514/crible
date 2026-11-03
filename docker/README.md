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

La configuration d'une instance est écrite par l'exploitant ; elle n'est jamais
dans l'image. `docker/crible.example.toml` est le modèle complet : il porte
toutes les clés exigées par `docs/07`, et ses `[paths]` désignent `/data`,
`/models` et `/exchange`. Pour un déploiement réel, copiez-le hors du dépôt,
remplissez `[auth.keys]` avec les empreintes de `crible key generate`
(ADR-0100), puis montez votre copie en lecture seule. Les commandes ci-dessous
montent le modèle tel quel : l'instance démarre sans clé, donc toutes les
routes authentifiées répondent `401`, et `/ready` répond normalement.

Préparer les volumes nommés et le dossier d'échange, puis lancer :

```
mkdir -p exchange
docker volume create crible-data
docker volume create crible-models

docker run -d --name crible -p 8080:8080 \
  --stop-timeout 65 \
  -v crible-data:/data \
  -v crible-models:/models \
  -v "$PWD/exchange:/exchange" \
  -v "$PWD/docker/crible.example.toml:/etc/crible/crible.toml:ro" \
  crible:dev
```

| Montage | Type | Pourquoi |
|---|---|---|
| `/data` | volume nommé | un mmap à travers le partage de fichiers de macOS est lent |
| `/models` | volume nommé | modèles ONNX et tokenizers, vérifiés par hash au démarrage |
| `/exchange` | dossier monté depuis l'hôte | lots, exports et instantanés : lectures et écritures séquentielles |
| `/etc/crible/crible.toml` | fichier monté, en lecture seule | configuration |

Sous Docker Desktop, un dossier de l'hôte arrive dans la VM par le partage de
fichiers (type `virtiofs`, `fakeowner`, `fuse.*`), alors qu'un volume nommé est
le système de fichiers de la VM (`ext4`). Le serveur lit
`/proc/self/mountinfo` au démarrage et écrit un avertissement quand `/data` est
un dossier monté depuis l'hôte, car le mmap des segments y est lent ; un volume
nommé n'en produit pas. `docker logs crible 2>&1 | grep "shared from the host"`
le montre.

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
docker create --name crible-test -p 18080:8080 --stop-timeout 65 \
  -v crible-test-data:/data \
  -v "$PWD/data/models:/models:ro" \
  -v "$PWD/docker/crible.example.toml:/etc/crible/crible.toml:ro" \
  crible:dev
docker cp data/indexes/synth-small crible-test:/data/indexes/synth-small
docker run --rm -u 0 -v crible-test-data:/data debian:bookworm-slim \
  chown -R 1000:1000 /data
docker start crible-test

curl -sS -o /dev/null -w '%{http_code}\n' http://127.0.0.1:18080/ready   # 200
curl -sS http://127.0.0.1:18080/ready                                    # {"status":"ready"}
```

Le volume `crible-test-data` est neuf à chaque essai sur une machine vierge ;
`docker rm crible-test`, puis `docker volume rm crible-test-data`, le remettent
à zéro (le volume ne se supprime pas tant qu'un conteneur l'utilise).

`/health` répond `200` dès la liaison du port ; `/ready` répond `503` jusqu'à ce
que les modèles soient chargés et que chaque index soit ouvert (ADR-0099). Le
`HEALTHCHECK` interroge `/health`, une condition de vie et non de démarrage.

Arrêter le conteneur par `SIGTERM` (`docker stop -t 65`) : le serveur retire
`/ready`, draine les requêtes en vol et attend les jobs d'écriture au plus
`[limits].wait_timeout_ms` (ADR-0099, ADR-0104). Le délai d'arrêt doit rester
au-dessus de cette valeur : `--stop-timeout 65` à la création du conteneur, ou
`docker stop -t 65`. Le délai par défaut de Docker (10 s) tuerait le processus
au milieu du drainage.

## Vérifier l'arrêt propre pendant une attache

`crates/crible/tests/docker_stop_during_attach.rs` envoie un `docker stop` au
milieu d'une attache, redémarre le conteneur, et vérifie l'état sur disque
(`VERIFIER-ARRET-PROPRE`, INV-03 et INV-04) :

- les analytics servies avant l'arrêt sont sur le disque, écrites par l'arrêt
  lui-même (ADR-0118, décision 3), lues sur le conteneur arrêté, avant tout
  redémarrage ;
- la génération publiée est celle d'avant l'attache (G) ou celle d'après
  (G+1), entière : ses comptes y correspondent, tous les documents du lot sont
  visibles avec G+1 et aucun avec G ;
- l'index ne garde aucun `tmp-*` de l'attache interrompue, et le job est
  terminal, ou le même dossier de lot est rejoué avec succès.

Un second test, `inv_04_…`, refait la séquence avec un `[limits].wait_timeout_ms`
d'une seconde : l'arrêt tombe alors **dans** l'attache au lieu d'attendre sa
fin, le job revient `failed` avec le message fixe de `docs/05-api.md`, la
génération reste la dernière commitée, et le même dossier de lot passe ensuite.

Le test est marqué `#[ignore]` : la CI n'a pas de Docker. Construire l'image de
la machine (voir « Construire »), puis lancer les deux tests, un à la fois
puisque chacun pilote ses propres conteneurs :

```
docker buildx build --platform linux/arm64 --progress=plain --load \
  -t crible:dev -f docker/Dockerfile .

CRIBLE_DOCKER_IMAGE=crible:dev cargo test -p crible \
  --test docker_stop_during_attach -- --include-ignored --test-threads=1
```

Le nom de l'image vient de `CRIBLE_DOCKER_IMAGE`, `crible:dev` par défaut.
Compter une quinzaine de secondes par test, attache, arrêt, redémarrage et
rejeu compris ; le lot de 200 000 passages dure quelques secondes, pour que
l'arrêt tombe pendant l'attache.

Le test crée un volume nommé et un conteneur dont le nom porte son numéro de
processus (`crible-stop-<pid>-<n>`, `crible-stop-data-<pid>-<n>`), et les
supprime lui-même, même en cas d'échec. Après une exécution, `docker ps -a` et
`docker volume ls` filtrés sur `crible-stop-` doivent être vides :

```
docker ps -a --filter name=crible-stop- --format '{{.Names}}'
docker volume ls --filter name=crible-stop-data- --format '{{.Name}}'
```

## Taille de l'image

Mesurée le 2026-11-02 avec `docker image ls`, binaire release, ONNX Runtime
1.31.0 :

| Architecture | Taille |
|---|---|
| `linux/arm64` | 177 Mo |
| `linux/amd64` | 162 Mo |

`docker image inspect` montre `ORT_DISABLE_TELEMETRY=1` dans `Config.Env` sur les
deux architectures, et `Config.User` vaut `crible`.
