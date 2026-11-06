# Comparaison avec pgvector — jeu de référence Legifacile

## Campagne du 2026-11-06

**Écart « même machine » — à lire avant tout chiffre.** Crible mesure sur le MacBook Air M3 de
développement, dans un conteneur Docker ; pgvector reste en production sur le Mac mini M1. Le
rappel et la fidélité ne dépendent pas de la machine ; seule la latence en dépend, et elle est
**indicative** ici (ADR-0129, décision 7 point 5 ; ADR-0140, décision 1). La latence
**contraignante** reste celle de `MESURER-SUR-MAC-MINI`, mesurée sur le Mac mini en conteneur.
Aucune conclusion de J10 n'est tirée de la latence de ce rapport.

- **Tâche** : `COMPARER-AVEC-PGVECTOR` (P10).
- **Commit Crible** : `012a5eb271ee275f51362c1f2f930e8cd6055cc2`, branche
  `tache/COMPARER-AVEC-PGVECTOR`. Aucun code de production n'a été modifié pendant la campagne.
- **Machine Crible** : MacBook Air Apple M3 (`Mac15,12`), 8 cœurs (4 performance, 4 efficacité),
  24 Go de RAM (25 769 803 776 octets), macOS 15.7.7 (24G720), Docker Desktop 28.5.1, VM de
  12 529 836 032 octets (11,67 Gio) et 8 processeurs. Mesure **dans un conteneur** `crible:p10`.
  Charge de la machine pendant la passe C : 3,84 au début (19:39:44Z), 3,14 à la fin (19:47:10Z),
  sur 8 cœurs.
- **Machine pgvector** : Mac mini de production (Apple M1, 8 cœurs, 16 Go), PostgreSQL 17 dans
  Docker Desktop, lecture seule, hors de `$PROD_DAILY_WINDOW`, entre 18:12:28Z et 18:15:03Z.
  Aucun redémarrage, aucune écriture, aucune valeur de production recopiée.
- **Jeu de données** : les six fichiers `<index>.jsonl` de `docs/legifacile/reference/`, 97 lignes,
  97 `expected`, 35 filtres, plus `mcp.jsonl` pour les périmètres et les paramètres. Empreinte des
  requêtes : SHA-256 `4da55cc1b034aee2b7f78c4c9f4ff8e5590d2b7cea5ebab12022f50f9df8c434`.
  Empreinte des 97 vecteurs de requête : SHA-256
  `b92e4ec1c42cc8b7fc74da370344eb7275f2e8d67e29422d4160e1d12e2ba494`.
- **Index Crible** : 6 660 239 documents et 9 600 105 passages, dans `crible-p10-data`, générations
  `articles` 1, `decisions` 2, `dole` 1, `jorf` 3, `sarde` 1, `titres` 1. Aucun passage mort.
- **Configuration** : `k = 10`, trois répétitions comptées, une passe d'échauffement, mode dense
  pour la qualité, `exact_threshold = 5000`, `rescore_candidates = 5000` ; fidélité avec
  `exact_threshold = 0`, N = 5000, dense seulement. Le rejeu MES-14 emploie le même `k`, le même
  seuil exact, et 20 lignes × 50 répétitions par cas (1 000 échantillons).

### Application d'ADR-0140

| Décision | Ce que la campagne a fait |
|---|---|
| **1 — même machine** | Écart assumé et écrit en tête. pgvector rejoué sur les **97 lignes** de `mcp.jsonl`, pas emprunté à MES-00. Date figée au **2026-11-03** pour les deux lignes `APPLICABLE`. Sessions en lecture seule, hors fenêtre, en série, sans redémarrage. |
| **2 — local, serveur arrêté** | `crible eval` tourne dans un **conteneur neuf éphémère** sur le volume `crible-p10-data`, avec le volume des modèles monté. Le serveur Crible est arrêté pendant la campagne ; aucune restauration des 25,48 Go d'instantanés, aucun index créé, aucun service redémarré. Un index par commande. |
| **3 — trois passes** | Dense primaire avec le **vecteur pgvector** (`halfvec(768)`), dense secondaire avec l'encodage Crible, hybride **en texte seulement** (fusion `rrf`). La passe de fidélité s'y ajoute, imposée par la décision 6. |
| **4 — populations** | Agrégation par `id`, sans filtre ajouté. Seules sont contraignantes les populations **globales** (56), `juri_texte_integral` (31) et `textes_vigueur` (12) ; `jurisprudence` est marquée non comparable par corpus, et les autres périmètres sont lus avec leur `n`. |
| **5 — dense porte J10** | Le dense primaire porte le seuil de 0,482 et les cibles par corpus ; l'hybride est mesuré sur tout le jeu et rapporté, sans faire échouer J10. Ses pertes par requête sont listées plus bas. |
| **6 — fidélité** | 97 requêtes, `exact_threshold = 0`, N = 5000, **dense seulement** ; tous les passages partent en stratégie `binary`. La fidélité pgvector n'est pas remesurée ; MES-00 §3 la donne. |
| **7 — latence indicative** | p50, p95 et p99 des 97 requêtes sont publiés **par étape et pour `total`**, au rang le plus proche et index par index. Aucun second jeu n'a été exécuté ; les 1 000 titres restent disponibles sous `data/mes00/` si le propriétaire l'exige. La différence de population est nommée : 97 requêtes de référence contre 1 000 titres pour MES-00. |
| **8 — MES-14 réel** | `decisions` et `articles`, domaine plein et domaine réduit par une clause en bitmap, sur un jeu filtré généré. Le banc de MES-14 n'est **pas** rejoué : le complément d'ADR-0140 du 2026-11-06 (décision 8) le fixe ainsi, parce que ses onze cas nomment des champs de `synth-small` absents des index réels. La scène `filter` de `crible eval` chronomètre exactement `Pool::evaluate`. |
| **9 — points réservés** | `dedupliquer: "contenu"` reste non mesurable et renvoyé à `BRANCHER-MCP` ; la date figée est nommée ; l'écart `partie_exclue` est documenté. |
| **10 — dérive** | Comptes pgvector relevés avant (18:12:28Z) et après (18:15:03Z), identiques ; comptes Crible relevés avant (18:20:33Z) et après (18:27:11Z), identiques. Les `expected` absents ou hors de leur propre filtre sont traités comme **écart de données**, hors des pertes de classement. |
| **11 — rapports** | Un rapport de comparaison, un JSON pgvector par requête, un JSON pour le rejeu MES-14, le harnais sous `tools/bench/p10-compare/`. Le complément du 2026-11-06 (décision 11) fixe **un rapport `eval` par index et par passe** : les 24 rapports de la passe B (48 fichiers) sont conformes, sans écart de forme. |
| **12 — rappel non atteint** | La tâche n'est pas bloquée. Le seuil global est atteint, `juri_texte_integral` est atteint, `textes_vigueur` manque sous la règle `doc_id` ; la cause est un écart de version, pas un classement perdu. La liste d'actions est en fin de rapport et ne tranche rien. |

