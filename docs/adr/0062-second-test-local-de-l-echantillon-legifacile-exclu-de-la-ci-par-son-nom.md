# ADR-0062 — Second test local de l'échantillon Legifacile, exclu de la CI par son nom

- **Statut** : Accepté (décision du propriétaire le 2026-10-25)
- **Date** : 2026-10-25 (proposition et acceptation)
- **Décideurs** : propriétaire
- **Décision** : accepté le 2026-10-25 par le propriétaire, tel que proposé. La décision 1 est l'option **(1a)**, un **second `--skip` explicite** nommant `inv_09_legifacile_sample_queries_return_the_same_results_after_compaction` ; la décision 2 est l'option **(2ii)**, **aucune borne par défaut sur la durée**, la durée étant relevée à J3 et notée dans le suivi. Les modifications à appliquer sont celles de la section « Modifications à appliquer après acceptation », points 1 à 8 ; le point 5 (borne et MES-16) n'est pas retenu.
- **Proposé par** : agent greffier, sur demande du propriétaire, à la suite de la relecture du 1er tour de la tâche `VERIFIER-EQUIVALENCE-COMPACTION` (phase P3, branche `tache/VERIFIER-EQUIVALENCE-COMPACTION`). Point bloquant relevé par `@relecteur` ; verdict de `@architecte` : **ECART sur la lettre, CONFORME sur le principe**.
- **Remplace** : — (complète ADR-0057, qui reste en vigueur et `Accepté` ; son corps n'est pas modifié, comme pour ADR-0059, qui le complète déjà)

## Contexte

### Ce que la fiche de la tâche demande

La fiche `VERIFIER-EQUIVALENCE-COMPACTION` (`docs/phases/P3-cycle-de-vie.md`, lignes 94 à 99) demande, au titre des livrables :

> un test d'intégration qui lance les 200 requêtes `synth-small`, et les requêtes de l'échantillon Legifacile en exact, avant et après la compaction.

Son critère d'acceptation est « mêmes clés et mêmes scores à 1e-6 près, sur toutes les requêtes », et elle ne cite que l'invariant INV-09. La liste du jalon J3, en tête du même fichier (lignes 5 à 11), reprend la même exigence : « sur l'échantillon Legifacile et `synth-small` […] trois lots quotidiens simulés, puis une compaction, donnent des résultats identiques avant et après ».

### Ce que le test écrit fait

`crates/crible/tests/inv_09_compaction_equivalence.rs` (écrit par `@testeur`, non commité) contient trois tests :

| Test | Données | Lancé par défaut |
|---|---|---|
| `inv_09_synth_small_200_queries_return_the_same_results_after_compaction` | `data/synth-small/` | oui |
| `inv_09_the_legifacile_sample_test_fails_without_its_prerequisites` | aucune | oui |
| `inv_09_legifacile_sample_queries_return_the_same_results_after_compaction` | `data/legifacile-sample/`, `data/models/mdenseon/` et ONNX Runtime | non, `#[ignore]` |

Le troisième, marqué `#[ignore]` (ligne 964 du fichier), demande trois prérequis : `data/legifacile-sample/`, qui est gitignoré et dérive de la production Legifacile (ADR-0032) ; `data/models/mdenseon/` ; et `ORT_DYLIB_PATH`. Il attache les six structures de l'échantillon, encode les 300 questions de `queries.jsonl` en fp32 — la précision par défaut des requêtes, ADR-0052 — puis compare les réponses de la recherche exacte avant et après la compaction. Il dure donc longtemps, et c'est ce que son commentaire de module annonce : « its duration and any bound on it belong to the pending ADR on running it » (lignes 70 à 73).

Les deux autres tests tournent sans données : le premier prouve INV-09 sur `synth-small`, le second prouve la règle anti-saut (un test `#[ignore]` lancé sans son prérequis échoue avec la commande qui le produit, `docs/08-invariants-et-tests.md`, ligne 43).

### Ce que la CI fait aujourd'hui

Le job `rust-onnx` d'`.github/workflows/ci.yml` exécute, ligne 132 (reprise dans `docs/07-stack-et-deploiement.md`, ligne 140) :

```
cargo test --locked --workspace -- --include-ignored --skip inv_12_the_legifacile_sample_passes_the_acceptance_check
```

Il n'exclut qu'**un** test, nommé. Le nouveau test serait donc exécuté par ce job, sans `data/legifacile-sample/`, sans modèle et sans `ORT_DYLIB_PATH`, et il échouerait — bruyamment, comme la règle anti-saut l'exige, mais à chaque push. La CI deviendrait rouge et la fusion de la phase serait bloquée.

### Les textes que la décision rend faux

- **`docs/07-stack-et-deploiement.md`, ligne 143** : « Le test de l'échantillon Legifacile est le seul exclu, par son nom ». Faux dès le second test local. La commande de la ligne 140 l'est aussi.
- **ADR-0057, point 3** (lignes 133 à 138) : « Cette commande exécute **19 tests ignorés** et laisse **1 test local** » et « Le test exclu est `inv_12_the_legifacile_sample_passes_the_acceptance_check` ». Le décompte change : 21 tests `#[ignore]` au total, dont 19 exécutés et **2** laissés locaux. Le nombre de tests exécutés passe de 419 à 421, parce que le nouveau fichier apporte aussi deux tests non ignorés.
- **Fiche `AJOUTER-ORT-CI`** (`docs/phases/P2-encodeur-et-recette.md`, lignes 163 et 164), reprise dans ADR-0057 (point 7, ligne 170) : « les 19 tests ignorés y passent » et « le test de l'échantillon Legifacile est le **seul** exclu ».
- **`docs/08-invariants-et-tests.md`, ligne 48** : « une CI qui exécute `--include-ignored` doit nommer explicitement chaque test qu'elle laisse local, par son nom, et le justifier ». La règle est **conforme** — elle admet plusieurs exclusions nommées. Seul son exemple (« c'est le cas de `inv_12_…` ») devient incomplet.

