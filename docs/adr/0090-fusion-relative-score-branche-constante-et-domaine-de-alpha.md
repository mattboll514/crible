# ADR-0090 — Fusion `relative_score` : branche constante dans la normalisation min-max et domaine de `alpha`

- **Statut** : Accepté (décision du propriétaire le 2026-10-28 ; accepté par anticipation le 2026-10-28, confirmé par le propriétaire le 2026-10-28)
- **Date** : 2026-10-28 (proposition et acceptation anticipée)
- **Décideurs** : propriétaire
- **Décision** : accepté le 2026-10-28 par le propriétaire, tel que proposé, lors d'une acceptation anticipée de sa part, confirmée par le propriétaire au point de fin de l'enchaînement de P5, le 2026-10-28. Les deux décisions sont retenues telles quelles : une branche constante normalise chacun de ses candidats à 1 et une branche vide ne contribue rien, l'absence restant à 0 ; `alpha ∈ [0, 1]`, bornes comprises et valeur finie, refusé en `invalid_request` (400) à la création de l'index, au `PATCH` de `search.fusion` et par requête, par un seul contrôle dans `crible-core` appelé depuis `FusionConfig::from_value` ; aucune valeur par défaut n'est fixée et le défaut `rrf` ne change pas.
- **Proposé par** : agent greffier, sur demande de l'orchestrateur, pour la tâche `FUSIONNER-HYBRIDE` (phase P5), à partir des choix locaux de `@dev` et des verdicts de `@architecte` du 2026-10-28
- **Remplace** : — (précise `docs/03` et ADR-0008, dont le corps n'est pas réécrit : cet ADR tranche deux cas que `docs/03` et ADR-0008 ne couvrent pas, sans changer ni la formule de `rrf`, ni le choix de la fusion par défaut, ni INV-05)

## Contexte

### Ce que la conception et les ADR exigent

- **La fusion hybride est livrée par `FUSIONNER-HYBRIDE`** (`docs/phases/P5-lexical-et-hybride.md:78-91`). Sa fiche demande les deux fusions, `rrf` (constante 60) et `relative_score` (min-max, α), et ses critères exigent des tests unitaires de fusion sur des listes connues et le traitement correct d'un document présent dans une seule branche.
- **`docs/03-recherche.md:102-105`** fixe `relative_score` en trois lignes : « chaque branche est normalisée min-max sur ses propres candidats », « score fusionné = α·dense + (1−α)·lexical », « un document absent d'une branche y vaut 0 ». **ADR-0008:17** dit la même chose en une ligne : « `relative_score` : min-max par branche, pondéré par α ».
- **INV-05** fait du pool l'autorité des résultats : les deux branches lisent le même pool, et aucun résultat ne sort du pool. Le pool peut donc être très petit, jusqu'à un seul passage.
- **Aucun texte ne borne `alpha`.** `docs/03:104` donne la formule sans domaine, et **`docs/04-donnees-et-ingestion.md:57-74`** ne borne que `acceptance_sample`, `acceptance_min_cosine` et `lexical.language`. **ADR-0054:110** et **ADR-0086:68** renvoient expressément les bornes des autres réglages à un ADR : la surveillance d'ADR-0054 couvre les réglages d'`ingest.*`, et l'ADR-0086 déclare ne pas décider « une éventuelle normalisation des autres réglages de configuration ».

### Verdict de `@architecte` — décision 1, branche constante : `NON-COUVERT`

La normalisation min-max divise par `max − min`. Quand tous les candidats d'une branche ont le même score, ce dénominateur vaut zéro, et la formule de `docs/03:103` n'a pas de valeur. Le cas est **atteignable**, et pas seulement en théorie :

- un pool d'un seul passage : la branche dense y a un candidat, et la branche lexicale aussi, si le passage contient un terme de la requête ;
- un seul candidat dans une branche parce que les autres sont hors pool ou coupés par `candidates_per_branch` ;
- des scores BM25 identiques, ce qui arrive sur des passages de même longueur portant les mêmes termes, et plus encore quand un seul terme de la requête est présent.

Une branche à un seul candidat est le cas le plus net : `max = min`, et la branche n'a qu'un score à normaliser. La laisser à zéro annulerait tout le signal qu'elle porte, alors que `docs/03:105` réserve le zéro à l'**absence**. Une valeur normalisée de 1 dit « présent, au mieux de ce que cette branche sait de cette requête », et c'est exactement ce que le zéro ne dit pas.

### Verdict de `@architecte` — décision 2, domaine de `alpha` : `NON-COUVERT`

Le code promet un domaine que rien ne vérifie. `crates/crible-core/src/config.rs:341-342` documente `alpha` comme « Weight given to the dense branch, between 0 and 1 », mais `FusionConfig::from_value` (`config.rs:360-363`) lit un `f64` quelconque et l'accepte tel quel : `NaN`, `±∞`, une valeur négative et une valeur supérieure à 1 passent tous. Trois conséquences, toutes silencieuses :

- `alpha = 5` **inverse** les poids : le dense pèse cinq fois plus que `1 − 5 = −4` pèse en négatif, et un document absent du dense remonte au-dessus d'un document présent dans les deux branches ;
- `NaN` rend un ordre arbitraire, non reproductible d'un tri à l'autre, et le classement n'est plus déterministe ;
- `±∞` produit des scores non finis, que la sérialisation JSON de la réponse ne sait pas rendre.

Le réglage se pose à trois endroits, et le domaine doit valoir aux trois : à la création de l'index (`docs/04:44-55`), au `PATCH` de `search.fusion` (`docs/04:48`, `docs/05-api.md:55` et `:72`), et par requête (`fusion.alpha`, `docs/05-api.md:163`), où `docs/05-api.md:31` prévoit déjà le refus « paramètre hors bornes » en `invalid_request` (400).

### Ce que cet ADR ne décide pas

Ni le choix de la fusion par défaut, ni la valeur par défaut d'`alpha` : Q-02 et MES-10 les tranchent, et `rrf` reste le défaut (`docs/03:106`, ADR-0008:18). Ni la constante 60 de `rrf`. Ni la façon dont la fusion ordonne deux scores fusionnés égaux : c'est la règle partagée de tri (score décroissant, puis clé `doc_id#ord` croissante) qui s'applique, déjà fixée par ADR-0045, ADR-0072 et ADR-0079.

## Décision

### 1. Une branche constante normalise chacun de ses candidats à 1 ; une branche vide ne contribue rien

Quand tous les candidats d'une branche ont le même score — y compris une branche à un seul candidat —, chacun reçoit la valeur normalisée **1**. La formule devient donc, pour une branche constante, une contribution pleine et identique pour tous ses candidats, et l'ordre entre eux vient du départage par clé.

Une branche qui n'a **aucun** candidat ne contribue rien : la normalisation n'a pas d'entrée, et le score fusionné est celui de l'autre branche, pondéré par son propre poids. Un document absent d'une branche continue de valoir 0 dans cette branche (`docs/03:105`).

Le motif est la distinction que `docs/03:105` établit : l'absence vaut 0. Une branche constante normalisée à 0 rendrait un document **présent** indiscernable d'un document **absent**, et le seul candidat d'une branche à un candidat recevrait le score d'un document que cette branche n'a jamais vu. La valeur 1 est la seule qui conserve l'information portée par la branche, et elle ne privilégie aucun candidat de la branche, puisque tous la reçoivent.

### 2. `alpha ∈ [0, 1]`, bornes comprises, valeur finie, refusé partout ailleurs en `invalid_request` (400)

`alpha` est un poids, et son domaine est l'intervalle fermé `[0, 1]`, bornes comprises, sur une valeur **finie**. Toute autre valeur — négative, supérieure à 1, `NaN`, `±∞` — est refusée en `invalid_request` (400), avec le message que le contrôle produit et qui nomme le chemin JSON fautif. Le refus vaut aux trois points d'entrée du réglage :

- à la **création de l'index**, où la configuration est validée en entier ;
- au **`PATCH` de `search.fusion`**, où la valeur reçue est validée comme celle de la création ;
- **par requête**, sur `fusion.alpha` : c'est le « paramètre hors bornes » de `docs/05-api.md:31`, et le message nomme `fusion.alpha`.

Un seul contrôle vit dans `crible-core`, il est appelé depuis `FusionConfig::from_value`, sur le modèle de `check_lexical_language` (`config.rs:383`), et il est réutilisé tel quel pour la requête : une seule règle, un seul message, donc un seul comportement à documenter et à tester.

Les bornes sont **fermées**, et les deux extrémités servent : α = 0 réduit la fusion au lexical seul et α = 1 au dense seul, ce qui donne à MES-10 deux points de comparaison exacts contre les branches isolées. MES-10 balaie par ailleurs α ∈ {0,3 ; 0,5 ; 0,75} (`docs/phases/P5-lexical-et-hybride.md:107`), toutes valeurs intérieures au domaine.

Cet ADR ne fixe **aucune valeur par défaut** pour `alpha` : `fusion` vaut `rrf` par défaut, et `alpha` n'est lu qu'avec `method: "relative_score"`, où il reste obligatoire, comme le code l'exige déjà (`config.rs:353-357` refuse `alpha` avec `rrf`).

## Conséquences

- **Positives** :
  - la fusion `relative_score` est définie pour tout pool non vide, y compris un pool d'un seul passage, un candidat unique et des scores identiques : plus de division par zéro ni de branche au comportement indéfini ;
  - « présent » et « absent » restent discernables, ce que `docs/03:105` exige et qu'une branche constante à 0 aurait effacé ;
  - `alpha` a un domaine écrit, vérifié aux trois points d'entrée, et un client ne peut plus inverser les poids ni rendre un classement arbitraire avec `NaN` ;
  - un seul contrôle dans `crible-core`, réutilisé par la validation de configuration et par la requête : un seul endroit à corriger si la règle évolue.
- **Négatives / coûts** :
  - `invalid_request` (400) s'élargit une fois de plus, pour un réglage de fusion dont la valeur était jusqu'ici libre ;
  - une configuration existante qui porterait un `alpha` hors domaine est refusée à la validation, alors qu'elle était acceptée : le cas est improbable, aucune valeur par défaut n'ayant jamais dépassé `[0, 1]`, mais il doit être dit ;
  - la branche constante reçoit 1, ce qui donne à un pool d'un seul passage un score fusionné de 1 pour ce passage : le score est correct mais ne se compare pas d'une requête à l'autre, ce que `docs/03:108` dit déjà de tout score ;
  - deux règles de plus à tester dans la fusion et dans la validation de configuration.
- **Tests exigés** :
  - une branche dont tous les scores sont égaux reçoit 1 pour chacun de ses candidats, et l'ordre entre eux vient de la clé ;
  - une branche à un seul candidat reçoit 1 pour ce candidat, sur un pool d'un seul passage ;
  - une branche vide ne contribue rien, et le score fusionné est celui de l'autre branche pondéré par son poids ;
  - un document présent dans une seule branche garde 0 dans l'autre ;
  - `alpha = 0`, `alpha = 1`, `alpha = 0,5` sont acceptés ; `alpha = -0,1`, `alpha = 1,1`, `NaN` et `±∞` sont refusés en `invalid_request`, à la création, au `PATCH` et par requête ;
  - le message du refus nomme `fusion.alpha`.
- **À surveiller** :
  - l'usage réel des bornes α = 0 et α = 1 : si MES-10 montre qu'ils ne servent pas, le domaine fermé reste juste, mais l'argument d'utilité s'affaiblit ;
  - la fréquence d'une branche constante en production : sur un pool large, elle signale surtout une requête à un seul terme très discriminant, ou un `candidates_per_branch` trop petit pour la branche lexicale ;
  - le comportement de la fusion quand les deux branches sont vides : aucune ne contribue, et la réponse doit rester vide sans erreur, ce qui reste un test de la tâche et non une règle nouvelle ;
  - la valeur par défaut d'`alpha`, qui restera ouverte tant que MES-10 et Q-02 ne sont pas tranchés.

- **Documents modifiés à l'acceptation** (appliqués avec cet ADR, le propriétaire l'ayant accepté par anticipation) :

