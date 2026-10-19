---
description: Préparer le rapport de validation d'un jalon (ex. /jalon J1)
agent: orchestrateur
---

Prépare la validation du jalon **$1** :
1. `@jalon $1` ;
2. `@git` → `commit-docs`, avec le message proposé par l'agent jalon ;
3. présente-moi le verdict, les décisions à prendre et les commandes de la démo, en 15 lignes au plus.

Jalons :
!`grep -n "^| J[0-9] |" docs/suivi.md`

État git :
!`git status --short --branch`

Rappel : un jalon n'est validé que lorsque j'écris explicitement « $1 validé » ou que je lance `/valider $1`.
