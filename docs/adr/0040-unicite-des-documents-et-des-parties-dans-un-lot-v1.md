# ADR-0040 — Unicité des documents et des parties dans un lot v1

- **Statut** : Accepté
- **Date** : 2026-10-22 (proposition et acceptation)
- **Décideurs** : propriétaire
- **Décision** : accepté le 2026-10-22 par le propriétaire, décision recommandée, sans modification.
- **Proposé par** : agent greffier, sur demande de l'orchestrateur, à la suite de la relecture de la tâche LIRE-LOT-PARQUET (phase P1), pour trancher deux points laissés ouverts par l'ADR-0039
- **Remplace** : —

## Contexte

La tâche LIRE-LOT-PARQUET est `FAIT` : le lecteur Rust (`crible-store::batch_reader`) valide un lot v1 partie par partie, sans charger le lot entier en mémoire. Sa relecture a laissé deux manques dans `docs/format-lot-v1.md`, tous deux situés dans le même format public (ADR-0012). Aucun ADR `Accepté` ne les tranche ; l'ADR-0039 les avait explicitement mis hors de son périmètre (`:37-38`, `:62-65`).

### Unicité d'un `doc_id`

`docs/format-lot-v1.md:210-211` ne dit qu'une chose sur les doublons : « Un `doc_id` présent à la fois dans `documents` et dans `deletions` fait rejeter le lot. » `docs/04-donnees-et-ingestion.md:156` dit la même chose. Ni l'un ni l'autre n'impose qu'un `doc_id` soit **unique dans `documents`**, toutes parties confondues, ni dans `deletions`.

Le lecteur ne refuse pas ces doublons aujourd'hui. Il les fusionne en silence (`crates/crible-store/src/batch_reader.rs:308-314` : « a duplicated `doc_id` is merged here rather than refused ») et n'en garde qu'une entrée. Rien n'interdit alors au même `doc_id` d'arriver avec deux `doc_hash` différents, ou deux jeux de champs différents, dans deux parties.

La suite dépend de `ATTACHER-LOT`, dont la fiche n'est pas encore écrite. La table `documents` de `records.sqlite` déclare `doc_id TEXT PRIMARY KEY` (`crates/crible-store/src/records.rs:89`). Un doublon produit donc, au choix, une violation de clé primaire SQLite remontée en `internal` alors que la faute vient du lot, ou un « la dernière ligne gagne » qui dépend de l'ordre de parcours des fichiers. Les deux affaiblissent l'idempotence (INV-10 : rejouer un lot ne fait rien) et la règle « même `doc_hash`, même contenu » (`docs/04-donnees-et-ingestion.md:153-154`).

Le producteur Python refuse déjà ces doublons : `clients/python/src/crible/batch.py:252` (« document … is already part of this batch ») et `:298` (« … is already deleted in this batch »).

### Unicité d'une partie

Les parties numérotées (`documents-NNNN.parquet`, `passages-NNNN.parquet`) portent « au moins quatre chiffres » et « se parcourent dans l'ordre croissant de leur suffixe » (`docs/format-lot-v1.md:25-33`, complété par l'ADR-0039). La largeur du suffixe est donc libre à partir de quatre chiffres. Deux parties d'un même rôle dont les suffixes ont la **même valeur numérique** mais des largeurs différentes, par exemple `passages-0001.parquet` et `passages-00001.parquet`, rendent cet ordre indéfini.

Le lecteur suit aujourd'hui l'ordre lexicographique des noms, hérité de la `BTreeMap` du manifeste (`crates/crible-core/src/batch.rs:397-399`, `:407-417`). Cet ordre ne coïncide avec l'ordre numérique que si les largeurs sont égales : `passages-00001.parquet` passe avant `passages-0001.parquet`. Le `BatchWriter` Python écrit des suffixes de largeur fixe (`:04d`, `batch.py:448`), donc le cas ne se produit pas avec lui ; il reste possible pour un producteur tiers.

