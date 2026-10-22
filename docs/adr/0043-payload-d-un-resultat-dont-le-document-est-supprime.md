# ADR-0043 — Payload d'un résultat dont le document a été supprimé après la génération du snapshot

- **Statut** : Proposé
- **Date** : 2026-10-22
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, sur demande de l'orchestrateur, au 2e tour de relecture de la tâche RECHERCHER-DENSE-EXACT (phase P1), point 1 de `@relecteur`, après le verdict **ECART + NON-COUVERT** de `@architecte`
- **Remplace** : —

## Contexte

La tâche RECHERCHER-DENSE-EXACT est en cours sur la branche `tache/RECHERCHER-DENSE-EXACT`. Son code non commité de `crible-search::dense::exact` rend une erreur `internal` quand un passage retenu appartient à un document dont la ligne `documents` a été supprimée après la prise de l'instantané de lecture. Une requête valide obtient donc un 500. Le point 1 de `@relecteur` relève ce cas ; `@architecte` le qualifie d'**écart** entre deux documents et de point **NON-COUVERT**.

### Ce que fait une requête

Une recherche prend un `Arc<Snapshot>` de la génération G (`docs/02-architecture.md:134`). L'instantané porte les segments, les codes, les bitmaps, les colonnes de valeurs, le `Searcher` tantivy et la génération ; **il ne porte pas de transaction SQLite**. Les filtres, le pool et les scores sont évalués sur G. Les textes et les champs des k résultats sont ensuite lus dans `records.sqlite`, via le pool de connexions en lecture, en mode WAL, donc **dans l'état SQLite courant** (`docs/02-architecture.md:136-138` ; `docs/03-recherche.md:14`, étape 8 « Payload »).

`docs/02-architecture.md:137` borne cette limite : « les champs d'un document peuvent refléter une génération plus récente que celle utilisée pour filtrer. Seule une modification de métadonnées peut produire cet écart, ce qui est accepté ». Le texte du passage, lui, est immuable pour un iid donné (`:138`).

### Ce que change ADR-0041

L'ADR-0041 (décision 1) supprime la ligne `documents` d'un document retiré dans la transaction du commit de génération ; ses passages restent en pierres tombales jusqu'à la compaction (ADR-0041:53, ADR-0006:23). La suppression produit donc un écart **plus fort** que celui décrit par `docs/02-architecture.md:137` : non pas un champ plus récent, mais **une ligne `documents` absente**. `docs/02-architecture.md:137` ne prévoit pas ce cas, et l'application d'ADR-0041 (`:112`) n'a pas fait aligner cette phrase.

Le volume n'est pas négligeable : environ **220 suppressions par nuit** pour Legifacile (ADR-0006:12). Un 500 sur un cas normal n'est pas acceptable, et `internal` est réservé à une « erreur inattendue » (`docs/05-api.md:44`). Le statut `error` dans les analytics (`docs/06-observabilite.md:26`) et le bruit dans le journal de debug (`:55`, `:72`) en découleraient.

### Ce que la conception garantit déjà

- La ligne `passages` n'est jamais supprimée en place dans une époque. À la compaction (P3), une époque et son `records.sqlite` restent tant qu'un lecteur les tient (`docs/02-architecture.md:135`, `:166`). La même question ne se pose donc pas pour `passages`, **à deux conditions** : que l'étape payload lise le `records.sqlite` de l'époque du snapshot, et qu'aucune époque ne soit effacée pendant qu'un instantané la tient.
- « Une pierre tombale fixe `dead_since_gen`. Le passage sort de la bitmap des vivants, donc du pool : **aucun résultat ne manque** à cause d'une suppression » (`docs/02-architecture.md:151`). Un résultat retenu sur G reste donc légitime même si son document a été retiré depuis.
- INV-03 dit qu'« une génération est visible entièrement ou pas du tout » (`docs/08-invariants-et-tests.md:11`). Il ne dit pas si la réponse entière, payloads compris, doit être cohérente avec la génération annoncée.

## Décision proposée

Cet ADR porte **six points à trancher**, dont le premier commande les autres. Chaque option est décrite à parité ; la recommandation de `@architecte` est indiquée, le propriétaire tranche.

### Point 1 — comportement de l'étape payload quand la ligne `documents` manque

Le passage retenu est vivant dans G. Sa ligne `documents` est absente de l'état SQLite courant.

