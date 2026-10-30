# ADR-0110 — Recherche multi-index : corps, fusion, validation et preuve de l'encodage unique

- **Statut** : Accepté (par anticipation, décision déléguée par le propriétaire le 2026-10-30 ; revue avec le propriétaire à la fin de la série)
- **Date** : 2026-10-30
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, tâche RECHERCHE-MULTI-INDEX (phase P6), sur délégation du propriétaire pour la troisième série de P6 et sur les verdicts de `@architecte` du 2026-10-30
- **Remplace** : — (ne remplace aucune décision : complète ADR-0109, ADR-0102 et ADR-0089, sans modifier le corps d'aucun d'eux)

## Contexte

### La fiche et ce qu'elle demande

La fiche `RECHERCHE-MULTI-INDEX` (`docs/phases/P6-serveur-et-api.md:113-127`) crée `POST /v1/search`. Elle demande cinq livrables : un seul encodage de la requête (`:118`), un filtre par cible (`:119`), « le refus d'un mélange de modèles (`incompatible_model`) » (`:120`), la fusion par cosinus en dense et par RRF en lexical et en hybride (`:121`), et le champ `index` dans chaque résultat (`:122`). Ses quatre critères d'acceptation sont : en dense, la fusion égale un tri global des top-k par index (`:124`) ; l'encodage n'est appelé qu'une fois, prouvé par un test (`:125`) ; les trois rôles atteignent la route, aucun `403` n'y est atteignable (`:126`, ADR-0095) ; au-delà de `[server].max_concurrent_searches` recherches simultanées, la route rend `503 overloaded` avec `Retry-After: 1`, comme `ROUTE-RECHERCHE` (`:127`, ADR-0102).

### Ce que la conception pose

`docs/03-recherche.md:119-132` pose le principe en sept points : une liste `targets: [{index, filter}]`, un filtre par index puisque chaque index a son schéma ; un modèle commun aux index d'une recherche **dense ou hybride**, sinon `incompatible_model` (`:122`) ; un encodage unique (`:123`) ; par index, pool, branches, fusion et regroupement séparés, puis le top-k de l'index (`:124`) ; le refus entier d'une cible sans lexical (`:126`, ADR-0089) ; une fusion par cosinus en dense et par RRF sur les rangs en lexical et en hybride (`:127-131`) ; le champ `index` et le rang dans l'index pour le debug (`:132`).

`docs/05-api.md:308-346` décrit la route : un exemple de corps, la présence d'`index` dans chaque résultat, la forme `{ "index": "...", "id": "..." }` des `explain_ids` en debug (P7), et le refus entier d'une cible sans lexical. Rien n'y décrit le tableau complet des champs du corps, la forme exacte de la réponse, l'ordre des validations ni les cas limites.

### Ce que les ADR voisins laissent ouvert

ADR-0109, décision 1, ferme le champ inconnu pour `POST /v1/indexes/{index}/search` et le dit explicitement : « le corps de `POST /v1/search` (fiche `RECHERCHE-MULTI-INDEX`, `docs/phases/P6:113-127`, `docs/05:304-318`) n'est pas couvert par la décision 1 : une divergence demanderait un ADR » (`docs/adr/0109-…md:72`). Son complément du 2026-10-30 fixe les entrées de la requête selon le mode (décision 4) et le refus d'un champ `returned: false` dans `return.fields` (décision 5), pour la route simple. Ces deux règles valent par leur énoncé pour un corps unique, pas pour un corps à cibles multiples.

ADR-0102 fixe la limite de corps (32 Mio), le sémaphore de `[server].max_concurrent_searches` pris sans attente, et l'échéance de `timeout_ms` ; ADR-0109 en précise le périmètre (tout le pipeline, encodage compris) et le point de départ (`took_ms` de la prise du jeton à la fin du calcul). Les deux ADR parlent de « la recherche » au singulier ; ils ne disent pas combien de jetons prend une requête à plusieurs cibles, ni comment une seule échéance couvre plusieurs index.

