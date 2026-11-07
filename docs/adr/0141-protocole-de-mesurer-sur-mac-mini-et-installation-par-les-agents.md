# ADR-0141 — Protocole de MESURER-SUR-MAC-MINI et installation par les agents

- **Statut** : Accepté — décidé par le propriétaire le 2026-11-07 (« ADR-0141 accepté »), sans réserve
- **Date** : 2026-11-07
- **Décideurs** : propriétaire
- **Proposé par** : `@greffier`, à la demande de l'orchestrateur, pour la tâche `MESURER-SUR-MAC-MINI` (P10), d'après les verdicts de `@architecte` du 2026-11-07 et les faits relevés les 2026-11-06 et 2026-11-07
- **Remplace** : — (aucun ADR ; voir la décision 1 pour les deux clauses d'ADR-0129 qu'il remplace, pour cette tâche seulement, sans modification de son corps)

## Contexte

La tâche `MESURER-SUR-MAC-MINI` (P10) porte la latence contraignante de J10 : une p95 d'au plus **698 ms**, en conteneur, sur le Mac mini, comparée à MES-00 du 2026-10-20 (fiche `docs/phases/P10-validation-a-l-echelle-reelle.md:175-195` ; `docs/08-invariants-et-tests.md:88` ; ADR-0129, décision 7 point 5, `:159`). La campagne ne peut pas démarrer en l'état. `@architecte`, interrogé par l'orchestrateur le 2026-11-07 avant toute installation et toute mesure, rend un `ECART` — l'installation et les arrêts sont réservés au propriétaire — et plusieurs points `NON-COUVERT`. Cet ADR fixe le protocole. Il ne mesure rien, et il ne modifie ni la fiche, ni `docs/01` à `docs/08`, ni un ADR accepté.

### Les faits

- **La cible.** Mac mini M1 (`Macmini9,1`, 8 cœurs, 16 Gio), macOS 26.5.2, Docker Desktop 4.83.0, moteur 29.6.2, VM `linux/aarch64`. L'hôte était sous pression mémoire avec une VM de 8 Gio : **1,1 Gio de swap** et **7,3 Gio compressés** (relevé du propriétaire).
- **La VM et le disque changent.** Le propriétaire passe la VM Docker de **7,75 Gio**, l'état de MES-00 et de P8, à **11 Gio**. Il déplace lui-même le disque de Docker Desktop (`Docker.raw`, 115 Go réels, qui porte le volume Postgres de Legifacile) sur un SSD externe NVMe en USB4 (`MattCorp-SSD`, APFS, 973 Go libres) ; le disque interne n'a que **39 Gio libres** (`docs/suivi.md:1354`).
- **Legifacile est arrêté.** Les six conteneurs Supabase sont arrêtés depuis le 2026-11-07 vers 00:30, par le propriétaire, pour le week-end ; il les relancera lui-même (`docs/suivi.md:1354`).
- **Aucun Crible sur le Mac mini** (`docs/suivi.md:1354`). Sur la machine de développement : l'image `crible:p10` (`linux/arm64`), le volume des modèles `crible-p10-models` (1,5 Gio) et six instantanés hors ligne dans `data/p10-exchange/snapshots/` — **23,73 GiB, soit 25,48 Go**, dont `decisions` pour **9,21 GiB** (`docs/mesures/import-legifacile-2026-11-06.md:44`, `:410` ; `docs/suivi.md:792`, `:794`).
- **Le propriétaire a autorisé les agents.** Le 2026-11-06 : « est-ce que tu peux préparer ce qu'il faut sur le mac mini en te connectant par ssh ? Je t'autorise à le faire, tu peux préparer mesurer sur mac mini » (`docs/suivi.md:1354`).
- **Le total de MES-00 est additif.** À chaud, sur 1 000 requêtes tirées de titres et de sommaires, `k = 10` : p50 **279**, p95 **698**, p99 **1 112 ms**. Ce total est la somme de l'**encodeur** (p95 50 ms), du SQL et de la **reconstruction** des textes des 10 résultats (MES-00 §4.1, `docs/mesures/MES-00-reference-pgvector.md:170-181` ; jeu de données `:15`).

