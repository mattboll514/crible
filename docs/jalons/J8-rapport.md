# Rapport de jalon J8 — Docker et déploiement

> Rédigé par l'agent `jalon` (commande `/jalon J8`). **Seul le propriétaire peut valider le jalon**, dans `docs/suivi.md`.

- **Date** : 2026-11-03
- **Branche de phase / commit** : `phase/P8-docker-et-deploiement` / `2aa2f12` (les deux derniers commits, `7b41a56` et `2aa2f12`, ne touchent que `docs/` ; la CI distante est verte sur `fa7fbb8`)
- **Machine** : MacBook Air M3 (`Mac15,12`), Apple M3 à 8 cœurs, 24 Gio, macOS 15.7.7 ; Docker Desktop 28.5.1, VM `linux/aarch64` à 8 cœurs et 11,7 Gio, pages de 4 Kio
- **Verdict de l'agent** : `PRET-POUR-VALIDATION`

Les trois critères du jalon (`docs/phases/P8-docker-et-deploiement.md:5-8`) sont tenus : l'image arm64 tourne sur la machine de développement et l'image amd64 se construit (7 min 26 s sous QEMU) ; MES-02, MES-05 et MES-09 sont refaites dans le conteneur, à titre indicatif (ADR-0129, décision 4 ; protocole ADR-0135) ; l'arrêt propre est vérifié par deux tests Docker verts. Les 12 tâches de P8 sont `FAIT`. Aucun ADR de la phase n'est resté `Proposé`.

## 1. Tâches de la phase

Les 12 tâches suivies dans `docs/suivi.md` sont `FAIT` : les 9 tâches de la fiche, `EVALUER-PAR-API` (hors plan, ajoutée par le propriétaire au rapport de J7) et les deux tâches hors plan de la fin de phase, `PLAFONNER-JOURNAL-DE-DEBUG`, `RESUME-PUBLIE-COHERENT` et `NETTOYER-P8`. Aucune n'est `A-FAIRE`, `EN-COURS` ou `BLOQUE`.

| Tâche | Statut dans `suivi.md` | Commit | Remarque |
|---|---|---|---|
| OUVRIR-EN-LECTURE-SANS-ECRIRE | FAIT | fusion `b25acb2` | ADR-0116, décision 1 ; INV-04 |
| VERROU-ECRIVAIN-PAR-INDEX | FAIT | fusion `85169d7` | ADR-0116, décision 2 ; INV-04 |
| PLAFONNER-JOURNAL-DE-DEBUG | FAIT | fusion `1d8e542` | Hors plan ; ADR-0127 |
| RESUME-PUBLIE-COHERENT | FAIT | fusion `2ed4e3e` | Hors plan ; application d'ADR-0105, décision 3 |
| CONSTRUIRE-IMAGE-DOCKER | FAIT | fusion `ef69a42` | `docker/Dockerfile`, `docker/README.md` |
| DEFINIR-VOLUMES-ET-CONFIG | FAIT | fusion `af9baf5` | `docker/crible.example.toml`, avertissement sur un partage d'hôte |
| EVALUER-PAR-API | FAIT | fusion `4735b00` | Hors plan ; ADR-0131 ; INV-13 |
| VERIFIER-ARRET-PROPRE | FAIT | fusion `3b15d92` | INV-03 et INV-04 ; ADR-0132 |
| COMPOSE-EXEMPLE | FAIT | fusion `25f531d` | Réseau interne, aucun port publié |
| PUBLIER-IMAGE | FAIT | fusion `0aaab7c` | ADR-0133 ; aucun tag poussé, geste du propriétaire |
| MESURER-EN-CONTENEUR-MAC | FAIT | fusion `81dc837` | ADR-0135 ; MES-02, MES-03, MES-05, MES-08, MES-09 en conteneur |
| NETTOYER-P8 | FAIT | fusion `fa7fbb8` | Hors plan, passages A à G |

## 2. Qualité

