# ADR-0158 — Sens de l'effet dans la règle de lecture de la seconde campagne de MES-17

- **Statut** : Proposé
- **Date** : 2026-11-22
- **Décideurs** : propriétaire
- **Proposé par** : agent `@greffier`, tâche `T05-SUITE-ATTRIBUER-ANONYME` (v1.1), sur la demande du propriétaire du 2026-11-22 et sur le verdict de `@architecte` du 2026-11-22
- **Remplace** : — (complète ADR-0157, **décision 5 seulement** ; le corps d'ADR-0157 n'est pas modifié)

## Contexte

### La règle écrite, et ce qu'elle ne dit pas

ADR-0157, accepté le 2026-11-21, porte le protocole de la seconde campagne de MES-17. Sa
décision 4 pose les variantes et la règle de V4 : « Si une seule variante a eu un effet, V4 reprend
cette variante seule ; si aucune n'en a eu, V4 ne tourne pas »
(`docs/adr/0157-protocole-de-la-seconde-campagne-de-mes-17-attribuer-la-memoire-anonyme-retenue.md:160-162`).
Sa décision 5 fixe la règle de lecture : « Une variante ne conclut que si **son effet sur l'anonyme
retenu dépasse la dispersion entre les deux processus de V0** » (`:175-176`), l'anonyme retenu étant
« anonyme de l'état (c) moins anonyme de l'état (b) » (`:178-179`). La table de correspondance de la
décision 5 associe ensuite chaque effet observé à une cause (`:185-190`), la ligne « Aucune variante
n'a d'effet » valant « structure vivante ou fragmentation » (`:190`), et renvoie à un nouvel ADR si
aucune variante n'a d'effet (`:192-193`).

Le texte de la décision 5 est d'**amplitude** : il compare la valeur absolue de l'écart à la
dispersion de V0. Son **intention est signée** : les trois hypothèses de la décision 4 sont des
causes de rétention, et la table lit un effet comme la confirmation d'une cause (`:185-190`). Une
hausse de l'anonyme retenu n'est donc pas l'effet cherché, et la table ne lui donne aucune ligne.

### Le verdict de `@architecte` du 2026-11-22

Verdict rendu en session `ses_dfd5f2ca0ffeu1Dwg171BrqHhW`, sur la demande de la tâche. Il n'est pas
un document du dépôt ; il est cité ici comme l'avis qui encadre la proposition.

- La décision 5 se lit de deux façons, l'amplitude et le sens signé ; les deux lectures ont des
  conséquences **opposées** : V4 avec V2 et V3, ou pas de V4. Verdict : `NON-COUVERT`.
- Combiner deux perturbations qui **augmentent** la rétention ne se lit sur aucune ligne de la
  table : la ligne « V2 et V3 ont un effet » n'existe pas, et son résultat serait une hausse.
- Pas d'erratum à ADR-0157 : cet ADR ne dit nulle part que V2 équivaut à `malloc_trim(0)`, donc il
  n'y a pas de phrase à corriger sur ce point.

### Les résultats des passes 1 et 2

Les deux passes de `@mesureur` du 2026-11-22 sont terminées, et leurs données brutes sont sous
`data/mes17/raw/2026-11-22/`, gitignorées. Le rapport daté de MES-17 n'est pas encore écrit. Les
chiffres ci-dessous sont les médianes de l'anonyme du cgroup, en MiB.

| Variante | (b) index créé | (c) six lots | (c)−(b) | Écart à la moyenne de V0 |
|---|---:|---:|---:|---:|
| V0-p1 | 1 543,7 | 2 494,8 | +951,1 | |
| V0-p2 | 1 489,3 | 2 485,7 | +996,4 | |
| V1 `glibc.malloc.arena_max=1` | 1 309,6 | 2 276,2 | +966,6 | −7,1 |
| V2 `glibc.malloc.trim_threshold=0:glibc.malloc.top_pad=0` | 1 314,3 | 2 367,9 | +1 053,6 | +79,8 |
| V3 `glibc.malloc.mmap_threshold=131072` | 1 329,4 | 2 359,7 | +1 030,3 | +56,5 |

La moyenne de V0 vaut 973,75 MiB et sa dispersion, c'est-à-dire l'écart entre ses deux processus,
45,3 MiB. Les trois tunables ont été prouvés présents par `ld.so --list-tunables` avant la
campagne. Aucun échange sur disque n'est intervenu. Le redémarrage avec l'index présent, état (e),
ramène l'anonyme entre 1 342,7 et 1 513,1 MiB selon la variante.

### La réserve technique sur V2

