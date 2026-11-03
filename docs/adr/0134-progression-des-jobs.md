# ADR-0134 — Progression des jobs : vocabulaire, granularité et types publiés

- **Statut** : Accepté (décidé par le propriétaire le 2026-11-03) — complété par la précision du 2026-11-03 sur la décision 2 (job réussi sans étape jouée), sans modification du corps
- **Date** : 2026-11-03
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, tâche NETTOYER-P8 (passage D), sur demande de l'orchestrateur, d'après le verdict de `@architecte` et quatre questions posées au propriétaire le 2026-11-03
- **Remplace** : —

## Contexte

La progression des jobs est annoncée par `docs/05-api.md`, mais aucun code ne la publie.

**Le constat.** `JobProgress::set` (`crates/crible-server/src/jobs.rs:143-177`) n'a **aucun appelant** dans `crates/crible-server/src`. Le champ `progress` d'un job reste donc `null` pour tous les types de job. Pourtant l'exemple de `docs/05-api.md:435` portait `"progress": { "step": "commit", "done": 932, "total": 932 }`, et `docs/05-api.md:443` fixe les unités : des passages pour `attach_batch` et `online_batch`, comme `counts.passages` du manifeste, des documents pour `patch_documents`. L'écart a été relevé par `@testeur` pendant VERIFIER-ARRET-PROPRE, puis noté dans le contenu de NETTOYER-P8. Le code le documente lui-même (`crates/crible/tests/docker_stop_during_attach.rs:660-662`).

**Le verdict de `@architecte`.** ECART, avec trois points **non couverts** par la conception, et deux points déjà couverts.

Les trois points non couverts :

1. **Aucune liste d'étapes par `kind`.** Le seul nom d'étape écrit dans `docs/05-api.md` était `commit`, dans l'exemple de `:435`. Or `docs/04-donnees-et-ingestion.md:239` nomme six étapes — `validation`, `segment`, `transaction`, `lexical`, `manifest`, `snapshot` —, dans l'ordre du protocole de `docs/02-architecture.md:124-131`. ADR-0042 et `AttachDurations` (`crates/crible-store/src/attach.rs:240-258`) portent exactement ces six noms, `lexical` inséré entre `transaction` et `manifest` par ADR-0087. Un client ne peut donc pas savoir quelles valeurs de `step` attendre, ni que `commit` n'est pas un nom du protocole.
2. **La granularité n'est pas écrite.** Rien ne dit si `done` avance passage par passage, par lot, ou par étape, ni quand `total` devient connu, ni ce que porte la progression d'un job échoué.
3. **Les types publiés ne sont pas écrits.** `docs/05-api.md:443` ne nomme que `attach_batch`, `online_batch` et `patch_documents` pour les unités, sans dire si les six autres `kind` publient une progression. `attach_batch`, `online_batch` et `patch_documents` sont les seuls types qui touchent des passages ou des documents ; `patch_config`, `compact`, `export`, `snapshot`, `rebuild_lexical` et `delete_index` n'ont aucune unité écrite.

Les deux points déjà couverts, qui ne sont pas rouverts :

- **La persistance.** ADR-0103, décision 3, écrit la table `jobs` aux seules transitions d'état, chaque écriture portant la progression de l'instant ; la progression fine vit en mémoire entre deux transitions. L'alternative « écrire la progression à chaque étape dans la table » y est explicitement écartée.
- **La présence du membre.** `progress` est un membre toujours présent de l'objet `Job`, `null` quand il n'y a rien : l'objet est fermé (ADR-0111, décision 7), et l'omission d'ADR-0119, décision 3, ne vise que les membres de `per_result` (`:63` et `:82` du même ADR). Il n'y a donc rien à décider sur la forme de l'enveloppe.

**Ce qui manque encore au-delà de la décision.** L'exemple de `docs/05-api.md:435` et l'annotation `#[schema(example = "commit")]` de `crates/crible-server/src/openapi.rs:557` portent le nom `commit`. Le premier est un document, corrigé par le présent ADR ; le second est du code, à corriger par une tâche.

## Décision

**Décision 1 — vocabulaire.** `progress.step` prend les noms d'étapes de `docs/04-donnees-et-ingestion.md:239`, qui sont ceux des durées d'ADR-0042 :

- pour `attach_batch` et `online_batch` : `validation`, `segment`, `transaction`, `lexical`, `manifest`, `snapshot`, dans l'ordre du protocole (`docs/02-architecture.md:124-131`) ;
- pour `patch_documents` : `transaction`, `manifest`, `snapshot`, les étapes 3, 5 et 6 que `docs/02-architecture.md:133` donne à un patch de métadonnées.