### Ce que les documents laissent ouvert

1. **L'installation et les gestes d'exploitation.** ADR-0129, décision 2, écrit : « Le propriétaire arrête Legifacile et installe Crible sur le Mac mini […] Aucun agent n'exécute ces gestes » (`:100`). La fiche le répète — « l'installation de Crible sur le Mac mini par le propriétaire, qui est hors du champ des agents » (`:177`) — et son critère exige qu'« aucun geste d'exploitation n'a été fait par un agent » (`:193`). La règle 4 de la décision 9 dispose qu'« aucun arrêt, redémarrage ni `docker stop` » n'est fait par un agent, et elle vise tout conteneur du Mac mini, celui de Crible compris (`:192`). Rien ne prévoit la préparation demandée par le propriétaire.
2. **Le plafond mémoire.** Poser `--memory` serait un `ECART` à ADR-0135, décision 5 (`:78`), et à ADR-0139, décision 3 (`:70`).
3. **La remesure de pgvector.** Elle exigerait Supabase en service, donc un geste du propriétaire ; aucun document ne la confie aux agents.
4. **Le canal et le mode de la latence.** La population des 97 requêtes est conforme à la fiche et à ADR-0140, décision 7 (`:104`), qui lit 698 ms comme un plafond sur une autre population. En revanche, ni le canal ni la passe ne sont tranchés : le mode local de `crible eval` lit `debug.timings_ms` (ADR-0125, décision 9, `:152`), le mode API mesure l'aller-retour client et n'est pas comparable (ADR-0131, décision 7, `:112`), et rien ne dit si J10 est porté par le dense primaire à vecteur fourni ou par le dense secondaire encodé par Crible.
5. **MES-09.** La série par index demanderait de déplacer des dossiers sous `/data`, ce que la règle 2 de la décision 9 d'ADR-0129 interdit (`:190`) ; la recette, elle, se rejoue sur un dossier distinct (règle 3, `:191`).
6. **Le sort de l'installation.** Aucun document ne dit ce qu'il advient de l'image, des volumes et des instantanés après la campagne.

### Ce qui est déjà conforme

- `/data` en volume nommé (`docs/07:138` ; ADR-0135, décision 3, `:62`).
- Le transfert par instantané puis restauration (ADR-0064, ADR-0065), avec ses preuves : `snapshot.json` (ADR-0064, décision 1, `:55-85`), rapport de restauration (génération, documents, passages ; ADR-0065, décision 6, `:119`) et `crible inspect` (`docs/07:15`). L'époque change — E+1 (ADR-0065, décision 2, `:69-77`) — et **ne se compare pas** ; la génération reste G.
- Les protocoles de MES-02, MES-03, MES-05 et MES-09 reprennent ADR-0135, avec la RAM de la VM déclarée (`docs/08:67`, `:68`, `:70`, `:74`).
- Le port sur `127.0.0.1`, la mesure hors de `$PROD_DAILY_WINDOW` (ADR-0032, point 2, `:23-27` ; MES-00 `:22`) et les fichiers temporaires hors des deux arbres (ADR-0129, règle 2, `:190`).

### Corrections de citation faites pour cet ADR

- **`$PROD_DAILY_WINDOW` est nommée à MES-00 `:22`, non à `:21`.** ADR-0140 `:65` cite `:21`. Le présent ADR cite `:22` pour le nom et ADR-0032 pour la règle.
- **Le total des instantanés est de 23,73 GiB (25,48 Go).** La demande l'arrondit à 23,8 Gio ; les deux chiffres viennent du rapport d'import (`:44`), et `decisions` pèse 9,21 GiB (`:410`).
- **Les 178 Mo de l'image sont ceux du build arm64 de J8** (`docs/suivi.md:1298`). La taille de `crible:p10` n'est pas publiée ; seule sa durée de construction l'est, **128 s** (`docs/suivi.md:1331`).
- **MES-00 §4.1 couvre `:170-181`, et son p95 de 698 ms est le `total` chaud de 1 000 requêtes**, non celui des 97 requêtes du jeu de référence. Les parts du total sont à `:183-184`.

