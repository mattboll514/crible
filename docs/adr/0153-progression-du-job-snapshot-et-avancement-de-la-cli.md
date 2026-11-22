# ADR-0153 — Progression du job `snapshot` et avancement de la CLI

- **Statut** : Accepté (accepté par anticipation par le propriétaire le 2026-11-12, confirmé par le propriétaire le 2026-11-12)
- **Date** : 2026-11-12
- **Décideurs** : propriétaire
- **Proposé par** : agent `@greffier`, tâche `T11-AVANCEMENT-SNAPSHOT` (v1.1), sur demande de l'orchestrateur et sur le verdict de `@architecte` du 2026-11-12
- **Remplace** : ADR-0134, **décisions 1, 2 et 4 seulement, et en partie** — pour le seul `kind` `snapshot`. Le reste d'ADR-0134 reste vrai pour les autres types, et ses décisions 3 et 5 ne changent pas.
- **Décision** : confirmée par le propriétaire le 2026-11-12, sa réponse : « les 2 adr sont accepté avec l'eratum », erratum du 2026-11-12 compris.

> **Erratum du 2026-11-12, sur l'implémentation de la tâche `T11-AVANCEMENT-SNAPSHOT`.** Il précise la décision 6 et **amende sur place sa puce** « une ligne toutes les 10 secondes », pour qu'elle ne contredise pas cet erratum ; l'ADR a été accepté par anticipation et sera relu par le propriétaire, qui peut refuser cette réécriture. Aucun autre point ne change.
>
> La ligne d'avancement de `crible snapshot` n'est pas écrite « toutes les 10 secondes » au sens strict. Elle est écrite à la **première publication** de la copie, à **chaque changement d'étape** (`copy`, `attest`, `rename`), et **sinon au plus une fois par période de 10 s**. Un ticker reprend le dernier état connu, de sorte qu'une étape qui ne publie rien — le `VACUUM INTO` — reste visible au lieu de laisser un silence. Une copie courte écrit donc au moins ses trois étapes. La forme de chaque ligne est `crible snapshot: <step>, <done> of <total> bytes copied, <n> s elapsed` (`SNAPSHOT_PROGRESS_PERIOD`, `ProgressLine` et `spawn_progress_ticker`, dans `crates/crible/src/commands.rs`).
>
> Le reste de la décision 6 est inchangé : la période reste **fixe**, à 10 s, sans option ni clé de configuration ; `stdout` ne change pas et garde la seule ligne de rapport ; la classe de compatibilité d'ADR-0143 ne change pas, elle reste **ajout compatible** (décision 7).

## Contexte

### Le ticket, et ce qu'il constate

Le ticket 11 de Legifacile (`/Users/mattboll/MattCorp/legifacile/tickets-crible/11-snapshot-sans-avancement.md`) porte sur la version 1.0.0. Un instantané d'un index volumineux copie plusieurs gigaoctets : sur le Crible de développement, `articles` rend `disk_bytes = 7 420 957 206` octets au 2026-11-10. Pendant cette copie, la route `POST /v1/indexes/{index}/snapshots` rend `202` avec un job `snapshot`, mais ce job garde `progress: null` jusqu'à la fin. Côté CLI, `crible snapshot` n'écrit qu'une ligne de rapport, après le renommage final.

Le ticket est un document d'un autre projet, lu en lecture seule ; il n'est pas modifié par cet ADR. Il demande trois choses : publier `progress` pour un `snapshot` avec des étapes nommées et un compte d'octets copiés sur le total estimé, écrire une ligne d'avancement sur `stderr` toutes les 10 à 15 s côté CLI, et ne pas promettre un total exact. Il note aussi que la copie peut être plus petite que le dossier, puisque le WAL, les `tmp-*` et les segments non référencés sont exclus.

Le ticket nomme l'étape finale `fsync`. Le présent ADR la nomme `rename`, parce que la synchronisation de l'arbre, le renommage et le fsync du dossier parent forment un seul acte : nommer `fsync` laisserait croire que la copie est utilisable avant le renommage, ce qui est faux (`docs/04-donnees-et-ingestion.md:359`).

