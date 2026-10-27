# ADR-0073 — Recette sans encodage par lots : la mesure ne donne aucun gain

- **Statut** : Accepté (accepté par anticipation le 2026-10-27, par délégation du propriétaire pour la série RESCORER-FP16, INSERER-POOL-EN-ORDRE, ACCELERER-RECETTE, PLANIFIER-STRATEGIE ; à confirmer par le propriétaire au point de fin)
- **Date** : 2026-10-27 (proposition et acceptation anticipée)
- **Décideurs** : propriétaire
- **Décision** : accepté par anticipation le 2026-10-27, par délégation du propriétaire pour la série RESCORER-FP16, INSERER-POOL-EN-ORDRE, ACCELERER-RECETTE, PLANIFIER-STRATEGIE, à confirmer par le propriétaire au point de fin. Les trois décisions sont retenues telles que proposées : le code du livrable (b) de la fiche `ACCELERER-RECETTE` est retiré, la recette reste encodée un passage à la fois et aucune taille de lot n'existe dans le code ; la tâche livre la ventilation, la mesure des tailles de lot, le rejeu de MES-08, les tests d'INV-11 et d'INV-12, les harnais de mesure et une proposition de suite, sans changer le contrat de la recette ; les fils et réglages d'ONNX Runtime se revoient en conteneur en P8, avec la recette comme cas de mesure, et un changement de fournisseur d'exécution ou un ré-export du graphe demande un nouvel ADR.
- **Proposé par** : agent greffier, sur demande de l'orchestrateur, pour la tâche `ACCELERER-RECETTE` (phase P4, ajoutée par ADR-0067), à partir d'une question de conception de `@dev` et du verdict `NON-COUVERT` de `@architecte`
- **Remplace** : —

## Contexte

### Ce que la fiche demande

La fiche `ACCELERER-RECETTE` (`docs/phases/P4-recherche-rapide.md:110-128`), ajoutée par ADR-0067, se fait en deux temps. Le premier est la ventilation mesurée de l'étape `validation` (`:116`). Le second est conditionnel :

> (b) **seulement si** le ré-encodage représente plus de la moitié de `validation` : l'encodage de la recette par **lots triés par longueur** (remplissage et masque d'attention), dans `crible-encode`, avec une taille de lot interne fixée par la mesure de cette tâche, **non configurable**, et notée dans le rapport. Sinon, la tâche s'arrête au rapport et propose la suite au propriétaire : elle ne code rien.
> — `docs/phases/P4-recherche-rapide.md:117`

Le critère d'acceptation du même livrable demande, s'il est fait, l'égalité à `1e-6` du cosinus de **chaque** passage et des statistiques de recette d'un lot de référence (`:123`). Si (b) n'est pas fait, la fiche demande « le rapport de (a) et une proposition de suite », sans toucher à `acceptance_sample`, au seuil ni à la précision (`:124`). Les réglages de threads et d'optimisation d'ONNX Runtime sont **hors périmètre**, renvoyés « en conteneur en P8 » (`:128`, ADR-0052).

La fiche ne prévoit donc que deux issues : la condition n'est pas remplie et la tâche s'arrête, ou elle l'est et l'encodage par lots est codé. Elle n'a pas de branche pour le cas rencontré ici : **la condition est remplie, le remède est écrit, et il ne donne aucun gain.**

### Le livrable (a) : la condition de (b) est remplie

La campagne de nuit du 2026-10-26 de MES-08 (`docs/mesures/MES-08-duree-attache-quotidienne.md:540-882`) ventile `validation` par postes mesurés, et non plus déduits d'un ajustement sur deux points. Les 200 ré-encodages pèsent **88,0 %** de `validation` pour le lot de 1 000 passages et **89,0 %** pour celui de 3 002 (`:757`, `:799-803`). Le seuil de la fiche — la moitié — est dépassé d'un facteur 1,8. La condition de (b) est donc remplie.

La même campagne corrige la « part fixe de 44,2 s » du 2026-10-25 (`:729-748`) : la recette est fixe en nombre de passages (200) mais pas en coût, parce qu'il suit la **longueur des passages tirés**. Le lot de 3 002 porte 13 161 ms de plus uniquement parce que ses 200 textes tirés sont plus longs (54 991 → 65 193 jetons, +18,6 %, `:741-742`). La taille du lot ne coûte, elle, que 0,06 ms par passage ajouté (`:745-746`).

### La mesure des tailles de lot : le temps croît avec la taille

