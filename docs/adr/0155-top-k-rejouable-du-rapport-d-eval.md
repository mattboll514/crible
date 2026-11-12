# ADR-0155 — Top-k rejouable du rapport d'éval

- **Statut** : Proposé
- **Date** : 2026-11-12
- **Décideurs** : propriétaire
- **Proposé par** : agent `@greffier`, tâche `T16-TOP-K-REJOUABLE` (v1.1), sur le verdict de `@architecte` du 2026-11-12 et le ticket 16 du dépôt Legifacile (`../legifacile/tickets-crible/16-reference-top-k-rejouable.md`)
- **Remplace** : — (complète ADR-0125, décisions 11, 12 et 16, et ADR-0131, décision 7, sans modifier le corps d'un ADR accepté, comme ADR-0131 l'a fait pour ADR-0125)

## Contexte

### Le besoin, et d'où il vient

Le ticket 16 du dépôt Legifacile constate que Legifacile mesure la qualité de ce qu'il sert en comparant le top 10 de chaque requête à un **rendu figé** de la sandbox pgvector. Ce rendu date du 2026-11-06, et le moteur qui l'a produit n'est plus servi : MES-05 le dit, « pgvector, moteur arrêté, a été figé ». Chaque mise à jour DILA éloigne donc ce rendu de l'index courant, et la comparaison ne peut plus rien attribuer.

MES-05 du 2026-11-11 donne trois recouvrements du top 10 sur 97 lignes : **0,520619** pour le MCP `recherche_semantique` (hybride servi), **0,520619** pour Crible hybride à filtres égaux, et **0,935052** pour Crible dense. Le rapport attribue l'écart dense de **0,065** « à la fois [au] changement de moteur d'encodage et [à] la dérive des données entre le 6 et le 11 novembre ». C'est exact, et c'est le problème : aucune des deux campagnes ne peut être rejouée sur l'état de l'autre.

**Aucune référence interne à Crible n'existe.** Les index ont reçu les archives DILA des 9 et 10 novembre. Un document attendu qui sort du top 10 parce qu'il a été remplacé n'est pas une régression de qualité ; un document qui en sort parce que la requête est encodée autrement en est une. Le chiffre de recouvrement ne les distingue pas.

### Ce que le rapport d'éval porte aujourd'hui, et ce qui lui manque

`crible eval` produit un JSON fermé, aujourd'hui en `format_version: 2` (`crates/crible/src/eval.rs:114`), et un Markdown. Son en-tête porte `command`, `source`, `date`, `game`, `index`, `generation`, `pool_size`, `version`, `k`, `repeat`, `queries` et `queries_with_expected` (`crates/crible/src/eval.rs:559-596`). Sa ligne par requête porte `id`, puis selon le mode `fidelity_recall_at_10`, `fidelity_overlap_at_10`, `fidelity_denominator` et `quality_recall_at_k`, `quality_mrr_at_k`, `quality_found` (`crates/crible/src/eval.rs:1106-1121`). Aucune de ces valeurs n'est un classement : `quality_found` est une liste d'**indices** d'attendus trouvés, pas un ordre de documents.

Trois conséquences se vérifient dans le code et dans la conception :

1. **`--baseline` ne compare que des moyennes.** `REGRESSION_THRESHOLD` vaut `0,005` en valeur absolue (`crates/crible/src/eval.rs:111`), et `compare` (`.rs:1402`) ne lit que des moyennes ; `lost_queries` (`.rs:1486`) ne nomme que les requêtes qui ont perdu un attendu que la référence trouvait. Deux requêtes qui échangent leurs rangs, ou un top 10 entièrement réordonné à moyenne constante, passent donc inaperçus.
2. **La fidélité ne voit pas un changement d'encodeur.** Elle compare le run normal au run `exact: true` de la **même** campagne, donc avec le même vecteur de requête produit par le même encodeur. Elle mesure la perte du scan approximatif, pas un ré-encodage ni une dérive des données : les deux déplacent les deux classements ensemble.
3. **L'en-tête ne porte ni l'empreinte du modèle, ni `query_precision`, ni une empreinte du jeu de requêtes.** L'empreinte du modèle existe pourtant déjà (`ModelConfig::fingerprint`, `crates/crible-core/src/model.rs:370`), et `query_precision` est réglée par ADR-0052. Sans ces trois valeurs, même un top-k enregistré ne dirait pas d'où vient un écart.