Toutes les commandes ont été lancées par `@jalon` le 2026-11-03 sur `2aa2f12`. La suite Rust a été découpée par crate, comme demandé ; les durées incluent la compilation incrémentale.

| Contrôle | Commande | Résultat |
|---|---|---|
| Formatage | `cargo fmt --all -- --check` | exit 0, 1,36 s |
| Lints | `cargo clippy --all-targets --locked -- -D warnings` | exit 0, 0,77 s (cible déjà compilée) |
| Tests Rust (debug) | `cargo test --locked -p <crate>`, crate par crate | **1 234 passés, 0 échoué, 28 ignorés** |
| Tests `failpoints` (`crible-store`) | `cargo test -p crible-store --locked --features failpoints` | **414 passés, 0 échoué, 1 ignoré**, 91 s |
| Tests `failpoints` (`crible-server`) | `cargo test -p crible-server --locked --features failpoints` | **352 passés, 0 échoué, 0 ignoré** |
| Tests `failpoints` (`crible`) | `cargo test -p crible --locked --features failpoints` | **137 passés, 0 échoué, 18 ignorés** |
| Tests Python du client | `cd clients/python && uv run pytest` | **107 passés, 12 désélectionnés**, 0,31 s |
| Tests Python des outils | `cd tools && uv run --no-sync pytest -q tests` puis `... -q bench` | **521 passés, 1 sauté** (188 + 333), 359,7 s au total |

Détail des tests en mode debug :

| Paquet | Passés | Échoués | Ignorés | Durée |
|---|---:|---:|---:|---:|
| `crible-core` | 179 | 0 | 1 | 12,4 s |
| `crible-encode` | 18 | 0 | 9 | 10,5 s |
| `crible-search` | 204 | 0 | 0 | 53,0 s |
| `crible-store` | 380 | 0 | 0 | 76,4 s |
| `crible-server` | 317 | 0 | 0 | 72,0 s |
| `crible` | 136 | 0 | 18 | 127,2 s |
| **Total** | **1 234** | **0** | **28** | — |

Les 28 tests ignorés sont les tests qui demandent ONNX Runtime, Docker ou l'échantillon Legifacile. La CI distante est verte sur `fa7fbb8` (run `41691838991`) : les jobs `Rust`, `Rust with ONNX Runtime`, `Python` et `Tools` passent, et le job `Rust with ONNX Runtime` y lance les tests `#[ignore]` qui demandent ONNX Runtime. Ces tests n'ont donc pas été rejoués ici.

## 3. Invariants de la phase

Les fiches de P8 citent INV-04 (ouverture en lecture, verrou, arrêt propre), INV-13 (mode API de `crible eval`) et INV-03 (arrêt propre). Les tests Docker ont été lancés avec l'image `crible:dev` reconstruite depuis `2aa2f12`.

| Invariant | Test(s) | Résultat |
|---|---|---|
| **INV-04** — une ouverture en lecture n'écrit rien sous le dossier de l'index | `crible-store/tests/inv_04_read_only_open.rs` : `inv_04_read_only_open_leaves_the_index_directory_untouched` (2 tests du binaire) | **2 passés, 0 échoué**, 0,16 s |
| **INV-04** — le verrou d'écrivain est rendu après la mort du processus | `crible/tests/verrou_ecrivain_par_index.rs` : `inv_04_writer_lock_is_free_after_the_serving_process_is_killed`, `inv_04_writer_lock_is_left_in_place_by_the_write_opening` | **2 passés, 0 échoué**, 1,20 s |
| **INV-13** — le mode API de `crible eval` n'écrit ni texte de requête ni clé | `crible/tests/eval_api_cli.rs` : `inv_13_eval_api_mode_never_writes_query_text_or_key` | **1 passé, 0 échoué**, 1,28 s |
| **INV-13** — ni les analytics ni le journal ne portent de contenu | `crible-server/tests/inv_13_analytics_without_content.rs` : 3 tests `inv_13_*` | **3 passés, 0 échoué**, 0,88 s |
| **INV-03** — un `docker stop` pendant une attache laisse une génération entière | `crible/tests/docker_stop_during_attach.rs` : `inv_03_a_docker_stop_during_an_attach_leaves_one_complete_generation` | **1 passé, 0 échoué** |
| **INV-04** — un `docker stop` pendant une attache ne laisse pas de travail à moitié publié | même fichier : `inv_04_a_docker_stop_during_an_attach_leaves_the_last_committed_generation` | **1 passé, 0 échoué** ; les deux tests Docker ensemble : **2 passés, 0 échoué**, 13,52 s |

