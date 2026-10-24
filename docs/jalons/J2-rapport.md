# Rapport de jalon J2 — Encodeur et recette

> Rédigé par l'agent `jalon` (commande `/jalon J2`). **Seul le propriétaire peut valider le jalon**, dans `docs/suivi.md`.

- **Date** : 2026-10-24
- **Branche de phase / commit** : `phase/P2-encodeur-et-recette` / `5a995f4` (`merge: NETTOYER-P2`), arbre propre avant les contrôles
- **Machine** : Mac15,12 (Apple M3, 8 cœurs, 24 Go de RAM), macOS 15.7.7 (24G720), arm64 ; Rust 1.99.1 (`cargo 1.99.1 (5f94df478 2026-08-27)`), uv 0.11.21
- **CI distante** : **aucun run sur cette branche** (voir § 7). Toutes les preuves de ce rapport sont locales, y compris les 20 tests qui demandent ONNX Runtime.
- **Verdict de l'agent** : `PRET-POUR-VALIDATION`

Les cinq points du jalon de la fiche P2 sont couverts : `crible search --query` fonctionne sur l'échantillon Legifacile, le lot au mauvais prompt est refusé par la recette et le lot correct accepté, le passage trop long est refusé, MES-04 et MES-05 sont consignées, et le seuil de recette `acceptance_min_cosine = 0,999` a été accepté par le propriétaire le 2026-10-24 (ADR-0052, qui ferme Q-06). Une décision reste à prendre par le propriétaire : la couverture des tests ONNX Runtime en intégration continue (§ 7).

## 1. Tâches de la phase

Les 10 tâches de la fiche P2 sont `FAIT` dans `docs/suivi.md`. `REEXPORTER-MODELE-TRANSFORMERS-5` a été ajoutée à la fiche par ADR-0050. Les commits ci-dessous sont les fusions sur la branche de phase.

| Tâche | Statut dans `suivi.md` | Commit | Remarque |
|---|---|---|---|
| EXPORTER-MODELE-ONNX | FAIT | `9135f00` | Export fp32 et INT8, `model.json` atomique, révision épinglée, parité min 0,9999999999259 sur 100 textes |
| CONSTITUER-ECHANTILLON-LEGIFACILE | FAIT | `af4d2f6` | 6 lots réels (205 261 documents, 257 626 passages), 100 % des textes conformes à `src_hash`, 300 requêtes réelles |
| REEXPORTER-MODELE-TRANSFORMERS-5 | FAIT | `223dc99` | Hors plan initial, ajoutée par ADR-0050 ; fidélité de production min 0,9999603983 sur 600 passages |
| CHARGER-MODELE | FAIT | `bc09132` | `crible-encode::Model`, hash vérifiés, `ort` en `load-dynamic`, registre par empreinte, parité 2,98e-8 |
| ENCODER-REQUETES | FAIT | `2fd3c29` | `QueryEncoder`, cache LRU, troncature signalée, repli fp32, test de bout en bout sur `synth-small` |
| MESURER-PARITE-ENCODEUR | FAIT | `54ed6ab` | MES-04 et MES-05, recommandation fp32 pour les requêtes et seuil 0,999 |
| CONTROLER-LONGUEURS | FAIT | `438ed5c` | Contrôle 8 dans `attach`, refus `invalid_request` avec les 20 premières clés, INV-11 |
| RECETTE-IMPORT | FAIT | `acba873` | Contrôle 9, échantillon déterministe, `acceptance_failed`, INV-10 et INV-12 |
| ENCODER-DOCUMENTS-EN-LIGNE | FAIT | `6fb72a7` | `crible-store::online`, `crible ingest`, verrou de présence (ADR-0055, ADR-0056) |
| CLI-RECHERCHE-TEXTE | FAIT | `7b24483` | `crible search --query`, `--format text`, `docs/jalons/J2-demo.md` |

**Tâche hors plan.** Une seule tâche hors plan dans cette phase, sans fiche, décidée par le propriétaire le 2026-10-23 :

| Tâche hors plan | Fusion | Objet |
|---|---|---|
| NETTOYER-P2 | `5a995f4` | Application d'ADR-0052 (requêtes en fp32 par défaut), journal JSON, mineurs de relecture des tâches P2, correction de `tools/export_onnx.py` et de la démo |

