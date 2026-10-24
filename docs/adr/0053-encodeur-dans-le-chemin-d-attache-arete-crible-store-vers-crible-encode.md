# ADR-0053 — Encodeur dans le chemin d'attache : arête `crible-store` → `crible-encode`

- **Statut** : Accepté (décision du propriétaire le 2026-10-24)
- **Date** : 2026-10-24 (proposition et acceptation)
- **Décideurs** : propriétaire
- **Décision** : accepté le 2026-10-24 par le propriétaire, tel que proposé : arête interne `crible-store` → `crible-encode` autorisée, unidirectionnelle, consignée dans `docs/07`.
- **Proposé par** : agent greffier, sur demande du propriétaire. Origine : tâche `CONTROLER-LONGUEURS` (phase P2, fiche `docs/phases/P2-encodeur-et-recette.md`), relecture du tour 1, puis verdict de `@architecte` du 2026-10-24 : conception `CONFORME`, mais **`ECART` de processus**.
- **Remplace** : —

## Contexte

### Pourquoi cet ADR existe

Le verdict de `@architecte` du 2026-10-24 juge la conception de `CONTROLER-LONGUEURS` conforme, mais relève un **écart de processus** : `docs/adr/README.md` (« Quand écrire un ADR ») exige un ADR pour toute décision qui « ajoute, retire ou remplace une dépendance ». La tâche ajoute une dépendance **interne** entre deux crates du workspace, et aucun ADR ne couvre aujourd'hui les arêtes internes. Le présent ADR régularise ce point ; il ne rouvre pas la conception, qui est jugée conforme.

### Ce que fait le code aujourd'hui

Le contrôle 8 de `docs/04-donnees-et-ingestion.md` (longueur des passages, **INV-11**) est implémenté dans `crible-store::attach` :

- il s'exécute **après** `reader.validate()` et **avant** toute écriture, donc dans la phase où `docs/02-architecture.md` (« Générations et point de commit », étape 1 : « Valider et faire la recette. Aucun état n'est modifié à cette étape ») place les contrôles ;
- il reçoit un `crible-encode::TokenCounter` **fourni par l'appelant**, sous la forme `Option<&TokenCounter>` ; la valeur `None` correspond à un modèle `external`, qui n'a pas de tokenizer à interroger.

Conséquence de compilation : `crible-store` déclare une dépendance vers `crible-encode`. La chaîne de compilation du store tire donc `crible-encode`, et avec lui `ort`, `tokenizers` et `rayon`. Il n'y a **pas de cycle** : `crible-encode` ne dépend pas de `crible-store`.

### Ce que la conception dit, et ce qu'elle ne dit pas

