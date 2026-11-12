# ADR-0156 — Temps par étape d'un mode lisibles sans debug par `GET /v1/stats`

- **Statut** : Proposé
- **Date** : 2026-11-12
- **Décideurs** : propriétaire
- **Proposé par** : agent `@greffier`, tâche `T17-TEMPS-PAR-ETAPE-SANS-DEBUG` (v1.1), sur le verdict de `@architecte` du 2026-11-12 et le point 2 du ticket 17 de Legifacile
- **Remplace** : — (remplace en partie ADR-0124, **décision 3 seulement** : le filtre `mode` s'ajoute à `index` et `operation`. Les autres décisions d'ADR-0124 ne changent pas.)

## Contexte

### Le besoin, et pourquoi `debug` ne le couvre pas

Le point 2 du ticket 17 de Legifacile (`../legifacile/tickets-crible/17-latence-hybride-et-temps-par-etape.md`) demande un temps par étape lisible **sans `debug`** en production. Il accepte deux formes : un membre de corps qui ne rend que `timings_ms`, ou « une métrique équivalente dans `GET /v1/stats`, agrégée par index, sans contenu ». Tous les numéros d'ADR du présent document sont des numéros **de Crible** ; le seul document de Legifacile cité est son ticket 17.

Legifacile refuse `debug: true` comme mesure permanente, et sa raison est écrite : `debug` ajoute `plan` et `per_result`, et le journal de debug, s'il était activé, écrirait le texte des questions (ADR-0122, décision 2). Or INV-01 de Legifacile interdit qu'un texte de question entre dans un journal persistant de ce projet, et le journal de Crible n'est pas sous son contrôle.

### Ce que la conception fournit déjà

Les analytics agrègent déjà, **par minute × index × opération × statut × client × mode × stratégie dense**, un histogramme de latence pour chacune des neuf étapes (`docs/06-observabilite.md:19-21` ; ADR-0117, décisions 1 et 2). Le mode est donc une **dimension stockée**, pas une information à ajouter.

`GET /v1/stats` rend, par seau et sur la fenêtre, les p50, p95 et p99 de chaque étape qui a au moins un échantillon (ADR-0124, décision 4 ; ADR-0117, décision 4). Les seaux sont géométriques, de facteur 1,25 : la résolution est de 25 % relatifs, et la valeur rendue est la borne haute du seau (`docs/06-observabilite.md:51-54`).

### Le trou, constaté par `@architecte`

ADR-0124, décision 3, fait de `index` et `operation` les **seuls** filtres de la route (`docs/adr/0124-route-stats-forme-parametres-seaux-et-cli.md:77` ; `docs/05-api.md:689`). `mode` reste une dimension agrégée à la lecture : sur une instance qui sert du dense et de l'hybride, un hybride ne se lit pas à part d'un dense, et une étape dominante ne peut pas être attribuée au mode servi.

Le verdict de `@architecte` du 2026-11-12, vérifié par lui dans le code, est **`NON-COUVERT`** pour la voie permanente. La classe de compatibilité est celle d'ADR-0143, décision 1 : un **ajout compatible**, où un champ de requête **optionnel** dont l'absence garde le comportement précédent est permis en version mineure (`docs/adr/0143-politique-de-version-de-la-ligne-1-x-et-procedure-de-montee-et-de-retour-arriere.md:51-60`).

### Ce que le code fait, et que la conception n'écrit pas

Dans une recherche multi-index, l'encodage est mesuré **une fois** pour la requête (`crates/crible-server/src/search_multi.rs:767-781`), et son échantillon est porté par l'événement de la **première cible** (`:800-819`) ; le commentaire de `crates/crible-server/src/search.rs:541-545` énonce la règle, et `crates/crible-server/src/search_multi.rs:1108-1123` laisse `encode` absent des durées d'une cible. Cette règle n'est écrite ni dans `docs/06` ni dans ADR-0117.

### Ce qui n'est pas mesuré ici

Le coût de lecture ajouté par un filtre d'égalité sur une colonne déjà présente dans la clé primaire de `minute_events` n'est pas mesuré. La table est indexée par `(minute, index, operation, status, client, mode, dense_strategy)` (`crates/crible-store/src/analytics.rs:867-892`), et le filtre reste une condition SQL sur une colonne indexée. Aucun chiffre n'est avancé.

## Décision

**`GET /v1/stats` reçoit un paramètre facultatif `mode`, de domaine fermé `dense`, `lexical` et `hybrid`, qui restreint les seaux et les totaux aux lignes de ce mode ; son absence garde exactement le comportement actuel, tous modes agrégés.**

### 1. Le paramètre `mode` de la route

