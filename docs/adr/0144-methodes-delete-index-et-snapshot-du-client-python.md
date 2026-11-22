# ADR-0144 — Méthodes `delete_index` et `snapshot` du client Python

- **Statut** : Accepté (accepté par anticipation par le propriétaire le 2026-11-11, confirmé par le propriétaire le 2026-11-11)
- **Date** : 2026-11-11
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, tâche T01-CLIENT-PYTHON-DELETE-SNAPSHOT (ligne `v1.1`), sur le verdict de `@architecte` du 2026-11-11 et la décision du propriétaire du même jour (« Si tu as des ADR à écrire tu peux les accepter par anticipation, on les regarde ensemble après »)
- **Remplace** : — (ne remplace aucune décision : complète ADR-0112, sans modifier son corps, et applique ADR-0105, décision 5)

## Contexte

### La demande

Le ticket 01 du dossier de tickets de Legifacile (`../legifacile/tickets-crible/01-client-python-delete-index-et-snapshot.md`, lecture seule) constate que le client Python livré avec Crible 1.0.0 n'expose ni la suppression d'un index ni la prise d'un instantané, alors que les deux routes existent côté serveur. Il demande `Client.delete_index(name, *, wait=True)` et `Client.snapshot(index, *, wait=True)`, avec leurs équivalents asynchrones, et cite les contournements en place : deux scripts de mesure réimplémentent la suppression en `httpx` (`tools/bench/mes02_encode_crible.py:157-177`, `tools/bench/mes02b_attach.py:175-190`), et la sauvegarde passe par la CLI `crible snapshot`, qui est hors ligne et exige l'arrêt du serveur.

### Ce que la conception et les ADR voisins posent

ADR-0112, décision 1 (`:52`), fait de la liste de méthodes de la fiche un **plancher** et renvoie explicitement le `PATCH` et le `DELETE` d'index ainsi que les instantanés à une tâche ultérieure. Le report est conditionnel : la ligne « Idées hors périmètre » du 2026-10-30 (`docs/suivi.md:870`) dit « à rouvrir seulement si le MCP de Legifacile ou `IMPORTER-BASE-COMPLETE` en a besoin ». La condition est remplie : Legifacile en a besoin, et le ticket 01 le documente.

Les deux routes existent et sont décrites par `docs/05-api.md` : `DELETE /v1/indexes/{index}?confirm={index}` (`:64`, `:137`) et `POST /v1/indexes/{index}/snapshots` (`:74`). ADR-0105, décision 5 (`:67`), fixe le protocole de suppression : un `confirm` absent ou différent du nom rend `400 invalid_request`, et le message nomme le paramètre `confirm` ; sinon la réponse est `202` avec un job `delete_index`. La route d'instantané rend `202` avec un job `snapshot`, sans corps.

**NON-COUVERT par ADR-0112.** Aucun texte ne dit qui remplit le `?confirm={index}` quand l'appelant est le client Python. C'est le seul point que cet ADR tranche vraiment ; les autres décisions ne font qu'appliquer ADR-0112 et `docs/05` aux deux nouvelles méthodes.

**Ce qui n'est pas à décider ici.** Le ticket demande `wait=True`. Ce paramètre est contraire à ADR-0112, décision 2 (`:65`), qui fait rendre le job sans attendre aux méthodes qui créent un job, et à sa décision 4 (`:92-96`), qui place l'attente dans `wait(job)`. Aucun des deux `compact` et `export` n'a de paramètre `wait`. La signature est donc fixée sans `wait`, et l'alternative est notée plus bas.

## Décision

### Décision 1 — `delete_index(index, *, confirm)`

`Client.delete_index(index, *, confirm)` et `AsyncClient.delete_index(index, *, confirm)` :

- `confirm` est un **argument nommé obligatoire**, sans valeur par défaut ;
- le client le transmet **tel quel** en paramètre de requête (`?confirm=<valeur>`) ;
- le client **ne le vérifie pas et ne le remplit pas** : c'est le serveur qui vérifie (ADR-0105, décision 5) ; un `confirm` faux rend `InvalidRequestError` (`400 invalid_request`) ;
- la méthode rend le job (`202`, `kind` valant `delete_index`) **sans attendre**.

### Décision 2 — `snapshot(index)`

`Client.snapshot(index)` et `AsyncClient.snapshot(index)` font un `POST` vers `/v1/indexes/{index}/snapshots`, **sans corps**, et rendent le job (`202`, `kind` valant `snapshot`) **sans attendre**.

### Décision 3 — l'attente passe par `client.wait(job)`

Aucune des deux méthodes n'a de paramètre `wait`. L'appelant qui veut la fin du job passe l'objet rendu à `client.wait(job)` (ADR-0112, décision 4), exactement comme pour `attach_batch`, `upsert_documents`, `patch_documents`, `compact` et `export`.

### Décision 4 — rien ne change côté serveur ni dans `docs/05`

