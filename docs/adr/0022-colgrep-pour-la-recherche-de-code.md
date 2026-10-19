# ADR-0022 — ColGrep et ColGrep-Agent pour la recherche de code par les agents

- **Statut** : Accepté
- **Date** : 2026-10-19
- **Décideurs** : propriétaire
- **Proposé par** : propriétaire

## Contexte

Les agents de développement tournent sur un modèle rapide (DeepSeek V4.1 Flash via Ollama), dont le contexte est précieux. Chercher du code avec `grep` en ouvrant des fichiers entiers consomme beaucoup de tokens.

LightOn publie deux outils :
- **ColGrep** (v1.8.5, déjà installé via Homebrew) : une recherche de code sémantique et locale, avec un index stocké hors du dépôt et le `.gitignore` respecté ;
- **ColGrep-Agent** (`colgrep --agent`, sorti le 12/10/2026) : un petit modèle local (MiniCPM5-2B, GGUF de ~2,5 Go, via llama.cpp sur Metal) qui cherche à la place du gros modèle. Il ne renvoie que des emplacements `fichier:début-fin`.

Mesures publiées par LightOn :
- dans Claude Code : −34 % de tokens et environ −40 % de coût ;
- 80,8 % de bonnes réponses contre 83,3 % avec grep, un écart **non significatif** ;
- mais une latence plus élevée (21,6 s contre 13,0 s par question).

Sur un M3 Pro, c'est environ 20 s par question en Metal, avec 3 à 4 Go de RAM au pic.

`colgrep --install-opencode` se contente d'ajouter la notice de ColGrep au fichier **global** `~/.config/opencode/AGENTS.md`, sans versionnement avec le projet. La délégation automatique par hooks n'existe que pour Claude Code.

## Décision

- **Intégrer ColGrep au niveau du dépôt** :
  - des consignes dans `AGENTS.md`, section « Recherche dans le code » ;
  - la permission `colgrep *` dans `opencode.json` et dans les agents en lecture seule ;
  - une commande `/chercher` qui interroge ColGrep-Agent.
- **Partage des outils** :
  - `colgrep "<intention>"` (avec `-e` pour une regex, `--include` pour un type de fichier) pour la recherche sémantique ;
  - `colgrep --agent "<question>"` pour les questions de comportement (« où est géré X ? ») qui demanderaient d'ouvrir plusieurs fichiers ;
  - `rg` pour les chaînes exactes : noms de tâches, codes d'erreur, `INV-07`.
- **Ne pas lancer** `colgrep --install-opencode`. Cela ferait doublon avec les consignes du dépôt, sans être versionné.
- Le modèle de l'agent (2,5 Go) est **installé une fois par le propriétaire**, avec `colgrep --install-agent`.

## Conséquences

- **Positives** : moins de contexte consommé par les agents ; des consignes versionnées, les mêmes pour tous ceux qui clonent le dépôt ; tout reste local, aucun code ne quitte la machine.
- **Négatives** :
  - environ 20 s par appel à `--agent`, avec 3 à 4 Go de RAM au pic ;
  - le premier index se construit en 30 à 90 s ;
  - l'agent renvoie moins de fichiers que ColGrep seul (~1,7 contre ~9,8), au risque d'être incomplet sur une question qui touche plusieurs fichiers ;
  - les consignes restent indicatives : le modèle peut les ignorer.
- **À surveiller** : la consommation de tokens et le taux de tâches réussies des agents, avant et après (relevés de façon informelle dans les rapports de jalon). Si l'agent ralentit sans gain visible, se limiter à ColGrep seul (nouvel ADR).

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| `colgrep --install-opencode` (global) | Non versionné, invisible pour qui clone le dépôt, doublon avec `AGENTS.md` |
| Seulement `grep` et `rg` | Plus de tokens consommés, recherche par intention impossible |
| Un sous-agent opencode dédié qui appelle ColGrep-Agent | Ajoute une couche de LLM inutile : l'agent principal peut appeler `colgrep --agent` directement |

## Références

- `AGENTS.md` (Recherche dans le code) ; `.opencode/commands/chercher.md`.
- https://github.com/lightonai/next-plaid (ColGrep) ; https://huggingface.co/blog/lightonai/colgrep-agent
