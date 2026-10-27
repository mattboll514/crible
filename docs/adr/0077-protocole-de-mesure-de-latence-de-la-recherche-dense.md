# ADR-0077 — Protocole de mesure de latence de la recherche dense : N, froid, pools, requêtes et RSS

- **Statut** : Accepté (accepté par anticipation le 2026-10-27, confirmé par le propriétaire le 2026-10-27)
- **Date** : 2026-10-27 (proposition et acceptation anticipée)
- **Décideurs** : propriétaire
- **Décision** : accepté par anticipation le 2026-10-27, par délégation du propriétaire pour la série REGROUPER-RESULTATS, MESURER-FIDELITE-BINAIRE, MESURER-LATENCE-DENSE, puis confirmé par le propriétaire le 2026-10-27. Les cinq décisions sont retenues telles que proposées, sans erratum : N ∈ {1000, 2000, 5000} avec rescoring et scan mesurés, plus une colonne EXACT au point de bascule d'ADR-0074 ; le froid mesuré par copies non cachées (`F_NOCACHE`, `mincore`) pour le rescoring seul, avec une série secondaire après 10 min d'inactivité déclarée comme borne basse ; des pools construits par un harnais aux tailles 1 k, 10 k, 100 k, 1 M et tous les vivants (7,74 M), contigus et dispersés, avec contre-contrôle par la CLI ; au moins 300 vecteurs de requête fp32 distincts, 3 passes après échauffement et p50/p95/p99 par rang le plus proche ; le RSS en trois états, scindé en mémoire anonyme et adossée à des fichiers.
- **Proposé par** : agent greffier, sur demande de l'orchestrateur, pour la tâche `MESURER-LATENCE-DENSE` (phase P4), à partir des verdicts de `@architecte` rendus avant de confier la mesure à `@mesureur` ; les options sont retenues par l'orchestrateur par délégation
- **Remplace** : —

## Contexte

### Ce que la fiche demande

La fiche `MESURER-LATENCE-DENSE` (`docs/phases/P4-recherche-rapide.md:130-137`) demande MES-02, MES-03 et MES-09. Ses livrables sont « sur `synth-large` (8,6 M passages) : la latence p50/p95 du scan selon la taille du pool (1 k, 10 k, 100 k, 1 M, 8,6 M) et le rescoring à froid et à chaud » (`:134`) et « la RAM réelle (RSS), en natif sur le Mac » (`:135`). Son critère est : « les protocoles sont reproductibles et les chiffres sont comparés au budget de `03-recherche.md` » (`:136`).

La fiche ne fixe ni la valeur de N, ni ce qu'est un état froid, ni comment obtenir les tailles de pool sur un jeu qui n'en fournit aucune, ni le nombre de requêtes, ni la définition du « régime » pour le RSS. Ces cinq points sont des décisions de protocole : sans eux, deux mesures du même nom ne seraient pas comparables.

### Ce que la conception fixe

