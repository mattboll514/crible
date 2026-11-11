# ADR-0147 — Domaine de `[limits].default_timeout_ms`

- **Statut** : Accepté (accepté par anticipation par le propriétaire le 2026-11-11, sur délégation du propriétaire du 2026-11-11 : « pour l'idée notée je te laisse décider ce qui est le mieux »)
- **Date** : 2026-11-11 (proposition et acceptation anticipée)
- **Décideurs** : propriétaire
- **Proposé par** : agent `@greffier`, pour la tâche `BORNER-DEFAULT-TIMEOUT` de la v1.1 (hors tickets de Legifacile), sur la base du verdict de `@architecte` du 2026-11-11 rendu pendant la relecture de `T06-LISTE-INDEX-AU-DEMARRAGE`
- **Remplace** : — (complète ADR-0097 et ADR-0102 sans modifier leur corps ; ne remplace aucun ADR)

## Contexte

### Le constat

`[limits].default_timeout_ms` est le défaut du `timeout_ms` d'une recherche. `docs/07-stack-et-deploiement.md:106` le déclare à `2000` sans écrire aucun domaine, et le chargeur de configuration du serveur accepte toute valeur positive ou nulle : `crates/crible-server/src/config.rs:420` lit la valeur par `milliseconds`, et cette fonction (`:802-815`) ne refuse que le négatif, avec le message « `'default_timeout_ms' must not be negative` ». Rien d'autre ne la contrôle : ni ADR-0097, qui règle les surcharges `CRIBLE_*` et le refus des clés inconnues sans parler de ce domaine, ni ADR-0102, dont la décision 3 borne la valeur **de la requête** et ne dit rien du domaine de son défaut.

Or la valeur de requête, elle, est bornée : `docs/05-api.md:284` donne `timeout_ms` de 1 à 30 000 ms, et ADR-0102, décision 3, refuse hors bornes en `invalid_request` (400). Le contrôle est unique, dans `crible_server::limits::bounded` (`crates/crible-server/src/limits.rs:83-90`), avec les constantes `MIN_TIMEOUT_MS = 1` et `MAX_TIMEOUT_MS = 30_000` (`:42`, `:45`) et le message « `'timeout_ms' must be between 1 and 30000 milliseconds` ».

**Les deux valeurs se rencontrent à chaque recherche sans `timeout_ms`.** `limits::timeout` prend la valeur du corps, ou à défaut `[limits].default_timeout_ms`, puis la passe à `bounded` (`crates/crible-server/src/limits.rs:70-72`). Les deux routes de recherche font ce chemin : la route simple par `state.default_timeout_ms()` puis `limits::timeout` (`crates/crible-server/src/search.rs:196-200`), la route multi-index de la même façon (`crates/crible-server/src/search_multi.rs:547`).

**Conséquence mesurable du constat :** avec `[limits].default_timeout_ms = 0` ou `60000`, **toute** recherche sans `timeout_ms`, simple ou multi-index, rend `400 invalid_request` et le message ci-dessus. Le serveur accuse donc le client d'une faute qui vient de sa propre configuration, et une recherche qui n'a rien demandé de particulier ne peut pas aboutir.

### Le second effet : la borne du drainage d'arrêt

Le drainage des requêtes en vol dure au plus `[limits].default_timeout_ms` + 1 000 ms (ADR-0099, étape 3 de la séquence d'arrêt ; `docs/07-stack-et-deploiement.md:130`, constante à côté de `crates/crible-server/src/lifecycle.rs:44` et appliquée à `:327`). Ce drainage n'a aujourd'hui aucun plafond : à `60000`, il dure 61 s, soit **au-delà de `wait_timeout_ms`** (60 000 ms par défaut, ADR-0067), sur lequel le délai d'arrêt du conteneur est calé — `docs/07-stack-et-deploiement.md:130` exige un `stop_grace_period` **au-dessus** de `wait_timeout_ms`. L'orchestrateur tuerait donc le processus pendant son drainage.

**Ce qui n'est pas en cause.** `?wait=true` et `wait_timeout_ms` ne bougent pas : l'attente d'un job est bornée par `wait_timeout_ms` et rend `202` à l'échéance, jamais `504` (ADR-0067, ADR-0104). `timeout_ms` d'une requête ne change pas non plus : son domaine, son défaut documenté et son `504` restent ceux d'ADR-0102, décision 3.

### Ce que la conception dit, et ne dit pas

`docs/05-api.md:615`, dans « Limites par défaut (configurables) », écrit « `timeout_ms` : 2 000 par défaut (`[limits].default_timeout_ms`), 30 000 au plus ». La ligne nomme la clé de configuration comme l'origine du défaut, sans dire ce que vaut le domaine **de cette clé**. `docs/07-stack-et-deploiement.md` documente ailleurs les domaines de ses clés : `ttl_days` et `max_rows` de `[debug_journal]` (« 0 refusé au démarrage »), `max_bytes` (« 0 refusé, minimum 1 Mio »), `sample_rate` (« dans [0, 1] »). `default_timeout_ms` est la seule ligne du bloc `[limits]` sans domaine.

**Le mécanisme du refus au démarrage existe déjà, et il est écrit dans le code trois fois** : `ttl_days = 0` (ADR-0122, décision 8, `crates/crible-server/src/config.rs:362-370`), `max_rows = 0` (ADR-0127, décision 1, `:371-379`), `max_bytes` sous 1 Mio (`:380-388`). Chacun rend une erreur `invalid_request` de configuration dont le message nomme la clé, le chemin, la variable de surcharge quand elle est en cause — `origin` écrit `limits.default_timeout_ms (set by CRIBLE_LIMITS__DEFAULT_TIMEOUT_MS)` (`:697-702`) — et la valeur reçue ; le serveur sort alors avec un code non nul, comme pour tout échec de chargement de `crible.toml` (`docs/07-stack-et-deploiement.md`, « Commande »). Ce n'est donc pas une mécanique nouvelle que cet ADR introduit : c'est la même, appliquée à une clé de plus.

**Aucun invariant ne couvre la validation de la configuration.** INV-15 ne vise que les formats sur disque, versionnés et little-endian ; le refus d'une valeur de configuration est un comportement du serveur, décrit par les ADR ci-dessus et testé par les tests unitaires de `crates/crible-server/src/config.rs`. Cet ADR n'ajoute ni ne change d'invariant.

### Ce qui a été écarté par la recherche des pistes, et ce qui reste à vérifier

La recherche des pistes déjà tranchées par un ADR ou un document de conception n'a **rien trouvé d'écarté** : ADR-0102 borne la valeur de la requête, ADR-0097 règle la forme des surcharges, ADR-0067 et ADR-0104 règlent l'attente des jobs, et aucun texte ne traite le domaine de `default_timeout_ms`. Il n'y a donc pas de décision antérieure à contredire, seulement un trou.

Reste à vérifier par la tâche qui appliquera cet ADR : qu'aucune configuration de test ni aucun gabarit de `docs/jalons/` ne porte une valeur hors du domaine — toutes les configurations versionnées de l'arbre valent `2000`, sauf le test `serve_exits_at_once_on_a_second_signal_during_the_drain` (`crates/crible/tests/serve_cli.rs:601`, 60 000), qui passera à la borne haute de 30 000 dans la tâche (détail au paragraphe « Le retour arrière n'est pas touché » de la décision 4).

## Décision

**`[limits].default_timeout_ms` est borné de 1 à 30 000 ms inclus, les bornes du `timeout_ms` d'une requête. Une valeur hors de ce domaine, venue de `crible.toml` ou de la surcharge `CRIBLE_LIMITS__DEFAULT_TIMEOUT_MS`, fait refuser le démarrage du serveur.**

### 1. Le domaine, et une seule source pour les deux bornes

Le domaine est **1 à 30 000 ms inclus**. Ce sont exactement les bornes d'ADR-0102, décision 3, pour la valeur de requête, et le défaut reste dans le domaine : 2 000 ms.

Le code **réutilise les constantes existantes** `crible_server::limits::MIN_TIMEOUT_MS` et `MAX_TIMEOUT_MS` (`crates/crible-server/src/limits.rs:42`, `:45`) dans la validation de la configuration. Il n'y a donc **qu'une source** pour les deux bornes, et une modification future du domaine de `timeout_ms` ne peut pas laisser son défaut dans un domaine différent.

Aucune **clé de configuration nouvelle** n'est ajoutée : le domaine est une constante du code, comme la limite de 32 Mio du corps d'une requête (ADR-0102, décision 1). Ajouter un plafond configurable serait « une configuration pour une valeur fixe », ce que la règle 12 d'`AGENTS.md` interdit.

### 2. Refus au démarrage, avec un message qui nomme la clé, la variable et le domaine

Une valeur hors du domaine fait **refuser le démarrage**, avec un **code de sortie non nul** et un message qui nomme :

- la clé et son chemin, `limits.default_timeout_ms` ;
- la variable de surcharge, `CRIBLE_LIMITS__DEFAULT_TIMEOUT_MS`, quand c'est elle qui porte la valeur fautive (mécanisme d'`origin`, `crates/crible-server/src/config.rs:697-702`) ;
- le domaine attendu et la valeur reçue.

C'est **le même mécanisme** que les refus existants de `[debug_journal]` : `ttl_days` (ADR-0122, décision 8), `max_rows` (ADR-0127, décision 1) et `max_bytes` (ADR-0127, décision 1). La forme attendue du message suit la leur, par exemple « `setting 'default_timeout_ms' must be between 1 and 30000 at limits.default_timeout_ms (set by CRIBLE_LIMITS__DEFAULT_TIMEOUT_MS), got 0` », la formulation exacte restant à `@dev` dans le style des messages voisins.

Le refus a lieu au chargement de la configuration, **avant la liaison au port et avant l'ouverture d'un index** : une erreur de configuration ne laisse donc jamais une instance à moitié démarrée.

### 3. Conséquence sur l'arrêt du serveur

Le drainage des requêtes en vol est désormais borné à **31 000 ms au plus** (30 000 + 1 000), donc **sous `wait_timeout_ms`** à son défaut de 60 000 ms. Le `stop_grace_period` réglé au-dessus de `wait_timeout_ms` couvre alors les deux bornes de l'arrêt, et non plus seulement l'attente des jobs.

**Rien d'autre ne change dans la séquence d'arrêt** : ADR-0099 et ADR-0104 restent la règle, l'ordre des étapes, le second signal et le traitement des jobs inchangés, et aucune constante de `lifecycle` n'est retouchée : c'est la valeur de `default_timeout_ms` qui est bornée, pas le + 1 000 ms.

### 4. Compatibilité : correction de comportement, à signaler dans la note de version

Au sens d'ADR-0143, décision 1, c'est une **correction de comportement** : le comportement actuel n'est documenté nulle part, et il contredit `docs/05`, qui borne `timeout_ms` de 1 à 30 000 ms alors que son défaut peut sortir de ces bornes. ADR-0143, décision 1, autorise une telle correction dans une mineure **à condition qu'un ADR dise pourquoi c'en est une** — c'est l'objet de celui-ci.

**Réserve à reporter dans la note de version de la 1.1.0** (ADR-0143, décision 4), pour la tâche `PUBLIER-V1.1.0` :

> une configuration 1.0 dont `default_timeout_ms` est hors de 1..30 000 ne démarre plus en 1.1 ; elle faisait déjà refuser toute recherche sans `timeout_ms`.

**Le retour arrière n'est pas touché.** Aucun format persistant ne change, aucune clé n'est ajoutée : une configuration valide en 1.1 se charge à l'identique en 1.0. `rg -n "default_timeout_ms"` dans les fichiers versionnés du dépôt ne rend que `2000` — `docker/crible.example.toml:60`, `docker/crible.demo.toml:45`, l'exemple embarqué de `crates/crible-server/src/config.rs:892`, les configurations des tests et celles de `tools/bench/` (`tools/bench/mes16/mes16_charge.py:779`, `tools/bench/p10-import/prepare.sh:139`) —, à une exception près : `crates/crible/tests/serve_cli.rs:601` construit une configuration à `60000` pour tenir le drainage ouvert dans `serve_exits_at_once_on_a_second_signal_during_the_drain`, et cette valeur sort du domaine ; ce test devra donc passer à la borne haute de 30 000, qui laisse le drainage à 31 s, bien au-delà des 2 s qu'il mesure. Une valeur hors domaine n'était déjà pas utilisable, puisqu'elle faisait refuser toute recherche sans `timeout_ms`. La configuration de production n'est pas lue ici (ADR-0032) : l'opérateur la vérifie avant la montée.

### Ce qui n'est pas décidé ici

- **Le domaine de `timeout_ms` d'une requête** : ADR-0102, décision 3, inchangé.
- **Le défaut de 2 000 ms** : `docs/07` et `docs/05`, inchangés.
- **La valeur de `wait_timeout_ms` et le `stop_grace_period`** : ADR-0067, ADR-0104 et `docs/07`, inchangés. Cet ADR ne fait que constater que le drainage passe désormais sous `wait_timeout_ms`.
- **Un contrôle de `[limits]` autre que celui-ci** : `writer_queue` et `query_cache_entries` gardent leur validation actuelle.
- **La note de version elle-même** : `docs/versions/1.1.0.md` revient à `PUBLIER-V1.1.0`.

## Conséquences

- **Positives** :
  - une recherche sans `timeout_ms` aboutit toujours sur une instance qui a démarré : le serveur ne peut plus accuser un client d'une faute de sa propre configuration ;
  - une faute de configuration se voit au démarrage, avec un message qui nomme la clé, la variable et le domaine, au lieu de se manifester par un `400` sur chaque recherche ;
  - le drainage d'arrêt est borné à 31 s au plus, sous `wait_timeout_ms` par défaut, ce qui rend le `stop_grace_period` cohérent avec les deux bornes de l'arrêt ;
  - une seule source pour les deux bornes : le défaut et la valeur de requête ne peuvent plus diverger par inadvertance ;
  - aucun invariant, aucun `format_version`, aucune route, aucun code d'erreur, aucune clé de configuration ne change.
- **Négatives / coûts** :
  - une configuration 1.0 qui portait une valeur hors domaine ne démarre plus en 1.1 : c'est une correction, et la note de version le dit ;
  - la validation de la configuration gagne une dépendance de plus vers `limits` — c'est une dépendance interne à `crible-server`, sans crate nouvelle ;
  - les tests de configuration gagnent les cas de bornes, un par borne et un par forme de message (fichier, surcharge).
- **À surveiller** :
  - une valeur de production hors domaine, qui ferait échouer une montée : le symptôme serait un serveur qui refuse de démarrer avec le message nommant la clé, et la réponse serait de corriger `crible.toml` ;
  - un futur changement du domaine de `timeout_ms` (ADR-0102, décision 3) qui déplacerait les bornes du défaut sans qu'on y pense : les constantes partagées sont là pour que le déplacement soit unique, et le test de bornes de la configuration le dit ;
  - un `stop_grace_period` réglé juste au-dessus de `wait_timeout_ms` alors que `default_timeout_ms` vaudrait la borne haute : 31 s contre 60 s laisse 29 s de marge, qu'un futur réglage de `wait_timeout_ms` à la baisse pourrait réduire.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| **Refuser seulement `0`** | Incomplet : une valeur **au-dessus** de 30 000 casse aussi toute recherche sans `timeout_ms`, avec le même `400`. Le domaine doit couvrir les deux bornes, les mêmes que celles de la requête. |
| **Ramener en silence la valeur dans les bornes** | Une faute de configuration passerait sans effet visible : le serveur servirait un défaut que l'opérateur n'a pas écrit, et il ne saurait jamais que sa configuration est fautive. C'est contraire au principe d'ADR-0031 et d'ADR-0097 — une faute de frappe ne doit pas passer sans effet. |
| **Documenter le domaine sans corriger le code** | Figerait la contradiction avec `docs/05` : la documentation dirait un domaine que le code n'applique pas, et le `400` sur chaque recherche resterait. Un document qui décrit un contrôle absent est pire qu'un document muet. |
| **Ajouter une clé de plafond configurable pour le drainage** | Une configuration pour une valeur fixe (règle 12 d'`AGENTS.md`) : le drainage n'a pas de raison de varier par instance, et une clé de plus serait un contrat de plus à versionner. |
| **Élargir le domaine de `timeout_ms` pour accepter 0 et les grandes valeurs** | Changement de contrat sur un cas **documenté** (`docs/05-api.md:284`, ADR-0102, décision 3), donc interdit dans une mineure (ADR-0143, décision 1) ; et `0` ne veut pas dire « pas d'échéance » dans la conception. |

## Références

- Suivi : `docs/suivi.md:177` (`BORNER-DEFAULT-TIMEOUT`, classe « correction de comportement »), `:933` (relecture de `T06-LISTE-INDEX-AU-DEMARRAGE`, où l'idée est née), `:1560` et `:1562` (idée notée puis tâche créée sur délégation du propriétaire).
- Conception : `docs/07-stack-et-deploiement.md:106` (`default_timeout_ms = 2000`), `:97-99` (domaines de `ttl_days`, `max_rows` et `max_bytes`, modèles de commentaire), `:130` (séquence d'arrêt, borne `default_timeout_ms` + 1 000 ms, `wait_timeout_ms` et `stop_grace_period`), « Commande » (code de sortie non nul sur échec de chargement) ; `docs/05-api.md:284` (`timeout_ms`, 1 à 30 000), `:615` (« Limites par défaut (configurables) », ligne `timeout_ms`), `:619` (échéance de `timeout_ms` et `504`).
- ADR : ADR-0031 (une clé inconnue ne passe pas sans effet), ADR-0067 (`wait_timeout_ms`, `?wait=true`), ADR-0097 (surcharges `CRIBLE_*`, refus au démarrage), ADR-0099 (séquence d'arrêt, drainage), ADR-0102 (décision 3 : `timeout_ms` de 1 à 30 000, `504`, défaut `[limits].default_timeout_ms`), ADR-0104 (attente des jobs bornée par `wait_timeout_ms`, `stop_grace_period`), ADR-0122 (décision 8, `ttl_days = 0` refusé au démarrage), ADR-0127 (décision 1, `max_rows` et `max_bytes`), ADR-0143 (décision 1, correction de comportement ; décision 4, note de version).
- Code : `crates/crible-server/src/config.rs:420` (lecture de `default_timeout_ms`), `:802-815` (`milliseconds`, qui ne refuse que le négatif), `:362-388` (refus de `ttl_days`, `max_rows`, `max_bytes`), `:697-702` (`origin`, nom de la variable de surcharge) ; `crates/crible-server/src/limits.rs:42` et `:45` (`MIN_TIMEOUT_MS`, `MAX_TIMEOUT_MS`), `:70-72` (`timeout`, valeur du corps ou défaut), `:83-90` (`bounded`, refus et message) ; `crates/crible-server/src/search.rs:196-200` et `crates/crible-server/src/search_multi.rs:547` (les deux routes de recherche) ; `crates/crible-server/src/state.rs:243` (`default_timeout_ms()`) ; `crates/crible-server/src/lifecycle.rs:420` (`Config::load` avant le runtime et avant la liaison), `:44` et `:327` (borne du drainage).
- Application : `docs/07-stack-et-deploiement.md` (commentaire de domaine sur la ligne `default_timeout_ms`) et `docs/05-api.md` (section « Limites par défaut (configurables) »), dans le même changement que le code de la tâche `BORNER-DEFAULT-TIMEOUT` ; note de version de la 1.1.0 par `PUBLIER-V1.1.0`.