### Le verdict de `@architecte` et la classe de la tâche

Le verdict de `@architecte` du 2026-11-12 est **NON-COUVERT** : aucune décision acceptée ne fixe un top-k dans le rapport, ni son empreinte de modèle ou de jeu, ni une comparaison de classements. La classe d'ADR-0143, décision 7, est confirmée : **ajout compatible**. La tâche ajoute des membres à un rapport fermé et change son `format_version`, sans toucher à un format persistant du moteur, à une route, à un membre d'erreur ni à une valeur par défaut de configuration.

Cet ADR **complète** ADR-0125 (décisions 11, 12 et 16) et ADR-0131 (décision 7) **sans modifier leur corps**, de la même façon qu'ADR-0131 a complété ADR-0125. Les décisions ci-dessous disent ce qui s'ajoute ; elles ne réécrivent rien de ce qui existe.

### Les faits vérifiés aux sources, avant rédaction

- Le rapport est en `format_version: 2` et `--baseline` lit les versions **1 et 2** (`BASELINE_VERSIONS`, `crates/crible/src/eval.rs:118-121`), en refusant toute autre par un message qui nomme la version lue (`.rs:1383-1392`).
- Le run **groupé par document** existe déjà et rend ses `doc_id` externes, meilleur d'abord : `Run.docs` est décrit comme « External `doc_id` of the results, best first » (`crates/crible/src/eval.rs:200-204`), et le run groupé qui porte la qualité est construit à `.rs:695-705`. La liste ordonnée demandée est donc déjà collectée ; elle n'est simplement pas écrite dans le rapport.
- `ModelConfig::fingerprint` rend le blake3 du JSON canonique d'ADR-0028, en **hexadécimal minuscule nu** (`crates/crible-core/src/model.rs:337-375`). Pour un modèle `kind: "external"`, la validation n'exige ni `hf_id`, ni `revision`, ni `files` (`crates/crible-core/src/model.rs:305-307`), et l'empreinte ne hasherait donc que `dims` et `normalize` : elle ne dirait rien de l'encodeur.
- `blake3` est déjà une dépendance Rust (`docs/07-stack-et-deploiement.md:44`) et Python (ADR-0034) : aucune dépendance nouvelle.
- Le jeu de référence de Legifacile est déjà versionné sous `docs/legifacile/reference/` (ADR-0138), et `docs/06-observabilite.md:150-179` décrit la commande, ses mesures, sa régression et ses sorties.

## Décision

### Décision 1 — Le rapport passe à `format_version: 3`, et `--baseline` lit 1, 2 et 3

**Décision.** Le rapport d'éval passe à **`format_version: 3`**. Le lecteur de `--baseline` accepte les versions **1, 2 et 3** et refuse toute autre par un message qui **nomme la version lue**.

Précisions :

- ADR-0143, décision 2, pose qu'« aucun `format_version` ne change dans la ligne 1.x, sauf par un ADR de migration ». Cette règle vise les formats **persistants que le moteur rouvre**, où une version inconnue est refusée à l'ouverture (INV-15 ; ADR-0143, décision 2). Un rapport d'éval n'est **jamais rouvert par le moteur** : il est écrit une fois, puis relu par un humain ou par `crible eval --baseline`, qui applique une règle de lecture explicite et non une égalité stricte ;
- ADR-0125, décision 13, interdit d'écraser un rapport : un rapport daté n'est jamais réécrit. Une version nouvelle ne crée donc aucune migration, aucun retour arrière et aucun fichier à convertir ;
- le précédent est celui de **v1 → v2**, décidé par ADR-0131, décision 7, pour ajouter `source` et `latency_basis` : même classe de changement, même mécanisme de lecture, et `--baseline` avait alors accepté deux versions au lieu d'une ;
- le refus nomme la version lue, comme aujourd'hui (`crates/crible/src/eval.rs:1383-1392`), et la liste des versions acceptées devient `[1, 2, 3]` ;
- une référence de version 1 ou 2 reste lisible pour les moyennes, la fidélité, la qualité et les latences ; seule la comparaison de top-k manque, et la décision 5 dit comment le rapport le signale ;
- la version du binaire, `command` et les autres membres de l'en-tête ne changent pas.

### Décision 2 — Chaque ligne porte `doc_ids`, la liste ordonnée du run groupé

