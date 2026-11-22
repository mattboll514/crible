# MES-18 — Coût de la route de facettes et du compte de documents

## Campagne du 2026-11-12

- **Tâche** : `T12-FACETTES-ET-COMPTES` (Crible 1.1). Mesure **MES-18** de `docs/08-invariants-et-tests.md`, protocole et critère de repli fixés par ADR-0154, décision 11.
- **Commit** : `bebf39a0242d27dbf6c83fe6b99fac883e44fe16` (`bebf39a`), branche `tache/T12-FACETTES-ET-COMPTES`, **plus le travail non commité de la tâche dans l'arbre** (`crates/crible-server/src/facets.rs` et `crates/crible-store/src/document_ordinals.rs` sont nouveaux et non suivis). Le binaire mesuré est construit depuis cet arbre, `cargo build --release`, empreinte SHA-256 `eb3a4a585728db8af68ff2476db10660b30f48091b189e0fe2a4b38ef452afbf`. Aucun code de production n'a été modifié pendant la session.
- **Machine** : MacBook Air `Mac15,12` (Apple M3), 8 cœurs, **25 769 803 776 octets de RAM (24 Gio)**, macOS 15.7.7 (24G720), **natif** (pas de Docker, pas de conteneur). Charge au départ, relevée avant l'import puis pendant la campagne : moyenne `1,62 2,46 3,95` sur 8 cœurs, et les processus tiers les plus actifs sont `opencode` (8 %), la VM Docker Desktop au repos (4,5 %) et un assistant de navigateur (4,5 %). Aucune autre charge lourde n'a été lancée : le serveur de mesure est le seul consommateur de CPU pendant les appels.
- **Jeu de données** : les six lots `replace_all` de l'échelle réelle, `data/legifacile-full/<index>/<index>-full-v*` — **9 600 105 passages, 6 660 239 documents, 0 mort**. Le lot de `titres` est `titres-full-v2`, et non `titres-full-v1` : le dossier ne contient que celui-là. Sommes de contrôle SHA-256 de chaque `manifest.json` (les fichiers Parquet ne sont pas hachés ici) :

| Index | Lot | SHA-256 du manifeste |
|---|---|---|
| `articles` | `articles-full-v1` | `550f164e96bc0a16907e7eecc0e3fabca28e3336f193f9db31784308f0a7c327` |
| `decisions` | `decisions-full-v1` | `d2432f6cca0ec2b40432614cde588de87059a381d8db11919c3023164498309d` |
| `dole` | `dole-full-v1` | `606e8c028431573e738f72971169d46ede6b77ab873a155c9ce0d8357def3a5f` |
| `jorf` | `jorf-full-v1` | `5ac78c1c44959628a161b8f9755a38c0eb42f58f894cb0a59cc8821b04913ce2` |
| `sarde` | `sarde-full-v1` | `f4eb42a07520907e1b5323eba501eb2390a90975e7571c8b3a34a1120edccac6` |
| `titres` | `titres-full-v2` | `57b4586efa161750acd9fe361f3ede4a7663bc3224aa3c65a12e5cbd15dab539` |

- **Import** : six index créés dans un dossier **neuf** `data/mes18/data` par `crible index create`, puis six attaches par la CLI, une commande par index, dans l'ordre des tailles croissantes. Durées relevées par `/usr/bin/time -p` : `dole` **129,07 s** (le premier appel paie le chargement à froid du modèle dans le cache de fichiers), `sarde` **55,44 s**, `titres` **102,38 s**, `jorf` **276,71 s**, `decisions` **698,25 s** (11 min 38 s), `articles` **442,59 s** (7 min 23 s), soit 28,4 min d'attache au total. Chaque attache rend le compte attendu du manifeste et une recette tenue (cosinus minimal de 0,999970 à 0,999994 sur 200 passages ré-encodés).
- **Configuration** : les six fichiers de `docs/legifacile/configs/`, copiés tels quels par `crible index create` (`reference_precision` `fp16`, `lexical.enabled` `true`, `lexical.positions` `false`, `ingest.acceptance_sample` 200, `acceptance_min_cosine` 0,999). L'instance est décrite par `data/mes18/crible.toml` : `bind = "127.0.0.1:18018"`, `search_threads = 0`, `max_concurrent_searches = 0` (les valeurs par défaut), `default_timeout_ms = 2000`, `encoder.query_precision = "fp32"`, `debug_journal.enabled = false`, une clé de rôle `admin` propre à la campagne. Le modèle est `data/models/mdenseon`, chargé par ONNX Runtime 1.31.0 désigné par `ORT_DYLIB_PATH` (`tools/.venv/lib/python3.13/site-packages/onnxruntime/capi/libonnxruntime.1.31.0.dylib`).
- **Espace** : `df -h .` relevait **79 Gio libres** avant l'import et **46 Gio** à la fin ; la consigne d'arrêt sous 20 Gio n'a jamais été approchée. `data/mes18/` pèse **25 Go**, dont 24,7 Go d'index et 1,2 Mo de sorties brutes.

