# ADR-0102 — Limites : taille de corps, recherches concurrentes, échéance de `timeout_ms`, features

- **Statut** : Accepté (accepté par anticipation par le propriétaire le 2026-10-29, à confirmer à la fin de la série DEMARRER-SERVEUR-AXUM, AUTHENTIFIER-CLES, FORMAT-ERREURS-ET-LIMITES)
- **Date** : 2026-10-29
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, tâche FORMAT-ERREURS-ET-LIMITES (phase P6), sur demande du propriétaire et le verdict de `@architecte` du 2026-10-29
- **Remplace** : —

## Contexte

La fiche `FORMAT-ERREURS-ET-LIMITES` (`docs/phases/P6-serveur-et-api.md:43-52`) demande une limite de taille de corps, une limite de recherches concurrentes (`503 overloaded`, avec `Retry-After`) et un `timeout_ms` par requête (`504`). `docs/05-api.md:263-277` donne des valeurs ; `docs/07-stack-et-deploiement.md` donne les clés (`[server]` `:66-69`, `[limits]` `:97-102`).

Quatre familles de questions restent ouvertes.

**1. La taille de corps.** `docs/05` plafonne le lot en ligne et le patch de métadonnées (`:269-270`) — « 32 Mo » avant application du présent ADR, 32 Mio après —, et `payload_too_large` (413) pour « corps au-delà des limites » (`:45`). ADR-0060 a repris ces bornes pour le patch (32 Mo et 10 000 entrées). Trois choses manquent :

- **l'unité** : « Mo » se lit 10⁶ ou 2²⁰ octets, et l'écart vaut 4,9 % ;
- **la borne applicable à toute requête**, pas seulement au lot en ligne et au patch : aucune autre route n'a de plafond écrit, et le serveur n'a aujourd'hui aucune couche de limite de corps (`crates/crible-server/src`, aucun `BodyLimit`) ;
- **la configurabilité** : `docs/07` ne porte aucune clé de taille de corps, et sa liste de clés est fermée (ADR-0097).

**2. Les recherches concurrentes.** `docs/05:272` promet « 2 × le nombre de cœurs », puis `503`, et `docs/07:69` déclare `[server].max_concurrent_searches = 0` avec « 0 = 2 × cœurs ». La clé est déjà lue par la configuration (`crates/crible-server/src/config.rs:301`) et n'est employée nulle part ailleurs. Quatre points ne sont pas écrits :

- **l'attente** : un `503` immédiat ou une mise en file ;
- **la forme du refus** : le code est `overloaded` (`docs/05:48`), mais la valeur de `Retry-After` n'est fixée nulle part ; la ligne `:271` de la file d'écriture le nomme sans valeur, `:48` le nomme entre parenthèses pour `overloaded` ;
- **le périmètre** : `docs/05:48` mêle dans `overloaded` la file d'écriture et « limite de recherches concurrentes », sans dire si les sondes, les écritures ou les routes d'administration y sont soumises ;
- **la preuve** : les routes de recherche n'existent pas encore. ADR-0095 a établi la règle : une tâche prouve ce qu'elle peut prouver sur un routeur de test, et le test sur les routes réelles revient à la tâche qui les crée.

**3. L'échéance de `timeout_ms`.** `docs/05:177` borne `timeout_ms` de 1 à 30 000 ms, avec 2 000 par défaut, et `:273` répète « 2 000 par défaut, 30 000 au plus » ; `[limits].default_timeout_ms` vaut 2 000 (`docs/07:99`). `docs/05:49` donne `timeout` → `504` pour « requête trop longue ». Ce qui n'est pas écrit :

- **d'où vient la valeur** : du corps de la recherche ou de la seule configuration ;
- **le sort du calcul à l'échéance** : la recherche dense tourne sur le pool rayon (`docs/02-architecture.md`), qui ne s'annule pas ; une échéance côté tokio n'arrête donc pas le travail déjà lancé ;
- **le sort du jeton de concurrence** de cette requête, si la limite de la décision 2 existe ;
- **le cas des écritures**, alors que `?wait=true` rend `202` à l'échéance de `wait_timeout_ms` et **jamais** `504` (ADR-0067, `docs/05:20`). ADR-0038 a déjà posé la distinction : `timeout` appartient au délai **de requête**, `overloaded` à une contention transitoire.

**4. Les features des couches.** `docs/07:31` disait seulement, avant application du présent ADR, que `tower-http` prend « `trace`, plus celles qu'exigent les limites ». ADR-0096 consigne les features déjà employées et n'a pas tranché celles-ci. La question est réelle : `tower::load-shed` existe pour limiter la concurrence, et `tower_http::timeout` pour l'échéance, mais leurs réponses ne suivent pas le format d'erreur d'ADR-0101 — `load-shed` rend une réponse vide, sans corps ni `Retry-After`.

## Décision

