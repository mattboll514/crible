# ADR-0140 — Protocole de la comparaison avec pgvector (COMPARER-AVEC-PGVECTOR)

- **Statut** : Accepté — décidé par le propriétaire le 2026-11-06 (« ADR-0140 accepté »), sans réserve
- **Date** : 2026-11-06 (proposition et décision le même jour)
- **Décideurs** : propriétaire
- **Proposé par** : `@greffier`, à la demande de l'orchestrateur, pour la tâche `COMPARER-AVEC-PGVECTOR` (P10), d'après les verdicts de `@architecte` du 2026-11-06
- **Remplace** : —

## Contexte

La tâche `COMPARER-AVEC-PGVECTOR` (P10) est la mesure décisive de la v1 : elle confronte Crible à pgvector sur le jeu de référence, et ce sont le **rappel** et la **fidélité** qui sont contraignants ; la latence n'est qu'indicative sur la machine de développement (fiche `docs/phases/P10-validation-a-l-echelle-reelle.md`, section `### COMPARER-AVEC-PGVECTOR`, `:141-153`). Le protocole n'est pourtant écrit nulle part en entier. `@architecte`, interrogé par l'orchestrateur le 2026-11-06 avant toute mesure, a rendu `CONFORME` sur quatre points et `NON-COUVERT` sur les autres, avec un `ECART` sur « la même machine ». Cet ADR fixe le protocole. Il ne mesure rien et ne modifie pas la fiche.

### Les faits

- **La base complète est sur la machine de développement.** Six index, **6 660 239 documents** et **9 600 105 passages**, comptes **égaux à pgvector** (`docs/mesures/import-legifacile-2026-11-06.md:35-36`). Ils sont dans le volume Docker `crible-p10-data` de l'image `crible:p10` (`:87` ; `docs/suivi.md:785`).
- **Six instantanés existent déjà.** Ils ont été pris hors ligne le 2026-11-06 dans `data/p10-exchange/snapshots/`, pour **23,73 GiB (25,48 Go)** au total, et sont complets (`import-legifacile-2026-11-06.md:19`, `:396`, `:411`, `:414` ; `docs/suivi.md:153`).
- **pgvector est en production sur le Mac mini.** Il est interrogé en lecture seule par SSH (ADR-0032, point 2). MES-00 a mesuré la référence le 2026-10-20, hors de `$PROD_DAILY_WINDOW`, par SQL direct sur `api.recherche_semantique` (`docs/mesures/MES-00-reference-pgvector.md:21`, `:73-77`).
- **Le jeu de référence compte 97 lignes.** Six fichiers `<index>.jsonl`, 97 requêtes, 97 `expected`, 35 filtres, et une ligne `mcp.jsonl` par `id` (`docs/legifacile/jeu-de-reference.md:12-20` ; ADR-0138 `:31`). Aucune ligne ne porte de `vector` : le jeu versionné est textuel (`jeu-de-reference.md:41-42`).
- **La mesure de rappel de référence porte sur 56 questions.** Le seuil de **0,482** se lit sur les **57 identifiants de base** (`Q01` à `Q58` sans `Q40`), dont `Q30` est non satisfiable et comptée à part ; la moyenne des 97 n'est pas comparable à 0,482 (`jeu-de-reference.md:245-251`). MES-00 donne 0,482 sur 56 questions (`MES-00-reference-pgvector.md:73-77`).
- **Les cibles par corpus portent sur 31 et 12 questions.** `juri_texte_integral` ≥ 0,290 sur 31 questions et `textes_vigueur` ≥ 0,750 sur 12 questions ; les corpus de 2 à 4 questions ne sont pas contraints un par un (`docs/suivi.md:31`). La latence chaude vaut p95 ≤ 698 ms, contraignante seulement sur le Mac mini en conteneur (`docs/suivi.md:32`).
- **`decisions` n'a pas de champ `corpus`.** Ses champs de document sont `section`, `juridiction`, `formation`, `nature`, `numero`, `annee` et `date_decision`, plus `partie` au passage (`docs/legifacile/configs/decisions.json`). `articles` porte `fonds`, `etat`, `annee`, `code`, `num`, `texte_cid`, `date_debut` et `date_fin` (`docs/legifacile/configs/articles.json`). pgvector, lui, partitionne `juri_sommaires` et `juri_texte_integral`, que le périmètre `jurisprudence` fusionne : c'est l'écart connu 3 du jeu (`jeu-de-reference.md:220-224`).
- **Le banc de MES-14 ne mesure que le filtre.** Il appelle `Pool::evaluate` seul, sur des fixtures synthétiques qui suivent la distribution de `synth-small`, et ne mesure ni l'encodage ni le scan (`docs/mesures/MES-14-cout-filtres-intervalles.md:40-64`).
- **Le jeu de latence de MES-00 est déjà local.** Les 1 000 titres sont dans `data/mes00/` (`queries.json`, `query-vectors.json`, `titres.csv`) ; le jeu de référence ne les reprend pas, pour ne pas fausser la moyenne de qualité (`jeu-de-reference.md:122-127`).