### Ce qui ne change pas

ADR-0057 décide que « l'échantillon Legifacile reste local » et son point 6 le répète. Le principe est intact : aucun contenu de production n'entre en CI (ADR-0032), et `#[ignore]` reste le seul mécanisme de saut (`docs/08`, ligne 43). Cet ADR ne touche ni au seuil de recette, ni à l'artefact du modèle, ni à la politique de reprise du téléchargement (ADR-0059).

### Ce que rien ne borne

Aucun texte ne borne la durée du test lancé à la main ni par `@jalon`. ADR-0057 point 5 ne vise que la durée du job `rust-onnx`, et `docs/09-plan-de-developpement.md` (ligne 92) demande à `@jalon` de lancer les tests sans donner de délai. La règle 7 d'`AGENTS.md` interdit d'affirmer une durée sans rapport de `@mesureur`. Un premier passage local, non consigné, a pris de l'ordre d'une demi-heure : cette valeur est un ordre de grandeur rapporté, pas une mesure.

Conséquence pratique : une demi-heure dépasse le délai de 10 minutes d'un appel d'agent au premier plan. `@jalon` devra donc lancer ce test **en arrière-plan**, quelle que soit l'option retenue pour la durée.

## Décision

**Un second test de l'échantillon Legifacile est admis, et la CI l'exclut par son nom, avec un second `--skip` explicite. Le test peut alors être commité avec son exclusion, dans le même travail. La durée du test n'est pas bornée par défaut : elle est relevée au moment de J3.**

Les deux points ci-dessous sont soumis au propriétaire. Chacun porte une recommandation ; rien n'est décidé ici.

### 1. Mécanisme d'exclusion : un second `--skip` explicite — **recommandé**

La commande du job `rust-onnx` devient :

```
cargo test --locked --workspace -- --include-ignored \
  --skip inv_12_the_legifacile_sample_passes_the_acceptance_check \
  --skip inv_09_legifacile_sample_queries_return_the_same_results_after_compaction
```

Propriétés recherchées, toutes trois déjà voulues par ADR-0057 et `docs/08` :

