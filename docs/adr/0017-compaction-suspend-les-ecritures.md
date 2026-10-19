# ADR-0017 — En v1, la compaction suspend les écritures

- **Statut** : Accepté (décision du propriétaire le 2026-10-19)
- **Date** : 2026-10-18
- **Décideurs** : propriétaire
- **Proposé par** : rédaction de la conception (non discuté pendant le brainstorm)

## Contexte

La compaction (ADR-0006) construit une nouvelle époque, avec des ids renumérotés. Accepter des écritures pendant ce temps imposerait de les rejouer sur la nouvelle époque, avec un journal de rejeu. Pour Legifacile, il y a un lot par nuit, et la compaction tourne surtout après une campagne. Sa durée est estimée à quelques dizaines de minutes (MES-07).

## Décision

En v1, **la compaction suspend l'écrivain de l'index**. Les écritures attendent dans la file, et au-delà de sa capacité, le serveur répond `503`. **Les lectures continuent normalement** sur l'ancienne époque, jusqu'à la bascule.

## Conséquences

- **Positives** : implémentation simple, aucune écriture perdue, aucune lecture bloquée (INV-02).
- **Négatives** : pas d'écriture pendant la compaction. Il faut planifier la compaction en dehors de l'heure de la tâche quotidienne.
- **À surveiller** : si MES-07 dépasse ce qui est acceptable, ou si un projet a besoin d'écrire en continu, il faudra un journal de rejeu (nouvel ADR).

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Journal de rejeu des écritures pendant la compaction | Complexité non justifiée par la cadence quotidienne |

## Références

`docs/02-architecture.md` (compaction) ; ADR-0006 ; MES-07.
