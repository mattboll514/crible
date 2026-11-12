# ADR-0150 — Refus `index_deleting` à la recréation d'un nom en cours de suppression, et ordre des étapes de `delete_index`

- **Statut** : Accepté (accepté par anticipation par le propriétaire le 2026-11-11, confirmé par le propriétaire le 2026-11-12)
- **Date** : 2026-11-11 (proposition et acceptation anticipée)
- **Décideurs** : propriétaire
- **Proposé par** : agent `@greffier`, à la demande de l'orchestrateur du 2026-11-11, pour la tâche `T03-RECREATION-APRES-SUPPRESSION` de la v1.1, sur le verdict de `@architecte` du 2026-11-11 et sur le ticket 03 du projet Legifacile (`tickets-crible/03-suppression-index-asynchrone.md`, lecture seule)
- **Remplace** : — (complète ADR-0105, décisions 2 et 6, sans modifier son corps)
- **Décision** : confirmée par le propriétaire le 2026-11-12, sa réponse : « les adr sont accepté ».

## Contexte

### Le constat du ticket 03

`DELETE /v1/indexes/{index}?confirm={index}` rend `202` avec un job `delete_index` mis dans la file de l'index (ADR-0105, décision 5). Le nom n'est libéré qu'à l'exécution de ce job. Tant qu'il est **en file ou en cours**, un `POST /v1/indexes` du même nom rend `400 invalid_request` avec le message « index already exists: '<nom>' » (ADR-0105, décision 2). Ce refus ne porte ni `job_id` ni discriminant : le client ne sait pas s'il doit attendre ou si le nom est réellement occupé.

Le cas est une course. Sur un écrivain au repos, la suppression d'un index vide peut finir avant la création suivante et rendre `201` ; avec un job devant la suppression, le `400` est reproductible. Le ticket mesure un délai de libération de 2 s sur un index vide, et cite le contournement de deux harnais de mesure : suppression, attente du job, puis sondage de `GET /v1/indexes/{name}` jusqu'au `404`. C'est un coût de diagnostic : l'échec ressemble à un bug du client, pas à une suppression encore en cours.

Pistes écartées au stade du ticket, avant cet ADR :

- un `409` à code dédié : ADR-0105 a déjà écarté un code nouveau pour un nom pris (alternatives, `docs/adr/0105-administration-des-index-par-l-api.md:177`), et changer le statut d'un cas **documenté** est interdit en mineure par ADR-0143, décision 1 (`docs/adr/0143-politique-de-version-de-la-ligne-1-x-et-procedure-de-montee-et-de-retour-arriere.md:56`) ;
- le retrait du registre dès le `DELETE` : ADR-0105 l'a écarté (alternatives, `:185`) ;
- une attente par défaut du `DELETE` : cela changerait un cas documenté (ADR-0105, décision 5 ; ADR-0143, décision 1), et bloquerait l'appelant plusieurs minutes sur un index occupé par une attache longue (ADR-0105, conséquences, `:149`).

### Ce que la conception et les ADR fixent déjà

- **`details` est fermé.** Il vaut `{}` par défaut ; une clé n'y figure que si `docs/05` la documente pour ce code, et une tâche qui ajoute une clé la documente **dans le même changement** que le code (ADR-0101, décision 3 ; `docs/05-api.md:35`).
- **`reason` est un vocabulaire fermé.** Il dit pourquoi la requête est refusée ; `path` localise le champ fautif et n'est publié, pour un `invalid_request`, **qu'avec** une `reason` (ADR-0145, décisions 2 et 3). Aujourd'hui, une seule valeur existe : `lexical_branch_disabled`.
- **La classe de compatibilité est fixée.** Une **clé de `details` nouvelle** est un ajout permis dans une mineure (ADR-0143, décision 1, `:56`), et l'ADR de chaque ticket de contrat déclare sa classe (décision 7, `:120`). `docs/suivi.md:164` classe `T03-RECREATION-APRES-SUPPRESSION` **ajout compatible**, et ADR-0143 nomme explicitement ce ticket : « Sa variante `job_id` dans le refus actuel reste possible : c'est une clé de `details` nouvelle sur un refus existant » (`:64`).
- **Les jobs ne sont pas persistés autrement que par leur table.** `jobs.sqlite` porte l'état de chaque job, `queued → running → succeeded | failed` ; au démarrage, tout job `queued` ou `running` passe à `failed` avec un message fixe, sans rejeu (ADR-0103, décision 5). Il n'existe aucun autre état persistant d'un job.
- **`?wait=true` existe déjà sur `DELETE`.** Il est honoré jusqu'à `wait_timeout_ms`, puis rend `202` dans l'état courant, jamais `504` (ADR-0067, ADR-0105 décision 5). C'est un moyen d'attendre, mais pas le défaut : faire attendre le `DELETE` par défaut changerait un cas documenté.
- **`GET /v1/jobs?index=<nom>&state=<état>` existe** et suffit à suivre un job de suppression (ADR-0103, décision 7).

