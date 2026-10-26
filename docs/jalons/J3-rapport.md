# Rapport de jalon J3 — Cycle de vie

> Rédigé par l'agent `jalon` (commande `/jalon J3`). **Seul le propriétaire peut valider le jalon**, dans `docs/suivi.md`.

- **Date** : 2026-10-26
- **Branche de phase / commit** : `phase/P3-cycle-de-vie` / `b66fddf` (`merge: NETTOYER-P3`), arbre propre, branche **jamais poussée** (aucun run de CI distante pour P3)
- **Machine** : Mac15,12 (Apple M3, 8 cœurs, 24 Go de RAM), macOS 15.7.7 (24G720), arm64 ; Rust 1.99.1 (`cargo 1.99.1 (5f94df478 2026-08-27)`)
- **Verdict de l'agent** : `PRET-POUR-VALIDATION` — sous deux conditions à la main du propriétaire, décrites au § 6 : confirmer ADR-0068, et faire exister un run de CI distante avant `/valider J3`.

Les trois points du jalon de la fiche P3 sont couverts sur cette machine : trois lots quotidiens puis une compaction donnent des résultats identiques (`synth-small` bit-identique, échantillon Legifacile à 1e-6), l'aller-retour export → import donne des résultats identiques sur les deux jeux, et un arrêt brutal à chacun des 12 points d'injection est suivi d'une reprise propre. Les cinq invariants nommés par le jalon (INV-03, INV-04, INV-08, INV-09, INV-14) sont verts, MES-07, MES-08 et MES-13 sont consignées sans cible dépassée, les 13 tâches de la phase sont `FAIT` et la démo se rejoue sans écart. **Rien n'a été poussé ni validé par cet agent** ; le point ouvert est la CI distante, qui n'a jamais vu le code de P3 (§ 6).

## 1. Tâches de la phase

`docs/suivi.md` porte 13 lignes pour P3, toutes `FAIT` : 12 tâches de la fiche P3, dont `DURCIR-CONTROLE-ANTI-FUITE` (ADR-0058), `DURCIR-ARTEFACT-MODELE-EN-CI` (ADR-0059), `REUTILISER-COMPRESSEUR-ZSTD` (ADR-0048) et `BORNER-COUT-DU-COMMIT` (ADR-0066) ajoutées à la fiche hors plan initial, et 1 tâche hors fiche (`NETTOYER-P3`). Les commits ci-dessous sont les fusions sur la branche de phase.

| Tâche | Statut dans `suivi.md` | Commit | Remarque |
|---|---|---|---|
| DURCIR-CONTROLE-ANTI-FUITE | FAIT | `b762cb5` | `.opencode/scripts/check_prod_leak.sh`, 12 cas et 29 assertions ; `@git` ne lit plus `.env.prod` autrement que par ce script |
| DURCIR-ARTEFACT-MODELE-EN-CI | FAIT | `de5e2ed` | 4 essais, `Retry-After` ≤ 60 s, échéance globale de 600 s, workflow `model-cache.yml` seul écrivain du cache ; runs distants verts (CI `40183664440`, cache `40183664493`) |
| PATCHER-METADONNEES | FAIT | `5534963` | `crible-store::patch` et `crible patch`, 17 + 4 tests, ADR-0060 |
| COMPACTER-INDEX | FAIT | `7b49e7f` | `crible-store::compact` et `crible compact`, époque E+1, `batches` conservés, 11 tests `inv_14_*` et 2 `inv_02_*` |
| VERIFIER-EQUIVALENCE-COMPACTION | FAIT | `690f82a` | `inv_09_…` sur `synth-small` (scores bit-identiques) et sur l'échantillon Legifacile (ADR-0062) |
| EXPORTER-LOT | FAIT | `331c009` | `crible-store::export` et `crible export`, lot `replace_all` lu sur l'instantané, `inv_01_export_no_iid` |
| VERIFIER-ALLER-RETOUR-EXPORT | FAIT | `c891bb3` | `inv_08_…` sur `synth-small` et sur l'échantillon Legifacile (ADR-0063) |
| CREER-ET-RESTAURER-INSTANTANE | FAIT | `cd51c02` | `crible snapshot` / `crible restore`, `snapshot.json` versionné, 25 tests (ADR-0064, ADR-0065) |
| TESTER-REPRISE-APRES-PANNE | FAIT | `46c2dd1` | `inv_04_crash_recovery.rs`, 12 points d'injection, 13 tests sous `--features failpoints` |
| REUTILISER-COMPRESSEUR-ZSTD | FAIT | `d5a8ea9` | `records::TextCompressor`, niveau inchangé, gain chiffré par MES-08 |
| MESURER-CYCLE-DE-VIE | FAIT | `4230154` | MES-07, MES-08 et MES-13 consignées, plus les rejeux de MES-15 et MES-13 |
| BORNER-COUT-DU-COMMIT | FAIT | `a54b4c9` | `after_commit` reçoit le delta du commit ; commit à 8,58 M passages ramené à 89,8 ms p50 (MES-15, ADR-0066) |
| NETTOYER-P3 | FAIT | `b66fddf` | Points 1 à 6 de la section « Contenu de NETTOYER-P3 » traités ; `failpoints` en CI (ADR-0068), borne du patch à 32 000 000 octets |