- `docs/07-stack-et-deploiement.md` (arborescence des crates) donne des **rôles** : « attache » à `crible-store`, « chargement des modèles, pooling, prompts » à `crible-encode`. Il ne donne **aucun ordre de couches** entre les crates.
- Aucun ADR ne couvre les arêtes internes du workspace. Sans décision écrite, la première arête posée par commodité devient un précédent tacite.
- `docs/02-architecture.md` (étape 1 de l'attache) et `docs/04-donnees-et-ingestion.md` (tableau « qui fait quoi » : Crible « vérifie les longueurs et fait la recette ») placent explicitement ces contrôles dans l'attache, côté Crible.

### Ce que les tâches suivantes de P2 apportent

Le contrôle 8 n'est pas isolé. La même phase prévoit deux autres travaux qui demandent le modèle pendant ou juste après l'attache :

- `RECETTE-IMPORT` : contrôle 9 de `docs/04` (recette fp32, **INV-12**), dans la même phase de validation ;
- `ENCODER-DOCUMENTS-EN-LIGNE` : encodage fp32 de documents sans vecteurs dans `crible-store::online`, qui « suit le même chemin d'attache ».

Une décision prise pour le seul contrôle 8 serait donc rouverte deux fois dans la même phase.

## Décision

**Proposition soumise au propriétaire : accepter l'arête interne `crible-store` → `crible-encode` (option a).**

Précisions, si l'arête est acceptée :

1. `crible-store::attach` reste le lieu des contrôles qui demandent le modèle : contrôle 8 (longueurs, `CONTROLER-LONGUEURS`), contrôle 9 (recette fp32, `RECETTE-IMPORT`), et l'encodage en ligne de documents (`crible-store::online`, `ENCODER-DOCUMENTS-EN-LIGNE`).
2. Le **modèle est chargé et fourni par l'appelant** (CLI, serveur, tests) ; l'attache ne charge aucun modèle et ne décide pas de sa configuration. `crible-store` dépend des **types** de `crible-encode` (compteur de tokens, encodeur), pas de la politique de chargement.
3. L'arête est **unidirectionnelle** : `crible-encode` ne dépend jamais de `crible-store`. Un cycle serait un défaut de conception, pas une variante acceptable.
4. L'arête est **consignée** dans `docs/07` à l'acceptation (voir « Conséquences »). Une arête non écrite redevient un précédent tacite.

Ce que cet ADR ne décide pas :

- il ne fixe pas d'ordre de couches général pour le workspace ; il autorise **cette** arête, pour cette raison ;
- il ne change ni le format d'échange, ni le protocole d'attache, ni l'API ;
- il n'ajoute **aucune dépendance externe** : `ort`, `tokenizers` et `rayon` sont déjà listés dans `docs/07`. Seule la chaîne de compilation du store change.

## Conséquences

- **Positives** :
  - les contrôles restent là où la conception les place (`docs/02` étape 1, `docs/04` « qui fait quoi »), et l'ordre **validation → refus → écriture** reste tenu par un seul module, `attach` ;
  - le lot n'est lu **qu'une fois**, en flux, conformément à ADR-0039 : un contrôle déplacé chez l'appelant imposerait une seconde lecture ;
  - `RECETTE-IMPORT` et `ENCODER-DOCUMENTS-EN-LIGNE` n'auront pas à rouvrir la question : l'arête est déjà autorisée et documentée ;
  - aucun cycle, aucune dépendance externe nouvelle.
- **Négatives / coûts** :
  - la chaîne de compilation de `crible-store` tire `ort`, `tokenizers` et `rayon` : temps de compilation et surface de dépendances du store augmentent, y compris pour les tests qui ne touchent qu'au stockage ;
  - la frontière entre « stockage » et « encodage » se brouille : un lecteur de `docs/07` ne peut plus déduire des rôles seuls quels crates dépendent de l'encodeur ;
  - une ligne de `docs/07` est à ajouter à l'acceptation (arête interne autorisée et sa raison), sans quoi la décision n'est pas visible là où les rôles sont décrits.
- **État des tâches** :
  - `CONTROLER-LONGUEURS` reste **`BLOQUE`** jusqu'à la décision. L'ADR n'attend **aucune autre modification de code** : le code décrit ci-dessus est celui déjà relu ;
  - `RECETTE-IMPORT` et `ENCODER-DOCUMENTS-EN-LIGNE` s'appuieront sur cette décision ;
  - `docs/suivi.md`, section « Écarts à la conception », doit citer ADR-0053 ; cette ligne n'est pas ajoutée par cet ADR et reste à la charge de l'orchestrateur.
- **À surveiller** :
  - l'apparition d'une dépendance inverse, même partielle (`crible-encode` → `crible-store`) : signal d'un cycle à trancher par un nouvel ADR ;
  - l'allongement du temps de compilation du workspace, en particulier en CI ;
  - la multiplication des arêtes internes non documentées : chaque nouvelle arête qui engage la conception demande son ADR ;
  - l'usage de `crible-store` par un binaire qui n'a pas besoin de l'encodeur : si la dépendance devient gênante (taille, temps de build), la réponse est un découpage de crate, pas un contournement du contrôle.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| **(b)** Contrôle fait par l'appelant avant `attach` | Pas d'arête, mais le lot est lu deux fois : une fois pour compter les tokens, une fois pour l'attache en flux (ADR-0039). Chaque appelant (CLI, serveur en P6, tests) devrait y penser, et un oubli produirait exactement la troncature silencieuse qu'INV-11 interdit. Ne survit pas à `RECETTE-IMPORT`, qui doit ré-encoder un échantillon au même endroit du protocole. |
| **(c)** `attach` prend une fonction de comptage (`&dyn Fn(&str) -> Result<usize> + Sync`) | Pas d'arête pour le seul contrôle 8, mais la recette fp32 et l'encodage en ligne la ramènent : on paierait l'abstraction aujourd'hui pour la retirer demain. La vérification du hash du tokenizer resterait chez l'appelant, donc la garantie serait partielle. C'est aussi une abstraction pour une seule implémentation, ce que la règle 12 d'`AGENTS.md` proscrit. |

Aucune autre option n'a été examinée : la conception de `CONTROLER-LONGUEURS` est jugée `CONFORME` par `@architecte`, et le débat porte sur le seul point de processus.

## Références

- Fiche : `docs/phases/P2-encodeur-et-recette.md`, `### CONTROLER-LONGUEURS`, `### RECETTE-IMPORT`, `### ENCODER-DOCUMENTS-EN-LIGNE`.
- Conception : `docs/02-architecture.md` (« Générations et point de commit », étape 1), `docs/04-donnees-et-ingestion.md` (contrôles 8 et 9, tableau « qui fait quoi »), `docs/07-stack-et-deploiement.md` (arborescence des crates, dépendances Rust), `docs/08-invariants-et-tests.md` (INV-11, INV-12).
- ADR liés : ADR-0039 (ordre des lignes dans un lot v1, lecture en flux), ADR-0041 (sémantique d'attache), ADR-0016 (point de commit SQLite), ADR-0028 (empreinte du modèle), ADR-0051 et ADR-0052 (encodeur et recette).
- Processus : `docs/adr/README.md` (« Quand écrire un ADR », cycle de vie, section « Écarts ») ; `AGENTS.md` (règle 12, simplicité).
