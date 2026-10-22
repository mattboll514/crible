# ADR-0046 — Budget des filtres d'intervalle à l'échelle de Legifacile

- **Statut** : Accepté (décision du propriétaire le 2026-10-22)
- **Date** : 2026-10-22 (proposition et acceptation)
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, sur demande du propriétaire. Décision « D1 » de `docs/suivi.md`, section « Décisions en attente », issue de MES-14 (tâche `EVALUER-FILTRES`, P1)
- **Remplace** : —

## Contexte

`docs/03-recherche.md`, section « Budget de latence indicatif », donne une seule ligne pour le filtrage : « Filtre (bitmaps) | < 1 ms | MES-14 ». MES-14 a mesuré cette ligne sur trois tailles d'index (9 750, 1 M et 8,58 M passages vivants, cache chaud) et rend deux résultats distincts.

**Les clauses en bitmap tiennent le budget.** À 8,58 M passages vivants, une clause `keyword` seule coûte 50,9 µs (`partie=corps`, 1 541 689 passages dans le pool) à 209,6 µs (`categorie=cat-03`, 536 250 passages dans le pool), soit 4,8 à 19,6 fois sous 1 ms.

**Les clauses d'intervalle le dépassent de deux ordres de grandeur.** Toujours à 8,58 M et cache chaud, sur un domaine plein (tous les vivants) :

| Cas, à 8,58 M passages | iid lus | p50 | Coût par iid lu |
|---|---|---|---|
| `score > 0.6` | 8 150 880 | 101,2 ms | 12,4 ns |
| `date_publication` 2000-01-01..2003-12-31 | 9 513 200 (deux bornes) | 114,7 ms | 12,1 ns |
| `annee` 2000..2010 | 14 034 695 (deux bornes) | 194,4 ms | 13,9 ns |

Ces chiffres sont les p50 de MES-14 §4.2 ; les p95 valent 101,6, 116,1 et 216,9 ms. Un domaine déjà réduit par une clause en bitmap ne suffit pas à revenir sous 1 ms : `categorie=cat-03` ramène la plage d'années à 21,1 ms et celle des dates à 9,1 ms, `partie=corps` à 49,2 et 24,9 ms (MES-14 §4.2 et §7). Le coût est **linéaire dans le nombre d'iid lus**, entre 9 et 24 ns par iid, et la loi tient sur les trois tailles mesurées, sans extrapolation.

MES-14 §5 donne la règle d'exploitation qui en découle : à 8,58 M passages vivants et cache chaud, **1 ms d'intervalle ≈ 40 000 à 110 000 iid lus**. Pour tenir sous 1 ms, une clause en bitmap doit donc avoir réduit le domaine à quelques dizaines de milliers de passages, soit une sélectivité 5 à 13 fois supérieure à celle de `categorie=cat-03` sur ce jeu.

