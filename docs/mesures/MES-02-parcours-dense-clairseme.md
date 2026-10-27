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

## Campagne multi-segments du 2026-10-27

- **Tâche** : `UNIFIER-PARCOURS-SCAN` (P4), **livrable (1) seulement** — la mesure ; ni le code du
  curseur de segment ni la suppression de `dense_window` ne sont faits ici.
- **Demande** : préalable de la **décision 2 d'ADR-0081**, qui subordonne la mise en œuvre du parcours
  clairsemé unique à une mesure du **nombre de segments**. C'est le livrable (1) de la tâche
  `UNIFIER-PARCOURS-SCAN` (P4). Ce n'est pas une nouvelle tâche du plan : `docs/suivi.md` est tenu à
  jour par l'orchestrateur, pas par cette campagne.
- **Commit** : `e7d4e083462b1799dd3fe506ca4a1977ae82481e`, branche `phase/P4-recherche-rapide`.
  **Aucun code de production n'a été modifié** : rien dans `crates/*/src/`. Sont livrés avec ce
  rapport, **non commités** au moment de la mesure :
  - `crates/crible/examples/dense_latency.rs` (+56 / −14) : l'ordre des deux parcours **alterne d'une
    paire à l'autre** (`pair_order`, décision 3 d'ADR-0081), le document brut enregistre `order` par
    paire et le découpage en segments (`segments`, `segment_counts`, `segment_iid_starts`) ;
  - `tools/bench/mes02/build-multiseg-copies.sh` (construction des trois copies),
    `make_multiseg_batch.py` (découpage du lot en 100 lots), `run-multiseg.sh` (campagne séquentielle),
    `run-multiseg-interleaved.sh` (série entrelacée) ;
  - `tools/bench/mes02/mes02_multiseg_analyze.py` (nouvel analyseur) et
    `tools/bench/mes02/tests/test_multiseg_analyze.py` (54 tests avec les autres tests de `bench/mes02`) ;
  - ce rapport et son artefact `MES-02-multisegments.2026-10-27.json` (2 256 015 octets, durées par
    requête comprises).
- **Machine** : MacBook Air M3, 8 cœurs, 24 Go, macOS 15.7.7 (24G720), APFS, **natif**. La machine
  n'était **pas au repos** et la charge a **monté** pendant toute la fenêtre de mesure : `2,28 1,89 2,14`
  au début de la campagne (`data/mes02/raw/conditions-multiseg-campaign-start.txt`) et `4,35 6,11 6,62`
  à la fin (`…-campaign-end.txt`), avec `mediaanalysisd` entre 64 et 94 % d'un cœur, `mds_stores`
  jusqu'à 53 %, Lightroom entre 14 et 32 %. La série entrelacée a mesuré entre `3,61 5,73 6,46` et
  `5,97 5,86 6,48` (`…-interleaved-start.txt`, `…-interleaved-end.txt`).
- **Jeu de données** : `synth-large`, 7 740 000 passages vivants, 768 dimensions, 300 requêtes
  (`data/mes02/queries.f32`, graine 20261021, flux 7, loi normale normalisée, SHA-256
  `03cb2a2b3bb7951892179ee8d612b0970626f0f041fe6b3e18c2f0393452948b`), et **trois copies de mesure**
  sous `data/mes02/indexes/` (gitignorées) qui ne diffèrent **que par le nombre de segments** :

  | copie | segments | passages vivants | passages morts | densité | construction |
  |---|---|---|---|---|---|
  | `synth-large` | 1 (`seg-large-0001`) | 7 740 000 | 860 000 | 0,90 | copie de MES-02, **non modifiée** |
  | `synth-large-8seg` | 8 | 7 740 000 | 0 | 1,00 | clonage puis `crible compact` (11 min) |
  | `synth-large-multi` | 100 | 7 740 000 | 860 000 | 0,90 | index neuf, **100 attaches** de 86 000 passages (19 min) |

  `data/synth-large/` n'a pas été touché et les 7,74 M de vivants sont les mêmes partout : la copie
  multi-attaches a reçu **le même lot de suppressions** que la référence, et la jointure SQLite sur
  `iid` donne **0 différence** sur `7 740 000` passages (même vivant ou mort, même `doc_id`).
