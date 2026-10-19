# ADR-0004 — Pas d'ANN en v1 : codes binaires, scan du pool et rescoring

- **Statut** : Accepté
- **Date** : 2026-10-17
- **Décideurs** : propriétaire
- **Proposé par** : brainstorm de conception

## Contexte

Machine de production : Mac mini M1 de 16 Go, avec Docker dans une VM d'environ 8 Go. Pour Legifacile (8,58 M passages en 768 dimensions), les vecteurs pèsent 13,2 Go en fp16. Dans pgvector, les index HNSW représentent 17,3 Gio pour 7,9 Gio de RAM, ne tiennent pas en mémoire, et le `REINDEX` d'un seul index a pris 1 h 26.

Les requêtes sont presque toujours filtrées, si bien que le pool à parcourir est souvent bien plus petit que la base.

## Décision

**Pas d'index ANN (ni HNSW, ni IVF) en v1.** On garde en RAM des **codes binaires** (1 bit par dimension, soit 0,82 Go pour Legifacile). Une recherche fait trois étapes :
1. elle **scanne le pool filtré** en distance de Hamming ;
2. elle garde les N meilleurs ;
3. elle les **re-score** en exact avec les vecteurs fp16 lus sur disque (mmap).

Si le pool est sous le seuil exact, le calcul est exact sur tout le pool.

## Conséquences

- **Positives** :
  - tout tient en RAM ;
  - le résultat est exact par construction sur un petit pool ;
  - le coût suit la taille du pool, pas celle de la base ;
  - il n'y a pas de graphe à construire ni à réparer, donc les mises à jour sont triviales.
- **Négatives** :
  - le binaire perd de l'information, et le rappel n'est pas garanti en 768 dimensions (MES-01) ;
  - une requête sans filtre sur une très grosse base coûte cher.
- **À surveiller** : MES-01 (fidélité selon N) et MES-02 (latence selon la taille du pool). Au-delà d'environ 30 M passages par index, ajouter un partitionnement IVF (nouvel ADR).

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| HNSW | Graphe trop gros pour la RAM, construction lente, suppressions coûteuses, post-filtrage qui perd des résultats |
| IVF dès la v1 | Pas nécessaire à l'échelle de Legifacile ; dérive des centroïdes à gérer |
| Scan en int8 | 6,6 Go en RAM pour Legifacile, trop pour la VM ; c'est un levier de repli possible |

## Références

`docs/03-recherche.md` (branche dense) ; `docs/02-architecture.md` (mémoire) ; MES-01, MES-02, MES-03.