- **Domaine fermé** : `dense`, `lexical`, `hybrid`, les trois valeurs que le mode d'une recherche prend (`crates/crible-search/src/pipeline.rs:101-107`) et que la colonne `mode` de `minute_events` stocke.
- **Absence** : aucune condition sur `mode`. La réponse est **identique** à celle d'avant le changement, y compris pour les lignes dont `mode` est vide.
- **Présence** : une condition d'égalité sur `minute_events.mode`. Les lignes dont `mode` est vide — une requête refusée avant la résolution du mode (ADR-0124, décision 10) — sont alors **exclues**, comme toute ligne d'un autre mode. C'est le sens d'un filtre d'égalité, et la réponse ne invente pas de mode pour ces lignes.
- **Valeur inconnue** : `400 invalid_request`, avec un message qui **nomme le paramètre**, sur la forme qu'ADR-0124, décision 2, fixe pour `group_by` et `operation` (`'group_by' must be 'minute', 'hour' or 'day': '<valeur>'`, et `'operation' is not one of …: '<valeur>'`). Le message retenu est `'mode' must be 'dense', 'lexical' or 'hybrid': '<valeur>'`. Le message du paramètre inconnu énumère désormais les six noms (`crates/crible-server/src/http.rs:497-514`).
- **Portée** : ce filtre **remplace en partie ADR-0124, décision 3**. `status`, `client` et `dense_strategy` restent **non filtrables** : ils ne sont pas demandés, et les rendre filtrables serait un autre besoin.

### 2. Les membres que le filtre restreint, et ceux qu'il ne touche pas

Le filtre s'applique aux lectures de `minute_events` et **pas** aux tables qui n'ont pas de colonne `mode`. Le schéma tranche (`crates/crible-store/src/analytics.rs:860-917`) :

- **Restreints** : `buckets` et `totals`, donc `count`, `errors`, `zero_results_rate` et tous les `latency_ms` ; et `top_indexes`. Ces membres lisent `minute_events`, qui porte `mode`. Pour une opération qui n'est pas une recherche, la ligne a un `mode` vide : un filtre explicite l'exclut, et `count` ne compte alors que les recherches du mode demandé.
- **Non touchés** : `top_filter_shapes`, parce que la table `filter_shapes(minute, index, shape, count)` n'a **pas** de colonne `mode` et que seules les recherches y écrivent une forme (ADR-0124, note du 2026-11-01, `docs/adr/0124-…md:249-255`) ; `indexes`, parce que `day_indexes` n'a pas de colonne `mode` et que l'état journalier est celui d'un index, pas d'un mode ; `analytics_dropped`, parce que c'est un compteur du processus, jamais persisté (ADR-0118, décision 2) ; les échos `from`, `to` et `group_by`, qui décrivent la fenêtre et la largeur, pas un mode.

Restreindre `top_filter_shapes` ou `indexes` par `mode` demanderait une colonne nouvelle, donc un changement de schéma et un ADR distinct. Le présent ADR ne le fait pas.

- **Aucun membre de réponse nouveau, aucune table nouvelle, aucun `format_version`** : les histogrammes sont déjà écrits, et la réponse reste l'objet à membres fixes d'ADR-0124, décision 1. La spécification OpenAPI gagne le paramètre, pas un composant de réponse.

### 3. Le même filtre là où la route a un équivalent

- **`crible stats`** existe (ADR-0124, décision 13) et expose aujourd'hui `--data`, `--from`, `--to`, `--index`, `--operation`, `--group-by` et `--format` (`crates/crible/src/main.rs:282-309` ; `crates/crible/src/commands.rs:1339-1354`). Il reçoit `--mode`, avec la **même sémantique** et la **même forme d'erreur** que la route : la vérification passe par le même `StatsFilter::parse` (`crates/crible/src/commands.rs:1374-1385`). La commande et la route restent comparables.
- **Le client Python n'a pas de méthode `stats`** : `clients/python/src/crible/client.py` couvre la recherche, les facettes, les documents, les lots, les jobs et l'administration des index, mais pas les statistiques. Il n'y a donc rien à changer dans le client. Si une méthode `stats` est ajoutée plus tard, elle devra exposer `mode` ; c'est une conséquence à tenir, pas une décision de cet ADR.
- **OpenAPI, `docs/05` et un exemple dans le même changement** (ADR-0111, complément du 2026-10-30) : le paramètre entre dans les `params` de la route (`crates/crible-server/src/http.rs:444-450`), dans le tableau des paramètres de `docs/05-api.md:617-623`, et un exemple d'appel `GET /v1/stats?mode=hybrid` avec sa réponse est ajouté à la section « Statistiques ». Aucun exemple existant n'est réécrit.

### 4. Règle d'attribution de l'encodage en multi-index, écrite

