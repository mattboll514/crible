# P6 — Serveur et API

**But** : exposer tout le cœur derrière l'API REST `/v1`. Cette phase couvre l'authentification, le format d'erreur, les limites, un écrivain par index avec ses jobs, la recherche simple et multi-index, la doc OpenAPI et le client Python.

**Jalon J6** :
- le scénario J6 passe en entier, via Swagger UI et via le client Python : créer les index de l'échantillon, attacher les lots, chercher en simple et en multi-index, patcher, compacter, exporter ;
- INV-02 est vert sous charge ;
- INV-16 est vert ;
- la spécification OpenAPI correspond à `05-api.md`.

---

### DEMARRER-SERVEUR-AXUM
- **Objectif** : le squelette du serveur.
- **Dépend de** : —
- **À lire** : `07-stack-et-deploiement.md` (config, Docker : arrêt propre) ; `05-api.md` (conventions).
- **Livrables** :
  - `crible-server` : lecture de `crible.toml` avec surcharges par variables d'environnement (forme, refus et liste des variables exclues : ADR-0097) ;
  - au démarrage, chargement des modèles et des index, avec la reprise (ADR-0099) ;
  - routes `/health` et `/ready` (formes et sens : ADR-0099) ;
  - `x-request-id` sur chaque réponse (reprise ou génération : ADR-0098) ;
  - logs `tracing` en JSON (`RUST_LOG`, défaut `info` : ADR-0096) ;
  - arrêt propre sur SIGTERM et SIGINT (drainage borné : ADR-0099) ;
  - commande `crible serve` (ADR-0097).
- **Critères d'acceptation** :
  - `/ready` rend `503` tant que tous les index n'ont pas publié leur instantané, puis `200` (ADR-0099) ;
  - un test d'arrêt montre qu'une requête en vol se termine avant la sortie (drainage borné, ADR-0099). Le critère « aucun job en cours n'est perdu à l'arrêt » passe à FILE-D-ECRITURE-ET-JOBS (ADR-0095).

### AUTHENTIFIER-CLES
- **Objectif** : les clés d'API et les rôles.
- **Dépend de** : DEMARRER-SERVEUR-AXUM
- **À lire** : `05-api.md` (conventions, rôles) ; `07-stack-et-deploiement.md` (config).
- **Livrables** :
  - un middleware `Bearer` : la clé reçue est hachée en SHA-256 et comparée en temps constant aux hachés de la config ;
  - les rôles `read`, `write` et `admin` ;
  - le nom de la clé attaché à la requête (pour le « client » des analytics) ;
  - la commande `crible key generate`, qui affiche la clé et son haché.
- **Critères d'acceptation** :
  - `401` sans clé ou avec une clé inconnue ;
  - le middleware et les trois rôles (`read`, `write`, `admin`) sont testés sur un routeur de test : chaque rôle atteint les routes de son niveau, et une clé d'un rôle inférieur reçoit `403 forbidden` sur une route d'un rôle supérieur ; le test `403` de chaque famille de routes réelles est porté par la tâche qui la crée (ADR-0095) ;
  - la clé en clair n'apparaît jamais dans les logs.

### FORMAT-ERREURS-ET-LIMITES
- **Objectif** : des erreurs uniformes et des limites appliquées.
- **Dépend de** : DEMARRER-SERVEUR-AXUM
- **À lire** : `05-api.md` (erreurs, limites).
- **Livrables** :
  - la conversion de `CribleError` vers le JSON d'erreur avec le bon statut HTTP ;
  - une limite de taille de corps ;
  - une limite de recherches concurrentes (`503 overloaded`, avec `Retry-After`) ;
  - un `timeout_ms` par requête (`504`).
- **Critères d'acceptation** : un test par code d'erreur atteignable.

### FILE-D-ECRITURE-ET-JOBS
- **Objectif** : un écrivain par index, une file bornée et le suivi des jobs.
- **Dépend de** : DEMARRER-SERVEUR-AXUM
- **À lire** : `02-architecture.md` (concurrence) ; `05-api.md` (jobs, `?wait=true`) ; ADR-0067.
- **Livrables** :
  - une tâche tokio par index, avec une file bornée (`writer_queue`) ;
  - les jobs (états, progression, résultat, erreur), gardés en mémoire et dans une table SQLite `jobs`, au niveau de l'instance ;
  - `?wait=true` ;
  - `503` quand la file est pleine.