### Ce que les documents laissent ouvert

1. **La même machine et le rejeu pgvector.** La fiche exige « la même machine » (`:144`) ; ADR-0129 met Crible sur le MacBook et pgvector sur le Mac mini (décision 2, `:92-103`) et rend la latence indicative (décision 7, point 5, `:159`). MES-00 ne couvre en outre que 57 questions, pas les 40 autres lignes du jeu.
2. **L'endroit où tourne Crible.** La mesure peut se faire sur le volume existant ou sur une restauration.
3. **Le vecteur de requête.** Rien ne dit si la comparaison emploie le vecteur de pgvector ou l'encodage de Crible.
4. **Les populations et les corpus.** Le seuil de 0,482 ne se lit ni sur 97 lignes ni sur la moyenne du périmètre `jurisprudence`.
5. **Le mode qui porte J10.** La fiche demande le dense et l'hybride ; ADR-0129, décision 7, point 4, ne nomme aucun mode.
6. **La fidélité à 9,6 M passages.** Le protocole doit dire le N, le seuil exact et le mode.
7. **La latence.** Aucun second jeu n'est prévu pour J10, et `debug.timings_ms` reste indicatif (ADR-0125, décision 9, `:140-153`).
8. **Le rejeu de MES-14 sur l'index réel.** La grandeur est tranchée — p95 de bout en bout (ADR-0046 `:43` ; fiche `:153`) —, mais ni les index, ni les champs, ni les bornes, ni l'outil.
9. **Les points réservés.** `dedupliquer: "contenu"` n'est pas mesurable par ce jeu ; la date figée du 2026-11-03 doit être nommée ; l'écart `partie_exclue` doit être documenté.
10. **La dérive des données.** Les comptes peuvent bouger entre la mesure pgvector et la mesure Crible.
11. **Le rapport.** Le rapport `eval` est spécifié, pas le rapport de comparaison.

### Ce qui est déjà conforme

- **Crible peut mesurer en local, à côté d'un serveur.** `crible eval` ouvre un dossier en lecture, sans verrou d'écrivain (ADR-0125, décision 1, `:46-56` ; ADR-0116, décisions 1 et 2a, `:29-46`).
- **La fidélité a une définition.** recall@10 au passage contre le même pipeline en `exact: true`, moyenne sur toutes les requêtes, cible de 0,99 au N par défaut (ADR-0125, décision 6, `:104-115` ; ADR-0076).
- **N vaut 5000 par défaut** (ADR-0078), et `--exact-threshold 0` force BINARY sans écrire (ADR-0125, décision 7, `:116-128`).
- **La latence de `eval` est lue dans `debug.timings_ms`**, avec échauffement, trois répétitions par défaut et percentiles au rang le plus proche (ADR-0125, décision 9, `:140-153`).
- **Les rapports `eval` et leur confidentialité sont spécifiés** : deux fichiers par campagne, identifiés par `id` seulement (ADR-0125, décisions 11 et 16, `:164-176` et `:226-236`).

### Corrections de citation faites pour cet ADR

