# Rapport de jalon J1 — Stockage, lots et recherche exacte

> Rédigé par l'agent `jalon` (commande `/jalon J1`). **Seul le propriétaire peut valider le jalon**, dans `docs/suivi.md`.

- **Date** : 2026-10-22
- **Branche de phase / commit** : `phase/P1-stockage-et-recherche-exacte` / `0a6205d` (arbre propre avant et après les contrôles)
- **Machine** : Mac15,12 (Apple M3, 24 Go de RAM), macOS 15.7.7, arm64 ; Rust 1.99.1 (`cargo 1.99.1 (5f94df478 2026-08-27)`)
- **Verdict de l'agent** : `INCOMPLET` — le code et les tests sont verts, mais deux manques subsistent : la mesure MES-14 n'a aucun rapport, et la démo J1 ne peut pas être rejouée telle qu'écrite sur un dossier `data/` neuf (voir §5 et §7).

## 1. Tâches de la phase

Les 11 tâches de la fiche P1 sont `FAIT` dans `docs/suivi.md`. Les commits ci-dessous sont les fusions sur la branche de phase.

| Tâche | Statut dans `suivi.md` | Commit | Remarque |
|---|---|---|---|
| ECRIRE-LOTS-PYTHON | FAIT | `8768b37` | Paquet `crible`, `BatchWriter`, checksums blake3 (ADR-0034), 41 tests pytest à l'époque de la tâche |
| GENERER-JEU-SYNTHETIQUE | FAIT | `362de21` | `synth-small` en trois lots, `reference_topk.py`, somme `blake3:2d3d8265…58805` |
| STOCKER-ENREGISTREMENTS-SQLITE | FAIT | `5230a46` | `crible-store::records`, JSON canonique, INV-15 |
| ECRIRE-SEGMENTS-VECTEURS | FAIT | `0a70e15` | `vectors.f16`/`f32` little-endian, `segment.json`, mmap, écriture atomique, INV-15 |
| GERER-GENERATIONS | FAIT | `e26ca4e` | Point de commit SQLite (ADR-0016), reprise, `Snapshot` `ArcSwap`, failpoints |
| LIRE-LOT-PARQUET | FAIT | `742e791` | Lecture en flux, contrôles, ADR-0039 et ADR-0040 |
| ATTACHER-LOT | FAIT | `bf609f1` | `upsert`/`replace_all`, une seule transaction, ADR-0041 et ADR-0042 |
| CONSTRUIRE-BITMAPS | FAIT | `7b19d35` | Bitmaps `keyword`/`bool`/présence, colonnes de valeurs, copie sur écriture |
| EVALUER-FILTRES | FAIT | `c0408b3` | `Pool::evaluate`, INV-07 en proptest (ADR-0035 pour les nulls) |
| RECHERCHER-DENSE-EXACT | FAIT | `c5b4b16` | Top-k exact, INV-06 sur 200 requêtes, ADR-0043 |
| CLI-PREMIERES-COMMANDES | FAIT | `bb24cde` | `index create`, `attach`, `search`, `inspect`, démo J1 |

**Tâches hors plan de la phase.** Le journal de `docs/suivi.md` en compte **huit**, et non cinq : `AMELIORER-LOTS-ET-REFERENCE`, `RENFORCER-TESTS-POOL`, `REGLER-BUSY-TIMEOUT` (ADR-0038), `COMPLETER-GENERATIONS`, `MESURER-COUT-COMMIT` (MES-15), `COMPLETER-LECTURE-LOT` (ADR-0040), `COMPLETER-ATTACHE` (ADR-0042) et `NETTOYER-P1`. Toutes sont terminées et relues.

| Tâche hors plan | Fusion | Objet |
|---|---|---|
| AMELIORER-LOTS-ET-REFERENCE | `b59f946` | Écriture atomique du dossier de lot, validation type → opérateurs dans l'oracle |
| RENFORCER-TESTS-POOL | `76ffde5` | Deux mineurs de STOCKER-ENREGISTREMENTS-SQLITE, test de pool rendu déterministe |
| REGLER-BUSY-TIMEOUT | `737e8ad` | Application d'ADR-0038 : `busy_timeout` 1 000 ms, `SQLITE_BUSY` → `overloaded` |
| COMPLETER-GENERATIONS | `12ea703` | Garde de génération étendu, formats dans `docs/02`, MES-15 ajoutée à `docs/08` |
| MESURER-COUT-COMMIT | `434952d` | MES-15 : coût d'un commit de génération |
| COMPLETER-LECTURE-LOT | `854b8c1` | Application d'ADR-0040 : unicité, ordre numérique des parties |
| COMPLETER-ATTACHE | `e908054` | Trois mineurs d'ATTACHER-LOT, plus aucun `allow(clippy…)` dans `crates/` |
| NETTOYER-P1 | `0a6205d` | 12 mineurs de relecture, `clippy::panic`, CI sur Node.js 24, `--locked` |