### Ce qui est vérifié dans l'arbre le 2026-11-11

- **Le refus de création** est prononcé par `create_index` : contrôle du registre puis du dossier, deux fois le même message (`crates/crible-server/src/state.rs:866-879`). La création ne lit jamais la table des jobs.
- **Le rendu des erreurs** ne publie `details` que pour `invalid_filter` (`path` seul) et pour un `invalid_request` qui porte une `reason` (`path` et `reason`) (`crates/crible-server/src/error.rs:157-179`). La `reason` est un type fermé, `ErrorReason`, avec un seul variant (`crates/crible-core/src/error.rs:101-121`).
- **La table des jobs en mémoire** expose `unfinished()`, qui rend les jobs dont l'état n'est pas terminal (`crates/crible-server/src/jobs.rs:630-636`), et une vue `JobView` qui porte `id`, `kind`, `index` et `state` (`:58-71`).
- **L'ordre réel de `delete_index`** est : renommage du dossier en quarantaine, libération du verrou d'écrivain, **puis** retrait du registre, marquage de l'écrivain et purge (`crates/crible-server/src/indexes.rs:441-466`, commentaire de la ligne 441 compris). Le test `a_detail_read_between_the_rename_and_the_deregistration_is_a_404` reproduit l'état intermédiaire et l'affirme (`crates/crible-server/tests/routes_indexes.rs:657-687`).
- **`docs/suivi.md:111`** décrit le même ordre : « renommage en `<data>/tmp/deleted-*` puis désinscription ».
- **`docs/05-api.md:140`** est **incohérent avec lui-même** : sa phrase principale place le retrait du registre avant le renommage, comme ADR-0105, alors que sa dernière phrase (« accepté après le renommage ») décrit l'ordre du code. Cette ligne est à corriger (tableau d'application ci-dessous).

### L'écart à trancher

ADR-0105, décision 6, numérote le **retrait du registre** comme étape 1 et le **renommage** comme étape 2 (`docs/adr/0105-administration-des-index-par-l-api.md:69-76`). Le code, le test et `docs/suivi.md:111` font l'inverse : renommage d'abord, désinscription ensuite. Le document de conception qui fait foi, `docs/05`, ne tranche pas, puisqu'il se contredit. Cet ADR fixe l'ordre qui fait foi, sans réécrire ADR-0105 : une mention sous sa décision 6 renvoie ici.

## Décision

### 1. Refus `invalid_request` avec `details = {"reason": "index_deleting", "job_id": "job_…"}`

Quand le nom demandé est tenu par un job `delete_index` **non terminal** — en file ou en cours —, `POST /v1/indexes` rend le refus **existant** :

```json
{"error": {"code": "invalid_request",
           "message": "index already exists: 'decisions'",
           "details": { "reason": "index_deleting", "job_id": "job_7f3c…" },
           "request_id": "…"}}
```

Le code (`invalid_request`, 400) et le message ne changent pas : c'est le corps du ticket 03 qui est complété, pas remplacé. Aucune clé de `details` n'est retirée.

