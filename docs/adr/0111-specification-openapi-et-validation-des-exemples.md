# ADR-0111 — Spécification OpenAPI : Swagger UI embarqué, sécurité, référence des routes et validation des exemples

- **Statut** : Accepté (par anticipation, décision déléguée par le propriétaire le 2026-10-30 ; revue avec le propriétaire à la fin de la série)
- **Date** : 2026-10-30
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, tâche PUBLIER-OPENAPI (phase P6), sur délégation du propriétaire pour la troisième série de P6 et sur les verdicts de `@architecte` du 2026-10-30
- **Remplace** : — (ne remplace aucune décision : complète ADR-0015 et applique ADR-0031, ADR-0101, ADR-0109 et ADR-0110 à la spécification, sans modifier le corps d'aucun d'eux)

## Contexte

### La fiche et ce qu'elle demande

La fiche `PUBLIER-OPENAPI` (`docs/phases/P6-serveur-et-api.md:129-135`) a pour objectif « une spécification et une UI à jour ». Elle dépend de `ROUTE-RECHERCHE` et de `ROUTES-DOCUMENTS-ET-LOTS` (`:131`). Ses livrables sont « des annotations `utoipa` sur toutes les routes et tous les types, `/openapi.json` et `/docs` (Swagger UI) » (`:132`). Ses deux critères d'acceptation sont :

1. « un test compare la liste des routes de la spécification à celle de `05-api.md` » (`:134`) ;
2. « les exemples du doc sont valides contre le schéma » (`:135`).

La fiche ne dit ni comment Swagger UI est livrée au build, ni quelle version d'OpenAPI est publiée, ni comment la sécurité est décrite, ni ce que « la liste des routes de `05-api.md` » compare exactement, ni comment un exemple du document est associé à un schéma. Le critère 1 fait en outre de `docs/05` la référence de la liste des routes, ce qu'aucun ADR n'avait dit avant.

### Ce que la conception pose

`docs/07-stack-et-deploiement.md:32` déclarait, avant cet ADR, le rôle OpenAPI de la façon la plus courte possible :

| Rôle | Crate | Remarque |
|---|---|---|
| OpenAPI | `utoipa`, `utoipa-swagger-ui` | Spécification générée depuis le code |

La ligne ne donne ni version ni feature. Le `Cargo.toml` du workspace les épingle déjà, sans feature : `utoipa = { version = "=6.0.0" }` et `utoipa-swagger-ui = { version = "=10.0.1" }` (`Cargo.toml:42-44`). `crible-server` ne les déclare pas encore : la spécification n'existe pas à ce stade de P6.

`docs/05-api.md` porte le tableau des routes (`:55-77`), avec une colonne « Rôle » où les quatre routes techniques valent « — » : `GET /health` · `GET /ready` (`:76`) et `GET /openapi.json` · `GET /docs` (`:77`). Le middleware d'authentification déjà écrit traite ces quatre chemins comme hors authentification (`crates/crible-server/src/auth.rs:276`).

ADR-0015 (`docs/adr/0015-api-rest-anglais-licence-nom.md:14`) dit, depuis P0, que « la spécification OpenAPI est générée depuis le code (Swagger UI) ». Il décrit un **moyen** de production de la spécification ; il ne dit pas ce qui fait foi de son contenu.

### Ce que les ADR voisins laissent ouvert

- **ADR-0101** fixe la forme du corps d'erreur (`code`, `message`, `details`, `request_id`) et réserve `details` aux clés documentées dans `docs/05` (décision 3). Il ne dit pas comment cette forme se traduit en composant de schéma, ni si les statuts d'erreur sont déclarés par opération.
- **ADR-0031** et **ADR-0109** refusent une clé inconnue dans un corps ; **ADR-0110** étend la règle à tous les niveaux d'un corps à cibles multiples. Ces décisions portent sur le comportement du serveur, pas sur sa description : rien ne dit si les schémas publiés doivent fermer leurs objets.
- **ADR-0099** fixe le corps du `503` de `/ready`, seule exception au format d'erreur (ADR-0101, décision 1). La spécification doit la décrire sans la transformer en règle générale.
- **ADR-0110**, décision 10, signale que `pool_size` et `generation` changent de place entre la route de recherche simple et la route multi-index : « la spécification OpenAPI devra le dire (`PUBLIER-OPENAPI`) ».

