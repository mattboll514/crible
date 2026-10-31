# ADR-0116 — Concurrence entre la CLI et une instance en service : ouverture en lecture sans écriture et verrou d'écrivain par index

- **Statut** : Accepté (décidé par le propriétaire le 2026-10-31)
- **Date** : 2026-10-31 (proposition et acceptation)
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, sur demande du propriétaire dans la conversation de l'orchestrateur le 2026-10-31, à partir du contrôle de fin de phase de P6 (`NETTOYER-P6`) et du dossier de `@architecte` du 2026-10-31. Les deux tâches que cet ADR crée sont `OUVRIR-EN-LECTURE-SANS-ECRIRE` et `VERROU-ECRIVAIN-PAR-INDEX`, en P8.
- **Remplace** : —

## Contexte

Deux questions distinctes restaient ouvertes depuis P4 et le contrôle de fin de P6 : ce qu'une **ouverture en lecture** écrit, et ce qui arrive quand la **CLI et une instance en service** écrivent le même index. `docs/suivi.md` les portait aux lignes 560, 585 et 586 des « Idées hors périmètre », et ADR-0099 (ligne 40) renvoie explicitement la première à « un ADR avant tout réplica ».

**Une ouverture en lecture écrit sous le dossier de l'index.** `GenerationStore::open_read_only` passe par `open_index` (`crates/crible-store/src/generation.rs`), qui appelle `recover` (vers la ligne 789) et `ensure_manifest` (vers 820) dans les deux modes. `recover` (vers 2158-2200) crée `segments/` et `manifests/`, supprime les `tmp-*` de `manifests/` et du dossier de l'index, tout dossier de segment absent de la table `segments`, et un `tmp-codes.bin` ; `ensure_manifest` (vers 2437-2442) écrit `manifests/<G>.json` s'il manque. Sur un index propre, rien n'est écrit, ce qui explique que le défaut soit resté invisible.

**Les commandes de lecture et d'écriture de la CLI.** `crible search` ouvre en lecture seule (`crates/crible/src/commands.rs`, vers 765), `crible inspect` aussi (vers 984). `crible export` ouvre en **écriture** (vers 397), ainsi que `index create`, `attach`, `ingest`, `patch`, `compact`, `rebuild-lexical`, `snapshot` et `restore`. Le serveur ouvre chaque index en écriture (`crates/crible-server/src/state.rs`, vers 985 ; ADR-0099).

**Un risque latent existe dès aujourd'hui, sans serveur.** `docs/04` (vers la ligne 326, colonne « Export ») prévoit un export pendant que l'écrivain continue. Or l'ouverture en écriture d'`export` lance `recover`, qui peut supprimer le `segments/tmp-<id>` d'une attache en cours du serveur, ou un segment renommé mais pas encore commité. Le défaut est dans le code depuis P3, et il ne se manifeste qu'en concurrence.

**Une lecture qui ne répare pas reste correcte.** Un `tmp-*` laissé n'a qu'un coût disque ; le manifeste n'est lu par aucun chemin de lecture (ADR-0016) ; et `recover` ne touche pas au dossier `epochs/`. La réparation peut donc attendre l'ouverture en écriture suivante sans changer un résultat de recherche.

**Face à un serveur vivant, une commande d'écriture de la CLI est dangereuse.** `recover` peut détruire le travail en cours du serveur. `compact` écrit `CURRENT_EPOCH = E+1` puis `reclaim_epochs` (`crates/crible-store/src/compact.rs`) supprime l'époque E, que les lecteurs du serveur tiennent encore : la rétention ne connaît que les baux du processus (`generation.rs`, vers 1113). Les lectures du serveur échouent, ses commits sont refusés par la garde d'ADR-0108, et un redémarrage est nécessaire. `restore` est de la même classe. Le serveur ne voit pas les commits de la CLI, son état étant publié en `ArcSwap` (`docs/02`, section « Concurrence »). SQLite (WAL, `busy_timeout` d'ADR-0038) n'apporte que la vivacité, jamais l'exclusion entre processus.

