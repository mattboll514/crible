# P5 — Lexical et hybride

**But** : ajouter la branche BM25 (tantivy) sur le même pool que la branche dense, avec l'analyse française et les deux méthodes de fusion. Le cas réel à couvrir est celui de « l'article 1240 ».

**Jalon J5** :
- en mode hybride, sur l'échantillon Legifacile, une requête qui vise un article par son numéro (type « article 1240 code civil ») le place dans le top 3 ;
- MES-06, MES-10 et MES-11 sont consignées ;
- la fusion par défaut est choisie et validée (Q-02), ainsi que le traitement des élisions et des identifiants (Q-04) ;
- INV-05 est vert ;
- l'équivalence avant/après compaction (INV-09) tient toujours, branche lexicale comprise.

---

### CONFIGURER-ANALYSEUR
- **Objectif** : construire la chaîne d'analyse tantivy à partir de la config de l'index.
- **Dépend de** : DEFINIR-CONFIG-INDEX
- **À lire** : `03-recherche.md` (branche lexicale).
- **Livrables** :
  - `crible-store::lexical::analyzer` : `SimpleTokenizer`, `RemoveLongFilter(40)`, `LowerCaser`, `AsciiFoldingFilter`, `StopWordFilter(langue)` — absent pour `ar`, `el`, `ro`, `ta` et `tr` —, `Stemmer(langue)` ;
  - les langues prises en charge : les dix-huit de tantivy, désignées par un code ISO 639-1 en minuscules (`fr`, `en`, `no`, `el`… ; table d'ADR-0086). Toute autre valeur est refusée en `invalid_request` (400) à la validation de la configuration, avec reconstruction du lexical à chaque `PATCH` accepté ;
  - des tests sur des phrases juridiques (« l'article 1240 du code civil », « L. 1132-1 », « qu'il », « dommages-intérêts »), avec les jetons obtenus.
- **Critères d'acceptation** :
  - les jetons sont documentés dans le rapport de la tâche ;
  - les cas problématiques sont signalés pour MES-11, sans être corrigés ici.
- **Invariants / mesures** : MES-11 (préparation).

### INDEXER-TANTIVY-A-L-ATTACHE
- **Objectif** : intégrer tantivy au protocole d'attache.
- **Dépend de** : CONFIGURER-ANALYSEUR, ATTACHER-LOT
- **À lire** : `02-architecture.md` (générations : étape 4, reprise au démarrage).
- **Livrables** :
  - un index tantivy par époque, avec un champ `iid` (u64, fast et indexed) et un champ `text` (analysé, non stocké) ;
  - à l'étape 4 : ajout des nouveaux passages, suppression des passages tombés, commit avec la charge utile G+1 ;
  - un lecteur rechargé dans le `Snapshot` ;
  - la reprise : un index lexical est **à jour pour `G`** quand `M ≤ Gt ≤ G`, où `Gt` est la charge utile du dernier commit tantivy et `M` vaut `meta.passage_change_generation` (dernière génération qui a créé ou fait mourir un passage, `0` sinon) (ADR-0088). Si `M ≤ Gt`, aucun rattrapage ni commit tantivy, même quand `Gt < G` — c'est le cas d'un patch ; si `M > Gt`, ré-indexer les écarts ; si `Gt > G`, tout reconstruire. La colonne `meta.passage_change_generation` est complétée à l'ouverture en écriture si elle manque (`ALTER TABLE`, puis calcul unique par époque), sans génération nouvelle. Une charge utile ou un dossier `lexical/` illisible est un état indécidable : reconstruit à l'ouverture en écriture, comme `Gt > G`, avec une ligne de journal. La reprise et la comparaison n'ont lieu qu'à l'ouverture en **écriture** (ADR-0087) : en lecture seule, aucun `IndexWriter` n'est ouvert, rien n'est commité ni supprimé sous `lexical/`, rien n'est écrit dans l'époque, et un lecteur n'est ouvert que si `M ≤ Gt ≤ G` — une colonne `M` absente réduit le critère à `Gt = G` —, un lexical illisible laissant l'instantané sans lecteur lexical sans faire échouer l'ouverture (ADR-0088) ;
  - la durée de l'étape 4 mesurée à part : champ `lexical` d'`AttachDurations`, dans l'ordre `validation`, `segment`, `transaction`, `lexical`, `manifest`, `snapshot` (ADR-0087, télémétrie d'appel seulement).