**Décision 1 — taille de corps.** Une limite **globale fixe de 32 Mio (33 554 432 octets)**, constante du code, sans clé de configuration : la liste de `docs/07` reste fermée (ADR-0097). Au-delà, `413 payload_too_large`. Les « 32 Mo » de `docs/05` (lot en ligne et patch, ADR-0060) s'entendent en **Mio** : la même borne vaut pour toute requête. Les limites propres à une route restent celles de `docs/05` — par exemple les 10 000 entrées d'un patch, qui ne sont pas une taille d'octets.

**Décision 2 — recherches concurrentes.** Un sémaphore de `[server].max_concurrent_searches` jetons (`0` = 2 × cœurs, `docs/07`), pris **sans attente**. S'il n'y a plus de jeton, la réponse est `503 overloaded` **immédiate**, avec `Retry-After: 1` en secondes entières.

La limite ne s'applique **qu'aux routes de recherche** : `/v1/indexes/{index}/search` (ROUTE-RECHERCHE) et `/v1/search` (RECHERCHE-MULTI-INDEX). Elle ne s'applique **jamais** à `/health`, à `/ready` ni aux écritures : une sonde doit répondre pendant une surcharge (ADR-0099), et les écritures ont leur propre limite, la file d'écriture de `docs/05:271`.

`FORMAT-ERREURS-ET-LIMITES` livre le **mécanisme**, prouvé sur un routeur de test. Le test sur les routes réelles est porté par ROUTE-RECHERCHE et RECHERCHE-MULTI-INDEX, et le critère est ajouté à leurs fiches : même règle qu'ADR-0095.

**Décision 3 — échéance de `timeout_ms`.** La valeur est lue dans le **corps de la recherche** (`docs/05:177`, 1 à 30 000 ms ; hors bornes, `invalid_request`), et son défaut est `[limits].default_timeout_ms`.

À l'échéance, la réponse est `504 timeout`. Le calcul sur le pool rayon **n'est pas annulé** en v1 : il finit seul, et **son jeton de sémaphore reste pris jusqu'à sa fin réelle**, pour que la limite de concurrence reste vraie. Une annulation coopérative est une piste future, hors v1.

`timeout_ms` ne s'applique **pas** aux écritures : `?wait=true` rend `202` à l'échéance de `wait_timeout_ms`, jamais `504` (ADR-0067).

`FORMAT-ERREURS-ET-LIMITES` livre le mécanisme — échéance, `504`, jeton tenu — prouvé sur un routeur de test. ROUTE-RECHERCHE le branche sur sa route, et le critère est ajouté à sa fiche.

**Décision 4 — features.** `tower-http` gagne les features `limit` (taille de corps, décision 1) et `catch-panic` (panique rattrapée, ADR-0101 décision 7).

L'échéance de la décision 3 utilise `tokio::time::timeout` (feature `time`, déjà consignée par ADR-0096) : **pas** de couche `timeout` de `tower-http`, qui ne laisserait pas le jeton tenu et ne servirait pas le corps d'ADR-0101.

La feature `load-shed` de `tower` est **écartée** : sa réponse n'a ni corps ni `Retry-After`, ce qui contredit ADR-0101 et la décision 2.

Aucune crate nouvelle : les trois crates concernées (`tokio`, `tower`, `tower-http`) sont dans `docs/07`.

