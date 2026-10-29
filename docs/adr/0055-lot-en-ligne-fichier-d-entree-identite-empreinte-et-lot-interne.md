# ADR-0055 — Lot en ligne : fichier d'entrée, identité, empreinte des documents et lot interne

- **Statut** : Accepté (décision du propriétaire le 2026-10-24) — décision 4, nettoyage des lots résiduels : remplacée par ADR-0056
- **Date** : 2026-10-24 (proposition et acceptation)
- **Décideurs** : propriétaire
- **Décision** : accepté le 2026-10-24 par le propriétaire, tel que proposé : option (a) sur les décisions 1, 2 et 4, option (b) sur la décision 3, donc fichier JSONL d'un document par ligne en `upsert`, `batch_id` = `online-<blake3 des octets du fichier>` et manifeste déterministe, `doc_hash` absent = blake3 du JSON canonique des passages, lot interne dans `<data>/tmp/online-<batch_id>/` avec `std::fs` et nettoyé par la reprise, `replace_all` refusé en ligne.
- **Proposé par** : agent greffier, sur demande de l'orchestrateur, à la suite d'une `QUESTION-CONCEPTION` de `@dev` sur la tâche ENCODER-DOCUMENTS-EN-LIGNE (phase P2), instruite par `@architecte` le 2026-10-24
- **Remplace** : —

## Contexte

La tâche ENCODER-DOCUMENTS-EN-LIGNE (`docs/phases/P2-encodeur-et-recette.md`) doit livrer `crible-store::online` : à partir d'une liste de documents au format JSON de l'API, encoder en fp32 les passages qui n'ont pas de vecteur, puis construire un **lot interne** qui suit le même chemin d'attache qu'un lot déposé. Elle doit aussi livrer la commande `crible ingest --data … --index … documents.jsonl`. La route HTTP correspondante arrive en P6 (`docs/05-api.md:57`).

