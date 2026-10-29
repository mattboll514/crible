# 02 — Architecture

## Vue d'ensemble

```
 Producteurs (Python, hors du moteur)                     Clients
 ┌───────────────────────────────┐                ┌────────────────────┐
 │ quotidien : Mac natif (MPS)   │──┐          ┌──│ MCP Legifacile     │
 │ campagnes : OVH (CUDA)        │  │  lots    │  │ (client Python)    │
 └───────────────────────────────┘  │ Parquet  │  └────────────────────┘
                                    ▼          │ HTTP /v1
                    ┌──────────────────────────┴──────────────┐
                    │ Instance Crible (une par projet)        │
                    │  API REST /v1 : auth, limites, jobs     │
                    │  ├─ écrivain : un par index, file bornée│
                    │  ├─ recherche : instantanés, pool rayon │
                    │  ├─ encodeur de requêtes (ONNX fp32)    │
                    │  └─ analytics + journal (asynchrones)   │
                    │  index : articles, jorf, decisions, …   │
                    └─────────────────────────────────────────┘
                      volumes : data · models · exchange
```

## Instance, index, documents, passages

- **Une instance par projet** (ADR-0002). Elle a son conteneur, ses volumes, sa config et ses analytics. Une même image sert à lancer N services.
- **Plusieurs index par instance** (ADR-0003). La règle de découpage :
  - même schéma, même découpage et même gabarit ⇒ même index ;
  - ce qui ne diffère que par une valeur ⇒ un champ filtrable.
- **Un document se compose de passages.** Le document est l'unité de mise à jour ; le passage est l'unité de recherche.

Découpage validé pour Legifacile (cas réel de la v1) :

| Index | Corpus Legifacile actuels | Unité | Champs propres (indicatif) |
|---|---|---|---|
| `articles` | textes_vigueur + textes_hors_vigueur | article | num, texte_cid, hiérarchie, date_debut, date_fin, fonds |
| `jorf` | jorf_normatif | article publié | nature, date de publication |
| `decisions` | juri_texte_integral + juri_sommaires | passage (une partie ou le sommaire) | juridiction, formation, date, numéro ; `partie` au niveau du passage |
| `titres` | titres_textes | titre | nature, dates |
| `sarde` | sarde | bloc | thème |
| `dole` | dole | dossier | type, dates |

Le schéma exact de chaque index est l'objet de la tâche `DEFINIR-INDEX-LEGIFACILE` (P9).

## Identifiants

- **Id externe du document** (`doc_id`) : chaîne UTF-8 d'au plus 512 octets, fournie par le producteur.
- **Clé de passage** : `doc_id#ord`, où `ord` (`u32`) est la position du passage dans le document, à partir de 0.
- **Id interne (`iid`)** : `u32` dense, attribué à l'attache par ordre croissant.
  - Un segment couvre une plage contiguë d'iid.
  - Un iid n'est jamais réutilisé au sein d'une époque.
  - Les iid sont renumérotés à chaque compaction (nouvelle époque).
- **INV-01** : aucun iid ne sort du moteur, ni dans l'API, ni dans les logs, ni dans les analytics.

## Organisation sur disque

