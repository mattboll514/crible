# Rapport de jalon J10 — Validation à l'échelle réelle (fin de la v1)

> Rédigé par l'agent `jalon` (commande `/jalon J10`). **Seul le propriétaire peut valider le jalon**, dans `docs/suivi.md`.

- **Date** : 2026-11-08
- **Branche de phase / commit** : `phase/P10-validation-a-l-echelle-reelle` / `8558f55` (arbre propre)
- **Machine** : MacBook Air M3 (`Mac15,12`), Apple M3 à 8 cœurs, 24 Gio, macOS 15.7.7 (24G720), cargo 1.99.1, rustc 1.99.1 ; Docker Desktop 28.5.1, VM `linux/aarch64` à 8 cœurs et 11,67 Gio
- **Verdict de l'agent** : `PRET-POUR-VALIDATION`

Les cinq premiers critères du jalon (`docs/phases/P10-validation-a-l-echelle-reelle.md:5-14`) sont tenus et vérifiables sur le poste de développement ; le sixième est le bilan `docs/jalons/J10-bilan.md`, **écrit et soumis au propriétaire**, dont la validation et le tag de la v1 restent ses gestes. Les 11 tâches de P10 sont `FAIT`. Aucun ADR n'est resté `Proposé` : les sept ADR de la phase et le complément d'ADR-0113 sont `Accepté`. Aucun test n'a échoué : 1 250 tests Rust en mode debug, 911 avec la feature `failpoints`, 603 tests Python, 0 échec. Les tests d'invariant qui demandent ONNX Runtime, Docker ou l'échantillon Legifacile ont été lancés en `--release` avec `--include-ignored` et sont verts.

## 1. Tâches de la phase

Les 11 tâches suivies dans `docs/suivi.md` (lignes `| P10 |`) sont `FAIT` ; aucune n'est `A-FAIRE`, `EN-COURS` ou `BLOQUE`.

| Tâche | Statut dans `suivi.md` | Commit | Remarque |
|---|---|---|---|
| DEFINIR-INDEX-LEGIFACILE | FAIT | fusion `44f9c6d` | Six configs dans `docs/legifacile/configs/` ; Q-03 tranchée par ADR-0137 ; **six configs validées par le propriétaire le 2026-11-04** (critère 1) |
| EXPORTER-BASE-LEGIFACILE | FAIT | fusion `7b7fa31` | Six lots `replace_all`, 9 600 105 passages, 100 % des textes conformes à `src_hash` ; INV-11 et INV-12 prouvés à l'attache |
| CORRIGER-BLOCAGE-APRES-COMMIT | FAIT | fusion `5037216` | Hors plan ; diagnostic puis correction du coût quadratique d'`after_commit` ; INV-07 et INV-04 |
| CORRIGER-MEMOIRE-APRES-COMMIT | FAIT | fusion `e059fdf` | Hors plan ; cache d'un seul document ; pic d'`articles` de 8,24 à 6,54 Go (−20,7 %) |
| IMPORTER-BASE-COMPLETE | FAIT | fusion `e1daa7d` | **6 660 239 documents, 9 600 105 passages, comptes égaux à pgvector** ; MES-07 et MES-09 à l'échelle ; six instantanés |
| CONSTITUER-JEU-DE-REFERENCE | FAIT | fusion `3431e34` | 97 requêtes, 97 `expected`, 35 filtres ; jeu **validé par le propriétaire le 2026-11-04** (critère 3) |
| COMPARER-AVEC-PGVECTOR | FAIT | fusion `ab176c5` | Rappel et fidélité mesurés ; ADR-0140 ; latence de la machine de développement **indicative** |
| ACTUALISER-JEU-DE-REFERENCE | FAIT | fusion `c8c4a0f` | Hors plan ; seul l'`expected` périmé de `Q04` change ; les trois cibles de rappel sont atteintes sur le jeu actualisé |
| MESURER-SUR-MAC-MINI | FAIT | fusion `190e4e3` | Latence contraignante : 79,355 ms (critère), 358,959 ms (prudent), 418,425 ms (API), contre 698 ms (critère 5) |
| NETTOYER-P10 | FAIT | fusion `8aa9097` | Hors plan ; horloge des tests, plancher d'INV-02 à 25 ms (complément d'ADR-0113), garde des épinglages de Rust |
| BILAN-V1 | FAIT | fusion `4b4ffb5` | `docs/jalons/J10-bilan.md` écrit ; **critère d'acceptation encore ouvert : la validation du bilan par le propriétaire** (critère 6) |