- **l'exclusion est nommée**, donc lisible et justifiable une par une, comme `docs/08` ligne 48 l'exige ;
- **l'oubli est bruyant** : un futur test de l'échantillon ajouté sans sa ligne `--skip` est exécuté en CI, échoue faute de données et rougit le job. Un oubli rend la CI **rouge, pas verte** — c'est la propriété d'ADR-0057, et le prix assumé est une ligne par test local ;
- **aucun saut silencieux** : le test n'est pas filtré par une règle générale, il est nommé un par un, et `--skip` ne s'applique qu'à la commande, pas au code.

Cette option ne demande aucune modification de code de test, aucun renommage, et laisse `inv_12_…` tel quel — dont le nom est un contrat (ADR-0057, « À surveiller »).

### 2. Durée du test lancé à la main et par `@jalon` : pas de borne par défaut — **recommandé**

Aucune mesure n'existe ; écrire une borne maintenant serait inventer un chiffre, ce que la règle 7 d'`AGENTS.md` interdit. La durée est donc relevée **au moment de J3**, quand le test tourne pour de vrai, et notée dans `docs/suivi.md` par l'orchestrateur. Aucune valeur par défaut n'est inscrite, ni dans `docs/08`, ni dans la consigne de `jalon`.

Dans les deux options, et c'est le point non négociable :

- `@jalon` lance le test **en arrière-plan**, puis relève sa durée et son résultat dans le rapport de jalon. Un appel au premier plan ne tient pas la demi-heure constatée ;
- le test n'est jamais « sauté » faute de données : sans `data/legifacile-sample/`, sans modèle ou sans `ORT_DYLIB_PATH`, il échoue avec la commande qui les produit (règle anti-saut, `docs/08` ligne 43) ;
- aucune affirmation de durée n'entre dans un document avant d'être relevée.

Si le propriétaire préfère une garde contre un blocage, l'option (i) la fournit : une mesure MES-16 « durée du test INV-09 sur l'échantillon Legifacile », puis une borne écrite (par exemple 45 minutes) dans `docs/08` et dans la consigne de `jalon`. Le coût est un rapport de mesure et une valeur à maintenir.

### 3. Ce qui n'est pas décidé ici

- Aucun autre test n'est exclu, et aucune règle générale d'exclusion n'est créée.
- L'échantillon Legifacile ne sera pas publié, ni reconstitué, ni réduit pour tenir en CI : ADR-0032 et ADR-0057 restent en vigueur.
- Aucun mécanisme d'exclusion autre que `--skip` (feature Cargo, binaire séparé, variable d'environnement) n'est admis : `docs/08` ligne 43 fait d'`#[ignore]` le seul mécanisme de saut, et `--skip` ne fait que choisir ce que la commande lance.
- La durée du job `rust-onnx` et son `timeout-minutes` ne changent pas : le test exclu ne s'y exécute pas.

### 4. Effet sur la tâche `VERIFIER-EQUIVALENCE-COMPACTION`

Le test **ne peut pas être commité seul**. Un commit qui l'ajoute sans le second `--skip` fait échouer le job `rust-onnx` au push suivant, faute de données. L'exclusion doit donc entrer dans le même travail que le test.

Tant qu'ADR-0062 est `Proposé`, la tâche reste `BLOQUE` : `docs/07` et `ci.yml` ne peuvent pas être modifiés sans décision. La ligne d'écart correspondante revient à l'orchestrateur, qui tient `docs/suivi.md` : elle n'y figure pas encore au moment de la proposition.

## Conséquences

- **Positives** :
  - INV-09 est prouvé sur les requêtes réelles de l'échantillon Legifacile, comme la fiche et le jalon J3 le demandent, sans faire entrer une donnée de production en CI ;
  - l'exclusion reste nommée, donc lisible, justifiable et vérifiable : `docs/08` ligne 48 et ADR-0057 sont respectés dans leur lettre comme dans leur esprit ;
  - un oubli d'exclusion rougit la CI au lieu de la laisser verte : le mécanisme échoue du bon côté ;
  - aucune modification de code de test, aucun renommage, aucun nouveau mécanisme : un `--skip` de plus dans un fichier YAML et une ligne de commande dans `docs/07` ;
  - la durée du test devient un fait consigné au moment de J3, sans chiffre inventé.