`reason = "index_deleting"` est une **valeur nouvelle** d'un vocabulaire fermé, sur le modèle d'ADR-0145 : une valeur par cas documenté dans `docs/05`, et une valeur nouvelle demande son ADR. `job_id` vient de `JobView::id` (ADR-0103) et se rend tel quel : c'est un identifiant **public**, jamais un iid (**INV-01**).

**Pas de `path`.** Aucun champ du corps n'est fautif : la configuration envoyée peut être parfaitement valide, c'est l'état de l'instance qui refuse. `path` est un chemin JSON de requête (`$.mode`, `$.filter.<champ>` par exemple), et la règle de rendu d'ADR-0145, décision 3, ne l'impose pas : elle publie `path` **quand l'erreur en porte un**, et cette erreur n'en a pas. Publier `path: "$.name"` nommerait un champ qui n'est pas en faute, et `$.name` est déjà le chemin d'autres refus de création (nom refusé par `path_component`, clé inconnue), ce qui rendrait le chemin non discriminant, exactement le défaut que le ticket 15 a fait corriger par ADR-0145. `reason` et `job_id` suffisent.

La règle de rendu reste fermée, comme pour le couple `path`/`reason` : `index_deleting` n'est **jamais** publié sans `job_id`, et un `job_id` n'est jamais publié sans `index_deleting`. Côté code, `ErrorReason` reçoit le variant, et le rendu refuse de produire l'un sans l'autre.

### 2. `details` vide dans tous les autres cas

Deux situations gardent le corps d'aujourd'hui, `details: {}` :

- le nom est tenu par un **index vivant** du registre, et aucun job `delete_index` de ce nom n'est en file ni en cours ;
- un **dossier** existe sur disque hors registre, cas d'un index orphelin ou d'une création interrompue.

Un `details` non vide est donc le discriminant : sa présence dit « attends ce job puis réessaie », son absence dit « le nom est réellement occupé ». Aucun code, aucun statut et aucun message ne change ; un client 1.0 qui ignore `details` lit le même corps qu'hier.

### 3. Source du renseignement : la table des jobs en mémoire, aucun état persisté

Le contrôle lit `unfinished()` (`crates/crible-server/src/jobs.rs:630-636`) et filtre sur `kind == delete_index` et `index == <nom>`. Il n'ajoute **aucun état persistant**, **aucun marqueur** sur disque, **aucune clé de configuration** et **aucun `format_version`** : la table des jobs existe déjà, et un état nouveau aurait demandé une migration.

Règles qui en découlent :

1. **Le contrôle précède celui du registre et du dossier.** Un `delete_index` accepté engage la suppression : même si l'index est encore servi en attendant son tour, la création du même nom rend `index_deleting`, ce qui est l'information utile. Sur un `delete_index` en cours, il couvre aussi la fenêtre entre le renommage et la désinscription (décision 4), où le registre tient encore le nom alors que le dossier a disparu.
2. **La course est fermée du côté de la libération.** Le nom se libère à la **désinscription**, qui précède l'état terminal du job (décision 4) : tant qu'un job `delete_index` n'est pas terminal, aucune création ne peut donc réussir, et dès qu'il est terminal, plus aucun refus `index_deleting` n'est possible.
3. **Après la fin du job, la création est acceptée** : `succeeded` et `failed` sont terminaux, et un job `failed` sur un renommage raté laisse l'index vivant, donc un refus `details: {}` correct.
4. **La course est fermée du côté de la soumission.** Un `DELETE` exige que l'index soit au registre, donc `404 unknown_index` avant la fin de la création : aucun `delete_index` ne peut apparaître entre le contrôle et la création. Un refus `index_deleting` devenu périmé (le job s'est terminé juste après le contrôle) se corrige par un nouvel essai, qui réussit. Le sens inverse n'existe pas.
5. **Après un redémarrage, aucun `job_id` périmé n'est présenté** : tout job `queued` ou `running` passe à `failed` au démarrage (ADR-0103, décision 5), et `failed` est terminal. Un job `succeeded` d'avant l'arrêt est terminal aussi. Le seul cas restant, un arrêt brutal avant le renommage, laisse l'index intact et enregistré : le refus `details: {}` y est vrai.
6. **Si plusieurs `delete_index` de ce nom ne sont pas terminaux**, le plus ancien est nommé — `created_at` croissant, puis `id`, l'ordre d'exécution des jobs d'un index (ADR-0103, décision 7) : c'est celui qui libérera le nom.

