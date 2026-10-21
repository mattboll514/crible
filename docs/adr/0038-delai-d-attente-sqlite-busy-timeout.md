# ADR-0038 — Régler un délai d'attente sur les verrous SQLite (`busy_timeout`)

- **Statut** : Proposé
- **Date** : 2026-10-21
- **Décideurs** : propriétaire
- **Proposé par** : propriétaire, relayé par l'orchestrateur ; rédaction par l'agent greffier, d'après la relecture de la tâche STOCKER-ENREGISTREMENTS-SQLITE (commit `9f4cecb`)
- **Remplace** : —

## Contexte

`records.sqlite` est la base d'enregistrements d'une époque et le support du point de commit unique (ADR-0016). `crible-store::records` l'ouvre en mode WAL : un écrivain, `RecordStore`, qui détient la base par `&mut self`, et un pool borné de connexions en lecture seule, `ReadPool` (`crates/crible-store/src/records.rs`). Le mode WAL garantit ce qui compte pour INV-02 : un lecteur ne bloque jamais l'écrivain, et l'écrivain ne bloque jamais les lecteurs (`docs/02-architecture.md`, « Concurrence »).

**Le manque.** Aucun `busy_timeout` n'est réglé, ni sur la connexion de l'écrivain ni sur celles du pool. La documentation de `RecordStore::open` le dit et annonce la décision à venir : « A transient `SQLITE_BUSY` is reported as `internal`: no `busy_timeout` is set until the value is decided by an ADR ». Toute erreur SQLite passe par `database_error`, qui la traduit en `ErrorCode::Internal` (HTTP 500). Un `SQLITE_BUSY` de quelques millisecondes ressort donc comme une panne interne, alors que c'est une contention passagère.

**Ce qui pèse.** Fixer ce délai est une valeur par défaut : c'est un compromis d'exploitation, donc un ADR (règle 4 d'`AGENTS.md`, et `docs/adr/README.md`, « Quand écrire un ADR »). Le relecteur de STOCKER-ENREGISTREMENTS-SQLITE l'a relevé ; `@dev` et l'orchestrateur ont refusé de trancher seuls. La ligne correspondante est dans `docs/suivi.md` (« Écarts à la conception »).

### Où `SQLITE_BUSY` peut survenir malgré WAL et l'écrivain unique

La règle « un écrivain par index » est une règle **de processus** : une tâche tokio, une file bornée (`docs/02-architecture.md`, « Concurrence »). Elle ne verrouille pas le fichier entre processus. Quatre cas restent possibles :

1. **Deux processus sur la même base.** Le binaire `crible` est une CLI qui expose `serve`, `attach`, `inspect`, `compact`, `snapshot`, `export` (`docs/07-stack-et-deploiement.md`, arborescence). Rien n'empêche un opérateur de lancer `crible attach` sur une époque pendant que le serveur la sert. Le second écrivain attend le verrou d'écriture WAL, qui n'admet qu'un détenteur. C'est le cas le plus probable en exploitation, et celui qu'aucune règle interne ne couvre.
2. **Récupération du WAL à l'ouverture.** Quand la dernière connexion ne s'est pas fermée proprement, la première connexion suivante rejoue le WAL. En WAL, cette récupération demande un verrou exclusif sur le `-shm` ; si une autre connexion détient un verrou partagé, SQLite renvoie `SQLITE_BUSY` (ou `SQLITE_BUSY_RECOVERY`). Cela touche les deux chemins d'ouverture : `RecordStore::create`/`open` et `ReadPool`, qui ouvre ses connexions à la demande.
3. **Initialisation du `-shm`.** Une connexion en lecture seule ne peut pas créer ni réparer le `-shm` : elle suppose que l'index du WAL existe déjà. Le commentaire de `ReadPool` le note (« Reading a WAL database read-only works because the epoch directory is writable, so SQLite may create the `-shm` file »). Si le `-shm` manque ou doit être récupéré, le pool peut échouer à l'ouverture ; selon le cas SQLite renvoie `SQLITE_BUSY` ou `SQLITE_READONLY_RECOVERY`. Le second n'est pas un délai d'attente, il restera une erreur interne.
4. **Checkpoint.** Le checkpoint automatique se déclenche côté écrivain ; un checkpoint explicite (`compact`, instantané par `VACUUM INTO`, `docs/04-donnees-et-ingestion.md`) prend un verrou. Un checkpoint passif qui trouve des lecteurs actifs rend `SQLITE_BUSY` sans être fatal : il retentera. Une transaction d'écriture concurrente, elle, attend.

**Ce qui n'est pas en cause.** Le délai d'une requête n'est pas ce sujet : `limits.default_timeout_ms = 2000` (`docs/07-stack-et-deploiement.md`, section « Configuration de l'instance ») borne la durée d'une recherche. Un `busy_timeout` plus long que ce plafond ferait attendre une lecture au-delà de sa propre échéance.

