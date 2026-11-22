# ADR-0160 — Rendre la mémoire libre de l'allocateur par `malloc_trim(0)` à la fin de chaque job

- **Statut** : Accepté (décidé par le propriétaire le 2026-11-22, sa réponse : « ADR-0160 accepté », tel que proposé)
- **Date** : 2026-11-22 (proposition et acceptation le même jour)
- **Décideurs** : propriétaire
- **Proposé par** : agent `@greffier`, sur la demande du propriétaire du 2026-11-22 (« Oui, lance l'ADR »), tâche `T05-CORRIGER-MALLOC-TRIM` (v1.1), et sur le verdict de `@architecte` du 2026-11-22 (session `ses_dfcf75276ffeO3gTxS79CN5x2E`)
- **Remplace** : —

## Contexte

### Le ticket, et la troisième campagne qui départage

Le ticket 05 de Legifacile (`tickets-crible/05-memoire-non-rendue.md`, projet Legifacile, lecture
seule) constate que la mémoire du conteneur ne redescend pas après une campagne d'attache, et qu'un
OOM pendant une attache `decisions` reste inexpliqué. ADR-0151 a répondu par l'instrumentation seule :
une ligne `memory`, la mesure MES-17, et la règle « aucune correction avant une mesure ». Deux
campagnes n'avaient pas départagé fragmentation et structure vivante.

La **troisième campagne de MES-17** (`docs/mesures/MES-17-classes-de-memoire.md`, section « Troisième
campagne, 2026-11-22 : essai de `malloc_trim(0)` », ADR-0159) appelle `malloc_trim(0)` **par `gdb -p`**,
depuis un conteneur annexe, sur le processus du serveur au repos. Résultats, médianes de l'anonyme du
cgroup :

| Processus | (c)−(b) retenu par l'attache | (c)−(c′) après un trim | Baisse ≥ seuil | Part rendue |
|---|---:|---:|---|---:|
| `trim-p1` | +1 214,86 MiB | −647,57 MiB | oui | 53,3 % |
| `trim-p2` | +941,89 MiB | −378,63 MiB | oui | 40,2 % |

Le seuil de cette campagne est la dispersion **entre ses deux processus** sur (c)−(b), soit
**272,96 MiB** (`:745-748`) ; le sens de lecture est la baisse (ADR-0158). Le premier appel rend `1`
dans les deux processus. Ce qui reste au-dessus de l'état (b) après les appels, en (c″), vaut
**542,9 et 555,9 MiB** (`:763-768`), et **aucune ligne de lecture d'ADR-0159, décision 5, ne le
classe** (`:770-777`). La campagne ne conclut donc ni « fuite » ni « pas de fuite », et ne dit rien de
la latence, du coût d'un appel en service, de l'échelle réelle, ni de l'OOM.

À l'échelle réelle, la mémoire anonyme d'un `replace_all` complet de `decisions` ne retombe qu'au
redémarrage : **10,55 Gio après l'attache, 3,57 Gio au redémarrage** (ADR-0142 `:79`).

### Ce que les décisions déjà prises imposent

- **La correction entre dans la 1.1.0.** Décision du propriétaire du 2026-11-22 : « Oui, lance
  l'ADR ». Le reste, environ 550 MiB non classés, et l'échelle de Legifacile partent dans la liste de
  la v2.
- **Une correction demande un ADR distinct** (ADR-0151 `:196-197` : « elle ne corrige rien […] Toute
  correction demande la mesure de MES-17 puis un ADR distinct » ; ADR-0159, décision 7, `:215-216` ;
  ADR-0157, décision 7). La mesure est faite, le présent ADR est cet ADR-ci.
- **`libc` n'est pas dans `docs/07`**, et son introduction demande un ADR de dépendance (ADR-0080,
  décision 3, `:80`). `docs/07` ne nomme que `memmap2` et `half` côté vecteurs (`:37`).
