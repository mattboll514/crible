---
description: Opération git directe via l'agent git (ex. /git etat, /git pousser main)
agent: git
subtask: true
---

Opération demandée par le propriétaire : **$ARGUMENTS**

État actuel :
!`git status --short --branch`

Exécute cette opération selon tes règles. Si elle n'est pas dans ta liste d'opérations, ou si elle est interdite, dis-le et propose l'opération autorisée la plus proche.