### Les NON-COUVERT du verdict `@architecte` du 2026-10-30

Le verdict relève **douze points non couverts**. Ils se rangent sous huit thèmes, qui donnent les huit décisions de cet ADR :

1. le mode de livraison de Swagger UI au build (décision 1) ;
2. les features et les versions des deux crates (décision 2) ;
3. la version d'OpenAPI et le contenu de l'en-tête `info` (décision 3) ;
4. la description de la sécurité, et le sort des quatre routes sans rôle (décision 4) ;
5. l'autorité de `docs/05` sur la liste des routes, et ce que le test compare (décision 5) ;
6. le composant d'erreur et les statuts déclarés par opération (décision 6) ;
7. la fermeture des schémas et le traitement des objets dont la forme dépend de la configuration d'un index (décision 7) ;
8. la preuve que les exemples du document sont valides contre le schéma (décision 8).

Le verdict relève aussi un **écart** : la fiche fait de `docs/05` la référence des routes (`docs/phases/P6:134`), alors qu'ADR-0015 (`:14`) et `docs/07:32` disent la spécification « générée depuis le code », sans dire qui cède en cas de divergence. La décision 5 tranche : les deux propositions ne s'opposent pas, l'une décrit le **moyen**, l'autre le **contenu de référence**.

## Décision

### Décision 1 — Swagger UI sans réseau au build

**Constat.** `utoipa-swagger-ui` 10.0.1 embarque un `build.rs` qui télécharge par défaut un zip de Swagger UI sur GitHub, à l'adresse `https://github.com/swagger-api/swagger-ui/archive/refs/tags/v5.32.6.zip`, puis l'extrait dans `OUT_DIR` (source de la crate 10.0.1, `build.rs`). La compilation exige donc un accès réseau, sauf si la feature `vendored` est activée : le zip est alors lu depuis la crate `utoipa-swagger-ui-vendored`, embarquée dans le binaire de build. Une variable d'environnement, `SWAGGER_UI_DOWNLOAD_URL`, permet de changer la source sans toucher au code.

`docs/07` refuse déjà ce genre de dépendance pour `ort` : « la feature `download-binaries` d'`ort` n'est pas utilisée : elle ferait entrer un binaire non contrôlé dans le build » (`docs/07:54`). La CI (`docs/07:144`) et l'image Docker de P8 (`docs/07:112-125`) suivent le même principe : un build ne dépend pas d'un service tiers disponible à cet instant.

**Décision.** `utoipa-swagger-ui` est activée avec la feature **`vendored`**, ainsi qu'avec la feature d'intégration `axum` que la crate exige pour être montée sur un routeur axum. Aucun téléchargement n'a lieu pendant la compilation.

**Conséquences.** Le build est reproductible hors réseau, et la version de Swagger UI est celle que la crate embarque : elle ne change pas d'un build à l'autre sans changement de version de `utoipa-swagger-ui`. Coût : la taille du binaire de `crible-server` augmente du poids des fichiers de l'UI (HTML, JavaScript, CSS) ; mettre à jour Swagger UI demande de mettre à jour la crate, pas de changer une variable.

### Décision 2 — Features et versions des deux crates

**Constat.** `utoipa` 6 a pour features par défaut `macros` seulement. Sa feature `axum_extras` est celle qui donne aux macros la connaissance des extracteurs d'axum (`Path`, `Query`, `Json`, `State`), donc des chemins et des paramètres d'une route axum. Sans elle, chaque paramètre se décrit à la main. `Cargo.toml:42-44` n'active ni ne désactive aucune feature.