- **Rien de nouveau ne s'expose par l'API** : un chiffre de mémoire dans `GET /v1/stats` serait un
  écart à ADR-0124, décision 1, et la question y est reportée (ADR-0151, décision 4).
- **Les contrats de journal déjà fermés ne bougent pas** : les quatre valeurs de `step` de la ligne
  `memory` (ADR-0151, décision 2, `:107` ; `docs/06:211-213`) et les six clés de `durations`
  (ADR-0149, `:57`).

### Le verdict de `@architecte` du 2026-11-22

La proposition est acceptée avec trois corrections, et deux points `NON-COUVERT` — concurrence,
observabilité — sont tranchés par les décisions 3 et 4 ci-dessous. Le point restant, l'échelle réelle,
est tranché par la décision 7.

## Décision

**Le serveur appelle `malloc_trim(0)` à la fin de chaque job, après la publication de l'état
terminal, au plus un appel à la fois par instance, et journalise une ligne `trim` de niveau `debug`.
Aucune dépendance, aucun contrat d'API et aucun format ne changent.**

### 1. L'appel, sans crate `libc`

`malloc_trim` est déclaré **localement**, dans un bloc `unsafe extern "C"` — la forme de l'édition
2024 qu'emploie déjà `crates/crible/examples/rescore_ab.rs:914-925` pour `mincore` et `getpagesize` —,
avec la signature `fn malloc_trim(pad: usize) -> core::ffi::c_int`.

- **Portée** : tout le module est sous `cfg(all(target_os = "linux", target_env = "gnu"))`. C'est la
  seule cible où glibc fournit le symbole : l'image de production est `debian:${DEBIAN_VERSION}-slim`
  (`docker/Dockerfile:73`), donc glibc.
- **Hors glibc, une enveloppe sans effet** : macOS de développement (`target_os` différent) et musl
  (`target_env` différent) compilent une fonction qui rend `false` et n'appelle rien. Le code appelant
  ne connaît qu'une fonction `trim_allocator(pad: usize) -> bool`, définie deux fois.
- **`unsafe` et `// SAFETY:`** : la déclaration et l'appel portent un commentaire `// SAFETY:`
  (règle d'or 10 d'`AGENTS.md`) qui dit l'hypothèse — `malloc_trim` est sans état, prend un `usize`
  par valeur, rend un `c_int`, et ne peut échouer que par un retour `0`.
- **Pas de crate `libc`** : `docs/07` reste inchangé et ADR-0080, décision 3, est respecté, pour un
  seul symbole et une seule cible.

### 2. Un point d'appel unique, à la fin de `mark_finished`

L'appel est placé **à la fin de `JobBoard::mark_finished`** (`crates/crible-server/src/jobs.rs:459-514`),
**après** la publication de l'état terminal dans les deux branches : la branche de succès publie par
`publish_with` (`:492`, définition `:546`), la branche d'échec par `publish` (`:509`, définition
`:537`, qui appelle `publish_with`), et `store_and_notify` (`:564`) stocke la vue et réveille les
attentes. L'appel se fait donc après le `match` (`:513`).

- **Tous les genres de job sont couverts** : les neuf `kind` d'`openapi.rs:567-586` passent par
  `mark_finished`, et tous ses appels y mènent — `:1229`, `:1240`, `:1272`, `:1284`, et `fail_queued`
  `:1442`.
- **Pourquoi tous les genres, et non un sous-ensemble** : un tas au repos rend `0` vite, et un
  sous-ensemble serait une règle sans preuve. Il n'y a pas de genre « qui ne trime pas ».
- **Un `?wait=true` n'attend pas le trim** : la publication réveille l'attente (`:564-570`) avant
  l'appel. La latence vue par le client ne dépend donc pas du trim.
- **Aucun verrou d'index n'est tenu** : le magasin appartient à la tâche d'écriture et il est rendu à
  `holder` (`:1265`) avant l'appel de `:1272`. L'appel ne se fait pas sous un verrou du magasin.