- **`$PROD_DAILY_WINDOW` n'est pas nommée par ADR-0032.** ADR-0032, point 2 (`:23-28`), exige seulement de mesurer hors des heures d'usage et jamais pendant la tâche quotidienne. La variable est nommée par MES-00 §1 (`:21`), qui l'a réellement employée. Le présent ADR cite donc MES-00 pour le nom et ADR-0032 pour la règle.
- **« La même machine » est un écart, pas une erreur de citation.** La fiche `:144` l'exige ; ADR-0129, décisions 2 et 7 point 5, le contredisent pour la latence. C'est l'`ECART` relevé par `@architecte`, et la décision 1 ci-dessous le tranche.
- **La restriction de l'hybride sur `vector` vient du format de ligne.** Le complément d'ADR-0125 (`:270-274`) l'explique pour un index `external`, mais la cause vaut pour tout index : une ligne porte exactement un de `query` ou `vector` (décision 2, `:57-67`), et l'hybride demande un texte pour le lexical et un vecteur pour le dense. Le jeu de référence est textuel (ADR-0138 `:31`), donc l'hybride se mesure en texte seulement.
- **Les 25 Go sont ceux des six instantanés.** Le total mesuré est de 23,73 GiB, soit 25,48 Go (`import-legifacile-2026-11-06.md:411`, `:414`), et non 25 Go par index. La décision 2 s'appuie sur ce total.
- **« 31/12 » désigne deux populations, pas un ratio.** Les cibles par corpus portent sur 31 questions (`juri_texte_integral`) et 12 questions (`textes_vigueur`) (`docs/suivi.md:31`).
- **Les noms de champs sont ceux des configurations.** Ils ont été relus dans `docs/legifacile/configs/decisions.json` et `articles.json` ; la décision 8 les emploie tels quels.

## Décision

**La comparaison suit le protocole ci-dessous. Douze points sont tranchés ; les autres sont des rappels de documents acceptés.** Rien ne démarre avant que cet ADR soit `Accepté`.

### Décision 1 — Même machine : écart assumé, pgvector rejoué sur les 97 lignes, en lecture seule et hors fenêtre

- **L'écart est assumé.** Crible mesure sur la machine de développement, pgvector reste en production sur le Mac mini. Le rappel et la fidélité ne dépendent pas de la machine ; seule la latence en dépend, et elle est indicative ici (ADR-0129, décisions 2 et 7 point 5). Le rapport écrit cet écart en tête.
- **pgvector est rejoué aujourd'hui.** Le rejeu porte sur les **97 lignes de `mcp.jsonl`**, par SQL direct sur `api.recherche_semantique`, comme MES-00 (`MES-00-reference-pgvector.md:21`, `:73-77`). Il n'est pas emprunté à MES-00, qui ne couvre que les 57 questions de base et pas les 40 autres lignes.
- **La date est figée au 2026-11-03.** Les deux lignes `APPLICABLE` sont rejouées avec cette date, et le rapport la nomme (`jeu-de-reference.md:112-120`).
- **La lecture est seule.** Sessions en lecture seule (ADR-0032, point 2), hors de `$PROD_DAILY_WINDOW` (`MES-00-reference-pgvector.md:21`), requêtes strictement en série. Aucune écriture, aucun redémarrage. Aucune valeur de production n'est recopiée : seuls les noms de variables apparaissent.

### Décision 2 — Crible mesure en local, dans un conteneur neuf sur le volume existant, serveur arrêté

- **Lieu.** `crible eval` tourne dans un **conteneur neuf** sur le volume `crible-p10-data`, avec le volume des modèles monté. Le serveur est arrêté pendant la campagne, même si `eval` est une lecture permise à côté d'un serveur (ADR-0125, décision 1 ; ADR-0116, décision 2a) : l'arrêt rend la mesure reproductible et évite toute charge tierce.
- **Pas de restauration native.** Les six index sont déjà sur le volume. Restaurer un instantané coûterait 25,48 Go pour un jeu d'index identique, sans utilité pour une lecture (voir les corrections de citation ci-dessus ; `import-legifacile-2026-11-06.md:411`).
- **Pas de mode API.** Le mode API refuse `--exact-threshold` et `--rescore-candidates` (ADR-0131, décision 6, `:99-109`), donc il ne peut pas forcer BINARY : il est inapte à la fidélité au protocole de la décision 6 ci-dessous (ADR-0131, décision 9, `:135-145`).
- **Ce qui n'est pas décidé ici.** Aucune écriture sous `/data`, aucune création d'index, aucun redémarrage de service.