### Ce que le code fait aujourd'hui, vérifié le 2026-11-12

- **Le job reçoit la progression et ne la publie pas.** `snapshots_inner` passe `_progress` au travail du job et ne l'utilise pas (`crates/crible-server/src/maintenance.rs:245-247`) : le travail se réduit à un appel de `snapshot::create` (`:247`). Le type du travail reçoit pourtant le manche de progression (`crates/crible-server/src/jobs.rs:304-309`), et les deux routes d'attache s'en servent déjà (`crates/crible-server/src/batches.rs:184`).
- **La copie est une suite d'actes nommables.** `snapshot::create` (`crates/crible-store/src/snapshot.rs:205-340`) prend un `VACUUM INTO` de `records.sqlite` (`:252-255`), copie `index.json` (`:262-270`), `manifests/<G>.json` (`:272-280`), les `segment.json` et fichiers de vecteurs des segments publiés (`:282-294`) et `lexical/` s'il existe (`:296-304`), hache chaque fichier copié par `attest` (`:257-260`, `:950-966`), somme les octets (`:306`), écrit `snapshot.json` (`:307-320`), synchronise l'arbre (`:324`), renomme le dossier de transit vers sa destination (`:326-327`) et fsync le dossier parent (`:329`). Le rapport porte `bytes`, la somme des octets copiés, `snapshot.json` excepté (`:137-138`, `:338`).
- **`JobProgress::complete` force `done = total`.** À la transition terminale d'un job réussi, `complete` remplace `done` par `total` (`crates/crible-server/src/jobs.rs:207-216`). Un job `failed` ne passe pas par là : il garde la dernière valeur publiée.
- **La CLI n'a qu'une sortie de contrat.** `crible snapshot` imprime une ligne de rapport sur `stdout` (`crates/crible/src/commands.rs:424-442`), et `docs/04-donnees-et-ingestion.md:361` fixe ce que le rapport porte. Le journal d'une commande est du JSON sur `stderr`, filtré à `warn` quand `RUST_LOG` est absent (`crates/crible/src/main.rs:554-576`, constante `DEFAULT_COMMAND_FILTER` à `:567`) : une ligne `info!` n'y serait jamais visible, et ce serait de toute façon du JSON, pas la ligne lisible que demande un opérateur.

### Ce que la conception fixe déjà

- **ADR-0134, décision 1** (`docs/adr/0134-progression-des-jobs.md:32-37`) : `progress.step` prend les noms du protocole d'attache, `validation`, `segment`, `transaction`, `lexical`, `manifest` et `snapshot`. Le nom `snapshot` y désigne l'étape 6 de l'attache, celle qui construit et publie le nouvel instantané de lecture (`docs/02-architecture.md:131`).
- **ADR-0134, décision 2** (`:39-41`) : `done` avance aux frontières d'étape, `total` est connu dès la lecture du manifeste ou du corps du patch, et un job échoué garde la dernière progression publiée.
- **ADR-0134, décision 4** (`:45`) : seuls `attach_batch`, `online_batch` et `patch_documents` publient `progress` ; les autres `kind` gardent `null`, et en inventer une unité par type demanderait un ADR. C'est cette décision, avec les deux précédentes, que le présent ADR remplace pour le seul `kind` `snapshot`.
- **La progression vit en mémoire.** La table `jobs.sqlite` n'est écrite qu'aux transitions d'état, et chaque écriture porte la progression de l'instant (ADR-0103, décision 3). `jobs.sqlite` garde `format_version = 1` (ADR-0103, décision 1).
- **Un instantané est hors ligne en CLI.** `crible snapshot` demande l'arrêt du serveur : un processus ne peut pas suspendre l'écrivain d'un autre (ADR-0065, décision 1, `docs/adr/0065-instantane-hors-ligne-epoque-de-restauration-et-retention.md:61-63`). La forme des commandes, le rapport et les codes d'erreur sont fixés par sa décision 6 (`:109`).
- **Au démarrage, tout job `queued` ou `running` passe à `failed`** avec le message fixe « the job was interrupted by a server restart » (ADR-0103, décision 5, `docs/adr/0103-jobs-de-l-instance-table-etats-retention-file-et-retry-after.md:41`).
- **Le format de l'instantané ne bouge pas.** `snapshot.json` est versionné (`format_version: 1`, INV-15) et liste chaque fichier copié avec sa taille et son blake3, la liste étant triée par chemin (ADR-0064, `docs/adr/0064-format-et-contenu-d-un-instantane.md:55-58` ; `docs/04-donnees-et-ingestion.md:356`).
- **`live_bytes` existe.** Depuis ADR-0152, la taille des fichiers d'une génération publiée — `segment.json` et vecteurs, sans parcours ni `stat` — se calcule depuis l'état publié (ADR-0152, décision 2, `docs/adr/0152-live-bytes-et-documentation-de-disk-bytes.md:60-64` ; `crates/crible-server/src/indexes.rs:570`).

