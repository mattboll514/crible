# ADR-0011 — Stocker le texte exact envoyé à l'encodeur

- **Statut** : Accepté
- **Date** : 2026-10-18
- **Décideurs** : propriétaire
- **Proposé par** : brainstorm de conception

## Contexte

Aujourd'hui, Legifacile ne garde que le sha256 du texte composé. À chaque requête, il recompose le texte en relançant le compositeur et le découpeur : environ **39 ms par résultat**, et une erreur (`IndexError`) si la source a changé depuis l'encodage. La raison était légitime : garantir que le texte affiché soit bien le texte encodé.

## Décision

Crible **stocke l'entrée encodeur exacte** de chaque passage (en-tête compris, sans le prompt), compressée en zstd dans SQLite. C'est ce texte qui est :
- affiché ;
- indexé en BM25 (ADR-0020) ;
- ré-encodé par la recette ;
- utilisé par le rerank futur et pour une migration de modèle.

## Conséquences

- **Positives** :
  - le texte affiché est par définition le texte encodé ;
  - plus de recomposition ;
  - un changement de modèle peut se faire sans le pipeline source.
- **Négatives** : de l'espace disque, estimé à 3 ou 4 Go compressés pour Legifacile.
- **À surveiller** : la taille réelle à l'import complet (`IMPORTER-BASE-COMPLETE`).

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Ne garder qu'un hash (existant) | Recomposition coûteuse et fragile ; recette, BM25 et rerank impossibles sans la source |

## Références

`docs/04-donnees-et-ingestion.md` (modèle documents → passages) ; `docs/02-architecture.md` (table `passages`).
