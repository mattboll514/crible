# ADR-0112 — Client Python HTTP : méthodes, exceptions typées, attente des jobs et tests contre le binaire

- **Statut** : Accepté (accepté par anticipation par le propriétaire le 2026-10-30, confirmé par le propriétaire le 2026-10-30)
- **Date** : 2026-10-30
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, tâche CLIENT-PYTHON-HTTP (phase P6), sur délégation du propriétaire pour la troisième série de P6 et sur les verdicts de `@architecte` du 2026-10-30
- **Remplace** : — (ne remplace aucune décision : complète ADR-0101, ADR-0103, ADR-0067 et ADR-0111, sans modifier le corps d'aucun d'eux)

## Contexte

### La fiche et ce qu'elle demande

La fiche `CLIENT-PYTHON-HTTP` (`docs/phases/P6-serveur-et-api.md:137-147`) demande quatre livrables : un `crible.Client(base_url, api_key)` qui expose `search`, `search_multi`, `get_document`, `attach_batch`, `upsert_documents`, `patch_documents`, `job` et `wait` (`:141`) ; des erreurs typées, `CribleError` portant `code` (`:142`) ; une version asynchrone sur `httpx.AsyncClient` (`:143`) ; des tests contre un serveur lancé dans la fixture pytest (`:144`). Ses deux critères d'acceptation sont : « le scénario J6 se joue entièrement en Python » (`:146`), et « chaque code d'erreur devient une exception typée » (`:147`).

Le jalon J6 (`docs/phases/P6-serveur-et-api.md:5-9`) fixe le scénario : « créer les index de l'échantillon, attacher les lots, chercher en simple et en multi-index, patcher, compacter, exporter », et ce scénario doit passer « via Swagger UI et via le client Python ».

### Ce que le dépôt contient aujourd'hui

`clients/python/src/crible/errors.py` ne définit qu'une exception, `BatchError`, levée par le constructeur de lots et sans rapport avec l'API : le module des erreurs d'API n'existe pas. `clients/python/pyproject.toml` ne déclare ni `httpx` ni marqueur pytest, et ses options sont réduites à `testpaths = ["tests"]`.

`clients/python/tests/test_online_batch.py:87` saute par `pytest.skip` quand `CRIBLE_BIN` est absent. Le job Python de la CI n'a pas Rust : il ne peut pas fournir ce binaire. Le job Rust de la CI porte en revanche déjà une étape « Test the Python client against the binary » (`.github/workflows/ci.yml:51-56`), qui exporte `CRIBLE_BIN` vers `target/debug/crible` et lance `uv run pytest tests/test_online_batch.py`.

### Ce que la conception et les ADR voisins posent

`docs/05-api.md:34-52` est la référence de l'API : un tableau de **quatorze** codes d'erreur (`invalid_request`, `invalid_filter`, `unauthorized`, `forbidden`, `unknown_index`, `not_found`, `immutable_setting`, `batch_conflict`, `payload_too_large`, `incompatible_model`, `acceptance_failed`, `overloaded`, `timeout`, `internal`), et le format du corps d'erreur, fixé par ADR-0101 : `{ "error": { "code", "message", "details", "request_id" } }`, `details` valant `{}` sauf clé documentée (`path` pour `invalid_filter`). Toute réponse `overloaded` porte `Retry-After: 1` (ADR-0103). `docs/05-api.md` ne contient **aucune section** sur le client Python : c'est un document de l'API, pas du client.

ADR-0103 fixe les jobs : `queued → running → succeeded | failed`, une erreur de job au format d'ADR-0101 **sans `request_id`**, un job inconnu ou purgé en `404 not_found`. ADR-0067 fixe l'échéance de `?wait=true` : `200` si le job finit dans `wait_timeout_ms`, sinon `202` avec le job dans son état courant, jamais `504`. ADR-0110 fixe la forme de `targets` pour la recherche multi-index : `[{ "index", "filter" }]`. ADR-0111 décide que la spécification est générée depuis le code et que « le client Python est écrit à la main ».

### Les verdicts de `@architecte` du 2026-10-30

