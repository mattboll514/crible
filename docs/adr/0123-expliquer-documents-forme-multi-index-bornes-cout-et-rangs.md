# ADR-0123 — Route d'explication `explain_ids` : forme de la réponse, multi-index, bornes, coût et rangs

- **Statut** : Accepté (accepté par anticipation par le propriétaire le 2026-11-01, délégation du propriétaire pour la suite de P7 ; validation officielle à la fin de la série)
- **Date** : 2026-11-01
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, tâche EXPLIQUER-DOCUMENTS (phase P7), sur l'analyse de `@architecte` et les décisions retenues par l'orchestrateur, en application de la délégation du propriétaire du 2026-11-01 pour la série EXPLIQUER-DOCUMENTS, ROUTE-STATS, COMMANDE-EVAL, REUTILISER-LECTEURS-DENSES
- **Remplace** : — (ne remplace aucune décision ; précise ADR-0041, ADR-0079, ADR-0109, ADR-0110, ADR-0111, ADR-0117, ADR-0119, ADR-0120 et ADR-0121 sans modifier le corps d'un ADR accepté)

## Contexte

### La fiche et ce qu'elle demande

La fiche `EXPLIQUER-DOCUMENTS` (`docs/phases/P7-observabilite.md:86-98`) a pour objet `explain_ids`. Ses livrables sont, pour chaque document demandé : s'il existe et s'il est vivant (`:91`) ; s'il est dans le pool et, sinon, les clauses qui l'excluent, **chaque clause évaluée séparément** (`:92`) ; son rang dense exact par scan exact du pool, son rang binaire et sa présence parmi les N candidats (`:93`) ; son rang et son score BM25 « avec l'explication tantivy » (`:94`) ; son rang fusionné et s'il a été absorbé par un regroupement (`:95`).

Ses critères sont : un test par cas — exclu par le filtre, mort, hors des N candidats binaires, rang lexical seul, absorbé par un regroupement (`:97`) —, et « le cas « rang 70 en dense, rang 2 en lexical » de l'échantillon est expliqué » (`:98`). La fiche ne cite aucun invariant ; elle ne demande ni route nouvelle, ni drapeau CLI.

### Ce que la conception pose

`docs/05:271` déclare le champ : `explain_ids`, « liste de `doc_id` ou de clés `doc_id#ord` », valeur par défaut `[]`, rôle « diagnostic de documents précis ». `docs/05:278` le refuse aujourd'hui en `invalid_request` avec un message qui dit que le champ n'est pas encore pris en charge, jusqu'à cette tâche (ADR-0109), et `docs/05:356` dit la même chose pour la route multi-index.

`docs/05:327-342` donnait le seul exemple existant, marqué `<!-- openapi: none — le bloc explain arrive avec EXPLIQUER-DOCUMENTS (P7) -->` (`:329`). `docs/05:340` réservait le scan exact à `explain_ids` en citant ADR-0109. `docs/05:410` donnait la forme que prendrait l'entrée multi-index, `{ "index": "...", "id": "..." }`, et renvoyait à ADR-0110, dont le `:180` renvoie lui-même la forme de la réponse à P7. `docs/05:344-365` fixe le corps, la fusion et l'ordre des validations de la recherche multi-index.

`docs/06:96-106` décrit le diagnostic en six lignes et conclut : « le rang dense exact impose un scan exact du pool, d'où un coût réservé au debug » (`:106`).

Les ADR voisins bornent le reste. ADR-0041 fixe `exists` (« la ligne `documents` est présente ») et `alive` (« le document a au moins un passage vivant »), deux booléens distincts. ADR-0079 rend le classement total : score décroissant, puis **clé textuelle** croissante (`:50`, `:117`) ; l'iid ne départage qu'en interne, et il ne sort jamais du moteur. ADR-0109 fixe `took_ms` et le refus des champs inconnus. ADR-0110 fixe le corps et la fusion multi-index. ADR-0111 exige que chaque exemple de `docs/05` soit un JSON valide associé à un composant fermé. ADR-0117, décision 5, ferme `StageDurations` à neuf clés. ADR-0119, décision 3, impose d'**omettre** un membre sans valeur, jamais `null`, jamais `0` ; sa décision 10 dit que `debug` ne change ni les résultats ni `took_ms` ; sa décision 12 refuse un drapeau `--debug` dans la CLI. ADR-0120, décision 1, arrête `took_ms` à la fin de `fetch`. ADR-0121 ferme la liste du payload du journal, où `explain_ids` n'entre pas (`:121`).

### L'état du code aux sources citées

- `crates/crible-search/src/lexical.rs:155-160` : une clé mal formée fait échouer `PassageKey::parse` et remonte en `invalid_request`. Une clé valide absente de l'état rend `None`, sans erreur (`:146-152`).
- `crates/crible-search/src/dense/binary.rs:85-95` : la sélection binaire ordonne par distance croissante, puis par **iid** croissant. Ce départage est interne et ne peut pas être l'ordre d'un rang rendu.
- `crates/crible-core/src/filter.rs:99-108` : chaque `Condition` porte son chemin JSON complet, produit par le parseur. Les chemins de `$or` et `$not` existent donc déjà ; c'est leur forme rendue qui n'était pas fixée.
- `crates/crible-search/src/lexical.rs` expose aujourd'hui un `Explanation` tantivy brut, non borné, pour le diagnostic de la branche lexicale.

### Les écarts et les points non couverts

**Écart de forme — `null` dans l'exemple.** `docs/05:331-338` rendait `"fused_rank": null` et `"grouped_under": null`, ce que `docs/05:410` et ADR-0119, décision 3, interdisent : on omet un membre sans valeur. Le membre `id` de l'exemple ne dit pas non plus s'il porte un `doc_id` ou une clé `doc_id#ord`, alors que `docs/05:271` accepte les deux (`:1` de l'analyse).