`docs/05-api.md:74-90` décrit le **corps HTTP** du lot en ligne : `batch_id`, `mode`, `documents` (avec `doc_id`, `doc_hash` facultatif, `fields`, `passages`), `deletions` ; il précise que `doc_hash` absent vaut le blake3 du contenu des passages, et plafonne une requête à 10 000 passages. `docs/04-donnees-et-ingestion.md` décrit le mode en ligne (« le serveur construit ensuite un lot interne et suit exactement le même chemin qu'un lot produit ailleurs ») et la sémantique d'attache (INV-10 : un `batch_id` déjà appliqué ne fait rien ; même `batch_id` et `manifest_blake3` différent → `409 batch_conflict`).

La tâche est arrêtée **avant implémentation** : quatre points ne sont tranchés ni par `docs/01` à `08` ni par un ADR `Accepté`. Deux sont **NON-COUVERT** (le fichier d'entrée de la CLI, l'identité et le manifeste du lot interne) et deux sont des **écarts** (les octets du `doc_hash`, l'emplacement et le mode du lot interne).

Cet ADR porte **quatre décisions séparables**. Chacune peut être rejetée sans les autres : les trois restantes tiennent, et la tâche reprend sur le périmètre accepté. Dans chaque décision, l'option recommandée est nommée (a) et les alternatives écartées sont listées avec leur raison.

## Décision

### Décision 1 — forme du fichier d'entrée de `crible ingest` (NON-COUVERT)

**Option (a) recommandée.** Le fichier d'entrée est du **JSONL** : un objet document par ligne, dont les champs sont exactement ceux d'un élément de `documents` du corps HTTP de `docs/05-api.md:81-83` (`doc_id`, `doc_hash` facultatif, `fields`, `passages` avec `text`, `fields`, `vector` facultatif).

- L'enveloppe est **implicite** : `mode` vaut `upsert`, et `deletions` n'existe pas dans le fichier (une suppression passe par un lot déposé).
- La désérialisation utilise le **même type** que celui que consommera la route P6. Un seul type décrit un document en ligne, pour la CLI et pour l'API.

### Décision 2 — identité et manifeste du lot interne (NON-COUVERT)

**Option (a) recommandée.** `batch_id` = `online-<blake3 des octets du fichier>`. L'identité vient du **contenu**, dans l'esprit de la décision 2 d'ADR-0041 (le digest identifie un contenu, pas une exécution) : rejouer le même fichier ne fait rien (INV-10). Deux fichiers différents qui portent les mêmes documents donnent deux lots ; le second est entièrement `unchanged`, ce qui est le comportement correct.

Le manifeste du lot interne est **déterministe**, pour que le rejeu ne produise pas de `409 batch_conflict` :

| Champ | Valeur |
|---|---|
| `producer.name` | `crible-online` |
| `producer.version` | `1` — version du **format du lot en ligne**, pas du binaire |
| `producer.host` | `crible-online` |
| `chunker` | `{ "name": "none", "version": "1" }` |
| `template` | `{ "name": "none", "version": "1" }` |
| `created_at` | `1970-01-01T00:00:00Z`, sentinelle documentée |

`producer.version` porte la version du format, parce que la version du binaire change à chaque mise à jour de Crible : un rejeu après mise à jour produirait un manifeste différent, donc un `409`. Le champ reste renseigné (il est obligatoire dans `docs/format-lot-v1.md:85`), mais sa valeur ne dépend pas du binaire.

`created_at` est une sentinelle parce qu'aucun contrôle ni rapport ne la lit (`docs/format-lot-v1.md:83` ne vérifie que sa non-vacuité) et que le mtime n'est pas stable d'une copie à l'autre : le manifeste doit dépendre du seul contenu.

### Décision 3 — octets du `doc_hash` quand le document n'en fournit pas (ECART)

**Option (b) recommandée.** Quand le document ne fournit pas de `doc_hash`, le lot en ligne calcule le **JSON canonique des passages** — pour chaque passage `ord`, `text` et `fields`, clés triées, sous la forme d'ADR-0019 — puis en prend le digest blake3, rendu `blake3:<hex>` (ADR-0034, ADR-0041). Le `doc_hash` du Parquet étant non nullable (`docs/04-donnees-et-ingestion.md:154`), le lot interne le remplit **toujours**.

L'écart vient de ce que `docs/05-api.md:88` dit « blake3 du contenu des passages » sans définir les octets, alors que `docs/04-donnees-et-ingestion.md` impose au producteur de changer le `doc_hash` dès que le **texte**, les **champs** ou le **découpage** des passages changent, et qu'un `doc_hash` identique fasse ignorer les passages. La forme retenue couvre les trois : le texte et les champs entrent dans le JSON, et `ord` porte le découpage.

### Décision 4 — emplacement, cycle de vie et mode du lot interne (ECART)

**Option (a) recommandée.** Le lot interne est écrit dans `<data>/tmp/online-<batch_id>/`, **hors du dossier de l'index**, avec `std::fs` (aucune dépendance nouvelle), et supprimé par un garde qui s'exécute en succès comme en échec. La reprise à l'ouverture supprime aussi les `online-*` résiduels de `<data>/tmp/` (INV-04), ce qui est sûr parce que `docs/02-architecture.md` impose un écrivain par index.

Un `tmp-*` dans le dossier de l'index serait effacé par la reprise (INV-04, `docs/02-architecture.md:120-121`) lors d'une ouverture concurrente : le lot interne disparaîtrait sous l'attache en cours.

**Mode.** Le lot en ligne accepte **`upsert` seulement**, pour la CLI comme pour la route P6. `replace_all` est refusé en `invalid_request`, avec un message qui nomme le mode : `docs/05-api.md:90` plafonne une requête à 10 000 passages, ce qui rend `replace_all` inutilisable, et ni `docs/04` ni `docs/05` ne tranchent le cas. `replace_all` reste le mode du lot déposé.

### Cohérence avec P6

Pour que la route P6 n'ait pas à rouvrir cet ADR :

- la CLI et la route utilisent le **même constructeur** de lot interne (`crible-store::online`) ;
- la route prend son `batch_id` dans le **corps** de la requête (`docs/05-api.md:78`) au lieu de le dériver du contenu ; l'idempotence reste celle de `docs/04-donnees-et-ingestion.md:178-180` ;
- les champs de manifeste déterministes, le dossier temporaire, la règle de `doc_hash` et le refus de `replace_all` sont les mêmes ; `producer.host` vaut `crible-online` dans les deux cas.

### Point déjà conforme, rappelé pour mémoire

- L'encodage se fait avec la session **fp32**, mais les vecteurs sont **écrits à la précision de référence de l'index** et le manifeste déclare `vector_dtype` = `reference_precision` (`docs/format-lot-v1.md:119`, ADR-0005). Sinon le contrôle 3 de l'attache refuserait le lot interne.
- Les passages **mixtes** (avec et sans vecteur) sont décidés **par passage** (`docs/05-api.md:89`).
- Le refus de longueur se fait **avant l'encodage** et `attach` le re-vérifie (INV-11, ADR-0053).

## Conséquences

- **Positives** :
  - la CLI et la route P6 partagent un seul format de document et un seul constructeur de lot ; aucun format parallèle à maintenir ;
  - l'identité par le contenu donne un rejeu sûr (INV-10) et un manifeste stable d'une copie à l'autre ;
  - aucune dépendance nouvelle : `std::fs` remplace `tempfile` à l'exécution ;
  - le `doc_hash` couvre texte, champs et découpage, donc les passages ne sont pas ignorés à tort.
- **Négatives / coûts** :
  - un fichier d'entrée ne peut pas supprimer de document ; la suppression exige un lot déposé ;
  - deux fichiers de même contenu mais d'octets différents créent deux lots, dont le second ne fait rien : la table `batches` garde deux lignes pour un même effet ;
  - `<data>/tmp/` devient un dossier que la reprise doit connaître, en plus des `tmp-*` de l'index ;
  - la sentinelle `created_at` prive le lot interne d'une date d'ingestion : l'horodatage se lit dans les journaux et dans la génération, pas dans le manifeste.
- **À surveiller** :
  - si un besoin de suppression en ligne apparaît, il faudra un nouvel ADR : le fichier JSONL et la route devraient alors porter des `deletions`, et `invalid_request` sur `replace_all` serait réexaminé ;
  - la taille de `<data>/tmp/` : un garde qui échoue laisserait des `online-*` jusqu'à la prochaine ouverture ;
  - deux versions du format de lot en ligne qui diffèrent sur les champs de manifeste produiraient deux manifests pour les mêmes documents : `producer.version` doit être incrémenté seulement avec un changement de format, et documenté.
  - **Note ajoutée le 2026-10-24, après l'acceptation, avec l'accord du propriétaire (NETTOYER-P2).** Le déterminisme du manifeste du lot interne ne vaut que pour un binaire donné. Les checksums des fichiers Parquet écrits dépendent de la version des crates `arrow` et `parquet` ; après une mise à jour de ces crates, le rejeu du même fichier `documents.jsonl` peut produire un manifeste différent pour le même `batch_id`, et donc renvoyer `409 batch_conflict` au lieu d'un rejeu sans effet. À revoir si l'on met à jour `arrow` ou `parquet`, ou si un rejeu après une telle mise à jour devient un besoin.
- **Documents à modifier à l'acceptation** : `docs/04-donnees-et-ingestion.md` (mode en ligne), `docs/05-api.md` (lot en ligne et `doc_hash`), `docs/02-architecture.md` (reprise, `<data>/tmp/`), et le document du format de lot si le manifeste interne y est décrit.
- **Effet sur la tâche** : ENCODER-DOCUMENTS-EN-LIGNE reste `BLOQUE` jusqu'à la décision du propriétaire. Aucune dépendance nouvelle n'est introduite.

## Alternatives écartées

| Décision | Alternative | Pourquoi écartée |
|---|---|---|
| 1 | (b) un corps d'API entier par ligne | Répète `batch_id` et `mode` à chaque ligne et permet un contenu incohérent d'une ligne à l'autre, alors que le fichier est un seul lot |
| 1 | (c) un tableau JSON | Contredit l'extension `.jsonl` imposée par la fiche ; charge le fichier entier en mémoire |
| 2 | (b) `batch_id` par exécution | Une nouvelle génération à chaque exécution : INV-10 est perdu |
| 2 | (c) option `--batch-id` | Hors fiche, et l'appelant peut casser l'idempotence en changeant l'id du même contenu |
| 2 | version du binaire dans `producer.version` | Le manifeste change à chaque mise à jour de Crible : le rejeu du même fichier renvoie `409 batch_conflict` |
| 3 | (a) textes joints par `\n`, puis blake3 | Un changement de champ de passage serait perdu sans erreur : les passages seraient ignorés à tort |
| 3 | (c) exiger `doc_hash` dans le fichier CLI | `doc_hash` est facultatif côté API (`docs/05-api.md:88`) ; la route P6 divergerait de la CLI |
| 4 | (b) `tempfile` à l'exécution | `tempfile` n'est listé que côté « Tests » de `docs/07-stack-et-deploiement.md:48` : l'ajouter au binaire demanderait son propre ADR, pour un besoin que `std::fs` couvre |
| 4 | (c) `tmp-*` dans le dossier de l'index | Effacé par la reprise (INV-04, `docs/02-architecture.md:120-121`) lors d'une ouverture concurrente, en pleine attache |
| 4 | (d) lot en mémoire | La conception et `attach` travaillent sur des fichiers, et `manifest_blake3` est calculé sur les octets lus (ADR-0041) : un lot en mémoire imposerait un second chemin d'attache |

## Complément du 2026-10-29

Un renvoi, sans changement des décisions ci-dessus.

**Constat.** L'« À surveiller » ci-dessus (`:89`) dit qu'un besoin de suppression en ligne demanderait un nouvel ADR. La route `POST /v1/indexes/{index}/documents` porte `deletions` (`docs/05-api.md:128`).

**Décision.** ADR-0107 accepte les `deletions` sur la route : le lot interne écrit un `deletions.parquet` conforme au format de lot v1, et l'attache les applique comme pour tout lot. Le fichier d'entrée de `crible ingest` (décision 1) reste sans `deletions` : une suppression en CLI passe par un lot déposé. ADR-0107 confirme aussi que la route prend son `batch_id` dans le corps (« Cohérence avec P6 »), le constructeur de lot interne restant le même pour la CLI et la route.

**Raison.** `docs/05-api.md:128` montre `deletions` depuis l'origine, et `replace_all` est refusé en ligne : sans ce chemin, l'API ne pourrait retirer aucun document.

## Références

- `docs/phases/P2-encodeur-et-recette.md`, tâche ENCODER-DOCUMENTS-EN-LIGNE
- `docs/04-donnees-et-ingestion.md` (mode en ligne, sémantique d'attache, `doc_hash`, INV-10)
- `docs/05-api.md` (lot en ligne, `doc_hash`, limite de 10 000 passages, route `POST /v1/indexes/{index}/documents`)
- `docs/02-architecture.md` (reprise, nettoyage des `tmp-*`, INV-04)
- `docs/07-stack-et-deploiement.md` (`tempfile` côté Tests)
- `docs/format-lot-v1.md` (manifeste, `vector_dtype` = `reference_precision`, `created_at`)
- `docs/08-invariants-et-tests.md` (INV-04, INV-10, INV-11)
- ADR-0005, ADR-0019, ADR-0034, ADR-0041, ADR-0053
