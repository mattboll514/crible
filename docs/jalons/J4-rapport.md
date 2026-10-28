# Rapport de jalon J4 — Recherche rapide

> Rédigé par l'agent `jalon` (commande `/jalon J4`). **Seul le propriétaire peut valider le jalon**, dans `docs/suivi.md`.

- **Date** : 2026-10-28
- **Branche de phase / commit** : `phase/P4-recherche-rapide` / `b684db1` (`docs(adr): accept ADR-0084 and apply the owner's P4 document fixes`), arbre propre
- **Machine** : Apple M3 (8 cœurs, 24 Gio de RAM), macOS 15.7.7, arm64 ; Rust 1.99.1 (`rustc 1.99.1 (58cf8f9c3 2026-10-13)`)
- **Verdict de l'agent** : `PRET-POUR-VALIDATION`

Les cinq points du jalon de la fiche P4 sont couverts : MES-01 atteint la cible de fidélité (recall@10 global **0,9977** sans filtre et **0,9989** en filtrée, à N = 5000) ; MES-02, MES-03, MES-09 et MES-12 sont consignées ; le N par défaut vaut 5000 (ADR-0078, Q-01 close) ; INV-06 est vert ; `group_by` et `dedup` fonctionnent sur la démo. Les 14 tâches de la phase sont `FAIT`, les contrôles de qualité sont verts et aucun ADR n'est `Proposé`.

## 1. Tâches de la phase

`docs/suivi.md` porte 14 lignes pour P4, toutes `FAIT` : les 10 tâches de la fiche P4 et 4 tâches hors plan (`UNIFIER-PARCOURS-SCAN`, `REDUIRE-COUT-DES-CLES`, `GROUPER-LECTURES-VECTEURS`, `NETTOYER-P4`). Les commits sont les fusions sur la branche de phase.

| Tâche | Statut dans `suivi.md` | Commit | Remarque |
|---|---|---|---|
| QUANTIFIER-BINAIRE | FAIT | `fb08184` | `codes.bin` sans en-tête, reconstruction à l'ouverture, codes en RAM (ADR-0069) |
| NOYAUX-SIMD | FAIT | `4106d56` | Noyaux NEON/AVX2 et repli scalaire, code maison (ADR-0070) |
| SCANNER-POOL-BINAIRE | FAIT | `b559cd4` | Top-N par distance de Hamming, départage par iid |
| RESCORER-FP16 | FAIT | `4c7dd6c` | Rescoring exact des N candidats, calcul partagé avec EXACT (ADR-0072) |
| INSERER-POOL-EN-ORDRE | FAIT | `366c5d0` | Bitmap de résultat construite en ordre croissant d'iid ; MES-14 rejouée |
| ACCELERER-RECETTE | FAIT | `b389ceb` | Ventilation de la recette ; encodage par lots mesuré puis retiré (ADR-0073) |
| PLANIFIER-STRATEGIE | FAIT | `5e85b24` | EXACT/BINARY selon `exact_threshold`, option `--exact` (ADR-0074) |
| REGROUPER-RESULTATS | FAIT | `f18d91a` | `dedup` puis `group_by`, sur-échantillonnage (ADR-0075) |
| MESURER-FIDELITE-BINAIRE | FAIT | `ef9de37` | MES-01 et MES-12 ; recommandation N = 5000 (ADR-0076) |
| MESURER-LATENCE-DENSE | FAIT | `5f99899` | MES-02, MES-03 et MES-09 (ADR-0077) |
| UNIFIER-PARCOURS-SCAN | FAIT | `1b1f283` | Parcours clairsemé unique avec curseur de segment (ADR-0081, ADR-0082) |
| REDUIRE-COUT-DES-CLES | FAIT | `07a82d7` | Clés lues après le tri par score ; (2a) et (2b) retirées (ADR-0079, ADR-0083) |
| GROUPER-LECTURES-VECTEURS | FAIT | `8ca5a1e` | Lecture groupée et parallèle des lignes fp16 (ADR-0080) |
| NETTOYER-P4 | FAIT | `70baa7f` | Trois passages de `@dev`, cinq relectures ; ADR-0084 rédigé puis accepté |

Aucune tâche de la phase n'est `A-FAIRE`, `EN-COURS` ou `BLOQUE`.

## 2. Qualité

Exécuté sur `phase/P4-recherche-rapide`, au commit `b684db1`, le 2026-10-28.

