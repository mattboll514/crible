# 03 — Recherche

## Le pipeline d'une requête

| # | Étape | Ce qu'elle fait | Clé de temps (debug + analytics) |
|---|---|---|---|
| 1 | Validation | Analyse la requête, valide le filtre contre le schéma | `validate` |
| 2 | Encodage | Encode la requête en ONNX fp32, avec un cache LRU en RAM | `encode` |
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
- **Code binaire** : le bit i vaut 1 si x_i > 0 (ADR-0004). Les requêtes sont codées de la même façon. La distance de Hamming est le popcount du XOR sur des mots `u64`. Un centrage préalable est à étudier (MES-12) ; son protocole de mesure — variante principale, contrôle du harnais, absence de changement du format — est fixé par ADR-0076, et son adoption par défaut demanderait un ADR qui remplace ADR-0069, décision 6.
- **Parallélisme** : le pool est découpé en tranches contiguës d'iid, une par thread rayon. Chaque thread garde son propre tas, puis les tas sont fusionnés.
- **Parcours du scan** : le scan binaire n'a qu'un parcours, **clairsemé**, sur l'itérateur trié de la bitmap du pool (ADR-0081) ; la mise en œuvre est faite (`UNIFIER-PARCOURS-SCAN`, fusionnée en `1b1f283`) et le curseur de segment d'ADR-0082 supprime la recherche binaire par iid quand la table des segments compte plusieurs entrées.
- **Sortie** : les `candidates_per_branch` meilleurs, avec leur cosinus. Ce nombre est augmenté quand il faut regrouper (voir plus bas) ; la règle est calculée par la couche de recherche partagée, pas par la CLI (ADR-0075).
- **Nom de la stratégie** : elle vaut `"exact"` ou `"binary"` dans le plan de debug (ADR-0074). `search.scan: none` donne `"exact"`, comme `exact: true` et comme un pool de taille inférieure ou égale au seuil.
- **Rangement physique** : à la compaction, les passages sont triés selon `cluster_by`. Un filtre sur ce champ parcourt alors une zone mémoire contiguë.

Réglages de précision :

| Réglage | Statut | Défaut |
|---|---|---|
| `reference_precision` (`fp16` ou `fp32`) | figé à la création | `fp16` |
| `search.scan` (`binary` ou `none` = toujours exact) | réglable | `binary` |
| `search.rescore_candidates` (N) | réglable | `5000` par défaut (ADR-0078) |
| `search.exact_threshold` | réglable | égal à N à la création de l'index, puis **matérialisé** (ADR-0074) |
| `query_precision` (`[encoder].query_precision` : `int8` ou `fp32`) | réglable | `fp32` (ADR-0052) |

`exact_threshold` et `rescore_candidates` sont deux leviers distincts (ADR-0074). Le seuil accepte tout entier de `0` à `u32::MAX`, sans autre borne et sans nouvelle erreur : `0` ne fait passer par EXACT qu'un pool vide, sauf `exact: true` ; `u32::MAX` fait toujours passer par EXACT. Un pool de taille **égale** au seuil passe par EXACT. La clé est **optionnelle** à la création de l'index : absente, elle prend la valeur de `rescore_candidates` à ce moment, et cette valeur est écrite dans la configuration stockée, puis relue comme une valeur explicite ; un `PATCH` de `rescore_candidates` ne la déplace pas.

Sous BINARY, quand `k` (le nombre de résultats demandés par la branche dense, soit `candidates_per_branch` quand la requête regroupe ; ADR-0074, ADR-0075) dépasse N, le scan garde `max(N, k)` candidats, tous rescorés, et le plan expose ce nombre effectif. Aucune erreur n'est rendue et il n'y a pas de repli sur EXACT.

`query_precision` règle la précision d'encodage des requêtes. Le défaut est `fp32` : la session fp32 (plus d'1 Gio) est résidente dès la première requête, et la chaîne d'encodage est fidèle au modèle PyTorch à 1e-12 près (MES-04). `int8` reste une valeur acceptée, pour les comparaisons (MES-04), le repli et le diagnostic ; l'INT8 ne conserve que 79,2 % du top-10 exact du fp32, il n'est donc pas un mode de production (ADR-0052). Le cache LRU des requêtes est borné par `[limits].query_cache_entries`, 1 024 entrées par instance, et n'est jamais persisté.

Si le rappel du binaire est insuffisant, plusieurs leviers, dans cet ordre :
1. augmenter N ;
2. passer au scoring asymétrique (requête en flottants contre documents en ±1) ;
3. scanner en int8 (hors v1).

## Branche lexicale (tantivy)

