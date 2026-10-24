# P4 — Recherche rapide

**But** : passer d'un calcul exact partout au **scan binaire suivi d'un rescoring fp16**, choisir la stratégie selon la taille du pool, puis regrouper par document. On garantit la fidélité par la mesure.

**Jalon J4** :
- MES-01 atteint la cible de fidélité (recall@10 contre l'exact, par défaut au moins 0,99) ;
- MES-02, MES-03, MES-09 et MES-12 sont consignées ;
- le N par défaut est choisi et validé (Q-01) ;
- INV-06 est toujours vert ;
- `group_by` et `dedup` fonctionnent.

---

### QUANTIFIER-BINAIRE
- **Objectif** : les codes binaires de chaque segment.
- **Dépend de** : ECRIRE-SEGMENTS-VECTEURS, ATTACHER-LOT, COMPACTER-INDEX
- **À lire** : `03-recherche.md` (branche dense) ; `02-architecture.md` (mémoire).
- **Livrables** :
  - l'écriture de `codes.bin` (bit i = x_i > 0, mots `u64`, little-endian) à l'attache et à la compaction ;
  - un job de reconstruction des codes depuis les vecteurs de référence, pour les index existants ;
  - le chargement des codes en RAM (pas en mmap), dans le `Snapshot`.
- **Critères d'acceptation** :
  - test sur des valeurs connues ;
  - les codes reconstruits sont identiques à ceux écrits à l'attache ;
  - `crible inspect` affiche la mémoire occupée par les codes.
- **Invariants** : INV-15.

### NOYAUX-SIMD
- **Objectif** : les calculs de base, rapides et sûrs.
- **Dépend de** : —
- **À lire** : `07-stack-et-deploiement.md` (noyaux SIMD).
- **Livrables** :
  - `crible-search::kernels` : distance de Hamming (popcount sur des `u64`) et produit scalaire fp16 × f32 ;
  - des implémentations NEON (aarch64) et AVX2 (x86_64), avec un repli scalaire et une détection à l'exécution ;
  - des benchmarks `criterion` ;
  - le choix entre une crate dédiée et le code maison, justifié dans le rapport.
- **Critères d'acceptation** :
  - un proptest vérifie que chaque noyau SIMD donne le même résultat que le scalaire (égalité exacte pour Hamming, 1e-5 pour le produit scalaire) ;
  - chaque `unsafe` est accompagné d'un commentaire `SAFETY` ;
  - les benchmarks sont consignés.
- **Hors périmètre** : int8, l'asymétrique.

### SCANNER-POOL-BINAIRE
- **Objectif** : obtenir le top-N par distance de Hamming sur le pool.
- **Dépend de** : QUANTIFIER-BINAIRE, NOYAUX-SIMD, EVALUER-FILTRES
- **À lire** : `03-recherche.md` (branche dense : parallélisme).
- **Livrables** : un parcours du pool par tranches d'iid sur le pool rayon, avec un tas par thread, puis la fusion des tas. Si le pool est dense, on itère sur les codes contigus en testant la bitmap. S'il est clairsemé, on itère sur la bitmap.
- **Critères d'acceptation** :
  - le résultat est égal à un tri complet par Hamming sur `synth-small` ;
  - les égalités de score sont départagées de façon déterministe (par iid).

### RESCORER-FP16
- **Objectif** : le score exact des N candidats.
- **Dépend de** : SCANNER-POOL-BINAIRE
- **Livrables** : la lecture des vecteurs de référence des N candidats (mmap), le produit scalaire via les noyaux, puis le tri final.
- **Critères d'acceptation** : quand N est au moins égal à la taille du pool, le résultat est identique à la stratégie EXACT.

### PLANIFIER-STRATEGIE
- **Objectif** : choisir EXACT ou BINARY, et forcer l'exact à la demande.
- **Dépend de** : RESCORER-FP16
- **À lire** : `03-recherche.md` (tableau des stratégies) ; `04-donnees-et-ingestion.md` (réglages).
- **Livrables** :
  - la sélection selon `exact_threshold` et `search.scan` ;
  - l'option `exact` ;
  - la stratégie choisie, exposée dans le plan (pour le debug, P7) ;
  - le réglage par `PATCH` au niveau de la bibliothèque.
- **Critères d'acceptation** :
  - **INV-06** : un pool sous le seuil donne un résultat identique à l'exact ;
  - les 200 requêtes de `synth-small` donnent les mêmes résultats avec `exact: true` qu'en P1.
- **Invariants** : INV-06.

### REGROUPER-RESULTATS
- **Objectif** : `group_by: document`, `passages_per_document` et `dedup: content`.
- **Dépend de** : PLANIFIER-STRATEGIE
- **À lire** : `03-recherche.md` (regroupement).
- **Livrables** : le sur-échantillonnage (`clamp(k × 20, 200, 1000)` quand on regroupe), le regroupement, et la fusion des passages identiques par `passage_hash`.
- **Critères d'acceptation** :
  - on obtient k documents distincts ;
  - le meilleur passage de chaque document est conservé ;
  - les doublons de contenu sont fusionnés ;
  - des tests couvrent un document qui a plusieurs passages dans le top.

### MESURER-FIDELITE-BINAIRE
- **Objectif** : MES-01 et MES-12, puis le choix de N.
- **Dépend de** : PLANIFIER-STRATEGIE, CONSTITUER-ECHANTILLON-LEGIFACILE
- **À lire** : `08-invariants-et-tests.md` (cibles) ; `docs/mesures/README.md`.
- **Livrables** : le recall@10 et le recouvrement@10 de BINARY contre EXACT sur les requêtes réelles de l'échantillon, pour N ∈ {200, 500, 1000, 2000, 5000}, avec et sans centrage, et par structure d'index.
- **Critères d'acceptation** :
  - le rapport contient une recommandation de N et de centrage ;
  - la cible est atteinte, ou bien le rapport propose un levier (scoring asymétrique) à faire valider.
- **Invariants / mesures** : MES-01, MES-12.

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

### MESURER-LATENCE-DENSE
- **Objectif** : MES-02, MES-03 et MES-09.
- **Dépend de** : PLANIFIER-STRATEGIE
- **Livrables** :
  - sur `synth-large` (8,6 M passages) : la latence p50/p95 du scan selon la taille du pool (1 k, 10 k, 100 k, 1 M, 8,6 M) et le rescoring à froid et à chaud ;
  - la RAM réelle (RSS), en natif sur le Mac.
- **Critères d'acceptation** : les protocoles sont reproductibles et les chiffres sont comparés au budget de `03-recherche.md`.
- **Invariants / mesures** : MES-02, MES-03, MES-09.