Le coût est une lecture de la table des jobs en mémoire à chaque `POST /v1/indexes`, sous le verrou de création déjà pris : pas d'entrée-sortie, une comparaison par job retenu, et la rétention des jobs est bornée à 30 jours (ADR-0103). Le renseignement n'est journalisé nulle part : `details` ne l'est pas (ADR-0101, décision 4).

### 4. Ordre des étapes de `delete_index` : le renommage d'abord, la désinscription ensuite

L'ordre qui fait foi est celui du code, déjà testé et documenté dans `docs/suivi.md:111` :

1. le dossier `<data>/indexes/<nom>` est **renommé** en `<data>/tmp/deleted-<nom>-<job_id>` (`crates/crible-server/src/indexes.rs:438-450`) ;
2. le verrou d'écrivain de l'index est **libéré** (ADR-0116, décision 2) ;
3. l'index est **retiré du registre** : les nouvelles requêtes reçoivent `404 unknown_index`, les requêtes en vol finissent sur l'instantané qu'elles tiennent (**INV-02**) ;
4. l'écrivain est marqué supprimé, la quarantaine est purgée ;
5. les jobs restés derrière échouent en `unknown_index`, et l'historique des jobs reste dans `jobs.sqlite` (ADR-0103).

La numérotation d'ADR-0105, décision 6, qui met le retrait du registre avant le renommage, est donc **précisée** par cet ADR : son corps n'est pas réécrit, une mention sous la décision 6 renvoie ici.

**Raison** : le renommage est la seule étape qui puisse échouer pour une raison de système de fichiers, et il est réversible dans son effet sur le service. Placé en premier, son échec laisse l'index **entier, enregistré, servi et verrouillé** : le job passe à `failed`, et rien n'est à moitié supprimé. Placé en second, son échec laisserait un index absent du registre et toujours sur disque, donc invisible et non servi jusqu'au redémarrage suivant, ce qu'ADR-0105 voulait précisément éviter (alternatives, `:185`).

**Conséquences de cet ordre** :

- entre le renommage et la désinscription, le registre tient encore le nom et le dossier a disparu : `GET /v1/indexes/{index}` rend `404 unknown_index`, jamais `500` (test `a_detail_read_between_the_rename_and_the_deregistration_is_a_404`) ;
- dans cette même fenêtre, un `POST` du même nom rend `index_deleting` avec le `job_id` du job en cours (décision 1), et non un « already exists » muet ;
- une panne **avant** le renommage laisse l'index intact et enregistré, le job passant à `failed` au redémarrage (ADR-0103) ; une panne **après** laisse l'index supprimé et le dossier en quarantaine, que le démarrage purge avec les `online-*` (ADR-0056). Cette lecture d'ADR-0105, décision 6, reste vraie ; seul l'ordre des deux premières étapes change.

### 5. Hors périmètre : pas d'état `deleting` dans `GET /v1/indexes/{index}`

Exposer un état « en suppression » sur le détail d'un index serait un ajout compatible, mais il est **reporté** : `GET /v1/jobs?index=<nom>&state=<état>` donne déjà le `job_id`, le `kind` et l'état, et le refus de la décision 1 porte désormais ce `job_id`. Une seconde représentation du même fait serait à tenir. Le ticket 03 le proposait comme piste ; il reste noté dans le suivi.

### 6. Classe de compatibilité et invariants

**Ajout compatible**, au sens d'ADR-0143, décision 1, déclaré conformément à sa décision 7 : une clé de `details` nouvelle sur un refus existant, sans changement de code, de statut, de message, de route, ni de format. La même classe figure dans `docs/suivi.md:164`. L'ordre des étapes de la décision 4 n'est pas un changement de contrat : `docs/05` n'est pas modifié sur ce point de comportement, seulement **corrigé** pour dire ce que le code fait depuis ROUTES-INDEX.

