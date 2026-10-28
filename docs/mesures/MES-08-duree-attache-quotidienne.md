# MES-08 — Durée d'une attache quotidienne type (~1 000 à 3 000 passages)

## Campagne du 2026-10-25

- Tâche : `MESURER-CYCLE-DE-VIE` (phase P3), passage B.
- Commit : `46c2dd1` (branche `tache/MESURER-CYCLE-DE-VIE`). **Aucun code de
  production n'a été modifié pendant la campagne.** Le banc
  `crates/crible/benches/attach_cost.rs` et les scripts `tools/bench/mes08/run.sh`,
  `tools/bench/mes08/make_batches.py` et `tools/bench/mes08/merge.py` sont neufs et
  non commités au moment de la mesure ; l'attache mesurée est celle de `46c2dd1`.
- Machine : MacBook Apple M3 (8 cœurs : 4 performance, 4 efficacité), 24 Go de RAM,
  macOS 15.7.7 (24G720), APFS, 104 Gio libres avant et après la campagne. Mesure
  **native**, aucun conteneur. Le banc est un binaire de `crates/crible/benches/`,
  compilé par `cargo bench` (profil `bench`, optimisations actives).
- Charge ambiante : la machine n'était pas au repos, comme pour MES-07 et le
  passage A. Charge moyenne 2,74 au début, 2,31 à la fin (8 cœurs) ;
  `mediaanalysisd` a occupé 91 à 104 % d'un cœur et `mds_stores` 13 à 28 % pendant
  la campagne (`data/mes08/raw/conditions-start.txt` et `conditions-end.txt`).
  L'effet de cette charge n'est pas isolé.
- Jeux de données mesurés :
  - `data/mes07/indexes/articles` : l'échantillon Legifacile, structure
    `articles` — 50 000 passages, 42 417 documents, textes réels (1 167 octets en
    moyenne), vecteurs fp16 768 dimensions, modèle `mdenseon` (`onnx`),
    `reference_precision` `fp16`, `acceptance_sample` 200. 44 890 passages vivants
    (dead ratio 0,1022), 1 segment. Index : 130 Mo.
  - `data/synth-large/indexes/synth-large` : `synth-large`, 8 600 000 passages
    (`tools/gen_synth.py --large`, graine 20261021), un passage par document,
    modèle `synthetic-external` (`external`). 7 740 000 passages vivants (dead
    ratio 0,1000), 1 segment. Index : 16 Go.
  - Les deux index de base sont construits par `tools/bench/mes07/run.sh`
    (fonction `prepare`, idempotente : `tools/gen_synth.py --large`,
    `crible index create`, `crible attach`) ; c'est ce script qui fait foi pour
    leur contenu, aucune somme de contrôle n'est recalculée ici.
- Lots quotidiens mesurés, écrits par `make_batches.py daily` (mode `upsert`,
  graine 20261025) :

  | Lot | Passages | Documents | Texte moyen | Vecteurs | Lot (taille) |
  |---|---|---|---|---|---|
  | `daily-articles-1000` | 1 000 | 905 | 1 132 octets | fp16 réels de l'échantillon | 2,2 Mo |
  | `daily-articles-3000` | 3 002 | 2 517 | 1 221 octets | fp16 réels de l'échantillon | 6,5 Mo |
  | `daily-synth-1000` | 1 000 | 905 | 1 132 octets | fp16 aléatoires normalisés | 2,1 Mo |
  | `daily-synth-3000` | 3 002 | 2 517 | 1 221 octets | fp16 aléatoires normalisés | 6,3 Mo |

  Les textes sont de **vrais textes de l'échantillon Legifacile** (structure
  `articles`), pas des textes de synthèse : c'est ce qui rend le coût de
  compression comparable à la production. Les vecteurs des lots `synth-large` sont
  aléatoires mais de même taille que les vecteurs réels (768 × 2 octets) ; ils ne
  changent donc ni le volume écrit ni le coût du segment, mais ils ne portent pas
  la similarité réelle des passages.

### Protocole

```sh
# campagne complète : préparation des lots, puis 5 répétitions par point
ORT_DYLIB_PATH=tools/.venv/lib/python3.13/site-packages/onnxruntime/capi/libonnxruntime.1.31.0.dylib \
  sh tools/bench/mes08/run.sh
```

`run.sh` construit ce qui manque (les index de base viennent de `mes07/run.sh`),
mesure, puis fusionne les rapports bruts dans
`docs/mesures/MES-08-duree-attache-quotidienne.2026-10-25.json`.

Une répétition, dans un processus unique :

1. clone APFS (`cp -cR`) de l'index de base dans
   `data/mes08/work/<lot>-rep-<i>` : une attache change la génération, et rejouer
   le même `batch_id` serait un no-op (INV-10) ;
2. `GenerationStore::open` du clone, relevé de l'état avant (non chronométré) ;
3. **`attach()` réel**, chronométré, avec le modèle de l'index : compteur de jetons
   pour `onnx`, encodeur fp32 pour la recette (contrôle 9), rien pour `external` ;
4. lecture des cinq durées d'étape du rapport d'attache de production
   (`AttachDurations`, `docs/04`), de l'état après, puis suppression du clone.

Avant la boucle du lot, le banc rejoue la **compression de ses textes** deux fois,
hors attache : une fois avec un `zstd::bulk::Compressor` réutilisé (ce que fait la
production depuis ADR-0048), une fois avec un `zstd::stream::encode_all` par texte
(ce qu'elle faisait avant). C'est une **réplique** : la compression réelle tourne
dans l'étape `transaction`. Elle borne le gain de `REUTILISER-COMPRESSEUR-ZSTD` sur
un lot quotidien réel.

Les cinq étapes sont celles de `docs/04`, mesurées par le code de production :
`validation` (ouverture du lot, contrôles, décision par document, recette),
`segment` (écriture des vecteurs du segment neuf), `transaction` (corps de la
transaction SQLite, compression des textes comprise), `manifest` (manifeste dérivé
et sa publication) et `snapshot` (construction de l'instantané après le point de
commit, `FilterIndex::after_commit` et métadonnées de segments). Aucun
préchauffage séparé n'est fait : chaque répétition part d'un clone neuf, relu en
entier par l'attache elle-même. Le cache est chaud — le clone vient d'être écrit —
mais il n'est pas le cache de la répétition précédente.

Percentiles par rang le plus proche sur 5 répétitions ; sur 5 échantillons, p95 et
p99 valent le maximum.

### Résultats

Attache complète, 5 répétitions, en millisecondes. `vivants` est le nombre de
passages vivants de l'index avant l'attache.

| Index | Vivants | Lot | p50 | p95 | p99 | min | max |
|---|---|---|---|---|---|---|---|
| `articles` | 44 890 | 1 000 passages | **52 333** | 53 668 | 53 668 | 47 972 | 53 668 |
| `articles` | 44 890 | 3 002 passages | **68 467** | 70 561 | 70 561 | 67 399 | 70 561 |
| `synth-large` | 7 740 000 | 1 000 passages | **3 257** | 3 279 | 3 279 | 3 252 | 3 279 |
| `synth-large` | 7 740 000 | 3 002 passages | **3 333** | 3 359 | 3 359 | 3 320 | 3 359 |

Décomposition par étape, p50 en millisecondes ; la part est celle du p50 de
l'attache. Le maximum de chaque étape est donné entre parenthèses quand il
dépasse le p50 de plus de 10 %.

| Index | Lot | Attache | `validation` | `segment` | `transaction` | `manifest` | `snapshot` |
|---|---|---|---|---|---|---|---|
| `articles` | 1 000 | 52 333 | **51 469 (98,3 %)** | 19 (0,04 %) | 19 (0,04 %) | 6 (0,01 %) | 823 (1,6 %) |
| `articles` | 3 002 | 68 467 | **65 923 (96,3 %)** | 34 (0,05 %) | 58 (0,08 %) | 7 (0,01 %) | 2 439 (3,6 %) |
| `synth-large` | 1 000 | 3 257 | 7 (0,2 %) | 19 (0,6 %) | 18 (0,6 %) | 7 (0,2 %) | **3 207 (98,5 %)** |
| `synth-large` | 3 002 | 3 333 | 19 (0,6 %) | 33 (1,0 %), max 58 | 55 (1,7 %) | 6 (0,2 %) | **3 219 (96,6 %)** |

La somme des cinq étapes égale l'attache à 6 ms près au plus, jamais à plus de
0,04 % en relatif : la
décomposition couvre tout l'appel, il n'y a pas de « reste » significatif. Le
maximum de RSS du processus de banc est de 2,81 Gio pour l'index `articles`
(encodeur fp32 chargé) et de 591 Mio pour `synth-large`.

Compression des textes du lot, en millisecondes pour le lot entier, p50 de cinq
répliques, plus le gain par passage :

| Index | Lot | Textes | Contexte réutilisé | Un contexte par texte | Gain | Gain par passage |
|---|---|---|---|---|---|---|
| `articles` | 1 000 | 1 000 | 5,84 | 38,55 | **32,71 (−84,9 %)** | 32,7 µs |
| `articles` | 3 002 | 3 002 | 22,43 | 130,81 | **108,38 (−82,9 %)** | 36,1 µs |
| `synth-large` | 1 000 | 1 000 | 6,07 | 43,37 | **37,30 (−86,0 %)** | 37,3 µs |
| `synth-large` | 3 002 | 3 002 | 19,26 | 135,41 | **116,15 (−85,8 %)** | 38,7 µs |

Comparaison avec MES-15, en gardant la différence de textes en tête (MES-15
compresse des phrases de banc, cette campagne des textes réels) :

| Source | Textes | Taille des textes | `encode_all` par ligne | Contexte réutilisé | Gain par passage |
|---|---|---|---|---|---|
| MES-15, campagne du 2026-10-21 (§ 4) | 2 000 | ~130 octets (phrase de banc) | 67,2 à 89,9 ms | 5,9 à 6,1 ms (point de contrôle) | 30 à 41 µs |
| MES-15, rejeu du 2026-10-25 (§ 10.3) | 2 000 | ~130 octets | — | 6,9 à 7,0 ms | — |
| MES-08, cette campagne | 1 000 et 3 002 | 1 132 et 1 221 octets (textes réels) | 38,6 à 135,4 ms | 5,8 à 22,4 ms | **32,7 à 38,7 µs** |

Le gain par passage est le même, à 10 % près, sur des textes de taille très
différente : c'est le **contexte zstd construit par ligne** qui coûte, pas les
octets compressés. Le coût résiduel, lui, suit la taille du texte (5,8 µs par texte
de 1 132 octets ici, 3,0 à 3,5 µs par phrase de ~130 octets dans MES-15). La
comparaison avec le corps de transaction de MES-15 (77,6 à 100,4 ms ramenés à 16,1
à 17,3 ms) ne peut pas se transposer telle quelle : MES-15 mesure un commit de
2 000 passages sans recette ni écriture de segment, alors que l'étape
`transaction` mesurée ici porte aussi la ligne de lot, son rapport JSON, la ligne
de segment et les pierres tombales. Elle vaut 18 à 19 ms pour 1 000 passages et 55
à 58 ms pour 3 002, **indépendamment du modèle et de la taille de l'index**
(19 et 58 ms à 44 890 vivants, 18 et 55 ms à 7 740 000) : le chemin d'écriture ne
dépend que du lot.

Part du commit de génération dans l'attache, c'est-à-dire de l'étape `snapshot` :

| Index | Vivants | Lot | Attache p50 | `snapshot` p50 | Part |
|---|---|---|---|---|---|
| `articles` | 44 890 | 1 000 | 52 333 | 823 | 1,6 % |
| `articles` | 44 890 | 3 002 | 68 467 | 2 439 | 3,6 % |
| `synth-large` | 7 740 000 | 1 000 | 3 257 | 3 207 | 98,5 % |
| `synth-large` | 7 740 000 | 3 002 | 3 333 | 3 219 | 96,6 % |

À 7,74 M passages vivants, l'étape `snapshot` est **plate en taille de lot**
(3 207 et 3 219 ms pour 1 000 et 3 002 passages, soit +0,4 %) : ce n'est pas le lot
qui la paie, c'est l'index. Ce coût est celui que MES-15 (§ 10.5) a isolé hors
banc : `FilterIndex::after_commit` exécute deux requêtes sans index sur
`passages(created_generation)` et `documents(updated_generation)`, qui balaient
toute la table — 2,92 s en p50 à 8,58 M passages vivants, moins de 0,01 ms avec les
deux index de diagnostic. La mesure de MES-08 le retrouve par un autre chemin (une
attache réelle de production) et lui donne la même valeur.

Sur l'échantillon, la même étape croît avec le nombre de passages ajoutés (823 ms
pour 905 documents, 2 439 ms pour 2 517, soit ≈ 1,0 ms par document ajouté et
≈ 0,8 ms par passage ajouté) : les
deux balayages n'y expliquent qu'une faible part, et le travail par ligne ajoutée
(analyse des champs, insertion dans les six champs de document et les cinq champs
de passage du schéma) n'est **pas isolé**. L'écart de comportement entre les deux
index n'est donc pas expliqué ; c'est une raison de plus de ne pas extrapoler cette
étape depuis l'échantillon.