### Le point de bascule

Un instantané de plusieurs gigaoctets dure des minutes, et le seul signal du job est son état. Un opérateur ne peut donc pas distinguer une copie lente d'un écrivain bloqué, ni estimer la fenêtre pendant laquelle la sauvegarde l'occupe. `?wait=true` ne rend rien pendant ce temps : il attend au plus `wait_timeout_ms`, 60 000 ms par défaut, puis rend `202` avec le job en cours (ADR-0067). Le seul endroit où l'avancement peut être lu est donc l'objet `Job`, comme ADR-0149 l'a fait pour les durées d'attache.

Côté CLI, `stdout` est un contrat : un script le lit, et `docs/04:361` fixe le rapport. Le journal, lui, est du JSON filtré à `warn`. Aucune des deux sorties existantes ne peut porter une ligne d'avancement.

## Décision

**Le job `snapshot` publie `progress` en octets, avec trois étapes, et `crible snapshot` écrit une ligne d'avancement sur `stderr` toutes les 10 s sans changer `stdout`.** Les décisions 1, 2 et 4 d'ADR-0134 sont remplacées pour le seul `kind` `snapshot` ; les autres types, et le reste d'ADR-0134, ne changent pas.

### 1. Les étapes du job `snapshot`

`progress.step` prend trois noms, dans cet ordre :

1. **`copy`** — la copie de `records.sqlite` par `VACUUM INTO`, puis de `index.json`, du manifeste `manifests/<G>.json`, des `segment.json` et fichiers de vecteurs des segments publiés et de `lexical/` s'il existe, avec le hachage blake3 de chaque fichier copié (`crates/crible-store/src/snapshot.rs:252-304`, `attest` à `:950-966`).
2. **`attest`** — l'écriture de `snapshot.json`, qui atteste la copie (`snapshot.rs:307-320`).
3. **`rename`** — la synchronisation de l'arbre, le renommage final du dossier de transit et le fsync du dossier parent (`snapshot.rs:322-329`).

Le nom `snapshot` n'est pas réutilisé : c'est déjà l'étape 6 de l'attache (`docs/02-architecture.md:131`, ADR-0134, décision 1), et un `step` valant `snapshot` dans un job `snapshot` se lirait de travers, sans dire ce qui est copié.

### 2. Unité : octets

`progress.done` et `progress.total` sont des **octets entiers**. Ils ne se comparent pas d'un `kind` à l'autre — un `attach_batch` compte des passages, un `patch_documents` des documents —, seulement dans le temps d'un même job. C'est la réponse au troisième point du ticket : l'unité est nommée, et un client n'a pas à la deviner.

### 3. Total estimé pendant `copy`, puis exact

Pendant `copy`, `total` est une **estimation**, calculée sans parcourir les segments depuis l'état publié :

- `live_bytes` (ADR-0152, décisions 3 et 4), qui couvre les `segment.json` et les fichiers de vecteurs des segments publiés ;
- la taille de `records.sqlite`, lue par un `stat` ;
- la taille du dossier `lexical/` s'il existe, par une marche bornée à ce dossier.

