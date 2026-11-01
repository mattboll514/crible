# ADR-0126 — Cache des lecteurs denses : lecteurs par segment, pool de charges utiles par époque, remplissage avant publication

- **Statut** : Accepté (accepté par anticipation par le propriétaire le 2026-11-01, délégation du propriétaire pour la suite de P7 ; validation officielle à la fin de la série)
- **Date** : 2026-11-01
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, tâche REUTILISER-LECTEURS-DENSES (phase P7), sur l'analyse de `@architecte`, la mesure du livrable 1 et les décisions retenues par l'orchestrateur, en application de la délégation du propriétaire du 2026-11-01 pour la série EXPLIQUER-DOCUMENTS, ROUTE-STATS, COMMANDE-EVAL, REUTILISER-LECTEURS-DENSES
- **Remplace** : ADR-0120, **décisions 2 et 3 seulement** — la décision 3 (granularité et clé du cache) est remplacée ; la décision 2 (ouverture des lecteurs rattachée à `fetch`) devient **sans objet**. Les décisions 1, 4 et 5 d'ADR-0120 restent en vigueur, et son corps n'est pas réécrit.

## Contexte

### La fiche et la mesure du livrable 1

La fiche `REUTILISER-LECTEURS-DENSES` (`docs/phases/P7-observabilite.md:128-144`) vient de la décision 3 d'ADR-0120 : elle demande **d'abord une mesure** du coût d'ouverture des lecteurs denses à 8,58 M passages (`:134`), puis, si ce coût n'est pas négligeable, que « les lecteurs denses sont ouverts **une fois par génération publiée** et réutilisés par les requêtes de cette génération, puis libérés quand la génération n'est plus servie » (`:136`). Un ADR doit fixer « la durée de vie, l'interaction avec la compaction, la rétention des époques et la mémoire » (`:136`). Les critères sont : aucune ouverture de lecteur dense sur le chemin d'une requête d'une génération déjà servie, INV-02 vert, aucun lecteur d'une génération retirée gardé au-delà de sa rétention, le critère des 10 % de REPONSE-DEBUG toujours vert, et la latence avant/après mesurée (`:139-143`).

La mesure du livrable 1 (`docs/mesures/lecteurs-denses-ouverture-2026-11-01.md`) répond par un chiffre sans ambiguïté :

| Point | Passages vivants | Octets de vecteurs | p50 | p95 |
|---|---|---|---|---|
| `synth-large` | 7 740 000 | 13 209 600 000 | 8 262,9 ms | **8 406,9 ms** |
| `articles` | 50 000 | 76 800 000 | 43,8 ms | 44,9 ms |
| `synth-small` | 9 750 | 14 976 000 | 7,9 ms | 8,2 ms |

Deux cents ouvertures par point, cache chaud, `cargo build --release`, harnais `crates/crible/examples/dense_latency.rs`. Le temps suit les **octets** du fichier de vecteurs à débit presque constant (1,60 à 1,89 Go/s), ce qui désigne la **relecture intégrale pour la somme de contrôle blake3** à l'ouverture (`crates/crible-store/src/segment.rs:983-1007`). Le rapport dit lui-même que ce n'est pas isolé par un test séparé, puisqu'un chemin sans sceau serait un changement de code interdit dans cette session (`:67`).

Le seuil de la fiche est de 1 ms au p95 : la branche « négligeable, la tâche se ferme sans code » n'est pas la situation mesurée, et la branche 3 (cache) s'applique.

### Ce que le code fait aujourd'hui

