# ADR-0056 — Verrou de présence des lots en ligne (remplace la décision 4 d'ADR-0055 sur le nettoyage)

- **Statut** : Accepté (décision du propriétaire le 2026-10-24)
- **Date** : 2026-10-24 (proposition et acceptation)
- **Décideurs** : propriétaire
- **Décision** : accepté le 2026-10-24 par le propriétaire, tel que proposé : option (a), verrou de présence pris avec `std::fs::File::try_lock` ; la contention renvoie `overloaded` (503) ; la commande `crible ingest` documentée porte `--models`.
- **Proposé par** : agent greffier, sur demande de l'orchestrateur, à la suite de la relecture (tour 1) de la tâche ENCODER-DOCUMENTS-EN-LIGNE (phase P2) et du verdict `ECART` de `@architecte` du 2026-10-24
- **Remplace** : ADR-0055, **décision 4 seulement**, et seulement sur le nettoyage des lots résiduels. L'emplacement et le cycle de vie du dossier (`<data>/tmp/online-<batch_id>/`, hors du dossier de l'index) restent ceux d'ADR-0055.

## Contexte

La décision 4 d'ADR-0055 (`docs/adr/0055-lot-en-ligne-fichier-d-entree-identite-empreinte-et-lot-interne.md:54-60`) place le lot interne du mode en ligne dans `<data>/tmp/online-<batch_id>/` et fait supprimer par la reprise « les `online-*` résiduels de `<data>/tmp/` ». Elle justifie cette suppression par la phrase : « ce qui est sûr parce que `docs/02-architecture.md` impose un écrivain par index ». La même règle est écrite dans `docs/02-architecture.md:123`.

Or cette prémisse est fausse à la portée qu'elle énonce :

- `docs/02-architecture.md:135` dit **un écrivain par index**, et autorise les lectures concurrentes (**INV-02**). La portée est **l'index**, pas la racine de données.
- `<data>/tmp/` est **commun à la racine**, donc à tous les index qu'elle contient. Deux index d'une même racine ont deux écrivains légitimes, qui écrivent dans le même dossier temporaire.
- Le nettoyage est appelé **à chaque ouverture, lecture comprise**, et balaie **toute la racine**. Une commande de lecture sur l'index A peut donc supprimer le lot vivant de l'index B, pendant que B attache ce lot.
- Un dossier `online-*` **vivant** et un résidu de crash sont **indistinguables** par leur seul contenu : rien dans le dossier ne porte la preuve qu'un écrivain le tient encore.

Conséquence : l'ingestion concurrente échoue. Elle n'écrit pas de corruption — `attach` lit le lot avant le point de commit SQLite (ADR-0016), donc un lot disparu avant le commit ne laisse rien derrière lui — mais elle échoue sans raison valable, et l'échec est difficile à diagnostiquer.

Il faut donc une décision nouvelle. Elle ne change **pas** la disposition du dossier, seulement la façon dont un lot est reconnu comme vivant.

Faits utiles à la décision :

- Rust a stabilisé `std::fs::File::lock`, `try_lock` et `unlock` en 1.89 ; `rust-toolchain.toml` fixe `channel = "1.99.1"`. Ces appels sont disponibles sans dépendance nouvelle, ce que `docs/07-stack-et-deploiement.md` exige.
- `flock` est un verrou **consultatif**, qui fonctionne sur macOS et Linux **sur un volume local**. Le déploiement monte `/data` comme volume Docker nommé (`docs/07-stack-et-deploiement.md:120`), donc en local. Sur un montage réseau (NFS, CIFS), le verrou peut ne pas être partagé entre clients ou ne pas fonctionner du tout : il faut une règle pour ce cas.
- Le code de la tâche ENCODER-DOCUMENTS-EN-LIGNE implémente déjà l'option (a) ci-dessous. La tâche reste `BLOQUE` jusqu'à la décision du propriétaire.

## Décision

**Option (a) : verrou de présence pris avec la bibliothèque standard.** Un lot en ligne est vivant tant qu'un verrou exclusif est tenu sur un fichier `.lock` voisin. Le nettoyage ne supprime que ce qu'il peut verrouiller.

**1. Prise du verrou par l'ingestion.** L'ingestion ouvre en lecture-écriture avec création `<data>/tmp/online-<batch_id>.lock`, y prend un verrou exclusif avec `try_lock` **avant** de créer le dossier du lot, et le tient jusqu'**après** `attach`. Si le verrou est déjà tenu (`io::ErrorKind::WouldBlock`), elle échoue **sans rien créer** — ni dossier, ni ligne dans `batches`. Le garde qui supprime le dossier en succès comme en échec supprime ensuite le `.lock`, dans cet ordre : dossier, puis verrou.

**2. Nettoyage fondé sur le verrou.** Le nettoyage ne traite **jamais** l'absence de `.lock` comme la preuve d'un résidu. Pour chaque `online-*`, il ouvre en lecture-écriture avec création le `.lock` correspondant et tente `try_lock` :