ADR-0075 fixe l'ordre `dedup` puis `group_by`, la forme plate de la réponse et la contiguïté des entrées d'un document. Il ne dit pas si ces étapes s'appliquent à l'intérieur d'un index ou entre index.

### Les NON-COUVERT du verdict `@architecte` du 2026-10-30

Le verdict relève quatre points non couverts :

1. **la forme du corps** : quels champs sont communs, lesquels sont propres à une cible, et ce que devient un champ inconnu dans un objet de `targets` ;
2. **la réponse** : la place des scalaires `pool_size` et `generation` de la route simple quand plusieurs index répondent, et la valeur de `truncated_query` quand un seul encodage sert toutes les cibles ;
3. **les cas limites** : `targets` absent, vide, avec un doublon ou un index inconnu ; `return.fields` sur des schémas hétérogènes ; l'ordre des refus quand plusieurs fautes coexistent ; le mode par défaut quand les cibles diffèrent par leur lexical ;
4. **la preuve de l'encodage unique** : le critère `:125` demande « un test », sans dire où il tourne ni comment il compte les appels, alors que la CI Rust n'a pas ONNX Runtime (`docs/07:144`) et qu'ADR-0062 interdit d'exclure un test de plus sans décision.

Le verdict relève aussi un **écart** : la fiche écrit « le refus d'un mélange de modèles » sans condition (`docs/phases/P6:120`), `docs/03:122` le limite aux recherches dense et hybride. La décision 3 traite cet écart comme une précision de la fiche par la conception, et non comme une contradiction à arbitrer : un modèle ne se compare que là où des vecteurs sont produits ou comparés, et `lexical` n'en produit ni n'en compare.

## Décision

### Décision 1 — le corps de `POST /v1/search`

**Constat.** `docs/05:228-249` donne un tableau fermé de quatorze champs pour la route simple. Le corps multi-index n'a ni tableau ni règle de composition ; la seule indication est l'exemple de `docs/05:313-316`, qui montre `query`, `mode`, `k`, `targets`, `group_by` au premier niveau et `filter` dans chaque cible.

**Décision.** Tous les champs de la route simple (`docs/05`, « Recherche : requête », `:232-247`) sont **communs, au premier niveau**, avec la même sémantique et les mêmes bornes : `query`, `vector`, `mode`, `k`, `fusion`, `group_by`, `passages_per_document`, `dedup`, `return`, `exact`, `timeout_ms`. `debug` et `explain_ids` sont refusés en P6, comme sur la route simple (ADR-0109, décision 1).

Seul `filter` est propre à une cible. Le corps porte `targets: [{ "index": "...", "filter": {...} }]` : `index` est obligatoire, `filter` est facultatif, et son absence vaut « aucun filtre ».

`fusion` absent : chaque index applique sa fusion configurée (`docs/03:106`). `fusion` présent : la valeur s'applique à **toutes** les cibles.

**Conséquences.** Un client apprend une seule grammaire et n'a qu'un jeu de bornes à retenir. Le corps multi-index reste une extension du corps simple, donc un futur champ de la route simple se transporte sans décision nouvelle. Coût : la validation doit distinguer le premier niveau des objets de `targets`, et `fusion` devient une valeur de requête partagée, sans réglage par cible.

### Décision 2 — le champ inconnu à tous les niveaux

**Constat.** ADR-0109, décision 1, refuse un champ inconnu « à tous les niveaux du corps » et nomme son chemin JSON, mais son périmètre s'arrête à la route simple (`docs/adr/0109-…md:72`). Un corps à cibles multiples a un niveau de plus : `targets[i]`. Une faute de frappe comme `$.targets[0].filtre` y serait ignorée sans cette décision.

**Décision.** La décision 1 d'ADR-0109 s'étend à ce corps, **à tous les niveaux**, objets de `targets[i]` compris : un champ inconnu rend `400 invalid_request`, et le message nomme le **chemin JSON** du champ (`$.targets[0].filtre`).

Une clause hors grammaire dans un filtre reste `invalid_filter` (400), avec `details.path` préfixé par `$.targets[i].filter` : le code ne change pas, seul le chemin s'allonge.

