# ADR-0120 — Fin de `took_ms`, attribution de l'ouverture des lecteurs et rendu hors chronomètre

- **Statut** : Accepté (accepté par anticipation par le propriétaire le 2026-10-31, confirmé par le propriétaire le 2026-11-01)
- **Date** : 2026-10-31
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, tâche REPONSE-DEBUG (phase P7), sur une `QUESTION-CONCEPTION` de `@dev` et le verdict ECART + NON-COUVERT de `@architecte`, en application de la délégation du propriétaire du 2026-10-31 pour la série de P7
- **Remplace** : — (ne remplace aucune décision : comble une lacune d'ADR-0119, qui ne dit ni où `took_ms` s'arrête exactement ni à quelle clé appartient l'ouverture des lecteurs, sans modifier le corps d'aucun ADR accepté)

## Contexte

### Ce que la fiche exige, et ce que la mesure montre

La fiche `REPONSE-DEBUG` (`docs/phases/P7-observabilite.md:74-82`) demande que « la somme des étapes reste cohérente avec le total (écart inférieur à 10 %) » (`:80`). ADR-0119, décision 6, a fixé la somme : c'est le chemin critique `encode + filter + max(dense, lexical) + fusion + group + fetch`, `validate` exclu, comparé à `took_ms`.

Le critère ne tient pas en mode `dense` avec le code livré par REPONSE-DEBUG : la somme du chemin critique ne vaut que **82 %** de `took_ms`, soit un écart de **18 à 27 %**. En `lexical` et en `hybride`, l'écart retombe à **2 à 4 %**. Le mode `dense` n'est donc pas le plus lent : c'est celui où **deux travaux réels entrent dans `took_ms` sans clé d'étape**.

### Les deux travaux sans clé

**1. `DenseSearcher::open`.** `search_index` ouvre les lecteurs de segments et le lecteur de charges utiles avant de lancer le pipeline (`crates/crible-server/src/search.rs:743-744`), et cette ouverture est comprise dans le chronomètre `started`. Elle coûte **0,6 ms pour 4 000 passages** et **2,3 ms pour 20 000 passages**, et **croît avec le corpus**. En `dense`, où le reste du pipeline est court, cette constante pèse 18 à 27 % du total. En `lexical` et en `hybride`, le reste du travail est plus long et la même constante reste dans les 2 à 4 %.

**2. Le rendu JSON des résultats.** `results_json` (`crates/crible-server/src/search.rs:618-627`) construit la réponse — champs rendus, texte, forme plate — et il est appelé **avant** `started.elapsed()` (`crates/crible-server/src/search.rs:281`). Le rendu n'est donc pas mesuré par une étape, mais il est compté dans `took_ms`.

### Ce que la conception et le code disent déjà

- `docs/03-recherche.md:5-15` énumère les neuf clés et ne rattache **aucune** ouverture de lecteur à une clé. `:14` définit `fetch` comme la lecture des champs et du texte des `k` résultats, et cette étape est **toujours** exécutée.
- Le lecteur ouvert par `DenseSearcher::open` sert aussi la lecture des charges utiles (`crates/crible-search/src/dense/exact.rs:148-153`), et `exact.rs:152` dit déjà que les lecteurs sont « kept for every later query ». En `lexical`, il sert d'abord le hachage de regroupement (`crates/crible-search/src/pipeline.rs:410`), puis les charges utiles (`:419`).
- La CLI arrête son total **avant** de construire et d'imprimer le JSON (`crates/crible/src/commands.rs:827`, puis `:836`). ADR-0109 `:35` dit que le total de la CLI comprend l'ouverture des lecteurs et que la route doit mesurer la même chose. `docs/05-api.md:299` énumère ce que `took_ms` couvre **jusqu'à la lecture des payloads**.
- Le correctif vit dans `search_index` (`crates/crible-server/src/search.rs:723-772`), partagé par la route simple et par chaque cible de la route multi-index (`crates/crible-server/src/search_multi.rs:589-607`).

### L'écart que cet ADR tranche

ADR-0119, décision 6, dit que « le total est l'instant de `took_ms` » et que `fetch` « couvre la lecture des payloads », sans dire si le rendu de la réponse appartient au total ni où l'ouverture des lecteurs se rattache. Avec le code tel qu'il est, le critère de 10 % échoue en `dense` pour deux raisons distinctes, et l'une des deux — le rendu — n'est pas du travail de recherche.

## Décision

### Décision 1 — `took_ms` s'arrête à la fin de `fetch`, avant le rendu JSON

