# ADR-0104 — Arrêt du serveur et jobs d'écriture

- **Statut** : Accepté (accepté par anticipation par le propriétaire le 2026-10-29, à confirmer à la revue de fin de série)
- **Date** : 2026-10-29
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, tâche FILE-D-ECRITURE-ET-JOBS (phase P6), sur demande de l'orchestrateur et le verdict de `@architecte` du 2026-10-29
- **Remplace** : —

## Contexte

ADR-0099 fixe la séquence d'arrêt du serveur sur SIGTERM et SIGINT : `/ready` passe à `503`, le serveur arrête d'accepter, il draine les requêtes en vol pendant au plus `[limits].default_timeout_ms` + 1 000 ms, il vide les analytics quand elles existent (P7), puis il sort. Mais son point 5 (`:62`) laisse l'attente des jobs d'écriture « fixée par `FILE-D-ECRITURE-ET-JOBS` », et « Ce qui n'est pas décidé ici » (`:66`) répète que « la durée d'attente des jobs d'écriture » reste ouverte. Dans la même section, l'alternative « Attendre les jobs d'écriture dans cette tâche » est écartée parce que les jobs n'existaient pas encore (`:95`).

La fiche `FILE-D-ECRITURE-ET-JOBS` reprend le critère d'ADR-0095 (`docs/phases/P6-serveur-et-api.md:66`) : « un test d'arrêt montre qu'aucun job en cours n'est perdu : les jobs acceptés avant le signal se terminent, et l'attente bornée de l'arrêt les couvre ». Le critère nomme trois choses et n'en définit aucune : la **borne** de l'attente, le **sort des jobs encore en file** au signal, et le **statut des nouvelles soumissions** pendant le drainage.

Trois points de conception pèsent. La file et la table des jobs sont posées par ADR-0103, avec le `Retry-After: 1` posé au rendu de toute réponse `overloaded` (`:9`). `?wait=true` rend `202` avec le job dans son état courant à l'échéance de `wait_timeout_ms`, et n'est jamais annulé (ADR-0067, `docs/05:20`). Les données d'un index restent cohérentes à toute coupure par la reprise (INV-02, INV-03, `docs/02-architecture.md:138-148`).

## Décision

**Décision 1 — refus dès le drainage.** Dès le début du drainage — la même transition que le passage de `/ready` à `503` —, toute **nouvelle** soumission de job est refusée en `503 overloaded`, avec le message « the server is shutting down » et `Retry-After: 1` (ADR-0103, décision 9).

**Décision 2 — les jobs acceptés se terminent.** Les jobs acceptés **avant** le signal, en cours ou en attente, se terminent : chaque écrivain vide sa file.

**Décision 3 — borne de l'attente.** L'attente des jobs est bornée par **`[limits].wait_timeout_ms`** (60 000 ms par défaut), comptée **depuis le signal**, et court **en parallèle** du drainage des requêtes d'ADR-0099. Aucune clé de configuration n'est ajoutée. Le processus sort quand les deux drainages sont finis, ou à l'échéance de chacun.

**Décision 4 — à l'échéance.** Le processus sort sans attendre. Les jobs non terminés restent `queued` ou `running` dans la table et passent à `failed` au démarrage suivant (ADR-0103, décision 5). Le journal nomme alors, au niveau `warn`, le nombre de jobs abandonnés et, pour chacun, `job_id`, `kind` et `index` : aucun texte client. Le code de sortie est celui qu'ADR-0099 donne à une coupure du drainage des requêtes.

**Décision 5 — `?wait=true` pendant le drainage.** Une requête `?wait=true` en attente au début du drainage rend **tout de suite** `202` avec le job dans son état courant (ADR-0067).

**Décision 6 — second signal.** Un second signal pendant le drainage fait **toujours** sortir immédiatement, avec un code non nul (ADR-0099, inchangé).

**Conséquence d'exploitation, à écrire dans `docs/07`.** L'exploitant règle le délai d'arrêt du conteneur (`stop_grace_period`, P8) **au-dessus** de `wait_timeout_ms`, faute de quoi l'orchestrateur tue le processus avant la fin de l'attente. Les données restent cohérentes à toute coupure : la reprise rattrape l'état de l'index (INV-02, INV-03), et la table des jobs corrige les états transitoires au démarrage (ADR-0103).

ADR-0099 n'est **pas modifié** : il est accepté, et seul un renvoi est ajouté dans `docs/05` et `docs/07`.

Ce qui n'est décidé ici : le délai de vidage des analytics, qui appartient à P7 ; le réglage exact de `stop_grace_period`, qui est une valeur de déploiement de P8 ; la conduite à tenir si un job bloqué ne rend jamais la main, que seul le second signal couvre.

## Conséquences

