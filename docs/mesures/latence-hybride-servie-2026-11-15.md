# Latence hybride servie — où passe le temps d'une recherche servie à Legifacile

## Campagne du 2026-11-15

> Campagne de mesure de la tâche `T17-TEMPS-PAR-ETAPE-SANS-DEBUG` (Crible 1.1), en réponse au point 1 du ticket 17 de Legifacile. Ce n'est **pas** une nouvelle mesure MES-xx de `docs/08` : c'est un rapport de campagne, comme le prévoit le point 1 du critère d'acceptation du ticket (« une campagne ponctuelle dans `docs/mesures/` suffit »). Il ne conclut rien sur la cible de Legifacile (p50 ≤ 150 ms, p95 ≤ 800 ms, contraignante sur le Mac mini) et ne dit rien du Mac mini.

- **Tâche** : `T17-TEMPS-PAR-ETAPE-SANS-DEBUG`.
- **Commit** : `28c072e5b016770f48ccb967bab7d88d8be57faa` (`28c072e`), branche `tache/T17-TEMPS-PAR-ETAPE-SANS-DEBUG`. Binaire mesuré : `cargo build --release`, `target/release/crible`, construit depuis cet arbre (`cargo build --release` ne recompile rien au 2026-11-15). Aucun code de production ni document de conception n'a été modifié **par cette campagne** ; l'arbre de travail porte par ailleurs des modifications d'une autre tâche (application d'ADR-0156), qui ne sont pas de ce rapport.
- **Machine** : MacBook Air `Mac15,12` (Apple M3), 8 cœurs, **25 769 803 776 octets de RAM (24 Gio)**, macOS 15.7.7 (24G720), **natif** — Docker n'était pas lancé, aucun conteneur n'a servi.
  - Charge au départ, avant l'import : `1.42 2.13 2.09`, relevée le 2026-11-15 à 14 h 48.
  - Charge avant la campagne d'appels : `3.46 3.58 3.95`.
  - Charge pendant la passe après redémarrage : `2.35 3.06 3.66`, puis `2.68 3.11 3.67` à la fin.
  - `uptime` a été relevé avant et après **chaque** passe ; les valeurs sont dans `data/t17/raw/phase1.json` et `phase2.json`. Aucune autre charge lourde n'a été lancée : le serveur de mesure est le seul consommateur de CPU pendant les appels, mais la machine n'était pas au repos (charge 2 à 4 sur 8 cœurs, processus tiers compris).
- **Jeu de données** : les six lots `replace_all` de l'échelle réelle, `data/legifacile-full/<index>/<index>-full-v*`, **9 600 105 passages, 6 660 239 documents, 0 mort**, génération 1 sur chaque index. Sommes de contrôle SHA-256 des `manifest.json`, identiques à celles de MES-18 :

| Index | Lot | SHA-256 du manifeste | Passages | Documents |
|---|---|---|---:|---:|
| `articles` | `articles-full-v1` | `550f164e96bc0a16907e7eecc0e3fabca28e3336f193f9db31784308f0a7c327` | 2 476 715 | 2 135 023 |
| `decisions` | `decisions-full-v1` | `d2432f6cca0ec2b40432614cde588de87059a381d8db11919c3023164498309d` | 3 589 776 | 1 196 623 |
| `jorf` | `jorf-full-v1` | `5ac78c1c44959628a161b8f9755a38c0eb42f58f894cb0a59cc8821b04913ce2` | 1 833 500 | 1 636 145 |
| `titres` | `titres-full-v2` | `57b4586efa161750acd9fe361f3ede4a7663bc3224aa3c65a12e5cbd15dab539` | 1 477 883 | 1 477 883 |
| `sarde` | `sarde-full-v1` | `f4eb42a07520907e1b5323eba501eb2390a90975e7571c8b3a34a1120edccac6` | 214 578 | 210 965 |
| `dole` | `dole-full-v1` | `606e8c028431573e738f72971169d46ede6b77ab873a155c9ce0d8357def3a5f` | 7 653 | 3 600 |

