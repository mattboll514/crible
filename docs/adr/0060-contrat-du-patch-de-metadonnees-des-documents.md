# ADR-0060 — Contrat du patch de métadonnées des documents

- **Statut** : Accepté (décision du propriétaire le 2026-10-25)
- **Date** : 2026-10-25 (proposition et acceptation)
- **Décideurs** : propriétaire
- **Décision** : accepté le 2026-10-25 par le propriétaire, tel que proposé, avec toutes les options (a) : entrée JSONL `{doc_id, fields}` et corps HTTP de même forme, fusion par champ, `null` valeur posée, refus atomique `not_found` d'un `doc_id` inconnu ou retiré, noms réservés refusés par la règle du champ inconnu et champs non filtrables ou non rendus acceptés, 32 Mo et 10 000 entrées en `payload_too_large`, rapport `{updated, unchanged, generation}` non persisté, `doc_hash` inchangé et `updated_generation = G+1`, aucune génération pour un patch sans effet.
- **Proposé par** : agent greffier, sur demande de l'orchestrateur, à la suite d'une `QUESTION-CONCEPTION` de `@dev` pendant la tâche PATCHER-METADONNEES (phase P3), instruite par `@architecte` le 2026-10-25
- **Remplace** : —

## Contexte

La tâche PATCHER-METADONNEES est commencée sur la branche `tache/PATCHER-METADONNEES`, avant implémentation. Sa fiche (`docs/phases/P3-cycle-de-vie.md:60-71`) demande `crible-store::patch`, une commande `crible patch` et trois critères d'acceptation : les filtres reflètent la nouvelle valeur, les vecteurs et les iid sont inchangés, une valeur mal typée est refusée sans aucun effet. `docs/05-api.md:58` annonce la route correspondante, `PATCH /v1/indexes/{index}/documents`, qui arrive en P6.

La conception ne décrit nulle part le contrat de cette opération. `docs/05-api.md:58` est une ligne de tableau, sans corps de requête ; `docs/07-stack-et-deploiement.md:15` ne liste pas `patch` dans les commandes de la CLI ; `docs/02-architecture.md:117` dit seulement que « modifier des métadonnées » exécute les étapes 3, 5 et 6 de la séquence d'écriture. Rien ne dit la forme de l'entrée, la règle de fusion, le sens d'un `null`, le sort d'un `doc_id` inconnu, le rapport, ni ce que la transaction écrit.

Ce que le contexte tranche déjà, et sur quoi cet ADR s'appuie :

- **unités inchangées** : un patch ne touche ni le texte, ni les vecteurs, ni les iid, ni le lexical (`docs/02-architecture.md:142` : « Textes et vecteurs sont immuables pour un iid donné ») ;
- **un écrivain par index** : les écritures d'un index passent une seule file bornée (`docs/02-architecture.md:136`), et un patch y passe comme une attache ;
- **code d'erreur de contention** : `overloaded` 503 avec `Retry-After`, y compris pour un verrou SQLite non obtenu en 1 000 ms (ADR-0038, `docs/05-api.md:42`) ;
- **`not_found`** est déjà réservé au « Document, job ou lot introuvable » (`docs/05-api.md:36`) ;
- **`invalid_request`** couvre déjà une faute de schéma (`docs/04-donnees-et-ingestion.md:42`, `:64`) et une valeur hors bornes (`docs/05-api.md:31`) ;
- **`payload_too_large`** existe (413, `docs/05-api.md:39`) mais n'est plafonné que pour le lot en ligne : 32 Mo et 10 000 passages (`docs/05-api.md:240`) ;
- **une seule forme d'entrée par usage** : `docs/04-donnees-et-ingestion.md:229` impose qu'« un seul type décrit ce document : il sert au corps HTTP de la route et à la commande de la CLI, pour qu'aucun format parallèle ne soit maintenu » ;
- **INV-01** : ni la CLI ni la route ne rendent d'iid (`docs/08-invariants-et-tests.md:9`, `docs/05-api.md:18`).

### Un fait à corriger d'abord : un patch de champs de document ne change pas `doc_hash`

Une question posée pendant la tâche supposait qu'un patch de métadonnées modifie le `doc_hash`. C'est faux, et il faut l'écrire avant tout le reste.

