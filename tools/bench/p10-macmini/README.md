# Harnais de la campagne Mac mini — `MESURER-SUR-MAC-MINI` (P10)

Harnais des **passes A, B et C** de la tâche `MESURER-SUR-MAC-MINI`. La passe A installe Crible sur
le Mac mini, transfère l'image, les modèles et les six instantanés, restaure et démarre le serveur.
La passe B porte la **latence contraignante** et les lectures : `crible eval` en conteneurs éphémères
sur le volume restauré, plus le contrôle du top-10. La passe C refait MES-02, MES-03, MES-05 et
MES-09 en conteneur, rejoue la recette INV-12 et lit la latence du serveur avec un cache de requêtes
vide. Il suit ADR-0141, qui remplace pour cette seule tâche les clauses d'ADR-0129 réservant
l'installation au propriétaire.

Les scripts ne touchent ni Supabase, ni le dépôt Legifacile, ni Docker Desktop. Ils ne suppriment
rien. Aucune valeur de `.env.prod` n'est affichée : `common.sh` la charge et `ssh` la reçoit par le
nom de variable `$PROD_SSH`.

| Fichier | Rôle |
|---|---|
| `common.sh` | racine du dépôt, chargement de `.env.prod`, chemins du SSD, noms des volumes, port, ordre des index |
| `host-before.sh` | relevé de l'hôte : espace, swap, pression mémoire, conteneurs, volumes, taille de la VM |
| `copy-image.sh` | `docker save \| ssh … docker load` et comparaison des identifiants d'image |
| `copy-models.sh` | volume `crible-p10-models` à distance, copie du contenu par `tar`, SHA-256 des deux côtés |
| `copy-snapshots.sh` | les six dossiers sur le SSD, un index à la fois, le plus gros d'abord, `rsync -a --partial` |
| `verify-arrival.sh` | taille de chaque fichier contre `snapshot.json`, sur l'hôte distant |
| `check-arrival.py` | programme exécuté à distance par `verify-arrival.sh` (bibliothèque standard seule) |
| `restore-all.sh` | `crible restore` des six instantanés dans `crible-p10-data`, conteneur `--rm` au premier plan |
| `inspect-all.sh` | `crible inspect` des six index, serveur arrêté |
| `start-server.sh` | configuration copiée, conteneur `crible-p10`, port `127.0.0.1:55400`, attente de `/ready`, `docker stats` |
| `checks.py` | lecture des rapports de restauration et comparaison aux comptes de référence |
| `tests/test_checks.py` | tests pytest de `checks.py` |
| `macmini.py` | fonctions pures de la passe B : corpus → index et filtres, percentiles au rang le plus proche, groupement, rappel, comparaison de top-10 |
| `build_pass_b_inputs.py` | écrit les cinq jeux d'entrée par index et les descripteurs du top-10 sous `data/p10-macmini/inputs/` |
| `run_pass_b.sh` | pilote de la passe B : relevés d'hôte, arrêt et redémarrage du seul conteneur, séries, top-10, transferts |
| `top10.sh` | boucle de `crible search` exécutée dans le conteneur, une requête par fichier, une réponse JSON par requête |
| `aggregate_pass_b.py` | agrège les 30 rapports, les deux relevés de top-10 et les contrôles des JSON versionnés en un JSON |
| `tests/test_pass_b.py` | tests pytest du code pur de la passe B |
| `Dockerfile.harness` | image `crible-p10-harness:dev` : les exemples `dense_latency` et `measure_queries` greffés sur le runtime de `crible:p10` |
| `run_pass_c.sh` | pilote de la passe C : scans, rescorings, encodages, latence directe |
| `run_pass_c_mes09.sh` | relevés MES-09 de l'instance servie (cgroup et `/proc/1/status`) |
| `run_recette.sh` | recette INV-12 sur `dole`, branche d'acceptation, dans le volume distinct `crible-p10-recette` |
| `build_refus_batch.py` | fabrique un lot qui déclare un autre modèle, pour la branche de refus d'INV-12 |
| `run_refus.sh` | attache ce lot dans `crible-p10-recette` et vérifie `incompatible_model` sans génération nouvelle |
| `direct_latency.py` | latence d'API de `POST /v1/indexes/<index>/search` avec `debug`, cache de requêtes vide, un POST par requête |
| `aggregate_pass_c.py` | recalcule les percentiles, refuse un chiffre divergent et écrit les quatre JSON versionnés |
| `tests/test_pass_c.py` | tests pytest du code pur de la passe C |

## Ordre d'exécution