**Décision.** Chaque ligne par requête, dans **chaque mode**, porte `"doc_ids": [...]` : la liste ordonnée, meilleur d'abord, des `doc_id` **externes** rendus par le **run groupé** à `k` — celui dont le rapport publie déjà la qualité. Le membre est **toujours présent** : `[]` quand le run a tourné sans rendre de document.

Précisions :

- `[]` est une **valeur mesurée**, pas une mesure absente : une requête dont le pool filtré est vide a bien été mesurée, et son top-k est vide. La règle « un membre sans valeur est omis » d'ADR-0125, décision 11, ne s'applique donc pas à ce membre ;
- la source est `Run.docs` (`crates/crible/src/eval.rs:200-204`), déjà collecté, écrit tel quel, sans retri ni recomplètement ;
- `k` est celui de la qualité, celui que le rapport publie déjà (ADR-0125, décision 4) : la liste a au plus `k` entrées, et moins quand le pool ou le regroupement n'en fournit pas `k` (ADR-0075, décision 1) ;
- la liste est conservée **même quand la requête n'a pas d'`expected`**. La qualité n'a pas alors de moyenne, mais le top-k reste une mesure de classement rejouable, et l'exclure rendrait la comparaison dépendante du jugement ;
- **aucune clé `doc_id#ord`** n'est écrite : le jugement est au document (ADR-0125, décision 4 ; ADR-0092, décision 2), et la fidélité au passage garde ses propres métriques, déjà calculées, qui portent les clés ;
- l'ordre écrit est exactement l'ordre rendu, sans recalcul, pour que deux rapports se comparent sur des listes produites par le même chemin.

### Décision 3 — L'en-tête du rapport porte l'empreinte du modèle, la précision des requêtes et l'empreinte du jeu (mode local)

**Décision.** En mode **local**, l'en-tête du rapport gagne trois membres : `model_fingerprint`, `query_precision` et `queries_fingerprint`.

Précisions :

- **`model_fingerprint`** est `ModelConfig::fingerprint` (`crates/crible-core/src/model.rs:370`), en **hexadécimal minuscule nu** (ADR-0028), sans préfixe ni enveloppe. Il est **omis** pour un modèle `kind: "external"`, dont la configuration ne porte ni fichiers ni révision : une empreinte réduite à `dims` et `normalize` ferait croire à une attribution qu'elle ne permet pas. Le Markdown dit alors que l'écart n'est **pas attribuable à l'encodeur** ;
- **`query_precision`** vaut `"fp32"` (ADR-0052) et n'est écrit **que si la campagne a encodé un texte dense** : une campagne entièrement en `vector`, ou une campagne `lexical`, n'a encodé aucune requête, et écrire `"fp32"` y serait faux. Aucune option nouvelle n'est ajoutée : la valeur vient de la configuration de l'encodeur, et la règle 12 interdit une configuration pour une valeur fixe ;
- **`queries_fingerprint`** est le blake3, en hexadécimal minuscule nu, de la **forme canonique du jeu parsé** : une entrée par ligne, **dans l'ordre du fichier**, clés triées, JSON compact, UTF-8 sans échappement des caractères non ASCII — la forme d'ADR-0028 —, avec les membres `id`, `query` **ou** `vector`, `filter`, `expected`. Les membres facultatifs absents ne sont pas écrits ;
- le hash porte sur le jeu **parsé**, pas sur les octets bruts : un simple reformatage du fichier, un espace ou un ordre de clés différent changerait le hash des octets sans changer le jeu, et ferait croire à une campagne différente ;
- `blake3` est déjà dans `docs/07` (ligne « Hash ») : aucune dépendance nouvelle, ni Rust ni Python ;
- les trois membres sont **fermés** : ils ne prennent aucune autre valeur, et aucun n'est `null`.

### Décision 4 — Le mode API gagne `doc_ids`, garde `queries_fingerprint`, et n'écrit ni empreinte de modèle ni `query_precision`

**Décision.** Le membre `doc_ids` de ligne s'applique **aussi au mode API** : les deux modes partagent le même calcul, seule la source du run change. En mode API, l'en-tête **ne porte ni `model_fingerprint` ni `query_precision`** ; `queries_fingerprint`, qui ne dépend que du jeu, est écrit **dans les deux modes**.

Précisions :