**Décision.**

- `utoipa` : version `=6.0.0` inchangée, features par défaut plus **`axum_extras`**.
- `utoipa-swagger-ui` : version `=10.0.1` inchangée, feature `vendored` (décision 1) et intégration `axum`.
- Les deux versions restent compatibles avec `axum = 0.8.9` (`Cargo.toml:20`) ; `utoipa-swagger-ui` 10.0.1 dépend d'`axum` 0.8.4, qui est dans la même série 0.8.
- `docs/07:32` reporte les deux crates avec leurs versions et leurs features.

**Conséquences.** Une seule déclaration de route alimente à la fois le routeur et la spécification, sans duplication de la description des paramètres. Coût : toute nouvelle version d'`axum` hors de la série 0.8 demandera de vérifier la compatibilité des deux crates, et `docs/07` est le seul endroit où les features sont écrites.

### Décision 3 — Version d'OpenAPI et en-tête `info`

**Constat.** `utoipa` 6 produit par défaut une spécification **OpenAPI 3.1.0**, dont les schémas suivent JSON Schema 2020-12. Rien dans `docs/05` ni dans `docs/07` ne fixe la version publiée, ni le contenu de `info.version`, ni la présence d'un tableau `servers`.

**Décision.**

- La spécification est publiée en **OpenAPI 3.1.0**, la valeur par défaut d'`utoipa` 6. Le choix est explicite parce que la décision 8 s'appuie sur JSON Schema 2020-12.
- `info.version` vaut la version du binaire, `CARGO_PKG_VERSION` : la spécification décrit le serveur qui la sert.
- Aucun tableau `servers` n'est publié. L'UI est servie par l'instance elle-même, et aucun hôte n'entre dans un artefact : la règle 11 d'`AGENTS.md` interdit de recopier une valeur de production dans un fichier versionné, un rapport ou un ADR, et un `servers` figé y mettrait un hôte.

**Conséquences.** Un client lit la version du serveur dans la spécification qu'il vient d'obtenir. L'absence de `servers` fait viser à « Try it out » l'origine qui a servi `/docs` : c'est l'instance interrogée, pas une adresse écrite d'avance. Coût : l'`info.version` change à chaque version du binaire, même quand l'API ne change pas ; un consommateur ne peut pas déduire d'elle la compatibilité, et doit lire les chemins et les schémas.

### Décision 4 — La sécurité dans la spécification

**Constat.** `docs/05:11-13` décrit un en-tête `Authorization: Bearer <clé>` et trois rôles hiérarchiques (`read`, `write`, `admin`). ADR-0100 fixe le format de la clé, l'analyse de l'en-tête et les réponses `401` et `403`. Quatre routes n'ont pas de rôle : `/health`, `/ready` (`docs/05:76`), `/openapi.json` et `/docs` (`docs/05:77`). Rien ne dit comment la spécification décrit tout cela, et une spécification muette ferait croire que les routes sont ouvertes.

**Décision.**

- un schéma de sécurité unique, `bearerAuth`, de type HTTP, schéma `Bearer`, qui décrit la clé d'API d'ADR-0100 ;
- l'exigence `bearerAuth` vaut **au niveau du document**, donc pour toutes les opérations ;
- `/health`, `/ready`, `/openapi.json` et `/docs` la neutralisent par `security: []` : elles restent sans authentification (ADR-0099, ADR-0100) ;
- le rôle requis (`read`, `write`, `admin`) est écrit dans la `description` de chaque opération, avec la valeur du tableau des routes de `docs/05`. Aucun des trois rôles ne se devine depuis la spécification seule : c'est un texte, pas un mécanisme d'autorisation.

**Conséquences.** Une route ajoutée sans `security: []` hérite de `bearerAuth` : l'oubli penche du côté sûr, et une route ouverte doit être déclarée comme telle. Le bouton « Authorize » de Swagger UI prend une clé et la rejoue sur chaque appel, ce qui permet de dérouler le scénario de J6 depuis l'UI. Coût : le rôle est une phrase, et un client qui génère du code à partir de la spécification ne peut pas en déduire la matrice des droits ; `docs/05` et ADR-0100 restent la référence pour cela.

