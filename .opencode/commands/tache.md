---
description: Exécuter une tâche du plan avec le pipeline complet (ex. /tache INITIALISER-WORKSPACE-RUST)
agent: orchestrateur
---

Exécute la tâche **$1** avec ton pipeline complet : préconditions, `@git debut-tache`, tests, `@dev`, `@relecteur`, suivi, `@git commit`, `@git fin-tache`, compte rendu.

Emplacement de la fiche :
!`grep -n "^### $1$" docs/phases/*.md`

Ligne de la tâche dans le suivi :
!`grep -n "| $1 |" docs/suivi.md`

Jalons :
!`grep -n "^| J[0-9] |" docs/suivi.md`

État git :
!`git status --short --branch`

Si la fiche est introuvable, n'invente rien : donne la liste des tâches proches (`grep -n "^### " docs/phases/*.md`) et arrête-toi.
