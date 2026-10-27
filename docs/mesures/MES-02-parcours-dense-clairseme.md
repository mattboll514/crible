# MES-02 — Parcours dense et clairsemé du scan binaire : passes alternées et densités

## Campagne du 2026-10-27

- **Demande** : mesure hors plan, demandée par le propriétaire le 2026-10-27 pour instruire le seuil
  d'**ADR-0071** et la surveillance d'**ADR-0077** (section « À surveiller »). Ce n'est pas une tâche
  du plan ; `docs/suivi.md` n'est pas modifié **par cette campagne** (il l'est dans le même arbre,
  par l'orchestrateur).
- **Commit** : `da34f7020d715d18dcd8e484e4157fbfd0aa8da1`, branche `phase/P4-recherche-rapide`.
  **Aucun code de production n'a été modifié** : rien dans `crates/*/src/`. Sont **non commités** au
  moment de la mesure, et livrés avec ce rapport :
  - l'extension du harnais `crates/crible/examples/dense_latency.rs` (modes `cross` et `agree`,
    générateur `Draw`, constructeur `pool_window`) ;
  - `tools/bench/mes02/mes02_parcours_analyze.py` (nouveau) et
    `tools/bench/mes02/tests/test_parcours_analyze.py` (nouveau) ;
  - `tools/bench/mes02/run-parcours.sh` (nouveau) ;
  - ce rapport et son artefact.

  Le générateur pseudo-aléatoire de `pool_dispersed` a été extrait dans `Draw` sans changer un seul
  tirage : les tailles, premiers et derniers iid des pools du mode `cross` ont été vérifiés
  identiques avant et après l'extraction.
