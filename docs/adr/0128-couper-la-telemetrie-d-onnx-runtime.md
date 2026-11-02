# ADR-0128 — Couper la télémétrie d'ONNX Runtime dans Crible

- **Statut** : Proposé
- **Date** : 2026-11-02
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, sur demande du propriétaire du 2026-11-02 (« oui vas-y pour 1 et 2 »), pour la préparation de J7 et d'après l'analyse de `@architecte` du 2026-11-02 (verdict `NON-COUVERT`, session `ses_e646075a7ffe9elNLW7GZaq5yZ`)
- **Remplace** : —

## Contexte

### Le blocage observé

La série locale `cargo test --release -- --include-ignored` de la préparation de J7 s'est bloquée **deux fois sur deux** sur `api_and_cli_return_the_same_results_on_the_legifacile_sample`. Le processus n'épuise pas le temps d'un test : un `crible search` **fini** tourne indéfiniment dans `exit()`. La pile relevée est celle de la sortie d'ONNX Runtime :

```
__cxa_finalize → ~PosixEnv → OneDsTelemetry::Shutdown
  → LogManagerImpl::FlushAndTeardown → TransmissionPolicyManager::handleStop
```

`handleStop` boucle sur `uploadCount()` : le client de télémétrie attend un envoi qui ne se termine pas.

### Ce que fait ONNX Runtime 1.31.0

La bibliothèque désignée par `ORT_DYLIB_PATH` — ONNX Runtime **1.31.0**, la version du groupe `ci` de `docs/07` — embarque le client de télémétrie Microsoft **1DS** :

- il crée un **identifiant d'appareil** et un stock SQLite sous `~/Library/Application Support/Microsoft/DeveloperTools/.onnxruntime/` (macOS) ;
- il envoie des données d'usage vers `mobile.events.data.microsoft.com` ;
- il s'arrête de lui-même quand il reconnaît un environnement d'intégration continue : `TF_BUILD`, `GITHUB_ACTIONS`, `GITLAB_CI`, `ORT_RUNNING_UNIT_TESTS`… ou quand `ORT_DISABLE_TELEMETRY` est posé. C'est pourquoi **la CI distante est toujours verte** : elle ne reproduit pas le défaut.

Le crate `ort` **2.0.0-rc.15** (`Cargo.toml:54`, `load-dynamic`) active la télémétrie par défaut. Son option `with_telemetry(false)` n'appelle `DisableTelemetryEvents` qu'**après** `CreateEnv` : elle ne couvre pas tout le cycle de vie de l'environnement. `crates/crible-encode/src/model.rs:619-648` construit cet environnement (`ort::init_from(...).build()`) sans rien couper.

### La preuve sur le binaire

Sur le binaire `crible`, avec un `HOME` isolé :

- **sans variable**, un `crible search` hybride crée l'identifiant d'appareil et le stock sous ce `HOME` ;
- **avec `ORT_DISABLE_TELEMETRY=1`**, rien n'est créé, et le résultat de recherche est le même.

### Les deux conséquences

1. **Toute sortie de processus peut se bloquer** : la CLI après une commande, `crible serve` à l'arrêt. Le défaut est indépendant de la charge du moteur et ne dépend ni de la requête ni de l'index.
2. **Crible envoie des données d'usage à un tiers, production comprise.** La conception ne le prévoit ni ne l'interdit :
   - `docs/06-observabilite.md:8` et `:36-40`, INV-13, ADR-0014 et la règle 11 d'`AGENTS.md` ne couvrent que les sorties **propres à Crible** (analytics, journal de debug, logs) ;
   - ADR-0032 ne parle que des accès des agents à la production ;
   - `docs/07-stack-et-deploiement.md:35`, `:54` et `:59` décrivent le chargement d'ONNX Runtime sans rien dire du réseau.

Le verdict de `@architecte` est donc `NON-COUVERT` : le comportement n'est ni conforme ni interdit, et il faut une décision.

### La question

Couper la télémétrie d'ONNX Runtime dans Crible, partout où ORT est chargé, et par quel mécanisme, sachant que le binaire, les binaires de test et l'image de P8 ne chargent pas ORT au même moment.

## Décision

**Crible coupe la télémétrie d'ONNX Runtime partout où il charge ORT.** La coupure est inconditionnelle.

