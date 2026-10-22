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
| P1 | ATTACHER-LOT | A-FAIRE | | |
| P1 | CONSTRUIRE-BITMAPS | A-FAIRE | | |
| P1 | EVALUER-FILTRES | A-FAIRE | | |
| P1 | RECHERCHER-DENSE-EXACT | A-FAIRE | | |
| P1 | CLI-PREMIERES-COMMANDES | A-FAIRE | | |
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

## Décisions en attente

ADR au statut `Proposé`, à trancher par le propriétaire. La liste à jour est dans l'index de `docs/adr/README.md`.

| ADR | Sujet | À décider avant |
|---|---|---|

## Idées hors périmètre

Ce qui est utile mais hors de la fiche en cours. On ne l'implémente pas : on le note ici.

| Date | Origine (tâche) | Idée |
|---|---|---|
| 2026-10-19 | INITIALISER-WORKSPACE-RUST | Revérifier la cohabitation `tower-http =0.7.1` / `axum 0.8` (`^0.6.8`) quand `crible-server` utilisera les deux |
| 2026-10-19 | CONFIGURER-QUALITE-ET-CI | Ajouter `clippy::panic` aux lints workspace pour couvrir la règle 10 d'`AGENTS.md` |
| 2026-10-19 | CONFIGURER-QUALITE-ET-CI | CI : ajouter `--locked` aux commandes Cargo, épingler `dtolnay/rust-toolchain`, cache Cargo si la CI ralentit |
| 2026-10-19 | DEFINIR-TYPES-DE-BASE | Consigner dans `docs/02` la règle `PassageKey` (découpage sur le dernier `#`, `DocId` pouvant contenir `#`) |
| 2026-10-19 | DEFINIR-TYPES-DE-BASE | Envisager `trybuild` pour un vrai test de compilation négatif d'INV-01 si cette dépendance entre dans `docs/07` |
| 2026-10-19 | DEFINIR-FORMAT-LOT | P1 : validation stricte de l'hexadécimal et de la longueur des checksums (contrôle 4) |
| 2026-10-19 | DEFINIR-FORMAT-LOT | P1 : décider si le mélange `documents.parquet` / `documents-0001.parquet` doit être refusé (non refusé en P0, documenté) |
| 2026-10-20 | MESURER-REFERENCE-PGVECTOR | Rejouer la passe `--perimeter-first` hors des heures d'usage pour obtenir une référence de latence de fusion non biaisée, si une cible de fusion devient utile |
| 2026-10-21 | Jalon J0 | CI : passer `actions/checkout` et `astral-sh/setup-uv` à des versions qui tournent sur Node.js 24 (Node.js 20 est déprécié sur les runners GitHub) ; surveiller la migration de `ubuntu-latest` vers Ubuntu 26, commencée le 2026-10-19 |
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
| 2026-10-21 | GERER-GENERATIONS (`@dev`) | Pour CLI-PREMIERES-COMMANDES : `GenerationStore::create` sur un index existant réécrit `CURRENT_EPOCH` et `meta.epoch`, à refuser. Pour ATTACHER-LOT : la table `batches` n'est pas couverte par `GenerationChange` |
| 2026-10-21 | LIRE-LOT-PARQUET (relecture) | arrow-rs aplatit une erreur Parquet en `ArrowError::ParquetError(String)` et perd la cause : une vraie E/S pendant le décodage d'un groupe de lignes ressort en `invalid_request` au lieu de `internal` (documenté dans `arrow_failure`) |
| 2026-10-22 | COMPLETER-LECTURE-LOT (`@dev`) | Mélange `documents.parquet` + `documents-NNNN.parquet` toujours non contrôlé (laissé ouvert par ADR-0040) : le lecteur lit la forme non numérotée après les parties numérotées, comportement déterministe et commenté (`Parts::into_names`), à trancher par un ADR si un producteur mélange un jour les deux formes |
| 2026-10-22 | LIRE-LOT-PARQUET (orchestrateur) | `.opencode/agents/relecteur.md` autorise `cargo fmt --check*`, mais le relecteur lance `cargo fmt --all -- --check`, refusé par la règle : élargir le motif ou imposer la forme exacte dans la consigne |

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
