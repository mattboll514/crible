# ADR-0092 — Protocole de MES-10 et MES-11 (qualité de la fusion et analyse française)

- **Statut** : Accepté (accepté par anticipation par le propriétaire le 2026-10-28, à confirmer à la fin de la série MESURER-LEXICAL, MESURER-HYBRIDE)
- **Date** : 2026-10-28 (proposition et acceptation anticipée)
- **Décideurs** : propriétaire
- **Décision** : accepté par anticipation le 2026-10-28, par délégation du propriétaire pour la série MESURER-LEXICAL et MESURER-HYBRIDE, à confirmer à la fin de la série. Les sept décisions sont retenues telles que proposées : les 57 questions étiquetées servent de jeu de référence principal, sur un jeu de mesure enrichi `data/mes10/` qui ajoute les 55 documents attendus absents de l'échantillon ; recall@10 et MRR@10 au document sur les 56 questions satisfiables, avec le rang médian en ligne secondaire ; campagne principale aux défauts de production (`rescore_candidates` et `exact_threshold` à 5000 par double `PATCH`, dense BINARY) et colonne secondaire dense EXACT ; cinq variantes du cas « article 1240 » jugées sur le rang, critère du jalon au top 3 ; MES-11 mesurée par une chaîne d'analyse répliquée dans le harnais, avec quatre variantes V0 à V3 et des sondes déclarées ; règle de décision au recall@10 départagé par le MRR@10 puis par l'α le plus proche de 0,5, avec bootstrap apparié à 95 %, `rrf` restant le défaut en cas d'égalité ou de résultat non concluant ; deux rapports, un harnais hors production et aucune valeur de `.env.prod` dans un fichier versionné.
- **Proposé par** : agent greffier, sur demande de l'orchestrateur, pour la tâche `MESURER-HYBRIDE` (phase P5), à partir des verdicts de `@architecte` du 2026-10-28 (sept points NON-COUVERTS, plusieurs CONFORMES)
- **Remplace** : —

## Contexte

### Ce que la fiche demande

La fiche `MESURER-HYBRIDE` (`docs/phases/P5-lexical-et-hybride.md:106-114`) demande MES-10 et MES-11, puis les choix par défaut. Ses livrables sont : « recall@10 et MRR en dense, lexical, `rrf` et `relative_score` (α ∈ {0,3, 0,5, 0,75}), mesurés sur le jeu de référence » (`:110`), « le cas « article 1240 » et ses variantes » (`:111`) et « l'effet des élisions et des identifiants » (`:112`). Son critère est : « le rapport recommande une fusion par défaut et un traitement des élisions et des identifiants (Q-02, Q-04), qui sont ensuite validés par le propriétaire » (`:113`).

Le jalon J5 reprend deux de ces points comme critères de validation : « en mode hybride, sur l'échantillon Legifacile, une requête qui vise un article par son numéro (type « article 1240 code civil ») le place dans le top 3 » (`:6`), et « la fusion par défaut est choisie et validée (Q-02), ainsi que le traitement des élisions et des identifiants (Q-04) » (`:8`). La ligne « MES-10 et MES-11 sont consignées » (`:7`) exige les deux rapports.

La fiche ne dit ni ce qu'est le « jeu de référence » de P5, ni comment calculer le recall@10 et le MRR, ni avec quelle configuration on mesure, ni quelles variantes exactes portent sur le cas « article 1240 », ni comment comparer des configurations sans conclure sur du bruit. Ce sont des décisions de protocole : sans elles, deux mesures portant le même nom ne seraient pas comparables, et la recommandation ne serait pas reproductible.

### Ce que la conception fixe

