# ADR-0041 — Sémantique d'attache : document supprimé, digest du manifeste et rapport

- **Statut** : Proposé
- **Date** : 2026-10-22
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, sur demande de l'orchestrateur, à la suite de la préparation de la tâche ATTACHER-LOT (phase P1) ; trois points relevés par `@testeur` et instruits par `@architecte`
- **Remplace** : —

## Contexte

La tâche ATTACHER-LOT est commencée sur la branche `tache/ATTACHER-LOT`, avant implémentation. Sa fiche impose d'intégrer un lot selon la sémantique d'attache, de poser les pierres tombales et d'écrire toutes les données dans une seule transaction, celle du commit de génération (`docs/phases/P1-stockage-et-recherche-exacte.md:104-122`). Trois points de cette sémantique ne sont tranchés par aucun ADR `Accepté` et ne sont pas couverts par la conception actuelle. Deux sont **NON-COUVERT** ; le troisième est un **écart** entre deux documents.

Cet ADR porte **trois décisions séparées**. Chacune peut être acceptée ou rejetée indépendamment : l'option recommandée est indiquée, et les alternatives sont décrites avec la raison de leur écart. Si le propriétaire en refuse une, les deux autres tiennent.

### Décision 1 — ligne `documents` d'un document supprimé (NON-COUVERT)

`docs/04-donnees-et-ingestion.md:155` dit, pour un document dans `deletions` : « Pierres tombales sur tous ses passages ; document retiré ». Il ne dit pas quelle ligne tombe. La table `documents` de `records.sqlite` ne porte que `doc_id`, `doc_hash`, `fields` et `updated_generation` (`docs/02-architecture.md:92`). Le seul état de suppression est `passages.dead_since_gen` : un document n'a pas d'état propre.

Les phases suivantes acceptent les deux lectures :

- la compaction lit les passages vivants et écrit un nouveau SQLite avec les `documents` (`docs/02-architecture.md:156-158`), sans exiger que la ligne existe encore ;
- l'export P3 produit un lot `replace_all` des données vivantes, et le contrôle « chaque document a au moins un passage » (`docs/04-donnees-et-ingestion.md:171`) porte sur les passages ; un filtrage sur les passages vivants suffit ;
- `explain` (P7) distingue deux booléens, `exists` et `alive` (`docs/05-api.md:182-186`), ce qui laisse ouverte la définition de `exists`.

Le piège est ailleurs. Si la ligne `documents` survit à la suppression avec son `doc_hash`, un `upsert` ultérieur du même `doc_id` avec le même `doc_hash` tombe dans la ligne « Connu, `doc_hash` identique → passages ignorés ; champs de document mis à jour s'ils diffèrent » (`docs/04-donnees-et-ingestion.md:153`). Le document reste alors vide pour toujours : ses passages sont morts, et l'attache refuse de les réécrire puisque le hash n'a pas changé. Un ré-ajout légitime devient impossible sans changer le `doc_hash` ou compacter.

### Décision 2 — base de `manifest_blake3` (NON-COUVERT)

`docs/02-architecture.md:90` (table `batches`) et `docs/04-donnees-et-ingestion.md:162` nomment `manifest_blake3` sans le définir. La règle d'idempotence en dépend : un `batch_id` déjà appliqué ne fait rien et renvoie le rapport d'origine (INV-10) ; le même `batch_id` avec un autre manifeste, c'est-à-dire un `manifest_blake3` différent, renvoie `409 batch_conflict`.

La seule base écrite pour un digest dans le projet est celle des checksums de fichiers : « digest blake3 des octets du fichier », forme `blake3:<hex>` en hexadécimal minuscule (`docs/format-lot-v1.md:132-136`, ADR-0034). L'ADR-0028 définit une forme canonique du JSON, mais pour l'empreinte du modèle seulement, et pour un objet construit par le moteur, pas pour un fichier fourni par un producteur.