### Extrapolation à l'échelle de Legifacile

Trois extrapolations, toutes signalées comme telles.

1. **Attache quotidienne avec un modèle `external`, à l'échelle de Legifacile.**
   L'étape `snapshot` est plate en taille de lot et croît avec le nombre de
   passages vivants (MES-15 : 350 ms à 1 M, 3 059 ms à 8,58 M). Appliquée
   linéairement au point mesuré ici (3 207 et 3 219 ms à 7,74 M vivants), la loi
   donne 3 555 à 3 568 ms à 8,58 M vivants. MES-15 mesure directement 3 085 ms à
   8,58 M sur son propre index. Les deux valeurs diffèrent de 15 % et la cause
   n'est pas isolée (taille et contenu de `records.sqlite` différents) : on retient
   **3,1 à 3,6 s** pour l'étape `snapshot`, et donc **3,2 à 3,7 s** pour une
   attache quotidienne de 1 000 à 3 000 passages sur un index de 8,58 M passages
   vivants, modèle `external`.
2. **Attache quotidienne avec le modèle réel `mdenseon`.** L'étape `validation` est
   la seule que le modèle change : elle vaut 51 469 ms pour 1 000 passages et
   65 923 ms pour 3 002 sur l'échantillon. Un ajustement linéaire sur ces deux
   points donne une part fixe de **44,2 s** et **7,2 ms par passage ajouté**
   (44,2 s + 7,2 s = 51,5 s ; 44,2 s + 21,7 s = 65,9 s). La part fixe est
   cohérente avec la recette du contrôle 9 : elle ré-encode 200 passages en fp32,
   quel que soit le lot (`acceptance_sample` 200, plafonné par le lot), soit
   ≈ 221 ms par passage ré-encodé. La part par passage (comptage de jetons et
   contrôles) n'est pas isolée. Ajoutée aux 3,2 à 3,7 s ci-dessus, l'attache
   quotidienne à l'échelle de Legifacile coûterait **≈ 55 s pour 1 000 passages et
   ≈ 69 s pour 3 000**, dont 93 à 96 % de recette. Cette extrapolation suppose que
   la recette coûte le même prix à 8,58 M passages qu'à 44 890 : elle ne dépend que
   du lot et du modèle, pas de l'index, mais ce n'est pas mesuré à 8,58 M.
3. **Gain du compresseur zstd réutilisé, à l'échelle de Legifacile.** À 32,7 à
   38,7 µs par passage, un lot quotidien de 1 000 à 3 000 passages gagne **33 à
   116 ms** par attache. C'est le gain mesuré, sans extrapolation ; il ne dépend ni
   du modèle ni de la taille de l'index (les quatre points le confirment).

### Écarts au protocole

- **La machine n'était pas au repos** (voir plus haut) ; la mesure est faite dans
  ces conditions et le signale. La dispersion reste faible : le point de 1 000
  passages va de 47 972 à 53 668 ms (−8,3 % à +2,6 % autour de son p50), celui de
  3 002 de 67 399 à 70 561 ms (−1,6 % à +3,1 %), et les deux points `synth-large`
  de 3 252 à 3 359 ms (−0,2 % à +3,1 %). Aucune valeur n'est retirée.
- **Pas de préchauffage séparé.** Chaque répétition part d'un clone APFS neuf ;
  l'attache relit le lot et écrit, mais ne relit pas tout l'index. Sur `articles`,
  la première répétition est la plus rapide (47 972 ms) et les suivantes montent
  (50 212 à 53 668 ms) : la cause n'est pas isolée (charge ambiante ou
  ordonnancement). Le p50 est retenu.
- **La réplique de compression n'est pas l'appel de production.** Elle compresse
  les mêmes textes, hors attache, et sert de borne : le coût réel est dans l'étape
  `transaction`, mesurée globalement.
- **Un seul segment source** dans les deux index. L'étape `manifest` vaut 6 à 8 ms,
  alors que MES-15 mesure 7,5 à 8,9 ms à 1 000 segments : la marge est faible et
  non isolée (deux fsync, selon MES-15). Un index à plusieurs centaines de segments
  n'est pas mesuré ici.
- **Les lots font 1 000 et 3 002 passages**, pas 3 000 : le générateur découpe par
  document, et 3 000 passages demandés donnent 3 002 passages et 2 517 documents.
  C'est la valeur réelle qui est reportée partout.
- **`synth-large` n'exécute pas la recette** (modèle `external`) : c'est ce qui
  permet d'y voir le commit seul, mais son étape `validation` (7 à 19 ms) n'est pas
  comparable à celle de l'échantillon.
- **`articles` n'est qu'une des six structures** de l'échantillon Legifacile (même
  limite que MES-07) : sa densité (1,18 passage par document) est proche de celle
  de l'échantillon (1,2551, MES-13).
- **Vecteurs aléatoires** dans les lots `synth-large` : même taille, contenu
  différent des vecteurs réels. Aucun effet attendu sur les étapes mesurées, non
  vérifié.

### Conclusion

Ce que la mesure permet d'affirmer :

1. **À l'échelle de Legifacile, une attache quotidienne type dure quelques
   secondes, et c'est le commit de génération qui la paie.** Sur l'index de 8,6 M
   passages, 1 000 passages s'attachent en 3,26 s et 3 002 en 3,33 s (p50), dont
   **96,6 à 98,5 % dans l'étape `snapshot`** (3 207 et 3 219 ms). Le lot ne coûte
   que 18 à 55 ms de transaction, 19 à 33 ms de segment et 6 à 7 ms de manifeste.
   La taille du lot ne change donc presque rien ; la taille de l'index change tout.
2. **La part du commit est exactement celle que MES-15 avait isolée.** Les deux
   balayages complets de `passages` (`created_generation`,
   `documents(updated_generation)`, sans index) coûtent 2,92 s à 8,58 M passages
   vivants dans le relevé hors banc de MES-15, et l'attache réelle les retrouve à
   3,21 s à 7,74 M. Un lot quotidien n'ajoute à ce coût que sa propre écriture.
3. **Avec le modèle réel, c'est la recette qui domine sur l'échantillon** :
   51,5 s (1 000 passages) et 65,9 s (3 002), soit 96,3 à 98,3 % de l'attache. La
   recette ré-encode 200 passages en fp32 quel que soit le lot ; la part fixe
   ajustée est de 44,2 s, et la part par passage de 7,2 ms (ajustement à deux
   points, cause non isolée). Une attache de 1 000 passages sur l'échantillon
   (52,3 s) tient dans `wait_timeout_ms` = 60 s ; celle de 3 002 (68,5 s) ne tient
   pas.
4. **Le gain de `REUTILISER-COMPRESSEUR-ZSTD` est confirmé sur un lot quotidien
   réel** : 32,7 à 38,7 µs par passage, soit 32,7 à 116,2 ms par lot, et 82,9 à
   86,0 % du coût d'avant. C'est le même gain par passage que MES-15 (30 à 41 µs),
   alors que les textes de MES-15 sont huit à neuf fois plus courts : le coût évité
   est la construction du contexte, pas la compression des octets.
5. **Le chemin d'écriture ne dépend ni du modèle ni de l'index** : `transaction`
   vaut 18 à 19 ms pour 1 000 passages et 55 à 58 ms pour 3 002, à 44 890 comme à
   7 740 000 passages vivants. Rien à optimiser là avant longtemps.

