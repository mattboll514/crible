# ADR-0145 — Discriminant stable du refus d'une branche lexicale désactivée dans `details`

- **Statut** : Accepté (accepté par anticipation par le propriétaire le 2026-11-11)
- **Date** : 2026-11-11 (proposition et acceptation anticipée)
- **Décideurs** : propriétaire
- **Proposé par** : agent `@greffier`, à la demande du propriétaire du 2026-11-11, pour la tâche `T15-CODE-REFUS-LEXICAL` de la v1.1, sur le verdict de `@architecte` du 2026-11-11 et sur le ticket 15 du projet Legifacile (`tickets-crible/15-code-stable-pour-le-refus-de-mode.md`, lecture seule)
- **Remplace** : — (complète ADR-0089 et ADR-0101 sans modifier leur corps ; ne remplace aucun ADR)

## Contexte

### Ce que la conception et les ADR exigent déjà

- **Le refus existe et son code est fixé.** Une recherche `lexical` ou `hybrid` sur un index dont `lexical.enabled` vaut `false` est refusée en `invalid_request` (400), avec le message « lexical branch is disabled for this index » ; une recherche multi-index dont une cible a le lexical désactivé est refusée en entier, le message nommant l'index fautif (ADR-0089, décisions 3 et 4, `docs/adr/0089-branche-lexicale-indisponible-ou-desactivee.md:62-72`). ADR-0089 a écarté un code dédié `lexical_disabled` (alternatives, `:124`).
- **`details` est fermé.** Il vaut `{}` par défaut ; une clé n'y figure que si `docs/05` la documente pour ce code — aujourd'hui `path` pour `invalid_filter` seulement —, et une tâche qui ajoute une clé la documente dans `docs/05` **dans le même changement** (ADR-0101, décision 3, `docs/adr/0101-corps-d-erreur-message-details-et-reponses-hors-route.md:33` ; `docs/05-api.md:35`).
- **La ligne de la table d'erreurs est déjà écrite.** `docs/05-api.md:39` documente le cas « recherche `lexical` ou `hybrid` sur un index dont `lexical.enabled` vaut `false` » et renvoie à ADR-0089 ; elle ne dit rien de `details` pour ce cas.
- **La classe de compatibilité est fixée.** Une **clé de `details` nouvelle** est un ajout permis dans une mineure (ADR-0143, décision 1, `docs/adr/0143-politique-de-version-de-la-ligne-1-x-et-procedure-de-montee-et-de-retour-arriere.md:56`), et l'ADR de chaque ticket de contrat déclare sa classe (décision 7, `:120`). La tâche `T15-CODE-REFUS-LEXICAL` est classée **ajout compatible** (`docs/suivi.md:176`).
- **La note de version liste les ajouts de contrat**, dont les clés de `details` (ADR-0143, décision 4, `:90-92`).
- **Le client Python expose `details` tel quel** (ADR-0112) : un membre nouveau ne casse aucun client ancien, et aucune version de client n'est à synchroniser.

### Le constat du ticket 15

Le corps observé sur Crible 1.0.0 est `{"error":{"code":"invalid_request","message":"…","details":{},"request_id":"…"}}`, sur la route simple comme sur la route multi-index. Le serveur MCP de Legifacile doit reconnaître ce refus sans jamais basculer en `dense` ; il le fait par un test de sous-chaîne sur le message, et son premier membre — `details.path == "$.mode"` — est toujours faux en production, parce que Crible n'envoie pas cette clé. Une reformulation du message ferait retomber son erreur sur un texte générique, et un `400` qui parle de « lexical » pour une autre raison serait un faux positif. Le discriminant manque donc, non le code d'erreur.

### Pourquoi `path` seul ne discrimine pas

`$.mode` est déjà le chemin d'une autre faute `invalid_request` : la valeur de mode inconnue, `parse_mode` (`crates/crible-server/src/search.rs:1704-1714`), dont le message est « '$.mode' must be dense, lexical or hybrid, got … ». Ce message contient le mot « lexical » lui-même. Un client qui lit `details.path == "$.mode"` ne peut donc pas distinguer « le mode demandé n'existe pas » de « le mode demandé est inapplicable à cet index » si `path` est publié sur l'un et pas sur l'autre avec le même sens.

