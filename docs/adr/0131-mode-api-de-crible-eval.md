# ADR-0131 — Mode API de `crible eval`

- **Statut** : Accepté (par anticipation le 2026-11-02, sur délégation du propriétaire du 2026-11-02 ; confirmé par le propriétaire le 2026-11-03)
- **Date** : 2026-11-02
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, tâche EVALUER-PAR-API (phase P8), sur l'analyse de `@architecte` et les décisions retenues par l'orchestrateur, en application de la délégation du propriétaire du 2026-11-02
- **Remplace** : — (complète ADR-0125, décision 1, sans modifier le corps d'un ADR accepté)

## Contexte

### La fiche et ce qu'elle demande

La fiche `EVALUER-PAR-API` (`docs/phases/P8-docker-et-deploiement.md:124-146`) est une tâche hors plan ajoutée par le propriétaire le 2026-11-02, au rapport de J7. Elle fait passer `crible eval` « contre une instance en service, par `POST /v1/search` », avec « les **mêmes** métriques que le mode local (ADR-0125) : fidélité contre `exact`, qualité contre `expected`, latence » (`:126`). Le format d'entrée JSONL et le schéma de rapport restent ceux du mode local, pour que les rapports restent comparables. La latence est mesurée **côté client** et doit être distinguée de celle du mode local.

La fiche exige un **ADR accepté avant `@dev`** (`:128`), qui tranche quatre points : le client HTTP et son coût, TLS ou HTTP seul, le passage de la clé `read` sans fichier versionné ni journal, et les délais et erreurs réseau (`:129-132`). Son invariant est **INV-13** : aucun texte de requête et aucune clé dans les logs ni dans le rapport (`:142`, `:145`).

### Ce que les décisions voisines ont laissé ouvert

ADR-0125, décision 1, reporte explicitement le mode « contre une instance via l'API » : il demande un client HTTP absent de `docs/07`, donc une dépendance et un ADR (`docs/adr/0125-commande-eval-mode-local-format-metriques-et-rapport.md:46-55`). ADR-0124, décision 13, a refusé un client HTTP pour `crible stats`, en écrivant que « `docs/07:29-50` ne porte aucun client HTTP côté Rust, et en ajouter un pour lire un fichier local serait une dépendance sans emploi » (`docs/adr/0124-route-stats-forme-parametres-seaux-et-cli.md:195-206`).

Le tableau des dépendances de `docs/07-stack-et-deploiement.md:29-51` ne portait, avant cet ADR, **aucun client HTTP** : la ligne « HTTP » ne cite qu'`axum`, `tokio`, `tower` et `tower-http`, côté serveur. Le mode API doit donc ajouter une dépendance, et cette dépendance doit rester bornée à `crible eval` : elle ne sert ni `crible stats` (ADR-0124, décision 13), ni `crible-server`, ni `crible-store`.

ADR-0032, décision 1, impose que les accès vivent hors git et que les documents, rapports, ADR et commits ne citent que les **noms** des variables, jamais leurs valeurs (`docs/adr/0032-acces-a-la-production.md:19-22`). ADR-0097 tient une **liste fermée** des variables `CRIBLE_*` documentées hors configuration, et refuse au démarrage toute autre variable de ce préfixe (`docs/adr/0097-configuration-de-l-instance-surcharges-crible-et-options-de-serve.md:35-47`). Une variable nouvelle employée par `crible eval` doit donc y entrer, sinon `crible serve` refuserait de démarrer quand elle traîne dans son environnement.

ADR-0129, décision 10, résout Q-07 (section « Questions ouvertes » de `docs/adr/README.md`) : la latence contraignante de la v1 se mesure sur un **Mac mini M1 16 Go, en conteneur**, et la production officielle n'est pas actée. Crible tourne donc sur un réseau interne, sans port publié vers l'extérieur.

### Les faits vérifiés aux sources

- La route simple de recherche est `POST /v1/indexes/{index}/search` ; `POST /v1/search` est la route **multi-index** (`docs/05-api.md:70-71`).
- La réponse multi-index ne porte pas de `generation` scalaire : elle porte `targets: [{ "index", "generation", "pool_size" }]` (`docs/05-api.md:384` ; ADR-0110, décision 1).
- Le champ `exact` est à `docs/05-api.md:273`, et `timeout_ms` à `:276`, avec un maximum de **30 000 ms** (`1 à 30000`).
- Le chapitre « Éval : `crible eval` » de `docs/06-observabilite.md` est aux lignes **148-179**. La fiche cite `:124-144`, référence périmée : ces lignes décrivent `explain_ids`, pas `eval`.
- Le décodeur base64 existe déjà : `pub fn decode_base64(text: &str) -> Result<Vec<u8>, String>` dans `crates/crible-store/src/online.rs:1814`. Aucun encodeur n'existe à côté.
- `GET /v1/indexes/{index}` rend la configuration et l'état de la génération publiée, dont `state.generation` (`docs/05-api.md:62`, `:102-127`).
- `hyper` 1.12.0, `hyper-util` 0.1.21 et `http-body-util` 0.1.5 sont déjà dans `Cargo.lock`, tirés par `axum`. En revanche, l'activation de `client-legacy` fait entrer dans le `Cargo.lock` trois paquets transitifs qui n'y figurent pas au 2026-11-02 : `want` et `try-lock` (feature `client` de `hyper`) et `futures-channel` (feature `client` de `hyper-util`). Aucune crate **directe** de plus n'est nécessaire.

## Décision

### Décision 1 — Le mode se choisit par `--url`, et vise la route simple

**Décision.** Le mode API de `crible eval` se choisit par `--url <base>` et vise `POST /v1/indexes/{index}/search`. Sans `--url`, le mode local est **inchangé**.

Précisions :

- la route simple a la même forme que le mode local, qui évalue **un seul index** ; c'est elle qui porte `generation` et `pool_size` au premier niveau ;
- la fiche écrit `POST /v1/search` (route multi-index) ; c'est un **écart**, assumé : la réponse multi-index n'a pas de `generation` scalaire (`docs/05-api.md:384`), et `crible eval` épingle la génération de l'index qu'il évalue ;
- `--url` prend une base telle que `http://hôte:port` ; le chemin de la route est construit par la commande ;
- le mode local garde son comportement, son format d'entrée et son rapport, à l'exception du `format_version` de la décision 7.

### Décision 2 — Client HTTP : `hyper`, `hyper-util` et `http-body-util`, dans la crate `crible` seulement

**Décision.** Le client HTTP est écrit avec `hyper` + `hyper-util` (features `client-legacy` et `http1`) + `http-body-util`, dans un **module privé de la crate `crible` seulement**, sur un runtime tokio `current_thread` créé par `eval`. Aucune crate **directe** nouvelle : ces trois crates sont déjà dans `Cargo.lock`, tirées par `axum`.

Précisions :

- l'activation de `client-legacy` ajoute au `Cargo.lock` trois paquets transitifs — `want`, `try-lock` et `futures-channel` —, sans TLS et sans client alternatif ; aucune crate **directe** de plus n'est nécessaire ;
- le client ne sert **ni** `crible stats` (ADR-0124, décision 13, reste entier), **ni** `crible-server`, **ni** `crible-store` ; il vit dans `crates/crible/`, à côté de `eval` ;
- le refus d'ADR-0124, décision 13, ne s'applique pas : là-bas HTTP était inutile (lecture d'un fichier local), ici il est le **sujet** de la tâche ;
- le runtime `current_thread` suffit : les requêtes sont séquentielles (décision 10), et la commande n'a pas besoin d'un pool multi-thread ;
- aucune autre crate HTTP n'entre dans le binaire.

### Décision 3 — HTTP/1.1 en clair seulement, refus explicites

**Décision.** Le client parle **HTTP/1.1 en clair**. Une URL `https://` est refusée **avant toute entrée-sortie**, en `invalid_request`, avec un message qui nomme le schéma et dit que TLS demanderait un ADR.

Précisions :

- Crible tourne sur un réseau interne sans port publié (ADR-0129, décision 10) ; TLS n'est pas nécessaire à la cible de P8 ;
- sont aussi refusées, **par leur nom**, sans retrait silencieux : l'information d'utilisateur (`user:pass@`), une chaîne de requête, un fragment, et un chemin autre que `/` ;
- un refus nomme le composant fautif ; il ne le supprime pas pour continuer ;
- tout autre schéma que `http` est refusé de la même façon.

### Décision 4 — La clé `read` vient de `CRIBLE_EVAL_API_KEY`, jamais d'une option ni d'un fichier

**Décision.** La clé `read` est lue **une fois** dans la variable d'environnement `CRIBLE_EVAL_API_KEY`. Elle n'est **jamais** journalisée, affichée, ni écrite dans le rapport ou dans un message d'erreur.

Précisions :

- ni option `--api-key` (visible dans `ps` et l'historique du shell), ni fichier de clé : la variable d'environnement est le seul chemin ;
- la clé est envoyée dans l'en-tête `Authorization: Bearer`, et nulle part ailleurs ;
- le message d'erreur et le rapport ne portent que `http://hôte:port`, sans la clé ;
- **complément à ADR-0097** : la liste fermée des variables `CRIBLE_*` gagne `CRIBLE_EVAL_API_KEY`, pour que `crible serve` ne refuse pas de démarrer quand elle traîne dans son environnement ;
- une variable absente ou vide rend une erreur claire qui nomme `CRIBLE_EVAL_API_KEY`, sans valeur ;
- INV-13 et ADR-0032 s'appliquent : aucun texte de requête, aucune valeur de filtre, aucune clé dans les logs ni dans le rapport.

### Décision 5 — Délais fixes et erreurs nommées

**Décision.** Les délais sont **fixes, sans option** : connexion **5 s**, requête entière **35 s**. Le délai de 35 s est au-dessus du maximum de `timeout_ms` (30 s, `docs/05-api.md:276`), pour laisser au serveur le temps de rendre son `504`.

Précisions :

- statut non 2xx : le code et le message du **corps d'erreur du serveur** sont repris, avec `request_id` (`docs/05-api.md:25-35`) ;
- délai client dépassé : code `timeout` ;
- erreur de transport (DNS, connexion refusée, réponse tronquée ou illisible) : code `internal`, avec un message **côté CLI** qui nomme l'URL (sans clé) et la cause ;
- **aucun `ErrorCode` nouveau** : les codes sont ceux de `docs/05` ;
- code de sortie **1** à la première erreur, en nommant l'`id` de la requête du jeu (**jamais son texte**) ;
- aucun rapport partiel, aucune reprise, aucun réessai automatique.

### Décision 6 — Ligne de commande

**Décision.** `--data` et `--url` sont **exclusifs** : `--data` est requis sans `--url`. `--index` est requis dans les deux modes. En mode API, `--models`, `--exact-threshold` et `--rescore-candidates` sont refusés en `invalid_request`, avec la raison.

Précisions :

- `--models`, `--exact-threshold` et `--rescore-candidates` ne sont pas des champs de requête : le serveur encode lui-même, et `exact_threshold` comme N sont des préconditions de l'instance (décision 9) ;
- `--queries`, `--modes`, `--k`, `--repeat`, `--out` et `--baseline` gardent la même sémantique qu'en local ;
- un refus nomme l'option fautive et dit pourquoi elle n'a pas de sens en mode API ;
- le mode local ne change pas : ses surcharges restent en mémoire et sans écriture (ADR-0125, décision 7).

### Décision 7 — Rapport `format_version: 2` et latences distinguées

**Décision.** Le rapport passe à `format_version: 2`. À la racine, il porte `source: { "mode": "local" | "api", "url"? }`. Par mode, `latency` garde sa forme, avec un membre `latency_basis` valant `"server_stages"` en local et `"client_round_trip"` en API.

Précisions :

- en API, `latency` ne porte que `total`, l'aller-retour HTTP mesuré **côté client** ;
- le lecteur de `--baseline` accepte les versions **1 et 2** ;
- qualité et fidélité se comparent **toujours** ; les latences ne se comparent que si `latency_basis` est identique, sinon elles sont dites « non comparables », sans erreur et sans changement du code de sortie ;
- le Markdown nomme la section « **Latence côté client (aller-retour HTTP)** » en mode API ;
- `url` n'est écrite que si le rapport vient du mode API, et sans la clé (décision 4) ;
- le reste du schéma fermé d'ADR-0125, décision 11, est conservé.

### Décision 8 — Requêtes envoyées et encodage du `vector`

**Décision.** La fidélité se mesure par une **seconde requête** `exact: true` à `k = 10`, même filtre et même mode ; la qualité par une requête **groupée** (`group_by: "document"`, `passages_per_document: 1`), comme le mode local. Chaque requête porte `return: { "fields": [], "text": false }`. Elle ne porte **pas** `debug: true`.

Précisions :

- `debug: true` écrirait le journal de debug du serveur (ADR-0121, ADR-0122) ; il est donc absent, et le rapport API ne porte **ni étapes serveur ni `dense_strategy`** ;
- le `vector` du jeu (tableau de nombres) est converti en **base64 fp32 little-endian** par un encodeur écrit à la main, à côté du décodeur existant `crible_store::online::decode_base64` (`crates/crible-store/src/online.rs:1814`), **sans crate** ;
- le corps de requête est celui de `docs/05-api.md:259-282` : mêmes bornes, mêmes règles d'entrée selon le mode (ADR-0109) ;
- aucun champ inconnu n'est envoyé : le serveur les refuse (ADR-0109) ;
- les métriques de fidélité et de qualité sont celles d'ADR-0125, décisions 4 à 6, sans duplication de calcul.

### Décision 9 — `exact_threshold` et N sont des préconditions de l'instance

**Décision.** `exact_threshold` et N (rescoring) ne sont pas des surcharges du mode API : ce sont des **préconditions de l'instance** évaluée. Le Markdown les rappelle ; le JSON ne les porte pas.

Précisions :

- le mode API ne peut pas modifier la configuration du serveur : `--exact-threshold` et `--rescore-candidates` sont refusés (décision 6) ;
- le rapport **dit** les valeurs attendues de l'instance et renvoie au `PATCH` d'administration ou à la configuration pour les changer ;
- un rapport API ne prétend donc pas avoir forcé BINARY ; la fidélité se lit contre `exact: true`, comme en local, mais à la stratégie de l'instance ;
- le JSON ne porte pas ces deux valeurs : elles ne sont pas des paramètres de la campagne, et les y mettre laisserait croire qu'elles ont été appliquées.

### Décision 10 — Génération épinglée, requêtes séquentielles, preuve de bout en bout

**Décision.** La génération est **épinglée au départ** par `GET /v1/indexes/{index}` (`state.generation`). Si une réponse porte une autre génération, la commande **s'arrête** en nommant les deux générations. Les requêtes sont **séquentielles**, une à la fois.

Précisions :

- une campagne porte donc sur une seule génération, et le rapport le dit ;
- l'arrêt nomme la génération épinglée et celle reçue, sans texte de requête ;
- le test de bout en bout tourne contre un **serveur lancé par le test**, avec `exact_threshold = 0` ;
- ses chiffres de **fidélité** et de **qualité** égalent ceux d'un `eval` local `--exact-threshold 0` sur le même dossier ;
- la latence n'est pas comparée dans ce test : les deux modes n'ont pas le même `latency_basis`.

## Conséquences

- **Positives** :
  - `crible eval` peut évaluer une instance en service, sans copie du dossier de données ;
  - les métriques de fidélité et de qualité restent celles d'ADR-0125 : les rapports des deux modes restent comparables ;
  - aucune crate **directe** nouvelle, et le client reste confiné à la crate `crible` ;
  - la clé ne passe ni par un fichier, ni par la ligne de commande, ni par un log ;
  - la latence est nommée pour ce qu'elle est : un aller-retour HTTP côté client, distinct des étapes serveur ;
  - les erreurs réseau rendent un code connu et un code de sortie non nul, sans panique.
- **Négatives / coûts** :
  - trois paquets transitifs entrent dans `Cargo.lock` (`want`, `try-lock`, `futures-channel`), sans TLS ; le coût reste faible ;
  - pas de TLS : une instance exposée hors du réseau interne ne peut pas être évaluée sans un nouvel ADR ;
  - le rapport API perd les latences par étape et `dense_strategy`, faute de `debug: true` ;
  - les requêtes séquentielles allongent une campagne sur un jeu de plusieurs centaines de requêtes ;
  - les délais fixes ne se règlent pas : une instance plus lente que 35 s par requête rend `timeout` ;
  - `format_version: 2` demande au lecteur de `--baseline` d'accepter deux versions.
- **À surveiller** :
  - un besoin de TLS, si la cible de déploiement change : il demandera un ADR, jamais une option cachée ;
  - la liste fermée d'ADR-0097 et la variable `CRIBLE_EVAL_API_KEY` doivent rester alignées ;
  - une génération qui change pendant une campagne fait échouer la commande : c'est voulu, mais un serveur en écriture permanente rendra le mode API inutilisable ;
  - la comparabilité des latences entre rapports : un `latency_basis` différent doit rester dit « non comparable » ;
  - le coût réel de `client-legacy` dans `Cargo.lock`, si `hyper` ou `hyper-util` changent de features.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| `reqwest` | Arbre neuf et TLS, pour un client qui sert une seule commande sur un réseau interne ; la dépendance serait bien plus lourde que `hyper-util` |
| `ureq` | Arbre neuf, TLS, et un client synchrone qui n'utilise pas les crates déjà verrouillées par `axum` |
| Écrire le client à la main sur `TcpStream` | Code HTTP à maintenir (analyse de la réponse, en-têtes, corps, erreurs) pour un gain nul : `hyper` est déjà là |
| Viser `POST /v1/search` comme la fiche l'écrit | La réponse multi-index n'a pas de `generation` scalaire (`docs/05:384`), et le mode local évalue un seul index |
| Accepter `--api-key` | La clé serait visible dans `ps` et dans l'historique du shell, contrairement à ADR-0032 |
| Lire la clé dans un fichier | Un fichier de secret sur disque se versionne ou fuite plus facilement qu'une variable d'environnement ; ADR-0032 place les accès hors git |
| Accepter `https://` en v1 | TLS demanderait une dépendance et une configuration de certificats, pour une cible qui n'est pas exposée hors du réseau interne |
| Retirer silencieusement `user:pass@`, la requête, le fragment ou un chemin | Un retrait silencieux ferait croire que l'URL a été respectée ; le refus nommé est plus sûr |
| Supporter les surcharges `--exact-threshold` et `--rescore-candidates` par requête | Ce ne sont pas des champs de la route ; le serveur ne les lit pas, et le mode local les applique en mémoire seulement |
| Poser `debug: true` pour obtenir les étapes serveur | Cela écrirait le journal de debug du serveur (ADR-0121, ADR-0122) pour une campagne, et ferait entrer un contenu de diagnostic dans l'instance évaluée |
| Garder `format_version: 1` et ajouter `latency_basis` sans changer de version | Un lecteur v1 ne connaît pas le membre ; le changement de version rend la distinction explicite |
| Comparer les latences des deux modes | Ce ne sont pas les mêmes mesures : étapes serveur d'un côté, aller-retour HTTP de l'autre |
| Réessayer ou reprendre une requête en échec | Une campagne est une mesure datée ; une reprise partielle rendrait les chiffres ambigus, et la fiche exclut un démon |
| Paralléliser les requêtes | Le serveur a un sémaphore de recherches concurrentes (ADR-0102) ; la latence côté client serait bruitée par la file |
| Étendre le client à `crible stats` | ADR-0124, décision 13, a refusé ce mode : la commande lit un fichier local, et le refus reste entier |

## Références

- Fiche : `docs/phases/P8-docker-et-deploiement.md:124-146` (`EVALUER-PAR-API`).
- Conception : `docs/05-api.md:25-35` (corps d'erreur), `:62` et `:102-127` (`GET /v1/indexes/{index}`, `state.generation`), `:70-71` (routes de recherche), `:259-282` (« Recherche : requête », `exact` à `:273`, `timeout_ms` à `:276`), `:384` (réponse multi-index) ; `docs/06-observabilite.md:148-179` (« Éval ») ; `docs/07-stack-et-deploiement.md:29-51` (dépendances Rust).
- ADR : ADR-0032 (accès et secrets), ADR-0097 (liste fermée des `CRIBLE_*`), ADR-0102 (`timeout_ms` et `504`), ADR-0109 (entrées selon le mode), ADR-0110 (réponse multi-index), ADR-0116 (lecture à côté d'un serveur), ADR-0121 et ADR-0122 (journal de debug), ADR-0124 (décision 13, pas de client HTTP pour `crible stats`), ADR-0125 (mode local, métriques et rapport, décision 1), ADR-0129 (décision 10, Q-07).
- Code : `crates/crible-store/src/online.rs:1814` (`decode_base64`), `crates/crible/src/eval.rs` (mode local).
- Dépendances : `Cargo.lock` (`hyper` 1.12.0, `hyper-util` 0.1.21, `http-body-util` 0.1.5 ; absence de `want`, `try-lock` et `futures-channel` au 2026-11-02).