- **Négatives / coûts** :
  - chaque futur test de l'échantillon demandera une ligne `--skip` de plus, et un oubli coûtera un run rouge ;
  - la commande du job `rust-onnx` s'allonge et devient moins lisible ; elle est dupliquée dans `ci.yml` et dans `docs/07`, donc à corriger deux fois ;
  - le test local reste **non prouvé à la fusion** : une régression de la compaction qui ne touche que les données réelles ne sera vue qu'au passage local ou à J3 ;
  - le corps d'ADR-0057 garde des décomptes faux (« 19 tests ignorés, 1 test local », « le seul exclu »). Un ADR accepté n'est pas réécrit : cet ADR fait foi pour ces chiffres, et l'index le signale. Un lecteur qui ne lit que l'ancien texte peut se tromper ;
  - sans borne, un blocage du test ne se voit que par l'absence de résultat dans le rapport de jalon ;
  - `@jalon` doit désormais lancer un test long en arrière-plan, ce que sa consigne ne dit pas encore.
- **À surveiller** :
  - le décompte des tests `#[ignore]` et des tests exécutés par `rust-onnx` : toute phrase qui cite un nombre (« 19 », « 420 », « 421 ») doit être relue à chaque test local ajouté ; c'est ce qui a rendu ce point nécessaire ;
  - les noms des deux tests exclus sont des **contrats** : les renommer sans mettre à jour les deux `--skip` ferait exécuter un test local en CI, qui échouerait bruyamment ;
  - la durée relevée à J3 : si elle dépasse largement la demi-heure constatée, ou si elle varie d'un passage à l'autre, rouvrir le point 2 et écrire une borne (option (i)) ;
  - la duplication de la commande entre `ci.yml` et `docs/07` : une divergence ferait décrire à la documentation autre chose que ce que la CI exécute ;
  - le fait que `docs/07` et `docs/08` ne citent plus de texte faux après application : la ligne 143 de `docs/07` et la ligne 48 de `docs/08` sont les deux endroits à relire.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| **(1a) Second `--skip` explicite, nommant le test** — recommandé | Un oubli de ligne est bruyant (CI rouge), l'exclusion reste nommée et justifiable une par une, et rien ne bouge dans le code de test. Coût : une ligne par test local. |
