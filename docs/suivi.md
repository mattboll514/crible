# Suivi du projet

Mis à jour par les agents à chaque changement d'état d'une tâche, et par le propriétaire à chaque validation de jalon.

**Statuts des tâches** : `A-FAIRE` · `EN-COURS` · `FAIT` · `BLOQUE` (raison obligatoire dans « Notes »).
**Statuts des jalons** : `EN-ATTENTE` · `PRET-POUR-VALIDATION` (rapport écrit par l'agent `jalon`) · `VALIDE <date>` (écrit **uniquement** par le propriétaire).

## Jalons

| Jalon | Phase | Statut | Rapport |
|---|---|---|---|
| J0 | P0 — Fondations | VALIDE 2026-10-21 | [J0-rapport.md](jalons/J0-rapport.md) |
| J1 | P1 — Stockage, lots et recherche exacte | EN-ATTENTE | |
| J2 | P2 — Encodeur et recette | EN-ATTENTE | |
| J3 | P3 — Cycle de vie | EN-ATTENTE | |
| J4 | P4 — Recherche rapide | EN-ATTENTE | |
| J5 | P5 — Lexical et hybride | EN-ATTENTE | |
| J6 | P6 — Serveur et API | EN-ATTENTE | |
| J7 | P7 — Observabilité | EN-ATTENTE | |
| J8 | P8 — Docker et déploiement | EN-ATTENTE | |
| J9 | P9 — Migration Legifacile (v1) | EN-ATTENTE | |

## Cibles

Fixées le 2026-10-20 d'après MES-00, par l'orchestrateur sur délégation du propriétaire, qui a posé le principe : **le rappel prime, la latence est indicative**, car les mesures de Crible se font sur une machine de développement. La règle de latence de `docs/08` ne vaut que pour une mesure sur le Mac mini en conteneur ; ailleurs, l'écart à pgvector est rapporté, sans bloquer un jalon. Les propositions du §8 du rapport MES-00 sont remplacées par ce tableau.

| Cible | Valeur | Nature | Source |
|---|---|---|---|
| Rappel global (57 questions étiquetées, `k = 10`, déduplication par document) | rappel@10 moyen ≥ **0,482** sur les 56 questions satisfiables (`Q30` à part) | Contraignante | MES-00 §2 |
| Rappel par corpus | `juri_texte_integral` ≥ **0,290** (31 questions), `textes_vigueur` ≥ **0,750** (12 questions). Les corpus de 2 à 4 questions ne sont pas contraints un par un : chaque question où Crible perd un attendu trouvé par pgvector est listée et expliquée dans le rapport de comparaison | Contraignante | MES-00 §2 |
| Latence chaude mono-corpus `k = 10` | p95 ≤ **698 ms** (pgvector : p50 279, p95 698, p99 1 112 ms) | Contraignante seulement sur le Mac mini en conteneur (`docs/08`) ; indicative ailleurs | MES-00 §4.1 |
| Latence froide | référence pgvector : p95 1 163 ms après 10 min d'inactivité (8 mesures, borne basse du froid réel) | Indicative, pas de cible | MES-00 §4.1 |
| Latence de fusion | aucune référence valable (fusion du 20 biaisée à la baisse, passe `--perimeter-first` non rejouée) | Pas de cible | MES-00 §4.2 |
| Fidélité binaire contre exact (recall@10) | ≥ 0,99 (provisoire, à confirmer après MES-01) | Contraignante | `08-invariants-et-tests.md` |

## Tâches

| Phase | Tâche | Statut | Branche / commit | Notes |
|---|---|---|---|---|
| P0 | INITIALISER-WORKSPACE-RUST | FAIT | tache/INITIALISER-WORKSPACE-RUST | Workspace resolver 3, édition 2024, toolchain 1.99.1 ; 6 crates ; dépendances `docs/07` épinglées ; `--version` testé ; fmt/clippy/test/build OK, relecture OK |
| P0 | CONFIGURER-QUALITE-ET-CI | FAIT | tache/CONFIGURER-QUALITE-ET-CI | Workflow CI (fmt/clippy/test + Python conditionnel), `rustfmt.toml`, `clippy.toml`, lints workspace `unwrap_used`/`expect_used`, `Makefile` (`check`/`fmt`/`lint`/`test`) ; `make check` OK ; relecture OK |
| P0 | DEFINIR-TYPES-DE-BASE | FAIT | tache/DEFINIR-TYPES-DE-BASE | `DocId`, `PassageKey`, `Iid`, `Generation`, `Epoch`, `ErrorCode` (14 codes/statuts), `CribleError` ; INV-01 prouvé par analyse statique ; 32 tests ; relecture OK |
| P0 | DEFINIR-CONFIG-INDEX | FAIT | tache/DEFINIR-CONFIG-INDEX | `IndexConfig`, `Schema`, `ModelConfig` avec empreinte canonique UTF-8 (ADR-0028), AST `Filter` et validation (ADR-0029/0030/0031), `apply_patch` atomique (INV-16) ; 116 tests, test croisé Python ; relecture OK |
| P0 | DEFINIR-FORMAT-LOT | FAIT | tache/DEFINIR-FORMAT-LOT | `BatchManifest` + `check_manifest` (format, version, mode, modèle champ par champ, comptes, fichiers) ; `docs/format-lot-v1.md` ; 23 tests, 139 au total ; relecture OK |
| P0 | MESURER-REFERENCE-PGVECTOR | FAIT | tache/MESURER-REFERENCE-PGVECTOR | MES-00 mesurée en production en lecture seule (campagne du 2026-10-20 : 1 000 requêtes chaudes, 8 froides, fidélité HNSW, contre-mesure, 57 questions) ; harnais `tools/bench/mes00/` (73 tests pytest, job CI `tools`) ; preuve d'absence d'écriture par `find -newermt` ; cibles fixées par délégation du propriétaire ; relecture OK au 7e tour (tours 4 à 7 autorisés par le propriétaire), 2 mineurs notés en idées |
| P1 | ECRIRE-LOTS-PYTHON | FAIT | tache/ECRIRE-LOTS-PYTHON | Paquet `crible` (`clients/python`, backend `uv_build`) : `BatchWriter`, `documents.parquet` + parties `passages-NNNN.parquet` (≤ 1 M, écrites par blocs de 64 k), checksums blake3 (ADR-0034), manifeste écrit de façon atomique, contrôles locaux miroirs de `check_manifest` ; 41 tests pytest, 139 Rust ; relecture OK au 2e tour |
| P1 | GENERER-JEU-SYNTHETIQUE | FAIT | tache/GENERER-JEU-SYNTHETIQUE | `tools/gen_synth.py` (`synth-small` en 3 lots, `--large`, `--overwrite` limité aux fichiers produits), `tools/reference_topk.py` (top-k exact en f32 comme le moteur, nulls selon ADR-0035) ; état de référence après L1-L3 : 1 950 documents, 9 750 passages ; somme `blake3:2d3d8265…58805` vérifiée par test (artefacts connus seulement) ; `BatchWriter` reçoit `created_at` et `host` (ADR-0036) ; 125 tests `tools`, 52 `clients`, 139 Rust ; relecture OK au 3e tour |
| P1 | STOCKER-ENREGISTREMENTS-SQLITE | FAIT | tache/STOCKER-ENREGISTREMENTS-SQLITE | `crible-store::records` : schéma des 5 tables et 2 index, WAL réaffirmé à l'ouverture, texte en zstd, champs en JSON canonique (ADR-0019), `format_version` vérifié avant toute modification (INV-15, 5 tests `inv_15_*`), pool borné de connexions en lecture seule (bibliothèque standard) ; 28 tests `crible-store`, 167 Rust ; relecture OK au 2e tour, 2 mineurs notés en idées |
| P1 | ECRIRE-SEGMENTS-VECTEURS | FAIT | tache/ECRIRE-SEGMENTS-VECTEURS | `crible-store::segment` : `vectors.f16`/`vectors.f32` en little-endian explicite, `segment.json` versionné et canonique (champs de `docs/02` + `format_version`, `precision` et `segment_id` dérivés à l'ouverture sur verdict de `@architecte`), checksums blake3 vérifiés à l'ouverture, écriture atomique `tmp-<id>` → fsync → rename, lecteur mmap (`// SAFETY:` testé) ; erreurs : entrée client → `invalid_request`, état disque ou iid hors plage → `internal` (INV-01) ; 21 tests (8 `inv_15_*`), 189 Rust ; relecture OK au 2e tour |
| P1 | GERER-GENERATIONS | FAIT | tache/GERER-GENERATIONS | `crible-store::generation` : `GenerationStore` (`CURRENT_EPOCH` décimal, une transaction SQLite comme unique point de commit, ADR-0016, refus des lignes d'une autre génération, manifeste `manifests/<G>.json` dérivé en JSON canonique versionné, `Snapshot` publié par `ArcSwap`), reprise (nettoyage des `tmp-*` et des segments orphelins, réécriture du manifeste manquant), 4 failpoints derrière la feature `failpoints` ; le `Snapshot` ne porte que des métadonnées (politique de `SegmentReader::open` inchangée) ; 12 tests `inv_03_*`/`inv_04_*` ; 203 tests Rust (207 avec `failpoints`) ; relecture OK au 2e tour, 2 mineurs notés en idées |
| P1 | LIRE-LOT-PARQUET | FAIT | tache/LIRE-LOT-PARQUET | `crible-store::batch_reader` : lecture de `manifest.json`, `check_manifest`, checksums blake3, schémas Parquet, puis un seul passage en flux par fichier (blocs Arrow de 8 192 lignes au plus) pour les comptes, les séquences d'`ord`, les documents et les vecteurs finis et normés ; ordre des lignes imposé par ADR-0039 (mémoire bornée à une entrée par document) ; défaut du lot → `invalid_request`, vraie E/S → `internal` ; la CI génère `synth-small` avant `cargo test` ; 20 tests d'intégration, 227 tests Rust ; relecture OK au 3e tour (bloquée au 2e par ADR-0039), 2 mineurs notés en idées |
| P1 | ATTACHER-LOT | FAIT | tache/ATTACHER-LOT | `crible-store::attach` : modes `upsert` et `replace_all`, règles du tableau de `docs/04`, plan en lecture seule puis segment écrit et renommé avant la transaction, toutes les écritures SQLite (`documents`, `passages`, pierres tombales, suppression des lignes `documents`, `batches`, `segments`, `meta`) dans la seule transaction du commit de génération (ADR-0016), aucune écriture après le point de commit ; idempotence par `batch_id` et `batch_conflict` sur le digest des octets de `manifest.json` (ADR-0041) ; six comptes par document (ADR-0041) ; durées rendues au seul appelant, jamais persistées (ADR-0042) ; contrôle croisé `documents` ∩ `deletions` avant toute écriture. 6 tests de `@testeur` + 4 ajoutés (10 avec `failpoints`), 248 tests Rust, 112 avec `failpoints` ; ADR-0041 et ADR-0042 décidés en cours de tâche ; relecture OK au 4e tour (4e tour autorisé par le propriétaire), 3 mineurs notés en idées |
| P1 | CONSTRUIRE-BITMAPS | FAIT | tache/CONSTRUIRE-BITMAPS | `crible-store::filters_index` : bitmap roaring par valeur `keyword` (clés `Arc<str>` partagées entre générations), bitmaps `bool` et « présent » par champ (null absent, ADR-0035), colonnes `i64`/`f64`/`i32` (jours) avec sentinelles, bitmap des vivants (le `Snapshot` n'en garde plus de copie en `Vec`), héritage document → passage ; construites depuis SQLite à la création et à l'ouverture, dérivées en copie sur écriture à chaque commit ; `Snapshot::filter_memory` (affichage par `crible inspect` en CLI-PREMIERES-COMMANDES) ; `GenerationStore::create`/`open` prennent `&Schema` ; 263 tests Rust, 127 avec `failpoints` ; relecture OK au 3e tour, 4 mineurs notés en idées |
| P1 | EVALUER-FILTRES | FAIT | tache/EVALUER-FILTRES | `crible-search::pool` : `Pool::evaluate(&FilterIndex, &Filter)` (bitmaps d'abord, triées par taille, puis colonnes restreintes au domaine réduit à chaque niveau d'imbrication ; `$or` en union ; `$not` en complément dans le domaine reçu, sous-ensemble des vivants ; intersection finale avec les vivants), `bitmap()`/`len()`/`is_empty()` ; toutes les clauses contrôlées même quand le pool est déjà vide ; `invalid_filter` pour un filtre non conforme, sans valeur de filtre dans le message ; `in: []` = ∅ et `nin: []` = présent, conséquence directe d'ADR-0035 ; `date_days` de `crible-store` rendu public ; tests INV-07 écrits d'abord par `@testeur` (propriété à 1 000 filtres contre `json_extract`, 74 cas fixes et plus) ; 269 tests Rust ; relecture OK aux trois tours, mineurs corrigés après les deux premiers, 4 notés en idées |
| P1 | RECHERCHER-DENSE-EXACT | FAIT | tache/RECHERCHER-DENSE-EXACT | `crible-search::dense::exact` : `ExactSearcher::open`/`top_k`, produit scalaire fp16 → f32 scalaire, pool découpé en au plus une fenêtre contiguë d'iid par worker rayon, un tas borné par fenêtre, fusion par cosinus décroissant puis clé textuelle `doc_id#ord` croissante (ordre de `reference_topk.py`) ; clé seule lue pendant le parcours, payload des k retenus après la fusion ; résultat dont la ligne `documents` a disparu rendu avec texte et champs de passage (ADR-0043) ; génération sans segment → aucun résultat ; requête malformée → `invalid_request` ; `records.sqlite` illisible refusé à l'ouverture ; `ReadPool::read_passage`/`read_document`/`read_passage_key`/`verify` dans `crible-store` ; CI : `reference_topk.py` génère `expected.jsonl` ; tests INV-06 écrits d'abord par `@testeur` (200 requêtes, comparateur de tolérance strict, 0 inversion, 0 remplacement) ; 303 tests Rust, 130 avec `failpoints` ; relecture OK au 4e tour (autorisé par le propriétaire) |
| P1 | CLI-PREMIERES-COMMANDES | FAIT | tache/CLI-PREMIERES-COMMANDES | Crate `crible` : `index create`, `attach` (`--models <dir>` explicite, volume séparé de `docs/02`/`docs/07`, sans valeur par défaut), `search` (`.npy` f4/f8 ou JSON, `--filter`, `--k`, `took_ms` total), `inspect` (génération, époque, comptes, segments sans iid, mémoire des bitmaps ; exécute la reprise, documenté) ; erreurs `code` + message (`unknown_index`, `invalid_request`) ; `docs/jalons/J1-demo.md` rejoué tel qu'écrit et couvert par `cli::the_j1_demo_creates_attaches_searches_and_inspects` (dont la recherche filtrée et le vecteur absent). Trois idées intégrées sur décision du propriétaire : `create` refuse un index existant ou déchiré (`CURRENT_EPOCH`, `epochs/`, `records.sqlite`) avant toute écriture ; `index.json` écrit atomiquement par `GenerationStore::write_index_config` et recoupé à `create`/`open` ; pool rayon dédié `crible-search::scan_pool::ScanPool`, chemins d'époque publics. 321 tests Rust, 138 avec `failpoints` ; relecture A-CORRIGER au 1er tour (1 bloquant, 4 importants), OK au 2e (7 mineurs corrigés), OK au 3e (5 mineurs notés en idées) ; garde-fou ADR-0037 sans écart aux trois tours |
| P2 | EXPORTER-MODELE-ONNX | A-FAIRE | | |
| P2 | CONSTITUER-ECHANTILLON-LEGIFACILE | A-FAIRE | | |
| P2 | CHARGER-MODELE | A-FAIRE | | |
| P2 | ENCODER-REQUETES | A-FAIRE | | |
| P2 | MESURER-PARITE-ENCODEUR | A-FAIRE | | |
| P2 | CONTROLER-LONGUEURS | A-FAIRE | | |
| P2 | RECETTE-IMPORT | A-FAIRE | | |
| P2 | ENCODER-DOCUMENTS-EN-LIGNE | A-FAIRE | | |
| P2 | CLI-RECHERCHE-TEXTE | A-FAIRE | | |
| P3 | PATCHER-METADONNEES | A-FAIRE | | |
| P3 | COMPACTER-INDEX | A-FAIRE | | |
| P3 | VERIFIER-EQUIVALENCE-COMPACTION | A-FAIRE | | |
| P3 | EXPORTER-LOT | A-FAIRE | | |
| P3 | VERIFIER-ALLER-RETOUR-EXPORT | A-FAIRE | | |
| P3 | CREER-ET-RESTAURER-INSTANTANE | A-FAIRE | | |
| P3 | TESTER-REPRISE-APRES-PANNE | A-FAIRE | | |
| P3 | MESURER-CYCLE-DE-VIE | A-FAIRE | | |
| P4 | QUANTIFIER-BINAIRE | A-FAIRE | | |
| P4 | NOYAUX-SIMD | A-FAIRE | | |
| P4 | SCANNER-POOL-BINAIRE | A-FAIRE | | |
| P4 | RESCORER-FP16 | A-FAIRE | | |
| P4 | PLANIFIER-STRATEGIE | A-FAIRE | | |
| P4 | REGROUPER-RESULTATS | A-FAIRE | | |
| P4 | MESURER-FIDELITE-BINAIRE | A-FAIRE | | |
| P4 | MESURER-LATENCE-DENSE | A-FAIRE | | |
| P5 | CONFIGURER-ANALYSEUR | A-FAIRE | | |
| P5 | INDEXER-TANTIVY-A-L-ATTACHE | A-FAIRE | | |
| P5 | RECONSTRUIRE-LEXICAL | A-FAIRE | | |
| P5 | RECHERCHER-BM25-DANS-LE-POOL | A-FAIRE | | |
| P5 | FUSIONNER-HYBRIDE | A-FAIRE | | |
| P5 | MESURER-LEXICAL | A-FAIRE | | |
| P5 | MESURER-HYBRIDE | A-FAIRE | | |
| P6 | DEMARRER-SERVEUR-AXUM | A-FAIRE | | |
| P6 | AUTHENTIFIER-CLES | A-FAIRE | | |
| P6 | FORMAT-ERREURS-ET-LIMITES | A-FAIRE | | |
| P6 | FILE-D-ECRITURE-ET-JOBS | A-FAIRE | | |
| P6 | ROUTES-INDEX | A-FAIRE | | |
| P6 | ROUTES-DOCUMENTS-ET-LOTS | A-FAIRE | | |
| P6 | ROUTE-RECHERCHE | A-FAIRE | | |
| P6 | RECHERCHE-MULTI-INDEX | A-FAIRE | | |
| P6 | PUBLIER-OPENAPI | A-FAIRE | | |
| P6 | CLIENT-PYTHON-HTTP | A-FAIRE | | |
| P6 | TESTER-CHARGE-CONCURRENTE | A-FAIRE | | |
| P7 | AGREGER-ANALYTICS | A-FAIRE | | |
| P7 | TESTER-ANALYTICS-SANS-CONTENU | A-FAIRE | | |
| P7 | JOURNAL-DE-DEBUG | A-FAIRE | | |
| P7 | REPONSE-DEBUG | A-FAIRE | | |
| P7 | EXPLIQUER-DOCUMENTS | A-FAIRE | | |
| P7 | ROUTE-STATS | A-FAIRE | | |
| P7 | COMMANDE-EVAL | A-FAIRE | | |
| P8 | CONSTRUIRE-IMAGE-DOCKER | A-FAIRE | | |
| P8 | DEFINIR-VOLUMES-ET-CONFIG | A-FAIRE | | |
| P8 | VERIFIER-ARRET-PROPRE | A-FAIRE | | |
| P8 | COMPOSE-EXEMPLE | A-FAIRE | | |
| P8 | PUBLIER-IMAGE | A-FAIRE | | |
| P8 | MESURER-EN-CONTENEUR-MAC | A-FAIRE | | |
| P9 | DEFINIR-INDEX-LEGIFACILE | A-FAIRE | | |
| P9 | ADAPTER-PRODUCTEUR-LEGIFACILE | A-FAIRE | | |
| P9 | IMPORTER-BASE-COMPLETE | A-FAIRE | | |
| P9 | CONSTITUER-JEU-DE-REFERENCE | A-FAIRE | | |
| P9 | COMPARER-AVEC-PGVECTOR | A-FAIRE | | |
| P9 | BRANCHER-MCP | A-FAIRE | | |
| P9 | MODE-OMBRE | A-FAIRE | | |
| P9 | BRANCHER-TACHE-QUOTIDIENNE | A-FAIRE | | |
| P9 | BASCULER-PRODUCTION | A-FAIRE | | |
| P9 | BILAN-V1 | A-FAIRE | | |

## Écarts à la conception

Quand une tâche ne peut pas respecter la conception (docs 01 à 08 ou un ADR accepté), ou demande un choix que rien ne tranche, l'agent fait rédiger un **ADR `Proposé`** par `@greffier`, ajoute une ligne ici et **s'arrête**. Seul le propriétaire tranche, en passant l'ADR en `Accepté` ou en `Rejeté`.

| Date | Tâche | Écart constaté | ADR proposé | Décision du propriétaire |
|---|---|---|---|---|
| 2026-10-19 | DEFINIR-CONFIG-INDEX | `serde_json` est requis pour l'analyse JSON mais absent de `docs/07`, qui ne liste que `serde` et `toml` | ADR-0027 | Accepté le 2026-10-19 |
| 2026-10-19 | DEFINIR-CONFIG-INDEX | Forme canonique de l'empreinte du modèle non spécifiée (clés, compacité, champs imbriqués, encodage des hash) | ADR-0028 | Accepté le 2026-10-19, avec UTF-8 sans échappement non ASCII |
| 2026-10-19 | DEFINIR-CONFIG-INDEX | Comptage de la profondeur et des clauses d'un filtre non défini | ADR-0029 | Accepté le 2026-10-19 |
| 2026-10-19 | DEFINIR-CONFIG-INDEX | Comportement non défini quand un nom de champ est déclaré aux deux niveaux du schéma | ADR-0030 | Accepté le 2026-10-19 |
| 2026-10-19 | DEFINIR-CONFIG-INDEX | Comportement non défini pour une clé inconnue dans un `PATCH` de configuration | ADR-0031 | Accepté le 2026-10-19 |
| 2026-10-21 | ECRIRE-LOTS-PYTHON | Le producteur Python doit calculer un digest blake3, absent de `docs/07` (seule la crate Rust y figure) et de `hashlib` | ADR-0034 | Accepté le 2026-10-21 |
| 2026-10-21 | GENERER-JEU-SYNTHETIQUE | Sémantique d'un champ null pour `ne`, `nin`, `exists` et `$not` non définie, alors que la référence numpy (`expected.jsonl`) l'impose à EVALUER-FILTRES et à INV-07 | ADR-0035 | Accepté le 2026-10-21 |
| 2026-10-21 | GENERER-JEU-SYNTHETIQUE | `producer.host` (« Machine qui a produit le lot ») rend la somme de contrôle versionnée de `synth-small` propre à une machine | ADR-0036 | Accepté le 2026-10-21 |
| 2026-10-21 | STOCKER-ENREGISTREMENTS-SQLITE | Aucun `busy_timeout` n'est réglé sur `records.sqlite` : un `SQLITE_BUSY` transitoire ressort en `internal`. Fixer un délai est une valeur par défaut, donc un ADR | ADR-0038 | Accepté le 2026-10-21 |
| 2026-10-21 | LIRE-LOT-PARQUET | Le lecteur en flux exige que les lignes d'un document arrivent dans l'ordre croissant des `ord` (séquence continue d'une partie à l'autre, parties par suffixe croissant) pour garder une mémoire bornée ; `format-lot-v1.md` et `docs/04` n'imposent que des `ord` contigus. Verdict de `@architecte` : NON-COUVERT, option A recommandée (figer l'ordre dans le format v1, `format_version` inchangée) | ADR-0039 | Accepté le 2026-10-21, option A, `format_version` reste à 1 |
| 2026-10-22 | LIRE-LOT-PARQUET (relecture) | Le format ne dit pas qu'un `doc_id` est unique dans `documents` ni dans `deletions` (le lecteur fusionne un doublon en silence), et deux parties de même valeur numérique de suffixe rendent l'ordre des parties indéfini ; le lecteur trie les parties par nom, pas par valeur numérique | ADR-0040 | Accepté le 2026-10-22, tel que proposé, `format_version` reste à 1 |
| 2026-10-22 | ATTACHER-LOT | Avant implémentation, verdicts de `@architecte` : (1) NON-COUVERT, « document retiré » (`docs/04`) ne dit pas si la ligne `documents` tombe au commit ou survit jusqu'à la compaction ; (2) NON-COUVERT, la base de `manifest_blake3` (octets lus ou JSON canonique) n'est pas définie ; (3) ECART, `docs/04` énumère six comptes du rapport, l'exemple de `docs/05` quatre, en mélangeant passages et documents | ADR-0041 | Accepté le 2026-10-22, tel que proposé (options A pour 1 et 2, six comptes par document pour 3) |
| 2026-10-22 | ATTACHER-LOT (relecture, 3e tour) | `docs/04` exige à la fois « la durée de chaque étape » dans le rapport et qu'un rejeu « renvoie le rapport d'origine », alors que trois des cinq durées (`transaction`, `manifest`, `snapshot`) se terminent au COMMIT ou après, et qu'aucune écriture n'est permise après le point de commit (ADR-0016, fiche). Verdict de `@architecte` : ECART, option B recommandée (durées non persistées) | ADR-0042 | Accepté le 2026-10-22, option B |
| 2026-10-22 | RECHERCHER-DENSE-EXACT (relecture, 2e tour) | Les payloads sont lus dans l'état SQLite courant, pas dans la génération du snapshot ; depuis ADR-0041, un document retiré au commit suivant n'a plus de ligne `documents` et le moteur rend `internal` (500) sur une requête valide. `docs/02` (« Concurrence », « Payloads ») n'admet qu'un écart dû à une modification de métadonnées. Verdict de `@architecte` : ECART et NON-COUVERT, option A recommandée (résultat rendu avec le texte et les seuls champs de passage, sans erreur) | ADR-0043 || Accepté le 2026-10-22, option A sans marqueur |

## Décisions en attente

ADR au statut `Proposé`, à trancher par le propriétaire. La liste à jour est dans l'index de `docs/adr/README.md`.

| ADR | Sujet | À décider avant |
|---|---|---|

## Idées hors périmètre

Ce qui est utile mais hors de la fiche en cours. On ne l'implémente pas : on le note ici.

| Date | Origine (tâche) | Idée |
|---|---|---|
| 2026-10-19 | INITIALISER-WORKSPACE-RUST | Revérifier la cohabitation `tower-http =0.7.1` / `axum 0.8` (`^0.6.8`) quand `crible-server` utilisera les deux |
| 2026-10-19 | CONFIGURER-QUALITE-ET-CI | CI : épingler `dtolnay/rust-toolchain`, cache Cargo si la CI ralentit (`--locked` ajouté par NETTOYER-P1) |
| 2026-10-19 | DEFINIR-TYPES-DE-BASE | Consigner dans `docs/02` la règle `PassageKey` (découpage sur le dernier `#`, `DocId` pouvant contenir `#`) |
| 2026-10-19 | DEFINIR-TYPES-DE-BASE | Envisager `trybuild` pour un vrai test de compilation négatif d'INV-01 si cette dépendance entre dans `docs/07` |
| 2026-10-19 | DEFINIR-FORMAT-LOT | P1 : validation stricte de l'hexadécimal et de la longueur des checksums (contrôle 4) |
| 2026-10-19 | DEFINIR-FORMAT-LOT | P1 : décider si le mélange `documents.parquet` / `documents-0001.parquet` doit être refusé (non refusé en P0, documenté) |
| 2026-10-20 | MESURER-REFERENCE-PGVECTOR | Rejouer la passe `--perimeter-first` hors des heures d'usage pour obtenir une référence de latence de fusion non biaisée, si une cible de fusion devient utile |
| 2026-10-21 | Jalon J0 | CI : surveiller la migration de `ubuntu-latest` vers Ubuntu 26, commencée le 2026-10-19 (actions passées sur Node.js 24 par NETTOYER-P1) |
| 2026-10-21 | AMELIORER-LOTS-ET-REFERENCE | `tools/gen_synth.py --overwrite` ne nettoie pas un `.<batch_id>.tmp-*` laissé par un crash antérieur (sans effet sur la somme ni sur la référence) ; course résiduelle sur un dossier cible vide créé entre le contrôle et le renommage, faute de `RENAME_NOREPLACE` portable en Python |
| 2026-10-21 | ECRIRE-LOTS-PYTHON (verdict de `@architecte`) | Pour GENERER-JEU-SYNTHETIQUE : une modification de métadonnées seules renvoie le document avec le même `doc_hash` **et tous ses passages** (`docs/04`, Règles), donc le lot `L3` inclut les passages de ses 50 documents ; un lot de suppressions seules garde des fichiers `documents…` et `passages…`, vides si besoin (`format-lot-v1.md`) |
| 2026-10-21 | STOCKER-ENREGISTREMENTS-SQLITE | Niveau de compression zstd non mesuré ; schéma non `STRICT`, qui laisse passer des types faux dans `generation`/`next_iid` |
| 2026-10-21 | STOCKER-ENREGISTREMENTS-SQLITE (relecture) | Un fichier non SQLite ouvert par `create` remonte en `internal` plutôt qu'en `invalid_request` (état inattendu, défendable) |
| 2026-10-21 | STOCKER-ENREGISTREMENTS-SQLITE (`@dev`) | Pour GERER-GENERATIONS et ATTACHER-LOT : accesseurs typés de `batches` et `segments` à ajouter à `crible-store::records` |
| 2026-10-21 | ECRIRE-SEGMENTS-VECTEURS | Pour GERER-GENERATIONS : `SegmentReader::open` relit tout `vectors.*` pour vérifier le checksum (≈ 13 Go par ouverture à l'échelle Legifacile) ; décider d'une vérification différée ou d'un `open` sans vérification pour les instantanés et le redémarrage. Un `tmp-<id>` résiduel fait échouer `SegmentWriter::create` jusqu'à la reprise (comportement figé par un test) : le nettoyage des `tmp-*` de la reprise doit le couvrir (nettoyage des `tmp-*` fait par la reprise de GERER-GENERATIONS ; la question du checksum reste ouverte, le `Snapshot` n'ouvrant aucun segment) |
| 2026-10-21 | ECRIRE-SEGMENTS-VECTEURS (`@dev`) | Test d'un segment dont `count × dims` dépasse `u32`, impossible à produire avant les générations |
| 2026-10-21 | REGLER-BUSY-TIMEOUT (relecture) | `tests/sqlite_busy_timeout.rs` : le test de contention courte parie sur un `sleep(100 ms)` avant le `COMMIT` du détenteur et exige une attente ≥ 50 ms (échec possible si le thread écrivain est privé d'ordonnancement plus de 100 ms, jamais observé en 40 passages) ; la borne basse du test d'expiration (`elapsed >= 1 000 ms`) n'a aucune tolérance. Élargir les marges si la CI se montre instable |
| 2026-10-21 | REGLER-BUSY-TIMEOUT (`@dev`) | `SQLITE_BUSY` à l'ouverture d'une connexion (récupération du WAL, `-shm`) couvert par `database_error` mais pas testé en intégration, faute de scénario déterministe ; `docs/02` (« Concurrence ») ne cite pas ADR-0038, que seul `docs/05` reprend |
| 2026-10-21 | GERER-GENERATIONS, mesuré par MES-15 | Commit en O(passages vivants) : à 8,58 M passages, p50 598 ms (S = 10) à 641 ms (S = 1 000), dont 86,5 % pour matérialiser les iid vivants ; transaction SQLite constante (78-100 ms), dominée par `zstd::encode_all` par ligne (≈ 6 ms avec un compresseur réutilisé) ; ≈ 13-15 µs par `segment.json` relu. Propositions de `@mesureur`, à valider par le propriétaire : dérivation incrémentale de la bitmap des vivants comme critère de CONSTRUIRE-BITMAPS ; cache des `SegmentMeta` seulement si S dépasse quelques centaines ; compresseur zstd réutilisé, à évaluer avec MES-08 |
| 2026-10-21 | GERER-GENERATIONS (`@dev`) | Pour ATTACHER-LOT : la table `batches` n'est pas couverte par `GenerationChange` |
| 2026-10-21 | LIRE-LOT-PARQUET (relecture) | arrow-rs aplatit une erreur Parquet en `ArrowError::ParquetError(String)` et perd la cause : une vraie E/S pendant le décodage d'un groupe de lignes ressort en `invalid_request` au lieu de `internal` (documenté dans `arrow_failure`) |
| 2026-10-22 | COMPLETER-LECTURE-LOT (`@dev`) | Mélange `documents.parquet` + `documents-NNNN.parquet` toujours non contrôlé (laissé ouvert par ADR-0040) : le lecteur lit la forme non numérotée après les parties numérotées, comportement déterministe et commenté (`Parts::into_names`), à trancher par un ADR si un producteur mélange un jour les deux formes |
| 2026-10-22 | ATTACHER-LOT (`@git`, orchestrateur) | Le contrôle « aucune valeur de `.env.prod` dans le diff indexé » de `@git` écrit ces valeurs dans un fichier temporaire d'opencode (`prodvals.txt`), que ses permissions l'empêchent ensuite de supprimer (`rm` refusé) : des valeurs confidentielles restent sur disque hors du dépôt (supprimées à la main par le propriétaire). Faire le contrôle sans fichier intermédiaire (substitution de processus, comme dans `git.md`), ou autoriser la suppression de ce seul fichier ; à trancher par un ADR si les permissions changent (ADR-0026). Revu le 2026-10-22 : `git.md` impose déjà la substitution de processus et l'incident ne s'est pas reproduit sur les 40 passages suivants ; à surveiller |
| 2026-10-22 | CONSTRUIRE-BITMAPS (`@dev`) | À trancher par mesure (MES-13 ou MES-15) : le delta de copie sur écriture balaie `passages` faute d'index SQLite sur `created_generation` (`docs/02` n'en liste que deux, ajouter un index demande un ADR) ; `KeywordField::clear` balaie toutes les valeurs du champ par passage effacé (coût documenté, un index inverse `iid → valeur` demanderait un ADR) |
| 2026-10-22 | EVALUER-FILTRES (relecture) | `crible-core::filter::check_scalar` recopie la valeur d'une date dans son message d'erreur : une valeur de filtre peut finir dans un log (règle 11). À traiter avec FORMAT-ERREURS-ET-LIMITES ou JOURNAL-DE-DEBUG. Et `docs/05` ne dit pas explicitement qu'une liste vide est permise pour `in`/`nin` (le résultat découle d'ADR-0035) |
| 2026-10-22 | RECHERCHER-DENSE-EXACT (relecture et `@dev`) | Le classement départage sur la clé textuelle `doc_id#ord` (comme `reference_topk.py`), alors que `PassageKey::Ord` compare `(doc_id, ord)` numériquement : divergence dès `ord = 10`, à trancher avant `explain_ids` (P4/P6) |
| 2026-10-22 | RECHERCHER-DENSE-EXACT (relecture, 4e tour, et `@dev`) | À mesurer ou à améliorer au planificateur P4 : un aller-retour SQLite par candidat qui entre dans un tas (lecture des clés en lot possible) ; bornes des fenêtres placées par un parcours complet de la bitmap (`RoaringBitmap::select` possible) ; paramètre `total` de `windows()` redondant avec `bitmap.len()` ; capacité du pool de lecture côté serveur. Et : `ReadPool::verify` ne contrôle ni les tables ni `format_version` ; rejouer une partie des 200 requêtes INV-06 dans un pool à 1 thread couvrirait le découpage sur un pool fragmenté multi-segments |
| 2026-10-22 | CLI-PREMIERES-COMMANDES (`@dev` et relecture) | `inspect` ouvre le store et exécute la reprise (nettoyage de `tmp-*`, segments orphelins, verrou d'écriture SQLite) : il échoue sur un montage en lecture seule ; une ouverture en lecture seule pour `inspect`/`search` est à prévoir (P3 ou P6). `took_ms` n'est pas asserté par le test de démo. Le constructeur `test_config` reste recopié par cible de `crible-store` (tests unitaires, intégration, bench), faute de crate partagée : à laisser, sauf ADR |
| 2026-10-22 | NETTOYER-P1 (`@dev`) | Le `Makefile` (`make lint`, `make test`) n'utilise pas `--locked`, contrairement à la CI |
| 2026-10-22 | NETTOYER-P1 (relecture) | Les règles de `@relecteur` refusent `cargo doc --no-deps …` : les liens intra-doc ne sont vérifiés que par lecture ; autoriser `cargo doc --no-deps*` demanderait un ADR (ADR-0026, ADR-0044) |

## Journal

Une ligne par événement notable : début et fin de tâche, blocage, validation de jalon.

| Date | Événement |
|---|---|
| 2026-10-18 | Conception terminée ; dépôt initialisé ; plan de 10 phases et 83 tâches |
| 2026-10-19 | Projet installé dans le dépôt `mattboll514/crible` ; registre de décisions converti en ADR (0001 à 0020) ; ADR-0021 (adoption des ADR) et ADR-0022 (ColGrep) acceptés ; ADR 0016 à 0019 en attente de décision |
| 2026-10-19 | ADR 0016 à 0019 acceptés par le propriétaire : plus aucune décision en attente avant P1 |
| 2026-10-19 | Agents orchestrateur et git ajoutés ; pipeline séquentiel ; ADR-0023 et ADR-0024 proposés ; guide opencode écrit |
| 2026-10-19 | ADR-0023 et ADR-0024 acceptés par le propriétaire |
| 2026-10-19 | ADR-0025 accepté : opencode v2, configuration conservée au format v1 |
| 2026-10-19 | Environnement installé et vérifié : opencode 2.0.32 relié à Ollama (DeepSeek V4.1 Flash), plugins caveman et notifier chargés, permissions des agents vérifiées par simulation |
| 2026-10-19 | Commit initial `4bbfb9d` (conception, ADR, agents, configuration opencode) poussé sur `origin/main` |
| 2026-10-19 | Début de la tâche INITIALISER-WORKSPACE-RUST sur `tache/INITIALISER-WORKSPACE-RUST` |
| 2026-10-19 | INITIALISER-WORKSPACE-RUST terminée : workspace Cargo et 6 crates, `--version` testé ; revue OK ; Rust 1.99.1 installé hors dépôt (prérequis) |
| 2026-10-19 | ADR-0026 accepté (permissions des agents, mode `--auto`) ; configuration commitée sur `phase/P0-fondations` |
| 2026-10-19 | Début de la tâche CONFIGURER-QUALITE-ET-CI sur `tache/CONFIGURER-QUALITE-ET-CI` |
| 2026-10-19 | CONFIGURER-QUALITE-ET-CI terminée : CI, lints workspace, `Makefile` ; `make check` OK ; revue OK ; actionlint indisponible en local |
| 2026-10-19 | Début de la tâche DEFINIR-TYPES-DE-BASE sur `tache/DEFINIR-TYPES-DE-BASE` |
| 2026-10-19 | DEFINIR-TYPES-DE-BASE terminée : identifiants et erreurs typées, INV-01 couvert, 32 tests ; revue OK |
| 2026-10-19 | Début de la tâche DEFINIR-CONFIG-INDEX sur `tache/DEFINIR-CONFIG-INDEX` |
| 2026-10-19 | DEFINIR-CONFIG-INDEX bloquée : l'architecte relève 5 points non couverts ; ADR-0027 à 0031 proposés, en attente de décision |
| 2026-10-19 | ADR-0027 à 0031 acceptés ; documents mis à jour ; reprise de DEFINIR-CONFIG-INDEX |
| 2026-10-19 | DEFINIR-CONFIG-INDEX terminée : config, modèle, filtres, patch ; test croisé Rust/Python de l'empreinte ; 116 tests ; revue OK |
| 2026-10-19 | ADR-0032 accepté (accès production) ; fiche MES-00 réécrite ; contrôle anti-fuite `.env.prod` OK |
| 2026-10-19 | Début de la tâche DEFINIR-FORMAT-LOT sur `tache/DEFINIR-FORMAT-LOT` |
| 2026-10-19 | DEFINIR-FORMAT-LOT terminée : manifeste et validation, spécification `format-lot-v1.md` ; 23 tests ; revue OK |
| 2026-10-19 | ADR-0026 accepté : utilitaires autorisés à tous les agents (fin des rejets silencieux sur les commandes composées), git réservé à `@git` même en `--auto`, interdictions renforcées |
| 2026-10-19 | ADR-0032 accepté : mesures en production sur le Mac mini, en lecture seule ; accès dans `.env.prod`, non versionné ; fiche MESURER-REFERENCE-PGVECTOR réécrite |
| 2026-10-19 | Écart de relecture DEFINIR-FORMAT-LOT accepté par le propriétaire : `check_manifest` prend aussi `&ModelConfig` ; fiche, doc 04 et `format-lot-v1.md` alignés |
| 2026-10-19 | Début de la tâche MESURER-REFERENCE-PGVECTOR sur `tache/MESURER-REFERENCE-PGVECTOR` |
| 2026-10-20 | MES-00 bloquée après 3 tours de relecture : mesure conforme rejouée le 20 (1 000 chaudes, 8 froides, fidélité, contre-mesure), mais preuves d'inventaire et 8 points de documentation restent à corriger ; décision du propriétaire requise |
| 2026-10-20 | ADR-0033 appliqué : l'orchestrateur tourne dans Claude Code, outillage commité (`b813ab9`) |
| 2026-10-20 | MES-00 : 4e et dernier tour autorisé par le propriétaire ; 4e relecture `A-CORRIGER` (3 bloquants, 3 importants, 9 mineurs) ; preuve d'inventaire par fichier datée 08:25 UTC et argv q0018 ef=1000 ajoutés au rapport par `@mesureur`, sans remesure |
| 2026-10-20 | MES-00 : 12 corrections locales par `@dev` (valeurs `PGOPTIONS` fictives dans les tests, harnais sans 0 ms fabriqué, README et rapport alignés) ; 5e relecture `A-CORRIGER` (1 important, 6 mineurs) ; tâche de nouveau BLOQUE, décision du propriétaire requise |
| 2026-10-20 | Cibles MES-00 fixées par l'orchestrateur sur délégation du propriétaire : rappel contraignant (≥ 0,482 global, par corpus pour `juri_texte_integral` et `textes_vigueur`), latence indicative hors du Mac mini en conteneur |
| 2026-10-20 | MES-00 : 5e tour (option A du propriétaire) ; 7 points corrigés par `@dev` ; relecture `A-CORRIGER` (1 important, 3 mineurs) ; tâche BLOQUE, décision du propriétaire requise |
| 2026-10-20 | MES-00 : 6e micro-tour autorisé par le propriétaire ; 4 points corrigés par `@dev` ; relecture `A-CORRIGER` (1 important de libellé du protocole, 2 mineurs) ; tâche BLOQUE, décision du propriétaire requise |
| 2026-10-20 | MESURER-REFERENCE-PGVECTOR terminée : 7e et dernier tour autorisé par le propriétaire, relecture OK (139 tests Rust, 73 tests Python, anti-fuite 0) ; cibles fixées ; toutes les tâches de P0 sont FAIT |
| 2026-10-21 | J0 préparé par `@jalon` : 6 tâches FAIT, `make check` vert (139 tests Rust), 73 tests Python, INV-01 (3 tests) et INV-16 (18 tests) verts, MES-00 et cibles présentes, ADR 0026 à 0033 acceptés ; rapport `docs/jalons/J0-rapport.md`, statut PRET-POUR-VALIDATION ; réserve : aucun run CI distant (branche non poussée) |
| 2026-10-21 | Rapport MES-00 : 2 mineurs de relecture appliqués par `@mesureur` (consigne du `find -newermt` reformulée, inventaire `ls-files` d'avant situé dans le protocole reproductible), sans nouvelle mesure ; idées retirées de la liste. La passe `--perimeter-first` reste en idée, à revoir en P5 |
| 2026-10-21 | `phase/P0-fondations` poussée avec l'accord du propriétaire ; premier run CI GitHub vert (run 39574670548 : Rust 139 tests, Python, Tools 73 tests), ce qui lève la réserve CI du rapport J0 ; avertissements de dépréciation Node.js 20 notés en idée |
| 2026-10-21 | J0 validé par le propriétaire ; phase P0 fusionnée dans `main`, tag `jalon-J0` |
| 2026-10-21 | Phase P1 ouverte (`phase/P1-stockage-et-recherche-exacte`) ; début de la tâche ECRIRE-LOTS-PYTHON sur `tache/ECRIRE-LOTS-PYTHON` |
| 2026-10-21 | ECRIRE-LOTS-PYTHON bloquée : `@dev` remonte l'absence de blake3 côté Python ; `@architecte` rend NON-COUVERT (et CONFORME sur les métadonnées seules et sur le paramètre `model`) ; ADR-0034 proposé, décision du propriétaire requise |
| 2026-10-21 | ADR-0034 accepté par le propriétaire : `blake3` ajouté aux dépendances Python de `docs/07` ; reprise de ECRIRE-LOTS-PYTHON |
| 2026-10-21 | ECRIRE-LOTS-PYTHON terminée : paquet Python `crible` et `BatchWriter` (Parquet, blake3, manifeste) ; 41 tests pytest ; revue OK au 2e tour (backend `hatchling` remplacé par `uv_build`) |
| 2026-10-21 | Début de la tâche GENERER-JEU-SYNTHETIQUE sur `tache/GENERER-JEU-SYNTHETIQUE` |
| 2026-10-21 | GENERER-JEU-SYNTHETIQUE bloquée avant relecture : `@architecte` rend NON-COUVERT sur les nulls dans les filtres et ECART sur `producer.host` ; ADR-0035 et ADR-0036 proposés, décision du propriétaire requise |
| 2026-10-21 | ADR-0035 (nulls dans les filtres, logique SQL) et ADR-0036 (`producer.host` réglable) acceptés par le propriétaire ; `docs/03`, `docs/05` et `format-lot-v1.md` mis à jour ; reprise de GENERER-JEU-SYNTHETIQUE |
| 2026-10-21 | GENERER-JEU-SYNTHETIQUE terminée : générateur `synth-small` reproductible au bit près sur toute machine, référence exacte f32 ; 125 tests `tools` ; revue OK au 3e tour (2e tour relancé une fois après une coupure réseau) |
| 2026-10-21 | Idées traitées sur décision du propriétaire : aller-retour Python → Rust intégré à la fiche LIRE-LOT-PARQUET, tolérance des quasi-égalités intégrée à la fiche RECHERCHER-DENSE-EXACT ; ADR-0037 (permissions de `@relecteur`) proposé ; écriture atomique du lot et table type → opérateurs lancées en tâche hors plan |
| 2026-10-21 | Tâche hors plan AMELIORER-LOTS-ET-REFERENCE terminée : écriture atomique du dossier de lot (dossier temporaire caché, `fsync`, renommage) et validation type → opérateurs dans l'oracle ; 64 tests `clients`, 145 `tools`, 139 Rust ; somme de `synth-small` inchangée ; revue OK au 2e tour |
| 2026-10-21 | ADR-0037 accepté par le propriétaire : `@relecteur` reçoit les utilitaires de vérification et l'exécution Python (`xargs` écarté, condition non remplie) ; garde-fou `git status` inscrit dans `orchestrateur.md` ; 9 commandes testées acceptées et `xargs` refusé (`timeout` absent de macOS, motif sans effet ici) |
| 2026-10-21 | Début de la tâche STOCKER-ENREGISTREMENTS-SQLITE sur `tache/STOCKER-ENREGISTREMENTS-SQLITE` |
| 2026-10-21 | STOCKER-ENREGISTREMENTS-SQLITE terminée : base `records.sqlite` d'une époque (schéma, zstd, JSON canonique, INV-15, pool en lecture seule) ; 28 tests `crible-store` ; revue OK au 2e tour (pool rouvert en lecture seule, version vérifiée avant le DDL) |
| 2026-10-21 | Sur demande du propriétaire : ADR-0038 (`busy_timeout` SQLite, recommandation 1 000 ms et `overloaded`) proposé ; fiche ATTACHER-LOT précisée (une seule transaction, celle du commit de génération, et test d'échec au milieu des écritures) ; les 2 mineurs de relecture de STOCKER-ENREGISTREMENTS-SQLITE confiés à la tâche hors plan RENFORCER-TESTS-POOL ; idée « table `deletions` » retirée (`deletions` est un fichier du lot, pas une table) |
| 2026-10-21 | Début de la tâche hors plan RENFORCER-TESTS-POOL sur `tache/RENFORCER-TESTS-POOL` (2 mineurs de relecture de STOCKER-ENREGISTREMENTS-SQLITE) |
| 2026-10-21 | Tâche hors plan RENFORCER-TESTS-POOL terminée : refus d'écriture du pool prouvé par le code `ReadOnly` puis une lecture sur la même connexion, libération du slot après la panique d'un lecteur testée ; test préexistant `read_pool_waits_when_its_only_connection_is_busy` rendu déterministe (il échouait 4 fois sur 10, trouvé par le relecteur) ; 168 tests Rust, `read_pool` vert 12 fois sur 12 ; revue OK au 2e tour |
| 2026-10-21 | Début de la tâche ECRIRE-SEGMENTS-VECTEURS sur `tache/ECRIRE-SEGMENTS-VECTEURS` |
| 2026-10-21 | ECRIRE-SEGMENTS-VECTEURS terminée : segments de vecteurs immuables (f16/f32 little-endian, `segment.json` versionné, blake3, écriture atomique, lecture mmap) ; format de `segment.json` et codes d'erreur tranchés par `@architecte` avant l'implémentation (sans ADR) ; 21 tests ; revue OK au 2e tour |
| 2026-10-21 | ADR-0038 accepté par le propriétaire : busy_timeout fixe de 1 000 ms sur toutes les connexions SQLite, expiration en overloaded ; application dans le code à faire (petite tâche hors plan) |
| 2026-10-21 | Début de la tâche hors plan REGLER-BUSY-TIMEOUT sur `tache/REGLER-BUSY-TIMEOUT` (application d'ADR-0038) |
| 2026-10-21 | Tâche hors plan REGLER-BUSY-TIMEOUT terminée : `busy_timeout` fixe de 1 000 ms sur les trois chemins d'ouverture de `records.sqlite`, `SQLITE_BUSY` et ses variantes traduits en `overloaded`, le reste en `internal` ; 4 tests (attente puis expiration, contention courte surmontée, traduction) ; 193 tests Rust ; revue OK au 1er tour, 2 mineurs de robustesse des tests notés en idées |
| 2026-10-21 | Début de la tâche GERER-GENERATIONS sur `tache/GERER-GENERATIONS` |
| 2026-10-21 | GERER-GENERATIONS terminée : protocole de commit des générations (point de commit SQLite unique), reprise au démarrage, `Snapshot` publié par `ArcSwap`, failpoints ; 11 tests d'invariants écrits par `@testeur` et passés sans retouche, 1 ajouté ; revue OK au 2e tour (fsync des dossiers dans `create`, `tmp-CURRENT_EPOCH`, refus des lignes d'une autre génération) |
| 2026-10-21 | Début de la tâche hors plan COMPLETER-GENERATIONS sur `tache/COMPLETER-GENERATIONS` (2 mineurs de relecture de GERER-GENERATIONS, formats de `CURRENT_EPOCH` et du manifeste dans `docs/02`, ligne MES-15 dans `docs/08`, sur décision du propriétaire) |
| 2026-10-21 | Tâche hors plan COMPLETER-GENERATIONS terminée : garde de génération étendu aux documents, branches « passage » et « document » testées, `SerialGuard` sur tous les tests `inv_03` ; formats de `CURRENT_EPOCH` et de `manifests/<G>.json` décrits dans `docs/02` ; MES-15 (coût d'un commit) ajoutée à `docs/08` ; 205 tests Rust (70 `crible-store` avec `failpoints`) ; revue OK, puis 6 mineurs de précision corrigés et revue OK sans point ; 2 idées retirées de la liste |
| 2026-10-21 | Début de la tâche hors plan MESURER-COUT-COMMIT sur `tache/MESURER-COUT-COMMIT` (MES-15, coût d'un commit de génération, à la demande du propriétaire) |
| 2026-10-21 | Tâche hors plan MESURER-COUT-COMMIT terminée : MES-15 mesurée (banc `crible-store/benches/commit_cost.rs`, pilote `tools/bench/mes15/`), commit de 2 000 passages en O(N), p50 598 ms à 8,58 M passages ; campagne sur machine chargée (Ollama actif), chiffres plutôt pessimistes ; jeux dans `data/mes15/` (6,3 Go, gitignoré) ; revue OK au 2e tour (scripts en anglais, garde `--overwrite`, attribution de la charge, commandes exactes), 3 mineurs corrigés ensuite |
| 2026-10-21 | Début de la tâche LIRE-LOT-PARQUET sur `tache/LIRE-LOT-PARQUET` |
| 2026-10-21 | LIRE-LOT-PARQUET bloquée au 2e tour de relecture : le lecteur impose l'ordre des lignes d'un lot, ce que le format public ne dit pas ; `@architecte` rend NON-COUVERT ; ADR-0039 proposé, décision du propriétaire requise. 1er tour OK avec 8 mineurs (6 corrigés : erreurs d'E/S en `internal`, taille de lecture fixe, tests `normalize: false`, borne de norme, lot en deux parties) ; 2e tour : 3 mineurs corrigés ; travail non commité dans l'arbre |
| 2026-10-21 | ADR-0039 accepté par le propriétaire (option A, `format_version` reste à 1) ; `format-lot-v1.md` et `docs/04` mis à jour ; LIRE-LOT-PARQUET repasse EN-COURS, reprise au 3e tour de relecture |
| 2026-10-22 | LIRE-LOT-PARQUET terminée : commentaires de `batch_reader.rs` et de deux tests réalignés sur ADR-0039 accepté ; revue OK au 3e tour, garde-fou ADR-0037 sans écart ; 227 tests Rust ; 2 mineurs notés en idées (ordre des parties par nom plutôt que par suffixe, en-tête de test en français) |
| 2026-10-22 | ADR-0040 proposé (unicité des `doc_id` et des parties dans un lot v1), à la demande du propriétaire, avant ATTACHER-LOT ; une petite tâche hors plan corrigera le lecteur après acceptation |
| 2026-10-22 | ADR-0040 accepté par le propriétaire, tel que proposé ; `format-lot-v1.md` (noms des parties, tableaux `documents` et `deletions`, contrôle 6, erreurs) et `docs/04` (contrôle 6) mis à jour |
| 2026-10-22 | Début de la tâche hors plan COMPLETER-LECTURE-LOT sur `tache/COMPLETER-LECTURE-LOT` (application d'ADR-0040 au lecteur, et en-tête de `tests/lire_lot_parquet.rs`, sur décision du propriétaire) |
| 2026-10-22 | Tâche hors plan COMPLETER-LECTURE-LOT terminée : parties triées par valeur numérique du suffixe (`passages-9999` avant `passages-10000`), valeur répétée et suffixe hors `u64` refusés ; `doc_id` unique dans `documents` (toutes parties) et dans `deletions` ; suppressions bornées par `counts.deletions` avant insertion ; en-têtes de tests en anglais ; 12 tests ajoutés, 238 tests Rust ; revue A-CORRIGER au 1er tour (borne des suppressions, en-tête de `batch_manifest.rs`), OK au 2e ; 3 idées retirées de la liste |
| 2026-10-22 | Début de la tâche ATTACHER-LOT sur `tache/ATTACHER-LOT` |
| 2026-10-22 | ATTACHER-LOT : 6 tests écrits par `@testeur` (INV-03, INV-10, comptes L1-L3, `replace_all`), non commités ; 3 points de conception soumis à `@architecte` avant `@dev` (2 NON-COUVERT, 1 ECART) ; ADR-0041 proposé ; tâche BLOQUE, décision du propriétaire requise |
| 2026-10-22 | ADR-0041 accepté par le propriétaire, tel que proposé ; `docs/02` (`batches.manifest_blake3`, ligne `documents` d'un document vivant), `docs/04` (document retiré, `manifest_blake3`, six comptes du rapport) et `docs/05` (exemple de rapport, `exists`/`alive` d'`explain`) mis à jour ; ATTACHER-LOT repasse EN-COURS |
| 2026-10-22 | ATTACHER-LOT bloquée au 3e tour de relecture. 1er tour A-CORRIGER (segment écrit dans la transaction, `unchanged`/`ignored` non testés, test de conflit sur une valeur et non sur la forme, 5 mineurs) ; 2e tour OK avec 6 mineurs, corrigés à la demande de l'orchestrateur, dont la persistance des durées dans `batches.report` ; 3e tour A-CORRIGER : cette persistance passe par un `UPDATE` post-commit contraire à ADR-0016 et à la fiche, et `docs/04` ne permet pas de trancher le contenu du rapport persisté ; `@architecte` rend ECART ; ADR-0042 proposé ; travail non commité dans l'arbre |
| 2026-10-22 | ADR-0042 accepté par le propriétaire (option B) ; `docs/04` (idempotence, rapport persisté sans durées) et `docs/05` (`result.report` sans durées) mis à jour ; 4e tour de relecture autorisé ; ATTACHER-LOT repasse EN-COURS |
| 2026-10-22 | ATTACHER-LOT terminée : écriture post-commit supprimée, durées non persistées (`None` au rejeu), test `no_sqlite_write_follows_the_commit_point` ; revue OK au 4e tour, garde-fou ADR-0037 sans écart à chaque tour ; 248 tests Rust, 112 avec `failpoints` ; 3 mineurs notés en idées (dont un `allow` de lint contraire à la règle 5) |
| 2026-10-22 | Début de la tâche hors plan COMPLETER-ATTACHE sur `tache/COMPLETER-ATTACHE` (3 mineurs de la relecture d'ATTACHER-LOT, sur décision du propriétaire) |
| 2026-10-22 | Tâche hors plan COMPLETER-ATTACHE terminée : `batch_id` retiré du rapport persisté (ADR-0042), `#[allow(clippy::too_many_arguments)]` supprimé par une structure `SegmentTarget` (plus aucun `allow(clippy` dans `crates/`), message d'erreur du rapport stocké juste pour chaque clé ; 248 tests Rust, 112 avec `failpoints` ; revue OK au 1er tour sans point ; 3 idées retirées de la liste |
| 2026-10-22 | Début de CONSTRUIRE-BITMAPS sur `tache/CONSTRUIRE-BITMAPS` (aucun invariant cité, pas de passage par `@testeur` ; `crible inspect` relève de CLI-PREMIERES-COMMANDES, la tâche expose seulement la taille mémoire) |
| 2026-10-22 | CONSTRUIRE-BITMAPS terminée : 1er tour A-CORRIGER (clés `String` recopiées à chaque commit, hypothèse du delta mal sourcée, test des vivants tautologique, `date_days` trop lâche, 3 mineurs) ; 2e tour OK avec 4 mineurs, corrigés à la demande de l'orchestrateur (snapshots tenus réaffirmés, `Vec<Iid>` retiré du `Snapshot`, ajout hors séquence refusé) ; 3e tour OK, 4 mineurs notés en idées ; garde-fou ADR-0037 sans écart à chaque tour ; tests d'invariants `inv_03`/`inv_04` non affaiblis (API seule) ; 263 tests Rust, 127 avec `failpoints` |
| 2026-10-22 | Début d'EVALUER-FILTRES sur `tache/EVALUER-FILTRES` (INV-07 cité : tests d'invariant d'abord par `@testeur`) |
| 2026-10-22 | EVALUER-FILTRES terminée : tests INV-07 écrits d'abord par `@testeur` (propriété à 1 000 filtres contre `json_extract`, cas fixes ADR-0035) ; désaccord testeur/dev sur `ne` tranché par la relecture d'après `docs/05` (`ne` permis sur les types numériques, `nin` réservé à `keyword`) ; oracle ajusté pour `nin: []` conformément à ADR-0035 ; 1er tour OK avec 4 mineurs, 2e tour OK avec 6 mineurs, corrigés chaque fois à la demande de l'orchestrateur (court-circuit retiré, domaine propagé aux nœuds imbriqués, valeur de filtre retirée des messages) ; 3e tour OK, 4 mineurs notés en idées ; contrôle empirique annoncé par le relecteur au 3e tour mais refusé par ses règles, noté en idée ; garde-fou ADR-0037 sans écart à chaque tour ; 269 tests Rust |
| 2026-10-22 | Début de RECHERCHER-DENSE-EXACT sur `tache/RECHERCHER-DENSE-EXACT` (INV-06 cité : tests d'invariant d'abord par `@testeur`) |
| 2026-10-22 | RECHERCHER-DENSE-EXACT bloquée au 2e tour de relecture : tests INV-06 écrits d'abord par `@testeur` (200 requêtes contre `expected.jsonl`, comparateur de tolérance strict), implémentation par `@dev`, 1er tour OK avec 8 mineurs corrigés (payload lu après la fusion, aucun iid dans les messages, score non fini refusé) ; 2e tour A-CORRIGER, dont un point de conception soumis à `@architecte` (ECART) et l'ADR-0043 proposé par `@greffier` ; garde-fou ADR-0037 sans écart à chaque tour |
| 2026-10-22 | ADR-0043 accepté par le propriétaire (option A sans marqueur) ; appliqué par `@greffier` à `docs/02` (« Payloads »), `docs/03` (étape 8), `docs/05` (`return.fields`), `docs/08` (portée d'INV-03), fiches P1 (critère et test `missing_document_row_renders_passage_fields`) et P3 (COMPACTER-INDEX) ; RECHERCHER-DENSE-EXACT repasse EN-COURS |
| 2026-10-22 | RECHERCHER-DENSE-EXACT bloquée au 3e tour de relecture : ADR-0043 mis en œuvre (test `missing_document_row_renders_passage_fields` sur un ancien instantané), points 2 à 9 du 2e tour corrigés, mais la suppression du `Vec<u32>` du pool (point 8) découpe le parcours par run de la bitmap, soit un travail rayon par passage sur un pool fragmenté ; garde-fou ADR-0037 sans écart à chaque tour ; 301 tests Rust, 130 avec `failpoints` ; 4e tour soumis au propriétaire |
| 2026-10-22 | 4e tour de relecture de RECHERCHER-DENSE-EXACT autorisé par le propriétaire ; la tâche repasse EN-COURS |
| 2026-10-22 | RECHERCHER-DENSE-EXACT terminée : 4e tour de relecture OK sans point bloquant (fenêtres de parcours bornées par le nombre de workers, test unitaire du découpage sur un pool fragmenté, chercheur ouvert à 1 thread et interrogé à 4) ; relecteur honnête sur ses commandes refusées ; garde-fou ADR-0037 sans écart aux quatre tours ; 303 tests Rust, 130 avec `failpoints` ; 4 idées notées |
| 2026-10-22 | ADR-0044 proposé (règles de `@relecteur` : forme `cargo fmt --all -- --check`, `RAYON_NUM_THREADS` à valeurs fixes, vérifications non exécutées signalées), sur décision du propriétaire de traiter les idées d'outillage avant la fin de P1 |
| 2026-10-22 | Début de CLI-PREMIERES-COMMANDES sur `tache/CLI-PREMIERES-COMMANDES` (aucun invariant cité, pas de passage par `@testeur`) ; trois idées notées « pour CLI-PREMIERES-COMMANDES » transmises à `@dev` sur décision du propriétaire : refus de `create` sur un index existant, schéma lu dans `index.json`, pool rayon dédié aux scans et chemins d'époque publics |
| 2026-10-22 | ADR-0044 accepté par le propriétaire, tel que proposé ; application à `.opencode/agents/relecteur.md` par `@greffier` après le passage de `@dev` (un seul agent à la fois), avant la relecture de CLI-PREMIERES-COMMANDES |
| 2026-10-22 | CLI-PREMIERES-COMMANDES terminée : 1er tour A-CORRIGER (modèle lu sous `--data` au lieu du volume séparé, schéma recoupé seulement si `index.json` existe, `iid_start` affiché par `inspect` contre INV-01, `unknown_index` absent, démo couverte en partie) ; 2e tour OK avec 7 mineurs, corrigés à la demande de l'orchestrateur (dont le refus d'un arbre d'index déchiré) ; 3e tour OK, 5 mineurs notés en idées, dont 4 pour NETTOYER-P1 ; ADR-0044 appliqué en cours de tâche, vérifications à 1 et 4 threads rayon faites par `@relecteur` ; garde-fou ADR-0037 sans écart aux trois tours ; 321 tests Rust, 138 avec `failpoints` ; 5 lignes d'idées retirées et une raccourcie (3 idées intégrées à la tâche, 3 réglées par ADR-0044), 2 lignes ajoutées |
| 2026-10-22 | Début de la tâche hors plan NETTOYER-P1 sur `tache/NETTOYER-P1`, sur décision du propriétaire : mineurs restants des relectures de CONSTRUIRE-BITMAPS (4), EVALUER-FILTRES (4) et CLI-PREMIERES-COMMANDES (4), lint `clippy::panic` (règle 10), CI sur Node.js 24 (`actions/checkout@v5`, `astral-sh/setup-uv@v7`) et `--locked` sur les commandes Cargo |
| 2026-10-22 | Tâche hors plan NETTOYER-P1 terminée : 12 mineurs de relecture soldés (CONSTRUIRE-BITMAPS : `FilterIndex::live_iids()` retiré, doc d'`after_commit`, ligne longue, `SerialGuard` sur tous les tests qui commitent ; EVALUER-FILTRES : docs de `pool.rs` et en-têtes des tests, cas fixe `inv_07_a_nested_or_and_column_read_the_reduced_domain` ; CLI-PREMIERES-COMMANDES : messages distincts pour un index complet et un arbre partiel, `index.json` validé avant écriture, test d'un `records.sqlite` seul) ; lint `clippy::panic` (`allow-panic-in-tests`, `#![allow(clippy::panic)]` dans les 25 fichiers de tests dont un helper panique, seul `panic!` hors tests `stop_at_failpoint`, justifié), fiche P0 mise à jour ; CI sur `actions/checkout@v5` et `astral-sh/setup-uv@v7` (Node.js 24), `--locked` sur `cargo clippy` et `cargo test` ; en-tête d'`inv_06` à jour ; 3 tests ajoutés, 324 tests Rust, 140 avec `failpoints` ; relecture OK au 1er tour (1 mineur corrigé) et au 2e sans point ; garde-fou ADR-0037 sans écart ; 4 lignes d'idées retirées, 2 raccourcies, 2 ajoutées |