### Décision 3 — Trois passes : dense avec le vecteur de pgvector, dense avec l'encodage de Crible, hybride en texte

- **Passe primaire, dense.** Elle emploie le **vecteur de requête de pgvector**, le même des deux côtés : le vecteur est calculé par l'encodeur de Legifacile, en lecture seule, puis fourni à `crible eval` par la forme `vector` (ADR-0125, décision 2, `:57-67`). C'est la passe qui compare les **classements** sans mélanger la différence d'encodeur. Le format numérique est celui que pgvector emploie, `halfvec(768)` (`MES-00-reference-pgvector.md:44`).
- **Passe secondaire, dense.** Elle emploie l'encodage de Crible sur le même texte. L'écart entre les deux passes mesure la part de l'encodeur dans le résultat ; il est rapporté, jamais corrigé.
- **Passe hybride.** Elle est **textuelle seulement** : une ligne du jeu porte un texte pour la branche lexicale et ne peut pas porter en même temps un vecteur pour la branche dense (ADR-0125, complément `:270-274` ; ADR-0138 `:31`).
- **Où vivent les fichiers.** Le jeu versionné reste textuel. Le fichier des 97 vecteurs de requête va sous `data/`, gitignoré, comme tout fichier d'entrée (ADR-0125, décision 16, `:226-236`).

### Décision 4 — Populations : agréger par `id`, ne comparer que le global et les deux corpus contraints

- **Agrégation.** Les lignes sont agrégées par `id`, avec le corpus de la question de base lu dans `questions.yaml` ; pour les 24 lignes `MES00-*`, le corpus est celui de `data/mes00/queries.json`. **Aucun filtre n'est ajouté** pour reconstituer un corpus : les filtres sont ceux du jeu.
- **Comparaisons contraignantes.** Deux seulement : le **global** sur les 56 questions satisfaisables, `Q30` à part (`jeu-de-reference.md:245-251`), et les **deux corpus contraints**, `juri_texte_integral` sur 31 questions et `textes_vigueur` sur 12 questions (`docs/suivi.md:31`). Chaque question où Crible perd un attendu trouvé par pgvector est listée et expliquée.
- **Lectures non contraignantes.** Les autres périmètres sont rapportés pour lecture, avec leur `n`. Les corpus de 2 à 4 questions ne sont pas contraints un par un (`docs/suivi.md:31`).
- **`jurisprudence` est marquée non comparable.** `decisions` n'a pas de champ `corpus` (`docs/legifacile/configs/decisions.json`), et pgvector fusionne deux classements sous `jurisprudence` : la comparaison par corpus n'a pas de forme côté Crible (`jeu-de-reference.md:220-224`).
- **Ce qui n'est pas décidé ici.** Aucune cible nouvelle, aucun seuil relevé ou abaissé.

### Décision 5 — Le dense porte J10, l'hybride est rapporté

- **Le mode qui tranche.** Le **dense** porte le seuil de 0,482 et les cibles par corpus. C'est le mode que le jeu désigne comme confronté à pgvector (`jeu-de-reference.md:155`), et ADR-0129, décision 7, point 4, ne nomme aucun mode.
- **L'hybride est rapporté.** Il est mesuré sur tout le jeu, et ses pertes par requête sont listées. Il ne fait pas échouer J10 si le dense tient, parce qu'il n'a pas d'équivalent dans le MCP : `recherche_semantique` et `recherche_texte` ne sont pas fusionnés (`jeu-de-reference.md:153-161`).
- **Ce qui n'est pas décidé ici.** Aucun changement de la fusion par défaut ; Q-02 reste tranchée par ADR-0093 et se rouvre seulement sur le vrai jeu, sans que cet ADR la tranche.

### Décision 6 — Fidélité à 9,6 M passages : 97 requêtes, BINARY forcé, N = 5000, dense

