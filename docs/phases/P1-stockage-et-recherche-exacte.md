# P1 — Stockage, lots et recherche exacte

**But** : écrire des lots en Python, les attacher dans un index, filtrer et chercher en **exact** à partir d'un vecteur fourni. Pas encore de modèle, de binaire ni de serveur : tout passe par la CLI et les tests.

**Jalon J1**, sur `synth-small` :
- `crible index create`, puis `crible attach` (trois lots : création, modification, suppression), puis `crible search --vector-file … --filter …` ;
- les résultats sont identiques à la référence numpy sur 200 requêtes ;
- INV-03 (lecteurs concurrents), INV-06, INV-07 et INV-10 sont verts ;
- `crible inspect` affiche l'état.

---

### ECRIRE-LOTS-PYTHON
- **Objectif** : le paquet Python `crible` sait écrire un lot conforme.
- **Dépend de** : DEFINIR-FORMAT-LOT
- **À lire** : `docs/format-lot-v1.md` ; `04-donnees-et-ingestion.md` (contrôles 5 à 7).
- **Livrables** :
  - `clients/python/` (projet uv, paquet `crible`) ;
  - `BatchWriter(index_config, out_dir, batch_id, mode, model, chunker, template)` avec les méthodes `add_document(doc_id, doc_hash, fields, passages)`, `delete(doc_id)` et `close()` ;
  - l'écriture du Parquet en plusieurs parties (≤ 1 M passages par partie), les checksums blake3 et le manifeste ;
  - les contrôles locaux : schéma, `ord` contigus, vecteurs finis, normes, comptes ;
  - `tests/` avec pytest.
- **Critères d'acceptation** :
  - un lot écrit se relit avec pyarrow, et ses colonnes ont les types de la spec ;
  - le vecteur est un `fixed_size_binary` de `dims × 2` octets en fp16 LE (vérifié par un test d'aller-retour numpy) ;
  - chaque contrôle local a son test d'échec.
- **Hors périmètre** : le client HTTP (P6).

### GENERER-JEU-SYNTHETIQUE
- **Objectif** : des données de test déterministes et leur référence exacte.
- **Dépend de** : ECRIRE-LOTS-PYTHON
- **À lire** : `08-invariants-et-tests.md` (jeux de données).
- **Livrables** :
  - `tools/gen_synth.py`, qui produit `synth-small` en trois lots :
    - `L1` : création, 10 000 passages, 2 000 documents ;
    - `L2` : 300 documents modifiés et 100 nouveaux ;
    - `L3` : 150 suppressions et 50 modifications de métadonnées seules ;
  - pour l'option `--large` : 8,6 M passages aléatoires, en plusieurs parties ;
  - `tools/reference_topk.py` : top-k exact numpy (cosinus) pour un jeu de requêtes et des filtres, avec la même grammaire de filtre (sous-ensemble suffisant) ;
  - `data/synth-small/queries.jsonl` (200 requêtes : vecteurs et filtres variés) et `expected.jsonl` ;
  - une somme de contrôle du jeu, consignée dans `tools/README.md`.
- **Critères d'acceptation** :
  - deux générations avec la même graine donnent des fichiers identiques au bit près ;
  - le jeu contient tous les types de champs, des nulls, des champs de document et de passage, et des vecteurs en grappes.
- **Hors périmètre** : des textes réalistes (du texte aléatoire suffit).

### STOCKER-ENREGISTREMENTS-SQLITE
- **Objectif** : la base de référence d'une époque.
- **Dépend de** : DEFINIR-CONFIG-INDEX
- **À lire** : `02-architecture.md` (organisation sur disque, tables SQLite) ; `docs/adr/0019-champs-en-json-canonique.md` (ADR-0019).
- **Livrables** :
  - `crible-store::records` : création du schéma (`meta`, `batches`, `segments`, `documents`, `passages`), mode WAL ;
  - texte compressé en zstd, champs en JSON canonique ;
  - un pool de connexions en lecture ;
  - les requêtes nécessaires : passages d'un document, passages vivants, documents par id, lecture par iid.
- **Critères d'acceptation** :
  - tests en dossier temporaire : aller-retour texte et champs ;
  - le JSON canonique est stable (clés triées) ;
  - `format_version` est écrit et vérifié à l'ouverture (refus d'une version inconnue, INV-15).
