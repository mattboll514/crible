# ADR-0099 — Cycle de vie du serveur : démarrage, `/health`, `/ready`, arrêt propre

- **Statut** : Accepté (accepté par anticipation par le propriétaire le 2026-10-29, à confirmer à la fin de la série DEMARRER-SERVEUR-AXUM, AUTHENTIFIER-CLES, FORMAT-ERREURS-ET-LIMITES)
- **Date** : 2026-10-29
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, tâche DEMARRER-SERVEUR-AXUM (phase P6), sur la demande du propriétaire
- **Remplace** : —

## Contexte

`docs/06-observabilite.md:113-114` fixe le sens des deux sondes : `/health` dit que le processus est vivant, `/ready` dit que les modèles sont chargés, que chaque index a publié un instantané et qu'aucune reprise n'est en cours. `docs/05-api.md:69` leur donne un rôle « — », donc aucune authentification. `docs/07-stack-et-deploiement.md`, section « Docker » (`:115` avant application du présent ADR) décrit un arrêt propre sur SIGTERM : arrêter d'accepter, terminer les jobs d'écriture, vider les analytics, sortir.

Ces phrases laissent ouvertes six questions que `DEMARRER-SERVEUR-AXUM` doit trancher avant d'écrire le squelette :

1. **l'ordre** : se lier au port avant ou après avoir chargé les modèles et ouvert les index ;
2. **les corps** de `/health` et `/ready`, et ce qu'ils ont le droit de révéler ;
3. **le mode d'ouverture des index** par le serveur, et donc ce que le démarrage a le droit d'écrire ;
4. **le sort d'un modèle manquant** au démarrage ;
5. **la place des tests qui demandent ONNX Runtime**, puisque la CI ordinaire n'en a pas (ADR-0057) ;
6. **la durée et la forme de l'arrêt**, y compris le sort des requêtes en vol et d'un second signal.

Le contexte de P6 ajoute une contrainte : le serveur est l'**écrivain** de chaque index (`docs/02-architecture.md`, « Concurrence » : un écrivain par index). Il n'y a pas encore de réplica, et aucune ouverture en lecture seule n'existe dans le chemin du serveur.

## Décision

### Démarrage

Le serveur se **lie d'abord au port**, puis charge les modèles et ouvre les index.

- `/health` rend `200` **dès la liaison**, tant que le processus vit, avec un corps JSON fixe. Il ne dépend ni des index, ni des modèles.
- `/ready` rend `503` **jusqu'à la publication du dernier instantané**, puis `200`. Les deux corps distinguent « en vie » et « prêt », et ne portent **aucun contenu d'index sensible** : pas de nom de champ, pas de chemin, pas de compte détaillé.
- Une instance **sans index** est prête.
- Seules les trois conditions de `docs/06` décident de `/ready` : modèles chargés, chaque index a publié son instantané, aucune reprise en cours. Un index dont le lexical est refusé ou indisponible (ADR-0087, ADR-0088, ADR-0089) **ne bloque pas** `/ready` : la génération est publiée, le mode `dense` est servi, et la branche lexicale renvoie `503` avec son message (ADR-0089).
- `/health` et `/ready` ne demandent **aucune authentification** (`docs/05`, colonne rôle « — »).

### Index

Le serveur ouvre chaque index de `<data>/indexes/*` **en écriture**, avec la reprise de `docs/02-architecture.md` : il est l'écrivain de chaque index. Conséquence assumée : le démarrage nettoie `segments/tmp-*`, répare le lexical (`ALTER TABLE meta`, reconstruction si nécessaire, ADR-0088), reconstruit les `codes.bin` manquants (ADR-0069) et purge `<data>/tmp/online-*` dont le verrou de présence est libre (ADR-0055, ADR-0056).

**Aucune ouverture en lecture seule en P6.** La question `open_read_only` — `GenerationStore::open_read_only` exécute encore `recover` et `ensure_manifest`, qui écrivent sous `epochs/` — reste à trancher par un ADR **avant tout réplica** (note « NETTOYER-P4 (`@architecte`, Q2) » de `docs/suivi.md`).

### Modèles

Le serveur charge une session par **empreinte** référencée par les index ouverts. Un index `kind: external` ne demande aucun modèle.

Si un index demande un modèle et qu'ONNX Runtime, le modèle ou son hash manque, le démarrage **échoue**, avec un message qui nomme le modèle, le fichier ou `ORT_DYLIB_PATH`, et un code de sortie non nul. Il n'y a pas de service partiel : une instance qui répondrait `200` sur `/ready` sans pouvoir encoder une requête serait pire qu'une instance absente.

Une instance sans index `onnx` démarre **sans** ONNX Runtime.

### Tests

Tout test qui lance le serveur sur un index `onnx` est `#[ignore]` et prouvé dans le job `rust-onnx` (ADR-0057). Les critères de `DEMARRER-SERVEUR-AXUM` se prouvent en CI ordinaire, sur un index `external` ou sans index.

### Arrêt

**SIGTERM et SIGINT** déclenchent la même séquence :

