# ADR-0101 — Corps d'erreur : exposition du message, `details`, réponses hors route

- **Statut** : Accepté (accepté par anticipation par le propriétaire le 2026-10-29, à confirmer à la fin de la série DEMARRER-SERVEUR-AXUM, AUTHENTIFIER-CLES, FORMAT-ERREURS-ET-LIMITES)
- **Date** : 2026-10-29
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, tâche FORMAT-ERREURS-ET-LIMITES (phase P6), sur demande du propriétaire et le verdict de `@architecte` du 2026-10-29
- **Remplace** : —

## Contexte

`docs/05-api.md` fixe déjà deux choses. La section « Erreurs » (`:24-52`) donne **un format unique** — `{"error": {"code", "message", "details", "request_id"}}` (`:26-31`) — et une table de **14 codes** avec leur statut HTTP (`:35-50`). Ces 14 codes correspondent un à un aux variants de `crible_core::error::ErrorCode`, dont la constante `ALL` est déclarée « every code of `docs/05-api.md`, in the order of the table » (`crates/crible-core/src/error.rs:44-58`). La correspondance code → statut est donc **conforme** : rien à décider de ce côté.

Le serveur possède déjà la fonction qui construit le corps, `error::body` (`crates/crible-server/src/error.rs:34-48`), et celle qui rend la réponse avec le statut du code et `x-request-id` (`:50-72`). `AUTHENTIFIER-CLES` en est le premier utilisateur, pour `401` et `403` seulement (ADR-0100, décision 8), et `docs/05-api.md:16-18` promet déjà que **toutes** les réponses portent l'identifiant, y compris `404`, `405`, `413` et `500` (ADR-0098).

Sept points restent non couverts, et la tâche ne peut pas les inventer :

1. **L'exposition de `message`.** `docs/05` en donne un exemple (`:31`) sans dire d'où il vient. Le message d'une `CribleError` est écrit pour un développeur : il peut nommer un fichier, un chemin, une valeur interne.
2. **Le cas `internal`.** `docs/05:50` dit « Erreur inattendue (journalisée avec le `request_id`) », donc la cause est journalisée, mais rien ne dit ce que le client reçoit. Une erreur d'entrée-sortie porte souvent un chemin absolu du serveur.
3. **Le contenu de `details`.** Le seul exemple documenté est `{"path": "$.filter.juridicton"}` pour `invalid_filter` (`:31`). Ni la règle générale, ni la liste des clés autorisées ne sont écrites.
4. **Les réponses hors route.** Une route inconnue, une méthode non permise, un rejet d'extracteur ou une panique de gestionnaire ne passent par aucun code de la table. Aujourd'hui, le `404` d'une route inconnue sort du routeur d'axum **sans corps** : le test `a_response_before_any_route_carries_the_identifier` (`crates/crible-server/src/http.rs:258-266`) vérifie le statut et l'en-tête, jamais le corps. Un client reçoit donc un `404` qu'aucun document ne décrit, contrairement au format unique de `docs/05:26`.
5. **Le rejet d'un extracteur.** axum rend par défaut `400`, `413`, `415` ou `422` avec un corps en texte brut. `415` n'existe pas dans la table de `docs/05`, et `422` y désigne autre chose (`incompatible_model`, `acceptance_failed`, `docs/05:46-47`).
6. **Les chemins et les identifiants.** `INV-01` interdit qu'un iid sorte du moteur « ni API, ni logs, ni analytics, ni exports » (`docs/08-invariants-et-tests.md:9`). Un message d'erreur est une sortie d'API comme une autre.
7. **La journalisation des messages.** `docs/06-observabilite.md:116` et ADR-0096 interdisent le texte de requête et les valeurs de filtre **à tout niveau de log**. Or un message de `4xx` cite parfois une valeur fournie par le client — une date mal formée dans un filtre, par exemple —, qui est précisément une valeur de filtre.

