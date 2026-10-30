# MES-16 — Charge concurrente pendant les écritures

Question : les recherches concurrentes d'un index restent-elles sous le critère
d'INV-02 pendant des attaches puis une compaction ? Protocole fixé par
ADR-0113 ; mesure créée par la même décision (décision 4).

## Campagne du 2026-10-30

- **Tâche** : `TESTER-CHARGE-CONCURRENTE` (phase P6).
- **Commit** : `d0138bd` (`d0138bdbec6b7776148baf0736db6dc20bdc70e1`) plus le travail non commité de la tâche dans l'arbre, sur la branche `tache/TESTER-CHARGE-CONCURRENTE`. Le binaire mesuré est construit depuis cet arbre ; son empreinte SHA-256 est
  `1deac2ad491bd5fccb816286e16c216642b618f68689bd548436890f40006c89`.
- **Machine** : Mac mini `Mac15,12` (Apple M4), 8 cœurs, 24 Gio de mémoire,
  macOS 15.7.7, exécution native, build `--release`
  (`cargo build --release -p crible`). La mesure a tourné sur la machine de
  développement, pas en conteneur.
- **Charge de la machine au départ** : `uptime` à 18:29 donne une moyenne de
  charge de 3,19 sur 8 cœurs, puis 4,05 avant la première campagne, 5,43 avant
  la deuxième et 6,02 avant la troisième. Les processus tiers les plus actifs
  relevés par `ps -Ao pcpu,comm -r` entre deux campagnes sont `WindowServer`
  (34 %), `opencode` (26 %), `ControlCenter` (16 %), `coreaudiod` (13 %) et
  `Code Helper` (11 %). Aucun de ces processus n'a été fermé : la mesure se
  compare donc à elle-même sur la même machine chargée, elle ne donne pas une
  latence absolue transposable.
- **Jeu de données** : synthétique, produit par le harnais avec la graine
  `20261030` (`tools/bench/mes16/mes16_charge.py`, `SEED`), 50 000 passages de
  base puis cinq lots de 2 000 passages, modèle `external` à 1 024 dimensions,
  branche lexicale activée. Les vecteurs sont tirés selon une loi normale
  normalisée en fp32, les textes depuis un vocabulaire de seize mots. Le harnais
  supprime le dossier temporaire de chaque campagne : il n'existe donc pas de
  somme de contrôle du jeu à citer, seuls l'identifiant du lot de base
  (`mes16-base`), ceux des cinq lots de mise à jour et les générations sont
  relevés. Le jeu se reproduit par la graine et les paramètres du protocole.
- **Configuration** : `reference_precision = fp16`, `search.scan = binary`,
  `rescore_candidates = 5000`, `exact_threshold = 5000`,
  `candidates_per_branch = 100`, `fusion = rrf`, `lexical.enabled = true` avec
  `language = fr` et `positions = false`, `compaction.dead_ratio = 0.10`,
  `search_threads = 0` (un thread par cœur disponible) et
  `max_concurrent_searches = 0` (valeur par défaut du serveur). `k = 10`,
  `N = 4` clients, aucun encodeur chargé.

### Protocole

Le protocole est celui d'ADR-0113, décision 5, sans écart : échauffement de
30 s écarté des résultats, repos de 60 s servant de référence, puis cinq
attaches et une compaction ; latence à l'horloge murale côté client, `took_ms`
du serveur rapporté à côté ; percentiles au rang le plus proche (ADR-0077). Les
attaches sont soumises une à une et attendues jusqu'à leur état terminal
(`client.wait`, sondage toutes les 0,2 s). Le critère est celui de la décision 1
(pendant chaque phase d'écriture, **chaque** recherche se termine en moins de
3 × le p99 du repos) ; le critère de la fiche (p99 de la phase d'écriture sous
3 × le p99 du repos) est rapporté aussi. Les codes d'état suivent la décision 6.

La consigne prévoyait une commande unique ; elle aurait tourné environ cinq
minutes sans rendre la main au terminal. Les trois campagnes ont donc été
lancées une à une, chacune sur un dossier de sortie distinct et sur des données
neuves :

