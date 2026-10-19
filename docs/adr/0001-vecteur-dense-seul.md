# ADR-0001 — Vecteur dense seul, sans late interaction

- **Statut** : Accepté
- **Date** : 2026-10-17
- **Décideurs** : propriétaire
- **Proposé par** : brainstorm de conception

## Contexte

Crible s'inspire de NextPlaid (LightOn), une base multi-vecteurs (late interaction, ColBERT). Dans les projets visés, à commencer par Legifacile, les données d'entrée sont très structurées. Les filtres (fonds, état, dates, juridiction…) apportent l'essentiel de la précision, et les champs texte sont souvent courts.

Le multi-vecteur coûte entre 5 et 25 fois le stockage du dense. Pour Legifacile, mLateOn aurait représenté environ 333 Go d'index, contre 33 Go en dense, sur un Mac mini de 16 Go.

Dans le sandbox LightOn, un hybride suivi d'un rerank mLateOn sur des candidats a atteint 0,713, contre 0,714 pour un index mLateOn complet. Le rerank récupère donc presque tout le gain du late interaction.

## Décision

Un **seul vecteur dense par passage**. Le late interaction n'est pas indexé. Il pourra venir plus tard comme **étape de rerank optionnelle** sur les candidats, hors v1.

## Conséquences

- **Positives** :
  - stockage et RAM minimaux ;
  - mises à jour simples, sans centroïdes à faire dériver ni à reconstruire ;
  - un seul système à exploiter.
- **Négatives** : le dense seul rate les articles courts et canoniques (l'art. 1240 du Code civil est au rang 70). C'est compensé par l'hybride BM25 (ADR-0008, ADR-0020), puis par le futur rerank.
- **À surveiller** : la qualité de l'hybride sur le jeu de référence de Legifacile (MES-10). Si elle reste insuffisante, il faudra prioriser le rerank.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Index multi-vecteur complet (NextPlaid, mLateOn) | Volume (333 Go), réécritures à l'ingestion, second système à opérer |
| MUVERA (multi-vecteur réduit à un vecteur) | Complexité ajoutée pour un gain incertain sur des données structurées |

## Références

`docs/01-vision-et-perimetre.md` ; `docs/03-recherche.md` (rerank hors v1). Dossier LightOn : `02-moteurs-index-nextplaid.md` et `12-projet-legifacile.md`.