Ce que la mesure ne permet **pas** d'affirmer :

- rien sur le coût par ligne ajoutée de l'étape `snapshot` sur l'échantillon
  (≈ 1,0 ms par document ajouté et ≈ 0,8 ms par passage ajouté, entre 823 et
  2 439 ms) : ni sa cause, ni son
  comportement à 8,58 M, où il est masqué par les balayages ;
- rien sur un index à plusieurs centaines de segments, ni sur une attache
  concurrente de lectures (INV-02) ;
- rien sur le disque froid : toutes les répétitions partent d'un clone qui vient
  d'être écrit ;
- rien sur la répartition interne de l'étape `validation` (recette contre comptage
  de jetons), ni sur une recette à `acceptance_sample` différent ;
- rien sur l'étape lexicale de `docs/02`, non implémentée à ce commit.

Cible ou budget : **il n'en existe aucun** pour MES-08 dans `docs/08`, `docs/03` ni
`docs/04`. Le seul délai écrit est `wait_timeout_ms` = 60 s (`docs/05`), qui borne
l'attente du client et non la durée de l'attache : la mesure ne peut donc pas
conclure à un dépassement, mais elle montre qu'une attache de 3 000 passages avec
le modèle réel (68,5 s) le dépasse déjà sur l'échantillon.

Recommandations, **à valider par le propriétaire** :

1. **Ajouter les deux index SQLite de diagnostic** (`passages(created_generation)`
   et `documents(updated_generation)`). C'est la seule action qui change l'ordre de
   grandeur : MES-15 chiffre les deux requêtes à moins de 0,01 ms avec ces index,
   contre 2,92 s sans. Il resterait la part que cette campagne ne sait pas chiffrer
   à 8,58 M (sur l'échantillon, l'étape vaut encore 0,8 à 2,4 s pour 1 000 à 3 000
   passages, sans que son contenu soit isolé). C'est un changement de schéma SQLite,
   donc une décision et un ADR ; le coût d'écriture de ces index sur un index en
   service n'est pas mesuré.
2. **Traiter le délai de recette séparément du budget client.** 44 s de recette
   fixe par attache, quel que soit le lot, dépassent à eux seuls la moitié de
   `wait_timeout_ms`. Trois options, à trancher par le propriétaire : relever
   `wait_timeout_ms` pour les attaches quotidiennes, réduire `acceptance_sample`,
   ou faire tourner la recette hors du chemin d'attente (asynchrone). Aucune n'est
   appliquée.
3. **Ne pas optimiser `segment`, `transaction` ni `manifest`** : 19 à 58 ms au
   total pour 3 002 passages, indépendants du modèle et de l'index, et le gain du
   compresseur zstd est déjà acquis.
4. **Ne pas extrapoler une attache depuis l'échantillon de 50 000 passages** : sur
   cet index, la recette masque le commit (1,6 à 3,6 %), alors qu'à 8,58 M c'est
   l'inverse. Toute campagne future sur le cycle de vie doit inclure un index de
   taille réelle, comme ici `synth-large`.

## Fichiers et données de la campagne

- Rapport brut fusionné : `docs/mesures/MES-08-duree-attache-quotidienne.2026-10-25.json`.
- Rapports par index et relevés de charge : `data/mes08/raw/` (gitignoré).
- Lots : `data/mes08/batches/` (17 Mo, gitignoré). Clones de travail supprimés
  après chaque répétition ; `data/mes08/work/` est vide.
- Banc : `crates/crible/benches/attach_cost.rs`. Scripts : `tools/bench/mes08/run.sh`,
  `make_batches.py`, `merge.py`.
- Disque : 104 Gio libres avant et après ; la campagne n'ajoute que les 17 Mo de
  lots (les index de base sont ceux de MES-07).

## Campagne du 2026-10-26 — rejeu sur `synth-large` après `BORNER-COUT-DU-COMMIT`

Rejeu demandé par la fiche `BORNER-COUT-DU-COMMIT` (phase P3, hors plan) et par la
décision 3 de l'ADR-0066, sur le même index et les mêmes lots que la campagne du
2026-10-25. La partie « échantillon Legifacile avec `mdenseon` » n'est pas
demandée par la fiche et n'est pas rejouée : seule la mesure `synth-large` l'est.

- Tâche : `BORNER-COUT-DU-COMMIT`, étape mesure.
- Références : ADR-0066 (décisions 1, 3 et 4), fiche de la tâche, campagne du
  2026-10-25 ci-dessus (résultats, part du `snapshot`, écarts au protocole).
- Commit de base : `4d9c7c01022aba10b9b143de274a460f7ada5581` (branche
  `tache/BORNER-COUT-DU-COMMIT`). **L'attache mesurée est celle du diff de travail
  non commité de la tâche** : `crates/crible-store/src/attach.rs`,
  `filters_index.rs`, `generation.rs`, `patch.rs`, plus
  `crates/crible-store/Cargo.toml` et `tests/construire_bitmaps.rs`. Le banc
  `crates/crible/benches/attach_cost.rs` n'est **pas** modifié ; les cinq durées
  d'étape viennent toujours du rapport de production (`AttachDurations`,
  `docs/04`). Aucun code n'a été modifié pendant la campagne.
- Machine : MacBook Apple M3 (`Mac15,12`, 8 cœurs), 24 Go de RAM, macOS 15.7.7
  (24G720), APFS, 101 Gio libres avant et après. Mesure **native**, aucun
  conteneur ; banc compilé par `cargo bench` (profil `bench`).
- Charge ambiante : la machine n'était pas au repos. Moyenne de charge 1,59 au
  début et 1,11 à la fin (8 cœurs) ; au début, `opencode` (agent de mesure)
  occupait 33,8 % d'un cœur, `mediaanalysisd` 3,4 %, `mdworker_shared` 1,1 % ; à la
  fin, `fseventsd` 1,1 % et `mds_stores` 0,2 %
  (`data/mes08/replay-raw-2026-10-26/conditions-start.txt` et
  `conditions-end.txt`). C'est la même charge
  de fond que celle signalée le 2026-10-25 (`mediaanalysisd`, `mds_stores`), d'un
  ordre plus faible qu'alors. Son effet n'est pas isolé.
- Jeu de données : `data/synth-large/indexes/synth-large`, inchangé depuis le
  2026-10-25 : 8 600 000 passages, 7 740 000 vivants, un passage par document,
  modèle `synthetic-external`, 1 segment, `records.sqlite` de 3 758 346 240 octets,
  SHA-256 `d07fd173b6199074…` (relevé à la main après la campagne ; la campagne du
  2026-10-25 ne l'avait pas consigné).
- Lots : `daily-synth-1000` (1 000 passages, 905 documents, 1 132 octets de texte
  moyen) et `daily-synth-3000` (3 002 passages, 2 517 documents, 1 221 octets),
  ceux du 2026-10-25, réutilisés tels quels (`data/mes08/batches/`, 17 Mo).
- Configuration : 5 répétitions par lot, une attache par répétition sur un clone
  APFS de l'index de base, cache chaud (le clone vient d'être écrit), aucun
  préchauffage séparé — protocole du 2026-10-25 sans changement.

### Protocole

```sh
# une fois, hors mesure : compiler le banc (profil `bench`)
cargo build --release
cargo bench -p crible --bench attach_cost --no-run

# rejeu du seul index synth-large, sortie datée séparée
MES08_ONLY=synth-large MES08_DAY=2026-10-26 \
MES08_RAW="$PWD/data/mes08/replay-raw-2026-10-26" \
    sh tools/bench/mes08/run.sh
```

`run.sh` est réutilisé avec trois variables d'environnement, toutes dans le script
et aucune dans le banc : `MES08_ONLY` restreint la campagne à un index, pour ne pas
rejouer les 52 à 68 s par attache de l'échantillon `mdenseon` ni exiger
`ORT_DYLIB_PATH` ; `MES08_RAW` déplace le dossier brut, pour que le rejeu n'écrase
ni `articles.json` ni `synth-large.json` du 2026-10-25 ; `MES08_DAY` nomme la date
de campagne, parce que le défaut `date -u` désigne encore le 2026-10-25 à 00 h 46
heure locale.

### Résultats

Attache complète, 5 répétitions, en millisecondes, sur 7 740 000 passages vivants.

| Lot | p50 | p95 | p99 | min | max |
|---|---|---|---|---|---|
| 1 000 passages | **63** | 72 | 72 | 62 | 72 |
| 3 002 passages | **143** | 147 | 147 | 141 | 147 |

Décomposition par étape, p50 en millisecondes ; la part est celle du p50 de
l'attache. Les durées d'étape viennent du rapport de production (`AttachDurations`)
et sont arrondies à la milliseconde dans le JSON fusionné. La somme des cinq vaut
64 ms pour la première ligne et 142 ms pour la seconde : la décomposition couvre
tout l'appel, à 1 ms près, et il n'y a pas de « reste » significatif, comme le
2026-10-25.

| Lot | Attache | `validation` | `segment` | `transaction` | `manifest` | `snapshot` |
|---|---|---|---|---|---|---|
| 1 000 | 63 | 8 (12,7 %) | 19 (30,2 %) | 19 (30,2 %) | 8 (12,7 %) | **10 (15,9 %)** |
| 3 002 | 143 | 19 (13,3 %) | 32 (22,4 %) | 55 (38,5 %) | 7 (4,9 %) | **29 (20,3 %)** |

Part de l'étape `snapshot`, c'est-à-dire du commit de génération, et comparaison
avec la campagne du 2026-10-25 :

| Lot | Attache 10-25 | Attache 10-26 | Écart | `snapshot` 10-25 | `snapshot` 10-26 | Écart | Part 10-25 | Part 10-26 |
|---|---|---|---|---|---|---|---|---|
| 1 000 | 3 257 | **63** | −3 194 (−98,1 %) | 3 207 | **10** | −3 197 (−99,7 %) | 98,5 % | **15,9 %** |
| 3 002 | 3 333 | **143** | −3 190 (−95,7 %) | 3 219 | **29** | −3 190 (−99,1 %) | 96,6 % | **20,3 %** |

