# ADR-0149 — Durées d'attache dans l'objet `Job`, en mémoire et jamais persistées

- **Statut** : Accepté (accepté par anticipation par le propriétaire le 2026-11-11, confirmé par le propriétaire le 2026-11-11)
- **Date** : 2026-11-11
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, tâche v1.1 `T02-DUREES-ATTACHE-DANS-JOB`, sur demande de l'orchestrateur, d'après le verdict de `@architecte` du 2026-11-11 et la décision du propriétaire du 2026-11-11
- **Remplace** : ADR-0107, **décision 4 seulement, et en partie** — la ligne de journal `attach durations` reste. Le reste d'ADR-0107 n'est pas modifié.
- **Décision** : confirmée par le propriétaire le 2026-11-11, sa réponse : « ADR-0149 accepté ».

**Erratum du 2026-11-11 (correction d'une phrase de plan devenue sans objet, sans réécriture d'aucune décision).** Le tableau d'application portait, pour `docs/05-api.md:467-476`, la phrase « l'autre exemple d'objet sans durée reste `null` ». Elle était sans objet : la section « Jobs » de `docs/05` ne contient que deux exemples d'objet `Job`, ceux d'un `attach_batch` et d'un `online_batch`, et les deux portent désormais `durations`. La ligne du tableau est corrigée ci-dessous et ne demande plus que l'ajout du membre à l'exemple `online_batch`. Aucun argument, aucune décision et aucune alternative ne changent.

## Contexte

### Le besoin, venu du projet Legifacile

La tâche quotidienne d'ingestion de Legifacile doit publier **où passent les minutes** d'une attache, sans lire `docker logs` sur la machine qui héberge Crible : ce chemin n'est pas portable — production, Mac mini, sauvegarde, rejeu hors du conteneur. Les mesures MES-02 et MES-08 ont dû analyser les journaux par expression régulière, avec des échecs d'attribution déjà constatés (ticket `02-durees-attache-dans-job.md`, « Impact sur Legifacile »).

Le ticket est `/Users/mattboll/MattCorp/legifacile/tickets-crible/02-durees-attache-dans-job.md`, lu en lecture seule ; c'est un autre projet, il n'est pas modifié par cet ADR.

### Ce qui existe dans le code

Les six durées de l'attache — `validation`, `segment`, `transaction`, `lexical`, `manifest`, `snapshot` — sont mesurées dans `AttachDurations` (`crates/crible-store/src/attach.rs:240-258`) et rendues par le rapport d'appel dans `AttachReport::durations`, `Option<AttachDurations>` (`crates/crible-store/src/attach.rs:161-168`). Le serveur ne les lit qu'à **un seul endroit** : `log_attach_durations` (`crates/crible-server/src/jobs.rs:1371-1398`), qui écrit une ligne `info` de message `attach durations`, avec `job_id`, `kind`, `index` et six champs entiers `validation_ms`, `segment_ms`, `transaction_ms`, `lexical_ms`, `manifest_ms`, `snapshot_ms`, chacun en **millisecondes entières** par `millis()` (`crates/crible-server/src/jobs.rs:1400-1404`). Cette fonction ne fait rien quand `durations` vaut `None` (`:1383-1385`). Elle est appelée par les deux routes d'attache, après une attache réussie seulement : `crates/crible-server/src/batches.rs:192-197` et `crates/crible-server/src/documents.rs:194-199` — un échec d'attache sort par `?` avant l'appel.

L'objet `Job` est rendu par `JobView::to_json` (`crates/crible-server/src/jobs.rs:106-120`) et décrit par le composant `Job` de la spécification (`crates/crible-server/src/openapi.rs:605-636`). Il porte déjà un membre gardé en mémoire et jamais écrit dans la table : `result` d'un `patch_documents` (`crates/crible-server/src/jobs.rs:377-411`, `publish_with` à `:458-474`, ADR-0060, ADR-0103 décision 3). `?wait=true` rend le **même** objet, par le même `to_json` (`crates/crible-server/src/http.rs:530-545`) : il n'existe pas de seconde forme de réponse de job.

### Ce que la conception décide déjà, et qui ne change pas

