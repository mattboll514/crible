# Rapport de jalon J7 — Observabilité

> Rédigé par l'agent `jalon` (commande `/jalon J7`). **Seul le propriétaire peut valider le jalon**, dans `docs/suivi.md`.

- **Date** : 2026-11-02
- **Branche de phase / commit** : `phase/P7-observabilite` / `3ca63b7`
- **Machine** : MacBook Air M3 (`Mac15,12`), Apple M3 à 8 cœurs, 24 Gio de mémoire, macOS 15.7.7, exécution native (pas de conteneur)
- **Verdict de l'agent** : `PRET-POUR-VALIDATION`, avec un écart de rédaction dans `docs/jalons/J7-demo.md` (section 7) et des décisions ouvertes qui ne bloquent pas le jalon

Les quatre critères du jalon (`docs/phases/P7-observabilite.md:5-9`) sont tenus : `crible eval` produit un rapport complet pour les trois modes sur l'échantillon, `explain_ids` dit pourquoi un document absent n'est pas remonté, `crible stats` montre le trafic de la démo, et INV-13 et INV-17 sont verts. La CI distante est verte sur `3ca63b7` (run `41430988487`).

## 1. Tâches de la phase

Les 11 tâches de P7 suivies dans `docs/suivi.md` sont `FAIT`. Dix viennent de la fiche (`docs/phases/P7-observabilite.md`), et une, `NETTOYER-P7`, est hors plan. `COUPER-TELEMETRIE-ONNX` est aussi hors plan, ajoutée le 2026-11-02 et documentée par sa fiche en fin de phase. Aucune tâche de P7 n'est `A-FAIRE`, `EN-COURS` ou `BLOQUE`.

| Tâche | Statut dans `suivi.md` | Commit | Remarque |
|---|---|---|---|
| CORRIGER-ARRET-SOUS-CHARGE | FAIT | fusion `c646e45` | Hors plan ; cause dans le test, pas dans le serveur |
| AGREGER-ANALYTICS | FAIT | fusion `3b88ebc` | ADR-0117, ADR-0118 ; INV-17 |
| TESTER-ANALYTICS-SANS-CONTENU | FAIT | fusion `62c2986` | INV-13 |
| JOURNAL-DE-DEBUG | FAIT | fusion `598bfaa` | ADR-0121, ADR-0122 ; INV-17 ; rejeu de MES-16 |
| REPONSE-DEBUG | FAIT | fusion `a1d2a01` | ADR-0119, ADR-0120 |
| EXPLIQUER-DOCUMENTS | FAIT | fusion `00da2fd` | ADR-0123 |
| ROUTE-STATS | FAIT | fusion `13158e5` | ADR-0124 |
| COMMANDE-EVAL | FAIT | fusion `789abfa` | ADR-0125 ; mode API reporté |
| REUTILISER-LECTEURS-DENSES | FAIT | fusion `54b6ddb` | Hors plan ; ADR-0126 ; INV-02 |
| NETTOYER-P7 | FAIT | fusion `62c09da` | Hors plan, six passages A à F, chacun relu `OK` |
| COUPER-TELEMETRIE-ONNX | FAIT | fusion `e82b978` | Hors plan, 2026-11-02 ; ADR-0128 |

Deux tâches de P8 ont été ouvertes pendant la phase et ne sont pas des manques de J7 : `PLAFONNER-JOURNAL-DE-DEBUG` (ADR-0127) et `RESUME-PUBLIE-COHERENT`. Elles restent `A-FAIRE` en P8, sur décision du propriétaire.

## 2. Qualité

Toutes les commandes ont été lancées par `@jalon` le 2026-11-02 sur `3ca63b7`. La série `--release --include-ignored` a été lancée par l'orchestrateur au même commit et est reportée telle quelle, sans être rejouée.