```
data/                                   # volume Docker nommé (mmap rapide)
  instance.json                         # format_version, created_at
  analytics.sqlite                      # analytics agrégées (toute l'instance)
  debug.sqlite                          # journal de debug (opt-in)
  jobs.sqlite                           # jobs d'écriture de l'instance, consultables après un redémarrage (ADR-0103)
  tmp/online-<batch_id>/                # lot interne du mode en ligne, protégé par un verrou de présence, supprimé par un garde (ADR-0055, ADR-0056)
  tmp/online-<batch_id>.lock            # verrou de présence du lot en ligne ; un lot est vivant tant qu'il est tenu (ADR-0056)
  indexes/<index>/
    index.json                          # config de l'index, figée + réglable
    CURRENT_EPOCH                       # époque courante (écrit tmp puis rename)
    epochs/<epoch>/
      records.sqlite                    # ÉTAT DE RÉFÉRENCE (WAL)
      segments/<segment_id>/
        segment.json                    # iid_start, count, dims, checksums, generation, batch_id
        vectors.f16                     # n × d × 2 octets, fp16 little-endian (ou vectors.f32)
        codes.bin                       # count × ceil(dims/64) × 8 octets, codes binaires (dérivé ; ADR-0069)
      lexical/                          # index tantivy (dérivé)
      manifests/<generation>.json       # vue dérivée d'une génération
      cache/                            # bitmaps sérialisées, estampillées par génération (dérivé)
models/<model_name>/                    # volume : model.json, model.onnx, model.int8.onnx, tokenizer.json
exchange/                               # dossier monté : inbox/ (lots), outbox/ (exports), snapshots/ (instantanés, ADR-0064)
```

`jobs.sqlite` n'appartient à aucune époque : c'est la table des jobs d'écriture de l'instance, versionnée par sa propre table `meta` (`format_version = 1`), ouverte et écrite par `crible-store` avec les réglages de connexion de `records.sqlite` (ADR-0103). Elle survit à la suppression d'un index.

Tout ce qui est marqué « dérivé » se reconstruit à partir de `records.sqlite` et des fichiers `vectors.*`. La reconstruction a lieu à l'ouverture en **écriture** ; une ouverture en lecture seule n'écrit rien dans l'époque et tient en RAM ce qu'elle doit reconstruire (ADR-0069, ADR-0087). La phrase vaut pour `lexical/` sans exception. Pour `lexical/`, « absent » et « illisible » ont la même conséquence : un état indécidable est reconstruit à l'ouverture en écriture, et la lecture seule publie l'instantané sans lecteur lexical (ADR-0088).

Deux fichiers méritent une description de format propre :

- **`CURRENT_EPOCH`** : un nombre décimal ASCII suivi d'un saut de ligne, sans version propre. La version des formats est portée par `meta.format_version` de `records.sqlite` et par les `format_version` des fichiers de l'époque. Le fichier est écrit dans `tmp-CURRENT_EPOCH`, puis fsync, renommage et fsync du dossier de l'index ; `meta.epoch` en garde une copie, vérifiée à l'ouverture : un désaccord est une erreur interne. `create` fsynce les dossiers de l'époque de bas en haut (`segments/`, puis le dossier de l'époque, puis `epochs/`) avant d'écrire `CURRENT_EPOCH`.
- **`manifests/<generation>.json`** : un JSON canonique — clés triées, compact (ADR-0027) ; versionné (INV-15) — dérivé de SQLite, jamais source de vérité (ADR-0016). Il porte `epoch`, `format_version` (1), `generation` et `segments`, la liste des segments de la génération, triée par `iid_start` puis par `segment_id`. Chaque segment y reprend les champs de son `segment.json` (`iid_start`, `count`, `dims`, `format_version`, `generation`, `batch_id`, `checksums`) et son `segment_id`, tiré du nom du dossier. Il est écrit de façon atomique — fichier `tmp-`, fsync, renommage, fsync du dossier — et la reprise le reconstruit depuis SQLite et les `segment.json` s'il manque.

Hors du volume de données, l'instantané ajoute un troisième fichier de format propre :

- **`exchange/snapshots/<index>-e<E>-g<G>/snapshot.json`** (ADR-0064) : un JSON canonique, **versionné** (`format_version` 1, INV-15) et **sans horodatage**, écrit après la copie de l'instantané — fichier temporaire, fsync, renommage, fsync du dossier. Il porte le nom de l'index, l'époque source E, la génération G et, pour chaque fichier copié sauf lui-même, son chemin relatif, sa taille et son blake3 (`blake3:<hex>`), triés par chemin. Il **atteste une copie de fichiers** : il ne décrit pas une génération, ne remplace ni `manifests/<G>.json` ni `meta.generation`, et la restauration ne s'en sert pas comme source de vérité. Sa présence à la racine du dossier est le signal que la copie est allée au bout.

