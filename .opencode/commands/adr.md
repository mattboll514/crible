---
description: Faire rédiger un ADR en statut Proposé, puis le committer (ex. /adr choisir le N par défaut du scan binaire)
agent: orchestrateur
---

Fais rédiger par `@greffier` un nouvel ADR en statut **Proposé**, sur le sujet suivant :

$ARGUMENTS

Puis lance `@git` → `commit-docs`, avec le message proposé par le greffier. Termine par le chemin du fichier et un résumé de l'ADR en 3 lignes, et rappelle-moi que c'est à moi de l'accepter ou de le rejeter.

Index actuel des ADR :
!`grep -E "^\| \[ADR-" docs/adr/README.md`