- **Import** : dossier **neuf** `data/t17/data`, créé une première fois le 2026-11-12 puis **détruit et refait le 2026-11-15** après l'arrêt de la session précédente, qui avait laissé l'attache de `decisions` inachevée. Six index créés par `crible index create --data data/t17/data --config docs/legifacile/configs/<index>.json`, puis six attaches par la CLI, une commande par index, dans l'ordre des tailles croissantes. Durées relevées par le harnais `data/t17/scripts/attach_one.py` : `sarde` **55,0 s**, `titres` **90,1 s**, `dole` **120,1 s**, `jorf` **250,4 s**, `articles` **405,6 s**, `decisions` **615,6 s**, soit **25 min 37 s** au total. Chaque attache rend le compte du manifeste et une recette tenue (cosinus minimal de 0,999970 à 0,999994 sur 200 passages ré-encodés).
- **Espace** : `df -h .` relevait **59 Gio libres** avant l'import et **37 Gio** à la fin ; la consigne d'arrêt sous 30 Gio n'a pas été approchée. `data/t17/` pèse **25 Go**, dont 24,8 Go d'index.
- **Modèle** : `data/models/mdenseon` — `lightonai/mDenseOn`, révision `a5fdb000f7a21da96c3bddde3a782ef777316df3`, 768 dimensions, pooling `cls`, normalisation, `max_tokens` 8192, prompts `document: ` / `query: `, fp32 `blake3` `7b6eb68099dbeac6826e282dd575aec766dfbe1bedac143bae094b12f82094a2`, tokenizer `blake3` `8a801317cfb1eb744734392dc0aeef36d6d39660a0a884f06a0338e9bb5357db`. ONNX Runtime 1.31.0 désigné par `ORT_DYLIB_PATH="…/onnxruntime/capi/libonnxruntime.1.31.0.dylib"`. **`query_precision` = `fp32`** (`[encoder]` de `data/t17/crible.toml`). L'empreinte de modèle d'ADR-0028 est le hachage canonique de ces ingrédients (identifiant, révision, dimensions, pooling, normalisation, prompts, hash des fichiers fp32 et du tokenizer) ; aucune route ne la publie octet pour octet, donc le rapport nomme les ingrédients plutôt que le digest.
- **Configuration** : `data/t17/crible.toml` — `bind = "127.0.0.1:18019"`, `search_threads = 0` et `max_concurrent_searches = 0` (valeurs par défaut), `default_timeout_ms = 2000` (le corps demande `timeout_ms: 8000`), `query_cache_entries = 1024`, `analytics.retention_days = 90`, `reference_precision = "fp16"`, `lexical.enabled = true`, `lexical.positions = false`, `rescore_candidates = 5000`, `exact_threshold = 5000`, **`debug_journal.enabled = false`**. Trois clés locales (rôles `read`, `write`, `admin`), générées pour la campagne par `crible key generate`, écrites seulement sous `data/t17/keys.txt` : la clé `admin` a servi aux lectures de `/v1/stats` et de `/v1/debug/queries`, que les rôles `read` et `write` n'ont pas le droit de lire.
- **Journal de debug** : **désactivé**, valeur par défaut de `docs/05-api.md` (ADR-0122), et vérifié par `GET /v1/debug/queries`, qui rend `enabled: false` après la campagne. `debug: true` n'est demandé que dans le **corps** des requêtes, pour obtenir `timings_ms` ; aucune entrée du journal n'a donc été écrite, et aucun texte de question n'a touché le disque.

### But

Répondre à la question du ticket 17 : **sur la forme servie à Legifacile, quelle étape porte le temps de Crible, et quelle part du p95 vient de l'encodage froid ?** Legifacile mesure, en conteneur Docker Desktop sur cette même machine chargée, un appel Crible à **p50 301,227 ms** pour un total de 322,276 ms (84,3 %).

Ce que ce rapport ne fait pas : il ne mesure pas la cible de Legifacile, il ne transpose rien au Mac mini, et il ne compare pas le conteneur au natif sur la même forme — le facteur du conteneur n'est pas mesuré ici.

### Forme servie reproduite

Corps envoyé par le harnais, un appel par requête, exactement la forme de `src/legifacile/mcp/search.py` (`search_crible`, lignes 406-433, relu le 2026-11-15 et inchangé depuis le 12) : route `POST /v1/search`, **une seule cible**, `mode: "hybrid"`, `k: 10`, `group_by: "document"`, `return: {"fields": <INDEX_FIELDS de l'index>, "text": true}`, `timeout_ms: 8000`, `debug: true`, et le `filter` de la ligne de référence quand elle en porte un.

Les `return.fields` sont ceux de Legifacile, index par index : `articles` `fonds, etat, annee, code, num, texte_cid, date_debut, date_fin` ; `decisions` `section, juridiction, annee, numero, partie` ; `jorf` `fonds, etat, annee, nature, num, texte_cid` ; `titres` `fonds, nature, etat, annee, num, texte_cid` ; `sarde` `theme` ; `dole` `type, annee`.

Le regroupement par document porte `candidates_per_branch` à `clamp(k × 20, 200, 1000)` = **200** (`docs/03-recherche.md:119-121`), et non la valeur 100 du fichier de configuration de l'index ; en mode hybride, la fusion entre index s'exécute même avec une seule cible, `score_type` valant `rrf` (`docs/03-recherche.md:132`).

### Protocole

Le harnais est `data/t17/scripts/measure.py`. Une seule commande (`python3 data/t17/scripts/measure.py campaign`) **démarre le serveur, attend `/ready`, joue les appels, lit `/v1/stats`, puis arrête le serveur** ; aucun processus ne survit à la commande, et aucune commande n'a tourné en arrière-plan. Deux démarrages ont eu lieu dans cette même commande :

