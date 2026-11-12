# 06 — Observabilité

L'observabilité a deux composants, avec deux politiques différentes :

| | Analytics | Journal de debug |
|---|---|---|
| Activation | Toujours | À la demande : `debug: true` toujours quand le journal est activé, `sample_rate` pour les autres requêtes (ADR-0122, décision 1) |
| Contenu | Agrégats chiffrés **sans aucun contenu** | Requête complète : texte, filtre, plan, résultats |
| Conservation | Longue (90 jours par défaut) | Courte (7 jours par défaut) |
| Fichier | `data/analytics.sqlite` | `data/debug.sqlite` |
| Accès | `admin` | `admin` |

Crible n'émet **aucune télémétrie vers un tiers** (ADR-0128) : ses seules sorties sont ses analytics et son journal de debug, tous deux internes au projet, et la télémétrie embarquée par ONNX Runtime 1.31.0 est coupée partout où Crible charge ORT, production comprise. INV-13 et ADR-0014 restent la règle propre à Crible pour les analytics ; ADR-0128 traite le composant tiers, qui n'est pas couvert par cette règle.

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

Opérations comptées : `search`, `search_multi`, `facets`, `get_document`, `online_batch`, `attach_batch`, `patch_documents`, `compact`, `export`, `snapshot`, `index_admin`. Le statut est `ok` ou un code d'erreur de l'API.

Une facette produit un `OpEvent` d'opération `facets`, avec l'index, le statut, le client et la durée totale ; elle n'ajoute **aucun champ** à `OpEvent`, ne porte aucune valeur de champ ni de filtre (INV-13), n'écrit aucune ligne `filter_shapes` — la table est écrite par les seules recherches — et n'alimente pas le journal de debug (ADR-0154, décision 9).

La clé `fetch` ne porte **que la lecture des charges utiles** des `k` résultats. Les lecteurs de segments et le pool de charges utiles sont ouverts au remplissage du cache, avant la publication de la génération, et jamais sur le chemin d'une requête (ADR-0126) : à `k` constant, le p50 de `fetch` ne monte plus avec la taille du corpus, et une croissance en octets de vecteurs signalerait une ouverture restée sur le chemin.

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
- Une recherche multi-index produit **un événement par cible**, jamais une ligne consolidée (ADR-0117, décision 5) : le pool, la stratégie et les contributions sont propres à chaque cible. Un `504` — et tout échec postérieur à la résolution des cibles — suit la même règle, un événement par cible (ADR-0124, décision 10). `mode` est vide quand le mode n'a pas été résolu, et une requête refusée **avant** la résolution des cibles émet **un seul** événement, `index` vide : le nom d'une cible n'est pas repris du corps, puisqu'il n'a pas été résolu.
- **Échantillon de pool d'un échec** (note du 2026-11-01). Un événement d'échec — `504`, refus postérieur à la résolution des cibles — écrit un échantillon de pool dans le **seau 0**, celui du pool vide, alors que sa cible n'a rien filtré : `SearchEvent` porte un `pool` entier, sans emplacement qui dise « aucun pool », et l'observateur d'un échec laisse le pool à zéro (`crates/crible-server/src/search.rs:703` ; `crates/crible-server/src/analytics.rs:576`). C'est une convention, sans effet aujourd'hui, puisque `GET /v1/stats` ne rend aucun percentile de pool (ADR-0124, décision 4) ; un futur lecteur de ces percentiles devrait écarter le seau 0 des échecs, et rendre l'échantillon optionnel demanderait un ADR, la liste des champs de `SearchEvent` étant fermée (ADR-0117, décision 5).

Le format du fichier est celui d'ADR-0117 : `analytics.sqlite` à la racine du dossier de données, `meta.format_version = 1`, une version inconnue refusée à l'ouverture (INV-15).

### Lecture

`GET /v1/stats?from=…&to=…&index=…&operation=…&group_by=minute|hour|day` renvoie (ADR-0124) :
- les comptes et les erreurs par code ;
- p50, p95 et p99 par étape, à la résolution d'un seau (ADR-0117, décision 4) ;
- le taux de requêtes à 0 résultat ;
- les index et les formes de filtre les plus fréquents ;
- l'état journalier des index ;
- le compteur de pertes d'écriture `analytics_dropped` (ADR-0118, décision 2).