- `DenseSearcher::open` (`crates/crible-search/src/dense/exact.rs:154-190`) ouvre **un lecteur par segment** et un **pool de connexions en lecture** vers `records.sqlite` de l'époque, puis les garde. Sa documentation dit déjà que « the searcher opens every segment once, at construction, so it serves many queries: `SegmentReader::open` verifies the blake3 checksum of the whole vector file, which must not be paid per query » (`:124-131`).
- Or, chaque recherche du serveur construit un `DenseSearcher` **neuf** : `crates/crible-server/src/search.rs:1043-1046` chronomètre l'ouverture et la rattache à `fetch` (`:1067`), règle provisoire d'ADR-0120, décision 2. Le corps du searcher est donc jeté à la fin de chaque requête, et le coût de la mesure ci-dessus est payé **à chaque requête dense**.
- La CLI ouvre **un searcher par processus** : `crible search` (`crates/crible/src/commands.rs:810`) et `crible eval` (`crates/crible/src/eval.rs:282`, un searcher porté par la campagne entière). La CLI est donc déjà conforme à l'intention, et le problème est celui du serveur.

### Ce que la conception pose, et ce qu'elle laisse ouvert

- **Segments immuables.** Un lot crée un segment immuable qui couvre une plage contiguë d'iid (ADR-0006 ; `docs/02:50`, `:125`). Un segment n'est jamais réécrit dans son époque.
- **Un patch ne crée aucun segment.** Les étapes 3, 5 et 6 seulement ont lieu, et le manifeste de `G+1` reprend « les mêmes segments que G » (`docs/02:137`).
- **Une compaction crée une époque neuve, donc des segments neufs.** `segments.segment_id` vaut `c<E>-<iid_start>` et `batch_id` vaut `compaction-e<E>-e<E+1>` (ADR-0115 ; `docs/02:108`).
- **Une génération est visible entièrement ou pas du tout** (INV-03), et la publication est un seul point (`docs/02:120`, `:129`, `:138`). La configuration et l'instantané sont publiés ensemble sous un seul verrou (`Published`, `crates/crible-server/src/jobs.rs:607-631`, `:794-812` ; ADR-0105, décision 4).
- **Les lectures ne sont jamais bloquées par une écriture** (INV-02 ; `docs/08:10`), et MES-16 (ADR-0113) en fixe le critère mesuré.
- **Ce qui est parcouru en entier à chaque requête est en RAM ; ce qui est lu pour quelques candidats reste en mmap** (`docs/02:224`). Les vecteurs de référence restent en mmap : 13,2 Go pour Legifacile (`docs/02:233`).
- **La récupération des époques est gouvernée par les instantanés.** `reclaim_epochs` supprime une époque que ni le magasin ni un instantané ne tient (`crates/crible-store/src/compact.rs:189-209`), et un instantané tient son époque par une `EpochLease` (`crates/crible-store/src/generation.rs:326-327`, ADR-0065, ADR-0017).

### Les points non couverts

1. **La granularité et la clé du cache.** ADR-0120, décision 3, dit « par génération » et renvoie forme, éviction et mémoire à la tâche. Or un patch crée une génération **sans segment neuf** : un cache par génération jetterait des lecteurs encore valides et repaierait l'ouverture pour rien.
2. **Le sceau blake3.** Aucun document de conception ne l'impose à chaque ouverture. Seul le commentaire de `crates/crible-store/src/segment.rs:445-459` le décrit, et il laisse explicitement ouverte la question de le différer « once per generation, or only on recovery ». ADR-0084, décision 8, range « un chemin d'ouverture sans vérification du sceau » parmi ce qui n'est pas décidé.
3. **Le propriétaire du cache.** ADR-0120, décision 3, ne dit pas où il vit, ni qui le remplit, ni sous quel verrou.
4. **La durée de vie.** Un lecteur en cache ne tient aucune `EpochLease` ; rien ne dit comment la rétention des époques et la libération des lecteurs s'articulent.
5. **La concurrence.** La première requête d'une génération non encore remplie n'est couverte par aucun texte, et le remplissage pourrait vouloir le verrou d'écriture de `Published`.
6. **Le sens de `fetch`.** `docs/03:14` et `docs/06:32` décrivent une attribution provisoire qui disparaît avec le cache ; il faut écrire la suite.
7. **La mesure avant/après.** `crible eval` ouvre un searcher par campagne (ADR-0125, décision 9) : il ne peut pas montrer le gain d'une réutilisation.

## Décision

### Décision 1 — Granularité : lecteurs par segment, pool de charges utiles par époque

