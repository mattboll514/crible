# ADR-0143 — Politique de version de la ligne 1.x et procédure de montée et de retour arrière en production

- **Statut** : Accepté (décidé par le propriétaire le 2026-11-11, « L'adr 143 est accepté »)
- **Date** : 2026-11-11
- **Décideurs** : propriétaire
- **Proposé par** : agent `@greffier`, à la demande du propriétaire du 2026-11-11 (« oui volontiers, tu peux rédiger l'ADR »), hors tâche, sur l'audit en lecture seule de `@architecte` du 2026-11-11 consigné dans `docs/suivi.md:618-638`
- **Remplace** : — (ne remplace aucun ADR ; ne modifie le corps d'aucun ADR accepté)

## Contexte

### Où en est le projet

La v1.0.0 est livrée et publiée : J10 est validé le 2026-11-08, le tag `v1.0.0` est posé, et l'image `1.0.0` est publiée sous `ghcr.io/<owner>/crible:1.0.0` et `:cpu` (`docs/suivi.md:22`, `:1527`). Elle sert **six index en production** — `dole`, `sarde`, `titres`, `jorf`, `articles`, `decisions` —, soit 6 660 239 documents et 9 600 105 passages (ADR-0142, contexte).

Une **1.1.0** s'ouvre, faite de correctifs tirés des tickets du projet Legifacile, sur la branche `dev` (`docs/suivi.md:23`, `:586-638`). Quinze tickets sont ouverts le 2026-11-11 (`T01` à `T15`) et d'autres arriveront (`docs/suivi.md:592`). L'audit de `@architecte` du 2026-11-11 a classé ces tickets et relevé ce que rien ne tranche (`docs/suivi.md:618-638`). Le propriétaire a demandé cet ADR avant le premier correctif de contrat (`docs/suivi.md:632`).

### Ce que la conception ne dit pas

**Aucune règle ne dit ce qu'une version mineure peut changer.** ADR-0133 ne règle que la **publication** : le tag vaut `v` suivi de `[workspace.package] version` de `Cargo.toml` (décision 2), le workflow est déclenché par un tag et par lui seul (décision 1), et le tag est un geste du propriétaire (décision 7). Rien n'y dit ce qu'une `1.x` peut ajouter, retirer ou corriger. Le suivi le dit à sa manière : « aucune règle ne dit ce qu'une version mineure peut changer » (`docs/suivi.md:619`).

**Tous les formats persistants sont versionnés, et aucun ne migre.** L'INV-15 exige que tout format sur disque soit little-endian explicite et versionné (`docs/08-invariants-et-tests.md:23`). Un `format_version` différent est refusé à l'ouverture : `check_format_version` compare à l'**égalité stricte** et rend `invalid_request` (« unsupported … format_version N; this build reads format_version M », `crates/crible-store/src/sqlite.rs:135-141`). Il n'existe **aucune migration**. Seuls les fichiers **dérivés** se reconstruisent, à l'ouverture en écriture, depuis `records.sqlite` et les `vectors.*` (`docs/02-architecture.md:87`) ; pour le lexical, la reconstruction est le comportement normal d'un état indécidable (`docs/02-architecture.md:157`, ADR-0106). `index.json` fait exception dans l'autre sens : il **n'a pas de `format_version`** (`docs/02-architecture.md:69`).

**L'empreinte de la chaîne d'analyse est partielle.** ADR-0106 enregistre `lexical.language` et `lexical.positions` dans la charge utile du commit tantivy (décision 1, `docs/adr/0106-empreinte-de-la-chaine-d-analyse-lexicale.md:25`). Un changement de **tokenizer sous la même langue** n'est donc pas détecté : l'option « une empreinte des fichiers du tokenizer » a été écartée à la rédaction (`docs/adr/0106-empreinte-de-la-chaine-d-analyse-lexicale.md:79`). C'est exactement la piste de T14 (`docs/suivi.md:175`).

**La configuration d'instance refuse une clé inconnue** (ADR-0097) : une `crible.toml` 1.0 se charge en 1.1, mais une `crible.toml` 1.1 qui porte une clé nouvelle ne se charge pas en 1.0. Pour `index.json`, le **code** refuse aussi une clé inconnue : `IndexConfig::from_value` compare les clés à une liste fermée (`crates/crible-core/src/config.rs:633-656`) et `json::reject_unknown_keys` rend `invalid_request` (`crates/crible-core/src/json.rs:141-152`). Ce que fait une **instance 1.0** qui ouvre un index dont l'`index.json` porte une clé 1.1 — refus de l'index, refus de démarrer, autre — n'est écrit nulle part : **à vérifier**, et à écrire dans la note de version.

**Le client Python n'est pas lié à la version du serveur.** Il rend le JSON en `dict`, sans type déclaré (ADR-0112, `docs/adr/0112-client-python-http.md:63`), et un code d'erreur inconnu devient la base `CribleError`, `code` conservé tel quel (`:78`). Un membre de réponse ajouté ne casse donc aucun client ancien. Les règles qui vont avec : une clé de `details` se documente dans `docs/05` **dans le même changement** (ADR-0101, décision 3, `docs/adr/0101-corps-d-erreur-message-details-et-reponses-hors-route.md:33`), et une route s'ajoute au tableau des routes de `docs/05`, qui est la référence (`docs/05-api.md:58-78`, ADR-0111, décision 5, `docs/adr/0111-specification-openapi-et-validation-des-exemples.md:106`).

**Aucune procédure de montée ni de retour arrière n'est écrite.** Le mécanisme du retour arrière existe : une restauration d'instantané vers un index cible à une génération **supérieure** est acceptée, « c'est le retour arrière voulu » (`docs/02-architecture.md:218`, `docs/04-donnees-et-ingestion.md:360`, ADR-0065, décision 3). Mais aucun texte n'exige d'instantané **avant** une montée (`docs/suivi.md:623`). Et en production, les agents sont en **lecture seule** et ne redémarrent aucun service (ADR-0032, `docs/adr/0032-acces-a-la-production.md:23-26`) : la montée est un geste du propriétaire.

**La CI distante ne voit pas `dev`.** `ci.yml` tourne sur tout `push` et toute `pull_request`, sans filtre de branche (`.github/workflows/ci.yml:3-5`). `dev` n'est pas poussée : `docs/suivi.md:604` dit que rien n'est poussé sans confirmation du propriétaire, `dev` comprise, et le point 4 de `docs/suivi.md:638` demande de la pousser après chaque ticket fusionné.

### Ce qui est vérifié, et ce qui reste à vérifier

Vérifié le 2026-11-11 dans l'arbre : l'égalité stricte de `format_version` ; la liste fermée des clés d'`index.json` ; l'absence de `format_version` dans `index.json` ; les références d'ADR-0101, ADR-0106, ADR-0111, ADR-0112, ADR-0133 et ADR-0065 citées ci-dessus ; les déclencheurs de `ci.yml`.

À vérifier avant d'appliquer cet ADR : le comportement exact d'une instance 1.0 devant un `index.json` portant une clé qu'elle ne connaît pas ; et le moyen de remettre en place un `index.json` d'avant montée (voir décision 2).

## Décision

**La ligne 1.x suit une version sémantique stricte : une mineure n'ajoute que des choses dont l'absence garde le comportement précédent, un correctif ne change aucun contrat, et toute rupture attend une `2.0.0`. Aucun `format_version` ne change dans la ligne 1.x sans un ADR de migration. Chaque montée de production s'appuie sur un instantané des six index et sur une note de version.**

### 1. Version sémantique pour le serveur, le workspace et le client Python

Les trois portent la même règle : `[workspace.package] version` de `Cargo.toml`, la version de `clients/python/pyproject.toml`, et le tag d'ADR-0133 (décisions 2 et 7, inchangées).

- Un **correctif** (`1.x.y`, `y > 0`) ne change **aucun contrat** : ni route, ni champ, ni membre, ni clé, ni code d'erreur, ni valeur par défaut, ni format.
- Une **mineure** (`1.x.0`) n'apporte que des **ajouts compatibles**.
- Toute **rupture** attend une `2.0.0`.

**Permis dans une mineure** :

- une **route nouvelle** (déclarée dans le tableau de `docs/05`, ADR-0111) ;
- un **champ de requête optionnel** dont l'absence garde le comportement précédent ;
- un **membre de réponse nouveau** ;
- une **clé de `details` nouvelle** (documentée dans `docs/05` dans le même changement, ADR-0101) ;
- une **clé de configuration optionnelle** — `crible.toml` ou `index.json` — dont le défaut est le comportement précédent ;
- la **correction d'un comportement non documenté qui contredit la conception**, à condition qu'un ADR dise pourquoi c'est une correction et non un changement de contrat.

**Interdit dans une mineure** : retirer ou renommer une route, un champ, un membre, une clé ou un code d'erreur ; changer le sens d'une valeur ; changer le statut HTTP ou le code d'un cas **documenté** ; modifier un `format_version` ; rendre une clé obligatoire.

**Conséquences pour les tickets de la v1.1** :

- **T03** (`docs/suivi.md:164`) : la piste d'un `409` dédié remplacerait le `400` d'un cas **documenté** — `POST /v1/indexes` rend un `400 invalid_request` avec le message `index already exists`, et « il n'y a pas de code dédié » (`docs/05-api.md:90`). C'est **interdit en 1.1**. Sa variante `job_id` dans le refus actuel reste possible : c'est une clé de `details` nouvelle sur un refus existant.
- **T06** (`docs/suivi.md:167`) : remplacer `200 {"indexes":[]}` par `503` pendant le démarrage n'est pas un cas documenté de `docs/05` — le tableau ne dit rien du comportement pendant la reprise (`docs/05-api.md:61`) —, et le comportement actuel contredit l'esprit d'ADR-0099 (`/ready` vaut `503` « jusqu'à la publication du dernier instantané », `docs/05-api.md:77`). C'est donc une **correction de comportement**, autorisée, et elle demande **son ADR**, qui dira pourquoi c'est une correction.

### 2. Formats persistants : rien ne change dans la ligne 1.x sans ADR de migration

Aucun `format_version` ne change dans la ligne 1.x, sauf par un **ADR de migration** qui fournit la migration **et** le retour arrière. Les fichiers dérivés peuvent se reconstruire (`docs/02-architecture.md:87`, ADR-0106 pour le lexical).

Un réglage nouveau d'`index.json` est **optionnel**, et son absence vaut le comportement 1.0. Deux conséquences en découlent :

- un index qui **utilise** un réglage nouveau peut ne plus s'ouvrir avec une 1.x antérieure ; la note de version doit le dire, et le retour arrière passe alors par l'instantané ;
- `index.json` n'a pas de `format_version` (`docs/02-architecture.md:69`) : c'est ce qui fait d'une clé nouvelle un changement de format de fait, et c'est pourquoi elle reste **optionnelle et sans effet par défaut**.

**Précision vérifiée, à écrire dans la note de version.** La forme stockée est **matérialisée** : `IndexConfig::to_value` écrit **chaque** réglage explicitement, `search.exact_threshold` comprise (ADR-0074), et c'est cette forme que le job de `PATCH` réécrit dans `index.json` (`crates/crible-core/src/config.rs:685-692`, `crates/crible-server/src/indexes.rs:320-333`, ADR-0105). Donc, sous 1.1, **le premier `PATCH` d'un index matérialise la clé nouvelle même si personne ne l'a réglée** : l'`index.json` diverge alors de tout instantané pris avant la montée, et une 1.0 ne le relit plus. Un index que l'on veut pouvoir ramener en 1.0 ne reçoit aucun `PATCH` sous 1.1, ou bien sa note de version dit que le retour arrière repasse par l'instantané.

**Précision vérifiée, à trancher.** La restauration exige que l'`index.json` de la cible soit **identique** à celui de l'instantané (ADR-0065, décision 3 ; `docs/02-architecture.md:218`). Un retour arrière vers un instantané d'avant montée suppose donc de remettre en place l'`index.json` d'avant. Or **aucune commande documentée ne réécrit l'`index.json` d'un index existant** : `crible index create` refuse un index présent, et le seul écrivain est le job de `PATCH`. Le chemin éprouvé est celui d'un index **absent**, créé depuis l'`index.json` de l'instantané (`docs/02-architecture.md:218`, `docs/04-donnees-et-ingestion.md:360`). Le moyen retenu — remise en place du fichier serveur arrêté, ou index retiré puis restauré — est **à vérifier et à écrire** dans la procédure de `docker/README.md` ; cet ADR ne le tranche pas.

### 3. Analyse lexicale : tout changement d'analyse change l'empreinte

Tout changement de l'analyse d'un jeton — tokenizer, filtres, normalisation — doit **changer l'empreinte comparée à l'ouverture**, en complément d'ADR-0106. La reconstruction est alors automatique, et un retour arrière ne sert jamais un index construit avec une autre analyse. Le mécanisme — par exemple un réglage d'`index.json` qui entre dans l'empreinte — sera fixé par l'**ADR du ticket T14** (`docs/suivi.md:175`), avec sa mesure ; cet ADR ne choisit pas la piste.

Raison : sans cela, un index construit en 1.0 serait servi avec l'analyse 1.1 sans reconstruction, ou l'inverse après un retour arrière, avec des résultats faux et **aucune erreur** — le cas que l'audit signale (`docs/suivi.md:620`).

### 4. Note de version versionnée pour chaque mineure

Chaque `1.x.0` a une **note de version** versionnée, écrite par la tâche de publication, dans **`docs/versions/<version>.md`** (par exemple `docs/versions/1.1.0.md`). C'est un document de `docs/`, donc en français, et il cite les ADR et les mesures. `docs/` ne contient aujourd'hui aucun journal de versions : ce dossier est créé par la première note.

La note liste :

1. les **ajouts de contrat** — routes, champs, membres, clés de `details`, clés de configuration ;
2. les **corrections de comportement**, chacune avec son ADR ;
3. les **reconstructions au premier démarrage**, avec l'index et la **durée attendue** : ADR-0142 donne les durées mesurées de la reconstruction lexicale — `dole` 1 s, `sarde` 5 s, `titres` 13 s, `jorf` 71 s, `articles` 127 s, `decisions` 185 s (`docs/adr/0142-limites-acceptees-de-la-v1-reconstruction-lexicale-et-memoire-d-un-replace-all.md:21-26`, `docs/mesures/import-legifacile-2026-11-06.md:357-365`) ;
4. les **clés de configuration ajoutées**, leur défaut et ce que ce défaut vaut ;
5. les **conditions du retour arrière**, dont la présence ou l'absence de clé nouvelle dans `index.json`.

### 5. Procédure de montée en production, faite par le propriétaire

En production, les agents restent en lecture seule et ne redémarrent rien (ADR-0032). La montée est un geste du propriétaire.

1. **Lire la note de version** (`docs/versions/<version>.md`).
2. **Prendre un instantané des six index**, serveur 1.0 encore en marche : `POST /v1/indexes/{index}/snapshots` (`docs/05-api.md:74`), qui suspend l'écrivain et laisse les lectures continuer (ADR-0017, ADR-0065). `crible snapshot` convient aussi, mais serveur arrêté (ADR-0065, décision 1). Garder aussi **la configuration** (`crible.toml`) et **l'image en place** (sa version, ou son empreinte `sha256`).
3. **Arrêter le serveur, épingler la nouvelle image par sa version ou son empreinte, jamais par `:cpu`** — `:cpu` est une étiquette mobile (ADR-0133, décision 4) —, puis redémarrer. Le conteneur se recrée avec la même configuration et les mêmes volumes (`docker/README.md`, section « Lancer »).
4. **Attendre `/ready`** : il vaut `503` jusqu'à la publication du dernier instantané de chaque index (ADR-0099, `docs/05-api.md:77`).
5. **Contrôler** :
   - `crible --version` rend la version attendue ;
   - les **comptes de chaque index** — documents, passages vivants, génération, segments — sont **égaux** à ceux d'avant la montée ;
   - `crible eval` sur le **jeu de référence** de `docs/legifacile/reference/` rend des **rappels au moins égaux** à ceux de la 1.0, écarts expliqués s'il y en a. La commande documentée est `crible eval --data <dossier> --index <index> --queries docs/legifacile/reference/<index>.jsonl --models <dir> --out <dossier>` (`docs/legifacile/jeu-de-reference.md:23`) ; elle lit un index servi sans écrire (ADR-0116, décision 2), et le mode API existe aussi (ADR-0131). Une baisse est un défaut, pas un réglage.
6. **Faire passer Legifacile ensuite** : la montée de son image épinglée et de son client Python revient au projet Legifacile (`docs/suivi.md:611`).

### 6. Retour arrière

Image précédente, plus la **restauration des instantanés** et de la **configuration d'avant** si la note de version le demande (voir les deux précisions de la décision 2). Ensuite, `/ready` et les **mêmes contrôles** qu'à la montée.

`crible restore` est **hors ligne** : le serveur est arrêté (ADR-0065, décision 1). La restauration installe l'instantané comme nouvelle époque en **laissant la génération à G** (décision 2) : elle **remplace** donc tout ce qui a été commité après l'instantané. Les **lots attachés après la montée sont à rattacher**. L'époque remplacée n'est pas supprimée (décision 4) : rien n'est détruit.

### 7. Classe de compatibilité de chaque tâche

Chaque tâche de la v1.1 porte **dans le suivi** sa classe de compatibilité, et l'ADR de chaque ticket de contrat la **déclare** :

- **sans risque** : aucun contrat, aucun format ;
- **ajout compatible** : une des catégories permises de la décision 1 ;
- **correction de comportement** : un comportement non documenté qui contredit la conception, avec son ADR ;
- **reconstruction ou migration** : l'analyse lexicale, les fichiers dérivés, ou un `format_version`.

`docs/suivi.md:625-631` classe déjà les quinze tickets ; la décision 1 les confirme, sauf **T03**, dont la piste `409` est écartée pour la 1.1.

### 8. CI distante : `dev` est poussée après chaque ticket fusionné

`dev` est poussée **après chaque ticket fusionné**, avec la **confirmation du propriétaire dans la conversation**, pour que `ci.yml` la teste avant la publication (`.github/workflows/ci.yml:3-5`, `docs/suivi.md:638`). La règle de git ne change pas : rien n'est poussé sans confirmation, `main` ne bouge pas jusqu'à la publication (`docs/suivi.md:604`). La CI distante a déjà attrapé ce que la vérification locale ratait — liens de rustdoc, courses de concurrence (`docs/suivi.md:638`).

### Ce qui n'est pas décidé ici

- Le numéro de version à publier, la date de publication et le tag : ils restent au propriétaire (ADR-0133, décisions 2 et 7).
- Le contenu de chaque ticket, la piste retenue et son ADR : chaque ticket garde son ADR.
- Le mécanisme exact de l'empreinte d'analyse (T14), la forme d'un éventuel `format_version` futur et sa migration.
- Le moyen de remettre en place un `index.json` d'avant montée (décision 2, « à trancher »).
- Le mode d'exploitation de la production (hôte, chemins, secrets) : hors git (ADR-0032).

### Ce que l'acceptation appliquera

- `docker/README.md` : une section « Monter de version » et une section « Revenir en arrière », reprenant les décisions 5 et 6, avec les commandes du projet et le renvoi à la note de version.
- `docs/07-stack-et-deploiement.md:119-149` (section « Docker ») : un paragraphe qui renvoie aux deux sections ci-dessus et à cet ADR.
- `docs/02-architecture.md` et `docs/04-donnees-et-ingestion.md` : une phrase disant qu'aucun `format_version` ne change dans la ligne 1.x sans ADR de migration (décision 2), si le propriétaire le demande.
- `docs/05-api.md` : rien. La politique de version ne change aucune route.
- `docs/suivi.md:625-631` : la classe de chaque ligne `v1.1`, par l'orchestrateur (décision 7). `@greffier` n'y touche pas.
- `docs/versions/1.1.0.md` : écrit par la tâche `PUBLIER-V1.1.0` (décision 4), pas par cet ADR.

Aucun de ces documents n'est modifié avant l'acceptation.

## Conséquences

- **Positives** :
  - une mineure a une définition écrite : un ticket sait, avant d'être chiffré, s'il tient dans la 1.1 ;
  - les deux pièges de la v1.1 sont nommés d'avance : le `409` de T03 est écarté sans essai inutile, et T06 sait qu'il lui faut son ADR ;
  - un `format_version` ne peut plus changer par inadvertance : la règle est unique et l'ADR de migration est la seule porte ;
  - l'empreinte d'analyse devient complète pour tout changement futur, ce qui ferme le trou par lequel un index faux serait servi sans erreur ;
  - la montée a une liste de gestes et une liste de contrôles : `--version`, les comptes, et `crible eval` sur le jeu de référence versionné ;
  - le retour arrière a un mécanisme éprouvé — la restauration vers une génération supérieure — et il est désormais exigé d'avoir un instantané avant de monter ;
  - la classe de compatibilité rend le suivi lisible d'un coup d'œil : on voit ce qui peut casser un client.
- **Négatives / coûts** :
  - une mineure ne peut plus corriger un `400` documenté, même quand le `400` est un mauvais choix de conception : le correctif attend une `2.0.0`, ou un ADR qui assume la rupture ;
  - la note de version est un document de plus à écrire à chaque mineure, et `docs/versions/` est un dossier nouveau ;
  - un instantané des six index avant chaque montée demande de l'espace disque sur le volume `/exchange`, et le temps de copie des six index ;
  - la vérification de `crible eval` demande le jeu de référence, les modèles et le dossier de données : une demi-heure de plus qu'un simple `--version` ;
  - un index qui a reçu un `PATCH` sous 1.1 ne revient plus en 1.0 sans passer par l'instantané, même si la clé nouvelle n'a jamais été réglée ;
  - pousser `dev` à chaque ticket fusionné ajoute un run de CI par ticket, et le workflow rejoue tout le job Rust.
- **À surveiller** :
  - le premier `index.json` d'un index de production qui porte une clé 1.1 : c'est le signal que le retour arrière par simple changement d'image n'est plus possible ;
  - une mineure qui, à l'usage, s'avère avoir changé un contrat sans le dire : le symptôme serait un client qui casse entre deux mineures, et la réponse est un ADR, jamais une correction silencieuse ;
  - `crible eval` après montée : une baisse de rappel sur un index signale soit une régression du code, soit une reconstruction qui n'a pas eu lieu — les deux se distinguent par la génération et par les journaux de démarrage ;
  - la durée du premier démarrage après une montée qui reconstruit le lexical : la note de version donne la durée attendue par index, et un dépassement net signale un changement d'analyse non prévu ;
  - un `format_version` modifié dans une branche de ticket : le refus à l'ouverture est immédiat, mais une revue qui ne le voit pas livrerait une 1.1 incapable d'ouvrir les données de la 1.0 ;
  - l'espace de `/exchange` occupé par les instantanés : la rétention d'ADR-0065 n'est pas modifiée ici, et six instantanés de six index ne tiennent pas forcément côte à côte.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| **Pas de politique de version : chaque ticket décide au cas par cas** | C'est l'état actuel, et il a fallu un audit pour découvrir que T03 ne tenait pas dans une mineure. Une règle écrite une fois évite de la redécouvrir à chaque ticket. |
| **Autoriser les ruptures dans une mineure, en le documentant** | Un client 1.0 casserait sans changer de version : ni le tag, ni `Cargo.toml`, ni le client Python ne le diraient. La version sémantique n'a de sens que si la mineure est sûre. |
| **Autoriser un `format_version` nouveau dans une mineure, avec refus à l'ouverture** | Le refus est net, mais il rend une 1.0 incapable d'ouvrir les données écrites par sa propre mineure : c'est une rupture de fait, et elle coûte un retour arrière complet. Un ADR de migration, avec sa migration et son retour arrière, est le seul cadre acceptable. |
| **Ne rien imposer sur l'empreinte d'analyse, et compter sur la revue** | Le trou est silencieux : le lexical d'un index construit en 1.0 serait servi avec l'analyse 1.1, avec des résultats faux et sans erreur. Une revue ne le voit pas, l'empreinte le voit. |
| **Note de version seulement dans les notes de publication GitHub** | Le dépôt est la mémoire du projet : une note hors du dépôt n'est liée ni à un tag, ni à un ADR, et ne se relit pas à côté des documents de conception. `docs/versions/` est versionné et cité par la procédure. |
| **Montée sans instantané, en comptant sur la CI et sur `crible eval`** | `crible eval` détecte une régression de qualité, pas une perte de données ni un format illisible. L'instantané est le seul retour arrière qui redonne la génération et des résultats identiques. |
| **Montée par `:cpu`** | `:cpu` est une étiquette mobile : une montée par `:cpu` suivrait une version qui n'a pas été choisie, et le retour arrière n'aurait pas de référence stable à épingler. La version ou l'empreinte sont explicites. |
| **Retour arrière par redémarrage de l'image précédente, sans restauration** | L'image précédente ne suffit pas quand `index.json` ou un `format_version` a bougé, ni quand la montée a attaché des lots avec un défaut corrigé. La note de version dit quand la restauration est nécessaire ; la procédure la garde. |
| **Pousser `dev` en continu, sans confirmation** | La règle de git du projet veut que rien ne soit poussé sans confirmation du propriétaire. La confirmation par ticket garde la règle et fait tester la CI avant la publication. |

## Références

- Suivi : `docs/suivi.md:23` (jalon `v1.1.0`), `:586-638` (section « Version 1.1.0 — tickets de Legifacile », dont `:618-631` l'audit et `:632-638` les décisions attendues), `:604` (branche `dev`), `:161` (`ACTUALISER-DEPOT-APRES-V1`, version 1.0.0), `:22` et `:1527` (J10 validé, tag `v1.0.0`, image `1.0.0` publiée).
- Tickets : `docs/suivi.md:162-175` (`T01` à `T15`), en particulier `:164` (T03), `:167` (T06) et `:175` (T14).
- Conception : `docs/02-architecture.md:69` (`index.json` sans version), `:87` (fichiers dérivés), `:157` (chaîne d'analyse et reconstruction), `:218` (restauration, `index.json` identique, génération supérieure) ; `docs/04-donnees-et-ingestion.md:49` (réglages modifiables), `:323` et `:345` (contenu d'un instantané), `:360` (contrôle de la cible) ; `docs/05-api.md:58-78` (tableau des routes), `:74` (route d'instantané), `:77` (`/ready`), `:90` (« index already exists », pas de code dédié) ; `docs/06-observabilite.md:165` (jeu de référence) ; `docs/07-stack-et-deploiement.md:119-149` (Docker), `:151-162` (CI) ; `docs/08-invariants-et-tests.md:23` (INV-15).
- Jeu de référence : `docs/legifacile/reference/` (six fichiers `<index>.jsonl` et `mcp.jsonl`), `docs/legifacile/jeu-de-reference.md:23` (commande `crible eval`).
- ADR : ADR-0017 (compaction et écrivain suspendu pendant une copie), ADR-0065 (instantané, restauration, retour arrière), ADR-0074 (`search.exact_threshold` optionnelle et matérialisée), ADR-0097 (clé inconnue dans `crible.toml`), ADR-0099 (`/health`, `/ready`), ADR-0101 (corps d'erreur, `details`), ADR-0105 (administration des index, job de `PATCH`), ADR-0106 (empreinte de l'analyse lexicale), ADR-0111 (référence des routes), ADR-0112 (client Python), ADR-0116 (lecture d'un index servi), ADR-0125 (`crible eval`, `--baseline`), ADR-0131 (mode API d'`eval`), ADR-0133 (publication de l'image), ADR-0142 (durées de reconstruction lexicale, règle d'exploitation).
- Code : `crates/crible-store/src/sqlite.rs:135-141` (égalité stricte de `format_version`), `crates/crible-core/src/config.rs:633-656` et `crates/crible-core/src/json.rs:141-152` (liste fermée des clés), `crates/crible-core/src/config.rs:685-692` (forme matérialisée), `crates/crible-server/src/indexes.rs:320-333` (le job de `PATCH` réécrit `index.json`), `crates/crible/src/main.rs:24-30` (`crible --version`).
- Workflow : `.github/workflows/ci.yml:3-5` (`push` et `pull_request`, sans filtre de branche), `.github/workflows/publish-image.yml` (garde de version, étiquettes, `:cpu`).
- Application : `docker/README.md` (sections « Monter de version » et « Revenir en arrière », après acceptation).
