---
description: Localiser du code avec ColGrep-Agent, lecture seule (ex. /chercher où sont appliquées les pierres tombales)
agent: plan
subtask: true
---

Question de localisation : **$ARGUMENTS**

Emplacements trouvés par ColGrep-Agent (modèle local, lecture seule) :
!`colgrep --agent "$ARGUMENTS" --color never 2>/dev/null || echo "COLGREP-AGENT INDISPONIBLE : lancer 'colgrep --install-agent' (une fois), et vérifier que du code existe dans le dépôt."`

Présente ces emplacements sous forme de liste `fichier:lignes`, avec une phrase par emplacement pour dire ce qu'on y trouve. Ne modifie rien. Si la sortie est vide ou indique une indisponibilité, dis-le et propose une recherche équivalente avec `colgrep "<intention>"` ou `rg`.
