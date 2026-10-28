# ADR-0083 — Suites de REDUIRE-COUT-DES-CLES : seule la lecture des clés après le tri est livrée

- **Statut** : Accepté (accepté par anticipation le 2026-10-28, par délégation du propriétaire pour l'enchaînement `UNIFIER-PARCOURS-SCAN`, `REDUIRE-COUT-DES-CLES` et `GROUPER-LECTURES-VECTEURS`, à confirmer avec lui au point de fin des trois tâches)
- **Date** : 2026-10-28 (proposition et acceptation anticipée le même jour)
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, sur demande du propriétaire, à partir de la section « Rejeu après REDUIRE-COUT-DES-CLES (ADR-0079) » de `docs/mesures/MES-03-latence-rescoring.md` (2026-10-28) et du verdict de `@architecte` sur la fiche `REDUIRE-COUT-DES-CLES`
- **Remplace** : —

## Contexte

### La question

ADR-0079 a rangé le coût des clés au rescoring en trois leviers ordonnés : un profil, puis deux mesures neutres — (2a) parcourir les candidats dans l'ordre croissant des iid, et (2b) emprunter la ligne `f16` au mmap au lieu de la copier —, puis la lecture des clés après le tri par score (b), avec la lecture par lot (a) en repli. Sa décision 2 dit que (2a) et (2b) sont « deux mesures neutres », **mesurées séparément puis ensemble** (`docs/adr/0079-cout-des-cles-au-rescoring.md:71`). Elle ne dit pas qu'elles doivent rester dans le code.

La fiche `REDUIRE-COUT-DES-CLES` les reprend en livrable (`docs/phases/P4-recherche-rapide.md:164`), **sans la clause de retrait** que porte la fiche voisine pour `madvise(MADV_RANDOM)` (`:185` : « sinon il est retiré sans trace dans le code »). La question est donc : que livre-t-on quand la mesure dit que deux des quatre changements coûtent ou ne paient pas ?

### Ce que le rejeu établit

Le rejeu du 2026-10-28 est consigné en section datée de MES-03 (`docs/mesures/MES-03-latence-rescoring.md:768-1094`). Il construit cinq variantes hors de l'arbre de travail (`base`, `2a`, `2b`, `2a-2b`, `final`), avec deux rondes d'ordre alterné, et applique la règle de bruit d'ADR-0082, décision 3 : un écart ne compte que s'il dépasse l'amplitude des écarts de la variante d'une passe à l'autre. La machine est sous pression mémoire déclarée (swap occupé à 4,85 Gio, 135 Mio de pages libres au départ, `records.sqlite` résident à 1,7 à 42 %) : **seuls les écarts entre variantes mesurés dans les mêmes rondes sont comparables entre eux** (`:945-951`).

| Variante | N | p50 (ms) | écart vs `base` | dispersion propre | verdict |
|---|---|---:|---:|---:|---|
| `2a` | 1000 | 69,1 | **+6,6 %** | 4,7 % | **perte** |
| `2b` | 1000 | 63,6 | −1,8 % | 23,1 % | match nul |
| `2a-2b` | 1000 | 69,8 | **+7,7 %** | 5,8 % | **perte** |
| `2a` | 5000 | 569,4 | **+10,2 %** | 4,2 % | **perte** |
| `2b` | 5000 | 509,9 | −1,3 % (6,9 ms) | 1,1 % | gain marginal |
| `2a-2b` | 5000 | 567,5 | **+9,8 %** | 3,5 % | **perte** |

Quatre faits portent la lecture :

- **(2a) est une perte reproductible.** Elle dépasse sa propre dispersion aux deux N et se reproduit dans les deux ordres de ronde ; `2a-2b` la reproduit presque à l'identique (+7,7 % et +9,8 %), donc elle est portée par le tri par iid, pas par (2b). La localité attendue n'apparaît pas ; la cause n'est pas attribuée (aucun profil de (2a) n'a été fait) et **le parcours par iid ne peut pas être décrit comme neutre** (`MES-03:1058-1063`).
- **(2b) ne se distingue pas du bruit.** Match nul à N = 1000, gain marginal de 6,9 ms à N = 5000. Rapporté au profil à chaud, qui borne la copie et le décodage à 0,14 à 0,36 µs par candidat, soit moins de 0,1 ms par millier de candidats, un gain de 6,9 ms pour 5 000 candidats n'est pas dans l'ordre de grandeur de ce que la copie coûte (`MES-03:1064-1071`). Or (2b) introduit un emprunt au mmap, donc un `unsafe` dans `crates/crible-store/src/segment.rs` (`vector_f16`).
- **(b) fait tout le gain, mais sa part n'est pas isolée.** Dans `final` (2a + 2b + b), le p50 tombe de 64,7 à **9,8 ms** à N = 1000 (−84,9 %), de 100,4 à **11,9 ms** à N = 2000 (−88,2 %) et de 539,5 à **170,7 ms** à N = 5000 (−68,4 %) (`MES-03:1011-1017`). `final` rend exactement 100 résultats par requête et sa troisième passe vaut 0,8 ms à N = 1000 : la lecture des clés n'est plus le poste, l'accès aux vecteurs le devient. Le rapport dit lui-même que **la part de (b) seule n'est pas mesurée** (`:1081-1085`), `final` contenant aussi (2a) et (2b).
- **L'égalité est préservée sur `final`.** 0 différence sur **90 000 entrées** (clé, score comparé par son motif binaire, ordre, `doc_id`, `ord`) entre `final` et `base` ; 0 écart sur les **1 500 réponses-requête** du top-10 de MES-01 (30 points, 6 structures × 5 valeurs de N) contre les fichiers enregistrés par sa campagne (`MES-03:896-936`, `:1072-1075`).