Les commits postérieurs à la CI verte sont documentaires : `git diff --stat 8aa9097..HEAD` ne touche que `docs/08-invariants-et-tests.md`, `docs/jalons/J10-bilan.md` et `docs/suivi.md` (3 fichiers, +353/−13). La CI distante du run `42468997328`, sur `8aa9097`, est verte sur ses quatre jobs (`Rust`, `Tools`, `Rust with ONNX Runtime`, `Python`), vérifié par `gh run view`.

## 2. Qualité

Toutes les commandes ont été lancées par `@jalon` le 2026-11-08 sur `8558f55`. Les durées incluent la compilation incrémentale (cible déjà compilée).

| Contrôle | Commande | Résultat |
|---|---|---|
| Formatage | `cargo fmt --all -- --check` | exit 0 |
| Lints | `cargo clippy --all-targets --locked -- -D warnings` | exit 0, 0,14 s (cible déjà compilée) |
| Documentation | `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace --locked` | exit 0, 3,43 s, 6 crates documentées |
| Tests Rust (debug) | `cargo test --locked -p <crate>`, crate par crate | **1 250 passés, 0 échoué, 28 ignorés** |
| Tests `failpoints` (`crible-store`) | `cargo test --locked -p crible-store --features failpoints` | **420 passés, 0 échoué, 1 ignoré** (`crash_child_entry`, processus enfant), 86 s |
| Tests `failpoints` (`crible-server`) | `cargo test --locked -p crible-server --features failpoints` | **352 passés, 0 échoué, 0 ignoré**, 90 s |
| Tests `failpoints` (`crible`) | `cargo test --locked -p crible --features failpoints` | **139 passés, 0 échoué, 18 ignorés**, 139 s |
| Tests Python du client | `cd clients/python && uv run pytest` | **107 passés, 12 désélectionnés** (`-m 'not server'`, ADR-0112), 0,38 s |
| Tests Python des outils | `cd tools && uv run --no-sync pytest -q tests` puis `uv run --no-sync pytest -q bench` | **603 passés, 1 sauté** (188 + 415), 338,26 s + 0,67 s |

Détail des tests en mode debug, crate par crate :

| Paquet | Passés | Échoués | Ignorés | Durée |
|---|---:|---:|---:|---:|
| `crible-core` | 181 | 0 | 1 | 11 s |
| `crible-encode` | 18 | 0 | 9 | 10 s |
| `crible-search` | 210 | 0 | 0 | 40 s |
| `crible-store` | 386 | 0 | 0 | 73 s |
| `crible-server` | 317 | 0 | 0 | 65 s |
| `crible` | 138 | 0 | 18 | 126 s |
| **Total** | **1 250** | **0** | **28** | — |

Le total dépasse d'un test les 1 249 relevés par `NETTOYER-P10` : le test de troncature ajouté par cette tâche. Les 28 tests ignorés sont ceux qui demandent ONNX Runtime, Docker ou l'échantillon Legifacile ; ils ont été rejoués en `--release` (section 3) et sont verts. Les 18 ignorés côté `crible` incluent les deux tests Docker. Les 12 tests `-m server` du client ne sont pas lancés ici : ils demandent `CRIBLE_BIN` et le job `Rust` de la CI les exécute.

## 3. Invariants de la phase

Les fiches de P10 citent INV-03, INV-04, INV-07, INV-11 et INV-12. S'y ajoutent les tests `inv_02_*` de `crible-server`, dont `NETTOYER-P10` a changé le seuil (`max(3 × p99, 25 ms)`, complément du 2026-11-08 à ADR-0113). Tous sont verts.