**Décision.** `took_ms` s'arrête à la **fin de `fetch`**, donc **avant** le rendu JSON des résultats. Le rendu n'est plus chronométré : il n'appartient ni à `took_ms`, ni au membre `total` de `debug.timings_ms`, ni à la colonne `total` des analytics.

Précisions :

- c'est **une seule mesure** qui sert aux trois usages (ADR-0119, décision 7) : l'instant du total est arrêté une fois, à la fin de `fetch`, et la réponse comme l'événement d'analytics en dérivent ;
- la règle aligne la route sur la CLI, qui arrête déjà son total avant de construire le JSON (`crates/crible/src/commands.rs:827`, `:836`), et sur `docs/05-api.md:299`, qui énumère `took_ms` jusqu'à la lecture des payloads ;
- l'**échéance** (`timeout_ms`, ADR-0109) et le **jeton** de concurrence ne changent pas : ils couvrent toujours tout le travail de la requête, rendu compris. Un rendu lent peut donc encore rendre un `504` ; il n'allonge plus `took_ms` ;
- le rendu ne disparaît pas de la réponse : il reste dans le handler, après l'arrêt du chronomètre.

### Décision 2 — l'ouverture des lecteurs reste dans `took_ms`, rattachée à la clé `fetch`

**Décision.** L'ouverture des lecteurs de segments et du lecteur de charges utiles reste **dans** `took_ms`, comme le veut ADR-0109 `:35`, et sa durée est rattachée à la clé **`fetch`**, dans **tous** les modes.

Précisions :

- la règle est **uniforme** : en `dense`, en `lexical` et en `hybride`, la même clé porte la même dépense, et un changement de mode ne déplace pas l'ouverture d'une clé à l'autre ;
- `fetch` est choisie parce qu'elle est **toujours exécutée**, quel que soit le mode, et parce que le lecteur ouvert sert effectivement la lecture des payloads (`crates/crible-search/src/dense/exact.rs:148-153`) ; en `lexical`, il sert aussi le hachage de regroupement (`crates/crible-search/src/pipeline.rs:410`) avant les charges utiles (`:419`) ;
- la décision est **provisoire** et son coût analytique est écrit noir sur blanc : la clé `fetch` porte désormais une **constante par requête qui croît avec le corpus**, et non plus seulement le coût des charges utiles. Dans les histogrammes d'analytics de `docs/06`, `fetch` se lit comme « ouverture + charges utiles » jusqu'au cache de la décision 3, et son p50 ne mesure pas la lecture des `k` résultats seule ;
- `filter` n'est pas retenue : elle mesure le pool, calculé une seule fois par requête (INV-05), et l'ouverture des lecteurs n'a lieu qu'après elle ;
- aucune clé nouvelle n'est ajoutée : la forme des neuf clés d'ADR-0119, décision 5, et la forme de `StageDurations` (ADR-0117, décision 5) sont inchangées.

### Décision 3 — le cache du `DenseSearcher` par génération est la cible retenue en principe

**Décision.** La cible est un **cache du `DenseSearcher`** — lecteurs de segments et lecteur de charges utiles — **par génération** d'index. Il supprime la dépense au lieu de la renommer, et rendra à `fetch` son sens de `docs/03-recherche.md:14`.

Précisions :

- la tâche qui l'implémente **n'existe pas encore** : elle est à créer sur décision du propriétaire, **hors de REPONSE-DEBUG** ;
- quand elle sera faite, l'ouverture disparaîtra du chemin de la requête et la règle de la décision 2 deviendra **sans objet** ; `fetch` ne mesurera plus que la lecture des charges utiles ;
- la clé du cache est la génération servie : un changement de génération invalide l'entrée, et un instantané de lecture reste tenu du début à la fin d'une requête ;
- cet ADR ne fixe ni la forme du cache, ni sa politique d'éviction, ni son coût mémoire : ce sera l'objet de la tâche et, si elle engage la conception, d'un nouvel ADR.

### Décision 4 — dette consignée : `debug` en multi-index passe `capture: None`

**Décision.** La promesse d'ADR-0119, décision 8, selon laquelle « JOURNAL-DE-DEBUG n'aura pas à modifier le pipeline » ne vaut que pour la **route simple**. En multi-index, REPONSE-DEBUG passe `capture: None` (`crates/crible-server/src/search_multi.rs:590-607`).

Précisions :

