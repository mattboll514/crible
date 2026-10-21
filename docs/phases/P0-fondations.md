# P0 — Fondations

**But** : un workspace propre, les types partagés et validés, le format de lot spécifié en code, et la référence pgvector mesurée pour fixer les cibles.

**Jalon J0** :
- la CI est verte ;
- les configs d'index et les manifestes de lot (valides comme invalides) sont couverts par des tests ;
- MES-00 est consignée ;
- les cibles de latence et de rappel sont écrites dans `docs/suivi.md`.

---

### INITIALISER-WORKSPACE-RUST
- **Objectif** : créer le workspace Cargo et les six crates vides qui compilent.
- **Dépend de** : —
- **À lire** : `07-stack-et-deploiement.md` (organisation du dépôt, dépendances).
- **Livrables** :
  - `Cargo.toml` (workspace) et `rust-toolchain.toml` (stable épinglée) ;
  - les crates `crible-core`, `crible-store`, `crible-encode`, `crible-search`, `crible-server` (bibliothèques) et `crible` (binaire `crible` affichant `--version`) ;
  - les dépendances communes déclarées dans `[workspace.dependencies]`, avec des versions épinglées vérifiées ;
  - l'en-tête de licence `SPDX-License-Identifier: Apache-2.0` en tête de chaque fichier source.
- **Critères d'acceptation** :
  - `cargo build`, `cargo test`, `cargo fmt --check` et `cargo clippy --all-targets -- -D warnings` passent ;
  - `cargo run -p crible -- --version` affiche la version.
- **Hors périmètre** : toute logique métier.

### CONFIGURER-QUALITE-ET-CI
- **Objectif** : rendre la qualité automatique.
- **Dépend de** : INITIALISER-WORKSPACE-RUST
- **À lire** : `07-stack-et-deploiement.md` (CI).
- **Livrables** :
  - `.github/workflows/ci.yml` : fmt, clippy `-D warnings`, test (Linux), plus `pytest` quand `clients/python` existera (étape conditionnelle) ;
  - `rustfmt.toml` ;
  - les lints de workspace : interdire `unwrap` et `expect` hors tests (`clippy::unwrap_used`, `clippy::expect_used` en `warn`, qui devient bloquant avec `-D warnings`) ;
  - `Makefile` ou `justfile` avec les cibles `check`, `test` et `lint`.