| Invariant | Test(s) | Résultat |
|---|---|---|
| **INV-03** — une génération n'est visible qu'entière | `crible-store` : `inv_03_generation_visibility.rs` (7), `attacher_lot.rs` (2), `construire_bitmaps.rs` (1), plus `inv_03_a_crashed_attach_never_exposes_half_a_generation` de `inv_04_crash_recovery.rs` sous `--features failpoints` | **11 passés, 0 échoué**, 14 s |
| **INV-03** — un `docker stop` pendant une attache laisse une génération entière | `crible/tests/docker_stop_during_attach.rs` : `inv_03_a_docker_stop_during_an_attach_leaves_one_complete_generation` | **1 passé, 0 échoué** (avec INV-04 ci-dessous : 2 passés, 13,45 s) |
| **INV-04** — au redémarrage, l'état est la dernière génération commitée | `crible-store` : les binaires `inv_04_recovery.rs`, `inv_04_crash_recovery.rs` (17 points sous `--features failpoints`), `inv_04_read_only_open.rs`, `inv_04_lexical_chain.rs`, `inv_04_lexical_attach.rs`, `inv_04_lexical_read_only.rs`, `inv_04_lexical_marker.rs`, `inv_04_after_commit_replace_all.rs` et `inv_04_after_commit_created_passages.rs` | **42 passés, 0 échoué**, 4 s |
| **INV-04** — verrou d'écrivain et `docker stop` | `crible/tests/verrou_ecrivain_par_index.rs` (2) et `docker_stop_during_attach.rs` : `inv_04_a_docker_stop_during_an_attach_leaves_the_last_committed_generation` | **3 passés, 0 échoué** |
| **INV-07** — filtre → bitmap ≡ évaluation SQL du même filtre | `crible-store` : `inv_07_after_commit_of_a_large_replace_all_equals_a_full_rebuild`, `filters_index::tests::inv_07_after_commit_does_not_clear_the_fields_it_has_just_written`, `filters_index::tests::inv_07_created_passages_keeps_at_most_one_document_in_the_cache` (3) ; `crible-search` : `inv_07_filter_bitmap_matches_sql.rs` (2), `inv_07_filter_semantics.rs` (3), `inv_07_after_commit_many_values.rs` (2), `inv_07_after_commit_created_passages.rs` (3) | **13 passés, 0 échoué**, 1 s |
| **INV-11** — longueur d'un passage contrôlée, indépendante des autres | `crible-encode` : `inv_11_the_token_count_of_a_passage_does_not_depend_on_the_other_passages`, `inv_11_the_real_model_token_count_of_the_reference_texts_is_pinned` (`--release --include-ignored`, modèle `mdenseon`) ; `crible` : `controler_longueurs.rs`, 7 `inv_11_*` (`--release --include-ignored`, ONNX Runtime) | **2 passés, 0 échoué**, 12,54 s ; **7 passés, 0 échoué**, 0,63 s |
| **INV-12** — la recette refuse un lot sous le seuil, sans effet | `crible/tests/recette_import.rs`, 8 tests `inv_12_*` et 3 tests du même binaire (`inv_10_a_replayed_onnx_attach_returns_the_acceptance_statistics`, deux tests de fixtures) en `--release --include-ignored` avec ONNX Runtime | **11 passés, 0 échoué**, 227,76 s, dont `inv_12_the_legifacile_sample_passes_the_acceptance_check` et `inv_12_the_reference_batch_acceptance_statistics_are_unchanged` sur `data/legifacile-sample` |
| **INV-02** — une recherche tient sous `max(3 × p99 du repos, 25 ms)` pendant une écriture | `crible-server --features failpoints : inv_02_reads_during_writes.rs` (4), `inv_02_dense_readers.rs` (2) | **6 passés, 0 échoué**, 0,53 s et 0,26 s |

Commandes :

```
cargo test --locked -p crible-store --features failpoints inv_03_
cargo test --locked -p crible-store inv_04_
cargo test --locked -p crible-store --features failpoints inv_04_
cargo test --locked -p crible-store inv_07_
cargo test --locked -p crible-search inv_07_
cargo test --locked -p crible-server --features failpoints inv_02_
export ORT_DYLIB_PATH="$PWD/tools/.venv/lib/python3.13/site-packages/onnxruntime/capi/libonnxruntime.1.31.0.dylib"
cargo test --locked --release -p crible-encode --test accelere_recette -- --include-ignored
cargo test --locked --release -p crible --test controler_longueurs -- --include-ignored
cargo test --locked --release -p crible --test recette_import -- --include-ignored
CRIBLE_DOCKER_IMAGE=crible:dev cargo test --locked --release -p crible --test docker_stop_during_attach -- --include-ignored --test-threads=1
```

Les deux tests Docker ont utilisé l'image `crible:dev` déjà présente sur la machine ; ils créent et arrêtent leurs propres conteneurs, et aucun conteneur n'est resté après la passe. **Aucun test d'invariant n'a été sauté** : il n'y a donc pas de commande à rejouer par le propriétaire.