- **Protocole.** La fidélité porte sur **tout le jeu** : 97 requêtes, `exact_threshold = 0` (ADR-0125, décision 7), N = 5000 (ADR-0078), **dense seulement** — le scan binaire est dense par construction (ADR-0076). La commande reprend `dense_strategy` du plan et le rapport le vérifie.
- **Pas de nouvelle fidélité HNSW.** La fidélité de pgvector n'est pas remesurée : MES-00 §3 la donne (`MES-00-reference-pgvector.md:111-155`), et sa passe de référence dit ce qui n'a pas été refait (`:521-527`). Le rapport cite MES-00 §3.
- **Ce qui n'est pas décidé ici.** Aucune modification de la cible de 0,99, aucun autre N.

### Décision 7 — Latence : pas de second jeu pour J10, publier les percentiles des 97 en nommant la population

- **Pas de second jeu.** La latence est indicative ici (ADR-0129, décision 7 point 5 ; fiche `:151`). Le rapport publie p50, p95 et p99 des **97 requêtes**, par étape et pour `total`, lus dans `debug.timings_ms` (ADR-0125, décision 9).
- **La différence de population est nommée.** MES-00 a mesuré 1 000 titres et 24 requêtes de fidélité ; le jeu de référence en compte 97, plus courts, et la p95 de 698 ms se lit comme un plafond sur cette population, non comme une comparaison de même population (`jeu-de-reference.md:122-127`).
- **Le second jeu reste possible, sans réexport.** Si le propriétaire l'exige, les 1 000 titres et leurs vecteurs sont déjà dans `data/mes00/` (`queries.json`, `query-vectors.json`, `titres.csv`) ; aucune campagne de production n'est refaite.
- **Ce qui n'est pas décidé ici.** Aucun second jeu n'est exécuté pour J10 ; aucune cible de latence n'est modifiée.

### Décision 8 — Rejeu de MES-14 sur l'index réel : `decisions` et `articles`, domaine plein et domaine réduit

- **Index mesurés.** `decisions` (3 589 776 passages) et `articles` (2 476 715 passages). Ce sont les deux plus gros index ; ils portent des champs de date et d'année et des champs en bitmap (`docs/legifacile/configs/decisions.json`, `articles.json`).
- **Cas « domaine plein ».** Une clause d'intervalle qui couvre tout le domaine observé d'un champ de date ou d'année : `date_decision` ou `annee` pour `decisions` ; `date_debut`, `date_fin` ou `annee` pour `articles`.
- **Cas « domaine réduit ».** La même clause, combinée à une clause en bitmap : `section` (le `fonds` du MCP, `docs/legifacile/index-legifacile.md:110`) ou `juridiction` pour `decisions` ; `etat`, `fonds` ou `code` pour `articles`.
- **Outil.** `crible eval` sur un **jeu filtré généré** donne la p50 et la p95 de **bout en bout** ; le banc de MES-14 est rejoué en complément, pour le coût de `Pool::evaluate` seul (`MES-14-cout-filtres-intervalles.md:57-64`). Le jeu généré reste sous `data/`, une entrée par cas et par index.
- **Comparaison.** Les chiffres sont comparés à **698 ms** à titre **indicatif** ici. La règle d'ouverture d'un ADR d'index de plage reste celle de la fiche : une p95 de bout en bout au-dessus de la cible, mesurée **sur le Mac mini** par `MESURER-SUR-MAC-MINI` (ADR-0046 `:43` ; fiche `:153`).
- **Ce qui n'est pas décidé ici.** Aucun index de plage, aucune correction de performance, aucune modification du banc.

### Décision 9 — Points réservés : `dedup` renvoyé à `BRANCHER-MCP`, date figée, écart `partie_exclue`