- succès : aucun écrivain ne tient le lot, il est résiduel ; le dossier est supprimé, puis le `.lock` ;
- `WouldBlock` : un écrivain tient le lot, il est laissé en place.

**3. Toute autre erreur de verrou ne supprime rien.** Un système de fichiers qui ne sait pas verrouiller (montage réseau NFS ou CIFS, par exemple) produit une erreur autre que `WouldBlock`. Dans ce cas le nettoyage **ne supprime rien** pour cet élément, et journalise. La règle est asymétrique à dessein : un résidu qui s'accumule est un coût, une suppression de lot vivant est une panne.

**4. Portée du nettoyage réduite aux ouvertures en écriture.** Le nettoyage n'est plus exécuté par les commandes de lecture (`search`, `inspect`) : elles ne suppriment rien dans `<data>/tmp/`. Il reste exécuté à l'ouverture en écriture. Ce n'est pas ce qui apporte la sûreté — c'est le verrou — mais cela réduit la surface où le nettoyage s'exécute, et une lecture ne modifie plus le disque.

**5. Code d'erreur de la contention.** La contention sur le verrou renvoie **`overloaded`** (503, avec `Retry-After` dans la couche HTTP). C'est le code existant le plus juste : il désigne déjà, dans `docs/05-api.md:42`, une ressource momentanément occupée qu'il faut réessayer, aux côtés du verrou SQLite non obtenu dans le délai d'ADR-0038. Le message nomme le fichier de verrou et invite à réessayer.

`batch_conflict` (409) est **exclu** : `docs/05-api.md:38` et ADR-0041 le réservent au cas « même `batch_id`, manifeste différent » (INV-10). L'employer pour une contention rendrait ce diagnostic ambigu, alors que les deux causes appellent des gestes différents : corriger le manifeste, ou attendre.

Si le propriétaire veut distinguer la contention dans les analytics, il faudra un code dédié (par exemple `batch_in_progress`, 409) : cela demande une ligne dans `docs/05` et un ADR. **Ce n'est pas décidé ici.**

**Précisions.**

- Le nom du verrou suit celui du dossier : `<data>/tmp/online-<batch_id>.lock` est le voisin de `<data>/tmp/online-<batch_id>/`. Les deux noms doivent rester dérivés de la même valeur, sinon le nettoyage examinerait le verrou d'un autre dossier.
- Le verrou porte sur un `batch_id`, donc sur un contenu (ADR-0055, décision 2). Deux ingestions du **même** fichier en parallèle se sérialisent : la seconde échoue en `overloaded`, et un rejeu ultérieur ne fait rien (INV-10).
- Ce qui n'est pas décidé ici : le code d'erreur dédié éventuel, et le comportement du futur serveur HTTP, qui réutilise le même constructeur (`crible-store::online`) et hérite donc de `overloaded` par la file d'écriture.

### Point annexe — commande `crible ingest` documentée sans `--models`

La commande citée par `docs/04-donnees-et-ingestion.md:231` et par la fiche de la tâche, `crible ingest --data … --index … documents.jsonl`, **omet `--models`**. Or la CLI l'exige, comme `crible attach` : le modèle vit hors du volume de données (`docs/07-stack-et-deploiement.md:121`, `crates/crible/src/main.rs:56-70`). La commande documentée ne peut donc pas s'exécuter telle quelle.

Alignement retenu : `docs/04` écrit la commande complète, `crible ingest --data … --models … --index … documents.jsonl`. Le corps d'ADR-0055 n'est pas réécrit : un ADR accepté ne se corrige pas, et la citation fautive y reste, sans effet normatif.

## Conséquences

- **Positives** :
  - un lot vivant n'est plus supprimable par un autre index ni par une commande de lecture : la cause de l'échec disparaît, alors que la disposition du dossier ne change pas ;
  - aucune dépendance nouvelle : `std::fs::File::lock` remplace une détection heuristique ; le verrou est libéré par le système même si le processus meurt, donc la reprise reste correcte (INV-04) ;
  - la règle de sûreté est explicite et vérifiable par un test : pas de verrou pris, pas de suppression ;
  - le code d'erreur de la contention est celui qui existait déjà pour « réessayer plus tard ».