## 4. Mesures de la phase

Les chiffres sont repris des rapports, sans arrondi supplémentaire. La latence n'est contraignante que sur le Mac mini en conteneur (critère 5) ; ailleurs elle est indicative (ADR-0129).

### 4.1 Critères chiffrés du jalon

| Mesure | Rapport | Chiffre clé | Cible | Atteinte ? |
|---|---|---|---|---|
| Import à l'échelle | `docs/mesures/import-legifacile-2026-11-06.md:35-36` | **6 660 239 documents, 9 600 105 passages**, 0 mort, comptes égaux à pgvector | 8,58 M passages, comptes égaux | **Oui** ; l'écart de 12 % avec la conception est expliqué et `docs/08:57` est aligné (D1, bilan) |
| Rappel global (`k = 10`, 56 questions satisfaisables) | `docs/mesures/comparaison-pgvector-2026-11-06.md:543` | **0,500000** | ≥ 0,482 | **Oui** (+0,018000) |
| Rappel `juri_texte_integral` (31 questions) | même rapport, `:544` | **0,322581** | ≥ 0,290 | **Oui** (+0,032581) |
| Rappel `textes_vigueur` (12 questions) | même rapport, `:545` | **0,750000** | ≥ 0,750 | **Oui**, sans marge |
| Fidélité binaire contre exact (97 requêtes, N = 5000) | même rapport, `:154` | **0,995876** | ≥ 0,99 | **Oui** ; le plus bas par index est `decisions` 0,991111 |
| Latence p95 sur le Mac mini en conteneur | `docs/mesures/latence-mac-mini-2026-11-07.md:129`, `:256`, `:258` | **79,355 ms** (critère d'ADR-0141, décision 4) ; **358,959 ms** (estimation prudente avec encodage) ; **418,425 ms** (lecture par l'API, cache vide) | ≤ 698 ms | **Oui** sous les trois lectures |

### 4.2 MES-xx citées par les fiches de P10

| Mesure | Rapport | Chiffre clé | Cible ou budget | Atteinte ? |
|---|---|---|---|---|
| MES-00 (référence) | `MES-00-reference-pgvector.md:11`, `:77`, `:177` | pgvector : p50 279 ms, p95 698 ms, p99 1 112 ms ; rappel@10 0,482 | référence | Sans objet |
| MES-02 — scan binaire | `MES-02-latence-scan-binaire.md:401`, `:447` | à 3,59 M passages, p95 **47,535 ms** à N = 5000 | budget indicatif 20 à 80 ms (`docs/03:154`) | Indicatif ; dans le budget |
| MES-03 — rescoring | `MES-03-latence-rescoring.md:2070`, `:2112` | à N = 5000, p95 **4,274 ms** | aucune cible | Sans objet |
| MES-04 — parité de l'encodeur | `MES-04-parite-encodeur.md:109`, `:130` | fp32 ONNX contre PyTorch : cosinus min **0,999999999989** ; INT8 : rappel@10 **0,7920** | parité ≥ 0,9999 ; recette ≥ 0,999 | **Oui** (fp32) |
| MES-05 — encodage | `MES-05-latence-encodage.md:329`, `:358-359` | fp32 p95 **279,604 ms**, INT8 p95 **84,147 ms** | budget indicatif 20 à 50 ms (INT8) | **Non** (indicatif) ; le fp32 est la précision de requête par défaut |
| MES-07 — compaction à l'échelle | `MES-07-duree-compaction.md:264-266`, `:302-304` | `decisions` **371,0 s**, `jorf` **150,5 s**, seconde passe **142,1 s** | aucune cible ; ADR-0006:37 surveille | Sans objet |
| MES-09 — RAM réelle | `MES-09-ram-reelle.md:527-531`, `:603-604` | six index ouverts : **3,54 GiB** d'anonyme en régime (plancher 1,43 GiB) ; Mac mini : 3,529 GiB au repos, `memory.current` de 9,048 à 9,196 GiB | minimum 4 Gio, confortable 12 Gio | Dimensionnement, pas un critère |
| MES-14 — coût des filtres d'intervalle | `comparaison-pgvector-2026-11-06.md:275-276` | sur l'index réel : **41,5 à 68,2 ms** ; fixtures : 101 à 194 ms | aucune cible (ADR-0046) | Sans objet |
| MES-15 — coût d'un commit de génération | `MES-15-cout-commit-generation.md:793-794` | à 8,58 M passages : p50 **89,8 ms** (S = 10), **111,1 ms** (S = 1 000) | aucune cible | Sans objet |