- **Critères d'acceptation** :
  - **patch puis ouverture en lecture seule : lecteur présent**, `lexical` et `hybrid` servis à `G` (ADR-0088) ;
  - un patch puis une ouverture en écriture ne produit aucun commit tantivy et laisse la charge utile inchangée (ADR-0088) ;
  - une charge utile non décimale et un dossier `lexical/` illisible : l'ouverture en écriture reconstruit et journalise sans erreur, et l'ouverture en lecture seule sert `dense` et refuse `lexical` en `overloaded` (ADR-0088) ;
  - un test de panne entre l'étape 3 et l'étape 4, suivi d'un redémarrage, retrouve le lexical à G (**INV-04**) ;
  - un index avec `lexical.enabled: false` ne crée aucun dossier lexical ;
  - une ouverture en lecture seule quand le critère `M ≤ Gt ≤ G` n'est pas tenu n'écrit rien et n'ouvre aucun `IndexWriter` (ADR-0087, ADR-0088) ;
  - la durée de l'étape 4 est attribuable séparément de celle de `snapshot`.
- **Invariants** : INV-04.

### RECONSTRUIRE-LEXICAL
- **Objectif** : reconstruire tantivy à partir du texte stocké dans SQLite.
- **Dépend de** : INDEXER-TANTIVY-A-L-ATTACHE, COMPACTER-INDEX
- **Livrables** :
  - la reconstruction branchée sur la compaction (point d'accroche prévu en P3) ;
  - un job `rebuild_lexical`, lancé quand l'analyseur ou les positions changent ;
  - la commande `crible rebuild-lexical`.
- **Critères d'acceptation** :
  - un index reconstruit donne les mêmes résultats BM25 que l'index construit au fil des lots, une fois les passages morts retirés ;
  - **INV-09** reste vert avec la branche lexicale.
- **Invariants** : INV-09.

### RECHERCHER-BM25-DANS-LE-POOL
- **Objectif** : la branche lexicale, limitée au pool.
- **Dépend de** : INDEXER-TANTIVY-A-L-ATTACHE, EVALUER-FILTRES
- **À lire** : `03-recherche.md` (branche lexicale).
- **Livrables** :
  - la requête analysée et transformée en sac de termes (OU) ;
  - un collecteur personnalisé qui lit le fast field `iid`, ne garde que les passages du pool et tient un tas top-k ;
  - les scores BM25 ;
  - l'explication tantivy disponible pour `explain` (P7) ;
  - le refus explicite en `overloaded` (503) quand l'instantané n'a pas de lecteur lexical, c'est-à-dire quand le critère `M ≤ Gt ≤ G` n'est pas tenu à l'ouverture en lecture seule (ADR-0087, ADR-0088) : le message nomme l'état constaté et garde la phrase d'ADR-0087:38 pour le seul cas où elle est vraie — « lexical index behind generation Gt of G; open the index for writing to repair it » (`M > Gt`), « lexical index ahead of generation Gt of G; open the index for writing to repair it » (`Gt > G`), « lexical index state at generation Gt is not certified for generation G; open the index for writing to repair it » (colonne `M` absente), « lexical index is unreadable at generation G; open the index for writing to repair it » (`lexical/` illisible, `M > G`) — sans dégradation silencieuse de `hybrid` vers le dense, le mode `dense` restant servi. Deux états n'ont aucun `Gt` à citer et reçoivent leur propre message (ADR-0089) : « no lexical index is committed in this epoch although passages exist (generation G); open the index for writing to rebuild it » (`Behind` sans commit tantivy) et « lexical index state is not certified for generation G; open the index for writing to repair it » (`NotCertified` sans commit tantivy). `Missing` (`M = 0`) n'a aucun passage à servir et rend une réponse vide, sans refus. Une recherche `lexical` ou `hybrid` sur un index dont `lexical.enabled` vaut `false` est refusée en `invalid_request` (400), message « lexical branch is disabled for this index » (ADR-0089). Aucun `iid` ne figure dans ces messages (INV-01).
- **Critères d'acceptation** :
  - aucun résultat ne sort du pool (**INV-05**) ;
  - aucun passage mort n'apparaît, même si tantivy ne l'a pas encore supprimé ;
  - les résultats sont égaux à un calcul BM25 de référence sur un petit corpus de test ;
  - une recherche `lexical` ou `hybride` sur un instantané sans lecteur lexical échoue en `overloaded` (503) avec le message de l'état constaté — l'un des quatre de la décision 5 d'ADR-0088 ou l'un des deux de la décision 1 d'ADR-0089 —, et une recherche `dense` sur le même instantané réussit (ADR-0087, ADR-0088) ;
  - une recherche `lexical` ou `hybride` sur un instantané dont l'état est `Behind` ou `NotCertified` sans commit tantivy échoue en `overloaded` avec le message de cet état, et une époque `Missing` rend une réponse vide en `lexical` comme en `hybrid`, sans erreur (ADR-0089) ;
  - une recherche `lexical` ou `hybride` sur un index avec `lexical.enabled: false` échoue en `invalid_request` (400), et une recherche `dense` sur le même index réussit (ADR-0089).
- **Invariants** : INV-05.

### FUSIONNER-HYBRIDE
- **Objectif** : le mode `hybrid`.
- **Dépend de** : RECHERCHER-BM25-DANS-LE-POOL, REGROUPER-RESULTATS
- **À lire** : `03-recherche.md` (fusion, pipeline).
- **Livrables** :
  - les deux branches exécutées en parallèle sur le même pool ;
  - les deux fusions : `rrf` (constante 60) et `relative_score` (min-max, α), avec la règle de la branche constante et le domaine de `alpha` (ADR-0090) ;
  - le rang et le score de chaque branche conservés pour chaque résultat ;
  - le `score_type` ;
  - les modes `dense`, `lexical` et `hybrid` dans la CLI.
- **Critères d'acceptation** :
  - des tests unitaires de fusion sur des listes connues ;
  - un document présent dans une seule branche est traité correctement ;
  - une branche dont tous les candidats ont le même score, y compris une branche à un seul candidat, donne 1 à chacun d'eux, et une branche vide ne contribue rien (ADR-0090) ;
  - `alpha` hors de `[0, 1]`, `NaN` ou infini est refusé en `invalid_request` (400) à la création de l'index, au `PATCH` et par requête (ADR-0090) ;
  - le regroupement s'applique bien après la fusion.

### MESURER-LEXICAL
- **Objectif** : MES-06.
- **Dépend de** : RECHERCHER-BM25-DANS-LE-POOL
- **Livrables** :
  - la taille de l'index, avec et sans positions ;
  - la durée de construction, mesurée sur l'échantillon et extrapolée aux 8,58 M passages ;
  - la latence BM25 filtrée selon la taille du pool.
- **Critères d'acceptation** : un rapport reproductible, avec une alerte si le filtrage du pool est trop lent ; dans ce cas, proposer le repli de ADR-0007.
- **Invariants / mesures** : MES-06.
- **Protocole** : fixé par ADR-0091 (jeu et extrapolation, positions, pools et requêtes, seuil de l'alerte, périmètre, harnais).

### MESURER-HYBRIDE
- **Objectif** : MES-10 et MES-11, puis les choix par défaut.
- **Dépend de** : FUSIONNER-HYBRIDE, CONSTITUER-ECHANTILLON-LEGIFACILE
- **Livrables** :
  - recall@10 et MRR en dense, lexical, `rrf` et `relative_score` (α ∈ {0,3, 0,5, 0,75}), mesurés sur le jeu de référence ;
  - le cas « article 1240 » et ses variantes ;
  - l'effet des élisions et des identifiants.
- **Critères d'acceptation** : le rapport recommande une fusion par défaut et un traitement des élisions et des identifiants (Q-02, Q-04), qui sont ensuite validés par le propriétaire.
- **Invariants / mesures** : MES-10, MES-11.