### Décision 5 — La référence des routes est `docs/05`

**Constat.** La fiche écrit « un test compare la liste des routes de la spécification à celle de `05-api.md` » (`docs/phases/P6:134`). ADR-0015 (`:14`) et `docs/07:32` écrivent « générée depuis le code ». Prises à la lettre, les deux phrases se contredisent dès qu'une route existe dans le code sans ligne dans `docs/05`, ou l'inverse. Personne n'avait dit qui cède.

**Décision.**

- **`docs/05` fait foi** pour le contenu : la liste des routes de la spécification doit être **égale** à celle du tableau des routes (`docs/05:55-77`).
- La comparaison porte sur des **ensembles de couples (méthode, chemin)**, comparés **dans les deux sens** : une route présente d'un côté seulement fait échouer le test, dans les deux sens. Le chemin est pris **sans paramètre de requête** : `DELETE /v1/indexes/{index}?confirm={index}` compte pour `DELETE /v1/indexes/{index}`.
- Une ligne du tableau qui porte **deux routes séparées par `·`** compte pour deux routes. C'est le cas de `GET /v1/jobs/{job_id}` · `GET /v1/jobs` (`docs/05:68`), de `GET /health` · `GET /ready` (`:76`) et de `GET /openapi.json` · `GET /docs` (`:77`).
- `/health` et `/ready` **figurent** dans la spécification, avec leurs corps documentés.
- `/openapi.json` et `/docs` **ne figurent pas dans la spécification** : ce sont les deux seules exceptions nommées par le test, et elles le sont par leur chemin.
- **Toute route ajoutée au code est documentée dans `docs/05` dans le même changement.** C'est la règle qu'ADR-0101, décision 3, pose pour les clés de `details`, appliquée ici au tableau des routes.
- « Générée depuis le code » (ADR-0015, `docs/07`) reste vrai : il décrit le **moyen** de production de la spécification. `docs/05` reste la **référence du contenu**.

**Conséquences.** L'écart du verdict est fermé sans réécrire ADR-0015 : les deux phrases décrivent deux choses différentes. Une divergence est un échec de test, pas une question d'interprétation. Coût : deux endroits à tenir à jour dans le même changement, et le test dépend de la forme du tableau de `docs/05` (une ligne, un ou deux couples méthode-chemin, une colonne « Rôle ») : reformater le tableau sans prévenir casse le test, ce qui est le but.

### Décision 6 — Les erreurs dans la spécification

**Constat.** ADR-0101 fixe un format de corps d'erreur unique, avec une exception (`docs/05:25-53`). Le tableau des codes (`docs/05:36-50`) associe un statut HTTP à chaque code. Rien ne dit comment la spécification déclare ces corps ni ces statuts, et une opération sans réponse d'erreur déclarée laisse un client généré sans type pour les cas courants.

**Décision.**

- un **composant unique `Error`**, de la forme d'ADR-0101 (`error` objet, avec `code`, `message`, `details`, `request_id`), référencé par toutes les réponses d'erreur : un seul schéma, un seul `$ref`, pas un composant par code ;
- chaque opération déclare **ses statuts de succès** et **les statuts d'erreur que `docs/05` lui attribue** ;
- **`401` et `403`** sont déclarés sur **toute route à rôle**, en plus des statuts propres de la route ;
- `/ready` déclare son **`503`** avec le corps d'ADR-0099 : c'est la **seule exception** au format d'erreur, et elle est déclarée sur cette seule opération.

**Conséquences.** Un client généré dispose d'un type d'erreur unique et des statuts attendus, sans lire `docs/05`. Coût : la liste des statuts d'une opération est écrite deux fois — dans `docs/05` et dans l'annotation —, et rien ne prouve automatiquement leur égalité : les deux critères de la fiche portent sur les routes et sur les exemples, pas sur les statuts. Un écart de statut reste donc invisible au test (voir « À surveiller »).