Compression des textes du lot, réplique hors attache, p50 de cinq répétitions, en
millisecondes pour le lot entier :

| Lot | Textes | Contexte réutilisé | Un contexte par texte | Gain |
|---|---|---|---|---|
| 1 000 | 1 000 | 12,10 | 47,00 | 34,9 (−74,3 %) |
| 3 002 | 3 002 | 19,77 | 118,06 | 98,3 (−83,3 %) |

Le maximum de RSS du processus de banc est de 566 Mio, contre 591 Mio le
2026-10-25.

Deux lectures immédiates :

1. **L'attache ne paie plus la taille de l'index.** Elle passe de 3,26 à 0,063 s
   pour 1 000 passages et de 3,33 à 0,143 s pour 3 002, et l'étape `snapshot`
   tombe de 3 207 à 10 ms (−99,7 %) et de 3 219 à 29 ms (−99,1 %) : c'est la
   disparition des deux balayages complets que MES-15 chiffrait à 2,92 s au § 10.5
   de son rapport, et que le rejeu du 2026-10-26 de MES-15 ramène à 63,2 ms de
   « reste » à 8,58 M.
2. **L'étape `snapshot` suit désormais le lot, non l'index.** Elle valait 3 207 et
   3 219 ms, plate en taille de lot (+0,4 %) le 2026-10-25 ; elle vaut 10 et 29 ms
   (+190 %), soit ≈ 9,5 µs par passage ajouté, pour un travail par ligne ajoutée
   qui reste non isolé. C'est le comportement attendu d'une dérivation bornée par
   le delta : à 7,74 M passages vivants, elle ne dépend plus que de ce que
   l'attache écrit.

### Conclusion

Ce que la mesure permet d'affirmer :

1. **Sur `synth-large`, une attache quotidienne type ne dure plus quelques
   secondes mais quelques dizaines à quelques centaines de millisecondes** :
   63 ms pour 1 000 passages et 143 ms pour 3 002 (p50), contre 3 257 et 3 333 ms
   le 2026-10-25. La part du commit de génération, 96,6 à 98,5 % le 2026-10-25,
   tombe à 15,9 % et 20,3 %.
2. **Les quatre autres étapes sont inchangées**, aux valeurs du 2026-10-25 :
   `validation` 8 et 19 ms (7 et 19), `segment` 19 et 32 ms (19 et 33),
   `transaction` 19 et 55 ms (18 et 55), `manifest` 8 et 7 ms (7 et 6). L'écart
   est dans la dispersion de cinq répétitions.
3. **Le gain de la tâche est celui de MES-15, vu par l'attache réelle** : −3,19 s
   par attache aux deux tailles de lot, soit la quasi-totalité de ce que la
   campagne du 2026-10-25 attribuait au commit.

Ce que la mesure ne permet pas d'affirmer :

- Rien sur l'échantillon Legifacile avec le modèle réel : la recette (`validation`,
  51 à 66 s) n'est pas rejouée ici. L'extrapolation du 2026-10-25 (« ≈ 55 s pour
  1 000 passages et ≈ 69 s pour 3 000, dont 93 à 96 % de recette ») n'est pas
  infirmée, mais elle n'est pas vérifiée : sur `synth-large`, le modèle est
  `external` et la recette ne tourne pas.
- Rien sur un index à plusieurs centaines de segments, ni sur une attache
  concurrente de lectures (INV-02) : l'index mesuré a un segment.
- Rien sur le disque froid : chaque répétition part d'un clone qui vient d'être
  écrit.
- Rien sur le coût par ligne ajoutée de l'étape `snapshot` (≈ 9,5 µs par passage,
  entre 1 000 et 3 002 passages) : ni sa cause, ni son comportement au-delà.
- Rien sur la réplique de compression : elle vaut 12,10 ms pour 1 000 textes ici
  contre 6,07 ms le 2026-10-25, à textes et niveaux identiques ; elle est hors du
  chemin d'attache, l'écart n'est pas isolé et n'entre dans aucune conclusion.

Cible ou budget : **il n'en existe aucun** pour MES-08 dans `docs/08`, `docs/03` ni
`docs/04`. Le seul délai écrit reste `wait_timeout_ms` = 60 s (`docs/05`) : à
143 ms, l'attache en consomme 0,24 %, contre 5,6 % avant la tâche. Avec le modèle
réel, la recette resterait le terme dominant, comme le 2026-10-25 l'a montré sur
l'échantillon ; ce rejeu ne le mesure pas.

Recommandation, **à valider par le propriétaire** : aucune n'est formulée ici. La
recommandation 1 du 2026-10-25 (ajouter les deux index SQLite) n'a plus d'objet
pour le commit ; elle reste sans effet sur la recette, qui est un autre sujet.

### Écarts au protocole

- **La machine n'était pas au repos** (charge 1,59 puis 1,11), comme le
  2026-10-25 ; la dispersion reste faible (1 000 passages : 62 à 72 ms ;
  3 002 : 141 à 147 ms). Aucune valeur n'est retirée.
- **Pas de préchauffage séparé**, comme le 2026-10-25 : chaque répétition part
  d'un clone APFS neuf.
- **Seul `synth-large` est rejoué** : la fiche ne demande pas l'échantillon, et
  `MES08_ONLY` permet de l'exclure sans toucher au banc.
- **Date des fichiers** : le lot et les points portent la date passée au banc
  (`2026-10-26`) ; la campagne a tourné de 00 h 46 à 00 h 50, heure de Paris, soit
  le 2026-10-25 en UTC (23 h 46 à 23 h 50). Le fichier fusionné porte la date
  locale, `2026-10-26`.
