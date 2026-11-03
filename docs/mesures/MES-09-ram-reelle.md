# MES-09 — RAM réelle (RSS) en régime, par index

## Campagne du 2026-10-27

- **Tâche** : `MESURER-LATENCE-DENSE` (P4), mesure MES-09.
- **Commit** : `5542ac095e4404cf2a4ff294309fbf95dfe50df2`, branche `tache/MESURER-LATENCE-DENSE`.
  **Aucun code de production n'a été modifié pendant la campagne.** L'exemple
  `crates/crible/examples/dense_latency.rs`, les scripts `tools/bench/mes02/` et leur entrée pytest
  dans `tools/pyproject.toml` sont **neufs et non commités au moment de la mesure** : la campagne porte
  donc sur `5542ac0` plus ce harnais, sans changement dans `crates/*/src/`.
- **Machine** : MacBook Air M3, 8 cœurs, 24 Go, macOS 15.7.7 (24G720), APFS, **natif**, pas de conteneur.
  Charge au départ `3.28 3.53 3.59` ; `mediaanalysisd`, Lightroom, `mds_stores` et l'agent tournaient.
  **La mémoire a manqué** : selon les exécutions, de 0 à 1,1 Gio des pages de l'instance sont parties au
  swap pendant la mesure. C'est le fait dominant de cette campagne, et il est déclaré dans chaque
  tableau.
- **Jeu de données** : `synth-large`, copie de mesure `data/mes02/indexes/synth-large`, génération 2,
  époque 1, **7 740 000 vivants**, 860 000 morts, un segment de 8 600 000 vecteurs `f16`
  (13 209 600 000 octets, mappés en lecture). `records.sqlite` : 3,7 Go.
  Codes binaires : 825 600 000 octets. Bitmaps et colonnes : 361 661 936 octets
  (`live` 1 082 408, `keywords` 16 579 528, `columns` 344 000 000).
- **Modèle** : `data/models/mdenseon`, session **fp32** chargée par la première requête encodée
  (`ORT_DYLIB_PATH` vers la roue `onnxruntime` de `tools/.venv`), mesurée **à part** (ADR-0077,
  décision 5).

### Protocole

Trois états, définis par ADR-0077 :

- **(a) index ouvert**, instantané chargé, aucune requête ;
- **(b) après une passe** de 300 requêtes à vecteur fourni, sans session ONNX — c'est le « régime » ;
- **(c) session fp32 chargée**, rapportée à part, dans deux exécutions sans passe (`--skip-pass`) pour
  que les 3,8 à 4,0 Gio de pages de vecteurs d'une passe ne se mêlent pas au chiffre.

Chaque état est lu par `/usr/bin/vmmap -summary` sur le processus lui-même, complété par `ps -o rss=`
pour le total exact : `vmmap` affiche une décimale (`4.0G`), donc son découpage vaut à ±0,05 Gio par
ligne, alors que `ps` donne le RSS au Kio. **Mémoire anonyme = RSS de `ps` − pages `mapped file` de
`vmmap`** ; l'empreinte (`Physical footprint`) est rapportée à côté, parce qu'elle reste stable quand
des pages partent au swap.

Commandes (recette complète : `tools/bench/mes02/run.sh`) :

```
H=./target/release/examples/dense_latency
I="--data data/mes02 --index synth-large --vectors data/mes02/queries.f32"
for run in 1 2 3; do $H rss $I --passes 1 --out data/mes02/raw/rss-base-$run.json; done
for run in 1 2; do $H rss $I --model data/models/mdenseon --skip-pass \
    --out data/mes02/raw/rss-model-cold-$run.json; done
```

Écarts et limites, déclarés :

- **La pression mémoire domine la variance.** Le même état (a) mesure 1148, 665 puis 1148 MiB de RSS
  selon l'exécution, avec 0, 484 puis 0 MiB au swap ; son **empreinte est identique** (1126 MiB). Les
  chiffres du tableau sont donc rapportés à l'empreinte pour la comparaison à `docs/02`, et au RSS pour
  la question posée.
- **`vmmap` arrondit à 0,1 Gio.** Les colonnes « fichiers » et « empreinte » héritent de cet arrondi.
- **Le modèle est mesuré sans passe** : la comparaison (c) − (a) isole la session, mais elle ne dit pas
  ce que coûte la session quand les pages de vecteurs occupent déjà 3,8 à 4,0 Gio. Les deux exécutions
  partent d'états très différents (817 MiB de RSS et 331 MiB au swap pour la première, 140 MiB de RSS
  et 998 MiB au swap pour la seconde) : le RSS de la session est donc peu interprétable, l'empreinte
  l'est davantage.