**Les règles écrites sont partielles.** Seuls `patch` (`docs/04`, vers la ligne 304), `snapshot` et `restore` (`docs/04`, vers les lignes 349-354 ; ADR-0065 : « Aucun verrou inter-processus n'est ajouté ») ont une règle d'exploitation. Aucun mécanisme ne la fait respecter.

**Le raccourci d'exploitant est le vrai risque.** P8 et P9 passent par l'API (`docs/phases/P9-migration-legifacile.md`, vers les lignes 109-111). Le danger n'est donc pas le chemin prévu, mais le `docker exec … crible …`, ou la CLI de l'hôte lancée sur le volume monté.

**Le verrou ne demande aucune dépendance.** `std::fs::File::try_lock` est déjà employé (`crates/crible-store/src/online.rs`, vers 1223, avec la vérification d'identité après verrouillage d'ADR-0056, vers 1200-1206) ; la toolchain est 1.99.1 (`rust-toolchain.toml`).

## Décision

**Décision 1 — une ouverture en lecture n'écrit rien sous le dossier de l'index.**

`open_read_only` n'exécute plus `recover` ni `ensure_manifest`. Les réparations n'ont lieu qu'à l'ouverture en écriture, et INV-04 est inchangé : au redémarrage, l'état est la dernière génération commitée, tout le reste est reconstruit ou nettoyé à cette ouverture. La seule exception est explicite : les fichiers que SQLite gère lui-même en mode WAL (`-wal`, `-shm`, ADR-0038).

`crible export` ouvre en lecture. Les codes binaires sont construits en mémoire, comme pour `search` (ADR-0069, décision 2), et non reconstruits sur disque.

Le commentaire d'`inspect` qui parle d'un montage inscriptible est corrigé par la tâche `OUVRIR-EN-LECTURE-SANS-ECRIRE`.

**Hors de cette décision** : la lecture depuis un volume réellement monté en lecture seule. Le `-shm` du WAL exige un dossier inscriptible, et cette question reste séparée.

**Décision 2 — un seul écrivain par index, règle écrite et verrou.**

**(a) Règle écrite dans `docs/02`, section « Concurrence ».** Aucune commande d'écriture de la CLI ne s'exécute sur un index servi par une instance en service : on arrête le serveur d'abord, comme pour `snapshot` et `restore`. Les commandes de lecture (`search`, `inspect`, `export`) restent permises à côté d'un serveur en service.

**(b) Verrou consultatif par index.** Un fichier `<dossier de l'index>/WRITER_LOCK` est pris par `std::fs::File::try_lock`, exclusif et non bloquant, avec la vérification d'identité après verrouillage d'ADR-0056 : un fichier recréé entre l'ouverture et le verrouillage ne doit pas donner deux verrous.

- Le serveur prend le verrou à l'ouverture de chaque index, au démarrage comme à la création par l'API, et le tient tant qu'il sert l'index. Il le relâche à la suppression de l'index.
- Chaque commande de la CLI qui ouvre un index existant **en écriture** prend le verrou pour toute sa durée.
- Si le verrou est déjà tenu : refus immédiat, sans attente, avec le code `overloaded` (503) de `docs/05`. Le message nomme l'index et dit qu'un autre processus l'écrit ; il ne porte jamais de chemin du serveur, comme l'exige ADR-0101.
- Le noyau relâche le verrou à la mort du processus, y compris après un arrêt brutal.
- Le fichier est nommé dans la description du disque de `docs/02`. Il est exclu de l'inventaire d'époque, des instantanés et des exports.
- Les commandes de lecture ne prennent pas le verrou.

**Choix du code d'erreur.** `overloaded` (503) est le seul code existant de `docs/05` qui décrive cet état, et c'est déjà celui qu'ADR-0056 retient quand un verrou de présence est tenu par une autre ingestion. Les autres codes sont écartés : `invalid_request` (400) qualifierait une requête fautive, alors que la commande est valide ; `internal` (500) annoncerait une erreur inattendue, alors que le cas est prévu et que la cause est connue ; `immutable_setting` et `batch_conflict` (409) sont réservés à d'autres situations. Une réponse `overloaded` porte `Retry-After: 1` par la règle unique d'ADR-0103, ce qui reste correct : le refus dure tant que le serveur sert l'index.

**Ce qui n'est pas décidé ici** : aucun nouveau code d'erreur, aucune attente avec délai, aucun verrou en lecture, et aucun changement du comportement du serveur quand il est seul.

## Conséquences

- **Positives** :
  - une commande de lecture n'écrit plus rien sous le dossier de l'index : elle peut viser un index servi sans risquer de détruire une attache en cours, ni de réécrire un manifeste ;
  - `crible export` redevient ce que `docs/04` décrit : une opération qui laisse l'écrivain continuer, sans réparation sur le disque ;
  - l'erreur d'exploitant devient un refus immédiat et nommé, au lieu d'un état corrompu qui demande un redémarrage ;
  - la règle est écrite et vérifiée à la fois : l'écrit protège de l'oubli, le verrou protège de l'erreur ;
  - aucune dépendance nouvelle, le verrou venant de la bibliothèque standard.
- **Négatives / coûts** :
  - un index ouvert en lecture seule par la CLI ne se répare plus : un `tmp-*` ou un manifeste absent persiste jusqu'à la prochaine ouverture en écriture, et coûte du disque entre-temps ;
  - le verrou ajoute un fichier par index, et un appel système à chaque ouverture en écriture ;
  - le serveur doit tenir un descripteur de plus par index, et le code de suppression d'index doit le relâcher explicitement ;
  - un opérateur qui écrivait par la CLI sur un serveur vivant perd une habitude et reçoit un `503` : il doit arrêter le serveur ;
  - deux tâches de plus en P8, avant l'image Docker.
- **À surveiller** :
  - la multiplication des fichiers résiduels `tmp-*` et des manifestes absents sur les index servis, qui signalerait une ouverture en écriture trop rare ; le signal est un `crible inspect` qui ne répare plus ;
  - les `503 overloaded` de la CLI sur un index servi, dans les journaux d'exploitation : un `503` fréquent signale soit un serveur oublié, soit un enchaînement de commandes qui devrait être une route de l'API ;
  - le comportement de `try_lock` sur un montage réseau, déjà signalé par ADR-0056 : si `/data` passe sur NFS ou CIFS, le verrou peut échouer ou devenir local au client ;
  - le cas où un volume serait réellement monté en lecture seule : la décision 1 ne le couvre pas, et la question reste ouverte.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Statu quo, avec `export` déclaré commande d'écriture | Contredit `docs/04` (l'écrivain continue pendant l'export) et laisse le risque latent : `recover` peut supprimer le travail d'une attache en cours. |
| Un drapeau `--read-only` sur les commandes | Crée deux chemins à tester, en lecture et en écriture, pour le même comportement que la décision 1, qui elle est inconditionnelle. |
| La règle écrite seule, sans verrou | Protège de l'oubli, pas de l'erreur : rien n'empêche un `docker exec` de lancer `crible compact` sur un index servi. |
| Généraliser la garde d'ADR-0108 à la CLI | Ne couvre pas `reclaim_epochs` : la compaction supprimerait toujours l'époque que les lecteurs du serveur tiennent, parce que la rétention ne voit que les baux du processus. |
| Un verrou par une dépendance externe | Inutile : `std::fs::File::try_lock` est stable depuis 1.89 et déjà employé par ADR-0056, sans dépendance nouvelle. |

## Références

- `docs/02-architecture.md` : « Organisation sur disque » (fichier `WRITER_LOCK`), « Reprise au démarrage » (réparation à l'ouverture en écriture seulement), « Concurrence » (règle (a)).
- `docs/04-donnees-et-ingestion.md` : « Export, instantanés, restauration » (export en lecture ; règle d'arrêt du serveur pour `patch`, `snapshot` et `restore`).
- `docs/05-api.md` : code `overloaded` (503), cause du verrou d'écrivain.
- `docs/phases/P8-docker-et-deploiement.md` : fiches `OUVRIR-EN-LECTURE-SANS-ECRIRE` et `VERROU-ECRIVAIN-PAR-INDEX` ; dépendance ajoutée à `DEFINIR-VOLUMES-ET-CONFIG`.
- `docs/suivi.md` : « Idées hors périmètre », lignes 560, 585 et 586.
- ADR-0016 (le commit SQLite est l'unique point de commit), ADR-0038 (`busy_timeout`), ADR-0056 (verrou de présence, vérification d'identité), ADR-0069 (codes binaires, décision 2 : en RAM seulement en lecture seule), ADR-0065 (instantané et restauration hors ligne), ADR-0087 et ADR-0088 (reprise lexicale à l'ouverture en écriture), ADR-0099 (le serveur ouvre en écriture), ADR-0101 (message sans chemin du serveur), ADR-0103 (`Retry-After: 1`), ADR-0108 (garde de cohérence de l'époque).