## Décision

**La campagne suit le protocole ci-dessous. Sept points sont tranchés ; les autres sont des rappels de documents acceptés, ou des écarts assumés et nommés.** Rien ne démarre avant que cet ADR soit `Accepté`.

### Décision 1 — Gestes permis aux agents, pour cette tâche seulement

- **Périmètre.** Par `$PROD_SSH` et hors de `$PROD_DAILY_WINDOW`, les agents peuvent, **pour la seule tâche `MESURER-SUR-MAC-MINI`** :
  - charger l'image par `docker save | ssh … docker load`, sans reconstruction ni publication ;
  - créer les volumes nommés de Crible — `crible-p10-data`, `crible-p10-models`, et un volume distinct pour la recette — et copier dans `crible-p10-models` le contenu du volume des modèles de la machine de développement ;
  - copier les six instantanés dans un dossier d'échange du SSD externe, **hors du dépôt Legifacile et hors du dossier de données de Crible** ;
  - lancer `crible restore` dans des conteneurs éphémères ;
  - démarrer, arrêter et retirer **le seul conteneur de Crible**. C'est l'exception explicite à la règle 4 de la décision 9 d'ADR-0129 (`:192`), limitée à ce conteneur.
- **Ce qui reste interdit.** Tout geste sur les conteneurs, images et volumes de Supabase ; tout geste sur le dépôt Legifacile ; tout geste sur Docker Desktop, réglages, redémarrage et mise à jour compris ; toute suppression hors des fichiers d'échange créés par les agents. Aucune valeur de `.env.prod` n'est recopiée : seuls les noms de variables apparaissent (ADR-0032, point 1, `:19-22`).
- **Ce qui est remplacé, et seulement ici.** La clause « installation par le propriétaire » d'ADR-0129, décision 2 (`:100`) et de la fiche (`:177`), ainsi que le critère « aucun geste d'exploitation n'a été fait par un agent » (`:193`), ne s'appliquent pas à cette tâche. Le corps d'ADR-0129 n'est pas modifié, et la règle reste entière pour toute autre tâche.

### Décision 2 — Transfert et preuves

- **Copie.** Les six dossiers d'instantanés sont copiés **tels quels**, sous leur nom `<index>-e<E>-g<G>` (ADR-0139, décision 5 point 7, `:91`).
- **Contrôle à l'arrivée.** Chaque fichier est vérifié contre son blake3 de `snapshot.json` (ADR-0064, décision 1, `:55-85`). Le contrôle principal est celui de `crible restore`, qui refuse un fichier absent, une taille différente, un blake3 différent ou un fichier non listé (`:76-81`). Si un outil blake3 est disponible sur l'hôte, un contrôle indépendant est publié ; sinon, le rapport dit que le contrôle est celui de `crible restore`.
- **Restauration.** Dans `crible-p10-data`, par `crible restore`, dont le rapport donne l'époque remplacée ou « aucune », la nouvelle époque, la génération, le nombre de documents et de passages (ADR-0065, décision 6, `:114-121`).
- **Preuves publiées.** Le blake3 des sources, les six rapports de restauration et `crible inspect` par index. Les comptes doivent être égaux à ceux de la machine de développement : **6 660 239 documents** et **9 600 105 passages** (`docs/mesures/import-legifacile-2026-11-06.md:35-36`).
- **Preuve de non-régression du classement.** Les top-10 du jeu de référence doivent être égaux à ceux de la passe B de `COMPARER-AVEC-PGVECTOR` pour le **dense primaire** — même vecteur, même index. La comparaison se fait contre les rapports déjà versionnés `docs/mesures/eval-<index>-dense-primaire-2026-11-06.json` (`docs/mesures/comparaison-pgvector-2026-11-06.md:49`, `:344`), et la passe secondaire y donne les mêmes top-10 sur 97 requêtes sur 97 (`:329`).
- **Ce qui n'est pas décidé ici.** Aucune reprise de l'import, aucun ré-encodage, aucune écriture d'index hors de la restauration.

