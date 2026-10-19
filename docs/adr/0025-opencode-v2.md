# ADR-0025 — Utiliser opencode v2, avec une configuration au format v1

- **Statut** : Accepté (décision du propriétaire le 2026-10-19)
- **Date** : 2026-10-19
- **Décideurs** : propriétaire
- **Proposé par** : propriétaire (le site d'opencode recommande la v2) ; analyse par l'assistant
- **Remplace** : ADR-0023, point 5 seulement (« opencode v1 »). Le reste de l'ADR-0023 est inchangé.

## Contexte

L'ADR-0023 avait retenu opencode **v1**, pour deux raisons : notre configuration est écrite au format v1, et la v2 n'exécute pas les plugins v1. Or le site d'opencode recommande désormais la v2 (`brew install anomalyco/tap/opencode-v2`). Vérifications faites le 19/10/2026 dans la documentation de migration v1 → v2 :

- **Configuration** : la v2 **lit `opencode.json` au format v1 tel quel**. Elle convertit en mémoire les anciennes clés (`agent`, `provider`, `plugin`, permissions par outil) sans réécrire le fichier, et elle traduit automatiquement l'en-tête de nos fichiers d'agents.
- **Commandes** :
  - `$ARGUMENTS`, `$1` et `` !`commande` `` sont pris en charge ;
  - `subtask` reste accepté comme alias de `subagent` ;
  - en revanche, `@fichier` **n'est plus développé** dans les modèles de commande. C'est corrigé dans `/etat`, qui demande maintenant de lire le fichier.
- **Sécurité** : la v2 autorise par défaut l'accès aux fichiers hors du projet, alors que la v1 demandait confirmation. C'est rétabli par `"external_directory": "ask"` dans `opencode.json`.
- **Plugins** :
  - les plugins écrits pour la v1 ne tournent pas en v2 ;
  - le mécanisme principal de caveman (son bloc de règles dans `AGENTS.md`) a été testé par ses auteurs sur opencode 2.0.22, mais la compatibilité de son plugin natif n'est pas documentée ;
  - le plugin de notifications (`@mohak34/opencode-notifier`) annonce la compatibilité v1 et v2.
- **Différences de comportement** :
  - la v2 **ne fait pas tourner de serveurs de langage** (pas de diagnostics rust-analyzer) ; nos agents s'appuient déjà sur `cargo check`, `clippy` et `test` ;
  - les commandes déléguées tournent en arrière-plan ;
  - la compaction fonctionne par points de reprise ;
  - la profondeur des sous-agents se règle toujours par `experimental.subagent_depth`.
- **Maturité** : la documentation ne dit pas si la v2 est stable. Le paquet AUR s'appelle `opencode-beta`. La v1 reste maintenue (v1.19.3 le 16/10/2026).

## Décision

- **Installer opencode v2** (`brew install anomalyco/tap/opencode-v2`), comme le recommande le site.
- **Garder la configuration du dépôt au format v1.** La v2 la lit, et elle fonctionne aussi avec la v1, ce qui permet de revenir en arrière ou à quelqu'un de rester en v1. La conversion au format natif v2 se fera dans un futur ADR, si la v2 abandonne la lecture du format v1.
- **Deux ajustements** sont déjà appliqués, et compatibles avec les deux versions : `"external_directory": "ask"`, et `/etat` sans `@fichier`.

## Conséquences

- **Positives** : on suit la version recommandée et activement développée, et on évite une migration en cours de projet.
- **Négatives** :
  - une version plus jeune, peut-être encore en bêta ;
  - pas de serveur de langage ;
  - le plugin natif de caveman n'est peut-être pas actif, même si ses règles dans `AGENTS.md` le sont.
- **À surveiller**, dès la première tâche :
  - les avertissements de configuration au démarrage ;
  - les permissions de `@git` ;
  - le déclenchement de la compaction ;
  - le comportement des commandes déléguées (`/git`, `/relire`, `/chercher`).

  En cas de blocage, revenir à la v1 (`brew install anomalyco/tap/opencode`), sans rien changer à la configuration.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Rester en v1 (ADR-0023, point 5) | Stable, mais impose une migration plus tard, alors que le site recommande déjà la v2 |
| Convertir tout de suite la configuration au format natif v2 | Coupe la compatibilité avec la v1 pour aucun gain immédiat, puisque la v2 lit déjà le format v1 |

## Références

opencode.ai/v2/docs, opencode.ai/v2/docs/migrate-v1, opencode.ai/v2/docs/commands ; github.com/JuliusBrussee/caveman (`INSTALL.md`) ; ADR-0023, ADR-0024.
