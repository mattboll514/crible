# Rapport de jalon J5 — Lexical et hybride

> Rédigé par l'agent `jalon` (commande `/jalon J5`). **Seul le propriétaire peut valider le jalon**, dans `docs/suivi.md`.

- **Date** : 2026-10-29
- **Branche de phase / commit** : `phase/P5-lexical-et-hybride` / `7f0cb28` (`docs(adr): add erratum to ADR-0069 on rebuild-lexical subcommand`), arbre propre
- **Machine** : Apple M3 (8 cœurs, 24 Gio de RAM), macOS 15.7.7, arm64 ; Rust 1.99.1 (`rustc 1.99.1 (58cf8f9c3 2026-10-13)`) ; cargo 1.99.1
- **Verdict de l'agent** : `PRET-POUR-VALIDATION`, **sous réserve de la CI distante** (la branche de phase n'a jamais été poussée pour P5 ; voir § 2 et § 7)

Les cinq points du jalon de la fiche P5 sont couverts : en mode hybride, la requête « article 1240 code civil » place `LEGIARTI000032041571` au **rang 1** (et « article 1240 » au **rang 2** en `rrf`, dans le top 3 attendu) ; MES-06, MES-10 et MES-11 sont consignées ; la fusion par défaut (`rrf`) et le traitement des élisions et des identifiants (analyseur inchangé) sont tranchés par ADR-0093 et ADR-0094 ; INV-05 est vert ; INV-09 tient toujours, branche lexicale comprise. Les 8 tâches de P5 sont `FAIT`, les contrôles de qualité locaux sont verts et aucun ADR n'est `Proposé`.

## 1. Tâches de la phase

`docs/suivi.md` porte 8 lignes pour P5, toutes `FAIT` : les 7 tâches de la fiche P5 et une tâche hors plan, `NETTOYER-P5` (décision du propriétaire du 2026-10-28, sans fiche). Les commits cités sont ceux des tâches sur la branche de phase.

| Tâche | Statut dans `suivi.md` | Commit | Remarque |
|---|---|---|---|
| CONFIGURER-ANALYSEUR | FAIT | `9c23535` | Chaîne d'analyse tantivy construite depuis `lexical.language` ; 18 codes ISO 639-1, refus `invalid_request` hors table ; cinq langues sans mots vides (ADR-0086) |
| INDEXER-TANTIVY-A-L-ATTACHE | FAIT | `90d21ad` | Index tantivy par époque, étape 4 de l'attache, reprise à l'ouverture en écriture seulement, « à jour » = `M ≤ Gt ≤ G`, marqueur `meta.passage_change_generation` (ADR-0087, ADR-0088) |
| RECONSTRUIRE-LEXICAL | FAIT | `68dbeef` | Reconstruction branchée sur la compaction et job `crible rebuild-lexical` ; 5 tests `inv_09_*` de compaction |
| RECHERCHER-BM25-DANS-LE-POOL | FAIT | `a8549f5` | Branche lexicale limitée au pool, sac de termes (OU), refus `overloaded` à six messages, `Missing` vide, branche désactivée en `invalid_request` (ADR-0089) |
| FUSIONNER-HYBRIDE | FAIT | `50ad400` | `rrf` (60) et `relative_score` (min-max, α), branches en parallèle sur le même pool, `dedup` puis `group_by` après la fusion, modes CLI (ADR-0090) |
| MESURER-LEXICAL | FAIT | `ac306bd` | MES-06 : taille, reconstruction extrapolée, latence BM25 filtrée, alerte par soustraction (ADR-0091) |
| MESURER-HYBRIDE | FAIT | `79d83a5` | MES-10 et MES-11 : recall@10, MRR@10, cas 1240, variantes d'analyse, règle de décision (ADR-0092) |
| NETTOYER-P5 | FAIT | `d092016`, `c35d25f`, `7ca3e63` | Hors plan, sans fiche : trois passages de `@dev` relus `OK` au 1er tour ; corrections de code, de mesures et de documents ; erratum d'ADR-0069 en `7f0cb28` |

Aucune tâche de la phase n'est `A-FAIRE`, `EN-COURS` ou `BLOQUE`.

## 2. Qualité

Exécuté sur `phase/P5-lexical-et-hybride`, au commit `7f0cb28`, le 2026-10-29. Les lignes marquées « orchestrateur » reprennent des commandes déjà exécutées au même commit (ou au commit indiqué) et **n'ont pas été relancées** ici, pour ne pas dépasser la borne d'une commande muette de trois minutes.

