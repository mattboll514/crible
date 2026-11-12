# ADR-0154 — Route de facettes et comptes de passages et de documents sous filtre

- **Statut** : Accepté (accepté par anticipation par le propriétaire le 2026-11-12)
- **Date** : 2026-11-12
- **Décideurs** : propriétaire
- **Proposé par** : agent `@greffier`, tâche `T12-FACETTES-ET-COMPTES` (v1.1), sur demande de l'orchestrateur et sur le verdict de `@architecte` du 2026-11-12
- **Remplace** : — (complète ADR-0102 pour le sémaphore et le délai, ADR-0146 pour la liste des routes refusées au démarrage, et la liste fermée des opérations de `docs/06-observabilite.md`)
- **Décision** : acceptation par anticipation par le propriétaire le 2026-11-12, sa réponse : « si tu as des adr a ecrire considiere qu'il sont accpeté par anticipation on les regarde ensemble apres les 2 tickets ».

## Contexte

### Le ticket, et ce qu'il constate

Le ticket 12 de Legifacile (`/Users/mattboll/MattCorp/legifacile/tickets-crible/12-facettes-valeurs-frequentes.md`, lu en lecture seule ; c'est un autre projet, il n'est pas modifié par cet ADR) porte sur Crible 1.0.0. Il constate qu'aucune route, aucune commande et aucune opération OpenAPI ne rendent les valeurs les plus fréquentes d'un champ filtrable, ni un compte de passages ou de documents sous un filtre (`:11-14`). Trois choses existent pourtant et bornent le besoin :

1. **Les comptes d'index entier.** `GET /v1/indexes/{index}` rend `documents` et `live_passages` (`docs/05-api.md:64`, `:102-130`). C'est exact pour un index, sans filtre.
2. **Le compte de passages filtrés, en passant.** Toute réponse de recherche porte `pool_size`, la taille du pool après filtre (`docs/05-api.md:349` ; `crates/crible-server/src/search.rs:1073`, `let pool_size = pool.len();`). C'est un compte exact de passages vivants filtrés, mais il coûte une recherche complète et exige une `query` ou un `vector`.
3. **Les briques internes du comptage.** `FilterIndex` porte la bitmap des vivants (`crates/crible-store/src/filters_index.rs:1002`), et `KeywordField::values()` expose chaque valeur distincte avec sa bitmap (`:429`). Les champs `integer`, `float` et `date` passent par une `ValueColumn` (`:545`), donc par un parcours de colonne, pas par un bitmap.

Le besoin est celui d'ADR-0033 de **Legifacile** (`/Users/mattboll/MattCorp/legifacile/docs/adr/0033-contenu-de-corpus-disponibles-sans-le-stockage-vectoriel.md`), décisions 2 et 3 : `corpus_disponibles` doit rendre `n_passages` et `n_documents` par corpus, et `valeurs_frequentes` reste retirée tant que Crible ne sait pas rendre de valeurs fréquentes. Un corpus de Legifacile n'est pas un index : quatre corpus partagent un index avec un autre, `textes_vigueur` et `textes_hors_vigueur` dans `articles`, `juri_sommaires` et `juri_texte_integral` dans `decisions` (ADR-0033 de Legifacile, `:93`). Un corpus est donc un **filtre sur un index partagé**, et un total d'index présenté comme un total de corpus serait faux.

### Ce que la conception fixe déjà

- **ADR-0111, décision 5** : toute route ajoutée au code est documentée dans le tableau de `docs/05` **dans le même changement**, et son opération OpenAPI avec un exemple validé (`docs/adr/0111-specification-openapi-et-validation-des-exemples.md:117`). Cet ADR ne demande **aucune commande CLI** : le ticket écrit à tort que le point 4 de son constat exige une commande CLI ; ADR-0111, décision 5, exige le tableau des routes, la spécification et la validation des exemples, rien de plus.
- **ADR-0143, décision 1** : une mineure n'apporte que des ajouts compatibles ; une route nouvelle et un membre de réponse nouveau sont permis (`:55`), et changer le sens d'une valeur ou un `format_version` est interdit (`:60`). Aucun code d'erreur nouveau n'est donc nécessaire.
- **ADR-0137** : le schéma ne connaît qu'un `keyword` scalaire, et les listes s'expriment par `in` et `nin` (`docs/adr/0137-aucun-champ-multivalue-en-v1.md:15`). Une facette porte donc sur une valeur scalaire, jamais sur une liste.
- **ADR-0119, décision 3** : dans un objet fermé, un membre sans valeur est **omis**, jamais rendu `null` ni `0` (`docs/adr/0119-objet-debug-de-la-reponse-de-recherche-forme-rangs-temps-et-capture.md:61-63`). C'est la convention d'omission que le repli de la décision 11 utilise.
- **ADR-0102, décisions 2 et 3** : un sémaphore de `[server].max_concurrent_searches` jetons, pris sans attente, rend `503 overloaded` immédiat ; à l'échéance de `timeout_ms`, `504 timeout`, et le jeton reste tenu jusqu'à la fin réelle du calcul (`:41`, `:47-49`). Sa décision 2 dit que la limite ne s'applique **qu'aux routes de recherche** ; cet ADR l'étend à la route de facettes, sans réécrire le corps d'ADR-0102.
- **ADR-0146** : tant que le registre n'est pas chargé, les quatorze routes du registre et les quatre routes de l'état publié rendent `503 overloaded` avec le message fixe « the server is starting » (`docs/adr/0146-routes-de-l-api-pendant-le-chargement-au-demarrage.md:69-73`). La route nouvelle rejoint cette liste.
- **ADR-0147** : `[limits].default_timeout_ms` est borné de 1 à 30 000 ms, les bornes du `timeout_ms` d'une requête (`docs/adr/0147-domaine-de-default-timeout-ms.md:43`), et `docs/05` donne `timeout_ms` de 1 à 30 000 ms avec 2 000 ms par défaut (`:318`, `:654`).
- **ADR-0142** : l'attache complète d'un grand index laisse 0,58 Gio de marge mémoire sur la VM de 11,67 Gio (`docs/adr/0142-limites-acceptees-de-la-v1-reconstruction-lexicale-et-memoire-d-un-replace-all.md:34-36`). Toute structure en RAM nouvelle se compte contre cette marge.
- **ADR-0117, décision 5** : `OpEvent` a une liste **fermée** de champs, sans emplacement pour un texte, une valeur de filtre, un hash ou un id de résultat (`docs/adr/0117-analytics-stockage-et-calcul-schema-histogrammes-percentile-et-formes-de-filtre.md:167-176`), et la liste des opérations comptées est celle de `docs/06-observabilite.md:32`.