**Décision.** Le cache garde des **lecteurs de vecteurs par segment**, clé `segment_id`, et un **pool de charges utiles par époque**. Un `DenseSearcher` se **compose** à partir des lecteurs de la génération publiée et de ses bitmaps, **sans aucune lecture de fichier de vecteurs**.

Précisions :

- les segments sont **immuables** (ADR-0006) : un lecteur ouvert pour un `segment_id` reste valide tant que ce segment est dans une génération publiée, quelle que soit la génération ;
- un **patch ne crée aucun segment** (`docs/02:137`) : il ne fait ouvrir aucun lecteur, et le cache garde les mêmes entrées ;
- une **attache** ne fait ouvrir que le **segment neuf** de son lot ; les segments des générations précédentes restent en cache ;
- le **démarrage**, la **reprise**, la **compaction** et la **réouverture après échec** (ADR-0108, décision 2) ouvrent **tous** les segments de la génération qu'ils publient, parce qu'ils publient une époque neuve ou un processus neuf ;
- le **pool de charges utiles** est clé par l'**époque** : `records.sqlite` change avec l'époque, pas avec la génération. Un patch ne le rouvre donc pas non plus ;
- « composer sans I/O » vaut pour les **vecteurs** : la création du pool de charges utiles ouvre les connexions de l'époque une fois, à son entrée dans le cache, jamais par requête ;
- le `DenseSearcher` reçoit ces lecteurs déjà ouverts ; il ne les ouvre plus lui-même sur le chemin d'une requête.

### Décision 2 — Le sceau blake3 est vérifié une fois par segment et par processus, pendant le remplissage

**Décision.** La vérification blake3 reste **obligatoire**, mais elle a lieu **une fois par segment et par processus**, à l'ouverture du lecteur **pendant le remplissage du cache**, jamais sur le chemin d'une requête. **Aucun second mode d'ouverture sans vérification n'est créé.**

Précisions :

- la règle est obtenue par le **cache**, pas en retirant le contrôle : `SegmentReader::open` continue de vérifier (`crates/crible-store/src/segment.rs:983-1007`), et c'est le nombre d'ouvertures qui tombe à un par segment ;
- après une compaction, les segments sont **neufs** (`c<E>-<iid_start>`, ADR-0115) ; après une restauration ou un redémarrage, c'est un **processus neuf** qui vérifie chaque segment une fois ;
- le réchauffement du fichier par la vérification est **conservé au remplissage** : le fichier est lu en entier juste avant sa première utilisation, comme aujourd'hui ;
- la série froide qui fait foi (ADR-0084, décision 1) ne change pas : le protocole de mesure continue de lire le cache de pages tel qu'il est, et un chemin d'ouverture sans sceau reste **non décidé** (ADR-0084, décision 8) ;
- un segment dont le sceau ne correspond pas fait échouer **le remplissage**, donc la publication : l'index ne sert pas une génération dont un segment est corrompu. C'est plus tôt et plus fort qu'aujourd'hui, où chaque requête échouait ;
- la décision ne change ni le format de `segment.json`, ni le calcul du sceau, ni les fichiers.

### Décision 3 — Propriété : les lecteurs et leur cache vivent dans `crible-store`

**Décision.** Les lecteurs et leur cache vivent dans **`crible-store`**, à côté de `SegmentReader` et des locations d'époque. `crible-search` **reçoit** les lecteurs et construit le `DenseSearcher` sans I/O. Le serveur porte les lecteurs dans son **état publié** (`Published`), avec la configuration et l'instantané. La **CLI** garde une ouverture par processus, par le même code de remplissage.

Précisions :

