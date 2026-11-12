# ADR-0152 — `sizes.disk_bytes` documenté, et un membre `sizes.live_bytes` stable

- **Statut** : Accepté (accepté par anticipation par le propriétaire le 2026-11-12)
- **Date** : 2026-11-12
- **Décideurs** : propriétaire
- **Proposé par** : agent `@greffier`, tâche `T10-DISK-BYTES-STABLES` (v1.1), sur demande de l'orchestrateur et sur le verdict de `@architecte` du 2026-11-12
- **Remplace** : — (complète ADR-0105, décision 3, sans modifier son corps ; ne change ni ADR-0117, ni ADR-0124, ni `docs/05`)

> **Erratum du 2026-11-12, sur la relecture de la tâche `T10-DISK-BYTES-STABLES`.** Trois corrections de forme, aucune décision ne change.
>
> 1. Le Contexte comptait les dossiers `deleted-*` parmi ce que la marche de `disk_bytes` voit. C'est faux : le job `delete_index` renomme le dossier de l'index sous `<data>/tmp/deleted-<nom>-<job_id>`, **hors** de `<data>/indexes/<nom>` (`docs/02-architecture.md:67` et `:150`, ADR-0105, décision 6, ADR-0150), et `directory_size` ne parcourt que le dossier de l'index : il ne peut donc jamais compter un `deleted-*`. La phrase du Contexte est corrigée en conséquence, et `deleted-*` disparaît aussi des listes qui l'avaient recopié : la liste des exclusions de la décision 2 et la ligne d'application de `docs/05`. La décision 1 reste valable telle quelle, et son sens n'est pas modifié.
>
> 2. La décision 2 nomme `GET /v1/indexes/{index}` comme seule route portant `live_bytes`, et c'est exact au sens du contrat : le `201` de `POST /v1/indexes` rend le **même corps** que ce `GET` (même composant `IndexDetail`, `detail` partagé, `crates/crible-server/src/indexes.rs:97`), donc le même `state.sizes`, où `live_bytes` vaut `0` sur un index neuf. La décision 5 se lit donc ainsi : l'exposition est limitée aux réponses qui portent `IndexDetail`, c'est-à-dire le `GET` du détail et le `201` de la création. Aucune autre route n'est concernée. Le titre de la décision 5, « Exposition limitée à une route », devient « Exposition limitée au corps du détail », ce que le fond ne change pas.
>
> 3. La décision 4 portait une phrase mal construite sur l'effet d'une compaction (« fusionne plusieurs segments en un découpage … en retire ») ; elle est réécrite sans changer la règle : la somme des vecteurs reste invariante, et la compaction retire le `segment.json` de chaque segment supprimé.

## Contexte

### Le ticket, et ce qu'il constate

Le ticket 10 du projet Legifacile (`../legifacile/tickets-crible/10-disk-bytes-instables.md`) porte sur la version 1.0.0. `sizes.disk_bytes` de `GET /v1/indexes/{index}` est la taille du dossier de l'index, mesurée par une marche du système de fichiers (`crates/crible-server/src/indexes.rs:578`, `directory_size`). Cette marche compte tout ce que le dossier contient :

- les fichiers `records.sqlite-wal` et `-shm`, dont la taille dépend des points de contrôle de SQLite ;
- les dossiers `tmp-*` d'une attache en cours ;
- les `codes.bin` et le dossier `cache/`, qui sont **dérivés** et se reconstruisent (`docs/02-architecture.md:80`, `:87`, ADR-0069) ;
- les dossiers de segment **absents de la table `segments`**, que la reprise supprime à la prochaine ouverture en écriture (`crates/crible-store/src/generation.rs:57`, `docs/02-architecture.md:144-152`).

Le même index rend donc deux valeurs différentes sans qu'une ligne de données ait changé, et sa valeur **baisse au redémarrage** quand la reprise efface un résidu. Sur le Crible de développement, l'index `articles` rend `7 420 957 206` octets alors que son WAL ne pèse que 4,2 Mo et qu'aucun segment excédentaire n'existe : l'écart actuel est faible, mais il n'est borné par aucune construction.

Legifacile veut un chiffre **stable** pour l'état du service et pour l'estimation d'une sauvegarde. Un chiffre qui inclut des orphelins surestime le besoin, et un chiffre qui chute au redémarrage fait croire à une perte. `docs/suivi.md` cite d'ailleurs `disk_bytes` pour suivre la croissance des index de P2, et la comparaison entre deux jours n'est fiable que si aucun WAL ni `tmp-*` ne traîne.