## 2. Qualité

Exécuté sur `phase/P1-stockage-et-recherche-exacte`, arbre propre.

| Contrôle | Commande | Résultat |
|---|---|---|
| Formatage | `cargo fmt --all -- --check` (forme d'ADR-0044) | OK, code de sortie 0 |
| Lints | `cargo clippy --all-targets --locked -- -D warnings` | OK, 0 avertissement, code de sortie 0 |
| Tests Rust | `cargo test --locked` | **324 passés, 0 échoué, 0 ignoré** (38 binaires de test, Doc-tests compris) |
| Tests Rust avec pannes | `cargo test -p crible-store --features failpoints --locked` | **140 passés, 0 échoué** |
| Tests Python du producteur | `cd clients/python && uv run pytest -q` | **64 passés, 0 échoué**, 0,97 s |
| Tests Python des outils | `cd tools && uv run pytest -q` | **145 passés, 0 échoué**, 14,89 s |

Les totaux concordent avec `docs/suivi.md` : 324 tests Rust et 140 avec `failpoints` pour NETTOYER-P1. Le total Python de `clients/python` a grandi de 52 à 64 depuis GENERER-JEU-SYNTHETIQUE, sans que le suivi le mentionne ; ce n'est pas un écart, seulement une information.

## 3. Invariants de la phase

| Invariant | Test(s) | Résultat |
|---|---|---|
| INV-03 — génération visible entièrement ou pas du tout | `crates/crible-store/tests/inv_03_generation_visibility.rs` (7 tests) + `inv_03_failed_attach_leaves_the_records_database_untouched`, `inv_03_doc_id_in_documents_and_in_deletions_rejects_the_batch` dans `attacher_lot.rs` (avec `failpoints`) | **9 passés, 0 échoué** |
| INV-04 — au redémarrage, dernière génération commitée | `crates/crible-store/tests/inv_04_recovery.rs`, 7 tests avec `--features failpoints` (dont `inv_04_crash_at_every_step_yields_old_or_new_generation`) ; 3 tests de reprise sans la feature | **7 passés avec pannes, 0 échoué** |
| INV-06 — pool ≤ seuil exact ⇒ résultat exact | `crates/crible-search/tests/inv_06_dense_exact_synth.rs` : `inv_06_synth_small_200_queries_match_the_numpy_reference`, 3 gardes de tolérance, contrôle des quasi-égalités de `expected.jsonl` | **12 passés, 0 échoué**, 15,85 s |
| INV-07 — filtre → bitmap ≡ SQL | `inv_07_filter_bitmap_matches_sql.rs` (2 tests, propriété à 1 000 filtres) + `inv_07_filter_semantics.rs` (5 tests) | **7 passés, 0 échoué** |
| INV-10 — rejouer un lot ne fait rien | `inv_10_replaying_an_applied_batch_changes_nothing`, `inv_10_same_batch_id_with_another_manifest_is_a_batch_conflict` | **2 passés, 0 échoué** |
| INV-15 — format little-endian explicite et versionné | `inv_15_records_format_version.rs` (5 tests) + `inv_15_segment_vectors.rs` (8 tests) | **13 passés, 0 échoué** |

INV-15 est cité par deux fiches de P1 (STOCKER-ENREGISTREMENTS-SQLITE et ECRIRE-SEGMENTS-VECTEURS) ; il est donc contrôlé ici en plus des quatre invariants du jalon.

## 4. Mesures de la phase

| Mesure | Rapport | Chiffre clé | Cible | Atteinte ? |
|---|---|---|---|---|
| MES-13 — temps de démarrage, reconstruction des bitmaps | **aucun** | — | aucune cible chiffrée | n/a |
| MES-14 — coût des filtres d'intervalle selon le domaine | **aucun** | — | budget de `docs/03` : filtre < 1 ms ; aucune cible de `docs/suivi.md` | **non mesurée** |
| MES-15 — coût d'un commit de génération | `docs/mesures/MES-15-cout-commit-generation.md` (campagne du 2026-10-21) | p50 **598,3 ms** à 8 580 000 passages et 10 segments, **641,3 ms** à 1 000 segments ; décomposition : 86,5 % pour matérialiser les iid vivants, transaction SQLite 77,6 ms | aucune cible chiffrée | n/a (mesure de coût) |

Précisions : MES-13 est citée par la fiche CONSTRUIRE-BITMAPS (« premier chiffre »), mais `docs/08-invariants-et-tests.md` la place en **P3** ; le code et les tests le disent aussi (`generation.rs`, `construire_bitmaps.rs`). Aucun rapport n'existe, et ce n'est pas un manque de P1 si l'on suit `docs/08`. MES-14 est en revanche placée en **P1, P4** par `docs/08` et citée par la fiche EVALUER-FILTRES (« premier chiffre sur `synth-small` ») : son absence est un manque de la phase.

MES-15 a été mesurée sur une machine chargée (Ollama actif, charge moyenne 5,55 → 11,49 sur 8 cœurs) : les chiffres sont plutôt pessimistes, et la reprise de la campagne donne 595,6 ms et 644,3 ms, soit ± 0,5 %.

**Cibles de `docs/suivi.md` applicables à P1** : aucune cible contraignante ne s'applique. Le rappel ≥ 0,482 et les seuils par corpus exigent l'encodeur et le jeu Legifacile (P2 et P9) ; la latence p95 ≤ 698 ms ne vaut que pour le Mac mini en conteneur avec le serveur (P8) et reste indicative ailleurs ; la fidélité binaire ≥ 0,99 est une mesure de P4. Le principe « le rappel prime, la latence est indicative » ne change donc rien au verdict de J1.

## 5. Démo à rejouer par le propriétaire

Les commandes exactes, à copier-coller depuis la racine du dépôt. Elles créent un dossier `data/` jetable (gitignoré) et ne touchent à aucune production.

**Préparation du jeu.** La démo écrite dans `docs/jalons/J1-demo.md` s'arrête à `gen_synth.py` ; c'est insuffisant, voir l'écart §7. La version corrigée ajoute `reference_topk.py`, comme le fait déjà `tools/README.md` :

```sh
cd /Users/mattboll/MattCorp/crible
rm -rf data/synth-small data/indexes
mkdir -p models/synthetic-external
cat > models/synthetic-external/model.json <<'JSON'
{ "name": "synthetic-external", "kind": "external", "dims": 768, "normalize": true }
JSON
cd tools
uv sync
uv run python gen_synth.py --out ../data/synth-small --overwrite
uv run python reference_topk.py --dataset ../data/synth-small
cd ..
```

Attendu : `../data/synth-small: 1950 documents, 9750 live passages after l1+l2+l3, seed 20261021`, puis `reference over 1950 documents, 9750 live passages`, la somme `dataset checksum: blake3:2d3d8265a8df350d816bccb5732fe338921d275cae5d091a96d0497646e58805` et `wrote ../data/synth-small/expected.jsonl`. C'est la somme consignée dans `tools/README.md` : elle ne dépend ni de la machine ni de la date.

**Création de l'index.**

```sh
cargo build --release --locked
./target/release/crible index create --data data --config data/synth-small/index.json
```

Attendu : `index 'synth-small' created at data/indexes/synth-small (epoch 1, generation 0)`. Le dossier contient `index.json`, `CURRENT_EPOCH`, `epochs/1/records.sqlite`, `epochs/1/manifests/0.json`, puis les segments après les attaches.

**Attache des trois lots.**

```sh
./target/release/crible attach --data data --models models --index synth-small data/synth-small/l1
./target/release/crible attach --data data --models models --index synth-small data/synth-small/l2
./target/release/crible attach --data data --models models --index synth-small data/synth-small/l3
```

Attendu, dans l'ordre (observé à l'identique) :

