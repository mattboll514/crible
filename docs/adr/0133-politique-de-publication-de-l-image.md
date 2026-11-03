# ADR-0133 — Politique de publication de l'image

- **Statut** : Accepté (par anticipation le 2026-11-03, sur délégation du propriétaire du 2026-11-02 ; confirmation à venir)
- **Date** : 2026-11-03
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, tâche PUBLIER-IMAGE (phase P8), sur les recommandations de `@architecte` retenues telles quelles par l'orchestrateur, en application de la délégation du propriétaire du 2026-11-02
- **Remplace** : — (ne remplace aucun ADR ; complète ADR-0129, décision 2, sans modifier son corps)

La délégation du propriétaire du 2026-11-02 est citée telle quelle : « Si tu as des ADR a ecrire considere qu'ils sont accpeté par anticipation, tu n'as pas besoin de t'arrêter, on les valide officienlement ensemble quand tu as fini ».

## Contexte

### La fiche, et les deux points qu'elle laisse ouverts

La fiche `PUBLIER-IMAGE` (`docs/phases/P8-docker-et-deploiement.md:162-167`) a pour objectif « la publication automatique ». Son livrable est :

> un workflow sur tag `v*` qui construit l'image multi-arch avec buildx et la pousse sur GHCR (`ghcr.io/<owner>/crible:<version>` et `:cpu`).

Elle ne dit pas ce que vaut `<version>` : la version du tag, celle du `Cargo.toml`, ou une autre forme. Elle ne dit pas non plus ce que signifie `:cpu`, ni quand cette étiquette se déplace. Le critère d'acceptation est double : « le workflow est valide », et « **le premier tag est poussé par le propriétaire** (l'agent ne pousse jamais) ».

### Ce que la conception impose déjà

`docs/07-stack-et-deploiement.md:121` pose une **seule image serveur, CPU, multi-architecture** : `linux/arm64` pour les deux Mac — machine de développement et Mac mini —, `linux/amd64` pour la cible future. ADR-0129, décision 2, reprend la règle : « L'image reste multi-architecture : `linux/arm64` pour les deux Mac, `linux/amd64` pour la cible future ». Aucune décision de cet ADR ne peut donc publier une seule architecture.

ADR-0129, décision 4, fixe aussi J8 : « l'image tourne sur la machine de développement en arm64 et **se construit en amd64** ». Les deux architectures sont donc prouvées en P8, et la publication en est la première mise en pratique à chaque tag.

### Le coût d'une compilation croisée, et le fait qui le tranche

Une compilation Rust en release du `workspace` pour `linux/arm64` sous QEMU, sur un runner amd64, prend des dizaines de minutes à froid. Le projet a mesuré le cas symétrique : la construction de l'image amd64 sous QEMU sur la machine arm64 a pris **10 min 13 s**, et la construction multi-architecture sur un seul constructeur **25 min 37 s** (suivi, `CONSTRUIRE-IMAGE-DOCKER`, `docs/suivi.md:134`). Le dépôt Crible est sur GitHub, sous licence Apache 2.0 (`docs/adr/0032-acces-a-la-production.md:14`) et public : les runners arm64 natifs de GitHub (`ubuntu-24.04-arm`) y sont gratuits. Deux runners natifs coûtent donc moins, en temps de mur, qu'un seul runner qui émule l'autre architecture.

### Une politique de CI se trace par un ADR

Chaque règle de la CI qui engage le projet a son ADR : ADR-0057 (ONNX Runtime et vrai modèle dans la CI), ADR-0059 (téléchargement robuste de l'artefact et workflow `model-cache.yml` seul écrivain du cache), ADR-0068 (feature `failpoints` construite et testée en CI), ADR-0112 (`uv run pytest` sans binaire Rust), ADR-0114 (`cargo doc` et `RUSTDOCFLAGS`), ADR-0132 (exclusion des tests de l'arrêt propre). La publication de l'image est une politique de la même famille : elle décide de qui publie, quand, sous quels noms et avec quels droits. La relecture du premier tour de PUBLIER-IMAGE a demandé cette trace.

### L'état de l'arbre au moment de la proposition

Le fichier `.github/workflows/publish-image.yml` existe dans l'arbre de travail, **non commité**. Il porte déjà le déclencheur par tag, la garde de version, la matrice des deux runners natifs, la portée de cache par architecture, la poussée par empreinte, le job `merge` et les permissions minimales. Les décisions 2 (pré-version), 4 (garde de monotonie de `:cpu`) et 5 (`concurrency`) ci-dessous restent à écrire par `@dev`, dans le commit de la tâche. Cet ADR ne décrit donc pas seulement ce que le fichier fait : il fixe ce qu'il doit faire.

## Décision

**La publication de l'image est un workflow dédié, déclenché par un tag `v*` et par lui seul. Il publie une image multi-architecture sous deux étiquettes — la version du tag et `:cpu` —, et `:cpu` ne recule jamais.** Aucun agent ne crée ni ne pousse de tag.

