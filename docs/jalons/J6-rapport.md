# Rapport de jalon J6 — Serveur et API

> Rédigé par l'agent `jalon` (commande `/jalon J6`). **Seul le propriétaire peut valider le jalon**, dans `docs/suivi.md`.

- **Date** : 2026-10-31
- **Branche de phase / commit** : `phase/P6-serveur-et-api` / `b3fdf9b`
- **Machine** : Mac de développement, Apple M3, 24 Gio, macOS 15.7.7, exécution native (pas de conteneur)
- **Verdict de l'agent** : `PRET-POUR-VALIDATION`, avec une réserve connue sur un test intermittent (section 7)

## 1. Tâches de la phase

Les 12 tâches de `docs/phases/P6-serveur-et-api.md` sont `FAIT` dans `docs/suivi.md`, et aucune autre tâche de P6 n'est ouverte.

| Tâche | Statut dans `suivi.md` | Commit | Remarque |
|---|---|---|---|
| DEMARRER-SERVEUR-AXUM | FAIT | fusion `7d6070c` | ADR-0095 à ADR-0099 |
| AUTHENTIFIER-CLES | FAIT | fusion `1824177` | ADR-0100 |
| FORMAT-ERREURS-ET-LIMITES | FAIT | fusion `eaa8b96` | ADR-0101, ADR-0102 |
| FILE-D-ECRITURE-ET-JOBS | FAIT | fusion `1d95e15` | ADR-0103, ADR-0104 |
| ROUTES-INDEX | FAIT | fusion `fb663e5` | INV-16 ; message de fusion resté `merge: NOM` (section 7) |
| ROUTES-DOCUMENTS-ET-LOTS | FAIT | fusion `11ed107` | INV-01 ; ADR-0107, ADR-0108 |
| ROUTE-RECHERCHE | FAIT | fusion `301255a` | ADR-0109 |
| RECHERCHE-MULTI-INDEX | FAIT | fusion `23e0978` | ADR-0110 |
| PUBLIER-OPENAPI | FAIT | fusion `63a1779` | ADR-0111 |
| CLIENT-PYTHON-HTTP | FAIT | fusion `370a0c4` | ADR-0112 |
| TESTER-CHARGE-CONCURRENTE | FAIT | fusion `467a859` | INV-02, MES-16, ADR-0113 |
| NETTOYER-P6 | FAIT | fusions `041fb53` et `3413c38` | Passage F après un échec de la CI distante |

## 2. Qualité

Toutes les commandes ont été lancées par `@jalon` le 2026-10-31 sur `b3fdf9b`, sauf la série `--release --include-ignored`, lancée par l'orchestrateur au même commit et reportée telle quelle.

| Contrôle | Commande | Résultat |
|---|---|---|
| Formatage | `cargo fmt --all -- --check` | exit 0 |
| Lints | `cargo clippy --all-targets --locked -- -D warnings` | exit 0, 31,54 s |
| Liens de documentation (ADR-0114) | `RUSTDOCFLAGS='-D warnings' cargo doc --no-deps --workspace --locked` | exit 0, 8,49 s |
| Tests Rust | `cargo test --locked --no-fail-fast` | **1 002 passés, 0 échoué, 25 ignorés**, sur 118 lignes `test result` |
| Lints `failpoints` (`crible-store`) | `cargo clippy -p crible-store --all-targets --locked --features failpoints -- -D warnings` | exit 0 |
| Tests `failpoints` (`crible-store`) | `cargo test -p crible-store --locked --features failpoints` | **348 passés, 0 échoué, 1 ignoré** |
| Lints `failpoints` (`crible-server`) | `cargo clippy -p crible-server --all-targets --locked --features failpoints -- -D warnings` | exit 0 |
| Tests `failpoints` (`crible-server`) | `cargo test -p crible-server --locked --features failpoints` | **229 passés, 0 échoué, 0 ignoré** |
| Tests Python du client | `cd clients/python && uv run pytest` | **107 passés, 12 désélectionnés** (marqueur `server` exclu par défaut) |
| Tests Python des outils | `cd tools && uv run --no-sync pytest` | **494 passés, 1 sauté**, 378,16 s |
| Série complète avec ONNX Runtime (**orchestrateur**) | `ORT_DYLIB_PATH=$PWD/tools/.venv/lib/python3.13/site-packages/onnxruntime/capi/libonnxruntime.1.31.0.dylib cargo test --locked --workspace --release -- --include-ignored` | **1 027 passés, 0 échoué, 0 ignoré**, 118 lignes `test result` (113 binaires et 5 séries de doc-tests), 4 626 s dont 2 min 40 s de compilation |