### Protocole

**Passe A — pgvector, production, lecture seule.** `replay.py` appelle
`api.recherche_semantique` par SQL direct, sur une connexion neuve dont
`SHOW transaction_read_only` vaut `on`. Un périmètre mono-corpus fait un appel avec
`p_limite = k` ; un périmètre multi-corpus fait un appel par corpus avec `p_limite = k × 2`, puis
trie par distance croissante, filtre l'applicabilité et coupe à `k`. Une passe d'échauffement
précède trois répétitions chronométrées, strictement en série. L'encodeur n'est pas dans la
boucle : les 97 vecteurs sont figés et leur encodage est chronométré à part. Les centiles sont
interpolés, comme dans MES-00, sur 97 × 3 = 291 mesures par poste.

**Passe B — Crible, conteneurs neufs.** `run_eval.sh` lance quatre passes par index : dense
primaire (vecteur de pgvector), dense secondaire (encodage Crible), hybride (texte, fusion `rrf`
par défaut) et fidélité (même jeu que le dense primaire, `exact_threshold = 0`). Un index par
commande, un conteneur éphémère par passe. Le volume est monté en lecture-écriture parce que
SQLite refuse ses fichiers WAL sur un montage `:ro` ; `eval` ouvre l'index en lecture seule et
les relevés `inspect` avant/après sont identiques.