- `crible-search` dépend déjà de `crible-store` (crate `crible-search`), donc aucun sens d'arête ne change et aucune dépendance n'est ajoutée ;
- le cache appartient à l'époque et au processus : il n'est ni global au moteur, ni statique, ni partagé entre deux index ;
- dans le serveur, `Published` porte déjà la configuration, l'instantané et les compteurs sous un même verrou (ADR-0105, décision 4) ; il porte **désormais aussi** les lecteurs de l'instantané publié, remplacés **ensemble** sous le même verrou d'écriture ; aucune requête ne lit une configuration d'une génération avec les lecteurs d'une autre ;
- la CLI ne tient pas de cache entre commandes : un processus, une ouverture, comme aujourd'hui (`crates/crible/src/commands.rs:810`, `crates/crible/src/eval.rs:282`) ;
- aucun nouveau crate, aucune nouvelle dépendance : `SegmentReader`, `ReadPool` et le comptage de références de la bibliothèque standard suffisent.

### Décision 4 — Remplissage avant publication, hors du verrou d'écriture

**Décision.** Celui qui **publie** une génération ouvre d'abord les lecteurs manquants, **hors** du verrou d'écriture de `Published`, en réutilisant ceux des segments déjà ouverts, puis publie **instantané, configuration et lecteurs ensemble**. Définition écrite : **« une génération servie est une génération publiée avec ses lecteurs »**. Aucune requête ne voit une génération sans ses lecteurs.

Précisions :

- les points de publication sont ceux de `docs/02` : démarrage et reprise (`:142-162`), commit d'une attache, d'un patch ou d'un lot en ligne (`:118-137`), compaction et réouverture après échec (`:179-207`, ADR-0108, décision 2), restauration hors ligne (ADR-0065) ;
- le **patch** ne fait ouvrir aucun lecteur et republie les mêmes ; l'**attache** n'ouvre que son segment neuf ; le démarrage, la reprise et la fin d'une compaction ouvrent **tous** les segments de la génération qu'ils publient ;
- le remplissage ne tient **jamais** le verrou d'écriture de `Published` : la requête en cours lit l'ancien état publié pendant que les nouveaux lecteurs s'ouvrent ;
- **coût écrit** : le démarrage et la fin d'une compaction paient l'ouverture de tous les segments, soit environ **8,3 s par 13,2 Go** sur la machine de développement (p50 8 262,9 ms, p95 8 406,9 ms ; `docs/mesures/lecteurs-denses-ouverture-2026-11-01.md`). Ce coût est à mesurer contre MES-13 et contre le budget de démarrage de 60 s rappelé par ADR-0047. Le commit d'une attache ne paie que l'ouverture de son segment neuf ;
- le cas « première requête d'une génération non remplie » **n'existe plus** : la publication a lieu après le remplissage, et une génération non remplie n'est pas servie ;
- un échec du remplissage **empêche la publication** et remonte à l'appelant (commit, démarrage, compaction) : s'il existait une génération précédente, l'index reste dessus, avec ses lecteurs ; au démarrage, il n'y en a pas, et l'ouverture échoue. La règle d'INV-03 « entièrement ou pas du tout » vaut aussi pour les lecteurs.

### Décision 5 — Durée de vie : les lecteurs de la génération publiée, et rien de plus

**Décision.** Le cache ne garde que les lecteurs des segments de la **génération publiée courante**. À chaque publication, il réutilise ceux qui restent et **lâche** les autres. Un lecteur lâché vit tant qu'un instantané de requête en cours le tient, par **comptage de références**, puis il est libéré. **Aucun lecteur ne tient d'`EpochLease`.**

Précisions :

- la rétention des époques reste gouvernée par les **instantanés seuls** : `reclaim_epochs` (`crates/crible-store/src/compact.rs:189-209`) supprime une époque que ni le magasin ni un instantané ne tient (ADR-0065, ADR-0017) ;
- une requête qui tient un vieux lecteur tient aussi l'instantané de sa génération, donc sa location d'époque : le dossier ne peut pas disparaître sous ses fichiers mmapés ;
- un lecteur **lâché** n'est pas détruit tant que la dernière requête qui le tient n'a pas fini : c'est le comptage de références qui décide, pas le cache ;
- la **suppression d'un index** (ADR-0105) lâche ses lecteurs avec son état publié ;
- **jamais `codes_words` sur un lecteur en cache** : les codes binaires restent dans l'instantané (`crates/crible-store/src/segment.rs:689`, champ `:407`), et un lecteur en cache n'a pas à les charger ni à garder 0,82 Go de plus ;
- la **mémoire possédée est inchangée** : les vecteurs restent en mmap (`docs/02:224`, `:233`). Ce qui change est le nombre de mappings et de descripteurs, pas la résidence ni la taille des fichiers ;
- le pool de charges utiles de l'époque est lâché avec l'époque qu'il sert, quand plus aucun instantané ne la tient.