1. **Campagne**, serveur neuf (65,6 s avant `/ready`, ouverture des six index et chargement du modèle compris). Pour chaque index, dans l'ordre `articles`, `decisions`, `jorf`, `titres`, `sarde`, `dole` (les deux plus lourds en premier) :
   - la **première requête** de l'index est relevée **à part** (`cold_first`) : cache des requêtes vide (ADR-0051), et c'est la première lecture de cet index par ce processus ;
   - puis **une passe d'échauffement non comptée** sur le jeu de l'index ;
   - puis **3 passes comptées** du jeu (ADR-0125, décision 9).
2. **Redémarrage**, second processus neuf (66,5 s avant `/ready`), **une seule passe** du jeu par index, sans échauffement : chaque requête est un **manque du cache d'encodage**. Le cache de pages du système n'est pas purgé pour autant — macOS ne l'expose pas sans `purge` et sans droits d'administration, qui n'ont pas été pris — donc cette passe est « processus froid », pas « disque froid ».

Chaque appel est chronométré à l'horloge murale autour de l'aller-retour HTTP (temps client) et rend, côté serveur, `took_ms` et `debug.timings_ms` (les neuf clés, **global** et celui de la **cible**). Les fichiers bruts ne gardent que l'`id`, l'index, le statut, `took_ms`, le temps client, la génération, la taille du pool et les neuf temps : **ni `per_result`, ni le texte, ni les résultats**.

Durées relevées : démarrage 1 **65,6 s**, campagne **94,3 s**, démarrage 2 **66,5 s**, passe après redémarrage **78,9 s**, soit **5 min 05 s** pour la commande entière. Un **troisième démarrage**, sans aucun appel de recherche, a servi à relire `/v1/stats` avec le bon filtre d'opération et à revérifier `/v1/debug/queries` (66,6 s, `data/t17/raw/serve-stats.log`).

Les percentiles sont au **rang le plus proche** (ADR-0077), sur les valeurs brutes : rang `ceil(p/100 × n)`. Parce que `dense` et `lexical` tournent en parallèle (ADR-0119, décision 6), la part d'une étape se lit en deux colonnes, et **seule la seconde s'additionne** :

- la part **brute** d'une étape est son p50 divisé par le p50 de `total`. Elle ne s'additionne pas : la somme des p50 des termes vaut 48,427 ms pour un p50 de `total` de 50,157 ms, soit 96,6 %, et les deux branches y dépassent 100 % à elles deux. La p50 de chaque terme ne dit rien de la p50 de leur somme ;
- la part **par requête** est, pour chaque requête, `terme / total` de cette requête, puis la médiane de ces parts. Les termes du chemin critique sont `encode`, `filter`, `max(dense, lexical)`, `fusion`, `group` et `fetch` : par construction, ils somment au `total` de la requête, au résidu près (`total − Σ`), et les parts médianes somment à **99,54 %** avec un résidu médian de 0,13 % (0,064 ms).

`max(dense, lexical)` est calculé **requête par requête**, puis sa p50 est prise : sur la campagne chaude elle vaut 47,996 ms, et sur la passe après redémarrage **101,829 ms**, donc bien au-dessus du maximum des deux p50 (90,496 et 91,330) — c'est exactement le piège que la colonne brute évite. `filter` est le temps de construction du pool ; `fetch` la lecture des payloads et du texte ; `group` le regroupement par document ; `fusion` la fusion RRF de l'index puis entre index.

### Résultats

#### 1. Cache chaud, 3 passes comptées, toutes requêtes

291 appels (97 requêtes × 3), tous `200`, aucun `504`. Temps serveur en ms.

| Étape | p50 | p95 | p99 |
|---|---:|---:|---:|
| `validate` | 0,058 | 0,068 | 0,078 |
| `encode` | **0,039** | **0,048** | 0,051 |
| `filter` | 0,047 | 0,259 | 47,965 |
| `dense` | 15,344 | 42,352 | 50,916 |
| `lexical` | **47,996** | **100,593** | 119,268 |
| `fusion` | 0,031 | 0,036 | 0,041 |
| `group` | 0,011 | 0,013 | 0,014 |
| `fetch` | 0,303 | 2,597 | 3,610 |
| `total` | **50,157** | **102,540** | 121,773 |

L'encodage est à 0,039 ms : c'est un **cache chaud** (les 97 requêtes tiennent dans les 1 024 entrées, ADR-0051), et ce chiffre ne décrit pas une requête neuve.

#### 2. Par index, cache chaud

| Index | n | p50 `total` | p95 `total` | p99 `total` | Branche dominante au p50 | p50 de la branche | p50 `dense` | p50 `lexical` |
|---|---:|---:|---:|---:|---|---:|---:|---:|
| `articles` | 75 | 46,162 | 83,720 | 98,242 | lexicale | 41,013 | 11,488 | 41,013 |
| `decisions` | 135 | 75,767 | 107,537 | 122,627 | lexicale | 75,105 | 25,345 | 75,105 |
| `jorf` | 30 | 24,776 | 36,991 | 58,803 | dense | 15,003 | 15,003 | 11,807 |
| `titres` | 9 | 18,259 | 19,598 | 19,598 | lexicale | 17,850 | 13,162 | 17,850 |
| `sarde` | 24 | 5,922 | 7,740 | 8,139 | dense | 5,603 | 5,603 | 3,480 |
| `dole` | 18 | 2,663 | 11,283 | 11,283 | dense | 2,292 | 2,292 | 0,804 |

