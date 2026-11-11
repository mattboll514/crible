# ADR-0148 — Jeton canonique des identifiants dans l'analyse lexicale

- **Statut** : Proposé
- **Date** : 2026-11-11
- **Décideurs** : propriétaire
- **Proposé par** : agent `@greffier`, tâche `T14-IDENTIFIANTS-NUS-LEXICAL` (v1.1), sur la base du verdict de `@architecte` du 2026-11-11 et du ticket 14 de Legifacile
- **Remplace** : ADR-0094 (à l'acceptation), en reprenant telle quelle sa décision sur les élisions (V1 écartée, effet nul mesuré)

## Contexte

### Le ticket et son critère

Le ticket 14 de Legifacile (`../legifacile/tickets-crible/14-identifiants-nus-en-lexical.md`) porte sur Crible 1.0.0 et sur des mesures du 2026-11-10 faites en production et lues par `explain_ids`. Son critère d'acceptation est unique et mesurable : **`10-13.625` envoyé en `lexical` sur l'index `decisions` rend `JURITEXT000023607955` dans le top 3**. C'est ce critère qui lève la régression déclarée par l'ADR-0037 **de Legifacile**, dont la décision 4 a été confirmée par le propriétaire le 2026-11-11 : la recherche plein texte de Legifacile ne repose plus que sur le lexical de Crible.

La cause est le découpage. La chaîne d'ADR-0086 commence par `SimpleTokenizer`, qui coupe sur tout caractère non alphanumérique (`crates/crible-store/src/lexical/analyzer.rs:60-66`). `10-13.625` devient donc les trois jetons `10`, `13` et `625`, trois nombres très fréquents : le document est trouvé, mais au rang lexical 234 et au rang hybride 468. `81-70.489` subit le même découpage et sort au rang 9, ce qui ne contredit pas le constat : cette décision est courte. Le NOR `SANP0422530D` est déjà un jeton unique et sort au rang 2. Ce n'est donc pas l'identifiant qui coûte, c'est la ponctuation.

Le ticket relève deux autres défauts, distincts : la forme citée `L. 1132-1` ne produit pas les jetons de la forme stockée `L1132-1` (rang 498 425 sur `articles`, alors que `L1132-1` sort au rang 79), et un numéro entier comme `1240` reste noyé (rang 183) parce qu'il est déjà un jeton unique.

### Pourquoi aucun champ ne porte le pourvoi

Le ticket établit, mesures sur la base de développement à l'appui, que le champ `numero` de l'index `decisions` porte le numéro interne de la juridiction, pas le numéro de pourvoi. Pour `JURITEXT000023607955`, `numero` vaut `41100170` ; le pourvoi `10-13.625` n'apparaît que dans le titre Postgres, donc dans le texte composé par le gabarit `juri_titre_corps` v1, et seulement sous les trois jetons. `select count(*) from juri.decisions where numero in ('10-13.625', '81-70.489')` rend **0**, et 117 388 des 145 408 décisions `CASS` (80,7 %) n'ont aucun `numero`.

Deux conséquences en découlent, et elles ferment deux pistes du ticket. Une route de recherche exacte sur les **champs** ne retrouve pas un pourvoi, puisqu'aucun champ ne le porte : il faudrait un champ des numéros d'affaire **alimenté par le producteur**, c'est-à-dire un contrat de lot. Et un champ lexical `ident` alimenté depuis `document_fields` (`num`, `numero`) ne le retrouve pas davantage. Seul le texte indexé porte le pourvoi, et le texte indexé est l'entrée encodeur (`ADR-0020`, `docs/03-recherche.md:83`).

### Ce que MES-11 et ADR-0094 ont mesuré, et ce qui manque

MES-11 a mesuré quatre variantes, dont **V2**, « identifiants conservés en un seul jeton » (`docs/mesures/MES-11-analyse-francaise.md:38`). V2 **remplaçait** les morceaux : `L. 1132-1` et `L1132-1` donnaient `l1132-1`, et rien d'autre. Deux effets ont été mesurés sur l'échantillon : le rang lexical de `L1132-1` passe de 1 à 4 (`:94`), et la branche lexicale ne renvoie **aucun** document pour le contrôle négatif `l1132-11` (`:100`). ADR-0094 a donc écarté V2 comme option de produit non retenue, faute de questions à identifiant dans le jeu de P5, et a renvoyé la question en P9 (`:33`, `:39`, `:42`).

Cette remesure n'a jamais eu lieu, et la mesure de MES-11 a une limite de méthode écrite noir sur blanc : la branche lexicale venait d'une **réplique**, avec son propre tokenizer et son propre filtre (`:130`), et le rapport dit qu'un gain n'y vaut qu'après modification de l'analyseur (`:117`). Les rangs du ticket, eux, viennent du moteur, mais des index de production de développement ; les index complets de P10 ont été détruits le 2026-11-08 (`docs/suivi.md`, journal de ce jour), si bien qu'aucun index local ne porte aujourd'hui ces rangs.

**Ce qui n'a jamais été mesuré** est la différence entre **remplacer** les morceaux et leur **ajouter** un jeton canonique. Ajouter laisse les jetons de 1.0 en place, dans le même ordre : le rang de `L1132-1` ne peut pas reculer comme il a reculé en V2, et le contrôle négatif garde les morceaux qui le distinguaient. C'est la piste 1 du ticket, et c'est elle que cet ADR spécifie.

### Ce que le présent ADR reprend d'ADR-0094, et ce qu'il rouvre

Q-04 portait deux sujets, les **élisions** et les **identifiants**. Cet ADR **ne rouvre que le second**, et reprend le premier tel quel.

La décision d'ADR-0094 sur les élisions reste en vigueur, sans réserve et sans remesure :

1. **V1, le filtre d'élision, reste écartée** (ADR-0094, décision 2). Son effet mesuré est nul en recall comme en MRR : 0,9149 et 0,7826, identiques à V0, écart `[0 ; 0]` (MES-11), alors qu'elle change 15 213 passages sur 258 071.
2. **La cause en est connue** : la liste de mots vides française retire déjà les élisions simples (ADR-0094, décision 2 et « À surveiller »). Aucun filtre d'élision n'est ajouté, et le présent ADR ne crée aucune valeur de configuration pour les élisions.
3. **Le seul champ ajouté est `identifiers`.** Il porte sur les identifiants, jamais sur les élisions. Son domaine `"none" | "v1"` nomme la **version des formes d'identifiant reconnues** ; `v1` n'est pas la V2 de MES-11, qui remplaçait les morceaux au lieu de s'y ajouter.

Autrement dit, ADR-0094 est remplacé parce qu'il fermait aussi les identifiants ; sa partie « élisions », elle, n'est pas rouverte et se lit désormais ici, à l'identique.

### Les deux trous que la conception laisse

**Le premier trou est l'empreinte d'analyse.** ADR-0106 enregistre `lexical.language` et `lexical.positions` dans la charge utile du commit tantivy, et a explicitement écarté « une empreinte des fichiers du tokenizer » (`docs/adr/0106-empreinte-de-la-chaine-d-analyse-lexicale.md:25`, `:79`). Un changement de tokenizer **sous la même langue** n'est donc pas détecté : un index construit en 1.0 serait servi avec l'analyse 1.1 sans reconstruction, avec des résultats faux et sans erreur. ADR-0143, décision 3, exige désormais que **tout** changement d'analyse change l'empreinte, et renvoie le mécanisme au présent ADR (`docs/adr/0143-politique-de-version-de-la-ligne-1-x-et-procedure-de-montee-et-de-retour-arriere.md:82`). Le suivi le signale comme le point à trancher avant tout code (`docs/suivi.md:639`, `:649`).

**Le second trou est la forme stockée d'un réglage nouveau.** ADR-0143, décision 2, pose qu'un réglage nouveau d'`index.json` est **optionnel** et que son absence vaut le comportement 1.0 (`:71`). Or la forme stockée est aujourd'hui **matérialisée** : `IndexConfig::to_value` écrit chaque réglage explicitement (`crates/crible-core/src/config.rs:696-703`), la création par l'API écrit cette forme (`crates/crible-server/src/state.rs:883-885`), et le job de `PATCH` la réécrit (`docs/adr/0143-…md:76`). Une instance 1.0 qui ouvre un `index.json` portant une clé qu'elle ne connaît pas **refuse de démarrer** : `LexicalConfig::from_value` compare les clés à la liste fermée `["enabled", "language", "positions"]` (`crates/crible-core/src/config.rs:475`) et rend `invalid_request`, et l'échec de chargement d'un index termine l'instance (`crates/crible-server/src/lifecycle.rs:376-385`, « the startup failed; the instance exits »). Matérialiser `"none"` rendrait donc **tout** index créé ou patché sous 1.1 non démarrable en 1.0, même sans le réglage, et rendrait fausse la phrase d'ADR-0143, décision 2, « un index qui **utilise** un réglage nouveau peut ne plus s'ouvrir » (`:73`).

### La classe de compatibilité, et l'erreur de prémisse du ticket

Au sens d'ADR-0143, décision 7, la tâche est déjà classée **« reconstruction ou migration »** (`docs/suivi.md:175`) : la chaîne d'analyse change, donc l'index lexical se reconstruit. Aucun `format_version` public ne bouge : la charge utile du commit tantivy est un marqueur interne d'un dossier **dérivé**, que l'ouverture en écriture sait reconstruire (`crates/crible-store/src/lexical/mod.rs:37-40`, ADR-0106). ADR-0143, décision 1, autorise par ailleurs une **clé de configuration optionnelle dont le défaut est le comportement précédent** (`:57`) : la présente décision reste donc dans le cadre d'une mineure.

**Une prémisse du ticket est fausse et ne doit pas fonder une piste.** Le ticket écrit que `lexical.positions` est « figé à la création ». `docs/04-donnees-et-ingestion.md:49` le classe parmi les réglages **modifiables par `PATCH`**, avec `lexical.enabled` et `lexical.language`, et un `PATCH` déclenche le job `rebuild_lexical` (`:55`, ADR-0105). ADR-0106 le redit (`:15`). La piste 4 du ticket n'est donc pas écartée parce qu'elle serait impossible à régler, mais pour les motifs de la section « Alternatives écartées ».

### Ce qui reste à vérifier

Les références de code du verdict ont été relues pour cet ADR, à l'exception de trois points que la tâche qui appliquera la décision devra confirmer : le comportement exact du client Python devant une réponse `GET /v1/indexes/{index}` qui **omet** `lexical.identifiers` (le client rend des `dict`, ADR-0112, donc rien à déclarer) ; la lecture d'une charge utile de version 2 par un binaire qui écrit la version 3, aujourd'hui refusée par une égalité stricte de version (`crates/crible-store/src/lexical/mod.rs:873-880`) ; et le coût exact, en taille et en durée, de la reconstruction des six index sous `"v1"`, qui est l'objet de la mesure ci-dessous.

## Mesure

Le protocole et la règle de décision sont écrits **avant la campagne**, sur la discipline d'ADR-0092, décisions 5 et 6, reprise par ADR-0094 (`:18`). Les résultats manquent à ce jour : les rangs du ticket ne sont pas reproductibles, puisque les index complets de P10 ont été détruits le 2026-11-08.

### Ce qui est mesuré, et où

La mesure est le **rejeu de Q-04** qu'ADR-0094 prévoyait et qui n'a pas eu lieu. Elle se fait **dans le moteur**, pas en réplique, sur un prototype de la piste 1 derrière le réglage `lexical.identifiers`, sur la branche `tache/T14-IDENTIFIANTS-NUS-LEXICAL` ; si l'ADR est accepté, ce prototype devient l'implémentation et n'est pas jeté.

Deux raisons de refuser la réplique, toutes deux écrites. MES-11 réserve 6 dit que la réplique a son propre tokenizer et son propre filtre, et que la fusion y est recalculée (`docs/mesures/MES-11-analyse-francaise.md:130`) ; sa réserve finale dit qu'un gain mesuré en réplique ne vaut qu'après modification de l'analyseur (`:117`). Et le critère est un **rang absolu** sur l'index `decisions` complet : un échantillon de 258 071 passages a d'autres statistiques de termes, donc un autre `idf`, donc un autre classement, et ne peut pas prouver un top 3 sur 3 589 776 passages.

L'échantillon est donc écarté pour les rangs. L'import est **natif**, par la même voie que la campagne P10, `POST /v1/indexes/{index}/batches?wait=true` (`docs/mesures/import-legifacile-2026-11-06.md:171-175`), depuis les six lots assemblés de `data/legifacile-full/`.

### Données, espace et durées attendues

Les six index complets s'importent depuis `data/legifacile-full/` : environ **25 Go** d'index attachés, plus environ **4 Go** d'espace de travail temporaire à l'attache de `decisions`, libérés à l'arrêt (`docs/mesures/import-legifacile-2026-11-06.md:229-236`, `:248-255`). Somme des durées d'attache mesurées en P10 : environ 2 000 s de jobs, soit **45 min à 1 h** pour une passe complète, modèle et démarrage compris (`:177-184`).

Les rangs se mesurent sur les index qui les portent : `decisions` complet (critère du ticket), `articles` complet (formes d'article), `sarde` complet (NOR). `titres` ne sert pas aux rangs, puisque le gabarit `titre_seul` n'y met pas le numéro dans le texte et que la piste 1 ne peut rien y faire ; il sert aux durées et aux tailles. Les durées de reconstruction lexicale et les tailles de `lexical/` se mesurent **sur les six**, sans extrapolation : c'est un chiffre qui entrera dans la note de version, et ADR-0143 exige des durées **mesurées par index** (`:94`).

Deux passes, avec la même commande de mesure : une passe **1.0** (`identifiers` absent, donc `"none"`), puis une passe **`"v1"`** obtenue par `PATCH` et reconstruction `rebuild_lexical` (ADR-0105). La reconstruction se mesure par la commande, comme ADR-0142 (`:17`), et non par l'attache.

Les rangs du ticket, mesurés en production le 2026-11-10 (234, 79, 183, 9), ne sont **plus reproductibles**. La passe 1.0 les re-mesure localement : c'est une étape du protocole, pas un obstacle, et la règle de décision porte sur les **rangs re-mesurés**, jamais sur les chiffres du ticket.

### Questions

Les **97 questions** du jeu de référence versionné de `docs/legifacile/reference/` sont **inchangées** : c'est la référence de la montée, et la modifier invaliderait la comparaison (ADR-0138, ADR-0143 `:108`). Deux d'entre elles portent déjà un pourvoi dans un texte long (`decisions.jsonl:40` pour `81-70.489`, `:43` pour `10-13.625`).

S'y ajoute un **complément déclaré** de questions à identifiant nu, **hors des 97** et compté à part, sur le modèle du complément déclaré de MES-11 (`docs/mesures/MES-11-analyse-francaise.md:24`) :

- pourvoi : `10-13.625` (critère du ticket) et `81-70.489` ;
- article : `L1132-1` et `L. 1132-1`, attendu `LEGIARTI000045391841` ;
- contrôle négatif : `l1132-11`, sans attendu, qui ne doit pas placer cet article dans le top 3 lexical ;
- NOR : `SANP0422530D`, attendu `SARDBLOC000007251655`.

### Règle de décision

1. **Bloquant** : `10-13.625` en `lexical` sur `decisions` complet rend `JURITEXT000023607955` dans le top 3.
2. **Bloquant** : le contrôle négatif `l1132-11` ne place pas l'article `L1132-1` dans le top 3 lexical sur `articles` complet.
3. **Non-régression** : sur les 97 questions, l'intervalle de confiance à 95 % du bootstrap apparié de la différence de recall@10 **lexical** (`"v1"` − `"none"`) n'est **pas entièrement négatif**. Convention d'ADR-0092, décision 6 : 10 000 rééchantillonnages, graine fixée, différence `variante − référence`.
4. **Budget** : la durée de reconstruction lexicale et la taille de `lexical/` des six index sont rapportées contre les valeurs d'ADR-0142 (`:21-26` : 1, 5, 13, 71, 127 et 185 s), et tout dépassement est expliqué index par index.

Sont rapportés aussi, **sans être bloquants** : le rang hybride de l'attendu du critère, les rangs de `L1132-1` et de `L. 1132-1`, et la part des passages dont les jetons changent (même grandeur qu'en MES-11 `:80-87`).

### Résultats

**À compléter après la mesure.** Aucun chiffre n'est avancé ici. La section recevra, dans cet ordre : le tableau des rangs lexicaux et hybrides de la passe 1.0 et de la passe `"v1"` sur les questions du complément déclaré, le tableau des 97 questions avec recall@10 et MRR@10 lexicaux des deux passes et l'intervalle bootstrap de leur différence, le tableau des six index avec durée de reconstruction et taille de `lexical/` des deux passes, et la part des passages retokenisés.

## Décision

**Un jeton canonique d'identifiant est ajouté aux jetons de la chaîne d'analyse, jamais à leur place, derrière un réglage d'index optionnel `lexical.identifiers` dont l'absence vaut le comportement 1.0 ; le réglage entre dans l'empreinte d'analyse, et la charge utile passe en version 3, lue champ par champ.**

### 1. La piste 1 seule, et ce qu'elle ne règle pas

La piste retenue est la **piste 1 du ticket** : un `Tokenizer` qui émet, **en plus** des morceaux découpés par `SimpleTokenizer`, le jeton canonique de l'identifiant reconnu. Les jetons de 1.0 restent, dans le même ordre, avec les mêmes positions relatives. Aucun vecteur n'est réencodé, `doc_hash` ne bouge pas, et le texte indexé reste l'entrée encodeur (ADR-0020).

Les pistes 2 (champ lexical `ident` pondéré), 3 (route `lookup`) et 4 (`positions` et requêtes de phrase) sont **écartées**, avec leur motif dans la section « Alternatives écartées ». Elles sont écartées ici explicitement pour ne pas être rouvertes au ticket suivant sans un nouvel ADR.

**`1240` nu n'est pas réglé par cette décision.** Un numéro entier est déjà un jeton unique, et le jeton canonique ne s'y ajoute pas : son rang reste celui de 1.0. C'est la piste 2 qui le règle, et elle demande une donnée que l'index n'a pas ; elle est hors de T14.

### 2. Le tokenizer, et le reste de la chaîne inchangé

Le tokenizer est un **`Tokenizer` maison**, enveloppe de `SimpleTokenizer`, enregistré sous le **même** `TOKENIZER_NAME` (`crates/crible-store/src/lexical/mod.rs:96`). Ce nom est inscrit dans le schéma de l'index tantivy : le changer rendrait un index existant illisible, donc une reconstruction que rien n'exige.

Le reste de la chaîne d'ADR-0086 est **inchangé** et s'applique à **tous** les jetons, canonique compris : `RemoveLongFilter(40)`, `LowerCaser`, `AsciiFoldingFilter`, `StopWordFilter(langue)`, `Stemmer(langue)` (`crates/crible-store/src/lexical/analyzer.rs:60-66`).

Le tokenizer reçoit le **texte brut** ; un filtre ne le reçoit pas, et les offsets seuls ne distinguent ni `-` de `.`, ni la présence d'un séparateur. C'est ce qui impose un tokenizer et non un filtre.

**Un test fige l'analyse complète** : `tokens("fr", "L. 1132-1")` vaut `["1132", "1", "l1132-1"]`. Le test actuel de `crates/crible-store/src/lexical/analyzer.rs` (`french_identifier_tokens`) attend `["1132", "1"]` ; il est remplacé par celui-ci, qui enregistre la valeur et la symétrie.

### 3. Les formes reconnues, domaine fermé `v1`

Deux formes seulement sont reconnues, **à une frontière de mot** :

| Forme | Motif | Canonique | Exemples |
|---|---|---|---|
| A, article | une lettre `[A-Za-z]`, puis au plus un `.` et au plus une espace blanche, puis `\d+`, puis `(-\d+)*` | lettre, chiffres et groupes joints par `-` ; point et espace retirés | `L. 1132-1` et `L1132-1` → `l1132-1` ; `R. 233-45` et `R233-45` → `r233-45` |
| B, numéro | `\d{2,}([.-]\d+)+` : au moins deux groupes, le premier de deux chiffres ou plus | groupes joints par `-`, `.` et `-` confondus | `10-13.625` → `10-13-625` ; `81-70.489` → `81-70-489` ; `92-12345` → `92-12345` |

**Ce qui n'est pas reconnu**, et se lit comme une limite du domaine `v1` :

- `1240` nu : reste `1240`, aucun jeton ajouté, rang inchangé (décision 1) ;
- `SANP0422530D` : rien à faire, c'est déjà un jeton unique de douze caractères alphanumériques que `SimpleTokenizer` ne coupe pas (rang lexical 2 mesuré au ticket) ; la forme A exige **une seule** lettre, donc elle ne le capte pas ;
- `ECLI:FR:CCASS:2011:C100123` : hors du domaine `v1`, parce que les `:` le découpent et qu'aucun besoin mesuré ne le porte ;
- une décimale comme `1.5` : le premier groupe d'un chiffre l'exclut ;
- des milliers séparés par une espace, comme `1 240` : l'espace n'est pas un séparateur de la forme B ;
- `N°1234` : `°` n'est ni `.` ni espace, et la forme A exige la lettre en tête.

Le domaine est **fermé et nommé** : l'étendre (ECLI, décimales, milliers) est une valeur nouvelle, donc un nouvel ADR, comme pour la table de langues d'ADR-0086.

### 4. Normalisation, position et côté requête

**Normalisation.** Le tokenizer émet le canonique **avec sa casse et ses accents d'origine** ; `LowerCaser` et `AsciiFoldingFilter` le normalisent ensuite exactement comme les morceaux. Rien n'est normalisé deux fois, donc l'index et la requête ne peuvent pas diverger.

**Mots vides et stemmer.** Le canonique traverse les deux, comme tout jeton. Il n'est jamais un mot vide, puisqu'il porte des chiffres, et le `Stemmer` français ne doit pas le changer : le test de la décision 2 le fige. S'il le changeait, les deux côtés changeraient identiquement, et la propriété qui compte est la symétrie.

**Position.** Quand `lexical.positions` vaut `true`, le canonique porte la **position du morceau dont l'offset d'octet égale le premier octet de l'identifiant**, et **n'incrémente pas** le compteur : c'est un synonyme de l'étendue entière sur une position. Les positions des jetons suivants ne bougent donc pas, et une future requête de phrase sur les morceaux reste valable. L'alternative, la position du **dernier** morceau, donne le même résultat observable aujourd'hui et décale l'ancrage sémantique ; elle est écartée pour cette raison.

**Requête.** La requête passe par la **même** chaîne enregistrée (`crates/crible-search/src/lexical.rs:475-481` construit un OU de termes) : elle gagne le même terme, et l'ajout d'un terme **ajoute une clause `Should`** sans en retirer aucune. Sur une question en texte libre qui porte un identifiant, le terme ajouté est rare, donc d'`idf` élevé, et n'apparie que les passages qui portent l'identifiant : leur score monte, les autres gardent le leur. `explain_ids` affichera ce terme dans `lexical.terms` : c'est le diagnostic attendu d'ADR-0123, pas un changement de contrat. Le risque nommé, et à mesurer, est celui d'un identifiant **incident** dans une question longue (« décision `10-13.625` et délai de prescription »), qui pourrait dominer le score.

**Cas limites à tester.** `10-13.625` (le critère) ; `L. 1132-1` équivalent à `L1132-1`, même rang ; le contrôle négatif `l1132-11` (les morceaux `1132` et `11` n'apparient ni `l1132` ni `1`, et le canonique diffère : l'article ne doit pas être dans le top 3, formulation qui n'exige pas une liste vide) ; `1240` (aucun jeton ajouté) ; `15-02-2011` (canonique ajouté, sans effet attendu sur le rang) ; `SANP0422530D` (inchangé) ; un identifiant répété (`tf` 2) ; un identifiant de 40 octets ou plus, retiré entier des deux côtés par `RemoveLongFilter`.

### 5. Le réglage `lexical.identifiers`

Il est **optionnel**, de domaine fermé `"none" | "v1"`, et **l'absence vaut `"none"`, c'est-à-dire le comportement 1.0**. Toute autre valeur est refusée en `invalid_request` (400), à la création de l'index comme au `PATCH`, comme le domaine de `lexical.language` (ADR-0086, décision 3).

Il est **modifiable par `PATCH`**, avec le job `rebuild_lexical`, comme `language` et `positions` (`docs/04-donnees-et-ingestion.md:49`, `:55`, ADR-0105). La valeur en vigueur ne change qu'à la fin du job. **INV-16 n'est donc pas en jeu** : aucun réglage figé n'est touché.

### 6. La forme stockée : `"none"` n'est pas écrit

`IndexConfig::to_value` **n'écrit pas** `identifiers` quand il vaut `"none"`, et l'écrit quand il vaut `"v1"`. `GET /v1/indexes/{index}`, qui rend la configuration matérialisée (`docs/05-api.md:104`), suit la même règle : le membre est absent pour `"none"`. `from_value` lit l'absence **et** `"none"` comme la même valeur, de sorte qu'un `index.json` écrit à la main avec `"none"` reste lisible.

Le motif est la compatibilité d'ADR-0143, décision 2 : sans cela, tout index créé ou patché sous 1.1, même sans le réglage, porterait une clé 1.1 et empêcherait une instance 1.0 de démarrer (`crates/crible-core/src/config.rs:475`, `crates/crible-server/src/lifecycle.rs:376-385`). Un défaut **constant** n'a pas besoin d'être matérialisé : le seul réglage que `docs/04-donnees-et-ingestion.md` décrit comme matérialisé, `search.exact_threshold` (`:82`), l'est parce que son défaut est **dérivé** d'un autre réglage (ADR-0074, décision 2).

La règle est **figée par cet ADR**, et non laissée à l'implémentation : la restauration d'un instantané exige un `index.json` **identique**, la comparaison portant sur l'**arbre JSON** (`crates/crible-store/src/snapshot.rs:692-733`, `first_difference` à `:737`, ADR-0065, décision 3). Un `index.json` qui nommerait `"none"` explicitement différerait d'un instantané qui l'omet, et la restauration serait refusée avec un message nommant la clé.

Tests demandés : `to_value` omet quand `"none"` et écrit `"v1"` ; `from_value` rend la même configuration pour une clé absente et pour `"none"` ; un aller-retour d'instantané d'un index dont la clé est absente restaure ; une cible qui nomme `"none"` est refusée avec le message nommant `$.lexical.identifiers`.

### 7. L'empreinte : `identifiers` entre dans la chaîne, charge utile en version 3

`lexical.identifiers` entre dans **`AnalysisChain`** (`crates/crible-store/src/lexical/mod.rs:145`), dans la **charge utile** du commit tantivy (`payload`, `:842-850`) et dans la **comparaison à l'ouverture** (`:509-527`, état `OtherChain`), en complément d'ADR-0106. **`PAYLOAD_VERSION` passe à 3** (`:835`). La charge utile de la version 3 nomme **toujours** les trois champs, `identifiers` compris quand il vaut `"none"` : elle est un marqueur interne d'un dossier dérivé, jamais un format public, et une charge utile inconnue se reconstruit (ADR-0106, décision 3 ; `:45` pour le coût d'une reconstruction au retour arrière).

**La lecture d'une charge utile ancienne se fait champ par champ** : un champ absent de la charge utile vaut la **valeur 1.0 de ce champ**. Les versions 1 (nombre décimal, sans chaîne) et 2 (objet JSON avec `language` et `positions`, sans `identifiers`) se lisent donc avec `identifiers = "none"`, `language` et `positions` lus comme aujourd'hui. La version 3 lit les trois champs. Conséquence, et c'est ce qu'on veut : un index 1.0 dont `identifiers` est absent n'est **jamais** reconstruit, et un index où l'on active le réglage l'est **toujours**.

En pratique, le lecteur doit accepter les **versions 1 à 3**, là où le code actuel refuse toute version différente de celle qu'il écrit (`crates/crible-store/src/lexical/mod.rs:873-880`, `internal`). C'est une précision que la lettre d'ADR-0106, décision 3, ne donnait pas, et qui découle de la règle champ par champ.

**La lecture littérale d'ADR-0106, décision 3, est écartée avec son défaut** : elle dit qu'une charge utile ancienne est lue comme construite avec la chaîne d'`index.json` (`:29`). Appliquée au champ nouveau, elle servirait un index 1.0 comme construit avec `"v1"` dès que `index.json` le nomme, sans reconstruction, donc avec des résultats faux et **aucune erreur**.

### 8. Compatibilité et retour arrière

Une instance **1.0 refuse de démarrer** devant un `index.json` qui porte `lexical.identifiers` : `LexicalConfig::from_value` rend `invalid_request` sur la clé inconnue (`crates/crible-core/src/config.rs:475`) et l'échec de chargement d'un index termine l'instance (`crates/crible-server/src/lifecycle.rs:376-385`). Avec la décision 6, **seul un index où le réglage vaut `"v1"` est concerné** : un index créé ou patché sous 1.1 sans le réglage n'écrit pas la clé et reste lisible en 1.0.

Le retour arrière d'un index où le réglage vaut `"v1"` passe par un **instantané pris avant l'activation**, puis par la **recréation de l'index depuis l'`index.json` de l'instantané**, jamais par un simple changement d'image. Le motif est qu'aucune commande documentée ne réécrit l'`index.json` d'un index existant : `crible index create` refuse un index présent, et le seul écrivain est le job de `PATCH` (ADR-0143, `:78`).

Ces deux points sont à **reporter dans la note de version 1.1.0** (`docs/versions/1.1.0.md`, tâche `PUBLIER-V1.1.0`, ADR-0143, décision 4), avec les **durées de reconstruction mesurées** par index.

**Le second point « à trancher » d'ADR-0143 reste ouvert en général** : savoir comment remettre en place l'`index.json` d'avant la montée (fichier posé serveur arrêté, ou index retiré puis restauré) n'est pas décidé ici, et cet ADR ne fournit qu'un chemin pour son propre réglage, l'instantané et la recréation.

### 9. Invariants concernés

| Invariant | Pourquoi | Tests |
|---|---|---|
| **INV-04** (au redémarrage, l'état est la dernière génération commitée ; tout le reste est reconstruit) | c'est le cœur du changement : version 3 écrite, versions 1 et 2 lues sans reconstruction, différence d'`identifiers` reconstruite en écriture, refus `overloaded` en lecture seule | `crates/crible-store/tests/inv_04_lexical_chain.rs`, à étendre (`inv_04_a_legacy_payload_is_served_without_rebuild` existe déjà) |
| **INV-09** (compaction ⇒ mêmes résultats) | la compaction reconstruit le lexical avec la chaîne courante : les mêmes résultats doivent tenir sous `"v1"` | `crates/crible-store/tests/inv_09_lexical_compaction.rs` |
| **INV-05** (même pool pour les deux branches) | un terme ajouté ne doit jamais faire sortir du pool ni entrer un passage hors pool | `crates/crible-search/tests/inv_05_lexical_uses_the_pool.rs` |

Les tests `inv_04_*`, `inv_09_*` et `inv_05_*` sont **étendus d'abord par `@testeur`**, avant le code.

**INV-15 est respecté, sans exception nouvelle** : la charge utile du commit tantivy est **déjà versionnée** (`PAYLOAD_VERSION`, `crates/crible-store/src/lexical/mod.rs:835`), et passer à 3 est une version nouvelle, pas un format non versionné. Deux précisions tiennent au format lui-même : la charge utile est du JSON, sans entier multi-octets, donc l'exigence d'un little-endian explicite ne s'y applique pas ; et le dossier est **dérivé**, donc une version inconnue se reconstruit au lieu d'être refusée (ADR-0106, décision 3), sans qu'aucun `format_version` public ne bouge. **INV-16 n'est pas concerné** : le réglage est modifiable par `PATCH`, donc aucun `409 immutable_setting`.

### Ce qui n'est pas décidé ici

- **Le format exact de la charge utile version 3** au-delà des trois champs : c'est un détail d'implémentation, comme l'encodage de la version 2 l'était pour ADR-0106.
- **Le nombre et la forme des identifiants au-delà du domaine `v1`** : ECLI, décimales, milliers et numéros nus demandent un domaine nouveau, donc un ADR.
- **Le champ `ident` pondéré et une route `lookup`** : écartés (voir les alternatives), et rouverts seulement par un ADR qui dit ce qui a changé.
- **Le comportement de l'index `titres`** : la piste 1 ne peut rien y faire, le numéro n'étant pas dans le texte.
- **La note de version et sa date de publication** : elles reviennent à `PUBLIER-V1.1.0` et au propriétaire.

### Ce que l'acceptation appliquera

| Document | Modification |
|---|---|
| `docs/03-recherche.md:86-91` | la chaîne peut porter un jeton canonique d'identifiant, derrière `lexical.identifiers` ; le texte de la ligne sur ADR-0094 change de renvoi |
| `docs/03-recherche.md:90` | la chaîne enregistrée dans la charge utile comprend `identifiers` ; charge utile version 3, lecture champ par champ |
| `docs/03-recherche.md:96` | la liste des cas de reconstruction ne change pas, la comparaison de chaîne couvrant le champ nouveau |
| `docs/04-donnees-et-ingestion.md:49` | `lexical.identifiers` dans la colonne des réglages modifiables, avec son domaine et son déclenchement de `rebuild_lexical` |
| `docs/04-donnees-et-ingestion.md`, section des domaines | domaine fermé `"none" | "v1"`, absence = `"none"`, et la forme stockée qui omet `"none"` |
| `docs/05-api.md:104` | `GET /v1/indexes/{index}` omet `lexical.identifiers` quand il vaut `"none"` |
| `docs/05-api.md:86`, `:137` | `lexical.identifiers` accepté au `PATCH`, avec `rebuild_lexical` |
| `docs/02-architecture.md:157` | la comparaison de chaîne porte les trois champs, et la lecture d'une charge utile ancienne est champ par champ |
| `docs/adr/0094-…md` | passe en `Remplacé par ADR-0148`, seule modification permise sur un ADR accepté |
| `docs/adr/README.md` | statut de l'index, ligne de Q-04 |
| `docs/versions/1.1.0.md` | clé ajoutée, son défaut, la reconstruction au premier démarrage avec ses durées, et les conditions du retour arrière (ADR-0143, décision 4) |

Aucun de ces documents n'est modifié avant l'acceptation.

## Conséquences

- **Positives** :
  - le critère du ticket devient atteignable sans ré-encodage : le texte des passages, les vecteurs et `doc_hash` ne bougent pas, et seule la branche lexicale se reconstruit ;
  - les jetons de 1.0 restent en place, donc la régression que MES-11 avait mesurée pour V2 (rang de `L1132-1` de 1 à 4, lexical vide sur le contrôle négatif) ne se reproduit pas par construction. MES-11 ne disait rien de l'ajout, qui n'avait jamais été mesuré : c'est l'objet de la section « Mesure » ;
  - l'empreinte d'analyse devient complète : un index construit avec une autre analyse ne peut plus être servi avec des résultats faux et sans erreur, et la règle champ par champ évite de reconstruire inutilement les index 1.0 ;
  - l'omission de `"none"` garde un index créé ou patché sous 1.1 démarrable par une 1.0 tant que le réglage n'est pas activé : la phrase d'ADR-0143, décision 2, reste vraie ;
  - les quatre pistes du ticket sont tranchées, avec leur motif mesuré, donc le ticket suivant ne les rouvrira pas par inadvertance ;
  - `explain_ids` montre le terme canonique dans `lexical.terms` : un identifiant qui ne sort pas reste diagnosticable sans nouvel outil (ADR-0123).
- **Négatives / coûts** :
  - les six index lexicaux se reconstruisent à l'activation, et la somme des durées mesurées en 1.0 vaut déjà 402 s (ADR-0142, `:21-26`) ; la mesure dira ce que le jeton supplémentaire y ajoute ;
  - l'index lexical grossit d'un terme par occurrence d'identifiant, et la part des passages retokenisés est de l'ordre de ce que MES-11 mesurait pour V2 (21,1 %) sans lui être identique : la mesure la chiffrera ;
  - un identifiant **incident** dans une question longue peut dominer un score BM25, puisque le terme ajouté est rare ; c'est le risque à surveiller de cette décision ;
  - une charge utile en version 3 n'est pas lue par un binaire 1.0, qui reconstruirait le lexical : c'est un coût de reconstruction au retour arrière, pas une erreur (même famille qu'ADR-0106) ;
  - un index où le réglage vaut `"v1"` ne revient plus en 1.0 par simple changement d'image : il faut un instantané et une recréation de l'index depuis son `index.json`, et la note de version doit le dire ;
  - une forme citée hors du domaine `v1` (ECLI, décimales, milliers) reste perdue, et un utilisateur n'a aucun signal pour le distinguer d'un identifiant trouvé.
- **À surveiller** :
  - **la mesure de la section « Mesure »** : si le contrôle négatif `l1132-11` place l'article dans le top 3, ou si l'intervalle de la différence de recall@10 lexical est entièrement négatif, la décision ne s'applique pas et le propriétaire rejette l'ADR ;
  - **la durée de reconstruction réelle des six index** contre les 1, 5, 13, 71, 127 et 185 s d'ADR-0142 : un écart net signalerait que le tokenizer coûte plus que la chaîne actuelle, et l'écart serait expliqué index par index ;
  - **une reconstruction à chaque démarrage** : le symptôme serait une voie qui oublie d'écrire `identifiers` dans la charge utile, comme un `M` oublié pour ADR-0088 ;
  - **le rang de `L1132-1` sous `"v1"`** : il doit rester celui de 1.0, puisque les morceaux sont intacts ; un recul signalerait que le canonique a déplacé des positions ou des termes, contre la décision 4 ;
  - **un identifiant incident dans une longue question en texte libre** : un attendu qui sort du top 10 sur les 97 questions pour cette raison dirait que le terme ajouté pèse trop, et la réponse serait une variante de domaine (`v2`), donc un ADR ;
  - **la forme stockée** : un premier `index.json` de production qui nommerait `"none"` explicitement serait un signe qu'un binaire a écrit hors de la règle de la décision 6, et casserait la restauration des instantanés qui l'omettent.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| **Piste 2 : un champ lexical `ident` pondéré**, alimenté depuis `document_fields` | Ne retrouve pas le pourvoi, puisque `decisions.numero` porte le numéro interne et vaut `null` pour 80,7 % des décisions `CASS` : le critère du ticket n'est pas atteint. Le régler demanderait un champ neuf alimenté par le producteur, donc un contrat de lot. Il ne règle `1240` et `titres` qu'en second temps, après mesure de la piste 1. Il change aussi le schéma lexical, donc une reconstruction de plus. |
| **Piste 3 : une route `lookup` exacte par champ** | Écartée par la mesure du ticket : `{"numero": "10-13.625"}` sur `decisions` rend zéro résultat, et aucun champ ne porte un pourvoi. Elle ne satisfait donc pas le critère, sauf à ajouter le champ des numéros d'affaire au producteur. Elle serait une route nouvelle permise par ADR-0143, mais sans effet sur le cas qui motive le ticket. |
| **Piste 4 : `positions: true` et requêtes de phrase** | Sa prémisse est fausse : `positions` est modifiable par `PATCH` (`docs/04:49`, ADR-0106 `:15`), donc ce n'est pas un réglage figé. Elle suppose une requête de phrase qui n'existe pas (la requête est un OU de termes, `docs/03:91`, `crates/crible-search/src/lexical.rs:475-481`), et son activation augmente la taille de l'index lexical sans gain démontré. Elle ne sert pas le critère, qui ne demande pas l'adjacence mais le document. |
| **V2 de MES-11 : identifiants conservés en un seul jeton, à la place des morceaux** | V2 **remplaçait**, d'où une régression mesurée : rang lexical de `L1132-1` de 1 à 4, et branche lexicale vide sur le contrôle négatif `l1132-11` (`docs/mesures/MES-11-analyse-francaise.md:94`, `:100`). La piste 1 ajoute au lieu de remplacer, ce qui garde les morceaux de 1.0 et évite cette régression. V2 n'avait d'ailleurs jamais été mesurée dans le moteur, seulement en réplique (`:130`). |
| **Un booléen `lexical.identifiers: true/ false`** | Un booléen ne distingue pas « ces formes » de « des formes étendues » : le jour où l'on reconnaît ECLI ou les décimales, la valeur ne change pas et l'empreinte ne voit rien. Un domaine à valeurs nommées dit la version des formes et suit le précédent de la table de langues d'ADR-0086. |
| **Figer le réglage à la création** | Aucun précédent : `lexical.enabled`, `language` et `positions` sont modifiables, et un réglage figé forcerait un `409 immutable_setting` et une recréation d'index pour un changement qui ne touche qu'un dérivé. Cela contredirait ADR-0143, décision 1, qui n'autorise dans une mineure qu'une clé **optionnelle** sans rupture. |
| **Matérialiser `"none"` dans `index.json`** | Tout index créé ou patché sous 1.1 porterait alors une clé 1.1, et une instance 1.0 refuserait de démarrer, même sans le réglage (`crates/crible-core/src/config.rs:475`, `crates/crible-server/src/lifecycle.rs:376-385`). La phrase d'ADR-0143, décision 2, « un index qui **utilise** un réglage nouveau », deviendrait fausse. |
| **Lire une charge utile ancienne littéralement, comme construite avec la chaîne d'`index.json`** | Servirait un index 1.0 comme construit avec `"v1"` dès que `index.json` nomme la valeur, sans reconstruction, donc des résultats faux et aucune erreur. La lecture champ par champ évite ce défaut sans reconstruire les index 1.0 qui n'ont pas le réglage. |
| **Mesurer sur la réplique de MES-11, ou sur l'échantillon** | La réplique a son propre tokenizer et son propre filtre, et son résultat ne vaut qu'après modification de l'analyseur (`docs/mesures/MES-11-analyse-francaise.md:130`, `:117`). Un échantillon de 258 071 passages a un autre `idf` qu'un index de 3 589 776 passages, et ne peut donc pas prouver un rang absolu dans le top 3. |
| **Élargir le domaine `v1` à ECLI, aux décimales et aux milliers** | Aucun besoin mesuré ne le porte, et chaque forme ajoute un motif, un test et un risque de faux positif ; une décimale comme `1.5` ou une date comme `15-02-2011` deviendraient des identifiants, avec un effet non mesuré sur les 97 questions. Le domaine est fermé et nommé, et son extension est un ADR. |

## Références

- Ticket : `../legifacile/tickets-crible/14-identifiants-nus-en-lexical.md` (critère d'acceptation, constat, « Quel champ porte quel identifiant », pistes 1 à 4, données). Ses ADR non qualifiés sont ceux de Legifacile : ADR-0037 de Legifacile (décisions 3 et 4, filtre `num` et régression du pourvoi nu).
- Verdict de `@architecte` du 2026-11-11, questions 1 à 7 (piste retenue, spécification S1 à S7, réglage, empreinte, invariants), et son complément du même jour : A, données de mesure, qui corrige la question 5 du premier verdict ; B, matérialisation de la clé. Non versionnés, cités par la tâche `T14-IDENTIFIANTS-NUS-LEXICAL`.
- Suivi : `docs/suivi.md:175` (ligne `T14`, classe « reconstruction ou migration »), `:642-651` (travail en amont, mesure avant décision, ADR du ticket), `:639` et `:649` (l'empreinte d'ADR-0106 à trancher).
- Mesures : `docs/mesures/MES-11-analyse-francaise.md:38` (V2), `:80-87` (passages retokenisés), `:94` et `:100` (rangs et contrôle négatif), `:117` et `:130` (réplique et réserves), `:24` (complément déclaré) ; `docs/mesures/import-legifacile-2026-11-06.md:171-175` (attache par l'API), `:177-184` (durées), `:229-236` et `:248-255` (tailles et espace).
- Conception : `docs/03-recherche.md:83` (texte indexé), `:84-91` (chaîne d'analyse, positions, requête en OU), `:96` (reconstruction) ; `docs/04-donnees-et-ingestion.md:49` et `:55` (réglages modifiables et job), `:360` (restauration, `index.json` identique) ; `docs/05-api.md:86`, `:104`, `:137` (configuration rendue, `PATCH`, job) ; `docs/02-architecture.md:69` (`index.json` sans `format_version`), `:218` (restauration) ; `docs/08-invariants-et-tests.md:12`, `:13`, `:17`, `:23`, `:24` (INV-04, INV-05, INV-09, INV-15, INV-16).
- ADR : ADR-0020 (texte BM25 égal à l'entrée encodeur), ADR-0031 (clé inconnue refusée), ADR-0065 (instantané, `index.json` identique), ADR-0086 (chaîne d'analyse, domaine de `lexical.language`, `:68` ce qu'il ne tranche pas), ADR-0087 (ouverture en lecture seule), ADR-0088 (charge utile, état indécidable), ADR-0092 (protocole de MES-11, décisions 5 et 6), ADR-0094 (analyseur inchangé, **remplacé par le présent ADR**), ADR-0105 (job de `PATCH`), ADR-0106 (empreinte de la chaîne, décisions 1 à 3, alternative `:79`), ADR-0123 (`explain_ids`), ADR-0142 (durées de reconstruction acceptées), ADR-0143 (politique de version, décisions 1 à 4 et 7).
- Code : `crates/crible-store/src/lexical/analyzer.rs:60-66` (chaîne) et `:106` (helper `tokens` des tests) ; `crates/crible-store/src/lexical/mod.rs:37-40` (charge utile d'un dérivé), `:96` (`TOKENIZER_NAME`), `:145` (`AnalysisChain`), `:396` et `:509-527` (`OtherChain` et comparaison), `:835` (`PAYLOAD_VERSION`), `:842-850` (`payload`), `:873-880` (contrôle de version) ; `crates/crible-search/src/lexical.rs:475-481` (OU de termes) ; `crates/crible-core/src/config.rs:475-483` (`LexicalConfig::from_value`), `:696-703` (`to_value`), `:742` (`materialised_json`) ; `crates/crible-server/src/state.rs:883-885` (création par l'API), `:1082-1092` (lecture d'`index.json`) ; `crates/crible-server/src/lifecycle.rs:376-385` (échec de démarrage) ; `crates/crible-store/src/snapshot.rs:692-733` (comparaison d'`index.json`) et `:737` (`first_difference`).