### Pourquoi une décision

Le lot est un format public. Ajouter une contrainte sur ce qu'un lot peut contenir est une décision, au sens de la règle 4 d'`AGENTS.md`. Elle doit figurer dans la spécification, sous peine d'être une exigence cachée du lecteur.

## Décision

Dans un lot v1, **un `doc_id` apparaît au plus une fois dans `documents`, toutes parties confondues, et au plus une fois dans `deletions`**. Un doublon, dans l'un ou l'autre rôle, fait rejeter le lot avec `400 invalid_request` par le lecteur de lot, avant toute écriture.

**L'ordre des parties d'un rôle est l'ordre croissant de la valeur numérique de leur suffixe.** Deux parties d'un même rôle qui portent la même valeur numérique font rejeter le lot.

Précisions :

- l'unicité est **par lot**, pas entre lots : deux lots successifs peuvent porter le même `doc_id`, et c'est le sens d'`upsert` ;
- la règle existante sur `documents` et `deletions` à la fois est inchangée : un `doc_id` dans les deux rôles fait toujours rejeter le lot ;
- **`format_version` reste à 1.** Le producteur Python respecte déjà les deux règles (`batch.py:252`, `:298`, `:448`), et aucun lot produit jusqu'ici n'est refusé. La décision restreint ce qu'un lot peut contenir, elle ne change ni le schéma, ni les colonnes, ni le manifeste, et tout lot déjà accepté reste lisible. Si le propriétaire jugeait la restriction incompatible avec un producteur existant, elle prendrait le numéro de version suivant ;
- le mélange des deux formes de nommage pour un même rôle (`documents.parquet` et `documents-0001.parquet` dans le même manifeste) reste hors périmètre : il n'est toujours pas contrôlé (`docs/format-lot-v1.md:35-38`), et l'ordre des parties n'y suffit pas à le trancher.

## Conséquences

- **Positives** :
  - une faute du lot est signalée comme telle, en `invalid_request`, au moment de la lecture et avant toute écriture, au lieu de ressortir en `internal` depuis SQLite ;
  - l'idempotence (INV-10) et la règle « même `doc_hash`, même contenu » ne dépendent plus de l'ordre des fichiers : un `doc_id` n'a qu'un `doc_hash` et qu'un jeu de champs dans un lot ;
  - l'ordre des parties ne dépend plus de la largeur du suffixe, donc le lecteur et un producteur tiers s'accordent sur le même parcours ;
  - le refus des doublons dans `documents` ne coûte rien : le lecteur tient déjà une entrée par `doc_id` (`batch_reader.rs:295-315`), il suffit de transformer la fusion en erreur ;
  - aucun producteur actuel n'est touché et aucun lot déjà produit n'est refusé.
- **Négatives / coûts** :
  - c'est une restriction d'un format public : un producteur tiers qui répéterait un `doc_id`, ou qui numéroterait ses parties de façon incohérente, serait refusé alors que les documents actuels ne l'interdisent pas ;
  - le refus des doublons dans `deletions` ajoute un ensemble d'ids au lecteur, qui ne comptait jusqu'ici que les lignes ; la mémoire croît avec `counts.deletions`, borne déjà déclarée par le manifeste ;
  - le lecteur doit changer avant `ATTACHER-LOT` : tri numérique des parties et refus des doublons. C'est une petite tâche hors plan, à ajouter au suivi.
- **À surveiller** : toute demande d'un producteur qui aurait besoin de répéter un `doc_id` dans un lot, ou de parties de largeurs mélangées. L'un ou l'autre rouvrirait la décision, avec un coût mémoire à mesurer. Surveiller aussi le cas du mélange des formes de nommage, qui reste non contrôlé.

## Application

Après acceptation seulement, et dans un commit qui cite `ADR-0040` :