- **Machine** : MacBook Air M3, 8 cœurs, 24 Go, macOS 15.7.7 (24G720), APFS, **natif**, pas de
  conteneur. Charge `3.91 2.88 2.79` au départ et `4.41 4.67 4.25` à la fin
  (`data/mes02/raw/conditions-parcours-start.txt`, `…-end.txt`). Lightroom (20 à 31 % d'un cœur),
  `opencode`, Arc, `WindowServer` et `coreaudiod` tournaient : la machine n'était **pas** au repos,
  et la charge a **monté** pendant la campagne.
- **Jeu de données** : copie de mesure `data/mes02/indexes/synth-large` (17 Go, gitignorée, déjà
  construite par la campagne MES-02 du même jour), **7 740 000 passages vivants** sur 8 600 000,
  768 dimensions, un seul segment `seg-large-0001`. `data/synth-large/` n'a pas été touché.
- **Requêtes** : les 300 vecteurs de `data/mes02/queries.f32` de MES-02 (graine 20261021, flux 7,
  loi normale normalisée), SHA-256 de la matrice
  `03cb2a2b3bb7951892179ee8d612b0970626f0f041fe6b3e18c2f0393452948b`.
- **Configuration** : copie de mesure avec `search.exact_threshold = 0` et `search.scan = "binary"`,
  donc **BINARY sur chaque point** ; `candidates_per_branch = 100` (k) ; 8 fils de scan. Le harnais ne
  mesure **que le scan**, sans rescoring ni payload, comme MES-02. Mesure à **N = 5000** (défaut
  d'ADR-0078) et, sur les deux pools de MES-02 seulement, à **N = 1000** pour la continuité.

### Protocole

Le protocole est celui d'**ADR-0077**, avec l'écart de sa décision 4 corrigé : **une passe
d'échauffement séparée précède les passes mesurées**. Chaque point exécute ensuite **3 paires de
passes alternées** : dense, clairsemé, dense, clairsemé, dense, clairsemé, toujours sur les mêmes
300 requêtes, dans l'ordre des requêtes de MES-02. Les passes d'échauffement sont tenues à part et
**exclues** des séries mesurées ; l'artefact n'en garde que les **résumés** (clé `warmup`), les durées
par requête restant dans les documents bruts gitignorés. Aucune valeur mesurée n'est écartée.

Les deux parcours sont **forcés** : le harnais ne laisse pas ADR-0071 choisir, sinon chaque pool
n'aurait qu'un parcours et il n'y aurait rien à comparer.

**L'ordre dans une paire est fixe : dense, puis clairsemé.** Il ne s'inverse pas d'une paire à
l'autre. La dérive de charge de la machine ne frappe donc **pas** les deux parcours également : le
clairsemé passe toujours en second, à un moment où la charge est un peu plus haute, ce qui le
**pénalise**. L'ordre de grandeur se lit sur l'échauffement : les passes d'échauffement, mesurées
juste avant les paires, sont plus rapides que les passes mesurées de 1,3 % en médiane pour un
intervalle d'environ 3,5 passes, soit **≈ 0,4 % par passe**. Le biais est donc d'environ 0,4 % **en
faveur du dense**, et il est **conservateur** : les écarts du tableau sont, au pire, sous-estimés de
0,4 point de pourcentage. Il ne peut renverser qu'un seul point, le match nul de 1 M contigu à
N = 1000 (−0,006 %), et il y pousserait le clairsemé devant — dans le sens de la conclusion, jamais
contre elle.

Les pools sont construits par le harnais, avec la graine 2026 :

- **`full`** : le bitmap des vivants entier (7 740 000 passages, plage de 8 600 000 iid).
- **`contiguous`** : une fenêtre de passages vivants consécutifs de la liste des vivants, à partir du
  tiers de cette liste — le `pool_contiguous` de MES-02, repris tel quel.
- **`density`** : `fraction` des identifiants d'une fenêtre de `span` identifiants, tirés
  uniformément parmi les passages vivants de la fenêtre, jusqu'à obtenir la taille visée. La densité
  rapportée est celle d'ADR-0071, `taille du pool / plage d'iid`, calculée sur les iid réels du pool ;
  elle vaut donc exactement la fraction demandée à 10⁻³ près.

Deux limites de construction, déclarées :

- **100 % de densité n'est pas constructible sur `synth-large`.** Le jeu porte 90 % de vivants : aucune
  fenêtre n'a tous ses iid vivants. La série monte donc à **90 %**, et c'est aussi la densité réelle
  des deux pools « contigus » de MES-02 (7 740 000 / 8 600 000 = 0,90 pour le pool complet,
  1 000 000 / 1 111 111 = 0,90 pour 1 M contigu). La campagne mesure donc la bande 10 – 90 %, et
  **la borne 100 % n'est pas mesurée** : aucun pool de `synth-large` ne peut l'atteindre. Les deux
  pools contigus de MES-02 sont à 0,90, pas à 1.
- **Deux tailles de plage** : 1 000 000 et 7 740 000 iid. Ce sont les deux échelles du budget
  (`docs/03-recherche.md:143`) et de la surveillance d'ADR-0077.

Commandes exactes, depuis la racine du dépôt (recette complète : `tools/bench/mes02/run-parcours.sh`) :

```
cargo build --release --example dense_latency
H=./target/release/examples/dense_latency
I="--data data/mes02 --index synth-large --vectors data/mes02/queries.f32"

$H cross $I --ns 5000 --pairs 3 --sizes 1000000,all \
    --densities 10,25,40,50,60,75,90 --spans 1000000,7740000 \
    --out data/mes02/raw/cross-n5000.json

$H cross $I --ns 1000 --pairs 3 --sizes 1000000,all --densities "" --spans "" \
    --out data/mes02/raw/cross-n1000.json

# Contrôle : les deux parcours rendent les mêmes candidats sur chaque pool.
$H agree $I --ns 5000 --queries 5 --sizes 1000000,all \
    --densities 10,25,40,50,60,75,90 --spans 1000000,7740000 \
    --out data/mes02/raw/agree-n5000.json
$H agree $I --ns 1000 --queries 5 --sizes 1000000,all --densities "" --spans "" \
    --out data/mes02/raw/agree-n1000.json

(cd tools && uv run --no-sync python bench/mes02/mes02_parcours_analyze.py \
    --commit "$(git rev-parse HEAD)" --date 2026-10-27 \
    --machine "MacBook Air M3, 8 cœurs, 24 Go, macOS 15.7.7 (24G720), APFS, natif")
```

Durée : 15 minutes pour les deux campagnes et l'analyse, mesurée sur les deux fichiers de conditions
(13:08:13Z → 13:23:23Z), dont environ 3 minutes pour la campagne à N = 1000.

Les percentiles sont ceux d'ADR-0077 : **rang le plus proche**, `ceil(ratio × n)`, sans
interpolation. Aucune valeur n'est écartée. `tools/bench/mes02/mes02_parcours_analyze.py` recalcule
chaque percentile à partir des durées par requête et **refuse** un chiffre qui diffère du résumé
écrit par le harnais ; il importe `nearest_rank` et `check_summary` de `mes02_analyze.py`, pour que
les deux campagnes de MES-02 ne puissent pas diverger sur la définition.

Le scan n'a **pas d'état froid** (ADR-0077, décision 2) : ses codes binaires sont en RAM et parcourus
en entier à chaque requête. La campagne ne comporte donc aucune série froide.

La **dispersion** retenue est celle de MES-02 — `max / min − 1` — mais calculée **entre les paires
d'une même campagne**, et non plus entre deux exécutions séparées : c'est la dispersion de la mesure
elle-même, celle qu'un écart entre parcours doit dépasser pour être lu comme une différence. Pour
l'écart relatif entre parcours, qui peut être négatif, la dispersion est une amplitude en **points
de pourcentage** (`max − min`).

### Contrôles

- **La réplique est ancrée au moteur, et ce diff ne la change pas.** Le mode `cross` appelle
  `scan_replica`, `Scan`, `windows` et `traversal_of`, que cette campagne **n'a pas modifiés** : seuls
  le constructeur de pools (`pool_window`), le générateur (`Draw`, extraction sans changement de
  tirage) et le mode `agree` sont neufs. L'ancrage lui-même est celui de MES-02, conservé : le mode
  `compare` y a montré que la réplique et le moteur rendent les mêmes dix premiers résultats, mêmes
  clés et mêmes scores, sur un pool de 537 500 vivants (filtre `categorie = cat-01`) et sur les
  7 740 000 vivants, `plan binary` dans les deux cas ; la CLI publique `crible search --vector-file`
  a rendu les mêmes résultats, `same_keys_as_engine: true` et `max_score_delta: 0.0`
  (`MES-02-latence-scan-binaire.md`, section « Contrôles », artefact
  `MES-02-latence-scan-binaire.2026-10-27.json`, clés `controls.compare` et `controls.cli`). Ces
  documents bruts sont gitignorés ; c'est leur résumé qui est versionné, dans l'artefact de MES-02.
- **Contrôle neuf, propre à cette campagne : les deux parcours rendent les mêmes candidats sur chaque
  pool.** Une erreur de réplique dans un seul parcours aurait changé le travail mesuré sans rien
  changer de visible. Le mode `agree` reconstruit **les 18 pools de la campagne** avec le même
  constructeur et la même graine, et compare, requête par requête, les listes rendues par les deux
  parcours (même iid, même distance, même ordre) sur les 5 premières requêtes. Résultat :
  **18 pools, 90 requêtes contrôlées, 18/18 identiques**, aucune différence (`data/mes02/raw/agree-n5000.json`,
  `agree-n1000.json`, résumé dans la clé `controls` de l'artefact). L'analyseur **refuse** d'écrire
  l'artefact si un pool diverge.
- **Le mode `agree` ne remplace pas la comparaison à un tri complet** que `binary.rs` porte pour le
  moteur (`dense_and_sparse_traversals_agree_with_a_full_sort`) : il vérifie que les deux parcours
  font le même travail, pas que ce travail est le bon. Le second point est celui du moteur, et il est
  testé à chaque `cargo test`.

### Écarts au protocole

ADR-0077 fixe cinq décisions ; cette campagne applique la décision 4, applique **partiellement** les
décisions 1 et 3, et laisse les décisions 2 et 5 hors de son objet. Comme MES-02, elle les déclare :

- **Décision 4 (requêtes et passes), appliquée, et l'écart de MES-02 corrigé** : 300 requêtes fp32
  distinctes, 3 passes après échauffement, p50/p95/p99 par rang le plus proche. MES-02 ne faisait pas
  d'échauffement séparé ; ici il y en a un, et les passes mesurées forment 3 paires alternées.
- **Décision 2 (froid), sans objet** : le scan n'a pas d'état froid, ses codes étant en RAM.
- **Décision 3 (pools), appliquée en partie** : les pools sont bien construits par le harnais et les
  deux parcours comparés sur un même pool, mais la campagne **ne reprend ni les cinq tailles de pool**
  d'ADR-0077 (1 k, 10 k, 100 k, 1 M, tous les vivants : elle mesure 100 000 à 7 740 000, la bande du
  seuil, plus 1 M contigu) **ni le contre-contrôle par la CLI**, hérité de MES-02 sur le même index et
  le même jour. Les tailles qu'elle ne mesure pas sont citées dans la conclusion, avec leur source.
- **Décision 1 (jeu de N et colonne EXACT), appliquée en partie** : N = 5000 partout et N = 1000 sur
  les deux pools de MES-02, mais **N = 2000 et la colonne EXACT** ne sont pas rejoués ; ils
  appartiennent à la campagne MES-02 de référence, qui les porte sur le même index.
- **Décision 5 (RSS), sans objet** : MES-09, pas MES-02.

### Résultats

Durées brutes par requête, passe par passe : `MES-02-parcours-dense-clairseme.2026-10-27.json`
(clés `rows[].dense.durations_us`, `rows[].sparse.durations_us`). Documents du harnais :
`data/mes02/raw/cross-n5000.json`, `cross-n1000.json` (gitignorés).

Scan seul, en millisecondes, 900 échantillons par parcours et par point (3 paires × 300 requêtes),
8 fils, après une passe d'échauffement de chaque parcours.

| Campagne | Pool | Forme | Plage d'iid | Densité | N | dense p50 | p95 | p99 | clairsemé p50 | p95 | p99 |
|---|---|---|---|---|---|---|---|---|---|---|---|
| N = 5000 | 100 000 | densité | 999 992 | 0,100 | 5000 | 3,522 | 4,211 | 4,422 | 2,638 | 3,025 | 3,154 |
| N = 5000 | 250 000 | densité | 999 999 | 0,250 | 5000 | 4,432 | 5,045 | 5,271 | 3,694 | 4,159 | 4,310 |
| N = 5000 | 400 000 | densité | 999 999 | 0,400 | 5000 | 5,799 | 6,766 | 7,745 | 4,720 | 5,455 | 6,158 |
| N = 5000 | 500 000 | densité | 999 999 | 0,500 | 5000 | 6,160 | 7,863 | 9,556 | 5,177 | 6,281 | 8,233 |
| N = 5000 | 600 000 | densité | 999 999 | 0,600 | 5000 | 6,705 | 7,810 | 8,415 | 5,637 | 8,891 | 11,576 |
| N = 5000 | 750 000 | densité | 999 999 | 0,750 | 5000 | 7,307 | 8,476 | 8,873 | 6,482 | 7,835 | 8,402 |
| N = 5000 | 900 000 | densité | 999 999 | 0,900 | 5000 | 7,758 | 9,272 | 9,561 | 7,410 | 8,751 | 9,110 |
| N = 5000 | 774 000 | densité | 7 739 999 | 0,100 | 5000 | 28,882 | 31,975 | 35,725 | 13,557 | 15,669 | 17,543 |
| N = 5000 | 1 935 000 | densité | 7 739 999 | 0,250 | 5000 | 29,674 | 33,716 | 40,528 | 19,398 | 20,130 | 21,119 |
| N = 5000 | 3 096 000 | densité | 7 739 999 | 0,400 | 5000 | 35,052 | 39,093 | 41,206 | 22,868 | 24,423 | 25,741 |
| N = 5000 | 3 870 000 | densité | 7 739 999 | 0,500 | 5000 | 39,306 | 43,497 | 44,983 | 25,245 | 27,079 | 28,789 |
| N = 5000 | 4 644 000 | densité | 7 739 999 | 0,600 | 5000 | 41,353 | 45,600 | 48,718 | 27,948 | 30,553 | 32,748 |
| N = 5000 | 5 805 000 | densité | 7 739 999 | 0,750 | 5000 | 43,319 | 47,397 | 51,566 | 32,455 | 34,992 | 37,808 |
| N = 5000 | 6 966 000 | densité | 7 739 999 | 0,900 | 5000 | 47,382 | 53,983 | 63,285 | 39,396 | 44,908 | 49,270 |
| N = 5000 | 1 000 000 | contigu | 1 111 111 | 0,900 | 5000 | 7,083 | 8,161 | 8,294 | 6,687 | 7,817 | 7,963 |
| N = 5000 | 7 740 000 | complet | 8 599 999 | 0,900 | 5000 | 43,949 | 47,223 | 49,602 | 35,510 | 37,570 | 40,109 |
| N = 1000 | 1 000 000 | contigu | 1 111 111 | 0,900 | 1000 | 5,445 | 7,251 | 7,905 | 5,445 | 6,441 | 6,928 |
| N = 1000 | 7 740 000 | complet | 8 599 999 | 0,900 | 1000 | 50,738 | 55,701 | 57,493 | 39,719 | 43,204 | 44,641 |

Écart relatif entre parcours sur le même pool : `(clairsemé − dense) / dense`, en pour cent ; un
signe négatif veut dire que le clairsemé est plus rapide. Dispersion : `max / min − 1` des p50 de
chaque parcours d'une paire à l'autre, et amplitude de l'écart en points de pourcentage.

| Campagne | Pool | Densité | N | écart p50 | dispersion dense | dispersion clairsemé | amplitude de l'écart | écarts par paire |
|---|---|---|---|---|---|---|---|---|
| N = 5000 | 100 000 | 0,100 | 5000 | −25,1 % | 4,6 % | 3,9 % | 3,9 pts | −23,2 / −25,6 / −27,1 |
| N = 5000 | 250 000 | 0,250 | 5000 | −16,7 % | 1,1 % | 2,4 % | 1,1 pt | −17,1 / −16,1 / −16,6 |
| N = 5000 | 400 000 | 0,400 | 5000 | −18,6 % | 16,2 % | 17,3 % | 11,8 pts | −16,9 / −15,0 / −26,8 |
| N = 5000 | 500 000 | 0,500 | 5000 | −15,9 % | 4,8 % | 6,9 % | 9,6 pts | −10,1 / −18,2 / −19,7 |
| N = 5000 | 600 000 | 0,600 | 5000 | −15,9 % | 6,8 % | 14,3 % | 10,3 pts | −8,5 / −18,9 / −13,5 |
| N = 5000 | 750 000 | 0,750 | 5000 | −11,3 % | 4,4 % | 1,8 % | 3,1 pts | −9,2 / −10,6 / −12,3 |
| N = 5000 | 900 000 | 0,900 | 5000 | −4,5 % | 0,9 % | 0,8 % | 0,3 pt | −4,5 / −4,9 / −4,5 |
| N = 5000 | 774 000 | 0,100 | 5000 | −53,1 % | 3,2 % | 1,6 % | 2,0 pts | −53,6 / −53,6 / −51,6 |
| N = 5000 | 1 935 000 | 0,250 | 5000 | −34,6 % | 3,9 % | 0,4 % | 2,2 pts | −35,9 / −33,7 / −34,1 |
| N = 5000 | 3 096 000 | 0,400 | 5000 | −34,8 % | 2,7 % | 2,1 % | 0,8 pt | −34,4 / −35,2 / −34,8 |
| N = 5000 | 3 870 000 | 0,500 | 5000 | −35,8 % | 1,2 % | 0,6 % | 1,1 pt | −36,3 / −36,0 / −35,2 |
| N = 5000 | 4 644 000 | 0,600 | 5000 | −32,4 % | 1,4 % | 1,2 % | 0,9 pt | −32,4 / −32,0 / −32,8 |
| N = 5000 | 5 805 000 | 0,750 | 5000 | −25,1 % | 0,6 % | 0,1 % | 0,5 pt | −24,9 / −25,4 / −24,9 |
| N = 5000 | 6 966 000 | 0,900 | 5000 | −16,9 % | 9,0 % | 3,1 % | 4,7 pts | −13,9 / −18,6 / −18,4 |
| N = 5000 | 1 000 000 | 0,900 | 5000 | −5,6 % | 1,1 % | 0,5 % | 0,7 pt | −6,1 / −5,4 / −5,6 |
| N = 5000 | 7 740 000 | 0,900 | 5000 | −19,2 % | 3,6 % | 0,8 % | 2,3 pts | −18,0 / −19,2 / −20,2 |
| N = 1000 | 1 000 000 | 0,900 | 1000 | −0,0 % | 34,2 % | 27,6 % | 21,0 pts | −6,9 / +9,6 / −11,4 |
| N = 1000 | 7 740 000 | 0,900 | 1000 | −21,7 % | 0,3 % | 2,2 % | 1,6 pt | −20,9 / −22,0 / −22,4 |

Les passes d'échauffement, tenues à part, sont **plus rapides que les passes mesurées** dans 29 cas
sur 36 (médiane −1,3 %, pire −17,2 %) et plus lentes dans 7 (+3,8 % au plus). Ce n'est pas l'effet
d'un démarrage à froid — les codes sont en RAM — mais celui de la **dérive de charge** de la machine
pendant la campagne (charge 3,91 → 4,41). L'alternance ramène ainsi l'écart entre deux exécutions
séparées, qu'aucun appariement ne contrôle, à un **écart entre deux passes voisines du même point** ;
mais comme l'ordre dense → clairsemé ne s'inverse pas, elle laisse un biais d'environ **0,4 % en
faveur du dense** (voir le protocole), donc **conservateur** : il réduit les écarts mesurés, il ne
les fabrique pas.

### Conclusion

**Le seuil de 50 % d'ADR-0071 n'est pas confirmé par la mesure : sur `synth-large`, le parcours
clairsemé est plus rapide que le dense sur 17 des 18 points mesurés, à toutes les densités de 10 à
90 % et sur les deux tailles de plage ; sur le dix-huitième, il est à égalité.** Aucun point ne donne
le dense vainqueur. Les 17 gains dépassent l'amplitude de leur propre écart d'une paire à l'autre ;
le 18e est dans le bruit, et le biais conservateur de 0,4 % décrit au protocole y pousserait le
clairsemé devant.

Ce que la mesure établit :

- **L'inversion du pool complet, vue par MES-02 en une seule exécution, est confirmée par
  l'alternance.** À N = 5000, le clairsemé prend 35,510 ms en p50 contre 43,949 ms au dense
  (**−19,2 %**), avec des écarts par paire de −18,0, −19,2 et −20,2 % (amplitude 2,3 points) et une
  dispersion du dense de 3,6 %. À N = 1000, l'écart est de **−21,7 %** (39,719 contre 50,738 ms),
  écarts par paire −20,9 / −22,0 / −22,4 et dispersion du dense de 0,3 %. Dans les deux cas l'écart
  dépasse largement la dispersion : ADR-0077 avait raison de mettre ce point sous surveillance, et
  la règle « dense au-delà de 50 % » y coûte environ un cinquième du scan.
- **La seconde inversion de MES-02, sur 1 M contigu, se réduit à un match nul à N = 1000 et à un
  gain faible mais net à N = 5000.** À N = 1000, l'écart est de −0,0 % (5,445 contre 5,445 ms) avec
  des écarts par paire de −6,9 / +9,6 / −11,4 % et une dispersion du dense de 34,2 % : les deux
  parcours sont **indiscernables**, et le −11,8 % de MES-02 était bien du bruit d'exécution. À
  N = 5000, l'écart est de **−5,6 %** (6,687 contre 7,083 ms), avec une amplitude de 0,7 point et une
  dispersion du dense de 1,1 % : petit, mais hors dispersion.
- **L'écart s'accroît globalement quand la densité baisse, et quand la plage s'allonge à densité
  égale ; il n'est pas monotone.** À plage de 1 M, l'écart va de −4,5 % (densité 0,90) à −25,1 %
  (0,10), mais −18,6 % à 0,40 passe devant −16,7 % à 0,25 ; à plage de 7,74 M, il va de −16,9 %
  (0,90) à −53,1 % (0,10), et −35,8 % à 0,50 passe devant −34,6 % à 0,25. Ces deux inversions valent
  1,9 et 1,2 point, dans l'amplitude des points concernés (11,8 et 1,1 points). Le dense paie la **plage**, le clairsemé paie le **pool** : à densité 0,50 et plage
  7,74 M, le dense parcourt 7,74 M d'iid pendant que le clairsemé n'en lit que 3,87 M, et l'écart est
  de −35,8 %.
- **17 points sur 18 dépassent l'amplitude de leur propre écart d'une paire à l'autre.** Le seul
  point dans le bruit est celui du match nul ci-dessus (1 M contigu, N = 1000). Les dispersions les
  plus fortes (16,2 % pour le dense et 17,3 % pour le clairsemé à densité 0,40 sur 1 M ; 14,3 % pour
  le clairsemé à densité 0,60) tombent sur des points où l'écart mesuré vaut −18,6 %, −15,9 % et
  −15,9 %, donc au-delà de l'amplitude de leur propre écart par paire (11,8, 9,6 et 10,3 points). La dispersion de 16,1 % que MES-02
  avait tirée de deux exécutions séparées est donc **du même ordre que la dispersion réelle de la
  machine**, mais elle n'efface pas les écarts de cette campagne, qui sont mesurés dans une même
  paire.
- **Le modèle de coût d'ADR-0071 est contredit sur ce jeu.** L'ADR attend du dense un avantage
  d'ordre mémoire dès la moitié de densité ; la mesure n'en voit aucun, même à 90 %. L'explication
  probable — non mesurée ici — est que le clairsemé parcourt lui aussi les iid en **ordre croissant**
  (l'itérateur d'un bitmap est trié) et qu'un index à **un seul segment** rend quasi gratuit son pas
  de bitmap et sa recherche de segment, alors que le dense paie un test de bitmap sur **chaque** iid
  de la plage, morts compris, plus l'équivalent en lectures. La mesure ne mesure pas cette
  explication : elle constate le résultat.

Ce que la mesure **ne** permet pas d'affirmer :

- **que la règle « toujours clairsemé » tienne sur un index multi-segments.** `synth-large` est un
  segment unique de 8,6 M d'iid ; le clairsemé y paie une recherche de segment par passage, coût qui
  croît avec le nombre de segments d'un index compacté de Legifacile. La mesure ne dit rien de ce
  cas, et c'est la principale raison de ne pas changer le seuil sur ces seuls chiffres.
- que les chiffres vaudraient en conteneur : ils sont natifs, sur un MacBook Air chargé (charge 3,9 à
  4,4, Lightroom actif), pas sur le Mac mini de `docs/08:87` ;
- que les valeurs absolues se comparent d'une campagne à l'autre au-delà de cet accord : **à N égal,
  les deux campagnes concordent** — 1 M contigu, dense, 7,083 ms ici contre **7,155 ms** dans la série
  automatique de MES-02 à N = 5000 (`MES-02-latence-scan-binaire.md:140`), soit **−1,0 %** ; pool
  complet, dense, 43,949 contre **44,578 ms** à N = 5000 (`:142`), soit **−1,4 %**. Le 5,338 ms cité
  parfois est le dense **forcé à N = 1000** (`:155`), qui ne se compare pas à une mesure à N = 5000.
  L'accord à N égal vaut pour ces deux pools ; il ne garantit pas les points de densité, que MES-02
  ne mesure pas ;
- que la réplique du harnais soit neutre : elle est **favorable au dense**, puisqu'elle ne fait pas
  le comptage des vivants et la comparaison de rang que `dense_window` fait en production. Si biais
  il y a, il va contre la conclusion ci-dessus ;
- que le gain du clairsemé se retrouve sur une machine au repos, ou sur un pool dont les passages
  vivants ne sont pas triés dans l'ordre des segments.

**Comparaison au budget.** `docs/03-recherche.md:143` donne « 20 à 80 ms » pour 8,6 M et
« quelques ms » pour un pool de 1 M. Au pool complet, le clairsemé mesure **35,5 ms en p50 et 37,6 ms
en p95** à N = 5000, dans la fourchette ; le dense y mesure 43,9 et 47,2 ms, également dans la
fourchette. À 1 M contigu, le clairsemé mesure **6,7 ms en p50 et 7,8 ms en p95** à N = 5000, et
5,4 / 6,4 ms à N = 1000 : MES-02 avait déjà relevé que « quelques ms » n'est tenu que pour un pool
dont les passages sont contigus, et cette campagne ne change pas cette lecture. Le budget n'est donc
tenu dans les deux cas, et le choix du parcours ne fait pas basculer un point hors budget : il coûte
ou rend 5 à 36 % du scan.

**Proposition, à valider par le propriétaire, et bornée au domaine mesuré.** Instruire par un nouvel
ADR le remplacement du seuil d'ADR-0071, **pour les pools de 100 000 passages et plus et pour une
plage d'iid de 1 000 000 et plus**, par la règle **« parcours clairsemé »** — c'est-à-dire supprimer
le seuil, ce qui revient à « toujours clairsemé » dans ce domaine. Trois raisons de borner :

- la campagne ne mesure que des pools de **100 000 à 7 740 000 passages**, sur des plages de 1 M et
  7,74 M ; en dessous, elle n'a **aucune donnée** ;
- les seuls points disponibles en dessous sont ceux de MES-02, forcés à N = 1000, et ils donnent le
  dense **devant**, de très peu, sur les pools contigus de 1 k, 10 k et 100 k : 0,055 contre 0,056 ms,
  0,375 contre 0,376 ms, 0,984 contre 0,991 ms (`MES-02-latence-scan-binaire.md:149-153`). Ces trois
  écarts (0,2 à 0,7 %) sont **dans** la dispersion de 16,1 % de MES-02 et ne concluent rien dans un
  sens ni dans l'autre : ils ne sont ni une objection, ni un appui. Toute règle sur ces tailles
  demande une campagne à passes alternées sur ces pools, que celle-ci ne fait pas ;
- la mesure ne fournit **aucune** densité, jusqu'à 90 %, où le dense gagne ; aucun seuil supérieur à
  90 % n'est donc étayé non plus, et un seuil maintenu au-delà de 90 % serait une règle sans chiffre.

Gain de la règle sur les pools que la règle actuelle envoie au dense, à N = 5000 :

| Pool envoyé au dense par ADR-0071 | dense p50 | clairsemé p50 | gain |
|---|---|---|---|
| plage 1 M, densité 0,50 (500 k) | 6,160 ms | 5,177 ms | −15,9 % |
| plage 1 M, densité 0,75 (750 k) | 7,307 ms | 6,482 ms | −11,3 % |
| plage 1 M, densité 0,90 (900 k) | 7,758 ms | 7,410 ms | −4,5 % |
| plage 7,74 M, densité 0,50 (3,87 M) | 39,306 ms | 25,245 ms | −35,8 % |
| plage 7,74 M, densité 0,75 (5,81 M) | 43,319 ms | 32,455 ms | −25,1 % |
| plage 7,74 M, densité 0,90 (6,97 M) | 47,382 ms | 39,396 ms | −16,9 % |
| 1 M contigu (MES-02), densité 0,90 | 7,083 ms | 6,687 ms | −5,6 % |
| pool complet 7,74 M (MES-02), densité 0,90 | 43,949 ms | 35,510 ms | −19,2 % |

Les pools **dispersés**, ceux qu'un filtre produit réellement, ne changent pas : la règle actuelle
choisit déjà le clairsemé sous 50 % de densité, et c'est là que le gain du clairsemé est le plus
grand (−25 à −53 %). Le changement porte donc sur les pools **contigus**, issus d'une compaction
avec `cluster_by`, où il vaut 5 à 36 % du scan. La recommandation est une proposition, pas une
décision : elle demande un ADR, et le code du moteur n'a pas été modifié.

**Note de forme.** Ce rapport est un **second fichier pour MES-02**, alors que `docs/mesures/README.md:5`
demande un fichier par mesure et une section datée pour une nouvelle campagne. Le choix est assumé et
demandé par la commande : la campagne ne rejoue pas MES-02, elle instruit une question distincte
(le seuil d'ADR-0071), avec son propre harnais, sa propre recette et son propre artefact ; la mêler au
rapport de la tâche `MESURER-LATENCE-DENSE` rendrait les deux protocoles illisibles. Le rapport de
référence porte un **complément daté** qui renvoie ici, et l'artefact garde le nom de la mesure
(`MES-02-parcours-dense-clairseme.2026-10-27.json`, 612 862 octets = 598 Kio, durées par requête
comprises).
