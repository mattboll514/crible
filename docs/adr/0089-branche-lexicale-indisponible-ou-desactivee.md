# ADR-0089 — Branche lexicale indisponible ou désactivée : messages des états sans commit tantivy et code d'erreur quand `lexical.enabled` vaut `false`

- **Statut** : Accepté (accepté par anticipation par le propriétaire le 2026-10-28, à confirmer au point de fin de l'enchaînement)
- **Date** : 2026-10-28 (proposition et acceptation anticipée)
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, sur demande de l'orchestrateur, pour la tâche `RECHERCHER-BM25-DANS-LE-POOL` (phase P5), à partir des questions de `@testeur` avant `@dev` et des verdicts de `@architecte` du 2026-10-28
- **Remplace** : — (précise ADR-0088, décision 5, et ADR-0087, dont le corps n'est pas réécrit : cet ADR ajoute deux messages aux quatre d'ADR-0088 et tranche le code d'erreur qu'ADR-0088, décision 8, laissait ouvert)

## Contexte

### Ce que la conception et les ADR exigent

- **La branche lexicale est livrée par `RECHERCHER-BM25-DANS-LE-POOL`** (`docs/phases/P5-lexical-et-hybride.md:59-75`). Son livrable de refus (`:68`) reprend les quatre messages de la décision 5 d'ADR-0088, et son critère d'acceptation (`:73`) exige qu'une recherche `lexical` ou `hybride` échoue en `overloaded` (503) « avec le message de l'état constaté », le mode `dense` restant servi sur le même instantané.
- **ADR-0087, décision 2**, interdit la dégradation silencieuse de `hybrid` vers le dense : une réponse réduite au dense porterait un `score_type` et un classement que le client n'a pas demandés. **Décision 3** fixe `overloaded` (503) pour un lecteur lexical absent, parce que l'état est temporaire et se répare à la prochaine ouverture en écriture.
- **ADR-0088, décision 5**, donne quatre messages qui nomment l'état constaté : « behind generation `Gt` of `G` » (`M > Gt`), « ahead of generation `Gt` of `G` » (`Gt > G`), « state at generation `Gt` is not certified for generation `G` » (colonne `M` absente), et « unreadable at generation `G` » (`lexical/` illisible ou `M > G`). **Décision 8** laisse explicitement ouvert « le code d'erreur d'une recherche lexicale quand `lexical.enabled` vaut `false` » (`docs/adr/0088-index-lexical-a-jour-et-lexical-illisible-a-la-reprise.md:126`), comme ADR-0087:48.
- **Les invariants en cause** : INV-01 interdit qu'un identifiant interne sorte du moteur, y compris dans un message d'erreur (`docs/08-invariants-et-tests.md:9`) ; INV-05 fait du pool l'autorité des résultats, et la bitmap des vivants reste l'autorité sur les suppressions (`docs/08:13`, `docs/03-recherche.md:92`).

### Verdict de `@architecte` — Q1, états sans commit tantivy : `NON-COUVERT`

`LexicalStatus` (`crates/crible-store/src/lexical/mod.rs:228-272`) décrit six états, dont deux n'ont **aucun commit tantivy** à citer :

- `Behind { committed: None }` (`mod.rs:246-250`) : des passages existent — `M` vaut une génération non nulle — mais l'époque ne porte aucun index lexical commité. `no_commit_status` le rend pour tout marqueur non nul (`mod.rs:374-381`).
- `NotCertified { committed: None }` (`mod.rs:262-267`) : la colonne `M` manque, l'époque est antérieure à P5, et aucun index n'est commité ; l'absence d'index ne dit rien des passages que l'époque porte (`mod.rs:369-373`).

Or les quatre messages d'ADR-0088 s'écrivent pour une époque dotée d'un commit : trois citent `Gt`, le quatrième nomme `G`, et aucun ne décrit une époque qui n'a jamais commité. Écrire « behind generation `Gt` of `G` » quand aucun commit n'existe affirmerait un fait faux, et une phrase unique sans numéro contredirait ADR-0088:97, qui exige que le message nomme l'état réellement constaté. Le cas n'est pas théorique : il est celui d'une époque restaurée sans son dossier dérivé, ou d'un index ancien jamais ouvert en écriture, et l'opérateur doit distinguer un **rattrapage** (`M > Gt`) d'une **reconstruction** (aucun commit).

**`Missing` (`mod.rs:232-234`) n'est pas un refus.** L'état vaut `M = 0` : aucune génération de l'époque n'a créé ni fait mourir un passage (`mod.rs:377-378`). Le pool est donc vide, et la réponse vide est exacte. Refuser en `overloaded` annoncerait une indisponibilité pour une branche qui n'a rien à servir, et ferait échouer une requête que le pool vide suffit à expliquer : rien n'est en retard ni illisible.

### Verdict de `@architecte` — Q3, `lexical.enabled` à `false` : `NON-COUVERT`

`LexicalStatus::Disabled` (`mod.rs:228-231`) est rendu dès que `lexical.enabled` vaut `false` (`mod.rs:313-315`), et aucun ADR ne dit ce que fait alors une recherche `lexical` ou `hybrid`. Le défaut de `mode` est déjà réglé et ne change pas : `hybrid` si le lexical est activé, sinon `dense` (`docs/05-api.md:160`). Reste la requête explicite.

La distinction qui tranche est la durée : `lexical.enabled` est une propriété **permanente** de l'index, connue à la validation de la configuration et figée pour l'index, alors qu'`overloaded` désigne une indisponibilité **temporaire** qu'on réessaie (`docs/05:42`, ADR-0087:38). Répondre 503 avec `Retry-After` ferait réessayer indéfiniment une requête qui ne peut pas réussir.

Le cas voisin doit être tranché dans le même ADR : une recherche **multi-index** (`docs/03:119-128`) dont une cible a le lexical désactivé. La conception ne décrit que le partage du pool, des branches, de la fusion et du regroupement par index (`docs/03:124`), et l'interdiction de dégradation d'ADR-0087 ne vaut pas moins pour une cible que pour un index seul.

### Rappels, sans nouvelle décision

- **Départage du classement lexical** : score décroissant, puis clé `doc_id#ord` croissante (ADR-0045, ADR-0072, ADR-0079). L'`iid` n'est jamais un critère final : il n'est stable que dans une époque et ne sort pas du moteur (INV-01).
- **`explain`** : la fonction de P5 rend l'explication tantivy d'un passage désigné par sa clé externe, sans structure JSON figée (`docs/06-observabilite.md:84`, `docs/05-api.md:211-215`). Cet ADR ne change ni la règle d'ordre ni la forme de `explain`.

## Décision

### 1. Deux messages de plus pour les états sans commit tantivy, toujours en `overloaded` (503)

Le refus de la branche lexicale garde `overloaded` (503) pour tous les états non servis, comme le fixe ADR-0087, décision 3. Deux états n'ont pas de `Gt` à citer et reçoivent un message propre, qui nomme l'état constaté et la génération de référence `G` :

| État constaté | Message |
|---|---|
| `Behind { committed: None }` | « no lexical index is committed in this epoch although passages exist (generation G); open the index for writing to rebuild it » |
| `NotCertified { committed: None }` | « lexical index state is not certified for generation G; open the index for writing to repair it » |

`G` est remplacé par le numéro de la génération de référence. Le premier message dit **rebuild**, et non *repair*, parce qu'aucun commit n'existe : l'ouverture en écriture reconstruit l'index de l'époque, elle ne rattrape pas un retard. Le second ne promet pas non plus un rattrapage : sans marqueur, l'ouverture en écriture complète `M` puis décide (ADR-0088, décision 4).

Les quatre messages d'ADR-0088, décision 5, restent inchangés et valent pour les états qui ont un commit : `Behind { committed: Some(Gt) }`, `Ahead`, `NotCertified { committed: Some(Gt) }` et `Unreadable`. Aucun `iid` ne figure dans ces six messages (INV-01) : `G`, `Gt` et l'état sont des faits visibles.

### 2. `Missing` ne refuse pas : la réponse est vide

Une époque dont `M = 0` et qui ne porte aucun commit lexical n'a jamais créé de passage : son pool est vide. `lexical` et `hybrid` réussissent alors avec une réponse vide, sans erreur et sans message. `dense` est servi comme partout ailleurs. Aucun message de la décision 1 ne s'applique.

### 3. `lexical.enabled` à `false` : `invalid_request` (400), pour `lexical` comme pour `hybrid`

Une recherche qui demande explicitement la branche lexicale sur un index dont `lexical.enabled` vaut `false` est refusée en `invalid_request` (400), avec le message « lexical branch is disabled for this index », que le mode soit `lexical` ou `hybrid`. La requête n'est pas fautive au sens d'une syntaxe invalide ; elle est **inapplicable à cet index**, et cette propriété est permanente et connue à la validation. `overloaded` est écarté : réessayer ne peut pas réussir. `internal` est écarté : ce n'est pas une panne.

Le défaut de `mode` reste celui de `docs/05:160` : `hybrid` si le lexical est activé, sinon `dense`. Une requête qui n'énonce pas `mode` n'est donc jamais refusée pour cette raison sur un index sans lexical.

### 4. Recherche multi-index : une cible sans lexical fait refuser la requête entière

Si une cible d'une recherche `lexical` ou `hybrid` a `lexical.enabled` à `false`, la requête entière est refusée en `invalid_request` (400), avec le message « lexical branch is disabled for this index » suivi du nom de l'index fautif. Aucune cible n'est servie en dense à sa place, et aucune réponse partielle n'est rendue : une réponse `hybrid` réduite au dense pour une cible porterait le même `score_type` faux que la dégradation qu'ADR-0087, décision 2, interdit pour un index seul.

Le nom d'index est public et figure déjà dans chaque résultat (`docs/03:128`) : il ne viole pas INV-01.

## Conséquences

- **Positives** :
  - chaque refus dit la vérité sur l'état constaté : l'opérateur distingue un rattrapage d'une reconstruction, et une branche désactivée d'un index en retard ;
  - un index sans passage ne rend plus d'erreur là où la réponse vide est exacte, ce qui évite de refuser une requête que le pool vide explique entièrement ;
  - un client qui réessaie sur `overloaded` ne boucle plus sur une propriété permanente ;
  - aucune cible d'une recherche multi-index n'est silencieusement servie par une seule branche.
- **Négatives / coûts** :
  - six messages au lieu de quatre, donc six cas à tester dans la branche lexicale, alors qu'ADR-0088 en annonçait déjà quatre ;
  - `overloaded` couvre encore deux cas de plus, dans un code déjà chargé (`docs/05:42`) ;
  - une recherche multi-index `hybrid` échoue en entier pour une seule cible mal configurée : le client doit corriger sa liste de cibles, là où un service partiel aurait rendu des résultats ;
  - `invalid_request` (400) est employé pour une requête bien formée mais inapplicable à l'index, ce qui élargit légèrement le sens du code dans `docs/05`.
- **Tests exigés** :
  - `Behind { committed: None }` : `lexical` et `hybrid` refusés en `overloaded` avec le message « no lexical index is committed… rebuild », `dense` servi ;
  - `NotCertified { committed: None }` : refus en `overloaded` avec le message « … is not certified for generation G; … repair », `dense` servi ;
  - `Missing` : `lexical` et `hybrid` rendent `200` avec une réponse vide, sans erreur ;
  - `lexical.enabled: false` : `lexical` et `hybrid` refusés en `invalid_request` avec « lexical branch is disabled for this index », `dense` servi ;
  - multi-index avec une cible sans lexical : la requête entière est refusée en `invalid_request`, et le message nomme l'index fautif ;
  - aucun message ne contient d'`iid` (INV-01).
- **À surveiller** :
  - la fréquence de `Behind { committed: None }` : un index sain qui tombe dans cet état à chaque démarrage signalerait une reconstruction qui ne commite pas, ou un dossier dérivé effacé hors du moteur ;
  - l'usage réel de `hybrid` multi-index avec des cibles hétérogènes : si les clients configurent souvent des cibles sans lexical, le refus global deviendra gênant et il faudra peut-être une règle de périmètre, décidée par un ADR ;
  - l'hypothèse de `Missing` repose sur la tenue de `M` par tout chemin qui crée un passage (ADR-0088, « À surveiller ») : un `M` resté à `0` alors que des passages existent ferait rendre une réponse vide fausse au lieu d'une erreur. Un test de propriété sur `M` reste le garde-fou ;
  - le nombre de messages à tenir : ajouter un état à `LexicalStatus` obligera à repasser par `docs/05` et par la fiche.
- **Documents modifiés à l'acceptation** (appliqués avec cet ADR, le propriétaire l'ayant accepté par anticipation) :

