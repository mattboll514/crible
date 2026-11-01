# ADR-0125 — Commande `crible eval` : mode local, format d'entrée, métriques, surcharges et rapport

- **Statut** : Accepté (accepté par anticipation par le propriétaire le 2026-11-01, délégation du propriétaire pour la suite de P7 ; validation officielle à la fin de la série)
- **Date** : 2026-11-01
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, tâche COMMANDE-EVAL (phase P7), sur l'analyse de `@architecte` et les décisions retenues par l'orchestrateur, en application de la délégation du propriétaire du 2026-11-01 pour la série EXPLIQUER-DOCUMENTS, ROUTE-STATS, COMMANDE-EVAL, REUTILISER-LECTEURS-DENSES
- **Remplace** : — (ne remplace aucune décision ; précise ADR-0076, ADR-0092, ADR-0116, ADR-0117, ADR-0119, ADR-0123 et ADR-0124 sans modifier le corps d'un ADR accepté)

## Contexte

### La fiche et ce qu'elle demande

La fiche `COMMANDE-EVAL` (`docs/phases/P7-observabilite.md:113-124`) fait de `crible eval` « l'instrument de validation du propriétaire ». Ses livrables sont la commande — « contre une instance via l'API, ou en local sur un dossier de données » (`:118`) —, les mesures de fidélité contre `exact`, de qualité contre `expected`, les latences et la comparaison `--baseline` (`:119`), un rapport JSON et un rapport Markdown dans `docs/mesures/` (`:120`), et `docs/jalons/J7-demo.md` (`:121`). Ses critères sont : « le rapport sur l'échantillon est lisible sans contexte » (`:123`) et « une régression injectée (N réduit) est bien détectée par `--baseline` » (`:124`). La fiche ne cite aucun invariant.

### Ce que la conception pose, et ce qu'elle laisse ouvert

`docs/06:124-144` décrit `crible eval` : une ligne de commande à cinq options (`--index`, `--queries`, `--modes`, `--k`, `--out`, `:127`), un format d'entrée JSONL de quatre membres (`{id, query, filter?, expected?}`, `:130-136`), trois familles de mesures — fidélité (`:139`), qualité (`:140`), latence (`:141`) —, la comparaison `--baseline` (`:142`) et deux sorties, « un JSON brut et un rapport Markdown dans `docs/mesures/` » (`:144`). `docs/07:15` place `eval` dans le binaire `crible` et `:29-50` ne porte **aucun client HTTP** côté Rust.

Les décisions voisines bornent une partie du terrain. ADR-0076, décision 1, définit le recall@10 **au passage** et le recouvrement@10 **au document**, la cible de 0,99 portant sur le recall@10 en moyenne sur toutes les requêtes ; `docs/08:86` reprend la cible et le N = 5000 d'ADR-0078. ADR-0076, décision 4, force BINARY par `PATCH search.exact_threshold = 0`. ADR-0092, décision 2, définit recall@10 et MRR@10 **au document**, sur des résultats regroupés, et son alternative `:132` écarte explicitement nDCG@k, nommé sans définition. ADR-0116, décisions 1 et 2 (a), permet les lectures de la CLI à côté d'un serveur en service sans aucune écriture. ADR-0117, décision 4, fixe le percentile au rang le plus proche. ADR-0119, décision 12, et ADR-0123, décision 10, refusent un drapeau `--debug` ou `--explain` sur `crible search` et renvoient à COMMANDE-EVAL. ADR-0124, décisions 13 et 14, a refusé un client HTTP pour `crible stats`.

### L'état des sources

L'analyse de `@architecte` a été vérifiée aux sources citées ; les dix-sept points sont confirmés, avec deux précisions utiles :

- `docs/06:140` nomme bien « recall@k, MRR et nDCG@k », alors qu'aucun jugement gradué n'existe dans les données et que le seul précédent, ADR-0092, a écarté nDCG faute de définition ;
- `data/legifacile-sample/queries.jsonl` porte `id`, `question`, `corpus`, `structure`, `satisfiable` et un `expected` d'**objets** (`src_id`, `num`, `texte_cid`…), et non le format de `docs/06:133` ; un convertisseur est donc nécessaire. Le fichier n'est pas en CI, et `synth-small` y est généré (`.github/workflows/ci.yml:37-42`) ;
- `docs/05:115` montre `search.rescore_candidates` et `search.exact_threshold` comme clés de configuration, et `docs/05:258-274` ne porte aucun champ de requête qui les remplacerait ;
- `docs/mesures/README.md:5-8` ne prévoit que `MES-xx-<slug>.md`, et `:20` demande d'écrire l'échauffement et le nombre de répétitions, sans les fixer.

### Les écarts et les points non couverts

**Écart 1 — le mode API.** La fiche offre « contre une instance via l'API » (`P7:118`). `docs/07:29-50` ne porte aucun client HTTP, comme ADR-0124 l'a constaté pour `crible stats`. Le mode API demanderait donc une dépendance et un ADR.

**Écart 2 — nDCG@k.** `docs/06:140` le nomme, mais aucune donnée ne porte de jugement gradué et ADR-0092 l'a écarté. Il est incalculable sans inventer des pertinences.

**Écart 3 — le forçage de la stratégie.** ADR-0076, décision 4, passe par un `PATCH` sur des **copies** ; `crible eval` lit un dossier en place et ne doit rien écrire (ADR-0116). `docs/06:139` ne dit pas comment forcer BINARY sans écrire.

**Écart 4 — « N réduit ».** Le critère de la fiche (`P7:124`) parle de N réduit, mais `rescore_candidates` est une clé de configuration, pas un champ de requête (`docs/05:115`).

**Écart 5 — les latences.** `docs/06:141` demande p50/p95/p99 par étape, qui n'existent que dans `debug.timings_ms` (ADR-0119), sans dire comment les obtenir sans changer la requête.

**Points non couverts.** Le mode d'accès au dossier (local ou API) ; l'encodage des requêtes textuelles et le chemin `vector` ; le format exact des entrées et la conversion de l'échantillon ; le niveau de `expected` ; la définition de la qualité et le sort de nDCG ; les surcharges en mémoire ; la régression injectée ; l'échauffement, les répétitions et la règle de percentile ; les drapeaux de `crible search` ; le schéma, les noms et le lieu des rapports ; la définition d'une régression, ses seuils et le code de sortie ; l'écriture dans `docs/mesures/` ; les tests en CI sans l'échantillon ; la démo de J7 ; la confidentialité des rapports.

## Décision

### Décision 1 — Périmètre P7 : mode local seulement, mode API reporté

**Décision.** `crible eval` fonctionne **en local seulement**, sur un dossier de données ouvert **en lecture** (ADR-0116), y compris à côté d'un serveur en service. Le mode « contre une instance via l'API » de la fiche est **reporté** : il demande un client HTTP absent de `docs/07`, donc une dépendance et un ADR, à proposer au propriétaire avec P8 ou P9 (instance déployée). Ce report est écrit comme **écart à la fiche** et annoté dans la fiche par une **note datée**.

Précisions :

- la commande n'ouvre aucun index en écriture, ne prend pas le verrou d'écrivain et n'écrit rien sous le dossier de données ; elle est donc une lecture permise à côté d'un serveur, au sens d'ADR-0116, décision 2 (a) ;
- elle lit une **génération publiée** du dossier, comme `crible search`, et n'exige aucun serveur ;
- le report ne retire pas le livrable : la commande locale couvre la fidélité, la qualité, les latences et `--baseline` ;
- le mode API, quand il viendra, devra réutiliser les mêmes métriques, le même format d'entrée et le même schéma de rapport, pour que les rapports restent comparables.

### Décision 2 — Encodage : modèle de l'index, ou `vector` fourni

**Décision.** Les requêtes **textuelles** sont encodées par le **modèle de l'index** visé. Un élément peut porter `vector` **à la place** de `query` : c'est le seul chemin pour un index `external` (`docs/04:119`, `docs/05:274`) et le chemin des tests de CI.

Précisions :

- un élément porte **exactement un** de `query` ou `vector` ;
- un index à modèle `external` exige `vector` pour chaque élément, et `query` y est refusé avec un message qui nomme le modèle, comme la route de recherche (ADR-0109) ;
- l'encodage se fait au même cache de requêtes qu'une recherche (ADR-0051), et la précision reste celle de la configuration de l'encodeur (ADR-0052) ;
- un `vector` fourni n'est pas ré-encodé : il est utilisé tel quel, comme en `dense` sur la route.

### Décision 3 — Entrée : le format de `docs/06` est normatif, JSONL, converti depuis l'échantillon

**Décision.** Le format d'entrée est celui de `docs/06:130-136`, **normatif** : un fichier **JSONL**, une ligne par requête, `{ "id", "query" | "vector", "filter"?, "expected"? }`. Un **convertisseur Python** dans `tools/bench/` produit ce format depuis `data/legifacile-sample/queries.jsonl`. Une ligne invalide **arrête** la commande avec son **numéro de ligne**.

Précisions :

- `id` est obligatoire et unique ; `query` et `vector` suivent la décision 2 ; `filter` suit la grammaire de `docs/05` ; `expected` suit la décision 4 ;
- le convertisseur prend `question` pour `query`, `expected[].src_id` pour `expected`, et `id` pour `id` ; il n'écrit aucune donnée hors de `data/`, et ses fichiers de sortie restent dans `data/` ;
- le convertisseur relève du **harnais de mesure**, comme `tools/bench/mes10/` (ADR-0092, décision 7) ; il n'entre ni dans le binaire ni dans le client Python ;
- une ligne dont le JSON est invalide, dont un membre est inconnu, ou qui porte à la fois `query` et `vector`, arrête la commande en nommant le **numéro de ligne** et la cause ;
- le format est **fermé** : un membre inconnu est refusé, jamais ignoré.

### Décision 4 — `expected` : des `doc_id`, qualité au document

**Décision.** `expected` est une **liste de `doc_id`**. La qualité se mesure **au document**, sur des résultats **regroupés par document** (`group_by: "document"`), avec `k = 10` par défaut, conformément à ADR-0092, décision 2.

Précisions :

- `expected` est facultatif ; une requête sans `expected`, ou avec un `expected` vide, n'entre pas dans les moyennes de qualité ;
- la fidélité, elle, porte sur **toutes** les requêtes (décision 6) ;
- `expected` porte des `doc_id` externes, jamais un `iid` ni une clé `doc_id#ord` : le niveau de jugement est le document ;
- les résultats sont regroupés par document pour que `k` compte des documents, comme `group_by: "document"` sur la route (ADR-0075) ;
- un `expected` qui nomme un document absent du jeu mesuré n'est pas une erreur : il compte simplement comme non trouvé.

### Décision 5 — Qualité : recall@k et MRR@k, nDCG@k retiré

**Décision.** La qualité se mesure par **recall@k** et **MRR@k**, **par requête** et **en moyenne**, sur les seules requêtes qui ont un `expected` **non vide**. **nDCG@k est retiré** faute de jugements gradués, et `docs/06:140` est corrigé.

Précisions :

- le **recall@k** est |attendus ∩ top-k documents| / |attendus| ; le **MRR@k** est la moyenne de 1/rang du premier attendu, et **0** quand aucun attendu n'est dans le top-k ;
- les définitions sont celles d'ADR-0092, décision 2, reprises telles quelles ;
- la moyenne est calculée sur les requêtes qui ont un `expected` non vide, avec son `n` écrit dans le rapport ; aucune moyenne inter-corpus n'est rendue sans `n` ;
- nDCG@k supposerait un jugement gradué par attendu, qu'aucune donnée ne porte ; l'inventer produirait un chiffre non comparable, et ADR-0092 l'a déjà écarté pour la même raison ;
- `docs/06:140` ne nomme plus nDCG@k.

### Décision 6 — Fidélité : la définition d'ADR-0076, reprise telle quelle

**Décision.** La fidélité reprend **telle quelle** ADR-0076, décision 1 : **recall@10 au passage** contre le même pipeline en `exact: true`, **moyenne sur toutes les requêtes**, et **recouvrement@10 au document** rapporté **sans cible**. La cible de **0,99** de `docs/08:86` est rappelée dans le rapport, avec le N par défaut de 5000 (ADR-0078).

Précisions :

- le recall@10 au passage compare les clés `doc_id#ord` du pipeline normal et celles du même pipeline en `exact: true`, à requête identique ;
- le recouvrement@10 projette les deux top-10 sur `doc_id`, les dédoublonne et divise par le nombre de `doc_id` distincts du top-10 exact ;
- les deux métriques sont rendues par requête et en moyenne, et la moyenne du recall@10 porte sur **toutes** les requêtes, pas seulement celles qui ont un `expected` ;
- la cible de 0,99 porte sur le recall@10 au passage, au N par défaut, en moyenne ; les valeurs par structure sont rapportées quand le jeu les porte, et toute structure sous 0,99 est signalée ;
- le recouvrement@10 n'a pas de cible : il dépend du nombre de passages par document, donc du corpus.

### Décision 7 — Surcharges en mémoire, sans écriture

**Décision.** Deux options remplacent, **pour la durée de la commande seulement**, les valeurs de `search` de la configuration de l'index : `--exact-threshold <n>` et `--rescore-candidates <n>`. Elles n'écrivent **rien** ; le rapport les **reprend**. Par défaut, la fidélité est mesurée avec **`exact_threshold = 0`** (stratégie binaire forcée, comme ADR-0076, décision 4) ; le rapport **vérifie et reprend** `dense_strategy` du plan.

Précisions :

- les surcharges s'appliquent en mémoire à la configuration chargée, sans `PATCH`, sans génération nouvelle et sans verrou d'écrivain ;
- `--exact-threshold 0` fait passer tout pool non vide sous `binary` (ADR-0074) ; c'est le forçage de la fidélité, sans copie ni écriture ;
- la fidélité se mesure avec `exact_threshold = 0` par défaut ; une valeur donnée par l'opérateur est respectée et reprise dans le rapport, avec la conséquence écrite ;
- `--rescore-candidates` remplace N pour la commande ; le N effectif est repris dans le rapport ;
- le rapport reprend `dense_strategy` lu dans le plan de chaque requête mesurée, pour que la mesure de fidélité soit vérifiable ;
- aucun autre réglage de `search` n'est surchargeable : ni `scan`, ni la fusion, ni les prompts.

### Décision 8 — Régression injectée : `--rescore-candidates 20`

**Décision.** La régression de référence du critère de la fiche est `--rescore-candidates 20` avec `k = 10`. Le test de CI vérifie que `--baseline` la **détecte**.

Précisions :

- N = 20 avec `k = 10` réduit fortement les candidats rescorés sans rendre la recherche vide : le top-10 exact n'est plus atteint de façon fiable, et la baisse du recall@10 dépasse le seuil de la décision 12 ;
- la régression est **injectée par la ligne de commande**, sans modification de la configuration de l'index ni du code ;
- le test compare deux exécutions du même index : une référence à N par défaut, une seconde à N = 20, puis `--baseline` sur la seconde ;
- le test vérifie aussi le **code de sortie 1** et la présence de la requête fautive dans le rapport.

### Décision 9 — Latences : `debug` interne, échauffement, `--repeat`, rang le plus proche

**Décision.** `eval` pose **`debug: true` en interne** et lit `timings_ms` (ADR-0119). Une **passe d'échauffement non comptée** précède `--repeat R` répétitions (**défaut 3**). Les p50, p95 et p99 sont rendus **par étape** et pour `total`, au **rang le plus proche** (règle d'ADR-0117, décision 4), en millisecondes. Les latences sont **indicatives**.