Aucune tâche de la phase n'est `A-FAIRE`, `EN-COURS` ou `BLOQUE`.

## 2. Qualité

Exécuté sur `phase/P3-cycle-de-vie`, au commit `b66fddf`, le 2026-10-26. Les commandes sont celles d'`.github/workflows/ci.yml`.

| Contrôle | Commande | Résultat |
|---|---|---|
| Formatage | `cargo fmt --all -- --check` | OK, code de sortie 0 |
| Lints | `cargo clippy --all-targets --locked -- -D warnings` | OK, 0 avertissement, 13,87 s |
| Lints sous `failpoints` | `cargo clippy -p crible-store --all-targets --locked --features failpoints -- -D warnings` | OK, 0 avertissement, 5,52 s |
| Tests Rust (forme CI) | `cargo test --locked --workspace` | **502 passés, 0 échoué, 22 ignorés**, 128,77 s |
| Tests Rust sous `failpoints` | `cargo test -p crible-store --locked --features failpoints` | **266 passés, 0 échoué, 1 ignoré**, 72,59 s |
| Tests Python des outils | `cd tools && uv run --no-sync pytest -q` | **273 passés, 1 sauté**, 597,18 s |
| Tests Python du client | `cd clients/python && CRIBLE_BIN="$PWD/../../target/debug/crible" uv run --no-sync pytest -q` | **66 passés, 0 échoué, 0 sauté**, 0,48 s (65 sans `CRIBLE_BIN`, `test_online_batch.py` sauté) |
| Tests longs, avec ONNX Runtime | `ORT_DYLIB_PATH=… cargo test --locked --release -p crible --test inv_08_export_import_roundtrip --test inv_09_compaction_equivalence -- --include-ignored` | **14 passés, 0 échoué**, 1 868,81 s au total : binaire `inv_08` 1 283,17 s (11 tests), binaire `inv_09` 578,90 s (3 tests) |
| Tests longs, avec ONNX Runtime | `ORT_DYLIB_PATH=… cargo test --locked --release -p crible --test recette_import -- --include-ignored` | **10 passés, 0 échoué**, 232,83 s |

Précisions :

- **22 ignorés** : les 20 d'avant P3 plus les deux tests de l'échantillon Legifacile ajoutés par ADR-0062 (`inv_09_…`) et ADR-0063 (`inv_08_…`), tous deux exclus de la CI par leur nom et rejoués ici (§ 2, dernière ligne). Sous `failpoints`, l'unique ignoré est `crash_child_entry`, le processus enfant des tests de panne, marqué `#[ignore]` par construction — **aucun test d'invariant n'est sauté**.
- **Le défaut connu de P3 est fermé** : `attacher_lot::no_sqlite_write_follows_the_commit_point`, rouge sous `failpoints` depuis avant `COMPACTER-INDEX`, passe (point 1 de NETTOYER-P3). Aucun rouge ne subsiste sous `failpoints`.
- **Les trois tests locaux de l'échantillon Legifacile tournent** : `inv_08_legifacile_sample_queries_return_the_same_results_after_export_and_import`, `inv_09_legifacile_sample_queries_return_the_same_results_after_compaction` et `inv_12_the_legifacile_sample_passes_the_acceptance_check` sont passés en `--release` avec `ORT_DYLIB_PATH` et les données locales. Leurs durées (1 283,17 s, 578,90 s et 232,83 s de binaire) sont relevées, pas bornées (ADR-0062).
- **Aucun `uv sync` n'a été lancé** : `uv run --no-sync` partout dans `tools/`, pour ne pas retirer le groupe `export` ni la bibliothèque ONNX Runtime locale.
- **Pas de run de CI distante** : voir le § 6. Tous les chiffres ci-dessus sont locaux.
- Le total de 502 tests et celui de 266 tests sous `failpoints` concordent avec le suivi de NETTOYER-P3 (502 et 266) ; l'orchestrateur avait relevé 524 tests avec ONNX Runtime et 0 ignoré sur le workspace en `--release` le matin même, sur un commit antérieur aux corrections du tour 1.

## 3. Invariants de la phase

Les fiches de P3 citent INV-01 (EXPORTER-LOT), INV-02 (COMPACTER-INDEX, BORNER-COUT-DU-COMMIT), INV-03 (TESTER-REPRISE-APRES-PANNE, BORNER-COUT-DU-COMMIT), INV-04 (TESTER-REPRISE-APRES-PANNE), INV-08 (VERIFIER-ALLER-RETOUR-EXPORT), INV-09 (VERIFIER-EQUIVALENCE-COMPACTION) et INV-14 (COMPACTER-INDEX, CREER-ET-RESTAURER-INSTANTANE). Chacun a un test nommé `inv_xx_*`, retrouvé par `rg` et lancé ici.

