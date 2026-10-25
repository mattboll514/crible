# MES-13 — Temps de démarrage : reconstruction des structures de filtrage depuis `records.sqlite`

> **Premier chiffre, sans cache (P3).** Le cache des bitmaps n'existe pas encore :
> il est l'objet de la seconde moitié de MES-13, en P3 (`MESURER-CYCLE-DE-VIE`).
> Ce rapport ne mesure donc que la **reconstruction** des structures de filtrage à
> l'ouverture d'un index, et non la comparaison avec un chargement de cache.

## 1. Campagne du 2026-10-22

- Tâche : `CONSTRUIRE-BITMAPS` (phase P1), qui cite « MES-13 (premier chiffre) ».
  Mesure demandée par le propriétaire avant la validation du jalon J1.
- Références : `docs/08-invariants-et-tests.md` (MES-13),
  `docs/02-architecture.md` (« Reprise au démarrage »),
  `docs/phases/P3-cycle-de-vie.md` (critère des 60 s),
  `docs/03-recherche.md` (« Filtres → pool »), ADR-0018, ADR-0019, ADR-0035,
  `crates/crible-store/src/filters_index.rs`,
  `crates/crible-store/src/generation.rs` (`GenerationStore::open`,
  `build_snapshot`).
- Commit : `6dbb22dc5be87dea7979f69ca03efaef11aabbb1`
  (branche `phase/P1-stockage-et-recherche-exacte`). **Aucun code de production
  n'a été modifié pendant la campagne.** Seuls ont été ajoutés le banc
  `crates/crible-store/benches/startup_cost.rs`, son déclencheur `[[bench]]` dans
  `crates/crible-store/Cargo.toml` et les scripts `tools/bench/mes13/`. Aucune
  dépendance nouvelle : le banc emploie `rusqlite`, `serde_json`, `crible-core`
  et `crible-store`, déjà déclarés.
- Machine : MacBook Air Apple M3 (8 cœurs : 4 performance, 4 efficacité), 24 Go de
  RAM (25 769 803 776 octets), macOS 15.7.7 (24G720), APFS, 142 Go libres.
  Mesure **native**, aucun conteneur. Le banc est un binaire de
  `crates/crible-store/benches/`, compilé par `cargo bench` (profil `bench`,
  optimisations actives).
- Charge ambiante : la machine n'était **pas au repos**. La moyenne de charge est
  passée de 4,06 à 3,01 sur 8 cœurs entre le début et la fin de la campagne
  (`data/mes13/raw/conditions-start.txt` et `conditions-end.txt`). Le runtime du
  modèle local qui sert cet agent (`/Applications/Ollama.app`) et Docker Desktop
  tournaient ; je n'ai pas vérifié si un conteneur était actif, et leur part dans
  la charge n'a pas été isolée. La
  campagne elle-même n'a lancé aucun autre travail lourd. Les p95 et p99 sont
  très proches des p50, donc l'effet de cette charge est faible sur ces chiffres,
  sans être mesuré.
- Jeux de données : un index attaché et deux fixtures à l'échelle. Les tailles de
  fichiers sont décimales (Mo = 10⁶ octets) ; les mémoires sont en Mio
  (1 Mio = 1 048 576 octets). Les SHA-256 portent sur le `records.sqlite`
  **mesuré**, relevés hors banc par `shasum -a 256`.

| Point | Lignes `passages` | Vivants | Documents | Origine | `records.sqlite` | SHA-256 |
|---|---:|---:|---:|---|---:|---|
| `synth-small` | 12 000 | 9 750 | 1 950 | `crible attach` l1+l2+l3 | 4,25 Mo | `f341df0d21e0ad13b4233016b224bdf6f5c869826b2a12db57b73163066ad229` |
| `fixture-1m` | 1 000 000 | 1 000 000 | 200 000 | banc MES-14, `prepare` | 213,8 Mo | `5ce1be32488c69560c86dfaa7bdad9443c6680211bfa40745ea083cbb142d4f4` |
| `fixture-8m58` | 8 580 000 | 8 580 000 | 1 716 000 | banc MES-14, `prepare` | 1 849,3 Mo | `d53184ac6821267a07cdf030aa15370579f9695d729b1d5434b304e809d68c66` |

  - `synth-small` est l'index de référence de J1 : génération 3, 1 950 documents,
    9 750 passages vivants et 2 250 pierres tombales, deux segments (`seg-l1`,
    `seg-l2`). Somme de contrôle du jeu, calculée à la main avec
    `gen_synth.dataset_checksum` : `blake3:2d3d8265a8df350d816bccb5732fe338921d275cae5d091a96d0497646e58805`.
    C'est encore la valeur consignée dans `tools/README.md`. ADR-0045 (statut
    `Proposé`) annonce qu'elle changera à l'application de la référence sans
    BLAS ; cette somme couvre `expected.jsonl`, pas l'index, et n'a donc **aucun
    effet** sur les chiffres ci-dessous. Le `records.sqlite` attaché est identique
    à celui qu'a mesuré MES-14 (même SHA-256).
  - Les deux fixtures à l'échelle sont celles de MES-14 : même schéma que
    `synth-small` (11 champs : 6 de document, 5 de passage), un document pour
    cinq passages, 5 à 10 % de nuls, aucune pierre tombale, **aucune ligne dans
    `segments`** et aucun fichier de vecteurs. Ce sont des dispositifs de mesure :
    `FilterIndex` ne lit que `passages` et `documents`.
  - L'index de 8,58 M de MES-15 (`data/mes15/i-8580000-s10`) n'a **pas** été
    employé : il ne porte pas d'`index.json`, donc `GenerationStore::open` le
    refuse (voir § 8). La fixture de MES-14 a l'avantage d'un schéma plus riche en
    champs filtrables, et son `records.sqlite` est ouvert par le même code.

