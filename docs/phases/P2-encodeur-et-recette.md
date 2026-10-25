# P2 — Encodeur et recette

**But** : Crible encode lui-même les requêtes, contrôle les longueurs, fait la recette d'import et accepte de petits lots en ligne. On constitue aussi l'échantillon réel de Legifacile, qui servira jusqu'à la fin.

**Jalon J2** :
- `crible search --query "…"` fonctionne sur l'échantillon Legifacile ;
- un lot corrompu (mauvais prompt) est refusé par la recette, et le lot correct est accepté ;
- un passage trop long est refusé ;
- MES-04 et MES-05 sont consignées ;
- le seuil de recette (Q-06) est validé par le propriétaire ;
- un run CI distant vert sur la branche de phase, job `rust-onnx` compris (ADR-0057).

---

### EXPORTER-MODELE-ONNX
- **Objectif** : produire les fichiers du modèle mDenseOn, prêts pour Crible.
- **Dépend de** : —
- **À lire** : `04-donnees-et-ingestion.md` (le modèle).
- **Livrables** :
  - `tools/export_onnx.py` (uv) : export fp32 et quantification INT8 dynamique ;
  - `tokenizer.json` ;
  - `model.json` avec les hash blake3, la révision HF épinglée, `max_tokens` réellement supporté, les prompts, le pooling CLS et la normalisation ;
  - `tools/README.md` : la procédure, et la sortie dans `data/models/mdenseon/`.
- **Critères d'acceptation** :
  - le script est reproductible : même révision, mêmes hash fp32 ;
  - sur 100 textes, le cosinus entre PyTorch et ONNX fp32 vaut au moins 0,9999 (chiffre dans le rapport de la tâche).
- **Hors périmètre** : le code Rust.

### CONSTITUER-ECHANTILLON-LEGIFACILE
- **Objectif** : un échantillon réel, représentatif des six structures. **Se fait avec le propriétaire**, dans `legifacile-sandbox`.
- **Dépend de** : ECRIRE-LOTS-PYTHON
- **À lire** : `04-donnees-et-ingestion.md` (producteurs) ; `02-architecture.md` (découpage validé).
- **Livrables** :
  - un script dans le dépôt Legifacile qui sort, avec `BatchWriter`, ~50 000 passages par structure :
    - les vecteurs existants de pgvector, en fp16 ;
    - les textes recomposés avec le code actuel et vérifiés contre `src_hash` ;
  - un jeu de ~300 requêtes réelles (jsonl, avec `expected` quand il est connu) ;
  - les configs d'index provisoires dans `data/legifacile-sample/`.
- **Critères d'acceptation** :
  - 100 % des textes recomposés correspondent à `src_hash`, ou les écarts sont listés et expliqués ;
  - les lots sont lus par `LIRE-LOT-PARQUET`.
- **Hors périmètre** : les schémas définitifs (P9).

### REEXPORTER-MODELE-TRANSFORMERS-5
- **Objectif** : ré-exporter l'ONNX de mDenseOn sous `transformers==5.14.1`, la version qui a produit les vecteurs de production de Legifacile, et prouver la fidélité contre ces vecteurs.
- **Dépend de** : EXPORTER-MODELE-ONNX, CONSTITUER-ECHANTILLON-LEGIFACILE
- **À lire** : `04-donnees-et-ingestion.md` (le modèle) ; `08-invariants-et-tests.md` (INV-12, MES-04) ; `adr/0050-reference-mdenseon-sous-transformers-5-14-1-et-reexport-onnx.md` ; `data/legifacile-sample/README.md` (« Limite connue »).
- **Livrables** :
  - `tools/pyproject.toml` et `tools/uv.lock` : `optimum` retiré du groupe `export`, `transformers==5.14.1` épinglé, `onnx` et `onnxscript` ajoutés si l'exportateur les exige ;
  - `tools/export_onnx.py` adapté : export fp32 par `torch.onnx.export`, quantification INT8 dynamique par `onnxruntime.quantization` ; `model.json` porte le champ `export` (versions de `transformers`, `torch`, `onnxruntime`) ;
  - `model.json` réécrit de façon atomique, `max_tokens` reverifié à 8192 ;
  - `tools/README.md` : la nouvelle procédure d'export, la disparition d'`optimum`, et la façon de reproduire les deux mesures ;
  - les chiffres des deux critères, dans le rapport de la tâche.