### Ce que la conception fixe déjà, et ce qu'elle interdit

- **`disk_bytes` compte tous les fichiers.** ADR-0117, décision 2, écrit que `disk_bytes` est « le total occupé par le dossier de l'index à l'instant de la mesure, **tous fichiers confondus** ». ADR-0105, décision 3, et `docs/05-api.md:134` le répètent : « la taille du dossier de l'index sur disque ».
- **Changer le sens d'une valeur est interdit dans la ligne 1.x.** ADR-0143, décision 1, range « changer le sens d'une valeur » parmi les changements interdits dans une mineure. Filtrer la marche de `disk_bytes` serait exactement cela : un client qui compare la valeur à `du` verrait un écart non documenté, et une 1.0 et une 1.1 ne rendraient plus le même nombre. **`disk_bytes` ne change pas.**
- **Ajouter un membre est permis.** ADR-0143, décision 1, liste « un membre de réponse nouveau » parmi les ajouts compatibles d'une mineure. Un client 1.0 rend le JSON en `dict` sans type déclaré (ADR-0112) : un membre de plus ne casse rien.
- **Le corps de `GET /v1/indexes` est fermé** (ADR-0105, décision 3), et cette route est servie pendant le démarrage, ce que T06 et ADR-0146 viennent de traiter : elle ne reçoit aucun membre nouveau.
- **`/v1/stats` a des membres fixes** (ADR-0124, décision 8), et `disk_bytes` y est **la seule taille stockée** (`docs/adr/0124-route-stats-forme-parametres-seaux-et-cli.md:146`). Ajouter une colonne à `analytics.sqlite` serait un changement de format, interdit en 1.x (ADR-0143, décision 2) et contraire à INV-15.
- **Le coût de la marche est mesuré.** `docs/mesures/routes-en-conteneur-2026-11-03.md:125` donne `sizes.disk_bytes` à **0,0483 ms** en p50 et **0,104 ms** en p95, sur 31 fichiers et 16 Go d'index. La marche n'est donc pas un problème de coût ; c'est sa **source** qui ne convient pas à un chiffre stable.

### Ce que le code permet, vérifié le 2026-11-12

- **L'état publié porte les segments.** `Snapshot::segments()` rend les `SegmentMeta` de la génération publiée (`crates/crible-store/src/generation.rs:421`). `SegmentMeta` porte `count`, `dims`, `precision` et `checksums` (`crates/crible-store/src/segment.rs:115-134`).
- **La taille des vecteurs se calcule.** `SegmentMeta::vectors_len` vaut `count × dims × bytes_per_component(precision)`, avec 2 octets en fp16 et 4 en fp32 (`segment.rs:86-91`, `:138-162`). La fonction est privée aujourd'hui ; elle ne lit aucun fichier.
- **La taille de `segment.json` se calcule aussi.** Le fichier est écrit en JSON canonique par `manifest_json` (`segment.rs:1097-1113`), à partir de sept champs : `batch_id`, `checksums`, `count`, `dims`, `format_version`, `generation`, `iid_start`. **Tous sont dans `SegmentMeta` en mémoire**, `checksums` compris (une entrée, `blake3:` suivi de 64 hexadécimaux). Sérialiser les mêmes champs avec la même fonction donne donc exactement les octets du fichier, sans `stat`. Un `segment.json` est de l'ordre de quelques centaines d'octets.
- **Le manifeste de génération ne porte aucune taille.** `manifests/<G>.json` reprend les champs de `segment.json` plus `segment_id` (`generation.rs:2822-2855`) : ni taille de fichier, ni taille de segment. Le ticket proposait de lire les tailles dans le manifeste ; **le manifeste ne les porte pas**, et cette source est donc écartée.
- **La compaction réécrit les segments et en change le nombre.** Elle recopie les passages vivants par tranches de `MAX_SEGMENT_PASSAGES = 1_000_000` (`crates/crible-store/src/compact.rs:97`, `:431-492`), en gardant la précision de l'index (`:451`) et sans réencoder aucun passage. Le nombre de segments est donc celui du découpage, pas celui de l'époque précédente.

## Décision