- Configuration mesurée, par point : les quatre phases du § 2, `--repeats 5`,
  `--warmup 1`. Aucun réglage de `N`, de seuil exact ou de précision n'entre dans
  cette mesure : elle ne lit que `records.sqlite`, jamais un vecteur.

## 2. Protocole

Le banc est `crates/crible-store/benches/startup_cost.rs` (`harness = false`). Le
pilote complet est `tools/bench/mes13/run.sh`, qui enchaîne compilation, copies,
campagne, tour froid, second tour et fusion. Commandes exactes :

```sh
# compilation du banc avant la campagne : aucune mesure ne paie rustc
cargo bench -p crible-store --bench startup_cost --no-run

# copie de travail (clone APFS, instantané) : les fixtures de MES-14 restent intactes
cp -cR data/indexes/synth-small data/mes13/synth-small
cp -cR data/mes14/fixture-1m data/mes13/fixture-1m
cp -cR data/mes14/fixture-8m58 data/mes13/fixture-8m58

# cache chaud : 1 ouverture non chronométrée, puis 5 passes des quatre phases
cargo bench -p crible-store --bench startup_cost -- measure \
    --index "$PWD/data/mes13/fixture-8m58" --label fixture-8m58 \
    --repeats 5 --warmup 1 \
    --commit 6dbb22dc5be87dea7979f69ca03efaef11aabbb1 \
    --machine "Apple M3 (8 cores) / 24 GB / macOS 15.7.7" \
    --date 2026-10-22 --out "$PWD/data/mes13/raw/fixture-8m58.json"

# premier accès : une copie neuve par phase, chronométrée une fois
cargo bench -p crible-store --bench startup_cost -- cold \
    --index "$PWD/data/mes13/fixture-8m58" --label fixture-8m58 \
    --work "$PWD/data/mes13/cold-work" \
    --commit 6dbb22dc5be87dea7979f69ca03efaef11aabbb1 \
    --machine "Apple M3 (8 cores) / 24 GB / macOS 15.7.7" \
    --date 2026-10-22 --out "$PWD/data/mes13/raw/fixture-8m58-cold.json"

# fusion en un seul fichier de données brutes, à côté de ce rapport
python3 tools/bench/mes13/merge.py \
    docs/mesures/MES-13-temps-de-demarrage.2026-10-22.json \
    data/mes13/raw/synth-small.json data/mes13/raw/synth-small-cold.json \
    data/mes13/raw/fixture-1m.json data/mes13/raw/fixture-1m-cold.json \
    data/mes13/raw/fixture-8m58.json data/mes13/raw/fixture-8m58-cold.json \
    data/mes13/raw/round2-*.json
```

### Ce qui est reconstruit

`GenerationStore::open` fait, dans cet ordre : lire `CURRENT_EPOCH`, ouvrir
SQLite, lire `meta`, vérifier le schéma déclaré par `index.json`, lancer la
reprise d'INV-04 (suppression des `tmp-*` et des segments orphelins), construire
le snapshot, écrire le manifeste s'il manque, publier (`ArcSwap`). Seule la
construction du snapshot dépend du nombre de lignes, et elle se décompose en :

1. **lecture SQLite** : un `SELECT` de toutes les lignes de `passages` jointes à
   leur ligne `documents`, ordonné par `iid` ;
2. **décodage JSON** : `serde_json` sur les champs canoniques du passage et, quand
   elle existe, du document (ADR-0019) ;
3. **construction** : insertion de chaque valeur dans la bitmap de sa valeur
   (`keyword`), dans la bitmap `bool`, ou dans la colonne de valeurs, plus la
   bitmap « présent » de chaque champ et la bitmap des vivants. Les structures
   couvrent **toutes** les lignes, pierres tombales comprises ; la bitmap des
   vivants porte la distinction. Le coût est donc par ligne, pas par passage
   vivant.

Quatre phases sont chronométrées, chacune sur la même base :

| Phase | Ce qu'elle ajoute |
|---|---|
| `sql_read` | le `SELECT` et la matérialisation des colonnes, sans décodage |
| `json_decode` | le même `SELECT` plus le décodage `serde_json` des deux JSON |
| `build` | `FilterIndex::from_records`, donc le décodage plus les insertions |
| `open` | `GenerationStore::open` entier, donc `build` plus l'ouverture SQLite, `meta`, la reprise, les métadonnées de segments et le manifeste |

Les deux premières phases sont des **répliques** : le SQL de
`FilterIndex::from_records` y est recopié (la fonction est privée), et la
connexion est ouverte par le banc parce que `RecordStore::connection` est
`pub(crate)`. Une réplique mesure une copie du code de production, pas le code
lui-même. La phase `build` appelle bien `FilterIndex::from_records`, et `open`
appelle bien `GenerationStore::open`.

- **Cache chaud** : une ouverture complète non chronométrée précède les cinq
  passes, pour que toutes partent du même état de cache de pages. Les phases se
  suivent dans l'ordre du tableau : `sql_read` réchauffe les pages pour les
  phases suivantes. C'est l'état d'un redémarrage de service sur une machine qui
  a déjà lu son index, pas un premier démarrage après amorçage.
- **Cache froid** : macOS ne purge pas son cache de pages sans `root`. Le mode
  `cold` copie l'index (`cp -cR`, clone APFS) une fois **par phase** et
  chronomètre le premier accès de cette copie. C'est une **approximation** : rien
  ne prouve que les blocs partagés par le clone ne sont pas déjà en cache, et le
  rapport ne présente pas ces valeurs comme un froid contrôlé.
