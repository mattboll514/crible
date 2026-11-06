# Harnais de la comparaison avec pgvector — `COMPARER-AVEC-PGVECTOR` (P10)

Harnais de la **passe A** de la tâche `COMPARER-AVEC-PGVECTOR` : le rejeu des 97 lignes du jeu de
référence contre la production pgvector, en lecture seule. Il applique le protocole d'ADR-0140
(décisions 1, 3, 4, 9, 10 et 11). Il ne modifie ni le code de production, ni un ADR, ni un document
de conception ; il n'écrit aucun fichier dans le dépôt Legifacile.

| Fichier | Rôle | Où il tourne |
|---|---|---|
| `queries.py` | fonctions pures : lecture du jeu versionné, jointure des 97 textes, correspondance MCP → SQL, tri/filtre/coupe de la fusion, rapports publiable et comptes | partout, couvert par pytest |
| `build_inputs.py` | écrit `data/p10-compare/queries.json` : les 97 lignes avec leur texte, leur périmètre et leurs paramètres ; imprime son SHA-256 | machine de développement |
| `encode_queries.py` | encode les 97 textes avec `serveur_mcp.encodage` (le chemin exact de MES-00) et écrit les vecteurs, sans aucun texte | Mac mini de production |
| `counts.py` | comptes de documents et de passages par corpus, avant et après la passe | Mac mini de production |
| `replay.py` | rejeu des 97 lignes par SQL direct sur `api.recherche_semantique`, échauffement puis trois répétitions chronométrées, strictement en série | Mac mini de production |
| `publish.py` | assemble le JSON publiable depuis le rejeu et les deux relevés de comptes | machine de développement |
| `pass_b.py` | fonctions pures de la passe B : jeux d'entrée `vector`/texte, agrégats par population, fidélité, latence, comparaison et écarts de données | partout, couvert par pytest |
| `build_eval_inputs.py` | écrit les 24 jeux d'entrée de la passe B sous `data/p10-compare/eval/inputs/` (quatre par index) | machine de développement |
| `aggregate_eval.py` | agrège les 24 rapports `eval`, le rejeu pgvector, les deux `inspect` et les comptes pgvector en un JSON sous `data/` | machine de développement |
| `run_eval.sh` | recette de la passe B : `inputs`, `counts before/after`, `index <nom>`, `aggregate`, `publish` ; un index par commande | machine de développement |
| `mes14.py` | fonctions pures et commandes de la passe C : jeux filtrés `games`, comptes de domaine `counts`, jointure `aggregate` ; aucune base de production, aucun import de production | machine de développement |
| `run_mes14.sh` | recette de la passe C : `games`, `counts`, `index decisions`, `index articles`, `aggregate` ; un cas par `crible eval` | machine de développement |
| `tests/` | tests pytest du code pur (aucune base, aucun import de production) | partout |

`mes00lib.py` n'est **pas** dupliqué : les scripts importent `tools/bench/mes00/mes00lib.py`
(garde-fous de chemin de sortie, `$PROD_PGOPTIONS` en lecture seule, écriture atomique, centiles).
Copier ce fichier à côté des scripts sur la machine de production, avec les scripts eux-mêmes.

## Recette exacte

Les valeurs de `.env.prod` ne sont jamais recopiées : seuls les **noms** de variables apparaissent.
Chaque bloc est annoté `[local]` ou `[distant]`.

