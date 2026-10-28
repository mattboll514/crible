# ADR-0074 — Planification de la stratégie dense : seuil exact, valeur par défaut, nom de la stratégie et k au-delà de N

- **Statut** : Accepté (accepté par anticipation le 2026-10-27, confirmé par le propriétaire le 2026-10-27)
- **Date** : 2026-10-27 (proposition et acceptation anticipée)
- **Décideurs** : propriétaire
- **Décision** : accepté par anticipation le 2026-10-27, par délégation du propriétaire pour la série RESCORER-FP16, INSERER-POOL-EN-ORDRE, ACCELERER-RECETTE, PLANIFIER-STRATEGIE, confirmé par le propriétaire le 2026-10-27. Les cinq décisions sont retenues telles que proposées, sans erratum : le domaine du seuil exact va de `0` à `u32::MAX` sans autre borne ; `exact_threshold` devient optionnelle dans `index.json` et sa valeur par défaut, égale à `rescore_candidates`, est **matérialisée** à la création de l'index ; `dense_strategy` vaut `"exact"` ou `"binary"` ; sous BINARY, le scan garde `max(N, k)` candidats, tous rescorés ; un pool de taille égale au seuil passe par EXACT, et le glossaire est aligné sur cette lecture.
- **Proposé par** : agent greffier, sur demande de l'orchestrateur, pour la tâche `PLANIFIER-STRATEGIE` (phase P4), à partir des questions de `@testeur` et des verdicts de `@architecte` ; les options sont retenues par l'orchestrateur par délégation
- **Remplace** : —

## Contexte

### Ce que la fiche demande

La fiche `PLANIFIER-STRATEGIE` (`docs/phases/P4-recherche-rapide.md:59-71`) demande de choisir entre EXACT et BINARY, de forcer l'exact à la demande, et d'exposer la stratégie choisie dans le plan. Ses livrables sont « la sélection selon `exact_threshold` et `search.scan` », « l'option `exact` », « la stratégie choisie, exposée dans le plan (pour le debug, P7) » et « le réglage par `PATCH` au niveau de la bibliothèque » (`:63-67`). Ses critères sont INV-06 et l'égalité des résultats de `exact: true` avec P1 (`:68-71`).

La fiche ne dit pas **quelle valeur exacte** prend la stratégie dans le plan, ni **ce qui se passe** quand `k` dépasse N, ni **quel domaine** vaut `exact_threshold`, ni **comment** se calcule le défaut « égal à N ».

### Ce que la conception fixe

- **La règle de choix** (`docs/03-recherche.md:43-47`) : un pool de taille **≤** `exact_threshold` passe par EXACT ; sinon BINARY ; `exact: true` force EXACT quelle que soit la taille du pool.
- **Le défaut du seuil** (`docs/03-recherche.md:62`) : `search.exact_threshold` est réglable, avec pour défaut « égal à N ».
- **N est un levier de rappel** (`docs/03-recherche.md:67-68`) : si le rappel du binaire est insuffisant, on augmente N d'abord. Le seuil exact suit N par défaut, et les deux sont réglables séparément.
- **Les seuls domaines écrits** sont ceux d'`ingest.*` (`docs/04-donnees-et-ingestion.md:57-64`, ADR-0054) : `acceptance_sample ≥ 1` et `acceptance_min_cosine ∈ ]0, 1]`. Aucune autre clé de configuration n'a de domaine décrit. `search.*` est réglable (`docs/04:48`), et l'exemple d'`index.json` écrit `exact_threshold` en clair (`docs/04:29`).
- **`k`** va de 1 à 1000 (`docs/05-api.md:162`). L'exemple de plan expose déjà `"dense_strategy": "binary"` et `rescore_candidates` (`docs/05:197-198`), sans énumérer les valeurs admises de `dense_strategy`, et sans rien dire du nombre de candidats rescorés quand `k` dépasse N.
- **Le code** exige `exact_threshold` à la lecture d'`index.json` (`crates/crible-core/src/config.rs:428`), accepte les clés `search.*` une à une dans un `PATCH` (`:854-899`, ADR-0031), et nomme les stratégies `Binary` et `None` (`:313`, `ScanStrategy`). `None` veut dire « toujours exact » (`docs/03:60`).
- **Le glossaire** définit le seuil exact comme la « taille de pool **en dessous de laquelle** on calcule tout en exact » (`docs/00-glossaire.md:21`), ce qui exclut l'égalité, contrairement à `docs/03:45`.