- **Les élisions et les identifiants sont à mesurer, pas tranchés.** `docs/03-recherche.md:87` écrit : « Le traitement des élisions (l', d', qu') et des identifiants comme `L1132-1` est à mesurer (MES-11). » ADR-0086 le confirme dans sa section « Ce que cet ADR ne décide pas » (`docs/adr/0086-domaine-de-lexical-language-et-chaine-sans-liste-de-mots-vides.md:68`) : « le traitement des élisions et des identifiants (`L1132-1`, `1240`), qui reste ouvert (Q-04, MES-11) ».
- **Les deux fusions et leur défaut sont écrites.** `docs/03:101-106` fixe `rrf` (Σ 1/(60 + rang), rang à partir de 1) et `relative_score` (min-max par branche, score = α·dense + (1−α)·lexical), puis conclut : « Défaut par index : `rrf`, qui n'a rien à régler. On peut le changer par requête, et choisir le réglage par défaut avec MES-10. » ADR-0008 porte la même réserve : le défaut est `rrf` « en attendant MES-10 (Q-02) », et sa section « À surveiller » demande « MES-10 (qualité par méthode et par α) ».
- **Le domaine de α est fixé, sa valeur ne l'est pas.** ADR-0090 retient `alpha ∈ [0, 1]`, bornes comprises et valeur finie, et précise qu'« aucune valeur par défaut » n'est choisie et que le défaut `rrf` ne change pas. `docs/03:104` ajoute que « α = 0 sert le lexical seul, α = 1 le dense seul » : ces deux bornes sont un contrôle de cohérence, pas une préférence.
- **Les deux branches partagent le même pool** (INV-05, ADR-0008) et chacune sort `candidates_per_branch` résultats (`docs/03:100`). La profondeur d'une branche avant fusion est fixée par la règle de sur-échantillonnage de `docs/03:113-117` : `clamp(k × 20, 200, 1000)` quand la liste est groupée par document, `max(k, 100)` sinon. ADR-0075 y ajoute `max(N, candidates_per_branch)` candidats rescorés côté dense sous BINARY.
- **Le regroupement compte des documents.** `docs/03:113` écrit que, avec `group_by: "document"`, « `k` compte alors des documents ». Un recall@10 au document est donc la métrique qui correspond à ce que rend le moteur, et le passage n'est pas l'unité de jugement de la fiche.
- **Le texte BM25 est l'entrée encodeur** (ADR-0020). Analyser « les mêmes textes de passages » (décision 5) veut donc dire analyser exactement le texte que la branche lexicale indexe.
- **Les requêtes s'encodent en fp32** (ADR-0052) et **N vaut 5000 par défaut** (ADR-0078, qui fixe aussi `exact_threshold` à la création et prévoit qu'un double `PATCH` fait monter un index existant).
- **La production est en lecture seule** (ADR-0032). Toute donnée extraite de la production l'est par SSH en lecture seule, et aucun nom d'hôte ni chemin de production n'entre dans un fichier versionné : seulement le nom des variables de `.env.prod`.
- **La cible de rappel est celle de MES-00.** `docs/08-invariants-et-tests.md:86` porte la règle : la cible est « au moins égal à pgvector (MES-00) sur le jeu de référence ». `docs/01:63` et `docs/08:56` emploient la même expression sans la définir, et `docs/08:57` place `legifacile-full` en phase 9.
- **Le vocabulaire des métriques n'est pas défini.** `docs/06-observabilite.md:105` nomme « recall@k, MRR et nDCG@k » pour la qualité, sans donner de formule, ni le niveau (passage ou document), ni la valeur de `k`. `docs/08:75-76` nomme les mesures MES-10 et MES-11 sans plus de détail.

### Ce qu'aucun texte ne fixe

Le « jeu de référence » de P5 n'est défini nulle part. `docs/08:56` dit seulement que l'échantillon Legifacile porte « un jeu de requêtes de référence », `docs/08:86` et `docs/01:63` parlent de « la » cible de rappel sur « le » jeu de référence, et la tâche `CONSTITUER-JEU-DE-REFERENCE` est prévue en **P9**. En P5, aucun document ne dit donc quelles questions comptent, sur quelles données on les juge, ni comment traiter les attendus absents de l'échantillon. C'est le premier point NON-COUVERT relevé par `@architecte`.

### L'état des données au moment de la décision

`@architecte` a vérifié les faits suivants le 2026-10-28 :