```bash
# [local] entrée de la passe : les 97 textes joints à mcp.jsonl
python3 tools/bench/p10-compare/build_inputs.py \
    --reference-dir docs/legifacile/reference \
    --mes00-queries data/mes00/queries.json \
    --output data/p10-compare/queries.json
# -> « sha256 <empreinte> » sur la sortie standard

# [local] copie du harnais et de mes00lib dans un dossier de travail distant
#   (le dossier reste plat, aucune arborescence du dépôt n'est copiée)
set -a; . ./.env.prod; set +a
ssh "$PROD_SSH" 'mkdir -p /tmp/p10-compare && chmod 700 /tmp/p10-compare'
scp tools/bench/p10-compare/{queries,encode_queries,replay,counts}.py \
    tools/bench/mes00/mes00lib.py data/p10-compare/queries.json \
    "$PROD_SSH:/tmp/p10-compare/"

# [distant] environnement de chaque commande : PATH de macOS (le shell non interactif
#   n'hérite pas de celui de macOS), $PROD_REPO et $PROD_PGOPTIONS transmises dans la
#   ligne de commande depuis .env.prod (jamais écrites dans un fichier), et
#   PYTHONDONTWRITEBYTECODE=1. L'interpréteur est celui du projet Legifacile :
#   "$PROD_REPO/.venv/bin/python"

# [distant] comptes AVANT
"$PROD_REPO/.venv/bin/python" /tmp/p10-compare/counts.py --label before \
    --output /tmp/p10-compare/counts-before.json --project "$PROD_REPO"

# [distant] les 97 vecteurs de requête (lecture seule, aucune écriture dans le dépôt)
"$PROD_REPO/.venv/bin/python" /tmp/p10-compare/encode_queries.py \
    --queries /tmp/p10-compare/queries.json \
    --output /tmp/p10-compare/query-vectors.json --project "$PROD_REPO"
# -> « sha256 <empreinte> » sur la sortie standard

# [distant] rejeu : 1 passe d'échauffement non comptée, puis 3 répétitions chronométrées
"$PROD_REPO/.venv/bin/python" /tmp/p10-compare/replay.py \
    --queries /tmp/p10-compare/queries.json \
    --vectors /tmp/p10-compare/query-vectors.json \
    --output /tmp/p10-compare/replay.json --project "$PROD_REPO" \
    --repetitions 3 --warmup 1

# [distant] comptes APRÈS
"$PROD_REPO/.venv/bin/python" /tmp/p10-compare/counts.py --label after \
    --output /tmp/p10-compare/counts-after.json --project "$PROD_REPO"

# [local] rapatriement des sorties brutes sous data/ (gitignoré)
scp "$PROD_SSH:/tmp/p10-compare/{query-vectors,replay,counts-before,counts-after}.json" \
    data/p10-compare/

# [local] JSON publiable : id et doc_id seulement, plus centiles et comptes
python3 tools/bench/p10-compare/publish.py \
    --replay data/p10-compare/replay.json \
    --counts-before data/p10-compare/counts-before.json \
    --counts-after data/p10-compare/counts-after.json \
    --vectors data/p10-compare/query-vectors.json \
    --output docs/mesures/comparaison-pgvector-2026-11-06.pgvector.json

# [local] tests du code pur
uv run --with pytest python -m pytest tools/bench/p10-compare/tests -q
```

## Passe B — Crible, sur la machine de développement

La passe B mesure Crible sur le volume `crible-p10-data`, serveur arrêté, dans des conteneurs
neufs et éphémères (`docker run --rm`, jamais `-d`). Le volume est monté **en lecture-écriture**
parce que SQLite refuse d'ouvrir ses fichiers WAL sur un montage `:ro` ; `crible eval` ouvre
chaque index **en lecture seule** (ADR-0116), et les deux relevés `inspect` identiques avant et
après prouvent qu'aucun document ni passage n'a bougé. Aucune écriture **durable** n'est laissée
sous `/data` : seuls les `-wal` et `-shm` transitoires de SQLite apparaissent puis disparaissent.

```bash
# [local] quatre jeux par index, dont le jeu dense primaire à `vector`
sh tools/bench/p10-compare/run_eval.sh inputs

# [local] comptes Crible avant, puis un index par commande (quatre passes chacune)
sh tools/bench/p10-compare/run_eval.sh counts before
sh tools/bench/p10-compare/run_eval.sh index articles
sh tools/bench/p10-compare/run_eval.sh index decisions
sh tools/bench/p10-compare/run_eval.sh index dole
sh tools/bench/p10-compare/run_eval.sh index jorf
sh tools/bench/p10-compare/run_eval.sh index sarde
sh tools/bench/p10-compare/run_eval.sh index titres
sh tools/bench/p10-compare/run_eval.sh counts after

# [local] agrégats et rapports versionnés
sh tools/bench/p10-compare/run_eval.sh aggregate
sh tools/bench/p10-compare/run_eval.sh publish
```