| **(1b) Un motif de nom commun filtré par un seul `--skip`** (sous-chaîne libtest) | Trois défauts. (i) Un motif comme `legifacile_sample` exclurait aussi `inv_09_the_legifacile_sample_test_fails_without_its_prerequisites`, qui **doit** tourner pour prouver la règle anti-saut (`docs/08`, ligne 47). (ii) Il obligerait à renommer `inv_12_the_legifacile_sample_passes_the_acceptance_check`, dont le nom est un contrat (ADR-0057, « À surveiller »). (iii) Il ferait un saut **silencieux** : un futur test de l'échantillon serait exclu sans que personne l'ait écrit, contrairement à `docs/08` lignes 42 et 43 et à la ligne 48, qui exige de nommer « explicitement chaque test ». |
| **(1c) Un autre mécanisme** (feature Cargo, binaire de test séparé, variable d'environnement, `cfg`) | `docs/08` ligne 43 fait d'`#[ignore]` le seul mécanisme de saut autorisé ; un `cfg` ou une feature conditionnerait la **compilation** du test, donc il n'apparaîtrait plus dans le compte des tests, et le saut deviendrait invisible. Un binaire séparé déplacerait le test hors du workspace, donc hors de `cargo test --workspace`, sans décision sur qui le lance. |
| **Committer le test et corriger la CI au push suivant** | La CI serait rouge entre les deux commits, sur la branche de phase comme sur toute branche issue d'elle, et la fusion serait bloquée par un état intermédiaire connu. Un commit qui casse la CI se répare avant d'être poussé, pas après. |
| **Faire tourner le test en CI sans son prérequis, et le laisser échouer** | Un test rouge à chaque push ne prouve rien et rend la CI inutilisable : on ne distingue plus une régression d'un prérequis manquant. C'est exactement ce qu'ADR-0057 et `docs/08` refusent. |
| **Rendre le test silencieux quand les données manquent** (`if !path.exists() { return; }`) | Interdit par `docs/08` ligne 43 et déjà corrigé une fois dans `recette_import.rs` par `AJOUTER-ORT-CI`. Un test qui saute sans le dire se lit comme vert. |
| **Publier ou réduire l'échantillon pour qu'il tienne en CI** | Contredit ADR-0032 : les données dérivent de la production. Un échantillon réduit ou anonymisé ne prouverait plus l'équivalence sur les données réelles, qui est l'objet d'INV-09. |
| **(2i) Une mesure MES-16, puis une borne de commande** (par exemple 45 minutes) | Défendable, et à préférer si le propriétaire veut une garde contre un blocage. Écartée par défaut pour la simplicité : elle demande un rapport de mesure pour une seule durée, puis une valeur à maintenir dans deux documents (consigne de `jalon` et `docs/08`), alors que J3 produira la même durée sans dispositif nouveau. La règle 7 d'`AGENTS.md` interdit de toute façon d'écrire la borne avant le relevé. |
| **(2ii) Pas de borne, durée relevée à J3 et notée dans le suivi** — recommandé | Aucun chiffre inventé, aucun document de plus, aucun réglage à maintenir. Le seul coût est qu'un blocage se voit par l'absence de résultat dans le rapport de jalon, et non par une coupure nette. |
| **Lancer le test au premier plan depuis `@jalon`** | Une demi-heure constatée dépasse le délai de 10 minutes d'un appel d'agent au premier plan : l'appel serait coupé avant la fin, et le rapport de jalon conclurait à un échec qui n'en est pas un. L'arrière-plan est obligatoire dans les deux options. |

## Modifications à appliquer après acceptation

Cette section décrit les modifications. **Aucune n'est appliquée par cet ADR** : elles le seront par `@greffier`, dans un commit qui cite `ADR-0062`, après acceptation du propriétaire.

### `.github/workflows/ci.yml`

1. **Job `rust-onnx`, étape `Test with ONNX Runtime`** (ligne 132) : ajouter le second `--skip inv_09_legifacile_sample_queries_return_the_same_results_after_compaction` à la commande, et un commentaire d'une ligne qui renvoie à cet ADR. Rien d'autre ne change : ni `timeout-minutes`, ni le cache, ni le téléchargement.

### `docs/07-stack-et-deploiement.md`

2. **« CI (GitHub Actions) »**, commande ligne 140 : ajouter le second `--skip`. La commande doit rester identique à celle de `ci.yml`.
3. **« CI (GitHub Actions) »**, ligne 143 : remplacer « Le test de l'échantillon Legifacile est le seul exclu, par son nom » par le fait que **les tests de l'échantillon Legifacile sont les deux exclus, nommés un par un**, parce que leurs données dérivent de la production (ADR-0032) et que l'un d'eux demande en plus ONNX Runtime et le vrai modèle ; citer `ADR-0057` et `ADR-0062`.

### `docs/08-invariants-et-tests.md`

4. **« Sauts de test »**, ligne 48 : l'exemple devient deux cas nommés — `inv_12_the_legifacile_sample_passes_the_acceptance_check` et `inv_09_legifacile_sample_queries_return_the_same_results_after_compaction` — et la phrase qui les justifie reste celle d'ADR-0032. La règle elle-même ne change pas.
5. **(Option 2i seulement)** Ajouter la borne retenue pour la durée du test, à côté de la règle ou dans une note, avec le renvoi à MES-16. **(Option 2ii, recommandée)** Rien à ajouter : la durée est un fait de suivi, pas une règle.

### `docs/phases/P2-encodeur-et-recette.md`

6. **Fiche `AJOUTER-ORT-CI`**, lignes 163 et 164, et le même critère repris dans ADR-0057 point 7 : ces deux phrases décrivent l'état à la clôture de la tâche (`FAIT`) et deviennent fausses. Ne pas réécrire le critère d'une tâche terminée : ajouter une note datée sous les critères, du type « état à la clôture d'`AJOUTER-ORT-CI` ; depuis `ADR-0062`, la CI exclut deux tests de l'échantillon Legifacile, nommés un par un ». La forme exacte de la note revient au propriétaire s'il préfère une correction du critère.

### `.opencode/agents/jalon.md`

7. **Méthode, étape 4 (« Invariants »)** : dire que les tests d'invariant qui demandent l'échantillon Legifacile sont lancés **en arrière-plan** et que leur durée est relevée dans le rapport. **(Option 2i seulement)** Y écrire aussi la borne de durée.

### `docs/adr/`

8. **`docs/adr/README.md`** : passer la ligne d'ADR-0062 au statut décidé, et compléter la ligne d'ADR-0057 par « complété par ADR-0059 et ADR-0062, sans modification du corps ».
9. **ADR-0057** : **aucune modification de son corps**. Ses points 3 et 7 gardent leurs décomptes d'origine ; le présent ADR fait foi pour ces chiffres, comme ADR-0059 fait foi pour la politique de reprise du téléchargement.

### Hors de ces documents

`crates/crible/tests/inv_09_compaction_equivalence.rs` relève de la tâche `VERIFIER-EQUIVALENCE-COMPACTION`, pas de cet ADR : le fichier est écrit, et il sera commité avec l'exclusion. `docs/suivi.md` reste tenu par l'orchestrateur : ligne d'écart de la tâche, statut de la tâche, durée relevée à J3.

## Références

- `docs/phases/P3-cycle-de-vie.md`, lignes 5 à 11 (jalon J3) et lignes 94 à 99 (fiche `VERIFIER-EQUIVALENCE-COMPACTION`).
- `crates/crible/tests/inv_09_compaction_equivalence.rs` (non commité) : trois tests, dont `inv_09_legifacile_sample_queries_return_the_same_results_after_compaction`, `#[ignore]` à la ligne 964 ; commentaire de module lignes 39 à 79 (prérequis, six structures, 300 questions, fp32).
- `.github/workflows/ci.yml`, job `rust-onnx`, ligne 132 : commande `cargo test … --include-ignored --skip inv_12_…`.
- `docs/07-stack-et-deploiement.md`, « CI (GitHub Actions) », lignes 137 à 144, en particulier la commande ligne 140 et la phrase ligne 143 sur « le seul exclu ».
- `docs/08-invariants-et-tests.md`, « Sauts de test », lignes 42 à 48 : `#[ignore]` seul mécanisme autorisé, échec franc sans prérequis, exclusion nommée un par un (ligne 48, conforme) ; « Jeux de données », ligne 56 (`legifacile-sample`).
- `docs/phases/P2-encodeur-et-recette.md`, fiche `AJOUTER-ORT-CI`, lignes 163 et 164 (« les 19 tests ignorés », « le seul exclu »).
- `docs/09-plan-de-developpement.md`, ligne 92 : `/jalon J<n>` lance les tests et la démo, sans délai prescrit.
- `AGENTS.md`, règle 7 (« Mesurer avant d'affirmer ») et règle 14 (« Rien de volumineux dans git »).
- ADR-0057 (ONNX Runtime et vrai modèle en CI : échantillon Legifacile local, `--skip` nommé, règle anti-saut, décomptes) ; ADR-0059 (complète ADR-0057 sans modifier son corps, précédent de forme) ; ADR-0032 (accès à la production, données non publiées) ; ADR-0052 (requêtes encodées en fp32) ; ADR-0006 (segments, pierres tombales, compaction).
- `docs/suivi.md`, ligne de la tâche `VERIFIER-EQUIVALENCE-COMPACTION` ; la section « Écarts à la conception » doit y gagner une ligne citant `ADR-0062`, tenue par l'orchestrateur.