Le verdict relève un **écart interne à la fiche** : les huit méthodes de `:141` ne suffisent pas au scénario J6, qui demande de créer un index, de le compacter et de l'exporter, alors que ces trois opérations n'ont pas de méthode dans la fiche. Il relève aussi un **écart de règle de saut** : `docs/08-invariants-et-tests.md:43` fait de `#[ignore]` le seul mécanisme de saut autorisé côté Rust, et le `pytest.skip` de `test_online_batch.py:87` est le saut silencieux que cette règle interdit dans son esprit — un test qui saute sans son prérequis se lit comme vert et ne prouve rien. Il relève enfin **quinze points NON-COUVERT**, dont le nom des exceptions, la portée des options, l'encodage des vecteurs, le comportement de `wait`, la liste des dépendances, l'emplacement des tests et la preuve de la couverture des codes.

## Décision

### Décision 1 — les méthodes et les options

**Constat.** La fiche nomme huit méthodes (`:141`) ; le scénario J6 (`:5-9`) en demande davantage. Aucun document ne dit comment une option de requête est transmise au corps JSON, ni comment un `vector` numpy est transporté.

**Décision.**

- `crible.Client(base_url, api_key, *, timeout=60.0)` et `crible.AsyncClient`, **mêmes méthodes**, les secondes déclarées `async`. Les deux sont utilisables en gestionnaire de contexte et exposent `close()`.
- Les huit méthodes de la fiche, avec ces signatures :
  - `search(index, **options)`,
  - `search_multi(targets, **options)`,
  - `get_document(index, doc_id, *, text=False)`,
  - `attach_batch(index, directory)`,
  - `upsert_documents(index, batch_id, documents, deletions=None)`,
  - `patch_documents(index, documents)`,
  - `job(job_id)`,
  - `wait(job, *, timeout=600.0, poll_interval=0.5)`.
- Plus celles qu'exige le scénario J6 : `create_index(config)`, `list_indexes()`, `get_index(index)`, `compact(index)`, `export(index)`.
- **La liste de la fiche est un plancher.** Les autres routes — `PATCH` et `DELETE` d'index, instantanés, liste des jobs — restent **hors de la tâche** et sont une idée pour une tâche ultérieure.
- Les options de recherche reprennent **les noms de champs de `docs/05`** tels quels (`query`, `vector`, `mode`, `k`, `fusion`, `group_by`, `passages_per_document`, `dedup`, `return`, `exact`, `timeout_ms`), et **seules les options fournies** — non `None` — sont envoyées : un `k=None` n'écrit pas `"k": null` et laisse le défaut du serveur s'appliquer.
- `targets` est une liste de `{"index", "filter"}` (ADR-0110).
- `vector` accepte un tableau numpy ou une séquence de flottants ; le client l'encode en **base64 fp32 little-endian**, la forme attendue par l'API.

**Conséquences.** Un appelant Python atteint tout le scénario J6 sans descendre à `httpx`, et les options gardent le vocabulaire de `docs/05`, donc un champ ajouté au corps se transmet sans nouvelle méthode. Coût : `**options` n'est pas vérifié par le client, si bien qu'une faute de frappe part au serveur et revient en `400 invalid_request` — ce que la décision 2 d'ADR-0109 rend explicite côté serveur.

### Décision 2 — la forme des retours

**Constat.** Rien ne dit si le client rend des objets typés ou le JSON brut. Un client typé devrait suivre chaque évolution du corps ; un client brut laisse l'appelant lire un dictionnaire.

**Décision.** Les méthodes rendent **le JSON de la réponse en `dict` Python**, sans dataclass ni `TypedDict`. Le client reste un miroir mince de l'API, dont `docs/05` est la référence : un champ ajouté à une réponse ne casse aucun client.

Les méthodes qui **créent un job** — `attach_batch`, `upsert_documents`, `patch_documents`, `compact`, `export` — rendent le job (`202`) **sans attendre** ; `wait` attend (décision 4).