### Décision 1 — pas d'opt-in

Aucune clé de configuration, aucune variable lue par Crible, aucun drapeau CLI ne rouvre la télémétrie. Si on veut la rouvrir un jour, ce sera par un **nouvel ADR** (règle 12 : pas de configuration pour une valeur fixe).

### Décision 2 — le binaire `crible` pose la variable au début de `main`

`crates/crible/src/main.rs` pose `ORT_DISABLE_TELEMETRY=1` au tout début de `main`, **avant** `Cli::parse()` (`:338-339`) et avant tout fil.

Précisions :

- la valeur est **forcée à `1`**, même si l'environnement en donne une autre ;
- `std::env::set_var` est `unsafe` en édition 2024 : le code porte un commentaire `// SAFETY:` qui dit qu'aucun fil n'existe encore, et un test le couvre (règle 10) ;
- un seul point de pose couvre la CLI et `serve`, puisque les deux passent par `main`.

### Décision 3 — `with_telemetry(false)` en plus, sans effet sur la sûreté

L'`EnvironmentBuilder` de `crible-encode` reçoit aussi `with_telemetry(false)`. Ce n'est pas la preuve : c'est une seconde barrière pour les cas où ORT serait chargé sans passer par le binaire. Elle ne porte aucune garantie de sûreté.

### Décision 4 — jamais de `set_var` dans `crible-encode`

Une bibliothèque appelée après le démarrage de tokio ou de rayon ferait la course avec des `getenv` concurrents. La variable est donc posée par le binaire seulement, jamais par la bibliothèque.

### Décision 5 — `.cargo/config.toml`, section `[env]`

Un `.cargo/config.toml` nouveau porte :

```toml
[env]
ORT_DISABLE_TELEMETRY = { value = "1", force = true }
```

`force = true` écrase une valeur déjà présente dans l'environnement. Cela couvre les **binaires de test** qui chargent ORT dans leur propre processus et ne passent pas par `main` du binaire `crible`.

### Décision 6 — l'image Docker porte la variable (P8)

L'image de P8 pose `ENV ORT_DISABLE_TELEMETRY=1`. Ce point n'est pas implémenté ici : il s'applique par la fiche `CONSTRUIRE-IMAGE-DOCKER` de `docs/phases/P8-docker-et-deploiement.md`, au moment de l'application du présent ADR.

### Décision 7 — Python : documentation seulement

- le client Python (`clients/python/`) est **hors périmètre** : il ne parle que HTTP au serveur et ne charge pas ORT ;
- `tools/` utilise la même roue ORT, mais rien n'y est livré. Le fait est documenté dans `tools/README.md`, sans code.

### Décision 8 — la preuve

Un test `#[ignore]` lance un enfant `crible search` avec des `HOME`, `XDG_DATA_HOME` et `XDG_CONFIG_HOME` temporaires. Il retire de l'environnement de l'enfant :

`ORT_DISABLE_TELEMETRY`, `TF_BUILD`, `GITHUB_ACTIONS`, `GITLAB_CI`, `CI`, `ORT_RUNNING_UNIT_TESTS`.

Il vérifie :

- le code de sortie `0` ;
- l'absence de tout `.onnxruntime` ou `onnxruntime.db` sous ces dossiers.

Le test tourne dans le job `rust-onnx` (`--include-ignored`, ADR-0057). Le compte rendu de `@dev` montre l'**échec avant correction** sous macOS ; si Linux n'écrit rien, il le dit.

### Décision 9 — pas de nouvel invariant

Le test de la tâche suffit. Un INV-18 « aucune télémétrie tierce » dans `docs/08-invariants-et-tests.md` est écarté : la propriété n'est pas vérifiée à chaque exécution de la suite, et un invariant impliquerait un test non ignoré sur toutes les plateformes, ce que la CI distante ne peut pas prouver (elle coupe d'elle-même la télémétrie).

### Décision 10 — documents à modifier à l'application

Quand le propriétaire acceptera l'ADR :

- `docs/07-stack-et-deploiement.md` : politique ORT près de `:54` et `:59`, et la section Docker vers `:116-123` (`ENV ORT_DISABLE_TELEMETRY=1`) ;
- `docs/06-observabilite.md` : une phrase disant qu'aucune télémétrie tierce n'est émise ;
- la fiche `CONSTRUIRE-IMAGE-DOCKER` de P8 : livrable et critère ;
- `tools/README.md`.