**`disk_bytes` garde son sens et gagne une définition documentée ; un membre nouveau, `sizes.live_bytes`, donne la taille des fichiers d'une génération publiée, calculée depuis l'état publié sans parcours du dossier.**

### 1. `disk_bytes` ne change pas, et sa définition est écrite

La marche reste la marche : tous les fichiers du dossier, WAL, `tmp-*`, `codes.bin`, `cache/`, segments non référencés compris. `docs/05` dit ce que la marche inclut et **pourquoi la valeur peut bouger entre deux appels et baisser au redémarrage** : la reprise supprime les segments absents de la table et les `tmp-*` (`docs/02-architecture.md:144-152`).

C'est la piste 1 du ticket, et elle est **sans risque** : aucun contrat ne change, et le membre garde son sens « espace occupé par le dossier », utile à l'exploitation. Elle est livrée dans cette tâche.

### 2. `live_bytes` est ajouté à `state.sizes` de `GET /v1/indexes/{index}` seulement

Membre **`live_bytes`**, **entier d'octets**, **toujours présent**, `0` pour un index sans segment. Il s'ajoute à `state.sizes`, à côté de `disk_bytes`, `filter_memory_bytes` et `codes_memory_bytes` (ADR-0105, décision 3). Aucune autre route, aucun autre document.

**Définition.** `live_bytes` est la **somme, sur les segments de la génération publiée**, de la taille de leur `segment.json` et de la taille de leur fichier de vecteurs (`vectors.f16` ou `vectors.f32`). C'est la partie vecteurs qu'un instantané copie (`docs/04-donnees-et-ingestion.md:331`).

**Exclus, et nommés :**

- `codes.bin` et `cache/` : dérivés, reconstruits, propres à la machine ;
- `manifests/` : vue dérivée d'une génération (ADR-0016) ;
- `records.sqlite`, son `-wal` et son `-shm` : leur taille dépend des points de contrôle ;
- `lexical/` : index dérivé, dont la taille dépend des fusions tantivy ;
- `tmp-*`, et tout dossier de segment absent de la table `segments`.

### 3. La source est l'état publié, et le calcul ne lit aucun fichier

`live_bytes` se calcule depuis `snapshot.segments()` (`generation.rs:421`), donc depuis la génération publiée, comme les autres comptes de `state` (ADR-0105, décision 3 ; ADR-0146 pour la fenêtre de démarrage). **Aucun appel au système de fichiers** n'a lieu sur le chemin de la route pour ce membre.

**Taille des vecteurs.** `count × dims × 2` ou `× 4` selon `precision`, exactement ce que `SegmentMeta::vectors_len` calcule déjà (`segment.rs:138-162`). `crible-store` rend cette grandeur accessible (`vectors_bytes` publique), le serveur ne la recalcule pas.

**Taille de `segment.json` : calculée, sans `stat`.** `crible-store` rend une seconde grandeur (`manifest_bytes`) qui sérialise, avec **la même fonction de sérialisation** que l'écriture (`manifest_json`), les sept champs de `SegmentMeta`. Le fichier est canonique et son jeu de clés est fermé par `format_version` : la longueur calculée est celle du fichier, à l'octet. Deux raisons de préférer ce calcul à un `stat` par segment :

- un `stat` ramènerait une **seconde source**, le disque, à côté de l'état publié : un fichier supprimé pendant la requête, ou un dossier de segment remplacé par une compaction, ferait varier la valeur sans nouvelle génération — exactement l'instabilité qu'on supprime ;
- un `<segment>/segment.json` absent obligerait à choisir entre une erreur (`internal`) et un zéro, donc à **inventer** un état, alors que le contrat dit que le membre est toujours présent et vaut la taille de la génération publiée.

Le coût est négligeable devant la marche mesurée : sérialiser quelques centaines d'octets par segment coûte moins que les 0,0483 ms d'un parcours de 31 fichiers (`docs/mesures/routes-en-conteneur-2026-11-03.md:125`), et le nombre de segments est du même ordre (37 dans l'exemple d'ADR-0105). Le dépassement du p95 de 5 ms d'ADR-0135 n'est pas en cause.