## Tables SQLite (`records.sqlite`, une base par époque)

| Table | Colonnes principales |
|---|---|
| `meta` | `generation`, `epoch`, `format_version`, `next_iid`, `passage_change_generation` |
| `batches` | `batch_id` (PK), `generation`, `applied_at`, `manifest_blake3`, `manifest` (JSON), `report` (JSON) |
| `segments` | `segment_id`, `iid_start`, `count`, `generation`, `batch_id`, `checksums` |
| `documents` | `doc_id` (PK), `doc_hash`, `fields` (JSON canonique), `updated_generation` |
| `passages` | `iid` (PK), `doc_id`, `ord`, `text` (zstd), `passage_hash`, `fields` (JSON), `segment_id`, `created_generation`, `dead_since_gen` (NULL = vivant) |

Index SQLite : `passages(doc_id)`, `passages(dead_since_gen)`.

- `meta.passage_change_generation` est la **dernière génération qui a créé ou fait mourir un passage** dans l'époque (`0` si l'époque n'en a jamais porté). Elle sert à définir un index lexical à jour, avec la charge utile du dernier commit tantivy (ADR-0088). Elle est écrite dans la transaction de l'étape 3, n'est pas modifiée par une génération qui ne crée ni ne tue aucun passage, et une colonne absente se lit `NULL` : la colonne est optionnelle et additive, sans changement de `format_version` (ADR-0088).

- `batches.manifest_blake3` est le digest blake3 des octets de `manifest.json` tels que lus, sous la forme `blake3:<hex>`, `<hex>` en hexadécimal minuscule (ADR-0041). Il identifie le contenu du manifeste pour comparer deux tentatives du même `batch_id` ; ce n'est pas un contrôle d'intégrité.
- Une ligne `documents` n'existe que pour un document vivant. Sa suppression appartient à la transaction du commit de génération, que le document soit nommé dans `deletions` ou absent d'un lot `replace_all` (ADR-0041). Un document ré-ajouté ensuite est un document neuf.
- `documents.updated_generation` est la génération de la **dernière écriture de la ligne `documents`** : création par une attache, mise à jour de champs par une attache ou par un patch (ADR-0060). Elle sert à sélectionner les documents dont les champs ont changé, pour que leurs passages héritent des nouvelles valeurs lors de la construction des bitmaps. Le `doc_hash` d'un document patché ne change pas : les champs de document sont hors du hash (ADR-0055, `docs/04`).

En v1, les champs sont stockés en JSON canonique (ADR-0019). Les bitmaps et les colonnes de valeurs en sont dérivées.

## Générations et point de commit

Chaque changement visible crée une génération G+1. Le **point de commit unique** est le commit SQLite (ADR-0016).

Attacher un lot (un seul écrivain par index) :

1. **Valider et faire la recette.** Aucun état n'est modifié à cette étape.
2. **Écrire le segment** dans `segments/tmp-<id>/`, faire un fsync, puis le renommer en `segments/<id>/`. Il reste invisible tant que SQLite ne le référence pas.
3. **Transaction SQLite** : `batches`, `segments`, `documents`, `passages`, les pierres tombales, `meta.generation = G+1`, `next_iid`. Quand la transaction crée au moins un passage ou pose au moins une pierre tombale, elle écrit aussi `meta.passage_change_generation = G+1` ; une transaction qui n'en fait ni l'un ni l'autre la laisse telle quelle (ADR-0088). **COMMIT ← point de commit.**
4. **tantivy** : ajouter les nouveaux passages, supprimer les iid tombés, faire un commit avec une charge utile égale à **la génération dont le texte est indexé** (`G+1`), puis recharger le lecteur. Un patch de métadonnées ne produit aucun commit tantivy (ADR-0088).
5. **Écrire** `manifests/<G+1>.json`.
6. **Construire le nouvel instantané de lecture** (bitmaps en copie sur écriture) et le publier (`ArcSwap`).

