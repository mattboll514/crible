# ADR-0157 — Protocole de la seconde campagne de MES-17 : attribuer la mémoire anonyme retenue

- **Statut** : Accepté (décidé par le propriétaire le 2026-11-21, sa réponse : « ADR-0157 accepté », tel que proposé)
- **Date** : 2026-11-21 (proposition et acceptation le même jour)
- **Décideurs** : propriétaire
- **Proposé par** : agent `@greffier`, sur demande du propriétaire du 2026-11-21, tâche `T05-SUITE-ATTRIBUER-ANONYME` (v1.1), suite du ticket 05 de Legifacile
- **Remplace** : — (complète ADR-0151, **décision 3 seulement**, pour cette campagne ; le corps d'ADR-0151 n'est pas modifié)
- **Décision** : confirmée par le propriétaire le 2026-11-21, sa réponse : « ADR-0157 accepté », sans réserve et sans modification du corps.

## Contexte

### Le ticket et la mesure déjà faite

Le ticket 05 de Legifacile, en lecture seule
(`/Users/mattboll/MattCorp/legifacile/tickets-crible/05-memoire-non-rendue.md:10-32` et `:63-64`),
constate que la mémoire du conteneur ne redescend ni après une campagne d'attache ni après la
suppression des index jetables, et qu'aucun compteur ne sépare l'anonyme du cache de fichiers. La
VM de Docker y est donnée pour 9,704 GiB, soit 10 420 109 312 octets (`:126`) — projet Legifacile,
dont les numéros d'ADR et de mesure recouvrent ceux de Crible et ne désignent pas les mêmes
documents.

La tâche `T05-MEMOIRE-NON-RENDUE`, `FAIT` le 2026-11-12 (`docs/suivi.md:166`), a répondu par
l'instrumentation seule, décidée par ADR-0151 : le chiffre qui fait foi, la ligne de journal
`memory` et la mesure MES-17 (`docs/adr/0151-chiffre-de-memoire-qui-fait-foi-journal-et-mesure-mes-17.md:62-65`,
protocole `:121-171`, erratum `:173-181`).

