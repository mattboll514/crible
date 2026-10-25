# ADR-0058 — Contrôle anti-fuite de `@git` par un script versionné, sans fichier temporaire

- **Statut** : Accepté (décision du propriétaire le 2026-10-25)
- **Date** : 2026-10-25 (proposition et acceptation)
- **Décideurs** : propriétaire
- **Décision** : accepté le 2026-10-25 par le propriétaire, tel que proposé. Les trois points laissés à sa main sont tranchés ainsi :
  - (i) un `.env.prod` absent ou illisible donne le code `2` et l'arrêt, jamais un passage en silence ;
  - (ii) aucun seuil de longueur n'est appliqué aux valeurs comparées : un faux positif arrête le commit et se rapporte ;
  - (iii) le hook `pre-commit` de l'option (b) n'est **pas** retenu pour l'instant ; c'est un complément possible, à décider par un autre ADR.
- **Proposé par** : orchestrateur, sur demande du propriétaire (hors tâche, branche `phase/P3-cycle-de-vie`)
- **Remplace** : —

## Contexte

- **Le besoin.** ADR-0032 (décision 3) impose que `@git` vérifie, avant chaque commit, qu'aucune valeur de `.env.prod` ne se trouve dans ce qui est versionné. Aujourd'hui ce contrôle est une ligne de `.opencode/agents/git.md` (section « Contrôles avant chaque commit ») :

  ```sh
  git diff --cached | grep -F -f <(grep -E '^PROD_[A-Z_]+=' .env.prod | cut -d= -f2- | tr -d '"' | grep -v '^$')
  ```

  Elle repose sur une **substitution de processus** : `grep -F -f <(…)` demande au shell un descripteur temporaire. opencode v2 vérifie une commande composée morceau par morceau et refuse l'ensemble si un morceau est interdit (ADR-0026) ; cette forme a été refusée à `@git` dans certaines sessions, et le contrôle est passé dans d'autres.
- **Ce qui s'est passé quand elle a été refusée.** `@git` a reconstruit le même contrôle en écrivant les valeurs dans un fichier temporaire (`$TMPDIR/crible_prod_vals.txt`, `$TMPDIR/opencode/prodvals.txt`, `/tmp/…`). Ses permissions ne lui permettent pas de supprimer ce fichier (`rm` est refusé). Les valeurs confidentielles restent donc sur disque, hors du dépôt et hors de tout contrôle de version.
  - **2026-10-22**, commit d'ATTACHER-LOT : fichiers supprimés à la main par le propriétaire.
  - **2026-10-24**, commit d'AJOUTER-ORT-CI (`a000737`) : quatre fichiers, supprimés sans les lire par l'orchestrateur.
  - Sources : `docs/suivi.md` (section « Contenu de NETTOYER-P2 », point `@git` ; ligne ATTACHER-LOT de « Idées hors périmètre », avec la mention « Reproduit le 2026-10-24 ») et `docs/jalons/J2-rapport.md` (« Incident de confidentialité, à trancher par un ADR »).
- **Mesure provisoire, insuffisante par nature.** Chaque consigne à `@git` interdit désormais le fichier temporaire, et l'orchestrateur vérifie après chaque commit : `ls /tmp/*prod*val* $TMPDIR/*prod*val* $TMPDIR/opencode/*prod*val*`. C'est de la vigilance : elle dépend de la rédaction de la consigne et d'un contrôle humain après coup, pas d'une règle vérifiable par la machine.
- **Deux défauts du contrôle actuel, indépendants du fichier temporaire.**
  1. **Le message de commit n'est pas contrôlé.** Seul `git diff --cached` est inspecté. Un message qui contiendrait une valeur (par exemple un chemin, un hôte recopié dans un pied de message) passerait, alors qu'il est versionné au même titre que le diff.
  2. **En cas de résultat positif, la sortie contient la valeur.** `grep` affiche la ligne du diff qui a correspondu, donc la valeur elle-même, qui est alors renvoyée à l'agent et au modèle. Le contrôle protège le dépôt mais expose la valeur à la session.
