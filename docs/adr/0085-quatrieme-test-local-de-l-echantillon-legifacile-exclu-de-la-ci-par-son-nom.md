# ADR-0085 — Quatrième test local de l'échantillon Legifacile, exclu de la CI par son nom

- **Statut** : Accepté (décision du propriétaire le 2026-10-28, option (1a) telle que proposée)
- **Date** : 2026-10-28 (proposition et acceptation le même jour)
- **Décideurs** : propriétaire
- **Décision** : accepté le 2026-10-28, option (1a) telle que proposée. Un quatrième test de l'échantillon Legifacile, `inv_12_the_reference_batch_acceptance_statistics_are_unchanged`, reste local ; la CI l'exclut par son nom, avec un quatrième `--skip` explicite dans la commande du job `rust-onnx`. Aucun autre test n'est exclu, aucune règle générale d'exclusion n'est créée, et le corps d'ADR-0057, d'ADR-0062 et d'ADR-0063 n'est pas modifié.
- **Proposé par** : agent greffier, sur demande du propriétaire, à la suite du run de CI distante `40665513025` et du verdict `ECART` de `@architecte` du 2026-10-28. Tâche d'origine : `ACCELERER-RECETTE` (phase P4, commit `c62e82f`), dont le test est la cause du run rouge.
- **Remplace** : — (complète ADR-0057, ADR-0062 et ADR-0063, qui restent en vigueur et `Accepté` ; aucun de leurs corps n'est modifié, comme ADR-0059 puis ADR-0063 l'ont déjà fait pour ADR-0057)

## Contexte

### Ce que la CI distante observe

Le run de CI distante `40665513025` (workflow `CI`, push de la branche `phase/P4-recherche-rapide`, commit `cd6617c`, le 2026-10-28) échoue dans le job `Rust with ONNX Runtime`, à l'étape `Test with ONNX Runtime`.

Le test `inv_12_the_reference_batch_acceptance_statistics_are_unchanged` (`crates/crible/tests/recette_import.rs:1527`) panique : `data/legifacile-sample/configs/dole.json` est absent. L'échantillon Legifacile est gitignoré et dérive de la production (ADR-0032) ; il n'existe pas en CI, et il ne doit pas y être publié.

L'oubli n'a été vu qu'à J4, parce que la branche de phase P4 n'avait jamais été poussée avant ce run. Le même test passe en local, avec l'échantillon : le rapport de J4 le compte parmi les 11 tests verts en `--release` avec ONNX Runtime, sous INV-12.

### D'où vient le test

Le test a été écrit par `ACCELERER-RECETTE` (commit `c62e82f`), en application d'ADR-0073, décision 2 : les statistiques de la recette du lot de référence (`sample`, `min_cosine`, `p1`, `p50`) sont épinglées à `1e-6` près. ADR-0073 couvre donc l'**existence** du test. Il ne dit rien de sa **place** en CI, et c'est là que le point bloque.

### Ce que la CI exclut aujourd'hui

Le job `rust-onnx` d'`.github/workflows/ci.yml` (ligne 140, reprise dans `docs/07-stack-et-deploiement.md:141`) exclut **trois** tests, nommés un par un :

```
cargo test --locked --workspace -- --include-ignored \
  --skip inv_12_the_legifacile_sample_passes_the_acceptance_check \
  --skip inv_09_legifacile_sample_queries_return_the_same_results_after_compaction \
  --skip inv_08_legifacile_sample_queries_return_the_same_results_after_export_and_import
```

Ces trois exclusions viennent d'ADR-0057, d'ADR-0062 et d'ADR-0063. ADR-0062, décision 3, fixe la règle : « Aucun autre test n'est exclu, et aucune règle générale d'exclusion n'est créée. » ADR-0063, décision 3, la confirme : « le point 3 d'ADR-0062 reste vrai, et le restera après cet ADR. » ADR-0063, dans ses conséquences, prévoit le coût : « chaque futur test de l'échantillon demandera une ligne `--skip` de plus ».

Le quatrième test local demanderait donc une quatrième exclusion. Le verdict de `@architecte` du 2026-10-28 est `ECART` : cette exclusion demande une décision nouvelle, et c'est l'objet de cet ADR.

### D'où vient la dépendance à l'échantillon

Elle vient du **code**, pas de la conception. Le test passe par `attach_reference_batch` (`crates/crible/tests/recette_import.rs:1397`), puis par `legifacile_sample_paths` (`:1330`), qui exigent `data/legifacile-sample/configs/dole.json` et `data/legifacile-sample/batches/legifacile-sample-dole-v1`. Sans eux, le test échoue franchement, avec le chemin manquant et la commande qui produit les données (`docs/08-invariants-et-tests.md:43`).

