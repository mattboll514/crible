# ADR-0063 — Troisième test local de l'échantillon Legifacile, exclu de la CI par son nom

- **Statut** : Accepté (décision du propriétaire le 2026-10-25 ; accepté par anticipation le 2026-10-25, confirmé par le propriétaire le 2026-10-26)
- **Date** : 2026-10-25 (proposition et acceptation)
- **Décideurs** : propriétaire
- **Décision** : accepté le 2026-10-25 par le propriétaire, tel que proposé, lors d'une acceptation anticipée de sa part, confirmée par le propriétaire au point de fin de l'enchaînement de P3, le 2026-10-26. La décision 1 est l'option **(1a)**, un **troisième `--skip` explicite** nommant `inv_08_legifacile_sample_queries_return_the_same_results_after_export_and_import` ; la décision 2 est l'option **(2ii)**, **aucune borne par défaut sur la durée**, la durée étant relevée à J3 et notée dans le suivi. Les modifications à appliquer sont celles de la section « Modifications à appliquer après acceptation », points 1 à 9 : le point 1 (`ci.yml`) revient à `@dev`, dans le même travail que le test ; les points 2 à 8 sont appliqués par `@greffier` ; le point 9 ne demande aucune modification du corps d'ADR-0057 ni d'ADR-0062, et le point 6 correspond à l'option (2ii), donc rien n'est ajouté pour la durée.
- **Proposé par** : agent greffier, sur demande du propriétaire, à la suite de l'écart relevé par l'orchestrateur **avant l'écriture des tests** de la tâche `VERIFIER-ALLER-RETOUR-EXPORT` (phase P3, branche `tache/VERIFIER-ALLER-RETOUR-EXPORT`). Verdict de `@architecte` : **ECART**.
- **Remplace** : — (complète ADR-0057 et ADR-0062, qui restent en vigueur et `Accepté` ; aucun de leurs corps n'est modifié, comme ADR-0059 puis ADR-0062 l'ont déjà fait pour ADR-0057)

## Erratum du 2026-10-26

Le nom du test écrit au point 3 de la section « Modifications à appliquer après acceptation » était mal orthographié, et il est corrigé, sans qu'aucune décision ne change. La correction de cette coquille a été acceptée par anticipation le 2026-10-26, confirmé par le propriétaire le 2026-10-26.

- **Avant** : `inv_08_the_legifacile_sample_queries_return_the_same_results_after_export_and_import`.
- **Après** : `inv_08_legifacile_sample_queries_return_the_same_results_after_export_and_import`.

