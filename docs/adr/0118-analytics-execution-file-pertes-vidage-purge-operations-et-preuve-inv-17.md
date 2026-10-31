# ADR-0118 — Analytics : exécution (file, pertes, vidage, purge, opérations, preuve d'INV-17)

- **Statut** : Accepté (accepté par anticipation par le propriétaire le 2026-10-31)
- **Date** : 2026-10-31
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, tâche AGREGER-ANALYTICS (phase P7), sur les options recommandées par `@architecte` et retenues par l'orchestrateur, en application de la délégation du propriétaire du 2026-10-31 pour la série de P7
- **Remplace** : — (ne remplace aucune décision : complète ADR-0099 et ADR-0104 sur le délai de vidage qu'ils renvoyaient à P7, et reprend la seconde campagne d'ADR-0113, sans modifier le corps d'aucun ADR accepté)

## Contexte

### La fiche et ce qu'elle demande

La fiche `AGREGER-ANALYTICS` (`docs/phases/P7-observabilite.md:32-46`) demande un **vidage asynchrone toutes les 10 s et à l'arrêt**, dans `analytics.sqlite` (`:40`), la **purge selon la rétention** (`:42`), les **lignes journalières par index** (`:41`), et pour critère de l'invariant : « le vidage ne bloque jamais une requête (**INV-17**, file saturée artificiellement) » (`:45`), seul invariant cité (`:46`). Cet ADR traite l'**exécution** ; le **stockage et le calcul** (schéma, seaux, percentile, formes de filtre) sont l'objet d'ADR-0117.

### Ce que la conception pose

`docs/06:47-50` décrit l'écriture : « Chaque requête met à jour un agrégateur en mémoire, avec un verrou court ou un `DashMap` » ; « Une tâche asynchrone vide l'agrégateur dans SQLite toutes les 10 secondes et à l'arrêt » ; « L'écriture ne bloque jamais une requête. Si la file de vidage est pleine, la perte est comptée dans `analytics_dropped` (**INV-17**) » ; « Rétention : `analytics.retention_days`, 90 par défaut. Une purge tourne chaque jour ».

Deux points y restent ouverts pour l'implémentation. Le premier est le **mécanisme** : `DashMap` n'est pas une dépendance de `docs/07:29-50`, et la règle 12 d'`AGENTS.md` demande un ADR pour toute dépendance nouvelle ; un « verrou court » tenu par chaque requête est exactement ce qu'INV-17 interdit de laisser sur le chemin d'une recherche. Le second est le **compteur de pertes** : `docs/06:49` le nomme et INV-17 l'exige, mais rien ne dit où il est exposé — c'est `GET /v1/stats` qui lit les analytics (`docs/05:75`, fiche `ROUTE-STATS`, `docs/phases/P7:94-104`).

Les bornes de l'arrêt viennent d'ailleurs : `docs/07:121` décrit la séquence — `/ready` à `503`, arrêt des nouvelles requêtes, drainage des requêtes en vol pendant au plus `[limits].default_timeout_ms` + 1 000 ms, puis « il vide les analytics quand elles existent (P7) ». ADR-0099 (`:54-66`) écrit la même séquence en cinq étapes et laisse ouvert, à sa ligne 66, « le délai de vidage des analytics, fixé par P7 ». ADR-0104 (`:35`) écrit de même, dans son « Ce qui n'est décidé ici », « le délai de vidage des analytics, qui appartient à P7 ». C'est ce délai que la décision 3 fixe.

ADR-0113, décision 2 (`:41-47`), a sorti INV-17 de P6 et promis une **seconde campagne en P7**, sur le même harnais étendu au journal et aux analytics, parce que `docs/08:25` associe INV-17 aux deux écritures et que seule la première existe en P6. C'est `JOURNAL-DE-DEBUG` (`docs/phases/P7:55-69`) qui livre le second écrivain, et c'est donc cette tâche qui peut porter la campagne. ADR-0113 a aussi posé le précédent de la preuve d'invariant en CI, déterministe, sous la feature `failpoints` du serveur (décision 3, `:49-58`), et la règle que « une latence de runner partagé n'est pas reproductible » (`:94`).

Le mécanisme de retenue existe déjà : `crates/crible-server/src/barriers.rs` (en-tête, `:1-27`) décrit les barrières du build `failpoints`, pilotées par `CRIBLE_TEST_BARRIER`, avec un fichier `ready-<scope>` puis un fichier `go-<scope>`, une attente bornée à 30 s, et l'exigence qu'un build avec la feature mais sans barrière armée se comporte comme un build normal. C'est le seul outil qui permette de remplir une file sans mesurer une latence.

## Décision

### Décision 1 — un canal borné et un seul écrivain, sans dépendance nouvelle

**Décision.** La requête dépose son événement dans un canal **`tokio::mpsc` borné**, par `try_send`, et **un seul écrivain** reçoit, agrège en mémoire et vide. Aucun verrou n'est tenu par une requête, aucune carte concurrente n'est partagée.

Précisions :

- la feature `sync` de `tokio` est déjà activée (`docs/07:31`, complément du 2026-10-29 à ADR-0096) : le canal ne demande **aucune dépendance nouvelle**, et `DashMap` n'entre pas dans `docs/07` ;
- `try_send` ne rend jamais la main en attente : file pleine, il rend `Full` immédiatement (décision 2) ;
- l'écrivain est **unique** : il n'y a donc pas de course entre deux agrégateurs, pas de fusion de cartes, et l'ordre des additions de compteurs est sans importance ;
- l'agrégateur en mémoire est la propriété de l'écrivain seul ; aucune structure partagée n'existe entre les requêtes et lui, ce qui rend INV-17 vrai **par construction** et pas seulement par mesure ;
- la variante « verrou court » de `docs/06:47` est écartée : un verrou, même court, est pris sur le chemin d'une recherche, et une requête qui attend un verrou d'analytics attend une écriture, ce qu'INV-17 interdit.

### Décision 2 — taille fixe 4 096, perte comptée dans `analytics_dropped`

**Décision.** La capacité du canal est la constante **4 096 événements**, sans clé de configuration (règle 12 : pas de configuration pour une valeur fixe). Quand la file est pleine, **l'événement neuf est perdu** et incrémente un compteur `AtomicU64` **`analytics_dropped`**, par processus.

Précisions :

- c'est la sémantique naturelle de `try_send` : l'événement refusé est le plus récent, les plus anciens sont déjà dans la file et seront écrits. Perdre le neuf plutôt que d'écraser l'ancien évite de réordonner les minutes ;
- `analytics_dropped` est **exposé par `GET /v1/stats`** (fiche `ROUTE-STATS`, `docs/phases/P7:94-104`) sous ce nom exact. La forme de la réponse et le reste de son contenu relèvent de ROUTE-STATS ; cet ADR ne fixe que le nom et la présence du compteur ;
- à **chaque vidage où le compteur a augmenté**, l'écrivain journalise une ligne `warn` avec le nombre de pertes depuis le vidage précédent et le total depuis le démarrage. Une perte silencieuse serait un trou dans les chiffres que personne ne verrait ;
- le compteur est **par processus**, jamais persisté : il mesure l'état d'une instance en service, pas une histoire. Un redémarrage le remet à zéro, et c'est cohérent avec une file qui ne survit pas non plus ;
- 4 096 événements couvrent, à 400 requêtes par seconde, dix secondes de trafic, soit un intervalle de vidage entier : une file pleine signale un écrivain bloqué (disque lent, verrou SQLite), pas un pic de trafic normal, et un débit supérieur à 400 requêtes par seconde sur une instance Legifacile est déjà un incident en soi.

### Décision 3 — vidage toutes les 10 s et à l'arrêt, borne de 2 s

**Décision.** L'écrivain vide la file dans `analytics.sqlite` **toutes les 10 secondes** (`docs/06:48`) et **à l'arrêt**.

Précisions :

- à l'arrêt, la séquence est celle de `docs/07:121` et d'ADR-0099 : `/ready` à `503`, arrêt des nouvelles requêtes, **drainage des requêtes en vol**, puis **vidage des analytics**. La file est fermée avant le vidage, donc plus aucun événement n'arrive : le vidage écrit tout ce qui reste, puis l'écrivain s'arrête ;
- le vidage d'arrêt, écriture comprise, est **borné à 2 secondes**, par `tokio::time::timeout` ; les écritures périodiques, elles, ne sont pas bornées, puisque rien n'attend leur fin ;
- un échec de vidage est journalisé en `warn` avec l'erreur et le nombre d'événements perdus, **sans reprise** : les événements de ce vidage sont perdus, et l'instance continue. Une reprise en boucle sur un disque plein empêcherait l'arrêt et remplirait le journal ;
- cette perte **n'incrémente pas** `analytics_dropped`, qui compte les refus de file (décision 2) : mêler deux causes dans un compteur le rendrait inexploitable. Le nombre perdu est dans la ligne `warn` ;
- un vidage est **une transaction** : un échec au milieu ne laisse pas la moitié d'une minute écrite. Les lignes touchées sont lues, additionnées seau par seau en `u32` saturant, puis réécrites, parce que SQLite ne sait pas additionner deux BLOB ;
- la minute de rattachement d'un vidage est obtenue **par SQLite** (`strftime('%Y-%m-%dT%H:%M:00Z','now')`), pas par le code : c'est le précédent de `jobs.sqlite` (`crates/crible-store/src/jobs.rs:22-31` : « the code owns no clock »), et aucune crate d'horloge calendaire n'existe dans `docs/07`. Conséquence assumée : un événement arrivé dans les 10 s qui précèdent un vidage est rattaché à la minute du vidage. Le décalage est borné par l'intervalle de vidage et ne fausse aucune somme sur une période, seule la frontière entre deux minutes peut glisser de moins de 10 s. Horodater chaque événement demanderait une horloge dans le code, donc une dépendance nouvelle, pour un gain que la lecture ne demande pas ;
- ce délai de 2 s **complète** ADR-0099 (`:66`) et ADR-0104 (`:35`), qui renvoyaient tous deux ce chiffre à P7 : il n'est pas ajouté à leur corps, et la séquence d'arrêt de `docs/07:121` reste la leur.

### Décision 4 — purge de la rétention, sans minuteur dédié

**Décision.** La purge applique `analytics.retention_days` (90 jours par défaut, `docs/07:86-87`). Elle tourne **à l'ouverture**, puis **à chaque vidage si la dernière purge date de plus de 24 heures**.

Précisions :

- aucun minuteur n'est créé : la tâche de vidage est déjà périodique, et lui adjoindre une purge conditionnelle coûte une comparaison par vidage ;
- le motif est celui de `jobs.sqlite` : `crates/crible-server/src/jobs.rs:254` purge à l'ouverture, `:463` purge à chaque fin de job, et `crates/crible-store/src/jobs.rs:449` porte la requête. La différence est la **condition de 24 h**, qui remplace le rythme des jobs : une instance au repos ne purge pas toutes les dix secondes une table qu'elle ne remplit pas ;
- la purge supprime les lignes de `minute_events`, `filter_shapes` et `day_indexes` plus anciennes que la rétention. Un échec est journalisé en `warn` et n'empêche ni les vidages suivants ni l'arrêt ;
- le seuil de 24 h est une **constante**, pas une clé : `docs/07:86-87` ne déclare que `retention_days`.

### Décision 5 — lignes journalières au premier vidage après minuit UTC

**Décision.** Les lignes de `day_indexes` (documents, passages vivants et morts, taille sur disque, génération, nombre de lots) sont calculées **au premier vidage qui suit minuit UTC**, une ligne par index. La règle exacte est : **à chaque vidage, une ligne est écrite pour chaque index dont la ligne du jour UTC courant manque**. En marche continue, cela tombe au premier vidage après minuit ; après un redémarrage à 10 h, cela tombe au premier vidage qui suit le démarrage.

Précisions :

- le jour est obtenu par SQLite (`strftime('%Y-%m-%d','now')`), comme la minute (décision 3) : une seule règle d'horloge pour tout le fichier ;
- c'est un **instantané de l'état de l'index à cet instant**, pas une moyenne de la journée : la table dit « où en était chaque index le 31 octobre », ce qui suffit pour lire une croissance et ne demande aucun calcul incrémental ;
- la clé `(day, "index")` rend l'écriture **idempotente** : deux redémarrages ou deux instances le même jour écrivent la même ligne, la dernière valeur l'emportant, et aucune ligne n'est dupliquée ;
- une instance arrêtée pendant minuit ne rattrape rien pour les jours où elle ne servait pas : ces jours n'ont pas de ligne. Un trou dit la vérité, et `day_indexes` n'a pas à inventer un état ;
- le calcul lit l'état des index, ce que `crible-server` sait déjà faire pour ses autres routes ; `crible-store` ne fait qu'écrire la ligne.

### Décision 6 — `OpEvent` : une liste fermée de routes, une clé résolue

**Décision.** Un `OpEvent` n'est produit que par les **gestionnaires des routes** dont l'opération figure dans la liste close de `docs/06:26`, et seulement quand une **clé d'API a été résolue**.

Précisions :

- les opérations sont exactement `search`, `search_multi`, `get_document`, `online_batch`, `attach_batch`, `patch_documents`, `compact`, `export`, `snapshot`, `index_admin`. `index_admin` couvre les routes d'administration des index de `docs/05` : création, `PATCH`, `DELETE` et les deux `GET` d'index. Aucune autre opération n'existe, et un gestionnaire qui n'en porte pas ne produit rien ;
- une réponse `400`, `404` ou `409` **produit un événement** : une requête refusée est une opération tentée, et le statut est justement là pour la compter (`docs/06:26` : le statut est `ok` ou un code d'erreur de l'API). Une erreur est un fait d'usage, pas une absence d'usage ;
- `401` et `403` ne produisent **rien** : pour `401`, aucune clé n'est résolue, donc la dimension `client` manquerait et l'événement serait inclassable ; pour `403`, l'opération n'a pas été exécutée, et compter un refus d'autorisation comme un appel d'opération gonflerait le trafic de chaque client mal configuré en le faisant passer pour un usage ;
- `/health`, `/ready` et `/v1/jobs` sont exclus : ce sont des sondes et des lectures d'état, non des opérations du produit, et `docs/06:26` ne les liste pas. Les sondes d'un orchestrateur sont nombreuses et régulières ; les compter écraserait le signal du trafic réel ;
- la CLI ne produit **aucun** `OpEvent` : elle n'a ni clé d'API, ni instance, et ses analytics sont celles du serveur qu'elle interroge ;
- la dimension `client` est le **nom** de la clé (ADR-0100, `crates/crible-server/src/config.rs:140`), jamais la clé, son haché ni l'en-tête `Authorization`. Une clé tournée change de nom ou disparaît de la configuration ; son haché n'a rien à faire dans une table conservée 90 jours.

### Décision 7 — preuve d'INV-17 : barrière, file pleine, pertes comptées

**Décision.** INV-17 est prouvé par un test déterministe de `crible-server`, nommé `inv_17_<description>`, sous la feature `failpoints`, sur le modèle de la décision 3 d'ADR-0113 (`:49-58`) et du protocole de `crates/crible-server/src/barriers.rs`.

**Protocole du test.**

1. Le test arme une barrière, par les variables et les fichiers d'`barriers.rs` (en-tête `:1-27`), à un point **en tête de la boucle de l'écrivain, avant qu'il ne reçoive** : tant qu'elle n'est pas relâchée, l'écrivain n'enlève rien de la file et n'écrit rien.
2. Le test envoie une recherche de référence, dont il garde le corps.
3. Il envoie `QUEUE_CAPACITY + m` recherches. Les `QUEUE_CAPACITY` premières sont acceptées, les `m` suivantes sont refusées par `try_send` et comptées.
4. Assertions, toutes indépendantes du temps :
   - **chaque recherche rend `200`**, et son corps est **identique** à celui de la référence : mêmes clés, même ordre, mêmes scores, même `generation`. Les analytics ne changent pas une réponse ;
   - `analytics_dropped` a monté de **exactement `m`** : ni plus (aucun événement perdu ailleurs), ni moins (aucun refus silencieux) ;
   - le test relâche la barrière, attend le vidage, puis lit `analytics.sqlite` : les agrégats valent exactement les `QUEUE_CAPACITY` événements acceptés, comptés par une seule ligne de minute, avec le total des `count` attendu ;
   - une perte annoncée par `analytics_dropped` **ne doit pas** apparaître dans les agrégats : les deux mesures se contrôlent l'une l'autre.
5. Aucun seuil de latence n'entre dans le test : la latence d'un runner n'est pas reproductible (ADR-0113, `:94`). Le test prouve que la recherche **se termine** et rend la même réponse quand l'écriture est retenue, pas qu'elle est rapide.

**Précisions.**

- la barrière a un **domaine propre**, par exemple `analytics-flush`, et suit le protocole existant : `ready-<scope>`, `go-<scope>`, attente bornée, aucune barrière armée = comportement normal ;
- `QUEUE_CAPACITY` est la constante de la décision 2, lue par le test depuis le module, jamais recopiée : un test qui recopie 4 096 cesse de prouver la bonne chose au premier changement ;
- le test vit dans `crates/crible-server/tests/`, sous `cfg(feature = "failpoints")`, et rejoint l'étape « Test the server failpoint build » de la CI (ADR-0068, complément du 2026-10-29) ;
- le test vérifie aussi qu'**aucun** marqueur de texte ni de valeur de filtre n'entre dans le fichier, par la même occasion : c'est un contrôle d'INV-13 à coût nul, sans remplacer le test dédié de `TESTER-ANALYTICS-SANS-CONTENU`.

### Décision 8 — seconde campagne MES-16 portée par JOURNAL-DE-DEBUG

**Décision.** La seconde campagne MES-16 qu'ADR-0113 a promise pour P7 est portée par la tâche **JOURNAL-DE-DEBUG** (`docs/phases/P7:55-69`), et se déroule **après** le `@dev` de cette tâche.

Précisions :

- la campagne ne peut avoir lieu qu'une fois les analytics **et** le journal de debug écrits tous deux de façon asynchrone : `docs/08:25` nomme les deux, et mesurer avec un seul écrivain actif ne prouverait rien sur l'autre ;
- `@mesureur` rejoue MES-16 (protocole d'ADR-0113, décision 5) avec **les deux actifs** : `[analytics]` à ses défauts et `[debug_journal].enabled = true` avec un `sample_rate` qui garantisse des écritures pendant la charge, ce taux étant rapporté dans le rapport. La règle 7 d'`AGENTS.md` réserve la mesure à `@mesureur` ;
- le test `inv_17_<description>` du journal de debug est écrit par JOURNAL-DE-DEBUG, **sur le même modèle** que la décision 7 : même barrière sur l'écrivain du journal, même file pleine, mêmes assertions sur les réponses et sur le compteur de pertes ;
- le rapport vient à côté de celui de P6, sous son propre nom (`docs/mesures/MES-16-charge-concurrente-analytics-et-journal.md`), avec les données brutes en JSON, et cite la campagne de P6 comme référence ;
- aucune dépendance ne change : le harnais de `tools/bench/mes16/` existe (ADR-0113, décision 7) et se contente de tourner avec les deux écrivains actifs.

## Conséquences

- **Positives** :
  - INV-17 est vrai **par construction** : aucune requête ne partage une structure avec l'écrivain, et une file pleine rend la main immédiatement ;
  - les pertes sont **visibles** : un compteur exposé par l'API, une ligne `warn` à chaque augmentation, et un test qui compare le compteur aux agrégats ;
  - le délai laissé ouvert par ADR-0099 (`:66`) et ADR-0104 (`:35`) est fixé : 2 secondes, bornées, après le drainage des requêtes ;
  - aucune dépendance nouvelle : ni `DashMap`, ni crate d'horloge, ni clé de configuration ;
  - la purge ne crée aucun minuteur et ne tourne pas à vide sur une instance au repos ;
  - la preuve d'INV-17 est déterministe et tourne en CI, sur le modèle d'ADR-0113, au lieu de dépendre d'une machine et d'un chiffre de latence ;
  - la seconde campagne de MES-16 a un porteur nommé et un critère écrit, au lieu d'être une promesse d'ADR-0113 sans titulaire.
- **Négatives / coûts** :
  - au-delà de 4 096 événements en attente, des événements sont perdus : les analytics d'une minute sous écriture bloquée sont incomplètes, et `analytics_dropped` le dit mais ne les reconstitue pas ;
  - la borne de 2 s à l'arrêt peut être dépassée par un disque lent : le processus sort alors avec les événements non écrits perdus, et le journal le dit ;
  - la minute de rattachement est celle du vidage : jusqu'à 10 s de trafic basculent d'une minute à l'autre, ce qui interdit de lire une frontière de minute à la seconde près ;
  - les pertes d'un vidage en échec ne sont pas dans `analytics_dropped` : deux causes de perte, deux endroits pour les lire ;
  - les lignes journalières manquent pour les jours où l'instance était arrêtée à minuit UTC ;
  - la campagne MES-16 est rejouée une seconde fois, avec deux écrivains actifs, ce qui allonge la validation de P7.
- **À surveiller** :
  - la première ligne `warn` d'`analytics_dropped` en exploitation : elle dit un écrivain bloqué (disque, verrou SQLite) et doit être traitée comme un incident, pas comme du bruit ;
  - la durée du vidage d'arrêt mesurée en P8 dans le conteneur : si les 2 secondes sont régulièrement atteintes, c'est l'écriture qui est en cause, et le remède est un ADR, pas une borne allongée en silence ;
  - le décalage de frontière de minute : si une lecture par minute devient un besoin de produit, il faudra une horloge dans le code, donc une dépendance et un ADR ;
  - le résultat de la campagne de JOURNAL-DE-DEBUG : un dépassement du critère d'ADR-0113 avec les deux écrivains actifs rouvre la question du mécanisme d'écriture, et non le critère ;
  - `docs/08:36` garde la formulation « INV-17 sera prouvé en P7, avec le même harnais » : elle reste vraie pour la campagne, et le test de barrière est une preuve de plus. Aucune réécriture de `docs/08` n'est faite ici.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Un verrou court tenu par chaque requête sur un agrégateur partagé (`docs/06:47`) | Une requête attendrait une écriture dès qu'une autre tient le verrou : c'est exactement ce qu'INV-17 interdit, et le verrou serait tenu sur le chemin de toutes les recherches pour un compteur. |
| Une `DashMap` en accès concurrent | Dépendance absente de `docs/07:29-50`, donc ADR et revue de dépendance pour une structure que le canal remplace ; et un agrégateur partagé garde une course sur les additions de BLOB qu'il faudrait arbitrer. |
| Une file non bornée | Une file non bornée ne perd rien et **consomme toute la mémoire** quand l'écrivain est bloqué ; à la limite, le processus est tué par l'orchestrateur et toutes les données non écrites sont perdues, sans compteur pour le dire. |
| Une file bornée plus grande, ou réglable par configuration | 4 096 couvre déjà des dizaines de secondes de trafic ; une taille réglable est une valeur de plus à documenter et à justifier (règle 12), et un exploitant qui l'augmente masque un écrivain bloqué au lieu de le voir. |
| Écraser l'événement le plus ancien quand la file est pleine | `try_send` refuse le neuf sans coût ; écraser l'ancien demanderait une structure de plus et réordonnerait les minutes, donc fausserait les comptes par minute dans un sens que rien ne signalerait. |
| Compter les pertes de file dans les agrégats, avec un statut à part | Une ligne « perdue » dans `analytics.sqlite` n'est pas une ligne de trafic : elle n'a ni durée, ni pool, ni index. Un compteur exposé et journalisé dit la même chose sans polluer les compteurs de recherche. |
| Persister `analytics_dropped` dans le fichier | Le compteur mesure l'état d'une instance en service ; persisté, il mélangerait les vies d'un processus et ne serait plus comparable à une file qui, elle, ne survit pas au redémarrage. |
| Écrire le compteur dans les logs seulement | `ROUTE-STATS` lit les analytics par l'API (`docs/05:75`) ; un chiffre qui ne serait que dans les logs demanderait un accès au conteneur pour la question la plus simple : « est-ce que je perds des événements ? ». |
| Reprendre un vidage en échec, en boucle, jusqu'au succès | Une boucle sur un disque plein remplit le journal et empêche l'arrêt ; l'information perdue est bornée et déjà annoncée, alors qu'un arrêt bloqué ne l'est pas. |
| Une borne plus longue pour le vidage d'arrêt, ou aucune borne | Sans borne, un disque lent bloque l'arrêt jusqu'au `stop_grace_period` de l'orchestrateur, qui tue le processus : le résultat serait une sortie brutale, sans journal. 2 s laisse la place à une écriture normale et rend la main sinon. |
| Un minuteur dédié à la purge | Une tâche de plus pour une table qu'une instance au repos ne remplit pas ; la purge au vidage, conditionnée à 24 h, ne coûte qu'une comparaison. |
| Purger à chaque vidage sans condition de 24 h | Une requête `DELETE` sur trois tables toutes les dix secondes, sur une table dont la plus vieille ligne a 90 jours : du travail pour rien, et un verrou d'écriture de plus. |
| Horodater chaque événement par une crate d'horloge | Dépendance nouvelle (`docs/07:29-50` n'en a pas), pour corriger un glissement de frontière de minute inférieur à l'intervalle de vidage, qu'aucune lecture de `docs/06:54-59` ne demande. |
| Compter les `401` et les `403` comme des opérations | Un `401` n'a pas de clé résolue, donc pas de dimension `client` ; un `403` n'a exécuté aucune opération. Les compter gonflerait le trafic d'un client mal configuré et masquerait le trafic réel. |
| Compter `/health`, `/ready`, `/v1/jobs` | Sondes et lectures d'état, absentes de la liste close de `docs/06:26`, et émises à un rythme qui écraserait le signal du trafic. |
| Prouver INV-17 par une mesure de latence de recherche sous charge | La latence d'un runner n'est pas reproductible (ADR-0113, `:94`) et une attente de quelques millisecondes dans un vidage ne se distinguerait pas du bruit ; la barrière rend le test déterministe. |
| Prouver INV-17 au niveau du journal seulement | INV-17 couvre les deux écritures (`docs/08:25`) ; un seul écrivain prouvé laisserait l'autre sans preuve, et c'est justement l'objet de la décision 8. |
| Mesurer la seconde campagne MES-16 dans AGREGER-ANALYTICS | Le journal de debug n'existe pas encore à ce moment : la campagne doit suivre le second écrivain, donc appartenir à JOURNAL-DE-DEBUG, comme ADR-0113 l'avait prévu en renvoyant la preuve au moment où les deux existeraient. |

## Références

- `docs/phases/P7-observabilite.md` : `:32-46` (fiche `AGREGER-ANALYTICS` ; `:40` vidage, `:42` purge, `:45` INV-17, `:46` invariant cité), `:48-53` (`TESTER-ANALYTICS-SANS-CONTENU`), `:55-69` (fiche `JOURNAL-DE-DEBUG`), `:94-104` (fiche `ROUTE-STATS`).
- `docs/06-observabilite.md` : `:30-37` (INV-13, sûreté par construction), `:45-50` (écriture : verrou ou `DashMap`, vidage 10 s et à l'arrêt, INV-17 et `analytics_dropped`, rétention de 90 jours), `:54-59` (ce que lit `GET /v1/stats`), `:74` (écriture asynchrone du journal de debug).
- `docs/07-stack-et-deploiement.md` : `:29-50` (dépendances, `tokio` feature `sync` ligne 31 ; aucune `DashMap`), `:86-87` (`[analytics].retention_days = 90`), `:89-92` (`[debug_journal]`), `:121` (séquence d'arrêt, vidage des analytics).
- `docs/08-invariants-et-tests.md` : `:25` (INV-17), `:36` (niveau « Charge », INV-02 et INV-17).
- `docs/05-api.md:75` (`GET /v1/stats`, rôle `admin`), `:41-53` (codes d'erreur, catégories des analytics).
- `crates/crible-server/src/barriers.rs` (en-tête `:1-27` : `CRIBLE_TEST_BARRIER`, `ready-<scope>` et `go-<scope>`, attente bornée, build normal sans barrière armée), `crates/crible-server/src/jobs.rs:254` et `:463` (purge à l'ouverture et après une transition terminale), `crates/crible-server/src/config.rs:140` (nom de la clé = dimension `client`), `crates/crible-store/src/jobs.rs:22-31` (le code ne porte pas d'horloge), `crates/crible-store/src/jobs.rs:449` (purge SQL), `crates/crible-server/Cargo.toml:51-57` (feature `failpoints`, éteinte par défaut).
- ADR-0014 (analytics sans contenu), ADR-0038 (`busy_timeout`), ADR-0068 et son complément du 2026-10-29 (features `failpoints` en CI, `crible-server` comprise), ADR-0096 (features `tokio`, dont `sync`), ADR-0099 (`:54-66`, arrêt propre, délai de vidage renvoyé à P7), ADR-0100 (nom de la clé), ADR-0103 (`jobs.sqlite`, purge opportuniste), ADR-0104 (`:35`, même renvoi), ADR-0113 (`:41-47` INV-17 hors de P6 et seconde campagne, `:49-58` preuve sous `failpoints`, `:60-78` protocole MES-16, `:94` latence de runner), ADR-0117 (stockage et calcul).

## Documents mis à jour par cet ADR

| Document | Modification |
|---|---|
| `docs/06-observabilite.md:45-50` | file bornée de 4 096 événements, perte comptée dans `analytics_dropped` et exposée par `GET /v1/stats`, vidage à l'arrêt borné à 2 s, purge à l'ouverture puis toutes les 24 h (ADR-0118, décisions 1 à 4) |
| `docs/06-observabilite.md:28` | ligne journalière calculée au premier vidage après minuit UTC (décision 5) |
| `docs/07-stack-et-deploiement.md:121` | la séquence d'arrêt dit que le vidage des analytics est borné à 2 s, après le drainage des requêtes (décision 3) |
| `docs/phases/P7-observabilite.md:32-46` | la fiche `AGREGER-ANALYTICS` renvoie à ADR-0118 pour la file, les pertes, le vidage et la preuve d'INV-17 |
| `docs/phases/P7-observabilite.md:55-69` | la fiche `JOURNAL-DE-DEBUG` reçoit le livrable « rejeu de MES-16 avec analytics et journal actifs par `@mesureur` » (ADR-0118, décision 8) et le test `inv_17` du journal |
| `docs/adr/README.md` | ligne d'index de cet ADR |

Sont **non modifiés** : `docs/08` (sa ligne « Charge » reste vraie), les ADR-0099, ADR-0104 et ADR-0113 (complétés par renvoi, corps inchangé ; le dépôt n'a pas de ligne « Complété par » en tête d'ADR, donc aucune n'est ajoutée), `docs/suivi.md` (tenu par l'orchestrateur), et tout fichier de code à ce stade : le code est livré par AGREGER-ANALYTICS et JOURNAL-DE-DEBUG.

Les numéros de ligne de `docs/06-observabilite.md` cités dans cet ADR sont ceux de son **état d'avant application** : la mise à jour de ce document décale les sections suivantes, et c'est la modification ci-dessus qui fait foi.

