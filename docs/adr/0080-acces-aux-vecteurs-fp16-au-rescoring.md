# ADR-0080 — Accès aux vecteurs fp16 au rescoring

- **Statut** : Accepté (décision du propriétaire le 2026-10-27, tel que proposé)
- **Date** : 2026-10-27 (proposition et acceptation le même jour)
- **Décideurs** : propriétaire
- **Décision** : accepté tel que proposé le 2026-10-27. La mise en œuvre se fait par une tâche de **P4, avant le jalon J4** : `GROUPER-LECTURES-VECTEURS` ; ses voisines sont `REDUIRE-COUT-DES-CLES` (ADR-0079) et `UNIFIER-PARCOURS-SCAN` (ADR-0081).
- **Proposé par** : agent greffier, sur demande du propriétaire, à partir de la mesure MES-03 (`docs/mesures/MES-03-latence-rescoring.md`), de la mesure MES-09 (`docs/mesures/MES-09-ram-reelle.md`) et de l'analyse de `@architecte` reprise par l'orchestrateur
- **Remplace** : —

## Contexte

### La question

MES-03 mesure le rescoring fp16 selon le protocole d'ADR-0077 et constate que son coût n'est pas arithmétique. À N = 1000, le produit scalaire sur des pages chaudes coûte **0,36 ms**, et le rescoring complet **29,8 ms** en p50 ; sur des pages réellement absentes, le seul produit scalaire coûte **85,0 ms**. À N = 5000, les mêmes chiffres sont **2,04 ms**, **61,1 ms** et **177,5 ms**. L'écart vient de la lecture des lignes `f16` sur disque, à raison d'environ une page par candidat.

ADR-0079, accepté le 2026-10-27, traite le coût des **clés** et renvoie explicitement l'**accès aléatoire aux vecteurs fp16** à un ADR séparé. C'est cet ADR.

La question tranchée ici est : **comment lire les N vecteurs fp16 du rescoring pour rapprocher son coût du calcul seul, sans changer le résultat, le format, ni la mémoire possédée.**

### Ce que la mesure établit

Chiffres de MES-03, mesurés sur `synth-large` (7,74 M vivants), moteur natif sur un MacBook Air M3, macOS 15.7.7, disque APFS interne :

| Poste | N = 1000 | N = 2000 | N = 5000 |
|---|---|---|---|
| pages de vecteurs touchées | 1 052 | 2 102 | 5 288 |
| pages par candidat | 1,05 | 1,05 | 1,06 |
| coût par page, série froide | 80,8 µs | 74,4 µs | 33,6 µs |
| produit scalaire seul, pages chaudes | 0,36 ms | 0,80 ms | 2,04 ms |
| produit scalaire seul, copie `F_NOCACHE` | **85,0 ms** | 156,4 ms | **177,5 ms** |
| moteur entier après 10 min d'inactivité | 117,4 ms | 159,1 ms | 245,7 ms |

Trois faits commandent la lecture de ce tableau.

- **Une page par candidat.** Un vecteur fp16 de 768 dimensions pèse 1 536 octets ; une page en fait 16 Kio sur Apple Silicon. Deux candidats ne partagent donc presque jamais une page : 5 288 pages pour 5 000 candidats, soit 1,06 page par candidat, et 5 à 6 % des pages portent deux candidats. Grouper les lectures ne réduit pas le nombre de pages ; cela ne peut que mieux recouvrir leur attente.
- **Le débit monte avec le travail en vol.** Le coût par page passe de 80,8 µs à N = 1000 à 33,6 µs à N = 5000, parce que les demandes se recouvrent. C'est le seul signal favorable du tableau : la profondeur de file est le levier disponible sans toucher au format.
- **Le froid reste hors budget.** Même à 33,6 µs par page, 5 288 pages valent 177,5 ms. Aucune réorganisation de la lecture ne ramène le froid sous la dizaine de millisecondes ; elle peut seulement le rapprocher du coût des pages, pas le supprimer.

MES-09 mesure l'autre moitié du problème : **1,12 Gio possédés par index** de 7,74 M passages, et **3,80 à 4,00 Gio de pages de vecteurs résidentes** après une passe, sur un RSS de 4,63 à 4,83 Gio. Les pages de vecteurs ne sont pas possédées : le système peut les reprendre, et MES-03 montre qu'il le fait.