- **Critères d'acceptation** :
  - (i) sur les 100 textes de parité, cosinus PyTorch (transformers 5.14.1) contre ONNX fp32 ≥ 0,9999 ;
  - (ii) sur un tirage déterministe de 100 passages par structure, soit 600 passages de `data/legifacile-sample/`, cosinus ONNX fp32 avec prompt de document contre les vecteurs de production ≥ 0,999 ; minimum, p1 et médiane dans le rapport ;
  - `max_tokens` = 8192 vérifié sur le graphe ré-exporté ;
  - l'export est reproductible : même révision, mêmes hash fp32 ;
  - `tools/export_onnx.py` échoue proprement, sans `unwrap()` ni trace non interceptée, si le moteur d'export refuse le modèle.
- **Invariants / mesures** : INV-11 (aucune troncature silencieuse), INV-12 (seuil de recette) ; prépare MES-04.
- **Hors périmètre** : le code Rust ; les seuils de recette ; le modèle INT8 au-delà de sa quantification ; toute modification de `docs/`.

**Clause d'arrêt** : si `torch.onnx.export` ne produit pas un graphe exécutable pour ce modèle (opérateur non supporté, sortie instable, exécution à 8 192 tokens impossible), la tâche **s'arrête** et remonte une `QUESTION-CONCEPTION` ; elle ne réutilise pas l'ONNX exporté sous transformers 4.57.6, qui ne retrouve les vecteurs de production qu'à 0,983–0,990.

### CHARGER-MODELE
- **Objectif** : charger un modèle ONNX dans `crible-encode`.
- **Dépend de** : REEXPORTER-MODELE-TRANSFORMERS-5
- **À lire** : `04-donnees-et-ingestion.md` (le modèle) ; `07-stack-et-deploiement.md` (environnement de développement, ONNX Runtime).
- **Livrables** :
  - `crible-encode::Model` : lecture de `model.json`, vérification des hash, `ort` en `load-dynamic`, tokenizer, pooling CLS, normalisation L2, prompts ;
  - un registre qui charge un modèle **une seule fois** par empreinte ;
  - la doc d'installation d'ONNX Runtime sur Mac (`ORT_DYLIB_PATH`), dans `tools/README.md`.
- **Critères d'acceptation** :
  - un fichier modifié (hash faux) est refusé ;
  - le vecteur produit pour 20 textes de référence est égal, à 1e-4 près, à celui de `tools/export_onnx.py` en fp32.

### ENCODER-REQUETES
- **Objectif** : l'encodage rapide des requêtes.
- **Dépend de** : CHARGER-MODELE
- **À lire** : `03-recherche.md` (pipeline, étape 2, « Réglages de précision ») ; `04-donnees-et-ingestion.md` (usage des précisions) ; `05-api.md` (limites) ; `07-stack-et-deploiement.md` (`[encoder]`, `[limits]`) ; ADR-0051 (cache des requêtes, sessions INT8 et fp32).
- **Livrables** :
  - `encode_query(text)` : INT8 par défaut, fp32 en option, avec le prompt `query: ` ;
  - la troncature à `max_tokens`, signalée par `truncated: true` ;
  - un cache LRU en RAM (taille configurable), jamais persisté ;
  - un repli vers fp32 si le modèle INT8 est absent.
- **Critères d'acceptation** :
  - le cache sert la deuxième requête identique (test) ;
  - la troncature est signalée ;
  - `crible search --query` fonctionne de bout en bout sur `synth-small`, avec un modèle factice de test.