### 3. Concurrence : au plus un trim à la fois pour l'instance

Un `AtomicBool` dans `JobBoard` (à côté de `pending`, `:323`) sert de garde : `compare_exchange(false,
true)` avant l'appel, remise à `false` après. **Un trim déjà en cours fait sauter le suivant.**

- On ne saute **pas** parce qu'un job tourne sur un autre index : le comportement dépendrait alors
  d'une charge étrangère à l'index qui vient de finir, et la même fin de job donnerait deux résultats
  selon le voisinage. La garde porte sur l'allocateur, qui est celui du processus, donc de l'instance.
- L'effet d'un trim sur une recherche en vol n'est pas supposé : il est mesuré (décision 6).

### 4. Observabilité : une ligne `trim`, et les lignes existantes inchangées

Une ligne `tracing` de **niveau `debug`**, dont le message est `trim`, porte les champs suivants :

| Champ | Contenu |
|---|---|
| `pad` | l'argument passé à `malloc_trim`, soit `0` |
| `trimmed` | `1` si l'appel a rendu non nul (des pages ont été rendues), `0` sinon |
| `duration_us` | durée de l'appel, en microsecondes |
| `job_id`, `kind`, `index` | le job dont la fin a déclenché l'appel, sous les noms et formes déjà employés (ADR-0104, ADR-0149, ADR-0151) |

- **Nombres et noms seulement** : aucun texte de requête, aucune valeur de filtre, aucun iid (INV-13,
  ADR-0096).
- **La ligne `memory` d'ADR-0151 ne change pas** : ni ses champs, ni ses quatre valeurs de `step`
  (ADR-0151, décision 2, `:107` ; la liste fermée de `docs/06:211-213`). `trim` est un **message
  distinct**, pas une cinquième valeur de `step`.
- **Le membre `durations` d'ADR-0149 ne change pas** : toujours exactement six clés (`:57`).
- **La ligne `memory` de `job_end` reste où elle est** (`jobs.rs:467`), **avant** le trim, pour que
  son chiffre garde le même sens entre 1.0 et 1.1 : elle mesure la mémoire à la fin du travail du job,
  pas après un trim.
- **Aucun contrat d'API, aucun `format_version`, `docs/05` inchangé.** Le niveau `debug` garde la ligne
  silencieuse par défaut ; la mesure de contrôle l'active explicitement.

- **Un trim sauté n'écrit aucune ligne.** Quand la garde de la décision 3 fait sauter l'appel, il n'y
  a **pas eu d'appel, donc rien à mesurer** : aucune ligne `trim` n'est écrite. `trimmed = 0` garde
  ainsi un seul sens, celui d'un appel qui n'a rien rendu ; le job suivant retente un trim.

### 5. Tests

Aucun test ni lint n'est désactivé.

1. **`trim_is_a_no_op_off_glibc`** : hors glibc, l'enveloppe rend `false` et n'appelle rien.
2. **Sur glibc, en CI Linux** (job « Rust », `ci.yml:17-19`) : allouer, libérer, appeler, et vérifier
   que la valeur de retour vaut `0` ou `1`. Ce test prouve l'édition de liens du symbole, que rien
   d'autre ne prouve à la compilation.
3. **Test d'intégration avec le puits de journal existant** (`crates/crible-server/tests/common/mod.rs:838-891`)
   sur un petit job (`patch_documents` ou `compact` sur un index temporaire) : **une seule** ligne
   `trim`, et **son ensemble exact de champs**, sur le modèle d'`assert_attach_duration_line`
   (`:514`), sous `cfg` glibc. Attention vérifiée : le puits s'installe **une fois** par binaire de
   test et le premier niveau gagne (`journal_at_level`, `:885-891`) ; la lecture d'une ligne `debug`
   demande donc un binaire de test qui installe le puits à `debug`.