Sur les trois index les plus lourds, c'est la **branche lexicale (BM25)** qui porte le temps ; sur `jorf`, `sarde` et `dole`, c'est la branche dense, mais les deux y sont petites.

#### 3. Attribution du temps : p50 par terme et part médiane par requête

| Terme | p50 (ms) | Part médiane par requête | Part du p50 de `total` |
|---|---:|---:|---:|
| `encode` | 0,039 | 0,08 % | 0,1 % |
| `filter` | 0,047 | 0,07 % | 0,1 % |
| `max(dense, lexical)` | 47,996 | **98,27 %** | **95,7 %** |
| `fusion` | 0,031 | 0,06 % | 0,1 % |
| `group` | 0,011 | 0,02 % | 0,0 % |
| `fetch` | 0,303 | 0,91 % | 0,6 % |
| **somme des termes** | **48,427** | **99,54 %** | **96,6 %** |
| **`total`** | **50,157** | 100 % | 100 % |

Le résidu par requête, `total − Σ`, vaut 0,064 ms en médiane, soit 0,13 % : les parts par requête somment donc à 99,54 %, et c'est cette colonne qui répond à « quelle étape porte le temps ». La colonne brute ne s'additionne pas : la somme des p50 des termes vaut 48,427 ms pour un p50 de `total` de 50,157 ms, et l'écart de 1,73 ms (3,4 %) n'est pas un travail mesuré, c'est la non-additivité des médianes (la p50 d'une somme n'est pas la somme des p50). L'écart est plus grand par index : 4,53 ms (9,8 %) sur `articles` et 4,16 ms (16,8 %) sur `jorf`.

Par index, la part médiane des branches vaut **98,1 %** (`articles`), 99,2 % (`decisions`), 98,0 % (`jorf`), 97,9 % (`titres`), 95,2 % (`sarde`) et 86,3 % (`dole`) ; celle de `fetch` 0,87 %, 0,50 %, 0,93 %, 1,31 %, 2,48 % et 7,11 %. `fetch` reste donc sous 1,4 % sur les quatre gros index, malgré `return.text: true` et une liste de champs.

**Quatre requêtes sur 97 font exception et paient `filter`.** Les `id` `Q02-APPLICABLE` et `Q30-APPLICABLE` d'`articles` portent un prédicat de dates (forme `date_debut:lte,date_fin:gt` dans `/v1/stats`) : `filter` y coûte **47,8 et 49,0 ms** sur 986 172 passages, stable sur les trois passes, et porte leur `total` à 97,9 et 83,0 ms. `Q01-ANNEE` d'`articles` (`annee:gte,etat:in`, 379 081 passages) paie 9,3 ms de filtre (total 35,1 ms), et `Q21-ANNEE` de `jorf` 17,1 ms sur 63 819 passages (total 26,9 ms). Ces quatre cas sont le seul endroit de la campagne où `filter` dépasse 0,3 ms, et les deux `APPLICABLE` sont le `p99` de `filter` du tableau 1.

#### 4. Première requête de chaque index, à part

Au premier démarrage, cache des requêtes vide et index jamais lus par ce processus :

| Index | `id` | `encode` (ms) | `dense` (ms) | `lexical` (ms) | `total` (ms) | Client (ms) |
|---|---|---:|---:|---:|---:|---:|
| `articles` | `Q01` | **75,204** | 170,303 | 144,101 | 253,420 | 254,788 |
| `decisions` | `Q13` | 30,254 | 220,230 | 219,566 | 255,122 | 255,831 |
| `jorf` | `Q21` | 23,736 | 171,493 | 110,172 | 197,300 | 198,054 |
| `titres` | `MES00-q0011` | 18,921 | 53,581 | 168,864 | 189,532 | 190,381 |
| `sarde` | `Q25` | 20,186 | 16,771 | 19,589 | 41,462 | 42,169 |
| `dole` | `Q28` | 22,736 | 25,533 | 24,737 | 49,669 | 50,434 |

Le premier `encode` de la campagne (`articles`, 75,2 ms) est plus cher que les suivants (18,9 à 30,3 ms) : c'est le premier appel au modèle dans le processus, session ONNX comprise, et il n'est pas représentatif d'une requête froide ordinaire. Les totaux de cette ligne dépassent largement ceux du tableau 2 parce que les deux branches lisent pour la première fois les pages de l'index.