| Document | Modification |
|---|---|
| `docs/03-recherche.md:102-105` | `relative_score` : règle de la branche constante (1 pour chaque candidat), branche vide sans contribution, domaine de `alpha` (ADR-0090) |
| `docs/04-donnees-et-ingestion.md:66-74` | section des domaines : ligne de `search.fusion.alpha`, `∈ [0, 1]` bornes comprises et valeur finie, refus en `invalid_request` (ADR-0090) |
| `docs/05-api.md:31` | `invalid_request` : `fusion.alpha` hors de `[0, 1]` ou non fini, à la création, au `PATCH` et par requête (ADR-0090) |
| `docs/05-api.md:163` | champ `fusion` de la requête : domaine de `alpha` et renvoi à ADR-0090 |
| `docs/phases/P5-lexical-et-hybride.md:82-91` | fiche `FUSIONNER-HYBRIDE` : livrable et critère sur la branche constante et sur le domaine de `alpha` |
| `docs/adr/README.md` | ligne d'index de cet ADR |
| `docs/08-invariants-et-tests.md` | **aucune modification nécessaire** : INV-05 ne change pas, et les deux règles sont des tests |
| `docs/suivi.md` | hors de cet ADR : la ligne d'écart du 2026-10-28 sur `FUSIONNER-HYBRIDE` cite ADR-0090, et l'orchestrateur la tient |