**Conséquences.** Aucun décalage entre la réponse du serveur et ce que voit l'appelant, et un test peut comparer directement le `dict` au corps attendu. Coût : pas d'autocomplétion ni de contrôle de type sur les clés de la réponse ; la découverte d'un champ passe par `docs/05` ou par la spécification OpenAPI d'ADR-0111.

### Décision 3 — les exceptions typées

**Constat.** La fiche exige `CribleError` avec `code` (`:142`), sans dire ce que porte l'exception, ni ce qu'elle devient pour un code inconnu, pour une erreur de transport ou pour une réponse qui n'a pas le corps d'ADR-0101. Le tableau de `docs/05:38-52` compte quatorze codes.

**Décision.** Le module `crible.errors` gagne :

- une base **`CribleError(Exception)`** portant `code`, `status`, `message`, `details`, `request_id` et `retry_after` (valeur de l'en-tête `Retry-After` en secondes, sinon `None`) ;
- **une sous-classe par code** du tableau des erreurs de `docs/05`, nommée selon la règle `<Code en CamelCase>Error` (`InvalidRequestError`, `InvalidFilterError`, `UnauthorizedError`, `ForbiddenError`, `UnknownIndexError`, `NotFoundError`, `ImmutableSettingError`, `BatchConflictError`, `PayloadTooLargeError`, `IncompatibleModelError`, `AcceptanceFailedError`, `OverloadedError`, `RequestTimeoutError`, `InternalError`) ;
- la sous-classe du code `timeout` s'appelle **`RequestTimeoutError`**, et non `TimeoutError`, pour ne pas masquer l'exception **standard** `TimeoutError` que `wait` lève à son échéance (décision 4), importée dans le même module ;
- un **code inconnu du client** rend la base `CribleError`, `code` conservé tel quel : un serveur plus récent ne casse pas un client plus ancien ;
- une réponse d'erreur **sans corps au format d'ADR-0101** rend `CribleError` avec `code = None` et le statut ;
- une **erreur de transport** `httpx` (connexion, délai) rend `CribleTransportError`, sous-classe de `CribleError` avec `code = None`, l'exception d'origine en `__cause__` ;
- `BatchError` reste **inchangé** : c'est une erreur de construction de lot, hors du chemin HTTP ;
- **aucun réessai automatique**, même sur `503 overloaded` : l'appelant lit `retry_after`. Un réessai implicite masquerait un état d'index — l'`overloaded` d'ADR-0088 dit que le lexical est en retard, non certifié ou illisible, et que la réouverture en écriture le répare.

**Conséquences.** Un appelant écrit `except NotFoundError:` au lieu de comparer une chaîne, et lit `retry_after` pour décider lui-même d'attendre. Coût : une classe par code, donc un fichier à tenir à jour quand le tableau de `docs/05` change, et la preuve de cette tenue est la décision 8.

### Décision 4 — `wait`

**Constat.** ADR-0103 décrit les états d'un job, ADR-0067 décrit `?wait=true` avec son échéance de 60 s, mais rien ne dit comment le client attend, ni ce qu'il lève quand un job échoue ou quand l'attente dépasse son propre délai.

**Décision.**

- `wait` interroge `GET /v1/jobs/{id}` toutes les `poll_interval` secondes jusqu'à un état terminal ; son argument est l'objet job rendu par une méthode qui crée un job, dont le client lit `job_id` ;
- `succeeded` rend le job ;
- `failed` lève **l'exception typée du `error.code` du job** (décision 3), avec `request_id` à `None` — ADR-0103 décide qu'une erreur de job ne porte pas de `request_id` ;
- au-delà de `timeout` (600 s par défaut), `wait` lève `TimeoutError`, l'exception **standard** de Python ;
- le `?wait=true` du serveur (ADR-0067) **n'est pas utilisé** par le client : la boucle d'interrogation est la sienne, donc une échéance réseau n'est pas confondue avec une échéance serveur, et `poll_interval` reste réglable.

**Conséquences.** Le client dit exactement pourquoi un job a échoué, avec le même vocabulaire que les réponses HTTP (`except NotFoundError:` vaut pour un `GET` comme pour un job). Coût : `wait` fait une requête par `poll_interval`, soit deux par seconde au défaut, sur une route `write` ; c'est le prix d'un suivi indépendant de l'échéance du serveur.

### Décision 5 — le transport et la dépendance

**Constat.** `httpx` est cité par `docs/07-stack-et-deploiement.md:52` parmi les paquets Python, mais n'est pas une dépendance de `clients/python/pyproject.toml`. Rien ne fixe le délai, l'en-tête d'authentification ni la façon d'éviter que les versions synchrones et asynchrones divergent.

**Décision.**

- **`httpx` ajouté** aux dépendances de `clients/python/pyproject.toml`, avec la borne **`httpx>=0.27,<1`** ; `uv.lock` à jour ;
- en-tête **`Authorization: Bearer <api_key>`** sur chaque requête ;
- **un seul délai** `httpx`, réglable par le constructeur (`timeout=60.0`), valeur par défaut au-dessus du `timeout_ms` maximal de 30 s : le client n'expire pas avant le serveur ;
- le client **n'envoie pas** de `x-request-id` : le serveur le génère (ADR-0098), et le client lit celui de la réponse dans l'exception (décision 3) ;
- **un cœur commun** porte la construction des requêtes, l'encodage des vecteurs, l'analyse des réponses et des erreurs ; deux **transports minces**, synchrone et asynchrone, l'appellent. Les deux versions ne peuvent donc pas diverger sur le contrat, seulement sur l'appel.

**Conséquences.** Une seule dépendance nouvelle, bornée, et deux versions du client à comportement identique. Coût : `httpx` entre dans l'arbre de dépendances du paquet installé par le MCP de Legifacile (ADR-0111) ; la borne `<1` protège d'une rupture majeure, pas des mineures.

### Décision 6 — les tests contre le binaire, marqueur `server` et échec sans `CRIBLE_BIN`

**Constat.** La fiche demande « des tests contre un serveur lancé dans la fixture pytest » (`:144`). Le seul test existant qui lance le binaire saute sans `CRIBLE_BIN` (`test_online_batch.py:87`), et `docs/08-invariants-et-tests.md:43` fait de `#[ignore]` le seul mécanisme de saut autorisé : un `pytest.skip` est exactement ce que la règle refuse. Le job Python de la CI n'a pas de binaire Rust ; le job Rust en a un et possède déjà l'étape qui l'exporte.

**Décision.**

- les tests qui lancent `crible` portent le marqueur pytest **`server`**, déclaré dans `clients/python/pyproject.toml` ;
- ils **échouent** (`pytest.fail`) sans `CRIBLE_BIN`, avec le message qui donne la commande qui produit le binaire. **Jamais de saut** : un test sans son prérequis est rouge, pas vert ;
- `addopts` exclut le marqueur par défaut (`-m "not server"`) : `uv run pytest` reste vert sans binaire, et **le job Python de la CI ne change pas** ;
- le job Rust lance **`uv run pytest -m server`** dans son étape « Test the Python client against the binary », à la place de `uv run pytest tests/test_online_batch.py` ;
- `tests/test_online_batch.py` passe au **même mécanisme** : marqueur `server`, `pytest.skip` remplacé par un échec ;
- la fixture lance `crible serve` avec un `crible.toml` **temporaire** — dossier de données temporaire, clés de test des trois rôles, port libre —, attend `/ready`, et arrête le serveur par **SIGTERM** (ADR-0099) ;
- les données sont fabriquées par le test avec `crible.BatchWriter`, sur un modèle **`external`** et des vecteurs aléatoires de petite dimension : aucun test `server` ne dépend de `data/` ni d'ONNX Runtime.

**Conséquences.** Aucun saut silencieux ne subsiste côté Python, et le test qui lance le binaire tourne là où le binaire existe : le job Rust, déjà équipé de `CRIBLE_BIN`. Coût : `uv run pytest` seul ne prouve plus le chemin serveur — c'est le job Rust qui le prouve, et le développeur qui veut le rejouer exporte `CRIBLE_BIN` lui-même.

### Décision 7 — le scénario J6 en Python

**Constat.** Le critère `:146` demande que le scénario J6 se joue « entièrement en Python ». Le jalon J6 nomme en plus un passage sur l'échantillon Legifacile, qui demande ONNX Runtime et le vrai modèle, donc le job `rust-onnx` et une exclusion de plus au sens d'ADR-0062.

**Décision.** Un test `server` joue **tout le scénario J6** sur les données synthétiques de la décision 6, **en synchrone**, et **l'essentiel en asynchrone** : créer deux index, attacher un lot à chacun, chercher en simple et en multi-index, `upsert_documents`, `patch_documents`, compacter, exporter — chaque job attendu par `wait`.

Le passage sur l'échantillon Legifacile (modèle ONNX) relève de la **démonstration du jalon** : `docs/jalons/J6-demo.md`, écrit par TESTER-CHARGE-CONCURRENTE (`docs/phases/P6-serveur-et-api.md:156`), puis rapport de `@jalon`. Ce n'est pas un test local exclu de la CI (ADR-0062).

**Conséquences.** Le critère d'acceptation a une preuve continue, dans le job Rust, sans ONNX Runtime ni exclusion nouvelle, et le client est prouvé sur les treize méthodes des décisions 1 et 2. Coût : la démonstration sur l'échantillon reste un geste de jalon, non rejoué à chaque commit ; c'est le partage qu'ADR-0062 et ADR-0110 ont déjà retenu pour leurs preuves.

### Décision 8 — la couverture des codes d'erreur

**Constat.** Le critère `:147` exige que « chaque code d'erreur devienne une exception typée ». Rien ne dit comment cette correspondance est tenue quand `docs/05` gagne ou perd un code. Les quatorze codes n'ont pas tous le même coût d'atteinte : `unauthorized` demande une clé fausse, `overloaded` demande de remplir une file ou un sémaphore.

**Décision.**

- un **test unitaire lit le tableau des erreurs de `docs/05`** et vérifie que **chaque code y a sa sous-classe**, et inversement — dans les deux sens, puisque le tableau est la référence ;
- des **tests `server` provoquent contre le binaire chaque code atteignable à peu de frais** et vérifient la **classe**, `code`, `status` et `request_id` : au moins `unauthorized`, `forbidden`, `invalid_request`, `invalid_filter`, `not_found`, `incompatible_model`, et `overloaded` **si** il est atteignable de façon déterministe.

**Conséquences.** Ajouter un code à `docs/05` sans sa classe fait échouer un test unitaire, sans binaire et sans réseau ; la preuve sur le binaire vérifie en plus que le statut et le `request_id` arrivent jusqu'à l'exception. Coût : le test unitaire lit un fichier Markdown, donc il dépend de la forme du tableau ; ce couplage est voulu, `docs/05` étant la référence des codes.

### Ce qui n'est pas décidé ici

- le **texte exact des messages** des exceptions côté client (`__str__`), hors la reprise de `message` du serveur ;
- le **marqueur des codes non atteignables en CI** (`internal`, `acceptance_failed`, `batch_conflict`, `immutable_setting`, `payload_too_large`, `unknown_index`, `timeout`) : ils sont couverts par le test unitaire de la décision 8, sans preuve sur le binaire ;
- les **routes hors périmètre** (`PATCH` et `DELETE` d'index, instantanés, liste des jobs) : idée pour une tâche ultérieure ;
- le **contenu de `clients/python/README.md`** : il appartient à `@dev`, avec la tâche ;
- le **détail de la fixture** (découpage en modules, noms de fonctions) : choix d'implémentation, sous les contraintes de la décision 6.

## Conséquences

- **Positives** :
  - un appelant Python joue tout le scénario J6 avec un seul paquet, sans `httpx` ni `requests` à écrire ;
  - les erreurs sont des classes, choisies par le serveur, et `retry_after` laisse la décision de réessayer à l'appelant ;
  - le client suit `docs/05` et la spécification d'ADR-0111 sans génération de code : un champ ajouté ne casse rien ;
  - aucun saut silencieux ne reste côté Python : un test sans binaire est rouge ;
  - le job Python de la CI ne change pas, et le job Rust gagne une preuve de plus sans dépendance nouvelle ;
  - les versions synchrones et asynchrones partagent un cœur unique, donc un contrat unique.
- **Négatives / coûts** :
  - `httpx` entre dans les dépendances installées par le MCP de Legifacile ;
  - `uv run pytest` seul ne prouve plus le chemin serveur : il faut le job Rust, ou `CRIBLE_BIN` exporté ;
  - une classe d'exception par code, et un test unitaire couplé à la forme du tableau de `docs/05` ;
  - `wait` interroge toutes les `poll_interval` secondes, soit deux requêtes par seconde au défaut, sur une route `write`.
- **À surveiller** :
  - le coût réel des tests `server` dans le job Rust : s'ils rallongent nettement le job, c'est la fixture qui se règle, pas la preuve qui se retire ;
  - la fréquence des `overloaded` vus par le client : si elle domine, la question d'un réessai explicite (jamais implicite) revient au propriétaire ;
  - tout code ajouté à `docs/05` : le test unitaire de la décision 8 est le signal, et son échec est attendu tant que la classe manque ;
  - la borne `httpx<1` : une version majeure demandera un ADR, pas un relèvement silencieux ;
  - `docs/08-invariants-et-tests.md:43` : la règle du saut y est écrite pour Rust ; le marqueur `server` n'ajoute aucun saut et la remplace par un échec, donc `docs/08` reste inchangé. Si la règle devait un jour couvrir Python explicitement, ce serait par un ADR.

- **Documents mis à jour par cet ADR** :

| Document | Modification |
|---|---|
| `docs/adr/0112-client-python-http.md` | cet ADR |
| `docs/adr/README.md` | ligne d'index de cet ADR |
| `docs/07-stack-et-deploiement.md`, « CI (GitHub Actions) » | le job Rust lance `uv run pytest -m server` dans son étape « Test the Python client against the binary », et la description du job nomme cette étape ; le job Python reste sur `uv run pytest`, `addopts` excluant `server` |

Sont **non modifiés** : `docs/05-api.md` (aucune section ne parle du client Python ; le client s'y réfère, il ne la modifie pas), `docs/08-invariants-et-tests.md` (règle du saut écrite pour Rust, voir « À surveiller »), `docs/suivi.md` (tenu par l'orchestrateur), `clients/python/README.md` (`@dev`), tout fichier de code, tout test, `.github/workflows/ci.yml` (`@dev`), et les ADR antérieurs cités.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| S'en tenir aux huit méthodes de la fiche | Le scénario J6 du même document demande de créer un index, de le compacter et de l'exporter (`docs/phases/P6:6`) : le critère `:146` serait inatteignable avec le plancher seul. |
| Exposer toutes les routes de l'API dès cette tâche | `PATCH` et `DELETE` d'index, instantanés et liste des jobs ne servent aucun critère de la fiche ; les livrer élargirait la tâche sans preuve, et ils restent une idée. |
| Des dataclasses ou des `TypedDict` pour les réponses | Chaque champ ajouté au serveur demanderait une modification du client dans le même changement, alors que la décision d'ADR-0111 est un client écrit à la main contre `docs/05` comme référence. Le `dict` est le miroir exact. |
| Une exception unique avec un attribut `code` | Le critère `:147` demande « une exception typée » par code : un `except` sur la classe est plus sûr qu'une comparaison de chaîne, qu'une faute de frappe rend silencieusement fausse. |
| Réessayer automatiquement sur `503 overloaded` | Un `overloaded` d'ADR-0088 dit que le lexical est en retard, non certifié ou illisible : un réessai masquerait cet état d'index. Le client expose `retry_after` et laisse la décision à l'appelant. |
| Lever `BatchError` pour les erreurs d'API | `BatchError` appartient à la construction des lots, hors du chemin HTTP ; la fusionner avec `CribleError` brouillerait deux causes distinctes et casserait les appelants existants. |
| Utiliser `?wait=true` au lieu de la boucle d'interrogation | L'échéance de 60 s d'ADR-0067 est celle du serveur, et un `202` à l'échéance n'est pas une erreur : le client devrait de toute façon interroger ensuite. La boucle unique rend `timeout` et `poll_interval` réglables côté appelant. |
| Laisser `pytest.skip` sans `CRIBLE_BIN` | C'est le saut silencieux que `docs/08:43` refuse dans son esprit, et il laisse un test vert qui ne prouve rien. L'échec avec la commande à lancer est la règle du dépôt. |
| Lancer les tests serveur dans le job Python de la CI | Ce job n'a ni Rust ni `target/debug/crible` : il faudrait y ajouter une toolchain et un build complet, alors que le job Rust a déjà le binaire et l'étape. |
| Faire tourner le scénario J6 sur l'échantillon Legifacile en CI | Il demanderait ONNX Runtime et le vrai modèle, donc une sixième exclusion nommée (ADR-0062), et rejoindrait le job `rust-onnx`. La démonstration du jalon est le lieu de ce passage. |
| Un seul transport synchrone, l'asynchrone écrit à part | Deux implémentations divergeraient sur l'encodage des vecteurs et l'analyse des erreurs, et le critère de la fiche ne serait pas tenu de la même façon des deux côtés. |
| Générer le client depuis la spécification OpenAPI | ADR-0111 décide que le client est écrit à la main ; la spécification sert de référence et de contrôle (`docs/phases/P6:134`), pas de générateur. |
| Ne tester que les codes atteignables sans binaire | `unauthorized`, `forbidden` et `invalid_request` exigent un serveur : un test unitaire seul ne prouverait pas que le statut et le `request_id` arrivent jusqu'à l'exception. Les deux preuves sont gardées. |

## Références

- `docs/phases/P6-serveur-et-api.md` : `:5-9` (jalon J6), `:137-147` (fiche `CLIENT-PYTHON-HTTP`), `:149-157` (`TESTER-CHARGE-CONCURRENTE`, `docs/jalons/J6-demo.md`).
- `docs/05-api.md` : `:12` et `:21` (rôles, `?wait=true` et son échéance), `:34-52` (corps d'erreur, tableau des quatorze codes), `:56-100` (routes : index, documents, lots, jobs, recherche, compaction, export), `:393-400` (rapport de patch non persisté, rétention, liste des jobs, arrêt), `:410-416` (limites).
- `docs/07-stack-et-deploiement.md` : `:52` (paquets Python, `httpx`), `:106` (`CRIBLE_BIN` hors configuration), `:143` (CI, job Rust et `uv run pytest`), `:144` (job `rust-onnx` sans les tests locaux).
- `docs/08-invariants-et-tests.md:43` (règle du saut, `#[ignore]` côté Rust) ; `:36-42` (outils de test, `pytest` pour le client HTTP).
- `clients/python/src/crible/errors.py` (`BatchError` seul) ; `clients/python/tests/test_online_batch.py:87` (`pytest.skip` sans `CRIBLE_BIN`) ; `clients/python/pyproject.toml` ; `.github/workflows/ci.yml:51-56` (étape « Test the Python client against the binary »).
- ADR-0062 (aucun test exclu sans décision) ; ADR-0067 (`?wait=true` et son échéance) ; ADR-0098 (`x-request-id`) ; ADR-0099 (arrêt SIGTERM, `/ready`) ; ADR-0101 (corps d'erreur, `details`, `request_id`) ; ADR-0103 (jobs, états, erreur sans `request_id`, `Retry-After`) ; ADR-0109 (champs inconnus, entrées selon le mode) ; ADR-0110 (`targets`) ; ADR-0111 (spécification générée, client écrit à la main).
- Verdicts de `@architecte` du 2026-10-30 sur la fiche `CLIENT-PYTHON-HTTP` : un `ECART` interne à la fiche (les huit méthodes ne suffisent pas au scénario J6), un `ECART` de règle de saut (`docs/08:43` contre `test_online_batch.py:87`) et quinze `NON-COUVERT`.