### Les questions tranchées ici

`@architecte` a rendu les verdicts suivants sur les quatre questions de `@testeur` :

- point 1, domaine de `search.exact_threshold` : **NON-COUVERT** ;
- point 2, valeur par défaut du seuil : **ECART**, parce que « égal à N » n'est pas une valeur stockée et que `docs/03:62` ne dit pas quand elle est figée ;
- point 3, nom de la stratégie dans le plan : **NON-COUVERT** ;
- point 4, `k` au-delà de N sous BINARY : **NON-COUVERT** ;
- la forme interne du plan est **CONFORME** : `docs/05` décrit le plan exposé, et rien n'oblige la forme interne de P4 à être sérialisée avant P7 ;
- la borne à l'égalité est **confirmée** par `docs/03:45` : le glossaire est plus étroit que la conception, et c'est la conception qui fait foi.

Aucune de ces questions ne se tranche par la lecture seule : elles fixent des valeurs par défaut et un contrat de debug, donc un ADR.

## Décision

**La stratégie dense est planifiée par une règle unique et entièrement déterminée : le domaine du seuil, son défaut matérialisé, le nom publié de la stratégie et le nombre effectif de candidats rescorés sont fixés ici.**

Précisions :

1. **Domaine de `search.exact_threshold`.** Tout entier de `0` à `u32::MAX`, sans autre borne. `0` signifie que seul un pool vide passe par EXACT, sauf `exact: true` ; `u32::MAX` signifie toujours EXACT. Aucune nouvelle erreur n'est introduite : toute valeur du domaine est acceptée, et les valeurs hors du domaine (négatives, non entières, au-delà de `u32::MAX`) restent refusées par la validation de type existante.
2. **Valeur par défaut.** La clé `search.exact_threshold` devient **optionnelle** dans `index.json`. Absente à la création de l'index, elle prend la valeur de `search.rescore_candidates` à ce moment, et cette valeur est **écrite dans la configuration stockée** : elle se lit ensuite comme une valeur explicite. Un `PATCH` ultérieur de `rescore_candidates` ne déplace pas le seuil ; seul un `PATCH` de `exact_threshold` le change. Le comportement de `PATCH` (ADR-0031) est inchangé : les clés connues restent réglables, la clé inconnue reste refusée.
3. **Nom de la stratégie dans le plan.** `dense_strategy` vaut `"exact"` ou `"binary"`. `search.scan: none` donne `"exact"`, puisque ce réglage force le calcul exact (`docs/03:60`). La forme interne du plan est libre en P4 et n'est pas sérialisée ; le plan exposé est assemblé selon `docs/05` en P7. Aucune valeur autre que ces deux chaînes n'est publiée.
4. **`k` au-delà de N sous BINARY.** Le scan binaire garde `max(N, k)` candidats, tous rescorés, et le plan expose ce nombre **effectif** de candidats rescorés. `k` est le nombre de résultats que demande la branche dense. Aucune erreur n'est rendue, et il n'y a pas de repli sur EXACT.
5. **Borne à l'égalité.** Un pool de taille **égale** à `exact_threshold` passe par EXACT, conformément à `docs/03:45`. Le glossaire est aligné : « taille de pool jusqu'à laquelle (incluse) la recherche est exacte ».

Ce qui n'est **pas** décidé ici : la valeur de N, et donc la valeur par défaut du seuil, qui relèvent de MES-01 et de Q-01 ; le seuil entre parcours dense et clairsemé du scan binaire, déjà fixé par ADR-0071 ; la forme sérialisée du plan, qui relève de P7 ; la parallélisation du scan et du rescoring, hors fiche.

## Conséquences

- **Positives** :
  - la règle de choix est totale : chaque couple (taille du pool, `exact_threshold`, `exact`, `scan`) donne une stratégie et un nombre de candidats, sans cas non défini ;
  - le défaut matérialisé rend la configuration stockée auto-descriptive : relire `index.json` suffit à savoir quel seuil s'applique, sans rejouer la règle « égal à N » ni l'ordre des `PATCH` ;
  - séparer N et le seuil garde à MES-01 deux leviers distincts : augmenter N élargit le rappel sans changer la frontière EXACT/BINARY, changer le seuil déplace la frontière sans changer N ;
  - `"exact"` et `"binary"` nomment la stratégie, pas le réglage : le plan reste lisible quand `exact: true` force EXACT alors que `search.scan` vaut `binary` ;
  - `max(N, k)` évite de rendre moins de résultats que `k` demandés, et le plan dit le nombre réellement rescoré.