- **Critères d'acceptation** :
  - les jobs d'un même index s'exécutent dans l'ordre d'arrivée ;
  - les jobs terminés restent consultables après un redémarrage ;
  - un test d'arrêt montre qu'aucun job en cours n'est perdu : les jobs acceptés avant le signal se terminent, et l'attente bornée de l'arrêt les couvre (repris de DEMARRER-SERVEUR-AXUM, ADR-0095, avec le drainage d'ADR-0099) ;
  - une clé `read` reçoit `403` sur `GET /v1/jobs` et `GET /v1/jobs/{job_id}`, réservés au rôle `write` (ADR-0095) ;
  - à l'échéance de `wait_timeout_ms`, `?wait=true` rend `202` avec le job en cours ; un job terminé avant est rendu terminé ; un test pour chaque cas (ADR-0067) ;
  - le test de file pleine renvoie bien `503`.

### ROUTES-INDEX
- **Objectif** : l'administration des index.
- **Dépend de** : FILE-D-ECRITURE-ET-JOBS, AUTHENTIFIER-CLES
- **Livrables** :
  - `POST`, `GET`, `PATCH` et `DELETE` sur `/v1/indexes[/…]` ;
  - l'état d'un index : génération, comptes, ratio de passages morts, tailles.
- **Critères d'acceptation** :
  - **INV-16** : un `PATCH` sur un réglage figé renvoie `409` en nommant le champ ;
  - `DELETE` exige `confirm` ;
  - un `PATCH` de `lexical.language` déclenche le job `rebuild_lexical` ;
  - une clé `write` reçoit `403` sur une route `admin` de `/v1/indexes`, et les deux `GET` restent accessibles avec une clé `read` (ADR-0095).
- **Invariants** : INV-16.

### ROUTES-DOCUMENTS-ET-LOTS
- **Objectif** : écrire par l'API.
- **Dépend de** : ROUTES-INDEX, ENCODER-DOCUMENTS-EN-LIGNE, PATCHER-METADONNEES
- **Livrables** :
  - `POST` et `PATCH` sur `/documents` ;
  - `GET /documents/{doc_id}` ;
  - `POST /batches`, avec un chemin limité à `exchange/inbox` : refus de `..` et des liens symboliques qui sortent du dossier ;
  - `POST /compact`, `/exports` et `/snapshots`.
- **Critères d'acceptation** :
  - un chemin de lot hors de l'inbox est refusé (test de sécurité) ;
  - chaque route a un test d'intégration ;
  - une clé `read` reçoit `403` sur les familles `write` (documents, lots) et `admin` (compaction, export, instantanés) écrites ici (ADR-0095) ;
  - les réponses ne contiennent aucun iid (**INV-01**).
- **Invariants** : INV-01.

### ROUTE-RECHERCHE
- **Objectif** : `POST /v1/indexes/{index}/search`.
- **Dépend de** : ROUTES-INDEX, FUSIONNER-HYBRIDE
- **À lire** : `05-api.md` (requête, réponse).
- **Livrables** :
  - tous les champs de la requête, sauf `debug` et `explain_ids` (P7), acceptés et validés ;
  - la réponse conforme au doc, avec `generation`, `pool_size`, `took_ms` et `truncated_query` ;
  - les calculs exécutés sur le pool rayon, jamais sur les threads tokio.
- **Critères d'acceptation** :
  - les résultats de l'API sont identiques à ceux de la CLI sur les requêtes de l'échantillon ;
  - les trois rôles atteignent la route : aucun `403` n'y est atteignable, puisque `write` et `admin` incluent `read` (ADR-0095).
  - `timeout_ms` est lu dans le corps et vaut `[limits].default_timeout_ms` par défaut ; une recherche qui dépasse son échéance rend `504 timeout` et garde son jeton de concurrence jusqu'à la fin réelle du calcul (ADR-0102) ;
  - au-delà de `[server].max_concurrent_searches` recherches simultanées, la route rend `503 overloaded` avec `Retry-After: 1` ; un test pour chaque cas (ADR-0102, mécanisme livré par FORMAT-ERREURS-ET-LIMITES).

### RECHERCHE-MULTI-INDEX
- **Objectif** : `POST /v1/search`.
- **Dépend de** : ROUTE-RECHERCHE
- **À lire** : `03-recherche.md` (recherche multi-index).
- **Livrables** :
  - un seul encodage de la requête ;
  - un filtre par cible ;
  - le refus d'un mélange de modèles (`incompatible_model`) ;
  - la fusion par cosinus en dense, par RRF en lexical et en hybride ;
  - le champ `index` dans chaque résultat.
- **Critères d'acceptation** :
  - en dense, la fusion donne le même résultat qu'un tri global des top-k par index ;
  - l'encodage n'est appelé qu'une fois (test) ;
  - les trois rôles atteignent la route : aucun `403` n'y est atteignable (ADR-0095) ;
  - au-delà de `[server].max_concurrent_searches` recherches simultanées, la route rend `503 overloaded` avec `Retry-After: 1` (même limite que `ROUTE-RECHERCHE`, mécanisme livré par FORMAT-ERREURS-ET-LIMITES, ADR-0102).