La forme de la réponse est **fermée** (ADR-0124, décision 1) : un objet à membres fixes — `from`, `to`, `group_by`, `buckets`, `totals`, `top_indexes`, `top_filter_shapes`, `indexes`, `analytics_dropped` — dont un membre sans valeur est **omis**, jamais `null` ni `0` inventé. `buckets` ne porte que les seaux qui ont des événements, dans l'ordre chronologique ; `totals` a la même forme qu'un seau, sans `start`. Les listes sont toujours présentes, même vides. Le format exact est dans `05-api.md`, section « Statistiques ».

- **Fenêtre et seaux** (ADR-0124, décision 2). `from` et `to` sont des dates ISO 8601 avec décalage, normalisées en UTC, et les deux bornes sont **incluses**, comme pour le journal de debug (ADR-0122, décision 9). `group_by` vaut `minute`, `hour` ou `day`, `hour` par défaut : `minute` est la granularité de stockage de `minute_events`, et un seau `minute` agrège les lignes de même minute, toutes clés confondues. La fenêtre par défaut est **toute la rétention** : la seule borne est `analytics.retention_days`, et aucun plafond de fenêtre ne s'y ajoute. Un paramètre inconnu ou invalide, un `group_by` hors du domaine, une date qui n'est pas ISO 8601 et un `from` postérieur à `to` rendent `400 invalid_request` en nommant le paramètre.
- **Aucun filtre ni aucune répartition de plus** (ADR-0124, décision 3). `index` et `operation` sont les seuls filtres. `status`, `client`, `mode` et `dense_strategy` restent des dimensions de la ligne de minute, agrégées à la lecture, jamais offertes en filtre. Le filtre `index` porte sur les noms **présents dans `analytics.sqlite`**, jamais sur les index vivants : l'historique d'un index supprimé reste lisible, et un nom sans événement rend `200` avec une réponse vide ; seul un nom invalide au sens des règles de nommage des index rend `400 invalid_request` (ADR-0124, décision 2).
- **Percentiles** (ADR-0124, décision 4). Chaque étape des neuf clés qui a **au moins un échantillon** dans le seau porte ses trois percentiles ; une étape sans échantillon est omise, et `total` a autant d'échantillons que d'événements. La valeur est la **borne haute** du seau, donc exacte sur la somme des seaux et sur-estimée à la résolution d'un seau. Les percentiles de taille de pool ne sont pas rendus : l'histogramme de pool est écrit, et cette route ne le lit pas. Le tableau de bord ne les affiche pas davantage (ADR-0130, décision 8) : ils sont écartés de l'interface comme de la route.
- **Taux sans résultat** (ADR-0124, décision 5). Il vaut la somme des `zero_results` divisée par la somme des `count` des seules recherches (`search` et `search_multi`) de statut `ok` ; il est omis quand ce dénominateur est nul. Une recherche multi-index **pèse une fois par cible**, parce qu'elle produit un événement par cible (ADR-0117, décision 5) : le taux n'est pas « par requête reçue ».
- **Classements** (ADR-0124, décisions 6 et 7). `top_filter_shapes` et `top_indexes` rendent **au plus 10** entrées sur la fenêtre, par compte décroissant, départagées par l'ordre lexicographique de la forme et par le nom d'index croissant. Les deux bornes sont des constantes, sans clé de configuration. Le classement des formes porte sur les recherches de la fenêtre **quel que soit `operation`** : seules les recherches écrivent une forme, et la table n'a pas de colonne d'opération (ADR-0124, note du 2026-11-01). Une forme ne contient que des champs et des opérateurs (INV-13).
- **État journalier** (ADR-0124, décision 8). `indexes` rend, par index, sa **dernière** ligne de `day_indexes` de la fenêtre, `day` compris, avec les champs stockés et le `dead_ratio` dérivé. Les jours absents ne sont **pas comblés** : un trou dit la vérité (ADR-0118, décision 5).
- **Pertes** (ADR-0124, décision 9). `analytics_dropped` est le compteur du **processus** serveur depuis son démarrage (ADR-0118, décision 2, `:48-50`), rendu **par la route seule** ; `debug_journal_dropped` reste sur `GET /v1/debug/queries` (ADR-0122, décision 9).
- **Chiffres sur les vidages** (ADR-0124, décision 11). Tout ce que la route rend porte sur ce qui a été **vidé** dans `analytics.sqlite` : une requête de la dernière seconde peut manquer, et le décalage est borné par l'intervalle de vidage de 10 s (ADR-0118, décision 3). La route ne force **jamais** un vidage.
- **Lecture** (ADR-0124, décision 12). L'agrégation se fait **en SQL**, filtrée sur la fenêtre et sur `index` et `operation`, par une connexion `SQLITE_OPEN_READ_ONLY` **dédiée**, jamais sous le verrou de l'écrivain. `open_or_create` n'est pas utilisé pour lire : il écrit le schéma et purge. Les histogrammes s'additionnent seau par seau, en `u32` saturant.
- **Rôle** (ADR-0124, décision 16). La route est réservée à `admin` ; une clé `read` ou `write` reçoit `403` (ADR-0095).

