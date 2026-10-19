---
description: Résumer l'état du projet Crible et proposer la prochaine tâche (sans rien modifier)
agent: orchestrateur
---

Fais le point **sans rien modifier**.

Commence par lire `docs/suivi.md` (jalons, tâches, décisions en attente, écarts).

Branche et état git :
!`git status --short --branch`
!`git log --oneline -10`

Réponds en 15 lignes au plus :
1. Le jalon courant et son statut, la phase ouverte.
2. Les tâches de la phase : nombre FAIT, EN-COURS, BLOQUE, A-FAIRE. Les tâches BLOQUE, avec leur raison.
3. Les ADR `Proposé` et les écarts ouverts : ce sont des décisions à prendre par le propriétaire.
4. **La prochaine tâche recommandée** : la première `A-FAIRE` de la phase ouverte dont toutes les dépendances sont `FAIT` (vérifie le champ « Dépend de » de sa fiche). Donne la commande exacte `/tache NOM`.
5. Si toutes les tâches de la phase sont FAIT, recommande `/jalon J<n>`.