Le seul terme inexact est la sortie du `VACUUM INTO` : elle rend une base compactée, souvent plus petite que `records.sqlite`, et elle n'est connue qu'une fois l'appel terminé. L'estimation ne compte pas `index.json` ni `manifests/<G>.json`, dont la taille est négligeable devant des gigaoctets.

À partir de `attest`, `total` vaut la **somme exacte des octets copiés**, celle du rapport (`bytes`, `snapshot.rs:306` et `:338`).

**Règle** : pendant `copy`, `total` ne baisse jamais. Si les octets copiés dépassent l'estimation, `total` est relevé à `done`, de sorte que `done ≤ total` à chaque lecture. À `attest`, `total` est remplacé une seule fois par la somme exacte des octets copiés, qui peut être plus petite ou plus grande que l'estimation ; ensuite il ne bouge plus. Un job réussi finit avec `done = total = bytes` du rapport ; c'est ce que `JobProgress::complete` impose déjà, puisque `total` vaut `bytes` avant la transition terminale (`crates/crible-server/src/jobs.rs:207-216`).

### 4. Granularité

La progression est publiée :

- aux frontières d'étape, comme ADR-0134, décision 2 ;
- **et après chaque fichier copié** pendant `copy` : un `JobProgress::set` en mémoire par fichier, dont le nombre est de l'ordre de la dizaine sur un index courant, aucune écriture SQLite. ADR-0103, décision 3, reste vrai sans exception.

Aucune publication n'est plus fine que le fichier. **Limite nommée** : il n'y a aucun avancement pendant le `VACUUM INTO`, qui est un appel SQLite unique (`snapshot.rs:252-255`). Sur un gros index, `records.sqlite` peut représenter une part importante de la copie, et cette part reste muette.

### 5. Échec

ADR-0134, décision 2, est reconduite telle quelle : un job `failed` garde la dernière progression publiée, et `null` si l'échec précède la première publication. La progression d'un instantané interrompu dit donc jusqu'où la copie est allée.

Le résidu `.tmp-*` d'un échec reste sur disque, et ni la reprise ni une commande ne le supprime (`docs/04-donnees-et-ingestion.md:359`). Son nettoyage est **hors du présent ADR** : il demande sa propre décision, parce qu'il touche à l'exploitation et à la reprise.

### 6. La CLI hors ligne écrit sur `stderr` toutes les 10 s

`crible snapshot` ne crée aucun job : le serveur est arrêté (ADR-0065, décision 1), et un job écrit hors ligne serait basculé `failed` au démarrage suivant, sans que personne ne puisse le lire (ADR-0103, décision 5). La commande écrit donc son avancement elle-même :

- sur **`stderr`**, directement et **hors du journal `tracing`**. Une ligne du journal est du JSON filtré à `warn` (`crates/crible/src/main.rs:554-576`) : une ligne `info!` serait invisible, et un objet JSON n'est pas la ligne lisible qu'attend un opérateur devant un terminal ;
- une ligne **au plus toutes les 10 secondes** : à la première publication de la copie, à chaque changement d'étape, et sinon au plus une par période de 10 s (voir l'erratum du 2026-11-12), valeur fixe, sans option ni clé de configuration ;
- chaque ligne porte l'étape, les octets copiés, le total estimé et le temps écoulé ;
- format **texte lisible, une ligne par tick**, en **anglais** : c'est une sortie du programme, pas un document ;
- **`stdout` ne change pas** : une seule ligne de rapport, à la fin, celle de `docs/04:361`.

Cette décision est séparable de la décision 1 : un client qui ne lit que l'API n'en dépend pas, et l'inverse est vrai.

### 7. Classe de compatibilité (ADR-0143) : ajout compatible

**Ajout compatible**, au sens de la décision 1 d'ADR-0143 (`docs/adr/0143-politique-de-version-de-la-ligne-1-x-et-procedure-de-montee-et-de-retour-arriere.md:43-67`), qui range « un membre de réponse nouveau » parmi les ajouts permis dans une mineure.