- c'est une **dette consignée**, pas un oubli : le pipeline de chaque cible reçoit une capture vide, et aucun `top_n` de branche n'est collecté pour la route multi-index ;
- **JOURNAL-DE-DEBUG** définira le journal multi-index, avec l'une des deux formes : une capture **par cible**, ou un journal **sans capture** qui s'en tient au plan et aux temps ;
- la dette ne change ni la réponse, ni `took_ms`, ni les analytics, et elle ne bloque pas REPONSE-DEBUG.

### Décision 5 — le test du critère reste le même, sans seuil abaissé

**Décision.** Le test `the_critical_path_stays_within_ten_percent_of_the_total` (`crates/crible-server/tests/search_debug.rs`) reste la preuve du critère d'acceptation, **sans seuil abaissé**.

Précisions :

- le seuil de 10 % d'ADR-0119, décision 6, ne bouge pas ; c'est la mesure qui est corrigée, pas le critère ;
- un dépassement après application de cet ADR dit encore qu'une étape n'est pas mesurée ou que le total n'est pas pris au bon instant (ADR-0119, `:112`) ;
- le correctif vit dans `search_index` (`crates/crible-server/src/search.rs:723-772`), donc il vaut pour la route simple **et** pour chaque cible de la route multi-index (`crates/crible-server/src/search_multi.rs:589-607`).

## Conséquences

- **Positives** :
  - le critère de cohérence des temps devient **atteignable en `dense`**, le mode où il échouait, sans toucher au seuil ni au critère ;
  - `took_ms` mesure du **travail de recherche**, et non la construction de la réponse : la route rejoint la CLI et `docs/05:299` ;
  - l'ouverture des lecteurs a une **clé**, donc les neuf temps couvrent tout ce qui est chronométré, et le total n'a plus de travail invisible ;
  - la règle de la décision 2 est uniforme : un changement de mode ne déplace pas une dépense d'une clé à l'autre ;
  - la cible est écrite (décision 3) : la règle provisoire a une fin annoncée, et `fetch` retrouvera son sens de `docs/03:14`.
- **Négatives / coûts** :
  - la clé `fetch` des analytics devient **ambiguë** jusqu'au cache : elle porte une constante par requête qui croît avec le corpus, et son histogramme se lit « ouverture + charges utiles » ;
  - `took_ms` n'est plus comparable à la durée murale d'un appel HTTP : la sérialisation, le rendu JSON et l'écriture de la réponse sortent du total ;
  - l'échéance et le total ne couvrent plus exactement la même chose : une requête peut rendre un `504` à cause d'un rendu lent, alors que `took_ms` d'une requête réussie aurait été court. C'est assumé : l'échéance protège la requête entière, le total décrit la recherche ;
  - la dette multi-index (décision 4) laisse JOURNAL-DE-DEBUG avec une décision de forme à prendre.
- **À surveiller** :
  - le test `the_critical_path_stays_within_ten_percent_of_the_total` : s'il échoue encore en `dense` après le correctif, c'est une troisième dépense sans clé, à identifier avant de toucher au seuil ;
  - l'histogramme `fetch` des analytics : un p50 qui monte avec la taille du corpus, à requête et `k` constants, confirme que la décision 2 renomme une constante plutôt qu'elle ne la supprime, et donne la priorité au cache de la décision 3 ;
  - la latence du rendu JSON, désormais hors total : si elle devient visible côté client sans apparaître dans `took_ms`, c'est un besoin de mesure séparée, pas une raison de rallonger le total ;
  - `debug` multi-index : tant que JOURNAL-DE-DEBUG n'a pas tranché, aucune capture de branche n'existe sur cette route.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Laisser le rendu JSON dans `took_ms` et élargir le critère de 10 % | Le seuil d'ADR-0119, décision 6, est un critère d'acceptation, pas un réglage : l'élargir ferait passer le test en changeant la question. Le rendu n'est pas du travail de recherche, et la CLI l'exclut déjà. |