Le modèle est déjà en CI (ADR-0057) ; seul le lot y manque.

### Ce qui ne change pas

- L'échantillon Legifacile reste local : aucun contenu de production n'entre en CI (ADR-0032), et `#[ignore]` reste le seul mécanisme de saut (`docs/08:43`).
- Le contrat de la recette est intact : `acceptance_sample`, `acceptance_min_cosine` à 0,999, fp32, défauts d'ONNX Runtime (ADR-0052, ADR-0054, ADR-0067).
- L'épinglage des statistiques à `1e-6` reste décidé par ADR-0073, décision 2.
- Le test `inv_12_the_legifacile_sample_test_fails_without_its_prerequisites` (`:1359`) reste exécuté en CI : il prouve qu'un test de l'échantillon échoue, bruyamment, sans ses prérequis.

## Décision

**Un quatrième test de l'échantillon Legifacile est admis, et la CI l'exclut par son nom, avec un quatrième `--skip` explicite. Le test reste local, et il continue de prouver INV-12 à chaque jalon.**

Les deux points ci-dessous sont soumis au propriétaire. Chacun porte une recommandation ; rien n'est décidé ici.

### 1. Mécanisme d'exclusion : un quatrième `--skip` explicite — **recommandé par `@architecte`**

La commande du job `rust-onnx` devient :

```
cargo test --locked --workspace -- --include-ignored \
  --skip inv_12_the_legifacile_sample_passes_the_acceptance_check \
  --skip inv_09_legifacile_sample_queries_return_the_same_results_after_compaction \
  --skip inv_08_legifacile_sample_queries_return_the_same_results_after_export_and_import \
  --skip inv_12_the_reference_batch_acceptance_statistics_are_unchanged
```

Propriétés recherchées, déjà voulues par ADR-0057, ADR-0062 et `docs/08:48` :

- **l'exclusion est nommée**, donc lisible et justifiable une par une ;
- **l'oubli est bruyant** : un futur test de l'échantillon ajouté sans sa ligne `--skip` serait exécuté en CI, échouerait faute de données et rougirait le job. C'est exactement ce qui s'est produit ici ;
- **aucun saut silencieux** : le test n'est pas filtré par une règle générale, il est nommé, et `--skip` ne s'applique qu'à la commande, pas au code.

Le nom du test est un contrat, comme les trois déjà exclus : le renommer sans mettre à jour son `--skip` le ferait exécuter en CI.

### 2. Ce qui n'est pas décidé ici

- Aucun autre test n'est exclu, et aucune règle générale d'exclusion n'est créée : le point 3 d'ADR-0062 et le point 3 d'ADR-0063 restent vrais, et le resteront après cet ADR.
- L'échantillon Legifacile ne sera ni publié, ni reconstitué, ni réduit pour tenir en CI (ADR-0032, ADR-0057, ADR-0062, ADR-0063).
- Aucun mécanisme d'exclusion autre que `--skip` (feature Cargo, binaire séparé, variable d'environnement, `cfg`) n'est admis (`docs/08:43`).
- Le contrat de la recette, le seuil de recette, l'artefact du modèle et la politique de reprise du téléchargement (ADR-0059) ne changent pas.
- La durée du job `rust-onnx` et son `timeout-minutes` ne changent pas : le test exclu ne s'y exécute pas.
- `docs/08-invariants-et-tests.md:16`, qui décrit la preuve attendue d'INV-12, reste inchangée.

## Conséquences

- **Positives** :
  - la CI redevient verte sur la branche de phase, et la fusion de P4 n'est plus bloquée par un état intermédiaire connu ;
  - INV-12 reste prouvé, avec le vrai lot de référence de l'échantillon, comme ADR-0073 décision 2 le demande ;
  - l'exclusion reste nommée, donc lisible et vérifiable : `docs/08:48` et ADR-0057 sont respectés dans leur lettre comme dans leur esprit ;
  - la correction est petite et sans code : une ligne de commande dans un YAML et dans `docs/07`, plus des notes datées.
