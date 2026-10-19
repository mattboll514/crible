# Crible — règles pour les agents

Crible est un moteur de recherche **dense + BM25** pour données structurées, écrit en **Rust** (serveur) avec des producteurs et un client en **Python**. Une instance par projet, CPU seulement, exact par défaut sur les pools filtrés, débuggable par construction. Le premier projet servi est Legifacile (8,58 M passages). La conception est **terminée** et figée dans `docs/`, et chaque décision est tracée dans un ADR (`docs/adr/`). Le développement suit un plan de 10 phases (P0 → P9) et 83 tâches, piloté par l'agent **orchestrateur** (ADR-0023).

## Où trouver quoi

| Besoin | Fichier |
|---|---|
| Vocabulaire (à employer tel quel) | `docs/00-glossaire.md` (déjà chargé dans ta session) |
| La fiche d'une tâche | `docs/phases/P<n>-*.md`, section `### NOM-DE-TACHE` |
| État des tâches, jalons, cibles, écarts, idées | `docs/suivi.md` (tenu par l'orchestrateur) |
| Règles du plan, git, format des fiches | `docs/09-plan-de-developpement.md` |
| Conception (lire seulement ce que la fiche cite) | `docs/01` à `docs/08` |
| Invariants INV-xx et mesures MES-xx | `docs/08-invariants-et-tests.md` |
| **Décisions** : ADR-xxxx et questions ouvertes Q-xx | `docs/adr/README.md` (index et processus), puis un fichier par décision |

**Économise ton contexte** : ne lis que ce dont ton rôle a besoin. Pour chercher dans le code, suis la section « Recherche dans le code » plus bas.

## Règles d'or

1. **Une tâche à la fois**, pilotée par l'orchestrateur, qui fait passer chaque tâche par un pipeline fixe :
   `@git debut-tache` → `@testeur` (si la tâche cite des invariants) → `@dev` → `@relecteur` → suivi → `@git commit` → `@git fin-tache`.

   Une tâche ne démarre que si :
   - le jalon précédent est `VALIDE` (sauf pour P0) ;
   - ses dépendances sont `FAIT` ;
   - les ADR qu'elle cite sont `Accepté`.
2. **La fiche fait foi pour le périmètre.** Ne fais rien de plus que la fiche. Une bonne idée hors fiche se signale dans ton compte rendu ; l'orchestrateur la note dans « Idées hors périmètre ».
3. **La conception et les ADR acceptés font foi.** Si une tâche est impossible, contredit `docs/01` à `08` ou un ADR `Accepté`, ou demande un choix que rien ne tranche, arrête-toi et remonte une **`QUESTION-CONCEPTION`** à l'orchestrateur. Il la soumet à `@architecte`, fait rédiger un ADR `Proposé` par `@greffier`, puis bloque la tâche.

   Personne ne modifie un ADR accepté ni un document de conception sans décision du propriétaire.
4. **Toute décision qui engage le projet passe par un ADR** : nouvelle dépendance, changement de format, seuil, valeur par défaut, compromis. Une décision qui n'existe que dans une session est perdue à la session suivante.
5. **Les tests d'abord.** Chaque critère d'acceptation a au moins un test. Chaque invariant cité dans la fiche a un test nommé `inv_xx_<description>`. On ne désactive jamais un test ni un lint pour « faire passer ».
6. **Définition de FAIT** :
   - `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` et `cargo test` passent (plus `uv run pytest` si du Python est touché) ;
   - `@relecteur` a rendu le verdict `OK` ;
   - le suivi est à jour ;
   - `@git` a commité, puis fusionné la tâche dans la branche de phase.
7. **Mesurer avant d'affirmer.** Aucune affirmation de performance sans un rapport de `@mesureur` dans `docs/mesures/`.
8. **Langue.** Le code, les identifiants, les commentaires, les messages de commit et l'API sont en **anglais**. Les documents de `docs/` (ADR compris) sont en **français**.
9. **Style.** Si un plugin de style concis (caveman) est actif, il ne s'applique **qu'aux messages de conversation**. Les fichiers restent rédigés normalement, en phrases complètes : code, commentaires, documents, ADR, rapports et messages de commit. Ce sont des documents lus par des humains qui découvrent le projet.
10. **Robustesse.**
    - Pas de `unwrap()`, `expect()` ni `panic!` hors des tests. Les erreurs passent par `CribleError` et `ErrorCode`.
    - Pas de `unsafe` sans un commentaire `// SAFETY:` et un test.
    - Les formats sur disque sont **little-endian explicites** et **versionnés**.
11. **Confidentialité.**
    - Un id interne (`Iid`) ne sort jamais du moteur.
    - Les analytics et les logs ne contiennent jamais de texte de requête ni de valeur de filtre.
12. **Simplicité.**
    - Un module = une responsabilité.
    - Pas d'abstraction pour une seule implémentation, pas de configuration pour une valeur fixe.
    - Une dépendance absente de `docs/07` demande un ADR.
13. **Si tu bloques** deux fois sur le même problème : arrête-toi et décris précisément le problème (`STATUT : BLOQUE`).
14. **Données.** Rien de volumineux dans git. Les jeux générés, les échantillons et les modèles vont dans `data/` (gitignoré).

## Git (voir `docs/09-plan-de-developpement.md`)

- **Seul l'agent `@git` modifie le dépôt** : branches, commits, fusions, tags, push. Les autres agents n'utilisent git qu'en lecture (`status`, `diff`, `log`, `show`).
- `main` ne reçoit que des phases validées. La fusion se fait par `@git fin-phase`, **uniquement sur instruction explicite du propriétaire**.
- La branche de phase s'appelle `phase/P<n>-<slug>`. La branche de tâche s'appelle `tache/NOM-DE-TACHE`.
- **Commits conventionnels** en anglais, avec la tâche en pied (`Task: NOM`) et les ADR concernés (`Refs: ADR-xxxx`).
- **Interdits** : push sans confirmation du propriétaire, push forcé, `reset`, `rebase`, `--amend`, et toute réécriture d'historique.

## Recherche dans le code (ADR-0022)

| Besoin | Outil | Exemple |
|---|---|---|
| Chaîne exacte : nom de tâche, code d'erreur, `INV-07`, identifiant | `rg` | `rg -n "ErrorCode::InvalidFilter"` |
| Code par intention, quand on ne connaît pas le nom exact | `colgrep` | `colgrep "atomic segment write" --include "*.rs" -k 10` |
| Regex combinée à une intention | `colgrep -e` | `colgrep -e "fn .*commit" "generation commit protocol"` |
| Fichiers seulement | `colgrep -l` | `colgrep -l "roaring bitmap filters"` |
| Question de comportement qui demanderait d'ouvrir plusieurs fichiers (« où est géré X ? ») | `colgrep --agent` | `colgrep --agent "where are tombstones applied during attach" --color never` |

Règles :
- **`colgrep --agent`** fait tourner un petit modèle local (~20 s par question). Il renvoie des emplacements `fichier:début-fin`. N'ouvre ensuite que ces passages.
- **Ne lance jamais colgrep en arrière-plan.** La première requête construit l'index (30 à 90 s), les suivantes prennent quelques secondes.
- **Pour la documentation** (`docs/`), `rg` suffit.

## Commandes utiles

```
cargo build                                   # compiler
cargo test                                    # tous les tests
cargo test -p crible-store <filtre>           # tests ciblés
cargo fmt --check && cargo clippy --all-targets -- -D warnings
cargo build --release                         # obligatoire pour toute mesure
cd clients/python && uv sync && uv run pytest # Python
```

## Agents et commandes opencode

| Agent | Rôle |
|---|---|
| `orchestrateur` (principal, par défaut) | Discute avec le propriétaire, tient le suivi, fait exécuter le pipeline de chaque tâche. Ne code pas |
| `@git` | Seul agent qui modifie le dépôt git |
| `@dev` | Implémente une tâche : tests, code, vérifications. Rend un compte rendu avec un message de commit proposé |
| `@testeur` | Écrit les tests (les invariants d'abord). Ne touche pas au code de production |
| `@relecteur` | Revue de code : diff de la tâche, verdict `OK` ou `A-CORRIGER`. Lecture seule |
| `@architecte` | Répond aux questions de conception en citant la doc et les ADR. Lecture seule |
| `@greffier` | Rédige les ADR (`Proposé`). Applique un ADR accepté sur demande du propriétaire |
| `@mesureur` | Exécute une mesure MES-xx et écrit le rapport |
| `@jalon` | Prépare le rapport de validation d'un jalon. Ne valide jamais |

| Commande | Effet |
|---|---|
| `/etat` | Fait le point et propose la prochaine tâche |
| `/tache NOM-DE-TACHE` | Exécute la tâche avec le pipeline complet |
| `/jalon J<n>` | Prépare le rapport de validation du jalon |
| `/valider J<n>` | Le propriétaire valide le jalon : `VALIDE`, fusion dans `main`, tag, push proposé |
| `/adr <sujet>` | Fait rédiger et committer un ADR `Proposé` |
| `/mesurer MES-xx` | Exécute une mesure et la commite |
| `/chercher <question>` | Localise du code avec ColGrep-Agent (lecture seule) |
| `/relire` | Revue de code du travail en cours par `@relecteur` |
| `/git <opération>` | Opération git directe (`etat`, `pousser main`…) |