### Décision 3 — Conditions de mesure

- **Crible seul dans la VM.** Legifacile est arrêté par le propriétaire ; les agents n'arrêtent rien.
- **Aucun plafond `--memory`.** La RAM de la VM fait la borne (ADR-0135, décision 5, `:78` ; ADR-0139, décision 3, `:70`).
- **Port publié sur `127.0.0.1` seulement.**
- **Ce que le rapport déclare en tête.** La VM de 11 Gio, le disque externe et l'état de Supabase, arrêté ou en service (`docs/suivi.md:1354`).
- **Pression mémoire.** Le swap et la pression mémoire de l'hôte sont relevés **avant et après chaque série** ; la VM a déjà connu 1,1 Gio de swap et 7,3 Gio de compressions.
- **Aucune remesure de pgvector par un agent.** La référence reste **MES-00 du 2026-10-20**. Trois écarts de conditions sont écrits dans le rapport : VM de 11 Gio contre 7,75, disque externe, Supabase arrêté.
- **Si le propriétaire relance Supabase pendant la campagne.** Les agents n'arrêtent rien, suspendent la campagne, le préviennent et rejouent la série en cours.

### Décision 4 — Latence contraignante (critère de J10)

- **La passe qui tranche.** `crible eval` en mode local, dans un conteneur éphémère, sur les **97 requêtes du jeu de référence en texte** (`docs/legifacile/jeu-de-reference.md:12-19`, `:41-42`), **encodées par Crible** — c'est la passe « dense secondaire » de `COMPARER-AVEC-PGVECTOR` (`docs/suivi.md:155`) —, en mode **dense**, sur le `total` **chaud** de `debug.timings_ms` (ADR-0125, décision 9, `:152`).
- **Pourquoi ce total est comparable.** `total` est la valeur de `took_ms`, qui suit le chemin critique `encode + filter + max(dense, lexical) + fusion + group + fetch` ; `validate` en est exclu, car il est chronométré avant la prise du jeton (ADR-0119, décision 6, `:98-104`). `fetch` couvre la lecture des charges utiles, donc le texte des résultats (`:109` ; `crates/crible-search/src/pipeline.rs:235-236`, `:363`). Il comprend donc l'encodage et la lecture des textes, comme le total de MES-00 comprend l'encodeur et la reconstruction (MES-00 §4.1, `:170-181`).
- **Écart à la recommandation de `@architecte`.** Il recommandait le dense primaire, à vecteur fourni. Cette passe exclurait l'encodage, qui pèse **11,1 %** du total chaud de MES-00 (`:183-184`). Comparer un total sans encodage à un total qui en contient un mélangerait deux grandeurs ; l'écart est assumé, écrit et justifié.
- **Répétitions.** Une passe d'échauffement non comptée, puis **3 passes** (ADR-0135, décision 7, `:96`).
- **Rapporté sans être contraignant.** Le dense primaire à vecteur fourni, l'hybride `rrf` en texte, et un froid partiel nommé comme tel (ADR-0135, décision 1, `:50` ; ADR-0084, décision 7, `:57`).
- **Lecture complémentaire, non contraignante.** Les 1 000 requêtes de MES-00 (`data/mes00/`, déjà locaux ; ADR-0140 `:23`) passées en texte par le même protocole, pour donner un p95 de même population que MES-00.
- **Ce qui n'est pas décidé ici.** Aucune cible nouvelle : 698 ms reste la cible de `docs/08:88` et le critère de J10 (fiche `:190`).