- **ADR-0042** : les durées sont de la télémétrie d'appel, **jamais persistées** ; `batches.report` porte les six comptes, les statistiques de recette et la génération, sans durées ; un rejeu (INV-10, `docs/08-invariants-et-tests.md:18`) rend le rapport persisté de l'attache d'origine, identique clé pour clé. `docs/04-donnees-et-ingestion.md:206` et `:247` le disent. **Rien de tout cela ne change ici.**
- **ADR-0103, décisions 3 et 6** : la table `jobs.sqlite` n'est écrite qu'aux transitions d'état ; la progression fine vit en mémoire ; un job terminé depuis plus de 30 jours est purgé, de la table et de la mémoire, et un job inconnu ou purgé rend `404`.
- **ADR-0107, décision 4** : aucune réponse de l'API ne rend les durées, `?wait=true` compris ; le job les journalise une fois, à sa fin ; l'alternative « rendre les durées dans `?wait=true` » est écartée dans son tableau des alternatives (`docs/adr/0107-ecriture-de-documents-et-de-lots-par-l-api.md:144`). C'est cette décision, et elle seule, que le présent ADR remplace, en partie.
- **ADR-0134** : l'objet `Job` est fermé (ADR-0111, décision 7), `progress` vit en mémoire et n'est publié que par `attach_batch`, `online_batch` et `patch_documents` ; un job réussi qui n'a joué aucune étape, comme le rejeu d'un lot déjà attaché, garde `progress: null`.
- **ADR-0117, décision 5** : `OpEvent` a une liste **fermée** de champs, dont un seul `duration`, la durée totale de l'opération (`docs/adr/0117-analytics-stockage-et-calcul-schema-histogrammes-percentile-et-formes-de-filtre.md:167-176` ; `crates/crible-server/src/analytics.rs:235-248`).
- **ADR-0143** : une mineure n'apporte que des **ajouts compatibles** ; `format_version` ne change pas dans la ligne 1.x ; un client ancien ne casse pas, le client Python rendant des `dict` (ADR-0112).
- **ADR-0067** : `?wait=true` attend au plus `wait_timeout_ms`, 60 000 ms par défaut, puis rend `202` avec le job dans son état courant.

### Le point de bascule : `?wait=true` n'attend jamais une attache réelle

Une attache réelle dure **253,626 s** par l'ancien envoi de vecteurs en ligne, et **283,155 s** par le chemin de production `write_batch` + `attach_batch` depuis l'inbox, dont **277 815 ms de recette**, soit 98,1 % : c'est la mesure **MES-02 de Legifacile**, projet distinct dont les numéros de mesure recouvrent ceux de Crible, `../legifacile/docs/mesures/MES-02-encodage-quotidien.md:204`, `:511` et `:513` — le ticket lit les 277,8 s comme une durée d'attache, la mesure les donne comme la part de recette de l'attache de production. Le rendu de la soumission, lui, est d'environ 0,02 s (verdict de `@architecte` du 2026-11-11, consigné dans `docs/suivi.md:163`). Une attache réelle dépasse donc **toujours** les 60 s d'ADR-0067 : `?wait=true` rend `202` avec le job `queued` ou `running`, et le client lit le job ensuite. Rendre les durées dans `?wait=true` ne les rendrait jamais pour une attache réelle. Il faut donc un membre **du job**, lisible après coup.

### Le verdict de `@architecte` du 2026-11-11

- **(a) Les durées dans le job** : **ECART** à ADR-0107, décision 4. C'est la seule piste qui donne le découpage par étape. `?wait=true` n'attend jamais une attache réelle, pour la raison ci-dessus.
- **(b) Les durées par étape dans la télémétrie de P7** : **ECART** à ADR-0117, décision 5, `OpEvent` n'ayant qu'un champ `duration`, et changement de format d'`analytics.sqlite`. Le `attach_batch` de P7 ne mesure de toute façon que la requête de soumission, environ 0,02 s (`crates/crible-server/src/batches.rs:92-108`, `crates/crible-server/src/documents.rs:120-137`) : il ne verrait aucune étape.
- **(c) La documentation seule** : conforme, mais sans découpage par étape, donc sans réponse au besoin.
- **(d) `?durations=true`, ou une restriction à la clé qui a soumis** : demanderait une colonne `client` dans `jobs.sqlite`, donc un changement de format, ou un état propre à l'auteur absent du serveur.

