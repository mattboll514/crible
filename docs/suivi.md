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
| P1 | ECRIRE-SEGMENTS-VECTEURS | A-FAIRE | | |
| P1 | GERER-GENERATIONS | A-FAIRE | | |
| P1 | LIRE-LOT-PARQUET | A-FAIRE | | |
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
| 2026-10-21 | STOCKER-ENREGISTREMENTS-SQLITE | Aucun `busy_timeout` n'est réglé sur `records.sqlite` : un `SQLITE_BUSY` transitoire ressort en `internal`. Fixer un délai est une valeur par défaut, donc un ADR | ADR-0038 | En attente |

## Décisions en attente

ADR au statut `Proposé`, à trancher par le propriétaire. La liste à jour est dans l'index de `docs/adr/README.md`.

| ADR | Sujet | À décider avant |
|---|---|---|
| ADR-0038 | Délai d'attente sur les verrous SQLite (`busy_timeout`) et code d'erreur à l'expiration | MES de concurrence (TESTER-CHARGE-CONCURRENTE, P6) |

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
