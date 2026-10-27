# ADR-0079 — Coût des clés au rescoring

- **Statut** : Proposé
- **Date** : 2026-10-27
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, sur demande du propriétaire, à partir de la mesure MES-03 (`docs/mesures/MES-03-latence-rescoring.md`) et de l'analyse de `@architecte` reprise par l'orchestrateur
- **Remplace** : —

## Contexte

### La question

MES-03 mesure le rescoring fp16 selon le protocole d'ADR-0077 et constate que le budget de `docs/03-recherche.md:144` n'est pas tenu : **29,8 ms** en p50 à N = 1000 (p95 90,5) là où le budget promet « < 1 ms à chaud », et **61,1 ms** (p95 186,9) à N = 5000, valeur que le défaut retient depuis ADR-0078. Or le calcul seul coûte **0,36 ms** à N = 1000 et **2,04 ms** à N = 5000. L'écart n'est donc pas arithmétique : il vient de la mémoire, c'est-à-dire des deux lectures aléatoires faites par candidat, la ligne `f16` du segment et la ligne de `records.sqlite` qui porte la clé.

La question tranchée ici est : **comment lire la clé d'un candidat au rescoring, et dans quel ordre traiter les pistes, pour ramener ce coût sans changer le résultat.**

### Ce que la mesure établit

Chiffres de MES-03, mesurés sur `synth-large` (7,74 M vivants), moteur natif sur le Mac :

| Poste | N = 1000 | N = 5000 |
|---|---|---|
| rescoring complet, p50 (p95) | **29,8 ms** (90,5) | **61,1 ms** (186,9) |
| produit scalaire seul, pages chaudes | 0,36 ms | 2,04 ms |
| lecture des clés seule, pages déjà chaudes | 4,0 ms | 20,1 ms |
| lecture des clés seule, sans le moteur devant elle | 50,4 à 60,8 ms | — (non mesurée) |
| temps non attribué à un poste (`:207-215`, `:232-234`) | **25,4 ms, soit 85 %** | — |
| froid : moteur entier | 179,0 ms | — |
| froid : produit scalaire seul, pages absentes | 85,0 ms | 177,5 ms |

Deux réserves de la mesure comptent pour cet ADR. D'abord, **25,4 ms sur 29,8 ne sont attribués à aucun poste** : les répliques sont mesurées après le moteur, donc à cache chaud, et l'ordre inverse n'a pas été mesuré. MES-03 (`:232-234`, `:246-247`) demande explicitement un **profil par échantillonnage** avant toute répartition, et déclare que « la première étape reste de profiler le rescoring pour confirmer la part des deux lectures ». Ensuite, le terme « clés » varie d'un facteur 12 selon l'état du cache : 4,0 ms à chaud, 50,4 à 60,8 ms sans le moteur devant. Ce que la clé coûte réellement dans le chiffre moteur n'est donc pas connu à ce jour.

### Le chemin actuel du code

Une requête SQLite par candidat :

- `crates/crible-search/src/dense/rescore.rs:110` appelle `key_of` (`crates/crible-search/src/dense/exact.rs:372-377`) à chaque tour de boucle, en même temps que le score ;
- `key_of` appelle `RecordStore::read_passage_key` (`crates/crible-store/src/records.rs:1019`) ;
- celui-ci passe par `with_connection` (`records.rs:981-994`), soit **un emprunt de connexion par appel** ;
- puis `passage_key_by_iid` (`records.rs:1445-1451`) exécute `SELECT doc_id, ord FROM passages WHERE iid = ?1` ;
- une `String` est allouée par candidat.

Côté vecteur, `cosine` (`crates/crible-search/src/dense/mod.rs:151-160`) appelle `SegmentReader::vector_f16` (`crates/crible-store/src/segment.rs:571`), qui lit `row_bytes` (`segment.rs:691`) sur le mmap puis `decode_vector_f16` (`segment.rs:1050`), lequel **alloue et copie 1 536 octets par candidat**.

EXACT, lui, saute déjà la lecture de la clé quand le score ne peut pas battre le pire du tas (`exact.rs:316-341`). Le rescoring ne connaît pas cette économie.

### Les contraintes qui pèsent

