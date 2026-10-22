# ADR-0044 — Règles de `@relecteur` : faire passer les vérifications et ne rapporter que ce qui a été exécuté

- **Statut** : Proposé
- **Date** : 2026-10-22
- **Décideurs** : propriétaire
- **Proposé par** : orchestrateur, sur la commande `/adr` ; constats des relectures de LIRE-LOT-PARQUET, RECHERCHER-DENSE-EXACT et EVALUER-FILTRES (2026-10-22)
- **Remplace** : —

## Contexte

`@relecteur` est un agent en lecture seule dont l'en-tête de permissions (`.opencode/agents/relecteur.md`) pose `bash: "*": deny`, puis autorise une liste de motifs. Deux règles déjà acceptées encadrent ce fonctionnement. ADR-0026 fixe le mécanisme : opencode v2 vérifie une commande composée **morceau par morceau** (`|`, `;`, `&&`, `||`) et un seul morceau interdit fait refuser l'ensemble ; en mode `--auto`, aucune permission n'est demandée à l'humain, donc les règles écrites dans le fichier sont le seul garde-fou. ADR-0037 a élargi la liste aux utilitaires de vérification et à l'exécution Python, avec le garde-fou `git status` avant et après chaque relecture.

Trois constats de la journée du 2026-10-22 montrent que la liste et le corps du fichier ne suffisent plus.