- **Hors périmètre** : les générations (tâche dédiée).

### ECRIRE-SEGMENTS-VECTEURS
- **Objectif** : les fichiers de vecteurs immuables d'un segment.
- **Dépend de** : INITIALISER-WORKSPACE-RUST
- **À lire** : `02-architecture.md` (organisation sur disque).
- **Livrables** :
  - `crible-store::segment` : écriture de `vectors.f16` ou `vectors.f32` (little-endian explicite) et de `segment.json` (version, `iid_start`, `count`, `dims`, checksums blake3) ;
  - un lecteur mmap (`memmap2`) avec accès par iid ;
  - l'écriture atomique : dossier `tmp-<id>`, fsync, rename.
- **Critères d'acceptation** :
  - test d'aller-retour sur des valeurs connues ;
  - test qui lit des octets attendus écrits à la main (INV-15) ;
  - un checksum corrompu est détecté à l'ouverture.
- **Invariants** : INV-15.

### GERER-GENERATIONS
- **Objectif** : le protocole de commit et la reprise au démarrage.
- **Dépend de** : STOCKER-ENREGISTREMENTS-SQLITE, ECRIRE-SEGMENTS-VECTEURS
- **À lire** : `02-architecture.md` (générations et point de commit, concurrence) ; `docs/adr/0016-point-de-commit-sqlite.md` (ADR-0016).
- **Livrables** :
  - `crible-store::generation` : `CURRENT_EPOCH`, une transaction de commit qui incrémente `meta.generation`, les manifestes dérivés ;
  - la reprise : nettoyage de `tmp-*` et des segments orphelins, réécriture du manifeste manquant ;
  - le type `Snapshot` (`Arc`), publié par `ArcSwap` ;
  - des points d'injection de panne derrière une feature `failpoints`.
- **Critères d'acceptation** :
  - test : une panne simulée avant ou après le commit SQLite donne, au redémarrage, l'ancienne ou la nouvelle génération complète, jamais un mélange (INV-04) ;
  - aucun segment orphelin ne survit à la reprise.
- **Invariants** : INV-03, INV-04.

### LIRE-LOT-PARQUET
- **Objectif** : lire et valider un lot côté Rust.
- **Dépend de** : DEFINIR-FORMAT-LOT, ECRIRE-LOTS-PYTHON
- **À lire** : `docs/format-lot-v1.md` ; `04-donnees-et-ingestion.md` (contrôles 1 à 7).
- **Livrables** :
  - `crible-store::batch_reader` (arrow-rs, parquet) : lecture du manifeste, vérification des checksums, conformité du schéma, lecture en flux par parties ;
  - les contrôles d'`ord`, de finitude et de norme.
- **Critères d'acceptation** :
  - les lots `synth-small` sont lus ;
  - chaque contrôle a son test d'échec, avec le bon `ErrorCode` ;
  - la mémoire reste bornée : la lecture se fait partie par partie, jamais en chargeant tout le lot.
- **Hors périmètre** : les longueurs en tokens et la recette (P2).