1. `/ready` passe à `503` ;
2. le serveur arrête d'accepter de nouvelles requêtes ;
3. il **draine les requêtes en vol** pendant au plus `[limits].default_timeout_ms` + 1 000 ms (`docs/07`, `default_timeout_ms` = 2 000 ms par défaut ; aucune clé de configuration nouvelle), puis sort en journalisant les requêtes coupées ;
4. il vide les analytics, **quand elles existeront** (P7) ;
5. l'attente des jobs d'écriture est fixée par `FILE-D-ECRITURE-ET-JOBS` (ADR-0095), qui crée la file ; `DEMARRER-SERVEUR-AXUM` n'a aucun job à attendre.

Un **second signal** pendant le drainage fait sortir **immédiatement**, avec un code non nul : c'est l'échappatoire de l'exploitant quand le drainage ne finit pas.

Ce qui n'est pas décidé ici : la durée d'attente des jobs d'écriture, fixée par `FILE-D-ECRITURE-ET-JOBS` ; le délai de vidage des analytics, fixé par P7 ; le comportement d'un déploiement à deux instances sur les mêmes données, qui n'existe pas en v1.

## Conséquences

- **Positives** :
  - l'orchestrateur de conteneurs distingue « le processus tourne » de « le service peut répondre », ce qui évite d'envoyer du trafic à une instance en cours de reprise ;
  - la liaison au port avant le chargement laisse une sonde répondre pendant une reprise longue, et le diagnostic devient possible ;
  - un modèle manquant produit un échec net, avec le nom du fichier cherché, au lieu d'une erreur à la première requête ;
  - les critères de la tâche sont prouvables en CI ordinaire, sans ONNX Runtime ;
  - le drainage est borné par une clé qui existe déjà : aucune configuration nouvelle.
- **Négatives / coûts** :
  - le serveur écrit dans les index au démarrage : un index monté en lecture seule ne peut pas être servi par cette version ;
  - une instance sans index `onnx` et une instance avec index `onnx` n'ont pas les mêmes prérequis d'image, ce qui doit être dit dans la documentation de déploiement ;
  - un arrêt qui attend jusqu'à 3 s retarde la sortie du conteneur ; un second signal reste nécessaire pour les cas bloqués ;
  - `/ready` ne dit pas **pourquoi** il n'est pas prêt : le diagnostic passe par les logs, par construction sans contenu sensible.
- **À surveiller** :
  - le temps de démarrage d'une instance réelle, avec la reprise de tous les index : s'il dépasse le délai de démarrage du conteneur, `/health` doit rester le seul critère de vivacité ;
  - un test d'arrêt qui laisse une requête en vol doit la voir se terminer, et un test à deux signaux doit voir une sortie immédiate non nulle ;
  - la question `open_read_only` doit être tranchée par un ADR **avant** le premier réplica, sinon deux instances écriront dans les mêmes époques.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Charger les modèles et les index avant de se lier au port | Une reprise longue laisse le conteneur sans aucune sonde joignable ; l'orchestrateur ne peut pas distinguer « en cours » de « mort ». |
| Un seul endpoint `/health` qui vaut « prêt » | Mélange vivacité et disponibilité : l'orchestrateur redémarrerait une instance simplement lente à reprendre. |
| Ouvrir les index en lecture seule en P6 | Le serveur est l'écrivain de chaque index, et `open_read_only` exécute encore `recover` et `ensure_manifest`, qui écrivent : la décision serait fausse sans un ADR dédié. |
| Démarrer en mode dégradé quand un modèle manque | Une instance qui accepte des requêtes sans pouvoir encoder renvoie des erreurs à chaque appel : le défaut serait découvert par les clients, pas au déploiement. |
| Drainer sans borne, jusqu'à la fin des requêtes | Un client qui garde une connexion ouverte empêcherait la sortie ; la borne est déjà une clé de configuration. |
| Attendre les jobs d'écriture dans cette tâche | Les jobs n'existent pas encore ; le critère et l'attente passent à `FILE-D-ECRITURE-ET-JOBS` (ADR-0095). |
| Arrêter immédiatement sur SIGTERM | Coupe des requêtes en vol et perd le travail du client, alors que le drainage coûte au plus quelques secondes. |

## Références

`docs/06-observabilite.md:113-114` (santé) ; `docs/05-api.md:69` (rôle « — » des deux sondes) ; `docs/07-stack-et-deploiement.md` (sections « Configuration de l'instance » pour `[limits].default_timeout_ms`, et « Docker » pour l'arrêt propre) ; `docs/02-architecture.md` (« Concurrence », « Reprise au démarrage ») ; `docs/suivi.md` (« Idées hors périmètre », `NETTOYER-P4`, `@architecte` Q2 sur `open_read_only`) ; ADR-0055, ADR-0056 (lots en ligne et verrou de présence) ; ADR-0057 (job `rust-onnx`) ; ADR-0069 (`codes.bin`) ; ADR-0087, ADR-0088, ADR-0089 (lexical) ; ADR-0095 (report du critère « jobs ») ; ADR-0096 (dépendances) ; ADR-0098 (`x-request-id`) ; fiche `DEMARRER-SERVEUR-AXUM`.