### Décision 7 — La fermeture des schémas et les objets libres

**Constat.** Le serveur refuse une clé inconnue dans un corps (`ADR-0031` pour la configuration, `ADR-0109` pour la recherche, `ADR-0110` à tous les niveaux). Un schéma JSON Schema ouvert (`additionalProperties` absent ou `true`) décrirait un corps que le serveur refuse, et un client généré depuis la spécification enverrait des champs rejetés. À l'inverse, trois endroits du corps n'ont pas de forme fixe : `filter` (grammaire de `docs/05:192-226`), `return.fields` (`docs/05:243` et `:253`) et la configuration libre d'un schéma d'index, qui dépend de chaque index.

**Décision.**

- **`additionalProperties: false`** partout où le serveur refuse les clés inconnues, c'est-à-dire sur les objets de corps et de réponse dont la liste de champs est fermée (ADR-0031, ADR-0109, ADR-0110) ;
- `filter`, `return.fields` et la configuration libre d'un schéma d'index sont des **objets libres**, dont la **grammaire est décrite dans la `description`**. Ces formes dépendent du schéma de chaque index : un `oneOf` statique les figerait à une liste de champs connus à la compilation, fausse dès qu'un index déclare un champ de plus ;
- la spécification est **générée seulement** et servie par `GET /openapi.json` : **aucun fichier de spécification n'est versionné** ;
- le client Python de `CLIENT-PYTHON-HTTP` est **écrit à la main** avec `httpx` (`docs/07:52`), **sans générateur** : la spécification n'a pas de consommateur généré dans ce dépôt.

**Conséquences.** Un client généré depuis la spécification n'envoie pas de champ que le serveur refuserait, et un lecteur trouve la grammaire d'un filtre là où il la cherche. Un seul artefact fait foi, la spécification servie : pas de fichier à resynchroniser. Coût : la `description` d'un objet libre porte du texte que rien ne vérifie — une faute dans la grammaire de `filter` ne fait échouer aucun test ; et la fermeture des objets est une propriété du code (`deny_unknown_fields`, validateur) et de l'annotation, qui doivent dire la même chose.

### Décision 8 — La validation des exemples du document

**Constat.** Le second critère de la fiche demande que « les exemples du doc soient valides contre le schéma » (`docs/phases/P6:135`). Les blocs ```` ```json ```` de `docs/05` sont écrits à la main, souvent partiels : un bloc montre l'objet `details` d'une erreur, un autre le fragment d'une réponse. Aucun mécanisme ne relie un bloc à un composant de la spécification, et rien ne vérifie aujourd'hui que le document et le code disent la même forme.

**Décision.**

- **nouvelle dépendance de développement** de `crible-server` : `jsonschema` `=0.60.2`, avec `default-features = false`. Ses features par défaut sont `resolve-http`, `resolve-file`, `tls-aws-lc-rs` et `idna` : les activer ferait entrer une résolution réseau et une résolution de fichiers dans les tests. `crible-server` est le seul crate concerné, et la dépendance n'entre **pas** dans le binaire.
- Le test lit **chaque bloc ```` ```json ```` de `docs/05-api.md`**, le valide contre le schéma du composant qu'il illustre, **pris dans la spécification générée**, et **échoue sur un bloc sans association**. Il tourne dans le job Rust de la CI, sans réseau et sans ONNX Runtime.
- L'association est un **commentaire HTML invisible**, placé juste avant le bloc : `<!-- openapi: NomDuComposant -->`. Le nom est celui du composant dans `components.schemas` de la spécification générée.
- Un bloc qui n'illustre **aucun corps** porte `<!-- openapi: none -->`, et sa raison est dans le commentaire.
- Un exemple doit être du **JSON valide**. Les blocs partiels de `docs/05` sont **complétés** ou **réduits à un objet JSON valide du sous-composant qu'ils illustrent**, **sans changer leur sens** : un fragment `"debug": { ... }` devient l'objet du composant de debug. Les `…` **dans une chaîne** restent permis, puisque la chaîne reste du JSON valide.
- Les commentaires d'association et la mise en forme des exemples sont posés par la tâche, dans le même changement que la spécification : les noms de composants viennent du code, et cet ADR ne les fixe pas.