| Option | Comportement | Ce qu'elle implique |
|---|---|---|
| **A (recommandée par `@architecte`)** | Rendre le résultat, avec le texte (immuable, présent) et les seuls champs de niveau passage. Les champs de document sont absents. Aucune erreur. | Respecte `docs/02-architecture.md:136-138` et `:151`. Contredit en partie `return.fields: "all"` (`docs/05-api.md:145`), qui devient « tous les champs disponibles ». Aucun mécanisme nouveau, aucune transaction tenue. Deux sous-choix restent ouverts : marqueur explicite par résultat (nouveau champ d'API dans `docs/05`) ou absence simple des champs de document. |
| **B** | Écarter le résultat et continuer avec les suivants. | Viole `docs/02-architecture.md:151`. Le top-k peut être inférieur à k sans sur-lecture du pool (`docs/03-recherche.md:52`). L'éviction est invisible dans les analytics (`docs/06-observabilite.md:19-27`), qui ne comptent ni résultats écartés ni documents retirés. |
| **C** | Lire les payloads dans une vue cohérente avec G : transaction de lecture ouverte à la prise de l'instantané, avec vérification de `meta.generation` = G. | INV-03 couvrirait alors la réponse entière. Coût : une transaction tenue pendant toute la requête, donc checkpoint WAL bloqué ; une connexion du `ReadPool` tenue pendant le scan dense ; interaction avec ADR-0038 (délai de 1 000 ms, `overloaded`) et avec la capacité du pool. Change une phrase de conception acceptée (`docs/02-architecture.md:136-138`). |
| **D** | Garder `internal` (statu quo du code non commité). | À rejeter : 500 sur un cas normal, statut `error` dans les analytics (`docs/06-observabilite.md:26`), bruit dans le journal (`:55`, `:72`). |

Dans tous les cas sauf D, `internal` est **retiré de ce cas** et reste réservé aux vraies incohérences : segment absent, JSON de champs qui n'est pas un objet, ligne `passages` absente.

### Point 2 — périmètre d'INV-03

- **Statu quo** : INV-03 porte sur le pool, les scores et le classement, pas sur les payloads. C'est la lecture actuelle de `docs/02-architecture.md:137` ; à dire explicitement dans `docs/08-invariants-et-tests.md:11`.
- **Extension** : INV-03 couvre la réponse entière, payloads compris. Cela n'est possible qu'avec l'option C du point 1.

La recommandation de `@architecte` découle du point 1 (option A ⇒ statu quo).

### Point 3 — reproductibilité à génération fixée

La réponse est-elle stable quand la génération est fixée ? Le client dispose de `generation` dans chaque réponse de recherche (`docs/05-api.md:14`, `:162`).

- Avec A, la réponse est stable pour les scores, le classement et le texte ; les champs de document peuvent manquer, ou différer après une modification de métadonnées (`docs/02-architecture.md:137`). La stabilité est donc **partielle** et documentée comme telle.
- Avec C, la réponse est stable pour toute sa durée de vie, payloads compris.
- Avec B, la réponse dépend du nombre de résultats écartés, donc du hasard de la concurrence : ni stable, ni traçable.

### Point 4 — contraintes de rétention à conserver en P3

Quelle que soit l'option, deux règles sont à garder :

- une époque n'est jamais supprimée tant qu'un instantané la tient (`docs/02-architecture.md:135`, `:166`) ;
- l'étape payload lit toujours le `records.sqlite` de l'époque du snapshot, jamais un chemin relu à chaque requête.

Ces règles sont déjà écrites pour la compaction ; l'ADR les rend explicites comme condition de correction de l'étape payload, et les cite dans la fiche de P3.

### Point 5 — forme de la réponse si un marqueur est retenu

Si le sous-choix « marqueur explicite » du point 1 est retenu, `docs/05-api.md` gagne un champ par résultat, par exemple `document_deleted: true` ou un état de payload. Le nom, la présence conditionnelle et la valeur par défaut restent à fixer ; l'ADR ne les invente pas. Si l'absence simple est retenue, `docs/05` précise seulement que `fields: "all"` signifie « tous les champs disponibles pour ce résultat ».

### Point 6 — application

Après acceptation seulement, et dans un commit qui cite `ADR-0043` :

- `docs/02-architecture.md:137` : deux causes d'écart, la modification de métadonnées et la suppression du document, avec le comportement retenu au point 1 ;
- `docs/05-api.md` : selon le point 5, marqueur ou définition de `fields: "all"` ; `:44` reste inchangé (`internal` = erreur inattendue) ;
- `docs/08-invariants-et-tests.md:11` : périmètre d'INV-03 selon le point 2 ;
- `docs/03-recherche.md:14` (étape 8) : une phrase sur le cas « ligne `documents` absente » ;
- `docs/phases/P1-stockage-et-recherche-exacte.md` (tâche RECHERCHER-DENSE-EXACT) : le critère d'acceptation du cas, et un test nommé qui le couvre ;
- `docs/suivi.md`, section « Écarts à la conception » : ligne citant `ADR-0043`, à ajouter par l'orchestrateur ;
- code, dans la tâche : remplacer l'erreur `internal` par le comportement retenu, et adapter les tests.

**RECHERCHER-DENSE-EXACT reste bloquée jusqu'à la décision.**

## Conséquences

- **Positives** :
  - une requête valide ne renvoie plus 500 à cause d'une suppression courante (environ 220 par nuit) ;
  - `internal` retrouve son sens : une incohérence réelle, pas un état normal de la base ;
  - la limite connue de `docs/02-architecture.md:137` devient exacte, avec ses deux causes ;
  - le comportement de l'étape payload devient testable sans transaction longue, si l'option A est retenue ;
  - la rétention des époques et la lecture du `records.sqlite` du snapshot sont explicitées avant P3.
- **Négatives / coûts** :
  - avec A, `fields: "all"` ne garantit plus tous les champs : un client peut recevoir un résultat sans champs de document, et doit le supporter ;
  - avec A, la stabilité à génération fixée est partielle, ce qui demande une phrase de documentation et un test qui la fixe ;
  - avec C, une transaction de lecture est tenue pendant toute la requête : checkpoint WAL retardé, connexion du `ReadPool` immobilisée, interaction à vérifier avec ADR-0038 et la taille du pool ;
  - avec B, le top-k n'est plus garanti et les analytics ne montrent pas l'éviction.
- **À surveiller** :
  - un client qui exige tous les champs de document, ou qui traite un champ absent comme une erreur : il rouvrirait le point 1 ou le point 5 ;
  - une mesure de latence de l'étape `fetch` et du taux de checkpoints retardés, si C est retenue : elle dirait si le coût de la cohérence totale est acceptable ;
  - l'arrivée de P3 : la première compaction concurrente d'une recherche doit encore trouver le `records.sqlite` de l'époque du snapshot (INV-09) ;
  - un pic de suppressions qui rendrait le cas fréquent au point de justifier un marqueur explicite plutôt qu'une absence.

## Alternatives écartées

Les options B, C et D du point 1 sont décrites ci-dessus avec leur coût. Les raisons de leur écart, si l'option A est retenue :

| Alternative | Pourquoi écartée |
|---|---|
| **B.** Écarter le résultat. | Contredit `docs/02-architecture.md:151` : l'absence de résultat serait causée par une suppression, exactement ce que la conception interdit. Le top-k devient variable sans sur-lecture, et l'éviction n'est pas observable, donc non diagnosticable. |
| **C.** Transaction de lecture cohérente avec G. | Donne une réponse entièrement cohérente, au prix d'une transaction et d'une connexion tenues pendant tout le scan dense, d'un checkpoint WAL retardé et d'un couplage nouveau avec ADR-0038. Change une phrase de conception acceptée. Le gain ne couvre qu'un cas rare (suppression entre la prise du snapshot et l'étape payload), et A le couvre sans mécanisme nouveau. |
| **D.** Garder `internal`. | 500 sur un cas normal, statut `error` dans les analytics et bruit dans le journal. `internal` est réservé à une erreur inattendue (`docs/05-api.md:44`). |