### Décision 6 — Concurrence : aucune ouverture sur le chemin d'une génération servie, INV-02 prouvé

**Décision.** Aucune ouverture de lecteur dense n'a lieu sur le chemin d'une requête d'une génération **servie**. Le remplissage ne tient jamais le verrou d'écriture de `Published`. **INV-02 est prouvé par un test.**

Précisions :

- une requête prend son `Arc<Snapshot>` au départ (`docs/02:172`), lit `Published` sous un seul verrou de lecture, et compose son `DenseSearcher` avec des `Arc` déjà ouverts ; elle ne fait aucune I/O d'ouverture de vecteurs ;
- les lecteurs sont partagés entre requêtes : le pool de charges utiles de l'époque est désormais **commun** aux requêtes concurrentes, alors qu'il était créé par requête. Un appel à `checkout` peut donc **attendre** une connexion libre, sans blocage circulaire, la règle de non-imbrication de `ReadPool` restant observée à l'intérieur d'une requête. C'est un point de contention nouveau, à surveiller, pas un blocage ;
- le test d'INV-02 suit le critère d'ADR-0113 : chaque recherche reste sous 3 × le p99 du repos pendant une attache et une compaction, et il vérifie en plus qu'aucune ouverture de lecteur n'a lieu sur le chemin d'une génération déjà servie ;
- la preuve d'absence d'ouverture se fait sur le chemin observable : un compteur d'ouvertures de segment par processus (ou un point de défaillance de la feature `failpoints`), lu par le test, plutôt qu'une affirmation de lecture de code ;
- la génération servie sans lecteurs n'existe pas (décision 4), donc le test n'a pas de cas « première requête » à tolérer.

### Décision 7 — `fetch` retrouve son sens : lecture des charges utiles seulement

**Décision.** La clé `fetch` redevient ce que `docs/03:14` décrit : la **lecture des champs et du texte des `k` résultats**. La règle provisoire d'ADR-0120, décision 2, **devient sans objet**, et `docs/03:14` et `docs/06:32` sont mis à jour en conséquence.

Précisions :

- `took_ms` ne change pas : il s'arrête toujours à la fin de `fetch`, avant le rendu JSON (ADR-0120, décision 1) ;
- les neuf clés de `docs/03` restent les mêmes, aucune dixième clé n'est créée ;
- le critère des 10 % de REPONSE-DEBUG reste la preuve de cohérence, **sans seuil abaissé** (ADR-0120, décision 5 ; ADR-0119, décision 6) ;
- l'histogramme `fetch` des analytics redevient interprétable seul : une valeur qui monte avec le corpus ne peut plus venir de l'ouverture masquée ;
- la formulation des deux documents est corrigée, pas complétée d'une nouvelle règle : l'ouverture n'est plus dans `fetch` parce qu'elle n'est plus dans la requête.

### Décision 8 — Mesure avant/après : le serveur, pas `crible eval`

**Décision.** La preuve du gain est une mesure **avant/après du serveur** en `--release`, sur `synth-large`, mode `dense`, `k` et N fixes, **au moins 300 requêtes à cache chaud**, `debug: true`, avec p50, p95 et p99 de `total` et de `fetch`, **avant** (commit de base) et **après**. Le **temps de démarrage du serveur** avant/après est mesuré aussi. Le contrôle reproductible est le harnais `crates/crible/examples/dense_latency.rs`. Le rapport va sous `docs/mesures/`.

Précisions :