Les 1 002 tests verts et les 25 ignorés du mode debug rejoignent exactement les 1 027 de la série `--release --include-ignored` : aucune divergence de décompte.

Durées longues de la série de l'orchestrateur, toutes `ok` :

| Test | Durée |
|---|---|
| `crible/tests/api_and_cli_on_sample.rs` | 1 901,11 s |
| `inv_08_export_import_roundtrip.rs` | 1 213,83 s |
| `inv_09_compaction_equivalence.rs` | 843,28 s |
| `recette_import.rs` | 409,81 s |

**Réserve constatée (`@jalon`).** Le premier `cargo test --locked` a échoué sur `serve_answers_the_probes_and_stops_on_sigterm` : `panicked at crates/crible/tests/serve_cli.rs:260:34: the reply is readable: Os { code: 54, kind: ConnectionReset, message: "Connection reset by peer" }`. Le test est vert 5 fois sur 5 lancé seul, et la suite complète `--no-fail-fast` est verte (1 002 / 0 / 25). C'est la même famille d'échec que `serve_stops_on_sigint`, déjà notée dans « Idées hors périmètre » le 2026-10-31 (`suivi.md`, ligne du 2026-10-31 dans « Idées hors périmètre »). Proposition de tâche en section 8.

## 3. Invariants de la phase

Les fiches de P6 citent INV-16 (`ROUTES-INDEX`), INV-01 (`ROUTES-DOCUMENTS-ET-LOTS`) et INV-02 (`TESTER-CHARGE-CONCURRENTE`). `ROUTES-INDEX` livre aussi la protection d'INV-04 par ADR-0106.