Aucune tâche de la phase n'est `A-FAIRE`, `EN-COURS` ou `BLOQUE`.

## 2. Qualité

Exécuté sur `phase/P2-encodeur-et-recette`, au commit `5a995f4`, le 2026-10-24.

| Contrôle | Commande | Résultat |
|---|---|---|
| Formatage | `cargo fmt --check` | OK, code de sortie 0 |
| Lints | `cargo clippy --all-targets -- -D warnings` | OK, 0 avertissement, code de sortie 0 |
| Tests Rust (CI) | `cargo test --locked --workspace` | **400 passés, 0 échoué, 20 ignorés** |
| Tests Rust avec ONNX Runtime | `ORT_DYLIB_PATH="$PWD/tools/.venv/lib/python3.13/site-packages/onnxruntime/capi/libonnxruntime.1.31.0.dylib" cargo test --locked --workspace -- --include-ignored` | **420 passés, 0 échoué, 0 ignoré**, 3 min 49 s |
| Tests Python du client | `cd clients/python && CRIBLE_BIN="$PWD/../../target/release/crible" uv run pytest -q` | **66 passés, 0 échoué, 0 sauté**, 0,90 s |
| Tests Python du client, sans binaire | `cd clients/python && uv run pytest -q` | **65 passés, 1 sauté**, 0,17 s |
| Tests Python des outils | `cd tools && uv run pytest -q` | **242 passés, 1 sauté**, 323,50 s |

Précisions :

- **Le total Rust concorde avec `docs/suivi.md`** : 400 tests en CI et 420 avec ONNX Runtime. La suite complète rejoue chaque test, y compris ceux déjà passés sans ORT.
- **Durée de la suite avec ONNX Runtime** : 3 min 49 s (229 s) sur cette machine, contre environ 150 s annoncées. Aucun test n'échoue ; la différence est une durée, pas un résultat. Les tests les plus lents sont `inv_12_the_legifacile_sample_passes_the_acceptance_check` (attache réelle sur l'échantillon) et les tests d'attache qui chargent le graphe fp32 de 1,1 Gio.
- **Tests Python du client** : la ligne « 65 tests, 1 sauté » correspond au lancement **sans** `CRIBLE_BIN` ; le test `test_online_batch` est alors sauté explicitement. Avec `CRIBLE_BIN` qui pointe sur `target/release/crible`, ce test s'exécute et passe : 66 tests, aucun sauté. Les deux formes sont donc vertes, et la seconde prouve le critère « lot produit en Python » d'ENCODER-DOCUMENTS-EN-LIGNE.
- **Aucune commande n'a été refusée** par les règles de l'agent, y compris la forme `ORT_DYLIB_PATH=… cargo test … -- --include-ignored`.

## 3. Invariants de la phase

Les fiches de P2 citent INV-10 (RECETTE-IMPORT, rejeu du rapport de recette), INV-11 (REEXPORTER-MODELE-TRANSFORMERS-5, CONTROLER-LONGUEURS) et INV-12 (REEXPORTER-MODELE-TRANSFORMERS-5, RECETTE-IMPORT). Tous ont un test nommé et tous passent avec ONNX Runtime.

| Invariant | Test(s) | Résultat |
|---|---|---|
| INV-10 — rejouer un lot (même `batch_id`) ne fait rien | `crates/crible-store/tests/attacher_lot.rs` : `inv_10_replaying_an_applied_batch_changes_nothing`, `inv_10_same_batch_id_with_another_manifest_is_a_batch_conflict` (en CI) ; `crates/crible/tests/recette_import.rs` : `inv_10_a_replayed_onnx_attach_returns_the_acceptance_statistics` (ONNX Runtime) | **3 passés, 0 échoué** |
| INV-11 — aucun passage tronqué en silence | `crates/crible/tests/controler_longueurs.rs`, 7 tests `inv_11_*` : rejet sans effet, rejet après un lot accepté, limite exacte 32/33 prompt compris, 20 clés ordonnées, troncature déclarée ignorée (deux variantes), lot à `max_tokens − 1` accepté | **7 passés, 0 échoué** (4 en CI, 3 avec ONNX Runtime) |
| INV-12 — un lot incompatible est refusé | `crates/crible/tests/recette_import.rs`, 6 tests `inv_12_*` : lot au prompt accepté avec distribution, lot sans prompt refusé `acceptance_failed`, autre modèle refusé `incompatible_model`, modèle `external` sans recette, échantillon déterministe par `batch_id`, échantillon Legifacile accepté | **6 passés, 0 échoué** (2 en CI, 4 avec ONNX Runtime) |