Un `mode` **absent** ne produit jamais ce refus, sur les deux routes : la route simple prend le défaut `dense` quand le lexical est désactivé (ADR-0089, décision 3, `docs/05-api.md:160`), et la route multi-index prend `hybrid` seulement si **toutes** les cibles ont le lexical (ADR-0110, `docs/05-api.md:393`). En cas de refus, `$.mode` est donc toujours présent dans le corps : le chemin est vrai, mais il n'est pas discriminant.

### Ce que le rendu fait aujourd'hui

- `CribleError` porte un code, un message et un `path` facultatif (`crates/crible-core/src/error.rs:105-111`), attaché par `with_path` (`:124-132`).
- Le rendu ne publie `path` que pour `invalid_filter` (`crates/crible-server/src/error.rs:140-143`) ; tout le reste rend `{}`.
- `with_path` n'a **qu'un appelant** dans le code, sur `invalid_filter` (`crates/crible-core/src/filter.rs:396`). Aucun `invalid_request` ne publie de `path`.
- Le refus qui nous occupe est construit à deux endroits de l'API : route simple (`crates/crible-server/src/search.rs:204-212`, pré-contrôle avant le jeton) et route multi-index (`crates/crible-server/src/search_multi.rs:585-591`).
- La bibliothèque construit le même refus pour ses propres appelants (`crates/crible-search/src/lexical.rs:418-427`), et la CLI le construit une troisième fois pour son message (`crates/crible/src/commands.rs:772`). Aucun des deux ne rend un corps d'erreur d'API.

### Ce qui est vérifié

Vérifié le 2026-11-11 dans l'arbre : les deux sites de refus et leurs lignes, `parse_mode` et son message, `with_path` et son unique appelant, la règle de rendu `error.rs:140-143`, les trois passages cités de `docs/05`, `docs/03-recherche.md:98`, la description de `ErrorBody.details` (`crates/crible-server/src/openapi.rs:76-82`), et les décisions citées d'ADR-0089, ADR-0101, ADR-0112 et ADR-0143. Le ticket 15 a été lu dans le dépôt de Legifacile, en lecture seule.

## Décision

### 1. `details = {"path": "$.mode", "reason": "lexical_branch_disabled"}` sur les deux routes

Sur ce refus, route simple et route multi-index, le corps porte :

```json
{"error": {"code": "invalid_request",
           "message": "lexical branch is disabled for this index",
           "details": { "path": "$.mode", "reason": "lexical_branch_disabled" },
           "request_id": "…"}}
```

Le code (`invalid_request`, 400) et le message ne changent pas. En multi-index, le message continue de nommer l'index fautif après `: '<index>'` (ADR-0089, décision 4) ; le corps ne porte **pas** de clé `details.index`.

### 2. `reason` est le discriminant, `path` localise

`reason` dit **pourquoi** le mode est refusé ; `path` dit **où** se trouve le champ fautif. Le vocabulaire de `reason` est **fermé** et documenté dans `docs/05` : une valeur par cas documenté, aujourd'hui la seule `lexical_branch_disabled`.

Une valeur nouvelle suit la règle d'ADR-0101, décision 3 : elle est documentée dans `docs/05` **dans le même changement** que le code qui la produit. En pratique, `reason` étant un vocabulaire fermé de contrat, une valeur nouvelle demande aussi son ADR, comme toute décision qui engage le projet (règle 4 d'`AGENTS.md`) : c'est ce que la documentation dans le même changement rend possible, pas ce qu'elle remplace.

### 3. Rendu : `path` pour `invalid_request` seulement quand l'erreur porte une `reason`

Le rendu de `invalid_request` ne publie `path` que si l'erreur porte aussi une `reason`. Côté code, la `reason` est un type fermé — par exemple `ErrorReason`, avec un seul variant `LexicalBranchDisabled` aujourd'hui —, pour qu'un chemin ne puisse pas être publié sans discriminant.

- `invalid_filter` garde `path` seul, comme aujourd'hui ; son corps ne change pas.
- Tout le reste rend `{}`.
- Les refus `invalid_request` qui nomment un chemin dans leur **message** sans porter de `reason` — champ inconnu, mode inconnu (`parse_mode`), `targets` vide ou dupliqué, etc. — gardent `details: {}`. Leur message reste leur seul chemin, comme avant.

Le rendu ne publie jamais de texte de requête ni de valeur de filtre : `path` est un chemin JSON fixe et `reason` une valeur d'un vocabulaire fermé. `details` n'est pas journalisé (ADR-0101, décision 4), donc rien de nouveau n'entre dans les logs.

