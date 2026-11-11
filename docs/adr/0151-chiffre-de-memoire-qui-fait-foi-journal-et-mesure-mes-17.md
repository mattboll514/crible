# ADR-0151 — Chiffre de mémoire qui fait foi, journal de mémoire du serveur et mesure MES-17

- **Statut** : Accepté (accepté par anticipation par le propriétaire le 2026-11-11)
- **Date** : 2026-11-11 (proposition et acceptation le même jour)
- **Décideurs** : propriétaire
- **Proposé par** : greffier, sur demande de l'orchestrateur, tâche `T05-MEMOIRE-NON-RENDUE` (v1.1)
- **Remplace** : —

## Contexte

### Le constat qui déclenche la tâche

Le ticket 05 de `tickets-crible` (projet Legifacile, lecture seule,
`/Users/mattboll/MattCorp/legifacile/tickets-crible/05-memoire-non-rendue.md`) constate que la
mémoire du conteneur ne redescend ni après une campagne d'attache, ni après la suppression des index
jetables, et qu'un OOM pendant une attache `decisions` reste inexpliqué. Les chiffres du ticket sont
ceux de **Legifacile** : **ADR-0026 de Legifacile** (budget mémoire de l'attache quotidienne,
`/Users/mattboll/MattCorp/legifacile/docs/mesures/ADR-0026-memoire-attache-quotidienne.md`) et
**MES-02 de Legifacile** (`MES-02-encodage-quotidien.md`). Leurs numéros recouvrent ceux de Crible et
ne désignent pas les mêmes documents : 3,591 GiB au départ, 4,698 GiB de pic, 5,049 GiB de
`memory.current` après la commande, 6,17 GiB au repos après la campagne MES-02 de Legifacile, et
3,61 GiB après un redémarrage relevé en session le 2026-11-10, non archivé. Le budget de **Legifacile
ADR-0026** est vérifié avec `docker stats`, donc cache de pages compris, et ce même rapport relève
que `docker stats --no-stream` coûte environ 1,5 s par appel.

### Ce que Crible a déjà vu

- **MES-09 de Crible** (`docs/mesures/MES-09-ram-reelle.md`, campagne en conteneur du 2026-11-06,
  vers 433-446) a déjà observé, à l'échelle de Legifacile importée, « un delta de commit [qui] reste
  en mémoire anonyme après une attache (10,55 GiB), et ne retombe qu'au redémarrage ». La campagne
  part donc chaque fois d'un serveur frais.
- **ADR-0077, décision 5** fixe le RSS en trois états — index ouvert, après une passe de requêtes,
  session fp32 à part — et le scinde en mémoire anonyme et mémoire adossée à des fichiers. La
  session est par instance (`docs/02:217-219`).
- **ADR-0135, décision 5** demande de lire `memory.stat` aux états (b) et (c) d'ADR-0077, scindé en
  anonyme et en fichiers ; sa **décision 7** publie `memory.current` et `memory.stat` avec le RSS, et
  mesure la taille de page.
- **`docs/02`** (vers 226-243) écrit la règle de mémoire : ce qui est parcouru en entier à chaque
  requête est en RAM, ce qui est lu pour quelques candidats reste en `mmap`, donc en pages
  évinçables ; les lecteurs denses en cache (ADR-0126) n'ajoutent que des mappings.
- **`docs/06`** n'a **aucune** section mémoire : il s'arrête à « Santé et logs ».
- **`docs/08`** porte les mesures jusqu'à MES-16 : MES-17 n'existe pas.
- **`crates/crible-server/src/mounts.rs`** donne le précédent d'une lecture de `/proc` par
  `std::fs`, sans dépendance, sous Linux seulement, avec un `None` silencieux quand le fichier
  manque.

### Ce qui manque, et pourquoi il faut trancher avant de coder