4. **La garde « un trim à la fois »** : deux fins de job concurrentes ne produisent qu'un appel, et
   **l'appel sauté n'écrit aucune ligne `trim`** (décision 4).

### 6. Mesure de contrôle avant la fusion

`@mesureur` ajoute une **section datée** à MES-17, avec le harnais `tools/bench/mes17/` :

- **Deux images** : celle de la branche et celle de son commit parent. La seule différence est la
  correction.
- **Deux processus chacune**, états (a), (b), (c), **trois lectures à 3 s**.
- **Seuil** : la dispersion de **cette** campagne sur (c)−(b), entre ses deux processus
  (ADR-0157 `:101-102`), **jamais les 273,0 MiB de la troisième campagne**. **Sens** : la baisse
  (ADR-0158).
- **Lectures** : (c) avec trim contre (c) de référence ; le compte des `trimmed = 1` ; les durées de la
  ligne `trim` ; et **300 requêtes hybrides `k = 10` en vol pendant les fins de job**, p50 et p99
  contre la référence, avec le critère d'INV-02 (`docs/08:10`, p99 × 3).
- **Arrêt** : si le gain n'atteint pas le seuil, ou si le critère de latence échoue, la tâche s'arrête
  et le propriétaire décide. Le seuil n'est pas relâché.

### 7. Échelle réelle : non bloquant

Aucune mesure sur `decisions` ni sur le Mac mini n'est exigée avant la 1.1.0 (point `NON-COUVERT`
tranché ainsi). La **note de version** de la 1.1.0, écrite par la tâche `PUBLIER-V1.1.0` dans
`docs/versions/1.1.0.md` (ADR-0143, décision 4, `:86-88`), dira :

- la correction de comportement et son ADR ;
- « gain mesuré sur l'échantillon seulement », sans reprendre 10,55 → 3,57 Gio ;
- retour arrière = image précédente (ADR-0143) ;
- le reste (environ 550 MiB) et l'échelle de Legifacile renvoyés à la v2.

Une mesure à l'échelle réelle reste un choix du propriétaire (ADR-0142 : deux machines).

### 8. Ce que la correction ne fait pas