**Conséquences.** Le document cesse d'être une prose non vérifiée : un exemple qui contredit un schéma fait échouer la CI, et un bloc oublié aussi. Coût : deux dépendances de test de plus pour `crible-server` (`jsonschema` et ce qu'elle traîne), un commentaire à poser sur chaque bloc, et une réécriture des blocs partiels en objets valides ; la validation porte sur un composant, pas sur le contexte d'une opération, donc un exemple peut être valide contre son composant et faux dans l'opération qui l'utilise.

### Ce qui n'est pas décidé ici

- le **nom exact** des composants de schéma au-delà de `Error` et `bearerAuth` : ils viennent du code et de la tâche ;
- le **texte exact** des `description` d'opération, au-delà de la mention du rôle (décision 4) et de la grammaire des objets libres (décision 7) ;
- le **mécanisme** de lecture des blocs ```` ```json ```` et de résolution des `$ref` internes à la spécification : le choix appartient à l'implémentation, sous les contraintes de la décision 8 (aucun réseau, aucune dépendance de production) ;
- la **vérification automatique des statuts** d'erreur par opération : aucun test de cet ADR ne la couvre ;
- la **publication d'un fichier de spécification** versionné ou attaché à une release : la décision 7 l'écarte ;
- la **documentation OpenAPI dans `docs/06`** (analytics, journal de debug) : hors de la tâche, dont les routes sont celles du tableau de `docs/05`.

## Conséquences

- **Positives** :
  - le build ne dépend plus d'un téléchargement au moment de la compilation, comme pour `ort` (`docs/07:54`) ;
  - la spécification est servie par l'instance qu'elle décrit, avec sa version, et sans hôte écrit d'avance ;
  - une route non documentée dans `docs/05` fait échouer un test, dans les deux sens ;
  - la sécurité est lisible dans la spécification, et le bouton « Authorize » suffit à jouer le scénario de J6 ;
  - un corps fermé dans le code est fermé dans la spécification, donc un client généré n'envoie pas de champ que le serveur refuse ;
  - chaque exemple JSON de `docs/05` est prouvé valide contre un composant de la spécification générée, en CI, sans réseau ;
  - un seul composant `Error` et un seul schéma de sécurité : deux noms à retenir pour tout le document.
- **Négatives / coûts** :
  - le binaire de `crible-server` grossit du poids de Swagger UI embarquée, et la version de l'UI suit celle de la crate ;
  - la liste des statuts d'erreur est écrite deux fois, et rien ne compare les deux ;
  - la `description` des objets libres (`filter`, `return.fields`, configuration d'index) n'est vérifiée par aucun test ;
  - chaque bloc ```` ```json ```` de `docs/05` doit porter un commentaire d'association, et les blocs partiels ont dû être réécrits en JSON valide ;
  - `jsonschema` ajoute une arborescence de dépendances de test à `crible-server`, même sans résolution réseau ;
  - reformater le tableau des routes de `docs/05` (une ligne, un ou deux couples, une colonne « Rôle ») casse le test de la décision 5.