**Passe C — MES-14 sur l'index réel.** `run_mes14.sh games` écrit dix jeux filtrés sous
`data/p10-compare/mes14/games/` : cinq cas par index, 20 lignes chacun. Les bornes répètent celles
de MES-14 (`annee 2000..2010`, `2000-01-01..2003-12-31`) pour que les deux campagnes se lisent
côte à côte ; la clause en bitmap réduit le domaine sans changer la borne. `counts` relit les
instantanés de la même génération, en lecture seule, pour donner `scan_iids` (la taille de
`présent ∩ vivants ∩ bitmap` que reçoit la clause d'intervalle) et `pool_size`. `index` lance un
`crible eval` par cas, en mode dense, avec `--repeat 50` ; `aggregate` joint les rapports et les
comptes. Le banc de MES-14 n'est pas rejoué : le complément d'ADR-0140 du 2026-11-06 (décision
8) le fixe ainsi, pour la raison donnée à la décision 8 ci-dessus.

### Résultats

#### Comptes et dérive des données

| Index | Crible documents | pgvector documents | Écart | Crible passages | pgvector passages | Écart |
|---|---:|---:|---:|---:|---:|---:|
| `articles` | 2 135 023 | 2 135 410 | −387 | 2 476 715 | 2 477 229 | −514 |
| `decisions` | 1 196 623 | 1 196 624 | −1 | 3 589 776 | 3 589 775 | +1 |
| `dole` | 3 600 | 3 600 | 0 | 7 653 | 7 653 | 0 |
| `jorf` | 1 636 145 | 1 636 191 | −46 | 1 833 500 | 1 833 499 | +1 |
| `sarde` | 210 965 | 210 965 | 0 | 214 578 | 214 578 | 0 |
| `titres` | 1 477 883 | 1 478 044 | −161 | 1 477 883 | 1 478 044 | −161 |
| **Total** | **6 660 239** | **6 660 834** | **−595** | **9 600 105** | **9 600 778** | **−673** |

Les comptes pgvector sont **identiques avant et après** le rejeu ; les comptes Crible sont
identiques avant et après la passe B, et l'`inspect` d'après la passe C est identique. Aucune
dérive n'a été observée pendant la campagne. L'écart de 161 documents **et** passages sur
`titres` est un écart de données entre les deux bases, pas un effet du classement ; il n'est pas
corrigé ici (ADR-0140, décision 10).

#### Rappel à `k = 10`

Le seuil de **0,482** se lit sur les 56 questions satisfaisables, `Q30` à part ; les cibles par
corpus portent sur `juri_texte_integral` (31 questions, ≥ 0,290) et `textes_vigueur` (12
questions, ≥ 0,750). Les valeurs sont données sans arrondi au-delà de la sixième décimale.

| Population | n | Dense primaire | Dense secondaire | Hybride | pgvector rejoué | MES-00 (2026-10-20) | Cible |
|---|---:|---:|---:|---:|---:|---:|---:|
| Global satisfaisable | 56 | **0,482143** | 0,482143 | 0,517857 | 0,446429 | 0,482 | **0,482** |
| `juri_texte_integral` | 31 | **0,322581** | 0,322581 | 0,419355 | 0,258065 | 0,290 | **0,290** |
| `textes_vigueur` | 12 | **0,666667** | 0,666667 | 0,583333 | 0,666667 | 0,750 | **0,750** |
| Toutes les 97 lignes | 97 | 0,546392 | 0,546392 | 0,618557 | 0,515464 | — | — |
| `Q30`, non satisfaisable | 1 | 0 | 0 | 0 | 0 | — | — |

MRR à 10, pour lecture : dense primaire 0,285339 (global), 0,155236 (`juri_texte_integral`),
0,354167 (`textes_vigueur`) ; hybride 0,304110, 0,226152, 0,311343 ; pgvector rejoué 0,265696,
0,119752, 0,354167.

Le **dense porte J10** (ADR-0140, décision 5) : il atteint le seuil global (+0,000143) et la cible
`juri_texte_integral` (+0,032581). La cible `textes_vigueur` manque de 0,083333 **sous la règle
`doc_id`** ; la cause est analysée plus bas et n'est pas une perte de classement. L'hybride
dépasse le seuil global et `juri_texte_integral`, mais recule sur `textes_vigueur` ; il reste une
lecture, pas le mode qui tranche.

**Pertes de l'hybride, par requête (ADR-0140, décision 5).** L'hybride gagne neuf lignes sur le
dense primaire et en perd deux, toutes deux dans `textes_vigueur` :

| `id` | Index | Corpus | Dense primaire | Hybride | Cause probable |
|---|---|---|---:|---:|---|
| `Q07` | `articles` | `textes_vigueur` | rappel 1 | rappel 0 | le dense seul trouve le passage ; la fusion `rrf` avec la branche lexicale le fait sortir du top 10 |
| `Q09` | `articles` | `textes_vigueur` | rappel 1 | rappel 0 | même cause |

Les neuf gains sont `Q02` (`articles`, `textes_vigueur`), `Q34`, `Q49` et `Q57` (`decisions`,
`juri_texte_integral`) dans les 56 questions de base, plus `MES00-q0001`, `MES00-q0009`,
`MES00-q0010`, `MES00-q0019` et `Q02-APPLICABLE` hors base. La cause des deux pertes est
**probable** : `crible eval` ne publie pas le rang de chaque branche, seulement le rappel après
fusion. Ce que les rapports prouvent est que le dense primaire trouve `Q07` et `Q09` et que
l'hybride ne les trouve pas ; le seul changement est l'ajout de la branche lexicale et de sa
fusion.

#### Fidélité à 9,6 M passages

Protocole d'ADR-0140, décision 6 : 97 requêtes, `exact_threshold = 0`, N = 5000, dense seulement.
Tous les passages sont partis en stratégie `binary` (`dense_strategy.binary` = nombre de
requêtes).

| Index | n | Rappel@10 | Chevauchement@10 |
|---|---:|---:|---:|
| `articles` | 25 | 1,000000 | 1,000000 |
| `decisions` | 45 | 0,991111 | 0,990864 |
| `dole` | 6 | 1,000000 | 1,000000 |
| `jorf` | 10 | 1,000000 | 1,000000 |
| `sarde` | 8 | 1,000000 | 1,000000 |
| `titres` | 3 | 1,000000 | 1,000000 |
| **Global** | **97** | **0,995876** | — |

La cible de 0,99 est tenue globalement et sur chaque index. Le point le plus bas est `decisions`
(0,991111), au-dessus de la cible. La fidélité de pgvector n'est pas remesurée : MES-00 §3 la
donne, et ADR-0140, décision 6, ne demande pas de nouvelle campagne HNSW.

#### Latence par étape, 97 requêtes

Latence **indicative**, lue dans `debug.timings_ms`, **par étape et pour `total`**. Les centiles de
`eval` sont au **rang le plus proche** (ADR-0117, décision 4 ; ADR-0125, décision 9), **index par
index** : l'échantillon vaut 3 répétitions × le nombre de requêtes de l'index, soit 75
(`articles`), 135 (`decisions`), 18 (`dole`), 30 (`jorf`), 24 (`sarde`) et 9 (`titres`). Le p99
est donc grossier sur les petits index, et `dole` et `titres` ont p95 = p99 parce que le rang le
plus proche tombe sur le maximum. Seuls les centiles pgvector sont **interpolés**, sur 97 × 3 =
291 mesures (ADR-0140, conséquence négative).

| Index | Dense primaire p50 | p95 | p99 | Hybride p50 | p95 | p99 |
|---|---:|---:|---:|---:|---:|---:|
| `articles` | 9,979 | 58,708 | 60,337 | 42,372 | 80,947 | 94,756 |
| `decisions` | 18,932 | 23,531 | 30,098 | 73,985 | 105,891 | 113,060 |
| `dole` | 2,399 | 9,845 | 9,845 | 1,971 | 7,825 | 7,825 |
| `jorf` | 11,961 | 21,712 | 21,844 | 19,588 | 34,664 | 34,722 |
| `sarde` | 4,972 | 6,987 | 7,059 | 5,393 | 6,072 | 6,357 |
| `titres` | 11,004 | 11,779 | 11,779 | 17,228 | 21,039 | 21,039 |

Le p95 de `articles` (58,708 ms) vient d'un filtre d'ensemble `etat` dont la scène `filter` atteint
49,397 ms au p95 ; le reste de la distribution est à 9 à 12 ms. La cible de 698 ms n'est pas jugée
ici : elle ne vaut que sur le Mac mini en conteneur.

**Par étape.** Les tableaux suivants donnent p50 / p95 / p99 de chaque étape, en ms ; `total`
reste celui du tableau précédent. Les étapes sont celles de `debug.timings_ms` : `validate`,
`filter`, `encode`, `dense`, `lexical`, `fusion`, `fetch`, `group`.

*Dense primaire*

| Index | validate | filter | dense | fetch | group |
|---|---:|---:|---:|---:|---:|
| `articles` | 0,005 / 0,008 / 0,008 | 0,209 / 49,397 / 50,468 | 9,152 / 11,170 / 11,584 | 0,197 / 0,469 / 0,671 | 0,001 / 0,001 / 0,002 |
| `decisions` | 0,000 / 0,006 / 0,008 | 0,048 / 0,211 / 0,311 | 18,556 / 22,827 / 29,768 | 0,223 / 0,521 / 1,179 | 0,001 / 0,002 / 0,002 |
| `dole` | 0,000 / 0,008 / 0,008 | 0,002 / 0,005 / 0,005 | 2,204 / 9,441 / 9,441 | 0,155 / 0,339 / 0,339 | 0,001 / 0,003 / 0,003 |
| `jorf` | 0,000 / 0,005 / 0,005 | 0,023 / 17,827 / 18,069 | 11,526 / 13,436 / 13,860 | 0,145 / 0,232 / 0,233 | 0,001 / 0,002 / 0,003 |
| `sarde` | 0,000 / 0,005 / 0,005 | 0,004 / 0,007 / 0,008 | 4,777 / 6,769 / 6,837 | 0,162 / 0,180 / 0,184 | 0,001 / 0,002 / 0,002 |
| `titres` | 0,000 / 0,000 / 0,000 | 0,021 / 0,024 / 0,024 | 10,629 / 11,553 / 11,553 | 0,138 / 0,566 / 0,566 | 0,001 / 0,001 / 0,001 |

*Hybride*

| Index | validate | filter | encode | dense | lexical | fusion | fetch | group |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| `articles` | 0,006 / 0,011 / 0,018 | 0,208 / 49,155 / 49,625 | 0,003 / 0,004 / 0,009 | 10,020 / 12,032 / 12,123 | 40,720 / 67,495 / 70,360 | 0,018 / 0,021 / 0,026 | 0,201 / 0,252 / 0,271 | 0,002 / 0,002 / 0,002 |
| `decisions` | 0,000 / 0,009 / 0,076 | 0,059 / 0,304 / 0,594 | 0,007 / 0,023 / 0,058 | 20,767 / 25,797 / 30,918 | 73,574 / 105,449 / 111,658 | 0,020 / 0,026 / 0,052 | 0,256 / 1,081 / 1,896 | 0,002 / 0,002 / 0,003 |
| `dole` | 0,000 / 0,005 / 0,005 | 0,001 / 0,003 / 0,003 | 0,001 / 0,004 / 0,004 | 1,753 / 7,584 / 7,584 | 0,601 / 4,191 / 4,191 | 0,013 / 0,016 / 0,016 | 0,173 / 0,211 / 0,211 | 0,001 / 0,002 / 0,002 |
| `jorf` | 0,000 / 0,007 / 0,009 | 0,031 / 18,007 / 20,405 | 0,002 / 0,003 / 0,004 | 12,913 / 14,951 / 15,001 | 12,062 / 34,364 / 34,377 | 0,015 / 0,020 / 0,029 | 0,150 / 0,276 / 0,317 | 0,001 / 0,002 / 0,005 |
| `sarde` | 0,000 / 0,005 / 0,005 | 0,004 / 0,007 / 0,007 | 0,002 / 0,002 / 0,003 | 5,183 / 5,829 / 6,171 | 3,278 / 5,291 / 5,359 | 0,013 / 0,023 / 0,024 | 0,167 / 0,185 / 0,185 | 0,001 / 0,002 / 0,002 |
| `titres` | 0,000 / 0,000 / 0,000 | 0,025 / 0,031 / 0,031 | 0,002 / 0,003 / 0,003 | 11,577 / 12,278 / 12,278 | 16,923 / 20,714 / 20,714 | 0,018 / 0,020 / 0,020 | 0,188 / 0,213 / 0,213 | 0,001 / 0,002 / 0,002 |

*Dense secondaire*

| Index | validate | filter | encode | dense | fetch | group |
|---|---:|---:|---:|---:|---:|---:|
| `articles` | 0,006 / 0,011 / 0,020 | 0,203 / 49,143 / 50,173 | 0,003 / 0,012 / 0,017 | 9,103 / 10,878 / 11,650 | 0,198 / 0,307 / 0,343 | 0,001 / 0,001 / 0,002 |
| `decisions` | 0,000 / 0,008 / 0,046 | 0,047 / 0,275 / 0,323 | 0,002 / 0,024 / 0,029 | 18,788 / 20,968 / 22,523 | 0,227 / 1,612 / 3,512 | 0,001 / 0,001 / 0,007 |
| `dole` | 0,000 / 0,005 / 0,005 | 0,001 / 0,003 / 0,003 | 0,001 / 0,003 / 0,003 | 1,815 / 8,494 / 8,494 | 0,156 / 0,219 / 0,219 | 0,001 / 0,002 / 0,002 |
| `jorf` | 0,000 / 0,008 / 0,008 | 0,024 / 17,748 / 18,904 | 0,002 / 0,004 / 0,006 | 11,509 / 13,187 / 13,247 | 0,145 / 0,208 / 0,210 | 0,001 / 0,001 / 0,001 |
| `sarde` | 0,000 / 0,005 / 0,006 | 0,004 / 0,007 / 0,007 | 0,002 / 0,002 / 0,002 | 4,983 / 6,731 / 6,989 | 0,164 / 0,190 / 0,195 | 0,001 / 0,002 / 0,002 |
| `titres` | 0,000 / 0,000 / 0,000 | 0,022 / 0,030 / 0,030 | 0,002 / 0,002 / 0,002 | 10,258 / 11,270 / 11,270 | 0,139 / 0,192 / 0,192 | 0,001 / 0,001 / 0,001 |

L'écart entre les deux passes, en valeur absolue, atteint au plus 0,491 ms au p50 (`titres`),
1,173 ms au p95 (`decisions`) et 4,549 ms au p99 (`decisions`) ; il change de signe selon
l'index.

Pour mémoire, le rejeu pgvector sur la même machine de production donne, sur 291 mesures :

| Poste pgvector | p50 | p95 | p99 |
|---|---:|---:|---:|
| `sql` | 13,357 | 27,723 | 32,754 |
| `reconstruction` | 80,189 | 955,245 | 1 986,913 |
| `total` | 95,458 | 975,760 | 2 009,289 |

L'encodage des 97 textes, chronométré à part, donne p50 32,874 ms, p95 51,640 ms et p99
59,966 ms. Cette p95 pgvector dépasse 698 ms, mais sur une **autre population** (97 requêtes de
référence, plus longues que les 1 000 titres de MES-00) et avec la reconstruction dans le total ;
elle ne se lit pas comme un échec de J10.

#### Rejeu de MES-14 sur l'index réel

`filter_ms` est la scène `filter` de `crible eval`, c'est-à-dire l'appel `Pool::evaluate` seul,
avant le scan dense. `total_ms` est la requête dense complète. Chaque cas porte 1 000
échantillons (20 lignes × 50 répétitions), cache chaud. `scan_iids` est la taille de
`présent ∩ vivants ∩ bitmap` que reçoit la clause d'intervalle ; `pool` est la taille du résultat.

**`decisions` — 3 589 776 passages vivants, génération 2**

| Cas | `scan_iids` | `pool` | Filtre p50 | p95 | p99 | Total p50 | p95 | p99 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Clause en bitmap seule | — | 202 425 | 0,004 | 0,006 | 0,008 | 4,949 | 5,470 | 5,946 |
| `annee`, domaine plein | 3 589 753 | 1 133 472 | **63,740** | **68,167** | 69,570 | 73,052 | 77,705 | 79,542 |
| `date_decision`, domaine plein | 3 589 776 | 263 057 | **59,414** | **63,313** | 64,375 | 64,907 | 68,778 | 70,471 |
| `annee`, domaine réduit | 202 425 | 48 197 | 4,404 | 4,557 | 4,655 | 7,909 | 8,314 | 8,701 |
| `date_decision`, domaine réduit | 202 425 | 1 655 | 3,986 | 4,154 | 4,352 | 7,618 | 8,696 | 10,852 |

**`articles` — 2 476 715 passages vivants, génération 1**

| Cas | `scan_iids` | `pool` | Filtre p50 | p95 | p99 | Total p50 | p95 | p99 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Clause en bitmap seule | — | 168 875 | 0,040 | 0,050 | 0,059 | 4,713 | 5,225 | 5,526 |
| `annee`, domaine plein | 2 231 423 | 550 966 | **39,761** | **41,496** | 42,554 | 47,076 | 49,016 | 51,368 |
| `date_debut`, domaine plein | 2 476 715 | 138 740 | **41,344** | **42,504** | 43,777 | 46,771 | 48,035 | 49,531 |
| `annee`, domaine réduit | 168 860 | 31 869 | 4,134 | 4,306 | 4,392 | 7,641 | 8,026 | 8,401 |
| `date_debut`, domaine réduit | 168 875 | 7 046 | 3,768 | 3,988 | 4,029 | 6,104 | 6,435 | 6,671 |

Le coût par iid lu est de 17,76 ns (`decisions`, `annee` plein), 16,55 ns (`decisions`, date
pleine), 17,82 ns (`articles`, `annee` plein) et 16,69 ns (`articles`, date pleine). Sur un
domaine réduit, il monte à 19,7 à 24,5 ns : le parcours est épars, comme MES-14 l'avait observé.
MES-14 donnait 13,9 et 12,1 ns/iid à 8,58 M sur des fixtures ; l'index réel est donc environ
28 à 38 % plus cher par iid à domaine plein, sans que la matrice isole la cause (distribution,
colonnes trouées par les valeurs nulles, résidence des pages). Le rapport ne l'extrapole pas.

La réduction du domaine par la clause en bitmap divise le coût du filtre par **14,5** pour
`annee` et **14,9** pour la date sur `decisions`, par **9,6** et **11,0** sur `articles` ; elle
ramène `scan_iids` de 3,59 M à 202 k (`decisions`) et de 2,23–2,48 M à 169 k (`articles`). Le
filtre reste néanmoins à 3,8–4,4 ms, soit 3,8 à 4,4 fois la ligne « filtre (bitmaps) < 1 ms » de
`docs/03-recherche.md`. Le coût d'intervalle mesuré ici (41,5–68,2 ms au p95 à domaine plein)
reste dans la ligne que `docs/03` a déjà ouverte avec MES-14 (101 à 194 ms à 8,58 M) ; il ne la
dépasse pas.

#### Requêtes gagnantes et perdantes

Le dense primaire **gagne trois lignes et n'en perd aucune** face au rejeu pgvector :
`Q52`, `Q58` et `MES00-q0018`. Seules les deux premières appartiennent aux 56 questions
satisfaisables ; `MES00-q0018` est hors base et ne pèse pas sur 0,482.

- **`Q52` — gagnée par la forme du périmètre.** MES-00 la lisait sur le seul corpus
  `juri_texte_integral` et la trouvait au rang 3 (`MES-00…eval-recherche.md:167`) ; le diagnostic
  mono-corpus du rejeu la place aussi au rang 3 (`diag.json`, `hnsw`). Le rejeu suit le périmètre
  `jurisprudence` de `mcp.jsonl` : deux corpus, `k × 2` résultats chacun, fusion, coupe à `k`.
  Sous cette forme, pgvector la laisse **hors de son top 10** (`replay.json` ne publie que dix
  résultats). Crible la trouve, au rang 10 en dense binaire avec rescoring
  (`diag-vectors/pairs-out/decisions/Q52.primary.json`) ; la recherche exacte la place au rang 11
  (`diag-vectors/Q52.search.json`). C'est un **choix de périmètre**, pas un gain de classement :
  la ligne est à la frontière du top 10.
- **`Q58` — gagnée sur une vraie ratée HNSW.** pgvector la manque alors que l'exact la place au
  rang 1 : c'est le comportement HNSW attendu, et Crible la trouve. Aucune action pgvector.
- **`MES00-q0018` — gagnée hors base.** Ligne `MES00-*`, hors des 56 ; elle est rapportée pour
  lecture seulement.

Le dense primaire ne **perd** aucune question face au rejeu pgvector. Les questions manquées par
les deux côtés sont `Q01`, `Q02`, `Q04`, `Q06`, `Q14`, `Q15`, `Q16`, `Q19`, `Q20`, `Q22`, `Q23`,
`Q31`, `Q34`, `Q35`, `Q38`, `Q39`, `Q42`, `Q43`, `Q45`, `Q47`, `Q48`, `Q49`, `Q50`, `Q51`,
`Q53`, `Q54`, `Q55`, `Q56` et `Q57`. Deux questions demandent une explication de données : `Q04`,
parmi les précédentes, et `Q30`, comptée à part.

- **`Q04` — écart de version, pas de classement.** MES-00 trouvait l'attendu
  `LEGIARTI000047381704` au rang 3 dans `textes_vigueur` (`MES-00…eval-recherche.md:120`). Le
  rejeu ne le trouve plus : il est absent du corpus `textes_vigueur` (`diag.json`, `presence: []`),
  et seule y subsiste une version plus récente du même article, `LEGIARTI000054716237` (`texte_cid`
  et `num` identiques, `article_versions`). Crible place cette version au **rang 3** avec le filtre
  de la ligne (`diag-vectors/Q04.filtered.search.json`). Ce n'est donc pas une règle d'appariement
  implicite : la version attendue a changé entre la campagne MES-00 et le rejeu. C'est **à lui
  seul** l'écart de `textes_vigueur` (0,666667 au lieu de 0,750) sous la règle `doc_id`.
- **`Q30` — non satisfaisable, comptée à part.** L'`expected` est un article `ABROGE_DIFF`, que
  le filtre d'état de la ligne exclut ; MES-00 le signalait déjà comme anomalie quand pgvector le
  trouvait (`MES-00…eval-recherche.md:41`, `:146`). Le rejeu ne le trouve pas (`presence: []`) ;
  seule la version courante `LEGIARTI000052849819` est présente, au rang 1 côté pgvector
  (`article_match_hnsw`). Sous `doc_id`, la ligne reste non satisfaisable, et le jeu le déclare
  déjà.

Les autres questions manquées par les deux côtés sont des ratées de classement partagées, pas des
pertes face à pgvector. Les rangs Crible relevés avec filtre sont `Q02` 14, `Q06` 171 et `Q01`
au-delà des 968 résultats rendus ; `Q02` est donc la plus proche du top 10.

#### Vérifications de la passe B

**Le dense secondaire est bien l'encodage de Crible, pas une copie du primaire.** Les 97 vecteurs
de Crible ont été comparés aux 97 vecteurs de pgvector : cosinus minimum 0,9999999999847,
médiane 0,9999999999982, maximum 0,9999999999991 en fp32 ; en int8, minimum 0,8986 et médiane
0,9432. Les deux passes donnent les **mêmes top 10 sur 97 requêtes sur 97**. L'écart d'encodeur
est donc nul à la précision fp32 sur ce jeu, et la passe secondaire n'explique aucun écart de
rappel : elle le confirme.

**Les passes B et C n'ont laissé aucune écriture durable sous `/data`.** Les inventaires
conservés après chaque passe portent les mêmes **361 chemins**, les mêmes tailles et les mêmes
horodatages de fichier. Entre les deux, seuls les six répertoires `epochs/<n>` changent de mtime
(18:26:47Z–18:27:11Z après B, 19:47:19Z–19:47:42Z après C), ce qui correspond à la création puis
à la suppression des fichiers `records.sqlite-wal` et `-shm` transitoires du montage
lecture-écriture. Aucun `-wal` ni `-shm` d'index ne subsiste ; les `jobs.sqlite-wal` et `-shm`
datent de 14:13 et précèdent la campagne. Les comptes `inspect` sont identiques avant et après.

#### Écarts et points réservés

- **Forme des rapports `eval`.** Le complément d'ADR-0140 du 2026-11-06 (décision 11) fixe
  **un rapport par index et par passe**. La passe B publie donc **24 rapports** (quatre passes ×
  six index), soit 48 fichiers `.json` et `.md`, sans écart : le harnais écrit **un mode par
  rapport**, pour que chaque fichier reste le rapport brut d'un seul réglage, alors que
  `crible eval` accepte plusieurs modes et les publie dans une carte `modes`. Le suffixe de
  passe est explicite et chaque fichier reste le rapport brut de `crible eval`, sans retouche.
  Aucun rapport n'est écrasé ni renommé.
- **`dedupliquer: "contenu"`.** Non mesurable par ce jeu : le SQL du MCP déduplique sur
  l'empreinte du document, alors que Crible fusionne les passages de contenu identique ; aucune
  ligne ne porte l'option et `crible eval` ne l'expose pas. Renvoyé à `BRANCHER-MCP` (ADR-0140,
  décision 9a). Aucune conclusion ici.
- **Date figée.** Les deux lignes `APPLICABLE` sont rejouées au 2026-11-03 des deux côtés. Le
  serveur n'expose aucun moyen de fixer la date ; le rejeu appelle la même fonction avec cet
  argument (décision 9b).
- **`partie_exclue`.** Les deux lignes `Q13-PARTIE-EXCLUE-DEFAUT` et
  `Q31-PARTIE-EXCLUE-DEFAUT` passent la liste explicite des deux côtés. Le défaut du serveur
  (`moyens`, `faits`) est une décision de `serveur_mcp.outils`, pas de la base. Ces deux lignes
  sont hors des 56 questions satisfaisables (décision 9c).
- **p99.** Les centiles de `eval` sont au rang le plus proche, index par index, sur 9 à 135
  mesures ; `dole` et `titres` ont p95 = p99. Le rejeu pgvector porte sur 291 mesures
  interpolées. Le rapport le déclare ; aucune décision n'est prise sur p99.
- **Le rejeu MES-14 ne remplace pas le banc.** `crible eval` mesure le bout en bout et sa scène
  `filter` ; le banc mesure `Pool::evaluate` seul, sans encodage ni scan, sur des fixtures. Les
  deux chiffres sont complémentaires. Le banc n'est pas rejoué ici : le complément d'ADR-0140
  du 2026-11-06 (décision 8) le fixe, parce que ses onze cas nomment des champs de `synth-small`
  absents des index réels et qu'ADR-0140 interdit de le modifier.

### Lecture

Ce que la campagne permet d'affirmer :

1. **Le seuil global de 0,482 est atteint** par le dense primaire : 0,482143 sur 56 questions,
   soit +0,000143. `juri_texte_integral` est atteint avec 0,322581 (cible 0,290). La fidélité
   globale est de 0,995876, au-dessus de 0,99, et aucun index n'est sous 0,991.
2. **`textes_vigueur` manque sous la règle `doc_id`** (0,666667 au lieu de 0,750), mais l'écart
   à la cible est intégralement expliqué par un changement de version : la version courante de
   `Q04` est au rang 3 avec le filtre de la ligne. Sous une règle d'identité d'article
   `(texte_cid, num)` — règle que MES-00 n'écrit pas et qui reste à décider —, la population
   reviendrait à 9/12, soit 0,750. Ce n'est pas une perte de classement, et ce n'est pas une
   régression de Crible.
3. **Le dense primaire ne perd aucune question face au rejeu pgvector** et en gagne trois, dont
   deux dans les 56. `Q52` relève du périmètre, `Q58` d'une vraie ratée HNSW.
4. **La latence de bout en bout reste sous 698 ms** sur les 97 requêtes, dans le conteneur de
   développement : p95 maximale 105,891 ms en hybride, 58,708 ms en dense primaire. Ce n'est
   qu'indicatif ; la mesure contraignante est celle du Mac mini.
5. **Le rejeu MES-14 retrouve l'ordre de grandeur de la campagne de fixtures** : 63,3 à 68,2 ms
   au p95 pour un domaine plein de 3,59 M passages (`decisions`), 41,5 à 42,5 ms pour 2,23 à
   2,48 M (`articles`), et 4,0 à 4,6 ms sous un domaine réduit à 169 à 202 k. Le filtre
   d'intervalle reste 3,8 à 68 fois au-dessus de la ligne « filtre (bitmaps) < 1 ms », mais il
   tient dans le budget de bout en bout.
6. **Aucune dérive de données pendant la campagne** : comptes pgvector identiques avant/après,
   comptes Crible identiques, aucune écriture durable sous `/data` (seuls les `-wal` et `-shm`
   transitoires apparaissent puis disparaissent).

Ce que la campagne ne permet pas d'affirmer :

- **Rien sur la latence contraignante de J10.** Les deux machines diffèrent, les populations
  diffèrent (97 requêtes contre 1 000 titres), et la reconstruction pgvector n'a pas
  d'équivalent chronométré côté Crible. Seul `MESURER-SUR-MAC-MINI` tranche.
- **Rien sur `dedup: "content"`.** Le jeu ne le porte pas et `crible eval` ne l'expose pas.
- **Rien sur la fidélité HNSW de pgvector.** Elle n'est pas remesurée ; MES-00 §3 la donne.
- **Rien sur l'origine de l'écart de coût par iid** entre les fixtures de MES-14 et l'index réel
  (17 à 18 ns contre 12 à 14 ns à domaine plein). La matrice ne l'isole pas.
- **Rien sur un domaine froid.** Toutes les structures de filtre sont en RAM ; le cache de
  pages n'entre pas dans la boucle.

### Actions soumises au propriétaire

Aucune n'est appliquée. Chacune demande une décision ; les agents ne modifient ni le jeu, ni la
cible, ni un ADR.

1. **Trancher la règle d'identité de `textes_vigueur`.** Lire la cible de 0,750 sur le `doc_id`
   du jeu, ou sur une identité d'article `(texte_cid, num)` — règle que ni le jeu ni MES-00
   n'écrivent, et qu'il faudrait donc ajouter. Si l'identité d'article est retenue, l'écrire dans
   le jeu et dans ADR-0140, et rejouer les deux côtés ; la version courante de `Q04` est au rang
   3 et `textes_vigueur` revient à 0,750. Si le `doc_id` est retenu, rafraîchir les `expected`
   périmés (`Q04`, et un audit des 35 lignes filtrées) puis rejouer.
2. **Rafraîchir l'`expected` de `Q04` côté Legifacile.** L'`expected` de `Q04` n'est plus la
   version courante : l'article a changé de version. C'est un travail de données du projet
   Legifacile, pas une correction de Crible. `Q30` reste hors de cette action : le jeu le
   déclare non satisfaisable par construction, et le rafraîchir changerait la population
   satisfaisable (56 → 57) et la base de `textes_vigueur` (12 → 13), donc les cibles.
3. **Fixer la forme du périmètre `jurisprudence`.** Le rejeu suit la fusion multi-corpus de
   `mcp.jsonl` ; MES-00 lisait `juri_texte_integral` seul. Écrire la règle dans le jeu, puis
   relire `Q52` : Crible la place au rang 11 en exact sous la fusion et au rang 3 sous le
   mono-corpus ; pgvector la laisse hors de son top 10 sous la fusion et la place au rang 3 sous
   le mono-corpus. Sans cette règle, la comparaison de `Q52` n'a pas de forme stable.
4. **Ne pas ouvrir d'ADR d'index de plage maintenant.** Le déclencheur d'ADR-0046 et de la fiche
   est une p95 de bout en bout **mesurée sur le Mac mini** au-dessus de 698 ms. Le rejeu local
   donne 58,7 ms au p95 en dense primaire et 105,9 ms en hybride ; le rejeu MES-14 donne 68,2 ms
   au p95 pour le filtre seul. Aucun de ces chiffres ne déclenche l'ADR. La question reste
   ouverte pour `MESURER-SUR-MAC-MINI`.
5. **Compléter `docs/03-recherche.md` avec les chiffres de l'index réel.** La ligne « filtre
   d'intervalle » ne cite que les fixtures à 8,58 M (101 à 194 ms à domaine plein, 9,1 à 49,2 ms
   sous domaine réduit). Les valeurs réelles sont 63,3 à 68,2 ms (`decisions`) et 41,5 à 42,5 ms
   (`articles`) à domaine plein, 3,8 à 4,4 ms sous domaine réduit. La ligne resterait une
   **mesure**, pas une cible.
