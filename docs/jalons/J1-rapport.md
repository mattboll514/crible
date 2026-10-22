# Rapport de jalon J1 — Stockage, lots et recherche exacte

> Rédigé par l'agent `jalon` (commande `/jalon J1`). **Seul le propriétaire peut valider le jalon**, dans `docs/suivi.md`.

- **Date** : 2026-10-22, **second passage** (le premier, commit `b595386`, concluait `INCOMPLET`)
- **Branche de phase / commit** : `phase/P1-stockage-et-recherche-exacte` / `7e73df3` (arbre propre avant et après les contrôles, hors la ligne de journal non commitée de `docs/suivi.md`)
- **Machine** : Mac15,12 (Apple M3, 24 Go de RAM), macOS 15.7.7, arm64 ; Rust 1.99.1 (`cargo 1.99.1 (5f94df478 2026-08-27)`), uv 0.11.21
- **CI distante** : run `39824366948` **vert** (Rust, Tools, Python), vérifié par `gh run view 39824366948` ; le premier run `39794175404` était rouge (voir § 6 et § 7)
- **Verdict de l'agent** : `PRET-POUR-VALIDATION`

Les deux manques du premier passage sont levés : MES-14 a un rapport (`d82f8b7`) et la démo J1 se rejoue intégralement telle qu'écrite (`7e73df3`). Les quatre points de décision du premier passage sont traités : MES-13 et MES-14 sont mesurées, la démo est corrigée, la branche de phase est poussée et la CI est verte.

## 1. Tâches de la phase

Les 11 tâches de la fiche P1 sont `FAIT` dans `docs/suivi.md`. Les commits ci-dessous sont les fusions sur la branche de phase.

| Tâche | Statut dans `suivi.md` | Commit | Remarque |
|---|---|---|---|
| ECRIRE-LOTS-PYTHON | FAIT | `8768b37` | Paquet `crible`, `BatchWriter`, checksums blake3 (ADR-0034) |
| GENERER-JEU-SYNTHETIQUE | FAIT | `362de21` | `synth-small` en trois lots, `reference_topk.py`, somme remplacée par `blake3:a9854f1e…12b1f` (ADR-0045) |
| STOCKER-ENREGISTREMENTS-SQLITE | FAIT | `5230a46` | `crible-store::records`, JSON canonique, INV-15 |
| ECRIRE-SEGMENTS-VECTEURS | FAIT | `0a70e15` | `vectors.f16`/`f32` little-endian, `segment.json`, mmap, écriture atomique, INV-15 |
| GERER-GENERATIONS | FAIT | `e26ca4e` | Point de commit SQLite (ADR-0016), reprise, `Snapshot` `ArcSwap`, failpoints |
| LIRE-LOT-PARQUET | FAIT | `742e791` | Lecture en flux, contrôles, ADR-0039 et ADR-0040 |
| ATTACHER-LOT | FAIT | `bf609f1` | `upsert`/`replace_all`, une seule transaction, ADR-0041 et ADR-0042 |
| CONSTRUIRE-BITMAPS | FAIT | `7b19d35` | Bitmaps `keyword`/`bool`/présence, colonnes de valeurs, copie sur écriture ; MES-13 désormais mesurée |
| EVALUER-FILTRES | FAIT | `c0408b3` | `Pool::evaluate`, INV-07 en proptest (ADR-0035 pour les nulls) ; MES-14 désormais mesurée |
| RECHERCHER-DENSE-EXACT | FAIT | `c5b4b16` | Top-k exact, INV-06 sur 200 requêtes, ADR-0043 |
| CLI-PREMIERES-COMMANDES | FAIT | `bb24cde` | `index create`, `attach`, `search`, `inspect`, démo J1 |

**Tâches hors plan de la phase.** Le journal de `docs/suivi.md` en compte **dix**. Toutes sont terminées et relues.

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
| REFERENCE-EXACTE-SANS-BLAS | `ac5cc5e` | Application d'ADR-0045 : référence exacte (`math.fsum`), nouvelle somme de `synth-small` |
| CORRIGER-DEMO-J1 | `7e73df3` | Démo J1 rejouable de bout en bout, borne d'écart corrigée à moins de 5 × 10⁻⁷ |

