# ADR-0057 — ONNX Runtime et vrai modèle mDenseOn en intégration continue

- **Statut** : Proposé
- **Date** : 2026-10-24
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, sur demande du propriétaire, à la suite du rapport de jalon J2 (`docs/jalons/J2-rapport.md`, §7) et des décisions du propriétaire du 2026-10-24. Révisé le même jour, avant acceptation : le propriétaire a déplacé l'hébergement de l'artefact de modèle d'une Release GitHub vers un dépôt de modèle Hugging Face dédié.
- **Remplace** : —

## Contexte

### Ce que le jalon J2 laisse non prouvé

Le rapport de jalon J2 (`docs/jalons/J2-rapport.md`, §7) constate deux faits :

- **aucun run de CI n'existe sur `phase/P2-encodeur-et-recette`** : la dernière CI verte date de J1, et tous les chiffres du rapport viennent d'un Mac arm64 ;
- **la CI n'embarque pas ONNX Runtime** : 20 tests Rust sont marqués `#[ignore]` et ne sont prouvés qu'en local. Le rapport les compte ainsi : 400 tests passent en CI, 20 sont ignorés ; avec `ORT_DYLIB_PATH`, 420 passent, en 3 min 49 s (229 s) sur la machine de mesure.

Une régression du chargement du modèle, de la parité numérique ou du chemin d'attache ONNX ne serait donc vue qu'au prochain passage local, pas à la fusion. La CI ne vérifie pas non plus que le projet compile et s'exécute sur Linux avec ONNX Runtime, alors que le déploiement est Linux (`docs/07-stack-et-deploiement.md`, « Docker »).

### Répartition des 20 tests ignorés

Le verdict de `@architecte` du 2026-10-24 répartit ces tests selon leur prérequis réel :

| Fichier | Nombre | Prérequis | Remarque |
|---|---:|---|---|
| `crates/crible-encode/tests/encoder_requetes.rs` | 5 | ONNX Runtime et modèle factice versionné | Cache de requêtes, prompt dans `max_tokens`, troncature déclarée, repli fp32, deux précisions |
| `crates/crible/tests/query_cli.rs` | 2 | ONNX Runtime, modèle factice et `synth-small` | `crible search --query` de bout en bout |
| `crates/crible/tests/lot_en_ligne.rs` | 1 | ONNX Runtime et modèle factice | 100 documents ingérés en ligne |
| `crates/crible/tests/controler_longueurs.rs` | 3 | ONNX Runtime et modèle factice | INV-11 côté ONNX Runtime |
| `crates/crible/tests/recette_import.rs` | 4 | ONNX Runtime et modèle factice | Recette d'import et INV-12 sans données de production |
| `crates/crible-encode/tests/charger_modele.rs` | 3 | ONNX Runtime et **vrai modèle** | `loads_a_model_once_per_fingerprint`, `loads_a_model_once_when_callers_race`, `matches_the_python_fp32_vectors_within_1e_4` |
| `crates/crible-core/tests/model_config.rs` | 1 | **vrai `model.json`** | `real_model_json_with_export_is_accepted` |
| `crates/crible/tests/recette_import.rs` | 1 | **échantillon Legifacile** et vrai modèle | `inv_12_the_legifacile_sample_passes_the_acceptance_check` |

Total : **20 tests**. Quinze ne demandent qu'ONNX Runtime et le modèle factice versionné `crates/crible-encode/tests/fixtures/fake-model/` (ou le jeu `synth-small`, généré par `tools/`) ; trois demandent le vrai graphe ; un ne demande que le vrai `model.json` ; un reste local, parce que ses données dérivent de la production Legifacile (ADR-0032).

Le test de parité `matches_the_python_fp32_vectors_within_1e_4` compare le vrai modèle à **20 textes et 20 × 768 composantes fp32 versionnés** dans `crates/crible-encode/tests/fixtures/mdenseon/`. Il ne demande donc ni Python, ni torch, ni export en CI.

### Textes contredits par la décision

Trois textes décrivent l'état actuel et deviendraient faux ou incomplets :

1. **`docs/07-stack-et-deploiement.md`, « Côté Python »** : « Ce groupe n'est pas installé par un `uv sync` par défaut, donc la CI ne l'installe pas ; il s'installe avec `uv sync --group export` ». La phrase reste vraie pour le groupe `export`, mais elle devient incomplète : un second groupe, `ci`, contiendra ONNX Runtime sans torch.
2. **`docs/07-stack-et-deploiement.md`, « CI (GitHub Actions) »** : « Les jeux de données lourds ne tournent pas en CI : les mesures se font sur la machine cible et sont consignées dans `docs/mesures/` ». L'intention — ne pas générer ni mesurer de jeux lourds en CI — reste valable. La phrase doit distinguer les **jeux de données** des **artefacts de modèle déjà exportés**, sinon elle interdit à tort le téléchargement du modèle.
3. **ADR-0049** (remplacé par ADR-0050) et **ADR-0050, point 3** : tous deux décrivent une CI sans torch, ce qui reste vrai, mais aucun ne prévoit un job qui exécute ONNX Runtime. ADR-0049 reste exact pour le groupe `export` ; il n'a pas à être réécrit.