- Pas de **clé de configuration** (règle d'or 12).
- Pas de **changement d'allocateur** : c'est la liste de la v2 (ADR-0142 `:79`).
- Pas de **déchargement de session** (ADR-0105, décision 2).
- Aucun trim **périodique**, ni **pendant** un job.
- Ne promet rien sur le **pic pendant une attache**, ni sur l'**OOM** du ticket 05.
- N'attribue pas les **550 MiB restants**.

### 9. Classe de version

Classe **« ajout compatible », forme la plus faible**, au sens d'ADR-0143, décision 1 (`:48-58`), par
le précédent d'ADR-0151, décision 5 : la seule addition visible est une **ligne de journal nouvelle**,
sous `debug`. Aucun contrat, aucune route, aucun champ, aucun membre, aucune clé, ni `format_version`
ne change. Ni OpenAPI, ni client Python, ni `docs/05` ne sont touchés.

### 10. Tableau d'application, après acceptation

| Document ou livrable | Ce que l'application y écrit |
|---|---|
| `docs/adr/README.md` | une ligne d'index, statut passé à `Accepté` par le propriétaire |
| `docs/06-observabilite.md` | une **addition datée** dans « Mémoire » : trim en fin de job, son ordre après la publication, et la ligne `trim` avec ses champs et son niveau |
| `docs/adr/0142-…md` | un **renvoi daté** sous la limite de la mémoire anonyme (`:79`) ; corps non réécrit, **pas** de « Remplacé par » : la limite reste vraie pour la 1.0. Précédent : « Renvoi du 2026-11-21 » d'ADR-0151 (`:183-187`) |
| `docs/mesures/MES-17-classes-de-memoire.md` | une **section datée** de la campagne de contrôle, écrite par `@mesureur` |
| `tools/bench/mes17/` | la campagne de contrôle (deux images, deux processus, deux états, les requêtes en vol) |

**Inchangés** : `docs/02`, `docs/05`, `docs/07`, `docs/08`, et les corps d'ADR-0142, ADR-0143,
ADR-0149, ADR-0151, ADR-0157, ADR-0158 et ADR-0159.

## Conséquences

- **Positives** :
  - la part de l'anonyme retenu que la mesure attribue à la fragmentation de l'allocateur est rendue
    sans attendre un redémarrage : 40 à 53 % sur l'échantillon, au-dessus du seuil de la campagne ;
  - aucune dépendance nouvelle, aucun format, aucun contrat : `docs/07` reste fermé et le retour
    arrière est l'image précédente (ADR-0143) ;
  - la ligne `trim` rend le geste observable sans toucher aux contrats de journal déjà fermés ;
  - le chiffre de la ligne `memory` de `job_end` garde le même sens entre 1.0 et 1.1.
- **Négatives / coûts** :
  - un `unsafe extern "C"` de plus, sous une seule cible, à maintenir et à justifier ;
  - le comportement **diffère** entre glibc et le reste : une mesure native sur macOS ne dit rien du
    conteneur, comme pour la ligne `memory` (ADR-0151) ;
  - le coût d'un appel **en service** n'est pas connu : les 0,3 à 0,6 s de la troisième campagne
    mesurent `gdb`, pas la correction. La décision 6 le mesure, mais seulement sur l'échantillon ;
  - le gain reste **non démontré à l'échelle réelle** ; la note de version le dira ;
  - une fin de job peut sauter son trim quand un autre est en cours : la mémoire rendue dépend alors
    de l'ordre d'arrivée des jobs, et le trim suivant rattrape la plus grande part, pas la totalité ;
  - environ 550 MiB restent non classés et non rendus.
- **À surveiller** :
  - **la campagne de contrôle de la décision 6** : un gain sous son propre seuil, ou un p99 qui
    franchit 3 × la référence (INV-02), arrête la tâche ;
  - **`trimmed` à `0` de façon répétée** en exploitation : le tas n'a plus de pages libres à rendre, et
    la correction cesse d'apporter quoi que ce soit ;
  - **l'écart entre la ligne `memory` de `job_end` et l'anonyme du cgroup après le trim** : un écart
    qui grandit dirait que la fragmentation revient plus vite qu'elle est rendue ;
  - **une mesure à l'échelle réelle** : si elle montre un gain net sur `decisions`, la v2 doit
    reprendre la question du reste et, peut-être, celle de l'allocateur.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Ajouter `libc` en dépendance directe | Une ligne de plus dans `docs/07` et un épinglage pour un seul symbole, alors qu'une déclaration `extern` locale suffit ; ADR-0080, décision 3, demande un ADR pour `libc`, et le précédent de `rescore_ab.rs:914-925` montre que le dépôt s'en passe |
| Appeler `malloc_trim` dans chaque job, à la main | Une liste à tenir, un oubli à chaque genre nouveau, et une fin de job qui ne trime pas sans que rien ne le signale ; `mark_finished` est le passage unique de tous les genres |
| N'appeler que pour un sous-ensemble de genres | Une règle sans preuve : aucun genre n'est exempt de fragmentation, et un tas au repos rend `0` vite ; le sous-ensemble demanderait sa propre mesure pour se justifier |
| Une clé de configuration (`memory.trim = true/false`) | Règle d'or 12 : pas de configuration pour une valeur fixe ; la ligne `trim` et le retour arrière par image suffisent |
| Un trim périodique, ou sur minuterie | Une charge sans lien avec l'allocation : il rendrait des pages au hasard du calendrier, à un coût non mesuré, et sans rapport avec le moment où le tas se fragmente |
| Noter le trim dans la ligne `memory`, ou dans le membre `durations` | Les deux listes sont fermées : quatre `step` (ADR-0151, décision 2) et six clés (ADR-0149, `:57`). Une ligne `trim` distincte ne rouvre aucun des deux contrats |
| Un champ `skipped` dans la ligne `trim` | Un champ de plus pour un cas rare, sans lecture prévue : la mesure de contrôle compte les `trimmed = 1`, et un trim sauté se voit déjà à l'absence de ligne |
| `trimmed = 0` pour un appel sauté | Deux sens pour une même valeur : « appel fait, rien rendu » et « appel non fait » |
| Déplacer la ligne `memory` de `job_end` après le trim | Le chiffre changerait de sens entre 1.0 et 1.1, et les campagnes de MES-17 déjà écrites ne seraient plus comparables ; la ligne reste avant le trim |
| Changer d'allocateur (jemalloc, mimalloc) | Dépendance nouvelle et choix de compromis, rangé dans la liste de la v2 depuis le 2026-11-08 (ADR-0142 `:79`) ; aucun gain mesuré pour l'instant |
| Attribuer d'abord les 550 MiB restants (`mallinfo2`, profil de tas) | Un diagnostic, donc aucune mémoire rendue, et une dépendance ou un outil externe qui demandent leur ADR ; le propriétaire l'a renvoyé à la v2 le 2026-11-22 |
| Mesurer à l'échelle réelle sur `decisions` avant la 1.1.0 | Deux machines et une campagne lourde pour un correctif dont l'échantillon donne déjà le sens ; le propriétaire a tranché « non bloquant » (décision 7), la mesure restant son choix |

## Références

- Ticket : `/Users/mattboll/MattCorp/legifacile/tickets-crible/05-memoire-non-rendue.md` (projet
  Legifacile, lecture seule).
- Mesure : `docs/mesures/MES-17-classes-de-memoire.md`, section « Troisième campagne, 2026-11-22 :
  essai de `malloc_trim(0)` » (`:570-860`), et les deux campagnes antérieures de la même mesure.
- ADR : ADR-0080 (décision 3, `libc`), ADR-0104 et ADR-0105 (décision 2, sessions gardées), ADR-0107
  et ADR-0149 (durées d'attache, six clés), ADR-0124 (décision 1, forme fermée de `GET /v1/stats`),
  ADR-0142 (`:79`, mémoire anonyme après un `replace_all`), ADR-0143 (décisions 1 et 4, classes de
  version et note de version), ADR-0151 (décisions 2, 4 et 5, ligne `memory`, `:196-197`),
  ADR-0157 (`:101-102`, seuil interne à la campagne), ADR-0158 (sens de l'effet), ADR-0159
  (décision 7, `:204-216`, correction hors campagne).
- Code : `crates/crible-server/src/jobs.rs` (`mark_finished` `:459-514`, `publish` `:537`,
  `publish_with` `:546`, `store_and_notify` `:564`, appels `:1229`, `:1240`, `:1272`, `:1284`,
  `fail_queued` `:1442`, `JobBoard` `:320`), `crates/crible-server/src/openapi.rs:567-586` (les neuf
  genres), `crates/crible-server/src/memory.rs` (ligne `memory` d'ADR-0151),
  `crates/crible/examples/rescore_ab.rs:914-925` (précédent d'une déclaration `extern "C"` sans
  `libc`), `crates/crible-server/tests/common/mod.rs` (`Sink` `:838-891`,
  `assert_attach_duration_line` `:514`), `docker/Dockerfile:73` (image de production, glibc).
- CI : `.github/workflows/ci.yml:17-19` (job « Rust », image Linux).
- Règles : `AGENTS.md`, règle d'or 10 (robustesse, `unsafe` et `// SAFETY:`), règle d'or 12
  (simplicité, pas de configuration pour une valeur fixe) ; `docs/08:10` (INV-02).