| Contrôle | Commande | Résultat |
|---|---|---|
| Formatage | `cargo fmt --all -- --check` | exit 0, 1,16 s |
| Lints | `cargo clippy --all-targets --locked -- -D warnings` | exit 0, 9,08 s |
| Liens de documentation (ADR-0114) | `RUSTDOCFLAGS='-D warnings' cargo doc --no-deps --workspace --locked` | exit 0, 2,27 s |
| Tests Rust (debug) | `cargo test --locked --no-fail-fast`, paquet par paquet | **1 147 passés, 0 échoué, 26 ignorés**, sur 139 lignes `test result` |
| Lints `failpoints` (`crible-store`) | `cargo clippy -p crible-store --all-targets --locked --features failpoints -- -D warnings` | exit 0 |
| Tests `failpoints` (`crible-store`) | `cargo test -p crible-store --locked --features failpoints` | **389 passés, 0 échoué, 1 ignoré** |
| Lints `failpoints` (`crible-server`) | `cargo clippy -p crible-server --all-targets --locked --features failpoints -- -D warnings` | exit 0 |
| Tests `failpoints` (`crible-server`) | `cargo test -p crible-server --locked --features failpoints` | **312 passés, 0 échoué, 0 ignoré** |
| Tests `failpoints` (`crible`, contrôle supplémentaire) | `cargo test -p crible --locked --features failpoints` | **112 passés, 0 échoué, 16 ignorés** |
| Tests Python du client | `cd clients/python && uv run pytest` | **107 passés, 12 désélectionnés** (marqueur `server` exclu par défaut), 0,44 s |
| Tests Python des outils | `cd tools && uv run --no-sync pytest` | **521 passés, 1 sauté**, 396,72 s |
| Série complète avec ONNX Runtime (**orchestrateur**) | `ORT_DYLIB_PATH=$PWD/tools/.venv/lib/python3.13/site-packages/onnxruntime/capi/libonnxruntime.1.31.0.dylib cargo test --locked --workspace --release --no-fail-fast -- --include-ignored` | **1 173 passés, 0 échoué, 0 ignoré**, 139 lignes `test result` (134 binaires et 5 séries de doc-tests), 3 923 s dont 1 min 24 s de compilation |

Détail des tests en mode debug, paquet par paquet, puis somme :

| Paquet | Passés | Échoués | Ignorés | Lignes `test result` | Durée |
|---|---|---|---|---|---|
| `crible-core` | 179 | 0 | 1 | 12 | 2,14 s |
| `crible-encode` | 18 | 0 | 9 | 5 | 2,34 s |
| `crible-search` | 204 | 0 | 0 | 21 | 37,92 s |
| `crible-store` | 355 | 0 | 0 | 33 | 70,46 s |
| `crible-server` | 280 | 0 | 0 | 43 | 25,01 s |
| `crible` | 111 | 0 | 16 | 25 | 98,65 s |
| **Total** | **1 147** | **0** | **26** | **139** | — |

Les 1 147 tests verts et les 26 ignorés du mode debug rejoignent exactement les 1 173 de la série `--release --include-ignored` : aucune divergence de décompte.

Durées longues de la série de l'orchestrateur, toutes `ok` :

| Test | Durée |
|---|---|
| `crible/tests/api_and_cli_on_sample.rs` | 1 773 s |
| `inv_08_export_import_roundtrip.rs` | 1 070 s |
| `inv_09_compaction_equivalence.rs` | 614 s |
| `recette_import.rs` | 286 s |

Le test `json_examples_conform_to_the_specification` est vert sur ce commit, ce qui confirme le retrait des membres d'ADR-0127 de l'exemple `DebugQueryList` de `docs/05` (`3ca63b7`). Le test de télémétrie `a_text_search_creates_no_telemetry_file_under_a_temporary_home` est vert dans la série de l'orchestrateur, donc sans variable posée à la main.

## 3. Invariants de la phase

Les fiches de P7 citent INV-13 (`TESTER-ANALYTICS-SANS-CONTENU`), INV-17 (`AGREGER-ANALYTICS`, `JOURNAL-DE-DEBUG`) et INV-02 (`REUTILISER-LECTEURS-DENSES`, puisque le cache a été fait). Tous ces tests sont dans la suite ordinaire ou sous `failpoints` ; aucun ne demande l'échantillon Legifacile ni ONNX Runtime, donc aucun n'a été laissé à la série `--release --include-ignored`.

