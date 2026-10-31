# ADR-0103 — Jobs de l'instance : table, états, rétention, file et `Retry-After`

- **Statut** : Accepté (accepté par anticipation par le propriétaire le 2026-10-29, confirmé par le propriétaire le 2026-10-30 ; complément du 2026-10-29 compris dans cette confirmation)
- **Date** : 2026-10-29
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, tâche FILE-D-ECRITURE-ET-JOBS (phase P6), sur demande de l'orchestrateur et le verdict de `@architecte` du 2026-10-29
- **Remplace** : —

## Contexte

La fiche `FILE-D-ECRITURE-ET-JOBS` (`docs/phases/P6-serveur-et-api.md:54-69`) demande une tâche tokio par index avec une file bornée (`writer_queue`), des jobs « gardés en mémoire et dans une table SQLite `jobs`, au niveau de l'instance », et un `503` quand la file est pleine. Quatre points de `docs/05-api.md` décrivent déjà la forme d'un job : l'identifiant `job_…` (`:248`), les huit `kind` (`:260`), les quatre `state` (`:261`) et les filtres `index` et `state` de la liste (`:67`). Deux points sont conformes et ne demandent aucune décision : la clé `[limits].writer_queue` et son défaut 16 (`docs/07-stack-et-deploiement.md:98`), et `?wait=true` avec `wait_timeout_ms` (ADR-0067, `docs/05:19-20`).

Le reste n'est écrit nulle part, et les verdicts de `@architecte` du 2026-10-29 sur la fiche ont relevé sept manques.

**1. Le support.** `docs/02-architecture.md:58-82` énumère le contenu de `<data>` : `instance.json`, `analytics.sqlite`, `debug.sqlite`, `tmp/`, `indexes/`, `models/` et `exchange/`. Aucun fichier de jobs n'y figure. La fiche dit « une table SQLite `jobs`, au niveau de l'instance », sans nommer de fichier ni de module, et sans dire si cette table partage la base des analytics ou celle d'un index.

**2. Les états au redémarrage.** La fiche exige que « les jobs terminés restent consultables après un redémarrage » (`:65`), et `docs/05:261` donne quatre états, dont deux transitoires (`queued`, `running`). Rien ne dit ce que devient, au démarrage, un job resté `queued` ou `running` parce que le processus est mort au milieu. Un job qui resterait `running` pour toujours mentirait à son client.

**3. La rétention et la liste.** Un fichier de jobs croît indéfiniment si personne ne le purge, et `docs/05:67` ne dit ni la rétention, ni l'ordre de la liste, ni s'il y a une pagination. Il ne dit pas non plus si une clé voit les jobs des autres clés.

**4. Le sens de « 16 jobs ».** `docs/05-api.md`, avant application du présent ADR, écrivait « 16 jobs, puis `503` avec `Retry-After` » dans son tableau des limites, et `docs/07:98` déclare `writer_queue = 16`. Le job en cours compte-t-il dans ces 16, ou seulement ceux qui attendent ? De la réponse dépend le débit maximal réel : 16 ou 17 jobs admis par index.