La campagne du 2026-10-27 (`MES-08:884-1127`) mesure le **temps** d'encodage par taille de lot, ce que la veille n'avait pas fait : elle n'avait chiffré que la surcharge de remplissage. Temps total des 200 passages, médiane de trois tours entrelacés (`:961-968`) :

| Lot | B = 1 | B = 4 | B = 8 | B = 16 |
|---|---|---|---|---|
| 1 000 passages | 56 314,7 ms | 59 681,1 ms | 63 633,4 ms | 68 440,9 ms |
| 3 002 passages | 70 987,2 ms | 73 561,7 ms | 77 908,0 ms | 86 140,6 ms |

Trois constats, tous contraires à l'hypothèse du livrable (b) :

1. **Le temps croît avec la taille de lot** : +6,0 %, +13,0 % et +21,5 % pour le lot de 1 000 ; +3,6 %, +9,8 % et +21,4 % pour celui de 3 002 (`:978-982`). La surcharge de temps dépasse toujours la surcharge de jetons.
2. **Le coût par jeton monte aussi** : 1 024 → 1 245 µs (lot de 1 000) et 1 089 → 1 321 µs (lot de 3 002), `:983-986`. Un lot ne partage donc pas le travail d'un appel : il paie ses jetons de remplissage et un peu plus cher par jeton réel.
3. **Les lignes d'un lot ne s'exécutent pas en parallèle** avec les réglages par défaut : huit copies du plus long passage coûtent 8,05 à 8,08 fois un appel seul, soit strictement linéaire (`:994-1013`). Le gain d'un lot ne peut venir que d'un parallélisme entre lignes, et il n'y en a pas.

La conclusion de la campagne est nette : **la valeur mesurée de la taille de lot est 1** (`:1027-1035`), et « le code du livrable (b) n'apporte rien » (`:1037-1039`). Les tests d'égalité de la tâche montrent par ailleurs des cosinus identiques entre lot et appel seul (écart maximal relevé 4,19e-9), et les statistiques de recette épinglées sont inchangées : le lot ne change pas la fidélité, seulement le coût.

### Le rejeu de MES-08 (livrable c)

Le rejeu confirme la structure du 2026-10-25 : `validation` domine la durée totale à plus de 95 %, `snapshot`, `segment` et `transaction` restent sous les 5 % (`MES-08:1051-1063`). L'écart de valeur absolue est de **+13,0 %** pour 1 000 passages et **+15,8 %** pour 3 002 (`:1053-1054`), attribué à la charge de la machine (charge moyenne 5,2 à 5,9 contre 2,3 à 2,7, `:1064-1070`). Aucun code de `validation` n'a changé entre les deux campagnes.

### Le verdict de `@architecte`

`@architecte` a rendu **NON-COUVERT** : la fiche n'a pas de branche « condition remplie, remède sans gain ». La conception et les ADR acceptés ne disent pas quoi faire quand le second temps est déclenché et que la mesure le déclare inutile.

Le verdict établit aussi que retirer (b) **s'écarte de la lettre de la fiche mais pas d'ADR-0067**. Celui-ci écrit que le second temps vient « seulement si la ventilation le justifie » (`docs/adr/0067-duree-de-la-recette-a-l-attache-et-echeance-de-wait-true.md:105`), que la tâche « s'arrête si le ré-encodage ne pèse pas la moitié de `validation` » (`:144`) et que « le gain d'encodage par lots n'est pas promis par cet ADR » (`:149`). Il ajoute même, à la ligne 153 : « si le chargement du modèle ou le comptage des jetons dominent, la tâche propose la suite au propriétaire et ne code pas l'encodage par lots ». L'intention — coder un remède seulement s'il est justifié par la mesure — est donc respectée ; c'est la lettre du livrable (b) qui est plus étroite que la décision qu'elle met en œuvre.

Enfin, garder dans le code un mécanisme de lot à taille 1 serait une **abstraction sans seconde implémentation** et une constante à une seule valeur utile : cela contredit la règle 12 d'`AGENTS.md` (`AGENTS.md:50`, « pas d'abstraction pour une seule implémentation, pas de configuration pour une valeur fixe »). Le code du livrable (b) est aujourd'hui présent dans l'arbre : `crates/crible-encode/src/batch.rs`, `ACCEPTANCE_BATCH_SIZE = 1` (`batch.rs:47`), `Model::encode_batch` (`crates/crible-encode/src/model.rs:328`), l'export `pub use batch::ACCEPTANCE_BATCH_SIZE` (`crates/crible-encode/src/lib.rs:29`), l'appel dans `check_acceptance` (`crates/crible-store/src/attach.rs:784`) et ses tests (`attach.rs:1607-1690`).

