# ADR-0161 — ONNX Runtime de l'image tiré de la roue PyPI épinglée

- **Statut** : Proposé
- **Date** : 2026-11-22
- **Décideurs** : propriétaire
- **Proposé par** : agent `@greffier`, sur la demande de l'orchestrateur, à partir du verdict
  `NON-COUVERT` de `@architecte` du 2026-11-22 sur la source d'ONNX Runtime dans l'image
- **Remplace** : — (complète ADR-0057 ; laisse ADR-0128 intact)

## Contexte

### Ce qui a échoué à la publication de la 1.1.0

Le tag annoté `v1.1.0` a été posé par le propriétaire sur `7746c98`, puis `main` et le tag ont été
poussés. La fusion `dev` → `main`, la version `1.1.0` et la note de version sont en place
(`docs/suivi.md`, journal du 2026-11-22), mais **aucune image `1.1.0` n'existe** : le run
`45039326345` de `publish-image.yml` a échoué en 1 min 3. La garde de version est passée, puis les
deux constructions, `linux/arm64` et `linux/amd64`, ont échoué à l'étape `runtime` du
`docker/Dockerfile`, sur `curl: (22) … 404` en téléchargeant ONNX Runtime 1.31.0.

L'image `1.0.0` n'est pas touchée : elle est publiée (run `42489535885`) et reste récupérable. Rien
n'a changé dans le `Dockerfile` ni dans `publish-image.yml` depuis `v1.0.0` : **la cause est
extérieure au dépôt**.

### La cause : le dépôt amont n'est plus servi

Le `Dockerfile` télécharge les deux archives officielles des publications GitHub de
`microsoft/onnxruntime` — l'archive `x64` et l'archive `aarch64`, choisies par `TARGETARCH`
(`docker/Dockerfile:18-24`, `:75-78`, `:87-105`). Depuis le 17 ou le 18 novembre 2026, Microsoft a
rendu privé ce dépôt et plusieurs dépôts liés. Un porte-parole, cité par The Register le
2026-11-18 : « We have made some repositories private while we investigate malicious content ».
**Aucune date de retour n'est annoncée.**

Le contexte est la compromission du paquet PyPI `agent-framework` de Microsoft (versions 1.26.1 et
1.26.2). **Aucune source ne dit compromis les paquets d'ONNX Runtime** : PyPI, NuGet et Maven
Central les servent toujours. Le journal de `docs/suivi.md` du 2026-11-22 donne ces faits et leurs
sources ; c'est là que le projet les tient.

### Aucun ADR ne fixe la source d'ONNX Runtime dans l'image

- **ADR-0128, décision 6**, fixe la télémétrie, pas la provenance : `ORT_DISABLE_TELEMETRY=1` est
  posée par `ENV` dans l'image (`docker/Dockerfile:111-113`).
- **ADR-0128** ne dit pas non plus d'où vient la bibliothèque, et son corps est accepté.
- **`docs/07:121-125`** décrit l'image et nomme « la bibliothèque ONNX Runtime CPU de l'architecture
  visée » sans dire où elle est prise.
- La source n'est écrite que dans **`docker/README.md:23-27`** (« jamais d'un binaire non
  contrôlé », téléchargement « des versions officielles du dépôt `microsoft/onnxruntime` ») et dans
  les commentaires du `Dockerfile`. Aucune de ces deux places n'est une décision.

### La roue PyPI est déjà la source de la CI, et elle est épinglée

- **ADR-0057, décision 1** (`:56`, `:79`) fait de la roue `onnxruntime==1.31.0` la source de la
  bibliothèque Linux en intégration continue, et `ORT_DYLIB_PATH` désigne
  `onnxruntime/capi/libonnxruntime.so.1.31.0`.
- `.github/workflows/ci.yml:174-180` installe le groupe `ci` (`uv sync --group ci`), prend
  `libonnxruntime.so.1.31.0` dans la roue et l'expose par `ORT_DYLIB_PATH`. Le job `rust-onnx`
  passe toute la suite `--include-ignored`, sauf les tests exclus par leur nom (ADR-0062,
  ADR-0063, ADR-0085, ADR-0109), dont celui de télémétrie d'ADR-0128.
- **`tools/uv.lock`** épingle les roues `onnxruntime-1.31.0` `manylinux_2_28` x86_64 (`:850`) et
  aarch64 (`:849`) par SHA-256, de `cp311` à `cp314`, déposées sur PyPI le **2026-10-09**, donc
  **avant l'incident**. PyPI interdit de redéposer un fichier de même nom : le fichier qu'une somme
  désigne ne peut pas être remplacé en place.
