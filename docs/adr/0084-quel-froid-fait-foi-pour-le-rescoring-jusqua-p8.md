# ADR-0084 — Quel froid fait foi pour le rescoring jusqu'à P8

- **Statut** : Proposé
- **Date** : 2026-10-28
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, sur demande du propriétaire du 2026-10-28, tâche `NETTOYER-P4` (phase P4, hors plan), à partir du verdict de `@architecte` (ECART et NON-COUVERT) et du rejeu de MES-03 du 2026-10-28
- **Remplace** : — (précise, s'il est accepté, la décision 6 d'ADR-0080 et la ligne MES-03 de `docs/08`)

## Contexte

### Ce que la conception et les ADR exigent

ADR-0080, décision 6 (`:84`), demande cinq contrôles pour valider toute mise en œuvre de la lecture groupée des vecteurs fp16, dont « une passe par série froide, série froide `F_NOCACHE` avec `mincore` », la série après 10 minutes d'inactivité, et une série nouvelle, « searcher déjà ouvert, cache déplacé après l'ouverture » (`:88`). Cette dernière « s'ajoute sans réécrire ADR-0077 » : elle ne change ni N, ni le nombre de requêtes, ni les percentiles. La fiche `GROUPER-LECTURES-VECTEURS` reprend les trois séries (`docs/phases/P4-recherche-rapide.md:188`), et `docs/08-invariants-et-tests.md:68` fixe le protocole de MES-03 : « froid par copies non cachées (`F_NOCACHE`, `mincore`) ».

ADR-0077, décision 2 (`:57`), fait de `F_NOCACHE` la série **principale** du froid pour le rescoring seul, garde la série de 10 minutes comme série **secondaire**, dite « borne basse », et renvoie le froid contrôlé à P8, en conteneur.

### Ce que la mesure du 2026-10-28 a produit

Le rejeu de MES-03 après `REDUIRE-COUT-DES-CLES` déclare deux faits.

- **La série `F_NOCACHE` n'a pas été rejouée sur le moteur.** `SegmentReader::open` vérifie le sceau blake3 du fichier de vecteurs entier, ce qui réchauffe le fichier avant la première requête ; un `F_NOCACHE` posé après l'ouverture ne rendrait donc pas les pages froides (`crates/crible-store/src/segment.rs:444-446` ; `docs/mesures/MES-03-latence-rescoring.md:1367-1377`, écart 2). La série `F_NOCACHE` de MES-03 n'existe que sur un harnais `--skip-engine` qui ouvre les fichiers à la main, et la copie « vecteurs seuls » n'a pas été rejouée (`MES-03:1377`).
- **La série « cache déplacé après l'ouverture » a été mesurée, avec un froid partiel chiffré.** 19,5 Go sont lus juste après l'ouverture, puis une seule passe, avec le taux de résidence relevé par `mincore` à chaque requête. La résidence moyenne des pages de candidats va de **0,48 à 0,84** selon la série et le N (`MES-03:1375-1377`), et de 0,48 à 0,80 dans le tableau publié (`:1489-1496`) ; la première requête réside de 0,47 à 0,58 des pages à N = 5000 (`:1506-1507`).

Deux conséquences se lisent directement dans le rapport. La p50 froide de `base` sur cette série (58 à 116 ms) est **plus basse** que les 85,0 et 177,5 ms d'ADR-0080, qui supposaient le fichier entièrement absent du cache (`MES-03:1509-1511`). Les gains mesurés sont donc **relatifs** : −64,1 %, −64,7 % et −72,0 % en p50 à N = 1000, 2000 et 5000, avec les amplitudes et les verdicts de la règle d'ADR-0082 (`MES-03:1500-1504`).

Les 85,0 et 177,5 ms de `docs/03-recherche.md:145-146` restent la seule mesure à froid complet. Ces lignes les attribuent au « produit scalaire seul (MES-03) » : c'est une copie lue avec `F_NOCACHE` par un harnais, jamais le moteur.

### Ce que les textes ne couvrent pas

- **Quel chiffre juge le moteur quand le froid n'est que partiel.** Aucun document ne dit si un gain mesuré sur 60 à 80 % de pages résidentes vaut comme gain « à froid ».
- **Comment comparer à la borne de 15 à 25 ms à N = 1000 et de 60 à 90 ms à N = 5000** (`ADR-0080:79`, reprise à `:109` et dans la fiche `:187`). Cette borne est calculée contre 85,0 et 177,5 ms, donc contre un fichier entièrement absent du cache, ce que P4 ne sait pas produire.
- **Si les chiffres de `docs/03:145-146` restent la référence « disque ».** Ils sont la seule mesure à froid complet, mais d'un harnais, pas du moteur.