| Invariant | Test(s) | Résultat |
|---|---|---|
| **INV-13** — aucun contenu dans les analytics ni dans les logs | `crible-server/tests/inv_13_analytics_without_content.rs` : `inv_13_the_analytics_and_the_journal_hold_no_query_text_no_filter_value_and_no_result_id`, `inv_13_a_refused_search_leaks_neither_its_query_text_nor_its_filter_value` | **3 passés, 0 échoué**, 0,81 s |
| **INV-17** — le vidage des analytics ne bloque jamais une requête | `crible-server/tests/inv_17_analytics_never_blocks.rs` sous `--features failpoints` : 3 `inv_17_*` (écrivain retenu, file pleine, événement d'opération) | **3 passés, 0 échoué**, 2,77 s |
| **INV-17** — le journal de debug ne bloque jamais une requête | `crible-server/tests/inv_17_debug_journal_never_blocks.rs` sous `--features failpoints` : `inv_17_searches_answer_while_the_debug_journal_writer_is_held`, `inv_17_accepted_queries_are_all_written_after_the_release`, `inv_17_holding_the_debug_journal_does_not_hold_the_analytics` | **3 passés, 0 échoué**, 4,11 s |
| **INV-02** — une lecture n'est jamais bloquée par une écriture (cache des lecteurs denses) | `crible-server/tests/inv_02_dense_readers.rs` sous `failpoints` : 2 `inv_02_*` ; `crible-server/tests/inv_02_reads_during_writes.rs` sous `failpoints` : 4 `inv_02_*` ; `crible-store/tests/inv_02_compaction_reads.rs` sous `failpoints` : 2 `inv_02_*` | **8 passés, 0 échoué**, 1,04 s au total |

Le critère de charge d'INV-02 (chaque recherche sous 3 × le p99 du repos) n'est pas un test : il est prouvé par le rejeu de MES-16 (section 4).

## 4. Mesures de la phase

| Mesure | Rapport | Chiffre clé | Cible | Atteinte ? |
|---|---|---|---|---|
| MES-16 — charge concurrente avec analytics et journal de debug actifs (rejeu d'ADR-0122, décision 12) | `docs/mesures/MES-16-charge-concurrente-analytics-et-journal.md` et son JSON, campagne du 2026-11-01, MacBook Air M3, `--release`, `sample_rate = 1.0` | INV-02 tenu sur les trois campagnes : limites 465,34 / 465,24 / 463,71 ms (3 × p99 du repos 155,11 / 155,08 / 154,57 ms) ; pire cas 344,0 ms, soit 0,74 de la limite ; aucun `5xx` ; aucune perte du journal | Chaque recherche sous 3 × le p99 du repos, et p99 de la phase d'écriture sous le même seuil (ADR-0113, décision 1) | Oui |
| MES-16 — campagne de référence sans analytics ni journal | `docs/mesures/MES-16-charge-concurrente.md` et son JSON, campagnes du 2026-10-30 | Pire cas 0,61 de la limite ; aucun `5xx` | Même critère | Oui |
| Campagne `crible eval` de l'échantillon à N = 5000 | `docs/mesures/eval-<jeu>-2026-11-02.{json,md}`, six jeux, et la note `docs/mesures/eval-n5000-2026-11-02.md` | Fidélité moyenne 0,9977 en dense et 0,9917 en hybride ; `decisions` hybride seul à 0,9890, qui remonte à 1,0000 à `--rescore-candidates 50000` ; qualité `decisions` 0,5063 dense / 0,5800 lexicale / 0,5638 hybride | Fidélité ≥ 0,99 en moyenne au N par défaut de 5000 (`docs/08-invariants-et-tests.md`) | Oui en moyenne ; `decisions` hybride est l'unique structure sous la cible, expliquée par la troncature des candidats |
| Coût d'ouverture des lecteurs denses à 8,58 M passages (livrable 1 de REUTILISER-LECTEURS-DENSES) | `docs/mesures/lecteurs-denses-ouverture-2026-11-01.md` | p95 8 406,9 ms à l'ouverture, vérification blake3 des 13,2 Go comprise | Branche « négligeable » si p95 < 1 ms ; sinon cache (fiche) | Non négligeable : la branche cache a été suivie |
| Latence avant/après du cache des lecteurs denses (livrable 2) | `docs/mesures/lecteurs-denses-latence-2026-11-01.md` | `fetch` p50 10 845,140 → 3,333 ms et `total` p50 12 095,658 → 67,884 ms sur le pool filtré ; démarrage jusqu'à `/ready` 25,7 → 31,5 s ; critère des 10 % : écart maximal 0,130 % | Startup sous 60 s (ADR-0047) ; écart des temps sous 10 % (ADR-0119) ; aucune cible de latence pour cette tâche | Oui, avec deux écarts de protocole acceptés (100 requêtes « avant » au lieu de 300, cache de pages non atteint) |

## 5. Démo à rejouer par le propriétaire

Le scénario est décrit dans `docs/jalons/J7-demo.md`. Les commandes exactes suivent ; elles ont toutes été jouées le 2026-11-02 par `@jalon`, sauf ce qui demanderait un navigateur, et cette démo n'en demande pas. Elle n'écrit que sous `data/j7-demo/`, retiré au nettoyage. `data/j7-demo` a été laissé en place à la fin de ce passage pour que le propriétaire puisse rejouer une étape sans tout reconstruire ; le préambule le supprime de toute façon.

### Préambule — état propre, binaire, environnement

```sh
rm -rf data/j7-demo
mkdir -p data/j7-demo/data data/j7-demo/reports data/j7-demo/exchange/inbox data/j7-demo/exchange/outbox
cargo build --release
export ORT_DYLIB_PATH="$(cd tools && uv run --no-sync python -c 'import onnxruntime, pathlib; print(next((pathlib.Path(onnxruntime.__file__).parent / "capi").glob("libonnxruntime*")))')"
```

Attendu : `cargo build --release` se termine par `Finished` ; la dernière commande n'affiche rien et `ORT_DYLIB_PATH` nomme `libonnxruntime.1.31.0.dylib`. **Observé** : conforme, build en 17,67 s sur un `target/` chaud ; le chemin vaut `/Users/mattboll/MattCorp/crible/tools/.venv/lib/python3.13/site-packages/onnxruntime/capi/libonnxruntime.1.31.0.dylib`.

### Étape 1 — créer les six index

```sh
for n in articles decisions dole jorf sarde titres; do
    ./target/release/crible index create \
        --data data/j7-demo/data \
        --config "data/legifacile-sample/configs/$n.json"
done
```

Attendu, une ligne par index. **Observé**, six fois sur le modèle :

```text
index 'articles' created at data/j7-demo/data/indexes/articles (epoch 1, generation 0)
```

### Étape 2 — attacher les six lots

```sh
for n in articles decisions dole jorf sarde titres; do
    ./target/release/crible attach \
        --data data/j7-demo/data \
        --models data/models \
        --index "$n" \
        "data/legifacile-sample/batches/legifacile-sample-$n-v1"
done
```

Attendu, deux lignes par lot avec les comptes annoncés. **Observé**, conforme, cosinus minimale au-dessus de 0,999 :

```text
batch 'legifacile-sample-articles-v1' attached to index 'articles': generation 1 (added 42417, replaced 0, unchanged 0, metadata_updated 0, deleted 0, ignored 0)
  acceptance: 200 passage(s) re-encoded, min cosine 0.999991, p1 0.999992, p50 0.999998
batch 'legifacile-sample-decisions-v1' attached to index 'decisions': generation 1 (added 20120, …)
  acceptance: 200 passage(s) re-encoded, min cosine 0.999992, p1 0.999993, p50 0.999998
batch 'legifacile-sample-dole-v1' attached to index 'dole': generation 1 (added 3593, …)
  acceptance: 200 passage(s) re-encoded, min cosine 0.999991, p1 0.999994, p50 0.999998
batch 'legifacile-sample-jorf-v1' attached to index 'jorf': generation 1 (added 39457, …)
  acceptance: 200 passage(s) re-encoded, min cosine 0.999993, p1 0.999993, p50 0.999998
batch 'legifacile-sample-sarde-v1' attached to index 'sarde': generation 1 (added 49674, …)
  acceptance: 200 passage(s) re-encoded, min cosine 0.999995, p1 0.999995, p50 0.999999
batch 'legifacile-sample-titres-v1' attached to index 'titres': generation 1 (added 50000, …)
  acceptance: 200 passage(s) re-encoded, min cosine 0.999975, p1 0.999987, p50 0.999998
```

Les passages vivants rendus par `crible inspect` sont ceux du document : 50 000 pour `articles`, `jorf`, `sarde` et `titres`, 49 998 pour `decisions`, 7 628 pour `dole`. **Observé** : conforme pour les six index.

### Étape 3 — convertir les questions de l'échantillon

```sh
cd tools
uv run --no-sync python -m bench.convert_queries \
    --queries ../data/legifacile-sample/queries.jsonl \
    --out ../data/j7-demo/queries
cd ..
```

Attendu et **observé** :

```text
300 queries in 6 game(s), written under ../data/j7-demo/queries
  articles: 63
  decisions: 200
  dole: 8
  jorf: 12
  sarde: 9
  titres: 8
  ../data/j7-demo/queries/articles.jsonl
  … (une ligne de chemin par jeu)
```

### Étape 4 — la campagne d'éval sur les trois modes

```sh
for n in articles decisions dole jorf sarde titres; do
    ./target/release/crible eval \
        --data data/j7-demo/data \
        --index "$n" \
        --queries "data/j7-demo/queries/$n.jsonl" \
        --models data/models \
        --modes dense,lexical,hybrid \
        --k 10 --repeat 3 \
        --out data/j7-demo/reports
done
```

Attendu, trois lignes par index (`json`, `markdown`, `regression`). **Observé**, six fois, `regression: none` :

```text
json: data/j7-demo/reports/eval-articles-2026-11-02.json
markdown: data/j7-demo/reports/eval-articles-2026-11-02.md
regression: none
```

Durées observées : 8,65 / 23,24 / 2,40 / 3,13 / 2,82 / 2,60 s dans l'ordre de la boucle. Les six rapports produits ont la même fidélité et la même qualité, au chiffre près, que les six rapports versionnés `docs/mesures/eval-<jeu>-2026-11-02.{json,md}` : contrôle fait par comparaison des champs `fidelity.recall_at_10` et `quality.recall_at_k` des trois modes, identiques pour les six jeux. Les latences, elles, varient d'un rejeu à l'autre et restent indicatives.

### Étape 5 — la régression injectée

```sh
./target/release/crible eval \
    --data data/j7-demo/data --index decisions \
    --queries data/j7-demo/queries/decisions.jsonl --models data/models \
    --modes dense,hybrid --k 10 --rescore-candidates 20 \
    --baseline docs/mesures/eval-decisions-2026-11-02.json \
    --out data/j7-demo/regression
echo "exit=$?"
```

Attendu et **observé**, identique au document, code de sortie 1 compris :

```text
json: data/j7-demo/regression/eval-decisions-2026-11-02.json
markdown: data/j7-demo/regression/eval-decisions-2026-11-02.md
regression: detected
exit=1
```

Section « Comparaison à la référence » du rapport produit, **observée** à l'identique :

```text
- Régression détectée : oui
- `dense` fidelity.overlap_at_10 : 0.9975 → 0.8123 (-0.1852)
- `dense` fidelity.recall_at_10 : 0.9965 → 0.8045 (-0.1920)
- `dense` quality.mrr_at_k : 0.5068 → 0.5043 (-0.0025)
- `dense` quality.recall_at_k : 0.5063 → 0.4938 (-0.0125)
- `dense` : requêtes ayant perdu un attendu — `J005`, `J111`, `J131`
- `hybrid` fidelity.overlap_at_10 : 0.9891 → 0.7519 (-0.2373)
- `hybrid` fidelity.recall_at_10 : 0.9890 → 0.7350 (-0.2540)
- `hybrid` quality.mrr_at_k : 0.5701 → 0.5609 (-0.0092)
- `hybrid` quality.recall_at_k : 0.5638 → 0.5413 (-0.0225)
- `hybrid` : requêtes ayant perdu un attendu — `J005`, `J066`, `J096`, `J111`, `J136`, `J155`
```

### Étape 6 — le serveur : debug, explain, journal, stats

Deux clés, une par rôle. Ce bloc les génère et écrit `data/j7-demo/crible.toml`, sans recopie manuelle des `sha256` :

```sh
admin=$(./target/release/crible key generate)
ADMIN=$(printf '%s\n' "$admin" | sed -n 's/^key: //p')
ADMIN_SHA=$(printf '%s\n' "$admin" | sed -n 's/^sha256: //p')
read=$(./target/release/crible key generate)
READ=$(printf '%s\n' "$read" | sed -n 's/^key: //p')
READ_SHA=$(printf '%s\n' "$read" | sed -n 's/^sha256: //p')

cat > data/j7-demo/crible.toml <<TOML
[server]
bind = "127.0.0.1:8080"
search_threads = 0
max_concurrent_searches = 0

[paths]
data = "data/j7-demo/data"
models = "data/models"
exchange = "data/j7-demo/exchange"

[[auth.keys]]
name = "j7-admin"
role = "admin"
sha256 = "$ADMIN_SHA"

[[auth.keys]]
name = "j7-read"
role = "read"
sha256 = "$READ_SHA"

[analytics]
retention_days = 90

[debug_journal]
enabled = true
sample_rate = 0.0
ttl_days = 7

[encoder]
query_precision = "fp32"

[limits]
writer_queue = 16
default_timeout_ms = 2000
wait_timeout_ms = 60000
query_cache_entries = 1024
TOML
```

Attendu : deux fois `key: …` puis `sha256: …`, et le fichier écrit sans message. **Observé** : conforme ; chaque clé et chaque empreinte font 64 caractères hexadécimaux minuscules.

Démarrer le serveur, jouer tous les appels, puis l'arrêter, **dans une seule commande au premier plan** ; le bloc ne laisse aucun processus derrière lui :

```sh
./target/release/crible serve --config data/j7-demo/crible.toml > data/j7-demo/serve.log 2>&1 &
pid=$!
for i in $(seq 1 120); do
    [ "$(curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:8080/ready)" = "200" ] && break
    sleep 1
done

curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:8080/ready

curl -s -H "Authorization: Bearer $READ" -H 'content-type: application/json' \
     -d '{"query":"Un bailleur peut-il obtenir la résiliation d'"'"'un bail commercial en se prévalant d'"'"'une clause résolutoire ?","mode":"hybrid","k":3,"debug":true}' \
     http://127.0.0.1:8080/v1/indexes/decisions/search
echo

curl -s -H "Authorization: Bearer $READ" -H 'content-type: application/json' \
     -d '{"query":"clause resolutoire bail commercial","mode":"hybrid","k":2,"filter":{"annee":{"eq":2019}},"explain_ids":["JURITEXT000044525272"]}' \
     http://127.0.0.1:8080/v1/indexes/decisions/search
echo

# Le journal de debug est vidé toutes les 10 s (ADR-0122, décision 7) :
# attendre le vidage avant de le lire, sinon la liste est encore vide.
sleep 11
curl -s -H "Authorization: Bearer $ADMIN" \
     'http://127.0.0.1:8080/v1/debug/queries?limit=5'
echo
curl -s -o /dev/null -w '%{http_code}\n' -H "Authorization: Bearer $READ" \
     'http://127.0.0.1:8080/v1/debug/queries?limit=5'

curl -s -H "Authorization: Bearer $ADMIN" http://127.0.0.1:8080/v1/stats
echo
curl -s -o /dev/null -w '%{http_code}\n' -H "Authorization: Bearer $READ" \
     http://127.0.0.1:8080/v1/stats

kill -INT $pid
wait $pid
echo "serveur exit=$?"
```

Attendu et **observé** :

- le journal du serveur dans `data/j7-demo/serve.log` porte `"the server is bound"` avec `127.0.0.1:8080` ; `/ready` rend `200` après environ 4 s et des `503` avant ;
- la recherche debug rend `200`. Premier résultat `JURITEXT000044525272#6`, rangs dense 3 et lexical 2, donc rang fusionné 1 ; `debug.plan` porte `dense_strategy: binary` et `rescore_candidates: 5000` ; `debug.timings_ms` porte les neuf clés `validate`, `encode`, `filter`, `dense`, `lexical`, `fusion`, `group`, `fetch`, `total` ; `pool_size` vaut 49 998 ; `took_ms` vaut 39,789 ms sur ce passage ;
- la réponse `explain` porte le tableau frère attendu, avec `exists: true`, `alive: true`, `in_pool: false` et `excluded_by: ["$.filter.annee"]` ;
- le journal de debug rend `200` pour `admin` et `403` pour `read`. La réponse `200` porte `enabled: true`, `dropped: 0` et, après le vidage, deux entrées `index: "decisions"`, `mode: "hybrid"`, `status: "ok"`, `total_ms`, `payload.capture` et les cinq clés `capture`, `generation`, `plan`, `request`, `timings_ms`. La première entrée vient du premier passage du serveur, vidée à son arrêt ; la seconde est celle du second passage. Lu **immédiatement** après la recherche, le même appel rend une liste vide : c'est l'écart de rédaction de la section 7 ;
- `GET /v1/stats` rend `200` pour `admin` et `403` pour `read`. Juste après la recherche, le corps ne porte ni `totals` ni seau (compte encore en file) ; après 11 s, il porte `totals` avec `count` et `latency_ms` par étape, plus un seau ;
- le serveur s'arrête avec le code `0` sur `SIGINT`, et son arrêt vide le reste de la file.

Puis, sans serveur :

```sh
./target/release/crible stats --data data/j7-demo/data --format table
```

Attendu et **observé** : des tableaux ASCII `buckets` (compte, erreurs, taux sans résultat), `latency` (une ligne par étape et par seau), `top indexes`, `top filter shapes` avec `annee:eq` et `indexes`, les six index à la génération 1, leurs passages vivants et leurs octets sur disque. Sur ce passage, `buckets` compte 3 recherches (les deux du premier serveur et celle du second), `top indexes` donne `decisions` 3, et `top filter shapes` donne `annee:eq` 1. Les horodatages et les latences varient d'une exécution à l'autre ; les comptes et les index, non.

### Nettoyage

```sh
rm -rf data/j7-demo
```

Seul ce dossier est retiré : il porte la configuration, les six index, les jeux convertis, les rapports du rejeu, `serve.log`, `analytics.sqlite` et `debug.sqlite`. `data/legifacile-sample/` et `data/models/` restent en place. **Non appliqué** à la fin de ce passage, pour laisser la démo rejouable ; le préambule le supprime de toute façon.

## 6. Décisions (ADR) de la phase

Les douze ADR de P7 sont `Accepté` ; aucun n'est `Proposé`. Les fiches de P7 ne citent aucun ADR non tranché.

| ADR | Titre | Statut | Bloque la validation ? |
|---|---|---|---|
| ADR-0117 | Analytics : stockage et calcul, schéma, histogrammes, percentile et formes de filtre | Accepté (confirmé le 2026-11-01) | Non |
| ADR-0118 | Analytics : exécution, file, pertes, vidage, purge, opérations et preuve d'INV-17 | Accepté (confirmé le 2026-11-01) | Non |
| ADR-0119 | Objet `debug` de la réponse de recherche : forme, rangs, temps et capture | Accepté (confirmé le 2026-11-01 ; borne des 10 % lue sur la médiane, confirmée le 2026-11-02) | Non |
| ADR-0120 | Fin de `took_ms`, attribution de l'ouverture des lecteurs et rendu hors chronomètre | Accepté (confirmé le 2026-11-01) | Non |
| ADR-0121 | Journal de debug : `debug.sqlite`, schéma, payload et protections | Accepté (confirmé le 2026-11-01), complété par ADR-0127 | Non |
| ADR-0122 | Journal de debug : activation, capture, file, vidage, purge, route et preuves | Accepté (confirmé le 2026-11-01), complété par ADR-0127 | Non |
| ADR-0123 | `explain_ids` : forme, multi-index, bornes, coût et rangs | Accepté (confirmé le 2026-11-01 et le 2026-11-02) | Non |
| ADR-0124 | `GET /v1/stats` : forme, paramètres, seaux et CLI | Accepté (confirmé le 2026-11-01 et le 2026-11-02) | Non |
| ADR-0125 | `crible eval` : mode local, format, métriques et rapport | Accepté (confirmé le 2026-11-01 et le 2026-11-02) | Non |
| ADR-0126 | Cache des lecteurs denses par segment et remplissage avant publication | Accepté (confirmé le 2026-11-01) | Non |
| ADR-0127 | Plafond de taille de `debug.sqlite` | Accepté (décidé le 2026-11-02) ; la tâche `PLAFONNER-JOURNAL-DE-DEBUG` est en P8 | Non |
| ADR-0128 | Couper la télémétrie d'ONNX Runtime | Accepté (décidé le 2026-11-02) | Non |

ADR-0127 a été accepté avec ses recommandations (`max_rows = 500000`, `max_bytes = 2147483648`, membres `capped`, `rows`, `bytes`, `trimmed`). Les quatre membres ont été retirés de l'exemple `DebugQueryList` de `docs/05` le 2026-11-02, sur décision du propriétaire, et reviendront avec `PLAFONNER-JOURNAL-DE-DEBUG` en P8 ; le test `json_examples_conform_to_the_specification` est vert. Aucune décision de P7 n'attend donc une signature.

## 7. Écarts, idées, risques

- **Écart de rédaction dans la démo** : `docs/jalons/J7-demo.md:371-382` décrit `GET /v1/debug/queries` lu juste après une recherche `debug: true` comme portant déjà l'entrée. Le journal est vidé toutes les 10 s (ADR-0122, décision 7), et la lecture immédiate rend `200` avec `enabled: true`, `dropped: 0` et une liste vide. Après 11 s, l'entrée apparaît. Le rapport ci-dessus ajoute l'attente au bloc de l'étape 6 ; le document, lui, ne la porte pas. Proposition `CORRIGER-DEMO-J7` en section 8. Aucun code n'est en cause.
- **Report déjà consigné** : le mode « contre une instance via l'API » de `crible eval` est reporté (ADR-0125, décision 1), faute de client HTTP dans `docs/07`. Le rouvrir avec P8 ou P9 demande une dépendance et un ADR, ou un appel au client Python existant. Ce n'est pas un manque de J7 : la fiche porte la note datée du 2026-11-01.
- **Tâches de P8, pas de J7** : `PLAFONNER-JOURNAL-DE-DEBUG` et `RESUME-PUBLIE-COHERENT` ont été ouvertes le 2026-11-02 sur décision du propriétaire et restent `A-FAIRE` en P8. Elles ne bloquent pas J7.
- **Télémétrie ONNX Runtime** : le test `#[ignore]` de COUPER-TELEMETRIE-ONNX passe en CI distante sous Linux, et son échec sans la correction n'a été montré que sous macOS. On ne sait donc pas si la bibliothèque Linux écrit un stock 1DS sans la variable ; le risque est résiduel, la variable est forcée dans `.cargo/config.toml` et au début de `main`.
- **Tests intermittents de P7, corrigés ou non reproduits** : les deux tests d'arrêt de `serve_cli` sont corrigés par CORRIGER-ARRET-SOUS-CHARGE (cause : port réservé puis relâché). Le test `the_journal_names_errors_without_any_client_message` a été isolé dans son binaire par NETTOYER-P7, passage B. Aucun échec intermittent n'est apparu pendant ce passage de J7 : 1 147 tests debug et 1 173 tests release sont verts du premier coup.
- **Risques de mesure** : le rejeu de MES-16 tourne sur une machine chargée (3,7 à 6,6 sur 8 cœurs) avec des phases d'écriture courtes ; ses latences absolues ne sont pas transposables. La mesure des lecteurs denses n'atteint pas le cache de pages (13,2 Go de vecteurs pour 24 Gio de RAM) et sa série « avant » compte 100 requêtes au lieu de 300 ; le rejeu est renvoyé au conteneur en P8. Ces réserves sont écrites dans les rapports.
- **Données de mesure conservées** : `data/mes16-journal/` (2,9 Mo), `data/mes-lecteurs-denses/` (25 Mo) et `data/j7-demo/` (669 Mo, données de démo seulement). `data/eval-n5000/` a été supprimé le 2026-11-02 sur décision du propriétaire. Le retrait des trois dossiers restants est à décider après la validation.
- **Points à décider par le propriétaire** : (1) retirer `data/mes16-journal/`, `data/mes-lecteurs-denses/` et `data/j7-demo/` ; (2) rouvrir le mode API de `crible eval` avec P8 ou P9 ; (3) le rejeu de MES-16 et de la latence des lecteurs denses en conteneur, déjà porté par `MESURER-EN-CONTENEUR-MAC` ; (4) les deux mesures de route de `GET /v1/indexes/{index}` et de `GET …/documents/{doc_id}`, ajoutées à la fiche de `MESURER-EN-CONTENEUR-MAC` le 2026-11-02, restent non mesurées.

## 8. Si le jalon n'est pas validé

Aucune correction n'est exigée pour valider J7. Les propositions suivantes sont **hors périmètre** et n'ont pas été appliquées :

- `CORRIGER-DEMO-J7` — dans `docs/jalons/J7-demo.md`, à l'étape 6, écrire que `GET /v1/debug/queries` est vidé toutes les 10 s (ADR-0122, décision 7) et ajouter au bloc de commandes l'attente du vidage avant la lecture du journal, comme le fait la section 5 de ce rapport. Critère : la commande de l'étape 6 rend `200` avec une entrée après l'attente, et la note de l'étape dit pourquoi.
- `CORRIGER-DOC-DECISIONS-HYBRIDE` (facultative, sans correction exigée) — `J7-demo.md` dit deux fois l'exception de qualité de `decisions` hybride (`:153-155` et `:162-164`) ; la fusion en une seule phrase supprimerait la redite relevée par `@architecte`. Critère : une seule mention, chiffres inchangés.

Les corrections qu'un relecteur demanderait après un refus deviendraient, elles, des tâches `CORRIGER-…` dans `docs/phases/P7-observabilite.md`, avec leurs critères d'acceptation.