Invariants :

- **INV-01** : `job_id` est un identifiant public (ADR-0103), `details` ne porte ni iid, ni chemin du serveur, ni texte de requête ;
- **INV-02** : le contrôle lit une table en mémoire ; il ne touche ni l'instantané, ni les segments, ni le dossier, et ne bloque aucune lecture ;
- **INV-04** : au redémarrage, aucun état nouveau n'est à réconcilier ; les jobs non terminaux passent à `failed` (ADR-0103, décision 5), et la purge des `deleted-*` reste celle d'ADR-0056.

### 7. Application : documents, spécification, code et tests

| Emplacement | Modification |
|---|---|
| `docs/05-api.md:35` | règle de `details` : ajouter `reason` et `job_id` pour le refus `index_deleting`, et dire que `index_deleting` et `job_id` ne vont jamais l'un sans l'autre |
| `docs/05-api.md:39` | ligne `invalid_request` : cas « `POST /v1/indexes` dont le nom est tenu par un `delete_index` non terminal », message inchangé, `details = {"reason": "index_deleting", "job_id": "job_…"}` |
| `docs/05-api.md:92` | paragraphe de `POST /v1/indexes` : le refus garde son message, porte le discriminant quand un `delete_index` du nom n'est pas terminal, et garde `details: {}` pour un index vivant ou un dossier hors registre |
| `docs/05-api.md:140` | paragraphe du `DELETE` : ordre corrigé en « renomme le dossier, puis retire l'index du registre », et dernière phrase remplacée : un `POST` du même nom rend `index_deleting` jusqu'à la fin du job, puis est accepté |
| `crates/crible-core/src/error.rs:101-121` | `ErrorReason` : variant `IndexDeleting`, valeur `index_deleting` |
| `crates/crible-server/src/error.rs:157-179` | rendu : `index_deleting` sort avec son `job_id`, jamais seul ; le reste inchangé |
| `crates/crible-server/src/state.rs:866-879` | `create_index` : contrôle du `delete_index` non terminal avant le registre et le dossier, refus porteur de la `reason` et du `job_id` |
| `crates/crible-server/src/jobs.rs:630-636` | accès au job `delete_index` non terminal d'un index (`unfinished()` filtré, ou fonction voisine) |
| `crates/crible-server/src/openapi.rs:70-90` | description de `ErrorBody.details` : citer `reason` et `job_id` pour ce refus |
| Note de version de la 1.1.0 (tâche `PUBLIER-V1.1.0`, ADR-0143, décision 4) | lister `details.reason = "index_deleting"` et `details.job_id` |
| `docs/adr/README.md` | ligne d'index de cet ADR, et mention sur la ligne d'ADR-0105 |
| `docs/adr/0105-administration-des-index-par-l-api.md` | une mention sous la décision 6 : « ordre des étapes 1 et 2 précisé par ADR-0150 le 2026-11-11 », sans réécriture du corps |

Tests attendus :

- `delete_index` retenu derrière un job : `POST /v1/indexes` du même nom rend `400`, code et message inchangés, `details == {"reason": "index_deleting", "job_id": "<id du job>"}` ;
- `delete_index` en cours, entre le renommage et la désinscription : même refus, même `job_id` ;
- index vivant sans `delete_index` en file : `400`, `details == {}` ;
- dossier présent sur disque hors registre : `400`, `details == {}` ;
- après la fin du job : `201`, et le corps est celui de `GET /v1/indexes/{index}` (ADR-0105, décision 2) ;
- après un redémarrage qui a marqué les jobs en cours `failed` : aucun `details.job_id` n'est présenté ; la création rend `201` quand le dossier est absent, et `details: {}` quand l'arrêt a eu lieu avant le renommage et que l'index vit encore ;
- aucun corps ne contient d'iid ni de chemin absolu (**INV-01**).

