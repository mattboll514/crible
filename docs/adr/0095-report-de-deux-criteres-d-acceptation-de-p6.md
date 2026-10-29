# ADR-0095 — Report de deux critères d'acceptation de P6

- **Statut** : Accepté (accepté par anticipation par le propriétaire le 2026-10-29, à confirmer à la fin de la série DEMARRER-SERVEUR-AXUM, AUTHENTIFIER-CLES, FORMAT-ERREURS-ET-LIMITES)
- **Date** : 2026-10-29
- **Décideurs** : propriétaire (décision du 2026-10-29)
- **Proposé par** : agent greffier, tâche DEMARRER-SERVEUR-AXUM (phase P6), sur la demande du propriétaire
- **Remplace** : —

## Contexte

Deux critères d'acceptation de la phase P6 demandent une fonctionnalité qui n'existe pas encore au moment où leur tâche s'exécute.

1. **Fiche `DEMARRER-SERVEUR-AXUM`** (`docs/phases/P6-serveur-et-api.md`), critère : « un test d'arrêt montre qu'aucun job en cours n'est perdu ». Or les jobs d'écriture n'existent qu'à partir de `FILE-D-ECRITURE-ET-JOBS`, qui dépend de cette tâche. À son terme, le serveur n'a aucune file d'écriture à drainer : le critère est invérifiable.
2. **Fiche `AUTHENTIFIER-CLES`**, critère : « `403` si le rôle est insuffisant, avec un test par famille de routes ». Or à ce moment-là seules `/health` et `/ready` existent, et ces deux routes ne demandent aucune authentification (colonne « rôle » à « — » dans le tableau des routes de `docs/05-api.md:50-70`). Aucune route protégée n'existe donc : le critère est invérifiable.

Les deux critères restent utiles. Il s'agit de les placer sur la tâche qui peut les prouver, pas de les abandonner.

## Décision

**Décision 1 — arrêt et jobs.** Le critère « aucun job en cours n'est perdu à l'arrêt » passe à `FILE-D-ECRITURE-ET-JOBS`, qui crée la file et les jobs. `DEMARRER-SERVEUR-AXUM` le remplace par : « un test d'arrêt montre qu'une requête en vol se termine avant la sortie (drainage borné, ADR-0099) ».

**Décision 2 — `403` par famille de routes.** `AUTHENTIFIER-CLES` teste le middleware et les trois rôles (`read`, `write`, `admin`) sur un routeur de test, monté dans la tâche avec des routes factices. Le test `403` d'une famille de routes réelles est porté par la tâche qui crée cette famille, à partir du tableau des routes de `docs/05-api.md` :

| Famille de routes | Rôle minimal | Tâche qui la crée |
|---|---|---|
| `/v1/indexes` : `POST`, `GET` (liste et détail), `PATCH`, `DELETE` | `admin` ; les deux `GET` demandent `read` | `ROUTES-INDEX` (P6) |
| `/v1/indexes/{index}/documents` : `POST`, `PATCH`, `GET` | `write` ; le `GET` demande `read` | `ROUTES-DOCUMENTS-ET-LOTS` (P6) |
| `/v1/indexes/{index}/batches` : `POST` | `write` | `ROUTES-DOCUMENTS-ET-LOTS` (P6) |
| `/v1/jobs` : `GET` (liste et détail) | `write` | `FILE-D-ECRITURE-ET-JOBS` (P6) |
| `/v1/indexes/{index}/search` : `POST` | `read` | `ROUTE-RECHERCHE` (P6) |
| `/v1/search` : `POST` | `read` | `RECHERCHE-MULTI-INDEX` (P6) |
| `/v1/indexes/{index}/compact`, `/exports`, `/snapshots` : `POST` | `admin` | `ROUTES-DOCUMENTS-ET-LOTS` (P6) |
| `/v1/stats` : `GET` | `admin` | `ROUTE-STATS` (P7) |
| `/v1/debug/queries` : `GET` | `admin` | `JOURNAL-DE-DEBUG` (P7) |
| `/health`, `/ready`, `/openapi.json`, `/docs` | aucun (colonne « — ») | `DEMARRER-SERVEUR-AXUM` et `PUBLIER-OPENAPI` (P6) ; aucun test `403` |

Chaque tâche listée prouve le contrôle de rôle de sa famille. Pour une famille dont le rôle minimal est `admin` ou `write`, le test vérifie qu'une clé d'un rôle inférieur reçoit `403 forbidden`. Pour une famille dont le rôle minimal est `read` (`/v1/…/search` et `/v1/search`), aucun `403` n'est atteignable : `write` et `admin` incluent `read`, donc tout porteur d'une clé valide a ce droit. Le test y vérifie l'acceptation par les trois rôles, et le refus `401` d'une clé absente ou inconnue reste couvert par `AUTHENTIFIER-CLES`. Les routes sans rôle ne reçoivent aucun test `403` ; elles restent accessibles sans clé.

Ce qui n'est pas décidé ici : ni la forme du middleware, ni le hachage des clés, ni le contenu du message d'erreur, qui relèvent de la fiche et de `docs/05`.

## Conséquences

- **Positives** :
  - chaque critère est prouvé par la tâche qui dispose du code correspondant ;
  - le drainage des requêtes en vol est testé dès le squelette du serveur, ce qui donne tôt le comportement d'arrêt décrit par ADR-0099 ;
  - la couverture `403` est répartie et nommée, au lieu d'être concentrée sur une tâche qui n'a aucune route à protéger.
- **Négatives / coûts** :
  - cinq fiches de P6 et une de P7 gagnent un critère, donc un test de plus à écrire ;
  - `AUTHENTIFIER-CLES` teste son middleware sur un routeur factice : ce test ne prouve pas l'application réelle sur les routes de production, qui dépend des tests par famille.
- **À surveiller** : si une famille de routes est ajoutée sans son test de contrôle de rôle, le tableau ci-dessus est incomplet. La revue de `PUBLIER-OPENAPI`, qui compare la liste des routes de la spécification à celle de `docs/05`, est le point de contrôle naturel.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Laisser les critères où ils sont et marquer les tâches `BLOQUE` | Bloque la phase sur une impossibilité de plan, alors que le critère reste vrai plus tard : le report le préserve. |
| Supprimer les deux critères | Les deux comportements sont nécessaires (arrêt sans perte, refus par rôle) ; les supprimer perdrait la preuve. |
| Créer d'avance dans `AUTHENTIFIER-CLES` des routes factices de production | Ferait entrer des routes sans implémentation dans le binaire, et la fiche ne le demande pas. |
| Faire dépendre `AUTHENTIFIER-CLES` de toutes les tâches de routes | Inverse l'ordre du plan et retarde l'authentification, dont toutes les autres tâches dépendent. |

## Références

`docs/phases/P6-serveur-et-api.md` (fiches `DEMARRER-SERVEUR-AXUM`, `AUTHENTIFIER-CLES`, `FILE-D-ECRITURE-ET-JOBS`, `ROUTES-INDEX`, `ROUTES-DOCUMENTS-ET-LOTS`, `ROUTE-RECHERCHE`, `RECHERCHE-MULTI-INDEX`) ; `docs/phases/P7-observabilite.md` (`ROUTE-STATS`, `JOURNAL-DE-DEBUG`) ; `docs/05-api.md` (tableau des routes et colonne rôle) ; ADR-0099 (arrêt propre et drainage) ; ADR-0023 (orchestration).