Précisions :

- `debug: true` est posé par la commande, pas par le fichier d'entrée : aucune requête du jeu ne porte ce membre ;
- la passe d'échauffement sert à charger les lecteurs, le modèle et les caches ; elle n'entre dans aucun percentile ;
- chaque répétition porte sur le même jeu, dans le même ordre ; `R` répétitions donnent `R` échantillons par requête et par étape ;
- les percentiles sont calculés sur les échantillons bruts, au rang le plus proche, jamais sur une moyenne de percentiles ;
- une étape absente d'un plan n'a pas d'échantillon et est omise ;
- les latences sont **indicatives** : elles ne changent jamais le code de sortie (décision 12) et ne valent pas mesure de performance au sens de `docs/mesures/README.md` ;
- les temps viennent de `debug.timings_ms` ; `eval` ne réimplémente aucun chronomètre.

### Décision 10 — Aucun drapeau `--debug` ni `--explain` sur `crible search`

**Décision.** Aucun drapeau `--debug` ni `--explain` n'est ajouté à `crible search`. La question laissée ouverte par ADR-0119, décision 12, et ADR-0123, décision 10, est **close** : c'est `crible eval` qui lit le plan et les temps, en interne.

Précisions :

- `crible eval` n'a pas besoin de drapeau : il pose `debug: true` lui-même ;
- `crible search` reste la commande de recherche, avec sa sortie de réponse d'API ; un drapeau de diagnostic y ajouterait une surface à documenter et à tester sans emploi ;
- `explain_ids` reste un champ de l'API, sans équivalent CLI, comme ADR-0123, décision 10, l'a décidé.