Le client Python n'a **aucun** changement : il expose `details` tel quel (ADR-0112), et un script qui attend peut lire `details.job_id` puis `GET /v1/jobs/{job_id}`.

## Conséquences

- **Positives** :
  - un client sait **s'il doit attendre** : un `details` non vide porte la raison et le job à suivre, un `details` vide dit que le nom est réellement occupé ;
  - le discriminant est une valeur fermée, pas une phrase anglaise : une reformulation du message ne casse aucun client, comme pour ADR-0145 ;
  - aucun état nouveau, aucun fichier, aucune migration : la table des jobs existait déjà, et rien n'est à réconcilier après un arrêt brutal ;
  - la course est fermée des deux côtés : rien ne se crée tant qu'un `delete_index` n'est pas terminal, et un `DELETE` ne peut pas apparaître pendant une création ;
  - l'ordre des étapes de `delete_index` devient un fait unique, écrit une fois, avec l'état intermédiaire prouvé par un test ;
  - les deux harnais de mesure de Legifacile peuvent remplacer leur sondage jusqu'au `404` par la lecture du `job_id`.
- **Négatives / coûts** :
  - une valeur de plus dans le vocabulaire fermé de `reason`, et un `details` qui n'est plus vide sur un refus `invalid_request` déjà connu : un client qui compare `details` à `{}` doit lire les clés ;
  - `create_index` dépend de la table des jobs : la création n'est plus une fonction pure du registre et du disque, et un test de création doit disposer d'une table de jobs ouverte ;
  - le rendu doit coupler `reason` et `job_id`, ce qui ajoute un cas de plus dans le rendu des erreurs, avec sa règle testée ;
  - un `delete_index` **en file** derrière une attache longue fait désormais répondre `index_deleting` là où le corps disait seulement « already exists » : l'information est plus juste, mais l'attente annoncée peut durer des minutes ;
  - un job `delete_index` qui échoue au renommage laisse l'index vivant : le client qui a lu `index_deleting` doit relire, et la création rend alors `details: {}` ;
  - `docs/05:140` doit être corrigé, donc un document de conception se met à jour dans le changement de code, comme le veut ADR-0101, décision 3.
- **À surveiller** :
  - le coût du contrôle : si la liste des jobs retenus croît (rétention portée au-delà de 30 jours, ADR-0103), une lecture par création pourrait se voir ; le signe serait une latence de `POST /v1/indexes` qui suit le nombre de jobs ;
  - une valeur de `reason` ajoutée sans documentation dans `docs/05` ni ADR : le signe serait un client qui découvre une valeur absente de la table ;
  - un `details` non vide sur un refus qui n'est pas `index_deleting` : le signe serait un client qui attend un job qui ne libérera pas le nom ;
  - l'usage réel de `details.job_id` par Legifacile : le ticket 03 est clos quand ses deux harnais n'ont plus de boucle de sondage ;
  - l'ordre des étapes : un futur remaniement de `delete_index` qui remettrait la désinscription en tête doit rouvrir cet ADR, parce que la panne ne laisserait plus le même état.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| `409` avec un code nouveau (`index_deleting` ou `index_exists`) | Déjà écarté par ADR-0105 (alternatives, `:177`), et interdit en 1.1 : changer le statut ou le code d'un cas documenté est une rupture (ADR-0143, décision 1, `:56`). Il faudrait une ligne de table, un variant d'`ErrorCode` et une version de client synchronisée, pour une information qui tient dans `details`. |
