# 03 — Recherche

## Le pipeline d'une requête

| # | Étape | Ce qu'elle fait | Clé de temps (debug + analytics) |
|---|---|---|---|
| 1 | Validation | Analyse la requête, valide le filtre contre le schéma | `validate` |
| 2 | Encodage | Encode la requête en ONNX INT8, avec un cache LRU en RAM | `encode` |
| 3 | Pool | Calcule filtre ∧ vivants, sous forme de bitmap | `filter` |
| 4 | Branche dense | Choisit une stratégie et sort ses candidats (voir plus bas) | `dense` |
| 5 | Branche lexicale | BM25 tantivy limité au pool, en parallèle de l'étape 4 | `lexical` |
| 6 | Fusion | RRF ou `relative_score` (mode `hybrid`) | `fusion` |
| 7 | Regroupement | `group_by` et `dedup` | `group` |
| 8 | Payload | Lit les champs et le texte des k résultats dans SQLite ; si la ligne `documents` est absente (document supprimé depuis le snapshot), rend le texte et les champs de passage seuls, sans erreur (ADR-0043) | `fetch` |
| — | Total | — | `total` |

Les étapes ne s'exécutent que si le mode les demande : `dense`, `lexical` ou `hybrid`. Le cache d'encodage reste en RAM et n'est jamais persisté.

## Filtres → pool

La grammaire est décrite dans `05-api.md`. Côté structures de données :

| Type de champ | Structure en RAM | Opérateurs |
|---|---|---|
| `keyword` | une bitmap par valeur + une bitmap « présent » | `eq`, `ne`, `in`, `nin`, `exists` |
| `bool` | deux bitmaps + « présent » | `eq`, `exists` |
| `integer`, `float`, `date` | une colonne de valeurs indexée par iid (`i64`, `f64`, `i32` en jours), avec une sentinelle pour null, + une bitmap « présent » | `eq`, `ne`, `lt`, `lte`, `gt`, `gte`, `in`, `exists` |

Évaluation :
- **Nœud ET** : on évalue d'abord les clauses en bitmap, de la plus sélective à la moins sélective. Les clauses d'intervalle se calculent ensuite en ne parcourant que le résultat intermédiaire.
- **Nœud OU** : union des enfants. **NON** : vivants privés de l'enfant.
- **Valeurs nulles** (ADR-0035) : les comparaisons ne sont jamais satisfaites par un null. `ne` et `nin` se calculent en `présent ∩ ¬bitmap(...)`, jamais comme le complément sur les vivants. `exists: true` vaut « présent et non null » ; `exists: false` vaut « absent ou null ». **NON** reste le complément sur les vivants, nulls compris.
- **Résultat final** : on fait l'intersection avec les vivants, et on obtient la taille exacte du pool.
- **Niveau des champs** : tous les champs sont matérialisés au niveau du passage ; un passage hérite des champs de son document.
- **Coût** : une clause d'intervalle sans autre clause parcourt toute la colonne (MES-14).

## Branche dense

Les vecteurs sont normalisés, donc **score = produit scalaire = cosinus**.

Choix de la stratégie :