`docs/03:106` (défaut `rrf`), `docs/03:108` (`score_type`) et le corps d'ADR-0008 restent **inchangés**.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Branche constante normalisée à **0** | Annule le signal d'une branche à un seul candidat, et rend un document présent indiscernable d'un document absent, ce qui contredit `docs/03:105` : le zéro est réservé à l'absence |
| Branche constante normalisée à **0,5** | Aucune justification : la valeur ne correspond ni à « présent au mieux » ni à « absent », et introduit un troisième état que la conception ne connaît pas |
| Branche constante traitée comme une branche vide | Une branche constante a des candidats : les ignorer perdrait des résultats que le pool contient, ce qu'INV-05 interdit de fait |
| Rejeter la requête quand une branche est constante | Le cas est normal sur un petit pool : refuser une requête valide pour un artefact de normalisation serait faux, et un pool d'un seul passage est légitime |
| Laisser le cas indéfini et compter sur les tests de `@dev` | Un comportement qui n'existe que dans le code est perdu à la session suivante, et deux implémentations pourraient diverger sans qu'aucun texte ne tranche |
| Domaine `]0, 1[`, bornes ouvertes | α = 0 et α = 1 sont utiles : ils donnent à MES-10 les deux branches isolées comme points de comparaison exacts, et la formule de `docs/03:104` les accepte sans ambiguïté |
| Aucun domaine, contrôle absent | `alpha = 5` inverse les poids, `NaN` rend un ordre arbitraire, `±∞` produit des scores non finis : trois comportements silencieux et non reproductibles |
| Domaine vérifié seulement à la création de l'index | Le réglage se pose aussi au `PATCH` et par requête : un contrôle partiel laisserait passer exactement les valeurs qu'on veut refuser |
| Un contrôle séparé dans chaque couche (serveur, CLI, requête) | Trois règles à tenir au lieu d'une, et trois occasions de divergence ; `check_lexical_language` montre qu'un contrôle unique dans `crible-core` suffit |
| Un code d'erreur dédié pour `alpha` | `invalid_request` décrit déjà « paramètre hors bornes » (`docs/05:31`) ; une ligne de plus dans `docs/05` et dans les analytics pour une situation couverte |
| Ramener `alpha` dans `[0, 1]` en silence (bornage) | Une valeur corrigée sans le dire change le classement demandé par le client, et laisse croire que la valeur a été prise en compte |

