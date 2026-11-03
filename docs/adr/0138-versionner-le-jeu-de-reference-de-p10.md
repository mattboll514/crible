# ADR-0138 — Versionner le jeu de référence de P10 (exception à ADR-0125, décision 16)

- **Statut** : Accepté (par anticipation le 2026-11-03, sur délégation du propriétaire du 2026-11-03 ; à confirmer par le propriétaire le 2026-11-04 au matin)
- **Date** : 2026-11-03 (proposition et acceptation anticipée)
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, tâche `CONSTITUER-JEU-DE-REFERENCE` (phase P10), sur l'écart 6 relevé par `@dev` dans `docs/legifacile/jeu-de-reference.md`
- **Remplace** : — (exception à ADR-0125, décision 16 ; complète ADR-0125 et ADR-0129 sans modifier leur corps)

La délégation du propriétaire est citée telle quelle, le 2026-11-03 : « on part sur l'option B » (enchaîner `DEFINIR-INDEX-LEGIFACILE` puis `CONSTITUER-JEU-DE-REFERENCE`, validation par anticipation) et « si tu doit ecrire des ADR considere qu'il sont accepter par anticipation, on les valide ensemble demain matin ». L'acceptation anticipée porte sur cet ADR seul ; le propriétaire le confirme ou le rejette le 2026-11-04.

## Contexte

### La règle existante et ce qu'elle protège