- Le membre `progress` est **toujours présent** dans l'objet `Job` (ADR-0134, contexte ; ADR-0111, décision 7). Sa forme `{step, done, total}` ne change pas : trois membres, mêmes types.
- Aucun client n'est obligé de le lire : un client 1.0 rend le JSON en `dict` sans type déclaré (ADR-0112) et ignore le membre. Les autres `kind` gardent `progress: null`, sauf les trois d'ADR-0134, décision 4.
- La ligne `stderr` de la CLI est une **sortie ajoutée** : `stdout` reste la seule ligne de rapport, et un script qui la parse ne voit aucune différence.
- **Aucun format persistant ne change.** `snapshot.json` garde son `format_version` et sa liste de fichiers (ADR-0064 ; `crates/crible-store/src/snapshot.rs:102`, `:307-320`), et `jobs.sqlite` garde le sien (ADR-0103, décision 1). `progress` ne s'écrit que dans la colonne existante, aux transitions d'état, comme aujourd'hui.

### Ce que cet ADR ne décide pas

- Le mécanisme : forme du rappel passé à `snapshot::create`, compteur partagé, ou fermeture pour la CLI. C'est un choix d'implémentation, réversible, qui appartient à la tâche.
- Le coût de la publication, et le nombre exact de fichiers d'un index donné.
- Le libellé exact de la ligne `stderr` au-delà de ses quatre champs et de sa langue.
- Le nettoyage des résidus `.tmp-*` (décision 5).
- Une progression pour les autres `kind` : elle demanderait son unité et son ADR, comme ADR-0134, décision 4, le disait.
- La mention de ce changement dans la note de version `docs/versions/1.1.0.md`, qui appartient à `PUBLIER-V1.1.0` : elle y figurera comme avancement ajouté pour un `kind` qui n'en avait pas.

## Conséquences

- **Positives** :
  - une sauvegarde devient observable : l'opérateur distingue une copie lente d'un blocage, et il estime la fenêtre pendant laquelle l'écrivain est suspendu ;
  - le job et la CLI publient la même information, dans la même unité, sans second calcul de taille : l'estimation part de `live_bytes` et du rapport, tous deux déjà calculés par le code ;
  - `done ≤ total` est une règle vérifiable, qui rend le membre lisible sans que le client ait à interpréter une estimation ;
  - aucun format ne change, donc aucune migration ni restauration spéciale : une 1.0 relit un instantané pris en 1.1 ;
  - `stdout` reste propre, donc les scripts de sauvegarde de Legifacile continuent de parser une seule ligne.
- **Négatives / coûts** :
  - un trou d'avancement pendant le `VACUUM INTO`, qui peut être long et qui est nommé sans être mesuré ;
  - le total est approximatif pendant `copy` : un client qui affiche un pourcentage verra un saut à `attest`, quand `total` est remplacé par la somme exacte, et ce saut peut aller dans les deux sens ; il ne doit donc pas traiter ce pourcentage comme une mesure ;
  - l'unité est différente de celle des autres `kind` : un tableau de bord qui affiche `done / total` sans lire `kind` ni `step` comparerait des octets et des passages ;
  - une publication par fichier copié ajoute un verrou en mémoire par fichier, coût négligeable mais réel, et une ligne `stderr` toutes les 10 s ajoute une sortie que les journaux d'exploitation ne contenaient pas ;
  - la ligne `stderr` ne passe pas par le journal : elle échappe donc au format JSON et à `RUST_LOG`, ce qui est voulu mais doit être dit dans `docs/04`.

