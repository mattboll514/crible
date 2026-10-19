# Suivi du projet

Mis à jour par les agents à chaque changement d'état d'une tâche, et par le propriétaire à chaque validation de jalon.

**Statuts des tâches** : `A-FAIRE` · `EN-COURS` · `FAIT` · `BLOQUE` (raison obligatoire dans « Notes »).
**Statuts des jalons** : `EN-ATTENTE` · `PRET-POUR-VALIDATION` (rapport écrit par l'agent `jalon`) · `VALIDE <date>` (écrit **uniquement** par le propriétaire).

## Jalons

| Jalon | Phase | Statut | Rapport |
|---|---|---|---|
| J0 | P0 — Fondations | EN-ATTENTE | |
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

À remplir après MES-00 (tâche `MESURER-REFERENCE-PGVECTOR`), puis à faire valider par le propriétaire.

| Cible | Valeur | Source |
|---|---|---|
| Rappel minimal (jeu de référence Legifacile) | _à fixer_ | MES-00 |
| Latence p95 maximale, recherche à l'échelle de Legifacile | _à fixer_ | MES-00 |
| Fidélité binaire contre exact (recall@10) | ≥ 0,99 (provisoire) | `08-invariants-et-tests.md` |

## Tâches

| Phase | Tâche | Statut | Branche / commit | Notes |
|---|---|---|---|---|
| P0 | INITIALISER-WORKSPACE-RUST | FAIT | tache/INITIALISER-WORKSPACE-RUST | Workspace resolver 3, édition 2024, toolchain 1.99.1 ; 6 crates ; dépendances `docs/07` épinglées ; `--version` testé ; fmt/clippy/test/build OK, relecture OK |
| P0 | CONFIGURER-QUALITE-ET-CI | A-FAIRE | | |
| P0 | DEFINIR-TYPES-DE-BASE | A-FAIRE | | |
| P0 | DEFINIR-CONFIG-INDEX | A-FAIRE | | |
| P0 | DEFINIR-FORMAT-LOT | A-FAIRE | | |
| P0 | MESURER-REFERENCE-PGVECTOR | A-FAIRE | | |
| P1 | ECRIRE-LOTS-PYTHON | A-FAIRE | | |
| P1 | GENERER-JEU-SYNTHETIQUE | A-FAIRE | | |
| P1 | STOCKER-ENREGISTREMENTS-SQLITE | A-FAIRE | | |
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

## Décisions en attente

ADR au statut `Proposé`, à trancher par le propriétaire. La liste à jour est dans l'index de `docs/adr/README.md`.

| ADR | Sujet | À décider avant |
|---|---|---|

## Idées hors périmètre

Ce qui est utile mais hors de la fiche en cours. On ne l'implémente pas : on le note ici.

| Date | Origine (tâche) | Idée |
|---|---|---|
| 2026-10-19 | INITIALISER-WORKSPACE-RUST | Revérifier la cohabitation `tower-http =0.7.1` / `axum 0.8` (`^0.6.8`) quand `crible-server` utilisera les deux |

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