| Retirer l'index du registre dès le `DELETE` | Écarté par ADR-0105 (alternatives, `:185`) : le client verrait `404` sur un index qu'un job efface encore, et deux `DELETE` concurrents se disputeraient le dossier. |
| Faire attendre le `DELETE` par défaut, ou attendre la libération du nom | Change un cas documenté (ADR-0105, décision 5 ; ADR-0143, décision 1) et rend `202` une réponse qui n'arrive qu'à l'échéance : sur un index occupé par une attache, l'appelant resterait bloqué plusieurs minutes alors qu'il peut faire autre chose. `?wait=true` couvre le besoin de qui veut attendre. |
| Un état `deleting` dans `GET /v1/indexes/{index}` | Reporté (décision 5) : `GET /v1/jobs?index=&state=` couvre le besoin, et une seconde représentation du même fait serait à tenir ; la fenêtre est courte et le `job_id` du refus suffit à la traverser. |
| Un marqueur persisté dans le registre ou sur disque | Ajoute un état à réconcilier au démarrage, donc une décision sur INV-04 et, si le marqueur vit dans un fichier versionné, un `format_version`. La table des jobs porte déjà l'information, sans écriture nouvelle. |
| Rendre le `job_id` seul, sans `reason` | Un `details` non vide serait alors le seul discriminant, et un futur refus porterait aussi une clé sans son vocabulaire fermé : c'est exactement le `details` « seconde API » qu'ADR-0101, décision 3, et ADR-0145 écartent. |
| Ajouter `path: "$.name"` | Aucun champ du corps n'est fautif ; `$.name` désigne déjà d'autres fautes de création (nom refusé, clé inconnue), donc le chemin ne discrimine rien et contredit la règle d'ADR-0145, décision 3. |
| Modifier le message, par exemple « index is being deleted: '<nom>' » | Les clients lisent ce message depuis la 1.0, dont le MCP de Legifacile ; une prose reformulable n'est pas un discriminant, et deux messages à tenir pour un seul cas coûtent une ligne de table et une note de version. |
| Garder l'ordre d'ADR-0105 (désinscription puis renommage) | Cet ordre est celui qu'aucun test n'exerce et qui rend l'index invisible avant d'être effacé : un échec du renommage laisserait un index sur disque hors du registre, non servi, jusqu'au redémarrage suivant. Le code, le test et `docs/suivi.md:111` font déjà l'inverse ; cet ADR aligne le texte sur le comportement éprouvé plutôt que de réécrire le comportement. |

## Références

- Ticket : `tickets-crible/03-suppression-index-asynchrone.md` (projet Legifacile, lecture seule) — constat, reproduction, impact et contournements.
- Conception : `docs/05-api.md:35` (règle de `details`), `:39` (ligne `invalid_request`), `:66` (tableau des routes), `:90-92` (administration des index, `POST /v1/indexes`), `:140` (`DELETE` et ordre des étapes) ; `docs/suivi.md:111` (ordre livré par ROUTES-INDEX), `:164` (tâche et classe), `:632` (classement des tickets de la 1.1).
- Code : `crates/crible-server/src/state.rs:866-879` (`create_index`), `crates/crible-server/src/indexes.rs:410-478` (`delete_index_inner`, ordre des étapes), `crates/crible-server/src/jobs.rs:58-71` (`JobView`), `:630-636` (`unfinished`), `crates/crible-server/src/error.rs:157-179` (rendu de `details`), `crates/crible-core/src/error.rs:101-121` (`ErrorReason`), `crates/crible-server/src/openapi.rs:70-90` (`ErrorBody.details`).
- Tests : `crates/crible-server/tests/routes_indexes.rs:657-687` (`a_detail_read_between_the_rename_and_the_deregistration_is_a_404`), et les tests de suppression et de panne de quarantaine du même fichier.
- Mesures et rapports : `docs/mesures/MES-02-encodage-quotidien.md:366-367` (suppression asynchrone constatée).
- ADR : ADR-0056 (purge des `online-*` et `deleted-*` au démarrage), ADR-0067 (`?wait=true` et `wait_timeout_ms`), ADR-0101 (corps d'erreur, `details` fermé, décisions 3 et 4), ADR-0103 (jobs, états, règle 5 du redémarrage, décision 7), ADR-0105 (décisions 2, 5 et 6, et alternatives), ADR-0112 (client Python, `details` rendu tel quel), ADR-0116 (verrou d'écrivain, décision 2), ADR-0143 (décisions 1, 4 et 7), ADR-0145 (`reason` fermé, `path` seulement avec une `reason`), ADR-0146 (routes pendant le chargement : le contrôle du job se fait derrière la garde `loaded`, et le `503` « the server is starting » la précède).