Les 18 tests ignorés de `crible` en mode debug comprennent ces deux tests Docker et les tests qui demandent ONNX Runtime. Les tests Docker se lancent avec `CRIBLE_DOCKER_IMAGE=crible:dev` et `-- --include-ignored --test-threads=1`, comme documenté dans `docker/README.md` et dans l'en-tête du fichier. Leur critère de 10 passages sur 10 a été tenu pendant `VERIFIER-ARRET-PROPRE` (13 à 15 s par passage) ; `@jalon` en a rejoué un passage vert après reconstruction de l'image.

## 4. Mesures de la phase

Toutes les mesures en conteneur sont **indicatives** (ADR-0129, décision 4) : la latence contraignante se mesure sur le Mac mini en P10, par `MESURER-SUR-MAC-MINI`.

| Mesure | Rapport | Chiffre clé | Cible | Atteinte ? |
|---|---|---|---|---|
| MES-02 — latence du scan binaire | `MES-02-latence-scan-binaire.md`, section « Campagne en conteneur (P8, ADR-0135) » ; artefact `MES-02-….2026-11-03.conteneur.json` | pool complet de 7 740 000 passages : p50 **27,622 ms**, p95 **31,201 ms** à N = 1000 ; 1 M dispersé : p50 10,910 ms | Aucune cible pour cette mesure : indicative (ADR-0129) | Sans objet |
| MES-05 — latence d'encodage d'une requête | `MES-05-latence-encodage.md`, section du 2026-11-03 ; artefact `MES-05-….2026-11-03.conteneur.json` | p50 int8 **41,5 à 47,5 ms**, fp32 **100,3 à 103,6 ms** ; conteneur ≈ **2,4 fois** le natif du même jour | Budget indicatif de `docs/03` : 20 à 50 ms | Oui pour int8, non pour fp32 sur cette VM ; recommandation d'int8 à valider |
| MES-09 — RAM réelle par index | `MES-09-ram-reelle.md`, section du 2026-11-03 ; artefact `MES-09-….2026-11-03.conteneur.json` | plancher **2 637 à 2 650 MiB** (2,58 à 2,59 Gio) ; RSS en régime 9 777 à 9 972 MiB, dont 8,6 Gio de cache évincible | Recommandation de RAM pour la VM Docker | Oui : minimum **4 Go**, confortable **12 Go** |
| MES-03 — rescoring fp16 en conteneur (cas de la fiche) | `MES-03-latence-rescoring.md`, sections conteneur et `conteneur-limite` ; artefacts du 2026-11-03 | à chaud, k = 100 : 1,1 ms à N = 1000, 1,6 ms à N = 2000, 288 ms à N = 5000 (pages non résidentes) | Aucune cible en conteneur (ADR-0129) | Sans objet |
| MES-08 — recette et tailles de lot (cas de la fiche) | `MES-08-duree-attache-quotidienne.md`, section du 2026-11-03 ; artefact `MES-08-….2026-11-03.conteneur.json` | B = 8 : **−23,5 %** en conteneur | Aucune taille de lot retenue sans cette mesure ; proposition seulement (ADR-0073) | Proposition, à trancher en P10 |
| Routes de lecture (cas ajouté le 2026-11-02) | `routes-en-conteneur-2026-11-03.md` ; artefact `routes-en-conteneur-2026-11-03.json` | `GET /v1/indexes/{index}` : p50 **0,401 ms**, p95 **0,491 ms** ; document : p95 0,494 ms ; `sizes.disk_bytes` 0,048 ms | Seuil d'ADR-0135 : 5 ms en p95 | Oui, dix fois sous le seuil |

