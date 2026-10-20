# MES-00 — scripts de mesure de la référence pgvector

Scripts de la tâche `MESURER-REFERENCE-PGVECTOR`. Ils mesurent la recherche
actuelle de Legifacile sur le Mac mini de production : latence décomposée et
fidélité HNSW contre recherche exacte. Le rapport est écrit dans
`docs/mesures/MES-00-reference-pgvector.md` par `@mesureur`.

Les identifiants, les options de ligne de commande et les clés JSON produits par
ces scripts sont en anglais (règle 8). Les documents et le rapport restent en
français. Les anciennes clés françaises (`requetes`, `requete_id`, `texte`,
`graine`, etc.) restent acceptées **en entrée** pour rejouer les fichiers de la
campagne, mais aucun fichier produit n'en contient.

## Règles de sécurité (ADR-0032)

- **Lecture seule.** `$PROD_PGOPTIONS` doit être défini et contenir
  `default_transaction_read_only=on`. Les scripts qui ouvrent une session
  Postgres (`gen_requetes.py --from-db`, `latence_directe.py`,
  `vecteurs_requetes.py`, `fidelite_hnsw.py`) refusent de démarrer sinon. Les
  commandes enfants le reçoivent dans `PGOPTIONS`. `filter_queries.py` ne
  touche pas Postgres et ne demande pas la garde ; `gen_requetes.py` la demande
  aussi pour `--from-csv`/`--from-json`, par prudence.
- **Aucune valeur de `.env.prod`** n'est écrite dans un script, un fichier de
  sortie, un rapport ou un commit. Ce README ne cite que les **noms** des
  variables : `$PROD_SSH`, `$PROD_REPO`, `$PROD_DB_CONTAINER`,
  `$PROD_PGOPTIONS`, `$PROD_DAILY_WINDOW`, `$PROD_SERVICES`.