La CLI `crible stats` affiche la même chose en tableau (ADR-0124, décisions 13 à 15). Elle lit `analytics.sqlite` **en lecture seule**, sans aucune écriture, et cette lecture reste permise à côté d'un serveur en service, au sens d'ADR-0116. Elle ne passe pas par l'API : `docs/07` ne porte aucun client HTTP, et il n'y a donc pas de mode API. Un fichier absent rend un message clair, pas une erreur interne. `analytics_dropped` est absent de la CLI, parce qu'il mesure un processus serveur et que le fichier ne le porte pas. Les options sont `--data`, `--from`, `--to`, `--index`, `--operation` et `--group-by`, avec la sémantique de la route ; `--format table` est le défaut, en tableaux ASCII rendus à la main sans dépendance, et `--format json` rend exactement le corps de la route **moins** `analytics_dropped`.

## Journal de debug

- **Config** : `debug_journal.enabled` (false par défaut), `sample_rate` (0 par défaut, dans `[0, 1]`), `ttl_days` (7 par défaut, `0` refusé au démarrage) (ADR-0122, décisions 1 et 8), `max_rows` (500 000 par défaut) et `max_bytes` (2 147 483 648 octets, 2 Gio, par défaut), `0` refusé au démarrage pour les deux, `max_rows >= 1` et `max_bytes >= 1 Mio` (ADR-0127, décision 1).
- **Deux voies d'enregistrement** : une requête qui porte `debug: true` est **toujours** enregistrée quand le journal est activé, quel que soit `sample_rate` ; `sample_rate` ne vise que les autres requêtes, dont une fraction tirée au hasard est enregistrée (ADR-0122, décision 1). Le tirage a lieu avant la prise du jeton de concurrence, donc il n'allonge pas `took_ms`.
- **Rôle** : aucune restriction de rôle ne s'ajoute à `debug: true` ; toute clé valide peut faire enregistrer **sa propre** requête, et la lecture du journal reste réservée à `admin` (ADR-0122, décision 2).
- **Contenu** :
  - `request_id`, horodatage lu par SQLite, index, client (nom de la clé) ;
  - la requête complète (texte et filtre tel que reçu) ;
  - le plan et les temps par étape, repris de l'objet `debug` sans recalcul ;
  - le top 20 (clé, rang et score de chaque branche) ;
  - la génération et l'erreur éventuelle, réduite à son code ;
  - en multi-index, les cibles avec leur filtre, leur génération et leur capture, et `index` à `NULL` sur la ligne (ADR-0121, décision 4 ; ADR-0122, décision 4).