ADR-0018 avait annoncé le cas (« une clause d'intervalle seule parcourt toute la colonne ») et renvoyé la décision à cette mesure : « Si c'est trop lent, ajouter des bitmaps par tranche (par année), avec un nouvel ADR. » MES-14 §7 propose l'index de plage, mais le gain attendu n'est **pas mesuré** : il se déduit du coût d'une clause en bitmap (51 à 210 µs), sans campagne sur une implémentation existante.

**Ce que pèse un dépassement à l'échelle de la requête.** Le pool se calcule avant le scan dense et avant la branche lexicale. Les autres étapes du budget de `docs/03` sont l'encodage de la requête (20 à 50 ms, MES-05) et le scan du pool (20 à 80 ms pour 8,6 M passages, MES-02). Les cibles de `docs/suivi.md` fixent la latence chaude mono-corpus à **p95 ≤ 698 ms**, contraignante seulement sur le Mac mini en conteneur, et rappellent le principe : « le rappel prime, la latence est indicative ».

**Ce que la mesure ne dit pas.** Les points à l'échelle sont des fixtures écrites par le banc, sans pierres tombales, sans segments et sans vecteurs, et leur distribution est celle de `synth-small`, **pas celle de Legifacile** (longueur des plages de dates, cardinalité réelle des champs). La part de `RoaringBitmap::insert` dans `scan_column` est **calculée** (environ 70 ms des 194,4 ms du domaine plein ; environ 13,3 ms des 21,1 ms du cas `categorie=cat-03` ET `annee`), pas mesurée isolément : c'est une déduction du § 5 de MES-14, appuyée sur la lecture de `roaring` 0.11.5, où l'insertion fait un `binary_search` puis un `Vec::insert`.

## Décision

Le budget de filtrage est **séparé en deux lignes**, et le dépassement des filtres d'intervalle est **assumé dans le budget de bout en bout**. Aucune cible de `docs/suivi.md` ne change, aucun seuil (`N`, `exact_threshold`) ne change, et **aucun index de plage n'entre en v1**. Une tâche dédiée supprime la part d'insertion dans la bitmap de résultat. Le déclencheur d'un index de plage est conservé, avec une mesure de plus en P9 pour le trancher.

1. **Budget séparé (`docs/03`).** La ligne « Filtre (bitmaps) < 1 ms » reste, validée par MES-14. Une ligne distincte, « Filtre d'intervalle (colonne de valeurs) », porte la valeur mesurée (101 à 194 ms à domaine plein, 9,1 à 49,2 ms sous un domaine réduit) et la règle d'exploitation de MES-14 §5 : 1 ms ≈ 40 000 à 110 000 iid lus. Ce n'est pas une cible : c'est un ordre de grandeur mesuré, avec la règle qui permet de dire, requête par requête, si le filtre tiendra.

2. **Dépassement assumé de bout en bout.** Le pire cas mesuré donne 194 ms de filtre, 20 à 50 ms d'encodage et 20 à 80 ms de scan, soit environ 330 ms, sous la cible de 698 ms. Aucune cible de latence n'est révisée : celle de 698 ms reste contraignante sur le Mac mini en conteneur et indicative ailleurs.

3. **Nouvelle tâche `INSERER-POOL-EN-ORDRE`, en P4, juste avant `MESURER-LATENCE-DENSE`.** Elle construit la bitmap de résultat de `scan_column` dans l'ordre croissant des iid, au lieu d'insérer iid par iid. Pas de changement de format, pas de nouvelle structure, pas de dépendance nouvelle. Ses critères sont un test d'égalité des bitmaps produites contre l'implémentation actuelle, puis un rejeu de MES-14 avec les mêmes fixtures ; `docs/08` classe déjà MES-14 en P1 et en P4. La fiche complète figure dans les conséquences ci-dessous.

4. **Index de plage différé (option b).** Le déclencheur d'ADR-0018 est conservé et précisé : la fiche `COMPARER-AVEC-PGVECTOR` (P9) reçoit un livrable de plus, la latence p50/p95 des requêtes avec filtre d'intervalle sur l'index réel de Legifacile (rejeu de MES-14 sur cet index). On n'ouvre un ADR d'index de plage que si la **p95 de bout en bout** de ces requêtes dépasse la cible de latence v1.

   Trois raisons au report. Le gain d'un index de plage n'est pas mesuré. Les fixtures de MES-14 suivent la distribution de `synth-small`, pas celle de Legifacile, et une structure dimensionnée sur ces fixtures pourrait être inutile ou mal calibrée. Enfin l'index coûterait de la mémoire, du temps de démarrage (déjà 20 s à 8,58 M passages d'après MES-13) et de la maintenance en P3.

Ce qui n'est **pas** décidé ici : la forme d'un futur index de plage, la valeur d'une éventuelle cible distincte pour les filtres d'intervalle, et tout changement de `N` ou de `exact_threshold` (MES-14 §7 ne permet pas de les déduire).

## Conséquences

- **Positives** :
  - le budget de `docs/03` cesse d'annoncer une cible que la structure d'ADR-0018 ne peut pas tenir : une revue ne supposera plus que les filtres d'intervalle tiennent sous 1 ms à 8,58 M passages ;
  - la règle « 1 ms ≈ 40 000 à 110 000 iid lus » rend le coût prévisible requête par requête, sans nouvelle mesure ;
  - le budget de bout en bout est cohérent avec les chiffres : ~330 ms au pire cas mesuré contre 698 ms de cible, et aucune cible n'est affaiblie ;
  - une tâche bornée (`INSERER-POOL-EN-ORDRE`) attaque le seul terme identifié comme évitable, sans format nouveau ni dépendance.