Aucun document ne dit lequel de `RssAnon`, de `memory.current` ou de `docker stats` fait foi. Un
exploitant ne peut donc pas dire s'il lui reste de la marge ou s'il approche de l'OOM. Le ticket
demande trois choses : un moyen d'observer la ventilation, un comportement documenté à la
suppression d'un index, et une règle d'exploitation qui dise quel chiffre compte pour un budget
mémoire.

Le verdict de `@architecte` du 2026-11-11 tranche la portée : **`T05-MEMOIRE-NON-RENDUE`
instrumente et mesure, sans corriger.** Aucune correction n'est décidée ici, et aucune ne peut l'être
avant une mesure.

## Décision

**La tâche `T05-MEMOIRE-NON-RENDUE` fixe le chiffre de mémoire qui fait foi, ajoute une ligne de
journal de mémoire au serveur, sous Linux seulement, et définit la mesure MES-17 qui dira quelle
classe de mémoire grandit, à quelle étape, et si la suppression d'un index ou le redémarrage la
rend. Elle ne corrige rien.**

### 1. Chiffre qui fait foi et ventilation

Trois chiffres sont en présence et ils ne mesurent pas la même chose. La règle est la suivante.

- **`memory.current` du cgroup v2 fait foi pour la limite du conteneur et pour l'OOM.** C'est le
  compteur que le noyau compare à `memory.max`, donc le seul dont le dépassement tue le processus. Il
  est **ventilé par `memory.stat`** : `anon` pour la mémoire anonyme, `file` pour le cache de pages,
  le reste (slab, piles noyau, tables de pages) restant non ventilé et déclaré comme tel. Sa source
  est `/sys/fs/cgroup/memory.current`, ou `memory.current` du cgroup du processus quand le conteneur
  n'est pas à la racine.
- **`RssAnon` du processus fait foi pour la mémoire possédée.** Source : `/proc/<pid>/status`, champ
  `RssAnon`, complété par `VmRSS`, `RssFile` et `RssShmem`. C'est le chiffre qui répond à « qu'est-ce
  que Crible garde pour lui », et celui que MES-09 a vu croître après une attache.
- **`RssFile` et `file` du cgroup sont le cache évinçable des `mmap`.** Ils peuvent monter sans
  faute : sous pression, le noyau les réclame. Un chiffre élevé de ce côté n'est pas une fuite et se
  déclare comme récupérable.
- **`docker stats` ne fait pas foi.** Il affiche le même compteur de cgroup avec le cache compris, et
  son échantillonnage coûte environ 1,5 s par appel (rapport de Legifacile ADR-0026). Quand il
  s'écarte de `memory.current`, c'est `memory.current` qui a raison.
- **La session ONNX ne se sépare que par différence au chargement.** Aucun compteur par session
  n'existe : `crible-encode` tient la `Session` sous un `Mutex` (`crates/crible-encode/src/model.rs`)
  et n'en publie rien. La part du modèle se lit donc en comparant deux états, l'un avant le
  chargement, l'autre après, ce qu'ADR-0077, décision 5, appelle déjà l'état (c).
- **Hors Linux, aucun de ces chiffres n'existe.** Il n'y a ni cgroup v2 ni `/proc` au sens du noyau
  Linux sous macOS : la règle vaut dans le conteneur, et une mesure native ne se compare pas à elle.

### 2. Ligne de journal du serveur

Le serveur émet **une ligne `tracing` de niveau `info`**, à nombres seuls, sous Linux seulement, dont
le message est `memory`.

- **Sources, lues par `std::fs`, sans dépendance nouvelle** : `/proc/self/status` pour `RssAnon`,
  `VmRSS` et `RssFile` ; `/sys/fs/cgroup/memory.current` et
  `/sys/fs/cgroup/memory.stat` pour le cgroup, avec le repli sur les fichiers du cgroup du processus.
  Le module suit le précédent de `mounts.rs` : lecture du fichier comme du texte, analyse sur place,
  aucune erreur remontée.