- **À surveiller** :
  - la première route ajoutée au code sans ligne dans `docs/05` : c'est le test de la décision 5 qui la révèle, et il ne doit pas être désactivé ;
  - un écart entre les statuts déclarés par une opération et ceux de `docs/05` : aucune décision ne le prouve automatiquement, et une divergence des deux sources est un défaut à corriger, pas une tolérance ;
  - le poids de l'image Docker en P8 : si Swagger UI embarquée pèse trop, c'est la question de la place de `/docs` en production qui se rouvre, pas le principe du `vendored` ;
  - la version de Swagger UI embarquée : une faille corrigée en amont n'arrive que par une mise à jour d'`utoipa-swagger-ui` ;
  - la montée d'`axum` hors de la série 0.8 : les deux crates devront être vérifiées ensemble (`docs/07:32`).

- **Documents mis à jour par cet ADR** :

| Document | Modification |
|---|---|
| `docs/adr/0111-specification-openapi-et-validation-des-exemples.md` | cet ADR |
| `docs/adr/README.md` | ligne d'index de cet ADR |
| `docs/07-stack-et-deploiement.md` | ligne OpenAPI (versions, features) et dépendance de développement `jsonschema` |
| `docs/05-api.md` | une phrase près de `/openapi.json` et `/docs` : version d'OpenAPI, `bearerAuth`, `docs/05` référence des routes |