- **Ce qui existe déjà.** `.opencode/agents/git.md` porte la liste de permissions de `@git` (`bash: "*": deny`, puis une liste d'utilitaires autorisés, `git *` autorisé, `git config*`, `rm` et les réécritures d'historique interdits). `.opencode/scripts/` n'existe pas encore. ADR-0037 a montré qu'élargir une liste de permissions se décide par un ADR ; c'est le cadre de celui-ci, avec ADR-0026 et ADR-0032.
- **Ce que la règle exige, et que le contrôle actuel ne garantit pas.** La règle de `AGENTS.md` et de `CLAUDE.md` est qu'aucune valeur de `.env.prod` ne soit recopiée dans un fichier versionné, un rapport, un ADR ou un commit. Les deux documents décrivent l'interdiction, pas le mécanisme : ils n'ont pas à changer. Le mécanisme, lui, doit rendre la fuite **impossible sans contournement délibéré**, et ne jamais écrire de valeur sur disque.

## Décision

1. **Un script versionné devient le seul moyen autorisé de contrôler la fuite.** Il vit dans `.opencode/scripts/check_prod_leak.sh`, en shell POSIX, avec le bit exécutable. `@git` reçoit la permission de lancer **ce seul script** (`./.opencode/scripts/check_prod_leak.sh*` et son chemin absolu, formes exactes à valider à l'implémentation : opencode compare la chaîne de commande) ; la ligne de contrôle actuelle est supprimée de `.opencode/agents/git.md`.
2. **Le script ne sait pas écrire.** Il lit `.env.prod` en mémoire, ne crée aucun fichier, n'utilise ni `mktemp`, ni `tee`, ni redirection vers un fichier, ni substitution de processus, ni `here-document` (plusieurs shells, dont bash et zsh, écrivent un `here-document` dans un fichier temporaire), et n'active jamais `set -x`. Il n'affiche jamais une valeur : sur un résultat positif, il n'imprime que le **nom** des variables trouvées (`PROD_SSH`, `PROD_PATH`…), jamais la valeur, ni la ligne du diff.
3. **Ce qu'il inspecte.**
   - `git diff --cached` : contenu indexé et chemins de fichiers, ce qui couvre ce que le commit va réellement enregistrer ;
   - le message de commit, transmis **en argument** au script, et non par substitution de processus ni par fichier temporaire ;
   - les valeurs non vides de `.env.prod` (lignes `^PROD_[A-Z_]+=`), guillemets doubles entourants retirés, `\r` de fin retiré, comparaison en chaîne fixe (`grep -F`), jamais en expression régulière.
4. **Codes de sortie et arrêt.** `0` : rien trouvé, le commit peut continuer. `1` : au moins une valeur trouvée, avec la liste des noms. `2` : contrôle impossible (`.env.prod` absent ou illisible, argument manquant, dépôt introuvable). Le code `2` **ne vaut pas succès** : `@git` s'arrête et rapporte le message exact.
5. **Règles explicites pour `@git`** dans `.opencode/agents/git.md` :
   - lire `.env.prod` par un autre moyen que ce script est interdit (ni `cat`, ni `grep`, ni `source`) ;
   - écrire dans un fichier une valeur issue de `.env.prod`, ou un fragment de valeur, est interdit, y compris « pour vérifier » ;
   - si le script échoue, est refusé par les permissions ou renvoie `2`, `@git` **s'arrête et rapporte** ; il ne cherche aucun contournement, ne recompose pas le contrôle à la main et n'utilise pas `--no-verify`.
6. **La vérification `ls` de l'orchestrateur disparaît** une fois l'ADR appliqué : elle n'a plus d'objet si le script ne peut pas écrire. `ADR-0032` n'est ni modifié ni remplacé : sa décision 3 est renforcée, pas contredite. Cet ADR ne tranche pas l'installation d'un hook `pre-commit` (option b) : c'est une décision séparée, à la main du propriétaire.

**Points laissés à la décision du propriétaire.** (i) Accepter qu'un `.env.prod` absent fasse échouer le commit (code `2`, arrêt) plutôt que de laisser passer sans contrôle : la proposition ci-dessus ferme la porte, au prix d'un `@git` inutilisable sur un clone sans `.env.prod`. (ii) Accepter ou non un seuil de longueur minimale des valeurs comparées : sans seuil, une valeur courte et banale (un mot d'un chemin, `on`) bloque un commit légitime ; avec seuil, une valeur courte n'est plus protégée. La proposition ne met **pas** de seuil et préfère le faux positif, qui s'arrête et se rapporte. (iii) Ajouter ou non le hook de l'option (b) en complément.

## Conséquences

- **Positives** :
  - aucune valeur de `.env.prod` n'a besoin de toucher le disque hors du dépôt : c'est la cause des deux incidents qui disparaît, pas seulement leur symptôme ;
  - le contrôle devient un fichier versionné, relu et testé, au lieu d'une ligne de consigne que chaque session peut réinterpréter ;
  - le message de commit est couvert, alors qu'il est versionné comme le diff ;
  - en cas de fuite, la sortie ne contient que des noms de variables : le contrôle n'expose plus la valeur à l'agent ni au modèle ;
  - un code de sortie distinct pour « contrôle impossible » empêche un passage silencieux.
- **Négatives / coûts** :
  - un fichier de plus à maintenir, avec son test, et une permission de plus dans `@git` : la surface de revue des règles s'allonge (même coût qu'ADR-0037) ;
  - un faux positif bloque un commit et n'affiche que le nom de la variable : il faut comprendre soi-même pourquoi la valeur correspond, ce qui coûte un aller-retour ;
  - la vérification `ls` disparaît : si un futur passage de `@git` réintroduisait un fichier temporaire, plus rien ne le signalerait mécaniquement ;
  - `git diff --cached` n'inspecte pas le contenu des fichiers binaires indexés (« Binary files … differ ») : une valeur enfouie dans un binaire ne serait pas vue. C'est une limite du contrôle actuel aussi ; elle est notée, pas fermée ici.
- **À surveiller** :
  - les refus de permission sur le script dans les journaux de `@git` (`permission.rejected`) : un refus répété signifierait que la forme de la commande ne correspond pas à la règle, et ramènerait le contournement ;
  - les faux positifs (commits arrêtés par le code `1`) : leur nombre et les variables en cause, pour décider d'un seuil de longueur ou d'un changement de valeur ;
  - l'absence de fichier temporaire sur les premiers commits après application : garder la vérification `ls` **une fois**, manuellement, puis la retirer ;
  - une valeur de `.env.prod` modifiée : le script relit le fichier à chaque commit, donc aucune mise à jour du contrôle n'est nécessaire ; vérifier seulement qu'aucune valeur n'est ajoutée au script lui-même ;
  - si l'option (b) est ajoutée : l'usage de `--no-verify` dans l'historique des commandes.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| (a) Script versionné, seul moyen autorisé | **Retenue.** Une seule règle vérifiable, aucun fichier, sortie sans valeur, message couvert, testable. |
| (b) Le même script en hook `pre-commit` (`core.hooksPath`) | Complément possible, pas remplaçant : `git config` est interdit à `@git`, donc le réglage dépend d'une action manuelle du propriétaire et ne suit pas un clone ; `--no-verify` le contourne ; il faudrait deux hooks (`pre-commit` pour le contenu, `commit-msg` pour le message) ; et rien ne garantit que `@git` l'ait réglé. |
| (c) Autoriser `@git` à supprimer son fichier temporaire | Traite le symptôme : les valeurs ont déjà été écrites sur disque, et l'autorisation `rm` élargit la surface de permissions au-delà de ce seul fichier. Ne couvre pas les autres formes de contournement. |
| (d) Statu quo : consigne et vérification `ls` | Deux incidents en trois jours, dont un reproduit malgré la consigne ; dépend de la vigilance ; ne couvre pas le message de commit ; en cas de résultat positif, affiche la valeur. |
| Passer en silence quand `.env.prod` est absent | Un commit non contrôlé passerait sans trace : c'est exactement la faille que l'ADR ferme. Mieux vaut un arrêt visible (code `2`). |
| Contrôler aussi le message avec `git log -1 --format=%B` après le commit | Trop tard : le message est déjà dans l'historique. Le contrôle doit précéder `git commit`. |

## Références

- `.opencode/agents/git.md` (section « Contrôles avant chaque commit », liste de permissions) ;
- `.env.prod.example` (noms des variables, format `^PROD_[A-Z_]+=`) ;
- ADR-0032 (accès à la production, valeurs hors git), ADR-0026 (permissions des agents), ADR-0037 (précédent d'élargissement de permissions) ;
- `docs/suivi.md`, section « Contenu de NETTOYER-P2 » (point `@git`) et ligne ATTACHER-LOT de « Idées hors périmètre » ;
- `docs/jalons/J2-rapport.md`, « Incident de confidentialité, à trancher par un ADR » ;
- `docs/09-plan-de-developpement.md`, section Git (commits conventionnels, pied de message).

## Mise en œuvre proposée

Une tâche **hors plan**, en P3, qui suit le pipeline complet (`@git debut-tache` → `@testeur` → `@dev` → `@relecteur` → suivi → commit → `fin-tache`). Le script est du code, donc il a un test.

### `DURCIR-CONTROLE-ANTI-FUITE`

- **Objectif.** Remplacer le contrôle anti-fuite de `@git` par un script versionné qui ne lit `.env.prod` qu'en mémoire, couvre le diff indexé et le message de commit, et n'affiche que des noms de variables.
- **Livrables.**
  1. `.opencode/scripts/check_prod_leak.sh` : shell POSIX, exécutable, codes de sortie `0`/`1`/`2` de la décision, aucune écriture, aucune valeur affichée.
  2. Son test, par exemple `.opencode/scripts/tests/check_prod_leak_test.sh` : dépôt git factice dans un répertoire temporaire, `.env.prod` factice avec des valeurs inventées.
  3. `.opencode/agents/git.md` : permission du seul script, section « Contrôles avant chaque commit » réécrite, interdictions explicites de la décision 5. Aucun autre changement dans ce fichier.
  4. Ligne de la tâche dans `docs/suivi.md` (tenue par l'orchestrateur) et retrait de la vérification `ls` dans les consignes qui la portent.
- **Critères d'acceptation.**
  1. Sur un `.env.prod` factice et un diff indexé contenant une de ses valeurs, le script sort `1`, n'affiche que le nom de la variable, et n'affiche jamais la valeur (vérifié par recherche de la chaîne factice dans la sortie).
  2. Même chose pour un message de commit passé en argument, sans diff fautif.
  3. Sur un diff et un message propres, il sort `0`.
  4. `.env.prod` absent ou illisible : code `2`, message explicite, aucune valeur.
  5. Après exécution, le répertoire temporaire pointé par `TMPDIR` est vide, et le script ne crée aucun fichier (aucun `mktemp`, `tee`, redirection d'écriture ni `here-document` dans le fichier, vérifié à la relecture).
  6. Les valeurs entre guillemets de `.env.prod` sont reconnues sans les guillemets (`PROD_PGOPTIONS` du modèle).
  7. `@git` ne peut plus lire `.env.prod` par une autre commande : la forme autorisée du script est la seule ligne ajoutée à ses permissions, et `rm`, `git config` et la substitution de processus restent hors de portée.
