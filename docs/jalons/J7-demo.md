# Démo J7 — observabilité et éval

Commandes exactes du jalon J7, à jouer depuis la racine du dépôt. La démo crée
les six index de l'échantillon Legifacile, convertit les 300 questions en jeux
d'éval, fait tourner `crible eval` sur les six index dans les trois modes,
injecte une régression et la fait détecter par `--baseline`, puis allume un
serveur pour montrer `debug`, `explain_ids`, le journal de debug, `GET
/v1/stats` et `crible stats`.

Références : `docs/phases/P7-observabilite.md:5-9` (jalon J7), ADR-0125 (commande
`crible eval`), ADR-0119 (`debug`), ADR-0121 et ADR-0122 (journal de debug),
ADR-0123 (`explain_ids`), ADR-0117 et ADR-0124 (analytics et `GET /v1/stats`),
ADR-0095 (rôles des clés).

L'état propre ne touche qu'un dossier nommé, gitignoré : `data/j7-demo`. Aucun
`rm -rf data`, et les index existants de `data/indexes/` ne sont pas touchés.

## Prérequis

- `data/legifacile-sample/` (gitignoré) : les six lots réels et `configs/` ;
- `data/models/mdenseon/` : `model.json`, `tokenizer.json`, `model.onnx` et
  `model.int8.onnx` du ré-export d'ADR-0050 ;
- l'environnement de `tools/` synchronisé comme en J6 (`uv sync --inexact`, qui
  garde le groupe `export`), pour ONNX Runtime et le convertisseur ;
- ONNX Runtime, chargé dynamiquement par `crible-encode` : `ORT_DYLIB_PATH`
  nomme la bibliothèque, sans être figé dans ce document.

## Préambule — état propre, binaire, environnement

```sh
rm -rf data/j7-demo
mkdir -p data/j7-demo/data data/j7-demo/reports data/j7-demo/exchange/inbox data/j7-demo/exchange/outbox
cargo build --release
export ORT_DYLIB_PATH="$(cd tools && uv run --no-sync python -c 'import onnxruntime, pathlib; print(next((pathlib.Path(onnxruntime.__file__).parent / "capi").glob("libonnxruntime*")))')"
```

Attendu : `cargo build --release` se termine par `Finished` ; la dernière
commande n'affiche rien et `ORT_DYLIB_PATH` nomme `libonnxruntime…` (`.dylib`
sur macOS, `.so` sur Linux).

## Étape 1 — créer les six index

```sh
for n in articles decisions dole jorf sarde titres; do
    ./target/release/crible index create \
        --data data/j7-demo/data \
        --config "data/legifacile-sample/configs/$n.json"
done
```

Attendu, une ligne par index :

```text
index 'articles' created at data/j7-demo/data/indexes/articles (epoch 1, generation 0)
```