Motif : ce nom est un contrat (section « Contexte », « Le test s'appellera… »), et il est écrit sans l'article défini partout ailleurs dans ce document, dans `docs/07-stack-et-deploiement.md`, dans `docs/08-invariants-et-tests.md` et dans le code. La décision elle-même, l'option (1a) et le troisième `--skip` explicite, est inchangée.

## Contexte

### Ce que la fiche de la tâche demande

La fiche `VERIFIER-ALLER-RETOUR-EXPORT` (`docs/phases/P3-cycle-de-vie.md`, lignes 114 à 119) demande de prouver INV-08 :

> **Livrables** : un test qui exporte, crée un index vide de même config, importe, puis compare les recherches.
> **Critères d'acceptation** : résultats identiques, avec les mêmes clés et les mêmes scores à 1e-6 près.
> **Invariants** : INV-08.

### Ce que la conception demande en plus

`docs/08-invariants-et-tests.md`, ligne 16, définit la preuve d'INV-08 : « Aller-retour sur le jeu synthétique **et sur l'échantillon Legifacile** ». La fiche ne contredit donc pas la conception : elle est **incomplète**. Un test qui ne couvrirait que `synth-small` laisserait INV-08 prouvé à moitié, et la fiche lue seule ne le dirait pas.

### Ce que le test devra faire

Le test s'appellera `inv_08_legifacile_sample_queries_return_the_same_results_after_export_and_import`. Ce nom est **fixé ici** : c'est un contrat, au même titre que `inv_12_the_legifacile_sample_passes_the_acceptance_check` et `inv_09_legifacile_sample_queries_return_the_same_results_after_compaction` (ADR-0057, ADR-0062).

Il sera écrit par `@testeur`, puis commité avec `@dev`, dans la tâche `VERIFIER-ALLER-RETOUR-EXPORT`. Il sera marqué `#[ignore]` : c'est le seul mécanisme de saut autorisé (`docs/08`, ligne 43). Il demandera les trois mêmes prérequis que le test local d'ADR-0062 : `data/legifacile-sample/`, qui est gitignoré et dérive de la production Legifacile (ADR-0032) ; `data/models/mdenseon/` ; et `ORT_DYLIB_PATH`, parce que l'import passe par l'attache, donc par la recette, qui charge le modèle fp32 (ADR-0054). Sans l'un de ces prérequis, il **échoue** avec la commande qui le produit, jamais en silence.

### Ce que la CI fait aujourd'hui

Le job `rust-onnx` d'`.github/workflows/ci.yml` exécute, ligne 133 (reprise dans `docs/07-stack-et-deploiement.md`, ligne 140) :

```
cargo test --locked --workspace -- --include-ignored --skip inv_12_the_legifacile_sample_passes_the_acceptance_check --skip inv_09_legifacile_sample_queries_return_the_same_results_after_compaction
```

Il exclut **deux** tests, nommés. Le nouveau test serait donc exécuté par ce job, sans `data/legifacile-sample/`, sans modèle et sans `ORT_DYLIB_PATH`, et il échouerait à chaque push. La CI deviendrait rouge et la fusion de la phase serait bloquée.

### La règle qu'ADR-0062 fixe

ADR-0062, point 3, décide : « Aucun autre test n'est exclu, et aucune règle générale d'exclusion n'est créée. » Un troisième test local ne peut donc pas être ajouté sans une décision nouvelle. C'est l'objet de cet ADR, et c'est ce qui rend le point bloquant pour `@architecte`.

### Les textes que la décision rend faux

- **`docs/07-stack-et-deploiement.md`, ligne 140** : la commande n'a que deux `--skip`.
- **`docs/07-stack-et-deploiement.md`, ligne 143** : « Les tests de l'échantillon Legifacile sont les **deux exclus** » et la liste des deux noms. Faux dès le troisième test local.
- **`docs/08-invariants-et-tests.md`, ligne 48** : la liste des tests locaux nommés un par un n'en contient que deux. La règle elle-même est **conforme** : elle admet plusieurs exclusions nommées, seul l'exemple est incomplet.
- **`docs/phases/P2-encodeur-et-recette.md`, ligne 174**, note du 2026-10-25 ajoutée en application d'ADR-0062 : « la CI exclut **deux** tests de l'échantillon Legifacile ». Cette note datera de la clôture d'`AJOUTER-ORT-CI` sous ADR-0062 ; elle appelle une note de même forme pour le troisième.
- **ADR-0057, points 3 et 7** : « 19 tests ignorés, 1 test local », « 419 tests », « le seul exclu ». Déjà périmés par ADR-0062, ils le restent. Leur corps n'est pas réécrit.
- **ADR-0062, contexte et « À surveiller »** : son décompte (« 21 tests `#[ignore]`, dont 19 exécutés et 2 laissés locaux ») et sa phrase « les noms des **deux** tests exclus » deviennent incomplets. Son corps n'est pas réécrit : cet ADR fait foi pour ces chiffres et ces noms.

### Ce qui ne change pas

ADR-0057 et ADR-0062 décident que « l'échantillon Legifacile reste local ». Le principe est intact : aucun contenu de production n'entre en CI (ADR-0032), et `#[ignore]` reste le seul mécanisme de saut (`docs/08`, ligne 43). Cet ADR ne touche ni au seuil de recette, ni à l'artefact du modèle, ni à la politique de reprise du téléchargement (ADR-0059), ni au manifeste d'un lot exporté (ADR-0061).

### Ce que rien ne borne

Aucun texte ne borne la durée du nouveau test. ADR-0062 a écarté la borne par défaut (option 2ii) et renvoyé la durée à un relevé à J3. La règle 7 d'`AGENTS.md` interdit d'affirmer une durée sans rapport de `@mesureur` ; aucune mesure ne porte sur ce test, qui n'est pas encore écrit. Conséquence pratique déjà tirée par ADR-0062 : `@jalon` lance ces tests **en arrière-plan**, et sa consigne (`.opencode/agents/jalon.md`, ligne 18) le dit déjà pour « les tests d'invariant qui demandent l'échantillon Legifacile » — formulation qui couvre le troisième sans modification.

## Décision

**Un troisième test de l'échantillon Legifacile est admis, et la CI l'exclut par son nom, avec un troisième `--skip` explicite. Le test peut alors être commité avec son exclusion, dans le même travail. La durée du test n'est pas bornée par défaut : elle est relevée au moment de J3.**

Les deux points ci-dessous sont soumis au propriétaire. Chacun porte une recommandation ; rien n'est décidé ici.

### 1. Mécanisme d'exclusion : un troisième `--skip` explicite — **recommandé**

La commande du job `rust-onnx` devient :

```
cargo test --locked --workspace -- --include-ignored \
  --skip inv_12_the_legifacile_sample_passes_the_acceptance_check \
  --skip inv_09_legifacile_sample_queries_return_the_same_results_after_compaction \
  --skip inv_08_legifacile_sample_queries_return_the_same_results_after_export_and_import
```

Propriétés recherchées, toutes trois déjà voulues par ADR-0057, ADR-0062 et `docs/08` :

- **l'exclusion est nommée**, donc lisible et justifiable une par une, comme `docs/08` ligne 48 l'exige ;
- **l'oubli est bruyant** : un futur test de l'échantillon ajouté sans sa ligne `--skip` est exécuté en CI, échoue faute de données et rougit le job. Un oubli rend la CI **rouge, pas verte** ;
- **aucun saut silencieux** : le test n'est pas filtré par une règle générale, il est nommé, et `--skip` ne s'applique qu'à la commande, pas au code.

Cette option ne demande aucune modification de code de test et ne renomme rien : les deux noms déjà exclus restent des contrats, et le troisième aussi.

### 2. Durée du test lancé à la main et par `@jalon` : pas de borne par défaut — **recommandé**

Aucune mesure n'existe ; écrire une borne maintenant serait inventer un chiffre, ce que la règle 7 d'`AGENTS.md` interdit. La durée est donc relevée **au moment de J3**, quand le test tourne pour de vrai, et notée dans `docs/suivi.md` par l'orchestrateur. Aucune valeur par défaut n'est inscrite, ni dans `docs/08`, ni dans la consigne de `jalon`.

Dans les deux options, et c'est le point non négociable :

- `@jalon` lance le test **en arrière-plan**, puis relève sa durée et son résultat dans le rapport de jalon ;
- le test n'est jamais « sauté » faute de données : sans `data/legifacile-sample/`, sans modèle ou sans `ORT_DYLIB_PATH`, il échoue avec la commande qui les produit (`docs/08`, ligne 43) ;
- aucune affirmation de durée n'entre dans un document avant d'être relevée.

Si le propriétaire préfère une garde, l'option (i) la fournit : une mesure dédiée (« durée du test INV-08 sur l'échantillon Legifacile », le numéro MES restant à attribuer), puis une borne écrite dans `docs/08` et dans la consigne de `jalon`. Le coût est un rapport de mesure et une valeur à maintenir.