### Décision 11 — Rapport : JSON fermé et Markdown lisible

**Décision.** La commande écrit **deux fichiers** : un **JSON fermé**, `format_version: 1`, et un **Markdown** lisible sans contexte, au modèle de `docs/mesures/README.md`. Le JSON reprend la commande, l'index, la génération, `k`, les modes, N, `exact_threshold`, la version du binaire et le commit s'il est connu, puis les mesures **par mode** (moyennes et lignes par requête par `id`). Le Markdown suit le modèle but, machine, données, paramètres, résultats, lecture. Les noms sont `eval-<jeu>-<AAAA-MM-JJ>.json` et `eval-<jeu>-<AAAA-MM-JJ>.md`.

Précisions :

- `format_version` vaut **1** ; un lecteur refuse une version inconnue (décision 12) ;
- « fermé » veut dire : membres fixes, aucun membre libre, et un membre sans valeur omis plutôt que rendu `null` ou `0` inventé (précédent d'ADR-0119, décision 3) ;
- le JSON porte les **moyennes** par mode, puis les **lignes par requête**, chacune identifiée par son `id` seulement ;
- le Markdown dit le **but** de la campagne, la **machine**, les **données** (jeu, taille, somme de contrôle quand elle existe), les **paramètres** (modes, `k`, N, `exact_threshold`, `R`, `--baseline` éventuel), les **résultats** (tableaux) et la **lecture** (ce que les chiffres permettent d'affirmer, ce qu'ils ne permettent pas) ;
- `<jeu>` est le slug du fichier de requêtes, c'est-à-dire son nom sans extension, et la date est celle du jour de la campagne ;
- les deux fichiers sont écrits dans le dossier donné par `--out` (décision 13).

### Décision 12 — `--baseline` : régression de qualité et de fidélité, latence indicative

**Décision.** `--baseline <rapport.json>` compare les mesures de **qualité** et de **fidélité** de **même mode**. Une **régression** est une **baisse de plus de 0,005 en valeur absolue** d'une moyenne : recall@k, MRR@k de qualité, ou recall@10 de fidélité. Le rapport liste aussi les **requêtes qui ont perdu un attendu trouvé par la référence**. Le code de sortie vaut **1** en cas de régression, **0** sinon. Les latences sont comparées et rapportées (écart relatif des p50 et p95), **sans jamais** changer le code de sortie : « le rappel prime, la latence est indicative » (`docs/suivi.md:25`). Une référence de `format_version` inconnue ou d'un autre `k` est **refusée** avec un message clair.

Précisions :

- la comparaison est **par mode** : un rapport `dense` ne se compare qu'à un rapport `dense` ; un mode absent de la référence n'est pas comparé et le rapport le dit ;
- le seuil de 0,005 est **absolu**, pas relatif ; il vaut pour les trois moyennes citées, jamais pour le recouvrement@10 ni pour une valeur par requête ;
- la liste des requêtes perdues se calcule sur les `expected` : une requête dont un attendu présent dans la référence a disparu du top-k est nommée par son `id` ;
- le code de sortie est **1** dès qu'une moyenne régresse, **0** sinon, y compris si une latence augmente ;
- un `k` différent rend les moyennes non comparables : la référence est refusée avec un message qui nomme le `k` des deux rapports ;
- une `format_version` inconnue est refusée avec un message qui nomme la version lue ;
- la référence est un rapport **JSON** de la décision 11, pas un Markdown.

### Décision 13 — `--out` obligatoire, aucun écrasement, aucune écriture ailleurs

**Décision.** `--out <dossier>` est **obligatoire**. La commande y écrit les **deux** fichiers et **refuse d'écraser** un fichier existant ; elle n'écrit **jamais ailleurs**. Le versionnement passe par `@git`.

Précisions :

- un fichier déjà présent rend une erreur claire qui nomme le chemin, sans écrire l'autre fichier ;
- la commande ne crée aucun fichier temporaire hors du dossier donné ;
- aucun fichier n'est écrit dans `data/` par la commande, et aucun dans `docs/mesures/` sans `--out` qui le demande ;
- `@dev` produit le rapport de l'échantillon en local, et `@git` le versionne (décision 14) ; la commande elle-même ne commite rien.

### Décision 14 — Tests en CI sur `synth-small`, rapport de l'échantillon en local

**Décision.** Les tests d'`eval` tournent **en CI sur `synth-small`**, avec des requêtes à **`vector`** : aucun nouveau test local n'est exclu de la CI (ADR-0062, ADR-0123, décision 11). Le rapport de l'**échantillon** (`data/legifacile-sample`) est produit **en local par `@dev`** dans `docs/mesures/`, et la démo de J7 le rejoue.

Précisions :

- `synth-small` est généré en CI (`.github/workflows/ci.yml:37-42`) ; ses vecteurs permettent d'exercer `eval` sans modèle ni données de production ;
- les tests de CI couvrent le format d'entrée, les métriques, `--exact-threshold`, `--rescore-candidates`, `--repeat`, `--baseline` et le refus d'écrasement ;
- le test de la régression injectée (décision 8) est ordinaire, en CI, sur `synth-small` ;
- l'échantillon Legifacile reste hors de la CI ; aucun `--skip` n'est ajouté à la liste fermée de `docs/07` ;
- le rapport de l'échantillon est une campagne locale du propriétaire, pas un test ; il est relu par `@relecteur` et versionné par `@git`.

### Décision 15 — `docs/jalons/J7-demo.md`

**Décision.** `docs/jalons/J7-demo.md` est écrit au modèle de `docs/jalons/J6-demo.md` : prérequis, commandes exactes depuis la racine, conversion de l'échantillon, rapport, régression détectée par `--baseline` (code de sortie 1), et les autres démonstrations de P7 (debug, journal, explain, stats).

Précisions :

- les commandes sont copiables et partent de la racine du dépôt ;
- la démo dit l'état propre attendu et ne touche pas aux index existants ;
- la régression est **injectée** par `--rescore-candidates 20`, comme la décision 8 ;
- la démo montre `debug` dans la réponse, le journal de debug, `explain_ids` et `crible stats` ;
- la démo est jouée par `@jalon` à J7, comme `J6-demo.md` l'est à J6.

### Décision 16 — Confidentialité : les rapports n'identifient que par `id`

**Décision.** Le rapport et le JSON n'identifient les requêtes que par leur **`id`**, **sans texte de requête ni valeur de filtre** ; les fichiers d'entrée restent dans `data/`.

Précisions :

- aucune ligne du JSON ni du Markdown ne porte `query`, `filter` ni un extrait de l'un ou de l'autre ;
- l'`id` est un identifiant de jeu de requêtes (`Q01`, `q-001`), pas un `iid` ni un `doc_id` de résultat ;
- les fichiers d'entrée convertis restent sous `data/`, gitignoré ; le rapport versionné ne les cite que par leur nom de jeu ;
- la règle suit INV-13 et ADR-0032 : les questions de l'échantillon sont dérivées de la production, et MES-10 n'identifie déjà les questions que par `id`.

### Décision 17 — Implémentation Rust dans la crate `crible`, convertisseur Python

**Décision.** L'implémentation d'`eval` est **Rust**, dans la crate `crible`, comme **sous-commande du binaire** (`docs/07:15`) ; seul le **convertisseur** est en **Python**, dans `tools/bench/`.

Précisions :

- `eval` réutilise les types et les fonctions du moteur : configuration d'index, ouverture en lecture, encodage, recherche, regroupement ; aucun second moteur n'est écrit ;
- le convertisseur Python lit `data/legifacile-sample/queries.jsonl` et écrit le format de la décision 3, sans toucher au moteur ;
- aucune dépendance nouvelle, ni Rust ni Python : `serde_json` (ADR-0027) et `clap` suffisent, et le convertisseur n'utilise que la bibliothèque standard ;
- `docs/07:15` cite déjà `eval` dans le binaire ; cette décision ne change pas la liste des crates.

## Conséquences

- **Positives** :
  - `crible eval` devient l'instrument unique du propriétaire : fidélité, qualité, latence et régression dans un seul rapport, sans lire le code ;
  - le mode local ne demande ni serveur ni client HTTP, et se lit à côté d'une instance en service sans rien écrire ;
  - les métriques de fidélité et de qualité reprennent les définitions déjà validées (ADR-0076, ADR-0092) : les chiffres de P4, P5 et P7 restent comparables ;
  - les surcharges en mémoire remplacent le `PATCH` sur copies d'ADR-0076, sans écriture ni génération nouvelle ;
  - la régression injectée a une valeur fixe et un test de CI, donc le critère de la fiche est reproductible ;
  - le rapport est fermé et versionné, et son Markdown se lit sans contexte ;
  - aucun nouveau test local exclu, et aucune dépendance nouvelle.
- **Négatives / coûts** :
  - le mode API de la fiche est reporté : une instance déployée ne pourra pas être évaluée par l'API avant un ADR et une tâche en P8 ou P9 ;
  - nDCG@k disparaît de `docs/06`, alors qu'il y était nommé : un lecteur qui l'attendait devra lire cet ADR ;
  - les latences d'`eval` sont indicatives et ne valent pas une mesure MES-xx : elles servent à la lecture, pas à la cible ;
  - le convertisseur ajoute un artefact Python à maintenir pour suivre le format de `queries.jsonl` ;
  - `--out` obligatoire et refus d'écrasement demandent à l'opérateur de nommer chaque campagne, donc de dater ses rapports.
- **À surveiller** :
  - la fidélité mesurée sur l'échantillon (pools d'au plus 50 000 passages) ne vaut pas preuve à 8,58 M ; la confirmation reste en P9 (ADR-0076) ;
  - le seuil de régression de 0,005 en valeur absolue : sur un jeu de quelques dizaines de requêtes, une seule requête vaut plus que le seuil, et le rapport doit dire son `n` ;
  - la durée d'une campagne avec `R = 3` et l'échauffement sur six index : un temps qui rendrait la démo de J7 pénible demanderait un réglage, pas un code différent ;
  - le format de `data/legifacile-sample/queries.jsonl` : un changement de schéma casserait le convertisseur, et la conversion doit être testée ;
  - le mode API, quand il viendra : s'il n'utilise pas les mêmes métriques et le même schéma, les rapports cesseront d'être comparables.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Implémenter le mode API en P7 | `docs/07:29-50` ne porte aucun client HTTP ; ce serait une dépendance et un ADR, comme ADR-0124, décision 13, l'a refusé pour `crible stats` |