- **`debian:bookworm-slim`** a la glibc 2.36, au-dessus du plancher 2.28 de l'étiquette
  `manylinux_2_28`.

### Ce qui n'est pas prouvé à ce jour

Ces points ne sont pas des doutes de rédaction : ils sont **à établir avant publication**, et la
décision 5 en fait des vérifications.

1. **Le contenu complet de `capi/`** dans la roue : `libonnxruntime.so.1.31.0` est certain, mais la
   présence d'autres fichiers, notamment `libonnxruntime_providers_shared.so`, n'est pas établie.
2. **Les dépendances du `.so`** : son édition de liens (glibc, `libstdc++`, `libgcc_s`) n'a pas été
   relevée dans l'image d'exécution.
3. **L'égalité du `.so` entre roues `cp` différentes** : la même bibliothèque est-elle identique
   dans la roue `cp311` et dans la roue `cp313` ?
4. **Tout essai `aarch64`** : aucun job de CI ne le couvre, alors que c'est l'architecture des deux
   Mac. L'image arm64 n'a donc **jamais** chargé une bibliothèque venue de la roue.

## Décision

**La bibliothèque ONNX Runtime de l'image vient de la roue PyPI `onnxruntime==1.31.0` de
l'architecture visée, téléchargée à l'adresse que `tools/uv.lock` donne et vérifiée par la même
somme SHA-256 que le fichier de verrouillage. Le téléchargement depuis GitHub quitte le
`Dockerfile` : une seule source pour l'image et pour la CI.**

### 1. La source

Pour la plateforme `linux/amd64`, la roue est `onnxruntime-1.31.0-cp311-cp311-manylinux_2_28_x86_64.whl`;
pour `linux/arm64`, `onnxruntime-1.31.0-cp311-cp311-manylinux_2_28_aarch64.whl`. La roue est donc
**nommée explicitement**, par architecture et par l'étiquette `cp311`, la première épinglée dans
`tools/uv.lock` (`:849` et `:850`). L'étiquette `cp` n'a pas de sens pour une bibliothèque native :
elle nomme le fichier que PyPI sert et que le verrou contrôle, rien de plus.

- **L'adresse** est celle que `tools/uv.lock` donne pour ce fichier ; elle n'est pas recopiée dans
  le `Dockerfile` à la main sans le verrou.
- **La somme** est celle du verrou, à l'octet près.
- **Une seule source** : le téléchargement depuis les publications GitHub de
  `microsoft/onnxruntime` est retiré, et ses deux variables de somme (`ORT_SHA256_AMD64`,
  `ORT_SHA256_ARM64`) avec lui.
- **Ce qui est extrait** : `libonnxruntime.so.1.31.0` et, **seulement**, les bibliothèques que la
  vérification de la décision 5 montre nécessaires au chargement. La liste part de la roue, pas
  d'une supposition sur le contenu de `capi/`.

### 2. L'extraction

Un **étage de construction dédié** — distinct de `chef`, `planner`, `builder` et `runtime` —
télécharge la roue, vérifie sa somme et l'extrait. Une roue est une archive ZIP : l'étage installe
`unzip`, que l'étage `runtime` ne reçoit pas.

- **L'étage `runtime` ne reçoit que la ou les bibliothèques**, par `COPY --from`. Aucun outil
  nouveau n'entre dans l'image d'exécution : ni `unzip`, ni l'archive, ni le reste de la roue.
- **L'environnement ne change pas** : `ORT_DYLIB_PATH=/opt/ort/libonnxruntime.so.1.31.0`,
  `LD_LIBRARY_PATH=/opt/ort` et `ORT_DISABLE_TELEMETRY=1` (`docker/Dockerfile:109-113`) gardent
  leur valeur et leur sens, et ADR-0128 n'est pas touché.
- **La vérification de la somme précède l'extraction**, comme aujourd'hui : une archive qui ne
  correspond pas fait échouer la construction.

### 3. Les montées d'ONNX Runtime

`ORT_VERSION`, les noms et les sommes des roues de l'image **et** ceux de `tools/uv.lock` bougent
**ensemble**, par un ADR. La ligne de `docker/README.md` qui promet l'alignement avec le groupe `ci`
de `docs/07` reste vraie, et ADR-0057, `:202`, qui exige une décision pour une mise à jour du
runtime, vaut aussi pour l'image.