- **Négatives / coûts** :
  - chaque futur test de l'échantillon demandera une ligne `--skip` de plus, et un oubli coûtera un run rouge ; c'est le coût déjà annoncé par ADR-0063 ;
  - la commande du job `rust-onnx` s'allonge encore ; elle est dupliquée dans `ci.yml` et dans `docs/07`, donc à corriger deux fois ;
  - le test reste **non prouvé à la fusion** : une régression des statistiques de la recette ne sera vue qu'au passage local ou à J4 ;
  - les décomptes d'ADR-0057, d'ADR-0062, d'ADR-0063, de `docs/07`, de `docs/08` et des notes de `docs/phases/P2-encodeur-et-recette.md` vieillissent une fois de plus. Un ADR accepté n'est pas réécrit : cet ADR fait foi pour ces chiffres.
- **À surveiller** :
  - **toute phrase qui cite un nombre de tests `#[ignore]`, exclus ou exécutés doit être relue** : c'est ce qui a rendu ce point nécessaire trois fois ;
  - la duplication de la commande entre `ci.yml` et `docs/07` : une divergence ferait décrire à la documentation autre chose que ce que la CI exécute ;
  - les noms des **quatre** tests exclus sont des **contrats** ;
  - si un cinquième test de l'échantillon apparaît, ne pas improviser : rouvrir cet ADR plutôt que d'ajouter une exclusion sans décision.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| **(1a) Quatrième `--skip` explicite, nommant le test** — recommandé par `@architecte` | Un oubli de ligne est bruyant (CI rouge), l'exclusion reste nommée et justifiable une par une, et rien ne bouge dans le code de test. Coût : une ligne par test local. |
| **(1b) Un motif commun d'exclusion** (par exemple la sous-chaîne `legifacile_sample`) | Déjà écartée par ADR-0062 et ADR-0063, pour trois raisons. De plus, `--skip` filtre par sous-chaîne : un tel motif exclurait aussi `inv_12_the_legifacile_sample_test_fails_without_its_prerequisites`, qui doit rester en CI pour prouver `docs/08:43`. Enfin, il ferait un saut silencieux, contrairement à `docs/08:48`, qui exige de nommer chaque test explicitement. |
| **(2) Un lot de référence versionné à la place de l'échantillon, pour garder le test en CI** | Trois obstacles. ADR-0032 interdit de publier un lot dérivé de la production, et ADR-0062 et ADR-0063 ont écarté une réduction de l'échantillon pour la CI. Un lot synthétique donnerait d'autres valeurs épinglées, donc un autre contrat. Et l'égalité à `1e-6` entre macOS et Linux n'est pas mesurée (inconnues d'ADR-0057). Ce serait une tâche avec mesure, pas une correction de CI. |
| **(3) Retirer le test** | Le livrable (b) d'ADR-0073 dont il venait (l'encodage par lots) a été retiré par ADR-0073, décision 1, mais la décision 2 garde l'épinglage des statistiques. Retirer le test contredirait ADR-0073, décision 2, et laisserait INV-12 sans preuve sur le lot de référence. |
| **Laisser la CI rouge et corriger plus tard** | La branche de phase resterait rouge, la fusion de P4 serait bloquée, et aucune autre branche issue d'elle ne pourrait être poussée sans le même échec. |
| **Rendre le test silencieux quand les données manquent** (`if !path.exists() { return; }`) | Interdit par `docs/08:43`. Un test qui saute sans le dire se lit comme vert. |
| **Faire tourner le test en CI sans son prérequis, et le laisser échouer** | Un test rouge à chaque push ne prouve rien et rend la CI inutilisable : on ne distingue plus une régression d'un prérequis manquant. |

## Modifications à appliquer après acceptation

Cette section décrit les modifications. **Aucune n'est appliquée par cet ADR** : elles le seront après acceptation du propriétaire, dans un commit qui cite `ADR-0085`, et par une tâche dédiée.

La correction se fera dans une petite tâche hors plan de P4, avant la validation de J4, avec le pipeline complet (nom proposé : `EXCLURE-QUATRIEME-TEST-ECHANTILLON`).

### `.github/workflows/ci.yml`

1. **Job `rust-onnx`, étape `Test with ONNX Runtime`** (ligne 140) : ajouter `--skip inv_12_the_reference_batch_acceptance_statistics_are_unchanged` à la commande. Le commentaire de la ligne 137, « The three Legifacile sample tests », devient « The four Legifacile sample tests », et le renvoi cite `ADR-0085`. Rien d'autre ne change : ni `timeout-minutes`, ni le cache, ni le téléchargement.

### `docs/07-stack-et-deploiement.md`