| Garder nDCG@k dans `docs/06` | Aucun jugement gradué n'existe : le chiffre serait inventé, et ADR-0092 l'a déjà écarté pour cette raison |
| Mesurer la qualité au passage | Le jugement de `expected` porte des `doc_id`, et ADR-0092, décision 2, mesure au document ; le passage n'est pas l'unité de jugement |
| Prendre `expected` pour des clés `doc_id#ord` | Le jeu de l'échantillon ne porte que des `src_id` de documents ; une clé de passage n'y existe pas |
| Forcer BINARY par `PATCH` sur des copies | `eval` lit en place et ne doit rien écrire (ADR-0116) ; la surcharge en mémoire obtient le même effet sans copie |
| Forcer BINARY par une nouvelle clé de configuration | `exact_threshold` existe déjà et ADR-0074 en fixe le sens ; une clé de plus serait une configuration pour une valeur fixe |
| Lire les latences par un chronomètre propre à `eval` | `debug.timings_ms` porte déjà les neuf étapes (ADR-0119) ; un second chronomètre donnerait deux vérités |
| Ajouter `--debug` à `crible search` pour obtenir les temps | ADR-0119, décision 12, et ADR-0123, décision 10, ont refusé ce drapeau ; `eval` pose `debug` en interne |
| Écrire un seul JSON, sans Markdown | La fiche et `docs/06:144` demandent les deux ; le propriétaire doit pouvoir lire le rapport sans contexte |
| Écrire directement dans `docs/mesures/` sans `--out` | Une commande qui choisit son chemin d'écriture rendrait le refus d'écrasement et le versionnement imprévisibles |
| Écraser un rapport existant | Une campagne est une mesure datée : l'écraser perdrait la trace, et `docs/mesures/README.md` interdit d'écraser une campagne |
| Faire échouer `--baseline` sur une hausse de latence | « Le rappel prime, la latence est indicative » (`docs/suivi.md:25`) ; les latences sont rapportées, pas bloquantes |
| Comparer deux rapports de `k` différents | Les moyennes ne portent pas sur le même top-k ; la comparaison serait fausse |
| Tester `eval` sur l'échantillon en CI | L'échantillon est absent en CI, et ADR-0062 et ADR-0063 ferment la liste des tests locaux exclus ; `synth-small` et ses `vector` suffisent |
| Écrire un second moteur en Python | Le moteur est Rust (`docs/07:15`) ; un second calcul donnerait deux vérités et divergerait |
| Identifier les requêtes par leur texte dans le rapport | Les questions de l'échantillon dérivent de la production (ADR-0032) ; seul l'`id` est versionné |

