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
| Tableau de bord | aucun dans l'image : la phase P9 est reportée après J10 (ADR-0136), et le binaire n'embarque aucune interface |

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

## Publier l'image

`.github/workflows/publish-image.yml` publie l'image sur GHCR. Il ne se déclenche
que sur un tag `v*` : une poussée sur une branche ne touche jamais le registre.
Le **premier tag est poussé par le propriétaire**, jamais par un agent ; le
workflow ne crée ni ne pousse de tag lui-même.

Ce que fait le workflow :

1. il vérifie que le tag correspond à la version du workspace dans `Cargo.toml`
   (`v1.0.0` pour `version = "1.0.0"`), et échoue sinon ;
2. il construit les deux architectures, `linux/arm64` sur `ubuntu-24.04-arm` et
   `linux/amd64` sur `ubuntu-latest`, deux runners **natifs** : aucune
   compilation émulée par QEMU. Chaque job met ses couches en cache
   (`type=gha`, une portée par architecture) et pousse son image par empreinte,
   sans étiquette ;
3. il assemble la liste de manifestes avec `docker buildx imagetools create`,
   qui étiquette les deux empreintes d'un coup.

Étiquettes publiées, `<owner>` étant le propriétaire du dépôt en minuscules
(GHCR l'exige) et `<version>` le tag sans son `v` initial :

| Étiquette | Sens |
|---|---|
| `ghcr.io/<owner>/crible:<version>` | la version publiée : `v1.0.0` publie `1.0.0` |
| `ghcr.io/<owner>/crible:cpu` | étiquette **mobile** de la dernière version **stable** publiée |

Règle de `:cpu` (ADR-0133, décision 4) : elle n'est déplacée que si le tag
poussé est le plus haut tag stable `vX.Y.Z` du dépôt. Les tags sont comparés
comme des versions et non comme du texte, donc `v0.10.0` passe au-dessus de
`v0.9.0`. Un correctif d'une version antérieure — `v0.1.1` poussé après `v0.2.0`
— publie son étiquette de version et laisse `:cpu` en place : l'étiquette mobile
ne recule jamais.

Une **pré-version** Cargo, c'est-à-dire une version contenant un `-` (par exemple
`1.0.0-rc.1`, tag `v1.0.0-rc.1`), est admise : elle publie son étiquette de
version mais **ne déplace jamais `:cpu`**. C'est l'essai du workflow avant la
première version stable, et il vaut mieux cet essai qu'une première publication
ratée, une image poussée sur GHCR étant difficile à retirer. La garde de version
compare le tag à `[workspace.package] version` de `Cargo.toml` : la pré-version
se met donc dans `Cargo.toml` avant de pousser le tag.

Le `v` reste dans git et disparaît de l'étiquette de l'image : c'est une
convention de gestion de versions, dont une référence d'image n'a pas besoin.

Au niveau du workflow, les permissions sont `contents: read` ; `packages: write`
n'est accordé qu'aux deux jobs qui publient, `build` et `merge`, le job `meta`,
qui décide du nom et de la version, ne demandant que `contents: read`.
L'authentification se fait par le `GITHUB_TOKEN` automatique, et aucun autre
secret n'est employé. Le groupe de `concurrency` est fixe (`publish-image`) avec
`cancel-in-progress: false` : deux publications ne se chevauchent pas, et seules
les empreintes orphelines d'une **exécution annulée** sont évitées. Avec
`fail-fast: false`, si une architecture échoue après que l'autre a poussé son
empreinte, le job `merge` ne tourne pas et cette empreinte reste sans étiquette
dans le registre ; `cancel-in-progress: false` ne couvre pas ce cas d'échec
d'une seule architecture (ADR-0133, décision 5).

Les commandes ci-dessous sont celles du **propriétaire** ; aucun agent ne crée
ni ne pousse de tag. La version `1.0.0` est la première version stable : le tag
`v1.0.0` est repointé sur le commit de la tâche `ACTUALISER-DEPOT-APRES-V1`, et
sa publication suit la fusion. La marche à suivre ci-dessous vaut pour les
**versions suivantes** ; les numéros ci-dessous ne servent que d'exemple.
D'abord l'essai, avec la pré-version écrite dans
`Cargo.toml` (`version = "1.0.0-rc.1"`), qui publie `1.0.0-rc.1` sans toucher
`:cpu` :

```
git tag v1.0.0-rc.1
git push origin v1.0.0-rc.1
```

Puis la version stable, `[workspace.package] version` remise à son numéro sans
suffixe dans `Cargo.toml` et commitée :

