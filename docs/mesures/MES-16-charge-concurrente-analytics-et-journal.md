# MES-16 — Charge concurrente avec analytics et journal de debug actifs

Question : les recherches concurrentes d'un index restent-elles sous le critère
d'INV-02 quand les analytics tournent à leur valeur par défaut **et** que le
journal de debug enregistre chaque requête (`sample_rate = 1.0`, le pire cas) ?
C'est le rejeu de MES-16 demandé par ADR-0122, décision 12, qui renvoie au
protocole d'ADR-0113, décision 5, et à ADR-0118, décision 8. La campagne de
référence, sans analytics ni journal actifs, est
`docs/mesures/MES-16-charge-concurrente.md` (campagnes du 2026-10-30).

## Campagne du 2026-11-01

- **Tâche** : `JOURNAL-DE-DEBUG` (phase P7). Livrable de la fiche
  `docs/phases/P7-observabilite.md`, section `JOURNAL-DE-DEBUG`.
- **Commit** : `dacb239` (`dacb239f1a445fcb321d80852f8b61523eb70b13`) plus le
  travail non commité de la tâche dans l'arbre, sur la branche
  `tache/JOURNAL-DE-DEBUG`. Le binaire mesuré est construit depuis cet arbre par
  `cargo build --release -p crible` ; son empreinte SHA-256 est
  `3f0532a6292a5d875e441034f66bf07eea84a2dc5682cc9c627ef7544c0223ac`.
- **Machine** : MacBook Air M3 (`Mac15,12`), 8 cœurs, 24 Gio de mémoire
  (25 769 803 776 octets), macOS 15.7.7 (24G720), exécution **native**, pas de
  conteneur. `hw.model` rend `Mac15,12` et `machdep.cpu.brand_string` rend
  `Apple M3`, lus par `sysctl` au moment de la campagne. C'est le modèle que
  nomment déjà MES-01 (`docs/mesures/MES-01-fidelite-scan-binaire.md:26`), MES-02
  et MES-03 : les deux séries de MES-16 ont donc tourné sur le même modèle, et la
  comparaison avec les campagnes du 2026-10-30 porte bien sur le même matériel.
  Le « Mac mini (Apple M4) » du rapport de P6 est une coquille, sur la forme
  comme sur la puce ; ce rapport-ci ne la corrige pas, le fichier étant commité
  et la correction appartenant au propriétaire.
  Les données brutes de cette campagne ne portent pas encore `hw.model` ni
  `machdep.cpu.brand_string` (le harnais ne les enregistre que depuis cette
  relecture) : l'identification ci-dessus vient de `sysctl` lu pendant la
  campagne, et concorde avec MES-01 à MES-03, qui nomment la même machine.