Sur glibc 2.36, `trim_threshold=0` ne rend que le haut du tas, c'est-à-dire l'arène principale, et
rétrécit le tas des autres arènes. `malloc_trim(0)`, depuis glibc 2.8, rend aussi les pages libres
entières au milieu des arènes, par `madvise`. V2 ne teste donc pas cette fragmentation-là. Cette
équivalence inexacte figure dans `docs/mesures/MES-17-classes-de-memoire.md`, recommandation 2
(`:157`), et dans le verdict de `@architecte` du 2026-11-21, pas dans ADR-0157.

### Le contexte d'exploitation

L'instance de développement de Legifacile, conteneur `legifacile-dev-crible-1`, est arrêtée pendant
la campagne pour libérer la VM de Docker. Chaque parcours supplémentaire du protocole occupe donc
cette VM, et l'arrêt de l'instance de développement en est le coût.

### Forme retenue : un complément, selon le précédent d'ADR-0113

Un ADR accepté ne se réécrit pas ; pour changer d'avis, on écrit un ADR nouveau
(`docs/adr/README.md:33-35`). Le précédent cité par `@architecte` est ADR-0113, qui porte
`Remplace : — (ne remplace aucune décision : complète ADR-0077, ADR-0102, ADR-0068 et ADR-0067, sans
modifier le corps d'aucun d'eux)` (`docs/adr/0113-charge-concurrente-inv-02-et-mes-16.md:7`). Le
présent ADR suit cette forme : il **complète** la décision 5 d'ADR-0157, pour en fixer le sens, et
il ne remplace aucune décision. ADR-0157 n'est pas modifié, pas même dans son index.

La forme du remplacement partiel, employée par ADR-0149 (`:7`) et ADR-0156 (`:7`) sous la forme
`Remplace : — (remplace en partie ADR-XXXX, décision N seulement)`, ne convient pas ici : aucune
clause écrite d'ADR-0157 n'est retirée. La décision 5 reste entière, et c'est son sens que le
présent ADR fixe, là où le texte laissait l'amplitude et le signe possibles.

## Décision

**Le sens de l'effet, dans la règle de lecture de la seconde campagne de MES-17, est une baisse.**

### 1. Un effet est une baisse

Une variante a un effet seulement si son anonyme retenu, c'est-à-dire (c)−(b), **baisse d'au moins
la dispersion entre les deux processus de V0**, mesurée par rapport à leur moyenne. Une hausse
au-delà du seuil est une **observation**, consignée dans le rapport, et jamais la confirmation
d'une cause. Le seuil de la campagne du 2026-11-22 est donc 973,75 − 45,3 = 928,45 MiB : une
variante n'a d'effet que si son (c)−(b) est inférieur ou égal à cette valeur.

### 2. La campagne du 2026-11-22 s'arrête à cinq parcours

Avec les écarts mesurés, V1 vaut −7,1 MiB, V2 +79,8 MiB et V3 +56,5 MiB à la moyenne de V0. Aucune
de ces valeurs n'est une baisse d'au moins 45,3 MiB : **aucune variante n'a d'effet**. V4, qui est
la combinaison des variantes qui ont eu un effet (ADR-0157, décision 4, `:158-162`), **ne tourne
pas**. La campagne compte donc **cinq parcours** — V0 deux fois, V1, V2 et V3 une fois — au lieu des
sept qu'ADR-0157 prévoyait (`:170-171`), et le rapport le dit.

### 3. Lecture, et les trois réserves obligatoires du rapport

La ligne « Aucune variante n'a d'effet » de la table d'ADR-0157 (`:190`) s'applique : la rétention
relève d'une structure vivante ou d'une fragmentation. Le rapport porte trois réserves :

- **V2 ne teste pas la fragmentation que `malloc_trim(0)` rendrait**, c'est-à-dire les pages libres
  au milieu des arènes, pour la raison technique exposée plus haut. La ligne « fragmentation »
  reste donc ouverte, et V2 ne la ferme pas.
- **L'état (e) est un fait, hors de la règle de lecture.** Le redémarrage avec l'index présent rend
  la rétention : elle n'est pas nécessaire pour servir l'index. Le rapport le consigne sans en
  tirer de conclusion sur la cause.
- **La baisse absolue de l'anonyme en (b) et en (c) sous les trois variantes est une observation.**
  Le chargement de la session coûte environ 200 MiB de moins sous V1, V2 et V3 que sous V0, et le
  rapport le dit sans conclusion : la règle de lecture porte sur (c)−(b), pas sur les niveaux
  absolus.

### 4. La suite demande un nouvel ADR