2. **« CI (GitHub Actions) »**, commande ligne 141 : ajouter le quatrième `--skip`. La commande doit rester identique à celle de `ci.yml`.
3. **« CI (GitHub Actions) »**, ligne 144 : remplacer « les **trois exclus** » et la liste des trois noms par **les quatre exclus**, nommés un par un, le quatrième étant `inv_12_the_reference_batch_acceptance_statistics_are_unchanged`. La justification reste celle d'ADR-0032, complétée par ONNX Runtime et le vrai modèle (ADR-0057). Citer `ADR-0085`.

### `docs/08-invariants-et-tests.md`

4. **« Sauts de test »**, ligne 48 : l'exemple devient quatre cas nommés, et la phrase qui les justifie reste celle d'ADR-0032. La règle elle-même ne change pas.
5. **Ligne 16** : **inchangée.** Elle décrit déjà la preuve d'INV-12.

### `docs/phases/`

6. **`P4-recherche-rapide.md`**, fiche `ACCELERER-RECETTE` : ajouter une note datée, sous la forme des notes d'ADR-0062 et d'ADR-0063, disant que le test écrit en application d'ADR-0073 décision 2 reste local, que la CI l'exclut par son nom depuis `ADR-0085`, et que `ADR-0085` fait foi pour ce décompte. Ne pas réécrire les critères d'une tâche terminée.
7. **`P2-encodeur-et-recette.md`**, fiche `AJOUTER-ORT-CI`, après la note du 2026-10-25 prise en application d'ADR-0063 (ligne 175) : ajouter une note du même type, disant que depuis `ADR-0085` la CI exclut **quatre** tests de l'échantillon Legifacile, nommés un par un.

### `docs/adr/`

8. **`docs/adr/README.md`** : passer la ligne d'ADR-0085 au statut décidé, et compléter la ligne d'ADR-0063 par « complété par ADR-0085 », sans modification du corps.
9. **ADR-0057, ADR-0062 et ADR-0063** : **aucune modification de leur corps.** Leurs décomptes et leurs listes de noms gardent leur valeur d'origine ; cet ADR fait foi pour les chiffres et les noms à partir de son acceptation.

### Hors de ces documents

Aucune modification de code. `crates/crible/tests/recette_import.rs` n'est pas touché : le test existe déjà, seule son exécution en CI change. `docs/suivi.md` reste tenu par l'orchestrateur.

## Références

- Run de CI distante `40665513025`, workflow `CI`, commit `cd6617c`, 2026-10-28 : job `Rust with ONNX Runtime`, étape `Test with ONNX Runtime`, échec sur `data/legifacile-sample/configs/dole.json` absent.
- `crates/crible/tests/recette_import.rs` : `:29` (table des tests), `:1330` (`legifacile_sample_paths`), `:1359` (`inv_12_the_legifacile_sample_test_fails_without_its_prerequisites`), `:1397` (`attach_reference_batch`), `:1527` (`inv_12_the_reference_batch_acceptance_statistics_are_unchanged`).
- `.github/workflows/ci.yml` : `:137` (commentaire « The three Legifacile sample tests »), `:140` (commande à trois `--skip`).
- `docs/07-stack-et-deploiement.md` : `:141` (commande), `:144` (« les trois exclus » et sa liste).
- `docs/08-invariants-et-tests.md` : `:16` (preuve d'INV-12), `:43` (`#[ignore]` seul mécanisme de saut, échec franc sans prérequis), `:48` (exclusion nommée un par un).
- `docs/phases/P4-recherche-rapide.md`, fiche `ACCELERER-RECETTE` (`:111-129`), dont les critères d'acceptation `:124` exigent l'égalité des statistiques à `1e-6`.
- `docs/phases/P2-encodeur-et-recette.md` : `:174` (note après ADR-0062), `:175` (note après ADR-0063).
- Rapport de J4 et INV-12 : 11 tests verts en `--release` avec ONNX Runtime, échantillon présent.
- ADR-0073 (recette sans encodage par lots ; décision 2 : statistiques épinglées à `1e-6`) ; ADR-0057 (ONNX Runtime et vrai modèle en CI, échantillon Legifacile local) ; ADR-0062 (second `--skip` explicite, aucun autre test exclu) ; ADR-0063 (troisième `--skip` explicite, point 3 d'ADR-0062 maintenu) ; ADR-0059 (précédent de forme : complète ADR-0057 sans modifier son corps) ; ADR-0032 (accès à la production, données non publiées) ; ADR-0052 et ADR-0054 (précision, seuil et bornes de la recette) ; ADR-0067 (recette dans le chemin de l'attache, `wait_timeout_ms`).
- Verdict de `@architecte` du 2026-10-28 : `ECART`, une quatrième exclusion demande une décision nouvelle.
