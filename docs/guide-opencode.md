# Guide : installer opencode et travailler sur Crible

Pour le propriétaire et toute personne qui rejoint le projet. Les choix sont expliqués dans l'ADR-0023 (orchestration), l'ADR-0024 (plugins), l'ADR-0025 (opencode v2) et l'ADR-0033 (orchestrateur dans Claude Code). Les faits ont été vérifiés les 19 et 20/10/2026.

## 0. Le mode de travail actuel : Claude Code orchestre (ADR-0033)

- **Qui fait quoi** :
  - tu parles à l'orchestrateur dans **Claude Code**, sur Opus, avec ton abonnement Claude ;
  - il fait travailler les agents **opencode** sur DeepSeek (Ollama), sans interface, par `.claude/agent.sh`.

  Les sections 1 à 4 restent donc nécessaires : opencode, Ollama et ColGrep doivent être installés.
- **Lancer une session** :

  ```bash
  cd ~/MattCorp/crible
  claude            # le modèle (Opus) vient de .claude/settings.json
  ```

  Puis `/etat`. Les commandes sont les mêmes qu'avec opencode : `/tache`, `/jalon`, `/valider`, `/adr`, `/mesurer`, `/git`, `/relire` et `/chercher`.
- **Sessions** : `/clear` remplace le `/new` d'opencode. La stratégie de la section 6 ne change pas : une session par phase ou par journée.
- **Voir ce que fait un agent** :
  - `/tasks` dans Claude Code liste les agents qui tournent en arrière-plan ;
  - dans `opencode`, `/sessions` montre chaque passage d'agent. Ses sessions sont titrées « agent : consigne » ;
  - le journal brut est dans `.claude/runs/`.
- **Consommation** : `/usage` dans Claude Code pour les fenêtres de 5 heures et de 7 jours ; la page de ton compte ollama.com pour Ollama.
- **Secours** : si le quota Claude est épuisé, lance `opencode --auto` entre deux tâches. Son orchestrateur reprend à partir des mêmes fichiers (sections 5 à 7).

## 1. Installer opencode (v2)

```bash
brew install anomalyco/tap/opencode-v2
opencode --version          # doit afficher 2.x
```

- **La configuration du dépôt est volontairement au format v1** (`agent`, `provider`, `plugin`, permissions par outil). La v2 la lit et la convertit en mémoire, et elle reste utilisable en v1 (ADR-0025).
- **Différences v2 à connaître** :
  - pas de serveur de langage : les agents vérifient avec `cargo check`, `clippy` et `test` ;
  - les commandes déléguées (`/git`, `/relire`, `/chercher`) tournent en arrière-plan, puis rendent leur résultat ;
  - `@fichier` n'est pas développé dans les commandes.
- **Retour en v1 si besoin** : `brew install anomalyco/tap/opencode`. Rien à changer dans le dépôt.

## 2. Brancher Ollama et le modèle

```bash
ollama signin                                  # compte ollama.com, nécessaire pour les modèles cloud
ollama pull deepseek-v4.1-flash:1017-cloud     # ne télécharge que les métadonnées : le modèle tourne chez Ollama Cloud
ollama run deepseek-v4.1-flash:1017-cloud "Réponds OK"   # test rapide
```

- La connexion est **déjà décrite** dans `opencode.json`, à la racine du dépôt : fournisseur `ollama` sur `http://localhost:11434/v1`, modèle par défaut, limite de contexte. Rien à configurer de plus.
- Un modèle cloud utilise sa fenêtre maximale par défaut, ici 1 M tokens. Inutile de régler `OLLAMA_CONTEXT_LENGTH`, qui ne concerne que les modèles locaux.
- `ollama launch opencode` existe, mais il démarre opencode avec sa propre configuration en ligne. Pour Crible, lance simplement `opencode` à la racine du dépôt.

## 3. ColGrep et son agent (ADR-0022)

```bash
brew install lightonai/tap/colgrep    # déjà fait sur cette machine (1.8.5)
colgrep --install-agent               # une fois : télécharge le modèle local (~2,5 Go, Hugging Face)
```

⚠️ **Ne pas** lancer `colgrep --install-opencode`. Les consignes sont déjà dans `AGENTS.md`, versionnées avec le dépôt.