- **`run.sh` modifié** (trois variables d'environnement) ; le banc ne l'est pas.
- **Les durées d'étape sont arrondies à la milliseconde** dans le JSON fusionné :
  la somme des cinq étapes diffère de l'attache de 1 ms au plus sur les p50, et de
  1,9 ms au plus sur une répétition brute (lot de 3 002), comme le 2026-10-25
  (±6 ms).

### Données brutes

`docs/mesures/MES-08-duree-attache-quotidienne.2026-10-26.json` : les deux lots,
5 répétitions chacun, les cinq durées d'étape par répétition, les deux répliques de
compression, le maximum de RSS du processus. Les relevés de charge et le JSON de
point sont dans `data/mes08/replay-raw-2026-10-26/` (gitignoré) ; ceux du
2026-10-25 restent dans `data/mes08/raw/`, intacts.

### Corrections du 2026-10-26 (NETTOYER-P3)

Texte seulement : aucune mesure n'a été rejouée, les chiffres mesurés restent ceux
du 2026-10-25.

- **Jeux de données, index de base** : avant, le rapport ne disait pas d'où
  venaient `data/mes07/indexes/articles` et `data/synth-large/indexes/synth-large`.
  Après, il renvoie au script qui les construit, `tools/bench/mes07/run.sh`
  (fonction `prepare`, idempotente : `tools/gen_synth.py --large`,
  `crible index create`, `crible attach`). Aucune somme de contrôle n'est
  recalculée.
- **Somme des cinq étapes** : avant « −0,01 % à +0,01 % », après « ±0,04 % ». Le
  plus grand écart relatif est sur les points `synth-large` (1 ms sur 3 257 ms et
  sur 3 333 ms, soit 0,03 %) ; sur `articles`, 6 ms sur 52 333 ms valent 0,01 %.
- **Coût par ligne ajoutée de `snapshot`** (deux endroits : § « Part du commit de
  génération » et « Ce que la mesure ne permet pas d'affirmer ») : avant
  « ≈ 1,0 ms par passage ajouté ». Après « ≈ 1,0 ms par document ajouté et
  ≈ 0,8 ms par passage ajouté ». Détail : `(2 439 − 823) / (2 517 − 905) = 1,00 ms`
  par document, `(2 439 − 823) / (3 002 − 1 000) = 0,81 ms` par passage.

## Campagne du 2026-10-26 (nuit) — ventilation de l'étape `validation` (`ACCELERER-RECETTE`, livrable a)

Cette section ne rejoue pas les campagnes précédentes et ne les réécrit pas : elle
ajoute la **ventilation mesurée** que la campagne du 2026-10-25 déclarait ne pas
avoir (`MES-08`, « Ce que la mesure ne permet **pas** d'affirmer : rien sur la
répartition interne de l'étape `validation` »). Elle répond au livrable (a) de la
fiche `ACCELERER-RECETTE` (ADR-0067, décision 3). Aucun contrat ne change :
`acceptance_sample` reste 200, la recette reste en fp32, `acceptance_min_cosine`
reste 0,999, les réglages d'ONNX Runtime restent les défauts de la bibliothèque,
et la recette reste dans le chemin de l'attache avant toute écriture (INV-12).

- Tâche : `ACCELERER-RECETTE` (phase P4, hors plan), livrable (a) seulement. Le
  livrable (b) n'est pas réalisé : aucune ligne de code de production n'a été
  écrite pendant cette campagne.
- Commit : `366c5d018745b3f172d2c3d217abea42b1795354` (branche
  `tache/ACCELERER-RECETTE`). L'attache mesurée est celle de ce commit.
- Machine : MacBook Apple M3 (8 cœurs), 24 Go de RAM, macOS 15.7.7 (24G720),
  APFS, 84 Gio libres avant et après. Mesure **native**, aucun conteneur. Le
  pilote des postes est compilé par `cargo build --release` ; le banc d'attache
  de production par `cargo bench` (profil `bench`), comme le 2026-10-25.
- Charge ambiante : **la machine était nettement plus chargée que le 2026-10-25**
  (2,3 à 2,7 de charge moyenne ce jour-là). Ici, charge moyenne 5,88 au début et
  5,17 à la fin : `mediaanalysisd` 71,6 % d'un cœur, `mds_stores` 42,6 %,
  `mediaanalysisd-access` 36,9 %, Adobe Lightroom 22,8 %, puis `WindowServer` et
  Arc à 20 à 45 % pendant la première moitié de la campagne
  (`data/accelere-recette/conditions-start.txt` et `conditions-end.txt`). L'effet
  n'est pas isolé ; il explique l'écart absolu avec le 2026-10-25 (voir
  « Comparaison avec la campagne du 2026-10-25 »), et il dérive pendant la
  campagne (voir « Écarts au protocole »).
- Jeu de données : `data/mes07/indexes/articles`, inchangé depuis le 2026-10-25 —
  50 000 passages, 44 890 vivants (5 110 morts), 38 175 documents, génération 2,
  époque 1, un segment, `reference_precision` `fp16`, `acceptance_sample` 200,
  `acceptance_min_cosine` 0,999, modèle `mdenseon` (`onnx`),
  `records.sqlite` de 58 933 248 octets, SHA-256
  `b579a29cf1aac8f7b9a3ce30116a5d43044408ce8a7c8e98828ae97ca48fc169`.
  Le graphe fp32 `data/models/mdenseon/model.onnx` fait 1,1 Gio et déclare des
  dimensions de **lot et de séquence dynamiques** (`input_ids` :
  `['batch_ids', 'sequence_ids']`, sortie `['batch_ids', 'sequence_ids', 768]`) :
  un encodage par lots ne demande pas de réexport du modèle.
- Lots : ceux du 2026-10-25, réutilisés tels quels (`data/mes08/batches/articles/`)
  — `daily-articles-1000` (1 000 passages, 905 documents, 2,2 Mo) et
  `daily-articles-3000` (3 002 passages, 2 517 documents, 6,5 Mo). Un troisième
  lot a été écrit **pour cette campagne**, `accelere-200` (200 passages,
  190 documents, 452 K, `make_batches.py daily --seed 20261026 --vectors source`,
  `data/accelere-recette/batches/`) : il sert de point bas où la recette tire ses
  200 passages dans un lot qui n'en contient que 200.

### Protocole

Deux instruments, tous deux **hors du code de production** :

1. `crates/crible-store/examples/acceptance_split.rs`, pilote neuf (aucun
   changement de comportement du moteur : il ouvre le lot avec le lecteur de
   production, lit `records.sqlite` en lecture seule, et appelle l'encodeur et le
   compteur de jetons de production). Trois modes :
   - `load` : charge le graphe fp32, une fois par processus ;
   - `posts` : rejoue et chronomètre chaque poste de `validation`, cinq
     répétitions par lot ;
   - `both` : le même rejeu **juste avant et juste après** un `attach` réel sur un
     clone de l'index, dans le même processus — c'est ce qui encadre la mesure
     contre la dérive de la machine (voir plus bas).
2. `crates/crible/benches/attach_cost.rs`, **inchangé**, pour la durée de
   `validation` que le vrai `attach` rapporte (`AttachDurations`), et pour le lot
   de 200 passages.

```sh
# pilote : chargement fp32 à froid, un processus par répétition, puis à chaud
for i in 1 2 3 4 5; do ./target/release/examples/acceptance_split load \
    --model data/models/mdenseon --repetitions 1 --out data/accelere-recette/load-rep-$i.json; done
./target/release/examples/acceptance_split load \
    --model data/models/mdenseon --repetitions 5 --out data/accelere-recette/load-warm.json

# pilote : les postes, cinq répétitions par lot
./target/release/examples/acceptance_split posts \
    --index data/mes07/indexes/articles --models data/models \
    --batch data/mes08/batches/articles/daily-articles-1000 \
    --repetitions 5 --out data/accelere-recette/posts-daily-articles-1000.json
# idem pour daily-articles-3000

# pilote : réplique avant / attach réel / réplique après, même processus
./target/release/examples/acceptance_split both \
    --index data/mes07/indexes/articles --models data/models \
    --batch data/mes08/batches/articles/daily-articles-1000 \
    --work data/accelere-recette/work --repetitions 3 --after 1 \
    --out data/accelere-recette/both-1000.json

# banc de production : `validation` du vrai attach, 5 répétitions, deux lots
MES08_ONLY=articles MES08_DAY=2026-10-26 \
MES08_RAW="$PWD/data/accelere-recette/attach-raw" \
ORT_DYLIB_PATH=tools/.venv/lib/python3.13/site-packages/onnxruntime/capi/libonnxruntime.1.31.0.dylib \
    sh tools/bench/mes08/run.sh
```

Les tableaux de postes et d'encadrement de cette section viennent de
`tools/bench/accelere-recette/merge.py`, qui relit les JSON bruts de
`data/accelere-recette/` (gitignoré). Le troisième tableau qu'il imprime — la
surcharge de remplissage — est un **calcul** sur les longueurs mesurées
(`lengths-*.json`), et non un temps : la campagne du 2026-10-27 le remplace par
des temps mesurés et corrige ce calcul (voir cette sous-section).

**Ce que chaque poste rejoue.** `ouverture du lot` est `BatchReader::open`
(manifeste, contrôles 4 et 5, sommes de contrôle des fichiers). `validate` est
`BatchReader::validate` (contrôles 6 et 7, une passe complète). Le poste
`comptage des jetons` est `TokenCounter::count_documents`, appelé chunk par
chunk, exactement comme `check_passage_lengths`. Les deux postes de `contrôle 9 :
lecture` et `tirage` sont une **réplique** de `draw_acceptance_sample` : même
priorité blake3 sur `(empreinte du batch_id, index de ligne)`, même plafond de
200, même ordre par index de ligne, même décodage fp16 des vecteurs gardés ;
l'égalité **exacte** des trois statistiques de recette entre la réplique et le
vrai `attach` sur le même lot (min 0,9999913843534255, p1 0,9999919863118626,
p50 0,9999983411024005) prouve que la réplique ré-encode les **mêmes 200
textes**. Le poste `plan_documents` est une réplique des **lectures seulement**
(la requête `SELECT doc_hash, fields FROM documents WHERE doc_id = ?1` par
document, base en lecture seule) : il ne contient ni la construction des
enregistrements, ni les compteurs, ni `retire_passages`.

### Résultats — les postes de `validation`

Cinq répétitions par lot, en millisecondes, p50 avec `[min–max]` entre crochets.
Le chargement fp32 est commun aux deux tailles : c'est le même modèle.

| Poste | 1 000 passages | 3 002 passages |
|---|---|---|
| chargement fp32 (processus neuf, 5 processus) | 1 862,4 [1 849,4–1 946,5] | idem |
| chargement fp32 (rechargement dans un même processus) | 1 827,7 [1 792,1–1 850,5] | idem |
| ouverture du lot (contrôles 4 et 5, sommes de contrôle) | 2,1 [1,9–4,9] | 5,8 [4,2–5,9] |
| contrôles 6 et 7 (`validate`, une passe) | 3,5 [3,0–3,6] | 9,9 [6,6–11,0] |
| contrôle 8 : lecture des passages | 1,7 [1,3–1,7] | 5,2 [3,9–5,9] |
| contrôle 8 : comptage des jetons | 38,9 [31,9–40,3] | 123,4 [102,8–130,0] |
| contrôle 9 : lecture du tirage | 1,9 [1,7–2,0] | 5,9 [4,6–6,4] |
| contrôle 9 : tirage (priorité, texte, vecteur) | 0,5 [0,4–0,7] | 1,0 [0,9–1,0] |
| `plan_documents` : lecture des documents | 0,5 [0,4–0,5] | 1,1 [0,9–1,2] |
| `plan_documents` : lectures SQLite (réplique) | 1,6 [1,1–1,7] | 4,4 [3,1–4,5] |
| **recette : 200 ré-encodages** | **57 015,2 [56 327,8–61 130,1]** | **70 343,6 [69 176,1–71 516,6]** |
| recette : cosinus | 0,3 [0,2–0,3] | 0,3 [0,3–0,3] |
| total des postes (somme mesurée, hors chargement) | 57 066,6 [56 381,9–61 176,4] | 70 511,4 [69 305,4–71 674,7] |

Trois lectures immédiates :

1. **Les 200 ré-encodages font tout.** 57,0 s et 70,3 s, soit 99,8 à 99,9 % de la
   somme des postes. Par passage ré-encodé : **285,1 ms** pour le lot de 1 000 et
   **351,7 ms** pour celui de 3 002. Le chiffre de 221 ms du 2026-10-25 était une
   déduction (44,2 s / 200) ; ici c'est une mesure directe, sur une machine plus
   chargée.
2. **Tout le reste de `validation` tient en 51 ms et 157 ms** : comptage des
   jetons 38,9 et 123,4 ms, contrôles 6 et 7 3,5 et 9,9 ms, ouverture du lot 2,1
   et 5,8 ms, `plan_documents` 2,1 et 5,5 ms, tirage 2,4 et 6,9 ms. Le chargement
   du fp32 (1,86 s) est **du même ordre que tout le travail hors recette réuni**
   pour le lot de 1 000 passages.
3. **Le coût par jeton ré-encodé** est mesuré, pas déduit : les 200 passages tirés
   du lot de 1 000 font 54 991 jetons (p50 165, p90 705, max 1 026), ceux du lot
   de 3 002 en font 65 193 (p50 194, p90 964, max 1 090) — soit 1 037 µs et
   1 079 µs par jeton. Les deux lots tirent 200 passages, mais ceux du second sont
   plus longs (255 559 octets de texte contre 223 094) : c'est la **longueur du
   tirage**, pas la taille du lot, qui fixe le coût de la recette.

### Résultats — la somme des postes retrouve `validation`

Comparer des processus différents ne tient pas sur cette machine : la charge
ambiante dérive (voir « Écarts au protocole »). Le mode `both` encadre donc chaque
mesure : réplique des postes, puis vrai `attach` sur un clone, puis réplique des
postes de nouveau, dans le même processus et pour la même répétition. Chaque
valeur est en millisecondes et **inclut le chargement fp32** de son propre
processus ou de son propre appel.

| Lot | rép. | postes + chargement, avant | `validation` du vrai attach | postes + chargement, après | recette / `validation` |
|---|---|---|---|---|---|
| 1 000 | 0 | 72 144,6 | **70 529,2** | 64 262,0 | 102,1 % |
| 1 000 | 1 | 64 402,4 | **64 812,4** | 64 419,5 | 99,3 % |
| 1 000 | 2 | 64 675,5 | **64 747,7** | 73 209,9 | 99,9 % |
| 3 002 | 0 | 77 668,0 | **79 008,6** | 79 628,9 | 98,0 % |
| 3 002 | 1 | 78 928,0 | **79 549,1** | 81 083,9 | 99,1 % |

La colonne « recette / `validation` » est `(200 ré-encodages + chargement fp32 de
l'attache) / validation` ; le chargement mesuré dans l'attache vaut 1 780,9 à
1 842,6 ms selon la répétition. Pour la répétition 1 du lot de 1 000 (l'encadrement
le plus serré : 64 402,4 / 64 812,4 / 64 419,5), la différence entre la réplique et
`validation` est de **+410 ms (+0,6 %)** ; pour la répétition 0 du lot de 3 002,
+1 340,6 ms (+1,7 %) au-dessus de la réplique d'avant et −620,3 ms en dessous de
celle d'après. **Aucun poste n'est manquant** : la somme des postes mesurés et du
chargement retrouve `validation` à la variabilité de la mesure près, à condition
de comparer des mesures voisines dans le temps.

Pour mémoire, la campagne de référence (banc de production, 5 répétitions, même
séance, machine un peu plus rapide) donne `validation` = **58 181,3 ms**
[56 131,8–63 208,9] pour 1 000 passages et **76 327,1 ms** [75 594,4–79 005,2]
pour 3 002. La somme des postes de la même séance (57 066,6 + 1 862,4 = 58 929,0
et 70 511,4 + 1 862,4 = 72 373,8) la retrouve à +1,3 % et −5,2 % : ces deux écarts sont la dérive de la machine
entre les deux séries de mesures, qui n'est pas monotone (la charge fluctue dans
les deux sens pendant la campagne), et c'est précisément ce que l'encadrement du
tableau ci-dessus élimine.

### Conséquence sur la déduction du 2026-10-25

La campagne du 2026-10-25 ajustait `validation` sur deux points et en tirait une
part fixe de **44,2 s** et une part de **7,2 ms par passage ajouté**, en
attribuant la part fixe à la recette et la part par passage au comptage des jetons
et aux contrôles. La mesure sépare les deux et **infirme la part par passage** :

- `validation` augmente de **14 196,2 ms** (+21,9 %) entre la répétition 1 du lot
  de 1 000 (64 812,4 ms) et la répétition 0 du lot de 3 002 (79 008,6 ms), les
  deux mesures de l'encadrement `both` du tableau précédent ;
- les postes qui dépendent du **lot** augmentent de **106 ms** au total, mais ces
  chiffres-là viennent des **p50 de la séance** (`posts`, cinq répétitions par
  lot) et non de l'encadrement : comptage des jetons +84,5 ms (38,9 → 123,4),
  contrôles 6 et 7 +6,4 ms, ouverture +3,7 ms, lectures des passages et du tirage
  +8,0 ms, `plan_documents` +3,4 ms ;
- les 200 ré-encodages augmentent de **13 161,0 ms** (+21,1 %) entre les mêmes
  deux répétitions de l'encadrement (62 519,3 → 75 680,3 ms), parce que les 200
  passages tirés du lot de 3 002 sont plus longs (54 991 → 65 193 jetons, +18,6 %).

Autrement dit : la recette est **fixe en nombre de passages (200)** mais pas en
coût, puisqu'elle dépend de la longueur des textes tirés ; la taille du lot ne
coûte, elle, que 0,06 ms par passage ajouté. La « part fixe » du 2026-10-25 était
la recette de ce lot-là, et sa « part par passage » était la recette des textes un
peu plus longs du lot de 3 002, pas le comptage des jetons.

### Comparaison avec la campagne du 2026-10-25

| Grandeur | 2026-10-25 | 2026-10-26 (nuit) | Écart |
|---|---|---|---|
| `validation`, 1 000 passages | 51 469 | 58 181 (séance) · 64 812 (encadré) | +13,0 % · +25,9 % |
| `validation`, 3 002 passages | 65 923 | 76 327 (séance) · 79 009 (encadré) | +15,8 % · +19,8 % |
| recette (200 ré-encodages) | non mesurée (44,2 s déduits) | **57 015 · 70 344** | — |
| part de la recette dans `validation` | 86 % · 67 % (déduite de la part fixe) | **88,0 % · 89,0 %** | révisée à la hausse |
| part de la recette + chargement fp32 | non mesurée | **90,8 % · 91,4 %** | — |

L'écart absolu vient de la charge ambiante (5,2 à 5,9 de charge moyenne contre 2,3
à 2,7 le 2026-10-25), pas d'un changement de code : la campagne du 2026-10-25 a
mesuré l'attache de `46c2dd1`, celle-ci `366c5d0`. Le diff de `attach.rs` entre
les deux commits porte le delta d'index de filtres (`CommitDelta`,
`INSERER-POOL-EN-ORDRE`, ADR-0066) et le renommage des points d'arrêt
(`attach_failpoint` → `stop_at_failpoint`) ; aucun hunk ne touche
`check_acceptance` ni la boucle de recette. Le rapport
mesuré aujourd'hui (88,0 % et 89,0 %) ne dépend pas de cette charge : la déduction
du 2026-10-25 le sous-estimait au lot de 3 002 (67 %) parce qu'elle attribuait à la
taille du lot ce qui est la longueur du tirage. C'est ce rapport,
et non les durées absolues, qui porte la conclusion.

