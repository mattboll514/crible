# ADR-0024 — Plugins opencode : caveman (avec garde-fous) et notifications

- **Statut** : Accepté (décision du propriétaire le 2026-10-19)
- **Date** : 2026-10-19
- **Décideurs** : propriétaire
- **Proposé par** : propriétaire (caveman) ; sélection des autres plugins par l'assistant

## Contexte

Le propriétaire souhaite **caveman**, et demande quels autres plugins seraient utiles. Faits vérifiés au 19/10/2026 :

- **caveman** (github.com/JuliusBrussee/caveman, v4.0.1) a un support natif d'opencode.
  - Il rend la **prose de l'assistant télégraphique**, pour économiser des tokens. Le code, les commandes, les chemins et les messages d'erreur ne changent pas.
  - Son installateur ajoute :
    - un plugin et une entrée MCP dans la config **globale** ;
    - des fichiers de skill, d'agent et de commande ;
    - un bloc « toujours actif » dans l'`AGENTS.md` global, qui s'applique aussi aux sous-agents.
  - Ses règles coûtent environ 1 160 tokens d'entrée.
  - Sa télémétrie est **active par défaut**.
  - Sa passerelle ne couvre pas Ollama : seules les règles de style s'appliquent.
  - Le comportement en **français n'est pas documenté**.
  - `/caveman-commit` produit un commit d'une seule ligne, et `/caveman-compress` réécrit des fichiers de mémoire en les envoyant au modèle.
- **Notifications** : `@mohak34/opencode-notifier`, maintenu et compatible v1 et v2. Sur macOS, il passe par osascript.
- **Plugins évalués et écartés** : DCP (élagage de contexte), oh-my-opencode, opencode-supermemory, opencode-worktree.

## Décision

- Les plugins sont installés **dans la configuration globale** de l'utilisateur (`~/.config/opencode/`), **pas dans le `opencode.json` du projet** : ce sont des préférences personnelles, et la config du dépôt doit fonctionner sans eux.
- **caveman** : installé avec son installateur officiel (`--only opencode`, après un `--dry-run`), télémétrie **désactivée**. Garde-fous :
  - règle 9 d'`AGENTS.md` : le style concis ne s'applique **qu'aux messages de conversation**, jamais aux fichiers (code, documents, ADR, rapports, commits) ;
  - ne **pas** utiliser `/caveman-commit` : les commits passent par `@git`, avec le pied `Task:` et les références d'ADR ;
  - ne **pas** utiliser `/caveman-compress` sur `AGENTS.md`, `docs/` ou les ADR, qui doivent rester lisibles par des humains.
- **Notifications** : `@mohak34/opencode-notifier`, pour être prévenu quand une tâche longue se termine ou qu'une permission est demandée.

## Conséquences

- **Positives** : des réponses de chat plus courtes, donc moins de tokens en sortie ; pas besoin de surveiller l'écran pendant les tâches longues.
- **Négatives** :
  - ~1 160 tokens d'entrée de plus par session et par sous-agent ;
  - les comptes rendus des sous-agents deviennent télégraphiques ;
  - qualité en français inconnue.
- **À surveiller** :
  - que les fichiers écrits restent bien rédigés (le relecteur et l'agent jalon le voient) ;
  - la lisibilité des comptes rendus de l'orchestrateur.

  Si la qualité baisse, désactiver caveman (nouvel ADR).

## Alternatives écartées

| Plugin | Pourquoi écarté |
|---|---|
| DCP (`@tarquinen/opencode-dcp`) | Redondant avec les sessions enfants et `/new` ; invalide le cache de prompt ; AGPL ; développement ralenti |
| oh-my-opencode | Voir ADR-0023 : lourd, agents en parallèle, licence non standard |
| opencode-supermemory | Les données partent vers un service hébergé ; la mémoire du projet est dans les fichiers et les ADR |
| opencode-worktree | Archivé, non maintenu |

## Références

`docs/guide-opencode.md` ; `AGENTS.md` (règle 9) ; github.com/JuliusBrussee/caveman (`INSTALL.md`) ; github.com/mohak34/opencode-notifier ; opencode.ai/docs/plugins.