Dans une recherche multi-index, l'encodage est mesuré **une fois** pour la requête, et son temps est porté par l'événement de la **première cible**. Les autres cibles n'ont **aucun échantillon** d'encodage ; leur `total` reste celui de leur propre travail, du filtre à la lecture de leurs payloads.

C'est le comportement actuel du code (`crates/crible-server/src/search_multi.rs:767-781`, `:800-819` ; commentaire de `crates/crible-server/src/search.rs:541-545`), et il est cohérent avec ADR-0119, décision 9, qui met `encode` dans le `timings_ms` global et non dans celui d'une cible. **Aucun changement de code** : l'ADR constate la règle, et l'application l'écrit dans `docs/06`, section « Analytics ».

Conséquence de lecture : avec `?index=<deuxième cible>`, l'échantillon d'encodage de cette requête n'apparaît pas ; si cette cible n'est jamais la première, aucun percentile d'encodage n'apparaît pour elle. L'encodage se lit sur la première cible ou sans filtre `index`. Ce n'est pas une perte : c'est la vérité de la mesure, et sommer l'encodage par cible le compterait plusieurs fois.

### 5. Résolution des percentiles, et lecture fine

Les percentiles de `/v1/stats` ont une résolution de **25 % relatifs** à la borne haute du seau (facteur 1,25 ; `docs/06-observabilite.md:51-54`). Ils nomment une **étape dominante** quand l'écart entre étapes dépasse un seau ; ils ne séparent pas deux étapes proches. La lecture fine reste `debug: true` **hors production**, ou `crible eval` (ADR-0125), qui mesure par requête.

### 6. Tests attendus

`@testeur` puis `@dev` couvrent, au minimum :

