# ADR-0039 — Ordre des lignes dans un lot v1

- **Statut** : Proposé
- **Date** : 2026-10-21
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, sur demande de l'orchestrateur, à la suite de la tâche LIRE-LOT-PARQUET (phase P1), après un point bloquant du relecteur et un verdict NON-COUVERT de l'architecte
- **Remplace** : —

## Contexte

La tâche LIRE-LOT-PARQUET demande au lecteur Rust (`crible-store::batch_reader`) de valider un lot sans charger tout le lot en mémoire : le critère 4 de la fiche impose une lecture « partie par partie », jamais fichier entier (`docs/phases/P1-stockage-et-recherche-exacte.md`). Le lecteur lit donc les passages en flux, par `passages-NNNN.parquet`, par lots d'enregistrements de 8 192 lignes au plus.

Le contrôle 6 de `docs/04-donnees-et-ingestion.md:171` exige que « les `ord` sont contigus ». `docs/04-donnees-et-ingestion.md:96` et la colonne `ord` de `docs/format-lot-v1.md:196` (« Rang du passage dans le document, contigu et à partir de 0 ») imposent la contiguïté des **valeurs**. Aucun de ces textes n'impose l'**ordre des lignes** dans les fichiers Parquet.

Or le lecteur ne peut pas prouver la contiguïté ligne à ligne sans retenir, par document, les `ord` déjà vus. Pour garder une mémoire d'une entrée par document, il exige aujourd'hui que les lignes d'un document arrivent dans l'ordre croissant des `ord` à partir de 0, que la séquence continue d'une partie à l'autre, et que les parties soient parcourues dans l'ordre du manifeste. Un lot conforme aux documents actuels, mais dont les lignes seraient permutées, est donc refusé.

`docs/format-lot-v1.md:23-31` autorise un rôle réparti sur plusieurs fichiers (`passages-0001.parquet`, …), sans fixer l'ordre de parcours de ces parties : la règle appliquée par le lecteur manque donc à la spécification.

Deux décisions dépendent de ce point :

- le lot est un **format public** (ADR-0012). Restreindre ce qu'un lot peut contenir est une décision, au sens de la règle 4 d'`AGENTS.md` ;
- aucun ADR `Accepté` ne traite de la lecture en flux ni de l'ordre des lignes. Le commentaire de `crates/crible-store/src/batch_reader.rs` renvoie à « pending ADR-0039 ».

Le seul producteur existant est le `BatchWriter` Python (`clients/python/src/crible/batch.py:398`, `:432-470`). Il numérote les passages par `enumerate`, découpe les parties sans réordonner les lignes et peut couper au milieu d'un document. Il est donc déjà conforme à la règle proposée, et les lots déjà écrits restent valides.

## Décision

Dans un lot v1, **les lignes des passages d'un même document apparaissent dans l'ordre croissant des `ord`, à partir de 0**. Un document peut s'étendre sur plusieurs parties ; sa séquence continue alors d'une partie à l'autre. Les fichiers `passages-NNNN.parquet` se parcourent par suffixe croissant.

Cette règle complète la contiguïté des valeurs par l'ordre des lignes. Elle ne dit rien du placement des documents entre eux : l'ordre relatif de deux documents distincts reste libre.

**`format_version` reste à 1.** La règle est une **restriction** de ce qu'un lot peut contenir, pas un changement de forme : le schéma, les colonnes, les types et le manifeste sont inchangés, et tout lot déjà accepté reste lisible. Aucun producteur conforme au `BatchWriter` actuel ne change. Le propriétaire tranche explicitement ce point en acceptant cet ADR : si la restriction était jugée incompatible, elle prendrait le numéro de version suivant.

Sont hors du périmètre de cet ADR :

- le mélange des deux formes de nommage pour un même rôle (`documents.parquet` et `documents-0001.parquet` dans le même manifeste), qui n'est pas contrôlé (`docs/format-lot-v1.md:31-35`), alors que l'ordre de parcours des parties en dépend aussi ;
- le refus d'un `doc_id` dupliqué dans `documents`, que le lecteur n'applique pas aujourd'hui.

Ces deux points demandent une décision séparée (voir « Points liés »).

## Conséquences

- **Positives** :
  - la mémoire reste **O(1) par document** : une seule entrée par document déclaré, jamais une par passage, et le critère 4 de la fiche est tenu ;
  - la vérification de la contiguïté est **exacte** : aucun faux positif possible, ce qui est cohérent avec le principe « exact par défaut » ;
  - aucun producteur actuel n'est touché : le `BatchWriter` Python écrit déjà dans cet ordre ;
  - le refus est explicite : le message d'erreur dit que les lignes doivent arriver dans l'ordre croissant des `ord`.
- **Négatives / coûts** :
  - un producteur tiers qui permuterait les lignes, ou qui écrirait les documents dans un ordre quelconque à l'intérieur d'un fichier, serait refusé alors que les documents actuels ne l'interdisent pas ;
  - la règle doit être écrite dans la spécification publique, sous peine d'être une exigence cachée du lecteur.
- **À surveiller** : toute demande d'un producteur qui ne peut pas écrire dans cet ordre, ou un besoin de lecture parallèle des parties. L'un ou l'autre rouvrirait le choix, avec un coût mémoire à mesurer.

## Application

Après acceptation seulement, et dans un commit qui cite `ADR-0039` :

- `docs/format-lot-v1.md` : colonne `ord` (`:196`), section « Dossier d'un lot » (`:23-31`) pour l'ordre de parcours des parties, liste des contrôles (`:243`) ;
- `docs/04-donnees-et-ingestion.md:96` et `:171` ;
- `crates/crible-store/src/batch_reader.rs` : retirer la mention « pending ADR-0039 » du commentaire de module et citer l'ADR.

## Points liés, hors décision

- **Mélange des formes de nommage.** `docs/format-lot-v1.md:31-35` n'interdit pas `documents.parquet` et `documents-0001.parquet` dans le même manifeste. L'ordre de parcours retenu ici suppose une numérotation cohérente par rôle ; ce contrôle demande un ADR ou une tâche à part.
- **`doc_id` dupliqué dans `documents`.** Le lecteur ne refuse pas encore un document déclaré deux fois. La décision est à prendre séparément.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| **B.** Accepter tout ordre de lignes, avec une vérification exacte de la contiguïté. | Il faut retenir les `ord` vus pour chaque document : l'état devient proportionnel au nombre de passages, ce qui contredit le critère 4 de la fiche. Un tri externe ou une seconde passe ajoute du coût et de la complexité pour un cas qu'aucun producteur connu ne produit. |
| **C.** Accepter tout ordre de lignes, avec une vérification par compteurs (nombre de passages, somme et somme des carrés des `ord`). | La mémoire reste O(1), mais ce n'est plus une preuve : des collisions restent possibles, et le lecteur accepterait des lots faux. C'est contraire au principe « exact par défaut ». |

## Références

- `docs/format-lot-v1.md` : `:23-31` (dossier d'un lot), `:196` (colonne `ord`), `:243` (contrôle 6).
- `docs/04-donnees-et-ingestion.md` : `:96` (règles des passages), `:171` (contrôle 6).
- `docs/phases/P1-stockage-et-recherche-exacte.md`, tâche LIRE-LOT-PARQUET, critère 4.
- `clients/python/src/crible/batch.py:398`, `:432-470` : producteur Python, déjà conforme.
- ADR-0012 : format de lot public (Parquet + manifeste), format interne privé.
- `crates/crible-store/src/batch_reader.rs`, commentaire de module : règle appliquée par le lecteur et mention « pending ADR-0039 ».
