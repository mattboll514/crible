# ADR-0117 — Analytics : stockage et calcul (schéma, histogrammes, percentile, formes de filtre)

- **Statut** : Accepté (accepté par anticipation par le propriétaire le 2026-10-31)
- **Date** : 2026-10-31
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, tâche AGREGER-ANALYTICS (phase P7), sur les options recommandées par `@architecte` et retenues par l'orchestrateur, en application de la délégation du propriétaire du 2026-10-31 pour la série de P7
- **Remplace** : — (ne remplace aucune décision : applique ADR-0014 et précise `docs/06`, en suivant le précédent de `jobs.sqlite` posé par ADR-0103, sans modifier le corps d'aucun ADR accepté)

## Contexte

### La fiche et ce qu'elle demande

La fiche `AGREGER-ANALYTICS` (`docs/phases/P7-observabilite.md:32-46`) demande cinq livrables : les types `SearchEvent` et `OpEvent` **sans champ** pour un texte, une valeur de filtre ou un identifiant (`:37`) ; le calcul de `filter_shape` à partir de l'AST, champs et opérateurs seulement (`:38`) ; un agrégateur par minute, avec des histogrammes à 64 seaux et un histogramme de pool en log2 (`:39`) ; un vidage asynchrone toutes les 10 s et à l'arrêt dans `analytics.sqlite` (`:40`) ; les lignes journalières par index (`:41`) ; la purge selon la rétention (`:42`).

Ses deux critères d'acceptation sont : « un test vérifie que les histogrammes s'additionnent (un p95 calculé sur deux minutes fusionnées égale le p95 calculé sur les événements bruts, **à la résolution d'un seau près**) » (`:44`), et le vidage ne bloque jamais une requête, INV-17, file saturée artificiellement (`:45`). La fiche ne cite qu'INV-17 (`:46`).

Cet ADR traite le **stockage et le calcul** ; l'**exécution** (file, pertes, vidage, purge, opérations, preuve d'INV-17) est l'objet d'ADR-0118.

### Ce que la conception pose

`docs/06-observabilite.md:17` définit la ligne de minute : une ligne par **minute × index × opération × statut × client × mode × stratégie dense**, avec `count`, un histogramme de latence par étape, un histogramme de taille de pool, le nombre de recherches à 0 résultat, le nombre de requêtes tronquées et la contribution des branches en hybride. `:19` nomme les neuf étapes mesurées, avec les clés de `docs/03-recherche.md:5-15` : `validate`, `encode`, `filter`, `dense`, `lexical`, `fusion`, `group`, `fetch`, `total`. `:26` ferme la liste des opérations — `search`, `search_multi`, `get_document`, `online_batch`, `attach_batch`, `patch_documents`, `compact`, `export`, `snapshot`, `index_admin` — et le statut vaut `ok` ou un code d'erreur de l'API. `:28` définit la ligne journalière : documents, passages vivants et morts, ratio de morts, tailles sur disque, génération, nombre de lots.

`:41` fixe 64 seaux géométriques, `0,05 ms × 1,25^i`, jusqu'à environ 60 s, en compteurs `u32`. `:42` affirme que ces seaux fixes s'additionnent et qu'on calcule ainsi un p50, un p95 ou un p99 **exact** sur n'importe quelle période, sans jamais moyenner des percentiles. `:43` fixe « 33 seaux en log2 » pour la taille du pool.

L'invariant INV-15 (`docs/08-invariants-et-tests.md:23`) exige que tout format sur disque soit little-endian explicite et versionné, et qu'une version inconnue soit refusée. INV-13 (`:21`) interdit dans `analytics.sqlite` un texte de requête, une valeur de filtre, un hash et un identifiant de résultat ; INV-01 (`docs/02-architecture.md:53`) interdit qu'un `iid` y sorte du moteur. ADR-0014 fixe la raison d'être de ces deux interdits.

`docs/02-architecture.md:60` place `analytics.sqlite` à la racine du dossier de données, à côté d'`instance.json`, `jobs.sqlite` et `debug.sqlite` (`:62`). ADR-0103 a créé le précédent d'un fichier SQLite d'instance possédé par `crible-store` : `crates/crible-store/src/jobs.rs:22-31` rappelle que ce module porte déjà `rusqlite` (ADR-0038) et que `crible-server` n'a **aucune arête** vers cette crate. `crates/crible-store/src/sqlite.rs:83-143` (`check_format_version`) est le contrôle de version à réutiliser : table `meta` absente, ligne absente, valeur non entière et version inconnue sont quatre refus distincts, chacun nommant `format_version` et le fichier.

### Les deux écarts que cet ADR tranche

**Écart 1 — la résolution du percentile.** `docs/06:42` annonce un percentile « exact » ; la fiche écrit « à la résolution d'un seau près » (`docs/phases/P7:44`). Un histogramme de seaux ne peut pas rendre une valeur exacte : il rend la borne du seau qui contient le rang. Les deux phrases ne se contredisent que si « exact » veut dire « exact à l'unité » ; elles s'accordent si « exact » porte sur la **somme des seaux**. C'est la lecture retenue, et elle est écrite ci-dessous.

**Écart 2 — la cardinalité de `filter_shape`.** `docs/06:17` énumère les dimensions de la clé de ligne et n'y met pas la forme du filtre ; `:24` la cite pourtant comme une colonne agrégée de cette même ligne. Une ligne dont la clé porterait la forme aurait autant de clés distinctes que de filtres différents : la rétention ne purgerait presque rien et les comptes par minute deviendraient illisibles. La forme est donc rangée dans une table à part, comme le demande la tâche.

## Décision

### Décision 1 — `analytics.sqlite`, possédé par `crible-store`, versionné

**Décision.** `analytics.sqlite` vit à la racine du dossier de données (`docs/02:60`). Il est ouvert, lu et écrit par **`crible-store`**, dans un module propre, et `crible-server` ne gagne **aucune arête** vers `rusqlite` : c'est l'organisation déjà retenue par ADR-0103 pour `jobs.sqlite` (`crates/crible-store/src/jobs.rs:22-31`), et `docs/07:39` ne déclare `rusqlite` qu'auprès du stockage. Le fichier est en WAL, avec le `busy_timeout` d'ADR-0038, comme `records.sqlite` et `jobs.sqlite`.

Il porte quatre tables :

```sql
CREATE TABLE meta (
  format_version INTEGER NOT NULL          -- 1 pour ce build
);

CREATE TABLE minute_events (
  minute            TEXT NOT NULL,         -- 'AAAA-MM-JJTHH:MM:00Z'
  "index"           TEXT NOT NULL,
  operation         TEXT NOT NULL,         -- liste fermée de docs/06:26
  status            TEXT NOT NULL,         -- 'ok' ou un code d'erreur de l'API
  client            TEXT NOT NULL,         -- nom de la clé (ADR-0100)
  mode              TEXT NOT NULL,         -- '' quand l'opération n'a pas de mode
  dense_strategy    TEXT NOT NULL,         -- '' idem (ADR-0074)
  count             INTEGER NOT NULL,
  zero_results      INTEGER NOT NULL,
  truncated         INTEGER NOT NULL,
  dense_only        INTEGER NOT NULL,      -- contribution des branches (hybride)
  lexical_only      INTEGER NOT NULL,
  both              INTEGER NOT NULL,
  latency_validate  BLOB, latency_encode BLOB, latency_filter BLOB,
  latency_dense     BLOB, latency_lexical BLOB, latency_fusion BLOB,
  latency_group     BLOB, latency_fetch BLOB, latency_total BLOB,
  pool              BLOB,                 -- 33 seaux log2 (décision 3)
  PRIMARY KEY (minute, "index", operation, status, client, mode, dense_strategy)
);

CREATE TABLE day_indexes (
  day             TEXT NOT NULL,           -- 'AAAA-MM-JJ'
  "index"         TEXT NOT NULL,
  documents       INTEGER NOT NULL,
  live_passages   INTEGER NOT NULL,
  dead_passages   INTEGER NOT NULL,
  disk_bytes      INTEGER NOT NULL,
  generation      INTEGER NOT NULL,
  batches         INTEGER NOT NULL,
  PRIMARY KEY (day, "index")
);

CREATE TABLE filter_shapes (
  minute   TEXT NOT NULL,
  "index"  TEXT NOT NULL,
  shape    TEXT NOT NULL,                  -- couples champ:opérateur (décision 6)
  count    INTEGER NOT NULL,
  PRIMARY KEY (minute, "index", shape)
);
```

Précisions :

- `meta.format_version` vaut **1**. À l'ouverture, une version inconnue est **refusée** par le contrôle de `crates/crible-store/src/sqlite.rs:83-143`, réutilisé tel quel : INV-15 (`docs/08:23`) ne laisse pas le choix, et `jobs.sqlite` a déjà ce comportement (ADR-0103).
- `mode` et `dense_strategy` sont `NOT NULL`, la **chaîne vide** valant « sans objet » pour une opération. Ce n'est pas un détail : SQLite accepte `NULL` dans la clé primaire d'une table `rowid`, ce qui laisserait deux lignes identiques coexister et dédoublerait les comptes.
- Le ratio de morts de `docs/06:28` n'a **pas de colonne** : il se dérive de `dead_passages / (live_passages + dead_passages)` à la lecture. Un ratio stocké pourrait contredire ses deux comptes.
- `disk_bytes` est le total occupé par le dossier de l'index à l'instant de la mesure, tous fichiers confondus. Un découpage par famille de fichiers n'est demandé par aucune lecture et n'est pas décidé ici.
- Chaque histogramme est un **BLOB de compteurs `u32` little-endian**, sans en-tête (INV-15) : 64 compteurs, soit 256 octets, pour la latence ; 33 compteurs, soit 132 octets, pour le pool. Les bornes des seaux et le nombre de seaux appartiennent à `format_version` : ils ne sont pas stockés, et aucune valeur flottante n'entre dans le fichier. Un BLOB d'une autre longueur est **refusé**, jamais tronqué ni complété en silence, avec le même code d'erreur et le même style de message que `check_format_version`.
- Un compteur de seau est `u32` dans le fichier (`docs/06:41`) et s'incrémente en **saturation** : il ne déborde jamais et ne panique jamais (règle 10 d'`AGENTS.md`). La somme d'une période se fait en `u64`, donc additionner des semaines de lignes ne peut pas déborder.
- Seuls les `crates/crible-store` connaissent ces tables ; `crible-server` ne parle que du type `SearchEvent`, `OpEvent` et des lectures des analytics.

