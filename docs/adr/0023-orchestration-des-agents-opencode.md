# ADR-0023 — Orchestration des agents dans opencode : un orchestrateur, un pipeline séquentiel, un seul agent git

- **Statut** : Accepté (décision du propriétaire le 2026-10-19) — point 5 (opencode v1) remplacé par ADR-0025
- **Date** : 2026-10-19
- **Décideurs** : propriétaire
- **Proposé par** : propriétaire (besoins : orchestrateur, agent git, agent de revue) ; conception détaillée par l'assistant

## Contexte

Le propriétaire veut discuter avec **un orchestrateur** qui lance des agents sur les tâches, confier **toute la gestion git** à un agent dédié, et disposer d'un agent de **revue de code**.

Faits vérifiés sur opencode v1 au 19/10/2026 :
- Les sous-agents tournent dans des **sessions enfants**, chacune avec son propre contexte.
- Depuis la v1.18.2, **un sous-agent ne peut plus lancer de sous-agent** (`experimental.subagent_depth` vaut 1 par défaut).
- La **compaction automatique ne se déclenche pas** si `limit.context` n'est pas déclaré pour un modèle hors du catalogue models.dev, ce qui est le cas des modèles Ollama.
- opencode existe désormais en **deux lignées**. La v2 renomme les clés de configuration et n'exécute pas les plugins v1. `brew install opencode` (homebrew-core) installe la v2.

Le modèle de développement est DeepSeek V4.1 Flash, servi par Ollama Cloud, avec une fenêtre de 1 M tokens. Un modèle « flash » suit mieux des étapes courtes et bien bornées que de longues chaînes de délégation.

## Décision

1. **Agent principal `orchestrateur`**, agent par défaut.
   - Il discute avec le propriétaire, tient `docs/suivi.md` (le seul fichier qu'il modifie) et délègue tout le reste.
   - Il ne code jamais.
2. **Pipeline séquentiel fixe** pour chaque tâche, piloté par l'orchestrateur :
   - `@git debut-tache` ;
   - `@testeur`, si la fiche cite des invariants ;
   - `@dev` ;
   - `@relecteur`, avec au plus 3 tours de correction ;
   - mise à jour du suivi ;
   - `@git commit`, puis `@git fin-tache`.

   **Pas de sous-agents imbriqués.** On n'active pas l'option expérimentale de profondeur. `@dev` ne fait plus de git et n'appelle aucun autre agent : il remonte un `STATUT` (`PRET-POUR-RELECTURE`, `QUESTION-CONCEPTION` ou `BLOQUE`).
3. **`@git` est le seul agent qui modifie le dépôt.**
   - La fusion dans `main`, le tag de jalon et le push se font uniquement sur instruction explicite du propriétaire (`/valider`, `/git pousser`).
   - Le push forcé et la réécriture d'historique sont interdits.
   - Les autres agents ont git en lecture seule. Leurs commandes git qui modifient le dépôt demandent confirmation.
4. **La revue de code est obligatoire** : `@relecteur` (qui existait déjà) fait partie du pipeline, et son verdict `OK` est nécessaire au commit.
5. **opencode v1**, installé par `brew install anomalyco/tap/opencode`. Pas de passage en v2 (`opencode upgrade --major`) sans nouvel ADR.
6. **Le modèle est déclaré avec `limit.context = 200000` et `limit.output = 32768`.**
   - La compaction automatique se déclenche donc vers 180 k tokens.
   - On reste volontairement bien en dessous de la fenêtre de 1 M : plus le contexte est long, moins le modèle suit bien les consignes.
   - Le chiffre de sortie est prudent et n'a pas été vérifié.
7. **Stratégie de sessions** :
   - une session d'orchestrateur par phase, ou par journée de travail ;
   - plutôt que d'enchaîner les compactions, `/new` puis `/etat`, puisque la mémoire du projet est dans les fichiers ;
   - le détail est dans `docs/guide-opencode.md`.

## Conséquences

- **Positives** :
  - le contexte de l'orchestrateur reste léger, car le travail lourd se fait dans des sessions enfants jetables ;
  - chaque étape est visible et traçable ;
  - le git est cohérent et sûr ;
  - pas de dépendance à une option expérimentale.
- **Négatives** :
  - plus d'appels de modèle par tâche (5 à 8 sous-agents), donc plus de latence ;
  - si la permission d'un agent ne prime pas sur la permission globale, `@git` demandera des confirmations. À vérifier dès la première tâche.
- **À surveiller** :
  - la durée et le coût en tokens d'une tâche type ;
  - la taille du contexte de l'orchestrateur au fil d'une phase ;
  - les sorties de `@dev` qui ne respectent pas le format du compte rendu.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Sous-agents imbriqués (`experimental.subagent_depth: 2`), `@dev` pilotant ses propres aides | Option expérimentale ; chaînes de délégation plus longues, donc plus fragiles avec un modèle « flash » |
| Plugin oh-my-opencode (orchestration « tout compris ») | Lourd, très opinionné, agents en parallèle qui multiplient les tokens, licence non standard ; contraire au plan séquentiel |
| Travailler directement avec `@dev`, sans orchestrateur | Pas de suivi ni de git automatiques ; tout repose sur la discipline humaine |
| Un seul long chat pour tout le projet, en compactant | Les résumés de compaction perdent de l'information ; la qualité baisse avec la longueur du contexte |

## Références

`AGENTS.md` ; `.opencode/agents/orchestrateur.md` et `git.md` ; `docs/guide-opencode.md`. Documentation opencode : agents, configuration, TUI ; notes de version v1.18.2 et v1.19.x (github.com/anomalyco/opencode).