```
cargo build --release -p crible
cd tools
uv run --no-sync python -u bench/mes16/mes16_charge.py --bin ../target/release/crible --out ../data/mes16/campaign-1 --campaigns 1
uv run --no-sync python -u bench/mes16/mes16_charge.py --bin ../target/release/crible --out ../data/mes16/campaign-2 --campaigns 1
uv run --no-sync python -u bench/mes16/mes16_charge.py --bin ../target/release/crible --out ../data/mes16/campaign-3 --campaigns 1
```

Chaque campagne dure 97,8 à 98,4 s, préparation des données et attache de base
comprises. Aucun `503` n'est attendu sur les écritures dans ce protocole ; aucun
n'a été observé.

Un essai préalable de 5 000 passages (`data/mes16/essai/`) a servi à vérifier
que l'outil tournait ; ses chiffres ne sont pas une campagne et ne sont pas
rapportés, sa taille le plaçant à la frontière de `exact_threshold` (repos à
5 000 passages en chemin exact, écritures à 5 100 en chemin binaire).

### Résultats

Rappel de lecture : `pool_size` est le nombre de passages vivants vus par la
requête ; il vaut 50 000 au repos et croît de 1 000 passages vivants par attache
(chaque lot de 2 000 passages en ajoute 1 000 et en remplace 1 000), soit
55 000 après les cinq attaches. Toutes les phases sont au-dessus de
`exact_threshold = 5000` : chaque recherche prend le scan binaire suivi du
rescoring, jamais le chemin exact.

**Campagne 1** — limite 466,41 ms (3 × p99 du repos 155,47 ms) — verdict PASS.

| Phase | n | p50 | p95 | p99 | max | pool_size | dense/hybrid | statuts | `took_ms` p50 / p99 | au-delà de la limite |
|---|---|---|---|---|---|---|---|---|---|---|
| repos | 2 930 | 77,8 | 138,0 | 155,5 | 279,5 | 50 000–50 000 | 1 465 / 1 465 | 200 ×2 930 | 76,0 / 153,9 | — |
| attaches | 42 | 89,4 | 99,6 | 236,9 | 236,9 | 50 000–55 000 | 21 / 21 | 200 ×42 | 86,6 / 235,1 | 0 |
| compaction | 133 | 99,6 | 174,0 | 224,2 | 270,7 | 55 000–55 000 | 67 / 66 | 200 ×133 | 96,8 / 222,2 | 0 |

**Campagne 2** — limite 540,55 ms (3 × p99 du repos 180,18 ms) — verdict PASS.

| Phase | n | p50 | p95 | p99 | max | pool_size | dense/hybrid | statuts | `took_ms` p50 / p99 | au-delà de la limite |
|---|---|---|---|---|---|---|---|---|---|---|
| repos | 2 725 | 81,6 | 146,1 | 180,2 | 268,5 | 50 000–50 000 | 1 364 / 1 361 | 200 ×2 725 | 79,7 / 178,3 | — |
| attaches | 39 | 95,7 | 178,8 | 186,6 | 186,6 | 50 000–54 000 | 20 / 19 | 200 ×39 | 93,0 / 183,7 | 0 |
| compaction | 85 | 108,8 | 212,3 | 292,9 | 292,9 | 55 000–55 000 | 41 / 44 | 200 ×85 | 106,4 / 289,8 | 0 |

**Campagne 3** — limite 485,31 ms (3 × p99 du repos 161,77 ms) — verdict PASS.

| Phase | n | p50 | p95 | p99 | max | pool_size | dense/hybrid | statuts | `took_ms` p50 / p99 | au-delà de la limite |
|---|---|---|---|---|---|---|---|---|---|---|
| repos | 2 827 | 80,4 | 123,7 | 161,8 | 335,7 | 50 000–50 000 | 1 414 / 1 413 | 200 ×2 827 | 78,6 / 160,0 | — |
| attaches | 41 | 94,7 | 181,2 | 294,4 | 294,4 | 50 000–55 000 | 21 / 20 | 200 ×41 | 92,1 / 289,5 | 0 |
| compaction | 91 | 109,7 | 122,9 | 210,0 | 210,0 | 55 000–55 000 | 45 / 46 | 200 ×91 | 107,7 / 206,2 | 0 |

Échauffement, p99 par tranche de 10 s (hors critère) :