- **Le tri final est total et textuel.** Le classement de `Scored` ordonne par score décroissant, puis par clé textuelle croissante (ADR-0045, ADR-0072). L'ordre ne dépend donc ni de l'ordre d'itération des candidats, ni de l'iid.
- **Egalité au bit près avec EXACT.** ADR-0072, décision 3, exige des scores et un ordre identiques au bit près entre EXACT et le rescoring. INV-06 en est le test de référence (« pool ≤ seuil exact ⇒ résultat identique au calcul exact »).
- **L'iid ne sort jamais du moteur** (`AGENTS.md`, règle 11) : une clé affichée ou triée est toujours la clé externe `doc_id#ord`, jamais l'iid.
- **La règle de mémoire de `docs/02-architecture.md:204`** : « ce qui est parcouru en entier à chaque requête est en RAM ; ce qui est lu seulement pour quelques candidats reste sur disque (mmap) ». Les clés sont lues pour quelques candidats : par la règle, elles restent sur disque.
- **Le budget mémoire est tendu.** MES-09 mesure **1,12 Gio** possédés par index de 7,74 M passages et **2,20 à 2,30 Gio** pour une instance avec la session fp32, soit 10 à 15 % au-dessus de la conception. Toute pièce ajoutée en RAM est un nouvel arbitrage.
- **La règle d'imbrication de `rescore.rs`** : la connexion SQLite ne doit pas être tenue pendant les lectures de vecteurs. Toute lecture groupée doit respecter cette discipline.
- **`dedup` et `group_by` changent le nombre de clés nécessaires.** ADR-0075 fait consommer par le regroupement plus de candidats que `k` résultats : le nombre de clés à lire n'est pas `k`, mais le nombre de candidats que le regroupement consomme.

### Ce que cet ADR ne traite pas

L'**accès aléatoire aux vecteurs fp16** : 1 536 octets par vecteur, lus à la granularité de page (16 Kio sur Apple Silicon), dans un fichier de 12,3 Gio, soit 85,0 ms à froid pour le seul produit scalaire. Son remède profond touche le format et l'ordre physique des segments (ADR-0006, ADR-0069) et relève d'un ADR séparé. Seule la mesure commune aux deux sujets — parcourir les candidats dans l'ordre des iid — vit ici.

## Décision

> Statut **Proposé** : ce qui suit est la décision **proposée**, à trancher par le propriétaire. Rien n'est appliqué aux documents de conception tant que l'ADR n'est pas accepté.

**La décision proposée est de traiter le coût des clés dans cet ordre : d'abord un profil et deux mesures neutres (e), puis la lecture des clés après le tri par score (b), puis la lecture des clés par lot (a). Les clés en RAM (c) et la colonne de clés en mmap (d) sont écartées pour l'instant, et rejugées après le profil.**

Précisions :