- **Négatives / coûts** :
  - une requête dont le filtre d'intervalle porte sur un domaine large reste lente (101 à 194 ms de filtre à 8,58 M) jusqu'à un éventuel index de plage ;
  - la ligne « intervalle » du budget de `docs/03` n'est pas une cible vérifiable : elle demande au lecteur de comparer à une mesure, pas à un seuil ;
  - le report de l'index de plage laisse la décision ouverte jusqu'en P9, donc proche de la bascule v1, et la tâche `INSERER-POOL-EN-ORDRE` ne restitue qu'une fraction calculée (≈ 70 ms sur 194,4 au plus) ;
  - une tâche de plus (84 au total).
- **À surveiller** :
  - le rejeu de MES-14 après `INSERER-POOL-EN-ORDRE` : si le gain est nul, la part d'insertion était mal calculée, et l'interprétation du § 5 de MES-14 doit être corrigée ;
  - la latence p50/p95 des requêtes filtrées par intervalle sur l'index réel, livrée par `COMPARER-AVEC-PGVECTOR` : une p95 de bout en bout au-dessus de la cible v1 rouvre l'option (b) ;
  - toute mesure de bout en bout qui placerait le filtre d'intervalle au-delà de ~400 ms dans le budget chaud : la marge sous 698 ms ne serait plus de deux fois.

### Modifications à appliquer après acceptation

Aucune n'est appliquée par cet ADR : le propriétaire doit d'abord le passer en `Accepté`. Une fois accepté, `@greffier` applique les modifications suivantes, dans un commit qui cite `ADR-0046`.

1. **`docs/03-recherche.md`**, section « Budget de latence indicatif ». Remplacer la ligne

   ```
   | Filtre (bitmaps) | < 1 ms | MES-14 |
   ```

   par ces deux lignes, en conservant l'ordre des étapes (le filtre reste une seule étape) :

   ```
   | Filtre (bitmaps) | < 1 ms | MES-14 |
   | Filtre d'intervalle (colonne de valeurs) | 101 à 194 ms à domaine plein, 9,1 à 49,2 ms sous un domaine réduit, à 8,58 M passages ; 1 ms ≈ 40 000 à 110 000 iid lus (ADR-0046) | MES-14 |
   ```

   Ajouter sous le tableau une phrase de renvoi : `ADR-0046` sépare les deux lignes ; la valeur d'intervalle est **mesurée, pas une cible**, et le dépassement est assumé dans les budgets de bout en bout.

2. **`docs/phases/P4-recherche-rapide.md`** : insérer la fiche `INSERER-POOL-EN-ORDRE` juste avant `### MESURER-LATENCE-DENSE`, telle qu'écrite ci-dessous.

3. **`docs/phases/P9-migration-legifacile.md`**, fiche `COMPARER-AVEC-PGVECTOR`, section « Livrables ». Ajouter un livrable, dans le même esprit que les autres :

   - la latence p50/p95 des requêtes **avec filtre d'intervalle** sur l'index réel de Legifacile, par rejeu de MES-14 sur cet index (mêmes cas : domaine plein, domaine réduit par une clause en bitmap), comparée à la cible de latence v1.

   Et une phrase au critère d'acceptation : un ADR d'index de plage n'est ouvert que si cette p95 **de bout en bout** dépasse la cible (ADR-0046, qui prolonge ADR-0018).

4. **`docs/09-plan-de-developpement.md`** : vérifié, ce document ne porte **pas** le nombre total de tâches. Le nombre 83 n'y figure pas ; il n'apparaît que dans le journal de `docs/suivi.md` (ligne du 2026-10-18), qu'on ne réécrit pas. Aucune modification n'est donc nécessaire dans `docs/09` ; à revérifier au moment de l'application, si le document a changé d'ici là.

5. **`docs/08-invariants-et-tests.md`** : aucune modification nécessaire, la ligne MES-14 y est déjà classée « P1, P4 ».

6. **`docs/suivi.md`** : c'est l'**orchestrateur** qui tient ce fichier, pas `@greffier`. Il y ajoute la ligne de tâche `INSERER-POOL-EN-ORDRE` (phase P4, statut à faire, branche à créer) et retire D1 de « Décisions en attente » quand l'ADR est accepté. `@greffier` ne touche ni au suivi des tâches ni au journal.

### Fiche à insérer dans `docs/phases/P4-recherche-rapide.md`