Les six configurations activent la branche lexicale et posent un
`rescore_candidates` de 1000 (le réglage de l'échantillon), ce qui rend les
trois modes disponibles.

## Étape 2 — attacher les six lots

Le lot est passé directement, sans passer par l'inbox : `crible attach` lit le
dossier nommé.

```sh
for n in articles decisions dole jorf sarde titres; do
    ./target/release/crible attach \
        --data data/j7-demo/data \
        --models data/models \
        --index "$n" \
        "data/legifacile-sample/batches/legifacile-sample-$n-v1"
done
```

Attendu, deux lignes par lot ; les comptes et la recette sont ceux-ci (la
cosinus minimale reste au-dessus du seuil de 0,999, sa valeur exacte dépend de
l'échantillon de 200 passages ré-encodés) :

```text
batch 'legifacile-sample-articles-v1' attached to index 'articles': generation 1 (added 42417, replaced 0, unchanged 0, metadata_updated 0, deleted 0, ignored 0)
  acceptance: 200 passage(s) re-encoded, min cosine 0.9999…, p1 0.9999…, p50 0.9999…
batch 'legifacile-sample-decisions-v1' attached to index 'decisions': generation 1 (added 20120, …)
batch 'legifacile-sample-dole-v1' attached to index 'dole': generation 1 (added 3593, …)
batch 'legifacile-sample-jorf-v1' attached to index 'jorf': generation 1 (added 39457, …)
batch 'legifacile-sample-sarde-v1' attached to index 'sarde': generation 1 (added 49674, …)
batch 'legifacile-sample-titres-v1' attached to index 'titres': generation 1 (added 50000, …)
```

Après l'attache, les passages vivants valent 50 000 pour `articles`, `jorf`,
`sarde` et `titres`, 49 998 pour `decisions` et 7 628 pour `dole` : c'est ce
que rend `crible inspect`, et c'est le nombre de passages, pas de documents.

## Étape 3 — convertir les questions de l'échantillon

`crible eval` lit un JSONL fermé, `{ "id", "query" | "vector", "filter"?,
"expected"? }` (ADR-0125, décision 3). Le fichier de l'échantillon porte une
autre forme ; le convertisseur de `tools/bench/` la traduit, sans jamais
afficher un texte de question.

```sh
cd tools
uv run --no-sync python -m bench.convert_queries \
    --queries ../data/legifacile-sample/queries.jsonl \
    --out ../data/j7-demo/queries
cd ..
```

Attendu :

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

Un fichier par structure, parce qu'un index sert une structure (ADR-0003) ; le
nom du fichier devient le `game` du rapport.

## Étape 4 — la campagne d'éval sur les trois modes

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

Attendu, trois lignes par index (`json`, `markdown`, `regression`) :

```text
json: data/j7-demo/reports/eval-articles-2026-11-01.json
markdown: data/j7-demo/reports/eval-articles-2026-11-01.md
regression: none
```

Les rapports versionnés de `docs/mesures/` ont été produits par la **même**
commande, avec `--out docs/mesures` : leurs chiffres de fidélité et de qualité
sont identiques à ceux du rejeu, seules les latences diffèrent. La commande
refuse d'écraser un rapport existant, ce qui interdit de rejouer la campagne
dans `docs/mesures/` sans supprimer les fichiers avant ; le rejeu écrit donc
sous `data/`.

Les chiffres de `docs/mesures/` (fidélité : recall@10 au passage contre la
recherche exacte ; qualité : recall@k au document contre les attendus) :

| Jeu | Requêtes | Avec attendus | Fidélité dense | Fidélité hybride | Qualité dense | Qualité lexicale | Qualité hybride |
|---|---|---|---|---|---|---|---|
| `articles` | 63 | 13 | 0,9825 | 0,9508 | 0,0000 | 0,0000 | 0,0000 |
| `decisions` | 200 | 200 | 0,9705 | 0,9345 | 0,5063 | 0,5800 | 0,5563 |
| `dole` | 8 | 8 | 1,0000 | 1,0000 | 1,0000 | 1,0000 | 1,0000 |
| `jorf` | 12 | 12 | 0,9750 | 0,9417 | 0,5833 | 0,5833 | 0,5833 |
| `sarde` | 9 | 9 | 1,0000 | 0,9778 | 0,6667 | 0,6667 | 0,6667 |
| `titres` | 8 | 8 | 1,0000 | 0,9875 | 1,0000 | 1,0000 | 1,0000 |

Le mode `lexical` n'a pas de fidélité : il n'y a pas de recherche exacte à
laquelle le comparer, donc les membres `fidelity` et `fidelity_excluded` sont
omis. La qualité de
`articles` est nulle sur les 13 requêtes qui portent des attendus : aucun des
13 documents attendus n'appartient aux six index de l'échantillon, alors que
les questions, elles, viennent du jeu complet. C'est une propriété des données,
pas de la recherche, et le rapport la montre telle quelle.

Chaque rapport reprend sa ligne de commande complète et dit si N vient de la
configuration de l'index (`rescore_candidates_source: configuration`) ou de
`--rescore-candidates` : les six campagnes de l'échantillon lisent les
configurations de l'étape 1, donc N = 1000.

Les latences de ces rapports (p50 du total, mode par mode) vont de 0,517 ms
(`dole` lexical) à 1,716 ms (`decisions` dense) ; le p95 le plus haut est
3,637 ms et le p99 le plus haut 4,371 ms, tous deux sur `articles`, en hybride
pour le p95 et en dense pour le p99. Elles sont **indicatives** : elles viennent
de `debug.timings_ms` et ne valent pas une mesure MES-xx (règle 7 d'`AGENTS.md`).
Elles varient d'un rejeu à l'autre ; la fidélité et la qualité, non.

## Étape 5 — la régression injectée

Un `rescore_candidates` de 20 au lieu de 1000 réduit les candidats re-scorés,
donc la fidélité. La campagne se compare au rapport de `decisions` :

```sh
./target/release/crible eval \
    --data data/j7-demo/data --index decisions \
    --queries data/j7-demo/queries/decisions.jsonl --models data/models \
    --modes dense,hybrid --k 10 --rescore-candidates 20 \
    --baseline docs/mesures/eval-decisions-2026-11-01.json \
    --out data/j7-demo/regression
echo "exit=$?"
```

Attendu :

```text
json: data/j7-demo/regression/eval-decisions-2026-11-01.json
markdown: data/j7-demo/regression/eval-decisions-2026-11-01.md
regression: detected
exit=1
```

Et dans le rapport produit, section « Comparaison à la référence » :

```text
- Régression détectée : oui
- `dense` fidelity.overlap_at_10 : 0.9709 → 0.8123 (-0.1585)
- `dense` fidelity.recall_at_10 : 0.9705 → 0.8045 (-0.1660)
- `dense` : requêtes ayant perdu un attendu — `J005`, `J111`, `J131`
- `hybrid` fidelity.recall_at_10 : 0.9345 → 0.7350 (-0.1995)
- `hybrid` : requêtes ayant perdu un attendu — `J005`, `J111`, `J136`, `J155`
```

Le code de sortie 1 est le contrat : une baisse de plus de 0,005 en valeur
absolue d'une moyenne de fidélité ou de qualité rend `regression: detected` et
`1`. Les latences sont rapportées, jamais bloquantes. Les identifiants de
requêtes perdus sont des `id` du jeu, jamais des documents.

## Étape 6 — le serveur : debug, explain, journal, stats

Deux clés, une par rôle utile ici : `admin` pour le journal et les stats,
`read` pour les recherches et pour prouver le `403`.

```sh
./target/release/crible key generate
./target/release/crible key generate
```

Attendu, deux fois : `key: …` puis `sha256: …`. Écrire
`data/j7-demo/crible.toml` en remplaçant les deux `sha256` :

```sh
cat > data/j7-demo/crible.toml <<'TOML'
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
sha256 = "<sha256 de la clé admin>"

[[auth.keys]]
name = "j7-read"
role = "read"
sha256 = "<sha256 de la clé read>"

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

`paths.exchange` est obligatoire même sans écriture : la configuration est
refusée au démarrage sans lui. `sample_rate = 0.0` garde le journal silencieux
sauf pour une requête `debug: true` (ADR-0122, décision 1).

Démarrer le serveur dans un terminal dédié, avec `ORT_DYLIB_PATH` posé :

```sh
./target/release/crible serve --config data/j7-demo/crible.toml
```

Attendu : le journal JSON sur la sortie d'erreur, dont `"the server is bound"`
avec `127.0.0.1:8080`. Dans un second terminal :

```sh
curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:8080/ready
```

Attendu : `200`. Puis, avec la clé `read` dans `$READ` et la clé `admin` dans
`$ADMIN` :

```sh
curl -s -H "Authorization: Bearer $READ" -H 'content-type: application/json' \
     -d '{"query":"Un bailleur peut-il obtenir la résiliation d'"'"'un bail commercial en se prévalant d'"'"'une clause résolutoire ?","mode":"hybrid","k":3,"debug":true}' \
     http://127.0.0.1:8080/v1/indexes/decisions/search
```

Attendu : `200` avec `debug.per_result` (un rang dense, un score dense, un rang
lexical, un score lexical et un rang fusionné par résultat), `debug.plan`
(`dense_strategy: binary`, `rescore_candidates: 1000`) et `debug.timings_ms`
aux clés `validate`, `encode`, `filter`, `dense`, `lexical`, `fusion`, `group`,
`fetch`, `total`. Le premier résultat est `JURITEXT000044525272#6`, de rangs
dense 3 et lexical 2, donc de rang fusionné 1.

`explain_ids` sur un document hors du pool, avec un filtre qui l'exclut :

```sh
curl -s -H "Authorization: Bearer $READ" -H 'content-type: application/json' \
     -d '{"query":"clause resolutoire bail commercial","mode":"hybrid","k":2,"filter":{"annee":{"eq":2019}},"explain_ids":["JURITEXT000044525272"]}' \
     http://127.0.0.1:8080/v1/indexes/decisions/search
```

Attendu, dans `explain` (tableau frère de `results`) :

```json
[ { "id": "JURITEXT000044525272", "doc_id": "JURITEXT000044525272",
    "exists": true, "alive": true, "in_pool": false,
    "excluded_by": [ "$.filter.annee" ] } ]
```

Le diagnostic nomme la feuille du filtre par son chemin JSON complet, ce qui dit
pourquoi le document n'est pas remonté.

Le journal de debug garde la requête `debug: true`, et seulement elle :

```sh
curl -s -o /dev/null -w '%{http_code}\n' -H "Authorization: Bearer $ADMIN" \
     'http://127.0.0.1:8080/v1/debug/queries?limit=5'
curl -s -o /dev/null -w '%{http_code}\n' -H "Authorization: Bearer $READ" \
     'http://127.0.0.1:8080/v1/debug/queries?limit=5'
```

Attendu : `200`, puis `403`. La réponse `200` porte `enabled: true`, `dropped: 0`
et une entrée avec `index: "decisions"`, `mode: "hybrid"`, `status: "ok"`,
`total_ms` et `payload.capture` (les vingt premiers rangs par branche).

`GET /v1/stats` est réservé à `admin` (ADR-0095) :

```sh
curl -s -H "Authorization: Bearer $ADMIN" http://127.0.0.1:8080/v1/stats
curl -s -o /dev/null -w '%{http_code}\n' -H "Authorization: Bearer $READ" \
     http://127.0.0.1:8080/v1/stats
```

Attendu : un corps avec `buckets`, `totals` et les sections par étape, puis
`403`. Le vidage des analytics est asynchrone, toutes les 10 s (ADR-0118) :
juste après une recherche, le compte peut être en retard ; une seconde lecture
dix secondes plus tard montre le trafic complet.

Arrêter le serveur par `SIGTERM` (ou `Ctrl-C`). Le vidage de l'arrêt écrit le
reste des événements. Puis, sans serveur :

```sh
./target/release/crible stats --data data/j7-demo/data --format table
```

Attendu : des tableaux ASCII — `buckets` (compte, erreurs, taux sans résultat,
p50/p95/p99), `latency` (une ligne par étape et par seau), `top indexes`,
`top filter shapes` (`annee:eq` pour la recherche filtrée) et `indexes` (les six
index, leur génération, leurs passages vivants et leurs octets sur disque). Les
horodatages et les latences varient d'une exécution à l'autre ; les comptes et
les index, non.

## Ce que la démo prouve, et ce qu'elle renvoie

La démo montre les quatre points du jalon qui la concernent
(`docs/phases/P7-observabilite.md:5-9`) :

- `crible eval` produit, sur l'échantillon Legifacile, un rapport complet —
  fidélité, qualité, latence — pour les trois modes, et ce rapport est lisible
  sans contexte ;
- une régression injectée par `--rescore-candidates 20` est détectée par
  `--baseline`, avec le code de sortie 1 ;
- `explain_ids` sur un document absent des résultats dit pourquoi ;
- `crible stats` montre le trafic de la démo, et `GET /v1/stats` le même trafic
  par l'API.

Elle ne prouve pas les deux invariants du jalon, qui sont des tests :
INV-13 (aucun contenu dans les analytics) est
`crates/crible-server/tests/inv_13_analytics_without_content.rs` ; INV-17 (le
vidage ne bloque jamais une requête) est
`crates/crible-server/tests/inv_17_analytics_never_blocks.rs`, plus
`inv_17_debug_journal_never_blocks.rs` pour le journal, sous la feature
`failpoints`.

## Nettoyage

```sh
rm -rf data/j7-demo
```

Seul ce dossier est retiré : il porte la configuration, les six index, les jeux
convertis, les rapports du rejeu et les fichiers d'analytics. `data/legifacile-sample/`
et `data/models/` restent en place.