**Conséquences.** Aucune ignorance silencieuse, et le chemin JSON reste exploitable par un client qui veut corriger sa requête. Coût : la validation descend d'un niveau de plus et doit tenir à jour la liste des champs de `docs/05`.

### Décision 3 — le modèle commun

**Constat.** `docs/03:122` exige la même empreinte de modèle (ADR-0028) pour « tous les index d'une recherche dense ou hybride ». La fiche écrit le refus sans condition (`docs/phases/P6:120`). `docs/04:119` rappelle qu'un modèle `external` n'encode pas et exige un `vector` par requête. ADR-0109, décision 4 et complément du 2026-10-30, fixe les entrées selon le mode et le cas `external`, mais pour un seul index.

**Décision.** En `dense` et en `hybrid`, **toutes** les cibles doivent avoir la même empreinte de modèle, modèles `external` compris. Un `external` et un `onnx`, ou deux modèles différents, rendent `422 incompatible_model`, et le message nomme **deux index d'empreintes différentes** : la première cible, et la première qui diffère. En `lexical`, le modèle n'est pas comparé.

Les règles d'entrée d'ADR-0109 (décision 4 et complément du 2026-10-30) s'appliquent au **modèle commun** : sur des cibles `external`, `dense` exige `vector`, et `hybrid` exige `query` **et** `vector`.

La fiche est lue à travers `docs/03` : son « refus d'un mélange de modèles » vaut pour les modes qui encodent ou comparent des vecteurs.

**Conséquences.** Aucune cible ne peut être notée sur une échelle qui n'est pas la même que celle de ses voisines, ce qui rend le tri par cosinus de la décision 9 exact. `lexical` reste utilisable sur des index aux modèles différents, puisqu'aucun vecteur n'y entre. Coût : un refus `422` arrive après les contrôles `400` et `404` (décision 7), donc une requête peut coûter deux lectures de configuration avant d'être refusée.

### Décision 4 — le `mode` par défaut

**Constat.** `docs/05:236` donne le défaut de la route simple : `hybrid` si le lexical est activé, sinon `dense`. Avec plusieurs cibles, « le lexical est activé » n'a pas de référent unique.

**Décision.** Le défaut vaut `hybrid` si **toutes** les cibles ont le lexical activé, sinon `dense`. Un `mode` explicite `lexical` ou `hybrid` avec une cible sans lexical reste refusé en entier (ADR-0089, `docs/03:96`), le message nommant l'index fautif.

**Conséquences.** Le défaut ne choisit jamais un mode que la requête refuserait : il ne peut donc pas produire un `400` par sa seule valeur. Coût : un client qui ajoute une cible sans lexical voit le défaut basculer de `hybrid` à `dense` sans changer sa requête ; `score_type` le lui dit (`cosine` au lieu de `rrf`).

### Décision 5 — les cibles

**Constat.** Ni `docs/05` ni `docs/03` ne disent ce que vaut une liste absente, vide, avec un doublon ou avec un index inconnu. Le doublon n'est pas anodin : la fusion RRF de la décision 9 additionne des rangs, et le même index compté deux fois recevrait deux contributions.

**Décision.**

- `targets` absent ou vide : `400 invalid_request` ;
- un même index deux fois : `400 invalid_request`, message nommant `$.targets[i].index` ;
- un index inconnu : `404 not_found` nommant l'index, **aucun résultat partiel** ;
- **aucune borne propre** sur le nombre de cibles : les doublons et les inconnus étant refusés, il est borné par le nombre d'index enregistrés, lui-même borné par la taille de corps d'ADR-0102 (32 Mio). Aucune constante, aucune clé de configuration.

**Conséquences.** Une recherche partielle serait un résultat faux présenté comme complet ; le `404` franc évite ce mensonge. Le refus du doublon protège la fusion RRF d'un double comptage. Coût : une instance qui enregistrerait des milliers d'index n'a pas de garde propre, et la décision repose sur la limite de corps ; c'est un point à surveiller, pas une valeur à inventer.

### Décision 6 — `return.fields` sur des schémas hétérogènes