Les quatre passes d'un index sont : `dense-primaire` (vecteur de pgvector, `--exact-threshold`
5000, le défaut de l'index), `dense-secondaire` (texte, `--exact-threshold` 5000), `hybride`
(texte, fusion `rrf` par défaut, `--exact-threshold` 5000) et `fidelite` (vecteur de pgvector,
`--exact-threshold 0`, N = 5000, ADR-0140 décision 6). Les trois passes de qualité portent la
latence de `debug.timings_ms` ; la passe de fidélité a son propre rapport parce qu'une même
exécution ne peut pas servir les deux réglages de `exact_threshold` sans fausser l'un des deux.

`aggregate_eval.py` agrège par `id` : le global sur les 56 questions satisfaisables (`Q30` à
part), `juri_texte_integral` sur 31 et `textes_vigueur` sur 12 ; le corpus d'une question de base
vient de `questions.yaml`, transcrit dans `pass_b.BASE_CORPUS` depuis le tableau de
`docs/mesures/MES-00-reference-pgvector.2026-10-20.eval-recherche.md` ; celui d'une ligne
`MES00-*` vient de `data/mes00/queries.json`. Les documents absents de Crible sont cherchés dans
les `records.sqlite` des instantanés de `data/p10-exchange/snapshots/`, qui portent la même
génération que le volume.