### Protocole

Le protocole applique ADR-0154, décision 11, et la ligne MES-18 de `docs/08`. Le serveur est **démarré, interrogé puis arrêté dans une seule commande** par le harnais `data/mes18/scripts/measure.py` : aucun processus ne survit à la commande, et aucune commande n'a tourné en arrière-plan. Le harnais attend `/ready`, puis **un seul client** envoie les appels **séquentiellement**, chacun chronométré à l'horloge murale autour de l'appel HTTP. Chaque cas fait **10 appels d'échauffement écartés**, puis **100 appels mesurés**. Les percentiles sont au **rang le plus proche** (ADR-0077). Trois démarrages du serveur ont eu lieu : une exploration à 3 appels par cas (écartée des résultats), la campagne principale, puis un complément de deux cas. Les deux démarrages dont la durée a été relevée ont demandé 62,6 s (exploration) et 67,7 s (campagne principale) avant `/ready`, ouverture des six index et chargement du modèle compris.

Les cinq cas du point 1 de la consigne, tous avec `count.documents`, `limit` par défaut 20, sur les filtres de corpus de `docs/legifacile/index-legifacile.md` :

| Cas | Index | Filtre | Champs |
|---|---|---|---|
| A1 | `decisions` | aucun (plus grand pool) | `juridiction` (`cle_domaine`), `section` (`fonds` de la vue) |
| A2 | `articles` | `textes_vigueur` : `etat in [VIGUEUR, VIGUEUR_DIFF, VIGUEUR_ETEN, VIGUEUR_NON_ETEN]` | `code` (`cle_domaine`), `fonds` |
| A3 | `articles` | `textes_hors_vigueur` : `$or` de `etat nin` ces quatre valeurs et de `etat exists: false` | `code`, `fonds` |
| A4 | `decisions` | `juri_sommaires` : `partie exists: false` | `juridiction`, `section` |
| A5 | `decisions` | `juri_texte_integral` : `partie exists: true` | `juridiction`, `section` |

Le point 4 mesure le **coût par champ** par des appels à **un seul champ**, sans filtre et sous le filtre de corpus de l'index, `count.documents` compris. Deux cas supplémentaires (C1, C2) bornent le champ d'`articles` qui porte le plus de valeurs distinctes.

**Cache.** Les appels mesurés tournent sur un **cache chaud** : les données viennent d'être écrites par l'import et le noyau garde leurs pages. Aucune mesure à cache froid n'a été obtenue, et c'est déclaré : macOS n'expose pas d'équivalent à `drop_caches`, et `purge` demande les droits d'administrateur, qui n'ont pas été pris dans cette session.

### Résultats

#### 1. p95 des cas de corpus, avec `count.documents`

Tous les appels rendent `200` ; aucun `504` n'a été observé. Latence murale en ms, `n = 100` par cas après échauffement.