### Décision du propriétaire du 2026-11-11

Les durées vont dans un membre `durations` de l'objet `Job`, gardé **en mémoire**, **jamais persisté** ; lisible par tout client qui peut lire le job (niveau `write`, comme le rapport), sans restriction à la clé qui a soumis. `batches.report` et `jobs.sqlite` ne changent pas.

## Décision

**Les durées de l'attache sont portées par un membre `durations` de l'objet `Job`, gardé en mémoire et jamais persisté ; toute clé qui peut lire le job le voit ; `batches.report` et la table `jobs.sqlite` ne changent pas.** ADR-0107, décision 4, est remplacée pour la seule partie qui interdisait de rendre les durées ; la ligne de journal, elle, reste.

### 1. Forme du membre

`durations` est un membre **toujours présent** de l'objet `Job` — l'objet est fermé, ADR-0111 décision 7 — et vaut `null` quand il n'y a rien à rendre. Quand il porte un objet, celui-ci est **fermé** et a **exactement six clés**, les mêmes noms que la ligne de journal `attach durations` :

```json
"durations": { "validation_ms": 1234, "segment_ms": 5678, "transaction_ms": 90,
               "lexical_ms": 0, "manifest_ms": 12, "snapshot_ms": 34 }
```

- **Type et unité** : chaque valeur est un **entier** de **millisecondes entières**, la conversion `millis()` de `crates/crible-server/src/jobs.rs:1400-1404`, celle de la ligne de journal. Les valeurs numériques sont donc **identiques**, étape par étape, à celles que le journal publie pour le même `job_id` : un exploitant peut comparer les deux sans conversion.
- **Pourquoi pas des `f64` à trois décimales comme `timings_ms`** : `docs/05-api.md:344` fixe `timings_ms` en `f64` à trois décimales, parce que ses étapes sont mesurées en microsecondes et sommées pour un test de cohérence (ADR-0119, décision 5). Ici la référence est la ligne de journal, dont l'unité est la milliseconde entière ; passer en `f64` afficherait pour la même étape un nombre différent de celui du journal, et la règle « exactement les durées que la ligne publie » deviendrait fausse.
- **Étape non jouée** : la clé est **présente et vaut `0`**, jamais `null`. C'est ce que `AttachDurations` porte déjà — `lexical` vaut zéro quand l'étape ne tourne pas, configuration sans branche lexicale ou génération qui ne change aucun texte (`crates/crible-store/src/attach.rs:249-253`) — et ce que la ligne de journal écrit (`lexical_ms = 0`). Un `null` dirait « non mesurée », ce qui n'est pas le cas : l'étape a été mesurée et n'a rien coûté.

### 2. Quand le membre est non nul

**Règle, pour une seule source** : `durations` porte **exactement** les durées que la ligne `attach durations` publie, **quand** elle est publiée, et vaut `null` sinon. Les deux sortent de la même valeur, `report.durations` (`Option<AttachDurations>`), et de la même conversion en millisecondes entières : pas de second calcul, pas de second chemin.

| Cas | `durations` | Pourquoi |
|---|---|---|
| Job `queued` ou `running` | `null` | Le rapport de l'attache n'existe pas encore. |
| Job `succeeded`, attache qui a joué les étapes | objet à six clés | C'est le cas où la ligne de journal est publiée. |
| Job `failed` | `null` | L'attache échoue par `?` avant `log_attach_durations` (`batches.rs:186-197`, `documents.rs:183-199`) : la ligne n'est pas publiée, et des durées partielles seraient une seconde source. Les quatre états sont `queued`, `running`, `succeeded` et `failed` : il n'y a pas d'état « annulé ». |
| Rejeu d'un `batch_id` déjà appliqué | `null` | Aucune étape n'est jouée : `report.durations` vaut `None` (`crates/crible-store/src/attach.rs:161-168`), et `log_attach_durations` sort sans rien journaliser. Même raisonnement qu'ADR-0134 pour `progress: null`. Le job est `succeeded`. |
| Autres natures : `patch_documents`, `patch_config`, `compact`, `export`, `snapshot`, `rebuild_lexical`, `delete_index` | `null` | Aucune ne produit d'`AttachDurations` ; `patch_documents` porte son propre rapport en mémoire (ADR-0060) et n'a pas d'étape mesurée. |
| Après un redémarrage, job relu de `jobs.sqlite` | `null` | Le membre vit en mémoire seulement ; la ligne relue ne le porte pas. |
| Après la purge des 30 jours | job absent | `404 not_found` (ADR-0103, décision 6) : il n'y a plus de job à lire. |