**La passe d'échauffement non comptée le dit mieux encore.** Le cache des requêtes est indexé par le **texte** de la requête (ADR-0051) : cette passe est donc le premier encodage de chaque requête du jeu, sauf la première, déjà relevée à part. Son `encode` est bien **froid**, p50 de 18,0 à 37,8 ms selon l'index (contre 0,0625 ms, seau 0, au repos), et chaque index y paie en plus sa première lecture. p50 de `total` sur cette passe : `articles` 118,1 ms, `decisions` 176,5, `jorf` 137,3, `titres` 68,5, `sarde` 44,2, `dole` 30,0 — à comparer aux 46,2 / 75,8 / 24,8 / 18,3 / 5,9 / 2,7 du tableau 2. **Le premier passage sur un index coûte 2,3 à 11,3 fois le régime** (2,6 sur `articles`, 2,3 sur `decisions`, 5,5 sur `jorf`, 3,8 sur `titres`, 7,5 sur `sarde`, 11,3 sur `dole`).

#### 5. Après redémarrage : encodage à froid, requêtes toutes neuves

97 appels, tous `200`, un par requête du jeu, sans échauffement.

| Étape | p50 | p95 | p99 |
|---|---:|---:|---:|
| `validate` | 0,063 | 0,083 | 0,545 |
| `encode` | **29,389** | **53,248** | 61,576 |
| `filter` | 0,049 | 0,603 | 50,732 |
| `dense` | 90,496 | 155,916 | 210,224 |
| `lexical` | 91,330 | 173,925 | 209,373 |
| `fusion` | 0,031 | 0,044 | 0,204 |
| `group` | 0,010 | 0,013 | 0,017 |
| `fetch` | 2,645 | 4,238 | 5,058 |
| `total` | **134,189** | **231,046** | 255,952 |

Part du p50 de `total` dans cette passe : `encode` **21,9 %**, `max(dense, lexical)` **75,9 %** (p50 de 101,829 ms, au-dessus des 90,496 et 91,330 des deux branches prises séparément), `fetch` 2,0 %. Le résidu de non-additivité, défini comme au §3 par `total − Σ p50 des termes`, vaut ici **0,236 ms, soit 0,2 %** (133,953 ms de somme pour 134,189 ms de `total`).

| Index | p50 `total` | p95 `total` | p50 `encode` | p95 `encode` | p50 `dense` | p50 `lexical` | p50 `total` des 10 dernières requêtes | `encode` de ces 10 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| `articles` | 114,445 | 169,148 | 20,877 | 38,409 | 65,703 | 62,906 | 86,0 | 28,4 |
| `decisions` | 175,289 | 235,890 | 37,623 | 60,539 | 104,677 | 120,674 | 135,2 | 28,3 |
| `jorf` | 120,796 | 193,026 | 23,254 | 37,318 | 90,496 | 84,904 | 120,8 | 23,3 |
| `titres` | 84,435 | 192,807 | 30,105 | 40,858 | 34,543 | 42,407 | 84,4 | 30,1 |
| `sarde` | 41,923 | 60,738 | 25,826 | 45,185 | 13,352 | 14,232 | 41,9 | 25,8 |
| `dole` | 30,141 | 49,027 | 23,510 | 33,247 | 4,016 | 2,735 | 30,1 | 23,5 |

**L'encodage froid est réel et mesuré, mais il n'est pas seul.** Il coûte 20,9 à 37,6 ms au p50 selon l'index (0,039 ms en cache chaud), soit de **18,2 % du `total`** sur `articles` à **78,0 %** sur `dole` (21,5 % sur `decisions`, 35,7 % sur `titres`, 61,6 % sur `sarde`), et 53,2 ms au p95 sur l'ensemble. En revanche les deux branches sont aussi plus lentes qu'en régime : `dense` passe de 15,3 à 90,5 ms au p50 et `lexical` de 48,0 à 91,3 ms. La décroissance à l'intérieur de la passe le montre : sur `articles`, `dense` vaut 154, 139, 88, 129 et 107 ms sur les cinq premières requêtes, puis 30 à 35 ms sur les cinq dernières ; sur `decisions`, 210, 156, 201, 122 et 150 ms sur les cinq premières, 104, 122, 84, 90 et 97 ms sur les cinq dernières. La cause la plus probable est le **cache de pages** : les 25 Go d'index dépassent les 24 Gio de RAM, le démarrage relit des gigaoctets, et macOS ne permet pas de purger ce cache à la demande. **Aucun compteur de défauts de page n'a été relevé** ; l'attribution au cache est une lecture des décroissances, pas une mesure.

#### 6. `took_ms` contre temps client

Sur les 291 appels chauds : `took_ms` p50 50,157 ms contre 50,830 ms côté client, soit un écart p50 de **0,625 ms**, p95 de **0,725 ms** et p99 de **0,820 ms**. Cet écart contient l'aller-retour HTTP, la sérialisation des résultats, du texte et de `per_result`, et la lecture de la réponse — `took_ms` s'arrête avant le rendu JSON (ADR-0120, décision 1). Le surcoût de `debug` (neuf nombres, `plan`, `per_result`) est donc **inférieur à 1 ms** sur cette forme, ce qui est cohérent avec `k = 10` et `passages_per_document = 1`.

