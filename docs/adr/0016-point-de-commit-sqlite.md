# ADR-0016 — Le commit SQLite est l'unique point de commit

- **Statut** : Accepté (décision du propriétaire le 2026-10-19)
- **Date** : 2026-10-18
- **Décideurs** : propriétaire
- **Proposé par** : rédaction de la conception (non discuté pendant le brainstorm)

## Contexte

Une attache de lot écrit dans plusieurs structures : fichiers de segment, SQLite, tantivy, manifeste, instantané en mémoire. Un arrêt brutal peut survenir à n'importe quelle étape. Il faut une seule réponse à la question « quelle est la dernière génération valide ? ».

## Décision

- **La transaction SQLite qui passe `meta.generation` à G+1 est le seul point de commit.**
- Tout le reste en découle et se rattrape au démarrage :
  - les segments écrits avant le commit restent invisibles tant que SQLite ne les référence pas ; orphelins, ils sont supprimés ;
  - tantivy est rattrapé grâce à la génération portée par son commit ;
  - le manifeste est réécrit s'il manque.
- Ordre : segment → **commit SQLite** → tantivy → manifeste → publication.

## Conséquences

- **Positives** : une seule source de vérité, une reprise simple et testable (INV-04).
- **Négatives** : tantivy peut être temporairement en retard après une panne. Ce n'est pas grave, car la bitmap des vivants fait autorité.
- **À surveiller** : le test de panne à chaque étape (`TESTER-REPRISE-APRES-PANNE`).

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Manifeste JSON comme point de commit | Deux sources de vérité, SQLite et le manifeste, à garder cohérentes |
| Validation en deux phases entre SQLite et tantivy | Complexité inutile, puisque tantivy se reconstruit depuis SQLite |

## Références

`docs/02-architecture.md` (générations et point de commit) ; INV-03, INV-04.
