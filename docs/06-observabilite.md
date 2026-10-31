# 06 — Observabilité

L'observabilité a deux composants, avec deux politiques différentes :

| | Analytics | Journal de debug |
|---|---|---|
| Activation | Toujours | À la demande (config + `debug: true`, ou échantillonnage) |
| Contenu | Agrégats chiffrés **sans aucun contenu** | Requête complète : texte, filtre, plan, résultats |
| Conservation | Longue (90 jours par défaut) | Courte (7 jours par défaut) |
| Fichier | `data/analytics.sqlite` | `data/debug.sqlite` |
| Accès | `admin` | `admin` |

## Analytics

### Ce qu'on agrège

Une ligne par **minute** × **index** × **opération** × **statut** × **client** × **mode** × **stratégie dense** :
- `count` ;
- histogrammes de latence pour chaque étape : `validate`, `encode`, `filter`, `dense`, `lexical`, `fusion`, `group`, `fetch`, `total` ;
- histogramme de la taille du pool ;
- nombre de recherches à 0 résultat ;
- nombre de requêtes tronquées ;
- contribution des branches en hybride : résultats apportés par le dense seul, par le lexical seul, par les deux ;
- `filter_shape` : la liste triée des couples `champ:opérateur`, par exemple `date_debut:lte,fonds:eq`. On n'y met **jamais de valeur**.

`filter_shape` ne fait pas partie de la clé de la ligne : elle est rangée dans sa propre table, `filter_shapes(minute, index, shape, count)`, avec un compteur par forme et par minute (ADR-0117, décision 6). Une ligne de minute porte donc un nombre de clés borné par le nombre de clés d'API, le nombre d'index, les modes et les stratégies, jamais par la variété des filtres.

La forme est **canonicalisée** (ADR-0117, décision 6) : un scalaire vaut `eq`, un objet à plusieurs opérateurs donne un couple par opérateur, `$and`, `$or` et `$not` sont traversés et n'apparaissent pas — la forme dit quels champs et quels opérateurs sont exercés, pas la structure booléenne —, les couples sont triés par octets, dédoublonnés et joints par une virgule. Un filtre absent ou vide ne produit aucune ligne.

Opérations comptées : `search`, `search_multi`, `get_document`, `online_batch`, `attach_batch`, `patch_documents`, `compact`, `export`, `snapshot`, `index_admin`. Le statut est `ok` ou un code d'erreur de l'API.

Une ligne par **jour** et par **index** : documents, passages vivants et morts, ratio de morts, tailles sur disque, génération, nombre de lots. Elle est écrite au premier vidage où la ligne du jour UTC courant manque — donc, en marche continue, au premier vidage après minuit UTC —, comme un instantané de l'état de chaque index à cet instant ; le ratio de morts se dérive des deux comptes et n'est pas stocké (ADR-0118, décision 5 ; ADR-0117, décision 1).

### Ce qu'on ne stocke jamais (INV-13)

- le texte des requêtes ;
- les valeurs de filtre ;
- les hashs de texte : une requête courte comme « article 1240 » se retrouve avec un dictionnaire ;
- les ids des résultats.

**Sûr par construction** : l'agrégateur reçoit un `SearchEvent` et un `OpEvent` dont la liste de champs est **fermée** et ne contient aucun emplacement pour ces informations (ADR-0117, décision 5) ; ajouter un tel champ demande un ADR. Un test vérifie aussi qu'après une série de requêtes, aucun texte de requête ni aucune valeur de filtre ne se trouve dans `analytics.sqlite`. Un nom de champ de schéma, lui, est permis : c'est le schéma de l'index, pas une donnée du client.

### Histogrammes