- **Champs**, tous numériques et en octets : `rss_bytes`, `rss_anon_bytes`, `rss_file_bytes`,
  `cgroup_current_bytes`, `cgroup_anon_bytes`, `cgroup_file_bytes`. S'y ajoutent, selon le point
  d'émission, `job_id`, `kind`, `index` et `step`, sous les noms et formes qu'ADR-0104 et ADR-0149
  emploient déjà pour `job_id`, `kind` et `index`.
- **`step` prend exactement quatre valeurs** : `job_end` à la fin de chaque job, quel qu'il soit ;
  `delete_before_rename` dans `delete_index`, juste avant le renommage du dossier vers la
  quarantaine ; `delete_after_removal` après la suppression du dossier en quarantaine ;
  `session_load` au chargement d'une session ONNX, `index` nommant alors l'index qui a déclenché le
  chargement.
- **Une source illisible fait un champ absent, jamais une erreur, jamais un zéro.** Un `0` serait un
  état inventé, comme ADR-0119, décision 3, le refuse pour une réponse d'API. Aucun démarrage, aucune
  requête et aucun job n'échoue à cause de cette ligne.
- **Hors Linux, aucune ligne n'est émise.** Le module est compilé sous `#[cfg(target_os = "linux")]`,
  avec un remplaçant qui ne fait rien ailleurs.
- **Confidentialité** : la ligne ne porte que des nombres, un identifiant de job, un genre de job, un
  nom d'index et une étape. Aucun texte de requête, aucune valeur de filtre, aucun identifiant
  interne, à aucun niveau de log (INV-13, ADR-0096).

### 3. Mesure MES-17, à ajouter à `docs/08`

MES-17 mesure la mémoire d'un conteneur Crible dans le temps, pour attribuer la croissance observée à
une classe et à une étape. Son entrée est ajoutée au tableau des mesures de `docs/08`, sur le modèle
des précédentes.

- **Lieu** : la machine de développement, MacBook Air M3, Docker Desktop, selon ADR-0135 —
  `/data` en **volume nommé** (décision 3), taille de page mesurée et déclarée (décision 7), aucune
  limite `--memory` posée sur le conteneur, la RAM de la VM faisant la borne (décision 5).
- **Données** : **index jetables seulement**, et **la production n'est jamais touchée** (ADR-0032).
  Le jeu est l'**échantillon Legifacile**, `data/legifacile-sample` : six lots réels, **205 261
  documents** et **257 626 passages**, **515 Mo** sur disque, avec le texte, les vecteurs fp16
  recopiés de la production et **300 requêtes** réelles, produit par la tâche
  `CONSTITUER-ECHANTILLON-LEGIFACILE` et décrit par son `README.md` (`docs/08` le nomme `n`). Trois
  raisons le retiennent contre `synth-large`. **Disque** : `synth-large` n'est plus dans `data/` ; le
  régénérer demande environ 13 Go de lots par `tools/gen_synth.py --large`, puis de l'ordre de 15 à
  17 Go d'index — MES-07 de Crible mesure 15,25 Go et 8 segments pour `synth-large` —, plus la copie
  dans le volume nommé du conteneur, alors que la machine de développement n'a que **60 Gio libres**
  le 2026-11-11 : le risque de remplir le disque pendant la mesure est réel. **Pertinence** : l'OOM
  du ticket a lieu pendant une attache, et c'est l'attache d'un lot à texte qui charge le modèle et
  encode la recette par ONNX (ADR-0053, `docs/04`) ; l'échantillon a du texte et le modèle réel de
  `data/models`, alors que `synth-large` n'a que des vecteurs et le lexical désactivé. L'échantillon
  permet donc de séparer la session ONNX par différence au chargement, comme la décision 1 le
  demande, et de voir la classe qui grandit à l'attache. **Limite assumée** : à la taille de
  l'échantillon, les volumes absolus sont petits ; le rapport conclut sur les **classes** de mémoire
  et leur évolution entre états, pas sur les volumes de production. La campagne en conteneur de
  MES-09 (import Legifacile complet, 9 600 105 passages sur six index, volume `crible-p10-data`) reste
  un point de comparaison **déclaré**, jamais rejouée.