| Invariant | Test(s) | Résultat |
|---|---|---|
| INV-01 — aucun iid ne sort du moteur | `crates/crible-store/tests/inv_01_export_no_iid.rs` : 4 tests (`inv_01_exported_batch_names_no_iid`, `inv_01_exported_identifiers_are_doc_ids_and_ords`, plus deux gardes) ; `crates/crible-core/tests/inv_01_iid_not_serializable.rs` (3 tests, `Iid` non sérialisable) | **4 + 3 passés, 0 échoué**, 0,22 s pour le fichier d'export |
| INV-02 — une lecture n'est jamais bloquée par une écriture | `inv_02_reads_during_compaction_complete_without_waiting`, `inv_02_payload_reads_on_held_epoch_during_compaction` (sous `failpoints`), `inv_02_commit_publishes_without_mutating_the_structures_a_reader_holds` (`construire_bitmaps`) | **2 + 1 passés, 0 échoué**, 0,21 s pour le fichier de compaction |
| INV-03 — une génération est visible entièrement ou pas du tout | `inv_03_generation_visibility.rs` (7 tests), `inv_03_a_crashed_attach_never_exposes_half_a_generation` (sous `failpoints`), `inv_03_failed_attach_leaves_the_records_database_untouched`, `inv_03_doc_id_in_documents_and_in_deletions_rejects_the_batch`, `inv_03_after_mixed_attach_and_metadata_patch_equals_a_full_rebuild` (BORNER-COUT-DU-COMMIT) | **7 passés, 0 échoué**, 0,46 s pour `inv_03_generation_visibility` ; le cas de panne est dans le fichier d'INV-04 ci-dessous |
| INV-04 — au redémarrage, l'état est la dernière génération commitée | `crates/crible-store/tests/inv_04_crash_recovery.rs` (sous `failpoints`) : 12 points d'injection (attache 1 à 6, compaction 1 à 6), `SIGKILL` du processus enfant, redémarrage, état exactement G ou G+1, rejeu jusqu'à G+1, aucun `tmp-*` ; `inv_04_recovery.rs` (4 tests) | **13 passés, 0 échoué, 1 ignoré** (`crash_child_entry`, le processus enfant), 1,02 s ; `inv_04_recovery` passe aussi (forme CI) |
| INV-08 — export puis import ⇒ mêmes résultats | `inv_08_synth_small_200_queries_return_the_same_results_after_export_and_import` et `inv_08_legifacile_sample_queries_return_the_same_results_after_export_and_import` (300 questions, ONNX Runtime), plus 8 gardes `comparison_guards` et le test anti-saut | **11 passés, 0 échoué** en `--release` avec `--include-ignored`, 1 283,17 s |
| INV-09 — compaction ⇒ mêmes résultats avant et après | `inv_09_synth_small_200_queries_return_the_same_results_after_compaction` (scores bit-identiques) et `inv_09_legifacile_sample_queries_return_the_same_results_after_compaction` (300 questions, ONNX Runtime), plus le test anti-saut | **3 passés, 0 échoué** en `--release` avec `--include-ignored`, 578,90 s |
| INV-14 — rien de ce qui a coûté du GPU n'est détruit sans copie valide | `inv_14_compaction_retention.rs` (sous `failpoints`) : 11 tests, dont `inv_14_epoch_held_by_reader_survives_compaction`, `inv_14_epoch_unchanged_and_readable_while_compaction_runs` et les deux pannes de bascule ; `inv_14_restore_rolls_back_and_keeps_the_replaced_epoch` (`instantane.rs`) | **11 passés, 0 échoué**, 1,22 s ; le test de restauration passe dans la série à 502 |

Le jalon demande explicitement INV-03, INV-04, INV-08, INV-09 et INV-14 : **les cinq sont verts**. INV-01 et INV-02, cités par les fiches, le sont aussi. Les invariants de P1 et P2 (INV-06, INV-07, INV-10, INV-11, INV-12, INV-15) passent dans les séries à 502 et 266 tests, et les trois tests longs de l'échantillon dans celle du § 2.

## 4. Mesures de la phase

`docs/08-invariants-et-tests.md` classe en P3 MES-07, MES-08 et MES-13 (`MES-15` est une mesure de P1, rejouée ici parce que `MESURER-CYCLE-DE-VIE` et `BORNER-COUT-DU-COMMIT` la citent). Les trois rapports attendus existent, avec leur JSON brut et les sections datées des corrections de NETTOYER-P3.