- `data/legifacile-sample/queries.jsonl` porte **300 lignes**, dont **57 questions étiquetées `Q01` à `Q57`**, avec **un attendu chacune**, et **56 satisfiables** : `Q30` ne l'est pas. C'est le jeu de MES-00.
- **Deux questions seulement ont leur attendu dans l'échantillon** (`Q28`, `Q29`). Les 55 autres attendus sont hors de l'échantillon : mesurer le recall@10 sur l'échantillon tel quel donnerait 2/56 au mieux, quelle que soit la fusion.
- **145 requêtes portent `expected_in_sample: true`** : 115 `decisions`, 8 `dole`, 8 `jorf`, 6 `sarde`, 8 `titres`, et **aucune `articles`**.
- **Les configurations de l'échantillon ne portent pas le défaut de production** : `data/legifacile-sample/configs/*.json` écrit `rescore_candidates: 1000` et `exact_threshold: 1000`, là où ADR-0078 fixe 5000.
- **Les vecteurs des 57 questions n'existent pas** : `data/mes00/query-vectors.json` n'en contient que 24.
- **Le modèle fp32 `data/models/mdenseon/` est présent** : les vecteurs manquants peuvent être produits localement.
- **L'attendu du cas « article 1240 » (`Q02`) vaut `LEGIARTI000032041571`** et il est **absent de l'échantillon**, donc du seul jeu dont on dispose aujourd'hui.
- L'échantillon porte **257 626 passages** sur six structures.

Sans enrichissement, ni MES-10 ni le critère « top 3 » du jalon J5 ne peuvent être mesurés. C'est le deuxième point NON-COUVERT.

### Les questions tranchées ici

`@architecte` a rendu ces verdicts le 2026-10-28, avant de confier la mesure à `@mesureur` :

1. **Jeu de référence** : **NON-COUVERT**. Aucun texte ne le définit, et 55 attendus sur 57 sont hors de l'échantillon.
2. **Métriques** : **NON-COUVERT**. `docs/06:105` nomme recall@k, MRR et nDCG sans formule, sans niveau et sans `k` ; la fiche demande recall@10 et MRR.
3. **Configuration mesurée** : **NON-COUVERT**. Les configurations de l'échantillon portent N = 1000 et un seuil exact de 1000, et la fiche ne dit pas s'il faut mesurer aux défauts de production.
4. **Cas « article 1240 »** : **NON-COUVERT**. La fiche dit « et ses variantes » sans les énumérer, et son attendu manque à l'échantillon.
5. **MES-11** : **NON-COUVERT**. « L'effet des élisions et des identifiants » ne dit ni quelles variantes d'analyse, ni sur quelles requêtes, ni avec quel instrument.
6. **Règle de décision** : **NON-COUVERT**. La fiche demande une recommandation sans dire ce qui la fonde, ni ce qui se passe en cas d'égalité.
7. **Harnais et artefacts** : **NON-COUVERT**. Les deux mesures demandent des rapports, des artefacts et un code de mesure qui n'existent pas ; le précédent de MES-01, MES-02 et MES-06 doit être suivi ou amendé.

Plusieurs points sont **CONFORMES** : les deux branches sur le même pool (INV-05), `rrf` à 60 pour la constante, le domaine de α (ADR-0090), le texte BM25 égal à l'entrée encodeur (ADR-0020), N = 5000 (ADR-0078), les requêtes fp32 (ADR-0052), et le fait que Q-02 et Q-04 ne se closent que par validation du propriétaire.

Aucune de ces questions ne se tranche par la lecture seule : elles fixent un jeu, des définitions, une règle de décision et un instrument.

## Décision

**MES-10 et MES-11 ont un protocole unique : les 57 questions étiquetées servent de jeu de référence principal, sur un jeu de mesure enrichi qui contient les 55 documents attendus absents de l'échantillon ; recall@10 et MRR@10 sont mesurés au document, aux défauts de production ; le cas « article 1240 » se juge sur cinq variantes et le top 3 ; MES-11 se mesure par une chaîne d'analyse répliquée dans le harnais ; une règle de décision statistique tranche, `rrf` restant le défaut par défaut.**

Précisions :