### Ce que le profil du rescoring ajoute

Les deux sections « Profil du rescoring » de MES-03 (`:249-503` et `:504-767`) établissent que le coût suit **le nombre de lignes lues**, pas le nombre d'allers-retours : préparation des requêtes, verrous et emprunts de connexion valent ensemble 2,320 ms (1,48 %) à N = 1000 et 11,188 ms (1,51 %) à N = 5000, quand les lectures de pages valent 58,06 % et 63,21 % (`:383-403`). La condition qu'ADR-0079, décision 4, pose pour (a) — « si le profil montre que le coût est dans les N allers-retours » — **n'est donc pas remplie**, et (a) n'est pas mise en œuvre : le code est déjà conforme à ADR-0079 sur ce point. Il n'y a rien à décider ni à livrer de ce côté.

Le profil borne aussi (2b) par avance : dans `vector_f16`, le temps est dans le service des pages, pas dans l'instruction de copie (`_platform_memmove` n'apparaît que 12 fois sur 6 220 échantillons, `:421-436`). Le rejeu ne fait que confirmer cette borne.

### Le verdict de `@architecte`

- Le **retrait de (2a) et (2b) est conforme à ADR-0079** : sa décision 2 décide une mesure, pas une conservation ; son alternative « Ordonner les candidats par iid et s'arrêter là » est déjà écartée (`ADR-0079:117`, « conserve au mieux le gain de localité, qui n'est pas chiffré »).
- Le retrait est un **ECART à la fiche `:164`**, qui les liste en livrables sans clause de retrait.
- Rien d'autre n'est à décider : ADR-0081, point 7 (« à écrire et non à décider ici », `docs/adr/0081-parcours-du-scan-binaire-un-seul-parcours-clairseme.md:60`), et ADR-0080, option 3, déjà écartée (`docs/adr/0080-acces-aux-vecteurs-fp16-au-rescoring.md:82`), ne demandent aucune suite ici.
- **La mesure de (b) seul est exigée** : `AGENTS.md:39` (« aucune affirmation de performance sans un rapport de `@mesureur` ») et ADR-0079, décision 6 (`:75`), qui demande le rejeu, l'égalité au bit près, le top-10 à 0 écart et le test des égalités à la frontière. Le gain reporté ne peut pas être celui de `final`.
- Les **contrôles d'égalité doivent être rejoués sur la variante livrée**, puisqu'ils ont été faits sur `final`, qui contient des changements qu'on retire.

### Ce que la mesure ne tranche pas

Aucun chiffre de la section n'est transposable à la campagne du 2026-10-27 ni à un conteneur : la machine était sous pression mémoire et les valeurs absolues de `base` valent ×2,2 et ×8,8 celles de la campagne. Le budget « < 1 ms à chaud » de `docs/03-recherche.md:144` n'est atteint que par la troisième passe de `final` (0,8 ms), pas par son p50 combiné, et la machine n'est pas celle de P8.

## Décision

