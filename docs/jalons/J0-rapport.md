# Rapport de jalon J0 — Fondations

> Rédigé par l'agent `jalon` (commande `/jalon J0`). **Seul le propriétaire peut valider le jalon**, dans `docs/suivi.md`.

- **Date** : 2026-10-21
- **Branche de phase / commit** : `phase/P0-fondations` / `8bb8da4`
- **Machine** : Mac de développement, macOS 15.7.7, arm64 ; Rust 1.99.1 (`cargo 1.99.1 (5f94df478 2026-08-27)`)
- **Verdict de l'agent** : `PRET-POUR-VALIDATION` (une réserve de traçabilité sur la CI distante, voir §7)

## 1. Tâches de la phase

Toutes les tâches de P0 sont `FAIT` dans `docs/suivi.md`.

| Tâche | Statut dans `suivi.md` | Commit | Remarque |
|---|---|---|---|
| INITIALISER-WORKSPACE-RUST | FAIT | `6db606c` (fusion `255cf47`) | Workspace resolver 3, édition 2024, 6 crates ; `cargo run -p crible -- --version` rend `crible 0.1.0` |
| CONFIGURER-QUALITE-ET-CI | FAIT | `a3ebb71` (fusion `0f35339`) | CI Rust/Python/Tools, `rustfmt.toml`, `clippy.toml`, lints `unwrap_used`/`expect_used`, `Makefile` ; `make check` vert |
| DEFINIR-TYPES-DE-BASE | FAIT | `164715e` (fusion `2ea7ef8`) | `DocId`, `PassageKey`, `Iid`, `Generation`, `Epoch`, `ErrorCode` (14 codes), `CribleError` ; INV-01 couvert |
| DEFINIR-CONFIG-INDEX | FAIT | `9e60413` (fusion `6c8eb34`) | Config, schéma, modèle, empreinte canonique, AST `Filter`, `apply_patch` atomique ; INV-16 couvert |
| DEFINIR-FORMAT-LOT | FAIT | `12fd586` (fusion `0c97f5e`), complément `98355e5` | `BatchManifest`, `check_manifest`, `docs/format-lot-v1.md` |
| MESURER-REFERENCE-PGVECTOR | FAIT | `c38a3de` (fusion `8bb8da4`) | MES-00 mesurée en production en lecture seule le 2026-10-20 ; harnais `tools/bench/mes00/` |

## 2. Qualité

Exécuté sur `phase/P0-fondations`, arbre de travail propre avant et après les contrôles.

| Contrôle | Commande | Résultat |
|---|---|---|
| Formatage | `cargo fmt --check` | OK, code de sortie 0 |
| Lints | `cargo clippy --all-targets -- -D warnings` | OK, 0 avertissement, code de sortie 0 |
| Tests Rust | `cargo test` | **139 passés, 0 échoué, 0 ignoré** |
| Cible agrégée | `make check` (`fmt` + `lint` + `test`) | OK, code de sortie 0 |
| Version du binaire | `cargo run -q -p crible -- --version` | `crible 0.1.0` |
| Tests Python | `cd tools && uv run pytest -q` | **73 passés, 0 échoué**, 0,20 s |
| Syntaxe du workflow | `actionlint` | non exécuté : outil absent de la machine (déjà noté au suivi) |

Détail des binaires de test Rust : `crible` 1 (`version`), `crible-core` 138, dont
`inv_01` 3, `inv_16` 18, `batch_manifest` 23, `filter` 38, `index_config` 16,
`model_config` 8, `doc_id` 5, `passage_key` 15, `error_codes` 12, tests unitaires 12.

## 3. Invariants de la phase

| Invariant | Test(s) | Résultat |
|---|---|---|
| INV-01 — aucun iid ne sort du moteur | `crates/crible-core/tests/inv_01_iid_not_serializable.rs` : `inv_01_iid_is_declared_in_the_crates`, `inv_01_iid_does_not_implement_serialize`, `inv_01_detector_flags_a_serializable_iid` | **3 passés, 0 échoué** (`cargo test -p crible-core --test inv_01_iid_not_serializable`) |
| INV-16 — un réglage figé ne se modifie pas | `crates/crible-core/tests/inv_16_immutable_settings.rs` : 18 tests (`inv_16_renaming_the_index_is_rejected`, `inv_16_changing_the_model_is_rejected`, `inv_16_changing_the_reference_precision_is_rejected`, `inv_16_changing_the_type_of_an_existing_field_is_rejected`, `inv_16_a_rejected_patch_changes_nothing`, `inv_16_every_immutable_top_level_setting_is_rejected`, …) | **18 passés, 0 échoué** (`cargo test -p crible-core --test inv_16_immutable_settings`) |