### 4.3 Reconstruction lexicale (à surveiller de la fiche `IMPORTER-BASE-COMPLETE`)

`docs/mesures/import-legifacile-2026-11-06.md:43` : reconstruction **1, 5, 13, 71, 127, 185 s** pour 1 022 144 487 jetons sur `dole`, `sarde`, `titres`, `jorf`, `articles`, `decisions`. **Trois index dépassent la cible de 60 s d'ADR-0047** ; le dépassement est accepté comme limite de la v1 par ADR-0142 (décision 1), sans correction et sans relèvement du budget.

### 4.4 Concordance avec le bilan

Aucun écart de chiffre n'a été trouvé entre les rapports, le suivi et `docs/jalons/J10-bilan.md` : rappel 0,500000 / 0,322581 / 0,750000, fidélité 0,995876, latence 79,355 / 358,959 / 418,425 ms, comptes 9 600 105 / 6 660 239, MES-07 371,0 / 150,5 / 142,1 s, MES-09 3,54 GiB, reconstruction lexicale 71 / 127 / 185 s. Le bilan donne en plus les trois lectures de latence et nomme le chiffre de critère, ce que le rapport de mesure justifie par le cache d'encodage (ADR-0051) : la lecture est cohérente, avec la réserve déjà écrite que 79,355 ms ne comprend presque pas l'encodage.

## 5. Démo à rejouer par le propriétaire

Il n'existe pas de `docs/jalons/J10-demo.md` : la démonstration ci-dessous est construite depuis la section « Jalon J10 » de la fiche, et **chaque commande a été exécutée le 2026-11-08** sur le poste de développement. Elle n'utilise ni SSH, ni le Mac mini, ni pgvector, ni `legifacile-sandbox`, ni la production, et n'ouvre pas `.env.prod`. Toutes les commandes sont **de lecture** : `inspect`, `search`, `stats`, `eval`, plus deux commandes de test Rust. Les sorties de la passe sont dans `data/j10-demo/` (gitignoré). Les commandes se lancent depuis la racine du dépôt.

Préalable : la cible utilise le volume `crible-p10-data` (six index importés), l'image `crible:p10` et les modèles de `data/models/`. Espace libre mesuré avant la passe : **113 Gio** sur `/`, **102 Gio** après (cible d'arrêt : 30 Gio).

### Critère 1 — les six configs d'index sont définies et validées

```
cd /Users/mattboll/MattCorp/crible
ls docs/legifacile/configs/
cargo test --locked -p crible-core --test legifacile_configs
```

Attendu : six fichiers (`articles.json`, `decisions.json`, `dole.json`, `jorf.json`, `sarde.json`, `titres.json`) ; `2 passed; 0 failed`, soit `each_legifacile_config_is_read_and_validated` et `each_legifacile_config_declares_the_documented_fields`. La validation par le propriétaire est du 2026-11-04 (suivi, tâche `DEFINIR-INDEX-LEGIFACILE`).

### Critère 2 — les passages sont importés, compte par index

```
docker run --rm -v crible-p10-data:/data crible:p10 inspect --data /data
```

Attendu, six index et 0 mort partout :

| Index | Génération | Documents | Passages vivants |
|---|---:|---:|---:|
| `articles` | 1 | 2 135 023 | 2 476 715 |
| `decisions` | 2 | 1 196 623 | 3 589 776 |
| `dole` | 1 | 3 600 | 7 653 |
| `jorf` | 3 | 1 636 145 | 1 833 500 |
| `sarde` | 1 | 210 965 | 214 578 |
| `titres` | 1 | 1 477 883 | 1 477 883 |
| **Total** | — | **6 660 239** | **9 600 105** |

Ces comptes sont **égaux à pgvector** (`docs/mesures/import-legifacile-2026-11-06.md:35-36`). `inspect` ouvre l'index en écriture, comme toute la CLI (« il exécute la reprise ») et n'a rien modifié : les générations et les segments sont ceux laissés par la campagne. MES-07 et MES-09 à l'échelle réelle sont dans `docs/mesures/import-legifacile-2026-11-06.md:41-42` (371,0 s pour `decisions`, 3,54 GiB d'anonyme) : pas de rejeu, la compaction serait une écriture.