ADR-0125, décision 16 (`docs/adr/0125-commande-eval-mode-local-format-metriques-et-rapport.md:226-240`) tient deux règles d'un seul tenant : le rapport et le JSON n'identifient les requêtes **que par leur `id`**, sans texte de requête ni valeur de filtre, et « les fichiers d'entrée restent dans `data/` », gitignoré. La décision 3 (`:68-78`) fixe par ailleurs le format d'entrée, fermé ; `docs/06-observabilite.md:165` reprenait les deux règles et écrivait que le convertisseur laisse ses sorties sous `data/` (cette phrase porte désormais l'exception, voir « Documents mis à jour par cet ADR »).

La règle protège une chose précise : un texte de requête réel ne doit pas entrer dans git, où il serait ensuite indélébile — les règles git du projet interdisent la réécriture d'historique. Elle suit INV-13 et ADR-0032, et MES-10 n'identifiait déjà ses questions que par `id`.

### Ce que la fiche de P10 demande, et ce que la règle empêche

La fiche `CONSTITUER-JEU-DE-REFERENCE` (`docs/phases/P10-validation-a-l-echelle-reelle.md:77-87`) demande un jeu JSONL, **validé par le propriétaire**, **le même pour pgvector et pour Crible**. Sa note, reprise d'ADR-0129, ajoute que « le jeu de P10 sert aussi au projet Legifacile (nouveau dépôt), qui en garde la propriété après la passation ».

Les trois exigences portent sur le fichier du jeu lui-même, pas sur un rapport : le propriétaire valide un contenu, la comparaison à pgvector rejoue les mêmes lignes, et la passation transmet un artefact. Un jeu gitignoré ne satisfait aucune des trois : il n'est ni relu en diff, ni rejoué par la CI, ni transmis. Le conflit est donc réel, et il a été relevé par `@dev` comme écart 6 de `docs/legifacile/jeu-de-reference.md:213-219`.

### Ce qui a été livré, et sa provenance vérifiée

L'orchestrateur a fait placer le jeu sous `docs/legifacile/reference/` : six fichiers `<index>.jsonl` au format fermé d'ADR-0125, décisions 3 et 4 (`articles`, `jorf`, `decisions`, `titres`, `sarde`, `dole`), un fichier `mcp.jsonl` pour l'appel équivalent du serveur MCP, et le document de provenance `docs/legifacile/jeu-de-reference.md`. Deux tests de `crates/crible/src/eval.rs` (`reference_game`, `:2000-2187`) lisent ces fichiers par l'analyseur de `crible eval` lui-même, valident chaque filtre contre la configuration de son index, et vérifient qu'un `id` du jeu a exactement une ligne dans `mcp.jsonl`, et réciproquement.

La provenance a été relue pièce par pièce. **Aucune requête d'utilisateur final n'entre dans le jeu**, et les chiffres se tiennent :

- **97 requêtes** au total, réparties en 25, 10, 45, 3, 8 et 6 par fichier ; 97 `id` distincts sur l'ensemble, 97 lignes dans `mcp.jsonl`, et **35 requêtes filtrées**. Aucune ligne ne porte de `vector` : tout le jeu est textuel, donc lisible et rejouable sans modèle enregistré ;
- **57 questions** viennent de `~/MattCorp/legifacile-sandbox/docs/mcp/evaluation/questions.yaml`, décrit par `~/MattCorp/legifacile-sandbox/docs/mcp/EVALUATION.md`. Le fichier compte bien 57 `id: Q…`, `Q40` exceptée, et son en-tête dit sa méthode : chaque question a été écrite **après** qu'un document réel y répondant a été trouvé en base. `EVALUATION.md:1630` le redit : « Questions écrites en interne, pas par une juriste ». Ce sont des questions d'évaluation rédigées pour le projet, pas des requêtes d'usage ;
- **24 requêtes** reprennent le jeu de fidélité de MES-00 : ce sont exactement les 24 `query_id` distincts de `docs/mesures/MES-00-reference-pgvector.2026-10-20.fidelity-ef200.json`, trois par corpus. Leur texte est le **titre** du document, et `data/mes00/queries.json` le dit dans ses métadonnées : `origin: "article and summary titles, never a user question"`. Ces titres portent sur des documents publics ;
- **16 variantes** sont composées pour cette tâche à partir d'une question existante et d'une valeur de filtre sourcée ; le tableau de `docs/legifacile/jeu-de-reference.md:87-104` dit, pour chacune, la question de base, le filtre et la source de la valeur. Les textes des variantes filtrées sont ceux de leur question de base ; seule `Q02-VARIANTE-COURTE` porte une formulation citée par `EVALUATION.md`.

Le jeu ne contient donc que trois natures de texte : des questions d'évaluation écrites pour le projet, des titres de documents publics, et des variantes de ces questions. Les valeurs de filtre sont des métadonnées publiques du corpus — codes (`Code du travail`), états (`VIGUEUR`), dates, thèmes — et non des valeurs saisies par un utilisateur.

## Décision

**Le jeu de référence de P10 est versionné sous `docs/legifacile/reference/`, par exception à la clause « les fichiers d'entrée restent dans `data/` » d'ADR-0125, décision 16.**

Précisions :

1. **L'exception porte sur ce jeu et sur lui seul** : les six fichiers `<index>.jsonl`, `mcp.jsonl` et le document de provenance `docs/legifacile/jeu-de-reference.md`. Le reste d'ADR-0125 ne bouge pas : le format d'entrée reste le format fermé des décisions 3 et 4, `crible eval` lit ces fichiers sans code nouveau, et le format du rapport reste celui de la décision 11.
2. **L'exception ne vaut que pour un jeu sans requête d'utilisateur final.** Trois natures de texte y sont admises : les questions d'évaluation **écrites pour le projet**, les **titres ou textes de documents publics**, et les **variantes de ces questions**. Tout jeu qui contient un texte de requête tiré de journaux ou d'un usage réel reste sous `data/`, gitignoré. L'exception ne déplace rien d'autre : l'exemple de latence de MES-00, mille requêtes tirées de titres, reste sous `data/` comme entrée de mesure, et un futur jeu bâti sur `mcp.appels_outil` y restera aussi.
3. **Le reste de la décision 16 est inchangé** : les rapports et le JSON n'identifient les requêtes que par leur `id`, sans texte de requête ni valeur de filtre. AGENTS.md, règle 11, reste entier : ni les analytics ni les logs ne portent de contenu.
4. **Le document du jeu porte la provenance de chaque requête**, pour que la condition 2 soit vérifiable : le bloc source de chaque requête, et, pour une variante, la question de base et la source de la valeur de filtre. Un jeu versionné sans provenance ne serait pas couvert par cette exception.

### Format de `mcp.jsonl`

**Décision.** `docs/legifacile/reference/mcp.jsonl` porte **une ligne par `id` du jeu**, et rien de plus, avec **quatre membres fermés** : `id`, `tool`, `perimetre`, `params`. `tool` vaut `recherche_semantique`. `perimetre` prend l'une des huit valeurs du MCP. `params` n'ouvre que les noms de paramètres de filtre de `recherche_semantique`.

Les valeurs sont celles du serveur MCP, relues le 2026-11-03 en lecture seule dans `~/MattCorp/legifacile-sandbox/serveur_mcp/` :

- **`perimetre`**, les huit de `perimetres.py` (`PERIMETRES`) : `droit_applicable`, `droit_en_vigueur`, `droit_historique`, `jurisprudence`, `journal_officiel`, `index_thematique`, `dossiers_legislatifs`, `titres` ;
- **`params`**, les huit paramètres de filtre de `recherche_semantique` (`outils.py:873-905`) : `k`, `dedupliquer`, `partie_exclue`, `fonds`, `etat`, `annee_min`, `annee_max`, `cle_domaine`. `question` n'est pas un filtre, et `corpus` est **exclu** : le périmètre suffit, et la forme experte mono-corpus n'a pas le même sens pour les deux périmètres multi-corpus ;
- **les valeurs de `params` sont celles du jeu**, rendues explicites dans chaque ligne : `k` vaut 10, `dedupliquer` vaut `document`, et `partie_exclue` vaut `[]`, la valeur neutre choisie pour ce jeu et non le défaut du serveur, qui est `["moyens", "faits"]` (`PARTIE_EXCLUE_DEFAUT`). Les noms de `params` sont ceux du MCP, pas ceux des index : le document du jeu donne la correspondance (`code`, `nature`, `theme` et `type` pour `cle_domaine`, `section` pour `fonds`, `annee` pour `annee_min` et `annee_max`).

Le test `every_id_has_exactly_one_mcp_line_and_the_converse` (`crates/crible/src/eval.rs:2112-2186`) tient ce format : il refuse un membre de plus ou de moins, un `tool` autre que `recherche_semantique`, un `perimetre` hors des huit, un nom de `params` hors des huit, un `id` absent des six jeux et un `id` en double ; il exige aussi une ligne par `id` et un `id` par ligne. Il ne contrôle pas les **valeurs** de `params` : elles relèvent du document du jeu et de sa relecture.

### Ce qui n'est pas décidé ici

- La **passation** elle-même, et le fait que le projet Legifacile garde la propriété du jeu après elle : c'est la note d'ADR-0129, que cet ADR ne modifie pas.
- Le **sort des autres entrées** de P10 : l'échantillon, `data/mes00/`, `data/mes10/` et les jeux de latence restent sous `data/`, gitignorés.
- L'**emplacement futur** du jeu dans le nouveau dépôt Legifacile : il appartient à la passation.

### Documents mis à jour par cet ADR

- `docs/adr/README.md` : ligne d'index d'ADR-0138 ; note de renvoi sur la ligne d'ADR-0125 (« décision 16 : exception pour le jeu de référence de P10 »). Le fichier d'ADR-0125 n'est pas modifié, un ADR accepté ne se réécrit pas.
- `docs/06-observabilite.md:165` : la phrase « les fichiers d'entrée restent dans `data/` » nomme désormais l'exception et renvoie à cet ADR ; appliqué par `@greffier` le 2026-11-03, comme ADR-0137 l'a été à `docs/01:56` et `docs/04:41`.
- `docs/legifacile/jeu-de-reference.md` : le renvoi de l'écart 6 vers cet ADR est ajouté par `@dev`, dans le travail de `CONSTITUER-JEU-DE-REFERENCE` ; `@greffier` n'y touche pas.
- `docs/suivi.md` : la ligne d'écart de `CONSTITUER-JEU-DE-REFERENCE` renvoyant à cet ADR est ajoutée par l'orchestrateur, qui tient le suivi.

## Conséquences

- **Positives** :
  - le jeu devient un artefact relu, diffé, validé puis transmis : le propriétaire valide un contenu, non un fichier local ;
  - les deux tests de `reference_game` tournent en CI, puisque les fichiers sont présents dans l'arbre : un filtre hors grammaire ou un `id` sans ligne MCP casse la construction, au lieu d'attendre une campagne locale ;
  - pgvector et Crible rejouent le même fichier, et la comparaison de `COMPARER-AVEC-PGVECTOR` porte sur une entrée stable ;
  - la provenance est lisible à côté du jeu, ce qui rend la condition 2 vérifiable par un lecteur, pas seulement par celui qui l'a écrit ;
  - la passation transmet le jeu sans copie manuelle.
- **Négatives / coûts** :
  - **97 textes de requête entrent dans l'historique git, définitivement.** C'est acceptable ici parce que ce sont des questions internes, des titres publics et leurs variantes, et parce qu'aucune ne vient d'un usage réel ; une erreur de provenance deviendrait, elle, indélébile. C'est le prix de la décision ;
  - 35 valeurs de filtre deviennent versionnées. Ce sont des métadonnées publiques, mais un futur filtre portant sur une valeur sensible devrait être refusé par relecture ;
  - le test Rust lie la CI au contenu du jeu : une modification du jeu sans mise à jour de `mcp.jsonl` fait échouer la CI, ce qui est voulu, mais demande de traiter les deux fichiers ensemble ;
  - l'exception crée une asymétrie dans la documentation, entre `data/` pour les entrées de mesure et `docs/legifacile/reference/` pour ce jeu ; la décision 2 et le document de provenance sont ce qui la borne.
- **À surveiller** :
  - **toute requête du jeu dont la provenance se révélerait être un journal ou un usage réel** : elle sort du jeu et repart sous `data/`, et la question se rouvre par un ADR ;
  - une croissance du jeu par ajout de requêtes réelles — par exemple un jeu de latence tiré de `mcp.appels_outil` : il reste sous `data/`, et le document de provenance doit dire pourquoi une requête n'est pas versionnée ;
  - un échec de `reference_game` en CI qui signalerait un filtre ou un `mcp.jsonl` désynchronisé ;
  - l'usage du jeu après la passation : si Legifacile y ajoute des requêtes d'usage réel dans son dépôt, cette exception ne le couvre plus.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| **Laisser le jeu sous `data/`, gitignoré, comme le veut la décision 16** | Non versionné, le jeu n'est pas relu en diff : le test `reference_game` serait ignoré en CI, et la validation du propriétaire comme la passation à Legifacile reposeraient sur un fichier local, donc sur une copie manuelle et invérifiable. |
| **Versionner tout ce que P10 produit, échantillon et jeux de mesure compris** | L'échantillon, `data/mes00/` et `data/mes10/` sont dérivés de la production et volumineux : les versionner contredirait ADR-0032 et la règle « rien de volumineux dans git ». La condition 2 de cet ADR ne les couvrirait pas. |
| **Versionner un jeu réduit aux `id`, les textes restant sous `data/`** | Un `id` seul ne se valide pas et ne se rejoue pas : `crible eval` a besoin du texte ou d'un vecteur, et pgvector a besoin de la question. Ce serait un index de jeu, pas un jeu. |
| **Versionner le jeu sans document de provenance** | La condition 2 — aucune requête d'utilisateur final — ne serait vérifiable par personne. Le document de provenance est ce qui rend l'exception contrôlable, et il coûte une page. |
| **Modifier la décision 16 d'ADR-0125 sur place** | Un ADR accepté ne se réécrit jamais ; la seule modification permise est le passage en « Remplacé par ». Une exception datée, dans un ADR à part, garde l'historique et permet de la retirer sans toucher à ADR-0125. |
| **Ne versionner le jeu que dans le dépôt Legifacile, qui en garde la propriété** | Le nouveau dépôt n'existe pas encore et ne peut ni valider ni tester le jeu maintenant ; la propriété après passation (ADR-0129) ne dispense pas de le livrer ici, où la comparaison à pgvector se fait. |

## Références

- Règle amendée : `docs/adr/0125-commande-eval-mode-local-format-metriques-et-rapport.md:226-240` (décision 16), complétée par `:68-78` (décision 3, format fermé) et `:80-90` (décision 4, `expected` au document).
- Conception : `docs/06-observabilite.md:148-179` (chapitre « Éval »), notamment `:165` (fichiers d'entrée sous `data/`) et `:179` (rapport sans texte de requête).
- Confidentialité : ADR-0032 (accès à la production), INV-13, et AGENTS.md, règle 11.
- Plan : `docs/phases/P10-validation-a-l-echelle-reelle.md:77-87` (fiche `CONSTITUER-JEU-DE-REFERENCE`), ADR-0129 (replanification de la fin du plan, et sa note sur la propriété du jeu par le projet Legifacile).
- Travail d'origine : `docs/legifacile/jeu-de-reference.md:76-111` (provenance des requêtes), `:213-219` (écart 6), et `docs/legifacile/reference/` (six fichiers d'index, `mcp.jsonl`).
- Tests : `crates/crible/src/eval.rs:2000-2187` (`reference_game`), dont `every_reference_game_parses_and_every_filter_validates_against_its_index` et `every_id_has_exactly_one_mcp_line_and_the_converse`.
- Sources relues en lecture seule : `~/MattCorp/legifacile-sandbox/docs/mcp/evaluation/questions.yaml` (`:1-40` pour la méthode d'écriture, `:146` pour `Q01`, `:961` pour `Q30`), `~/MattCorp/legifacile-sandbox/docs/mcp/EVALUATION.md:1630` (« Questions écrites en interne ») et `:1709-1712` (les options d'un appel sont invisibles dans `mcp.appels_outil`), `docs/mesures/MES-00-reference-pgvector.2026-10-20.fidelity-ef200.json` (24 `query_id`), `data/mes00/queries.json` (métadonnée `origin`).
