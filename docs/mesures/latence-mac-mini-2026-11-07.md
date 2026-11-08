# Latence sur le Mac mini — passe B de `MESURER-SUR-MAC-MINI`

Passe B de la tâche `MESURER-SUR-MAC-MINI` (P10) : la **série contraignante** de J10 (dense en
texte, 97 requêtes) et les latences de lecture, en conteneur, sur l'instance restaurée du Mac mini,
selon ADR-0141 (décisions 1, 3 et 4). Ce document n'est pas une MES-xx : la série contraignante n'a
pas de numéro de mesure, la référence est MES-00 du 2026-10-20
(`docs/08-invariants-et-tests.md:85-88`). Le critère est fixé par ADR-0141, décision 4 ; la passe C
en donne une estimation prudente et un recoupement par l'API, rappelés en conclusion.
Les rapports bruts sont les 30 couples `eval-*-macmini-2026-11-07.{json,md}` de ce dossier ; les
agrégats sont dans `eval-macmini-2026-11-07.aggregates.json` ; les données brutes sous
`data/p10-macmini/` (gitignoré).

## Campagne du 2026-11-07

- **Tâche** : `MESURER-SUR-MAC-MINI`, passe B seule (latence contraignante et lectures).
- **Commit** : `439d946bf28d63214e64aa7cb2e09e458f81a45d`, plus le harnais
  `tools/bench/p10-macmini/` de la passe A et de cette séance, pas encore commité.