- **Rien n'est ajouté au dépôt Legifacile.** Les scripts sont copiés dans un
  dossier temporaire du Mac mini et exécutés avec l'environnement de
  Legifacile. Aucun fichier **versionné** n'est modifié dans `$PROD_REPO`, et
  aucun fichier n'est créé hors de `/tmp`. Le contrôle combine `git status
  --porcelain` (fichiers suivis), le `diff` des **entrées de répertoire**
  ignorées (`--ignored=matching -uall`, qui affiche un dossier mais pas son
  contenu), l'inventaire fichier par fichier `git ls-files --others --ignored
  --exclude-standard` comparé avant/après, et la liste `find` des fichiers
  modifiés depuis le début de la campagne. Seuls `ls-files` et `find` sont
  fichier par fichier ; le `diff` des entrées de répertoire ne détecte pas un
  fichier ajouté dans `logs/` ou `scripts/db/__pycache__/`.
  Les scripts de Legifacile (`eval_recherche.py`, `froid_decomposition.py`)
  écrivent leurs journaux dans `LEGIFACILE_LOG_DIR` ; la campagne le fixe à
  `/tmp/mes00/logs`. Sans cette variable, leur défaut reste `$PROD_REPO/logs/`.
- **Aucune écriture hors de `/tmp`.** Chaque script vérifie que `--output` est
  un chemin absolu sous `/tmp` (ou sous un `--allow-root` explicitement passé
  et existant). Un chemin sous `$PROD_REPO` ou sous le dépôt Crible est refusé.
  Les commandes enfants lancées par `fidelite_hnsw.py` tournent avec leur
  répertoire courant dans le dossier de sortie (`cwd=output.parent`).
  `gen_requetes.py --from-db` lance `psql` dans le répertoire courant du
  processus : il ne lit qu'une sortie standard et n'écrit aucun fichier.
- **Racine protégée indépendante du niveau d'installation.** `mes00lib` cherche
  le dépôt Crible en remontant depuis le script jusqu'à un dossier contenant
  `.git` ou `Cargo.toml`. Copié dans `/tmp/mes00/`, le script ne trouve aucune
  racine de dépôt et ne protège donc que `$PROD_REPO` ; il n'y a plus de dossier
  imbriqué à créer.
- **Pas de bytecode dans `$PROD_REPO`.** `latence_directe.py` et
  `vecteurs_requetes.py` posent `sys.dont_write_bytecode = True` et changent de
  répertoire courant vers le dossier de sortie **avant** d'importer les modules
  de Legifacile.
- **Requêtes en série.** Les scripts n'ont aucune option de parallélisme.
- **Aucun redémarrage** de Postgres, de Docker ni des services
  `$PROD_SERVICES`. Jamais de mesure pendant `$PROD_DAILY_WINDOW`.
- **`eval_recherche.py` toujours avec `--sortie /tmp/…` et
  `--dedupliquer document`.** Sans `--sortie`, il réécrit
  `docs/mcp/EVALUATION.md` dans `$PROD_REPO`. Son journal suit
  `LEGIFACILE_LOG_DIR` ; la campagne le fixe à `/tmp/mes00/logs`, donc il ne
  crée rien dans `$PROD_REPO`.

## Scripts

| Script | Rôle |
|---|---|
| `gen_requetes.py` | génère le jeu de requêtes déterministe |
| `filter_queries.py` | sélectionne le sous-ensemble du périmètre (491 requêtes) |
| `latence_directe.py` | mesure la latence par requête, trois postes, deux vues |
| `vecteurs_requetes.py` | encode 3 requêtes par corpus |
| `fidelite_hnsw.py` | compare HNSW à la recherche exacte (recall@10) |
| `mes00lib.py` | garde-fous, percentiles, exécution des gabarits |

`latence.py` (ancien harnais à base de `froid_decomposition.py --json`) a été
supprimé : son interface ne permet pas une mesure par requête. La
contre-mesure indépendante reste l'appel direct à `froid_decomposition.py`
documenté dans le rapport.

## Contrat avec les scripts de Legifacile

L'interface exacte de `scripts/db/froid_decomposition.py` n'est pas décrite
dans la documentation de Crible. Le harnais est donc paramétrable : il reçoit un
**gabarit d'arguments JSON** (un tableau de chaînes) et remplace des
placeholders, sans passer par un shell. Un titre contenant des espaces, des
guillemets ou des points-virgules reste un seul argument.

Placeholders disponibles :

| Script | Placeholders |
|---|---|
| `fidelite_hnsw.py` | `{vector_literal}`, `{corpus}`, `{query_id}`, `{container}` |

Les alias historiques `{vecteur_litteral}`, `{requete_id}` et `{conteneur}`
restent acceptés pour les gabarits déjà écrits.

Une commande enfant doit imprimer sur la sortie standard un tableau d'ids ou un
objet `{"ids": [...]}`. Un échec de commande arrête la campagne et n'écrit aucun
résultat partiel.

**Hypothèses :**

- la similarité exacte est le **cosinus** (vecteurs normalisés), comme le
  `<=>` de pgvector utilisé par Legifacile ; le mode `sql` reprend l'opérateur
  fourni dans le gabarit ;
- le froid de `latence_directe.py` est une période d'inactivité contrôlée, pas
  un vidage de cache : le script ne peut pas vider un cache de production en
  lecture seule. `froid_decomposition.py` reste la contre-mesure indépendante.

## 1. Jeu d'environ 1 000 requêtes de latence

`gen_requetes.py` échantillonne des **titres d'articles et de sommaires** avec
une graine fixe. Il ne reprend jamais une question d'utilisateur. La sortie
contient les requêtes et leurs métadonnées (graine, date, répartition par
corpus, empreinte SHA-256).

La requête SQL est fournie par l'opérateur, car le schéma de Legifacile n'est
pas décrit dans Crible. Elle doit commencer par `SELECT` ou `WITH`, tenir en une
seule instruction et renvoyer exactement les colonnes `id`, `corpus` et `text`
(l'ancien nom `texte` est accepté). Le script refuse tout autre mot-clé
(`INSERT`, `UPDATE`, `COPY`…) et ajoute lui-même l'enveloppe
`COPY (…) TO STDOUT WITH (FORMAT csv, HEADER true)`. La requête est exécutée
dans `$PROD_DB_CONTAINER` avec `PGOPTIONS` et `psql -U postgres`.

```bash
python3 gen_requetes.py \
  --from-db \
  --sql-file /tmp/mes00/titres.sql \
  --count 1000 \
  --seed 20261019 \
  --output /tmp/mes00/out/queries.json
```

Mode hors production, pour rejouer un export existant (CSV, JSON ou JSONL avec
les mêmes colonnes) :

```bash
python3 gen_requetes.py \
  --from-csv /tmp/mes00/out/titres.csv \
  --count 1000 \
  --seed 20261019 \
  --output /tmp/mes00/out/queries.json
```

Les modes `--from-csv` et `--from-json` exigent quand même `$PROD_PGOPTIONS`, par
prudence, même s'ils ne touchent pas Postgres. `filter_queries.py` est le seul
script du dossier qui ne demande pas la garde : il ne lit qu'un fichier local.

### Sélection du périmètre

La campagne de fusion utilise 491 requêtes : les quatre corpus des périmètres
multi-corpus, plafonnés à **125 requêtes par corpus**, dans l'ordre du fichier
d'entrée (lui-même mélangé par la graine `20261019`). Aucun second tirage
aléatoire n'est fait.

```bash
python3 filter_queries.py \
  --input /tmp/mes00/out/queries.json \
  --output /tmp/mes00/out/queries-perimeter.json \
  --corpus juri_sommaires \
  --corpus juri_texte_integral \
  --corpus textes_hors_vigueur \
  --corpus textes_vigueur \
  --per-corpus 125