| Contrôle | Commande | Résultat |
|---|---|---|
| Formatage | `cargo fmt --all -- --check` | OK, code de sortie 0 |
| Lints | `cargo clippy --locked --all-targets -- -D warnings` | OK, 0 avertissement |
| Lints sous `failpoints` | `cargo clippy --locked --all-targets --features failpoints -- -D warnings` | OK, 0 avertissement |
| Tests Rust | `cargo test --locked --workspace` | **648 passés, 0 échoué, 24 ignorés** (≈ 5 min 51 s) |
| Tests Rust sous `failpoints` | `cargo test --locked --workspace --features failpoints` | **675 passés, 0 échoué, 25 ignorés** (≈ 5 min 24 s) |
| Tests Python des outils | `cd tools && uv run --no-sync pytest -q` | **413 passés, 1 sauté**, 567,19 s |
| Tests Python du client | `cd clients/python && uv run --no-sync pytest -q` | **65 passés, 1 sauté**, 0,37 s |
| Tests ONNX Runtime | `ORT_DYLIB_PATH=… cargo test --locked --release -p crible --test recette_import -- --include-ignored` | **11 passés, 0 échoué**, 296,33 s |
| Tests ONNX Runtime | `ORT_DYLIB_PATH=… cargo test --locked --release -p crible-encode --test accelere_recette -- --include-ignored` | **2 passés, 0 échoué**, 13,10 s |

Précisions :

- **24 ignorés** en configuration par défaut : les tests qui demandent ONNX Runtime ou les données locales de l'échantillon Legifacile, plus le processus enfant des tests de panne. Aucun test d'invariant n'est sauté.
- **25 ignorés sous `failpoints`** : les 24 précédents plus `crash_child_entry`, marqué `#[ignore]` par construction.
- **Aucun `uv sync` n'a été lancé** : `uv run --no-sync` partout.
- Les deux séries de tests ONNX Runtime couvrent les invariants INV-11 et INV-12 de la phase ; le test local de l'échantillon Legifacile d'INV-12 (`inv_12_the_legifacile_sample_passes_the_acceptance_check`) est passé.

## 3. Invariants de la phase

Les fiches de P4 citent INV-06 (PLANIFIER-STRATEGIE, UNIFIER-PARCOURS-SCAN, REDUIRE-COUT-DES-CLES, GROUPER-LECTURES-VECTEURS), INV-07 (INSERER-POOL-EN-ORDRE), INV-11 et INV-12 (ACCELERER-RECETTE) et INV-15 (QUANTIFIER-BINAIRE). Chacun a un test nommé `inv_xx_*`, retrouvé par `rg` et lancé ici.

| Invariant | Test(s) | Résultat |
|---|---|---|
| INV-06 — un pool sous le seuil exact donne le résultat exact | `inv_06_dense_exact_synth` (13), `planifier_strategie` (17), `dense_exact` (17), `reduire_cout_des_cles` (11), `grouper_lectures_vecteurs` (5) ; dont `inv_06_synth_small_200_queries_match_the_numpy_reference`, `inv_06_synth_small_200_queries_forced_exact_match_the_p1_reference`, `inv_06_pool_below_the_threshold_returns_the_exact_result`, `inv_06_exact_requested_above_the_threshold_returns_the_exact_result`, `inv_06_binary_rescoring_of_a_filtered_pool_returns_the_exact_result`, `inv_06_multi_segment_pool_fully_rescored_returns_the_exact_result`, `inv_06_ties_are_broken_by_ascending_key` | **63 passés, 0 échoué** au total sur les cinq fichiers ciblés |
| INV-07 — les filtres évalués par bitmap valent l'évaluation SQLite | `inv_07_filter_bitmap_matches_sql` (2) et `inv_07_filter_semantics` (5) | **7 passés, 0 échoué** |
| INV-11 — la longueur d'un passage ne dépend pas des autres | `inv_11_the_token_count_of_a_passage_does_not_depend_on_the_other_passages`, `inv_11_the_real_model_token_count_of_the_reference_texts_is_pinned` (ONNX Runtime) ; plus les tests `inv_11_*` de `controler_longueurs` dans la série workspace | **2 passés, 0 échoué** en `--release` avec ONNX Runtime ; **5 passés** pour `controler_longueurs` dans la série workspace |
| INV-12 — un lot incompatible est refusé, la recette garde son contrat | `recette_import.rs` : 8 tests `inv_12_*`, dont `inv_12_the_legifacile_sample_passes_the_acceptance_check` et `inv_12_the_reference_batch_acceptance_statistics_are_unchanged` | **11 passés, 0 échoué** en `--release` avec ONNX Runtime (296,33 s) |
| INV-15 — les codes binaires et les formats versionnés restent valides | `inv_15_codes_bin` (14), `inv_15_codes_rebuild_crash` (1, sous `failpoints`), `inv_15_segment_vectors` (8), `inv_15_records_format_version` (5) ; plus 3 tests `inv_15_*` unitaires ou de restauration dans la série workspace | **28 passés, 0 échoué** sur les quatre fichiers ciblés ; **31 noms `inv_15_*`** au total, tous verts dans la série workspace |