| Mesure | Rapport | Chiffre clé | Cible | Atteinte ? |
|---|---|---|---|---|
| MES-07 — durée d'une compaction | `docs/mesures/MES-07-duree-compaction.md` (campagne du 2026-10-25, corrections du 2026-10-26) | p50 **2 214 ms** sur `articles` (44 890 passages vivants, textes réels) et **377 924 ms** (6 min 18 s) sur `synth-large` (7 740 000 passages vivants) ; extrapolation à 8,58 M passages : **≈ 377 s** (loi directe) à **≈ 423 s** (borne haute, 10 % de morts) ; postes isolés : `passage_read` 9,6 %, `vector_read` 4,4 %, `segment_open` 2,1 % à 7,74 M, le reste (83,9 %) non décomposé | **Aucune** dans `docs/08`, `docs/03` ni `docs/04` (le seul délai écrit, `wait_timeout_ms` = 60 s, borne une attache côté client) | Sans objet : la mesure ne peut pas conclure à un dépassement. Trois recommandations sont ouvertes (§ 7) |
| MES-08 — durée d'une attache quotidienne type | `docs/mesures/MES-08-duree-attache-quotidienne.md` (campagnes du 2026-10-25 et du 2026-10-26) | `synth-large` (7 740 000 passages vivants), p50 : **63 ms** pour 1 000 passages et **143 ms** pour 3 002 le 2026-10-26, contre 3 257 et 3 333 ms le 2026-10-25 (−95,7 % à −98,1 %) ; part de l'étape `snapshot` : 15,9 % et 20,3 %, contre 96,6 % et 98,5 %. Échantillon Legifacile avec `mdenseon` : 52,3 s (1 000 passages) et 68,5 s (3 002), dont 96,3 à 98,3 % de recette. Gain du compresseur zstd réutilisé : **82,9 à 86,0 %**, soit 32,7 à 116,2 ms par lot | **Aucune** dans `docs/08`, `docs/03` ni `docs/04` | Sans objet comme durée. Constat : un lot de 3 002 passages sur l'échantillon (68,5 s) dépasse `wait_timeout_ms` = 60 s ; traité par ADR-0067 (critère ajouté à `FILE-D-ECRITURE-ET-JOBS` en P6, tâche `ACCELERER-RECETTE` en P4), pas par une mesure |
| MES-13 — temps de démarrage | `docs/mesures/MES-13-temps-de-demarrage.md` (campagne du 2026-10-25, corrections du 2026-10-26) | Extrapolation à 8,58 M passages : **16,6 s** (borne pondérée) et **18,5 s** avec 10 % de morts ; borne la plus chère **28,9 s**, **32,1 s** avec 10 % de morts (9,53 M lignes) ; part du décodage JSON **27,6 %** (lecture SQLite 39,0 %, insertion `keyword` 33,4 %) ; **1,2551** passage par document sur l'échantillon complet | **60 s** à l'échelle de Legifacile (`MESURER-CYCLE-DE-VIE`, ADR-0047) | **Oui** : 32,1 s au pire, soit 54 % du budget, marge de 1,9 à 3,6. Ni `DECODER-DOCUMENTS-UNE-FOIS` ni `AJOUTER-CACHE-BITMAPS` ne sont proposées ; ADR-0047 fixe la condition de réouverture |
| MES-15 (rejouée, P1) | `docs/mesures/MES-15-cout-commit-generation.md` (§ datés du 2026-10-25 et du 2026-10-26) | Commit de génération, p50, à 8 580 000 passages vivants : **598,3 ms** le 2026-10-21, **3 085,1 ms** le 2026-10-25, **89,8 ms** le 2026-10-26 (S = 10 ; 111,1 ms à S = 1 000) | Aucune cible ; le seuil d'alerte du § 10 de la fiche `BORNER-COUT-DU-COMMIT` est le retour sous les 598,3 ms du 2026-10-21 | **Oui** : −85,0 % contre le 2026-10-21 et −97,1 % contre le 2026-10-25 ; les deux index SQLite de recours (ADR-0066, décision 2) ne sont pas nécessaires |

Aucune mesure de la phase n'est manquante et aucune cible n'est dépassée. `data/synth-large` (28 Go) et `data/mes07/` restent dans `data/` (gitignoré), hors dépôt.

## 5. Démo à rejouer par le propriétaire

Aucun `docs/jalons/J3-demo.md` n'existait : les commandes ci-dessous ont été **construites puis exécutées** sur la branche de phase le 2026-10-26, dans l'ordre, et les sorties citées sont celles observées. Elles montrent les trois premiers points du jalon (lots quotidiens puis compaction, aller-retour export → import, reprise après panne) et les deux autres (invariants, mesures) sont renvoyés au § 3 et au § 4.

**Prérequis** : `data/synth-small/` (jeu de `tools/gen_synth.py` : les lots `l1`, `l2` et `l3` et `queries.jsonl`) ; `models/synthetic-external/model.json` (`{"name":"synthetic-external","kind":"external","dims":768,"normalize":true}`) ; le groupe `export` de `tools/`, appelé avec `--no-sync`. Tout est gitignoré et déjà présent sur cette machine.

**Préambule — état propre et binaire.**

```sh
cd /Users/mattboll/MattCorp/crible
rm -rf data/indexes/synth-small data/j3-import data/j3-exchange data/j3-broken
cargo build --release
```

Attendu : `Finished`. L'index `synth-small` est reconstruit par la démo elle-même ; les autres index de `data/indexes/` ne sont pas touchés.

**Étape 1 — trois lots quotidiens simulés.**

```sh
./target/release/crible index create --data data --config data/synth-small/index.json
./target/release/crible attach --data data --models models --index synth-small data/synth-small/l1
./target/release/crible attach --data data --models models --index synth-small data/synth-small/l2
./target/release/crible attach --data data --models models --index synth-small data/synth-small/l3
./target/release/crible inspect --data data --index synth-small
```

Attendu, dans l'ordre (observé à l'identique) :

```text
index 'synth-small' created at data/indexes/synth-small (epoch 1, generation 0)
batch 'l1' attached to index 'synth-small': generation 1 (added 2000, replaced 0, unchanged 0, metadata_updated 0, deleted 0, ignored 0)
batch 'l2' attached to index 'synth-small': generation 2 (added 100, replaced 300, unchanged 0, metadata_updated 0, deleted 0, ignored 0)
batch 'l3' attached to index 'synth-small': generation 3 (added 0, replaced 0, unchanged 0, metadata_updated 50, deleted 150, ignored 0)
index synth-small
  epoch: 1
  generation: 3
  documents: 1950
  passages: 9750 live, 2250 dead
  dead ratio: 0.1875
  segments: 2
    seg-l1: 10000 vectors
    seg-l2: 2000 vectors
  bitmaps: 671737 bytes (live 8208, keywords 88835, booleans 45446, columns 529248)
```