```

Si la campagne `--perimeter-first` du 2026-10-19 était rejouée, cette commande
**produirait** les 491 mêmes `query_id` (`juri_sommaires` 125,
`juri_texte_integral` 122, `textes_hors_vigueur` 119, `textes_vigueur` 125),
d'après la répartition consignée dans le rapport. Les fichiers du 19 ayant été
supprimés, la comparaison n'est pas rejouable et cette égalité n'est pas
conservée. Cette campagne n'a pas été rejouée le 20 (voir §2) : la passe de
référence mesure les 1 000 requêtes, vues mono-corpus et fusion.

## 2. Latence décomposée (`latence_directe.py`)

`latence_directe.py` appelle **les mêmes fonctions de production** que
`froid_decomposition.py`, une requête à la fois et en série :
`serveur_mcp.encodage.encoder_requete` (encodeur), `api.recherche_semantique`
(SQL + HNSW), puis la reconstruction du passage via `froid_decomposition._poste_c`.
Aucune logique n'est dupliquée.

Deux vues sont mesurées pour chaque requête :

- `corpus` : un appel `api.recherche_semantique` sur le corpus de la requête ;
- `perimeter` : la fusion multi-corpus via `froid_decomposition._poste_b` (même
  règle `k x 2`, même tri, même filtre d'applicabilité).

Un appel utilisateur réel paie l'une **ou** l'autre, jamais les deux ; le poste C
est donc payé deux fois par requête dans ce harnais. Les deux vues sont agrégées
séparément.

```bash
python3 latence_directe.py \
  --queries /tmp/mes00/out/queries.json \
  --project "$PROD_REPO" \
  --k 10 \
  --idle-minutes-per-corpus 10 \
  --output /tmp/mes00/out/latency.json
```

`--idle-minutes-per-corpus` fixe la période d'inactivité avant la première
requête de chaque corpus : c'est le seul « froid » que le script peut produire
honnêtement, il ne vide jamais un cache de production. `--limit N` sert aux
essais à blanc.

**Biais d'ordre.** Le second appel sur le même graphe trouve les pages chargées
par le premier : jusqu'à 30× sur le poste SQL dans la campagne. L'ordre décide
donc quelle vue paraît la plus rapide. `--perimeter-first` mesure la fusion en
premier. **Cet ordre n'a pas été rejoué dans la campagne de référence du
2026-10-20** : les chiffres de fusion publiés viennent de la passe principale,
mesurée après le mono-corpus, et restent donc biaisés à la baisse (rapport,
§4.2). Sans l'option, l'ordre est celui d'un appel mono-corpus en production.

La sortie contient `meta`, `measurements` et `aggregates`. Aucun texte de
requête n'y est écrit : seulement `query_id`, `corpus`, `perimeter`, les états
`cold`/`warm` et les durées par poste (`encoder`, `sql`, `reconstruction`,
`total`). Les mesures censurées par `statement_timeout` sont `null` et comptées
dans `censored`. Une mesure censurée ne fabrique aucune durée : la
reconstruction qui suit un appel SQL annulé est `null` elle aussi, comptée
censurée, et n'entre pas dans la série comme un 0 ms. Une mesure qui n'a pas eu
lieu (fusion réussie mais zéro résultat : rien à reconstruire) est également
`null`, mais comptée dans `not_run`, distinct de `censored`. Un échec
d'encodage (`failed_run`) laisse toutes les durées `null` et compte `failed` :
ni une censure, ni une étape non lancée. Les agrégats portent donc, par poste et
par état, `n`, `censored`, `not_run` et `failed`. Une requête
dont le corpus n'appartient à aucun périmètre n'a pas de vue fusion : elle est
exclue des agrégats `perimeter`.

## 3. Fidélité HNSW contre recherche exacte

`fidelite_hnsw.py` calcule le recall@10 par corpus :
`|exact_top_10 ∩ hnsw_top_10| / |exact_top_10|`.

Entrée : un JSON de vecteurs de requête, produit par l'encodeur de Legifacile
(même modèle que la production). Aucun texte n'est nécessaire.

`vecteurs_requetes.py` produit ce fichier : il garde les `--per-corpus` premières
requêtes de chaque corpus (le fichier est déjà mélangé par graine fixe), les
encode avec l'encodeur de production (`serveur_mcp.encodage`, chargé depuis
`$PROD_REPO`) et n'écrit que des vecteurs.

```bash
python3 vecteurs_requetes.py \
  --queries /tmp/mes00/out/queries.json \
  --per-corpus 3 \
  --project "$PROD_REPO" \
  --output /tmp/mes00/out/query-vectors.json