`@architecte` a rendu un verdict **ECART** (la série exigée n'a pas été rejouée) et **NON-COUVERT** (rien ne dit quelle série fait foi). Les deux questions se tranchent par une décision, pas par la lecture des textes.

### Les contraintes qui pèsent

- **Le sceau blake3 à l'ouverture n'est pas facultatif dans le code.** `segment.rs:444-446` le présente comme exigé par la tâche de segment (« a corrupted checksum is detected at open time »). ADR-0006 fixe les segments sans mentionner cette vérification, et aucune clause de conception ne la déclare optionnelle. Un chemin qui l'omettrait serait un second mode d'ouverture : il demanderait une décision propre, et un confinement explicite.
- **Une purge du cache de pages demande `root` sur macOS** (ADR-0077:27, MES-13).
- **`F_NOCACHE` n'existe que sur macOS.** Le froid Linux (`posix_fadvise(DONTNEED)`, cgroup) n'est défini nulle part (ADR-0080:89).
- **La cible de latence est le p95 de pgvector, en conteneur** (`docs/08:87`). Aucune cible absolue de froid n'existe aujourd'hui pour le rescoring.
- **La campagne de P4 ne vaut pas validation du budget** (ADR-0077:80) : elle est native, sur un Mac, et non en conteneur.

## Décision

**Pour P4, la série « searcher déjà ouvert, cache déplacé après l'ouverture » est celle qui fait foi pour juger les gains du rescoring ; la mesure absolue du froid est renvoyée à P8, en conteneur. Aucune affirmation de budget de latence à froid n'est permise avant P8.**

Précisions :

1. **Série qui fait foi.** Le gain de `GROUPER-LECTURES-VECTEURS` se juge sur la même série, en passes entrelacées entre `base` et la variante, avec la règle de bruit d'ADR-0082 (décision 3). Chaque point publié porte la **résidence des pages de candidats** : moyenne par requête et valeur de la première requête, par N et par variante. Un chiffre froid sans sa résidence n'est pas publiable.
2. **`F_NOCACHE` n'est pas rejouable sur le moteur en P4.** La série de la décision 6 d'ADR-0080 reste au protocole, mais elle n'est **pas un critère d'acceptation** de `GROUPER-LECTURES-VECTEURS` tant qu'un chemin d'ouverture sans sceau n'a pas été décidé. Son absence est **déclarée** dans le rapport, à l'endroit où MES-03 la nomme (écart 2) ; elle n'est pas remplacée en silence.
3. **La série de 10 minutes reste secondaire.** Elle garde son rôle de « borne basse » d'ADR-0077 ; elle n'est ni principale, ni un critère d'acceptation.
4. **`docs/03:145-146` : chiffres de harnais, jamais du moteur.** 85,0 et 177,5 ms proviennent d'une copie lue avec `F_NOCACHE` par un harnais qui contourne le moteur. Ils restent la seule référence à froid complet et se lisent comme tels ; aucun rapport de tâche, aucun ADR ni aucun document de conception ne les présente comme la latence du moteur.
5. **La borne absolue d'ADR-0080 n'est ni atteinte ni infirmée.** Les 15 à 25 ms et les 60 à 90 ms sont calculées sans réchauffement ; P4 ne peut ni les confirmer ni les réfuter, et la comparaison est réservée à P8.
6. **Le froid vrai est renvoyé à P8** (ADR-0077:57, ADR-0080:89). P8 doit définir le protocole de froid sous Linux **avant** de conclure sur le rescoring, et étendre la fiche `MESURER-EN-CONTENEUR-MAC` si le rescoring doit y être mesuré.
7. **Interdiction explicite.** Tant que P8 n'a pas produit une série froide contrôlée du moteur, aucun document, rapport, fiche, ADR ni compte rendu n'affirme qu'un budget de latence à froid est tenu ou manqué. Un chiffre de P4 s'appelle « froid partiel, résidence r » et jamais « à froid ».
8. **Ce qui n'est pas décidé ici** : le protocole de froid sous Linux, les cibles en conteneur, un chemin d'ouverture sans vérification du sceau, la valeur de N (ADR-0078), le format des segments, et l'adoption de `MADV_RANDOM` ou du `pread` groupé, qui restent celles d'ADR-0080.

## Conséquences

- **Positives** :
  - P4 juge ce qu'il peut mesurer : un gain relatif, sur des passes entrelacées, avec un froid partiel déclaré au lieu d'être tu ;
  - la résidence publiée par point rend le degré de froid vérifiable, et interdit de lire un chiffre partiel comme un chiffre à froid ;
  - aucune série n'est promise à la place d'une autre : `F_NOCACHE` reste au protocole, avec sa raison de non-rejeu écrite ;
  - le moteur ne change pas, et le sceau blake3 à l'ouverture reste intact ;
  - P8 hérite d'une question nette — le protocole de froid sous Linux — au lieu d'un chiffre ambigu.

- **Négatives / coûts** :
  - P4 ne produit **aucune borne absolue** : la borne de 15 à 25 ms et celle de 60 à 90 ms restent sans réponse jusqu'à P8 ;
  - un gain relatif mesuré sur 60 à 80 % de pages résidentes **ne se transpose pas** au disque froid : il surestime le gain attendu, et le rapport doit le dire ;
  - les campagnes ne se comparent entre elles que si la résidence est comparable ; deux séries de résidence différente ne se soustraient pas ;
  - les chiffres de P4 (58 à 116 ms pour `base` à froid partiel) peuvent être lus comme la latence froide du moteur ; seule la mention de la résidence l'empêche ;
  - P8 porte une décision de plus, et devra définir un protocole Linux qui n'existe nulle part.

- **À surveiller** :
  - si la résidence moyenne d'un point dépasse largement 0,8, le point n'est plus un froid partiel utile et doit être rejoué ou écarté ;
  - si P8 mesure un froid en conteneur très au-dessous des gains relatifs de P4, le protocole de P4 aura été trop optimiste et la série « cache déplacé » devra être révisée ;
  - si un changement du moteur rend la vérification du sceau paresseuse, `F_NOCACHE` redevient applicable et la question est rouverte ;
  - si un compte rendu ou un rapport affirme un budget à froid sans série P8, la règle 7 est violée et le document est corrigé.

**Application, seulement après acceptation.** L'acceptation demanderait, dans un commit qui cite ADR-0084 :

- une note datée dans ADR-0080, décision 6, et dans son « À surveiller » (`:121`), indiquant que la série `F_NOCACHE` n'est pas rejouable sur le moteur en P4 et que la série « cache déplacé après l'ouverture » fait foi jusqu'à P8 ;
- l'index des ADR, avec la mention « précisé par ADR-0084 » sur la ligne d'ADR-0080 ;
- la ligne MES-03 de `docs/08` (`:68`), pour y ajouter que la série `F_NOCACHE` est déclarée non rejouable en P4 et que le froid vrai est renvoyé à P8 ;
- le livrable de rejeu de la fiche `GROUPER-LECTURES-VECTEURS` (`docs/phases/P4-recherche-rapide.md:188`), pour y distinguer la série principale de P4 et la série renvoyée à P8.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| B — obtenir un froid vrai par une ouverture sans vérification blake3 réservée au harnais, ou par une purge après l'ouverture | La purge du cache de pages demande `root` sur macOS (ADR-0077:27, MES-13). Une ouverture sans sceau retire le seul contrôle d'intégrité du fichier de vecteurs à l'ouverture, que `segment.rs:444-446` présente comme exigé et qu'aucune clause ne déclare facultatif ; elle créerait un second mode d'ouverture, à confiner à `examples/` par une décision propre à ce sujet. Elle ne mesurerait de toute façon pas l'ouverture du moteur |
| Maintenir `F_NOCACHE` comme critère d'acceptation de P4 et bloquer la tâche | Le contrôle est inatteignable sans changer le moteur ou la conception ; une tâche ne se valide pas contre un contrôle impossible, et le blocage ne produit aucun chiffre |
| Déclarer la campagne de P4 nulle sur le froid et attendre P8 | Les mesures du 2026-10-28 répondent à la question que P4 pose — le gain de la lecture groupée — et les jeter ne rendrait pas le froid vrai plus mesurable |
| Prendre la série de 10 minutes comme série principale | Sa résidence de première requête (0,54 à 0,72) n'est pas plus froide que celle du cache déplacé, et elle coûte 600 s par point ; ADR-0077 en fait une borne basse, pas une référence |
| Comparer directement les chiffres de P4 à la borne de 15 à 25 ms et de 60 à 90 ms | La borne suppose un fichier entièrement absent du cache, la mesure porte sur 60 à 80 % de pages résidentes : la comparaison serait fausse dans les deux sens |
| Publier les gains relatifs sans la résidence | Un gain sur un froid partiel non chiffré ne se distingue pas d'un gain à chaud, et devient ininterprétable pour P8 comme pour une relecture |
| Renoncer à toute mesure froide en P4 et ne garder que le chaud | La fiche exige un gain chiffré à froid à N = 1000 et à N = 5000 (`:195`) ; le chaud ne dit rien du coût des défauts de page, qui est le sujet d'ADR-0080 |

## Références

- ADR liés : ADR-0006 (segments), ADR-0077 (protocole de latence, décision 2 et `:57`), ADR-0078 (N = 5000), ADR-0080 (accès aux vecteurs fp16, décision 6, `:63`, `:79`, `:84`, `:88`, `:89`, `:109`, `:121`), ADR-0082 (règle de bruit, décision 3), ADR-0083 (rejeu de MES-03 du 2026-10-28).
- Mesures : `docs/mesures/MES-03-latence-rescoring.md` — écarts au protocole `:1355-1394` (écart 2, `:1367-1377`) ; série froide `base` contre `grouped` `:1485-1511` ; série de 10 minutes `:1513-1527` ; `docs/mesures/MES-13-temps-de-demarrage.md:154-158` (cache non purgé sans `root`).
- Conception : `docs/03-recherche.md:135-146` (budget de latence, lignes du rescoring) ; `docs/08-invariants-et-tests.md:68` (MES-03), `:87` (cible de latence en conteneur).
- Fiches : `docs/phases/P4-recherche-rapide.md:179-198` (`GROUPER-LECTURES-VECTEURS`, livrable de rejeu `:188`, critère de gain `:195`).
- Code : `crates/crible-store/src/segment.rs:444-446` (coût et rôle de la vérification du sceau à l'ouverture).
