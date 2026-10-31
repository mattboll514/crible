# ADR-0105 — Administration des index par l'API : création à chaud, `PATCH`, suppression et corps des réponses

- **Statut** : Accepté (accepté par anticipation par le propriétaire le 2026-10-29, confirmé par le propriétaire le 2026-10-30 ; complément du 2026-10-29 compris dans cette confirmation)
- **Date** : 2026-10-29
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, tâche ROUTES-INDEX (phase P6), sur demande de l'orchestrateur et les verdicts de `@architecte` du 2026-10-29
- **Remplace** : —

## Contexte

`docs/05-api.md` donne les cinq routes d'administration des index en une ligne chacune (`:59-63`), mais ces lignes décrivent l'intention, pas le contrat. Il y a un `201` sans corps, un `PATCH` sans corps ni moment d'application, un `DELETE` sans protocole, et des « comptes » qui ne sont définis nulle part. Or `ROUTES-INDEX` doit écrire ces routes, et le reste de P6 s'appuie dessus : `ROUTES-DOCUMENTS-ET-LOTS` et `ROUTE-RECHERCHE` en dépendent (`docs/phases/P6-serveur-et-api.md:86,101`).

### Ce que la conception fixe déjà

- **Réglages figés et modifiables** : la table `docs/04-donnees-et-ingestion.md:44-55` sépare ce qui est figé à la création (`name`, `model`, `reference_precision`, type et niveau des champs) de ce qui se règle ensuite (`search.*`, `lexical.*`, `cluster_by`, `filterable`, `returned`, ajout d'un champ, `ingest.*`, `compaction.*`). Un réglage figé renvoie `409 immutable_setting` (**INV-16**) ; une clé inconnue renvoie `400` à tous les niveaux (ADR-0031).
- **Mutations asynchrones** : `202` avec un objet `job`, `?wait=true` honoré jusqu'à `wait_timeout_ms` puis `202` dans l'état courant, jamais `504` (`docs/05:20-21`, ADR-0067).
- **Jobs** : table `jobs.sqlite`, états `queued → running → succeeded | failed`, liste close des `kind` (`docs/05:298`), forme de `error`, rétention, file d'écriture par index (ADR-0103). Toute réponse `overloaded` porte `Retry-After: 1`, par une règle posée au rendu.
- **Rôles** : `POST`, `PATCH`, `DELETE` d'administration sont `admin`, les deux `GET` sont `read` (`docs/05:11-13`) ; un rôle insuffisant rend `403` sans `WWW-Authenticate` (ADR-0100), et chaque famille de routes prouve son `403` dans la tâche qui la crée (ADR-0095).
- **Démarrage** : ADR-0099 décrit un serveur qui ouvre `<data>/indexes/*` en écriture **une fois**, au démarrage, et charge une session par empreinte de modèle. La création d'un index **hors démarrage** n'y est pas décrite ; les modèles non plus.
- **Lecteurs** : `docs/02-architecture.md:165` — un instantané est libéré quand son dernier lecteur se termine, et les fichiers d'une époque supprimée ne sont effacés qu'après. **INV-02** interdit qu'une lecture bloque ou voie un état mixte.
- **Interdits d'API** : aucun iid dans une réponse (**INV-01**) ; `SegmentMeta` porte des bornes d'iid.

### Ce que la conception laisse ouvert

1. **Index créé à chaud.** Le registre d'index n'est décrit qu'au démarrage (ADR-0099) ; `state.rs` ne recharge jamais ses index en P6. Rien ne dit comment un index créé par l'API devient servible.
2. **Modèle d'un index créé hors démarrage.** ADR-0099 ne parle que des modèles des index ouverts au démarrage.
3. **Nom déjà pris.** Aucun des quatorze codes de `docs/05:36-51` ne dit « existe déjà ». La CLI rend `invalid_request` pour un nom pris (`crible-store`, `generation.rs:612-616`).
4. **Corps du `201`** (`docs/05:59`).
5. **Forme et moment d'application du `PATCH`** (`docs/05:62`) : aucun corps, aucun `kind` de configuration dans la liste close (`docs/05:298`), alors que `docs/05:20` fait de toute mutation un `202` + job.
6. **« Comptes », « tailles » et « dernier lot »** (`docs/05:60-61`), sans définition.
7. **Protocole de suppression** (`docs/05:63`) : aucun n'est écrit, alors que la suppression d'un index touche des requêtes en vol (`docs/02:165`) et des jobs acceptés (ADR-0103).

## Décision

**Décision 1 — registre d'index mutable.** Le serveur tient un **registre** des index ouverts, modifiable à chaud. `POST /v1/indexes` crée un index et le sert **sans redémarrage** ; le job `delete_index` l'en retire. Une seule création à la fois par instance, sous un verrou d'instance : deux créations concurrentes ne peuvent pas écrire le même nom ni charger deux fois le même modèle. Les lectures ne sont jamais bloquées par une création.

**Décision 2 — création synchrone, `201`.** `POST /v1/indexes` valide la configuration complète par **le même chemin que la CLI**, sans second validateur. Le nom est contrôlé par `path_component`. Un nom déjà présent, **dans le registre ou sur disque**, rend `400 invalid_request` avec le message « index already exists » : c'est le code que la CLI rend déjà, et l'API n'introduit aucun code. Le modèle est lu sous `[paths].models` (`model.json`) **avant toute écriture** : un modèle absent ou illisible rend `400 invalid_request`, et le message nomme le modèle. Si aucune session de cette empreinte n'est chargée, elle est chargée à la création, **hors des threads tokio** (`docs/02:171`), une session par empreinte (ADR-0099), et elle n'est plus déchargée avant le redémarrage. L'ordre est ensuite : écriture sur disque, ouverture, publication d'un instantané vide, démarrage de l'écrivain de l'index, inscription au registre. Un échec après le début de l'écriture retire le dossier créé (même quarantaine qu'à la décision 6) : un nom n'est jamais laissé à moitié créé. `/ready` n'est pas affecté : ses trois conditions sont celles d'ADR-0099, et un index créé ensuite est déjà prêt quand il est visible. Le **corps du `201` est celui de `GET /v1/indexes/{index}`**.

**Décision 3 — corps des deux `GET`.** `GET /v1/indexes` rend l'enveloppe d'ADR-0103 :

```json
{ "indexes": [ { "name": "…", "model": "…", "generation": 1288, "documents": 412000, "live_passages": 8580000 } ] }
```

triés par nom. `GET /v1/indexes/{index}` rend la configuration et l'état :

```json
{ "name": "…",
  "config": { "…": "index.json matérialisé, réglages figés et réglables" },
  "state": { "epoch": 4, "generation": 1288, "documents": 412000, "live_passages": 8580000,
             "dead_passages": 12400, "dead_ratio": 0.00144, "segments": 37,
             "sizes": { "disk_bytes": 21474836480, "filter_memory_bytes": 41943040, "codes_memory_bytes": 1287000000 },
             "last_batch": { "batch_id": "…", "applied_at": "…", "generation": 1288 } } }
```

Les comptes sont ceux de l'`IndexState` de la **génération publiée**, donc ceux de l'instantané courant et non d'une lecture de SQLite. `segments` est un **compte**, jamais la liste des `SegmentMeta`, qui portent des bornes d'iid (**INV-01**). `sizes.disk_bytes` est la taille du dossier de l'index sur disque, `sizes.filter_memory_bytes` et `sizes.codes_memory_bytes` les estimations mémoire de l'état. `last_batch` est la **dernière ligne de `batches`** (`batch_id`, `applied_at`, `generation`), et vaut `null` quand aucun lot n'a été attaché.

**Décision 4 — `PATCH` : validation synchrone, application par job.** La validation a lieu **avant toute mise en file**, contre la configuration courante : clé inconnue → `400` (ADR-0031), réglage figé → `409 immutable_setting` nommant le champ (**INV-16**), valeur hors domaine → `400`. Le changement est ensuite exécuté par **l'écrivain de l'index**, dans l'ordre d'arrivée de sa file, comme un job : `202` avec le job, `?wait=true` honoré (ADR-0067).

Le `kind` du job est :

- `rebuild_lexical` quand le job reconstruit l'index lexical — c'est-à-dire quand, après le patch, `lexical.enabled` vaut `true` et l'une de `lexical.enabled`, `lexical.language` ou `lexical.positions` a changé ;
- sinon **`patch_config`**, un `kind` nouveau, ajouté à la liste close de `docs/05:298`.

Le job écrit `index.json` de façon atomique, reconstruit ce qui en dérive — l'index lexical, et les structures de filtre pour `filterable`, `returned` et l'ajout d'un champ (`docs/04`) — puis publie **ensemble** la nouvelle configuration et le nouvel instantané : une requête voit l'ancienne configuration avec l'ancien instantané, ou la nouvelle avec le nouveau, jamais un mélange. `search.*` vaut pour les requêtes qui **suivent la fin du job** ; `ingest.*`, `compaction.*` et `cluster_by` valent pour le prochain job ou la prochaine compaction. Passer `lexical.enabled` à `false` ne reconstruit rien — les recherches `lexical` et `hybrid` sont refusées (ADR-0089) — et le repasser à `true` reconstruit. Le résultat du job vaut `null`.

**Décision 5 — `DELETE` avec `confirm`.** `DELETE /v1/indexes/{index}?confirm={index}` : un `confirm` absent ou différent du nom rend `400 invalid_request`, et le message **nomme le paramètre `confirm`**. Sinon, la réponse est `202` avec un job `delete_index`, **mis dans la file de l'index** : les jobs acceptés avant lui se terminent d'abord.

**Décision 6 — exécution de `delete_index`.** Quatre étapes, dans cet ordre :

1. l'index est **retiré du registre** : les nouvelles requêtes reçoivent `404 unknown_index`, et les requêtes en vol finissent sur l'instantané qu'elles tiennent (`docs/02:165`) ;
2. le dossier `<data>/indexes/<nom>` est **renommé** en `<data>/tmp/deleted-<nom>-<job_id>`, dans le même système de fichiers, puis supprimé ;
3. les jobs restés **derrière** lui dans la file échouent en `unknown_index` ;
4. l'historique des jobs de l'index **reste** dans `jobs.sqlite` (ADR-0103, `docs/02:82`).

Une panne **avant** le renommage laisse l'index intact : le job passe à `failed` au redémarrage (ADR-0103). Une panne **après** le renommage laisse l'index supprimé, et le démarrage purge les `deleted-*` de `<data>/tmp`, comme les `online-*` (ADR-0056). Un `POST` du même nom est refusé tant que l'index est au registre, et accepté après le renommage.

**Décision 7 — rôles.** `POST`, `PATCH` et `DELETE` sont réservés à `admin` ; les deux `GET` sont ouverts à `read` (ADR-0095). Rien de nouveau : ces rôles sont ceux de `docs/05:11-13`.

Ce qui n'est pas décidé ici : la forme exacte de `config` au-delà de « `index.json` matérialisé », qui suit `docs/04` ; le déchargement d'une session de modèle devenue inutile, qui n'existe pas en P6 ; la conduite à tenir si le renommage échoue pour une raison de système de fichiers, qui rend le job `failed` avec l'index intact ; le protocole de suppression d'une **époque** (compaction, instantané), qui appartient à ses tâches.

## Complément du 2026-10-29 — ordre des contrôles d'un `PATCH` sur le niveau d'un champ

- **Statut** : inchangé — Accepté (accepté par anticipation par le propriétaire le 2026-10-29, confirmé par le propriétaire le 2026-10-30). Les décisions 1 à 7 ci-dessus restent inchangées ; ce complément lève une ambiguïté entre deux règles déjà écrites.
- **Origine** : relecture de `ROUTES-INDEX` et rapport de `@testeur`, même jour.

### Contexte du complément

Un `PATCH` qui déclare un champ **existant** à l'autre niveau que le sien — par exemple `passage_fields.numero` alors que `numero` est un champ de document — relève de deux règles à la fois :

- la table `docs/04-donnees-et-ingestion.md:44-55` range « type et niveau des champs existants » parmi les réglages figés : changer le niveau d'un champ doit donc rendre `409 immutable_setting`, qui nomme le champ (**INV-16**) ;
- `docs/04-donnees-et-ingestion.md:42` et ADR-0030 refusent qu'un même nom soit déclaré à la fois dans `document_fields` et dans `passage_fields` : la déclaration doit donc rendre `400 invalid_request`.

Aucun texte ne disait laquelle des deux règles l'emporte. Deux implémentations également conformes pouvaient donc rendre deux codes différents pour la même requête, et `@testeur` ne pouvait pas écrire un test unique.

### Décision du complément

1. **Dans un `PATCH`, le contrôle des réglages figés passe avant la règle de collision d'ADR-0030.** Déclarer un champ existant à l'autre niveau est un **changement de niveau** : la requête est refusée en `409 immutable_setting`, et le message nomme le chemin du champ (par exemple `passage_fields.numero`).
2. **La règle d'ADR-0030 reste celle des cas où elle s'applique seule** : la **création** d'un index, et l'**ajout par `PATCH`** d'un champ **nouveau** déclaré aux deux niveaux dans le même corps. Ces deux cas rendent `400 invalid_request`, et le message nomme le champ et les deux niveaux.
3. **ADR-0030 n'est pas modifié** : il est accepté, et ce complément ne fait que fixer l'ordre des contrôles dans le `PATCH`.
4. **Le contrôle est celui de la décision 4**, sans mécanisme nouveau : validation synchrone, avant toute mise en file ; seuls l'ordre et le code rendu sont précisés.

### Raison

La table de `docs/04` nomme explicitement le **niveau des champs existants** parmi les réglages figés : le cas est donc un changement de niveau avant d'être une collision de noms. Le `409` porte en outre une information que le `400` ne porte pas : le client apprend qu'il doit **recréer l'index**, puisque le niveau d'un champ ne se change pas à chaud.

### Conséquences du complément

- **Positives** :
  - un seul code pour une même requête, donc un test unique et une implémentation non ambiguë dans `ROUTES-INDEX` ;
  - le client reçoit l'action à faire — recréer l'index — au lieu d'un refus de forme qui laisse croire qu'un autre corps serait accepté ;
  - l'ordre des contrôles est écrit dans un ADR, donc il ne dépend plus de l'ordre des branches d'un validateur.
- **Négatives / coûts** :
  - deux codes voisins pour des déclarations proches : un client qui envoie un champ existant au mauvais niveau reçoit `409`, alors qu'il pourrait s'attendre à `400` ;
  - l'ordre des contrôles devient un comportement observable, donc une régression possible : il devra être testé, pas seulement implémenté.
- **À surveiller** :
  - un test de `PATCH` déclarant un champ de document dans `passage_fields` : `409 immutable_setting`, message nommant `passage_fields.<champ>` ;
  - un test de `PATCH` ajoutant un champ **nouveau** aux deux niveaux dans le même corps : `400 invalid_request` ;
  - un test de création d'index déclarant un même nom aux deux niveaux : `400 invalid_request` ;
  - un test qui vérifie que le `409` cite le chemin du champ et non le seul nom du champ.
- **Documents mis à jour par ce complément** :

| Document | Modification |
|---|---|
| `docs/04-donnees-et-ingestion.md:42` | la règle de collision d'ADR-0030 s'applique à la création et à l'ajout d'un champ **nouveau** ; déclarer un champ existant à l'autre niveau rend `409 immutable_setting` |
| `docs/adr/README.md:161` | ligne d'index de cet ADR : ajout du complément du 2026-10-29 |

### Alternative écartée pour ce complément

| Alternative | Pourquoi écartée |
|---|---|
| Appliquer d'abord la règle de collision d'ADR-0030, donc `400 invalid_request` | Le `400` dit que le corps est invalide, pas que le réglage est figé : le client réessaierait avec un autre corps, alors qu'aucun `PATCH` ne peut changer le niveau d'un champ existant. La table de `docs/04` range ce cas parmi les réglages figés. |

## Conséquences

- **Positives** :
  - l'administration d'une instance ne demande plus un redémarrage par index : créer, régler et supprimer sont des opérations d'API, avec un job suivi comme les autres ;
  - un seul validateur de configuration sert la CLI et l'API, donc une configuration acceptée par l'une est acceptée par l'autre ;
  - le `201` rendant le corps du `GET` évite une seconde représentation à tenir : le client reçoit l'état de ce qu'il vient de créer ;
  - le `PATCH` hérite de tout ce qu'ADR-0103 pose déjà — ordre de la file, `?wait=true`, forme de `error`, rétention, `Retry-After` — sans mécanisme nouveau ;
  - configuration et instantané sont publiés ensemble : **INV-03** tient pour un réglage qui change le comportement d'une requête ;
  - la quarantaine de suppression fait qu'une panne laisse l'index entier ou supprimé, jamais à moitié, et le renommage laisse les requêtes en vol finir (**INV-02**) ;
  - l'historique des jobs survit à la suppression de l'index : on peut encore dire qui l'a supprimé et quand.
- **Négatives / coûts** :
  - `state.rs` doit porter un registre mutable et un verrou d'instance : la création devient une section critique, et une création lente (chargement de session, écriture disque) retarde les suivantes ;
  - une session chargée à la création n'est **jamais** déchargée avant le redémarrage : une instance qui crée des index de modèles différents garde leur mémoire résidente, et un modèle créé puis supprimé coûte encore son RSS ;
  - `patch_config` porte la liste close des `kind` à neuf entrées : tout client qui aiguille sur `kind` doit la connaître, et un job `queued` ou `running` au moment du `202` ne dit pas encore si le réglage est en vigueur ;
  - `sizes.disk_bytes` demande de parcourir le dossier de l'index à chaque `GET /v1/indexes/{index}` : le coût croît avec le nombre de segments, et la route de liste ne le paie pas — l'écart doit être assumé et mesuré ;
  - un `DELETE` attend les jobs acceptés avant lui : sur un index occupé par une attache longue, l'index reste visible plusieurs minutes après le `202` ;
  - « index already exists » réutilise `invalid_request`, déjà le code le plus chargé de `docs/05`, et le client ne distingue ce cas que par le message.
- **À surveiller** :
  - un test qui crée un index, cherche dedans et l'attache **sans redémarrer** ;
  - un test de nom déjà pris, dans le registre puis sur disque, qui vérifie `400 invalid_request` et le message ;
  - un test d'échec de création (modèle absent, écriture impossible) qui vérifie qu'aucun dossier ne reste dans `<data>/indexes` ;
  - un test de suppression avec une requête en vol, qui la voit finir sur son instantané, et un `POST` du même nom refusé avant le renommage puis accepté après ;
  - un test de panne entre le renommage et la suppression : le démarrage purge `deleted-*`, l'index n'est plus au registre, le job est `failed` ;
  - un test de `PATCH lexical.language` qui vérifie le job `rebuild_lexical`, un test de `PATCH search.*` qui vérifie `patch_config`, et un test qui lit la configuration et l'instantané avant et pendant le job sans voir d'état mixte ;
  - le coût du parcours de `disk_bytes` sur un index de 8,58 M passages et de plusieurs dizaines de segments : s'il dépasse quelques millisecondes, la valeur sera portée par l'état plutôt que recalculée ;
  - le nombre de modèles chargés à chaud sur une instance réelle : si l'exploitation en crée plusieurs, la règle « jamais déchargée » devra être revue.
- **Documents mis à jour par cet ADR** :

| Document | Modification |
|---|---|
| `docs/05-api.md:59-63` | corps du `201`, `PATCH` en `202` avec un job, `DELETE` avec `confirm`, renvois à ADR-0105 |
| `docs/05-api.md:81-116` | nouvelle section « Administration des index » : corps des deux `GET`, exemple, `kind` du `PATCH`, protocole de suppression |
| `docs/05-api.md:298` | ajouter `patch_config` à la liste close des `kind` |
| `docs/02-architecture.md:65` | ajouter `<data>/tmp/deleted-<index>-<job_id>/` à l'organisation sur disque |
| `docs/02-architecture.md:145` | purger les `deleted-*` de `<data>/tmp` au démarrage |
| `docs/02-architecture.md:164` | bullet « registre d'index mutable » dans « Concurrence » |
| `docs/04-donnees-et-ingestion.md:55` | une phrase : un `PATCH` accepté est exécuté par l'écrivain comme un job, la valeur ne change qu'à la fin du job |
| `docs/adr/README.md:161` | ligne d'index de cet ADR |

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| `409` avec un code nouveau (`index_exists`) pour un nom pris | Un code de plus dans une liste close, pour un cas que la CLI rend déjà en `invalid_request` ; le client n'a rien à faire de différent. |
| Créer l'index par un job, en `202` | La création ne fait qu'écrire un dossier et publier un instantané vide : c'est court, et le client veut le nom disponible tout de suite. Un job déplacerait la question « le nom est-il libre ? » après la réponse. |
| `PATCH` synchrone qui écrit `index.json` hors de l'écrivain | Course avec une attache en cours, et attente derrière un job long sous le verrou du magasin : le patch d'un réglage prendrait la durée d'une attache. |
| Un job `rebuild_lexical` séparé, mis en file après le patch | Ouvre une fenêtre où une attache indexerait avec la nouvelle chaîne un index construit avec l'ancienne, donc un index lexical mixte, que rien ne détecterait avant ADR-0106. |
| Supprimer en place, sans quarantaine | Une panne au milieu laisse un index partiellement effacé, que rien ne distingue d'un index sain : la reprise le servirait. |
| Exposer la liste des segments dans `GET /v1/indexes/{index}` | `SegmentMeta` porte `iid_start` et `count`, donc des bornes d'iid : **INV-01** interdit de les faire sortir. Un compte suffit à l'exploitation. |
| Rendre le `201` avec un corps vide, ou avec le seul nom | Le client devrait appeler `GET` pour connaître la génération et les comptes qu'il vient de créer ; deux représentations de la même chose sont à maintenir. |
| Un index créé à chaud n'est servible qu'après redémarrage | C'est l'état de `state.rs` en P6, et il rendrait `POST /v1/indexes` inutile : la route existe pour administrer une instance vivante. |
| Répondre `202` au `DELETE` après avoir déjà retiré l'index du registre | Le client verrait `404` sur un index qu'un job est encore en train d'effacer, et deux `DELETE` concurrents se disputeraient le dossier. |

## Références

- `docs/phases/P6-serveur-et-api.md:71-82` — fiche `ROUTES-INDEX`, et `:86`, `:101` pour les tâches qui en dépendent.
- `docs/05-api.md:11-13` (rôles), `:19-21` (mutations asynchrones, `?wait=true`), `:29-51` (corps d'erreur et codes), `:57-63` (routes d'index), `:81-116` (administration des index), `:283-305` (jobs, liste close des `kind`), `:306-318` (limites).
- `docs/02-architecture.md:55-94` (organisation sur disque), `:140-159` (reprise au démarrage), `:161-172` (concurrence, lecteurs, calcul hors tokio).
- `docs/04-donnees-et-ingestion.md:38-72` (champs, ce qui est figé, ce qui est réglable, domaines).
- `docs/08-invariants-et-tests.md` — INV-01, INV-02, INV-03, INV-16.
- `crates/crible-store/src/generation.rs:612-616` — nom déjà pris rendu `invalid_request`.
- ADR-0031 (clé inconnue), ADR-0067 (`?wait=true`), ADR-0089 (branche lexicale désactivée), ADR-0095 (403 par famille), ADR-0097 (liste fermée des clés), ADR-0099 (démarrage, `/ready`, une session par empreinte), ADR-0100 (`401`, `403`), ADR-0101 (corps d'erreur), ADR-0103 (jobs, enveloppe de liste, `Retry-After`), ADR-0104 (arrêt et jobs), ADR-0106 (chaîne d'analyse enregistrée).