```

```json
{
  "queries": [
    {
      "query_id": "q0001",
      "corpus": "juri_texte_integral",
      "vector": [0.01, -0.02, 0.03]
    }
  ]
}
```

### Mode `export` (recommandé)

1. De nuit, hors production, produire un export ponctuel avec
   `scripts/db/exporter_vecteurs.py`. Le script de Crible attend l'une des deux
   dispositions suivantes, une par corpus :

   ```
   /tmp/mes00/export/<corpus>/vectors.npy   # float16 ou float32, shape (N, D)
   /tmp/mes00/export/<corpus>/ids.jsonl     # un id externe par ligne, même ordre
   ```

   ou un fichier `/tmp/mes00/export/<corpus>.npz` avec les tableaux `ids` et
   `vectors` (l'ancien nom `vecteurs` est accepté). Le format `.npy` est préféré :
   il est lu en `mmap` et parcouru par blocs de `--chunk-size` vecteurs, donc un
   corpus plus gros que la RAM ne provoque pas d'OOM.

2. Interroger HNSW sur la production, une requête à la fois, avec un gabarit
   dont la sortie est un tableau d'ids ou un objet `{"ids": [...]}` :

```bash
python3 fidelite_hnsw.py \
  --mode export \
  --queries /tmp/mes00/out/query-vectors.json \
  --export-dir /tmp/mes00/export \
  --hnsw-command-json "[\"docker\",\"exec\",\"-i\",\"-e\",\"PGOPTIONS\",\"{container}\",\"psql\",\"-X\",\"-q\",\"-A\",\"-t\",\"-U\",\"postgres\",\"-c\",\"BEGIN READ ONLY; SET LOCAL hnsw.ef_search = 200; SELECT COALESCE(json_agg(json_build_object('id', src_id || '#' || passage_ord)), '[]'::json) FROM (SELECT src_id, passage_ord FROM vec.passages WHERE corpus = '{corpus}' ORDER BY embedding <=> '{vector_literal}'::halfvec(768) LIMIT 10) t; COMMIT;\"]" \
  --k 10 \
  --output /tmp/mes00/out/fidelity-ef200.json
```

### Gabarits de fidélité pour le mode `sql`

Les trois gabarits sont des JSON `argv`, sans shell ; `{corpus}` et
`{vector_literal}` sont remplacés par le harnais, et `{container}` par
`$PROD_DB_CONTAINER`. Ils sont écrits sous `/tmp/mes00/out`, puis lus :

```bash
mkdir -p /tmp/mes00/out
cat > /tmp/mes00/out/hnsw-ef40.json <<'JSON'
["docker","exec","-e","PGOPTIONS","{container}","psql","-X","-q","-A","-t","-U","postgres","-v","ON_ERROR_STOP=1","-c","BEGIN READ ONLY; SELECT COALESCE(json_agg(json_build_object('id', src_id || '#' || passage_ord)), '[]'::json) FROM (SELECT src_id, passage_ord FROM vec.passages WHERE corpus = '{corpus}' ORDER BY embedding <=> '{vector_literal}'::halfvec(768) LIMIT 10) t; COMMIT;"]
JSON

cat > /tmp/mes00/out/hnsw-ef200.json <<'JSON'
["docker","exec","-e","PGOPTIONS","{container}","psql","-X","-q","-A","-t","-U","postgres","-v","ON_ERROR_STOP=1","-c","BEGIN READ ONLY; SET LOCAL hnsw.ef_search = 200; SELECT COALESCE(json_agg(json_build_object('id', src_id || '#' || passage_ord)), '[]'::json) FROM (SELECT src_id, passage_ord FROM vec.passages WHERE corpus = '{corpus}' ORDER BY embedding <=> '{vector_literal}'::halfvec(768) LIMIT 10) t; COMMIT;"]
JSON

cat > /tmp/mes00/out/exact.json <<'JSON'
["docker","exec","-e","PGOPTIONS","{container}","psql","-X","-q","-A","-t","-U","postgres","-v","ON_ERROR_STOP=1","-c","BEGIN READ ONLY; SET LOCAL enable_indexscan = off; SET LOCAL enable_bitmapscan = off; SELECT COALESCE(json_agg(json_build_object('id', src_id || '#' || passage_ord)), '[]'::json) FROM (SELECT src_id, passage_ord FROM vec.passages WHERE corpus = '{corpus}' ORDER BY embedding <=> '{vector_literal}'::halfvec(768) LIMIT 10) t; COMMIT;"]
JSON

