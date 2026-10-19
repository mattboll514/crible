# P2 — Encodeur et recette

**But** : Crible encode lui-même les requêtes, contrôle les longueurs, fait la recette d'import et accepte de petits lots en ligne. On constitue aussi l'échantillon réel de Legifacile, qui servira jusqu'à la fin.

**Jalon J2** :
- `crible search --query "…"` fonctionne sur l'échantillon Legifacile ;
- un lot corrompu (mauvais prompt) est refusé par la recette, et le lot correct est accepté ;
- un passage trop long est refusé ;
- MES-04 et MES-05 sont consignées ;
- le seuil de recette (Q-06) est validé par le propriétaire.

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

### CHARGER-MODELE
- **Objectif** : charger un modèle ONNX dans `crible-encode`.
- **Dépend de** : EXPORTER-MODELE-ONNX
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
- **À lire** : `03-recherche.md` (pipeline, étape 2) ; `05-api.md` (limites).
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
- **À lire** : `04-donnees-et-ingestion.md` (mode en ligne) ; `05-api.md` (lot en ligne).
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