**Constat.** `docs/05:243` définit `return` pour un index : `"all"` vaut « tous les champs disponibles pour ce résultat » (ADR-0043), et un nom `returned: false` est refusé (ADR-0109, décision 5). Deux index n'ont pas le même schéma : un nom peut être déclaré par l'un et absent de l'autre.

**Décision.**

- une liste de noms est acceptée si **chaque nom est déclaré par au moins une cible** ;
- un nom déclaré par **aucune** cible est refusé en `400 invalid_request`, avec `$.return.fields[i]` ;
- un nom déclaré `returned: false` dans une cible est refusé en `400`, et le message nomme le champ, `$.return.fields[i]`, l'index et le fait qu'il est déclaré `returned: false` (ADR-0109, décision 5) ;
- chaque résultat porte **ceux des champs demandés que son index déclare** et qui sont disponibles pour lui (ADR-0043) ;
- `"all"` rend tous les champs `returned` de l'index du résultat.

**Conséquences.** Une demande reste satisfiable dès qu'une cible peut la servir, sans imposer un schéma commun aux index. Un résultat peut donc porter moins de champs qu'un autre : la liste plate et le champ `index` de la décision 10 rendent cet écart lisible. Coût : le contrôle d'un nom est global (toutes les cibles) alors que le refus cite une cible précise, ce qui demande de parcourir les schémas avant de rendre le message.

### Décision 7 — l'ordre des validations

**Constat.** Avec plusieurs cibles, plusieurs fautes peuvent coexister : un `k` hors bornes, un index inconnu, un filtre invalide et deux modèles différents. Rien ne dit laquelle rendre, et deux implémentations également conformes pourraient rendre des codes différents pour la même requête.

**Décision.** La première faute dans cet ordre est rendue :

1. **400** : forme du corps, champs inconnus, bornes, entrées selon le mode, `targets` vide, doublons ;
2. **404** : existence de chaque index, dans l'ordre de `targets` ;
3. **400** pour chaque cible, dans l'ordre : lexical désactivé, puis filtre contre son schéma (`invalid_filter`), puis `return.fields` contre l'ensemble des cibles ;
4. **422** : modèle commun ;
5. **503 `overloaded`** : prise du jeton de concurrence ;
6. **503 `overloaded`** : disponibilité lexicale de chaque cible pendant l'exécution (ADR-0088, ADR-0089).

**Conséquences.** Deux implémentations conformes rendent le même code pour la même requête, et un test peut fixer l'ordre. Les refus permanents et connus à la validation précèdent la prise du jeton, donc n'occupent aucune capacité. Coût : la validation n'est pas parallélisable par cible sans perdre l'ordre ; le parcours séquentiel est assumé, comme l'exécution (décision 8).

### Décision 8 — l'exécution

**Constat.** ADR-0102 et ADR-0109 fixent un jeton et une échéance pour **une** recherche. Une requête à plusieurs cibles pourrait en prendre un par cible, ou une échéance par cible ; dans les deux cas, `max_concurrent_searches` et `timeout_ms` ne voudraient plus dire la même chose que sur la route simple.

**Décision.**

- **un seul jeton** de concurrence pour toute la requête : même sémaphore et même `[server].max_concurrent_searches` que la route simple (ADR-0102) ;
- **une seule échéance** `timeout_ms` sur tout le pipeline, **encodage compris** (ADR-0109) ; au-delà, `504 timeout`, et le jeton reste tenu jusqu'à la fin réelle du calcul ;
- la requête est encodée **une fois**, par le même chemin et le même cache d'instance que la route simple (ADR-0051) ; le vecteur sert toutes les cibles ;
- les cibles s'exécutent **l'une après l'autre**, dans l'ordre de `targets`, dans la tâche bloquante de la requête : la recherche d'un index utilise déjà les fils de recherche (`search_threads`, `docs/07:68`), et aucun parallélisme entre cibles n'est ajouté sans mesure. Un parallélisme entre cibles est une idée à mesurer plus tard, pas une décision prise ici ;
- **par cible** : pool, branches, fusion, `dedup` puis `group_by` (ADR-0075), puis top-`k` de l'index, exactement comme la route simple ;
- **jamais** de `dedup` ni de regroupement entre index : un même `passage_hash` présent dans deux index reste deux entrées.