| Cas | Index | Filtre | `count.passages` | `count.documents` | p50 | **p95** | p99 |
|---|---|---|---:|---:|---:|---:|---:|
| A1 | `decisions` | aucun | 3 589 776 | 1 196 623 | 45,97 | **46,43** | 47,14 |
| A2 | `articles` | `textes_vigueur` | 986 044 | 885 200 | 119,72 | 124,85 | 133,74 |
| A3 | `articles` | `textes_hors_vigueur` | 1 490 671 | 1 249 823 | 124,63 | 126,09 | 127,12 |
| A4 | `decisions` | `juri_sommaires` | 590 196 | 578 250 | 8,86 | 8,95 | 9,04 |
| A5 | `decisions` | `juri_texte_integral` | 2 999 580 | 618 373 | 37,04 | 37,33 | 37,72 |

**`count.passages` égale `live_passages` sans filtre** : A1 rend 3 589 776, et `GET /v1/indexes/decisions` rend `live_passages` 3 589 776. Les cas B et C sans filtre confirment le même point sur `articles` (2 476 715) et sur `decisions` (3 589 776).

**Les deux corpus partagent exactement les documents de l'index** : A2 + A3 donnent 885 200 + 1 249 823 = **2 135 023**, soit `documents` d'`articles`, et A4 + A5 donnent 578 250 + 618 373 = **1 196 623**, soit `documents` de `decisions`. Le compte sans filtre égale aussi `documents` de l'état pour les deux index. ADR-0154 ne promet pas cette égalité, mais elle se vérifie ici sur un index sans mort.

Le cas A1 est le plus grand pool de la campagne (3 589 776 passages), et c'est lui que le critère de repli nomme.

#### 2. Mémoire de la table dérivée sur les six index

`documents_bytes` est la mémoire de la table telle que le code la compte, `4 × len(by_iid)` (`DocumentOrdinals::memory`, `document_ordinals.rs`), et `len(by_iid)` est `MAX(iid) + 1` relevé par SQLite sur chaque `records.sqlite` (aucun mort, donc `= count(*)`). `filter_memory_bytes` est le chiffre publié par `GET /v1/indexes/{index}` et `crible inspect`.

| Index | Passages (slots) | `documents_bytes` | `filter_memory_bytes` (filtres + table) | Part de la table |
|---|---:|---:|---:|---:|
| `articles` | 2 476 715 | 9 906 860 | 134 771 561 | 7,35 % |
| `decisions` | 3 589 776 | 14 359 104 | 98 473 439 | 14,58 % |
| `jorf` | 1 833 500 | 7 334 000 | 59 174 085 | 12,39 % |
| `titres` | 1 477 883 | 5 911 532 | 82 891 975 | 7,13 % |
| `sarde` | 214 578 | 858 312 | 2 832 801 | 30,30 % |
| `dole` | 7 653 | 30 612 | 170 045 | 18,00 % |
| **Total** | **9 600 105** | **38 400 420** | **378 313 906** | **10,15 %** |

La table ajoute donc **38 400 420 octets, soit 36,62 Mio**, sur les six index. Les filtres eux-mêmes pèsent 378 313 906 − 38 400 420 = **339 913 486 octets (324,17 Mio)**.

**Confirmation indépendante du calcul.** `crible inspect` publie `memory.total()` (qui inclut `documents`) et ses quatre autres composantes, `live`, `keywords`, `booleans` et `columns`. La différence `total − (live + keywords + booleans + columns)` redonne, index par index, exactement la valeur `4 × slots` du tableau : 30 612 pour `dole`, 858 312 pour `sarde`, 5 911 532 pour `titres`, 7 334 000 pour `jorf`, 14 359 104 pour `decisions` et 9 906 860 pour `articles`. Le harnais `ordinals_time` rend aussi, par construction, la taille de sa table (`bytes=30612`, `858312`, `5911532`, `7334000`, `14359104`, `9906860`), identique aux six valeurs.

**Ce que la mesure ne peut pas faire ici.** ADR-0151 fait foi par `RssAnon`, mais macOS n'a pas de `/proc` ni de `RssAnon` : cette voie n'existe qu'en conteneur, et la campagne est native comme la consigne l'exige. La consigne autorise alors le calcul `4 octets × taille du tableau`, et c'est lui qui fait foi, doublé de la décomposition de `crible inspect`. Le RSS du processus après l'ouverture des six index vaut 1 207 872 Kio (1 179,6 MiB) et 1 918 848 Kio (1 873,9 MiB) après les appels ; il contient le modèle, les codes binaires et les bitmaps, et ne permet donc **pas** d'attribuer 36,62 Mio à la table par différence. Aucun chiffre de RSS n'est présenté comme une attribution.