Ce qui n'est décidé ici : le point exact où part le chronomètre de `timeout_ms` — validation et authentification comprises ou non —, que ROUTE-RECHERCHE fixe au plus tard ; la valeur de `Retry-After` pour les autres causes d'`overloaded` (file d'écriture, verrou SQLite d'ADR-0038, lexical d'ADR-0089), inchangées ; la forme d'une annulation coopérative, hors v1 ; la limitation de débit par client, qui n'existe pas.

## Conséquences

- **Positives** :
  - une seule borne de corps pour toute l'instance, en octets, sans ambiguïté d'unité et sans clé de plus à tenir dans deux documents ;
  - le refus de concurrence est **immédiat** et porte une valeur de `Retry-After` écrite : un client sait combien de temps attendre, au lieu de recevoir une réponse vide ;
  - le sémaphore ne ment pas : tant qu'un calcul tourne, son jeton est pris, donc une instance ne peut pas dépasser `max_concurrent_searches` calculs réels ;
  - les sondes et les écritures gardent leur comportement : une surcharge de recherche ne rend pas l'instance « non prête » ni ne bloque une attache ;
  - l'échéance n'ajoute ni crate ni couche : `tokio::time` est déjà là (ADR-0096) ;
  - la distinction d'ADR-0038 et d'ADR-0067 est préservée : `504` pour un délai de requête, `202` pour une attente de job.
- **Négatives / coûts** :
  - 32 Mio est un plafond unique : un projet qui voudrait un lot en ligne plus gros ne peut pas le régler sans un ADR, et un projet qui en voudrait un plus petit paie le même ;
  - sous une rafale de recherches lentes, les jetons restent pris au-delà de leur échéance : le client reçoit `504`, mais la capacité ne revient qu'à la fin réelle du calcul, donc un `503` peut suivre un `504` ;
  - `Retry-After: 1` est une valeur fixe, sans lien avec la durée réelle d'attente ;
  - la couche `limit` et la couche `catch-panic` rendent par défaut des réponses sans corps : il faut les réécrire dans le format d'ADR-0101, sinon le `413` et le `500` contrediraient la décision 1 du présent ADR et celle d'ADR-0101 ;
  - chaque requête traverse deux couches de plus.
- **À surveiller** :
  - la fréquence des couples `504` puis `503` sous charge : si elle est élevée, l'annulation coopérative cesse d'être une piste lointaine ;
  - la valeur de `Retry-After: 1` : si une mesure montre qu'un jeton se libère en dizaines de millisecondes, ou en dizaines de secondes, la constante est à revoir ;
  - le profil de compilation : `catch-panic` ne rattrape rien si le binaire passe à `panic = "abort"` ;
  - un test qui vérifie que le corps d'un `413` et d'un `500` de panique suit le format, avec le `request_id` de la requête ;
  - la cohérence de `docs/05` et `docs/07` sur `max_concurrent_searches` et sur l'unité Mio ;
  - le test de charge de P6 (`TESTER-CHARGE-CONCURRENTE`, INV-02) : les `503` attendus y sont admis, les autres `5xx` non.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Une clé `[limits].max_body_bytes` | Ajoute une clé à une liste fermée (ADR-0097) pour une valeur qu'aucun besoin mesuré ne demande de changer ; la règle 12 des `AGENTS.md` interdit la configuration d'une valeur fixe. |
| Interpréter « 32 Mo » comme 32 000 000 octets | Laisse la borne du serveur sous celle des documents qui parlent de Mio ; l'écart silencieux de 4,9 % ferait refuser un lot que le producteur croit conforme. |
| Une limite de corps par route | Multiplie les valeurs à tenir dans `docs/05` et dans le code ; la borne globale suffit, et les limites de contenu (10 000 entrées) restent par route. |
| Mettre en file les recherches au-delà de `max_concurrent_searches` | Une file non bornée reporte la surcharge sur la latence et la mémoire, sans que le client le sache ; le refus immédiat dit la vérité. |
| `503` sans `Retry-After` | Un client ne sait pas quand réessayer et réessaie tout de suite, ce qui aggrave la surcharge. |
| `load-shed` de `tower` | Réponse sans corps ni `Retry-After` : contredit ADR-0101 et la décision 2, et ne porte pas le `request_id`. |
| La couche `timeout` de `tower-http` | Laisse le calcul rayon tourner sans que le jeton reste tenu, donc la limite de concurrence devient fausse ; et sa réponse ne suit pas le format d'ADR-0101. |
| Annuler le calcul rayon à l'échéance | Le pool rayon n'offre pas d'annulation coopérative ; l'implémenter demanderait de découper le calcul en points de contrôle, hors v1 et non mesuré. |
| Libérer le jeton à l'échéance | Rendrait vrais plus de calculs que de jetons, et une instance saturée répondrait `200` puis s'effondrerait ; la limite cesserait d'être une limite. |
| Appliquer `timeout_ms` aux écritures et rendre `504` à l'échéance de `?wait=true` | Contredit ADR-0067 : le job continue et le client doit le suivre en `202`, pas croire à un échec. |
| Limiter aussi les sondes | Une sonde refusée fait redémarrer une instance saine, ce qui est exactement ce qu'ADR-0099 évite. |
| Un plafond par client | Demande une identité de débit, une fenêtre et un stockage que `docs/05` ne décrit pas ; le sémaphore global couvre le risque de saturation. |

## Références

`docs/05-api.md` (section « Erreurs » `:24-52`, champ `timeout_ms` de la recherche `:177`, section « Limites par défaut (configurables) » `:263-277`) ; `docs/06-observabilite.md` (« Santé et logs » `:111-116`) ; `docs/07-stack-et-deploiement.md` (`[server]` `:66-69`, `[limits]` `:97-102`, ligne HTTP du tableau des dépendances `:31`) ; `docs/02-architecture.md` (« Concurrence », `:164` : pool rayon dédié, calculs jamais sur les threads tokio) ; `docs/phases/P6-serveur-et-api.md` (fiches `FORMAT-ERREURS-ET-LIMITES` `:43-52`, `ROUTE-RECHERCHE` `:99-111`, `RECHERCHE-MULTI-INDEX` `:113-127`) ; `crates/crible-server/src/config.rs` (`max_concurrent_searches`, `:301`) ; ADR-0038 (délai SQLite, `overloaded` plutôt que `timeout`) ; ADR-0060 (32 Mo et 10 000 entrées du patch) ; ADR-0067 (`?wait=true`, `202` à l'échéance) ; ADR-0095 (test sur routeur de test, critère reporté à la tâche qui crée la route) ; ADR-0096 (features du serveur) ; ADR-0097 (liste fermée des clés de configuration) ; ADR-0099 (sondes et arrêt) ; ADR-0101 (format unique du corps d'erreur).