### Ce que le code permet, vérifié le 2026-11-12

- **La bitmap des vivants est dans l'instantané publié.** `Snapshot` porte `filters: FilterIndex` (`crates/crible-store/src/generation.rs:313-344`), et `live_passages()` vaut `self.filters.live().len()` (`:385`). Le pool d'une requête est déjà l'intersection du filtre avec cette bitmap (`docs/03-recherche.md:35`), et sa taille est `pool_size` (`crates/crible-server/src/search.rs:1073`).
- **Chaque valeur `keyword` a sa bitmap.** `KeywordField::values()` rend les valeurs distinctes **en ordre croissant**, avec leur bitmap (`crates/crible-store/src/filters_index.rs:427-435`). Compter les passages d'une valeur sous un filtre est donc une intersection de bitmaps suivie de `.len()`.
- **Les numériques et les dates n'ont pas de bitmap.** `ValueColumn` est une colonne indexée par iid, avec une sentinelle pour null (`filters_index.rs:540-560`), et `FilterIndex::column` la rend (`:1023`). Un comptage par valeur y serait un parcours de colonne ou un histogramme, pas une intersection.
- **Aucun lien iid → document n'est gardé en mémoire.** `Snapshot` porte la génération, les comptes de documents et de passages, les segments, les codes, les filtres, le lecteur lexical et le bail d'époque (`generation.rs:313-344`) ; il ne porte **aucune** correspondance entre un iid et un `doc_id`. `state.documents` vient du compte de lignes `documents` enregistré à la construction de l'instantané (`:2593`, `:369`), pas d'un parcours des passages vivants.
- **Le chemin SQL existe, mais par iid.** Un comptage de documents distincts depuis `records.sqlite` demanderait une lecture par iid, ou un balayage complet ; l'instantané ne porte justement pas de transaction SQLite (`docs/adr/0043-payload-d-un-resultat-dont-le-document-est-supprime.md:18-20`).
- **Le sémaphore est déjà branché.** `SearchLimit::new` construit le sémaphore de `[server].max_concurrent_searches` (`crates/crible-server/src/limits.rs:103-108`) et le prend par `try_acquire_owned` (`:148`) ; la clé est lue par la configuration (`crates/crible-server/src/config.rs:167`).
- **Le message de démarrage est une constante.** `STARTING_MESSAGE` vaut « the server is starting » (`crates/crible-server/src/error.rs:70`), et le refus naît de l'accès au registre, dans le gestionnaire de chaque route (ADR-0146, décision 1).
- **Le client Python n'envoie que les options non nulles.** Le corps est construit par `{key: value for key, value in options.items() if value is not None}` (`clients/python/src/crible/client.py:451`), et les méthodes rendent le JSON en `dict` (ADR-0112, décision 3, `docs/adr/0112-client-python-http.md:63`). `snapshot` et `delete_index` sont le précédent d'une méthode ajoutée pour une route existante (ADR-0144, `:29-38`).

### Le point de bascule

Un client de Legifacile ne peut pas répondre à `corpus_disponibles` avec ce qui existe. Les comptes d'index sont faux pour les quatre corpus partagés, et `pool_size` exige une recherche complète avec une `query` factice, donc un encodage, une latence de recherche et un résultat dont personne ne veut. Aucune route ne rend de valeurs fréquentes.

Le compte de **documents** distincts est le point cher : une bitmap de passages ne dit pas combien de `doc_id` distincts elle couvre, et l'instantané ne garde aucun lien iid → document. La réponse doit donc ou payer une lecture SQL par iid, ou construire une structure dédiée. Cette structure coûte de la mémoire, et la marge de l'attache est de 0,58 Gio (ADR-0142) : le prix se mesure avant de promettre le membre.

## Décision