### MESURER-PARITE-ENCODEUR
- **Objectif** : MES-04 et MES-05.
- **Dépend de** : ENCODER-REQUETES, CONSTITUER-ECHANTILLON-LEGIFACILE
- **À lire** : `08-invariants-et-tests.md` ; `docs/mesures/README.md`.
- **Livrables** :
  - `docs/mesures/MES-04-parite-encodeur.md` : cosinus INT8 et fp32 contre PyTorch sur les requêtes réelles ; rappel@10 des requêtes encodées en INT8 contre fp32 sur l'échantillon (recherche exacte) ;
  - `docs/mesures/MES-05-latence-encodage.md` : p50 et p95 sur le CPU du Mac, en natif.
- **Critères d'acceptation** :
  - les protocoles sont reproductibles ;
  - une recommandation est faite (INT8 ou fp32 pour les requêtes) et une valeur de seuil de recette est proposée (Q-06).
- **Invariants / mesures** : MES-04, MES-05.

### CONTROLER-LONGUEURS
- **Objectif** : aucune troncature silencieuse à l'attache.
- **Dépend de** : CHARGER-MODELE, ATTACHER-LOT
- **À lire** : `04-donnees-et-ingestion.md` (contrôle 8).
- **Livrables** : comptage des tokens (prompt `document: ` compris) de chaque passage pendant la validation du lot, en parallèle ; rejet avec la liste des clés fautives (les 20 premières).
- **Critères d'acceptation** :
  - un lot avec un passage trop long est rejeté sans aucun effet (**INV-11**) ;
  - le coût est mesuré sur un lot de 10 000 passages et noté dans le rapport de la tâche.
- **Invariants** : INV-11.

### RECETTE-IMPORT
- **Objectif** : la recette à l'attache.
- **Dépend de** : CONTROLER-LONGUEURS
- **À lire** : `04-donnees-et-ingestion.md` (contrôle 9).
- **Livrables** :
  - un échantillon déterministe (graine dérivée du `batch_id`), ré-encodé en fp32 ;
  - la distribution des cosinus (minimum, p1, p50) dans le rapport ;
  - le refus avec `acceptance_failed` ;
  - pas de recette pour un modèle `external`.
- **Critères d'acceptation** :
  - le lot correct de l'échantillon est accepté ;
  - un lot dont les vecteurs ont été encodés sans le prompt est refusé (**INV-12**) ;
  - un lot dont le manifeste annonce un autre modèle est refusé avec `incompatible_model`.
- **Invariants** : INV-12.

### ENCODER-DOCUMENTS-EN-LIGNE
- **Objectif** : accepter des documents sans vecteurs.
- **Dépend de** : RECETTE-IMPORT
- **À lire** : `04-donnees-et-ingestion.md` (mode en ligne) ; `05-api.md` (lot en ligne) ; ADR-0055 (fichier JSONL, identité et manifeste du lot interne, `doc_hash`, emplacement du lot, `upsert` seul).
- **Livrables** :
  - `crible-store::online` : à partir d'une liste de documents (format JSON de l'API), encodage fp32 des passages sans vecteur ;
  - la construction d'un lot interne, qui suit le même chemin d'attache ;
  - la commande `crible ingest --data … --index … documents.jsonl`.
- **Critères d'acceptation** :
  - 100 documents ingérés en ligne sont retrouvés par `crible search --query` ;
  - le résultat est identique à celui d'un lot équivalent produit en Python avec les mêmes vecteurs.
- **Hors périmètre** : la route HTTP (P6).

### CLI-RECHERCHE-TEXTE
- **Objectif** : rendre la démo J2 possible.
- **Dépend de** : ENCODER-REQUETES, RECETTE-IMPORT
- **Livrables** :
  - `crible search --query "…"`, avec l'affichage clé, score, champs et un extrait de texte ;
  - `docs/jalons/J2-demo.md`, avec les commandes exactes sur l'échantillon Legifacile.
- **Critères d'acceptation** : le scénario J2 passe tel qu'écrit.

