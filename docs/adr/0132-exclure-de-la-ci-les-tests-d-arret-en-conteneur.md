# ADR-0132 — Exclure de la CI les tests de l'arrêt propre en conteneur

- **Statut** : Accepté (par anticipation le 2026-11-03, sur délégation du propriétaire du 2026-11-02 ; confirmé par le propriétaire le 2026-11-03)
- **Date** : 2026-11-03
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, tâche VERIFIER-ARRET-PROPRE (phase P8), sur la recommandation de `@architecte` retenue telle quelle par l'orchestrateur, en application de la délégation du propriétaire du 2026-11-02
- **Remplace** : — (complète ADR-0057, ADR-0062, ADR-0063, ADR-0085 et ADR-0109, sans modifier le corps d'un ADR accepté)

La délégation du propriétaire du 2026-11-02 est citée telle quelle : « Si tu as des ADR a ecrire considere qu'ils sont accpeté par anticipation, tu n'as pas besoin de t'arrêter, on les valide officienlement ensemble quand tu as fini ».

**Erratum du 2026-11-03 (correction de renvois de ligne, sans réécriture d'aucune décision).** Les renvois de code du corps étaient périmés, les fichiers ayant bougé depuis la rédaction ; ils sont corrigés ci-dessous, et aucun argument, aucune décision et aucune alternative ne changent. Dans `.github/workflows/ci.yml`, le commentaire du job `rust-onnx` occupe `:171-182` et la commande, à l'étape « Test with ONNX Runtime », `:184` (et non `:171-174`) ; dans `crates/crible/tests/docker_stop_during_attach.rs`, le test INV-03 est en `:123-126`, le test INV-04 en `:280-287`, la fonction `image()` qui lit `CRIBLE_DOCKER_IMAGE` en `:806-808`, la consigne de lancement un test à la fois en `:39-44` et la règle de saut en `:46-48` ; le test non ignoré `the_docker_example_configuration_is_accepted`, du fichier `crates/crible-server/tests/example_config.rs`, est cité par son nom sans numéro de ligne. Le renvoi à `docs/suivi.md` ne porte plus de numéro de ligne : le journal de `VERIFIER-EQUIVALENCE-COMPACTION` y est cité par son nom.

## Contexte

### La fiche et ce qu'elle demande

La fiche `VERIFIER-ARRET-PROPRE` (`docs/phases/P8-docker-et-deploiement.md:149-154`) demande « aucune perte lors d'un `docker stop` ». Son livrable est un test qui lance un `docker stop` pendant une attache, puis vérifie qu'après redémarrage l'état est `G` ou `G+1` complet, que les analytics ont été vidées et que le job est terminé ou rejouable. Son critère d'acceptation est que « le test passe 10 fois de suite ». Elle cite INV-03 et INV-04, et dépend de `CONSTRUIRE-IMAGE-DOCKER` et d'`AGREGER-ANALYTICS`, tous deux faits.

### Les deux tests livrés, et leurs prérequis

La tâche ajoute deux tests dans `crates/crible/tests/docker_stop_during_attach.rs`, tous deux marqués `#[ignore]` :

- `inv_03_a_docker_stop_during_an_attach_leaves_one_complete_generation` (`:123-126`) ;
- `inv_04_a_docker_stop_during_an_attach_leaves_the_last_committed_generation` (`:280-287`).

Ils exigent **Docker** et une **image construite localement**, dont le nom est lu dans `CRIBLE_DOCKER_IMAGE`, `crible:dev` par défaut (`:806-808`). Le commentaire du test donne la commande de construction et estime cette construction à « about four minutes » (`:29-36`). Chaque test pilote ses propres conteneurs, donc ils se lancent un par un (`:39-44`).

Sans ces prérequis, les deux tests **échouent** ; ils ne sautent pas. C'est la règle d'ADR-0057, décision 4, écrite dans `docs/08-invariants-et-tests.md:43` : `#[ignore]` est le seul mécanisme de saut autorisé, et un test `#[ignore]` lancé par `--include-ignored` sans son prérequis échoue avec le message qui donne la commande à lancer. Le commentaire du fichier le dit aussi (`:46-48`), et c'est ce qui rend l'oubli d'exclusion bruyant plutôt que silencieux.

### Ce que la CI faisait avant cet ADR

Avant cet ADR, le job `rust-onnx` lance, sur `ubuntu-latest` et à chaque push comme à chaque PR :

```
cargo test --locked --workspace -- --include-ignored --skip inv_12_the_legifacile_sample_passes_the_acceptance_check --skip inv_09_legifacile_sample_queries_return_the_same_results_after_compaction --skip inv_08_legifacile_sample_queries_return_the_same_results_after_export_and_import --skip inv_12_the_reference_batch_acceptance_statistics_are_unchanged --skip api_and_cli_return_the_same_results_on_the_legifacile_sample
```

Cette commande est à `.github/workflows/ci.yml:184`, à l'étape « Test with ONNX Runtime », et recopiée à l'identique dans `docs/07-stack-et-deploiement.md:156` (la prose qui compte les exclusions est à `:159`). `--include-ignored` y fait donc tourner les deux nouveaux tests, sur un runner qui n'a **ni image, ni construction d'image** : `ci.yml` ne comporte aucune étape `buildx` ni `docker build`, et le workflow de publication de l'image est la tâche `PUBLIER-IMAGE`, pas encore faite (`docs/phases/P8-docker-et-deploiement.md:162-166`) ; la publication multi-arch est prévue sur un tag `v*` (`docs/07-stack-et-deploiement.md:161`). Le job `rust-onnx` échouerait donc sur les deux tests, à chaque push, sans qu'aucun défaut du moteur soit en cause.

### Les précédents, et ce que chacun a coûté

Chaque exclusion nouvelle a demandé son propre ADR, et l'exclusion est entrée dans le même travail que le test :

- ADR-0057, décision 4 : la règle anti-saut, et le premier test de l'échantillon Legifacile exclu par son nom ;
- ADR-0062 : deuxième `--skip` explicite ; sa décision 3, « Ce qui n'est pas décidé ici », écrit qu'« aucun autre test n'est exclu, et aucune règle générale d'exclusion n'est créée » ; sa décision 4 impose que l'exclusion entre dans le même travail que le test ;
- ADR-0063 : troisième `--skip`, sur le même modèle, avec la même règle du même commit (décision 4) ;
- ADR-0085 : quatrième `--skip` ;
- ADR-0109 : cinquième `--skip`, et rappel que ces noms sont des **contrats** — les renommer sans leur `--skip` ferait exécuter un test local en CI.

Le point bloquant est déjà connu du suivi : le journal de `VERIFIER-EQUIVALENCE-COMPACTION`, dans `docs/suivi.md`, le consigne pour cette tâche, où le test Legifacile aurait été exécuté par `--include-ignored` faute de `--skip`.

## Décision

**Les deux tests de l'arrêt propre restent locaux, et la CI les exclut par leur nom, avec deux `--skip` explicites de plus dans la commande du job `rust-onnx`.** Aucune règle générale d'exclusion n'est créée.

Précisions :

1. **Les deux noms sont ajoutés à la fin de la commande, dans cet ordre** : `inv_03_a_docker_stop_during_an_attach_leaves_one_complete_generation`, puis `inv_04_a_docker_stop_during_an_attach_leaves_the_last_committed_generation`. Le nombre de tests exclus par le job `rust-onnx` passe de **cinq à sept**.
2. **Ces noms sont des contrats**, comme les cinq déjà exclus. Renommer un test sans renommer son `--skip` le ferait exécuter en CI, où l'image manque, et rougirait le job : l'oubli reste bruyant, ce qui est la propriété voulue par ADR-0057.
3. **Aucune règle générale d'exclusion n'est créée**, et les deux motifs d'exclusion ne sont pas fusionnés. `--skip` filtre par sous-chaîne sur **tous** les tests de la commande, y compris non ignorés : un `--skip docker` exclurait en silence `the_docker_example_configuration_is_accepted` (`crates/crible-server/tests/example_config.rs`), qui n'est pas marqué `#[ignore]` et doit continuer de tourner en CI. C'est la raison déjà écrite par ADR-0062 et rappelée par ADR-0109.
4. **Le commentaire du workflow distingue désormais deux motifs.** Le commentaire de `ci.yml:171-182` ne parle plus d'une seule famille : il nomme les cinq tests de l'échantillon Legifacile, dont les données dérivent de la production (ADR-0032), et les deux tests de l'arrêt propre, qui demandent Docker et une image construite localement (cet ADR).
5. **Les deux `--skip` entrent dans le même commit que les tests** (ADR-0062, décision 4 ; ADR-0063, décision 4). Un commit qui ajoute les tests sans les exclusions fait échouer le job `rust-onnx` au push suivant.
6. **C'est une règle d'attente, pas une exclusion accordée d'avance.** Un futur test `#[ignore]` qui exigera Docker — par exemple pour `COMPOSE-EXEMPLE` (`docs/phases/P8-docker-et-deploiement.md:156-160`) — demandera sa propre décision, comme chaque exclusion précédente.
7. **Rien d'autre ne change dans le workflow** : ni `timeout-minutes`, ni le cache, ni le téléchargement de l'artefact du modèle, ni les étapes `failpoints`.
8. **Ce qui n'est pas décidé ici** : la construction d'une image en CI, l'usage d'un service Docker sur le runner, une commande de test propre aux tests de conteneur, et le sort des tests Docker des phases suivantes.

### Documents mis à jour par cet ADR

- `docs/07-stack-et-deploiement.md:156` : la commande du job `rust-onnx` gagne les deux `--skip`, et la prose de `:159` qui compte « cinq » exclusions est réécrite (sept, deux motifs).
- `docs/08-invariants-et-tests.md:48` : la liste des tests que la CI laisse locaux gagne les deux noms, avec leur motif (Docker et image locale) et le renvoi à cet ADR.
- `.github/workflows/ci.yml` : la commande et son commentaire (`:171-182`, commande `:184`) sont modifiés par `@dev`, dans le même commit que les tests ; `@greffier` n'y touche pas.

## Conséquences

- **Positives** :
  - le job `rust-onnx` redevient vert sur un runner sans Docker : les deux tests restent locaux sans être renommés ni affaiblis ;
  - chaque exclusion reste nommée, justifiable une par une et visible dans le diff ; un oubli rend la CI rouge, pas verte ;
  - aucun mécanisme nouveau : deux `--skip` de plus dans un fichier YAML et une commande de plus dans `docs/07` ;
  - le test non ignoré `the_docker_example_configuration_is_accepted` continue de tourner, ce qu'une règle générale aurait cassé en silence ;
  - INV-03 et INV-04 de l'arrêt propre restent prouvés, à la main et à chaque jalon, sur la machine qui a l'image.
- **Négatives / coûts** :
  - la commande du job `rust-onnx` s'allonge de deux `--skip`, et elle est dupliquée entre `.github/workflows/ci.yml` et `docs/07` ; les deux copies doivent rester identiques ;
  - la CI ne prouve plus ces deux tests à chaque push : une régression de l'arrêt propre ne sera vue qu'au lancement local, par `@jalon` ou en fin de phase ;
  - le commentaire du workflow porte maintenant deux motifs d'exclusion, donc une explication plus longue ;
  - chaque futur test de conteneur coûtera un ADR et une ligne `--skip` de plus.
- **À surveiller** :
  - une dérive entre les deux copies de la commande (`ci.yml` et `docs/07`) ;
  - le renommage d'un des deux tests sans son `--skip` : le job rougirait à chaque push ;
  - une exclusion de plus demandée sans ADR : le précédent de 0062, 0063, 0085, 0109 et de cet ADR est explicite, chaque ligne a sa décision ;
  - la durée du job `rust-onnx` : ces deux tests n'y tournent plus, mais un futur test ajouté en CI pourrait la changer.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| **Un motif commun, par exemple `--skip docker`** | `--skip` filtre par sous-chaîne sur tous les tests, y compris non ignorés : `the_docker_example_configuration_is_accepted` (`crates/crible-server/tests/example_config.rs`) serait exclu en silence, alors qu'il tourne aujourd'hui et doit continuer. Le saut silencieux est exactement ce qu'ADR-0057 et `docs/08:43` interdisent. |
| **Un préfixe de nom commun aux tests de conteneur** (`docker_` devant `inv_03_…`, ou un `#![cfg]` de fichier) | Il faudrait renommer des noms déjà commités, et un préfixe resterait un filtre par sous-chaîne : la même famille d'exclusion silencieuse reviendrait au premier test qui porterait le préfixe sans devoir être exclu. ADR-0062 a écarté cette forme pour les tests de l'échantillon. |
| **Une feature cargo** qui sort le test de la compilation | Le test disparaîtrait de la compilation et du compte des tests exécutés sur le runner comme en local : `docs/08:43` fait d'`#[ignore]` le seul mécanisme de saut autorisé, et ADR-0062 le redit pour toute exclusion autre que `--skip`. |
| **Construire l'image en CI et lancer les deux tests sur le runner** | C'est une décision distincte, non couverte ici : elle coûte environ quatre minutes de construction (`crates/crible/tests/docker_stop_during_attach.rs:29-36`), un daemon Docker par push, et un temps de job en hausse pour deux tests qui pilotent des conteneurs. Rien dans la fiche ne la demande. |
| **Marquer les tests `#[ignore]` sans `--skip`** | C'est l'état actuel, et il échoue : `--include-ignored` les lance, et ils n'ont pas d'image sur `ubuntu-latest`. |
| **Renoncer aux deux tests** | La fiche exige la vérification de « aucune perte lors d'un `docker stop` » ; l'arrêt propre d'ADR-0099 et ADR-0104 resterait sans preuve de bout en bout. |
| **Une commande `cargo test` séparée pour les tests de conteneur** | Elle ajouterait une étape et un second binaire de test au job, pour deux noms ; `--skip` fait déjà exactement ce travail, avec une ligne par test. |
| **Étendre l'exclusion accordée ici à `COMPOSE-EXEMPLE`** | Ce serait une exclusion donnée d'avance, sans test à nommer ; la fiche `COMPOSE-EXEMPLE` n'existe pas encore, et le nombre et les noms des tests qu'elle ajoutera sont inconnus. |

## Références

- Fiche : `docs/phases/P8-docker-et-deploiement.md:149-154` (`VERIFIER-ARRET-PROPRE`), `:156-166` (`COMPOSE-EXEMPLE`, `PUBLIER-IMAGE`).
- Tests : `crates/crible/tests/docker_stop_during_attach.rs` (`:29-48` prérequis et règle de saut, `:123-126` test INV-03, `:280-287` test INV-04, `:806-808` `CRIBLE_DOCKER_IMAGE`).
- CI : `.github/workflows/ci.yml:171-182` (commentaire du job `rust-onnx`) et `:184` (commande de l'étape « Test with ONNX Runtime ») ; `docs/07-stack-et-deploiement.md:156` (commande recopiée) et `:159` (prose des exclusions).
- Conception : `docs/08-invariants-et-tests.md:43` (règle normative du seul saut autorisé) et `:48` (tests nommés que la CI laisse locaux).
- Suivi : journal de `VERIFIER-EQUIVALENCE-COMPACTION`, dans `docs/suivi.md` (même point bloquant de `--include-ignored`).
- ADR : ADR-0032 (données de production hors CI), ADR-0057 (décision 4, règle anti-saut), ADR-0062 (décision 3 : aucune règle générale ; décision 4 : exclusion dans le même travail), ADR-0063 (décision 4, même règle), ADR-0085 (quatrième `--skip`), ADR-0099 et ADR-0104 (arrêt propre du serveur et jobs d'écriture), ADR-0109 (cinquième `--skip`, noms des tests comme contrats), ADR-0118 (vidage des analytics à l'arrêt).