| Rattacher l'ouverture des lecteurs à `filter` | `filter` mesure le calcul du pool (INV-05) et l'ouverture a lieu après lui : la clé mentirait sur ce qu'elle mesure, et le pool serait gonflé d'une constante croissante avec le corpus. |
| Créer une dixième clé `open` | La forme des neuf clés est fermée par ADR-0119, décision 5, et par les composants OpenAPI d'ADR-0119, décision 11 ; une clé nouvelle demande un ADR et change la forme de `timings_ms` pour tous les clients. La décision 3 supprime la dépense, ce qui est préférable à la nommer. |
| Sortir l'ouverture des lecteurs de `took_ms` | ADR-0109 `:35` dit que le total de la CLI la comprend et que la route doit mesurer la même chose ; la sortir créerait deux totaux qui divergent à mesure que le corpus grandit. |
| Rendre `took_ms` comme la durée murale complète de l'appel HTTP | Ce serait une mesure de service, non une mesure de recherche : elle dépendrait de la sérialisation, du client et du réseau, et contredirait la CLI et `docs/05:299`. |
| Garder le rendu hors total **et** hors échéance | L'échéance protège la requête entière (ADR-0109) ; un rendu qui échappe à l'échéance ferait tenir un jeton au-delà de `timeout_ms`, ce que la décision d'ADR-0109 sur le jeton interdit. |
| Résoudre la dépense d'ouverture dans REPONSE-DEBUG | La fiche REPONSE-DEBUG porte l'objet `debug`, pas le cache du moteur ; un cache par génération touche `DenseSearcher`, la publication d'instantanés et la mémoire, et mérite sa propre tâche et sa propre preuve. |
| Ouvrir les lecteurs une fois pour toutes au démarrage du serveur | Les lecteurs appartiennent à une époque et à une génération : une attache, une compaction ou une réouverture change les fichiers, et un lecteur tenu hors du cycle de publication servirait une génération morte. |
| Passer une capture en multi-index pour tenir la promesse d'ADR-0119, décision 8 | La route multi-index a une cible par index et un seul jeton : une capture par cible demande une agrégation et une forme de journal que la fiche de JOURNAL-DE-DEBUG doit d'abord définir. La dette est consignée plutôt que comblée à l'aveugle. |

## Références

- `docs/phases/P7-observabilite.md:74-82` — fiche `REPONSE-DEBUG` : objectif (`:75`), dépendance (`:76`), lectures (`:77`), livrables (`:78`), critère de cohérence des temps (`:80`), critère des rangs (`:81`).
- `docs/03-recherche.md:5-15` (neuf clés de temps, `fetch` en `:14`), `:17` (étapes selon le mode).
- `docs/05-api.md:299` (`took_ms`), `:319` (chemin critique et `total`), `:384` (`took_ms` de la route multi-index), `:406` (`total` d'une cible), `:416` (échéance).
- `docs/06-observabilite.md:19` (histogrammes de latence, clé `fetch`), `:49` (une étape non exécutée ne produit aucun échantillon).
- Code : `crates/crible-server/src/search.rs:281` (arrêt du total), `:618-627` (`results_json`), `:723-772` (`search_index`, ouverture des lecteurs), `crates/crible-server/src/search_multi.rs:589-607` (cible de la route multi-index, `capture: None`), `crates/crible/src/commands.rs:827` et `:836` (total de la CLI, avant le rendu), `crates/crible-search/src/dense/exact.rs:148-153` (ouverture des lecteurs, gardés pour les requêtes suivantes), `crates/crible-search/src/pipeline.rs:410` et `:419` (hachage de regroupement puis charges utiles en lexical).
- ADR-0109 (échéance et jeton, `took_ms`, preuve API = CLI), ADR-0117 (décision 5, `StageDurations`), ADR-0119 (décisions 5 à 8 : neuf clés, chemin critique, une seule mesure, capture), ADR-0110 (recherche multi-index).

## Documents mis à jour par cet ADR

| Document | Modification |
|---|---|
| `docs/03-recherche.md:14` | une phrase à la clé `fetch` : elle comprend l'ouverture des lecteurs, provisoirement (décision 2) |
| `docs/05-api.md:299`, `:319` | `took_ms` et `timings_ms.total` s'arrêtent à la fin de `fetch`, avant le rendu JSON (décision 1) |
| `docs/05-api.md:384`, `:406` | même fin pour la route multi-index et pour le `total` d'une cible (décision 1) |
| `docs/06-observabilite.md:19` | la clé `fetch` porte provisoirement l'ouverture des lecteurs : lecture des histogrammes (décision 2) |
| `docs/phases/P7-observabilite.md:77-80` | la fiche `REPONSE-DEBUG` renvoie à ADR-0120 pour la fin de `took_ms` et l'attribution de l'ouverture |
| `docs/adr/README.md` | ligne d'index de cet ADR |

Sont **non modifiés** : le corps d'ADR-0119 et de tout autre ADR accepté, `docs/08`, `docs/suivi.md` (tenu par l'orchestrateur, qui y cite ADR-0120 dans la section « Écarts à la conception »), et tout fichier de code.

Les numéros de ligne des documents cités sont ceux de leur **état d'avant application** : la mise à jour de ces documents décale les sections suivantes, et c'est la modification ci-dessus qui fait foi.