- `exact_threshold` et N sont des **préconditions de l'instance** en mode API (ADR-0131, décision 9) ; l'encodeur et sa précision en sont aussi. Écrire une empreinte de modèle dans un rapport API laisserait croire que la campagne l'a lue ou imposée, alors qu'elle ne lit que la réponse du serveur ;
- `queries_fingerprint` ne décrit que le fichier d'entrée : il ne dépend ni du mode, ni de l'instance, et il est donc écrit partout ;
- `doc_ids` est rempli par le run **groupé** de la qualité, comme en local (`group_by: "document"`, `passages_per_document: 1`, ADR-0131, décision 8) ;
- **limite à écrire explicitement.** Le mode API de `crible eval` vise la route **simple** `POST /v1/indexes/{index}/search` (ADR-0131, décision 1). La route **multi-index** `POST /v1/search`, celle qui sert Legifacile, a été écartée par cette même décision 1, parce que sa réponse ne porte pas de `generation` scalaire mais `targets: [{ "index", "generation", "pool_size" }]` (ADR-0110, décision 1). **T16 ne rend donc pas le top-k servi par la route multi-index rejouable.** L'atteindre demanderait un ADR qui remplace ADR-0131, décision 1, hors du périmètre de cette tâche ;
- cette limite est écrite dans l'ADR, dans `docs/06` à l'application et dans le Markdown du rapport, pour qu'un lecteur de Legifacile ne croie pas couvert ce qui ne l'est pas.

### Décision 5 — `--baseline` compare les top-k, sans changer le code de sortie

**Décision.** `--baseline` calcule, **par mode** et **par requête présente dans les deux rapports**, `overlap_at_k = |doc_ids courants ∩ doc_ids de référence| / |doc_ids de référence|`, en rend la moyenne avec son nombre de requêtes, et liste les `id` dont la **liste ordonnée** diffère. La comparaison est **informative** : le **code de sortie ne change pas**.

Précisions :

- le dénominateur est celui de la **référence**, comme le complément C1 d'ADR-0125 le fait pour le recouvrement de fidélité : une référence plus courte qu'un top-k courant ne doit pas diluer le chiffre ;
- une requête dont la **référence est vide** est **exclue de la moyenne** et **comptée à part**, avec son `id`, sur le modèle du même complément C1 ;
- une requête présente dans un seul des deux rapports n'est pas comparée ; elle est comptée à part, comme un mode absent de la référence l'est déjà (ADR-0131, décision 7) ;
- la liste des changements compare les **listes ordonnées**, pas les ensembles : un échange de rang **compte**. Deux listes de même contenu et d'ordre différent sont donc signalées ;
- **le code de sortie reste 0 ou 1 selon les seules moyennes de qualité et de fidélité** (ADR-0125, décision 12). La dérive des données déplace légitimement le top-k sans régression du moteur ; faire échouer `--baseline` sur ce chiffre confondrait les deux ;
- une référence en **version 1 ou 2** ne porte pas de `doc_ids` : la comparaison de top-k **n'est pas faite**, et le rapport le dit, sur le modèle de `latency_not_compared` d'ADR-0131, décision 7 ;
- les moyennes et listes déjà produites par `compare` (`.rs:1402`) et `lost_queries` (`.rs:1486`) sont conservées telles quelles : cet ajout ne retire ni ne réordonne rien.

### Décision 6 — Une attribution est écrite, et un écart de génération ou d'empreinte ne fait jamais refuser la référence

**Décision.** Quand les générations, `model_fingerprint` ou `queries_fingerprint` diffèrent entre les deux rapports, `--baseline` **ne refuse pas** : il écrit une **mention d'attribution** dans le bloc de comparaison.

Précisions :

- la forme proposée est `attribution: { generation: { reference, current }, model_fingerprint: "same" | "differs" | "unknown", queries_fingerprint: "same" | "differs" | "unknown" }`. `unknown` couvre la référence d'une version 1 ou 2, qui ne porte pas l'empreinte, et le mode API, qui n'écrit pas celle du modèle ;
- un écart de génération, de données ou d'encodeur rend la **cause ambiguë**, pas les chiffres faux : les moyennes restent comparables, et refuser la comparaison priverait le lecteur du seul chiffre disponible ;
- les refus d'ADR-0125, décision 12, restent entiers et portent sur ce qui rend les chiffres **non comparables** : un `k` différent, une `format_version` inconnue. La distinction est écrite ici : on refuse ce qui rend le calcul faux, on mentionne ce qui rend l'attribution incertaine ;
- la mention est aussi dans le Markdown (décision 7), pour que le lecteur du rapport seul la voie ;
- aucune comparaison n'est ajoutée pour `query_precision` : elle n'existe qu'en local, elle n'entre pas dans l'empreinte du modèle (ADR-0028 ne hashe ni `int8` ni la précision des requêtes), et deux rapports locaux qui en diffèrent le disent chacun dans leur en-tête. L'écart reste donc lisible sans qu'un membre de comparaison soit ajouté.

