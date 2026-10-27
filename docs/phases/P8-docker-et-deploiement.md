# P8 — Docker et déploiement

**But** : une image CPU multi-architecture qui tourne sur le Mac mini, avec les bons volumes, un arrêt propre, un compose d'exemple et des mesures faites dans le conteneur.

**Jalon J8** :
- l'image tourne sur le Mac mini (arm64) et sur amd64 ;
- le compose d'exemple démarre Crible avec un client sur un réseau interne ;
- MES-02, MES-05 et MES-09 sont refaites dans le conteneur ;
- l'arrêt propre est vérifié.

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
- **Dépend de** : CONSTRUIRE-IMAGE-DOCKER
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