**Conséquences.** Les limites de la route simple valent sans exception pour la route multi-index, et le `503` ne ment pas sur la capacité consommée. Un encodage unique est aussi ce qui donne un sens à `truncated_query` (décision 10). Coût : la latence croît avec le nombre de cibles, sans recouvrement ; une requête à trois cibles peut dépasser `timeout_ms` là où trois requêtes simples passeraient. Ce coût est mesuré avant toute parallélisation.

### Décision 9 — la fusion entre index

**Constat.** `docs/03:127-131` nomme les deux méthodes — cosinus en dense, RRF sur les rangs en lexical et en hybride — sans dire ce qu'est une unité, comment se départagent les égalités, ni ce que vaut le score d'un document regroupé. Sans ces règles, la réponse n'est pas reproductible d'une implémentation à l'autre, et le critère `:124` (« même résultat qu'un tri global ») n'est pas vérifiable.

**Décision.**

- **Unité** : le passage sans `group_by` ; le document sous `group_by: "document"`, ses entrées restant consécutives, dans leur ordre de l'index.
- **`dense`** : unités triées par `score` cosinus décroissant (score du meilleur passage pour un document), puis nom d'`index` croissant, puis clé croissante ; `score_type` vaut `cosine`. Le résultat égale le tri global des top-k de chaque index.
- **`lexical` et `hybrid`** : RRF sur les rangs de chaque index, score = `1 / (60 + rang)`, où `rang` est le rang de l'unité dans la liste finale de son index, à partir de 1 ; tri par ce score décroissant, puis `index` croissant, puis clé croissante ; toutes les entrées d'un document regroupé portent le score de son document ; `score_type` vaut `rrf`, y compris avec une seule cible.
- **Troncature finale à `k` unités** : `k` résultats, ou `k` documents distincts (paires `index`, `doc_id`) et au plus `k × passages_per_document` entrées.

**Conséquences.** L'ordre est total : deux unités de score égal sont départagées par l'index puis par la clé, donc la réponse ne dépend pas de l'ordre d'itération d'une table de hachage. Le RRF multi-index réutilise la constante 60 de la fusion interne (`docs/03:101`) : un seul nombre à retenir. Coût : un index qui répond à lui seul est noté `rrf` et non `bm25`, ce qui change `score_type` par rapport à la route simple ; la troncature à `k` unités peut couper un document regroupé dont les passages se suivent, et c'est `k` documents qui fait foi, pas `k` entrées.

### Décision 10 — la réponse

**Constat.** La réponse de la route simple porte `pool_size` et `generation` au premier niveau (`docs/05:265-266`), valeurs d'un seul index. Une réponse multi-index en a autant que de cibles, et `truncated_query` dépend de l'encodage, qui est unique.

**Décision.**

- `results` : liste plate ; chaque entrée porte `index` en plus des champs de la route simple (`doc_id`, `passage`, `key`, `score`, `score_type`, `fields`, `text`) ;
- au lieu de `pool_size` et `generation` scalaires : `targets: [{ "index", "generation", "pool_size" }]`, dans l'ordre de la requête ;
- puis `took_ms` (de la prise du jeton à la fin du calcul, ADR-0109), `truncated_query` (même définition que la route simple, une seule valeur puisque l'encodage est unique ; `false` si rien n'est encodé) et `request_id` ;
- `target_rank` reste réservé au debug de P7.

**Conséquences.** Le diagnostic garde une valeur par cible, sans objet par document ni structure imbriquée dans `results` : la forme plate d'ADR-0075 est conservée. `truncated_query` reste un booléen unique, cohérent avec l'encodage unique. Coût : le client ne peut plus lire `pool_size` et `generation` au premier niveau ; les deux champs changent de place et de type entre les deux routes, et la spécification OpenAPI devra le dire (`PUBLIER-OPENAPI`).

### Décision 11 — la preuve de l'encodage unique

