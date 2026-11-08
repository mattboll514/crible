# Jeu de référence de Legifacile

Ce document décrit le jeu de requêtes qui tranche la v1 de Crible sur la base réelle de
Legifacile (`CONSTITUER-JEU-DE-REFERENCE`, P10, jalon J10), et qui sert ensuite au projet
Legifacile, qui en garde la propriété après la passation (ADR-0129, décision 7). Il est le même
pour pgvector et pour Crible : chaque requête porte un identifiant, un texte, un filtre Crible
facultatif et les documents attendus, et chaque identifiant a exactement une ligne dans
`mcp.jsonl`, l'appel équivalent du serveur MCP.

Les fichiers sont dans `docs/legifacile/reference/` :

| Fichier | Index | Requêtes | Avec `expected` | Avec `filter` |
|---|---|---:|---:|---:|
| `articles.jsonl` | `articles` | 25 | 25 | 25 |
| `jorf.jsonl` | `jorf` | 10 | 10 | 3 |
| `decisions.jsonl` | `decisions` | 45 | 45 | 4 |
| `titres.jsonl` | `titres` | 3 | 3 | 0 |
| `sarde.jsonl` | `sarde` | 8 | 8 | 2 |
| `dole.jsonl` | `dole` | 6 | 6 | 1 |
| **Total** | — | **97** | **97** | **35** |

Chaque fichier est directement utilisable par
`crible eval --data <dossier> --index <index> --queries docs/legifacile/reference/<index>.jsonl --models <dir> --out <dossier>`.
`--out` est **obligatoire** : la commande y écrit ses deux rapports, qu'elle n'écrase jamais.
`--models` l'est aussi pour une campagne `dense`, le mode par défaut, sur un jeu textuel ; seule une
campagne `lexical` s'en passe.

## Format des deux fichiers

### `<index>.jsonl`

C'est le format fermé d'ADR-0125, décisions 3 et 4, tel que le lit `crates/crible/src/eval.rs` :
une ligne par requête, un objet JSON qui porte `id`, puis exactement un de `query` ou `vector`,
puis `filter` et `expected` facultatifs. **Aucune autre clé n'apparaît** ; une clé inconnue fait
refuser la ligne par la commande, avec son numéro.