## Références

- `docs/03-recherche.md:98-108` — fusion : pool commun, `rrf`, `relative_score`, défaut `rrf`, debug des branches, `score_type` ; `:102-105` — normalisation, formule, absence valant 0.
- `docs/04-donnees-et-ingestion.md:44-74` — réglages figés et réglables, domaines d'`ingest.*`, de `lexical.language`, et `exact_threshold` sans domaine.
- `docs/05-api.md:29-44` — codes d'erreur, `invalid_request` ; `:52-72` — endpoints et `PATCH` ; `:152-171` — champs de la requête, `fusion`.
- `docs/05-api.md:163` — `fusion` : `{method: "relative_score", alpha}`.
- `docs/phases/P5-lexical-et-hybride.md:78-91` — `FUSIONNER-HYBRIDE`, livrables et critères ; `:103-111` — `MESURER-HYBRIDE`, α ∈ {0,3 ; 0,5 ; 0,75}.
- `crates/crible-core/src/config.rs:334-369` — `FusionConfig`, `from_value`, refus d'`alpha` avec `rrf` ; `:371-391` — `LEXICAL_LANGUAGES` et `check_lexical_language`, modèle du contrôle unique.
- `docs/08-invariants-et-tests.md` — INV-05 (le pool est l'autorité).
- ADR-0008 (hybride sur le même pool, `rrf` ou `relative_score` ; corps non réécrit), ADR-0045 et ADR-0072 (ordre et calcul des scores), ADR-0079 (départage après tri), ADR-0054 (bornes des réglages d'`ingest.*`), ADR-0086 (domaine de `lexical.language`, et renvoi des autres bornes à un ADR).