### AJOUTER-ORT-CI
- **Objectif** : rendre la CI capable d'exécuter les 19 tests ONNX Runtime, sur le modèle factice et sur le vrai modèle mDenseOn, et rendre un saut de test silencieux impossible (ADR-0057).
- **Dépend de** : NETTOYER-P2, CLI-RECHERCHE-TEXTE, RECETTE-IMPORT, CONTROLER-LONGUEURS, ENCODER-DOCUMENTS-EN-LIGNE
- **À lire** : `adr/0057-onnx-runtime-et-vrai-modele-mdenseon-en-integration-continue.md` ; `07-stack-et-deploiement.md` (« CI (GitHub Actions) », « Côté Python », « Dépendances Rust » pour `load-dynamic`) ; `08-invariants-et-tests.md` (« Sauts de test », INV-11, INV-12) ; `.github/workflows/ci.yml` ; `tools/README.md` (« Révision et hash mesurés ») ; `docs/jalons/J2-rapport.md`, §7.
- **Livrables** :
  - `tools/pyproject.toml` : groupe `uv` `ci` contenant `onnxruntime==1.31.0` en version exacte, **sans** `torch`, installé par `uv sync --group ci` ; `tokenizers` s'y ajoute seulement s'il est la seule dépendance qui manque aux tests de `tools/bench/mes04/` ;
  - `.github/workflows/ci.yml` : job `rust-onnx`, sur `ubuntu-latest`, déclenché sur push et pull request, avec un `timeout-minutes` explicite fixé large puis resserré après le premier run à froid ; `ORT_DYLIB_PATH` désigne la bibliothèque de la roue (`onnxruntime/capi/libonnxruntime.so.1.31.0`), jamais un chemin de poste de développement ; la feature `download-binaries` d'`ort` n'est pas utilisée ;
  - le manifeste versionné de l'artefact (`ci/model-artifact.toml`) : dépôt Hugging Face, sha du commit HF épinglé, noms des fichiers, hash blake3 **complets** de chaque fichier téléchargé, `model.json` compris ;
  - le téléchargement et la vérification : URL explicite `https://huggingface.co/mattboll/mdenseon-onnx/resolve/<sha du commit HF>/<fichier>`, sans jeton, cache GitHub Actions indexé sur les hash du manifeste et **jamais** source de confiance (le contenu restauré est vérifié comme un téléchargement neuf), fichiers placés dans `data/models/mdenseon/` ;
  - le dossier d'envoi de l'artefact préparé dans `data/` (gitignoré) : `model.onnx`, `model.int8.onnx`, `tokenizer.json` et `model.json`, copies exactes de `data/models/mdenseon/`, plus `LICENSE` (texte d'Apache-2.0) et `README.md` (carte du modèle) ;
  - la correction de `crates/crible/tests/recette_import.rs` : le `return` silencieux (vers les lignes 1333-1352) devient un échec dont le message donne la commande qui produit `data/legifacile-sample/` et `data/models/mdenseon/` ;
  - les cinq corrections ci-dessous ;
  - les durées du job `rust-onnx`, relevées **à froid** (sans cache) et **à cache chaud**, remises à l'orchestrateur pour `docs/suivi.md`.
- **Critères d'acceptation** :
  - sur un run **distant**, le job `rust-onnx` exécute la commande `cargo test --locked --workspace -- --include-ignored --skip inv_12_the_legifacile_sample_passes_the_acceptance_check` et les 19 tests ignorés y passent ;
  - le test de l'échantillon Legifacile est le **seul** exclu, par son nom, et son exclusion est justifiée ;
  - le job Rust actuel reste inchangé et n'installe ni ONNX Runtime ni le groupe `ci` ;
  - le manifeste fige le dépôt, le sha du commit HF et les blake3 complets, `model.json` compris ;
  - un hash faux fait échouer le job en donnant le hash attendu et le hash obtenu ;
  - un fichier absent, une URL en erreur (code non 2xx) ou une révision introuvable font échouer le job en donnant l'URL et le nom du fichier ;
  - un cache restauré dont le contenu diverge échoue aussi, en donnant la clé de cache à purger ;
  - aucun jeton Hugging Face n'apparaît dans le workflow ni dans le dépôt ; aucun `continue-on-error`, aucun `|| true`, aucun saut implicite ;
  - un test `#[ignore]` lancé sans son prérequis échoue avec la commande qui le produit, et un test le prouve sur `recette_import.rs` ;
  - les cinq corrections ci-dessous sont faites, chacune avec sa preuve (test, sortie ou diff) ;
  - les durées à froid et à cache chaud sont relevées et remises à l'orchestrateur.
- **Note du 2026-10-25, après acceptation d'ADR-0062** : les critères ci-dessus décrivent l'état à la clôture d'`AJOUTER-ORT-CI` et ne sont pas réécrits. Depuis `ADR-0062`, la CI exclut **deux** tests de l'échantillon Legifacile, nommés un par un : `inv_12_the_legifacile_sample_passes_the_acceptance_check` et `inv_09_legifacile_sample_queries_return_the_same_results_after_compaction`. La phrase « les 19 tests ignorés » et celle du « seul exclu » valent donc pour la date de clôture, pas après ; `ADR-0062` fait foi pour ces décomptes.
- **Invariants / mesures** : INV-11 (aucune troncature silencieuse : les tests `inv_11_*` passent en CI avec ONNX Runtime), INV-12 (recette d'import : les tests `inv_12_*` passent en CI, sauf l'échantillon Legifacile, local).
- **Étapes de publication de l'artefact** :
  1. la tâche prépare dans `data/` (gitignoré) le dossier d'envoi décrit dans les livrables ;
  2. le **propriétaire** crée le compte et le dépôt Hugging Face public `mattboll/mdenseon-onnx` (créé le 2026-10-24, encore vide), puis fait l'envoi lui-même avec son jeton (`hf upload`) ; **aucun agent ne publie à sa place** ;
  3. la tâche relève le sha du commit HF obtenu, calcule les blake3 complets des fichiers téléchargés (`model.json` compris) et écrit le manifeste ;
  4. la tâche relance le job sur le sha épinglé et note les durées.
- **Les cinq corrections reprises par décision du propriétaire** :
  - figer les scores de la démo J2 dans `docs/jalons/J2-demo.md`, en fp32 par défaut (ADR-0052), avec les valeurs mesurées (top-1 `JORFDOLE000047079143#0`, 0,623664 en fp32) ;
  - citer `tools/bench/mes04/` dans le tableau des outils de `tools/README.md` ;
  - faire tourner dans le job `tools` les tests de `tools/bench/mes04/` qui demandent `tokenizers` (`importorskip`), par le groupe `ci` si `tokenizers` y suffit et **sans torch** ; à défaut, rendre le saut explicite et le justifier ;
  - écrire `precision_effective` aussi dans le JSON des vecteurs de `crates/crible-encode/examples/measure_queries.rs` ;
  - `tools/export_onnx.py` : `min_cosine_required` est écrit deux fois (`run_parity` puis `export`), et le paramètre `revision` d'`export_fp32` n'est pas lu ; le consommer ou l'assumer explicitement.
- **Clause d'arrêt** : si le dépôt Hugging Face ne peut pas être créé, si l'envoi ne peut pas être fait, si un hash ne correspond pas, ou si le graphe fp32 ne s'exécute pas sous Linux, la tâche **s'arrête** et remonte une `QUESTION-CONCEPTION`. Elle ne remplace pas le modèle par un autre, ne ré-exporte pas le modèle, ne déplace pas l'artefact vers un autre hébergement sans ADR, et ne désactive pas un test pour faire passer le job.
- **Hors périmètre** : la route HTTP et le serveur (P6) ; un ré-export du modèle ; les campagnes MES-04 et MES-05 ; la publication de l'image de déploiement.

**J2 n'est validé qu'après cette tâche et un run CI distant vert sur `phase/P2-encodeur-et-recette`.** Le jalon reste `PRET-POUR-VALIDATION` jusque-là.