- 64 seaux géométriques fixes : le seau `i` vaut `[0,05 ms × 1,25^i ; 0,05 ms × 1,25^(i+1))`, pour `i` de 0 à 63. Le seau 0 reçoit aussi tout ce qui est en dessous, et le seau 63 est fermé en haut sur le débordement, jusqu'à environ 80 s (ADR-0117, décision 2). Ils sont stockés en compteurs `u32` little-endian, un BLOB de 256 octets par étape.
- Pour la taille du pool : 33 seaux log2, un BLOB de 132 octets. Le seau 0 compte 0 passage, le seau `i` de 1 à 31 couvre `[2^(i-1) ; 2^i)`, et le seau 32 reçoit `2^31` et au-delà (ADR-0117, décision 3).
- Comme les seaux sont fixes, les histogrammes s'additionnent, seau par seau. On calcule ainsi un p50, un p95 ou un p99 **exact sur la somme des seaux** sur n'importe quelle période : le résultat est celui des événements bruts à un seau près. **On ne moyenne jamais des percentiles.**
- La règle du percentile est celle d'ADR-0117, décision 4 : rang le plus proche `⌈p × n / 100⌉` sur les comptes cumulés des seaux, valeur rendue = borne haute du seau qui contient le rang. La **résolution** est la largeur relative d'un seau : facteur 1,25, soit 25 % pour la latence, et facteur 2 pour le pool. Les valeurs sont donc sur-estimées, jamais sous-estimées : un p95 de 3,2 ms signifie « entre 3,2/1,25 et 3,2 ms ».
- Une étape qui ne s'exécute pas dans le mode demandé (la branche lexicale en mode `dense`) ne produit aucun échantillon : la somme de son histogramme n'égale pas `count`, alors que celle de `total` l'égale.

### Écriture

- Chaque requête dépose son événement dans un canal **borné** de 4 096 événements par `try_send`, et **un seul écrivain** agrège et vide ; aucune requête ne tient de verrou et ne partage de carte (ADR-0118, décision 1). La taille est une constante, pas une clé de configuration.
- L'écriture ne bloque jamais une requête. Si la file est pleine, l'événement neuf est perdu et la perte est comptée dans `analytics_dropped` (**INV-17**), un compteur par processus exposé par `GET /v1/stats` et journalisé en `warn` à chaque vidage où il a augmenté (ADR-0118, décision 2).
- Une tâche asynchrone vide la file dans SQLite toutes les 10 secondes. Un vidage est une transaction, et la minute de rattachement est celle du vidage, lue par SQLite : le décalage est borné par l'intervalle de vidage (ADR-0118, décision 3).
- À l'arrêt, la séquence est celle de `07-stack-et-deploiement.md` : drainage des requêtes d'abord, puis vidage des analytics, borne de 2 secondes, échec journalisé en `warn` sans reprise (ADR-0118, décision 3).
- Rétention : `analytics.retention_days`, 90 par défaut. La purge tourne à l'ouverture, puis à chaque vidage si la dernière purge date de plus de 24 heures : pas de minuteur dédié (ADR-0118, décision 4).
- Les `OpEvent` ne viennent que des gestionnaires de la liste fermée d'opérations donnée ci-dessus, et seulement quand une clé a été résolue : `400`, `404` et `409` compris, `401` et `403` exclus, comme `/health`, `/ready` et `/v1/jobs` ; la CLI n'en produit aucun (ADR-0118, décision 6).

Le format du fichier est celui d'ADR-0117 : `analytics.sqlite` à la racine du dossier de données, `meta.format_version = 1`, une version inconnue refusée à l'ouverture (INV-15).

### Lecture

`GET /v1/stats?from=…&to=…&index=…&operation=…&group_by=minute|hour|day` renvoie :
- les comptes et les erreurs par code ;
- p50, p95 et p99 par étape, à la résolution d'un seau (ADR-0117, décision 4) ;
- le taux de requêtes à 0 résultat ;
- les index et les formes de filtre les plus fréquents ;
- l'état journalier des index ;
- le compteur de pertes d'écriture `analytics_dropped` (ADR-0118, décision 2).

La CLI `crible stats` affiche la même chose en tableau. La forme de la réponse et la façon dont la CLI lit les analytics relèvent de la tâche ROUTE-STATS.