**Une route nouvelle, `POST /v1/indexes/{index}/facets`, rend en un appel les valeurs les plus fréquentes de un à dix champs `keyword` filtrables et les comptes de passages et de documents du pool filtré, sur un seul index et sous la grammaire de filtre existante.** Le compte de documents s'appuie sur une table dérivée en mémoire, iid → ordinal de document, construite avec l'index de filtres ; une mesure fixée d'avance, MES-18, peut le retirer au profit d'un nouvel ADR.

### 1. La route

`POST /v1/indexes/{index}/facets`, rôle `read`. Un seul index : il n'existe **aucune** forme multi-index, et `POST /v1/search` ne change pas. La route prend une ligne dans le tableau des routes de `docs/05` (`:60-78`) et une opération OpenAPI avec un exemple validé par le test `jsonschema` (ADR-0111, décision 5). `POST` est le verbe, comme pour la recherche : le corps porte la liste des champs et le filtre, et une route `GET` devrait encoder un filtre JSON dans l'URL.

### 2. Le corps

Le corps est un **objet fermé** : une clé inconnue rend `400 invalid_request`, comme pour la recherche (ADR-0109, `docs/05-api.md:35`). Ses membres sont :

| Membre | Règle |
|---|---|
| `fields` | **obligatoire**, de 1 à 10 noms **distincts** ; absent, vide, dupliqué, non textuel ou au-delà de dix rend `400 invalid_request`, et le message nomme le champ fautif |
| `filter` | facultatif ; grammaire, validation et codes existants (`invalid_filter`, `400`) ; absent ou vide vaut « aucun filtre » |
| `limit` | facultatif, défaut **20**, bornes **1 à 100** ; hors bornes, `400 invalid_request` |
| `timeout_ms` | facultatif, même domaine et même défaut que la recherche : 1 à 30 000 ms, défaut `[limits].default_timeout_ms` (ADR-0102, décision 3 ; ADR-0147) |

`limit` et ses bornes sont des **constantes du code**, sans clé de configuration : c'est une valeur fixe, et la règle d'or 12 interdit une configuration pour une valeur fixe. Le ticket laissait la borne haute « à décider » (`:84-85`) ; elle vaut 100, ce qui borne la réponse à 10 × 100 valeurs.

Le corps ne porte **ni** `query`, **ni** `vector`, **ni** `mode`, **ni** `k`, **ni** `return`. Une facette ne fait aucune recherche : elle ne classe rien et n'encode rien. Une clé de ces noms est une clé inconnue, donc `400 invalid_request`.

### 3. Les champs acceptés en 1.1

Un champ de `fields` est accepté s'il est **déclaré** par le schéma de l'index, marqué **`filterable`**, et de type **`keyword`**. Un champ inconnu, non filtrable, ou d'un autre type (`integer`, `float`, `date`, `bool`) rend `400 invalid_request`, avec un message qui **nomme le champ** et, quand son type est connu, **son type** ; `details` vaut `{}` (convention de `docs/05-api.md:35` ; aucune `reason` nouvelle n'est inventée).

`invalid_filter` reste réservé à la grammaire des filtres, avec `details.path = "$.filter.<champ>"` (`docs/05-api.md:35`, ADR-0101, décision 3). Le refus d'un champ de facette n'est pas un filtre invalide : c'est une demande invalide, et les deux codes ne se mélangent pas.

Les champs numériques et de date sont **hors de la 1.1**. Une facette exacte par valeur ou un histogramme demanderait un parcours de colonne (`ValueColumn`, pas de bitmap), un choix de tranches et une mesure ; un nouvel ADR les ouvrira. `annee`, que le ticket cite comme exemple, n'est pas nécessaire à ADR-0033 de Legifacile pour `valeurs_frequentes` (décision 3 du ticket, `:72-74`). `bool` est écarté pour la même raison de périmètre : deux valeurs possibles n'apportent pas une facette utile, et l'ouvrir élargirait la liste fermée sans besoin.

### 4. La réponse

```json
{ "generation": 1287,
  "count": { "passages": 1819006, "documents": 152864 },
  "fields": {
    "fonds": { "values": [ { "value": "LEGI", "count": 1819006 } ], "other": 0 },
    "etat":  { "values": [ { "value": "VIGUEUR", "count": 812004 },
                           { "value": "ABROGE", "count": 1200 } ], "other": 1005802 } } }
```

- **`generation`** est celle de l'instantané publié qui a servi toute la requête (décision 6).
- **`count.passages`** est la taille du pool (décision 5). **`count.documents`** est le nombre de `doc_id` distincts des passages du pool, exact (décision 5). Sous le repli de la décision 11, `documents` est **omis** et `count` ne porte que `passages` (ADR-0119, décision 3).
- **`fields`** porte exactement les champs demandés, chacun sous la forme `{ "values": [{ "value", "count" }], "other" }`. Les `count` de valeurs sont des **passages du pool**, pas des documents.
- **Ordre** : `count` décroissant, puis `value` croissant en **octets UTF-8**. C'est un ordre total, donc la réponse est reproductible. `KeywordField::values()` rend déjà les valeurs en ordre croissant (`filters_index.rs:427`) ; le tri final ne départage que les comptes égaux.
- **`other` est toujours présent.** Il vaut les passages du pool qui portent une valeur pour ce champ, **moins la somme des `count` rendus**. Un passage sans valeur (null) n'entre ni dans `values` ni dans `other`, donc `Σ count + other ≤ count.passages`, avec égalité seulement si chaque passage du pool porte une valeur. `other` vaut `0` quand tout tient dans `limit`.
- **Niveau du champ** : un champ de document compte les passages vivants de ce document **qui sont dans le pool**, pas un par document ; un champ de passage compte ses propres passages. Dans les deux cas, l'intersection de la bitmap de valeur avec le pool donne le compte, puisque les champs de document sont matérialisés au niveau du passage (`docs/03-recherche.md:36`).
- **Pool vide** : `200`, `count` à zéro (`documents: 0` quand il est présent), `values: []`, `other: 0`. Un pool vide n'est jamais une erreur.