### 4. Documentation, dans le même changement que le code

| Document | Modification |
|---|---|
| `docs/05-api.md:35` | règle de `details` : ajouter `path` et `reason` pour le refus de branche lexicale désactivée, et dire que `path` d'un `invalid_request` n'est publié qu'avec une `reason` |
| `docs/05-api.md:39` | ligne `invalid_request` : nommer `details = {"path": "$.mode", "reason": "lexical_branch_disabled"}` sur le cas déjà cité (ADR-0089) |
| `docs/05-api.md:393` | mode par défaut multi-index : le refus garde le message et le nom de l'index fautif, et porte le même `details` |
| `docs/03-recherche.md:98` | branche désactivée : ajouter le discriminant et renvoyer à cet ADR |
| `crates/crible-server/src/openapi.rs:76-82` | description de `ErrorBody.details` : citer les clés documentées (`path` et `reason` pour ce refus, `path` pour `invalid_filter`) ; le schéma ne change pas, `details` reste un objet libre |
| `docs/versions/1.1.0.md` | note de version (ADR-0143, décision 4) : lister la clé `details.reason` et sa valeur, à la publication |
| `docs/adr/README.md` | ligne d'index de cet ADR |

Le client Python n'a **aucun** changement : il expose `details` tel quel (ADR-0112). Les corps d'ADR-0089 et d'ADR-0101 ne sont **pas** réécrits.

### 5. Classe de compatibilité : ajout compatible

L'ajout d'une clé de `details` est une catégorie permise d'une mineure (ADR-0143, décision 1). La tâche `T15-CODE-REFUS-LEXICAL` est classée **ajout compatible** (`docs/suivi.md:176`) ; cet ADR le déclare conformément à la décision 7.

### Ce qui n'est pas décidé ici

- L'ajout d'une clé `details.index` en multi-index : le message nomme déjà l'index, et une clé de plus serait une décision séparée, à prendre si Legifacile la demande.
- Un code d'erreur dédié pour ce refus : écarté par ADR-0089 et interdit en mineure par ADR-0143, décision 1.
- Les producteurs non-API du même message — la bibliothèque (`crates/crible-search/src/lexical.rs:418-427`) et la CLI (`crates/crible/src/commands.rs:772`) : cet ADR ne leur ajoute rien, le discriminant étant un membre du corps d'erreur de l'API.
- Le libellé exact des autres messages et la liste future des valeurs de `reason` : chaque valeur nouvelle suit la décision 2.

### Ce que l'acceptation applique

La documentation de la décision 4 est faite **sur la branche de la tâche** `tache/T15-CODE-REFUS-LEXICAL`, dans le même changement que le code. `@greffier` ne l'a pas faite à l'acceptation. L'orchestrateur ajoute la ligne de la section « Écarts à la conception » de `docs/suivi.md` citant cet ADR ; `@greffier` ne touche pas à `docs/suivi.md`.

## Conséquences

- **Positives** :
  - un client reconnaît le refus par une valeur fermée, sans lire une phrase anglaise : une reformulation du message ne casse plus le MCP de Legifacile ;
  - le mode inconnu et la branche désactivée se distinguent, alors que `path` seul ne le permettait pas ;
  - le discriminant ne dépend ni d'une nouvelle route, ni d'un nouveau code, ni d'un membre de réponse : aucune version de client n'est à synchroniser ;
  - la règle de rendu reste fermée : un futur `with_path` sur un `invalid_request` ne publie rien sans `reason`, et ADR-0101, décision 3, reste vrai.
- **Négatives / coûts** :
  - une règle de rendu de plus, et un type nouveau côté `crible-core` pour porter la `reason` ;
  - trois documents de conception et une description d'OpenAPI à tenir à jour avec la même valeur ;
  - deux vocabulaires ouverts à surveiller : les valeurs de `reason` et les cas de `invalid_request` qui n'en portent pas ;
  - un client qui compare `details` à `{}` pour ce refus voit un objet nouveau : c'est un ajout, mais il doit lire les clés, pas l'égalité.