Deux réserves des rapports comptent pour la lecture du jalon : la machine n'était pas au repos (natif du jour environ deux fois plus lent que les campagnes publiées, et la campagne MES-02 le dit) ; et le comportement d'un index multi-segments en conteneur n'est pas mesuré, la copie à 8 segments étant prête mais non exercée.

## 5. Démo à rejouer par le propriétaire

La démonstration complète, jouée le 2026-11-03, est dans [`J8-demo.md`](J8-demo.md). Commandes exactes, depuis la racine du dépôt :

```
cd /Users/mattboll/MattCorp/crible
docker compose -f docker/compose.example.yml -p j8-demo down -v --remove-orphans
rm -rf data/j8-demo
mkdir -p data/j8-demo/exchange
time docker build -f docker/Dockerfile -t crible:dev .
cat > data/j8-demo/compose.override.yml <<YML
services:
  crible:
    volumes:
      - ${PWD}/data/j8-demo/exchange:/exchange
YML
docker compose -f docker/compose.example.yml -f data/j8-demo/compose.override.yml -p j8-demo build
docker compose -f docker/compose.example.yml -f data/j8-demo/compose.override.yml -p j8-demo run --rm --no-deps \
  -v "$PWD/data/indexes/synth-small:/seed/index:ro" \
  -v "$PWD/data/models/synthetic-external:/seed/model:ro" \
  --entrypoint sh crible -c 'mkdir -p /data/indexes /models && cp -a /seed/index /data/indexes/synth-small && cp -a /seed/model /models/synthetic-external'
docker compose -f docker/compose.example.yml -f data/j8-demo/compose.override.yml -p j8-demo up -d crible
CID=$(docker compose -f docker/compose.example.yml -f data/j8-demo/compose.override.yml -p j8-demo ps -q crible)
for i in $(seq 1 60); do
  code=$(docker exec "$CID" curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:8080/ready)
  [ "$code" = 200 ] && break
  sleep 1
done
echo "ready after ${i}s: $code"
docker exec "$CID" curl -sS -w '\nHTTP %{http_code}\n' http://127.0.0.1:8080/health
docker exec "$CID" curl -sS -w '\nHTTP %{http_code}\n' http://127.0.0.1:8080/ready
docker compose -f docker/compose.example.yml -f data/j8-demo/compose.override.yml -p j8-demo run --rm --no-deps client
time docker stop -t 65 "$CID"
echo "docker stop exit=$?"
docker inspect -f 'ExitCode={{.State.ExitCode}} OOMKilled={{.State.OOMKilled}} Status={{.State.Status}}' "$CID"
docker logs --tail 4 "$CID" 2>&1
time docker buildx build --platform linux/amd64 -f docker/Dockerfile -t crible:dev-amd64 --load .
docker image inspect -f 'arch={{.Architecture}} os={{.Os}} size={{.Size}}' crible:dev-amd64
docker run --rm --platform linux/amd64 --entrypoint crible crible:dev-amd64 --version
docker compose -f docker/compose.example.yml -f data/j8-demo/compose.override.yml -p j8-demo down -v --remove-orphans
```

Attendu : `/health` et `/ready` à 200 dans le conteneur (aucun port n'est publié, c'est le modèle du Compose), la recherche du client rend `top result: synth-01872 passage 2 (categorie=cat-00), score 0.9481` et `3 results out of 9750 passages` ; `docker stop -t 65` sort en 0, le conteneur a `ExitCode=0`, et le journal porte `received SIGTERM`, le drainage puis `the analytics are flushed` ; l'image amd64 se construit et répond `crible 0.1.0` sous émulation ; le nettoyage supprime conteneur, réseau et volumes, et garde les images.