- **Les deux exécutions du modèle ont été rejouées après la campagne** pour que le document enregistre
  `passes: 0` quand `--skip-pass` ne fait aucune passe (le harnais écrivait `1`) ; les chiffres du
  tableau sont ceux du rejeu, et l'empreinte de la session passe de 1 126 à 1 229 MiB selon l'exécution.

### Résultats

Les trois états de chaque exécution, région par région au-delà de 64 MiB, sont dans
`MES-09-ram-reelle.2026-10-27.json` ; les documents bruts du harnais dans `data/mes02/raw/rss-*.json`.

Mébioctets (1 MiB = 1 048 576 octets). « anon » = mémoire anonyme, « fichiers » = pages adossées au
fichier de vecteurs, « swap » = pages sorties, « empreinte » = `Physical footprint`.

| Exécution | État | RSS | anon | fichiers | swap | empreinte |
|---|---|---|---|---|---|---|
| `rss-base-1` | (a) ouvert | 1 148 | 1 148 | 0 | 0 | 1 126 |
| `rss-base-1` | après le searcher | 1 148 | 1 148 | 0 | 0 | 1 126 |
| `rss-base-1` | (b) après une passe | **4 743** | 852 | 3 891 | 331 | 1 126 |
| `rss-base-2` | (a) ouvert | 665 | 665 | 0 | 484 | 1 126 |
| `rss-base-2` | après le searcher | 51 | 51 | 0 | 1 097 | 1 126 |
| `rss-base-2` | (b) après une passe | **4 948** | 852 | 4 096 | 314 | 1 126 |
| `rss-base-3` | (a) ouvert | 1 148 | 1 148 | 0 | 0 | 1 126 |
| `rss-base-3` | après le searcher | 946 | 946 | 0 | 202 | 1 126 |
| `rss-base-3` | (b) après une passe | **4 855** | 861 | 3 994 | 346 | 1 126 |
| `rss-model-cold-1` | (a) ouvert | 1 148 | 1 148 | 0 | 0 | 1 126 |
| `rss-model-cold-1` | (a) + session fp32 | 1 558 | 1 557 | 1 | 1 161 | **2 253** |
| `rss-model-cold-2` | (a) ouvert | 1 150 | 1 150 | 0 | 0 | 1 126 |
| `rss-model-cold-2` | (a) + session fp32 | 1 567 | 1 566 | 1 | 1 144 | **2 355** |

Comparaison ligne à ligne au tableau de `docs/02-architecture.md:202-219`, qui estime 8,58 M × 768 ;
la mesure porte sur **7 740 000 vivants** et 768 dimensions.

| Pièce | `docs/02` | Mesuré | Verdict |
|---|---|---|---|
| Codes binaires | 0,82 Go, en RAM | 825 600 000 o = 0,83 Go = **787 MiB**, anonyme | conforme |
| Bitmaps (vivants, keyword, bool) | quelques Mo à ~50 Mo, en RAM | `live` 1,03 MiB + `keywords` 15,8 MiB = **16,8 MiB**, anonyme | conforme |
| Colonnes de valeurs | ~34 Mo par champ date, en RAM | **328 MiB** pour 10 colonnes de 4 o à 8,6 M iid (344 000 000 o) | conforme à la règle « 34 Mo par colonne » ; `synth-large` déclare 10 colonnes, d'où le total |
| Modèle ONNX fp32 | plus d'1 Go, RAM, par instance | **+1 127 à +1 229 MiB** d'empreinte, **+741 à +1 427 MiB** de RSS | conforme pour l'empreinte ; le RSS est plus bas que l'empreinte quand une partie de la session part au swap |
| Vecteurs fp16 | 13,2 Go, disque, mmap | 13 209 600 000 o mappés ; **3 891 à 4 096 MiB résidents** après une passe, en `mapped file` | conforme : ces pages ne sont pas possédées |
| SQLite (texte, champs) | ~4 à 5 Go, disque | 3,7 Go sur disque, hors RSS | conforme |
| Minimum d'une instance qui sert | ~2,0 Go | 1 126 MiB (index) + 1 127 à 1 229 MiB (fp32) = **2 253 à 2 355 MiB, soit 2,20 à 2,30 Gio** | conforme à l'ordre de grandeur, 10 à 15 % au-dessus du « ~2,0 Go » |