- `crible eval` n'est **pas** l'outil de cette preuve : il ouvre un searcher par campagne (ADR-0125, décision 9), donc il mesure un cache déjà rempli dans les deux versions, et ne peut pas montrer le gain d'une réutilisation par requête ;
- `synth-large` est le point qui compte : c'est là que l'ouverture coûte 8,3 s, et non sur l'échantillon ;
- l'échauffement précède les 300 requêtes mesurées ; les percentiles sont au rang le plus proche (ADR-0117, décision 4) ;
- le démarrage **augmente** nécessairement (décision 4) : la mesure le chiffre au lieu de le taire, et le rapporte au budget de 60 s d'ADR-0047 et à MES-13 ;
- le rapport dit ce qu'il ne prouve pas : une machine de développement, un seul segment, un mode, un `k` et un N ; le froid de P8 et le conteneur ne sont pas couverts ;
- aucune affirmation de gain n'est faite avant le rapport (règle 7 d'`AGENTS.md`).

## Conséquences

- **Positives** :
  - l'ouverture des lecteurs denses **sort du chemin de chaque requête** : le p95 de 8 406,9 ms à 8,58 M passages n'est plus payé qu'aux publications qui ouvrent des segments, et une attache ne paie que son segment neuf ;
  - la granularité par segment est la plus fine que permettent les segments immuables : un patch et une attache ne repaient pas ce qu'ils ne changent pas ;
  - `fetch` retrouve son sens, et l'histogramme des analytics redevient lisible ; `took_ms` et les neuf clés ne changent pas ;
  - la règle « une génération servie est une génération publiée avec ses lecteurs » donne un critère testable, qui couvre INV-02 et le cas de la première requête ;
  - aucun nouveau format, aucune dépendance nouvelle, aucune modification de `crible-search` hors de la composition : le sceau, les codes et les fichiers restent ce qu'ils sont ;
  - la CLI reste ce qu'elle est : une ouverture par processus, par le même code, donc une seule vérité.
- **Négatives / coûts** :
  - le **démarrage** et la **fin de compaction** paient l'ouverture de tous les segments : environ 8,3 s par 13,2 Go sur la machine de développement, à confronter aux 60 s d'ADR-0047 ;
  - une génération dont un segment est corrompu **ne se publie plus** : l'index ne démarre pas, ou le commit échoue, au lieu d'échouer requête par requête. Le diagnostic est plus tôt et plus net, mais il est bloquant ;
  - le **pool de charges utiles** devient commun aux requêtes d'une même époque : `checkout` peut attendre une connexion libre là où chaque requête avait les siennes, et la latence de `fetch` dépend désormais du nombre de recherches concurrentes ;
  - le cache garde des **mappings** et des **descripteurs** pour chaque segment de la génération publiée, même si un segment ne sert jamais ; l'ouverture au démarrage les établit tous ;
  - une publication qui change d'époque (compaction, ou ouverture après restauration) **paie tout** et jette les lecteurs de l'ancienne époque, sans réutilisation possible ;
  - `crible eval` ne peut pas servir de preuve de gain : il faut une campagne serveur de plus, avec son rapport.
- **À surveiller** :
  - le **temps de démarrage** après application : s'il rapproche le budget de 60 s d'ADR-0047, la question d'un remplissage différé, ou d'une ouverture parallèle des segments, se rouvre par un ADR ;
  - le **p50 de `fetch`** des analytics : il doit désormais croître avec `k` et les charges utiles, non avec la taille du corpus ; une croissance avec les octets de vecteurs signalerait une ouverture restée sur le chemin ;
  - la **contention du pool** de charges utiles sous `max_concurrent_searches` élevé : une attente visible dans `fetch`, sans erreur, demanderait un dimensionnement du pool, pas un retour au pool par requête ;
  - **INV-02** et MES-16 rejouée : une régression de latence sous charge dirait que le remplissage ou le partage du pool franchit le verrou de publication ;
  - la **mémoire résidente** : les mappings ne consomment pas de mémoire possédée, mais un nombre de segments en hausse augmente les descripteurs et la table de pages ; à suivre avec le RSS scindé de MES-03 ;
  - le **test des 10 %** en `dense` : sans l'ouverture dans `fetch`, la marge doit s'améliorer ; s'il échoue encore, une autre dépense est sans clé.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Garder le cache d'ADR-0120 tel quel : un `DenseSearcher` par génération | Un patch crée une génération sans segment neuf (`docs/02:137`) : le cache jetterait des lecteurs valides et repaierait 8,3 s pour un changement de métadonnée |