### Décision 2 — histogramme de latence, 64 seaux, bornes exactes

**Décision.** Le seau `i`, pour `i` de 0 à 63, couvre `[0,05 ms × 1,25^i ; 0,05 ms × 1,25^(i+1))`.

Précisions :

- le seau 0 reçoit aussi tout ce qui est **strictement inférieur** à sa borne basse, soit 0,05 ms : une durée nulle ou négligeable n'a pas de seau propre ;
- le seau 63 est **fermé à droite** et reçoit tout dépassement : `[0,05 × 1,25^63 ; +∞)`, dont la borne haute nominale est 0,05 × 1,25^64 ms, soit environ 80 s. Le « jusqu'à environ 60 s » de `docs/06:41` est un ordre de grandeur ; la borne exacte est celle-ci, et c'est elle que `docs/06` reçoit par cet ADR ;
- les bornes sont calculées en `f64` à la lecture, à partir de ces deux constantes (`0,05` ms et `1,25`). Le fichier ne contient que des comptes ;
- une étape qui ne s'exécute pas dans le mode demandé (la branche lexicale en mode `dense`, par exemple) ne produit **aucun échantillon** : la somme de son histogramme n'est donc pas égale au nombre d'événements de la ligne, alors que la somme de `latency_total` l'est. Un lecteur qui divise par `count` obtient une moyenne fausse et c'est voulu : la moyenne n'est pas ce qu'on lit.

