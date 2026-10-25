# ADR-0061 — Manifeste d'un lot exporté

- **Statut** : Proposé
- **Date** : 2026-10-25
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, sur demande de l'orchestrateur, pour la tâche EXPORTER-LOT (phase P3), après la relecture du tour 1 et le verdict `NON-COUVERT` de `@architecte`
- **Remplace** : —

## Contexte

### L'export et le lot qu'il écrit

La fiche `EXPORTER-LOT` (`docs/phases/P3-cycle-de-vie.md:101-112`) demande `crible-store::export` et la commande `crible export` : un lot `replace_all` des données vivantes à la génération G, en parties d'au plus un million de passages, écrit dans `exchange/outbox/<index>-g<G>/`, lu sur l'instantané pour ne pas bloquer l'écrivain, sans aucun iid (**INV-01**).

`docs/04-donnees-et-ingestion.md:296-304` décrit l'export comme un « lot `replace_all` des données vivantes à la génération G » vers `exchange/outbox/<index>-g<G>/`, et sa restauration comme « attacher le lot dans un index vide ».

Ce lot suit le **format public** (ADR-0012) décrit par `docs/format-lot-v1.md` :

- `batch_id` est aussi le nom du dossier ; c'est un segment de chemin unique (`docs/format-lot-v1.md:44-46`) ;
- le manifeste porte des champs obligatoires, dont `index`, `created_at`, `producer`, `model`, `chunker`, `template`, `counts` et `files` (`docs/format-lot-v1.md:76-91`), et refuse toute clé inconnue (`docs/format-lot-v1.md:93`, `crates/crible-core/src/batch.rs:164`) ;
- `index` est le « nom de l'index cible ; doit correspondre à l'index d'attache » (`docs/format-lot-v1.md:81`), et un lot destiné à un autre index est refusé en `400 invalid_request` (`docs/format-lot-v1.md:311`).

### Ce que l'implémentation actuelle écrit

L'implémentation, non commitée, remplit le manifeste ainsi :

| Champ | Valeur |
|---|---|
| `producer.name` | `crible` |
| `producer.version` | `CARGO_PKG_VERSION` |
| `producer.host` | `crible-export` |
| `chunker`, `template` | `{ "name": "none", "version": "1" }` |
| `created_at` | date RFC 3339 de l'export, lue dans SQLite (`crates/crible-store/src/export.rs:484-496`) |
| `index` | nom de l'index source |
| `batch_id`, dossier | `<index>-g<G>` |

Deux comportements l'accompagnent : un dossier `exchange/outbox/<index>-g<G>/` déjà présent est refusé en `invalid_request` (`crates/crible-store/src/export.rs:178-186`, `232-238`) ; une course de génération au démarrage de l'export est relancée sur un instantané neuf, au plus trois fois, puis rend `overloaded` (`crates/crible-store/src/export.rs:107-114`, `221-281`).

### Ce que la conception ne tranche pas

La relecture du tour 1 a relevé un point bloquant sur le manifeste de l'export ; `@architecte` a rendu le verdict `NON-COUVERT`. Cinq points restent sans réponse dans `docs/01` à `docs/08` et dans les ADR acceptés :

1. **`chunker` et `template`.** Ils sont obligatoires (`docs/format-lot-v1.md:88-89`), le producteur les déclare et le moteur « trace le découpeur de chaque lot » (ADR-0010 ; `docs/04-donnees-et-ingestion.md:313`). L'export n'a pas de producteur qui découpe : il recopie des entrées encodeur déjà stockées. Rien ne dit ce qu'un lot écrit par le moteur doit déclarer.
2. **`created_at` et le ré-export de la même génération.** Un horodatage rend le manifeste non reproductible sous un `batch_id` stable. Deux exports de la même G, attachés à un même index cible, produisent deux manifestes différents, donc `409 batch_conflict` (`docs/format-lot-v1.md:317-319`). L'écrasement d'un dossier `outbox` existant n'est décrit nulle part.
3. **Le champ `index`.** Il vaut l'index source, alors que `docs/format-lot-v1.md:81` exige qu'il corresponde à l'index d'attache, que `docs/04:302` dit « attacher le lot dans un index vide » et que la fiche `VERIFIER-ALLER-RETOUR-EXPORT` crée « un index vide de même config » (`docs/phases/P3-cycle-de-vie.md:114-119`) sans dire « de même nom ».
4. **`producer.version`.** `CARGO_PKG_VERSION` est conforme à `docs/format-lot-v1.md:85` (« version du programme »), mais diffère du choix d'ADR-0055 pour le lot en ligne, qui porte la version du format pour rester déterministe (`docs/format-lot-v1.md:184`, ADR-0055 décision 2).
5. **Le code d'une course de génération.** L'export rend `overloaded` (503), cause absente de la table de `docs/05-api.md:42`. Le nombre de tentatives est une mitigation interne, mais le code rendu engage le contrat.