### Ce que la mesure ne permet pas d'affirmer

- **La durée interne de `check_acceptance` n'est pas instrumentée.** Les 200
  ré-encodages sont chronométrés par une réplique de la boucle (même appel
  `Model::encode(Side::Document, …)`, mêmes textes, vérifiés par l'égalité exacte
  des statistiques de recette), pas par un chronomètre à l'intérieur de la
  fonction. Ce qu'il faudrait pour l'avoir du code de production, sans changer le
  comportement : ajouter à `AttachDurations` des sous-durées de `validation` —
  `acceptance_load`, `acceptance_encode`, `acceptance_cosine` — et, tant qu'on y
  est, `token_count`, `plan_documents`, `batch_open`. Ce sont des durées de
  télémétrie d'appel : elles ne sont pas persistées dans `batches.report`
  (ADR-0042), donc aucun format ni aucune génération ne change. **Non codé ici**,
  conformément à la consigne de la tâche.
- **`plan_documents` n'est mesuré qu'à travers ses lectures SQLite** (1,6 et
  4,4 ms). La construction des enregistrements, l'analyse des champs JSON, les
  compteurs et `retire_passages` ne sont pas chronométrés. Le lot mesuré ne
  contient que des documents nouveaux : le chemin `replaced` n'est pas exercé.
  Les sous-durées ci-dessus le donneraient pour rien.
- **Rien sur l'encodage par lots** (livrable b) : aucun gain n'est mesuré, seule
  la surcharge de remplissage l'est (voir la conclusion). Le pilote n'appelle que
  `Model::encode`, qui construit une forme `[1, length]`.
- **Rien sur un index de 8,58 M passages** : comme le 2026-10-25, la recette ne
  dépend que du lot et du modèle, mais ce n'est pas mesuré à l'échelle.
- **Rien sur le serveur** : le chargement fp32 (1,86 s) est payé à chaque attache
  par la CLI, alors que le serveur garde la session ouverte (ADR-0051). Les parts
  ci-dessus valent pour la CLI.

### Conclusion — la règle « plus de la moitié de `validation` »

**Oui, pour les deux tailles de lot.** Les 200 ré-encodages de la recette
représentent **88,0 %** de `validation` pour 1 000 passages et **89,0 %** pour
3 002 ; avec le chargement fp32 qu'ils déclenchent, 90,8 % et 91,4 %. Le seuil de
la fiche (la moitié) est dépassé d'un facteur 1,8 dans les deux cas, et la taille
du lot ne change pas la conclusion : la recette tire toujours 200 passages.

Le livrable (b) était donc justifié par cette seule mesure. La recommandation
formulée alors — **encodage par lots triés par longueur, taille de lot interne
8** — reposait uniquement sur la **surcharge de remplissage**, jamais sur un
temps d'encodage : le graphe accepte les dimensions dynamiques, la surcharge vaut
+2,6 % de jetons à B = 4 (lot de 1 000) et +2,4 % (lot de 3 002), +6,4 % et
+6,1 % à B = 8, +11,9 % et +13,2 % à B = 16, +28,2 % et +21,9 % à B = 32 ; B = 8
était le plus grand lot dont la surcharge restait sous les 10 %.

**Cette recommandation est remplacée par la mesure du 2026-10-27** (sous-section
suivante) : le temps d'encodage ne baisse à aucune taille de lot, et la valeur
mesurée de la taille interne est **1**. Les surcharges de remplissage ci-dessus
sont celles du calcul corrigé du 2026-10-27 ; le premier calcul, qui donnait
+26,9 %, +26,6 %, +73,0 % et +62,0 % à B = 16 et B = 32, est **abandonné** et
n'est conservé que sous forme de note dans cette sous-section.

Deux points qui restent valables, quelle que soit la taille de lot :

- **Ne rien optimiser d'autre dans `validation`.** Le comptage des jetons
  (38,9 et 123,4 ms) et tout le reste (moins de 30 ms cumulés) sont deux ordres
  de grandeur sous la recette ; les toucher demanderait de modifier INV-11 ou le
  chemin de l'attache pour un gain invisible.
- **Côté serveur, le chargement fp32 disparaît de l'attache** (session gardée,
  ADR-0051) : la part de la recette y monte encore, à recette constante.

### Écarts au protocole

- **La machine n'était pas au repos, et sa charge a dérivé pendant la campagne**
  (5,88 de moyenne au début, 5,17 à la fin, avec `mediaanalysisd` et `mds_stores`
  actifs). Conséquence mesurée : deux répliques identiques séparées de deux
  minutes diffèrent de 10 à 15 % (`both`, lot de 1 000 : 72 144,6 puis 64 262,0
  pour la même réplique). C'est pourquoi la comparaison retenue est celle du mode
  `both`, qui encadre chaque `validation` par deux répliques ; les durées absolues
  ne sont pas comparables d'une séance à l'autre. Aucune valeur n'est retirée.
- **Un premier passage des postes a été refait.** La réplique du tirage
  construisait un tas-max sur `Reverse((priorité, index))`, donc gardait les
  **plus grandes** priorités au lieu des plus petites : elle ré-encodait 200
  textes différents de ceux de la recette (statistiques de recette différentes,
  48,3 s au lieu de 57,0 s). Le tas a été corrigé en `BinaryHeap<(priorité, index)>`
  — les trois statistiques de recette sont alors identiques au chiffre près — et
  tous les chiffres de cette section viennent des passages corrigés. Les JSON du
  premier passage ne sont pas cités.
