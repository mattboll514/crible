# ADR-0033 — L'orchestrateur tourne dans Claude Code (Opus), les agents restent dans opencode (DeepSeek via Ollama)

- **Statut** : Accepté (décision du propriétaire le 2026-10-20)
- **Date** : 2026-10-20
- **Décideurs** : propriétaire
- **Proposé par** : propriétaire (Opus 5.5 comme orchestrateur, DeepSeek sur Ollama pour tous les agents) ; analyse et mise en place par l'assistant
- **Remplace** : ADR-0023, points 1 et 7, pour la place de l'orchestrateur seulement. Le pipeline, l'agent git unique, la revue obligatoire et les limites du modèle des agents sont inchangés.

## Contexte

- **Le besoin.** Le propriétaire veut un orchestrateur plus solide qu'un modèle « flash » : Claude Opus 5.5, avec son abonnement Claude. Il veut garder DeepSeek V4.1 Flash, servi par Ollama, pour tous les agents qui font le travail.
- **Ce qu'on a vérifié le 2026-10-20 dans la documentation de Claude Code** (code.claude.com) :
  - un sous-agent de Claude Code ne peut pas utiliser un autre fournisseur que la session principale : le point d'accès au modèle est commun à tout le processus ;
  - la documentation de la passerelle (llm-gateway) précise que le routage vers des modèles qui ne sont pas des modèles Claude n'est pas pris en charge ;
  - brancher Claude Code sur le point d'accès compatible Anthropic d'Ollama basculerait toute la session, orchestrateur compris.
- **Utiliser l'abonnement Claude dans opencode** n'est pas autorisé pour un outil tiers. La voie officielle serait une clé d'API facturée au token, ce que le propriétaire a écarté.
- **Ce qu'on a vérifié sur opencode 2.0.32, sans interface** (`opencode run`) :
  - `--agent` accepte un agent de mode `subagent`, avec ses propres permissions ;
  - `--auto` approuve ce qui n'est pas interdit. Une commande interdite échoue avec `Permission denied` sans bloquer la session ;
  - `--format json` produit un flux d'événements, dont on extrait le compte rendu final ;
  - `--session <id>` reprend une session avec son contexte ;
  - la session est enregistrée comme une session opencode normale, avec l'agent et le modèle utilisés (`ollama/deepseek-v4.1-flash:1017-cloud`).

## Décision

1. **L'orchestrateur est une session Claude Code** lancée à la racine du dépôt.
   - Le modèle est fixé par `.claude/settings.json` (`"model": "opus"`).
   - Ses consignes sont dans `CLAUDE.md`. Ce fichier importe `AGENTS.md` et renvoie à `.opencode/agents/orchestrateur.md`, qui reste la **définition unique** du rôle, du pipeline et des interdits. `CLAUDE.md` ne contient que les adaptations à Claude Code.
2. **Les agents ne changent pas** : mêmes définitions dans `.opencode/agents/`, mêmes permissions (ADR-0026), même modèle.
   - L'orchestrateur les lance avec `.claude/agent.sh <agent>`, c'est-à-dire `opencode run --auto --agent <agent> --format json`.
   - Le script ne rend que le compte rendu final, la liste des commandes refusées et une ligne de pied (session, durée, journal). Le contexte d'Opus reste ainsi léger.
   - Le journal complet va dans `.claude/runs/`, non versionné. La session est aussi visible dans opencode.
   - Les agents longs (`dev`, `testeur`, `mesureur`, `jalon`) tournent en arrière-plan. En session interactive, Claude Code ne limite pas la durée d'une commande en arrière-plan.
3. **Les commandes** (`/etat`, `/tache`, `/jalon`, `/valider`, `/adr`, `/mesurer`, `/git`, `/relire`, `/chercher`) sont des skills de `.claude/skills/`.
   - Chacune renvoie à son fichier `.opencode/commands/<nom>.md`, qui reste la définition unique.
   - Elles ne peuvent être lancées que par le propriétaire (`disable-model-invocation`).
4. **Garde-fous de l'orchestrateur**, dans `.claude/settings.json` :
   - les commandes git qui modifient le dépôt sont interdites : elles passent par `@git` ;
   - la modification du code (`crates/`, `tools/`, `clients/`) est interdite ;
   - `.env.prod` ne peut être ni lu ni modifié (ADR-0032) ;
   - l'outil `Agent` est interdit. Les sous-agents de Claude Code consommeraient l'abonnement Anthropic, alors que tout le travail délégué va aux agents opencode.

   Seule l'écriture de `docs/suivi.md` est autorisée d'office.
5. **Secours** : l'orchestrateur opencode reste en place. On peut passer de l'un à l'autre entre deux tâches, par exemple quand le quota Claude est épuisé, puisque l'état du projet est entièrement dans les fichiers.

## Conséquences

- **Positives** :
  - un orchestrateur plus fiable pour suivre le pipeline, arbitrer les relectures et préparer les décisions du propriétaire ;
  - le travail lourd reste sur Ollama : la consommation Claude se limite à la coordination ;
  - aucune duplication : le pipeline et les commandes restent définis à un seul endroit.
- **Négatives / coûts** :
  - deux outils à installer et à maintenir (Claude Code et opencode) ;
  - on ne suit plus un agent en direct depuis la conversation. On le voit dans opencode (`/sessions`) ou dans son journal ;
  - sans interface, toute demande d'autorisation est approuvée (`--auto`) : les interdictions de l'ADR-0026 sont le seul garde-fou des agents. Le push par `@git` n'est retenu que par la consigne de l'orchestrateur : jamais sans confirmation du propriétaire ;
  - l'orchestration consomme le quota Claude (fenêtres de 5 heures et de 7 jours).
- **À surveiller** :
  - la consommation Claude d'une tâche type (`/usage`) ;
  - les comptes rendus d'agents hors format, ou les consignes trop pauvres pour un agent qui part d'un contexte neuf ;
  - les commandes refusées que liste `.claude/agent.sh`.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Sous-agents de Claude Code sur DeepSeek | Impossible : le fournisseur est commun à toute la session, et la passerelle vers des modèles non-Claude n'est pas prise en charge |
| Claude Code entièrement sur Ollama | On perd Opus pour l'orchestrateur |
| Abonnement Claude dans opencode | Non autorisé pour un outil tiers |
| Clé d'API Anthropic dans opencode | Facturation au token ; écartée par le propriétaire |
| Garder l'orchestrateur DeepSeek | Statu quo ; ne répond pas au besoin |

## Références

ADR-0023, ADR-0025, ADR-0026, ADR-0032 ; `CLAUDE.md` ; `.claude/settings.json` ; `.claude/agent.sh` ; `.claude/skills/` ; `docs/guide-opencode.md`. Documentation Claude Code : sub-agents, llm-gateway, settings, permissions, skills, memory, tools-reference (code.claude.com/docs). `opencode run --help` (v2.0.32).