Deux moments distincts, une seule valeur : la ligne de journal est écrite **dans le travail du job**, avant la transition terminale, et le membre apparaît dans l'objet `Job` **à cette transition terminale**, quand le job devient visible `succeeded`. Un client qui interroge le job avant n'a rien, comme pour `progress`.

### 3. Les routes qui le rendent

Le membre est rendu, sans exception, par les trois chemins qui rendent un objet `Job` :

- `GET /v1/jobs/{job_id}` ;
- `GET /v1/jobs`, pour chaque job de `{ "jobs": [ … ] }` ;
- la réponse de `?wait=true` des deux routes d'attache, `POST /v1/indexes/{index}/documents` et `POST /v1/indexes/{index}/batches`, qui rendent le **même** objet `Job` par `JobView::to_json`, `200` si le job est terminé dans le délai, `202` avec son état courant sinon (ADR-0067, `crates/crible-server/src/http.rs:530-545`). Aucune seconde forme de réponse, aucun paramètre de requête nouveau.

**Rôle** : `write`, comme les deux routes de jobs aujourd'hui (`docs/05-api.md:71`) et comme le rapport. Aucune restriction à la clé qui a soumis : le serveur ne garde pas l'auteur d'un job, et toute clé `write` ou `admin` voit les jobs de l'instance (`docs/05-api.md:488`, ADR-0103, décision 7).

### 4. INV-10 ne bouge pas

`batches.report` et `result.report` restent **identiques clé pour clé**, sans durées : le membre `durations` est **hors** de `result`, à côté de `progress`, `generation` et `error`. Un rejeu rend `durations: null` (point 2), et le rapport rejoué reste celui de l'attache d'origine. Aucune écriture n'a lieu après le point de commit (ADR-0016) ; `jobs.sqlite` ne gagne **aucune colonne** et garde `format_version = 1`.

ADR-0103, décision 3, dit que « les durées de l'attache n'entrent jamais dans le job (ADR-0042) » : cette phrase vise la **persistance**, et elle reste vraie — les durées n'entrent ni dans une colonne de `jobs.sqlite` ni dans `result`. Le membre vit en mémoire seulement, exactement comme le rapport d'un `patch_documents`, que la même décision 3 autorise à rester en mémoire sans être persisté (ADR-0060) : c'est le précédent d'implémentation, et la publication par `publish_with` le suit.

### 5. Ce qui reste, et ne change pas

- La ligne de journal `attach durations` **reste** : c'est la partie d'ADR-0107, décision 4, que cet ADR ne remplace pas. Elle garde son niveau `info`, ses champs et son unicité par job (ADR-0096, INV-13), et ses tests actuels (`crates/crible-server/tests/common/mod.rs:514-566`, `crates/crible-server/tests/routes_documents.rs:538`).
- La durée `attach_batch` des analytics de P7 **reste la durée de la requête de soumission**, environ 0,02 s : `record_operation` est appelé autour de `post_batch_inner` et de `post_documents_inner`, pas autour du travail du job (`crates/crible-server/src/batches.rs:92-108`, `crates/crible-server/src/documents.rs:120-137`). Rien ne change dans `analytics.sqlite` ni dans `OpEvent` (ADR-0117, décision 5).

### 6. Classe de compatibilité (ADR-0143)

**Ajout compatible**, au sens de la décision 1 d'ADR-0143 : un **membre de réponse nouveau**, dont l'absence garde le comportement précédent.