| Ouvrir les lecteurs à la première requête d'une génération, sous le verrou de publication | Il faudrait publier une génération sans ses lecteurs, puis la compléter : c'est le cas non couvert que la décision 4 supprime, et la première requête paierait l'ouverture |
| Ouvrir les lecteurs à la première requête, hors verrou, avec un verrou par génération | Deux requêtes simultanées sur une génération non remplie ouvriraient deux fois, et l'une attendrait ; la règle « une génération servie a ses lecteurs » devient une course |
| Supprimer ou différer la vérification blake3 pour rendre l'ouverture bon marché | C'est un second mode d'ouverture, que le commentaire de `segment.rs` laisse ouvert et qu'ADR-0084, décision 8, n'a pas décidé ; le sceau est le seul contrôle d'intégrité du fichier de vecteurs |
| Vérifier le sceau en tâche de fond après publication | La génération serait servie avant d'être vérifiée ; un segment corrompu produirait des scores faux au lieu d'une erreur, contre INV-03 et INV-04 |
| Mettre le cache dans `crible-search`, ou dans le serveur seulement | `SegmentReader` et les locations d'époque vivent dans `crible-store` : un cache ailleurs dupliquerait la propriété, ou laisserait la CLI avec un autre chemin d'ouverture |
| Un cache global, statique, partagé par les index | Une instance sert plusieurs index (ADR-0003), chacun avec ses époques et ses générations : un cache global confondrait des segments homonymes et ne pourrait pas suivre les publications |
| Faire tenir une `EpochLease` aux lecteurs en cache | La rétention est gouvernée par les instantanés (`reclaim_epochs`, ADR-0065, ADR-0017) ; une location tenue par un cache empêcherait la récupération d'une époque que plus aucune requête ne lit |
| Garder les codes binaires avec les lecteurs | Les codes sont déjà dans l'instantané, en RAM (ADR-0069) ; `codes_words` sur un lecteur en cache doublerait 0,82 Go pour rien |
| Remplir le cache en tenant le verrou d'écriture de `Published` | Le verrou serait tenu pendant 8,3 s au démarrage et à chaque compaction, et les requêtes attendraient : INV-02 exigerait une exception, et le verrou de publication deviendrait un verrou de lecture |
| Garder `fetch` tel quel, avec l'ouverture masquée | ADR-0126 supprime la dépense ; garder la règle provisoire laisserait `fetch` décrire une ouverture qui n'existe plus, et l'histogramme mentirait |
| Mesurer le gain par `crible eval` | Il ouvre un searcher par campagne (ADR-0125, décision 9) : les deux versions y paraissent identiques, et le gain par requête du serveur n'y est pas observable |
| Un pool de charges utiles par requête, seuls les lecteurs en cache | Le `DenseSearcher` ne se composerait plus sans I/O, et chaque requête rouvrirait les connexions de l'époque ; la décision 3 tranche pour un pool par époque, avec la contention écrite comme coût |

## Références

