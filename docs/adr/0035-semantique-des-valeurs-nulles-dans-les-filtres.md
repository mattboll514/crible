# ADR-0035 — Sémantique des valeurs nulles dans les filtres

- **Statut** : Proposé
- **Date** : 2026-10-21
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, sur demande de l'orchestrateur, tâche GENERER-JEU-SYNTHETIQUE (phase P1), après le verdict NON-COUVERT de l'agent architecte
- **Remplace** : —

## Contexte

### Le trou dans la conception

`docs/04-donnees-et-ingestion.md` (l. 40) pose que « tous les champs acceptent null ». La grammaire des filtres de `docs/05-api.md` (l. 92-101) énumère les opérateurs par type de champ, mais ne dit jamais ce qu'un champ null fait à `eq`, `ne`, `in`, `nin`, `exists` ni `$not`.

`docs/03-recherche.md` (l. 25-27) décrit les structures qui permettront de trancher : une bitmap par valeur plus une bitmap « présent » pour `keyword` et `bool` ; une colonne de valeurs avec une sentinelle pour null plus une bitmap « présent » pour `integer`, `float` et `date` (ADR-0018, l. 14). La ligne 31 ne définit qu'un opérateur : « **NON** : vivants privés de l'enfant ». Rien ne dit si `ne` et `nin` se calculent comme ce complément, ou comme un complément restreint aux passages où le champ est présent.

`crates/crible-core/src/filter.rs` ne fait aujourd'hui que l'analyse syntaxique et la validation : il ne porte aucune sémantique d'évaluation. Rien dans le code ne tranche donc la question, et aucune Q-xx n'est ouverte sur le sujet.

### Ce qui rend la question contraignante

`docs/08-invariants-et-tests.md` (l. 15) définit INV-07 : « Filtre → bitmap ≡ évaluation SQL du même filtre », vérifié par un test de propriété qui compare le moteur à une évaluation SQLite JSON. SQL raisonne en logique à trois valeurs : `NULL <> 'a'` vaut UNKNOWN, et une clause UNKNOWN ne retient pas la ligne. Une sémantique de `ne` qui inclurait les nulls divergerait donc de la référence SQL que l'invariant impose.

### Ce que les outils de P1 font déjà

`tools/reference_topk.py`, livré par `@dev` pour GENERER-JEU-SYNTHETIQUE, calcule `expected.jsonl`, le top-k exact contre lequel J1, INV-06 et INV-07 compareront le moteur. Son en-tête (l. 29-30) annonce déjà la règle : « A null field value never matches a comparison, `ne` or `nin`; only `exists` looks at presence, and `$not` complements over every live passage. » La question n'est donc pas seulement documentaire : le choix retenu fixe aussi le contenu du jeu de référence.

## Décision

Adopter la logique à trois valeurs de SQL.

- `eq`, `ne`, `in`, `nin`, `lt`, `lte`, `gt` et `gte` sont des **comparaisons strictes** : un champ null n'en satisfait aucune.
- `exists: true` vaut « présent et non null » ; `exists: false` vaut « absent ou null ». Il n'existe pas, en v1, de moyen de filtrer un null explicite autrement que par `exists`.
- `$not` reste le complément ensembliste sur les vivants, tel que `docs/03-recherche.md` (l. 31) le définit. Il inclut donc les passages dont le champ est null : `{"$not": {"f": {"eq": "a"}}}` et `{"f": {"ne": "a"}}` ne sont **pas** équivalents.
- Mise en œuvre : `ne` et `nin` se calculent en `présent ∩ ¬bitmap(eq)` et `présent ∩ ¬bitmap(in)`, jamais comme un complément brut sur les vivants.

Ce qui n'est **pas** décidé ici :

- le comportement des champs multivalués, hors v1 (Q-03) ;
- la sémantique des filtres sur les champs non `filterable` ou inconnus, déjà couverte par `docs/05-api.md` et ADR-0030 ;
- la façon dont `$not` se combine aux clauses d'intervalle au-delà de ce que `docs/03` décrit déjà.

## Conséquences

- **Positives** :
  - une seule règle, celle de SQL, pour tous les opérateurs : `ne` et `nin` ne surprennent pas un appelant qui connaît SQL ;
  - INV-07 devient vérifiable sans convention cachée : le test de propriété peut traduire le filtre en SQL et comparer les ensembles ;
  - la bitmap « présent » prévue par ADR-0018 et `docs/03` trouve son usage : elle porte la différence entre `ne` et `$not` ;
  - `tools/reference_topk.py` et `expected.jsonl` restent valides : aucune régénération n'est nécessaire si cette option est retenue.
- **Négatives / coûts** :
  - deux opérateurs qui se ressemblent (`ne` et `$not` + `eq`) donnent des résultats différents sur les nulls : il faut le documenter explicitement, sinon c'est un piège pour l'appelant ;
  - `ne` et `nin` coûtent une intersection de plus avec la bitmap « présent » ;
  - le test de propriété d'INV-07 doit traduire `$not` en SQL sans tomber dans le piège des trois valeurs : un `NOT (x = 'a')` exclurait les nulls et ferait échouer le test pour une mauvaise raison. Il faut écrire `... IS NOT TRUE` ou `NOT COALESCE(..., 0)` ;
  - le comportement d'`exists` sur un champ absent et sur un champ null est identique : c'est voulu, mais cela interdit de distinguer les deux cas côté API.
- **À surveiller** : toute divergence constatée entre le moteur et l'évaluation SQL d'INV-07 sur un jeu contenant des nulls ; un besoin réel, chez un producteur, de distinguer un champ absent d'un null explicite, qui rouvrirait l'option C.

En cas d'acceptation :

- `docs/05-api.md` (grammaire des filtres) et `docs/03-recherche.md` (filtres → pool) documentent la règle ;
- EVALUER-FILTRES calcule `ne` et `nin` en `présent ∩ ¬bitmap(...)` ;
- les tests prévus comparent `{"f": {"ne": "a"}}` à `{"$not": {"f": {"eq": "a"}}}` sur un passage dont `f` est null, et vérifient `exists` sur un null ;
- `tools/reference_topk.py` reste inchangé.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| B — `ne` et `nin` sont le complément sur les vivants, nulls compris | `ne` devient redondant avec `$not` + `eq`, ce qui surprend l'appelant, et la sémantique diverge de SQL, donc de l'évaluation de référence d'INV‑07. |
| C — distinguer un champ absent d'un null explicite | Le JSON canonique les distingue (ADR-0019), mais les colonnes Parquet non : tout est NULL. Les deux niveaux divergeraient, avec un coût de stockage et une règle de plus à tenir. |

## Références

`docs/04-donnees-et-ingestion.md` (l. 40) ; `docs/05-api.md` (grammaire des filtres, l. 92-101) ; `docs/03-recherche.md` (l. 25-31) ; `docs/08-invariants-et-tests.md` (INV-07, l. 15) ; ADR-0018 (filtres d'intervalle par colonnes de valeurs) ; ADR-0019 (champs en JSON canonique) ; ADR-0029 (limites de filtre) ; ADR-0030 (nom de champ déclaré aux deux niveaux) ; `tools/reference_topk.py` ; fiche `GENERER-JEU-SYNTHETIQUE` (phase P1).
