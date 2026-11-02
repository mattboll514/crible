# ADR-0129 — Replanification de la fin du plan

- **Statut** : Proposé
- **Date** : 2026-11-02
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, sur demande du propriétaire du 2026-11-02 (conversation avec l'orchestrateur), à partir de l'analyse de `@architecte` du 2026-11-02 ; travail hors tâche, aucune tâche du plan ne le porte
- **Remplace** : — (ne remplace aucun ADR ; complète ADR-0032 sans toucher à son corps et résout Q-07)

## Contexte

### Ce qui change, et pourquoi un ADR

Le plan de Crible compte dix phases, de P0 à P9, et son dernier jalon, J9, est défini comme « la v1 » (`docs/09-plan-de-developpement.md:5`, `:79`). Les critères de la v1 (`docs/01-vision-et-perimetre.md:59-66`, repris par `docs/phases/P9-migration-legifacile.md:5-10`) mêlent deux projets : la qualité de recherche de Crible d'un côté, la mise en service de Legifacile de l'autre — tâche quotidienne stable 14 jours, retour arrière, bascule, puis bilan.

Le propriétaire a décidé le 2026-11-02 de séparer les deux projets, de placer un tableau de bord avant la validation et d'ajouter une phase de validation à l'échelle réelle. Aucun document ne prévoit cette replanification : l'analyse de `@architecte` du 2026-11-02 rend le verdict `NON-COUVERT`. Elle relève trois lacunes de plus :

- le transfert de l'index entre la machine de développement et le Mac mini n'est prévu par aucun document ;
- aucun document ne dit où vivent les fiches d'un autre projet que Crible (`docs/09:1-5` fait du plan un document de Crible) ;
- la portée de `MESURER-EN-CONTENEUR-MAC` (`docs/phases/P8-docker-et-deploiement.md:168-173`) suppose le Mac mini comme hôte, alors que l'import et la comparaison se font désormais sur la machine de développement.

Q-07 (`docs/adr/README.md:194`) est ouverte depuis le 2026-11-01 et bloque `MESURER-EN-CONTENEUR-MAC` (P8) et `BASCULER-PRODUCTION` (P9). Le principe de latence de `docs/suivi.md:25` et `:31` existe déjà : **le rappel prime, la latence est indicative**, et la règle de `docs/08-invariants-et-tests.md:88` ne vaut que sur le Mac mini en conteneur. Il manquait la décision sur les machines.

### L'état du plan avant cet ADR

| Élément | Contenu actuel | Référence |
|---|---|---|
| Chaîne des phases | `P0 → … → P8 → P9` | `docs/09:5`, `:79`, `:84` |
| P8 | Docker et déploiement ; **J8** : « l'image tourne sur le Mac mini » | `docs/09:78`, `docs/phases/P8…:3-9` |
| P9 | Migration de Legifacile, dix tâches ; **J9 = v1** | `docs/phases/P9…:1-136` |
| Mesure en conteneur | `MESURER-EN-CONTENEUR-MAC`, en P8, sur le Mac mini | `docs/phases/P8…:168-173` |
| Latence contraignante | p95 ≤ 698 ms (MES-00), **seulement** sur le Mac mini en conteneur | `docs/08:88`, `docs/suivi.md:31` |
| Machine de production | « Mac mini M1 16 Go en Docker » | `docs/01:61`, `:72` |
| Architecture de l'image | `linux/arm64` et `linux/amd64` | `docs/07:118`, `:140-143` |

### Les deux machines et leurs rôles

| Machine | Ce qu'elle est | Rôle retenu |
|---|---|---|
| **MacBook Air M3** (`Mac15,12`, 8 cœurs, 24 Gio) | machine de développement | Tout s'y fait pour le moment : import de la base complète, comparaison avec Legifacile/pgvector. Mesures non contraignantes. |
| **Mac mini M1 16 Go** | machine du propriétaire ; elle porte aujourd'hui la production de Legifacile (pgvector), test et production à la fois | Reste en production jusqu'à la confirmation de l'absence de régression. Ensuite, **le propriétaire** arrête Legifacile et y installe Crible. C'est la machine de référence de la latence contraignante. |

Le transfert de la base se fait par **instantané puis restauration** (`crible snapshot` / `crible restore`, ADR-0064 et ADR-0065) : c'est la base même qui a été comparée qui arrive sur le Mac mini, sans réimport ni ré-encodage. Les iid, les segments et les comptes ne changent pas.

La comparaison de temps de calcul est refaite sur le Mac mini, en conteneur, après l'installation, et comparée à MES-00 (pgvector, mesurée sur le Mac mini le 2026-10-20). C'est cette mesure-là qui porte la latence contraignante de `docs/08:88`.

La production officielle du nouveau Legifacile n'est pas actée ; ce sera probablement un serveur OVH, **hors v1**. L'image reste multi-architecture (`linux/arm64` pour les deux Mac, `linux/amd64` pour la cible future), conformément à `docs/07:118`.

### Les chiffres disponibles, et leur incertitude

Le disque de la machine de développement dispose de **151 Gio libres**. Le pic extrapolé pour la campagne complète est d'environ **105 Go** :

| Poste | Volume extrapolé |
|---|---|
| Lots de la base complète | ≈ 21 Go |
| Index Crible | ≈ 28 Go |
| Instantané de transfert | ≈ 28 Go |
| Compaction simultanée | + 28 Go |

L'extrapolation part de l'échantillon réel de **205 261 passages** : lots 514 Mo, index 669 Mo. La marge (environ 46 Go) est réelle mais mince, et l'extrapolation est linéaire depuis un échantillon vingt fois plus petit que la base ; le pic n'est pas mesuré. La fiche d'import doit donc exiger un **contrôle d'espace disque avant la campagne**.

Côté mémoire, la machine de développement a 24 Gio, dont environ **13,2 Go** de vecteurs fp16 à 8,58 M passages. Sous Docker Desktop, la RAM de la VM doit être indiquée dans la fiche de mesure. Le Mac mini a 16 Go, dont environ 8 Go alloués à la VM Docker aujourd'hui (`docs/01:72`).

### Trois sens du mot « v1 », à ne pas confondre

« v1 » porte déjà trois sens distincts dans le dépôt. Cet ADR n'en touche que le premier :

- **horizon du plan de Crible** : c'est le sens que cet ADR fixe à J10 ;
- **format de lot v1** (`docs/format-lot-v1.md`, ADR-0012, ADR-0039, ADR-0040, `format_version: 1`) : inchangé ;
- **API REST v1** (`docs/05-api.md`, ADR-0015) : inchangée ;
- le « format v1 » d'opencode (ADR-0025) est sans rapport.

Trois ADR acceptés portent « v1 » dans leur **titre** — ADR-0004 (« Pas d'ANN en v1 »), ADR-0015 (« API REST v1 »), ADR-0017 (« En v1, la compaction suspend les écritures »). Aucun titre n'est modifié.

### Q-07 et ADR-0032

Q-07 demande quelle est la machine de production. Les faits du 2026-11-01 étaient déjà connus : MacBook Air M3 pour le développement, Mac mini M1 pour le test, production probablement chez OVH mais non actée ; or `docs/01:61` et `:72`, ADR-0004 `:10` et `docs/08:88` désignaient le Mac mini comme machine de production. La décision du 2026-11-02 tranche la question.

ADR-0032, décision 2, interdit aux agents d'arrêter un service de production. L'arrêt de Legifacile et l'installation de Crible sont des gestes du **propriétaire** : ils sont hors du champ de l'ADR, et la règle reste **inchangée**. Les lectures de Crible sur un index en service sont déjà en lecture seule (ADR-0116, décision 1). Il manque seulement un complément explicite pour les mesures de P10 sur le Mac mini.

### La question

Comment replanifier la fin du plan sans réécrire les ADR acceptés, sans casser les citations de tâches existantes, et sans perdre la traçabilité de ce qui part chez Legifacile ?

## Décision

**La fin du plan est replanifiée en trois phases — P8 inchangée dans son esprit, P9 pour le tableau de bord, P10 pour la validation à l'échelle réelle —, et le jalon final devient J10.** Les documents antérieurs se lisent par une règle de correspondance bornée, sans réécriture d'ADR accepté. Le découpage et le mode d'import ne changent pas.

### Décision 1 — découpage inchangé, vecteurs pgvector

Aucune revectorisation complète sur GPU n'est faite pour l'import. Les vecteurs viennent de pgvector. Le découpage reste celui du producteur (ADR-0010) et le mode « migration » de l'ancien producteur est extrait en tâche ponctuelle (décision 6).

### Décision 2 — machines et rôles (résout Q-07)

Trois rôles distincts :

1. **Développement et mesures non contraignantes** : MacBook Air M3 (`Mac15,12`, 8 cœurs, 24 Gio). L'import de la base complète et la comparaison avec pgvector s'y font, pour le moment. Les mesures qui y sont prises sont **indicatives**.
2. **Référence de la latence contraignante** : Mac mini M1 16 Go, en conteneur, après installation de Crible par le propriétaire. Les mesures de P10 y sont refaites et comparées à MES-00 du 2026-10-20.
3. **Production officielle** : non actée, probablement un serveur OVH, hors v1.

Le transfert de la base entre les deux machines se fait par **instantané puis restauration** (ADR-0064, ADR-0065), pas par réimport. Le propriétaire arrête Legifacile et installe Crible sur le Mac mini **après** confirmation de l'absence de régression. Aucun agent n'exécute ces gestes.

L'image reste multi-architecture : `linux/arm64` pour les deux Mac, `linux/amd64` pour la cible future (`docs/07:118`).

### Décision 3 — règle de correspondance pour les documents antérieurs

Dans tout document accepté ou écrit **avant** ADR-0129 — ADR acceptés, `docs/01` à `docs/09`, fiches de phase, suivi — les mentions suivantes se lisent ainsi :

| Mention antérieure | Sens après ADR-0129 |
|---|---|
| `P9` (phase) | phase **P10**, « Validation à l'échelle réelle » |
| `J9` (jalon) | jalon **J10** |
| « la v1 », « cible de latence v1 », « en v1 » | horizon **J10** |
| `P9-migration-legifacile.md` | remplacé à l'application par `P9-tableau-de-bord.md` et `P10-validation-a-l-echelle-reelle.md` |
| noms de tâches conservés (`DEFINIR-INDEX-LEGIFACILE`, `IMPORTER-BASE-COMPLETE`, `CONSTITUER-JEU-DE-REFERENCE`, `COMPARER-AVEC-PGVECTOR`, `BILAN-V1`) | inchangés, désormais en P10 |
| noms de tâches déplacées en passation (`ADAPTER-PRODUCTEUR-LEGIFACILE`, `BRANCHER-MCP`, `MODE-OMBRE`, `BRANCHER-TACHE-QUOTIDIENNE`, `BASCULER-PRODUCTION`) | inchangés, désormais hors du plan de Crible |

Cette règle est **bornée** : elle ne touche ni le format de lot v1, ni l'API REST v1, ni le format v1 d'opencode, ni les titres d'ADR portant « v1 ». Aucun ADR accepté n'est remplacé et leurs corps ne sont pas modifiés ; seuls les documents de conception et les fiches sont mis à jour, à l'application, en citant ADR-0129.

### Décision 4 — P8 : la mesure en conteneur devient indicative, sur la machine de développement

`MESURER-EN-CONTENEUR-MAC` **garde son nom** (l'hôte reste un Mac arm64) et reste en P8, mais change de portée :

- elle se fait sur la **machine de développement**, sous Docker Desktop, avec la **RAM de la VM indiquée** dans le rapport ;
- la mesure est **indicative** : elle éprouve l'image et le conteneur, elle ne porte pas la cible de `docs/08:88` ;
- **J8 est réécrit** : l'image tourne sur la machine de développement en arm64 et se construit en amd64 ; MES-02, MES-05 et MES-09 sont refaites en conteneur, à titre indicatif ; l'arrêt propre est vérifié (`VERIFIER-ARRET-PROPRE`).

### Décision 5 — nouvelle table des phases

| Phase | Fichier | But | Jalon |
|---|---|---|---|
| **P8** | `phases/P8-docker-et-deploiement.md` | Image multi-arch, volumes, arrêt propre, compose, mesures en conteneur sur la machine de développement | **J8** : l'image tourne sur la machine de développement (arm64) et se construit en amd64 ; MES-02, MES-05 et MES-09 refaites en conteneur, indicatives ; arrêt propre vérifié |
| **P9** | `phases/P9-tableau-de-bord.md` | Tableau de bord ; contenu défini par **ADR-0130**, rédigé séparément | **J9** : critères définis par ADR-0130 |
| **P10** | `phases/P10-validation-a-l-echelle-reelle.md` | Six index Legifacile, export de migration, import complet, jeu de référence, comparaison avec pgvector, mesure sur le Mac mini, bilan | **J10** : critères de la décision 7 |

La chaîne devient `P0 → … → P8 → P9 → P10`. Deux notes l'accompagnent : « l'échantillon Legifacile (P2) sert à P4, P5 et P7 » et « le jeu de référence de P10 sert au projet Legifacile (nouveau dépôt) ». Le principe « une seule phase ouverte » (`docs/09:88`) impose P9 avant P10. Le propriétaire a choisi cet ordre : le tableau de bord passe **avant** la validation. Le contenu et les critères du tableau de bord ne sont pas décidés ici ; ils appartiennent à ADR-0130.

### Décision 6 — tâches de P10

Les noms existants sont conservés quand la tâche reste, pour ne pas casser les citations des ADR acceptés. Deux tâches sont nouvelles.

| Tâche | État | Portée |
|---|---|---|
| `DEFINIR-INDEX-LEGIFACILE` | reprise | Six configs d'index, champs, types, niveaux, `cluster_by`, gabarits, réponse à Q-03, correspondance des filtres du MCP ; validées par le propriétaire. |
| `EXPORTER-BASE-LEGIFACILE` | **nouvelle** | Le mode « migration » extrait de l'ancienne `ADAPTER-PRODUCTEUR-LEGIFACILE`. Script ponctuel dans `legifacile-sandbox`, fait **avec le propriétaire**, en **lecture seule** sur le Mac mini ; il écrit des lots au format v1 avec `crible.BatchWriter`, à partir des vecteurs pgvector et de textes recomposés vérifiés contre `src_hash` ; `producer.name` et `producer.host` sont renseignés (ADR-0036). Précédent : `CONSTITUER-ECHANTILLON-LEGIFACILE` (P2). |
| `IMPORTER-BASE-COMPLETE` | reprise | Sur la machine de développement ; dépend d'`EXPORTER-BASE-LEGIFACILE` ; contrôle d'espace disque avant la campagne ; lots `replace_all` attachés, durée et espace mesurés, `crible inspect`, instantané de départ ; comptes égaux à pgvector ou écarts expliqués ; MES-07 et MES-09 à l'échelle réelle. |
| `CONSTITUER-JEU-DE-REFERENCE` | reprise | Jeu JSONL de requêtes réelles avec périmètres et `expected` ; mêmes cas qu'aujourd'hui, validé par le propriétaire, identique pour pgvector et Crible. |
| `COMPARER-AVEC-PGVECTOR` | reprise | `crible eval` (dense et hybride) et mesure pgvector sur le même jeu. **Rappel et fidélité contraignants** ; **latence rapportée comme indicative** sur la machine de développement (option A de l'analyse). Rapport : rappel par périmètre, p50/p95/p99, requêtes gagnantes et perdantes, rejeu de MES-14 sur les filtres d'intervalle. |
| `MESURER-SUR-MAC-MINI` | **nouvelle** | Après l'installation par le propriétaire, en conteneur, sur l'instantané restauré. Comparée à MES-00 (2026-10-20). Porte la **latence contraignante** de `docs/08:88`, MES-02, MES-03, MES-05, MES-09 et le rejeu de la recette. Placée **avant** `BILAN-V1`. |
| `BILAN-V1` | reprise | Bilan de Crible : ce qui est tenu, ce qui ne l'est pas, les écarts expliqués. |

Un mode « import pgvector » dans Crible serait un écart à la conception : il ferait entrer le compositeur de Legifacile dans le moteur, alors que `docs/01:51` et `docs/04:312-315` placent le découpage et la composition chez le producteur. Il n'est pas retenu.

### Décision 7 — critères de J10 (fin du plan, « v1 » de Crible)

1. Les **six configs** d'index sont définies et validées par le propriétaire.
2. Les **8,58 M passages** sont importés ; les comptes par index sont égaux à pgvector ou les écarts sont expliqués ; MES-07 et MES-09 sont faites à l'échelle réelle.
3. Le **jeu de référence** est validé et identique pour pgvector et Crible.
4. **Rappel** : au moins **0,482** sur les 56 questions satisfaisables (`Q30` à part) et les cibles par corpus (`juri_texte_integral` ≥ 0,290, `textes_vigueur` ≥ 0,750). **Fidélité** : au moins **0,99** à 8,58 M passages, au N par défaut (5000, ADR-0078).
5. **Latence p95** : contraignante (≤ 698 ms, MES-00) seulement sur le Mac mini en conteneur, par `MESURER-SUR-MAC-MINI` ; indicative ailleurs.
6. Le **bilan de Crible** est écrit et validé.

Partent au projet Legifacile : la **tâche quotidienne stable 14 jours** et le **retour arrière testé** (interrupteur `backend` du MCP). Ils ne conditionnent plus J10. Le projet Legifacile a son propre bilan.

### Décision 8 — séparation de Crible et de Legifacile : document de passation

La tâche quotidienne appartient à Legifacile. Un **nouveau dépôt** portera le projet officiel Legifacile qui tourne avec Crible ; le dépôt actuel est `legifacile-sandbox`. Le nouveau dépôt a ses propres mécanismes, son authentification Firebase Auth et ses propres ADR. Crible reste **derrière le backend de Legifacile**, avec ses clés d'API ; il n'est pas exposé aux utilisateurs.

Les fiches de Legifacile vont dans un **document de passation**, `docs/passation-legifacile.md`, **hors du plan de Crible**. Il porte **mot pour mot** les fiches de :

- `ADAPTER-PRODUCTEUR-LEGIFACILE`, mode quotidien (encodage natif, pas la migration) ;
- `BRANCHER-MCP` ;
- `MODE-OMBRE` ;
- `BRANCHER-TACHE-QUOTIDIENNE` ;
- `BASCULER-PRODUCTION`.

Le document porte aussi le **contrat** que Crible garantit au nouveau dépôt, et rien de plus :

- le **format de lot v1** (`docs/format-lot-v1.md`, ADR-0012, ADR-0039, ADR-0040, `format_version: 1`) ;
- l'**API REST v1** et sa spécification OpenAPI (ADR-0015, ADR-0111) ;
- le **client Python** `clients/python` (`BatchWriter`, `Client`, `AsyncClient`, ADR-0112) ;
- l'**image Docker** multi-architecture publiée sur GHCR (P8 `PUBLIER-IMAGE`, `docs/07:116-145`).

Le nouveau dépôt épingle Crible à un **tag**. Aucune promesse de compatibilité au-delà de ce tag n'est faite, et aucune publication sur PyPI n'est décidée ici.

### Décision 9 — complément à ADR-0032, sans toucher à son corps

Les mesures de P10 sur le Mac mini respectent quatre règles :

1. elles n'exécutent que des **commandes de lecture** de Crible — `search`, `inspect`, `export`, `stats`, `eval` local ;
2. aucune **écriture** sous le dossier de données de Crible ni dans le dépôt Legifacile ; les fichiers temporaires vont hors de ces deux arbres, et les rapports sont rapatriés dans `docs/mesures/` de Crible ;
3. une mesure qui exigerait une **écriture d'index** (attache) se fait sur un **dossier de données distinct**, jamais sur l'instance servie ;
4. aucun **arrêt, redémarrage ni `docker stop`** par un agent, y compris pour libérer le `WRITER_LOCK`.

### Décision 10 — Q-07 résolue

> **Q-07 — résolue le 2026-11-02 par ADR-0129.** Trois rôles distincts. **Développement et référence des mesures non contraignantes** : MacBook Air M3 (`Mac15,12`, 8 cœurs, 24 Gio) ; c'est là que se font l'import de la base complète et la comparaison avec pgvector. **Machine de référence de la latence contraignante de la v1** : Mac mini M1 16 Go, en conteneur ; le propriétaire y installe Crible après confirmation de l'absence de régression, et les mesures de P10 y sont refaites et comparées à MES-00 (2026-10-20). **Production officielle** : non actée, probablement un serveur OVH, hors v1. Conséquences : `docs/01:61` et `:72`, `docs/08:88`, `docs/03:147` et le contexte d'ADR-0004 `:10` sont mis à jour ; ADR-0004 n'est pas remplacé ; l'architecture multi-arch de `docs/07:118` est conservée (`linux/arm64` pour les deux Mac, `linux/amd64` pour la cible future).

### Décision 11 — ce qui n'est pas décidé ici

- le **contenu** et les critères du tableau de bord : ADR-0130 ;
- les **fiches réécrites** dans le nouveau dépôt Legifacile et ses ADR ;
- la **production officielle** du nouveau Legifacile ;
- une promesse de **compatibilité du client Python** au-delà du tag épinglé, et toute publication sur PyPI ;
- toute **dépendance nouvelle** ;
- le détail des fiches de P10, qui seront écrites à l'application.

## Application

Rien n'est appliqué tant que le propriétaire n'a pas accepté cet ADR. À l'acceptation, `@greffier` met à jour les documents suivants, en citant `ADR-0129`.

**Fichiers nouveaux**

- `docs/phases/P9-tableau-de-bord.md` (coquille ; contenu par ADR-0130)
- `docs/phases/P10-validation-a-l-echelle-reelle.md` (les tâches Crible, `MESURER-SUR-MAC-MINI`, `BILAN-V1`)
- `docs/passation-legifacile.md` (cinq fiches mot pour mot et le contrat)

**Documents de conception**

- `docs/01-vision-et-perimetre.md` : `:45`, `:59-66`, `:72`
- `docs/02-architecture.md` : `:43`
- `docs/03-recherche.md` : `:147`, `:166`
- `docs/04-donnees-et-ingestion.md` : `:312-315`, `:370-379`
- `docs/07-stack-et-deploiement.md` : `:118`, `:140-143`
- `docs/08-invariants-et-tests.md` : `:71`, `:72`, `:73`, `:86`, `:88`
- `docs/09-plan-de-developpement.md` : `:5`, `:68-79`, `:81-86`, `:88`

**Fiches de phase**

- `docs/phases/P2-encodeur-et-recette.md` : `:42`
- `docs/phases/P4-recherche-rapide.md` : `:109`
- `docs/phases/P7-observabilite.md` : `:125`
- `docs/phases/P8-docker-et-deploiement.md` : `:3-9` (but et J8), `:168-173` (portée de la mesure), `:131` (Q-07)
- `docs/phases/P9-migration-legifacile.md` : fichier entier remplacé, contenu scindé entre P10 et la passation

**Index et suivi**

- `docs/adr/README.md` : `:192`, `:193` (P9 → P10), `:194` (Q-07 vers les questions tranchées), `:200`, `:202`, `:203` (phase du rejeu), `:132`, `:134`, `:147-150`, `:181` (notes de phase)
- `docs/suivi.md` : `:20-21` (jalons), `:25`, `:31`, `:34`, `:133-139` (P8), `:140-149` (table des tâches), `:540`, `:667`, `:668`, `:673`, `:681`

**Non modifiés** : les corps des ADR acceptés (`0003`, `0004`, `0009`, `0012`, `0032`, `0046`, `0047`, `0048`, `0076`, `0077`, `0078`, `0080`, `0081`, `0082`, `0088`, `0091`, `0092`, `0093`, `0094`, `0125`) ; `docs/format-lot-v1.md` ; les titres d'ADR portant « v1 ». `docs/mesures/README.md` ne demande aucune mise à jour.

## Conséquences

- **Positives** :
  - le plan ne mélange plus deux projets : Crible finit sur ce qu'il doit prouver, Legifacile porte sa mise en service ;
  - chaque mention antérieure de P9, J9 et « v1 » garde un sens, sans réécrire un seul ADR accepté ;
  - aucun nom de tâche conservé ne casse : les citations des ADR restent valides ;
  - la base comparée est celle qui est installée : l'instantané évite un réimport et un ré-encodage de 8,58 M passages ;
  - Q-07 est tranchée, ce qui débloque P8 et la suite ;
  - la latence contraignante reste mesurée là où elle a un sens, sur le Mac mini, comme le prévoyait `docs/08:88` ;
  - la règle de lecture seule d'ADR-0032 est précisée pour les mesures en production, sans être affaiblie.
- **Négatives / coûts** :
  - le plan gagne une phase et deux tâches nouvelles (`EXPORTER-BASE-LEGIFACILE`, `MESURER-SUR-MAC-MINI`) ;
  - l'import complet se fait sur une machine de 24 Gio avec 151 Gio libres : le pic disque d'environ 105 Go n'est pas mesuré et la marge est mince ;
  - la comparaison de latence sur la machine de développement ne vaut pas verdict ; il faut attendre l'installation sur le Mac mini pour trancher la cible de 698 ms ;
  - le transfert d'un index de 28 Go n'est encore ni scripté ni mesuré ; il repose sur ADR-0064 et ADR-0065, éprouvés sur l'échantillon seulement ;
  - les fiches de Legifacile quittent le plan : elles ne seront plus suivies par l'orchestrateur de Crible ;
  - deux documents de plus à tenir : `docs/passation-legifacile.md` et ADR-0130.
- **À surveiller** :
  - **l'espace disque** pendant l'import et la compaction : un dépassement du pic extrapolé arrête la campagne, et le contrôle préalable doit être fait ;
  - **la RAM de la VM Docker Desktop** sur la machine de développement, avec environ 13,2 Go de vecteurs : une VM sous-dimensionnée fausserait la mesure indicative et pourrait faire échouer l'import ;
  - **la latence p95 sur le Mac mini** : si `MESURER-SUR-MAC-MINI` dépasse 698 ms, J10 n'est pas tenu et il faut rouvrir la question de l'index de plage (ADR-0046) ou de la cible ;
  - **la durée de la reconstruction lexicale** à 8,58 M passages, contre la cible de 60 s d'ADR-0047 : `IMPORTER-BASE-COMPLETE` doit la relever, et un ADR est à écrire si elle n'est pas tenue ;
  - **le rappel par corpus** : la moyenne de 0,482 peut passer alors qu'un corpus décroche ; chaque perte d'attendu doit être listée et expliquée ;
  - **la fidélité à 8,58 M** : elle n'est prouvée qu'à l'échelle de l'échantillon (0,9977 sans filtre, 0,9989 en filtrée, MES-01) ; la confirmation est un critère de J10 ;
  - **la dérive du contrat** avec le nouveau dépôt : le tag épinglé est la seule garantie ; toute évolution du format de lot ou de l'API après ce tag devra être négociée.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| **Garder P9 mixte** (validation Crible et mise en service Legifacile dans la même phase et le même jalon) | Rejetée par le propriétaire le 2026-11-02. Elle fait dépendre la fin de Crible d'une bascule de production qui appartient à un autre projet, et laisse dans le plan de Crible des tâches qui ne mesurent pas Crible. |
| **Installer Crible sur le Mac mini avant la comparaison** | Contredit la décision 2 : il faudrait arrêter Legifacile, qui est encore la production et le test du propriétaire, avant d'avoir prouvé l'absence de régression. Sans retour arrière éprouvé, le risque est supporté par l'utilisateur final. |
| **Renommer `MESURER-EN-CONTENEUR-MAC`** | Le nom reste vrai (hôte macOS arm64) et le renommer casserait une douzaine de citations dans des ADR acceptés et dans le suivi (ADR-0046, ADR-0073 `:99`, ADR-0077 `:29`, ADR-0080 `:90` et `:148`, ADR-0082 `:84`, ADR-0127, `docs/suivi.md:131`, `:133`, `:139`, `:667`, `:668`, `:673`, `:681`). La portée change, pas le nom. |
| **Réimporter depuis les lots au lieu de l'instantané** | Le réimport refait l'attache de 8,58 M passages, prend du temps et de l'espace, et surtout ne garantit pas que la base installée soit identique à celle qui a été comparée. L'instantané puis la restauration (ADR-0064, ADR-0065) transporte la base même, iid et segments compris, pour environ 28 Go. |
| **Garder les fiches de Legifacile dans `docs/phases/P9-…`** | Laisse une « P9 » qui parle de Legifacile dans le plan de Crible, exactement ce que la séparation supprime, et donne à un autre projet un jalon de Crible. |
| **Mettre les fiches de Legifacile hors du dépôt (notes du propriétaire)** | Perd la traçabilité, contraire au principe 7 de `docs/01:87`. Le document de passation garde les fiches datées et le contrat, sans les faire entrer dans le plan. |
| **Réserver « v1 » au service Legifacile** et parler de « fin du plan (J10) » pour Crible | Plus clair pour le propriétaire, mais rend ambigus trois titres d'ADR acceptés (ADR-0004, ADR-0015, ADR-0017) et obligerait à relire tout le corpus documentaire. La règle de correspondance bornée coûte moins. |
| **Placer le tableau de bord après la validation** | Le propriétaire a choisi l'inverse le 2026-11-02. De plus, la numérotation P9 avant P10 et le principe « une seule phase ouverte » (`docs/09:88`) ne permettraient pas cet ordre sans renuméroter à nouveau. |
| **Déplacer `MESURER-EN-CONTENEUR-MAC` en P10** (option B de l'analyse) | Laisse P8 sans mesure et sans conteneur éprouvé ; l'image n'aurait été testée nulle part avant l'installation chez le propriétaire. |
| **Dédoubler la mesure en deux tâches de même nature** | Fait deux fiches pour un même protocole. La variante retenue garde la tâche P8 (indicative) et ajoute une tâche nouvelle et distincte en P10 (`MESURER-SUR-MAC-MINI`), ce qui nomme clairement la mesure contraignante. |
| **Ajouter un mode « import pgvector » dans Crible** | Ferait entrer le compositeur et le découpeur de Legifacile dans le moteur, alors que `docs/01:51` et `docs/04:312-315` les placent chez le producteur. L'export est un script ponctuel côté Legifacile, comme le précédent de P2. |
| **Remplacer ADR-0004, ADR-0032 ou ADR-0046 par de nouveaux ADR** | Inutile : leurs décisions ne dépendent pas du libellé de la machine ni du numéro de phase. Un ADR accepté ne se réécrit pas ; la règle de correspondance suffit et garde l'historique. |
| **Annoncer une compatibilité durable du client Python (PyPI, semver)** | Engagement que rien ne soutient aujourd'hui. Le tag épinglé par le nouveau dépôt est la garantie réelle, et `docs/01:85` interdit la fonctionnalité spéculative. |

## Références

**Documents de conception**

- `docs/01-vision-et-perimetre.md` `:45`, `:51`, `:59-66`, `:72`, `:85`, `:87`
- `docs/02-architecture.md` `:43`
- `docs/03-recherche.md` `:140`, `:144`, `:147`, `:166`
- `docs/04-donnees-et-ingestion.md` `:312-315`, `:370-379`
- `docs/07-stack-et-deploiement.md` `:114`, `:116-145`
- `docs/08-invariants-et-tests.md` `:65`, `:71-73`, `:86`, `:88`
- `docs/09-plan-de-developpement.md` `:1-5`, `:68-79`, `:81-88`
- `docs/format-lot-v1.md`
- `docs/suivi.md` `:20-21`, `:25`, `:29-31`, `:34`, `:133-149`
- `docs/phases/P2-encodeur-et-recette.md` `:34`, `:42`
- `docs/phases/P4-recherche-rapide.md` `:109`
- `docs/phases/P7-observabilite.md` `:125`
- `docs/phases/P8-docker-et-deploiement.md` `:3-9`, `:131`, `:168-173`
- `docs/phases/P9-migration-legifacile.md` `:1-136`

**ADR**

- ADR-0004 (pas d'ANN en v1), ADR-0010 (découpage chez le producteur), ADR-0012, ADR-0015, ADR-0016, ADR-0017
- ADR-0032 (lecture seule en production), ADR-0036 (`producer.host`), ADR-0039, ADR-0040
- ADR-0046 (index de plage), ADR-0047 (cible de reconstruction), ADR-0064, ADR-0065 (instantané et restauration)
- ADR-0076, ADR-0077, ADR-0078 (N = 5000), ADR-0088, ADR-0111 (OpenAPI), ADR-0112 (client Python), ADR-0116 (lecture sans écriture)
- ADR-0127, ADR-0128 (précédents de propositions hors tâche), ADR-0130 (tableau de bord, à rédiger)

**Mesures**

- MES-00 (pgvector, Mac mini, 2026-10-20)
- MES-01 (fidélité sur l'échantillon), MES-02, MES-03, MES-05, MES-06, MES-07, MES-09, MES-14

**Question ouverte**

- Q-07, résolue par cet ADR à son acceptation.