### 3. Ce qui n'est pas décidé ici

- Aucun autre test n'est exclu, et aucune règle générale d'exclusion n'est créée : le point 3 d'ADR-0062 reste vrai, et le restera après cet ADR.
- L'échantillon Legifacile ne sera ni publié, ni reconstitué, ni réduit pour tenir en CI : ADR-0032 et ADR-0057 restent en vigueur.
- Aucun mécanisme d'exclusion autre que `--skip` (feature Cargo, binaire séparé, variable d'environnement) n'est admis.
- La durée du job `rust-onnx` et son `timeout-minutes` ne changent pas : le test exclu ne s'y exécute pas.
- `docs/08`, ligne 16, est **inchangée** : elle décrit déjà la preuve attendue d'INV-08, échantillon compris.

### 4. Effet sur la tâche `VERIFIER-ALLER-RETOUR-EXPORT`

Le test **ne peut pas être commité seul**. Un commit qui l'ajoute sans le troisième `--skip` fait échouer le job `rust-onnx` au push suivant, faute de données. L'exclusion entre donc dans le même travail que le test, comme ADR-0062 l'a décidé pour le deuxième.

Tant qu'ADR-0063 est `Proposé`, la tâche reste `BLOQUE` : `docs/07`, `docs/08` et `ci.yml` ne peuvent pas être modifiés sans décision. La ligne d'écart correspondante revient à l'orchestrateur, qui tient `docs/suivi.md`.

