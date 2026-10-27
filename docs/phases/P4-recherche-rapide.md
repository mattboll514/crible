# P4 — Recherche rapide

**But** : passer d'un calcul exact partout au **scan binaire suivi d'un rescoring fp16**, choisir la stratégie selon la taille du pool, puis regrouper par document. On garantit la fidélité par la mesure.

**Jalon J4** :
- MES-01 atteint la cible de fidélité (recall@10 contre l'exact, par défaut au moins 0,99, en moyenne globale selon ADR-0076) ;
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
- **Précisé par ADR-0081** (accepté le 2026-10-27) : le livrable ci-dessus demande deux parcours selon la densité ; ADR-0081 les ramène à un seul, le parcours clairsemé sur l'itérateur trié de la bitmap du pool. La correction de la fiche est faite par la tâche `UNIFIER-PARCOURS-SCAN`, en P4 avant J4, après une mesure multi-segments ; la décision et non le livrable fait foi d'ici là.
- **Critères d'acceptation** :
  - le résultat est égal à un tri complet par Hamming sur `synth-small` ;
  - les égalités de score sont départagées de façon déterministe (par iid).

### RESCORER-FP16
- **Objectif** : le score exact des N candidats.
- **Dépend de** : SCANNER-POOL-BINAIRE
- **Livrables** : la lecture des vecteurs de référence des N candidats (mmap), le produit scalaire via les noyaux, puis le tri final.
- **Critères d'acceptation** : quand N est au moins égal à la taille du pool, le rescoring rend les mêmes clés que la stratégie EXACT, dans le même ordre (départage stable par clé `doc_id#ord`), avec des scores identiques au bit près ; les deux stratégies passent par la même fonction de calcul du produit scalaire (ADR-0072, décision 3). La comparaison à la référence numpy reste celle d'ADR-0045 (`1e-5`, quasi-égalités admises).

### PLANIFIER-STRATEGIE
- **Objectif** : choisir EXACT ou BINARY, et forcer l'exact à la demande.
- **Dépend de** : RESCORER-FP16
- **À lire** : `03-recherche.md` (tableau des stratégies) ; `04-donnees-et-ingestion.md` (réglages).
- **Livrables** :
  - la sélection selon `exact_threshold` et `search.scan` ;
  - l'option `exact` ;
  - la stratégie choisie, exposée dans le plan (pour le debug, P7 ; valeurs `"exact"` et `"binary"`, ADR-0074) ;
  - le réglage par `PATCH` au niveau de la bibliothèque.
- **Critères d'acceptation** :
  - **INV-06** : un pool sous le seuil donne un résultat identique à l'exact ;
  - les 200 requêtes de `synth-small` donnent les mêmes résultats avec `exact: true` qu'en P1.
- **Invariants** : INV-06.

### REGROUPER-RESULTATS
- **Objectif** : `group_by: document`, `passages_per_document` et `dedup: content`.
- **Dépend de** : PLANIFIER-STRATEGIE
- **À lire** : `03-recherche.md` (regroupement).
- **Livrables** : le sur-échantillonnage (`clamp(k × 20, 200, 1000)` quand on regroupe), le regroupement, et la fusion des passages identiques par `passage_hash`. **Précisé par ADR-0075** (accepté par anticipation le 2026-10-27) : `dedup` s'applique avant `group_by`, sur toute la liste sur-échantillonnée, et le regroupement se recomplète depuis les candidats restants ; la règle de sur-échantillonnage vit dans `crible-search`, la CLI ne passant que `k` et les options de la requête ; la réponse reste une liste plate d'entrées de passage, où `k` compte des documents et où les entrées d'un document sont consécutives ; sous BINARY, le scan garde `max(N, candidates_per_branch)` candidats.
- **Critères d'acceptation** :
  - on obtient k documents distincts, ou moins quand les candidats sur-échantillonnés sont épuisés, sans erreur ;
  - le meilleur passage de chaque document est conservé ;
  - les doublons de contenu sont fusionnés ;
  - des tests couvrent un document qui a plusieurs passages dans le top.

### MESURER-FIDELITE-BINAIRE
- **Objectif** : MES-01 et MES-12, puis le choix de N.
- **Dépend de** : PLANIFIER-STRATEGIE, CONSTITUER-ECHANTILLON-LEGIFACILE
- **À lire** : `08-invariants-et-tests.md` (cibles) ; `docs/mesures/README.md`.
- **Livrables** : le recall@10 et le recouvrement@10 de BINARY contre EXACT sur les requêtes réelles de l'échantillon, pour N ∈ {200, 500, 1000, 2000, 5000}, avec et sans centrage, et par structure d'index. **Précisé par ADR-0076** (accepté par anticipation le 2026-10-27) : recall@10 au niveau du passage et recouvrement@10 au niveau du document, cible de 0,99 en moyenne globale au N par défaut ; un tableau par structure plus la moyenne ; centrage mesuré hors moteur par un harnais contrôlé au bit près ; BINARY forcé par `search.exact_threshold = 0` sur des copies des index ; 300 requêtes fp32, campagne principale sans filtre et variante filtrée déclarée. **Précisé par ADR-0078** (2026-10-27) : le N par défaut retenu vaut 5000, seule valeur qui atteint la cible dans les deux campagnes et pour toutes les structures.
- **Critères d'acceptation** :
  - le rapport contient une recommandation de N et de centrage (rendue : N = 5000, sans centrage, ADR-0078) ;
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
- **Issue du livrable (b), note du 2026-10-27 (ADR-0073)** : il a été codé une fois la condition remplie (les 200 ré-encodages font 88,0 % et 89,0 % de `validation`), puis **mesuré et retiré**. Le temps de ré-encodage croît avec la taille de lot sur les deux lots (+3,6 % à +21,5 % dès B = 4), et huit copies du plus long passage coûtent 8,05 à 8,08 fois un appel seul : avec les défauts d'ONNX Runtime, les lignes d'un lot ne s'exécutent pas en parallèle. La recette reste donc encodée **un passage à la fois**, et aucune taille de lot ne subsiste dans le code. La mesure est consignée dans MES-08, campagne du 2026-10-27. Voir ADR-0073.
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
  - sur `synth-large` (8,6 M passages) : la latence p50/p95 du scan selon la taille du pool (1 k, 10 k, 100 k, 1 M, 8,6 M) et le rescoring à froid et à chaud. **Précisé par ADR-0077** (accepté par anticipation le 2026-10-27) : le point « tous les passages » vaut **7,74 M vivants** et non 8,6 M, le jeu portant 860 000 passages morts (MES-07) ; les pools sont construits par un harnais, contigus et dispersés, avec contre-contrôle par la CLI ; N ∈ {1000, 2000, 5000}, BINARY forcé par `search.exact_threshold = 0` sur des copies de mesure ; le froid ne concerne que le rescoring, ses vecteurs fp16 étant en mmap, et il se mesure par copies non cachées (`F_NOCACHE`, `mincore`) ; **Précisé par ADR-0078** : le défaut vaut N = 5000, et les lignes de rescoring de `docs/03-recherche.md` portent désormais les mesures à N = 1000 et à N = 5000 ;
  - la RAM réelle (RSS), en natif sur le Mac. **Précisé par ADR-0077** : trois états (index ouvert, après une passe de requêtes, session fp32 à part), mémoire anonyme et adossée à des fichiers séparées, comparaison au tableau de `docs/02-architecture.md:202-219`.
- **Critères d'acceptation** : les protocoles sont reproductibles et les chiffres sont comparés au budget de `03-recherche.md`.
- **Invariants / mesures** : MES-02, MES-03, MES-09.

### UNIFIER-PARCOURS-SCAN
- **Ajoutée par** : ADR-0081, accepté le 2026-10-27, hors plan initial. Elle met en œuvre la décision et corrige le livrable de `SCANNER-POOL-BINAIRE` qui demande encore deux parcours. Elle se fait en P4, avant J4.
- **Objectif** : un seul parcours clairsemé pour le scan binaire, à la place de la règle de densité d'ADR-0071.
- **Dépend de** : SCANNER-POOL-BINAIRE, MESURER-LATENCE-DENSE
- **À lire** : ADR-0081 (décisions 1 à 8, « Conséquences », « À surveiller ») ; ADR-0071 (le seuil retiré, que cet ADR remplace) ; ADR-0077 (protocole de mesure) ; `docs/mesures/MES-02-parcours-dense-clairseme.md` (§ « Protocole », « Contrôles », « Écarts au protocole », « Résultats », « Conclusion ») ; `docs/03-recherche.md:143` (budget du scan binaire).
- **Livrables** :
  - (1) **d'abord** la mesure multi-segments d'ADR-0081, par `@mesureur`, sur **une copie compactée** d'environ 8 segments de 1 M passages et sur **une copie multi-attaches** (nombreux petits segments), même pool, même plage, N = 5000, 300 requêtes, 3 paires de passes alternées avec **l'ordre des paires alterné** (dense puis clairsemé, puis clairsemé puis dense), et le contrôle `agree` (les deux parcours rendent les mêmes candidats). La mesure est consignée dans une section datée du rapport MES-02 ; les copies de mesure et les sorties brutes vont dans `data/`.
  - (2) **puis, si la mesure confirme ADR-0081**, le code : un seul parcours clairsemé sur l'itérateur trié de la bitmap du pool, `traversal` et `dense_window` supprimés avec la règle de densité ; et, si la mesure montre un coût lié au nombre de segments, un **curseur de segment** qui avance avec l'itérateur trié de la bitmap, en O(1) amorti, au lieu d'une recherche binaire par iid, avec un test du curseur sur des segments de tailles inégales. Le remède n'est pas le retour d'un second parcours.
  - (3) la mise à jour de la fiche `SCANNER-POOL-BINAIRE` ou de sa note (`docs/phases/P4-recherche-rapide.md:48-49`), selon ce qu'ADR-0081 prévoit en décision 6 : le livrable et la note cessent de demander deux parcours et décrivent le seul parcours clairsemé.
- **Critères d'acceptation** :
  - mêmes candidats et même ordre qu'avant : le test de tri complet existant `dense_and_sparse_traversals_agree_with_a_full_sort` (`crates/crible-search/src/dense/binary.rs:576`) est adapté au parcours unique et compare toujours le moteur à un tri complet ;
  - la latence du scan est au moins aussi bonne que celle de l'ancienne règle sur **tous** les points mesurés, et le budget de `docs/03-recherche.md:143` reste tenu ;
  - si la mesure multi-segments contredit ADR-0081 — le clairsemé perd du terrain à mesure que le nombre de segments croît —, une `QUESTION-CONCEPTION` est remontée **avant** le code ;
  - `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` et `cargo test` verts ; build `--release` pour la mesure.
- **Invariants / mesures** : MES-02 ; INV-06 (ADR-0081, décision 5 : le parcours unique ne le touche pas).
- **Hors périmètre** : un seuil par taille de pool ou un point de bascule de densité autre que la moitié (ADR-0081, alternatives (b) et (c)) ; le retour d'un second parcours, même comme filet ; la réécriture d'ADR-0071 et du protocole de MES-02 ; le budget de `docs/03-recherche.md:143` ; toute configuration du choix de parcours.

### REDUIRE-COUT-DES-CLES
- **Ajoutée par** : ADR-0079, accepté le 2026-10-27, hors plan initial. Elle met en œuvre la décision, en P4, avant J4.
- **Objectif** : réduire le coût des clés au rescoring (MES-03 : 29,8 ms en p50 à N = 1000 et 61,1 ms à N = 5000, contre 0,36 ms et 2,04 ms pour le calcul seul).
- **Dépend de** : RESCORER-FP16, MESURER-LATENCE-DENSE. **Pas** de dépendance à `UNIFIER-PARCOURS-SCAN` : ADR-0081 (point 7) fait du scan clairsemé trié une **aide** à l'ordre des iid que demande ADR-0079, décision 2, pas une condition ; les candidats peuvent être ordonnés par le rescoring lui-même, et ADR-0081 ne modifie pas ADR-0079. Si `UNIFIER-PARCOURS-SCAN` est déjà `FAIT`, le scan rend les candidats triés par iid et la tâche le constate par la mesure.
- **À lire** : ADR-0079 (décisions 1 à 7, « Conséquences », « À surveiller ») ; ADR-0072 (décision 3, égalité au bit près) ; ADR-0045 (tri final par score puis clé textuelle) ; `docs/mesures/MES-03-latence-rescoring.md` (tableau, réserves, proposition).
- **Livrables**, dans l'ordre d'ADR-0079 :
  - (e) un **profil par échantillonnage** du rescoring à N = 1000 et à N = 5000, consigné, **avant tout changement** : il attribue les 25,4 ms non expliquées entre la lecture de la clé, le calcul et le reste (allocation de `String`, emprunt de connexion, copie du vecteur, tri). Ni l'API, ni le format, ni les résultats ne changent.
  - (2a) le parcours des candidats dans l'**ordre croissant des iid** avant la boucle de rescoring, pour la localité des pages, et (2b) la ligne fp16 **empruntée** au mmap au lieu d'être copiée quand le décodeur peut travailler sur une vue ; les deux changements mesurés séparément, puis ensemble.
  - (b) la lecture des clés **après le tri par score** : tri intermédiaire sur `(score, iid)`, puis clés pour les `k` résultats, pour les candidats que consomme le regroupement s'il y en a (ADR-0075), et pour **toutes** les égalités de score à la frontière. Le tri final reste `(score décroissant, clé textuelle croissante)` et l'iid ne sort jamais du moteur.
  - (a) les clés **par lot**, en repli, si le profil montre que le coût est dans les N allers-retours et les N emprunts de connexion : une requête unique pour les N iid, découpée selon la limite de variables de SQLite, exécutée hors des lectures de vecteurs.
  - le rejeu de MES-03 au protocole d'ADR-0077 sur `data/mes02` (N ∈ {1000, 2000, 5000}), en section datée du rapport.
- **Critères d'acceptation** :
  - scores et ordre identiques au bit près à EXACT (ADR-0072, décision 3) ;
  - **INV-06** vert ;
  - un test dédié des égalités de score à la frontière, avec des vecteurs dupliqués, prouve que le résultat ne dépend pas de l'ordre d'itération ;
  - le top-10 du harnais de MES-01 donne **0 écart** ;
  - le rapport MES-03 est complété avec le gain chiffré à N = 1000 et à N = 5000 ;
  - `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` et `cargo test` verts ; build `--release` pour la mesure.
- **Invariants / mesures** : INV-06 ; MES-03.
- **Hors périmètre** : les clés en RAM (c) et la colonne de clés en mmap (d), écartées pour l'instant et rejugées après le profil, ce qui demanderait un ADR (budget de MES-09, `docs/02-architecture.md:204`) ; la valeur de `rescore_candidates` (ADR-0078) ; l'accès aux vecteurs fp16, traité par `GROUPER-LECTURES-VECTEURS` ; la règle d'imbrication de `rescore.rs`, qui interdit de tenir la connexion SQLite pendant les lectures de vecteurs.

### GROUPER-LECTURES-VECTEURS
- **Ajoutée par** : ADR-0080, accepté le 2026-10-27, hors plan initial. Elle met en œuvre la décision, en P4, avant J4.
- **Objectif** : réduire le coût de l'accès aux vecteurs fp16 au rescoring (MES-03 : 85,0 ms à froid à N = 1000 et 177,5 ms à N = 5000 pour le seul produit scalaire, contre 0,36 ms et 2,04 ms sur pages chaudes, à environ une page de 16 Kio par candidat).
- **Dépend de** : REDUIRE-COUT-DES-CLES
- **À lire** : ADR-0080 (décisions 1 à 8, « Conséquences », « À surveiller ») ; ADR-0079 (profil et mesures neutres, préalables) ; ADR-0072 (décision 3, égalité au bit près) ; `docs/mesures/MES-03-latence-rescoring.md`, `docs/mesures/MES-09-ram-reelle.md`.
- **Livrables** :
  - la **lecture groupée et parallèle des N lignes fp16** (rayon, tampon transitoire d'au plus 7,7 Mio à N = 5000, libéré après la requête), **avant** le calcul, qui reprend ensuite vecteur par vecteur avec le **même** `kernels::dot` et le même tri final ; la connexion SQLite n'est pas tenue pendant ces lectures ;
  - `madvise(MADV_RANDOM)` **testé dans la même campagne comme contrôle** ; il n'est gardé que si la mesure le retient, et alors avec l'ADR de dépendance `libc` qu'ADR-0080 exige, `libc` étant absent de `docs/07` ; sinon il est retiré sans trace dans le code ;
  - le `pread` groupé en repli, si l'option retenue plafonne sous sa borne (15 à 25 ms à N = 1000, 60 à 90 ms à N = 5000), avec les mêmes contrôles ;
  - le rejeu de MES-03 au protocole d'ADR-0077 avec la **série froide `F_NOCACHE` et `mincore`**, la **série 10 min**, et la **nouvelle série « searcher déjà ouvert, cache déplacé après l'ouverture »** (le froid du moteur n'est pas mesurable autrement, la vérification blake3 à l'ouverture réchauffant le fichier) ; N ∈ {1000, 2000, 5000}, 300 requêtes, 3 passes à chaud ;
  - le **RSS en trois états** (MES-09), mémoire anonyme et adossée à des fichiers séparées, pour montrer que la mémoire possédée ne bouge pas et que le tampon n'est que transitoire.
- **Critères d'acceptation** :
  - scores et ordre identiques au bit près à EXACT (ADR-0072, décision 3) ;
  - **INV-06** vert ;
  - le top-10 du harnais de MES-01 donne **0 écart** ;
  - la mémoire possédée reste inchangée : 1,12 Gio par index, au bruit de mesure près ;
  - le gain à froid est chiffré à N = 1000 **et** à N = 5000, et la contention avec le scan est mesurée (p50 et p95, pas seulement la médiane) ;
  - `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` et `cargo test` verts ; build `--release` pour la mesure.
- **Invariants / mesures** : INV-06 ; MES-03, MES-09.
- **Hors périmètre** : le format des segments et `format_version` (ADR-0006, ADR-0069), et une quantification du rescoring ; les vecteurs en RAM et `mlock` (écarts à la règle de mémoire de `docs/02-architecture.md:204` et `:213`) ; le préchargement `MADV_WILLNEED` et l'ordre physique selon `cluster_by` ; le préchauffage à l'ouverture (ADR-0047) ; la valeur de `rescore_candidates` (ADR-0078) ; les cibles de latence en conteneur et le protocole de froid sous Linux, qui appartiennent à P8.
