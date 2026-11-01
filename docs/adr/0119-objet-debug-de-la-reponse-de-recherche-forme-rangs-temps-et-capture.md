# ADR-0119 — Objet `debug` de la réponse de recherche : forme, rangs, temps et capture

- **Statut** : Accepté (accepté par anticipation par le propriétaire le 2026-10-31, confirmé par le propriétaire le 2026-11-01)
- **Date** : 2026-10-31
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, tâche REPONSE-DEBUG (phase P7), sur les options recommandées par `@architecte` et retenues par l'orchestrateur, en application de la délégation du propriétaire du 2026-10-31 pour la série de P7
- **Remplace** : — (ne remplace aucune décision : ouvre `debug` conformément à ADR-0109, précise `docs/03` et `docs/05`, et prépare la capture de JOURNAL-DE-DEBUG, sans modifier le corps d'aucun ADR accepté)

## Contexte

### La fiche et ce qu'elle demande

La fiche `REPONSE-DEBUG` (`docs/phases/P7-observabilite.md:74-81`) a pour objectif « `debug: true` dans la réponse ». Elle dépend de ROUTE-RECHERCHE et demande quatre choses : le **plan**, les **temps par étape** avec les clés de `docs/03-recherche.md`, pour chaque résultat le **rang et le score dans chaque branche** ainsi que le **rang fusionné**, et, en multi-index, le **rang dans son index** (`target_rank`).

Ses deux critères d'acceptation sont : « la somme des étapes reste cohérente avec le total (écart inférieur à 10 %) » (`:80`), et « les rangs affichés correspondent à ceux d'une exécution branche par branche » (`:81`). Elle ne cite aucun invariant.

### Ce que la conception pose

`docs/05-api.md:270` déclare le champ `debug` de la requête, avec pour rôle « Ajoute plan, temps et rangs ; journalise si le journal est activé ». `:303-317` donne la forme de l'objet et un exemple, et `:317` définit `dense_strategy` et `rescore_candidates` (ADR-0074). `:299` définit `took_ms` de la prise du jeton à la fin du calcul (ADR-0109), et `:416` rappelle que l'échéance couvre tout le pipeline.

`docs/03-recherche.md:5-15` fixe les neuf clés de temps — `validate`, `encode`, `filter`, `dense`, `lexical`, `fusion`, `group`, `fetch`, `total` — et dit que les étapes ne s'exécutent que si le mode les demande (`:17`). `:100-101` fixe la sortie des branches et le rang du RRF à partir de 1, et `:107` dit que « pour le debug, on conserve le rang et le score de chaque résultat dans chaque branche », sans dire à quel moment le rang est pris. `:132` annonce le rang dans l'index pour le debug en multi-index, sans en donner la forme.

`docs/06-observabilite.md:74-84` décrit le journal de debug : une requête avec `debug: true` y est enregistrée si le journal est activé, avec le plan, les temps et « le top 20 (clé et score de chaque branche) ». `:88-98` décrit `debug` et `explain_ids` dans la réponse.

Les ADR liés fixent le cadre. ADR-0043 garde le payload d'un résultat dont le document a été supprimé : `debug` ne change pas ce payload. ADR-0074 nomme la stratégie dense et le nombre effectif de candidats rescorés. ADR-0075 fixe l'ordre `dedup` puis `group_by`, le sur-échantillonnage et la forme plate de la réponse. ADR-0079 rend le tri total par score décroissant puis clé croissante, ce qui donne un rang reproductible. ADR-0109 a refusé `debug` en P6 et renvoyé son ouverture à P7, et fixe `took_ms`. ADR-0110 fixe la recherche multi-index, le RRF `1/(60 + rang)` entre index et `target_rank`. ADR-0111 impose `additionalProperties: false` là où le serveur refuse les clés inconnues et associe chaque exemple de `docs/05` à un composant par un marqueur `<!-- openapi: … -->`. ADR-0117, décision 5, définit `StageDurations`, les neuf durées d'une recherche, dont une étape non exécutée vaut `None` pour ne produire aucun échantillon d'histogramme.

### Ce que le code montre déjà

Les branches dense et lexicale tournent **en parallèle** : `crates/crible-search/src/pipeline.rs:361-380` les lance par `rayon::join` et mesure chacune séparément, puis les fusionne. La sortie d'une branche est la liste des `candidates_per_branch` meilleurs, calculée par `crates/crible-search/src/group.rs:178-184`, et le rang dense est lu **avant** le regroupement (`crates/crible-search/src/dense/planner.rs:253-265`). Le regroupement lui-même est `crates/crible-search/src/group.rs:206-232` : `dedup` d'abord, puis `group_by`. Les durées d'étape existent déjà pour les analytics (`crates/crible-server/src/analytics.rs:151-194`, `StageDurations`) et sont remplies par `crates/crible-server/src/search.rs:604-618` (`measure`). La validation est chronométrée **avant** la prise du jeton (`crates/crible-server/src/search.rs:235-243`), donc hors de `took_ms`.

### L'écart que cet ADR tranche

Le critère « somme des étapes cohérente avec le total, écart < 10 % » (`docs/phases/P7:80`) ne peut pas être lu comme une somme arithmétique des neuf clés en mode `hybride` : les branches dense et lexicale tournent en parallèle (`pipeline.rs:361-380`), alors que l'exemple de `docs/05:308-311` les additionne comme si elles étaient séquentielles. Dans cet exemple, la somme des neuf clés vaut 41,3 ms pour un total de 41,7 ms, soit 1 % d'écart, mais le **chemin critique** vaut 35,1 ms — 23,4 + 0,6 + max(9,8 ; 6,1) + 0,1 + 0,0 + 1,2 —, soit 16 % d'écart. Le critère passerait ou échouerait selon l'addition choisie, ce qui n'est pas un critère.

Le second point ouvert est la clause « journalise si le journal est activé » de `docs/05:270` : le journal est l'objet de JOURNAL-DE-DEBUG (`docs/phases/P7:55-69`), qui suit cette tâche. `debug` doit donc préparer la capture sans l'écrire.

## Décision

### Décision 1 — rang dans une branche : après le filtre, avant `dedup` et `group_by`

**Décision.** Le rang d'un résultat dans une branche, rendu par `dense_rank` ou `lexical_rank`, est son rang dans la **liste de sortie de la branche** : `candidates_per_branch` entrées, **après** le filtre et **avant** `dedup` et `group_by`, à partir de 1. L'égalité de score est départagée par la clé, comme le tri total d'ADR-0079.

Précisions :

- la liste de sortie est celle que la branche produit et que la fusion consomme : le top des `candidates_per_branch` meilleurs scores (`docs/03:53`, `docs/03:100`), sur le pool filtré ;
- le rang est donc **antérieur** au regroupement, qui peut supprimer une entrée par `dedup` ou absorber un passage dans son document. Un résultat regroupé garde le rang de branche du passage qui le représente ;
- l'égalité de score ne dépend pas de l'ordre d'itération : le tri de la branche est score décroissant puis clé croissante (ADR-0079, `:50`), et le rang suit ce tri ;
- le score de branche, `dense_score` ou `lexical_score`, est celui de la branche, sans normalisation : cosinus pour le dense, BM25 pour le lexical. Sous `relative_score`, la normalisation min-max d'ADR-0090 ne remplace pas ces scores.

### Décision 2 — `fused_rank` : rang dans la fusion, avant regroupement

**Décision.** `fused_rank` est le rang du résultat dans la **sortie de la fusion**, avant `dedup` et `group_by`, à partir de 1. Sous `group_by: "document"`, toutes les entrées d'un document portent le rang de son meilleur passage — la même règle qu'ADR-0110 pour le score d'un document regroupé.

Précisions :

- en mode `dense` ou `lexical`, il n'y a pas de fusion : `fused_rank` est le rang de la branche unique, et il reste présent ;
- en mode `hybride`, la fusion est celle d'ADR-0008 et ADR-0090, appliquée à la liste des candidats avant regroupement ;
- le rang fusionné n'est pas le rang de la réponse : la troncature à `k` et le regroupement viennent après, et un résultat de rang élevé peut ne pas être rendu ;
- sous `group_by`, un passage ajouté par `passages_per_document` porte le `fused_rank` du meilleur passage de son document, même s'il avait un rang moins bon dans la fusion.

### Décision 3 — membres absents : on omet, jamais `null`, jamais `0`

**Décision.** Un membre de `per_result` qui n'a pas de valeur est **omis**. On omet `dense_rank` et `dense_score` en mode `lexical`, `lexical_rank` et `lexical_score` en mode `dense`, et les membres d'une branche qui n'a pas trouvé le résultat. Un membre absent n'est jamais rendu `null`, ni `0`.

Précisions :

- en `hybride`, un résultat peut venir d'une seule branche : il porte alors les membres de cette branche seulement. En `lexical`, `dense_rank` et `dense_score` sont absents même si la branche dense a tourné pour une autre cible ;
- `0` est une valeur légitime d'un score de cosinus ou de BM25, et un rang ne vaut jamais 0 : écrire `0` pour dire « absent » rendrait un score nul indiscernable d'une absence ;
- `null` obligerait chaque client à distinguer trois états — absent, nul, trouvé — là où deux suffisent. Un objet fermé (ADR-0111) dit la même chose sans ambiguïté ;
- `key` est toujours présent : c'est la clé externe `doc_id#ord`, jamais un `iid` (INV-01, `docs/02:53`).

### Décision 4 — `plan` : liste fermée, `alpha` seulement pour `relative_score`

**Décision.** `plan` est un objet **fermé** qui porte les six membres de `docs/05:308-309` : `mode`, `dense_strategy`, `rescore_candidates`, `candidates_per_branch`, `fusion` et `group_by`. Il porte en plus `alpha` **seulement** quand `fusion` vaut `relative_score`. `dense_strategy` et `rescore_candidates` sont **absents** si aucune branche dense n'a tourné.

Précisions :

- `mode` est le mode effectif de la requête, après application du défaut (`docs/05:261`) ;
- `dense_strategy` vaut `"exact"` ou `"binary"` (ADR-0074). `rescore_candidates` est le nombre effectif de candidats rescorés, `max(N, candidates_per_branch)`, quand la branche dense tourne (`docs/05:317`, ADR-0074) ;
- `candidates_per_branch` est le nombre **effectif** de résultats demandés par branche, donc sur-échantillonné quand la requête regroupe : `clamp(k × 20, 200, 1000)` ou `max(k, 100)` (`docs/03:115-117`, ADR-0075) ;
- `fusion` est le nom de la méthode, `"rrf"` ou `"relative_score"`. `alpha` n'apparaît que dans le second cas, avec la valeur effectivement appliquée ;
- `group_by` vaut `"document"` ou `null`, comme dans `docs/05:309`. La règle « jamais `null` » de la décision 3 porte sur les membres de `per_result`, pas sur ce champ de plan, dont `null` est la valeur documentée de « pas de regroupement » ;
- l'objet est fermé (`additionalProperties: false`, ADR-0111) : un membre nouveau demanderait un ADR, pas une revue de code.

### Décision 5 — temps : les neuf clés, en ms, `f64` à trois décimales, `0.0` si l'étape n'a pas tourné

**Décision.** `timings_ms` porte **toujours** les neuf clés de `docs/03:5-15`, dans cet ordre : `validate`, `encode`, `filter`, `dense`, `lexical`, `fusion`, `group`, `fetch`, `total`. Chaque valeur est une durée en millisecondes, `f64`, arrondie à trois décimales. Une étape que le mode n'a pas exécutée vaut `0.0`.

Précisions :

- `0.0` est une valeur de rendu, pas une mesure : la mesure d'une étape non exécutée n'existe pas, et `StageDurations` la garde à `None` (ADR-0117, décision 2). Le rendu de `debug` convertit `None` en `0.0`, et les analytics ne reçoivent aucun échantillon pour cette étape ;
- `total` est la valeur de `took_ms`, arrondie de la même façon. `took_ms` garde sa propre forme et n'est pas modifié par `debug` (décision 10) ;
- trois décimales suffisent à distinguer les étapes courtes — un filtre à 0,6 ms, un regroupement à 0,0 ms — sans donner à un client l'illusion d'une précision à la microseconde. Le chronomètre du code est une horloge murale, pas un compteur de cycles ;
- les neuf clés sont toujours présentes, même si plusieurs d'entre elles valent `0.0`, pour qu'un client lise un objet de forme constante.

### Décision 6 — la somme de cohérence est le chemin critique, pas la somme des neuf clés

**Décision.** La somme de cohérence est celle du **chemin critique** :

```
encode + filter + max(dense, lexical) + fusion + group + fetch
```

`validate` est **exclu** : il est chronométré avant la prise du jeton (`crates/crible-server/src/search.rs:235-243`), donc hors de `took_ms`. Le `total` est l'instant de `took_ms`, pris à la prise du jeton (ADR-0109). Le test d'écart inférieur à 10 % porte sur cette somme, en mode `dense` ou `lexical` — une seule branche, le `max` se réduit à elle —, et en `hybride` sur la somme du chemin critique.

Précisions :

- `fusion` vaut `0.0` en mode `dense` et `lexical`, puisque la fusion n'y tourne pas ; le `max` d'une branche non exécutée est l'autre branche, et il vaut `0.0` seulement si aucune branche n'a tourné ;
- `group` couvre `dedup` et `group_by` (`docs/03:13`, `docs/03:112-114`) ; `fetch` couvre la lecture des payloads (`docs/03:14`) ;
- l'exemple de `docs/05:308-311` est **corrigé** par cet ADR : `encode` passe de 23,4 à 30,0 ms, et la somme du chemin critique vaut alors 30,0 + 0,6 + max(9,8 ; 6,1) + 0,1 + 0,0 + 1,2 = 41,7 ms, égal au total. La somme arithmétique des neuf clés vaut 47,8 ms et n'est pas la valeur de comparaison, parce que les deux branches ont tourné en parallèle ;
- en multi-index, la règle s'applique **par cible**, sur `targets[i].timings_ms`, avec le `total` de la cible. Le `timings_ms` global n'est pas un chemin critique : il additionne les étapes des cibles séquentielles (décision 9) ;
- le seuil de 10 % laisse la place au bruit d'horloge et aux arrondis, pas à une étape oubliée. Un dépassement régulier dit qu'une étape n'est pas mesurée ou que le total n'est pas pris au bon instant, pas que le seuil est trop strict.

### Décision 7 — une seule mesure pour `debug` et pour les analytics

**Décision.** Les durées d'étape de `StageDurations` (ADR-0117, décision 5), remplies par `measure` (`crates/crible-server/src/search.rs:604-618`), alimentent **à la fois** l'objet `debug` et l'événement d'analytics de la recherche. Le `total` est arrêté **une seule fois** et sert aux deux.

Précisions :

- `debug` ne crée aucun second chronomètre : deux mesures de la même étape dériveraient l'une de l'autre, et la réponse et les analytics pourraient se contredire ;
- `measure` rend `None` pour une étape que le mode n'a pas exécutée ; le rendu de `debug` en fait `0.0` (décision 5), et l'événement d'analytics garde `None` pour ne produire aucun échantillon (ADR-0117, décision 2). La conversion est faite au rendu, pas dans la mesure ;
- `total` est arrêté par `finish_at(started)` (`crates/crible-server/src/search.rs:431-432`) et vaut exactement `took_ms` : la même valeur sert au membre `total` et à la colonne `latency_total` ;
- une réponse `200` avec `debug: true` produit un événement d'analytics **comme une réponse sans `debug`** : `debug` n'ajoute ni ne retire d'événement, et ne change aucune durée.

### Décision 8 — capture `DebugCapture { top_n }`, construite seulement si `debug` est demandé

**Décision.** `crible-search` reçoit une structure de capture `DebugCapture { top_n }`, construite **seulement** quand `debug` est demandé. Quand `debug` est faux, aucune allocation, aucun vecteur, aucun champ de capture n'est créé sur le chemin de la recherche. `top_n` est un paramètre explicite du constructeur, utilisé plus tard par JOURNAL-DE-DEBUG avec la valeur 20 par branche (`docs/06:82`).

Précisions :

- la capture enregistre, par branche, au plus `top_n` entrées `(key, rank, score)`, où `key` est la clé externe `doc_id#ord`. Elle **n'expose jamais un `iid`** (INV-01, `docs/02:53`) ;
- `top_n` n'est pas une constante cachée : l'appelant le donne, et le journal choisira 20 ;
- la capture n'est **pas** la source des rangs de `per_result` : les rangs de branche viennent des listes de candidats, et `per_result` peut citer un rang supérieur à `top_n`. La capture sert au journal, dont `docs/06:82` ne garde que le top 20 ;
- le coût est borné par `top_n` et par le nombre de branches, pas par la taille du pool. Une recherche sans `debug` ne paie rien, ce qui est la condition pour que `debug` reste une option de diagnostic ;
- la capture est créée dès cette tâche pour que JOURNAL-DE-DEBUG n'ait pas à modifier le pipeline : elle sera consommée par l'écrivain du journal, pas par la réponse.

### Décision 9 — multi-index : `targets[]`, `target_rank`, et un `timings_ms` global

**Décision.** En multi-index, l'objet `debug` porte :

- `targets: [{ index, plan, timings_ms }]`, une entrée par cible, dans l'ordre de `targets` de la requête ;
- `per_result: [{ index, key, …, target_rank }]`, une entrée par résultat de la réponse, dans l'ordre de `results` ;
- `timings_ms` global, dont `total` vaut `took_ms`.

`target_rank` est le rang de l'unité dans la **liste finale de son index**, à partir de 1 : en `lexical` et en `hybride`, c'est exactement le rang du `1 / (60 + rang)` de la fusion entre index (ADR-0110) ; en `dense`, c'est le rang dans la liste triée par cosinus de l'index. Sous `group_by`, toutes les entrées d'un document portent le rang de son meilleur passage, comme le score d'ADR-0110.

Précisions :

- `targets[i].plan` et `targets[i].timings_ms` ont la forme des décisions 4 et 5. Les deux étapes `validate` et `encode` y valent `0.0`, parce que la validation et l'encodage ont lieu **une fois** pour la requête : ils appartiennent au `timings_ms` global ;
- le `total` d'une cible est la durée de son travail, du filtre à la lecture de ses payloads. La règle de cohérence de la décision 6 s'applique à ce `total` ;
- le `timings_ms` global porte les neuf clés : `validate` et `encode` mesurés une fois, `filter`, `dense`, `lexical`, `fusion`, `group` et `fetch` **sommes** sur les cibles séquentielles, la `fusion` globale incluant la fusion entre index. Son `total` vaut `took_ms`. Il n'est pas un chemin critique et n'est pas soumis au test d'écart de 10 % ;
- `per_result` porte les mêmes membres de branche que la route simple, plus `index` et `target_rank`. Le rang dans la fusion **entre index** n'a pas de membre : c'est la position de l'entrée dans `results`, que la réponse donne déjà ;
- `index` est le nom de l'index de la cible, et `key` la clé externe. Aucun `iid` n'apparaît (INV-01).

### Décision 10 — `debug` n'a aucun effet sur la réponse de recherche

**Décision.** `debug` ne change ni les résultats, ni leur ordre, ni `k`, ni l'échéance, ni `took_ms`. Il ajoute seulement un membre à une réponse `200`. Un `504` reste un `504` sans `debug`. `explain_ids` reste refusé jusqu'à EXPLIQUER-DOCUMENTS (ADR-0109).

Précisions :

- la capture de la décision 8 est construite après les décisions de recherche, ou à côté, sans modifier l'ordre des candidats ni le tri ;
- une réponse d'erreur, `400`, `404`, `413`, `422`, `503` ou `504`, ne porte **jamais** de membre `debug` : il n'y a pas de plan à décrire pour une requête qui n'a pas tourné ;
- `explain_ids` garde son refus de P6, avec un message qui dit qu'il n'est pas encore pris en charge (`docs/05:278`) ; le bloc `explain` de `docs/05:319-329` garde son marqueur `none` jusqu'à ce que cette tâche-là l'ouvre ;
- la preuve « API = CLI » d'ADR-0109 porte sur les résultats, qui sont inchangés par `debug` : elle n'a pas besoin d'un mode `debug` dans la CLI.

### Décision 11 — OpenAPI : composants fermés et marqueur de l'exemple `debug`

**Décision.** `debug` est ajouté aux schémas `SearchRequest` et `SearchMultiRequest`, et devient un membre facultatif de `SearchResponse` et `SearchMultiResponse`. Les composants sont **fermés** (`additionalProperties: false`, ADR-0111) : `SearchDebug`, `DebugPlan`, `DebugTimings`, `DebugResult` pour la route simple ; `SearchMultiDebug`, `DebugTarget`, `DebugMultiResult` pour la route multi-index.

Précisions :

- le marqueur `<!-- openapi: none — le bloc debug arrive avec REPONSE-DEBUG (P7) ; aucun composant ne le décrit en P6. -->` de `docs/05:305` est **remplacé** par `<!-- openapi: SearchDebug -->` ; l'exemple de `docs/05:306-315` devient un objet `debug` complet et valide ;
- le marqueur de l'exemple `explain` (`docs/05:321`) **reste** `none` jusqu'à EXPLIQUER-DOCUMENTS ;
- l'exemple multi-index porte `<!-- openapi: SearchMultiDebug -->` et ne montre qu'une cible, pour rester valide contre un composant fermé ; la prose dit que `targets` en porte une par cible ;
- les noms de composants sont ceux que le code expose dans `components.schemas` ; un nom différent dans le code ferait échouer le test d'association d'ADR-0111, et c'est le code qui fait foi pour le nom exact ;
- `debug` est un booléen facultatif dans les deux schémas de requête, et son absence vaut `false` (`docs/05:270`).

### Décision 12 — pas de drapeau `crible search --debug` dans cette tâche

**Décision.** La CLI ne reçoit pas de drapeau `--debug` dans REPONSE-DEBUG. La preuve API = CLI d'ADR-0109 porte sur les résultats, et `debug` ne les change pas (décision 10). Le besoin éventuel d'un mode debug appartient à COMMANDE-EVAL (`docs/phases/P7:109-120`).

Précisions :

- ajouter un drapeau serait hors de la fiche, qui demande « `debug: true` dans la réponse » et ne cite pas la CLI ;
- un drapeau `--debug` sans emploi dans `crible eval` serait une option de plus à documenter et à tester, pour un objet que la CLI n'a pas à afficher ;
- si COMMANDE-EVAL veut lire le plan et les temps pour produire son rapport, c'est cette tâche qui posera le drapeau et son format, avec son propre ADR si la forme de sortie engage la CLI.

### Décision 13 — la clause « journalise si le journal est activé » est remplie par JOURNAL-DE-DEBUG

**Décision.** La clause de `docs/05:270` est remplie par JOURNAL-DE-DEBUG (`docs/phases/P7:55-69`), qui suit cette tâche. REPONSE-DEBUG livre la capture de la décision 8, pas l'écriture du journal.

Précisions :

- `docs/05:270` est corrigé pour dire que `debug` alimente le journal **quand il est activé**, et que l'écriture appartient à JOURNAL-DE-DEBUG ; la formulation ne promet plus une journalisation que cette tâche ne livre pas ;
- `DebugCapture` est construite maintenant pour que le journal n'ait pas à modifier le pipeline : il consommera les `top_n` entrées de chaque branche (décision 8) ;
- le journal enregistre aussi le plan et les temps (`docs/06:81`), qui sont déjà dans l'objet `debug` ; il n'a pas à les recalculer ;
- `debug.sqlite`, `GET /v1/debug/queries`, la purge selon `ttl_days` et la preuve d'INV-17 du journal restent entièrement dans JOURNAL-DE-DEBUG.

### Décision 14 — aucune dépendance nouvelle

**Décision.** Cet ADR n'ajoute aucune dépendance à `docs/07`. Les types de `debug` sont sérialisés avec `serde` et `serde_json`, déjà présents (ADR-0027), et la capture est une structure de `crible-search` sans crate externe.

Précisions :

- aucun crate d'horloge n'est ajouté : les durées viennent des `Instant` déjà utilisés par `pipeline.rs` et `search.rs` ;
- aucun crate de schéma n'est ajouté pour OpenAPI : `utoipa` est déjà la dépendance d'ADR-0111 ;
- `DebugCapture` n'utilise que `Vec`, `String` et les types existants du moteur.

## Conséquences

- **Positives** :
  - `debug` a une forme **fermée** et testable, sur la route simple comme en multi-index, et un client n'a pas à deviner si un membre absent vaut `null` ou `0` ;
  - les rangs sont **reproductibles** : la règle dit où le rang est pris, et le tri total d'ADR-0079 départage les égalités par la clé ;
  - le critère de cohérence des temps devient une **phrase** — le chemin critique — au lieu d'une addition dont le résultat dépend du mode ; l'exemple de `docs/05` cesse d'être faux ;
  - une seule mesure alimente la réponse et les analytics, donc `debug` ne peut pas contredire `GET /v1/stats` sur la même requête ;
  - `debug` reste **gratuit** quand il est faux : aucune allocation, aucun vecteur de capture, aucun second chronomètre ;
  - la capture prépare JOURNAL-DE-DEBUG sans toucher au pipeline, et ne transporte que des clés externes, jamais un `iid` ;
  - la spécification OpenAPI décrit `debug` avec des composants fermés, et l'exemple de `docs/05` est validé par le test d'ADR-0111.
- **Négatives / coûts** :
  - la réponse grossit : un `per_result` par résultat, plus `targets[]` en multi-index, et jusqu'à neuf temps par cible ;
  - `per_result` ne décrit que les **résultats rendus** : il ne donne pas le classement complet des branches, qui demanderait une autre forme et un autre coût ;
  - `debug` n'est pas disponible sur un `504` : le diagnostic d'une requête trop lente reste hors de la réponse, et c'est le journal qui devra le porter ;
  - la CLI n'a pas de drapeau `--debug`, donc un utilisateur de `crible search` ne voit pas le plan sans passer par l'API ;
  - `plan.group_by` garde `null` alors que les membres de `per_result` s'omettent : deux conventions dans un même objet, qu'il faut documenter ;
  - les temps sont arrondis à trois décimales et ne sont pas une mesure de précision : un écart de 0,001 ms entre deux étapes n'a pas de sens.
- **À surveiller** :
  - le test d'écart de 10 % : s'il échoue en `hybride`, c'est la mesure d'une étape ou le point de départ du total qui est faux, pas le seuil. Un échec en `dense` ou `lexical` dit la même chose d'une seule branche ;
  - la taille d'un `per_result` sur un `k` élevé : à `k = 1000`, la réponse porte 1 000 entrées de diagnostic, et `debug` n'est pas fait pour cela ; si un client l'utilise ainsi, c'est un besoin de journal, pas de réponse ;
  - la cohérence entre `target_rank` et l'ordre de `results` en multi-index : les deux doivent être calculés sur la même liste finale de l'index, sinon le RRF et le diagnostic divergent ;
  - le premier usage de `DebugCapture` par JOURNAL-DE-DEBUG : si `top_n = 20` ne suffit pas à lire une requête, c'est la forme du journal qu'il faut revoir, pas `debug` ;
  - la mise à jour de `docs/05` décale ses lignes : les numéros cités ici sont ceux d'avant application, et le tableau « Documents mis à jour » fait foi.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Sommer les neuf clés et comparer au total, comme le suggère l'exemple de `docs/05` | Les branches dense et lexicale tournent en parallèle (`pipeline.rs:361-380`) : la somme compte deux fois un temps qui s'est écoulé une fois, et l'écart dépend du mode. Le critère serait vrai par accident sur un exemple et faux sur un autre. |
| Prendre `total` comme la somme des étapes | `took_ms` est mesuré de la prise du jeton à la fin du calcul (ADR-0109) ; le remplacer par une somme donnerait un second total, qui contredirait la réponse et la CLI. |
| Inclure `validate` dans la somme de cohérence | La validation est chronométrée avant la prise du jeton (`search.rs:235-243`), donc hors de `took_ms` : l'inclure ferait dépasser le total de sa durée. |
| Rendre `null` pour un membre de branche absent | Un client devrait distinguer `null`, `0` et « trouvé » ; pour un score, `0` est une valeur légitime, donc `null` et `0` ne suffisent pas à dire l'absence. L'omission dans un objet fermé le dit sans ambiguïté. |
| Rendre `0` pour un rang ou un score absent | Un rang ne vaut jamais 0, et un score nul est un score réel. `0` comme sentinelle rendrait un résultat de score 0 indiscernable d'un résultat absent de la branche. |
| Prendre le rang de branche après `dedup` et `group_by` | Le rang de branche doit décrire la liste que la fusion a consommée ; après regroupement, les entrées supprimées n'ont plus de rang et les entrées ajoutées n'en ont jamais eu. Le critère « rangs d'une exécution branche par branche » ne serait plus vérifiable. |
| Prendre le rang de branche avant le filtre | Il n'existe pas : la branche ne parcourt que le pool filtré. Un rang sur les vivants non filtrés décrirait une autre recherche. |
| Un `fused_rank` recalculé après regroupement | Le regroupement change la liste et le nombre d'unités ; le rang fusionné doit rester celui de la fusion, pour que le diagnostic explique le classement et non la troncature. |
| Un `alpha` toujours présent dans `plan` | En `rrf`, aucun α n'est appliqué : rendre `0.5` ou `0.0` inventerait un réglage qui n'existe pas. Le membre n'apparaît que pour `relative_score`. |
| Toujours calculer `debug` et le supprimer au rendu | Le coût serait payé par toutes les recherches, y compris celles qui ne demandent rien ; `debug` est une option de diagnostic, et la capture ne doit exister que si elle est demandée. |
| Une capture qui garde tous les candidats de chaque branche | Le coût croîtrait avec `candidates_per_branch` et la taille du pool, pour un journal qui ne garde que le top 20 (`docs/06:82`) ; `top_n` borne la mémoire et rend le coût explicite. |
| Une capture qui garde l'`iid` pour rejoindre les résultats | L'`iid` ne sort jamais du moteur (INV-01, `docs/02:53`) ; la clé `doc_id#ord` suffit à joindre un résultat et reste une donnée externe. |
| Un seul `plan` pour toutes les cibles d'une recherche multi-index | Les cibles peuvent avoir des stratégies, des seuils et des sur-échantillonnages différents : un plan global serait faux pour toutes sauf une. Le plan est par cible. |
| Un `target_rank` pris après la fusion entre index | Ce rang est celui de la réponse, pas celui de l'index ; le RRF `1/(60 + rang)` d'ADR-0110 utilise le rang dans la liste finale de l'index, et c'est celui-là que le diagnostic doit montrer. |
| Ouvrir `explain_ids` dans la même tâche | La fiche REPONSE-DEBUG ne le demande pas, et EXPLIQUER-DOCUMENTS a son propre périmètre, son propre coût — un scan exact du pool — et sa propre fiche (`docs/phases/P7:83-95`). |
| Ajouter un drapeau `crible search --debug` | Hors de la fiche, et la CLI n'a pas d'emploi du plan dans cette tâche ; COMMANDE-EVAL le demandera s'il lui sert. |
| Journaliser depuis REPONSE-DEBUG | Le journal a son fichier, sa file, sa purge et sa preuve d'INV-17 dans JOURNAL-DE-DEBUG (`docs/phases/P7:55-69`, ADR-0118 décisions 7 et 8) ; l'écrire ici doublerait le travail et séparerait la preuve de son objet. |
| Une dépendance pour mesurer ou sérialiser `debug` | `Instant`, `serde` et `utoipa` sont déjà là ; une crate de plus pour un objet de diagnostic serait une dépendance sans besoin. |

## Références

- `docs/phases/P7-observabilite.md:74-81` — fiche `REPONSE-DEBUG` : objectif (`:75`), dépendance (`:76`), lectures (`:77`), livrables (`:78`), critère de cohérence des temps (`:80`), critère des rangs (`:81`).
- `docs/05-api.md:270` (`debug` dans la requête), `:278` (`debug` et `explain_ids` refusés en P6), `:299` (`took_ms`), `:303-317` (objet `debug`, exemple, `dense_strategy` et `rescore_candidates`), `:319-329` (`explain`), `:348` (`debug` refusé en P6 sur la route multi-index), `:376` (`target_rank` et debug multi-index), `:416` (échéance).
- `docs/03-recherche.md:5-15` (neuf clés de temps), `:17` (étapes selon le mode), `:53` (sortie de la branche dense), `:100-108` (sortie des branches, RRF, rangs pour le debug), `:110-117` (ordre `dedup` puis `group_by`, sur-échantillonnage), `:119-132` (multi-index, `target_rank`).
- `docs/06-observabilite.md:74-84` (journal de debug, top 20 par branche), `:88-98` (`debug` et `explain` dans la réponse).
- `docs/02-architecture.md:53` (INV-01, aucun `iid` hors du moteur).
- Code : `crates/crible-search/src/pipeline.rs:361-380` (`rayon::join`, branches parallèles), `crates/crible-search/src/dense/planner.rs:253-265` (rangs lus avant le regroupement), `crates/crible-search/src/group.rs:178-184` (`candidates_per_branch`), `:206-232` (`dedup` puis `group_by`), `crates/crible-server/src/analytics.rs:151-194` (`StageDurations`, neuf étapes), `crates/crible-server/src/search.rs:235-243` (validation avant le jeton), `:431-432` (`total`), `:604-618` (`measure`).
- ADR-0008 (fusion RRF ou `relative_score`), ADR-0027 (`serde_json`), ADR-0043 (payload d'un document supprimé), ADR-0074 (stratégie dense, seuil exact, `rescore_candidates`), ADR-0075 (`dedup`, `group_by`, sur-échantillonnage, forme plate), ADR-0079 (tri total score puis clé), ADR-0090 (`relative_score`, `alpha`), ADR-0109 (refus de `debug` en P6, `took_ms`, preuve API = CLI), ADR-0110 (multi-index, RRF `1/(60 + rang)`, `target_rank`), ADR-0111 (objets fermés, marqueurs OpenAPI, validation des exemples), ADR-0117 (décision 5, `StageDurations`), ADR-0118 (journal de debug, preuve d'INV-17).

## Documents mis à jour par cet ADR

| Document | Modification |
|---|---|
| `docs/05-api.md:270` | le rôle de `debug` renvoie à ADR-0119 et dit que le journal est écrit par JOURNAL-DE-DEBUG |
| `docs/05-api.md:278` | `debug` est accepté depuis P7 (ADR-0119) ; `explain_ids` reste refusé jusqu'à EXPLIQUER-DOCUMENTS |
| `docs/05-api.md:303-317` | forme fermée de l'objet `debug`, exemple corrigé selon la décision 6, rangs et membres absents (décisions 1 à 5) |
| `docs/05-api.md:348`, `:376` | `debug` accepté sur la route multi-index, forme `targets[]` et `target_rank`, `timings_ms` global (décision 9) |
| `docs/03-recherche.md:100-108` | définition du rang d'une branche, après le filtre et avant `dedup`/`group_by` (décision 1) |
| `docs/03-recherche.md:132` | `target_rank` défini comme le rang dans la liste finale de l'index (décision 9) |
| `docs/phases/P7-observabilite.md:74-81` | la fiche `REPONSE-DEBUG` renvoie à ADR-0119 pour les rangs, les temps et la capture |
| `docs/adr/README.md` | ligne d'index de cet ADR |

Sont **non modifiés** : `docs/06` (sa prose reste vraie ; le journal de debug est livré par JOURNAL-DE-DEBUG), `docs/08`, `docs/suivi.md` (tenu par l'orchestrateur), le corps des ADR acceptés cités, et tout fichier de code à ce stade : le code est livré par REPONSE-DEBUG.

Les numéros de ligne de `docs/05-api.md`, de `docs/03-recherche.md` et de `docs/phases/P7-observabilite.md` cités dans cet ADR sont ceux de leur **état d'avant application** : la mise à jour de ces documents décale les sections suivantes, et c'est la modification ci-dessus qui fait foi.