- ADR-0055, décision 3 : le `doc_hash` calculé par le moteur est le blake3 du **JSON canonique des passages** — `ord`, `text` et `fields` **par passage**.
- `docs/04-donnees-et-ingestion.md:107` : « Le producteur doit le changer dès que **les passages** changent : leur texte, leurs champs ou leur découpage. »
- `docs/04-donnees-et-ingestion.md:209` : `metadata_updated` est le cas « `doc_hash` **identique**, champs de document différents ». Les champs de document sont donc, par construction, **hors** du hash.

**Un patch de champs de document laisse `doc_hash` inchangé.** Écrire l'inverse serait un écart à ADR-0055 décision 3 et à `docs/04:209`.

Corollaire documentaire : `docs/05-api.md:88` (« un `doc_hash` identique signifie bien que rien n'a changé ») est trop large. Il doit se lire « rien n'a changé **dans les passages** ».

### Conséquence acceptée, à écrire dans l'ADR et dans `docs/05`

Un patch n'est **pas durable** face à une republication du même document. `docs/04-donnees-et-ingestion.md:169` tranche déjà : « Connu, `doc_hash` identique → Passages ignorés ; champs de document mis à jour s'ils diffèrent ». Une attache ultérieure qui porte l'ancienne valeur du champ **écrase** la valeur patchée, sans erreur et sans nouvelle génération de passages. C'est cohérent avec la règle d'attache et cela ne se corrige pas ici.

### Découpage de cet ADR

Neuf décisions **séparables**. Les cinq points laissés au choix du propriétaire ont tous retenu l'option (a) ; les alternatives restent décrites, marquées comme écartées, dans chaque décision et dans « Alternatives écartées ».

Sont **hors** de cet ADR, et relèvent de l'implémentation : le nom des types Rust, le découpage des modules, la forme exacte des messages au-delà de ce qui est imposé ci-dessous, les options de la CLI autres que les deux chemins d'entrée, et l'ordre des étapes internes.

## Décision

### Décision 1 — forme de l'entrée, commune à la CLI et à HTTP (NON-COUVERT)

**Décision : option (a).** L'entrée d'un patch est une **liste de documents à patcher**, et son élément a une seule forme pour les deux chemins :

```json
{ "doc_id": "LEGIARTI000006419280", "fields": { "juridiction": "cass" } }
```

- **Fichier de la CLI** : `crible patch --data <fichier>.jsonl --index <nom>`. Le fichier est du **JSONL**, un objet par ligne, de la forme ci-dessus. Pas de `--models` : un patch n'encode rien, contrairement à `crible ingest` (`docs/04-donnees-et-ingestion.md:231`, ADR-0055).
- **Corps HTTP (P6)** : `{ "documents": [ { "doc_id": …, "fields": { … } } ] }`. L'élément du tableau est **exactement** la ligne du fichier.
- **Portée du « un seul type »** : `docs/04:229` vaut pour le **document en ligne**, qui porte des `passages`. Une entrée de patch ne peut pas en porter : un patch ne ré-encode rien et ne crée aucun passage. La règle se lit donc « **une seule forme par type d'entrée**, partagée par la CLI et la route », et l'entrée de patch est un **second type**, plus petit. `docs/04:229` et `docs/05` doivent le dire, sinon P6 rouvrira la question.
- Un patch **ne crée jamais** de document : la création exige des passages, donc passe par l'attache ou par le lot en ligne. Un `doc_id` inconnu relève de la décision 4.
- Ni la CLI ni la route ne rendent d'iid, et le rapport n'en contient aucun (INV-01).

### Décision 2 — fusion par champ, et `fields` vide (NON-COUVERT)

**Décision : option (a).** Le patch **fusionne** : chaque clé présente dans `fields` remplace la valeur du champ de même nom, les clés absentes gardent leur valeur. Un `fields: {}` est un **no-op** : le document est compté `unchanged`, aucune écriture.

Indices convergents : `docs/05-api.md:58` dit « mettre à jour des métadonnées », `docs/02-architecture.md:152` décrit le cas « Métadonnée seule » comme « mise à jour **du champ** et des bitmaps » (au singulier), et ADR-0031 a posé pour `PATCH /v1/indexes/{index}` qu'un `PATCH` refuse une clé inconnue au lieu de remplacer l'objet entier.

Précision : la comparaison qui distingue `updated` de `unchanged` porte sur le **JSON canonique** des champs (ADR-0019), donc l'ordre des clés ne crée pas de faux changement.

### Décision 3 — `null` est une valeur posée, jamais un retrait (NON-COUVERT, une seule option cohérente)

**Décision : option (a).** `"champ": null` **pose la valeur null** sur le champ. Il n'existe aucun moyen de retirer une clé de `fields` par un patch.

- `docs/04-donnees-et-ingestion.md:40` : « Tous les champs acceptent null. » Un null est donc une valeur du domaine, au même titre qu'un `keyword`.
- ADR-0035 a **refusé** l'option C, qui distinguait un champ absent d'un null explicite, et a fixé `exists: false` = « absent ou null » (`docs/05-api.md:107`). Un retrait de clé par `null` réintroduirait cette distinction, invisible aux filtres mais visible dans `GET /documents` : une asymétrie nouvelle entre les deux chemins de lecture.
- Un besoin réel de retrait rouvrirait ADR-0035, pas cet ADR.

### Décision 4 — `doc_id` inconnu ou retiré (choix du propriétaire)

Le comportement n'est tranché par aucune source. Deux points sont en revanche déjà fixés et ne sont pas des options :

- **atomicité** : le fichier s'applique **entièrement ou pas du tout**. `docs/02-architecture.md:106` n'admet de génération que pour « chaque changement visible », INV-03 exige qu'une génération soit visible entièrement ou pas du tout (`docs/08-invariants-et-tests.md:11`), et la fiche exige « sans aucun effet » sur une entrée refusée (`docs/phases/P3-cycle-de-vie.md:71`). Un refus partiel n'existe pas ;
- **code** : le refus se fait en `not_found`, code déjà réservé au « Document, job ou lot introuvable » (`docs/05-api.md:36`).

Ne peut pas être distingué : « jamais existé » et « retiré ». ADR-0041 supprime la ligne `documents` au commit de génération, donc le moteur ne garde aucune trace du second cas. Une option « refuser seulement un document retiré » est **indisponible**.

**Décision : option (a).** Un `doc_id` inconnu ou retiré fait **refuser le fichier entier** en `not_found`, et aucun document n'est appliqué. Le message nomme le `doc_id` fautif et son numéro de ligne.

Motif : un patch est une opération ciblée, écrite à la main ou par un script qui nomme des `doc_id` précis, contrairement à un lot `upsert` qui décrit un état. Un `doc_id` inconnu y est presque toujours une faute de frappe, un index différent de celui qu'on croit, ou une intention périmée après suppression. Ignorer en silence laisserait l'appelant croire que la métadonnée est posée, et le rapport ne le lui dirait qu'en creux. Le coût d'un refus est un aller-retour : l'appelant retire la ligne et rejoue, ce qui est justement l'occasion de constater que le document a disparu.

Alternative écartée : **ignorer** le `doc_id` inconnu, le compter `ignored` dans le rapport et appliquer le reste, par symétrie avec `docs/04-donnees-et-ingestion.md:173` (« Suppression d'un `doc_id` inconnu → ignorée, comptée dans le rapport »). Un fichier rejoué sur un index qui a changé depuis ne buterait alors pas sur une ligne morte, et un rapport `ignored: 3` le signalerait. Mais une erreur de producteur (mauvais index, mauvais `doc_id`) passerait tant que le compte n'est pas lu, et le précédent de `:173` porte sur une **suppression** dans un lot qui décrit un état, pas sur un patch ciblé.

Précisions communes : un `doc_id` dupliqué dans le fichier est refusé en `invalid_request` (même règle qu'ADR-0040 pour un lot, appliquée au fichier), pour que le résultat ne dépende pas de l'ordre des lignes. Le message nomme le `doc_id` — id externe, donc admissible hors du moteur (INV-01) — et **jamais** une valeur de champ.

### Décision 5 — champ de passage, champ inconnu, nom réservé, `filterable: false` / `returned: false` (choix du propriétaire pour les deux derniers)

**Décision : option (a), une seule règle.** Toute clé de `fields` qui n'est pas un **champ de document déclaré au schéma** est refusée en `invalid_request` (400), avec un message qui nomme le champ, le **niveau attendu** (`document_fields`), le **chemin JSON** (`$.documents[0].fields.<nom>` ou `$[ligne].fields.<nom>`) et, pour un fichier, le **numéro de ligne**. La valeur n'apparaît jamais dans le message.

- Un **champ de passage** n'est pas un champ de document : ADR-0030 pose qu'un nom désigne un seul niveau, et `docs/04:42` refuse déjà une faute de schéma en `invalid_request` avec un message qui nomme le champ et les niveaux. Refuser ici évite qu'un patch de document prétende modifier une métadonnée héritée par tous les passages.
- `docs/05-api.md:72` ne couvre que `PATCH /v1/indexes/{index}` ; la règle est **étendue** au patch de documents, ce qui est le point de cet ADR. `invalid_filter` n'est pas employé : il renvoie l'appelant à la grammaire des filtres (`docs/05-api.md:32`).

**Noms réservés (`doc_id`, `doc_hash`) — décision : option (a).** Ils sont refusés en `invalid_request` par la règle unique ci-dessus, avec un message qui dit « nom réservé ».

Aucune règle supplémentaire n'est nécessaire : ces deux noms ne sont pas des champs du schéma, donc la règle unique les attrape déjà. Le message précise seulement le motif, ce qui coûte une phrase et évite un diagnostic à l'appelant.

Alternatives écartées : **accepter et ignorer silencieusement** — un `doc_id` dans `fields` laisserait croire à un renommage de document, qui n'existe pas, et le patch écrirait un champ fantôme, invisible partout ; c'est exactement ce qu'ADR-0031 a refusé pour la configuration. **Code d'erreur dédié**, par exemple `reserved_field` — un code de plus dans `docs/05` et dans les analytics pour un cas que la règle unique couvre.

**Champs `filterable: false` ou `returned: false` — décision : option (a), accepter.** La valeur est stockée ; elle ne participe simplement ni aux filtres (`docs/05-api.md:117` : un champ `filterable: false` donne `invalid_filter`) ni au payload d'un document si `returned: false`.

C'est le comportement de l'attache, qui n'interdit pas de fournir une valeur pour un champ non filtrable : un patch qui refuserait divergerait du seul autre chemin d'écriture. Et `filterable` et `returned` sont **réglables** (`docs/04:51`, les bitmaps sont reconstruites) : refuser obligerait à patcher la configuration avant la donnée, alors que la donnée est légitime.

Alternative écartée : **refuser en `invalid_request`** — bloque une écriture légitime et impose un ordre d'opérations qui n'est écrit nulle part.

Une phrase suffit, et elle est écrite dans `docs/04` et `docs/05` : un champ `returned: false` patché reste invisible dans `GET /documents`, et un champ `filterable: false` patché n'est pas filtrable. Un champ **ajouté par un `PATCH` de configuration** est nullable (`docs/04:52`) : le patch peut donc y poser `null` (décision 3) comme une vraie valeur.

### Décision 6 — valeur mal typée (NON-COUVERT, ambiguïté réelle)

**Décision : option (a).** Une valeur qui ne correspond pas au type du champ est refusée en **`invalid_request`** (400), avec un message qui nomme le champ et le **type attendu**, et **jamais la valeur**.

- `docs/05-api.md:31` : `invalid_request` = « Corps invalide, paramètre hors bornes ». Un champ de document n'est pas un filtre.
- `docs/05-api.md:32` donne « valeur mal typée » à `invalid_filter`, mais **dans la grammaire des filtres**. Employer ce code renverrait l'appelant à une grammaire qu'il n'utilise pas.
- Précédents d'un refus de schéma en `invalid_request` : `docs/04:42`, `docs/04:64`.
- La fiche exige qu'une valeur mal typée soit « refusée sans aucun effet » (`docs/phases/P3-cycle-de-vie.md:71`) : le refus a lieu **avant** toute écriture, sur le fichier entier (décision 4, atomicité).

Le contrôle suit le type déclaré (`keyword`, `integer`, `float`, `date` au format `YYYY-MM-DD`, `bool`) et le domaine de `date`, comme `docs/04:38`.

### Décision 7 — limite de taille du corps et du fichier (choix du propriétaire)

Aucune source ne plafonne un patch. `docs/05-api.md:240` ne plafonne que le corps du lot en ligne : 32 Mo et 10 000 passages. Un patch n'a pas de passage à compter.

**Décision : option (a), 32 Mo et 10 000 entrées**, refus en `payload_too_large` (413), **mêmes bornes pour le fichier de la CLI** que pour le corps HTTP.

Les deux chemins partagent une forme d'entrée (décision 1) : leur appliquer deux bornes créerait un fichier accepté par la CLI et refusé par la route, ou l'inverse. Les valeurs reprennent celles du lot en ligne, l'unité passant de passage à document, et un patch de 10 000 documents est déjà une opération d'exploitation, pas un geste courant. Le contrôle se fait **avant** toute écriture, et pour la CLI **avant** de charger l'ensemble du fichier.

Alternatives écartées : **32 Mo seulement, sans compter les entrées** — un fichier de plusieurs millions de lignes courtes passerait sous 32 Mo et ferait un patch de plusieurs minutes sans borne de travail, alors que le compteur d'entrées est le garde-fou utile. **Aucune borne propre, la limite générale du corps** — la limite de `docs/05` est celle du lot en ligne ; s'en remettre à elle laisserait le patch sans plafond documenté, alors que `payload_too_large` existe et que P6 devra brancher la route sur une valeur écrite.

### Décision 8 — rapport, unité, `progress` et lieu de persistance (choix du propriétaire pour la persistance)

**Rapport : `{ updated, unchanged, generation }`.**

- `updated` : document dont au moins un champ change après fusion ;
- `unchanged` : document dont les champs, en JSON canonique, sont identiques — y compris un `fields: {}` ;
- `generation` : la génération visible **après** l'appel (`G+1` si au moins un document change, `G` sinon, décision 9) ;
- l'unité est le **document**, seul cas possible : un patch ne peut ni ajouter, ni remplacer, ni supprimer un document, donc les six comptes d'ADR-0041 se réduisent à deux. Aucune clé de passage n'apparaît.
- il n'y a **pas** de clé `ignored` : le refus d'un `doc_id` inconnu est atomique (décision 4).

**`progress`.** `docs/05-api.md:229` disait que `progress.done` et `progress.total` comptent les **passages** traités. C'est inapplicable à un patch, qui n'a aucun passage : la phrase se lit désormais « les passages pour `attach_batch` et `online_batch`, les **documents** pour `patch_documents` ». `progress.step` garde son rôle.

**Lieu de persistance — décision : option (a), rien n'est persisté.** Le rapport d'attache est persisté dans `batches.report` (`docs/02-architecture.md:92`, `docs/05-api.md:230`). Un patch n'a **pas** de ligne `batches` : pas de `batch_id`, pas de `manifest_blake3`, donc pas d'idempotence de rejeu par identité. Il n'a donc aucun emplacement de persistance.

En P3, la CLI rend le rapport sur sa sortie standard. En P6, le job porte `result.report` en mémoire, pour la durée de vie du job ; `docs/05-api.md:230` se lit alors « rapport persisté de l'attache » au sens strict.

Motif : en P3 il n'existe que la CLI, et une persistance n'aurait aucun lecteur. En P6, un rapport de patch est un compte de documents sans valeur d'archive, et la conception ne définit aucune table de jobs. Ajouter une table ou une ligne `batches` pour un rapport que personne ne relit serait un schéma de plus à maintenir et à compacter. Le rejeu est de toute façon sans effet par le contenu (décision 9), sans identité à mémoriser.

Alternatives écartées : **écrire une ligne `batches` avec un `batch_id` synthétique** — `batches` porte l'idempotence d'attache (INV-10), y écrire une opération qui n'attache rien brouille la lecture de la table, et le `manifest_blake3` n'aurait pas de sens. **Nouvelle table SQLite, par exemple `patches`** — un changement de schéma pour un rapport sans lecteur, qui demanderait son propre ADR et une migration d'époque. **Fichier JSON dans le dossier de l'index** — un fichier de plus à nettoyer et à versionner dans l'époque, sans lien avec la génération : la reprise devrait le connaître, et rien ne le lirait.

### Décision 9 — ce que le commit écrit : identité, `updated_generation`, manifeste, patch sans effet (choix du propriétaire pour le dernier point)

**Décision retenue sur chaque point.**

- **`doc_hash` inchangé** : un patch de champs de document ne modifie pas `doc_hash` (voir le fait corrigé ci-dessus ; ADR-0055 décision 3, `docs/04:107`, `docs/04:209`).
- **`updated_generation = G+1` est posé sur chaque ligne `documents` patchée.** La colonne est nommée une seule fois dans la conception (`docs/02-architecture.md:94`) et n'est **jamais définie**. Elle l'est ici : **génération de la dernière écriture de la ligne `documents`**. Ce n'est pas un détail : le rafraîchissement des champs hérités par les passages sélectionne les documents sur cette colonne (`crates/crible-store/src/filters_index.rs:1064-1075`, `WHERE d.updated_generation = ?1`). Si le patch ne la pose pas, les bitmaps ne bougent pas et le premier critère de la fiche échoue.
- **Transaction** : `documents` (champs et `updated_generation`) et `meta.generation = G+1` seulement. Pas de `segments`, pas de `batches`, pas de `passages` (`docs/02-architecture.md:112`, `:117`). Le point de commit reste le commit SQLite (ADR-0016).
- **Après le commit** : étape 5, le manifeste `G+1`, qui reprend **les mêmes segments que G** — aucun nouveau segment, aucune écriture de segment ; puis étape 6, bitmaps en copie sur écriture et publication de l'instantané (`docs/02-architecture.md:114-115`, INV-02, INV-03). L'étape 4 (tantivy) n'a pas lieu : le texte ne change pas. La forme `segments` du manifeste est celle de `docs/02-architecture.md:85` ; un patch n'y ajoute rien.
- **Aucun nouveau segment, aucun iid, aucun vecteur** : les critères de la fiche sont satisfaits par construction.

**Patch sans effet — décision : option (a), aucune génération nouvelle.** Si aucun document ne change, `meta.generation` reste `G`, aucun manifeste n'est écrit, aucun instantané n'est publié ; le rapport donne `updated: 0` et `generation: G`.

Motif : rien n'est visiblement changé, donc rien ne justifie une génération. Une génération de contenu identique ferait croître `manifests/` et forcerait un nouvel instantané pour rien, et rendrait un rejeu de fichier non idempotent **en effet** alors que le patch n'a pas d'identité de rejeu (décision 8). Coût accepté : la génération seule ne dit pas si l'appel a été appliqué ; `updated: 0` le dit, et c'est le rôle du rapport.

Alternative écartée : **toujours `G+1`, même sans aucun changement** — une trace de l'appel dans la génération, au prix d'un manifeste, d'un instantané et de bitmaps recopiés pour un contenu identique ; un fichier rejoué en boucle ferait croître l'historique sans fin.

## Conséquences

- **Positives** :
  - un contrat unique, écrit avant le code, que la CLI de P3 et la route de P6 partagent sans divergence : une forme d'entrée, une règle de fusion, un rapport, un code par cause de refus ;
  - `updated_generation` acquiert une définition, et le critère « les filtres reflètent la nouvelle valeur » devient démontrable au lieu de dépendre d'une colonne non documentée ;
  - le patch reste une écriture légère : pas de segment, pas de tantivy, pas de `batches`, ni vecteur ni iid touché ;
  - un rejeu de fichier est sans effet, par le contenu et non par une identité persistée ;
  - les refus sont atomiques et diagnostiquables : le message nomme le champ, son niveau, son chemin JSON et, pour un fichier, la ligne.
- **Négatives / coûts** :
  - **un patch n'est pas durable** : une attache ultérieure du même document avec un `doc_hash` identique écrase les champs patchés (`docs/04:169`). Un producteur qui republie régulièrement rétablit ses valeurs ; c'est accepté, et c'est le prix de « les champs de document ne font pas partie du `doc_hash` » ;
  - un `doc_id` inconnu fait échouer tout le fichier : un fichier rejoué sur un index où des documents ont été supprimés doit être filtré avant rejeu ;
  - un patch sans effet ne laisse aucune trace dans la génération : seul le rapport (sortie standard de la CLI, ou `result.report` du job) atteste l'appel ;
  - `docs/05-api.md:229` a été reformulée, `docs/05-api.md:88` restreinte aux passages, et `docs/04:229` précisée : trois textes de conception à corriger ensemble, sinon l'écart reparaît sous une autre forme ;
  - `docs/07:15` liste désormais `patch` — et `ingest`, absent depuis P2.
- **À surveiller** :
  - **l'écrasement par une attache ultérieure** : si un producteur se plaint qu'une valeur patchée disparaît après une republication, la question n'est pas le patch mais le contenu du `doc_hash` (ADR-0055 décision 3), donc un nouvel ADR ;
  - **la dérive de payload déjà admise** : `docs/02-architecture.md:141` accepte qu'un résultat rende des champs de document plus récents que la génération du snapshot ; un patch en est une cause de plus, et INV-03 ne porte pas sur les payloads (ADR-0043) ;
  - **deux processus sur le même index** : `crible patch` en CLI pendant qu'un serveur écrit. Chacun passe sa propre file d'écriture ; l'exclusion entre processus n'est pas donnée par `busy_timeout`, qui n'assure que la vivacité (`overloaded` après 1 000 ms, ADR-0038). Faire tourner la CLI sur un index servi reste une **règle d'exploitation**, à écrire dans `docs/05` ;
  - un pic de `invalid_request` sur des clés inconnues après l'ajout d'un client indiquerait un besoin d'assouplir la décision 5, comme ADR-0031 le prévoit pour la configuration ;
  - un rapport de patch qu'un consommateur voudrait relire après coup rouvrirait la décision 8.

## Application

Appliqué le 2026-10-25, dans un commit qui cite `ADR-0060` :

| Fichier | Modification |
|---|---|
| `docs/02-architecture.md` | définir `updated_generation` (génération de la dernière écriture de la ligne `documents`) et préciser ce que la transaction d'un patch écrit, dans « Générations et point de commit » (`:104-117`) ; noter qu'un patch ne produit aucun segment |
| `docs/04-donnees-et-ingestion.md` | nouvelle section « Patch de métadonnées » : forme de l'entrée (CLI et HTTP), fusion, `null` valeur posée, atomicité, `doc_hash` inchangé, écrasement par une attache ultérieure ; préciser la portée du « un seul type » (`:229`) |
| `docs/05-api.md` | corps de `PATCH /v1/indexes/{index}/documents` et limites (`:58`, `:240`), rapport et `progress` (`:220-231`), `not_found` pour un document inconnu (`:36`), reformuler `:88` (« rien n'a changé **dans les passages** ») |
| `docs/07-stack-et-deploiement.md` | `:15` : ajouter `patch` et `ingest` à la liste des commandes de la CLI |
| `docs/suivi.md` | ligne « Écarts à la conception » citant `ADR-0060`, **à ajouter par l'orchestrateur** |
| `docs/08-invariants-et-tests.md` | rien d'obligatoire : INV-03 (atomicité) et INV-07 (JSON → bitmaps) couvrent déjà l'effet d'un patch |

La fiche `PATCHER-METADONNEES` garde son périmètre ; seule sa section « À lire » gagne `ADR-0060`. Aucune dépendance nouvelle.

**PATCHER-METADONNEES reprend à l'acceptation.** Les neuf points étaient bloquants ; aucun ne se tranchait par une lecture de la conception.

## Alternatives écartées

| Décision | Alternative | Pourquoi écartée |
|---|---|---|
| 1 | (b) réutiliser le type du document en ligne, `passages` devenant facultatif | Un patch n'a pas de passage par construction ; rendre `passages` facultatif laisserait `POST /documents` accepter un document sans passage, ce que `docs/04:113` interdit (« un document a au moins un passage ») |
| 1 | (c) un corps JSON unique pour la CLI aussi, au lieu de JSONL | Contredit l'usage posé par ADR-0055 décision 1 pour `crible ingest`, et charge le fichier entier en mémoire alors que la décision 7 borne les entrées |
| 1 | (d) une option `--models` sur `crible patch` | Un patch n'encode rien : le modèle n'est pas lu, et l'option laisserait croire à un encodage |
| 2 | (b) remplacer `fields` en entier | Un `PATCH` deviendrait un `PUT` : un champ omis disparaîtrait, et une faute d'omission effacerait des métadonnées sans erreur |
| 2 | (c) fusion avec retrait explicite | Réintroduit la distinction absent / null refusée par ADR-0035 (voir décision 3) |
| 3 | (b) `null` retire la clé | ADR-0035 a refusé de distinguer un champ absent d'un null explicite ; le retrait créerait une asymétrie entre les filtres et `GET /documents` |
| 3 | (c) refuser `null` | `docs/04:40` accepte null sur tous les champs, et l'attache l'accepte : un patch plus strict que l'attache serait une règle de plus |
| 4 | (c) refuser seulement un document retiré, accepter un `doc_id` jamais vu | Indisponible : ADR-0041 supprime la ligne `documents`, donc le moteur ne peut plus distinguer les deux cas |
| 4 | (d) appliquer les lignes valides et rendre les autres en erreur | Contredit INV-03 et `docs/02:106` : un refus partiel rendrait une génération à moitié appliquée, ou forcerait plusieurs générations pour un fichier |
| 5 | (b) refus en `invalid_filter` pour un champ non filtrable | Confond l'écriture et la lecture : `invalid_filter` appartient à la grammaire des filtres, pas au corps d'un patch |
| 6 | (b) `invalid_filter` pour une valeur mal typée | `docs/05-api.md:32` range « valeur mal typée » dans la grammaire des filtres ; un champ de document n'est pas un filtre |
| 7 | (b) ou (c) : borne partielle ou aucune | Voir décision 7 : deux chemins, deux bornes, ou aucun plafond écrit pour une route que P6 doit brancher |
| 8 | (b), (c) ou (d) : persister le rapport | Voir décision 8 : `batches` porte l'idempotence d'attache, une table nouvelle est un changement de schéma, un fichier d'époque aurait la reprise à connaître — pour un rapport sans lecteur |
| 9 | (b) toujours `G+1` | Un manifeste et un instantané pour un contenu identique, et un rejeu non idempotent en effet |
| 9 | (c) ne pas définir `updated_generation`, laisser l'implémentation choisir | Le rafraîchissement des champs hérités (`filters_index.rs:1064-1075`) en dépend : sans définition, les bitmaps peuvent ne pas bouger et le critère de la fiche échoue silencieusement |

## Références

- `docs/phases/P3-cycle-de-vie.md:60-71` : fiche PATCHER-METADONNEES ; `docs/phases/P6-serveur-et-api.md` : routes et jobs.
- `docs/02-architecture.md` : `:85` (manifeste), `:94` (table `documents`, `updated_generation`), `:104-117` (générations, étapes, « modifier des métadonnées : étapes 3, 5 et 6 »), `:136` (un écrivain par index), `:141` (dérive de payload admise), `:151-152` (republication identique, métadonnée seule).
- `docs/04-donnees-et-ingestion.md` : `:38-42` (types, `filterable`/`returned`, null, ADR-0030), `:51-52` (réglages et ajout d'un champ nullable), `:105-113` (document, passage, `doc_hash`, au moins un passage), `:169-176` (sémantique d'attache, atomicité), `:202-213` (rapport d'attache), `:229-231` (document en ligne, fichier d'entrée).
- `docs/05-api.md` : `:18` (INV-01), `:29-44` (codes), `:58` (`PATCH documents`), `:72` (clé inconnue), `:88` (`doc_hash`), `:117` (`filterable: false`), `:216-232` (jobs, rapport, `progress`, `kind`), `:240` (limites du lot en ligne).
- `docs/06-observabilite.md:26` : `patch_documents` parmi les opérations comptées.
- `docs/07-stack-et-deploiement.md:15` : commandes de la CLI.
- `docs/08-invariants-et-tests.md:9` (INV-01), `:11` (INV-03), `:15` (INV-07).
- `crates/crible-store/src/filters_index.rs:1064-1075` : rafraîchissement des champs hérités, `WHERE d.updated_generation = ?1`.
- ADR-0016 (point de commit), ADR-0019 (JSON canonique), ADR-0030 (un nom, un niveau), ADR-0031 (clé inconnue refusée), ADR-0035 (valeurs nulles), ADR-0038 (`busy_timeout`), ADR-0041 (sémantique d'attache, six comptes), ADR-0043 (payloads hors INV-03), ADR-0055 (lot en ligne, octets du `doc_hash`), ADR-0056 (portée de la règle « un écrivain par index »).