Les sous-choix du point 5 (marqueur explicite ou absence simple) restent ouverts : l'ADR ne tranche pas la forme de l'API sans décision du propriétaire.

## Références

- `docs/02-architecture.md` : `:134` (instantané, génération), `:135` (libération, effacement différé), `:136-138` (payloads, limite connue, texte immuable), `:151` (aucun résultat ne manque), `:166` (suppression de l'époque après le dernier lecteur).
- `docs/03-recherche.md` : `:14` (étape 8 « Payload »), `:52` (sortie bornée par branche).
- `docs/05-api.md` : `:14` (génération dans la réponse), `:44` (`internal` = erreur inattendue), `:145` (`return.fields`), `:162` (génération renvoyée).
- `docs/06-observabilite.md` : `:19-27` (statuts et étapes agrégés), `:26` (`ok` ou code d'erreur), `:55`, `:72` (journal de debug).
- `docs/08-invariants-et-tests.md:11` (INV-03).
- ADR-0006 : segments, pierres tombales et compaction ; `:12` (volumes de la nuit), `:23` (pierre tombale).
- ADR-0016 : le commit SQLite est l'unique point de commit.
- ADR-0038 : délai d'attente sur les verrous SQLite (`busy_timeout`), `overloaded`.
- ADR-0041 : suppression de la ligne `documents` dans la transaction du commit (`:46-55`), application (`:108-116`).
- `docs/phases/P1-stockage-et-recherche-exacte.md` : fiche RECHERCHER-DENSE-EXACT.