- **Tests exigés** :
  - route simple, `lexical` et `hybrid` sur un index sans lexical : `400`, code et message inchangés, `details == {"path": "$.mode", "reason": "lexical_branch_disabled"}` ;
  - route multi-index, une cible sans lexical : même `details`, message nommant l'index fautif ;
  - mode inconnu : `400`, `details == {}`, message inchangé ;
  - `invalid_filter` : `details == {"path": "…"}` sans `reason` ;
  - une autre erreur d'API : `details == {}` ;
  - aucun corps d'erreur ne contient d'iid (INV-01) ni de texte de requête.
- **À surveiller** :
  - une valeur de `reason` ajoutée sans documentation dans `docs/05` ni ADR : le signe serait un client qui découvre une valeur absente de la table ;
  - un `with_path` posé sur un `invalid_request` sans `reason` : le rendu le masquerait, et la revue doit voir que le message reste le seul chemin ;
  - l'usage réel de `details.path == "$.mode"` par Legifacile : si le client s'appuie sur `path` seul, il retombe dans l'ambiguïté que cet ADR écarte ; c'est `reason` qui doit être lu.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Un code d'erreur dédié `lexical_disabled` (400) | Déjà écarté par ADR-0089 (alternatives, `:124`), et interdit en mineure : changer le code d'un cas documenté est une rupture (ADR-0143, décision 1). Il faudrait une ligne de table, un variant d'`ErrorCode`, et une version de client synchronisée. |
| `path` seul (`{"path": "$.mode"}`) | Ambigu avec le mode inconnu, dont le message cite déjà `$.mode` et contient « lexical » (`crates/crible-server/src/search.rs:1704-1714`) : le client ne peut pas distinguer les deux fautes. |
| Élargir le rendu de `path` à tout `invalid_request` | Bénirait tout futur `with_path` non documenté : `details` deviendrait une seconde API, contre ADR-0101, décision 3. |
| Une clé `details.index` en multi-index | Doublon du message, qui nomme déjà l'index fautif (ADR-0089, décision 4) ; ce serait une clé de plus à documenter pour une information déjà présente. Décision séparée si Legifacile la demande. |
| Une `reason` libre, remplie par chaque route | Seconde API non documentée : c'est exactement l'alternative qu'ADR-0101 a écartée (`:74`). Le vocabulaire est fermé et documenté. |
| Mettre la valeur `lexical_branch_disabled` dans `message` plutôt que dans `details` | Le message est déjà lu par des humains et reformulable ; un client ne doit pas dépendre d'une prose. C'est le problème que le ticket 15 signale. |
| Renvoyer le même `details` sur le mode inconnu | Le mode inconnu n'a pas de `reason` à publier : il est déjà localisé par son message, et lui donner un discriminant fermé serait une décision séparée. |

## Références

- Ticket : `tickets-crible/15-code-stable-pour-le-refus-de-mode.md` (projet Legifacile, lecture seule), constat, reproduction et impact sur `mode_refused` du serveur MCP.
- Conception : `docs/05-api.md:35` (règle de `details`), `:39` (ligne `invalid_request`), `:160` (défaut de `mode`), `:393` (mode par défaut multi-index) ; `docs/03-recherche.md:98` (branche désactivée).
- Code : `crates/crible-core/src/error.rs:105-132` (`CribleError`, `with_path`), `crates/crible-core/src/filter.rs:396` (unique appelant), `crates/crible-server/src/error.rs:140-143` (rendu de `details`), `crates/crible-server/src/search.rs:204-212` (refus simple) et `:1704-1714` (`parse_mode`), `crates/crible-server/src/search_multi.rs:585-591` (refus multi-index), `crates/crible-search/src/lexical.rs:418-427` (refus de la bibliothèque), `crates/crible/src/commands.rs:772` (message de la CLI), `crates/crible-server/src/openapi.rs:76-82` (`ErrorBody.details`).
- Suivi : `docs/suivi.md:176` (tâche `T15-CODE-REFUS-LEXICAL`, classe « ajout compatible »), section « Écarts à la conception » (ligne à ajouter par l'orchestrateur).
- ADR : ADR-0089 (branche lexicale désactivée, code et messages), ADR-0101 (corps d'erreur, décision 3 sur `details`, décision 4 sur la journalisation), ADR-0110 (recherche multi-index, mode par défaut), ADR-0112 (client Python, `details` rendu tel quel), ADR-0143 (version sémantique, décisions 1, 4 et 7), ADR-0144 (méthodes récentes du client Python, sans effet ici).