**Étape 2 — recherche de référence, avant toute écriture.** Le vecteur de la première requête de `queries.jsonl` est extrait une fois :

```sh
mkdir -p /tmp/j3
cd tools
uv run --no-sync python - <<'PY'
import json
from pathlib import Path

query = json.loads(
    Path("../data/synth-small/queries.jsonl").read_text(encoding="utf-8").splitlines()[0]
)
Path("/tmp/j3/query.json").write_text(json.dumps(query["vector"]), encoding="utf-8")
PY
cd ..
./target/release/crible search --data data --index synth-small --vector-file /tmp/j3/query.json --k 10 > /tmp/j3/before.json
```

Attendu : code de sortie 0 ; `generation: 3`, `pool_size: 9750`, 10 résultats dont les clés sont, dans l'ordre, `synth-01163#3`, `synth-00407#0`, `synth-01703#2`, … (celles de la première ligne d'`expected.jsonl`).

**Étape 3 — patch de métadonnées sans ré-encodage** (PATCHER-METADONNEES).

```sh
printf '%s\n' '{"doc_id":"synth-01163","fields":{"categorie":"cat-99"}}' \
              '{"doc_id":"synth-00407","fields":{"categorie":"cat-99"}}' \
              '{"doc_id":"synth-01703","fields":{"categorie":"cat-99"}}' > /tmp/j3/patch.jsonl
./target/release/crible patch --data data --index synth-small /tmp/j3/patch.jsonl
./target/release/crible search --data data --index synth-small --vector-file /tmp/j3/query.json \
    --filter '{"categorie":"cat-99"}' --k 20 > /tmp/j3/cat99.json
cd tools
uv run --no-sync python -c "import json; d=json.load(open('/tmp/j3/cat99.json')); print('generation', d['generation'], 'pool_size', d['pool_size'], 'results', len(d['results']))"
cd ..
```

