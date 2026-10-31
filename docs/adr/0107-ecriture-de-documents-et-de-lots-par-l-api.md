# ADR-0107 — Écriture de documents et de lots par l'API

- **Statut** : Accepté (accepté par anticipation par le propriétaire le 2026-10-29, confirmé par le propriétaire le 2026-10-30 ; complément du 2026-10-29 compris dans cette confirmation)
- **Date** : 2026-10-29
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, tâche ROUTES-DOCUMENTS-ET-LOTS (phase P6), sur demande de l'orchestrateur et les verdicts de `@architecte` du 2026-10-29
- **Remplace** : — (complète ADR-0055 et ADR-0042, dont le corps n'est pas réécrit : la route prend le `batch_id` du corps et porte des `deletions`, et le serveur ne rend les durées par aucune réponse)

## Contexte

La fiche `ROUTES-DOCUMENTS-ET-LOTS` (`docs/phases/P6-serveur-et-api.md:84-97`) demande d'écrire par l'API : `POST` et `PATCH` sur `/documents`, `GET /documents/{doc_id}`, `POST /batches` avec un chemin limité à `exchange/inbox`, et `POST /compact`, `/exports` et `/snapshots`. Les routes existent déjà en une ligne chacune dans `docs/05-api.md:64-73`, mais ces lignes décrivent l'intention, pas le contrat. Le reste de P6 en dépend : `PUBLIER-OPENAPI` et `CLIENT-PYTHON-HTTP` suivent cette tâche (`:131,139`), et le client Python doit exposer `get_document`, `attach_batch`, `upsert_documents`, `patch_documents` (`:141`).

### Ce que la conception fixe déjà

- **Lot en ligne** : `docs/05:118-139` donne le corps (`batch_id`, `mode`, `documents`, `deletions`), la règle du `doc_hash` (ADR-0055), la limite de 10 000 passages, le mode `upsert` seulement, et un manifeste interne déterministe. Le lot interne suit le même chemin d'attache qu'un lot déposé (`docs/04:247`).
- **Attache** : la sémantique est celle d'ADR-0041. Un document nommé dans `deletions` reçoit des pierres tombales et perd sa ligne `documents` dans la transaction du commit (décision 1) ; un `doc_id` présent à la fois dans `documents` et dans `deletions` fait rejeter le lot (`docs/04:192`, `docs/format-lot-v1.md:302`, ADR-0040:43).
- **Idempotence** : un `batch_id` déjà appliqué ne fait rien et rend le rapport persisté (INV-10) ; le même `batch_id` avec un manifeste différent rend `409 batch_conflict` (`docs/04:197-200`).
- **Patch de métadonnées** : `docs/05:153-171` et ADR-0060 fixent le corps, la fusion, les champs acceptés, le refus atomique `not_found` d'un document inconnu ou retiré, les limites et le rapport non persisté.
- **Mutations asynchrones** : `202` avec un objet `job`, `?wait=true` honoré jusqu'à `wait_timeout_ms` puis `202` dans l'état courant, jamais `504` (`docs/05:20-21`, ADR-0067).
- **Jobs** : table `jobs.sqlite`, états `queued → running → succeeded | failed`, forme de `error` sans `request_id`, liste close des `kind`, file d'écriture par index (ADR-0103). Toute réponse `overloaded` porte `Retry-After: 1`.
- **Rôles** : `read` couvre la lecture de documents, `write` les documents et les lots, `admin` la compaction, l'export et les instantanés (`docs/05:11-13`, ADR-0095). Un rôle insuffisant rend `403` sans `WWW-Authenticate` (ADR-0100), et chaque famille prouve son `403` dans la tâche qui la crée (ADR-0095).
- **Erreurs** : format unique, `message` jamais porteur d'un iid ni d'un chemin absolu du serveur, `details` à `{}` sauf clé documentée (ADR-0101). Corps borné à 32 Mio (ADR-0102).
- **Instantanés** : `docs/02:167-168` — une requête tient un `Arc<Snapshot>` du début à la fin, et les fichiers d'une époque supprimée ne sont effacés qu'après. `docs/04:318-367` et ADR-0064, ADR-0065 fixent le contenu, les destinations et les refus.
- **Compaction** : ADR-0017 suspend les écritures et laisse les lectures continuer ; `docs/02:189-199` donne les six étapes.

### Ce que la conception laisse ouvert

1. **`deletions` du lot en ligne (ECART).** `docs/05:135` montre `deletions` dans le corps et `docs/05:64` annonce « suppressions ». ADR-0055, « À surveiller » (`:89`), dit qu'un besoin de suppression en ligne demanderait un nouvel ADR, et le chemin d'ingestion en ligne n'a aujourd'hui aucun moyen de suppression (`crates/crible-store/src/online.rs`). Le lot interne ne sait donc pas porter un `deletions.parquet`.
2. **`batch_id` du lot en ligne (ECART apparent).** ADR-0055, « Cohérence avec P6 » (`:67`), et `docs/04:259` disent que la route prend son `batch_id` dans le corps ; `online.rs:626-635` le dérive des octets du contenu. L'écart est dans le code, pas dans les documents : la décision ci-dessous confirme la conception et donne la règle de construction.
3. **Durées de l'attache (NON-COUVERT).** ADR-0042, décision (`:31`), réserve explicitement la forme sous laquelle un appelant reçoit ses durées : « c'est un point d'API, hors du périmètre de cette décision ». `docs/05:328` dit seulement qu'un job relu plus tard n'en donne aucune.
4. **`GET /documents/{doc_id}` (NON-COUVERT).** `docs/05:66` annonce « Document, ses passages et son état », sans corps, sans source de lecture et sans règle pour `?text=true`. Lire l'état SQLite courant plutôt que l'époque de l'instantané publié rendrait des passages d'une génération que l'instantané ne contient pas.
5. **`POST /batches` (NON-COUVERT).** `docs/05:67` nomme `exchange/inbox/<dossier>` sans donner le corps de la requête. La fiche (`:90`) demande le refus de `..` et « des liens symboliques qui sortent du dossier » ; `crible-core` (`crates/crible-core/src/path.rs`) ne refuse que la chaîne vide, `.`, `..` et les séparateurs `/` et `\` : il ne voit aucun lien symbolique, et `path_component` seul ne suffit pas.
6. **Moment de `batch_conflict` (NON-COUVERT).** `docs/05:45` pose le code, mais la mutation est asynchrone : rien ne dit si le conflit est rendu par la soumission ou par le job.
7. **`adopt_epoch` (NON-COUVERT).** `POST /compact` rend la compaction atteignable depuis l'API, alors que le serveur garde le magasin en mémoire entre deux jobs. Un échec après `write_current_epoch` laisse le magasin sur E alors que `CURRENT_EPOCH` nomme E+1, et `GenerationStore::commit` écrirait des générations perdues. Ce point est tranché par **ADR-0108**.

Ce qui est conforme et n'est pas rouvert : le mode `upsert` seulement et le refus de `replace_all` en ligne (ADR-0055, `docs/05:138`) ; la limite de 32 Mio (ADR-0102) ; la forme de `error` (ADR-0101) ; l'absence d'iid dans une réponse (**INV-01**) ; l'attache d'un lot déposé comme d'un lot en ligne par le même chemin (ADR-0055).

## Décision

**Décision 1 — `deletions` du lot en ligne.** Les `deletions` du corps sont **acceptées**, comme `docs/05:135` le montre. Le lot interne d'ADR-0055 écrit les `doc_id` à supprimer dans un `deletions.parquet` conforme au format de lot v1 (`docs/format-lot-v1.md:293-297`, `:165-166` : le fichier existe si et seulement si `counts.deletions > 0`), et l'attache les applique comme pour tout lot : la ligne `documents` tombe dans la transaction du commit, avec les pierres tombales (ADR-0041, décision 1).

- **Aucun second chemin de suppression** : `GenerationStore::commit` ne reçoit aucune suppression de la route. Le point de `docs/suivi.md` du 2026-10-25 sur `commit(deletions)` reste hors d'atteinte, et le seul chemin reste l'attache.
- Les `deletions` **comptent dans la limite de 10 000 entrées** du corps, avec les passages décrits.
- Un `doc_id` présent à la fois dans `documents` et dans `deletions` rend **`400 invalid_request`**, avant la mise en file. C'est la règle du format de lot (`docs/format-lot-v1.md:302`, `docs/04:192`, ADR-0040:43), appliquée au corps de la route.
- Un corps qui ne porte que des `deletions` est un lot **sans passage** : son rapport porte `acceptance: null` (`docs/04:220`), et le lot interne contient un `documents.parquet` et un `passages.parquet` vides, `counts.documents` et `counts.passages` à zéro, puisque le format exige au moins un fichier de chaque rôle (`docs/format-lot-v1.md:143-144`) et qu'un lot sans document n'a aucun passage (`:162-163`).

**Décision 2 — `batch_id` du lot en ligne.** Le `batch_id` est celui du **corps**, obligatoire. Il est passé à `crible-store::online` par **un seul constructeur**, partagé par la CLI et la route ; la CLI continue de le dériver des octets du fichier (ADR-0055, décision 2). Un rejeu du même `batch_id` avec le même contenu ne fait rien (INV-10) ; un contenu différent rend `batch_conflict` dans le job. Le constructeur ne dérive jamais l'identité des octets pour la route : un corps JSON reformaté par un client ne doit pas changer l'identité d'un lot.

**Décision 3 — erreurs détectées par le job.** `batch_conflict` (lot en ligne et lot déposé), `not_found` d'un document inconnu ou retiré dans un patch (`docs/05:165`), `acceptance_failed` et `incompatible_model` sont détectés **pendant le job** : la soumission rend `202`, et le job passe à `failed` avec le `error.code` correspondant (ADR-0103, décision 4). Seule la **forme** du corps est validée avant la mise en file : `400 invalid_request` (corps illisible, clé inconnue, `replace_all`, `doc_id` dans les deux rôles, chemin de lot invalide) et `413 payload_too_large` (ADR-0102). Un conflit ou un document inconnu ne peut donc pas être annoncé dans la réponse de soumission : le client suit le job, ou attend `?wait=true`.

**Décision 4 — durées par étape de l'attache.** Dans le serveur, les durées de l'attache (ADR-0042) ne sont rendues par **aucune réponse de l'API**, y compris `?wait=true`. Le job les **journalise une fois, à sa fin**, au niveau `info`, avec `job_id`, `kind` et `index` ; aucune ligne ne porte de texte client (INV-13, ADR-0096). Elles ne sont ni persistées ni portées par le job.

- **Raison** : un job est asynchrone et peut être relu par d'autres que son auteur ; la télémétrie d'exploitation est le journal, et les métriques de P7.
- La CLI continue de les afficher à son appelant : elle n'a qu'un destinataire, et ADR-0042 le lui permet.

**Décision 5 — `POST /v1/indexes/{index}/batches`.** Le corps est `{ "directory": "<nom>" }`. `<nom>` est **un seul composant de chemin** sous `[paths].exchange`/`inbox` : `path_component` refuse la chaîne vide, `.`, `..` et les séparateurs `/` et `\` (`crates/crible-core/src/path.rs`). Avant la mise en file :

1. le chemin `inbox/<nom>` est **canonicalisé** et doit rester sous la racine canonique de l'inbox ;
2. `<nom>` lui-même ne doit pas être un **lien symbolique** ;
3. tout lien symbolique **à l'intérieur du dossier du lot, à toute profondeur**, fait refuser le lot, y compris un lien vers un autre lot de l'inbox ;
4. toute entrée qui n'est ni un fichier ni un dossier fait refuser le lot.

Un refus rend **`400 invalid_request`**, avec un message qui nomme le champ `directory` et **jamais un chemin absolu du serveur** (ADR-0101). Un dossier absent rend **`404 not_found`**. Le job relit le lot dans le même dossier : la course entre la validation et la lecture est **assumée et documentée**, l'inbox étant sous le contrôle de l'exploitant et le serveur seul à la lire. Le lecteur de lot ouvre les fichiers **sans suivre de lien** lorsque la plateforme le permet.

La règle est plus stricte que la formulation de la fiche (« les liens symboliques qui sortent du dossier ») : un lien interne rendrait le contenu du lot dépendant d'un autre dossier que celui validé, et un `manifest.json` lu à travers un lien pourrait changer entre la validation et le job.

**Décision 6 — `GET /v1/indexes/{index}/documents/{doc_id}`.** Le document est lu dans la **base de l'époque de l'instantané publié**, jamais dans l'état SQLite courant. Corps :

```json
{ "doc_id": "LEGIARTI…", "generation": 1288, "doc_hash": "blake3:…",
  "fields": { "num": "1240", "fonds": "LEGI" },
  "passages": [ { "ord": 0, "fields": { "partie": "motifs" }, "text": "…" } ] }
```

- `generation` est celle de l'instantané lu ;
- seuls les champs **`returned`** du schéma sont rendus, à la même règle que les résultats de recherche ;
- seuls les passages **vivants** sont rendus, triés par `ord` croissant ;
- `text` n'apparaît qu'avec `?text=true` ; sans lui, chaque passage porte `ord` et `fields` ;
- aucun vecteur, aucun iid (**INV-01**) ;
- un document inconnu ou retiré rend **`404 not_found`** (`docs/05:43`).

Rôle `read`.

**Décision 7 — `POST /compact`, `/exports`, `/snapshots`.** Ces trois routes sont **sans corps** : un corps non vide rend `400 invalid_request`. Chacune rend `202` avec un job (`compact`, `export`, `snapshot`) et honore `?wait=true`. Les destinations sont dérivées de la **génération publiée** (`docs/04:318-367`, ADR-0061, ADR-0064). Le job s'exécute dans **l'écrivain de l'index**, qui est donc suspendu pendant la copie ; les lectures continuent sur leur `Arc<Snapshot>` (ADR-0017, ADR-0065, `docs/02:190`). Aucune **restauration** n'est exposée par l'API en v1 : `crible restore` reste une commande hors ligne (ADR-0065). Un échec du job `compact` suit ADR-0108.

**Décision 8 — rôles.** `POST` et `PATCH /documents` et `POST /batches` sont `write` ; `GET /documents/{doc_id}` est `read` ; `compact`, `exports` et `snapshots` sont `admin` (`docs/05:11-13`, ADR-0095). Chaque famille prouve son `403` dans cette tâche (ADR-0095).

Ce qui n'est pas décidé ici : les libellés exacts des messages autres que ceux nommés ; la forme d'un rapport d'export ou d'instantané dans le job, qui suit `docs/04:350` ; le comportement d'une restauration par l'API, qui n'existe pas en v1 ; la conduite à tenir si un fichier du lot change entre la validation et le job (la course est assumée, pas détectée).

## Conséquences

- **Positives** :
  - la suppression en ligne existe par le **seul** chemin d'attache : un lot en ligne et un lot déposé ont la même sémantique, le même rapport et la même atomicité (**INV-03**), sans second protocole à tenir ;
  - un seul constructeur de lot interne pour la CLI et la route, avec une identité choisie par l'appelant côté API et dérivée du contenu côté fichier : les deux usages restent idempotents (INV-10) ;
  - `GET /documents/{doc_id}` lit l'instantané : il ne peut pas rendre un passage, un champ ou une génération que la requête de recherche n'aurait pas vus (**INV-02**, **INV-03**) ;
  - la règle de l'inbox ferme les liens symboliques, que `path_component` ne voit pas, et rend la validation et la lecture du lot indépendantes d'un autre dossier ;
  - les durées restent une télémétrie d'exploitation : aucun job relu ne porte d'information que son auteur seul devrait voir, et aucune réponse n'ajoute une forme réservée à `?wait=true` ;
  - `compact`, `exports` et `snapshots` se comportent comme les autres mutations : `202`, job suivi, écrivain suspendu, lectures servies.
- **Négatives / coûts** :
  - la limite de 10 000 devient un compte d'**entrées** mêlant passages et suppressions : le client ne peut plus dimensionner un corps en passages seuls, et le message de refus doit dire ce qui est compté ;
  - un `batch_conflict`, un `acceptance_failed` ou un document inconnu n'apparaît plus dans la réponse de soumission : un client qui n'attend pas le job doit lire son état pour connaître l'échec ;
  - la validation d'un lot déposé parcourt le dossier avant la mise en file — coût proportionnel au nombre de fichiers — et refuse des liens symboliques qu'un exploitant pouvait trouver commodes ;
  - la course entre validation et lecture est assumée : un lot modifié après validation est attaché dans son nouvel état, sans détection ;
  - les durées ne sont plus lisibles par le client : expliquer une attache passée demande le journal du serveur ;
  - les trois routes d'administration refusent un corps, alors qu'un client générique pourrait en envoyer un vide ou un objet de réglages.
- **À surveiller** :
  - un test de bout en bout qui envoie `documents` et `deletions`, vérifie `202`, le job `succeeded`, les comptes `added` et `deleted`, et que la ligne `documents` a disparu de `GET /documents/{doc_id}` ;
  - un test d'un corps qui ne porte que des `deletions` : lot interne sans passage, `acceptance: null`, suppression appliquée ;
  - un test d'un `doc_id` dans les deux rôles : `400 invalid_request`, avant toute mise en file et sans génération ;
  - un test de rejeu du même `batch_id` avec le même corps (aucune génération, INV-10) et avec un autre contenu (`failed`, `batch_conflict`) ;
  - un test de chemin de lot : `..`, chemin absolu, séparateur, `<nom>` lien symbolique, lien interne au dossier, entrée spéciale — `400` nommant `directory`, sans chemin absolu dans le message ; dossier absent `404` ;
  - un test qui remplace le contenu d'un lot après validation et vérifie que le job lit le nouvel état, donc que la course est bien celle qui est assumée ;
  - un test de `GET /documents/{doc_id}` pendant une compaction : le document est lu dans l'époque de l'instantané, `generation` est celle de l'instantané, aucun iid n'apparaît ;
  - un test de `?text=true` absent : aucun `text`, ni au document ni dans les passages ;
  - un test de durée : la réponse de `?wait=true` ne porte aucune durée, et le journal du job en porte une par étape avec `job_id`, `kind` et `index`.

- **Documents mis à jour par cet ADR** :

| Document | Modification |
|---|---|
| `docs/05-api.md:64-67` | table des routes : `POST /documents` (`batch_id` obligatoire, `deletions`, erreurs dans le job), `GET /documents/{doc_id}` (instantané publié, `?text=true`), `POST /batches` (corps `directory`, règle de chemin) |
| `docs/05-api.md:71-73` | `compact`, `exports`, `snapshots` : sans corps, renvoi à cet ADR |
| `docs/05-api.md:43,45` | `not_found` et `batch_conflict` : moment où le job les rend |
| `docs/05-api.md:118-139` | section « Lot en ligne » : `batch_id` du corps, `deletions` et `deletions.parquet`, `400` sur un `doc_id` dans les deux rôles, erreurs du job, limite de 10 000 entrées |
| `docs/05-api.md:165` | « Patch de métadonnées » : le `not_found` est porté par le job |
| `docs/05-api.md`, après `:171` | nouvelle section « Lecture d'un document », avec un exemple |
| `docs/05-api.md:328` | « Jobs » : durées journalisées à la fin du job, jamais rendues par l'API |
| `docs/05-api.md:343` | limite du corps d'un lot en ligne : 10 000 entrées, passages et `deletions` confondus |
| `docs/04-donnees-et-ingestion.md:133-152` | « Format de lot v1 » : règle de chemin d'un lot déposé dans l'inbox |
| `docs/04-donnees-et-ingestion.md:241-278` | « Mode en ligne » : `deletions.parquet` du lot interne, aucun second chemin de suppression, lot sans passage |
| `docs/adr/0055-…md` | note « Complément du 2026-10-29 » renvoyant à cet ADR, sans modification du corps |
| `docs/adr/0042-…md` | note « Complément du 2026-10-29 » renvoyant à cet ADR, sans modification du corps |
| `docs/adr/README.md` | ligne d'index de cet ADR |

`docs/02-architecture.md` n'est pas modifié : sa section `exchange/` (`:79`) nomme les trois dossiers sans décrire de lien symbolique, et la règle de l'inbox appartient à `docs/04` et à `docs/05`. `docs/format-lot-v1.md` n'est pas modifié non plus : sa section « lot interne du mode en ligne » (`:171-212`) ne dit pas que le lot n'a pas de `deletions`, et le format général autorise déjà un `deletions.parquet` en mode `upsert` (`:125-127`).

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Refuser `deletions` dans le corps du lot en ligne | Contredit `docs/05:135` et `docs/05:64`, et laisse une API d'écriture sans moyen de retirer un document, alors que le mode `replace_all` est refusé en ligne : un client ne pourrait jamais supprimer qu'en déposant un lot. |
| Un second chemin de suppression par `GenerationStore::commit(deletions)` | Deux sémantiques de suppression à tenir — rapport, atomicité, `batch_id`, `manifest_blake3` — pour un besoin que le `deletions.parquet` du lot interne couvre déjà (ADR-0041). |
| Dériver le `batch_id` de la route des octets du corps | Un corps JSON reformaté, réordonné ou ré-indenté changerait l'identité d'un lot identique sur le fond ; `docs/05:134`, `docs/04:259` et ADR-0055, « Cohérence avec P6 », disent que la route le prend dans le corps. |
| Rendre les durées dans la réponse `?wait=true` | Crée une forme de réponse de plus, réservée à un seul cas, et la rend à un client qui peut relire un job plus tard sans jamais l'obtenir ; le journal porte l'information pour l'exploitation. |
| Contrôler `batch_conflict` avant la mise en file | Demande de lire la table `batches` hors de l'écrivain, donc hors de l'ordre de la file : un lot accepté pourrait entrer en conflit avec un job déjà en attente. Le job est le seul endroit où l'ordre est connu. |
| Accepter un chemin relatif à plusieurs composants sous l'inbox | Rouvre `..` et les chemins absolus que `path_component` ferme, et rend la canonicalisation dépendante du contenu du disque plutôt que du seul corps. |
| Suivre les liens internes à l'inbox | Le lot validé ne serait plus celui que le job lit si le lien change, et un lot pourrait emprunter le `manifest.json` d'un autre : la fiche demande déjà de refuser les liens qui sortent du dossier (ADR-0107, décision 5). |
| N'interdire que les liens qui sortent du dossier (formulation de la fiche) | Une résolution par fichier, à chaque ouverture, laisse une fenêtre entre la validation et la lecture ; refuser tout lien à toute profondeur est plus simple à prouver et ne coûte rien à un lot déposé normalement. |
| Copier le lot validé sous `<data>/tmp` avant la mise en file | Ferme la course, mais copie un lot qui peut peser des gigaoctets et occupe deux fois le disque ; l'inbox est sous le contrôle de l'exploitant et le serveur seul à la lire, donc la course est assumée. |
| Lire le document dans l'état SQLite courant | Une compaction entre la publication de l'instantané et la lecture rendrait des passages, des champs ou une génération que l'instantané ne contient pas : contraire à **INV-02** et **INV-03**. |
| Exposer la restauration par l'API en v1 | La restauration est hors ligne (ADR-0065) : elle remplace une époque sous les lecteurs d'un index servi et demande l'arrêt du serveur. Un job ne peut pas la rendre sûre. |
| Accepter un corps sur `compact`, `exports` et `snapshots` | Aucun réglage de ces opérations n'est décidé : destinations et bornes viennent de la génération publiée et de `docs/04`. Un corps accepté puis ignoré serait un contrat faux. |

## Complément du 2026-10-29 (relecture de ROUTES-DOCUMENTS-ET-LOTS)

Deux précisions, sur les décisions 5 et 1. Le statut de l'ADR ne change pas, et le corps des décisions reste écrit tel qu'il a été accepté ; les deux règles ci-dessous font foi à sa place.

**Constat (a).** La décision 5 finit par : « Le lecteur de lot ouvre les fichiers sans suivre de lien lorsque la plateforme le permet. » Or ouvrir un fichier sans suivre de lien demande l'appel système `openat` avec `O_NOFOLLOW`, donc la crate `libc`, qui n'est pas dans `docs/07`. La relecture de `ROUTES-DOCUMENTS-ET-LOTS` montre en outre que le chemin qui lit réellement le lot est **l'écrivain**, au moment du job, et non le lecteur de l'attache.

**Décision (a).** La phrase de la décision 5 est **remplacée** par : « Le job revalide le dossier du lot — même règle : chemin canonique sous l'inbox, aucun lien symbolique à aucune profondeur, aucune entrée spéciale — juste avant de le lire, dans l'écrivain ; la fenêtre résiduelle entre cette revalidation et la lecture est assumée (l'inbox est sous le contrôle de l'exploitant). »

**Raison (a).** `O_NOFOLLOW` fermerait la fenêtre au prix d'une crate absente de `docs/07`, donc d'un ADR de dépendance, pour un intervalle que la revalidation réduit déjà à quelques millisecondes. Le coût de la règle augmente en revanche pour l'exploitant : deux parcours du dossier par lot, celui de la soumission et celui du job.

**Constat (b).** La décision 1 accepte un corps qui ne porte que des `deletions`, et ne dit rien d'un corps qui ne porte **ni** `documents` **ni** `deletions`. Le chemin par fichier refuse déjà un fichier vide (`crates/crible-store/src/online.rs`) : laisser passer le corps vide donnerait deux comportements pour la même absence de contenu, et créerait une génération sans rien à attacher.

**Décision (b).** Un corps de lot en ligne **sans aucun document et sans aucune suppression** rend **`400 invalid_request`**, avant la mise en file, comme le chemin par fichier refuse un fichier vide. Un corps qui ne porte que des `deletions` reste accepté : c'est un lot sans passage, et la décision 1 le décrit.

**Raison (b).** Un lot vide n'est un lot ni par ses documents ni par ses suppressions : l'accepter ferait écrire un lot interne aux fichiers vides, créerait une génération et un rapport sans effet, ou ferait échouer le job plus tard, loin de la requête qui pouvait être corrigée.

**Application.** `docs/05-api.md` reçoit les deux règles : la revalidation dans « Lot déposé » et dans la ligne `POST /batches` de la table des routes, le refus du corps vide dans « Lot en ligne », dans la table des routes et dans la ligne `invalid_request` de la table des codes. `docs/04-donnees-et-ingestion.md:144` n'est pas modifié : sa règle d'inbox décrit la canonicalisation, les liens et la course assumée, sans citer d'ouverture sans lien.

## Références

- `docs/phases/P6-serveur-et-api.md:84-97` — fiche `ROUTES-DOCUMENTS-ET-LOTS`, et `:131`, `:139`, `:141` pour les tâches qui en dépendent.
- `docs/05-api.md:11-13` (rôles), `:20-21` (`202`, `?wait=true`), `:25-51` (corps d'erreur, codes), `:55-77` (routes), `:118-139` (lot en ligne), `:153-171` (patch), `:314-336` (jobs), `:337-351` (limites).
- `docs/04-donnees-et-ingestion.md:133-201` (format de lot, sémantique d'attache, idempotence), `:220` (lot sans passage), `:241-278` (mode en ligne), `:279-305` (patch), `:318-367` (export, instantané, restauration).
- `docs/format-lot-v1.md:12-46` (dossier d'un lot), `:125-145` (modes, fichiers, `deletions.parquet`), `:171-212` (lot interne du mode en ligne), `:293-297` (colonne `deletions`).
- `docs/02-architecture.md:55-93` (organisation sur disque, `exchange/`), `:161-172` (concurrence, instantanés), `:174-203` (compaction).
- `docs/08-invariants-et-tests.md` — INV-01, INV-02, INV-03, INV-10, INV-13.
- `crates/crible-store/src/online.rs:626-635` — refus d'un lot sans document et `batch_id` dérivé des octets.
- `crates/crible-core/src/path.rs` — `path_component` : chaîne vide, `.`, `..`, `/` et `\`.
- ADR-0017 (compaction et écritures), ADR-0040 (unicité dans un lot), ADR-0041 (attache, document supprimé), ADR-0042 (durées), ADR-0055 (lot en ligne), ADR-0056 (verrou de présence), ADR-0060 (patch), ADR-0061 (export), ADR-0064 (instantané), ADR-0065 (instantané hors ligne, restauration), ADR-0067 (`?wait=true`), ADR-0095 (rôles), ADR-0096 (journal), ADR-0097 (liste fermée des clés), ADR-0100 (`401`, `403`), ADR-0101 (corps d'erreur), ADR-0102 (limites et 32 Mio), ADR-0103 (jobs), ADR-0108 (époque après une compaction en échec).