Sont **non modifiés** : `docs/suivi.md` (tenu par l'orchestrateur), tout fichier de code et tout test, et les ADR antérieurs cités.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Le téléchargement par défaut de Swagger UI (zip GitHub au build) | Il ferait dépendre la CI et l'image de P8 d'un service tiers disponible au moment de la compilation : le même défaut que la feature `download-binaries` d'`ort`, déjà refusée (`docs/07:54`). |
| `SWAGGER_UI_DOWNLOAD_URL` vers un miroir ou un fichier local | C'est un réglage d'environnement : le build réussirait sur un poste configuré et échouerait ailleurs, et un fichier local ferait entrer un artefact non versionné dans le build. La feature `vendored` donne le même résultat, sans réglage. |
| Publier la spécification en OpenAPI 3.0 | 3.0 n'est pas aligné sur JSON Schema 2020-12, et la validation des exemples (décision 8) contre un schéma 3.0 aurait des règles différentes de celles de la validation du serveur. `utoipa` 6 produit 3.1 par défaut : choisir 3.0 serait un convertisseur à maintenir. |
| Un `servers` avec l'hôte de production | Aucun hôte n'entre dans un artefact (règle 11 d'`AGENTS.md`), et un `servers` figé ferait viser une autre instance à « Try it out » que celle qui sert `/docs`. |
| Le rôle dans un mécanisme de sécurité (scopes OAuth) | L'authentification est une clé d'API, pas OAuth (ADR-0100) : des scopes décriraient une mécanique qui n'existe pas. Le rôle est écrit dans la `description`, et `docs/05` reste sa référence. |
| Une spécification écrite à la main, le code s'y conformant | Elle divergerait au premier changement de type, et « générée depuis le code » (ADR-0015) serait perdu. Générer ne dispense pas d'une référence : elle est dans `docs/05` (décision 5). |
| Un fichier de spécification versionné, servi depuis le disque | Deux sources à resynchroniser, et un fichier qui peut décrire une version antérieure du binaire. La spécification est générée et servie (décision 7). |
| Générer le client Python depuis la spécification | `docs/07:52` fixe un client écrit à la main avec `httpx`, et la tâche `CLIENT-PYTHON-HTTP` en dépend : un générateur introduirait une chaîne d'outils et un style de code qui ne sont pas décidés. |
| Comparer chaque bloc ```` ```json ```` à l'annotation d'un test, sans le schéma | Cette comparaison prouve que deux textes se ressemblent, pas que l'exemple est valide contre le schéma : c'est exactement ce que la fiche demande (`docs/phases/P6:135`). |
| Désérialiser les exemples dans les types Rust de réponse | Il faudrait dériver `Deserialize` sur les types de réponse pour les seuls tests, et la désérialisation ne vérifie pas les annotations : un champ absent de la spécification passerait. La validation contre le schéma vérifie la spécification elle-même. |
| Activer les features par défaut de `jsonschema` | `resolve-http` et `resolve-file` ouvriraient une résolution réseau et une lecture de fichiers dans les tests, précisément ce qu'un test de documentation ne doit pas faire. |
| Valider tous les exemples contre un schéma unique de réponse | Les blocs illustrent des composants différents (corps d'erreur, enveloppes de liste, objets d'état) : un schéma unique serait faux pour la plupart, et l'association explicite par commentaire rend l'intention lisible. |
| Laisser les schémas ouverts (`additionalProperties` par défaut) | Le serveur refuse les clés inconnues (`ADR-0031`, `ADR-0109`, `ADR-0110`) : la spécification décrirait un corps accepté que le serveur rejette. |
| Un `oneOf` statique pour `filter`, `return.fields` et la configuration d'index | Leurs formes dépendent du schéma de chaque index, connu à l'exécution seulement. Un `oneOf` figé à la compilation serait faux dès qu'un index déclare un champ de plus. |

## Références

- `docs/phases/P6-serveur-et-api.md:129-135` — fiche `PUBLIER-OPENAPI` : objectif (`:130`), dépendances (`:131`), livrables (`:132`), critères d'acceptation (`:134-135`) ; `:99-111` — `ROUTE-RECHERCHE` ; `:113-127` — `RECHERCHE-MULTI-INDEX` ; `:137-150` — `CLIENT-PYTHON-HTTP`.
- `docs/07-stack-et-deploiement.md:25-50` — tableau des dépendances ; `:31` (features d'`axum`, `tokio`, `tower`, `tower-http`), `:32` (ligne OpenAPI), `:50` (row « Tests »), `:52` (`httpx` côté Python), `:54` (refus de `download-binaries` d'`ort`), `:112-125` (Docker), `:144` (job `rust-onnx`, CI sans ONNX Runtime).
- `docs/05-api.md` : `:11-13` (rôles), `:25-53` (format d'erreur, tableau des codes), `:55-77` (tableau des routes ; `:68` deux routes sur une ligne, `:76` `/health` et `/ready`, `:77` `/openapi.json` et `/docs`), `:192-226` (grammaire de `filter`), `:228-253` (« Recherche : requête »), `:255-306` (« Recherche : réponse »), `:308-346` (« Recherche multi-index »).
- `Cargo.toml:20` (`axum = 0.8.9`), `:42-44` (`utoipa = 6.0.0`, `utoipa-swagger-ui = 10.0.1`, sans feature).
- `crates/crible-server/src/auth.rs:276` — les quatre routes hors authentification.
- `docs/adr/0015-api-rest-anglais-licence-nom.md:14` — « la spécification OpenAPI est générée depuis le code (Swagger UI) ».
- ADR-0031 (clé inconnue dans un `PATCH` de configuration) ; ADR-0099 (`/health`, `/ready`, corps du `503`) ; ADR-0100 (clés d'API, `Bearer`, `401` et `403`) ; ADR-0101 (corps d'erreur, clés de `details`, `404` hors route) ; ADR-0109 (champs inconnus, périmètre de l'échéance) ; ADR-0110 (recherche multi-index, `pool_size` et `generation` dans `targets`).
- Source de `utoipa-swagger-ui` 10.0.1 (`build.rs`) : téléchargement par défaut du zip `v5.32.6` sur GitHub, variable `SWAGGER_UI_DOWNLOAD_URL`, feature `vendored` lisant `utoipa-swagger-ui-vendored`.
- `jsonschema` 0.60.2 (crates.io) : features par défaut `resolve-http`, `resolve-file`, `tls-aws-lc-rs`, `idna`.
- Verdicts de `@architecte` du 2026-10-30 sur la fiche `PUBLIER-OPENAPI` : douze `NON-COUVERT` (livraison de Swagger UI, features et versions, version d'OpenAPI et `info`, sécurité, autorité de `docs/05`, erreurs et statuts, fermeture des schémas, validation des exemples) et un `ECART` (« générée depuis le code » contre `docs/05` référence des routes).