**Fichier référencé manquant.** Il n'y a pas de cas : la source ne lit aucun fichier. Un segment référencé par la génération publiée compte donc **sa taille déclarée**, même si son dossier a disparu du disque après publication. `live_bytes` n'est pas un contrôle de disponibilité ; une copie d'instantané qui ne trouve pas le fichier échoue avec son propre message, à son propre moment (ADR-0064, ADR-0065). Un `segment.json` ou un `vectors.*` illisible ou absent n'atteint jamais la route : `read_meta` lit le manifeste et exige exactement un fichier de vecteurs (`segment.rs:881-903`, `:950-965`), donc l'index ne s'ouvre pas.

### 4. Stabilité, et ce que devient la somme

`live_bytes` ne dépend que de la **génération publiée** et de ses segments. Il est donc :

- **stable entre deux appels**, sans écriture : ni le WAL, ni un `tmp-*`, ni un `cache/` ne le touchent ;
- **identique avant et après un redémarrage** : la reprise ne change ni la génération ni la table `segments`, et `live_bytes` ne lit pas le disque ;
- **croissant à l'attache**, de la taille des fichiers du segment ajouté : le nouveau segment entre dans la génération publiée avec son `segment.json` et son `vectors.*` ;
- **inchangé par un patch** de métadonnées : un patch ne crée aucun segment et republie les mêmes (`docs/02-architecture.md:140`) ;
- **modifié par une compaction, à un seul terme près.** La compaction garde les passages vivants, la dimension et la précision, donc la **somme des vecteurs est invariante** : mêmes composantes, mêmes 2 ou 4 octets par composante. Le terme `segment.json`, lui, suit le **nombre de segments** : une compaction qui ramène plusieurs segments à un découpage de 1 000 000 de passages **retire** le `segment.json` de chaque segment supprimé, soit quelques centaines d'octets par segment. La somme baisse donc légèrement, sans qu'un passage ait disparu. C'est écrit dans `docs/05`.

### 5. Exposition limitée au corps du détail

`live_bytes` n'apparaît **ni** dans `GET /v1/indexes` (corps fermé, ADR-0105 ; route servie pendant le démarrage, ADR-0146), **ni** dans `/v1/stats` (membres fixes, `disk_bytes` seule taille stockée, ADR-0124), **ni** dans `analytics.sqlite` (une colonne serait un changement de format, interdit en 1.x par ADR-0143 et INV-15), **ni** dans la CLI.

### 6. Ce que `live_bytes` ne mesure pas

**`live_bytes` n'est pas le coût d'un instantané.** Un instantané copie aussi `records.sqlite` et `lexical/` (`docs/04-donnees-et-ingestion.md:331`), que `live_bytes` exclut. Une estimation de sauvegarde part donc de `live_bytes` et **y ajoute** la taille de ces deux parties.

C'est une décision de l'orchestrateur, prise sur la délégation du propriétaire. Les deux variantes qui donneraient directement une empreinte de sauvegarde sont écartées pour cette tâche (voir les alternatives) : la taille de `records.sqlite` et de `lexical/` dépend des points de contrôle et des fusions tantivy, et **n'a pas la stabilité demandée**. Le nom `live_bytes` est celui du ticket ; il désigne les données vivantes **vectorielles**, et `docs/05` le dit en une phrase.

### 7. Classe de compatibilité : ajout compatible

Un membre de réponse nouveau, sans effet sur les membres existants (ADR-0143, décision 1). Aucun `format_version` ne change : rien n'est écrit sur disque, ni dans `index.json`, ni dans `analytics.sqlite`, ni dans un segment. Un client 1.0 qui reçoit la réponse ignore `live_bytes` (ADR-0112).

### 8. Ce qui sera appliqué à l'acceptation

