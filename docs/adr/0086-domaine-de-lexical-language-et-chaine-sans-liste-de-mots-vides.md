# ADR-0086 — Domaine de `lexical.language` et chaîne d'analyse des langues sans liste de mots vides

- **Statut** : Accepté (décidé le 2026-10-28, tel que proposé)
- **Date** : 2026-10-28 (proposition et décision)
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, à la demande de l'orchestrateur, sur une `QUESTION-CONCEPTION` de `@dev` dans la tâche `CONFIGURER-ANALYSEUR` (phase P5)
- **Remplace** : —

## Contexte

La tâche `CONFIGURER-ANALYSEUR` (`docs/phases/P5-lexical-et-hybride.md:14-25`) doit construire la chaîne d'analyse tantivy à partir de la configuration de l'index. Sa fiche impose deux choses qui ne se recouvrent pas :

- livrable `:19` : la chaîne contient `StopWordFilter(langue)`, sans condition ;
- livrable `:20` : « les langues prises en charge : celles de tantivy ».

Or `lexical.language` est aujourd'hui un `String` libre, non validé (`crates/crible-core/src/config.rs`, `LexicalConfig`). La conception ne donne qu'un exemple, `"fr"` (`docs/04-donnees-et-ingestion.md:25`), et rappelle que le réglage est modifiable par `PATCH` avec reconstruction du lexical (`docs/04:49`). Ni `docs/04:25` et `:49`, ni `docs/03-recherche.md:82`, ni `docs/07-stack-et-deploiement.md:41`, ni ADR-0007 ne fixent l'orthographe des valeurs. `docs/05-api.md` ne décrit pas les champs de configuration. La valeur est pourtant **persistée dans `index.json`** et **visible de l'API** : elle décide donc d'un format, et un format demande un ADR (`docs/adr/README.md:11-12`).

Faits mesurés par `@dev` sur tantivy 0.27.0 :

- tantivy n'expose aucun code ISO. Son `Language` (`src/tokenizer/stemmer.rs:12`) a dix-huit variantes nommées : `Arabic`, `Danish`, `Dutch`, `English`, `Finnish`, `French`, `German`, `Greek`, `Hungarian`, `Italian`, `Norwegian`, `Portuguese`, `Romanian`, `Russian`, `Spanish`, `Swedish`, `Tamil`, `Turkish` ;
- `StopWordFilter::new(langue)` renvoie `None` pour cinq d'entre elles — `Arabic`, `Greek`, `Romanian`, `Tamil`, `Turkish` (`src/tokenizer/stop_word_filter/mod.rs:35-60`) ; treize langues ont une liste de mots vides ;
- le stemmer existe pour les dix-huit ;
- le constructeur d'analyseur de tantivy est typé statiquement : un filtre optionnel impose deux branches de construction, ou un `StopWordFilter` à liste vide.

Verdicts de `@architecte` qui encadrent la proposition :

- **Q1, forme des valeurs : NON-COUVERT.** La fiche `:20` fixe l'ensemble des langues, aucune décision ne fixe leur écriture.
- **Q2, étage du refus : CONFORME, déjà tranché.** Une valeur refusée l'est à la **validation de la configuration dans `crible-core`**, en `invalid_request` (400), jamais à l'attache ni dans `crible-store` (`docs/04:64`, ADR-0054:79-80, `docs/05-api.md:31`, `docs/07:10`). Le présent ADR le rappelle sans le redécider.
- **Q3, langue sans liste de mots vides : ECART.** `docs/03-recherche.md:84` et la fiche `:19` imposent `StopWordFilter(langue)` sans condition, alors que la fiche `:20` accepte toutes les langues de tantivy.

## Décision

**1. `lexical.language` prend ses valeurs dans une table figée de dix-huit codes ISO 639-1, en minuscules.** Aucune autre valeur n'est acceptée.

| Code | Langue | Variante tantivy | Stemmer | Liste de mots vides |
|---|---|---|---|---|
| `ar` | arabe | `Arabic` | oui | non |
| `da` | danois | `Danish` | oui | oui |
| `de` | allemand | `German` | oui | oui |
| `el` | grec | `Greek` | oui | non |
| `en` | anglais | `English` | oui | oui |
| `es` | espagnol | `Spanish` | oui | oui |
| `fi` | finnois | `Finnish` | oui | oui |
| `fr` | français | `French` | oui | oui |
| `hu` | hongrois | `Hungarian` | oui | oui |
| `it` | italien | `Italian` | oui | oui |
| `nl` | néerlandais | `Dutch` | oui | oui |
| `no` | norvégien | `Norwegian` | oui | oui |
| `pt` | portugais | `Portuguese` | oui | oui |
| `ro` | roumain | `Romanian` | oui | non |
| `ru` | russe | `Russian` | oui | oui |
| `sv` | suédois | `Swedish` | oui | oui |
| `ta` | tamoul | `Tamil` | oui | non |
| `tr` | turc | `Turkish` | oui | non |

