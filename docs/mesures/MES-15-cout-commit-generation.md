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
