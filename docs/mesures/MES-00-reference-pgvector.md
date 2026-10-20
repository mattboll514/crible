# MES-00 — Référence pgvector actuelle de Legifacile : qualité, fidélité HNSW et latence

Ce document est le rapport de référence de la mesure MES-00. **La campagne du 2026-10-20 est la
seule campagne de référence** ; la passe du 2026-10-19 est remplacée et résumée en annexe. Les
résultats de référence des sections 1 à 7 et les fichiers de la section 9 viennent de la passe du
20 ; les cibles retenues sont fixées dans `docs/suivi.md`, et la section 8 conserve les propositions
initiales de cette passe.

- Tâche : `MESURER-REFERENCE-PGVECTOR`
- Commit : `98355e5` (dépôt Crible) ; mesure exécutée sur le dépôt Legifacile `8ed5321`
- Machine : Mac mini (Apple M1, 8 cœurs : 4 performance, 4 efficacité), 16 Go de RAM,
  macOS 26.5.2. PostgreSQL tourne dans Docker Desktop 29.6.2 (VM de 8 vCPU et 7,75 Gio de RAM),
  conteneur PostgreSQL 17. Toutes les mesures sont prises depuis l'hôte, par SSH (`$PROD_SSH`),
  en lecture seule.
- Jeu de données : `mes00-titres-v1`, 1 000 requêtes tirées de titres d'articles et de sommaires,
  graine `20261019`. Empreinte canonique des requêtes :
  `d01d6c78829be047a1b3352c959ce60c0b11ac7450afb522fa3b872ffc753696`. Répartition : `dole` 111,
  `jorf_normatif` 125, `juri_sommaires` 129, `juri_texte_integral` 122, `sarde` 129,
  `textes_hors_vigueur` 119, `textes_vigueur` 127, `titres_textes` 138.
- Configuration : `k = 10`, `p_dedupliquer = document`, `hnsw.ef_search = 200` (posé par
  `api.recherche_semantique`), `p_partie` au défaut du serveur, requêtes strictement en série.
- Cinq mesures successives, toutes hors de `$PROD_DAILY_WINDOW` (heures locales) : latence chaude
  07:35–07:44, latence froide 07:44–09:13, fidélité 09:16–09:19, contre-mesure 09:20–09:40,
  éval qualité 09:41–09:42.

## 1. État de la base mesurée

Relevé en lecture seule le 2026-10-20 à 09:41 (CEST), en fin de campagne.

| Élément | Valeur |
|---|---|
| Passages (`vec.passages`) | 9 589 652 |
| `p_juri_texte_integral` | 2 996 989 |
| `p_jorf_normatif` | 1 831 201 |
| `p_textes_hors_vigueur` | 1 488 729 |
| `p_titres_textes` | 1 476 030 |
| `p_textes_vigueur` | 984 336 |
| `p_juri_sommaires` | 590 161 |
| `p_sarde` | 214 578 |
| `p_dole` | 7 628 |
| Partitions | 8 (`vec.passages` déclarée `PARTITION BY LIST`) |
| PostgreSQL | 17.6 (aarch64) |
| pgvector | 0.8.2 |
| Type de vecteur | `halfvec(768)` |
| Index | un HNSW `halfvec_cosine_ops` par partition (8) |
| `shared_buffers` | 16 384 pages de 8 ko = 128 Mio |
| `work_mem` / `maintenance_work_mem` | 4 Mio / 64 Mio |
| `max_parallel_workers_per_gather` | 2 |
| `statement_timeout` du chemin applicatif | 8 000 ms (`serveur_mcp.base.TIMEOUT_MS`) |
| Base | 102 Go |
| Charge (1/5/15 min) | 2,43 / 2,03 / 1,85, relevée à 09:41 |
| Conteneurs | 6 services `Up 13 days`, `RestartCount` 0, `StartedAt` 2026-10-06T18:24:29Z |
| `transaction_read_only` | `on` (vérifié sur chaque connexion par le harnais) |

Le total des passages a augmenté de 635 par rapport à la passe remplacée (9 589 017 → 9 589 652),
réparti sur `juri_texte_integral` (+57), `jorf_normatif` (−285), `textes_vigueur` (+439),
`titres_textes` (+149), `textes_hors_vigueur` (+275) ; les autres partitions sont inchangées. Cette
dérive est normale (synchronisation quotidienne) et n'affecte pas la comparaison : les 1 000
requêtes sont identiques (même empreinte), les 24 requêtes de fidélité rejouées ont rendu les mêmes
résultats (comparaison faite pendant la campagne, non conservée, cf. §3), et les 57 questions sont
inchangées.