L'exemple ci-dessus suppose un filtre `{"fonds":"LEGI"}` : tous les passages du pool portent donc `fonds`, et `other` vaut `0` pour ce champ. Pour `etat`, il suppose que chaque passage du pool porte une valeur, dont 1 005 802 autres que les deux rendues.

### 5. Les deux comptes

**`count.passages`** est la taille du pool, le **même pool** qu'une recherche de même `filter` (INV-07, `docs/08-invariants-et-tests.md:15`) : filtre évalué en bitmaps, intersecté avec les vivants. Sans filtre, il égale `state.live_passages` (`generation.rs:385`, `docs/05-api.md:64`).

**`count.documents`** est le nombre de `doc_id` distincts des passages du pool, **exact**. Il peut différer de `state.documents` sans filtre, et l'ADR ne promet pas l'égalité : les deux nombres ont deux sources. `state.documents` est le compte de lignes `documents` enregistré à la construction de l'instantané (`generation.rs:369`, `:2593`), alors que `count.documents` compte des `doc_id` distincts parmi les passages **vivants**. ADR-0041 retire la ligne `documents` d'un document supprimé dans la transaction du commit (`:46-50`), et ADR-0043 décrit un passage vivant dont la ligne de document peut manquer dans l'état SQLite courant (`:18-20`) ; les deux documents montrent que la ligne d'un document et ses passages vivants ne sont pas une seule et même chose. Le contrat porte donc sur le pool, et le compte d'index garde son sens.

Le compte est calculé par une **table dérivée en mémoire** : un tableau dense qui relie chaque **iid** à un **ordinal de document**, les ordinaux étant attribués une fois par `doc_id` distinct. Compter les documents du pool revient alors à compter les ordinaux distincts des iid du pool, sans lire `records.sqlite` par iid. Le compte d'un appel se fait par un parcours des iid du pool et une bitmap d'ordinaux, dont la taille est celle du nombre de documents de l'index : moins d'un Mio pour 6,6 M documents, et non une structure proportionnelle au pool.

- **Construction et tenue** : la table est construite avec l'**index de filtres**, à l'ouverture (`FilterIndex::from_records`, `filters_index.rs:914`), et tenue avec lui pour **chaque génération qui change des passages ou des documents** : une telle génération la met à jour à partir du delta du commit, comme les bitmaps (`filters_index.rs:973` ; `docs/02-architecture.md:131`). Elle vit dans l'instantané, à côté de `filters` (`generation.rs:328`). Sa mise à jour coûte une copie de sa taille à chaque génération qui la touche, coût d'écriture qui n'est pas mesuré par MES-18 et qui reste à surveiller.
- **Aucun fichier** : elle n'est jamais écrite, donc aucun `format_version`, aucune migration, aucune entrée dans un instantané ou un export.
- **Mémoire comptée avec celle des filtres** : elle entre dans `filter_memory_bytes`, aux côtés de `FilterIndex::memory` (`filters_index.rs:1029`), et se lit dans `state.sizes` (`docs/05-api.md:128`).
- **Ordre de grandeur** : à 4 octets par iid, les 9 600 105 passages vivants des six index demandent au moins 36,6 Mio ; les iid morts avant une compaction ajoutent leur place, et MES-18 relève le chiffre réel, sous le plafond de 64 Mio de la décision 11.

Le **chemin SQL par iid** est écarté : il demanderait un appel par passage du pool, ou un balayage complet de `records.sqlite` par requête, donc une seconde source de lecture sur le chemin d'une route.

### 6. Cohérence

La requête prend **un seul instantané publié** au départ et le garde jusqu'au rendu, comme une recherche (`docs/02-architecture.md:175`, ADR-0043). `generation` est celle de cet instantané, et tous les comptes et toutes les valeurs en viennent : jamais de mélange entre deux générations pendant une attache (INV-03, `docs/08:11`). Une requête en vol sur un index retiré du registre finit sur son instantané (ADR-0105).

### 7. Erreurs

Aucun code nouveau. Les codes réutilisés sont :