| Document | Modification |
|---|---|
| `docs/05-api.md:31` | `invalid_request` : ajouter le cas d'une recherche `lexical` ou `hybrid` sur un index dont le lexical est désactivé (ADR-0089) |
| `docs/05-api.md:42` | `overloaded` : nommer aussi les deux états sans commit tantivy et leurs messages, et dire qu'une époque `Missing` rend une réponse vide sans erreur (ADR-0089) |
| `docs/05-api.md:224-237` | recherche multi-index : une cible sans lexical fait refuser la requête entière en `invalid_request`, le message nommant l'index (ADR-0089) |
| `docs/03-recherche.md:78-96` | branche lexicale : ajouter les deux messages des états sans commit, le cas `Missing` et le refus d'une branche désactivée (ADR-0089) |
| `docs/03:119-128` | recherche multi-index : ajouter le refus global quand une cible a le lexical désactivé (ADR-0089) |
| `docs/phases/P5-lexical-et-hybride.md:68,73-75` | fiche `RECHERCHER-BM25-DANS-LE-POOL` : deux messages de plus, `Missing` sans refus, branche désactivée en `invalid_request`, et critères correspondants |
| `docs/adr/README.md` | ligne d'index de cet ADR |
| `docs/08-invariants-et-tests.md` | **aucune modification nécessaire** : INV-01 et INV-05 ne changent pas ; les nouveaux comportements sont des tests |
| `docs/suivi.md` | hors de cet ADR : la ligne d'écart du 2026-10-28 sur `RECHERCHER-BM25-DANS-LE-POOL` doit citer ADR-0089, et l'orchestrateur la tient |