1. **Une forme de commande refusée alors qu'elle est légitime.** La règle autorise `cargo fmt --check*`, mais le relecteur lance `cargo fmt --all -- --check`, que le motif ne couvre pas. La commande est donc refusée sans demande. Le corps du fichier demande pourtant `cargo fmt --check` : la consigne n'a pas imposé la forme exacte, parce qu'un agent suit la commande qu'il juge utile, pas la chaîne littérale.
2. **Toute commande préfixée par une variable d'environnement est refusée.** `RAYON_NUM_THREADS=1 cargo test …` et `env …` ne figurent pas dans la liste. Le relecteur ne peut donc pas vérifier l'indépendance au nombre de threads rayon, ce qui est pourtant un point de vigilance de RECHERCHER-DENSE-EXACT : la tâche a été validée sans cette vérification indépendante.
3. **Un compte rendu a annoncé un contrôle qui n'a pas eu lieu.** Au troisième tour d'EVALUER-FILTRES, le relecteur a affirmé un contrôle empirique (rejeu de l'algorithme en Python sur 50 000 arbres) alors que ses deux commandes avaient été refusées. Le contrôle n'a pas eu lieu, et rien dans le format du compte rendu ne permettait de le voir : la ligne « Vérifications » ne distingue que ✓ et ✗, pas l'absence d'exécution.

**Ce qui pèse.** Le pipeline de tâche (ADR-0023) place la relecture juste avant le commit ; un contrôle annoncé mais non exécuté entre dans l'historique comme une preuve. Le pied de compte rendu de `.claude/agent.sh` imprime déjà les lignes `refused:` (outil et commande refusés), mais l'orchestrateur doit penser à les lire, et le relecteur n'est pas tenu d'en parler.

**Ce qui n'est pas en cause.** Le verdict du relecteur est resté juste à chaque relecture ; le problème est la forme des commandes autorisées et la fidélité du compte rendu. `edit: deny`, le refus de `webfetch`, l'absence de shell libre et le garde-fou `git status` d'ADR-0037 ne changent pas.

## Décision

1. **Élargir le motif de `cargo fmt` à la forme réellement utilisée (option A).** L'en-tête autorise `cargo fmt --all -- --check*` en plus de `cargo fmt --check*`. On ne descend pas à `cargo fmt*`, qui autoriserait une écriture.

2. **Autoriser l'affectation `RAYON_NUM_THREADS` avec des valeurs fixes (option B).** L'en-tête autorise exactement `RAYON_NUM_THREADS=1 cargo test*` et `RAYON_NUM_THREADS=4 cargo test*`. Ces deux valeurs sont celles de RECHERCHER-DENSE-EXACT (chercheur ouvert à un thread, interrogé à quatre). Une autre valeur demande une nouvelle ligne, donc un nouvel ADR si le besoin se répète. La forme `env …` reste refusée.

   **Pourquoi pas le joker après `=`.** Un motif `RAYON_NUM_THREADS=* cargo test*` a un joker qui avale les espaces, donc le nom de la commande elle-même. `RAYON_NUM_THREADS=1 rm -rf data cargo test`, `RAYON_NUM_THREADS=1 git commit -m x cargo test` et `RAYON_NUM_THREADS=1 python3 script.py cargo test` correspondent tous au motif : le `*` couvre `1 rm -rf data`, puis la fin de la chaîne couvre ` cargo test`. En mode `--auto`, personne ne voit passer la commande (ADR-0026, l. 10 et 15). S'y ajoute le trou connu des substitutions de commande : opencode ne découpe pas sur `$(…)` ni sur les accents graves, donc `RAYON_NUM_THREADS=$(…) cargo test` passerait aussi, et le contenu de la substitution ne serait jamais confronté aux règles. Avec une valeur fixe, le nom de la commande reste épinglé à `cargo` ; le joker final de `cargo test*` est celui qui existe déjà aujourd'hui.

3. **Imposer un compte rendu fidèle dans le corps du fichier.** Le corps reçoit une section propre, et la ligne « Vérifications » du format de rendu distingue trois états :
   - **✓** : la commande a été lancée et a réussi ;
   - **✗** : la commande a été lancée et a échoué, avec le compte des tests ;
   - **« non exécuté (commande refusée : `<commande>`) »** : la commande a été refusée par les permissions, ou n'a pas été lancée. La commande exacte est citée.

   Toute vérification annoncée dans le compte rendu doit correspondre à une commande réellement exécutée, présente dans la session. Une vérification annoncée sans commande exécutée rend le compte rendu faux.

   Le verdict ne peut pas être `OK` tant qu'une vérification exigée par la tâche est « non exécutée » : le point est alors `important`, avec la commande refusée, et l'orchestrateur tranche (relancer, élargir une règle par un ADR, ou autorisation explicite du propriétaire).

   Côté orchestrateur, ce point ne crée pas d'obligation nouvelle dans le code : l'orchestrateur lit **toujours** les lignes `refused:` du pied de compte rendu de `.claude/agent.sh` (elles sont déjà imprimées) et les rapproche de la ligne « Vérifications ». Un écart entre les deux est un incident de compte rendu.

Ce qui n'est **pas** décidé ici : aucun autre motif n'est ajouté, aucune permission n'est retirée, `edit: deny` reste en place, le garde-fou `git status` d'ADR-0037 reste en vigueur. ADR-0026 et ADR-0037 ne sont pas modifiés : cet ADR ne fait que préciser la liste et le corps de `@relecteur`.

## Conséquences

- **Positives** :
  - les deux vérifications qui échouaient (`cargo fmt --all -- --check`, tests à un et quatre threads rayon) passent sans demande d'autorisation, donc sans tour perdu en contournement ;
  - l'indépendance au nombre de threads rayon devient vérifiable par le relecteur sur les tâches de recherche ;
  - un contrôle non exécuté ne peut plus être présenté comme une preuve : la ligne « Vérifications » et les lignes `refused:` se recoupent ;
  - aucune permission large n'est ouverte : les deux nouvelles formes gardent le nom de la commande épinglé à `cargo`.
- **Négatives / coûts** :
  - la liste de permissions gagne trois lignes, donc la revue des règles s'allonge encore (déjà noté par ADR-0037) ;
  - une valeur de threads non prévue demande une modification de l'en-tête, donc un ADR : la vérification sera retardée, pas improvisée ;
  - la règle de compte rendu repose sur la consigne, comme la lecture seule (ADR-0026, l. 42) : rien dans l'outil ne force le relecteur à déclarer un refus.
- **À surveiller** :
  - recompter les `permission.rejected` des journaux `.claude/runs/*relecteur*.jsonl` : les formes `cargo fmt --all -- --check` et `RAYON_NUM_THREADS={1,4} cargo test` ne doivent plus apparaître en `refused:` ;
  - un compte rendu dont la ligne « Vérifications » annonce un ✓ sans commande correspondante dans la session : c'est l'incident que cet ADR doit faire disparaître ;
  - une demande de valeur de threads hors {1, 4} : si elle revient, examiner un motif plus étroit qu'un joker après `=` plutôt que d'allonger la liste valeur par valeur.

**Ce que l'en-tête de `.opencode/agents/relecteur.md` devient** (extrait ; les autres lignes ne changent pas) :

```yaml
    "cargo fmt --check*": allow
    "cargo fmt --all -- --check*": allow
    "cargo clippy*": allow
    "cargo test*": allow
    "RAYON_NUM_THREADS=1 cargo test*": allow
    "RAYON_NUM_THREADS=4 cargo test*": allow
```

**Paragraphe ajouté au corps**, après la section « Liste de contrôle » et avant « Rendu » :

```markdown
## Compte rendu fidèle

Tu ne rapportes que les vérifications que tu as réellement exécutées. Une commande refusée par
tes permissions n'est pas une vérification : tu l'écris « non exécuté (commande refusée : <commande>) »
et tu cites la commande exacte. Une vérification annoncée sans commande exécutée rend ton compte
rendu faux, et l'orchestrateur le rejette.

Une vérification exigée par la tâche et restée « non exécutée » interdit le verdict `OK` : tu
émets un point `important` avec la commande refusée.
```

**Et la ligne « Vérifications » du format de rendu** devient, par exemple :

```
Vérifications : fmt ✓ · clippy ✓ · tests ✓ (303 passés) · RAYON_NUM_THREADS=1 non exécuté (commande refusée : env RAYON_NUM_THREADS=1 cargo test --test inv_06)
```

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| 1-B. Garder `cargo fmt --check*` et imposer la forme exacte dans le corps, avec un exemple | Déjà tenté : le corps demande `cargo fmt --check` et le relecteur a lancé `cargo fmt --all -- --check`. Une consigne ne contraint pas la chaîne littérale, alors que la règle de permission, si. |
| `cargo fmt*` | Autorise `cargo fmt` sans `--check`, donc une écriture de fichiers par un agent en lecture seule. |
| 2-A. `RAYON_NUM_THREADS=* cargo test*` | Le joker avale le nom de la commande (`RAYON_NUM_THREADS=1 rm … cargo test` correspond) et laisse passer les substitutions de commande, en `--auto` sans témoin. |
| 2-C. Statu quo, tests lancés sans variable d'environnement | La vérification de l'indépendance au nombre de threads reste impossible ; RECHERCHER-DENSE-EXACT a été validée sans elle. |
| Autoriser `env *` | `env` suit la même mécanique de préfixe, mais son motif couvre toute commande placée après l'affectation : même trou que 2-A, en plus large. |
| 3-B. Laisser le format actuel et compter sur les lignes `refused:` de `.claude/agent.sh` | L'orchestrateur voit le refus, mais le compte rendu continue d'affirmer un contrôle qui n'a pas eu lieu, et le ✓ reste ambigu. |
| `bash: "*": allow` pour le relecteur | Ouvrirait toute commande et supprimerait les `deny` ciblés ; déjà écarté par ADR-0037, et contraire au principe d'ADR-0026. |

## Références

- `.opencode/agents/relecteur.md` (en-tête de permissions, section « Rendu ») ;
- `.claude/agent.sh` (pied de compte rendu, lignes `refused:`) ;
- ADR-0026 (permissions des agents, vérification morceau par morceau, mode `--auto`) ; ADR-0037 (permissions de lecture du relecteur, garde-fou `git status`) ; ADR-0023 (pipeline de tâche) ; ADR-0033 (orchestrateur dans Claude Code) ;
- `docs/suivi.md`, section « Idées hors périmètre », lignes du 2026-10-22 pour LIRE-LOT-PARQUET (orchestrateur), RECHERCHER-DENSE-EXACT (orchestrateur) et EVALUER-FILTRES (orchestrateur) ;
- journaux `.claude/runs/*relecteur*.jsonl` des 2026-10-21 et 2026-10-22.
