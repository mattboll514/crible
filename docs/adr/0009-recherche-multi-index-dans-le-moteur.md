# ADR-0009 — Recherche multi-index dans le moteur

- **Statut** : Accepté
- **Date** : 2026-10-18
- **Décideurs** : propriétaire
- **Proposé par** : brainstorm (conséquence d'ADR-0003)

## Contexte

Avec un index par structure (ADR-0003), un périmètre de Legifacile comme « droit applicable » couvre plusieurs index. Aujourd'hui, le MCP de Legifacile fusionne lui-même les corpus par cosinus brut, et l'évaluation a validé cette comparabilité (`serveur_mcp/perimetres.py`). Les scores BM25, en revanche, ne se comparent pas d'un index à l'autre : chaque index a ses propres IDF et sa longueur moyenne de document.

## Décision

La route `POST /v1/search` accepte des `targets: [{index, filter}]`. Le moteur :
- encode la requête **une seule fois**, et exige la même empreinte de modèle pour toutes les cibles ;
- exécute le pipeline complet sur chaque index ;
- fusionne les résultats :
  - en mode **dense**, **par cosinus**, ce qui est exact ;
  - en modes **lexical** et **hybride**, **par RRF** sur les rangs de chaque index.

## Conséquences

- **Positives** : un seul encodage, une seule trace de debug, une seule entrée d'analytics. Le MCP se simplifie.
- **Négatives** : le classement multi-index en lexical et en hybride repose sur les rangs, pas sur les scores.
- **À surveiller** : la qualité multi-index sur les périmètres réels (`COMPARER-AVEC-PGVECTOR`).

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Fusion côté client (comme aujourd'hui) | N encodages, pas de debug unifié, logique dupliquée dans chaque projet |
| Normaliser les scores BM25 entre index | Fragile, dépend de la distribution des scores de chaque requête |

## Références

`docs/03-recherche.md` (recherche multi-index) ; `docs/05-api.md` (`POST /v1/search`).