- Aucun `format_version` ne change : ni `jobs.sqlite`, ni `analytics.sqlite`, ni un segment. Aucune migration, donc.
- Aucune route, aucun champ de requête, aucun code d'erreur, aucune valeur par défaut ne change ; aucune clé ne devient obligatoire.
- Un client 1.0 **ignore** le membre : le client Python rend des `dict` sans type déclaré (ADR-0112), et la spécification ne décrit qu'une réponse possible.
- La **spécification OpenAPI** et `docs/05` sont mis à jour **dans le même changement** que le code (ADR-0111, décision 5 ; ADR-0143, décision 4 pour la note de version).

### Ce que cet ADR ne décide pas

- La forme d'un découpage par étape pour les autres natures de job (`compact`, `export`, `snapshot`, `patch_documents`) : il demanderait ses propres durées et son propre ADR.
- Le devenir de la ligne de journal, qui reste telle quelle.
- Le classement des durées dans les analytics, sur lequel rien ne change.
- La mention de ce membre dans la note de version `docs/versions/1.1.0.md`, qui appartient à la tâche de publication (`PUBLIER-V1.1.0`) ; elle y figurera comme membre ajouté.

## Conséquences

- **Positives** :
  - la tâche quotidienne de Legifacile publie où passent les minutes **sans lire `docker logs`** : un client HTTP suffit, et le découpage par étape est enfin lisible hors du conteneur ;
  - une seule source : le membre et la ligne de journal sortent de la même valeur et de la même conversion, donc ils ne peuvent pas diverger ;
  - INV-10 et ADR-0042 restent vrais sans exception : le membre est hors du rapport et hors de la table, et un rejeu ne rend rien ;
  - aucune écriture ni aucun format ne change : `jobs.sqlite` garde `format_version = 1`, les analytics et `OpEvent` sont intacts ;
  - un client 1.0 ne casse pas, et le client Python n'a rien à faire pour lire le membre.
- **Négatives / coûts** :
  - le membre est **perdu au redémarrage** : expliquer une attache d'avant le dernier arrêt demande toujours le journal, et un job relu plus tard rend `durations: null` ;
  - `durations` et `result.report` disent des choses voisines sans être au même endroit : un lecteur doit savoir que les durées sont à côté du rapport, pas dedans ;
  - deux lectures d'un même job dans le temps peuvent différer sur ce membre (objet, puis `null` après redémarrage), alors que `result` est stable ;
  - la milliseconde entière perd la résolution sous la milliseconde, que `AttachDurations` porte pourtant en microsecondes ; c'est le prix de l'identité avec la ligne de journal.
- **À surveiller** :
  - une demande d'exploitation pour les durées d'une attache **passée** après un redémarrage : elle rouvrirait la question de la persistance, que le propriétaire a fermée le 2026-11-11 ; la réponse serait une mesure ou une table de télémétrie séparée, pas un rapport ;
  - un écart constaté entre les nombres du membre et ceux du journal pour un même `job_id` : il signalerait un second calcul de durées, donc une régression de la règle « une seule source » ;
  - une attache dont le job est `failed` alors que des étapes ont tourné : le `null` est le contrat, mais si l'exploitation en a besoin, c'est un nouvel ADR ;
  - la tentation d'ajouter `durations` à `result`, qui casserait INV-10 clé pour clé.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| **(b) Les durées par étape dans la télémétrie de P7** | ECART à ADR-0117, décision 5 : `OpEvent` n'a qu'un `duration` et sa liste de champs est fermée ; y ajouter six durées change le format d'`analytics.sqlite`, donc `format_version` et migration, interdits dans la ligne 1.x sans ADR de migration (ADR-0143, décision 2). Et le `attach_batch` de P7 ne mesure que la soumission, environ 0,02 s : l'événement ne verrait aucune étape. |
