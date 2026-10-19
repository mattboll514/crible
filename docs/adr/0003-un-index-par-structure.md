# ADR-0003 — Un index par structure de données

- **Statut** : Accepté
- **Date** : 2026-10-18
- **Décideurs** : propriétaire
- **Proposé par** : propriétaire (correction de la proposition « un index par domaine »)

## Contexte

Legifacile a 8 corpus, de structures très différentes : articles de code, décisions découpées en parties, titres, blocs SARDE, dossiers législatifs. Aujourd'hui, ils partagent une table aux colonnes génériques, dont le sens change selon le corpus (`cle_domaine` désigne la juridiction pour les décisions et la nature du texte pour le JORF). Il a fallu ajouter une matrice de capacités (`vec.corpus_capacites`) pour rejeter les filtres sans objet.

La première proposition, un seul index par domaine, aurait reproduit ce problème.

## Décision

**Un index par structure.** La règle de découpage :
- même schéma, même découpage et même gabarit ⇒ même index ;
- ce qui ne diffère que par une valeur ⇒ un champ filtrable de l'index.

Pour Legifacile, il y a six index : `articles` (textes en vigueur et hors vigueur), `jorf`, `decisions` (texte intégral et sommaires, avec « sommaire » comme valeur de `partie`), `titres`, `sarde` et `dole`.

## Conséquences

- **Positives** :
  - chaque index a un schéma propre et validé, qui remplace la matrice de capacités ;
  - la vigueur devient une affaire de dates, si bien qu'un article ne change plus d'index quand il sort de vigueur.
- **Négatives** : une recherche qui couvre plusieurs structures demande la recherche multi-index (ADR-0009).
- **À surveiller** : la fusion des sommaires et du texte intégral dans `decisions` (validée, mais c'est la moins évidente).

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Un index unique aux colonnes génériques | Le sens des champs varie selon le corpus, il faut une matrice de capacités, la validation est faible |
| Un index par corpus actuel | Vigueur et hors vigueur, sommaires et texte intégral ont la même structure : on dupliquerait tout et on déplacerait des documents à chaque changement d'état |

## Références

`docs/02-architecture.md` (découpage validé pour Legifacile) ; `docs/phases/P9-migration-legifacile.md` (`DEFINIR-INDEX-LEGIFACILE`).
