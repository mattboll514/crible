# Démonstration de J8 — Docker et déploiement

Commandes à copier-coller depuis la racine du dépôt, dans l'ordre. Elles ont été
jouées le 2026-11-03 sur la machine de développement (MacBook Air M3, arm64,
Docker Desktop 28.5.1) sur la tête `2aa2f12` de `phase/P8-docker-et-deploiement`.
Les résultats observés sont donnés après chaque bloc.

Tout ce que la démonstration écrit va sous `data/j8-demo/`, qui est gitignoré.
Le projet Compose s'appelle `j8-demo`, pour ne pas toucher aux volumes d'un
autre projet. Les images restent à la fin ; les conteneurs, le réseau et les
volumes de la démonstration sont supprimés.

## 1. Repartir d'un état propre

```
cd /Users/mattboll/MattCorp/crible
docker compose -f docker/compose.example.yml -p j8-demo down -v --remove-orphans
rm -rf data/j8-demo
mkdir -p data/j8-demo/exchange
```

Le `down -v` peut afficher `no such project` au premier passage : c'est normal.

## 2. Construire l'image arm64

```
time docker build -f docker/Dockerfile -t crible:dev .
docker image ls crible:dev --format '{{.Repository}}:{{.Tag}} {{.Size}}'
```

Attendu : construction réussie, puis `crible:dev 178MB`. Sur une machine où le
cache de construction est froid, compter quelques minutes ; le 2026-11-03, avec
le cache de `cargo-chef`, la construction a pris 2 min 10 s.

## 3. Préparer la démonstration Compose

Le fichier Compose d'exemple monte `/exchange` depuis `../exchange`, c'est-à-dire
la racine du dépôt. Pour que la démonstration n'écrive que sous `data/j8-demo/`,
un fichier de surcharge remplace ce seul montage ; les autres réglages
(`/data` et `/models` en volumes nommés, configuration en lecture seule, réseau
interne, aucun port publié) restent ceux de l'exemple.

```
cat > data/j8-demo/compose.override.yml <<YML
services:
  crible:
    volumes:
      - ${PWD}/data/j8-demo/exchange:/exchange
YML
docker compose -f docker/compose.example.yml -f data/j8-demo/compose.override.yml -p j8-demo build
```

Attendu : `crible:dev Built` et `crible-demo-client:dev Built`.

Recopier l'échantillon dans les volumes nommés, avec l'utilisateur du conteneur
(`crible`, uid 1000), comme le demande `docker/README.md` : un index s'ouvre en
écriture au démarrage.

```
docker compose -f docker/compose.example.yml -f data/j8-demo/compose.override.yml -p j8-demo run --rm --no-deps \
  -v "$PWD/data/indexes/synth-small:/seed/index:ro" \
  -v "$PWD/data/models/synthetic-external:/seed/model:ro" \
  --entrypoint sh crible -c 'mkdir -p /data/indexes /models && cp -a /seed/index /data/indexes/synth-small && cp -a /seed/model /models/synthetic-external'
```

Attendu : le réseau `j8-demo_internal` et les volumes `j8-demo_crible-data` et
`j8-demo_crible-models` sont créés, la commande sort en 0 sans autre sortie.

## 4. Démarrer le serveur et voir `/health` et `/ready`

```
docker compose -f docker/compose.example.yml -f data/j8-demo/compose.override.yml -p j8-demo up -d crible
CID=$(docker compose -f docker/compose.example.yml -f data/j8-demo/compose.override.yml -p j8-demo ps -q crible)
for i in $(seq 1 60); do
  code=$(docker exec "$CID" curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:8080/ready)
  [ "$code" = 200 ] && break
  sleep 1
done
echo "ready after ${i}s: $code"
docker exec "$CID" curl -sS -w '\nHTTP %{http_code}\n' http://127.0.0.1:8080/health
docker exec "$CID" curl -sS -w '\nHTTP %{http_code}\n' http://127.0.0.1:8080/ready
docker ps --filter name=j8-demo-crible-1 --format '{{.Names}} {{.Status}}'
```

Attendu, observé le 2026-11-03 :

```
ready after 1s: 200
{"status":"ok"}
HTTP 200
{"status":"ready"}
HTTP 200
j8-demo-crible-1 Up 2 seconds (health: starting)
```

Le service ne publie aucun port : c'est voulu par `docker/compose.example.yml`
(réseau `internal: true`, aucun `ports:`). Les deux routes se lisent donc depuis
le conteneur, avec le `curl` de l'image. `health: starting` est l'état du
`HEALTHCHECK` de l'image, qui interroge `/health` toutes les 15 s.