HNSW_COMMAND="$(cat /tmp/mes00/out/hnsw-ef40.json)"
HNSW_COMMAND_EF200="$(cat /tmp/mes00/out/hnsw-ef200.json)"
EXACT_COMMAND="$(cat /tmp/mes00/out/exact.json)"
```

- HNSW à `ef_search` par défaut (40) : `$HNSW_COMMAND`, sans `SET LOCAL
  hnsw.ef_search`.
- HNSW à `ef_search = 200` : `$HNSW_COMMAND_EF200`.
- Exact : `$EXACT_COMMAND`, avec `SET LOCAL enable_indexscan=off; SET LOCAL
  enable_bitmapscan=off;`. Le plan obtenu est `Seq Scan` + `Sort`, le plan HNSW
  est `Index Scan using idx_<corpus>_hnsw`.

### Observation à `ef_search = 1000` (quatrième procédure, hors du harnais)

La campagne du 2026-10-20 a mesuré une requête à `ef_search = 1000` (`q0018`,
`juri_texte_integral`) pour vérifier que son recall nul ne venait pas du
réglage. **Cette procédure n'utilise pas `fidelite_hnsw.py`** : il n'y a donc
pas de quatrième gabarit JSON, mais un script d'une seule pièce. Le bloc
ci-dessous reprend ce script avec le libellé `# machine:` **déjà substitué**
(troisième retouche, cf. rapport §7.1). Il fait deux appels `psql` d'une
instruction chacun (`BEGIN READ ONLY;` … `COMMIT;`), en lecture seule, et écrit
le top 10 HNSW puis le top 10 exact avec leurs distances.

```bash
# [distant] observation ponctuelle de q0018 à ef_search = 1000
set -e
OUT=/tmp/mes00/out/fidelity-q0018-ef1000.txt
V=$("$PROD_REPO/.venv/bin/python" -c '
import json
d=json.load(open("/tmp/mes00/out/query-vectors.json"))
q=next(x for x in d["queries"] if x["query_id"]=="q0018")
print("["+",".join(f"{v:.8g}" for v in q["vector"])+"]")')
{
echo '# MES-00 - q0018 (juri_texte_integral), ef_search = 1000 contre exact, sortie brute'
echo "# date: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo '# machine: Mac mini de production'
echo '# HNSW, ef_search = 1000, top 10 : rang, id, distance'
docker exec -e PGOPTIONS="$PROD_PGOPTIONS" "$PROD_DB_CONTAINER" psql -X -q -A -t -U postgres -v ON_ERROR_STOP=1 -c "BEGIN READ ONLY; SET LOCAL hnsw.ef_search = 1000; SELECT row_number() OVER (ORDER BY embedding <=> '$V'::halfvec(768)) || ' ' || src_id || '#' || passage_ord || ' ' || round((embedding <=> '$V'::halfvec(768))::numeric, 6) FROM vec.passages WHERE corpus = 'juri_texte_integral' ORDER BY embedding <=> '$V'::halfvec(768) LIMIT 10; COMMIT;"
echo '# exact (enable_indexscan off, enable_bitmapscan off), top 10 : rang, id, distance'
docker exec -e PGOPTIONS="$PROD_PGOPTIONS" "$PROD_DB_CONTAINER" psql -X -q -A -t -U postgres -v ON_ERROR_STOP=1 -c "BEGIN READ ONLY; SET LOCAL enable_indexscan = off; SET LOCAL enable_bitmapscan = off; SELECT row_number() OVER (ORDER BY embedding <=> '$V'::halfvec(768)) || ' ' || src_id || '#' || passage_ord || ' ' || round((embedding <=> '$V'::halfvec(768))::numeric, 6) FROM vec.passages WHERE corpus = 'juri_texte_integral' ORDER BY embedding <=> '$V'::halfvec(768) LIMIT 10; COMMIT;"
} > "$OUT" 2>&1
wc -l "$OUT"
```

`$V` est la valeur littérale du vecteur de `q0018`, lue dans
`/tmp/mes00/out/query-vectors.json` (produit plus haut par
`vecteurs_requetes.py`) ; elle n'est jamais recopiée dans un fichier versionné.
La ligne `# machine:` est un libellé fixe : le nom d'hôte du Mac mini de
production n'est jamais écrit dans un fichier versionné (ADR-0032). Pour une
passe de harnais, le gabarit JSON se déduirait des deux `-c` ci-dessus ; il
n'est pas livré, puisque la campagne ne s'en est pas servie.