### Le défaut qui rend le statu quo dangereux

`crates/crible/tests/recette_import.rs` (vers les lignes 1333-1352) fait `return` lorsque `data/legifacile-sample/` ou `data/models/mdenseon/` manque. Lancé avec `--include-ignored` sur une machine sans ces dossiers, ce test **passe au vert sans rien prouver**. C'est un saut silencieux, pas un saut explicite.

Les tests de `charger_modele.rs` et `model_config.rs` font l'inverse : ils paniquent avec la commande d'export à lancer. C'est le bon comportement, et il doit devenir la règle.

### Ce qui est disponible

- **ONNX Runtime** : la roue pip `onnxruntime==1.31.0` fournit la bibliothèque Linux x86_64. C'est la version qui figure dans le champ `export` du `model.json` mesuré (ADR-0050) et celle qui a produit les hash de référence. Le binaire `ort` est compilé avec `load-dynamic` (`docs/07`, « Dépendances Rust »), donc la bibliothèque vient de l'extérieur et `ORT_DYLIB_PATH` doit la désigner.
- **Modèle factice** : `crates/crible-encode/tests/fixtures/fake-model/` est versionné. Il ne demande ni téléchargement ni données de production.
- **Vrai modèle exporté** : `model.onnx` fp32 (1,1 Gio), `model.int8.onnx` (293 Mio), `tokenizer.json` et `model.json` existent dans `data/models/mdenseon/`, gitignoré. `tools/README.md`, « Révision et hash mesurés », donne les hash complets mesurés le 2026-10-23 sur Mac arm64 :

  ```
  model.onnx         blake3:7b6eb68099dbeac6826e282dd575aec766dfbe1bedac143bae094b12f82094a2
  model.int8.onnx    blake3:0534b4ee028cfc4a6a7ca9dcca7a7c908110372033dab33a4f75fbe37d5941a5
  tokenizer.json     blake3:8a801317cfb1eb744734392dc0aeef36d6d39660a0a884f06a0338e9bb5357db
  ```

  Révision épinglée : `a5fdb000f7a21da96c3bddde3a782ef777316df3` (`lightonai/mDenseOn`). Deux exports complets sur Mac, à versions égales, rendent les mêmes hash. La reproductibilité des hash **entre plateformes** n'a pas été mesurée ; elle n'est pas nécessaire ici, puisque la CI ne ré-exporte pas et reçoit l'artefact déjà exporté.
- **Licence** : `lightonai/mDenseOn` déclare Apache-2.0 dans les métadonnées de sa carte de modèle, et son dépôt Hugging Face n'est pas restreint (vérifié le 2026-10-24 par l'API Hugging Face). Ce dépôt amont ne contient **ni `LICENSE` ni `NOTICE`** : la vérification du 2026-10-24 n'y trouve aucun de ces deux fichiers. La redistribution de l'artefact est donc permise, et la licence est portée par le dépôt qui héberge l'artefact : il contient un `LICENSE` au texte d'Apache-2.0. Aucun `NOTICE` n'est à reproduire, faute de source amont. Crible est lui-même sous Apache-2.0 (ADR-0015).
- **Accès réseau** : ADR-0032 ne vise que l'accès à la production Legifacile ; il ne dit rien d'un téléchargement d'artefact public en CI. Cette décision couvre ce cas : le job télécharge des fichiers publics de `huggingface.co`, sans jeton et sans donnée de production.
- **Inconnues** : la durée du job à froid et à cache chaud sur `ubuntu-latest` ; le comportement numérique exact des noyaux ONNX Runtime Linux sur le test à 1e-4 ; la disponibilité de `huggingface.co` et la stabilité du dépôt de modèle. La décision les traite par la mesure et par l'échec franc.

## Décision

**La CI vérifie le chemin ONNX avec ONNX Runtime, le modèle factice et le vrai modèle mDenseOn, dans un job séparé. L'échantillon Legifacile reste local.** Le job Rust actuel garde `cargo test` sans ONNX Runtime.

### 1. ONNX Runtime vient d'une roue pip, dans un groupe `ci` sans torch