| Contrôle | Commande | Résultat |
|---|---|---|
| Formatage | `cargo fmt --check` | OK, code de sortie 0 |
| Lints | `cargo clippy --all-targets -- -D warnings` | OK, 0 avertissement, 28,96 s |
| Lints sous `failpoints` | `cargo clippy -p crible-store --all-targets --features failpoints -- -D warnings` | OK, 0 avertissement, 9,53 s |
| Tests Rust (debug) | `cargo test --workspace` | **751 passés, 0 échoué, 24 ignorés** (86 lignes de résultat, doc-tests compris) |
| Tests Rust sous `failpoints` | `cargo test -p crible-store --features failpoints` | **334 passés, 0 échoué, 1 ignoré** |
| Tests Rust `--release --include-ignored` | orchestrateur : `ORT_DYLIB_PATH=… cargo test --locked --workspace --release -- --include-ignored` | **775 passés, 0 échoué, 0 ignoré**, 2 389 s au total |
| Tests Python des outils | orchestrateur : `cd tools && uv run --no-sync pytest` | **457 passés, 1 ignoré** au commit `c35d25f` |
| Tests Python du harnais MES-06 | `cd tools && uv run --no-sync pytest -q bench/mes06` | **19 passés**, 0,16 s |
| Tests Python du client | — | Non concerné : `git diff --stat main..HEAD -- clients/` est vide, P5 n'a pas touché `clients/python` |

Précisions :

- **24 ignorés** en configuration par défaut : les tests qui demandent ONNX Runtime ou les données locales de l'échantillon Legifacile. Les quatre tests de l'échantillon (ADR-0062, ADR-0063, ADR-0085) sont couverts par la série `--release --include-ignored` de l'orchestrateur, tous `ok` : `inv_08_legifacile_sample_queries_return_the_same_results_after_export_and_import` (binaire `inv_08_export_import_roundtrip`, 11 tests, 1 227,40 s), `inv_09_legifacile_sample_queries_return_the_same_results_after_compaction` (binaire `inv_09_compaction_equivalence`, 3 tests, 711,38 s), `inv_12_the_legifacile_sample_passes_the_acceptance_check` et `inv_12_the_reference_batch_acceptance_statistics_are_unchanged` (binaire de 11 tests, 331,68 s). Les tests ONNX Runtime (`recette_import`, `accelere_recette`, etc.) sont dans le total de 775.
- **1 ignoré sous `failpoints`** : `crash_child_entry`, processus enfant des tests de panne, marqué `#[ignore]` par construction.
- **Suite complète des outils** : je ne l'ai pas rejouée dans la borne de trois minutes. Une tentative a été interrompue à 280 s après 100 tests passés, sans échec (les tests de `bench/mes10` construisent de vrais index). Le seul changement Python depuis le passage de l'orchestrateur est une ligne morte retirée de `tools/bench/mes06/mes06_analyze.py` ; `bench/mes06` est vert ici (19 passés).
- **Aucun `uv sync` n'a été lancé** : `uv run --no-sync` partout.

## 3. Invariants de la phase

Les fiches de P5 citent INV-01 (RECHERCHER-BM25-DANS-LE-POOL : aucun `iid` dans les messages de refus), INV-04 (INDEXER-TANTIVY-A-L-ATTACHE), INV-05 (RECHERCHER-BM25-DANS-LE-POOL) et INV-09 (RECONSTRUIRE-LEXICAL). Chaque test nommé `inv_xx_*` a été retrouvé par `rg` et lancé par binaire, sauf les quatre tests de l'échantillon, dont le résultat vient de la série de l'orchestrateur.

