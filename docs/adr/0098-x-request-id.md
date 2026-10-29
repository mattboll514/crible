# ADR-0098 — `x-request-id`

- **Statut** : Accepté (accepté par anticipation le 2026-10-29, confirmé par le propriétaire le 2026-10-29, tel que rédigé)
- **Date** : 2026-10-29
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, tâche DEMARRER-SERVEUR-AXUM (phase P6), sur la demande du propriétaire
- **Remplace** : —

## Contexte

`docs/05-api.md` demande déjà deux choses : « chaque réponse porte l'en-tête `x-request-id` » (`docs/05:14`) et le corps d'erreur porte un champ `request_id` (`docs/05:26`). `docs/06-observabilite.md:115` demande que chaque ligne de log porte le `request_id` quand il existe, et le journal de debug l'enregistre (`docs/06:68`).

Ces trois phrases ne disent pas **d'où vient** la valeur : reprise de l'appelant ou génération par le serveur. La question n'est pas cosmétique :

- un client qui regroupe ses appels (le MCP de Legifacile) veut rapprocher sa trace de celle du serveur, donc fournir l'identifiant ;
- une valeur fournie est une donnée extérieure, de longueur et de contenu inconnus : la journaliser telle quelle ouvre la porte à une injection de ligne de log ou à une fuite de contenu arbitraire ;
- un identifiant manquant priverait de traçabilité les réponses d'erreur produites avant tout traitement, `404`, `405`, `413` et `500` comprises.

## Décision

**Reprise ou génération.** Si la requête entrante porte un en-tête `x-request-id` d'au plus **128 octets**, composé uniquement de caractères `[A-Za-z0-9._:-]`, sa valeur est reprise. Sinon — en-tête absent, trop long ou contenant un autre caractère — un **UUID v4** est généré. La valeur refusée n'est **jamais** journalisée : ni sa longueur, ni son contenu, ni un extrait.

**Diffusion.** La valeur retenue est :

- renvoyée dans l'en-tête `x-request-id` de **toutes** les réponses, y compris `404`, `405`, `413` et `500` ;
- portée dans le champ `request_id` du corps d'erreur (`docs/05-api.md`) et dans la réponse de recherche ;
- portée par **chaque ligne de log** émise pendant la requête.

Ce qui n'est pas décidé ici : le format des logs, fixé par `docs/06` ; la présence du `request_id` dans les analytics, qui n'en veulent aucun identifiant (`docs/06`, INV-13) ; la génération côté client, qui n'est pas du ressort de Crible.

## Conséquences

- **Positives** :
  - la trace est continue entre le MCP, la réponse et les logs du serveur ;
  - la borne de 128 octets et le jeu de caractères rendent la journalisation sûre par construction : aucune séquence de contrôle ne peut entrer dans une ligne de log ;
  - une réponse d'erreur précoce reste traçable, puisque l'en-tête est posé par la couche la plus externe.
- **Négatives / coûts** :
  - un client qui envoie un identifiant hors du jeu de caractères ne le retrouve pas dans la réponse ; il doit lire l'en-tête retourné plutôt que supposer le sien ;
  - la valeur refusée est ignorée sans trace, donc un client fautif ne reçoit aucun diagnostic sur son propre identifiant ;
  - une crate de plus (`uuid`, ADR-0096).
- **À surveiller** : un test doit vérifier les quatre états — repris tel quel, absent, trop long, caractère interdit — et le fait qu'aucune réponse, y compris `404` et `500`, n'omet l'en-tête. Si un middleware futur pose une réponse avant la couche qui attribue l'identifiant, ce test le dira.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Toujours générer, ignorer l'en-tête entrant | Le client perd le rapprochement avec sa propre trace, qui est la raison d'être de l'en-tête. |
| Reprendre l'en-tête sans borne ni filtre | Une valeur de plusieurs kilo-octets ou contenant un retour à la ligne entre dans les logs : c'est un vecteur d'injection et une fuite de contenu. |
| Reprendre et journaliser intégralement une valeur refusée | Journaliser ce qu'on refuse annule l'intérêt du refus. |
| Répondre `400` sur un identifiant invalide | Une valeur de trace n'est pas une donnée métier : refuser la requête pour cela rendrait un client incapable de chercher à cause d'un en-tête de diagnostic. |
| Un compteur par processus au lieu d'un UUID | Voir ADR-0096 : l'ordre et le volume fuient, et deux instances produisent les mêmes valeurs. |

## Références

`docs/05-api.md` (conventions, ligne 14 ; format d'erreur, ligne 26) ; `docs/06-observabilite.md` (santé et logs, ligne 115 ; journal de debug) ; ADR-0096 (crate `uuid`, feature `v4`) ; ADR-0099 (cycle de vie du serveur) ; fiche `DEMARRER-SERVEUR-AXUM`.
