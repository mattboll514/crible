# Rapport de jalon J2 — Encodeur et recette

> Rédigé par l'agent `jalon` (commande `/jalon J2`). **Seul le propriétaire peut valider le jalon**, dans `docs/suivi.md`.
>
> **Mise à jour du 2026-10-24** : première version écrite au commit `ef1b7a4`. Cette version couvre `AJOUTER-ORT-CI`, ADR-0057 et les runs de CI distante ; elle remplace la première.

- **Date** : 2026-10-24
- **Branche de phase / commit** : `phase/P2-encodeur-et-recette` / `b87d9af` (`merge: AJOUTER-ORT-CI`), arbre propre, branche à jour de `origin`
- **Machine** : Mac15,12 (Apple M3, 8 cœurs, 24 Go de RAM), macOS 15.7.7 (24G720), arm64 ; Rust 1.99.1 (`cargo 1.99.1 (5f94df478 2026-08-27)`), uv 0.11.21
- **CI distante** : **verte** sur la branche de phase, run [`40119640865`](https://github.com/mattboll514/crible/actions/runs/40119640865), commit `b87d9af`, tous les jobs verts, job `rust-onnx` compris (4 min 19 s)
- **Verdict de l'agent** : `PRET-POUR-VALIDATION`

Les six points du jalon de la fiche P2 sont couverts : `crible search --query` fonctionne sur l'échantillon Legifacile, le lot au mauvais prompt est refusé par la recette et le lot correct accepté, le passage trop long est refusé, MES-04 et MES-05 sont consignées, le seuil de recette `acceptance_min_cosine = 0,999` a été accepté par le propriétaire le 2026-10-24 (ADR-0052, qui ferme Q-06), et un run de CI distante est vert sur la branche de phase, job `rust-onnx` compris (ADR-0057, ajouté au jalon le 2026-10-24). Aucune décision de conception ne bloque la validation ; les points restants sont des suites à décider (§ 7).

## 1. Tâches de la phase

`docs/suivi.md` porte 12 lignes pour P2, toutes `FAIT` : 10 tâches de la fiche P2, dont `REEXPORTER-MODELE-TRANSFORMERS-5` ajoutée à la fiche par ADR-0050, et 2 tâches hors plan (`NETTOYER-P2`, `AJOUTER-ORT-CI`). Les commits ci-dessous sont les fusions sur la branche de phase.

| Tâche | Statut dans `suivi.md` | Commit | Remarque |
|---|---|---|---|
| EXPORTER-MODELE-ONNX | FAIT | `9135f00` | Export fp32 et INT8, `model.json` atomique, révision épinglée, parité min 0,9999999999259 sur 100 textes |
| CONSTITUER-ECHANTILLON-LEGIFACILE | FAIT | `af4d2f6` | 6 lots réels (205 261 documents, 257 626 passages), 100 % des textes conformes à `src_hash`, 300 requêtes réelles |
| REEXPORTER-MODELE-TRANSFORMERS-5 | FAIT | `223dc99` | Ajoutée à la fiche par ADR-0050 ; fidélité de production min 0,9999603983 sur 600 passages |
| CHARGER-MODELE | FAIT | `bc09132` | `crible-encode::Model`, hash vérifiés, `ort` en `load-dynamic`, registre par empreinte, parité 2,98e-8 |
| ENCODER-REQUETES | FAIT | `2fd3c29` | `QueryEncoder`, cache LRU, troncature signalée, repli fp32, test de bout en bout sur `synth-small` |
| MESURER-PARITE-ENCODEUR | FAIT | `54ed6ab` | MES-04 et MES-05, recommandation fp32 pour les requêtes et seuil 0,999 |
| CONTROLER-LONGUEURS | FAIT | `438ed5c` | Contrôle 8 dans `attach`, refus `invalid_request` avec les 20 premières clés, INV-11 |
| RECETTE-IMPORT | FAIT | `acba873` | Contrôle 9, échantillon déterministe, `acceptance_failed`, INV-10 et INV-12 |
| ENCODER-DOCUMENTS-EN-LIGNE | FAIT | `6fb72a7` | `crible-store::online`, `crible ingest`, verrou de présence (ADR-0055, ADR-0056) |
| CLI-RECHERCHE-TEXTE | FAIT | `7b24483` | `crible search --query`, `--format text`, `docs/jalons/J2-demo.md` |

**Tâches hors plan.** Deux tâches sans fiche, décidées par le propriétaire le 2026-10-23 et le 2026-10-24 :

| Tâche hors plan | Fusion | Objet |
|---|---|---|
| NETTOYER-P2 | `5a995f4` | Application d'ADR-0052 (requêtes en fp32 par défaut), journal JSON, mineurs de relecture des tâches P2, correction de `tools/export_onnx.py` et de la démo |
| AJOUTER-ORT-CI | `b87d9af` | ADR-0057 : ONNX Runtime et vrai modèle mDenseOn dans la CI (job `rust-onnx`), artefact Hugging Face épinglé, `tools/fetch_model_artifact.py`, échec franc d'un test `#[ignore]` sans prérequis, et 5 idées de P2 reprises (scores fp32 de la démo figés, `tools/bench/mes04/` documenté et exécuté en CI, `precision_effective`, `revision` et `min_cosine_required` d'`export_onnx.py`, `pytest -rs`) |