- **Stockage** : `data/debug.sqlite`, à la racine du dossier de données, ouvert et écrit par `crible-store`, avec une table `meta` à `format_version = 1` refusée quand elle est inconnue (INV-15). Le fichier n'est ni ouvert ni créé quand le journal est désactivé (ADR-0121).
- **Consultation** : `GET /v1/debug/queries?index=&min_total_ms=&from=&to=&limit=&before_id=`, réservée à `admin` ; réponse `{ "enabled": bool, "dropped": u64, "capped": bool, "rows": u64, "bytes": u64, "trimmed": u64, "queries": [...] }`, ordre `created_at DESC, id DESC`, pagination keyset sur `id` ; journal désactivé : `200` avec `enabled: false` et `queries: []` ; paramètre inconnu ou invalide : `400 invalid_request` nommant le paramètre (ADR-0122, décision 9 ; ADR-0127, décision 6). `capped`, `rows`, `bytes` et `trimmed` décrivent le plafond de taille, décrit plus bas.
- **Écriture** : asynchrone, comme les analytics. Chaque requête dépose sa ligne dans un canal **borné de 4 096 entrées** par `try_send`, et un seul écrivain vide ; aucune requête ne tient de verrou. File pleine : la ligne neuve est perdue et comptée dans `debug_journal_dropped`, un compteur par processus exposé par la route et journalisé en `warn` à chaque vidage où il augmente (INV-17 ; ADR-0122, décision 6).
- **Vidage** : toutes les 10 secondes et à l'arrêt. À l'arrêt, la file est fermée avant son drainage, et le journal est vidé **après les analytics**, avec une borne de 2 secondes ; un échec est journalisé en `warn` sans reprise (ADR-0122, décision 7).
- **Purge** : selon `ttl_days`, à l'ouverture puis au premier vidage dont la dernière purge date de plus de 24 heures ; pas de minuteur dédié (ADR-0122, décision 8).
- **Plafond de taille** (ADR-0127) : deux bornes indépendantes, `max_rows` et `max_bytes`, et la première atteinte déclenche la suppression des lignes les plus anciennes. `max_bytes` porte sur la **taille utilisée** du fichier, `(page_count - freelist_count) × page_size` ; la taille physique, pages libres comprises et WAL exclu, est `file_bytes`, rapportée à l'opérateur et absente de la route (décisions 1 et 2).
- **Contrôle du plafond** : à **chaque vidage**, dans la transaction, et **une fois à l'ouverture**, après la purge de rétention. La transaction insère d'abord les lignes du vidage, puis supprime les plus anciennes — ordre `created_at ASC, id ASC` — jusqu'à repasser sous les deux bornes ; elle compte les lignes supprimées dans `trimmed` et arme `capped`. Le dépassement est ainsi borné à un intervalle de vidage (décisions 3 et 4). Un vidage sans ligne à insérer et sous les deux bornes se contente d'une lecture, sans transaction d'écriture.
- **Compteurs du plafond** : `rows` est le nombre de lignes, `bytes` la taille utilisée, `trimmed` le nombre de lignes supprimées par le plafond depuis le démarrage, `capped` vrai quand le dernier contrôle a trouvé le journal au plafond et jusqu'au démarrage suivant. Compteurs de processus, jamais persistés (décision 3).
- **Récupération d'espace** : aucun `VACUUM` complet ; un fichier neuf est créé avec `auto_vacuum = INCREMENTAL`, et une récupération `incremental_vacuum` bornée à 256 pages par vidage suit une purge ou un trim, hors de la transaction. Un fichier existant sans ce mode garde son plus haut niveau ; un `VACUUM` hors ligne, journal désactivé, est documenté pour l'opérateur (décision 5).
- **Rétention effective** : le **minimum** de `ttl_days` et de ce que les plafonds laissent tenir. La purge de durée précède le trim, et le plafond peut raccourcir la rétention, jamais l'allonger : `ttl_days` reste la borne de confidentialité, le plafond la borne de disque (décision 7).
- **Logs** : le journal n'écrit jamais `query`, `filter`, la clé ni le payload, à aucun niveau et quel que soit `RUST_LOG` ; seules des lignes `warn` de comptage (pertes, échec de vidage ou de purge, plafond atteint) sont permises, et celle du plafond porte des nombres seulement — `rows`, `bytes`, `file_bytes`, `trimmed` —, au plus une fois par intervalle de purge (ADR-0122, décision 10 ; ADR-0127, décision 6).
- **Pourquoi** : les requêtes de Legifacile peuvent être sensibles. Le journal est donc désactivé par défaut, sa durée de vie est courte, et il n'est lisible que par un admin. Il **contient** le texte et le filtre par conception : INV-13 ne vise que les analytics (ADR-0014 ; ADR-0121, décision 6). Sa taille est bornée en lignes et en octets, indépendamment de `sample_rate` (ADR-0127).