- **Le budget ne cite qu'un N.** `docs/03-recherche.md:144` porte la ligne « Rescoring de N = 1000 : < 1 ms à chaud, ~10 ms à froid ». Le tableau est indicatif et « à remplacer par les mesures » (`:136`). Aucune autre valeur de N n'apparaît.
- **MES-01 recommande N = 5000 et renvoie le coût à MES-02 et MES-03.** `docs/mesures/MES-01-fidelite-scan-binaire.md:228-231` propose `search.rescore_candidates = 5000` et écrit : « 5 000 candidats rescored par requête contre 1 000 est un facteur 5 sur l'étape de rescoring, que MES-02 et MES-03 doivent chiffrer avant que le défaut soit figé ».
- **Le seuil exact vaut N par défaut, et le seuil est inclusif.** ADR-0074 (décisions 2 et 5) fixe `exact_threshold` optionnelle, matérialisée à la valeur de `rescore_candidates` à la création, et un pool de taille **égale** au seuil traité en EXACT. Sur un jeu dont les pools valent au moins N, tout point à N passages passerait donc en EXACT par construction, et le scan ne serait pas mesuré.
- **`synth-large` est réglé sans scan.** L'`index.json` de `synth-large` porte `"scan": "none"` et `"exact_threshold": 1000`. Mesurer le scan demande donc de changer ces deux réglages, sur une copie de mesure et non sur le jeu lui-même.
- **Les codes binaires vivent en RAM.** ADR-0069 (décision 3) et `docs/02-architecture.md:208` placent les codes en mémoire, chargés à l'ouverture, précisément « pour ne pas être évincés ». Le scan n'a donc **aucun état froid** : ses données ne sont jamais relues du disque. Seuls les **vecteurs de référence fp16** sont sur disque, en mmap (`docs/02:213`), et le rescoring est la seule étape qui puisse être froide.
- **La mémoire attendue est tabulée.** `docs/02:202-219` donne la règle (« ce qui est parcouru en entier à chaque requête est en RAM ; ce qui est lu seulement pour quelques candidats reste sur disque »), la part de chaque pièce pour 8,58 M × 768 passages, et le minimum de 2,0 Go d'une instance qui sert des requêtes. Les vecteurs fp16 (13,2 Go) sont en mmap : leurs pages comptent dans le RSS sans être de la mémoire possédée par l'instance.
- **La session de l'encodeur est par instance, pas par index.** `docs/02:217-219` : le modèle fp32 est chargé au premier usage et partagé par tous les index de même empreinte. MES-09 est libellée « RAM réelle (RSS) en régime, par index » (`docs/08:74`) : la part par index et la part par instance doivent être séparées.
- **Le précédent de mesure du froid est une approximation assumée.** MES-00 mesure le froid après 10 min d'inactivité et le déclare « borne basse du froid réel ». MES-13 (`docs/mesures/MES-13-temps-de-demarrage.md:154-158`, `:260-264`) constate que macOS ne purge pas son cache de pages sans `root`, qu'un clone APFS partage ses blocs, et que le mode `cold` « n'est pas un froid contrôlé ».
- **ADR-0071 attend des chiffres de MES-02.** Sa section « À surveiller » demande des latences par densité de pool et une comparaison des parcours dense et clairsemé du scan binaire.
- **Le budget se compare en conteneur.** `docs/08:87` fixe la cible de latence « sur le Mac mini en conteneur », et MES-02 est prévue « P4, puis P8 en conteneur » (`:67`).

### Les questions tranchées ici

`@architecte` a rendu ces verdicts avant la mesure :

