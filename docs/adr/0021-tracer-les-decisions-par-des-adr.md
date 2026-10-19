# ADR-0021 — Tracer les décisions par des ADR

- **Statut** : Accepté
- **Date** : 2026-10-19
- **Décideurs** : propriétaire
- **Proposé par** : propriétaire

## Contexte

Le développement est confié à des agents (opencode, DeepSeek), qui n'ont pas de mémoire durable d'une session à l'autre. Les décisions de conception ont été prises dans des conversations. Une personne qui rejoint le projet, humaine ou agent, doit pouvoir retrouver chaque décision, son pourquoi et ses alternatives, sans dépendre de la mémoire de qui que ce soit.

La première version de la conception tenait toutes les décisions dans un seul tableau (`09-decisions.md`).

## Décision

- **Chaque décision fait l'objet d'un Architecture Decision Record**, un fichier par décision, dans `docs/adr/`. Le processus est décrit dans `docs/adr/README.md` :
  - n'importe qui propose (statut `Proposé`), seul le propriétaire accepte ou rejette ;
  - un ADR accepté n'est jamais réécrit, on le **remplace** par un nouveau ;
  - les agents ne s'appuient que sur les ADR `Accepté`.
- **Les 20 décisions de la conception deviennent les ADR 0001 à 0020**, avec la même numérotation que les anciens D-01 à D-20.
- `09-decisions.md` est supprimé ; le plan de développement prend le numéro 09.
- **Un écart à la conception** rencontré pendant une tâche devient un ADR `Proposé`, et la tâche passe en `BLOQUE`.
- Un agent dédié, `greffier`, et la commande `/adr` rédigent les ADR.

## Conséquences

- **Positives** : une mémoire du projet versionnée, lisible par tous et indépendante des agents ; un historique des changements d'avis.
- **Négatives** : un peu d'écriture en plus à chaque décision ; il faut tenir l'index à jour.
- **À surveiller** : des décisions prises sans ADR. Le relecteur et l'agent jalon le vérifient.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Un tableau unique de décisions | Pas de contexte détaillé, pas d'historique des remplacements, conflits d'édition |
| Mémoire des agents ou messages de commit | Pas durable, pas lisible par un nouvel arrivant |

## Références

`docs/adr/README.md` ; `AGENTS.md` ; `.opencode/agents/greffier.md`.