## Références

- Fiche : `docs/phases/P7-observabilite.md:113-124` (`COMMANDE-EVAL`), jalon J7 `:5-9`.
- Conception : `docs/06-observabilite.md:124-144` (« Éval »), `docs/07-stack-et-deploiement.md:15` (commandes du binaire), `:29-50` (dépendances), `docs/05-api.md:258-278` (champs de recherche, `exact`), `docs/04-donnees-et-ingestion.md:119` (modèle `external`), `docs/08-invariants-et-tests.md:86` (cible de fidélité), `docs/mesures/README.md:5-8` et `:20` (modèle de rapport).
- ADR : ADR-0027 (`serde_json`), ADR-0032 (confidentialité), ADR-0051 (cache des requêtes), ADR-0052 (requêtes fp32), ADR-0062 et ADR-0063 (tests locaux exclus), ADR-0074 (`exact_threshold`), ADR-0075 (regroupement), ADR-0076 (fidélité, décisions 1 et 4), ADR-0078 (N = 5000), ADR-0092 (qualité au document, décision 2, nDCG écarté), ADR-0109 (entrées selon le mode), ADR-0116 (lecture sans écriture), ADR-0117 (percentile), ADR-0119 (objet `debug`), ADR-0123 (pas de `--explain` CLI), ADR-0124 (pas de client HTTP).
- Données : `data/legifacile-sample/queries.jsonl` (`question`, `structure`, `expected[].src_id`) ; `synth-small`, généré par `tools/gen_synth.py` (`.github/workflows/ci.yml:37-42`).
- Suivi : `docs/suivi.md:25` (« le rappel prime, la latence est indicative »), section « Écarts à la conception » (ligne du 2026-11-01 pour COMMANDE-EVAL).