Deux contraintes de plan pèsent. `ADR-0099` fixe le corps du `503` de `/ready` (`{"status": "not_ready"}`), qui n'est pas une erreur au sens de `docs/05`. `ADR-0100` (décision 8) a déjà arrêté la forme des corps `401` et `403` : le présent ADR la reprend sans la changer.

## Décision

**Décision 1 — un seul format, une seule exception.** Toute réponse d'erreur du serveur suit le format de `docs/05-api.md` (`code`, `message`, `details`, `request_id`) et porte `x-request-id` (ADR-0098). La **seule exception** est le `503` de `/ready`, dont le corps reste celui d'ADR-0099.

**Décision 2 — ce que `message` contient.** `message` est le message de la `CribleError`, sauf pour `internal`, qui rend un message **générique fixe** : « internal error; see the server log for this request_id ». Un message ne contient **jamais** d'iid (INV-01) et **jamais** de chemin absolu du système de fichiers du serveur.

**Décision 3 — ce que `details` contient.** `details` vaut `{}` par défaut. Une clé n'y figure que si `docs/05` la documente pour ce code — aujourd'hui, `path` pour `invalid_filter` seulement. Une tâche de routes qui ajoute une clé la documente dans `docs/05` dans le même changement.

**Décision 4 — journalisation.** Chaque réponse d'erreur produit une ligne de log avec le **code**, le **statut** et le **`request_id`**. Le `message` n'est journalisé que pour `internal`, comme cause du `500`. Un message de `4xx` peut citer une valeur fournie par le client, et n'est donc jamais journalisé (`docs/06`, ADR-0096).

**Décision 5 — hors route.** Une **route inconnue** et une **méthode non permise sur une route connue** rendent toutes deux `404 not_found`, avec le message `no route for <METHOD> <path>`, où `<METHOD>` est la méthode HTTP reçue et `<path>` le chemin demandé. Il n'y a **pas de `405`** : la ligne `not_found` de `docs/05` est élargie à « route inconnue ».

**Décision 6 — rejet d'extracteur.** Un JSON mal formé, un type de contenu absent ou faux, ou un corps illisible rendent `400 invalid_request`. Un corps au-delà de la limite rend `413 payload_too_large` (ADR-0102). Les statuts propres d'axum (`415`, `422`) ne sortent jamais.

**Décision 7 — panique.** Une panique de gestionnaire est rattrapée et rend `500 internal` au format de la décision 1, avec le message générique de la décision 2. La cause (le message de panique) part dans le log, sous le `request_id`.

**Décision 8 — `401` et `403`.** Les corps fixés par ADR-0100 passent par la même fonction. Leur forme ne change pas.

Ce qui n'est décidé ici : le libellé exact de chaque message, qui reste celui de `docs/05` et des tâches de routes ; la langue des messages, fixée en anglais par ADR-0015 ; l'ordre des clés dans le JSON ; la présence d'un champ de plus dans `details`, qui demande la décision 3.

## Conséquences

- **Positives** :
  - un client traite toute erreur avec un seul analyseur : le code, le statut et l'identifiant suffisent, y compris pour un `404` de route inconnue ;
  - aucune fuite par le corps : ni iid, ni chemin du serveur, ni cause interne, et le `500` reste diagnosticable par le `request_id` et le log ;
  - un rejet d'extracteur ne produit plus de statut absent de la table, donc la correspondance 1:1 entre `ErrorCode` et `docs/05` reste vraie ;
  - une valeur de filtre citée dans un message de `4xx` ne part jamais dans les logs, ce qui garde INV-13 et ADR-0096 vrais à tout niveau ;
  - une panique ne coupe pas la réponse : le client reçoit un `500` traçable au lieu d'une connexion fermée.