### ATTACHER-LOT
- **Objectif** : intégrer un lot selon la sémantique d'attache.
- **Dépend de** : LIRE-LOT-PARQUET, GERER-GENERATIONS
- **À lire** : `04-donnees-et-ingestion.md` (sémantique d'attache) ; `02-architecture.md` (générations, suppressions).
- **Livrables** :
  - `crible-store::attach` : modes `upsert` et `replace_all` ;
  - les règles du tableau du doc 04 (nouveau, hash identique, hash différent, suppression, conflit) ;
  - l'attribution des iid et l'écriture du segment ;
  - les pierres tombales (`dead_since_gen`), l'idempotence par `batch_id` et le `batch_conflict` ;
  - le rapport d'attache.
- **Critères d'acceptation** :
  - L1, L2 puis L3 donnent les comptes attendus ;
  - rejouer L2 ne change rien (INV-10) ;
  - un `doc_id` à la fois dans `documents` et `deletions` fait rejeter le lot, sans aucune trace (INV-03) ;
  - avec `replace_all`, tous les documents absents sont supprimés.
- **Invariants** : INV-03, INV-10.
- **Hors périmètre** : le lexical (P5), les codes binaires (P4).

### CONSTRUIRE-BITMAPS
- **Objectif** : les structures de filtrage en RAM.
- **Dépend de** : ATTACHER-LOT
- **À lire** : `03-recherche.md` (filtres → pool) ; ADR-0018 et ADR-0019 (`docs/adr/`).
- **Livrables** :
  - `crible-store::filters_index` : une bitmap roaring par valeur de `keyword`, les bitmaps `bool`, une bitmap « présent » par champ ;
  - les colonnes de valeurs (`i64`, `f64`, `i32` pour les jours), la bitmap des vivants, l'héritage des champs de document par les passages ;
  - la construction depuis SQLite, intégrée au `Snapshot` ;
  - la mise à jour en copie sur écriture après une attache.
- **Critères d'acceptation** :
  - test : après L1, L2 et L3, les bitmaps sont égales à une reconstruction complète depuis SQLite ;
  - la taille mémoire est rapportée dans `crible inspect`.
- **Invariants / mesures** : MES-13 (premier chiffre).

### EVALUER-FILTRES
- **Objectif** : du filtre JSON validé au pool.
- **Dépend de** : CONSTRUIRE-BITMAPS
- **À lire** : `03-recherche.md` (filtres → pool) ; `05-api.md` (grammaire).
- **Livrables** :
  - `crible-search::pool` : évaluation de l'AST (ET avec les bitmaps d'abord, puis les intervalles restreints au domaine ; OU ; NON = vivants privés de l'enfant), intersection avec les vivants ;
  - le résultat contient la bitmap du pool et sa taille.
- **Critères d'acceptation** :
  - **INV-07** en proptest : filtres aléatoires (≥ 1 000 cas), comparés à une évaluation SQL des mêmes filtres via `json_extract` sur `records.sqlite` ;
  - tous les opérateurs et les nulls (`exists`) sont couverts.
- **Invariants / mesures** : INV-07, MES-14 (premier chiffre sur `synth-small`).

### RECHERCHER-DENSE-EXACT
- **Objectif** : top-k exact en cosinus sur le pool, à partir d'un vecteur fourni.
- **Dépend de** : EVALUER-FILTRES
- **À lire** : `03-recherche.md` (branche dense, stratégie EXACT).
- **Livrables** :
  - `crible-search::dense::exact` : produit scalaire fp16 → f32 (implémentation scalaire simple ; le SIMD vient en P4), parallélisé avec rayon par tranches, puis fusion des tas ;
  - la lecture des payloads (champs, texte) pour les k résultats.
- **Critères d'acceptation** :
  - les 200 requêtes de `synth-small` donnent les mêmes clés que `expected.jsonl`, dans le même ordre (égalité à 1e-5 près sur les scores, et départage stable par clé) ;
  - **INV-06**.
- **Invariants** : INV-06.

### CLI-PREMIERES-COMMANDES
- **Objectif** : piloter tout ça depuis le binaire.
- **Dépend de** : RECHERCHER-DENSE-EXACT
- **À lire** : `07-stack-et-deploiement.md` (organisation du dépôt).
- **Livrables** :
  - `crible index create --data <dir> --config <json>` ;
  - `crible attach --data <dir> --index <i> <dossier-de-lot>` ;
  - `crible search --data <dir> --index <i> --vector-file <npy|json> [--filter <json>] [--k 10]` ;
  - `crible inspect --data <dir> [--index <i>]` (génération, époque, comptes, segments, mémoire des bitmaps) ;
  - un test d'intégration de bout en bout qui enchaîne ces commandes ;
  - une section « Démo J1 » dans `docs/jalons/J1-demo.md`, avec les commandes exactes.
- **Critères d'acceptation** :
  - le scénario de démo J1 passe tel qu'écrit ;
  - les erreurs sont lisibles (code et message).