```
git tag v1.0.0
git push origin v1.0.0
```

Récupérer ensuite l'image publiée avec `docker pull ghcr.io/<owner>/crible:1.0.0`.

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

## Monter de version

La montée en production est un **geste du propriétaire** : les agents y sont en
lecture seule et ne redémarrent aucun service (ADR-0032). La politique de
version qui encadre ce geste est ADR-0143 : une mineure n'apporte que des ajouts
compatibles, un correctif ne change aucun contrat, et aucun `format_version` ne
change dans la ligne 1.x sans un ADR de migration (décision 2).

1. **Lire la note de version** `docs/versions/<version>.md` — `docs/versions/1.1.0.md`
   par exemple. Le dossier `docs/versions/` est créé par la première note ; celle
   de `1.1.0` sera écrite par la tâche `PUBLIER-V1.1.0` (ADR-0143, décision 4).
   Elle donne les ajouts de contrat, les corrections de comportement avec leur ADR,
   les reconstructions au premier démarrage avec la durée attendue par index, les
   clés de configuration ajoutées et les conditions du retour arrière.

2. **Prendre un instantané de chaque index**, serveur encore en marche. La route
   `POST /v1/indexes/{index}/snapshots` est réservée au rôle `admin`, ne prend
   aucun corps et rend `202` avec un job `snapshot` (ADR-0107). Pendant la copie,
   l'écrivain est suspendu et les lectures continuent (ADR-0017, ADR-0065) :

   ```
   curl -sS -X POST -H "Authorization: Bearer <clé admin>" \
     http://127.0.0.1:8080/v1/indexes/<index>/snapshots
   ```

   `crible snapshot --data <dir> --exchange <dir> --index <nom>` convient aussi,
   mais **serveur arrêté** (ADR-0065, décision 1). Relever la génération et
   l'époque que le job ou la commande rapportent : ce sont les valeurs à
   retrouver après la montée. Les instantanés vont sous
   `/exchange/snapshots/<index>-e<E>-g<G>/` et occupent donc le volume
   `/exchange` (ADR-0064, décision 3). Garder aussi une copie de la
   **configuration** (`crible.toml`) et l'**image en place**, par sa version ou
   par son empreinte :

   ```
   docker image inspect --format '{{index .RepoDigests 0}}' ghcr.io/<owner>/crible:<version>
   ```

3. **Arrêter le serveur, puis épingler la nouvelle image par sa version ou son
   empreinte, jamais par `:cpu`** : `:cpu` est une étiquette mobile, qui suivrait
   une version non choisie et ne donnerait aucune référence stable au retour
   arrière (ADR-0133, décision 4). Recréer le conteneur avec la même
   configuration et les mêmes volumes qu'à la section « Lancer » :

   ```
   docker stop -t 65 crible
   docker rm crible
   docker pull ghcr.io/<owner>/crible:<version>
   docker run -d --name crible -p 8080:8080 \
     --stop-timeout 65 \
     -v crible-data:/data \
     -v crible-models:/models \
     -v "$PWD/exchange:/exchange" \
     -v "$PWD/docker/crible.example.toml:/etc/crible/crible.toml:ro" \
     ghcr.io/<owner>/crible:<version>
   ```

   Le délai d'arrêt reste au-dessus de `[limits].wait_timeout_ms` (ADR-0104).

4. **Attendre `/ready`** : il vaut `503` jusqu'à la publication du dernier
   instantané de chaque index, puis `200` (ADR-0099) :

   ```
   until curl -fsS -o /dev/null http://127.0.0.1:8080/ready; do sleep 2; done
   ```

   Un premier démarrage après une montée peut reconstruire des fichiers dérivés,
   le lexical notamment : la note de version donne la durée attendue par index
   (ADR-0143, décision 4 ; ADR-0142 donne les durées mesurées de la 1.0). Un
   dépassement net signale un changement d'analyse non prévu, que l'empreinte
   d'analyse doit de toute façon détecter (ADR-0143, décision 3).

5. **Contrôler** : la version rendue, les comptes de chaque index, et la qualité
   sur le jeu de référence.

   ```
   docker exec crible crible --version
   docker exec crible crible inspect --data /data
   ```

   `crible inspect` rapporte l'époque, la génération, les documents, les passages
   vivants et morts et le nombre de segments : ces valeurs doivent être **égales**
   à celles relevées avant la montée. Puis `crible eval` sur le jeu de référence
   de `docs/legifacile/reference/` :

   ```
   crible eval --data <dossier> --index <index> \
     --queries docs/legifacile/reference/<index>.jsonl --models <dir> --out <dossier>
   ```

   La commande lit un index servi sans écrire (ADR-0116, décision 2) ; une
   campagne par l'API existe aussi, avec `--url` et la variable
   `CRIBLE_EVAL_API_KEY` (ADR-0131). Les rappels doivent être **au moins égaux** à
   ceux de la version précédente, écarts expliqués s'il y en a : une baisse est un
   défaut, pas un réglage (ADR-0143, décision 5).