INV-01 est prouvé par analyse statique du code source (le test vérifie qu'`Iid`
ne déclare pas `Serialize` et qu'un détecteur repère une implémentation
sérialisable), conformément à la fiche : pas de test de compilation négatif.

## 4. Mesures de la phase

| Mesure | Rapport | Chiffre clé | Cible | Atteinte ? |
|---|---|---|---|---|
| MES-00 — référence pgvector | `docs/mesures/MES-00-reference-pgvector.md` (campagne du 2026-10-20) | rappel@10 moyen **0,482** sur 56 questions satisfiables (`k = 10`, dédoublonnage document) ; `juri_texte_integral` 0,290 (31 q.), `textes_vigueur` 0,750 (12 q.) ; latence chaude mono-corpus p50 **279 ms**, p95 **698 ms**, p99 **1 112 ms** (1 000 requêtes) ; froid p95 **1 163 ms** (8 mesures) ; fidélité HNSW contre exact : recall@10 1,000 à `ef = 200`, 0,933–0,988 à `ef = 40` selon corpus | Cibles de `docs/suivi.md` §« Cibles », fixées d'après MES-00 : rappel ≥ 0,482, par corpus ≥ 0,290 / ≥ 0,750, latence p95 ≤ 698 ms (contraignante seulement sur le Mac mini en conteneur), fidélité binaire ≥ 0,99 | n/a : MES-00 est la **référence** qui fixe les cibles, pas une mesure de Crible |

MES-00 n'a **pas** été rejouée : la campagne de production est figée, en lecture
seule (ADR-0032). Seuls le rapport et ses fichiers bruts ont été vérifiés :
8 fichiers dans `docs/mesures/` (`…md`, 3 JSON de latence/froid, 2 JSON de
fidélité, 1 éval de recherche, 1 sortie brute de requête). Les 73 tests du
harnais passent en local, sans réseau ni accès à la production.

Les cibles sont écrites dans `docs/suivi.md`, section « Cibles », avec leur
nature (contraignante ou indicative) et leur source. Le propriétaire les
confirme en validant J0.

## 5. Démo à rejouer par le propriétaire

Les commandes exactes, à copier-coller depuis la racine du dépôt, sur
`phase/P0-fondations`. Aucune ne touche à la production.

```bash
cd /Users/mattboll/MattCorp/crible

# 1. Qualité complète : formatage, lints, tests Rust
make check
```
Attendu : aucune sortie d'erreur, code de sortie 0, `test result: ok` sur chaque
binaire, 139 tests passés au total.

```bash
# 2. Le binaire répond
cargo run -q -p crible -- --version
```
Attendu : `crible 0.1.0`.

```bash
# 3. Configs d'index et manifestes de lot : cas valides et invalides
cargo test -p crible-core --test index_config --test model_config \
  --test filter --test batch_manifest
```
Attendu : 4 binaires verts, 16 + 8 + 38 + 23 = **85 tests passés**, 0 échoué.
Ces tests couvrent l'exemple de manifeste du doc 04, un mauvais
`format_version`, un modèle incompatible champ par champ, `replace_all` avec
fichier `deletions`, un fichier absent du manifeste, et au moins 15 cas de
filtre invalides.

```bash
# 4. Invariants de la phase
cargo test -p crible-core --test inv_01_iid_not_serializable \
  --test inv_16_immutable_settings
```
Attendu : **21 tests passés**, 0 échoué (3 pour INV-01, 18 pour INV-16).

```bash
# 5. Harnais Python de MES-00, hors production
cd tools && uv run pytest -q
```
Attendu : `73 passed`, 0 échoué, sans accès réseau.

```bash
# 6. MES-00 consignée et cibles écrites
cd /Users/mattboll/MattCorp/crible
ls -l docs/mesures/MES-00-reference-pgvector.md
sed -n '23,35p' docs/suivi.md
```
Attendu : le rapport existe ; la section « Cibles » affiche le rappel ≥ 0,482,
les seuils par corpus 0,290 / 0,750, la latence p95 ≤ 698 ms et la fidélité
binaire ≥ 0,99.