Un groupe `uv` dédié, `ci`, est ajouté à `tools/pyproject.toml`. Il contient `onnxruntime==1.31.0`, version exacte, installée depuis la roue Linux x86_64. Il ne contient **pas** `torch`, et il n'est **pas** le groupe `export` : `uv sync --group ci` n'installe pas la chaîne d'export.

`ORT_DYLIB_PATH` désigne la bibliothèque de la roue (`onnxruntime/capi/libonnxruntime.so.1.31.0`, chemin exact relevé par la tâche). Aucun chemin de poste de développement n'entre dans le workflow.

La feature `download-binaries` d'`ort` n'est pas utilisée : elle est contraire au mode `load-dynamic` retenu par `docs/07` et ferait entrer un binaire non contrôlé dans le build.

Le groupe `ci` peut aussi recevoir `tokenizers` si c'est la seule dépendance qui manque aux tests de `tools/bench/mes04/` (voir point 7). Il ne reçoit jamais `torch`.

### 2. Le vrai modèle vient d'un dépôt de modèle Hugging Face, épinglé par sha et vérifié par hash

Un artefact **déjà exporté** est publié dans un dépôt de modèle Hugging Face **dédié et public**, `<compte>/mdenseon-onnx`, sous le compte du propriétaire. Le nom exact du compte est fixé dans le manifeste versionné, pas dans cet ADR : le dépôt est désigné par le rôle « dépôt de l'artefact de CI », et seul le manifeste en donne l'adresse.

Le dépôt contient :