Modifier des métadonnées : étapes 3, 5 et 6. Supprimer seulement : étapes 3, 4 (suppressions), 5 et 6.

**Patch de métadonnées (ADR-0060).** Une mise à jour de champs de document sans ré-encodage ne touche ni le texte, ni les vecteurs, ni les iid, ni le `doc_hash` :

- l'étape 3 écrit la ligne `documents` de chaque document patché — `fields` et `updated_generation = G+1` — et `meta.generation = G+1`. Elle n'écrit ni `segments`, ni `batches`, ni `passages` ;
- l'étape 4 n'a pas lieu : le texte ne change pas, donc tantivy n'est pas touché ;
- l'étape 5 écrit `manifests/<G+1>.json`, qui reprend **les mêmes segments que G** : un patch ne crée aucun segment ;
- l'étape 6 reconstruit les bitmaps et les colonnes de valeurs en copie sur écriture, puis publie l'instantané, comme pour toute génération (INV-02, INV-03).

Une génération n'est créée que si au moins un document change réellement : un patch dont tous les documents sont inchangés laisse la génération à G, n'écrit aucun manifeste et ne publie aucun instantané.

**Reprise au démarrage**, pour chaque index :

- Lire `CURRENT_EPOCH`, ouvrir SQLite et lire la génération G.
- Supprimer `segments/tmp-*` et tout dossier de segment absent de la table `segments`.
- Supprimer les `tmp-*` résiduels du dossier de l'index (un `tmp-CURRENT_EPOCH` laissé par une création interrompue) et du dossier `manifests/`. La reprise ne touche pas au dossier `epochs/` : un dossier de transit `epochs/tmp-*`, laissé par une compaction ou une restauration interrompue, y reste, inerte, et se supprime à la main ou par l'opération suivante qui vise ce numéro d'époque (ADR-0065).
- Supprimer les dossiers `online-*` résiduels de `<data>/tmp/` : ce sont les lots internes du mode en ligne, qu'un garde n'a pas supprimés (ADR-0055). Un lot n'est résiduel que si son verrou de présence, `<data>/tmp/online-<batch_id>.lock`, peut être pris : le nettoyage ouvre ce fichier en lecture-écriture avec création, tente un verrou exclusif avec `try_lock`, et ne supprime le dossier puis le verrou qu'en cas de succès. Si le verrou est déjà tenu, un écrivain tient le lot et il est laissé en place. Toute autre erreur de verrou (système de fichiers sans verrou, montage réseau) ne supprime rien et est journalisée : la règle est asymétrique à dessein, car un résidu qui s'accumule est un coût alors qu'une suppression de lot vivant est une panne (ADR-0056). Ce nettoyage n'efface rien de plus large que `<data>/tmp/` et ne s'exécute qu'à l'ouverture en écriture.
- Comparer la charge utile du dernier commit tantivy (Gt) et `meta.passage_change_generation` (M) à G, **à l'ouverture en écriture seulement** (ADR-0087, ADR-0088) :
  - **Gt < G avec M ≤ Gt** : aucun rattrapage ni commit tantivy, le pool et les textes de G étant identiques à ceux de Gt (ADR-0088) ;
  - **Gt < G avec M > Gt** : ré-indexer les passages créés après Gt et supprimer ceux morts après Gt ;
  - **Gt > G** : incohérence, reconstruire tout le lexical depuis SQLite.
  - Si la colonne `meta.passage_change_generation` manque, elle est ajoutée par `ALTER TABLE meta ADD COLUMN`, puis M est calculé une fois par époque — `max(created_generation)` et `max(dead_since_gen)` sur `passages`, `0` si la table est vide — et écrit. Cela ne crée ni génération ni manifeste, et se paie une fois par époque, avant la comparaison de reprise et avant la publication de l'instantané (ADR-0088).
  - Une **charge utile non décimale**, un `meta.json` ou un segment que tantivy n'ouvre pas, un dossier `lexical/` qu'il n'ouvre pas, ou un M incohérent (`M > G`) sont un **état indécidable** : ils sont traités comme un dossier absent et empruntent le chemin de `Gt > G` — `remove_index` puis reconstruction — avec une ligne de journal qui nomme l'époque, le cas et le message d'origine. Aucune génération nouvelle, aucun changement du manifeste, aucune erreur (ADR-0088).
  - À l'ouverture en **lecture seule**, aucune comparaison n'a lieu, et rien n'est écrit : pas de `ALTER TABLE`, aucune complétion de M. Le moteur n'ouvre aucun `IndexWriter`, ne commite rien et ne supprime rien sous `lexical/`. Un lecteur tantivy est ouvert si **M ≤ Gt ≤ G** ; une colonne M absente réduit le critère à `Gt = G` et ne l'élargit jamais. Sinon l'instantané n'a pas de lecteur lexical et la branche lexicale est refusée en `overloaded` (503) avec un message qui nomme l'état constaté — retard, avance, état non certifié, ou lexical illisible — sans dégradation silencieuse de `hybrid` vers le dense ; le mode `dense` continue de servir la génération G. Un `lexical/` illisible ne fait pas échouer l'ouverture : elle journalise le cas et publie l'instantané sans lecteur lexical. La réparation appartient à la prochaine ouverture en écriture, qui répare avant de publier son instantané (ADR-0087, ADR-0088).