- **(a) `dedupliquer: "contenu"` n'est pas mesurable par ce jeu.** Le SQL du MCP déduplique sur l'empreinte du document, alors que `dedup: "content"` de Crible fusionne les passages de contenu identique ; aucune ligne du jeu ne porte cette option, et `crible eval` n'expose pas `dedup` (ADR-0125, décision 4, `:80-91` ; `jeu-de-reference.md:215-219`). La mesure est renvoyée à `BRANCHER-MCP`, côté projet Legifacile (ADR-0129, décision 8, `:164-184`). Le rapport de comparaison le dit, sans conclure.
- **(b) La date est figée au 2026-11-03.** pgvector et Crible sont rejoués avec cette date, et le rapport la nomme (`jeu-de-reference.md:112-120`).
- **(c) L'écart `partie_exclue` est documenté.** L'écart 1 du jeu (`jeu-de-reference.md:206-214`) est écrit dans le rapport, avec ses deux requêtes `Q13-PARTIE-EXCLUE-DEFAUT` et `Q31-PARTIE-EXCLUE-DEFAUT`. Elles sont hors des 56 questions satisfaisables et ne pèsent donc pas sur le seuil.

### Décision 10 — Dérive des données : compter avant et après, séparer l'écart de données de la perte de classement

- **Comptes.** Les comptes de pgvector et de Crible sont relevés **avant** et **après** la mesure, avec la date et l'heure. Le précédent est MES-00 §1, qui documente une dérive de 635 passages sur la base (`MES-00-reference-pgvector.md:55-61`).
- **Écart de données.** Tout `expected` absent de Crible et tout résultat pgvector absent de Crible sont listés comme **écart de données**, hors des pertes de classement. Un `expected` absent compte comme **non trouvé** (ADR-0125, décision 4, `:80-91`), pas comme une erreur de la campagne.
- **Fenêtre.** La mesure de production se fait hors de `$PROD_DAILY_WINDOW` (`MES-00-reference-pgvector.md:21` ; ADR-0032, point 2).
- **Ce qui n'est pas décidé ici.** Aucune resynchronisation, aucune correction de données.

### Décision 11 — Rapports : six rapports `eval`, un rapport de comparaison, un JSON pgvector par requête

- **Six rapports `eval`.** Un par index, au format d'ADR-0125, décision 11 : `eval-<index>-<AAAA-MM-JJ>.json` et `.md` (`:164-176`). Ils portent les passes dense primaire, dense secondaire et hybride.
- **Un rapport de comparaison.** `docs/mesures/comparaison-pgvector-<AAAA-MM-JJ>.md`, au modèle de `docs/mesures/README.md` : but, machines, données, paramètres, résultats, lecture. Il porte le seuil de 0,482, les cibles par corpus, la fidélité, les percentiles, les requêtes gagnantes et perdantes, le rejeu de MES-14 et les écarts.
- **Un JSON pgvector par requête.** Les résultats pgvector sont écrits sous `docs/mesures/`, avec l'`id` et les `doc_id` seulement : **aucun texte de requête, aucune valeur de filtre** (ADR-0125, décision 16). C'est la seule forme publiable d'un résultat de production.
- **Harnais et données.** Le harnais est versionné dans `tools/bench/p10-compare/` ; les entrées, les vecteurs et le jeu filtré généré restent sous `data/`. Aucune dépendance nouvelle.
- **Ce qui n'est pas décidé ici.** Aucun format de rapport nouveau pour `eval` ; le schéma d'ADR-0125 reste `format_version: 1` en local.

### Décision 12 — Rappel contraignant non atteint : la tâche n'est pas `BLOQUE`, le rapport porte la liste d'actions

- **La tâche n'est pas bloquée pour cela.** Si le rappel contraignant n'est pas atteint, la tâche rend son compte rendu et son rapport. Elle ne passe pas en `BLOQUE` : la fiche demande alors « une liste d'actions soumise au propriétaire » (`:152`).
- **La liste est dans le rapport.** Elle nomme, pour chaque question perdante, l'attendu, le corpus, la cause probable — filtre, encodage, N, seuil exact, répartition par corpus, écart de données — et le levier possible. Elle ne code rien et ne tranche rien : le propriétaire décide.
- **Distinction avec la fidélité.** Un rappel insuffisant n'est pas une fidélité insuffisante. Les deux sont rapportés séparément ; la fidélité sous 0,99 est un signal, pas une liste d'actions de qualité.

## Conséquences