## Décision

**Option B : un délai fixe de 1 000 ms, réglé sur toute connexion à `records.sqlite` — celle de l'écrivain comme celles du pool de lecture.** Quand le délai expire, l'erreur remonte en `ErrorCode::Overloaded` (`overloaded`, HTTP 503), et non en `internal`.

Précisions :

- **La valeur est une constante du code**, pas une clé de `crible.toml`. Un seul endroit : une fonction privée de `crible-store::records` qui applique le réglage, appelée par `open_or_create`, `open_writable` et `open_read_only`, donc par `RecordStore::create`, `RecordStore::open` et `ReadPool`.
- **1 000 ms** parce qu'un verrou légitime ne dure jamais si longtemps : un commit d'attache écrit quelques milliers de lignes, un checkpoint quelques millisecondes. Attendre une seconde entière signifie que deux écrivains se disputent la base, ce qui contredit la règle « un écrivain par index » et doit échouer vite, pas patienter. La valeur reste sous `default_timeout_ms = 2000`, donc une lecture ne dépasse jamais l'échéance de sa requête.
- **`overloaded` plutôt que `timeout`.** `timeout` (504) est réservé à une requête qui dépasse son délai (`limits.default_timeout_ms`). Ici, la cause est une contention transitoire : le client peut réessayer, c'est la sémantique d'`overloaded`. Cela impose de distinguer `SQLITE_BUSY` des autres erreurs SQLite dans la traduction : `database_error` garde `internal` pour tout le reste, et un chemin dédié rend `overloaded` pour `SQLITE_BUSY` (y compris `SQLITE_BUSY_SNAPSHOT` et `SQLITE_BUSY_RECOVERY`) une fois le délai écoulé.
- **Le code de `RecordStore::open` est mis à jour** : il documente aujourd'hui l'absence de délai et annonce cet ADR.

Ce qui n'est **pas** décidé ici : la politique de verrou entre processus (fichier de verrou, refus d'une deuxième instance sur la même époque), le délai du pool de connexions ou de la file d'écriture, et le traitement de `SQLITE_READONLY_RECOVERY`.

## Conséquences

- **Positives** :
  - une contention passagère ne remonte plus comme une panne interne : un `attach` concurrent d'un `inspect` réussit au lieu d'échouer en 500 ;
  - le code d'erreur redevient honnête : `overloaded` dit « réessaie », `internal` dit « bug » ;
  - la latence de recherche ne bouge pas en régime normal : un lecteur WAL n'attend pas l'écrivain (INV-02) ;
  - aucune clé de configuration ajoutée, donc la règle 12 d'`AGENTS.md` (« pas de configuration pour une valeur fixe ») est respectée et `crible.toml` ne s'allonge pas.
- **Négatives / coûts** :
  - au pire, une opération attend 1 000 ms avant d'échouer : une écriture en conflit est 1 s plus lente qu'aujourd'hui ;
  - le mapping `SQLITE_BUSY` → `overloaded` ajoute un cas à tester dans `database_error` ;
  - une attache longue (transaction de commit unique, voir ATTACHER-LOT) tient le verrou d'écriture pendant toute sa durée : un second écrivain attendra 1 s puis échouera, ce qui est le comportement voulu, mais l'exploitant doit le savoir.