1. **Profiler d'abord.** Un profil par échantillonnage du rescoring, sans changement de comportement, attribue les 25,4 ms non expliquées à N = 1000 entre la lecture de la clé, le calcul et le reste (allocation de `String`, emprunt de connexion, copie du vecteur, tri). Le profil est un préalable : sans lui, on ignore si le terme « clés » vaut 4,0 ms (cache chaud) ou 50,4 à 60,8 ms (sans le moteur devant), et donc si (b) peut suffire. Le profil ne change ni l'API, ni le format, ni les résultats.
2. **Deux mesures neutres, peu coûteuses.** Avant toute restructuration : parcourir les candidats dans l'**ordre croissant des iid** avant la boucle de rescoring, pour la localité des pages de `records.sqlite` et des vecteurs — le résultat ne change pas, le tri final étant total ; et **emprunter la ligne fp16 au mmap** au lieu de la copier, quand le décodeur peut travailler sur une vue. Ces deux changements sont mesurés séparément, puis ensemble.
3. **Clés après le tri par score (b).** Lire les clés seulement pour les candidats que le classement retient : les `k` résultats, les candidats consommés par le regroupement s'il y en a (ADR-0075), et **tous les candidats dont le score égale le score de la frontière**. Sans cette dernière règle, l'ensemble retenu dépendrait de l'ordre d'itération et ADR-0072 décision 3 tomberait. Le tri intermédiaire porte alors sur `(score, iid)`, l'iid ne servant qu'à départager de façon déterministe en interne et ne sortant jamais du moteur ; le tri final reste `(score décroissant, clé textuelle croissante)`. N lectures deviennent environ `k` plus les égalités, soit un facteur 20 à 50, et le rescoring s'aligne sur l'économie déjà pratiquée par EXACT (`exact.rs:316-341`). Le seul vrai risque est la règle des égalités ; un test à vecteurs dupliqués à la frontière le couvre.
4. **Clés par lot en repli (a).** Si le profil montre que le coût est dans les N allers-retours et les N emprunts de connexion plutôt que dans le nombre de lignes lues, `passage_key_by_iid` est remplacé par une requête unique pour les N iid, découpée selon la limite de variables de SQLite, exécutée hors des lectures de vecteurs (règle d'imbrication de `rescore.rs`). (a) supprime N allers-retours et N emprunts sans toucher ni à l'ordre ni aux bits. Son gain n'est pas chiffré : la borne haute est le terme « clés » mesuré seul, 4,0 à 20,1 ms à chaud.
5. **(c) et (d) écartées pour l'instant.** Les clés en RAM coûtent 86 à 172 Mo à 8,58 M passages (environ 69 Mo en colonnes `u32`/`u32` plus le dictionnaire des `doc_id`), soit 6 à 15 % du budget de MES-09, avec une reconstruction à chaque époque et une nouvelle décision de budget mémoire. La colonne de clés en mmap donne le même gain sans RAM possédée, mais change le format (`format_version`, chemin de reconstruction) et c'est le plus gros coût d'ingénierie des cinq options. Les deux restent recevables après le profil et le rejeu de MES-03.
6. **Validation de toute mise en œuvre.** Quatre contrôles, tous exigés : rejeu de MES-03 au protocole d'ADR-0077 (copie `data/mes02`, N ∈ {1000, 2000, 5000}) ; test d'égalité **au bit près** de RESCORER-FP16 (ADR-0072, décision 3) ; contrôle top-10 du harnais de MES-01, avec **0 écart** ; et test dédié des **égalités de score à la frontière**, avec vecteurs dupliqués, qui vérifie que le résultat ne dépend pas de l'ordre d'itération.
7. **Mise en œuvre.** Une tâche à créer (P4 ou plus tard) si le propriétaire accepte cet ADR. Le profil et les mesures neutres peuvent former une tâche distincte de (b) et (a), pour que leurs chiffres décident de la suite.

Ce qui n'est **pas** décidé ici : l'accès aléatoire aux vecteurs fp16, renvoyé à un ADR séparé ; une nouvelle valeur de `rescore_candidates`, qui appartient à ADR-0078 ; la révision de la règle mémoire de `docs/02`, qui demanderait un ADR si (c) ou (d) était retenue ; et les cibles de latence de P8 en conteneur.

## Conséquences

- **Positives** :
  - le profil remplace une répartition supposée (85 % inexpliqués) par une attribution mesurée, et il est peu coûteux ;
  - les mesures neutres (ordre des iid, emprunt au mmap) ne touchent ni l'ordre, ni les bits, ni le format, et se vérifient par les tests existants ;
  - (b) ramène la lecture des clés d'un facteur 20 à 50 sans ajouter de mémoire possédée, et aligne le rescoring sur l'économie déjà faite par EXACT ;
  - (a) reste disponible comme remède complémentaire, avec un risque nul sur les résultats ;
  - (c) et (d) ne sont pas fermées : elles sont rejugées avec des chiffres, pas écartées par principe.

- **Négatives / coûts** :
  - le profil est un travail de mesure à faire avant le code, et il ne produit aucun gain par lui-même ;
  - (b) déplace la complexité : le tri intermédiaire doit porter sur `(score, iid)` et le tri final sur la clé, avec une règle d'égalités à écrire, à documenter et à tester ; une erreur y change silencieusement le classement ;
  - (b) ne réduit pas les lectures de vecteurs : à froid, le produit scalaire coûte 85,0 ms à N = 1000 et reste le premier poste ;
  - le parcours dans l'ordre des iid ne sert que si les candidats arrivent dispersés ; son gain peut être nul quand le scan les rend déjà croissants ;
  - (a) garde une requête SQLite par lot, donc un coût qui croît avec N ;
  - aucune de ces options ne ramène le rescoring sous le budget « < 1 ms » pris isolément : elles rapprochent le coût du calcul seul, pas en dessous.

- **À surveiller** :
  - si le profil attribue l'essentiel des 25,4 ms au produit scalaire ou aux défauts de page, l'ADR des vecteurs fp16 devient prioritaire et (b) ne suffira pas ;
  - si le terme « clés » mesuré à chaud (4,0 ms) est confirmé par le profil, (a) et (b) ne peuvent pas expliquer les 29,8 ms, et la suite se décide sur le profil, pas sur cette liste d'options ;
  - si le rejeu de MES-03 montre un écart de latence entre l'ordre d'itération naturel et l'ordre des iid, la localité des pages est confirmée et la règle mérite d'être écrite dans `docs/03` ;
  - si le nombre d'égalités de score à la frontière dépasse largement `k` sur un jeu réel, le gain de (b) s'effondre vers N et (a) redevient le levier principal ;
  - si le regroupement (ADR-0075) demande beaucoup plus de candidats que `k`, (b) lit d'autant plus de clés : le facteur 20 à 50 est une borne, pas une promesse ;
  - si (c) ou (d) revient sur la table, le budget de MES-09 (1,12 Gio par index) et la règle de `docs/02:204` devront être révisés par un ADR, pas par une tâche.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Garder une requête SQLite par candidat et ne rien changer | C'est l'état mesuré : 29,8 ms à N = 1000 et 61,1 ms à N = 5000, très au-dessus du budget, sans perspective de tenir un jour N = 5000 |
| Commencer par les clés en RAM (c) | 86 à 172 Mo à 8,58 M passages, soit 6 à 15 % du budget de MES-09, plus une reconstruction à chaque époque, pour un gain que le profil n'a pas encore attribué aux clés |
| Commencer par la colonne de clés en mmap (d) | Même gain que (c), mais change `format_version` et le chemin de reconstruction : le plus gros coût d'ingénierie pour un gain non chiffré dans le chiffre moteur |
| Profiler et décider dans la même tâche | Le profil doit pouvoir contredire la liste d'options sans qu'on ait déjà changé le format ou ajouté de la mémoire ; il est donc un préalable isolé |
| Sauter le profil et appliquer (b) directement | MES-03 demande le profil, et les répliques mesurées à cache chaud ne permettent pas d'affirmer que les clés pèsent 4 ms ou 50 ms dans le chiffre moteur |
| Appliquer (b) sans la règle des égalités à la frontière | Le jeu retenu dépendrait alors de l'ordre d'itération : l'ordre et les scores d'EXACT et du rescoring cesseraient d'être identiques au bit près (ADR-0072, décision 3, et INV-06) |
| Trier d'abord par clé, puis par score | Inverse la priorité du tri final (score d'abord, clé ensuite ; ADR-0045, ADR-0072) et lit N clés, donc le coût qu'on veut supprimer |
| Trier sur l'iid pour départager les égalités | L'iid n'a pas de sens hors du moteur (règle 11) et son ordre n'est pas l'ordre textuel attendu : `doc#10` passe avant `doc#2` en texte |
| Ordonner les candidats par iid et s'arrêter là | Ne change ni les N requêtes SQLite, ni les N `String` allouées ; conserve au mieux le gain de localité, qui n'est pas chiffré |
| Lire les clés en une seule requête sans découpage | La limite de variables de SQLite est atteinte bien avant 5000 : la requête échouerait à N = 5000 ou sur un k élevé |
| Tenir la connexion SQLite pendant les lectures de vecteurs | Contredit la règle d'imbrication de `rescore.rs` et rend la latence dépendante du verrou d'écriture |
| Mettre la clé textuelle complète en RAM | 100 à 160 Mo à 8 M passages d'après MES-03, plus que les colonnes `u32`/`u32` de (c), pour un besoin qui est le classement, pas l'affichage |
| Traiter ici l'accès aléatoire aux vecteurs fp16 | Le remède profond touche le format et l'ordre physique des segments (ADR-0006, ADR-0069) : c'est un ADR séparé, et le mêler ici empêcherait de trancher les clés seules |
| Baisser N au lieu de traiter les clés | N = 5000 est décidé (ADR-0078) pour la fidélité ; baisser N dégraderait le rappel au lieu de réduire un coût parasite |