```markdown
### INSERER-POOL-EN-ORDRE
- **Objectif** : construire la bitmap de résultat d'une clause d'intervalle dans l'ordre croissant des iid, au lieu de l'insérer iid par iid.
- **Dépend de** : EVALUER-FILTRES
- **À lire** : `03-recherche.md` (filtres → pool, budget de latence) ; `mesures/MES-14-cout-filtres-intervalles.md` (§ 4.2 et § 5) ; ADR-0018 ; `crates/crible-search/src/pool.rs` (`scan_column`).
- **Livrables** :
  - dans `scan_column`, la construction de la bitmap de résultat en ordre croissant des iid : parcours croissant avec ajout en fin quand la colonne est lue dans l'ordre, ou construction triée puis conversion, au choix de l'implémentation ;
  - aucune nouvelle structure, aucun changement de format, aucune dépendance nouvelle ;
  - le rapport de MES-14 rejoué, avec les chiffres d'avant et d'après.
- **Critères d'acceptation** :
  - un test vérifie que la bitmap produite est **égale** à celle de l'implémentation actuelle, sur les cas de MES-14 (domaine plein, domaine réduit par `partie=corps` et par `categorie=cat-03`, plage à une et à deux bornes, aucun gardé, tous gardés) ;
  - **INV-07** reste vert : filtres aléatoires en proptest, comparés à l'évaluation SQLite ;
  - MES-14 est rejouée avec les mêmes fixtures (`fixture-1m`, `fixture-8m58`, mêmes distributions) et les mêmes cas ; le rapport donne le p50 et le p95 avant et après, et le coût par iid lu ;
  - `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` et `cargo test` passent.
- **Invariants / mesures** : INV-07, MES-14.
- **Hors périmètre** : les bitmaps par tranche (index de plage), écarté par ADR-0046 jusqu'à la mesure de P9 ; le parcours de la colonne lui-même (accès épars d'un domaine creux) ; le changement de `N` ou de `exact_threshold`.
```

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| (b) maintenant : ouvrir un ADR d'index de plage en v1, avec des bitmaps par année pour les dates et par tranche pour les entiers (MES-14 § 7, recommandation 2) | Le gain n'est pas mesuré : il se déduit seulement du coût d'une clause en bitmap (51 à 210 µs), sans campagne sur une implémentation. Les fixtures de MES-14 suivent la distribution de `synth-small`, pas celle de Legifacile. L'index coûterait de la mémoire, du temps de démarrage (déjà 20 s à 8,58 M d'après MES-13) et de la maintenance en P3. Le dépassement mesuré tient dans le budget de bout en bout, donc rien ne presse avant une mesure sur l'index réel. |
| (a) + (c) seuls, sans tâche d'insertion | Laisse en place un coût identifié comme évitable, environ 70 ms des 194,4 ms du pire cas d'intervalle (calculé, pas mesuré). La tâche est bornée, sans format nouveau ni dépendance, et son critère d'égalité des bitmaps rend le risque de régression faible. Sans elle, la part d'insertion resterait une interprétation. |
| Ne rien changer : garder « Filtre (bitmaps) < 1 ms » pour tous les filtres | Le budget serait faux pour les filtres d'intervalle, de 100 à 200 fois à 8,58 M passages. Une revue ou une fiche pourrait le supposer tenu et bâtir un plan sur une cible inatteignable avec la structure d'ADR-0018. |
| Fixer une cible distincte pour les filtres d'intervalle, par exemple 50 ms | Aucune référence ne la fonde, et le propriétaire a posé que le rappel prime et que la latence est indicative. Une cible inventée créerait un critère de jalon que rien ne justifie. |

## Références

- `docs/mesures/MES-14-cout-filtres-intervalles.md`, § 4.2, § 5 et § 7.
- `docs/03-recherche.md`, « Filtres → pool » et « Budget de latence indicatif ».
- `docs/08-invariants-et-tests.md` : INV-07, MES-14 (P1, P4), MES-02, MES-05, MES-13.
- ADR-0018, filtres d'intervalle par colonnes de valeurs en RAM (déclencheur conservé).
- `docs/suivi.md`, « Cibles » (p95 ≤ 698 ms) et « Décisions en attente » (D1).
- `docs/phases/P4-recherche-rapide.md` (`MESURER-LATENCE-DENSE`) ; `docs/phases/P9-migration-legifacile.md` (`COMPARER-AVEC-PGVECTOR`).
- `docs/mesures/MES-13-temps-de-demarrage.md` (démarrage à 8,58 M passages).