**Seule (b), la lecture des clés après le tri par score, est livrée par `REDUIRE-COUT-DES-CLES`. Les deux mesures neutres (2a) et (2b) sont retirées du code sans trace, et le gain chiffré de la tâche est celui de cette variante livrée, mesuré séparément.**

Précisions :

1. **Ce qui est livré.** (b) telle qu'ADR-0079, décision 3 (`:72`) la décrit : tri intermédiaire sur `(score, iid)`, clés lues pour les `k` résultats, pour les candidats que consomme le regroupement (ADR-0075) et pour **toutes** les égalités de score à la frontière ; tri final sur `(score décroissant, clé textuelle croissante)` ; l'iid ne sort jamais du moteur (`AGENTS.md`, règle 11).
2. **Ce qui est retiré, sans trace.** (2a) et (2b) disparaissent du code : **ni option, ni configuration, ni drapeau**. (2a) est une perte mesurée de 6,6 % et 10,2 %. (2b) ne paie pas l'`unsafe` qu'elle introduit : son gain est dans le bruit, et la règle 10 d'`AGENTS.md` demande un `unsafe` justifié, la règle 12 une implémentation sans abstraction ni configuration inutile. La clause de retrait sans trace qu'ADR-0080 écrit pour `madvise` (`:80`) est la même règle, appliquée ici.
3. **(a) n'est pas mise en œuvre.** La condition d'ADR-0079, décision 4, n'est pas remplie par le profil (1,5 % du coût). Aucun code n'est écrit pour (a), conformément à la fiche qui la laisse en repli.
4. **La condition d'« À surveiller » d'ADR-0079 est infirmée** (`ADR-0079:100`) : l'ordre des iid n'apporte aucun gain de latence tel que mesuré, il en coûte. **Aucune règle n'est donc écrite dans `docs/03`.** Le constat est noté ici ; ADR-0079, ADR-0080 et ADR-0081 gardent leur corps, puisqu'un ADR accepté ne se réécrit pas.
5. **Validation du code livré** (ADR-0079, décision 6, `:75`) : une mesure de confirmation de (b) seul contre `base`, et **contre `final`** pour situer l'effet des deux mesures retirées, à N ∈ {1000, 2000, 5000}, en passes alternées, avec la règle de bruit d'ADR-0082, décision 3, et la résidence des deux fichiers (`records.sqlite`, vecteurs `f16`) **déclarée par série**, comme le rejeu du 2026-10-28 le fait. Les contrôles sont rejoués sur la variante livrée : égalité au bit près, top-10 du harnais de MES-01 à **0 écart**, INV-06, et le test des égalités de score à la frontière avec vecteurs dupliqués. **Le gain chiffré que la fiche exige est celui de cette variante**, pas celui de `final`.
6. **Pas de mesure d'ordre ajoutée à `GROUPER-LECTURES-VECTEURS`.** Sa fiche n'en contient pas ; l'ajouter engagerait une décision qui appartient au propriétaire, et ADR-0080 traite des vecteurs, pas de l'ordre des candidats. Le résultat de (2a) est disponible ici pour qui voudra rouvrir la question.

Ce qui n'est **pas** décidé ici : la révision d'ADR-0079, d'ADR-0080 ou d'ADR-0081 ; la réécriture de la fiche `REDUIRE-COUT-DES-CLES` au-delà de la note d'application ; les clés en RAM (c) et la colonne de clés en mmap (d), toujours écartées pour l'instant ; le budget de `docs/03-recherche.md:144`, jugé en P8 ; et la suite de `GROUPER-LECTURES-VECTEURS`.

## Conséquences

- **Positives** :
  - le code livré ne garde **que ce qui est mesuré gagnant** : une variante, un mécanisme, aucune configuration à maintenir ;
  - la supposition de la décision 2 d'ADR-0079 — deux changements « neutres » — est **tranchée par la mesure**, dans le sens qu'elle n'avait pas prévu, et le fait est écrit au lieu de rester dans un rapport ;
  - le retrait supprime un `unsafe` et une allocation par candidat de moins à défendre en revue ;
  - le gain reporté devient attribuable : la mesure de confirmation sépare (b) des deux mesures retirées, ce que le rejeu de `final` ne permettait pas ;
  - les contrôles d'égalité sont rejoués sur le code réellement livré, donc la preuve porte sur ce qui reste.