Le jalon demande explicitement **INV-06** : il est vert, et le rejeu de la démo le montre aussi sur `synth-small` (pool 9750 sous le seuil 20 000, résultat identique à `--exact` au bit près).

## 4. Mesures de la phase

`docs/08-invariants-et-tests.md` classe en P4 MES-01, MES-02, MES-03, MES-09 et MES-12 ; MES-14 est rejouée par INSERER-POOL-EN-ORDRE.

| Mesure | Rapport | Chiffre clé | Cible | Atteinte ? |
|---|---|---|---|---|
| MES-01 — fidélité du scan binaire | `docs/mesures/MES-01-fidelite-scan-binaire.md` (campagne du 2026-10-27) | recall@10 global **0,9977** sans filtre et **0,9989** en filtrée à N = 5000 ; recouvrement@10 document 0,9983 et 0,9989 | **≥ 0,99** en moyenne globale au N par défaut (N = 5000, ADR-0078) | **Oui** dans les deux campagnes et pour toutes les structures ; borne basse à 95 % 0,9953 et 0,9976. Confirmation à 8,58 M renvoyée à P9 |
| MES-02 — latence du scan binaire | `docs/mesures/MES-02-latence-scan-binaire.md` et `MES-02-parcours-dense-clairseme.md` (campagnes du 2026-10-27) | pool complet de 7,74 M vivants : **40,0 ms p50 / 44,1 ms p95** ; 1 M contigu 4,9 / 5,4 ms ; 1 M dispersé **11,8 / 14,1 ms** ; parcours unique avec curseur : **50 points A/B, 30 gagnés, 20 nuls, 0 perdu** ; budget p95 8,34–8,67 ms à 1 M et 40,1–41,6 ms au complet | Budget `docs/03-recherche.md:143` : **20 à 80 ms** au pool complet, « quelques ms » à 1 M | **Oui au pool complet.** Le 1 M dispersé (**11,8 ms p50**) reste au-dessus de « quelques ms » : écart déclaré, non un critère de J4 (la fiche demande la consignation). Le critère A/B d'ADR-0082 est tenu |
| MES-03 — latence du rescoring fp16 | `docs/mesures/MES-03-latence-rescoring.md` (campagnes du 2026-10-27 et du 2026-10-28) | variante livrée `grouped` : à chaud **0,73–0,79 ms p50** à N = 1000 et **2,07–2,17 ms** à N = 5000 ; à froid partiel **20,3–24,5 ms** à N = 1000 et **29,2–33,0 ms** à N = 5000 ; gain contre `base` −90,7 % / −81,2 % à chaud, −64,1 % / −64,7 % / −72,0 % à froid ; après 10 min −68,0 % et −66,2 % | « MES-03 consignée » (le budget indicatif « < 1 ms à chaud » a été remplacé par MES-03, ADR-0078) | **Oui, consignée.** Le budget indicatif de 1 ms à chaud à N = 1000 est tenu par la variante groupée (0,73–0,79 ms p50). La série froide `F_NOCACHE` n'est pas rejouée ; **ADR-0084 (accepté le 2026-10-28)** déclare qu'elle n'est pas un critère en P4 et renvoie le froid vrai à P8 |
| MES-09 — RAM réelle | `docs/mesures/MES-09-ram-reelle.md` (campagne du 2026-10-27 et section du 2026-10-28) | index de 7,74 M passages : **4,63 à 4,83 Gio de RSS**, dont 3,80 à 4,00 Gio de pages de fichiers ; **1,12 Gio possédés** (1 126 Mio) ; instance avec session fp32 **2,20 à 2,30 Gio** ; la variante groupée ne change pas l'empreinte (1 126 Mio) | Tableau de `docs/02-architecture.md:202-219` | **Oui** ligne à ligne ; la dernière ligne vaut 2,20–2,30 Gio au lieu de « ~2,0 Go », même ordre de grandeur, arrondi de la conception |
| MES-12 — centrage des codes binaires | `docs/mesures/MES-12-centrage-codes-binaires.md` (campagne du 2026-10-27) | centrage principal (a) : **+0,0010** à N = 5000 sans filtre, **−0,0004** en filtrée ; variante asymétrique (c) : +0,0083 à N = 2000 | Aucune cible ; recommandation de centrage | **Sans objet comme cible** : recommandation rendue, **signe brut conservé** (aucun centrage) à N = 5000, conforme à ADR-0078 |
| MES-14 — coût des filtres d'intervalle, rejouée en P4 | `docs/mesures/MES-14-cout-filtres-intervalles.md` (§ 10, campagne du 2026-10-26) | à 8,58 M passages, **−15,5 à −31,9 % en p50** sur les huit cas qui insèrent ; insertion de **6,7 à 3,5 ns par iid gardé** ; parcours inchangé (8,9 ns par iid lu) | Aucune cible ; ADR-0046 | **Sans objet comme cible** ; le gain est consigné, aucune clause sous 1 ms, ADR-0046 non rouvert |