### Décision 5 — MES-02, MES-03, MES-05, MES-09 et recette

- **Protocoles repris.** Ceux d'ADR-0135, en conteneur sur le Mac mini : MES-02 (N ∈ {1000, 2000, 5000} et pools de 1 k à 7,74 M, ADR-0077 `:54` et `:57`), MES-03, MES-05, MES-09 (les trois états d'ADR-0077, `:60`). La **RAM de la VM** et la **taille de page** mesurée dans le conteneur (`getconf PAGESIZE`, ADR-0135, décision 7, `:93`) sont déclarées.
- **MES-09 en série d'instance seulement.** Les six index ouverts, **sans déplacement de dossiers sous `/data`** (ADR-0129, règle 2, `:190`). La série par index n'est pas faite, et le rapport le dit.
- **Recette.** INV-12 (`docs/08:20`) rejouée sur `dole`, dans le **volume distinct** de la décision 1, sur un dossier de données séparé (ADR-0129, règle 3, `:191`).

### Décision 6 — Après la campagne

- **Arrêt.** Les agents arrêtent le conteneur de Crible, qui ne sert aucun utilisateur.
- **Conservation.** L'image, `crible-p10-data`, `crible-p10-models` et le volume de recette sont conservés ; les instantanés copiés dans le dossier d'échange du SSD sont gardés jusqu'à la décision du propriétaire.
- **Ce qui appartient au propriétaire.** Tout retrait de Crible, comme le branchement de Legifacile sur Crible, est une décision **et** un geste du propriétaire.

### Décision 7 — Si la cible n'est pas tenue

- **La fiche s'applique.** J10 n'est pas tenu (fiche `:192`).
- **Un ADR rouvre la question**, de l'index de plage (ADR-0046 `:43`) ou de la cible. Aucune correction de performance n'est faite dans cette tâche (fiche `:195`).

## Application

Rien n'est appliqué avant que le propriétaire n'ait accepté cet ADR. À l'acceptation :

- cet ADR ne modifie **ni la fiche**, **ni `docs/01` à `docs/08`**, **ni le corps d'ADR-0129** ; les clauses qu'il remplace valent pour la seule tâche `MESURER-SUR-MAC-MINI` ;
- sur demande du propriétaire, `@greffier` ajoute une note à la ligne d'index d'ADR-0129 — « clauses de la décision 2 `:100` et de la décision 9 point 4 `:192` remplacées, pour la seule tâche `MESURER-SUR-MAC-MINI`, par ADR-0141 » — et aligne la fiche aux lignes `:177` et `:193`, en citant `ADR-0141` ;
- l'orchestrateur ajoute la ligne de l'écart dans `docs/suivi.md`, section « Écarts à la conception » (`:506-510`), avec le numéro `ADR-0141` ;
- les rapports de la campagne vont dans `docs/mesures/`, au modèle de `docs/mesures/README.md`.

## Conséquences

- **Positives** :
  - la campagne peut démarrer sur l'autorisation donnée par le propriétaire le 2026-11-06, sans attendre ses gestes ;
  - le périmètre interdit est explicite : Supabase, dépôt Legifacile et Docker Desktop sont hors de portée des agents ;
  - le transfert est prouvé quatre fois : blake3 des sources, refus de `crible restore` sur un fichier altéré, comptes de `crible inspect`, égalité des top-10 ;
  - la passe contraignante est celle dont le total contient l'encodage et la lecture des textes, comme le total de MES-00 ;
  - les conditions et leurs trois écarts sont déclarés, ce qui rend la comparaison à 698 ms honnête et vérifiable ;
  - aucun plafond `--memory` n'est posé, conformément à ADR-0135 et ADR-0139 ;
  - MES-09 est faite sans déplacer un dossier sous `/data`, et la recette sans écrire dans l'instance ;
  - l'installation survit à la campagne, pour la décision du propriétaire.