## Conséquences

- **Positives** :
  - INV-08 est prouvé sur les requêtes réelles de l'échantillon Legifacile, comme `docs/08` ligne 16 le demande, sans faire entrer une donnée de production en CI ;
  - l'exclusion reste nommée, donc lisible, justifiable et vérifiable : `docs/08` ligne 48 et ADR-0057 sont respectés dans leur lettre comme dans leur esprit ;
  - un oubli d'exclusion rougit la CI au lieu de la laisser verte ;
  - aucune modification de code de test, aucun renommage, aucun nouveau mécanisme : un `--skip` de plus dans un fichier YAML et une ligne de commande dans `docs/07` ;
  - la durée du test devient un fait consigné à J3, sans chiffre inventé.
- **Négatives / coûts** :
  - chaque futur test de l'échantillon demandera une ligne `--skip` de plus, et un oubli coûtera un run rouge ;
  - la commande du job `rust-onnx` s'allonge encore ; elle est dupliquée dans `ci.yml` et dans `docs/07`, donc à corriger deux fois ;
  - le test local reste **non prouvé à la fusion** : une régression de l'export ou de l'import qui ne touche que les données réelles ne sera vue qu'au passage local ou à J3 ;
  - les décomptes d'ADR-0057, d'ADR-0062, de `docs/07` et de la note de `docs/phases/P2-encodeur-et-recette.md` vieillissent une fois de plus. Un ADR accepté n'est pas réécrit : cet ADR fait foi pour ces chiffres ;
  - sans borne, un blocage du test ne se voit que par l'absence de résultat dans le rapport de jalon.
- **À surveiller** :
  - **toute phrase qui cite un nombre de tests `#[ignore]`, exclus ou exécutés doit être relue** : c'est ce qui a rendu ce point nécessaire une première fois (ADR-0062), et il se répète ;
  - les noms des **trois** tests exclus sont des **contrats** : les renommer sans mettre à jour les trois `--skip` ferait exécuter un test local en CI, qui échouerait bruyamment ;
  - la durée relevée à J3 : si elle dépasse largement celle des deux autres tests locaux, ou si elle varie d'un passage à l'autre, rouvrir le point 2 et écrire une borne (option (i)) ;
  - la duplication de la commande entre `ci.yml` et `docs/07` : une divergence ferait décrire à la documentation autre chose que ce que la CI exécute ;
  - l'application effective d'ADR-0062 : ses points 2 à 7 ont été appliqués (la note datée de `docs/phases/P2-encodeur-et-recette.md` ligne 174 et la consigne de `jalon` en témoignent). Si l'un d'eux ne l'était pas, le corriger avant celui-ci.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| **(1a) Troisième `--skip` explicite, nommant le test** — recommandé | Un oubli de ligne est bruyant (CI rouge), l'exclusion reste nommée et justifiable une par une, et rien ne bouge dans le code de test. Coût : une ligne par test local. |