- `id` est stable et unique sur **l'ensemble** du jeu, pas seulement dans un fichier : `Q01` à
  `Q58` pour les questions de `questions.yaml` (`Q40` n'existe pas, cf. sa source), `MES00-qNNNN`
  pour la campagne de fidélité de MES-00, et `<id-de-question>-<SUFFIXE>` pour les variantes
  composées.
- `query` est toujours textuel : aucune ligne ne porte de `vector`, pour que le jeu reste lisible
  et rejouable sans modèle enregistré.
- `filter` suit la grammaire de `docs/05-api.md` et la correspondance de
  `docs/legifacile/index-legifacile.md`. La forme `$or` y est celle de `partie_exclue`, et les
  listes du MCP y sont des `in` ou des `nin` (ADR-0137).
- `expected` est une liste de `doc_id` externes, jamais un `iid` ni une clé `doc_id#ord` : la
  qualité se mesure au document (ADR-0125, décision 4). Les `doc_id` ont la forme de l'index, par
  exemple `LEGIARTI…`, `JURITEXT…`, `SARDBLOC…` et `JORFDOLE…`.

Les deux tests de `crates/crible/src/eval.rs` (`reference_game`) chargent chaque fichier par
`read_queries`, l'analyseur de `crible eval` lui-même, puis valident chaque `filter` par
`Filter::from_json_str` et `Filter::validate` contre la configuration de son index, lue par
`IndexConfig::from_json_str` : c'est le chemin réel de l'étape 1 du pipeline. Ils vérifient aussi
que chaque `id` des six fichiers a exactement une ligne dans `mcp.jsonl`, et réciproquement.

### `mcp.jsonl`

Une ligne par `id`, et quatre membres seulement. Ce format — les quatre membres fermés et les noms
de `params` — est fixé par ADR-0138, au même titre que le versionnement du fichier :

```json
{"id": "Q01", "tool": "recherche_semantique", "perimetre": "droit_en_vigueur", "params": {"k": 10, "dedupliquer": "document", "partie_exclue": []}}
```

- `tool` vaut toujours `recherche_semantique` : c'est la branche que Crible remplace, et la seule
  que `COMPARER-AVEC-PGVECTOR` confronte à la recherche dense et hybride.
- `perimetre` est le périmètre du MCP du tableau des périmètres de
  `docs/legifacile/index-legifacile.md` : `droit_applicable`, `droit_en_vigueur`,
  `droit_historique`, `jurisprudence`, `journal_officiel`, `index_thematique`,
  `dossiers_legislatifs` ou `titres`. Aucune ligne ne porte `corpus` : le périmètre suffit, et la
  forme experte mono-corpus n'aurait pas le même sens pour les deux périmètres multi-corpus.
- `params` ne porte que les paramètres de filtre du MCP, avec les valeurs du jeu rendues explicites :
  `k` vaut 10, `dedupliquer` vaut `document` (le regroupement par document que `crible eval`
  applique, ADR-0125, décision 4), et `partie_exclue` vaut `[]`, la valeur neutre **choisie pour ce
  jeu** — « tout servir » —, et non le défaut du serveur, qui est `["moyens", "faits"]`
  (`serveur_mcp/outils.py`). Les variantes qui éprouvent ce défaut portent
  `partie_exclue: ["moyens", "faits"]`.
- Les filtres de variante se traduisent nom pour nom : `code`, `nature` (index `jorf`), `theme` et
  `type` sont le `cle_domaine` du MCP, `section` est son `fonds`, et `annee` ses `annee_min` et
  `annee_max`.

## Provenance des requêtes

Aucune ligne ne vient d'un journal de production, et aucune ne recopie une requête d'un
utilisateur final. Les trois blocs sont les suivants.

| Bloc | Requêtes | Source |
|---|---:|---|
| `Q01`–`Q58` | 57 | `docs/mcp/evaluation/questions.yaml@2026-08-07` du dépôt Legifacile, décrit par `docs/mcp/EVALUATION.md` ; jeu d'évaluation interne, rédigé par le propriétaire. Le texte est repris tel quel, et `expected` est le `src_id` de chaque attendu. `Q30` est la question non satisfiable, comptée à part. |
| `MES00-qNNNN` | 24 | Jeu de fidélité de MES-00 : les 24 `query_id` du fichier `MES-00-reference-pgvector.2026-10-20.fidelity-ef200.json`, trois par corpus. Le texte est le titre du document, et `expected` est son `source_id`, tous deux relus dans `data/mes00/queries.json`. |
| Variantes composées | 16 | Formulées pour cette tâche, à partir d'une question existante et d'une valeur de filtre sourcée ; le tableau ci-dessous dit laquelle. |

| `id` | Question de base | Filtre | Valeur et source |
|---|---|---|---|
| `Q02-VARIANTE-COURTE` | `Q02` | `etat` (périmètre) | Texte de la variante courte citée par `EVALUATION.md` §4 et §5 : « Quelles sont les conditions de la responsabilité civile pour faute ? ». `EVALUATION.md` §4 mesure, sous `fonds = 'LEGI'` et sur `textes_vigueur`, l'article 1240 au rang 70 du calcul exact ; la variante ne porte que le filtre `etat`, où ce rang n'est pas garanti. |
| `Q02-APPLICABLE` | `Q02` | `date_debut`, `date_fin` | Périmètre `droit_applicable` ; la date du filtre est le jour de la campagne (voir ci-dessous), et l'attendu de `Q02` naît le `2016-10-01` et ne finit qu'au `2999-01-01`, donc il satisfait ce filtre. |
| `Q30-APPLICABLE` | `Q30` | `date_debut`, `date_fin` | Même forme, sur l'attendu de `Q30` (`ABROGE_DIFF`, du `2014-10-01` au `2029-01-01`, qui satisfait ce même jour) : c'est le cas construit d'`EVALUATION.md` §9, où la fusion `droit_applicable` sert un article absent de `textes_vigueur`. |
| `Q01-CODE` | `Q01` | `code` | `cle_domaine` de l'attendu de `Q01`, « Code du travail ». |
| `Q06-FONDS` | `Q06` | `fonds` | Régime `fonds = LEGI` mesuré par `EVALUATION.md` §6. |
| `Q01-ANNEE` | `Q01` | `annee` | `annee` dérivée de `date_debut` de l'attendu (`2022`), bornée à `2020` et au-delà. |
| `Q21-NATURE` | `Q21` | `nature` | `cle_domaine` de l'attendu de `Q21`, `ARRETE`. |
| `Q21-ANNEE` | `Q21` | `annee` | `date_publi` de l'attendu de `Q21` (`2021-01-01`), donc `2021`. |
| `Q21-FONDS` | `Q21` | `fonds` | `JORF`, la seule valeur du fonds de l'index `jorf`. |
| `Q13-SECTION` | `Q13` | `section` | `section` de l'attendu de `Q13`, `CASS`, qui est le `fonds` du MCP sur `decisions`. |
| `Q13-JURIDICTION` | `Q13` | `juridiction` | `juridiction` de l'attendu de `Q13`, « Cour de cassation », qui est le `cle_domaine` du MCP. |
| `Q13-PARTIE-EXCLUE-DEFAUT` | `Q13` | `partie` (`$or`) | Défaut du MCP `partie_exclue`, `moyens` et `faits` (`serveur_mcp/outils.py`, `PARTIE_EXCLUE_DEFAUT`), exprimé par la forme `$or` de `index-legifacile.md`. |
| `Q31-PARTIE-EXCLUE-DEFAUT` | `Q31` | `partie` (`$or`) | Même forme, sur une décision dont la motivation porte la réponse (`partie_attendue: motifs` du bloc PARTIE de `questions.yaml`). |
| `Q25-THEME` | `Q25` | `theme` | `libelleparent` de l'attendu de `Q25`, qui est le `cle_domaine` de l'index `sarde`. |
| `Q26-THEME` | `Q26` | `theme` | `libelleparent` de l'attendu de `Q26`. |
| `Q28-TYPE` | `Q28` | `type` | `dole_type` de l'attendu de `Q28`, `LOI_PUBLIEE`, qui est le `cle_domaine` de l'index `dole`. |

Deux dates différentes ne doivent pas être confondues dans les deux lignes `APPLICABLE`. La
première est l'**intervalle du document** — `date_debut` et `date_fin` de l'attendu, lus dans
`questions.yaml`. La seconde est le **jour de la campagne** : le filtre de `droit_applicable`
s'écrit `date_debut <= <jour>` et `date_fin > <jour>`, et ce `<jour>` vaut **`2026-11-03`**, figé
dans les fichiers. Ces deux variantes sont donc datées : rejouées après le `2029-01-01`,
`Q30-APPLICABLE` ne satisferait plus le filtre et son attendu serait écarté par le filtre lui-même,
et non par la qualité de la recherche. Toute campagne postérieure doit relire ces deux lignes, et
le rapport doit nommer la date figée qu'il a rejouée ; le reste du jeu, lui, ne dépend pas du jour
de la campagne.

Le jeu de latence de MES-00 (1 000 requêtes tirées de titres) n'est **pas** repris ici. Ses
requêtes ont un `expected` connu, mais elles sont faciles et nombreuses : mêlées aux 57 questions,
elles domineraient la moyenne de qualité, qui n'aurait plus aucun rapport avec le seuil de 0,482
du jalon. Seules les 24 requêtes que MES-00 a réellement mesurées pour la fidélité sont reprises.
La cible de latence p95 de 698 ms se lit donc comme un **plafond** sur ce jeu-ci, plus court, et
non comme une comparaison de même population ; le rapport de `COMPARER-AVEC-PGVECTOR` doit le dire.

## Couverture

### Périmètres

Les huit périmètres du MCP sont couverts, et chacun par l'index que `index-legifacile.md` lui
associe.

| Périmètre | Index | Requêtes |
|---|---|---:|
| `droit_en_vigueur` | `articles` | 20 |
| `droit_historique` | `articles` | 3 |
| `droit_applicable` | `articles` | 2 |
| `jurisprudence` | `decisions` | 45 |
| `journal_officiel` | `jorf` | 10 |
| `index_thematique` | `sarde` | 8 |
| `dossiers_legislatifs` | `dole` | 6 |
| `titres` | `titres` | 3 |

L'index `articles` porte les deux corpus `textes_vigueur` et `textes_hors_vigueur`, là où pgvector
les partitionne. Le corpus d'une question y est donc exprimé par un filtre, celui du périmètre :
`etat` avec les quatre valeurs `VIGUEUR*` pour `droit_en_vigueur`, et le `$or` de `etat nin` et de
`etat exists: false` pour `droit_historique`. C'est la correspondance de `index-legifacile.md`,
appliquée telle quelle.

Le périmètre `jurisprudence` suit la **fusion multi-corpus du MCP**. Les 45 requêtes de ce
périmètre rejouent la forme de `mcp.jsonl` : les deux corpus `juri_sommaires` et
`juri_texte_integral` sont interrogés séparément, chacun rend `k × 2` résultats, puis le MCP
fusionne les deux classements et coupe à `k`. C'est la forme du rejeu de `COMPARER-AVEC-PGVECTOR`,
et non la lecture de `juri_texte_integral` seul que faisait MES-00. La règle est écrite ici sur
décision du propriétaire du 2026-11-08, d'après l'action 3 de
`docs/mesures/comparaison-pgvector-2026-11-06.md`. Elle ne change **aucune ligne** du jeu : les
45 appels portaient déjà le périmètre `jurisprudence`, sans membre `corpus`. L'index `decisions`
reste la cible physique du périmètre (`index-legifacile.md:319`, filtre ajouté « aucun »), parce
qu'il réunit les deux corpus ; la fusion est le fait du MCP, pas un filtre d'index.

### Modes visés

- **Dense** : tout le jeu. C'est le mode que `COMPARER-AVEC-PGVECTOR` confronte à pgvector.
- **Hybride** : tout le jeu. Il n'a pas d'équivalent direct dans le MCP, qui expose deux outils
  séparés (`recherche_semantique` et `recherche_texte`) et ne fusionne pas leurs classements : la
  comparaison hybride est une lecture de Crible, et le rapport doit le dire.
- **Lexical** : tout le jeu aussi, mais la comparaison est **indicative**. `recherche_texte` du MCP
  est une recherche plein texte PostgreSQL, dont les domaines et les filtres ne se projettent pas
  exactement sur les six index ; c'est pourquoi `mcp.jsonl` ne décrit que l'appel sémantique.

### Cas « article 1240 » et variantes

Quatre lignes portent l'article 1240 du code civil (`LEGIARTI000032041571`) comme attendu :
`Q02` (la question d'origine, sous le périmètre `droit_en_vigueur`), `Q02-VARIANTE-COURTE` (la
formulation courte mesurée par `EVALUATION.md` §4, où l'article est au rang 70 du calcul exact sous
`fonds = 'LEGI'` sur `textes_vigueur` : un rang de provenance, que le filtre `etat` de cette ligne
ne reproduit pas),
`Q02-APPLICABLE` (le même attendu derrière le filtre de dates de `droit_applicable`) et
`Q30-APPLICABLE`, le cas construit qui montre ce que la fusion `droit_applicable` apporte et que
`textes_vigueur` seul ne peut pas rendre.

### Requêtes filtrées

`EVALUATION.md` §18 mesure ce que le serveur **appelle**, et constate explicitement que les
options d'un appel restent invisibles dans `mcp.appels_outil` : aucune fréquence de filtre n'existe
dans les sources. Les filtres retenus sont donc ceux que le MCP **expose** et que
`index-legifacile.md` désigne comme les filtres principaux de chaque index, plus les deux défauts
du serveur :

- `cle_domaine` (nommé `code`, `nature`, `juridiction`, `theme` ou `type` selon l'index) : « le
  premier filtre d'un juriste » pour `articles`, « le filtre principal de la jurisprudence » pour
  `decisions`, et le filtre des périmètres `index_thematique` et `dossiers_legislatifs` ;
- `fonds` : régime mesuré par `EVALUATION.md` §6 ;
- `annee` : intervalle de période, filtre discriminant de `jorf` ;
- `partie_exclue` : appliqué **par défaut** à tout appel sémantique sur `decisions`, donc le filtre
  le plus fréquent du serveur par construction, mais éprouvé seulement par **deux** variantes
  (`Q13-PARTIE-EXCLUE-DEFAUT` et `Q31-PARTIE-EXCLUE-DEFAUT`) : les questions de base de
  `decisions` restent sans filtre pour rester comparables à la ligne de base de MES-00 (0,482), et
  les 43 autres lignes portent donc `partie_exclue: []` ;
- `etat` et les dates : les deux traductions du corpus et de l'applicabilité, portées par toutes
  les lignes `articles`.

Aucune variante filtrée n'est ajoutée sur `titres` : ni le périmètre `titres` ni les sources citées
ne donnent de valeur de `fonds`, de `nature` ou de `date_publi` pour cet index, et une valeur
inventée produirait un zéro qui ne mesurerait rien.

### Part des requêtes avec `expected`

**97 sur 97**, soit 100 %. Chaque requête a un attendu connu par une source citée — le `src_id`
d'un attendu de `questions.yaml`, le `source_id` d'une requête de MES-00, ou l'attendu de la
question de base pour une variante. Aucun `expected` n'est inventé, et aucune requête n'est
ajoutée sans attendu.

### Écarts connus du MCP, et requêtes touchées

1. **`partie_exclue` et le garde-fou au niveau du document.** Quand tous les passages d'une
   décision tombent dans l'exclusion, le SQL du MCP laisse passer tous ses passages exclus pour ne
   jamais retirer une décision entière (`index-legifacile.md`, écart 1). La grammaire des filtres
   ne sait pas exprimer cette condition, qui porte sur le document. Les requêtes touchées sont les
   deux variantes qui portent `partie_exclue` (`Q13-PARTIE-EXCLUE-DEFAUT` et
   `Q31-PARTIE-EXCLUE-DEFAUT`), et, en pratique, tout appel sémantique sur `decisions` qui laisse
   le défaut du serveur s'appliquer.
2. **`dedupliquer: "contenu"`.** Le SQL déduplique sur l'empreinte du document entier, alors que
   `dedup: "content"` de Crible fusionne les passages de contenu identique. Aucune requête du jeu
   ne le touche : toutes portent `dedupliquer: "document"`, et `crible eval` n'expose aucun réglage
   de déduplication de contenu (ADR-0125, décision 4). L'écart reste à mesurer, mais il ne peut pas
   l'être par ce jeu tel quel.
3. **Les deux corpus `juri_*` dans un seul index.** `decisions` réunit `juri_sommaires` et
   `juri_texte_integral`, que pgvector partitionne et que le périmètre `jurisprudence` fusionne.
   Les 45 requêtes de cet index sont touchées : la fusion de deux classements n'est pas la
   recherche unique de Crible sur les deux corpus réunis, et le rapport doit lire les écarts par
   corpus, jamais sur la moyenne du périmètre.
4. **`etat` refusé par la recherche sémantique sur `jorf` et `titres`.** Le champ est déclaré dans
   ces deux index pour la recherche lexicale, et la sémantique du MCP refuse ce filtre
   (`filtre_etat = false`). Aucune requête du jeu ne l'y porte, puisque `mcp.jsonl` ne décrit que
   des appels sémantiques ; l'adaptateur de `BRANCHER-MCP` doit conserver ce refus.
5. **Pas de date d'applicabilité dans la recherche sémantique du MCP.** `recherche_semantique`
   n'expose ni `date_debut` ni `date_fin` : le périmètre `droit_applicable` applique l'applicabilité
   après la fusion, et un « droit applicable à l'époque des faits » n'a aucune forme côté MCP. Le
   jeu ne contient donc aucune variante de date en dehors du périmètre, alors que l'index sait
   l'exprimer ; c'est un manque du MCP, à trancher dans `BRANCHER-MCP`.
6. **Le texte des requêtes est versionné.** ADR-0125, décision 16, range les fichiers d'entrée
   sous `data/`, gitignoré, et n'identifie les requêtes que par leur `id` dans les rapports. Le jeu
   est versionné dans `docs/legifacile/reference/` **par exception**, tranchée par **ADR-0138**
   (« Versionner le jeu de référence de P10 »), parce que le propriétaire valide un contenu, que la
   comparaison rejoue les mêmes lignes et que le projet Legifacile en garde la propriété après la
   passation (ADR-0129). L'exception ne couvre qu'un jeu sans requête d'utilisateur final : celui-ci
   ne contient que des questions d'évaluation écrites pour le projet, des titres de documents
   publics et des variantes de ces questions, et sa provenance est donnée plus haut. Le reste
   d'ADR-0125 ne bouge pas : les **rapports** de campagne continuent de n'identifier les requêtes
   que par leur `id`, sans texte ni valeur de filtre.

## Lecture du jeu

- Le seuil de rappel du jalon (au moins **0,482** sur les 56 questions satisfaisables, `Q30` à
  part) se lit sur les **57 identifiants de base**, `Q01` à `Q58` sans `Q40`, et non sur la moyenne
  du jeu entier : `crible eval` rend une ligne par requête, et c'est cette population-là qui
  reproduit la mesure de MES-00. La moyenne du jeu, elle, porte sur 97 requêtes et n'est pas
  comparable à 0,482 ; le rapport doit le dire et donner son `n`.
- `Q30` est la question non satisfiable de `questions.yaml` : sous le périmètre `droit_en_vigueur`,
  son attendu `ABROGE_DIFF` est écarté par le filtre `etat`, donc elle échoue **par construction**.
  `Q30-APPLICABLE` est sa contrepartie : le même attendu, derrière le filtre de dates, doit être
  trouvé. Les deux lignes se lisent ensemble.
- La fidélité (recall@10 au passage contre `exact: true`, cible 0,99) porte sur tout le jeu, sans
  `expected` : les requêtes sans attendu n'existent pas ici, mais la mesure, elle, ne regarde que
  le classement.
- Un `expected` absent du jeu mesuré n'est pas une erreur (ADR-0125, décision 4) : il compte comme
  non trouvé. C'est ce qui rend le jeu rejouable sur l'échantillon de P2 comme sur la base
  complète, où les `doc_id` sont ceux de la production.

## Validation du propriétaire

Validée par anticipation le 2026-11-03, confirmée par le propriétaire le 2026-11-04. Sa réponse, exacte :
« ok merci beacou les 3 point sont confirmé ».

## Actualisation du 2026-11-06 (`ACTUALISER-JEU-DE-REFERENCE`)

Le propriétaire a demandé le 2026-11-06 d'investiguer l'écart de `textes_vigueur` et d'actualiser
les `expected` qui désignent un texte qui n'est plus en vigueur. L'audit porte sur les **97**
`expected` du jeu, confrontés à l'état et aux dates de version lus dans les `records.sqlite` des
six index de l'instance importée, ouverts en lecture seule, et au périmètre de leur propre ligne
dans `mcp.jsonl`. Il ne porte pas sur les textes de requête.

### Ce que l'audit a trouvé

- Les **97** `expected` sont présents dans leur index : aucun document absent.
- Hors `Q30`, un seul `expected` n'est ni en vigueur ni admis par le filtre et le périmètre de sa
  ligne : celui de `Q04`. Sous le périmètre `droit_en_vigueur`, son filtre `etat` et son périmètre
  exigent tous deux un état `VIGUEUR*`, alors que le document attendu est `MODIFIE`.
- `Q30` reste le cas non satisfaisable **par construction** : son `expected` est `ABROGE_DIFF` et
  le filtre d'état de sa ligne l'écarte. Il n'est pas modifié, conformément à la fiche.
- `Q30-APPLICABLE` est admise par son filtre de dates : son `expected` `ABROGE_DIFF` commence le
  `2014-10-01` et finit le `2029-01-01`, donc il contient le jour figé du `2026-11-03`.
- `MES00-q0004`, `MES00-q0009` et `MES00-q0019` désignent des documents `ABROGE`, mais leurs
  lignes relèvent du périmètre `droit_historique` et portent le filtre `etat nin` correspondant :
  ils sont admis et restent tels quels.
- `Q13-PARTIE-EXCLUE-DEFAUT` et `Q31-PARTIE-EXCLUE-DEFAUT` portent un filtre de **passage**
  (`partie`), pas de document : l'audit les signale comme telles et ne les exclut pas.
- Sur les 25 `expected` d'`articles`, qui seuls portent un état et des dates de version, 19 sont
  `VIGUEUR` et `Q04` est `MODIFIE` avant actualisation ; **après actualisation, 20 sont
  `VIGUEUR`** et admis par leur filtre. Les 5 autres sont `Q30` (non satisfaisable par
  construction), `Q30-APPLICABLE` (admise par ses dates) et `MES00-q0004`, `MES00-q0009`,
  `MES00-q0019` (`ABROGE`, admis par `droit_historique`). Les **72** autres lignes (`decisions`
  45, `jorf` 10, `sarde` 8, `dole` 6, `titres` 3) relèvent de périmètres sans état de version ;
  leur `expected` est admis par le filtre de sa ligne.
- En particulier, `Q01`, `Q02` et `Q06` sont manquées alors que leur `expected` est **en
  vigueur** : ce sont de vraies pertes de classement, analysées dans
  `docs/mesures/comparaison-pgvector-2026-11-06.md`, et leur `expected` ne change pas.

Les preuves brutes sont sous `data/p10-compare/audit-before.json` (avant actualisation : 19
`VIGUEUR` et `Q04` `MODIFIE`) et `data/p10-compare/audit-after.json` (après actualisation : 20
`VIGUEUR`), gitignorés, produits par `tools/bench/p10-compare/audit_expected.py`.

**Cohérence avec MES-00 (constat du 2026-11-06).** MES-00 (2026-10-20) trouvait encore
`LEGIARTI000047381704` au rang 3 de `textes_vigueur`
(`MES-00-reference-pgvector.2026-10-20.eval-recherche.md:120`), alors que l'index importé donne à
ce document une `date_fin` au 2026-08-20 et à la version courante une `date_debut` au 2026-08-20,
toutes deux antérieures à MES-00. Le corpus a donc changé entre les deux campagnes : soit il était
en retard au 2026-10-20, soit la mise à jour a été intégrée plus tard avec une date d'effet
rétroactive ; les données ne permettent pas de trancher. MES-00 mesurait déjà 6/130 résultats hors
vigueur dans `textes_vigueur` (`:34-35`), ce qui montre qu'un décalage de ce type existait. Le
rejeu du 2026-11-06 ne rend plus l'ancien document (`diag.json`, `presence: []`) : son retrait du
corpus `textes_vigueur` est donc postérieur à MES-00. La date d'ingestion de la nouvelle version
n'est pas dans les données ; seule la conséquence est établie.

### Changement appliqué

Un seul `expected` change. Les `id`, les textes de requête, les filtres et `mcp.jsonl` sont
inchangés, ainsi que la population (56 questions satisfaisables, `Q30` à part, 31 questions
`juri_texte_integral` et 12 questions `textes_vigueur`) et les cibles.

| `id` | Index | Ancien `doc_id` | État et dates de l'ancien | Nouveau `doc_id` | État et dates du nouveau | Raison |
|---|---|---|---|---|---|---|
| `Q04` | `articles` | `LEGIARTI000047381704` | `MODIFIE`, du 2023-04-01 au 2026-08-20 | `LEGIARTI000054716237` | `VIGUEUR`, du 2026-08-20 au 2999-01-01 | Même article (`texte_cid` `LEGITEXT000005634379`, `num` `L442-1`) ; l'ancienne version n'est plus en vigueur depuis le 2026-08-20 et le filtre d'état de la ligne l'exclut. La nouvelle version est l'unique version en vigueur de cet article, au rang 3 dans Crible et dans pgvector. |

Décision du propriétaire : actualiser les `expected` qui attendent un texte qui n'est plus en
vigueur, sans toucher aux `expected` en vigueur ni à `Q30`. Le recalcul du rappel sur le jeu
actualisé est publié dans la section datée du rapport de comparaison ; les chiffres d'origine du
rapport restent inchangés.