| Invariant | Test(s) | Résultat |
|---|---|---|
| **INV-16** — un réglage figé ne se modifie pas par l'API | `crible-server/tests/inv_16_immutable_settings.rs` : 7 `inv_16_*` (`409` nommant le champ pour `name`, `model`, `reference_precision`, type de champ, niveau de champ ; `PATCH` mixte sans effet ; réglage mutable accepté) | **7 passés, 0 échoué** |
| INV-16 (noyau) | `crible-core/tests/inv_16_immutable_settings.rs` : 6 `inv_16_*` et 12 tests unitaires de `apply_patch` | **18 passés, 0 échoué** |
| **INV-01** — aucun iid dans les réponses | `crible-server/tests/inv_01_no_iid_in_responses.rs` : `inv_01_write_routes_render_no_iid_key_or_value`, `inv_01_write_routes_journal_carries_no_iid` | **2 passés, 0 échoué** |
| INV-01 (noyau) | `crible-core/tests/inv_01_iid_not_serializable.rs` : 3 `inv_01_*` | **3 passés, 0 échoué** |
| **INV-02** — une lecture n'est jamais bloquée par une écriture | `crible-server/tests/inv_02_reads_during_writes.rs` sous `--features failpoints` : recherche `dense` et `hybrid` pendant une attache retenue avant le COMMIT et après le COMMIT SQLite, recherche pendant une compaction retenue | **4 passés, 0 échoué** |
| INV-02 (store) | `crible-store/tests/inv_02_compaction_reads.rs` sous `--features failpoints` : 2 `inv_02_*` | **2 passés, 0 échoué** |
| INV-04 (protection ADR-0106) | `crible-store/tests/inv_04_lexical_chain.rs` : 5 `inv_04_*` (chaîne enregistrée au commit, reconstruction à l'ouverture en écriture, refus en lecture seule, charge utile v1, compaction) | **5 passés, 0 échoué** |

Aucun de ces tests ne demande l'échantillon Legifacile ; ils sont tous dans la suite ordinaire ou sous `failpoints`. Le critère de charge d'INV-02 (chaque recherche sous 3 × le p99 du repos) n'est pas un test : il est prouvé par MES-16 (section 4).

## 4. Mesures de la phase

| Mesure | Rapport | Chiffre clé | Cible | Atteinte ? |
|---|---|---|---|---|
| MES-16 — charge concurrente pendant les écritures | `docs/mesures/MES-16-charge-concurrente.md` (campagne du 2026-10-30, Mac mini M4 8 cœurs, `--release`, 3 campagnes) | INV-02 tenu sur les trois campagnes : aucune recherche des phases d'écriture au-delà de 3 × p99 du repos ; limites 466,41 / 540,55 / 485,31 ms ; p99 du repos 155,47 / 180,18 / 161,77 ms ; pire cas 0,61 de la limite (attache de la campagne 3) ; toutes les recherches en `200`, aucun `5xx` | Chaque recherche sous 3 × p99 du repos (ADR-0113, décision 1) ; p99 de la phase d'écriture sous 3 × p99 du repos | Oui |

Réserves du rapport, reprises telles quelles : machine sous charge tierce de 3 à 6 sur 8 cœurs (latences absolues non transposables), attaches d'environ 1 s (39 à 42 recherches par campagne pendant les attaches). Le rejeu en conteneur ou à l'échelle de P9 est noté dans « Idées hors périmètre ».

## 5. Démo à rejouer par le propriétaire

Le scénario est décrit dans `docs/jalons/J6-demo.md`. Les commandes exactes suivent ; elles ont été jouées le 2026-10-31 par `@jalon`, sauf l'étape 4 (navigateur), laissée au propriétaire. La démo n'écrit que sous `data/j6-demo/`, retiré au nettoyage.

### Préambule — état propre, binaire, environnement

```sh
rm -rf data/j6-demo
mkdir -p data/j6-demo/data data/j6-demo/exchange/inbox data/j6-demo/exchange/outbox
cargo build --release
cd tools && uv sync --inexact && cd ..
export ORT_DYLIB_PATH="$(cd tools && uv run --no-sync python -c 'import onnxruntime, pathlib; print(next((pathlib.Path(onnxruntime.__file__).parent / "capi").glob("libonnxruntime*")))')"
```

Attendu : `Finished` pour `cargo build --release` ; la dernière commande n'affiche rien et `ORT_DYLIB_PATH` nomme `libonnxruntime.1.31.0.dylib` sur cette machine. **Observé le 2026-10-31** : conforme. `uv sync --inexact` n'a réinstallé aucun paquet et n'a pas retiré le groupe `export`.

### Étape 1 — configuration de l'instance et clés

```sh
./target/release/crible key generate
./target/release/crible key generate
./target/release/crible key generate
```

Attendu, trois fois : `key: …` puis `sha256: …`, 64 caractères hexadécimaux minuscules. **Observé** : conforme ; première paire prise pour `admin`, deuxième pour `read`, troisième pour `write`, comme le document.

Écrire ensuite `data/j6-demo/crible.toml` avec les trois `sha256` affichés, exactement comme au bloc de `docs/jalons/J6-demo.md:65-107` (chemins relatifs à la racine, `bind = "127.0.0.1:8080"`, `[limits] writer_queue = 16`, `default_timeout_ms = 2000`, `wait_timeout_ms = 60000`). **Observé** : fichier créé sans message.

> Point de vigilance : le serveur refuse de démarrer si la variable `CRIBLE_ADMIN_KEY` (ou toute autre `CRIBLE_*` inconnue de `crible.toml`) est présente dans son environnement — `invalid_request: CRIBLE_ADMIN_KEY names no key of crible.toml: 'admin_key' is unknown` (ADR-0097). Les clés de l'étape 5 s'exportent donc **dans le terminal du client**, pas dans celui du serveur ; le document les sépare bien, il faut le respecter.

### Étape 2 — les lots dans l'inbox

```sh
for batch in data/legifacile-sample/batches/*; do
    cp -Rl "$batch" data/j6-demo/exchange/inbox/
done
ls data/j6-demo/exchange/inbox
```

Attendu : les six noms `legifacile-sample-articles-v1`, `-decisions-v1`, `-dole-v1`, `-jorf-v1`, `-sarde-v1`, `-titres-v1`. **Observé** : conforme, 514 Mo en liens durs.

### Étape 3 — démarrer le serveur

Dans un terminal dédié, depuis la racine du dépôt, avec `ORT_DYLIB_PATH` posé et **sans** variable `CRIBLE_*` :

```sh
./target/release/crible serve --config data/j6-demo/crible.toml
```

Puis, dans un second terminal :

```sh
curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:8080/ready
```

Attendu : `200`, après des `503` tant que les modèles ne sont pas chargés (ADR-0099). **Observé** : `/ready` passe de `503` à `200` en 2 s ; journal JSON sur la sortie d'erreur, ligne `"the server is bound"` avec `"address":"127.0.0.1:8080"`, puis `"models and indexes loaded","indexes":0,"encoders":0` et l'avertissement `"the instance serves no index"` ; `/health` et `/ready` rendent `200`.

### Étape 4 — le scénario via Swagger UI

**À jouer par le propriétaire** : elle demande un navigateur, que `@jalon` n'a pas. Vérification partielle faite avec `curl` pendant que le serveur tournait :

```sh
curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:8080/openapi.json
curl -s -o /dev/null -w '%{http_code} %{redirect_url}\n' http://127.0.0.1:8080/docs
curl -s -o /dev/null -L -w '%{http_code}\n' http://127.0.0.1:8080/docs
```

**Observé** : `/openapi.json` rend `200` (OpenAPI 3.1.0, `Crible API`, 14 chemins) ; `/docs` rend `303` vers `http://127.0.0.1:8080/docs/`, puis `200` en suivant la redirection. Le `303` est la forme décidée par le complément du 2026-10-30 d'ADR-0111 ; un navigateur suit la redirection. Les sept opérations de l'étape 4 (création des six index, attaches, recherche simple, recherche multi-index, `PATCH`, `compact`, `export`) ont été jouées par HTTP à l'étape 5, avec les mêmes corps et les mêmes réponses ; les clics **Try it out** et **Authorize** restent à faire par le propriétaire.

### Étape 5 — le scénario via le client Python

Terminal du client, serveur de l'étape 3 toujours en marche :

```sh
cd tools
export CRIBLE_ADMIN_KEY='<clé admin>'
export CRIBLE_READ_KEY='<clé read>'
export CRIBLE_WRITE_KEY='<clé write>'
uv run --no-sync python -u - <<'PY'
import asyncio, json
from pathlib import Path

from crible import AsyncClient

URL = "http://127.0.0.1:8080"
SAMPLE = Path("../data/legifacile-sample")
ADMIN, READ, WRITE = (
    __import__("os").environ["CRIBLE_ADMIN_KEY"],
    __import__("os").environ["CRIBLE_READ_KEY"],
    __import__("os").environ["CRIBLE_WRITE_KEY"],
)
QUESTION = (
    "Quel dossier législatif correspond à la loi de 2024 pour contrôler "
    "l'immigration et améliorer l'intégration ?"
)


async def main() -> None:
    async with AsyncClient(URL, ADMIN) as admin, \
               AsyncClient(URL, WRITE) as writer, \
               AsyncClient(URL, READ) as reader:
        for path in sorted((SAMPLE / "configs").glob("*.json")):
            config = json.loads(path.read_text(encoding="utf-8"))
            await admin.create_index(config)
            print(f"index {config['name']} created")
        for config in sorted((SAMPLE / "configs").glob("*.json")):
            index = json.loads(config.read_text(encoding="utf-8"))["name"]
            job = await writer.attach_batch(index, f"legifacile-sample-{index}-v1")
            job = await writer.wait(job, timeout=1800, poll_interval=1.0)
            print(f"attached {index}: generation {job['generation']}")
        answer = await reader.search("dole", query=QUESTION, mode="hybrid", k=3)
        print("dole:", answer["results"][0]["key"], "pool", answer["pool_size"])
        multi = await reader.search_multi(
            [{"index": "dole"}, {"index": "jorf"}], query=QUESTION, mode="hybrid", k=5
        )
        print("multi:", [result["doc_id"] for result in multi["results"]])
        job = await writer.patch_documents(
            "dole", [{"doc_id": "JORFDOLE000047079143", "fields": {"annee": 2023}}]
        )
        job = await writer.wait(job, timeout=600, poll_interval=0.5)
        print("patch:", job["result"]["report"])
        document = await reader.get_document("dole", "JORFDOLE000047079143")
        print("fields:", document["fields"])
        for name, starter in (("compact", admin.compact), ("export", admin.export)):
            job = await starter("dole")
            job = await admin.wait(job, timeout=1800, poll_interval=0.5)
            print(f"{name}:", job["result"]["report"])
        listing = await reader.list_indexes()
        for entry in listing["indexes"]:
            print(entry["name"], entry["generation"], entry["live_passages"])


asyncio.run(main())
PY
cd ..
```

Attendu, le bloc `docs/jalons/J6-demo.md:294-313`. **Observé le 2026-10-31, identique ligne pour ligne** (durée du script : 556 s, soit 9 min 16 s) :

```text
index articles created
index decisions created
index dole created
index jorf created
index sarde created
index titres created
attached articles: generation 1
attached decisions: generation 1
attached dole: generation 1
attached jorf: generation 1
attached sarde: generation 1
attached titres: generation 1
dole: JORFDOLE000047079143#0 pool 7628
multi: ['JORFDOLE000047079143', 'JORFARTI000026179331', 'JORFDOLE000051903428', 'JORFARTI000037598356', 'JORFDOLE000047079143']
patch: {'generation': 2, 'unchanged': 0, 'updated': 1}
fields: {'annee': 2023, 'type': 'LOI_PUBLIEE'}
compact: {'from_epoch': 1, 'generation': 3, 'live_passages': 7628, 'removed_passages': 0, 'to_epoch': 2}
export: {'directory': 'outbox/dole-g3', 'documents': 3593, 'generation': 3, 'passages': 7628}
articles 1 50000
decisions 1 49998
dole 3 7628
jorf 1 50000
sarde 1 50000
titres 1 50000
```

`python -u` a été utilisé et une ligne d'avancement s'affichait toutes les 30 s, comme demandé ; le script lui-même n'imprime rien pendant les attaches.

### Étape 6 — état de l'instance

```sh
curl -s -H "Authorization: Bearer <clé read>" http://127.0.0.1:8080/v1/indexes
ls data/j6-demo/exchange/outbox
curl -s -H "Authorization: Bearer <clé read>" http://127.0.0.1:8080/v1/indexes/dole
```

Attendu : six index triés par nom, `dole` à la génération 3, `dole-g3` dans l'outbox, et le détail de `dole` sur la deuxième époque. **Observé** : conforme ; `dole` rend `"epoch": 2`, `"generation": 3`, `"live_passages": 7628`, `"documents": 3593`, `"segments": 1`, `"last_batch":{"batch_id":"legifacile-sample-dole-v1","generation":1}`, et la configuration matérialisée (`rrf`, `fp16`, `exact_threshold":1000`).

### Nettoyage

```sh
rm -rf data/j6-demo
```

**Observé** : `data/j6-demo` retiré ; `data/legifacile-sample/` et `data/models/` intacts. Le serveur a été arrêté par `SIGTERM` et a rendu `0`.

## 6. Décisions (ADR) de la phase

| ADR | Titre | Statut | Bloque la validation ? |
|---|---|---|---|
| ADR-0095 | Report de deux critères d'acceptation de P6 | Accepté (confirmé le 2026-10-29) | Non |
| ADR-0096 | Dépendances et features du serveur | Accepté (confirmé le 2026-10-29) | Non |
| ADR-0097 | Configuration de l'instance : surcharges `CRIBLE_*`, chemin de `crible.toml`, options de `crible serve` | Accepté (confirmé le 2026-10-29) | Non |
| ADR-0098 | `x-request-id` | Accepté (confirmé le 2026-10-29) | Non |
| ADR-0099 | Cycle de vie du serveur : démarrage, `/health`, `/ready`, arrêt propre | Accepté (confirmé le 2026-10-29) | Non |
| ADR-0100 | Clés d'API : dépendances, format, validation et middleware `Bearer` | Accepté (confirmé le 2026-10-29) | Non |
| ADR-0101 | Corps d'erreur : exposition du message, `details`, réponses hors route | Accepté (confirmé le 2026-10-29) | Non |
| ADR-0102 | Limites : taille de corps, recherches concurrentes, échéance de `timeout_ms`, features | Accepté (confirmé le 2026-10-29) | Non |
| ADR-0103 | Jobs de l'instance : table, états, rétention, file et `Retry-After` | Accepté (confirmé le 2026-10-30, complément du 2026-10-29 compris) | Non |
| ADR-0104 | Arrêt du serveur et jobs d'écriture | Accepté (confirmé le 2026-10-30) | Non |
| ADR-0105 | Administration des index par l'API : création à chaud, `PATCH`, suppression et corps des réponses | Accepté (confirmé le 2026-10-30) | Non |
| ADR-0106 | Empreinte de la chaîne d'analyse lexicale | Accepté (confirmé le 2026-10-30) | Non |
| ADR-0107 | Écriture de documents et de lots par l'API | Accepté (confirmé le 2026-10-30) | Non |
| ADR-0108 | Cohérence de l'époque après une compaction en échec | Accepté (confirmé le 2026-10-30) | Non |
| ADR-0109 | Route de recherche : champs inconnus, périmètre de l'échéance et preuve « API = CLI » | Accepté (confirmé le 2026-10-30) | Non |
| ADR-0110 | Recherche multi-index : corps, fusion, validation et preuve de l'encodage unique | Accepté (confirmé le 2026-10-30) | Non |
| ADR-0111 | Spécification OpenAPI : Swagger UI embarqué, sécurité, référence des routes et validation des exemples | Accepté (confirmé le 2026-10-30, compléments compris) | Non |
| ADR-0112 | Client Python HTTP : méthodes, exceptions typées, attente des jobs et tests contre le binaire | Accepté (confirmé le 2026-10-30) | Non |
| ADR-0113 | Charge concurrente : critère d'INV-02, mesure MES-16, protocole et démonstration J6 | Accepté (confirmé le 2026-10-30) | Non |
| ADR-0114 | Vérifier les liens intra-doc en intégration continue | Accepté (confirmé le 2026-10-31) | Non |
| ADR-0115 | Précisions de fin de P6 : ordre des refus de `POST documents` et identifiants d'une époque compactée | Accepté (confirmé le 2026-10-31) | Non |
| ADR-0116 | Concurrence entre la CLI et une instance en service : ouverture en lecture sans écriture et verrou d'écrivain par index | Accepté (décidé le 2026-10-31) | Non |

Aucun ADR de la phase n'est `Proposé` : rien n'attend une décision du propriétaire pour valider J6. Les deux tâches ouvertes par ADR-0116 (`OUVRIR-EN-LECTURE-SANS-ECRIRE`, `VERROU-ECRIVAIN-PAR-INDEX`) appartiennent à P8.

## 7. Écarts, idées, risques

- **Écart à la conception ouvert** : aucun. Aucun ADR `Proposé` ne dépend de P6. Les écarts de la phase ont tous été tranchés par un ADR `Accepté` (section 6).
- **Reprise demandée par `suivi.md`** (« Idées hors périmètre », 2026-10-29, ROUTE-RECHERCHE, `@dev`) : le test local `api_and_cli_return_the_same_results_on_the_legifacile_sample` dure **1 796 s** à l'époque de la tâche et **1 901,11 s** dans la série de l'orchestrateur du 2026-10-31, en `--release`. C'est le test le plus long du jalon ; il n'est pas rejoué en CI (cinquième exclusion nommée, ADR-0109). Un mode « plusieurs requêtes par appel » de `crible search` diviserait la durée par dix environ (la CLI recharge le modèle à chaque processus) ; c'est hors périmètre et **à décider par le propriétaire**. Aucune correction n'est demandée pour valider J6.
- **Test intermittent confirmé** : `serve_answers_the_probes_and_stops_on_sigterm` a échoué une fois sur la suite complète (`crates/crible/tests/serve_cli.rs:260`, `ConnectionReset`), vert 5/5 seul et vert dans la seconde suite complète. `suivi.md` note déjà ce symptôme pour `serve_stops_on_sigint` (même fichier, même ligne). Il ne bloque pas J6, mais mérite une tâche `CORRIGER-…` (section 8).
- **Message de fusion** : la fusion de ROUTES-DOCUMENTS-ET-LOTS est `fb663e5 merge: NOM` (paramètre non substitué de `@git`). Aucune réécriture d'historique n'est permise ; la ligne est signalée pour mémoire, sans effet sur J6.
- **`/docs` rend `303`** vers `/docs/` : c'est la forme d'ADR-0111, mais la consigne de préparation attendait `200` brut. Rien à corriger ; le navigateur suit la redirection.
- **Piège d'exploitation** : exporter `CRIBLE_ADMIN_KEY` dans le terminal du serveur le fait refuser au démarrage (`invalid_request`, ADR-0097). Le document sépare bien les deux terminaux ; le signaler à qui rejoue la démo d'une seule commande.
- **Report déjà consigné** : `GET /v1/indexes/{index}` parcourt le dossier pour `sizes.disk_bytes` (à mesurer au rejeu de MES-16, P8/P9) ; le client Python ne couvre pas `PATCH`/`DELETE` d'index, les instantanés ni `GET /v1/jobs` (à rouvrir seulement si le MCP ou P9 en a besoin) ; MES-16 est à rejouer en conteneur ou à l'échelle de P9.
- **Données à retirer après validation** : `data/mes16/` (2,6 Mo, sorties brutes des campagnes). Décision laissée au propriétaire, comme noté dans `suivi.md` (« Données de mesure conservées »).
- **Points à décider par le propriétaire** : (1) l'option « plusieurs requêtes par appel » de `crible search`, pour ramener la preuve API = CLI sous une minute ; (2) la conduite à tenir sur le test intermittent d'arrêt ; (3) le retrait de `data/mes16/` ; (4) le sort des trois lignes de « Idées hors périmètre » déjà renvoyées à P8/P9 (lignes 608, 611, 613 de `suivi.md`).

## 8. Si le jalon n'est pas validé

Aucune validation n'est demandée ici. Les propositions suivantes sont **hors périmètre** et n'ont pas été appliquées :

- `CORRIGER-ARRET-SOUS-CHARGE` — rendre `try_get` de `crates/crible/tests/serve_cli.rs` tolérant à une connexion réinitialisée pendant le démarrage ou l'arrêt (réessayer au lieu de `expect("the reply is readable")`), pour supprimer l'échec intermittent des deux tests d'arrêt sous la charge de la suite complète. Critère : 20 exécutions consécutives de `cargo test -p crible --test serve_cli` vertes, et aucune régression des assertions `200` de `/health` et `/ready`.
- `ACCELERER-PREUVE-API-CLI` (à décider) — si le propriétaire le veut, ajouter à `crible search` un mode « plusieurs requêtes par appel » pour ramener `api_and_cli_return_the_same_results_on_the_legifacile_sample` de 1 901 s à quelques minutes. Critère : résultats identiques au bit près sur les 300 questions × 3 modes, durée divisée par dix environ.

Les corrections qu'un relecteur demanderait après un refus deviendraient, elles, des tâches `CORRIGER-…` dans `docs/phases/P6-serveur-et-api.md`, avec leurs critères d'acceptation.
