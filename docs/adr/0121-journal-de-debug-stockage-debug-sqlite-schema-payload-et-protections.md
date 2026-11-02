# ADR-0121 — Journal de debug : stockage (`debug.sqlite`, schéma, payload, protections)

- **Statut** : Accepté (accepté par anticipation par le propriétaire le 2026-10-31, confirmé par le propriétaire le 2026-11-01 ; délégation du propriétaire pour la série de P7) — complété par ADR-0127 (plafond de taille de `debug.sqlite`, décision du propriétaire du 2026-11-02), sans modification du corps
- **Date** : 2026-10-31
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, tâche JOURNAL-DE-DEBUG (phase P7), sur les options recommandées par `@architecte` et retenues par l'orchestrateur, en application de la délégation du propriétaire du 2026-10-31 pour la série de P7
- **Remplace** : — (ne remplace aucune décision : applique ADR-0014, précise `docs/06`, et complète ADR-0119, décisions 8, 10 et 13, sans modifier le corps d'aucun ADR accepté)

## Contexte

### La fiche et ce qu'elle demande

La fiche `JOURNAL-DE-DEBUG` (`docs/phases/P7-observabilite.md:55-72`) demande un journal complet, **activé à la demande** : `debug.sqlite` (`:60`), un enregistrement selon `debug` et `sample_rate` (`:61`), une écriture asynchrone (`:62`), la purge selon `ttl_days` (`:63`), `GET /v1/debug/queries` réservé à l'admin (`:64`), le test `inv_17_*` du journal (`:65`) et le rejeu de MES-16 par `@mesureur` (`:66`).

Ses critères sont : désactivé par défaut, aucun fichier écrit (`:68`) ; la purge supprime les entrées expirées (`:69`) ; un rôle `read` reçoit `403` (`:70`) ; la route figure dans la spécification OpenAPI et son exception nommée quitte le test des routes dans le même changement (`:71`) ; le test `inv_17_*` est vert et le rapport de MES-16 existe et tient le critère d'ADR-0113 (`:72`). La fiche cite INV-17 (`:73`).

Cet ADR traite le **stockage** : fichier, schéma, contenu et protections. L'**exécution** (activation, capture, file, vidage, purge, route, preuves) est l'objet d'ADR-0122.

### Ce que la conception pose

`docs/02-architecture.md:60-62` place `debug.sqlite` à la racine du dossier de données, à côté d'`instance.json`, `analytics.sqlite` et `jobs.sqlite`. `docs/06-observabilite.md:5-11` oppose les deux composants : analytics toujours actifs, sans contenu, conservés 90 jours ; journal à la demande, avec la requête complète, conservé 7 jours ; les deux fichiers sont lus par un `admin`.

`docs/06:80-85` fixe le contenu : `request_id`, horodatage, index et client ; la requête complète, texte et filtre ; le plan et les temps par étape ; le top 20 (clé et score de chaque branche) ; la génération et l'erreur éventuelle. `docs/06:88` donne la raison des protections : les requêtes de Legifacile peuvent être sensibles, donc le journal est désactivé par défaut, sa durée de vie est courte, et il n'est lisible que par un admin.

`docs/05-api.md:76` déclare `GET /v1/debug/queries` avec le rôle `admin`, et `:270` dit que `debug` alimente le journal quand il est activé, l'écriture appartenant à JOURNAL-DE-DEBUG (ADR-0119, décision 13). `docs/07-stack-et-deploiement.md:89-92` fixe les trois clés de configuration et leurs défauts : `enabled = false`, `sample_rate = 0.0`, `ttl_days = 7`.

Les invariants qui pèsent sont INV-15 (`docs/08-invariants-et-tests.md:23`), qui exige un format sur disque versionné et le refus d'une version inconnue ; INV-01 (`docs/02:53`), qui interdit qu'un `iid` sorte du moteur ; et INV-17 (`docs/08:25`), qui porte sur l'écriture et relève d'ADR-0122. INV-13 (`docs/08:21`) ne vise que `analytics.sqlite` : le journal a précisément pour but de contenir le texte et le filtre, et ADR-0014 l'écrit ainsi.

Le précédent de fichier d'instance est double. ADR-0103 a créé `jobs.sqlite`, ouvert et écrit par `crible-store`, avec une table `meta` et `format_version = 1`. ADR-0117 a fait de même pour `analytics.sqlite`, avec quatre tables et le contrôle de `crates/crible-store/src/sqlite.rs:83-143` (`check_format_version`) réutilisé tel quel. `docs/07:39` ne déclare `rusqlite` qu'auprès du stockage : `crible-server` n'a aucune arête vers cette crate.

### Ce que cet ADR ne tranche pas

L'activation à deux voies, la capture, la file, le vidage, la purge, la route et les preuves sont d'ADR-0122. Cet ADR ne fixe ni l'ordre d'arrêt, ni la forme exacte de la réponse de la route, ni le rôle de `debug: true`.

## Décision

### Décision 1 — `debug.sqlite` à la racine du dossier de données, possédé par `crible-store`

**Décision.** Le journal vit dans **`data/debug.sqlite`**, à la racine du dossier de données (`docs/02:60-62`). Il est ouvert, lu et écrit par **`crible-store`**, dans un module propre ; `crible-server` ne gagne **aucune arête** vers `rusqlite`.

Précisions :

- c'est l'organisation d'ADR-0103 pour `jobs.sqlite` et d'ADR-0117 pour `analytics.sqlite` : `crible-store` porte déjà `rusqlite` (`docs/07:39`), et le serveur ne voit que des types ;
- le fichier est en **WAL**, avec le `busy_timeout` d'ADR-0038, comme `records.sqlite`, `jobs.sqlite` et `analytics.sqlite` ;
- le fichier n'est **ni ouvert ni créé** quand `debug_journal.enabled` vaut `false`. Un fichier laissé par une exécution précédente n'est pas purgé et ne rend pas le journal actif : l'état est donné par la configuration, pas par la présence du fichier ;
- quand le journal est activé et que le fichier est absent, il est créé avec sa table `meta` (décision 2) ; quand il existe, il est ouvert et contrôlé avant tout usage.

### Décision 2 — table `meta`, `format_version = 1`, contrôle réutilisé

**Décision.** Le fichier porte une table `meta` à une colonne `format_version`, qui vaut **1** pour ce build. À l'ouverture, le contrôle de `crates/crible-store/src/sqlite.rs:83-143` est **réutilisé tel quel**, avec le nom de fichier `debug journal`.

```sql
CREATE TABLE meta (
  format_version INTEGER NOT NULL          -- 1 pour ce build
);
```

Précisions :

- les refus du contrôle sont ceux d'INV-15 : table `meta` absente, ligne absente, valeur non entière ou hors du domaine de `u32`, version inconnue. Chacun nomme `format_version` et le fichier, comme pour `records` et `jobs` ;
- un fichier **neuf** est créé avec sa table et sa ligne ; un fichier **existant sans `meta`** est refusé, jamais complété en silence ;
- un refus à l'ouverture fait **échouer le démarrage** du serveur, avec un code de sortie non nul : une instance qui ne peut pas tenir son journal ne démarre pas en prétendant l'avoir activé ;
- `format_version` couvre le schéma **et la forme du payload** (décision 4) : un lecteur de la route analyse ce JSON, et changer la liste fermée sans changer la version produirait un lecteur qui interprète de travers. Toute évolution passe à la version suivante et refuse la précédente.

### Décision 3 — table `queries` et ses index

**Décision.** Les lignes du journal sont dans une seule table :

```sql
CREATE TABLE queries (
  id           INTEGER PRIMARY KEY,
  created_at   TEXT NOT NULL,              -- RFC 3339 UTC, lu par SQLite
  request_id   TEXT NOT NULL,
  "index"      TEXT,                      -- NULL pour une requête multi-index
  client       TEXT NOT NULL,             -- nom de la clé, jamais la clé
  mode         TEXT NOT NULL,             -- '' quand le mode n'est pas connu
  status       TEXT NOT NULL,             -- 'ok' ou un code d'erreur de l'API
  total_ms     REAL,                      -- NULL si fetch n'a pas fini
  payload      TEXT NOT NULL              -- objet JSON (décision 4)
);

CREATE INDEX queries_created_at ON queries (created_at);
CREATE INDEX queries_index      ON queries ("index");
CREATE INDEX queries_total_ms   ON queries (total_ms);
```

Précisions :

- `id` est la clé de pagination de la route (`before_id`, ADR-0122, décision 9). SQLite peut réutiliser le plus grand identifiant après une purge qui vide la table : en marche normale, la purge retire les lignes les plus anciennes et les identifiants croissent, mais un curseur de pagination n'est pas stable au-delà d'une purge complète. Aucune promesse d'`AUTOINCREMENT` n'est faite ici ;
- `created_at` est **rempli par SQLite** à l'insertion, par `strftime('%Y-%m-%dT%H:%M:%fZ','now')`. Aucune horloge calendaire n'entre dans le code : c'est la règle de `jobs.sqlite` (`crates/crible-store/src/jobs.rs:22-31`) et d'ADR-0118, décision 3. Le format est RFC 3339 en UTC, avec millisecondes, pour que l'ordre `created_at DESC, id DESC` reste total ;
- `request_id` est celui d'ADR-0098. C'est un identifiant de requête, pas un identifiant de document, et il ne viole pas INV-01 ;
- `"index"` est `NULL` pour **toute** requête passée par la route multi-index, même quand elle ne porte qu'une cible : la route est la dimension, pas le nombre de cibles (décision 4) ;
- `client` est le **nom** de la clé résolue (ADR-0100), jamais la clé, son haché ni l'en-tête `Authorization` ;
- `mode` est `NOT NULL`, la **chaîne vide** valant « mode inconnu », par exemple quand le corps a été refusé avant l'analyse du mode. C'est la convention d'ADR-0117, décision 1 : SQLite accepterait `NULL` dans une clé primaire `rowid`, et l'absence de valeur doit être explicite ;
- `status` vaut `ok` ou le **code d'erreur de l'API** (`docs/05:37-53`), comme la colonne de statut des analytics. Le statut HTTP n'est pas une colonne : il se déduit du code par le tableau de `docs/05` ;
- `total_ms` est la durée totale de la requête, **celle de `took_ms`**, arrêtée à la fin de `fetch` (ADR-0120, décision 1). Elle est `NULL` quand `fetch` n'a pas fini — un `503` pris au jeton, un `504` rendu pendant le calcul. Le filtre `min_total_ms` de la route exclut alors la ligne (ADR-0122, décision 9) ;
- `payload` est du texte JSON, jamais un BLOB ni une colonne par membre. Les trois index couvrent les seuls filtres de la route qui ne lisent pas le JSON : `created_at`, `"index"` et `total_ms`.

### Décision 4 — payload : une liste fermée de membres

**Décision.** Le payload est un **objet JSON** dont les membres sont exactement les suivants, et **aucun autre**.

| Membre | Contenu |
|---|---|
| `request` | le texte `query`, le `filter` **tel que reçu**, le `mode`, `k`, `exact`, `fusion`, `group_by`, `passages_per_document`, `dedup`, `return` |
| `plan` | le plan d'ADR-0119, décision 4, repris de l'objet `debug` |
| `timings_ms` | les neuf temps d'ADR-0119, décision 5, repris de l'objet `debug` |
| `capture` | le top 20 par branche, entrées `(key, rank, score)` (ADR-0119, décision 8) |
| `targets[]` | en multi-index, une entrée par cible : `index`, `filter`, `generation`, `capture` |
| `generation` | la génération de l'index ; en multi-index, la génération de chaque cible dans `targets[]` |
| `error` | le code d'erreur seul, sous la forme `{ "code": "…" }` |

Précisions :

- **`vector` n'est jamais recopié.** Le payload enregistre seulement sa **présence** et sa **dimension**, par exemple `{ "vector": { "present": true, "dimension": 1024 } }`. Les octets d'un vecteur sont une donnée du client, volumineuse et inutile au diagnostic : le plan et les temps suffisent ;
- **`filter` est le filtre tel que reçu**, sans canonicalisation ni réduction aux champs et opérateurs. C'est l'inverse d'`INV-13` et c'est le but du journal : `docs/06:80-85` demande « la requête complète (texte et filtre) », et `docs/06:88` assume le risque, borné par l'activation, la durée de vie et le rôle `admin` ;
- **`plan` et `timings_ms` ne sont pas recalculés.** Ils sont produits par le même code que l'objet `debug` d'ADR-0119, puis consommés par l'écrivain du journal. Une requête seulement échantillonnée les produit pour le journal sans les rendre dans la réponse, le membre `debug` restant conditionné au seul `debug` (ADR-0119, décision 10 ; ADR-0122, décision 3) ;
- **la capture est le top 20 de chaque branche**, `key` étant la clé externe `doc_id#ord`. Aucun `iid` n'y figure (INV-01). Une branche qui n'a pas tourné n'a pas d'entrée : on omet, on n'écrit ni `null` ni tableau vide (ADR-0119, décision 3) ;
- **en multi-index, `request.filter` est absent** : chaque cible porte son propre `filter` dans `targets[]` (ADR-0110). Le reste de `request` est commun à la requête. `targets[]` est dans l'ordre de la requête ;
- **`error` ne porte que le code.** Ni message, ni `details`, ni `request_id` redondant : le message d'une erreur `internal` peut contenir un chemin du serveur (ADR-0101), et le journal n'a pas à le conserver sept jours. Le `request_id` est déjà une colonne ;
- **la liste est fermée** : `timeout_ms`, `debug` et `explain_ids` n'entrent pas dans le payload. Un membre ajouté plus tard demande un ADR et une version de format (décision 2) ;
- **un membre absent n'est jamais `null`.** Quand le corps n'a pas pu être analysé, `request` peut manquer ; quand aucune capture n'a été construite, `capture` manque ; quand aucune branche n'a tourné, `plan` et `timings_ms` manquent. C'est la règle d'ADR-0119, décision 3, appliquée au journal.

### Décision 5 — sérialisation par le serveur, avec `serde_json`, sans toucher à `crible-search`

**Décision.** Le payload est construit et sérialisé par **`crible-server`**, avec `serde_json`, déjà présent (ADR-0027). Aucun type de `crible-search` ne reçoit `Serialize`, et **aucune dépendance n'est ajoutée**.

Précisions :

- `crible-search` ne connaît ni SQLite ni le journal : il expose `DebugCapture`, `CapturedEntry` et les listes de candidats, que le serveur lit pour former l'objet JSON ;
- `serde_json::Value` et la macro `json!` suffisent : il n'y a pas de type de payload à dériver, donc pas de `Serialize` à ajouter à un type du moteur ;
- `crible-store` reçoit le payload comme une **chaîne déjà sérialisée** et l'écrit dans `payload` : il ne connaît pas la forme du JSON, ce qui garde la frontière entre stockage et API ;
- aucun crate d'horloge n'est ajouté : `created_at` est lu par SQLite (décision 3).

### Décision 6 — protections : contenu assumé, lecture `admin`, durée courte, aucun log

**Décision.** Le journal **contient** le texte et le filtre, par conception. Quatre protections bornent ce contenu : il est **désactivé par défaut**, sa lecture est **réservée à `admin`**, sa durée de vie est **courte** (`ttl_days`, 7 par défaut), et **aucun contenu du journal n'entre dans les logs, à aucun niveau**.

Précisions :

- **INV-13 ne s'applique pas au journal.** `docs/08:21` ne nomme que les analytics, et la section « Ce qu'on ne stocke jamais » de `docs/06:36-41` est dans la partie Analytics. Le journal est l'autre composant d'ADR-0014, celui qui existe pour retrouver une requête précise (`docs/06:88`) ;
- **lecture `admin` seule** : `GET /v1/debug/queries` rend `403 forbidden` à une clé `read` ou `write` (ADR-0095 ; ADR-0122, décision 9). Une clé qui écrit le journal ne peut donc pas le relire ;
- **`ttl_days` court** : 7 par défaut (`docs/07:91`), et `0` est refusé au démarrage (ADR-0122, décision 8). Une valeur qui viderait le journal à chaque purge rendrait l'activation mensongère ;
- **fichier absent quand le journal est désactivé** (décision 1) : aucune trace n'est laissée par une instance qui n'a jamais activé le journal ;
- **aucun contenu dans les logs** : ni `query`, ni `filter`, ni la clé, ni le payload, ni un extrait, à aucun niveau et quel que soit `RUST_LOG` (`docs/06:128-129`, ADR-0096). Les seules lignes du journal sont des `warn` de comptage : pertes de file, échec de vidage, échec de purge (ADR-0122, décision 10) ;
- **le nom de la clé est permis**, la clé, son haché et l'en-tête ne le sont pas (ADR-0100). Un `iid` n'entre nulle part (INV-01).

## Conséquences

- **Positives** :
  - le journal a un format sur disque **versionné** et refusé quand la version est inconnue, avec le même contrôle que `jobs.sqlite` et `analytics.sqlite` : INV-15 est tenu par un mécanisme déjà éprouvé ;
  - la frontière est nette : `crible-store` possède le fichier, `crible-server` possède la forme du payload, et `crible-search` ne sait rien du journal ;
  - le payload est **fermé** : une fuite de contenu supplémentaire demande de changer un ADR et la version, pas d'oublier un champ ;
  - le texte et le filtre sont conservés **là où ils servent**, avec trois protections cumulées (désactivation, `admin`, durée courte) au lieu d'être dispersés dans les logs ;
  - le vecteur n'est pas recopié : le journal reste lisible sans stocker une donnée volumineuse et client ;
  - les trois index couvrent les filtres de la route qui ne lisent pas le JSON, donc la consultation reste bornée par la pagination.
- **Négatives / coûts** :
  - le payload JSON n'est **pas interrogeable par champ** en SQL : `min_total_ms`, `index` et les dates ont une colonne, le reste demande de lire et d'analyser le JSON. C'est le prix d'une liste fermée qui évolue avec `format_version` ;
  - une requête multi-index à N cibles écrit un payload qui contient N filtres et N captures : la ligne est plus grosse qu'une ligne simple, et la borne de 4 096 entrées de file compte des requêtes, pas des octets ;
  - **le contenu est en clair dans le fichier** : quiconque lit `debug.sqlite` avec un accès disque lit les requêtes. C'est le choix d'ADR-0014, et il n'est acceptable que parce que le journal est désactivé par défaut et lu par un `admin` ;
  - `created_at` est l'heure du **vidage**, pas celle de la requête, comme la minute des analytics : l'horodatage peut glisser de moins de 10 s, et une ligne lue par `from`/`to` peut changer de borne selon l'intervalle de vidage ;
  - `total_ms` est `NULL` pour les requêtes qui n'ont pas fini `fetch` : une lecture par `min_total_ms` ne les voit pas, et il faut une lecture sans ce filtre pour les compter ;
  - toute évolution de la forme du payload demande une version de format et un refus de l'ancienne, donc une migration explicite.
- **À surveiller** :
  - la taille de `debug.sqlite` quand `sample_rate = 1.0` : c'est le pire cas de MES-16, et une instance laissée dans cette configuration remplit le disque à la vitesse du trafic ;
  - la première ligne `warn` de `debug_journal_dropped` : elle dit un écrivain bloqué (disque, verrou SQLite), pas un pic de trafic normal (ADR-0122, décision 6) ;
  - la proportion de lignes à `total_ms` `NULL` : si elle devient forte, c'est que les `503` et les `504` dominent, et le journal mesure alors des échecs plus que des requêtes ;
  - la forme du payload après quelques semaines : un membre qui grossit sans borne (une capture plus profonde, un filtre très profond) doit se voir dans la taille des lignes, et se corriger par un ADR ;
  - la version du fichier : la première évolution du payload ou du schéma passe `format_version` à 2 et refuse la 1, jamais ne lit les deux.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Ouvrir `debug.sqlite` depuis `crible-server` avec `rusqlite` | Ajoute l'arête que `docs/07:39`, ADR-0103 et ADR-0117 refusent déjà : `rusqlite` reste dans `crible-store`, et le serveur ne voit que des types. |
| Réutiliser `analytics.sqlite` pour le journal | Mêle un fichier toujours actif, sans contenu et conservé 90 jours, à un fichier optionnel, avec contenu et conservé 7 jours : les deux politiques de `docs/06:5-11` deviendraient une seule, et INV-13 serait impossible à tenir par construction. |
| Une table par membre (`request`, `plan`, `capture`, `error`) | Beaucoup de tables, de clés et de jointures pour un objet qui se lit toujours entier ; la liste fermée est plus simple à versionner et à sérialiser. |
| Une colonne SQL par membre du payload | Fige la forme dans le schéma et multiplie les migrations ; le payload est un document de diagnostic, pas une table à interroger champ par champ. |
| Stocker le payload en BLOB avec un format binaire maison | Le JSON est déjà la forme de l'API et de `serde_json` (ADR-0027) ; un format binaire demanderait un encodeur, un décodeur et une version de plus, pour un fichier que seul le serveur relit. |
| Recopier le vecteur de la requête | Donnée volumineuse et sensible, sans valeur pour le diagnostic : le plan, les temps et la présence avec dimension suffisent, et le journal reste petit. |
| Hasher le texte ou le filtre | ADR-0014 a écarté le hachage : une requête courte comme « article 1240 » se retrouve par dictionnaire. Un journal qui hasherait ne servirait à rien. |
| Ne pas versionner `format_version` | INV-15 l'interdit, et la route analyse le JSON : un lecteur d'une autre version interpréterait de travers sans le dire. |
| Écrire `created_at` depuis une horloge Rust | Aucune crate d'horloge calendaire n'est dans `docs/07:29-50`, et `jobs.sqlite` a déjà la règle « le code ne porte pas d'horloge ». |
| Laisser le fichier en place quand le journal est désactivé | Une instance désactivée laisserait un fichier de requêtes en clair, et la présence du fichier ferait croire que le journal est actif : la configuration seule décide. |
| Exposer `query`, `filter` ou le payload dans une ligne de log `info` par requête | Recopie le contenu hors de `debug.sqlite`, où la durée de vie et le rôle `admin` ne s'appliquent plus ; `docs/06:128-129` et ADR-0096 l'interdisent à tout niveau. |

## Références

- `docs/phases/P7-observabilite.md` : `:55-72` (fiche `JOURNAL-DE-DEBUG` ; `:60` `debug.sqlite`, `:61` enregistrement, `:62` écriture asynchrone, `:63` purge, `:64` route, `:65` test `inv_17_*`, `:66` MES-16, `:68-72` critères, `:73` INV-17).
- `docs/06-observabilite.md` : `:5-11` (tableau analytics / journal), `:36-41` (INV-13, analytics), `:74-88` (journal de debug : config, contenu, consultation, écriture, raison), `:124-129` (santé et logs).
- `docs/02-architecture.md:53` (INV-01), `:60-62` (`analytics.sqlite`, `debug.sqlite`, `jobs.sqlite` à la racine du dossier de données).
- `docs/05-api.md:37-53` (codes d'erreur et statuts HTTP), `:76` (`GET /v1/debug/queries`, rôle `admin`), `:270` (`debug` alimente le journal quand il est activé).
- `docs/07-stack-et-deploiement.md:29-50` (dépendances ; `rusqlite` ligne 39, `serde_json` ligne 48), `:89-92` (`[debug_journal]`), `:121` (ordre d'arrêt).
- `docs/08-invariants-et-tests.md:9` (INV-01), `:21` (INV-13), `:23` (INV-15), `:25` (INV-17).
- `crates/crible-store/src/sqlite.rs:83-143` (`check_format_version` : quatre refus, INV-15) ; `crates/crible-store/src/jobs.rs:22-31` (le code ne porte pas d'horloge) ; `crates/crible-server/src/config.rs:331-337` (domaine de `sample_rate`) ; `crates/crible-server/src/debug.rs:160-178` (`SearchDebug` : `plan` et `timings_ms`) ; `crates/crible-search/src/debug.rs:38` (`DEFAULT_TOP_N = 20`) et `:53-108` (`DebugCapture`, `CapturedEntry`).
- ADR-0014 (analytics sans contenu, journal de debug à la demande), ADR-0027 (`serde_json`), ADR-0038 (`busy_timeout`), ADR-0095 (rôle `admin` par famille de routes), ADR-0096 (aucun log de contenu), ADR-0098 (`x-request-id`), ADR-0100 (nom de la clé), ADR-0103 (`jobs.sqlite` : fichier d'instance, `format_version`), ADR-0117 (`analytics.sqlite`, même propriété et même contrôle), ADR-0119 (décisions 3, 4, 5, 8, 10 et 13 : plan, temps, capture, réponse, renvoi), ADR-0120 (décision 1 : fin de `took_ms`), ADR-0122 (exécution du journal).

## Documents mis à jour par cet ADR

| Document | Modification |
|---|---|
| `docs/06-observabilite.md:80-88` | contenu du journal : `request_id`, horodatage, index, client, requête complète, plan et temps, top 20, génération, erreur (code seul), et en multi-index `targets[]` ; renvoi à ADR-0121 pour le fichier, le schéma et le payload |
| `docs/05-api.md:76` | la ligne de la route renvoie à ADR-0121 et ADR-0122, et dit que la route rend `200` avec `enabled: false` quand le journal est désactivé |
| `docs/07-stack-et-deploiement.md:89-92` | les trois clés de `[debug_journal]` renvoient à ADR-0121 et ADR-0122 |
| `docs/phases/P7-observabilite.md:55-72` | la fiche `JOURNAL-DE-DEBUG` renvoie à ADR-0121 pour le stockage et à ADR-0122 pour l'exécution |
| `docs/adr/README.md` | ligne d'index de cet ADR |

Sont **non modifiés** : `docs/01` à `docs/04`, `docs/08` (INV-13 ne vise que les analytics, et sa formulation reste vraie), `docs/suivi.md` (tenu par l'orchestrateur), et tout fichier de code à ce stade : le code est livré par JOURNAL-DE-DEBUG.

Les numéros de ligne de `docs/06-observabilite.md` cités dans cet ADR sont ceux de son **état d'avant application** : la mise à jour de ce document décale les sections suivantes, et c'est la modification ci-dessus qui fait foi.
