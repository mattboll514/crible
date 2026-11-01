# ADR-0122 — Journal de debug : activation, capture, file, vidage, purge, route et preuves

- **Statut** : Accepté (accepté par anticipation par le propriétaire le 2026-10-31, confirmé par le propriétaire le 2026-11-01 ; délégation du propriétaire pour la série de P7 ; les décisions 1 et 2, prises par l'orchestrateur, sont confirmées par le propriétaire après explication, avec la consigne d'exploitation écrite dans l'index)
- **Date** : 2026-10-31
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, tâche JOURNAL-DE-DEBUG (phase P7), sur les options recommandées par `@architecte` et retenues par l'orchestrateur, en application de la délégation du propriétaire du 2026-10-31 pour la série de P7
- **Remplace** : — (ne remplace aucune décision : complète ADR-0118, décisions 7 et 8, et ADR-0120, décision 4, sans modifier le corps d'aucun ADR accepté)

## Contexte

### La fiche et ce qu'elle demande

La fiche `JOURNAL-DE-DEBUG` (`docs/phases/P7-observabilite.md:55-72`) demande `debug.sqlite` (`:60`), un enregistrement selon `debug` et `sample_rate` (`:61`), une écriture asynchrone (`:62`), la purge selon `ttl_days` (`:63`), `GET /v1/debug/queries` réservé à l'admin avec ses filtres (`:64`), le test `inv_17_*` du journal (`:65`) et le rejeu de MES-16 par `@mesureur` (`:66`).

Ses critères sont : aucun fichier écrit quand le journal est désactivé (`:68`) ; la purge supprime les entrées expirées (`:69`) ; un rôle `read` reçoit `403` (`:70`) ; la route figure dans la spécification OpenAPI et son exception nommée quitte le test des routes dans le même changement (`:71`) ; le test `inv_17_*` est vert et le rapport de MES-16 tient le critère d'ADR-0113 (`:72`). La fiche cite INV-17 (`:73`).

Cet ADR traite l'**exécution** ; le **stockage** (fichier, schéma, payload, protections) est l'objet d'ADR-0121.

### Ce que la conception pose

`docs/06:78` fixe la configuration : `debug_journal.enabled` (false par défaut), `sample_rate` (0 par défaut), `ttl_days` (7 par défaut). `docs/06:79` dit ce qui est enregistré : « une requête avec `debug: true`, si le journal est activé, et une fraction `sample_rate` des autres requêtes ». `docs/05:270` dit la même chose en une phrase : `debug` « alimente le journal de debug quand il est activé ».

`docs/06:87` fixe l'écriture : asynchrone, comme les analytics, avec une purge quotidienne ; INV-17 couvre les deux écritures, et le journal est prouvé « sur le même modèle que les analytics (ADR-0118, décisions 7 et 8) ». `docs/07:89-92` porte les trois clés ; `docs/07:121` place le vidage des analytics dans la séquence d'arrêt, après le drainage des requêtes, borné à 2 s (ADR-0118, décision 3).

Les ADR voisins bornent le reste. ADR-0118, décision 1, a retenu un canal `tokio::mpsc` borné et un seul écrivain, sans verrou sur le chemin d'une requête ; sa décision 2 a fixé la capacité à 4 096 et le compteur de pertes par processus ; sa décision 3 a fixé le vidage toutes les 10 s et à l'arrêt, borné à 2 s ; sa décision 7 a posé le protocole de preuve d'INV-17 sous `failpoints` ; sa décision 8 a nommé JOURNAL-DE-DEBUG comme porteur de la seconde campagne MES-16. ADR-0113, décision 5, fixe le protocole de cette campagne. ADR-0119 a créé `DebugCapture { top_n }`, construite seulement si `debug` est demandé, et sa décision 13 a renvoyé l'écriture du journal à cette tâche. ADR-0120, décision 4, a consigné la dette du multi-index : `search_multi.rs` passe `capture: None` (`:589-607`), et JOURNAL-DE-DEBUG doit choisir entre une capture par cible et un journal sans capture.

### L'état du code au moment de la rédaction

`crates/crible-server/src/search.rs:512-527` définit `Attempt` avec le résultat, les étapes mesurées, le total optionnel, la stratégie, la taille du pool et les contributions ; **il ne porte pas de capture**. `:586` construit `DebugCapture::new(crible_search::debug::DEFAULT_TOP_N)` seulement quand `debug` est vrai, et la passe au pipeline par `capture.as_mut()` (`:600`). `crates/crible-server/src/search_multi.rs:589-607` passe `capture: None` pour chaque cible. `crates/crible-server/src/debug.rs:160-178` porte `SearchDebug`, qui rend `plan`, `timings_ms` et `per_result`.

`crates/crible-server/src/config.rs:331-337` valide déjà `sample_rate` dans `[0, 1]`, et refuse une valeur hors domaine au démarrage. `crates/crible-server/src/auth.rs:120` montre l'usage de `getrandom::fill` pour un tirage aléatoire, sans crate nouvelle (ADR-0100).

### Les deux écarts que l'orchestrateur tranche

**Écart 1 — l'activation.** La fiche écrit « un enregistrement si `debug: true` et que le journal est activé, et selon `sample_rate` » (`docs/phases/P7:61`). La conjonction se lit de deux façons : (a) `debug: true` **et** un tirage `sample_rate` réussi ; (b) deux voies, `debug: true` **toujours** quand le journal est activé, et `sample_rate` pour les autres. `docs/06:79` et `docs/05:270` disent (b). L'orchestrateur tranche (b) au titre de la délégation ; c'est la **décision 1** ci-dessous, et le propriétaire la reverra en priorité.

**Écart 2 — le rôle.** Ni la fiche ni `docs/06` ne restreignent `debug: true` à un rôle. `docs/05:270` place `debug` dans le corps de recherche, et les deux routes de recherche sont ouvertes à `read` (`docs/05:70-71`) ; la lecture du journal, elle, est `admin` (`docs/05:76`). L'orchestrateur décide qu'aucune restriction de rôle ne s'ajoute à `debug: true` ; c'est la **décision 2**, et le propriétaire la reverra en priorité.

## Décision

### Décision 1 — activation à deux voies (DÉCISION DE L'ORCHESTRATEUR, délégation du 2026-10-31)

**Décision.** La configuration est celle de `docs/07:89-92` : `debug_journal.enabled` (false par défaut), `sample_rate` (0 par défaut, dans `[0, 1]`), `ttl_days` (7 par défaut). L'enregistrement suit **deux voies** :

1. une requête qui porte `debug: true` est **toujours** enregistrée quand le journal est activé, quel que soit `sample_rate` ;
2. `sample_rate` ne vise que les **autres** requêtes : une fraction tirée au hasard de celles qui ne demandent pas `debug` est enregistrée.

Précisions :

- le tirage passe par une fonction **pure** `should_sample(rate, u64)`, testable sans aléa ; l'entier est fourni par `getrandom::fill`, déjà utilisé par `crates/crible-server/src/auth.rs:120` (ADR-0100). Aucune crate nouvelle ;
- le tirage a lieu **avant la prise du jeton de concurrence**, donc il ne consomme pas le jeton, n'allonge pas `took_ms` et ne dépend pas du mode ni du corps de la requête ;
- `sample_rate` est validé dans `[0, 1]` par `crates/crible-server/src/config.rs:331-337`, au démarrage, avec le mécanisme d'ADR-0097 ; une valeur hors domaine fait échouer le démarrage. `0.0` n'échantillonne aucune requête ordinaire, `1.0` les échantillonne toutes ;
- un échec de la source aléatoire ne fait pas échouer la requête : elle est traitée comme **non échantillonnée**, et la recherche continue. Une erreur d'aléa ne doit pas produire un `5xx` ni un `panic!` (règle 10 d'`AGENTS.md`) ;
- `debug: true` est lu dans le corps de la route de recherche, et le membre `debug` de la réponse reste conditionné au seul `debug` (ADR-0119, décision 10 ; décision 3 ci-dessous) ;
- la fiche est adaptée : son livrable d'enregistrement dit les deux voies, et son critère d'acceptation ajoute qu'avec `sample_rate = 0`, une requête `debug: true` produit une ligne et une requête sans `debug` n'en produit aucune.

### Décision 2 — rôle de `debug: true` (DÉCISION DE L'ORCHESTRATEUR, délégation du 2026-10-31)

**Décision.** Aucune restriction de rôle ne s'ajoute à `debug: true` : toute clé valide peut demander `debug` et faire enregistrer **sa propre** requête dans le journal quand il est activé. Une clé `read` peut donc écrire son texte au journal, et elle ne peut pas le relire.

Précisions :

- l'authentification reste celle d'ADR-0100 : une clé absente, mal formée ou inconnue rend `401` et n'écrit rien ; le rôle n'entre pas dans la décision d'enregistrer ;
- une clé `read` ne voit que ses propres requêtes par le journal, et seulement si elle a un moyen de les relire — ce qui n'est pas le cas, la route étant `admin`. Elle ne peut ni lire le journal, ni lire les requêtes d'une autre clé ;
- les garde-fous sont ceux d'ADR-0121, décision 6 : journal **désactivé par défaut**, `ttl_days` court (7 par défaut), lecture réservée à `admin`, fichier absent quand le journal est désactivé, aucun contenu dans les logs ;
- cette décision ne donne aucun droit nouveau sur les résultats : `debug: true` ajoute le plan, les temps et les rangs d'une requête à laquelle la clé a déjà droit (ADR-0119, décision 10) ;
- le propriétaire reverra cette décision en priorité, avec la décision 1.

### Décision 3 — capture : `Attempt` porte `Option<DebugCapture>`, construite si `debug` ou si la requête est échantillonnée

**Décision.** `Attempt` (`crates/crible-server/src/search.rs:512-527`) reçoit un membre `capture: Option<DebugCapture>`. La capture est construite quand la requête porte `debug: true` **ou** quand elle a été échantillonnée ; elle est ensuite consommée par l'écrivain du journal. Le membre `debug` de la réponse reste conditionné au seul `debug`.

Précisions :

- la capture est celle d'ADR-0119, décision 8 : `DebugCapture::new(DEFAULT_TOP_N)`, avec `DEFAULT_TOP_N = 20` (`crates/crible-search/src/debug.rs:38`), et au plus 20 entrées `(key, rank, score)` par branche ;
- une requête seulement échantillonnée, sans `debug`, construit la capture pour le journal mais **ne rend aucun membre `debug`** : la règle d'ADR-0119, décision 10, est inchangée, et `debug` n'a toujours aucun effet sur les résultats, l'ordre, `k` ou `took_ms` ;
- la capture est **possédée** par `Attempt` après le calcul, et non plus seulement empruntée par le pipeline : la route peut ainsi la remettre à la file du journal une fois la réponse formée, sans relire le pipeline ;
- **en multi-index**, la boucle de `search_multi.rs` construit **une `DebugCapture` par cible** et la passe à `search_index`, au lieu de `capture: None` (`:589-607`). Cela solde la dette consignée d'ADR-0120, décision 4, sans modifier le pipeline de `crible-search` : le type, le constructeur et le point d'appel existent déjà ;
- une cible qui échoue avant sa capture n'en produit pas ; une cible qui n'a pas tourné n'entre pas dans le payload (ADR-0121, décision 4) ;
- la capture n'est **pas** la source des rangs de `per_result` : elle sert au journal, et `per_result` peut citer un rang supérieur à 20 (ADR-0119, décision 8) ;
- le coût est borné par `top_n` et par le nombre de branches, pas par la taille du pool. Une requête non échantillonnée et sans `debug` ne paie rien : le `Option` reste `None`.

### Décision 4 — multi-index : une ligne par requête, `targets[]`, `index` à `NULL`

**Décision.** Une requête multi-index écrit **une seule ligne**, avec `index` à `NULL` et un payload qui porte `targets[]`. Le filtre `index` de la route s'applique **par appartenance** : la ligne multi-index est retenue si le nom demandé figure dans ses cibles.

Précisions :

- **une ligne par cible réécrirait N fois le texte et le filtre de la requête**, alors que le texte est commun et que seule une partie du travail est propre à la cible. La duplication gonflerait le fichier et multiplierait par N l'exposition d'un même texte sensible, pour un gain de lecture nul : la route peut filtrer par appartenance ;
- `targets[]` porte, dans l'ordre de la requête, `index`, `filter`, `generation` et `capture` de chaque cible (ADR-0121, décision 4 ; ADR-0110). Le reste de `request` est commun ;
- `"index"` reste `NULL` même quand la requête multi-index ne porte qu'une cible : la colonne dit **quelle route** a écrit la ligne, pas combien de cibles elle avait ;
- le filtre `index` de la route est donc exact sur la colonne pour les lignes simples, et par appartenance dans `payload.targets[]` pour les lignes multi-index. L'index SQL sur `"index"` ne sert qu'aux lignes simples ; c'est un coût assumé, et la pagination borne la lecture ;
- le payload d'une requête multi-index peut être plus gros qu'un payload simple : la file compte des **requêtes**, pas des octets, et la borne de 4 096 de la décision 6 s'entend en requêtes.

### Décision 5 — échecs : statut et code enregistrés après la clé, `401`/`403` exclus

**Décision.** Quand la condition d'enregistrement de la décision 1 est remplie, toute erreur survenue **après la résolution de la clé** est enregistrée, avec son statut et son code : la colonne `status` porte `ok` ou le code d'erreur de l'API, et `payload.error` porte le code seul. Les `401` et les `403` sont **exclus**. Un `503` ou un `504` porte les étapes déjà mesurées, **sans plan ni capture**.

Précisions :

- `401` n'a pas de clé résolue, donc pas de dimension `client` ; `403` n'a exécuté aucune opération. C'est la règle d'ADR-0118, décision 6, appliquée au journal : compter un refus d'autorisation comme un usage gonflerait le trafic d'un client mal configuré ;
- un `400`, un `404`, un `413`, un `422`, un `500` et un `503` **produisent une ligne** quand la condition d'enregistrement de la décision 1 est remplie : une requête refusée est une requête tentée, et le statut est là pour la compter ;
- `payload.error` ne porte que `{ "code": "…" }`. Ni message, ni `details`, ni `request_id` : le message d'un `internal` peut nommer un chemin du serveur (ADR-0101), et la ligne est conservée sept jours ;
- quand le corps n'a pas pu être analysé, `request` est absent du payload ; les membres disponibles seulement sont écrits, jamais `null` (ADR-0119, décision 3) ;
- un `503` pris au jeton de concurrence n'a **aucune étape mesurée** : sa ligne porte le statut, le code, et un payload sans plan ni capture ; `total_ms` reste `NULL` (ADR-0121, décision 3) ;
- un `504` rendu pendant le calcul enregistre les étapes **déjà mesurées** au moment où la route rend l'erreur, et **n'attend pas** la fin du calcul qui continue (ADR-0109 : le jeton reste tenu jusqu'à la fin réelle). Il ne porte ni plan ni capture, et `total_ms` reste `NULL` ;
- une ligne d'échec ne contient aucun `iid` (INV-01) : le code et les étapes suffisent.

### Décision 6 — file bornée de 4 096, `try_send`, perte du neuf, `debug_journal_dropped`

**Décision.** La requête dépose sa ligne dans un canal **`tokio::mpsc` borné à 4 096 entrées**, par `try_send`, et **un seul écrivain** vide. Quand la file est pleine, la **ligne neuve est perdue** et incrémente un compteur `AtomicU64` **`debug_journal_dropped`**, par processus.

Précisions :

- la capacité est une **constante**, pas une clé de configuration (règle 12 d'`AGENTS.md`), sur le précédent exact d'ADR-0118, décision 2 ;
- `try_send` ne rend jamais la main en attente : file pleine, il rend `Full` immédiatement. Aucune requête ne tient de verrou et ne partage de structure avec l'écrivain : INV-17 est vrai **par construction** ;
- la sémantique est celle de `try_send` : la ligne refusée est la plus récente, les plus anciennes sont déjà dans la file et seront écrites. Perdre le neuf évite de réordonner les lignes ;
- `debug_journal_dropped` est **par processus**, jamais persisté : il mesure l'état d'une instance en service, pas une histoire. Un redémarrage le remet à zéro ;
- à **chaque vidage où le compteur a augmenté**, l'écrivain journalise une ligne `warn` avec le nombre de pertes depuis le vidage précédent et le total depuis le démarrage. Aucun contenu n'y figure (décision 10) ;
- le compteur est **exposé par `GET /v1/debug/queries`** (décision 9) sous la clé `dropped`, le nom `debug_journal_dropped` restant celui de la métrique dans le code et les logs. Une perte silencieuse serait un trou dans les chiffres que personne ne verrait ;
- une perte de file **n'incrémente pas** un second compteur et ne crée pas de ligne dans `queries` : une ligne perdue n'est pas une requête, et mélanger les deux rendrait le journal incompréhensible.

### Décision 7 — vidage toutes les 10 s et à l'arrêt, après les analytics, borne de 2 s

**Décision.** L'écrivain vide la file dans `debug.sqlite` **toutes les 10 secondes** et **à l'arrêt**. La file est **fermée avant le drainage** ; le journal est vidé **après les analytics** ; le vidage d'arrêt est **borné à 2 secondes**. Un échec est journalisé en `warn`, **sans reprise**.

Précisions :

- la séquence d'arrêt est celle d'ADR-0099 et `docs/07:121` : `/ready` à `503`, arrêt des nouvelles requêtes, drainage des requêtes en vol, puis vidage. Le journal vient **après** les analytics, parce que les analytics sont toujours actives et que le journal est le diagnostic : si le budget d'arrêt manque, c'est le journal qui doit céder, pas les chiffres d'usage ;
- la file est fermée **avant** le drainage du journal : plus aucune requête n'y dépose, et l'écrivain écrit tout ce qui reste, puis s'arrête ;
- le vidage d'arrêt, écriture comprise, est borné par `tokio::time::timeout` à **2 secondes** ; les vidages périodiques ne sont pas bornés, puisque rien n'attend leur fin. Les deux bornes — analytics puis journal — s'ajoutent à la séquence d'arrêt, et `stop_grace_period` devra en tenir compte en P8 ;
- un échec de vidage est journalisé en `warn` avec l'erreur et le nombre de lignes perdues, **sans reprise** : les lignes de ce vidage sont perdues et l'instance continue. Une boucle de reprise sur un disque plein empêcherait l'arrêt et remplirait le journal, comme ADR-0118, décision 3, l'a décidé pour les analytics ;
- cette perte **n'incrémente pas** `debug_journal_dropped`, qui compte les refus de file (décision 6) : deux causes de perte, deux endroits pour les lire ;
- un vidage est **une transaction** : un échec au milieu ne laisse pas la moitié d'un lot écrit. Le payload étant du texte, l'insertion est directe, sans addition de BLOB ;
- l'écriture périodique ne bloque jamais une requête ; la preuve est la décision 11.

### Décision 8 — purge à l'ouverture puis toutes les 24 h, `ttl_days = 0` refusé

**Décision.** La purge supprime les lignes plus anciennes que `ttl_days`. Elle tourne **à l'ouverture**, puis **au premier vidage dont la dernière purge date de plus de 24 heures**. `ttl_days = 0` est **refusé au démarrage**.

Précisions :

- aucun minuteur dédié n'est créé : la tâche de vidage est déjà périodique, et la purge conditionnelle ne coûte qu'une comparaison par vidage, comme ADR-0118, décision 4 ;
- la date de coupure est lue par SQLite, par `strftime('%Y-%m-%dT%H:%M:%fZ','now','-<ttl_days> days')`, et comparée à `created_at` : une seule règle d'horloge pour tout le fichier, et aucune horloge dans le code ;
- `ttl_days = 0` supprimerait **toutes** les lignes à chaque purge : le journal serait activé, écrirait, puis se viderait, et l'activation serait un mensonge. Le refus est celui d'une valeur invalide au démarrage, par le mécanisme d'ADR-0097 : message qui nomme `debug_journal.ttl_days`, code de sortie non nul, aucun démarrage sur une valeur qui ne peut pas fonctionner ;
- un échec de purge est journalisé en `warn` et n'empêche ni les vidages suivants ni l'arrêt ;
- le seuil de 24 h est une **constante**, pas une clé : `docs/07:89-92` ne déclare que `enabled`, `sample_rate` et `ttl_days`.

### Décision 9 — `GET /v1/debug/queries` : rôle `admin`, paramètres, réponse, `400`

**Décision.** La route `GET /v1/debug/queries` est réservée au rôle **`admin`** : une clé `read` ou `write` reçoit `403 forbidden` (ADR-0095). Elle lit `debug.sqlite` par `crible-store` et rend les lignes dans l'ordre `created_at DESC, id DESC`.

**Paramètres.**

| Paramètre | Type | Défaut | Rôle |
|---|---|---|---|
| `index` | chaîne | — | nom exact pour une ligne simple ; appartenance à `payload.targets[]` pour une ligne multi-index |
| `min_total_ms` | nombre | — | garde les lignes dont `total_ms` est supérieur ou égal à la valeur ; une ligne à `total_ms` `NULL` est exclue |
| `from` | chaîne ISO 8601 | — | borne basse **incluse** sur `created_at` |
| `to` | chaîne ISO 8601 | — | borne haute **incluse** sur `created_at` |
| `limit` | entier | 100 | nombre de lignes, de 1 à 1 000 |
| `before_id` | entier | — | pagination par clé : ne rend que les lignes d'`id` strictement inférieur |

**Réponse.**

```json
{
  "enabled": true,
  "dropped": 0,
  "queries": [
    {
      "id": 42,
      "created_at": "2026-10-31T10:00:00.123Z",
      "request_id": "…",
      "index": "decisions",
      "client": "legifacile-mcp",
      "mode": "hybrid",
      "status": "ok",
      "total_ms": 41.7,
      "payload": {
        "request": { "query": "responsabilité du fait des choses", "filter": { "fonds": "LEGI" } },
        "plan": { "…": "…" },
        "timings_ms": { "…": "…" },
        "capture": { "dense": [ { "key": "LEGIARTI…#0", "rank": 1, "score": 0.72 } ] },
        "generation": 1287
      }
    }
  ]
}
```

Précisions :

- `enabled` reflète `debug_journal.enabled` ; `dropped` est le compteur de la décision 6 ; `queries` est la page demandée, et chaque entrée porte les colonnes d'ADR-0121 et son `payload` **analysé en objet JSON**, jamais rendu comme une chaîne ;
- quand le journal est **désactivé**, la route rend `200` avec `{ "enabled": false, "dropped": 0, "queries": [] }`, **sans ouvrir le fichier** : un client n'a pas à traiter un `404` pour un journal simplement éteint ;
- en multi-index, `index` vaut `null` et `payload.targets[]` porte les cibles ; le filtre `index` retient la ligne par appartenance (décision 4) ;
- `limit` hors de `[1, 1_000]`, `before_id` négatif, une date qui n'est pas ISO 8601, un `min_total_ms` négatif ou non numérique, et tout **paramètre inconnu** rendent `400 invalid_request`, avec un message qui **nomme le paramètre** et son chemin. C'est la règle d'ADR-0109 pour les champs inconnus, appliquée aux paramètres de requête ;
- la pagination est **keyset** sur `id`, pas un `OFFSET` : les lignes neuves arrivent en tête, et un décalage ferait sauter ou répéter des lignes entre deux pages. Le client reprend le dernier `id` reçu ;
- l'ordre `created_at DESC, id DESC` est **total** : deux lignes du même vidage partagent `created_at`, et `id` les départage ;
- un `payload` illisible est une erreur `internal` de la route, journalisée avec le `request_id` ; il n'est jamais rendu comme une chaîne opaque ;
- le composant OpenAPI de la liste est déclaré dans le même changement, et l'**exception nommée** `GET /v1/debug/queries` est retirée du test des routes de PUBLIER-OPENAPI dans ce même changement (ADR-0111, complément du 2026-10-30 ; `docs/05:80`). Aucune exception ne survit à la tâche qui monte la route.

### Décision 10 — logs : aucun contenu, seulement des `warn` de comptage

**Décision.** Le journal n'écrit **jamais** dans les logs le `query`, le `filter`, la clé, le nom de la clé, le payload ou un extrait de l'un d'eux, **à aucun niveau** et quel que soit `RUST_LOG`. Seules des lignes `warn` de comptage sont permises : pertes de file, échec de vidage, échec de purge.

Précisions :

- c'est la règle d'ADR-0096 et de `docs/06:128-129`, étendue au journal : la confidentialité ne dépend pas du niveau de log ;
- aucune ligne `info` par requête enregistrée : le journal est le lieu du contenu, et un log d'accès ne doit pas en devenir une copie ;
- les lignes `warn` portent des **nombres** et l'erreur technique, jamais un texte de requête ni une valeur de filtre. Une erreur SQLite peut nommer le fichier ; elle ne nomme pas une ligne ;
- la route d'erreur d'ADR-0101 continue de journaliser le code, le statut et le `request_id` ; elle ne journalise pas le corps de la requête.

### Décision 11 — preuve d'INV-17 : `inv_17_debug_journal_never_blocks`

**Décision.** INV-17 est prouvé par un test déterministe de `crible-server`, nommé **`inv_17_debug_journal_never_blocks`**, sous la feature `failpoints`, sur le modèle d'ADR-0118, décision 7, avec une **barrière propre** `debug-journal-flush`, distincte d'`analytics-flush`.

**Protocole du test.**

1. Le test arme la barrière à un point **en tête de la boucle de l'écrivain du journal, avant qu'il ne reçoive** : tant qu'elle n'est pas relâchée, l'écrivain n'enlève rien de la file et n'écrit rien.
2. Le test envoie une recherche de référence avec `debug: true`, et garde son corps.
3. Il envoie `QUEUE_CAPACITY + m` recherches avec `debug: true`, journal activé. Les `QUEUE_CAPACITY` premières sont acceptées, les `m` suivantes sont refusées par `try_send` et comptées.
4. Assertions, toutes indépendantes du temps :
   - **chaque recherche rend `200`**, et son corps porte les **mêmes résultats, le même ordre, les mêmes scores et la même `generation`** que la référence ; `took_ms` et `request_id`, propres à chaque requête, sont exclus de la comparaison ;
   - `debug_journal_dropped` a monté de **exactement `m`** : ni plus, ni moins ;
   - le test relâche la barrière, attend le vidage, puis lit `debug.sqlite` : le nombre de lignes vaut exactement `QUEUE_CAPACITY`, et chaque payload porte les membres attendus ;
   - une perte annoncée par `debug_journal_dropped` **n'apparaît pas** dans les lignes : les deux mesures se contrôlent l'une l'autre ;
   - aucun `iid` ni clé d'API n'apparaît dans les lignes lues.
5. Aucun seuil de latence n'entre dans le test : la latence d'un runner n'est pas reproductible (ADR-0113, `:94`). Le test prouve que la recherche **se termine** et rend la même réponse quand l'écriture est retenue, pas qu'elle est rapide.

Précisions :

- la barrière suit le protocole de `crates/crible-server/src/barriers.rs` (`CRIBLE_TEST_BARRIER`, `ready-<scope>`, `go-<scope>`, attente bornée, build normal sans barrière armée) ;
- `QUEUE_CAPACITY` est la constante de la décision 6, lue depuis le module, jamais recopiée : un test qui recopie 4 096 cesse de prouver la bonne chose au premier changement ;
- le test vit dans `crates/crible-server/tests/`, sous `cfg(feature = "failpoints")`, et rejoint l'étape « Test the server failpoint build » de la CI (ADR-0068, complément du 2026-10-29) ;
- la portée `debug-journal-flush` est distincte d'`analytics-flush` pour que les deux tests puissent tourner sans se bloquer l'un l'autre ; une barrière armée pour l'un ne doit pas retenir l'autre ;
- le test ne remplace pas `TESTER-ANALYTICS-SANS-CONTENU` : le journal **contient** le texte par conception, et un contrôle d'INV-13 sur `debug.sqlite` serait faux.

### Décision 12 — MES-16 : rejeu par `@mesureur` avec analytics et journal actifs

**Décision.** Le rejeu de MES-16 qu'ADR-0118, décision 8, a confié à JOURNAL-DE-DEBUG a lieu **après le `@dev` de la tâche**, par `@mesureur`, avec le protocole d'ADR-0113, décision 5 : `--release`, index `external` à 1 024 dimensions, 50 000 passages de base, 5 lots de 2 000, une compaction, 200 requêtes `dense` et `hybride`, N = 4 clients en boucle fermée sous le nombre de cœurs, échauffement de 30 s, repos de 60 s, attaches puis compaction, **trois campagnes** sur des données neuves.

Précisions :

- `[analytics]` reste à ses défauts ; `[debug_journal]` est activé avec `enabled = true` et `sample_rate = 1.0`, le **pire cas**, et ce taux est rapporté dans le rapport. `sample_rate = 1.0` enregistre aussi les requêtes sans `debug`, donc chaque requête écrit une ligne : c'est la charge d'écriture maximale que la décision 1 permet ;
- le nombre de cœurs de la machine est rapporté, comme le demande ADR-0113 ; le harnais de `tools/bench/mes16/` est réutilisé, et `@dev` l'étend seulement s'il ne sait pas activer le journal, sans changer le protocole ;
- le critère est celui d'ADR-0113, décision 1 : **chaque recherche sous 3 × le p99 du repos**, et le p99 de la phase d'écriture est aussi rapporté ; **aucun `5xx`**, `503` compris, ne fait échouer une recherche ;
- les écritures sont soumises une à une : tout `503` d'écriture est compté, son `Retry-After` relevé et expliqué, sans faire échouer la campagne (ADR-0113, décision 6) ;
- le rapport est `docs/mesures/MES-16-charge-concurrente-analytics-et-journal.md`, avec les données brutes en **JSON** à côté, selon `docs/mesures/README.md` ; il cite la campagne de P6 comme référence ;
- un dépassement du critère met la tâche en `BLOQUE` et le rapport analyse la cause avant toute décision du propriétaire ; le critère n'est jamais relâché (ADR-0113, décision 10).

### Décision 13 — aucune dépendance nouvelle

**Décision.** Cet ADR n'ajoute aucune dépendance à `docs/07`. Le canal vient de la feature `sync` de `tokio`, déjà activée (`docs/07:31`, ADR-0096) ; le tirage vient de `getrandom`, déjà déclaré pour les clés (ADR-0100) ; le payload vient de `serde_json`, déjà présent (ADR-0027).

Précisions :

- aucune `DashMap` : la variante « verrou court ou `DashMap` » de `docs/06:47` est écartée par ADR-0118, décision 1, et le journal suit le même mécanisme ;
- aucune crate d'horloge : `created_at` est lu par SQLite (ADR-0121, décision 3) ;
- aucun crate d'analyse JSON : `serde_json` lit le payload de la route.

## Conséquences

- **Positives** :
  - les deux voies d'activation sont celles de `docs/06:79` et de `docs/05:270` : une requête `debug: true` est toujours enregistrée quand le journal est activé, même avec `sample_rate = 0`, donc un diagnostic demandé n'est jamais perdu par un tirage ;
  - INV-17 est vrai **par construction** : aucune requête ne partage une structure avec l'écrivain, et une file pleine rend la main immédiatement ;
  - les pertes sont **visibles** : un compteur exposé par la route, une ligne `warn` à chaque augmentation, et un test qui compare le compteur aux lignes ;
  - la capture multi-index solde la dette d'ADR-0120, décision 4, sans toucher au pipeline de `crible-search` : une capture par cible, une ligne par requête ;
  - le vidage après les analytics protège les chiffres toujours actifs : si le budget d'arrêt manque, c'est le journal, écrit en second, qui cède ;
  - la route est `admin`, la durée de vie est courte, le journal est désactivé par défaut : le contenu assumé reste borné ;
  - aucune dépendance nouvelle.
- **Négatives / coûts** :
  - **une clé `read` peut écrire son propre texte au journal** quand `debug: true` et que le journal est activé ; c'est la décision 2, prise par l'orchestrateur au titre de la délégation, et le propriétaire la reverra en priorité avec la décision 1 ;
  - au-delà de 4 096 requêtes en attente, des lignes sont perdues ; `debug_journal_dropped` le dit mais ne les reconstitue pas ;
  - le vidage d'arrêt ajoute jusqu'à 2 secondes après celui des analytics : `stop_grace_period` devra couvrir les deux en P8 ;
  - les requêtes seulement échantillonnées construisent une capture et les éléments du plan pour le journal, même sans `debug` : un coût borné, mais non nul, sur le chemin d'une requête qui ne demande rien ;
  - une ligne multi-index est plus grosse et son filtre `index` lit le JSON : la lecture par index est moins directe que pour une ligne simple ;
  - `sample_rate = 1.0` écrit une ligne par requête : le fichier croît à la vitesse du trafic, et c'est le pire cas de MES-16 ;
  - `created_at` est l'heure du vidage : une ligne peut changer de borne `from`/`to` selon l'intervalle de 10 s.
- **À surveiller** :
  - la **revue par le propriétaire des décisions 1 et 2** : ce sont les deux points que l'orchestrateur a tranchés au titre de la délégation, et une validation officielle en fin de série peut les renverser ;
  - la première ligne `warn` de `debug_journal_dropped` en exploitation : elle dit un écrivain bloqué (disque, verrou SQLite), et doit être traitée comme un incident ;
  - la durée du vidage d'arrêt mesurée en P8 : si les 2 secondes du journal sont régulièrement atteintes en plus de celles des analytics, `stop_grace_period` ou le mécanisme d'écriture est en cause, et le remède est un ADR ;
  - le résultat de MES-16 avec `sample_rate = 1.0` : un dépassement du critère rouvre la question du mécanisme d'écriture, pas le critère ;
  - la taille de `debug.sqlite` après une campagne : elle dit ce que coûte le pire cas, et une instance laissée à `1.0` doit être un choix explicite ;
  - le coût du filtre `index` par appartenance quand le fichier grossit : si la route devient lente, un index sur les cibles ou une table de jointure demandera un ADR, pas une requête plus permissive.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Faire passer `debug: true` par le tirage `sample_rate` | À `sample_rate = 0`, un diagnostic explicitement demandé ne serait jamais enregistré ; `docs/06:79` et `docs/05:270` décrivent deux voies, et la fiche est adaptée. |
| Appliquer `sample_rate` à toutes les requêtes, `debug` compris | Rend le journal inutilisable pour le cas qui le justifie : une requête qu'on veut diagnostiquer serait perdue une fois sur deux à `0,5`. |
| Réserver `debug: true` à `admin` ou à `write` | Ajoute une restriction que ni la fiche ni `docs/05` ne posent ; `debug` est un diagnostic de la requête de l'appelant, pas un accès aux données d'un autre. La lecture du journal reste `admin`, et c'est elle qui protège le contenu. |
| Une ligne par cible en multi-index | Réécrit N fois le texte et le filtre, gonfle le fichier et multiplie l'exposition d'un même texte sensible ; la route peut filtrer par appartenance dans `targets[]`. |
| Un journal multi-index sans capture (seconde option d'ADR-0120) | `docs/06:82` demande le top 20 par branche ; une capture par cible coûte le même `top_n` par cible et ne change pas le pipeline. |
| Écrire dans SQLite sur le chemin de la requête | Une requête attendrait le disque ou un verrou SQLite : c'est exactement ce qu'INV-17 interdit. |
| Une file non bornée | Consomme toute la mémoire quand l'écrivain est bloqué ; à la limite, le processus est tué et toutes les lignes non écrites sont perdues, sans compteur. |
| Une file bornée plus grande, ou réglable | 4 096 requêtes couvrent déjà des dizaines de secondes de trafic ; une taille réglable est une valeur de plus à justifier, et l'augmenter masque un écrivain bloqué. |
| Écraser la ligne la plus ancienne quand la file est pleine | `try_send` refuse le neuf sans coût ; écraser l'ancien demanderait une structure de plus et réordonnerait les lignes. |
| Un verrou court ou une `DashMap` partagée | Un verrou, même court, est pris sur le chemin d'une recherche : une requête attendrait une écriture, ce qu'INV-17 interdit (ADR-0118, décision 1). |
| Compter les pertes de vidage dans `debug_journal_dropped` | Mêle deux causes — file pleine et disque en échec — dans un compteur, qui devient inexploitable ; le nombre perdu est dans la ligne `warn`. |
| Vider le journal avant les analytics à l'arrêt | Les analytics sont toujours actives et portent les chiffres d'usage ; le journal est le diagnostic et doit céder le premier si le budget manque. |
| Aucune borne pour le vidage d'arrêt | Un disque lent bloquerait l'arrêt jusqu'au `stop_grace_period`, qui tue le processus : sortie brutale, sans journal. 2 s laisse la place à une écriture normale. |
| Reprendre un vidage en échec, en boucle | Une boucle sur un disque plein remplit le journal et empêche l'arrêt ; la perte est bornée et déjà annoncée. |
| Purger à chaque vidage sans condition de 24 h | Une requête `DELETE` toutes les dix secondes sur une table dont la plus vieille ligne a 7 jours : du travail pour rien et un verrou d'écriture de plus. |
| Un minuteur dédié à la purge | Une tâche de plus pour une table qu'une instance au repos ne remplit pas ; la purge au vidage conditionnée à 24 h ne coûte qu'une comparaison. |
| Accepter `ttl_days = 0` | Viderait le journal à chaque purge : le journal serait activé, écrirait, puis se viderait, et l'activation serait un mensonge. |
| Rendre `404` quand le journal est désactivé | Un client devrait traiter un cas spécial pour un journal simplement éteint ; `200` avec `enabled: false` et `queries: []` dit la même chose sans erreur. |
| Paginer par `OFFSET` | Les lignes neuves arrivent en tête ; un décalage ferait sauter ou répéter des lignes entre deux pages. Le keyset sur `id` est stable. |
| Réserver la route à `write` | `docs/05:76` et ADR-0095 fixent `admin` pour `/v1/debug/queries` ; `write` verrait le texte des requêtes d'autres clients. |
| Rendre le payload comme une chaîne JSON | Le client devrait l'analyser lui-même, et une évolution de forme passerait sans contrôle ; la route rend un objet, et un payload illisible est une erreur `internal` déclarée. |
| Prouver INV-17 par une mesure de latence sous charge | La latence d'un runner n'est pas reproductible (ADR-0113, `:94`) ; la barrière rend le test déterministe et tourne en CI. |
| Mesurer MES-16 avec `sample_rate` faible | Le pire cas d'écriture est `1.0` ; mesurer à `0,1` laisserait croire que le journal est gratuit alors que la configuration permet le contraire. |

## Références

- `docs/phases/P7-observabilite.md` : `:55-72` (fiche `JOURNAL-DE-DEBUG` ; `:60` `debug.sqlite`, `:61` enregistrement, `:62` écriture asynchrone, `:63` purge, `:64` route, `:65` test `inv_17_*`, `:66` MES-16, `:68-72` critères, `:73` INV-17).
- `docs/06-observabilite.md` : `:5-11` (tableau analytics / journal), `:47-50` (mécanisme d'écriture des analytics, transposé), `:74-88` (journal : config, enregistrement, contenu, consultation, écriture, raison), `:124-129` (santé et logs).
- `docs/05-api.md:37-53` (codes et statuts), `:70-76` (rôles des routes de recherche et de la route du journal), `:270` (`debug` alimente le journal quand il est activé), `:278` (champ inconnu refusé en `invalid_request`), `:80` (exceptions nommées du test des routes).
- `docs/07-stack-et-deploiement.md:29-50` (dépendances ; `tokio` feature `sync` ligne 31, `rusqlite` ligne 39, `serde_json` ligne 48, `getrandom` ligne 44), `:89-92` (`[debug_journal]`), `:121` (séquence d'arrêt).
- `docs/08-invariants-et-tests.md:9` (INV-01), `:21` (INV-13), `:23` (INV-15), `:25` (INV-17).
- `crates/crible-server/src/search.rs:512-527` (`Attempt`), `:586` (capture construite pour `debug`), `:600` (`capture.as_mut()`) ; `crates/crible-server/src/search_multi.rs:589-607` (`capture: None`, dette d'ADR-0120) ; `crates/crible-server/src/debug.rs:160-178` (`SearchDebug`, plan et temps) ; `crates/crible-server/src/config.rs:331-337` (domaine de `sample_rate`) ; `crates/crible-server/src/auth.rs:120` (`getrandom::fill`) ; `crates/crible-server/src/barriers.rs` (barrières `failpoints`) ; `crates/crible-store/src/sqlite.rs:83-143` (`check_format_version`) ; `crates/crible-search/src/debug.rs:38` (`DEFAULT_TOP_N = 20`) et `:53-108` (`DebugCapture`).
- ADR-0014 (journal à la demande), ADR-0027 (`serde_json`), ADR-0068 et son complément du 2026-10-29 (features `failpoints` en CI), ADR-0095 (`admin` par famille de routes), ADR-0096 (aucun log de contenu, feature `sync`), ADR-0097 (validation de la configuration au démarrage), ADR-0099 (arrêt propre), ADR-0100 (`getrandom`, nom de la clé), ADR-0101 (corps d'erreur), ADR-0103 (`busy_timeout`), ADR-0104 (arrêt et jobs), ADR-0109 (champs inconnus, échéance), ADR-0110 (corps multi-index, filtres par cible), ADR-0111 et son complément du 2026-10-30 (exception nommée retirée dans le même changement), ADR-0113 (décisions 1, 5, 6 et 10 : critère, protocole, codes, échec), ADR-0118 (décisions 1 à 3, 6 à 8), ADR-0119 (décisions 3, 8, 10 et 13), ADR-0120 (décision 4), ADR-0121 (stockage).

## Documents mis à jour par cet ADR

| Document | Modification |
|---|---|
| `docs/06-observabilite.md:76-88` | journal de debug : deux voies d'activation, aucune restriction de rôle pour `debug: true`, capture si `debug` ou échantillonnage, une ligne par requête en multi-index, file de 4 096 et `debug_journal_dropped`, vidage après les analytics borné à 2 s, purge à l'ouverture puis toutes les 24 h, route `admin` avec ses paramètres et son `400`, aucun contenu dans les logs |
| `docs/05-api.md:76` | la ligne de la route du journal renvoie à ADR-0121 et ADR-0122 et dit le `200` avec `enabled: false` |
| `docs/05-api.md:80` | l'exception nommée de `GET /v1/debug/queries` est retirée de la liste des routes de P6 ; seule `GET /v1/stats` y reste (ADR-0111, complément du 2026-10-30) |
| `docs/05-api.md` (nouvelle section « Journal de debug ») | forme de la réponse de `GET /v1/debug/queries`, paramètres `index`, `min_total_ms`, `from`, `to`, `limit`, `before_id`, ordre, pagination keyset, `400 invalid_request` nommant le paramètre, composant OpenAPI |
| `docs/05-api.md:270` | `debug` alimente le journal **toujours** quand il est activé ; `sample_rate` ne vise que les autres requêtes (ADR-0122, décisions 1 et 2) |
| `docs/07-stack-et-deploiement.md:89-92` | commentaires de `[debug_journal]` : `sample_rate` ne vise que les requêtes sans `debug`, `ttl_days = 0` refusé au démarrage |
| `docs/07-stack-et-deploiement.md:121` | la séquence d'arrêt vide le journal après les analytics, borne de 2 s, échec en `warn` sans reprise |
| `docs/phases/P7-observabilite.md:55-72` | la fiche `JOURNAL-DE-DEBUG` reçoit les deux voies d'activation, le critère d'enregistrement associé, les renvois à ADR-0121 et ADR-0122, et la capture multi-index |
| `docs/phases/P7-observabilite.md:80` | la fiche `REPONSE-DEBUG` dit que la capture est construite si `debug` ou si la requête est échantillonnée (décision 3) |
| `docs/adr/README.md` | ligne d'index de cet ADR |

Sont **non modifiés** : `docs/01` à `docs/04`, `docs/08` (ses formulations restent vraies), `docs/suivi.md` (tenu par l'orchestrateur), les ADR-0113, ADR-0118, ADR-0119 et ADR-0120 (complétés par renvoi, corps inchangé ; le dépôt n'a pas de ligne « Complété par » en tête d'ADR, donc aucune n'est ajoutée), et tout fichier de code à ce stade : le code est livré par JOURNAL-DE-DEBUG.

Les numéros de ligne de `docs/06-observabilite.md` cités dans cet ADR sont ceux de son **état d'avant application** : la mise à jour de ce document décale les sections suivantes, et c'est la modification ci-dessus qui fait foi.