Aucune tâche de la phase n'est `A-FAIRE`, `EN-COURS` ou `BLOQUE`.

## 2. Qualité

Exécuté sur `phase/P2-encodeur-et-recette`, au commit `b87d9af`, le 2026-10-24.

### 2.1 En local

| Contrôle | Commande | Résultat |
|---|---|---|
| Formatage | `cargo fmt --check` | OK, code de sortie 0 |
| Lints | `cargo clippy --all-targets -- -D warnings` | OK, 0 avertissement, code de sortie 0 |
| Tests Rust (forme CI) | `cargo test --locked --workspace` | **401 passés, 0 échoué, 20 ignorés** |
| Tests Rust avec ONNX Runtime, sans filtre | `ORT_DYLIB_PATH="$PWD/tools/.venv/lib/python3.13/site-packages/onnxruntime/capi/libonnxruntime.1.31.0.dylib" cargo test --locked --workspace -- --include-ignored` | **421 passés, 0 échoué, 0 ignoré** |
| Tests Python des outils | `cd tools && uv run --no-sync pytest -q` | **256 passés, 1 sauté**, 368,12 s |
| Tests Python du client | `cd clients/python && CRIBLE_BIN="$PWD/../../target/release/crible" uv run pytest -q` | **66 passés, 0 échoué, 0 sauté**, 0,72 s |

Précisions :