### Critère 3 — le jeu de référence est identique pour pgvector et Crible

```
wc -l docs/legifacile/reference/*.jsonl
cargo test --locked -p crible reference_game
```

Attendu : 25 + 45 + 10 + 3 + 8 + 6 = **97 lignes**, et **97 lignes** dans `mcp.jsonl` ; `2 passed; 0 failed` pour `every_id_has_exactly_one_mcp_line_and_the_converse` et `every_reference_game_parses_and_every_filter_validates_against_its_index`. Toutes les lignes portent un `expected` et 35 portent un filtre. Le jeu a été validé par le propriétaire le 2026-11-04 et sert aux deux moteurs (critère 3).

### Critère 4 — le rappel et la fidélité à l'échelle réelle

Une recherche qui montre le moteur sur la vraie base (article 1240) :

```
docker run --rm \
  -v crible-p10-data:/data \
  -v "$PWD/data/models:/models:ro" \
  crible:p10 search --data /data --models /models --index articles \
  --query "responsabilité du fait personnel article 1240" --k 3 --format text
```

Attendu : pool de 2 476 715 passages, génération 1 ; premier résultat `LEGIARTI000032041571#0` (article 1240 du Code civil, `etat: VIGUEUR`, `date_debut: 2016-10-01`), puis l'article 1241. Durée observée : 14 307,5 ms à froid (ouverture, codes binaires et modèle), 15,2 s au total.

Rejeu de la qualité sur l'index `articles` et le jeu de référence :

```
mkdir -p data/j10-demo
docker run --rm \
  -v crible-p10-data:/data \
  -v "$PWD/data/models:/models:ro" \
  -v "$PWD/docs/legifacile/reference:/game:ro" \
  -v "$PWD/data/j10-demo:/out" \
  crible:p10 eval --data /data --index articles --queries /game/articles.jsonl \
  --models /models --modes dense --k 10 --repeat 1 --exact-threshold 0 \
  --out /out/eval-articles-$(date +%Y%m%d-%H%M%S)
```

Attendu : `json:` et `markdown:` écrits sous `data/j10-demo/`, `regression: none`. Le rapport donne `Stratégie dense du plan : binary × 25`, **fidélité recall@10 au passage 1.0000** (n = 25) et **qualité recall@k 0.4800**, MRR@k 0.2483 (n = 25, 25 requêtes avec attendus). Durée observée : 21 s. Les cibles du jalon portent sur les populations du jeu complet, que cette passe ne recalcule pas : **0,500000 sur 56 questions satisfaisables, 0,322581 sur 31 questions `juri_texte_integral`, 0,750000 sur 12 questions `textes_vigueur`**, et **0,995876 de fidélité sur les 97 requêtes**, dans `docs/mesures/comparaison-pgvector-2026-11-06.md:154` et `:543-545`. Pour les relire :

```
sed -n '150,160p' docs/mesures/comparaison-pgvector-2026-11-06.md
sed -n '535,548p' docs/mesures/comparaison-pgvector-2026-11-06.md
```

Une lecture d'exploitation, sans serveur (aucune écriture sur les index) :

```
docker run --rm -v crible-p10-data:/data crible:p10 stats --data /data
```

Attendu : fenêtre du 2026-11-06, 2 166 requêtes au total, `zero_results` moyen 0,0005, centiles par étape. Ces chiffres sont ceux laissés par la campagne d'import.

### Critère 5 — la latence p95 contraignante, Mac mini en conteneur

Pas de rejeu : la mesure est celle de `MESURER-SUR-MAC-MINI`, sur le Mac mini, et la machine de développement ne peut pas la refaire. Rapports versionnés :

```
sed -n '126,140p' docs/mesures/latence-mac-mini-2026-11-07.md
sed -n '252,262p' docs/mesures/latence-mac-mini-2026-11-07.md
```