### Décision 3 — histogramme de pool, 33 seaux log2

**Décision.** Les 33 seaux sont indexés de 0 à 32 :

| Seau | Taille de pool couverte |
|---|---|
| 0 | 0 passage |
| `i`, de 1 à 31 | `[2^(i-1) ; 2^i)`, donc `[1 ; 2)` pour le seau 1, `[2 ; 4)` pour le seau 2, jusqu'à `[2^30 ; 2^31)` |
| 32 | `2^31` et au-delà |

Précisions :

- 0 a son seau propre, parce que « aucun passage ne satisfait le filtre » est un fait métier distinct de « un passage » ;
- 1 est dans le seau 1 avec le seul entier 1 : la première frontière est `2^0`, pas `2^1` ;
- le seau 32 est ouvert à droite : il reçoit tout ce qui atteint `2^31`, c'est-à-dire les pools géants. Aucun index de v1 n'approche 2^31 passages (8,58 M pour Legifacile), donc ce seau est un garde-fou, pas une mesure ;
- le compteur de pool est `u32` comme les autres ; un pool est un nombre de passages, donc de même domaine.

### Décision 4 — règle du percentile : rang le plus proche, borne haute du seau

**Décision.** Un percentile se calcule **sur un histogramme**, par ces quatre étapes :