**Constat.** Le critère `docs/phases/P6:125` demande « un test » sans dire où il tourne. Un test qui encoderait réellement demanderait ONNX Runtime et le vrai modèle, donc le job `rust-onnx` (`docs/07:144`), et rejoindrait les tests locaux exclus par leur nom. ADR-0062, décision 3, interdit d'ajouter une exclusion sans décision, et ADR-0109 vient d'en nommer une cinquième : un test `#[ignore]` de plus serait un écart.

**Décision.** Le test qui montre que l'encodeur n'est appelé qu'une fois **tourne dans le job Rust de la CI**, sans ONNX Runtime et sans nouveau test `#[ignore]` (ADR-0062, ADR-0109). Il compte les **appels réels** à l'encodeur — pas les lectures du cache — au point d'encodage du serveur, par un double de test ou un compteur réservé aux tests. Les autres critères — fusion dense contre tri global, `503`, aucun `403` — tournent aussi en CI, sur des index `external` avec `vector` dans la requête.

**Conséquences.** Le critère a une preuve continue, sur le chemin réel du serveur, sans dépendance ni exclusion nouvelle. Le point d'encodage devient un point d'observation stable : c'est lui qui porte la preuve, pas une couche au-dessus. Coût : le test vérifie le nombre d'appels, pas l'identité du vecteur servi à chaque cible ; cette identité est couverte par le critère de fusion dense (`:124`), qui compare le résultat au tri global des top-k.

### Ce qui n'est pas décidé ici

- le **texte exact** des messages d'erreur, au-delà de ce qu'ils doivent nommer (chemin JSON, index, champ, `returned: false`) ;
- le **parallélisme entre cibles** : idée à mesurer, hors de cette tâche (décision 8) ;
- `target_rank`, `debug` et `explain_ids` : P7 (`REPONSE-DEBUG`, `EXPLIQUER-DOCUMENTS`) ;
- la forme des `explain_ids` en multi-index (`docs/05`, « Recherche multi-index »), qui appartient à P7 avec le champ lui-même ;
- la **spécification OpenAPI** de la route : `PUBLIER-OPENAPI` ;
- le **mécanisme exact** du double de test ou du compteur de la décision 11 : le choix appartient à l'implémentation, sous la contrainte de tourner dans le job Rust sans ONNX Runtime.

## Conséquences

- **Positives** :
  - le corps multi-index est une extension du corps simple : un seul tableau de champs, un seul jeu de bornes, un seul vocabulaire d'erreurs ;
  - aucune ignorance silencieuse, y compris dans `targets[i]` : le chemin JSON nomme la faute au niveau où elle vit ;
  - l'ordre total de la fusion (score, puis index, puis clé) rend la réponse reproductible et le critère « tri global » testable ;
  - l'encodage unique devient une propriété observable, et sa preuve tourne en CI, sur le chemin réel du serveur ;
  - le `503`, le `504` et `took_ms` gardent le sens qu'ils ont sur la route simple : un jeton, une échéance, une capacité ;
  - `lexical` reste disponible sur des index aux modèles différents, et le refus de mélange ne frappe que les modes qui comparent des vecteurs ;
  - la décision 5 ne crée ni constante ni clé de configuration : le nombre de cibles est borné par les index enregistrés et la limite de corps.
- **Négatives / coûts** :
  - deux champs de la réponse changent de place et de type entre la route simple et la route multi-index (`pool_size` et `generation` passent dans `targets`) ;
  - un résultat de la route multi-index porte `score_type: "rrf"` même en `lexical`, alors que la route simple rend `bm25` ;
  - la validation est séquentielle et le parcours des schémas est global pour `return.fields` : le code de validation grossit ;
  - la latence croît linéairement avec le nombre de cibles, sans recouvrement, et une requête à plusieurs cibles peut rendre `504` là où les mêmes recherches séparées passeraient ;
  - la troncature à `k` unités sous `group_by` peut couper une liste de passages, et le client doit lire `targets` et le nombre d'entrées plutôt que supposer `k × passages_per_document`.
