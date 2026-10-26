# MES-15 — Coût d'un commit de génération selon le nombre de passages vivants et de segments

## 1. Campagne du 2026-10-21

- Tâche : `MESURER-COUT-COMMIT` (hors plan, phase P1, à la demande du propriétaire)
- Références : `docs/08-invariants-et-tests.md` (MES-15), `docs/02-architecture.md`
  (« Générations et point de commit »), ADR-0016 (point de commit), ADR-0019
  (champs en JSON canonique)
- Commit : `12ea703` (branche `tache/MESURER-COUT-COMMIT`) ; aucun code de production
  n'a été modifié pendant la campagne
- Machine : MacBook Apple M3 (8 cœurs : 4 performance, 4 efficacité), 24 Go de RAM
  (25 769 803 776 octets), macOS 15.7.7 (24G720), disque APFS avec 163 Go libres.
  Mesure **native**, aucun conteneur : Docker Desktop était lancé, mais aucun
  conteneur ne servait la mesure. Le banc est un binaire de
  `crates/crible-store/benches/`, compilé par `cargo bench` (profil `bench`,
  optimisations actives).
- Charge ambiante : **non isolée**, donc la machine n'était **pas au repos** comme le
  veut le protocole. La moyenne de charge de l'hôte est passée de 5,55 à 11,49 sur
  8 cœurs (`uptime`, relevé dans `data/mes15/raw/conditions-start.txt` et
  `conditions-end.txt`).
  - **Observé** : les deux processus du runtime du modèle local
    (`/Applications/Ollama.app/Contents/MacOS/Ollama` et
    `/Applications/Ollama.app/Contents/Resources/ollama`) tournaient au moment du
    contrôle, le 2026-10-21 à 22 h 11 CEST, avec 4 h 59 de fonctionnement, donc
    déjà lancés avant l'ouverture de la campagne (21 h 57). C'est ce runtime qui
    sert l'agent de mesure (ADR-0033).
  - **Supposé, non mesuré individuellement** : un éditeur, son helper, Spotlight,
    et Docker Desktop lancé sans conteneur actif ; leur part dans la charge n'a pas
    été isolée.
  - La campagne elle-même n'a lancé aucun autre travail lourd.
  Conséquence : les chiffres sont plutôt **pessimistes** et dispersés. Les p50
  restent reproductibles (voir § 7.2), les p95 et p99 portent cette charge.
- Jeu de données : sept index générés par le banc dans `data/mes15/` (gitignoré),
  768 dimensions, précision fp16, un document pour deux passages. Les fichiers
  `vectors.f16` sont **creux** : ils ont leur longueur réelle (768 × 2 octets par
  passage) sans allouer de blocs, sauf le point de contrôle `i-1000000-s10-empty`
  dont le fichier de vecteurs est vide. Somme de contrôle SHA-256 du
  `epochs/1/records.sqlite` mesuré, par index. Les `segment.json` de ces index
  portent un marqueur `blake3:unverified-control-point` : `segment::read_meta` ne
  vérifie jamais la somme, et le point de contrôle n'en dépend pas.

| Index | Passages | Segments | `records.sqlite` | SHA-256 |
|---|---|---|---|---|
| `i-10000-s10` | 10 000 | 10 | 3,2 Mo | `a56f01fd3fa6318…e1825ee12` |
| `i-100000-s10` | 100 000 | 10 | 32,1 Mo | `795e6e29dc7aa69…cd1285e79` |
| `i-1000000-s10` | 1 000 000 | 10 | 324,9 Mo | `2679d0c3619b6fd…d450160af7` |
| `i-1000000-s10-empty` | 1 000 000 | 10 | 324,9 Mo | `2679d0c3619b6fd…d450160af7` |
| `i-1000000-s1000` | 1 000 000 | 1 000 | 325,0 Mo | `f1ee86b3dffedd0…e9137288e` |
| `i-8580000-s10` | 8 580 000 | 10 | 2 885,1 Mo | `04802183f3ce408…ca2c12d54` |
| `i-8580000-s1000` | 8 580 000 | 1 000 | 2 885,2 Mo | `fb9836d4b41a104…7ab35c9da` |

  Les deux index de 1 M ont le **même** `records.sqlite` octet pour octet : seul le
  fichier de vecteurs les distingue, ce qui fait d'eux un point de contrôle net
  (§ 7.1). Les sommes sont celles des fichiers **mesurés** ; les index se
  régénèrent avec les commandes du protocole, et le banc n'a pas démontré que la
  régénération donne le même fichier octet pour octet. Les tailles de fichiers sont
  décimales (Mo = 10⁶ octets) ; les mémoires du § 5 sont en Mio, et chaque tableau
  le dit. Les tailles et les SHA-256 de ce tableau sont relevés **hors banc**, à la
  main (`stat`, `shasum -a 256`) : le JSON brut ne les porte pas.
- Configuration mesurée : **commit type** = 1 segment neuf de 2 000 passages
  (`SegmentWriter`, 768 dims, fp16), 1 000 documents, 50 pierres tombales choisies
  parmi les iid vivants. 10 répétitions par point, `N` et `S` identiques à chaque
  répétition.

## 2. Protocole

Le banc est `crates/crible-store/benches/commit_cost.rs` (`harness = false`) ; le
pilote complet est `tools/bench/mes15/run.sh`, qui enchaîne préparation, campagne
et fusion des données brutes. Commandes exactes :

```sh
mkdir -p data/mes15/raw

# une fois, hors mesure de RSS : compiler le banc avant la campagne
cargo bench -p crible-store --bench commit_cost --no-run

# préparation d'un index de base (N passages vivants répartis sur S segments)
cargo bench -p crible-store --bench commit_cost -- prepare \
    --index "$PWD/data/mes15/i-1000000-s10" --passages 1000000 --segments 10

# mesure d'un point : 10 répétitions, commit de 2 000 passages + 50 tombstones
# `2>` n'est pas optionnel : `merge.py` refuse un point sans son `.time`
/usr/bin/time -l cargo bench -p crible-store --bench commit_cost -- measure \
    --index "$PWD/data/mes15/i-1000000-s10" --work "$PWD/data/mes15/work" \
    --repeats 10 --commit 12ea703 --machine "Apple M3 / 24 GB / macOS 15.7.7" \
    --date 2026-10-21 --out "$PWD/data/mes15/raw/i-1000000-s10.json" \
    >/dev/null 2>"$PWD/data/mes15/raw/i-1000000-s10.time"

# fusion en un seul fichier de données brutes, à côté de ce rapport
python3 tools/bench/mes15/merge.py docs/mesures/MES-15-cout-commit-generation.2026-10-21.json \
    data/mes15/raw/i-*.json data/mes15/raw/round2-*.json
```

`run.sh` fait exactement ces étapes dans `measure()` ; `prepare` y est appelé sans
`--overwrite`, et refuse donc une cible non vide, comme `tools/gen_synth.py`.

- **Pourquoi des copies** : un commit ajoute 2 000 passages et 50 pierres tombales,
  donc `N` dérive à chaque répétition. Chaque répétition clone l'index de base
  (`cp -cR`, clonefile APFS ; repli `cp -R`) et commite une fois : les dix
  répétitions voient le même `N` et le même `S`. Le type de copie est relevé par
  répétition : `clonefile` pour les 100 répétitions de la campagne.
- **Préchauffage** : avant chaque répétition, le banc lit `records.sqlite` en entier,
  par blocs de 8 Mio, pour que toutes les répétitions partent du même état de cache.
  Il relève la durée de cette lecture (`warm_up_ns`), qui est la mesure du coût
  d'entrée/sortie du fichier.
- **Cache** : **chaud** pour les chiffres du § 3, après ce préchauffage. Le cache
  froid n'est pas une variable contrôlée : purger le cache de pages macOS demande
  `root`. Les éléments mesurés sur ce point sont au § 7.3.
- **Répétitions et percentiles** : 10 répétitions par point ; p50, p95 et p99 par
  rang le plus proche. Sur 10 échantillons, p95 vaut le 10ᵉ et p99 le maximum : les
  deux colonnes mesurent la queue de la campagne, pas une queue de production.
  C'est la convention de **tout** le rapport, y compris pour la médiane des
  répliques : jamais de moyenne des deux valeurs centrales.