### Récupérer l'espace de `debug.sqlite` hors ligne

Le serveur ne lance **jamais** de `VACUUM` complet (ADR-0127, décision 5) : il récupère au plus 256 pages par vidage, sur un fichier créé avec `auto_vacuum = INCREMENTAL`. Un fichier plus ancien, créé sans ce mode, garde son plus haut niveau de `freelist_count`, et seul l'opérateur peut le compacter, hors ligne.

1. Arrêter l'instance, ou la redémarrer avec `debug_journal.enabled = false` : le fichier n'est alors ni ouvert ni créé (ADR-0121).
2. Vérifier qu'aucun autre processus ne tient le fichier, notamment un `sqlite3` resté ouvert.
3. Lancer le `VACUUM` sur le fichier, sans le serveur :

   ```
   sqlite3 data/debug.sqlite 'VACUUM;'
   ```

4. Contrôler le résultat, puis redémarrer l'instance : `PRAGMA page_count;` et `PRAGMA freelist_count;` doivent montrer un `freelist_count` proche de zéro et une taille utilisée, `(page_count - freelist_count) × page_size`, sous `max_bytes`.

Le `VACUUM` réécrit le fichier en place : il demande de l'espace libre au moins égal à la taille du fichier, et il n'est pas interruptible sans laisser un fichier à reconstruire depuis une sauvegarde. Il ne change ni `format_version` ni le contenu des lignes (INV-15).

## Debug dans la réponse et explain

- **`debug: true`** : plan, temps par étape, et rangs et scores de chaque résultat dans chaque branche (format dans `05-api.md`).
- **`explain_ids`** : pour chaque document demandé, le moteur répond à « pourquoi il est, ou n'est pas, remonté » (ADR-0123 ; format dans `05-api.md`) :
  - existe-t-il ? est-il vivant ?
  - est-il dans le pool ? sinon, quelles clauses du filtre l'excluent ? On évalue chaque clause séparément pour ce document et on ne nomme que des **feuilles**, par leur chemin JSON complet, jamais un nœud combinatoire (ADR-0123, décision 7).
  - son rang dense exact, son rang binaire, et s'il faisait partie des N candidats ; le rang exact porte sur **tout le pool filtré** et reste indépendant de `k`, alors que les rangs de `debug` comptent les `candidates_per_branch` entrées de la branche : les deux échelles ne se comparent pas (ADR-0123, décision 5).
  - son rang et son score BM25, avec la liste fermée `terms` — un terme analysé par entrée, `tf` et contribution, triée par score décroissant, dont la somme égale le score à `1e-4` près. Aucune `Explanation` tantivy brute n'est rendue (ADR-0123, décision 6).
  - son rang après fusion, et la clé du passage qui l'a absorbé s'il a été absorbé par un regroupement.
  - `explain` est un tableau frère de `results`, une entrée par id demandé, dans l'ordre de la requête ; les membres absents sont omis, jamais `null` ni `0` ; au plus 100 ids par requête, les doublons fusionnés (ADR-0123, décisions 1 à 3).
  - en multi-index, chaque élément demandé est un couple `{ "index", "id" }` et la réponse est une liste plate dont chaque entrée porte `index` (ADR-0123, décision 2).