### Les faits qui pèsent

- **La règle `none`/`1` n'est prescrite que pour le lot en ligne.** Elle y est écrite comme propre à ce lot, pour qu'un rejeu ne produise pas de `409` (`docs/format-lot-v1.md:171-194`, ADR-0055 décision 2). Ce n'est pas une règle générale du format.
- **Aucun contrôle ni invariant ne lit `chunker` et `template`.** Les contrôles 1 à 9 (`docs/format-lot-v1.md:279-305`, `docs/04:182-200`) ne les mentionnent pas ; les types sont opaques (`crates/crible-core/src/batch.rs:123-144`). En v1, `deny_unknown_fields` sur `BatchManifest` (`crates/crible-core/src/batch.rs:164`) interdit une clé nouvelle.
- **La provenance du découpage reste dans l'index source**, dans `batches.manifest` (`docs/02-architecture.md:92`) et `segments.batch_id` (`docs/02-architecture.md:93`). Mais une compaction réécrit `segments.batch_id` en `compaction-e<E>-e<E+1>` (note de COMPACTER-INDEX dans `docs/suivi.md`) et recopie `batches` à l'identique, `applied_at` d'origine compris (`crates/crible-store/src/records.rs:145`, `560-575`) : après une compaction, l'origine par document n'est plus reconstructible. La table `documents` ne porte aucune colonne de lot.
- **Aucun instant de commit de génération n'est stocké.** `batches.applied_at` est l'instant de l'**attache** d'un lot, écrit par SQLite (`crates/crible-store/src/records.rs:700-713`) ; une génération peut exister sans ligne `batches` (patch, ADR-0060:161-175 ; compaction). « L'instant du commit de G » n'est donc pas disponible.
- **INV-10 porte sur un lot déjà appliqué.** Même `batch_id` et même manifeste : rien ne se fait, le rapport d'origine est rendu ; même `batch_id` et manifeste différent : `409 batch_conflict` (`docs/format-lot-v1.md:317-319`, `docs/08-invariants-et-tests.md:18`). Réattacher **le même dossier** est donc toujours sans effet, quel que soit `created_at`.
- **`created_at` n'est contrôlé que pour sa non-vacuité** (`docs/format-lot-v1.md:83`). En revanche, la graine de l'échantillon de recette dérive du `batch_id` (`docs/04:193`) : un `batch_id` qui porterait un digest de contenu changerait l'échantillon d'un export à l'autre.
- **`outbox` est hors de l'index.** La reprise nettoie `segments/tmp-*`, les `tmp-*` du dossier de l'index et `manifests/` (`docs/02-architecture.md:132-133`), pas `<exchange>/outbox/`. L'export écrit sous `outbox/.tmp-<batch_id>-<pid>-<n>-<nanos>` (`crates/crible-store/src/export.rs:105`, `307-316`) : une panne y laisse un dossier `.tmp-*` que personne ne supprime automatiquement.
- **La recette de la ré-attache ne lit ni le découpage ni la date.** Le contrôle 9 tire un échantillon selon une graine dérivée du `batch_id`, ré-encode les passages en ONNX fp32 avec `prompt_document`, compare les cosinus aux vecteurs du lot et refuse sous 0,999 (`docs/04:192-196`, ADR-0052, ADR-0054). Ni `chunker`, ni `template`, ni `created_at`, ni `index`, ni `producer.version` n'y entrent.