### Mode `sql` (repli, de nuit)

Réservé à un petit échantillon. La requête exacte doit désactiver l'index HNSW
et rester en lecture seule. `--max-queries` vaut 25 par défaut et ne peut pas
dépasser 100 ; au-delà, le script refuse de démarrer.

```bash
python3 fidelite_hnsw.py \
  --mode sql \
  --queries /tmp/mes00/out/query-vectors.json \
  --max-queries 25 \
  --hnsw-command-json "$HNSW_COMMAND" \
  --exact-command-json "$EXACT_COMMAND" \
  --output /tmp/mes00/out/fidelity-ef40.json

python3 fidelite_hnsw.py \
  --mode sql \
  --queries /tmp/mes00/out/query-vectors.json \
  --max-queries 25 \
  --hnsw-command-json "$HNSW_COMMAND_EF200" \
  --exact-command-json "$EXACT_COMMAND" \
  --output /tmp/mes00/out/fidelity-ef200.json
```

Le nom de table, la colonne de vecteur et le filtre de corpus doivent être
adaptés au schéma de Legifacile. Le mode `sql` n'écrit rien et ne crée aucun
index. Le corpus est validé par `[a-z0-9_]+` dans les deux modes, y compris
avant son injection dans `psql -c`.

Le `meta` de la sortie porte `ef_search` (valeur lue dans le gabarit HNSW),
`hnsw_template_sha256` et `exact_template_sha256` (empreintes des `argv`, sans
valeur d'environnement) : deux passes qui ne diffèrent que par `ef_search` ne se
distinguent plus par leur seul nom de fichier. Ces clés datent de la correction
du 2026-10-20 ; les fichiers bruts déjà produits
(`…2026-10-20.fidelity-ef40.json`, `…fidelity-ef200.json`) ne les portent pas et
ne sont pas réécrits.

## Protocole complet sur le Mac mini

Le `set -a; . ./.env.prod` exécuté sur le poste de développement exporte les
variables **dans le shell local** : il ne les transmet pas à `ssh`. Les trois
variables nécessaires (`PROD_REPO`, `PROD_PGOPTIONS`, `PROD_DB_CONTAINER`)
doivent être exportées **dans le shell distant**, à partir d'une source non
versionnée située hors de `$PROD_REPO` (fichier créé par le propriétaire, mode
600) ou par un `export` manuel. Aucune valeur n'est recopiée ici.

```bash
# [local] seul $PROD_SSH est lu depuis .env.prod pour ouvrir la session
set -a; . ./.env.prod; set +a
ssh "$PROD_SSH"

# [distant] le shell non interactif n'hérite pas du PATH de macOS : docker est
# sous /usr/local/bin. Sans cet export, fidelite_hnsw.py échoue sur
# « docker: No such file ».
export PATH=/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin
# Aucun bytecode dans $PROD_REPO (même garde que dans le rapport).
export PYTHONDONTWRITEBYTECODE=1
# source non versionnée, hors du dépôt de production
set -a; . "$HOME/.config/crible/prod.env"; set +a
test -n "$PROD_REPO" && test -n "$PROD_PGOPTIONS" && test -n "$PROD_DB_CONTAINER"
mkdir -p /tmp/mes00/out /tmp/mes00/logs
chmod 700 /tmp/mes00 /tmp/mes00/out /tmp/mes00/logs
export LEGIFACILE_LOG_DIR=/tmp/mes00/logs
# Optionnel, pour forcer un fichier unique :
# export LEGIFACILE_LOG_FILE=/tmp/mes00/logs/legifacile.log
date -u > /tmp/mes00/out/status-before.date
# Début de campagne, pour le contrôle `find -newermt` de fin.
date '+%Y-%m-%d %H:%M:%S' > /tmp/mes00/out/campaign-start.txt
# Entrées de répertoire ignorées, avant : ce `status` affiche un dossier, pas son contenu.
git --no-optional-locks -C "$PROD_REPO" status --porcelain \
  > /tmp/mes00/out/porcelain-before.txt
git --no-optional-locks -C "$PROD_REPO" status --porcelain --ignored=matching -uall \
  > /tmp/mes00/out/ignored-before.txt
# Inventaire fichier par fichier des ignorés, avant.
git --no-optional-locks -C "$PROD_REPO" ls-files --others --ignored --exclude-standard \
  | sort > /tmp/mes00/out/ignored-files-before.txt
```

Copier **seulement** les scripts et ce README. Ne jamais copier `.env.prod`.
La racine protégée est trouvée par recherche de `.git`/`Cargo.toml` : le dossier
`/tmp/mes00/` peut être plat.

```bash
# [local]
ssh "$PROD_SSH" 'mkdir -p /tmp/mes00/out /tmp/mes00/logs && chmod 700 /tmp/mes00 /tmp/mes00/out /tmp/mes00/logs'
scp tools/bench/mes00/*.py tools/bench/mes00/README.md \
  "$PROD_SSH:/tmp/mes00/"

# [distant] export CSV des titres, lecture seule
docker exec -e PGOPTIONS="$PROD_PGOPTIONS" "$PROD_DB_CONTAINER" \
  psql -X -q -A -t -U postgres -v ON_ERROR_STOP=1 \
  -c "COPY (<requête ci-dessous>) TO STDOUT WITH (FORMAT csv, HEADER true)" \
  > /tmp/mes00/out/titres.csv

# [distant] 1 000 requêtes (l'export CSV est converti à la lecture)
"$PROD_REPO/.venv/bin/python" /tmp/mes00/gen_requetes.py \
  --from-csv /tmp/mes00/out/titres.csv \
  --count 1000 --seed 20261019 \
  --output /tmp/mes00/out/queries.json

# [distant] latence chaude : 1 000 requêtes, en série, sans inactivité
"$PROD_REPO/.venv/bin/python" /tmp/mes00/latence_directe.py \
  --queries /tmp/mes00/out/queries.json --project "$PROD_REPO" \
  --output /tmp/mes00/out/latency.json \
  --idle-minutes-per-corpus 0 --k 10

# [distant] latence froide : 10 min d'inactivité avant chaque corpus
# (8 mesures froides + 992 chaudes). Lancée en arrière-plan pour survivre à une
# coupure SSH, puis ATTENDUE avant toute autre mesure : les requêtes restent en
# série, aucune passe ne tourne en parallèle d'une autre.
nohup "$PROD_REPO/.venv/bin/python" /tmp/mes00/latence_directe.py \
  --queries /tmp/mes00/out/queries.json --project "$PROD_REPO" \
  --output /tmp/mes00/out/latency-cold.json \
  --idle-minutes-per-corpus 10 --k 10 \
  > /tmp/mes00/out/latency-cold.stdout 2>&1 &
echo $! > /tmp/mes00/out/latency-cold.pid

# `wait` ne voit que les enfants du shell courant ; on sonde donc le processus.
while kill -0 "$(cat /tmp/mes00/out/latency-cold.pid)" 2>/dev/null; do sleep 60; done
test -s /tmp/mes00/out/latency-cold.json

# [distant] sous-ensemble du périmètre (491 requêtes)
"$PROD_REPO/.venv/bin/python" /tmp/mes00/filter_queries.py \
  --input /tmp/mes00/out/queries.json \
  --output /tmp/mes00/out/queries-perimeter.json \
  --corpus juri_sommaires --corpus juri_texte_integral \
  --corpus textes_hors_vigueur --corpus textes_vigueur --per-corpus 125

# [distant] NON EXÉCUTÉE dans la campagne de référence du 2026-10-20.
# La campagne `--perimeter-first` (491 requêtes) de la passe du 19 n'a pas été
# rejouée, et `perimeter-latency.json` n'est pas livré : la fusion publiée est
# celle de la passe principale, mesurée après le mono-corpus (biais de cache).
# Bloc conservé pour une campagne ultérieure.
# "$PROD_REPO/.venv/bin/python" /tmp/mes00/latence_directe.py \
#   --queries /tmp/mes00/out/queries-perimeter.json \
#   --output /tmp/mes00/out/perimeter-latency.json \
#   --idle-minutes-per-corpus 2 --k 10 --perimeter-first

# [distant] qualité sur les 57 questions étiquetées (sortie hors du dépôt)
"$PROD_REPO/.venv/bin/python" "$PROD_REPO/scripts/db/eval_recherche.py" \
  --sortie /tmp/mes00/out/eval-recherche.md --k 10 --dedupliquer document

# [distant] vecteurs de requête puis fidélité HNSW / exact
# Les gabarits $HNSW_COMMAND, $HNSW_COMMAND_EF200 et $EXACT_COMMAND sont définis
# dans « Gabarits de fidélité pour le mode sql », avant ce protocole.
"$PROD_REPO/.venv/bin/python" /tmp/mes00/vecteurs_requetes.py \
  --queries /tmp/mes00/out/queries.json --per-corpus 3 \
  --output /tmp/mes00/out/query-vectors.json

"$PROD_REPO/.venv/bin/python" /tmp/mes00/fidelite_hnsw.py \
  --mode sql --queries /tmp/mes00/out/query-vectors.json \
  --hnsw-command-json "$HNSW_COMMAND" --exact-command-json "$EXACT_COMMAND" \
  --max-queries 24 --k 10 --timeout 600 \
  --output /tmp/mes00/out/fidelity-ef40.json

"$PROD_REPO/.venv/bin/python" /tmp/mes00/fidelite_hnsw.py \
  --mode sql --queries /tmp/mes00/out/query-vectors.json \
  --hnsw-command-json "$HNSW_COMMAND_EF200" --exact-command-json "$EXACT_COMMAND" \
  --max-queries 24 --k 10 --timeout 600 \
  --output /tmp/mes00/out/fidelity-ef200.json

# [distant] contre-mesure indépendante avec le harnais de Legifacile
"$PROD_REPO/.venv/bin/python" "$PROD_REPO/scripts/db/froid_decomposition.py" \
  --series 1 --repetitions 2 --attente-min 10 \
  --perimetres jurisprudence,droit_applicable --json /tmp/mes00/out/froid.json
```

Les gabarits de fidélité sont définis dans « Gabarits de fidélité pour le mode
`sql` », avant ce protocole. `$HNSW_COMMAND` (ef=40) omet `SET LOCAL
hnsw.ef_search` ; `$HNSW_COMMAND_EF200` le fixe à 200 ; `$EXACT_COMMAND`
désactive `enable_indexscan` et `enable_bitmapscan`.

La requête d'export des titres (une instruction, `SELECT`, aucune donnée
personnelle) :