1. **Jeu de référence de P5.** Le jeu **principal** est celui des **57 questions étiquetées** `Q01` à `Q57`, avec les moyennes calculées sur les **56 satisfiables** et `Q30` **comptée à part**, jamais mêlée aux moyennes. Il est mesuré sur un **jeu de mesure enrichi**, `data/mes10/`, défini comme une **copie de l'échantillon Legifacile** plus les **documents attendus absents** (55 documents), chacun avec **tous ses passages** et ses **vecteurs fp16 de production**, au **format des lots de l'échantillon**. Ces documents sont **extraits en lecture seule de la production** (ADR-0032) par un **script de `tools/bench/mes10/`** ; **aucun nom d'hôte ni chemin de production** n'apparaît dans un fichier versionné, seulement le **nom des variables de `.env.prod`**. Les **257 626 passages** de l'échantillon servent de **distracteurs**. Le jeu **secondaire** est celui des **145 requêtes à `expected_in_sample: true`**, employé **en robustesse seulement** : aucune ne vise `articles`, elles ne peuvent donc pas porter de **garde-fou par corpus**. La **cible de 0,482** de MES-00 **n'est pas comparée en P5** : le jeu enrichi est **plus facile** que la production (attendus ajoutés, 8,58 M distracteurs en moins), et la comparaison reste en **P9**. Le jeu est **provisoire** jusqu'à `CONSTITUER-JEU-DE-REFERENCE` (P9). Si l'extraction est **impossible**, `@mesureur` s'arrête en **`BLOQUE`** et le dit ; **rien n'est remplacé en silence**.
2. **Métriques.** La liste jugée est de **10 documents distincts**, obtenue par `group_by: "document"`, avec `k = 10`. Le **recall@10** est la moyenne, sur les **56 questions**, de |attendus ∩ top-10| / |attendus|. Le **MRR@10** est la moyenne de **1/rang du premier attendu**, et **0** quand aucun attendu n'est dans le top 10. Une **ligne secondaire** donne le **rang médian des questions trouvées**, dans la forme de MES-00 § 2. Les tableaux sont donnés **par corpus avec leur `n`**, et **jamais de moyenne inter-corpus sans `n`**.
3. **Configuration mesurée.** La campagne **principale** tourne aux **défauts de production** : dense **BINARY**, `rescore_candidates` = **5000** et `exact_threshold` = **5000**, posés par **double `PATCH`** sur les **copies de mesure** — jamais sur le jeu versionné de l'échantillon. Une **colonne secondaire** dense **EXACT** (`exact_threshold` au maximum) sert de **plafond** : elle permet d'attribuer un écart à la **fusion** plutôt qu'au **scan**. Les **vecteurs de requête** sont **fp32** (ADR-0052), encodés depuis `data/models/mdenseon/`, et **archivés avec leur SHA-256** dans l'artefact. `k = 10`. La **profondeur de chaque branche avant fusion** est celle du **plan du moteur** pour une liste **groupée par document** (`docs/03:116-117`, `clamp(k × 20, 200, 1000)`), et le rapport l'**écrit telle que le plan la donne**. `rrf` garde la **constante 60**. Les valeurs mesurées de α sont **0 ; 0,3 ; 0,5 ; 0,75 ; 1** ; **α = 0 et α = 1 servent de contrôle de cohérence** : ils doivent **reproduire le lexical seul et le dense seul** (ADR-0090), et un écart signale un **défaut de la mesure**, pas une préférence. `lexical.positions` vaut **`false`** et **n'est pas varié** : MES-06 couvre les deux valeurs (ADR-0091).
4. **Cas « article 1240 ».** Cinq variantes sont mesurées : « l'article 1240 du code civil », « article 1240 du code civil », « article 1240 », « art. 1240 code civil » et « Code civil, article 1240 ». L'attendu est **`LEGIARTI000032041571`**, qui est **présent dans le jeu enrichi**. La mesure est le **rang du document** en **dense**, en **lexical**, en **`rrf`** et **à chaque valeur de α**. Le critère est celui de la fiche : **top 3 en hybride** (`docs/phases/P5-lexical-et-hybride.md:6`).
5. **MES-11, élisions et identifiants.** La **chaîne d'analyse est répliquée dans le harnais**, **hors du moteur** : un **index tantivy de mesure par variante**, construit depuis **les mêmes textes de passages**. Aucun **index** ni **code de production** n'est touché. Les variantes sont : **V0** la **chaîne actuelle** (`SimpleTokenizer`, `RemoveLongFilter(40)`, `LowerCaser`, `AsciiFoldingFilter`, `StopWordFilter`, `Stemmer`) ; **V1** la chaîne actuelle plus un **filtre d'élision** (`qu'il` → `il`, `l'article` → `article`) ; **V2** la chaîne actuelle avec les **identifiants conservés en un seul jeton** (`L. 1132-1` et `L1132-1` donnent le même jeton, le tiret est gardé, et `L. 1132-1` reste discernable de `L. 1132-11`) ; **V3** = **V1 + V2**. Les requêtes sont les **questions contenant une élision** et celles **contenant un identifiant**, **comptées et listées par le harnais**, plus des **sondes déclarées** : « L. 1132-1 », « L1132-1 », « L. 1132-11 », « qu'il », et les **variantes du cas 1240**. Les mesures sont le **recall@10** et le **rang de l'attendu** en **lexical** (principal) et en **`rrf`** (secondaire). Un **résultat positif ne justifie qu'un ADR puis une tâche** : **rien n'est codé dans la campagne**.
6. **Règle de décision.** La métrique **principale** est le **recall@10 au document sur les 56 questions**. Le **départage** se fait par le **MRR@10**, puis par l'**α le plus proche de 0,5**. Le **test** est un **bootstrap apparié par question**, à **10 000 rééchantillonnages**, avec une **graine fixée**, et l'on rapporte l'**intervalle de confiance à 95 % de la différence contre `rrf`**. Une autre configuration n'est **recommandée** que si la **borne basse dépasse 0**, **sans régression sur le jeu secondaire**, et **sans perte au-delà du bruit** sur `juri_texte_integral` et `textes_vigueur`. En cas d'**égalité** ou de **résultat non concluant**, **`rrf` reste le défaut** : c'est le statu quo d'ADR-0008, et c'est une **conclusion valide**. Le rapport **écrit la puissance** : avec 56 questions, **une question vaut environ 0,018**, et un écart **sous environ 0,1 n'est pas détectable**. Q-04 suit la **même règle**, avec en plus un **critère explicite sur les sondes** — l'attendu doit être dans le **top 3** — faute de **puissance statistique**. La **recommandation est soumise au propriétaire**, qui **valide** : ni Q-02 ni Q-04 ne sont closes par le présent ADR.
7. **Harnais et artefacts.** Deux **rapports** : `docs/mesures/MES-10-qualite-fusion.md` et `docs/mesures/MES-11-analyse-francaise.md`, au **format de `docs/mesures/README.md`**, avec les **artefacts JSON datés à côté** et des **chemins relatifs**. Le **harnais** est un **exemple Rust** `crates/crible/examples/hybrid_eval.rs`, qui porte aussi la **réplique d'analyse**, plus des **scripts et un agrégateur dans `tools/bench/mes10/`**, commun à MES-10 et MES-11, et des **tests pytest**. L'**API publique ne change pas**, **aucun code de production n'est modifié**, et **aucune valeur de `.env.prod`** n'entre dans un fichier versionné.

