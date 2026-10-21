# Mesures

**Règle** : aucune affirmation de performance ou de qualité sans une mesure consignée ici. La liste des mesures (MES-00 à MES-15) et des cibles se trouve dans `docs/08-invariants-et-tests.md`.

- **Un fichier par mesure** : `MES-xx-<slug>.md`. Une nouvelle campagne sur la même mesure ajoute une section datée ; on n'écrase jamais l'ancienne.
- **Données brutes** : en JSON ou CSV à côté du rapport (`MES-xx-<slug>.<date>.json`) si elles sont petites ; sinon, le chemin dans `data/` est cité.
- **Interdit** : modifier du code pour améliorer un chiffre dans la même session que la mesure. On mesure, on consigne, puis l'optimisation devient une tâche à part.

## Modèle

```markdown
# MES-xx — <question>

## Campagne du <date>

- Tâche : NOM-DE-TACHE
- Commit : <sha>
- Machine : <modèle, CPU, RAM ; natif ou Docker (RAM de la VM)>
- Jeu de données : <nom, taille, somme de contrôle>
- Configuration : <paramètres pertinents : N, mode, précision, threads…>

### Protocole
Commandes exactes pour reproduire. Préchauffage (oui/non), nombre de répétitions, cache froid ou chaud.

### Résultats
| Paramètre | p50 | p95 | p99 | Autre métrique |
|---|---|---|---|---|

### Conclusion
Ce que la mesure permet d'affirmer, et ce qu'elle ne permet pas. Comparaison à la cible ou au budget. Recommandation éventuelle (à valider par le propriétaire).
```