- **À surveiller** :
  - compter les `overloaded` d'origine SQLite dans les analytics : quelques-uns par jour sont normaux (CLI concurrente), une série signifie deux écrivains installés ;
  - la mesure de concurrence de P6 (TESTER-CHARGE-CONCURRENTE, INV-02) : le p99 des recherches doit rester dans la cible × 3, ce qui vérifie que les lecteurs n'attendent pas ;
  - si un `SQLITE_BUSY` survient alors qu'un seul processus tourne, la valeur est à revoir : le cas est un défaut, pas une contention.

### Application attendue dans le code

- `crates/crible-store/src/records.rs` : une constante (`BUSY_TIMEOUT_MS = 1000`), une fonction qui applique `Connection::busy_timeout(Duration::from_millis(BUSY_TIMEOUT_MS))`, appelée dans `open_or_create`, `open_writable` et `open_read_only` ;
- `database_error` : `SQLITE_BUSY` et ses variantes deviennent `overloaded`, le reste reste `internal` ;
- test attendu : deux connexions écrivent la même base ; la seconde, lancée pendant que la première tient une transaction d'écriture, attend puis échoue en `overloaded` après le délai ; un second test où le verrou est libéré au bout de quelques dizaines de millisecondes montre que l'attente réussit, ce qui prouve que le délai est bien appliqué et non un simple échec immédiat ;
- INV-02 et INV-03 restent inchangés : ce délai ne crée ni attente de lecture, ni génération partielle.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| A. Ne rien régler : échec immédiat | Un `SQLITE_BUSY` de quelques millisecondes devient un 500 `internal` ; l'exploitant ne peut pas distinguer une contention d'un bug, et une CLI concurrente fait échouer une attache valide. C'est l'état actuel, que le relecteur a signalé |
| C. Délai réglable dans `crible.toml` | Une valeur par défaut réglable pour un cas rare : une clé de plus à documenter, à valider et à surcharger par variable d'environnement, pour un réglage que personne ne devrait changer. Contraire à la règle 12 d'`AGENTS.md` et à l'esprit de `docs/07`, dont la configuration ne contient que ce qui varie d'une instance à l'autre |
| B′. Délai distinct pour l'écrivain et pour les lecteurs | Deux valeurs à justifier et à tester, alors que la même seconde couvre les deux besoins ; le lecteur n'a pas besoin d'attendre plus, et l'écrivain ne doit pas attendre plus |
| Délai long (10 s, 30 s) | Dépasse `default_timeout_ms = 2000` : une recherche attendrait au-delà de sa propre échéance, ce qui rend le dépassement de délai incohérent |
| Traduire tout `SQLITE_BUSY` en `timeout` | `timeout` (504) décrit une requête trop lente, pas une contention de verrou ; le client réessaierait avec la mauvaise politique |

## Références

- `docs/02-architecture.md` (« Générations et point de commit », « Concurrence ») ;
- ADR-0016 (le commit SQLite est l'unique point de commit) ; ADR-0017 (la compaction suspend les écritures) ;
- `docs/08-invariants-et-tests.md` : INV-02, INV-03 ;
- `docs/07-stack-et-deploiement.md` (« Configuration de l'instance » : `limits.default_timeout_ms`) et l'arborescence du binaire `crible` ;
- `crates/crible-store/src/records.rs` (`RecordStore::create`, `RecordStore::open`, `ReadPool`, `database_error`) ;
- `docs/suivi.md` (« Écarts à la conception », ligne STOCKER-ENREGISTREMENTS-SQLITE) ;
- tâche STOCKER-ENREGISTREMENTS-SQLITE (commit `9f4cecb`) ; tâche TESTER-CHARGE-CONCURRENTE (P6).