**Forme des rapports `eval` (complément d'ADR-0140 du 2026-11-06, décision 11).** Un rapport
`eval` ne porte qu'un jeu d'entrée, donc un seul réglage de `exact_threshold` : les quatre passes
d'un index donnent quatre couples `eval-<index>-<passe>-2026-11-06.{json,md}`. Le complément fixe
cette forme — **un rapport par index et par passe** —, et le suffixe de passe rend chaque fichier
explicite ; chaque rapport reste le rapport brut de `crible eval`, sans retouche.

## Passe C — MES-14 sur l'index réel

ADR-0140, décision 8, demande la p50 et la p95 des requêtes portant un filtre d'intervalle sur
`decisions` et `articles`, à domaine plein et à domaine réduit par une clause en bitmap. La
passe C génère dix jeux filtrés (cinq cas par index, 20 lignes chacun) sous
`data/p10-compare/mes14/games/`, les fait passer par `crible eval` en mode dense, puis joint les
rapports aux comptes de domaine lus dans les instantanés de la même génération.

```bash
# [local] les dix jeux filtrés (20 lignes par cas) et leurs SHA-256
sh tools/bench/p10-compare/run_mes14.sh games

# [local] scan_iids et pool_size de chaque cas, sur les instantanés, en lecture seule
sh tools/bench/p10-compare/run_mes14.sh counts

# [local] un cas par `crible eval`, 50 répétitions sur 20 lignes = 1 000 échantillons
sh tools/bench/p10-compare/run_mes14.sh index decisions
sh tools/bench/p10-compare/run_mes14.sh index articles

# [local] agrégat publiable, sans valeur de filtre
sh tools/bench/p10-compare/run_mes14.sh aggregate
```

Les cas répètent les bornes de MES-14 (`annee 2000..2010`, `2000-01-01..2003-12-31`) : seule la
présence ou l'absence d'une clause en bitmap change entre « domaine plein » et « domaine
réduit ». La clause en bitmap est une valeur de `juridiction` pour `decisions` et une valeur de
`etat` pour `articles`, les deux champs nommés par ADR-0140. Les valeurs restent dans les jeux
sous `data/`, jamais dans l'agrégat.

La scène `filter` de `crible eval` est l'appel `Pool::evaluate` seul : c'est la grandeur de
MES-14, lue sans modifier le banc. Le banc lui-même n'est **pas** rejoué : ses onze cas nomment
aussi `categorie`, `date_publication` et `score`, absents des index réels, et le complément
d'ADR-0140 du 2026-11-06 (décision 8) fixe cette lecture. Le `total` de `crible eval` donne en
plus le bout en bout.

**Forme des rapports, même règle que la passe B.** Un rapport `eval` porte un seul jeu et un
seul `exact_threshold` : la passe C écrit dix rapports bruts sous `data/p10-compare/mes14/runs/`,
et seul l'agrégat `docs/mesures/comparaison-pgvector-2026-11-06.mes14.json` est versionné.
L'agrégat daté refuse d'écraser un fichier existant ; `--overwrite` le remplace explicitement.
Avec les quatre passes de la passe B sur six index, `docs/mesures/` reçoit **48 fichiers**
`eval-*`, conformément au complément d'ADR-0140 du 2026-11-06 (décision 11) : un rapport par
index et par passe, jamais un rapport écrasé ni renommé.

## Ce que le rejeu appelle, exactement

- Un périmètre mono-corpus : un appel `api.recherche_semantique` avec `p_limite = k`.
- Un périmètre multi-corpus (`jurisprudence`, `droit_applicable`) : un appel par corpus avec
  `p_limite = k x 2`, puis tri par distance croissante, filtre d'applicabilité pour
  `droit_applicable`, coupe à `k` — la règle de `outils._fusionner_semantique`.
- `partie_exclue: []` devient `NULL` : la fonction SQL est neutre, et le défaut du serveur
  (`moyens`, `faits`) est une décision de `serveur_mcp.outils`, pas de la base. Les deux lignes
  `Q13-PARTIE-EXCLUE-DEFAUT` et `Q31-PARTIE-EXCLUE-DEFAUT` passent la liste explicite.
- `droit_applicable` applique `outils._applicable_aujourdhui` avec la date **figée au 2026-11-03**
  (ADR-0140, décision 9b) au lieu de `date.today()` : c'est la même fonction du serveur, appelée
  avec un autre argument. Le serveur, lui, n'expose aucun moyen de fixer cette date.
- La latence est mesurée côté hôte, en deux postes : `sql` (appels SQL + tri + filtre) et
  `reconstruction` (`api.source_vectorielle` + `encodage.passage_exact` +
  `rendu.plafonner_passage`). L'encodeur n'est **pas** dans le rejeu : les vecteurs sont figés
  (ADR-0140, décision 3). `encode_queries.py` chronomètre l'encodage des 97 textes à part.
- Les centiles sont **interpolés** (définition de `mes00lib.percentile`, celle de MES-00), sur
  l'ensemble des répétitions chronométrées : 97 requêtes × 3 répétitions = 291 mesures par poste.

## Lecture seule (ADR-0032)

- `$PROD_PGOPTIONS` doit contenir `default_transaction_read_only=on`, et chaque script vérifie
  `SHOW transaction_read_only` sur une connexion neuve avant de mesurer.
- Aucune écriture hors du dossier de sortie gardé sous `/tmp` ; le rapport est réécrit
  atomiquement après chaque requête.
- Les modules de production sont importés avec `PYTHONDONTWRITEBYTECODE=1` et un répertoire de
  travail hors du dépôt, pour qu'aucun fichier ne soit créé dans `$PROD_REPO`.
- `serveur_mcp.metriques` doit être absent : rien n'écrit dans `mcp.appels_outil`.
- Requêtes strictement en série, aucun redémarrage de service, aucune écriture en base.

## Sorties

| Chemin | Contenu | Versionné |
|---|---|---|
| `data/p10-compare/queries.json` | les 97 textes, périmètres et paramètres | non (gitignoré) |
| `data/p10-compare/query-vectors.json` | les 97 vecteurs, modèle, révision, appareil, précision | non |
| `data/p10-compare/replay.json` | par requête : rang, `doc_id`, corpus, score, distance, latences des trois répétitions | non |
| `data/p10-compare/counts-before.json`, `counts-after.json` | comptes par corpus et par index, datés | non |
| `docs/mesures/comparaison-pgvector-<date>.pgvector.json` | `id` et `doc_id` seulement, centiles, comptes | **oui** |
| `data/p10-compare/mes14/games/`, `games.sha256` | dix jeux filtrés de la passe C et leurs SHA-256 | non |
| `data/p10-compare/mes14/counts.json` | `scan_iids` et `pool_size` par cas, lus sur les instantanés | non |
| `data/p10-compare/mes14/runs/` | dix rapports bruts `crible eval` de la passe C | non |
| `docs/mesures/comparaison-pgvector-<date>.mes14.json` | cas, `scan_iids`, `pool`, centiles du rejeu MES-14 | **oui** |
