# P8 — Docker et déploiement

**But** : une image CPU multi-architecture qui tourne sur le Mac mini, avec les bons volumes, un arrêt propre, un compose d'exemple et des mesures faites dans le conteneur.

**Jalon J8** :
- l'image tourne sur le Mac mini (arm64) et sur amd64 ;
- le compose d'exemple démarre Crible avec un client sur un réseau interne ;
- MES-02, MES-05 et MES-09 sont refaites dans le conteneur ;
- l'arrêt propre est vérifié.

---

### OUVRIR-EN-LECTURE-SANS-ECRIRE
- **Objectif** : une ouverture en lecture ne modifie rien sous le dossier de l'index.
- **Dépend de** : EXPORTER-LOT, NETTOYER-P4. `NETTOYER-P4` est la tâche hors plan qui a livré `GenerationStore::open_read_only` pour `search` et `inspect` (`recover` et `ensure_manifest` inchangés) ; `EXPORTER-LOT` a livré `crible export`, qui ouvre encore en écriture.
- **À lire** : `02-architecture.md` (« Organisation sur disque », « Reprise au démarrage »), `04-donnees-et-ingestion.md` (« Export, instantanés, restauration »), ADR-0116 (décision 1), ADR-0069 (décision 2), ADR-0056, `08-invariants-et-tests.md` (INV-04).
- **Livrables** :
  - `open_index` n'exécute plus `recover` ni `ensure_manifest` quand l'ouverture est en lecture ; les réparations restent à l'ouverture en écriture ;
  - `crible export` ouvre l'index en lecture, les codes binaires manquants étant construits en RAM comme pour `search` (ADR-0069, décision 2) ;
  - le commentaire d'`inspect` qui parle d'un montage inscriptible est corrigé.
- **Critères d'acceptation** :
  - sur un index qui porte un fichier `tmp-*`, un dossier de segment absent de la table `segments` et un manifeste absent, une ouverture en lecture laisse le dossier de l'index identique octet pour octet, `-wal` et `-shm` exceptés (comparaison de l'inventaire des chemins, des tailles et des blake3) ; c'est un test nommé `inv_04_*` ;
  - la même lecture rend les mêmes résultats qu'avant le changement, sur les mêmes requêtes ;
  - l'ouverture en écriture qui suit répare comme avant : le `tmp-*` et le segment non référencé disparaissent, le manifeste manquant est écrit ;
  - `crible export` ouvre en lecture et rend le même lot qu'avant le changement, à la date du manifeste près.
- **Invariants** : INV-04.
- **Hors périmètre** : la lecture depuis un volume réellement monté en lecture seule, le `-shm` du WAL exigeant un dossier inscriptible ; le verrou d'écrivain (`VERROU-ECRIVAIN-PAR-INDEX`) ; tout changement de format sur disque.