## 4. Plugins (ADR-0024), installés dans ta config globale

**caveman** : réponses de chat télégraphiques, pour moins de tokens.

```bash
npx -y github:JuliusBrussee/caveman -- --only opencode --dry-run   # voir d'abord ce qui sera modifié
npx -y github:JuliusBrussee/caveman -- --only opencode
caveman telemetry off && caveman telemetry status                  # la télémétrie est active par défaut
```

L'installateur écrit dans `~/.config/opencode/` :
- le plugin (`plugins/caveman/`) ;
- 8 commandes, 3 agents « cavecrew » et 9 skills ;
- l'entrée `"plugin": ["./plugins/caveman"]` dans `opencode.jsonc` (format v1, que la v2 lit) ;
- un bloc de règles dans l'`AGENTS.md` global.

Vérifié le 19/10/2026 sur opencode 2.0.32 : le plugin se charge sans erreur. Les règles de caveman précisent elles-mêmes que le code, les commentaires, les commits et les docs restent rédigés normalement. Désinstallation : `npx -y github:JuliusBrussee/caveman -- --uninstall`.

Garde-fous, déjà dans `AGENTS.md` (règle 9) :
- le style ne s'applique qu'au chat, jamais aux fichiers ;
- n'utilise pas `/caveman-commit` (les commits passent par `@git`) ;
- n'utilise pas `/caveman-compress` sur `AGENTS.md`, `docs/` ou les ADR.

**Notifications** : un message macOS quand une tâche se termine ou qu'une permission est demandée.

```bash
opencode plugin add @mohak34/opencode-notifier    # commande native v2 : ajoute « plugins » dans la config globale
```

Ensuite, dans Réglages Système › Notifications, autorise **Éditeur de script** (le plugin passe par osascript). Le plugin se configure dans `~/.config/opencode/opencode-notifier.json`.

**Vérifier que tout se charge** (depuis la racine du dépôt) :

```bash
opencode debug config     # sources de config lues : la config globale (plugins) et celle du dépôt
opencode debug agents     # nos 9 agents, avec leurs permissions converties
opencode run --standalone --print-logs --agent plan "Réponds OK" 2>&1 | grep -iE "loading plugin|level=(WARN|ERROR)"
```

Redémarre opencode après toute installation de plugin (`opencode reload` recharge la configuration).

## 5. Premier lancement

```bash
cd ~/MattCorp/crible
opencode
```

- L'agent actif est l'**orchestrateur** (agent par défaut). La touche Tab fait passer d'un agent principal à l'autre.
- Tape `/etat` : il résume le projet et propose la première tâche, `INITIALISER-WORKSPACE-RUST`.
- Tape `/tache INITIALISER-WORKSPACE-RUST`. `@git` doit créer `phase/P0-fondations` et la branche de tâche **sans te demander de permission**. Les règles d'un agent passent après les règles globales, et c'est la dernière règle qui correspond qui l'emporte. Seul un push te demandera confirmation.

### Mode `--auto` : travailler sans demandes d'autorisation (ADR-0026)

```bash
alias oc='opencode --auto'      # à mettre dans ~/.zshrc
cd ~/MattCorp/crible && oc
```

- `--auto` approuve automatiquement **tout ce qui n'est pas explicitement interdit**. Une règle `deny` n'est jamais levée.
- **Ce qui reste interdit, même en `--auto`** :
  - les commandes git qui modifient le dépôt, sauf pour `@git` ;
  - le push forcé, `reset --hard`, `rebase` ;
  - `find … -delete` et `find … -exec` ;
  - `brew`, `npm install -g`, `pip install` ;
  - les pipes vers un shell (`curl … | sh`) ;
  - `sudo`, `rm -rf /`, `rm -rf ~`.