## Décision

**Le livrable (b) est retiré, et la recette reste encodée un passage à la fois.** Il n'y a pas d'encodage par lots de la recette, et aucune taille de lot n'existe dans le code.

Précisions :

1. **Retrait du code du livrable (b).** `check_acceptance` ré-encode ses 200 passages par `Model::encode`, un appel par passage, comme avant la tâche. Le module `batch.rs`, la constante `ACCEPTANCE_BATCH_SIZE`, la méthode `Model::encode_batch`, son export dans `lib.rs` et les tests qui l'exerçaient sont supprimés. Aucune constante, aucun réglage, aucun chemin de code ne subsiste pour une taille de lot. Le retrait est fait par `@dev` ; cet ADR ne modifie pas le code.
2. **Ce que la tâche livre.** `ACCELERER-RECETTE` livre : la ventilation (a) ; la mesure des tailles de lot, consignée dans MES-08 ; le rejeu de MES-08 (c) ; les tests d'**INV-11** (comptage des jetons épinglé) et d'**INV-12** (statistiques de la recette du lot de référence épinglées à `1e-6`) écrits par `@testeur` ; les harnais de mesure (`crates/crible-store/examples/acceptance_split.rs`, `tools/bench/accelere-recette/`) ; et la proposition de suite de la mesure (`MES-08:1072-1106`).
3. **Rien ne change au contrat de la recette** : `acceptance_sample` 200, ré-encodage en fp32, `acceptance_min_cosine` 0,999, défauts d'ONNX Runtime, recette dans le chemin de l'attache avant toute écriture (ADR-0052, ADR-0054, ADR-0067, INV-12). La décision ne touche ni `docs/04`, ni `docs/05`, ni aucun format, ni aucun invariant.
4. **Suite.** Les fils et réglages d'ONNX Runtime se revoient **en conteneur, en P8** (ADR-0052, décision 3, `docs/adr/0052-...:97`), avec la recette comme cas de mesure : coût par jeton, ré-encodages par lot de taille 1 et par lot de taille supérieure. Le protocole de la campagne est réutilisable tel quel (`MES-08:1078-1087`). Un **fournisseur d'exécution** et un **ré-export du graphe** ne se font que par un **nouvel ADR** : ils changent l'artefact du modèle et ses hash (ADR-0050, note du 2026-10-24 en `docs/adr/0050-...:85` ; ADR-0057), et un ré-export imposerait de ré-encoder tous les vecteurs. Aucun des deux n'est planifié. La durée de la recette reste celle qui est mesurée : environ 1 µs par jeton des passages tirés, soit 52 à 71 s par attache pour 200 passages.

Ce qui n'est **pas** décidé ici :

- la valeur d'un éventuel réglage de fils en conteneur, qui n'est pas mesurée ;
- un fournisseur d'exécution, un ré-export du graphe, ou une réduction de `acceptance_sample` : chacun demande son propre ADR ;
- la généralisation du groupement trié par longueur à un autre usage. La mesure note qu'il « devient payant le jour où les lignes d'un lot s'exécutent réellement en parallèle » (`MES-08:1040-1042`) ; ce jour n'est pas venu, et rien n'est gardé dans le code pour l'attendre.

## Conséquences

- **Positives** :
  - le code ne contient plus de mécanisme de lot à une seule taille utile : une constante, un module et ses tests en moins, et un seul chemin d'encodage de la recette, celui d'avant la tâche ;
  - la question « faut-il encoder la recette par lots ? » est tranchée par une mesure, pas par une hypothèse : trois tours entrelacés, deux lots, encadrement par le code de production (`MES-08:942-974`) ;
  - la tâche se termine sur un livrable complet — ventilation, mesure, rejeu, tests, harnais, proposition de suite — au lieu de laisser un code inutile derrière elle ;
  - le levier restant est nommé, chiffré et daté : les fils d'ONNX Runtime en conteneur, en P8, avec la recette comme cas de mesure.