- **Positives** :
  - la comparaison devient **reproductible** : lieux, jeu, vecteurs, populations, modes et rapports sont fixés ;
  - le rappel se compare sur les **mêmes vecteurs** des deux côtés, ce qui isole la part de l'encodeur dans une passe secondaire ;
  - les populations sont explicites : 56 questions satisfaisables, 31 et 12 pour les deux corpus contraints, et les autres périmètres en lecture seule ;
  - le dense porte J10, l'hybride reste une lecture, conformément à ce que le jeu dit de pgvector ;
  - les six rapports `eval` et le rapport de comparaison sont bornés à `id` et `doc_id`, sans contenu de production ;
  - la liste d'actions est prévue par le protocole, donc un rappel insuffisant ne bloque pas la campagne.
- **Négatives / coûts** :
  - l'`ECART` « même machine » reste : la latence de Crible et celle de pgvector ne sont pas mesurées sur la même machine, et la comparaison de latence est seulement indicative ;
  - le rejeu pgvector est une lecture de production, soumise à la fenêtre quotidienne et au bon vouloir de la machine servie ;
  - les **97 requêtes** donnent un p99 grossier : le percentile haut repose sur peu d'échantillons, et le rapport doit le dire ;
  - le rejeu de MES-14 sur l'index réel demande un jeu filtré généré et une campagne de plus, absente des six rapports `eval` ;
  - la passe dense secondaire ajoute une campagne et ne tranche rien : elle explique un écart, elle ne le corrige pas ;
  - les 40 lignes hors des 57 questions de base n'ont pas de référence MES-00 : leur comparaison est neuve, donc sans point de contrôle historique.
- **À surveiller** :
  - la dérive des comptes entre le relevé d'avant et celui d'après ; tout `expected` absent est un écart de données, pas une perte de classement ;
  - le rappel global et les deux cibles par corpus ; toute question où Crible perd un attendu trouvé par pgvector est listée ;
  - la fidélité sous 0,99 à 9,6 M passages ;
  - la p95 de bout en bout sur le Mac mini, seule mesure qui rouvre la question de l'index de plage (ADR-0046) ;
  - les pertes de l'hybride par requête, rapportées sans faire échouer J10 ;
  - la taille de l'échantillon pour p99 (97 requêtes), à déclarer dans le rapport.