Aucune mesure de la phase n'est manquante. Le seul écart de budget est le pool dispersé de 1 M de MES-02, déjà déclaré par le rapport de mesure et hors du critère de J4.

## 5. Démo à rejouer par le propriétaire

Aucun `docs/jalons/J4-demo.md` n'existait : les commandes ci-dessous ont été **construites puis exécutées** sur la branche de phase le 2026-10-28, dans l'ordre, et les sorties citées sont celles observées. Elles montrent les points de démonstration du jalon : stratégie EXACT/BINARY et INV-06, `group_by`, `dedup`, puis le test d'invariant.

**Prérequis** : `data/synth-small/` (lots `l1`, `l2`, `l3` et `queries.jsonl`), `data/models/synthetic-external/`, et les paquets Python du client (`clients/python`). Tout est gitignoré et déjà présent sur cette machine. La démo crée ses propres données sous `data/j4-demo/` et ne touche ni `data/legifacile-sample/`, ni les index de mesure conservés.

**Préambule — état propre et binaire.**

```sh
cd /Users/mattboll/MattCorp/crible
rm -rf data/j4-demo /tmp/j4
mkdir -p /tmp/j4 data/j4-demo/configs
cargo build --release --locked
```

Attendu : `Finished`. Les index de mesure de `data/mes01`, `data/mes02` et des tâches précédentes ne sont pas touchés.

**Étape 1 — trois configurations de démonstration.** La graine est `data/synth-small/index.json` ; seule la stratégie change.

```sh
python3 - <<'PY'
import json
from pathlib import Path
base = json.loads(Path('data/synth-small/index.json').read_text())
for path, name, threshold in [
    ('data/j4-demo/configs/j4-exact.json', 'synth-small', 20000),
    ('data/j4-demo/configs/j4-binary.json', 'synth-small', 0),
    ('data/j4-demo/configs/j4-dedup.json', 'j4-dedup', 0),
]:
    config = json.loads(json.dumps(base))
    config['name'] = name
    config['search'] = dict(config['search'])
    config['search']['scan'] = 'binary'
    config['search']['exact_threshold'] = threshold
    config['search']['rescore_candidates'] = 5000
    Path(path).write_text(json.dumps(config, indent=2, sort_keys=True, ensure_ascii=False) + '\n')
print('three configurations written')
PY
```

Attendu : `three configurations written`. Le pool de `synth-small` vaut 9750 passages vivants : il est **sous** le seuil 20 000 (index `j4-exact`, donc EXACT) et **au-dessus** du seuil 0 (index `j4-binary`, donc BINARY). N vaut 5000 partout (ADR-0078).

**Étape 2 — un petit lot pour `dedup`.** `synth-small` n'a aucun contenu dupliqué ; ce lot en porte trois copies identiques et trois passages distincts.