```sql
WITH s AS (
  SELECT DISTINCT ON (corpus, src_id) corpus, src_id, titre_court AS text
  FROM vec.passages
  WHERE titre_court IS NOT NULL
    AND length(btrim(titre_court)) >= 25
    AND abs(hashtext(src_id)) % 100 < 5
  ORDER BY corpus, src_id, passage_ord
), t AS (
  SELECT s.*, row_number() OVER (PARTITION BY corpus ORDER BY md5(src_id)) AS rn
  FROM s
)
SELECT src_id AS id, corpus, text FROM t WHERE rn <= 200 ORDER BY corpus, text
```

À la fin, contrôler le dépôt et les fichiers ignorés :

```bash
# [distant]
date -u > /tmp/mes00/out/status-after.date
git --no-optional-locks -C "$PROD_REPO" status --porcelain \
  > /tmp/mes00/out/porcelain-after.txt
test ! -s /tmp/mes00/out/porcelain-after.txt
git --no-optional-locks -C "$PROD_REPO" status --porcelain --ignored=matching -uall \
  > /tmp/mes00/out/ignored-after.txt
diff -u /tmp/mes00/out/ignored-before.txt /tmp/mes00/out/ignored-after.txt
# Inventaire fichier par fichier des ignorés, après, puis comparaison.
git --no-optional-locks -C "$PROD_REPO" ls-files --others --ignored --exclude-standard \
  | sort > /tmp/mes00/out/ignored-files-after.txt
diff -u /tmp/mes00/out/ignored-files-before.txt /tmp/mes00/out/ignored-files-after.txt
# Fichiers modifiés depuis le début de la campagne : chaque ligne rendue doit
# venir de la production, jamais de la mesure (en particulier aucun __pycache__).
cd "$PROD_REPO" && find . -path ./.git -prune -o -type f \
  -newermt "$(cat /tmp/mes00/out/campaign-start.txt)" -print | sort
```