**Écart de coût — la citation d'ADR-0109.** `docs/05:340` justifie le scan exact en citant ADR-0109, qui ne parle pas d'`exact_rank` ; la fiche impose ce rang sous toute stratégie (`P7:93`). Le coût doit donc être fixé ici, et la citation corrigée (`:4` de l'analyse).

**Écart de critère — le cas « rang 70 / rang 2 ».** Ce cas vient de l'anecdote d'ADR-0008 `:10` et d'ADR-0001 `:26` (article 1240) ; aucun document de conception ni `tools/` ne le définit. Sur l'échantillon, MES-10 (`docs/mesures/MES-10-qualite-fusion.md:124-159`) mesure le **même** article à rang dense 1 et lexical 5 pour la variante V3 ; le cas dense 70 / lexical 2 n'y existe pas. Un test sur `data/legifacile-sample` serait un **sixième** test local hors de la liste fermée des cinq exclus (`docs/07:148-150`, ADR-0057, ADR-0062, ADR-0063, ADR-0085, ADR-0109) (`:11` de l'analyse).

**Points non couverts.** Multi-index (forme de la requête et de la réponse, `:2`) ; bornes (nombre d'ids, doublons, id vide, `:3`) ; échelle et définition des rangs dense exact, binaire, lexical et fusionné, et contenu de `grouped_under` (`:5`) ; contenu fermé de l'explication lexicale, à la place de l'`Explanation` tantivy brut (`:6`) ; forme d'`excluded_by` sous `$or` et `$not` (`:7`) ; interaction avec le journal et les analytics (`:8`) ; confidentialité (`:9`) ; CLI (`:10`).

**Déjà tranché.** Le départage textuel contre `PassageKey::Ord` (note du 2026-10-22 de `docs/suivi.md`) est réglé par ADR-0079 : le classement départage sur la clé textuelle, l'iid ne départage qu'en interne (`:12` de l'analyse). Cet ADR le note sans rouvrir ADR-0079.

## Décision

### Décision 1 — Forme fermée de `explain`

**Décision.** `explain` est un tableau **frère** de `results`, avec **une entrée par id demandé, dans l'ordre de la requête**. Un membre sans valeur est **omis**, jamais rendu `null` ni `0` (ADR-0119, décision 3).

Précisions :

- une entrée porte, dans cet ordre : `id` (l'identifiant **tel que demandé**, `doc_id` ou clé `doc_id#ord`), `doc_id`, `exists`, `alive`, `in_pool`, et `excluded_by` ;
- `alive` est absent quand `exists` est faux : la question n'a pas de sens sur un document inexistant (ADR-0041) ;
- `in_pool` se lit sur l'id demandé : pour une **clé**, le passage lui-même est dans le pool filtré ; pour un **`doc_id`**, au moins un de ses passages y est. `excluded_by` suit la même unité : les feuilles qui échouent pour ce passage, ou pour le document quand l'id est un `doc_id` ;
- `excluded_by` n'est présent que lorsque `in_pool` est faux ; il est absent d'une entrée dans le pool ;
- suivent les objets de branche — `dense`, `lexical`, `fused` — puis `grouped_under` ;
  - `dense` : `{ "key", "exact_rank", "binary_rank", "in_binary_candidates" }` ;
  - `lexical` : `{ "key", "rank", "score", "terms" }` ;
  - `fused` : `{ "rank" }` ;
- `grouped_under` porte la clé du passage retenu qui a absorbé celui-ci ;
- une branche qui n'a pas tourné n'a pas d'objet : en mode `lexical`, `dense` est absent ; en mode `dense`, `lexical` est absent ; `fused` suit ADR-0119, décision 2 : il reste présent avec le rang de la branche unique, et il est absent seulement quand le passage n'est pas dans la liste fusionnée ;
- **quand l'id demandé est un `doc_id`**, chaque branche décrit le **passage le mieux classé** du document dans cette branche, et le nomme par sa `key`. Un document peut donc avoir trois `key` différentes, une par branche ;
- `explain` n'apparaît que si `explain_ids` est présent et non vide, et n'apparaît jamais dans une réponse d'erreur, `504` compris (même règle que `debug`, ADR-0119, décision 10).

### Décision 2 — Multi-index : `{ "index", "id" }` et liste plate

**Décision.** Sur `POST /v1/search`, chaque élément de `explain_ids` est un objet `{ "index", "id" }`. `index` doit être l'une des cibles de `targets` ; sinon la requête est refusée en `400 invalid_request`, avec le chemin JSON (`$.explain_ids[i].index`). La réponse est une **liste plate**, dans l'ordre de la requête, et chaque entrée porte `index` en plus des membres de la décision 1.

Précisions :

- sur la route mono-index (`POST /v1/indexes/{index}/search`), `explain_ids` reste la liste de ids de `docs/05:271`, et `index` est absent des entrées ;
- la forme `{ "index", "id" }` est celle que `docs/05:410` annonçait, sur le renvoi d'ADR-0110 `:180` ;
- deux entrées qui nomment la même cible et le même id suivent la règle des doublons de la décision 3 ;
- les validations d'ADR-0110 gardent leur ordre : la forme d'`explain_ids` est contrôlée avec le corps, en `400`, avant l'existence des index (`docs/05:377`).

### Décision 3 — Bornes d'`explain_ids`

**Décision.** Une requête demande **au plus 100 ids**. Au-delà, elle est refusée en `400 invalid_request` sur `$.explain_ids`. Les doublons sont **fusionnés en gardant la première occurrence**. Un id vide ou une clé mal formée rend `400 invalid_request` en nommant `$.explain_ids[i]`. Un `doc_id` inconnu n'est pas une erreur : il rend `exists: false`.

Précisions :

- la borne de 100 est une **constante**, sans clé de configuration : `docs/07` ne porte aucune clé pour ce champ, et la règle de simplicité interdit une clé pour une valeur fixe ;
- « 100 ids » compte les ids de la requête **après fusion des doublons** ; en multi-index, c'est le total toutes cibles confondues ;
- la fusion garde l'ordre de la requête, donc l'ordre des entrées de `explain` est celui des premières occurrences ;
- une clé mal formée est celle que `PassageKey::parse` refuse (`crates/crible-search/src/lexical.rs:155-160`) : la requête est refusée, jamais rendue en `exists: false` ;
- un `doc_id` qui a existé puis a été supprimé suit ADR-0041 : `exists: false`, sans `alive` ;
- aucune borne propre à une cible : la borne est celle de la requête.

### Décision 4 — Coût : après `fetch`, hors des durées, sous l'échéance

**Décision.** L'explication se calcule **après `fetch`**, hors de `took_ms` et hors de `StageDurations`, qui reste fermé à **neuf clés** (aucune dixième clé), mais **sous l'échéance de la requête** : une explication qui ne finit pas dans `timeout_ms` rend le même `504 timeout` qu'une recherche lente. Le **rang dense exact est toujours calculé par un scan exact du pool filtré**, quelle que soit la stratégie dense, et **seulement quand `explain_ids` est présent**.

Précisions :

- l'ordre est celui d'ADR-0120, décision 1 : `took_ms` s'arrête à la fin de `fetch`, avant le rendu JSON. L'explication vient après, comme un travail de diagnostic ;
- elle ne produit donc aucun échantillon d'analytics et ne fausse ni `total_ms` du journal (qui vaut `took_ms`, ADR-0121), ni le chemin critique d'ADR-0119, décision 6 ;
- l'échéance d'ADR-0102 et d'ADR-0109 couvre **tout** le travail de la requête, rendu compris : le jeton de concurrence reste tenu jusqu'à la fin réelle, y compris après un `504` ;
- le rang dense exact ne peut pas être déduit du top des N candidats : le rang 70 d'un passage peut sortir de N. Il demande le score exact de tout le pool filtré, comme EXACT (ADR-0072, décision 3), avec le même départage textuel ;
- le scan exact est **partagé** par tous les ids d'une même cible : le vecteur de requête est le même, un seul parcours suffit, et le coût ne croît donc pas avec le nombre d'ids. Ce sont les feuilles du filtre qui sont évaluées par id ;
- `docs/05:340` est corrigé : la réservation du scan exact à `explain_ids` renvoie désormais au présent ADR, et non à ADR-0109.

### Décision 5 — Rangs : définitions, échelles et `grouped_under`

**Décision.** Tous les rangs rendus partent de **1** et ont les définitions suivantes.

- **`dense.exact_rank`** : rang du passage sur **tout le pool filtré**, au score dense exact, **indépendant de `k`**, l'égalité étant départagée par la clé textuelle (ADR-0079, `:50`).
- **`dense.binary_rank`** : rang du passage dans l'**ordre exact de la sélection binaire** sur le pool filtré — distance croissante, départage interne sur l'iid compris (`crates/crible-search/src/dense/binary.rs:85-95`) —, indépendant de `k`. Il est rendu sous stratégie binaire, que le passage soit retenu ou non.
- **`dense.in_binary_candidates`** : vrai **si et seulement si** le passage figure parmi les candidats retenus, c'est-à-dire si son `binary_rank` est au plus le nombre **effectif** de candidats (`max(N, candidates_per_branch)`, ADR-0074 ; ADR-0075). Il est rendu sous stratégie binaire.
- **`dense.binary_rank` et `dense.in_binary_candidates` sont absents sous stratégie exacte** : il n'y a pas de sélection binaire, et un rang ou un booléen faux y serait faux.
- **`lexical.rank`** : rang BM25 du passage parmi les passages du pool qui ont un score, **indépendant de `k`**.
- **`fused.rank`** : rang dans la liste fusionnée **avant regroupement**, la même échelle que `fused_rank` d'ADR-0119, décision 2 ; il est absent si le passage n'y figure pas.
- **`grouped_under`** : la clé du passage rendu dans `results` qui a absorbé celui-ci par `dedup: "content"` ou par `group_by: "document"` ; il est absent quand le passage n'a pas été absorbé.

Précisions :

- ces rangs **ne sont pas sur la même échelle** que `per_result.dense_rank` et `per_result.lexical_rank` de `debug`, qui sont des rangs dans la liste de `candidates_per_branch` entrées (ADR-0119, décision 1). Un `exact_rank` de 70 et un `dense_rank` de 70 ne désignent pas la même liste, et la documentation le dit ;
- un passage dont le `binary_rank` dépasse le nombre effectif de candidats porte `in_binary_candidates: false` : c'est le cas « hors des N candidats binaires » de la fiche, et son rang binaire reste lisible ;
- en multi-index, les rangs sont ceux de l'index de l'entrée ; `fused.rank` est la fusion **de l'index**, avant son regroupement, comme `fused_rank` en multi-index (ADR-0119, décision 9) ; le rang dans la fusion entre index n'est pas rendu ;
- `grouped_under` renvoie une clé, jamais un iid (INV-01) ; la clé est celle du `results` de la même réponse. Quand `dedup` et `group_by` s'enchaînent (ADR-0075), c'est le passage finalement rendu qui est nommé.

### Décision 6 — Explication lexicale fermée, sans `Explanation` tantivy brut

**Décision.** `lexical.terms` est une liste de `{ "term", "tf", "score" }`, **un élément par terme analysé de la requête présent dans le passage**, triée par `score` décroissant. La somme des `score` égale `lexical.score` à `1e-4` près. La liste est bornée par le nombre de termes de la requête. **Aucun `Explanation` tantivy brut n'est rendu.**

Précisions :

- `term` est le terme **analysé** par la chaîne de l'index (ADR-0086), pas le texte saisi ; un terme de la requête absent du passage n'a pas d'entrée ;
- `tf` est la fréquence du terme dans le passage ; `score` est la contribution BM25 du terme au score du passage, dans l'unité de `lexical.score` ;
- la borne est structurelle : au plus un élément par terme de la requête, donc au plus le nombre de termes analysés, sans plafond propre ;
- un terme présent avec une contribution nulle garde son entrée, parce que `0` y est une valeur réelle et non une absence (ADR-0119, décision 3) ;
- l'`Explanation` de tantivy reste un outil interne de la branche lexicale : sa forme dépend de la version de tantivy, elle n'est pas bornée, et `docs/05:335` la montrait par un faux JSON (`{ "…": "détail tantivy par terme" }`), ce qu'ADR-0111, décision 8, refuse ;
- la tolérance de `1e-4` est un contrôle de cohérence, pas une garantie d'exactitude au bit près : les contributions sont sommées dans l'ordre rendu.

### Décision 7 — `excluded_by` : feuilles du filtre, chemins JSON complets

**Décision.** `excluded_by` est la liste des **feuilles** du filtre qui échouent pour l'id demandé — le passage quand l'id est une clé, le document quand c'est un `doc_id` —, chacune par son **chemin JSON complet** (par exemple `$.filter.$or[1].fonds`), dans l'**ordre du filtre**. Aucun nœud combinatoire (`$and`, `$or`, `$not`) n'y figure.

Précisions :

- une feuille est une condition de champ, la même unité que la « clause » de `docs/05:232` ; les nœuds combinatoires ne comptent pas, comme dans la limite de 64 clauses (ADR-0029) ;
- le chemin est celui du parseur (`crates/crible-core/src/filter.rs:99-108`), préfixé par `$.filter` sur la route simple et par `$.targets[i].filter` sur la route multi-index, comme les chemins d'`invalid_filter` (`docs/05:370`) ;
- sous `$not`, une feuille qui « échoue » est une feuille dont la condition **n'est pas satisfaite** pour ce document, ce qui exclut le document du complément ; la sémantique des valeurs nulles d'ADR-0035 s'applique, et une condition non satisfaite par un null est bien un échec ;
- l'ordre est celui de la lecture du filtre, donc reproductible ; un filtre qui n'exclut pas le document ne produit aucune entrée, et `excluded_by` est alors absent ;
- `excluded_by` ne nomme ni champ absent du schéma ni opérateur : le filtre a été validé avant l'exécution, et une clause hors grammaire est un `invalid_filter`, pas une explication.

### Décision 8 — Ni journal, ni analytics

**Décision.** L'explication n'entre **ni** dans `debug.sqlite` (la liste du payload d'ADR-0121 est fermée et exclut `explain_ids`, `:121`) **ni** dans les analytics, et **aucun terme ni id n'apparaît dans les logs**. `explain_ids` n'a **aucun effet sur `results`** — ni contenu, ni ordre, ni `k` — et **ne dépend pas de `debug: true`**.

Précisions :

- `explain_ids` et `debug` sont deux champs indépendants : l'un peut être présent sans l'autre, et l'explication ne réutilise pas `DebugCapture` ;
- la règle « aucun effet sur les résultats » est celle d'ADR-0119, décision 10, pour `debug` ; elle vaut ici à l'identique ;
- aucun événement `SearchEvent` ni `OpEvent` ne porte de terme, d'id ni de chemin de filtre (INV-13, ADR-0117) ; un échec de validation d'`explain_ids` produit un `400` compté par son code, sans contenu ;
- les lignes de log d'une requête gardent `request_id`, le code et le statut ; le message d'un `invalid_request` sur `explain_ids` peut nommer `$.explain_ids[i]`, jamais la valeur de l'id ;
- le journal de debug garde sa liste fermée : y ajouter l'explication demanderait un nouvel ADR et une version de format (ADR-0121, décision 2).

### Décision 9 — Confidentialité : aucun iid

**Décision.** Aucun `iid` ne sort, y compris par `explain` : `key`, `doc_id`, `grouped_under` et les chemins de filtre sont les seules références rendues. Le test `inv_01` est **étendu à `explain`**.

Précisions :

- INV-01 n'admet aucune exception (règle d'or 11) ; `binary_rank` repose sur un ordre interne qui départage par iid, mais seul le rang sort, jamais l'identifiant ;
- `terms` ne contient que des termes analysés de la requête et du passage, jamais un id ;
- le test étendu couvre les deux routes et les deux formes d'id, `doc_id` et clé.

### Décision 10 — Pas de `--explain` dans la CLI

**Décision.** Aucun drapeau `--explain` n'est ajouté à `crible search` dans cette tâche, comme `--debug` (ADR-0119, décision 12). La question est renvoyée à `COMMANDE-EVAL`.

Précisions :

- la fiche ne demande aucune surface CLI, et son périmètre fait foi (règle d'or 2) ;
- la CLI suit les champs de l'API ; l'outil de validation du propriétaire est `crible eval` (ADR-0119, décision 12 ; `docs/06:110-130`).

### Décision 11 — Le cas « rang 70 en dense, rang 2 en lexical » est prouvé par une fixture synthétique

**Décision.** Le critère de la fiche est prouvé par une **fixture synthétique construite pour reproduire le cas** — rang dense exact 70 et rang lexical 2 pour le même passage —, **verte en CI**. Aucun test local supplémentaire exclu de la CI n'est ajouté. Le critère de la fiche est mis à jour en conséquence, par une note datée qui renvoie au présent ADR.

Précisions :

- la fixture fixe les scores ou les textes pour que le rang dense exact vaille 70 et le rang lexical 2, sans dépendre du modèle ni de `data/legifacile-sample` ; elle tourne donc dans le job Rust, sans ONNX Runtime ;
- le cas « rang 70 / rang 2 » n'existe pas sur l'échantillon : MES-10 mesure l'article 1240 à rang dense 1 et lexical 5 (variante V3), et le cas d'ADR-0008 vient de l'anecdote ;
- ajouter un sixième test local exclu contredirait la liste fermée des cinq `--skip` de `docs/07:148-150` (ADR-0057, ADR-0062, ADR-0063, ADR-0085, ADR-0109) ;
- les cinq cas de la fiche (`:97`) restent couverts par des tests unitaires ou d'intégration ordinaires, en CI.

### Décision 12 — Le départage textuel reste celui d'ADR-0079

**Décision.** Le présent ADR **note** que le départage du classement est textuel, par la clé `doc_id#ord`, et que l'iid ne départage qu'en interne (ADR-0079, `:50` et `:117`). Il **ne rouvre pas** ADR-0079.

Précisions :

- la note du 2026-10-22 de `docs/suivi.md` (divergence entre la clé textuelle et `PassageKey::Ord`) est réglée par ADR-0079 ; c'est l'orchestrateur qui la marque réglée ;
- `exact_rank`, `lexical.rank` et `fused.rank` suivent tous ce départage ; seul `binary_rank` suit l'ordre interne de la sélection binaire, qui départage par iid, et c'est pourquoi il n'est pas comparable aux autres.

### OpenAPI

`explain` et l'élément de requête multi-index `{ "index", "id" }` deviennent des **composants fermés** (`additionalProperties: false`), conformément à ADR-0111. Le remplacement du marqueur `<!-- openapi: none — … -->` de l'exemple `explain` de `docs/05:329` est fait par `@dev` avec le composant.

## Conséquences

- **Positives** :
  - la forme de `explain` est fermée, sans `null` ni `0`, et chaque branche se lit indépendamment ;
  - l'échelle des rangs est dite, ce qui évite de confondre `exact_rank` (pool entier) et `per_result.dense_rank` (`candidates_per_branch`) ;
  - le coût du scan exact est payé **une fois par requête et par cible**, et le nombre d'ids est borné : les mesures et les analytics restent comparables ;
  - l'explication lexicale est bornée et stable entre versions de tantivy ;
  - `excluded_by` nomme des feuilles, ce qui rend `$or` et `$not` diagnosticables ;
  - le critère « rang 70 / rang 2 » devient une preuve en CI, sans test local supplémentaire.
- **Négatives / coûts** :
  - une requête d'explication porte un scan exact de tout le pool filtré, même sous stratégie binaire : c'est le prix du rang exact, et il n'est pas mesuré dans `took_ms` ;
  - la réponse d'explication n'entre dans aucune mesure d'analytics : son coût réel n'apparaîtra dans aucun tableau de bord ;
  - `dense` ne rend pas de score : la fiche ne le demande pas, mais un diagnostic de score dense demandera un nouvel ADR ;
  - le calcul des feuilles qui échouent évalue le filtre feuille par feuille pour chaque id demandé : au plus 100 ids × 64 clauses.
- **À surveiller** :
  - le temps réel d'une requête d'explication sur un pool de 8,58 M passages — un scan exact partagé plus l'évaluation par id — ; s'il approche `timeout_ms` par défaut (2 000 ms), la borne de 100 ou le protocole d'échéance devra être revu par un nouvel ADR ;
  - l'écart entre la somme des `terms` et `lexical.score` : un dépassement de `1e-4` signalerait un changement de formule BM25 ou de version tantivy ;
  - le coût de `excluded_by` quand un filtre approche la limite de 64 clauses ;
  - la tentation d'ajouter `explain` au journal ou aux analytics : ce serait un ADR, pas un ajout de champ.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Garder `"fused_rank": null` et `"grouped_under": null` dans l'exemple | Contredit `docs/05:410` et ADR-0119, décision 3 : un membre sans valeur est omis, jamais `null` ni `0` |
| Garder `fused_rank` au premier niveau au lieu de `fused: { rank }` | La fiche et la forme retenue groupent les membres par branche ; un membre de fusion au premier niveau se lirait comme un rang de résultat |
| Rendre l'`Explanation` tantivy brut | Forme non bornée, dépendante de la version de tantivy, et `docs/05:335` la montrait par un faux JSON, refusé par ADR-0111, décision 8 |
| Ne pas borner le nombre d'ids | Le scan exact du pool est payé par id ; sans borne, une requête peut porter des milliers de scans sous la même échéance |
| Faire de la borne de 100 une clé de configuration | `docs/07` ne porte aucune clé pour `explain_ids`, et une valeur fixe n'a pas besoin d'une clé (règle de simplicité) |
| Refuser les doublons comme `targets` le fait (ADR-0110) | Un doublon dans `targets` compterait deux fois dans le RRF ; un doublon d'id ne fausse rien, le fusionner garde la réponse courte et prévisible |
| Rendre `exists: false` pour une clé mal formée | Une clé mal formée est une faute de requête, pas une absence de donnée : le code existant la refuse déjà en `invalid_request` (`lexical.rs:155-160`) |
| Calculer l'explication dans `took_ms` | `took_ms` deviendrait dépendant du diagnostic et cesserait d'être comparable entre une requête expliquée et les autres ; les analytics et le journal hériteraient de l'artefact |
| Ajouter une dixième clé à `StageDurations` | La liste est fermée à neuf clés (ADR-0117, décision 5 ; ADR-0119, décision 5) et une étape de diagnostic n'est pas une étape de recherche |
| Calculer `exact_rank` depuis le top des N candidats binaires | Un passage de rang 70 peut être hors des N ; le rang exact demande le score exact de tout le pool filtré |
| Rendre un `dense_rank` de branche au lieu d'`exact_rank` | C'est le rang de `candidates_per_branch` (ADR-0119, décision 1), pas celui du pool : la fiche demande le rang exact (`P7:93`) |
| Rendre `binary_rank` sous stratégie exacte | Il n'y a pas de sélection binaire ; un rang fabriqué ou un `in_binary_candidates: false` seraient faux |
| Nommer dans `excluded_by` le nœud `$or` qui échoue | Un nœud combinatoire ne dit pas quelle feuille a exclu le document ; la fiche demande chaque clause évaluée séparément (`P7:92`) |
| Écrire l'explication dans `debug.sqlite` ou les analytics | La liste du payload d'ADR-0121 est fermée et exclut `explain_ids` ; INV-13 interdit les termes et valeurs dans les analytics |
| Ajouter `--explain` à la CLI | La fiche ne le demande pas ; ADR-0119, décision 12, a refusé `--debug` pour la même raison, et `COMMANDE-EVAL` porte l'outil de validation |
| Prouver « rang 70 / rang 2 » sur `data/legifacile-sample` | L'échantillon ne présente pas ce cas (MES-10 : dense 1, lexical 5) et un test local exclu serait le sixième de la liste fermée de `docs/07:148-150` |
| Rouvrir ADR-0079 pour le départage `PassageKey::Ord` | Le départage textuel est déjà décidé (`:50`, `:117`) ; le rouvrir pour un diagnostic n'apporte rien |

## Complément du 2026-11-01 (relecture, tour 1)

Ce complément répond aux points 1, 2, 4, 5, 6 et 8 de la relecture du tour 1 de la tâche `EXPLIQUER-DOCUMENTS`. Il **ne réécrit aucune des douze décisions** ci-dessus ; il en fixe l'interprétation là où la relecture a montré une ambiguïté. Les règles sont retenues par l'orchestrateur, par délégation du propriétaire pour la suite de P7, et portent le **même statut d'acceptation par anticipation** que l'ADR.

### Lecture d'un id : clé de passage ou `doc_id` (point 2)

Un id demandé est une **clé de passage** quand il se termine par `#` suivi d'un entier décimal valide comme `ord` — même analyse que `PassageKey::from_str`, qui coupe au **dernier** `#` (`crates/crible-core/src/ids.rs:99-114`). **Tout autre id est un `doc_id`**, y compris quand il contient `#` : `A#x` est un `doc_id` et rend une entrée, jamais un `400`. Aucune interdiction de `#` dans un `doc_id` n'est ajoutée, ni au code (`crates/crible-core/src/ids.rs:56-58` admet ce caractère) ni à la conception (`docs/02:47` ne l'interdit pas).

Précisions :

- un id **vide** reste un `400 invalid_request` sur `$.explain_ids[i]` (décision 3), de même qu'un id dont la partie document d'une clé est vide (`#3`) ou dépasse la longueur maximale de `DocId` ;
- « clé mal formée » au sens de la décision 3 se réduit donc à ces cas : un suffixe non numérique, ou absent, fait de l'id un `doc_id` et non une clé fautive ;
- **conséquence assumée et écrite** : un document dont le `doc_id` se termine lui-même par `#<entier>` ne peut être expliqué que **passage par passage** (`A#3#0`), parce que `A#3` se lit comme la clé du passage 3 du document `A` ; la documentation le dit (`docs/05-api.md`, « Recherche : réponse ») ;
- le membre `doc_id` d'une entrée de clé reste la partie document de la clé.

### Périmètre des membres : le passage, jamais le document ni un frère (points 1 et 5)

Pour une **clé**, `in_pool`, `excluded_by`, `dense`, `lexical`, `fused` et `grouped_under` décrivent **le passage lui-même**, jamais son document ni un passage frère. Pour un **`doc_id`**, `in_pool` et `excluded_by` portent sur le document (décisions 1 et 7), et chaque branche décrit le passage le mieux classé de ce document dans cette branche (décision 1).

Pour un `doc_id` **hors du pool**, `excluded_by` est l'**union**, dans l'ordre du filtre, des feuilles qui échouent pour **au moins un** de ses passages vivants. Elle n'est donc jamais vide quand le document a des passages vivants et que `in_pool` est faux : le pool vaut vivants ∧ filtre, donc chaque passage vivant hors du pool échoue à au moins une feuille. Un document **sans** passage vivant n'a aucune feuille à nommer : le membre est alors omis, comme tout membre sans valeur (décision 1 ; ADR-0119, décision 3).

### Coût : ce qui entre dans `took_ms` (point 4)

Le **seul** travail lié à `explain_ids` compté dans `took_ms` est la **capture des rangs avant regroupement** dans le pipeline, nécessaire à `fused.rank` et à `grouped_under` : elle a lieu pendant la recherche, donc avant l'arrêt du total, et seulement quand `explain_ids` est présent. **Tout le reste** — calcul des passages absorbés, lectures des empreintes de contenu sous `dedup`, scans exacts, rangs binaires et lexicaux — se fait **après la lecture du total**, comme le dit la décision 4.

### `grouped_under` sous `dedup` (point 8)

Sous `dedup: "content"`, `grouped_under` nomme le passage **rendu** de **même empreinte de contenu** ; sous `group_by: "document"`, ou sous une déduplication par document, il nomme le passage **rendu du même document**. Quand les deux s'enchaînent (ADR-0075), on cherche **d'abord** le passage rendu de même empreinte, **puis**, à défaut, le passage rendu du même document.

### À surveiller (point 6)

Sous `explain_ids`, `lexical.rank` demande de scorer **tout le pool marqué par la requête**, et non les seuls candidats retenus. Son coût — mémoire et temps — à 8,58 M passages **n'est pas mesuré**. La mise en œuvre doit préférer un **comptage des scores supérieurs**, départage textuel compris, à un classement complet du pool ; une mesure devra précéder toute conclusion sur la tenue de la borne de 100 sous l'échéance.

## Références

- Fiche : `docs/phases/P7-observabilite.md:86-101`.
- Conception : `docs/05-api.md` (« Recherche : requête », « Recherche : réponse », « Recherche multi-index »), `docs/06-observabilite.md:96-108` (« Debug dans la réponse et explain »), `docs/03-recherche.md` (étapes et rangs), `docs/07-stack-et-deploiement.md:140-155` (CI et tests exclus), `docs/08-invariants-et-tests.md:9` (INV-01) et `:21` (INV-13).
- ADR : ADR-0001, ADR-0008, ADR-0041, ADR-0057, ADR-0062, ADR-0063, ADR-0072, ADR-0075, ADR-0079, ADR-0085, ADR-0086, ADR-0102, ADR-0109, ADR-0110, ADR-0111, ADR-0117, ADR-0119, ADR-0120, ADR-0121.
- Mesure : `docs/mesures/MES-10-qualite-fusion.md:124-159` (cas « article 1240 »).
- Code : `crates/crible-search/src/lexical.rs:146-170`, `crates/crible-search/src/dense/binary.rs:85-95`, `crates/crible-core/src/filter.rs:99-108`.