## 6. Décisions (ADR) de la phase

| ADR | Titre | Statut | Bloque la validation ? |
|---|---|---|---|
| ADR-0026 | Permissions des agents : utilitaires autorisés, interdictions renforcées, mode `--auto` | Accepté (2026-10-19) | Non |
| ADR-0027 | Ajouter `serde_json` comme dépendance | Accepté (2026-10-19) | Non |
| ADR-0028 | Forme canonique de l'empreinte du modèle | Accepté (2026-10-19) | Non |
| ADR-0029 | Limites de filtre : comptage de la profondeur et des clauses | Accepté (2026-10-19) | Non |
| ADR-0030 | Refuser un nom de champ déclaré aux deux niveaux | Accepté (2026-10-19) | Non |
| ADR-0031 | Refuser une clé inconnue dans un `PATCH` de configuration | Accepté (2026-10-19) | Non |
| ADR-0032 | Accès des agents à la production Legifacile : SSH, lecture seule, informations hors git | Accepté (2026-10-19) | Non |
| ADR-0033 | L'orchestrateur tourne dans Claude Code (Opus), les agents restent dans opencode | Accepté (2026-10-20) | Non |

ADR-0021 à ADR-0025, acceptés avant l'ouverture de P0, encadrent aussi la phase
(ADR). ADR-0023 a été modifié en cours de phase (points 1 et 7 remplacés par
ADR-0033). **Aucun ADR n'est au statut `Proposé`** : la table « Décisions en
attente » de `docs/suivi.md` est vide et l'index `docs/adr/README.md` ne liste
que des `Accepté`.

## 7. Écarts, idées, risques

- **Écarts à la conception ouverts (voir `suivi.md`)** : aucun. Les cinq écarts
  de DEFINIR-CONFIG-INDEX (ADR-0027 à 0031) ont été acceptés le 2026-10-19 et
  appliqués.
- **Réserve de traçabilité sur la CI** : le workflow `.github/workflows/ci.yml`
  est présent et valide en lecture, mais **GitHub n'a exécuté aucun run**
  (`gh api repos/mattboll514/crible/actions/runs` → `total_count: 0`), car la
  branche de phase n'est pas poussée (`origin/main` s'arrête à `4bbfb9d`). Le
  critère « la CI est verte » est donc établi par l'équivalent local
  `make check`, pas par un run distant. `actionlint` est absent de la machine.
  Ce n'est pas un défaut de code : les pushes sont réservés au propriétaire.
- **Risques constatés** : aucun bloquant. La mesure MES-00 est une référence
  de production non rejouable sans accès ; ses fichiers bruts sont conservés
  dans `docs/mesures/`.
- **Points à décider par le propriétaire** :
  1. confirmer les cibles de `docs/suivi.md` (critère d'acceptation de
     MESURER-REFERENCE-PGVECTOR : « le propriétaire valide les cibles ») ;
  2. dire si l'absence de run CI distant est acceptable pour J0, ou demander un
     push de `phase/P0-fondations` (via `@git`) pour obtenir un run avant
     validation ;
  3. noter que les 3 idées de relecture MES-00 restent hors périmètre
     (`suivi.md`, section « Idées hors périmètre »).
- **Idées hors périmètre** : 11 lignes au suivi, dont l'ajout de
  `clippy::panic`, `--locked` en CI, `trybuild` pour INV-01 et la validation
  stricte des checksums en P1. Aucune ne bloque J0.

## 8. Si le jalon n'est pas validé

Les corrections demandées deviennent des tâches `CORRIGER-…` dans
`docs/phases/P0-fondations.md`, avec leurs critères d'acceptation. Candidats
immédiats, si le propriétaire les juge nécessaires :

- `CORRIGER-RUN-CI-DISTANT` : pousser la branche de phase et vérifier un run
  vert des trois jobs (`rust`, `python`, `tools`), puis consigner l'URL du run
  dans ce rapport. Critère : `gh run list` montre un run `completed/success`.
- `CORRIGER-ACTIONLINT-LOCAL` : installer `actionlint` ou l'ajouter à la CI,
  puis valider la syntaxe du workflow en local. Critère : `actionlint` rend 0.