### Le chemin actuel du code

Pour chaque candidat du rescoring :

- `dense::cosine` (`crates/crible-search/src/dense/mod.rs:151-160`) appelle `SegmentReader::vector_f16` (`crates/crible-store/src/segment.rs:571`) ;
- `vector_f16` lit la ligne par `row_bytes` (`segment.rs:691`) sur le mmap ;
- `decode_vector_f16` (`segment.rs:1050`) **alloue un `Vec` et copie 1 536 octets** ;
- `kernels::dot` calcule ensuite le produit scalaire sur ce vecteur.

La lecture est donc séquentielle dans le code, une ligne à la fois, et chaque ligne paie un défaut de page isolé. Il n'existe que **trois leviers** : moins de pages, plus de profondeur de file, ou sortir du disque. Le premier toucherait le format ou l'ordre physique ; le deuxième ne touche que la façon de lire ; le troisième touche la règle de mémoire.

L'analyse de `@architecte` qualifie les dix options examinées : l'option 1 est **conforme** à la conception ; les options 2 et 9 ne sont **couvertes par aucun document** et demandent donc une décision explicite ; les options 5, 6 et 8 sont des **écarts** à la règle de mémoire ou à ADR-0072 ; les options 3, 4, 7 et 10 sont déjà couvertes, hors sujet ou sans gain démontré.

### Les contraintes qui pèsent

- **Égalité au bit près avec EXACT.** ADR-0072, décision 3, exige des scores et un ordre identiques au bit près entre EXACT et le rescoring. INV-06 en est le test de référence (« pool ≤ seuil exact ⇒ résultat identique au calcul exact »). Toute lecture groupée doit alimenter le **même** `kernels::dot`, vecteur par vecteur ; elle ne peut pas changer l'ordre des additions ni la précision.
- **Le format ne change pas sans ADR.** Les segments, leur en-tête et leur version sont décidés par ADR-0006 et ADR-0069. Cet ADR ne les touche pas.
- **La règle de mémoire de `docs/02-architecture.md:204`** : « ce qui est parcouru en entier à chaque requête est en RAM ; ce qui est lu seulement pour quelques candidats reste sur disque (mmap) ». La ligne `docs/02:213` chiffre les vecteurs de référence fp16 à **13,2 Go sur disque, en mmap**, pour 8,58 M passages. Les sortir du disque demande de réviser cette règle par un ADR.
- **Le budget mémoire est tendu.** 1,12 Gio possédés par index, 2,20 à 2,30 Gio pour une instance avec la session fp32, soit 10 à 15 % au-dessus de la conception (MES-09).
- **La règle d'imbrication d'ADR-0079** : la connexion SQLite ne doit pas être tenue pendant les lectures de vecteurs.
- **La séquence d'ADR-0079.** Son profil par échantillonnage et ses deux mesures neutres (ordre croissant des iid, ligne fp16 empruntée au mmap au lieu d'être copiée) sont des **préalables**. Cet ADR ne les re-décide pas et ne les remplace pas.
- **ADR-0047** : toute charge ajoutée au démarrage se mesure avant d'être retenue.
- **ADR-0077** fixe le protocole de mesure de latence : N ∈ {1000, 2000, 5000}, 300 requêtes, 3 passes, froid par copies non cachées (`F_NOCACHE`, `mincore`), série après 10 minutes d'inactivité. MES-03 y ajoute une réserve : l'ouverture du searcher vérifie le sceau blake3 du fichier de vecteurs entier et le réchauffe, si bien que le froid du **moteur** n'est pas mesurable aujourd'hui.
- **N = 5000 par défaut** (ADR-0078) : la campagne doit chiffrer N = 5000, pas seulement N = 1000.

### Ce que cet ADR ne traite pas

Le coût des **clés**, qui appartient à ADR-0079 ; la valeur de `rescore_candidates`, qui appartient à ADR-0078 ; le **format** des segments et une éventuelle quantification, qui touchent ADR-0006, ADR-0069 et ADR-0072 ; les cibles de latence en conteneur et le protocole de froid sous Linux, qui appartiennent à P8. La compaction et l'ordre physique selon `cluster_by` restent ce qu'ADR-0006 en dit.