| **(1b) Un motif de nom commun filtré par un seul `--skip`** (sous-chaîne libtest) | Trois défauts, déjà constatés pour ADR-0062. (i) Un motif comme `legifacile_sample` exclurait aussi les tests anti-saut de l'échantillon, qui **doivent** tourner pour prouver `docs/08` ligne 43 — `inv_09_the_legifacile_sample_test_fails_without_its_prerequisites`, et le test équivalent qu'écrira `@testeur` pour INV-08. (ii) Il obligerait à renommer des noms qui sont des contrats (ADR-0057, ADR-0062). (iii) Il ferait un saut **silencieux** : un futur test de l'échantillon serait exclu sans que personne l'ait écrit, contrairement à `docs/08` ligne 48, qui exige de nommer « explicitement chaque test ». |
| **(1c) Un autre mécanisme** (feature Cargo, binaire de test séparé, variable d'environnement, `cfg`) | `docs/08` ligne 43 fait d'`#[ignore]` le seul mécanisme de saut autorisé ; un `cfg` ou une feature conditionnerait la **compilation** du test, donc il n'apparaîtrait plus dans le compte des tests, et le saut deviendrait invisible. Un binaire séparé déplacerait le test hors du workspace, donc hors de `cargo test --workspace`, sans décision sur qui le lance. |
| **Committer le test et corriger la CI au push suivant** | La CI serait rouge entre les deux commits, sur la branche de phase comme sur toute branche issue d'elle, et la fusion serait bloquée par un état intermédiaire connu. |
| **Faire tourner le test en CI sans son prérequis, et le laisser échouer** | Un test rouge à chaque push ne prouve rien et rend la CI inutilisable : on ne distingue plus une régression d'un prérequis manquant. |
| **Rendre le test silencieux quand les données manquent** (`if !path.exists() { return; }`) | Interdit par `docs/08` ligne 43. Un test qui saute sans le dire se lit comme vert. |
| **Publier ou réduire l'échantillon pour qu'il tienne en CI** | Contredit ADR-0032 : les données dérivent de la production. Un échantillon réduit ou anonymisé ne prouverait plus l'aller-retour sur les données réelles, qui est l'objet d'INV-08. |
| **Prouver INV-08 sur `synth-small` seulement, et s'en tenir à la fiche** | La fiche est incomplète, pas contraire : `docs/08` ligne 16 demande aussi l'échantillon Legifacile. S'en tenir à la fiche laisserait la moitié d'INV-08 non prouvée, et l'écart reviendrait à la relecture. |
| **(2i) Une mesure, puis une borne de commande** | Défendable, et à préférer si le propriétaire veut une garde contre un blocage. Écartée par défaut pour la simplicité : elle demande un rapport de mesure pour une seule durée, puis une valeur à maintenir dans deux documents, alors que J3 produira la même durée sans dispositif nouveau. La règle 7 d'`AGENTS.md` interdit d'écrire la borne avant le relevé. |
| **(2ii) Pas de borne, durée relevée à J3 et notée dans le suivi** — recommandé | Aucun chiffre inventé, aucun document de plus, aucun réglage à maintenir. Le seul coût est qu'un blocage se voit par l'absence de résultat dans le rapport de jalon, et non par une coupure nette. |
| **Lancer le test au premier plan depuis `@jalon`** | Les tests locaux de l'échantillon durent bien plus que le délai de 10 minutes d'un appel d'agent au premier plan : l'appel serait coupé avant la fin, et le rapport de jalon conclurait à un échec qui n'en est pas un. L'arrière-plan est obligatoire dans les deux options. |

## Modifications à appliquer après acceptation

Cette section décrit les modifications. **Aucune n'est appliquée par cet ADR** : elles le seront après acceptation du propriétaire, dans un commit qui cite `ADR-0063`. Les points 1 à 6 relèvent de `@greffier` ; le point 7 relève de `@dev`, avec le test.

### `.github/workflows/ci.yml`

1. **Job `rust-onnx`, étape `Test with ONNX Runtime`** (ligne 133) : ajouter le troisième `--skip inv_08_legifacile_sample_queries_return_the_same_results_after_export_and_import` à la commande, et une ligne de commentaire qui renvoie à cet ADR. Rien d'autre ne change : ni `timeout-minutes`, ni le cache, ni le téléchargement. Cette modification se fait **dans le même travail que le test**, jamais avant.

### `docs/07-stack-et-deploiement.md`

2. **« CI (GitHub Actions) »**, commande ligne 140 : ajouter le troisième `--skip`. La commande doit rester identique à celle de `ci.yml`.
3. **« CI (GitHub Actions) »**, ligne 143 : remplacer « les **deux exclus** » et la liste des deux noms par **les trois exclus**, nommés un par un : `inv_12_…`, `inv_09_…` et `inv_08_legifacile_sample_queries_return_the_same_results_after_export_and_import`. La justification reste celle d'ADR-0032, complétée par ONNX Runtime et le vrai modèle (ADR-0057). Citer `ADR-0057`, `ADR-0062` et `ADR-0063`.

### `docs/08-invariants-et-tests.md`

4. **« Sauts de test »**, ligne 48 : l'exemple devient trois cas nommés, et la phrase qui les justifie reste celle d'ADR-0032. La règle elle-même ne change pas.
5. **Ligne 16** : **inchangée.** Elle décrit déjà la preuve d'INV-08 sur l'échantillon Legifacile.
6. **(Option 2i seulement)** Ajouter la borne retenue pour la durée du test, avec le renvoi à la mesure. **(Option 2ii, recommandée)** Rien à ajouter : la durée est un fait de suivi, pas une règle.

### `docs/phases/P2-encodeur-et-recette.md`

7. **Fiche `AJOUTER-ORT-CI`**, note datée ligne 174, ajoutée en application d'ADR-0062 : elle dit « deux tests ». La compléter par une note du même type, ou la prolonger, pour dire que depuis `ADR-0063` la CI en exclut **trois**, nommés un par un, et qu'`ADR-0063` fait foi pour ce décompte. Ne pas réécrire les critères d'une tâche terminée.

### `docs/adr/`

8. **`docs/adr/README.md`** : passer la ligne d'ADR-0063 au statut décidé, et compléter la ligne d'ADR-0062 par « complété par ADR-0063, sans modification du corps ».
9. **ADR-0057 et ADR-0062** : **aucune modification de leur corps.** Leurs décomptes et leurs listes de noms gardent leur valeur d'origine ; cet ADR fait foi pour les chiffres et les noms à partir de son acceptation.

### `.opencode/agents/jalon.md`

Aucune modification. La ligne 18, ajoutée en application d'ADR-0062, dit déjà que « les tests d'invariant qui demandent l'échantillon Legifacile se lancent **en arrière-plan** » ; le troisième test entre dans cette formulation.

### Hors de ces documents

`crates/crible/tests/inv_08_export_import.rs` (nom de fichier indicatif, à l'appréciation de `@testeur`) relève de la tâche `VERIFIER-ALLER-RETOUR-EXPORT`, pas de cet ADR. `docs/suivi.md` reste tenu par l'orchestrateur : ligne d'écart de la tâche, statut de la tâche, durée relevée à J3.

## Références

- `docs/phases/P3-cycle-de-vie.md`, fiche `VERIFIER-ALLER-RETOUR-EXPORT`, lignes 114 à 119 : livrables, critères d'acceptation, invariant INV-08.
- `docs/08-invariants-et-tests.md` : ligne 16 (INV-08 se teste « sur le jeu synthétique et sur l'échantillon Legifacile »), ligne 43 (`#[ignore]` seul mécanisme de saut, échec franc sans prérequis), ligne 48 (exclusion nommée un par un), ligne 56 (`legifacile-sample`).
- `docs/07-stack-et-deploiement.md`, « CI (GitHub Actions) », commande ligne 140 et phrase ligne 143 sur les « deux exclus ».
- `.github/workflows/ci.yml`, job `rust-onnx`, ligne 133 : commande `cargo test … --include-ignored --skip inv_12_… --skip inv_09_…`.
- `docs/phases/P2-encodeur-et-recette.md`, fiche `AJOUTER-ORT-CI`, ligne 174 : note du 2026-10-25 prise en application d'ADR-0062.
- `.opencode/agents/jalon.md`, ligne 18 : tests de l'échantillon lancés en arrière-plan, aucune borne de durée.
- `docs/09-plan-de-developpement.md`, ligne 92 : `/jalon J<n>` lance les tests et la démo, sans délai prescrit.
- `AGENTS.md`, règle 7 (« Mesurer avant d'affirmer »).
- ADR-0062 (second test local, `--skip` explicite, pas de borne de durée ; point 3 : aucun autre test exclu) ; ADR-0057 (ONNX Runtime et vrai modèle en CI, échantillon Legifacile local) ; ADR-0059 (complète ADR-0057 sans modifier son corps, précédent de forme) ; ADR-0032 (accès à la production, données non publiées) ; ADR-0054 (chargement du modèle à l'attache, recette) ; ADR-0061 (manifeste d'un lot exporté, dont l'index de restauration).