### PUBLIER-OPENAPI
- **Objectif** : une spécification et une UI à jour.
- **Dépend de** : ROUTE-RECHERCHE, ROUTES-DOCUMENTS-ET-LOTS
- **Livrables** : des annotations `utoipa` sur toutes les routes et tous les types, `/openapi.json` et `/docs` (Swagger UI).
- **Critères d'acceptation** :
  - un test compare la liste des routes de la spécification à celle de `05-api.md` ;
  - les exemples du doc sont valides contre le schéma.

### CLIENT-PYTHON-HTTP
- **Objectif** : le client utilisé par le MCP de Legifacile.
- **Dépend de** : PUBLIER-OPENAPI
- **Livrables** :
  - `crible.Client(base_url, api_key)` : `search`, `search_multi`, `get_document`, `attach_batch`, `upsert_documents`, `patch_documents`, `job`, `wait` ;
  - des erreurs typées (`CribleError` avec `code`) ;
  - une version asynchrone (`httpx.AsyncClient`) ;
  - des tests contre un serveur lancé dans la fixture pytest.
- **Critères d'acceptation** :
  - le scénario J6 se joue entièrement en Python ;
  - chaque code d'erreur devient une exception typée.

### TESTER-CHARGE-CONCURRENTE
- **Objectif** : prouver INV-02 dans des conditions réelles.
- **Dépend de** : ROUTE-RECHERCHE, ROUTES-DOCUMENTS-ET-LOTS
- **Livrables** : un script de charge (N clients de recherche) qui tourne pendant des attaches, puis pendant une compaction ; un rapport avec les percentiles.
- **Critères d'acceptation** :
  - le p99 de recherche pendant les écritures reste sous 3 fois le p99 au repos ;
  - aucune erreur `5xx` autre que les `503` attendus ;
  - `docs/jalons/J6-demo.md` est écrit.
- **Précisions** : ADR-0113 (critère d'INV-02 par requête, MES-16, protocole, `503` attendus : aucun).
- **Invariants** : INV-02.

### CORRIGER-OPERATIONID-OPENAPI
- **Origine** : ajoutée par le propriétaire le 2026-10-31, hors plan initial (« oui lance la correction »). Elle vient du rejeu de l'étape 4 de `docs/jalons/J6-demo.md` (Swagger UI) le 2026-10-31. Dans Swagger UI, `POST /v1/search` ne rend jamais de réponse, et rien n'arrive côté serveur (0 % de CPU), alors que la même requête passée par `curl` rend `200` en 75 ms avec les résultats attendus. Cliquer sur l'un des deux blocs de recherche ouvre les deux. La cause est dans `/openapi.json` : `POST /v1/indexes/{index}/search` et `POST /v1/search` ont le même `operationId` (`search`) et le même tag (`search`), `utoipa` reprenant le nom de la fonction Rust. La spécification OpenAPI 3.1 exige qu'un `operationId` soit unique parmi toutes les opérations ; Swagger UI s'en sert pour identifier les blocs et confond donc les deux opérations. Les 16 autres opérations ont un `operationId` unique, mais ni `docs/05-api.md` ni ADR-0111 ne fixent les noms. Le critère de J6 « le scénario passe en entier via Swagger UI » n'est pas tenu tant que ce défaut reste.
- **Objectif** : un `operationId` unique pour chaque opération de la spécification servie, pour que Swagger UI distingue la recherche simple de la recherche multi-index.
- **Dépend de** : PUBLIER-OPENAPI
- **À lire** : `adr/0111-specification-openapi-et-validation-des-exemples.md` (spécification OpenAPI, Swagger UI embarqué) ; `05-api.md` (tableau des routes) ; `docs/jalons/J6-demo.md` (étape 4).
- **Livrables** :
  - un `operationId` explicite `search_index` pour `POST /v1/indexes/{index}/search`, et `search_multi` pour `POST /v1/search` (noms choisis avec le propriétaire) ;
  - un test sur la spécification générée, qui échoue si deux opérations partagent un `operationId` et qui nomme le doublon.
- **Critères d'acceptation** :
  - le test d'unicité passe, et il échoue si l'on remet le doublon (`@dev` le vérifie une fois avant de livrer) ;
  - les 16 autres `operationId` sont inchangés ;
  - les tests existants de la spécification restent verts (égalité avec les routes de `docs/05`, exemples validés, ADR-0111) ;
  - aucun test n'est désactivé.
- **Invariants** : aucun.