### VERROU-ECRIVAIN-PAR-INDEX
- **Objectif** : un seul processus écrit un index donné, et une erreur d'exploitant devient un refus immédiat et nommé.
- **Dépend de** : OUVRIR-EN-LECTURE-SANS-ECRIRE, DEMARRER-SERVEUR-AXUM.
- **À lire** : `02-architecture.md` (« Organisation sur disque », « Concurrence »), `05-api.md` (code `overloaded`), ADR-0116 (décision 2), ADR-0056 (verrou de présence et vérification d'identité), ADR-0108, ADR-0101.
- **Livrables** :
  - le verrou `<dossier de l'index>/WRITER_LOCK`, pris par `std::fs::File::try_lock` exclusif et non bloquant, avec la vérification d'identité après verrouillage d'ADR-0056 ;
  - dans le serveur, la prise du verrou à l'ouverture de chaque index, au démarrage comme à la création par l'API, sa tenue tant que l'index est servi et son relâchement à la suppression (job `delete_index`) ;
  - dans la CLI, la prise du verrou par chaque commande qui ouvre un index existant en écriture, pour toute sa durée ; refus immédiat, sans attente, en `overloaded` (503), avec un message qui nomme l'index et dit qu'un autre processus l'écrit, sans chemin du serveur ;
  - l'exclusion de `WRITER_LOCK` de l'inventaire d'époque, des instantanés et des exports ;
  - la règle (a) de l'ADR, déjà écrite dans `docs/02`, appliquée telle quelle : aucune commande d'écriture de la CLI sur un index servi, arrêt du serveur d'abord, lectures permises.
- **Critères d'acceptation** :
  - une commande d'écriture de la CLI, `compact` par exemple, est refusée en `503 overloaded` pendant qu'un vrai processus `crible serve` sert l'index ; le test de bout en bout lance le binaire, attend `/ready`, lance la commande, vérifie le code et le message, puis vérifie que l'index est intact ;
  - le verrou est libre après la mort du processus qui le tenait, y compris après un `SIGKILL` : la commande suivante passe ;
  - un fichier `WRITER_LOCK` recréé entre l'ouverture et le verrouillage ne donne pas deux verrous ;
  - `search`, `inspect` et `export` passent à côté d'un serveur qui tient le verrou ;
  - `WRITER_LOCK` n'apparaît ni dans un instantané, ni dans son `snapshot.json`, ni dans un export.
- **Invariants** : INV-04.
- **Hors périmètre** : un nouveau code d'erreur ; une attente avec délai ; un verrou partagé en lecture ; la lecture depuis un volume monté en lecture seule ; un réglage de configuration pour le verrou.

### PLAFONNER-JOURNAL-DE-DEBUG
- **Origine** : tâche hors plan ajoutée par le propriétaire le 2026-11-02, sur ADR-0127 accepté ce jour, d'après l'idée JOURNAL-DE-DEBUG du 2026-11-01 de « Idées hors périmètre » (`docs/suivi.md`) et le contrôle de fin de P7. Aucun plafond de taille n'existait : `ttl_days` borne la durée (ADR-0122, décision 8), pas les octets.
- **Objectif** : borner `debug.sqlite` en lignes et en octets, sans coût sur le chemin de requête, et rendre le plafond visible à un `admin`.
- **Dépend de** : JOURNAL-DE-DEBUG.
- **À lire** : ADR-0127 (décisions 1 à 7, « Tests attendus » et « Décision du propriétaire du 2026-11-02 »), ADR-0121, ADR-0122, `06-observabilite.md` (« Journal de debug »), `07-stack-et-deploiement.md` (`[debug_journal]`), `05-api.md` (`GET /v1/debug/queries`, composant `DebugQueryList`), `08-invariants-et-tests.md` (INV-15, INV-17).
- **Livrables** :
  - `[debug_journal]` : `max_rows` (500 000 par défaut) et `max_bytes` (2 Gio, 2 147 483 648 octets, par défaut), dans `DebugJournalConfig` et dans la liste fermée des clés de la section ; `0` refusé au démarrage pour les deux, `max_rows >= 1`, `max_bytes >= 1 Mio` ; surcharges `CRIBLE_DEBUG_JOURNAL__MAX_ROWS` et `CRIBLE_DEBUG_JOURNAL__MAX_BYTES` (ADR-0097 ; ADR-0127, décision 1) ;
  - le contrôle du plafond à **chaque vidage**, dans la transaction — insertion des lignes du vidage, puis suppression des plus anciennes (`created_at ASC, id ASC`) jusqu'à repasser sous les deux bornes —, et **une fois à l'ouverture**, après la purge de rétention ; compteur `trimmed` et drapeau `capped`, tous deux de processus (ADR-0127, décisions 3 et 4) ;
  - `max_bytes` mesuré sur la taille **utilisée**, `(page_count - freelist_count) × page_size` ; `file_bytes` pour l'opérateur, absent de la route (ADR-0127, décision 2) ;
  - `auto_vacuum = INCREMENTAL` à la création du fichier, puis `incremental_vacuum` borné à 256 pages par vidage, hors de la transaction ; aucun `VACUUM` complet ; procédure de `VACUUM` hors ligne, journal désactivé, documentée dans `docs/06` (ADR-0127, décision 5) ;
  - `warn` de plafond au plus une fois par intervalle de purge, avec `rows`, `bytes`, `file_bytes` et `trimmed` seulement (ADR-0122, décision 10) ;
  - les membres `capped`, `rows`, `bytes` et `trimmed` de `GET /v1/debug/queries`, dans la réponse, le composant `DebugQueryList` et l'exemple de `docs/05` ; l'exemple avait été vidé de ces membres le 2026-11-02 pour laisser le test des exemples vert, et la tâche les y remet avec l'implémentation (ADR-0127, décision 6) ;
  - rétention effective = **minimum** de `ttl_days` et de ce que les plafonds laissent tenir, la purge de durée précédant le trim (ADR-0127, décision 7).
- **Critères d'acceptation** :
  - `max_rows` atteint : les lignes les plus anciennes sont supprimées et les plus récentes gardées ; même contrôle sur `max_bytes`, y compris une ligne plus grosse que `max_bytes`, supprimée par le trim ;
  - les deux plafonds ensemble, et le cas où un trim pour l'un laisse l'autre loin de sa borne ;
  - purge de durée et trim dans le même vidage : la purge passe d'abord, et `trimmed` ne compte que le trim ;
  - le contrôle à l'ouverture ramène sous les bornes un fichier laissé au-dessus (arrêt brutal, baisse de configuration, fichier d'un binaire sans plafond) ;
  - sur un fichier neuf, `PRAGMA auto_vacuum` vaut `2` et `incremental_vacuum` fait baisser `freelist_count` sans changer le nombre de lignes ; le budget de 256 pages par vidage est respecté ;
  - `capped`, `rows`, `bytes` et `trimmed` sont justes, le schéma `DebugQueryList` reste fermé et le test des routes d'ADR-0111 passe ;
  - l'exemple JSON de `GET /v1/debug/queries` dans `docs/05-api.md` porte de nouveau `capped`, `rows`, `bytes` et `trimmed`, et le test `json_examples_conform_to_the_specification` du crate `crible-server` passe ;
  - `format_version` reste `1`, et un fichier écrit par la version bornée est relu par la version précédente, et inversement (INV-15) ;
  - le `warn` de plafond ne porte aucun texte de requête, aucun filtre et aucune clé ;
  - `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` et `cargo test` passent.