- **Répétitions et percentiles** : 5 répétitions par phase et par point ; p50,
  p95 et p99 par **rang le plus proche**. Sur 5 échantillons, p95 et p99 valent
  le maximum : ces deux colonnes mesurent la queue de la campagne, pas une queue
  de production. C'est la convention de MES-15 et de MES-14, reprise ici.
- **Second tour** : les trois points chauds rejoués intégralement le même jour,
  même banc, mêmes copies (§ 7).

## 3. Résultats — cache chaud

En millisecondes, 5 répétitions par phase et par point.

| Point | Lignes | `sql_read` p50 | `json_decode` p50 | `build` p50 | `open` p50 | `open` p95 | `open` p99 |
|---|---:|---:|---:|---:|---:|---:|---:|
| `synth-small` | 12 000 | 5,7 | 18,9 | 23,5 | **23,6** | 24,1 | 24,1 |
| `fixture-1m` | 1 000 000 | 442,9 | 1 542,7 | 1 976,4 | **2 020,7** | 2 026,7 | 2 026,7 |
| `fixture-8m58` | 8 580 000 | 6 473,4 | 15 885,7 | 20 161,3 | **20 124,3** | 20 151,7 | 20 151,7 |

Le minimum et le maximum de `open` : 23,2 et 24,1 ms (`synth-small`), 1 963 et
2 027 ms (`fixture-1m`), 20 086 et 20 152 ms (`fixture-8m58`).

**Chiffre clé** : à 8 580 000 lignes, l'ouverture complète d'un index coûte
**20,12 s en p50** (20,15 s en p95 et p99) à cache chaud. À l'échelle de
`synth-small`, elle coûte **23,6 ms**.

## 4. Décomposition

Chaque terme est la p50 de sa phase. « Décodage » est `json_decode − sql_read`,
« insertion » est `build − json_decode` ; le pourcentage est la part de `build`.
« Reste de `open` » est `open − build` : ouverture SQLite, `meta`, reprise,
métadonnées de segments et manifeste.

| Point | Lignes | Lecture SQLite | Décodage JSON | Insertion | Reste de `open` |
|---|---:|---:|---:|---:|---:|
| `synth-small` | 12 000 | 5,7 ms (24,2 %) | 13,2 ms (55,6 %) | 4,7 ms (19,7 %) | +0,1 ms |
| `fixture-1m` | 1 000 000 | 442,9 ms (21,9 %) | 1 099,8 ms (54,4 %) | 433,7 ms (21,5 %) | +44,3 ms (2,2 %) |
| `fixture-8m58` | 8 580 000 | 6 473,4 ms (32,2 %) | 9 412,3 ms (46,8 %) | 4 275,6 ms (21,2 %) | −37,0 ms (−0,2 %) |

Par ligne, en microsecondes :

| Point | Lecture SQLite | Décodage JSON | Insertion | `open` entier |
|---|---:|---:|---:|---:|
| `synth-small` (12 000) | 0,48 | 1,10 | 0,39 | 1,97 |
| `fixture-1m` | 0,44 | 1,10 | 0,43 | 2,02 |
| `fixture-8m58` | 0,75 | 1,10 | 0,50 | 2,35 |

Ce que ces chiffres montrent :

1. **Le décodage JSON est le premier poste** : environ 47 à 56 % de la
   construction, et 1,10 µs par ligne aux deux échelles, donc stable. Le
   `from_records` actuel décode les champs du document à **chaque passage** qui
   l'hérite : la fixture a un document pour cinq passages, donc le même JSON de
   document est décodé cinq fois. C'est une lecture du code et de la
   construction de la fixture, pas une réplique ; le gain d'un cache par
   document n'est pas mesuré.
2. **La lecture SQLite est le second poste** : 32,2 % à 8,58 M, et son coût par
   ligne passe de 0,44 à 0,75 µs entre 1 M et 8,58 M (+70 %), alors que le
   décodage reste à 1,10 µs. La cause n'est pas isolée : la fixture de 1 M tient
   dans un cache plus favorable, et la jointure répète le JSON du document.
3. **L'insertion représente environ un cinquième** : 21 à 22 % aux deux échelles
   à l'échelle, 0,39 à 0,50 µs par ligne. Elle ne domine pas.
4. **Le reste de `open` est négligeable** : −37 ms à +44 ms selon le point, dans
   la dispersion. La fixture n'a **aucun segment** ; MES-15 a mesuré la relecture
   des `segment.json` à 0,36 ms pour 10 segments et 12,8 à 15,2 ms pour 1 000
   segments. À l'échelle de Legifacile, ce terme reste donc sous quelques
   dizaines de millisecondes tant que `S` reste sous quelques centaines : c'est
   une borne issue d'une autre mesure, pas un terme de celle-ci.

**Linéarité.** De 1 M à 8,58 M lignes, `open` passe de 2,02 à 20,12 s, soit
9,96 fois pour 8,58 fois plus de lignes : 16 % de surcoût par ligne. La loi est
donc linéaire à 16 % près sur cette plage, sans extrapolation au-delà. Diviser
par le nombre de lignes ne donne pas un modèle pour un autre schéma : le coût
dépend des champs et de leur cardinalité.

**Lignes mortes.** La reconstruction lit toutes les lignes. `synth-small` le
montre : 12 000 lignes lues pour 9 750 vivants. Un index de Legifacile à
8,58 M vivants et au seuil de compaction de 10 % (ADR-0006) compterait environ
9,53 M lignes ; à 2,35 µs par ligne, cela ferait **≈ 22,4 s** — calculé par la
règle linéaire mesurée, pas mesuré.

## 5. Mémoire des structures

`FilterIndex::memory`, la valeur que `crible inspect` rapporte
(`bitmaps: N bytes (live …, keywords …, booleans …, columns …)`). Elle compte
les octets sérialisés des bitmaps, les octets des colonnes et le texte des
valeurs `keyword` ; elle ignore les en-têtes `Arc`, les nœuds des `BTreeMap` et
le schéma, fixes ou petits devant le total.