Sans définition, deux moteurs peuvent calculer deux digests différents du même manifeste, et un rejeu du même dossier peut être refusé en `409` par un moteur qui vient d'être mis à jour.

### Décision 3 — compteurs du rapport d'attache (ECART)

L'unité est couverte : le tableau de `docs/04-donnees-et-ingestion.md:150-157` est indexé par « situation du document », le glossaire fait du document l'unité de mise à jour, et la suppression d'un `doc_id` inconnu est « ignorée, comptée dans le rapport » (`:157`).

Il y a en revanche une contradiction interne. `docs/04-donnees-et-ingestion.md:182-183` énumère **six** comptes : ajoutés, remplacés, inchangés, métadonnées mises à jour, supprimés, ignorés. L'exemple de `docs/05-api.md:216-217` n'en donne que **quatre** : `added`, `replaced`, `unchanged`, `deleted`. Il mélange aussi les unités : `done` et `total` valent 932, qui est le `counts.passages` du manifeste d'exemple, alors que `replaced` vaut 428 et ne peut être qu'un compte de documents. Un test qui lit le rapport tel que `docs/05` le montre échouerait sur un rapport conforme à `docs/04`.

Le choix n'est pas neutre pour ATTACHER-LOT : les comptes sont le principal résultat observable de l'attache, et ils servent de contrôle après coup sur un mode `replace_all`, où tout document absent est supprimé.

## Décision

### Décision 1 — supprimer la ligne `documents`

**Un document supprimé perd sa ligne `documents` dans la transaction du commit de génération**, qu'il soit nommé dans `deletions` ou absent d'un lot `replace_all`. Un document ré-ajouté ensuite est un document neuf : il n'a ni `doc_hash` ni champs hérités.

Précisions :

- la suppression de la ligne appartient à la même transaction que les pierres tombales, le segment, la ligne `batches` et `meta.generation` ; il n'y a pas de seconde écriture après le point de commit (ADR-0016) ;
- les pierres tombales restent posées sur les passages : c'est elles qui portent l'état de suppression jusqu'à la compaction ;
- pour `explain`, `exists` vaut « la ligne `documents` est présente » et `alive` vaut « le document a au moins un passage vivant ». Les deux booléens restent distincts, et `exists: false` signifie que le `doc_id` n'a jamais existé ou a été supprimé ;
- la table `documents` de `docs/02` garde ses colonnes actuelles : aucune colonne d'état n'est ajoutée.

### Décision 2 — digest des octets lus

**`manifest_blake3` est le digest blake3 des octets de `manifest.json` tels qu'ils ont été lus**, stocké sous la forme `blake3:<hex>`, `<hex>` en hexadécimal minuscule, comme les checksums de fichiers du lot.

Précisions :

- le moteur calcule le digest sur le fichier reçu, avant toute désérialisation ; il ne le recalcule pas depuis une structure en mémoire ;
- un manifeste re-sérialisé à l'identique sur le fond, mais différent sur la forme (espaces, ordre des clés, échappements), donne un `manifest_blake3` différent, donc `409 batch_conflict` si le `batch_id` a déjà été appliqué ;
- l'idempotence d'INV-10 sert à **rejouer le même dossier**, octet pour octet, pas à rattraper un manifeste réécrit ;
- la vérification de la valeur du digest n'est pas un contrôle d'intégrité du manifeste : c'est une identification de contenu, utilisée pour comparer deux tentatives du même `batch_id`.

### Décision 3 — six comptes par document

**Le rapport d'attache donne six comptes, par document et mutuellement exclusifs**, avec les clés `added`, `replaced`, `unchanged`, `metadata_updated`, `deleted` et `ignored`.

Précisions :

- `added` : `doc_id` inconnu, passages ajoutés ;
- `replaced` : `doc_id` connu, `doc_hash` différent, anciens passages en pierres tombales et nouveaux ajoutés ;
- `unchanged` : `doc_id` connu, `doc_hash` identique, champs de document identiques ;
- `metadata_updated` : `doc_id` connu, `doc_hash` identique, champs de document différents ;
- `deleted` : `doc_id` connu nommé dans `deletions`, ou absent d'un lot `replace_all` ;
- `ignored` : suppression d'un `doc_id` inconnu.