### Points déjà conformes, rappelés pour mémoire

- `producer.name = "crible"` et `producer.host = "crible-export"` : `docs/format-lot-v1.md:84-86` et ADR-0036 laissent le producteur se nommer lui-même, et aucun contrôle ne lit la valeur.
- `mode = replace_all`, sans fichier `deletions` et avec `counts.deletions = 0` (`docs/format-lot-v1.md:128-130`).
- `model` reprend la configuration résolue de l'index, avec `vector_dtype = reference_precision` (`docs/format-lot-v1.md:111-121`, ADR-0005).
- Parties d'au plus un million de passages (`docs/04:301`) et aucun iid dans les fichiers (**INV-01**).

## Décisions

Aucune décision n'est prise ici : chacune est soumise au propriétaire. Les cinq décisions sont **séparables**, comme celles d'ADR-0055 : en rejeter une laisse les quatre autres tenir, et la tâche reprend sur le périmètre accepté. Pour chacune sont donnés les options, leurs conséquences sur `format_version`, sur INV-08 (aller-retour export puis import, `docs/08-invariants-et-tests.md:16`), sur la recette à la ré-attache et sur INV-10, puis une recommandation.

### Décision 1 — `chunker` et `template` d'un lot écrit par le moteur sans découpage

- **(a) `none`/`1`, au sens du lot en ligne : « aucun découpage par le producteur de ce lot ».** C'est la recommandation de `@architecte`.
- **(b) recopier les valeurs des lots d'origine quand tous les documents vivants s'accordent, sinon `none`.**
- **(c) porter la provenance par document**, donc changer de format.

**Conséquences de (a).** `format_version` reste 1, aucune clé nouvelle, aucun contrôle nouveau. Le manifeste dit la vérité du lot exporté — il n'a subi ni découpage ni recomposition par son producteur — sans prétendre décrire l'histoire des documents. La provenance d'origine reste dans l'index source (`batches.manifest`, `segments.batch_id`) et n'est pas transportée : c'est une perte assumée, et la seule que l'export introduit. INV-08 est inchangé (l'aller-retour compare des résultats de recherche). La recette est inchangée (elle ne lit pas ces champs). INV-10 est inchangé (la valeur est déterministe).

**Conséquences de (b).** `format_version` resterait 1, mais la règle « quand tous les documents vivants s'accordent » n'est pas applicable telle quelle : après une compaction, `segments.batch_id` ne nomme plus un lot d'origine, et `documents` n'a pas de colonne de lot. La règle dégénère en « tous les lots de l'histoire de l'index déclaraient la même valeur », ce qui n'est pas ce qu'elle annonce. Le manifeste dépendrait en plus de l'histoire : une compaction changerait les octets d'un ré-export de la même génération. INV-08, la recette et INV-10 restent inchangés ; le code et la documentation augmentent pour une information que rien ne lit.

**Conséquences de (c).** Il faudrait une clé de manifeste ou une colonne Parquet par document. En v1, `deny_unknown_fields` refuse la clé (`crates/crible-core/src/batch.rs:164`) et `docs/format-lot-v1.md:217` réserve les colonnes Parquet aux champs déclarés dans l'index : c'est un `format_version` 2. Un moteur v1 refuserait alors le lot (`docs/format-lot-v1.md:311`), et l'export perdrait la portabilité entre versions que lui prête ADR-0012:31. INV-08, la recette et INV-10 resteraient inchangés sur le fond, mais l'aller-retour exigerait un moteur v2 des deux côtés.

**Recommandation : (a)**, celle de `@architecte`. Le format n'a pas de notion de provenance de découpage à l'échelle du document, et l'inventer pour un export coûterait une version de format, contre aucun contrôle et aucun invariant.

### Décision 2 — `created_at` et ré-export de la même génération

- **(i) horodatage de l'export et refus d'écraser un export existant** (comportement actuel).
- **(ii) valeur déterministe** : l'instant du commit de G s'il est stocké, ou une sentinelle comme pour le lot en ligne.
- **(iii) `batch_id` portant un digest de contenu.**