- **Négatives / coûts** :
  - le message d'un `500` perd sa cause côté client : un opérateur doit lire le log avec le `request_id`, et un client qui ne le transmet pas complique le diagnostic ;
  - la couche `catch-panic` ajoute un coût par requête, faible, et ne rattrape rien si le profil de compilation passe à `panic = "abort"` ;
  - un `404` de route inconnue porte un corps JSON alors qu'aucun client ne le lit : la réponse est un peu plus lourde ;
  - pas de `405` prive un client d'un diagnostic précis sur la méthode ; le message « no route for … » le lui rend en texte ;
  - les tâches de routes doivent désormais documenter toute clé de `details` dans `docs/05`, donc un changement de code touche un document.
- **À surveiller** :
  - un test qui cherche un iid et un chemin absolu du serveur dans **toutes** les réponses d'erreur, `500` compris, doit rester vert ;
  - un test qui vérifie qu'un message de `invalid_filter` citant une valeur de filtre n'apparaît pas dans les logs produits avec `RUST_LOG=trace` ;
  - `docs/05-api.md:18` cite encore `405` dans la liste des statuts qui portent `x-request-id` (ADR-0098). La mention reste sans effet : aucun `405` n'est rendu. Si un jour une route en rend un, la décision 5 est à rouvrir ;
  - la fidélité des corps `401` et `403` d'ADR-0100 : une divergence entre les deux tâches casserait la promesse d'un format unique.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Exposer tel quel le message de toute `CribleError`, `internal` compris | Une erreur d'entrée-sortie nomme un chemin absolu du serveur, et une erreur interne nomme des structures : c'est une fuite d'implémentation, et parfois une information d'exploitation. |
| Un message vide pour tous les codes | Un client perd le diagnostic des `4xx`, qui sont justement ceux qu'il peut corriger lui-même. |
| Journaliser tous les messages | Un message de `4xx` cite des valeurs fournies par le client, donc des valeurs de filtre : c'est exactement ce que `docs/06:116` et ADR-0096 interdisent. |
| Laisser `details` libre, rempli par chaque route | Le contenu devient une seconde API non documentée, où un iid ou un chemin peut entrer sans revue. |
| `405` pour une méthode non permise | Ajoute un cas de plus à tester et un statut que la table ne portait pas ; le gain pour le client est un mot, que le message donne déjà. |
| Laisser le `404` d'axum sans corps | Contredit le format unique de `docs/05` et le test d'ADR-0098 ne couvrirait que l'en-tête d'une réponse non documentée. |
| Laisser passer les rejets d'axum (`400`, `413`, `415`, `422` en texte brut) | `415` n'existe pas dans la table, `422` y a un autre sens, et le corps ne suit pas le format : trois contrats différents pour une même famille de fautes. |
| Un `400` pour une méthode non permise | Confond « la requête est mal formée » et « cette route n'existe pas pour cette méthode » ; le `404` est la réponse que le routeur peut produire sans connaître la ressource. |
| Un corps `500` sans message du tout | Le format de `docs/05` nomme `message` comme clé obligatoire ; un message générique fixe garde la forme sans rien révéler. |

## Références

`docs/05-api.md` (conventions `:5-22`, section « Erreurs » `:24-52`) ; `docs/06-observabilite.md` (« Santé et logs » `:111-116`, INV-13) ; `docs/08-invariants-et-tests.md` (`INV-01` `:9`, `INV-13` `:21`) ; `crates/crible-core/src/error.rs` (`ErrorCode`, `:12-41`, `ALL`, `:44-58`) ; `crates/crible-server/src/error.rs` (`body`, `:34-48`, `response`, `:50-72`) ; `crates/crible-server/src/http.rs` (couches communes `:63-90`, route inconnue `:258-266`) ; fiche `FORMAT-ERREURS-ET-LIMITES` (`docs/phases/P6-serveur-et-api.md:43-52`) ; ADR-0015 (API en anglais) ; ADR-0038 (`overloaded` pour une contention SQLite) ; ADR-0095 (test sur routeur de test) ; ADR-0096 (logs sans contenu) ; ADR-0098 (`x-request-id`) ; ADR-0099 (corps du `503` de `/ready`) ; ADR-0100 (corps `401` et `403`) ; ADR-0102 (limites et `413`).