### Décision 7 — Le Markdown gagne une section de recouvrement de top-k

**Décision.** Le Markdown du rapport gagne une **section nouvelle** qui donne la moyenne du recouvrement de top-k, son **nombre de requêtes**, la liste des `id` dont le classement a changé, et la mention d'attribution.

Précisions :

- les **listes entières** de `doc_ids` restent dans le JSON : un Markdown de campagne doit rester lisible sans contexte, et 97 × 10 identifiants n'y sont pas lisibles ;
- la section suit le modèle de `docs/mesures/README.md`, dans la partie « Résultats », et reprend son `n` comme les autres moyennes (ADR-0125, décision 5) ;
- quand la comparaison n'a pas été faite — référence en version 1 ou 2, mode absent —, la section le **dit** au lieu de disparaître : une section absente se lirait comme un zéro ;
- la section nomme la comparaison **informative**, pour qu'aucun lecteur n'en déduise un verdict ;
- le reste du Markdown, ses sections but, machine, données, paramètres, lecture (ADR-0125, décision 11), ne change pas.

### Décision 8 — Confidentialité : des `doc_id` de résultat, aucun `iid`, aucun texte

**Décision.** Le rapport continue de n'identifier les **requêtes** que par leur `id`, et n'écrit que des `doc_id` **externes** de résultat : aucun `Iid`, aucun texte de requête, aucune valeur de filtre.

Précisions :

- la règle 11 d'`AGENTS.md`, INV-13 et ADR-0032 sont respectées : le `doc_id` externe est le seul identifiant visible hors du moteur, et il l'est déjà dans toute réponse de recherche ;
- ADR-0125, décision 16, borne l'identification **des requêtes** — par leur `id`, sans texte ni valeur de filtre — et n'interdit pas des `doc_id` de résultat. L'ADR-0138 a par ailleurs déjà versionné le jeu de référence de Legifacile sous `docs/legifacile/reference/`, qui ne contient aucune requête d'utilisateur final ;
- `queries_fingerprint` ne divulgue rien : c'est un hash, et il n'est pas réversible sur un jeu de phrases ;
- le coût est borné et daté : environ **97 × 10 identifiants**, soit **~20 Ko** par index et par mode. Le rapport reste un artefact unique, versionné par `@git`, jamais réécrit (ADR-0125, décision 13) ;
- les listes viennent de réponses déjà rendues à la commande : aucune donnée nouvelle n'est lue dans l'index pour les produire.

### Décision 9 — Reproductibilité : un ordre total, et un test en CI sur `synth-small`

**Décision.** À génération, jeu de requêtes et empreinte égaux, sur le même binaire et la même machine, le top-k groupé est **déterministe**. Un test de CI le vérifie sur `synth-small`, avec deux `crible eval` locaux et des requêtes à `vector`.

Précisions :

- l'ordre est total : score décroissant, puis clé croissante (ADR-0045 ; ADR-0072, décision 3 ; ADR-0079), le départage de la fusion (ADR-0090) et le regroupement (ADR-0075, décisions 1 et 3) étant eux aussi déterminés ;
- **réserve à écrire** : le produit scalaire fp16 avec dispatch SIMD (ADR-0070) peut départager autrement une **quasi-égalité** sur une autre architecture. Le test prouve le déterminisme sur une machine donnée, pas l'identité entre architectures ;
- le test suit ADR-0125, décision 14 : `synth-small` est généré en CI, ses vecteurs évitent modèle et données de production, et aucun test local n'est exclu de la CI ;
- le test vérifie deux choses : l'égalité des `modes.<mode>.queries[].doc_ids` et l'égalité de `queries_fingerprint` entre les deux rapports, à même génération ;
- il **prouve le déterminisme, pas la qualité** : aucun seuil de rappel, aucune cible n'y est assertée ;
- la comparaison de top-k de la décision 5 n'a de sens que si ce test passe : sans lui, un écart peut être un défaut de déterminisme.