```text
batch 'l1' attached to index 'synth-small': generation 1 (added 2000, replaced 0, unchanged 0, metadata_updated 0, deleted 0, ignored 0)
batch 'l2' attached to index 'synth-small': generation 2 (added 100, replaced 300, unchanged 0, metadata_updated 0, deleted 0, ignored 0)
batch 'l3' attached to index 'synth-small': generation 3 (added 0, replaced 0, unchanged 0, metadata_updated 50, deleted 150, ignored 0)
```

**État de l'instance.**

```sh
./target/release/crible inspect --data data
```

Attendu (observé à l'identique) :

```text
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

**Recherche avec un vecteur fourni.**

```sh
cd tools
uv run python - <<'PY'
import json
from pathlib import Path

query = json.loads(
    Path("../data/synth-small/queries.jsonl").read_text(encoding="utf-8").splitlines()[0]
)
Path("/tmp/crible-j1-query.json").write_text(
    json.dumps(query["vector"]), encoding="utf-8"
)
PY
cd ..
./target/release/crible search --data data --index synth-small \
    --vector-file /tmp/crible-j1-query.json --k 10
```

Attendu : un objet JSON avec `generation: 3`, `pool_size: 9750`, `took_ms` (42,8 ms ici) et 10 résultats. Les clés observées sont, dans l'ordre, celles de la première ligne de `data/synth-small/expected.jsonl` :

```text
synth-01163#3, synth-00407#0, synth-01703#2, synth-02087#4, synth-00087#1,
synth-00807#4, synth-01766#2, synth-00134#3, synth-00359#0, synth-00379#4
```

Les scores diffèrent de la référence de moins de 2 × 10⁻⁷ (par exemple 0,6926817298 contre 0,6926815510), donc dans la tolérance de 1e-5 de la fiche. Les 10 clés sont identiques et dans le même ordre.

**Recherche avec un filtre.**

```sh
./target/release/crible search --data data --index synth-small \
    --vector-file /tmp/crible-j1-query.json \
    --filter '{"categorie": "cat-03"}' --k 5
```

Attendu : 5 résultats, tous en `cat-03`, `pool_size: 585` (bien inférieur à 9 750), scores cosinus strictement décroissants (−0,0417295, −0,0426792, −0,0461998, −0,0462879, −0,0468779), `took_ms` 49,2 ms ici.

**Erreurs lisibles.**

```sh
./target/release/crible index create --data data --config data/synth-small/index.json
```

Attendu, code de sortie 1 (observé) :

```text
invalid_request: an index already exists at data/indexes/synth-small: data/indexes/synth-small/CURRENT_EPOCH is present, and creating an index never overwrites one; use `crible attach`, `crible search` or `crible inspect` on it, or create another index under another name
```

```sh
./target/release/crible search --data data --index synth-small --vector-file missing.npy
```

Attendu, code de sortie 1 (observé) :

```text
invalid_request: cannot read the vector file missing.npy: No such file or directory (os error 2)
```

Le message d'arbre d'index partiel, décrit dans la démo sans commande, a été vérifié sur un dossier temporaire (`indexes/synth-small/epochs/` sans `CURRENT_EPOCH`) ; il sort bien en `invalid_request` avec le texte annoncé. Il n'apparaît pas dans le bloc ci-dessus parce que la démo ne fournit pas de commande pour le produire.

## 6. Décisions (ADR) de la phase

ADR créés ou modifiés entre le tag `jalon-J0` et `0a6205d` (`git log -- docs/adr/`) :

| ADR | Titre | Statut | Bloque la validation ? |
|---|---|---|---|
| ADR-0034 | Dépendance Python `blake3` pour les checksums des lots | Accepté (2026-10-21) | Non |
| ADR-0035 | Sémantique des valeurs nulles dans les filtres | Accepté (2026-10-21) | Non |
| ADR-0036 | `producer.host` réglable par le producteur | Accepté (2026-10-21) | Non |
| ADR-0037 | Permissions de lecture de `@relecteur` élargies | Accepté (2026-10-21) | Non |
| ADR-0038 | Délai d'attente SQLite `busy_timeout` | Accepté (2026-10-21) | Non |
| ADR-0039 | Ordre des lignes dans un lot v1 | Accepté (2026-10-21) | Non |
| ADR-0040 | Unicité des documents et des parties dans un lot v1 | Accepté (2026-10-22) | Non |
| ADR-0041 | Attache : document supprimé, digest du manifeste, rapport | Accepté (2026-10-22) | Non |
| ADR-0042 | Durées du rapport d'attache non persistées | Accepté (2026-10-22) | Non |
| ADR-0043 | Payload d'un résultat dont le document est supprimé | Accepté (2026-10-22) | Non |
| ADR-0044 | Règles de `@relecteur` : vérifications exécutées | Accepté (2026-10-22) | Non |

**Aucun ADR n'est au statut `Proposé`.** La table « Décisions en attente » de `docs/suivi.md` est vide et l'index `docs/adr/README.md` ne liste que des `Accepté`. Aucune décision d'ADR ne bloque J1.

## 7. Écarts, idées, risques

- **Écart de démo (bloquant pour le critère « le scénario de démo J1 passe tel qu'écrit »)** : `docs/jalons/J1-demo.md` fait régénérer `synth-small` par `gen_synth.py --overwrite` et s'arrête là. Or `gen_synth.py` **supprime** `expected.jsonl` (il figure dans `SMALL_FILES`) sans jamais l'écrire : c'est `tools/reference_topk.py` qui le produit, et `tools/README.md` enchaîne bien les deux commandes. Sur un dossier `data/` neuf, `expected.jsonl` est donc absent après l'étape de préparation, et la comparaison demandée par la démo (« les clés sont celles de la première ligne de `expected.jsonl` ») est impossible. Le défaut est documentaire, pas logiciel : en ajoutant `uv run python reference_topk.py --dataset ../data/synth-small`, la somme de contrôle retrouve exactement `blake3:2d3d8265…58805` et toutes les commandes de la démo rendent le résultat attendu. Proposition : `CORRIGER-DEMO-J1`, ajouter la commande `reference_topk.py` à la section « Préparation du jeu » et rappeler que `--overwrite` supprime `expected.jsonl`.
- **Mesure manquante (bloquant pour la règle 7)** : MES-14 (« coût des filtres d'intervalle selon le domaine ») est citée par la fiche EVALUER-FILTRES et classée en P1 par `docs/08`, mais aucun rapport n'existe dans `docs/mesures/`. Le budget de `docs/03` (« filtre < 1 ms ») n'est donc étayé par aucune mesure de Crible. Proposition : `CORRIGER-MES-14`, exécuter MES-14 sur `synth-small` (colonnes `i64`, `f64`, `i32`, domaines réduits et non réduits) et écrire `docs/mesures/MES-14-….md`.
- **Ambiguïté MES-13** : la fiche CONSTRUIRE-BITMAPS la cite (« premier chiffre ») alors que `docs/08` la place en P3 et que le code le documente. Aucun rapport n'existe. À trancher par le propriétaire : soit MES-13 reste une mesure de P3 et la fiche P1 est alignée, soit elle est exigée pour J1.
- **Réserve de traçabilité sur la CI** : la branche `phase/P1-stockage-et-recherche-exacte` n'a pas d'équivalent distant (`origin/phase/P1-…` absent), donc GitHub n'a exécuté aucun run pour P1. Le critère « la CI est verte » est établi par les équivalents locaux (`cargo fmt`, `cargo clippy`, `cargo test`, les deux `pytest`), pas par un run distant. Les pushes sont réservés au propriétaire.
- **État git annexe** : `phase/P0-fondations` est en avance de 2 commits sur `origin/phase/P0-fondations` (les commits de validation de J0 ne sont pas poussés). Rien à voir avec la qualité de P1, mais `@git` doit le savoir avant tout push.
- **Risques constatés** : aucun risque fonctionnel. Les points de robustesse connus sont déjà notés en idées hors périmètre (marges des tests de `busy_timeout`, `inspect` qui exige un montage inscriptible, `ReadPool::verify` partiel, ordre textuel `doc_id#ord` contre `PassageKey::Ord`).
- **Points à décider par le propriétaire** :
  1. exiger MES-14 avant de valider J1, ou l'accepter comme différée après P4 (elle est aussi classée P4) ;
  2. confirmer que MES-13 reste en P3 et que la fiche P1 sera alignée ;
  3. accepter la correction documentaire de la démo (`CORRIGER-DEMO-J1`) ou valider avec la commande `reference_topk.py` ajoutée à la main ;
  4. dire si l'absence de run CI distant est acceptable pour J1, ou demander un push de la branche de phase.

## 8. Si le jalon n'est pas validé

Les corrections demandées deviennent des tâches `CORRIGER-…` dans `docs/phases/P1-stockage-et-recherche-exacte.md`, avec leurs critères d'acceptation. Candidats immédiats :

- `CORRIGER-DEMO-J1` : ajouter `uv run python reference_topk.py --dataset ../data/synth-small` à la préparation de `docs/jalons/J1-demo.md`, et une phrase disant que `--overwrite` supprime `expected.jsonl`. Critère : sur un `data/` vide, la suite des commandes de la démo recrée `expected.jsonl`, retrouve la somme `blake3:2d3d8265…58805` et rend les sorties attendues.
- `CORRIGER-MES-14` : mesurer le coût des filtres d'intervalle sur `synth-small` et écrire le rapport. Critère : un rapport `docs/mesures/MES-14-….md` conforme au modèle, avec machine, commit, protocole et chiffres bruts, comparés au budget de `docs/03`.
- `CORRIGER-MES-13-FICHE` (si le propriétaire choisit cette branche) : retirer la mention « premier chiffre » de la fiche CONSTRUIRE-BITMAPS ou la remplacer par un renvoi à P3. Critère : la fiche et `docs/08` disent la même phase pour MES-13.