### 4. Le retour de GitHub

Si GitHub sert de nouveau les archives, **la roue reste la source**. Revenir aux archives demande
un nouvel ADR. Le motif est simple : deux sources pour la même bibliothèque feraient deux
bibliothèques à tenir alignées, et la CI n'a jamais cessé d'utiliser la roue.

NuGet et Maven Central sont des alternatives écartées (voir le tableau) : mêmes archives à
extraire, aucune somme déjà épinglée dans le dépôt.

### 5. Les vérifications avant publication

`@dev` exécute les vérifications suivantes et les consigne dans son compte rendu, avant toute
publication :

1. **la somme de chaque roue** téléchargée est égale à celle de `tools/uv.lock` ;
2. **la liste de `capi/`** dans les deux roues ;
3. **`readelf -d` et `ldd` sur le `.so` dans l'image**, pour nommer ses dépendances et vérifier
   qu'aucune ne manque dans `debian:bookworm-slim` ;
4. **la somme du `.so` comparée entre deux roues `cp`** (par exemple `cp311` et `cp313`), pour
   savoir si la bibliothèque dépend de la version de Python ;
5. **la construction des deux images**, `linux/arm64` et `linux/amd64` ;
6. **dans l'image arm64 au moins** : `crible --version`, `/ready`, une recherche, et la recette
   d'INV-12 sur l'échantillon `legifacile-sample`
   (`inv_12_the_legifacile_sample_passes_the_acceptance_check`, ADR-0057 `:34`).

Le point 6 est un **essai local de l'image**, hors de la CI, comme le veut ADR-0132 pour les essais
en conteneur. Ce n'est **pas une mesure MES nouvelle** : aucun rapport de `docs/mesures/` n'est
écrit, et aucune affirmation de performance n'en sort.

### 6. Les documents à appliquer après acceptation

| Document ou livrable | Ce que l'application y écrit |
|---|---|
| `docker/README.md:23-27` | la source : roue PyPI `manylinux_2_28` de l'architecture visée, somme du verrou, plus de téléchargement GitHub |
| `docker/Dockerfile` (`:18-21`, `:87-90`) | les commentaires de la source et des sommes, avec le renvoi à cet ADR |
| `docs/07-stack-et-deploiement.md` | la ligne de l'image (`:123`) et la section « ONNX Runtime » nomment la roue PyPI comme source, avec le renvoi |
| `docs/adr/0057-…md` | un **renvoi daté**, corps non réécrit, sans « Remplacé par » : la décision 1 d'ADR-0057 reste vraie pour la CI et s'étend à l'image |
| `docs/versions/1.1.0.md` | une ligne sur la source d'ONNX Runtime de l'image (option A) ou la note de la version publiée (option B) |

**Inchangés** : `docs/adr/0128-…md`, son corps et sa décision 6 ; `docs/05`, `docs/08` ; le
`Dockerfile` de l'étape de compilation ; `publish-image.yml`. Aucun changement de code Rust, de
contrat d'API et de `format_version`.

### 7. La livraison du correctif : deux options

Le correctif se livre par une des deux voies suivantes. **La recommandation de l'orchestrateur est
(A).** Le choix revient au propriétaire.

**(A, recommandée) Déplacer le tag `v1.1.0` sur le commit qui porte le correctif.**

- **Précédent** : pour `v1.0.0`, le 2026-11-08, `@git` a refusé `git tag -d` au titre d'ADR-0133,
  décision 7 ; le propriétaire a donc lui-même supprimé puis reposé le tag annoté sur le commit
  voulu, puis poussé `main` et le tag. Voir le journal de `docs/suivi.md` et la section « Contenu de
  ACTUALISER-DEPOT-APRES-V1 ».
- **Pourquoi c'est sans risque ici** : **aucune image `1.1.0` n'existe**, donc personne n'en
  dépend. L'image `1.0.0` et l'étiquette `:cpu` ne sont pas touchées.
- **Ce qui ne bouge pas** : `Cargo.toml` reste à `1.1.0`, et la garde de version de
  `publish-image.yml` continue de passer.
- **Ce qui bouge** : `docs/versions/1.1.0.md` gagne une ligne sur la source d'ONNX Runtime, car la
  note décrit l'image que l'exploitant va tirer.
