---
description: Faire relire les changements de la branche de tâche courante
agent: relecteur
subtask: true
---

Relis les changements de la branche courante, par rapport à sa branche de phase.

Branche courante :
!`git branch --show-current`

Branches :
!`git branch --list "phase/*" "tache/*"`

Derniers commits :
!`git log --oneline -15`

La tâche relue est celle qui suit `tache/` dans le nom de la branche courante. Suis ta liste de contrôle et rends le verdict au format imposé.