6. **Décider si la p95 pgvector de 975,760 ms doit être relue sur la population de MES-00.** Ce
   chiffre est sur 97 requêtes de référence et inclut la reconstruction ; il n'est pas comparable
   aux 698 ms des 1 000 titres. Si le propriétaire veut une p95 de même population, les 1 000
   titres et leurs vecteurs sont déjà sous `data/mes00/` (ADR-0140, décision 7), sans réexport.
7. **Documenter l'écart de 161 documents et passages sur `titres`.** Crible en a 1 477 883,
   pgvector 1 478 044. C'est un écart de données, pas de classement ; ni resynchronisation ni
   correction de données ne sont proposées par un agent.

**Recommandation.** Ne pas ouvrir d'ADR d'index de plage : le déclencheur n'est pas atteint.
Traiter d'abord la règle d'identité et le rafraîchissement des `expected` (actions 1 et 2) : c'est
la seule cause de l'écart de `textes_vigueur`, et cela suffit à ramener la population à sa cible
de 0,750 si le propriétaire retient l'identité d'article. Ensuite, fixer la forme du périmètre
`jurisprudence` (action 3) et lancer `MESURER-SUR-MAC-MINI`, seule mesure qui peut rouvrir la
question de l'index de plage.

### Données brutes

| Chemin | Contenu | Versionné |
|---|---|---|
| `docs/mesures/comparaison-pgvector-2026-11-06.pgvector.json` | rejeu pgvector : `id`, `doc_id`, centiles, comptes | oui |
| `docs/mesures/comparaison-pgvector-2026-11-06.mes14.json` | rejeu MES-14 : cas, `scan_iids`, `pool`, centiles | oui |
| `docs/mesures/eval-<index>-<passe>-2026-11-06.{json,md}` | 24 rapports bruts de la passe B (48 fichiers) | oui |
| `data/p10-compare/queries.json`, `query-vectors.json` | 97 textes joints et 97 vecteurs | non |
| `data/p10-compare/replay.json`, `counts-{before,after}.json` | rejeu pgvector brut et comptes datés | non |
| `data/p10-compare/diag.json`, `diag-vectors/` | diagnostic pgvector des `Q04`, `Q30`, `Q52`, `Q58` : présence, versions, rangs HNSW et exacts, recherches Crible filtrées | non |
| `data/p10-compare/eval/aggregates.json`, `runs/`, `inputs/` | agrégats, rapports et jeux d'entrée de la passe B | non |
| `data/p10-compare/eval/counts-{before,after}.{txt,at}`, `inputs.sha256` | comptes Crible datés avant/après et empreinte des jeux d'entrée | non |
| `data/p10-compare/data-inventory-after-passeB.txt` | inventaire de `/data` après la passe B | non |
| `data/p10-compare/mes14/games/`, `games.sha256` | dix jeux filtrés générés et leurs SHA-256 | non |
| `data/p10-compare/mes14/counts.json`, `runs/`, `inspect-after.txt` | comptes de domaine, rapports et `inspect` de la passe C | non |
| `data/p10-compare/mes14/data-inventory-after-passeC.txt` | inventaire de `/data` après la passe C | non |
| `data/p10-compare/mes14/conditions-{start,end}.txt` | machine, commit et charge | non |

Les jeux filtrés et les textes de requête restent sous `data/` (gitignoré) : ni le rapport, ni le
JSON pgvector, ni le JSON MES-14 ne portent un texte de requête ou une valeur de filtre
(ADR-0125, décision 16 ; ADR-0140, décision 11).
