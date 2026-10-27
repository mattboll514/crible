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

Opérations comptées : `search`, `search_multi`, `get_document`, `online_batch`, `attach_batch`, `patch_documents`, `compact`, `export`, `snapshot`, `index_admin`. Le statut est `ok` ou un code d'erreur de l'API.

Une ligne par **jour** et par **index** : documents, passages vivants et morts, ratio de morts, tailles sur disque, génération, nombre de lots.

### Ce qu'on ne stocke jamais (INV-13)

- le texte des requêtes ;
- les valeurs de filtre ;
- les hashs de texte : une requête courte comme « article 1240 » se retrouve avec un dictionnaire ;
- les ids des résultats.

**Sûr par construction** : l'agrégateur reçoit un `SearchEvent` qui n'a **aucun champ** pour ces informations. Un test vérifie aussi qu'après une série de requêtes, aucun texte de requête ni aucune valeur de filtre ne se trouve dans `analytics.sqlite`.

### Histogrammes

- 64 seaux géométriques fixes : 0,05 ms × 1,25^i, pour i de 0 à 63 (jusqu'à ~60 s). Ils sont stockés en compteurs `u32`.
- Comme les seaux sont fixes, les histogrammes s'additionnent. On calcule ainsi un p50, un p95 ou un p99 exact sur n'importe quelle période. **On ne moyenne jamais des percentiles.**
- Pour la taille du pool : 33 seaux en log2.

### Écriture

- Chaque requête met à jour un agrégateur en mémoire, avec un verrou court ou un `DashMap`.
- Une tâche asynchrone vide l'agrégateur dans SQLite toutes les 10 secondes et à l'arrêt.
- L'écriture ne bloque jamais une requête. Si la file de vidage est pleine, la perte est comptée dans `analytics_dropped` (**INV-17**).
- Rétention : `analytics.retention_days`, 90 par défaut. Une purge tourne chaque jour.

### Lecture

`GET /v1/stats?from=…&to=…&index=…&operation=…&group_by=minute|hour|day` renvoie :
- les comptes et les erreurs par code ;
- p50, p95 et p99 par étape ;
- le taux de requêtes à 0 résultat ;
- les index et les formes de filtre les plus fréquents ;
- l'état journalier des index.

La CLI `crible stats` affiche la même chose en tableau.

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
- **Écriture** : asynchrone, comme les analytics. La purge selon la durée de vie tourne chaque jour.
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
- **`GET /ready`** : les modèles sont chargés, chaque index a publié un instantané, et il n'y a pas de reprise en cours.
- **Logs** : `tracing` au format JSON sur stdout (Docker). Chaque ligne porte le `request_id` quand il existe.
- **Ce que les logs ne contiennent jamais** : le texte des requêtes et les valeurs de filtre, sauf en niveau `trace`, désactivé en production.