- **Négatives / coûts** :
  - les agents touchent désormais l'hôte de production, même dans un périmètre étroit : c'est un élargissement de la règle d'ADR-0129, justifié par une autorisation ponctuelle ;
  - la campagne tourne avec Supabase arrêté et une VM de 11 Gio : le p95 n'est pas un rejeu à conditions égales de MES-00, et l'écart est déclaré, non prouvé ;
  - la pression mémoire est plus basse qu'au temps de MES-00, ce qui peut flatter le résultat ;
  - copier 23,73 GiB sur le SSD coûte du temps et de l'espace, dont 9,21 GiB pour `decisions` ;
  - le SSD en USB4 ajoute une variable, les IOPS, à MES-02, à MES-03 et au froid partiel ;
  - la passe contraignante ajoute l'encodage au total mesuré : l'écart à la recommandation de `@architecte` doit être défendu dans le rapport ;
  - 97 requêtes donnent un p99 grossier, comme dans ADR-0140 ;
  - la lecture complémentaire sur 1 000 requêtes est un travail de plus, non contraignant, qui ne doit pas être confondu avec le critère ;
  - la série MES-09 par index est perdue.
- **À surveiller** :
  - le swap et la compression de l'hôte avant et après chaque série : une croissance dit que la VM de 11 Gio est trop grosse pour 16 Gio ;
  - la résidence des pages : tout chiffre dont la résidence dépasse 0 s'appelle « froid partiel, résidence r » (ADR-0084, `:57`) ;
  - les points de MES-02 que l'instance restaurée ne permet pas de construire : ils sont déclarés absents avec leur cause, jamais remplacés par un chiffre d'une autre population ;
  - l'égalité des top-10 : une différence arrête la campagne, car le transfert n'est plus fidèle ;
  - les comptes contre 6 660 239 documents et 9 600 105 passages ;
  - un redémarrage de Supabase par le propriétaire : la série en cours est rejouée ;
  - une p95 au-dessus de 698 ms : la fiche s'applique, et un ADR rouvre l'index de plage ou la cible ;
  - une limite `--memory` réintroduite plus tard : elle romprait la comparabilité avec cette campagne.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Laisser l'installation au propriétaire, comme ADR-0129, décision 2, l'écrit | Le propriétaire a explicitement autorisé les agents le 2026-11-06 ; attendre ses gestes bloquerait la campagne pendant le week-end, alors que Legifacile est déjà arrêté. |