| Invariant | Test(s) | Résultat |
|---|---|---|
| INV-01 — un `iid` ne sort jamais du moteur | `crates/crible-core/tests/inv_01_iid_not_serializable.rs` (3 tests `inv_01_*`) et `crates/crible-store/tests/inv_01_export_no_iid.rs` (4 tests, dont 2 `inv_01_*`) | **7 passés, 0 échoué** ; les messages de refus sans `iid` sont aussi couverts par `lexical_refusal_without_reader.rs` et `hybrid_refusal_without_reader.rs` (dans les 751) |
| INV-04 — une panne laisse une génération cohérente, lexical compris | `inv_04_lexical_attach` (3), `inv_04_lexical_marker` (7), `inv_04_lexical_read_only` (3), `inv_04_recovery` (7 sous `failpoints`, 3 en défaut) ; `inv_04_crash_recovery` (16 passés, 1 ignoré, sous `failpoints`), dont `inv_04_crash_after_lexical_commit_keeps_the_committed_generation` | **Tous verts.** En défaut : 3 + 7 + 3 + 3 = 16 passés. Sous `failpoints` : 16 + 7 passés, 1 ignoré (processus enfant) |
| INV-05 — la branche lexicale ne sort jamais du pool | `crates/crible-search/tests/inv_05_lexical_uses_the_pool.rs` (3 tests : pool partagé, pool dense = pool lexical, mort encore présent dans tantivy) ; INV-05 en `hybrid` dans `fusionner_hybride.rs` | **3 passés, 0 échoué** ; le cas hybride est vert dans la série workspace (751) |
| INV-09 — l'équivalence avant/après compaction tient, branche lexicale comprise | `crates/crible-store/tests/inv_09_lexical_compaction.rs` (5 tests) ; `crates/crible/tests/inv_09_compaction_equivalence.rs` (2 passés, 1 ignoré : 200 requêtes `synth-small` + le test de prérequis) ; test de l'échantillon repris de l'orchestrateur | **5 passés, 0 échoué** ; **2 passés, 1 ignoré**, 13,13 s ; `inv_09_legifacile_sample_queries_return_the_same_results_after_compaction` **ok**, 711,38 s |

Le jalon demande explicitement **INV-05** (vert) et la persistance d'**INV-09** avec la branche lexicale (vert, sur les trois formes : sans mort, avec morts, contre un index construit sur les seuls vivants).

## 4. Mesures de la phase

`docs/08-invariants-et-tests.md` classe en P5 MES-06, MES-10 et MES-11. Les trois rapports existent dans `docs/mesures/`.