#### 7. Contre-lecture par `GET /v1/stats`

Lue sur la fenêtre de la campagne (`from=2026-11-15T14:16:00Z`), `operation=search_multi`, `group_by=minute`, avec la clé `admin` : une fois à la fin du premier démarrage avec le filtre erroné `operation=search`, puis, au troisième démarrage et sans aucun nouvel appel de recherche, avec le bon filtre — c'est cette seconde lecture qui est comparée, et `data/t17/raw/stats.json` garde les deux. `count` = **491**, ce qui égale exactement les 6 premières requêtes froides, les 97 d'échauffement, les 291 des trois passes et les 97 de la passe après redémarrage.

Comparaison avec les percentiles calculés depuis `debug` sur ces mêmes 491 requêtes. Pour une recherche multi-index, l'événement d'une cible porte les étapes **de la cible** et son `total` de cible ; `encode`, mesuré une fois pour la requête, est porté par l'événement de la première cible. La comparaison porte donc, pour `validate` et `encode`, sur le `timings_ms` global, et pour les autres étapes sur le `timings_ms` de la cible.

| Étape | p50 `debug` | p50 `/v1/stats` | p95 `debug` | p95 `/v1/stats` | Rapport p95 |
|---|---:|---:|---:|---:|---:|
| `validate` | 0,061 | 0,0625 | 0,077 | 0,078 | 0,99 |
| `encode` | 0,043 | 0,0625 | 48,467 | 50,487 | 0,96 |
| `filter` | 0,048 | 0,0625 | 0,369 | 0,373 | 0,99 |
| `dense` | 25,092 | 25,849 | 141,239 | 154,074 | 0,92 |
| `lexical` | 66,516 | 78,886 | 138,668 | 154,074 | 0,90 |
| `fusion` | 0,027 | 0,0625 | 0,033 | 0,0625 | 0,53 |
| `group` | 0,011 | 0,0625 | 0,013 | 0,0625 | 0,21 |
| `fetch` | 0,485 | 0,582 | 3,819 | 4,337 | 0,88 |
| `total` | 72,800 | 78,886 | 159,164 | 192,593 | 0,83 |

La lecture est cohérente : pour toutes les étapes au-dessus du seau 0, `/v1/stats` **sur-estime** et l'écart reste sous les 25 % relatifs attendus (ADR-0117, décision 4 ; rapports de 0,83 à 0,99). `fusion` et `group` valent 0,011 à 0,031 ms, donc tombent dans le **seau 0**, dont la borne haute est 0,0625 ms : le rapport de 0,21 n'est pas un désaccord, c'est la résolution. Par index, la même lecture tient : p50 de `total` de `articles` 57,5 mesuré contre 63,1 (seau), `decisions` 91,8 contre 98,6, `sarde` 6,2 contre 6,8, `dole` 2,8 contre 3,5.

**Trois constats de route, utiles pour la voie permanente.**

1. **L'opération est `search_multi`, pas `search`.** Une première lecture avec `operation=search`, à la fin du premier démarrage, a rendu `200` avec **zéro** événement — `buckets` vide, aucune clé `totals` — alors que la campagne venait de tourner ; la même fenêtre lue avec `operation=search_multi` rend 491 événements. La route multi-index écrit un événement par cible sous `search_multi`. `top_filter_shapes` restait rempli dans la lecture erronée (10 formes, 6 index, `200`), parce que cette table ne porte pas de colonne d'opération (ADR-0124, note du 2026-11-01).
2. **Le `total` d'un événement multi-index est le `total` de la cible, `encode` exclu.** C'est net sur `sarde`, **minute 14:19** de la fenêtre (8 événements, ceux de la passe après redémarrage) : `took_ms` p50 41,9 ms, `encode` 25,8 ms, `total` de cible **15,5 ms** — et `/v1/stats` rend, pour cette même minute, **16,5 ms**, la borne haute du seau qui contient 15,5. L'événement porte donc les étapes de la cible et son `total` de cible ; `encode`, mesuré une fois pour la requête, est porté à part par la première cible. Un lecteur qui compare `/v1/stats.total` à `took_ms` sur cette route se trompe de l'encodage entier à froid, et de rien à chaud (encodage en cache). Au commit mesuré (`28c072e`), ce n'était écrit ni dans `docs/05` ni dans `docs/06` ; l'arbre de travail contient depuis une application d'ADR-0156 qui documente les deux points (et nomme `search_multi`), hors de cette campagne, et cette campagne les confirme sur 491 événements.
3. **La borne haute de la fenêtre est ramenée au début de sa minute, et la minute entière est comptée.** `to=2026-11-15T14:19:10Z` et `to=2026-11-15T14:19:40Z` rendent tous deux les 97 événements de la minute 14:19, et l'écho affiche `14:19:00Z`. La fenêtre appliquée est donc **plus large** que celle demandée, jamais plus étroite ; l'écho de `to` n'est pas la borne appliquée, et il ne faut pas s'en servir comme contrôle.

