# P9 — Migration de Legifacile (validation de la v1)

**But** : servir la recherche de Legifacile avec Crible, mesurer contre pgvector, puis basculer. Le retour arrière reste possible pendant toute la bascule. La plupart des tâches touchent **le dépôt Legifacile** et se font **avec le propriétaire**.

**Jalon J9 = v1**, d'après les critères de `01-vision-et-perimetre.md` :
- le rappel est au moins égal à pgvector (MES-00) ;
- le p95 est au plus égal à pgvector ;
- la tâche quotidienne est stable 14 jours ;
- le retour arrière a été testé ;
- un bilan est écrit.

---

### DEFINIR-INDEX-LEGIFACILE
- **Objectif** : les six configs d'index définitives.
- **Dépend de** : J8 validé
- **À lire** : `02-architecture.md` (découpage validé) ; `04-donnees-et-ingestion.md` (configuration). Dans Legifacile : les vues `vec.src_*`, `scripts/db/sync_vecteurs.py` et `serveur_mcp/outils.py` (filtres exposés).
- **Livrables** :
  - `articles`, `jorf`, `decisions`, `titres`, `sarde` et `dole`, avec leurs champs, leurs types et leur niveau ;
  - `cluster_by` pour chaque index ;
  - pour chaque index, le gabarit à garder ou à changer ;
  - la réponse à Q-03 (champs multivalués) ;
  - un tableau de correspondance entre les filtres actuels du MCP et les filtres Crible (`fonds`, `etat` → dates, `annee`, `cle_domaine`, `partie_exclue`, `dedupliquer`).
- **Critères d'acceptation** :
  - chaque filtre actuel du MCP a son équivalent ;
  - chaque config est validée par `crible-core` ;
  - le propriétaire valide les six configs.

### ADAPTER-PRODUCTEUR-LEGIFACILE
- **Objectif** : le producteur de lots de Legifacile.
- **Dépend de** : DEFINIR-INDEX-LEGIFACILE
- **À lire** : `04-donnees-et-ingestion.md` (producteurs, format de lot).
- **Livrables** : dans Legifacile, un module qui :
  - réutilise le compositeur et le découpeur actuels ;
  - écrit des lots avec `crible.BatchWriter` ;
  - gère deux modes : la migration (vecteurs pgvector existants, textes vérifiés contre `src_hash`) et le quotidien (encodage en natif sur MPS) ;
  - calcule un `doc_hash` cohérent avec `src_hash` ;
  - renseigne la déclaration `chunker` et `template` du manifeste.
- **Critères d'acceptation** :
  - les lots de migration des six index passent la validation et la recette ;
  - les écarts de texte sont comptés et expliqués.

### IMPORTER-BASE-COMPLETE
- **Objectif** : charger les 8,58 M passages.
- **Dépend de** : ADAPTER-PRODUCTEUR-LEGIFACILE
- **Livrables** :
  - des lots `replace_all` attachés dans une instance Crible sur le Mac mini, en conteneur ;
  - la durée et l'espace disque mesurés ;
  - le résultat de `crible inspect` ;
  - un instantané de départ.
- **Critères d'acceptation** :
  - les comptes par index sont égaux à ceux de pgvector, ou les écarts sont expliqués ;
  - MES-07 et MES-09 sont refaites à l'échelle réelle.
- **Invariants / mesures** : MES-07, MES-09.
- **À relever** : mesurer la durée de la reconstruction lexicale à 8,58 M passages, par `crible rebuild-lexical` ou à la première reconstruction, et la comparer à la cible de 60 s d'ADR-0047. MES-06 (§ 8.3.1) l'extrapole à 325 s, de 49 à 762 s selon la composition du corpus, et ADR-0088 le rappelle en « À surveiller ». Le propriétaire a décidé le 2026-10-29 d'attendre cette première mesure réelle avant tout ADR : si la cible n'est pas tenue, faire rédiger un ADR qui relève la cible pour ce cas de reprise, ou qui parallélise l'écriture (`WRITER_THREADS` vaut 1 par ADR-0087).

### CONSTITUER-JEU-DE-REFERENCE
- **Objectif** : le jeu de requêtes qui tranchera la v1.
- **Dépend de** : DEFINIR-INDEX-LEGIFACILE
- **Livrables** :
  - un jeu JSONL de requêtes réelles, avec leurs périmètres et `expected` quand il est connu (en repartant de `docs/mcp/EVALUATION.md` de Legifacile et du jeu de MES-00) ;
  - les cas « article 1240 » et leurs variantes ;
  - les requêtes filtrées les plus fréquentes.
