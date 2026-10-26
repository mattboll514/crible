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
- **Précisé par ADR-0069** (accepté par anticipation le 2026-10-26) : le « job de reconstruction » est l'ouverture du segment par l'écrivain ; `codes.bin` absent y est reconstruit depuis les `vectors.*` du segment et persisté atomiquement, sans nouvelle génération ni nouvelle époque. En lecture seule, les codes manquants sont construits en RAM et jamais écrits dans l'époque. Un fichier absent n'est pas une erreur ; un fichier de longueur fausse en est une (`Internal`, fichier nommé). Le format n'a pas d'en-tête : taille `count × ceil(dims/64) × 8` octets, version portée par `format_version` de `segment.json`, incrémenté à tout changement de disposition, et `codes.bin` reste hors des `checksums`. Aucune sous-commande CLI n'est ajoutée en P4.

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

### ACCELERER-RECETTE
- **Ajoutée par** : ADR-0067, hors plan. Elle vient de la tâche `MESURER-CYCLE-DE-VIE` et du verdict `NON-COUVERT` de `@architecte` du 2026-10-26.
- **Objectif** : établir où passent les 44,2 s fixes de l'étape `validation` d'une attache, puis ramener la recette sous ce qu'elle coûte aujourd'hui sans changer ce qu'elle garantit. Avec `mdenseon`, `validation` vaut 51,5 s pour 1 000 passages et 65,9 s pour 3 002 sur l'échantillon Legifacile ; la part fixe de 44,2 s est déduite d'un ajustement sur deux points, et MES-08 ne dit rien de sa répartition interne. Rien ne change au contrat : `acceptance_sample` 200, fp32, `acceptance_min_cosine` 0,999, défauts d'ONNX Runtime, recette dans le chemin de l'attache avant toute écriture (ADR-0052, ADR-0054, ADR-0067, INV-12).
- **Dépend de** : `MESURER-CYCLE-DE-VIE` (`FAIT`), qui a mesuré MES-08 et produit les chiffres du 2026-10-25.
- **À lire** : `adr/0067-duree-de-la-recette-a-l-attache-et-echeance-de-wait-true.md` (décisions 1 et 3, conséquences, documents à appliquer) ; `docs/mesures/MES-08-duree-attache-quotidienne.md`, campagne du 2026-10-25 (§ « Résultats », § « Extrapolation à l'échelle de Legifacile », § « Conclusion ») et campagne du 2026-10-26 (§ « Conclusion », qui rappelle que la recette sur l'échantillon n'a pas été rejouée) ; `docs/04-donnees-et-ingestion.md`, contrôle 9 (`:192-202`) ; `docs/02-architecture.md`, étapes de l'attache et sessions de l'encodeur gardées par le serveur (`:218`, ADR-0051) ; `docs/07-stack-et-deploiement.md`, `[limits]` et absence de réglage de session (`:95-103`). Code : `crates/crible-store/src/attach.rs` (`validation` et appel de `check_acceptance`, `:458` et `:482-484`, `check_acceptance`, `:748-800`, boucle de ré-encodage, `:778-783`, `plan_documents`, `:496-510`), `crates/crible-encode/src/model.rs` (forme `[1, length]`, `:257-263`, session sans réglage, `:155-157`, `count_documents`, `:473-478`), `crates/crible/src/commands.rs` (chargeur du fp32 par commande, `:146` et `:189`). ADR liés : ADR-0052 (fp32, seuil 0,999, défauts d'ONNX Runtime), ADR-0053 et ADR-0054 (chargeur différé et bornes de la recette), ADR-0051 (sessions gardées).
- **Livrables** :
  - (a) une section **datée** de MES-08 qui **ventile** l'étape `validation` sur l'échantillon (index `articles`, lots de 1 000 et 3 002 passages, modèle `mdenseon`) : chargement du modèle, 200 ré-encodages, comptage des jetons, passes du lot, `plan_documents`. La ventilation est mesurée, pas déduite : chaque poste est chronométré ou isolé par une exécution qui le retire.
  - (b) **seulement si** le ré-encodage représente plus de la moitié de `validation` : l'encodage de la recette par **lots triés par longueur** (remplissage et masque d'attention), dans `crible-encode`, avec une taille de lot interne fixée par la mesure de cette tâche, **non configurable**, et notée dans le rapport. Sinon, la tâche s'arrête au rapport et propose la suite au propriétaire : elle ne code rien.
  - (c) le rejeu de MES-08, comparé à la campagne du 2026-10-25.
  - Les résultats des contrôles remis à l'orchestrateur pour `docs/suivi.md`, qui reste tenu par lui.
- **Critères d'acceptation** :
  - la ventilation est mesurée et reproductible : les postes de l'étape `validation` sont chiffrés pour les deux tailles de lot, et leur somme retrouve la durée de `validation` de la campagne du 2026-10-25 à la variabilité de la mesure près ;
  - si (b) est fait : un test prouve que le cosinus de **chaque** passage de la recette, encodé par lots, égale celui du même passage encodé seul à **1e-6** près ; et les statistiques de la recette d'un lot de référence (`sample`, `min_cosine`, `p1`, `p50`) sont inchangées à **1e-6** ;
  - si (b) n'est pas fait, la tâche rend le rapport de (a) et une proposition de suite ; elle ne modifie ni `acceptance_sample`, ni le seuil, ni la précision ;
  - rejeu de MES-08 avec l'écart à la campagne du 2026-10-25, en section datée, sans réécrire les chiffres de cette campagne ;
  - `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` et `cargo test` verts ; un build `--release` pour le rejeu de la mesure.
- **Invariants / mesures** : **INV-12** (le lot incompatible reste refusé : la recette garde son échantillon, sa précision et son seuil, et l'égalité à 1e-6 prouve que le ré-encodage par lots ne change pas le verdict) ; **INV-11** (le comptage des jetons ne change pas : la tâche le mesure, elle ne le modifie pas) ; MES-08.
- **Hors périmètre** : `acceptance_sample`, `acceptance_min_cosine`, la précision de référence ou de ré-encodage ; les réglages de threads ou d'optimisation d'ONNX Runtime (ADR-0052, à revoir en conteneur en P8) ; une recette asynchrone ou hors du chemin de l'attache ; le comptage des jetons, sauf proposition au propriétaire si la mesure le montre dominant ; l'encodage en ligne de documents ; toute modification de `docs/04` ou de `docs/05`.

### MESURER-LATENCE-DENSE
- **Objectif** : MES-02, MES-03 et MES-09.
- **Dépend de** : PLANIFIER-STRATEGIE
- **Livrables** :
  - sur `synth-large` (8,6 M passages) : la latence p50/p95 du scan selon la taille du pool (1 k, 10 k, 100 k, 1 M, 8,6 M) et le rescoring à froid et à chaud ;
  - la RAM réelle (RSS), en natif sur le Mac.
- **Critères d'acceptation** : les protocoles sont reproductibles et les chiffres sont comparés au budget de `03-recherche.md`.
- **Invariants / mesures** : MES-02, MES-03, MES-09.