Ce qui n'est **pas** décidé ici : la **fusion par défaut** et l'**α par défaut**, qui appartiennent à un ADR consécutif à la mesure et à la validation du propriétaire (Q-02) ; le **traitement des élisions et des identifiants**, qui reste ouvert (Q-04) et ne serait codé qu'après un ADR et une tâche ; le **vrai jeu de référence** et la **comparaison à la cible de 0,482**, renvoyés à `CONSTITUER-JEU-DE-REFERENCE` en P9 ; la valeur de `lexical.positions`, qui relève de MES-06 (ADR-0091).

## Conséquences

- **Positives** :
  - MES-10 et MES-11 deviennent reproductibles : jeu, métriques, configuration, variantes, sondes et règle de décision sont fixés ;
  - le jeu enrichi rend la mesure possible en P5, là où l'échantillon seul ne contient que 2 attendus sur 57 ;
  - les 257 626 passages de l'échantillon restent des distracteurs réels, ce qui évite de mesurer sur un jeu où chaque attendu serait seul dans son corpus ;
  - la colonne dense EXACT permet de distinguer un défaut de fusion d'un défaut de scan, sans confondre les deux ;
  - α = 0 et α = 1 donnent un contrôle de cohérence gratuit : un écart signale une erreur de mesure avant toute conclusion ;
  - la règle de décision rend l'égalité acceptable : `rrf` reste le défaut par défaut, ce qui est une conclusion, pas un échec ;
  - le harnais reste hors production, l'API publique ne bouge pas, et MES-11 ne modifie aucun analyseur du moteur.