Attendu : `79,355 ms` au plus grand p95 par index sur les 97 requêtes en texte (le `total` chaud, critère d'ADR-0141, décision 4) ; `358,959 ms` en estimation prudente avec l'encodage fp32 sans cache, `418,425 ms` par l'API à cache vide ; **les trois sous 698 ms** (MES-00). La comparaison avec pgvector est dans `docs/mesures/comparaison-pgvector-2026-11-06.md` (rappel et latence indicative), sans rejeu.

### Critère 6 — le bilan de la v1

```
sed -n '42,60p' docs/jalons/J10-bilan.md
```

Attendu : le tableau des critères de J10, avec les cinq premiers « tenu » et le sixième « écrit ; validation en attente ». **C'est au propriétaire de le valider**, puis de taguer la v1 (ADR-0129, décision 7).

### Ménage

Aucun conteneur démarré par cette passe n'est resté : toutes les commandes utilisent `docker run --rm`. Vérification :

```
docker ps --format '{{.Names}} {{.Status}}'
```

Attendu : **aucune ligne**. L'image `crible:p10`, le volume `crible-p10-data` et les conteneurs arrêtés `crible-p10-*` sont conservés ; rien n'est supprimé sous `data/`.

## 6. Décisions (ADR) de la phase

Sept ADR ont été créés pendant P10, et un ADR existant a reçu un complément. Tous sont `Accepté` : aucun ne bloque la validation.

| ADR | Titre | Statut | Bloque la validation ? |
|---|---|---|---|
| ADR-0136 | Report du tableau de bord (P9) après J10 | Accepté (2026-11-03) | Non |
| ADR-0137 | Aucun champ multivalué en v1 (réponse à Q-03) | Accepté (2026-11-04) | Non |
| ADR-0138 | Versionner le jeu de référence de P10 | Accepté (2026-11-04) | Non |
| ADR-0139 | Protocole de la campagne d'import à l'échelle réelle | Accepté (2026-11-06) | Non |
| ADR-0140 | Protocole de la comparaison avec pgvector, et son complément du 2026-11-06 | Accepté (2026-11-06) | Non |
| ADR-0141 | Protocole de MESURER-SUR-MAC-MINI et installation par les agents, et son complément du 2026-11-07 | Accepté (2026-11-07) | Non |
| ADR-0142 | Limites acceptées de la v1 : reconstruction lexicale et mémoire d'un `replace_all` | Accepté (2026-11-08) | Non |
| Complément à ADR-0113 | Plancher du critère d'INV-02 à `max(3 × p99, 25 ms)` | Accepté (2026-11-08) | Non |

`docs/adr/README.md` ne compte **aucun ADR `Proposé`** : les 142 décisions de l'index sont `Accepté` (140) ou `Remplacé` (2, ADR-0049 et ADR-0071), et la table « Décisions en attente » de `docs/suivi.md` est vide.

## 7. Écarts, idées, risques

- **Écarts à la conception ouverts (voir `suivi.md`)** : aucun écart de P10 n'est resté sans traitement. Les deux limites mesurées sont acceptées par ADR-0142 (reconstruction lexicale à 71, 127 et 185 s contre 60 s ; 0,58 GiB de marge à l'attache de `decisions`). L'écart de 12 % entre les 8,58 M passages de la conception et les 9 600 105 passages importés est expliqué (`docs/02:230`) et `docs/08:57` est aligné depuis D1.
- **Risques constatés** :
  - `textes_vigueur` est atteint **sans marge** (0,750000 contre 0,750). Toute régression future sur une seule de ces 12 questions ferait passer le critère sous sa cible.
  - La latence de 79,355 ms ne comprend presque pas l'encodage, à cause du cache (ADR-0051) : c'est pourquoi deux lectures plus sévères sont publiées. Les trois restent sous 698 ms, la plus sévère avec 279,575 ms de marge.
  - `crible inspect` reste bloqué sur un volume ouvert en écriture (idée de v2, point 1 du bilan) : une commande de lecture seule ne doit pas attendre sans fin.
- **Points à décider par le propriétaire** :
  1. **Valider `docs/jalons/J10-bilan.md`** : c'est le critère 6 du jalon et le critère d'acceptation de `BILAN-V1`. Le bilan reprend D1 à D4, déjà validées le 2026-11-08 ; la validation porte sur le document lui-même.
  2. **Taguer la v1** : le tag est posé par le propriétaire seul (ADR-0129, décision 7).
  3. **Valider J10** dans `docs/suivi.md`.

## 8. Si le jalon n'est pas validé

Les corrections demandées deviendraient des tâches `CORRIGER-…` dans la fiche de la phase, avec leurs critères d'acceptation. Aucune correction de code n'est proposée ici : tous les contrôles sont verts et aucun défaut n'a été trouvé pendant cette passe.