1. deux modes dans la même fenêtre rendent des percentiles **distincts** sous `mode=dense` et sous `mode=hybrid`, et la réponse sans `mode` les agrège ;
2. sans `mode`, la réponse est **identique** à celle d'avant le changement, à fenêtre et trafic égaux ;
3. une valeur inconnue de `mode` rend `400 invalid_request` et **nomme** le paramètre ;
4. `INV-13` : aucun texte de requête ni valeur de filtre dans la réponse, ni dans `analytics.sqlite` (test nommé `inv_13_…`) ;
5. aucun changement de résultat de recherche : mêmes résultats, même ordre, même `took_ms` (précédent d'ADR-0119, décision 10) ;
6. la route et `crible stats --mode` rendent le même corps, moins `analytics_dropped` (ADR-0124, décision 13) ;
7. en multi-index, l'encodage ne produit **qu'un** échantillon, sur la première cible.

### 7. Ce que l'acceptation appliquera

| Document | Modification |
|---|---|
| `docs/05-api.md:617-623` | ligne du paramètre `mode` dans le tableau de la section « Statistiques », domaine fermé et absence = tous modes |
| `docs/05-api.md:689` | la phrase « ni `status`, ni `client`, ni `mode` » nomme `mode` comme filtre offert, les deux autres restant refusés |
| `docs/05-api.md`, section « Statistiques » | exemple d'appel `?mode=hybrid` et sa réponse |
| `docs/06-observabilite.md:72-80` | la liste des paramètres de la route porte `mode`, et la section « Lecture » dit ce que le filtre restreint et ce qu'il ne touche pas |
| `docs/06-observabilite.md`, section « Analytics » | règle d'attribution de l'encodage en multi-index : mesuré une fois, porté par l'événement de la première cible |
| `docs/06-observabilite.md:51-54` | rappel que la résolution de 25 % nomme une étape dominante et ne sépare pas deux étapes proches |
| `docs/adr/README.md:179` | ligne d'ADR-0124 : « décision 3 remplacée en partie par ADR-0156 le 2026-11-12 », sous la forme que l'index emploie déjà |
| `docs/versions/1.1.0.md` | note de version laissée à `PUBLIER-V1.1.0` : paramètre `mode`, ajout compatible, aucun `format_version` |

Aucun de ces documents n'est modifié avant l'acceptation. La spécification OpenAPI est **du code** : elle est changée par la tâche, dans le même changement que la route (ADR-0111).

### Ce que le présent ADR ne fait pas

- Il **n'expose pas** les compteurs du cache des requêtes. `CacheStats { hits, misses, entries, capacity }` existe (`crates/crible-encode/src/query.rs:54-66`) et n'est lu que par des tests (`crates/crible-encode/tests/encoder_requetes.rs`) et un exemple (`crates/crible-encode/examples/measure_queries.rs`). L'état chaud ou froid est rapporté par la **campagne de mesure**, qui compte la première requête à part : c'est la variante que le point 3 du ticket accepte.
- Il **n'ajoute pas** de membre `timings` à la recherche.
- Il **ne corrige pas** la latence : elle dépend de la campagne, et aucune correction ne se décide sans la lecture par étape.

## Conséquences

- **Positives** :
  - un hybride se lit à part d'un dense sur une instance qui sert les deux, sans `debug` et sans journal ;
  - aucune écriture nouvelle : les histogrammes et la colonne `mode` existent déjà, et aucun `format_version` ne bouge ;
  - la CLI et la route gardent la même sémantique, et l'absence du paramètre préserve les clients 1.0 ;
  - la règle d'attribution de l'encodage en multi-index cesse d'être un comportement de code non écrit.
- **Négatives / coûts** :
  - un paramètre de plus dans une liste fermée, donc un message d'erreur et une spécification à tenir ;
  - `top_filter_shapes` et `indexes` ne suivent pas le filtre : une lecture par mode ne dit rien des formes de filtre ni de l'état journalier de ce mode, et la réponse mêle alors deux portées ;
  - la résolution de 25 % ne sépare pas deux étapes proches, même avec le filtre ;
  - avec `?index=<deuxième cible>`, l'échantillon d'encodage d'une requête n'apparaît pas sur cette cible, par construction ;
  - le coût de lecture du filtre n'est pas mesuré.
- **À surveiller** :
  - si une lecture par mode des formes de filtre devient nécessaire, il faut une colonne `mode` dans `filter_shapes`, donc un changement de schéma et un ADR distinct ;
  - si `status`, `client` ou `dense_strategy` deviennent nécessaires, c'est un autre ADR, et la cardinalité de `minute_events` reste la contrainte ;
  - si la campagne montre que le p95 vient du cache d'encodage, l'exposition permanente de `CacheStats` demandera un ADR, la réponse d'ADR-0124 étant fermée ;
  - si un lecteur interprète l'absence d'`encode` sur une cible comme une mesure nulle, la règle de la décision 3 doit être rappelée dans `docs/05`.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Un membre `timings: true` du corps de recherche (piste 1 du ticket) | Il change le contrat de requête et de réponse, remplace en partie ADR-0119, décision 9, et demande OpenAPI, validation, tests et rétrocompatibilité pour un besoin que les analytics couvrent déjà. Il rend en plus les temps par requête, alors que la lecture demandée est agrégée par index. |
| Documenter seulement `debug: true` avec le journal désactivé | Le ticket l'accepte en repli, mais il ne règle pas la mesure permanente : `plan` et `per_result` alourdissent la réponse, et le journal reste un risque dès qu'il est activé. |
| Exposer `CacheStats` dans `/v1/stats` | C'est un membre nouveau d'une réponse fermée (ADR-0124, décision 1), et il n'est pas nécessaire si la campagne compte la première requête à part (point 3 du ticket). |
| Ventiler tous les membres par mode dans une même réponse | C'est une forme nouvelle, plus lourde qu'un filtre : elle change la forme de chaque membre et oblige chaque client à choisir une clé de plus, alors qu'un paramètre absent garde la réponse actuelle. |
| Rendre aussi `status`, `client` ou `dense_strategy` filtrables | Hors besoin : le ticket demande le mode, et chaque dimension de plus augmente la surface de la route et de la CLI sans lecture demandée. |

## Références

- Ticket 17 de Legifacile, point 2 et point 3 : `../legifacile/tickets-crible/17-latence-hybride-et-temps-par-etape.md`.
- ADR-0117 (analytics : schéma, histogrammes, percentile, formes de filtre), décisions 1, 2, 4 et 5.
- ADR-0124 (route `GET /v1/stats`), décisions 1, 2, 3, 4, 10 et 13, et sa note du 2026-11-01 sur `top_filter_shapes`.
- ADR-0118, décision 2 (`analytics_dropped` par processus).
- ADR-0122, décision 9 (forme d'erreur d'un paramètre de lecture) ; ADR-0111, complément du 2026-10-30 (route, spécification et exception dans le même changement).
- ADR-0119, décisions 9 et 10 ; ADR-0125 (`crible eval`).
- ADR-0143, décision 1 (ajout compatible en version mineure).
- ADR-0148 et ADR-0149, pour la forme du tableau d'application et du remplacement partiel d'une décision acceptée.
- `docs/05-api.md:613-689` (section « Statistiques ») ; `docs/06-observabilite.md:15-90` (sections « Analytics » et « Lecture »).
- Code : `crates/crible-server/src/http.rs:444-514`, `crates/crible-store/src/analytics.rs:860-917` et `:1186-1270`, `crates/crible/src/commands.rs:1339-1385`, `crates/crible/src/main.rs:282-309`, `crates/crible-server/src/search_multi.rs:767-819` et `:1108-1123`, `crates/crible-server/src/search.rs:535-545`, `crates/crible-encode/src/query.rs:54-66`, `clients/python/src/crible/client.py`.