| Mesure | Rapport | Chiffre clé | Cible | Atteinte ? |
|---|---|---|---|---|
| MES-06 — taille, construction et latence de la branche lexicale | `docs/mesures/MES-06-lexical-bm25-filtre.md` (campagne du 2026-10-28, section « Corrections du 2026-10-29 ») | index de l'échantillon (257 626 passages) : **29,8 Mo** sans positions, **62,5 Mo** avec (× 2,096), extrapolés à 8,58 M en **≈ 1 Go** (0,3 à 2,1) et **≈ 2 Go** (0,4 à 4,8) ; reconstruction **2,63 M jetons/s**, extrapolée à **325 s** contre 60 s ; p95 de la branche **≤ 1,163 ms** à tous les pools de l'échantillon (≤ 50 000), `k` = 10 et 100 ; **budget restant 565,813 ms** (corrigé par NETTOYER-P5), sensibilité 348,965 ms ; **aucune alerte sur l'échantillon** (au plus 0,21 % du reste), alerte seulement sur le pire cas synthétique (`synth-large` à `df` = 100 % : 2,1 s puis 21,6 s, 3,7 et 38,2 fois le reste) | Latence de bout en bout p95 **≤ 698 ms** (`docs/suivi.md`) ; cible de reconstruction **60 s** (ADR-0047) ; seuil d'alerte par soustraction (ADR-0091) | **Latence : oui** sur l'échantillon, et budget restant intact. **Reconstruction : non**, 325 s contre 60 s (5,4 fois) ; le propriétaire a décidé le 2026-10-29 de différer l'ADR à la première mesure réelle en P9. **Alerte synthétique** : déclarée, repli d'ADR-0007 seulement proposé, rien de codé |
| MES-10 — qualité de la fusion | `docs/mesures/MES-10-qualite-fusion.md` (campagne du 2026-10-28) | 56 questions, 258 071 passages : recall@10 / MRR@10 **dense** 0,8571 / 0,6839 (BINARY = EXACT), **lexical** 0,9107 / 0,7699, **`rrf`** 0,9107 / 0,7645, **α = 0,3** 0,9464 / 0,7812 (IC de l'écart à `rrf` [+0,0000 ; +0,0893]), α = 0,5 0,9107, α = 0,75 0,8929 ; contrôles α = 0 et α = 1 identiques au lexical et au dense (56/56) ; cas « article 1240 » : cinq variantes dans le top 3 en hybride, pire rang 2 en `rrf` | Critère du jalon : top 3 en hybride sur le cas 1240 ; aucune cible de rappel comparable (0,482 renvoyé à P9) | **Oui pour le cas 1240.** `rrf` reste le défaut : aucune borne basse n'est supérieure à zéro, l'écart d'α = 0,3 vaut deux questions sur 56 |
| MES-11 — élisions et identifiants français | `docs/mesures/MES-11-analyse-francaise.md` (campagne du 2026-10-28) | 47 questions (union élision ∪ identifiant) : V0 à V3 à égalité stricte, recall@10 lexical **0,9149**, MRR@10 **0,7826**, écart [0 ; 0] ; 56 questions en complément : 0,9107 / 0,7699 pour les quatre variantes ; V1 change 5,9 % des passages, V2 21,1 %, V3 24,5 %, sans effet mesuré ; sondes : P1 (« L. 1132-1 ») au rang lexical 4 chez V2/V3 et absent chez V0/V1, mais le contrôle négatif P3 est satisfait par le dense seul | Aucune cible ; recommandation pour Q-04 | **Sans objet comme cible** : égalité, analyseur inchangé (ADR-0094) ; V2 reste une option de produit non retenue, remesure en P9 |

Aucune mesure de la phase n'est manquante. Deux écarts de cible sont déclarés par les rapports eux-mêmes et déjà tranchés : la reconstruction à 325 s (ADR différé à P9) et l'alerte sur le pire cas synthétique (repli d'ADR-0007 seulement instruit si P9 confirme).

## 5. Démo à rejouer par le propriétaire

Aucun `docs/jalons/J5-demo.md` n'existait : les commandes ci-dessous ont été **construites puis exécutées** sur la branche de phase le 2026-10-29, dans l'ordre, et les sorties citées sont celles observées. Elles montrent le cas « article 1240 », la branche lexicale seule, les deux fusions et un rejeu d'invariant.

**Prérequis** : `data/mes10/indexes/articles/` (jeu de référence enrichi d'ADR-0092, 258 071 passages, index `articles`, lexical activé, génération 2), `data/models/`, le binaire `target/release/crible` et la bibliothèque ONNX Runtime du venv `tools/`. Tout est gitignoré et déjà présent sur cette machine.

**Aucune commande n'écrit dans `data/`.** `crible search` ouvre l'index en lecture : c'est vérifiable sur un index non certifié, où il refuse au lieu de réparer (`data/indexes/`, construit avant P5). Les commandes ci-dessous ne font que lire `data/mes10/`.

**Préambule — depuis la racine du dépôt.**

```sh
cd /Users/mattboll/MattCorp/crible
export ORT_DYLIB_PATH=$PWD/tools/.venv/lib/python3.13/site-packages/onnxruntime/capi/libonnxruntime.1.31.0.dylib
B=./target/release/crible
```

Attendu : aucune sortie. Si le binaire manque, `cargo build --release --locked` le construit (quelques minutes).

**Étape 1 — le cas du jalon : « article 1240 code civil ».**

```sh
$B search --data data/mes10 --models data/models --index articles \
    --query "article 1240 code civil" --mode hybrid --k 10 \
    --group-by document --format text | head -4
```

Attendu, et **observé** :

```text
index 'articles': 50015 passages in the pool, generation 2, 10 result(s), …
1. LEGIARTI000032041571#0  score 0.032787 (rrf)
   branches: dense #1, 0.451503 · lexical #1, 16.677507
```

`LEGIARTI000032041571` est **au rang 1**, dans les deux branches. C'est la variante V2 de MES-10.

**Étape 2 — « article 1240 » : dense, lexical, puis les deux fusions.** Le même document est attendu au rang **1** en dense, **5** en lexical, **2** en `rrf`, **2** à α = 0,3 et **1** à α = 0,5.

```sh
$B search --data data/mes10 --models data/models --index articles \
    --query "article 1240" --mode dense --k 10 --group-by document \
    --format text | grep LEGIARTI000032041571
$B search --data data/mes10 --models data/models --index articles \
    --query "article 1240" --mode lexical --k 10 --group-by document \
    --format text | grep LEGIARTI000032041571
$B search --data data/mes10 --models data/models --index articles \
    --query "article 1240" --mode hybrid --fusion rrf --k 10 --group-by document \
    --format text | grep LEGIARTI000032041571
$B search --data data/mes10 --models data/models --index articles \
    --query "article 1240" --mode hybrid --fusion relative_score --alpha 0.3 \
    --k 10 --group-by document --format text | grep LEGIARTI000032041571
$B search --data data/mes10 --models data/models --index articles \
    --query "article 1240" --mode hybrid --fusion relative_score --alpha 0.5 \
    --k 10 --group-by document --format text | grep LEGIARTI000032041571
```

Attendu, et **observé** :

```text
1. LEGIARTI000032041571#0  score 0.391698 (cosine)
5. LEGIARTI000032041571#0  score 10.841934 (bm25)
2. LEGIARTI000032041571#0  score 0.031778 (rrf)
2. LEGIARTI000032041571#0  score 0.900560 (relative)
1. LEGIARTI000032041571#0  score 0.928971 (relative)
```

Ces cinq rangs sont **identiques** à la table de MES-10 (V3 : dense 1, lexical 5, `rrf` 2, α = 0,3 → 2, α = 0,5 → 1). Le lexical seul laisse l'article au rang 5 ; la fusion le remonte. Chaque commande dure moins de 3 s à chaud (2,4 s pour la première, puis 0,2 à 1 s).

**Étape 3 — `score_type` et forme de la réponse.**

```sh
$B search --data data/mes10 --models data/models --index articles \
    --query "article 1240 code civil" --mode hybrid --k 3 --group-by document \
    --format json | head -6
```

Attendu : `"generation": 2`, `"pool_size": 50015`, puis des résultats portant `doc_id`, `fields` et `score_type`. Les modes `dense`, `lexical` et `hybrid` et les deux fusions sont les options `--mode` et `--fusion` de `crible search --help`.

**Étape 4 — rejeu des invariants du jalon.**

```sh
cargo test -p crible-search --test inv_05_lexical_uses_the_pool
cargo test -p crible --test inv_09_compaction_equivalence
```

Attendu : **3 passés, 0 échoué** (INV-05) puis **2 passés, 0 échoué, 1 ignoré**, 13 s environ. Le test ignoré est celui de l'échantillon Legifacile, déjà passé en `--release --include-ignored` (711,38 s, § 2) : le lancer à la main demande `data/legifacile-sample`, le modèle et ONNX Runtime.

**Nettoyage** : aucun. La démo ne crée ni dossier ni fichier.

**Bilan de la démo** : les quatre étapes ont été exécutées dans l'ordre, sans écart. La seule valeur variable est la durée affichée par la CLI.

## 6. Décisions (ADR) de la phase

ADR créés ou modifiés pendant P5, d'après `git log --oneline main..HEAD -- docs/adr` et l'index `docs/adr/README.md` :

| ADR | Titre | Statut | Bloque la validation ? |
|---|---|---|---|
| ADR-0086 | Domaine de `lexical.language` et chaîne d'analyse sans mots vides | Accepté (2026-10-28) | Non |
| ADR-0087 | Reprise lexicale à l'ouverture en écriture et comportement en lecture seule | Accepté (2026-10-28) | Non |
| ADR-0088 | Index lexical à jour (`M ≤ Gt ≤ G`) et lexical illisible à la reprise | Accepté (2026-10-28) | Non |
| ADR-0089 | Branche lexicale indisponible ou désactivée : messages et codes d'erreur | Accepté (2026-10-28, confirmé après acceptation anticipée) | Non |
| ADR-0090 | Fusion `relative_score` : branche constante et domaine de `alpha` | Accepté (2026-10-28, confirmé après acceptation anticipée) | Non |
| ADR-0091 | Protocole de MES-06 | Accepté (2026-10-29, après acceptation anticipée) | Non |
| ADR-0092 | Protocole de MES-10 et MES-11 | Accepté (2026-10-29, après acceptation anticipée) | Non |
| ADR-0093 | Fusion par défaut : `rrf` confirmé par MES-10 (Q-02) | Accepté (2026-10-29) | Non |
| ADR-0094 | Analyseur français inchangé après MES-11 (Q-04) | Accepté (2026-10-29) | Non |
| ADR-0069 | Reconstruction et format de `codes.bin` | Accepté (2026-10-26) ; **erratum du 2026-10-29** (`rebuild-lexical` dans la liste de `docs/07`), citation et décisions inchangées | Non |

**Aucun ADR n'est `Proposé`.** Q-02 (« fusion par défaut ») est close par ADR-0093 : `rrf` reste le défaut, aucune valeur d'α par défaut, α = 0,3 non retenu ; la question se rouvre en P9 sur le vrai jeu de référence. Q-04 (« élisions et identifiants ») est close par ADR-0094 : chaîne d'analyse d'ADR-0086 inchangée, V2 non retenue, remesure en P9 avec des questions à identifiant.

## 7. Écarts, idées, risques

- **Écarts à la conception ouverts** : aucun. Les écarts constatés pendant P5 ont tous été tranchés par un ADR `Accepté` (ADR-0086 à ADR-0092), y compris la reprise lexicale en lecture seule (ADR-0087), le patch de métadonnées (ADR-0088), les états sans commit tantivy et la branche désactivée (ADR-0089), la branche constante de `relative_score` et le domaine d'`alpha` (ADR-0090), et les protocoles de mesure (ADR-0091, ADR-0092). Les deux mineurs documentaires de la relecture sont réglés (erratum de `docs/04` et erratum d'ADR-0069).
- **Risque — reconstruction lexicale** : 325 s extrapolées contre la cible de 60 s d'ADR-0047. Le propriétaire a tranché le 2026-10-29 : un ADR à la première mesure réelle en P9, ligne « À relever » ajoutée à `IMPORTER-BASE-COMPLETE`. Aucun code livré n'est en cause.
- **Risque — branche lexicale à grand pool** : MES-06 ne mesure qu'un pire cas synthétique (`df` = 100 %), où la branche dépasse le budget restant dès 537 500 passages. Une latence réelle à 8,58 M n'est pas mesurée ; elle est renvoyée à P9 (MES-06 § 8.2), avec MES-07. Le repli d'ADR-0007 n'est instruit que si P9 confirme.
- **Risque — volume** : l'index lexical de production est estimé entre 1 et 2 Go (0,3 à 4,8 Go selon la composition), et MES-06 a laissé `data/mes06/` à 17 Go ; `data/mes10/` pèse 1,8 Go. Ces deux dossiers ne figurent pas encore dans la table « Données de mesure conservées » de `docs/suivi.md` : leur tri est à décider.
- **Portée du critère « article 1240 »** : la fiche dit « sur l'échantillon Legifacile », mais le document attendu `LEGIARTI000032041571` n'est pas dans l'échantillon brut. La mesure et la démo portent sur le jeu de référence provisoire enrichi d'ADR-0092 (`data/mes10/`), accepté par le propriétaire : c'est le seul jeu où le critère est mesurable. Le jeu reste provisoire jusqu'à `CONSTITUER-JEU-DE-REFERENCE` en P9.
- **Conséquence opérationnelle d'ADR-0088** : un index construit avant P5 et ouvert en lecture seule refuse `lexical` et `hybrid` en `overloaded` (« lexical index state is not certified for generation 1 ») ; c'est le cas des index de `data/indexes/` construits avant la colonne `M`. Une ouverture en écriture (`crible rebuild-lexical` ou tout écrivain) les certifie. Conforme aux ADR acceptés, mais à connaître pour la suite.
- **CI distante** : la branche `phase/P5-lexical-et-hybride` n'a pas été poussée (`git branch -r --contains HEAD` est vide). **La CI GitHub Actions doit tourner avant `/valider J5`**, comme pour J4, qui avait corrigé un test local exécuté en CI (ADR-0085). Je ne pousse pas.
- **Points à décider par le propriétaire** :
  1. faire tourner la CI distante sur la branche de phase, puis reporter son résultat dans ce rapport ou au journal ;
  2. décider du sort des données de mesure de P5 (`data/mes06`, 17 Go ; `data/mes10`, 1,8 Go ; index de `data/indexes/` antérieurs à ADR-0088), en complétant la table « Données de mesure conservées » ;
  3. valider le jalon lui-même, par `VALIDE` dans `docs/suivi.md`.
- **Mineurs restants** : aucun. Les points de « Contenu de NETTOYER-P5 » sont tous traités (passages A, B et C, plus l'erratum d'ADR-0069).

## 8. Si le jalon n'est pas validé

Aucun manque bloquant n'est constaté sur les tâches, les tests, les invariants, les mesures ou la démo locale. Le seul préalable non exécuté est la CI distante. Si le propriétaire refuse malgré tout la validation, les points restants deviennent des tâches `CORRIGER-…` dans `docs/phases/P5-lexical-et-hybride.md` :

- `CORRIGER-CI-J5` (si la CI distante échoue) : corriger la cause du rouge, sans désactiver ni exclure un test sans décision. Critère : le run GitHub Actions de la branche de phase est vert, `cargo test --locked --workspace`, `--features failpoints`, `cargo fmt --check` et `cargo clippy --all-targets -- -D warnings` restent verts en local, et le rapport de J5 cite le numéro de run.
- `CORRIGER-DONNEES-MESURE-P5` (facultative, documentaire) : ajouter `data/mes06/` et `data/mes10/` à la table « Données de mesure conservées » de `docs/suivi.md`, avec taille, usage et condition de suppression. Critère : les deux dossiers y figurent, la suppression reste sur instruction du propriétaire.