6. **Faire passer Legifacile ensuite** : la montée de son image épinglée et de son
   client Python revient au projet Legifacile.

## Revenir en arrière

Le retour arrière repart de l'**image précédente**, plus la **restauration des
instantanés** et la **configuration d'avant** quand la note de version le demande
(ADR-0143, décisions 2 et 6). Viennent ensuite `/ready` et les **mêmes contrôles**
qu'à la montée, version, comptes et `crible eval` compris.

`crible restore` est **hors ligne** : le serveur est arrêté (ADR-0065, décision 1).
La restauration installe l'instantané comme nouvelle époque en **laissant la
génération à G** (ADR-0065, décision 2) : elle **remplace** donc tout ce qui a été
commité après l'instantané, et les **lots attachés après la montée sont à
rattacher**. L'époque remplacée n'est pas supprimée (ADR-0065, décision 4) :
rien n'est détruit, mais le disque croît jusqu'à ce qu'elle soit supprimée à la
main. `crible restore` ne prend pas d'option `--index` : le nom de l'index cible
vient de `snapshot.json` (ADR-0065, décisions 3 et 6).

Deux précisions d'ADR-0143 (décision 2) commandent ce retour :

- **L'`index.json` de la cible doit être identique à celui de l'instantané**
  (ADR-0065, décision 3 ; `docs/04-donnees-et-ingestion.md`, règles de la
  restauration). Sous 1.1, le premier `PATCH` d'un index **matérialise** les clés
  nouvelles même si personne ne les a réglées : l'`index.json` diverge alors de
  tout instantané pris avant la montée, et une 1.0 ne le relit plus. Un index que
  l'on veut pouvoir ramener en 1.0 ne reçoit donc aucun `PATCH` sous 1.1, ou bien
  le retour arrière repasse par l'instantané. La note de version dit lequel.
- **Le moyen de remettre en place l'`index.json` d'avant montée n'est pas
  tranché** (ADR-0143, décision 2, point « à trancher ») : aucune commande
  documentée ne réécrit l'`index.json` d'un index existant, et le chemin éprouvé
  est celui d'un index **absent**, créé depuis l'`index.json` de l'instantané
  (ADR-0065, décision 3). La note de version énonce la condition retenue pour
  l'index concerné ; aucune procédure n'est écrite ici tant que ce point n'est pas
  décidé.

Serveur arrêté, restaurer chaque index depuis son instantané :

```
docker stop -t 65 crible
docker rm crible
docker run --rm -v crible-data:/data -v "$PWD/exchange:/exchange" \
  ghcr.io/<owner>/crible:<version-précédente> \
  restore --data /data --snapshot /exchange/snapshots/<index>-e<E>-g<G>
```

Recréer ensuite le conteneur avec l'image précédente et la configuration d'avant,
comme à la section « Monter de version » avec `<version-précédente>`, puis
attendre `/ready` et refaire les contrôles.

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

## Démarrer la démonstration Compose

