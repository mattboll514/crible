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

La somme des cinq étapes égale l'attache à ±6 ms près (−0,01 % à +0,01 %) : la
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
pour 905 documents, 2 439 ms pour 2 517, soit ≈ 1,0 ms par passage ajouté) : les
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
  (≈ 1,0 ms par passage ajouté, entre 823 et 2 439 ms) : ni sa cause, ni son
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
