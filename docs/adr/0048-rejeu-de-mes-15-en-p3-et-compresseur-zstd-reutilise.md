# ADR-0048 — Rejeu de MES-15 en P3 et compresseur zstd réutilisé

- **Statut** : Accepté (décision du propriétaire le 2026-10-22)
- **Date** : 2026-10-22 (proposition et acceptation)
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, sur demande du propriétaire. Décision « D3 » de `docs/suivi.md`, section « Décisions en attente », issue de MES-15 (tâche hors plan `MESURER-COUT-COMMIT`, P1)
- **Remplace** : —

## Contexte

MES-15 a mesuré le coût d'un commit de génération et proposé trois remèdes. Le propriétaire a tranché la suite à leur donner : un rejeu de la mesure en P3, une tâche pour le compresseur zstd, et rien pour le cache des métadonnées de segments. Cet ADR fixe cette suite, parce qu'elle change `docs/08` (phase d'une mesure) et le plan de P3 (une tâche nouvelle et une fiche de mesure étendue).

**Ce que la mesure dit.** Le rapport `docs/mesures/MES-15-cout-commit-generation.md` mesure un commit type — 1 segment neuf de 2 000 passages, 1 000 documents, 50 pierres tombales — à 8,58 M passages vivants, cache chaud, 10 répétitions par point (§ 3, § 4) :

| Grandeur | Valeur mesurée | Source |
|---|---|---|
| Commit complet (p50) | 598,3 ms (S = 10) et 641,3 ms (S = 1 000) | § 3 |
| Matérialisation des iid vivants | 517,8 ms (86,5 %) à S = 10 ; 534,2 ms (83,3 %) à S = 1 000 | § 4 |
| Transaction SQLite (corps) | 77,6 à 100,4 ms, constante en `N` | § 4 |
| Compression des textes (`encode_all` par passage) | 67,2 à 89,9 ms par attache de 2 000 passages, soit 34 à 45 µs par passage | § 4 |
| Mêmes textes, `zstd::bulk::Compressor` réutilisé | 5,9 à 6,1 ms (point de contrôle du banc, pas le code de production) | § 4 et § 6 |
| `COMMIT` SQLite | 1,4 à 1,5 ms, constant | § 4 |
| Relecture d'un `segment.json` | 12,8 à 15,2 µs par segment | § 4 |

La conclusion du § 6 est nette : le commit est en O(N) et en O(S) ; à l'échelle de Legifacile, **c'est la matérialisation des iid vivants qui porte le coût** (0,9 ms à 10 k passages, 62,7 ms à 1 M, 517,8 ms à 8,58 M), et la **transaction est dominée par la compression zstd d'une trame par passage**, alors que les mêmes textes avec un contexte réutilisé coûtent environ six millisecondes.

**Le budget.** Il n'existe aucune cible chiffrée pour MES-08 ni pour MES-15 dans `docs/08`. Le seul budget écrit est celui du client : un job d'attache avec `?wait=true` attend au plus `wait_timeout_ms`, 60 s par défaut (`docs/05`). Le commit mesuré en consomme 0,1 à 0,6 s, soit 0,2 à 1 % (§ 6). La contrainte réelle n'est donc pas ce délai, mais la durée pendant laquelle l'écrivain unique de l'index est sérialisé, et la durée d'un import complet, où la compression se paie une fois par passage.

**Recommandation 1 — dérivation incrémentale de la bitmap des vivants : déjà implémentée.** Constat de l'orchestrateur, non une mesure : la tâche `CONSTRUIRE-BITMAPS` (statut `FAIT`) a livré cette dérivation. `build_snapshot_after_commit` (`crates/crible-store/src/generation.rs`) appelle `FilterIndex::after_commit` quand la génération suit la précédente, et le `Snapshot` ne garde plus de `Vec<Iid>`. **Le gain n'est pas mesuré** : MES-15 n'a pas été rejouée depuis. La campagne du 2026-10-21 décrit donc un état de code qui n'est plus celui du dépôt sur ce point, alors que ses chiffres continuent de servir de référence.

**Recommandation 2 — cache des `SegmentMeta` : non implémentée.** Constat de l'orchestrateur : `segments_of` relit tous les `segment.json` à chaque commit. Le calcul du § 6 donne environ 4,7 ms à 366 segments (366 × 12,8 µs), soit un an d'attaches quotidiennes sans compaction, et la mesure donne 12,8 à 15,2 ms à 1 000 segments. Le § 6 conclut : « À ne retenir que si `S` doit dépasser quelques centaines. »

**Recommandation 3 — compresseur zstd réutilisé : non implémentée.** Constat de l'orchestrateur : `records::encode_text` (`crates/crible-store/src/records.rs`) appelle `zstd::stream::encode_all`, qui construit un contexte par passage. Calcul, non mesuré : à 34 à 45 µs par passage, un import complet de 8,58 M passages coûte environ 5 à 6 min de compression, contre environ 25 s avec un compresseur réutilisé. Le point de contrôle du banc borne le gain d'un commit de 2 000 passages (67 à 90 ms contre 5,9 à 6,1 ms), mais **aucun chiffre ne porte sur le code de production**.

**Ce que la mesure ne dit pas.** MES-15 ne remplace pas MES-08 : la recette, l'écriture du segment et l'étape tantivy de `docs/02` sont hors du commit mesuré (§ 6). Elle ne dit rien du cache froid de façon contrôlée (§ 7.3), rien de fin sur `S`, et rien du gain réel de la dérivation des iid vivants, faute d'un rejeu après `CONSTRUIRE-BITMAPS`.

**Précédent de rédaction.** `docs/08` classe déjà MES-13 et MES-14 sur deux phases (ADR-0047) : premier chiffre dans la phase qui en a besoin, mesure complète dans la phase qui la consomme. MES-15 suit maintenant le même modèle, et ADR-0047 sert de précédent de forme.

## Décision

La suite donnée aux trois recommandations est : **rejeu de MES-15 en P3**, **une tâche `REUTILISER-COMPRESSEUR-ZSTD` en P3**, et **aucune tâche pour le cache des `SegmentMeta`**.

1. **Rejeu de MES-15 en P3.** La colonne de phase de MES-15 passe de « P1 » à « **P1, P3** » dans `docs/08` : le premier chiffre est celui de la campagne du 2026-10-21, le rejeu est en P3. La fiche `MESURER-CYCLE-DE-VIE` de `docs/phases/P3-cycle-de-vie.md` ajoute MES-15 à ses mesures (objectif, livrables, ligne « Invariants / mesures »). Le livrable est précis : rejouer MES-15 avec le **même banc** (`crates/crible-store/benches/commit_cost.rs`), aux **mêmes points `N` et `S`** que la campagne du 2026-10-21, et comparer à cette campagne, pour chiffrer la dérivation incrémentale de la bitmap des vivants. Le rejeu a lieu en P3, après `REUTILISER-COMPRESSEUR-ZSTD`, pour qu'une seule campagne mesure les deux changements, la décomposition du § 4 les séparant.

2. **Nouvelle tâche `REUTILISER-COMPRESSEUR-ZSTD` en P3**, placée juste avant `MESURER-CYCLE-DE-VIE`. La compression des textes de passages réutilise un compresseur zstd, au même niveau de compression (`TEXT_COMPRESSION_LEVEL`), au lieu de construire un contexte par passage.
   - **Pas de changement de format** : le schéma de `records.sqlite`, `format_version` et la nature de la colonne `passages.text` (une trame zstd) ne changent pas.
   - **Les textes déjà écrits se relisent sans changement** : aucune migration, aucune réécriture.
   - **Les trames ne sont pas forcément identiques octet pour octet** : l'en-tête zstd peut porter la taille du contenu. Le critère porte donc sur la **décompression**, jamais sur l'égalité des trames.
   - **Critères d'acceptation** : un test d'aller-retour (le texte compressé puis décompressé est égal à l'original, y compris un texte vide et un texte long) ; un test qui relit une trame produite par l'ancienne méthode (`zstd::stream::encode_all`) ; le gain chiffré par MES-08 dans `MESURER-CYCLE-DE-VIE`.
   - **Dépend de** : `STOCKER-ENREGISTREMENTS-SQLITE` (`FAIT`).

3. **Cache des `SegmentMeta` : pas de tâche.** L'orchestrateur note l'idée dans « Idées hors périmètre » de `docs/suivi.md`, avec sa condition de réouverture : le nombre de segments d'un index devra dépasser quelques centaines, d'après les mesures de compaction de P3.

Ce qui n'est **pas** décidé ici : aucun seuil, ni `N`, ni `exact_threshold`, ni `TEXT_COMPRESSION_LEVEL` ; aucune forme de cache des métadonnées de segments, ni son invalidation ; aucun changement du protocole de commit ni du format sur disque ; aucune cible chiffrée nouvelle pour MES-08 ou MES-15.

## Conséquences

- **Positives** :
  - le coût du commit redevient connu sur le code réel : la campagne du 2026-10-21 décrit un `Snapshot` qui garde encore un `Vec<Iid>`, et personne ne sait aujourd'hui ce que la dérivation a retiré des 517,8 ms ;
  - le rejeu est planifié là où il sert : en P3, avant la compaction et l'attache quotidienne, et non en P1, où aucun consommateur ne l'attend ;
  - le remède zstd est borné, local et sans changement de format : une fonction, un test d'aller-retour, un test de relecture, et un gain chiffré par la mesure d'attache qui existe déjà ;
  - la liste des remèdes cesse d'être une liste d'idées : chaque recommandation de MES-15 a désormais un sort écrit — faite, planifiée, ou conditionnée ;
  - une seule campagne de rejeu mesure les deux changements, au lieu de deux campagnes.
- **Négatives / coûts** :
  - MES-15 porte deux phases : la ligne de `docs/08` oblige à lire `docs/mesures/MES-15-cout-commit-generation.md` pour savoir ce qui est déjà fait ;
  - le gain de la dérivation des iid vivants reste inconnu jusqu'en P3, alors qu'il porte 86,5 % du commit mesuré : d'ici là, tout chiffre cité sur le coût d'un commit est celui de l'ancien code ;
  - la fiche `MESURER-CYCLE-DE-VIE` s'alourdit : trois mesures en P3, chacune avec son protocole, et un rejeu qui exige de reconstruire les index de `data/mes15/` (6,3 Go, gitignorés) sur la machine de campagne ;
  - `MESURER-CYCLE-DE-VIE` ne déclare pas `REUTILISER-COMPRESSEUR-ZSTD` dans « Dépend de » : l'ordre des deux fiches n'est porté que par leur placement dans le fichier, et l'orchestrateur doit le respecter ;
  - l'import complet n'est pas mesuré par MES-08, qui porte sur 1 000 à 3 000 passages : les 5 à 6 min calculées pour 8,58 M resteront un calcul, sauf mesure dédiée en P9.
- **À surveiller** :
  - le commit p50 du rejeu, aux points `N` et `S` de la campagne : un gain très inférieur aux 517,8 ms attendus indiquerait que la lecture SQLite ou la transaction dominent désormais, et les remèdes suivants changeraient de cible ;
  - le nombre de segments d'un index après les compactions de P3 : s'il dépasse quelques centaines, l'idée du cache des `SegmentMeta` se rouvre avec son calcul de 12,8 à 15,2 µs par segment ;
  - le gain de `REUTILISER-COMPRESSEUR-ZSTD` sur l'attache quotidienne : si MES-08 le trouve inférieur au point de contrôle du banc, la différence viendra du coût d'écriture SQLite, et c'est là qu'il faudra regarder ;
  - la durée d'un import complet en P9 : les 5 à 6 min calculées aujourd'hui sont à confirmer par une mesure, et non à citer comme un chiffre.

### Modifications à appliquer après acceptation

Aucune n'est appliquée par cet ADR : le propriétaire doit d'abord le passer en `Accepté`. Une fois accepté, `@greffier` applique les modifications suivantes, dans un commit qui cite `ADR-0048`.

1. **`docs/08-invariants-et-tests.md`**, ligne MES-15 du tableau des mesures. Remplacer

   ```
   | MES-15 | Coût d'un commit de génération selon le nombre de passages vivants (matérialisation des iid vivants, relecture des `segment.json`) | P1 |
   ```

   par

   ```
   | MES-15 | Coût d'un commit de génération selon le nombre de passages vivants (matérialisation des iid vivants, relecture des `segment.json`) | P1, P3 |
   ```

   Aucune autre ligne du tableau ne change ; la ligne MES-08 reste « P3, P9 ».

2. **`docs/phases/P3-cycle-de-vie.md`** : insérer la fiche suivante **juste avant** `### MESURER-CYCLE-DE-VIE`, et avant aucune autre fiche.

   ```
   ### REUTILISER-COMPRESSEUR-ZSTD
   - **Objectif** : compresser les textes de passages avec un compresseur zstd réutilisé, au même niveau de compression, au lieu de construire un contexte par passage.
   - **Dépend de** : STOCKER-ENREGISTREMENTS-SQLITE
   - **À lire** : `02-architecture.md` (table `passages`, colonne `text` en zstd) ; `docs/mesures/MES-15-cout-commit-generation.md`, § 4 (tableau de la compression des textes) et § 6 (recommandation 3).
   - **Livrables** :
     - `crible-store::records` : `encode_text` n'appelle plus `zstd::stream::encode_all` ; la compression passe par un compresseur zstd réutilisé (`zstd::bulk::Compressor`), au niveau `TEXT_COMPRESSION_LEVEL` inchangé ;
     - le compresseur est créé une fois pour l'écriture d'un lot de passages, et non une fois par passage ;
     - aucun changement de schéma, de `format_version` ni de nature de la trame stockée.
   - **Critères d'acceptation** :
     - test d'aller-retour : le texte compressé puis décompressé est égal à l'original, y compris pour un texte vide et pour un texte long ;
     - test de relecture : une trame produite par l'ancienne méthode (`zstd::stream::encode_all`) se décompresse sans changement ;
     - les trames ne sont pas comparées octet pour octet : l'en-tête zstd peut porter la taille du contenu, et le critère porte sur la décompression ;
     - le gain sur une attache de 2 000 passages est chiffré par MES-08, dans `MESURER-CYCLE-DE-VIE`.
   - **Invariants / mesures** : MES-08, MES-15.
   - **Hors périmètre** : changer `TEXT_COMPRESSION_LEVEL` ; changer le schéma, `format_version` ou la nature de la trame ; compresser autre chose que `passages.text` ; ajouter une dépendance ; réécrire les textes déjà stockés.
   ```

3. **`docs/phases/P3-cycle-de-vie.md`**, fiche `MESURER-CYCLE-DE-VIE`, section « Objectif ». Remplacer

   ```
   - **Objectif** : MES-07, MES-08 et MES-13.
   ```

   par

   ```
   - **Objectif** : MES-07, MES-08, MES-13 et MES-15.
   ```

4. **`docs/phases/P3-cycle-de-vie.md`**, même fiche, section « Livrables ». Remplacer

   ```
     - attache d'un lot quotidien type de 1 000 à 3 000 passages ;
   ```

   par

   ```
     - attache d'un lot quotidien type de 1 000 à 3 000 passages, avec le gain du compresseur zstd réutilisé chiffré (ADR-0048) ;
   ```

   Puis, après le livrable sur l'ordre des remèdes (ADR-0047), ajouter

   ```
     - rejeu de MES-15 avec le même banc (`crates/crible-store/benches/commit_cost.rs`) et les mêmes points `N` et `S` que la campagne du 2026-10-21, comparé à cette campagne, pour chiffrer la dérivation incrémentale de la bitmap des vivants (ADR-0048).
   ```

5. **`docs/phases/P3-cycle-de-vie.md`**, même fiche, section « Critères d'acceptation ». Après le dernier critère (celui qui porte `AJOUTER-CACHE-BITMAPS`, ADR-0047) et avant la ligne « Invariants / mesures », ajouter

   ```
     - le rapport donne, aux points `N` et `S` de la campagne du 2026-10-21, le commit p50 du rejeu de MES-15 et son écart avec cette campagne, et il chiffre ainsi la dérivation incrémentale de la bitmap des vivants (ADR-0048) ;
     - le rapport chiffre le gain de `REUTILISER-COMPRESSEUR-ZSTD` sur l'attache d'un lot quotidien type, par comparaison avec la compression mesurée par MES-15 (ADR-0048).
   ```

6. **`docs/phases/P3-cycle-de-vie.md`**, même fiche, dernière ligne. Remplacer

   ```
   - **Invariants / mesures** : MES-07, MES-08, MES-13.
   ```

   par

   ```
   - **Invariants / mesures** : MES-07, MES-08, MES-13, MES-15.
   ```

7. **`docs/suivi.md`** : c'est l'**orchestrateur** qui tient ce fichier, pas `@greffier`. Une fois l'ADR accepté, il y fait quatre choses.
   - Dans le tableau des tâches, ajouter la ligne de P3 `REUTILISER-COMPRESSEUR-ZSTD`, statut `À FAIRE`, dépendance `STOCKER-ENREGISTREMENTS-SQLITE` (`FAIT`), placée juste avant `MESURER-CYCLE-DE-VIE`.
   - Dans « Idées hors périmètre », ajouter l'idée conditionnelle : cache des `SegmentMeta` (`segments_of` relit tous les `segment.json` à chaque commit ; 12,8 à 15,2 µs par segment, environ 4,7 ms à 366 segments, calculé), **à rouvrir seulement si le nombre de segments d'un index dépasse quelques centaines**, d'après les mesures de compaction de P3 (ADR-0048).
   - Dans « Idées hors périmètre », marquer la ligne du 2026-10-21 « `GERER-GENERATIONS`, mesuré par MES-15 » comme **traitée** : dérivation incrémentale livrée par `CONSTRUIRE-BITMAPS` et rejeu planifié en P3 ; cache des `SegmentMeta` renvoyé à l'idée conditionnelle ci-dessus ; compresseur zstd porté par la tâche `REUTILISER-COMPRESSEUR-ZSTD` (ADR-0048).
   - Dans « Décisions en attente », retirer D3 de la liste active, comme pour D1 et D2 : la ligne garde la trace de l'ADR et de sa date de décision.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Rejouer MES-15 tout de suite, avant P2 | Le rejeu ne servirait à rien maintenant. La mesure n'a pas de consommateur avant P3 : ni la compaction, ni l'attache quotidienne ne sont ouvertes. Surtout, un rejeu immédiat ne mesurerait que la dérivation des iid vivants : le compresseur zstd n'est pas encore changé, et il faudrait une seconde campagne plus tard. Le même rejeu, placé en P3 après `REUTILISER-COMPRESSEUR-ZSTD`, chiffre les deux changements en une campagne, et la décomposition du § 4 les sépare. Enfin, il faudrait reconstruire les index de `data/mes15/` sur une machine dont la campagne du 2026-10-21 a déjà montré qu'elle n'était pas au repos. |
| Se contenter de constater, sans rejeu planifié ni tâche zstd | Les chiffres resteraient ceux d'un code qui n'existe plus pour les iid vivants : 86,5 % du commit mesuré n'auraient plus de valeur connue, et aucun document ne dirait ce qu'il en reste. La compression, elle, coûte 67 à 90 ms par attache de 2 000 passages et 5 à 6 min calculées pour un import complet, pour un remède local, sans changement de format, dont le gain est déjà borné par un point de contrôle du banc. Constater sans agir laisserait les deux coûts en place et la liste des recommandations ouverte indéfiniment. |
| Ouvrir maintenant une tâche de cache des `SegmentMeta` | Le coût est du second ordre : 0,36 ms à 10 segments, 12,8 à 15,2 ms à 1 000 segments, et environ 4,7 ms à 366 segments. Le § 6 de MES-15 le conditionne explicitement à un `S` qui dépasse quelques centaines. Le cache ajoute un état dérivé à maintenir entre générations, pour un gain qui n'apparaît qu'avec des milliers de segments. L'idée est notée avec sa condition, et les mesures de compaction de P3 diront si elle se rouvre. |

## Références

- `docs/mesures/MES-15-cout-commit-generation.md`, § 3 (commit complet), § 4 (décomposition, compression des textes, coûts unitaires) et § 6 (conclusion et recommandations 1 à 3).
- `docs/08-invariants-et-tests.md`, tableau des mesures : MES-15 (« P1 », à passer en « P1, P3 »), MES-08 (« P3, P9 », durée d'une attache quotidienne), MES-13 et MES-14 (« P1, P3 » et « P1, P4 », précédents de rédaction).
- `docs/phases/P3-cycle-de-vie.md`, fiche `MESURER-CYCLE-DE-VIE`, déjà modifiée par ADR-0047 (livrables et critères du démarrage).
- `docs/09-plan-de-developpement.md`, « Format d'une fiche de tâche ».
- `docs/02-architecture.md`, table `passages` (colonne `text` en zstd), « Générations et point de commit ».
- `docs/suivi.md`, « Décisions en attente » (D3) et « Idées hors périmètre » (ligne `GERER-GENERATIONS, mesuré par MES-15`).
- ADR-0016 (le commit SQLite est l'unique point de commit), ADR-0019 (champs en JSON canonique), ADR-0047 (même méthode : phase de mesure, conséquences, modifications à appliquer après acceptation).