| Point | Vivants | `live` | `keywords` | `booleans` | `columns` | Total | Par ligne |
|---|---:|---:|---:|---:|---:|---:|---:|
| `synth-small` | 9 750 | 8 208 o | 88 835 o | 45 446 o | 529 248 o | **671 737 o (656 Kio)** | 56,0 o |
| `fixture-1m` | 1 000 000 | 131 208 o | 3 563 813 o | 785 496 o | 40 787 248 o | **45 267 765 o (43,2 Mio)** | 45,3 o |
| `fixture-8m58` | 8 580 000 | 1 074 208 o | 30 064 429 o | 6 445 248 o | 349 645 248 o | **387 229 133 o (369,3 Mio)** | 45,1 o |

Sur `synth-small`, la valeur du banc est **exactement** celle que
`crible inspect --data data --index synth-small` imprime, ce qui valide la
mesure. À 8,58 M, les colonnes de valeurs portent 90,3 % du total
(333,4 Mio sur 369,3 Mio) : c'est le prix d'une colonne indexée par iid pour six
champs de type `integer`, `float` et `date` dans ce schéma. La mémoire ne dépend
pas du nombre de valeurs distinctes, mais du nombre de lignes et du nombre de
champs. Un schéma de Legifacile différent donnera un autre total.

## 6. Cache froid — approximation

Une copie neuve par phase, chronométrée une fois. Ce n'est **pas** un froid
contrôlé : le cache de pages de macOS n'est pas purgé sans `root`, et un clone
APFS partage ses blocs avec l'original.

| Point | `sql_read` | `json_decode` | `build` | `open` | `open` contre chaud |
|---|---:|---:|---:|---:|---:|
| `synth-small` | 14,1 ms | 26,5 ms | 33,5 ms | 32,7 ms | +38,3 % |
| `fixture-1m` | 752,8 ms | 1 872,8 ms | 2 342,2 ms | 2 307,1 ms | +14,2 % |
| `fixture-8m58` | 6 820,5 ms | 16 266,6 ms | 20 331,7 ms | 20 384,9 ms | +1,3 % |

À 8,58 M, le premier accès coûte **20,38 s**, soit 261 ms de plus que le même
`open` à cache chaud. La lecture SQLite ne gagne que 347 ms. Autrement dit, à
cette échelle, l'ouverture est **dominée par le calcul**, pas par l'entrée/sortie :
le décodage et l'insertion coûtent 13,5 s des 20,4 s. À `synth-small`, l'écart de
38 % porte sur des dizaines de millisecondes et mesure surtout le bruit du
système de fichiers et de l'ordonnanceur.

Ce que cette approximation ne permet pas d'affirmer : le coût d'un vrai premier
démarrage après amorçage, quand les pages de `records.sqlite` sont réellement sur
disque et que le cache disque de la machine est froid. Pour le borner, on peut
citer MES-15 (§ 7.3) : la même requête de lecture des iid sur un fichier de
2,88 Go a coûté 2,26 s à froid contre 0,25 s à chaud. Ici, la lecture complète
coûte 6,47 s à chaud ; un froid réel pourrait donc ajouter au plus quelques
secondes, pas un facteur.

## 7. Répétabilité

Second tour intégral, même banc, même jour, mêmes copies. Écart des p50 du
second tour contre la campagne :

| Point | `sql_read` | `json_decode` | `build` | `open` |
|---|---:|---:|---:|---:|
| `synth-small` | −2,9 % | +0,4 % | −0,5 % | +0,7 % |
| `fixture-1m` | +2,0 % | +0,6 % | +1,8 % | −0,2 % |
| `fixture-8m58` | −0,2 % | +0,1 % | −0,2 % | 0,0 % |

Les trois points se suivent à 2 % près, sauf `synth-small` sur la lecture SQLite,
où quelques dixièmes de milliseconde suffisent à faire 3 %. Une répétition du
point à 8,58 M est sortie à 17,86 s d'`open` (contre 20,15 s aux quatre autres) :
c'est un maximum d'écart de 11 %, non expliqué, et il ne change pas la p50. Les
chiffres du § 3 sont donc reproductibles au pourcent à l'échelle.

## 8. Conclusion

Ce que la mesure permet d'affirmer :