**Le point de fait qui tranche une partie de (ii).** L'instant du commit de G n'est **pas** stocké : `batches.applied_at` est l'instant de l'attache (`crates/crible-store/src/records.rs:700-713`), recopié tel quel par la compaction (`:145`), et une génération peut naître d'un patch ou d'une compaction, sans ligne `batches` (ADR-0060:161-175). Il ne reste donc, pour (ii), que la sentinelle `1970-01-01T00:00:00Z`, déjà employée par le lot en ligne (`docs/format-lot-v1.md:188`, ADR-0055:42-46).

**Conséquences de (i).** `format_version` reste 1. Le manifeste n'est pas reproductible : exporter deux fois la même G donne deux manifestes, et les attacher tous deux au même index cible donne `409 batch_conflict` — ce qui est exact, puisque ce sont deux lots distincts. Réattacher **le même dossier** reste un no-op (INV-10). INV-08 est inchangé : l'aller-retour attache l'export une fois. La recette est inchangée. Le coût est une règle d'écriture à documenter (le refus d'écraser, absent de `docs/04`) et un code d'erreur à nommer : aucun code de `docs/05:31-44` ne décrit une destination déjà occupée, et `invalid_request` (400) en est le plus proche ; sa description (« corps invalide, paramètre hors bornes ») doit alors mentionner ce cas.

**Conséquences de (ii).** `format_version` reste 1 (la sentinelle est une date RFC 3339 valide et `created_at` n'est contrôlé que pour sa non-vacuité, `docs/format-lot-v1.md:83`). Le manifeste devient déterministe **à condition** que `producer.version` le soit aussi : la décision 2 et la décision 4 sont liées. INV-10 gagne le cas « deux exports de la même G, attachés au même index cible, dont le second ne fait rien », mais seulement tant que les octets Parquet sont identiques — or les checksums dépendent des versions des crates `arrow` et `parquet`, réserve déjà consignée pour le lot en ligne (ADR-0055:92). INV-08 et la recette sont inchangés. Le coût est la perte de la date d'export, qu'aucun autre champ ne porte (ni le dossier, ni le rapport, qui donnent G).

**Conséquences de (iii).** `batch_id` devrait contenir un digest de tout le lot, donc être connu avant d'écrire, ou renommer après écriture. Cela contredit la forme `<index>-g<G>` de `docs/04:301`, rend le dossier imprévisible pour l'opérateur, et ne suffit pas à rendre le manifeste identique : `batch_id` figure dans le manifeste, donc `created_at` devrait **aussi** être déterministe. En prime, la graine de la recette dérive du `batch_id` (`docs/04:193`) : l'échantillon changerait d'un export à l'autre. `format_version` resterait 1 ; INV-08 et INV-10 seraient inchangés sur le fond, au prix d'un nommage plus complexe.

**Recommandation : (i), pour la simplicité.** L'export produit une **photographie de données**, pas un artefact de construction : aucune exigence du projet ne demande que deux exports d'une même génération soient interchangeables, et INV-10 porte déjà sur le lot tel qu'il est écrit, pas sur une ré-exécution de la commande. Une date vraie vaut mieux qu'une sentinelle ; le refus d'écraser protège un dossier qu'une attache peut être en train de lire. Choisir (ii) reste défendable si le propriétaire veut un manifeste reproductible, mais il faut alors l'accepter avec la décision 4 option (b), et documenter que la reproductibilité s'arrête aux changements de `arrow` et `parquet`.

### Décision 3 — champ `index` du manifeste exporté

- **(a) garder le nom de l'index source** ; la restauration se fait dans un index de même nom, et `docs/04` l'écrit.
- **(b) permettre de renommer**, à l'export (option de commande) ou à l'attache.

**Conséquences de (a).** `format_version` reste 1, le manifeste reste la description exacte du lot, et l'attache fonctionne sans règle nouvelle : `docs/format-lot-v1.md:81` et `:311` s'appliquent tels quels. Il faut écrire dans `docs/04`, à la ligne 302, que la restauration crée un index **vide, de même nom et de même configuration**, et préciser que `VERIFIER-ALLER-RETOUR-EXPORT` créera cet index dans une seconde racine de données — la fiche dit « de même config » et omet le nom, c'est le seul mot à ajouter. INV-08 est inchangé : l'aller-retour se fait à nom constant, et c'est ce qu'il doit prouver. La recette et INV-10 sont inchangés.

**Conséquences de (b).** Renommer à l'export fait porter au manifeste un nom d'index cible que le dossier ne porte pas (`<index source>-g<G>`), et rend le lot inattachable à un index d'un autre nom. Renommer à l'attache est plus coûteux : `manifest_blake3` est le digest des **octets lus** (`docs/02-architecture.md:99`, ADR-0041), donc réécrire le manifeste à l'attache ferait stocker un digest qui ne correspond plus aux octets, ou changerait ce que « même manifeste » signifie pour INV-10 et le `409`. `format_version` resterait 1 ; INV-08 demanderait de définir quel nom compare l'aller-retour ; la recette serait inchangée.

**Recommandation : (a).** C'est la seule option qui laisse `index` dire ce que le format lui fait dire. Un besoin de migration sous un autre nom relèvera d'un nouvel ADR, avec la question du digest du manifeste traitée pour elle-même.

### Décision 4 — `producer.version`

- **(a) `CARGO_PKG_VERSION`** (comportement actuel, conforme à `docs/format-lot-v1.md:85`).
- **(b) la version du format du lot exporté**, comme le lot en ligne (`docs/format-lot-v1.md:184`, ADR-0055:38 et 44).

**Conséquences.** Ni l'une ni l'autre ne touche `format_version`, INV-08, la recette ou INV-10 : aucun contrôle ne lit `producer.version`, et la ré-attache du même dossier reste un no-op dans les deux cas. (a) dit quel binaire Crible a écrit le lot — utile quand on suspecte l'écrivain — mais change le manifeste à chaque publication, donc rend un ré-export après mise à jour différent sous le même `batch_id`. (b) rend le manifeste stable, mais perd la version du binaire et introduit une constante de version de format d'export à maintenir.

**Recommandation : (a), en cohérence avec la décision 2 option (i).** Le déterminisme n'étant pas recherché pour l'export, la version du programme est l'information la plus utile. Si le propriétaire retient la décision 2 option (ii), alors (b) devient nécessaire : sans elle, le déterminisme tombe à la première publication.

### Décision 5 — code d'une course de génération pendant l'export

- **(a) ajouter la cause à `overloaded`** dans `docs/05-api.md:42`.
- **(b) rendre `internal`** (500).
- **(c) introduire un code nouveau**, par exemple `busy`.

**Conséquences de (a).** Aucun changement de format, aucun invariant touché : INV-08, la recette et INV-10 sont inchangés. Le client reçoit un code qu'il traite déjà comme réessayable, et `Retry-After` s'applique. La table de `docs/05` gagne une cause, pas un code : « course de génération pendant un export (l'index a commité entre l'instantané et la première lecture) ». Le nombre de tentatives (trois, `crates/crible-store/src/export.rs:107-114`) reste une mitigation interne et ne doit pas entrer dans le contrat, sous peine de figer un réglage d'implémentation.

**Conséquences de (b) et de (c).** `internal` dit que l'état est incohérent et fait lever un opérateur pour une course transitoire : c'est faux et coûteux. Un code nouveau ajoute une entrée à une table volontairement courte (`docs/05:31-44`), pour une seule cause, et oblige clients et analytics à le connaître.

**Recommandation : (a).** C'est le seul code de la table qui dise « pas maintenant, réessayez », et la course est bien un défaut de disponibilité momentanée, pas une erreur du client ni un état corrompu.

## Résumé des recommandations

| Décision | Recommandation | Effet sur le code déjà écrit |
|---|---|---|
| 1 | (a) `none`/`1`, `format_version` 1 | aucun |
| 2 | (i) horodatage de l'export, refus d'écraser un dossier existant | aucun (documenter le refus et le code) |
| 3 | (a) nom de l'index source, restauration dans un index de même nom | aucun |
| 4 | (a) `CARGO_PKG_VERSION`, cohérent avec 2 (i) | aucun |
| 5 | (a) cause ajoutée à `overloaded` | aucun (documentation seule) |

Le jeu recommandé décrit **exactement** ce que fait l'implémentation actuelle. S'il est accepté, l'acceptation ne demande donc aucune modification de code de production : seulement les documents de conception, et le renvoi des commentaires de `crates/crible-store/src/export.rs` vers le présent ADR.

## Conséquences

- **Positives** :
  - les cinq points de contrat du manifeste exporté qui n'étaient tranchés nulle part le sont, sans changer `format_version` ni ajouter de dépendance ;
  - l'export reste un lot du format public, portable entre versions de Crible (ADR-0012) ;
  - le jeu recommandé est déjà implémenté : la décision coûte de la documentation, pas une réécriture ;
  - INV-08, la recette et INV-10 sont explicitement hors de cause pour les cinq décisions, ce qui borne la portée de l'acceptation.
- **Négatives / coûts** :
  - l'export ne transporte pas la provenance du découpage : un lot restauré ne dit pas quel découpeur avait produit ses passages, et cette information n'est plus reconstructible après une compaction ;
  - deux exports de la même génération ne sont pas interchangeables pour un index cible qui en a déjà reçu un (`409 batch_conflict`) ;
  - `outbox` gagne une règle d'écriture (refus d'écraser) et un résidu possible : un dossier `.tmp-*` laissé par une panne n'est nettoyé ni par la reprise (`docs/02:132-133` ne couvre que l'index) ni par une commande ; il faut le dire dans `docs/04` ;
  - `overloaded` est employé pour une cause de plus, sans que sa description de `docs/05:42` la couvre aujourd'hui.