| Poser un plafond `--memory` sur le conteneur | `ECART` à ADR-0135, décision 5 (`:78`), et à ADR-0139, décision 3 (`:70`) : la RAM de la VM fait la borne, et un plafond changerait le reclaim de toute la campagne. |
| Remesurer pgvector pendant la campagne | Exige Supabase en service, donc un geste du propriétaire, et remet la charge sur une machine de 16 Gio. La référence reste MES-00 du 2026-10-20. |
| Faire porter J10 par le dense primaire, à vecteur fourni | La recommandation de `@architecte`, écartée ici : cette passe exclurait l'encodage, qui pèse 11,1 % du total de MES-00 ; les deux totaux ne seraient pas de même nature. |
| Mesurer la latence par le mode API de `crible eval` | Le mode API mesure l'aller-retour client, non comparable aux étapes serveur (ADR-0131, décision 7), et il écrirait le journal de debug de l'instance. |
| Faire MES-09 par index en déplaçant les dossiers | La règle 2 de la décision 9 d'ADR-0129 interdit toute écriture sous le dossier de données (`:190`) ; le déplacement est une écriture, et la série d'instance suffit à éprouver la somme des mémoires. |
| Ne pas faire MES-09 du tout | Perd la mesure de l'instance à six index, qui est le cas réellement servi et celui qui dimensionne la VM. |
| Restaurer dans le volume de la machine de développement ou dans l'instance servie | La restauration change l'époque et écrit sous `/data` ; elle se fait dans un volume nommé neuf, et l'instance servie n'existe pas encore sur le Mac mini. |
| Copier les instantanés dans le dépôt Legifacile ou sous `/data` | Interdit par la règle 2 de la décision 9 (`:190`) et par la séparation d'ADR-0129, décision 8. Le dossier d'échange du SSD est hors des deux arbres. |
| Supprimer les instantanés après la restauration | Ils sont la copie valide de ce qui a coûté du GPU (INV-14) et servent à rejouer une restauration ; ils sont gardés jusqu'à la décision du propriétaire. |
| Mesurer avec Supabase en service | La campagne ne serait plus « Crible seul », et la pression mémoire de l'hôte fausserait la latence ; le propriétaire n'a pas à relancer un service pour une mesure. |
| Prendre une exécution locale, hors conteneur, comme mesure contraignante | `docs/08:88` et la fiche (`:176`) exigent le conteneur et le Mac mini ; une mesure native ne vaudrait pas verdict. |
| Toucher à Docker Desktop pour agrandir la VM ou déplacer le disque | C'est un réglage de l'hôte, déjà fait par le propriétaire ; la décision 1 l'interdit aux agents, et la VM de 11 Gio est déclarée telle quelle. |
| Traiter les 97 requêtes comme la population de MES-00 | Les populations diffèrent ; ADR-0140, décision 7 (`:104`), lit 698 ms comme un plafond sur une autre population. La lecture complémentaire sur 1 000 requêtes est là pour cela. |

## Références

- `docs/phases/P10-validation-a-l-echelle-reelle.md`, section `### MESURER-SUR-MAC-MINI` (`:175-195`), `### COMPARER-AVEC-PGVECTOR` (`:141-153`).
- `docs/mesures/MES-00-reference-pgvector.md` : jeu de données (`:15`), fenêtre (`:22`), §4.1 (`:170-184`).
- `docs/mesures/import-legifacile-2026-11-06.md` : synthèse (`:35-36`), volume (`:87`), instantanés (`:44`, `:410`).
- `docs/mesures/comparaison-pgvector-2026-11-06.md` : rapports de la passe B (`:49`, `:344`), top-10 identiques (`:329`).
- `docs/suivi.md` : cibles (`:29-33`), tâche (`:155`, `:157`), données conservées (`:792`, `:794`), journal des 2026-11-03, 2026-11-06 et 2026-11-07 (`:1298`, `:1331`, `:1346`, `:1354`), écarts (`:506-510`).
- `docs/legifacile/jeu-de-reference.md` : 97 requêtes (`:12-19`), jeu textuel (`:41-42`).
- `docs/08-invariants-et-tests.md` : INV-12 (`:20`), MES-02 (`:67`), MES-03 (`:68`), MES-05 (`:70`), MES-09 (`:74`), cible de latence (`:88`).
- `docs/07-stack-et-deploiement.md` : volumes (`:138-140`), CLI (`:15`).
- Code : `crates/crible-search/src/pipeline.rs:235-236` et `:363` (`fetch` des survivants, qui lit les textes).
- ADR-0032 (accès à la production), ADR-0046 (index de plage, `:43`), ADR-0064 et ADR-0065 (instantané et restauration), ADR-0077 (protocole de latence dense, `:54`, `:57`, `:60`), ADR-0084 (froid partiel, `:57`), ADR-0119 (temps de `debug`, `:87`, `:101`, `:109`), ADR-0125 (commande `eval`, décision 9), ADR-0129 (replanification, décisions 2, 7 et 9), ADR-0131 (mode API), ADR-0135 (protocole en conteneur), ADR-0139 (protocole d'import), ADR-0140 (comparaison avec pgvector).
- Verdicts de `@architecte` du 2026-11-07, recopiés dans le contexte de cet ADR.