1. **Valeurs de N** : **NON-COUVERT** pour le jeu de valeurs. Le budget ne cite que N = 1000 (`docs/03:144`), MES-01 recommande 5000 et demande le coût avant de figer le défaut. Le seuil par défaut égal à N rendrait trivial le point « pool 1 k ».
2. **Froid** : **NON-COUVERT**. MES-00 mesure une borne basse, MES-13 constate qu'on ne purge pas le cache de macOS sans `root`. Le scan n'a pas d'état froid (codes en RAM) ; seul le rescoring en a un.
3. **Pools** : **NON-COUVERT**, et la fiche est littéralement inapplicable sur `synth-large`. Son schéma déclare plusieurs champs, mais seul `categorie` est rempli (16 valeurs, ≈ 537 500 passages par valeur), avec `cluster_by: ["categorie"]` : les valeurs sont contiguës après compaction, et rien ne donne 1 k, 10 k, 100 k, ni un pool dispersé. MES-14 (`docs/mesures/MES-14-cout-filtres-intervalles.md:292-296`) a déjà constaté que ce jeu ne porte que `categorie`. Par ailleurs le jeu porte 860 000 passages morts et **7 740 000 vivants** (MES-07 `:34-39`), donc « 8,6 M » doit se lire 7,74 M.
4. **Requêtes** : les vecteurs fournis sont **CONFORMES** (`synthetic-external`, `crible search --vector-file`, l'encodage est MES-05) ; le nombre de requêtes et de répétitions est **NON-COUVERT**.
5. **RSS** : « en régime » est **NON-COUVERT** ; la comparaison au tableau de `docs/02:202-219` est **CONFORME** ; la session ONNX est par instance ; les pages mmap des vecteurs gonflent le RSS sans être possédées.

Aucune de ces questions ne se tranche par la lecture seule : elles fixent un plan d'expérience, un mode de forçage de stratégie et des définitions. Ce sont des décisions.

### L'état des données au moment de la décision

- `data/synth-large/indexes/synth-large` porte **8 600 000** passages, dont **860 000** supprimés, donc **7 740 000** vivants, un passage par document, 768 dimensions, vecteurs fp16, modèle `external` (MES-07 `:34-39`).
- L'index pèse **16 Go** : 12,9 Go de vecteurs et 3,76 Go de `records.sqlite`. Une copie de mesure doit donc disposer de cet espace, et une seule copie existe à la fois.
- Le seul champ rempli est `categorie`, 16 valeurs, ≈ 537 500 passages par valeur, contiguës après compaction (`cluster_by: ["categorie"]`). Deux pools réels sont donc disponibles par la CLI : une valeur de `categorie` (≈ 537 k) et tous les vivants (7,74 M).
- Les réglages utiles se changent par `PATCH` : `search.exact_threshold` (ADR-0074) et `search.scan`.

## Décision

**La mesure de latence dense a un protocole unique : N ∈ {1000, 2000, 5000} mesurés sur des copies de mesure, un froid obtenu par copies non cachées et vérifié par `mincore` pour le rescoring seul, des pools construits à cinq tailles et deux densités, un jeu de 300 requêtes fp32 répété trois fois, et un RSS en trois états scindé en mémoire anonyme et adossée à des fichiers.**

Précisions :

1. **N et stratégies.** Le rescoring et le scan sont mesurés pour **N ∈ {1000, 2000, 5000}**. N = 1000 est la ligne du budget (`docs/03:144`) ; 2000 et 5000 donnent le coût marginal, et la linéarité du rescoring est vérifiée, puisque MES-01 annonce un facteur 5 entre 1000 et 5000. BINARY est forcé par `PATCH search.exact_threshold = 0`, avec `search.scan` réglé pour autoriser le scan, sur des **copies de mesure** dans `data/`, jamais sur `data/synth-large/` lui-même. Un chiffre n'est retenu que si le plan rend `dense_strategy = "binary"`. Une **colonne EXACT** complète la campagne, sur un pool de taille N pour chaque N : c'est le point de bascule d'ADR-0074 (décision 2), et il permet de lire le prix du choix.
2. **Froid, pour le rescoring seul.** Le rapport écrit que le scan n'a pas d'état froid, ses codes étant en RAM (ADR-0069, décision 3). La série **principale** est le « froid par copie non cachée » : le harnais copie le fichier de vecteurs de la copie de mesure en lecture et en écriture avec `F_NOCACHE` (`fcntl`, macOS), de sorte que la copie n'entre pas dans le cache de pages, puis vérifie la résidence des pages (`mincore`) avant la mesure. Une copie sert plusieurs requêtes froides tant que les pages qu'elles lisent n'ont pas déjà été lues ; le rapport donne la proportion de pages déjà résidentes par requête. La place disque est vérifiée avant chaque copie, une seule copie existe à la fois et elle est supprimée après usage. La série **secondaire** est le « froid après 10 min d'inactivité » (convention MES-00), déclarée comme borne basse. Aucune des deux n'est appelée « purge » ; le froid contrôlé reste pour P8, en conteneur. Aucun changement du moteur.
3. **Pools.** La campagne **principale** est menée par un harnais qui construit les pools aux tailles **1 k, 10 k, 100 k, 1 M et tous les vivants (7,74 M)**. Le chiffre réel est déclaré à la place du « 8,6 M » de la fiche. Chaque taille est produite sous **deux formes** : **contiguë** et **dispersée**, par tirage déterministe à graine fixe. Le **parcours retenu** (dense ou clairsemé, ADR-0071) est rapporté pour chaque point, et les deux parcours sont comparés sur un même pool quand c'est possible. Un **contre-contrôle par la CLI** porte sur des pools réels — une valeur de `categorie` (≈ 537 k) et tous les vivants — pour prouver que le harnais ne mesure pas un monde à part. Si construire un pool arbitraire exige de rendre public du code du moteur, le harnais réplique ce code (précédent MES-13) ou la question remonte ; l'API publique ne change pas.
4. **Requêtes.** Au moins **300** vecteurs de requête **fp32 distincts**, tirés du générateur de `synth-large` à graine fixe et tracés par SHA-256. Au moins **3 passes** par point, après échauffement. Le rapport donne **p50, p95 et p99** par rang le plus proche, avec `n` écrit sur chaque ligne. Pour le froid, les valeurs par requête froide sont données en **médiane et maximum**, sans p95 sur moins de 20 points.
5. **RSS.** Trois états sont mesurés : **(a)** index ouvert et instantané chargé, aucune requête ; **(b)** après une passe de requêtes à vecteur fourni, sans session ONNX. Ces deux états forment le « RSS de l'index ». **(c)** avec la session fp32 chargée, rapporté **à part**, puisqu'elle est par instance (`docs/02:217-219`) ; il n'est mesuré que si un index à modèle ONNX est disponible localement, sinon il est renvoyé à P8. Le RSS est scindé en **mémoire anonyme** et **mémoire adossée à des fichiers** (les pages mmap des vecteurs fp16), puis comparé ligne à ligne au tableau de `docs/02:202-219`, en rappelant que le point mesuré a **7,74 M** passages vivants et non 8,58 M.

Ce qui n'est **pas** décidé ici : la valeur de N par défaut, qui fera l'objet d'un ADR séparé après la mesure (Q-01) ; le remplacement de la règle de parcours d'ADR-0071, qui demandera un nouvel ADR si les chiffres le justifient ; le froid contrôlé et la comparaison en conteneur, renvoyés à P8 ; la fidélité du scan, qui appartient à MES-01 et à ADR-0076 ; les cibles de latence, qui restent celles de `docs/suivi.md`.

## Conséquences

- **Positives** :
  - N = 1000, 2000 et 5000 répondent à la demande de MES-01, et la vérification de linéarité distingue un coût proportionnel d'un coût qui dérape ;
  - la colonne EXACT au point de bascule chiffre le compromis d'ADR-0074 au lieu de le supposer ;
  - `F_NOCACHE` avec vérification par `mincore` donne un froid reproductible sans `root`, là où MES-13 ne pouvait qu'approximer ;
  - les pools construits par le harnais couvrent les tailles que la fiche demande, ce que `synth-large` ne permet pas ;
  - les deux densités séparent l'effet de la taille de celui de la dispersion, ce qu'ADR-0071 attend ;
  - le contre-contrôle par la CLI rattache le harnais au moteur réel ;
  - le RSS scindé évite de compter 13,2 Go de pages mmap comme de la mémoire possédée, et le partage par instance est rapporté à part.
- **Négatives / coûts** :
  - la campagne a cinq tailles, deux densités, trois valeurs de N et une colonne EXACT : le rapport est long ;
  - les copies de mesure demandent 16 Go d'espace libre, et une seule à la fois ;
  - le froid par `F_NOCACHE` ne vaut que pour macOS ; il devra être refait en conteneur en P8, et le rapport doit le dire ;
  - le harnais de pools reproduit du code du moteur (précédent MES-13) : il doit être versionné avec le rapport pour rester vérifiable ;
  - les chiffres portent sur 7,74 M passages vivants, et non sur les 8,58 M de Legifacile : la comparaison au budget garde cet écart ;
  - la mesure native sur le Mac n'est pas la cible de `docs/08:87`, qui est le conteneur : les chiffres de P4 ne valent pas validation du budget.
- **À surveiller** :
  - si les codes binaires ne sont pas tous résidents au premier scan, un froid apparaîtrait sur le scan et l'affirmation « pas d'état froid » devra être revue ;
  - si le parcours clairsemé dépasse le dense sur un pool contigu de 1 M, la règle d'ADR-0071 sera à réexaminer par un nouvel ADR ;
  - si la proportion de pages déjà résidentes dépasse largement zéro sur la série `F_NOCACHE`, la copie n'est pas réellement froide et la série secondaire devient la seule référence ;
  - si la mémoire adossée à des fichiers approche le RSS total, la lecture du tableau de `docs/02:202-219` devra être corrigée par un ADR ;
  - si le rescoring à N = 5000 dépasse le budget de bout en bout, le N par défaut ne pourra pas être 5000 sans un autre levier, et le choix se fera dans l'ADR de Q-01.
  - **Note du 2026-10-27 (ADR-0081, accepté)** : ADR-0081 retire le seuil de densité d'ADR-0071 et retient un seul parcours du scan binaire, le clairsemé, sous condition d'une **mesure multi-segments**. Cette mesure se fait par passes alternées sur une copie compactée (environ 8 segments de 1 M passages) et sur une copie multi-attaches, même pool, même plage, N = 5000, 300 requêtes, 3 paires. Si le clairsemé perd plus de quelques pour cent sur 8 segments, le remède est un curseur de segment, pas le retour du parcours dense. Le protocole fixé ici — N, nombre de requêtes, percentiles, séries froides — n'est pas modifié par cette note.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Mesurer N ∈ {200, 500, 1000, 2000, 5000}, comme la fidélité | La latence n'a pas besoin des petits N : le budget ne cite que 1000, et MES-01 demande le coût en 2000 et 5000. Trois points suffisent à lire la linéarité |
| Figer N = 5000 dans cet ADR | MES-01 demande que le coût soit chiffré avant de figer le défaut ; le choix de N est une décision distincte, prise après la mesure |
| Mesurer le scan directement sur `data/synth-large/` | Le jeu est la référence partagée et ses réglages (`scan: "none"`, `exact_threshold: 1000`) ne mesurent pas le scan ; une copie dans `data/` rend la manipulation réversible |
| Garder `exact_threshold` par défaut (= N) | Un pool de taille N passerait en EXACT par construction : le point « 1 k » ne mesurerait pas le scan |
| Appeler « purge » le froid par copie | macOS ne purge pas son cache sans `root` (MES-13) ; le mot promettrait plus que la mesure ne fait |
| Attendre P8 et le conteneur pour tout mesurer | La fiche est en P4, et les chiffres natifs suffisent à comparer les tailles de pool et les N ; le conteneur reste la validation |
| Mesurer le froid sur le scan | Les codes sont en RAM (ADR-0069, décision 3) : il n'y a pas d'état froid à mesurer, et l'annoncer serait faux |
| Un seul pool « tous les vivants » et une extrapolation | Une extrapolation n'est pas une mesure, et ADR-0071 demande explicitement des chiffres par densité |
| Pools uniquement par la CLI, sur `categorie` | Les valeurs de `categorie` donnent ≈ 537 k passages contigus : ni 1 k, ni 10 k, ni 100 k, ni un pool dispersé |
| Pools uniquement par le harnais | Le harnais pourrait mesurer un monde à part ; le contre-contrôle par la CLI sur deux pools réels le rattache au moteur |
| Rendre public du code du moteur pour construire les pools | Change l'API publique pour une mesure ; le harnais réplique le code (précédent MES-13), ou la question remonte |
| Un seul parcours du scan, choisi par avance | ADR-0071 attend la comparaison des deux parcours ; retenir l'un sans mesurer l'autre priverait l'arbitrage de sa donnée |
| 100 requêtes et une passe | Trois passes et 300 requêtes bornent la variance, et une seule passe ne distingue pas un échauffement d'une mesure |
| p95 sur les valeurs froides | Moins de 20 requêtes froides par point : un p95 y serait un maximum déguisé. Médiane et maximum disent la même chose sans faux percentile |
| RSS total seul | Il mélange la mémoire possédée et les pages mmap des vecteurs fp16 (13,2 Go) ; scindé, il se compare au tableau de `docs/02:202-219` |
| RSS avec la session ONNX dans le même chiffre | La session est par instance, pas par index (`docs/02:217-219`) ; la mêler au chiffre par index rendrait MES-09 ininterprétable |

## Références

- Fiche : `docs/phases/P4-recherche-rapide.md:130-137` (`MESURER-LATENCE-DENSE`).
- Conception : `docs/03-recherche.md:134-148` (budget de latence, ligne N = 1000) ; `docs/02-architecture.md:202-219` (mémoire : ce qui est en RAM, sessions par instance) ; `docs/08-invariants-et-tests.md:52-57` (`synth-large`), `:67-68`, `:74` (MES-02, MES-03, MES-09), `:86-87` (cibles de rappel et de latence) ; `docs/00-glossaire.md` (pool, rescoring, N, seuil exact, codes binaires, compaction).
- Mesures : `docs/mesures/MES-01-fidelite-scan-binaire.md:228-231` (coût de N = 5000 renvoyé à MES-02 et MES-03) ; `docs/mesures/MES-07-duree-compaction.md:34-39` (7 740 000 passages vivants) ; `docs/mesures/MES-13-temps-de-demarrage.md:154-158`, `:260-264` (cache de pages non purgé, copie approximative) ; `docs/mesures/MES-14-cout-filtres-intervalles.md:292-296` (`synth-large` ne porte que `categorie`) ; `docs/mesures/README.md` (modèle de rapport).
- Données : `data/synth-large/indexes/synth-large` (8,6 M passages, 7,74 M vivants, 16 Go, `categorie`).
- ADR liés : ADR-0004 (codes binaires et scan du pool), ADR-0069 décision 3 (codes en RAM), ADR-0071 (seuil entre parcours dense et clairsemé, attend MES-02), ADR-0074 (seuil exact et planification de la stratégie), ADR-0076 (protocole de fidélité du scan binaire, forme reprise ici).
- Question ouverte liée : Q-01 (N par défaut et cible de fidélité du scan binaire, MES-01).