- **Négatives / coûts** :
  - la tâche coûte **une campagne de mesure de plus** avant d'être déclarée `FAIT` : trois variantes (`base`, `b` seul, `final`), trois N, deux rondes ;
  - la mesure de confirmation est faite sur la même machine sous pression mémoire : elle donnera des écarts relatifs fiables et des valeurs absolues non transposables ;
  - (2a) est retiré avec une cause **non attribuée** (aucun profil de la variante) : on sait qu'il coûte, pas pourquoi ; le motif du retrait est donc la mesure, pas l'explication ;
  - (2b) est retiré alors qu'un gain marginal apparaît à N = 5000 (6,9 ms) : on renonce à un gain non démontré pour ne pas porter un `unsafe`, ce qui est un choix, pas une constatation ;
  - `final`, qui portait le gain de 84,9 % et 68,4 %, n'est plus la variante livrée : les chiffres publiés du rejeu du 2026-10-28 décrivent désormais un état qui n'existe pas dans le code.
- **À surveiller** :
  - **le gain de (b) seul** : s'il s'effondre très en dessous de celui de `final`, la perte vient du retrait de (2a) — un tri dont on sait qu'il coûte — ou de (2b), et il faudra le dire plutôt que de revenir à `final` ;
  - **les égalités de score à la frontière** sur un jeu réel : si leur nombre dépasse largement `k`, le gain de (b) s'effondre vers N et (a) redevient le levier, comme ADR-0079 le prévoit (`:101`) ;
  - **le poste restant** : à N = 5000, la lecture des vecteurs `f16` domine une fois les clés lues ; c'est le sujet de `GROUPER-LECTURES-VECTEURS`, et la confirmation de (b) seule dira quelle part lui revient ;
  - **la tenue du budget de `docs/03-recherche.md:144`** : elle n'est pas atteinte par un p50 sur cette machine, et P8 la jugera en conteneur ;
  - **une réouverture de l'ordre des iid** si `GROUPER-LECTURES-VECTEURS` change le mode de lecture des vecteurs : le résultat de (2a) a été mesuré avec la lecture page par page, et un remède aux défauts de page pourrait changer son coût ; ce serait un nouvel ADR.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Garder (2a) et (2b) parce que la fiche les liste | La fiche décrit des livrables, pas une obligation de conservation ; la mesure dit que (2a) coûte 6,6 % et 10,2 % et que (2b) ne sort pas du bruit, tout en ajoutant un `unsafe` |
| Garder (2b) seule, pour son gain marginal à N = 5000 | 6,9 ms sur 516,8 ms, pour une dispersion de 1,1 % et une borne de coût de copie à moins de 0,1 ms par millier de candidats : le gain n'est pas dans l'ordre de grandeur de la cause invoquée, et il ne paie pas l'`unsafe` (règle 10) |
| Garder (2a) derrière une option ou une configuration | Une option pour un chemin mesuré plus lent est une abstraction sans usage (règle 12) ; elle laisserait deux tris à tester et deux comportements à documenter, pour aucun gain |
| Reporter le gain de `final` comme celui du code livré | `final` contient (2a) et (2b) ; le gain serait attribué à un code qui n'existe plus. Contraire à `AGENTS.md:39` (mesurer avant d'affirmer) et à ADR-0079, décision 6 |
| Déclarer la tâche `FAIT` sans nouvelle mesure, au motif que les contrôles de `final` sont déjà faits | Les contrôles d'égalité ont porté sur une variante qui contient les changements retirés ; ils ne prouvent pas l'égalité du code livré |
| Rouvrir ADR-0079 pour y inscrire le retrait | Un ADR accepté ne se réécrit pas ; le retrait est un fait postérieur, écrit ici, et ADR-0079 garde sa valeur d'historique |
| Écrire la règle de localité des iid dans `docs/03` malgré la perte mesurée | La condition d'« À surveiller » d'ADR-0079 (`:100`) demandait un écart **en faveur** de l'ordre des iid ; c'est l'inverse qui est mesuré. Écrire la règle contredirait la mesure |
| Ajouter à `GROUPER-LECTURES-VECTEURS` une mesure de l'ordre des iid | Sa fiche n'en contient pas ; ce serait élargir son périmètre par un ADR qui ne la concerne pas, et trancher à la place du propriétaire |
| Mettre en œuvre (a) malgré la condition non remplie | Le profil chiffre son plafond à 1,5 % du coût ; ce serait du code sans gain, contre la décision 4 d'ADR-0079 |