```sh
cd clients/python
uv run --no-sync python - <<'PY'
import json, blake3, numpy as np
from pathlib import Path
from crible.batch import BatchWriter

root = Path('/Users/mattboll/MattCorp/crible')
config = json.loads((root / 'data/j4-demo/configs/j4-dedup.json').read_text())
queries = [json.loads(line) for line in (root / 'data/synth-small/queries.jsonl').read_text().splitlines()]
vectors = [np.asarray(q['vector'], dtype=np.float32) for q in queries[:4]]
writer = BatchWriter(
    config, root / 'data/j4-demo/batches', 'j4-dedup', 'upsert',
    {"name": "synthetic-external", "kind": "external", "dims": 768, "normalize": True},
    {"name": "synthetic-units", "version": "1", "params": {"passages_per_document": 1}},
    {"name": "synthetic-text", "version": "1"},
    created_at='2026-10-28T10:00:00Z', host='j4-demo',
)

def add(doc_id, text, vector):
    hasher = blake3.blake3()
    hasher.update(b'\0'); hasher.update(text.encode()); hasher.update(b'')
    hasher.update(np.asarray(vector, dtype='<f2').tobytes())
    writer.add_document(doc_id, hasher.hexdigest(), {"categorie": "cat-01", "annee": 2020},
                        [{"text": text, "vector": [float(x) for x in vector]}])

for index in range(3):
    add(f'j4-dup-{index}', 'passage identique de la demonstration J4', vectors[0])
for index in range(3):
    add(f'j4-uniq-{index}', f'passage distinct numero {index} de la demonstration J4', vectors[index + 1])
print(writer.close())
PY
cd ../..
```

Attendu : le chemin `.../data/j4-demo/batches/j4-dedup`.

**Étape 3 — les trois index et leurs attaches.**

```sh
B=./target/release/crible
$B index create --data data/j4-demo/root-exact --config data/j4-demo/configs/j4-exact.json
for l in l1 l2 l3; do $B attach --data data/j4-demo/root-exact --models models --index synth-small data/synth-small/$l; done
$B inspect --data data/j4-demo/root-exact --index synth-small
$B index create --data data/j4-demo/root-binary --config data/j4-demo/configs/j4-binary.json
for l in l1 l2 l3; do $B attach --data data/j4-demo/root-binary --models models --index synth-small data/synth-small/$l; done
$B index create --data data/j4-demo/root-dedup --config data/j4-demo/configs/j4-dedup.json
$B attach --data data/j4-demo/root-dedup --models models --index j4-dedup data/j4-demo/batches/j4-dedup
```