| **(c) La documentation seule** | Conforme à ADR-0107, décision 4, mais ne rend aucun découpage : la tâche quotidienne devrait toujours lire `docker logs`, ce qui est exactement le besoin du ticket. |
| **Restreindre la lecture à la clé qui a soumis** | Le serveur ne garde pas l'auteur d'un job : il faudrait une colonne `client` dans `jobs.sqlite`, donc un changement de format, ou un état propre à l'auteur absent du serveur. Les deux coûtent un ADR de format et une migration pour un besoin que le ticket n'exprime pas — son rapport est produit par l'exploitant, qui détient de toute façon une clé `write`. |
| **`GET /v1/jobs/{id}?durations=true`** | Crée une seconde forme de réponse pour la même ressource et un paramètre de plus à documenter, à spécifier et à tester, alors que le membre est petit, fermé et sans contenu sensible. Il rendrait aussi le contrat dépendant d'un paramètre que le client doit connaître, là où `progress` a montré qu'un membre toujours présent se lit sans surprise. |
| **Persister les durées dans `jobs.sqlite` ou dans `batches.report`** | Contredit ADR-0042 et INV-10 : `batches.report` est écrit dans la transaction du commit, et aucune écriture n'a lieu après le point de commit (ADR-0016). Une colonne de plus dans `jobs.sqlite` change le format, donc `format_version`, pour une information que le propriétaire a explicitement voulue non persistée. |
| **Rendre les durées dans la réponse `?wait=true` seulement** | L'alternative qu'ADR-0107 écartait déjà (`docs/adr/0107-…:144`) : une forme de réponse de plus, réservée à un cas, et qui ne rendrait **jamais** les durées d'une attache réelle, puisque celle-ci dépasse toujours les 60 s de `wait_timeout_ms` (ADR-0067). |
| **Rendre les durées dans `result`, à côté du rapport** | Casserait INV-10 clé pour clé : `result.report` doit rester le rapport persisté, identique au rejeu, sans durées. Le membre doit être hors du rapport. |
| **Des `f64` à trois décimales, comme `timings_ms`** | Cohérent avec les durées de recherche, mais incohérent avec la source : le journal publie des millisecondes entières, et le membre doit rendre exactement les mêmes nombres. Deux représentations d'une même mesure finiraient par diverger. |
| **Un `null` pour une étape non jouée** | `AttachDurations` mesure l'étape même quand elle ne coûte rien, et la ligne de journal écrit `0`. Un `null` dirait « non mesurée », un troisième état que ni la structure ni le journal ne connaissent. |

## Application

Après acceptation seulement, sur la branche de la tâche `T02-DUREES-ATTACHE-DANS-JOB`, et dans un commit qui cite `ADR-0149` :

| Document ou code | Modification |
|---|---|
| `docs/05-api.md:454-461` | exemple d'un job `attach_batch` : ajouter `"durations": { … }` avec les six clés, à côté de `progress` |
| `docs/05-api.md:467-476` | exemple d'un job `online_batch` : ajouter `"durations": { … }` |
| `docs/05-api.md:482` | remplacer « **aucune réponse de l'API ne les rend**, `?wait=true` compris ; elles ne sont ni persistées ni portées par le job » par la règle de cet ADR : l'objet `Job` porte `durations`, en mémoire, `null` quand il n'y a rien, jamais dans `report` ni dans la table ; la ligne de journal reste |
| `docs/05-api.md:71` | ligne des deux routes de jobs : mention « `durations` d'attache en mémoire, `null` sinon » |
| `docs/04-donnees-et-ingestion.md:247` | « Durées d'attache » : les durées sont rendues par l'objet `Job` au membre `durations`, en mémoire, jamais persistées ; le rapport persisté reste sans durées (ADR-0149) |
| `docs/04-donnees-et-ingestion.md:206` | aucune modification : le rejeu rend le rapport sans durées, et `durations: null` est la règle du point 2 |
| `crates/crible-server/src/jobs.rs` | `JobOutcome` gagne `durations: Option<Value>` ; une fonction unique construit l'objet des six clés (celle que `log_attach_durations` utilise déjà pour ses champs) ; `JobView` gagne le membre, rendu par `to_json` ; `mark_finished` publie le membre **en mémoire** par `publish_with`, sans l'écrire dans la table |
| `crates/crible-server/src/batches.rs:192-201`, `crates/crible-server/src/documents.rs:194-203` | transmettre `report.durations` au membre, la même valeur qu'à `log_attach_durations` |
| `crates/crible-server/src/openapi.rs:605-636` | composant `Job` : membre `durations`, objet fermé à six entiers `*_ms`, `null` quand vide, description citant ADR-0149 ; les deux exemples de `docs/05` restent valides au test `jsonschema` |
| `docs/adr/0107-…md`, `docs/adr/0042-…md`, `docs/adr/README.md` | mentions de remplacement et ligne d'index — faites par cet ADR |