Attendu (observé à l'identique) : le filtre suit la nouvelle valeur, sans qu'aucun vecteur ne change.

```text
{"generation":4,"unchanged":0,"updated":3}
generation 4 pool_size 15 results 15
```

**Étape 4 — compaction** (COMPACTER-INDEX).

```sh
./target/release/crible compact --data data --index synth-small
./target/release/crible inspect --data data --index synth-small
```

Attendu (observé à l'identique ; 0,68 s au total sur cet index) :

```text
index 'synth-small': epoch 1, generation 4, 9750 live and 2250 dead passages (dead ratio 0.1875)
index 'synth-small' compacted: epoch 1 -> 2, generation 5, 9750 live passages, 2250 removed
index synth-small
  epoch: 2
  generation: 5
  documents: 1950
  passages: 9750 live, 0 dead
  dead ratio: 0.0000
  segments: 1
    c2-0000000000: 9750 vectors
  bitmaps: 571461 bytes (live 8208, keywords 79701, booleans 44304, columns 439248)
```

**Étape 5 — mêmes résultats après la compaction** (INV-09).

```sh
./target/release/crible search --data data --index synth-small --vector-file /tmp/j3/query.json --k 10 > /tmp/j3/after-compaction.json
cd tools
uv run --no-sync python -c "
import json
a = json.load(open('/tmp/j3/before.json')); b = json.load(open('/tmp/j3/after-compaction.json'))
ka = [(r['key'], r['score']) for r in a['results']]; kb = [(r['key'], r['score']) for r in b['results']]
print('same keys/scores:', ka == kb, '| max abs diff', max(abs(x[1] - y[1]) for x, y in zip(ka, kb)))
"
cd ..
```

Attendu (observé à l'identique) :

```text
same keys/scores: True | max abs diff 0.0
```

Les scores sont **bit-identiques** ; le banc de 200 requêtes et celui de l'échantillon Legifacile sont au § 3.

**Étape 6 — aller-retour export → import** (EXPORTER-LOT, VERIFIER-ALLER-RETOUR-EXPORT). L'import se fait dans une **seconde instance**, sous `data/j3-import`, pour ne pas mélanger les deux `records.sqlite`.

```sh
./target/release/crible export --data data --models models --exchange data/j3-exchange --index synth-small
./target/release/crible index create --data data/j3-import --config data/synth-small/index.json
./target/release/crible attach --data data/j3-import --models models --index synth-small data/j3-exchange/outbox/synth-small-g5
./target/release/crible search --data data/j3-import --index synth-small --vector-file /tmp/j3/query.json --k 10 > /tmp/j3/imported.json
cd tools
uv run --no-sync python -c "
import json
a = json.load(open('/tmp/j3/after-compaction.json')); b = json.load(open('/tmp/j3/imported.json'))
ka = [(r['key'], r['score']) for r in a['results']]; kb = [(r['key'], r['score']) for r in b['results']]
print('same keys:', [k for k, _ in ka] == [k for k, _ in kb], '| max abs diff', max(abs(x[1] - y[1]) for x, y in zip(ka, kb)))
"
cd ..
```

Attendu (observé à l'identique) :

```text
index 'synth-small' exported to data/j3-exchange/outbox/synth-small-g5: generation 5, 1950 document(s), 9750 passage(s)
index 'synth-small' created at data/j3-import/indexes/synth-small (epoch 1, generation 0)
batch 'synth-small-g5' attached to index 'synth-small': generation 1 (added 1950, replaced 0, unchanged 0, metadata_updated 0, deleted 0, ignored 0)
same keys: True | max abs diff 0.0
```

**Étape 7 — instantané puis restauration** (CREER-ET-RESTAURER-INSTANTANE). On prend l'instantané à la génération 5, on écrit une génération de plus, puis on restaure.

```sh
./target/release/crible snapshot --data data --exchange data/j3-exchange --index synth-small
printf '%s\n' '{"doc_id":"synth-00000","fields":{"annee":1999}}' > /tmp/j3/patch2.jsonl
./target/release/crible patch --data data --index synth-small /tmp/j3/patch2.jsonl
./target/release/crible inspect --data data --index synth-small
./target/release/crible restore --data data --snapshot data/j3-exchange/snapshots/synth-small-e2-g5
./target/release/crible inspect --data data --index synth-small
./target/release/crible search --data data --index synth-small --vector-file /tmp/j3/query.json --k 10 > /tmp/j3/restored.json
```

Attendu (observé à l'identique) : l'instantané est nommé `synth-small-e2-g5`, 18 537 110 octets ; le patch mène à la génération 6 (`{"generation":6,"unchanged":0,"updated":1}`) ; la restauration réinstalle l'état de la génération 5 dans une **nouvelle époque 3** :

```text
index 'synth-small' snapshot to data/j3-exchange/snapshots/synth-small-e2-g5: epoch 2, generation 5, 1950 document(s), 9750 passage(s), 18537110 bytes
index 'synth-small' restored from data/j3-exchange/snapshots/synth-small-e2-g5: replaced epoch 2 (generation 6), new epoch 3, generation 5, 1950 document(s), 9750 passage(s)
index synth-small
  epoch: 3
  generation: 5
```

et la recherche après restauration est celle d'avant le patch (`same keys/scores: True` avec `/tmp/j3/after-compaction.json`).

**Étape 8 — un instantané incomplet est refusé** (critère de CREER-ET-RESTAURER-INSTANTANE).

```sh
cp -R data/j3-exchange/snapshots/synth-small-e2-g5 data/j3-broken
rm data/j3-broken/segments/*/vectors.f16
./target/release/crible restore --data data --snapshot data/j3-broken
```

Attendu (observé à l'identique), **code de sortie 1**, et l'index reste à l'époque 3, génération 5 :

```text
invalid_request: the snapshot is missing segments/c2-0000000000/vectors.f16, which its attestation lists: No such file or directory (os error 2)
```

**Étape 9 — arrêt brutal à chaque point d'injection** (TESTER-REPRISE-APRES-PANNE, INV-03, INV-04, INV-14). Les tests de panne n'existent que sous la feature `failpoints` :

```sh
cargo test --locked -p crible-store --features failpoints --test inv_04_crash_recovery
```

Attendu : **13 tests passés, 0 échoué**, 1 ignoré (`crash_child_entry`, le processus enfant qui se tue) ; observé à l'identique en 1,02 s. Chaque point d'injection laisse l'état exactement G ou G+1, le rejeu converge vers G+1, et aucun `tmp-*` ne reste dans les dossiers contrôlés, `epochs/` compris.

**Étape 10 — les trois tests longs de l'échantillon Legifacile** (ADR-0062, ADR-0063). Ils demandent ONNX Runtime et les données `data/legifacile-sample` et `data/models/mdenseon` ; ils sont **exclus de la CI par leur nom** et prennent plusieurs minutes, à lancer en `--release` :

```sh
export ORT_DYLIB_PATH="$PWD/tools/.venv/lib/python3.13/site-packages/onnxruntime/capi/libonnxruntime.1.31.0.dylib"
cargo test --locked --release -p crible --test inv_08_export_import_roundtrip --test inv_09_compaction_equivalence -- --include-ignored
cargo test --locked --release -p crible --test recette_import -- --include-ignored
```

Attendu, et **observé** : `inv_08` **11 passés** en 1 283,17 s (dont `inv_08_legifacile_sample_queries_return_the_same_results_after_export_and_import`), `inv_09` **3 passés** en 578,90 s (dont le test de l'échantillon), `recette_import` **10 passés** en 232,83 s (dont `inv_12_the_legifacile_sample_passes_the_acceptance_check`). Les durées sont relevées, pas bornées (ADR-0062) ; la machine n'est pas dédiée à la mesure.

**Nettoyage.**

```sh
rm -rf data/indexes/synth-small data/j3-import data/j3-exchange data/j3-broken /tmp/j3
```

Seuls ces chemins sont retirés ; `data/synth-small/`, `data/legifacile-sample/` et `data/models/` restent en place. L'index `synth-small` est recréé par l'étape 1.

**Bilan de la démo** : les dix blocs ont été exécutés dans l'ordre, sans écart. Sorties obtenues : trois lots quotidiens puis compaction, scores **bit-identiques** avant et après (écart maximal 0,0) ; filtre sur un champ patché à 15 passages, sans ré-encodage ; export puis import dans une seconde instance, mêmes clés et mêmes scores (écart 0,0) ; instantané puis restauration à la génération sauvegardée, dans une nouvelle époque ; instantané incomplet refusé en `invalid_request` sans toucher l'index ; 12 points de panne et INV-03 verts. Seules varient les valeurs que ce rapport annonce comme variables (`took_ms`, durées de compaction et d'instantané).

## 6. Décisions (ADR) de la phase

ADR créés ou modifiés entre la fusion de P2 (`5ad5eb8`) et `b66fddf`, d'après `git log -- docs/adr/` et l'index `docs/adr/README.md` :

| ADR | Titre | Statut | Bloque la validation ? |
|---|---|---|---|
| ADR-0058 | Contrôle anti-fuite de `@git` par script versionné | Accepté (2026-10-25), avec une précision datée du 2026-10-25 | Non |
| ADR-0059 | Robustesse du téléchargement de l'artefact du modèle en CI | Accepté (2026-10-25) ; complète ADR-0057 sans modifier son corps | Non |
| ADR-0060 | Contrat du patch de métadonnées des documents | Accepté (2026-10-25) ; erratum du 2026-10-25 sur la forme de la CLI | Non |
| ADR-0061 | Manifeste d'un lot exporté | Accepté (2026-10-25) | Non |
| ADR-0062 | Second test local de l'échantillon Legifacile, exclu de la CI par son nom | Accepté (2026-10-25) | Non ; appliqué, la durée est relevée ici et aucune borne n'est prescrite |
| ADR-0063 | Troisième test local de l'échantillon Legifacile, exclu de la CI par son nom | Accepté (2026-10-25 ; confirmé le 2026-10-26) ; **erratum du 2026-10-26 accepté par anticipation, à confirmer** | Non, mais la confirmation de l'erratum est une décision à prendre |
| ADR-0064 | Format et contenu d'un instantané | Accepté (2026-10-25, confirmé le 2026-10-26) | Non |
| ADR-0065 | Instantané hors ligne, époque de restauration et rétention | Accepté (2026-10-25, confirmé le 2026-10-26) | Non ; un erratum est possible (§ 7) |
| ADR-0066 | Dérivation de l'instantané après commit, sans balayage complet | Accepté (2026-10-26) | Non |
| ADR-0067 | Durée de la recette à l'attache et échéance de `?wait=true` | Accepté (2026-10-26) | Non |
| ADR-0068 | Construire et tester `crible-store` avec la feature `failpoints` en CI | **Accepté par anticipation** (2026-10-26, à confirmer au point de fin de NETTOYER-P3) | **Oui, à confirmer** : la phase applique l'ADR (deux étapes dans `ci.yml`, `docs/07` à jour) alors qu'il n'est pas encore accepté définitivement |

ADR-0047 et ADR-0048, acceptés le 2026-10-22 avant l'ouverture de P3, sont appliqués par la phase (MES-13 en P3 et ordre des remèdes ; rejeu de MES-15 et compresseur zstd réutilisé).

**Décisions à prendre par le propriétaire**, aucune n'étant une découverte de ce rapport : elles viennent de NETTOYER-P3 ou d'une relecture antérieure.

1. **Confirmer ADR-0068** (accepté par anticipation le 2026-10-26). Sans confirmation, la phase valide une règle de CI que le propriétaire n'a pas encore entérinée.
2. **Confirmer l'erratum d'ADR-0063** du 2026-10-26 (nom de test corrigé dans le corps, aucune décision changée).
3. **Confirmer la borne du patch à 32 000 000 octets** (décision de la partie 6 de NETTOYER-P3, appliquée sans ADR nouveau) : `patch::MAX_BYTES = 32_000_000`, aligné sur « 32 Mo » d'ADR-0060, `docs/04` et `docs/05`, figé par un test.
4. **Trancher la question d'un erratum d'ADR-0065** : les lignes 75 et 180 situent `INITIAL_EPOCH` à `crates/crible/src/commands.rs:67`, alors que la constante est désormais unique dans `crates/crible-store/src/generation.rs`. Laissé tel quel (un ADR accepté décrit l'état de sa date), sauf décision contraire.

**Condition restante liée à la CI distante.** La branche `phase/P3-cycle-de-vie` n'a jamais été poussée : il n'existe **aucun run de CI pour le code de P3**. Deux conséquences, à lire avant `/valider J3` :

- les deux étapes `failpoints` qu'ajoute ADR-0068 (`cargo clippy -p crible-store --all-targets --features failpoints -- -D warnings` et `cargo test -p crible-store --locked --features failpoints`) n'ont **jamais tourné en CI** : elles n'existent que dans le fichier et ont été rejouées en local (§ 2). Le job `rust-onnx` n'a pas non plus été exécuté sur le code de P3 (son dernier run vert, `40183664440`, est celui de `DURCIR-ARTEFACT-MODELE-EN-CI`, avant onze tâches, `NETTOYER-P3` comprise) ;
- la fiche `DURCIR-ARTEFACT-MODELE-EN-CI` demande, « après la fusion de la phase seulement », que le cache de `main` soit constaté chaud au moment de `/valider J3` (premier passage planifié, `workflow_dispatch` sur `main`, ou le `push` de fusion s'il touche le manifeste). Ce constat dépend d'un push que **seul le propriétaire autorise** ; il n'a pas été fait ici.

Recommandation, sans l'exécuter : pousser la branche de phase et attendre un run vert (job `Rust` avec les deux étapes `failpoints`, job `rust-onnx`) **avant** `/valider J3`, puis constater le cache de `main` après la fusion. Aucun run distant n'a été lancé par cet agent.

## 7. Écarts, idées, risques

- **Écarts à la conception** : aucun écart ouvert pour P3. Les écarts constatés pendant la phase sont tous tranchés par un ADR `Accepté` (ADR-0060 à ADR-0068, plus ADR-0058 et ADR-0059), avec deux exceptions de forme : l'erratum d'ADR-0063 (à confirmer) et la borne du patch à 32 000 000 octets (décision de NETTOYER-P3, sans ADR nouveau). La table « Écarts à la conception » de `docs/suivi.md` ne porte aucune ligne P3 sans décision.
- **Risque — aucun run de CI pour le code de P3.** Voir le § 6 : la branche de phase n'a jamais été poussée, donc ni les deux étapes `failpoints` d'ADR-0068 ni le job `rust-onnx` n'ont vu ce code. Les mêmes commandes passent en local (§ 2), mais une différence d'environnement (Ubuntu, toolchain de la CI, artefact du modèle) ne serait vue qu'au premier push.
- **Risque — la CI ne couvre pas les trois tests de l'échantillon Legifacile.** Ils restent locaux par décision (ADR-0062, ADR-0063) et n'ont été vus qu'ici et par l'orchestrateur. Une régression sur ces trois chemins ne serait vue qu'au prochain passage local.
- **Risque — recette à l'attache et `wait_timeout_ms`.** Sur l'échantillon avec `mdenseon`, une attache de 3 002 passages dure 68,5 s, au-delà des 60 s de `wait_timeout_ms` (`docs/05`). ADR-0067 tranche (l'attache est déjà asynchrone, `?wait=true` rend `202` à l'échéance, critère ajouté à `FILE-D-ECRITURE-ET-JOBS` en P6) et ouvre `ACCELERER-RECETTE` en P4. Ce n'est pas un critère de J3, mais le serveur (P6) doit le tenir.
- **Risque — coût de la compaction non décomposé.** 83,9 % du temps de compaction à 7,74 M passages reste hors des trois postes isolés (MES-07), et la mémoire de la compaction à 8,58 M passages n'est couverte par aucune mesure. À instruire en P9 (MES-07 rejouée), pas ici.
- **Risque — reprise des époques non courantes.** `docs/02` ne traite que l'époque courante : une époque abandonnée avant la bascule (`epochs/tmp-*`) ou une époque restée après une panne post-bascule ne sont nettoyées qu'à la compaction suivante. `inv_04` prouve l'état et l'absence de fichier temporaire dans les dossiers contrôlés, mais la question de conception reste ouverte (« Idées hors périmètre », 2026-10-25, `COMPACTER-INDEX`).
- **Idées hors périmètre** : la liste de `docs/suivi.md` porte trois candidats explicites pour un `NETTOYER-P4` (nommage de `prepare` dans MES-08, commentaire perdu sur `after_attach_documents`/`after_attach_passages`, `"indexes"` en dur dans `inv_08`/`inv_09`), plus les questions de conception réservées au serveur (verrou d'écrivain par index, `ReadPool` sur l'époque du snapshot, `adopt_epoch`, identifiants de `segment.json` d'une époque compactée) et les suites de mesure (MES-07, MES-08, `CommitDelta` en mémoire).
- **Points à décider par le propriétaire** :
  1. confirmer ADR-0068, l'erratum d'ADR-0063 et la borne du patch à 32 000 000 octets ; trancher l'erratum d'ADR-0065 (§ 6) ;
  2. décider du push de la branche de phase et du run distant avant `/valider J3`, puis du constat du cache de `main` après la fusion (§ 6) ;
  3. la validation du jalon lui-même, par `VALIDE` dans `docs/suivi.md`.

## 8. Si le jalon n'est pas validé

Aucun manque bloquant n'est constaté sur le code, les tests ou les mesures : les 13 tâches de P3 sont `FAIT`, les contrôles locaux sont verts (§ 2), les sept invariants cités par les fiches sont verts (§ 3), MES-07, MES-08 et MES-13 sont consignées sans dépassement de cible (§ 4) et la démo se rejoue sans écart (§ 5). Si le propriétaire refuse malgré tout la validation, les points restants deviennent des tâches `CORRIGER-…` dans `docs/phases/P3-cycle-de-vie.md` :

- `CORRIGER-CI-P3` (si le run distant échoue) : corriger ce que la CI révèle, en particulier les deux étapes `failpoints` d'ADR-0068 et le job `rust-onnx` ; critère : tous les jobs verts sur la branche de phase, les deux étapes `failpoints` vertes et le job `rust-onnx` vert avec les trois tests locaux de l'échantillon exclus par leur nom.
- `CORRIGER-DEMO-J3` (si un écart de démo est constaté au rejeu) : corriger ce rapport et, si le propriétaire le souhaite, en extraire un `docs/jalons/J3-demo.md` ; critère : chaque sortie annoncée est reproduite, hors valeurs déclarées variables (`took_ms`, durées).
- `CORRIGER-DECISIONS-P3` (si le propriétaire veut fermer les décisions de forme) : appliquer les erratums d'ADR-0063 et d'ADR-0065 par `@greffier` et figer la borne du patch dans le document qui la nomme ; critère : plus aucun ADR en `accepté par anticipation` pour P3.