Le rapport de MES-17 (`docs/mesures/MES-17-classes-de-memoire.md`, campagne du 2026-11-12,
`:3-11` pour la machine, la VM, l'image et le jeu) donne ceci :

- **la mémoire anonyme est la classe qui grandit à l'attache** : +1 155,6 MiB sur le premier
  processus et +1 514,6 MiB sur le second, sur 257 626 passages (`:139` ; le suivi résume ces deux
  valeurs en « +1,16 à +1,51 Gio », `docs/suivi.md:166`) ;
- **le `DELETE` ne rend que le cache de fichiers** : −753,9 et −754,0 MiB de `file`, contre −60,9
  et −63,7 MiB d'anonyme (`:142`) ;
- **le redémarrage rend tout** : l'état (f) revient au niveau du serveur neuf (`:143`) ;
- **la session ONNX pèse 1,45 à 1,49 Gio d'anonyme**, une fois par processus (`:98`) ;
- **la mesure ne distingue pas une rétention d'allocateur d'une structure vivante** (`:150`).

Les recommandations 1 et 5 de ce rapport restent ouvertes (`:156` et `:160`) : attribuer l'anonyme
retenu avant de le corriger, et, pour une prochaine campagne, placer la passe de requêtes après
l'attache et ajouter un septième état « redémarrage avec l'index encore présent ».

### Ce qui pèse sur la suite

- **La correction n'est décidée nulle part.** ADR-0151 (`:190-191` et `:198-199`) exclut de sa
  tâche `malloc_trim`, `madvise`, le déchargement de session et le changement d'allocateur, et
  demande un ADR distinct après mesure. ADR-0142 (`:79`) range un changement d'allocateur dans les
  dépendances nouvelles, donc dans un ADR.
- **Le propriétaire a tranché le 2026-11-21 : « Mesurer d'abord »**, c'est-à-dire attribuer
  l'anonyme retenu avant toute correction. C'est aussi la recommandation 1 de MES-17.
- **Le 2026-11-08, un changement d'allocateur a été rangé dans la liste de la v2**
  (`docs/suivi.md:918`, ligne « IMPORTER-BASE-COMPLETE (passe A, `@mesureur`) »), et ADR-0142 le
  rappelle dans « À surveiller » (`:79`). Rien ne l'en sort ici.
- **La campagne ne peut pas réutiliser le protocole tel quel.** Les six états d'ADR-0151,
  décision 3 (`:150-155`), et la ligne MES-17 de `docs/08-invariants-et-tests.md:82` placent la
  passe de requêtes sur un index encore vide : MES-17 a mesuré +2,4 MiB, avec `pool_size` à 0
  (`docs/mesures/MES-17-classes-de-memoire.md:129`). Un septième état n'existe pas.
- **L'image est glibc.** La couche d'exécution est `debian:bookworm-slim`
  (`docker/Dockerfile:16` et `:73`), donc les réglages `glibc.malloc.*` existent dans le conteneur.
  `GLIBC_TUNABLES` n'est pas une variable `CRIBLE_*` : la liste fermée d'ADR-0097 ne s'y applique
  pas, et poser cette variable dans `docker run` ne change ni l'image ni `crible.toml`.
- **Le harnais de la première campagne existe, mais hors du dépôt.** `prepare_lots.py` et
  `measure.py` sont sous `data/mes17/`, gitignoré, et MES-17 le déclare comme un écart
  (`docs/mesures/MES-17-classes-de-memoire.md:130`). Les harnais versionnés des autres mesures
  vivent sous `tools/bench/mesNN/`, par exemple `tools/bench/mes16/` et `tools/bench/mes10/`.
- **Moyens de la campagne, constatés le 2026-11-21** : la VM Docker a 10 420 109 312 octets et
  8 vCPU, identique à celle de la première campagne (`docs/mesures/MES-17-classes-de-memoire.md:7`) ;
  101 Gio libres sur le disque.

### Le verdict de `@architecte` du 2026-11-21

Verdict rendu en session `ses_e00b86f85ffeHRmlRyD4GdpcJq`, sur la demande de la tâche. Il n'est
pas un document du dépôt ; il est cité ici comme l'avis qui encadre la proposition.

| Point examiné | Verdict | Ce que cela impose |
|---|---|---|
| Attitude « mesurer, ne rien corriger » | `CONFORME` | ADR-0151, décision 4, et ADR-0142, « À surveiller » |
| Tunables glibc en campagne (`GLIBC_TUNABLES`) | `NON-COUVERT` | Les moyens sont permis — ce n'est pas une variable `CRIBLE_*`, ni l'image ni `crible.toml` ne changent —, mais **aucun protocole de campagne de perturbation n'est décidé** : d'où cet ADR |
| `malloc_trim` appelé par `gdb -p`, et profils de tas | `NON-COUVERT` | Exclus de cette campagne (décision 6) |
| `mallinfo2` dans la ligne `memory` | `ECART` | `libc` est absente de `docs/07` (ADR-0080, décision 3, `docs/adr/0080-acces-aux-vecteurs-fp16-au-rescoring.md:80`), et la forme de la ligne est fixée par ADR-0151, décision 2 |
| Passe de requêtes après l'attache, et septième état | `ECART` à ADR-0151, décision 3 | Les six états et leur ordre y sont fixés, et repris par `docs/08` : d'où un ADR nouveau qui complète la décision 3 |
| Machine, jeu et VM | `CONFORME` | Mêmes conditions que la première campagne |

## Décision

**Une seconde campagne de MES-17 attribue la mémoire anonyme retenue par des variantes de réglage
de l'allocateur, sans toucher au code, à l'image ni à la configuration de production. Elle ne
corrige rien.**

### 1. Une section datée de MES-17, pas une mesure nouvelle

La campagne garde l'identifiant **MES-17**. Elle ajoute une **section datée** à
`docs/mesures/MES-17-classes-de-memoire.md`, sans réécrire la campagne du 2026-11-12 : c'est la
règle de `docs/mesures/README.md:5`, une nouvelle campagne sur la même mesure ajoute une section
datée. Les données brutes vont sous `data/mes17/raw/<date>/`.

Les conditions restent celles de la première campagne : même machine de développement, même VM
(10 420 109 312 octets, 8 vCPU, sans `--memory`, ADR-0135, décision 5), même échantillon
Legifacile `data/legifacile-sample`. L'image est **construite depuis le commit de `dev` au
lancement**, et son commit est noté dans le rapport. Le contrôle de la première campagne est
**rejoué dans cette campagne** : les comparaisons sont internes à la campagne, jamais entre une
variante d'aujourd'hui et un chiffre du 2026-11-12.

### 2. Sept états, dans cet ordre, pour cette campagne

Les états suivants **complètent** ADR-0151, décision 3, pour cette seule campagne. Le septième état
et la place de la passe de requêtes sont l'écart relevé par `@architecte`, et c'est cet ADR qui le
porte.

| État | Description |
|---|---|
| (a) | processus neuf, aucun index |
| (b) | index créé, avec la ligne `session_load` |
| (c) | six lots attachés |
| (d) | passe de requêtes sur l'index plein, la même que la campagne complémentaire de MES-17 (300 requêtes de `queries.jsonl`, mode `hybrid`, `k = 10`, `pool_size` non nul) |
| (e) | redémarrage avec l'index présent |
| (f) | `DELETE` de l'index |
| (g) | redémarrage sans index |

Les états (a) à (g) se lisent dans cet ordre, sur le même conteneur et le même volume, comme les
six états de la première campagne. L'état (e) répond à la question laissée ouverte par MES-17
(`:151`) : ce que coûte la parade « redémarrer », avec le rechargement de la session et de l'index.

### 3. Relevés : trois lectures par état, et le contrôle du journal

Chaque état est lu **trois fois, à 3 secondes d'intervalle** ; le rapport donne la **médiane** et la
**dispersion** (maximum moins minimum), comme la première campagne.

Sources lues à chaque état :

- `/proc/1/status` (`VmRSS`, `RssAnon`, `RssFile`, `RssShmem`, `VmHWM`) ;
- `/proc/1/smaps_rollup` ;
- `/proc/1/smaps` complet, avec le **nombre et le total des régions anonymes d'environ 64 Mio**,
  signature des arènes de glibc ;
- `memory.current` et `memory.stat` du cgroup ;
- les lignes `memory` du journal du serveur.

Le relevé de `smaps` est l'addition de cette campagne : il ne demande aucun code, seulement la
lecture d'un fichier que la première campagne ignorait.

**Contrôle du journal.** Le rapport vérifie la présence des **quatre `step`** d'ADR-0151,
décision 2 (`:107-111`) : `job_end`, `delete_before_rename`, `delete_after_removal` et
`session_load`. Il vérifie en plus **un `job_end` par lot attaché** et **deux `session_load`**, aux
états (b) et (e). Une ligne manquante est un **défaut du rapport**, pas une observation à
interpréter (ADR-0151, erratum, `:173-177`).

### 4. Variantes : quatre réglages d'allocateur, posés dans le harnais seulement

Les variantes sont posées par `docker run -e GLIBC_TUNABLES=…` dans le harnais de campagne,
**jamais dans l'image, jamais dans `crible.toml`, jamais comme défaut de production**.

| Variante | Réglage | Hypothèse testée |
|---|---|---|
| V0 | aucun, environnement par défaut | contrôle |
| V1 | `glibc.malloc.arena_max=1` | rétention par les arènes |
| V2 | `glibc.malloc.trim_threshold=0` et `glibc.malloc.top_pad=0` | tas libre non rendu au système |
| V3 | `glibc.malloc.mmap_threshold=131072` | gros tampons transitoires |
| V4 | la combinaison des variantes qui ont eu un effet | effet cumulé |

V2 pose deux réglages dans la même variable, séparés par `:`, syntaxe des tunables glibc. Si une
seule variante a eu un effet, V4 reprend cette variante seule ; si aucune n'en a eu, V4 ne tourne
pas (décision 5).

**V0 et V4 tournent sur deux processus chacun.** La dispersion entre processus de MES-17 — la
session à 1 493,9 MiB sur le premier et 1 446,6 MiB sur le second, l'attache à +1 155,6 et
+1 514,6 MiB (`docs/mesures/MES-17-classes-de-memoire.md:96`) — interdit de lire une variante sur
un seul processus. V1 à V3 tournent sur un processus chacune, puisque leur seul rôle est de
départager des hypothèses, avant une confirmation par V4.

La campagne compte donc **sept exécutions** du parcours (a) à (g) : V0 deux fois, V1 à V3 une fois,
V4 deux fois.

### 5. Règle de lecture

Une variante ne conclut que si **son effet sur l'anonyme retenu dépasse la dispersion entre les deux
processus de V0**. L'effet se lit dans cet ordre :

1. **l'anonyme retenu** = anonyme de l'état (c) moins anonyme de l'état (b), c'est-à-dire la
   croissance de l'attache ;
2. **l'état (d)** dit si la passe de requêtes sur l'index plein ajoute ou non de l'anonyme ;
3. **l'état (f)** dit ce que le `DELETE` rend.

Correspondance entre l'effet observé et la cause :

| Résultat | Lecture |
|---|---|
| V1 a un effet | rétention par les arènes de glibc |
| V2 a un effet | tas libre non rendu au système |
| V3 seule a un effet | gros tampons transitoires |
| Aucune variante n'a d'effet | structure vivante ou fragmentation |

Si **aucune variante n'a d'effet**, la suite n'est pas dans cet ADR : elle demande un **nouvel ADR**,
avec un profil de tas ou des compteurs d'allocateur (décision 6).

**Aucune conclusion de latence n'est tirée des variantes.** Les réglages changent l'allocateur, donc
le temps ; la campagne ne mesure pas la latence, et une phrase sur ce sujet serait une affirmation
sans mesure.

### 6. Exclus de cette campagne

- **`malloc_trim` appelé par `gdb -p`**, et tout `ptrace` : `NON-COUVERT` pour `@architecte`, et hors
  du périmètre d'une campagne de réglages.
- **Les profileurs de tas** : heaptrack, massif, ou tout autre outil équivalent.
- **`mallinfo2`, donc `libc`, dans la ligne `memory`** : `ECART` pour `@architecte`, parce que
  `libc` est absente de `docs/07` (ADR-0080, décision 3) et que la forme de la ligne est fixée par
  ADR-0151, décision 2.

Ces trois moyens ne reviennent que par un **nouvel ADR**, si V1 à V3 ne tranchent pas.

### 7. Aucune correction dans ce travail

Aucune correction n'entre dans cette tâche. Les tunables dans l'image, `malloc_trim`, un autre
allocateur et le déchargement de session demandent **chacun un ADR distinct** après la campagne.
Le propriétaire décidera alors si la correction entre dans la 1.1.0 ou reste dans la liste de la v2
(décision du 2026-11-08, `docs/suivi.md:918`).

### 8. Harnais versionné

`prepare_lots.py` et `measure.py` rejoignent `tools/bench/mes17/`, complétés des variantes et du
septième état, pour que la campagne se rejoue depuis le dépôt. Aucun chemin de production ni aucune
valeur de `.env.prod` n'y figure (ADR-0032) : le harnais ne connaît que la machine de développement,
l'échantillon et des volumes jetables.

### 9. Disque

Au moins **15 Gio libres au départ**. `df -h` est relevé au début et à la fin. L'index est supprimé
et le volume réutilisé entre deux exécutions, comme dans la première campagne. Le constat du
2026-11-21 donne 101 Gio libres, au-dessus du plancher.

### 10. Classe de compatibilité

Classe **« sans risque »** au sens d'ADR-0143, décision 1 (`:122`) : aucun code du serveur, aucun
format, aucun contrat ne change. La campagne ajoute un rapport, une section datée de MES-17 et un
harnais versionné.

### 11. Ce que cet ADR ne fait pas

Il ne modifie **ni ADR-0151, ni `docs/08`, ni `docs/06`, ni `docs/suivi.md`**. À l'acceptation,
ADR-0151 recevra seulement un renvoi vers cet ADR, écrit par `@greffier` sur décision du
propriétaire ; la ligne MES-17 de `docs/08` et la section « Mémoire » de `docs/06` suivront de même
si le propriétaire le demande. D'ici là, les six états d'ADR-0151 restent la règle écrite, et cette
campagne s'en écarte par le présent ADR.

## Conséquences

- **Positives** : la question laissée ouverte par MES-17 (« rétention d'allocateur ou structure
  vivante ») reçoit un protocole reproductible, sans code, sans dépendance et sans toucher à la
  production. Les réglages se posent en une variable d'environnement, donc la campagne se rejoue
  depuis le dépôt. Le relevé de `smaps` donne la signature des arènes sans `libc`. L'état (e)
  chiffre enfin ce que coûte un redémarrage avec l'index présent.
- **Négatives / coûts** : sept exécutions du parcours complet, soit sept campagnes d'attache de six
  lots et deux redémarrages par exécution, ce qui prend du temps et de l'espace disque ; deux
  processus pour V0 et V4, sans quoi la lecture n'a pas de bruit de référence ; une variable
  d'environnement
  d'allocateur est un levier grossier, qui peut masquer un effet au lieu de l'expliquer ; `smaps`
  complet est volumineux, et son analyse demande de compter les régions plutôt que de les
  conserver.
- **À surveiller** : si la dispersion entre les deux processus de V0 dépasse l'effet de toutes les
  variantes, la campagne ne conclura pas et le protocole se revoit par un ADR ; si V1 a un effet
  franc, la rétention par les arènes devient la cause à instruire pour une correction, avec son ADR ;
  si aucune variante n'a d'effet, la piste « structure vivante ou fragmentation » s'ouvre, et elle
  demande un profil de tas, donc un ADR et une dépendance ou un outil externe.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Profil de tas d'abord (heaptrack, massif), sans variantes | `NON-COUVERT` pour `@architecte` : l'outil demande un accès au processus et une installation, alors qu'une variable d'environnement tranche les trois hypothèses les plus probables sans rien ajouter au dépôt. Le profil reste la suite si les variantes ne tranchent pas |
| `malloc_trim` appelé par `gdb -p` | `NON-COUVERT` : c'est déjà une correction, elle demande un ADR, et `ptrace` sur un processus en service est un geste d'exploitation lourd, hors du périmètre d'une mesure |
| `mallinfo2` dans la ligne `memory` | `ECART` : `libc` est absente de `docs/07` (ADR-0080, décision 3), et la forme de la ligne `memory` est fixée par ADR-0151, décision 2. La changer est une correction d'instrumentation, pas une campagne |
| Corriger sans attribuer (décharger la session, `malloc_trim`, changer d'allocateur) | La mesure ne distingue pas une rétention d'allocateur d'une structure vivante (`docs/mesures/MES-17-classes-de-memoire.md:150`) : corriger à l'aveugle contredirait ADR-0151, décision 4, et ADR-0142, et empêcherait de dire si le gain vient de la cause visée |
| Nouvel identifiant de mesure, par exemple MES-18 | Une nouvelle campagne sur la même question est une section datée de la même mesure (`docs/mesures/README.md:5`) ; un identifiant nouveau laisserait croire à une question différente et couperait la comparaison interne |
| Écrire un complément daté dans le corps d'ADR-0151 | Refusé : un ADR accepté ne se réécrit pas (`docs/adr/README.md:33-35`). Un ADR nouveau complète la décision 3, et ADR-0151 ne recevra qu'un renvoi à l'application |
| Poser les tunables dans l'image ou dans `crible.toml` | Ce serait une correction de production, donc un ADR distinct (décision 7). Une campagne ne change pas ce que sert l'image |
| Lire une variante sur un seul processus, V0 compris | Les deux processus de MES-17 diffèrent de 1 155,6 à 1 514,6 MiB sur l'attache (`docs/mesures/MES-17-classes-de-memoire.md:96`) : une lecture unique ne distingue pas un effet d'un écart de processus |

## Références

- Ticket : `/Users/mattboll/MattCorp/legifacile/tickets-crible/05-memoire-non-rendue.md` (projet
  Legifacile, lecture seule ; ses numéros d'ADR et de mesure ne sont pas ceux de Crible).
- Mesure : `docs/mesures/MES-17-classes-de-memoire.md` (campagne du 2026-11-12, résultats `:56-124`,
  écarts `:126-133`, conclusion `:135-162`) ; `docs/mesures/README.md:5` (section datée).
- Suivi : `docs/suivi.md:166` (tâche `T05-MEMOIRE-NON-RENDUE`, `FAIT`), `:918` (changement
  d'allocateur dans la liste de la v2), `:948` (décisions attendues sur la suite du ticket 05).
- ADR : ADR-0151 (chiffre de mémoire, ligne `memory`, protocole de MES-17, décision 4 « ne corrige
  rien »), ADR-0142 (`:79`, mémoire anonyme après un `replace_all` et allocateur), ADR-0143
  (`:122`, classe « sans risque »), ADR-0080 (décision 3, `libc` absente de `docs/07`), ADR-0097
  (liste fermée des variables `CRIBLE_*`), ADR-0105 (décision 2, sessions jamais déchargées),
  ADR-0135 (décisions 5 et 7, protocole de la mesure en conteneur), ADR-0032 (production en lecture
  seule), ADR-0139 (VM de référence des campagnes d'import, pour situer les chiffres).
- Documents de conception : `docs/08-invariants-et-tests.md:82` (ligne MES-17, six états) ;
  `docs/06-observabilite.md` (section « Mémoire ») ; `docs/07-stack-et-deploiement.md` (liste des
  dépendances, sans `libc`).
- Outils et données : `data/mes17/prepare_lots.py` et `data/mes17/measure.py` (harnais de la
  première campagne, gitignoré), à porter sous `tools/bench/mes17/` ; `tools/bench/mes16/` et
  `tools/bench/mes10/` (modèle de harnais versionné) ; `docker/Dockerfile:16` et `:73` (image
  `debian:bookworm-slim`, donc glibc).
- Verdict : `@architecte`, session `ses_e00b86f85ffeHRmlRyD4GdpcJq`, 2026-11-21.