### Décision 10 — Ce que l'acceptation appliquera

**Décision.** L'acceptation applique le tableau ci-dessous. Aucun de ces documents n'est modifié avant l'acceptation.

| Document | Modification |
|---|---|
| `docs/06-observabilite.md`, section « Éval » (`:150-179`) | les trois membres d'en-tête, `doc_ids` par ligne, `format_version: 3`, les versions lues par `--baseline`, la comparaison de top-k informative, la section nouvelle du Markdown, et la limite du mode API sur la route multi-index |
| `docs/mesures/README.md`, ligne des rapports d'éval (`:7`) | la mention du membre `doc_ids` et de la section de recouvrement, si le plan du Markdown change |
| `docs/adr/README.md` | mention de complément sur la ligne d'ADR-0125 et sur celle d'ADR-0131 |
| `docs/08-invariants-et-tests.md` | **rien** : aucun invariant nouveau n'est créé, le top-k n'est pas un invariant mais une mesure |
| `docs/versions/1.1.0.md` | **rien ici** : la mention du changement de `format_version` du rapport d'éval revient à `PUBLIER-V1.1.0`, qui écrit la note |

Précisions :

- ADR-0125 et ADR-0131 gardent leur corps : seule la mention « complété par ADR-0155 » s'ajoute à leur ligne d'index, comme ADR-0131 l'a fait pour ADR-0125 ;
- `docs/suivi.md` est tenu par l'orchestrateur ; la ligne de `T16-TOP-K-REJOUABLE` et son numéro d'ADR lui reviennent ;
- aucune reconstruction d'index, aucun ré-encodage, aucune migration de données n'est demandée par cette application.

## Conséquences

- **Positives** :
  - un écart de qualité devient **attribuable** : génération, empreinte de modèle et empreinte de jeu disent si les données, l'encodeur ou les requêtes ont changé ;
  - le top-k enregistré rend une campagne **rejouable** contre la précédente, sans dépendre d'un moteur arrêté ;
  - les échanges de rang, invisibles aux moyennes, deviennent **visibles** dans la liste des `id` changés ;
  - le rapport reste un **artefact unique**, daté et jamais réécrit ; aucun second fichier à synchroniser ;
  - aucune dépendance nouvelle : `blake3` est déjà là, et l'empreinte du modèle aussi ;
  - la confidentialité ne bouge pas : des `doc_id` externes et des `id` de requête, rien d'autre.
- **Négatives / coûts** :
  - le rapport grossit d'environ **20 Ko** par index et par mode, soit ~60 Ko pour une campagne à trois modes ;
  - une **troisième** version entre dans le lecteur de `--baseline` : le code de lecture gagne une branche, et chaque version future en ajoutera une ;
  - la comparaison de top-k est **informative** : un lecteur qui attend un garde-fou automatique contre un réordonnancement ne l'aura pas ;
  - le **mode API reste borné à la route simple** : le top-k servi par la route multi-index, celui de Legifacile, n'est pas couvert, et le ticket 16 n'est donc pas entièrement satisfait pour le servi ;
  - `queries_fingerprint` demande une forme canonique écrite une fois de plus, et un test qui la fige.
- **À surveiller** :
  - la **taille** du rapport si `k` ou le nombre de requêtes augmente : 97 × 10 tient, 97 × 100 demanderait une décision ;
  - le **recouvrement de top-k** après chaque mise à jour DILA : une baisse sans changement d'empreinte est une dérive des données, pas une régression, et la lecture doit rester prudente ;
  - les **quasi-égalités fp16** sur une autre architecture (ADR-0070) : un écart de top-k entre deux machines de modèles de CPU différents n'est pas forcément un défaut ;
  - un besoin du **top-k servi** par la route multi-index : il demanderait un ADR qui remplace ADR-0131, décision 1, et la question se rouvrira si Legifacile en dépend pour J7 ;
  - la stabilité de la forme canonique du jeu : la changer invaliderait toutes les empreintes passées sans invalider les rapports, ce qui rendrait la comparaison d'attribution muette pour les campagnes antérieures.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Un fichier annexe, `--record-top-k` et `--compare-top-k` | Deux artefacts à garder synchronisés, et le rapport cesserait d'être la source unique d'une campagne ; c'est contraire à ADR-0125, décisions 11 et 13 |