| Campagne | 0–10 s | 10–20 s | 20–30 s | p99 global |
|---|---|---|---|---|
| 1 | 153,4 (n=501) | 183,3 (n=485) | 184,5 (n=458) | 164,6 (n=1 444) |
| 2 | 208,4 (n=463) | 196,0 (n=466) | 178,9 (n=463) | 195,9 (n=1 396) |
| 3 | 161,9 (n=451) | 352,2 (n=409) | 160,2 (n=485) | 229,5 (n=1 347) |

Le repos, découpé de la même façon, montre le même régime : p99 de 160,2 puis
153,8 à 159,2 ms en campagne 1 ; 159,2 à 191,9 ms en campagne 2, avec une tranche
à 222,4 ms (40–50 s) ; 156,2 à 177,5 ms en campagne 3, avec une tranche à
259,6 ms (40–50 s). Une dernière tranche (90–100 s) apparaît dans les trois
campagnes avec 4, 4 et 2 requêtes (`slices.n`) : ce sont les requêtes parties
juste avant la bascule vers les attaches, comptées dans le repos. Elles sont
incluses telles quelles, et elles ne changent pas le p99 de la phase, identique
sans elles (155,469 / 180,185 / 161,771 ms).

Écritures : attache de base 1 818,7 / 1 828,4 / 1 823,1 ms, génération 1 dans les
trois campagnes ; cinq attaches de 207,1 à 202,6 ms, générations 2 à 6 ;
compaction 3 459,6 / 2 642,7 / 2 641,4 ms, génération 7. Les six écritures
mesurées de chaque campagne (cinq attaches et la compaction) rendent `202` puis
`succeeded`, sans `Retry-After` ni `503` ; l'attache de base, qui est le décor de
la campagne et non une écriture mesurée (`criterion.writes.n = 6`), rend elle
aussi `succeeded` et publie la génération 1.

### Verdicts

- **Invariant INV-02, critère d'ADR-0113 décision 1 (bloquant)** : tenu sur les
  trois campagnes. Sur 42, 39 et 41 recherches d'attaches et 133, 85 et
  91 recherches de compaction, aucune n'atteint la limite de sa campagne ; le
  pire cas est 294,4 ms contre 485,31 ms (campagne 3), soit 0,61 de la limite.