- **Négatives / coûts** :
  - **la lettre du livrable (b) n'est pas suivie** : la fiche dit « encodage par lots » quand la condition est remplie, et il n'y en a pas. C'est assumé, et c'est ce que le verdict `NON-COUVERT` demande de trancher ;
  - **la durée de la recette ne baisse pas** : 52 à 71 s par attache pour 200 passages, et un lot de 3 002 passages reste au-delà de `wait_timeout_ms` (`docs/adr/0067-...:147`, `MES-08:1053-1054`) ;
  - **le groupement trié par longueur est perdu** alors qu'il était écrit et testé. Il faudrait le réécrire si un parallélisme entre lignes apparaissait un jour ;
  - **la mesure des tailles de lot ne vit que dans MES-08** : le code qui l'a permise n'est plus dans l'arbre. Les relevés bruts et la copie du code mesuré restent dans `data/accelere-recette/` (gitignoré, `MES-08:1108-1127`), et le harnais ne dépend d'aucune crate de Crible.
- **À surveiller** :
  - **la mesure en conteneur en P8** : si les fils intra-op d'ONNX Runtime laissent un parallélisme entre lignes apparaître, un lot pourrait devenir payant et la question reviendrait, avec des chiffres et un nouvel ADR ;
  - **le coût par jeton** : il est aujourd'hui d'environ 1 µs par jeton ré-encodé. Une dérive nette signalerait un changement d'environnement ou d'artefact, pas la recette ;
  - **un ré-export du graphe** : il change l'empreinte du modèle, donc tous les vecteurs, et impose un ré-encodage complet du corpus (extrapolation non mesurée : ≈ 28 jours de CPU à 1 024 µs/jeton, `MES-08:1095-1100`). C'est le seul levier qui pourrait réduire la recette d'un facteur deux, et il coûte ce ré-encodage ;
  - **la tentation de garder le code « au cas où »** : un mécanisme de lot à taille 1 se remettrait en service sans mesure, ce qui est exactement ce que cet ADR refuse.

### Documents appliqués