- **Critères d'acceptation** :
  - le jeu est validé par le propriétaire ;
  - il est le même pour pgvector et pour Crible.

### COMPARER-AVEC-PGVECTOR
- **Objectif** : la mesure décisive.
- **Dépend de** : IMPORTER-BASE-COMPLETE, CONSTITUER-JEU-DE-REFERENCE
- **Livrables** : `crible eval` (modes dense et hybride) et la mesure pgvector, sur le même jeu, la même machine et dans des conditions comparables. Le rapport donne :
  - le rappel par périmètre ;
  - p50, p95 et p99 ;
  - les requêtes gagnantes et perdantes ;
  - la latence p50/p95 des requêtes **avec filtre d'intervalle** sur l'index réel de Legifacile, par rejeu de MES-14 sur cet index (mêmes cas : domaine plein, domaine réduit par une clause en bitmap), comparée à la cible de latence v1.
- **Critères d'acceptation** :
  - les critères de rappel et de p95 de la v1 sont atteints ;
  - sinon, une liste d'actions est soumise au propriétaire ;
  - un ADR d'index de plage n'est ouvert que si cette p95 **de bout en bout** dépasse la cible (ADR-0046, qui prolonge ADR-0018).

### BRANCHER-MCP
- **Objectif** : le MCP de Legifacile interroge Crible.
- **Dépend de** : COMPARER-AVEC-PGVECTOR, CLIENT-PYTHON-HTTP
- **Livrables** : dans `serveur_mcp`, un adaptateur `crible.Client` qui fait les conversions suivantes :
  - périmètres → `POST /v1/search` multi-index ;
  - `dedupliquer` → `group_by` et `dedup` ;
  - `partie_exclue` → filtre `partie: {nin: …}` ;
  - `etat` → filtres de dates.

  L'encodage local des requêtes est retiré du MCP. Un interrupteur de config `backend = pgvector | crible` permet le retour arrière.
- **Critères d'acceptation** :
  - les outils MCP renvoient le même format qu'avant ;
  - le texte affiché vient de Crible, sans recomposition ;
  - le retour arrière par l'interrupteur est testé.

### MODE-OMBRE
- **Objectif** : comparer en conditions réelles avant de basculer.
- **Dépend de** : BRANCHER-MCP
- **Livrables** :
  - un mode où le MCP sert pgvector mais interroge aussi Crible, puis consigne pour chaque requête le recouvrement des top-k et les latences ;
  - aucune donnée de requête n'est conservée au-delà de la durée de vie du journal de debug ;
  - au bout de 7 jours, un rapport.
- **Critères d'acceptation** : le rapport est validé par le propriétaire.

### BRANCHER-TACHE-QUOTIDIENNE
- **Objectif** : les mises à jour quotidiennes passent par Crible.
- **Dépend de** : ADAPTER-PRODUCTEUR-LEGIFACILE, MODE-OMBRE
- **Livrables** :
  - la tâche de 5 h 30 : synchronisation, lots `upsert` produits en natif (MPS), attache par l'API avec un `batch_id` daté (idempotent), puis vérification du rapport ;
  - une alerte en cas d'échec de la recette ou de l'attache ;
  - la compaction automatique au seuil.
- **Critères d'acceptation** :
  - 14 jours consécutifs sans incident ;
  - MES-08 refaite à l'échelle réelle.
- **Invariants / mesures** : MES-08.

### BASCULER-PRODUCTION
- **Objectif** : Crible devient la source de la recherche de Legifacile.
- **Dépend de** : BRANCHER-TACHE-QUOTIDIENNE
- **Livrables** :
  - l'interrupteur positionné sur `crible` ;
  - pgvector conservé en lecture seule pendant 4 semaines ;
  - une procédure de retour arrière écrite et testée une fois.
- **Critères d'acceptation** :
  - la décision de basculer est prise par le propriétaire ;
  - le retour arrière est testé.

### BILAN-V1
- **Objectif** : clore la v1.
- **Dépend de** : BASCULER-PRODUCTION
- **Livrables** : `docs/jalons/J9-bilan.md`, qui contient :
  - toutes les mesures comparées aux cibles ;
  - les écarts à la conception et la façon dont ils ont été tranchés ;
  - les leçons ;
  - la liste priorisée pour la v2 (rerank, IVF, fusions progressives, découpeur par défaut…).
- **Critères d'acceptation** : le bilan est validé par le propriétaire. La v1 est taguée **par le propriétaire**.