- **Invariants / mesures** : INV-17, par le test `inv_17_debug_journal_cap_never_blocks` sous `failpoints` ; INV-15 seulement si le schéma change, ce qu'ADR-0127 écarte.
- **Hors périmètre** : tout `VACUUM` complet exécuté par le serveur ; un plafond pour `analytics.sqlite` ; un membre de plafond sur `GET /v1/stats` ou `/ready` ; un changement de `format_version` ; le coût de `sizes.disk_bytes` et du `ReadPool` des documents, qui relèvent de `MESURER-EN-CONTENEUR-MAC`.

### RESUME-PUBLIE-COHERENT
- **Origine** : tâche hors plan ajoutée par le propriétaire le 2026-11-02, sur l'idée REUTILISER-LECTEURS-DENSES (`@dev`) du 2026-11-01 notée dans « Idées hors périmètre » (`docs/suivi.md`) et estimée par `@architecte` le 2026-11-02. Aucun ADR nouveau : la tâche applique ADR-0105, décision 3.
- **Objectif** : après un échec postérieur au commit, `GET /v1/indexes/{index}` et `GET /v1/indexes` rendent un résumé dont tous les comptages décrivent le **même** instantané, celui qui est servi, comme le veulent ADR-0105, décision 3, et `05-api.md:128` (« les comptes sont ceux de l'instantané publié »).
- **Constat** : `GenerationStore::state()` (`crates/crible-store/src/generation.rs:1175-1200`) prend `live_passages` au bitmap de l'instantané et `documents` / `passages` à SQLite. Le mélange apparaît quand un job commite `G+1` puis échoue avant de publier son instantané (écriture du manifeste) : `refresh_after_job` (`crates/crible-server/src/jobs.rs:870-905`) publie quand même le résumé. Il dure jusqu'au prochain job réussi ou au redémarrage, et `day_indexes` des analytics le fige pour la journée (ADR-0118, décision 5).
- **Dépend de** : REUTILISER-LECTEURS-DENSES.
- **À lire** : ADR-0105 (décision 3), ADR-0126, ADR-0118 (décision 5), `05-api.md` (résumé d'index et état).
- **Livrables** :
  - la correction : l'instantané porte `documents` et `passages`, remplis à sa construction, et le résumé publié les lit, au lieu d'interroger `records.sqlite` ; `last_batch` reste la dernière ligne de `batches` (ADR-0105, décision 3), asymétrie voulue et écrite dans le test ;
  - le test `crates/crible-server/tests/job_failure_after_commit.rs` aux nouvelles attentes, avec son commentaire réécrit ;
  - un test « `PATCH` après l'échec → détail et recherche d'accord » ;
  - la vérification, citée dans le compte rendu, des autres appelants de `state()` — création d'instantané (`crates/crible-store/src/snapshot.rs:214`, `:336-337`) et rapport de compaction (`crates/crible-store/src/compact.rs:260-261`) —, avec la correction si l'un d'eux peut aussi mêler deux générations.
- **Critères d'acceptation** :
  - après la séquence d'échec, le résumé publié ne mêle plus deux générations : `documents`, `live_passages`, `dead_passages` et `dead_ratio` décrivent tous l'instantané servi ;
  - le détail (`GET /v1/indexes/{index}`) et une recherche qui suit l'échec sont d'accord avec ce résumé ;
  - aucun format sur disque ni réponse d'API ne change de forme ;
  - aucune concurrence nouvelle : la lecture du résumé ne prend aucun verrou nouveau et ne bloque pas un job ;
  - `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` et `cargo test` passent.
- **Invariants / mesures** : aucun invariant cité, et c'est délibéré : la tâche corrige une cohérence interne sans changer un format sur disque (INV-15), une transaction (INV-03) ni un débit mesuré.
- **Note** : la publication de la génération commitée sans redémarrage — reprendre le commit dans le rafraîchissement, plutôt que d'attendre le job suivant — est **hors de cette tâche** ; elle demanderait un ADR.
- **Hors périmètre** : republier la génération commitée ; changer la forme d'une réponse d'API ; toucher aux analytics ; modifier ADR-0105.

---

### CONSTRUIRE-IMAGE-DOCKER
- **Objectif** : l'image de production.
- **Dépend de** : DEMARRER-SERVEUR-AXUM
- **À lire** : `07-stack-et-deploiement.md` (Docker). Référence possible : le `Dockerfile` de NextPlaid (`~/MattCorp/next-plaid/next-plaid-api/Dockerfile`, Apache 2.0).
- **Livrables** :
  - `docker/Dockerfile` : cargo-chef, build release, image d'exécution `debian:bookworm-slim` ;
  - la bibliothèque ONNX Runtime CPU de la bonne architecture, avec `ORT_DYLIB_PATH` ;
  - un utilisateur non-root ;
  - un `HEALTHCHECK` ;
  - `ENV ORT_DISABLE_TELEMETRY=1` dans l'image (ADR-0128), pour que la télémétrie du client 1DS d'ONNX Runtime reste coupée en conteneur ;
  - `docker/README.md`.
- **Critères d'acceptation** :
  - `docker buildx build --platform linux/arm64,linux/amd64` réussit ;
  - l'image démarre et `/ready` répond 200 avec un modèle et un index de test montés ;
  - `docker image inspect` montre `ORT_DISABLE_TELEMETRY=1` dans l'environnement de l'image (ADR-0128) ;
  - la taille de l'image est rapportée.

### DEFINIR-VOLUMES-ET-CONFIG
- **Objectif** : un déploiement correct sur Mac.
- **Dépend de** : CONSTRUIRE-IMAGE-DOCKER, VERROU-ECRIVAIN-PAR-INDEX
- **À lire** : `07-stack-et-deploiement.md` (volumes, configuration).
- **Livrables** :
  - `docker/crible.example.toml` ;
  - la documentation des volumes : `/data` et `/models` en volumes nommés, `/exchange` monté depuis l'hôte, la config en lecture seule ;
  - au démarrage, un avertissement si `/data` est un dossier monté depuis l'hôte (détection du système de fichiers), car le mmap y serait lent.
- **Critères d'acceptation** : la procédure du README fonctionne de bout en bout sur une machine vierge (vérifiée sur le Mac du propriétaire).

### VERIFIER-ARRET-PROPRE
- **Objectif** : aucune perte lors d'un `docker stop`.
- **Dépend de** : CONSTRUIRE-IMAGE-DOCKER, AGREGER-ANALYTICS
- **Livrables** : un test qui lance un `docker stop` pendant une attache, puis vérifie qu'après redémarrage l'état est G ou G+1 complet, que les analytics ont été vidées et que le job est terminé ou rejouable.
- **Critères d'acceptation** : le test passe 10 fois de suite.
- **Invariants** : INV-03, INV-04.

### COMPOSE-EXEMPLE
- **Objectif** : un modèle de déploiement par projet.
- **Dépend de** : DEFINIR-VOLUMES-ET-CONFIG
- **Livrables** : `docker/compose.example.yml`, avec le service `crible`, un client (conteneur Python de démo qui utilise `crible.Client`) et un réseau interne. Aucun port Crible n'est publié vers l'extérieur.
- **Critères d'acceptation** : `docker compose up` donne une démo qui cherche dans l'échantillon depuis le conteneur client.

### PUBLIER-IMAGE
- **Objectif** : la publication automatique.
- **Dépend de** : CONSTRUIRE-IMAGE-DOCKER, CONFIGURER-QUALITE-ET-CI
- **Livrables** : un workflow sur tag `v*` qui construit l'image multi-arch avec buildx et la pousse sur GHCR (`ghcr.io/<owner>/crible:<version>` et `:cpu`).
- **Critères d'acceptation** : le workflow est valide ; **le premier tag est poussé par le propriétaire** (l'agent ne pousse jamais).

### MESURER-EN-CONTENEUR-MAC
- **Objectif** : les chiffres réels dans l'environnement de production.
- **Dépend de** : DEFINIR-VOLUMES-ET-CONFIG, PLAFONNER-JOURNAL-DE-DEBUG. Note du 2026-11-02 : la mesure de P8 doit voir le comportement disque final du journal, plafond compris (ADR-0127, accepté le 2026-11-02).
- **Livrables** : MES-02, MES-05 et MES-09 refaites dans Docker sur le Mac mini, avec la RAM de la VM indiquée, et comparées aux mesures natives. Un compte rendu en cas d'écart important. **Cas de mesure ajouté le 2026-10-27 (ADR-0073, ADR-0052 décision 3)** : la recette de l'attache (contrôle 9), avec les fils et réglages d'ONNX Runtime en conteneur. Mesurer le coût par jeton ré-encodé et le temps de ré-encodage des 200 passages tirés pour une taille de lot de 1 et pour une taille supérieure (B = 4 et 8 suffisent), avec le protocole de MES-08, campagne du 2026-10-27 (tailles entrelacées, trois tours, encadrement par B = 1, test de linéarité sur copies du plus long passage). But : savoir si un parallélisme entre lignes apparaît avec les cœurs et le voisinage du conteneur, ce que le Mac de développement n'a pas montré (8 copies coûtent 8,05 à 8,08 fois un appel seul). Aucun réglage n'est retenu sans cette mesure, et changer de fournisseur d'exécution ou ré-exporter le graphe demande un nouvel ADR. **Cas de mesure ajouté le 2026-10-27 (décision du propriétaire ; ADR-0077, ADR-0080)** : le rescoring fp16, à chaud et à froid, avec les deux lignes de MES-03 et au N par défaut de 5000, aux N ∈ {1000, 2000, 5000} du protocole d'ADR-0077, et avec les séries d'ADR-0080, dont la série nouvelle « searcher déjà ouvert, cache déplacé après l'ouverture ». Note : le froid sous Linux — `posix_fadvise(POSIX_FADV_DONTNEED)`, `drop_caches` sous root, comptabilité cgroup — n'est défini par aucun document ; il est tranché par un ADR au début de la tâche. Sur Linux x86_64, les pages font 4 Kio : il reste environ une page par candidat, avec quatre fois moins d'octets par page (ADR-0080). **Cas de mesure ajouté le 2026-11-02 (décision du propriétaire, contrôle de fin de P7)** : deux coûts de routes, mesurés à 8,58 M passages avec le nombre de segments réel. D'abord le coût de `sizes.disk_bytes` : `GET /v1/indexes/{index}` parcourt le dossier de l'index à chaque appel (`crates/crible-server/src/indexes.rs`, `directory_size`), et ADR-0105 range ce coût dans « À surveiller » ; si le coût dépasse quelques millisecondes, la taille est portée dans l'état publié au commit (ADR-0105), ce qui serait alors une tâche à part. Ensuite la connexion par requête de `GET /v1/indexes/{index}/documents/{doc_id}` : la route ouvre une connexion `ReadPool` (capacité 1) au `records.sqlite` de l'époque à chaque requête (`crates/crible-server/src/documents.rs`, commentaire des lignes 528-533) ; mesurer la latence de la route et la part de cette ouverture, un pool par époque étant retenu si la latence de lecture compte, ce qui serait alors une tâche à part. Source : lignes « ROUTES-INDEX » et « ROUTES-DOCUMENTS-ET-LOTS » du 2026-10-29 dans « Idées hors périmètre » de `docs/suivi.md`.
- **Critères d'acceptation** : les rapports sont consignés ; une recommandation de RAM pour la VM Docker est faite ; le coût par jeton de la recette et l'effet d'une taille de lot supérieure à 1 y sont chiffrés, avec la taille de lot retenue ou le constat qu'aucune ne l'emporte ; le rescoring fp16 y est chiffré à chaud et à froid, aux N du protocole d'ADR-0077 et au N par défaut de 5000, avec le protocole de froid sous Linux retenu par l'ADR pris au début de la tâche ; le coût de `sizes.disk_bytes` de `GET /v1/indexes/{index}` et la latence de `GET /v1/indexes/{index}/documents/{doc_id}`, avec la part de l'ouverture de sa connexion, y sont chiffrés à 8,58 M passages au nombre de segments réel, et disent si la borne de quelques millisecondes est dépassée et si un pool par époque se justifie, la correction restant une tâche à part.
- **Invariants / mesures** : MES-02, MES-03, MES-05, MES-09 ; MES-08 (volet recette et tailles de lot).
