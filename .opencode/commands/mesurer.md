---
description: Exécuter une mesure MES-xx, écrire son rapport et le committer (ex. /mesurer MES-02)
agent: orchestrateur
---

Fais exécuter la mesure **$1** :
1. `@mesureur $1` ;
2. `@git` → `commit-docs`, avec les fichiers et le message proposés par le mesureur ;
3. résume-moi le résultat en 5 lignes : chiffre clé, cible, conclusion, recommandation éventuelle à valider.

Définition de la mesure :
!`grep -n "$1" docs/08-invariants-et-tests.md`

Tâches qui la citent :
!`grep -n "$1" docs/phases/*.md`