- **Négatives / coûts** :
  - l'extraction des 55 documents attendus depuis la production est une étape nouvelle, qui peut échouer et bloquer la campagne ; elle demande un script versionné et une discipline stricte sur `.env.prod` ;
  - le jeu enrichi est **plus facile** que la production, donc le rappel mesuré en P5 ne vaut pas comme rappel de production ;
  - la puissance statistique est faible : avec 56 questions, un écart sous environ 0,1 n'est pas détectable, et le rapport doit le répéter ;
  - MES-11 mesure une **réplique** de la chaîne d'analyse, pas le moteur : un gain mesuré dans le harnais devra être confirmé par une tâche qui modifie l'analyseur ;
  - deux rapports et un harnais commun à deux mesures : le code de mesure doit rester versionné pour que les chiffres restent vérifiables.
- **À surveiller** :
  - **le jeu enrichi surestime le rappel** : tout chiffre de P5 est relatif à ce jeu, et la comparaison à la cible de 0,482 n'a lieu qu'en P9 ;
  - **la décision de Q-02 et Q-04 se rouvre en P9** sur le vrai jeu de référence ; une recommandation de P5 qui ne tient pas à 8,58 M doit être revue, pas défendue ;
  - **un résultat positif de MES-11 ouvre un ADR puis une tâche d'analyseur**, jamais une modification directe de la chaîne de production ;
  - si l'extraction des attendus échoue et que le jeu enrichi ne peut pas être construit, la campagne s'arrête en `BLOQUE` : aucun chiffre n'est publié sur un jeu incomplet ;
  - si le contrôle de cohérence α = 0 ou α = 1 s'écarte du lexical seul ou du dense seul, la campagne est reprise : l'écart est un défaut de mesure, pas un résultat ;
  - si les vecteurs de requête archivés ne portent pas leur SHA-256, la campagne n'est pas reproductible et les chiffres ne sont pas opposables ;
  - la profondeur `clamp(k × 20, 200, 1000)` de `docs/03:116-117` est citée telle que le plan la donne : si le plan change, la campagne se rejoue avant comparaison.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Mesurer sur l'échantillon Legifacile tel quel | 2 attendus sur 57 sont présents : le recall@10 plafonnerait à 0,036 quelle que soit la fusion, et le critère « top 3 » du jalon serait intestable |
| Mesurer sur `legifacile-full` (8,58 M) | C'est la comparaison de P9, elle demande le vrai jeu de référence et un coût d'exploitation qui n'est pas celui de P5 |
| Extraire seulement l'attendu du cas 1240 | Le jeu resterait à 3 attendus sur 57 ; MES-10 ne pourrait pas recommander une fusion |
| Extraire les 55 documents avec tout le corpus `articles` autour d'eux | Le volume et la composition changeraient les distracteurs, et la mesure ne serait plus celle de l'échantillon enrichi |
| Comparer le rappel de P5 à la cible de 0,482 | Le jeu est plus facile que la production (attendus ajoutés, 8,58 M distracteurs en moins) : la comparaison flatterait Crible et serait fausse |
| Prendre les 145 requêtes à `expected_in_sample: true` comme jeu principal | Elles ne visent aucune structure `articles` : elles ne peuvent pas porter de garde-fou par corpus, et elles ne sont pas étiquetées comme les 57 questions |
| Compter `Q30` dans les moyennes | La question n'est pas satisfiable : l'inclure ferait baisser toute configuration d'une quantité qui ne mesure pas la qualité |
| Mesurer au passage au lieu du document | La fiche et le regroupement comptent des documents (`docs/03:113`) ; le recall au passage ne correspondrait pas à ce que le moteur rend |
| Garder les configurations de l'échantillon (N = 1000) | Ce n'est pas le défaut de production : ADR-0078 fixe 5000, et une mesure à 1000 ne dirait rien du comportement servi |
| Modifier les configurations versionnées de l'échantillon | Le jeu est partagé par plusieurs mesures ; ADR-0078 prévoit un double `PATCH`, appliqué ici aux copies de mesure |
| Comparer directement `rrf` et `relative_score` sans colonne EXACT | Un écart pourrait venir du scan binaire et non de la fusion ; la colonne EXACT sert de plafond |
| Varier `lexical.positions` dans cette campagne | MES-06 couvre déjà les deux valeurs (ADR-0091) ; varier ici doublerait la campagne pour une question déjà tranchée |
| Se limiter aux α de la fiche (0,3 ; 0,5 ; 0,75) | Les bornes 0 et 1 coûtent deux points de plus et donnent un contrôle de cohérence du moteur (ADR-0090) |
| Réutiliser les 24 vecteurs de `data/mes00/query-vectors.json` | Ils ne couvrent que 24 requêtes sur 57, et leur provenance ne serait pas celle du protocole ; les encoder en fp32 depuis `mdenseon` est reproductible |
| Mesurer MES-11 en modifiant l'analyseur du moteur | Un essai d'analyseur n'est pas une décision : il faudrait un ADR puis une tâche, et la campagne ne doit toucher aucun code de production |
| Construire un index de mesure dans le moteur | Un index de mesure dans le moteur mêlerait l'expérience et le produit ; un index tantivy par variante, dans le harnais, isole la question |
| Décider sur la seule moyenne, sans test | Une question vaut environ 0,018 : un écart de deux ou trois questions se lirait comme un gain et serait du bruit |
| Corriger pour comparaisons multiples | Une seule comparaison est faite contre `rrf` par configuration, et l'intervalle à 95 % est rapporté tel quel ; le rapport écrit la puissance, ce qui suffit |
| Trancher Q-02 et Q-04 dans le présent ADR | L'ADR fixe le protocole, pas le résultat ; la fiche et `docs/adr/README.md` réservent la clôture des questions au propriétaire |
| Ajouter nDCG@k aux métriques | `docs/06:105` le nomme sans le définir, et la fiche ne demande que recall@10 et MRR ; l'ajouter aurait créé une troisième définition non tranchée |