| Document ou code | Ce qui change |
|---|---|
| `docs/05-api.md` (définition de `disk_bytes`, `:134`) | ce que la marche inclut (WAL, `-shm`, `tmp-*`, `cache/`, `codes.bin`, segments non référencés), le fait qu'un `deleted-*` vit sous `<data>/tmp` et n'est jamais compté, et l'effet de la reprise : la valeur peut changer entre deux appels et baisser au redémarrage |
| `docs/05-api.md` (définition de `live_bytes`) | la définition de la décision 2, sa stabilité (décision 4), et ce qu'il ne mesure pas (décision 6) |
| `docs/05-api.md` (exemple de corps, `:128`) | `live_bytes` ajouté dans `state.sizes`, à côté de `disk_bytes` |
| `docs/02-architecture.md` | rien, sauf si le propriétaire veut un renvoi à cet ADR à côté de la marche ; la reprise est déjà décrite (`:144-152`) |
| OpenAPI (`IndexSizes`, `crates/crible-server/src/openapi.rs:484-491`) | champ `live_bytes`, documenté, et l'exemple de schéma mis à jour |
| Code (`crates/crible-store/src/segment.rs`, `crates/crible-server/src/indexes.rs:519-552`) | `vectors_bytes` et `manifest_bytes` publiques sur `SegmentMeta`, calculées sans le système de fichiers ; la somme dans `detail`, depuis `snapshot.segments()` ; `directory_size` et `disk_bytes` inchangés |
| Tests attendus | `live_bytes` identique avant et après un redémarrage ; croissance à l'attache égale à la taille des fichiers du segment ajouté (`segment.json` et `vectors.*`) ; **aucun parcours du dossier** : un fichier `tmp-*` ou un `-wal` ajouté dans le dossier ne change pas la valeur ; `0` pour un index vide ; le schéma OpenAPI porte le champ |

## Conséquences

- **Positives** :
  - Legifacile obtient le chiffre stable qu'il demande, sur la route qu'il consulte déjà, sans changer de client ni de version de contrat ;
  - `disk_bytes` garde son sens d'exploitation — l'espace réellement occupé, qu'un opérateur compare à `du` — sans qu'aucun client 1.0 ne voie de différence ;
  - `live_bytes` ne lit pas le disque : la route ne peut plus ni échouer sur un fichier disparu ni varier pendant qu'une attache renomme un `tmp-*` ;
  - les deux nombres se lisent ensemble : `disk_bytes − live_bytes` est l'espace non vectoriel, transitoires compris, et l'écart qui choquait le ticket devient une information ;
  - la définition de `disk_bytes` est enfin écrite là où un client la lit, ce qui ferme la cause du ticket autant que le nouveau membre.
- **Négatives / coûts** :
  - deux tailles dans la même réponse, dont une seule est stable : `docs/05` doit dire clairement laquelle sert à quoi, et une confusion reste possible ;
  - la taille de `segment.json` est calculée, pas mesurée : elle dépend de la fonction de sérialisation canonique, donc un futur changement de forme de `segment.json` doit mettre `manifest_bytes` à jour dans le même changement ;
  - `live_bytes` sous-estime le coût d'une sauvegarde, et l'estimation juste demande d'y ajouter `records.sqlite` et `lexical/` à la main ;
  - un membre de plus dans un corps déjà long, et un champ de plus à tenir dans l'OpenAPI, les exemples et les tests ;
  - la réponse à `GET /v1/indexes/{index}` a maintenant deux origines — le disque pour `disk_bytes`, l'état publié pour `live_bytes` : le commentaire de la route doit le dire, sinon la prochaine lecture les croira de même nature.
