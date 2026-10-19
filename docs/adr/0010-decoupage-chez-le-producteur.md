# ADR-0010 — Le découpage reste chez le producteur

- **Statut** : Accepté
- **Date** : 2026-10-18
- **Décideurs** : propriétaire
- **Proposé par** : brainstorm (question du propriétaire sur le découpage)

## Contexte

Le découpage de Legifacile est un savoir métier :
- 1 024 tokens avec 15 % de chevauchement ;
- les décisions découpées en parties (faits, moyens, motifs, dispositif), dans sa version 3 ;
- l'en-tête de décision exclu ;
- des gabarits par corpus.

Ce découpage a déjà évolué, puisque la jurisprudence est passée de 2,0 M à 3,0 M passages. Le moteur, lui, doit savoir qu'un document se compose de passages : pour les mises à jour, le regroupement et la traçabilité.

## Décision

- **Le producteur découpe**, compose le gabarit et encode. Il déclare `chunker` et `template` dans le manifeste du lot.
- **Le moteur modélise documents → passages** :
  - le document est l'unité de mise à jour, et il arrive toujours avec tous ses passages ;
  - le moteur vérifie les longueurs (INV-11) et trace le découpeur de chaque lot ;
  - il regroupe par document (`group_by`) et dédoublonne par contenu.
- Pas de découpeur dans le moteur en v1. Le `decouper` générique de Legifacile pourra devenir un découpeur par défaut plus tard.

## Conséquences

- **Positives** : la logique métier reste dans le projet qui la connaît, et le moteur reste générique.
- **Négatives** : un projet sans pipeline doit découper lui-même avant d'envoyer ses documents.
- **À surveiller** : la demande d'autres projets pour un découpeur intégré.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Découpeur configurable dans le moteur dès la v1 | Ne couvre pas les règles métier (parties des décisions) ; fonctionnalité spéculative |

## Références

`docs/04-donnees-et-ingestion.md` (modèle documents → passages ; le découpage, qui fait quoi).