- **À surveiller** :
  - le nombre d'index enregistrés par instance : au-delà de quelques dizaines, l'absence de borne propre sur `targets` (décision 5) devient une question, à rouvrir par un ADR et non par une constante ajoutée dans le code ;
  - la part des `504` attribuée aux cibles successives : si elle domine, le parallélisme entre cibles redevient une idée à mesurer (décision 8) ;
  - la fréquence des `422 incompatible_model` en `hybrid` : elle dirait qu'un client mélange des index que le mode par défaut rend incompatibles, et que la documentation du déploiement doit le dire plus tôt ;
  - le test de la décision 11 : s'il finit par demander ONNX Runtime, c'est le test qui change de job, pas le job Rust qui gagne ONNX ;
  - toute phrase de `docs/05` ou `docs/03` qui décrit la forme de la réponse : les deux routes diffèrent désormais par la place de `pool_size` et `generation` ;
  - `PUBLIER-OPENAPI` : la spécification doit décrire deux schémas de réponse distincts, et un test de `docs/05` contre la spécification le vérifiera (`docs/phases/P6:134`).

- **Documents mis à jour par cet ADR** :

| Document | Modification |
|---|---|
| `docs/adr/0110-recherche-multi-index-corps-fusion-et-validation.md` | cet ADR |
| `docs/adr/README.md` | ligne d'index de cet ADR |
| `docs/05-api.md`, « Recherche multi-index » | corps, réponse, ordre des validations, cas limites, en citant ADR-0110 |
| `docs/03-recherche.md`, « Recherche multi-index » | unité de fusion, RRF par rang, ordre des égalités, exécution séquentielle, `return.fields` |

