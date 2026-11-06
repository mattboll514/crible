# ADR-0139 — Protocole de la campagne d'import à l'échelle réelle (IMPORTER-BASE-COMPLETE)

- **Statut** : Accepté — décidé par le propriétaire le 2026-11-06 (« ADR-0139 accepté »), sans réserve
- **Date** : 2026-11-06 (proposition et décision le même jour)
- **Décideurs** : propriétaire
- **Proposé par** : `@greffier`, à la demande de l'orchestrateur, pour la tâche `IMPORTER-BASE-COMPLETE` (P10), d'après les verdicts de `@architecte` du 2026-11-06
- **Remplace** : —

## Contexte

La tâche `IMPORTER-BASE-COMPLETE` (P10) doit charger la base Legifacile complète dans une instance Crible, puis refaire MES-07 et MES-09 à l'échelle réelle (fiche `docs/phases/P10-validation-a-l-echelle-reelle.md`, section `### IMPORTER-BASE-COMPLETE`, `:88-103`). La campagne ne peut pas démarrer telle quelle : quatre points de méthode ne sont tranchés par aucun document accepté. `@architecte`, interrogé par l'orchestrateur le 2026-11-06, a rendu `CONFORME` sur la forme de l'instance, l'attache, l'instantané et le rapport, et `NON-COUVERT` sur le contrôle d'espace, les répétitions de MES-07, la méthode de MES-09 et le périmètre de la reconstruction lexicale. Cet ADR fixe ces quatre points. Il ne mesure rien et ne modifie pas la fiche.

### Les faits