Les deux routes existent, et leur contrat est déjà écrit. Cet ADR ne modifie ni le serveur, ni `docs/05-api.md` : `docs/05` décrit l'API, et le client s'y réfère (ADR-0112, décision 2). La documentation des deux méthodes va dans `clients/python/README.md`, avec les autres méthodes du client.

### Décision 5 — classe de compatibilité

**Sans risque** (ADR-0143, décision 7, `:118-125`) : aucun contrat et aucun format ne changent. L'ajout est limité au paquet Python ; un client 1.0.0 reste valide, et un serveur 1.0.0 accepte déjà les deux routes.

### Ce qui n'est pas décidé ici

- le **contenu exact de `clients/python/README.md`** : il appartient à `@dev`, avec la tâche ;
- le **détail des tests** (fichiers, noms, données) : ils suivent les mécanismes d'ADR-0112, décisions 6 et 7, dont le marqueur `server` ;
- la **forme de l'objet job rendu** : c'est le JSON de la réponse (ADR-0112, décision 2), et `docs/05` le décrit ;
- la **ligne T01 de `docs/suivi.md:162`**, qui annonce `wait=True` : cet ADR fixe la signature sans ce paramètre, et l'orchestrateur met la ligne à jour. `@greffier` ne touche pas au suivi.

## Conséquences

- **Positives** :
  - un script Python supprime un index et demande un instantané à chaud sans descendre à `httpx` ni arrêter le serveur ;
  - le garde-fou d'ADR-0105 reste entier : un appel qui se trompe de nom est refusé par le serveur, pas masqué par le client ;
  - la signature reste celle des autres méthodes du client : un job rendu, l'attente par `wait`, les erreurs typées d'ADR-0112 ;
  - aucun changement de contrat, de format ni de serveur : la classe est « sans risque » au sens d'ADR-0143.
- **Négatives / coûts** :
  - la signature est un peu redondante : le nom de l'index apparaît deux fois dans l'appel (`index` et `confirm`), et un appelant qui recopie mal le second reçoit un `400` au lieu d'un succès ;
  - les scripts de Legifacile doivent remplacer leur code `httpx` par deux appels, et leurs boucles d'attente par `client.wait(job)`.
- **À surveiller** :
  - la fréquence des `400` dus à un `confirm` erroné : si elle domine, la question d'un contrôle côté client revient au propriétaire, par un nouvel ADR, et non par une dérive silencieuse ;
  - un besoin réel d'attente immédiate dans les scripts : il se règle par `client.wait(job)`, pas par un paramètre ajouté sans décision ;
  - `docs/05-api.md` : toute évolution du contrat de suppression ou d'instantané doit rester la référence, et le client suivre sans réécrire cet ADR.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Le client remplit `confirm = index` | Le garde-fou d'ADR-0105 deviendrait décoratif pour tout appelant Python : le client supprimerait l'index nommé sans que l'appelant ait à confirmer quoi que ce soit. Le `confirm` est là pour qu'une suppression ne parte pas sur une faute de frappe, et un client qui le remplit annule cette protection. |
| `delete_index(index, *, wait=True)` et `snapshot(index, *, wait=True)`, comme le demande le ticket | Contraire à ADR-0112, décisions 2 (`:65`) et 4 (`:92-96`) : les méthodes qui créent un job le rendent sans attendre, et l'attente est dans `wait(job)` avec son échéance et son pas de sondage réglables. `compact` et `export` n'ont pas de paramètre `wait`. Changer la règle demanderait de réécrire ADR-0112, ce que cet ADR ne fait pas. |
| Documenter les deux méthodes dans `docs/05-api.md`, comme le propose la piste du ticket | ADR-0112, décision 2, et le rôle de `docs/05` : ce document décrit l'API, pas le client Python. Une section de client y créerait une seconde référence à tenir. Le README du client est le lieu de la documentation. |
| Ajouter `delete_index` et `snapshot` au serveur ou à l'API | Les deux routes existent déjà (`docs/05:64`, `:74`, `:137`), avec leur contrat et leurs jobs. Le manque est dans le client. |

## Références

- Ticket 01 de Legifacile : `../legifacile/tickets-crible/01-client-python-delete-index-et-snapshot.md` (lecture seule).
- `docs/05-api.md:64` (tableau des routes, `DELETE ?confirm=`), `:74` (`POST …/snapshots`), `:137` (protocole de suppression).
- `docs/suivi.md:162` (ligne T01), `:870` (report conditionnel des routes hors ADR-0112), `:628` (T01 classée sans risque).
- ADR-0105, décision 5 (`:67`) : `confirm` absent ou différent du nom rend `400 invalid_request`.
- ADR-0112 : décision 1 (`:52`, la liste est un plancher, `DELETE` et instantanés renvoyés à plus tard), décision 2 (`:65`, job rendu sans attendre), décision 4 (`:92-96`, `wait`), décisions 6 et 7 (tests `server`, scénario J6).
- ADR-0143, décision 7 (`:118-125`) : classes de compatibilité, dont « sans risque ».
- `clients/python/src/crible/client.py` (`_Core`, `Client`, `AsyncClient`, `compact`, `export`) ; `clients/python/README.md`.