- **Campagne d'attache** : plusieurs lots tirés de l'échantillon, attachés à un **index jetable créé
  pour la mesure puis supprimé**, ce qui donne l'état « après le `DELETE` » et l'état après
  redémarrage.
- **États mesurés**, dans cet ordre, chacun sur le même processus : serveur neuf ; index ouvert ;
  après une passe de requêtes, aux états d'ADR-0077 ; après une campagne d'attache ; après le
  `DELETE` de l'index jetable ; après un redémarrage.
- **Sources à chaque état** : `/proc/<pid>/smaps_rollup`, `/proc/<pid>/status`, `memory.current`,
  `memory.stat` du cgroup, et les lignes `memory` du journal du serveur. Chaque état est lu au moins
  trois fois, les trois lectures et leur dispersion sont rapportées, une lecture unique ne suffisant
  pas puisque `memory.current` contient un cache qui se déplace.
- **Ce que le rapport doit dire** : quelle classe de mémoire grandit — anonyme possédée, cache de
  fichiers, session ONNX, non ventilé —, à quelle étape elle grandit, et si le `DELETE` ou le
  redémarrage la rend. Il dit aussi ce qu'il ne peut pas conclure, en particulier sur l'OOM
  d'attache du ticket, qui n'est pas archivé et demande sa propre reproduction.
- **Rapport** : `docs/mesures/MES-17-<slug>.md`, écrit par `@mesureur`, au modèle de
  `docs/mesures/README.md`.

### 4. Ce que cette tâche ne fait pas

- **Elle ne touche pas à ADR-0105, décision 2.** Les sessions de modèle restent chargées jusqu'au
  redémarrage ; c'est un choix assumé, à mesurer et non à changer à l'aveugle.
- **Elle n'expose aucun chiffre par l'API.** Une route ou un membre nouveau pour la mémoire serait un
  **écart** à ADR-0124, décision 1, dont la réponse de `GET /v1/stats` est un objet à membres fixes,
  et la question du chiffre de mémoire y est reportée jusqu'à ce qu'un chiffre soit défini et mesuré.
- **Elle ne corrige rien** : ni déchargement de session, ni `madvise`, ni `malloc_trim`, ni
  changement d'allocateur. Toute correction demande la mesure de MES-17 puis un ADR distinct.
- **Bornes déjà décidées, citées et non rouvertes** : ADR-0105, décision 2, sur les sessions ;
  ADR-0080, **décision 3** (un ADR de dépendance avant toute introduction de `libc`) et
  **décision 5** (vecteurs en RAM et `mlock` écartés).
- **Nuance au ticket** : `memmap2` est déjà dans `docs/07` et expose `Advice::DontNeed`, donc
  `MADV_DONTNEED` ne demanderait pas `libc`. Il ne vise pas la bonne classe pour autant : MES-09
  montre une croissance **anonyme**, que `MADV_DONTNEED` sur des `mmap` de fichiers ne rend pas.
- **`malloc_trim` et le choix d'allocateur ne sont décidés nulle part.** Aucun des deux n'entre dans
  cette tâche, même en dépendance de développement.

### 5. Classe de version

Classe **« ajout compatible »** au sens d'ADR-0143, décision 1, et dans sa forme la plus faible :
aucun contrat, aucune route, aucun champ, aucun membre, aucune clé, aucun code d'erreur, aucune valeur
par défaut et aucun format ne change. La seule addition visible est une **ligne de journal nouvelle**.
Ni `format_version`, ni OpenAPI, ni client Python ne sont touchés.

### 6. Tableau d'application

