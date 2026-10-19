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
  - `check_manifest(manifest, index_config) -> Result<(), CribleError>` : format, version, mode, compatibilité du modèle (`incompatible_model`), cohérence des comptes et de la liste des fichiers ;
  - `docs/format-lot-v1.md` : la spécification de référence, colonnes Parquet comprises, à partir de `04` (c'est elle que le code Python suivra).
- **Critères d'acceptation** :
  - l'exemple de manifeste du doc 04 est valide ;
  - des tests couvrent un mauvais `format_version`, un modèle incompatible (chaque champ), `replace_all` avec un fichier `deletions`, un fichier absent du manifeste.
- **Hors périmètre** : la lecture Parquet (P1).

### MESURER-REFERENCE-PGVECTOR
- **Objectif** : mesurer MES-00, la référence actuelle de Legifacile (rappel et latence). **Se fait avec le propriétaire**, dans le dépôt `legifacile-sandbox`.
- **Dépend de** : —
- **À lire** : `08-invariants-et-tests.md` (mesures, cibles) ; `docs/mesures/README.md` ; dans Legifacile : `docs/mcp/EVALUATION.md`.
- **Livrables** :
  - `docs/mesures/MES-00-reference-pgvector.md` : jeu de requêtes utilisé (le chemin, pas le contenu sensible), rappel contre la recherche exacte, latence p50/p95/p99 par corpus et par périmètre, machine et conditions ;
  - les cibles reportées dans `docs/suivi.md`, section « Cibles ».
- **Critères d'acceptation** :
  - le protocole est reproductible (commande ou script cité) ;
  - les chiffres bruts sont joints ;
  - les cibles sont validées par le propriétaire.
- **Invariants / mesures** : MES-00.
- **Hors périmètre** : modifier Legifacile.