- `model.onnx` (fp32, hash `7b6eb680…094a2`) ;
- `model.int8.onnx` (hash `0534b4ee…941a5`), publié pour rester cohérent avec la déclaration `files.int8` du `model.json`, mais téléchargé seulement si un test l'exige ;
- `tokenizer.json` (hash `8a801317…357db`) ;
- `model.json` (celui de l'export, champ `export` compris) ;
- `LICENSE` : le texte d'Apache-2.0 ;
- `README.md` : la carte du modèle, avec `base_model: lightonai/mDenseOn`, la révision amont épinglée `a5fdb000f7a21da96c3bddde3a782ef777316df3`, la licence `apache-2.0`, la mention de la modification (« export ONNX par `tools/export_onnx.py`, transformers 5.14.1, et quantification INT8 dynamique »), un lien vers le dépôt Crible, et les hash blake3 des fichiers.

Les quatre premiers fichiers sont des **copies exactes** de `data/models/mdenseon/`. `parity.json` n'est pas publié : il décrit la parité mesurée en local, pas un fichier dont la CI a besoin.

La CI télécharge par URL explicite, épinglée sur le **sha du commit Hugging Face**, jamais sur `main` :

```
https://huggingface.co/<compte>/mdenseon-onnx/resolve/<sha du commit HF>/<fichier>
```

Le dépôt est public : aucun jeton n'est nécessaire, ni en CI ni dans le dépôt Crible. Seuls les fichiers dont les tests ont besoin sont téléchargés ; l'INT8 n'est téléchargé que si un test l'exige. Les 19 tests retenus n'en ont pas besoin : les trois tests du vrai modèle chargent le graphe fp32 et le tokenizer, et les tests d'INT8 utilisent le modèle factice.

Le manifeste de l'artefact est **versionné dans le dépôt** (par exemple `ci/model-artifact.toml`). Il fige le dépôt Hugging Face, le sha du commit HF, le nom des fichiers et les **hash blake3 complets** de chaque fichier téléchargé, **`model.json` compris**. Le hash de `model.json` est relevé sur le dépôt au moment de l'envoi : il ne peut pas être écrit avant. Placer ce hash dans `model.json` lui-même serait circulaire ; le manifeste, qui est un fichier distinct, évite la circularité.

Le téléchargement et la vérification échouent franchement :

- téléchargement impossible, code HTTP non 2xx, fichier absent, dépôt ou révision introuvable : le job échoue avec l'URL et le nom du fichier ;
- hash différent : le job échoue avec le hash attendu et le hash obtenu ;
- aucun repli silencieux : pas de `continue-on-error`, pas de `|| true`, pas de « on saute le test ».

Le cache GitHub Actions est indexé sur les hash du manifeste. Il n'est **jamais** une source de confiance : le contenu restauré est vérifié comme un téléchargement neuf ; un écart échoue avec la clé de cache à purger. Un cache manquant déclenche le téléchargement, qui est ensuite vérifié.

La création du compte et du dépôt Hugging Face, et l'envoi des fichiers, sont faits par le **propriétaire**, avec son propre jeton (`hf upload`). Aucun agent ne publie à sa place, et aucun jeton n'entre en CI ni dans le dépôt Crible. Le sha du commit HF obtenu après l'envoi est reporté dans le manifeste par la tâche `AJOUTER-ORT-CI`.

### 3. Un job `rust-onnx` séparé

Un job `rust-onnx` est ajouté à `.github/workflows/ci.yml`, sur `ubuntu-latest`, déclenché sur **push et pull request**. Il a un `timeout-minutes` explicite : la tâche fixe une valeur initiale large, puis la resserre après le premier run à froid. Le job Rust actuel reste inchangé : `cargo test --locked` sans ONNX Runtime.

Le job, dans l'ordre :

1. installe la toolchain Rust et `uv` ;
2. génère `synth-small` avec `tools/`, comme le job Rust actuel ;
3. installe le groupe `ci` (`uv sync --group ci`) et exporte `ORT_DYLIB_PATH` vers la bibliothèque de la roue ;
4. restaure le cache, sinon télécharge les fichiers depuis le dépôt Hugging Face épinglé par sha, puis vérifie les hash du manifeste et place les fichiers dans `data/models/mdenseon/`, chemin attendu par les tests ;
5. exécute :

   ```
   cargo test --locked --workspace -- --include-ignored --skip inv_12_the_legifacile_sample_passes_the_acceptance_check
   ```

Cette commande exécute **19 tests ignorés** et laisse **1 test local**, exclu explicitement par son nom. Elle exécute aussi les 400 tests non ignorés : le job `rust-onnx` rejoue donc 419 tests au total. C'est voulu : il prouve le workspace entier dans l'environnement Linux avec ONNX Runtime, pas seulement les tests ignorés.

Le test exclu est `inv_12_the_legifacile_sample_passes_the_acceptance_check`. Il reste local parce que ses données dérivent de la production Legifacile (ADR-0032) et ne sont pas publiées. Le rapport J2 (§7) constate déjà que ce test est local ; cette phrase reste vraie après la tâche.

### 4. Règle anti-saut silencieux

La règle suivante est inscrite dans `docs/08-invariants-et-tests.md`, section « Stratégie de tests » :

> **Un test `#[ignore]` lancé explicitement et privé de son prérequis échoue.** Le message d'échec donne la commande qui produit le prérequis. `#[ignore]` est le seul mécanisme de saut autorisé : un `return` anticipé, un `if !path.exists() { return; }` ou toute sortie silencieuse équivalente sont interdits. Un test non lancé n'est pas un saut : il n'apparaît pas dans le compte des tests exécutés.

`crates/crible/tests/recette_import.rs` est corrigé en conséquence : le `return` des lignes 1333-1352 devient un échec avec la commande qui construit `data/legifacile-sample/` et `data/models/mdenseon/`. Les tests de `charger_modele.rs` et `model_config.rs` sont déjà conformes et servent de modèle.

### 5. La durée du job est mesurée et consignée

La durée du job `rust-onnx` est relevée **à froid** (sans cache) et **à cache chaud**, puis notée dans `docs/suivi.md` à la fin de la tâche. Aucune affirmation de performance n'est faite avant ce relevé (règle 7 de `AGENTS.md`). Le premier run à froid sert aussi à vérifier que le `timeout-minutes` est adapté.

### 6. Ce qui ne change pas

- Le seuil de recette reste `acceptance_min_cosine = 0,999` (ADR-0052).
- La forme de l'empreinte du modèle reste celle d'ADR-0028 ; le champ `export` reste hors empreinte (ADR-0050).
- La révision `a5fdb000f7a21da96c3bddde3a782ef777316df3` et les hash du modèle restent ceux d'ADR-0050 et de `tools/README.md`.
- MES-04 et MES-05 restent les **preuves de fidélité** ; le test à 1e-4 est un garde-fou, pas une mesure de campagne.
- Le test de l'échantillon Legifacile reste local.
- Aucun index de production n'est créé, aucun format disque ne change, aucune donnée de production n'entre dans la CI.

### 7. Une tâche hors plan `AJOUTER-ORT-CI` en fin de P2

Une nouvelle tâche hors plan, **`AJOUTER-ORT-CI`**, est ajoutée à la fin de `docs/phases/P2-encodeur-et-recette.md`, avec une fiche complète. Le précédent est `REEXPORTER-MODELE-TRANSFORMERS-5`, ajoutée par ADR-0050.

- **Dépend de** : `NETTOYER-P2`, `CLI-RECHERCHE-TEXTE`, `RECETTE-IMPORT`, `CONTROLER-LONGUEURS`, `ENCODER-DOCUMENTS-EN-LIGNE`.
- **À lire** : cet ADR ; `docs/07-stack-et-deploiement.md` (« CI (GitHub Actions) », « Côté Python ») ; `docs/08-invariants-et-tests.md` (« Stratégie de tests ») ; `.github/workflows/ci.yml` ; `tools/README.md` (« Révision et hash mesurés »).
- **Objectif** : rendre la CI capable d'exécuter les 19 tests ONNX Runtime sur le modèle factice et le vrai modèle, et rendre le saut silencieux impossible.
- **Livrables** : le groupe `ci` dans `tools/pyproject.toml` ; le job `rust-onnx` dans `.github/workflows/ci.yml` ; le manifeste versionné ; le téléchargement et la vérification des hash ; la préparation du dossier d'envoi de l'artefact ; la correction de `recette_import.rs` ; la mesure des durées à froid et à cache chaud ; les petites corrections listées ci-dessous.
- **Étapes de publication de l'artefact** : la tâche prépare dans `data/` (gitignoré) un dossier contenant `model.onnx`, `model.int8.onnx`, `tokenizer.json` et `model.json`, copies exactes de `data/models/mdenseon/`, plus `LICENSE` (texte d'Apache-2.0) et `README.md` (carte du modèle : `base_model: lightonai/mDenseOn`, révision amont épinglée, licence `apache-2.0`, mention de la modification « export ONNX par `tools/export_onnx.py`, transformers 5.14.1, et quantification INT8 dynamique », lien vers le dépôt Crible, hash blake3) ; le **propriétaire** crée le compte et le dépôt Hugging Face, puis fait l'envoi lui-même avec son jeton (`hf upload`) ; la tâche écrit ensuite le manifeste avec le dépôt, le sha du commit HF obtenu et les blake3 complets.
- **Critères d'acceptation** : les 19 tests ignorés passent dans le job `rust-onnx` sur un run distant ; le test de l'échantillon Legifacile est le seul exclu, par son nom ; le manifeste fige le dépôt, le sha du commit HF et les blake3 complets, `model.json` compris ; un hash faux fait échouer le job ; un fichier manquant fait échouer le test avec la commande qui le produit ; un dépôt ou une révision introuvable fait échouer le job ; le job Rust actuel reste sans ONNX Runtime ; aucun jeton Hugging Face n'apparaît dans la CI ni dans le dépôt ; les cinq corrections listées ci-dessous sont faites ; les durées sont notées dans le suivi.
- **Clause d'arrêt** : si le dépôt Hugging Face ne peut pas être créé, si l'envoi ne peut pas être fait, si un hash ne correspond pas, ou si le graphe fp32 ne s'exécute pas sous Linux, la tâche s'arrête et remonte une `QUESTION-CONCEPTION`. Elle ne remplace pas le modèle par un autre, ne ré-exporte pas, ne déplace pas l'artefact vers un autre hébergement sans ADR, et ne désactive pas un test pour faire passer le job.
- **Hors périmètre** : la route HTTP et le serveur (P6), un ré-export du modèle, les campagnes MES-04 et MES-05, la publication de l'image de déploiement.

