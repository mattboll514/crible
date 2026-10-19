# ADR-0018 — Filtres d'intervalle par colonnes de valeurs en RAM

- **Statut** : Accepté (décision du propriétaire le 2026-10-19)
- **Date** : 2026-10-18
- **Décideurs** : propriétaire
- **Proposé par** : rédaction de la conception (non discuté pendant le brainstorm)

## Contexte

Il faut filtrer sur des dates et des nombres : vigueur à une date, année, intervalles. Une bitmap par valeur ne convient pas, car les dates ont trop de valeurs distinctes. On a besoin d'une structure simple, correcte et peu gourmande.

## Décision

Pour chaque champ `integer`, `float` ou `date`, on garde en RAM **une colonne de valeurs indexée par iid** : `i64`, `f64`, ou `i32` en jours pour les dates, avec une sentinelle pour null. On y ajoute une bitmap « présent ».

Une clause d'intervalle se calcule en parcourant la colonne, mais seulement sur le domaine déjà réduit par les clauses en bitmap.

## Conséquences

- **Positives** : simple, exact, et environ 34 Mo par champ date pour Legifacile.
- **Négatives** : une clause d'intervalle seule parcourt toute la colonne (8,58 M valeurs).
- **À surveiller** : MES-14. Si c'est trop lent, ajouter des bitmaps par tranche (par année), avec un nouvel ADR.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Une bitmap par tranche (année) + vérification aux bornes | Plus complexe ; à ajouter seulement si la mesure l'exige |
| Bitmaps par tranches de bits | Complexe pour un gain incertain à cette échelle |
| Filtrer par SQL dans SQLite | Trop lent sur le chemin de chaque requête |

## Références

`docs/03-recherche.md` (filtres → pool) ; MES-14.
