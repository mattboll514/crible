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
  - `docker/README.md`.
- **Critères d'acceptation** :
  - `docker buildx build --platform linux/arm64,linux/amd64` réussit ;
  - l'image démarre et `/ready` répond 200 avec un modèle et un index de test montés ;
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
- **Dépend de** : DEFINIR-VOLUMES-ET-CONFIG
- **Livrables** : MES-02, MES-05 et MES-09 refaites dans Docker sur le Mac mini, avec la RAM de la VM indiquée, et comparées aux mesures natives. Un compte rendu en cas d'écart important. **Cas de mesure ajouté le 2026-10-27 (ADR-0073, ADR-0052 décision 3)** : la recette de l'attache (contrôle 9), avec les fils et réglages d'ONNX Runtime en conteneur. Mesurer le coût par jeton ré-encodé et le temps de ré-encodage des 200 passages tirés pour une taille de lot de 1 et pour une taille supérieure (B = 4 et 8 suffisent), avec le protocole de MES-08, campagne du 2026-10-27 (tailles entrelacées, trois tours, encadrement par B = 1, test de linéarité sur copies du plus long passage). But : savoir si un parallélisme entre lignes apparaît avec les cœurs et le voisinage du conteneur, ce que le Mac de développement n'a pas montré (8 copies coûtent 8,05 à 8,08 fois un appel seul). Aucun réglage n'est retenu sans cette mesure, et changer de fournisseur d'exécution ou ré-exporter le graphe demande un nouvel ADR. **Cas de mesure ajouté le 2026-10-27 (décision du propriétaire ; ADR-0077, ADR-0080)** : le rescoring fp16, à chaud et à froid, avec les deux lignes de MES-03 et au N par défaut de 5000, aux N ∈ {1000, 2000, 5000} du protocole d'ADR-0077, et avec les séries d'ADR-0080, dont la série nouvelle « searcher déjà ouvert, cache déplacé après l'ouverture ». Note : le froid sous Linux — `posix_fadvise(POSIX_FADV_DONTNEED)`, `drop_caches` sous root, comptabilité cgroup — n'est défini par aucun document ; il est tranché par un ADR au début de la tâche. Sur Linux x86_64, les pages font 4 Kio : il reste environ une page par candidat, avec quatre fois moins d'octets par page (ADR-0080).
- **Critères d'acceptation** : les rapports sont consignés ; une recommandation de RAM pour la VM Docker est faite ; le coût par jeton de la recette et l'effet d'une taille de lot supérieure à 1 y sont chiffrés, avec la taille de lot retenue ou le constat qu'aucune ne l'emporte ; le rescoring fp16 y est chiffré à chaud et à froid, aux N du protocole d'ADR-0077 et au N par défaut de 5000, avec le protocole de froid sous Linux retenu par l'ADR pris au début de la tâche.
- **Invariants / mesures** : MES-02, MES-03, MES-05, MES-09 ; MES-08 (volet recette et tailles de lot).