- **À surveiller** :
  - un `snapshot` à chaud qui publie `copy`, puis `attest`, puis `rename`, avec `done` croissant, `done ≤ total` à chaque lecture et `done = total = bytes` à la fin ;
  - un échec simulé qui garde la dernière progression publiée, et `progress: null` si l'échec précède la première publication ;
  - les autres `kind`, sauf les trois d'ADR-0134, qui gardent `progress: null` ;
  - une copie longue dont la CLI écrit au moins une ligne sur `stderr`, avec `stdout` réduit à la seule ligne de rapport ;
  - INV-15 : `format_version` de `snapshot.json` et contenu de l'attestation inchangés ;
  - une demande de pourcentage exact, ou d'un avancement pendant le `VACUUM INTO` : elle rouvrirait la décision 3 ou la décision 4, et demanderait un ADR, pas un ajustement de code.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| **Nommer `snapshot` l'étape de copie du job `snapshot`** | Le nom est déjà l'étape 6 de l'attache (`docs/02-architecture.md:131`, ADR-0134 décision 1) : un client lirait `snapshot` dans un job `snapshot` sans savoir si c'est la copie ou la publication de l'instantané de lecture. |
| **Nommer `fsync` l'étape finale**, comme le ticket | La synchronisation de l'arbre, le renommage et le fsync du dossier parent sont un seul acte : `fsync` laisserait croire que la copie est utilisable avant le renommage, alors que seule la destination finale l'est (`docs/04:359`). |
| **Compter des fichiers, ou un pourcentage** | Les fichiers n'ont pas la même taille — `records.sqlite` peut dominer la copie — et un pourcentage suppose un total connu d'avance, ce qu'aucune source ne donne sans parcours. L'octet est la seule unité que la copie produit réellement. |
| **Compter des passages ou des documents** | Un instantané ne traite ni passage ni document : il copie des fichiers. Ce serait l'unité d'un autre `kind`, et `done` ne mesurerait pas le travail fait. |
| **Publier à chaque bloc lu ou écrit** | Des milliers de publications pour une information qu'un client lit toutes les secondes au mieux, et un verrou en mémoire pris par bloc. Le fichier est la granularité la plus fine qui reste utile. |
| **Écrire la progression dans `jobs.sqlite` à chaque fichier** | Déjà écarté par ADR-0103, décision 3 : la table n'est écrite qu'aux transitions d'état. Le gain serait nul, puisqu'un job interrompu est `failed` de toute façon. |
| **Faire de `crible snapshot` un job** | La commande est hors ligne (ADR-0065, décision 1) et aucun serveur ne lit ses jobs ; un job écrit hors ligne passerait `failed` au démarrage suivant (ADR-0103, décision 5). Le canal est `stderr`, pas `jobs.sqlite`. |
| **Écrire une ligne `info!` dans le journal de la CLI** | Le journal d'une commande est du JSON filtré à `warn` (`crates/crible/src/main.rs:554-576`) : la ligne serait invisible par défaut, dépendrait de `RUST_LOG`, et ne serait pas la ligne lisible demandée par le ticket. |
| **Une option `--progress`, ou une clé de configuration pour la période** | Le ticket demande un comportement par défaut, et la valeur de 10 s ne demande aucun réglage. Une option obligerait chaque script à la connaître pour obtenir ce qu'il attend ; une clé serait une configuration pour une valeur fixe (`AGENTS.md`, règle 12). |
| **Un total exact dès le départ, par un parcours du dossier** | Un parcours compterait le WAL, les `tmp-*` et les segments non référencés — exactement ce que la copie exclut — et ramènerait le disque comme seconde source, ce qu'ADR-0152, décision 3, a écarté pour `live_bytes`. Le total resterait faux, avec un coût de plus. |
| **Laisser le client surveiller `du` sur la destination** | C'est le contournement que le ticket décrit (`tools/bench/snapshot_progress.py`) : il lit le disque du serveur, ne marche pas à distance, et ne dit rien à un client HTTP. |

## Application

Après acceptation seulement, sur la branche de la tâche `T11-AVANCEMENT-SNAPSHOT`, par une étape suivante, dans un commit qui cite `ADR-0153` :