#### 3. Durée ajoutée à l'ouverture

`DocumentOrdinals::from_records` est la fonction que `build_snapshot` appelle à l'ouverture d'une génération. Le harnais `data/mes18/scripts/ordinals_time.rs` l'appelle directement sur chaque `records.sqlite` en lecture seule, après une première requête qui lit les mêmes lignes sans rien construire : l'écart entre les deux est la construction, déclaré comme un écart et non comme une mesure séparée. Trois répétitions par index ; la première paie la lecture à froid du fichier, les deux suivantes sont chaudes.

| Index | SQL seule, médiane chaude (ms) | `from_records`, médiane chaude (ms) | Construction (écart, ms) |
|---|---:|---:|---:|
| `dole` | 1,644 | 1,695 | 0,05 |
| `sarde` | 25,302 | 26,439 | 1,14 |
| `titres` | 177,340 | 184,904 | 7,56 |
| `jorf` | 218,520 | 226,348 | 7,83 |
| `decisions` | 426,386 | 452,629 | 26,24 |
| `articles` | 294,654 | 313,731 | 19,08 |
| **Total** | **1 143,846** | **1 205,746** | **61,90** |

La construction de la table ajoute donc **environ 1,2 s à l'ouverture des six index**, dont 1,14 s de lecture SQL ordonnée par `doc_id` et environ 62 ms de construction en mémoire. Une précision de lecture : la première répétition lit le fichier à froid et son passage SQL seule est beaucoup plus lent (2,5 ms pour `dole`, 122,9 ms pour `sarde`, 870,0 ms pour `titres`, 860,4 ms pour `jorf`, 1 586,7 ms pour `decisions`, 442,6 ms pour `articles`) ; elle réchauffe le cache pour l'appel `from_records` qui suit, ce qui rend son écart négatif et inutilisable. Ces valeurs à froid sont rapportées ici, pas retenues dans le tableau.

**Le journal de démarrage ne donne pas cette durée.** Il écrit une ligne `index opened for writing` par index et une ligne `models and indexes loaded`, sans durée par étape : la mesure par le harnais est donc la seule disponible. Pour situer le poids, l'ouverture complète d'un index par `crible inspect` dure 0,04 s (`dole`), 0,56 s (`sarde`), 5,19 s (`titres`), 8,78 s (`jorf`), 20,57 s (`decisions`) et 18,96 s (`articles`), codes binaires et filtres compris : la table est de l'ordre de **2 % de l'ouverture** de `decisions`, et moins ailleurs.

#### 4. Coût par champ

Valeurs distinctes relevées par SQLite sur les documents (et sur les passages pour `partie`, champ de passage), nulls exclus. Un appel par champ, `limit` 20, `count.documents` compris, `n = 100`.

| Cas | Index | Champ | Valeurs distinctes | Filtre | `count.passages` | p50 | p95 | p99 |
|---|---|---|---:|---|---:|---:|---:|---:|
| B1 | `articles` | `etat` | 15 | aucun | 2 476 715 | 27,79 | 28,08 | 28,30 |
| B2 | `articles` | `fonds` | 2 | aucun | 2 476 715 | 27,60 | 27,74 | 27,86 |
| B3 | `articles` | `num` | 180 797 | aucun | 2 476 715 | 102,85 | 107,84 | 111,06 |
| B4 | `articles` | `code` | 192 876 | aucun | 2 476 715 | 140,70 | 146,76 | 160,31 |
| C1 | `articles` | `texte_cid` | 195 535 | aucun | 2 476 715 | 111,13 | 117,98 | 137,02 |
| B5 | `articles` | `num` | 180 797 | `textes_vigueur` | 986 044 | 86,33 | 87,96 | 90,62 |
| B6 | `articles` | `code` | 192 876 | `textes_vigueur` | 986 044 | 119,08 | 120,40 | 124,24 |
| C2 | `articles` | `texte_cid` | 195 535 | `textes_vigueur` | 986 044 | 93,29 | 95,34 | 96,22 |
| B7 | `decisions` | `section` | 6 | aucun | 3 589 776 | 45,21 | 45,95 | 46,74 |
| B8 | `decisions` | `juridiction` | 399 | aucun | 3 589 776 | 45,79 | 46,25 | 46,41 |
| B9 | `decisions` | `numero` | 743 117 | aucun | 3 589 776 | 301,95 | 306,19 | 306,60 |
| B10 | `decisions` | `numero` | 743 117 | `juri_texte_integral` | 2 999 580 | 255,11 | 256,95 | 257,27 |
| B11 | `decisions` | `juridiction` | 399 | `juri_texte_integral` | 2 999 580 | 36,85 | 37,26 | 37,74 |