| Écrire des clés `doc_id#ord` dans `doc_ids` | Le jugement est au document (ADR-0125, décision 4 ; ADR-0092, décision 2) ; la fidélité au passage a déjà ses métriques, et une clé de passage rendrait la comparaison de Legifacile dépendante du découpage |
| Faire échouer `--baseline` sur le recouvrement de top-k | La dérive des données déplace le top-k sans régression du moteur ; ADR-0125, décision 12, ne fait valoir 1 qu'aux moyennes de qualité et de fidélité |
| Refuser une référence d'une autre génération ou d'une autre empreinte | Cela rendrait la cause ambiguë, pas les chiffres faux ; refuser priverait le lecteur du seul chiffre disponible, et la mention d'attribution suffit |
| Hasher les octets bruts du jeu de requêtes | Un simple reformatage du fichier changerait le hash sans changer le jeu, et ferait croire à une campagne différente |
| Étendre le mode API à la route multi-index dans T16 | La réponse multi-index n'a pas de `generation` scalaire (ADR-0110, décision 1), et ADR-0131, décision 1, a écarté cette route ; l'étendre demanderait un ADR qui la remplace, hors du périmètre de la tâche |
| Garder `format_version: 2` et ajouter les membres sans changer de version | Un lecteur v2 ne connaît ni `doc_ids` ni les trois membres d'en-tête ; le changement de version rend l'ajout explicite, comme v1 → v2 par ADR-0131, décision 7 |
| Écrire `model_fingerprint` même pour un modèle `external` | L'empreinte ne hasherait que `dims` et `normalize` (`crates/crible-core/src/model.rs:305-307`) : elle ferait croire à une attribution que la configuration ne permet pas |
| Écrire `query_precision` dans toutes les campagnes | Une campagne à `vector` ou `lexical` n'encode aucun texte : `"fp32"` y serait faux |
| Ajouter une option pour choisir la précision écrite dans le rapport | Ce serait une configuration pour une valeur fixe, contraire à la règle 12 ; la valeur est déjà dans la configuration de l'encodeur |

## Références

Les ADR cités dans ce document sont ceux de **Crible**, sauf mention « ADR de Legifacile ». La source du besoin est le ticket **16 de Legifacile** (`../legifacile/tickets-crible/16-reference-top-k-rejouable.md`), lu en lecture seule ; ses ADR, s'il en cite, sont ceux de son dépôt.

- Fiche et suivi : `docs/suivi.md:178` (`T16-TOP-K-REJOUABLE`, classe provisoire d'ADR-0143) ; verdict de `@architecte` du 2026-11-12 (NON-COUVERT, ajout compatible).
- Conception : `docs/06-observabilite.md:150-179` (« Éval : `crible eval` ») ; `docs/mesures/README.md:7` (rapports d'éval) ; `docs/07-stack-et-deploiement.md:44` (`blake3`) ; `docs/08-invariants-et-tests.md` (INV-13, INV-15).
- Code : `crates/crible/src/eval.rs` (`FORMAT_VERSION` à `:114`, `BASELINE_VERSIONS` à `:118-121`, `REGRESSION_THRESHOLD` à `:111`, `Run.docs` à `:200-204`, `header` à `:559-596`, ligne par requête à `:1106-1121`, run groupé à `:695-705`, refus de version à `:1383-1392`, `compare` à `:1402`, `lost_queries` à `:1486`) ; `crates/crible-core/src/model.rs` (`fingerprint` à `:370`, validation d'un modèle `external` à `:305-307`).
- ADR de Crible : ADR-0028 (forme canonique de l'empreinte), ADR-0032 (accès et secrets), ADR-0045, ADR-0070, ADR-0072 (décision 3), ADR-0075 (décisions 1 et 3), ADR-0079, ADR-0090, ADR-0092 (décision 2), ADR-0052 (`query_precision` fp32), ADR-0110 (décision 1, réponse multi-index), ADR-0125 (décisions 4, 11, 12, 13, 14 et 16), ADR-0131 (décisions 1, 7, 8 et 9), ADR-0138 (jeu de référence versionné), ADR-0143 (décisions 2 et 7), ADR-0148 (forme du tableau d'application).
- Mesures : MES-05 (recouvrements du top 10 et écart inattribuable) ; MES-10 et MES-11 (identification des questions par `id`).