## Décision

> Statut **Accepté** (décision du propriétaire le 2026-10-27, tel que proposé) : les précisions ci-dessous font la décision ; leur mise en œuvre demande la tâche `GROUPER-LECTURES-VECTEURS` (P4, avant J4).

**La décision proposée est de retenir la lecture groupée et parallèle des N lignes avant le calcul (option 1), de mesurer `madvise(MADV_RANDOM)` dans la même campagne comme contrôle bon marché (option 2), et de garder le `pread` groupé (option 9) en repli. Les vecteurs en RAM, `mlock` et la quantification int8 du rescoring restent écartés en l'état.**

Précisions :

1. **Séquence après ADR-0079.** La campagne part du profil et des deux mesures neutres d'ADR-0079 : ils disent quelle part du rescoring revient aux vecteurs, et la ligne fp16 empruntée peut déjà réduire la copie. Aucune option d'ADR-0079 n'est re-décidée ici. Si le profil attribue l'essentiel des 25,4 ms inexpliquées aux défauts de page, cet ADR devient prioritaire, mais l'ordre de traitement d'ADR-0079 reste le sien.
2. **Option 1 retenue : lecture groupée et parallèle, puis calcul.** Les N lignes sont lues par lots, en parallèle (rayon), dans un tampon transitoire qui vaut au plus **7,7 Mio à N = 5000** (5 000 × 1 536 octets), puis le calcul reprend vecteur par vecteur avec le **même** `kernels::dot` et le même tri final. Sa taille suit le lot en vol, et il est libéré après la requête. Il est anonyme et transitoire : il n'entre pas dans les 1,12 Gio possédés de MES-09, mais il apparaît dans le RSS anonyme pendant la requête. La connexion SQLite n'est pas tenue pendant ces lectures. Le format ne change pas. La borne **optimiste** annoncée par `@architecte`, à profondeur 8, est de **15 à 25 ms à N = 1000** (au lieu de 85,0) et de **60 à 90 ms à N = 5000** (au lieu de 177,5) ; elle est à mesurer, pas à promettre.
3. **Option 2 testée comme contrôle, pas retenue d'emblée.** `madvise(MADV_RANDOM)` sur la région mmap coupe le read-ahead que le noyau fait pour un accès séquentiel et qui est inutile ici. Le gain n'est pas chiffré, la sémantique est réversible, et `libc` n'est pas dans la liste de dépendances de `docs/07` : si la mesure la retient, une dépendance s'ajoute par un ADR avant la fusion. Sinon, elle est retirée sans trace dans le code.
4. **Option 9 en repli, pas en principal.** Le `pread` groupé au lieu du mmap pour le rescoring est de la même classe que l'option 1, avec une profondeur de file maîtrisée explicitement. Il n'est pas retenu pour la mise en œuvre : `docs/02:213` dit mmap, et changer ce point demande une décision explicite. Si l'option 1 plafonne sous sa borne, le repli est instruit avec les mêmes contrôles, et son adoption passe par une précision dans cet ADR ou par un nouvel ADR.
5. **Options écartées en l'état.** Les vecteurs en RAM (option 6) et `mlock` (option 5) contredisent la règle de mémoire de `docs/02:204` et `:213` : 13,2 Go par index contre 1,12 Gio possédés. La quantification int8 du rescoring (option 8) contredit ADR-0072, décision 3, et INV-06, et ne réduit pas le nombre de pages au froid. Le préchargement `MADV_WILLNEED` (option 3) recoupe l'ordre des iid d'ADR-0079 et ne couvre que les 5 à 6 % de pages qui portent deux candidats sur un pool complet. Le préchauffage à l'ouverture (option 4) est déjà en place de fait par la vérification blake3 et ne vaut rien en froid réel ; ADR-0047 impose de mesurer avant d'y ajouter du travail. L'ordre physique selon `cluster_by` (option 7) est déjà fait par la compaction (`docs/02:177-180`). Baisser N (option 10) appartient à ADR-0078 et dégraderait la fidélité.
6. **Validation de toute mise en œuvre.** Cinq contrôles, tous exigés :
   - rejeu de MES-03 au protocole d'ADR-0077 : N ∈ {1000, 2000, 5000}, 300 requêtes, 3 passes à chaud et une passe par série froide, série froide `F_NOCACHE` avec `mincore`, série après 10 min d'inactivité ;
   - égalité **au bit près** RESCORER-FP16 / EXACT (ADR-0072, décision 3) ;
   - contrôle top-10 du harnais de MES-01, avec **0 écart** ;
   - RSS en trois états (MES-09) pour prouver que la mémoire possédée reste à **1,12 Gio** ;
   - une série nouvelle, **« searcher déjà ouvert, cache déplacé après l'ouverture »** : le froid du moteur n'est pas mesurable aujourd'hui, la vérification blake3 à l'ouverture réchauffant le fichier (MES-03 série B : 56 % de résidence à N = 5000). Cette série s'ajoute sans réécrire ADR-0077 : elle ne change ni N, ni le nombre de requêtes, ni les percentiles.
   - **Note du 2026-10-28 (ADR-0084, accepté)** : en P4, la série froide `F_NOCACHE` n'est **pas rejouable sur le moteur**. `SegmentReader::open` lit le fichier de vecteurs entier pour vérifier son sceau blake3, ce qui le réchauffe avant la première requête (`crates/crible-store/src/segment.rs:445-447`) ; un `F_NOCACHE` posé après l'ouverture ne rendrait donc pas les pages froides (MES-03, écart 2, `:1367-1377`). La série « searcher déjà ouvert, cache déplacé après l'ouverture » **fait foi** pour juger les gains du rescoring jusqu'à P8, avec la résidence des pages de candidats publiée pour chaque point (ADR-0084, décisions 1 et 2). La série de 10 minutes reste secondaire. Les autres contrôles de ce point sont inchangés.