| Document | Modification |
|---|---|
| `docs/adr/README.md` | ligne d'index de l'ADR-0073 |
| `docs/phases/P4-recherche-rapide.md` | fiche `ACCELERER-RECETTE` : note sous les livrables, (b) codé puis retiré sur mesure, renvoi à ADR-0073 |
| `docs/phases/P8-docker-et-deploiement.md` | fiche `MESURER-EN-CONTENEUR-MAC` : cas de mesure de la recette (fils d'ONNX Runtime et lots), renvoi à ADR-0073 et ADR-0052 |
| `docs/adr/0067-duree-de-la-recette-a-l-attache-et-echeance-de-wait-true.md` | note en tête : renvoi à ADR-0073 pour l'issue du second temps, décision inchangée |

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Garder le mécanisme de lot à taille 1, non configurable | Une abstraction pour une seule implémentation et une constante à une seule valeur utile : contraire à la règle 12 d'`AGENTS.md` (`:50`). Le mécanisme ne servirait qu'au jour où la taille changerait, et ce jour demande une mesure et un ADR |
| Garder le code du livrable (b) sans l'appeler | Code mort dans un chemin critique de l'attache : il serait relu, maintenu et finirait par être utilisé sans mesure. La copie du code mesuré reste dans `data/accelere-recette/livrable-b/`, ce qui suffit à la reproductibilité |
| Choisir B = 8, la plus grande taille dont le remplissage reste sous 10 % (`MES-08:805-811`) | Le raisonnement ne portait que sur la surcharge de jetons, jamais sur le temps. Mesuré, B = 8 coûte +13,0 % et +9,8 % de temps (`MES-08:961-968`) |
| Garder le groupement trié par longueur seul, sans exécution par lots | Il ne sert à rien tant que les lignes d'un lot ne s'exécutent pas en parallèle, et il faudrait maintenir et tester un tri dont aucun appel ne dépend. `MES-08:1040-1042` le note comme utile plus tard, pas maintenant |
| Modifier la fiche pour supprimer la condition de (b) | La fiche est la trace de ce qui a été demandé. Une note datée sous les livrables dit ce qui a été fait et pourquoi, sans réécrire la demande ni laisser croire que la condition n'a jamais été remplie |
| Régler les fils d'ONNX Runtime tout de suite, dans cette tâche | Hors périmètre de la fiche (`docs/phases/P4-recherche-rapide.md:128`) et d'ADR-0052, décision 3 : les défauts sont les plus rapides sur le Mac de développement, et la question se pose en conteneur, en P8, où les cœurs et le voisinage changent (`docs/adr/0052-...:97`, `:145-147`) |
| Changer de fournisseur d'exécution (CoreML, par exemple) pour accélérer la recette | Change une dépendance et le chemin d'inférence pour un gain **non mesuré** ; c'est un ADR, et il se pose après la mesure en conteneur, pas à la place |
| Ré-exporter le graphe (fp16, longueur maximale réduite) | Change l'artefact et ses hash (ADR-0050, note `:85` ; ADR-0057), donc l'empreinte du modèle et **tous** les vecteurs : un ré-encodage complet du corpus, environ 28 jours de CPU en extrapolation (`MES-08:1092-1100`). Un nouvel ADR est requis, et il n'est pas planifié |
| Réduire `acceptance_sample` sous 200 | Change un contrat fixé par `docs/04` et ADR-0067, décision 1 : c'est perdre de la couverture sur le contrôle qui a détecté un écart de version de transformers. Un ADR serait nécessaire |
| Ne rien décider et laisser la fiche en l'état | La condition de (b) est remplie : sans décision, le code du lot resterait dans l'arbre, ou serait retiré sans trace. Ni l'un ni l'autre n'est acceptable pour une tâche qui engage INV-12 |

## Références

- `docs/phases/P4-recherche-rapide.md` : fiche `ACCELERER-RECETTE` (`:110-128`), livrable (a) (`:116`), livrable (b) (`:117`), livrable (c) (`:118`), note d'issue du livrable (b) (`:120`), critères (`:121-126`), hors périmètre (`:128`)
- `docs/mesures/MES-08-duree-attache-quotidienne.md` : campagne du 2026-10-26 (nuit), ventilation de `validation` (`:540-882`), postes mesurés (`:653-673`), somme des postes contre `validation` (`:693-728`), correction de la part fixe (`:729-748`), comparaison au 2026-10-25 (`:750-768`), règle de la moitié (`:797-823`) ; campagne du 2026-10-27 (`:884-1127`), protocole des tailles de lot (`:912-952`), temps par taille (`:953-992`), linéarité des lignes (`:994-1013`), correction de la surcharge (`:1015-1025`), conclusion « la taille de lot mesurée est 1 » (`:1027-1042`), rejeu de MES-08 (`:1044-1070`), proposition de suite (`:1072-1106`)
- `docs/adr/0067-duree-de-la-recette-a-l-attache-et-echeance-de-wait-true.md` : décision 1 (la recette ne change pas), décision 3 (tâche `ACCELERER-RECETTE` en deux temps, `:103-136`), conséquences (`:138-169`), dont « le gain d'encodage par lots n'est pas promis » (`:149`) et « la tâche propose la suite au propriétaire » (`:153`) ; note de tête du 2026-10-27 renvoyant à cet ADR
- `docs/adr/0052-requetes-encodees-en-fp32-et-seuil-de-recette-a-0-999.md` : décision 3, réglages de session ONNX aux défauts, à revoir en conteneur en P8 (`:93-97`), alternatives écartées sur les fils (`:141-147`)
- `docs/adr/0050-reference-mdenseon-sous-transformers-5-14-1-et-reexport-onnx.md` : note du 2026-10-24 après acceptation, toute modification de l'artefact ou de ses hash relève d'un nouvel ADR (`:85`)
- `docs/adr/0057-onnx-runtime-et-vrai-modele-mdenseon-en-integration-continue.md` : artefact déjà exporté, épinglé par sha et vérifié par hash
- `docs/adr/0054-chargement-differe-du-modele-a-l-attache-et-bornes-de-la-recette.md` : bornes de la recette, chargeur différé
- `docs/08-invariants-et-tests.md` : INV-11 (`:19`), INV-12 (`:20`)
- `docs/04-donnees-et-ingestion.md` : contrôle 9, la recette (`:192-202`)
- `AGENTS.md` : règle 12, simplicité (`:50`)
- Code concerné par le retrait : `crates/crible-encode/src/batch.rs` (`ACCEPTANCE_BATCH_SIZE`, `:47`), `crates/crible-encode/src/model.rs` (`Model::encode_batch`, `:328`), `crates/crible-encode/src/lib.rs` (`:29`), `crates/crible-store/src/attach.rs` (appel dans `check_acceptance`, `:784` ; tests, `:1607-1690`)
- Harnais de mesure conservés : `crates/crible-store/examples/acceptance_split.rs`, `tools/bench/accelere-recette/run.sh`, `tools/bench/accelere-recette/merge.py`