Attendu (observé à l'identique) :

```text
index 'synth-small' created at data/j4-demo/root-exact/indexes/synth-small (epoch 1, generation 0)
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
  codes: 1152000 bytes
index 'synth-small' created at data/j4-demo/root-binary/indexes/synth-small (epoch 1, generation 0)
batch 'l1' attached to index 'synth-small': generation 1 (added 2000, replaced 0, unchanged 0, metadata_updated 0, deleted 0, ignored 0)
batch 'l2' attached to index 'synth-small': generation 2 (added 100, replaced 300, unchanged 0, metadata_updated 0, deleted 0, ignored 0)
batch 'l3' attached to index 'synth-small': generation 3 (added 0, replaced 0, unchanged 0, metadata_updated 50, deleted 150, ignored 0)
index 'j4-dedup' created at data/j4-demo/root-dedup/indexes/j4-dedup (epoch 1, generation 0)
batch 'j4-dedup' attached to index 'j4-dedup': generation 1 (added 6, replaced 0, unchanged 0, metadata_updated 0, deleted 0, ignored 0)
```

`codes: 1152000 bytes` est la mémoire des codes binaires affichée par `crible inspect` (9750 passages × 12 mots de 8 octets, bourrage compris).

**Étape 4 — la requête de référence.**

```sh
python3 - <<'PY'
import json
from pathlib import Path
query = json.loads(Path('data/synth-small/queries.jsonl').read_text().splitlines()[0])
Path('/tmp/j4/query.json').write_text(json.dumps(query['vector']))
print('query_vector_dims', len(query['vector']))
PY
```

Attendu : `query_vector_dims 768`.

**Étape 5 — EXACT sous le seuil, et INV-06.**

```sh
$B search --data data/j4-demo/root-exact --index synth-small --vector-file /tmp/j4/query.json --k 10 > /tmp/j4/e-default.json
$B search --data data/j4-demo/root-exact --index synth-small --vector-file /tmp/j4/query.json --k 10 --exact > /tmp/j4/e-forced.json
python3 - <<'PY'
import json
from pathlib import Path
def load(name): return json.loads(Path('/tmp/j4/' + name).read_text())
a, b = load('e-default.json'), load('e-forced.json')
ka = [(r['key'], r['score']) for r in a['results']]
kb = [(r['key'], r['score']) for r in b['results']]
print('pool', a['pool_size'], 'top3', [r['key'] for r in a['results'][:3]])
print('default == --exact:', ka == kb, '| max abs diff', max(abs(x[1] - y[1]) for x, y in zip(ka, kb)))
PY
```

Attendu, et **observé** : `pool 9750 top3 ['synth-01163#3', 'synth-00407#0', 'synth-01703#2']` puis `default == --exact: True | max abs diff 0.0`. C'est INV-06 : le pool est sous `exact_threshold`, donc la planification choisit EXACT et les deux réponses sont identiques.

**Étape 6 — BINARY au-dessus du seuil, puis `--exact`.**

```sh
$B search --data data/j4-demo/root-binary --index synth-small --vector-file /tmp/j4/query.json --k 10 > /tmp/j4/b-default.json
$B search --data data/j4-demo/root-binary --index synth-small --vector-file /tmp/j4/query.json --k 10 --exact > /tmp/j4/b-forced.json
python3 - <<'PY'
import json
from pathlib import Path
def load(name): return json.loads(Path('/tmp/j4/' + name).read_text())
b, e = load('b-default.json'), load('b-forced.json')
kb = [r['key'] for r in b['results']]
ke = [r['key'] for r in e['results']]
print('pool', b['pool_size'], 'top3', kb[:3])
print('binary top-10 == exact top-10:', kb == ke, '| overlap', len(set(kb) & set(ke)), '/', len(ke))
PY
```

Attendu, et **observé** : `pool 9750 top3 ['synth-01163#3', 'synth-00407#0', 'synth-01703#2']` puis `binary top-10 == exact top-10: True | overlap 10 / 10`. Un seul point ne suffit pas à conclure sur la fidélité : MES-01 le fait sur 300 requêtes réelles (§ 4).

**Étape 7 — fidélité BINARY contre EXACT sur les 200 requêtes de `synth-small`.** Ce contrôle est indicatif, il ne remplace pas MES-01.

```sh
python3 - <<'PY'
import json, subprocess
from pathlib import Path
B = './target/release/crible'
queries = [json.loads(line) for line in Path('data/synth-small/queries.jsonl').read_text().splitlines()]
def top10(root, vector, extra):
    path = Path('/tmp/j4/loop-query.json')
    path.write_text(json.dumps(vector))
    out = subprocess.run([B, 'search', '--data', root, '--index', 'synth-small',
                          '--vector-file', str(path), '--k', '10'] + extra,
                         capture_output=True, text=True, check=True).stdout
    return [r['key'] for r in json.loads(out)['results']]
hits = total = identical = 0
for query in queries:
    exact = top10('data/j4-demo/root-exact', query['vector'], ['--exact'])
    binary = top10('data/j4-demo/root-binary', query['vector'], [])
    hits += len(set(exact) & set(binary)); total += len(exact)
    identical += exact == binary
print('queries', len(queries), '| recall@10 binary vs exact', round(hits / total, 4),
      '| identical top-10 orders', identical)
PY
```

Attendu, et **observé** : `queries 200 | recall@10 binary vs exact 0.998 | identical top-10 orders 196`. À N = 5000 et sur des vecteurs synthétiques, quatre requêtes sur 200 changent l'ordre du top-10 sans perdre de passage.

**Étape 8 — `group_by: document` et `passages_per_document`.**

```sh
$B search --data data/j4-demo/root-binary --index synth-small --vector-file /tmp/j4/query.json \
    --k 5 --group-by document --passages-per-document 2 > /tmp/j4/grouped.json
python3 - <<'PY'
import json
from pathlib import Path
grouped = json.loads(Path('/tmp/j4/grouped.json').read_text())
documents = [r['key'].split('#')[0] for r in grouped['results']]
runs = []
for document in documents:
    if not runs or runs[-1] != document:
        runs.append(document)
print('rows', len(documents), '| distinct documents', len(set(documents)),
      '| entries of a document consecutive', len(runs) == len(set(documents)))
print('order', documents)
PY
```

Attendu, et **observé** : `rows 7 | distinct documents 5 | entries of a document consecutive True`, puis l'ordre `['synth-01163', 'synth-00407', 'synth-01703', 'synth-01703', 'synth-02087', 'synth-02087', 'synth-00087']`. `k` compte cinq documents ; deux d'entre eux rendent deux passages, et les entrées d'un document sont consécutives.

**Étape 9 — `dedup: content`.**

```sh
$B search --data data/j4-demo/root-dedup --index j4-dedup --vector-file /tmp/j4/query.json --k 6 > /tmp/j4/d-off.json
$B search --data data/j4-demo/root-dedup --index j4-dedup --vector-file /tmp/j4/query.json --k 6 --dedup content > /tmp/j4/d-on.json
python3 - <<'PY'
import json
from pathlib import Path
def load(name): return json.loads(Path('/tmp/j4/' + name).read_text())
off, on = load('d-off.json'), load('d-on.json')
print('without dedup: rows', len(off['results']), '| distinct texts', len({r['text'] for r in off['results']}))
print('with dedup:    rows', len(on['results']), '| distinct texts', len({r['text'] for r in on['results']}))
print('with dedup keys', [r['key'] for r in on['results']])
PY
```

Attendu, et **observé** :

```text
without dedup: rows 6 | distinct texts 4
with dedup:    rows 4 | distinct texts 4
with dedup keys ['j4-dup-0#0', 'j4-uniq-1#0', 'j4-uniq-2#0', 'j4-uniq-0#0']
```

Les trois passages de texte identique sont fusionnés en un seul ; le premier d'entre eux, par le départage déterministe, est `j4-dup-0#0`.

**Étape 10 — le test d'invariant INV-06.**

```sh
cargo test --locked -p crible-search --test inv_06_dense_exact_synth
```

Attendu : **13 passés, 0 échoué**, 9,09 s, dont les 200 requêtes de `synth-small` contre la référence numpy et le même jeu avec `exact: true`.

**Nettoyage.**

```sh
rm -rf data/j4-demo /tmp/j4
```

Seuls ces chemins sont retirés ; `data/synth-small/`, `data/legifacile-sample/`, `data/models/` et les index de mesure conservés restent en place.

**Bilan de la démo** : les dix blocs ont été exécutés dans l'ordre, sans écart. La seule valeur variable est `took_ms`, absente des comparaisons.

## 6. Décisions (ADR) de la phase

ADR créés ou modifiés pendant P4, d'après `git log` sur `docs/adr/` et l'index `docs/adr/README.md` :

| ADR | Titre | Statut | Bloque la validation ? |
|---|---|---|---|
| ADR-0069 | Reconstruction et format de `codes.bin` | Accepté (2026-10-26) | Non |
| ADR-0070 | Noyaux SIMD : code maison dans `crible-search::kernels` | Accepté (2026-10-26) ; erratum du 2026-10-28 sur la tolérance relative | Non |
| ADR-0071 | Seuil entre parcours dense et clairsemé du scan binaire | **Remplacé par ADR-0081** | Non |
| ADR-0072 | Un seul calcul du produit scalaire pour EXACT et le rescoring | Accepté (2026-10-26, confirmé le 2026-10-27) | Non |
| ADR-0073 | Recette sans encodage par lots | Accepté (2026-10-27) | Non |
| ADR-0074 | Planification de la stratégie dense | Accepté (2026-10-27) | Non |
| ADR-0075 | Regroupement des résultats : ordre, sur-échantillonnage, forme | Accepté (2026-10-27) | Non |
| ADR-0076 | Protocole de mesure de fidélité du scan binaire | Accepté (2026-10-27) | Non |
| ADR-0077 | Protocole de mesure de latence de la recherche dense | Accepté (2026-10-27) | Non |
| ADR-0078 | N par défaut à 5000 et sort des index existants | Accepté (2026-10-27) ; tranche Q-01 | Non |
| ADR-0079 | Coût des clés au rescoring | Accepté (2026-10-27) | Non |
| ADR-0080 | Accès aux vecteurs fp16 au rescoring | Accepté (2026-10-27), précisé par ADR-0084 | Non |
| ADR-0081 | Parcours du scan binaire : un seul parcours clairsemé | Accepté (2026-10-27), remplace ADR-0071 | Non |
| ADR-0082 | Scan binaire multi-segments : curseur et vérification A/B | Accepté (2026-10-27, confirmé le 2026-10-28) | Non |
| ADR-0083 | Suites de REDUIRE-COUT-DES-CLES : seule (b) est livrée | Accepté (2026-10-28) | Non |
| ADR-0084 | Quel froid fait foi pour le rescoring jusqu'à P8 | Accepté (2026-10-28) | Non |

**Aucun ADR n'est `Proposé`.** Q-01 (« N par défaut et cible de fidélité du scan binaire ») est close par ADR-0078 : N = 5000, cible atteinte sur l'échantillon, confirmation à 8,58 M en P9.

Un point de forme reste ouvert pour le propriétaire : ADR-0078 a été accepté avec la mention « mécanisme du défaut et sort des index existants retenus par l'orchestrateur sur recommandation de `@architecte`, **à confirmer** ». Le code les applique (clé `search.exact_threshold` toujours obligatoire dans `index.json`, défaut documentaire et de gabarit à 5000, aucune migration des index existants). La confirmation finale de ce mécanisme, et non la valeur 5000, est la seule signature qui manque.

## 7. Écarts, idées, risques

- **Écarts à la conception** : aucune ligne P4 de la table « Écarts à la conception » n'est sans décision. Les écarts constatés pendant la phase ont tous été tranchés par un ADR `Accepté` (ADR-0069 à ADR-0084), y compris l'écart du 2026-10-28 sur la série froide `F_NOCACHE` de MES-03, **clos par ADR-0084**.
- **Risque — fidélité à l'échelle** : MES-01 mesure des pools d'au plus 50 000 passages, contre 8,58 M en production. La confirmation de N = 5000 est renvoyée à P9 (`legifacile-full`) ; aucune extrapolation n'est faite ici.
- **Risque — budget du scan sur un pool dispersé** : à 1 M de passages dispersés, MES-02 mesure 11,8 ms p50 et 14,1 ms p95, au-dessus de « quelques ms ». Le pool complet reste dans la fourchette 20 à 80 ms. Le sujet appartient à P8 (conteneur) et à la revue du budget de `docs/03`, pas à J4.
- **Risque — froid du rescoring** : le froid publié est partiel (résidence moyenne 0,78 à 0,80 des pages à N = 5000) et la série `F_NOCACHE` n'est pas rejouable sur le moteur en P4 : la vérification blake3 à l'ouverture réchauffe le fichier. ADR-0084 le déclare et renvoie le froid vrai, sous Linux, à P8.
- **Risque — mémoire** : les chiffres de MES-09 valent pour `synth-large` (10 colonnes de valeurs, aucun index lexical). Un index multi-segments, compacté ou lexical n'est pas mesuré ; le coût des colonnes (34 Mo par colonne à 8,58 M) est le premier facteur à recalculer pour Legifacile.
- **Points à décider par le propriétaire** :
  1. confirmer le **mécanisme** d'ADR-0078 (défaut documentaire et de gabarit, clé obligatoire, aucune migration), la valeur 5000 étant déjà décidée ;
  2. décider du sort des données de mesure conservées à la validation de J4 (`data/mes02`, copies multi-segments, `data/ab-old`, `data/reduire-cles`, `data/mes03`, `data/grouper-vecteurs`, `data/mes01`, `data/mes13` à `mes15`), comme la table « Données de mesure conservées » l'indique ;
  3. valider le jalon lui-même, par `VALIDE` dans `docs/suivi.md`.
- **Mineurs restants** : la section « Contenu de NETTOYER-P4 » garde des points mineurs de documentation et d'outils (rapports MES-01, MES-12 et MES-03, harnais, points de performance non mesurés). Aucun ne change un chiffre ni un contrat de la phase ; ils sont proposés ci-dessous en tâche de nettoyage.

## 8. Si le jalon n'est pas validé

Aucun manque bloquant n'est constaté sur les tâches, les tests, les invariants, les mesures ou la démo. Si le propriétaire refuse malgré tout la validation, les points restants deviennent des tâches `CORRIGER-…` dans `docs/phases/P4-recherche-rapide.md` :

- `CORRIGER-MINEURS-J4` (facultative) : traiter les mineurs encore ouverts de « Contenu de NETTOYER-P4 » qui ne demandent ni mesure ni ADR (rapports MES-01, MES-12 et MES-03, harnais `tools/bench`, commentaires de code). Critère : chaque point restant est soit corrigé, soit explicitement renvoyé à P8 ou P9 avec sa justification ; `cargo test --locked --workspace`, `--features failpoints`, `cargo fmt` et `clippy` restent verts.
- `CORRIGER-BUDGET-SCAN-1M` (seulement si le propriétaire veut trancher avant P8) : documenter dans `docs/03-recherche.md` que le budget « quelques ms » vaut pour un pool contigu et que le pool dispersé de 1 M mesure 11,8 ms p50. Critère : la ligne de budget distingue contigu et dispersé, avec renvoi à MES-02 ; aucun code modifié.
- `CORRIGER-ADR-0078-MECANISME` : sans objet si le propriétaire confirme le mécanisme au point 1 du § 7. Sinon, corriger la mention « à confirmer » de l'ADR et l'index. Critère : plus aucune mention « à confirmer » dans ADR-0078 ni dans son entrée d'index.