- **Négatives / coûts** :
  - un fichier `.lock` de plus par lot dans `<data>/tmp/`, qu'il faut supprimer avec le dossier ;
  - sur un système de fichiers sans verrou, les résidus s'accumulent et demandent un nettoyage manuel ; le moteur ne les supprime plus tout seul ;
  - le nettoyage reste appelé à l'ouverture en écriture : deux écrivains de deux index peuvent encore se rencontrer sur `<data>/tmp/`, mais le verrou les arbitre au lieu de les blesser ;
  - `overloaded` couvre maintenant deux causes différentes (file d'attente, verrou de présence) : la distinction se lit dans le message, pas dans le code.
- **À surveiller** :
  - **montages réseau** : si `/data` passe sur NFS ou CIFS, `try_lock` échoue ou devient local au client ; le nettoyage cesse alors de fonctionner et les résidus s'accumulent. Signal : des `online-*` qui survivent à plusieurs ouvertures en écriture ;
  - **résidus qui s'accumulent** : la taille de `<data>/tmp/` est le signal d'un verrou indisponible ou d'un garde qui échoue ;
  - **contention fréquente** : un `overloaded` répété sur un même `batch_id` signale deux ingestions concurrentes du même fichier, à corriger chez l'appelant.
  - **Note ajoutée le 2026-10-24, après l'acceptation, avec l'accord du propriétaire (NETTOYER-P2).** Le verrou de présence garde un résidu de type ABA. Dans `lock_presence`, la vérification de l'identité du fichier (device, inode) après la prise du verrou laisse un cas résiduel : un inode recyclé avec le même numéro dans une fenêtre de quelques microsecondes, qui demande trois acteurs concurrents. Ce cas est accepté parce que seul un détenteur du verrou supprime le fichier `.lock`. À revoir si le serveur (P6) multiplie les écrivains concurrents sur une même racine.
- **Documents à modifier à l'acceptation** : `docs/02-architecture.md` (« Reprise » et « Concurrence »), `docs/04-donnees-et-ingestion.md` (mode en ligne, commande `crible ingest`), `docs/format-lot-v1.md` (« Lot interne du mode en ligne »), et une mention sur ADR-0055 du type « décision 4 : nettoyage complété ou remplacé par ADR-0056 », sans réécrire son corps.
- **Effet sur la tâche** : ENCODER-DOCUMENTS-EN-LIGNE reste `BLOQUE` jusqu'à la décision du propriétaire. Le code déjà écrit implémente l'option (a) et n'a pas besoin d'être repris si elle est acceptée.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| (b) sous-dossier par index, `<data>/tmp/<index>/online-<batch_id>/`, nettoyé par l'écrivain de l'index | Plus simple, aucun verrou, mais deux ingestions parallèles du **même** index (deux processus CLI) se supprimeraient encore, et la disposition change dans trois documents (ADR-0055, `docs/02`, `docs/format-lot-v1.md`) |
| (c) nettoyer seulement à l'ouverture en écriture | Ne couvre pas deux écrivains sur deux index : la suppression croisée reste possible. Insuffisant seul ; retenu seulement comme réduction de portée, en complément du verrou (point 4) |
| Pas de nettoyage du tout | Laisse les résidus de crash s'accumuler sans borne, et ne résout rien de plus que le verrou, qui coûte moins cher qu'un nettoyage manuel |
| Verrou par une caisse (`fs2`, `fd-lock`) | `docs/07` ne liste pas ces caisses et n'en ajoute pas pour un besoin que la bibliothèque standard couvre depuis Rust 1.89 |
| Marqueur de vie écrit dans le dossier (`pid`, horodatage, battement) | Un `pid` est réutilisable et un horodatage ne distingue pas un écrivain lent d'un résidu ; un battement demande un thread et une fenêtre de tolérance, donc une heuristique là où le système fournit un verrou exact |
| Renvoyer `batch_conflict` (409) sur la contention | `docs/05-api.md:38` et ADR-0041 réservent ce code à « même `batch_id`, manifeste différent » (INV-10) ; la contention appelle « réessayer », pas « corriger le manifeste » |
| Verrou pris par le serveur seul, CLI non verrouillée | La CLI et la route partagent le même constructeur (ADR-0055, « Cohérence avec P6 ») : deux chemins, deux comportements, et la CLI resterait capable de casser un lot du serveur |

## Références

- ADR-0055 (lot en ligne : emplacement, identité, manifeste, `doc_hash`, mode), décision 4 remplacée sur le nettoyage
- ADR-0016 (point de commit SQLite), ADR-0038 (`busy_timeout`), ADR-0041 (sémantique d'attache, `batch_conflict`, INV-10)
- `docs/02-architecture.md` (« Reprise », « Concurrence », un écrivain par index)
- `docs/04-donnees-et-ingestion.md` (mode en ligne, fichier d'entrée de `crible ingest`)
- `docs/05-api.md` (table des erreurs, lot en ligne, codes `batch_conflict` et `overloaded`)
- `docs/07-stack-et-deploiement.md` (volume `/data`, modèles hors du volume de données)
- `docs/format-lot-v1.md` (« Lot interne du mode en ligne »)
- `docs/08-invariants-et-tests.md` (INV-02, INV-04, INV-10)
- `docs/phases/P2-encodeur-et-recette.md`, tâche ENCODER-DOCUMENTS-EN-LIGNE
- `crates/crible/src/main.rs` (la CLI `ingest` exige `--models`)
- Rust 1.89 : stabilisation de `File::lock`, `try_lock` et `unlock`