- Fiche : `docs/phases/P7-observabilite.md:128-144` (`REUTILISER-LECTEURS-DENSES`), jalon J7.
- Mesure : `docs/mesures/lecteurs-denses-ouverture-2026-11-01.md` (p50 8 262,9 ms, p95 8 406,9 ms sur `synth-large`, 200 ouvertures, cache chaud), harnais `crates/crible/examples/dense_latency.rs` et `tools/bench/ouvertures-denses/run.sh`.
- Conception : `docs/02-architecture.md` (segments et identifiants `:108`, générations et point de commit `:118-138`, reprise `:142-162`, concurrence `:164-177`, compaction `:179-207`, mémoire `:222-239`), `docs/03-recherche.md:14` (`fetch`), `docs/06-observabilite.md:19` et `:32` (histogramme `fetch`), `docs/08-invariants-et-tests.md:10` (INV-02), `:81` (MES-16), `docs/05-api.md:299` (`took_ms`).
- Code : `crates/crible-search/src/dense/exact.rs:124-190` (`DenseSearcher`, ouverture des lecteurs), `crates/crible-store/src/segment.rs:407` (champ `codes`), `:445-459` (coût du sceau, question laissée ouverte), `:689` (`codes_words`), `:983-1007` (`check_checksum`), `crates/crible-store/src/generation.rs:326-327` (`EpochLease` d'un instantané), `crates/crible-store/src/compact.rs:189-209` (`reclaim_epochs`), `crates/crible-server/src/search.rs:1043-1046` et `:1067` (ouverture rattachée à `fetch`), `crates/crible-server/src/jobs.rs:607-631` (`Published`), `:794-812` (`publish`), `:919-925` (`refresh` après un job), `crates/crible/src/commands.rs:810` et `crates/crible/src/eval.rs:282` (ouverture par processus).
- ADR : ADR-0003 (plusieurs index), ADR-0006 (segments immuables), ADR-0017 (compaction), ADR-0047 (ordre des remèdes au démarrage), ADR-0065 (rétention des époques), ADR-0069 (codes binaires), ADR-0084 (froid, décisions 1 et 8), ADR-0105 (publication conjointe, décision 4), ADR-0108 (réouverture après compaction en échec), ADR-0113 (INV-02 et MES-16), ADR-0115 (identifiants d'une époque compactée), ADR-0119 (objet `debug`, décision 6), **ADR-0120 (décisions 1, 4 et 5 maintenues ; décisions 2 et 3 remplacées ou sans objet)**, ADR-0125 (pourquoi `eval` ne sert pas de preuve).

## Documents mis à jour par cet ADR

| Document | Modification |
|---|---|
| `docs/02-architecture.md`, « Générations et point de commit » | les étapes 6 et le paragraphe du patch publient les lecteurs avec l'instantané, et le remplissage a lieu avant la publication, hors verrou (décisions 3 et 4) |
| `docs/02-architecture.md`, « Reprise au démarrage » | la reprise ouvre les lecteurs de la génération qu'elle publie, après la construction de l'instantané (décisions 1 et 4) |
| `docs/02-architecture.md`, « Concurrence » | un paragraphe sur les lecteurs en cache, leur partage entre requêtes, l'absence d'`EpochLease` et la libération par comptage de références (décisions 5 et 6) |
| `docs/02-architecture.md`, « Compaction » | la fin de compaction publie les lecteurs de l'époque neuve, et `reclaim_epochs` ne voit que les instantanés (décisions 4 et 5) |
| `docs/03-recherche.md:14` | la clé `fetch` redevient la seule lecture des charges utiles (décision 7) |
| `docs/06-observabilite.md:32` | la clé `fetch` ne porte plus l'ouverture : lecture de l'histogramme (décision 7) |
| `docs/phases/P7-observabilite.md:136-143` | note datée : granularité par segment et pool par époque, remplissage avant publication, preuve de latence par le serveur et non par `crible eval` (décisions 1, 4 et 8) |
| `docs/suivi.md`, « Écarts à la conception » | ligne du 2026-11-01 pour `REUTILISER-LECTEURS-DENSES`, citant cet ADR |
| `docs/adr/README.md` | ligne d'index de cet ADR, et mention du remplacement partiel sur la ligne d'ADR-0120 |

Sont **non modifiés** : le corps d'ADR-0120 et de tout autre ADR accepté (seule une note datée de renvoi est ajoutée à l'en-tête d'ADR-0120), `docs/05`, `docs/07`, `docs/08`, et tout fichier de code.

Les numéros de ligne des documents cités sont ceux de leur **état d'avant application** : la mise à jour de ces documents décale les sections suivantes, et c'est la modification ci-dessus qui fait foi.