- **Application.** La fiche n'est pas modifiée par cet ADR. À l'acceptation, si le propriétaire le demande, `@greffier` appliquera la décision aux documents de conception concernés, en citant `ADR-0140` ; le corps de l'ADR accepté ne sera pas réécrit.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Mesurer Crible sur le Mac mini pour respecter « la même machine » | ADR-0129, décision 2, place Crible sur le MacBook et pgvector sur le Mac mini ; déplacer l'instance pour une mesure indicative coûte un transfert et un arrêt de production, sans changer le rappel ni la fidélité. La latence contraignante appartient à `MESURER-SUR-MAC-MINI`. |
| Emprunter les résultats pgvector à MES-00 sans rejouer | MES-00 ne couvre que les 57 questions de base ; les 40 autres lignes du jeu n'ont pas de résultat pgvector, et certaines sont filtrées ou datées. |
| Rejouer pgvector pendant la fenêtre quotidienne | ADR-0032, point 2, et MES-00 §1 imposent de mesurer hors des heures d'usage. |
| Comparer la moyenne des 97 requêtes au seuil de 0,482 | La moyenne des 97 n'est pas comparable à 0,482 : le seuil se lit sur les 57 identifiants de base, et `Q30` est comptée à part (`jeu-de-reference.md:245-251`). |
| Prendre l'encodage de Crible pour la passe primaire | La comparaison mélangerait la différence d'encodeur et la différence de classement. Le vecteur de pgvector est la seule base commune ; l'encodage de Crible reste une passe secondaire. |
| Faire porter J10 par l'hybride | Le MCP ne fusionne pas ses deux outils, et l'hybride n'a pas d'équivalent direct (`jeu-de-reference.md:153-161`). Le dense est le mode confronté ; l'hybride est rapporté. |
| Contraindre chaque corpus un par un | Les corpus de 2 à 4 questions ne sont pas contraints ; seuls `juri_texte_integral` (31 questions) et `textes_vigueur` (12 questions) le sont (`docs/suivi.md:31`). |
| Comparer `jurisprudence` par corpus | `decisions` n'a pas de champ `corpus`, et pgvector fusionne `juri_sommaires` et `juri_texte_integral` ; la comparaison par corpus n'a pas de forme (`jeu-de-reference.md:220-224`). |
| Restaurer un instantané pour mesurer | Les six index sont déjà sur `crible-p10-data` ; une restauration coûte 25,48 Go pour un jeu identique, sans utilité pour une lecture (`import-legifacile-2026-11-06.md:411`). |
| Mesurer par le mode API de `crible eval` | Le mode API refuse `--exact-threshold` et `--rescore-candidates` (ADR-0131, décision 6) ; il ne peut pas forcer BINARY, donc il ne mesure pas la fidélité. |
| Exiger un second jeu de 1 000 titres pour J10 | La latence est indicative ici, et le second jeu ne change pas le rappel. Les 1 000 titres restent disponibles sous `data/mes00/` si le propriétaire l'exige. |
| Prendre la p95 locale comme déclencheur d'un ADR d'index de plage | ADR-0046 `:43` et la fiche `:153` réservent le déclencheur à la p95 de bout en bout mesurée sur le Mac mini par `MESURER-SUR-MAC-MINI`. |
| Mesurer la fidélité de pgvector à nouveau | MES-00 §3 la donne, et sa passe de référence dit ce qui n'a pas été refait (`MES-00-reference-pgvector.md:521-527`). La remesurer ajoute une lecture de production sans changer la comparaison. |
| Remplacer le banc de MES-14 par `crible eval` | Le banc mesure `Pool::evaluate` seul, sans encodage ni scan ; `crible eval` mesure le bout en bout. Les deux chiffres sont complémentaires, pas substituables (`MES-14-cout-filtres-intervalles.md:57-64`). |
| Mettre la comparaison dans les six rapports `eval` seulement | Les rapports `eval` ne portent ni les résultats pgvector ni les écarts de données ; la comparaison a besoin d'un rapport propre. |
| Passer la tâche en `BLOQUE` si le rappel n'est pas atteint | La fiche demande une liste d'actions soumise au propriétaire (`:152`), pas un blocage. La liste est dans le rapport. |

## Références

- `docs/phases/P10-validation-a-l-echelle-reelle.md`, section `### COMPARER-AVEC-PGVECTOR` (`:141-153`) et section `### MESURER-SUR-MAC-MINI` (`:155-175`).
- `docs/mesures/import-legifacile-2026-11-06.md` : synthèse (`:35-44`), volume (`:87`), instantanés (`:396-414`).
- `docs/mesures/MES-00-reference-pgvector.md` : état de la base et fenêtre (`:21`, `:26-70`), qualité (`:73-77`), fidélité HNSW (`:111-155`), latence (`:177`), passe non refaite (`:521-527`).
- `docs/mesures/MES-14-cout-filtres-intervalles.md` : fixtures et protocole (`:40-64`).
- `docs/legifacile/jeu-de-reference.md` : format (`:12-42`), date figée (`:112-120`), modes (`:153-161`), écarts (`:206-224`), lecture (`:245-261`).
- `docs/legifacile/configs/decisions.json` et `docs/legifacile/configs/articles.json` (champs réels).
- `docs/legifacile/index-legifacile.md` (`section` = `fonds` du MCP, `:110`).
- `docs/suivi.md` : cibles (`:31-32`), import (`:153`), tâche (`:155`), volumes conservés (`:785`).
- ADR-0032 (accès à la production), ADR-0046 (déclencheur de l'index de plage), ADR-0076 (fidélité du scan binaire), ADR-0078 (N = 5000), ADR-0116 (lecture à côté d'un serveur), ADR-0125 (commande `eval`, décisions 1, 2, 4, 6, 7, 9, 11 et 16), ADR-0129 (replanification, décisions 2, 7 et 8), ADR-0131 (mode API de `eval`, décisions 6 et 9), ADR-0138 (jeu de référence versionné), ADR-0139 (protocole d'import).
- Verdicts de `@architecte` du 2026-11-06, recopiés dans le contexte de cet ADR.