| Document ou livrable | Ce que l'application y écrit |
|---|---|
| `docs/06-observabilite.md` | Une sous-section « Mémoire » après « Santé et logs » : le chiffre qui fait foi, sa ventilation, et la ligne `memory` avec son message, son niveau, ses champs et ses étapes |
| `docs/08-invariants-et-tests.md` | Une ligne MES-17 dans le tableau des mesures, avec le protocole de la décision 3 |
| `crates/crible-server` | Un module de lecture de `/proc` et du cgroup, sous Linux seulement, et les points d'émission de la décision 2 |
| Tests | Le lecteur de `/proc` et de `memory.stat` sur des fichiers d'exemple ; un champ absent quand la source est illisible ; aucune ligne hors Linux |
| `docs/mesures/MES-17-<slug>.md` | Le rapport de la mesure, écrit par `@mesureur` |

Les documents `docs/02` à `docs/05` et `docs/07` ne changent pas : la règle de mémoire de `docs/02`
reste vraie, et la liste de dépendances de `docs/07` reste fermée.

## Conséquences

- **Positives** : un exploitant peut enfin lire la ventilation au lieu d'un total qui mélange
  mémoire possédée et cache ; un OOM futur s'attribue à une classe et à une étape ; le budget d'un
  projet se vérifie contre un chiffre nommé, et non contre `docker stats` ; toute correction future
  part d'une mesure reproductible. Le coût de la ligne est négligeable : trois lectures de fichiers
  pseudo-virtuels à la fin d'un job, plus deux à la suppression d'un index.
- **Négatives / coûts** : une ligne de plus par job dans le journal, ce qui pèse sur un journal
  bavard sans être un problème à l'échelle d'une instance ; une dépendance implicite au format des
  fichiers du noyau, qui peut changer, d'où le champ absent plutôt qu'une erreur ; un lecteur de plus
  à maintenir et à tester ; le risque qu'un lecteur prenne `cgroup_current_bytes` pour une fuite
  alors qu'il contient du cache, ce que `docs/06` doit dire noir sur blanc.
- **À surveiller** : si MES-17 montre une classe qui **ne retombe pas après le redémarrage**, ou qui
  grandit à chaque attache sans jamais se stabiliser, la conclusion n'est plus une question de
  chiffre mais de correction, et elle demande un ADR distinct — déchargement, `madvise`,
  `malloc_trim` ou allocateur, selon ce que la mesure désigne. Si `memory.current` et `RssAnon`
  divergent sans que `memory.stat` explique l'écart, la ventilation est incomplète et
  l'instrumentation se revoit.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Corriger sans mesurer (décharger les sessions, `madvise`, `malloc_trim`, changer d'allocateur) | On ne sait pas quelle classe grandit : MES-09 montre une croissance anonyme, que rien ne relie encore à une cause. Une correction à l'aveugle contredirait ADR-0105, décision 2, sans preuve de gain |