1. `n` est la somme des compteurs des 64 seaux (ou des 33 seaux du pool), en `u64` ;
2. le rang est le **rang le plus proche** de `p` : `r = ⌈p × n / 100⌉`, en arithmétique entière, et `r` vaut au moins 1 dès que `n ≥ 1` (le même esprit que le rang le plus proche d'ADR-0077) ;
3. on parcourt les seaux par valeur croissante en cumulant leurs compteurs ; le premier seau dont le cumul atteint `r` **contient** le rang ;
4. la valeur rendue est la **borne haute** de ce seau : `0,05 × 1,25^(i+1)` pour la latence, `2^i` pour le pool (`0` pour le seau 0, `2^31` pour le seau 32).

Précisions :

- **« exact » de `docs/06:42` veut dire exact sur la somme des seaux.** On additionne des histogrammes, seau par seau, puis on applique la règle : le résultat est celui qu'on aurait obtenu sur les événements bruts **à un seau près**. C'est ce que la fiche appelle « à la résolution d'un seau près » (`docs/phases/P7:44`) et l'écart entre les deux phrases tombe ;
- **on ne moyenne jamais des percentiles** (`docs/06:42`) : le percentile de deux minutes est celui de la somme des deux histogrammes, jamais la moyenne des deux percentiles ;
- la **résolution** est la largeur relative d'un seau : facteur 1,25, soit 25 % pour la latence, et facteur 2 pour le pool. Un p95 de 3,2 ms signifie « entre 3,2/1,25 et 3,2 ms » ;
- le seau 0 rend `0,0625 ms`, sa borne haute : une durée inférieure est donc **sur-estimée**, jamais sous-estimée. Le biais est toujours du même côté, ce qui garde les comparaisons entre périodes valides ;
- pour le pool, un seau de 1 se lit `2` et un seau de 3 se lit `4` : la borne haute est le prix de la résolution log2, et elle surestime d'un facteur au plus 2, jamais de plus ;
- `p = 0` n'a pas de sens et n'est pas servi : les percentiles lus sont p50, p95 et p99 (`docs/06:56`).

### Décision 5 — `SearchEvent` et `OpEvent` : liste fermée de champs

**Décision.** Les deux types ont exactement les champs suivants, et **aucun autre**.

`SearchEvent` :

| Champ | Contenu |
|---|---|
| `minute` | minute de rattachement (décision 1) |
| `index` | nom de l'index cible |
| `operation` | `search` ou `search_multi` |
| `status` | `ok` ou le code d'erreur rendu |
| `client` | **nom** de la clé d'API résolue (ADR-0100), jamais la clé, son haché ni l'en-tête |
| `mode` | `dense`, `lexical` ou `hybrid` |
| `dense_strategy` | `exact` ou `binary` (ADR-0074) |
| durées | une durée par étape exécutée, dans les clés de `docs/03:5-15` |
| `pool` | nombre de passages du pool |
| `zero_results` | vrai si la réponse ne porte aucun résultat |
| `truncated` | vrai si la requête a été tronquée |
| contributions | trois booléens : résultats apportés par le dense seul, par le lexical seul, par les deux |

`OpEvent` :

| Champ | Contenu |
|---|---|
| `minute` | minute de rattachement |
| `index` | nom de l'index, ou `''` pour une opération d'instance |
| `operation` | une des huit opérations non-`search` de `docs/06:26` |
| `status` | `ok` ou le code d'erreur rendu |
| `client` | nom de la clé résolue (ADR-0100) |
| `duration` | durée totale de l'opération |

Précisions :

- **aucun champ** pour un texte de requête, une valeur de filtre, un hash, un identifiant de résultat ou un `iid` : ni sous forme de chaîne, ni sous forme de nombre, ni « pour le debug ». C'est la sûreté **par construction** d'ADR-0014 et d'INV-13 (`docs/06:37`, `docs/08:21`), et INV-01 interdit l'`iid` (`docs/02:53`). Un champ ajouté plus tard demanderait un ADR, pas une revue de code ;
- **une recherche multi-index produit un événement par cible**, jamais une ligne consolidée : le pool, la stratégie et les contributions sont propres à chaque cible, et une ligne globale ne pourrait que les moyenner, ce que la conception refuse par ailleurs. La somme par période se fait à la lecture, qui relève de ROUTE-STATS ;
- les contributions ne sont renseignées qu'en `hybrid` ; en `dense` et en `lexical`, les trois compteurs restent à zéro ;
- le statut est un code d'erreur de l'API (`docs/05-api.md:41-53`), pas un statut HTTP : plusieurs codes partagent un statut, et `docs/06:26` dit « un code d'erreur de l'API ».

### Décision 6 — `filter_shape` : règle de canonicalisation, et table séparée

**Décision.** La forme d'un filtre est la liste **triée** des couples `champ:opérateur` du filtre, **sans aucune valeur**, écrite comme dans `docs/06:24` (`date_debut:lte,fonds:eq`). Elle se calcule ainsi :

1. **Feuille.** `{"fonds": "LEGI"}` donne `fonds:eq` : un scalaire vaut `eq`. `{"date_debut": {"lte": "2020-01-01"}}` donne `date_debut:lte`. Un objet à plusieurs opérateurs donne un couple par opérateur, donc `{"annee": {"gte": 2000, "lt": 2010}}` donne `annee:gte` et `annee:lt` ;
2. **Connecteurs.** `$and`, `$or` et `$not` sont **traversés** et **n'apparaissent pas** dans la forme. `$not` ne change pas l'opérateur des feuilles qu'il enveloppe : `{"$not": {"f": {"eq": "a"}}}` et `{"f": "a"}` donnent tous deux `f:eq`. La forme décrit **quels champs et quels opérateurs sont exercés**, pas la structure booléenne. Écrire la structure demanderait soit les valeurs, soit une seconde grammaire, et multiplierait la cardinalité sans qu'aucune lecture ne la demande — or c'est précisément la cardinalité que la décision corrige ;
3. **Tri et doublons.** Les couples sont triés par ordre lexicographique stable sur les octets, puis les doublons sont retirés. `{"$and": [{"f": "a"}, {"f": "b"}]}` et `{"$or": [{"f": "b"}, {"f": "a"}]}` donnent tous deux `f:a,f:b` ;
4. **Jointure.** Les couples sont joints par une virgule, sans espace, comme l'exemple de `docs/06:24` ;
5. **Filtre absent ou vide.** Aucun couple, donc **aucune ligne** dans `filter_shapes` : une forme vide n'est pas un filtre et n'a pas à figurer au classement des formes les plus fréquentes.

**Rangement.** La forme est stockée dans la table `filter_shapes(minute, "index", shape, count)` (décision 1), **pas** dans la clé de `minute_events`. Un compteur est incrémenté par événement de recherche portant un filtre non vide, pour l'index de cet événement ; les opérations autres que les recherches n'en produisent pas.

**Pourquoi cette table.** Une clé de ligne qui porterait la forme aurait une clé distincte par filtre distinct : sur Legifacile, où un filtre combine librement fonds, dates, numéros et parties, la table des minutes grossirait sans borne utile et la rétention ne purgerait presque rien. La table séparée garde la ligne de minute bornée par le nombre de clés d'API fois le nombre d'index fois deux modes fois deux stratégies, et la forme reste lisible comme ce que `docs/06:58` demande : « les formes de filtre les plus fréquentes ».

Ce que cet ADR **ne décide pas** : la forme exacte de la réponse de `GET /v1/stats`, l'agrégation `group_by=minute|hour|day` et la façon dont `crible stats` lit les analytics ; tout cela relève de ROUTE-STATS.

## Conséquences

- **Positives** :
  - les analytics ont un format sur disque **versionné** et **little-endian**, refusé quand la version est inconnue, avec le même contrôle que `jobs.sqlite` : un fichier d'une autre version ne se lit pas de travers, il se refuse ;
  - la sûreté d'INV-13 est **structurelle** : les types n'ont pas de champ pour le contenu, donc la fuite demande de changer un type, pas d'oublier un filtre ;
  - la règle du percentile est une seule phrase, testable, et la fiche comme `docs/06` disent la même chose ;
  - la ligne de minute reste bornée : la forme du filtre ne fait pas exploser la table ;
  - l'absence de colonne pour le ratio de morts et pour les bornes des seaux supprime deux façons de se contredire ;
  - le pool a un seau pour 0 et un pour 1, donc la lecture ne confond plus « rien » et « un ».
- **Négatives / coûts** :
  - un percentile est **sur-estimé** d'au plus un seau (25 % en relatif pour la latence, facteur 2 pour le pool) : un p95 de 3,2 ms ne dit pas 3,2 ms ;
  - la forme d'un filtre perd la structure booléenne : `$not` et le filtre direct sont indiscernables, et c'est assumé — mais un lecteur qui chercherait « les filtres négatifs » ne les trouvera pas ;
  - `mode` et `dense_strategy` vides au lieu de `NULL` obligent à connaître la convention pour écrire une requête SQL à la main ;
  - quatre tables au lieu d'une : une lecture qui veut le compte et la forme d'une minute fait une jointure, et ROUTE-STATS doit la faire ;
  - la liste des colonnes d'histogrammes est **répétée neuf fois** : l'ajout d'une étape de pipeline demande une migration de `format_version`, pas un `ALTER` silencieux.
- **À surveiller** :
  - le test d'addition de la fiche (`docs/phases/P7:44`) : s'il ne tombe pas juste à un seau près, c'est la règle du rang ou la borne haute qui est fausse, pas le test ;
  - la fréquence réelle du seau 32 du pool : s'il se remplit, la conception du pool a changé et le seau ne dit plus rien ;
  - la taille de `filter_shapes` après quelques semaines de trafic : si elle dépasse celle de `minute_events`, la règle de canonicalisation laisse passer trop de variantes, et c'est la règle qu'il faut resserrer par un ADR ;
  - la version du fichier : la première évolution du schéma (une étape de plus, un compteur de plus) doit passer `format_version` à 2 et refuser la 1, jamais lire les deux.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Écrire les événements bruts dans SQLite et calculer les percentiles à la lecture | La rétention de 90 jours sur un trafic réel produirait des millions de lignes ; `docs/06:17` décide l'agrégation par minute, et `docs/06:42` en donne la raison : des seaux fixes s'additionnent. |
| Moyenner les percentiles de deux minutes pour obtenir celui de la période | Faux dès que les deux minutes n'ont pas le même nombre de requêtes, et explicitement interdit par `docs/06:42`. |
| Donner un percentile « exact » en gardant, par seau, la somme des valeurs | Il faudrait une somme flottante par seau et par étape, donc 9 × 64 flottants par ligne, et le résultat ne serait exact que pour la moyenne, pas pour le rang. La précision promise serait un chiffre faux de plus. |
| Seaux linéaires plutôt que géométriques | Une latence de 0,5 ms à 60 s demande soit des millions de seaux, soit une résolution grossière là où le trafic se concentre. Le géométrique donne une résolution **relative** constante, 25 %, ce qui est la bonne unité pour une latence. |
| Un seul seau de débordement commun à la latence et au pool | Les deux grandeurs n'ont ni la même unité ni la même échelle ; un seau commun obligerait à convertir l'une dans l'autre et à figer un facteur arbitraire dans le format. |
| Mettre `filter_shape` dans la clé de la ligne de minute | Une clé par filtre distinct : la table grossit avec la variété des filtres, la rétention ne purge presque rien, et le compte d'une minute n'est plus lisible. C'est l'écart que la tâche demande de trancher. |
| Garder la structure booléenne dans la forme (`and(f:a,f:b)`) | Exige une grammaire de plus dans une colonne qui sert à compter des fréquences, et sépare `f:a,f:b` de `f:b,f:a` selon l'ordre d'écriture du client : la cardinalité double pour aucune lecture. |
| Préfixer les feuilles niées (`!f:eq`) | Distingue `$not` de son complément, mais la valeur reste absente : `!f:ne` et `f:eq` deviennent deux formes pour un même filtre réel, et le classement se scinde. La structure n'est pas ce que la colonne mesure. |
| Stocker les bornes des seaux dans le fichier | Un flottant de plus par seau, donc un format plus long et une cohérence à tenir entre les bornes et les compteurs ; les bornes sont une propriété de `format_version`, elles se recalculent. |
| Utiliser `NULL` pour `mode` et `dense_strategy` d'une opération | SQLite accepte `NULL` dans la clé primaire d'une table `rowid`, donc deux lignes pour la même clé existeraient et les comptes se dédoubleraient sans erreur. |
| Laisser `crible-server` ouvrir `analytics.sqlite` avec `rusqlite` | Ajoute une arête que `docs/07:39` et ADR-0103 refusent déjà pour `jobs.sqlite` : `rusqlite` reste dans `crible-store`, et le serveur ne voit que des types. |
| Écrire la forme du filtre dans les logs plutôt qu'en base | Un log n'est pas interrogeable par `GET /v1/stats`, et `docs/06:58` demande les formes les plus fréquentes : c'est une lecture agrégée, donc une table. |

## Références

- `docs/phases/P7-observabilite.md:32-46` — fiche `AGREGER-ANALYTICS` ; `:37` (types sans contenu), `:38` (`filter_shape`), `:39` (agrégateur, 64 seaux, pool log2), `:40` (vidage), `:41` (lignes journalières), `:42` (purge), `:44` (critère d'addition « à la résolution d'un seau près »), `:45` (INV-17), `:46` (invariant cité).
- `docs/06-observabilite.md` : `:17` (clé de la ligne de minute), `:19` (neuf étapes), `:24` (`filter_shape`, exemple), `:26` (opérations et statut), `:28` (ligne journalière), `:30-37` (INV-13, sûreté par construction), `:41` (64 seaux `u32`), `:42` (addition, percentiles « exacts », jamais de moyenne), `:43` (33 seaux log2), `:47-50` (écriture, INV-17, rétention), `:56` (p50, p95, p99 lus), `:58` (formes de filtre les plus fréquentes).
- `docs/03-recherche.md:5-15` — clés de temps des étapes (`validate`, `encode`, `filter`, `dense`, `lexical`, `fusion`, `group`, `fetch`, `total`).
- `docs/02-architecture.md:53` (INV-01, aucun `iid` hors du moteur), `:60` (`analytics.sqlite` à la racine du dossier de données), `:62` (`jobs.sqlite`).
- `docs/07-stack-et-deploiement.md:29-50` (dépendances ; `rusqlite` ligne 39), `:86-87` (`[analytics].retention_days = 90`).
- `docs/08-invariants-et-tests.md:21` (INV-13), `:23` (INV-15), `:25` (INV-17), `:36` (niveau « Charge »).
- `docs/05-api.md:41-53` — codes d'erreur, qui sont aussi les catégories des analytics.
- `crates/crible-store/src/sqlite.rs:83-143` (`check_format_version` : quatre refus, INV-15) ; `crates/crible-store/src/jobs.rs:22-31` (propriété du fichier, aucune arête serveur → `rusqlite`) ; `crates/crible-store/src/jobs.rs:449` (`purge`).
- ADR-0014 (analytics sans contenu, journal de debug à la demande), ADR-0038 (`busy_timeout`), ADR-0074 (`exact` et `binary`), ADR-0077 (percentile au rang le plus proche), ADR-0100 (nom de la clé = dimension `client`), ADR-0103 (`jobs.sqlite` : fichier d'instance, `format_version`, rétention), ADR-0118 (exécution des analytics : file, vidage, preuve d'INV-17).

## Documents mis à jour par cet ADR

| Document | Modification |
|---|---|
| `docs/06-observabilite.md:17-24` | la forme du filtre renvoyée à la table `filter_shapes` (ADR-0117, décision 6) |
| `docs/06-observabilite.md:39-43` | bornes exactes des seaux de latence et de pool, règle du percentile et résolution (décisions 2 à 4) |
| `docs/06-observabilite.md:30-37` | renvoi à la liste fermée des champs des types (décision 5) |
| `docs/phases/P7-observabilite.md:32-46` | la fiche `AGREGER-ANALYTICS` renvoie à ADR-0117 pour le schéma, les seaux et la résolution du percentile, et à ADR-0118 pour INV-17 |
| `docs/adr/README.md` | ligne d'index de cet ADR |

Sont **non modifiés** : `docs/01` à `docs/05`, `docs/08` (ligne « Charge », dont la formulation reste vraie), `docs/suivi.md` (tenu par l'orchestrateur), et tout fichier de code.

Les numéros de ligne de `docs/06-observabilite.md` cités dans cet ADR sont ceux de son **état d'avant application** : la mise à jour de ce document décale les sections suivantes, et c'est la modification ci-dessus qui fait foi.