- **Critère de la fiche (p99 de la phase d'écriture sous 3 × p99 du repos)** :
  tenu aussi. Le p99 des attaches vaut 0,51 / 0,35 / 0,61 de la limite et celui
  des compactions 0,48 / 0,54 / 0,43.
- **Codes d'état (décision 6)** : 3 105, 2 849 et 2 959 recherches mesurées,
  toutes en `200` ; les six écritures mesurées par campagne (cinq attaches et la
  compaction) sont toutes en `202`, toutes `succeeded`, l'attache de base restant
  hors du décompte, comme décor. Aucun `5xx`, aucun `503` de recherche ni d'écriture,
  aucun échec de transport.

### Contrôles demandés par la relecture

1. **Référence au-dessus du seuil exact et même `pool_size` que les écritures.**
   Le repos voit 50 000 passages, donc au-dessus de `exact_threshold = 5000` ;
   les attaches passent de 50 000 à 55 000 et la compaction reste à 55 000. Les
   trois phases prennent le même chemin de scan (binaire puis rescoring) : la
   référence ne mesure pas un chemin plus rapide que les phases d'écriture. La
   différence de `pool_size` entre le repos et les écritures est exactement
   celle des cinq lots attachés, soit 1 000 passages vivants de plus par attache
   (1 000 ajoutés, 1 000 remplacés).
2. **Le p99 du repos ne dépasse celui d'aucune phase d'écriture.** Il vaut
   155,5 / 180,2 / 161,8 ms, contre 236,9 / 186,6 / 294,4 ms pour les attaches et
   224,2 / 292,9 / 210,0 ms pour les compactions. Aucune référence n'est donc
   gonflée par un artefact de cache ou de stratégie de scan. Deux réserves
   factuelles : la campagne 2 a une attache à 186,6 ms contre 180,2 ms au repos,
   une marge de 3,6 % seulement, et le maximum du repos (335,7 ms en campagne 3)
   dépasse le p99 de plusieurs phases d'écriture : la queue haute appartient à
   la machine entière, pas aux écritures.
3. **Convergence de l'échauffement.** Le p50 est stable dès la première tranche
   dans les trois campagnes (77,3 ; 79,1 ; 82,2 ms) et reste entre 77,3 et
   82,8 ms hors tranche de bascule. Les tranches de bascule du repos (90–100 s)
   sont plus hautes, à 86,5 ; 89,1 et 92,6 ms de p50, sur 4, 4 et 2 requêtes
   seulement : ce sont les dernières requêtes du repos, parties juste avant la
   bascule. Le p99
   des tranches oscille entre 153 et 208 ms, sauf la tranche 10–20 s de la
   campagne 3 à 352,2 ms : cinq requêtes entre 12,86 s et 15,32 s y suffisent
   (468,8 ; 399,2 ; 395,8 ; 355,1 ; 352,2 ms), puis la tranche suivante revient à
   160,2 ms. Le repos connaît des bouffées comparables (p99 259,6 ms sur une
   tranche), attribuables à la charge tierce de la machine documentée plus haut :
   la charge des clients a convergé, la queue ne converge pas parce qu'elle est
   du bruit de machine, pas un transitoire de démarrage.
4. **Attente contre calcul.** L'écart entre l'horloge murale du client et le
   `took_ms` du serveur est de 1,7 ms au p50 et de 3,5 à 4,1 ms au p99 du repos,
   avec un maximum de 10,5 ms ; il reste du même ordre pendant les écritures
   (p50 1,6 à 2,1 ms, maximum 9,5 ms). La latence mesurée est donc du calcul de
   recherche, non une attente dans une file HTTP : les écritures n'ont pas
   retardé la prise en charge des requêtes.

### Données

- Résumé versionné : `docs/mesures/MES-16-charge-concurrente.2026-10-30.json`
  (32 Ko, statistiques par campagne et par phase, sans échantillons), produit par
  `uv run --project tools --no-sync python tools/bench/mes16/resume_mes16.py --raw data/mes16/campaign-1 data/mes16/campaign-2 data/mes16/campaign-3 --out docs/mesures/MES-16-charge-concurrente.2026-10-30.json`.
  Le réducteur importe le percentile d'ADR-0077 et le découpage en tranches de
  `mes16_charge.py`, sans les réécrire ; ses tests sont dans
  `tools/bench/mes16/tests/test_resume_mes16.py` et tournent avec
  `cd tools && uv run --no-sync pytest bench/mes16` (37 tests, harnais et
  réducteur compris).
- Chiffres bruts, gitignorés : `data/mes16/campaign-1/campaign-1.json`,
  `data/mes16/campaign-2/campaign-1.json`,
  `data/mes16/campaign-3/campaign-1.json` (environ 800 Ko chacun, tous les
  échantillons et leur horodatage). L'essai est sous `data/mes16/essai/`.

### Conclusion

Sur cette machine, avec ce binaire, INV-02 est tenu au sens d'ADR-0113 sur les
trois campagnes : aucune recherche des phases d'écriture n'atteint 3 × le p99 du
repos, et le p99 de chaque phase d'écriture reste lui aussi sous ce seuil. Le
pire cas observé est une attache de la campagne 3 à 294,4 ms, sous une limite de
485,31 ms.

Ce que la mesure ne permet pas d'affirmer : rien sur une machine au repos, car
la charge tierce était de 3 à 6 sur 8 cœurs pendant toute la campagne ; rien sur
les 8,58 M passages de Legifacile, le jeu étant de 50 000 passages synthétiques ;
rien sur INV-17, hors de P6 (décision 2). Les phases d'écriture sont courtes
(environ 1,0 s pour les cinq attaches, 2,6 à 3,5 s pour la compaction) : le
critère par requête y porte sur 39 à 133 recherches seulement, ce qui laisse
peu de place à un incident rare. La route `GET /v1/indexes/{index}` signalée au
suivi n'est pas appelée par le harnais, qui sonde l'état des jobs toutes les
0,2 s : son coût n'est pas relevé ici.

Deux propositions, à valider par le propriétaire : rejouer une campagne sur une
machine sans charge tierce pour pouvoir citer une latence absolue, et allonger
la phase d'attache (plusieurs lots ou lots plus gros) pour que le critère par
requête porte sur un nombre de recherches comparable à celui du repos. Aucune ne
change le critère.