- **À surveiller** :
  - un besoin réel de provenance du découpage dans un export, par exemple pour diagnostiquer une recherche dégradée après restauration, rouvrirait la décision 1 ;
  - un besoin d'attacher un export sous un autre nom rouvrirait la décision 3 ;
  - une génération qui commit sans cesse rendrait la course de la décision 5 structurelle : chaque tentative réécrit le lot entier, donc un index très actif pourrait faire échouer un gros export en `overloaded` sans que la machine soit surchargée. À mesurer si le cas se présente ;
  - si `arrow` ou `parquet` change, un ré-export de la même génération produit d'autres checksums : la remarque déjà consignée pour le lot en ligne (ADR-0055:92) vaut aussi pour l'export, surtout si la décision 2 option (ii) était retenue.

### Documents à modifier après acceptation

| Document | Modification, selon les décisions retenues |
|---|---|
| `docs/04-donnees-et-ingestion.md` | section « Export, instantanés, restauration » : règles du manifeste exporté (1, 4), refus d'écraser et code rendu (2), restauration dans un index vide de **même nom et même configuration** (3), résidus `.tmp-*` de `outbox` |
| `docs/05-api.md` | `overloaded` : ajouter la course de génération pendant un export (5), avec `Retry-After` ; `invalid_request` : mentionner la destination d'export déjà occupée (2) |
| `docs/format-lot-v1.md` | nouvelle sous-section « Lot d'export », à côté de « Lot interne du mode en ligne » (`:171-194`) : `chunker`/`template` `none`/`1` et leur sens (1), `producer` (4), `created_at` = date de l'export (2), `index` = index source (3), et le fait que le manifeste d'export n'a pas à être déterministe (2) |
| `docs/02-architecture.md` | seulement si la décision 2 retient (ii) : noter qu'aucun instant de génération n'est stocké et que `batches.applied_at` est l'instant de l'attache |
| `docs/phases/P3-cycle-de-vie.md` | seulement si le propriétaire veut figer le nom de l'index cible dans la fiche `VERIFIER-ALLER-RETOUR-EXPORT` (décision 3) |