`git status --porcelain` doit être vide. Le `diff` des **entrées de répertoire**
`--ignored=matching -uall` doit être vide, mais il ne prouve rien fichier par
fichier : un fichier ajouté dans `logs/` ne change pas cette liste. Les contrôles
fichier par fichier sont le `diff` des inventaires `git ls-files --others
--ignored --exclude-standard` (vide) et la liste `find` des fichiers modifiés
depuis le début de la campagne. Chaque ligne du `find` doit être expliquée par
la production (journal d'un service), et aucune ne doit venir de la mesure —
en particulier aucun `__pycache__` dans `$PROD_REPO`. Aucun service de
`$PROD_SERVICES` n'est redémarré.

Rapatrier les JSON sous `docs/mesures/` uniquement s'ils ne contiennent ni texte
de requête d'utilisateur ni valeur de `.env.prod` ; sinon citer leur empreinte
et les garder dans `data/` (gitignoré).

## Tests locaux

Les tests ne touchent ni au réseau ni à une base. Ils couvrent les parties
pures : percentiles, garde-fous de chemin et de variables (dernière valeur de
`default_transaction_read_only`), racine de dépôt, génération déterministe,
sélection du périmètre, agrégation de latence (censure incluse), validation des
noms de corpus dans les deux modes, calcul du recall et scan exact par blocs.

```bash
cd tools
uv sync
uv run pytest bench/mes00/tests
```