Les valeurs de faible cardinalité coûtent le prix du pool et du compte de documents : environ 27,6 ms sur `articles` et 45,2 ms sur `decisions`, avec un écart de 0,2 à 0,6 ms entre `fonds` (2 valeurs), `etat` (15) et `section` (6). Le coût du champ devient visible à partir de quelques centaines de valeurs : `juridiction` (399) ajoute moins de 1 ms, `num` (180 797) ajoute environ 75 ms, `code` (192 876) environ 113 ms et `numero` (743 117) environ 257 ms.

**Deux constats qui portent sur la lecture des chiffres, pas sur un défaut.**

1. **`cle_domaine` d'`articles` n'est pas de faible cardinalité.** `code` porte **192 876** valeurs distinctes, parce que les conventions collectives `KALI` y mettent un titre long et souvent unique (« ACCORD D'INTERPRETATION Accord du 2 avril 1982 », etc.), là où `LEGI` met un nom de code. ADR-0154 range `cle_domaine` parmi les vocabulaires courts ; c'est vrai pour `juridiction` (399), mais faux pour `code` sur le jeu complet (`theme` et `type` n'ont pas été mesurés ici). Le tableau distingue donc la cardinalité réelle, et C1 montre que `texte_cid` (195 535) n'est pas plus cher que `code` malgré un peu plus de valeurs.
2. **`limit` ne réduit pas le coût.** La route compte toutes les valeurs du champ, puis trie et coupe : `numero` rend 20 valeurs en 302 ms, et non un coût proportionnel à `limit`. C'est conforme à ADR-0154, décision 8, qui borne ce coût par le délai plutôt que par `limit`.

**L'échéance par défaut n'est atteinte par aucun appel.** Le pire temps mesuré est le p99 de B9, **306,60 ms**, pour un `default_timeout_ms` de 2 000 ms : aucun `504` n'est apparu sur les 1 800 appels mesurés. Le champ le plus chargé de la campagne, `numero` (743 117 valeurs), reste donc six fois et demie sous l'échéance.

#### Contrôles de cohérence

Sur les 18 cas et leurs 1 800 appels mesurés :

- **aucun statut autre que `200`** ; aucune erreur, aucun `503`, aucun `504` ;
- pour les neuf cas sans filtre, `count.passages` égale `live_passages` de l'état publié ;
- `Σ count + other` ne dépasse jamais `count.passages` sur aucun champ ; `other` vaut 0 quand toutes les valeurs tiennent dans `limit` (`fonds`, `etat`, `section`), et il est strictement positif pour les champs tronqués (`code`, `num`, `numero`, `juridiction`, `texte_cid`) ;
- `count.documents` sans filtre égale `documents` de l'état, et les comptes des deux corpus d'un index s'additionnent exactement à ce nombre ;
- les `count` et `other` rapportés sont ceux du dernier appel de chaque cas ; le harnais n'a pas comparé les 100 corps octet pour octet, donc la reproductibilité de l'ordre et des valeurs n'est pas démontrée ici, et ce sont les tests de la tâche qui la couvrent.

Aucun comportement anormal de la route n'a été observé.

### Écarts au protocole et limites

1. **Cache chaud seulement.** Les pages des segments et de `records.sqlite` sont chaudes, entretenues par l'import et par les 18 cas successifs. macOS ne permet pas de les jeter sans `purge` et les droits d'administrateur n'ont pas été pris : aucune valeur à froid n'est publiée pour la route. Les chiffres décrivent donc la latence d'un serveur qui a déjà servi, pas la première requête après un démarrage.
2. **Un seul client, aucune concurrence.** Le protocole de la consigne demande un seul client à la fois, et c'est ce qui a été fait. Le sémaphore de `max_concurrent_searches` et le comportement sous saturation (`503 overloaded`), décrits par ADR-0154, décision 8, ne sont donc pas exercés ici : ils le sont par les tests de `crates/crible-server/tests/facets_semaphore_and_analytics.rs`.
3. **Une seule génération, aucun mort.** Les six index sont à la génération 1, avec 0 passage mort et 0 compaction. La table est donc au plus dense : `len(by_iid)` égale exactement le nombre de passages. Des iid morts avant une compaction ajouteraient leurs 4 octets, comme ADR-0154, décision 5, le prévoit, mais aucune mesure ne chiffre ce cas ici.
4. **La mémoire est une estimation du code, pas une lecture de l'allocateur.** `4 × len` est la formule que `DocumentOrdinals::memory` publie et que `filter_memory_bytes` additionne ; elle ignore la capacité excédentaire du `Vec` et la surcharge de l'`Arc`. Sur un vecteur de plusieurs millions d'éléments, la capacité vaut la longueur à l'allocation près, et l'écart reste très inférieur au seuil de 64 Mio, mais il n'est pas mesuré.
5. **Pas de `RssAnon` natif.** La voie d'ADR-0151 est celle du conteneur. En natif, macOS ne l'expose pas ; la confirmation passe par la décomposition de `crible inspect` et par le harnais, qui redonnent les mêmes octets.
6. **Le coût de mise à jour de la table à chaque commit n'est pas mesuré**, et ADR-0154, décision 5, le déclare hors du périmètre de MES-18 : les six index sont issus d'une attache unique, donc `after_commit` n'a tourné qu'une fois par index, et aucune génération de métadonnées seule n'a été jouée.
7. **Une machine de développement, native.** Les chiffres valent pour ce MacBook Air M3, sans conteneur. ADR-0129 range la latence hors du Mac mini en conteneur comme indicative ; rien ici ne dit ce que la même route rend dans la VM Docker de 11,67 Gio, où MES-05 mesure un coût d'encodage 2,2 à 2,6 fois supérieur. Cette route-ci ne fait aucun encodage, mais elle parcourt des bitmaps et un pool de 3,6 M passages, et le facteur du conteneur n'est pas connu pour ce profil.
8. **Les lots ne sont pas hachés au-delà de leurs manifestes.** Les SHA-256 du tableau identifient la version des manifestes, pas les 18 Go de Parquet. C'est la convention reprise de MES-11, et elle suffit à nommer le jeu importé.

### Conclusion

**Verdict sur le critère de repli d'ADR-0154, décision 11 : `CRITERE TENU`.**

| Grandeur du critère | Seuil | Mesuré | Marge |
|---|---:|---:|---:|
| p95 du cas A1 (plus grand pool, `count.documents`) | ≤ 250 ms | **46,43 ms** | 5,4 fois sous le seuil |
| Mémoire anonyme de la table sur les six index | ≤ 64 Mio | **36,62 Mio** (38 400 420 o) | 27,38 Mio sous le seuil |

Les deux conditions du repli sont donc remplies dans le bon sens : `count.documents` **reste exposé** en 1.1, et la table dérivée **reste en place**. Aucune modification de code n'est demandée par cette mesure.

**Ce que les chiffres disent du coût de la route.** Le compte de documents et le pool dominent les cas ordinaires : 46 ms sur le plus grand pool de `decisions`, 9 à 126 ms sur les quatre filtres de corpus, avec un p95 toujours sous 127 ms. Les cinq cas de corpus passent donc le seuil avec une marge d'un facteur 2 à 28. Le coût des champs est celui du nombre de valeurs distinctes : négligeable sous quelques centaines de valeurs, de l'ordre de 75 à 160 ms pour 180 000 à 196 000 valeurs, et de 255 à 306 ms pour les 743 117 valeurs de `numero`. Aucun de ces cas n'approche l'échéance par défaut de 2 000 ms.

**Points à porter au propriétaire, sans décision prise ici.**

1. **Publier la cardinalité réelle dans la lecture des champs.** ADR-0154 range `cle_domaine` parmi les vocabulaires courts ; la mesure montre que `code` sur `articles` porte 192 876 valeurs, du fait des conventions collectives `KALI`. Si Legifacile demande `cle_domaine` sur `articles` comme facette, il paie le coût d'un champ à forte cardinalité, pas celui d'un vocabulaire court. Une note dans `docs/05` ou un ajout à MES-18 suffirait ; le choix appartient au propriétaire.
2. **Surveiller `numero` et les identifiants si la route est appelée en concurrence.** Ces champs coûtent 250 à 310 ms par appel sur un seul client. Sous plusieurs appels simultanés, ils prennent plusieurs jetons du sémaphore des recherches et peuvent faire grimper la latence des recherches elles-mêmes. La route est correcte telle quelle (le délai la borne), mais un usage de `valeurs_frequentes` sur un identifiant de décision est le cas à observer en production.
3. **Reporter la mesure dans un conteneur si la production y tourne.** La campagne est native, comme la consigne le demande ; la comparaison au budget de la VM de production reste à faire si le propriétaire veut un chiffre transposable.

**Ce que la mesure ne permet pas d'affirmer.**

- Rien sur la latence à froid, ni sur un index chargé par plusieurs générations avec des passages morts.
- Rien sur la concurrence, ni sur le `503 overloaded`, ni sur le `504` de la route : les tests les couvrent, pas cette campagne.
- Rien sur le coût du commit qui met la table à jour, ni sur une compaction à l'échelle réelle.
- Rien sur le temps de réponse d'un client distant : la mesure est locale, sur la boucle locale.

### Données conservées

Tout est sous `data/mes18/`, **25 Go** au total, gitignoré et conservé pour rejouer la mesure :

- `data/mes18/data/indexes/<index>/` — les six index importés, 24,7 Go (`decisions` 9,6 Go, `articles` 6,9 Go, `jorf` 4,6 Go, `titres` 3,1 Go, `sarde` 511 Mo, `dole` 28 Mo) ;
- `data/mes18/crible.toml` — la configuration de l'instance de mesure (clé locale de rôle `admin`, aucune valeur de production) ;
- `data/mes18/scripts/measure.py` — le harnais de la route : démarrage, `/ready`, échauffement, 100 appels par cas, arrêt propre, JSON brut ;
- `data/mes18/scripts/ordinals_time.rs` et son binaire compilé — le harnais de la durée et des octets de la table, lié à `libcrible_store.rlib` de la construction `--release` ;
- `data/mes18/raw/facets.json` — les 16 cas principaux, percentiles, statuts, comptes et les 100 durées de chaque cas ;
- `data/mes18/raw/facets-extra.json` — les deux cas du complément (`texte_cid`) ;
- `data/mes18/raw/checks.json` — les contrôles de cohérence par cas (`Σ count + other`, `live_passages`, partitions de documents) ;
- `data/mes18/raw/ordinals-build.txt` — les trois répétitions par index de la durée et des octets de la table ;
- `data/mes18/raw/explore.json` — l'exploration à 3 appels par cas, écartée des chiffres et gardée comme trace ;
- `data/mes18/raw/serve.log`, `serve-extra.log`, `explore-serve.log` — les journaux JSON du serveur, pour chaque démarrage.

Les scripts restent sous `data/mes18/scripts/` et non sous `tools/bench/mes18/` : la consigne laissait le choix, et le dossier de données garde le harnais à côté de ses sorties, comme MES-17. Les déplacer sous `tools/bench/mes18/` pour les versionner est une proposition, pas un fait accompli.

Les chiffres bruts sont aussi dans `docs/mesures/MES-18-facettes.json`, versionné avec ce rapport.
