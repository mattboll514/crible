# ADR-0026 — Permissions des agents : utilitaires autorisés, interdictions renforcées, mode `--auto`

- **Statut** : Accepté (décision du propriétaire le 2026-10-19)
- **Date** : 2026-10-19
- **Décideurs** : propriétaire
- **Proposé par** : propriétaire (éviter les demandes d'autorisation) ; analyse et correction par l'assistant

## Contexte

- **Le besoin.** Le propriétaire veut travailler sans valider chaque autorisation. opencode v2 propose `opencode --auto`, qui approuve automatiquement **toute permission qui n'est pas explicitement interdite**. Une règle `deny` n'est jamais levée.
- **Ce qu'ont montré les premières sessions.**
  - Opencode v2 vérifie une commande composée (`|`, `;`, `&&`, `||`) **morceau par morceau**, et un seul morceau interdit fait refuser l'ensemble.
  - Les agents restreints (`"*": deny` : orchestrateur, git, architecte, relecteur, greffier) n'autorisaient pas `tail`, `echo`, `sort`…
  - Des commandes banales ont donc été **rejetées sans demande**, par exemple `colgrep init 2>&1 | tail -20` ou `git log … || echo …`. Les agents ont perdu des tours à les contourner.
- **Le risque de `--auto`.** Il approuverait aussi ce qui était seulement « demandé », notamment un commit lancé par `@dev`, alors que seul `@git` doit commiter (ADR-0023).

## Décision

1. **Une liste d'utilitaires sans danger**, autorisée à tous les agents, y compris les restreints :
   - `echo`, `printf`, `head`, `tail`, `sort`, `uniq`, `cut`, `tr`, `wc`, `du`, `which`, `diff`, `stat`, `jq`, `sed -n`, `cd`, `pwd`, `true`, `date` ;
   - plus `ls`, `cat`, `grep`, `rg`, `find`, `colgrep`.
2. **Les interdictions deviennent des `deny`**, pour que `--auto` ne puisse jamais les lever :
   - **les commandes git qui modifient le dépôt** (`add`, `commit`, `merge`, `switch`, `checkout`, `restore`, `reset`, `rebase`, `cherry-pick`, `revert`, `tag`, `push`, `stash`, `clean`, `rm`, `mv`, `branch -d/-D/-m`, `config`, `filter-branch`) sont interdites à tous les agents **sauf `@git`**, qui les réautorise dans ses propres règles. Pour `@git` lui-même, le push reste « demandé », et le push forcé, `reset`, `rebase` et `--amend` restent interdits ;
   - `find … -delete` et `find … -exec` ;
   - l'installation de logiciels sur la machine : `brew`, `npm install -g`, `pip install` ;
   - un shell lancé seul (`sh`, `bash`, `zsh`), ce qui bloque les `curl … | sh` ;
   - et les interdictions existantes : `sudo`, `rm -rf /`, `rm -rf ~`, `docker push`.
3. **Usage recommandé** :
   - `opencode --auto` pour le travail courant (`alias oc='opencode --auto'`) ;
   - `opencode` sans option quand le propriétaire veut tout contrôler.

   En `--auto`, ce qui reste approuvé sans intervention : les commandes inconnues de `@dev` et des agents non restreints (par exemple `python3`, `rm` dans le dépôt, `curl` sans pipe vers un shell), les accès hors du dépôt, les lectures web, et le push par `@git`. Ce dernier reste gardé par sa règle : il ne pousse que sur demande explicite du propriétaire dans le chat.

## Conséquences

- **Positives** :
  - plus de rejets silencieux sur les commandes composées ;
  - plus aucune demande d'autorisation en `--auto` ;
  - les garde-fous structurels (git réservé à `@git`, pas d'installation système, pas de pipe vers un shell) tiennent même en `--auto`.
- **Négatives** :
  - en `--auto`, une commande destructrice qui n'est pas listée (un `rm` à l'intérieur du dépôt) passe sans demande ;
  - `echo … > fichier` permet techniquement à un agent en lecture seule d'écrire un fichier : seule sa consigne l'en empêche.
- **À surveiller** : des rejets encore présents dans les sessions (`permission.rejected` dans la base locale d'opencode), et des commandes inattendues dans les sessions des sous-agents. Toute nouvelle règle passe par un ADR.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| `"*": "allow"` dans le `opencode.json` du dépôt | Imposerait le mode permissif à toute personne qui clone le dépôt ; `--auto` est un choix explicite à chaque lancement |
| « Allow always » au fil des demandes | Les règles sont stockées dans la base locale d'opencode, pas dans le dépôt : invisibles pour les autres et non revues |
| Garder `--auto` sans changer les règles | `@dev` pourrait commiter, et les rejets silencieux sur les commandes composées resteraient |

## Références

`opencode.json` ; `.opencode/agents/*.md` ; ADR-0023 ; opencode.ai/v2/docs/permissions (« The last matching rule wins », « any deny denies the operation ») ; `opencode --help` (`--auto`).