Précisions :

1. **Déclencheur.** `.github/workflows/publish-image.yml` publie sur un tag `v*`, et sur lui seul : ni branche, ni pull request, ni `workflow_dispatch`. Un push de branche ne touche jamais le registre.

2. **Garde de version.** Le tag doit valoir `v` suivi de `[workspace.package] version` de `Cargo.toml` ; sinon le workflow échoue **avant toute construction**. La valeur de `[workspace.package] version` est `0.1.0` au moment de cet ADR. `<version>`, l'étiquette de l'image, est le tag sans son `v` : `v0.1.0` publie `0.1.0`. Une **pré-version Cargo** — par exemple `0.1.0-rc.1`, tag `v0.1.0-rc.1` — reste admise et publie son étiquette de version, mais **ne déplace jamais `:cpu`** : elle sert d'essai du workflow avant la première version stable.

3. **Multi-architecture, sans QEMU.** Deux runners natifs construisent chacun une architecture : `ubuntu-24.04-arm` pour `linux/arm64`, `ubuntu-latest` pour `linux/amd64`. Chaque architecture a sa portée de cache `type=gha`, pour que les deux jobs n'écrasent pas les couches l'un de l'autre. Chaque job pousse **par empreinte**, sans étiquette. Un job `merge` assemble ensuite la liste de manifestes avec `docker buildx imagetools create` et pose les étiquettes ; cette commande n'écrit aucune couche, elle pointe les étiquettes vers la liste. Aucune étape QEMU.

4. **Sémantique de `:cpu`.** `:cpu` est l'étiquette **mobile** de la dernière version **stable** publiée. Elle n'est déplacée que si le tag poussé est le plus haut tag stable `vX.Y.Z` du dépôt, obtenu par le tri des tags git, ce qui impose `fetch-depth: 0` au job qui décide. Un correctif d'une version antérieure — par exemple `v0.1.1` après `v0.2.0` — publie son étiquette de version et **laisse `:cpu` en place**.

5. **Sérialisation des publications.** Le workflow déclare une `concurrency` de groupe fixe (`publish-image`) avec `cancel-in-progress: false`. Les publications sont ainsi sérialisées sans laisser d'empreintes orphelines derrière une exécution annulée. Ce réglage **ne garantit pas l'ordre** : GitHub ne garde qu'une exécution en attente. C'est la garde de la décision 4, et elle seule, qui empêche `:cpu` de reculer.

6. **Secret et permissions.** Le seul secret employé est `GITHUB_TOKEN`, le jeton automatique du workflow ; aucun secret ajouté au dépôt. Les permissions sont minimales : `contents: read` au niveau du workflow, et `packages: write` sur les deux jobs qui publient seulement — `build` et `merge` dans le fichier. Le job `meta`, qui décide du nom et de la version, ne demande que `contents: read`.

7. **Le premier tag est un geste du propriétaire.** Aucun agent ne crée ni ne pousse de tag : la fiche le dit (`docs/phases/P8-docker-et-deploiement.md:166`), et la règle de git du projet laisse déjà les tags à `@git` sur instruction explicite. Le premier `v0.1.0` est donc poussé par le propriétaire, après vérification du workflow.

### Ce qui n'est pas décidé ici

- Le contenu du `Dockerfile` et les étapes de l'image : `docs/07-stack-et-deploiement.md:121-130`, tâche `CONSTRUIRE-IMAGE-DOCKER`, déjà faite.
- La construction locale de l'image et son nom (`crible:dev`, `CRIBLE_DOCKER_IMAGE`) : ADR-0132 et les tests de l'arrêt propre.
- La publication d'un tag par un agent, sous quelque forme que ce soit : elle reste interdite.
- Une publication automatique depuis `main`, un registre autre que GHCR, une signature d'image (`cosign` ou équivalent), et un `workflow_dispatch` de republication.

### Documents mis à jour par cet ADR

- `docs/07-stack-et-deploiement.md:161` : la puce « build de l'image multi-arch avec buildx et publication sur GHCR » devient un paragraphe qui nomme le workflow et décrit déclencheur, garde de version, pré-versions, deux runners natifs, empreinte et `merge`, sémantique de `:cpu`, `concurrency` et renvoi à cet ADR, sur le modèle du paragraphe de `model-cache.yml` (`:160`).
- `docs/phases/P8-docker-et-deploiement.md:162-167` : une note datée (`:167`) cite cet ADR pour `<version>` et `:cpu`.
- `.github/workflows/publish-image.yml` : complété par `@dev` dans le commit de la tâche, qui y écrit les décisions 2, 4 et 5 ; `@greffier` n'y touche pas.

## Conséquences