- **Négatives / coûts** :
  - un index existant doit être relu avec `exact_threshold` explicite, et la création doit matérialiser la valeur : c'est un changement de code à faire dans `PLANIFIER-STRATEGIE`, sans migration de format (la clé reste au même endroit, elle devient seulement optionnelle à la création) ;
  - deux index créés avec le même `rescore_candidates` mais un `PATCH` de seuil différent divergent silencieusement : la divergence est voulue, mais elle n'est visible que dans `index.json` ;
  - sous BINARY avec un `k` proche de 1000 et un N faible, le rescoring travaille plus que N : le budget de MES-03 (< 1 ms à chaud pour N = 1000) ne couvre plus ce cas ;
  - `0` et `u32::MAX` sont des valeurs de bord utiles au test, mais elles rendent un index volontairement dégradé (tout binaire, ou tout exact) sans avertissement.
- **À surveiller** :
  - MES-01 : la valeur retenue pour N fixe le défaut du seuil des index créés ensuite ; si le rappel du binaire reste insuffisant à N = 1000, la décision 4 fait grossir le rescoring avec `k` ;
  - MES-03 : si un `k` élevé sous BINARY pèse sur la latence de la branche dense, la règle `max(N, k)` devra être revue par un nouvel ADR ;
  - toute reprise de la sémantique du défaut (déplacer le seuil avec N) demanderait un nouvel ADR, puisque la matérialisation est un choix, pas un détail d'implémentation.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Borner `exact_threshold` à `[1, taille de l'index]` | Aucun usage ne le demande, et la borne serait arbitraire ou dépendante de l'index au moment du `PATCH` ; `0` et `u32::MAX` ont un sens clair et servent aux tests de bord |
| Garder `exact_threshold` obligatoire dans `index.json` | La conception parle d'un défaut (`docs/03:62`), et un défaut obligatoire n'en est pas un : la création devrait toujours écrire la clé, ce qui contredit « égal à N » |
| Recalculer le seuil à chaque lecture depuis `rescore_candidates` | Le seuil suivrait N après un `PATCH` de N, donc changerait de comportement sans `PATCH` du seuil, et la valeur effective ne serait plus lisible dans `index.json` |
| Publier `dense_strategy` avec les valeurs du réglage `"none"` et `"binary"` | Le plan décrit la stratégie exécutée, pas le réglage : `"none"` pour un calcul exact est obscur, et `exact: true` donnerait `"none"` alors que la branche calcule en exact |
| Garder N candidats quand `k` dépasse N | La branche dense rendrait moins de `k` résultats que demandés, ou devrait compléter par un autre chemin non décrit |
| Se replier sur EXACT quand `k` dépasse N | Le coût deviendrait imprévisible à la requête, alors que le pool peut être grand ; la décision ne veut aucune bascule cachée |
| Aligner `docs/03` sur le glossaire (« en dessous ») | `docs/03:45` fait foi et INV-06 parle d'un pool « ≤ seuil exact » ; exclure l'égalité rendrait le test de bord ambigu, et le glossaire est plus facile à corriger |

## Références

- Fiche : `docs/phases/P4-recherche-rapide.md:59-71` (`PLANIFIER-STRATEGIE`).
- Conception : `docs/03-recherche.md:43-47`, `:57-62`, `:67-70` ; `docs/04-donnees-et-ingestion.md:26-35`, `:44-48`, `:57-66` ; `docs/05-api.md:162`, `:195-206` ; `docs/00-glossaire.md:19-21` ; INV-06 dans `docs/08-invariants-et-tests.md`.
- Code : `crates/crible-core/src/config.rs:313` (`ScanStrategy`), `:428` (`exact_threshold` exigé), `:854-899` (`PATCH`).
- ADR liés : ADR-0005 (étages de recherche réglables), ADR-0004 (codes binaires et scan), ADR-0031 (clé inconnue dans un `PATCH`), ADR-0054 (domaines de la recette), ADR-0070 (noyaux SIMD), ADR-0071 (seuil entre parcours du scan binaire), ADR-0072 (produit scalaire partagé), ADR-0073 (recette sans encodage par lots).
- Question ouverte liée : Q-01 (N par défaut et cible de fidélité du scan binaire, MES-01).
