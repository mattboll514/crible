# P7 — Observabilité

**But** : rendre Crible vérifiable sans lire le code. Cette phase couvre les analytics sans contenu, le journal de debug, le debug dans la réponse, `explain_ids`, les stats et l'éval.

**Jalon J7** :
- `crible eval` sur l'échantillon Legifacile produit un rapport complet : fidélité, qualité, latence, pour les trois modes ;
- `explain_ids` sur un document absent des résultats dit pourquoi ;
- `crible stats` montre le trafic de la démo ;
- INV-13 et INV-17 sont verts.

---

### AGREGER-ANALYTICS
- **Objectif** : les analytics agrégées.
- **Dépend de** : ROUTE-RECHERCHE, FILE-D-ECRITURE-ET-JOBS
- **À lire** : `06-observabilite.md` (analytics).
- **Livrables** :
  - `SearchEvent` et `OpEvent`, deux types **sans champ** pour le texte, les valeurs de filtre ou les ids ;
  - le calcul de `filter_shape` à partir de l'AST (champs et opérateurs seulement) ;
  - un agrégateur par minute, avec des histogrammes à 64 seaux et un histogramme de pool en log2 ;
  - un vidage asynchrone toutes les 10 s et à l'arrêt, dans `analytics.sqlite` ;
  - les lignes journalières par index ;
  - la purge selon la rétention.
- **Critères d'acceptation** :
  - un test vérifie que les histogrammes s'additionnent (un p95 calculé sur deux minutes fusionnées égale le p95 calculé sur les événements bruts, à la résolution d'un seau près) ;
  - le vidage ne bloque jamais une requête (**INV-17**, file saturée artificiellement).
- **Invariants** : INV-17.

### TESTER-ANALYTICS-SANS-CONTENU
- **Objectif** : prouver INV-13.
- **Dépend de** : AGREGER-ANALYTICS
- **Livrables** : un test d'intégration qui lance 100 requêtes avec des textes et des valeurs de filtre uniques (marqueurs aléatoires), force le vidage, puis cherche chaque marqueur dans le fichier `analytics.sqlite` (octets bruts) et dans les logs `info`.
- **Critères d'acceptation** : aucun marqueur n'est trouvé.
- **Invariants** : INV-13.

### JOURNAL-DE-DEBUG
- **Objectif** : un journal complet, activé à la demande.
- **Dépend de** : AGREGER-ANALYTICS
- **À lire** : `06-observabilite.md` (journal de debug).
- **Livrables** :
  - `debug.sqlite` ;
  - un enregistrement si `debug: true` et que le journal est activé, et selon `sample_rate` ;
  - une écriture asynchrone ;
  - la purge selon la durée de vie (`ttl_days`) ;
  - `GET /v1/debug/queries`, réservé à l'admin, avec ses filtres.
- **Critères d'acceptation** :
  - désactivé par défaut : aucun fichier écrit ;
  - la purge supprime bien les entrées expirées ;
  - un rôle `read` reçoit `403` (ADR-0095) ;
  - `GET /v1/debug/queries` figure dans la spécification OpenAPI, et son exception nommée est retirée du test des routes de PUBLIER-OPENAPI dans le même changement (ADR-0111, complément du 2026-10-30).

### REPONSE-DEBUG
- **Objectif** : `debug: true` dans la réponse.
- **Dépend de** : ROUTE-RECHERCHE
- **À lire** : `05-api.md` (réponse, objet `debug`).
- **Livrables** : le plan, les temps par étape (avec les clés de `03-recherche.md`), et pour chaque résultat le rang et le score dans chaque branche ainsi que le rang fusionné ; en multi-index, aussi le rang dans son index (`target_rank`).
- **Critères d'acceptation** :
  - la somme des étapes reste cohérente avec le total (écart inférieur à 10 %) ;
  - les rangs affichés correspondent à ceux d'une exécution branche par branche.

### EXPLIQUER-DOCUMENTS
- **Objectif** : `explain_ids`.
- **Dépend de** : REPONSE-DEBUG
- **À lire** : `06-observabilite.md` (explain) ; `05-api.md` (format).
- **Livrables** : pour chaque document demandé :
  - s'il existe et s'il est vivant ;
  - s'il est dans le pool, et sinon les clauses qui l'excluent (chaque clause est évaluée séparément) ;
  - son rang dense exact (scan exact du pool), son rang binaire et sa présence dans les N candidats ;
  - son rang et son score BM25, avec l'explication tantivy ;
  - son rang fusionné, et s'il a été absorbé par un regroupement.
- **Critères d'acceptation** :
  - un test par cas : exclu par le filtre, mort, hors des N candidats binaires, rang lexical seul, absorbé par un regroupement ;
  - le cas « rang 70 en dense, rang 2 en lexical » de l'échantillon est expliqué.

### ROUTE-STATS
- **Objectif** : lire les analytics.
- **Dépend de** : AGREGER-ANALYTICS
- **À lire** : `06-observabilite.md` (lecture).
- **Livrables** :
  - `GET /v1/stats` : comptes, erreurs par code, p50/p95/p99 par étape, taux de requêtes sans résultat, index et formes de filtre les plus fréquents, état journalier ;
  - la commande `crible stats`, qui affiche des tableaux.
- **Critères d'acceptation** :
  - les chiffres affichés pour un trafic de test connu sont exacts ;
  - une clé `read` ou `write` reçoit `403` sur `/v1/stats`, réservé au rôle `admin` (ADR-0095) ;
  - `GET /v1/stats` figure dans la spécification OpenAPI, et son exception nommée est retirée du test des routes de PUBLIER-OPENAPI dans le même changement (ADR-0111, complément du 2026-10-30).

### COMMANDE-EVAL
- **Objectif** : l'instrument de validation du propriétaire.
- **Dépend de** : REPONSE-DEBUG
- **À lire** : `06-observabilite.md` (éval).
- **Livrables** :
  - `crible eval` (contre une instance via l'API, ou en local sur un dossier de données) ;
  - les mesures : fidélité contre `exact`, qualité contre `expected`, latences, comparaison `--baseline` ;
  - un rapport JSON et un rapport Markdown dans `docs/mesures/` ;
  - `docs/jalons/J7-demo.md`.
- **Critères d'acceptation** :
  - le rapport sur l'échantillon est lisible sans contexte ;
  - une régression injectée (N réduit) est bien détectée par `--baseline`.