- **Positives** :
  - le critère de la fiche est satisfait sans clé nouvelle : la borne existe déjà pour `?wait=true` ;
  - le client voit immédiatement qu'une soumission est refusée, avec une valeur de `Retry-After` exploitable, au lieu d'un job accepté qui ne démarrerait jamais ;
  - les deux drainages courent ensemble, donc l'arrêt coûte au plus la plus longue des deux bornes, et non leur somme ;
  - un job abandonné est signalé dans le journal avec son identité technique, ce qui rend l'incident diagnosticable sans contenu client ;
  - un seul mécanisme corrige les états transitoires, celui du démarrage : rien n'est marqué `failed` à la sortie, donc une sortie brutale et une sortie propre laissent le même état sur disque.
- **Négatives / coûts** :
  - l'arrêt peut attendre jusqu'à 60 s par défaut, ce qui est long pour un orchestrateur de conteneurs et impose un `stop_grace_period` réglé ;
  - un job d'écriture long (une attache de plusieurs centaines de milliers de passages) peut dépasser la borne et être abandonné : son travail est repris par la reprise, mais son client doit resoumettre ;
  - réutiliser `wait_timeout_ms` lie deux besoins distincts : une instance qui voudrait attendre longtemps un job à l'arrêt allonge aussi l'attente d'un `?wait=true` ;
  - les jobs abandonnés restent visibles en `queued` ou `running` jusqu'au démarrage suivant, ce qui trompe un client qui interroge une instance encore vivante mais déjà sortante.
- **À surveiller** :
  - un test d'arrêt qui soumet des jobs, envoie SIGTERM pendant qu'ils tournent et vérifie qu'ils se terminent tous avant la sortie ;
  - un test qui soumet un job après le début du drainage et vérifie `503 overloaded`, le message « the server is shutting down » et `Retry-After: 1` ;
  - un test qui fait expirer l'attente et vérifie que le journal nomme les jobs abandonnés, sans texte client, et que le démarrage suivant les passe à `failed` ;
  - la durée d'arrêt mesurée en P8 dans le conteneur : si elle dépasse régulièrement quelques secondes, la borne ou le réglage de `stop_grace_period` sont à revoir ;
  - la cohérence de `docs/07` : la phrase d'ADR-0099 sur l'attente « fixée par `FILE-D-ECRITURE-ET-JOBS` » doit renvoyer à ADR-0104.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Attendre les jobs sans borne | Une attache longue ou un job bloqué empêcherait la sortie ; l'orchestrateur finirait par tuer le processus, sans que le journal dise pourquoi. |
| Une clé `[server].shutdown_timeout_ms` | Ajoute une clé à une liste fermée (ADR-0097) pour une valeur qu'une clé existante couvre ; la règle 12 des `AGENTS.md` interdit la configuration d'une valeur fixe. |
| Marquer `failed` avant la sortie les jobs restés en file | Crée un second mécanisme, qui devrait deviner quels jobs ont été abandonnés et écrire pendant un arrêt pressé ; le démarrage le fait déjà, une seule fois, sur un état stable. |
| Refuser aussi les jobs en attente au signal | Contredit le critère de la fiche : les jobs acceptés avant le signal se terminent. |
| Répondre `503` aux `?wait=true` en cours au début du drainage | Le job existe et continue : le client doit recevoir son état courant, en `202`, et non une erreur qui lui ferait croire à un échec (ADR-0067). |
| Faire durer l'attente des jobs `default_timeout_ms` + 1 000 ms, comme les requêtes | Cette borne couvre une requête de recherche ; une attache dure bien plus, et le critère de la fiche porte précisément sur les jobs. |
| Sortir immédiatement sur SIGTERM quand des jobs tournent | Perd le travail accepté, ce que le critère de la fiche interdit. |

## Références

`docs/phases/P6-serveur-et-api.md` (`FILE-D-ECRITURE-ET-JOBS` `:54-69`, `DEMARRER-SERVEUR-AXUM` `:27`) ; `docs/05-api.md` (`?wait=true` `:19-20`, endpoint des jobs `:67`, `wait_timeout_ms` `:279`) ; `docs/07-stack-et-deploiement.md` (section « Docker » `:112-123`, `[limits]` `:97-102`) ; `docs/02-architecture.md` (« Reprise au démarrage » `:138-148`, « Concurrence » `:158-167`) ; `docs/08-invariants-et-tests.md` (INV-02, INV-03) ; ADR-0038 (verrou SQLite, `overloaded`) ; ADR-0055 (idempotence par `batch_id`) ; ADR-0067 (`?wait=true`, `202` à l'échéance) ; ADR-0095 (critère d'arrêt repris par la fiche) ; ADR-0097 (liste fermée des clés) ; ADR-0099 (séquence d'arrêt, point 5 et « Ce qui n'est pas décidé ici ») ; ADR-0103 (table des jobs, `Retry-After`).
