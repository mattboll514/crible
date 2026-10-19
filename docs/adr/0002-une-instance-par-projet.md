# ADR-0002 — Une instance par projet

- **Statut** : Accepté
- **Date** : 2026-10-18
- **Décideurs** : propriétaire
- **Proposé par** : propriétaire (correction d'une première proposition de serveur partagé)

## Contexte

Crible doit servir Legifacile, puis d'autres projets. La première proposition était un serveur unique, partagé par tous les projets, avec des clés d'API et des permissions par projet.

## Décision

**Une instance de Crible par projet** : un conteneur, ses volumes, sa configuration et ses analytics. Une instance contient plusieurs index (ADR-0003). Tous les projets utilisent la même image.

## Conséquences

- **Positives** :
  - isolation complète : pannes, ressources, mises à jour et sauvegardes indépendantes ;
  - pas de multi-tenant, des clés d'API simples (une par client du projet) ;
  - des analytics propres à chaque projet.
- **Négatives** : chaque instance charge son propre modèle (~0,3 Go en INT8) et ses codes binaires. Il y a plusieurs conteneurs à surveiller.
- **À surveiller** : la RAM totale sur le Mac mini quand plusieurs projets tourneront (MES-09).

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Un serveur partagé entre projets | Multi-tenant, permissions croisées, un incident touche tous les projets |

## Références

`docs/01-vision-et-perimetre.md` (hors périmètre) ; `docs/02-architecture.md` (instance, index).