- **`run.sh` de MES-08 réécrit un rapport versionné.** Son étape de fusion écrit
  `docs/mesures/MES-08-duree-attache-quotidienne.<date>.json` ; avec
  `MES08_DAY=2026-10-26`, elle a écrasé le rapport du rejeu `synth-large` déjà
  commité. Le fichier a été restauré à l'identique depuis `HEAD`
  (`git diff --quiet` passe), et les relevés bruts de cette campagne sont restés
  dans `data/accelere-recette/attach-raw/`. Aucun autre fichier versionné n'a été
  touché par les mesures.
- **Pas de préchauffage séparé** pour les points d'attache : chaque répétition
  part d'un clone APFS neuf, protocole du 2026-10-25 sans changement. Pour les
  postes, le graphe fp32 est chargé une fois par processus puis réutilisé ; le
  chargement est mesuré à part, à froid (un processus par répétition) et à chaud
  (cinq dans un processus), et les deux valeurs diffèrent de moins de 2 %.
- **Le lot `accelere-200` est neuf** (200 passages, 190 documents) : il ne sert
  qu'à montrer qu'un lot dont tous les passages sont tirés coûte, en `validation`,
  la recette de ses 200 textes (48,3 s p50 [45,3–49,3]) et rien de plus.
- **Un seul segment** dans l'index, un seul modèle : mêmes limites que le
  2026-10-25.

### Données brutes et fichiers

Tout est dans `data/accelere-recette/` (gitignoré, aucune valeur de `.env.prod`) :

- `attach-raw/articles.json` et `attach-raw/accelere-200.json` : les points
  d'attache du banc de production (5 et 3 répétitions), plus `articles.time` et
  `conditions-start.txt` de `run.sh` ;
- `posts-daily-articles-1000.json`, `posts-daily-articles-3000.json` : les postes,
  5 répétitions par lot ;
- `both-1000.json`, `both-3000.json` : les encadrements réplique / attach /
  réplique ;
- `load-rep-1.json` à `load-rep-5.json`, `load-warm.json` : le chargement fp32 ;
- `lengths-1000.json`, `lengths-3000.json` : longueurs en jetons des 200 passages
  tirés, avec la liste triée qui donne les surcharges de remplissage ;
- `batches/accelere-200` : le lot de 200 passages ; `work/` est vide (les clones
  sont supprimés après chaque répétition) ;
- `conditions-start.txt`, `conditions-end.txt` : charge et processus.

Instruments, hors du code de production :
`crates/crible-store/examples/acceptance_split.rs` (pilote),
`tools/bench/accelere-recette/run.sh` (campagne) et
`tools/bench/accelere-recette/merge.py` (tableaux de cette section). Le banc
d'attache, `crates/crible/benches/attach_cost.rs`, n'est pas modifié.

## Campagne du 2026-10-27 — tailles de lot de la recette, et rejeu de MES-08

Cette sous-section mesure ce que la campagne du 2026-10-26 n'avait pas mesuré :
le **temps** d'encodage pour une taille de lot donnée, et non plus seulement la
surcharge de remplissage. Elle mesure aussi le **rejeu de MES-08** que la fiche
`ACCELERER-RECETTE` demande au livrable (c). Elle ne réécrit ni la campagne du
2026-10-25 ni celle du 2026-10-26 : elle les complète et signale ses corrections.

- Tâche : `ACCELERER-RECETTE`, livrables (b) mesuré et (c). Le code du livrable
  (b) — `crates/crible-encode/src/batch.rs`, `Model::encode_batch`,
  `check_acceptance` — était présent dans l'arbre pendant la campagne, non
  modifié, avec `ACCEPTANCE_BATCH_SIZE = 1`.
- Commit : `366c5d018745b3f172d2c3d217abea42b1795354`, plus le diff non commité du
  livrable (b), conservé pour la reproductibilité dans
  `data/accelere-recette/livrable-b/` (`livrable-b.diff`, `batch.rs`,
  `accelere_recette.rs`).
- Machine : MacBook Apple M3 (8 cœurs), 24 Go, macOS 15.7.7 (24G720), mesure
  native, `cargo build --release`, ONNX Runtime 1.31.0 (`ORT_DYLIB_PATH`),
  `mdenseon` fp32, réglages d'ONNX Runtime par défaut (ADR-0052).
- Charge ambiante : même machine chargée que la veille (`mediaanalysisd`,
  `mds_stores`, Lightroom ; relevé `data/accelere-recette/conditions-sweep-start.txt`).
- Échantillons : les 200 passages tirés par la recette, écrits hors ligne par
  `acceptance_split dump` dans `data/accelere-recette/sample-daily-articles-1000.json`
  (54 991 jetons, p50 165, max 1 026) et
  `sample-daily-articles-3000.json` (65 193 jetons, p50 194, max 1 090). Le
  nombre de jetons est identique à celui de la campagne du 2026-10-26 : le tirage
  est le même.

### Protocole des tailles de lot

Le harnais est temporaire et **hors des fichiers versionnés** :
`data/accelere-recette/harness/` (crate `recette-batch-sweep`, dépendances
épinglées `ort = 2.0.0-rc.15`, `tokenizers = 0.23.2`, comme le workspace). Il ne
dépend d'aucune crate de Crible et ne référence `encode_batch` nulle part : il
rejoue le même appel de graphe que `Model::encode_padded_batch` — `input_ids` et
`attention_mask` de forme `[rows, length]`, remplissage après les vrais jetons,
pooling CLS de la ligne 0 de chaque ligne, normalisation selon `model.json` — et
la même session par défaut.

```sh
# échantillon tiré, une fois : les textes et leurs longueurs en jetons
./target/release/examples/acceptance_split dump \
    --index data/mes07/indexes/articles --models data/models \
    --batch data/mes08/batches/articles/daily-articles-1000 \
    --out data/accelere-recette/sample-daily-articles-1000.json

# balayage entrelacé des tailles, trois tours
cd data/accelere-recette/harness
ORT_DYLIB_PATH=/…/libonnxruntime.1.31.0.dylib \
    ./target/release/recette-batch-sweep sweep \
    --model ../../../data/models/mdenseon \
    --sample ../sample-daily-articles-1000.json --sizes 1,4,8,16 --rounds 3

# linéarité : B copies du plus long passage tiré, contre un appel seul
… recette-batch-sweep longest --model ../../../data/models/mdenseon \
    --sample ../sample-daily-articles-1000.json --copies 1,4,8 --rounds 3
```

Trois précautions, dictées par la dérive constatée la veille :

1. **Les tailles sont entrelacées** : un tour exécute B = 1, 4, 8, 16, puis on
   recommence, trois tours. Une dérive de la machine frappe donc les quatre
   tailles, et pas seulement la dernière.
2. **Le B = 1 du code de production encadre le harnais** : `acceptance_split
   posts` (qui appelle `encode_batch` avec la constante de production, donc
   B = 1) est exécuté une fois avant et une fois après le balayage de chaque lot.
3. **La surcharge est recalculée à partir des vraies longueurs d'entrée** (prompt
   et jetons spéciaux compris), pas des longueurs de texte.

### Résultats — le temps ne baisse à aucune taille

Temps total de ré-encodage des 200 passages tirés, en millisecondes : médiane de
trois tours, `[min–max]` entre crochets, puis coût par jeton et surcharge de
remplissage mesurée.

| Lot | B | temps total (ms) | µs / jeton | jetons avec remplissage |
|---|---|---|---|---|
| 1 000 passages (54 991 jetons) | **1** | **56 314,7 [53 728,3–57 428,7]** | 1 024 | 54 991 (+0,0 %) |
| | 4 | 59 681,1 [56 540,7–60 143,3] | 1 085 | 56 448 (+2,6 %) |
| | 8 | 63 633,4 [60 143,9–65 078,7] | 1 157 | 58 504 (+6,4 %) |
| | 16 | 68 440,9 [64 731,2–68 618,7] | 1 245 | 61 552 (+11,9 %) |
| 3 002 passages (65 193 jetons) | **1** | **70 987,2 [67 387,1–72 109,0]** | 1 089 | 65 193 (+0,0 %) |
| | 4 | 73 561,7 [72 928,5–74 531,9] | 1 128 | 66 756 (+2,4 %) |
| | 8 | 77 908,0 [77 251,4–81 719,8] | 1 195 | 69 176 (+6,1 %) |
| | 16 | 86 140,6 [85 755,6–86 762,6] | 1 321 | 73 792 (+13,2 %) |

Encadrement par le code de production (B = 1 réel, `encode_batch` avec la
constante 1) : lot de 1 000, **51 744,5 ms** avant et **57 318,7 ms** après
(941 et 1 042 µs/jeton) ; lot de 3 002, **63 003,6 ms** avant et **71 342,3 ms**
après (966 et 1 094 µs/jeton). Les 56 314,7 et 70 987,2 du harnais tombent entre
ces bornes : le harnais mesure le même travail que le code de production.

Trois constats :

1. **Le temps croît avec la taille de lot, sur les deux lots.** Par rapport à
   B = 1 : +6,0 %, +13,0 % et +21,5 % (lot de 1 000) ; +3,6 %, +9,8 % et +21,4 %
   (lot de 3 002). À aucun moment un lot ne rattrape son remplissage : la
   surcharge de temps dépasse toujours la surcharge de jetons (+3,4, +6,6 et
   +9,6 points sur le lot de 1 000).
2. **Le coût par jeton monte lui aussi** : 1 024 → 1 245 µs (lot de 1 000) et
   1 089 → 1 321 µs (lot de 3 002). Un lot ne partage donc pas le travail d'un
   appel : il paie ses jetons de remplissage et, en plus, un peu plus cher par
   jeton réel.
3. **La dispersion est faible et n'inverse jamais l'ordre** : les trois tours
   donnent B = 1 < B = 4 < B = 8 < B = 16 dans les deux lots, et les intervalles
   `[min–max]` de deux tailles voisines ne se recouvrent pas — au dixième de
   milliseconde près entre B = 4 et B = 8 du lot de 1 000 (60 143,3 contre
   60 143,9), et sans ambiguïté ailleurs (lot de 3 002 : 74 531,9 contre
   77 251,4).