La charge n'a été relevée qu'à la fin ; aucune mesure de charge au début n'a été prise, ce qui est
une faiblesse du protocole (corrigée en pratique : les cinq mesures sont hors fenêtre quotidienne
et la machine n'a reçu aucun autre travail).

Note matérielle : `system_profiler` et `docker info` donnent 16 Go d'hôte et 7,75 Gio de VM, alors
que `sysctl hw.memsize` renvoie 25 769 803 776 octets (24 Gio). L'écart n'est pas expliqué ; les
chiffres retenus sont ceux de `system_profiler`, cohérents avec l'allocation Docker.

## 2. Qualité sur les 57 questions étiquetées

Mesurée par `eval_recherche.py --k 10 --dedupliquer document` le 2026-10-20 à 09:41, SQL direct :
57 questions, dont 56 « satisfiable » et 1 non satisfiable (`Q30`, comptée à part). Aucune erreur
d'appel.

- **rappel@10 moyen : 0,482** sur 56 questions (27 à rappel plein, 29 à rappel nul).
- **rang du premier document pertinent : médiane 2** sur les 27 questions où un attendu est trouvé.
- hors-vigueur non signalés : 6/130 (4,62 %), tous sur `textes_vigueur`.
- `Q30` (non satisfiable) rend un rappel non nul : c'est une anomalie signalée dans le rapport
  brut, comptée à part et jamais mêlée aux moyennes.

| Corpus | Questions | rappel@10 moyen | rappel plein | rang médian | trouvés |
|---|---|---|---|---|---|
| `juri_texte_integral` | 31 | 0,290 | 9 | 3,0 | 9 |
| `textes_vigueur` | 12 | 0,750 | 9 | 2,0 | 9 |
| `juri_sommaires` | 4 | 0,500 | 2 | 1,0 | 2 |
| `jorf_normatif` | 4 | 0,500 | 2 | 2,0 | 2 |
| `sarde` | 3 | 1,000 | 3 | 1,0 | 3 |
| `dole` | 2 | 1,000 | 2 | 2,5 | 2 |

| Périmètre | Questions | rappel@10 moyen | rappel plein | rang médian | trouvés |
|---|---|---|---|---|---|
| `jurisprudence` | 35 | 0,314 | 11 | 2,0 | 11 |
| `droit_en_vigueur` | 12 | 0,750 | 9 | 2,0 | 9 |
| `journal_officiel` | 4 | 0,500 | 2 | 2,0 | 2 |
| `index_thematique` | 3 | 1,000 | 3 | 1,0 | 3 |
| `dossiers_legislatifs` | 2 | 1,000 | 2 | 2,5 | 2 |

Les questions de `textes_vigueur` sont appelées en mono-corpus : le périmètre qui les contient
exactement est `droit_en_vigueur`, pas `droit_applicable` (deux corpus). `textes_hors_vigueur` et
`titres_textes` ne portent aucune question ; aucune ligne n'est inventée pour eux, et
`droit_applicable` n'est mesuré par aucune question.

**Reproductibilité** : pendant la campagne, les 57 lignes par question (rappel et rang) ont été
comparées à celles de la passe remplacée : zéro différence, question par question, malgré la dérive
de 635 passages. **Cette comparaison n'est plus vérifiable après coup** : les fichiers bruts du 19
ont été supprimés (§9) et seuls ceux du 20 sont conservés. La qualité est donc stable à jeu de
questions constant, sous cette réserve de traçabilité.

## 3. Fidélité HNSW contre recherche exacte

24 requêtes (3 par corpus), même vecteur des deux côtés, `k = 10`, identifiants comparés au niveau
**passage** (`src_id#passage_ord`). Recall = |exact ∩ HNSW| / |exact|. Mode `sql` : plan exact
obtenu par `enable_indexscan = off` et `enable_bitmapscan = off` (`Seq Scan` + `Sort`), plan HNSW
par l'index de la partition.

| Corpus | n | ef=40 (défaut) moyenne / min | ef=200 moyenne / min |
|---|---|---|---|
| `dole` | 3 | 0,933 / 0,90 | 1,000 / 1,00 |
| `jorf_normatif` | 3 | 1,000 / 1,00 | 1,000 / 1,00 |
| `juri_sommaires` | 3 | 0,633 / 0,00 | 0,767 / 0,40 |
| `juri_texte_integral` | 3 | 0,333 / 0,00 | 0,667 / 0,00 |
| `sarde` | 3 | 1,000 / 1,00 | 1,000 / 1,00 |
| `textes_hors_vigueur` | 3 | 0,633 / 0,00 | 0,967 / 0,90 |
| `textes_vigueur` | 3 | 1,000 / 1,00 | 1,000 / 1,00 |
| `titres_textes` | 3 | 0,967 / 0,90 | 1,000 / 1,00 |
| **Global** | 24 | **0,812** / 0,00 | **0,925** / 0,00 |

**Les 24 requêtes donnaient des résultats identiques à la passe remplacée** lors de la comparaison
faite pendant la campagne : mêmes moyennes, mêmes minima, mêmes requêtes sous 1,0 (`q0004`,
`q0006`, `q0010`, `q0012`, `q0015`, `q0018`, `q0019`, `q0030`, `q0042` à ef=40 ; `q0010`, `q0012`,
`q0018`, `q0019` à ef=200), aucune valeur modifiée ; seule la date du fichier différait. **Cette
identité n'est pas conservée** : les fichiers du 19 ont été supprimés (§9), donc elle n'est plus
vérifiable ; les valeurs citées sont celles du 20.

### Observation `q0018` à `ef_search = 1000` (sortie brute conservée)

`q0018` (`juri_texte_integral`) reste à **recall@10 = 0,00** à `ef=40`, `ef=200` **et** `ef=1000`.
Les dix identifiants HNSW sont disjoints des dix identifiants exacts, et les distances restent
nettement plus mauvaises des deux côtés du top 10 :

| | rang 1 | distance |
|---|---|---|
| HNSW, `ef_search = 1000` | `CETATEXT000044272723#0` | 0,388099 |
| Exact (`Seq Scan` + `Sort`) | `CETATEXT000043851789#0` | 0,310845 |

Les deux top 10 complets, avec distances, sont dans
`MES-00-reference-pgvector.2026-10-20.q0018-ef1000.txt` — cette fois c'est une **sortie brute**,
pas une note recopiée, et le fichier porte en tête l'argv qui l'a produite (§7.1). Elle montre que
`ef_search` n'est pas la cause : le graphe de la partition
`juri_texte_integral` ne relie pas cette région à ses vrais plus proches voisins. Cause exacte non
établie (couverture de l'index, ordre d'insertion, ou graphe dégradé) ; c'est une question pour
MES-01, pas une conclusion de cette mesure.

## 4. Latence décomposée

Postes : encodeur (`encodage.encoder_requete`), SQL (`api.recherche_semantique`), reconstruction
(`api.source_vectorielle` + `encodage.passage_exact` + `rendu.plafonner_passage`, pour les 10
résultats).

**Base des percentiles.** 1 000 requêtes mesurées au total ; deux passes avec le même fichier :
une passe chaude (`--idle-minutes-per-corpus 0`, 1 000 mesures chaudes) et une passe froide
(`--idle-minutes-per-corpus 10`) qui contient **8 mesures froides** (la première requête de chaque
corpus, après 10 min d'inactivité) et 992 mesures chaudes supplémentaires. Les p99 « chaud »
portent sur 1 000 mesures (passe chaude) ; les p99 « froid » sur 8. Aucun p99 mixte n'est calculé :
les deux populations n'ont pas la même définition. Aucune mesure censurée par `statement_timeout`
(0 sur 1 000, dans les deux passes).

### 4.1 Vue mono-corpus (1 000 requêtes)

| État | Poste | n | p50 | p95 | p99 | max |
|---|---|---|---|---|---|---|
| chaud | encodeur | 1 000 | 33 | 50 | 61 | 75 |
| chaud | SQL | 1 000 | 160 | 402 | 546 | 1 164 |
| chaud | reconstruction | 1 000 | 37 | 480 | 786 | 1 537 |
| chaud | **total** | 1 000 | **279** | **698** | **1 112** | **1 631** |
| froid | encodeur | 8 | 105 | 121 | 121 | 121 |
| froid | SQL | 8 | 269 | 375 | 394 | 398 |
| froid | reconstruction | 8 | 130 | 713 | 724 | 726 |
| froid | **total** | 8 | **483** | **1 163** | **1 196** | **1 204** |

Part moyenne de chaque poste dans le total, à chaud : SQL 56,0 %, reconstruction 32,8 %,
encodeur 11,1 %. À froid : encodeur 17,5 %, SQL 42,6 %, reconstruction 39,9 %.

À chaud, 125 requêtes dépassent 500 ms et 15 dépassent 1 000 ms ; la plus lente fait 1 631 ms,
loin du plafond de 8 000 ms.

| Corpus (chaud) | n | total p50 / p95 / p99 | SQL p50 / p95 / p99 | reconstruction p50 / p95 / p99 | encodeur p50 / p95 / p99 |
|---|---|---|---|---|---|
| `dole` | 111 | 238 / 942 / 1 392 | 16 / 27 / 57 | 190 / 883 / 1 335 | 32 / 59 / 67 |
| `jorf_normatif` | 125 | 380 / 846 / 1 107 | 275 / 460 / 552 | 39 / 523 / 759 | 38 / 50 / 52 |
| `juri_sommaires` | 129 | 144 / 372 / 593 | 91 / 266 / 425 | 20 / 102 / 155 | 32 / 40 / 49 |
| `juri_texte_integral` | 122 | 332 / 932 / 1 254 | 155 / 543 / 809 | 155 / 539 / 678 | 30 / 49 / 59 |
| `sarde` | 129 | 149 / 281 / 380 | 68 / 171 / 188 | 30 / 121 / 207 | 38 / 58 / 61 |
| `textes_hors_vigueur` | 119 | 282 / 507 / 725 | 177 / 315 / 422 | 45 / 252 / 298 | 34 / 42 / 50 |
| `textes_vigueur` | 127 | 318 / 501 / 654 | 211 / 373 / 423 | 49 / 231 / 304 | 33 / 42 / 44 |
| `titres_textes` | 138 | 304 / 504 / 544 | 254 / 454 / 495 | 16 / 21 / 25 | 32 / 39 / 45 |

La reconstruction domine `dole` (p95 883 ms) : passages longs, coût du tokenizer et du
plafonnement, pas du SQL (p95 27 ms sur 7 628 passages).

**Froid par corpus.** Une seule mesure froide par corpus : p50 = p95 = p99 = max.

| Corpus (froid) | n | encodeur | SQL | reconstruction | total |
|---|---|---|---|---|---|
| `dole` | 1 | 120 | 243 | 726 | 1 089 |
| `jorf_normatif` | 1 | 115 | 398 | 690 | 1 204 |
| `juri_sommaires` | 1 | 95 | 331 | 30 | 456 |
| `juri_texte_integral` | 1 | 121 | 199 | 191 | 511 |
| `sarde` | 1 | 98 | 119 | 34 | 251 |
| `textes_hors_vigueur` | 1 | 113 | 300 | 72 | 485 |
| `textes_vigueur` | 1 | 97 | 197 | 188 | 481 |
| `titres_textes` | 1 | 94 | 295 | 18 | 407 |

**Note de cohérence.** Les effectifs par corpus totalisent bien 1 000 à chaud et 8 à froid. Les
parts chaudes arrondies totalisent 99,9 % ; les parts froides 100,0 %. Les totaux sont arrondis
indépendamment des postes : la somme des postes arrondis diffère du total arrondi de 1 ms pour
`jorf_normatif` (1 203 vs 1 204), `textes_vigueur` (482 vs 481), et pour les périmètres froids
`droit_applicable` (408 vs 407), `index_thematique` (133 vs 132), `jurisprudence` (423 vs 422) et
`titres` (111 vs 112). Ces écarts sont des effets d'arrondi, pas des mesures manquantes.

### 4.2 Vue périmètre (fusion)

Même passe, autre vue : chaque requête est mesurée d'abord en mono-corpus, puis en fusion. Le
second appel retrouve les pages HNSW chargées par le premier — il est donc **biaisé à la baisse**
sur le poste SQL. La campagne séparée `--perimeter-first` de la passe remplacée (491 requêtes)
**n'a pas été rejouée** ; les chiffres de fusion ci-dessous ne mesurent donc pas le coût réel de la
fusion, surtout sur `journal_officiel` (p50 82 ms).

| Périmètre (chaud) | n | total p50 / p95 / p99 | SQL p50 / p95 / p99 | reconstruction p50 / p95 / p99 | encodeur p50 / p95 / p99 |
|---|---|---|---|---|---|
| `dossiers_legislatifs` | 111 | 229 / 929 / 1 381 | 10 / 17 / 18 | 183 / 887 / 1 326 | 32 / 59 / 67 |
| `droit_applicable` | 246 | 323 / 541 / 682 | 226 / 365 / 482 | 51 / 247 / 321 | 33 / 42 / 47 |
| `index_thematique` | 129 | 77 / 128 / 219 | 9 / 11 / 14 | 29 / 69 / 160 | 38 / 58 / 61 |
| `journal_officiel` | 125 | 82 / 519 / 749 | 9 / 14 / 18 | 33 / 469 / 698 | 38 / 50 / 52 |
| `jurisprudence` | 251 | 246 / 666 / 899 | 113 / 401 / 761 | 73 / 451 / 600 | 31 / 47 / 59 |
| `titres` | 138 | 56 / 67 / 80 | 8 / 11 / 12 | 15 / 21 / 23 | 32 / 39 / 45 |

Global fusion, même passe : total p50 177 / p95 622 / p99 884 / max 1 582 ms ; parts encodeur
14,9 %, SQL 41,5 %, reconstruction 43,6 %.

**Froid par périmètre.** `n = 1` pour les quatre périmètres mono-corpus, `n = 2` pour
`jurisprudence` et `droit_applicable` (une mesure froide par corpus). Pour `n = 2`, p50 est la
moyenne des deux valeurs et p95/p99 sont interpolés : ce n'est pas une distribution.

| Périmètre (froid) | n | encodeur p50 / p95 / p99 | SQL p50 / p95 / p99 | reconstruction p50 / p95 / p99 | total p50 / p95 / p99 |
|---|---|---|---|---|---|
| `dossiers_legislatifs` | 1 | 120 / 120 / 120 | 15 / 15 / 15 | 627 / 627 / 627 | 762 / 762 / 762 |
| `droit_applicable` | 2 | 105 / 112 / 113 | 170 / 202 / 205 | 133 / 182 / 187 | 407 / 482 / 489 |
| `index_thematique` | 1 | 98 / 98 / 98 | 7 / 7 / 7 | 28 / 28 / 28 | 132 / 132 / 132 |
| `journal_officiel` | 1 | 115 / 115 / 115 | 18 / 18 / 18 | 614 / 614 / 614 | 747 / 747 / 747 |
| `jurisprudence` | 2 | 108 / 120 / 121 | 235 / 355 / 365 | 80 / 132 / 137 | 422 / 479 / 484 |
| `titres` | 1 | 94 / 94 / 94 | 7 / 7 / 7 | 10 / 10 / 10 | 112 / 112 / 112 |

## 5. Contre-mesure avec `froid_decomposition.py`

Une série, deux répétitions, 10 minutes d'inactivité par périmètre, deux périmètres, mêmes
questions que la passe remplacée (`Q13`–`Q15`, `Q01`–`Q03`). Médianes du rapport du script :

| Périmètre | Poste | premier passage (froid) | répétitions, même question | répétitions, autre question |
|---|---|---|---|---|
| `jurisprudence` | encodeur | 114 ms | 49 ms | 41 ms |
| `jurisprudence` | SQL + HNSW | 1 097 ms | 23 ms | 1 018 ms |
| `jurisprudence` | reconstruction | 102 ms | 83 ms | 227 ms |
| `droit_applicable` | encodeur | 98 ms | 29 ms | 34 ms |
| `droit_applicable` | SQL + HNSW | 838 ms | 21 ms | 573 ms |
| `droit_applicable` | reconstruction | 155 ms | 106 ms | 197 ms |

Le harnais rend **`RENONCEMENT` sur les deux périmètres**. Définitions, pour lire le tableau : le
**delta par poste** vaut la médiane du premier appel froid moins la médiane sur une question
neuve à chaud ; la **somme** est la part de l'écart de référence (3 886 ms) que la contre-mesure
sait imputer ; le **reste** est ce qu'elle n'explique pas ; la colonne de droite nomme le poste de
plus grande part **positive** (une part négative signifie que le poste est plus rapide à froid,
donc qu'il n'explique rien — c'est le cas de la reconstruction sur `jurisprudence`, à −125,9 ms).

| Périmètre | Somme des deltas médians | Reste médian | Poste de plus grande part positive |
|---|---|---|---|
| `jurisprudence` | 26,1 ms | 3 859,9 ms | `B_sql_hnsw` à 2 % |
| `droit_applicable` | 287,2 ms | 3 598,8 ms | `B_sql_hnsw` à 7 % |

Référence du harnais, inchangée car elle vient de la table d'usage : écart froid-chaud
`ecart_reference_ms = 3 886` (population froide `n = 8`, écart-type 2 526 ms, erreur-type 893 ms,
IC 95 % [1 750 ; 6 022] ms). Population recalculée le 20 : chaud `n = 155`, moyenne 1 283 ms ;
froid `n = 39`, moyenne 4 216 ms. Les réserves restent valables : le froid de référence n'est pas
homogène (il enjambe une migration), l'écart n'est connu qu'à ±55 %, et le harnais ne voit ni la
pression mémoire macOS, ni le cache de la VM, ni le coût de `passage.invoquer`.

Deux enseignements qui recoupent la campagne principale :

- le coût SQL de la fusion (573–1 018 ms selon le périmètre) **n'est pas un effet de cache** : il
  persiste sur des questions différentes ; rejouer la même question ne coûte que 21–23 ms, ce qui
  mesure le cache de pages, pas la recherche ;
- l'encodeur coûte 98–114 ms au premier passage et 29–49 ms ensuite, même ordre de grandeur que le
  tableau 4.1 (105 ms froid, 33 ms chaud).

## 6. Incident de la passe remplacée et conformité de la référence

### 6.1 Incident de la passe remplacée

Trois fichiers **ignorés par git** avaient été créés dans `$PROD_REPO` pendant la passe du
2026-10-19 :

1. `logs/eval_recherche_20261019-204115.log` (écrit par `eval_recherche.py`, dont le journal
   n'obéit pas à `--sortie`) ;
2. `logs/froid_decomposition_20261019-223234.log` ;
3. `scripts/db/__pycache__/froid_decomposition.cpython-312.pyc`.

`git status --porcelain` restait vide parce que `.gitignore` couvre ces trois chemins : la
commande ne prouve que l'absence de modification **suivie**, pas l'absence d'écriture.

Le propriétaire a copié les trois fichiers sous `/tmp/mes00/sauvegarde-20261019/` avec leurs noms
d'origine, puis a supprimé exactement ces trois chemins. Aucun service n'a été redémarré. **Leur
absence n'est pas prouvée fichier par fichier par ce rapport** : l'inventaire de 05:34:55Z de la
version précédente (bloc retiré de 6.2, cf. le premier défaut décrit ci-dessous) était un
`git status --ignored` replié, qui liste `logs/` et `scripts/db/__pycache__/` comme répertoires et
ne peut donc pas distinguer ces trois chemins d'autres fichiers des mêmes dossiers. La preuve
fichier par fichier de 6.2 porte sur la fenêtre de la campagne du 20, pas sur la suppression du 19.
La recherche de `froid_decomposition*.pyc` récents après la campagne du 20 ne rend rien, ce qui ne
vaut que pour ce motif. Le détail de la passe remplacée est en annexe.

### 6.2 Preuve de la passe de référence

La version précédente de ce paragraphe avait deux défauts, corrigés ici.

- Le contrôle reposait sur une comparaison « avant / après » de `git status --ignored`, qui
  **replie les répertoires** : un fichier ajouté dans `logs/`, `scripts/db/__pycache__/` ou
  `.claude/` ne changeait pas la liste. L'égalité des listes ne prouvait donc rien **fichier par
  fichier**.
- Le bloc « avant » était annoncé « collé sans retouche » alors qu'il est **annoté** : ses deux
  premières lignes (`# 2026-10-20T05:34:55Z HEAD=8ed5321`, `# --porcelain --`) et ses marqueurs
  de section (`# --ignored --`) ont été écrits par son auteur. Ce n'est pas une sortie brute.

La preuve ci-dessous n'a **pas besoin d'un « avant »** : c'est un inventaire **après**, fichier par
fichier, relevé après la dernière mesure. Un fichier écrit pendant la campagne porterait une date
de modification dans la fenêtre de campagne, donc il apparaîtrait dans le relevé. Chaque bloc est
une sortie brute, sans annotation à l'intérieur ; la commande est donnée en clair au-dessus.

Relevé du **2026-10-20 à 10:25:15 CEST** (08:25:15 UTC), soit 43 minutes après la fin de l'éval de
qualité (09:41–09:42), la dernière écriture possible de la campagne. Une seule session SSH, en
lecture seule, hors de `$PROD_DAILY_WINDOW` ; `git` est appelé avec `--no-optional-locks` pour ne
pas réécrire `.git/index`. Tous les horodatages locaux de ce paragraphe sont en **CEST (UTC+2)**,
le fuseau du Mac mini de production.

**(a) Horodatage.** Commande : `date -u '+%Y-%m-%dT%H:%M:%SZ'; date '+%Y-%m-%dT%H:%M:%S%z'`.

```text
2026-10-20T08:25:15Z
2026-10-20T10:25:15+0200
```

**(b) Commit.** Commande : `git --no-optional-locks -C "$PROD_REPO" rev-parse --short HEAD`.

```text
8ed5321
```

C'est le commit Legifacile cité au §1.

**(c) Modifications suivies.** Commande : `git --no-optional-locks -C "$PROD_REPO" status
--porcelain`. Le bloc est vide : la commande n'écrit aucune ligne, donc aucun fichier suivi n'est
modifié ni non suivi.

```text
```

**(d) Non suivis et ignorés, par entrée.** Commande : `git --no-optional-locks -C "$PROD_REPO"
status --porcelain --ignored=matching -uall`.

```text
!! .claude/memory/
!! .claude/settings.local.json
!! .claude/worktrees/
!! .env
!! .venv/
!! data/
!! docs/ROADMAP-13-08-2026.md
!! logs/
!! rapports/
!! scripts/__pycache__/
!! scripts/db/__pycache__/
!! serveur_mcp/__pycache__/
!! supabase/.branches/
!! supabase/.temp/
!! web/dist/
!! web/node_modules/
```

`--ignored=matching` ajoute `.claude/worktrees/` par rapport à `git status --porcelain --ignored`,
qui rend aujourd'hui les 15 entrées du fichier annoté de 05:34:55Z, à l'identique. Cette entrée
supplémentaire n'est pas une écriture de la campagne : `.claude/worktrees/` est antérieur de deux
semaines (date de modification `2026-10-06T16:46:13+0200`, règle `.gitignore:53`) et ne contient
aucun fichier, seulement des répertoires vides (`web-consultation/supabase`).

**(e) Inventaire par fichier.** Commande : `git --no-optional-locks -C "$PROD_REPO" ls-files
--others --ignored --exclude-standard | wc -l`.

```text
   41736
```

Répartition par répertoire de tête de ces 41 736 fichiers (`awk -F/ '{print $1"/"}' | sort |
uniq -c | sort -rn`) :

```text
34243 .venv/
6643 web/
 709 logs/
  65 rapports/
  32 .claude/
  18 serveur_mcp/
  18 scripts/
   4 data/
   2 supabase/
   1 docs/
   1 .env/
```

La barre finale de `.env/` vient du `awk` : `.env` est un fichier. Les 41 736 lignes ne sont pas
recopiées ; le décompte et la répartition suffisent, parce que (f) donne la preuve qui compte :
aucun de ces fichiers n'a été touché pendant la campagne.

**(f) Fichiers modifiés depuis le début de la campagne.** Début de campagne : **07:35 CEST**
(§1, début de la mesure de latence chaude). La borne retenue est `2026-10-20 07:30:00`, heure
locale du Mac mini de production : cinq minutes de marge pour englober la préparation de 07:34
(inventaire, copie des scripts), et une fenêtre plus large ne peut que renforcer la preuve.
Commande : `cd "$PROD_REPO" && find . -path ./.git -prune -o -type f -newermt '2026-10-20
07:30:00' -print | sort` — `.git` est élagué et `-type f` exclut les liens symboliques.

```text
./logs/mcp_20261006-201803.log
```

Dates de modification des fichiers ci-dessus (`stat -f '%Sm %N' -t '%Y-%m-%dT%H:%M:%S%z'`) :

```text
2026-10-20T10:22:53+0200 ./logs/mcp_20261006-201803.log
```

**(g) Pour situer : tous les fichiers modifiés depuis 00:00 (heure locale).** Même commande avec
`-newermt '2026-10-20 00:00:00'`.

```text
./logs/exporter_vecteurs_20261020-054715.log
./logs/mcp_20261006-201803.log
./logs/quotidien_20261020-053001.log
./logs/quotidien.verrou
./logs/refresh_20261020-053001.log
./logs/sync_vecteurs_20261020-053808.log
./rapports/DERNIER.md
./rapports/quotidien-20261020-053001.md
```

Leurs dates de modification :

```text
2026-10-20T05:53:59+0200 ./logs/exporter_vecteurs_20261020-054715.log
2026-10-20T10:22:53+0200 ./logs/mcp_20261006-201803.log
2026-10-20T05:57:41+0200 ./logs/quotidien_20261020-053001.log
2026-10-20T05:30:01+0200 ./logs/quotidien.verrou
2026-10-20T05:38:07+0200 ./logs/refresh_20261020-053001.log
2026-10-20T05:47:13+0200 ./logs/sync_vecteurs_20261020-053808.log
2026-10-20T05:57:41+0200 ./rapports/DERNIER.md
2026-10-20T05:57:41+0200 ./rapports/quotidien-20261020-053001.md
```

**Explication de chaque fichier de (g).** Les huit sont écrits par la production, aucun par la
mesure. Sept portent l'heure de la tâche quotidienne, dans `$PROD_DAILY_WINDOW` : ils sont tous
antérieurs à 06:00, donc antérieurs au premier geste de la campagne (05:34:55Z = 07:34:55 CEST).

| Fichier | Date | Ce qui l'écrit |
|---|---|---|
| `logs/quotidien.verrou` | 05:30:01 | verrou de la tâche quotidienne, posé à son démarrage |
| `logs/refresh_20261020-053001.log` | 05:38:07 | journal de la tâche quotidienne |
| `logs/sync_vecteurs_20261020-053808.log` | 05:47:13 | journal de la synchronisation des vecteurs |
| `logs/exporter_vecteurs_20261020-054715.log` | 05:53:59 | journal de l'export des vecteurs |
| `logs/quotidien_20261020-053001.log` | 05:57:41 | journal de la tâche quotidienne |
| `rapports/quotidien-20261020-053001.md` | 05:57:41 | rapport de la tâche quotidienne |
| `rapports/DERNIER.md` | 05:57:41 | copie du rapport du jour, écrite par la même tâche |
| `logs/mcp_20261006-201803.log` | 10:22:53 | journal en ajout du service MCP de production, ouvert le 2026-10-06 |

Le seul fichier de la fenêtre de campagne, en (f), est donc `logs/mcp_20261006-201803.log`, daté
**10:22:53, deux minutes avant le relevé**, et il continue de grossir après la campagne : 34 lignes
entre 10:00 et 10:22, contre 5 entre 09:00 et 09:59. C'est un journal en ajout d'un service de
production en fonctionnement, pas un fichier de mesure. La mesure ne passe pas par ce service : la
qualité est calculée en SQL direct (`eval_recherche.py`), la fidélité par `psql`, la contre-mesure
par `api.*` en processus. Le contenu de ces lignes n'est pas recopié (il peut porter du contenu de
requête, cf. règle de confidentialité) ; seules leur date et leur niveau (`INFO`) ont été lus. Je
ne les attribue pas à une activité précise : je n'ai pas le compteur d'écritures du service.

**Ce que la preuve établit.** Aucun fichier suivi n'est modifié (c). Sur l'ensemble du dépôt,
`.git` excepté, **un seul** fichier a une date postérieure au début de la campagne (f), et c'est le
journal du service de production (g). L'inventaire par fichier de (e) porte sur les 41 736 fichiers
ignorés : la preuve ne dépend donc plus du repliement des répertoires, puisqu'un fichier ajouté
dans un dossier ignoré apparaîtrait dans (f).

**Ce qu'elle n'établit pas.** La date de modification est le seul témoin ; un fichier écrit puis
remis à une date antérieure échapperait au relevé. Aucune commande de la campagne ne rétablit une
date de modification.

Les artefacts de la campagne restent hors du dépôt, sous `/tmp/mes00/`, et sont donc absents de cet
inventaire ; leurs copies sont décrites au §9.

Autres contrôles, tous datés dans `/tmp/mes00/out/` :

- aucun fichier créé hors de `/tmp` : `find $HOME -maxdepth 3 -newermt …` ne rend que
  `~/.docker/buildx` (état du CLI Docker), le journal de l'agent VS Code distant, et le mtime du
  dossier `.git` (effet d'un `git status`, aucun fichier nouveau dedans) ;
- `mcp.appels_outil` inchangé : `audit_appels_outil` du harnais donne `avant_n` 1 052 =
  `apres_n` 1 052, `avant_id` 1 674 = `apres_id` 1 674, écart 0 — aucune métrique écrite ;
- aucun service redémarré : 6 conteneurs `Up 13 days`, `RestartCount` 0, `StartedAt`
  2026-10-06T18:24:29Z identique avant et après ; `$PROD_SERVICES` n'a reçu aucune commande ;
- journaux des scripts sous `/tmp/mes00/logs/` (`LEGIFACILE_LOG_DIR`), dont
  `eval_recherche_20261020-094146.log` et `froid_decomposition_20261020-092031.log` ;
- aucun bytecode écrit dans `$PROD_REPO` (`PYTHONDONTWRITEBYTECODE=1`, et
  `sys.dont_write_bytecode = True` dans les scripts de Crible).

### 6.3 Ce que la passe de référence n'a pas refait

La passe du 20 n'a **pas** rejoué la campagne `--perimeter-first` sur 491 requêtes : la fusion du
20 est mesurée après le mono-corpus et reste biaisée à la baisse. Elle n'a pas non plus refait
tourner la recette ni touché à la base. La passe `--perimeter-first` de la passe remplacée n'est
donc pas remplacée par une mesure équivalente du 20 ; elle n'est pas reprise comme référence.

## 7. Protocole reproductible

Seuls les **noms** des variables de `.env.prod` apparaissent ici. Chaque bloc est annoté
`[local]` (poste de développement) ou `[distant]` (Mac mini).

```bash
# [local] seul $PROD_SSH est lu depuis .env.prod pour ouvrir la session
set -a; . ./.env.prod; set +a
ssh "$PROD_SSH"

# [distant] le shell non interactif n'hérite pas du PATH de macOS : docker est sous
# /usr/local/bin. Sans cet export, fidelite_hnsw.py échoue sur « docker: No such file ».
export PATH=/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin
# source non versionnée, hors du dépôt de production (fichier du propriétaire, mode 600)
set -a; . "$HOME/.config/crible/prod.env"; set +a
test -n "$PROD_REPO" && test -n "$PROD_PGOPTIONS" && test -n "$PROD_DB_CONTAINER"
mkdir -p /tmp/mes00/out /tmp/mes00/logs
chmod 700 /tmp/mes00 /tmp/mes00/out /tmp/mes00/logs
export LEGIFACILE_LOG_DIR=/tmp/mes00/logs
export PYTHONDONTWRITEBYTECODE=1
# $EXPORT_TITRES_SQL : requête d'export du harnais, une seule instruction,
# colonnes id, corpus, text, aucune donnée personnelle (voir le README).
EXPORT_TITRES_SQL="$(cat <<'SQL'
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
SQL
)"
# Entrées de répertoire ignorées, avant : ce `status` affiche un dossier, pas son contenu.
date '+%Y-%m-%d %H:%M:%S' > /tmp/mes00/out/campaign-start.txt
git --no-optional-locks -C "$PROD_REPO" status --porcelain \
  > /tmp/mes00/out/porcelain-initial.txt
git --no-optional-locks -C "$PROD_REPO" status --porcelain --ignored=matching -uall \
  > /tmp/mes00/out/ignored-initial.txt
# Inventaire fichier par fichier des ignorés, avant.
git --no-optional-locks -C "$PROD_REPO" ls-files --others --ignored --exclude-standard \
  | sort > /tmp/mes00/out/ignored-files-initial.txt

# [local] copie des scripts ; le dossier reste plat
scp tools/bench/mes00/*.py tools/bench/mes00/README.md "$PROD_SSH:/tmp/mes00/"

# [distant] export des titres (lecture seule, une instruction)
docker exec -e PGOPTIONS="$PROD_PGOPTIONS" "$PROD_DB_CONTAINER" \
  psql -X -q -A -t -U postgres -v ON_ERROR_STOP=1 \
  -c "COPY ($EXPORT_TITRES_SQL) TO STDOUT WITH (FORMAT csv, HEADER true)" \
  > /tmp/mes00/out/titres.csv

# [distant] 1 000 requêtes (graine fixe), puis latence chaude
"$PROD_REPO/.venv/bin/python" /tmp/mes00/gen_requetes.py \
  --from-csv /tmp/mes00/out/titres.csv --count 1000 --seed 20261019 \
  --output /tmp/mes00/out/queries.json

"$PROD_REPO/.venv/bin/python" /tmp/mes00/latence_directe.py \
  --queries /tmp/mes00/out/queries.json --project "$PROD_REPO" \
  --k 10 --idle-minutes-per-corpus 0 \
  --output /tmp/mes00/out/latency.json

# [distant] latence froide : 10 min d'inactivité avant chaque corpus (8 mesures froides)
nohup "$PROD_REPO/.venv/bin/python" /tmp/mes00/latence_directe.py \
  --queries /tmp/mes00/out/queries.json --project "$PROD_REPO" \
  --k 10 --idle-minutes-per-corpus 10 \
  --output /tmp/mes00/out/latency-cold.json \
  > /tmp/mes00/out/latency-cold.stdout 2>&1 &
echo $! > /tmp/mes00/out/latency-cold.pid

# [distant] attendre la fin de la passe froide avant toute autre mesure : les
# requêtes restent en série. `wait` ne voit que les enfants du shell courant ;
# on sonde donc le processus.
while kill -0 "$(cat /tmp/mes00/out/latency-cold.pid)" 2>/dev/null; do sleep 60; done
test -s /tmp/mes00/out/latency-cold.json

# [distant] qualité sur les 57 questions (sortie hors du dépôt, obligatoire)
"$PROD_REPO/.venv/bin/python" "$PROD_REPO/scripts/db/eval_recherche.py" \
  --sortie /tmp/mes00/out/eval-recherche.md --k 10 --dedupliquer document

# [distant] gabarits JSON argv : ef=40, ef=200, exact (écrits sous /tmp/mes00/out)
#   ef=40   : BEGIN READ ONLY; SELECT … ORDER BY embedding <=> … LIMIT 10; COMMIT;
#   ef=200  : idem avec SET LOCAL hnsw.ef_search = 200;
#   exact   : idem avec SET LOCAL enable_indexscan = off; SET LOCAL enable_bitmapscan = off;
# {corpus} et {vector_literal} sont substitués par le harnais, {container} par
# $PROD_DB_CONTAINER. Les trois commandes `cat > …` qui créent ces fichiers sont
# dans la section « Gabarits de fidélité pour le mode sql » du README : elles
# doivent être exécutées avant les lignes `cat` ci-dessous. Le §7 ne les
# duplique pas.

# [distant] 24 vecteurs (3 par corpus) puis fidélité
"$PROD_REPO/.venv/bin/python" /tmp/mes00/vecteurs_requetes.py \
  --queries /tmp/mes00/out/queries.json --per-corpus 3 --project "$PROD_REPO" \
  --output /tmp/mes00/out/query-vectors.json

# Lecture des trois gabarits créés par le README (voir ci-dessus).
HNSW_COMMAND="$(cat /tmp/mes00/out/hnsw-ef40.json)"
HNSW_COMMAND_EF200="$(cat /tmp/mes00/out/hnsw-ef200.json)"
EXACT_COMMAND="$(cat /tmp/mes00/out/exact.json)"

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

# [distant] observation q0018 à ef_search = 1000 (top 10 HNSW et top 10 exact, avec distances)
#   sortie : /tmp/mes00/out/fidelity-q0018-ef1000.txt
#   pas de gabarit JSON ici : c'est un script d'une seule pièce, reproduit au §7.1

# [distant] contre-mesure indépendante
"$PROD_REPO/.venv/bin/python" "$PROD_REPO/scripts/db/froid_decomposition.py" \
  --series 1 --repetitions 2 --attente-min 10 \
  --perimetres jurisprudence,droit_applicable --json /tmp/mes00/out/froid.json

# [distant] entrées de répertoire après, puis contrôles fichier par fichier
git --no-optional-locks -C "$PROD_REPO" status --porcelain \
  > /tmp/mes00/out/porcelain-after.txt
test ! -s /tmp/mes00/out/porcelain-after.txt
git --no-optional-locks -C "$PROD_REPO" status --porcelain --ignored=matching -uall \
  > /tmp/mes00/out/ignored-after.txt
diff -u /tmp/mes00/out/ignored-initial.txt /tmp/mes00/out/ignored-after.txt
# Inventaire fichier par fichier des ignorés, après, puis comparaison.
git --no-optional-locks -C "$PROD_REPO" ls-files --others --ignored --exclude-standard \
  | sort > /tmp/mes00/out/ignored-files-after.txt
diff -u /tmp/mes00/out/ignored-files-initial.txt /tmp/mes00/out/ignored-files-after.txt
# Aucun fichier modifié depuis le début de la campagne, sauf la production.
cd "$PROD_REPO" && find . -path ./.git -prune -o -type f \
  -newermt "$(cat /tmp/mes00/out/campaign-start.txt)" -print | sort
```

La requête d'export des titres est celle du harnais (une instruction `WITH … SELECT`, colonnes
`id`, `corpus`, `text`, aucune donnée personnelle). Elle est définie dans le bloc ci-dessus
(`$EXPORT_TITRES_SQL`) et reprise à l'identique du README du harnais, pour que le bloc soit
copiable tel quel.

### 7.1 Commande réelle de l'observation `q0018` à `ef_search = 1000`

Cette observation n'est pas passée par `fidelite_hnsw.py` : elle n'a donc **pas** de gabarit JSON,
et c'est pourquoi le §7 n'en annonce que trois. Elle a été produite par un script d'une seule
pièce, `q0018.sh`, exécuté le 2026-10-20 à 09:19 (heure locale) et conservé sous `/tmp/mes00/`.
Voici ce script tel qu'il a été exécuté, à deux substitutions près : la valeur de `PGOPTIONS` est
écrite `$PROD_PGOPTIONS` (la variable dont elle vient), et le chemin absolu du dépôt est écrit
`$PROD_REPO`.

```bash
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
echo "# machine: $(hostname -s)"
echo '# HNSW, ef_search = 1000, top 10 : rang, id, distance'
docker exec -e PGOPTIONS="$PROD_PGOPTIONS" "$PROD_DB_CONTAINER" psql -X -q -A -t -U postgres -v ON_ERROR_STOP=1   -c "BEGIN READ ONLY; SET LOCAL hnsw.ef_search = 1000; SELECT row_number() OVER (ORDER BY embedding <=> '$V'::halfvec(768)) || ' ' || src_id || '#' || passage_ord || ' ' || round((embedding <=> '$V'::halfvec(768))::numeric, 6) FROM vec.passages WHERE corpus = 'juri_texte_integral' ORDER BY embedding <=> '$V'::halfvec(768) LIMIT 10; COMMIT;"
echo '# exact (enable_indexscan off, enable_bitmapscan off), top 10 : rang, id, distance'
docker exec -e PGOPTIONS="$PROD_PGOPTIONS" "$PROD_DB_CONTAINER" psql -X -q -A -t -U postgres -v ON_ERROR_STOP=1   -c "BEGIN READ ONLY; SET LOCAL enable_indexscan = off; SET LOCAL enable_bitmapscan = off; SELECT row_number() OVER (ORDER BY embedding <=> '$V'::halfvec(768)) || ' ' || src_id || '#' || passage_ord || ' ' || round((embedding <=> '$V'::halfvec(768))::numeric, 6) FROM vec.passages WHERE corpus = 'juri_texte_integral' ORDER BY embedding <=> '$V'::halfvec(768) LIMIT 10; COMMIT;"
} > $OUT 2>&1
wc -l $OUT
```

Ce que fait le script :

- `$V` est la **valeur littérale** du vecteur de `q0018` (768 composantes), lue dans
  `/tmp/mes00/out/query-vectors.json`, le fichier que la fidélité utilise ; elle n'est pas recopiée
  en dur dans le rapport ;
- chaque requête tient dans un seul `-c` : `BEGIN READ ONLY;` … `COMMIT;`, avec `-v
  ON_ERROR_STOP=1` et `-A -t` (sortie sans en-tête ni alignement) ;
- côté HNSW : `SET LOCAL hnsw.ef_search = 1000;` puis `ORDER BY embedding <=> … LIMIT 10` ;
- côté exact : `SET LOCAL enable_indexscan = off; SET LOCAL enable_bitmapscan = off;` (plan
  `Seq Scan` + `Sort`), même `ORDER BY … LIMIT 10` ;
- chaque ligne rendue est `rang id distance`, la distance étant `round(…::numeric, 6)` et l'id
  `src_id#passage_ord` ;
- l'en-tête du fichier de sortie prend le nom de la machine par `hostname -s` : c'est l'origine de
  la ligne `# machine:` du fichier brut, désormais corrigée en « Mac mini de production ».

Les deux top 10 produits par ce script sont le fichier
`MES-00-reference-pgvector.2026-10-20.q0018-ef1000.txt` (§3, §9). Un gabarit JSON équivalent au
sens du README du harnais (`["docker","exec",…,"-c","BEGIN READ ONLY; SET LOCAL hnsw.ef_search =
1000; …"]`) se déduirait de ces deux `-c`, mais il n'est pas livré : le README documente la
procédure réelle (le script ci-dessus), pas un gabarit JSON.

## Conclusion

Ce que la mesure permet d'affirmer :

1. **La recherche actuelle est lente sur les gros corpus.** À chaud, p50 279 ms et p95 698 ms
   pour un appel mono-corpus `k = 10`, p99 1 112 ms. Le SQL pèse 56,0 % du total, la reconstruction
   32,8 %, l'encodeur 11,1 %.
2. **Le froid coûte cher mais reste borné** : p50 483 ms, p95 1 163 ms avec 10 minutes
   d'inactivité. C'est une borne **basse** du froid réel : les trous d'usage de la production vont
   de 59 minutes à 35 heures (médiane 20 h, cf. le rapport de `froid_decomposition`).
3. **La qualité est faible et très inégale** : rappel@10 moyen 0,482, dominé par
   `juri_texte_integral` (0,290 sur 31 questions) ; 29 questions sur 56 ne trouvent aucun attendu
   dans le top 10.
4. **HNSW perd parfois tout** : 0,925 de recall@10 moyen à `ef=200`, avec au moins une requête à
   0,00. L'observation `q0018` à `ef=1000` est conservée en sortie brute et montre que ce n'est pas
   un réglage d'`ef_search` ; la cause reste à établir dans MES-01.
5. **La fusion du 20 est une mesure biaisée à la baisse** : p95 622 ms, mais le second appel
   profite des pages chargées par le premier. La passe `--perimeter-first` n'a pas été rejouée.

Ce que la mesure ne permet pas d'affirmer :

- la latence **sous charge** (une seule requête en vol à la fois) ni à travers le tunnel ;
- le froid au-delà de 10 minutes d'inactivité ;
- la distribution du rappel HNSW par corpus (3 requêtes par corpus) ;
- la qualité pour un usage professionnel réel : les 57 questions sont écrites en interne, pas par
  une juriste (limite déjà consignée dans le rapport d'`eval_recherche.py`) ;
- la latence de requêtes aussi longues qu'une question d'utilisateur : les requêtes mesurées sont
  des titres (≤ 300 caractères), l'encodage y est donc plutôt sous-estimé.

Les propositions initiales sont détaillées à la section 8. Les cibles retenues sont fixées dans
`docs/suivi.md` (section « Cibles »), le 2026-10-20, par l'orchestrateur sur délégation du
propriétaire ; elles remplacent les propositions de la section 8.

## 8. Propositions initiales (remplacées par les cibles de `docs/suivi.md`)

Les cinq cibles ci-dessous sont les **propositions initiales** de cette passe. Elles sont
**remplacées** par les cibles fixées dans `docs/suivi.md`, section « Cibles », qui est la seule
source des valeurs retenues ; aucune n'est reprise ici comme cible applicable.

- **Rappel** : rappel@10 moyen ≥ **0,48** sur les 56 questions satisfiable, et aucune régression
  par corpus par rapport aux valeurs du tableau 2. La comparaison se fait question par question,
  jamais sur la seule moyenne.
- **Latence chaude mono-corpus** `k = 10` : p95 ≤ **720 ms** et p99 ≤ **1 150 ms**, avec un
  garde-fou : aucun dépassement de 8 000 ms.
- **Latence froide mono-corpus** : p95 ≤ **1 250 ms**.
- **Fusion deux corpus** : p95 ≤ **900 ms**. La cible porte sur une fusion à deux corpus
  (`jurisprudence`, `droit_applicable`) et ne s'appuie pas sur la fusion biaisée du 20.
- **Fidélité binaire contre exact** : recall@10 ≥ **0,99** (provisoire, `docs/08`, à confirmer
  après MES-01).

## 9. Fichiers bruts du 2026-10-20

Tous les fichiers listés portent le préfixe `MES-00-reference-pgvector.2026-10-20.` et sont
conservés dans `docs/mesures/`. Les brouillons `MES-00-reference-pgvector.2026-10-19.*` ont été
supprimés du répertoire de travail ; ils ne doivent pas être commités.

| Fichier | Contenu | Taille | SHA-256 |
|---|---|---|---|
| `…2026-10-20.latency.json` | 1 000 mesures chaudes, vues mono-corpus et fusion | 492 168 o | `fedf5d0502737f5d08463b85f89989a1377c78c488960fd2913761e5e0d06610` |
| `…2026-10-20.latency-cold.json` | 1 000 mesures, 8 froides + 992 chaudes | 525 329 o | `4da09d4a68e252c38d280c2208c1f57fe2556ff916123beca0a5c5d308fb259d` |
| `…2026-10-20.fidelity-ef40.json` | recall@10, `ef_search` par défaut | 21 538 o | `df8be7f90d68ffc33a8814c3e56bc7c0a90fc2cef82610c3cce230f6c62f7848` |
| `…2026-10-20.fidelity-ef200.json` | recall@10, `ef_search = 200` | 21 527 o | `da7dc6ffd478b7c8099db1bcb82d465c7540060c5d6853db8136b3774b4feaac` |
| `…2026-10-20.eval-recherche.md` | rapport des 57 questions | 17 837 o | `bfe688064ec9a0b4b1d578da5f54b87d4af501c84b04f619cee38e866096a1ca` |
| `…2026-10-20.froid.json` | sortie de `froid_decomposition.py` | 16 882 o | `130689b66d4f1647b70299aba1a21c725dd9aec1088b679da3f63c611be436f8` |
| `…2026-10-20.q0018-ef1000.txt` | sortie brute : top 10 HNSW à ef=1000 et top 10 exact, avec distances, précédée de l'argv qui l'a produite | 3 130 o | `6adf1f97f488670ff86bb5d80cc078d713e32bccbc75fc6b272997b4b86b4c1b` |

Deux transformations ont été appliquées, identiques à celles de la passe remplacée :

1. dans `…2026-10-20.froid.json` et `…2026-10-20.eval-recherche.md`, le chemin absolu du dépôt de
   production a été remplacé par `$PROD_REPO` (ADR-0032) ; aucun chiffre ni aucune question n'a
   changé, et les empreintes ci-dessus sont celles **après** substitution ;
2. `froid.json` conserve les clés françaises de `froid_decomposition.py` : c'est une sortie
   externe, non renommée. `eval-recherche.md` reste en Markdown, faute d'export JSON côté
   Legifacile.

Une troisième transformation ne porte que sur `…2026-10-20.q0018-ef1000.txt` : l'argv du script qui
a produit la sortie lui est ajouté en tête, et sa ligne d'en-tête `# machine:` — qui rendait le nom
de la machine de production par `hostname -s` — devient `# machine: Mac mini de production`. Le
corps de la sortie est identique octet pour octet à celle du Mac mini de production ; seules ces
lignes ont changé, et l'empreinte ci-dessus est celle **après** ces retouches.

Les deux fichiers de latence (`…2026-10-20.latency.json` et
`…2026-10-20.latency-cold.json`) datent d'avant la correction des compteurs d'agrégats : leurs
agrégats ne portent ni la clé `not_run` (étape non lancée, par exemple une fusion vide) ni la clé
`failed` (échec d'encodage), décrites dans le README du harnais. Ces clés n'ont pas été ajoutées
après coup et les fichiers ne sont pas réécrits. Aucun chiffre publié ne change : les 2 × 1 000
mesures ne contiennent aucune durée nulle, donc aucune censure, aucune fusion vide et aucun échec.

Aucun texte de question d'utilisateur, aucune valeur de `.env.prod`, aucun chemin absolu de
production et aucun nom d'image Docker n'apparaît dans ces fichiers ni dans ce rapport.

## Annexe — Passe du 2026-10-19 (non conforme, remplacée)

La passe du 2026-10-19 a mesuré 1 000 requêtes chaudes, une vue fusion sur 491 requêtes en
`--perimeter-first`, 8 mesures froides, 24 requêtes de fidélité, 57 questions de qualité et une
contre-mesure `froid_decomposition.py`. Elle n'est **pas** la référence et aucun de ses tableaux
n'est repris ici.

Pendant cette passe, trois fichiers ignorés par git ont été créés dans `$PROD_REPO` : les deux
journaux `logs/eval_recherche_20261019-204115.log` et
`logs/froid_decomposition_20261019-223234.log`, et le bytecode
`scripts/db/__pycache__/froid_decomposition.cpython-312.pyc`. Ils ont été copiés sous
`/tmp/mes00/sauvegarde-20261019/`, puis supprimés ; aucun service n'a été redémarré. **Leur
absence n'est pas prouvée fichier par fichier** : l'inventaire du 20, pris avant la première
mesure, est un `git status --ignored` replié, qui ne distingue pas un fichier à l'intérieur d'un
dossier ignoré (cf. §6.1) ; leur retrait repose sur l'action du propriétaire, pas sur une
vérification de ce rapport.

Les chiffres utiles de qualité, de fidélité, de latence mono-corpus et de contre-mesure ont été
rejoués le 20. La campagne `--perimeter-first` de 491 requêtes n'a pas été rejouée ; la fusion du
20 est mesurée après le mono-corpus et reste biaisée à la baisse. Les fichiers bruts du 19 ont été
supprimés du répertoire de travail et ne doivent pas être commités.