- Le norvégien reçoit le code **`no`**. L'ISO 639-1 ne connaît pas `nb` : `nb` est un code ISO 639-2 et 639-3 (bokmål). Une seule écriture par index impose de choisir entre les deux familles, et `no` est le seul code de la famille 639-1. L'algorithme appliqué reste le stemmer Snowball « Norwegian », c'est-à-dire le **bokmål** : le choix de `no` ne prétend pas couvrir le nynorsk.
- `el` pour le grec : `gr` n'est pas un code ISO 639-1.
- La table est **figée par le présent ADR** : tantivy 0.27.0 sert de référence. Une langue ajoutée ou retirée par une version ultérieure de tantivy demande un nouvel ADR, comme tout changement de la liste.

**2. Seules les minuscules sont acceptées, sans normalisation silencieuse.** `"fr"` est valide ; `"FR"`, `"Fr"`, `" fr"`, `"français"`, `"French"` et `""` sont refusés. Un producteur qui écrit une autre orthographe reçoit une erreur explicite plutôt qu'une valeur corrigée dans son dos : la configuration stockée ne contient que la forme canonique, et deux index ne peuvent pas porter deux écritures du même réglage.

**3. La langue inconnue est refusée à la validation de la configuration, dans `crible-core`, en `invalid_request` (400).** Le contrôle vit au même endroit que l'analyse de la configuration et s'applique à la création de l'index comme à tout `PATCH` (ADR-0054, précision 1). Il n'est ni dans `crible-store`, ni dans le chemin d'attache : la configuration est invalide en elle-même. Ce point est **rappelé**, pas décidé ici.

**4. Les dix-huit langues sont acceptées. Pour les cinq sans liste de mots vides — `ar`, `el`, `ro`, `ta`, `tr` —, la chaîne d'analyse omet `StopWordFilter` ; le reste de la chaîne est inchangé** : `SimpleTokenizer`, `RemoveLongFilter(40)`, `LowerCaser`, `AsciiFoldingFilter`, `Stemmer(langue)`.

Le comportement attendu est celui d'une chaîne sans ce filtre : **les mêmes jetons** que si la chaîne ne comportait aucun filtre de mots vides. La manière d'y parvenir — deux branches de construction, ou un `StopWordFilter` à liste vide — est un détail d'implémentation laissé à `@dev` et n'est pas figée par le présent ADR.

Ce que cet ADR ne décide pas : le traitement des élisions et des identifiants (`L1132-1`, `1240`), qui reste ouvert (Q-04, MES-11) ; les valeurs de `lexical.enabled` et de `lexical.positions` ; une éventuelle normalisation des autres réglages de configuration, qui relève de la surveillance d'ADR-0054.

## Conséquences

- **Positives** :
  - `lexical.language` a un domaine écrit : la valeur stockée dans `index.json` et rendue par l'API est stable et comparable d'un index à l'autre ;
  - l'exemple `"fr"` de `docs/04` reste valide, et la valeur par défaut des gabarits ne change pas ;
  - un producteur qui se trompe de langue reçoit `invalid_request` (400) au lieu d'un échec tardif à l'attache ;
  - l'écart de la fiche `:19` avec sa ligne `:20` est levé sans refuser de langue : aucune capacité de tantivy n'est perdue ;
  - la table nomme le stemmer et la présence d'une liste de mots vides pour chaque langue : la chaîne appliquée se lit sans ouvrir le code de tantivy.
- **Négatives / coûts** :
  - sauter le filtre change les jetons pour les cinq langues concernées, donc le **contenu de l'index lexical** : la branche lexicale n'a pas la même qualité de rappel selon la langue. Il n'y a pas de rupture de correction, puisqu'un changement de réglage lance déjà une reconstruction (`docs/03-recherche.md:92`, `docs/04:49`) ;
  - les valeurs n'ont pas la forme des noms de tantivy : le code de `crible-store` porte une table de correspondance entre les dix-huit codes et l'énumération, à tenir à jour si tantivy change ;
  - le code de refus ajoute un cas à `invalid_request`, déjà surchargé (`docs/05-api.md:31`) ;
  - aucun invariant n'est en jeu : INV-04, INV-05 et INV-09 restent inchangés (`docs/08-invariants-et-tests.md:12-17`). Le refus d'une langue inconnue n'est pas un invariant, seulement un test de la validation.