1. **À 8,58 M lignes et sur le schéma de `synth-small`, le démarrage coûte
   20,12 s en p50 à cache chaud** (20,15 s en p95 et p99 ; 20,38 s au premier
   accès d'une copie neuve). À 1 M, 2,02 s ; à 9 750 passages vivants, 23,6 ms.
2. **Le calcul domine l'entrée/sortie** : 47 % du temps est le décodage JSON,
   32 % la lecture SQLite, 21 % l'insertion dans les bitmaps et les colonnes. Le
   premier accès d'une copie neuve n'ajoute que 1,3 % à 8,58 M.
3. **La mémoire des structures est de 369,3 Mio à 8,58 M lignes**, dont 333,4 Mio
   de colonnes de valeurs ; 45,1 octets par ligne. C'est ce que `crible inspect`
   rapporte, et la valeur du banc coïncide avec la sienne sur `synth-small`.
4. **Le coût est linéaire en nombre de lignes**, à 16 % près entre 1 M et 8,58 M.
   Un index qui porte des pierres tombales se reconstruit donc en lisant aussi
   les lignes mortes.

Ce que la mesure ne permet pas d'affirmer :

- **Rien sur l'index réel de Legifacile.** Le point à 8,58 M est une fixture au
  schéma de `synth-small`, pas l'échantillon Legifacile. Le nombre de champs
  filtrables, la cardinalité des `keyword` et la longueur des textes changent le
  coût, et aucun de ces facteurs n'est mesuré ici. La fiche de P3 demande
  d'ailleurs la mesure sur « l'échantillon Legifacile » : ce rapport est le
  premier chiffre, pas le rapport de P3.
- **Rien sur la comparaison avec un cache de bitmaps** : le cache n'existe pas.
  Aucune conclusion sur son gain, son format ou son invalidation.
- **Rien sur un froid contrôlé** : le § 6 est une approximation, pas une purge.
- **Rien sur la reprise** : la fixture n'a ni segment ni `tmp-*`, donc les étapes
  de nettoyage d'INV-04 ne coûtent rien ici. Un index réel avec des milliers de
  segments ajouterait la relecture des `segment.json`, bornée par MES-15.
- **Rien sur les autres étapes du démarrage** : le lexical (tantivy, P5), les
  `mmap` de vecteurs et le chargement du modèle n'existent pas dans le snapshot
  de P1 et ne sont pas mesurés.

**Comparaison au budget.** Il n'y a pas de cible chiffrée dans `docs/08`. Le seul
budget écrit est celui de la fiche `MESURER-CYCLE-DE-VIE`
(`docs/phases/P3-cycle-de-vie.md`) : « si le démarrage dépasse 60 s à l'échelle de
Legifacile, un cache des bitmaps est proposé ». Le point à 8,58 M mesure
**20,1 s, soit 33,5 % de ce budget et une marge de 2,98 fois** ; avec 10 % de
lignes mortes, le calcul du § 4 donne ≈ 22,4 s, soit 37 % du budget. Dans les
deux cas, le critère n'appelle pas de cache pour ce schéma et cette échelle.

**Recommandations, à valider par le propriétaire** (aucune n'a été appliquée) :

1. **Ne pas ouvrir `AJOUTER-CACHE-BITMAPS` sur la foi de ce chiffre.** Le critère
   des 60 s n'est pas atteint, avec un facteur 3 de marge. La décision doit
   toutefois être prise en P3, sur l'échantillon Legifacile et avec le schéma
   réel : c'est la mesure que la fiche `MESURER-CYCLE-DE-VIE` demande.
2. **Si le démarrage doit diminuer avant P3**, les deux pistes que cette mesure
   désigne, dans l'ordre, sont le décodage JSON (47 %) et la lecture SQLite
   (32 %). Le décodage répète le JSON du document pour chaque passage du
   document (un document pour cinq passages ici) : décoder chaque document une
   fois est la piste la plus directe. Elle n'est pas mesurée et demande une tâche
   dédiée, avec un test d'égalité des structures.
3. **Aucun seuil `N` ni `exact_threshold`** ne se déduit de cette mesure : elle ne
   porte que sur l'ouverture d'un index.

## 9. Données brutes

`docs/mesures/MES-13-temps-de-demarrage.2026-10-22.json` : les neuf points (trois
points chauds, trois froids approximatifs, trois du second tour), les 5
répétitions par phase, les percentiles, la mémoire des structures et la taille du
`records.sqlite`. Les copies de travail et les relevés intermédiaires restent dans
`data/mes13/` (gitignoré) : `raw/` (un JSON par invocation, les conditions de
charge) et `cold-work/` (vide après la campagne, le banc supprime chaque copie).

## 10. Constats hors mesure

- `data/mes15/i-8580000-s10`, l'index de 8,58 M de MES-15, **n'a pas d'`index.json`** :
  `GenerationStore::open` le refuse (`invalid_request`, « index … declares its
  schema in index.json »). Le banc MES-15 le préparait sans configuration. Le
  point à l'échelle de ce rapport emploie donc la fixture de MES-14, qui porte un
  `index.json` et un schéma plus riche. Si MES-13 doit un jour mesurer l'index de
  MES-15, il faudra soit le régénérer avec une configuration, soit ouvrir
  `RecordStore` sans `GenerationStore` — ce qui ne mesurerait plus le vrai
  démarrage.
- La fixture n'a **aucune ligne dans `segments`** : `build_snapshot` ne lit donc
  aucun `segment.json`. Ce n'est pas un écart mesuré du démarrage réel, mais une
  omission bornée par MES-15 (0,36 ms à 10 segments, 12,8 à 15,2 ms à 1 000).
- `cargo fmt` a été passé après la campagne, puis le second tour a été joué avec
  le binaire reconstruit : les chiffres du § 7 ne dépendent donc pas d'un écart
  entre le code mesuré et le code du dépôt.
- Le banc ouvre SQLite lui-même pour les trois phases de réplique, avec les mêmes
  options qu'une lecture de production (lecture-écriture, WAL, `busy_timeout`
  1 000 ms), parce que `RecordStore::connection` est `pub(crate)`. La phase
  `build` et la phase `open` appellent, elles, le code de production.

## 11. Campagne du 2026-10-25 — rejeu sur l'échantillon Legifacile (schéma réel)

Rejeu demandé par la fiche `MESURER-CYCLE-DE-VIE` (P3), pour remplacer la fixture
au schéma de `synth-small` du § 3 par **l'échantillon Legifacile** : schéma réel,
cardinalités réelles, proportions réelles de passages par document. C'est la
mesure que l'ADR-0047 attendait pour trancher l'ordre des remèdes.

- Tâche : `MESURER-CYCLE-DE-VIE` (phase P3), passage A.
- Références : `docs/08-invariants-et-tests.md` (MES-13), ADR-0047,
  `docs/02-architecture.md` (« Reprise au démarrage »),
  `docs/phases/P3-cycle-de-vie.md` (seuil de 60 s),
  `crates/crible-store/src/filters_index.rs` (`FilterIndex::from_records`),
  `crates/crible-store/src/generation.rs` (`GenerationStore::open`,
  `build_snapshot`).
- Commit : `46c2dd1edc8fcac06df61ea560c86ecc915de1fc` (branche
  `tache/MESURER-CYCLE-DE-VIE`). **Aucun code de production n'a été modifié
  pendant la campagne.** Le seul ajout de code est `tools/bench/mes13/replay.sh` ;
  le banc `crates/crible-store/benches/startup_cost.rs` est inchangé.
- Machine : MacBook Apple M3 (8 cœurs : 4 performance, 4 efficacité), 24 Go de RAM
  (25 769 803 776 octets), macOS 15.7.7 (24G720), APFS, 134 Go libres. Mesure
  **native**, aucun conteneur.
- Charge ambiante : la machine n'était **pas au repos**. La moyenne de charge est
  passée de 4,11 à 4,33 sur 8 cœurs pendant la campagne
  (`data/mes13/replay-raw/conditions-start.txt` et `conditions-end.txt`) ;
  `mediaanalysisd` et `mds_stores` ont occupé jusqu'à un cœur chacun par moments.
  Les p95 des cinq points sont à moins de 2 % des p50, donc l'effet sur les p50 est
  faible, sans être mesuré. La campagne du 2026-10-22 avait une charge du même ordre
  (4,06 à 3,01).
- Jeu de données : les **cinq index attachés de `data/legifacile-sample`**, copiés
  dans `data/mes13/legifacile-<structure>` (clone APFS) pour laisser les index
  attachés intacts. Tailles décimales (Mo = 10⁶ octets), mémoires en Mio.

| Index | Lignes `passages` | Documents | Passages / document | `records.sqlite` | SHA-256 |
|---|---:|---:|---:|---:|---|
| `articles` | 50 000 | 42 417 | 1,179 | 58,8 Mo | `1f1b5f7be15336f683e4fa44654a934bfbeaccb7e2de9bdaa6f62b7b36ab0985` |
| `decisions` | 49 998 | 20 120 | 2,485 | 62,0 Mo | `4d4c027857af3eeac8d3b96cadbac79994a552755204da49597793ad313a2f60` |
| `jorf` | 50 000 | 39 457 | 1,267 | 55,1 Mo | `795972b376b82b4a84e7df4bbb8930a698f8a0136a203d9408050689b3fbe5bb` |
| `titres` | 50 000 | 50 000 | 1,000 | 25,6 Mo | `e54016e7da5c955c3397ec72af3abfb1d6cce0d98873469e5385e56a34a7c88f` |
| `sarde` | 50 000 | 49 674 | 1,007 | 29,1 Mo | `e30c740496bec53973a8bfb07bdfb41056fb41b5ad7a317aff6b14d5e14ec08c` |
| **Total attaché** | **249 998** | **201 668** | **1,2397** | 230,6 Mo | — |

  Le lot `dole` de l'échantillon **n'est pas attaché** : il porte 3 593 documents et
  7 628 passages, soit 3,0 % des passages de l'échantillon complet. L'échantillon
  complet (six lots) porte 205 261 documents et 257 626 passages, soit
  **1,2551 passage par document** ; c'est ce dernier rapport que cite le § 11.5,
  et 1,2397 pour les cinq index mesurés. Aucune ligne n'est morte : les cinq index
  sont à leur première génération, sans pierre tombale. Les index ont des segments
  réels (un par lot) et leurs vecteurs fp16, contrairement aux fixtures du § 3.

- Configuration mesurée : `--repeats 5 --warmup 1`, cache chaud, puis mode `cold`
  (une copie neuve par phase). Aucun réglage de recherche n'entre dans la mesure :
  elle ne lit que `records.sqlite`.

### 11.1 Ce qui n'a pas pu être mesuré : le chargement du cache

La fiche demande un temps de démarrage « en reconstruisant les bitmaps puis en
chargeant le cache ». **Aucun cache de bitmaps n'existe** : la tâche
`AJOUTER-CACHE-BITMAPS` n'a jamais été ouverte, et l'ADR-0047 la conditionne à un
dépassement du seuil de 60 s **après** `DECODER-DOCUMENTS-UNE-FOIS`. Cette
campagne ne mesure donc que la **reconstruction**, comme celle du 2026-10-22 ; la
comparaison avec un chargement de cache reste impossible et n'est pas simulée.
C'est un écart au protocole demandé, imposé par l'état du code, et non un choix de
mesure.

### 11.2 Protocole

```sh
# compilation du banc avant la campagne
cargo bench -p crible-store --bench startup_cost --no-run

# cinq copies de travail, clone APFS des index attachés
for n in articles decisions jorf titres sarde; do
    cp -cR data/indexes/$n data/mes13/legifacile-$n
done

# chaud : 1 ouverture non chronométrée, puis 5 passes des quatre phases
# froid approximatif : une copie neuve par phase
sh tools/bench/mes13/replay.sh
```

`replay.sh` enchaîne les deux modes sur les cinq structures et fusionne le tout
dans `docs/mesures/MES-13-temps-de-demarrage.2026-10-25.json`. Les conventions du
§ 2 s'appliquent : quatre phases (`sql_read`, `json_decode`, `build`, `open`),
p50/p95/p99 par rang le plus proche, `open` appelle `GenerationStore::open` et
`build` appelle `FilterIndex::from_records`. Les deux premières phases restent des
répliques du SQL privé, comme en 2026-10-22.

### 11.3 Résultats — cache chaud

En millisecondes, 5 répétitions par phase et par point.

| Index | Lignes | `sql_read` p50 | `json_decode` p50 | `build` p50 | `open` p50 | `open` p95 | `open` p99 | `open` par ligne |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| `articles` | 50 000 | 45,0 | 82,4 | 168,5 | **168,7** | 171,8 | 171,8 | 3,37 µs |
| `decisions` | 49 998 | 35,4 | 67,7 | 86,3 | **86,4** | 86,7 | 86,7 | 1,73 µs |
| `jorf` | 50 000 | 37,8 | 67,2 | 97,3 | **97,8** | 98,0 | 98,0 | 1,96 µs |
| `titres` | 50 000 | 34,8 | 58,7 | 66,4 | **66,8** | 68,4 | 68,4 | 1,34 µs |
| `sarde` | 50 000 | 35,4 | 45,8 | 64,2 | **64,5** | 64,9 | 64,9 | 1,29 µs |
| **Total** | **249 998** | **188,3** | **321,8** | **482,6** | **484,1** | — | — | **1,94 µs** |

Le total n'est pas un index : c'est la somme des cinq ouvertures, utile pour le
coût par ligne pondéré. `open − build` vaut +0,2 ms à +0,5 ms par index : les
métadonnées de segments, la reprise et le manifeste sont négligeables à cette
échelle, comme le § 4 le prévoyait.

### 11.4 Décomposition

« Décodage » est `json_decode − sql_read`, « insertion » est
`build − json_decode` ; le pourcentage est la part de `build`.

| Index | Lecture SQLite | Décodage JSON | Insertion | Reste de `open` |
|---|---:|---:|---:|---:|
| `articles` | 45,0 ms (26,7 %) | 37,4 ms (22,2 %) | 86,1 ms (51,1 %) | +0,2 ms |
| `decisions` | 35,4 ms (41,0 %) | 32,3 ms (37,4 %) | 18,6 ms (21,6 %) | +0,1 ms |
| `jorf` | 37,8 ms (38,8 %) | 29,4 ms (30,2 %) | 30,1 ms (30,9 %) | +0,5 ms |
| `titres` | 34,8 ms (52,4 %) | 23,9 ms (36,1 %) | 7,7 ms (11,6 %) | +0,4 ms |
| `sarde` | 35,4 ms (55,1 %) | 10,4 ms (16,2 %) | 18,4 ms (28,7 %) | +0,3 ms |
| **Total** | **188,3 ms (39,0 %)** | **133,5 ms (27,6 %)** | **161,5 ms (33,4 %)** | **+1,5 ms** |

Le décodage JSON n'est plus le premier poste sur le schéma réel : il vaut
**27,6 %** de la construction, contre 46,8 % sur la fixture du § 4. Comme
`open − build` vaut +1,5 ms au total, cette part est aussi celle du démarrage
mesuré. La raison est
visible dans les extrêmes : sur `sarde`, un seul champ `keyword` de faible
cardinalité donne 0,21 µs de décodage par ligne ; sur `articles`, six champs dont
37 228 codes distincts donnent 1,72 µs d'insertion par ligne et 22,2 % de décodage.
L'insertion dans les bitmaps de `keyword` porte 51 % de `articles` et 11,6 % de
`titres` : c'est la cardinalité qui décide, pas le nombre de lignes.

### 11.5 À l'échelle de Legifacile — extrapolation

Aucun index de l'échantillon n'atteint 8,58 M passages. Les valeurs ci-dessous sont
donc **extrapolées** par la règle linéaire mesurée sur 50 k lignes par index, et
signalées comme telles ; ce ne sont pas des mesures. Deux bornes encadrent le
résultat :

| Base de l'extrapolation | Coût par ligne | 8,58 M passages | 9,44 M lignes (10 % de morts) | Mesuré ou extrapolé |
|---|---:|---:|---:|---|
| Total pondéré des cinq index | 1,94 µs | **16,6 s** | 18,3 s | extrapolé |
| Index le plus cher, `articles` | 3,37 µs | **28,9 s** | 31,8 s | extrapolé |
| Fixture `synth-small` à 8,58 M (§ 3) | 2,35 µs | 20,1 s | 22,4 s | mesuré le 2026-10-22 |

Deux corrections sont connues mais non mesurées sur ce schéma :

- la lecture SQLite par ligne augmente avec la taille du fichier : 0,44 → 0,75 µs
  entre 1 M et 8,58 M sur la fixture (§ 4), +70 %, alors que le décodage reste
  stable. Appliquée à `articles`, elle porterait l'extrapolation de 28,9 à environ
  **34 s** (calculé, non mesuré) ;
- les 10 % de lignes mortes du seuil de compaction (ADR-0006) ajoutent 10 % de
  lignes à lire, ce que la colonne de droite chiffre.

Même la borne la plus haute — `articles` à 8,58 M avec 10 % de morts, 31,8 s —
reste **sous le seuil de 60 s**, avec une marge de 1,9. La borne pondérée donne une
marge de 3,3 à 3,6.

### 11.6 Nombre de passages par document et gain maximal du décodage unique

L'échantillon complet porte **1,2551 passage par document** (257 626 / 205 261) ;
les cinq index mesurés portent **1,2397** (249 998 / 201 668). `from_records`
décode le JSON du document à **chaque passage** qui l'hérite : un document y est
donc décodé 1,24 fois en moyenne, et non cinq fois comme dans la fixture du § 4.

Conséquence : décoder chaque document une seule fois épargnerait au plus
`1 − 1 / 1,2397 = 19,3 %` du décodage des documents, et le décodage complet vaut
27,6 % de la construction. La borne **haute** du gain est donc **5,3 % du
démarrage**, soit environ **0,9 s** sur les 16,6 s extrapolés, ou 1,5 s sur les
28,9 s de `articles`. C'est calculé, pas mesuré, et c'est une borne : la part du
JSON de document dans le décodage n'a pas été isolée, et le décodage du passage
n'est pas répété. Sur ce corpus, `DECODER-DOCUMENTS-UNE-FOIS` ne peut donc pas
retirer les ≈ 7,5 s calculés sur la fixture à cinq passages par document (ADR-0047).

### 11.7 Cache froid — approximation

Une copie neuve par phase, chronométrée une fois. Ce n'est toujours pas un froid
contrôlé : macOS ne purge pas son cache de pages sans `root`, et un clone APFS
partage ses blocs avec l'original.

| Index | `sql_read` | `json_decode` | `build` | `open` | `open` contre chaud |
|---|---:|---:|---:|---:|---:|
| `articles` | 198,4 ms | 219,7 ms | 304,2 ms | 333,3 ms | +97,6 % |
| `decisions` | 159,4 ms | 177,0 ms | 192,4 ms | 216,5 ms | +150,6 % |
| `jorf` | 173,3 ms | 186,8 ms | 245,2 ms | 225,4 ms | +130,5 % |
| `titres` | 127,9 ms | 141,9 ms | 173,9 ms | 150,3 ms | +124,9 % |
| `sarde` | 132,5 ms | 132,7 ms | 153,8 ms | 153,8 ms | +138,4 % |

À cette échelle (25 à 62 Mo de `records.sqlite`), le premier accès d'un clone coûte
2 à 2,5 fois le chaud, mais les valeurs restent des centaines de millisecondes. Le
`build` de `jorf` et de `titres` sort au-dessus de leur `open` : c'est la
dispersion d'une mesure unique par phase, pas un ordre inversé du code.

### 11.8 Conclusion

Ce que la mesure permet d'affirmer, sur le schéma et les proportions de
l'échantillon Legifacile, à cache chaud :

1. **Le démarrage n'atteint pas le seuil de 60 s à l'échelle de Legifacile.** La
   borne pondérée donne 16,6 s pour 8,58 M passages (18,3 s avec 10 % de morts), la
   borne la plus chère 28,9 s (31,8 s avec 10 % de morts). Les deux sont des
   extrapolations linéaires, signalées ; la marge au seuil va de 1,9 à 3,6.
2. **Le décodage JSON n'est plus le premier poste** : 27,6 % de la construction,
   contre 46,8 % sur la fixture. La lecture SQLite (39,0 %) et l'insertion dans les
   `keyword` (33,4 %) pèsent davantage.
3. **Le nombre moyen de passages par document est 1,2551** sur l'échantillon complet
   et 1,2397 sur les cinq index mesurés. Le décodage unique par document ne peut
   donc retirer au plus que 5,3 % du démarrage, calculé, pas mesuré.
4. **Le schéma change le coût par ligne d'un facteur 2,6** : 1,29 µs (`sarde`) à
   3,37 µs (`articles`) pour 50 000 lignes. Un chiffre unique « par passage » n'a
   pas de sens sans dire le schéma.

Ce que la mesure ne permet pas d'affirmer :

- **Rien sur le chargement d'un cache** : il n'existe pas (§ 11.1), et la
  comparaison demandée par la fiche n'a pas été faite.
- **Rien de mesuré à 8,58 M sur ce schéma** : les valeurs du § 11.5 sont
  extrapolées d'un point à 50 k lignes par index. La croissance de la lecture
  SQLite avec la taille du fichier n'est pas modélisée, seulement bornée par
  analogie avec la fixture.
- **Rien sur `dole`** : 3,0 % des passages de l'échantillon ne sont pas attachés.
- **Rien sur les autres étapes du démarrage** : le lexical (tantivy, P5), les
  `mmap` de vecteurs et le chargement du modèle ne sont pas dans le snapshot
  mesuré.

**Comparaison au budget et ordre des remèdes (ADR-0047).** Le seuil est 60 s à
l'échelle de Legifacile ; la borne la plus haute mesurée par extrapolation vaut
31,8 s, soit **53 % du budget**. Le seuil **n'est pas dépassé** : ni
`DECODER-DOCUMENTS-UNE-FOIS` ni `AJOUTER-CACHE-BITMAPS` ne sont proposées par cette
campagne. La seconde ne pouvait de toute façon pas l'être avant la première, et le
gain maximal de la première (5,3 %, borne haute) ne suffirait pas à justifier un
format de cache.

**Recommandations, à valider par le propriétaire** (aucune n'a été appliquée) :

1. **Ne pas ouvrir `DECODER-DOCUMENTS-UNE-FOIS` sur la foi de cette campagne** : le
   seuil n'est pas atteint, et le gain maximal calculé (0,9 à 1,5 s) est faible sur
   un corpus à 1,24 passage par document. La condition de réouverture est un index
   réel dont le démarrage mesuré dépasserait 60 s.
2. **Ne pas ouvrir `AJOUTER-CACHE-BITMAPS`** : sa condition (dépassement après
   `DECODER-DOCUMENTS-UNE-FOIS`) n'est pas remplie.
3. **Si un jour le démarrage doit diminuer**, la mesure désigne la lecture SQLite
   (39 %) et l'insertion dans les `keyword` (33 %), pas le décodage (27,6 %) — un
   ordre différent de celui de la fixture. Aucune de ces pistes n'est chiffrée ici.
4. **Aucun seuil `N` ni `exact_threshold`** ne se déduit de cette mesure.

### 11.9 Données brutes

`docs/mesures/MES-13-temps-de-demarrage.2026-10-25.json` : les dix points (cinq
chauds, cinq froids approximatifs), 5 répétitions par phase pour les points chauds,
la mémoire des structures et la taille de `records.sqlite`. Les JSON par invocation
et les conditions de charge restent dans `data/mes13/replay-raw/` ; les copies de
travail dans `data/mes13/legifacile-*` (clone APFS, 586 Mo) et
`data/mes13/cold-work/` (vide après la campagne).