7. **Conteneur de P8 : signalé, pas décidé ici.** Sous Linux x86_64, les pages font 4 Kio : il reste environ une page par candidat, avec quatre fois moins d'octets par page, et le coût froid suit les IOPS du volume. `F_NOCACHE` n'existe que sur macOS ; le froid Linux (`posix_fadvise(DONTNEED)`, cgroup) n'est défini nulle part. `madvise`, le read-ahead et `mlock` ont d'autres sémantiques. La fiche `MESURER-EN-CONTENEUR-MAC` (`docs/phases/P8-docker-et-deploiement.md:57`) ne rejoue, pour la recherche, que MES-02, MES-05 et MES-09 (le volet recette de MES-08 s'y ajoute, hors recherche) : valider le rescoring en conteneur demanderait d'étendre la fiche. C'est une remarque pour P8, pas une décision de cet ADR.
8. **Mise en œuvre.** Une tâche à créer si le propriétaire accepte cet ADR, après la mise en œuvre d'ADR-0079. La campagne de mesure peut précéder le code retenu : la profondeur de file, le gain de `MADV_RANDOM` et la part restante des défauts de page se mesurent sur un prototype avant de figer le tampon et le découpage des lots.

Ce qui n'est **pas** décidé ici : le format des segments, une quantification des vecteurs de référence, une révision de la règle de mémoire de `docs/02`, les cibles de latence en conteneur, le protocole de froid sous Linux, et le coût des clés, qui appartient à ADR-0079.

## Conséquences

- **Positives** :
  - les lectures de pages se recouvrent au lieu de s'additionner, ce qui attaque le seul levier disponible sans toucher au format ni à la mémoire possédée ;
  - le calcul et le tri ne changent pas : même `kernels::dot` par vecteur, même tri final, donc égalité au bit près avec EXACT par construction (ADR-0072, décision 3) ;
  - le format et `format_version` restent intacts, et aucune migration n'est nécessaire ;
  - la mémoire possédée au repos reste à 1,12 Gio par index ; le tampon est transitoire et borné par lot ;
  - `MADV_RANDOM` est un contrôle bon marché et réversible, qui peut expliquer une partie du coût sans engagement de dépendance ;
  - le `pread` groupé reste un repli documenté si le mmap plafonne.

- **Négatives / coûts** :
  - le tampon ajoute **7,7 Mio** de RSS anonyme au pic d'une requête à N = 5000, même s'il n'est pas possédé au repos ;
  - les lectures parallèles prennent des fils à rayon, déjà utilisés par le scan binaire à 8 fils : le p95 peut se dégrader sous charge, et la campagne doit mesurer la contention, pas seulement la médiane ;
  - le code de `SegmentReader` gagne un chemin de lecture par lots et un découpage de tampon, à écrire, à documenter et à tester ;
  - le gain est borné par le nombre de pages : environ une par candidat. Grouper les lectures ne fusionne pas les pages ; seule la profondeur de file les recouvre ;
  - à N = 5000, la borne optimiste est de 60 à 90 ms à froid, toujours très au-dessus du budget indicatif : cet ADR rapproche du coût des pages, il ne rend pas le froid bon marché ;
  - il ne réduit rien du coût des clés, traité par ADR-0079 ;
  - `MADV_RANDOM` n'a pas la même sémantique sous Linux, et `libc` ajouterait une dépendance absente de `docs/07` ;
  - les chiffres de MES-03 varient d'un facteur deux d'une exécution à l'autre sur la machine de mesure : une campagne non entrelacée peut conclure à tort.

- **À surveiller** :
  - si le gain mesuré reste loin de la borne de 15 à 25 ms à N = 1000, la profondeur de file ne suffit pas et le repli `pread` (option 9) devient le sujet ;
  - si le RSS anonyme mesuré en trois états (MES-09) dépasse 1,12 Gio au repos, le tampon ou les lots sont mal dimensionnés ;
  - si le p95 se dégrade pendant que la p50 s'améliore, la contention avec le scan est le vrai coût et la profondeur doit baisser ;
  - si un pool filtré par `cluster_by` montre beaucoup plus de pages partagées que 5 à 6 %, le préchargement (option 3) redevient intéressant sur ce cas seulement ;
  - si `MADV_RANDOM` donne un gain instable ou propre à macOS, il n'est pas gardé et `libc` n'entre pas dans les dépendances ;
  - si le froid en conteneur ne suit pas le nombre de pages mais les IOPS du volume, la cible de P8 se discute en IOPS, pas en pages ;
  - si la vérification blake3 à l'ouverture continue de réchauffer le fichier, aucune série froide du moteur n'est interprétable sans la nouvelle série « cache déplacé après l'ouverture ».
  - **Note du 2026-10-28 (ADR-0084, accepté)** : la série `F_NOCACHE` n'est **pas rejouable sur le moteur** en P4, la vérification du sceau blake3 à l'ouverture réchauffant le fichier (`crates/crible-store/src/segment.rs:445-447`). La série « searcher déjà ouvert, cache déplacé après l'ouverture » **fait foi** jusqu'à P8, avec la résidence des pages de candidats publiée pour chaque point, et aucun chiffre de P4 n'est présenté comme une latence à froid (ADR-0084, décisions 1, 2 et 7). Le point ci-dessus reste le signal qui rouvre la question : si le moteur cesse de réchauffer le fichier, `F_NOCACHE` redevient applicable.
  - **Note du 2026-10-28 (ADR-0081, accepté, décision 7)** : le scan binaire n'a plus qu'un parcours, le clairsemé, qui suit l'itérateur trié de la bitmap du pool et rend donc les candidats déjà en **ordre croissant des iid** (ADR-0081, décisions 1 et 7). Cet ordre sert la localité que la décision 2 d'ADR-0079 recherche et rend le préchargement `MADV_WILLNEED` (option 3 ci-dessus) mesurable sur un ordre connu. Le corps de cet ADR, ses options et ses contrôles ne changent pas.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Garder la copie d'un vecteur par candidat et ne rien changer | C'est l'état mesuré : 85,0 ms à froid à N = 1000 et 177,5 ms à N = 5000 pour le seul produit scalaire, contre 0,36 et 2,04 ms sur pages chaudes ; l'écart est un coût de défauts de page, pas de calcul |
| Retenir seulement `madvise(MADV_RANDOM)` (option 2) | Gain non chiffré, qui ne coupe que le read-ahead et n'ajoute aucune profondeur de file ; insuffisant si l'essentiel du coût est l'attente isolée de chaque page. Testé comme contrôle, pas comme remède |
| Précharger les pages des candidats (`MADV_WILLNEED`, option 3) | Recoupe l'ordre des iid d'ADR-0079 et ne couvre que les 5 à 6 % de pages qui portent deux candidats sur le pool complet ; plus fort seulement sur un pool filtré par `cluster_by`, cas non général |
| Préchauffer à l'ouverture (option 4) | Déjà en place de fait : `SegmentReader::open` relit le fichier pour vérifier le sceau blake3 ; nul en froid réel, et ADR-0047 exige une mesure avant d'ajouter du travail au démarrage |
| `mlock` des vecteurs (option 5) | Écart à la règle de mémoire (`docs/02:204`, `:213`) : 13,2 Go par index contre 1,12 Gio possédés, et des pages qui ne sont plus récupérables sous pression |
| Garder les vecteurs en RAM (option 6) | Même écart, même ordre de grandeur ; MES-09 montre que les pages de vecteurs sont précisément ce que le système reprend, et MES-03 que le moteur paie alors 179,0 ms à N = 1000 |
| Quantifier le rescoring en int8 (option 8) | Écart à ADR-0072, décision 3, et à INV-06 : les scores changeraient. Le gain au froid reste faible, puisqu'il reste environ une page par candidat |
| `pread` groupé comme solution principale (option 9) | `docs/02:213` dit mmap ; même classe que l'option 1 avec plus de mécanique de file. Gardé en repli si l'option 1 plafonne |
| Ordonner physiquement les vecteurs selon `cluster_by` (option 7) | Déjà fait par la compaction (`docs/02:177-180`) : il n'y a rien à décider, seulement à vérifier que l'ordre est bien celui des lectures |
| Baisser N pour réduire le nombre de pages (option 10) | N = 5000 est décidé (ADR-0078) pour la fidélité ; baisser N dégraderait le rappel au lieu de réduire un coût parasite |
| Changer le format pour ranger plusieurs vecteurs par page | Touche ADR-0006 et ADR-0069, et n'est pas nécessaire si la profondeur de file suffit ; un tel changement demande son propre ADR, avec migration et `format_version` |
| Augmenter la profondeur de file sans mesurer | Peut saturer les IOPS du volume et prendre des fils au scan binaire ; la campagne mesure la profondeur, elle ne la fixe pas d'avance |
| Tenir la connexion SQLite pendant les lectures groupées de vecteurs | Contredit la règle d'imbrication d'ADR-0079 et rend la latence dépendante du verrou d'écriture |

## Références

- Mesures : `docs/mesures/MES-03-latence-rescoring.md` — tableau `:170-189`, conclusion `:195-227`, réserves `:229-238`, proposition `:240-247` ; détail par requête dans `MES-03-latence-rescoring.2026-10-27.json`. `docs/mesures/MES-09-ram-reelle.md:105-108`, `:119-121`, `:126-129` (1,12 Gio possédés, 3,80 à 4,00 Gio de pages de vecteurs). `docs/mesures/MES-01-fidelite-scan-binaire.md` (harnais top-10).
- Code : `crates/crible-search/src/dense/mod.rs:151-160` (`cosine`) ; `crates/crible-store/src/segment.rs:571` (`vector_f16`), `:691` (`row_bytes`), `:1050` (`decode_vector_f16`).
- Conception : `docs/02-architecture.md:177-180` (compaction et ordre `cluster_by`), `:204` (règle de mémoire), `:213` (vecteurs fp16 en mmap, 13,2 Go) ; `docs/03-recherche.md:144-151` (budget du rescoring et mesures) ; `docs/08-invariants-et-tests.md` (INV-06, MES-03) ; `docs/phases/P8-docker-et-deploiement.md:57` (`MESURER-EN-CONTENEUR-MAC`).
- ADR liés : ADR-0006 (segments, pierres tombales, compaction), ADR-0047 (mesure avant tout remède au démarrage), ADR-0069 (format de `codes.bin`, précédent de changement de format), ADR-0072 (produit scalaire unique, égalité au bit près, décision 3), ADR-0077 (protocole de latence de la recherche dense), ADR-0078 (N = 5000 par défaut), ADR-0079 (coût des clés au rescoring, accepté le 2026-10-27).