- **Critères d'acceptation** :
  - `make check` (ou `just check`) passe en local ;
  - le workflow est syntaxiquement valide (`actionlint` s'il est disponible).
- **Hors périmètre** : la publication d'images (P8).

### DEFINIR-TYPES-DE-BASE
- **Objectif** : les types partagés par toutes les crates.
- **Dépend de** : INITIALISER-WORKSPACE-RUST
- **À lire** : `00-glossaire.md` ; `02-architecture.md` (identifiants) ; `05-api.md` (erreurs).
- **Livrables** : dans `crible-core` :
  - `DocId` (validé : non vide, ≤ 512 octets) et `PassageKey` (`doc_id#ord`, analyse et affichage) ;
  - `Iid(u32)`, **non sérialisable par serde**, pour qu'un iid ne puisse pas fuiter (INV-01) ;
  - `Generation(u64)` et `Epoch(u32)` ;
  - `ErrorCode` (tous les codes de `05-api.md`, avec leur statut HTTP) et `CribleError` (thiserror).
- **Critères d'acceptation** :
  - tests unitaires de validation de `DocId` et d'aller-retour de `PassageKey` ;
  - un test vérifie la correspondance de chaque `ErrorCode` avec son code HTTP ;
  - `Iid` n'implémente pas `Serialize` (test de compilation négatif ou commentaire justifié).
- **Invariants** : INV-01.
- **Hors périmètre** : les filtres (tâche suivante).

### DEFINIR-CONFIG-INDEX
- **Objectif** : config d'index, schéma, modèle et AST des filtres, avec leur validation.
- **Dépend de** : DEFINIR-TYPES-DE-BASE
- **À lire** : `04-donnees-et-ingestion.md` (configuration d'un index, le modèle) ; `05-api.md` (grammaire des filtres).
- **Livrables** : dans `crible-core` :
  - `IndexConfig`, `Schema`, `FieldType` et `FieldLevel` ;
  - `ModelConfig` (`kind` `onnx` ou `external`), avec le calcul de l'**empreinte** (blake3 du JSON canonique) ;
  - la séparation figé / réglable : une fonction `apply_patch(config, patch)` renvoie `immutable_setting` en nommant le champ ;
  - `Filter` (AST), l'analyse depuis le JSON et la validation contre un schéma, avec des messages qui nomment ce qui était attendu et le chemin JSON.
- **Critères d'acceptation** :
  - les exemples JSON de `04-donnees-et-ingestion.md` et de `05-api.md` sont analysés et validés ;
  - au moins 15 cas invalides testés (champ inconnu, mauvais opérateur, mauvais type, profondeur > 8, `in` > 1 000 valeurs…) ;
  - l'empreinte est stable : un test fige sa valeur sur une config connue ;
  - modifier un réglage figé avec `apply_patch` donne `immutable_setting`.
- **Invariants** : INV-16 (au niveau des types).
- **Hors périmètre** : l'évaluation des filtres (P1).

### DEFINIR-FORMAT-LOT
- **Objectif** : les types du manifeste de lot et les règles de validation qui ne demandent pas de lire le Parquet.
- **Dépend de** : DEFINIR-CONFIG-INDEX
- **À lire** : `04-donnees-et-ingestion.md` (format de lot v1, contrôles 1 à 4).
- **Livrables** :
  - `BatchManifest` (serde) dans `crible-core` ;
  - `check_manifest(manifest, index_config, model_config) -> Result<(), CribleError>` : format, version, mode, compatibilité du modèle (`incompatible_model`), cohérence des comptes et de la liste des fichiers ; `model_config` est la configuration du modèle résolue (`&ModelConfig`), utilisée pour comparer le modèle du manifeste champ par champ ;
  - `docs/format-lot-v1.md` : la spécification de référence, colonnes Parquet comprises, à partir de `04` (c'est elle que le code Python suivra).
- **Critères d'acceptation** :
  - l'exemple de manifeste du doc 04 est valide ;
  - des tests couvrent un mauvais `format_version`, un modèle incompatible (chaque champ), `replace_all` avec un fichier `deletions`, un fichier absent du manifeste.
- **Hors périmètre** : la lecture Parquet (P1).

### MESURER-REFERENCE-PGVECTOR
- **Objectif** : mesurer MES-00, c'est-à-dire la référence de la recherche actuelle de Legifacile (pgvector), **sur le Mac mini de production**. Ces chiffres fixent les cibles de Crible.
- **Dépend de** : —
- **À lire** :
  - `08-invariants-et-tests.md` (mesures, cibles) et `docs/mesures/README.md` ;
  - `docs/adr/0032-acces-a-la-production.md` ;
  - **`.env.prod`**, à la racine du dépôt. Il contient les accès à la production, n'est **pas versionné** (voir `.env.prod.example`) et se demande au propriétaire s'il manque.
- **Où** :
  - sur le Mac mini, par SSH (`$PROD_SSH`), dans le dépôt Legifacile (`$PROD_REPO`), avec son environnement Python (`uv run`), donc le même encodeur que la production ;
  - la copie locale de Legifacile **n'est pas utilisée** : elle est figée à la mi-août, sans le découpage des décisions en parties.
- **Données disponibles sur le Mac mini** :
  - la base de production : ~9,6 M passages en 8 partitions avec index HNSW, colonne `partie` présente, pgvector 0.8.2. Accès en lecture seule via `$PROD_DB_CONTAINER` et `$PROD_PGOPTIONS` ;
  - `docs/mcp/evaluation/questions.yaml` : 57 questions étiquetées. Répartition : juri_texte_integral 31, textes_vigueur 13, juri_sommaires 4, jorf_normatif 4, sarde 3, dole 2. Aucune ne porte sur textes_hors_vigueur ni titres_textes ;
  - `scripts/db/eval_recherche.py` : rappel mesuré contre les étiquettes, via HNSW (ou `--via-mcp`) ;
  - `scripts/db/froid_decomposition.py --json` : latence décomposée en encodeur, SQL et reconstruction du texte, à froid (après une inactivité) et à chaud, par périmètre ;
  - les chiffres déjà connus, à citer sans les refaire : `docs/MDENSEON.md:404-445` (fidélité HNSW à `ef_search` 200) et `docs/mcp/EVALUATION.md` (latence, charge).
- **À construire** dans `tools/bench/mes00/` (versionné dans Crible ; **rien n'est ajouté au dépôt Legifacile**). Les scripts sont copiés dans un dossier temporaire du Mac mini et lancés avec l'environnement de Legifacile.
  1. **Un jeu d'environ 1 000 requêtes de latence**, généré sans données personnelles (titres d'articles et de sommaires tirés avec une graine fixe). Il est nécessaire pour un p99 : les 57 questions n'y suffisent pas.
  2. **Un script de fidélité HNSW contre recherche exacte** (recall@10 par corpus), sur un échantillon de requêtes. La recherche exacte se fait de préférence **hors production**, sur un export ponctuel des vecteurs (`scripts/db/exporter_vecteurs.py`, de nuit). À défaut, par un scan exact SQL de nuit, sur peu de requêtes.
- **Règles de sécurité, impératives (ADR-0032)** :
  - **Lecture seule.** Toute session Postgres ou tout script porte `PGOPTIONS=$PROD_PGOPTIONS`. Aucune modification de schéma, d'index, de réglage ni de données.
  - **Toujours `eval_recherche.py --sortie /tmp/…`.** Sans cette option, le script réécrit `docs/mcp/EVALUATION.md` dans le dépôt de production. Plus généralement, aucun fichier ne doit être modifié dans `$PROD_REPO` : `git status` doit y rester propre.
  - **Ne jamais redémarrer ni arrêter** Postgres, Docker, ni les services `$PROD_SERVICES`.
  - **Mesurer en dehors des heures d'usage**, le soir ou la nuit, et jamais pendant `$PROD_DAILY_WINDOW`. Les requêtes passent en série ; l'essai de charge reste court et borné.
  - **Aucune valeur de `.env.prod`** (hôte, chemins) dans un fichier versionné, un rapport, un ADR ou un commit. On n'y écrit que le nom des variables.
- **Particularité du pipeline** : `@dev` écrit les scripts. Avant la relecture, l'orchestrateur lance `@mesureur MES-00` pour exécuter la mesure et écrire le rapport, puis `@relecteur` relit les scripts et le rapport.
- **Livrables** :
  - `docs/mesures/MES-00-reference-pgvector.md`, qui contient :
    - l'état de la base mesurée : date, comptes par partition, commit Legifacile, version de pgvector, `shared_buffers`, charge de la machine ;
    - la qualité : recall@10 et rang du premier bon résultat, par corpus et par périmètre ;
    - la fidélité HNSW contre exact, par corpus ;
    - la latence p50/p95/p99 par poste (encodeur, SQL, reconstruction), à froid et à chaud, par corpus et par périmètre ;
  - les scripts `tools/bench/mes00/` et les données brutes en JSON, sans texte de requête réelle d'utilisateur ;
  - les cibles proposées dans `docs/suivi.md`, section « Cibles ».
- **Critères d'acceptation** :
  - le protocole est reproductible : les commandes exactes sont écrites avec les **noms** des variables de `.env.prod`, jamais leurs valeurs ;
  - le p99 repose sur au moins 1 000 requêtes ;
  - la mesure n'a rien écrit en production : `git status` propre dans `$PROD_REPO`, aucune requête d'écriture, aucun service redémarré ;
  - le propriétaire valide les cibles.
- **Invariants / mesures** : MES-00.
- **Hors périmètre** : modifier Legifacile ; utiliser ou rafraîchir la copie locale.