Un document compte dans exactement une de ces six catégories. Les comptes de passages restent ceux du manifeste du lot (`counts.passages`) et ne sont pas repris dans le rapport comme s'ils étaient des comptes de documents. L'exemple de `docs/05-api.md:213-218` est corrigé à l'acceptation : six clés, valeurs à l'échelle des documents.

Sont hors du périmètre de cet ADR :

- le format du rapport au-delà de ces six clés (statistiques de recette, durées, génération), qui reste celui de `docs/04` ;
- le comportement de `PATCH /documents`, qui ne passe pas par l'attache ;
- le mode `replace_all` au-delà de la suppression des documents absents, déjà couverte par la fiche.

## Conséquences

- **Positives** :
  - la sémantique d'attache ne dépend plus de l'état résiduel d'une ligne `documents` : un ré-ajout après suppression réussit, et l'idempotence ne peut pas transformer un document supprimé en document définitivement vide ;
  - `exists` et `alive` ont une définition unique, que `@dev` peut tester dès P1 et que P7 réutilisera sans réinterprétation ;
  - `manifest_blake3` devient reproductible et vérifiable : un dossier rejoué donne le même digest, et un manifeste modifié sur le fond ne peut pas passer pour un rejeu ;
  - le rapport a une unité unique, le document, conforme au glossaire et au tableau de `docs/04` ; les six comptes s'excluent, donc leur somme vaut le nombre de documents traités et se contrôle ;
  - aucune colonne ni aucun fichier ne sont ajoutés : la décision 1 et la décision 3 ne touchent pas le schéma, et la décision 2 ne touche pas le format du lot.
- **Négatives / coûts** :
  - la suppression de la ligne `documents` efface les champs du document ; un `explain` sur un `doc_id` supprimé ne peut plus montrer ses anciennes métadonnées, seulement `exists: false`. C'est cohérent avec « document retiré », mais c'est une perte d'information par rapport à un état conservé ;
  - un producteur qui régénère son manifeste à chaque tentative, par exemple avec un ordre de clés non stable, sera refusé en `409` au rejeu alors que le contenu du lot est identique. Il doit conserver le manifeste d'origine ;
  - six clés au lieu de quatre alourdissent l'exemple de `docs/05` et la lecture du rapport ; `metadata_updated` et `unchanged` doivent être distingués par une comparaison des champs, ce que l'attache fait de toute façon pour la mise à jour ;
  - trois documents de conception (`docs/02`, `docs/04`, `docs/05`) doivent être mis à jour en cohérence, sinon l'écart persiste sous une autre forme.