## Références

- Mesure : `docs/mesures/MES-03-latence-rescoring.md` — chiffres du tableau `:178-189`, conclusion `:195-227`, réserves `:229-238`, proposition `:240-247` ; détail par requête dans `MES-03-latence-rescoring.2026-10-27.json` et `data/mes02/raw/`.
- Protocole de mesure : ADR-0077 (N ∈ {1000, 2000, 5000}, froid par copies non cachées, 300 requêtes, RSS en trois états).
- Code : `crates/crible-search/src/dense/rescore.rs:110` (boucle et `key_of`) ; `crates/crible-search/src/dense/exact.rs:316-341` (économie de clé d'EXACT), `:372-377` (`key_of`) ; `crates/crible-store/src/records.rs:981-994` (`with_connection`), `:1019` (`read_passage_key`), `:1445-1451` (`passage_key_by_iid`) ; `crates/crible-search/src/dense/mod.rs:151-160` (`cosine`) ; `crates/crible-store/src/segment.rs:571` (`vector_f16`), `:691` (`row_bytes`), `:1050` (`decode_vector_f16`).
- Conception : `docs/02-architecture.md:204` (règle de mémoire, vecteurs en mmap) ; `docs/03-recherche.md:144` (budget du rescoring) ; `docs/08-invariants-et-tests.md` (INV-06, MES-03).
- Autres mesures : `docs/mesures/MES-01-fidelite-scan-binaire.md` (harnais top-10) ; `docs/mesures/MES-09-ram-reelle.md:101`, `:107-108`, `:119-120`, `:145-147` (1,12 Gio par index, 2,20 à 2,30 Gio par instance).
- ADR liés : ADR-0004 (codes binaires, scan du pool et rescoring), ADR-0006 (segments, pierres tombales, compaction), ADR-0045 (référence exacte, clause arithmétique remplacée par ADR-0072), ADR-0069 (reconstruction et format de `codes.bin`, précédent de changement de format), ADR-0072 (produit scalaire unique, égalité au bit près), ADR-0074 (seuil exact et stratégie dense), ADR-0075 (regroupement et sur-échantillonnage), ADR-0078 (N = 5000 par défaut).