### Ce qui n'est pas décidé ici

- la façon de rouvrir la télémétrie : elle demandera un nouvel ADR ;
- la mesure du temps de sortie avant et après : le test prouve l'absence de fichiers, pas un gain de latence ;
- le comportement d'ORT sous Windows : non testé.

## Conséquences

- **Positives** :
  - la sortie de processus ne peut plus se bloquer sur `OneDsTelemetry::Shutdown`, ce qui débloque la série `--include-ignored` de J7 et l'arrêt de `serve` ;
  - aucune donnée d'usage ne part vers un tiers, production comprise, sans changement de la conception observable par l'utilisateur ;
  - les résultats de recherche sont inchangés : la télémétrie ne touche pas au calcul.
- **Négatives / coûts** :
  - `set_var` reste `unsafe` et impose un commentaire `// SAFETY:` et un test ;
  - un `.cargo/config.toml` nouveau s'applique à tout le workspace, y compris aux outils qui chargeraient ORT hors Crible ;
  - la variable forcée dans l'environnement du processus masque un réglage que l'opérateur pourrait vouloir poser ;
  - le test de preuve est `#[ignore]` : il ne tourne pas dans la suite ordinaire, seule la CI `rust-onnx` le rejoue.
- **À surveiller** :
  - une montée de version d'`ort` ou d'ONNX Runtime qui changerait le nom ou l'effet de `ORT_DISABLE_TELEMETRY` : le test de preuve le dirait ;
  - l'apparition d'un nouveau fichier d'état sous le `HOME` dans le test ;
  - la durée de `api_and_cli_return_the_same_results_on_the_legifacile_sample` en local : un retour du blocage signifierait qu'une autre bibliothèque de sortie se bloque, pas qu'ORT a repris sa télémétrie.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Poser `set_var` dans `crible-encode` (décision 4) | Une bibliothèque peut être appelée après le démarrage de tokio ou de rayon : la course avec des `getenv` concurrents est indéfinie. Le point de pose doit être avant tout fil. |
| `with_telemetry(false)` seul (décision 3) | L'appel a lieu après `CreateEnv` : l'environnement a déjà pu créer l'identifiant et le stock, et il ne couvre ni les binaires de test ni un chargement hors de `crible-encode`. Insuffisant comme preuve. |
| Exiger la variable du seul déploiement (point 6 seul) | Ne couvre pas la CLI lancée à la main, ni les tests locaux, ni un `crible serve` démarré hors conteneur. Le blocage observé est justement local. |
| Laisser la télémétrie et contourner le blocage par `_exit` | Masque le symptôme, garde l'envoi de données à un tiers et laisse un identifiant d'appareil sur le disque. Contraire à la règle 11 et à l'esprit d'INV-13. |
| Un INV-18 « aucune télémétrie tierce » | Un invariant suppose un test non ignoré, rejoué partout. La CI distante coupe la télémétrie d'elle-même, donc elle ne prouverait rien ; le test de la tâche suffit (décision 9). |

## Références

- `docs/adr/0057-onnx-runtime-et-vrai-modele-mdenseon-en-integration-continue.md` (chargement d'ORT en CI, job `rust-onnx`, `--include-ignored`)
- `docs/adr/0014-analytics-sans-contenu.md` (analytics sans contenu)
- `docs/adr/0032-acces-a-la-production.md` (accès à la production)
- `docs/06-observabilite.md:8`, `:36-40` (INV-13, analytics et journal de debug)
- `docs/07-stack-et-deploiement.md:35`, `:54`, `:59`, `:116-123` (chargement d'ORT, version 1.31.0, image Docker)
- `docs/08-invariants-et-tests.md` (INV-13, et absence d'invariant sur la télémétrie tierce)
- `crates/crible/src/main.rs:338-339` (entrée de `main`)
- `crates/crible-encode/src/model.rs:619-648` (construction de l'environnement ORT)
- `Cargo.toml:54` (`ort = "=2.0.0-rc.15"`, `load-dynamic`)
- `crates/crible/tests/api_and_cli_on_sample.rs` (test bloqué)