### Conclusion

**Le RSS en régime d'un index de 7,74 M passages vaut 4,63 à 4,83 Gio (4 743 à 4 948 MiB), dont 3,80 à
4,00 Gio (3 891 à 4 096 MiB) de pages de vecteurs `f16` mappées : la mémoire réellement possédée par
l'index est de 1,12 Gio (1 148 MiB).** Le tableau de `docs/02` est vérifié ligne à ligne. L'écart de la
dernière ligne (2,20 à 2,30 Gio au lieu de « ~2,0 Go ») vient de l'arrondi de la conception, et la ligne
des colonnes de valeurs s'explique par le jeu de données (10 colonnes déclarées), pas par le moteur.

Ce que la mesure établit :

- **La règle « ce qui est parcouru en entier est en RAM, ce qui est lu pour quelques candidats reste
  sur disque » se voit dans le RSS.** L'état (a) porte **1 126 MiB d'empreinte**, soit les codes
  (787 MiB) et les bitmaps et colonnes (345 MiB) — la somme vaut 1 132 MiB, l'écart de 0,5 % venant de
  l'affichage de `vmmap` à trois chiffres significatifs ; l'état (b) ajoute **3 891 à 4 096 MiB** de pages
  `mapped file`, qui sont les lignes `f16` lues par le rescoring, et **rien** d'autre : l'empreinte ne
  bouge pas (1 126 MiB dans les trois états et les trois exécutions).
- **Une instance qui sert des requêtes coûte environ 2,20 à 2,30 Gio**, index et session fp32 comprises :
  1,12 Gio possédés par l'index, plus 1,10 à 1,20 Gio pour la session. C'est 10 à 15 % au-dessus du
  « minimum d'environ 2,0 Go » de `docs/02` : le même ordre de grandeur, pas la même valeur.
- **La session fp32 est un coût d'instance, pas d'index** : +1 127 à +1 229 MiB d'empreinte, et un RSS
  qui varie de +741 à +1 427 MiB selon ce que le swap avait déjà pris, mesurés deux fois sur des états
  de départ très différents (817 MiB et 140 MiB de RSS). Elle ne doit donc pas être additionnée par
  index.
- **Les pages de vecteurs ne sont pas possédées.** Sur les 4,63 à 4,83 Gio du régime, 3,80 à 4,00 Gio
  sont des pages de fichier que le système peut reprendre : MES-03 montre qu'elles disparaissent et que
  le rescoring passe alors de 29,8 à 179,0 ms. Un dimensionnement qui compterait le RSS total comme de
  la mémoire possédée se tromperait d'un facteur 4.
- **Les colonnes de valeurs pèsent plus que les bitmaps.** 328 MiB pour 10 colonnes de 4 o à 8,6 M iid,
  contre 16,8 MiB de bitmaps. À l'échelle de Legifacile, le nombre de champs d'intervalle déclarés est
  le premier facteur du coût fixe, avant les bitmaps.

Ce que la mesure **ne** permet pas d'affirmer :

- le RSS en conteneur (P8), ni la part du noyau et du système dans une instance Docker ;
- le comportement sous une pression mémoire normale : ici 0 à 1,1 Gio sont partis au swap et les états
  (a) et (b) varient d'un facteur 1,7 en RSS (665 contre 1 148 MiB à l'ouverture) pour une empreinte
  identique. Seule l'empreinte est stable ;
- le coût d'une session **INT8** (~0,3 Go selon `docs/02`), non mesuré : la campagne n'a chargé que le
  fp32 ;
- la mémoire d'un index à plusieurs segments ou après compaction, ni celle d'un index lexical
  (`synth-large` a `lexical.enabled = false`).

**Proposition, à valider par le propriétaire** : retenir **1,12 Gio par index de 7,74 M passages** et
**1,10 à 1,20 Gio par instance pour la session fp32** comme chiffres de dimensionnement issus de cette
mesure, soit **2,20 à 2,30 Gio pour une instance qui sert**, en notant qu'ils valent pour `synth-large`
(10 colonnes de valeurs, aucun index lexical). Le jour où Legifacile comptera ses champs d'intervalle,
le chiffre des colonnes devra être recalculé à 34 Mo par colonne, et un index lexical ajoutera la part
mmap de tantivy (MES-06).

## Lecture groupée des vecteurs au rescoring (ADR-0080) — 2026-10-28