| Document ou code | Modification |
|---|---|
| `docs/05-api.md:518` (puce `progress` de la section « Jobs ») | ajouter `snapshot` aux `kind` qui publient, avec les étapes `copy`, `attest` et `rename`, l'unité en octets, le total estimé puis exact et la règle `done ≤ total` |
| `docs/05-api.md:76` (route `POST /v1/indexes/{index}/snapshots`) | mentionner que le job publie un avancement en octets pendant la copie |
| `docs/04-donnees-et-ingestion.md:349-361` (« Règles de l'instantané ») | un point sur l'avancement : les trois étapes, l'unité en octets, le total estimé pendant `copy` puis exact, et la ligne `stderr` de la CLI toutes les 10 s, hors du journal |
| `crates/crible-server/src/openapi.rs:591-600` | commentaire de `JobProgress.step` : ajouter les trois noms du job `snapshot` et l'unité en octets, en citant ADR-0153 |
| `crates/crible-server/src/maintenance.rs:245-247` | publier la progression par le manche reçu, au lieu de `_progress` |
| `crates/crible-store/src/snapshot.rs:205-340` | rappel de progression par fichier copié et aux trois frontières d'étape, sans changer le format écrit |
| `crates/crible/src/commands.rs:424-442` | ligne d'avancement sur `stderr` toutes les 10 s ; `stdout` inchangé |
| `docs/versions/1.1.0.md` | mention de l'ajout, par `PUBLIER-V1.1.0` |
| `docs/adr/0134-…md`, `docs/adr/README.md` | note de remplacement partiel et ligne d'index — faites par cet ADR |

**Tests attendus** :

1. un `snapshot` à chaud publie `copy`, puis `attest`, puis `rename`, avec `done` croissant, `done ≤ total` à chaque lecture et `done = total = bytes` à la fin ;
2. un échec simulé garde la dernière progression publiée, et rend `progress: null` quand il précède la première publication ;
3. les autres `kind`, sauf les trois d'ADR-0134, gardent `progress: null` ;
4. `crible snapshot` écrit au moins une ligne sur `stderr` sur une copie longue, et `stdout` reste la seule ligne de rapport ;
5. INV-15 : `format_version` de `snapshot.json` et contenu de l'attestation inchangés.

## Références

- Ticket : `/Users/mattboll/MattCorp/legifacile/tickets-crible/11-snapshot-sans-avancement.md` (« Constat », « Observé / attendu », « Impact sur Legifacile », « Pistes de correction »), lu en lecture seule.
- Conception : `docs/05-api.md:76` (route des instantanés), `:518` (règle de `progress`) ; `docs/04-donnees-et-ingestion.md:349-361` (« Règles de l'instantané », `:353` contenu copié, `:356` `snapshot.json`, `:359` transit et résidu, `:361` rapport) ; `docs/02-architecture.md:131` (étape 6 de l'attache, `snapshot`).
- Code : `crates/crible-server/src/maintenance.rs:230-264` (`snapshots_inner`, `_progress` à `:245-247`) ; `crates/crible-server/src/jobs.rs:194-216` (`JobProgress::set`, `complete`), `:304-309` (`JobWork`) ; `crates/crible-server/src/batches.rs:184` (publication aux frontières d'étape) ; `crates/crible-server/src/indexes.rs:570` (`live_bytes`) ; `crates/crible-store/src/snapshot.rs:102` (`SNAPSHOT_FORMAT_VERSION`), `:205-340` (`create`), `:950-966` (`attest`) ; `crates/crible/src/main.rs:554-576` (journal d'une commande, `DEFAULT_COMMAND_FILTER` à `:567`) ; `crates/crible/src/commands.rs:424-442` (`snapshot_index`) ; `crates/crible-server/src/openapi.rs:591-600` (composant `JobProgress`).
- ADR : ADR-0064 et ADR-0065 (format de l'instantané, instantané hors ligne et rapport), ADR-0067 (`?wait=true` et `wait_timeout_ms`), ADR-0103 (jobs, décisions 1, 3 et 5), ADR-0111 (objets fermés), ADR-0112 (client Python en `dict`), ADR-0134 (progression des jobs, décisions 1, 2 et 4 remplacées en partie), ADR-0143 (ligne 1.x, ajouts compatibles), ADR-0149 (précédent d'un membre de l'objet `Job` en mémoire), ADR-0152 (`live_bytes`, calculé sans parcours).