## 5. Chercher avec le client de démonstration du Compose

```
docker compose -f docker/compose.example.yml -f data/j8-demo/compose.override.yml -p j8-demo run --rm --no-deps client
```

Attendu, observé le 2026-11-03 (le numéro de génération suit les attaches de
l'échantillon et peut différer) :

```
querying synth-small on http://crible:8080
top result: synth-01872 passage 2 (categorie=cat-00), score 0.9481
3 results out of 9750 passages, generation 5
```

Le client sort en 0 parce que le premier résultat appartient au groupe
`cat-00` ; il sortirait en 1 avec `demo failed: …` sinon.

## 6. Arrêter proprement et lire le code de sortie

```
CID=$(docker compose -f docker/compose.example.yml -f data/j8-demo/compose.override.yml -p j8-demo ps -q crible)
time docker stop -t 65 "$CID"
echo "docker stop exit=$?"
docker inspect -f 'ExitCode={{.State.ExitCode}} OOMKilled={{.State.OOMKilled}} Status={{.State.Status}}' "$CID"
docker logs --tail 4 "$CID" 2>&1
```

Attendu, observé le 2026-11-03 :

```
real 0.11
docker stop exit=0
ExitCode=0 OOMKilled=false Status=exited
{"timestamp":"...","level":"INFO","fields":{"message":"received SIGTERM"},"target":"crible_server::lifecycle"}
{"timestamp":"...","level":"INFO","fields":{"message":"stopping: new requests are refused and the requests in flight are drained"},"target":"crible_server::lifecycle"}
{"timestamp":"...","level":"INFO","fields":{"message":"the analytics are flushed","events":1,"rows":1,"shapes":0},"target":"crible_server::analytics"}
```

Le délai `-t 65` reste au-dessus de `[limits].wait_timeout_ms = 60000` de
`docker/crible.demo.toml`, comme le demande `docker/README.md` : le serveur
reçoit `SIGTERM`, retire `/ready`, draine les requêtes et vide les analytics.
Un `docker stop` sans `-t` (10 s par défaut) tuerait le processus au milieu du
drainage.

## 7. Construire et démarrer l'image amd64

La construction amd64 passe par l'émulation QEMU sur un Mac arm64. Le
2026-11-03, elle a pris 7 min 26 s avec le cache, pour une image de 163 Mo
(162 729 501 octets).

```
time docker buildx build --platform linux/amd64 -f docker/Dockerfile -t crible:dev-amd64 --load .
docker image inspect -f 'arch={{.Architecture}} os={{.Os}} size={{.Size}}' crible:dev-amd64
docker run --rm --platform linux/amd64 --entrypoint crible crible:dev-amd64 --version
docker image inspect -f '{{range .Config.Env}}{{println .}}{{end}}' crible:dev-amd64 | grep -E 'ORT_DISABLE_TELEMETRY|ORT_DYLIB_PATH'
docker image inspect -f 'User={{.Config.User}}' crible:dev-amd64
```

Attendu, observé le 2026-11-03 :

```
arch=amd64 os=linux size=162729501
crible 0.1.0
ORT_DYLIB_PATH=/opt/ort/libonnxruntime.so.1.31.0
ORT_DISABLE_TELEMETRY=1
User=crible
```

L'image amd64 démarre donc sous émulation en moins d'une seconde pour `--version`.
Ne pas lancer la construction multi-architecture en une seule commande pour ce
jalon : elle prend plus de 25 minutes (voir `docker/README.md`).

## 8. Nettoyer la démonstration, garder les images

```
docker compose -f docker/compose.example.yml -f data/j8-demo/compose.override.yml -p j8-demo down -v --remove-orphans
docker ps -a --filter name=j8-demo --format '{{.Names}}'
docker volume ls --format '{{.Name}}' | grep j8-demo || echo "aucun volume j8-demo"
docker image ls --format '{{.Repository}}:{{.Tag}} {{.Size}}' | grep -E '^crible'
```

Attendu : `down -v` supprime le conteneur, le réseau et les deux volumes ; les
deux premières vérifications ne rendent rien ; les images restent, dont
`crible:dev 178MB`, `crible:dev-amd64 163MB` et `crible-demo-client:dev 637MB`.

Les fichiers de la démonstration restent sous `data/j8-demo/` (`exchange/` vide
et `compose.override.yml`) et peuvent être supprimés avec le dossier.