- **Ce qui passe sans demande en `--auto`** : les autres commandes des agents (`python3`, `rm` dans le dépôt…), les accès hors du dépôt, les lectures web, et le push par `@git`. Ce dernier ne pousse que si tu le demandes dans le chat.
- **Sans `--auto`** (`opencode` seul), tu valides toi-même chacune de ces demandes. C'est utile pour une phase délicate.
- Une commande composée (`|`, `;`, `&&`) est vérifiée morceau par morceau : un seul morceau interdit fait tout refuser. Les utilitaires courants (`tail`, `echo`, `sort`…) sont autorisés à tous les agents pour éviter ces refus.
- **Pour repérer des rejets** :

  ```bash
  sqlite3 -readonly ~/.local/share/opencode/opencode.db "select count(*) from session_message where data like '%permission.rejected%'"
  ```

## 6. Comment travailler : sessions et fenêtre de contexte

**Le principe** : la mémoire du projet est dans les **fichiers** (`docs/suivi.md`, ADR, rapports, historique git), pas dans le chat. Une session ne coûte donc rien à jeter, alors qu'une compaction résume avec des pertes.

**Comment le contexte se répartit** :
- Tu parles à l'orchestrateur. Lui ne lit pas le code : il lance des sous-agents.
- **Chaque sous-agent tourne dans une session enfant, avec un contexte neuf**, et ne renvoie qu'un compte rendu court.
- Le gros du travail (lire la fiche, coder, tester) n'entre donc jamais dans ton chat. L'orchestrateur ne cumule que quelques résumés par tâche.

| Situation | Que faire |
|---|---|
| Début d'une phase (après `/valider`) | **`/new`**, puis `/etat`. Une session d'orchestrateur par phase |
| Nouvelle journée de travail | `/new`, puis `/etat` (ou `/sessions` pour reprendre la session de la veille si elle est courte) |
| Entre deux tâches d'une même phase | Rester dans la même session. Pas besoin d'en ouvrir une par tâche : les sous-agents ont déjà un contexte neuf |
| L'orchestrateur ralentit, se répète ou oublie des consignes, ou plus d'une vingtaine d'échanges | `/new`, puis `/etat`. L'état se recharge depuis les fichiers en quelques secondes |
| Une discussion en cours (un ADR à trancher…) que tu ne veux pas reprendre de zéro | `/compact` (ou `ctrl+x c`) |
| Compaction automatique | Filet de sécurité, vers 180 k tokens (`limit.context = 200000`). Si elle se déclenche, préfère ensuite un `/new` à la première occasion |
| Voir ce qu'a fait un sous-agent | `↓` (choix d'une session enfant), `←` et `→` pour passer de l'une à l'autre, `↑` pour revenir |
| Faire enchaîner plusieurs tâches | Dis à l'orchestrateur « enchaîne les tâches de la phase ». Il s'arrête de lui-même à chaque blocage, à chaque décision à prendre et à la fin de la phase |

**À éviter** :
- un seul chat pour tout le projet ;
- demander à l'orchestrateur de coder lui-même ;
- lancer deux tâches en parallèle ;
- valider un jalon sans avoir rejoué la démo.

## 7. Le cycle complet d'une phase

```
/new → /etat → /tache NOM (… autant de fois que nécessaire, ou « enchaîne »)
     → /jalon J<n> → lire docs/jalons/J<n>-rapport.md, rejouer la démo
     → /valider J<n> → confirmer le push → /new pour la phase suivante
```

Les décisions qui te reviennent :
- accepter ou rejeter les ADR `Proposé` (« j'accepte ADR-00xx ») ;
- valider les jalons ;
- confirmer les push.

## 8. Dépannage

| Symptôme | Cause probable / solution |
|---|---|
| Le modèle n'apparaît pas dans `/models` | `ollama pull deepseek-v4.1-flash:1017-cloud` n'a pas été fait, ou Ollama ne tourne pas |
| Les appels d'outils échouent avec un modèle **local** | Contexte Ollama trop petit : `OLLAMA_CONTEXT_LENGTH=65536` (ou `launchctl setenv …` pour l'app Mac). Ne concerne pas les modèles cloud |
| `@git` demande des permissions | Voir la section 5 : la permission de l'agent ne prime pas sur la globale. Ajuster `opencode.json` |
| `/chercher` répond « indisponible » | `colgrep --install-agent` n'a pas été fait, ou le dépôt ne contient pas encore de code |
| Les comptes rendus deviennent illisibles | Effet de caveman : le désactiver (« stop caveman ») ou le désinstaller, puis le signaler dans un ADR |