Aucun écart : chaque commande a rendu le résultat attendu, y compris la génération 5 de l'index et le code de sortie 0 de l'arrêt. La seule précaution non évidente est le fichier de surcharge : le Compose d'exemple monte `../exchange` depuis `docker/`, c'est-à-dire la racine du dépôt ; la surcharge redirige ce seul montage sous `data/j8-demo/`, sans toucher aux volumes nommés, au réseau interne ni à l'absence de port publié.

## 6. Décisions (ADR) de la phase

| ADR | Titre | Statut | Bloque la validation ? |
|---|---|---|---|
| ADR-0131 | Mode API de `crible eval` | Accepté (2026-11-02, confirmé le 2026-11-03) | Non |
| ADR-0132 | Exclure de la CI les tests d'arrêt en conteneur | Accepté (2026-11-03, erratum du 2026-11-03) | Non |
| ADR-0133 | Politique de publication de l'image | Accepté (2026-11-03 ; complément daté du 2026-11-03) | Non |
| ADR-0134 | Progression des jobs | Accepté (2026-11-03) | Non |
| ADR-0135 | Protocole de la mesure en conteneur sur la machine de développement | Accepté (2026-11-03) | Non |
| ADR-0097 | Configuration de l'instance (complément du 2026-11-03 : onze variables exclues) | Accepté, complément confirmé par le propriétaire le 2026-11-03 | Non |

Aucun ADR de la phase n'est resté `Proposé`. Les deux compléments datés du 2026-11-03 (ADR-0097 et ADR-0133) ont été confirmés par le propriétaire le 2026-11-03, après avoir été acceptés par anticipation pendant `NETTOYER-P8`.

## 7. Écarts, idées, risques

- **Écarts à la conception ouverts** : aucun écart nouveau relevé par `@jalon`. Les écarts traités pendant la phase sont documentés par les ADR ci-dessus (route d'`EVALUER-PAR-API` différente de la fiche, ADR-0131 ; tests Docker exclus de la CI, ADR-0132 ; politique de publication, ADR-0133 ; vocabulaire et granularité de `progress`, ADR-0134).
- **Risques constatés** :
  - les mesures en conteneur sont **indicatives** et la machine n'était pas au repos ; elles ne valident ni n'invalident la cible de latence, qui reste `MESURER-SUR-MAC-MINI` en P10 ;
  - MES-02 réserve le comportement d'un index **multi-segments en conteneur** : la copie à 8 segments est prête, mais aucune mesure ne l'exerce ;
  - à 8 Go de RAM pour la VM (le réglage de `docs/07`), le rescoring à N = 5000 ne tient pas en cache ; MES-09 propose soit **N = 2000** (rappel@10 0,9887, sous la cible de 0,99, donc un ADR serait nécessaire), soit 12 Go de confortable.
- **Points à décider par le propriétaire** :
  - retenir ou non la recommandation de MES-05 : `query_precision = "int8"` pour un budget d'encodage en conteneur, le fp32 dépassant 100 ms sur cette VM ;
  - retenir ou non la recommandation de RAM de MES-09 (minimum 4 Go, confortable 12 Go) et, si la cible reste à 8 Go, trancher N = 2000 par un nouvel ADR ;
  - décider du sort des données de mesure et de démonstration après J8 : `data/mes-p8-conteneur/` (17 Go), le volume Docker `crible-mes-p8-data` et l'image `crible-mes-p8-harness:dev` (32,9 Go et 210 Mo), et `data/j8-demo/` ;
  - décider si les deux propositions conditionnelles de `NETTOYER-P8` (« Idées hors périmètre ») méritent une tâche.
- **Propositions `CORRIGER-…`** : aucune. `@jalon` n'a trouvé ni défaut de code, ni écart de démonstration, ni test instable pendant les vérifications.

## 8. Si le jalon n'est pas validé

Les corrections demandées deviendraient des tâches `CORRIGER-…` dans `docs/phases/P8-docker-et-deploiement.md`, avec leurs critères d'acceptation. Aucune n'est proposée à ce jour.