## Application du 2026-10-28

L'ADR est accepté par anticipation. Ce qui a été appliqué : la fiche `REDUIRE-COUT-DES-CLES` reçoit une note datée qui renvoie à cet ADR et dit que le gain à publier est celui de (b) seule ; l'index de `docs/adr/README.md` reçoit sa ligne. ADR-0079, ADR-0080, ADR-0081 et `docs/03-recherche.md` ne sont **pas** modifiés : les trois premiers sont acceptés et leur corps ne se réécrit pas, et le budget de `docs/03:144` reste renvoyé à P8.

| Document | Modification |
|---|---|
| `docs/phases/P4-recherche-rapide.md` | note datée sous le livrable (2a)/(2b) : mesurées, puis retirées sans trace ; (b) seule livrée ; (a) non mise en œuvre ; gain chiffré à publier sur la variante livrée |
| `docs/phases/P4-recherche-rapide.md` | critère de gain : renvoi à cet ADR et à la mesure de confirmation de (b) seule, contrôles rejoués sur la variante livrée |
| `docs/adr/README.md` | ligne d'index de cet ADR en `Accepté` |
| `docs/suivi.md` | ligne dans « Écarts à la conception », tâche `REDUIRE-COUT-DES-CLES` |

Le code et les tests ne sont pas touchés par cet ADR : le retrait de (2a) et (2b) et la mesure de confirmation appartiennent à la tâche.

## Références

- `docs/mesures/MES-03-latence-rescoring.md` — section « Rejeu après REDUIRE-COUT-DES-CLES (ADR-0079) » du 2026-10-28, non commitée : protocole `:799-894`, contrôles `:896-936`, écarts au protocole `:943-981`, résultats `:982-1043`, conclusion `:1045-1094` ; sections « Profil du rescoring » `:249-503` et `:504-767` (réponse à la décision 4 `:383-403`, ordre des candidats `:405-419`, copie `f16` `:421-436`)
- Artefact versionné : `docs/mesures/MES-03-rejeu-reduire-cles.2026-10-28.json` ; bruts sous `data/reduire-cles/raw/`
- `docs/adr/0079-cout-des-cles-au-rescoring.md` — décision 2 (`:71`), décision 3 (`:72`, (b) et égalités à la frontière), décision 4 (`:73`, condition de (a)), décision 6 (`:75`, validation), « Conséquences » : « À surveiller » (`:100`, `:101`), alternatives `:117` (ordonner par iid et s'arrêter là), `:114` (égalités)
- `docs/adr/0082-scan-binaire-multi-segments-curseur-de-segment-et-verification-a-b.md` — décision 3 (`:60`, règle de bruit), précédent d'une acceptation par anticipation
- `docs/adr/0080-acces-aux-vecteurs-fp16-au-rescoring.md` — `:80` (contrôle retiré sans trace s'il n'est pas retenu), `:82` (option 3 écartée)
- `docs/adr/0081-parcours-du-scan-binaire-un-seul-parcours-clairseme.md` — point 7 (`:60`, dépendance croisée « à écrire et non à décider ici »)
- `docs/adr/0075-regroupement-des-resultats-ordre-et-forme-de-la-reponse.md` — candidats consommés par le regroupement, qui déterminent le nombre de clés de (b)
- `docs/adr/0072-un-seul-calcul-du-produit-scalaire-pour-exact-et-le-rescoring.md`, décision 3 — égalité au bit près avec EXACT ; INV-06
- `docs/phases/P4-recherche-rapide.md:157-176` — fiche `REDUIRE-COUT-DES-CLES`, livrables `:162-167`, critères `:168-174` ; `:185` — clause de retrait sans trace de la fiche voisine
- `docs/03-recherche.md:135`, `:144` — budget de latence indicatif, non tranché ici
- `AGENTS.md:39` (mesurer avant d'affirmer), règles 10 (robustesse, `unsafe` justifié) et 12 (simplicité)
- Code mesuré : `crates/crible-search/src/dense/rescore.rs` (boucle et tri), `crates/crible-store/src/records.rs` (`read_passage_key`, `passage_key_by_iid`), `crates/crible-store/src/segment.rs` (`vector_f16`, emprunt au mmap de (2b))