Une étape que le protocole saute pour un job donné n'est pas publiée : une attache dont la génération ne change aucun texte ne publie pas `lexical` (ADR-0088 ; `AttachDurations::lexical` vaut zéro quand l'étape ne tourne pas, `crates/crible-store/src/attach.rs:249-252`), et un patch sans effet ne publie rien du tout, puisqu'il ne crée aucune génération (`docs/02-architecture.md:142`). Le nom `commit` disparaît de l'exemple de `docs/05-api.md:435` : l'étape 3 s'appelle `transaction`, et cet exemple montre un job `succeeded` avec `done` égal à `total`, donc la dernière étape, `snapshot`, par la décision 2.

**Décision 2 — granularité.** `done` avance aux **frontières d'étape**, pas passage par passage. `total` est connu dès la lecture du manifeste — en passages, comme `counts.passages` — ou du corps du patch — en documents. Au début de chaque étape, `step` prend son nom. À la fin de la dernière étape d'un job réussi, `done` vaut `total`.

Un job échoué garde la dernière progression publiée, éventuellement avec `done < total`, ou `null` si l'échec précède la première publication. Rien n'est remis à zéro et rien n'est effacé : la dernière valeur publiée est celle que lit `GET /v1/jobs/{job_id}`.

**Décision 3 — persistance.** La progression vit en mémoire. La route `GET /v1/jobs/{job_id}` et la liste `GET /v1/jobs` la lisent en mémoire pendant le job. La table n'est écrite qu'aux transitions d'état (ADR-0103, décision 3), et la ligne finale porte la dernière progression. Le présent ADR ne change rien à cette règle ; il la rappelle pour que le vocabulaire et la granularité ne la contredisent pas.

**Décision 4 — types publiés.** Seuls `attach_batch`, `online_batch` et `patch_documents` publient `progress`. Les autres types — `patch_config`, `compact`, `export`, `snapshot`, `rebuild_lexical`, `delete_index` — gardent `progress: null`, membre présent. Ils n'ont aucune unité écrite, et en inventer une par type demanderait un ADR.

**Décision 5 — lot en ligne.** Pendant l'encodage d'un `online_batch`, avant que le lot interne et son manifeste existent, `progress` reste `null`. Il suit ensuite les étapes de l'attache, en passages. Aucune unité nouvelle n'est introduite : l'encodage n'a pas d'étape `encode`, et le job ne change pas d'unité en cours de route.

**Ce qui n'est pas décidé ici.** Le coût de la publication, la forme exacte du verrou de `JobProgress`, et la manière de faire passer la progression à travers `crible-store` jusqu'aux étapes de l'attache : ce sont des choix d'implémentation, réversibles, qui appartiennent à la tâche. Le présent ADR ne fixe que le contrat visible.

## Conséquences

- **Positives** :
  - un client sait quelles valeurs de `step` attendre, pour chaque `kind` qui publie une progression ;
  - un seul vocabulaire pour les étapes, partagé par les durées d'ADR-0042, `docs/04` et l'API : un lecteur d'un journal et un client de l'API nomment les mêmes choses de la même façon ;
  - `done` et `total` deviennent comparables d'un job à l'autre, puisqu'ils ne bougent qu'aux frontières d'étape ;
  - l'écart entre `docs/05` et le code est nommé, et le document peut être corrigé sans toucher au code dans le même changement ;
  - aucune écriture SQLite de plus : la règle d'ADR-0103 reste vraie.
- **Négatives / coûts** :
  - la progression est grossière : un client ne voit pas une attache avancer pendant l'étape `segment`, qui peut durer longtemps sur un gros lot ;
  - `done` reste à `0` pendant les premières étapes d'un `attach_batch`, `validation` et `segment` comprises, alors que le travail est réel ;
  - six types de job gardent `progress: null`, donc un client doit déjà traiter ce cas ;
  - l'exemple de `docs/05` montre `snapshot`, et non `transaction` : un job réussi et terminé est à la fin de sa dernière étape, ce que la décision 2 impose. L'ancien exemple, avec `done` égal à `total`, était déjà dans cet état.
- **À surveiller** :
  - un test qui observe `done < total` pendant un job, puis `done == total` et `step == "snapshot"` sur un job `succeeded` ;
  - un test qui vérifie qu'une attache qui saute le lexical ne publie pas `lexical`, et qu'un patch publie `transaction`, `manifest`, `snapshot` dans cet ordre ;
  - un test qui vérifie que les six autres `kind` rendent `progress: null` ;
  - la cohérence entre `docs/05-api.md:435` et l'exemple généré de `crates/crible-server/src/openapi.rs:557` : tant que le code porte `commit`, la spécification servie et le document divergent sur la valeur d'exemple, sans que les tests de routes le voient ;
  - si un client demande un jour une progression plus fine, c'est un nouvel ADR : la granularité par étape n'est pas un détail d'implémentation réversible, puisque des clients s'y fient.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Garder `commit` pour la troisième étape | Deux vocabulaires pour la même étape : `docs/04`, ADR-0042 et `AttachDurations` disent `transaction`. Un client devrait traduire, et l'exemple du document contredirait les durées journalisées. |