- **Configuration** : copies de mesure avec `search.exact_threshold = 0` et `search.scan = "binary"`,
  donc **BINARY sur chaque point** ; `candidates_per_branch = 100` (k) ; 8 fils de scan ; scan seul,
  sans rescoring ni payload. **N = 5000** partout (défaut d'ADR-0078).

### Protocole

Le protocole est celui d'ADR-0077 tel que la campagne précédente l'applique (300 requêtes, 3 paires
après un échauffement séparé, percentiles par rang le plus proche), avec les corrections de la
décision 3 d'ADR-0081 : **l'ordre des paires alterne** (dense puis clairsemé, clairsemé puis dense,
dense puis clairsemé), et la **densité 1,0** est obtenue sur un index **sans morts**.

Le harnais **force** les deux parcours sur le même pool et les mêmes vecteurs de requête. Sur chaque
copie, quatre formes de pool sont mesurées, à N = 5000 : 1 M contigu, le pool complet, densité 0,50 sur
une plage de 1 M et densité 0,50 sur une plage de 7,74 M. La campagne séquentielle mesure en plus les
densités 0,10 à 0,90 sur les deux plages, et la densité 1,0 sur la copie compactée (les deux autres
portent 860 000 passages morts et ne peuvent pas l'atteindre).

Commandes, dans l'ordre, depuis la racine du dépôt :

```sh
cargo build --release
sh tools/bench/mes02/build-multiseg-copies.sh

MACHINE='MacBook Air M3, 8 cores, 24 GB, macOS 15.7.7, native' \
  sh tools/bench/mes02/run-multiseg.sh          # campagne séquentielle + contrôles `agree`
MACHINE='MacBook Air M3, 8 cores, 24 GB, macOS 15.7.7, native' \
  sh tools/bench/mes02/run-multiseg-interleaved.sh   # série entrelacée, 3 tours

(cd tools && uv run --no-sync python bench/mes02/mes02_multiseg_analyze.py \
    --commit "$(git rev-parse HEAD)" --date 2026-10-27 \
    --machine 'MacBook Air M3, 8 cores, 24 GB, macOS 15.7.7, native')
(cd tools && uv run --no-sync pytest bench/mes02)
```

Chaque passe de la campagne séquentielle est une invocation du harnais, par exemple :

```sh
target/release/examples/dense_latency cross --data data/mes02 --index synth-large \
    --vectors data/mes02/queries.f32 --ns 5000 --pairs 3 --sizes 1000000,all \
    --spans 1000000,7740000 --densities 10,25,40,50,60,75,90 \
    --out data/mes02/raw/cross-synth-large.json
```

Les 100 lots d'attache (13 Go sous `data/mes02/batches/`) sont découpés depuis le lot de `synth-large`
par `make_multiseg_batch.py`, en 25 secondes, et ont été **supprimés** après la campagne : ils se
reconstruisent à l'identique, alors que les trois index de mesure restent en place. Le découpage est en
outre contrôlé par la jointure d'`iid` décrite ci-dessus.

La **série entrelacée** ajoute trois tours de trois copies en **carré latin** (tour 1 :
`synth-large`, `-8seg`, `-multi` ; tour 2 : `-8seg`, `-multi`, `synth-large` ; tour 3 :
`-multi`, `synth-large`, `-8seg`) sur les quatre formes de pool. Elle répond au seul défaut du
protocole d'ADR-0081 : la décision 2 alterne les parcours **dans** une paire, pas les **copies** entre
elles. L'ordre réel de chaque tour n'est pas écrit dans le document brut ; l'analyseur le lit sur la
date de modification des fichiers, puis **refuse** la série si les trois copies n'occupent pas chacune
une position différente à chaque tour (test `test_interleaved_documents_refuses_an_order_that_is_not_a_latin_square`).
Les écarts entre copies sont ensuite lus **à l'intérieur d'un tour**, entre deux exécutions séparées de
4 à 8 minutes, et non plus à un quart d'heure d'intervalle.

### Contrôles

- **Accord des deux parcours, sur chaque copie** : le mode `agree` rejoue les 16 pools de la campagne
  et compare, requête par requête, les listes rendues par les deux parcours (même iid, même distance,
  même ordre) sur 5 requêtes. Résultat : **48 pools, 240 requêtes, 48/48 identiques**, aucune
  différence (`data/mes02/raw/agree-synth-large.json`, `agree-synth-large-8seg.json`,
  `agree-synth-large-multi.json`, résumé dans la clé `controls` de l'artefact). L'analyseur **refuse**
  d'écrire l'artefact si un pool diverge, et refuse aussi un document dont le nombre de segments ou le
  nom d'index n'est pas celui de la copie déclarée.
- **Mêmes pools sur les copies comparées** : les 16 pools de la copie à un segment et ceux de la copie
  à 100 segments portent les **mêmes identifiants**, le même premier et dernier iid, la même taille et
  la même densité (`same_pools`, clé `identical: true`, 16 points comparés). C'est ce qui autorise à
  lire un écart de durée entre ces deux copies comme un écart de découpage, à travail égal.
- **Ordre alterné** : l'analyseur recalcule chaque paire et **refuse** toute paire dont l'ordre ne suit
  pas l'alternance (test `test_measure_row_refuses_a_pair_whose_order_does_not_alternate`). Les 66
  points de la campagne séquentielle et les 36 de la série entrelacée passent ce contrôle.
- **Témoin de dérive, parcours dense** : le parcours dense ne paie pas de recherche de segment ; il
  sert de témoin. À travail égal (copies à 1 et 100 segments, mêmes iids), son écart médian entre les
  deux copies vaut **+0,5 %, +0,2 %, −2,6 % et +1,5 %** sur les quatre pools de la série entrelacée :
  la machine ne fabrique pas les écarts du clairsemé.
- **Réplique** : le mode `cross` appelle toujours `scan_replica`, `Scan`, `windows` et `traversal_of`,
  inchangés par cette campagne ; l'ancrage de la réplique au moteur est celui de MES-02 et de la
  campagne précédente, non rejoué ici. Cette réplique ne fait pas le comptage des vivants ni la
  comparaison de rang que fait `dense_window` en production : elle reste **favorable au dense**, et le
  biais va contre les conclusions tirées ci-dessous.

### Écarts au protocole

- **Les petits pools (1 k, 10 k, 100 k) ne sont pas mesurés et sont déclarés non mesurés.** La décision
  3 d'ADR-0081 ne les demande que si le propriétaire préfère un seuil de taille ; aucun seuil n'a été
  demandé. Les seuls points existants (MES-02, à N = 1000) sont dans la dispersion de 16,1 %.
- **La densité 1,0 n'est atteinte que sur la copie compactée**, qui est la seule sans passage mort :
  deux points, `density-1000000-100` et `density-7740000-100`, sur les 18 de cette copie. Les deux
  autres copies portent 860 000 morts et plafonnent à 0,90 ; la densité 1,0 y est **hors d'atteinte**.
- **La copie compactée n'a pas la même plage d'iid sur deux des quatre formes** : sans passage mort,
  ses 7 740 000 vivants occupent 7 740 000 iid au lieu de 8 600 000. Sur `contiguous-1000000` et sur le
  pool complet, une fenêtre de 1 M ou de 7,74 M passages vivants y couvre donc **10 % d'iid de moins**
  que sur la référence. Les écarts **8 segments contre 1 segment** de ces deux formes sont confondus
  avec ce travail en moins, et c'est mesurable : le témoin dense y vaut **−7,1 %** et **−9,1 %**,
  proche des −10 % de plage. Ces deux lignes ne sont donc pas retenues pour chiffrer le coût des
  segments ; les pools de densité, dont la plage est la même à un iid près, le sont. Les écarts
  **100 contre 1**, eux, portent sur des ensembles identiques iid pour iid et restent valides partout.
- **Un seul N**, 5000 : la décision 1 d'ADR-0077 n'est appliquée qu'en partie (pas de N = 2000 ni de
  colonne EXACT, pas de N = 1000), comme dans la campagne précédente.
- **La série entrelacée est un ajout au protocole d'ADR-0081** : ni la décision 2 ni la décision 3 ne
  l'imposent. Elle est justifiée par la mesure elle-même : la copie de référence, mesurée au début et à
  la fin de la campagne séquentielle, dérive de **−10,1 % à +17,5 %** sur le dense et de **−10,1 % à
  +26,0 %** sur le clairsemé selon le pool (tableau ci-dessous). Des écarts entre copies de 15 à 30 %,
  mesurés à un quart d'heure d'intervalle, ne sont donc pas lisibles ; la série entrelacée est la preuve qui
  porte, la campagne séquentielle est conservée comme trace et non comme résultat.
- **100 segments, pas « de nombreux petits segments »** : la fourchette annoncée dans la tâche était de
  50 à 100 ; le haut a été retenu, 86 000 passages par segment, soit l'ordre de grandeur d'une attache
  journalière. C'est le cas le plus défavorable mesuré, et il rend l'écart lisible.
- **La machine n'était pas au repos** et la charge a monté de 2,28 à 5,97 pendant la fenêtre (voir
  l'en-tête). Aucune mesure n'a été écartée pour cette raison ; c'est précisément ce que la série
  entrelacée et le témoin dense servent à contrôler.

### Résultats

Durées brutes par requête, passe par passe : `MES-02-multisegments.2026-10-27.json` (clés
`rows[].dense.durations_us`, `rows[].sparse.durations_us`, `rows[].pairs[]`). Documents du harnais :
`data/mes02/raw/cross-synth-large*.json`, `cross-*-r<1..3>.json`, `agree-*.json` (gitignorés).

#### Parcours dense et clairsemé, copie par copie (campagne séquentielle)

Scan seul, en microsecondes, 900 échantillons par parcours et par point (3 paires × 300 requêtes),
8 fils, après une passe d'échauffement de chaque parcours. « clairsemé − dense » est
`(clairsemé − dense) / dense` en pour cent : un signe négatif veut dire que le clairsemé est plus
rapide. Les 66 lignes sont reproduites telles que l'analyseur les imprime, sans exclusion.

| copie | segments | pool | forme | span | densité | N | dense p50 | p95 | p99 | clairsemé p50 | p95 | p99 | clairsemé − dense |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| synth-large | 1 | 1000000 | contiguous | 1111111 | 0.900 | 5000 | 7232 | 8284 | 8391 | 6770 | 7840 | 7926 | -6.4 % |
| synth-large | 1 | 100000 | density | 999992 | 0.100 | 5000 | 3661 | 4161 | 4541 | 2843 | 3030 | 5824 | -22.3 % |
| synth-large | 1 | 250000 | density | 999999 | 0.250 | 5000 | 4563 | 5123 | 5256 | 3905 | 4260 | 4569 | -14.4 % |
| synth-large | 1 | 400000 | density | 999999 | 0.400 | 5000 | 5457 | 6278 | 6402 | 4529 | 5211 | 5355 | -17.0 % |
| synth-large | 1 | 500000 | density | 999999 | 0.500 | 5000 | 6038 | 6976 | 7169 | 5012 | 5799 | 6000 | -17.0 % |
| synth-large | 1 | 600000 | density | 999999 | 0.600 | 5000 | 6537 | 7611 | 10294 | 5585 | 6492 | 7432 | -14.6 % |
| synth-large | 1 | 750000 | density | 999999 | 0.750 | 5000 | 6834 | 7983 | 8214 | 6196 | 7293 | 7467 | -9.3 % |
| synth-large | 1 | 900000 | density | 999999 | 0.900 | 5000 | 7436 | 8636 | 8990 | 7007 | 8264 | 8490 | -5.8 % |
| synth-large | 1 | 774000 | density | 7739999 | 0.100 | 5000 | 29039 | 32078 | 34192 | 13740 | 16044 | 20369 | -52.7 % |
| synth-large | 1 | 1935000 | density | 7739999 | 0.250 | 5000 | 30467 | 34498 | 38046 | 19704 | 21041 | 21778 | -35.3 % |
| synth-large | 1 | 3096000 | density | 7739999 | 0.400 | 5000 | 36354 | 40727 | 45807 | 23426 | 25248 | 26108 | -35.6 % |
| synth-large | 1 | 3870000 | density | 7739999 | 0.500 | 5000 | 40961 | 45385 | 47465 | 25984 | 28872 | 29646 | -36.6 % |
| synth-large | 1 | 4644000 | density | 7739999 | 0.600 | 5000 | 45448 | 50468 | 52540 | 29627 | 34449 | 36286 | -34.8 % |
| synth-large | 1 | 5805000 | density | 7739999 | 0.750 | 5000 | 45880 | 50115 | 52592 | 33091 | 37621 | 38638 | -27.9 % |
| synth-large | 1 | 6966000 | density | 7739999 | 0.900 | 5000 | 46844 | 51204 | 54281 | 37740 | 42654 | 43861 | -19.4 % |
| synth-large | 1 | 7740000 | full | 8599999 | 0.900 | 5000 | 44088 | 48198 | 51853 | 35540 | 39031 | 39976 | -19.4 % |
| synth-large-8seg | 8 | 1000000 | contiguous | 1000000 | 1.000 | 5000 | 7926 | 10228 | 12750 | 7851 | 10464 | 13940 | -0.9 % |
| synth-large-8seg | 8 | 100000 | density | 999978 | 0.100 | 5000 | 5058 | 6010 | 7565 | 3715 | 4549 | 6257 | -26.5 % |
| synth-large-8seg | 8 | 250000 | density | 999992 | 0.250 | 5000 | 6577 | 9286 | 14477 | 5236 | 6850 | 7909 | -20.4 % |
| synth-large-8seg | 8 | 400000 | density | 999999 | 0.400 | 5000 | 7616 | 8926 | 10132 | 6434 | 7829 | 11602 | -15.5 % |
| synth-large-8seg | 8 | 500000 | density | 999999 | 0.500 | 5000 | 8048 | 10060 | 13788 | 7298 | 9896 | 12644 | -9.3 % |
| synth-large-8seg | 8 | 600000 | density | 999999 | 0.600 | 5000 | 8475 | 10334 | 11567 | 7419 | 8568 | 9408 | -12.5 % |
| synth-large-8seg | 8 | 750000 | density | 999999 | 0.750 | 5000 | 8651 | 10165 | 10747 | 8330 | 9665 | 11493 | -3.7 % |
| synth-large-8seg | 8 | 900000 | density | 1000000 | 0.900 | 5000 | 8561 | 10722 | 12627 | 8261 | 9715 | 10170 | -3.5 % |
| synth-large-8seg | 8 | 1000000 | density | 1000000 | 1.000 | 5000 | 9132 | 11266 | 15867 | 9681 | 11843 | 14195 | +6.0 % |
| synth-large-8seg | 8 | 774000 | density | 7739996 | 0.100 | 5000 | 31668 | 37096 | 41202 | 15201 | 17274 | 20242 | -52.0 % |
| synth-large-8seg | 8 | 1935000 | density | 7739996 | 0.250 | 5000 | 35405 | 40892 | 61271 | 23382 | 28877 | 38285 | -34.0 % |
| synth-large-8seg | 8 | 3096000 | density | 7740000 | 0.400 | 5000 | 40798 | 45362 | 47789 | 27213 | 31066 | 41940 | -33.3 % |
| synth-large-8seg | 8 | 3870000 | density | 7740000 | 0.500 | 5000 | 44597 | 49816 | 52074 | 30373 | 34077 | 40300 | -31.9 % |
| synth-large-8seg | 8 | 4644000 | density | 7740000 | 0.600 | 5000 | 44876 | 69395 | 95364 | 32266 | 40571 | 60549 | -28.1 % |
| synth-large-8seg | 8 | 5805000 | density | 7740000 | 0.750 | 5000 | 37972 | 41970 | 48810 | 30973 | 32782 | 34841 | -18.4 % |
| synth-large-8seg | 8 | 6966000 | density | 7740000 | 0.900 | 5000 | 40322 | 43951 | 46548 | 36321 | 38825 | 41075 | -9.9 % |
| synth-large-8seg | 8 | 7740000 | density | 7740000 | 1.000 | 5000 | 41392 | 43706 | 46547 | 39585 | 42621 | 44913 | -4.4 % |
| synth-large-8seg | 8 | 7740000 | full | 7740000 | 1.000 | 5000 | 50453 | 70788 | 94604 | 48084 | 59886 | 80185 | -4.7 % |
| synth-large-multi | 100 | 1000000 | contiguous | 1111111 | 0.900 | 5000 | 7783 | 9074 | 11493 | 8595 | 10272 | 13603 | +10.4 % |
| synth-large-multi | 100 | 100000 | density | 999992 | 0.100 | 5000 | 4079 | 4645 | 5001 | 3227 | 3601 | 4790 | -20.9 % |
| synth-large-multi | 100 | 250000 | density | 999999 | 0.250 | 5000 | 5242 | 5881 | 6060 | 4499 | 4888 | 5240 | -14.2 % |
| synth-large-multi | 100 | 400000 | density | 999999 | 0.400 | 5000 | 6030 | 6982 | 7336 | 5335 | 6094 | 6166 | -11.5 % |
| synth-large-multi | 100 | 500000 | density | 999999 | 0.500 | 5000 | 6474 | 7569 | 8000 | 5863 | 6790 | 7341 | -9.4 % |
| synth-large-multi | 100 | 600000 | density | 999999 | 0.600 | 5000 | 6839 | 8041 | 8194 | 6484 | 7561 | 7658 | -5.2 % |
| synth-large-multi | 100 | 750000 | density | 999999 | 0.750 | 5000 | 7201 | 8524 | 9635 | 7388 | 8622 | 9020 | +2.6 % |
| synth-large-multi | 100 | 900000 | density | 999999 | 0.900 | 5000 | 7447 | 8895 | 9634 | 8231 | 9413 | 9722 | +10.5 % |
| synth-large-multi | 100 | 774000 | density | 7739999 | 0.100 | 5000 | 27238 | 31171 | 35828 | 14772 | 16544 | 17257 | -45.8 % |
| synth-large-multi | 100 | 1935000 | density | 7739999 | 0.250 | 5000 | 27858 | 31898 | 34229 | 19377 | 21200 | 23136 | -30.4 % |
| synth-large-multi | 100 | 3096000 | density | 7739999 | 0.400 | 5000 | 34199 | 38115 | 40595 | 24142 | 26680 | 27817 | -29.4 % |
| synth-large-multi | 100 | 3870000 | density | 7739999 | 0.500 | 5000 | 37660 | 42476 | 44383 | 27726 | 31045 | 34981 | -26.4 % |
| synth-large-multi | 100 | 4644000 | density | 7739999 | 0.600 | 5000 | 40494 | 44629 | 46867 | 31663 | 34687 | 36207 | -21.8 % |
| synth-large-multi | 100 | 5805000 | density | 7739999 | 0.750 | 5000 | 43090 | 47878 | 51625 | 37721 | 43091 | 45181 | -12.5 % |
| synth-large-multi | 100 | 6966000 | density | 7739999 | 0.900 | 5000 | 42061 | 46407 | 49374 | 42631 | 46307 | 52611 | +1.4 % |
| synth-large-multi | 100 | 7740000 | full | 8599999 | 0.900 | 5000 | 47127 | 51438 | 53590 | 45694 | 50013 | 51274 | -3.0 % |
| synth-large-repeat | 1 | 1000000 | contiguous | 1111111 | 0.900 | 5000 | 7886 | 9088 | 9348 | 7257 | 8488 | 8658 | -8.0 % |
| synth-large-repeat | 1 | 100000 | density | 999992 | 0.100 | 5000 | 4174 | 4782 | 5257 | 3102 | 3391 | 3604 | -25.7 % |
| synth-large-repeat | 1 | 250000 | density | 999999 | 0.250 | 5000 | 5244 | 5873 | 6428 | 4350 | 4687 | 5069 | -17.0 % |
| synth-large-repeat | 1 | 400000 | density | 999999 | 0.400 | 5000 | 6100 | 7152 | 7925 | 5010 | 5707 | 5975 | -17.9 % |
| synth-large-repeat | 1 | 500000 | density | 999999 | 0.500 | 5000 | 6661 | 8387 | 9478 | 5389 | 6303 | 6417 | -19.1 % |
| synth-large-repeat | 1 | 600000 | density | 999999 | 0.600 | 5000 | 7684 | 9444 | 13385 | 7039 | 8087 | 15886 | -8.4 % |
| synth-large-repeat | 1 | 750000 | density | 999999 | 0.750 | 5000 | 7627 | 9017 | 9506 | 6624 | 7806 | 8072 | -13.1 % |
| synth-large-repeat | 1 | 900000 | density | 999999 | 0.900 | 5000 | 7762 | 8985 | 9381 | 7370 | 8610 | 8801 | -5.1 % |
| synth-large-repeat | 1 | 774000 | density | 7739999 | 0.100 | 5000 | 27389 | 31463 | 33926 | 13707 | 15762 | 18580 | -50.0 % |
| synth-large-repeat | 1 | 1935000 | density | 7739999 | 0.250 | 5000 | 28367 | 32721 | 34972 | 18662 | 19781 | 20661 | -34.2 % |
| synth-large-repeat | 1 | 3096000 | density | 7739999 | 0.400 | 5000 | 34341 | 38525 | 40382 | 22160 | 23829 | 24574 | -35.5 % |
| synth-large-repeat | 1 | 3870000 | density | 7739999 | 0.500 | 5000 | 38621 | 42900 | 45206 | 24124 | 26576 | 27472 | -37.5 % |
| synth-large-repeat | 1 | 4644000 | density | 7739999 | 0.600 | 5000 | 41220 | 45424 | 47562 | 26634 | 29744 | 30519 | -35.4 % |
| synth-large-repeat | 1 | 5805000 | density | 7739999 | 0.750 | 5000 | 42201 | 46347 | 48344 | 30416 | 33928 | 34949 | -27.9 % |
| synth-large-repeat | 1 | 6966000 | density | 7739999 | 0.900 | 5000 | 42112 | 46121 | 48346 | 34270 | 38168 | 38878 | -18.6 % |
| synth-large-repeat | 1 | 7740000 | full | 8599999 | 0.900 | 5000 | 48240 | 53311 | 56298 | 38504 | 42338 | 44299 | -20.2 % |

Sur la copie à un segment, le clairsemé est devant sur **16 points sur 16**, comme dans la campagne
précédente. Sur les copies à 8 et 100 segments, il **recule** : il est derrière sur **1 point sur 18**
à 8 segments (`density-1000000-100`, **+6,0 %**) et sur **4 points sur 16** à 100 segments
(`contiguous-1000000` **+10,4 %**, `density-1000000-75` **+2,6 %**, `density-1000000-90` **+10,5 %**,
`density-7740000-90` **+1,4 %**). Les quatre points perdus à 100 segments sont les pools **denses**
(0,75 et 0,90), c'est-à-dire exactement la zone où le parcours dense avait un avantage d'ordre mémoire.

#### Coût du nombre de segments, campagne séquentielle (non retenu pour la conclusion)

Même pool, même plage, même N ; p50 du clairsemé en microsecondes. Cette campagne mesure les trois
copies l'une après l'autre, sur une heure où la charge a monté de 2,28 à 4,35 : **ces écarts ne sont
pas retenus pour conclure**, et le tableau est reproduit pour que la comparaison avec la série
entrelacée soit possible.

| pool | forme | taille | N | clairsemé 1 seg | 8 seg | 100 seg | 8 vs 1 | 100 vs 1 | 100 vs 8 |
|---|---|---|---|---|---|---|---|---|---|
| contiguous-1000000 | contiguous | 1000000 | 5000 | 6770 | 7851 | 8595 | +16.0 % | +27.0 % | +9.5 % |
| density-1000000-10 | density | 100000 | 5000 | 2843 | 3715 | 3227 | +30.7 % | +13.5 % | -13.2 % |
| density-1000000-25 | density | 250000 | 5000 | 3905 | 5236 | 4499 | +34.1 % | +15.2 % | -14.1 % |
| density-1000000-40 | density | 400000 | 5000 | 4529 | 6434 | 5335 | +42.1 % | +17.8 % | -17.1 % |
| density-1000000-50 | density | 500000 | 5000 | 5012 | 7298 | 5863 | +45.6 % | +17.0 % | -19.7 % |
| density-1000000-60 | density | 600000 | 5000 | 5585 | 7419 | 6484 | +32.8 % | +16.1 % | -12.6 % |
| density-1000000-75 | density | 750000 | 5000 | 6196 | 8330 | 7388 | +34.5 % | +19.2 % | -11.3 % |
| density-1000000-90 | density | 900000 | 5000 | 7007 | 8261 | 8231 | +17.9 % | +17.5 % | -0.4 % |
| density-7740000-10 | density | 774000 | 5000 | 13740 | 15201 | 14772 | +10.6 % | +7.5 % | -2.8 % |
| density-7740000-25 | density | 1935000 | 5000 | 19704 | 23382 | 19377 | +18.7 % | -1.7 % | -17.1 % |
| density-7740000-40 | density | 3096000 | 5000 | 23426 | 27213 | 24142 | +16.2 % | +3.1 % | -11.3 % |
| density-7740000-50 | density | 3870000 | 5000 | 25984 | 30373 | 27726 | +16.9 % | +6.7 % | -8.7 % |
| density-7740000-60 | density | 4644000 | 5000 | 29627 | 32266 | 31663 | +8.9 % | +6.9 % | -1.9 % |
| density-7740000-75 | density | 5805000 | 5000 | 33091 | 30973 | 37721 | -6.4 % | +14.0 % | +21.8 % |
| density-7740000-90 | density | 6966000 | 5000 | 37740 | 36321 | 42631 | -3.8 % | +13.0 % | +17.4 % |
| full | full | 7740000 | 5000 | 35540 | 48084 | 45694 | +35.3 % | +28.6 % | -5.0 % |

La comparaison « 8 vs 1 » y culmine à +45,6 % et « 100 vs 1 » à +28,6 %, alors que la série
entrelacée donne +3,6 % et +19,5 % sur les mêmes formes : l'ordre de grandeur du biais est celui de la
dérive mesurée sur la copie de référence, ci-dessous. C'est cette campagne-là qui aurait conclu à tort.

#### Dérive de la machine, deux passes de la copie à un segment

Écart de la seconde passe (`synth-large-repeat`) contre la première (`synth-large`), p50, en pour
cent ; positive veut dire que la seconde passe est plus lente. Même copie, mêmes pools, même harnais,
43 minutes d'intervalle.

| pool | N | dense | clairsemé |
|---|---|---|---|
| contiguous-1000000 | 5000 | +9.0 | +7.2 |
| density-1000000-10 | 5000 | +14.0 | +9.1 |
| density-1000000-25 | 5000 | +14.9 | +11.4 |
| density-1000000-40 | 5000 | +11.8 | +10.6 |
| density-1000000-50 | 5000 | +10.3 | +7.5 |
| density-1000000-60 | 5000 | +17.5 | +26.0 |
| density-1000000-75 | 5000 | +11.6 | +6.9 |
| density-1000000-90 | 5000 | +4.4 | +5.2 |
| density-7740000-10 | 5000 | -5.7 | -0.2 |
| density-7740000-25 | 5000 | -6.9 | -5.3 |
| density-7740000-40 | 5000 | -5.5 | -5.4 |
| density-7740000-50 | 5000 | -5.7 | -7.2 |
| density-7740000-60 | 5000 | -9.3 | -10.1 |
| density-7740000-75 | 5000 | -8.0 | -8.1 |
| density-7740000-90 | 5000 | -10.1 | -9.2 |
| full | 5000 | +9.4 | +8.3 |

Selon le pool, la même copie, le même jour, à 43 minutes d'intervalle, varie de **−10,1 % à +17,5 %**
sur le dense et de **−10,1 % à +26,0 %** sur le clairsemé ; le signe change entre les deux plages. Un
écart de 15 à 30 % lu entre deux copies mesurées à un quart d'heure d'intervalle n'est donc pas
interprétable, ce qui condamne la lecture directe du tableau précédent.

#### Série entrelacée : les écarts entre copies, lus dans un même tour

Trois tours, carré latin, quatre pools, N = 5000. p50 du clairsemé en microsecondes ; « 8 vs 1 »,
« 100 vs 1 » et « 100 vs 8 » sont les écarts relatifs des p50 **à l'intérieur du tour**. La ligne
« médiane » donne la médiane des trois tours, le minimum observé et l'amplitude (max − min) en points
de pourcentage.

Tour 1 : `synth-large`, `synth-large-8seg`, `synth-large-multi` ; tour 2 : `synth-large-8seg`,
`synth-large-multi`, `synth-large` ; tour 3 : `synth-large-multi`, `synth-large`, `synth-large-8seg`.

| pool | tour | clairsemé 1 seg | 8 seg | 100 seg | 8 vs 1 | 100 vs 1 | 100 vs 8 |
|---|---|---|---|---|---|---|---|
| contiguous-1000000 n=5000 | 1 | 6876 | 7556 | 8182 | +9.9 % | +19.0 % | +8.3 % |
| contiguous-1000000 n=5000 | 2 | 7308 | 7753 | 8301 | +6.1 % | +13.6 % | +7.1 % |
| contiguous-1000000 n=5000 | 3 | 7427 | 7605 | 8294 | +2.4 % | +11.7 % | +9.1 % |
| contiguous-1000000 n=5000 | médiane | — | — | — | +6.1 % (min +2.4, amplitude 7.5 pts) | +13.6 % (min +11.7, amplitude 7.3 pts) | +8.3 % (min +7.1, amplitude 2.0 pts) |
| density-1000000-50 n=5000 | 1 | 5042 | 5238 | 6433 | +3.9 % | +27.6 % | +22.8 % |
| density-1000000-50 n=5000 | 2 | 5554 | 5756 | 5917 | +3.6 % | +6.5 % | +2.8 % |
| density-1000000-50 n=5000 | 3 | 5426 | 5496 | 5894 | +1.3 % | +8.6 % | +7.3 % |
| density-1000000-50 n=5000 | médiane | — | — | — | +3.6 % (min +1.3, amplitude 2.6 pts) | +8.6 % (min +6.5, amplitude 21.0 pts) | +7.3 % (min +2.8, amplitude 20.0 pts) |
| density-7740000-50 n=5000 | 1 | 22656 | 23550 | 30117 | +3.9 % | +32.9 % | +27.9 % |
| density-7740000-50 n=5000 | 2 | 25380 | 25934 | 28364 | +2.2 % | +11.8 % | +9.4 % |
| density-7740000-50 n=5000 | 3 | 24370 | 25150 | 27451 | +3.2 % | +12.6 % | +9.1 % |
| density-7740000-50 n=5000 | médiane | — | — | — | +3.2 % (min +2.2, amplitude 1.8 pts) | +12.6 % (min +11.8, amplitude 21.2 pts) | +9.4 % (min +9.1, amplitude 18.7 pts) |
| full n=5000 | 1 | 36642 | 39379 | 44877 | +7.5 % | +22.5 % | +14.0 % |
| full n=5000 | 2 | 38320 | 41960 | 45797 | +9.5 % | +19.5 % | +9.1 % |
| full n=5000 | 3 | 38030 | 39697 | 45225 | +4.4 % | +18.9 % | +13.9 % |
| full n=5000 | médiane | — | — | — | +7.5 % (min +4.4, amplitude 5.1 pts) | +19.5 % (min +18.9, amplitude 3.6 pts) | +13.9 % (min +9.1, amplitude 4.8 pts) |

Témoin : le parcours dense, qui ne dépend pas du découpage (médiane des trois tours).

| pool | 8 vs 1 | 100 vs 1 | 100 vs 8 |
|---|---|---|---|
| contiguous-1000000 | -7.1 % | +0.5 % | +8.0 % |
| density-1000000-50 | -2.0 % | -2.6 % | -0.5 % |
| density-7740000-50 | -2.1 % | +1.5 % | +1.1 % |
| full | -9.1 % | +0.2 % | +13.7 % |

Deux lectures, et deux seulement :

- **le clairsemé croît avec le nombre de segments, dans chaque tour et sur chaque pool.** Les 12
  combinaisons tour × pool donnent le p50 du clairsemé strictement ordonné `1 < 8 < 100` ; l'écart
  « 100 vs 1 » est **positif dans les 12 cas**, de **+6,5 %** au minimum à +32,9 %, médiane de +8,6 à
  +19,5 % selon le pool. Le minimum observé, pool par pool, vaut +11,7 % (1 M contigu), +6,5 %
  (densité 0,50 sur 1 M), +11,8 % (densité 0,50 sur 7,74 M) et +18,9 % (pool complet) ;
- **le témoin dense ne bouge pas entre 1 et 100 segments** : +0,5 %, +0,2 %, −2,6 % et +1,5 %, tous
  dans ±2,6 %, sans lien avec le nombre de segments. Sur les deux formes confondues (contigu et pool
  complet), son écart 8 vs 1 (−7,1 % et −9,1 %) mesure le travail en moins de la copie compactée, pas
  un effet de découpage, et disparaît entre 1 et 100 segments (+0,5 % et +0,2 %).

Ramené au passage vivant — écart de p50 divisé par le nombre de passages du pool, ce qui est un calcul
sur les chiffres mesurés, sans extrapolation —, le surcoût du découpage à 100 segments vaut
**0,99 ns** par passage sur 1 M vivants, **0,98 ns** sur 500 000, **1,03 ns** sur 3,87 M et **0,93 ns**
sur 7,74 M : il est **constant par passage** et ne dépend pas de la taille du pool. À 8 segments, il
vaut 0,14 à 0,30 ns par passage, du même ordre que le bruit du témoin dense.

#### Budget de `docs/03-recherche.md:143` (clairsemé, N = 5000)

La ligne donne « 20 à 80 ms » pour l'index entier et « quelques ms » pour un pool de 1 M passages ; la
campagne précédente lit « quelques ms » contre **10 ms**.

| copie | segments | pool complet p50 / p95 (ms) | dans 20–80 ms | pool de 1 M p50 / p95 (ms) | dans 10 ms (p95) |
|---|---|---|---|---|---|
| synth-large | 1 | 35.5 / 39.0 | oui | 6.8 / 7.8 | oui |
| synth-large-8seg | 8 | 48.1 / 59.9 | oui | 7.9 / 10.5 | non (p95 10,5) |
| synth-large-multi | 100 | 45.7 / 50.0 | oui | 8.6 / 10.3 | non (p95 10,3) |
| synth-large-repeat | 1 | 38.5 / 42.3 | oui | 7.3 / 8.5 | oui |

### Conclusion

**Le nombre de segments a un coût, il croît avec ce nombre, et il est mesurable.** Les cinq questions
posées à cette campagne, avec leurs chiffres :

1. **Le clairsemé est-il au moins aussi rapide que le dense sur chaque copie, à chaque point ? Non.**
   Sur la copie à un segment, oui, 16 points sur 16. Sur la copie à 8 segments, il perd le point de
   densité 1,0 (**+6,0 %**). Sur la copie à 100 segments, il perd **4 points sur 16**, tous des pools
   denses : 1 M contigu **+10,4 %**, densité 0,75 sur 1 M **+2,6 %**, densité 0,90 sur 1 M **+10,5 %**,
   densité 0,90 sur 7,74 M **+1,4 %**. C'est le premier point mesuré à ordre alterné qui donne le
   dense vainqueur, et ADR-0081 dit qu'« un seul suffirait à rouvrir la question ».
2. **Le clairsemé perd-il du terrain à mesure que le nombre de segments croît ? Oui, et de façon
   monotone.** Dans chaque tour de la série entrelacée et sur chaque pool, p50(clairsemé) est
   strictement croissant de 1 à 8 puis à 100 segments (12 combinaisons sur 12). L'écart à 100 segments
   contre 1 segment vaut **+8,6 % à +19,5 %** en médiane selon le pool (**+6,5 %** au minimum observé),
   alors que le témoin dense reste dans **±2,6 %** sur la même comparaison. À 8 segments, l'écart
   comparable (pools de densité, même plage) vaut **+3,2 % et +3,6 %** ; sur les deux formes où la
   copie compactée couvre 10 % d'iid de moins, il vaut +6,1 % et +7,5 %, ce qui est un **minorant**
   du surcoût, puisque le travail y est plus petit. Le seuil d'alerte d'ADR-0081, « au-delà de
   quelques pour cent sur une copie à 8 segments », est donc **atteint dès 8 segments** et largement
   dépassé à 100.
3. **Quel est le coût du nombre de segments par rapport à un segment, et où vit-il ?** À 100 segments, le
   clairsemé paie **0,93 à 1,03 ns par passage vivant**, constant sur les quatre pools (1 M, 500 000,
   3,87 M et 7,74 M vivants) : c'est un coût **par passage**, pas par requête. Il est dans
   `crates/crible-search/src/dense/binary.rs` : `sparse_window` (ligne 404) appelle `segment_of` pour
   **chaque** iid vivant, et `segment_of` (lignes 419-435) fait une **recherche binaire**
   (`partition_point`, ligne 422) dans la table des segments, plus un contrôle de borne
   (`segment.end()`, ligne 429). `dense_window` (lignes 351-388) ne la paie pas : il boucle sur les
   segments à l'extérieur et sur les iid à l'intérieur, donc le segment est déjà en main.
4. **Le contrôle d'accord tient-il sur les trois copies ? Oui.** 48 pools, 240 requêtes, 48/48
   identiques, mêmes iid, mêmes distances, même ordre ; l'analyseur refuse d'écrire l'artefact dans le
   cas contraire. Les 16 pools des copies à 1 et 100 segments portent en outre les mêmes identifiants.
5. **Le budget de `docs/03-recherche.md:143` est-il tenu partout ? Oui pour la ligne « 20 à 80 ms »,
   non pour la lecture de « quelques ms » à 10 ms en p95 sur les copies multi-segments.** Le pool
   complet reste entre **35,5 et 48,1 ms** en p50 et **39,0 et 59,9 ms** en p95 sur les quatre copies,
   dans la fourchette. À 1 M contigu, le p50 reste dans « quelques ms » (6,8 à 8,6 ms) mais le **p95
   atteint 10,5 ms à 8 segments et 10,3 ms à 100 segments**, soit 3 à 5 % au-dessus de la lecture de
   10 ms retenue par la campagne précédente ; la copie à un segment, elle, reste à 7,8 puis 8,5 ms.

**Ce que la mesure ne tranche pas.** Les petits pools (1 k, 10 k, 100 k) ne sont pas mesurés ; la
densité 1,0 n'existe que sur la copie compactée, avec un seul point par plage ; le régime au-delà de
100 segments n'est pas mesuré (Legifacile compacté est attendu à environ 9 segments, `docs/02:179`,
mais un index nourri par attaches journalières peut en compter davantage) ; la machine n'était pas au
repos, et les amplitudes de la série entrelacée (jusqu'à 21 points sur un pool) montrent que trois
tours ne suffisent pas à réduire le bruit sous quelques points.

**Question de conception remontée à l'orchestrateur.** La décision 2 d'ADR-0081 fait de la mesure
multi-segments la **condition** de la mise en œuvre du parcours clairsemé unique, et son « À
surveiller » dit que le **curseur de segment** (O(1) amorti, au lieu d'une recherche binaire par iid)
devient « la mise en œuvre à faire, pas une option » si le clairsemé perd du terrain à mesure que le
nombre de segments croît. La mesure dit que c'est le cas : +3,2 à +3,6 % dès 8 segments sur les pools
comparables, +6,5 à +19,5 % à 100 segments, et 4 points denses où le dense repasse devant à 100
segments. `UNIFIER-PARCOURS-SCAN` tel qu'il est ficelé ne prévoit ni le curseur ni ces points ;
l'arbitrage — curseur dans cette tâche, ou tâche distincte avant J4, ou acceptation du coût mesuré en
supprimant `dense_window` quand même — appartient au propriétaire. **Aucun code de production n'a été
modifié dans cette campagne, et aucune correction n'est proposée ici comme acquise.**

**Proposition, à valider par le propriétaire, et bornée au domaine mesuré.** Trois éléments, séparés :

- **implémenter le curseur de segment** dans `sparse_window` avant de supprimer `dense_window`, avec un
  test sur des segments de tailles inégales (le cas de `synth-large-8seg`, 7 × 1 M + 740 000, et celui
  de `synth-large-multi`, 100 × 86 000, sont tous deux disponibles comme copies de mesure) ; c'est le
  remède que l'ADR nomme lui-même ;
- **rejouer la série entrelacée après le curseur**, sur les mêmes copies, pour vérifier que l'écart
  « 100 vs 1 » retombe dans le bruit du témoin dense (±2,6 %) ; sans ce contrôle, on ne saura pas si le
  curseur a supprimé le coût ;
- **ne pas réintroduire de seuil de densité** : sur les 4 points où le dense repasse devant à 100
  segments, il gagne de +1,4 à +10,5 % du scan, alors que les mêmes copies donnent au clairsemé −20 à
  −53 % sur les pools dispersés ; un seuil rendrait deux parcours pour ces quelques points. C'est une
  proposition, pas une décision : elle demande l'accord du propriétaire, qui seul tranche.

**Note de forme.** La campagne est ajoutée au rapport de MES-02 comme **section datée**, conformément à
`docs/mesures/README.md:5`, avec son propre artefact (`MES-02-multisegments.2026-10-27.json`) : elle
rejoue la même mesure sur le même jeu de données et le même harnais, elle n'instruit pas une question
différente. Les durées par requête sont dans l'artefact ; les documents bruts du harnais restent
gitignorés sous `data/mes02/raw/`.

## Vérification A/B du parcours unique du 2026-10-27 (ADR-0082)

Section datée ajoutée **après le code** de `UNIFIER-PARCOURS-SCAN`, comme la décision 2 d'ADR-0082
l'exige avant de déclarer la tâche `FAIT`. Elle compare deux binaires sur les trois copies de la
campagne multi-segments : l'**ancien**, construit depuis le commit `e7d4e08`, où vit encore la règle
de densité d'ADR-0071, et le **nouveau**, l'arbre de travail, parcours clairsemé unique avec le
curseur de segment d'ADR-0082. Tous les points de la campagne multi-segments sont mesurés, sans
exception ni échantillon.

### Protocole

Machine : MacBook Air M3, 8 cœurs, 24 Gio, macOS 15.7.7, natif, aucun conteneur. Commit de l'arbre de
travail au moment de la mesure : `4a06e7c6fc21d1228453f84407c029b677985982` (`e7d4e08` est l'ancien).
Conditions relevées au début et à la fin de chaque étape, sous `data/mes02/raw/` :

| moment | charge (1, 5, 15 min) | processus les plus actifs |
|---|---|---|
| début (17:23 UTC) | 3,66 / 4,72 / 4,69 | `mediaanalysisd` 87 %, Lightroom 51 %, `mds_stores` 23 % |
| tour 1 (17:31 → 18:32 UTC), début → fin | 4,31 → 7,11 | `mediaanalysisd` 101 %, `mds_stores` 23 %, Lightroom 33 % |
| fin (20:40 UTC) | 2,45 / 4,18 / 5,74 | Lightroom 91 % |

Espace disque libre : 43 Gio au début, 45 Gio à la fin. Aucune commande en arrière-plan ; les trois
tours et le contrôle ont tourné au premier plan, un appel de shell par tour.

**Ancien binaire, hors de l'arbre de travail.** Aucune commande git qui modifie le dépôt n'a été
employée (ni `checkout`, ni `stash`, ni `worktree`) :

```
git archive e7d4e08 | tar -x -C data/ab-old/src
cd data/ab-old/src && CARGO_TARGET_DIR=/…/data/ab-old/target \
    cargo build --release --example dense_latency
```

**Nouveau binaire**, arbre de travail :

```
cargo build --release --example dense_latency
```

**Copie du harnais dans l'ancien arbre, et rien d'autre.** Deux ajouts, dans
`data/ab-old/src/crates/crible/examples/dense_latency.rs` seulement, pour que l'A/B soit lisible et
que le contrôle d'ADR-0082 soit possible :

- les métadonnées `segments` et `segment_counts` dans l'en-tête des documents `cross` et `agree`,
  comme le harnais du nouvel arbre les écrit, pour que l'analyseur refuse une copie qui n'a pas le
  nombre de segments annoncé ;
- un **oracle de tri de Hamming complet** dans `agree` (la fonction `full_sort` du harnais nouveau),
  et la comparaison des **deux** parcours de l'ancien binaire à cet oracle.

Le moteur de `e7d4e08` (`crates/*/src`), la réplique du scan (`scan_replica`, `window`, `offer`,
`windows`) et la règle de densité (`traversal_of`) n'ont **pas** été touchés : `git archive` sort le
commit tel quel, et les deux ajouts sont des lectures et une écriture de rapport.

**Mesure.** Six exécutions par copie, trois paires, l'ordre des paires alterné et l'ordre des copies
en carré latin, pour que le coût résiduel du nombre de segments se lise dans un même tour :

| tour | ordre des copies | ordre dans la paire |
|---|---|---|
| 1 | `synth-large`, `-8seg`, `-multi` | ancien → nouveau |
| 2 | `-8seg`, `-multi`, `synth-large` | nouveau → ancien |
| 3 | `-multi`, `synth-large`, `-8seg` | ancien → nouveau |

La ligne de commande est la même pour les deux binaires, et `--pairs 3` y est la passe triple
d'ADR-0077 (`sparse` pour le nouveau, les deux parcours pour l'ancien) :

```
data/ab-old/target/release/examples/dense_latency cross --data data/mes02 --index <copie> \
    --vectors data/mes02/queries.f32 --ns 5000 --pairs 3 --sizes 1000000,all \
    --spans 1000000,7740000 --densities <densités de la copie> --out data/mes02/raw/ab-old-<copie>-r<n>.json
target/release/examples/dense_latency cross --data data/mes02 --index <copie> \
    --vectors data/mes02/queries.f32 --ns 5000 --pairs 3 --sizes 1000000,all \
    --spans 1000000,7740000 --densities <densités de la copie> --out data/mes02/raw/ab-new-<copie>-r<n>.json
```

Les densités sont celles de la campagne, copie par copie, pour que les libellés soient les siens :
`10,25,40,50,60,75,90` sur les copies à 1 et 100 segments, `10,25,40,50,60,75,90,100` sur la copie
compactée, qui seule porte les points de densité 1,0. Cela donne **16, 18 et 16 points**, exactement
les 50 points de `MES-02-multisegments.2026-10-27.json`. N = 5000, 300 requêtes, 3 passes par
exécution, p50 et p95 au rang le plus proche (ADR-0077). Le harnais garde une passe de préchauffage
hors des passes mesurées.

Les recettes sont `tools/bench/mes02/run-ab.sh <tour>` et `tools/bench/mes02/run-ab-control.sh` ;
l'analyseur est `tools/bench/mes02/mes02_ab_analyze.py` :

```
sh tools/bench/mes02/run-ab.sh 1 ; sh tools/bench/mes02/run-ab.sh 2 ; sh tools/bench/mes02/run-ab.sh 3
sh tools/bench/mes02/run-ab-control.sh
(cd tools && uv run --no-sync python bench/mes02/mes02_ab_analyze.py --commit 4a06e7c6fc21d1228453f84407c029b677985982 \
    --date 2026-10-27 --machine "MacBook Air M3, 8 cores, 24 GB, macOS 15.7.7, native")
```

**Latence de l'ancien binaire.** C'est celle du parcours que la règle de densité d'ADR-0071 choisit
sur ce point, c'est-à-dire la série nommée par `traversal_automatic` dans le document : `dense` sur
les pools contigus et sur les densités d'au moins 0,50, `sparse` sur les pools dispersés. L'ancien
binaire enregistre les deux séries ; l'A/B ne lit que celle qui tournait réellement, ce qui est le
point de la décision 2 d'ADR-0082.

**Règle de bruit, décision 3.** Pour chaque point, trois écarts « nouveau contre ancien » en p50,
un par paire (positif : le nouveau est plus lent). L'écart du point est leur **médiane**, son bruit
est leur **amplitude** (max − min). Un point est **perdu** si la médiane est positive et dépasse
l'amplitude ; il est **gagné** si elle est négative et que sa valeur absolue dépasse l'amplitude
(lecture symétrique, ajoutée ici pour le compte rendu, l'ADR ne définissant que « perdu » et « match
nul ») ; il est **match nul** autrement, et un match nul **satisfait** le critère de la tâche.

**Ce que chaque binaire mesure, et pourquoi c'est comparable.** Les deux binaires mesurent une
**réplique du harnais**, pas l'entrée publique du moteur : le moteur n'expose aucun moyen d'imposer
un pool et un parcours, et c'est la raison d'être de `dense_latency.rs`. La réplique suit
`binary.rs` ligne à ligne : mêmes fenêtres (`windows`), même tas borné, même
`kernels::hamming_function`, même règle de densité chez l'ancien (`traversal_of`), même curseur chez
le nouveau (`SegmentCursor`, copie conforme de la boucle de `segment_of` du moteur). La réplique
diffère du moteur sur un point : elle rend `Option` et **saute** un iid que nul segment ne couvre, là
où le moteur rend `Internal` (`binary.rs:269-283`). Cet iid n'existe pas dans les pools de la campagne,
bâtis sur la bitmap des vivants, et la réplique se comporte là comme celle de l'ancien binaire : l'A/B
reste équitable. Trois éléments la rendent fidèle pour cette question : la fonction `windows` est **le
même texte** dans les deux harnais et dans le moteur ; le contrôle `agree` compare chaque réplique à un
tri de Hamming complet, écrit sans code partagé ; les tests de production `scanner_pool_binaire.rs`
exercent le parcours du moteur lui-même, y compris le curseur sur des segments inégaux. Aucun code de
production n'a été modifié pour cette campagne.

### Contrôles

**Mêmes candidats.** `agree` compare chaque parcours mesuré à un tri de Hamming complet du pool, sur
**tous** les pools de la campagne, les 5 premières requêtes et N = 5000. L'ancien binaire compare
ses **deux** parcours à l'oracle, le nouveau son parcours unique :

| binaire | copie | segments | pools | requêtes | identiques |
|---|---|---|---|---|---|
| ancien | synth-large | 1 | 16 | 5 | oui |
| nouveau | synth-large | 1 | 16 | 5 | oui |
| ancien | synth-large-8seg | 8 | 18 | 5 | oui |
| nouveau | synth-large-8seg | 8 | 18 | 5 | oui |
| ancien | synth-large-multi | 100 | 16 | 5 | oui |
| nouveau | synth-large-multi | 100 | 16 | 5 | oui |

Les deux binaires rendent donc les mêmes candidats, mêmes iid, mêmes distances, même ordre, sur les
mêmes pools et les mêmes requêtes : chacun égale le tri complet, donc ils sont égaux entre eux.

**Mêmes pools.** L'analyseur refuse un document dont les libellés, la taille, la plage, le premier et
le dernier iid diffèrent d'un autre document de la même copie. Les six documents d'une copie portent
les mêmes 16 ou 18 pools, dans le même ordre ; la copie à un segment garde la plage de 1 111 111 iid
et la densité 0,90 de la campagne.

**Autres refus de l'analyseur**, et ils n'ont pas été déclenchés : nombre de segments de la copie,
N = 5000, 300 requêtes, 3 passes, `traversal_automatic` présent et valant `dense` ou `sparse`,
`traversal: sparse` côté nouveau, et contrôle d'accord réussi. Les résumés enregistrés par le harnais
sont recalculés depuis les durées par requête (`check_summary`).

**Contrôles supplémentaires de deux anomalies** (documents sous `data/mes02/raw/ab-micro*.json`,
hors du verdict). Deux A/B courts, trois paires, sur l'arbre de travail et l'ancien binaire :
le pool contigu de 1 M de la copie à un segment (ancien 6 826 / 6 938 / 6 972 µs contre nouveau
6 748 / 6 971 / 6 766 µs) et le point `density-7740000-40` de la même copie (ancien 19 228 /
20 105 / 20 202 µs contre nouveau 19 380 / 19 909 / 20 026 µs). Les deux écarts sont dans le bruit
d'un aller-retour de quelques minutes : les valeurs positives du tour 1 viennent de la charge, pas du
code. Ces deux contrôles ne servent qu'à écarter une explication, ils ne remplacent aucune paire.

### Écarts au protocole

1. **Harnais de l'ancien binaire étendu**, comme décrit au protocole : métadonnées de segments et
   oracle de tri complet dans `agree`. Le moteur de `e7d4e08` et la réplique du scan sont intacts.
2. **Charge de la machine, et un tour contaminé.** Le tour 1 (17:31 → 18:32 UTC) s'est déroulé pendant
   une montée de charge de 4,31 à 7,11 (`mediaanalysisd` à 101 %, `mds_stores` à 23 %). Son côté
   nouveau sur la copie à un segment est gonflé de +9 à +23 % sur **tous** les points, y compris ceux
   pour lesquels les deux binaires exécutent le même parcours. Le tour est **conservé** : il n'est ni
   refait ni écarté, et l'amplitude de chaque point l'absorbe (c'est exactement ce que la règle de
   bruit d'ADR-0082 est faite pour traiter). Les deux contrôles supplémentaires ci-dessus montrent que
   l'écart ne se reproduit pas hors de cette fenêtre.
3. **Contrôle d'accord sur 5 requêtes**, comme la campagne multi-segments, et non sur les 300 : le
   tri complet est un tri de 7,74 M d'éléments par requête. La couverture est celle de la campagne.
4. **Densités par copie** : la copie compactée porte deux points de plus (densité 1,0), comme dans la
   campagne, pour que les libellés soient identiques. Aucun point n'est ajouté ni retiré.
5. **Ordre à l'intérieur de la paire, côté ancien.** L'ancien binaire mesure ses deux parcours dans
   chaque paire ; sa série `sparse` suit donc sa série `dense`, alors que le nouveau mesure ses passes
   à la suite. C'est une différence de contexte entre les deux côtés, inhérente à l'A/B : on compare
   ce qui tournait à ce qui tourne, pas deux boucles dans le même processus. Le préchauffage et les
   trois passes de chaque côté limitent son effet, et les contrôles supplémentaires le bornent.
6. **Pas de cache froid.** La question est celle du parcours à chaud, comme la campagne
   multi-segments ; ADR-0077 réserve le froid au rescoring.

### Résultats

**Compte des verdicts.** 50 points : **30 gagnés, 20 matchs nuls,
0 perdus**.

| copie | segments | points | gagnés | matchs nuls | perdus |
|---|---|---|---|---|---|
| synth-large | 1 | 16 | 7 | 9 | 0 |
| synth-large-8seg | 8 | 18 | 13 | 5 | 0 |
| synth-large-multi | 100 | 16 | 10 | 6 | 0 |

Aucun point n'est perdu : la décision 4 d'ADR-0082 (question de conception, tâche `BLOQUE`) n'est pas
déclenchée. Les médianes des **30 points gagnés** vont de **−8,5 % à −37,3 %**, et elles viennent de
deux effets distincts, que la colonne « ancien » des tableaux sépare :

- **24 points que l'ancienne règle envoyait au dense** (pools contigus, et densités d'au moins 0,50 sur
  les deux plages) : le nouveau parcours n'y paie plus le test d'appartenance au bitmap par iid de
  `dense_window`, et il lit les codes dans l'ordre de la bitmap du pool au lieu de balayer la plage.
  Les médianes vont de −8,8 % (`density-1000000-60`, copie à un segment) à −37,3 %
  (`density-7740000-50`, copie à cent segments) ;
- **6 points que l'ancienne règle envoyait déjà au clairsemé**, où le gain vient du seul remplacement
  de `segment_of` par le curseur : `density-1000000-10` **−13,7 %**, `-25` **−14,9 %** et `-40`
  **−11,9 %** à huit segments ; `density-1000000-25` **−12,7 %**, `-40` **−12,7 %** et
  `density-7740000-10` **−8,5 %** à cent segments. C'est l'effet qu'ADR-0082 visait : la recherche
  binaire par iid n'est remplacée par une avance de curseur que là où la table des segments a plus
  d'une entrée, donc le gain n'apparaît pas sur la copie à un segment.

Les **20 matchs nuls** sont de deux sortes : les **12** points déjà clairsemés chez l'ancien binaire,
dont les médianes vont de −13,6 % à +6,7 % et dont les valeurs positives restent dans l'amplitude de
leur propre point ; et les **8** points que l'ancienne règle envoyait au dense, où le nouveau est devant
de −0,4 % à −9,6 % sans que l'écart dépasse cette amplitude (par exemple `contiguous-1000000` à un
segment, −8,4 % de médiane pour une amplitude de 20,7 points).

#### Copie à un segment (`synth-large`, 16 points)

| pool | taille | ancien | p50 ancien (µs) | p50 nouveau (µs) | écart par paire (%) | écart médian | amplitude | p95 nouveau (µs) | écart p95 médian | verdict |
|---|---|---|---|---|---|---|---|---|---|---|
| contiguous-1000000 | 1000000 | dense | 7819 | 7274 | +12.0, -8.7, -8.4 | -8.4 % | 20.7 pts | 8667 | -6.6 % | match nul |
| full | 7740000 | dense | 47131 | 37310 | -11.7, -25.2, -22.0 | -22.0 % | 13.5 pts | 41630 | -20.8 % | gagné |
| density-1000000-10 | 100000 | sparse | 3084 | 3003 | +8.8, -4.2, -6.5 | -4.2 % | 15.3 pts | 3313 | -1.7 % | match nul |
| density-1000000-25 | 250000 | sparse | 4402 | 4415 | +10.9, -1.8, +3.4 | +3.4 % | 12.7 pts | 4862 | -1.3 % | match nul |
| density-1000000-40 | 400000 | sparse | 5008 | 5053 | +8.7, -3.2, +4.8 | +4.8 % | 11.8 pts | 5692 | +4.6 % | match nul |
| density-1000000-50 | 500000 | dense | 6441 | 5631 | -7.9, -17.5, -12.6 | -12.6 % | 9.7 pts | 6343 | -10.8 % | gagné |
| density-1000000-60 | 600000 | dense | 6717 | 5912 | -8.8, -15.4, -8.8 | -8.8 % | 6.6 pts | 6920 | -10.6 % | gagné |
| density-1000000-75 | 750000 | dense | 7109 | 6566 | -4.5, -16.5, -5.6 | -5.6 % | 12.0 pts | 7813 | -6.5 % | match nul |
| density-1000000-90 | 900000 | dense | 7365 | 7334 | +2.1, -4.6, -0.4 | -0.4 % | 6.7 pts | 8672 | -0.7 % | match nul |
| density-7740000-10 | 774000 | sparse | 13387 | 13926 | +22.8, +4.0, +0.7 | +4.0 % | 22.1 pts | 15998 | +3.5 % | match nul |
| density-7740000-25 | 1935000 | sparse | 18462 | 19247 | +8.4, +4.4, +3.8 | +4.4 % | 4.6 pts | 20986 | +3.2 % | match nul |
| density-7740000-40 | 3096000 | sparse | 21445 | 22873 | +11.6, +6.7, +2.1 | +6.7 % | 9.5 pts | 25107 | +10.8 % | match nul |
| density-7740000-50 | 3870000 | dense | 36561 | 25570 | -27.0, -28.7, -34.8 | -28.7 % | 7.8 pts | 29116 | -26.8 % | gagné |
| density-7740000-60 | 4644000 | dense | 38788 | 27078 | -30.2, -26.2, -31.7 | -30.2 % | 5.5 pts | 29992 | -30.1 % | gagné |
| density-7740000-75 | 5805000 | dense | 40491 | 31436 | -27.9, -21.4, -24.1 | -24.1 % | 6.6 pts | 35823 | -22.6 % | gagné |
| density-7740000-90 | 6966000 | dense | 43255 | 35660 | -15.6, -14.8, -19.5 | -15.6 % | 4.7 pts | 40248 | -11.3 % | gagné |

#### Copie à huit segments (`synth-large-8seg`, 18 points)

| pool | taille | ancien | p50 ancien (µs) | p50 nouveau (µs) | écart par paire (%) | écart médian | amplitude | p95 nouveau (µs) | écart p95 médian | verdict |
|---|---|---|---|---|---|---|---|---|---|---|
| contiguous-1000000 | 1000000 | dense | 7390 | 7099 | -9.0, -8.8, -0.3 | -8.8 % | 8.7 pts | 8340 | -9.6 % | gagné |
| full | 7740000 | dense | 43597 | 36556 | -28.4, -17.3, -14.5 | -17.3 % | 13.9 pts | 40114 | -20.5 % | gagné |
| density-1000000-10 | 100000 | sparse | 3220 | 2781 | -20.9, -13.7, -12.9 | -13.7 % | 7.9 pts | 2984 | -16.0 % | gagné |
| density-1000000-25 | 250000 | sparse | 4518 | 4013 | -14.9, -15.4, -3.9 | -14.9 % | 11.5 pts | 4438 | -14.7 % | gagné |
| density-1000000-40 | 400000 | sparse | 5118 | 4713 | -13.5, -11.9, -5.3 | -11.9 % | 8.2 pts | 5465 | -7.9 % | gagné |
| density-1000000-50 | 500000 | dense | 6545 | 5206 | -26.0, -24.9, -16.9 | -24.9 % | 9.1 pts | 6122 | -24.0 % | gagné |
| density-1000000-60 | 600000 | dense | 6919 | 5674 | -29.5, -21.5, -14.4 | -21.5 % | 15.0 pts | 6708 | -22.3 % | gagné |
| density-1000000-75 | 750000 | dense | 7239 | 6406 | -21.4, -16.0, -7.0 | -16.0 % | 14.5 pts | 7757 | -16.1 % | gagné |
| density-1000000-90 | 900000 | dense | 7486 | 7087 | -16.2, -9.6, -3.4 | -9.6 % | 12.9 pts | 8536 | -9.9 % | match nul |
| density-1000000-100 | 1000000 | dense | 7588 | 7625 | -8.5, -4.4, +1.9 | -4.4 % | 10.5 pts | 8921 | -3.9 % | match nul |
| density-7740000-10 | 774000 | sparse | 14032 | 13219 | -13.3, -20.5, -1.9 | -13.3 % | 18.6 pts | 15350 | -11.4 % | match nul |
| density-7740000-25 | 1935000 | sparse | 19117 | 19151 | -9.5, -3.2, +0.9 | -3.2 % | 10.4 pts | 20320 | -4.9 % | match nul |
| density-7740000-40 | 3096000 | sparse | 22933 | 22701 | -9.9, +1.7, -0.8 | -0.8 % | 11.7 pts | 24512 | -2.9 % | match nul |
| density-7740000-50 | 3870000 | dense | 38883 | 24661 | -38.4, -33.8, -33.5 | -33.8 % | 4.8 pts | 27253 | -33.7 % | gagné |
| density-7740000-60 | 4644000 | dense | 41884 | 26705 | -37.2, -34.6, -31.8 | -34.6 % | 5.4 pts | 30019 | -33.5 % | gagné |
| density-7740000-75 | 5805000 | dense | 42422 | 30962 | -32.6, -27.0, -22.7 | -27.0 % | 10.0 pts | 35202 | -24.2 % | gagné |
| density-7740000-90 | 6966000 | dense | 43331 | 35572 | -26.6, -17.2, -14.8 | -17.2 % | 11.7 pts | 40083 | -13.4 % | gagné |
| density-7740000-100 | 7740000 | dense | 43959 | 38226 | -21.3, -12.4, -12.3 | -12.4 % | 9.0 pts | 42285 | -15.2 % | gagné |

#### Copie à cent segments (`synth-large-multi`, 16 points)

| pool | taille | ancien | p50 ancien (µs) | p50 nouveau (µs) | écart par paire (%) | écart médian | amplitude | p95 nouveau (µs) | écart p95 médian | verdict |
|---|---|---|---|---|---|---|---|---|---|---|
| contiguous-1000000 | 1000000 | dense | 7493 | 7188 | -12.6, -3.1, -2.6 | -3.1 % | 9.9 pts | 8522 | -1.0 % | match nul |
| full | 7740000 | dense | 48675 | 37112 | -26.3, -24.7, -17.9 | -24.7 % | 8.4 pts | 41477 | -23.2 % | gagné |
| density-1000000-10 | 100000 | sparse | 3319 | 2983 | -25.3, -7.2, -13.6 | -13.6 % | 18.1 pts | 3208 | -15.0 % | match nul |
| density-1000000-25 | 250000 | sparse | 4623 | 4247 | -13.7, -2.6, -12.7 | -12.7 % | 11.1 pts | 4532 | -13.1 % | gagné |
| density-1000000-40 | 400000 | sparse | 5470 | 4862 | -14.2, -6.4, -12.7 | -12.7 % | 7.7 pts | 5490 | -12.8 % | gagné |
| density-1000000-50 | 500000 | dense | 6530 | 5389 | -7.5, -15.9, -21.8 | -15.9 % | 14.3 pts | 6309 | -16.0 % | gagné |
| density-1000000-60 | 600000 | dense | 6860 | 5772 | -16.2, -13.9, -16.5 | -16.2 % | 2.7 pts | 6724 | -17.8 % | gagné |
| density-1000000-75 | 750000 | dense | 7243 | 6493 | -21.0, -9.4, -9.4 | -9.4 % | 11.6 pts | 7823 | -7.2 % | match nul |
| density-1000000-90 | 900000 | dense | 7602 | 7188 | -11.7, -3.6, -5.8 | -5.8 % | 8.2 pts | 8642 | -3.9 % | match nul |
| density-7740000-10 | 774000 | sparse | 15386 | 13710 | -10.9, -8.4, -8.5 | -8.5 % | 2.5 pts | 15777 | -9.1 % | gagné |
| density-7740000-25 | 1935000 | sparse | 19735 | 19603 | -6.0, -0.7, +0.0 | -0.7 % | 6.0 pts | 20971 | -1.6 % | match nul |
| density-7740000-40 | 3096000 | sparse | 24228 | 23037 | -12.1, -4.9, -4.7 | -4.9 % | 7.4 pts | 25021 | -6.7 % | match nul |
| density-7740000-50 | 3870000 | dense | 38723 | 24750 | -38.4, -33.5, -37.3 | -37.3 % | 4.9 pts | 27382 | -36.9 % | gagné |
| density-7740000-60 | 4644000 | dense | 40290 | 27061 | -32.6, -32.2, -35.8 | -32.6 % | 3.6 pts | 30285 | -34.8 % | gagné |
| density-7740000-75 | 5805000 | dense | 41348 | 31288 | -20.9, -24.3, -26.1 | -24.3 % | 5.2 pts | 35340 | -22.3 % | gagné |
| density-7740000-90 | 6966000 | dense | 42268 | 35666 | -13.1, -14.5, -17.7 | -14.5 % | 4.6 pts | 40244 | -12.7 % | gagné |

#### Coût résiduel du nombre de segments (nouveau binaire)

Écart de p50 du **nouveau** binaire entre copies, lu dans le même tour, sur les 16 points que les
trois copies partagent ; médiane des trois tours et amplitude entre parenthèses. La copie à un
segment a 8,6 M de passages et une densité de 0,90 ; les deux autres, 7,74 M et une densité de 1,0 :
comme dans la campagne, l'écart mêle donc le découpage et le travail en moins des copies compactées,
et il est un **minorant** du surcoût du découpage là où la copie compactée a moins de données.

| pool | taille | 8 vs 1 | 100 vs 1 | 100 vs 8 | amplitudes 8/1, 100/1, 100/8 (pts) |
|---|---|---|---|---|---|
| contiguous-1000000 | 1000000 | -7.4 % | -0.2 % | +0.3 % | 14.0 / 16.5 / 10.3 |
| full | 7740000 | -3.3 % | +0.1 % | +1.5 % | 7.5 / 11.1 / 5.9 |
| density-1000000-10 | 100000 | -7.4 % | +0.2 % | +6.6 % | 10.0 / 23.9 / 16.9 |
| density-1000000-25 | 250000 | -9.1 % | -6.7 % | -0.6 % | 6.1 / 9.5 / 14.3 |
| density-1000000-40 | 400000 | -7.6 % | -7.1 % | +0.3 % | 2.2 / 7.0 / 9.8 |
| density-1000000-50 | 500000 | -7.6 % | -0.1 % | +9.7 % | 4.2 / 13.3 / 17.5 |
| density-1000000-60 | 600000 | -6.0 % | -2.8 % | +1.3 % | 3.2 / 6.1 / 7.9 |
| density-1000000-75 | 750000 | -2.4 % | -1.1 % | +1.3 % | 4.8 / 1.7 / 7.0 |
| density-1000000-90 | 900000 | -4.2 % | -2.4 % | +1.4 % | 3.7 / 3.0 / 6.6 |
| density-7740000-10 | 774000 | -5.8 % | -1.8 % | +3.7 % | 21.0 / 6.0 / 20.0 |
| density-7740000-25 | 1935000 | -0.1 % | +1.9 % | +2.0 % | 6.3 / 2.6 / 4.0 |
| density-7740000-40 | 3096000 | +0.6 % | +0.7 % | +0.5 % | 4.6 / 2.1 / 2.9 |
| density-7740000-50 | 3870000 | -1.3 % | -3.2 % | -1.6 % | 6.9 / 3.5 / 4.2 |
| density-7740000-60 | 4644000 | -1.4 % | -0.1 % | -0.0 % | 3.8 / 5.4 / 2.9 |
| density-7740000-75 | 5805000 | -1.5 % | -0.5 % | +1.1 % | 3.9 / 1.2 / 2.7 |
| density-7740000-90 | 6966000 | +0.6 % | -1.1 % | +0.7 % | 5.1 / 3.7 / 3.9 |

Deux lectures :

- **le coût du découpage a disparu.** « 100 vs 1 » va de **−7,1 % à +1,9 %** en médiane selon le pool
  (16 points), et « 8 vs 1 » de −9,1 % à +0,6 %. La campagne multi-segments mesurait, sur le même
  protocole entrelacé, **+6,5 % à +32,9 %** pour « 100 vs 1 » et +3,2 à +3,6 % dès 8 segments sur les
  pools comparables. Du côté positif, l'écart « 100 vs 1 » ne dépasse plus **+1,9 %**, donc il
  retombe dans le bruit du témoin dense de la campagne (±2,6 %) ; les écarts négatifs, jusqu'à
  −7,1 %, vont dans l'autre sens et ne sont pas un coût. Le seuil qu'ADR-0081 avait écrit (« quelques
  pour cent sur une copie à 8 segments ») n'est plus atteint ;
- **ramené au passage vivant**, l'écart « 100 vs 1 » vaut de −0,21 ns à +0,005 ns selon le pool, avec
  les quatre pools de la campagne : −0,01 ns (1 M contigu), −0,01 ns (densité 0,50 sur 1 M), −0,21 ns
  (densité 0,50 sur 7,74 M) et +0,005 ns (pool complet). Le coût par passage que `segment_of`
  facturait (0,93 à 1,03 ns) n'est plus mesurable ; le signe négatif de trois valeurs est dans le
  bruit de la mesure, pas un gain.

#### Budget de `docs/03-recherche.md:144` (nouveau binaire, p50 et p95)

Rapporté sans jugement : la décision 5 d'ADR-0082 laisse le percentile de « quelques ms » à P8,
mesuré en conteneur.

| copie | segments | pool | N | p50 médian (ms) | p95 médian (ms) | p50 min–max (ms) | p95 min–max (ms) |
|---|---|---|---|---|---|---|---|
| synth-large | 1 | contiguous-1000000 | 5000 | 7.27 | 8.67 | 7.16–8.25 | 8.34–9.59 |
| synth-large | 1 | full | 5000 | 37.31 | 41.63 | 36.75–39.90 | 40.49–49.42 |
| synth-large-8seg | 8 | contiguous-1000000 | 5000 | 7.10 | 8.34 | 6.74–7.17 | 7.86–8.48 |
| synth-large-8seg | 8 | full | 5000 | 36.56 | 40.11 | 36.07–36.71 | 39.71–40.43 |
| synth-large-multi | 100 | contiguous-1000000 | 5000 | 7.19 | 8.52 | 6.92–7.26 | 7.87–8.59 |
| synth-large-multi | 100 | full | 5000 | 37.11 | 41.48 | 35.86–37.35 | 38.24–41.92 |

Sur le pool de 1 M, le nouveau binaire donne **7,10 à 7,27 ms** en p50 et **8,34 à 8,67 ms** en p95
selon la copie (au pire relevé : 9,59 ms en p95, tour 1 de la copie à un segment, sous la charge
décrite aux écarts au protocole, point 2) ; sur le pool complet, **36,6 à 37,3 ms** en p50 et **40,1 à 41,6 ms** en p95. La campagne
multi-segments donnait 6,8 à 8,6 ms en p50 et **10,3 à 10,5 ms en p95** sur le pool de 1 M des copies
multi-segments, et 35,5 à 48,1 / 39,0 à 59,9 ms sur le pool complet. Aucun point du pool de 1 M ne
dépasse 10 ms en p95 dans les neuf exécutions.

### Conclusion

**Le critère d'ADR-0082 est tenu : aucun point perdu sur les 50 de la campagne, 30 points gagnés et
20 matchs nuls.** Aucun point ne déclenche la décision 4 de l'ADR,
donc **aucune question de conception n'est remontée** et la tâche peut être déclarée `FAIT` sur ce
critère.

Les trois réponses que la campagne d'aujourd'hui avait laissées ouvertes :

1. **Le curseur a supprimé le coût du nombre de segments.** Le parcours unique n'est plus ralenti par
   le découpage : « 100 vs 1 » va de −7,1 % à +1,9 % de médiane, contre +6,5 % à +32,9 % avant, et le
   coût par passage n'est plus mesurable. La prédiction d'ADR-0082 (« si l'écart ne retombe pas dans
   le bruit du témoin dense, le curseur n'a pas supprimé le coût ») est vérifiée du côté positif, le
   seul où un écart serait un coût : il y retombe.
2. **Le parcours unique n'ajoute pas de régression sur les points que l'ancienne règle envoyait déjà
   au clairsemé** : leurs médianes vont de −14,9 % à +6,7 % et toutes les valeurs positives sont dans
   l'amplitude de leur point ; les deux contrôles ciblés ne reproduisent aucun écart à ce sujet.
3. **Le budget mesuré est plus bas que celui de la campagne** : p95 de 8,34 à 8,67 ms sur le pool de
   1 M et de 40,1 à 41,6 ms sur le pool complet, contre 10,3 à 10,5 ms et 39,0 à 59,9 ms. La lecture
   du budget reste à P8 (ADR-0082, décision 5) ; aucun chiffre n'est proposé ici comme seuil.

**Ce que la mesure ne tranche pas.** Elle ne dit rien des petits pools, du régime au-delà de 100
segments, ni du p95 en conteneur. Elle repose sur trois paires par copie, dont une sous une charge
anormale : l'amplitude de plusieurs points atteint 10 à 24 points de pourcentage, et un écart de
quelques pour cent sur un point isolé n'est pas interprétable. Enfin, elle compare des **répliques**
du parcours, pas l'entrée publique du moteur, comme toute la série MES-02 ; la fidélité de la
réplique repose sur la copie ligne à ligne de `binary.rs`, sur le contrôle contre un tri complet et
sur les tests de production du parcours.

**Note de forme.** La section est ajoutée à la fin de ce rapport, avec son propre artefact
(`docs/mesures/MES-02-ab-parcours-unique.2026-10-27.json`, 70 Ko, sans les durées par requête) et ses
sorties brutes sous `data/mes02/raw/ab-*.json`. L'ancien arbre et son binaire restent sous
`data/ab-old/` (git ignoré), pour que la même A/B soit rejouable.