| Condition | Stratégie |
|---|---|
| taille du pool ≤ `exact_threshold` | **EXACT** : produit scalaire fp16 sur tout le pool |
| sinon | **BINARY** : scan de Hamming sur le pool → top-N → rescoring fp16 → tri |
| `exact: true` dans la requête | **EXACT** sur tout le pool, quelle que soit sa taille (référence pour l'éval et le debug) |

Détails :
- **Code binaire** : le bit i vaut 1 si x_i > 0 (ADR-0004). Les requêtes sont codées de la même façon. La distance de Hamming est le popcount du XOR sur des mots `u64`. Un centrage préalable est à étudier (MES-12).
- **Parallélisme** : le pool est découpé en tranches contiguës d'iid, une par thread rayon. Chaque thread garde son propre tas, puis les tas sont fusionnés.
- **Sortie** : les `candidates_per_branch` meilleurs, avec leur cosinus. Ce nombre est augmenté quand il faut regrouper (voir plus bas).
- **Rangement physique** : à la compaction, les passages sont triés selon `cluster_by`. Un filtre sur ce champ parcourt alors une zone mémoire contiguë.

Réglages de précision :

| Réglage | Statut | Défaut |
|---|---|---|
| `reference_precision` (`fp16` ou `fp32`) | figé à la création | `fp16` |
| `search.scan` (`binary` ou `none` = toujours exact) | réglable | `binary` |
| `search.rescore_candidates` (N) | réglable | 1000, à confirmer par MES-01 |
| `search.exact_threshold` | réglable | égal à N |
| `query_precision` (`[encoder].query_precision` : `int8` ou `fp32`) | réglable | `int8` (ADR-0051) |

`query_precision` règle la précision d'encodage des requêtes. `fp32` charge la session fp32 de l'encodeur, qui coûte plus d'1 Go de RAM : c'est un réglage d'évaluation et de diagnostic, pas un mode de production (ADR-0051). Le cache LRU des requêtes est borné par `[limits].query_cache_entries`, 1 024 entrées par instance, et n'est jamais persisté.

Si le rappel du binaire est insuffisant, plusieurs leviers, dans cet ordre :
1. augmenter N ;
2. passer au scoring asymétrique (requête en flottants contre documents en ±1) ;
3. scanner en int8 (hors v1).

## Branche lexicale (tantivy)

- **Un index tantivy par époque.** Chaque document tantivy est un passage, avec deux champs : `iid` (`u64`, en fast field et indexé) et `text` (analysé, non stocké).
- **Le texte indexé est l'entrée encodeur**, c'est-à-dire exactement le texte de la branche dense. Il contient donc le gabarit, par exemple « Code civil, article 1240 ».
- **Chaîne d'analyse**, choisie par index avec `lexical.language` :
  - `SimpleTokenizer`, puis `RemoveLongFilter(40)`, `LowerCaser` et `AsciiFoldingFilter` ;
  - `StopWordFilter(langue)`, puis `Stemmer(langue)`.

  Le traitement des élisions (l', d', qu') et des identifiants comme `L1132-1` est à mesurer (MES-11).
- **Positions désactivées par défaut** (`lexical.positions`). Les requêtes de phrase viendront plus tard.
- **Requête** : la requête passe par la même analyse et forme un sac de termes (OU), noté en BM25.
  - Le collecteur lit le fast field `iid` et ne garde un document que s'il est dans la bitmap du pool. Il remplit un tas de taille k.
  - Avec ce collecteur, l'élagage block-WAND ne s'applique probablement pas (à mesurer, MES-06).
- **Suppressions** : elles sont aussi faites dans tantivy, mais la bitmap des vivants reste l'autorité.
- **Reconstruction** depuis le texte SQLite, dans trois cas : compaction, reprise après incident, changement d'analyseur (le changement de réglage lance un job de reconstruction).

## Fusion (mode `hybrid`)

- **Les deux branches utilisent le même pool** (INV-05). Chacune sort ses `candidates_per_branch` meilleurs résultats.
- **`rrf`** : score = Σ 1 / (60 + rang), avec un rang qui commence à 1.
- **`relative_score`** :
  - chaque branche est normalisée min-max sur ses propres candidats ;
  - score fusionné = α·dense + (1−α)·lexical ;
  - un document absent d'une branche y vaut 0.
- **Défaut par index** : `rrf`, qui n'a rien à régler. On peut le changer par requête, et choisir le réglage par défaut avec MES-10.
- **Pour le debug**, on conserve le rang et le score de chaque résultat dans chaque branche.
- **`score_type`** vaut `cosine`, `bm25`, `rrf` ou `relative`. Un score ne se compare pas d'une requête à l'autre.

## Regroupement

- **`group_by: "document"`** : on garde le meilleur passage de chaque document. `passages_per_document` (1 par défaut, 5 au maximum) ajoute d'autres passages du même document qui ont aussi matché.
- **`dedup: "content"`** : les passages qui ont le même `passage_hash` sont fusionnés, et on garde le meilleur.
- **Sur-échantillonnage** :
  - quand on regroupe, `candidates_per_branch = clamp(k × 20, 200, 1000)` (la règle actuelle de Legifacile) ;
  - sinon, `max(k, 100)`.
- **Ordre** : le regroupement s'applique après la fusion et avant la lecture des payloads.

## Recherche multi-index

- **Requête** : une liste `targets: [{index, filter}]`, avec un filtre par index, puisque chaque index a son propre schéma.
- **Modèle commun** : tous les index d'une recherche dense ou hybride doivent partager la même empreinte de modèle. Sinon, erreur `incompatible_model`.
- **Encodage unique** : la requête n'est encodée qu'une fois.
- **Par index** : pool, branches, fusion et regroupement s'exécutent séparément, puis on garde le top-k de chaque index.
- **Fusion entre index** :
  - **mode `dense`** : par cosinus. C'est exact, car les scores sont comparables (déjà validé dans Legifacile, `serveur_mcp/perimetres.py`) ;
  - **modes `lexical` et `hybrid`** : RRF sur les rangs de chaque index.
- **Résultat** : chaque résultat porte son `index`, et le debug donne son rang dans son index.

## Rerank (hors v1, à garder en tête)

Le rerank viendra comme une étape optionnelle après la fusion et le regroupement :
- les textes sont déjà stockés, donc disponibles ;
- il tourne sur un exécuteur distant (GPU), avec un timeout ;
- en cas d'échec, on renvoie l'ordre fusionné, marqué `reranked: false`, et les analytics comptent ces replis.

**On ne l'implémente pas en v1**, mais la liste des étapes doit permettre de l'insérer sans refonte.

## Budget de latence indicatif

À remplacer par les mesures. Échelle Legifacile, Mac mini.

| Étape | Estimation | Mesure |
|---|---|---|
| Encodage de la requête (CPU, INT8) | 20 à 50 ms | MES-05 |
| Filtre (bitmaps) | < 1 ms | MES-14 |
| Filtre d'intervalle (colonne de valeurs) | 101 à 194 ms à domaine plein, 9,1 à 49,2 ms sous un domaine réduit, à 8,58 M passages ; 1 ms ≈ 40 000 à 110 000 iid lus (ADR-0046) | MES-14 |
| Scan binaire : 8,6 M / pool de 1 M | 20 à 80 ms / quelques ms | MES-02 |
| Rescoring de N = 1000 | < 1 ms à chaud, ~10 ms à froid | MES-03 |
| Branche lexicale | à mesurer | MES-06 |
| Payload (k = 10) | ~1 ms | — |

ADR-0046 sépare les deux lignes : la valeur d'intervalle est **mesurée, pas une cible**, et le dépassement est assumé dans les budgets de bout en bout.