| Une chaîne libre, non énumérée | Un client ne peut pas s'y fier : il ne saurait ni quelles valeurs attendre, ni quand elles changent. Une liste fermée par `kind` est ce qui rend le champ utile. |
| Une progression passage par passage | Demande un verrou par passage, ou une écriture atomique par passage, pour une information qu'aucun texte n'exige. Le coût serait payé par chaque attache, y compris celles que personne n'observe. |
| Écrire la progression dans la table à chaque étape | Déjà écarté par ADR-0103, décision 3 : des milliers d'écritures pour une information rendue en mémoire, sans contrepartie puisqu'un job interrompu est `failed` de toute façon. |
| Étendre `progress` aux six autres types | Chacun demanderait une unité à inventer — des index pour `compact`, des fichiers pour `export`, aucun pour `patch_config` —, sans besoin écrit. |
| Une étape `encode` comptée en documents pour le lot en ligne | Change l'unité en cours de job : `total` passerait des documents aux passages, et un client ne pourrait plus comparer `done` et `total` dans le temps. Le lot en ligne publie l'attache seule, après l'encodage. |
| Omettre `progress` plutôt que rendre `null` | L'objet `Job` est fermé (ADR-0111, décision 7) : un membre absent et un membre `null` se liraient différemment, alors que l'absence de progression est un état, pas une omission. |

## Complément du 2026-11-03 (précision du propriétaire sur la décision 2)

La décision 2 dit que `done` vaut `total` à la fin de la dernière étape d'un job réussi. Elle ne nommait pas le cas d'un job réussi qui n'a joué **aucune** étape. Le rejeu d'un lot déjà attaché est ce cas : `attach_batch` rend le rapport déjà appliqué quand le `batch_id` et le digest du manifeste concordent, sans jouer `validation`, `segment`, `transaction`, `lexical`, `manifest` ni `snapshot` (`crates/crible-store/src/attach.rs:585-587` ; rejeu idempotent d'ADR-0103). Aucune étape n'est donc publiée, et le job `succeeded` garde `progress: null`, membre présent (décision 4 et ADR-0111, décision 7).

**La précision, décidée par le propriétaire le 2026-11-03.** Un job réussi qui n'a joué aucune étape, comme le rejeu d'un lot déjà attaché, garde `progress: null` ; un client ne doit donc pas supposer `done = total` sur tout job réussi.

Le test `a_replayed_batch_publishes_no_progress` (`crates/crible-server/tests/job_progress.rs:392`) fige ce comportement. Le corps de l'ADR et les autres décisions ne changent pas.

## Références

- `docs/05-api.md`, section « Jobs » (`:429-451`), en particulier l'exemple `:432-440` et la puce `:443`.
- `docs/04-donnees-et-ingestion.md:239` (les six étapes de l'attache et leurs durées).
- `docs/02-architecture.md` (« Générations et point de commit » `:120-142`, ordre des étapes `:124-131`, patch de métadonnées `:133` et `:135-142`).
- `crates/crible-server/src/jobs.rs:140-187` (`JobProgress`, `set` sans appelant) ; `crates/crible-server/src/openapi.rs:552-561` (`#[schema(example = "commit")]` à `:557`).
- `crates/crible-store/src/attach.rs:230-258` (`AttachDurations`).
- `crates/crible-store/src/attach.rs:585-587` (rejeu d'un lot déjà attaché : rapport rendu sans jouer d'étape) ; `crates/crible-server/tests/job_progress.rs:392` (`a_replayed_batch_publishes_no_progress`).
- ADR-0016 (point de commit unique) ; ADR-0042 (durées d'attache, télémétrie d'appel non persistée) ; ADR-0060 (patch de métadonnées) ; ADR-0087 (`lexical` entre `transaction` et `manifest`) ; ADR-0088 (commit tantivy absent quand le texte ne change pas) ; ADR-0103 (table des jobs, décision 3) ; ADR-0105 (`kind` et corps des réponses) ; ADR-0111 (décision 7, objets fermés) ; ADR-0119 (décision 3, omission limitée à `per_result`).
- Tâche NETTOYER-P8, passage D ; écart relevé par `@testeur` pendant VERIFIER-ARRET-PROPRE.