- **Un index tantivy par époque.** Chaque document tantivy est un passage, avec deux champs : `iid` (`u64`, en fast field et indexé) et `text` (analysé, non stocké).
- **Le texte indexé est l'entrée encodeur**, c'est-à-dire exactement le texte de la branche dense. Il contient donc le gabarit, par exemple « Code civil, article 1240 ».
- **Chaîne d'analyse**, choisie par index avec `lexical.language` :
  - `lexical.language` prend ses valeurs dans une table figée de dix-huit codes ISO 639-1, en **minuscules**, sans normalisation : `ar`, `da`, `de`, `el`, `en`, `es`, `fi`, `fr`, `hu`, `it`, `nl`, `no`, `pt`, `ro`, `ru`, `sv`, `ta`, `tr` (table d'ADR-0086, avec le stemmer et la liste de mots vides de chaque langue). Toute autre valeur est refusée en `invalid_request` (400) à la validation de la configuration ;
  - `SimpleTokenizer`, puis `RemoveLongFilter(40)`, `LowerCaser` et `AsciiFoldingFilter` ;
  - `StopWordFilter(langue)`, puis `Stemmer(langue)`. `StopWordFilter` est **absent** pour `ar`, `el`, `ro`, `ta` et `tr`, les cinq langues sans liste de mots vides : ces langues reçoivent les mêmes jetons qu'une chaîne sans filtre de mots vides (ADR-0086).

  Le traitement des élisions (l', d', qu') et des identifiants comme `L1132-1` est à mesurer (MES-11).
- **Positions désactivées par défaut** (`lexical.positions`). Les requêtes de phrase viendront plus tard.
- **Requête** : la requête passe par la même analyse et forme un sac de termes (OU), noté en BM25.
  - Le collecteur lit le fast field `iid` et ne garde un document que s'il est dans la bitmap du pool. Il remplit un tas de taille k.
  - Avec ce collecteur, l'élagage block-WAND ne s'applique probablement pas (à mesurer, MES-06).
- **Suppressions** : elles sont aussi faites dans tantivy, mais la bitmap des vivants reste l'autorité.
- **Reconstruction** depuis le texte SQLite, dans trois cas : compaction, reprise après incident, changement d'analyseur (le changement de réglage lance un job de reconstruction).

## Fusion (mode `hybrid`)

- **Les deux branches utilisent le même pool** (INV-05). Chacune sort ses `candidates_per_branch` meilleurs résultats, calculés par la même règle partagée (ADR-0075).
- **`rrf`** : score = Σ 1 / (60 + rang), avec un rang qui commence à 1.
- **`relative_score`** :
  - chaque branche est normalisée min-max sur ses propres candidats ;
  - score fusionné = α·dense + (1−α)·lexical ;
  - un document absent d'une branche y vaut 0.
- **Défaut par index** : `rrf`, qui n'a rien à régler. On peut le changer par requête, et choisir le réglage par défaut avec MES-10.
- **Pour le debug**, on conserve le rang et le score de chaque résultat dans chaque branche.
- **`score_type`** vaut `cosine`, `bm25`, `rrf` ou `relative`. Un score ne se compare pas d'une requête à l'autre.

## Regroupement

- **Ordre** (ADR-0075) : `dedup: "content"` d'abord, puis `group_by: "document"`, après la fusion et avant la lecture des payloads, sur la liste des candidats sur-échantillonnés. La fusion retire les copies une fois, et le regroupement se recomplète depuis les candidats restants ; l'ordre inverse supprimerait le représentant d'un document. Quand les candidats sont épuisés avant `k` documents, la réponse en contient moins, sans erreur.
- **`group_by: "document"`** : on garde le meilleur passage de chaque document, et `passages_per_document` (1 par défaut, 5 au maximum) ajoute d'autres passages du même document qui ont aussi matché. `k` compte alors des documents : la réponse reste une liste plate, avec au plus `k` documents distincts et au plus `k × passages_per_document` entrées, les entrées d'un document étant consécutives et les documents rangés par leur meilleur passage (ADR-0075).
- **`dedup: "content"`** : les passages qui ont le même `passage_hash` sont fusionnés, et on garde le meilleur selon l'ordre de tri en place (score décroissant, puis clé croissante).
- **Sur-échantillonnage** : la règle est calculée dans la couche de recherche partagée, là où se construit la planification ; la CLI et le serveur ne passent que `k` et les options de la requête.
  - quand on regroupe, `candidates_per_branch = clamp(k × 20, 200, 1000)` (la règle actuelle de Legifacile) ;
  - sinon, `max(k, 100)`.

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
| Encodage de la requête (CPU, fp32) | 20 à 50 ms (estimé pour l'INT8 ; fp32 : p50 21 ms, p95 ≈ 56 ms hors conteneur, dépassement du p95 assumé, ADR-0052) | MES-05 |
| Filtre (bitmaps) | < 1 ms | MES-14 |
| Filtre d'intervalle (colonne de valeurs) | 101 à 194 ms à domaine plein, 9,1 à 49,2 ms sous un domaine réduit, à 8,58 M passages ; 1 ms ≈ 40 000 à 110 000 iid lus (ADR-0046) | MES-14 |
| Scan binaire, index entier (7,74 M vivants) | 20 à 80 ms ; mesuré au défaut N = 5000 avec le binaire actuel : 36,6 à 37,3 ms p50 et 40,1 à 41,6 ms p95 | MES-02 (A/B du 2026-10-27, ADR-0082) |
| Scan binaire, pool de 1 M contigu | quelques ms ; mesuré : 7,10 à 7,27 ms p50 et 8,34 à 8,67 ms p95 | MES-02 (A/B du 2026-10-27, ADR-0082) |
| Scan binaire, pool de 1 M dispersé (ce qu'un filtre produit, sauf sur le champ `cluster_by`) | quelques ms ; mesuré : 11,8 ms p50 et 14,1 ms p95 à N = 1000, 14,2 ms p50 à N = 5000 | MES-02 (campagne du 2026-10-27) |
| Rescoring, N = 1000 | À chaud : 0,73 à 0,79 ms p50 et 12,0 à 17,3 ms p95. À froid partiel (pages déplacées après l'ouverture, résidence moyenne 0,51 à 0,59) : 20,3 à 24,5 ms p50. Pour mémoire, 85,0 ms p50 pour le produit scalaire seul, **seule référence à froid complet**, mesurée par un harnais `F_NOCACHE`, jamais par le moteur (ADR-0084) | MES-03 |
| Rescoring, N = 5000 (défaut, ADR-0078) | À chaud : 2,07 à 2,17 ms p50 et 17,6 à 28,7 ms p95. À froid partiel (résidence moyenne 0,78 à 0,80) : 29,2 à 33,0 ms p50. Pour mémoire, 177,5 ms p50 pour le produit scalaire seul, **seule référence à froid complet**, mesurée par un harnais `F_NOCACHE`, jamais par le moteur (ADR-0084) | MES-03 |
| Branche lexicale | à mesurer | MES-06 |
| Payload (k = 10) | ~1 ms | — |

Ces chiffres du scan sont **natifs**, mesurés sur une machine de développement chargée, et le pool dispersé coûte environ deux fois le pool contigu. La lecture de « quelques ms » et son percentile sont jugés en P8, en conteneur (ADR-0081, ADR-0082 décision 5) ; un filtre sur le champ `cluster_by` rend son pool contigu (voir « Rangement physique »).

ADR-0046 sépare les deux lignes : la valeur d'intervalle est **mesurée, pas une cible**, et le dépassement est assumé dans les budgets de bout en bout.

Les deux lignes de rescoring remplacent le budget indicatif « < 1 ms à chaud, ~10 ms à froid » par les mesures de MES-03 sur la variante livrée, `grouped` (ADR-0080), à N = 1000 et au défaut N = 5000 (ADR-0078). Le dépassement est **assumé au profit de la fidélité** : à N = 5000 à chaud, 2,1 ms de p50 dépasse encore « < 1 ms ». La cible réelle est la latence de `08-invariants-et-tests.md`, p95 au plus égal à pgvector, mesurée en P8 et P9. Les chiffres à froid partiel ne jugent aucun budget : ils portent leur résidence de pages, et le froid vrai est renvoyé à P8 (ADR-0084). Le coût des clés au rescoring, distinct des produits scalaires, relève d'un ADR séparé.

Le protocole des deux lignes de la recherche dense (scan binaire et rescoring) est fixé par ADR-0077, accepté par anticipation le 2026-10-27 : N ∈ {1000, 2000, 5000}, pools construits à 1 k, 10 k, 100 k, 1 M et 7,74 M passages vivants, contigus et dispersés, et froid mesuré par copies non cachées pour le rescoring seul. Les trois lignes du scan binaire ne sont pas touchées par ADR-0078 : leur protocole est celui d'ADR-0077 et leurs chiffres ceux de MES-02, vérifiés en conteneur en P8.

Repères de lecture : la séparation de la ligne du scan le 2026-10-28 place le scan en `:144-146` et le rescoring en `:147-148`. ADR-0078 (`:144`) et ADR-0084 (`:145-146`) citent la numérotation antérieure.
