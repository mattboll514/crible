# ADR-0005 — Précision de référence figée, étages de recherche réglables

- **Statut** : Accepté
- **Date** : 2026-10-17
- **Décideurs** : propriétaire
- **Proposé par** : propriétaire (choisir la quantification à la création de l'index) + brainstorm

## Contexte

Le mot « fp16 » recouvre en fait deux rôles :
- la précision **stockée** : celle qu'on garde, et à partir de laquelle on peut recalculer le reste ;
- les précisions **de recherche** : binaire pour le scan, fp16 pour le rescoring.

Dans Legifacile, les mesures montrent que `halfvec` (fp16) donne le même rappel que fp32 (99,4 %). Revenir à une précision plus haute impose de ré-encoder sur GPU : 15 h de L4 pour Legifacile.

## Décision

- La **précision de référence** (`fp16` ou `fp32`, `fp16` par défaut) se choisit à la création de l'index et **ne change plus**.
- Les **étages de recherche** sont dérivés de la référence et **réglables à tout moment** : `scan` (binaire ou aucun), N (`rescore_candidates`), `exact_threshold`.
- Principe général : **tout ce qui coûte du GPU est figé, tout le reste se reconstruit localement.**

## Conséquences

- **Positives** : on peut ajuster la vitesse et la qualité sans ré-encoder, et la frontière entre ce qui est figé et ce qui est réglable est nette.
- **Négatives** : passer de fp16 à fp32 demande de recréer l'index et de ré-encoder.
- **À surveiller** : si un futur modèle supporte mal le fp16, il faudra choisir fp32 à la création.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Une seule précision, non réglable | Interdit d'ajuster le compromis vitesse/qualité après coup |
| Stockage en int8 comme référence | Perte d'information irréversible |

## Références

`docs/04-donnees-et-ingestion.md` (figé contre réglable) ; `docs/03-recherche.md` (réglages de précision).