- Le rang dense exact impose un scan exact du pool filtré, sous toute stratégie dense et seulement quand `explain_ids` est présent. L'explication se calcule **après `fetch`** : elle est hors de `took_ms` et hors des neuf clés de `03-recherche.md`, donc sans effet sur les analytics, mais elle reste **sous l'échéance** et rend `504` comme une recherche lente (ADR-0120, décision 1 ; ADR-0123, décision 4).
- `explain_ids` ne change pas les résultats, ne dépend pas de `debug: true`, n'entre ni dans `debug.sqlite` ni dans les analytics, et ne rend aucun `iid` (INV-01 ; ADR-0121 ; ADR-0123, décisions 8 et 9).

## Éval : `crible eval`

```
crible eval (--data <dossier> | --url <base>) --index articles --queries golden.jsonl \
  --modes dense,lexical,hybrid --k 10 --out docs/mesures/ \
  [--models <dir>] [--exact-threshold <n>] [--rescore-candidates <n>] \
  [--repeat <R>] [--baseline <rapport.json>]
```

La commande fonctionne **en local** sur un dossier de données, ou **contre une instance par l'API** (ADR-0131) : en local, elle ouvre le dossier **en lecture**, comme `crible search`, et cette lecture reste permise à côté d'un serveur en service (ADR-0116). Elle n'écrit rien sous le dossier de données. En mode API, elle se choisit par `--url <base>` et vise `POST /v1/indexes/{index}/search` (ADR-0131). `--data` et `--url` sont exclusifs ; `--models`, `--exact-threshold` et `--rescore-candidates` sont refusés en mode API (ADR-0131, décision 6).

**Format du jeu de requêtes**, en JSONL, normatif et fermé (ADR-0125, décisions 2 et 3) :

```json
{"id": "q-001", "query": "responsabilité du fait des choses", "filter": {"fonds": "LEGI"}, "expected": ["LEGIARTI…"]}
```

Chaque ligne porte `id`, puis **exactement un** de `query` ou `vector` ; `filter` et `expected` sont facultatifs. `query` est encodé par le modèle de l'index ; `vector` le remplace et c'est le seul chemin d'une campagne `dense` sur un index à modèle `external`, alors qu'une campagne `lexical` y accepte `query`, aucune branche dense n'ayant à encoder. Une campagne `hybrid` n'y a pas de forme possible : elle exige un texte **et** un vecteur dense, une ligne ne porte que l'un des deux, et le refus tombe dans les deux cas — une ligne à `query` faute de vecteur dense, dont l'encodage est refusé par un modèle sans encodeur, une ligne à `vector` plus tôt, faute de texte (`crates/crible/src/eval.rs:594-601`) (ADR-0109 ; ADR-0125, note du 2026-11-01). `--models <dir>` n'est obligatoire, en mode local, que lorsqu'une branche dense doit encoder une requête textuelle du jeu ; il est inutile autrement. Une ligne invalide arrête la commande avec son numéro de ligne. Un convertisseur Python dans `tools/bench/` produit ce format depuis `data/legifacile-sample/queries.jsonl` ; les fichiers d'entrée restent dans `data/`, **sauf le jeu de référence de P10**, versionné sous `docs/legifacile/reference/` parce qu'il ne contient aucune requête d'utilisateur final (ADR-0138, qui excepte ce seul jeu de la décision 16 d'ADR-0125).

`expected` est une liste de **`doc_id`**. La qualité se mesure **au document**, sur des résultats regroupés par document, avec `k = 10` par défaut (ADR-0125, décision 4).