- Écrire le manifeste G s'il manque.
- Reconstruire `codes.bin` des segments où il manque, depuis les `vectors.*` du segment, et le persister par fichier temporaire, fsync et renommage (ADR-0069). Un fichier absent n'est pas une erreur ; un fichier de longueur fausse en est une (`Internal`, fichier nommé). La reconstruction ne crée ni génération ni époque. Elle n'a lieu qu'à l'ouverture en **écriture** : un segment ouvert en lecture seule construit les codes manquants en RAM et n'écrit rien dans l'époque. `lexical/` suit la règle inverse pour une charge utile ou un dossier illisible : l'état est indécidable, donc il est reconstruit à l'ouverture en écriture au lieu d'être refusé, parce que la charge utile est l'unique état de l'index, que le format appartient à tantivy et qu'un refus rendrait l'index non ouvrable pour écrire (ADR-0088).
- Construire les bitmaps, ou charger le cache estampillé G.
- Publier l'instantané.

Invariants concernés : **INV-03** (une génération est visible entièrement ou pas du tout) et **INV-04** (au redémarrage, l'état est la dernière génération commitée).

## Concurrence

- **Un écrivain par index** : une tâche tokio avec une file bornée. Les écritures d'un index sont sérialisées ; les lectures ne sont jamais bloquées (**INV-02**).
- **La portée de cette règle est l'index, pas la racine de données.** `<data>/tmp/` est commun à tous les index d'une racine : deux index distincts y ont chacun un écrivain légitime, et l'ingestion en ligne peut donc s'y dérouler en parallèle. Ce qui arbitre ces rencontres n'est pas la règle « un écrivain par index », mais le **verrou de présence** du lot en ligne (ADR-0056) : l'ingestion prend un verrou exclusif sur `<data>/tmp/online-<batch_id>.lock` avant de créer le dossier du lot et le tient jusqu'après `attach`. Si le verrou est déjà tenu, l'ingestion échoue en `overloaded` sans rien créer ; elle ne détruit ni ne bloque un lot qu'un autre processus tient.
- **Chaque requête prend un `Arc<Snapshot>` au départ.** L'instantané contient les segments (fp16 en mmap, codes en RAM), les bitmaps, les colonnes de valeurs, le `Searcher` tantivy et la génération.
- **Libération.** Un instantané est libéré quand son dernier lecteur se termine. Les fichiers d'une époque supprimée ne sont effacés qu'après.
- **Payloads.** Les textes et champs des résultats viennent d'un pool de connexions SQLite en lecture (WAL).
  - Limite connue : les champs d'un document peuvent refléter une génération plus récente que celle utilisée pour filtrer. Deux causes produisent cet écart, toutes deux acceptées : une modification de métadonnées, et la suppression du document, dont la ligne `documents` a alors disparu (ADR-0041). Dans ce second cas, le résultat est rendu avec son texte et ses champs de passage ; ses champs de document sont absents du résultat, sans marqueur et sans erreur (ADR-0043).
  - Textes et vecteurs sont immuables pour un iid donné.
- **Calcul.** Les scans tournent sur un pool rayon dédié, jamais sur les threads tokio.

## Suppressions, pierres tombales et compaction (option B, ADR-0006)

| Changement | Exemple Legifacile | Action | GPU | Nouvel iid |
|---|---|---|---|---|
| Nouveau document | nouvel arrêt | ajout de passages | oui (producteur) | oui |
| Texte modifié (`doc_hash` différent) | article modifié | pierres tombales sur les anciens passages, ajout des nouveaux | oui (producteur) | oui |
| Republication identique (`doc_hash` égal) | ~1 230 par nuit | rien, sauf les champs de document s'ils diffèrent | non | non |
| Métadonnée seule | changement de champ | mise à jour du champ et des bitmaps | non | non |
| Suppression | ~220 par nuit | pierres tombales | non | — |

- Une pierre tombale fixe `dead_since_gen`. Le passage sort de la bitmap des vivants, donc du pool : **aucun résultat ne manque** à cause d'une suppression.
- Ce que coûtent les morts :
  - de l'espace : codes en RAM, vecteurs sur disque ;
  - des statistiques BM25 légèrement faussées ;
  - un rangement physique qui se dégrade.
- **Compaction** : déclenchée quand le ratio de morts atteint `compaction.dead_ratio` (0,10 par défaut), ou à la main.
  1. Suspendre l'écrivain. Les écritures attendent dans la file, les lectures continuent (ADR-0017).
  2. Lire les passages vivants de l'instantané courant, triés par (`cluster_by`, `doc_id`, `ord`).
  3. Écrire l'époque E+1 :
     - des segments d'au plus 1 M passages ;
     - un nouveau SQLite (documents, passages renumérotés, historique `batches` conservé pour l'idempotence) ;
     - tantivy reconstruit depuis le texte ;
     - les codes binaires.
  4. Vérifier : comptes, checksums, et équivalence d'un échantillon de requêtes (**INV-09**).
  5. Écrire `CURRENT_EPOCH ← E+1` (tmp puis rename). C'est le point de bascule ; la génération passe à G+1.
  6. Publier l'instantané, relancer l'écrivain, puis supprimer l'époque E quand plus aucun lecteur ne la tient.
- La compaction ne demande aucun GPU, mais il faut temporairement deux fois l'espace disque.
- Pour Legifacile, avec ~1 % de passages morts par mois, elle sert surtout après les campagnes de ré-encodage.

## Restauration d'un instantané (ADR-0065)

`crible restore` est **hors ligne**, comme `crible snapshot` : l'opérateur arrête le serveur, et aucun verrou inter-processus n'est ajouté (même règle d'exploitation que `crible patch`, ADR-0038). La commande n'encode rien, ne charge pas le modèle et ne fait aucune recette.

1. **Contrôler l'instantané** : `snapshot.json` présent, `format_version` connu, chaque fichier listé présent avec sa taille et son blake3, aucun fichier non listé (ADR-0064). Un écart est refusé en `invalid_request`, avant toute écriture.
2. **Contrôler la cible** : le nom de l'index vient de `snapshot.json`. Un index absent est créé depuis l'`index.json` de l'instantané, avec l'époque initiale d'une création et la génération G. Un index existant doit porter un `index.json` **identique** ; sinon le refus nomme le réglage divergent. Une cible à une génération supérieure à G est acceptée : c'est le retour arrière voulu.
3. **Écrire l'époque E+1** : la copie va dans `epochs/tmp-<E+1>/`, puis fsync et renommage vers `epochs/<E+1>/`, comme pour la compaction. Un dossier de transit ou une époque E+1 déjà présents sont supprimés avant l'écriture. `meta.epoch` de la base copiée et `manifests/<G>.json` reçoivent le numéro E+1.
4. **Basculer** : `CURRENT_EPOCH ← E+1` en dernier, tmp puis renommage. Une panne avant cette écriture laisse l'index cible intact (INV-03, INV-04).

La **génération reste G** : une restauration ne l'incrémente pas, c'est ce qui satisfait « une restauration redonne la génération ». L'époque est copiée en entier, donc les iid sont conservés et restent denses : aucun renumérotage, et les résultats sont identiques sans reconstruction. Les `segment.json` ne sont pas réécrits — leur `generation` et leur `batch_id` disent la provenance du segment, comme pour une compaction. Seuls `meta.epoch`, le manifeste de G et `CURRENT_EPOCH` changent. Le couple (époque, génération) n'est donc pas monotone dans ses deux composantes.

**L'époque remplacée n'est pas supprimée** : elle reste sur disque, le rapport la nomme, et elle se supprime à la main. Dès qu'une écriture a suivi la prise de l'instantané, elle contient des passages qu'aucune copie valide ne porte ; `reclaim_epochs` la détruirait, ce qui violerait INV-14 (ADR-0065).

## Mémoire : ce qui est en RAM

**Règle : ce qui est parcouru en entier à chaque requête est en RAM ; ce qui est lu seulement pour quelques candidats reste sur disque (mmap).**

| Pièce | Où | Ordre de grandeur pour Legifacile (8,58 M × 768) |
|---|---|---|
| Codes binaires | RAM, chargés en mémoire (pas en mmap, pour ne pas être évincés) | 0,82 Go |
| Bitmaps (keyword, bool, vivants) | RAM | quelques Mo à ~50 Mo |
| Colonnes de valeurs (nombres, dates) | RAM | ~34 Mo par champ date (`i32`) |
| Modèle ONNX fp32 | RAM, chargé au premier usage (une requête suffit) | plus d'1 Go |
| Modèle ONNX INT8 | RAM, chargé seulement si `query_precision = "int8"` (comparaison, repli, diagnostic) | ~0,3 Go |
| Vecteurs de référence fp16 | disque, mmap | 13,2 Go |
| SQLite (texte zstd, champs) | disque | ~4 à 5 Go (estimation) |
| tantivy | disque, mmap | à mesurer (MES-06) |

Le minimum incompressible est d'environ 1,2 Go quand seule la session INT8 est chargée ; une instance qui sert des requêtes charge la session fp32 (plus d'1 Gio) dès la première requête, et son minimum passe donc à environ 2,0 Go (ADR-0052). À cela s'ajoute ~34 Mo par champ date. Toute la RAM restante sert de cache au système. Un même modèle n'est chargé qu'une fois par instance : il est partagé par tous les index qui ont la même empreinte.

Les sessions de l'encodeur sont indexées par `(empreinte du modèle, précision)` et chargées **paresseusement**, chacune au premier usage de sa précision (ADR-0051). Les requêtes étant encodées en **fp32 par défaut** (ADR-0052), une instance qui ne sert que des requêtes ne charge que le fp32 : c'est le budget d'environ 2,0 Go. La session INT8 (~0,3 Go) n'apparaît que si `[encoder].query_precision = "int8"`, pour une comparaison (MES-04) ou un diagnostic. Toute session chargée est **gardée jusqu'à l'arrêt de l'instance** : une instance qui a fait une recette ou servi des requêtes INT8 porte donc les deux sessions.

## Évolutions prévues hors v1

- **Partitionnement IVF** par segment, quand un index dépasse ~30 M passages.
- **Fusions progressives** de segments (option C), si la compaction complète devient trop longue.
- **Rerank distant**, comme étape optionnelle après la fusion.
- **Écritures pendant la compaction**, grâce à un journal de rejeu.