**Tests attendus**, dans `crates/crible-server/tests/` :

1. une attache réussie rend `durations` à six clés entières, et les six nombres sont **égaux** à ceux de la ligne `attach durations` du même `job_id` ;
2. un rejeu du même `batch_id` rend `durations: null`, et `result.report` est identique à celui de l'attache d'origine, clé pour clé (INV-10) ;
3. un job d'une autre nature (`compact`, `export` ou `snapshot`) rend `durations: null`, membre présent ;
4. un job relu après redémarrage rend `durations: null` ;
5. `GET /v1/jobs` rend le même membre que `GET /v1/jobs/{job_id}` pour le même job, et une étape non jouée vaut `0`, jamais `null` ;
6. le rapport de `result` ne porte **aucune** clé de durée.

`docs/02-architecture.md` n'est pas modifié : il décrit les étapes de l'attache, pas la forme d'un job. `docs/06-observabilite.md` et `docs/08-invariants-et-tests.md` non plus : ni `OpEvent` ni les invariants ne changent.

## Références

- Ticket : `/Users/mattboll/MattCorp/legifacile/tickets-crible/02-durees-attache-dans-job.md` (« Constat », « Impact sur Legifacile », « Pistes de correction »), lu en lecture seule.
- Suivi : `docs/suivi.md:163` (ligne `T02-DUREES-ATTACHE-DANS-JOB`, classe ADR-0143, verdict de `@architecte` du 2026-11-11, chiffres 253 à 278 s et 0,02 s).
- Code : `crates/crible-store/src/attach.rs:161-168` (`AttachReport::durations`), `:240-258` (`AttachDurations`), `:249-253` (`lexical` à zéro) ; `crates/crible-server/src/jobs.rs:53-121` (`JobView`), `:217-225` (`JobOutcome`), `:377-411` et `:458-474` (rapport en mémoire, `publish_with`), `:1371-1404` (`log_attach_durations`, `millis`) ; `crates/crible-server/src/batches.rs:92-108` et `:177-201` ; `crates/crible-server/src/documents.rs:120-137` et `:181-204` ; `crates/crible-server/src/http.rs:530-545` (`accepted`) ; `crates/crible-server/src/openapi.rs:605-636` (composant `Job`) ; `crates/crible-server/src/analytics.rs:235-248` (`OpEvent`).
- Tests existants : `crates/crible-server/tests/common/mod.rs:514-566` (`assert_attach_duration_line`), `crates/crible-server/tests/routes_documents.rs:538` (ligne de journal), `crates/crible-store/tests/attacher_lot.rs:989` (`inv_10_replaying_an_applied_batch_changes_nothing`), `crates/crible-server/tests/job_progress.rs:392` (`a_replayed_batch_publishes_no_progress`).
- Conception : `docs/05-api.md:71` (rôle `write` des routes de jobs), `:344` (`timings_ms`), `:450-489` (« Jobs », dont `:482` la règle des durées), `:488` (aucune isolation par clé) ; `docs/04-donnees-et-ingestion.md:206` (idempotence, rapport sans durées), `:247` (durées de télémétrie d'appel) ; `docs/08-invariants-et-tests.md:18` (INV-10).
- ADR : ADR-0016 (point de commit unique), ADR-0042 (durées non persistées), ADR-0060 (rapport d'un patch en mémoire), ADR-0067 (`wait_timeout_ms`), ADR-0096 (journal sans texte client), ADR-0097, ADR-0103 (jobs, décisions 3, 4 et 6), ADR-0107 (décision 4, remplacée en partie), ADR-0111 (décision 5, référence des routes ; décision 7, objets fermés), ADR-0112 (client Python, `dict`), ADR-0117 (décision 5, `OpEvent`), ADR-0134 (progression en mémoire), ADR-0143 (ligne 1.x et classes de compatibilité).