- **Positives** :
  - `<version>` et `:cpu` ont une définition écrite : une étiquette ne peut plus changer de sens en silence ;
  - une faute de tag échoue avant la construction, donc avant de publier une image dont l'étiquette ment ;
  - la pré-version donne un essai à blanc du workflow avant `v0.1.0`, alors qu'une image publiée sur GHCR est difficile à retirer ;
  - les deux architectures sont construites nativement : la durée de publication ne dépend plus de l'émulation, et J8 « se construit en amd64 » reste vrai à chaque tag ;
  - `:cpu` ne recule pas sur un correctif d'une version antérieure, et la garde est une comparaison de tags, pas une politique d'étiquettes nouvelle ;
  - `cancel-in-progress: false` évite des empreintes poussées sans liste de manifestes, donc des couches orphelines dans le registre ;
  - les permissions restent minimales, et aucun secret de plus n'entre dans le dépôt.
- **Négatives / coûts** :
  - deux jobs de construction au lieu d'un : la facture de minutes double par publication, et le job `merge` ajoute un troisième passage ; les runners arm64 ne sont gratuits que tant que le dépôt reste public ;
  - deux portées de cache `type=gha` à tenir, et un cache qui reste froid à chaque changement de dépendances ;
  - `cancel-in-progress: false` fait attendre les publications en série, et GitHub ne garde qu'une exécution en attente : une rafale de tags peut en perdre une, qui devra être relancée ;
  - la garde de version impose de mettre à jour `[workspace.package] version` avant de pousser le tag, donc une version de `Cargo.toml` qui ne suit plus le code passerait inaperçue ;
  - la règle du plus haut tag stable demande `fetch-depth: 0`, donc un clonage complet sur le job qui décide.
- **À surveiller** :
  - un `:cpu` qui pointerait sur une pré-version ou sur un correctif ancien : le symptôme d'une garde de la décision 4 cassée ;
  - une image dont l'étiquette de version ne correspond pas au binaire, si la garde de la décision 2 est contournée ou déplacée ;
  - une exécution annulée qui laisse une empreinte sans liste de manifestes dans le registre ;
  - la durée des jobs de construction : un dépassement du `timeout-minutes` à froid signalerait une image qui grossit ou un cache qui ne restaure plus ;
  - le passage du dépôt en privé, qui rendrait les runners arm64 payants.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| **QEMU sur un seul runner** | La compilation Rust en release pour `linux/arm64` émulée sur amd64 prend des dizaines de minutes à froid, à chaque tag ; les deux runners natifs sont gratuits sur un dépôt public et font le même travail plus vite. |
| **arm64 seul** | Contredit `docs/07:121` et ADR-0129, décision 2 : `linux/amd64` est la cible future, et J8 exige que l'image se construise en amd64. |
| **Interdire les pré-versions** | Il n'y aurait aucun essai à blanc avant `v0.1.0`, alors qu'une image publiée sur GHCR est difficile à retirer : le premier essai serait la première vraie publication. La pré-version est admise et ne déplace pas `:cpu`. |
| **Déplacer `:cpu` sur une pré-version** | `:cpu` désignerait alors une version qui n'est pas stable, et un déploiement par `:cpu` suivrait une candidate sans le dire. |
| **Aucune garde de monotonie** | Une étiquette mobile qui recule en silence : un correctif d'une version antérieure ramènerait `:cpu` en arrière, et rien dans le registre ne le signalerait. |
| **Garde par étiquette OCI et `imagetools inspect`** | Il faudrait lire l'étiquette de l'image déjà publiée pour décider, donc une politique d'étiquettes nouvelle et une lecture du registre de plus ; le tri des tags git répond à la même question avec `fetch-depth: 0`. |
| **`cancel-in-progress: true`** | Une exécution annulée peut laisser des empreintes poussées sans liste de manifestes : des couches orphelines dans le registre. `false` sérialise les publications. |

## Références

- Fiche : `docs/phases/P8-docker-et-deploiement.md:162-167` (`PUBLIER-IMAGE`) ; `:149-154` (`VERIFIER-ARRET-PROPRE`, image locale) ; `:169-174` (`MESURER-EN-CONTENEUR-MAC`).
- Conception : `docs/07-stack-et-deploiement.md:121-130` (image unique, CPU, multi-architecture), `:150-163` (CI), `:160` (`model-cache.yml`, modèle du paragraphe), `:161` (la puce remplacée par cet ADR).
- Workflow : `.github/workflows/publish-image.yml` (dans l'arbre, non commité au moment de la proposition), `.github/workflows/model-cache.yml`, `.github/workflows/ci.yml`.
- ADR : ADR-0032 (dépôt sur GitHub, licence Apache 2.0), ADR-0057, ADR-0059, ADR-0068, ADR-0112, ADR-0114 et ADR-0132 (politiques de CI tracées par un ADR), ADR-0129 (décisions 2 et 4 : image multi-architecture, J8).
- Externe : documentation de `docker/build-push-action` pour la liste de manifestes à partir d'empreintes, et de `ubuntu-24.04-arm` pour les runners arm64 natifs.