**Mesures produites, pour chaque mode** :
- **fidélité de l'approximation** : recall@10 au **passage** contre le même pipeline en `exact: true`, en moyenne sur les requêtes, et recouvrement@10 au **document**, sans cible ; les définitions sont celles d'ADR-0076, décision 1, et la cible de 0,99 de `docs/08` est rappelée dans le rapport (ADR-0125, décision 6). Le dénominateur du recall@10 est **|top-10 EXACT|** et non 10 : il vaut 10 dès que le pool filtré compte au moins dix passages, ce qui rend la mesure identique à celle d'ADR-0076 et de MES-01, et il reste juste sur un pool plus petit. Le recouvrement@10 au document divise par le nombre de `doc_id` distincts du top-10 EXACT, même règle. Une requête dont la **liste exacte est vide** (pool vide) est **exclue** de la moyenne de fidélité et **comptée à part** dans le rapport (ADR-0125, complément du 2026-11-01, point C1) ;
- **qualité** : sur les requêtes dont `expected` est non vide, recall@k et MRR@k au document, par requête et en moyenne, avec leur `n` (ADR-0092, décision 2 ; ADR-0125, décision 5). nDCG@k n'est pas calculé : aucun jugement gradué n'existe ;
- **latence** : en local, p50, p95 et p99 par étape et pour `total`, au rang le plus proche (ADR-0117, décision 4), après une passe d'échauffement non comptée et `--repeat` répétitions (`3` par défaut) ; `eval` pose `debug: true` en interne et lit `timings_ms` (ADR-0119). En API, le rapport porte `latency_basis` (`server_stages` en local, `client_round_trip` en API) et seulement `total`, l'aller-retour HTTP mesuré côté client (ADR-0131, décision 7). Les latences sont **indicatives** ;
- les requêtes qui régressent par rapport à un rapport précédent (option `--baseline`).

**Surcharges** (ADR-0125, décision 7). En mode local, `--exact-threshold` et `--rescore-candidates` remplacent, pour la durée de la commande et **sans aucune écriture**, les valeurs de `search` de la configuration de l'index ; le rapport les reprend, avec `dense_strategy` lu dans le plan. En local, la fidélité est mesurée par défaut avec `exact_threshold = 0`, ce qui force la stratégie binaire (ADR-0076, décision 4).

**Régression** (ADR-0125, décisions 8 et 12). `--baseline <rapport.json>` compare, par mode, les moyennes de recall@k, de MRR@k et de recall@10 de fidélité. Une baisse de plus de **0,005 en valeur absolue** est une régression ; le code de sortie vaut alors **1**, sinon **0**. Le rapport liste aussi les requêtes qui ont perdu un attendu trouvé par la référence. Les latences ne sont comparées et rapportées (écart relatif des p50 et p95) que si leur `latency_basis` est identique ; sinon elles sont dites « non comparables », sans changer le code de sortie (ADR-0131, décision 7). Une référence d'une `format_version` autre que 1 ou 2, ou d'un autre `k`, est refusée avec un message clair.

**Sorties** (ADR-0125, décisions 11 à 13, 16 ; ADR-0131, décision 7). Un JSON **fermé**, `format_version: 2`, qui porte à la racine `source: { "mode": "local" | "api", "url"? }` et reprend la commande, l'index, la génération, `k`, les modes, et, en local, N et `exact_threshold`, la version du binaire et le commit s'il est connu, puis les mesures par mode ; et un Markdown lisible sans contexte, au modèle de `docs/mesures/README.md`. Les noms sont `eval-<jeu>-<AAAA-MM-JJ>.json` et `eval-<jeu>-<AAAA-MM-JJ>.md`. `--out <dossier>` est obligatoire ; la commande refuse d'écraser un fichier existant et n'écrit jamais ailleurs. Le rapport n'identifie les requêtes que par leur `id`, sans texte de requête ni valeur de filtre. C'est l'instrument principal du propriétaire pour valider sans lire le code.

## Santé et logs

- **`GET /health`** : le processus est vivant.
- **`GET /ready`** : les modèles sont chargés, chaque index a publié un instantané, et il n'y a pas de reprise en cours. Il rend `503` tant que ces trois conditions ne sont pas réunies, puis `200` ; `/health` rend `200` dès la liaison au port (ADR-0099). Une instance sans index est prête, et un index dont le lexical est refusé ne bloque pas `/ready` : la génération est publiée et seule la branche lexicale est refusée (ADR-0087, ADR-0088, ADR-0089).
- **Logs** : `tracing` au format JSON sur stdout (Docker). Chaque ligne porte le `request_id` quand il existe.
- **Ce que les logs ne contiennent jamais** : le texte des requêtes et les valeurs de filtre, à aucun niveau de log et quel que soit `RUST_LOG` (INV-13, ADR-0096).

### Mémoire