- **Charge de la machine** : la moyenne de charge a été relevée juste avant
  chaque campagne. `uptime` donne 3,74 sur 8 cœurs avant la première, 5,92 avant
  la deuxième et 6,58 avant la troisième. Les processus tiers les plus actifs
  relevés par `ps -Ao pcpu,comm -r` sont `mediaanalysisd` (77 à 88 % d'un cœur),
  `mds_stores` (9 à 36 %), `Arc` et ses aides de rendu (11 à 40 %), et
  `opencode` (11 à 19 %). Aucun n'a été arrêté : la mesure se compare à
  elle-même sur la même machine chargée, elle ne donne pas une latence absolue
  transposable.
- **Disque** : `df -h /System/Volumes/Data` donne 99 Gio libres avant la
  campagne, 99 Gio après.
- **Jeu de données** : synthétique, produit par le harnais avec la graine
  `20261030` (`tools/bench/mes16/mes16_charge.py`, `SEED`, inchangée), 50 000
  passages de base puis cinq lots de 2 000 passages, modèle `external` à
  1 024 dimensions, branche lexicale activée. Les vecteurs sont tirés selon une
  loi normale normalisée en fp32, les textes depuis un vocabulaire de seize
  mots. Le harnais supprime le dossier temporaire de chaque campagne : il
  n'existe donc pas de somme de contrôle du jeu à citer, seuls l'identifiant du
  lot de base (`mes16-base`), ceux des cinq lots de mise à jour et les
  générations sont relevés. Le jeu se reproduit par la graine et les paramètres
  du protocole.
- **Configuration** : identique à la campagne du 2026-10-30, à deux différences
  près, celles d'ADR-0122, décision 12. `[analytics]` garde sa valeur par défaut
  de `docs/07`, `retention_days = 90`. `[debug_journal]` porte
  `enabled = true`, `sample_rate = 1.0` et `ttl_days = 7` : chaque recherche est
  capturée et déposée dans la file du journal. Le reste ne change pas :
  `reference_precision = fp16`, `search.scan = binary`,
  `rescore_candidates = 5000`, `exact_threshold = 5000`,
  `candidates_per_branch = 100`, `fusion = rrf`, `lexical.enabled = true` avec
  `language = fr` et `positions = false`, `compaction.dead_ratio = 0.10`,
  `search_threads = 0` (un thread par cœur disponible) et
  `max_concurrent_searches = 0` (valeur par défaut du serveur). `k = 10`,
  `N = 4` clients, aucun encodeur chargé.

### Protocole

Le protocole est celui d'ADR-0113, décision 5, sans autre écart que les deux
clés de configuration ci-dessus : échauffement de 30 s écarté des résultats,
repos de 60 s servant de référence, puis cinq attaches et une compaction ;
latence à l'horloge murale côté client, `took_ms` du serveur rapporté à côté ;
percentiles au rang le plus proche (ADR-0077). Les attaches sont soumises une à
une et attendues jusqu'à leur état terminal (`client.wait`, sondage toutes les
0,2 s). Le critère bloquant est celui de la décision 1 (pendant chaque phase
d'écriture, **chaque** recherche se termine en moins de 3 × le p99 du repos) ;
le critère de la fiche (p99 de la phase d'écriture sous 3 × le p99 du repos) est
rapporté aussi. Les codes d'état suivent la décision 6.

Le harnais ne savait pas activer le journal : `configuration()` écrivait
`[debug_journal]` en dur avec `enabled = false`. Il a reçu le minimum nécessaire,
et rien de plus :

- deux options, `Options.debug_journal` et `Options.debug_journal_sample_rate`,
  exposées par `--debug-journal` et `--debug-journal-sample-rate` ; leurs
  valeurs par défaut sont celles de la campagne d'origine, donc une campagne
  lancée sans option se rejoue à l'identique ;
- une lecture de `GET /v1/debug/queries?limit=1` avec la clé `admin`, faite
  pendant que l'instance tourne et que plus aucun client n'envoie de requête,
  pour relever `enabled` et `dropped` ; c'est le seul endroit où le compteur
  `debug_journal_dropped` est exposé (ADR-0122, décisions 6 et 9) ;
- après l'arrêt du serveur, donc après le vidage d'arrêt, la taille de
  `debug.sqlite` (fichier principal et journal d'écriture anticipée) et le
  nombre de lignes de la table `queries`, lus par le module `sqlite3` de la
  bibliothèque standard ;
- le comptage des lignes `warn` de croissance des deux compteurs de pertes dans
  le journal du serveur, ces lignes ne portant que des nombres (ADR-0122,
  décision 10) ;
- deux lectures `sysctl -n`, `hw.model` et `machdep.cpu.brand_string`, consignées
  dans le bloc `machine` du JSON sous `model` et `cpu`, avec un repli `None`
  quand la clé ou `sysctl` manque : une campagne n'échoue jamais pour une
  identification qu'elle n'a pas pu lire ;
- `[analytics]` reste écrit avec `retention_days = 90`, qui est déjà le défaut ;
  le harnais le nomme maintenant par une constante, pour que le rapport puisse
  citer la valeur qu'il a réellement écrite.

Le réducteur `resume_mes16.py` recopie les deux nouveaux blocs dans le résumé
versionné. Aucun changement de protocole, de critère ni de jeu de requêtes.

Commandes exactes :

```
cargo build --release -p crible
cd tools
uv run --no-sync python -u bench/mes16/mes16_charge.py --bin ../target/release/crible \
    --out ../data/mes16-journal/campaign-1 --campaigns 1 \
    --debug-journal --debug-journal-sample-rate 1.0
uv run --no-sync python -u bench/mes16/mes16_charge.py --bin ../target/release/crible \
    --out ../data/mes16-journal/campaign-2 --campaigns 1 \
    --debug-journal --debug-journal-sample-rate 1.0
uv run --no-sync python -u bench/mes16/mes16_charge.py --bin ../target/release/crible \
    --out ../data/mes16-journal/campaign-3 --campaigns 1 \
    --debug-journal --debug-journal-sample-rate 1.0
```

Les trois campagnes ont été lancées une à une, chacune sur un dossier de sortie
distinct et sur des données neuves, comme le prévoyait le rapport précédent.
Chaque commande a été enveloppée d'un enregistreur qui réaffiche la sortie du
harnais et imprime une ligne d'attente après 25 s sans sortie ; cet enregistreur
ne modifie ni le harnais ni son protocole. Chaque campagne dure 97,2 à 97,6 s,
préparation des données et attache de base comprises.

Deux essais préalables ont servi à vérifier la chaîne. Le premier
(`data/mes16-journal/essai/`, 20 000 passages de base, lots de 200 passages,
échauffement et repos de 10 s) couvre le chemin complet journal → file →
`debug.sqlite` → route : `dropped = 0`, 2 374 lignes pour 2 374 recherches,
fichier de 8,29 Mo. Le second (6 000 passages de base, échauffement et repos de
2 s, dossier temporaire conservé puis supprimé) vérifie que le journal du serveur
est bien écrit et ne porte aucune ligne de croissance des deux compteurs. Leurs
chiffres ne sont pas une campagne et ne sont pas rapportés.

### Résultats

Rappel de lecture : `pool_size` est le nombre de passages vivants vus par la
requête ; il vaut 50 000 au repos et croît de 1 000 passages vivants par attache
(chaque lot de 2 000 passages en ajoute 1 000 et en remplace 1 000), soit
55 000 après les cinq attaches. Toutes les phases sont au-dessus de
`exact_threshold = 5000` : chaque recherche prend le scan binaire suivi du
rescoring, jamais le chemin exact.

**Campagne 1** — limite 465,34 ms (3 × p99 du repos 155,11 ms) — verdict PASS.

| Phase | n | p50 | p95 | p99 | max | pool_size | dense/hybrid | statuts | `took_ms` p50 / p99 | au-delà de la limite |
|---|---|---|---|---|---|---|---|---|---|---|
| repos | 2 984 | 77,9 | 84,1 | 155,1 | 286,4 | 50 000–50 000 | 1 491 / 1 493 | 200 ×2 984 | 76,1 / 153,4 | — |
| attaches | 42 | 87,7 | 96,8 | 163,8 | 163,8 | 50 000–54 000 | 22 / 20 | 200 ×42 | 85,6 / 162,2 | 0 |
| compaction | 88 | 98,3 | 114,7 | 344,0 | 344,0 | 55 000–55 000 | 43 / 45 | 200 ×88 | 95,3 / 341,8 | 0 |

**Campagne 2** — limite 465,24 ms (3 × p99 du repos 155,08 ms) — verdict PASS.

| Phase | n | p50 | p95 | p99 | max | pool_size | dense/hybrid | statuts | `took_ms` p50 / p99 | au-delà de la limite |
|---|---|---|---|---|---|---|---|---|---|---|
| repos | 2 945 | 78,2 | 86,9 | 155,1 | 273,1 | 50 000–50 000 | 1 472 / 1 473 | 200 ×2 945 | 76,3 / 153,5 | — |
| attaches | 43 | 93,9 | 178,8 | 294,3 | 294,3 | 50 000–55 000 | 23 / 20 | 200 ×43 | 91,6 / 290,0 | 0 |
| compaction | 84 | 106,4 | 191,8 | 251,6 | 251,6 | 55 000–55 000 | 41 / 43 | 200 ×84 | 104,7 / 249,3 | 0 |

**Campagne 3** — limite 463,71 ms (3 × p99 du repos 154,57 ms) — verdict PASS.

| Phase | n | p50 | p95 | p99 | max | pool_size | dense/hybrid | statuts | `took_ms` p50 / p99 | au-delà de la limite |
|---|---|---|---|---|---|---|---|---|---|---|
| repos | 2 959 | 78,7 | 84,9 | 154,6 | 294,2 | 50 000–50 000 | 1 479 / 1 480 | 200 ×2 959 | 76,9 / 152,8 | — |
| attaches | 43 | 89,8 | 154,8 | 229,7 | 229,7 | 50 000–55 000 | 21 / 22 | 200 ×43 | 88,1 / 228,5 | 0 |
| compaction | 90 | 107,1 | 118,8 | 198,9 | 198,9 | 55 000–55 000 | 45 / 45 | 200 ×90 | 104,9 / 196,8 | 0 |

Échauffement, p99 par tranche de 10 s (hors critère) :

| Campagne | 0–10 s | 10–20 s | 20–30 s | p99 global |
|---|---|---|---|---|
| 1 | 154,4 (n=487) | 154,6 (489) | 155,1 (501) | 154,6 (n=1 477) |
| 2 | 151,9 (496) | 154,2 (493) | 157,1 (497) | 154,2 (n=1 489) |
| 3 | 152,3 (502) | 153,3 (494) | 149,5 (504) | 152,3 (n=1 502) |

Deux tranches de bascule apparaissent en dehors des 30 s d'échauffement, avec
3 requêtes en campagne 2 et 2 en campagne 3 (p99 80,1 et 77,3 ms) : ce sont les
dernières requêtes de l'échauffement, parties juste avant la bascule vers le
repos. Elles sont rangées dans l'échauffement, hors critère.

Le repos, découpé de la même façon, donne un p99 de 156,0 ; 153,2 ; 155,8 ;
152,0 ; 156,7 ; 153,7 ms en campagne 1, puis une dernière tranche (90–100 s) à
159,6 ms sur 4 requêtes : les requêtes parties juste avant la bascule vers les
attaches. En campagne 2, la première tranche du repos (30–40 s) vaut 207,3 ms,
les suivantes 153,6 à 155,3 ms, et il n'y a pas de tranche 90–100 s. En
campagne 3, la première tranche vaut 87,2 ms, les suivantes 152,7 à 157,4 ms, et
la tranche 90–100 s porte 4 requêtes à 88,0 ms. Les tranches de bascule sont
incluses telles quelles ; elles ne changent pas le p99 de la phase.

Écritures et journal, par campagne :

| Campagne | Attache de base | Cinq attaches | Compaction | `dropped` (route) | Lignes `queries` | `debug.sqlite` | WAL | `warn` analytics / journal |
|---|---|---|---|---|---|---|---|---|
| 1 | 1 625,5 ms (gén. 1) | 204,3 à 207,7 ms (gén. 2 à 6) | 2 247,9 ms (gén. 7) | 0 | 4 591 | 16 232 448 o (15,48 Mio) | 0 o | 0 / 0 |
| 2 | 1 835,6 ms (gén. 1) | 205,7 à 207,7 ms (gén. 2 à 6) | 2 439,0 ms (gén. 7) | 0 | 4 561 | 16 031 744 o (15,29 Mio) | 0 o | 0 / 0 |
| 3 | 1 831,4 ms (gén. 1) | 204,6 à 207,5 ms (gén. 2 à 6) | 2 441,6 ms (gén. 7) | 0 | 4 594 | 15 994 880 o (15,25 Mio) | 0 o | 0 / 0 |

Les six écritures mesurées de chaque campagne (cinq attaches et la compaction)
rendent `202` puis `succeeded`, sans `Retry-After` ni `503` ; l'attache de base,
qui est le décor de la campagne et non une écriture mesurée
(`criterion.writes.n = 6`), rend elle aussi `succeeded` et publie la
génération 1.

Le nombre de lignes écrites égale exactement le nombre de recherches de chaque
campagne : 1 477 + 2 984 + 42 + 88 = 4 591 ; 1 489 + 2 945 + 43 + 84 = 4 561 ;
1 502 + 2 959 + 43 + 90 = 4 594. Aucune recherche n'est donc restée sans ligne,
et aucune ligne n'a été écrite sans recherche correspondante. Le fichier pèse
3,48 à 3,54 Ko par ligne sur ces campagnes.

`analytics_dropped` **n'est pas lisible par l'API** : la route de statistiques
(`GET /v1/stats`) relève de ROUTE-STATS, qui n'est pas faite, et `docs/06` ne
donne aucun autre point de lecture du compteur. La seule observation
disponible est le journal du serveur : aucune ligne `warn` « analytics_dropped
grew » n'y figure dans les trois campagnes (comptage de lignes, 0 sur les trois),
ce qui est cohérent avec 0 perte au sens du compteur d'ADR-0118, décision 2,
puisqu'une croissance est journalisée à chaque vidage où elle se produit, y
compris au vidage d'arrêt. La valeur elle-même n'est pas relevée.

L'écrivain des analytics n'a laissé aucune trace de fichier après coup, et je
ne la fabrique pas. `analytics.sqlite` vit dans le dossier de données du
serveur, c'est-à-dire dans le dossier temporaire de la campagne, que le harnais
supprime à la fin de chaque campagne (`shutil.rmtree`, `--keep` non passé). Les
dossiers conservés sous `data/mes16-journal/` ne contiennent que le JSON brut et
son résumé : ni `analytics.sqlite`, ni `debug.sqlite`, dont la taille et les
lignes ont été relevées par le harnais avant la suppression. Il n'est donc pas
possible de citer ici la taille du fichier d'analytics ni son nombre de lignes,
et une prochaine campagne devrait les relever comme celles du journal
(`debug_journal.file` dans le JSON). Ce qui établit l'écrivain actif est
structurel, et non l'observation d'un fichier : `[analytics]` n'a pas
d'interrupteur (`crates/crible-server/src/config.rs:181-186` ne porte que
`retention_days`), `crates/crible-server/src/state.rs:969-973` démarre
`Analytics::start` sans condition avant que l'instance réponde, et chaque
recherche dépose un événement (`crates/crible-server/src/search.rs:154` et
`crates/crible-server/src/search_multi.rs:117` appellent `analytics::record`).
L'absence de ligne `warn` de croissance du compteur va dans le même sens.
L'existence du fichier, elle, n'est pas constatée.

### Contrôles

1. **Référence au-dessus du seuil exact et même `pool_size` que les écritures.**
   Le repos voit 50 000 passages, donc au-dessus de `exact_threshold = 5000` ;
   les attaches passent de 50 000 à 55 000 et la compaction reste à 55 000. Les
   trois phases prennent le même chemin de scan (binaire puis rescoring) : la
   référence ne mesure pas un chemin plus rapide que les phases d'écriture.
2. **Le p99 du repos ne dépasse celui d'aucune phase d'écriture.** Il vaut
   155,1 / 155,1 / 154,6 ms, contre 163,8 / 294,3 / 229,7 ms pour les attaches et
   344,0 / 251,6 / 198,9 ms pour les compactions. Aucune référence n'est donc
   gonflée par un artefact de cache ou de stratégie de scan. Une réserve
   factuelle : le maximum du repos dépasse le p99 des attaches et des
   compactions de plusieurs campagnes (286,4 ; 273,1 ; 294,2 ms), ce qui situe la
   queue haute dans la machine entière, pas dans les écritures.
3. **Attente contre calcul.** L'écart entre l'horloge murale du client et le
   `took_ms` du serveur est de 1,66 à 2,31 ms au p50 et de 3,72 à 6,21 ms au p99
   des trois phases mesurées, avec un maximum de 7,13 ms sur toute la campagne ;
   il reste du même ordre, ou plus bas, que les 1,7 ms au p50, 3,5 à 4,1 ms au
   p99 et 10,5 ms de maximum du rapport précédent. La latence mesurée est donc du
   calcul de recherche, non une attente dans une file HTTP, et l'écriture du
   journal ne s'y voit pas.
4. **Débit du repos.** Le repos enregistre 2 984, 2 945 et 2 959 recherches en
   60 s, soit 49,7 ; 49,1 et 49,3 recherches par seconde, contre 2 930, 2 725 et
   2 827 (48,8 ; 45,4 ; 47,1 par seconde) dans le rapport précédent. Le journal à
   `sample_rate = 1.0` ne montre ici aucune perte de débit ni de décalage du p50
   (77,9 ; 78,2 ; 78,7 ms contre 77,8 ; 81,6 ; 80,4 ms). Ce n'est pas une preuve
   d'absence de coût : les deux séries ne sont pas appariées et la charge tierce
   de la machine diffère.

### Comparaison avec les trois campagnes du 2026-10-30

| Série | Campagne | p99 repos | Limite (3 ×) | Max attaches | Max compaction | `5xx` | Pertes |
|---|---|---|---|---|---|---|---|
| sans journal (P6) | 1 | 155,47 | 466,41 | 236,9 | 270,7 | 0 | non mesurées |
| sans journal (P6) | 2 | 180,18 | 540,55 | 186,6 | 292,9 | 0 | non mesurées |
| sans journal (P6) | 3 | 161,77 | 485,31 | 294,4 | 210,0 | 0 | non mesurées |
| journal à 1.0 (P7) | 1 | 155,11 | 465,34 | 163,8 | 344,0 | 0 | 0 |
| journal à 1.0 (P7) | 2 | 155,08 | 465,24 | 294,3 | 251,6 | 0 | 0 |
| journal à 1.0 (P7) | 3 | 154,57 | 463,71 | 229,7 | 198,9 | 0 | 0 |

Les deux séries tiennent le critère ; le rejeu a un p99 de repos nettement plus
stable (154,6 à 155,1 ms contre 155,5 à 180,2 ms). Le pire cas du rejeu est une
requête de compaction à 344,0 ms (campagne 1), soit 0,74 de la limite : il
dépasse les maxima des trois campagnes précédentes (270,7 ; 292,9 ; 210,0 ms).
Les campagnes ne sont pas appariées et la charge tierce diffère (3,7 à 6,6 ici,
3,2 à 6,0 alors) : ce dépassement n'est pas attribuable au journal sans une
comparaison contrôlée.

### Verdicts

- **Invariant INV-02, critère d'ADR-0113 décision 1 (bloquant)** : tenu sur les
  trois campagnes. Sur 42, 43 et 43 recherches d'attaches et 88, 84 et
  90 recherches de compaction, aucune n'atteint la limite de sa campagne ; le
  pire cas est 344,0 ms contre 465,34 ms (campagne 1), soit 0,74 de la limite.
- **Critère de la fiche (p99 de la phase d'écriture sous 3 × p99 du repos)** :
  tenu aussi. Le p99 des attaches vaut 0,35 / 0,63 / 0,50 de la limite et celui
  des compactions 0,74 / 0,54 / 0,43.
- **Codes d'état (décision 6)** : 3 114, 3 072 et 3 092 recherches mesurées,
  toutes en `200` ; les six écritures mesurées par campagne sont toutes en
  `202`, toutes `succeeded`, l'attache de base restant hors du décompte, comme
  décor. Aucun `5xx`, aucun `503` de recherche ni d'écriture, aucun échec de
  transport.
- **Pertes** : `debug_journal_dropped` vaut 0 sur les trois campagnes, lu par
  `GET /v1/debug/queries` avant l'arrêt, et aucune ligne `warn` de croissance
  n'apparaît dans le journal du serveur. Aucune ligne n'est perdue au vidage non
  plus : les lignes de `debug.sqlite` égalent exactement les recherches. Pour
  les analytics, le compteur n'a pas de lecture HTTP (`ROUTE-STATS` non faite) ;
  aucune ligne `warn` de croissance n'apparaît non plus.
- **INV-17** : la preuve déterministe reste le test `inv_17_*` sous `failpoints`
  (ADR-0122, décision 11), pas cette campagne. La mesure apporte une observation
  complémentaire : sous la charge la plus lourde d'écriture que la configuration
  permet, l'écart client-serveur reste du même ordre, p50 de 1,66 à 2,31 ms et
  p99 de 3,72 à 6,21 ms, contre 1,7 ms et 3,5 à 4,1 ms en P6, et aucune écriture
  ne retarde visiblement une recherche.

### Réserves

- **Étiquette du rapport de P6** (constat, non réserve de cette mesure) :
  `docs/mesures/MES-16-charge-concurrente.md:12` nomme la machine « Mac mini
  `Mac15,12` (Apple M4) ». Le dépôt dit autre chose : `Mac15,12` est un MacBook
  Air M3, `machdep.cpu.brand_string` rend `Apple M3`, et MES-01 à MES-03 nomment
  déjà ce modèle « MacBook Air M3 (`Mac15,12`) ». Aucune mesure n'a jamais lu
  `Apple M4` sur cette machine. La coquille est dans le rapport de P6, qui est
  commité : sa correction relève du propriétaire, et ce rapport-ci ne le modifie
  pas.
- **Charge tierce** : `mediaanalysisd` occupait 77 à 88 % d'un cœur pendant les
  trois campagnes, et la moyenne de charge allait de 3,74 à 6,58 sur 8 cœurs.
  Rien ici ne décrit une machine au repos, et aucune latence absolue n'est
  transposable.
- **Phases d'écriture courtes** : environ 1,0 s pour les cinq attaches et 2,2 à
  2,4 s pour la compaction. Le critère par requête y porte sur 42 à 90
  recherches, ce qui laisse peu de place à un incident rare ; un dépassement
  isolé resterait possible sans que trois campagnes le montrent.
- **Tailles de `debug.sqlite`** : 15,25 à 15,48 Mio pour environ 4 600 lignes.
  C'est le pire cas de configuration (`sample_rate = 1.0`) sur un jeu
  synthétique de 50 000 passages ; la taille dépend du texte et des filtres
  réels, et rien ici ne dit ce qu'elle vaudrait sur Legifacile.
- **`analytics_dropped` non lu** : l'absence de ligne `warn` est une observation
  indirecte. Une perte survenue après le dernier vidage périodique et non
  rattrapée par le vidage d'arrêt ne serait pas comptée ; la valeur du compteur
  demande ROUTE-STATS.
- Le jeu reste synthétique (50 000 passages, textes de six mots) : rien ne dit
  ce que donneraient 8,58 M passages réels, ni un texte long, qui pèsent
  directement sur la taille d'une ligne du journal.

### Données

- Résumé versionné : `docs/mesures/MES-16-charge-concurrente-analytics-et-journal.2026-11-01.json`
  (34 Ko, statistiques par campagne et par phase, sans échantillons), produit par
  `uv run --project tools --no-sync python tools/bench/mes16/resume_mes16.py --raw data/mes16-journal/campaign-1 data/mes16-journal/campaign-2 data/mes16-journal/campaign-3 --out docs/mesures/MES-16-charge-concurrente-analytics-et-journal.2026-11-01.json`.
  Le réducteur importe le percentile d'ADR-0077 et le découpage en tranches de
  `mes16_charge.py`, sans les réécrire.
- Chiffres bruts, gitignorés : `data/mes16-journal/campaign-1/campaign-1.json`,
  `data/mes16-journal/campaign-2/campaign-1.json`,
  `data/mes16-journal/campaign-3/campaign-1.json` (environ 860 Ko chacun, tous
  les échantillons et leur horodatage), avec le `summary.json` de chaque
  lancement. L'essai est sous `data/mes16-journal/essai/`.
- Tests du harnais et du réducteur : `cd tools && uv run --no-sync pytest bench/mes16`
  donne 48 tests verts, dont ceux des nouvelles options, du refus d'un
  `sample_rate` hors de `[0, 1]`, de la lecture de `debug.sqlite`, du comptage
  des lignes `warn`, de la lecture HTTP de `enabled` et `dropped`, et de la
  recopie (ou de l'absence) des blocs `journal` et `analytics` par le réducteur.
  La suite `bench` entière donne 333 tests verts.

### Conclusion

Sur cette machine, avec ce binaire, le critère d'ADR-0113 est tenu sur les trois
campagnes avec `[analytics]` à ses valeurs par défaut et `[debug_journal]` activé
au pire cas (`sample_rate = 1.0`) : aucune recherche des phases d'écriture
n'atteint 3 × le p99 du repos, le p99 de chaque phase d'écriture reste lui aussi
sous ce seuil, aucune réponse `5xx` n'est apparue, et le journal n'a perdu aucune
ligne (`dropped = 0`, lignes de `debug.sqlite` égales aux recherches). Le pire
cas observé est une requête de compaction à 344,0 ms, sous une limite de
465,34 ms.

Ce que la mesure ne permet pas d'affirmer : rien sur une machine au repos, car
`mediaanalysisd` et `mds_stores` chargeaient la machine pendant toute la
campagne ; rien sur les 8,58 M passages de Legifacile, le jeu étant de
50 000 passages synthétiques ; rien sur `analytics_dropped`, dont le compteur
n'a pas de lecture HTTP avant ROUTE-STATS, ni sur l'existence du fichier
d'`analytics.sqlite`, non relevée ; et rien sur l'attribution du pic de
compaction au journal, faute de comparaison appariée avec les campagnes sans
journal.

Deux propositions, à valider par le propriétaire : rejouer une campagne appariée
sur la même machine en alternant journal actif et journal inactif, pour que
l'écart entre les deux séries soit attribuable au journal ; et relever la taille
de `debug.sqlite` sur un échantillon Legifacile réel, puisque c'est le texte et
le filtre qui déterminent le poids d'une ligne. Aucune ne change le critère.