Sont **non modifiés** : `docs/suivi.md` (tenu par l'orchestrateur), tout fichier de code et tout test, et les ADR antérieurs cités.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Un corps entièrement propre à la route multi-index, avec ses propres champs et ses propres bornes | Deux grammaires voisines divergeraient au premier champ ajouté à la route simple ; le client devrait apprendre deux vocabulaires pour la même recherche. La décision 1 fait du corps multi-index une extension du corps simple. |
| Un `filter` commun au premier niveau, appliqué aux cibles qui déclarent ses champs | Chaque index a son propre schéma (`docs/03:121`) : un filtre commun serait invalide sur une partie des cibles, ou silencieusement ignoré sur celles qui ne déclarent pas le champ. Le filtre par cible est ce que la conception pose. |
| Une `fusion` par cible, dans `targets[i]` | `fusion` est un réglage de la requête, pas du schéma d'un index, et la fusion de la décision 9 compare des rangs entre index : une fusion par cible rendrait les scores hétérogènes avant même la comparaison. La valeur commune garde un seul `score_type`. |
| Ignorer les champs inconnus dans `targets[i]` | C'est exactement la faute que la décision 1 d'ADR-0109 ferme pour la route simple, et `$.targets[0].filtre` est le cas le plus probable d'une faute de frappe. |
| Refuser un mélange de modèles en `lexical` aussi | `lexical` ne produit ni ne compare de vecteur : deux modèles différents y sont sans effet sur le score. Refuser serait une contrainte sans justification, et contredirait `docs/03:122`. |
| Comparer les modèles avant les index (422 avant 404) | Un index inconnu n'a pas de modèle à comparer : le `422` ne peut pas précéder l'existence des cibles sans mentir sur la cause du refus. |
| Un mode par défaut unique, `hybrid`, quelle que soit la cible | Il produirait un `400` par sa seule valeur dès qu'une cible n'a pas le lexical, donc un défaut qui ne fonctionne pas par défaut. |
| Refuser `targets` au-delà d'une constante, par exemple 16 | Aucun fait ne fixe un tel nombre, et le doublon et l'inconnu étant refusés, la borne réelle est le nombre d'index enregistrés. Une constante serait une valeur inventée, à réviser sans mesure. |
| Accepter un index en double et le compter une fois | Le RRF de la décision 9 additionnerait deux rangs du même index, et le même passage gagnerait un score que rien ne justifie. Un refus nommé vaut mieux qu'une déduplication implicite. |
| Refuser un nom de `return.fields` absent d'**une** cible | Aucun nom ne serait alors acceptable dès que deux cibles ont des schémas différents, ce qui est le cas d'usage de la route. La règle « déclaré par au moins une cible » garde la demande satisfiable. |
| Fusionner les entrées identiques entre index (`dedup` global) | Un même `passage_hash` dans deux index désigne deux passages de deux index, avec deux `doc_id` possibles et deux ensembles de champs : les fusionner effacerait l'origine. ADR-0075 s'applique à l'intérieur d'un index. |
| Paralléliser les cibles dès cette tâche | La recherche d'un index utilise déjà les fils de recherche ; ajouter un parallélisme entre cibles multiplierait la pression sur le même pool sans mesure. ADR-0102 a fixé la concurrence par recherche, pas par cible. |
| Noter `bm25` en `lexical` mono-cible et `rrf` au-delà | Deux `score_type` pour la même route selon le nombre de cibles rendraient la réponse dépendante d'une valeur de configuration. Le RRF uniforme est plus simple à documenter et à tester. |
| Un test `#[ignore]` de plus pour l'encodage unique | ADR-0062, décision 3, interdit une exclusion nouvelle sans décision, et ADR-0109 vient d'en nommer une cinquième. Le test demandé n'a pas besoin du vrai modèle : un compteur au point d'encodage suffit, et tourne dans le job Rust. |

## Références

- `docs/phases/P6-serveur-et-api.md:113-127` — fiche `RECHERCHE-MULTI-INDEX` : livrables (`:118-122`), critères d'acceptation (`:124-127`) ; `:99-111` — fiche `ROUTE-RECHERCHE` ; `:129-135` — `PUBLIER-OPENAPI`.
- `docs/03-recherche.md` : `:96` (branche désactivée, refus entier d'une cible), `:98-108` (fusion hybride, RRF `1/(60 + rang)`), `:110-117` (regroupement, ordre `dedup` puis `group_by`), `:119-132` (recherche multi-index).
- `docs/05-api.md` : `:34` et `:38-50` (corps d'erreur et codes), `:206-226` (grammaire de `filter`), `:228-253` (« Recherche : requête », entrées selon le mode et champ inconnu), `:255-306` (« Recherche : réponse », `pool_size` et `generation` à `:265-266`, `took_ms` à `:273`), `:308-346` (« Recherche multi-index »), `:371-385` (« Limites par défaut », corps de 32 Mio et sémaphore à `:385`).
- `docs/04-donnees-et-ingestion.md:119` — modèle `kind: "external"` : pas d'encodeur, pas de recette, `vector` obligatoire par requête.
- `docs/07-stack-et-deploiement.md:68` (`[server].search_threads`), `:144` (job Rust sans ONNX Runtime).
- `docs/08-invariants-et-tests.md:43` (`#[ignore]` seul mécanisme de saut), `:48` (exclusion nommée un par un).
- ADR-0028 (forme canonique de l'empreinte du modèle) ; ADR-0043 (payload d'un résultat dont le document est supprimé, `"all"` = champs disponibles) ; ADR-0051 (cache des requêtes) ; ADR-0062 (aucun autre test exclu) ; ADR-0075 (`dedup` puis `group_by`, réponse plate, contiguïté) ; ADR-0088 et ADR-0089 (états lexicaux, branche désactivée) ; ADR-0090 (`alpha`) ; ADR-0095 (aucun `403` atteignable sur les routes `read`) ; ADR-0101 (corps d'erreur) ; ADR-0102 (taille de corps, sémaphore, échéance) ; ADR-0109 (champs inconnus, périmètre de l'échéance, entrées selon le mode, `returned: false`, preuve « API = CLI »).
- Verdicts de `@architecte` du 2026-10-30 sur la fiche `RECHERCHE-MULTI-INDEX` : quatre `NON-COUVERT` (forme du corps, forme de la réponse, cas limites, preuve de l'encodage unique) et un `ECART` (refus d'un mélange de modèles : sans condition dans la fiche, limité au dense et à l'hybride dans `docs/03`).