- **À surveiller** :
  - `live_bytes` qui change **sans nouvelle génération** : c'est le signal qu'une source de disque s'est glissée dans le calcul, et il doit être traité comme un défaut, pas comme un arrêté ;
  - `live_bytes` supérieur à `disk_bytes` : impossible par construction, sauf dossier de segment disparu sous une génération publiée (le disque a perdu les fichiers, l'état publié les compte encore) ; signal d'alerte ;
  - la taille d'un `segment.json` après une évolution de format : `manifest_bytes` doit suivre, et un test de cohérence avec le fichier écrit le prouve ;
  - un consommateur qui prendrait `live_bytes` pour le coût d'une sauvegarde : la question se repose au premier instantané qui ne tient pas dans l'espace prévu.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| **Piste 3 du ticket : exclure des fichiers de `disk_bytes`** (WAL, `-shm`, `tmp-*`, segments non référencés) | Change le sens d'une valeur documentée — `disk_bytes` est « tous fichiers confondus » (ADR-0117, décision 2), « la taille du dossier de l'index sur disque » (ADR-0105, décision 3, `docs/05-api.md:134`). **Interdit en 1.1** par ADR-0143, décision 1. Un client qui compare la valeur à `du` verrait un écart, et une 1.0 et une 1.1 ne rendraient plus le même nombre. |
| **Un parcours filtré du dossier** (marche qui ignore WAL, `tmp-*`, `cache/`, segments non référencés) | Même sens changé, donc même interdiction, avec un coût de code plus grand. Il garde en outre une lecture du disque, donc les courses qui font varier la valeur entre deux appels : le filtrage réduit l'instabilité, il ne la supprime pas. |
| **Lire les tailles dans `manifests/<G>.json`** (piste 2 du ticket) | Le manifeste ne porte **aucune taille** : il reprend les champs de `segment.json` et `segment_id` (`crates/crible-store/src/generation.rs:2822-2855`). La source proposée n'existe pas. |
| **Exposer `live_bytes` dans `GET /v1/indexes`, `/v1/stats`, `analytics.sqlite` ou la CLI** | `GET /v1/indexes` a un corps fermé (ADR-0105) et sert pendant le démarrage (ADR-0146) ; `/v1/stats` a des membres fixes et `disk_bytes` pour seule taille stockée (ADR-0124) ; une colonne d'`analytics.sqlite` serait un changement de format, interdit en 1.x (ADR-0143, INV-15). La CLI n'a aucune raison de porter un chiffre que l'API rend déjà. |
| **`live_bytes` = l'empreinte vivante complète de l'époque** (`records.sqlite` et `lexical/` compris) | La taille de `records.sqlite` dépend des points de contrôle, celle de `lexical/` des fusions tantivy : le nombre varierait sans attache ni suppression, et **n'aurait pas la stabilité demandée**. Le membre perdrait la propriété qui justifie son existence. |
| **Un second membre, `snapshot_bytes`** (empreinte complète, à côté de `live_bytes`) | Deux tailles proches dans la même réponse, dont une seule est stable, et deux définitions à tenir. L'estimation de sauvegarde s'obtient en ajoutant deux parties documentées ; un membre de plus n'ajoute pas d'information que `docs/05` ne donne pas déjà. La question se rouvre si un client ne peut pas faire cette addition. |

## Références

- Ticket : `../legifacile/tickets-crible/10-disk-bytes-instables.md` (constat, reproduction, pistes 1 à 3, relevé du 2026-11-10 sur `articles`).
- Conception : `docs/05-api.md:118-137` (corps de `GET /v1/indexes/{index}`, exemple `:128`, définitions `:133-134`) ; `docs/02-architecture.md:87` (fichiers dérivés), `:140` (un patch ne crée aucun segment), `:144-152` (reprise au démarrage, suppression des segments absents de la table) ; `docs/04-donnees-et-ingestion.md:331` (contenu d'un instantané).
- Mesure : `docs/mesures/routes-en-conteneur-2026-11-03.md:125` (coût de `sizes.disk_bytes`, 0,0483 ms en p50, 0,104 ms en p95, 31 fichiers), `:156` (0,05 ms, « ce qui domine une route n'est pas ce qu'on croit »).
- ADR : ADR-0016 (le commit SQLite est l'unique point de commit, manifeste dérivé), ADR-0064 et ADR-0065 (format et contenu d'un instantané, restauration), ADR-0069 (`codes.bin` dérivé et reconstruit), ADR-0105 (corps des deux `GET`, décision 3), ADR-0112 (client Python sans type déclaré), ADR-0117 (analytics, `disk_bytes` tous fichiers confondus, décision 2), ADR-0124 (`/v1/stats`, membres fixes, `disk_bytes` seule taille stockée), ADR-0135 (seuil de latence des routes), ADR-0143 (politique de version de la ligne 1.x, décisions 1 et 2), ADR-0146 (routes de l'API pendant le chargement au démarrage).
- Code : `crates/crible-server/src/indexes.rs:519-552` (`detail`, `disk_bytes`), `:578` (`directory_size`, marche du dossier), `crates/crible-server/src/analytics.rs:873` (la même marche pour `/v1/stats`), `crates/crible-server/src/openapi.rs:484-491` (`IndexSizes`), `crates/crible-store/src/generation.rs:421` (`Snapshot::segments`), `:2822-2855` (manifeste de génération, sans taille), `crates/crible-store/src/segment.rs:86-91` (`bytes_per_component`), `:115-134` (`SegmentMeta`), `:138-162` (`vectors_len`), `:881-903` (`read_meta`), `:1097-1113` (`manifest_json`, JSON canonique), `crates/crible-store/src/compact.rs:97` (`MAX_SEGMENT_PASSAGES`), `:431-492` (réécriture des segments par tranches), `:451` (précision conservée).
- Suivi : `docs/suivi.md:171` (ligne `T10-DISK-BYTES-STABLES`, classe « ajout compatible »).