- **Tâche** : `GROUPER-LECTURES-VECTEURS` (P4). Contrôle de RSS exigé par la décision 6 d'ADR-0080 :
  prouver que l'option 1 (lecture groupée et parallèle des N lignes `f16` dans un tampon transitoire)
  **ne fait pas grossir la mémoire possédée**. Le détail de la campagne est dans la section datée de
  `docs/mesures/MES-03-latence-rescoring.md` du même jour.
- **Commit** : `07a82d773db3245e6fc680b48baabe61fbd4ac21`, branche
  `tache/GROUPER-LECTURES-VECTEURS`, avec les modifications **non commitées** de l'arbre
  (`crates/crible-search/src/dense/rescore.rs`, `crates/crible-store/src/segment.rs`).
- **Machine** : MacBook Air M3, 8 cœurs, **24 Go**, macOS 15.7.7 (24G720), APFS, **natif**. Au moment
  de cette série, la machine était plus chargée que pendant la campagne du 2026-10-27 : **swap occupé
  7,2 à 7,7 Gio**, `mediaanalysisd`, Lightroom et `mds_stores` actifs. C'est ce qui explique les
  écarts de répartition entre RSS et swap d'un état à l'autre, et cela ne change pas l'empreinte.
- **Jeu de données** : `data/mes02/indexes/synth-large`, copie de mesure **en lecture seule**,
  **7 740 000 vivants**, un segment de 8 600 000 vecteurs `f16` (13 209 600 000 octets, mappés en
  lecture), `records.sqlite` de 3 758 346 240 octets. Codes binaires : 825 600 000 octets. Bitmaps et
  colonnes : 361 661 936 octets. Mêmes valeurs que la campagne du 2026-10-27, donc les chiffres sont
  comparables à mémoire possédée égale.

### Protocole

Deux variantes mesurées, `base` (`HEAD`) et `grouped` (option 1), par le même harnais `dense_latency
rss` que MES-09, sur **300 requêtes à N = 1000, une passe**, plus un échantillonnage de RSS pendant
l'exécution du rescoring à N = 5000 :

```
data/grouper-vecteurs/bin/base/dense_latency rss --data data/mes02 --index synth-large \
    --vectors data/mes02/queries.f32 --passes 1 --out data/grouper-vecteurs/raw/rss-base.json
data/grouper-vecteurs/bin/grouped/dense_latency rss --data data/mes02 --index synth-large \
    --vectors data/mes02/queries.f32 --passes 1 --out data/grouper-vecteurs/raw/rss-grouped.json
data/grouper-vecteurs/bin/grouped/rescore_ab measure --data data/mes02 --index synth-large \
    --vectors data/mes02/queries.f32 --ns 5000 --passes 1 --queries 30 --sample-rss \
    --out data/grouper-vecteurs/raw/rss-run-grouped-5000.json
```