### Effet sur la tâche EXPORTER-LOT

La tâche est arrêtée sur ce point, à la relecture du tour 1 ; `@relecteur` a rendu un point bloquant et `@architecte` le verdict `NON-COUVERT`. Elle reste bloquée tant que le propriétaire n'a pas statué, et c'est l'orchestrateur qui inscrit l'écart dans `docs/suivi.md` et passe son statut à `BLOQUE`.

- **Si le jeu recommandé est accepté** : l'implémentation déjà écrite est conforme, aucun code de production ne change ; il reste à mettre à jour les trois documents listés ci-dessus, à renvoyer les commentaires de `crates/crible-store/src/export.rs` vers ADR-0061, puis à repasser la relecture.
- **Si une autre option est acceptée** : la décision 1 (b) ou (c), la décision 2 (ii) ou (iii), la décision 3 (b) ou la décision 4 (b) changent le manifeste et parfois le dossier écrit, donc `crates/crible-store/src/export.rs` et ses tests ; la décision 5 (b) ou (c) change le code d'erreur rendu. La tâche reprend alors sur le périmètre accepté, avec un nouveau tour de relecture.

## Alternatives écartées

| Décision | Alternative | Pourquoi écartée |
|---|---|---|
| 1 | (b) recopier les valeurs des lots d'origine | L'origine par document n'est plus reconstructible après une compaction (`segments.batch_id` réécrit) ; la règle « tous s'accordent » dégénère, et le manifeste d'un ré-export dépendrait de l'histoire de l'index |
| 1 | (c) provenance par document, format v2 | Une version de format pour une information qu'aucun contrôle ne lit, et la perte de la portabilité vers un moteur v1 (ADR-0012) |
| 2 | (ii) instant du commit de G | Cet instant n'est stocké nulle part : `batches.applied_at` est l'instant de l'attache, et une génération peut exister sans ligne `batches` |
| 2 | (ii) sentinelle `1970-01-01T00:00:00Z` | Rend le manifeste stable au prix de la date d'export, et le déterminisme s'arrête de toute façon aux changements de `arrow` et `parquet` (ADR-0055:92) |
| 2 | (iii) `batch_id` avec digest de contenu | Contredit la forme `<index>-g<G>` de `docs/04:301`, rend le dossier imprévisible, et ne dispense pas de rendre `created_at` déterministe ; change aussi la graine de la recette (`docs/04:193`) |
| 3 | (b) renommer à l'export | Le manifeste nommerait un index cible que le dossier ne porte pas, et le lot ne serait plus attachable sous un autre nom |
| 3 | (b) renommer à l'attache | `manifest_blake3` est le digest des octets lus (ADR-0041) : réécrire le manifeste ferait mentir le digest ou changerait le sens de « même manifeste » pour INV-10 et le `409` |
| 4 | (b) version du format d'export | Cohérente seulement si la décision 2 retient (ii) ; sinon elle perd la version du binaire sans rien rendre stable |
| 5 | (b) `internal` | Annonce un état corrompu pour une course transitoire, et réveille un opérateur pour rien |
| 5 | (c) code nouveau | Un code de plus dans une table courte, pour une seule cause, à faire connaître des clients et des analytics |