```sh
sh tools/bench/p10-macmini/host-before.sh
sh tools/bench/p10-macmini/copy-image.sh
sh tools/bench/p10-macmini/copy-models.sh
sh tools/bench/p10-macmini/copy-snapshots.sh        # 60 à 90 min au débit mesuré
sh tools/bench/p10-macmini/restore-all.sh
sh tools/bench/p10-macmini/inspect-all.sh
sh tools/bench/p10-macmini/start-server.sh
cd tools && uv run --no-sync pytest bench/p10-macmini/tests -q
```

Après la restauration, les comptes se relisent ainsi :

```sh
cd tools && uv run --no-sync python -c "
import sys, pathlib; sys.path.insert(0, 'bench/p10-macmini')
from checks import parse_report, totals, compare_to_reference
reports = [parse_report(l) for p in pathlib.Path('../data/p10-macmini').glob('restore-*.txt')
           for l in p.read_text().splitlines() if l.startswith('index ')]
print(totals(reports)); print(compare_to_reference(reports))"
```

Résultat attendu : `{'documents': 6660239, 'passages': 9600105}` et `[]` (ADR-0141, décision 2).

## Passe B — latence

La recette de la passe B est décrite dans `docs/mesures/latence-mac-mini-2026-11-07.md`. Cinq séries
par index, une passe d'échauffement jetée puis `--repeat 3`, `--exact-threshold 5000` (la valeur de
l'index : sans elle, `eval` force le binaire) :

| Passe | Mode | Requêtes | Remarque |
|---|---|---|---|
| `dense-texte` | dense | les 97 en texte | **contraignante** (ADR-0141, décision 4) |
| `dense-vecteur` | dense | les 97, vecteurs de pgvector | contrôle du classement |
| `hybride` | hybride `rrf` | les 97 en texte | rapportée, non contraignante |
| `dense-texte-froid` | dense | les 97 en texte | purge globale avant la série, résidence non mesurée |
| `mes00-1000-titres` | dense | les 1 000 titres, par index | lecture complémentaire |

Résultat de la séance du 2026-11-07 : le critère de la décision 4 est le `total` chaud de
`crible eval` local, **79,355 ms** (plus grand p95 par index) pour la série contraignante ;
l'encodage y pèse 0,005 à 0,024 ms au p95 parce que le cache de requêtes sert les textes répétés.
Rappel@10 du dense en texte 0,500 / 0,322581 / 0,750 ; top-10 identique à la machine de
développement sur 97 requêtes sur 97. Le groupé exact n'est pas calculable : les rapports `eval` ne
portent aucun échantillon par requête (ADR-0125, décision 9). L'**estimation prudente** vaut
**358,959 ms** (79,355 + 279,604, p95 fp32 sans cache sur 300 textes, donc pas un majorant strict
pour les 97), et le **recoupement API** vaut **418,425 ms p95** à cache vide. Les trois lectures
sont sous 698 ms.

## Passe C — MES-02, MES-03, MES-05, MES-09 et recette

La recette de la passe C est décrite dans `docs/mesures/mac-mini-2026-11-07.md`. L'image de harnais
se construit depuis la racine du dépôt, puis se charge sur le Mac mini :

```sh
docker build -f tools/bench/p10-macmini/Dockerfile.harness -t crible-p10-harness:dev .
docker save crible-p10-harness:dev | ssh "$PROD_SSH" 'export PATH=/usr/local/bin:$PATH; docker load'
```

Les séries, un conteneur `--rm` par invocation :

| Commande | Mesure | Sortie |
|---|---|---|
| `run_pass_c.sh scan decisions <n>` | MES-02, 3 589 776 vivants, 4 segments | `scan-{main,all}-n<n>.json` |
| `run_pass_c.sh scan articles <n>` | MES-02, 2 476 715 vivants, 1 segment | `scan-articles-{main,all}-n<n>.json` |
| `run_pass_c.sh rescore-warm <n>` | MES-03 chaud, `articles`, 4 passes dont la première est jetée | `rescore-warm-articles-n<n>.json` |
| `run_pass_c.sh rescore-cold <n>` | MES-03 froid partiel, `--cold --evict`, une passe | `rescore-cold-articles-n<n>.json` |
| `run_pass_c.sh rescore-split` | lecture des clés seule contre produit scalaire seul | `rescore-warm-articles-dot-keys.json` |
| `run_pass_c.sh encode <int8\|fp32> <run>` | MES-05, cache de requêtes à capacité 0 | `mes05-encode-<precision>-<run>.json` |
| `run_pass_c.sh direct <jeton>` | latence d'API du serveur, cache vide, un POST par requête | `direct-latency.json` |
| `run_pass_c_mes09.sh record rest` | MES-09, six index ouverts, aucune requête | `mes09/rest.txt` |
| `run_pass_c_mes09.sh record after-load` | MES-09, après les 97 requêtes | `mes09/after-load.txt` |
| `run_recette.sh` | INV-12 sur `dole` | sortie `attach` et `inspect` |

