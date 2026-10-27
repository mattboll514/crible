# ADR-0067 — Durée de la recette à l'attache et échéance de `?wait=true`

- **Statut** : Accepté (décision du propriétaire le 2026-10-26)
- **Date** : 2026-10-26 (proposition et acceptation)
- **Décideurs** : propriétaire
- **Décision** : accepté le 2026-10-26 par le propriétaire, tel que proposé : décision 1 retenue (la recette ne change pas : `acceptance_sample` 200, fp32, `acceptance_min_cosine` 0,999, défauts d'ONNX Runtime, recette dans le chemin de l'attache avant toute écriture), décision 2 retenue (un job terminé dans le délai est rendu terminé en `200` ; à l'échéance de `wait_timeout_ms`, `202` avec le job dans son état courant, jamais `504` ni un code `timeout`, le client suit `GET /v1/jobs/{job_id}` ; `wait_timeout_ms` entre dans le tableau des limites de `docs/05` et dans `[limits]` de `docs/07`, 60 000 ms par défaut), décision 3 retenue (tâche hors plan `ACCELERER-RECETTE` en P4, en deux temps : ventilation mesurée de `validation`, puis encodage de la recette par lots seulement si la ventilation le justifie), décision 4 retenue (aucun de ces points ne bloque J3 : la durée de la recette est un fait documenté de la mesure, pas un critère de jalon). Aucun changement de format, de schéma ni d'invariant ; INV-12 reste tenu par la même preuve qu'aujourd'hui.
- **Proposé par** : agent greffier, sur demande de l'orchestrateur. Origine : tâche `MESURER-CYCLE-DE-VIE` (phase P3), campagne de MES-08 du 2026-10-25, point de fin avec le propriétaire du 2026-10-26, faits établis par `@architecte` le 2026-10-26
- **Remplace** : —

> **Note ajoutée le 2026-10-27, sans modification de la décision.** L'issue du second temps de la décision 3 est tranchée par **ADR-0073** : la ventilation a montré que les 200 ré-encodages font 88,0 % et 89,0 % de `validation`, la condition de l'encodage par lots était donc remplie, mais la mesure des tailles de lot ne montre aucun gain (le temps croît avec la taille de lot, et les lignes d'un lot ne s'exécutent pas en parallèle avec les défauts d'ONNX Runtime). Le code du livrable (b) est retiré et la recette reste encodée un passage à la fois ; les fils et réglages d'ONNX Runtime se revoient en conteneur en P8. La décision 3 reste celle qui est écrite ci-dessous : elle a ouvert les deux temps, elle ne promettait pas le gain.

## Contexte

### Ce que la mesure du 2026-10-25 dit

Sur l'échantillon Legifacile, avec le modèle réel `mdenseon`, l'attache d'un lot quotidien tient presque entièrement dans son étape `validation` :

| Lot | `validation` | Attache entière |
|---|---|---|
| 1 000 passages | 51 469 ms | 52,3 s |
| 3 002 passages | 65 923 ms | 68,5 s |

Un ajustement linéaire sur ces deux points donne une part fixe de **44,2 s** et une part de **7,2 ms par passage** ajouté (`docs/mesures/MES-08-duree-attache-quotidienne.md:190-200`). Les 7,2 ms par passage mêlent deux passes complètes du lot (les contrôles, puis `plan_documents`) et le comptage des jetons ; la part fixe est cohérente avec la recette du contrôle 9, qui ré-encode 200 passages quel que soit le lot, soit **≈ 221 ms par passage ré-encodé**.

**Aucune ventilation n'est mesurée.** MES-08 l'écrit noir sur blanc : elle ne dit « rien sur la répartition interne de l'étape `validation` (recette contre comptage de jetons), ni sur une recette à `acceptance_sample` différent » (`MES-08:280-281`). Les 44,2 s sont donc une **déduction sur deux points**, pas un relevé ; le rapport de mesure le dit aussi de la part par passage, dont la cause n'est pas isolée.

Trois ordres de grandeur se comparent mal, et c'est le nœud de la question :

- **le chargement du fp32** coûte ≈ 1,8 s par processus (ADR-0052, décision 1) ;
- **200 ré-encodages** à 221 ms pièce font 44,2 s, si l'attribution de la part fixe est juste ;
- **la recette tombe dans le délai d'attente ou non** : 1 000 passages (52,3 s) tiennent dans `wait_timeout_ms` = 60 s, 3 002 (68,5 s) n'y tiennent pas (`MES-08:256-263`). Le seul délai écrit du projet est d'ailleurs celui-là, et MES-08 rappelle qu'aucun budget de durée n'existe pour elle dans `docs/08`, `docs/03` ni `docs/04` (`MES-08:284-285`) : dépasser `wait_timeout_ms` n'est donc pas un dépassement de budget, c'est un fait d'API.

### Où le coût se trouve dans le code

La recette est le **contrôle 9** de `docs/04-donnees-et-ingestion.md:192-202` : un échantillon déterministe de `acceptance_sample` passages (200 par défaut), ré-encodé en ONNX fp32 avec `prompt_document`, comparé aux vecteurs du lot par cosinus ; sous `acceptance_min_cosine` (0,999), le lot est refusé en `acceptance_failed` (**INV-12**).

Dans l'ordre du code :

1. `attach` ouvre le chronomètre de `validation` puis lit les contrôles du lot (`crates/crible-store/src/attach.rs:458`) ;
2. le contrôle 9 est appelé à `attach.rs:482-484` (`check_acceptance`, `:748-800`) : il tire l'échantillon (`draw_acceptance_sample`, `:849`), charge le fp32 par le chargeur différé (`document_encoder()`, `:778`, ADR-0053, ADR-0054), puis **ré-encode les passages un par un** dans une boucle (`attach.rs:778-783`) ;
3. chaque appel passe une forme ONNX **`[1, length]`** (`crates/crible-encode/src/model.rs:257-263`) : un passage, une exécution de session ;
4. le chronomètre de `validation` est arrêté après `plan_documents` (`attach.rs:496-510`), donc la recette **et** le chargement du fp32 qu'elle déclenche sont comptés dans `validation` ;
5. s'y ajoutent le contrôle 8 (`count_documents`, `model.rs:473-478`), déjà **parallèle** sur le pool global rayon, et le comptage des jetons du lot (`attach.rs:689`) ;
6. la session ONNX n'est réglée ni en threads ni en niveau d'optimisation (`model.rs:155-157`) : c'est le défaut de la bibliothèque, figé par ADR-0052 (décision 3) et par `docs/07-stack-et-deploiement.md:103`.

Le chargement du fp32 est refait **à chaque attache** par la CLI : `crates/crible/src/commands.rs:146` et `:189` créent un chargeur neuf par commande. Le serveur, lui, gardera la session : les sessions de l'encodeur sont indexées par `(empreinte, précision)` et **gardées jusqu'à l'arrêt de l'instance** (`docs/02-architecture.md:218`, ADR-0051). Le chiffre de 44,2 s contient donc, côté CLI, un coût que le serveur ne paiera qu'une fois.

### Ce que la conception demande

- `docs/02-architecture.md` fait de l'attache une suite d'étapes dont la **première** est « validation, recette et table sémantique faites, aucun état modifié ». La recette est dans le chemin de l'attache, avant toute écriture, et cette place n'est pas un détail d'implémentation : c'est ce qui garantit qu'un lot refusé n'a rien écrit.
- `docs/04:192-202` fixe la recette (échantillon, fp32, cosinus, seuil) ; ADR-0052 fixe le seuil à 0,999 et les défauts d'ONNX Runtime ; ADR-0054 fixe les bornes (`acceptance_sample ≥ 1`, seuil dans `]0, 1]`) et le fait qu'un lot sans passage porte `acceptance: null`.
- `docs/05-api.md:15` : les mutations asynchrones renvoient `202 Accepted` avec un objet `job`.
- `docs/05-api.md:16` : « Avec `?wait=true`, l'appel attend la fin du job (au plus `wait_timeout_ms`, 60 s par défaut) et renvoie le job terminé. » **Ce qui est rendu à l'échéance n'est écrit nulle part**, et `wait_timeout_ms` ne figure pas dans le tableau « Limites par défaut (configurables) » (`docs/05:254-266`).
- `docs/05:43` définit `timeout` → `504` pour « requête trop longue », et `docs/05:264` borne `timeout_ms` à 2 000 ms par défaut, 30 000 au plus : c'est un délai **de requête**, pas de job.
- `docs/07-stack-et-deploiement.md:95-99` déclare `[limits]` avec `writer_queue`, `default_timeout_ms` et `query_cache_entries` : **`wait_timeout_ms` n'y est pas non plus**.

### Le verdict de `@architecte`

Le 2026-10-26, `@architecte` a rendu **NON-COUVERT** sur deux points :

1. **la ventilation de `validation`** : la conception ne dit pas ce qui, de la recette, du comptage des jetons, du chargement du modèle et de `plan_documents`, pèse dans les 44,2 s, et aucun document ne fixe de durée cible pour la recette. Améliorer la recette suppose donc d'abord de savoir ce qu'on améliore ;
2. **l'échéance de `?wait=true`** : `docs/05:16` dit ce que l'appel attend, jamais ce qu'il rend quand le délai expire, ni quel statut porte un job terminé rendu par ce chemin. Un serveur qui choisirait `504` ou une erreur `timeout` inventerait un contrat.

Vérification faite pour cet ADR : un job terminé rendu par `?wait=true` n'a **aucun statut HTTP écrit** dans `docs/05` ; les états de job sont `queued`, `running`, `succeeded` et `failed` (`docs/05:252`), et le seul statut documenté pour une mutation asynchrone est `202`. `wait_timeout_ms` n'existe que dans la phrase de `docs/05:16` : ni le tableau des limites, ni `[limits]` de `docs/07` ne le déclarent.

## Décisions

Aucune décision n'est prise ici : chacune est soumise au propriétaire. Les quatre décisions sont **séparables** : la décision 2 tient sans la décision 1, et la décision 3 se comprend sans les deux autres.

### Décision 1 — La recette ne change pas

**Proposition.** La recette reste celle d'aujourd'hui, sur ses cinq points :

| Paramètre | Valeur | Source |
|---|---|---|
| `acceptance_sample` | 200 | `docs/04`, ADR-0052, ADR-0054 |
| Précision du ré-encodage | fp32 | `docs/04`, ADR-0052 |
| `acceptance_min_cosine` | 0,999 | ADR-0052, ADR-0054, INV-12 |
| Réglages d'ONNX Runtime | défauts de la bibliothèque | ADR-0052, décision 3 |
| Place dans l'attache | chemin de l'attache, étape 1 de `docs/02`, avant toute écriture | `docs/02`, `docs/04` |

Aucun de ces cinq points ne change : le coût de la recette est traité par la mesure et l'encodage (décision 3), pas par un relâchement du contrôle.

**Justification.** La recette est ce qui a détecté un écart de version de transformers (cosinus 0,988, `docs/04:198`) et ce qui tient INV-12. Chacun des paramètres est adossé à une décision acceptée, et aucun n'est un réglage de confort : les réduire ou les déplacer changerait la garantie, pas seulement la durée.

**Ce que cette décision ne tranche pas.** La taille des lots d'encodage n'est pas fixée ici : elle le sera par la mesure de la décision 3, dans le code, et notée dans le rapport.

### Décision 2 — Échéance de `?wait=true` : `202` avec le job en cours, jamais `504`

**Proposition.** Deux règles, et rien d'autre :

- **un job terminé dans le délai** est rendu **terminé** : son objet `job` complet, avec `state` à `succeeded` ou `failed` et son `error` le cas échéant. Le statut HTTP est **`200`**, parce que `docs/05` n'en prescrit aucun pour ce cas : `200` est celui de `GET /v1/jobs/{job_id}`, qui rend le même objet, et `202` reste réservé au cas où le job n'est pas terminé ;
- **à l'échéance de `wait_timeout_ms`**, l'appel rend **`202 Accepted`** avec le job **dans son état courant** (`queued` ou `running`), sans erreur, et le client suit l'avancement par `GET /v1/jobs/{job_id}`.

**Jamais `504`, jamais un code `timeout`** : le job continue, l'attache n'est pas annulée. Le `504` de `docs/05:43` appartient au délai **de requête** (`timeout_ms`, `docs/05:264`), qui borne une requête et non un job ; l'employer ici ferait croire à un échec là où il n'y a qu'une attente écoulée.

**`wait_timeout_ms` entre dans le tableau des limites** de `docs/05:254-266`, avec **60 000 ms par défaut**, la valeur déjà écrite en prose à `docs/05:16`. Constat de rédaction, fait pour cet ADR : `[limits]` de `docs/07:95-99` **ne déclare pas** la clé. Deux documents sont donc à compléter, `docs/05` et `docs/07`, avec la même valeur.

**Mise en œuvre et tests.** Dans la tâche P6 `FILE-D-ECRITURE-ET-JOBS` : ajouter à sa fiche le critère « à l'échéance de `wait_timeout_ms`, `?wait=true` rend `202` avec le job en cours ; un job terminé avant est rendu terminé ; un test pour chaque cas », et citer **ADR-0067**.

**Ce que cette décision ne tranche pas.** Le domaine de la clé n'est pas fixé : ni la borne basse, ni le sens d'une valeur nulle ou négative. Le cas d'un job terminé en `failed` suit la première règle (rendu terminé, `200`, avec son `error`) ; si le propriétaire veut un autre statut pour ce cas, c'est une précision à ajouter, pas une conséquence de cette décision.

### Décision 3 — Tâche hors plan `ACCELERER-RECETTE`, en P4, en deux temps

**Proposition.** Ajouter la tâche `ACCELERER-RECETTE` en **P4**, donc **après J3**, à la fin du fichier `docs/phases/P4-recherche-rapide.md`. Elle se fait en deux temps et **ne change aucun contrat** : d'abord la ventilation de l'étape `validation`, ensuite — et seulement si la ventilation le justifie — l'encodage de la recette par lots. Sans le premier temps, le second serait un remède à une cause supposée.

**Pourquoi en P4 et pas en P3.** J3 n'attend aucune durée (décision 4) et la tâche `BORNER-COUT-DU-COMMIT` d'ADR-0066 occupe déjà la fin de P3. `ACCELERER-RECETTE` ne bloque pas J4 non plus : elle se place en fin de fichier, hors du chemin critique de la recherche rapide.

**Texte complet de la fiche, à insérer tel quel.**

```markdown
### ACCELERER-RECETTE
- **Ajoutée par** : ADR-0067, hors plan. Elle vient de la tâche `MESURER-CYCLE-DE-VIE` et du verdict `NON-COUVERT` de `@architecte` du 2026-10-26.
- **Objectif** : établir où passent les 44,2 s fixes de l'étape `validation` d'une attache, puis ramener la recette sous ce qu'elle coûte aujourd'hui sans changer ce qu'elle garantit. Avec `mdenseon`, `validation` vaut 51,5 s pour 1 000 passages et 65,9 s pour 3 002 sur l'échantillon Legifacile ; la part fixe de 44,2 s est déduite d'un ajustement sur deux points, et MES-08 ne dit rien de sa répartition interne. Rien ne change au contrat : `acceptance_sample` 200, fp32, `acceptance_min_cosine` 0,999, défauts d'ONNX Runtime, recette dans le chemin de l'attache avant toute écriture (ADR-0052, ADR-0054, ADR-0067, INV-12).
- **Dépend de** : `MESURER-CYCLE-DE-VIE` (`FAIT`), qui a mesuré MES-08 et produit les chiffres du 2026-10-25.
- **À lire** : `adr/0067-duree-de-la-recette-a-l-attache-et-echeance-de-wait-true.md` (décisions 1 et 3, conséquences, documents à appliquer) ; `docs/mesures/MES-08-duree-attache-quotidienne.md`, campagne du 2026-10-25 (§ « Résultats », § « Extrapolation à l'échelle de Legifacile », § « Conclusion ») et campagne du 2026-10-26 (§ « Conclusion », qui rappelle que la recette sur l'échantillon n'a pas été rejouée) ; `docs/04-donnees-et-ingestion.md`, contrôle 9 (`:192-202`) ; `docs/02-architecture.md`, étapes de l'attache et sessions de l'encodeur gardées par le serveur (`:218`, ADR-0051) ; `docs/07-stack-et-deploiement.md`, `[limits]` et absence de réglage de session (`:95-103`). Code : `crates/crible-store/src/attach.rs` (`validation` et appel de `check_acceptance`, `:458` et `:482-484`, `check_acceptance`, `:748-800`, boucle de ré-encodage, `:778-783`, `plan_documents`, `:496-510`), `crates/crible-encode/src/model.rs` (forme `[1, length]`, `:257-263`, session sans réglage, `:155-157`, `count_documents`, `:473-478`), `crates/crible/src/commands.rs` (chargeur du fp32 par commande, `:146` et `:189`). ADR liés : ADR-0052 (fp32, seuil 0,999, défauts d'ONNX Runtime), ADR-0053 et ADR-0054 (chargeur différé et bornes de la recette), ADR-0051 (sessions gardées).
- **Livrables** :
  - (a) une section **datée** de MES-08 qui **ventile** l'étape `validation` sur l'échantillon (index `articles`, lots de 1 000 et 3 002 passages, modèle `mdenseon`) : chargement du modèle, 200 ré-encodages, comptage des jetons, passes du lot, `plan_documents`. La ventilation est mesurée, pas déduite : chaque poste est chronométré ou isolé par une exécution qui le retire.
  - (b) **seulement si** le ré-encodage représente plus de la moitié de `validation` : l'encodage de la recette par **lots triés par longueur** (remplissage et masque d'attention), dans `crible-encode`, avec une taille de lot interne fixée par la mesure de cette tâche, **non configurable**, et notée dans le rapport. Sinon, la tâche s'arrête au rapport et propose la suite au propriétaire : elle ne code rien.
  - (c) le rejeu de MES-08, comparé à la campagne du 2026-10-25.
  - Les résultats des contrôles remis à l'orchestrateur pour `docs/suivi.md`, qui reste tenu par lui.
- **Critères d'acceptation** :
  - la ventilation est mesurée et reproductible : les postes de l'étape `validation` sont chiffrés pour les deux tailles de lot, et leur somme retrouve la durée de `validation` de la campagne du 2026-10-25 à la variabilité de la mesure près ;
  - si (b) est fait : un test prouve que le cosinus de **chaque** passage de la recette, encodé par lots, égale celui du même passage encodé seul à **1e-6** près ; et les statistiques de la recette d'un lot de référence (`sample`, `min_cosine`, `p1`, `p50`) sont inchangées à **1e-6** ;
  - si (b) n'est pas fait, la tâche rend le rapport de (a) et une proposition de suite ; elle ne modifie ni `acceptance_sample`, ni le seuil, ni la précision ;
  - rejeu de MES-08 avec l'écart à la campagne du 2026-10-25, en section datée, sans réécrire les chiffres de cette campagne ;
  - `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` et `cargo test` verts ; un build `--release` pour le rejeu de la mesure.
- **Invariants / mesures** : **INV-12** (le lot incompatible reste refusé : la recette garde son échantillon, sa précision et son seuil, et l'égalité à 1e-6 prouve que le ré-encodage par lots ne change pas le verdict) ; **INV-11** (le comptage des jetons ne change pas : la tâche le mesure, elle ne le modifie pas) ; MES-08.
- **Hors périmètre** : `acceptance_sample`, `acceptance_min_cosine`, la précision de référence ou de ré-encodage ; les réglages de threads ou d'optimisation d'ONNX Runtime (ADR-0052, à revoir en conteneur en P8) ; une recette asynchrone ou hors du chemin de l'attache ; le comptage des jetons, sauf proposition au propriétaire si la mesure le montre dominant ; l'encodage en ligne de documents ; toute modification de `docs/04` ou de `docs/05`.
```

### Décision 4 — Aucun de ces points ne bloque J3

**Proposition.** J3 n'attend rien de cet ADR : il demande que MES-07, MES-08 et MES-13 soient **consignées** (`docs/phases/P3-cycle-de-vie.md:4-11`), pas qu'une durée soit atteinte, et MES-08 est consignée depuis le 2026-10-25, avec le rejeu du 2026-10-26. La durée de la recette est un fait documenté de la mesure, pas un critère de jalon ; le dépassement de `wait_timeout_ms` par un lot de 3 002 passages est un fait d'API, que la décision 2 traite.

**Conséquence directe.** Le rapport J3 peut être écrit pendant que cet ADR est encore `Proposé` ; il cite le fait et renvoie à ADR-0067 pour la suite.

## Conséquences

- **Positives** :
  - la recette garde sa garantie intacte : échantillon, précision, seuil et place dans l'attache ne bougent pas, donc INV-12 est tenu par la même preuve qu'aujourd'hui ;
  - `?wait=true` devient un contrat écrit : `202` à l'échéance avec le job en cours, `200` pour un job terminé, jamais `504`. Un client sait quoi faire sans lire le code du serveur ;
  - `wait_timeout_ms` cesse d'exister dans une seule phrase : il entre dans les deux tableaux de limites, avec la même valeur ;
  - la tâche `ACCELERER-RECETTE` commence par la mesure : elle ne peut pas « optimiser » une part qui n'est pas dominante, et elle s'arrête si le ré-encodage ne pèse pas la moitié de `validation` ;
  - aucune tâche de P3 n'est retardée : J3 se valide sans attendre cet ADR.
- **Négatives / coûts** :
  - **la durée de l'attache ne baisse pas** : avec le modèle réel, un lot de 3 002 passages reste au-delà de 60 s sur l'échantillon, et le client devra suivre le job. La décision 2 rend ce comportement lisible, elle ne le raccourcit pas ;
  - **deux documents de plus à tenir cohérents** : la valeur de `wait_timeout_ms` vit dans `docs/05` et dans `docs/07` ; une divergence entre les deux est un défaut de configuration silencieux ;
  - **un chantier de mesure avant le remède** : la ventilation demande un protocole chronométré dans `attach.rs` ou à côté, et un rejeu de MES-08 ; le gain d'encodage par lots n'est pas promis par cet ADR ;
  - **l'encodage par lots touche la forme d'entrée** : passer de `[1, length]` à `[batch, length]` avec remplissage et masque d'attention change le chemin d'inférence de la recette ; le test d'égalité à 1e-6 est le seul garde-fou, et il ne porte que sur la recette, pas sur les requêtes ;
  - **la ventilation est faite côté CLI** : elle compte le chargement du fp32 à chaque commande, que le serveur ne paiera qu'une fois (`docs/02:218`). Le chiffre du serveur sera différent, et devra être revu quand l'attache tournera dans le serveur.
- **À surveiller** :
  - le résultat de la ventilation : si le chargement du modèle ou le comptage des jetons dominent, la tâche propose la suite au propriétaire et ne code pas l'encodage par lots ;
  - l'égalité à 1e-6 des cosinus : un écart plus grand signalerait un changement de chemin d'inférence (remplissage, masque, longueur effective) et non un simple gain de vitesse ;
  - la durée de `validation` sur un vrai index Legifacile après la tâche : si elle reste au-delà de `wait_timeout_ms` pour un lot quotidien, la question du délai revient, avec des chiffres et non une déduction ;
  - la cohérence de `wait_timeout_ms` entre `docs/05` et `docs/07` à chaque changement de défaut ;
  - une attache concurrente de recherches pendant la recette (INV-02) : la recette tourne sur un pool dont MES-08 ne dit rien, et le serveur la fera tourner à côté des requêtes.

### Documents à appliquer après acceptation

| Document | Modification |
|---|---|
| `docs/05-api.md` | ligne `:16` : dire ce que rend `?wait=true` à l'échéance (`202` avec le job dans son état courant, sans erreur) et le statut d'un job terminé rendu par ce chemin (`200`) ; tableau « Limites par défaut (configurables) » (`:254-266`) : ajouter `wait_timeout_ms`, 60 000 ms par défaut |
| `docs/07-stack-et-deploiement.md` | `[limits]` (`:95-99`) : ajouter `wait_timeout_ms = 60000`, la clé n'y est pas aujourd'hui |
| `docs/phases/P6-serveur-et-api.md` | fiche `FILE-D-ECRITURE-ET-JOBS` (`:54-66`) : critère « à l'échéance de `wait_timeout_ms`, `?wait=true` rend `202` avec le job en cours ; un job terminé avant est rendu terminé ; un test pour chaque cas », et référence à ADR-0067 |
| `docs/phases/P4-recherche-rapide.md` | fiche `ACCELERER-RECETTE` ajoutée en fin de fichier, texte de la décision 3 |
| `docs/suivi.md` | ligne « Idées hors périmètre » du 2026-10-25 (`MESURER-CYCLE-DE-VIE`, `:380`) : citer ADR-0067 et son statut ; la ligne existe déjà, elle ne nomme aucun ADR |

Rien n'est appliqué tant que l'ADR est `Proposé`. La proposition d'origine est consignée dans `docs/suivi.md`, section « Idées hors périmètre », ligne du 2026-10-25.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Réduire `acceptance_sample` | Change un défaut fixé par `docs/04` et ADR-0054, et perd de la couverture : la recette est ce qui a détecté un écart de version de transformers. Une valeur plus basse rendrait le seuil de 0,999 plus facile à passer par hasard sur un lot faux |
| Sortir la recette du chemin d'attente (asynchrone, ou après le commit) | Rouvre INV-12 et l'ordre du protocole : `docs/02` place « validation, recette et table sémantique » avant toute écriture, ce qui garantit qu'un lot refusé n'a rien écrit. Un lot écrit puis refusé demanderait une annulation de génération, donc un contrat de plus |
| Régler `intra_op_num_threads` ou le niveau d'optimisation de la session | ADR-0052 (décision 3) a mesuré que les défauts sont les plus rapides, dans les deux précisions ; un réglage n'aurait de sens que pour brider la consommation, et la question est à revoir en conteneur en P8 |
| Relever `wait_timeout_ms` par défaut au-delà de 60 s | Ne raccourcit rien : la durée vient de la recette, et un délai plus long ne fait que tenir un client en attente plus longtemps. La valeur par défaut reste 60 s ; un client qui veut attendre plus longtemps peut le demander |
| Rendre `504` (ou une erreur `timeout`) à l'échéance | Le job continue : rendre une erreur ferait croire à un échec et pousserait un client à réessayer une attache déjà en cours. `timeout` appartient au délai de requête (`docs/05:43`, `:264`) |
| Laisser `docs/05` tel quel et trancher dans le code du serveur | Un contrat qui n'existe que dans une route est perdu à la session suivante : `?wait=true` est une promesse de l'API, et deux implémentations (serveur, client Python) la liraient chacune à sa façon |
| Encoder la recette par lots tout de suite, sans ventilation | Les 44,2 s sont une déduction sur deux points, et MES-08 ne dit rien de leur répartition. Optimiser un poste qui n'est pas dominant ne changerait pas la durée, et le test d'égalité à 1e-6 paierait un changement de chemin d'inférence sans gain prouvé |
| Encoder les 200 passages en une seule exécution, sans tri par longueur | Le remplissage jusqu'à la longueur maximale ferait payer à chaque passage la longueur du plus long ; sur des articles de longueurs très inégales, le gain peut s'annuler. Le tri par longueur et la taille de lot sont fixés par la mesure de la tâche |
| Mettre `ACCELERER-RECETTE` en P3, avant J3 | J3 n'attend aucune durée (décision 4) et P3 est déjà chargée par `BORNER-COUT-DU-COMMIT` (ADR-0066). La mesure de la recette ne conditionne pas la validation du cycle de vie |
| Traiter la durée de la recette en modifiant `docs/02` (étape de recette séparée) | Créerait une étape de plus dans le protocole pour un besoin de mesure interne : la ventilation de la décision 3 se fait dans le code et dans le rapport, sans changer les étapes de l'attache |

## Références

- `docs/mesures/MES-08-duree-attache-quotidienne.md` : campagne du 2026-10-25 (§ « Résultats », § « Extrapolation à l'échelle de Legifacile » `:176-207`, § « Conclusion » `:240-313`, dont la ventilation manquante `:280-281` et l'absence de budget `:284-285`), campagne du 2026-10-26 (rejeu sur `synth-large` après `BORNER-COUT-DU-COMMIT`, § « Conclusion » `:442-485`)
- `docs/04-donnees-et-ingestion.md` : contrôle 9, la recette (`:192-202`)
- `docs/02-architecture.md` : étapes de l'attache (étape 1, validation, recette et table sémantique), sessions de l'encodeur gardées jusqu'à l'arrêt (`:218`)
- `docs/05-api.md` : mutations asynchrones et `?wait=true` (`:15-16`), table des erreurs (`:43`), objet `job` et états (`:236-252`), limites par défaut (`:254-266`)
- `docs/07-stack-et-deploiement.md` : `[limits]` (`:95-99`), absence de réglage de session ONNX (`:103`)
- `docs/08-invariants-et-tests.md` : INV-11 (`:19`), INV-12 (`:20`), MES-08 (`:73`)
- `docs/phases/P3-cycle-de-vie.md` : jalon J3 (`:4-11`), fiche `MESURER-CYCLE-DE-VIE` (`:161-178`), fiche hors plan `BORNER-COUT-DU-COMMIT` (`:180-202`, forme reprise par la décision 3)
- `docs/phases/P6-serveur-et-api.md` : fiche `FILE-D-ECRITURE-ET-JOBS` (`:54-66`)
- `crates/crible-store/src/attach.rs` : `attach` et chronomètre de `validation` (`:458`), appel de `check_acceptance` (`:482-484`), `plan_documents` et arrêt du chronomètre (`:496-510`), comptage des jetons du lot (`:689`), `check_acceptance` (`:748-800`), boucle de ré-encodage (`:778-783`), tirage de l'échantillon (`:849`)
- `crates/crible-encode/src/model.rs` : session sans réglage (`:155-157`), forme `[1, length]` (`:257-263`), `count_documents` parallèle (`:473-478`)
- `crates/crible/src/commands.rs` : chargeur du fp32 par commande (`:146`, `:189`)
- `docs/suivi.md` : « Idées hors périmètre », ligne du 2026-10-25 (`MESURER-CYCLE-DE-VIE`, `:380`)
- ADR-0051 (sessions INT8 et fp32 gardées par le serveur), ADR-0052 (fp32 pour les requêtes, seuil 0,999, défauts d'ONNX Runtime), ADR-0053 et ADR-0054 (encodeur dans le chemin d'attache, chargeur différé et bornes de la recette), ADR-0066 (dérivation de l'instantané après commit ; sa tâche `BORNER-COUT-DU-COMMIT` occupe la fin de P3)