`docs/05:160` (défaut de `mode`) et `docs/06:84` (`explain`) restent **inchangés**.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Une phrase unique sans numéro de génération pour tous les états | Contredit ADR-0088:97, qui exige que le message nomme l'état constaté ; l'opérateur ne saurait ni quelle génération est en cause, ni s'il faut rattraper ou reconstruire |
| Étendre le message « behind generation `Gt` of `G` » à `Behind { committed: None }` | Affirme un commit qui n'existe pas ; laisse croire qu'un rattrapage aura lieu alors que l'ouverture en écriture reconstruit l'index entier |
| Réutiliser le message « not certified at generation `Gt` » pour `NotCertified { committed: None }` | Cite un `Gt` absent. Le message sans `Gt` dit exactement ce qui est su : le marqueur manque, l'état n'est pas certifié |
| Refuser `Missing` en `overloaded` | Le pool est vide par construction (`M = 0`) : la réponse vide est exacte, et un refus annoncerait une indisponibilité qui n'existe pas |
| `overloaded` (503) pour une branche désactivée | La propriété est permanente et connue à la validation : `Retry-After` ferait réessayer une requête qui ne peut pas réussir (ADR-0087:38 distingue l'indisponibilité temporaire) |
| `internal` (500) pour une branche désactivée | Ce n'est pas une panne du moteur, et cela fausserait les analytics |
| Un code dédié `lexical_disabled` (400 ou 409) | Une ligne de plus dans `docs/05`, un cas de plus dans les analytics et dans les clients, pour une situation que `invalid_request` décrit déjà : la requête demande une branche absente de cet index. ADR-0087:42 a déjà écarté un code dédié pour l'indisponibilité |
| Servir une cible sans lexical en dense dans une recherche multi-index | Même dégradation silencieuse qu'ADR-0087, alternative (e) : le client ne peut pas distinguer une réponse partielle d'une réponse complète |
| Refuser seulement la cible fautive et rendre les autres | Réponse partielle non signalée : la liste `targets` demandée n'est pas celle qui a répondu, et le client n'a aucun moyen de le savoir |
| `not_found` (404) ou `unknown_index` (404) pour une branche désactivée | L'index existe et ses résultats denses sont disponibles : accuser son absence serait faux |
| Considérer `Disabled` comme un état lexical de plus dans la table d'ADR-0088 | La table d'ADR-0088 traite l'indisponibilité temporaire d'un index qui possède une branche ; une branche absente par configuration n'est pas un état du lexical, et le code d'erreur diffère (décision 3) |

## Références

- `docs/03-recherche.md:78-96` — branche lexicale, texte indexé, analyse, requête, suppressions, lexical à jour, reconstruction, états sans commit et branche désactivée ; `:119-128` — recherche multi-index.
- `docs/05-api.md:29-44` — codes d'erreur, `invalid_request`, `overloaded` ; `:152-171` — champs de la requête, défaut de `mode` ; `:208-218` — `explain` ; `:224-237` — recherche multi-index.
- `docs/06-observabilite.md:77-87` — debug et `explain`, explication tantivy.
- `docs/08-invariants-et-tests.md:9,13` — INV-01 (aucun id interne hors du moteur), INV-05 (le pool est l'autorité).
- `docs/phases/P5-lexical-et-hybride.md:59-75` — `RECHERCHER-BM25-DANS-LE-POOL`, livrables et critères ; `:76-89` — `FUSIONNER-HYBRIDE`.
- `crates/crible-store/src/lexical/mod.rs:228-272` — `LexicalStatus` et ses six états ; `:294-365` — `observe` ; `:367-410` — `no_commit_status` et `status_of` ; `:313-315` — `Disabled`.
- ADR-0045 (référence exacte, ordre des scores), ADR-0072 (produit scalaire partagé), ADR-0079 (coût des clés, ordre après tri), ADR-0086 (`lexical.language`), ADR-0087 (reprise lexicale, lecture seule, refus en `overloaded`), ADR-0088 (définition d'un index à jour, quatre messages, décision 8 laissant le code de la branche désactivée ouvert).
