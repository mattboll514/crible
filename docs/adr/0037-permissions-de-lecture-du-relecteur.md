# ADR-0037 — Élargir les permissions de `@relecteur` aux utilitaires de vérification et à Python

- **Statut** : Accepté
- **Date** : 2026-10-21 (proposition et acceptation)
- **Décideurs** : propriétaire
- **Proposé par** : propriétaire, relayé par l'orchestrateur ; rédaction par l'agent greffier
- **Remplace** : —

## Contexte

`@relecteur` est un agent en lecture seule : son en-tête de permissions (`.opencode/agents/relecteur.md`) pose `bash: "*": deny`, puis autorise une liste de commandes. ADR-0026 a fixé le principe : opencode v2 vérifie une commande composée **morceau par morceau**, et un seul morceau interdit fait refuser l'ensemble (ADR-0026 l. 12). Une commande absente de la liste est donc rejetée sans demande, même si elle est inoffensive.

**Le constat, chiffré.** Dans les journaux `.claude/runs/*relecteur*.jsonl` des 2026-10-20 et 2026-10-21, on compte de 1 à 14 commandes refusées par relecture, sur 10 relectures. Les commandes absentes de la liste sont :

- `awk`, `nl`, `shasum` et `openssl dgst`, pour les sommes de contrôle et la mise en forme ;
- `git ls-files`, `git check-ignore`, `git -C …` ;
- `uv run --directory …`, `uv run python …` et `python3 …`, pour les scripts de vérification (`-c` ou fichier temporaire) ;
- `timeout … uv run pytest`, `xargs` ;
- les structures shell `if …`.

**La conséquence.** Le relecteur ne peut pas recalculer lui-même un résultat : refaire le top-k de l'oracle `tools/reference_topk.py` par force brute, ou vérifier une somme blake3. Il se contente de lire le code et perd des tours à contourner les refus. Il a toutefois rendu des verdicts justes à chaque fois : le problème est le coût et la profondeur de la vérification, pas la qualité du verdict.

**Ce qui pèse.** Le pipeline de tâche (ADR-0023) place la relecture après `@dev`, sur des tâches où une erreur de référence se propage loin : l'oracle de `RECHERCHER-DENSE-EXACT` (P1), les mesures de P2 et de P4. Une relecture qui ne peut que relire le code ne voit pas une erreur de calcul. À l'inverse, `edit: deny` protège le relecteur des modifications directes, mais n'empêche pas un script d'écrire un fichier ; ADR-0026 l. 42 admet déjà ce trou pour `echo … >`.

## Décision

**Option B : ajouter les utilitaires de vérification, puis l'exécution Python, avec un garde-fou.**

1. **Utilitaires en lecture** : `awk *`, `nl *`, `shasum *`, `git ls-files*`, `git check-ignore*`, `git -C * diff*`, `git -C * log*`, `git -C * status*`, `git -C * show*`, `uv run --directory * pytest*`, `timeout * uv run pytest*` et `xargs *`.
2. **Exécution Python** : `uv run python *` et `python3 *`.
3. **Garde-fou** : l'orchestrateur compare `git status --short` avant et après chaque relecture, et signale au propriétaire toute différence.
4. **`xargs` sous condition** : `xargs *` n'est autorisé que si opencode garantit que la commande enchaînée est elle aussi contrôlée par les règles. Sinon, il est écarté de la liste. La vérification se fait avant la mise à jour de l'en-tête.

Ce qui n'est **pas** décidé ici : `edit: deny` reste en place, et le refus de `webfetch` aussi. Aucun accès réseau, aucune commande git qui modifie le dépôt, aucune installation logicielle n'est ajoutée. Le relecteur ne reçoit pas non plus un shell libre.

ADR-0026 n'est **pas** modifié : il est complété par cet ADR, qui ne fait que préciser la liste de `@relecteur`.

## Application

La vérification du point 4 a été faite le 2026-10-21 : la condition n'est **pas** remplie. opencode vérifie une commande composée morceau par morceau, en coupant sur `|`, `;`, `&&` et `||` (ADR-0026 l. 12) ; la commande lancée par `xargs` fait partie de ses arguments, donc le motif `xargs *` l'accepterait quelle qu'elle soit. **`xargs` est donc écarté** : l'en-tête de `.opencode/agents/relecteur.md` reçoit les douze autres motifs du point 1, plus les deux du point 2.

## Conséquences

- **Positives** :
  - le relecteur vérifie lui-même au lieu de lire : top-k de l'oracle recalculé, sommes blake3 contrôlées, résultats de mesure reproduits ;
  - moins de tours perdus en contournements, et des relectures plus courtes ;
  - le gain porte sur les tâches où une erreur se propage (oracle, mesures de P1, P2, P4).
- **Négatives / coûts** :
  - le relecteur peut exécuter du code arbitraire, donc écrire ou supprimer des fichiers. `edit: deny` ne couvre pas les écritures faites par un script : la lecture seule ne repose plus que sur sa consigne, comme `echo … >` l'admettait déjà (ADR-0026 l. 42) ;
  - `awk` peut écrire un fichier, pour la même raison ;
  - la liste de permissions s'allonge, donc la revue des règles devient plus difficile.
- **À surveiller** :
  - le garde-fou `git status` doit rester sans différence ; une différence est un incident, signalé au propriétaire ;
  - compter à nouveau les `permission.rejected` des journaux de relecture : la cible est zéro refus sur les commandes listées ;
  - si une écriture par script survient, resserrer vers l'option A.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| A. Utilitaires en lecture seuls, sans Python | Le relecteur ne peut toujours pas recalculer l'oracle ni vérifier une mesure : le gain reste cosmétique sur les tâches de référence et de mesure |
| C. Statu quo | 1 à 14 refus par relecture sur 10 relectures, et des contournements qui coûtent des tours sans renforcer la vérification |
| Autoriser `bash: "*": allow` au relecteur | Supprimerait aussi les `deny` ciblés et ouvrirait toute commande : trop loin pour le besoin mesuré |
| Autoriser un shell libre (`sh -c …`) | opencode ne contrôlerait plus les morceaux de la commande ; contraire au principe d'ADR-0026 |

## Références

- `.opencode/agents/relecteur.md` (en-tête de permissions) ;
- ADR-0026 (permissions des agents, vérification morceau par morceau) ; ADR-0023 (pipeline de tâche) ; ADR-0033 (orchestrateur dans Claude Code) ;
- journaux `.claude/runs/*relecteur*.jsonl` des 2026-10-20 et 2026-10-21 ;
- `tools/reference_topk.py` ; `docs/09-plan-de-developpement.md` (règles des agents).