- **RSS du processus et compilation** : `/usr/bin/time -l` englobe tout ce que
  l'invocation lance, compilation comprise. Pour cette campagne, le binaire du banc
  existait déjà au premier point (`target/release/deps/commit_cost-a7a9eba644d6144f`,
  écrit à 21:57:09, campagne ouverte à 21:57:16), et le maximum de RSS du premier
  point (76,0 Mio) est très inférieur à un pic de `rustc` : la compilation ne
  figure donc probablement pas dans les chiffres, sans preuve directe. `run.sh`
  précompile désormais le banc (`--no-run`) avant la première mesure.
- **Répliques** : pour décomposer, le banc rejoue les trois lectures et l'écriture du
  commit sur le même état, avec le SQL et les formats de `records.rs`,
  `generation.rs` et `segment.rs` (`segment::read_meta` et
  `records::canonical_json` sont privés : le banc en porte une copie). Chaque
  réplique tourne 4 fois par répétition ; la première passe est écartée et les trois
  suivantes donnent la médiane, au rang le plus proche. Une réplique mesure une
  copie du code de production, pas le code lui-même : c'est écrit dans le rapport
  et le § 9 en donne la limite.
- **Ce que le commit mesuré contient** : la transaction SQLite (segments, documents,
  passages, pierres tombales, `meta`), la reconstruction de l'instantané
  (`build_snapshot` : `meta`, `segment_ids`, un `segment.json` par segment, tri,
  `live_iids`), l'écriture de `manifests/<G>.json` et la publication (`ArcSwap`).
  L'écriture du segment neuf (`SegmentWriter`, 2 000 vecteurs puis fsync et rename)
  est **hors** du commit : elle est relevée à part, car `docs/02` la place avant.
  L'étape tantivy de `docs/02` n'existe pas encore dans le code et n'est donc pas
  mesurée ; ce commit n'est pas l'attache complète de MES-08.

## 3. Résultats

Commit complet, 10 répétitions par point, cache chaud, en millisecondes :

| N | S | p50 | p95 | p99 | min | max |
|---|---|---|---|---|---|---|
| 10 000 | 10 | 106,4 | 133,4 | 133,4 | 97,4 | 133,4 |
| 100 000 | 10 | 110,1 | 121,2 | 121,2 | 105,1 | 121,2 |
| 1 000 000 | 10 | 160,0 | 162,9 | 162,9 | 156,0 | 162,9 |
| 1 000 000 | 10 (vecteurs vides) | 154,3 | 180,3 | 180,3 | 151,6 | 180,3 |
| 1 000 000 | 1 000 | 195,4 | 206,7 | 206,7 | 193,1 | 206,7 |
| 8 580 000 | 10 | 598,3 | 630,5 | 630,5 | 594,4 | 630,5 |
| 8 580 000 | 1 000 | 641,3 | 645,5 | 645,5 | 622,0 | 645,5 |

## 4. Décomposition

Chaque terme est la p50 de sa réplique sur la même répétition, en millisecondes ;
le pourcentage est sa part du commit complet. « Reste » est ce que la décomposition
n'isole pas : allocation du `Vec<Iid>`, tri des segments, `meta` et `segment_ids`,
publication, et l'écart entre la réplique et le code réel.

| N | S | Commit | Transaction (corps) | `COMMIT` SQLite | iid vivants | Métadonnées de segments | Manifeste | Reste |
|---|---|---|---|---|---|---|---|---|
| 10 000 | 10 | 106,4 | 86,0 (80,8 %) | 1,5 (1,4 %) | 0,90 (0,8 %) | 0,33 (0,3 %) | 6,26 (5,9 %) | +11,4 (10,7 %) |
| 100 000 | 10 | 110,1 | 95,0 (86,3 %) | 1,5 (1,4 %) | 6,75 (6,1 %) | 0,51 (0,5 %) | 6,95 (6,3 %) | −0,6 (−0,5 %) |
| 1 000 000 | 10 | 160,0 | 91,9 (57,4 %) | 1,4 (0,9 %) | 62,7 (39,2 %) | 0,37 (0,2 %) | 6,25 (3,9 %) | −2,6 (−1,6 %) |
| 1 000 000 | 1 000 | 195,4 | 100,4 (51,4 %) | 1,5 (0,8 %) | 62,6 (32,0 %) | 15,2 (7,8 %) | 8,40 (4,3 %) | +7,3 (3,8 %) |
| 8 580 000 | 10 | 598,3 | 77,6 (13,0 %) | 1,5 (0,2 %) | 517,8 (86,5 %) | 0,36 (0,1 %) | 6,91 (1,2 %) | −5,7 (−1,0 %) |
| 8 580 000 | 1 000 | 641,3 | 99,0 (15,4 %) | 1,5 (0,2 %) | 534,2 (83,3 %) | 12,8 (2,0 %) | 8,91 (1,4 %) | −15,1 (−2,4 %) |

À l'intérieur de la transaction, toujours en p50 sur la même charge (2 000 passages,
1 000 documents, 50 pierres tombales) :

| N | S | Compression des textes (`encode_all` par ligne) | Mêmes textes, contexte zstd réutilisé | Insertions `passages` | `documents` | Pierres tombales | `meta` |
|---|---|---|---|---|---|---|---|
| 10 000 | 10 | 78,7 | 6,1 | 3,56 | 1,57 | 1,83 | 0,008 |
| 100 000 | 10 | 84,8 | 6,1 | 4,01 | 2,43 | 2,70 | 0,008 |
| 1 000 000 | 10 | 81,5 | 6,1 | 4,07 | 2,74 | 3,34 | 0,008 |
| 1 000 000 | 1 000 | 89,9 | 6,0 | 4,28 | 3,05 | 3,22 | 0,008 |
| 8 580 000 | 10 | 67,2 | 5,9 | 5,13 | 2,43 | 2,73 | 0,008 |
| 8 580 000 | 1 000 | 88,2 | 5,9 | 5,00 | 2,80 | 3,01 | 0,008 |

Le contexte zstd réutilisé est un **point de contrôle** : `records::encode_text`
appelle `zstd::stream::encode_all`, qui construit un contexte par ligne ; le banc
compresse les mêmes 2 000 textes avec un seul `zstd::bulk::Compressor` pour borner
la part évitable. Aucun code de production n'a été touché pour cela.

Coûts unitaires, tirés des p50 des régressions de `N` et de `S` :