**5. Le `Retry-After` de la file pleine.** `docs/05-api.md`, avant application du présent ADR, nommait l'en-tête sans valeur. ADR-0102 a fixé `Retry-After: 1` pour la limite de recherches concurrentes, et a laissé explicitement ouverte « la valeur de `Retry-After` pour les autres causes d'`overloaded` (file d'écriture, verrou SQLite d'ADR-0038, lexical d'ADR-0089), inchangées » (`:63`). Une même réponse `503` porterait donc une valeur sur une route et aucune sur une autre.

**6. La visibilité entre clés.** `docs/05` ne dit pas si `GET /v1/jobs` filtre par clé d'API. Une clé `write` d'un producteur verrait alors les jobs d'une autre clé `write`, avec leur `index` et leurs comptes.

**7. L'attente des jobs à l'arrêt.** ADR-0099, « Arrêt », point 5 (`:62`), renvoie la durée d'attente des jobs d'écriture à `FILE-D-ECRITURE-ET-JOBS` et note que `DEMARRER-SERVEUR-AXUM` n'avait aucun job à attendre ; « Ce qui n'est pas décidé ici » (`:66`) le répète. Le présent ADR tranche la table, les états, la rétention et la file ; l'arrêt est l'objet d'ADR-0104.

Ce qui est conforme et n'est pas rouvert : le rôle `write` sur `GET /v1/jobs` et `GET /v1/jobs/{job_id}`, donc `403` pour une clé `read` (ADR-0095, `docs/05:67`) ; la forme du corps d'erreur (ADR-0101) ; l'idempotence d'une attache par `batch_id` (ADR-0055, INV-10).

## Décision

**Décision 1 — support.** Le fichier est **`<data>/jobs.sqlite`**, à la racine de `<data>`, à côté de `instance.json`, `analytics.sqlite` et `debug.sqlite`. Il est ouvert et écrit par le module `crible-store`, qui porte déjà `rusqlite` (ADR-0038) : aucune dépendance nouvelle, et aucune arête `crible-server` → `rusqlite`. Les réglages de connexion sont ceux de `records.sqlite`, `busy_timeout` d'ADR-0038 compris. Une table `meta` porte `format_version = 1` ; une version inconnue fait **refuser le démarrage** (INV-15). Une seule connexion sert les écrivains de tous les index.

**Décision 2 — schéma.** Colonnes : `id`, `kind`, `index`, `state`, `created_at`, `started_at`, `finished_at`, `generation`, `progress`, `result`, `error`. `progress`, `result` et `error` sont du texte JSON. `job_id` vaut `job_` suivi d'un UUID v4 en 32 caractères hexadécimaux minuscules ; la crate `uuid` est déjà déclarée avec la feature `v4` (ADR-0096). Les horodatages suivent le format et l'horloge de `batches.applied_at` : RFC 3339 en UTC.

**Décision 3 — quand la table est écrite.** À chaque **transition d'état** seulement : création (`queued`), début (`running`), fin (`succeeded` ou `failed`), chaque écriture portant la progression de cet instant. La progression fine (`progress.step`, `done`, `total`) ne vit qu'en **mémoire** entre deux transitions. Le résultat n'est persisté que s'il l'est déjà par ailleurs : le rapport de `patch_documents` reste en mémoire seulement (ADR-0060), et les durées de l'attache n'entrent jamais dans le job (ADR-0042).

**Décision 4 — forme de `error`.** `{ "code", "message", "details" }`, avec les règles du corps d'erreur d'ADR-0101 : message générique et fixe pour `internal`, cause journalisée avec le `job_id`, `details` à `{}` sauf clé documentée. Le champ `request_id` est **absent** : un job n'est pas une requête.

**Décision 5 — états et redémarrage.** Les seules transitions sont `queued → running → succeeded | failed`. Au démarrage, **avant l'ouverture des files**, tout job resté `queued` ou `running` dans la table passe à `failed`, avec `finished_at` = l'heure du démarrage, `error.code = "internal"` et le message fixe « the job was interrupted by a server restart ». Aucun job n'est rejoué : le client resoumet, et une attache est idempotente par `batch_id` (ADR-0055, INV-10). La cohérence des données d'un index après une coupure reste celle de la reprise (`docs/02-architecture.md:138-148`, INV-02, INV-03).

**Décision 6 — rétention.** Les jobs terminés depuis plus de **30 jours** sont purgés, de la table et de la mémoire, au démarrage et à la fin de chaque job. Les jobs en cours ne sont jamais purgés. La valeur est **fixe, sans clé de configuration** (`AGENTS.md`, règle 12).

**Décision 7 — liste et détail.** `GET /v1/jobs` rend tous les jobs retenus, filtrables par `index` et par `state` (`docs/05:67`), triés par `created_at` décroissant puis `id`. Pas de pagination en v1. Une valeur de `state` hors des quatre états est un `400 invalid_request`. **Aucune isolation par clé** : toute clé `write` ou `admin` voit tous les jobs de l'instance. `GET /v1/jobs/{job_id}` sur un identifiant inconnu ou purgé rend `404 not_found`.

**Décision 8 — file.** Une tâche tokio et une file par index. `[limits].writer_queue` (16) compte les jobs **en attente** ; le job en cours n'en fait pas partie. Les jobs d'un même index s'exécutent dans l'ordre d'arrivée, un à la fois, et le calcul tourne hors des threads tokio (`docs/02-architecture.md:167`).

**Décision 9 — file pleine et `Retry-After`.** Une file pleine rend `503 overloaded`, avec un message qui nomme l'index et la file pleine. **Toute réponse `overloaded` porte `Retry-After: 1`**, quel que soit son producteur : limite de recherches d'ADR-0102, file d'écriture du présent ADR, arrêt en cours d'ADR-0104, verrou SQLite d'ADR-0038, branche lexicale d'ADR-0089. C'est une **seule règle, posée au rendu de l'erreur**, et non une valeur répétée par appelant. Le `503` de `/ready` (ADR-0099) n'est pas un `overloaded` : il ne porte pas l'en-tête.

Ce qui n'est décidé ici : la durée d'attente des jobs à l'arrêt, fixée par ADR-0104 ; le libellé exact des messages autres que ceux cités ; la forme d'une pagination, qui n'existe pas en v1 ; la purge des jobs d'un index supprimé, qui suit la rétention commune.

## Conséquences

- **Positives** :
  - un job survit au redémarrage dans un état terminal honnête, et un client qui relit `GET /v1/jobs/{job_id}` ne voit jamais un `running` éternel ;
  - la table est au niveau de l'instance, donc `delete_index` n'efface pas l'historique des jobs et un index recréé garde la trace de ses attaques passées ;
  - aucune dépendance nouvelle et aucune arête `crible-server` → `rusqlite` : la règle des couches de `docs/07` reste vraie ;
  - trois écritures par job au plus, donc un coût d'écriture négligeable devant l'attache elle-même ;
  - `Retry-After: 1` est écrit une fois, dans le rendu de l'erreur : un nouveau producteur d'`overloaded` en hérite sans décision ;
  - la rétention tient la taille du fichier sans clé de configuration de plus.
- **Négatives / coûts** :
  - une progression fine perdue à chaque coupure : un job interrompu ne dit pas où il s'est arrêté ;
  - 30 jours est une valeur unique : un projet qui veut garder un an d'historique ne peut pas le régler sans un ADR ;
  - aucune pagination : une instance qui accumule des milliers de jobs rend une liste de plusieurs mégaoctets ;
  - toute clé `write` voit les jobs des autres clés, avec leur `index` et leurs comptes — un choix assumé, qui repose sur la confiance entre producteurs d'une même instance ;
  - `Retry-After: 1` est une valeur fixe, sans lien avec la durée réelle d'attente, pour toutes les causes d'`overloaded`.
- **À surveiller** :
  - la taille de `jobs.sqlite` et le nombre de lignes vivantes : si la purge de 30 jours ne suffit pas sur une instance à forte ingestion, la rétention est à rouvrir par un ADR ;
  - un test qui tue le processus au milieu d'un job et vérifie, au démarrage suivant, l'état `failed` et le message fixe ;
  - un test qui remplit la file et vérifie le `503 overloaded`, le message qui nomme l'index et la présence de `Retry-After: 1` ;
  - un test qui vérifie que `Retry-After: 1` apparaît aussi sur un `overloaded` produit par le verrou SQLite et par la branche lexicale, donc que la règle est bien au rendu ;
  - l'incohérence possible avec `docs/05`, qui nommait `Retry-After` sans valeur : la ligne est mise à jour par le présent ADR.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Rejouer les jobs interrompus au démarrage | Un job d'écriture rejoué sans son client peut écrire ce que celui-ci a déjà abandonné ; l'attache est idempotente, mais un enchaînement de plusieurs jobs ne l'est pas. Le client resoumet, et il sait ce qu'il veut. |
| Une clé `[jobs].retention_days` | Ajoute une clé à une liste fermée (ADR-0097) pour une valeur qu'aucun besoin mesuré ne demande de changer ; la règle 12 des `AGENTS.md` interdit la configuration d'une valeur fixe. |
| Compter le job en cours dans la file | Rendrait le débit maximal dépendant de la durée du job en cours : une attache longue bloquerait une soumission alors que 15 places sont libres. Les 16 comptent les jobs en attente. |
| Paginer `GET /v1/jobs` en v1 | Aucun besoin écrit dans `docs/05`, aucune clé de tri stable négociée, et une rétention de 30 jours borne déjà la liste. Une pagination mal spécifiée serait plus coûteuse que son absence. |
| Isoler les jobs par clé d'API | `docs/05` ne donne pas d'identité de job par client, et deux clés `write` d'une même instance collaborent sur les mêmes index ; l'isolation demanderait de stocker la clé ou son nom, ce qui élargit la table et la confidentialité. |
| Une table `jobs` dans le `records.sqlite` d'un index | Les jobs sont ceux de l'instance : exporter, compacter ou supprimer un index effacerait leur historique, et un job `delete_index` disparaîtrait avec sa propre cible. |
| Une base `jobs.sqlite` ouverte par `crible-server` | Faire porter `rusqlite` au serveur ajoute une arête `crible-server` → `rusqlite` que `docs/07` évite, et duplique les réglages de connexion d'ADR-0038. |
| Écrire la progression à chaque étape dans la table | Une attache de plusieurs centaines de milliers de passages écrirait des milliers de lignes pour une information que le job rend en mémoire ; le coût n'a pas de contrepartie, un job interrompu étant de toute façon `failed`. |
| `Retry-After` fixé par chaque producteur d'`overloaded` | Cinq endroits à tenir d'accord, et la valeur diverge à la première évolution ; une seule règle au rendu garantit la cohérence. |
| `Retry-After` calculé d'après la charge observée | Demande une mesure de la durée d'attente réelle par cause, que rien ne fournit aujourd'hui ; la constante est honnête et révisable. |
| Pas de `Retry-After` sur la file d'écriture | Un client réessaierait immédiatement et aggraverait la saturation ; ADR-0102 a déjà tranché le principe pour la recherche. |

## Complément du 2026-10-29

Une précision sur l'enveloppe du corps des routes de liste. Elle ne change ni le statut ni les décisions ci-dessus.

**Constat.** `docs/05-api.md` ne fixe pas l'enveloppe du corps d'une route de liste ; `@dev` a rendu `GET /v1/jobs` sous la forme `{ "jobs": [ … ] }`.

**Décision.** Le corps d'une route de liste est un objet JSON à une seule clé, le nom pluriel de la ressource, dont la valeur est le tableau : `{ "jobs": [ … ] }` pour `GET /v1/jobs`, et la même règle pour `GET /v1/indexes` (`{ "indexes": [ … ] }`, livrée par `ROUTES-INDEX`).

**Raison.** Un objet laisse ajouter un champ (pagination éventuelle, compte) sans casser les clients ; un tableau nu ne le permet pas.

## Références

`docs/02-architecture.md` (« Organisation sur disque » `:56-82`, « Reprise au démarrage » `:138-148`, « Concurrence » `:158-167`) ; `docs/05-api.md` (conventions `?wait=true` `:19-20`, table des erreurs `:35-50`, endpoints `:67`, section « Jobs » `:245-266`, « Limites par défaut » `:268-282`) ; `docs/07-stack-et-deploiement.md` (`[limits]` `:97-102`) ; `docs/phases/P6-serveur-et-api.md` (`FILE-D-ECRITURE-ET-JOBS` `:54-69`) ; `docs/08-invariants-et-tests.md` (INV-01, INV-02, INV-03, INV-10, INV-15) ; ADR-0016 (point de commit) ; ADR-0038 (`busy_timeout`, `overloaded`) ; ADR-0042 (durées d'attache non persistées) ; ADR-0055 (idempotence par `batch_id`) ; ADR-0060 (rapport de patch non persisté) ; ADR-0067 (`?wait=true`) ; ADR-0095 (rôle `write` et critère d'arrêt) ; ADR-0096 (crate `uuid`) ; ADR-0097 (liste fermée des clés) ; ADR-0099 (arrêt) ; ADR-0101 (corps d'erreur) ; ADR-0102 (`Retry-After` de la recherche, laissé ouvert ailleurs) ; ADR-0104 (arrêt et jobs).
