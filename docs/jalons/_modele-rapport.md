# Rapport de jalon J<n> — <nom de la phase>

> Rédigé par l'agent `jalon` (commande `/jalon J<n>`). **Seul le propriétaire peut valider le jalon**, dans `docs/suivi.md`.

- **Date** :
- **Branche de phase / commit** :
- **Machine** :
- **Verdict de l'agent** : `PRET-POUR-VALIDATION` | `INCOMPLET`

## 1. Tâches de la phase

| Tâche | Statut dans `suivi.md` | Commit | Remarque |
|---|---|---|---|

## 2. Qualité

| Contrôle | Commande | Résultat |
|---|---|---|
| Formatage | `cargo fmt --check` | |
| Lints | `cargo clippy --all-targets -- -D warnings` | |
| Tests Rust | `cargo test` | `<n> passés, <n> échoués` |
| Tests Python | `uv run pytest` (si concerné) | |

## 3. Invariants de la phase

| Invariant | Test(s) | Résultat |
|---|---|---|

## 4. Mesures de la phase

| Mesure | Rapport | Chiffre clé | Cible | Atteinte ? |
|---|---|---|---|---|

## 5. Démo à rejouer par le propriétaire

Les commandes exactes, à copier-coller, puis ce qu'on doit observer.

```
# commande 1
```
Attendu : …

## 6. Décisions (ADR) de la phase

| ADR | Titre | Statut | Bloque la validation ? |
|---|---|---|---|

Un ADR encore `Proposé` dont dépend la phase doit être tranché par le propriétaire avant la validation.

## 7. Écarts, idées, risques

- Écarts à la conception ouverts (voir `suivi.md`) :
- Risques constatés :
- Points à décider par le propriétaire :

## 8. Si le jalon n'est pas validé

Les corrections demandées deviennent des tâches `CORRIGER-…` dans la fiche de la phase, avec leurs critères d'acceptation.