Toute campagne nouvelle — profil de tas, compteurs d'allocateur, essai de `malloc_trim(0)` — et
toute correction demandent un **nouvel ADR**, comme le posent les décisions 5 à 7 d'ADR-0157
(`:192-193`, `:199-208`, `:210-215`). Le propriétaire choisit la suite après le rapport, et il dit
si une correction entre dans la 1.1.0 ou reste dans la liste de la v2.

### 5. Classe de compatibilité

Classe **« sans risque »** au sens d'ADR-0143, décision 1
(`docs/adr/0143-politique-de-version-de-la-ligne-1-x-et-procedure-de-montee-et-de-retour-arriere.md:122`) :
aucun code, aucun format, aucun contrat ne change.

## Conséquences

- **Positives** : la règle de lecture n'a plus qu'une lecture possible, et elle s'accorde avec
  l'intention d'ADR-0157, où les trois hypothèses sont des causes de rétention. La campagne
  s'arrête à cinq parcours, ce qui évite deux exécutions complètes du protocole, soit environ
  28 minutes de VM pendant lesquelles l'instance de développement de Legifacile reste arrêtée. Les
  trois réserves du rapport empêchent de lire « structure vivante ou fragmentation » comme une
  cause identifiée.
- **Négatives / coûts** : le texte d'ADR-0157 garde son amplitude, donc un lecteur de cet ADR seul
  peut encore combiner V2 et V3. Le présent complément est la seule pièce qui fixe le sens, et il
  ne porte que sur la décision 5. La campagne s'arrête sans nommer la cause : la question du
  ticket 05 reste ouverte, et sa suite demande un ADR de plus.
- **À surveiller** : si une campagne ultérieure mesure une baisse d'au moins la dispersion sous un
  autre réglage, ou si un essai de `malloc_trim(0)` rend une part de l'anonyme retenu, la piste
  « fragmentation » se confirme et change la suite ; si un profil de tas montre une structure
  vivante, c'est une autre cause, et un autre ADR. Un rapport qui conclurait sur une hausse, ou qui
  omettrait l'une des trois réserves, contredirait cet ADR.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Lecture d'amplitude, donc V4 avec V2 et V3 | Sa finalité, confirmer un effet cumulé, ne se lit sur aucune ligne de la table d'ADR-0157 (`:185-190`) : une combinaison de deux perturbations qui augmentent la rétention n'y a pas de lecture. Elle coûte en plus deux parcours, environ 28 minutes de VM, avec l'instance de Legifacile arrêtée |
| Lire l'anonyme absolu en (c) au lieu de (c)−(b) | Cette lecture mêlerait l'effet des variantes sur le chargement de la session et la rétention de l'attache ; les trois variantes font baisser le niveau absolu d'environ 200 MiB, et la croissance de l'attache ne baisse sous aucune d'elles. La mesure fixée par ADR-0157, décision 5 (`:178-179`), reste (c)−(b) |
| Erratum de la décision 5 dans le texte d'ADR-0157 | Un ADR accepté ne se réécrit pas (`docs/adr/README.md:33-35`) ; la seule modification permise est le renvoi « Remplacé par ADR-NNNN » à l'acceptation d'un ADR qui le remplace, et ce complément ne remplace aucune décision |

## Références

- ADR : ADR-0157 (protocole de la seconde campagne de MES-17, décisions 4 et 5, `:160-162`,
  `:175-193` ; décisions 6 et 7, `:199-215`), ADR-0151 (chiffre de mémoire qui fait foi, protocole de
  MES-17 et « ne corrige rien »), ADR-0143 (décision 1, classe « sans risque », `:122`), ADR-0113
  (`:7`, forme du complément sans remplacement), ADR-0142 (allocateur et mémoire anonyme),
  ADR-0135 (mesure en conteneur sur la machine de développement), ADR-0032 (production en lecture
  seule).
- Processus : `docs/adr/README.md:33-35` (un ADR accepté n'est pas réécrit, forme du remplacement).
- Mesure : `docs/mesures/MES-17-classes-de-memoire.md`, recommandation 2 (`:157`) ; section datée à
  écrire après la campagne ; `docs/mesures/README.md:5` (une nouvelle campagne ajoute une section
  datée).
- Données : `data/mes17/raw/2026-11-22/` (gitignoré), un dossier par variante avec `summary.json`,
  `states/`, `logs/` et la preuve des tunables.
- Verdict : `@architecte`, session `ses_dfd5f2ca0ffeu1Dwg171BrqHhW`, 2026-11-22 ; verdict antérieur du
  2026-11-21 sur l'équivalence entre V2 et `malloc_trim(0)`.