Vérifié aussi pendant la campagne : le refus au-delà de `max_tokens` côté export est couvert par `tools/tests/test_export_onnx.py` (`test_encode_tokens_applies_the_prompt_and_the_limit`) et `tools/tests/test_reexport_transformers5.py`, verts dans les 242 tests `tools`, et la démo rejoue INV-11 et INV-12 de bout en bout par le CLI (§ 5).

## 4. Mesures de la phase

`docs/08-invariants-et-tests.md` classe en P2 seulement MES-04 et MES-05 (MES-05 est aussi une mesure de P8). Aucune autre mesure n'est attendue pour cette phase.

| Mesure | Rapport | Chiffre clé | Cible | Atteinte ? |
|---|---|---|---|---|
| MES-04 — parité de l'encodeur et rappel | `docs/mesures/MES-04-parite-encodeur.md` (campagne du 2026-10-24, commit `2fd3c29`) | fp32 ONNX contre PyTorch, 300 requêtes : min **0,999999999989**, médiane 0,999999999998. INT8 contre PyTorch : min **0,920358**, moyenne 0,948764, rappel@10 exact **0,7920**. Fidélité documentaire (600 passages) : min **0,9999604**, p1 0,9999935, médiane 0,9999983 | `docs/08` : fidélité ≥ 0,99 (règle visant le scan binaire), et parité d'export ≥ 0,9999 | **fp32 : oui.** INT8 pour les requêtes : **non** (0,792 de rappel@10), d'où la recommandation fp32, acceptée par ADR-0052 |
| MES-05 — latence d'encodage (natif, CPU du Mac) | `docs/mesures/MES-05-latence-encodage.md` (même campagne) | INT8 : p50 **8,6 à 8,8 ms**, p95 **22,5 à 23,0 ms**. fp32 : p50 **21,0 ms**, p95 **55,8 à 56,4 ms**. Défauts d'ONNX Runtime meilleurs que 1, 4 et 8 threads | `docs/03` : encodage de la requête 20 à 50 ms | **INT8 : oui.** fp32 : p50 dans le budget, p95 **dépassé d'environ 13 %**. Recommandation : aucun réglage de threads |

Recommandations de la campagne, toutes acceptées par ADR-0052 le 2026-10-24 : **requêtes en fp32**, `acceptance_min_cosine = 0,999` (Q-06 fermée), aucun réglage de threads. L'application du premier point au code (défaut `DEFAULT_QUERY_PRECISION = Fp32`) est faite par NETTOYER-P2.

## 5. Démo à rejouer par le propriétaire

La démo de `docs/jalons/J2-demo.md` a été **rejouée intégralement, bloc par bloc, depuis son préambule**, le 2026-10-24 sur cette branche. Les commandes ci-dessous sont celles du fichier, à copier-coller depuis la racine du dépôt. Elles ne touchent que `data/indexes/dole` et `data/j2-demo`, comme le texte le dit.

**Préambule — état propre et binaire.**

```sh
cd /Users/mattboll/MattCorp/crible
rm -rf data/indexes/dole data/j2-demo
cargo build --release
cd tools && uv sync --group export && cd ..
export ORT_DYLIB_PATH="$(cd tools && uv run python -c 'import onnxruntime, pathlib; print(next((pathlib.Path(onnxruntime.__file__).parent / "capi").glob("libonnxruntime*")))')"
```

Attendu : `cargo build --release` se termine par `Finished` ; la dernière commande n'affiche rien. **Observé** : `Finished \`release\` profile [optimized] target(s) in 6.07s`, puis aucune sortie.

**Étape 1 — construire les deux lots fautifs.**

```sh
cd tools
uv run python demo_j2_batches.py --sample ../data/legifacile-sample --models ../data/models --out ../data/j2-demo
cd ..
```

Attendu et **observé à l'identique** (environ 30 s) :

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

Attendu — **observé à l'identique**, code de sortie 0, 2 min 9 s sur cette machine :

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