| Code | HTTP | Cas |
|---|---|---|
| `invalid_request` | 400 | objet ouvert, `fields` absent, vide, dupliqué, non textuel ou au-delà de dix noms, `limit` ou `timeout_ms` hors bornes, champ inconnu, non filtrable ou d'un type autre que `keyword` |
| `invalid_filter` | 400 | filtre invalide, avec `details.path = "$.filter.<champ>"` |
| `unknown_index` | 404 | index inexistant |
| `unauthorized` | 401 | clé absente, mal formée ou inconnue (ADR-0100) |
| `forbidden` | 403 | rôle insuffisant, la route demandant `read` |
| `payload_too_large` | 413 | corps au-delà de 32 Mio (ADR-0102) |
| `overloaded` | 503 | démarrage non terminé (route ajoutée à la liste d'ADR-0146) ; aucun jeton de sémaphore libre |
| `timeout` | 504 | échéance de `timeout_ms` |
| `internal` | 500 | erreur interne, message fixe (ADR-0101) |

L'ordre des refus suit les règles existantes : `401` et `403` passent avant le `503` de démarrage (ADR-0100, ADR-0146), et une faute de forme du corps qui ne demande pas le schéma rend `400` avant l'accès au registre. L'acceptation d'un champ demande le schéma publié : elle suit donc la règle d'ADR-0146 sur l'accès au registre.

### 8. Concurrence et délai

La route prend un **jeton du même sémaphore** que les recherches, `[server].max_concurrent_searches`, pris sans attente (ADR-0102, décision 2 ; `limits.rs:103-108`). C'est l'extension annoncée en tête : la limite ne s'applique plus seulement aux deux routes de recherche. Un jeton non disponible rend `503 overloaded` immédiat avec `Retry-After: 1`, et la route n'a alors rien calculé.

L'échéance de `timeout_ms` part de la prise du jeton. Elle est vérifiée **entre deux valeurs** d'un champ et **pendant le compte des documents**, en plus des frontières naturelles du travail. À l'échéance, la réponse est `504 timeout`, **jamais un résultat tronqué ni un `other` faux** : une réponse partielle serait pire qu'une erreur, parce qu'un client la lirait comme complète. Le jeton reste tenu jusqu'à la fin réelle du calcul (ADR-0102, décision 3).

Le coût croît avec le **nombre de valeurs distinctes du champ sous filtre**. Il est faible pour `cle_domaine`, `fonds` ou `etat`, dont les vocabulaires sont courts ; il est fort pour un champ comme `num`, qui peut porter des milliers de valeurs distinctes, puisque chaque valeur demande une intersection de bitmaps. Ce coût n'a pas de critère chiffré : le délai le borne, et MES-18 le **publie** par champ.

### 9. Observabilité

Une opération nouvelle, **`facets`**, entre dans la liste **fermée** des opérations de `docs/06-observabilite.md:32`, et `OpEvent` ne gagne **aucun champ** : sa liste reste celle de `docs/06:45` et d'ADR-0117, décision 5. Un appel produit un événement `OpEvent`, avec l'index, l'opération `facets`, le statut (`ok` ou le code rendu), le client et la durée totale.

- **Aucune valeur de champ ni de filtre** n'entre dans les analytics ni dans les journaux (INV-13, `docs/06:45`). Le nom d'un champ de schéma reste permis, comme aujourd'hui : c'est le schéma de l'index, pas une donnée du client.
- **Aucune ligne `filter_shapes`** : la table est écrite par les seules recherches (`docs/06:28`), et une facette n'est pas une recherche. Écrire une forme ferait dire au classement `top_filter_shapes` que des recherches ont exercé ces filtres, ce qui serait faux.
- **Journal de debug non alimenté** : il enregistre des requêtes de recherche (`docs/06`), et une facette n'en est pas une.
- **Aucun iid ne sort** (INV-01, `docs/08:9`) : ni dans `generation`, ni dans un compte, ni dans un nom de champ, ni dans un journal.

### 10. Client Python

Deux méthodes, dans `Client` et `AsyncClient` :

```python
def facets(self, index, fields, *, filter=None, limit=None, timeout_ms=None) -> dict: ...
async def facets(self, index, fields, *, filter=None, limit=None, timeout_ms=None) -> dict: ...
```

Elles font un `POST` vers `/v1/indexes/{index}/facets`, placent `fields` dans le corps et **n'envoient que les options non `None`**, par le constructeur de corps existant (`client.py:451`). Elles rendent le JSON en `dict`, sans type déclaré (ADR-0112, décision 3), comme les méthodes ajoutées par ADR-0144. Aucun `wait` : la route ne crée pas de job.

**Pas de commande CLI en 1.1.** Aucun texte ne l'exige : ADR-0111, décision 5, demande `docs/05` et la spécification OpenAPI, pas une commande, et le ticket se trompe sur ce point (`:88-89`). Une commande serait une idée hors périmètre.

### 11. Mesure MES-18 et repli fixé d'avance

La règle d'or 7 demande de fixer le critère **avant** la mesure. Une mesure **MES-18** est donc faite sur l'implémentation, **avant** que la tâche passe à `FAIT`, en `--release`, sur la machine de développement (ADR-0129, ADR-0135), avec les **six index de l'échelle réelle** de `data/legifacile-full` : 6 660 239 documents et 9 600 105 passages (`docs/mesures/import-legifacile-2026-11-06.md:35`). Elle relève :

- le **p95** d'un appel `facets` avec `count.documents`, sur le plus grand pool (sans filtre, sur le plus grand index) et sous des filtres de corpus ;
- la **mémoire anonyme** ajoutée par la table dérivée sur les six index, au sens d'ADR-0151 (`RssAnon` fait foi pour la mémoire possédée, `docs/adr/0151-chiffre-de-memoire-qui-fait-foi-journal-et-mesure-mes-17.md:77-78`) ;
- la **durée ajoutée à l'ouverture** par la construction de la table ;
- le **coût par champ**, pour une faible cardinalité (`cle_domaine`, `fonds`, `etat`) et une forte cardinalité (`num`), sous filtre et sans.

**Critère de repli.** Si le p95 dépasse **250 ms**, ou si la table dérivée ajoute plus de **64 Mio** de mémoire anonyme sur les six index (la marge de l'attache est de 0,58 Gio, ADR-0142), alors la 1.1 **n'expose pas** `count.documents` : le membre est **omis** (convention d'omission d'ADR-0119, décision 3), la table dérivée est **retirée**, et le compte de documents revient par un **nouvel ADR**. Tout le reste de la route est gardé.

Le seuil de 250 ms est propre à cette route : elle n'est pas l'une des deux routes de métadonnées à 5 ms au p95 d'ADR-0135, décision 6 (`:80-86`), parce qu'elle parcourt des valeurs et des passages au lieu de lire un état. Le coût par champ n'a **pas** de critère : le délai le borne, et MES-18 le publie.

### 12. Classe de compatibilité (ADR-0143)

**Ajout compatible**, au sens de la décision 1 d'ADR-0143 (`:41-60`) : une **route nouvelle** et des **membres de réponse nouveaux**, dont l'absence garde le comportement précédent.

- Aucun `format_version` ne change : ni `records.sqlite`, ni un segment, ni `jobs.sqlite`, ni `analytics.sqlite`, ni un instantané. Aucune migration. La table dérivée vit en mémoire seulement.
- Aucun code d'erreur, aucun membre existant, aucune valeur par défaut existante ne change ; aucune clé ne devient obligatoire.
- La nouvelle valeur `facets` de `operation` dans `analytics.sqlite` est une valeur de plus dans une colonne texte, pas un changement de schéma : `format_version` reste `1`.
- Un client 1.0 ignore la route et les membres : il rend le JSON en `dict` sans type déclaré (ADR-0112).
- La spécification OpenAPI et `docs/05` sont mises à jour **dans le même changement** que le code (ADR-0111, décision 5).

### Ce que cet ADR ne décide pas

- Les facettes sur les champs `integer`, `float` et `date` : valeur exacte ou histogramme, tranches et coût demandent leur propre ADR (décision 3).
- Une forme multi-index de la route, et la fusion de comptes entre index : aucun besoin du ticket, et le contrat de fusion serait nouveau.
- Un cache des réponses de facettes, par index, filtre et génération.
- Le retour de `count.documents` si le repli de la décision 11 s'applique : ce sera un nouvel ADR, avec sa mesure.
- Une commande CLI (décision 10).
- La mention de la route dans la note de version `docs/versions/1.1.0.md`, qui appartient à `PUBLIER-V1.1.0` ; elle y figurera comme route ajoutée.

## Conséquences

- **Positives** :
  - Legifacile décrit ses corpus partagés et ses valeurs fréquentes en **un appel**, sans `query` factice, sans encodage et sans recherche complète ;
  - les deux comptes viennent du **même pool** qu'une recherche de même filtre, donc INV-07 reste vrai sans exception (décision 5), et INV-03 aussi, puisque chaque réponse vient d'un seul instantané (décision 6) ;
  - aucun format persistant ne change : ni migration, ni `format_version`, ni fichier dérivé à sauvegarder ;
  - la route réutilise les briques existantes — bitmaps de valeurs, bitmap des vivants, grammaire de filtre, codes d'erreur, sémaphore, échéance — et n'ajoute qu'une structure en mémoire ;
  - le repli est fixé **avant** la mesure, donc aucun chiffre ne sera choisi après coup pour sauver le membre.
- **Négatives / coûts** :
  - la table dérivée coûte de la mémoire, comptée avec celle des filtres : de l'ordre de 36,6 Mio pour 9,6 M passages, et mesurée exactement par MES-18 contre une marge d'attache de 0,58 Gio ;
  - un champ à forte cardinalité, comme `num`, coûte cher, et le délai est le seul garde-fou ;
  - `keyword` seul : `annee` et les dates ne sont pas servis en 1.1, alors que le ticket les évoque ;
  - une facette prend un jeton du sémaphore des recherches : sous saturation, elle peut refuser une recherche, et inversement ;
  - `count.documents` peut **disparaître** au lendemain de MES-18 si le repli s'applique : un client doit tolérer son absence dès la 1.1 ;
  - une seconde voie d'agrégation doit rester cohérente avec les filtres et le pool ; une divergence future entre `filters_index` et la table dérivée serait un défaut silencieux ;
  - les facettes n'alimentent pas `filter_shapes`, donc `top_filter_shapes` ne voit pas les filtres exercés par cette route.
- **À surveiller** : MES-18 doit passer ses deux seuils (250 ms de p95, 64 Mio de mémoire anonyme), et ses chiffres sont publiés par champ. Le coût de la mise à jour de la table dérivée à chaque commit n'est pas mesuré et reste un signal : un commit nettement ralenti par cette copie serait un défaut. Les douze critères suivants sont les tests attendus de la tâche :
  1. `count.passages` égale le `pool_size` d'une recherche `lexical` `k=1` de même `filter` à la même génération, et égale `live_passages` sans filtre ;
  2. `Σ count + other` égale le nombre de passages du pool qui portent une valeur pour le champ, et `other` vaut `0` quand tout tient dans `limit` ;
  3. l'ordre est `count` décroissant puis `value` croissant en octets UTF-8, et deux appels identiques rendent la même réponse ;
  4. `limit` accepte 1, 20 et 100 et refuse 0 et 101 ; `fields` refuse une liste vide, un doublon et onze noms ;
  5. un champ `integer` ou `date` est refusé en `400 invalid_request` avec `details: {}` ;
  6. un `filter` invalide rend `invalid_filter` avec `details.path` ; un index inconnu rend `404` ; une requête sans clé rend `401` ;
  7. un pool vide rend `200`, des comptes à zéro, `values: []` et `other: 0` ;
  8. pendant une attache, aucune réponse n'est mixte : tout vient d'une seule génération (INV-03) ;
  9. ni iid, ni valeur de champ, ni valeur de filtre ne sortent hors de `values`, et les journaux comme `analytics.sqlite` n'en contiennent aucun (INV-01, INV-13) ;
  10. la route figure dans `/openapi.json` et dans `docs/05`, et son exemple est validé ;
  11. au démarrage, avant la publication du registre, la route rend `503 overloaded` avec le message fixe ;
  12. `count.documents` est exact sur un jeu où plusieurs passages partagent un document, et après une attache qui retire puis ajoute des documents.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| **Dériver les valeurs et les comptes de Postgres**, côté Legifacile | Décrit un autre découpage que celui de Crible. Le ticket l'écarte explicitement (`:110-111`), et ADR-0033 de Legifacile écarte de même une seconde source (`:147-151`), parce qu'un corpus y est un filtre sur un index partagé, pas une table. |
| **Lire `pool_size` d'une recherche factice** | Exige une `query` ou un `vector`, donc un encodage et une latence de recherche, et ne rend ni valeurs ni compte de documents. Le ticket le qualifie lui-même de contournement inacceptable (`:98-100`). |
| **Compter les documents par un appel SQL par iid** | Un appel par passage du pool, ou un balayage complet de `records.sqlite` par requête : le coût dépasse celui de la requête, et l'instantané ne porte pas de transaction SQLite (ADR-0043, `:18-20`). |
| **Rendre un compte de documents par valeur** | Multiplie le travail par le nombre de valeurs et ne répond pas au besoin : `corpus_disponibles` veut un compte de documents par corpus, pas par valeur de champ. |
| **Servir les champs `integer` et `date` en 1.1** | `ValueColumn` n'a pas de bitmap (`filters_index.rs:540-560`) : il faudrait un parcours de colonne ou un histogramme, avec un choix de tranches et une mesure. Aucun texte ne le demande pour ADR-0033 de Legifacile, et le ticket lui-même renvoie `annee` à une décision (`:72-74`). |
| **Persister la table iid → document dans un fichier** | Un fichier dérivé est un format de plus, donc un `format_version`, une migration, une entrée dans les instantanés et les exports, pour une structure qui se reconstruit à l'ouverture comme les bitmaps. |
| **Une commande CLI `crible facets`** | Aucun texte ne l'exige : ADR-0111, décision 5, demande `docs/05` et la spécification OpenAPI, pas une commande. Le ticket l'affirme à tort (`:88-89`). |
| **Une route multi-index dès la 1.1** | Aucun besoin du ticket, et la fusion de comptes entre index (somme des passages, union ou somme des documents) est un contrat nouveau, à décider et à mesurer. `POST /v1/search` reste la seule route multi-index. |
| **Écrire une ligne `filter_shapes` pour les facettes** | La table est écrite par les seules recherches (`docs/06:28`) : une facette n'est pas une recherche, et une forme de plus fausserait `top_filter_shapes`. |
| **Un nouveau code d'erreur pour un champ non facetable** | ADR-0143 ne range pas un code nouveau parmi les ajouts permis en mineure (`:51-60`), et `invalid_request` couvre déjà la faute : le message nomme le champ et son type, et `details` reste `{}` comme pour tout refus de forme. |
| **Un `other` facultatif, omis quand il vaut zéro** | Un client ne pourrait pas distinguer « tout est rendu » de « membre absent », et le ticket demande que le client ne présente pas une liste tronquée comme complète (`:70-71`). `other` est toujours présent, et vaut `0`. |

## Application

Après acceptation seulement, sur la branche de la tâche `T12-FACETTES-ET-COMPTES`, dans un commit qui cite `ADR-0154` :

| Document ou code | Modification |
|---|---|
| `docs/05-api.md:60-78` | une ligne `POST /v1/indexes/{index}/facets`, rôle `read` |
| `docs/05-api.md:58` | la liste des routes refusées au démarrage passe de quatorze à **quinze** routes du registre, `facets` comprise (ADR-0146) |
| `docs/05-api.md` | nouvelle section « Facettes » : corps fermé, `fields`, `filter`, `limit`, `timeout_ms`, forme de la réponse, ordre, `other`, niveau des champs, pool vide, et un exemple validé |
| `docs/06-observabilite.md:32` | `facets` dans la liste fermée des opérations ; aucun champ nouveau dans `OpEvent` (`:45`), aucune ligne `filter_shapes` |
| `docs/03-recherche.md:21-40` | section « Filtres → pool » : la table dérivée iid → document, son coût, et les deux comptes de la route |
| `docs/02-architecture.md:226-243` | « Mémoire : ce qui est en RAM » : la table dérivée, comptée avec les filtres, et son absence de fichier |
| `docs/08-invariants-et-tests.md:82` | une ligne **MES-18** après MES-17, avec le protocole et les deux seuils de la décision 11 |
| `crates/crible-server/src/openapi.rs` | opération `facets`, corps fermé, réponse fermée, exemple, et `503` de démarrage (ADR-0146) |
| `clients/python/src/crible/client.py` | `facets` dans `Client` et `AsyncClient`, options non `None` seulement, retour `dict` |
| `clients/python/README.md` | la méthode `facets` et son exemple d'appel |
| `docs/versions/1.1.0.md` | mention de la route ajoutée, par `PUBLIER-V1.1.0` |

**Tests attendus** : les douze critères de la section « À surveiller », plus MES-18 pour les deux seuils. Les tests de route vivent dans `crates/crible-server/tests/`, le compte de documents et la table dérivée dans `crates/crible-store/tests/`, et le client dans `clients/python/tests/` (marqueur `server` pour l'appel réel, comme ADR-0112).

## Références

- Ticket : `/Users/mattboll/MattCorp/legifacile/tickets-crible/12-facettes-valeurs-frequentes.md` (« Constat », « Besoin », « Observé / attendu », « Impact sur Legifacile », « Contournement actuel », « Pistes de correction »), lu en lecture seule.
- ADR de Legifacile : `/Users/mattboll/MattCorp/legifacile/docs/adr/0033-contenu-de-corpus-disponibles-sans-le-stockage-vectoriel.md`, décisions 2 et 3 (`:89-100`).
- Conception : `docs/05-api.md:58` (routes refusées au démarrage), `:60-78` (tableau des routes), `:64` et `:102-130` (comptes de `GET /v1/indexes/{index}`), `:35` (format d'erreur et `details`), `:318` et `:654` (`timeout_ms`), `:349` (`pool_size`), `:658` (sémaphore des recherches) ; `docs/06-observabilite.md:28` (`filter_shapes`), `:32` (opérations), `:45` (champs fermés, INV-13), `:62` (gestionnaires d'`OpEvent`) ; `docs/03-recherche.md:21-40` (filtres → pool), `:36` (champs matérialisés au passage) ; `docs/02-architecture.md:131` (copie sur écriture), `:175` (instantané par requête), `:226-243` (mémoire) ; `docs/08-invariants-et-tests.md:9` (INV-01), `:11` (INV-03), `:15` (INV-07), `:21` (INV-13), `:82` (MES-17, dernière ligne de mesure).
- Code : `crates/crible-store/src/filters_index.rs:427-435` (`KeywordField::values`), `:540-560` (`ValueColumn`), `:914` (`from_records`), `:1002` (`live`), `:1023` (`column`), `:1029` (`memory`) ; `crates/crible-store/src/generation.rs:313-344` (`Snapshot`), `:328` (`filters`), `:369` (`documents`), `:385` (`live_passages`), `:439` (`filters()`), `:2593` (compte de documents) ; `crates/crible-server/src/search.rs:1073` (`pool_size`) ; `crates/crible-server/src/limits.rs:103-108` et `:148` (sémaphore) ; `crates/crible-server/src/error.rs:70` (`STARTING_MESSAGE`) ; `crates/crible-server/src/config.rs:167` (`max_concurrent_searches`) ; `clients/python/src/crible/client.py:451` (corps sans `None`), `:228` et `:349` (méthodes `search` des deux classes).
- Mesures : `docs/mesures/import-legifacile-2026-11-06.md:35` (6 660 239 documents et 9 600 105 passages sur les six index), `docs/mesures/routes-en-conteneur-2026-11-03.md` (coût des routes de métadonnées, comparaison indicative).
- ADR : ADR-0101 (corps d'erreur, décision 3), ADR-0102 (sémaphore et échéance, décisions 2 et 3, étendues à cette route), ADR-0105 (administration des index), ADR-0111 (spécification et référence des routes, décision 5), ADR-0112 (client Python en `dict`), ADR-0117 (analytics, décision 5), ADR-0119 (omission d'un membre sans valeur, décision 3), ADR-0129 et ADR-0135 (machine et protocole de mesure), ADR-0137 (`keyword` scalaire), ADR-0142 (marge mémoire de 0,58 Gio), ADR-0143 (ligne 1.x, ajout compatible), ADR-0144 (précédent d'une méthode de client), ADR-0146 (routes pendant le chargement), ADR-0147 (domaine de `default_timeout_ms`), ADR-0151 (`RssAnon` fait foi pour la mémoire possédée).
- Suivi : `docs/suivi.md`, ligne de la tâche `T12-FACETTES-ET-COMPTES`.
