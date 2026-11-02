# ADR-0124 — Route de statistiques `GET /v1/stats` : forme de la réponse, paramètres, seaux et commande `crible stats`

- **Statut** : Accepté (accepté par anticipation par le propriétaire le 2026-11-01, confirmé par le propriétaire le 2026-11-01, délégation du propriétaire pour la suite de P7 ; validation officielle à la fin de la série ; complément du 2026-11-01, `top_filter_shapes` et `operation`, confirmé par le propriétaire le 2026-11-02)
- **Date** : 2026-11-01
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, tâche ROUTE-STATS (phase P7), sur l'analyse de `@architecte` et les décisions retenues par l'orchestrateur, en application de la délégation du propriétaire du 2026-11-01 pour la série EXPLIQUER-DOCUMENTS, ROUTE-STATS, COMMANDE-EVAL, REUTILISER-LECTEURS-DENSES
- **Remplace** : — (ne remplace aucune décision ; précise ADR-0095, ADR-0111, ADR-0116, ADR-0117, ADR-0118, ADR-0122 sans modifier le corps d'un ADR accepté)

## Contexte

### La fiche et ce qu'elle demande

La fiche `ROUTE-STATS` (`docs/phases/P7-observabilite.md:101-111`) a pour objet de **lire les analytics**. Ses livrables sont `GET /v1/stats` — comptes, erreurs par code, p50/p95/p99 par étape, taux de requêtes sans résultat, index et formes de filtre les plus fréquents, état journalier (`:106`) — et la commande `crible stats`, qui « affiche des tableaux » (`:107`). Ses critères sont : les chiffres d'un trafic de test connu sont exacts (`:109`) ; une clé `read` ou `write` reçoit `403` sur `/v1/stats`, réservé à `admin` (`:110`) ; la route figure dans la spécification OpenAPI et son exception nommée quitte le test des routes dans le même changement (`:111`). La fiche ne cite aucun invariant.

### Ce que la conception pose, et ce qu'elle laisse ouvert

`docs/06:64-74` décrit la lecture : `GET /v1/stats?from=…&to=…&index=…&operation=…&group_by=minute|hour|day` (`:66`), puis six rubriques (`:67-72`) : comptes et erreurs par code ; p50, p95 et p99 par étape à la résolution d'un seau ; taux de requêtes à 0 résultat ; index et formes de filtre les plus fréquents ; état journalier des index ; `analytics_dropped`. `:74` ajoute que `crible stats` « affiche la même chose en tableau » et renvoie la forme de la réponse et la façon dont la CLI lit les analytics à ROUTE-STATS. `docs/05:75` déclare la route, réservée à `admin` (`docs/05:13`), et renvoie à `docs/06`.

ADR-0117, dernier alinéa de la décision 6 (`:199`), renvoie explicitement à ROUTE-STATS « la forme exacte de la réponse de `GET /v1/stats`, l'agrégation `group_by=minute|hour|day` et la façon dont `crible stats` lit les analytics ». Rien n'est donc décidé sur la forme, les paramètres, les défauts, les bornes, les erreurs, le contenu exact des seaux, le classement des listes et la surface CLI.

Les décisions voisines bornent le reste. ADR-0117 ferme les neuf étapes de latence (`docs/06:19`, `:51`) et fixe la règle du percentile (décision 4) : rang le plus proche sur les comptes cumulés, valeur rendue = borne haute du seau, « exact » valant exact sur la somme des seaux. ADR-0117, décision 5, pose **un événement par cible** en multi-index (`:181`) et ferme les champs de `SearchEvent`. ADR-0117, décision 6, range `filter_shape` dans sa propre table et en fixe la canonicalisation. ADR-0118, décision 5, fixe les lignes journalières `day_indexes` et écrit qu'un trou de jour est voulu (`:87`). ADR-0118, décision 2, déclare `analytics_dropped` **par processus**, jamais persisté, exposé par `GET /v1/stats` sous ce nom exact (`:48-50`). ADR-0122, décision 9, donne le précédent des paramètres d'une route de lecture : ISO 8601 pour `from` et `to`, bornes **incluses**, paramètre inconnu ou invalide en `400 invalid_request` nommant le paramètre (`:197`). ADR-0116, décision 2 (a), écrit quelles lectures de la CLI restent permises à côté d'un serveur en service (`search`, `inspect`, `export`) et n'y cite pas `stats`. ADR-0095 fixe `403` pour un rôle insuffisant. ADR-0111, complément du 2026-10-30, exige que la route entre dans la spécification et que son exception nommée quitte le test des routes dans le même changement.

### L'état du code aux sources citées

- `crates/crible-store/src/analytics.rs:649` : `read_minute_events` ouvre le fichier en `SQLITE_OPEN_READ_ONLY`, refuse une `format_version` inconnue, mais **charge toutes les lignes sans `WHERE`** (`:654-670`). C'est le seul lecteur existant.
- `crates/crible-store/src/analytics.rs:467` : `open_or_create` **écrit** (schéma, purge, lignes). Il ne convient pas à la lecture.
- `crates/crible-store/src/analytics.rs:256-284` : `ShapeCount` et `DayIndex` portent déjà les champs des deux tables secondaires ; `latency_percentile` et `pool_percentile` (`:362-370`) rendent la borne haute d'un seau, jamais une moyenne.
- `crates/crible-server/src/search_multi.rs:219-226` : quand l'échéance expire avant que le fil bloquant réponde, **un seul** événement est produit, sur la première cible résolue, alors que le commentaire de `:128-131` et ADR-0117, décision 5, en veulent un par cible. `:139-152` montre par ailleurs qu'un refus antérieur à la résolution des cibles reprend le premier nom d'index déclaré dans le corps (`first_target_name`), et laisse `mode` vide.
- `crates/crible/src/main.rs:209-212` : `crible search` a `--format json|text`, **JSON par défaut**, avec une énumération `clap` ; `crates/crible/src/main.rs:214-222` montre la forme des options d'une commande de lecture (`--data`, `--index`).

### Les écarts et les points non couverts

**Écart 1 — le `504` multi-index.** ADR-0117, décision 5, impose un événement par cible. Le code n'en émet qu'un sur un `504` multi-index, sur la première cible résolue (`search_multi.rs:219-226`). Le suivi le note depuis le 2026-10-31 (`docs/suivi.md:658`). Les statistiques d'un `504` multi-index ne se répartissent donc pas par cible.

**Écart 2 — `mode` vide et nom de cible repris du corps.** Une requête refusée avant la résolution du mode porte `mode` vide, et le code reprend le premier nom d'index déclaré dans le corps alors que ce nom n'a pas été résolu. Le second point n'est pas écrit dans `docs/06`.

**Écart 3 — la lecture de `crible stats`.** `docs/06:74` ne dit pas si la CLI lit par l'API ou en ouvrant `analytics.sqlite`. `docs/07:29-50` ne porte **aucun client HTTP** côté Rust, et ADR-0116 ne cite pas `stats` parmi les lectures permises à côté d'un serveur en marche.

**Écart 4 — le libellé de `docs/05:80`.** Le texte y dit encore « en P6, `GET /v1/stats` (`ROUTE-STATS`) » comme exception du test des routes.

**Points non couverts.** Forme de la réponse (`:1`) ; paramètres, bornes, défauts et erreurs (`:2`) ; filtres supplémentaires (`:3`) ; percentiles par étape et étapes sans échantillon (`:4`) ; dénominateur du taux sans résultat (`:5`) ; forme des listes les plus fréquentes, `N`, ordre et départage (`:6`, `:7`) ; état journalier (`:8`) ; coût de la lecture (`:12`) ; options et format de la CLI (`:14`, `:16`) ; preuve « chiffres exacts » (`:17`).

## Décision

### Décision 1 — Forme fermée de la réponse

**Décision.** La réponse est un **objet à membres fixes**. Les membres sont `from`, `to`, `group_by` (échos normalisés) ; `buckets` ; `totals` ; `top_indexes` ; `top_filter_shapes` ; `indexes` ; `analytics_dropped`. Un membre **sans valeur est omis**, jamais rendu `null` ni rempli d'un `0` inventé (ADR-0119, décision 3).

Précisions :

- `from`, `to` et `group_by` sont les **échos normalisés** des paramètres effectivement appliqués : bornes converties en UTC, `group_by` valant `minute`, `hour` ou `day` ;
- `buckets` est la liste **chronologique** des seaux qui ont au moins un événement, chacun `{ "start", "count", "errors", "zero_results_rate", "latency_ms" }` :
  - `start` est le début du seau, en UTC, tronqué à la minute, à l'heure ou au jour selon `group_by` ;
  - `count` est la somme des `count` des événements du seau ;
  - `errors` est un objet **code → nombre**, les seuls codes présents ; il est **omis** quand le seau ne porte aucune erreur ;
  - `zero_results_rate` suit la décision 5 ; il est **omis** quand son dénominateur est nul ;
  - `latency_ms` est un objet **étape → `{ "p50", "p95", "p99" }`**, selon la décision 4 ; une étape sans échantillon est **omise** ;
- `totals` a la **même forme qu'un seau**, sur toute la fenêtre, **sans `start`** ;
- les quatre listes (`buckets`, `top_indexes`, `top_filter_shapes`, `indexes`) sont **toujours présentes**, même vides : une fenêtre sans trafic rend des listes vides, jamais un membre absent. C'est la lecture d'ADR-0122, décision 9, qui rend `queries: []` plutôt que d'omettre le membre ;
- `analytics_dropped` est un nombre ; `0` y est une **mesure**, pas un `0` inventé (décision 9) ;
- aucun membre ne porte de texte de requête, de valeur de filtre, de hash ni d'id de résultat (INV-13) ; `filter_shape` ne porte que des noms de champs et d'opérateurs (ADR-0117, décision 6).

### Décision 2 — Paramètres : bornes incluses, domaine fermé, erreurs nommées

**Décision.** Les paramètres sont `from`, `to`, `index`, `operation` et `group_by`, et **rien d'autre**. `from` et `to` sont des dates ISO 8601 **avec décalage**, normalisées en UTC, et les deux bornes sont **incluses** (règle d'ADR-0122, décision 9, reprise). `group_by` vaut `minute`, `hour` ou `day`, **`hour` par défaut**. La fenêtre par défaut est **toute la rétention**. `index` et `operation` sont des filtres d'égalité. Un paramètre **inconnu ou invalide**, un `group_by` hors du domaine, une `operation` hors de la liste fermée de `docs/06:30`, une date qui n'est pas ISO 8601 et un `from > to` rendent `400 invalid_request`, avec un message qui **nomme le paramètre** (précédent d'ADR-0122, décision 9, et `docs/05:38`).

Précisions :

- **aucune autre borne que la rétention** : ni nombre maximal de seaux, ni fenêtre maximale, ni plafond propre à la route. `analytics.retention_days` (90 par défaut, ADR-0118, décision 4) borne ce que le fichier contient encore ;
- un `from` absent commence à la plus ancienne ligne conservée, un `to` absent finit à la plus récente : c'est la lecture « toute la rétention », et les échos `from` et `to` rendent les bornes effectivement appliquées ;
- `group_by` vaut `minute`, `hour` ou `day` : `minute` est la **granularité de stockage** de `minute_events` (ADR-0117, décision 1), et un seau `minute` agrège les lignes de même minute, toutes clés confondues ; `hour` et `day` agrègent les mêmes compteurs sur une largeur plus grande ;
- le tri chronologique des seaux est celui de `start`, croissant ;
- **le filtre `index`** porte sur les noms **présents dans `analytics.sqlite`**, jamais sur les index vivants : un index supprimé laisse son historique lisible, et un nom sans aucun événement rend `200` avec une réponse vide, membres omis et listes vides selon la décision 1, jamais `404` ;
- un `index` qui n'est pas un **nom d'index valide**, au sens des règles de nommage des index, rend `400 invalid_request` nommant le paramètre, comme tout autre paramètre invalide ; un nom valide sans ligne n'est pas une erreur ;
- la **même règle** s'applique à `crible stats --index` (décision 13).

### Décision 3 — Aucun filtre ni répartition supplémentaire

**Décision.** La route n'offre **aucun** autre filtre ni aucune répartition : ni `status`, ni `client`, ni `mode`, ni `dense_strategy`. La liste des paramètres de `docs/06:66` est **fermée**.

Précisions :

- `status`, `client`, `mode` et `dense_strategy` restent des **dimensions de la ligne** stockée (ADR-0117, décision 1) : elles sont agrégées, jamais offertes en filtre ;
- le seul endroit où un code d'erreur apparaît est l'objet `errors` de la décision 1, qui **compte** les codes au lieu de les filtrer ;
- ajouter un filtre demanderait un nouvel ADR : `docs/06:66` est la référence de la liste.

### Décision 4 — Percentiles : un par étape échantillonnée, valeur = borne haute

**Décision.** Pour **chaque étape de `StageDurations`** — `validate`, `encode`, `filter`, `dense`, `lexical`, `fusion`, `group`, `fetch`, `total` — qui a **au moins un échantillon** dans le seau, `latency_ms` porte `p50`, `p95` et `p99`, exprimés en millisecondes en `f64`, à la **borne haute du seau d'histogramme** qui contient le rang le plus proche (ADR-0117, décision 4). Une étape **sans échantillon est omise**. Les percentiles de **taille de pool** ne sont pas rendus.

Précisions :

- `total` est comprise dans les neuf étapes : c'est la durée de la requête, et son histogramme a autant d'échantillons que d'événements, alors qu'une branche qui n'a pas tourné n'en a aucun (`docs/06:51`) ;
- les trois rangs sont calculés sur les **comptes cumulés** des seaux, jamais sur une moyenne de percentiles (`docs/06:49`) ;
- la valeur rendue est **sur-estimée, jamais sous-estimée** : un p95 de 3,2 ms signifie « entre 2,56 et 3,2 ms », et la résolution de 25 % est celle d'ADR-0117, décision 2 ;
- « chiffres exacts » de la fiche (`P7:109`) vaut **exact sur la somme des seaux** (ADR-0117, décision 4) : la route ne peut pas rendre mieux que la résolution d'un seau ;
- l'histogramme de pool est stocké (ADR-0117, décision 3) mais **n'est pas lu** par cette route : ni `docs/06:67-72` ni la fiche ne le demandent, et `docs/06:68` dit « par étape » ;
- les étapes sont rendues dans l'ordre de `StageDurations`, pour que la lecture et la comparaison soient stables.

### Décision 5 — `zero_results_rate` : les recherches abouties, par cible

**Décision.** `zero_results_rate` est la **somme des `zero_results` divisée par la somme des `count`** des événements des opérations de recherche (**`search` et `search_multi`**) dont le statut vaut **`ok`**, par seau et sur la fenêtre. Il est **omis** quand le dénominateur est nul. Une recherche multi-index **pèse une fois par cible**, parce qu'elle produit un événement par cible.

Précisions :

- le numérateur et le dénominateur sont pris sur **le même ensemble** d'événements : restreindre le dénominateur aux seules recherches abouties évite de compter un `400` ou un `504` comme « recherche sans résultat », alors qu'il n'a rien cherché ;
- la règle « une fois par cible » est celle d'ADR-0117, décision 5, et elle est **écrite** dans `docs/06` : une recherche sur deux index pèse deux fois, un par index, ce qui est cohérent avec le pool et la stratégie, qui sont propres à chaque cible ;
- le taux est un rapport des deux comptes de la période, jamais une moyenne des taux des lignes : sommer les comptes et diviser une fois donne le taux des événements ;
- `zero_results` n'est pas rendu comme compte : la fiche demande le **taux**, et un compte de plus n'ajouterait rien à la lecture ;
- le taux vaut 0 quand aucun événement abouti n'a rendu zéro résultat : `0` est alors une valeur mesurée, et le membre reste présent.

### Décision 6 — `top_filter_shapes` : dix formes, ordre total

**Décision.** `top_filter_shapes` rend **au plus 10** objets `{ "shape", "count" }` sur la fenêtre, par `count` **décroissant**, l'égalité étant départagée par l'**ordre lexicographique croissant de la forme**. La forme est celle d'ADR-0117, décision 6 : elle ne contient **que des noms de champs et d'opérateurs** (INV-13).

Précisions :

- les comptes sont **sommés sur la fenêtre**, toutes minutes confondues : la table `filter_shapes` porte un compteur par minute, par index et par forme (ADR-0117, décision 6) ;
- la borne de 10 est une **constante**, sans clé de configuration : `docs/07` n'en porte aucune, et la règle de simplicité interdit une clé pour une valeur fixe ;
- le départage lexicographique rend l'ordre **total et reproductible** : deux formes de même compte sortent toujours dans le même ordre, ce qui rend la réponse comparable d'une requête à l'autre ;
- un filtre absent ou vide ne produit aucune ligne (ADR-0117, décision 6), donc aucune forme au classement ;
- une forme vide n'existe pas, et une liste vide est rendue vide (décision 1).

### Décision 7 — `top_indexes` : dix index, ordre total

**Décision.** `top_indexes` rend **au plus 10** objets `{ "index", "count" }` sur la fenêtre, par `count` **décroissant**, l'égalité étant départagée par le **nom d'index croissant**.

Précisions :

- `count` est la somme des `count` des événements de cet index sur la fenêtre, tous statuts et toutes opérations confondus, après les filtres de la décision 2 ;
- la borne de 10 et l'absence de clé de configuration suivent la décision 6 ;
- le départage par nom rend l'ordre total, comme pour les formes ;
- un index sans événement sur la fenêtre ne figure pas dans la liste : `indexes` (décision 8) porte l'état journalier, pas le trafic.

### Décision 8 — `indexes` : la dernière ligne journalière de la fenêtre, sans comblement

**Décision.** `indexes` porte **une entrée par index**, avec la **dernière** ligne de `day_indexes` de cet index **dans la fenêtre**, `day` compris, tous les champs stockés plus les ratios dérivés que `docs/06` nomme. Aucun jour absent n'est comblé.

Précisions :

- l'entrée porte `day`, `index`, `documents`, `live_passages`, `dead_passages`, `dead_ratio`, `disk_bytes`, `generation` et `batches`. `dead_ratio` est **dérivé**, jamais stocké (ADR-0117, décision 1), et vaut `dead_passages / (live_passages + dead_passages)` ;
- « la dernière ligne de la fenêtre » se lit par `day` décroissant : la fenêtre peut couvrir plusieurs jours, et c'est l'état le plus récent de la fenêtre qui intéresse ;
- les jours sans ligne ne sont **pas** comblés : ADR-0118, décision 5, écrit qu'« un trou dit la vérité, et `day_indexes` n'a pas à inventer un état » (`:87`). Un index arrêté pendant minuit n'a donc pas de ligne pour ce jour ;
- un index sans ligne journalière dans la fenêtre **ne figure pas** dans `indexes` : il n'y a rien à rendre, et une entrée à zéros serait un état inventé (décision 1) ;
- l'ordre des entrées est le **nom d'index croissant**, pour être stable ;
- `disk_bytes` est la seule taille stockée ; `docs/06:34` parle de « tailles sur disque » au pluriel, et la table n'en porte qu'une : la route rend celle qui existe.

### Décision 9 — `analytics_dropped` : le compteur du processus, sur la route seulement

**Décision.** `analytics_dropped` rend le compteur de pertes du **processus serveur depuis son démarrage**, tel qu'ADR-0118, décision 2, le définit (`:48-50`) : par processus, jamais persisté, remis à zéro par un redémarrage. Il n'est rendu **que** par la route. `debug_journal_dropped` reste sur `GET /v1/debug/queries` (ADR-0122, décision 9).

Précisions :

- le compteur n'est pas lu dans le fichier : il n'y est pas (ADR-0118, décision 2). La route le lit dans l'état de l'instance, et `crible stats` ne le rend donc pas (décision 13) ;
- la valeur est un nombre, et `0` y est une mesure : aucune perte a été comptée, c'est une information (décision 1) ;
- aucun autre compteur de pertes n'entre dans cette réponse : `debug_journal_dropped` mesure un autre journal, avec sa propre route, et les réunir ferait croire à une seule file.

### Décision 10 — Correction de l'écart : un événement par cible après la résolution des cibles

**Décision.** Un `504` — et **tout échec postérieur à la résolution des cibles** — d'une recherche multi-index émet **un événement par cible**, conformément à ADR-0117, décision 5. `mode` reste vide quand le mode n'a pas été résolu, et c'est **assumé et documenté**. Une requête refusée **avant** la résolution des cibles émet **un seul** événement, **sans index**. Le code est corrigé par `@dev` dans cette tâche.

Précisions :

- l'écart est celui de `crates/crible-server/src/search_multi.rs:219-226`, où l'échéance expirée hors du fil bloquant ne produit qu'un événement sur la première cible ; le commentaire `:128-131` et ADR-0117, décision 5, en veulent un par cible ;
- les cibles non atteintes après l'échéance portent des histogrammes **vides** : leur événement dit le statut `timeout`, et aucune durée ni taille de pool n'est inventée pour un travail qui n'a pas eu lieu ;
- `mode` vide est celui d'un événement produit avant la résolution du mode ; `docs/06` le dit, pour qu'un lecteur ne prenne pas la chaîne vide pour un mode inconnu du domaine ;
- une requête refusée avant la résolution des cibles n'a pas de cible : elle porte `index` vide. Le premier nom d'index déclaré dans le corps **n'est pas repris** : ce nom n'a pas été résolu, et le porter ferait attribuer une erreur à un index qui n'a pas été ouvert (précédent d'ADR-0118, décision 6 : la clé a été résolue, et rien n'est inventé) ;
- cette correction **change le comportement** de l'agrégation : un `504` multi-index se répartit désormais par cible, et le taux de la décision 5 comme `top_indexes` en tiennent compte ;
- aucune autre règle d'ADR-0117, décision 5, n'est rouverte : les champs de `SearchEvent` restent fermés.

### Décision 11 — Les chiffres portent sur les vidages

**Décision.** Les chiffres rendus portent sur ce qui a été **vidé** dans `analytics.sqlite`, et le décalage est **borné par l'intervalle de vidage de 10 s** (ADR-0118, décision 3). `docs/06` l'écrit.

Précisions :

- une requête de la dernière seconde peut manquer à la lecture, et ce n'est pas une erreur : le vidage suivant l'ajoute ;
- le décalage est **borné**, pas quelconque : au plus un intervalle de vidage, plus la durée du vidage lui-même ;
- une lecture ne force **jamais** un vidage : la route ne parle pas à l'écrivain des analytics, et un lecteur ne provoque aucune écriture (décision 12) ;
- la minute de rattachement est celle du vidage, lue par SQLite (ADR-0118, décision 3) : la borne vaut donc aussi pour la répartition des seaux.

### Décision 12 — Lecture : agrégation SQL sur la fenêtre, connexion en lecture seule dédiée

**Décision.** La route agrège **en SQL**, filtrée sur la fenêtre et sur `index` et `operation`, par une connexion `SQLITE_OPEN_READ_ONLY` **dédiée**, jamais sous le verrou de l'écrivain. `open_or_create` n'est **pas** utilisé pour lire.

Précisions :

- `read_minute_events` (`crates/crible-store/src/analytics.rs:649`) charge toutes les lignes sans `WHERE` : il ne convient pas à une fenêtre bornée sur 90 jours de trafic, et il n'est pas réutilisé tel quel ;
- `open_or_create` (`:467`) écrit le schéma et purge : l'appeler pour lire serait une écriture déclenchée par une lecture, contraire à ADR-0116, décision 1, et à la lecture faite à côté d'un serveur en marche ;
- la connexion de lecture est **dédiée** : elle ne partage pas celle de l'écrivain, donc une lecture ne peut pas attendre un vidage ni le bloquer. En mode WAL (ADR-0038), un lecteur ne bloque pas l'écrivain, et le `busy_timeout` reste celui d'ADR-0038 ;
- l'agrégation somme les compteurs et les BLOB d'histogrammes **seau par seau** : les seaux sont fixes, donc les histogrammes s'additionnent (ADR-0117, décision 2) ;
- `crible-store` reste le propriétaire du fichier (ADR-0117) : la route passe par lui, sans arête `crible-server` → `rusqlite` ;
- la somme des BLOB est faite en `u32` **saturant**, comme l'incrément d'ADR-0117, décision 2 : un dépassement ne revient pas à zéro.

### Décision 13 — `crible stats` lit le fichier, en lecture seule, sans mode API

**Décision.** `crible stats` lit `analytics.sqlite` **en lecture seule** : aucune écriture, et un fichier absent rend un **message clair**, pas une erreur interne. Cette lecture est **permise à côté d'un serveur en marche**, au sens d'ADR-0116, décision 2 (a), et `docs/02` la cite. La commande n'a **aucun mode API** : aucune dépendance HTTP cliente n'est ajoutée. `analytics_dropped` est **absent** de la CLI.

Précisions :

- `docs/07:29-50` ne porte aucun client HTTP côté Rust, et en ajouter un pour lire un fichier local serait une dépendance sans emploi (règle de simplicité) ;
- le serveur en marche écrit le même fichier en WAL : un lecteur en `SQLITE_OPEN_READ_ONLY` ne le gêne pas et n'attend pas son verrou, comme `crible inspect` lit un index servi (ADR-0116) ;
- `analytics_dropped` mesure un **processus** (décision 9) : un fichier ne le porte pas, et la CLI ne peut donc pas le rendre. La commande ne l'invente pas et ne l'affiche pas ;
- un fichier absent (instance jamais démarrée avec les analytics, ou `--data` faux) rend un message qui nomme le fichier `analytics.sqlite` et le dossier de données donné, et dit qu'aucune analytics n'a été écrite ; le cas n'est pas une panne ;
- un fichier d'une `format_version` inconnue est refusé par le contrôle existant (INV-15), avec le message d'ADR-0117, décision 1 ;
- la CLI n'écrit **jamais** dans `analytics.sqlite` : ni schéma, ni purge, ni ligne.

### Décision 14 — Options et formats de `crible stats`

**Décision.** `crible stats` prend `--data`, comme les autres commandes, puis `--from`, `--to`, `--index`, `--operation` et `--group-by`, avec la **même sémantique** que la route. `--format table` est le **défaut** : des tableaux ASCII rendus à la main, **sans aucune dépendance**. `--format json` rend **exactement** le corps de la route, **moins `analytics_dropped`**.

Précisions :

- les noms d'options suivent le `clap` de la CLI (`--group-by` en kebab-case pour `group_by`), et les valeurs de dates sont analysées par le même code que celui de la route quand il est partageable ;
- `--format table` est le défaut parce que la fiche dit « affiche des tableaux » (`P7:107`), à l'inverse de `crible search`, dont le défaut est JSON parce que sa sortie est une réponse d'API (`crates/crible/src/main.rs:209-212`) ;
- les tableaux sont **rendus à la main** : `docs/07:29-50` ne porte aucune crate de tableau, et en ajouter une pour aligner des colonnes serait une dépendance sans nécessité ;
- `--format json` sert la comparaison avec la route (décision 15) : à `analytics_dropped` près, les deux corps sont identiques, et le test le vérifie ;
- aucun autre format, aucun autre drapeau : ni `--host`, ni `--token`, ni `--output`.

### Décision 15 — Preuve « chiffres exacts »

**Décision.** La preuve se fait sur un **trafic de test à durées choisies tombant dans des seaux connus**, avec un **vidage forcé avant lecture**, et la comparaison aux **bornes hautes** des seaux. La route et la CLI sont comparées sur le **même trafic**.

Précisions :

- les durées sont choisies pour que le rang le plus proche tombe dans un seau **déterminé d'avance**, y compris une durée sous 0,05 ms (seau 0) et une durée au-delà du seau 62 (seau 63, ADR-0117, décision 2) ;
- le vidage forcé évite de dépendre du minuteur de 10 s (décision 11) : le test lit ce qu'il vient d'écrire, sans attente ;
- la comparaison porte sur les comptes, les codes d'erreur, les taux, les percentiles et les deux classements : chaque valeur attendue est calculée depuis les bornes des seaux, jamais recopiée d'une sortie ;
- l'égalité entre la route et la CLI se vérifie sur le même fichier et le même trafic, à `analytics_dropped` près (décision 14) ;
- le test est **ordinaire**, en CI : il ne demande ni modèle ni échantillon Legifacile, et n'ajoute aucun `--skip` à la liste fermée de `docs/07:148-150`.

### Décision 16 — Rôle `admin`, OpenAPI et exception retirée

**Décision.** `GET /v1/stats` est réservé au rôle **`admin`** : une clé `read` ou `write` reçoit `403 forbidden` (ADR-0095, `docs/05:13`). La route entre dans la spécification OpenAPI, et son **exception nommée quitte le test des routes dans le même changement** (ADR-0111, complément du 2026-10-30). Les composants sont **fermés**.

Précisions :

- les composants de la réponse, du seau, des deux classements et de l'entrée journalière portent `additionalProperties: false`, comme les autres composants d'ADR-0111 ;
- l'exemple de `docs/05` est un **JSON valide** associé à son composant par `<!-- openapi: StatsResponse -->`, et `@dev` le fait correspondre au composant généré, comme pour `explain` (ADR-0123) ;
- la correction de `docs/05:80` fait partie du même changement : le texte ne dit plus « en P6, `GET /v1/stats` (`ROUTE-STATS`) », puisque la route est livrée ;
- les codes d'erreur sont ceux de `docs/05` : `401`, `403`, `400 invalid_request` (décision 2), `500 internal`. Aucun code nouveau.

### OpenAPI

La réponse, le seau, les deux classements et l'entrée journalière deviennent des **composants fermés** (`additionalProperties: false`), conformément à ADR-0111. La route perd son exception dans le test qui compare `docs/05` à la spécification, et l'exemple de `docs/05` porte son association `<!-- openapi: … -->`.

## Complément du 2026-11-01 (relecture, tour 1)

Ce complément répond au point de la relecture du tour 1 de la tâche `ROUTE-STATS` sur le classement des formes. Il **ne réécrit aucune des seize décisions** ci-dessus ; il précise la portée du paramètre `operation` là où la décision 2 le laissait entendre plus large. La règle est retenue par l'orchestrateur, par délégation du propriétaire pour la suite de P7, et porte le **même statut d'acceptation par anticipation** que l'ADR.

### `top_filter_shapes` ignore `operation` (note à la décision 2)

La décision 2 fait de `operation` un filtre d'égalité de la route, et la décision 6 décrit le classement des formes sans dire sur quoi il porte. Le code tranche aujourd'hui autrement : `read_top_shapes` lit la table `filter_shapes` sans condition d'opération (`crates/crible-store/src/analytics.rs:1705`, `minute_conditions(filter, from, to, false)`), et cette table n'a pas de colonne d'opération, seules les recherches y écrivant une forme.

`top_filter_shapes` porte donc sur les **recherches de la fenêtre, quel que soit `operation`** : `?operation=compact` restreint les comptes, les erreurs, les percentiles, le taux sans résultat et `top_indexes`, mais rend toujours les formes des recherches. Le comportement est écrit tel quel dans `docs/05` et `docs/06` ; il n'est pas changé ici, et l'omettre quand `operation` n'est pas une recherche demanderait une décision de conception, donc un autre ADR.

## Conséquences

- **Positives** :
  - la forme de la réponse est fermée, sans `null` ni `0` inventé, et chaque membre se lit indépendamment ;
  - les paramètres sont ceux de `docs/06:66`, sans filtre supplémentaire : la lecture reste bornée par les dimensions stockées, et le coût ne dépend pas de la variété des filtres ;
  - « chiffres exacts » a un sens unique : exact sur la somme des seaux, à la résolution d'un seau, avec une valeur sur-estimée et jamais sous-estimée ;
  - la lecture ne prend pas le verrou de l'écrivain, ne force aucun vidage et n'écrit rien : la route peut être appelée sur une instance en service ;
  - `crible stats` ne demande aucune dépendance nouvelle et rend le même corps que la route en JSON ;
  - le `504` multi-index se répartit enfin par cible, ce qui rend le taux sans résultat et `top_indexes` comparables entre une panne et un trafic normal.
- **Négatives / coûts** :
  - une lecture sur 90 jours de trafic agrège beaucoup de lignes : c'est le prix d'une fenêtre bornée par la seule rétention, et aucune borne de fenêtre n'est ajoutée (décision 2) ;
  - `analytics_dropped` manque au JSON de la CLI : un script qui compare les deux corps doit en tenir compte, et le test le fait ;
  - le taux sans résultat pèse une recherche multi-index une fois par cible : il n'est pas le taux « par requête reçue », et `docs/06` le dit ;
  - la correction du `504` change le nombre d'événements d'une panne multi-index : les chiffres d'avant et d'après ne se comparent pas ligne à ligne ;
  - les percentiles de pool restent stockés sans lecteur : c'est une donnée écrite que rien ne rend aujourd'hui.
- **À surveiller** :
  - la durée d'une lecture sur la rétention complète à 8,58 M passages — un temps qui approcherait le délai d'une requête HTTP demanderait un ADR sur une borne de fenêtre ou une agrégation précalculée ;
  - le volume d'`analytics.sqlite` et le nombre de lignes de `minute_events` : la clé de ligne est bornée par les clés d'API, les index, les modes et les stratégies (ADR-0117, décision 6), mais la croissance sur 90 jours n'a pas été mesurée ;
  - l'égalité route/CLI à `analytics_dropped` près : un écart signalerait deux chemins d'agrégation, donc deux vérités ;
  - la règle d'un `index` sans événement (décision 2) : un nom historique reste lisible, et un consommateur qui attendrait `404` demanderait un ADR.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Rendre `null` ou `0` pour un membre sans valeur | ADR-0119, décision 3 : on omet, et un `0` inventé se lit comme une mesure |
| Borner la fenêtre (7 jours, 1 000 seaux) | `docs/06:66` ne pose aucune borne, et la rétention d'ADR-0118, décision 4, borne déjà ce que le fichier contient ; une borne nouvelle serait une clé de configuration sans besoin |
| Offrir `status`, `client`, `mode` en filtre | La liste de `docs/06:66` est fermée, et les quatre dimensions restent des clés de ligne, agrégées à la lecture |
| Rendre les percentiles de pool | Ni `docs/06:67-72` ni la fiche ne les demandent ; le seul endroit où le pool se lit est `pool_size` d'une réponse de recherche |
| Rendre une moyenne de latence à côté des percentiles | `docs/06:49` l'interdit, et une moyenne sur des seaux géométriques n'a pas de sens utile |
| Compter `zero_results` sur tous les statuts | Un `400` ou un `504` n'a rien cherché : il gonflerait le taux de « recherches sans résultat » |
| Rendre `zero_results` comme compte au lieu du taux | La fiche et `docs/06:69` demandent un taux ; le compte se relit en multipliant le taux par `count`, et un membre de plus n'ajoute rien |
| Contrôler `index` contre les index vivants | Les analytics sont un historique : un index supprimé y laisse ses lignes, et ce contrôle les rendrait illisibles ; il couplrait aussi une lecture de fichier à l'état d'attache des index |
| Rendre `404` pour un `index` sans événement | Un nom valide est un filtre, pas une ressource : une fenêtre vide est un résultat vide, et `404` confondrait « pas de trafic » avec « index absent » |
| Départager les deux classements par ordre d'insertion | Un ordre d'insertion dépend du hasard des minutes : le départage textuel rend la réponse reproductible et comparable |
| Combler les jours absents de `day_indexes` | ADR-0118, décision 5 (`:87`) : un trou dit la vérité, la table n'invente pas un état |
| Lire `analytics_dropped` dans le fichier | Le compteur est par processus et jamais persisté (ADR-0118, décision 2) : il n'y est pas |
| Réutiliser `read_minute_events` tel quel | Il charge toutes les lignes sans `WHERE` : la lecture d'une fenêtre deviendrait un balayage complet du fichier, mémoire comprise |
| Lire par `open_or_create` | Il écrit le schéma et purge : une lecture déclencherait une écriture, contre ADR-0116, décision 1 |
| Prendre le verrou de l'écrivain pour lire | Un lecteur n'écrit rien ; en WAL il ne bloque pas l'écrivain, et le verrou transformerait une lecture en attente |
| Faire lire `crible stats` par l'API | Aucun client HTTP dans `docs/07:29-50` ; la commande lit un fichier local, et ajouter un client serait une dépendance sans emploi |
| Rendre `analytics_dropped` dans la CLI | Il mesure un processus serveur : la CLI ne peut pas le connaître, et l'afficher à `0` serait un chiffre inventé |
| Faire de `--format json` le défaut de `crible stats` | La fiche dit « affiche des tableaux » (`P7:107`) ; le défaut JSON de `crible search` sert une réponse d'API |
| Ajouter une crate de tableau | `docs/07:29-50` n'en porte aucune ; aligner des colonnes ne justifie pas une dépendance |
| Laisser le `504` multi-index sur un seul événement | ADR-0117, décision 5, impose un événement par cible, et le statut d'un `504` ne se répartit pas sans cela |
| Reprendre le premier nom d'index du corps pour une requête non résolue | Ce nom n'a pas été résolu : le porter attribuerait une erreur à un index qui n'a pas été ouvert |
| Prouver les chiffres sur l'échantillon Legifacile | Un trafic à durées choisies prouve les bornes des seaux sans modèle ni données de production, et n'ajoute aucun `--skip` |

## Références

- Fiche : `docs/phases/P7-observabilite.md:101-111`.
- Conception : `docs/06-observabilite.md:13-74` (« Analytics », « Lecture »), `docs/05-api.md` (« Erreurs », « Endpoints », « Recherche : réponse »), `docs/02-architecture.md:164-176` (« Concurrence »), `docs/07-stack-et-deploiement.md:15` (liste des commandes) et `:29-50` (dépendances).
- ADR : ADR-0014, ADR-0095, ADR-0100, ADR-0103, ADR-0111 (complément du 2026-10-30), ADR-0116, ADR-0117, ADR-0118, ADR-0119, ADR-0122.
- Code : `crates/crible-store/src/analytics.rs:256-284`, `:362-370`, `:467`, `:649-670` ; `crates/crible-server/src/search_multi.rs:128-131`, `:139-152`, `:219-226` ; `crates/crible/src/main.rs:209-222`.
- Suivi : `docs/suivi.md:658` (écart du `504` multi-index et du `mode` vide, noté le 2026-10-31).