### Résultats — les lignes d'un lot ne s'exécutent pas en parallèle

Le plus long passage tiré, dupliqué B fois dans un seul appel (aucun remplissage
par construction) : si les lignes d'un lot s'exécutaient en parallèle, huit copies
coûteraient bien moins que huit appels.

| Échantillon | B copies | temps (ms), médiane de 3 tours | rapport au B = 1, par tour | médiane |
|---|---|---|---|---|
| 1 000 (1 026 jetons) | 1 | 1 338,1 [1 083,2–1 343,3] | — | — |
| | 4 | 5 317,2 [4 797,0–5 322,6] | 4,43 · 3,96 · 3,97 | **3,97** |
| | 8 | 10 786,7 [10 684,7–10 817,5] | 9,96 · 8,05 · 7,98 | **8,05** |
| 3 002 (1 090 jetons) | 1 | 1 468,4 [1 288,2–1 491,7] | — | — |
| | 4 | 5 894,0 [5 881,7–5 899,2] | 4,58 · 4,01 · 3,95 | **4,01** |
| | 8 | 11 875,0 [11 863,6–11 883,7] | 9,22 · 8,08 · 7,97 | **8,08** |

Huit copies coûtent 8,05 à 8,08 fois un appel seul : **strictement linéaire**.
Le gain d'un lot ne peut donc venir que du parallélisme entre lignes, et il n'y
en a pas avec les réglages par défaut d'ONNX Runtime — ce que confirme
l'observation de `@dev` (processus à ~400 % d'un CPU, donc les fils intra-op
travaillent **dans** une ligne, pas entre les lignes d'un lot).

### Correction de la surcharge de remplissage du 2026-10-26

Le premier calcul de la veille — celui que `merge.py` imprimait encore avant
cette correction — donnait +26,9 % et +26,6 % (B = 16) et +73,0 % et +62,0 %
(B = 32). Ces valeurs étaient **fausses** : le calcul multipliait le
dernier groupe, incomplet, par la taille de lot au lieu de son nombre réel de
passages. Recalculées sur les vraies longueurs d'entrée, elles valent **+11,9 %**
et **+13,2 %** (B = 16), **+28,2 %** et **+21,9 %** (B = 32). Les valeurs de B = 4
et B = 8 (+2,6 %/+2,4 % et +6,4 %/+6,1 %) étaient justes. Le calcul est corrigé
dans `tools/bench/accelere-recette/merge.py` (chaque groupe paie son propre plus
long passage, pondéré par son nombre réel de passages) et le script imprime
désormais +2,6 / +6,4 / +11,9 / +28,2 % et +2,4 / +6,1 / +13,2 / +21,9 % pour
B = 4, 8, 16 et 32. La correction ne change rien à la conclusion de la veille sur
la règle de la moitié, et elle ne change pas non plus le classement des tailles :
la surcharge croît avec B dans tous les cas.

### Conclusion — la taille de lot mesurée est 1

**Aucune taille de lot n'accélère la recette sur cette machine et avec les
réglages par défaut d'ONNX Runtime** : le temps total croît de +3,6 % à +21,5 %
dès B = 4, et le coût par jeton croît avec B. La valeur mesurée de la taille de
lot interne est donc **1**, ce que le code du livrable (b) portait déjà
(`ACCEPTANCE_BATCH_SIZE = 1`) sur la foi d'un relevé rapide de `@dev` ; cette
campagne le confirme avec trois tours entrelacés, deux lots et un encadrement par
le chemin de production.

Il en découle, pour la décision en cours : **le code du livrable (b) n'apporte
rien** et peut être retiré (remplissage, masque d'attention et tri par longueur
compris), le rapport ci-dessus restant la trace de ce qu'il aurait pu valoir. Ce
qui reste utile de ce travail est le **groupement trié par longueur** : il est
déjà écrit, testé, et il ne coûte rien — il devient payant le jour où les lignes
d'un lot s'exécutent réellement en parallèle.

### Rejeu de MES-08 (livrable c)

La campagne d'attache de référence du 2026-10-26 (banc `attach_cost` inchangé,
cinq répétitions par lot, index et lots identiques à ceux du 2026-10-25, relevés
dans `data/accelere-recette/attach-raw/articles.json`) tient lieu de rejeu exigé
par la fiche. Durées en millisecondes, p50 avec `[min–max]`.

| Grandeur | 2026-10-25 | 2026-10-26 (rejeu) | Écart |
|---|---|---|---|
| `validation`, 1 000 passages | 51 469 | **58 181,3 [56 131,8–63 208,9]** | +13,0 % |
| `validation`, 3 002 passages | 65 923 | **76 327,1 [75 594,4–79 005,2]** | +15,8 % |
| durée totale, 1 000 passages | 52 333 | **59 469,4 [57 385,1–64 577,4]** | +13,6 % |
| durée totale, 3 002 passages | 68 467 | **80 165,1 [79 467,7–82 986,6]** | +17,1 % |
| part de `validation` dans la durée totale | 98,3 % · 96,3 % | **97,8 % · 95,2 %** | −0,5 · −1,1 pt |
| `snapshot`, 1 000 · 3 002 | 823 · 2 439 | 1 188,5 · 3 746,8 | +44,4 % · +53,6 % |
| `segment`, 1 000 · 3 002 | 19 · 34 | 33,5 · 61,7 | +76 % · +81 % |
| `transaction`, 1 000 · 3 002 | 19 · 58 | 20,1 · 64,5 | +6 % · +11 % |

Le rejeu confirme la structure de la campagne du 2026-10-25 — `validation`
domine la durée totale à plus de 95 %, `snapshot`, `segment` et `transaction`
restent sous les 5 % — et il s'en écarte en valeur absolue de +13 à +17 %, dans
le même sens et du même ordre que la dérive de charge mesurée cette nuit-là
(charge moyenne 5,2 à 5,9 contre 2,3 à 2,7 le 2026-10-25, `mediaanalysisd` et
`mds_stores` actifs). L'étape `validation` n'a pas changé de code entre les deux
mesures (voir la sous-section précédente) : l'écart est celui de la machine, et
c'est pourquoi les conclusions de cette campagne s'appuient sur des rapports et
non sur des durées absolues.

### Proposition de suite (à valider par le propriétaire)

La recette est **1 ms par jeton** ré-encodé, mesuré entre 941 et 1 094 µs/jeton
selon le moment et le lot, soit 52 à 71 s par attache pour 200 passages. Les
leviers restants, et ce que les chiffres ci-dessus permettent d'en dire :

1. **Réglages et fils d'ONNX Runtime, revus en conteneur en P8** (ADR-0052,
   décision 3). C'est le seul levier qui ne change ni le modèle ni les vecteurs.
   Les mesures ci-dessus montrent que le parallélisme intra-op fonctionne déjà
   (~400 % d'un CPU) mais qu'aucun parallélisme entre lignes n'existe ; il faut
   donc mesurer, en conteneur et sur le matériel de la cible, si les fils
   intra-op sont réglés au nombre de cœurs utiles et si un lot devient alors
   payant. Le protocole de cette sous-section (harnais, tailles entrelacées,
   encadrement par B = 1, `longest` pour la linéarité) est réutilisable tel quel,
   pour une campagne courte. Gain : **non mesuré**, à établir avant toute
   décision.
2. **Fournisseur d'exécution** (par exemple CoreML sur Apple, ou le fournisseur
   par défaut d'un autre matériel). Même protocole, même harnais, un
   fournisseur à la fois ; le coût par jeton est la grandeur à comparer.
   Gain : **non mesuré**, et c'est un choix de dépendance, donc un ADR.
3. **Ré-export du graphe** (fp16, longueur maximale réduite, encodeur plus
   petit) : ADR-0050 et ADR-0057, nouvel ADR. À manier avec les chiffres en main :
   le coût est proportionnel aux jetons ré-encodés, et changer de graphe change
   l'empreinte du modèle, donc **tous** les vecteurs. Extrapolation linéaire, non
   mesurée : 8,58 M passages à ~275 jetons de moyenne font ≈ 2,4 × 10⁹ jetons,
   soit ≈ 28 jours de CPU à 1 024 µs/jeton pour ré-encoder le corpus. Un graphe
   deux fois plus rapide par jeton ramènerait la recette à ~26–36 s par attache,
   mais imposerait ce ré-encodage complet : la recette de 200 passages est
   justement ce qui rend un mauvais graphe détectable en une minute.
4. **Réduire l'échantillon sous 200** n'est pas un levier de mesure : c'est un
   contrat (ADR-0067, décision 1), et il faudrait un ADR pour y toucher.

Rien de tout cela n'est codé, et aucune de ces pistes n'est mesurée ici : la
seule chose que cette campagne établit, c'est que **le lot n'est pas le levier**
et que le coût est linéaire en jetons ré-encodés.

### Fichiers de cette sous-section

Tout est gitignoré, dans `data/accelere-recette/` :

- `harness/` : le crate `recette-batch-sweep` (harnais temporaire) et son
  `target/` ;
- `sample-daily-articles-1000.json`, `sample-daily-articles-3000.json` : les 200
  passages tirés, avec leurs textes et leurs longueurs en jetons ;
- `sweep-1000.log`, `sweep-3000.log`, `longest-1000.log`, `longest-3000.log` :
  les sorties brutes des balayages et des tests de linéarité ;
- `sweep1000-production-{before,after}.json`, `sweep3000-production-{before,after}.json` :
  l'encadrement par le code de production ;
- `livrable-b/` : `livrable-b.diff`, `batch.rs`, `accelere_recette.rs`, copie du
  code du livrable (b) tel qu'il était mesuré ;
- `conditions-sweep-start.txt` : charge et processus au début du balayage.

Le pilote `crates/crible-store/examples/acceptance_split.rs` gagne un mode `dump`
qui écrit l'échantillon tiré ; il ne référence ni `encode_batch` ni le code du
livrable (b), qui peuvent donc être retirés sans casser la reproductibilité de
cette campagne (le harnais, lui, ne dépend d'aucune crate de Crible).