## Références

- Fiche `EXPORTER-LOT` et fiche `VERIFIER-ALLER-RETOUR-EXPORT` : `docs/phases/P3-cycle-de-vie.md:101-119`
- `docs/04-donnees-et-ingestion.md` : contrôles et recette (`:182-200`), export et restauration (`:296-304`), découpage et rôles (`:306-315`)
- `docs/05-api.md` (`:31-44`, codes d'erreur et `overloaded` ; `:65`, route d'export)
- `docs/format-lot-v1.md` : `batch_id` (`:44-46`), manifeste et champs (`:76-97`), modes (`:123-130`), contrôles (`:279-305`), erreurs et INV-10 (`:307-319`), lot interne du mode en ligne (`:171-194`)
- `docs/02-architecture.md` : tables `batches` et `segments` (`:92-93`), reprise (`:132-133`)
- `docs/08-invariants-et-tests.md` : INV-01 (`:9`), INV-08 (`:16`), INV-10 (`:18`)
- `crates/crible-store/src/export.rs`, `crates/crible-core/src/batch.rs`, `crates/crible-store/src/records.rs`
- ADR-0005 (précision de référence), ADR-0010 (le découpage reste chez le producteur), ADR-0012 (format de lot public), ADR-0036 (`producer.host`), ADR-0041 (sémantique d'attache, digest du manifeste), ADR-0052 et ADR-0054 (recette), ADR-0055 (lot en ligne et manifeste déterministe), ADR-0060 (patch sans ligne `batches`)