| Exposer le chiffre par l'API maintenant, par `GET /v1/stats` ou une route nouvelle | Écart à ADR-0124, décision 1 : sa réponse est un objet à membres fixes, et la question du chiffre de mémoire y est reportée jusqu'à un chiffre défini et mesuré. Un contrat d'API se gèle après coup, une ligne de journal non |
| Ajouter la dépendance `procfs` | La lecture de `/proc` par `std::fs` suffit et `mounts.rs` en donne le précédent ; une dépendance nouvelle demande son ADR (ADR-0080, décision 3, et `docs/07`) |
| Ajouter la dépendance `libc` pour `madvise` ou `mallopt` | Même raison, et `memmap2` expose déjà `Advice::DontNeed` ; le besoin n'est de toute façon pas établi |
| Décharger les sessions ONNX à la suppression d'un index | ADR-0105, décision 2, les garde jusqu'au redémarrage. C'est un choix assumé qui se mesure, pas une erreur à corriger d'office |
| Prendre `docker stats` pour chiffre de référence | Il affiche le même compteur de cgroup, cache compris, coûte environ 1,5 s par appel (rapport de Legifacile ADR-0026) et ne se ventile pas ; le budget de Legifacile se vérifie aujourd'hui avec lui, ce qui est précisément le problème |
| Mesurer sur l'import Legifacile en conteneur, comme la campagne de MES-09 du 2026-11-06 | Les index de cet import ne sont pas jetables ; la campagne d'attache y est lourde et la production ne se touche jamais (ADR-0032). Il reste le point de comparaison déclaré, pas le jeu de la mesure |
| Prendre `synth-large` pour jeu de MES-17, comme la campagne native de MES-09 | Le jeu n'est plus dans `data/` : le régénérer demande environ 13 Go de lots puis 15 à 17 Go d'index (MES-07 de Crible), plus la copie dans le volume, pour 60 Gio libres sur la machine de développement. Il n'a que des vecteurs et le lexical désactivé, donc il ne charge ni le texte ni la recette ONNX que l'OOM du ticket met en cause |
| Écrire la règle seulement dans ce rapport de mesure | Une mesure sans règle d'exploitation ne se lit pas deux mois plus tard : `docs/06` est l'endroit où l'exploitant cherche le chiffre qui compte, et il n'en avait aucun |

## Références

- Ticket : `/Users/mattboll/MattCorp/legifacile/tickets-crible/05-memoire-non-rendue.md` (projet
  Legifacile, lecture seule).
- **Legifacile** : `/Users/mattboll/MattCorp/legifacile/docs/mesures/ADR-0026-memoire-attache-quotidienne.md`
  (budget mémoire, 3,591 / 4,698 / 4,671 / 5,049 GiB, coût de `docker stats`) et
  `/Users/mattboll/MattCorp/legifacile/docs/mesures/MES-02-encodage-quotidien.md` (6,17 GiB au repos
  après la campagne). Ces numéros sont ceux de **Legifacile** et ne désignent pas les ADR ni les
  mesures de Crible portant les mêmes numéros.
- Crible : `docs/mesures/MES-09-ram-reelle.md` (vers 433-446 pour le delta anonyme de 10,55 GiB) ;
  `docs/02-architecture.md` (vers 226-243, mémoire possédée et pages `mmap` évinçables) ;
  `docs/06-observabilite.md` (sections « Santé et logs » et « Ce que les logs ne contiennent
  jamais ») ; `docs/08-invariants-et-tests.md` (tableau des mesures jusqu'à MES-16, INV-13).
- ADR : ADR-0032 (production en lecture seule), ADR-0077 (décision 5, RSS en trois états), ADR-0080
  (décisions 3 et 5), ADR-0096 (dépendances du serveur et contenu des logs), ADR-0105 (décision 2,
  sessions jamais déchargées ; décision 6, suppression d'un index), ADR-0119 (décision 3, membre
  absent plutôt que `0`), ADR-0124 (décision 1, forme fermée de `GET /v1/stats`), ADR-0126 (lecteurs
  denses en cache), ADR-0135 (décisions 3, 5 et 7, protocole de la mesure en conteneur), ADR-0143
  (décision 1, classes de version), ADR-0149 (champs `job_id`, `kind` et `index` d'une ligne de
  journal).
- Code : `crates/crible-server/src/mounts.rs` (lecture de `/proc` par `std::fs`, Linux seulement),
  `crates/crible-server/src/indexes.rs` (les deux étapes de `delete_index`),
  `crates/crible-encode/src/model.rs` (la `Session` sous `Mutex`), `crates/crible-server/src/jobs.rs`
  (fin de job et ligne `attach durations`).
- Outils et données : `data/legifacile-sample/README.md` (contenu, tailles et requêtes de
  l'échantillon, jeu retenu) ; `tools/bench/mes02/` (harnais de MES-09) ; `tools/gen_synth.py
  --large` (lots de `synth-large`, jeu écarté par la décision 3).
