# ADR-0159 — Essai de `malloc_trim(0)` dans une troisième campagne de MES-17

- **Statut** : Accepté (décidé par le propriétaire le 2026-11-22, sa réponse : « ADR-0159 accepté », tel que proposé)
- **Date** : 2026-11-22 (proposition et acceptation le même jour)
- **Décideurs** : propriétaire
- **Proposé par** : agent `@greffier`, sur la demande du propriétaire du 2026-11-22, tâche `T05-ESSAI-MALLOC-TRIM` (v1.1), suite du ticket 05 de Legifacile
- **Remplace** : — (lève, **pour cette campagne seule**, l'exclusion de `malloc_trim` par `gdb -p` posée par ADR-0157, décision 6 ; le corps d'ADR-0157 n'est pas modifié)

## Contexte

### Le ticket, et les deux campagnes déjà faites

Le ticket 05 de Legifacile, en lecture seule
(`/Users/mattboll/MattCorp/legifacile/tickets-crible/05-memoire-non-rendue.md`), constate que la
mémoire du conteneur ne redescend ni après une campagne d'attache ni après la suppression des index
jetables. Ses numéros d'ADR et de mesure recouvrent ceux de Crible et ne désignent pas les mêmes
documents.

La première campagne de MES-17, du 2026-11-12, a répondu par l'instrumentation seule, décidée par
ADR-0151. Elle laisse une question ouverte : « la mesure ne distingue pas une arène d'allocateur
d'une structure vivante » (`docs/mesures/MES-17-classes-de-memoire.md:150`).

La seconde campagne, du 2026-11-22, a testé des variantes de réglage de l'allocateur, sous le
protocole d'ADR-0157 complété par ADR-0158. Ses résultats sont dans la section « Campagne du
2026-11-22 » de `docs/mesures/MES-17-classes-de-memoire.md:173` et suivantes, et dans le tableau
d'ADR-0158 (`:47-58`). Médianes de l'anonyme du cgroup, en MiB :

| Variante | (b) index créé | (c) six lots | (c)−(b) |
|---|---:|---:|---:|
| V0-p1 | 1 543,7 | 2 494,8 | +951,1 |
| V0-p2 | 1 489,3 | 2 485,7 | +996,4 |
| V1 `glibc.malloc.arena_max=1` | 1 309,6 | 2 276,2 | +966,6 |
| V2 `glibc.malloc.trim_threshold=0:glibc.malloc.top_pad=0` | 1 314,3 | 2 367,9 | +1 053,6 |
| V3 `glibc.malloc.mmap_threshold=131072` | 1 329,4 | 2 359,7 | +1 030,3 |

La dispersion entre les deux processus de V0 vaut **45,3 MiB**. Au sens fixé par ADR-0158, un effet
est une **baisse** : aucune variante n'a d'effet, V4 ne tourne pas, et la campagne s'arrête à cinq
parcours. Un redémarrage avec l'index présent rend la rétention (`ADR-0158:116-118`).

Restent donc deux causes possibles, non départagées : une **structure vivante** ou une
**fragmentation**. Et V2 ne teste pas toute la fragmentation : sur glibc 2.36,
`trim_threshold=0` ne rend que le haut du tas, c'est-à-dire l'arène principale, alors que
`malloc_trim(0)` rend aussi, depuis glibc 2.8, les pages libres entières au milieu des arènes, par
`madvise` (`ADR-0158:60-66`).

### Ce qui pèse sur la suite

- **La piste est déjà écrite.** La recommandation 3 de la section datée de MES-17
  (`:546-549`) est « Essai de `malloc_trim(0)` » : elle testerait la fragmentation que V2 ne rend
  pas.
- **Un ADR est obligatoire.** ADR-0158, décision 4 (`:124-129`), range « essai de `malloc_trim(0)` »
  parmi les campagnes nouvelles qui demandent un **nouvel ADR**. ADR-0157, décision 6
  (`:199-208`), excluait `malloc_trim` appelé par `gdb -p` et tout `ptrace` de sa campagne, et
  renvoyait ces moyens à un nouvel ADR. ADR-0151 (`:196`) exclut de même `malloc_trim` de sa tâche.
- **Le propriétaire a tranché le 2026-11-22** : il retient la piste (3) des recommandations de
  MES-17, « Essai `malloc_trim(0)` », et demande cet ADR avant tout lancement.
- **L'image mesurée existe déjà.** `crible:t05-suite-mes17`,
  `sha256:dc800f534ff50589038b0ca5d7a4fd9336a3259c45b70fdb00408b84366a8080`, a été construite par
  l'orchestrateur le 2026-11-22 depuis `31e9804`. Vérifié par l'orchestrateur le 2026-11-22 : aucun
  fichier de `crates/`, `Cargo.toml`, `Cargo.lock` ni `docker/` n'a changé entre `31e9804` et `dev`
  (`bf55d54`) ; l'image n'est donc **pas reconstruite** pour cette campagne.
- **Le harnais versionné existe.** `tools/bench/mes17/measure.py`, `prepare_lots.py` et leur
  `README.md` sont dans le dépôt depuis la seconde campagne ; cette campagne l'étend.

### Le verdict de `@architecte` du 2026-11-22

Verdict rendu en session `ses_dfd42a6f5ffe6fPIx9ubhz3wBN`, sur la demande de la tâche. Il n'est pas
un document du dépôt ; il est cité ici comme l'avis qui encadre la proposition.

| Point examiné | Verdict | Ce que cela impose |
|---|---|---|
| Campagne de mesure seule, sans correction | `CONFORME` | ADR-0151, décision 4 ; ADR-0157, décision 7 ; ADR-0158, décision 4 |
| `malloc_trim(0)` par `gdb -p`, dans un conteneur annexe | `NON-COUVERT` | L'exclusion d'ADR-0157, décision 6, est levée pour cette campagne seule, avec `ptrace` borné au conteneur de mesure : décision 1 |
| Image annexe avec `gdb` | `NON-COUVERT` | Un Dockerfile versionné, une base épinglée par digest, `gdb` hors de `docs/07` et de l'image de Crible : décision 2 |
| Contrôles de l'outil : valeur de retour, second appel, auto-test | `NON-COUVERT` | Trois contrôles obligatoires : décision 3 |
| États et ordre de la campagne | `NON-COUVERT` | Deux processus, et un parcours (a) à (f′) : décision 4 |
| Règle de lecture et seuil | `NON-COUVERT` | Baisse, seuil pris dans **cette** campagne, le 45,3 MiB n'est pas réutilisé : décision 5 |
| Garde, arrêt de l'instance de développement, `gdb` au repos | `CONFORME` sous conditions | Décision 6 |
| Aucune correction, aucun profil, aucune mesure de latence | `CONFORME` | Décision 7 |
| Classe de compatibilité | `CONFORME` | « sans risque », ADR-0143, décision 1 (`:122`) |

## Décision

**Une troisième campagne datée de MES-17 appelle `malloc_trim(0)` dans le processus du serveur, par
`gdb -p`, depuis un conteneur annexe jetable, pour mesurer ce qu'une fragmentation rendrait. C'est
une mesure seule : elle ne corrige rien, et `malloc_trim` n'entre ni dans le code, ni dans l'image,
ni dans la configuration.**

### 1. Campagne datée de MES-17, mesure seule, `ptrace` borné

La campagne garde l'identifiant **MES-17** et ajoute une **section datée** à
`docs/mesures/MES-17-classes-de-memoire.md`, sans réécrire les sections antérieures
(`docs/mesures/README.md:5`). Les données brutes vont sous `data/mes17/raw/<date>/`.

L'appel se fait dans le processus du serveur, depuis un **conteneur annexe jetable** :

```sh
gdb -p <pid> -batch -ex 'call (int) malloc_trim(0)'
```

Le conteneur annexe partage l'espace de PID du conteneur mesuré (`--pid=container:<nom>`) et reçoit
`--cap-add SYS_PTRACE`. `ptrace` n'est autorisé **que** sur ce conteneur de mesure jetable, jamais
sur `legifacile-dev-crible-1`, jamais sur la production (ADR-0032). La campagne tourne sur la machine
de développement seulement, avec la VM et le jeu de la première campagne.

L'exclusion d'ADR-0157, décision 6, sur `malloc_trim` par `gdb -p` et sur `ptrace` est **levée pour
cette campagne seule**. Les deux autres exclusions de cette décision restent : ni profileur de tas, ni
`mallinfo2`.

L'image mesurée est `crible:t05-suite-mes17`, conservée depuis la seconde campagne et non
reconstruite (voir le contexte). Le volume est le volume jetable `crible-mes17-data`, recréé pour la
campagne. Le harnais `tools/bench/mes17/` est étendu pour cette campagne.

### 2. Image annexe avec `gdb`, hors de Crible et hors de `docs/07`

Le conteneur annexe a sa propre image, construite depuis une **base Debian épinglée par digest**, de
la même version de Debian que l'étage `runtime` de `docker/Dockerfile` (`debian:bookworm-slim`), avec
`gdb` installé **dans l'image annexe seulement**. Son Dockerfile est versionné sous
`tools/bench/mes17/`. Le conteneur est lancé avec `--pull=never` une fois l'image construite, et le
digest de l'image est noté dans le rapport.

Construire cette image télécharge la base et le paquet `gdb` : c'est accepté avec cet ADR, et borné à
cette image.

**`gdb` n'entre pas dans `docs/07`** : ce n'est ni une dépendance de build ni une dépendance
d'exécution. C'est le même traitement que `ld.so --list-tunables` de la campagne précédente, qui a
servi de preuve sans entrer dans la liste des dépendances (`docs/07:25-53`). `gdb` et `libc` restent
absents de l'image de Crible et de `Cargo.toml` (ADR-0080, décision 3).

### 3. Contrôles de l'outil

Trois contrôles sont obligatoires, et leurs résultats vont au rapport :

1. **Valeur de retour** de chaque `malloc_trim(0)` notée : `1` signifie que des pages ont été
   rendues, `0` que rien ne l'a été.
2. **Second appel** juste après le premier, pour l'idempotence : attendu `0`, et aucune baisse
   supplémentaire de l'anonyme.
3. **Auto-test du harnais** sur un processus jetable **sans Crible**, avant la campagne : il vérifie
   que le conteneur annexe atteint bien le processus visé, que `gdb` rend la valeur de retour, et que
   la mesure d'anonyme suit. Un auto-test raté arrête la campagne avant toute exécution du parcours.

**Aucun trim entre (b) et (c).** Il perturberait le tas avant l'attache et détruirait la référence
(c)−(b), qui est le chiffre de la campagne (ADR-0157, `:178-179`).

### 4. États et ordre : deux processus, un parcours (a) à (f′)

La campagne tourne sur **deux processus**, comme V0 dans la campagne précédente, parce que la
dispersion entre processus interdit de lire une valeur unique. Chaque processus suit ce parcours,
dans cet ordre :

| Étape | Description |
|---|---|
| (a) | processus neuf, aucun index |
| (b) | index créé, avec la ligne `session_load` |
| (c) | six lots attachés |
| trim | premier `malloc_trim(0)`, valeur de retour notée |
| (c′) | relevé après le premier trim |
| trim | second appel, contrôle d'idempotence |
| (c″) | relevé après le second appel |
| `DELETE` | suppression de l'index |
| (f) | relevé après le `DELETE` |
| trim | `malloc_trim(0)` après le `DELETE`, valeur de retour notée |
| (f′) | relevé après ce trim |

Les états (d) et (e) d'ADR-0157 ne sont **pas rejoués** : (d) ne vaut qu'environ +18 MiB et ne
concerne pas la rétention de l'attache, et (e) est un fait déjà consigné (`ADR-0158:116-118`).

Les relevés gardent la forme de la campagne précédente : **trois lectures à 3 secondes
d'intervalle** par état, médiane et dispersion (ADR-0157, `:126-127`). Le chiffre qui fait foi reste
celui d'ADR-0151 et d'ADR-0157 ; cet ADR n'en change ni la définition ni la source.

### 5. Règle de lecture et seuil

L'effet se lit comme une **baisse**, au sens d'ADR-0158.

**Le seuil est la dispersion entre les deux processus de cette campagne sur l'anonyme retenu
(c)−(b).** Le chiffre du 2026-11-22 (45,3 MiB) **n'est jamais réutilisé** : les comparaisons sont
internes à la campagne (ADR-0157, `:101-102`). Le rapport donne aussi la dispersion intra-état, celle
des trois lectures de chaque état.

Lecture :

- une **baisse (c)−(c′)** d'au moins le seuil attribue cette part à une **fragmentation que
  `malloc_trim(0)` rend** ;
- une **baisse sous le seuil** est une **observation**, pas un effet ;
- après un premier appel rendant `1` et un second rendant `0`, ce qui reste est une **structure
  vivante** ou une **fragmentation non rendable**.

On ne conclut **ni** « fuite », **ni** « pas de fuite », **ni** sur l'échelle de production, **ni**
sur la latence.

### 6. Garde, arrêt de l'instance et appel au repos

- **Disque** : au moins **15 Gio libres**, `df` relevé au début et à la fin (ADR-0157, `:226-228`).
- **Exploitation** : l'instance `legifacile-dev-crible-1` est arrêtée pendant la campagne ; aucune
  autre charge Docker ne tourne ; aucun volume ni réseau autre que ceux de MES-17 n'est créé ou
  touché. La garde mémoire existante du harnais est conservée.
- **Appel au repos seulement.** `gdb call` gèle le processus et prend les verrous d'arène : il
  n'est appelé qu'au repos, sans job en cours et sans requête en vol. `gdb` tourne sous un délai
  borné.
- **En cas de blocage** : `detach`, puis redémarrage du conteneur. Le parcours est déclaré **en
  échec**, et n'est **jamais remplacé par un zéro**.

### 7. Interdits

Aucune correction n'entre dans ce travail. En particulier :

- aucun `malloc_trim` dans le code, l'image, `crible.toml` ou une valeur par défaut ;
- `libc` exclue, et tout profil de tas (heaptrack, massif) exclu ;
- aucune mesure de latence, ni du coût d'un `malloc_trim` périodique ;
- aucun `format_version`, aucune route, aucun invariant ;
- données brutes seulement sous `data/mes17/raw/<date>/` ;
- aucune conclusion de production.

Toute correction demande un **ADR distinct**. Le propriétaire dira alors si elle entre dans la 1.1.0
ou reste dans la liste de la v2 (décision du 2026-11-08).

### 8. Classe de compatibilité et tableau d'application

Classe **« sans risque »** au sens d'ADR-0143, décision 1
(`docs/adr/0143-politique-de-version-de-la-ligne-1-x-et-procedure-de-montee-et-de-retour-arriere.md:122`) :
aucun code du serveur, aucun format, aucun contrat ne change.

| Document | Ce qui change |
|---|---|
| `docs/adr/README.md` | une ligne d'index, statut `Proposé` |
| `docs/mesures/MES-17-classes-de-memoire.md` | une **section datée**, écrite par `@mesureur` pendant la tâche ; aucune section antérieure réécrite |
| `tools/bench/mes17/` | extension du harnais et Dockerfile de l'image annexe |
| `docs/06`, `docs/08`, ADR-0151, ADR-0157, ADR-0158 | aucun changement |

### 9. Ce que cet ADR ne fait pas

Il ne modifie **ni ADR-0151, ni ADR-0157, ni ADR-0158, ni `docs/06`, ni `docs/08`**. L'exclusion
d'ADR-0157, décision 6, reste la règle écrite pour toutes les autres campagnes ; seule celle-ci s'en
écarte, et par le présent ADR. Un renvoi daté dans ADR-0157 ne sera écrit que sur décision du
propriétaire.

## Conséquences

- **Positives** : la dernière ligne ouverte par les deux premières campagnes reçoit un test direct,
  sans code, sans dépendance ajoutée au produit et sans toucher à la production. La valeur de retour
  de `malloc_trim(0)` et l'idempotence du second appel distinguent nettement « pages rendues » de
  « rien à rendre ». Un résultat positif ferme la piste « fragmentation » et désigne une correction
  ultérieure ; un résultat nul renforce la piste « structure vivante », qui demande alors un autre
  moyen.
- **Négatives / coûts** : deux parcours complets du protocole, avec l'instance de développement de
  Legifacile arrêtée, soit environ le coût de deux parcours de la campagne précédente. Une image
  annexe à construire, donc un téléchargement de base et de paquet. `gdb` dans un conteneur qui
  partage l'espace de PID et porte `SYS_PTRACE` est un geste d'exploitation lourd, borné à une
  machine de développement et à un conteneur jetable. Un `gdb call` peut bloquer le processus mesuré
  et coûte alors un redémarrage et un parcours perdu.
- **À surveiller** : si l'auto-test échoue, ou si `gdb` se bloque malgré le repos et le délai borné,
  la campagne ne conclut pas et son protocole se revoit par un ADR. Si la dispersion entre les deux
  processus dépasse la baisse observée, la campagne ne départage rien et le rapport doit le dire. Si
  `malloc_trim(0)` rend une part de l'anonyme retenu, la piste « fragmentation » se confirme et une
  correction devient une décision distincte ; si le premier appel rend `0`, la piste « structure
  vivante » reste seule et le moyen suivant change de nature.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Appeler `malloc_trim` depuis le code, avec `libc` | `ECART` à `docs/07` et à ADR-0080, décision 3 (`:80`) : `libc` n'est pas dans la liste des dépendances. Cela créerait en plus un chemin de code de production pour un essai unique, alors que la campagne est une mesure seule |
| Exposer une route de debug qui appelle `malloc_trim` | Contrat nouveau, donc un ADR de contrat (ADR-0105 pour les routes d'administration, ADR-0146 pour l'état des routes au chargement), une route à maintenir, et une surface d'exploitation ouverte pour un essai unique. La mesure ne demande aucun contrat public |
| `LD_PRELOAD` d'une bibliothèque qui appelle `malloc_trim` | Change l'environnement du processus mesuré : constructeur au démarrage, mappings supplémentaires, gestionnaire de signal non conçu pour cet usage. Le processus mesuré ne serait plus celui de la campagne précédente, et l'écart contaminerait la lecture |
| `mallopt` par `gdb` | Moins informatif : `mallopt` règle un paramètre, il ne rend pas les pages déjà libres. V2 l'a déjà couvert par `GLIBC_TUNABLES`, et la réserve technique d'ADR-0158 (`:60-66`) dit précisément ce que ce réglage ne rend pas |
| Trim après (b), comme témoin de l'état avant attache | Détruit la référence : il perturberait le tas avant l'attache et la comparaison (c)−(b) ne serait plus celle des campagnes précédentes (ADR-0157, `:178-179`) |
| Profil de tas tout de suite (heaptrack, massif) | Outil et accès absents de `docs/07`, et c'est la piste (1) des recommandations de MES-17, distincte de la piste (3) que le propriétaire a retenue. Le profil garde son propre ADR |
| Réutiliser le seuil de 45,3 MiB de la campagne du 2026-11-22 | Les comparaisons sont internes à une campagne (ADR-0157, `:101-102`) : un seuil venu d'une autre série de parcours ne décrit pas la dispersion de celle-ci |
| Poser `malloc_trim` dans l'image, `crible.toml` ou un défaut | Ce serait une correction de production, donc un ADR distinct (décision 7). Une campagne ne change pas ce que sert l'image |
| Nouvel identifiant de mesure, par exemple MES-18 | Une nouvelle campagne sur la même question est une section datée de la même mesure (`docs/mesures/README.md:5`) ; un identifiant nouveau couperait la comparaison interne |

## Références

- Ticket : `/Users/mattboll/MattCorp/legifacile/tickets-crible/05-memoire-non-rendue.md` (projet
  Legifacile, lecture seule ; ses numéros d'ADR et de mesure ne sont pas ceux de Crible).
- Mesure : `docs/mesures/MES-17-classes-de-memoire.md` — première campagne (conclusion `:135-162`,
  dont `:150` sur la nature de l'anonyme retenu) ; section « Campagne du 2026-11-22 » (`:173` et
  suivantes), recommandation 3 (`:546-549`) ; `docs/mesures/README.md:5` (section datée).
- ADR : ADR-0157 (protocole de la seconde campagne, décisions 5, 6 et 7, `:173-215` ; dispersion et
  comparaisons internes `:101-102` ; relevés `:126-127` ; disque `:226-228`), ADR-0158 (sens de
  l'effet et seuil `:90-106` ; réserve technique sur V2 `:60-66` ; état (e) `:116-118` ; suite par un
  nouvel ADR `:124-129`), ADR-0151 (chiffre qui fait foi, décisions 1 à 4, `malloc_trim` exclu
  `:196`), ADR-0143 (décision 1, classe « sans risque », `:122`), ADR-0080 (décision 3, `libc`
  absente de `docs/07`, `:80`), ADR-0105 (routes d'administration), ADR-0146 (routes pendant le
  chargement), ADR-0032 (production en lecture seule), ADR-0135 (mesure en conteneur sur la machine
  de développement).
- Documents de conception : `docs/07-stack-et-deploiement.md:25-53` (liste des dépendances, sans
  `libc` ni `gdb`) ; `docs/06-observabilite.md` (section « Mémoire », non modifiée) ;
  `docs/08-invariants-et-tests.md:82` (ligne MES-17, non modifiée).
- Outils et données : `tools/bench/mes17/` (harnais versionné à étendre, Dockerfile de l'image
  annexe) ; `data/mes17/raw/<date>/` (données brutes, gitignoré) ; image `crible:t05-suite-mes17`
  (`sha256:dc800f534ff50589038b0ca5d7a4fd9336a3259c45b70fdb00408b84366a8080`, construite depuis
  `31e9804`).
- Verdict : `@architecte`, session `ses_dfd42a6f5ffe6fPIx9ubhz3wBN`, 2026-11-22.