Attendu : code de sortie 0, top-1 `JORFDOLE000047079143#0`, champs et extraits de 160 caractères au plus. **Observé à l'identique** ; le score fp32 du top-1 vaut **0,623664**, valeur annoncée par ADR-0052, et le score de l'étape 7 du fichier n'est pas figé :

```text
index 'dole': 7628 passages in the pool, generation 1, 3 result(s), 1836.7 ms
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

Attendu : un objet JSON avec `pool_size: 7628`, `generation: 1`, `truncated_query: false`, `score_type: "cosine"` et le texte complet. **Observé à l'identique** (score 0,5479894876480103, `took_ms` 1890,07 ms), top-1 `JORFDOLE000047079143#0`.

**Nettoyage.**

```sh
rm -rf data/indexes/dole data/j2-demo
```

L'index `dole` et les deux lots fautifs sont recréés par la démo ; les autres index de `data/indexes/` et `data/legifacile-sample/` ne sont pas touchés.

**Bilan de la démo** : tous les blocs passent, toutes les sorties annoncées sont reproduites, y compris le minimum de recette 0,999991, le refus `acceptance_failed` à 0,732275, le refus `invalid_request` du passage de 9 197 tokens, l'état inchangé de l'index après chaque refus et le top-1 attendu des deux recherches. Seules varient les valeurs que le texte annonce comme variables (`took_ms`).

## 6. Décisions (ADR) de la phase

ADR créés ou modifiés entre la fusion de P1 (`1e82ae0`) et `5a995f4`, d'après `git log -- docs/adr/` et l'index `docs/adr/README.md` :

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

**Aucun ADR n'est au statut `Proposé`.** La table « Décisions en attente » de `docs/suivi.md` est vide et l'index de `docs/adr/README.md` ne liste que des statuts terminaux pour P2. Aucune décision d'ADR ne bloque J2.

## 7. Écarts, idées, risques