- **Le geste est celui du propriétaire.** Le tag est déjà sur `origin` : il faut le supprimer sur
  `origin` puis le repousser. Aucun agent ne le fait (`AGENTS.md`, interdits de git ; ADR-0133,
  décision 7).

**(B) Publier une `1.1.1`.**

- **Le correctif y est permis** : ADR-0143, décision 1, définit le correctif comme un changement
  qui ne touche **aucun contrat** — ni route, ni champ, ni clé, ni code d'erreur, ni
  `format_version`. Changer la provenance d'une bibliothèque dans l'image n'en touche aucun.
- **Ce qui bouge** : `Cargo.toml`, `Cargo.lock`, le client Python (`pyproject.toml`,
  `_version.py`, `uv.lock`), `tools/uv.lock`, la somme de `synth-small` et `README.md` passent à
  `1.1.1`. C'est le passage A de `PUBLIER-V1.1.0`, rejoué pour un numéro de plus.
- **`v1.1.0` reste alors un tag sans image.** Il faut le dire, pour qu'un lecteur du dépôt ne
  cherche pas une image qui n'existe pas.
- **Note de version** : ADR-0143, décision 4, n'exige une note que pour une `1.x.0`. **Il est
  proposé d'écrire `docs/versions/1.1.1.md`** : la provenance d'une bibliothèque embarquée est une
  information d'exploitation, et une note courte coûte moins qu'une question d'exploitant.

**Pour les deux options** : l'étiquette `:cpu` ne bouge **qu'après une publication
multi-architecture réussie** (ADR-0133, décisions 4 et 5). Un run en échec ne déplace rien.

### Ce qui n'est pas décidé ici

- **La liste exacte des bibliothèques de `capi/`** que l'image doit porter : la décision 5 la
  produit, et la décision 1 la suit.
- **Le sort du run `45039326345`** : le relancer est inutile tant que les archives GitHub rendent
  404, et cette ADR ne prescrit aucune relance.
- **Le choix entre (A) et (B)**, et le geste sur le tag : propriétaire.
- **Une couverture `aarch64` en CI** : aucun job ne la donne aujourd'hui, et l'ajouter serait une
  décision de CI à part entière (ADR-0133 pour la publication, ADR-0132 pour les essais en
  conteneur). L'essai de la décision 5 est local.
- **Les montées d'ONNX Runtime** : elles restent soumises à un ADR (décision 3), sans calendrier.
- **Le choix d'une autre version d'ONNX Runtime** : cette ADR ne change pas la version, seulement
  la provenance.

## Conséquences

- **Positives** :
  - l'image et la CI chargent **la même bibliothèque**, venue du même fichier épinglé : une
    divergence entre les deux ne peut plus passer inaperçue en publication ;
  - la somme qui protège le téléchargement est **déjà dans le dépôt** (`tools/uv.lock`), versée
    avant l'incident, et PyPI interdit de redéposer un fichier de même nom ;
  - la construction ne dépend plus de la disponibilité d'un dépôt GitHub rendu privé sans date de
    retour ;
  - l'image d'exécution ne gagne **aucun outil** : `unzip` reste dans l'étage de construction ;
  - `ORT_DYLIB_PATH`, `LD_LIBRARY_PATH` et la télémétrie d'ADR-0128 ne changent pas ;
  - aucun contrat d'API, aucun format persistant, aucun code Rust.
- **Négatives / coûts** :
  - la construction de l'image dépend désormais de **PyPI** et de `files.pythonhosted.org` ; une
    indisponibilité de PyPI bloquerait une publication, sans repli automatique ;
  - **l'étiquette `cp311`** est un choix de fichier, pas une propriété de la bibliothèque : la
    montée de version devra suivre les étiquettes que PyPI sert encore ;
  - la roue pèse de l'ordre de 21 à 24 Mo pour une bibliothèque dont seule une partie entre dans
    l'image ; l'étage de construction paie le téléchargement et l'extraction à chaque construction
    sans cache de couche ;
  - **`aarch64` n'est prouvé par aucun job** : la décision 5 le vérifie localement, une fois, et
    rien ne le rejouera à la prochaine montée ;
  - l'égalité du `.so` entre roues `cp` n'est pas acquise, et la décision 5 la mesure au lieu de la
    supposer ;
  - un étage de construction de plus dans le `Dockerfile`.