- **Les lots sont complets et plus gros que la conception.** Six lots `replace_all` sont dans `data/legifacile-full/`, soit **6 660 239 documents** et **9 600 105 passages** pour **18,6 Go**, comptes égaux à pgvector (`docs/suivi.md`, journal du 2026-11-06 ; `EXPORTER-BASE-LEGIFACILE`). La conception chiffre **8,58 M passages** (`docs/02-architecture.md:230`), soit environ **12 %** de moins. Le critère de la fiche tranche : « les comptes par index sont égaux à ceux de pgvector, ou les écarts sont expliqués » (`:98`), repris par ADR-0129, décision 7, point 2. L'écart de 12 % doit être écrit dans le rapport. Corriger `docs/02:230` serait une décision du propriétaire, non bloquante.
- **L'attache rend de nouveau la main après son point de commit.** La correction de `CORRIGER-BLOCAGE-APRES-COMMIT` (2026-11-06) a supprimé le blocage de `FieldData::clear`. Pour `jorf` (1 833 500 passages), le point de commit tombe à **174 s**, puis le processus rend la main en **≤ 92 s** (`docs/suivi.md:1297`). Pour `decisions` (3 589 776 passages), la mesure de la tâche donne **412 s** jusqu'au commit, puis **≤ 227 s** (`docs/suivi.md:743` ne cite que la borne ≤ 227 s ; les 412 s viennent de la mesure de `CORRIGER-BLOCAGE-APRES-COMMIT`, pas d'un document de conception). Avant la correction, `jorf` était encore vivant à 600 s et arrêté par `SIGTERM` (fiche `:61`).
- **L'espace disque est tendu.** L'hôte a **107 Gio libres** le 2026-11-06 (`docs/suivi.md:1298`). Le pic extrapolé de la campagne vaut environ **105 Go**, soit **≈ 118 Go** après le facteur 1,12 des 9,6 M passages ; il n'est pas mesuré (ADR-0129, tableau du pic disque, `:50-59` ; `docs/suivi.md:741`). Le budget d'ADR-0129 additionne les lots (≈ 21 Go), l'index (≈ 28 Go), l'instantané de transfert (≈ 28 Go) et la compaction simultanée (+ 28 Go).
- **La VM Docker a ses propres bornes.** Docker Desktop est réglé à **12 Gio de RAM**, **8 cœurs** et un disque de VM plafonné à **200 Gio** ; le volume `crible-mes-p8-data` (**32,9 Go**) est encore présent, et deux corbeilles (24 Go et 25 Go) attendent leur vidage (`docs/suivi.md`, journal du 2026-11-06, `:1294` et `:1298`).
- **Les agents ne suppriment rien définitivement.** La table « Données de mesure conservées » de `docs/suivi.md` (`:746-765`) dit que les agents déplacent vers la corbeille de l'hôte et que le propriétaire supprime. Le volume Docker et l'image des harnais sont « à retirer par le propriétaire lui-même » (`:764`). Aucun agent n'exécute `docker volume rm`, `docker image rm` ni un vidage de corbeille.
- **Les lots sont conservés jusqu'à la fin de la campagne**, et les piles d'attache de `verif/` servent à `CORRIGER-BLOCAGE-APRES-COMMIT` (`docs/suivi.md:765`). Les six index jetables de `verif/indexes/` ont déjà été mis à la corbeille le 2026-11-06 (`:1294`) ; ils ne sont pas nécessaires à la campagne.

### Ce que les documents laissent ouvert

1. **Le contrôle d'espace n'a ni seuil ni lieu.** La fiche demande « un contrôle d'espace disque avant la campagne » (`:92`) et « l'espace disque libre reste suffisant pendant l'import et une compaction simultanée » (`:100`), sans chiffre. La contrainte liante est l'**hôte** : le disque de la VM est un fichier de l'hôte, et 107 Gio libres sont sous le pic de 118 Go. La « compaction simultanée » d'ADR-0129 (`:57`) est une **réservation d'espace**, pas une compaction lancée pendant l'import : ADR-0017 suspend l'écrivain pendant une compaction, et ADR-0116 n'admet qu'un écrivain par index. Une compaction concurrente d'une attache est donc exclue.
2. **MES-07 n'a pas d'index ni de répétitions pour l'échelle réelle.** La mesure de P3 (`docs/mesures/MES-07-duree-compaction.md`, campagne du 2026-10-25) lance `crible compact` à la main, et non par `dead_ratio` (`:44`), sur `articles` et `synth-large`. Son protocole clone l'index en APFS (`cp -cR`) pour **5 répétitions** (`:60`) : c'est impossible dans la VM Linux, et 5 copies ne tiennent pas dans le budget. À l'échelle réelle, le ratio de morts vaut **0** après un `replace_all` ; un seul lot `replace_all` donne **un segment source** (`:164-166`), et la compaction écrit des segments d'au plus 1 M passages (`docs/02-architecture.md:202`).
3. **MES-09 n'a pas de méthode avec six index ouverts.** Son protocole est fixé par ADR-0077, décision 5 : trois états **(a)** index ouvert, **(b)** après une passe de requêtes sans session ONNX, **(c)** avec la session fp32, rapportée à part. `docs/08-invariants-et-tests.md:74` la définit « par index ». Aucun document ne fixe la RAM : ADR-0129, décision 4, exige seulement d'indiquer la RAM de la VM dans le rapport. MES-09 mesure **1,10 GiB d'empreinte physique par index** à 8,6 M passages, dont **787 MiB de codes binaires** (campagne du 2026-10-27, `docs/mesures/MES-09-ram-reelle.md:95` et `:207`), et **1 147 MiB d'anonyme à l'ouverture** en conteneur (campagne du 2026-11-03, `:330` et `:368`). Une instance à six index n'est pas mesurée. L'extrapolation de `@architecte` d'environ **9 GiB anonymes** ajoute 0,92 Go de codes aux 1,1 GiB par index, qui les contiennent déjà : elle n'est pas retenue comme un chiffre mesuré, seulement comme un ordre de grandeur à vérifier.
4. **La reconstruction lexicale n'a pas de périmètre.** Le moyen est fixé : `crible rebuild-lexical` (`docs/07-stack-et-deploiement.md:15`) ou la première reconstruction à l'ouverture en écriture (ADR-0087, ADR-0088). La cible de **60 s** d'ADR-0047 se lit **par index** : « tout index dont le redémarrage dépasserait 60 s » (`:66`). Les six configs de `docs/legifacile/configs/` ont `lexical.enabled: true`, mais aucun document ne choisit les index à mesurer.

### Ce qui est déjà conforme

- `/data` en volume nommé (`docs/07:138`, ADR-0135 décision 3) ; `/exchange` monté depuis l'hôte pour les lots, exports et instantanés (`docs/07:140`).
- Attache par `POST /v1/indexes/{index}/batches` (`docs/05-api.md:68`, ADR-0107), avec le lot sous `exchange/inbox/<nom>`, un seul composant de chemin, sans lien symbolique (`docs/05:173-175`). `?wait=true` rend `202` à l'échéance de 60 s, puis le client suit `GET /v1/jobs/{id}` (ADR-0067, décision 2) ; la progression est publiée par ADR-0134. La CLI reste possible avec le serveur arrêté (ADR-0116, décision 2a), mais l'API garde l'instance debout et donne `progress`.
- Une instance, six index (ADR-0002, ADR-0009, `docs/legifacile/index-legifacile.md`).
- Instantané par index vers `exchange/snapshots/<index>-e<E>-g<G>/` (ADR-0064, décision 3), hors ligne (ADR-0065, décision 1), avec `--index` pour `crible snapshot` (ADR-0065, décision 6), et une copie qui peut vivre sur un autre volume (`:97`).
- MES-07 et MES-09 sont des mesures numérotées : `@mesureur` les exécute et écrit dans `docs/mesures/`. MES-09 porte déjà des sections datées (2026-10-27 et 2026-11-03) ; l'import, qui n'est pas une MES-xx, suit le modèle de `docs/mesures/routes-en-conteneur-2026-11-03.md`.

### Corrections de citation faites pour cet ADR

- ADR-0135, décision 2, porte sur l'**échelle** (`synth-large`, échelle réelle en P10), pas sur le conteneur. L'exécution en conteneur vient de la fiche `:93` et d'ADR-0129, décision 4.
- Les **31 Go** cités pour une copie de `decisions` valent pour l'**instance entière** à 9,6 M passages (28 Go à 8,58 M, ADR-0129 `:55`). L'extrapolation proportionnelle donne **≈ 12 Go** pour les 3 589 776 passages de `decisions`.
- `crible restore` n'a **pas** d'option `--index` : le nom de l'index cible vient de `snapshot.json` (ADR-0065, décision 3 et 6 ; `docs/04:360`). Une « restauration sous un autre nom » est donc impossible ; une répétition par restauration se fait dans un **dossier de données distinct**, avec le même nom d'index.

## Décision

**La campagne suit le protocole ci-dessous. Quatre points sont tranchés — contrôle d'espace, MES-07, MES-09, reconstruction lexicale — et les autres sont des rappels de documents acceptés.** Rien ne démarre avant que cet ADR soit `Accepté` et que le propriétaire ait libéré l'espace.

### Décision 1 — Contrôle d'espace disque : mesure sur l'hôte, départ à 130 Go, arrêt à 40 Go

- **Lieu de mesure.** L'espace est mesuré sur l'**hôte**, par `df -h /` et `df -H /`, sur le volume APFS qui porte le disque de la VM. C'est la contrainte liante : le disque de la VM est un fichier de l'hôte. Le plafond de **200 Gio** de la VM est une **seconde borne**, relevée par `df` dans la VM et par le réglage de Docker Desktop ; les deux chiffres sont publiés.
- **Seuil de départ.** Pas de départ sous **130 Go libres** sur l'hôte, soit 118 Go × 1,1. Le seuil est lu en **Go décimaux**, l'unité du budget d'ADR-0129 ; `df -h` (Gio) est publié en regard. À 107 Gio, l'hôte est sous le pic de 118 Go : la campagne est bloquée tant que le propriétaire n'a pas libéré l'espace.
- **Seuil d'arrêt.** L'import s'arrête sous **40 Go libres** sur l'hôte. L'arrêt se fait **entre deux lots** : aucun nouveau lot n'est soumis. Un job en cours n'est pas tué ; s'il échoue faute d'espace, l'échec est rapporté, et l'index est rouvert pour vérifier la dernière génération commitée (INV-04).
- **Fréquence.** `df` est relevé avant chaque attache, après chaque attache, après chaque compaction et à la fin de la campagne. Les sorties brutes datées vont dans le rapport d'import.
- **Réservation de compaction.** Les **+ 28 Go** d'ADR-0129 (`:57`) sont une **réservation d'espace**, comptée dans le pic de 118 Go. Elle n'est pas une compaction lancée pendant l'import : ADR-0017 suspend l'écrivain pendant une compaction, et ADR-0116 n'admet qu'un écrivain par index. Pendant MES-07, la compaction d'un index a besoin au plus de deux fois l'espace de cet index (`docs/02:210`), ce qui reste sous la réservation pour `decisions` et `jorf`.
- **Ce qui n'est pas décidé ici.** Aucune suppression n'est faite par un agent : le propriétaire vide les corbeilles, retire `crible-mes-p8-data` et l'image `crible-mes-p8-harness:dev` (ADR-0129, table du suivi). Le plafond de 200 Gio n'est pas changé.

### Décision 2 — MES-07 à l'échelle réelle : `decisions` puis `jorf`, une compaction en place, sans clone APFS

- **Index mesurés.** `decisions` (3 589 776 passages) puis `jorf` (1 833 500). `decisions` est le plus gros index de l'instance ; `jorf` est un index intermédiaire. Les deux dépassent 1 M passages, donc la compaction y produit plusieurs segments de sortie. La mesure se fait **en conteneur** (fiche `:93`, ADR-0129 décision 4), avec la RAM de la VM déclarée.
- **Provocation.** `crible compact` est lancé **à la main**, comme dans MES-07 (`:44`), et non par `dead_ratio`. Le ratio de morts réel vaut **0** après un `replace_all` ; le rapport le déclare.
- **Répétitions, option (a) — recommandée.** Une compaction par index mesuré, **en place** sur l'index importé, **avant l'instantané**. Aucune copie, donc aucune suppression : c'est la seule option compatible avec la règle de non-suppression et avec le budget disque. Une seconde compaction en place est **techniquement possible** — Crible ne réutilise aucun segment et écrit une nouvelle époque (`docs/02:208`) —, mais elle **ne rejoue pas le même état d'entrée** : la première passe laisse des segments d'au plus 1 M passages (`docs/02:202`), alors que l'import a laissé un seul segment source (`MES-07:164-166`). Elle est donc rapportée comme un **cas distinct** (entrée multi-segments), jamais comme une répétition. Si elle n'est pas exécutée, le rapport donne **une mesure par index**, déclarée comme telle.
- **Répétitions, option (b) — écartée.** `crible snapshot` puis `restore` dans un **dossier de données distinct**, avec le même nom d'index (`restore` n'a pas d'option `--index`). Chaque répétition coûte l'instantané **et** la copie restaurée, soit environ **24 Go pour `decisions`** (≈ 12 Go chacun) ; les agents ne peuvent pas les supprimer, et le budget de 118 Go ne les porte pas. Le clone APFS (`cp -cR`, 5 répétitions) du protocole de P3 est impossible dans la VM Linux et reste hors du protocole.
- **Rapport.** Durée de la compaction, nombre de segments en entrée et en sortie, octets écrits, espace temporaire, génération et époque avant et après. La comparaison à MES-07 du 2026-10-25 est déclarée comme **non directement comparable** : 8,6 M passages synthétiques et 10 % de morts en natif, contre des données réelles, 0 mort et un conteneur.
- **Ce qui n'est pas décidé ici.** Aucune cible de durée de compaction, aucun changement de code, aucune modification du protocole de compaction.

### Décision 3 — MES-09 à l'échelle réelle : trois états, instance puis index seul, RAM de la VM de 12 Gio

- **Protocole.** Les trois états d'ADR-0077, décision 5 : **(a)** index ouvert et instantané chargé, **(b)** après une passe de requêtes à vecteur fourni, sans session ONNX, **(c)** avec la session fp32, rapportée **à part**. La mesure se fait en conteneur, avec `memory.current` et `memory.stat` publiés avec le RSS (ADR-0135, décision 7).
- **Deux séries.** Une série avec les **six index ouverts** (l'instance), puis une série **par index seul**, puisque MES-09 est définie « par index » (`docs/08:74`). Pour la série par index, le serveur est arrêté et les cinq autres dossiers d'index sont **déplacés** de `/data/indexes/` vers `/data/staging-mes09/` — un déplacement dans le même volume, pas une suppression —, puis remis en place. Si le propriétaire refuse ce déplacement, la série par index se fait par `crible snapshot` puis `restore` dans un dossier de données distinct, et son coût disque est compté ; si aucune des deux méthodes n'est possible, seule la série d'instance est rapportée, et le rapport dit que la série par index n'a pas été faite.
- **RAM de référence.** Celle de la VM Docker, **12 Gio** réglés, **11,67 Gio** mesurés en P8 (`docs/mesures/routes-en-conteneur-2026-11-03.md:16`). Elle est indiquée dans le rapport (ADR-0129, décision 4). Le rapport donne une **RAM minimale** et une **RAM confortable** (ADR-0135, décision 5), à partir des états (b) et (c), et déclare que les 9 GiB extrapolés ne sont pas mesurés.
- **Conduite si la VM manque de mémoire.** En cas d'OOM, ou si `memory.current` s'approche de la limite, la série est arrêtée, les chiffres sont rapportés, et le propriétaire décide d'agrandir la VM. **Seul le propriétaire change les réglages de Docker Desktop.** Aucune limite `--memory` n'est posée sur le conteneur.
- **Ce qui n'est pas décidé ici.** Aucune valeur de RAM n'est fixée comme exigence ; la campagne mesure et recommande.

### Décision 4 — Reconstruction lexicale : les six index, comparés par index à 60 s

- **Périmètre.** Les **six index** de Legifacile. Chacun est mesuré par `crible rebuild-lexical`, **serveur arrêté**, en conteneur (fiche `:93`, ADR-0129 décision 4). Les six configs ont `lexical.enabled: true` (`docs/legifacile/configs/`).
- **Comparaison.** Chaque durée est comparée à la cible de **60 s par index** (ADR-0047 `:66`). La **somme** des six durées est rapportée, mais déclarée **non comparable** à 60 s : la cible est un budget de démarrage par index, pas un budget d'instance. La comparaison à l'extrapolation de 325 s de MES-06 n'est pas un résultat ; elle sert seulement de repère.
- **Si un index dépasse 60 s.** On applique la décision du propriétaire du 2026-10-29 (`docs/suivi.md:700` et `:1082`) : un ADR sera rédigé plus tard, pour relever la cible dans ce cas de reprise ou pour paralléliser l'écriture. **Rien n'est codé dans cette tâche** ; `WRITER_THREADS` reste à 1 (ADR-0087).
- **Rapport.** Durée par index, nombre de passages et de jetons analysés, taille de `lexical/`, et le total d'instance déclaré non comparable.
- **Ce qui n'est pas décidé ici.** Aucun changement de cible, aucun changement de code, aucune parallélisation.

### Décision 5 — Rappels conformes et déroulé de la campagne

Ces points ne sont pas de nouvelles décisions : ils appliquent des documents acceptés. Ils sont écrits ici pour que la campagne soit reproductible.

1. **Préalable.** Le propriétaire vide les corbeilles, retire le volume `crible-mes-p8-data` et l'image `crible-mes-p8-harness:dev`, et démarre Docker Desktop. Rien ne démarre avant que cet ADR soit `Accepté` et que l'hôte ait **≥ 130 Go libres**.
2. **Conteneur et volumes.** `/data` en **volume nommé neuf** (nom proposé `crible-p10-data`), `/models` en volume nommé, `/exchange` monté depuis l'hôte (`docs/07:138-140`). Chaque lot est monté **directement** sous `/exchange/inbox/<nom>`, en lecture seule, pour que `directory` soit un seul composant (ADR-0107 ; `docs/05:173`) ; monter `data/legifacile-full/` en bloc ne convient pas, car le chemin aurait deux composants.
3. **Ordre des index.** Du plus petit au plus gros : `dole` (7 653 passages), `sarde` (214 578), `titres` (1 477 883), `jorf` (1 833 500), `articles` (2 476 715), `decisions` (3 589 776). Un échec précoce coûte moins.
4. **Attache.** `POST /v1/indexes/{index}/batches` avec `{ "directory": "<nom>" }`, `?wait=true`, puis `GET /v1/jobs/{id}` (ADR-0107, ADR-0067 décision 2, ADR-0134). Les horodatages bruts du journal d'attache sont conservés, pour mesurer exactement le temps après le commit (idée du 2026-11-06, `docs/suivi.md:745`).
5. **Contrôle par index.** `crible inspect` par index, et `GET /v1/indexes/{index}` pour le nombre de segments (`docs/05:62`). Les comptes sont comparés à pgvector ; l'écart de **12 %** au chiffre de `docs/02:230` est expliqué dans le rapport. La durée et l'espace sont relevés.
6. **Mesures.** `@mesureur` exécute MES-07 sur `decisions` puis `jorf` (décision 2), puis MES-09 sur l'instance et par index (décision 3), puis `rebuild-lexical` sur les six (décision 4).
7. **Instantanés.** Serveur arrêté, six `crible snapshot` vers `/exchange/snapshots/<index>-e<E>-g<G>/`, avec `--index` (ADR-0064 décision 3, ADR-0065 décisions 1 et 6). Les tailles sont relevées ; le total attendu est d'environ **31 Go** à 9,6 M passages (28 Go × 1,12). L'instantané est pris **après** les compactions de MES-07 (`decisions` et `jorf`), et le rapport donne la génération et l'époque copiées.
8. **Rapports.** Un **rapport d'import daté** (durée, espace, `inspect`, comptes, instantanés), au modèle de `docs/mesures/routes-en-conteneur-2026-11-03.md`, qui n'est pas une MES-xx ; des **sections datées** ajoutées à MES-07 et à MES-09, avec leur JSON daté. Le suivi est mis à jour par l'orchestrateur.
9. **Interdits.** Aucune suppression définitive par un agent ; aucun changement de code ; aucun ADR accepté ni document de conception modifié par cette campagne.

## Conséquences

- **Positives** :
  - les quatre points `NON-COUVERT` sont tranchés avant la campagne, et la campagne devient reproductible ;
  - le seuil d'espace est explicite, mesuré au bon endroit (l'hôte) et doublé d'un seuil d'arrêt ;
  - la règle de non-suppression est respectée : aucune copie de répétition n'est créée pour MES-07 dans l'option retenue, et le propriétaire seul libère l'espace ;
  - MES-07 et MES-09 sont comparables à leurs campagnes antérieures, avec leurs différences déclarées (conteneur, 0 mort, données réelles, six index) ;
  - la reconstruction lexicale est comparée par index, ce qui est le sens de la cible d'ADR-0047.
- **Négatives / coûts** :
  - la campagne est **bloquée** tant que l'hôte n'a pas 130 Go libres : à 107 Gio (≈ 115 Go), il manque environ 15 Go si le seuil est lu en Go décimaux, et environ 25 Go s'il est lu en Gio ; le propriétaire doit vider les corbeilles et retirer le volume `crible-mes-p8-data` ;
  - la marge au-dessus du pic de 118 Go est de 12 Go seulement, et le pic n'est pas mesuré ;
  - MES-07 ne donne qu'une mesure par index, ou un cas distinct pour la seconde passe ; il n'y a pas de répétition statistique comme en P3 ;
  - la mémoire d'une instance à six index n'est pas encore mesurée : le dimensionnement reste une extrapolation jusqu'à la campagne ; la série par index demande un déplacement de dossiers ou des copies, et son coût est déclaré ;
  - l'instantané est pris après la compaction : il ne représente pas l'état brut de l'import, mais l'état compacté, ce que le rapport doit dire ;
  - les chiffres de latence en conteneur restent **indicatifs** ; la latence contraignante appartient à `MESURER-SUR-MAC-MINI` (ADR-0129).
- **À surveiller** :
  - l'espace libre sur l'hôte pendant chaque attache et chaque compaction ; un passage sous 40 Go arrête la campagne ;
  - `memory.current` et `memory.stat` pendant MES-09, en particulier avec les six index ouverts ;
  - le temps entre le point de commit et la fin du processus, sur les six index, avec les horodatages bruts ;
  - toute durée de reconstruction lexicale au-dessus de 60 s : elle déclenche l'ADR différé, pas un changement de code dans cette tâche ;
  - les comptes par index contre pgvector, et l'explication de l'écart de 12 %.
- **Application.** La fiche n'est pas modifiée par cet ADR. À l'acceptation, si le propriétaire le demande, `@greffier` appliquera la décision aux documents de conception concernés, en citant `ADR-0139` ; le corps de l'ADR accepté ne sera pas réécrit.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Mesurer l'espace seulement dans la VM | Le disque de la VM est un fichier de l'hôte : la VM peut afficher de la place alors que l'hôte n'en a plus. La borne de l'hôte est la contrainte liante ; le plafond de 200 Gio reste une seconde borne. |
| Démarrer à 107 Gio, sous le pic de 118 Go | Le pic n'est pas mesuré et la marge serait négative. Le seuil de 130 Go laisse 12 Go au-dessus du pic. |
| Lancer une compaction pendant l'import pour « tester » la réservation | ADR-0017 suspend l'écrivain pendant la compaction et ADR-0116 n'admet qu'un écrivain par index. La ligne « + 28 Go » est une réservation, pas une opération concurrente. |
| Répéter MES-07 par clone APFS (`cp -cR`, 5 fois) | Impossible dans la VM Linux, et 5 copies ne tiennent pas dans le budget. Le protocole de P3 n'est pas transposable. |
| Répéter MES-07 par `snapshot` + `restore` sous un autre nom | `restore` n'a pas d'option `--index` : le nom vient de `snapshot.json` (ADR-0065). Il faudrait un dossier de données distinct, avec le même nom ; chaque répétition coûte l'instantané et la copie, et les agents ne peuvent pas les supprimer. |
| Ne faire aucune compaction avant l'instantané | L'instantané de départ doit représenter l'état transféré ; compacter avant réduit l'époque et permet de mesurer MES-07 sur l'index réel. La compaction est sans perte de résultats (INV-09). |
| MES-09 seulement au niveau de l'instance | Ne sépare pas la mémoire par index, alors que MES-09 est définie par index (`docs/08:74`) et que la RAM d'un index est ce qui dimensionne la VM. |
| MES-09 seulement par index | Perd la mesure de l'instance à six index, qui est le cas réellement servi et le seul qui éprouve la somme des mémoires. |
| Prendre les 9 GiB extrapolés comme valeur de dimensionnement | L'extrapolation n'est pas mesurée et compte les codes binaires deux fois si elle ajoute 0,92 Go aux 1,1 GiB par index qui les contiennent déjà. Le rapport donne les chiffres mesurés et déclare l'inconnu. |
| Comparer la somme des six reconstructions lexicales à 60 s | La cible d'ADR-0047 est un budget de démarrage par index, pas une somme d'instance. La somme est rapportée comme non comparable. |
| Corriger le code si la reconstruction dépasse 60 s | La décision du propriétaire du 2026-10-29 renvoie à un ADR différé : relever la cible pour ce cas de reprise, ou paralléliser l'écriture. Rien n'est codé dans cette tâche. |
| Attacher par la CLI plutôt que par l'API | La CLI exige l'arrêt du serveur (ADR-0116) et ne donne pas `progress`. L'API laisse l'instance debout pour MES-09 et publie l'avancement (ADR-0134). |
| Monter `data/legifacile-full/` en bloc dans `exchange/inbox` | `directory` doit être un seul composant (ADR-0107) ; le chemin `<index>/<index>-full-v1` en a deux. Chaque lot est monté directement sous `inbox/<nom>`. |
| Faire supprimer les corbeilles, le volume et `verif/` par un agent | La table du suivi réserve la suppression définitive au propriétaire. Les agents déplacent vers la corbeille, et s'arrêtent là. |
| Corriger `docs/02:230` (8,58 M) dans cette campagne | Le critère de la fiche est l'égalité avec pgvector, pas le chiffre de la conception. Corriger le document est une décision du propriétaire, non bloquante. |

## Références

- `docs/phases/P10-validation-a-l-echelle-reelle.md`, section `### IMPORTER-BASE-COMPLETE` (`:88-103`), et `### CORRIGER-BLOCAGE-APRES-COMMIT` (`:60-86`).
- `docs/02-architecture.md` : mémoire et tailles (`:230`), compaction (`:198-210`), restauration (`:213-224`).
- `docs/05-api.md` : `POST /batches` (`:68`), inbox et liens symboliques (`:166-175`), état d'index et segments (`:62`), `wait_timeout_ms` (`:21`, `:579`).
- `docs/07-stack-et-deploiement.md` : CLI (`:15`), volumes (`:134-140`).
- `docs/08-invariants-et-tests.md` : MES-07 (`:72`), MES-09 (`:74`).
- `docs/suivi.md` : journal des 2026-11-05 et 2026-11-06 (`:1285-1298`), idées hors périmètre (`:741-745`), données de mesure conservées (`:746-765`), décision du 2026-10-29 sur la reconstruction lexicale (`:700`).
- `docs/mesures/MES-07-duree-compaction.md`, campagne du 2026-10-25 (`:44`, `:60`, `:164-166`).
- `docs/mesures/MES-09-ram-reelle.md`, campagnes du 2026-10-27 et du 2026-11-03 (`:95`, `:207`, `:330`, `:368`).
- `docs/mesures/routes-en-conteneur-2026-11-03.md` (modèle de rapport daté, `:1-6`).
- `docs/legifacile/index-legifacile.md` (six index) et `docs/legifacile/configs/*.json` (`lexical.enabled: true`).
- ADR-0002 (une instance par projet), ADR-0009 (recherche multi-index), ADR-0017 (la compaction suspend les écritures), ADR-0032 (accès à la production), ADR-0047 (cible de 60 s par index), ADR-0064 (format d'un instantané), ADR-0065 (instantané hors ligne, restauration, `--index`), ADR-0067 (échéance de `?wait=true`), ADR-0077 (trois états de RSS), ADR-0087 et ADR-0088 (reprise lexicale), ADR-0107 (écriture de lots par l'API), ADR-0116 (un seul écrivain par index), ADR-0129 (replanification, décisions 4 et 7, tableau du pic disque), ADR-0134 (progression des jobs), ADR-0135 (protocole de mesure en conteneur, décisions 3, 5 et 7).
- Verdicts de `@architecte` du 2026-11-06, recopiés dans le contexte de cet ADR.