### Lecture

**Trois lignes pour le ticket 17.**

1. Sur la forme servie et en natif, cache chaud, le temps est porté par **la branche la plus lente, presque toujours la lexicale (BM25)** : p50 47,996 ms de `lexical` pour 50,157 ms de `total`, soit **95,7 % du p50 de `total` et 98,27 % de la part médiane du temps par requête** (`dense` 15,344 ms court en parallèle et ne s'ajoute pas). `filter`, `fetch`, `fusion` et `group` sont sous 1 % ; l'encodage est à 0,039 ms.
2. **L'encodage froid ne porte pas le p95 à lui seul.** Dans la passe après redémarrage, il pèse 29,389 ms au p50 et 53,248 ms au p95, soit **21,9 % du p50 et 23,0 % du p95** (134,189 et 231,046 ms). Le reste vient des deux branches, qui repassent de 15,3 / 48,0 ms à 90,5 / 91,3 ms : le processus neuf retrouve un cache de pages partiel, pas seulement un cache de requêtes vide.
3. **Rien ici ne reproduit les 301 ms de Legifacile, et ce rapport ne prétend pas l'expliquer.** En natif, cache chaud, la même forme rend p50 50,2 ms et p95 102,5 ms ; processus neuf et requêtes toutes neuves, 134,2 et 231,0 ms. Le conteneur Docker Desktop (8 processeurs pour la VM, selon le ticket), la charge de la machine et la taille des données par rapport à la RAM restent hors de cette mesure : ce sont les candidats, pas des causes établies.

**Ce que la mesure dit de plus.**

- Le coût est dans les **branches**, pas dans la plomberie de la route. Fusion entre index, regroupement par document, lecture des payloads et du texte, sérialisation : ensemble **1,0 %** de la part médiane du temps par requête sur les six index (`fetch` 0,91 %, `fusion` 0,06 %, `group` 0,02 %), et moins de 1 ms d'écart client.
- Le **filtre** n'est pas un coût général : il est sous 0,3 ms sur 93 des 97 requêtes, et coûte 47,8 à 49,0 ms sur les deux requêtes d'`articles` qui portent un prédicat de dates sur 986 172 passages, puis 9,3 et 17,1 ms sur deux autres prédicats d'intervalle. C'est le seul endroit où une étape autre que les branches approche le p99.
- Le p95 de la campagne chaude (102,5 ms) vient de `lexical` (100,593 ms) et de `dense` (42,352 ms), sur `articles` et `decisions` : c'est la queue des deux plus gros index, pas une requête froide.
- **La lecture permanente est possible par `/v1/stats`** : sur 491 événements, elle redonne les neuf étapes, par index (`&index=`), et l'encodage froid y apparaît au p95 (50,487 ms), là où le p50 le masque (0,0625 ms, seau 0). ADR-0156 (accepté le 2026-11-15) ajoute un filtre `mode` : il n'est **pas nécessaire à Legifacile aujourd'hui**, qui ne sert qu'un mode, mais il le deviendrait le jour où une instance sert du dense et de l'hybride.

**Propositions, à valider par le propriétaire.** Aucune n'est appliquée.

1. **Les deux constats de route sont désormais couverts par ADR-0156, hors de cette campagne** : l'opération d'une recherche multi-index est `search_multi`, et le `total` de son événement exclut `encode`, porté par la première cible. L'arbre de travail (non commité au moment de cette correction) les écrit dans `docs/05` et `docs/06`. Reste au propriétaire à vérifier que l'application est commitée ; cette campagne n'a rien à y ajouter.
2. **Si une correction de latence est cherchée, elle est côté lexicale**, pas côté encodage ni côté payload : c'est la branche qui porte 98,3 % de la part médiane du temps par requête (95,7 % du p50 de `total`), et la seule qui reste au-dessus de 100 ms au p95. Ce rapport ne propose aucun réglage, et aucune modification n'a été faite.
3. **Mesurer dans le conteneur avant de comparer au 301 ms de Legifacile.** La même forme, dans la VM Docker Desktop de 8 processeurs, donnerait le facteur qui manque ; c'est la seule façon de savoir si l'écart natif/conteneur explique 50 → 301 ms.

### Limites

1. **Natif, pas conteneur.** Legifacile mesurait en conteneur Docker Desktop, avec 8 processeurs pour la VM et une machine chargée dont le ticket donne la moyenne mobile. Ici, 8 cœurs natifs et une charge de 2 à 4. MES-05 mesure un coût d'encodage 2,2 à 2,6 fois supérieur dans la VM ; ce facteur n'est **pas** rejoué ici, et aucune extrapolation n'est faite.
2. **Machine de développement, pas au repos.** La charge n'était pas nulle avant la campagne (3,46 sur 8 cœurs). Les chiffres sont donc des majorants indicatifs de ce que rendirait la même machine au repos, et ne disent rien du Mac mini.
3. **« Froid » veut dire processus neuf, pas disque froid.** macOS ne permet pas de purger le cache de pages sans droits d'administration ; la passe après redémarrage trouve un cache de pages partiellement chaud, et les décroissances internes montrent qu'il se réchauffe pendant la passe.
4. **Le régime « cache chaud » suppose une passe d'échauffement.** Les 3 passes comptées suivent une passe d'échauffement non comptée : elles décrivent un serveur qui a déjà lu ses index et qui rejoue des requêtes déjà vues. Le régime d'un serveur qui reçoit des requêtes **toutes neuves** sur des pages chaudes n'est pas mesuré isolément ; il est borné entre le tableau 2 (50 ms) et la queue de la passe après redémarrage (86 ms sur `articles`, 135 ms sur `decisions`).
5. **Un seul client, séquentiel.** Aucune concurrence : le sémaphore de `max_concurrent_searches`, la file d'attente et le `503 overloaded` ne sont pas exercés. Une file d'attente côté Legifacile, si elle existe, n'apparaît pas ici.
6. **Petits échantillons sur trois index.** `titres` (3 requêtes), `sarde` (8) et `dole` (6) ne donnent que 9, 24 et 18 appels chauds : leurs p95 et p99 sont des maxima de fait, pas des queues.
7. **Une seule génération, aucun mort.** Les six index sont à la génération 1, avec 0 passage mort et 0 compaction, issus d'une attache unique. Aucune génération de métadonnées, aucun `patch`, aucune compaction ne sont mesurés.
8. **Le coût de `debug` n'est pas isolé.** Aucune passe sans `debug` n'a été jouée : le surcoût est borné par l'écart client, inférieur à 1 ms, mais il n'est pas mesuré séparément.
9. **`/v1/stats` mêle les phases et les modes.** Les 491 événements de la contre-lecture comprennent la première requête froide de chaque index, la passe d'échauffement, les 3 passes et la passe après redémarrage. Un seul mode est joué ici, donc aucun mélange de modes ne pollue la lecture, mais cette lecture ne dit rien d'une instance qui sert plusieurs modes.
10. **Aucune conclusion sur la cible de Legifacile.** p50 ≤ 150 ms et p95 ≤ 800 ms sont contraignants sur le Mac mini au repos ; cette campagne ne mesure ni ce matériel, ni ce repos, ni ce conteneur.

### Données conservées

Tout est sous `data/t17/`, **25 Go** au total, gitignoré et laissé en place :

- `data/t17/crible.toml` — la configuration de l'instance de mesure, journal de debug désactivé ;
- `data/t17/keys.txt` — les trois clés locales de la campagne (`read`, `write`, `admin`), jamais recopiées dans un rapport ;
- `data/t17/data/indexes/<index>/` — les six index importés, 24,8 Go mesurés par `du` (`decisions` 9,6 Go, `articles` 7,0 Go, `jorf` 4,6 Go, `titres` 3,1 Go, `sarde` 516 Mo, `dole` 28 Mo ; les `disk_bytes` de l'API, 24,6 Go au total, sont dans le JSON) ;
- `data/t17/scripts/measure.py` — le harnais : deux démarrages, `/ready`, première requête à part, échauffement, 3 passes, passe après redémarrage, `/v1/stats`, `/v1/debug/queries`, arrêt propre ;
- `data/t17/scripts/aggregate.py` — l'agrégation : percentiles au rang le plus proche, p50 par terme, part brute du p50 et part médiane par requête (`max(dense, lexical)` requête par requête), résidu de non-additivité, comparaison à `/v1/stats` ;
- `data/t17/scripts/attach_one.py` — l'attache d'un index par la CLI, avec une ligne de progression toutes les 30 s ;
- `data/t17/raw/phase1.json` — premières requêtes froides, passe d'échauffement, 291 appels des trois passes, échos `/v1/stats` et `/v1/debug/queries` du premier démarrage, `uptime` ;
- `data/t17/raw/phase2.json` — les 97 appels de la passe après redémarrage, `uptime` ;
- `data/t17/raw/stats.json` — les lectures `GET /v1/stats` de la campagne (`search_multi`, global et par index), la lecture erronée avec `operation=search`, et `/v1/debug/queries` (`enabled: false`) ;
- `data/t17/raw/stats-search_multi-all.json` — la même fenêtre lue hors ligne par `crible stats` ;
- `data/t17/raw/serve-1.log`, `serve-2.log`, `serve-stats.log`, `serve-probe.log` — les journaux JSON du serveur, un par démarrage ;
- `data/t17/raw/durations.json` — les durées de chaque phase.

Les agrégats versionnés sont dans `docs/mesures/latence-hybride-servie-2026-11-15.json`. Aucun texte de requête, aucune valeur de filtre et aucune clé n'y figurent : les requêtes y sont nommées par leur `id`, et les formes de filtre par leurs champs et opérateurs seulement.