- **À surveiller** :
  - **le retour des archives GitHub** : s'il survient, la roue reste la source (décision 4), et un
    ADR serait nécessaire pour revenir en arrière ;
  - **une nouvelle version d'`onnxruntime` sur PyPI** : l'alignement de l'image et de
    `tools/uv.lock` est une exigence (décision 3), et un décalage se verrait par deux bibliothèques
    différentes entre l'image et la CI ;
  - **le contenu de `capi/`** : une bibliothèque nouvelle dans la roue, non copiée, ferait échouer le
    chargement à l'exécution alors que la construction aurait réussi ;
  - **la somme d'une roue qui ne correspond plus au verrou** : c'est un échec de construction, pas
    un avertissement, et il faut alors décider, pas desserrer la vérification ;
  - **la disponibilité de PyPI** le jour d'une publication : l'absence de repli est assumée.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| **Attendre le retour de GitHub sans date** | C'était la première décision du propriétaire du 2026-11-22 (« Attendre puis relancer »), et elle a été tenue jusqu'à 17:50 : les archives répondaient encore 404. Aucune date n'est annoncée, la 1.1.0 n'a pas d'image, et attendre laisse la publication indéfiniment en suspens alors qu'une source épinglée existe déjà dans le dépôt |
| **NuGet** | Les paquets NuGet d'ONNX Runtime portent les mêmes archives natives à extraire, sans aucune somme déjà épinglée dans le dépôt : il faudrait en relever une, la versionner et la tenir à jour, pour un gain nul sur le fichier obtenu |
| **Maven Central** | Même raison : des archives à extraire, aucune somme dans le dépôt, et un écosystème de plus à connaître pour une bibliothèque native |
| **Construire ONNX Runtime depuis les sources** | Le dépôt amont n'est plus servi, donc les sources ne sont pas téléchargeables ; la construction demanderait en plus `cmake`, une chaîne C++ complète et des heures par architecture, pour une bibliothèque que la roue fournit déjà compilée et vérifiée |
| **Garder deux sources** (GitHub quand il répond, roue en secours) | Deux chemins à tenir, deux sommes à maintenir, et une image qui dépend du chemin emprunté le jour de la construction : la même version publierait alors deux bibliothèques possibles. Une seule source, la roue, et un ADR pour en changer |

## Références

- **Constat** : `docs/suivi.md`, journal du 2026-11-22 (échec du run `45039326345`, constat des 404,
  contexte de la compromission d'`agent-framework`, sources de presse), et section « Contenu de
  ACTUALISER-DEPOT-APRES-V1 » pour le précédent du tag `v1.0.0` repointé.
- **Image** : `docker/Dockerfile:18-24` (arguments `ORT_VERSION` et sommes), `:75-78` (arguments de
  l'étage `runtime`), `:87-105` (téléchargement, vérification, extraction), `:109-113`
  (`ORT_DYLIB_PATH`, `LD_LIBRARY_PATH`, `ORT_DISABLE_TELEMETRY`).
- **Documentation de l'image** : `docker/README.md:23-27` (source de la bibliothèque),
  `docs/07-stack-et-deploiement.md:121-125` (section « Docker »).
- **CI** : `.github/workflows/ci.yml:174-180` (groupe `ci`, `ORT_DYLIB_PATH` sur la roue),
  `publish-image.yml` (garde de version, deux architectures, `:cpu`).
- **Verrouillage** : `tools/uv.lock:849` (roue `manylinux_2_28` aarch64, `cp311`),
  `:850` (roue `manylinux_2_28` x86_64, `cp311`), sommes SHA-256 et date de dépôt du 2026-10-09.
- **ADR** : ADR-0057 (décision 1, `:56` et `:79`, roue PyPI en CI ; `:202`, une mise à jour du
  runtime demande une décision ; `:34`, nom du test d'INV-12), ADR-0128 (décision 6, télémétrie),
  ADR-0132 (essais en conteneur hors CI), ADR-0133 (décisions 4, 5 et 7, `:cpu` et tag du
  propriétaire), ADR-0143 (décisions 1 et 4, correctif sans contrat et note de version des
  mineures).
- **Note de version** : `docs/versions/1.1.0.md` (première note, ADR-0143, décision 4).
- **Presse** : The Register, 2026-11-18, propos de Microsoft sur les dépôts rendus privés ; fil
  Hacker News `item?id=50539610` ; `runtimewire.com`. Aucune de ces sources ne dit compromis les
  paquets d'ONNX Runtime.