Trois états, lus par `/usr/bin/vmmap -summary` sur le processus lui-même et complétés par `ps -o
rss=`, comme dans la campagne du 2026-10-27 : **après l'ouverture** (instantané chargé, aucune
requête), **après la construction du searcher** (codes binaires en RAM), **après la passe** (régime).
**Mémoire anonyme = RSS de `ps` − pages `mapped file` de `vmmap`** ; l'empreinte (`Physical
footprint`) est rapportée à côté, parce qu'elle reste stable quand des pages partent au swap.

### Résultats

| État | variante | RSS | anonyme | pages de fichier | swap | empreinte |
|---|---|---:|---:|---:|---:|---:|
| après l'ouverture | `base` | 1 148 Mio | 1 148 Mio | 0 Mio | 0 Mio | 1 126 Mio |
| après l'ouverture | `grouped` | 859 Mio | 859 Mio | 0 Mio | 290 Mio | 1 126 Mio |
| après le searcher | `base` | 445 Mio | 445 Mio | 0 Mio | 646 Mio | 1 126 Mio |
| après le searcher | `grouped` | 41 Mio | 41 Mio | 0 Mio | 1 107 Mio | 1 126 Mio |
| après la passe | `base` | 4 938 Mio | **842 Mio** | 4 096 Mio | 316 Mio | **1 126 Mio** |
| après la passe | `grouped` | 4 943 Mio | **847 Mio** | 4 096 Mio | 325 Mio | **1 126 Mio** |

**La mémoire possédée ne bouge pas.** À l'état de régime, l'anonyme vaut 842 Mio pour `base` et
847 Mio pour `grouped` — 5 Mio d'écart, soit 0,6 %, à comparer aux 316 et 325 Mio de swap : la
différence est dans le bruit de la pression mémoire, pas dans une allocation. L'empreinte physique est
**identique au Mio près, 1 126 Mio (1,10 Gio) dans les six états**, y compris quand le RSS chute à
41 Mio parce que 1 107 Mio sont partis au swap. C'est la même lecture que la campagne du 2026-10-27 :
seule l'empreinte est stable, et un dimensionnement qui compterait le RSS total (4 938 Mio, dont
4 096 Mio de pages de fichier reprenables) se tromperait d'un facteur 4.

Le tampon transitoire de l'option 1 — **7,7 Mio à N = 5000** (5 000 × 1 536 octets) — n'est pas
visible dans ces chiffres, et la section le déclare : il est libéré avant le tri, donc absent de
l'état « après la passe », et l'échantillonnage de `ps` pendant une exécution de 30 requêtes à
N = 5000 ne le résout pas. Les maxima relevés valent 2 975 Mio pour `base` et 2 992 Mio pour
`grouped`, au-dessus d'un premier échantillon de 720 et 1 074 Mio : l'écart de 17 Mio est dominé par
la croissance des pages de fichier (2 255 et 1 918 Mio), et **la mesure ne sépare pas l'anonyme du
fichier à cette résolution** (période 100 ms, 37 et 19 échantillons). La preuve demandée par
ADR-0080 porte sur la mémoire possédée, et elle est faite ; la trace du tampon dans le RSS anonyme
reste établie par le code, pas par cette mesure.

### Conclusion

- **La variante groupée ne coûte pas de mémoire possédée** : 847 Mio d'anonyme contre 842 pour `base`
  après une passe de 300 requêtes à N = 1000, et **1 126 Mio d'empreinte dans les deux cas**.
- **Les chiffres de dimensionnement de MES-09 restent valables** : 1,10 à 1,12 Gio possédés par index
  de 7,74 M passages servis par `synth-large`, auxquels s'ajoutent 4,10 Gio de pages de fichier que
  le système peut reprendre — et qui expliquent la latence froide mesurée dans MES-03.
- **Le tampon de 7,7 Mio n'est pas mesuré**, seulement borné par le code et par sa taille de calcul.

Ce que la mesure **ne** permet pas d'affirmer :

- la mémoire d'un index à plusieurs segments, après compaction, ni celle d'un index lexical ;
- le RSS en conteneur (P8) ;
- la trace du tampon transitoire dans le RSS anonyme, non résolue par `ps` sous 2 Gio de croissance
  de pages de fichier.

## Campagne en conteneur (P8, ADR-0135) — 2026-11-03

- **Tâche** : `MESURER-EN-CONTENEUR-MAC` (P8), passe A, mesure MES-09.
- **Commit** : `3fb33439aa3349a31593b8281920af7ce51938bb`, branche
  `tache/MESURER-EN-CONTENEUR-MAC`. **Aucun code de production n'a été modifié.** L'exemple
  `crates/crible/examples/dense_latency.rs` est modifié pour Linux (lecture `/proc/self/status` et
  cgroup v2) et **non commité au moment de la mesure**. Binaire `crible`, SHA-256
  `a9658ecb…a399c54` ; harnais, `0eb8487f…307c4fe`.
- **Machine** : MacBook Air M3, 8 cœurs, 24 Go, macOS 15.7.7 (24G720), **en conteneur** : Docker
  Desktop 28.5.1, VM Linux `aarch64` de **12 529 836 032 octets (11,67 Gio)** de RAM et **8 vCPU**,
  **page de 4 096 octets**, volume nommé `crible-mes-p8-data` sur `/data`, **aucune limite
  `--memory`** (`memory.max = max`). La charge de l'hôte n'est pas relevée : limite déclarée.
- **Jeu de données** : `synth-large`, **7 740 000 passages vivants**, 860 000 morts, un segment,
  `vectors.f16` de 13 209 600 000 octets mappé en lecture, `records.sqlite` de 3,7 Go, codes binaires
  de 825 600 000 octets, bitmaps et colonnes de 361 661 936 octets. Mêmes 300 requêtes qu'en natif,
  SHA-256 `03cb2a2b…52948b`.
- **Modèle** : `mdenseon`, session **fp32** chargée par une requête encodée, mesurée à part comme
  ADR-0077, décision 5, l'exige.

### Protocole

Les trois états d'ADR-0077 sont repris sans changement :

- **(a) index ouvert**, instantané chargé, aucune requête ;
- **(b) après une passe** de 300 requêtes à vecteur fourni, **N = 1000** (défaut du mode `rss`), sans
  session ONNX ;
- **(c) session fp32**, deux exécutions **sans passe** (`--skip-pass`), pour ne pas mêler les pages
  de vecteurs d'une passe au chiffre de la session.

Chaque état est lu par `/proc/self/status` (`VmRSS`, `RssAnon`, `RssFile`, `RssShmem`, `VmSize`) et,
quand le conteneur expose cgroup v2, par `memory.current` et `memory.stat`. Trois exécutions pour
(a) et (b), deux pour (c).

```sh
sh tools/bench/mes-p8-conteneur/run.sh mes09
```

soit `dense_latency rss --data /data --index synth-large --vectors /data/queries.f32 --passes 1`,
puis `… --model /data/models/mdenseon --skip-pass`, un conteneur neuf par exécution.

Écarts au protocole, déclarés :

- **Pas d'équivalent de `vmmap`** : Linux ne fournit pas la `Physical footprint` qui sert de mesure
  de la mémoire possédée dans la campagne native. La comparaison porte donc sur le **RSS anonyme**
  contre l'empreinte native, deux grandeurs voisines mais non identiques.
- **Le cgroup ne referme pas toujours ses comptes** : à l'exécution 3, `RssFile` vaut 8 658 MiB quand
  `file` du cgroup vaut 6 810 MiB, et le RSS total (9 804 MiB) dépasse `memory.current`
  (7 987 MiB). Le mécanisme probable — des pages déjà présentes dans le cache de la VM, imputées à
  un cgroup antérieur, entrent dans la projection sans être réimputées — **n'est pas prouvé** : le
  harnais ne lit pas le cgroup parent. Les chiffres sont donnés tels quels.
- **La session est mesurée sans passe**, donc la comparaison (c) − (a) isole la session, mais pas son
  coût quand 8,7 Go de pages de vecteurs occupent déjà la VM.

### Résultats

Documents bruts : `data/mes-p8-conteneur/raw/rss-base-{1,2,3}.json` et
`rss-model-cold-{1,2}.json` (gitignorés) ; artefact versionné :
`MES-09-ram-reelle.2026-11-03.conteneur.json`, qui porte chaque état, les champs cgroup complets et
la référence native.

Mébioctets (1 MiB = 1 048 576 octets), « anon » = `RssAnon`, « fichiers » = `RssFile`,
« cgroup » = `memory.current` :

| Exécution | État | RSS | anon | fichiers | cgroup | récupéré pendant la passe |
|---|---|---:|---:|---:|---:|---:|
| `rss-base-1` | (a) ouvert | 1 148 | 1 141 | 7 | 3 382 | — |
| `rss-base-1` | après le searcher | 1 148 | 1 141 | 7 | 11 072 | — |
| `rss-base-1` | (b) après une passe | **9 972** | 1 147 | 8 825 | 11 216 | 10,7 Gio |
| `rss-base-2` | (a) ouvert | 1 148 | 1 141 | 6 | 5 518 | — |
| `rss-base-2` | après le searcher | 1 148 | 1 142 | 6 | 8 427 | — |
| `rss-base-2` | (b) après une passe | **9 777** | 1 147 | 8 630 | 11 242 | 11,7 Gio |
| `rss-base-3` | (a) ouvert | 1 148 | 1 141 | 7 | 3 353 | — |
| `rss-base-3` | après le searcher | 1 148 | 1 141 | 7 | 4 606 | — |
| `rss-base-3` | (b) après une passe | **9 804** | 1 147 | 8 658 | 7 987 | 6,2 Gio |

« Récupéré » est la **différence** de `pgsteal_kswapd` entre l'état (b) et l'ouverture, convertie en
Gio : 2 800 552, 3 070 424 et 1 625 311 pages de 4 Kio, l'ouverture de la troisième exécution en
comptant déjà 607 410. Pendant la même passe, `pgmajfault` vaut 70 432, 80 709 et 54 401, soit
**275, 315 et 213 MiB** relus depuis le volume.

| Exécution | État | RSS | anon | session (anon) | cgroup |
|---|---|---:|---:|---:|---:|
| `rss-model-cold-1` | (a) ouvert | 1 148 | 1 141 | — | 4 297 |
| `rss-model-cold-1` | avant la session | 1 148 | 1 141 | — | 7 392 |
| `rss-model-cold-1` | (a) + session fp32 | **2 650** | 2 627 | **+1 486** | 6 841 |
| `rss-model-cold-2` | (a) ouvert | 1 148 | 1 141 | — | 4 581 |
| `rss-model-cold-2` | avant la session | 1 148 | 1 142 | — | 7 266 |
| `rss-model-cold-2` | (a) + session fp32 | **2 637** | 2 613 | **+1 472** | 6 681 |

### Conclusion

- **Le conteneur n'ajoute rien à la mémoire possédée de l'index.** À l'ouverture, le RSS vaut
  **1 148 MiB** et l'anonyme **1 141 MiB**, contre 1 148 MiB de RSS et 1 126 MiB d'empreinte en natif
  (2026-10-27) : l'écart de 1,3 % est celui de deux grandeurs voisines, non un coût du conteneur.
- **Le RSS en régime est un chiffre de cache, pas de dimensionnement.** Après une passe, il vaut
  **9 777 à 9 972 MiB**, dont **8 630 à 8 658 MiB de pages de fichier** et **1 147 MiB d'anonyme** :
  2,1 à 2,3 fois le RSS natif (4 743 à 4 948 MiB), parce que la VM n'a pas de swap et garde résidentes
  environ 8,7 Go des 13,2 Go du fichier, là où la machine native en avait laissé partir 3,9 à 4,1 Go
  et 314 à 346 MiB au swap. La VM a néanmoins récupéré **6,2 à 11,7 Gio** pendant la passe
  (`pgsteal_kswapd`) : elle était sous pression, mais sur des pages de fichier. **La mémoire possédée
  ne bouge pas : 1 147 MiB d'anonyme dans les trois exécutions.**
- **La session fp32 coûte 1 472 à 1 486 MiB, entièrement résidente** (aucun swap dans la VM), contre
  **1 127 à 1 229 MiB d'empreinte** en natif, dont une partie était sortie au swap. L'écart de +19,8 %
  à +31,9 % compare un RSS à une empreinte et **ne doit pas être lu comme un coût du conteneur** ;
  il mesure surtout que la session native était partiellement évincée.
- **Le minimum d'une instance qui sert** vaut **2 637 à 2 650 MiB (2,58 à 2,59 Gio)** en conteneur,
  contre 2 253 à 2 355 MiB d'empreinte en natif (+12,0 % à +17,6 %) et « ~2,0 Go » dans `docs/02`
  (+28,8 % à +29,4 %). L'écart au tableau de conception reste celui que MES-09 avait déjà relevé,
  augmenté de la session non évincée et du noyau du conteneur (`kernel` 29 à 61 MiB, `pagetables`
  2,4 à 28 MiB selon l'état).
- **Aucune recommandation de RAM n'est formulée ici.** La décision 5 d'ADR-0135 la place dans la
  passe B, sur la base des états (b) et (c) ; cette section fournit les deux valeurs qui lui
  manquaient : **1 147 MiB possédés par l'index** et **1 472 à 1 486 MiB pour la session fp32**.
- **Le cache des vecteurs n'est pas un plancher** : 8 630 à 8 658 MiB sont évincibles et l'ont été
  en partie pendant la mesure elle-même.

Ce que la mesure **ne** permet pas d'affirmer :

- l'empreinte physique en conteneur, `vmmap` n'existant pas sous Linux ;
- l'écart exact entre session conteneur et session native, les deux grandeurs n'étant pas les mêmes ;
- la fermeture des comptes du cgroup à l'exécution 3, où `memory.current` est inférieur au RSS ;
- le RSS d'un index multi-segments ou d'un index lexical en conteneur ;
- l'effet d'une limite `--memory` : aucune n'était posée.

### Recommandation de RAM pour la VM Docker (passage B) — 2026-11-03

Cette sous-section **ajoute** une lecture de dimensionnement aux chiffres de la campagne en conteneur
ci-dessus. Elle ne mesure rien de nouveau en mémoire : elle confronte ces chiffres à la RAM que la
cible demande (ADR-0135, décision 5), et signale ce que la mesure ne couvre pas.

**Plancher, mesuré.** À l'ouverture, l'index possède **1 147 MiB d'anonyme** (`RssAnon`, identique dans
les trois exécutions) et **6 à 7 MiB de fichier**. Une session fp32 y ajoute **1 472 à 1 486 MiB**,
tous résidents. Le plancher d'une instance qui sert des requêtes en texte est donc
**2 613 à 2 627 MiB d'anonyme** (≈ 2,6 Gio), mesuré, hors noyau et hors démons de la VM.

**Ce que la cible peut garder en cache.** La VM de développement dispose de **11,67 Gio** et elle a
gardé, après une passe, **8 630 à 8 658 MiB de pages de fichier** sur les **12,3 Gio** de vecteurs fp16
et les **3,7 Go** de `records.sqlite` — en récupérant 6,2 à 11,7 Gio pendant la passe. Le Mac mini
M1 16 Go alloue **environ 8 Go** à la VM Docker (`docs/07-stack-et-deploiement.md`, section Docker) :
il lui resterait **≈ 5,4 Gio** de cache une fois le plancher de 2,6 Gio posé.

**Ce qui est demandé par une requête de rescoring.** La passe B de MES-03 a lu, par requête,
**1 214 / 2 448 / 6 210 pages de 4 Kio** de `vectors.f16` pour N = 1000 / 2000 / 5000, soit
**4,74 / 9,56 / 24,26 Mio** par requête et, pour la série de 300 requêtes du harnais,
**1,39 / 2,80 / 7,11 Gio**. Le nombre de pages par requête est mesuré (`mincore`) ; le passage en Gio
de la série de 300 requêtes est une multiplication déclarée, et cette série est un **artefact de
mesure** : un trafic réel ne rejoue pas 300 fois les mêmes candidats.

**Ce que cela donne pour la cible à 8 Go :**

| Besoin | Gio | Tient dans 8 Go ? |
|---|---:|---|
| plancher : index ouvert + session fp32 | 2,55 à 2,57 | oui |
| + pages de candidats, une requête à N = 1000 | +0,005 | oui |
| + pages de candidats, une requête à N = 5000 | +0,025 | oui |
| + série de 300 requêtes à N = 2000 (artefact) | +2,80 | oui, en laissant ≈ 2,6 Gio |
| + série de 300 requêtes à N = 5000 (artefact) | +7,11 | **non** |

Autrement dit : **8 Go suffisent au plancher et à une requête**, mais **pas à garder résidentes les
pages de vecteurs d'une longue série de requêtes à N = 5000**. À 8 Go, les pages de vecteurs fp16
**seront évincées** (12,3 Gio de fichier pour 5,4 Gio de cache disponible) ; le coût du rescoring se
situera entre le chiffre résident (1,10 à 1,64 ms à N = 1000 et 2000, MES-03 passage B) et le chiffre
« froid partiel » (55 à 174 ms), et **aucune mesure de cette campagne ne dit où**.

**Recommandation, à valider par le propriétaire** :

- **minimum : 4 Go** pour la VM de la cible (plancher de 2,6 Gio + marge pour le noyau, les démons et
  la fragmentation), en sachant que le rescoring y sera le plus souvent en lecture de pages ;
- **confortable : 12 Go**, soit ≈ 4,6 fois le plancher de 2,6 Gio, ce qui laisse ≈ 9,4 Gio de cache —
  assez pour la série à N = 5000 telle que mesurée (7,11 Gio) et pour `records.sqlite` ;
- **entre les deux, 8 Go** est le réglage de `docs/07` : il tient le service, pas le régime résident à
  N = 5000. Si la cible reste à 8 Go, retenir **N = 2000** (voir la recommandation de MES-03 passage B),
  en sachant que N = 2000 donne un rappel@10 de **0,9887**, sous la cible de **0,99**
  (`docs/mesures/MES-01-fidelite-scan-binaire.md:145`, ADR-0078) : ce compromis de fidélité, et le
  changement du défaut `rescore_candidates` de 5000 à 2000 qu'il suppose, demandent un **nouvel ADR**.

Ce que cette lecture **ne** permet pas d'affirmer :

- le coût du rescoring à 8 Go ou à 12 Go : seule la VM de 11,67 Gio a été mesurée ;
- la taille de la session **int8**, qui n'a pas été mesurée (seule la session fp32 l'a été) ; la
  recommandation de MES-05 (int8) changerait le plancher, sans chiffre ici ;
- la part du noyau et des démons de la VM dans le RSS, que le cgroup ne referme pas toujours ;
- le comportement d'un index multi-segments ou lexical, jamais mesuré en mémoire.