- **Écarts à la conception** : aucun écart ouvert pour P2. Les huit écarts constatés pendant la phase (ADR-0050 à ADR-0056, plus ADR-0049 remplacé) sont tous acceptés et consignés dans la table « Écarts à la conception » de `docs/suivi.md`. Les fiches de P2 ne citent aucun invariant ou mesure sans rapport dans `docs/08`.
- **Risque constaté — aucun run de CI sur la branche de phase.** `phase/P2-encodeur-et-recette` n'a pas de branche amont et `gh run list --branch phase/P2-encodeur-et-recette` ne renvoie aucun run : la dernière CI verte date de J1. Tous les chiffres de ce rapport viennent du poste de développement (`macOS` arm64), y compris les 20 tests ONNX Runtime. Rien n'indique un échec Linux, mais la CI n'a pas encore vérifié P2. C'est un point pour `@git` avant tout push.
- **Décision à prendre par le propriétaire — ONNX Runtime en intégration continue.** La CI n'embarque pas ONNX Runtime : 20 tests sont donc `#[ignore]` et ne sont prouvés qu'en local. Ils couvrent la parité 1e-4 de CHARGER-MODELE, le registre concurrent, la recette INV-12 sur l'échantillon Legifacile, INV-11 côté ONNX Runtime et la recherche texte de bout en bout. Quatre options, avec ce que chacune couvre :
  1. **ONNX Runtime en CI depuis la roue pip, avec le modèle factice versionné** `crates/crible-encode/tests/fixtures/fake-model/` (avis de l'orchestrateur, recommandé) : couvre le chargement, la vérification des hash, le registre, le cache, la troncature, `crible search --query` de bout en bout sur `synth-small`, et le chemin d'attache avec un modèle réellement exécuté. La parité sur le vrai mDenseOn et la recette sur l'échantillon Legifacile restent locales et mesurées par MES-04 (min 0,9999604). Coût : une étape `uv sync --group export` ou une roue `onnxruntime` dans le job Rust, aucune donnée lourde.
  2. **ONNX Runtime et vrai modèle téléchargé en CI** (révision épinglée, 1,1 Gio fp32 plus 293 Mio INT8) : couvre aussi la parité 1e-4 sur le vrai graphe et la recette sur un lot réel, mais coûte le téléchargement, le cache et environ 2 min de plus par run, et introduit une dépendance réseau vers Hugging Face dans la CI.
  3. **Statu quo** : les 20 tests restent locaux, avec leur commande inscrite dans chaque message d'`#[ignore]`. Rien ne change, mais une régression sur le chemin ONNX ne serait vue qu'au prochain passage local, pas à la fusion.
  4. **Job nocturne ou déclenché à la demande** avec le vrai modèle : couvre les mêmes tests que l'option 2 sans allonger la CI de chaque commit ; la couverture n'est pas celle de la fusion.
- **Idées de P2 encore ouvertes**, notées dans « Idées hors périmètre » et non traitées ici : `crible search` recharge la session ONNX à chaque appel (sans objet en P6, où le serveur garde le modèle) ; la route HTTP de P6 devra passer son `batch_id` à `crible-store::online` sans second chemin (ADR-0055) ; `truncated` à renommer `truncated_query` et chargeur de `crible.toml` en P6 ; requantification INT8 comme piste d'ADR-0052 ; rejouer MES-04 si une déduplication par contenu est ajoutée ; scripts de l'échantillon dans le clone Legifacile ; `RUST_LOG` honoré en P6 (feature `env-filter`, ADR à ouvrir) ; `QueryEncoder` qui fige `model.json` pour sa durée de vie ; `precision_effective` à écrire aussi dans le JSON des vecteurs de `measure_queries.rs` ; `tools/bench/mes04/` absent du tableau des outils de `tools/README.md` ; les tests de `tools/bench/mes04/` qui demandent `tokenizers` sont sautés dans le job `tools` de la CI, qui n'installe pas le groupe `export` ; scores de la démo J2 désormais figeables (fp32 par défaut) ; avertissement cosmétique de `torch` ; `data/legifacile-sample/README.md` « Limite connue » périmé depuis le ré-export ; trois mineurs de relecture de CONSTITUER-ECHANTILLON-LEGIFACILE (tests absents des scripts, `CRIBRE_DEPOT` non documentée, `PROD_REPO` supposée exportée) ; deux mineurs de REEXPORTER-MODELE-TRANSFORMERS-5 (`min_cosine_required` écrit deux fois, paramètre `revision` non lu) ; règles de `@relecteur` qui refusent la forme `ORT_DYLIB_PATH=… cargo test`, donc les tests ONNX ne sont jamais rejoués à la relecture (ADR à ouvrir, comme ADR-0044).
- **Mesure à ne pas confondre** : la règle de fidélité 0,99 de `docs/08` vise le scan binaire (P4), pas l'encodage des requêtes. MES-04 la cite comme ordre de grandeur pour condamner l'INT8 des requêtes ; ce n'est pas un critère manqué du jalon.
- **Points à décider par le propriétaire** :
  1. la couverture ONNX Runtime en CI (options 1 à 4 ci-dessus) ;
  2. la reprise des idées ouvertes ci-dessus dans les phases suivantes, ou leur classement sans suite ;
  3. la validation du jalon lui-même, par `VALIDE` dans `docs/suivi.md`.

## 8. Si le jalon n'est pas validé

Aucun manque bloquant n'est constaté : les 10 tâches de la fiche et NETTOYER-P2 sont `FAIT`, les contrôles sont verts, les trois invariants et les deux mesures sont prouvés, et la démo se rejoue sans écart. Si le propriétaire refuse malgré tout la validation, les points restants deviennent des tâches `CORRIGER-…` dans `docs/phases/P2-encodeur-et-recette.md` :

- `CORRIGER-CI-ONNX` (si l'option 1 ou 2 est retenue) : installer ONNX Runtime dans la CI et y exécuter les tests aujourd'hui `#[ignore]`, avec le modèle factice versionné ; critère : les 20 tests concernés ne sont plus ignorés en CI, et le job reste sous le temps actuel plus 2 min.
- `CORRIGER-DEMO-J2` (si un écart de démo est constaté au rejeu par le propriétaire) : corriger `docs/jalons/J2-demo.md` et rejouer ; critère : chaque sortie du fichier est reproduite, hors valeurs annoncées variables.
- `CORRIGER-IDEES-P2` (si le propriétaire veut vider la liste) : traiter les idées ouvertes ci-dessus, une tâche par point ; critère : chaque idée est soit appliquée avec son test, soit close par une décision inscrite dans `docs/suivi.md`.