La recette exerce les deux branches d'INV-12. L'acceptation est `run_recette.sh` ; le refus se
prépare et s'exécute ainsi, toujours dans `crible-p10-recette` :

```sh
python3 tools/bench/p10-macmini/build_refus_batch.py \
    --source data/legifacile-full/dole/dole-full-v1 \
    --out data/p10-macmini/pass-c/refus/dole-refus-model
sh tools/bench/p10-macmini/run_refus.sh
```

Le lot ne change que `batch_id` et `model.hf_id` : l'attache doit rendre `incompatible_model` avant
toute recette et laisser la génération inchangée.

Le rescoring de `decisions` n'est **pas** mesuré : l'index a quatre segments et le harnais
rescorerait contre les vecteurs du premier seulement. Les fichiers `rescore-warm-n*.json` de cet
index existent comme **tentative invalide** ; `aggregate_pass_c.py` les déclare et ne publie aucun
chiffre. MES-03 ne porte donc que sur `articles`, à un segment.

`direct_latency.py` s'exécute sur le Mac mini avec la bibliothèque standard seule. Il redémarre le
serveur avant la série pour vider le cache de requêtes, jette un appel d'échauffement, puis envoie
un POST par requête de référence avec `debug: true`. Les textes répétés sont des succès de cache :
le script les marque `first_occurrence: false` et publie la série **unique** à côté de la série
complète. Le chiffre est un aller-retour client (`latency_basis = "client_round_trip"`), non
comparable directement au total en processus de MES-00 (ADR-0131, décision 7). Aucun texte n'est
écrit.

```sh
# jeton hors du dépôt et hors de /data, retiré après la série
scp data/p10-import/token.txt "$PROD_SSH:$BASE/pass-c/token.txt"
ssh "$PROD_SSH" "chmod 600 $BASE/pass-c/token.txt"
sh tools/bench/p10-macmini/run_pass_c.sh direct "$BASE/pass-c/token.txt"
ssh "$PROD_SSH" "rm -f $BASE/pass-c/token.txt"
```

L'agrégateur refuse un percentile qui s'écarte du résumé du harnais et écrit :

```sh
python3 tools/bench/p10-macmini/aggregate_pass_c.py
cd tools && uv run --no-sync pytest -q bench/p10-macmini/tests
```

## Ce que la passe A a montré

Chiffres de la séance du 2026-11-07 ; le détail est dans
`docs/mesures/installation-mac-mini-2026-11-07.md`.

- **Débit** : 4 à 5 Mo/s sur le lien, mesuré trois fois (image 35 s pour 182 Mo, `tar` des modèles
  421 s pour 1,5 GiB, `rsync` 3,9 Mo/s en moyenne). Le lien est le facteur limitant, pas le chiffrement :
  trois chiffrements essayés, même débit.
- **Découpage obligatoire** : à ce débit, l'index `decisions` (9,88 Go) dépasserait les 25 minutes
  d'une commande. `copy-snapshots.sh` copie donc les fichiers de tête puis chaque sous-dossier de
  `segments/` séparément.
- **Arrivée** : 282 fichiers, 25 480 066 488 octets, aucune taille fausse. Le blake3 n'est pas
  calculable sur le Mac mini (ni `b3sum`, ni module `blake3`) : le contrôle de contenu est celui de
  `crible restore`, comme le prévoit ADR-0141.
- **Restauration** : 1 s à 82 s par index, 0 époque remplacée, six rapports conservés.
- **Serveur** : prêt en 60 s, 3,67 GiB de mémoire au repos pour 9,70 GiB de VM.

## Ce que le harnais ne fait pas

- Il n'arrête ni ne démarre Supabase : `host-before.sh` le relève arrêté, et rien d'autre.
- Il ne pose aucun plafond `--memory` : la RAM de la VM est la borne (ADR-0135, décision 5 ;
  ADR-0139, décision 3).
- Il ne supprime aucun instantané du dossier d'échange ; ADR-0141, décision 6, les garde jusqu'à la
  décision du propriétaire.
- Il n'écrit aucune MES-xx : la latence contraignante n'a pas de numéro, sa référence est MES-00. La
  passe B ne fait aucun geste sur Supabase, ouvre les index en lecture seule et ne laisse aucune
  écriture durable sous `/data`. La passe C ouvre MES-02 et MES-03 en **écriture** avec la reprise
  (`dense_latency` → `GenerationStore::open`), permise par le complément d'ADR-0141 ; MES-05
  n'ouvre **aucun index** et ne monte que `/models:ro`.