## 2. Qualité

Exécuté sur `phase/P1-stockage-et-recherche-exacte`, à la date du rapport.

| Contrôle | Commande | Résultat |
|---|---|---|
| Formatage | `cargo fmt --all -- --check` (forme d'ADR-0044) | OK, code de sortie 0 |
| Lints | `cargo clippy --all-targets --locked -- -D warnings` | OK, 0 avertissement, code de sortie 0 |
| Tests Rust | `cargo test --locked` | **324 passés, 0 échoué, 0 ignoré** (33 binaires de test + 5 Doc-tests, donc 38 cibles) |
| Tests Rust avec pannes | `cargo test -p crible-store --features failpoints --locked` | **140 passés, 0 échoué, 0 ignoré** (13 cibles) |
| Tests Python du producteur | `cd clients/python && uv run pytest -q` | **64 passés, 0 échoué**, 0,37 s |
| Tests Python des outils | `cd tools && uv run pytest -q` | **146 passés, 0 échoué**, 72,29 s |
| CI distante | `gh run view 39824366948` | **verte** : Python 11 s, Rust 3 min 5 s, Tools 2 min 38 s ; seule annotation, l'avertissement de migration `ubuntu-latest` vers Ubuntu 26 |

Les totaux concordent avec `docs/suivi.md` : 324 tests Rust et 140 avec `failpoints` depuis NETTOYER-P1, 146 tests `tools` et 64 `clients/python` depuis REFERENCE-EXACTE-SANS-BLAS.

## 3. Invariants de la phase

| Invariant | Test(s) | Résultat |
|---|---|---|
| INV-03 — génération visible entièrement ou pas du tout | `crates/crible-store/tests/inv_03_generation_visibility.rs` (7 tests) + `inv_03_failed_attach_leaves_the_records_database_untouched`, `inv_03_doc_id_in_documents_and_in_deletions_rejects_the_batch` dans `attacher_lot.rs` (avec `failpoints`) | **9 passés, 0 échoué** |
| INV-04 — au redémarrage, dernière génération commitée | `crates/crible-store/tests/inv_04_recovery.rs`, 7 tests avec `--features failpoints` | **7 passés, 0 échoué** |
| INV-06 — pool ≤ seuil exact ⇒ résultat exact | `crates/crible-search/tests/inv_06_dense_exact_synth.rs` : `inv_06_synth_small_200_queries_match_the_numpy_reference`, comparateur de tolérance, contrôle des quasi-égalités de `expected.jsonl` (4 sous 1e-6, 55 sous 1e-5) | **12 passés, 0 échoué**, 15,69 s pour la cible |
| INV-07 — filtre → bitmap ≡ SQL | `inv_07_filter_bitmap_matches_sql.rs` (2 tests, propriété à 1 000 filtres) + `inv_07_filter_semantics.rs` (5 tests) | **7 passés, 0 échoué** |
| INV-10 — rejouer un lot ne fait rien | `inv_10_replaying_an_applied_batch_changes_nothing`, `inv_10_same_batch_id_with_another_manifest_is_a_batch_conflict` | **2 passés, 0 échoué**, 4,53 s |
| INV-15 — format little-endian explicite et versionné | `inv_15_records_format_version.rs` (5 tests) + `inv_15_segment_vectors.rs` (8 tests) | **13 passés, 0 échoué** |

Les quatre invariants du jalon sont donc verts (INV-03, INV-06, INV-07, INV-10). INV-15 est cité par deux fiches de P1 (STOCKER-ENREGISTREMENTS-SQLITE et ECRIRE-SEGMENTS-VECTEURS) ; il est contrôlé ici en plus. INV-04 est cité par GERER-GENERATIONS et contrôlé aussi.

## 4. Mesures de la phase

| Mesure | Rapport | Chiffre clé | Cible | Atteinte ? |
|---|---|---|---|---|
| MES-13 — temps de démarrage, reconstruction des structures de filtrage (premier chiffre, sans cache) | `docs/mesures/MES-13-temps-de-demarrage.md` (`1038577`) | **20,12 s en p50** (p95 et p99 à 20,15 s) à 8 580 000 lignes, cache chaud ; 2,02 s à 1 M ; 23,6 ms sur `synth-small` ; 369,3 Mio de structures à 8,58 M | aucune cible dans `docs/08` ; seul budget écrit : 60 s de `MESURER-CYCLE-DE-VIE` (P3) | **33,5 % du budget de 60 s**, marge de 2,98 fois (≈ 22,4 s avec 10 % de lignes mortes) |
| MES-14 — coût des filtres d'intervalle selon le domaine | `docs/mesures/MES-14-cout-filtres-intervalles.md` (`d82f8b7`) | clauses en bitmap : **50,9 µs** (`partie=corps`) et **209,6 µs** (`categorie=cat-03`) à 8,58 M passages ; clauses d'intervalle : **101,2 à 194,4 ms** à domaine plein, **9,1 à 49,2 ms** sous domaine réduit ; **0,09 à 0,17 ms** sur `synth-small` | budget de `docs/03` : « Filtre (bitmaps) < 1 ms » | **tenu par les clauses en bitmap**, **dépassé par les clauses d'intervalle** dès que le domaine dépasse quelques dizaines de milliers de passages ; budget respecté sur `synth-small` seul |
| MES-15 — coût d'un commit de génération | `docs/mesures/MES-15-cout-commit-generation.md` (campagne du 2026-10-21) | p50 **598,3 ms** à 8 580 000 passages et 10 segments, **641,3 ms** à 1 000 segments ; 86,5 % pour matérialiser les iid vivants, transaction SQLite 77,6 ms | aucune cible chiffrée | n/a (mesure de coût) |

Précisions :

- **MES-13** est citée par la fiche CONSTRUIRE-BITMAPS (« premier chiffre ») alors que la table de `docs/08` la place en **P3**. Le rapport est le premier chiffre demandé par la fiche : l'ambiguïté n'a plus d'effet pratique. Reste à aligner la fiche et la table de `docs/08` sur la phase (décision du propriétaire, § 7).
- **MES-14** est classée **P1, P4** par `docs/08` et citée par la fiche EVALUER-FILTRES (« premier chiffre sur `synth-small` »). Le dépassement du budget à l'échelle Legifacile n'est **pas un critère du jalon J1** : c'est une décision ouverte du propriétaire, et elle est signalée comme telle au § 7.
- Les deux campagnes ont été faites sur une machine chargée (Ollama actif, Docker Desktop pour MES-13) et le disent ; leurs chiffres sont plutôt pessimistes. MES-14 a été mesurée au commit `b595386`, MES-13 au commit `6dbb22d` : aucun code de production n'a été modifié par ces campagnes, seuls des bancs et leurs déclencheurs `[[bench]]` ont été ajoutés.

**Cibles de `docs/suivi.md` applicables à P1** : aucune cible contraignante ne s'applique. Le rappel ≥ 0,482 et les seuils par corpus exigent l'encodeur et le jeu Legifacile (P2 et P9) ; la latence p95 ≤ 698 ms ne vaut que pour le Mac mini en conteneur avec le serveur (P8) et reste indicative ailleurs ; la fidélité binaire ≥ 0,99 est une mesure de P4. Le principe « le rappel prime, la latence est indicative » ne change rien au verdict de J1.

## 5. Démo à rejouer par le propriétaire

La démo de `docs/jalons/J1-demo.md` a été **rejouée intégralement, bloc par bloc, depuis son préambule**, le 2026-10-22 sur cette branche : chaque sortie a été comparée au texte, sans écart (§ 7). Les commandes ci-dessous sont celles du fichier, à copier-coller depuis la racine du dépôt. Elles créent un dossier `data/` jetable (gitignoré) et ne touchent à aucune production.

**Préambule et préparation du jeu.**

```sh
cd /Users/mattboll/MattCorp/crible
rm -rf data/synth-small data/indexes/synth-small
cd tools
uv sync
uv run python gen_synth.py --out ../data/synth-small --overwrite
uv run python reference_topk.py --dataset ../data/synth-small
cd ..
```

Attendu, dans l'ordre — **observé à l'identique** :

```text
../data/synth-small: 1950 documents, 9750 live passages after l1+l2+l3, seed 20261021
../data/synth-small/expected.jsonl: reference over 1950 documents, 9750 live passages
dataset checksum: blake3:a9854f1e3f14b8c6e9afb5bd484a85d0cdc453d6812670d7f092e31760112b1f
wrote ../data/synth-small/expected.jsonl
```

Cette somme ne dépend ni de la machine, ni de la date, ni de la bibliothèque BLAS : la référence est calculée exactement (ADR-0045). C'est la valeur consignée dans `tools/README.md` et celle du run CI vert.

**Modèle externe du jeu (une fois, à la main).**

```sh
mkdir -p models/synthetic-external
cat > models/synthetic-external/model.json <<'JSON'
{ "name": "synthetic-external", "kind": "external", "dims": 768, "normalize": true }
JSON
```

**Construction et création de l'index.**

```sh
cargo build --release
./target/release/crible index create --data data --config data/synth-small/index.json
```

Attendu — **observé à l'identique**, code de sortie 0 :

```text
index 'synth-small' created at data/indexes/synth-small (epoch 1, generation 0)
```

Le dossier `data/indexes/synth-small/` contient bien `index.json`, `CURRENT_EPOCH` et `epochs/1/` avec `records.sqlite`, `segments/` et `manifests/0.json`.

**Attache des trois lots.**

```sh
./target/release/crible attach --data data --models models --index synth-small data/synth-small/l1
./target/release/crible attach --data data --models models --index synth-small data/synth-small/l2
./target/release/crible attach --data data --models models --index synth-small data/synth-small/l3
```

Attendu, dans l'ordre — **observé à l'identique** :

```text
batch 'l1' attached to index 'synth-small': generation 1 (added 2000, replaced 0, unchanged 0, metadata_updated 0, deleted 0, ignored 0)
batch 'l2' attached to index 'synth-small': generation 2 (added 100, replaced 300, unchanged 0, metadata_updated 0, deleted 0, ignored 0)
batch 'l3' attached to index 'synth-small': generation 3 (added 0, replaced 0, unchanged 0, metadata_updated 50, deleted 150, ignored 0)
```

**État de l'instance.**

```sh
./target/release/crible inspect --data data
```

Attendu — **observé à l'identique**, code de sortie 0 :

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

Attendu : un objet JSON avec `generation: 3`, `pool_size: 9750`, `results` (10 résultats) et `took_ms` variable. **Observé** : `generation` 3, `pool_size` 9750, `took_ms` 41,54 ms, et les clés suivantes, dans l'ordre exact de la première ligne de `data/synth-small/expected.jsonl` :

```text
synth-01163#3, synth-00407#0, synth-01703#2, synth-02087#4, synth-00087#1,
synth-00807#4, synth-01766#2, synth-00134#3, synth-00359#0, synth-00379#4
```

Les scores diffèrent de la référence de **moins de 5 × 10⁻⁷**, pour un écart maximal mesuré de **4,768 × 10⁻⁷** sur cette requête (`synth-00087#1` : `0.6897367239` contre `0.6897372007`), donc dans la tolérance de `1e-5` de la fiche RECHERCHER-DENSE-EXACT. L'écart est déterministe : la référence est exacte et arrondie une seule fois en f32 (ADR-0045), alors que le moteur somme en f32 dans l'ordre des composantes.

**Recherche avec un filtre.**

```sh
./target/release/crible search --data data --index synth-small \
    --vector-file /tmp/crible-j1-query.json \
    --filter '{"categorie": "cat-03"}' --k 5
```

Attendu : 5 résultats, tous en `cat-03`, `pool_size: 585`, scores strictement décroissants. **Observé à l'identique**, `took_ms` 47,34 ms :

```text
synth-01603#4 -0.0417295
synth-00739#1 -0.0426792
synth-00675#4 -0.0461998
synth-00322#0 -0.0462879
synth-00003#1 -0.0468779
```

**Erreurs lisibles.**

```sh
./target/release/crible index create --data data --config data/synth-small/index.json
```

Attendu — **observé à l'identique**, code de sortie 1 :

```text
invalid_request: an index already exists at data/indexes/synth-small: data/indexes/synth-small/CURRENT_EPOCH is present, and creating an index never overwrites one; use `crible attach`, `crible search` or `crible inspect` on it, or create another index under another name
```

```sh
rm -rf data/tmp-partial-index
mkdir -p data/tmp-partial-index/indexes/synth-small/epochs
./target/release/crible index create --data data/tmp-partial-index --config data/synth-small/index.json
```

Attendu — **observé à l'identique**, code de sortie 1 :

```text
invalid_request: a partial index tree is present at data/tmp-partial-index/indexes/synth-small: data/tmp-partial-index/indexes/synth-small/epochs exists, but the index has no CURRENT_EPOCH pointer, so no creation can reopen it, and creating an index never overwrites one; remove it to create a new index
```

Nettoyage, puis dernier cas :

```sh
rm -rf data/tmp-partial-index
./target/release/crible search --data data --index synth-small --vector-file missing.npy
```

Attendu — **observé à l'identique**, code de sortie 1 :

```text
invalid_request: cannot read the vector file missing.npy: No such file or directory (os error 2)
```

**Bilan de la démo** : tous les blocs passent, toutes les sorties annoncées sont reproduites, y compris la somme `blake3:a9854f1e…12b1f`, les comptes d'attache, l'état d'`inspect`, les deux recherches et les trois erreurs. Seules varient les valeurs que le texte annonce comme variables (`took_ms`).

## 6. Décisions (ADR) de la phase

ADR créés ou modifiés entre le tag `jalon-J0` et `7e73df3` (`git log -- docs/adr/`) :

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
| ADR-0045 | Référence de `synth-small` calculée exactement, indépendante de la bibliothèque BLAS | Accepté (2026-10-22, proposé puis accepté le même jour) | Non |

**Aucun ADR n'est au statut `Proposé`.** La table « Décisions en attente » de `docs/suivi.md` est vide et l'index `docs/adr/README.md` ne liste que des `Accepté`. Aucune décision d'ADR ne bloque J1.

## 7. Écarts, idées, risques

- **Aucun écart de démo.** L'écart du premier passage est soldé par CORRIGER-DEMO-J1 (`7e73df3`) : `docs/jalons/J1-demo.md` enchaîne `reference_topk.py` après `gen_synth.py --overwrite`, donne sa raison, borne son préambule à `data/synth-small` et `data/indexes/synth-small`, fournit les commandes de l'arbre d'index partiel et son nettoyage, et cite la sortie exacte d'`inspect` sans iid. Le fichier a été rejoué intégralement le 2026-10-22 : aucune sortie ne diverge.
- **Aucune mesure manquante.** MES-13, MES-14 et MES-15 ont un rapport dans `docs/mesures/`, avec machine, commit, protocole, chiffres bruts et comparaison au budget.
- **Réserve de traçabilité sur la CI : levée.** La branche de phase est à jour d'`origin` (`7e73df3`), le run `39824366948` est vert sur les trois jobs, et le premier run rouge `39794175404` a motivé ADR-0045 et REFERENCE-EXACTE-SANS-BLAS. Le correctif (référence exacte sans BLAS) tient donc sur Linux x86_64 comme sur macOS arm64.
- **Décision ouverte — budget des filtres d'intervalle (MES-14, hors critère de J1).** `docs/03` annonce « Filtre (bitmaps) < 1 ms » et renvoie à MES-14. La mesure tient ce budget pour les clauses en bitmap (209,6 µs au pire cas à 8,58 M) mais le dépasse de 101 à 194 ms pour une clause d'intervalle à domaine plein, et de 9,1 à 49,2 ms sous domaine réduit. Cinq recommandations de `@mesureur` attendent le propriétaire, aucune n'est appliquée : (1) séparer la ligne de budget en deux dans `docs/03` ou donner une cible distincte à l'intervalle ; (2) ouvrir un ADR « index de plage », ce qu'ADR-0018 prévoyait ; (3) à défaut, assumer le dépassement et le porter dans les budgets de bout en bout ; (4) tâche à part pour l'insertion dans la bitmap de résultat (~13 ms sur 21,1 ms dans un cas mesuré, calcul et non mesure) ; (5) aucun seuil `N` ni `exact_threshold` ne s'en déduit. Tant que ce n'est pas tranché, `docs/03` peut laisser croire que le budget vaut aussi pour un intervalle.
- **Décision ouverte — phase de MES-13.** La fiche CONSTRUIRE-BITMAPS cite MES-13 (« premier chiffre »), `docs/08` la classe en P3, et le rapport MES-13 se présente lui-même comme le premier chiffre de P3. Le chiffre existe désormais ; il reste à aligner la fiche et la table de `docs/08`. Recommandation de `@mesureur`, à confirmer : ne pas ouvrir de cache de bitmaps sur ce chiffre (20,1 s contre un budget de 60 s), et trancher en P3 sur l'échantillon Legifacile.
- **Recommandations MES-15 encore ouvertes** (notées en idées, non appliquées) : dérivation incrémentale de la bitmap des vivants (86,5 % du commit), cache des `SegmentMeta` au-delà de quelques centaines de segments, compresseur zstd réutilisé (≈ 6 ms par commit).
- **État git annexe** : `phase/P0-fondations` est en avance de 2 commits sur `origin/phase/P0-fondations` (les commits de validation de J0 ne sont pas poussés). Rien à voir avec la qualité de P1, mais `@git` doit le savoir avant tout push.
- **Risques constatés** : aucun risque fonctionnel nouveau. Les points de robustesse connus restent notés en idées hors périmètre : marges des tests de `busy_timeout`, `inspect` et `search` qui exigent un montage inscriptible, `ReadPool::verify` partiel, ordre textuel `doc_id#ord` contre `PassageKey::Ord`, valeur de date recopiée dans un message d'erreur de `check_scalar`, `Makefile` sans `--locked`.
- **Points à décider par le propriétaire** :
  1. le budget des filtres d'intervalle (MES-14) : séparer la ligne de `docs/03`, ouvrir un ADR « index de plage », ou assumer le dépassement — et ouvrir ou non la tâche d'insertion en bitmap ;
  2. la phase de MES-13 (fiche CONSTRUIRE-BITMAPS contre `docs/08`) et la confirmation qu'aucun cache de bitmaps n'est ouvert avant P3 ;
  3. les recommandations MES-15 (bitmap des vivants incrémentale, cache des `SegmentMeta`, compresseur zstd réutilisé) ;
  4. la validation du jalon lui-même, par `VALIDE` dans `docs/suivi.md`.

## 8. Si le jalon n'est pas validé

Aucun manque bloquant ne subsiste : les deux du premier passage sont levés et les contrôles sont verts. Si le propriétaire refuse malgré tout la validation, les points restants deviennent des tâches `CORRIGER-…` dans `docs/phases/P1-stockage-et-recherche-exacte.md` :

- `CORRIGER-BUDGET-FILTRES` (si la décision 1 va dans ce sens) : séparer la ligne de budget de `docs/03` et, si un index de plage est retenu, ouvrir l'ADR correspondant. Critère : `docs/03` distingue explicitement le budget des clauses en bitmap et celui des clauses d'intervalle, avec les chiffres de MES-14.
- `CORRIGER-MES-13-FICHE` (si la décision 2 va dans ce sens) : aligner la mention de MES-13 dans la fiche CONSTRUIRE-BITMAPS sur `docs/08`. Critère : la fiche et la table de `docs/08` donnent la même phase pour MES-13.