| Grandeur | Valeur mesurée | Intervalle |
|---|---|---|
| Lecture des iid vivants | pente marginale de 65, 62 puis 60 ns par passage ajouté | 10 k → 100 k → 1 M → 8,58 M |
| Relecture des métadonnées de segments (boucle `build_snapshot` : `SELECT segment_id`, deux `is_file`, parse, tri) | 15,2 µs par segment (à 1 M, 1 000 segments), 12,8 µs (à 8,58 M, 1 000 segments) | 10 → 1 000 segments |
| Effet de `S` sur le commit entier | +35,4 ms (à 1 M), +43,0 ms (à 8,58 M) pour 990 segments de plus | soit 36 à 43 µs par segment |
| Manifeste | 6,2 à 8,9 ms ; interprétation : deux fsync, **non isolé** (aucune réplique sans fsync n'a été mesurée) | `S` de 10 à 1 000 |
| Écriture du segment neuf (hors commit) | 16,1 à 20,5 ms | 2 000 vecteurs fp16, 7 points |

La loi en `N` est linéaire sur trois décades (10 k → 8,58 M). Une extrapolation
au-delà est possible en le signalant : **à 30 M passages vivants, la seule lecture
des iid coûterait ~1,8 s par commit** si la loi linéaire tient (extrapolé de
8,58 M à 30 M, 60 ns par passage, sans mesure).

## 5. Mémoire

| N | S | Taille de l'instantané (calculée) | `ps` sur l'instantané tenu (p50, min–max) | Max RSS du processus (`/usr/bin/time -l`) |
|---|---|---|---|---|
| 10 000 | 10 | 40,5 Kio | 59,9 Mio (17,4–75,0) | 76,0 Mio |
| 100 000 | 10 | 392,1 Kio | 65,0 Mio (22,7–77,5) | 78,5 Mio |
| 1 000 000 | 10 | 3,82 Mio | 99,1 Mio (61,6–107,8) | 108,8 Mio |
| 1 000 000 | 10 (vecteurs vides) | 3,82 Mio | 97,9 Mio (60,2–110,6) | 111,6 Mio |
| 1 000 000 | 1 000 | 3,96 Mio | 96,4 Mio (70,4–111,4) | 113,4 Mio |
| 8 580 000 | 10 | 32,73 Mio | 747,8 Mio (276,3–824,8) | 857,7 Mio |
| 8 580 000 | 1 000 | 32,87 Mio | 245,8 Mio (222,0–295,7) | 329,7 Mio |

1 Mio = 1 048 576 octets. La taille de l'instantané est **calculée** d'après le
type : 4 octets par iid vivant (`Vec<Iid>`, un `u32`) plus ~152 octets par segment
(`SegmentMeta` : deux `String`, un `BTreeMap`, les champs scalaires), hors capacité
réservée excédentaire du `Vec`.

Les deux colonnes de droite sont **mesurées**, et aucune ne mesure l'instantané :

- la colonne `ps` est un relevé ponctuel par répétition (`sample_rss_bytes`), pris
  quand l'instantané est tenu ; dix relevés par point ;
- la colonne `/usr/bin/time -l` est le maximum de RSS du processus sur toute la
  répétition, relevé par le pilote (`run.sh`) dans le `.time` voisin du JSON de
  point ; elle **englobe les tampons du banc**, qui alloue à 8,58 M un `Vec<i64>` de
  65,5 Mio puis un `Vec<u32>` de 32,7 Mio par passe de réplique.

Les deux colonnes de droite portent la convention du § 2 : p50 au rang le plus
proche, ici comme dans le JSON (`rss_bytes.p50`). Aucune de ces mesures ne
s'explique par le commit. À `N` égal, la reprise du 8 580 000 / 10 donne 274,8 Mio
de `ps` p50 contre 747,8 Mio dans la campagne (rapport 2,72), et 326,0 Mio de max
RSS processus contre 857,7 Mio (rapport 2,63) ; le max de `ps` suit la même pente,
295,7 contre 824,8 Mio (rapport 2,79). Mêmes code et données dans les deux cas : la
dispersion vient de l'allocateur et du moment du relevé, pas de la génération.
Seule conclusion solide : l'instantané pèse 4 octets par iid vivant, soit 32,7 Mio à
8,58 M passages, et cette structure est reconstruite à chaque commit.

## 6. Conclusion

Ce que la mesure permet d'affirmer, à cache chaud, pour un commit de 2 000 passages
et 50 pierres tombales :

1. **Le commit est en O(N) et en O(S), et c'est la matérialisation des iid vivants
   qui porte le coût à l'échelle de Legifacile** : 0,9 ms à 10 k passages (0,8 % du
   commit), 62,7 ms à 1 M (39 %), 517,8 ms à 8,58 M (**86,5 %**). Le coût est
   linéaire, 60 à 65 ns par passage vivant, cache chaud.
2. **La transaction SQLite ne dépend pas de N** : 77,6 à 100,4 ms, dominée par la
   compression zstd d'une trame par passage (67,2 à 89,9 ms, soit 34 à 45 µs par
   passage), alors que les mêmes textes avec un contexte zstd réutilisé coûtent
   5,9 à 6,1 ms. Le `COMMIT` SQLite lui-même coûte 1,4 à 1,5 ms, constant.
3. **La relecture des métadonnées de segments (`segment.json`) est un coût du
   second ordre** : 0,36 ms à 10 segments, 12,8 à 15,2 ms à 1 000 segments. Si une
   attache quotidienne ne crée qu'un segment par jour et que la compaction garde `S`
   sous quelques centaines, cette part reste de l'ordre de 5 ms ; elle ne devient
   visible qu'avec des milliers de segments.
4. **La dimension des vecteurs ne pèse pas** : à N et S égaux, un index dont les
   `vectors.f16` ont leur longueur réelle (1,53 Go apparents, 0 bloc alloué) et un
   index dont le fichier de vecteurs est vide donnent 160,0 ms et 154,3 ms de p50
   (écart de 3,6 %, dans la dispersion de la campagne), pour un `records.sqlite`
   identique octet pour octet. C'est cohérent avec le fait que l'instantané n'ouvre
   jamais `vectors.*`.
5. **Mémoire** : la liste des iid vivants pèse 32,7 Mio à 8,58 M passages, et elle
   est reconstruite à chaque commit.

Ce que la mesure ne permet pas d'affirmer :

- Rien sur le cache froid de façon contrôlée (§ 7.3) : le chiffre de production est
  celui d'un index en service, donc chaud ; sur un index dont les pages sont sur
  disque, ajouter +0,6 s en p50 à 8,58 M, et jusqu'à +2,1 s au pire relevé (§ 7.3).
- Rien sur l'attache complète : la recette, l'écriture du segment (mesurée à part,
  16,1 à 20,5 ms) et l'étape tantivy de `docs/02`, non implémentée, sont hors du commit.
  Ces chiffres ne remplacent donc pas MES-08.
- Rien de fin sur `S` : 52 % de l'effet de `S` sur le commit entier à 1 M et 66 % à
  8,58 M (≈ 18 et 28 µs par segment) ne sont pas isolés par les répliques ; ce reste
  est signalé, pas attribué.
- Rien sur la composition du manifeste (6,2 à 8,9 ms) : l'attribution à deux fsync
  est une **interprétation** tirée du code de `write_atomically`, pas une mesure ;
  aucune réplique sans fsync n'a été jouée.
- Rien sur les seuils de recherche (`N`, `exact_threshold`) : la mesure ne porte que
  sur le commit.

**Budget implicite.** Il n'existe aucune cible chiffrée pour MES-08 ni pour MES-15
dans `docs/08`. Le seul budget écrit est celui du client : un job d'attache avec
`?wait=true` attend au plus `wait_timeout_ms`, 60 s par défaut (`docs/05`). Le commit
mesuré consomme 0,1 s à 10 k passages et 0,6 s à 8,58 M, soit 0,2 à 1 % de ce
budget. La contrainte réelle n'est donc pas le délai du client mais la durée pendant
laquelle l'écrivain unique de l'index est sérialisé (une attache à la fois), plus le
fait que les lecteurs ne sont jamais bloqués (INV-02, WAL) : 0,6 s d'écrivain par
attache reste sans effet mesurable sur les lectures, mais cette mesure ne le vérifie
pas — c'est l'objet d'INV-02 à l'échelle du pool.