- `docs/format-lot-v1.md` : dossier d'un lot (`:25-33`, ordre numérique et refus d'une valeur répétée) ; tableaux `documents.parquet` et `deletions.parquet` (`:173-211`, unicité du `doc_id`) ; contrôles à l'attache (`:229-250`) ; table des erreurs (`:254-261`, `400 invalid_request`) ;
- `docs/04-donnees-et-ingestion.md` : section « Sémantique d'attache » (`:148-157`) et liste des contrôles (`:164-187`) si elle énumère ces règles ;
- code, dans une petite tâche hors plan : `crates/crible-core/src/batch.rs` (tri des parties par valeur numérique) et `crates/crible-store/src/batch_reader.rs` (refus des doublons dans `documents` et `deletions`, suppression de la note sur la fusion silencieuse).

`ATTACHER-LOT` pourra alors supposer un `doc_id` unique par lot.

## Points liés, hors décision

- **Mélange des formes de nommage.** `documents.parquet` et `documents-0001.parquet` dans le même manifeste restent acceptés (`docs/format-lot-v1.md:35-38`). Ce point avait été laissé ouvert par l'ADR-0039 et le reste : il demande sa propre décision.
- **Ordre des lignes.** L'ordre croissant des `ord` dans un document reste régi par l'ADR-0039 ; cet ADR ne le modifie pas.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| **B.** Refuser les doublons dans `ATTACHER-LOT` plutôt que dans le lecteur de lot. | La détection arrive plus tard, après le décodage des parties, et laisse le doublon traverser les contrôles de comptes. Il faut ajouter une règle à la fiche `ATTACHER-LOT`, qui n'est pas écrite, et la faute risque d'être découverte par SQLite, donc en `internal`. Le lecteur tient déjà les `doc_id` : le refus y est gratuit. |
| **C.** Accepter les doublons dans `documents`, avec « la dernière ligne gagne ». | Le résultat dépend de l'ordre des fichiers et reste silencieux. Deux `doc_hash` ou deux jeux de champs coexisteraient pour un même document, ce qui contredit la règle « même hash, même contenu » et fragilise INV-10. Le producteur Python le refuse déjà. |
| **D.** Tolérer les doublons dans `deletions`, la suppression étant idempotente. | La suppression serait en effet sans effet supplémentaire, mais la règle serait asymétrique avec `documents` et avec le producteur Python, qui refuse (`batch.py:298`). Un doublon y est un défaut du producteur, et le lecteur lit déjà toutes les lignes pour vérifier `counts.deletions` : l'ensemble ne coûte presque rien. |

## Références

- `docs/format-lot-v1.md` : `:25-33` (dossier d'un lot, parties), `:35-38` (mélange des formes), `:173-211` (colonnes `documents` et `deletions`), `:229-250` (contrôles à l'attache), `:254-261` (erreurs).
- `docs/04-donnees-et-ingestion.md` : `:89-97` (modèle documents → passages), `:148-157` (sémantique d'attache), `:164-187` (contrôles et recette).
- `docs/08-invariants-et-tests.md:18` (INV-10).
- `docs/phases/P1-stockage-et-recherche-exacte.md`, tâche LIRE-LOT-PARQUET.
- `crates/crible-store/src/batch_reader.rs:295-315` : fusion silencieuse d'un `doc_id` dupliqué dans `documents` ; `:374-389` : lecture de `deletions`.
- `crates/crible-core/src/batch.rs:396-417` : ordre lexicographique des parties, hérité de la `BTreeMap` du manifeste.
- `crates/crible-store/src/records.rs:89` : `doc_id TEXT PRIMARY KEY` dans la table `documents`.
- `clients/python/src/crible/batch.py:252`, `:298`, `:448` : producteur Python, déjà conforme.
- ADR-0012 : format de lot public (Parquet + manifeste), format interne privé.
- ADR-0039 : ordre des lignes dans un lot v1 ; points liés laissés ouverts.
