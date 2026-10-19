# ADR-0019 — Champs stockés en JSON canonique dans SQLite

- **Statut** : Accepté (décision du propriétaire le 2026-10-19)
- **Date** : 2026-10-18
- **Décideurs** : propriétaire
- **Proposé par** : rédaction de la conception (non discuté pendant le brainstorm)

## Contexte

Chaque index a son propre schéma (ADR-0003). Les champs doivent être stockés de façon durable dans SQLite, puis servir à construire les bitmaps et les colonnes en RAM. On peut ajouter un champ à un index après sa création.

## Décision

Les champs de document et de passage sont stockés dans **une colonne JSON canonique** (clés triées) de `documents` et de `passages`. Les bitmaps et les colonnes de valeurs en sont **dérivées** au démarrage. Si le démarrage est trop lent, on ajoutera un cache sérialisé, estampillé par génération.

## Conséquences

- **Positives** :
  - générique quel que soit le schéma ;
  - ajouter un champ ne demande pas de migration SQL ;
  - le JSON sert de référence au test INV-07, grâce à `json_extract`.
- **Négatives** : lire et analyser 8,58 M lignes JSON au démarrage prend du temps.
- **À surveiller** : MES-13 (temps de démarrage). Au-delà de 60 s à l'échelle de Legifacile, ajouter le cache (tâche `AJOUTER-CACHE-BITMAPS`).

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Une table SQL typée par schéma, générée | Migrations à chaque ajout de champ, code plus complexe |

## Références

`docs/02-architecture.md` (tables SQLite) ; INV-07 ; MES-13.