**Recommandations, à valider par le propriétaire** (aucune n'a été appliquée) :

1. **`CONSTRUIRE-BITMAPS`** : remplacer `Vec<Iid>` par une bitmap. À 8,58 M, cela
   ramène l'instantané de 32,7 Mio à 1,02 Mio (calculé : un bit par iid) et supprime
   la copie d'un `Vec` de 32,7 Mio à chaque commit. La lecture SQLite des 8,58 M
   lignes resterait : pour supprimer les 0,5 s, il faut **dériver la nouvelle
   bitmap de la précédente** en copie sur écriture (nouveaux iid et pierres
   tombales du changement seulement), ce que `docs/02` prévoit déjà à l'étape 6.
   Proposition : faire de cette dérivation le critère d'acceptation de
   `CONSTRUIRE-BITMAPS`, et rejouer MES-15 après.
2. **Cache des métadonnées de segments** : garder les `SegmentMeta` de l'instantané
   précédent et n'ajouter que le segment neuf, au lieu de relire tous les
   `segment.json`. Gain mesuré disponible : 12,8 à 15,2 ms à 1 000 segments, et
   ~4,7 ms à 366 segments (calculé : 366 × 12,8 µs). À ne retenir que si `S` doit
   dépasser quelques centaines.
3. **Compression des textes, hors MES-15** : `records::encode_text` construit un
   contexte zstd par passage et coûte 34 à 45 µs par ligne, contre ~3 µs avec un
   contexte réutilisé — 67 à 90 ms par attache de 2 000 passages. Proposition :
   porter ce point à MES-08 (durée d'une attache), et décider d'un compresseur
   réutilisé dans une tâche dédiée, avec un test d'égalité des trames.
4. **Aucun seuil** n'est proposé ici : ni `N`, ni `exact_threshold` ne dépendent de
   cette mesure.

## 7. Points de contrôle

### 7.1 Dimension et taille des vecteurs

| Point | `vectors.f16` | Taille apparente | Blocs alloués | Commit p50 | p95 |
|---|---|---|---|---|---|
| `i-1000000-s10` | longueur réelle (768 × 2 × 1 M) | 1,53 Go | 0 | 160,0 ms | 162,9 ms |
| `i-1000000-s10-empty` | fichier vide | 0 | 0 | 154,3 ms | 180,3 ms |

`records.sqlite` identique (même SHA-256, cf. § 1) ; écart de p50 de 3,6 %, dans la
dispersion observée entre répétitions et entre passes. Conclusion : le commit ne
dépend pas des octets de vecteurs.

### 7.2 Répétabilité

Trois points rejoués intégralement après la campagne, même banc, même jour :

| Point | Campagne (p50) | Reprise (p50) | Écart |
|---|---|---|---|
| 1 000 000 / 10 | 160,0 ms | 154,4 ms | −3,5 % |
| 8 580 000 / 10 | 598,3 ms | 595,6 ms | −0,5 % |
| 8 580 000 / 1 000 | 641,3 ms | 644,3 ms | +0,5 % |

### 7.3 Cache froid

Le cache de pages ne se purge pas sans `root` : le froid n'est donc pas une variable
contrôlée, et les chiffres du § 3 sont chauds. Deux éléments mesurés bornent l'effet :

- la requête seule (`SELECT iid FROM passages WHERE dead_since_gen IS NULL ORDER BY
  iid`), passée avec `sqlite3` sur le fichier de 2,88 Go de `i-8580000-s10`, coûte
  **2,26 s à froid puis 0,25 s à chaud** (même requête, deux passages consécutifs) ;
- lors d'une passe **sans préchauffage** du banc dans sa version d'alors, le même
  jour, à 8,58 M / 1 000 segments, la lecture des iid a coûté **2,63 à 3,02 s au
  premier accès** (p50 2,79 s), puis 0,50 à 2,68 s aux passes suivantes (p50
  2,54 s), les trois dernières répétitions retombant à 0,50–0,66 s une fois les
  pages en cache ; le commit complet est allé de 0,62 à 2,70 s (p50 1,25 s), contre
  0,62 à 0,65 s avec préchauffage
  (§ 3).

Les deux bornes ne mesurent pas la même chose, et c'est l'écart du § 4 qui
l'explique. La réplique du § 4 rejoue `RecordStore::live_iids` à l'identique : même
`SELECT`, collecte en `Vec<i64>` puis conversion en `u32` (`records.rs`, méthode
`live_iids`), et pèse 517,8 ms à 8,58 M. Le CLI `sqlite3` parcourt les mêmes lignes
mais ne matérialise rien : d'où 0,25 s. L'écart (≈ 270 ms) est donc la
matérialisation elle-même, allocation et conversion comprises, et non un coût de
lecture disque. Cette attribution vient du code comparé aux deux mesures ; elle n'a
pas été isolée par une réplique dédiée.

Les chiffres `sqlite3` ont été relevés à la main : ils ne sont pas conservés dans les
données brutes. Le relevé `tmp-s1000.json` vient d'un banc antérieur à celui du
dépôt : il ne porte pas le champ `warm_up_ns`, et la passe n'est pas rejouable par
`run.sh` tel que commité. Il a servi de diagnostic, pas de point de campagne.

À l'échelle de Legifacile, un commit dont les pages sont sur disque coûte donc
**+0,6 s en p50 et jusqu'à +2,1 s sur la répétition la plus lente**, par rapport au
même commit à cache chaud (p50 au rang le plus proche, § 2). L'état du cache n'étant
pas contrôlé, ces valeurs bornent l'effet sans le mesurer en régime établi.

## 8. Données brutes

`docs/mesures/MES-15-cout-commit-generation.2026-10-21.json` : les dix points
(sept de la campagne, trois de la reprise), 10 répétitions chacun, le maximum de
RSS du processus par point (`process_max_rss_bytes`, lu par `merge.py` dans le
`.time` du pilote), et toutes les valeurs brutes par répétition pour le commit et
chaque réplique. Les index et les copies de travail restent dans `data/mes15/`
(gitignoré, 6,3 Go).

## 9. Constats hors mesure

- L'ordre du protocole diffère entre le commentaire de module de `generation.rs` et
  son code : le commentaire annonce manifeste puis instantané, `commit` construit
  l'instantané puis écrit le manifeste. Sans effet mesurable (le manifeste est une
  vue dérivée), mais la documentation du module mérite d'être alignée.
- La réplique de `segment::read_meta` et celle de `records::canonical_json` sont des
  copies : si ces fonctions changent, la décomposition peut dériver sans que le
  total bouge. Le « reste » du § 4 (de −2,4 % à +10,7 %) est là pour le montrer.

## 10. Campagne du 2026-10-25 — rejeu après `CONSTRUIRE-BITMAPS` et `REUTILISER-COMPRESSEUR-ZSTD`

Rejeu demandé par la fiche `MESURER-CYCLE-DE-VIE` (P3), pour chiffrer la dérivation
incrémentale de la bitmap des vivants (ADR-0048) et le compresseur zstd réutilisé
(ADR-0048). Même banc, mêmes points `N` et `S` que la campagne du 2026-10-21 (§ 1 à
§ 3), mêmes index de base.

- Tâche : `MESURER-CYCLE-DE-VIE` (phase P3), passage A. Le passage B (MES-07,
  MES-08, `synth-large`) n'est pas dans cette campagne.
- Références : `docs/08-invariants-et-tests.md` (MES-15), ADR-0048,
  `docs/02-architecture.md` (« Générations et point de commit »),
  `crates/crible-store/src/generation.rs` (`build_snapshot_after_commit`),
  `crates/crible-store/src/filters_index.rs` (`FilterIndex::after_commit`),
  `crates/crible-store/src/records.rs` (`TextCompressor`).
- Commit : `46c2dd1edc8fcac06df61ea560c86ecc915de1fc` (branche
  `tache/MESURER-CYCLE-DE-VIE`). **Aucun code de production n'a été modifié
  pendant la campagne.** L'arbre de travail portait, au moment de la mesure, deux
  changements de banc non commités, décrits au § 10.1. Le code de production mesuré
  est donc celui de `46c2dd1` ; le banc mesuré est celui de `46c2dd1` plus ces deux
  changements.
- Machine : MacBook Apple M3 (8 cœurs : 4 performance, 4 efficacité), 24 Go de RAM
  (25 769 803 776 octets), macOS 15.7.7 (24G720), APFS, 134 Go libres. Mesure
  **native**, aucun conteneur. Le banc est un binaire de
  `crates/crible-store/benches/`, compilé par `cargo bench` (profil `bench`,
  optimisations actives).
- Charge ambiante : la machine n'était **pas au repos**, comme le 2026-10-21. La
  moyenne de charge est passée de 5,08 à 5,53 sur 8 cœurs entre le début et la fin
  de la campagne (`data/mes15/replay-raw/conditions-start.txt` et
  `conditions-end.txt`). Entre les deux, `mediaanalysisd` et `mds_stores` ont
  occupé jusqu'à un cœur chacun par moments ; leur part n'a pas été isolée. Les
  p95 et p99 restent à moins de 1 % des p50 sur les points à 8,58 M, et le second
  tour
  (§ 10.4) reproduit les p50 à 0,4 % : l'effet de cette charge sur les chiffres
  comparés ici est faible, sans être mesuré. La campagne du 2026-10-21 avait une
  charge plus forte (5,55 à 11,49), donc les deux campagnes sont dans le même
  régime ou la précédente est plus pessimiste.
- Jeu de données : les **sept index de base de la campagne du 2026-10-21**, dans
  `data/mes15/` (gitignoré, 6,3 Go), inchangés. Leurs `records.sqlite` ont la même
  somme de contrôle que dans le tableau du § 1 (préfixes vérifiés après la
  campagne) : `a56f01fd…`, `795e6e29…`, `2679d0c3…` (deux fois), `f1ee86b3…`,
  `04802183…`, `fb9836d4…`. Un seul ajout : chaque index a reçu un `index.json`
  (§ 10.1), car `GenerationStore::open` refuse désormais un index qui ne déclare
  pas son schéma (constat § 10 de MES-13). Aucune ligne, aucun segment et aucun
  vecteur n'ont changé.
- Configuration mesurée : identique au § 1 — commit type de 1 segment neuf de
  2 000 passages (768 dims, fp16), 1 000 documents, 50 pierres tombales, 10
  répétitions par point, `N` et `S` fixes à chaque répétition.

### 10.1 Ce qui a changé dans le banc, et ce qui n'a pas changé

Trois changements, tous dans le banc ou ses scripts, aucun dans le code de
production :

1. **`crates/crible-store/benches/commit_cost.rs`, réplique de la transaction** :
   elle compresse maintenant les 2 000 textes avec **un seul**
   `zstd::bulk::Compressor`, créé une fois par répétition, comme
   `records::TextCompressor` (ADR-0048). Le niveau reste
   `zstd::DEFAULT_COMPRESSION_LEVEL`, qui est la valeur de
   `TEXT_COMPRESSION_LEVEL` ; la constante de production est `pub(crate)`, donc le
   banc nomme le niveau qu'il reproduit. Avant, la réplique appelait
   `zstd::stream::encode_all` par ligne, c'est-à-dire l'ancienne méthode.
2. **Le point de contrôle `encode_reused_ns` est retiré.** Il mesurait les mêmes
   textes avec un contexte réutilisé, pour borner la part évitable ; c'est
   désormais ce que fait la production, donc la série ferait doublon avec
   `transaction_encode_ns`. La fonction `reused_context_compression` et le champ
   JSON sont supprimés ; le JSON du 2026-10-25 ne porte donc **pas**
   `encode_reused_ns`.
3. **`tools/bench/mes15/replay.sh` est ajouté** : il ne fait que `measure` sur les
   sept index existants, écrit dans `data/mes15/replay-raw/` et fusionne. Il ne
   prépare rien, pour ne pas reconstruire un état de référence dont la somme de
   contrôle est connue. `tools/bench/mes15/run.sh` garde son rôle (campagne
   complète avec préparation) ; son `ROOT` est corrigé de deux à trois niveaux
   remontés, sinon il cherchait `tools/data/mes15` au lieu de `data/mes15`.

Ce que le banc mesure par ailleurs ne change pas : la transaction répliquée garde
le même SQL, les mêmes documents, les mêmes pierres tombales et le même `meta` ;
le commit réel, la réplique des `segment.json`, celle du manifeste et le
préchauffage sont intacts. Deux champs ne décrivent plus l'état courant et ne sont
utilisés dans aucun chiffre de ce rapport :

- `live_iids_ns` rejoue `RecordStore::live_iids`, que `build_snapshot_after_commit`
  **n'appelle plus** ; c'est le contrôle du coût que la dérivation a retiré, pas un
  terme du commit rejoué ;
- `snapshot_bytes_computed` calcule encore `4 × N + 152 × S`, la formule de
  l'ancien `Vec<Iid>` ; la bitmap réelle n'a pas cette taille. Le champ est laissé
  tel quel pour ne pas changer la sortie du banc ; il n'entre dans aucune
  conclusion.

### 10.2 Protocole

```sh
# une fois, hors mesure de RSS : compiler le banc
cargo bench -p crible-store --bench commit_cost --no-run

# campagne complète du rejeu (7 points, 10 répétitions, cache chaud)
sh tools/bench/mes15/replay.sh

# second tour de répétabilité sur trois points
for label in i-1000000-s10 i-8580000-s10 i-8580000-s1000; do
    /usr/bin/time -l sh -c 'cd "$PWD" && exec cargo bench -p crible-store \
        --bench commit_cost -- measure --index "$PWD/data/mes15/'"$label"'" \
        --work "$PWD/data/mes15/replay-work" --repeats 10 \
        --commit 46c2dd1 --machine "Apple M3 / 24 GB / macOS 15.7.7" \
        --date 2026-10-25 \
        --out "$PWD/data/mes15/replay-raw/round2-'"$label"'.json"' \
        >/dev/null 2>"data/mes15/replay-raw/round2-$label.time"
done
```

`replay.sh` fait la même chose dans une boucle, avec les chemins absolus et la
fusion. Le protocole du § 2 s'applique sans changement : dix répétitions, chaque
répétition part d'un clone APFS de l'index de base, préchauffage par lecture
complète de `records.sqlite`, cache chaud, p50/p95/p99 par rang le plus proche. Le
`.time` de chaque point donne le maximum de RSS du processus (`merge.py` l'ajoute
en `process_max_rss_bytes`).

### 10.3 Résultats

Commit complet, 10 répétitions par point, cache chaud, en millisecondes. « Campagne »
est le p50 du 2026-10-21 (§ 3) ; « écart » est `rejeu − campagne`.

| N | S | p50 rejeu | p95 rejeu | p99 rejeu | p50 campagne | Écart | Écart relatif |
|---|---|---|---|---|---|---|---|
| 10 000 | 10 | **41,1** | 42,1 | 42,1 | 106,4 | −65,3 | −61,4 % |
| 100 000 | 10 | **71,9** | 84,9 | 84,9 | 110,1 | −38,3 | −34,7 % |
| 1 000 000 | 10 | **375,8** | 381,1 | 381,1 | 160,0 | +215,8 | **+134,9 %** |
| 1 000 000 | 10 (vecteurs vides) | **375,3** | 390,6 | 390,6 | 154,3 | +221,0 | +143,2 % |
| 1 000 000 | 1 000 | **401,4** | 432,0 | 432,0 | 195,4 | +205,9 | +105,4 % |
| 8 580 000 | 10 | **3 085,1** | 3 103,0 | 3 103,0 | 598,3 | +2 486,7 | **+415,6 %** |
| 8 580 000 | 1 000 | **3 108,1** | 3 122,9 | 3 122,9 | 641,3 | +2 466,8 | +384,6 % |

Sur 10 échantillons, p95 et p99 valent le maximum : ces deux colonnes mesurent la
queue de la campagne, pas une queue de production (convention du § 2). Le contrôle
« vecteurs vides » donne le même résultat que son jumeau à 0,1 % près, comme le
2026-10-21 : le commit ne lit toujours pas les octets de vecteurs.

Décomposition, p50 en millisecondes sur la même répétition. « Reste » est
`commit − corps − COMMIT SQLite − métadonnées de segments − manifeste` ; il porte la
dérivation de la bitmap, la publication et l'écart entre la réplique et le code
réel. La colonne `live_iids` est la **réplique de l'ancienne lecture**, que le
commit n'exécute plus : elle n'est pas soustraite du reste.

| N | S | Commit | Transaction (corps) | `COMMIT` SQLite | `live_iids` (ancien chemin, non appelé) | Métadonnées de segments | Manifeste | Reste |
|---|---|---|---|---|---|---|---|---|
| 10 000 | 10 | 41,1 | 13,4 | 1,42 | 0,9 | 0,37 | 7,49 | 18,4 |
| 100 000 | 10 | 71,9 | 15,8 | 1,39 | 6,3 | 0,36 | 7,80 | 46,5 |
| 1 000 000 | 10 | 375,8 | 16,1 | 1,34 | 59,0 | 0,34 | 8,00 | 350,0 |
| 1 000 000 | 10 (vecteurs vides) | 375,3 | 16,2 | 1,36 | 59,2 | 0,33 | 7,55 | 349,8 |
| 1 000 000 | 1 000 | 401,4 | 16,4 | 1,48 | 58,1 | 13,27 | 8,74 | 361,4 |
| 8 580 000 | 10 | 3 085,1 | 17,1 | 1,43 | 496,5 | 0,36 | 7,49 | 3 058,7 |
| 8 580 000 | 1 000 | 3 108,1 | 17,3 | 1,41 | 504,3 | 12,79 | 8,93 | 3 067,7 |

Deux lectures immédiates :

1. **Le compresseur zstd réutilisé tient sa promesse.** La compression des
   2 000 textes passe de 67,2–89,9 ms (§ 4) à 6,9–7,0 ms, soit 60 à 83 ms de moins
   par attache, ou 30 à 41 µs par passage. Le corps de la transaction passe de
   77,6–100,4 ms à 16,1–17,3 ms, et ne dépend plus de `N` que par les insertions.
2. **Le reste a changé de nature.** Il valait +11,4 à −15,1 ms dans la campagne
   (§ 4) ; il vaut 350 ms à 1 M et 3 059 ms à 8,58 M. C'est ce terme qui explique
   l'écart positif des gros points : la dérivation n'a pas seulement retiré une
   lecture, elle a ajouté des requêtes.

Le § 10.5 isole ces requêtes hors banc, sur une copie en lecture seule de l'index
de 8,58 M.

### 10.4 Répétabilité

Second tour intégral, même banc, même jour, mêmes index, 10 répétitions :

| Point | Campagne (p50) | Second tour (p50) | Écart |
|---|---|---|---|
| 1 000 000 / 10 | 375,8 ms | 377,4 ms | +0,4 % |
| 8 580 000 / 10 | 3 085,1 ms | 3 094,4 ms | +0,3 % |
| 8 580 000 / 1 000 | 3 108,1 ms | 3 116,5 ms | +0,3 % |

Les deux tours se suivent à 0,4 % près, malgré la charge ambiante : les p50 du
§ 10.3 sont reproductibles. Le second tour est dans le JSON brut avec `round: 2`.

### 10.5 Le coût ajouté par `after_commit` : relevé hors banc

`build_snapshot_after_commit` appelle `FilterIndex::after_commit`, qui exécute
quatre requêtes sur les lignes que la génération a estampillées
(`filters_index.rs`) :

| Requête | Plan SQLite sur l'index de 8,58 M | p50 sans index | p50 avec index |
|---|---|---|---|
| passages créés (`WHERE p.created_generation = ?`) | `SCAN p` | 489,0 ms | 0,0 ms |
| documents modifiés (`WHERE d.updated_generation = ?`) | `SCAN p` puis `SEARCH d` | **2 432,6 ms** | 0,0 ms |
| pierres tombales (`WHERE dead_since_gen = ?`) | `SEARCH q USING INDEX passages_dead_since_gen` | 0,1 ms | 0,1 ms |
| documents retirés | `SEARCH p USING COVERING INDEX passages_doc_id` | 0,1 ms | 0,1 ms |

Protocole du relevé : cinq exécutions de chaque requête, en lecture seule, sur
`data/mes15/i-8580000-s10/epochs/1/records.sqlite`, p50 par rang le plus proche ;
puis les mêmes requêtes sur une copie de ce fichier où deux index ont été créés
(`passages(created_generation)`, `documents(updated_generation)`, 6,4 s de
construction). Les valeurs brutes sont dans
`data/mes15/replay-raw/after-commit-scans.json` et
`after-commit-scans-with-indexes.json`. Ce relevé **n'est pas** une mesure du banc
et ne remplace pas le § 10.3 : il explique le « reste ».

Le relevé tourne sur l'état de **base**, où la génération 2 n'existe pas encore :
les deux requêtes rendent 0 ligne. Le balayage, lui, est le même que pendant le
commit ; seule la jointure des lignes trouvées manque (2 000 passages, 1 000
documents dans le commit mesuré). Le coût du balayage domine, et c'est lui que le
tableau chiffre.

Les deux premières requêtes totalisent **2,92 s** en p50, contre 3,06 s de reste à
8,58 M / 10 : c'est l'essentiel du terme. Aucune des deux n'a d'index sur la
colonne qu'elle filtre, et la seconde part d'un balayage complet de `passages`
(2,88 Go) avant de rejoindre `documents`. `EXPLAIN QUERY PLAN` le dit sans
ambiguïté :

```text
SELECT p.iid, p.fields, d.fields, p.dead_since_gen
FROM passages p LEFT JOIN documents d ON d.doc_id = p.doc_id
WHERE p.created_generation = 2 ORDER BY p.iid;
|--SCAN p
`--SEARCH d USING INDEX sqlite_autoindex_documents_1 (doc_id=?) LEFT-JOIN

SELECT p.iid, d.fields FROM documents d JOIN passages p ON p.doc_id = d.doc_id
WHERE d.updated_generation = 2 ORDER BY p.iid;
|--SCAN p
`--SEARCH d USING INDEX sqlite_autoindex_documents_1 (doc_id=?)
```

Une seule exécution de `created_passages` est sortie à 2 929 ms au lieu de 484–489
(charge ambiante) ; le p50 n'en dépend pas, et la valeur est conservée dans le JSON
brut.

### 10.6 Conclusion du rejeu

Ce que la mesure permet d'affirmer, aux points `N` et `S` de la campagne du
2026-10-21, pour le même commit type :

1. **La dérivation incrémentale de la bitmap des vivants a retiré la lecture de
   `live_iids` du commit** : 517,8 ms (S = 10) et 534,2 ms (S = 1 000) à 8,58 M dans
   la campagne ; la réplique de cette lecture coûte toujours 496,5 et 504,3 ms, mais
   le commit ne l'appelle plus. À 10 k et 100 k, le retrait vaut 0,9 et 6,3 ms.
2. **Telle qu'elle est implémentée, elle ajoute cependant deux balayages complets
   de `passages` par commit**, mesurés à 2,92 s en p50 à 8,58 M (§ 10.5). Le commit
   complet passe donc de 598,3 à 3 085,1 ms à 8,58 M / 10 (+415,6 %) et de 160,0 à
   375,8 ms à 1 M / 10 (+134,9 %). À 10 k et 100 k, le solde reste négatif
   (−61,4 % et −34,7 %), porté par le compresseur zstd.
3. **Le compresseur zstd réutilisé est le gain net de la campagne** : 60 à 83 ms
   par attache de 2 000 passages, à tous les points, sans dépendre de `N`.
4. **La dérivation n'est pas en cause dans son principe, mais dans ses requêtes** :
   avec les deux index de diagnostic, les mêmes requêtes tombent sous 0,01 ms
   (§ 10.5). Le coût vient du plan d'exécution, pas de la copie sur écriture des
   structures.

Ce que la mesure ne permet pas d'affirmer :

- Rien sur le commit d'une attache réelle de MES-08 : la recette, l'écriture du
  segment (relevée à part), le lexical et la lecture du lot restent hors du commit.
- Rien sur le coût d'ajouter les deux index en production : le relevé du § 10.5 les
  crée sur une copie, sans mesurer le temps d'écriture d'un index sur un index en
  service, ni l'effet sur la taille de `records.sqlite` (le fichier de diagnostic
  n'a pas été conservé). Ce serait un changement de schéma SQLite, donc une
  décision à part.
- Rien sur un froid contrôlé : le protocole reste celui du § 2, cache chaud.
- Rien sur les points intermédiaires entre 100 k et 1 M : la marche se situe dans
  cet intervalle, non mesuré.

**Comparaison au budget.** Il n'existe toujours aucune cible chiffrée pour MES-15
dans `docs/08`. Le seul budget écrit reste `wait_timeout_ms` (60 s, `docs/05`) : à
3,09 s, le commit consomme 5,2 % de ce budget à 8,58 M, contre 1,0 % avant. La
contrainte réelle reste la sérialisation de l'écrivain unique, pas le délai du
client.

**Recommandations, à valider par le propriétaire** (aucune n'a été appliquée) :

1. **Traiter le coût ajouté par `after_commit` avant toute autre optimisation du
   commit.** Deux voies, à trancher par le propriétaire :
   - ajouter les index `passages(created_generation)` et
     `documents(updated_generation)` — le relevé du § 10.5 les chiffre à moins de
     0,01 ms par requête, mais c'est un changement de schéma SQLite, donc un
     format sur disque à versionner et à tester ;
   - ou dériver les passages créés de la plage d'iid du changement
     (`next_iid` avant le commit) et les documents modifiés du changement lui-même,
     sans requête sur `updated_generation` ; c'est un changement de code, avec le
     test d'égalité `after_commit` / `from_records` qui existe déjà.
   Dans les deux cas, rejouer MES-15 : c'est le seul moyen de savoir si le commit
   revient sous les 598,3 ms du 2026-10-21.
2. **Ne pas citer les 517,8 ms comme un gain acquis.** Le chiffre du rejeu est
   3 085,1 ms à 8,58 M / 10. Tant que le § 10.5 n'est pas traité, la dérivation
   coûte plus qu'elle ne rapporte à l'échelle de Legifacile.
3. **Aucun seuil** n'est proposé : ni `N`, ni `exact_threshold` ne dépendent de ce
   rejeu.

### 10.7 Données brutes

`docs/mesures/MES-15-cout-commit-generation.2026-10-25.json` : les dix points (sept
du rejeu, trois du second tour), 10 répétitions chacun, toutes les valeurs brutes
par répétition pour le commit et chaque réplique, le maximum de RSS du processus
par point. Le relevé hors banc est dans `data/mes15/replay-raw/` :
`after-commit-scans.json`, `after-commit-scans-with-indexes.json`, les `.time` et
les JSON de point. Les index de base restent dans `data/mes15/` (6,3 Go).

## 11. Campagne du 2026-10-26 — rejeu après `BORNER-COUT-DU-COMMIT` (ADR-0066)

Rejeu demandé par la fiche `BORNER-COUT-DU-COMMIT` (phase P3, hors plan) et par la
décision 3 de l'ADR-0066. Il mesure le commit après que `FilterIndex::after_commit`
a reçu le delta que la transaction vient d'écrire, au lieu de le relire dans
SQLite par estampille de génération. Même banc, mêmes points `N` et `S` que la
campagne du 2026-10-21 (§ 1 à § 3) et que le rejeu du 2026-10-25 (§ 10).

### 11.1 Conditions

- Tâche : `BORNER-COUT-DU-COMMIT` (phase P3, hors plan), étape mesure.
- Références : ADR-0066 (décisions 1, 3 et 4), `docs/phases/P3-cycle-de-vie.md`
  (`### BORNER-COUT-DU-COMMIT`), § 10 de ce rapport, `docs/02-architecture.md`
  (« Générations et point de commit »).
- Commit de base : `4d9c7c01022aba10b9b143de274a460f7ada5581` (branche
  `tache/BORNER-COUT-DU-COMMIT`, dernier commit `docs(adr)` d'acceptation de
  l'ADR-0066). **L'implémentation mesurée est le diff de travail non commité de
  la tâche**, et non ce commit : `crates/crible-store/src/attach.rs`,
  `filters_index.rs`, `generation.rs`, `patch.rs`, `crates/crible-store/Cargo.toml`
  (feature `trace` de `rusqlite`, réservée aux cibles de test) et
  `crates/crible-store/tests/construire_bitmaps.rs`. Aucun code n'a été modifié
  pendant la campagne. Le banc `crates/crible-store/benches/commit_cost.rs` n'est
  **pas** modifié par la tâche : il appelle déjà `GenerationStore::commit`, donc
  le chemin réel, et continue de le faire avec la nouvelle signature.
- Machine : MacBook Apple M3 (`Mac15,12`, 8 cœurs : 4 performance, 4 efficacité),
  24 Go de RAM (25 769 803 776 octets), macOS 15.7.7 (24G720), APFS, 102 Gio
  libres avant la campagne et 101 Gio après. Mesure **native**, aucun conteneur.
  Le banc est un binaire de `crates/crible-store/benches/`, compilé par
  `cargo bench` (profil `bench`, optimisations actives, héritées de `release`).
- Charge ambiante : la machine n'était **pas au repos**. Moyenne de charge de
  3,06 au début et 7,58 à la fin du premier tour
  (`data/mes15/replay-raw-2026-10-26/conditions-start.txt` et
  `conditions-end.txt`), 6,93 à la fin du second. Les
  processus visibles sont un éditeur (VS Code, jusqu'à 74 % d'un cœur avant la
  campagne), l'agent de mesure lui-même (`opencode`, 13 %), `claude`, `Arc`,
  `Ollama`, et par moments `mediaanalysisd` et `mds_stores`. Leur part n'a pas
  été isolée ; aucune autre charge lourde n'a été lancée par la campagne. Les
  p95 et p99 restent à moins de 35 % des p50 (le point `100 000` / 10 est le plus
  dispersé : 65,8 contre 48,7 ms) et le second tour reproduit les p50 à 3,3 % au
  plus (§ 11.4) : l'effet de cette charge sur les écarts comparés ici est faible,
  sans être mesuré.
- Jeu de données : les **sept index de base de la campagne du 2026-10-21**, dans
  `data/mes15/` (gitignoré), inchangés depuis le rejeu du 2026-10-25. Sommes de
  contrôle revérifiées à la main après la campagne sur quatre d'entre eux
  (`shasum -a 256`, préfixe de 16 caractères) : `2679d0c3619b6fd3`
  (`i-1000000-s10`), `04802183f3ce4087` (`i-8580000-s10`), `fb9836d4b41a1040`
  (`i-8580000-s1000`), `a56f01fd3fa63187` (`i-10000-s10`), identiques aux
  préfixes du § 1. Les trois autres n'ont pas été re-hachés.
- Configuration mesurée : identique au § 1 — commit type d'un segment neuf de
  2 000 passages (768 dimensions, fp16), 1 000 documents, 50 pierres tombales,
  10 répétitions par point, `N` et `S` fixes à chaque répétition, cache chaud
  (préchauffage par lecture complète de `records.sqlite` avant chaque
  répétition).

### 11.2 Protocole

```sh
# une fois, hors mesure : compiler le banc (profil `bench`)
cargo build --release
cargo bench -p crible-store --bench commit_cost --no-run

# premier tour : les sept points, 10 répétitions, sortie datée séparée
MES15_RAW="$PWD/data/mes15/replay-raw-2026-10-26" sh tools/bench/mes15/replay.sh

# second tour de répétabilité sur trois points
for label in i-1000000-s10 i-8580000-s10 i-8580000-s1000; do
    cargo bench -p crible-store --bench commit_cost -- measure \
        --index "$PWD/data/mes15/$label" --work "$PWD/data/mes15/replay-work" \
        --repeats 10 --commit "$(git rev-parse --short HEAD)" \
        --machine "Apple M3 / 24 GB / macOS 15.7.7" --date 2026-10-26 \
        --out "$PWD/data/mes15/replay-raw-2026-10-26/round2-$label.json"
done

# fusion des dix points
python3 tools/bench/mes15/merge.py \
    docs/mesures/MES-15-cout-commit-generation.2026-10-26.json \
    data/mes15/replay-raw-2026-10-26/i-*.json \
    data/mes15/replay-raw-2026-10-26/round2-*.json
```

`replay.sh` est réutilisé tel quel à deux ajouts près, tous deux dans le script et
aucun dans le banc : `MES15_RAW` déplace le dossier de sortie, pour que le rejeu
n'écrase ni les JSON de point du 2026-10-25 ni les `after-commit-scans*.json`
cités au § 10.5 ; `MES15_DAY` nomme la date de campagne, parce que le défaut
`date -u` désigne encore le 2026-10-25 à 00 h 35 heure locale. Le protocole du § 2
s'applique sans changement : dix répétitions, clone APFS de l'index de base par
répétition (`clone_kind` vaut `clonefile` pour les 100 répétitions), préchauffage,
cache chaud, p50/p95/p99 par rang le plus proche.

### 11.3 Résultats

Commit complet, 10 répétitions par point, cache chaud, en millisecondes.
« Campagne » est le p50 du 2026-10-21 (§ 3) ; « rejeu 25 » celui du 2026-10-25
(§ 10.3). Les deux colonnes d'écart valent `rejeu 26 − campagne`.

| N | S | p50 rejeu 26 | p95 | p99 | p50 2026-10-21 | Écart | Écart relatif | p50 2026-10-25 | Écart | Écart relatif |
|---|---|---|---|---|---|---|---|---|---|---|
| 10 000 | 10 | **45,5** | 46,8 | 46,8 | 106,4 | −60,9 | −57,2 % | 41,1 | +4,4 | +10,7 % |
| 100 000 | 10 | **48,7** | 65,8 | 65,8 | 110,1 | −61,4 | −55,8 % | 71,9 | −23,2 | −32,2 % |
| 1 000 000 | 10 | **62,0** | 64,8 | 64,8 | 160,0 | −98,0 | −61,3 % | 375,8 | −313,8 | −83,5 % |
| 1 000 000 | 10 (vecteurs vides) | **61,3** | 63,2 | 63,2 | 154,3 | −93,0 | −60,3 % | 375,3 | −314,0 | −83,7 % |
| 1 000 000 | 1 000 | **87,0** | 92,9 | 92,9 | 195,4 | −108,4 | −55,5 % | 401,4 | −314,4 | −78,3 % |
| 8 580 000 | 10 | **89,8** | 96,8 | 96,8 | 598,3 | −508,5 | **−85,0 %** | 3 085,1 | −2 995,3 | **−97,1 %** |
| 8 580 000 | 1 000 | **111,1** | 117,3 | 117,3 | 641,3 | −530,2 | −82,7 % | 3 108,1 | −2 997,0 | −96,4 % |

Sur 10 échantillons, p95 et p99 valent le maximum : ces colonnes mesurent la queue
de la campagne, pas une queue de production (convention du § 2). Le contrôle
« vecteurs vides » reste à 1,1 % de son jumeau, comme les deux campagnes
précédentes : le commit ne lit toujours pas les octets de vecteurs.

Décomposition, p50 en millisecondes sur la même répétition. « Reste » est
`commit − corps de transaction − COMMIT SQLite − métadonnées de segments −
manifeste` ; il porte la dérivation de l'instantané, la publication et l'écart entre
la transaction répliquée et la transaction réelle (constat du § 9).

| N | S | Commit | Transaction (corps) | `COMMIT` SQLite | Métadonnées de segments | Manifeste | Reste |
|---|---|---|---|---|---|---|---|
| 10 000 | 10 | 45,5 | 12,7 | 1,43 | 0,37 | 7,9 | 23,1 |
| 100 000 | 10 | 48,7 | 16,2 | 1,45 | 0,37 | 8,0 | 22,7 |
| 1 000 000 | 10 | 62,0 | 16,2 | 1,41 | 0,37 | 8,0 | 36,0 |
| 1 000 000 | 10 (vecteurs vides) | 61,3 | 16,2 | 1,44 | 0,36 | 8,0 | 35,3 |
| 1 000 000 | 1 000 | 87,0 | 16,5 | 1,57 | 14,70 | 9,0 | 45,2 |
| 8 580 000 | 10 | 89,8 | 16,8 | 1,48 | 0,32 | 8,0 | 63,2 |
| 8 580 000 | 1 000 | 111,1 | 17,4 | 1,49 | 13,83 | 9,7 | 68,6 |

Trois lectures immédiates :

1. **Le terme qui dominait le commit a disparu.** Le « reste » valait 350 ms à
   1 M et 3 058,7 ms à 8,58 M au § 10.3, pour l'essentiel les deux balayages
   complets de `passages` mesurés à 2,92 s au § 10.5. Il vaut 36,0 et 63,2 ms, et
   le commit entier passe de 3 085,1 à 89,8 ms à 8,58 M / 10.
2. **La dérivation reste légèrement linéaire en `N`, mais d'un autre ordre.** Le
   « reste » passe de 23,1 ms à 10 k à 63,2 ms à 8,58 M, soit ≈ 4,7 ns par passage
   ajouté, à comparer aux 60 à 65 ns par passage du § 4. La part de ce résidu qui
   vient de la copie sur écriture des structures et celle qui vient de l'écart
   entre la transaction répliquée et la transaction réelle ne sont pas isolées.
3. **La réplique `live_iids` n'est toujours pas appelée** : elle coûte encore
   495,9 et 508,5 ms à 8,58 M (champs `live_iids_ns` du JSON), et le commit ne la
   paie pas. C'est le contrôle de ce que la dérivation a retiré au § 10.3.

### 11.4 Répétabilité

Second tour intégral sur trois points, même banc, même jour, mêmes index,
10 répétitions :

| N | S | Premier tour (p50) | Second tour (p50) | Écart |
|---|---|---|---|---|
| 1 000 000 | 10 | 62,0 ms | 59,9 ms | −3,3 % |
| 8 580 000 | 10 | 89,8 ms | 88,2 ms | −1,7 % |
| 8 580 000 | 1 000 | 111,1 ms | 110,7 ms | −0,4 % |

Les deux tours se suivent à 3,3 % près au plus, malgré une charge ambiante plus
forte au second (moyenne de charge 6,93 à la fin). Le second tour est dans le JSON
brut avec `round: 2`.

### 11.5 Conclusion du rejeu

Ce que la mesure permet d'affirmer, aux points `N` et `S` de la campagne du
2026-10-21, pour le même commit type :

1. **Le commit à 8,58 M passages vivants repasse sous les 598,3 ms du
   2026-10-21** : 89,8 ms en p50 à `S` = 10 (−85,0 %) et 111,1 ms à `S` = 1 000
   (−82,7 %). Le commit est aussi plus court de 97,1 % et 96,4 % que celui du rejeu
   du 2026-10-25 (3 085,1 et 3 108,1 ms). **La décision 2 de l'ADR-0066 — les deux
   index SQLite `passages(created_generation)` et `documents(updated_generation)`
   — ne devient donc pas le recours à instruire** : elle n'est pas appliquée, et
   rien dans cette campagne ne demande de l'instruire.
2. **Le coût du commit ne suit plus la taille de l'index au premier ordre** : à
   8,58 M, 89,8 ms pour un commit de 2 000 passages, dont 16,8 ms de corps de
   transaction, 8,0 ms de manifeste et 1,5 ms de `COMMIT` SQLite. Le solde de
   63,2 ms est le seul terme qui croît encore avec `N`, à ≈ 4,7 ns par passage.
3. **Les petits points ne sont pas dégradés** : 45,5 ms à 10 k (+10,7 % contre le
   rejeu du 2026-10-25, qui avait déjà gagné 61,4 % sur la campagne du
   2026-10-21) et 48,7 ms à 100 k (−32,2 %). L'écart de +4,4 ms à 10 k est du même
   ordre que la dispersion des deux tours de cette campagne.
4. **Le contrôle « vecteurs vides » tient** : 61,3 ms contre 62,0 ms, écart de
   1,1 %, comme en 2026-10-21 (3,6 %) et en 2026-10-25 (0,1 %).

Ce que la mesure ne permet pas d'affirmer :

- Rien sur un cache froid contrôlé : le protocole reste celui du § 2, cache chaud.
  L'effet du froid sur un commit qui n'a plus de balayage complet n'est pas
  mesuré ; il porte désormais sur les accès bornés et sur la transaction.
- Rien sur l'attache complète : la recette, le segment, le manifeste et le lexical
  restent hors du commit. C'est l'objet de la campagne du 2026-10-26 de MES-08.
- Rien sur la répartition interne du « reste » de 63,2 ms : ni la part de la
  dérivation, ni celle de la transaction réelle contre sa réplique.
- Rien sur les points intermédiaires entre 100 k et 1 M, non mesurés.

**Comparaison au budget.** Il n'existe toujours aucune cible chiffrée pour MES-15
dans `docs/08`. Le seul budget écrit reste `wait_timeout_ms` (60 s, `docs/05`) :
à 89,8 ms, le commit en consomme 0,15 % à 8,58 M, contre 1,0 % avant la dérivation
et 5,2 % après elle. La contrainte qui reste est la sérialisation de l'écrivain
unique, pas le délai du client.

**Recommandation, à valider par le propriétaire** : aucune modification de code
n'est proposée ici. Le chiffre de 89,8 ms laisse la décision 2 de l'ADR-0066 en
recours non instruit, ce qui est l'état que l'ADR prévoit.

### 11.6 Écarts au protocole, et incident de fusion

- **La machine n'était pas au repos** : moyenne de charge 3,06 au début, 7,58 à la
  fin du premier tour, avec un éditeur et l'agent de mesure actifs. La dispersion
  reste bornée (p95 à 35 % au plus du p50, second tour à 3,3 %) ; aucune valeur
  n'est retirée.
- **Pas de cache froid contrôlé**, comme aux deux campagnes précédentes.
- **Incident de fusion, réparé** : au premier appel de `replay.sh`, le 2026-10-26 à
  00 h 43 heure locale, `merge.py` a écrit le fichier fusionné sous le nom
  `MES-15-cout-commit-generation.2026-10-25.json`, parce que `DAY` suit la date
  UTC (2026-10-25 à 23 h 43 UTC). Le JSON du 2026-10-25, versionné, a donc été
  écrasé. Il a été **restauré depuis `HEAD`** (`git show
  HEAD:docs/mesures/MES-15-cout-commit-generation.2026-10-25.json`), et son
  contenu est identique à la version commitée (`git status` ne le signale plus).
  La fusion de ce rejeu a été conservée sous le nom daté du jour local,
  `MES-15-cout-commit-generation.2026-10-26.json`. `replay.sh` porte désormais
  `MES15_DAY` pour que le cas ne se reproduise pas.
- **Date des fichiers de point** : le banc estampille la date qu'on lui passe.
  Les sept points du premier tour portent `2026-10-25` (date UTC au lancement) et
  les trois du second `2026-10-26`. La campagne fusionnée porte `2026-10-26`,
  date locale de la mesure (00 h 35 à 00 h 59, heure de Paris). Les fichiers
  bruts n'ont pas été réécrits pour cela.
- **`replay.sh` modifié** (deux variables d'environnement, `MES15_RAW` et
  `MES15_DAY`) ; le banc n'est pas modifié.

### 11.7 Données brutes

`docs/mesures/MES-15-cout-commit-generation.2026-10-26.json` : les dix points
(sept du premier tour, trois du second), 10 répétitions chacun, toutes les valeurs
brutes par répétition pour le commit et chaque réplique, le maximum de RSS du
processus par point. Les fichiers de point, les `.time` et les relevés de charge
sont dans `data/mes15/replay-raw-2026-10-26/` (gitignoré) ; les relevés du
2026-10-25 restent dans `data/mes15/replay-raw/`, intacts.