`docker/compose.example.yml` est le modèle de déploiement d'un projet : le
service `crible` (l'image de `docker/Dockerfile`), un service `client`
(conteneur Python qui interroge l'API avec `crible.Client`) et un réseau
**interne**. Crible ne publie aucun port : le service n'a pas de `ports:`, et le
réseau est déclaré `internal: true`, donc il n'a pas de passerelle et les deux
conteneurs ne joignent rien d'autre.

L'échantillon retenu est `data/indexes/synth-small` (36 Mo, 1 950 documents,
9 750 passages vivants) et son modèle `data/models/synthetic-external` (4 Ko,
modèle `external`, donc sans session ONNX Runtime). C'est le plus petit jeu de
`data/` qui donne une démo parlante : `data/indexes/j3-demo` est plus petit
(56 Ko) mais ne porte aucun passage, donc une recherche y rend une liste vide.

Ces deux dossiers vivent dans `data/`, gitignoré : un clone n'en a aucun, et le
`-v` de la préparation monterait alors une source absente. Les produire depuis la
racine du dépôt, comme la démo du jalon J1 (`docs/jalons/J1-demo.md`) :

```
cd tools
uv sync
uv run python gen_synth.py --out ../data/synth-small --overwrite
cd ..
mkdir -p data/models/synthetic-external
cat > data/models/synthetic-external/model.json <<'JSON'
{ "name": "synthetic-external", "kind": "external", "dims": 768, "normalize": true }
JSON
cargo build --release
./target/release/crible index create --data data --config data/synth-small/index.json
./target/release/crible attach --data data --models data/models --index synth-small data/synth-small/l1
./target/release/crible attach --data data --models data/models --index synth-small data/synth-small/l2
./target/release/crible attach --data data --models data/models --index synth-small data/synth-small/l3
```

Attendu, dans l'ordre : `1950 documents, 9750 live passages`, puis
`index 'synth-small' created at data/indexes/synth-small (epoch 1, generation 0)`,
puis `generation 1`, `generation 2` et `generation 3` à l'attache des trois lots.

La configuration de la démo est `docker/crible.demo.toml` : le modèle complet de
`docker/crible.example.toml`, avec **une clé de démonstration** nommée
`compose-demo` et de rôle `read`. La clé elle-même, `crible-compose-demo-key`,
est écrite dans le fichier compose où tout le monde la lit : elle ne protège
rien et ne doit jamais servir ailleurs.

Les volumes nommés ne viennent pas de l'image : la préparation y recopie l'index
et le modèle avec l'utilisateur du conteneur, `crible` (uid 1000), parce qu'un
index s'ouvre en écriture au démarrage. Elle suppose des volumes neufs ; pour
recommencer sur des volumes déjà remplis, passer d'abord par le `down -v` de la
fin de section.

```
mkdir -p exchange
docker compose -f docker/compose.example.yml -p crible-demo build
docker compose -f docker/compose.example.yml -p crible-demo run --rm --no-deps \
  -v "$PWD/data/indexes/synth-small:/seed/index:ro" \
  -v "$PWD/data/models/synthetic-external:/seed/model:ro" \
  --entrypoint sh crible -c 'mkdir -p /data/indexes /models && cp -a /seed/index /data/indexes/synth-small && cp -a /seed/model /models/synthetic-external'
```

Lancer la démo. Le client attend que le serveur soit sain sur `/ready`
(`depends_on` avec `condition: service_healthy`), rend le code 0 quand la
recherche donne le résultat attendu, et un code non nul sinon :

```
docker compose -f docker/compose.example.yml -p crible-demo up --build \
  --abort-on-container-exit --exit-code-from client
```

Sortie attendue, à la génération près de l'index :

```
client-1  | querying synth-small on http://crible:8080
client-1  | top result: synth-01872 passage 2 (categorie=cat-00), score 0.9481
client-1  | 3 results out of 9750 passages, generation 5
```

Le contrôle du service remplace celui de l'image : `/health` dit que le
processus vit, `/ready` dit que le modèle est chargé et les index ouverts
(ADR-0099), ce dont la démo a besoin. Ses délais (`start_period: 10s`, puis
15 essais à 2 s) visent l'échantillon, qui s'ouvre en moins d'une seconde ; un
projet à 8,58 M passages charge bien plus longtemps, et ADR-0099 garde ce temps
de démarrage sous surveillance : relever alors `start_period`, sans quoi le
conteneur serait déclaré `unhealthy` avant d'être prêt. Le délai d'arrêt reste
au-dessus de `wait_timeout_ms` (`stop_grace_period: 65s`), et le serveur sort
proprement sur le `SIGTERM` du `down`.

`docker/client/search_demo.py` est la démo **et** son test. Il recalcule le
vecteur de requête, le premier centre de cluster du jeu synthétique, à partir de
la graine de `tools/gen_synth.py`, puis exige que le premier résultat
appartienne à `categorie: "cat-00"` ; sinon il écrit `demo failed: …` sur la
sortie d'erreur et sort en 1. Un index absent le montre : lancer `up` sur des
volumes neufs sans préparation rend
`demo failed: unknown_index: unknown index 'synth-small'`.

Tout nettoyer, volumes compris. Le préfixe `-p` est propre à la démo : d'autres
projets peuvent tourner sur la même machine.

```
docker compose -f docker/compose.example.yml -p crible-demo down -v
```

Le contexte de l'image du client est `clients/python`, parce que le
`.dockerignore` de la racine exclut `clients/` et ne doit pas être affaibli. Le
script de démonstration est monté en lecture seule depuis `docker/client/`, et
le `.dockerignore` de `clients/python` écarte le même interdit qu'à la racine :
aucun fichier `.env*` n'entre dans un contexte (ADR-0032).

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