## Références

- Fiche : `docs/phases/P5-lexical-et-hybride.md:3-11` (but et jalon J5), `:106-114` (`MESURER-HYBRIDE`).
- Conception : `docs/03-recherche.md:87` (élisions et identifiants à mesurer), `:100-108` (fusion, `rrf`, `relative_score`, α et défaut `rrf`), `:113-117` (regroupement et profondeur des branches) ; `docs/06-observabilite.md:105` (recall@k, MRR, nDCG sans définition) ; `docs/08-invariants-et-tests.md:56` (jeu de requêtes de référence), `:75-76` (MES-10 et MES-11), `:86` (cible de rappel) ; `docs/01-vision-et-perimetre.md:63` (critère de réussite) ; `docs/mesures/README.md` (modèle de rapport) ; `docs/mesures/MES-00-reference-pgvector.md:71-98` (57 questions, cible 0,482, forme du rang médian) ; `docs/00-glossaire.md` (pool, fusion, regroupement, jeu de référence).
- ADR liés : ADR-0008 (hybride sur le même pool, `rrf` par défaut en attendant MES-10), ADR-0020 (texte BM25 égal à l'entrée encodeur), ADR-0032 (accès en lecture seule à la production), ADR-0052 (requêtes fp32), ADR-0075 (regroupement et sur-échantillonnage), ADR-0076 et ADR-0077 (protocoles de mesure de P4, forme reprise ici), ADR-0078 (N = 5000 et double `PATCH`), ADR-0086 (`:68`, élisions et identifiants non tranchés), ADR-0090 (domaine de α, aucun défaut), ADR-0091 (protocole de MES-06).
- Données : `data/legifacile-sample/` (300 requêtes, 57 étiquetées, 145 `expected_in_sample`, 257 626 passages, configurations à 1000) ; `data/mes00/query-vectors.json` (24 vecteurs) ; `data/models/mdenseon/` (modèle fp32) ; `data/mes10/` (jeu de mesure enrichi, à construire).
- Code et outils : `crates/crible/examples/hybrid_eval.rs` (harnais, à créer) ; `tools/bench/mes10/` (extraction, scripts, agrégateur, à créer) ; précédents `crates/crible/examples/fidelity.rs`, `dense_latency.rs`, `lexical_latency.rs` et `tools/bench/mes01`, `mes02`, `mes06`.

## Documents modifiés à l'acceptation

| Document | Modification |
|---|---|
| `docs/adr/README.md` | ligne d'index de l'ADR-0092 ; renvoi au protocole dans la table des questions ouvertes, pour Q-02 et Q-04, sans les clore |
| `docs/phases/P5-lexical-et-hybride.md` | section `MESURER-HYBRIDE` : ligne « Protocole » renvoyant à ADR-0092, livrables inchangés |
| `docs/08-invariants-et-tests.md:75-76` | lignes MES-10 et MES-11 : référence à ADR-0092 |
