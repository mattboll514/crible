# Passation — projet Legifacile

**Objet** : les fiches de mise en service du projet Legifacile, sorties du plan de Crible le **2026-11-02** (ADR-0129, décisions 7 et 8), et le contrat que Crible garantit à ce projet.

**Ce document est hors du plan de Crible.** Il n'a ni phase ni jalon dans `docs/09-plan-de-developpement.md`, et l'orchestrateur de Crible ne le suit pas. Le projet Legifacile vit dans un **nouveau dépôt**, avec ses propres mécanismes, son authentification Firebase Auth et ses propres ADR. `legifacile-sandbox` est le dépôt **d'origine** du projet (backend pgvector, serveur MCP, migrations ; ADR-0129, décision 8) ; le nouveau dépôt prend le relais, et `legifacile-sandbox` va être arrêté (décision du propriétaire du 2026-11-08, `docs/suivi.md`). Les scripts de l'échantillon et celui de l'export complet vivaient dans ce dépôt. Crible reste **derrière le backend de Legifacile**, avec ses clés d'API : il n'est pas exposé aux utilisateurs.

**Ce qui a changé de main.** Les cinq fiches ci-dessous sont reprises **mot pour mot** de l'ancienne phase P9 de Crible. Chacune garde son texte ; la note qui la précède dit seulement ce qui a changé de main. Les tâches restées dans Crible — `DEFINIR-INDEX-LEGIFACILE`, `EXPORTER-BASE-LEGIFACILE`, `IMPORTER-BASE-COMPLETE`, `CONSTITUER-JEU-DE-REFERENCE`, `COMPARER-AVEC-PGVECTOR`, `MESURER-SUR-MAC-MINI`, `BILAN-V1` — vivent en P10 (`docs/phases/P10-validation-a-l-echelle-reelle.md`).

## Le contrat que Crible garantit

Crible garantit au nouveau dépôt quatre choses, et **rien de plus** (ADR-0129, décision 8) :

- le **format de lot v1** (`docs/format-lot-v1.md`, ADR-0012, ADR-0039, ADR-0040, `format_version: 1`) ;
- l'**API REST v1** et sa spécification OpenAPI (ADR-0015, ADR-0111) ;
- le **client Python** `clients/python` : `BatchWriter`, `Client`, `AsyncClient` (ADR-0112) ;
- l'**image Docker** multi-architecture publiée sur GHCR (P8, `PUBLIER-IMAGE` ; `docs/07-stack-et-deploiement.md`, section « Docker »).

Le nouveau dépôt **épingle Crible à un tag**. Aucune promesse de compatibilité au-delà de ce tag n'est faite ; toute évolution du format de lot ou de l'API après ce tag se négocie. Aucune publication sur PyPI n'est décidée ici.

---

### ADAPTER-PRODUCTEUR-LEGIFACILE

**Note de passation (ADR-0129).** La fiche change de main : elle était dans la phase P9 de Crible, elle appartient désormais au projet Legifacile. Son mode « migration » ne suit pas : il devient la tâche ponctuelle `EXPORTER-BASE-LEGIFACILE` de P10. Le texte est conservé mot pour mot.

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

### BRANCHER-MCP

**Note de passation (ADR-0129).** La fiche change de main : elle appartient au projet Legifacile. L'interrupteur `backend` reste le retour arrière du projet ; il ne conditionne plus J10. Le texte est conservé mot pour mot.

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

**Note de passation (ADR-0129).** La fiche change de main : elle appartient au projet Legifacile, et la validation de son rapport revient au propriétaire du projet. Elle ne conditionne plus J10. Le texte est conservé mot pour mot.

- **Objectif** : comparer en conditions réelles avant de basculer.
- **Dépend de** : BRANCHER-MCP
- **Livrables** :
  - un mode où le MCP sert pgvector mais interroge aussi Crible, puis consigne pour chaque requête le recouvrement des top-k et les latences ;
  - aucune donnée de requête n'est conservée au-delà de la durée de vie du journal de debug ;
  - au bout de 7 jours, un rapport.
- **Critères d'acceptation** : le rapport est validé par le propriétaire.

### BRANCHER-TACHE-QUOTIDIENNE

**Note de passation (ADR-0129).** La fiche change de main : elle appartient au projet Legifacile. Les 14 jours de stabilité et MES-08 refaite à l'échelle réelle sont des critères de ce projet, et non plus de Crible (`docs/08-invariants-et-tests.md` renvoie MES-08 à cette passation). Le texte est conservé mot pour mot.

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

**Note de passation (ADR-0129).** La fiche change de main : la bascule appartient au projet Legifacile et ne conditionne plus J10. Côté Crible, la latence contraignante est mesurée **avant**, par `MESURER-SUR-MAC-MINI` en P10 ; l'arrêt de Legifacile et l'installation de Crible sur le Mac mini sont des gestes du **propriétaire**, qu'aucun agent n'exécute (ADR-0032, ADR-0129 décisions 2 et 9). Le texte est conservé mot pour mot.

- **Objectif** : Crible devient la source de la recherche de Legifacile.
- **Dépend de** : BRANCHER-TACHE-QUOTIDIENNE
- **Livrables** :
  - l'interrupteur positionné sur `crible` ;
  - pgvector conservé en lecture seule pendant 4 semaines ;
  - une procédure de retour arrière écrite et testée une fois.
- **Critères d'acceptation** :
  - la décision de basculer est prise par le propriétaire ;
  - le retour arrière est testé.