Trois chiffres de mémoire circulent et ne mesurent pas la même chose. La règle qui dit lequel fait foi est celle d'ADR-0151, et c'est elle qui sert à vérifier un budget ou à lire un OOM.

- **`memory.current` du cgroup v2 fait foi pour la limite du conteneur et pour l'OOM** (ADR-0151, ADR-0135 décisions 5 et 7). Sa source est `/sys/fs/cgroup/memory.current`, ou le fichier `memory.current` du cgroup du processus quand le conteneur n'est pas à la racine. C'est le compteur que le noyau compare à `memory.max`, donc le seul dont le dépassement tue le processus. Il se **ventile par `memory.stat`** : `anon` pour la mémoire anonyme, `file` pour le cache de pages ; le reste — slab, piles noyau, tables de pages — n'est pas ventilé et se déclare comme tel.
- **`RssAnon` du processus fait foi pour la mémoire possédée.** Sa source est `/proc/<pid>/status`, complété par `VmRSS`, `RssFile` et `RssShmem`. C'est le chiffre qui répond à « qu'est-ce que Crible garde pour lui ».
- **`RssFile` et `file` du cgroup sont le cache évinçable des `mmap`.** Les pages des vecteurs fp16 et de tantivy peuvent y monter sans faute : sous pression, le noyau les réclame (règle de mémoire de `docs/02`).
- **`docker stats` ne fait pas foi.** Il affiche le même compteur de cgroup avec le cache compris, et son échantillonnage coûte environ 1,5 s par appel. Quand il s'écarte de `memory.current`, c'est `memory.current` qui a raison.
- **La session ONNX ne se sépare que par différence au chargement** (ADR-0077, décision 5) : aucun compteur par session n'existe, la `Session` de `crible-encode` restant sous un `Mutex`. La part du modèle se lit donc entre deux états, l'un avant le chargement, l'autre après.
- **Hors Linux, aucun de ces chiffres n'existe** : ni cgroup v2, ni `/proc` au sens du noyau Linux. Une mesure native ne se compare pas à une mesure en conteneur.

**La ligne `memory`.** Sous Linux seulement, le serveur émet une ligne `tracing` de **niveau `info`** dont le message est `memory` (ADR-0151). Ses champs, tous en octets, sont `rss_bytes`, `rss_anon_bytes`, `rss_file_bytes` pour le processus, et `cgroup_current_bytes`, `cgroup_anon_bytes`, `cgroup_file_bytes` pour le cgroup ; s'y ajoutent `job_id`, `kind`, `index` et `step` selon le point d'émission. Les sources sont lues par `std::fs`, sans dépendance, comme `/proc/self/mountinfo` l'est à l'amorçage.

- **Points d'émission** : `step` vaut `job_end` à la fin de chaque job, quel qu'il soit ; `delete_before_rename` dans le job `delete_index`, juste avant le renommage du dossier vers la quarantaine ; `delete_after_removal` après la suppression du dossier ; `session_load` au chargement d'une session ONNX, `index` nommant alors l'index qui l'a déclenché. Sous charge, la ligne `session_load` peut nommer un index qui n'a pas déclenché le chargement : le compteur de chargements est partagé par l'instance, donc il est un signal et non une attribution (ADR-0151, erratum du 2026-11-11).
- **Source illisible** : le champ est **absent**, jamais une erreur et jamais un `0`, qui serait un état inventé. Aucun démarrage, aucune requête et aucun job n'échoue à cause de cette ligne.
- **Hors Linux** : aucune ligne n'est émise.
- **Confidentialité** : la ligne ne porte que des nombres, un identifiant de job, un genre de job, un nom d'index et une étape. Aucun texte de requête, aucune valeur de filtre, aucun identifiant interne (INV-13, ADR-0096).

Cette ventilation est ce que mesure MES-17 (`docs/08`). Elle n'est exposée par aucune route : un chiffre nouveau dans `GET /v1/stats` serait un écart à ADR-0124, décision 1, et toute correction de la mémoire — déchargement d'une session, `madvise`, `malloc_trim`, allocateur — demande la mesure puis un ADR distinct (ADR-0151).