- **Machine** : Mac mini M1 (`Macmini9,1`), 8 cœurs, 17 179 869 184 o (16 Gio), macOS 26.5.2,
  Docker Desktop 4.83.0, moteur 29.6.2. **VM mesurée : 8 vCPU, 10 418 937 856 o, soit 9,70 GiB**
  (la fiche et ADR-0141 annoncent 11 Gio ; c'est la valeur mesurée qui fait foi). Le disque de la
  VM est sur le SSD externe `MattCorp-SSD` (APFS, USB4).
- **Jeu de données** : `crible-p10-data`, restauré en passe A depuis les six instantanés
  (25 480 066 488 o, 282 fichiers, blake3 source conforme) : **6 660 239 documents**,
  **9 600 105 passages**, zéro passage mort. Générations `articles` 1, `decisions` 2, `dole` 1,
  `jorf` 3, `sarde` 1, `titres` 1 ; époque 1 pour les six (volume de destination vide, l'époque ne
  se compare pas).
- **Configuration** : celle servie par l'index — `exact_threshold` 5000, N = 5000
  (`rescore_candidates`), `k` = 10, précision de référence fp16, fusion `rrf` en hybride, un seul
  fil de scan. Aucun réglage n'est surchargé, sauf `--exact-threshold 5000`, qui **rétablit** la
  valeur de l'index : sans lui, `crible eval` force la stratégie binaire (ADR-0125, décision 7).
- **Fenêtre** : hors de `$PROD_DAILY_WINDOW`, de 19:03 à 20:15 UTC environ.
- **État de Supabase** : les six conteneurs `supabase_*_legifacile` sont **arrêtés** au début de
  la passe et le restent ; `docker ps --filter name=supabase_` est vide **avant chaque série**
  (contrôle du harnais). Aucun geste n'a été fait sur Supabase, le dépôt Legifacile ou Docker
  Desktop.

### Conditions et gestes

- **Le seul conteneur `crible-p10` est arrêté** pour toute la passe (ADR-0141, décision 1) : Crible
  est seul dans la VM, et les conteneurs de mesure ne doublent pas la mémoire du serveur.
- **Aucun plafond `--memory`** n'est posé (ADR-0135, décision 5 ; ADR-0139, décision 3).
- **Tous les conteneurs de mesure sont éphémères** (`docker run --rm`, au premier plan). Aucune
  commande n'est lancée en arrière-plan.
- **Le volume `crible-p10-data` est monté en lecture-écriture**, parce que SQLite refuse d'ouvrir
  ses fichiers WAL sur un montage `:ro`. `crible eval` ouvre chaque index en lecture seule
  (`crates/crible/src/eval.rs:454`, ADR-0116) et les comptes n'ont pas bougé. La passe C, elle,
  ouvre MES-02 et MES-03 en écriture avec reprise, et MES-05 n'ouvre aucun index.

## Protocole

Les commandes exactes sont celles du harnais `tools/bench/p10-macmini/` ; les valeurs de
`.env.prod` ne sont jamais affichées, la connexion passe par `$PROD_SSH`.

```sh
sh tools/bench/p10-macmini/run_pass_b.sh host before-pass
sh tools/bench/p10-macmini/run_pass_b.sh stop
python3 tools/bench/p10-macmini/build_pass_b_inputs.py
sh tools/bench/p10-macmini/run_pass_b.sh push
sh tools/bench/p10-macmini/run_pass_b.sh series <index> <passe> [cold]
sh tools/bench/p10-macmini/run_pass_b.sh top10 remote <index>
sh tools/bench/p10-macmini/run_pass_b.sh top10 local <index>
sh tools/bench/p10-macmini/run_pass_b.sh pull
python3 tools/bench/p10-macmini/aggregate_pass_b.py
sh tools/bench/p10-macmini/run_pass_b.sh start
```

### Entrées

- **Les 97 requêtes du jeu de référence en texte** (`docs/legifacile/reference/<index>.jsonl`,
  `Q04` actualisé par `ACTUALISER-JEU-DE-REFERENCE` le 2026-11-06) : 25 `articles`, 45 `decisions`,
  6 `dole`, 10 `jorf`, 8 `sarde`, 3 `titres`. `build_pass_b_inputs.py` les relit et écrit un jeu
  par index et par passe ; les textes et les filtres restent sous `data/` (ADR-0125, décision 16).
- **Les 97 vecteurs de requête de pgvector** (`data/p10-compare/query-vectors.json`, passe A de
  `COMPARER-AVEC-PGVECTOR`) : même jeu, `vector` à la place de `query`.
- **Les 1 000 titres de MES-00** (`data/mes00/queries.json`) : 246 `articles`, 251 `decisions`,
  111 `dole`, 125 `jorf`, 129 `sarde`, 138 `titres`, avec les filtres de périmètre de
  `docs/legifacile/index-legifacile.md` (les deux corpus d'articles portent le filtre d'état de
  `droit_en_vigueur` et de `droit_historique` ; les autres n'en portent aucun, comme le jeu de
  référence).
- Les trois familles sont copiées sur le SSD, sous le dossier d'échange, **hors du dépôt
  Legifacile et hors du dossier de données de Crible**.

### Séries

Une passe d'échauffement **non comptée**, puis `--repeat 3`, par `crible eval` local dans un
conteneur neuf, un index par commande :

1. **contraignante** — dense, les 97 requêtes **en texte, encodées par Crible** ;
2. dense à **vecteur fourni** (vecteurs de pgvector) ;
3. hybride `rrf`, les 97 en texte ;
4. **froid partiel** du dense en texte, nommé comme tel ;
5. **lecture complémentaire** — les 1 000 titres, dense en texte.

Le froid partiel applique le mécanisme de contre-contrôle d'ADR-0135, décision 1 : purge globale
de la VM par `sync; echo 3 > /proc/sys/vm/drop_caches` dans un conteneur privilégié, immédiatement
avant la série. La résidence n'est **pas** mesurée : l'image ne porte ni `mincore` ni `fincore`, et
l'hôte n'a aucun outil de ce type. Le chiffre s'appelle donc **froid partiel, résidence non
mesurée**, jamais « à froid ».

### Relevés de l'hôte

Le swap et la pression mémoire de l'hôte sont relevés avant la passe et après chaque série
(`data/p10-macmini/host/`). Le conteneur de mesure ne peut pas être lu par `docker stats` en cours
de route : la mesure est en avant-plan, et le harnais n'a pas le droit de lancer une commande en
arrière-plan pour l'échantillonner. La mémoire de la VM est donc déclarée par `docker info`
(10 418 937 856 o) et non par conteneur.

| Relevé | Swap utilisé | Mémoire libre de l'hôte |
|---|---:|---:|
| Avant la passe | 2 357,38 Mo / 3 072 Mo | 52 % |
| Après la dernière série en texte | 2 301,38 Mo | 66 % |
| Après la dernière série à vecteur | 2 277,38 Mo | 56 % |
| Après la dernière série hybride | 2 269,38 Mo | 53 % |
| Après la dernière série froide | 2 245,38 Mo | 68 % |
| Après la dernière série des 1 000 titres | 2 245,38 Mo | 51 % |

Le swap **décroît** de 2 357 à 2 245 Mo pendant la passe : l'arrêt du serveur libère 3,67 GiB et
la VM n'a plus à paginer. Aucun relevé n'approche le seuil d'arrêt de 4 Go. La passe A avait laissé
le swap à 2 461 Mo ; il était encore à 2 357 Mo au départ.

## Résultats

### Série contraignante — dense en texte (97 requêtes, 291 échantillons)

Percentiles du `total` de `debug.timings_ms`, au rang le plus proche, par index :

| Index | n | p50 (ms) | p95 (ms) | p99 (ms) | `dense` p95 | `encode` p95 | `filter` p95 |
|---|---:|---:|---:|---:|---:|---:|---:|
| `articles` | 75 | 21,862 | **79,355** | 82,060 | 27,099 | 0,008 | 58,610 |
| `decisions` | 135 | 44,597 | 52,553 | 53,685 | 51,851 | 0,024 | 0,345 |
| `dole` | 18 | 4,493 | 13,099 | 13,099 | 12,682 | 0,010 | 0,008 |
| `jorf` | 30 | 26,925 | 33,316 | 33,358 | 32,531 | 0,015 | 20,774 |
| `sarde` | 24 | 8,723 | 10,418 | 10,443 | 10,056 | 0,005 | 0,022 |
| `titres` | 9 | 20,325 | 29,848 | 29,848 | 29,288 | 0,005 | 0,115 |
| **Plus grand p95 par index (majorant brut)** | **291** | non calculable | **79,355** | non calculable | — | — | — |

**Le groupé n'est pas calculable à partir des rapports.** `crible eval` agrège les percentiles en
processus (ADR-0125, décision 9) et le rapport ne porte **aucun échantillon par requête** : ses
lignes ne contiennent que l'`id` et les figures de qualité et de fidélité. Les 291 échantillons ne
peuvent donc pas être reconstitués. **79,355 ms sur `articles`** est le **plus grand p95 par index
(majorant brut)** : c'est la valeur que l'orchestrateur a prévue pour ce cas. Obtenir
le groupé exact demanderait soit de modifier le code de production, interdit dans cette session,
soit 291 invocations `crible eval` à une requête — non faites. Le p50 groupé est encadré par les
p50 par index (4,493 à 44,597 ms) et le p99 par les p99 par index (10,443 à 82,060 ms) ; ces bornes
ne sont pas des percentiles groupés.

**Part de l'encodage.** Au p95, `encode` vaut 0,005 à 0,024 ms, soit **moins de 0,03 %** du total.
Ce n'est pas une erreur : le cache de requêtes (ADR-0051) est déjà chaud, la passe d'échauffement
et les trois répétitions réutilisent le même vecteur. MES-00 comptait, elle, **11,1 %** d'encodeur
(50 ms au p95) parce que chaque titre y était encodé une seule fois ; la comparaison à 698 ms est
donc favorable par construction sur ce poste, et il faut le lire ainsi.

**Fidélité.** Le rappel@10 au passage vaut 1,0000 sur cinq index et **0,9911 sur `decisions`**
(n = 45, dénominateur 10 partout, aucune requête exclue) ; **0,9959 sur les 97 requêtes**. La
cible de 0,99 est donc atteinte à l'échelle réelle, comme l'exige J10.

### Dense à vecteur fourni (97 requêtes, 291 échantillons)

| Index | n | p50 (ms) | p95 (ms) | p99 (ms) |
|---|---:|---:|---:|---:|
| `articles` | 75 | 20,975 | 78,216 | 86,724 |
| `decisions` | 135 | 40,548 | 53,605 | 56,350 |
| `dole` | 18 | 4,622 | 15,481 | 15,481 |
| `jorf` | 30 | 26,276 | 33,267 | 36,301 |
| `sarde` | 24 | 9,058 | 10,470 | 10,583 |
| `titres` | 9 | 19,532 | 30,461 | 30,461 |
| **Plus grand p95 par index (majorant brut)** | **291** | — | **≤ 78,216** | — |

Les 97 requêtes ont le **même top-10** que la machine de développement, et les mêmes figures de
qualité par requête (97/97, contrôle détaillé plus bas).

### Hybride `rrf` (97 requêtes, 291 échantillons)

| Index | n | p50 (ms) | p95 (ms) | p99 (ms) | `lexical` p95 | `dense` p95 |
|---|---:|---:|---:|---:|---:|---:|
| `articles` | 75 | 57,960 | 105,144 | 128,620 | 100,432 | 28,897 |
| `decisions` | 135 | 106,088 | **147,300** | 156,517 | 146,517 | 65,595 |
| `dole` | 18 | 4,769 | 12,309 | 12,309 | 5,058 | 11,853 |
| `jorf` | 30 | 35,396 | 80,051 | 81,172 | 75,751 | 71,756 |
| `sarde` | 24 | 10,061 | 10,641 | 11,008 | 6,720 | 10,295 |
| `titres` | 9 | 28,110 | 41,518 | 41,518 | 40,986 | 35,553 |
| **Plus grand p95 par index (majorant brut)** | **291** | — | **≤ 147,300** | — | — | — |

La branche lexicale porte l'hybride : son p95 va de 5,1 à 146,5 ms quand le dense reste sous
71,8 ms. Aucune cible ne porte sur l'hybride ; la série est rapportée sans être contraignante.

### Froid partiel du dense en texte (97 requêtes, 291 échantillons)

| Index | n | p50 (ms) | p95 (ms) | p99 (ms) |
|---|---:|---:|---:|---:|
| `articles` | 75 | 21,941 | 78,404 | 81,816 |
| `decisions` | 135 | 43,365 | 53,084 | 56,096 |
| `dole` | 18 | 4,467 | 11,312 | 11,312 |
| `jorf` | 30 | 26,999 | 33,095 | 33,200 |
| `sarde` | 24 | 9,124 | 10,307 | 10,567 |
| `titres` | 9 | 24,600 | 28,246 | 28,246 |
| **Plus grand p95 par index (majorant brut)** | **291** | — | **≤ 78,404** | — |

**La purge ne se voit pas.** Les chiffres sont ceux du chaud à quelques dixièmes de milliseconde
près (79,355 ms contre 78,404 ms au p95 d'`articles`). L'échauffement de `crible eval` recharge le
cache de pages avant les trois passes mesurées, puisque les trois répétitions portent sur le même
jeu. La série est donc un **froid partiel de purge unique, dont l'effet est effacé par
l'échauffement**, et la résidence n'est pas mesurée : elle ne prouve rien sur le froid vrai. La
purge a réussi (la commande a rendu 0 à chaque fois), mais elle porte sur toute la VM et sur un
cache que la passe d'échauffement reconstruit aussitôt.

### Lecture complémentaire — les 1 000 titres de MES-00 (3 000 échantillons)

| Index | n | p50 (ms) | p95 (ms) | p99 (ms) |
|---|---:|---:|---:|---:|
| `articles` | 738 | 23,375 | 32,395 | 57,563 |
| `decisions` | 753 | 45,578 | **55,427** | 59,085 |
| `dole` | 333 | 4,228 | 5,183 | 6,021 |
| `jorf` | 375 | 29,004 | 36,104 | 44,802 |
| `sarde` | 387 | 9,769 | 10,464 | 10,631 |
| `titres` | 414 | 24,098 | 28,796 | 29,415 |
| **Plus grand p95 par index (majorant brut)** | **3 000** | — | **≤ 55,427** | — |

C'est la population de même nature que MES-00 (1 000 titres et sommaires, `k` = 10, chaud). Le
p95 par index culmine à 55,4 ms, contre **698 ms** pour pgvector sur la même population. Le
groupé exact n'est pas non plus calculable, pour la même raison. Fidélité : rappel@10 au passage
0,9996 en moyenne sur les 246 titres d'`articles`, 1,0000 sur les cinq autres index.

### Contrôles du point 5

- **Top-10 du dense à vecteur fourni.** Les 97 requêtes ont été rejouées par `crible search`
  (`--mode dense`, `--k 10`, mêmes vecteurs, mêmes filtres) sur le Mac mini **et** sur la machine
  de développement, sur le même volume et la même génération. Les listes de `doc_id` sont
  **identiques sur 97 requêtes sur 97** (25/25 `articles`, 45/45 `decisions`, 6/6 `dole`,
  10/10 `jorf`, 8/8 `sarde`, 3/3 `titres`). Les 97 réponses ont dix résultats. C'est la preuve
  au niveau des `doc_id`, et elle compare deux rejeux de recherche, pas les fichiers versionnés.
- **Comparaison aux JSON versionnés.** Les fichiers
  `eval-<index>-dense-primaire-2026-11-06.json` exigés par ADR-0141, décision 2, **ne portent ni
  `results` ni `doc_id`** : ils ne contiennent que les contrôles par requête. Une comparaison de
  top-10 contre ces fichiers est donc impossible, et le résultat 97/97 ci-dessus reste la preuve
  disponible. La comparaison des contrôles présents, entre le rejeu `dense-vecteur` du Mac mini et
  les JSON versionnés `dense-primaire`, donne **fidélité 97/97** et **qualité 96/97** ;
  seul `Q04` diffère, parce que le jeu de référence a été actualisé le 2026-11-06 et que le rejeu
  local et distant emploie `Q04` actualisé. Le détail est dans `versioned_eval` de
  `eval-macmini-2026-11-07.aggregates.json`.
- **Contrôle supplémentaire.** Les lignes de qualité par requête du dense en texte sont
  **identiques** à celles du rejeu actualisé de `COMPARER-AVEC-PGVECTOR`
  (`data/p10-compare/eval/runs-actualise-complet/`), sur 97 requêtes sur 97.
- **Rappel@10 du dense en texte, jeu actualisé** : **0,500000 au global (n = 56)**,
  **0,322581 sur `juri_texte_integral` (n = 31)** et **0,750000 sur `textes_vigueur` (n = 12)**.
  Les trois valeurs attendues sont retrouvées exactement.

## Conclusion

**Ce que la passe établit.** Le critère d'ADR-0141, décision 4, est le `total` chaud de `crible
eval` local sur les 97 requêtes en texte : le **plus grand p95 par index** vaut **79,355 ms**. Ce
total ne comprend presque pas l'encodage, servi par le cache (0,005 à 0,024 ms au p95, ADR-0051) ;
c'est un fait de mesure, contrairement à l'hypothèse de la décision 4.

La passe C fournit deux lectures qui comprennent l'encodage : **358,959 ms**, **estimation
prudente** (79,355 + 279,604, le p95 fp32 sans cache de MES-05 mesuré sur **300 textes**, donc pas
un majorant strict pour les 97), et **418,425 ms p95** en lecture directe par l'API à cache vide
(82 requêtes uniques), **recoupement** en `client_round_trip` qui ne se compare pas directement aux
temps internes (ADR-0131, décision 7). Les trois lectures sont sous **698 ms** (MES-00,
`docs/08:88`) ; la plus sévère, 418,425 ms, laisse 279,575 ms de marge. Le dense à vecteur fourni
(plus grand p95 par index 78,216 ms), l'hybride (147,300 ms, branche lexicale dominante), le froid
partiel (78,404 ms, sans effet mesurable de la purge) et les 1 000 titres (55,427 ms) restent tous
très en dessous. Le rappel du dense en texte redonne exactement 0,500 / 0,322581 / 0,750, et le
classement du Mac mini est identique à celui de la machine de développement sur 97 requêtes sur 97.

**Ce qu'elle n'établit pas.**

- **Le groupé exact.** Les rapports `eval` ne portent pas d'échantillon par requête ; 79,355 ms est
  le plus grand p95 par index, pas le p95 des 291 échantillons.
- **Le froid.** La purge unique est effacée par l'échauffement, et la résidence n'est pas mesurée :
  aucun chiffre de cette passe ne vaut pour un disque froid.
- **La comparabilité stricte à MES-00.** Trois écarts de conditions, déclarés par ADR-0141,
  décision 3 : VM de **9,70 GiB** et non 11 (ni 7,75 comme au temps de MES-00), disque externe
  USB4, Supabase arrêté. S'y ajoute l'écart de population : 97 requêtes de référence contre 1 000
  titres, et un encodage servi par le cache chez Crible contre un encodage compté chez pgvector.
- **Les états mémoire du conteneur.** `docker stats` n'a pas pu être échantillonné pendant une
  mesure en avant-plan ; MES-09 (passe C) les portera.

**Recommandation** — soumise au propriétaire, aucune décision n'est prise ici : le critère de J10
est tenu selon la décision 4 (79,355 ms, plus grand p95 par index), avec une estimation prudente de
358,959 ms et un recoupement API de 418,425 ms p95 ; la plus sévère reste sous 698 ms. Aucun ADR n'a
à rouvrir la cible ni l'index de plage (ADR-0046). Le sort de l'installation reste celui d'ADR-0141,
décision 6.

## Points à signaler

1. **VM à 9,70 GiB, non 11.** `docker info` donne 10 418 937 856 o ; la valeur mesurée est déclarée
   partout, comme le demandait la passe A.
2. **Le poste `filter` domine `articles`.** 58,6 ms au p95 sur 2,47 M passages, contre 27,1 ms pour
   le scan dense : c'est le filtre d'état des deux corpus d'articles qui porte le p95 de 79,4 ms.
   Sur les 1 000 titres, le même filtre tombe à 0,85 ms au p95 parce que les 246 requêtes ne
   portent que deux filtres distincts, réutilisés. Aucune optimisation n'est faite ici.
3. **L'encodage est servi par le cache** (moins de 0,03 % du total), alors qu'il pèse 11,1 % du
   total de MES-00. La comparaison à 698 ms est favorable par construction sur ce poste.
4. **Le froid partiel ne prouve rien.** Purge unique, échauffement qui recharge le cache, résidence
   non mesurée : la série est publiée pour mémoire, jamais comme un froid.
5. **Une boucle locale a été coupée par le délai de 120 s du terminal**, pendant que le rapport
   `jorf` de la série contraignante s'écrivait. Le rapport a été vérifié complet (10 requêtes,
   `repeat` 3) et aucune série n'a été rejouée. Aucune commande n'a approché les 25 minutes ; la
   plus longue, les 1 000 titres de `decisions`, a pris environ 5 minutes.