La fiche reprend aussi, par décision du propriétaire, ces corrections issues des idées de P2 :

- **figer les scores de la démo J2** dans `docs/jalons/J2-demo.md`, en fp32 par défaut (ADR-0052), avec les valeurs mesurées (top-1 `JORFDOLE000047079143#0`, 0,623664 en fp32) ;
- **citer `tools/bench/mes04/`** dans le tableau des outils de `tools/README.md` ;
- **faire tourner les tests de `tools/bench/mes04/`** qui demandent `tokenizers` (`importorskip`) dans le job `tools`, par le groupe `ci` si `tokenizers` y suffit, sans torch ; à défaut, rendre le saut explicite et le justifier ;
- **écrire `precision_effective`** aussi dans le JSON des vecteurs de `crates/crible-encode/examples/measure_queries.rs` ;
- **`tools/export_onnx.py`** : `min_cosine_required` est écrit deux fois (`run_parity` puis `export`), et le paramètre `revision` d'`export_fp32` n'est pas lu ; le consommer ou l'assumer explicitement.

**J2 n'est validé qu'après cette tâche et un run CI distant vert sur `phase/P2-encodeur-et-recette`.** Le jalon reste `PRET-POUR-VALIDATION` jusque-là.

## Conséquences

- **Positives** :
  - 19 des 20 tests ONNX Runtime sont prouvés à chaque push et à chaque pull request, sur Linux, sans attendre un passage local ;
  - la parité à 1e-4 sur le vrai graphe et l'acceptation du vrai `model.json` sont gardées à la fusion, alors qu'elles ne l'étaient pas ;
  - le job Rust actuel reste rapide et sans torch : une panne de l'artefact ou d'ONNX Runtime ne bloque pas la suite de base ;
  - le saut silencieux de `recette_import.rs` est supprimé, et la règle vaut pour tous les tests futurs ;
  - `docs/07` cesse d'être incomplet : il nomme le groupe `ci` et le job `rust-onnx`, et distingue les jeux de données des artefacts de modèle ;
  - l'artefact est protégé par un sha de commit Hugging Face et des hash complets dans un manifeste versionné : un fichier remplacé ou corrompu ne peut pas passer pour le modèle mesuré ;
  - l'artefact est hébergé chez Hugging Face, lieu naturel d'un modèle, sans poids dans git ni dans une Release du dépôt de code, et chaque version y est un commit identifiable ;
  - aucun secret nouveau en CI, aucun torch, aucune donnée de production, aucun modèle dans git (règle 14 d'`AGENTS.md`).

- **Négatives / coûts** :
  - le job `rust-onnx` ajoute des minutes de CI à chaque push et chaque pull request ; le rapport J2 estimait environ 2 min de plus, la mesure réelle reste à faire ;
  - un run à froid télécharge environ 1,1 Gio (fp32, tokenizer, `model.json`), puis utilise le cache ; le cache consomme de l'espace et peut être purgé par GitHub ;
  - trois chemins de modèle coexistent désormais : le vrai modèle local, le vrai modèle d'artefact en CI et le modèle factice ; une divergence entre eux est possible et doit être surveillée ;
  - le dépôt Hugging Face et son compte sont **extérieurs au projet** : le propriétaire doit créer le compte et le dépôt, et faire l'envoi lui-même avec son jeton ; la CI dépend d'un compte personnel et d'un hébergeur tiers, même si le dépôt est public et lisible sans jeton ;
  - le dépôt Hugging Face est un objet **mutable** : un fichier peut être remplacé, un commit supprimé, une branche réécrite ; l'épinglage par sha et la vérification des blake3 sont la vraie protection, pas l'URL ;
  - la roue `onnxruntime==1.31.0` est épinglée pour Linux x86_64 ; un changement de runner ou une mise à jour du runtime demande une décision ;
  - le test de course à huit threads peut être plus lent ou plus variable sur un runner CI partagé ;
  - le `LICENSE` Apache-2.0 est fourni par le dépôt de l'artefact, pas par l'amont ; il doit être conservé dans toute redistribution de l'artefact.

- **À surveiller** :
  - la durée du job à froid et à cache chaud, notée dans le suivi : si le cache chaud dépasse nettement l'estimation du rapport J2, ou si le job approche son `timeout-minutes`, la décision est à revoir ;
  - un écart de parité à 1e-4 sur Linux : ce n'est pas un défaut du test, c'est un signal sur les noyaux ONNX Runtime Linux, à instruire avant de conclure (ADR-0050 et fiche `CHARGER-MODELE`) ;
  - toute modification d'un fichier du dépôt Hugging Face sans nouvel ADR : le sha du commit HF et les hash du manifeste doivent changer, et un nouvel export relève d'ADR-0028 et ADR-0050, pas d'une simple mise à jour de manifeste ;
  - le nom `inv_12_the_legifacile_sample_passes_the_acceptance_check` est un contrat : le renommer sans mettre à jour le `--skip` ferait exécuter le test local en CI, qui échouerait faute de données (échec bruyant, pas silencieux) ;
  - la disponibilité de `huggingface.co`, l'existence du dépôt et celle du commit épinglé ; une indisponibilité, une suppression ou une réécriture doit faire échouer le job, pas le rendre vert ; les blake3 restent le contrôle final ;
  - le taux de succès du cache, qui dépend des hash du manifeste et non des URLs ;
  - la version d'`onnxruntime` dans le groupe `ci` et celle du champ `export` de `model.json` : elles doivent rester alignées ;
  - la déclaration `files.int8` du `model.json` publié alors que l'INT8 n'est pas téléchargé en CI : acceptable tant que seul le fp32 est chargé, ce que le défaut fp32 d'ADR-0052 garantit pour les requêtes ; tout test qui demandera l'INT8 devra le télécharger.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| **(a) Exporter le modèle en CI avec `tools/export_onnx.py`** | L'export demande torch, transformers et la chaîne complète, soit plusieurs gigaoctets et un temps sans rapport avec le test. La reproductibilité des hash **entre plateformes** n'a pas été mesurée : le job pourrait produire un graphe différent de celui qui a servi aux mesures, et échouer ou, pire, tester un autre modèle. Cela contredit ADR-0049 et ADR-0050, qui gardent l'export hors de la CI. |
| **Télécharger les poids source amont depuis Hugging Face à chaque run** | Le dépôt `lightonai/mDenseOn` héberge les poids source, pas l'ONNX exporté par Crible, ni son `model.json`, ni ses hash. Il faudrait donc ré-exporter, avec les mêmes coûts et les mêmes risques que l'alternative (a). |
| **Release GitHub du dépôt Crible** | Envisagée d'abord, puis écartée par le propriétaire au profit de Hugging Face. Un modèle s'héberge naturellement dans un dépôt de modèle, versionné par commit : Hugging Face donne un sha stable à épingler, une carte de modèle et un `LICENSE` à côté des poids. Une Release mêle l'artefact de test au dépôt de code, se désigne par un tag qu'il faut traiter comme immuable par convention, et n'offre pas de versionnement natif du contenu. |
| **Git LFS** | Le modèle entrerait dans git, ce que la règle 14 d'`AGENTS.md` interdit (« Rien de volumineux dans git »). Le clone et chaque CI paieraient le poids du LFS, et l'historique garderait des binaires lourds. |
| **Image GHCR contenant le modèle** | L'image de déploiement n'a pas à porter un modèle de test, et le job de test n'a pas à tirer une image. Le modèle serait couplé à la version de l'image, avec une authentification de registre et un temps de tirage supérieurs au cache d'actions. Cela mélange la livraison (P8) et la vérification (P2). |
| **Job nocturne ou déclenché à la demande** | Il couvre les mêmes tests, mais pas à la fusion : une régression peut entrer dans la branche de phase et n'être vue que le lendemain. La couverture à la fusion est précisément ce que le rapport J2 reproche au statu quo. |
| **Modèle factice seul (option 1 du rapport J2)** | Moins de couverture que la décision : la parité à 1e-4 sur le vrai graphe, l'acceptation du vrai `model.json` et le chargement du vrai modèle ne seraient pas prouvés. Le modèle factice reste utilisé pour les 15 tests qui n'ont pas besoin du vrai, mais il ne suffit pas. |
| **Statu quo (option 3 du rapport J2)** | Les 20 tests restent locaux. Une régression du chemin ONNX ne serait vue qu'au prochain passage local, pas à la fusion, et le défaut de `recette_import.rs` continuerait de faire passer un test vide pour un test vert. |
| **Exécuter `--include-ignored` dans le job Rust actuel** | Le job de base perdrait sa propriété « sans ONNX Runtime » : toute panne de l'artefact, du cache ou du réseau bloquerait aussi les tests qui n'ont rien à voir avec l'encodeur. Un job séparé isole la couverture ONNX et garde un chemin de CI rapide. |
| **Feature `download-binaries` d'`ort`** | Contraire au mode `load-dynamic` retenu par `docs/07`. Elle téléchargerait un binaire à la compilation, hors du contrôle de version et de hash, alors que la roue pip est épinglée et que `ORT_DYLIB_PATH` désigne une bibliothèque connue. |

## Modifications à appliquer après acceptation

Cette section décrit les modifications. **Aucune n'est appliquée par cet ADR** : elles le seront par `@greffier`, dans un commit qui cite `ADR-0057`, après acceptation du propriétaire.

### `docs/07-stack-et-deploiement.md`

1. **« Côté Python »** : ajouter, après la phrase sur le groupe `export`, que le groupe `ci` contient `onnxruntime==1.31.0` pour le job `rust-onnx`, qu'il ne contient pas torch, qu'il s'installe avec `uv sync --group ci`, et que `ORT_DYLIB_PATH` désigne la bibliothèque de la roue. La phrase « donc la CI ne l'installe pas » reste vraie pour `export` ; elle est précisée pour `ci`.
2. **« CI (GitHub Actions) »** : ajouter un point sur le job `rust-onnx` — déclencheurs push et pull request, `timeout-minutes`, installation du groupe `ci`, téléchargement des fichiers depuis le dépôt de modèle Hugging Face épinglé par sha et vérification des hash, cache indexé sur les hash, commande `cargo test --locked --workspace -- --include-ignored --skip inv_12_the_legifacile_sample_passes_the_acceptance_check`. Reformuler la phrase sur les « jeux de données lourds » pour distinguer les jeux de données, qui restent hors CI, de l'artefact de modèle déjà exporté, qui est téléchargé et mis en cache.

### `docs/08-invariants-et-tests.md`

3. **« Stratégie de tests »** : ajouter la règle anti-saut silencieux du point 4, sous un titre « Sauts de test » ou en paragraphe encadré. La règle est normative : `#[ignore]` est le seul mécanisme de saut ; un test lancé explicitement sans son prérequis échoue avec la commande qui le produit.

### `docs/phases/P2-encodeur-et-recette.md`

4. **Liste du jalon J2** : ajouter un point « un run CI distant vert sur la branche de phase, job `rust-onnx` compris ».
5. **Fin de la fiche** : ajouter `### AJOUTER-ORT-CI` après `CLI-RECHERCHE-TEXTE`, avec les champs `Objectif`, `Dépend de`, `À lire`, `Livrables`, `Critères d'acceptation`, `Clause d'arrêt` et `Hors périmètre`, conformément au point 7. La fiche renvoie à cet ADR et reprend les cinq corrections listées au point 7.

### `docs/adr/`

6. **ADR-0050** : ajouter une note datée « À surveiller », corps inchangé, indiquant que la CI exécute désormais l'artefact exporté et non un ré-export, et que toute modification de l'artefact ou de ses hash relève d'un nouvel ADR. **ADR-0049** n'a pas besoin de note : il est déjà « Remplacé par ADR-0050 », et sa phrase sur la CI reste exacte pour le groupe `export`.

### Hors de ces documents

`.github/workflows/ci.yml`, `tools/pyproject.toml`, le manifeste de l'artefact, le téléchargement et la vérification des hash, la correction de `crates/crible/tests/recette_import.rs`, les tests et les durées relèvent de la tâche `AJOUTER-ORT-CI`. `docs/suivi.md` reste tenu par l'orchestrateur : il y ajoutera la ligne de tâche, l'état de J2 et les durées mesurées. `docs/jalons/J2-rapport.md` n'est pas modifié par cet ADR ; son §7 constate déjà que le test de l'échantillon est local. Après la tâche, l'orchestrateur y consignera le run CI vert et les durées, ou le fera dans le suivi.

## Références

- `docs/jalons/J2-rapport.md`, §7 (« Écarts, idées, risques ») : absence de run CI, 20 tests ignorés, options 1 à 4, idées de P2.
- `docs/07-stack-et-deploiement.md` : « Côté Python », « Environnement de développement (Mac) », « CI (GitHub Actions) », « Docker ».
- `docs/08-invariants-et-tests.md` : « Stratégie de tests », « Jeux de données », INV-10, INV-11, INV-12.
- `docs/09-plan-de-developpement.md` : règle 5 bis (dépendance absente de `docs/07`), pipeline et définition de `FAIT`.
- `docs/phases/P2-encodeur-et-recette.md` : fiches `CHARGER-MODELE`, `RECETTE-IMPORT`, `CONTROLER-LONGUEURS`, `ENCODER-DOCUMENTS-EN-LIGNE`, `CLI-RECHERCHE-TEXTE` ; précédent `REEXPORTER-MODELE-TRANSFORMERS-5` (ADR-0050).
- `docs/jalons/J2-demo.md` : démo J2, scores fp32.
- `tools/README.md`, « Révision et hash mesurés » : révision `a5fdb000…`, hash fp32 `7b6eb680…`, INT8 `0534b4ee…`, tokenizer `8a801317…`, export Mac arm64 du 2026-10-23.
- `.github/workflows/ci.yml` : jobs `rust`, `python`, `tools`.
- `tools/pyproject.toml` : groupes `dev` et `export`, index `pytorch-cpu`.
- Hugging Face, dépôt de modèle de l'artefact de CI : `<compte>/mdenseon-onnx`, public, désigné par son sha de commit dans le manifeste versionné ; contenu `model.onnx`, `model.int8.onnx`, `tokenizer.json`, `model.json`, `LICENSE`, `README.md`.
- `crates/crible-encode/tests/charger_modele.rs` : `loads_a_model_once_per_fingerprint`, `loads_a_model_once_when_callers_race`, `matches_the_python_fp32_vectors_within_1e_4` ; fixture `crates/crible-encode/tests/fixtures/mdenseon/`.
- `crates/crible-core/tests/model_config.rs` : `real_model_json_with_export_is_accepted`.
- `crates/crible/tests/recette_import.rs` : `inv_12_the_legifacile_sample_passes_the_acceptance_check`, `return` silencieux vers les lignes 1333-1352.
- ADR-0015 (licence Apache-2.0 de Crible) ; ADR-0028 (forme canonique de l'empreinte) ; ADR-0032 (accès à la production Legifacile, données de l'échantillon non publiées) ; ADR-0049 (groupe `export`, remplacé par ADR-0050) ; ADR-0050 (référence mDenseOn, révision, hash, champ `export`) ; ADR-0052 (requêtes fp32, seuil 0,999) ; ADR-0054 (chargement différé et bornes de la recette).
- `lightonai/mDenseOn`, révision `a5fdb000f7a21da96c3bddde3a782ef777316df3`, licence Apache-2.0 déclarée dans les métadonnées de la carte de modèle, dépôt Hugging Face non restreint, **sans `LICENSE` ni `NOTICE`** (vérifié le 2026-10-24 par l'API Hugging Face).
- PyPI, `onnxruntime` 1.31.0, roue Linux x86_64.