- **À surveiller** :
  - toute demande d'un producteur qui ne peut pas conserver un manifeste stable, ou qui veut rejouer un lot après re-sérialisation : elle rouvrirait la décision 2 ;
  - un besoin d'exploitation de retrouver les métadonnées d'un document supprimé sans compaction, ou de compter les passages morts par document : il rouvrirait la décision 1 ;
  - un besoin de volumes par passage dans le rapport, ou une consumer qui lit déjà quatre clés : il rouvrirait la décision 3 ;
  - la première compaction après une suppression, qui doit produire un SQLite sans ligne `documents` orpheline (INV-09, équivalence d'un échantillon de requêtes).

## Application

Après acceptation seulement, et dans un commit qui cite `ADR-0041` :

- `docs/02-architecture.md:92` : préciser que la ligne `documents` n'existe que pour un document vivant, et que sa suppression appartient au commit de génération ;
- `docs/04-donnees-et-ingestion.md:148-162` : ligne « Dans `deletions` » (suppression de la ligne `documents`), définition de `manifest_blake3`, et section « Le rapport d'attache » (`:182-186`) avec les six comptes et leur définition ;
- `docs/05-api.md:213-218` : exemple de rapport corrigé, six clés, valeurs à l'échelle des documents ;
- `docs/suivi.md`, section « Écarts à la conception » : ligne citant `ADR-0041` pour la décision 3, à ajouter par l'orchestrateur ;
- code : rien à modifier avant la décision, puisque ATTACHER-LOT n'est pas implémentée.

Les tests préparés par `@testeur` supposent déjà les options recommandées des décisions 2 et 3. Ils n'imposent rien sur la décision 1 : un test de ré-ajout après suppression devra être ajouté si l'option A est acceptée, et un test d'état de document si l'option B l'est.

**ATTACHER-LOT reste bloquée jusqu'à la décision.** Aucun des trois points ne peut être tranché par une lecture de la conception actuelle.

## Alternatives écartées

### Décision 1

| Alternative | Pourquoi écartée |
|---|---|
| **B.** Garder la ligne `documents` jusqu'à la compaction, avec un état de document. | Il faut ajouter une colonne `deleted_since_gen` à `docs/02`, absente du schéma actuel, ou adopter la règle « vivant = au moins un passage vivant » et l'appliquer partout : comptes, export, `explain`. Il faut aussi une règle de ré-ajout explicite, sinon le piège du `doc_hash` identique subsiste : la ligne garde son hash, et l'`upsert` suivant ignore les passages. Le coût est un état de plus à maintenir et à compacter, pour une information que la compaction jette de toute façon. |

### Décision 2

| Alternative | Pourquoi écartée |
|---|---|
| **B.** Calculer le digest sur une forme canonique du JSON, à la manière de l'ADR-0028 (UTF-8, compact, clés triées). | La forme canonique de l'ADR-0028 a été définie pour un objet construit par le moteur, pas pour un document fourni par un producteur. L'étendre au manifeste oblige à définir la forme des nombres (précision, exposant), des échappements et des tableaux, et à imposer cette définition à tout producteur, y compris tiers. Le gain — tolérer une re-sérialisation — n'est pas nécessaire : l'idempotence sert à rejouer le même dossier, pas à absorber une réécriture. |

### Décision 3

| Alternative | Pourquoi écartée |
|---|---|
| **C.** Garder quatre clés (`added`, `replaced`, `unchanged`, `deleted`) comme dans l'exemple de `docs/05`, et corriger `docs/04`. | Il faudrait alors ranger `metadata_updated` et `ignored` dans une des quatre clés. « Inchangé » deviendrait faux pour un document dont seuls les champs changent, et une suppression inconnue serait comptée comme une suppression réelle. Le rapport perdrait la trace des deux cas que `docs/04` distingue explicitement (`:153`, `:157`), et le contrôle de la somme des comptes ne serait plus possible. |

## Références

- `docs/04-donnees-et-ingestion.md` : `:144-146` (modes), `:148-157` (sémantique d'attache), `:160-162` (idempotence), `:171` (contrôle « chaque document a au moins un passage »), `:182-186` (rapport d'attache).
- `docs/02-architecture.md` : `:88-92` (tables SQLite, dont `batches` et `documents`), `:105-108` (transaction et point de commit), `:153-163` (compaction).
- `docs/05-api.md` : `:182-186` (`explain`, `exists` et `alive`), `:210-218` (exemple de job d'attache).
- `docs/format-lot-v1.md` : `:132-136` (checksums `blake3:<hex>`), `:246` (contrôle de forme des checksums).
- `docs/08-invariants-et-tests.md:18` (INV-10), et INV-03, INV-09.
- `docs/phases/P1-stockage-et-recherche-exacte.md:104-122` : fiche ATTACHER-LOT ; `docs/phases/P3-cycle-de-vie.md` : export et compaction.
- ADR-0016 : le commit SQLite est l'unique point de commit.
- ADR-0028 : forme canonique de l'empreinte du modèle.
- ADR-0034 : dépendance Python `blake3` pour les checksums des lots.
- ADR-0039 et ADR-0040 : règles du format de lot v1, laissées hors de cet ADR.