## Journal de debug

- **Config** : `debug_journal.enabled` (false par défaut), `sample_rate` (0 par défaut), `ttl_days` (7 par défaut).
- **Ce qui est enregistré** : une requête avec `debug: true`, si le journal est activé, et une fraction `sample_rate` des autres requêtes.
- **Contenu** :
  - `request_id`, horodatage, index, client ;
  - la requête complète (texte et filtre) ;
  - le plan et les temps par étape ;
  - le top 20 (clé et score de chaque branche) ;
  - la génération et l'erreur éventuelle.
- **Consultation** : `GET /v1/debug/queries?index=&min_total_ms=&from=&to=&limit=`.
- **Écriture** : asynchrone, comme les analytics. La purge selon la durée de vie tourne chaque jour. L'écriture du journal ne bloque pas plus une requête que celle des analytics : INV-17 couvre les deux, et le journal est prouvé sur le même modèle que les analytics (ADR-0118, décisions 7 et 8).
- **Pourquoi** : les requêtes de Legifacile peuvent être sensibles. Le journal est donc désactivé par défaut, sa durée de vie est courte, et il n'est lisible que par un admin.

## Debug dans la réponse et explain

- **`debug: true`** : plan, temps par étape, et rangs et scores de chaque résultat dans chaque branche (format dans `05-api.md`).
- **`explain_ids`** : pour chaque document demandé, le moteur répond à « pourquoi il est, ou n'est pas, remonté » :
  - existe-t-il ? est-il vivant ?
  - est-il dans le pool ? sinon, quelles clauses du filtre l'excluent ? (on évalue chaque clause séparément pour ce document)
  - son rang dense exact, son rang binaire, et s'il faisait partie des N candidats ;
  - son rang et son score BM25, avec l'explication tantivy terme par terme ;
  - son rang après fusion ;
  - s'il a été absorbé par un regroupement.
- Le rang dense exact impose un scan exact du pool, d'où un coût réservé au debug.

## Éval : `crible eval`

```
crible eval --index articles --queries golden.jsonl --modes dense,lexical,hybrid --k 10 --out docs/mesures/
```

**Format du jeu de requêtes**, en JSONL :

```json
{"id": "q-001", "query": "responsabilité du fait des choses", "filter": {"fonds": "LEGI"}, "expected": ["LEGIARTI…"]}
```

`expected` est facultatif.

**Mesures produites, pour chaque mode** :
- **fidélité de l'approximation** : recall@k et recouvrement@k du pipeline normal contre le même pipeline en `exact: true` ; les deux définitions — recall@k au niveau du passage, recouvrement@k au niveau du document — sont fixées par ADR-0076 et s'appliquent ici comme aux mesures de P4 ;
- **qualité** : si `expected` est fourni, recall@k, MRR et nDCG@k ;
- **latence** : p50, p95 et p99 par étape ;
- les requêtes qui régressent par rapport à un rapport précédent (option `--baseline`).

**Sorties** : un JSON brut et un rapport Markdown dans `docs/mesures/`. C'est l'instrument principal du propriétaire pour valider sans lire le code.

## Santé et logs

- **`GET /health`** : le processus est vivant.
- **`GET /ready`** : les modèles sont chargés, chaque index a publié un instantané, et il n'y a pas de reprise en cours. Il rend `503` tant que ces trois conditions ne sont pas réunies, puis `200` ; `/health` rend `200` dès la liaison au port (ADR-0099). Une instance sans index est prête, et un index dont le lexical est refusé ne bloque pas `/ready` : la génération est publiée et seule la branche lexicale est refusée (ADR-0087, ADR-0088, ADR-0089).
- **Logs** : `tracing` au format JSON sur stdout (Docker). Chaque ligne porte le `request_id` quand il existe.
- **Ce que les logs ne contiennent jamais** : le texte des requêtes et les valeurs de filtre, à aucun niveau de log et quel que soit `RUST_LOG` (INV-13, ADR-0096).