- **Sans migration** : aucun index existant ne porte encore de lexical, P5 commence. Rien à renormaliser, rien à réécrire dans `index.json`. La valeur par défaut des gabarits reste `"fr"`.
- **État des tâches** : `CONFIGURER-ANALYSEUR` reste **`BLOQUE`** jusqu'à la décision du propriétaire. `docs/suivi.md`, section « Écarts à la conception », doit citer ADR-0086 ; cette ligne est tenue par l'orchestrateur.
- **Documents à modifier à l'acceptation** :
  - `docs/03-recherche.md:82-84` : nommer la forme des valeurs (codes ISO 639-1, minuscules), et rendre `StopWordFilter` conditionnel — absent pour les cinq langues sans liste ;
  - `docs/04-donnees-et-ingestion.md` : ajouter le domaine de `lexical.language` là où les autres domaines sont écrits, section « Ce qui est figé, ce qui est réglable » (`:44-66`), avec le renvoi à ADR-0086. L'exemple `:25` reste `"fr"` ;
  - `docs/phases/P5-lexical-et-hybride.md:19-20` : mêmes précisions dans la fiche `CONFIGURER-ANALYSEUR`, pour que la fiche et l'ADR se lisent de la même façon ;
  - `docs/05-api.md` : **aucune modification nécessaire** ; le document ne décrit pas les champs de configuration, et `invalid_request` couvre déjà « paramètre hors bornes » (`:31`) ;
  - facultatif, à la main du propriétaire : une entrée « Langue d'analyse » au glossaire (`docs/00-glossaire.md`), si le terme doit être employé ailleurs que dans cet ADR.
- **À surveiller** :
  - `lexical.language` est la première valeur de configuration dont le domaine est une **table** et non un intervalle. Si d'autres réglages de ce type apparaissent, l'ADR général sur les bornes de configuration prévu par ADR-0054 (`:110`) sera préférable à une décision par valeur ;
  - une montée de version de tantivy qui change la liste des langues, des stemmers ou des listes de mots vides : signal que la table doit être revue par un nouvel ADR, et non corrigée dans le code ;
  - MES-11 (élisions et identifiants, P5) : si l'absence de liste de mots vides sur l'une des cinq langues se traduit par un rappel sensiblement plus faible, la décision se rouvre — avec une mesure, pas par intuition.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Noms des variantes tantivy (`French`, `Norwegian`, …) | Contredit l'exemple `"fr"` de `docs/04:25` et exposerait dans `index.json` et dans l'API le vocabulaire d'une bibliothèque interne ; une mise à jour de tantivy renommerait un format public |
| Accepter les deux orthographes (code et nom) | Deux écritures pour un même index : la configuration stockée ne serait plus comparable, et un `PATCH` changerait la forme sans changer le comportement |
| Normaliser la casse ou rogner les espaces en silence | Cache une erreur du producteur, et fixe un comportement sans le documenter ; la valeur stockée cesserait d'être celle saisie. Mieux vaut un refus explicite |
| Accepter un `String` libre et laisser tantivy échouer | Déplace l'échec à l'attache ou à la construction de l'analyseur, contrairement à `docs/04:64` et ADR-0054 : la configuration est invalide en elle-même |
| N'accepter que les treize langues pourvues d'une liste de mots vides | Contredit la fiche `:20` (« celles de tantivy ») ; refuse cinq langues sans motif de correction, et rendrait la chaîne d'analyse tributaire de la présence d'une liste |
| Refuser les cinq langues sans liste, mais les laisser dans la table | Même contradiction, avec une table qui annoncerait des valeurs refusées : deux sources de vérité |
| Retenir `nb` pour le norvégien | N'existe pas en ISO 639-1 ; introduirait un code d'une autre famille pour une seule langue, et obligerait le client à connaître deux nomenclatures |
| Accepter `no` et `nb` pour le même index | Deux écritures pour un même comportement : même défaut que l'alternative des deux orthographes |

## Références

- `docs/03-recherche.md:78-92` (branche lexicale, chaîne d'analyse, reconstruction) ;
- `docs/04-donnees-et-ingestion.md:25`, `:44-66` (exemple `"fr"`, réglages réglables, domaines `ingest.*`) ;
- `docs/05-api.md:31`, `:72` (`invalid_request`, clé inconnue dans un `PATCH`) ;
- `docs/07-stack-et-deploiement.md:10`, `:41` (`crible-store`, ligne tantivy 0.27) ;
- `docs/08-invariants-et-tests.md:12-17` (INV-04, INV-05, INV-09) ;
- `docs/phases/P5-lexical-et-hybride.md:14-25` (fiche `CONFIGURER-ANALYSEUR`) ;
- ADR-0007 (tantivy pour BM25), ADR-0015 (API en anglais), ADR-0031 (clé inconnue refusée), ADR-0054 (contrôle de domaine dans `crible-core`, `invalid_request` à la validation, et surveillance `:110`) ;
- MES-11 (élisions et identifiants), Q-04 ;
- tantivy 0.27.0, `src/tokenizer/stemmer.rs:12`, `src/tokenizer/stop_word_filter/mod.rs:35-60`.