- **421 et non 420** : cette fois la commande locale ne porte **pas** `--skip`, donc `inv_12_the_legifacile_sample_passes_the_acceptance_check` tourne aussi. Il passe (plus de 60 s, attache réelle sur l'échantillon) et porte le total de 420 à 421. Le test est le seul que la CI laisse local.
- **`uv run --no-sync` dans `tools/`** : `uv sync` retirerait le groupe `export` déjà installé et priverait la démo d'`onnxruntime` et `tokenizers`. Aucun `uv sync` n'a été lancé pendant cette mise à jour.
- **Les 256 tests `tools` en local contre 247 en CI** : la différence vient du groupe `export` et des données gitignorées, absents du job `tools` ; la CI nomme ses 10 sauts par `pytest -rs` (`docs/08`, « Sauts de test »).
- **Aucune commande n'a été refusée** par les règles de l'agent, y compris la forme `ORT_DYLIB_PATH=… cargo test … -- --include-ignored`.

### 2.2 En CI distante (GitHub Actions, `mattboll514/crible`)

| Run | Branche / commit | Objet | Tous les jobs | Job `rust-onnx` | Tests |
|---|---|---|---|---|---|
| [`40118324771`](https://github.com/mattboll514/crible/actions/runs/40118324771) | `tache/AJOUTER-ORT-CI` / `a000737` | Premier run, cache froid | Verts (Rust 3 min 16 s, Tools 2 min 31 s, Python 9 s) | **4 min 20 s** | `rust-onnx` 420 passés, 0 échoué, 1 filtré ; modèle téléchargé et vérifié en ≈ 8 s |
| [`40119158468`](https://github.com/mattboll514/crible/actions/runs/40119158468) | `tache/AJOUTER-ORT-CI` / `a5a9d2e` | Cache chaud, `timeout-minutes: 20` et `pytest -rs` | Verts (Rust 4 min 35 s, Tools 2 min 40 s, Python 10 s) | **3 min 16 s** | `rust-onnx` 420 passés, 0 échoué, 1 filtré ; cache restauré en ≈ 9 s |
| [`40119640865`](https://github.com/mattboll514/crible/actions/runs/40119640865) | **`phase/P2-encodeur-et-recette` / `b87d9af`** | **Run de la branche de phase** | **Verts** (Rust 3 min 30 s, Tools 2 min 29 s, Python 8 s) | **4 min 19 s** | `rust-onnx` **420 passés, 0 échoué, 1 filtré** ; `tools` **247 passés, 10 sauts justifiés** |

Le job `rust-onnx` exécute `cargo test --locked --workspace -- --include-ignored --skip inv_12_the_legifacile_sample_passes_the_acceptance_check`. Le test filtré est le seul maintenu local : ses données dérivent de la production Legifacile (ADR-0032). `docs/08` impose de le nommer et de le justifier, ce que fait la CI.

Annotations du run de phase, sans effet sur le verdict : une course d'enregistrement du cache `setup-uv` (« another job may be creating this cache »), la dépréciation annoncée de Node.js 20 par GitHub, et le futur passage de `ubuntu-latest` à Ubuntu 26. Aucune n'échoue un job.

## 3. Invariants de la phase

Les fiches de P2 citent INV-10 (RECETTE-IMPORT, rejeu du rapport de recette), INV-11 (REEXPORTER-MODELE-TRANSFORMERS-5, CONTROLER-LONGUEURS) et INV-12 (REEXPORTER-MODELE-TRANSFORMERS-5, RECETTE-IMPORT). Tous ont un test nommé. Depuis ADR-0057, **19 des 20 tests qui demandent ONNX Runtime tournent en CI** ; seul le test d'échantillon reste local.

| Invariant | Test(s) | Résultat |
|---|---|---|
| INV-10 — rejouer un lot (même `batch_id`) ne fait rien | `crates/crible-store/tests/attacher_lot.rs` : `inv_10_replaying_an_applied_batch_changes_nothing`, `inv_10_same_batch_id_with_another_manifest_is_a_batch_conflict` (en CI) ; `crates/crible/tests/recette_import.rs` : `inv_10_a_replayed_onnx_attach_returns_the_acceptance_statistics` (ONNX Runtime) | **3 passés, 0 échoué** ; les 3 tournent désormais en CI |
| INV-11 — aucun passage tronqué en silence | `crates/crible/tests/controler_longueurs.rs`, 7 tests `inv_11_*` : rejet sans effet, rejet après un lot accepté, limite exacte 32/33 prompt compris, 20 clés ordonnées, troncature déclarée ignorée (deux variantes), lot à `max_tokens − 1` accepté | **7 passés, 0 échoué** ; les 7 en CI (4 dans `rust`, 3 dans `rust-onnx`) |
| INV-12 — un lot incompatible est refusé | `crates/crible/tests/recette_import.rs`, 6 tests `inv_12_*` : lot au prompt accepté avec distribution, lot sans prompt refusé `acceptance_failed`, autre modèle refusé `incompatible_model`, modèle `external` sans recette, échantillon déterministe par `batch_id`, échantillon Legifacile accepté | **6 passés, 0 échoué** ; 5 en CI, `inv_12_the_legifacile_sample_passes_the_acceptance_check` en local seulement (ADR-0032), vérifié dans la série à 421 tests |

Vérifié aussi pendant la campagne : le refus au-delà de `max_tokens` côté export est couvert par `tools/tests/test_export_onnx.py` (`test_encode_tokens_applies_the_prompt_and_the_limit`) et `tools/tests/test_reexport_transformers5.py`, verts dans les 256 tests `tools`, et la démo rejoue INV-11 et INV-12 de bout en bout par le CLI (§ 5). La règle « Sauts de test » de `docs/08` est appliquée : `recette_import.rs` échoue désormais franchement quand son prérequis manque, prouvé par `inv_12_the_legifacile_sample_test_fails_without_its_prerequisites`.

## 4. Mesures de la phase

`docs/08-invariants-et-tests.md` classe en P2 seulement MES-04 et MES-05 (MES-05 est aussi une mesure de P8). Aucune autre mesure n'est attendue pour cette phase. Les rapports sont inchangés depuis la première version de ce rapport.

| Mesure | Rapport | Chiffre clé | Cible | Atteinte ? |
|---|---|---|---|---|
| MES-04 — parité de l'encodeur et rappel | `docs/mesures/MES-04-parite-encodeur.md` (campagne du 2026-10-24, commit `2fd3c29`) | fp32 ONNX contre PyTorch, 300 requêtes : min **0,999999999989**, médiane 0,999999999998. INT8 contre PyTorch : min **0,920358**, moyenne 0,948764, rappel@10 exact **0,7920**. Fidélité documentaire (600 passages) : min **0,9999604**, p1 0,9999935, médiane 0,9999983 | `docs/08` : fidélité ≥ 0,99 (règle visant le scan binaire), et parité d'export ≥ 0,9999 | **fp32 : oui.** INT8 pour les requêtes : **non** (0,792 de rappel@10), d'où la recommandation fp32, acceptée par ADR-0052 |
| MES-05 — latence d'encodage (natif, CPU du Mac) | `docs/mesures/MES-05-latence-encodage.md` (même campagne) | INT8 : p50 **8,6 à 8,8 ms**, p95 **22,5 à 23,0 ms**. fp32 : p50 **21,0 ms**, p95 **55,8 à 56,4 ms**. Défauts d'ONNX Runtime meilleurs que 1, 4 et 8 threads | `docs/03` : encodage de la requête 20 à 50 ms | **INT8 : oui.** fp32 : p50 dans le budget, p95 **dépassé d'environ 13 %**. Recommandation : aucun réglage de threads |

Recommandations de la campagne, toutes acceptées par ADR-0052 le 2026-10-24 : **requêtes en fp32**, `acceptance_min_cosine = 0,999` (Q-06 fermée), aucun réglage de threads. L'application du premier point au code (défaut `DEFAULT_QUERY_PRECISION = Fp32`) est faite par NETTOYER-P2.

## 5. Démo à rejouer par le propriétaire

La démo de `docs/jalons/J2-demo.md` a été **rejouée intégralement, bloc par bloc, depuis son préambule**, le 2026-10-24 sur cette branche (`b87d9af`). Les commandes ci-dessous sont celles du fichier, à copier-coller depuis la racine du dépôt. Elles ne touchent que `data/indexes/dole` et `data/j2-demo`, comme le texte le dit.

**Préambule — état propre et binaire.**

```sh
cd /Users/mattboll/MattCorp/crible
rm -rf data/indexes/dole data/j2-demo
cargo build --release
cd tools && uv sync --group export && cd ..
export ORT_DYLIB_PATH="$(cd tools && uv run python -c 'import onnxruntime, pathlib; print(next((pathlib.Path(onnxruntime.__file__).parent / "capi").glob("libonnxruntime*")))')"
```

Attendu : `cargo build --release` se termine par `Finished` ; la dernière commande n'affiche rien. **Observé** : `Finished \`release\` profile [optimized] target(s) in 0.30s`, puis `ORT_DYLIB_PATH=/Users/mattboll/MattCorp/crible/tools/.venv/lib/python3.13/site-packages/onnxruntime/capi/libonnxruntime.1.31.0.dylib`. Le rejeu de cette mise à jour n'a pas lancé `uv sync --group export` : le groupe `export` était déjà installé, et `uv run --no-sync` a été employé pour ne pas le retirer.

**Étape 1 — construire les deux lots fautifs.**

```sh
cd tools
uv run python demo_j2_batches.py --sample ../data/legifacile-sample --models ../data/models --out ../data/j2-demo
cd ..
```

Attendu et **observé à l'identique** (35,1 s) :

```text
j2-wrong-prompt: 5 documents, 20 passages -> ../data/j2-demo/j2-wrong-prompt
  cosine with the 'document: ' prompt: min 0.7323, median 0.8166; the acceptance threshold is 0.999
j2-too-long: 1 document, 1 passage, 9197 tokens with the prompt (max_tokens 8192) -> ../data/j2-demo/j2-too-long
```

**Étape 2 — créer l'index.**

```sh
./target/release/crible index create --data data --config data/legifacile-sample/configs/dole.json
```

Attendu — **observé à l'identique**, code de sortie 0 :

```text
index 'dole' created at data/indexes/dole (epoch 1, generation 0)
```

**Étape 3 — le lot corrompu est refusé par la recette.**

```sh
./target/release/crible attach --data data --models data/models --index dole data/j2-demo/j2-wrong-prompt
```

Attendu — **observé à l'identique**, code de sortie 1, rien sur la sortie standard :

```text
acceptance_failed: batch 'j2-wrong-prompt' is refused: the acceptance check re-encoded 20 passage(s) in fp32 and got min cosine 0.732275, below acceptance_min_cosine = 0.999000 (INV-12); p1 = 0.732275, p50 = 0.813774
```

```sh
./target/release/crible inspect --data data --index dole
```

Attendu — **observé à l'identique** :

```text
index dole
  epoch: 1
  generation: 0
  documents: 0
  passages: 0 live, 0 dead
  dead ratio: 0.0000
  segments: 0
  bitmaps: 0 bytes (live 0, keywords 0, booleans 0, columns 0)
```

**Étape 4 — le lot correct est accepté.**

```sh
./target/release/crible attach --data data --models data/models --index dole data/legifacile-sample/batches/legifacile-sample-dole-v1
```

Attendu — **observé à l'identique**, code de sortie 0, 2 min 22 s sur cette machine :

```text
batch 'legifacile-sample-dole-v1' attached to index 'dole': generation 1 (added 3593, replaced 0, unchanged 0, metadata_updated 0, deleted 0, ignored 0)
  acceptance: 200 passage(s) re-encoded, min cosine 0.999991, p1 0.999994, p50 0.999998
```

**Étape 5 — le passage trop long est refusé.**

```sh
./target/release/crible attach --data data --models data/models --index dole data/j2-demo/j2-too-long
```

Attendu — **observé à l'identique**, code de sortie 1 :

```text
invalid_request: batch 'j2-too-long' is refused: 1 passage(s) exceed max_tokens = 8192 tokens, prompt included (INV-11); the first 1 keys, in the order of the batch rows, are: J2-TOO-LONG#0
```

**Étape 6 — état de l'instance.**

```sh
./target/release/crible inspect --data data --index dole
```

Attendu — **observé à l'identique** :

```text
index dole
  epoch: 1
  generation: 1
  documents: 3593
  passages: 7628 live, 0 dead
  dead ratio: 0.0000
  segments: 1
    seg-legifacile-sample-dole-v1: 7628 vectors
  bitmaps: 100381 bytes (live 8208, keywords 22941, booleans 0, columns 69232)
```

**Étape 7 — recherche en texte.**

```sh
./target/release/crible search --data data --index dole --models data/models \
    --query "Quel dossier législatif correspond à la loi de 2024 pour contrôler l'immigration et améliorer l'intégration ?" \
    --k 3 --format text
```

Attendu : code de sortie 0, top-1 `JORFDOLE000047079143#0`, champs et extraits de 160 caractères au plus. **Observé à l'identique**, y compris les **trois scores désormais figés en fp32** par `AJOUTER-ORT-CI` : 0,623664, 0,547399 et 0,529660.

```text
index 'dole': 7628 passages in the pool, generation 1, 3 result(s), 1873.1 ms
1. JORFDOLE000047079143#0  score 0.623664 (cosine)
   fields: {"annee":2024,"type":"LOI_PUBLIEE"}
   text: LOI n° 2024-42 du 26 janvier 2024 pour contrôler l'immigration, améliorer l'intégration
2. JORFDOLE000051903428#0  score 0.547399 (cosine)
   fields: {"annee":2025,"type":"ORDONNANCE_PUBLIEE"}
   text: Ordonnance n° 2025-646 du 16 juillet 2025 portant extension et adaptation des dispositions de la loi n° 2024-42 du 26 janvier 2024 pour contrôler l'immigration,…
3. JORFDOLE000047079143#3  score 0.529660 (cosine)
   fields: {"annee":2024,"type":"LOI_PUBLIEE"}
   text: La loi d’orientation et du programmation du ministère de l’intérieur3 a acté une hausse du budget pour permettre d’atteindre l’objectif de 3 000 places de réten…
```

```sh
./target/release/crible search --data data --index dole --models data/models \
    --query "Quel dossier législatif correspond à la loi sur l'immigration de 2024 ?" \
    --k 1
```

Attendu : un objet JSON avec `pool_size: 7628`, `generation: 1`, `truncated_query: false`, `score_type: "cosine"` et le texte complet. **Observé à l'identique** : top-1 `JORFDOLE000047079143#0`, score 0,5479894876480103, `took_ms` 1 870,92 ms, texte de 87 caractères (non tronqué).

**Nettoyage.**

```sh
rm -rf data/indexes/dole data/j2-demo
```

L'index `dole` et les deux lots fautifs sont recréés par la démo ; les autres index de `data/indexes/` et `data/legifacile-sample/` ne sont pas touchés.

**Bilan de la démo** : tous les blocs passent, toutes les sorties annoncées sont reproduites, y compris le minimum de recette 0,999991, le refus `acceptance_failed` à 0,732275, le refus `invalid_request` du passage de 9 197 tokens, l'état inchangé de l'index après chaque refus et le top-1 attendu des deux recherches. Seules varient les valeurs que le texte annonce comme variables (`took_ms`, `took_ms` du JSON). **Aucun écart.**

## 6. Décisions (ADR) de la phase

ADR créés ou modifiés entre la fusion de P1 (`1e82ae0`) et `b87d9af`, d'après `git log -- docs/adr/` et l'index `docs/adr/README.md` :

| ADR | Titre | Statut | Bloque la validation ? |
|---|---|---|---|
| ADR-0049 | Dépendances du groupe `export` de `tools/` nommées dans `docs/07` | Remplacé par ADR-0050 | Non |
| ADR-0050 | Référence mDenseOn sous transformers 5.14.1 et ré-export de l'ONNX | Accepté (2026-10-23, proposé et accepté) | Non |
| ADR-0051 | Cache des requêtes et sessions INT8 et fp32 de l'encodeur | Accepté (décidé le 2026-10-24 ; point 3 remplacé par ADR-0052) | Non |
| ADR-0052 | Requêtes encodées en fp32 et seuil de recette à 0,999 | Accepté (2026-10-24, proposé et accepté) | Non ; ferme Q-06 et satisfait le critère « seuil de recette validé par le propriétaire » |
| ADR-0053 | Encodeur dans le chemin d'attache : arête `crible-store` → `crible-encode` | Accepté (2026-10-24) | Non |
| ADR-0054 | Chargement différé du modèle à l'attache et bornes de la recette | Accepté (2026-10-24) | Non |
| ADR-0055 | Lot en ligne : fichier d'entrée, identité, empreinte des documents et lot interne | Accepté (2026-10-24 ; décision 4 remplacée par ADR-0056) | Non |
| ADR-0056 | Verrou de présence des lots en ligne | Accepté (2026-10-24) | Non |
| ADR-0057 | ONNX Runtime et vrai modèle mDenseOn en intégration continue | Accepté (2026-10-24, proposé, révisé puis accepté) | Non ; satisfait le sixième critère du jalon et appliqué par AJOUTER-ORT-CI |

**Aucun ADR n'est au statut `Proposé`.** La table « Décisions en attente » de `docs/suivi.md` est vide et l'index de `docs/adr/README.md` ne liste que des statuts terminaux pour P2. Aucune décision d'ADR ne bloque J2.

ADR-0057 a modifié `docs/07`, `docs/08` (règle « Sauts de test ») et la section « Jalon » de la fiche P2. L'artefact du modèle est hébergé sur Hugging Face, dépôt public `mattboll/mdenseon-onnx`, épinglé au commit `491bbcd6cb1b0a7a97995a5d767ab8f5270a7b84`, avec les blake3 complets dans `ci/model-artifact.toml`.

## 7. Écarts, idées, risques

- **Écarts à la conception** : aucun écart ouvert pour P2. Les neuf écarts constatés pendant la phase (ADR-0050 à ADR-0057, plus ADR-0049 remplacé) sont tous acceptés et consignés dans la table « Écarts à la conception » de `docs/suivi.md`. Les fiches de P2 ne citent aucun invariant ou mesure sans rapport dans `docs/08`.
- **CI distante — réglé.** Les trois runs sont verts (§ 2.2), dont celui de la branche de phase au commit `b87d9af`, job `rust-onnx` compris. Les deux risques de la première version de ce rapport (CI sans ONNX Runtime, aucun run distant) sont **fermés**.
- **Risque — dépendance à un artefact externe.** Le job `rust-onnx` télécharge le vrai modèle depuis Hugging Face à chaque run sans cache. Le dépôt est public, épinglé par sha et vérifié par trois blake3 ; une indisponibilité de Hugging Face ferait échouer le job, sans toucher la branche ni les autres jobs. Le cache GitHub Actions absorbe le cas courant (3 min 16 s à chaud contre 4 min 20 s à froid).
- **Risque — un test reste local.** `inv_12_the_legifacile_sample_passes_the_acceptance_check` ne tourne qu'en local, sur les données dérivées de la production (ADR-0032). Il a été rejoué dans la série à 421 tests et passe. Une régression sur ce seul chemin ne serait vue qu'au prochain passage local, pas à la fusion.
- **Incident de confidentialité, à trancher par un ADR.** `@git` a laissé des valeurs de `.env.prod` dans un fichier temporaire d'opencode (supprimé par le propriétaire), reproduit le 2026-10-24 au commit `a000737`. C'est l'idée `ATTACHER-LOT` de « Idées hors périmètre ». La correction des règles de `@git` demande un ADR (comme ADR-0026) : **décision du propriétaire**.
- **Idées de P2 triées.** Les idées reprises par `AJOUTER-ORT-CI` sont closes dans `docs/suivi.md` (scores fp32 figés, `tools/bench/mes04/` documenté et exécuté, `precision_effective`, `revision` et `min_cosine_required` d'`export_onnx.py`) et la question de la CI sans ONNX Runtime est tranchée par ADR-0057. Restent ouvertes, gardées pour les phases suivantes : `crible search` recharge la session ONNX à chaque appel (sans objet en P6) ; la route P6 devra passer son `batch_id` à `crible-store::online` sans second chemin (ADR-0055) ; `truncated` à renommer `truncated_query` et chargeur de `crible.toml` en P6 ; requantification INT8 comme piste d'ADR-0052 ; rejouer MES-04 si une déduplication par contenu est ajoutée ; scripts de l'échantillon dans le clone Legifacile ; `RUST_LOG` honoré en P6 (feature `env-filter`, ADR à ouvrir) ; `QueryEncoder` qui fige `model.json` pour sa durée de vie ; rééquilibrage du jeu de requêtes et inclusion des attendus pour `CONSTITUER-JEU-DE-REFERENCE` (P9) ; les règles de `@relecteur` qui refusent la forme `ORT_DYLIB_PATH=… cargo test` (moins pressant depuis ADR-0057, puisque la CI rejoue ces tests à chaque push).
- **Mesure à ne pas confondre** : la règle de fidélité 0,99 de `docs/08` vise le scan binaire (P4), pas l'encodage des requêtes. MES-04 la cite comme ordre de grandeur pour condamner l'INT8 des requêtes ; ce n'est pas un critère manqué du jalon.
- **Points à décider par le propriétaire** :
  1. la validation du jalon lui-même, par `VALIDE` dans `docs/suivi.md` ;
  2. la rédaction d'un ADR pour durcir les règles de `@git` après la fuite de `.env.prod` dans un fichier temporaire ;
  3. la reprise des idées ouvertes ci-dessus dans les phases suivantes, ou leur classement sans suite.

## 8. Si le jalon n'est pas validé

Aucun manque bloquant n'est constaté : les 12 tâches de P2 sont `FAIT`, les contrôles locaux et les trois runs de CI sont verts, les trois invariants et les deux mesures sont prouvés, et la démo se rejoue sans écart. Si le propriétaire refuse malgré tout la validation, les points restants deviennent des tâches `CORRIGER-…` dans `docs/phases/P2-encodeur-et-recette.md` :

- `CORRIGER-DEMO-J2` (si un écart de démo est constaté au rejeu par le propriétaire) : corriger `docs/jalons/J2-demo.md` et rejouer ; critère : chaque sortie du fichier est reproduite, hors valeurs annoncées variables.
- `CORRIGER-CI-ONNX` (si le run de phase se dégrade) : rétablir le job `rust-onnx` ; critère : les 19 tests ONNX Runtime rejoués en CI passent, et le job reste sous 20 min.
- `CORRIGER-IDEES-P2` (si le propriétaire veut vider la liste) : traiter les idées ouvertes ci-dessus, une tâche par point ; critère : chaque idée est soit appliquée avec son test, soit close par une décision inscrite dans `docs/suivi.md`.
