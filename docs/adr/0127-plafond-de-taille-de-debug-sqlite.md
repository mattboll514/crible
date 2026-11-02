# ADR-0127 — Plafond de taille de `debug.sqlite` : deux bornes, trim au vidage et récupération incrémentale

- **Statut** : Accepté (décidé par le propriétaire le 2026-11-02, avec les recommandations de l'ADR : valeurs par défaut `max_rows = 500000` et `max_bytes = 2147483648`, membres `capped`, `rows`, `bytes` et `trimmed` sur `GET /v1/debug/queries`, tâche `PLAFONNER-JOURNAL-DE-DEBUG` hors plan en P8)
- **Date** : 2026-11-02 (proposition et acceptation le même jour)
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, sur demande du propriétaire du 2026-11-02 (« tu peux faire un adr pour le plafond de taille de debug.sqlite »), à partir de l'analyse de `@architecte` du 2026-11-02 rendue au contrôle de fin de P7 (verdict `NON-COUVERT`, tâche `NETTOYER-P7`)
- **Remplace** : — (complète ADR-0121 et ADR-0122 par renvoi, sans réécrire leur corps ; ne remplace aucune décision)

## Contexte

### Ce qui existe

Le journal de debug écrit dans `data/debug.sqlite` (`crates/crible-store/src/debug_journal.rs:140-160`), ouvert par `crible-store` seulement quand `debug_journal.enabled` est vrai (ADR-0121, décision 1). Le fichier est en WAL, avec `busy_timeout` de 1 s, une table `meta` à `format_version = 1`, une seule connexion sous `Mutex` et un seul écrivain.

Le chemin d'une requête ne touche jamais SQLite : le tirage `should_sample` a lieu avant la prise du jeton, `debug: true` est toujours enregistré quand le journal est actif, et la ligne part par `try_send` dans un canal borné à 4 096 entrées (ADR-0122, décisions 1 et 6). L'écrivain est une tâche tokio (`crates/crible-server/src/debug_journal.rs:367-430`), qui vide toutes les 10 s (`FLUSH_INTERVAL`, `:76`) et à l'arrêt, chaque vidage tenant dans une transaction exécutée en `spawn_blocking` (`Flush::run`, `:472-510`).

La purge de rétention a lieu à l'ouverture, puis au premier vidage dont la dernière purge date de plus de 24 h (`PURGE_INTERVAL`, `:82`). Elle exécute `DELETE FROM queries WHERE created_at < …'-<ttl_days> days'` (`crates/crible-store/src/debug_journal.rs:220-241`), et `ttl_days = 0` est refusé au démarrage (`crates/crible-server/src/config.rs:338-347` ; ADR-0122, décision 8).

### Ce que les ADR acceptés bornent, et ce qu'ils ne bornent pas

ADR-0121, décision 6, et ADR-0122, décision 8, bornent la **durée** de conservation, par `ttl_days` (7 jours par défaut, `docs/07-stack-et-deploiement.md:89-92`). Aucun ADR accepté ne borne la **taille** du fichier : les deux le disent seulement dans « À surveiller » (ADR-0121 `:165`, ADR-0122 `:285`). La consigne d'exploitation d'ADR-0122, décision 2, « un `sample_rate` bas en production » est un conseil, pas un mécanisme.

Trois faits du code rendent cette absence sensible :

- **aucun `VACUUM` ni `auto_vacuum`** n'est posé sur `debug.sqlite` (le seul `VACUUM` du dépôt est le `VACUUM INTO` d'un instantané, `crates/crible-store/src/snapshot.rs:1009`). Les pages libérées par une purge sont réutilisées, mais le fichier ne redescend jamais sous son plus haut niveau : la purge de rétention ne réduit pas la place occupée sur le disque ;
- **un disque plein ne remonte pas comme tel** : `SQLITE_FULL` tombe dans la branche par défaut de `database_error` et devient `Internal` (`crates/crible-store/src/records.rs:1296-1304`), le vidage échoue avec une ligne `warn` qui compte les lignes perdues et sans reprise (`crates/crible-server/src/debug_journal.rs:475-485`), `debug_journal_dropped` ne compte que les refus de file, et `/ready` ne regarde pas le journal (`crates/crible-server/src/http.rs:632-641`, ADR-0099) ;
- **une ligne peut être grosse** : le payload porte la requête et sa capture, et la limite de corps est de 32 Mio (ADR-0102), donc un plafond exprimé en lignes seules ne borne pas les octets.

### Les chiffres, et ce qui n'est pas mesuré

MES-16 mesure **3,48 à 3,54 Ko par ligne**, soit 15,25 à 15,48 Mio pour environ 4 600 lignes, à `sample_rate = 1.0` sur un jeu synthétique de 50 000 passages (`docs/mesures/MES-16-charge-concurrente-analytics-et-journal.md:201-215`, `:338`). Le même rapport relève 49,7 ; 49,1 et 49,3 recherches par seconde en régime de repos (`:267`) et avertit que rien ne dit ce que vaudrait la taille d'une ligne sur Legifacile, dont les textes et les filtres sont réels (`:338-348`).

Les débits ci-dessous sont une **hypothèse de travail, non mesurée** : ils prennent 3,6 Ko par ligne, haut de la fourchette de MES-16, et `sample_rate = 1.0`, pire cas de configuration. Ils ne valent pas prévision pour Legifacile ; ils disent seulement l'ordre de grandeur du risque.

| Débit supposé | Par jour | Sur 7 jours (`ttl_days`) |
|---|---|---|
| 0,5 requête/s | ≈ 148 Mio | ≈ 1,0 Gio |
| 1 requête/s | ≈ 295 Mio | ≈ 2,0 Gio |
| 5 requêtes/s | ≈ 1,44 Gio | ≈ 10,1 Gio |
| 20 requêtes/s | ≈ 5,8 Gio | ≈ 40 Gio |
| 49 requêtes/s (débit de MES-16) | ≈ 14,1 Gio | ≈ 99 Gio |

### La question

Il manque une **borne de disque**, indépendante de `sample_rate`, de `ttl_days` et de la discipline d'exploitation. Sans elle, le journal peut remplir le disque d'une instance, et le seul signal serait une perte silencieuse au moment où SQLite ne peut plus écrire. Cet ADR propose cette borne ; il ne change ni le contenu du journal, ni son schéma, ni le régime de la purge de durée.

## Décision

**Décision acceptée par le propriétaire le 2026-11-02.** Les valeurs par défaut, les membres de la route et la place de la tâche d'implémentation sont tranchés dans la section « Décision du propriétaire du 2026-11-02 » ; le corps ci-dessous est celui qui a été accepté.

### Décision 1 — deux plafonds dans `[debug_journal]`, `0` refusé au démarrage

**Décision.** Deux clés de configuration s'ajoutent à `[debug_journal]` : `max_rows` et `max_bytes`, avec `max_rows = 500000` et `max_bytes = 2147483648` (2 Gio) par défaut, dans le gabarit de `docs/07`. Le contrôle du premier plafond atteint déclenche le trim. `0` est refusé au démarrage pour les deux clés, comme `ttl_days` l'est déjà : un plafond nul recréerait exactement le mode non borné que cet ADR supprime.

Précisions :

- la structure `DebugJournalConfig` (`crates/crible-server/src/config.rs:189-196`) reçoit les deux champs, et la liste fermée des clés de la section (`:329`) les déclare : une clé inconnue reste refusée (ADR-0031) ;
- validation au démarrage : `max_rows >= 1` et `max_bytes >= 1 Mio`. Le minimum d'un Mio laisse passer une ligne courante tout en refusant une valeur qui viderait le journal à chaque vidage ; une ligne plus grosse que le plafond est supprimée par le trim, ce qui est le comportement voulu et testé ;
- les surcharges d'ADR-0097 s'appliquent sans mécanisme nouveau : `CRIBLE_DEBUG_JOURNAL__MAX_ROWS` et `CRIBLE_DEBUG_JOURNAL__MAX_BYTES`, analysées comme des scalaires ;
- deux clés plutôt qu'une : le nombre de lignes borne le coût de la route de lecture et de la purge, les octets bornent le disque, et aucun des deux ne dérive de l'autre quand la taille d'une ligne varie ;
- les deux plafonds sont **indépendants** : le premier atteint déclenche le trim, et un trim pour `max_bytes` peut laisser `max_rows` très en dessous de sa borne, comme l'inverse.

### Décision 2 — `max_bytes` porte sur la taille utilisée, `file_bytes` sur la taille physique

**Décision.** `max_bytes` compare la **taille utilisée** de `debug.sqlite` : `(page_count - freelist_count) × page_size`, lue par les pragmas de SQLite. La **taille physique** du fichier — pages libres et WAL exclus — est une seconde valeur, `file_bytes`, rapportée à l'opérateur.

Précisions :

- la taille utilisée est la seule que le trim peut faire baisser sans `VACUUM` : elle compte les pages réellement occupées, et la suppression des lignes les plus anciennes la réduit à mesure que les pages passent à la liste libre ;
- la taille physique, elle, ne baisse qu'avec la récupération incrémentale de la décision 5 : elle peut rester au-dessus de `max_bytes` après un trim, et ce n'est pas un dépassement du plafond ;
- le WAL n'entre dans aucune des deux valeurs : il est vidé par les points de contrôle de SQLite et n'appartient pas au fichier borné ;
- `file_bytes` est écrit dans le `warn` de plafond et dans la documentation d'exploitation, jamais comme membre de la route (décision 6) : l'opérateur qui veut la taille physique d'un journal au repos la lit avec les outils de son système, ce qui ne demande aucune API.

### Décision 3 — au plafond, supprimer les lignes les plus anciennes dans la transaction du vidage

**Décision.** Quand un vidage trouve le journal au-dessus de `max_rows` ou de `max_bytes`, la même transaction supprime les lignes les plus anciennes — ordre `created_at ASC, id ASC` — jusqu'à repasser sous les deux plafonds. Elle compte les lignes supprimées dans `trimmed` et arme `capped`.

Précisions :

- l'insertion du vidage précède le trim, dans la même transaction : une ligne reçue n'est jamais perdue par le trim du même vidage ;
- l'ordre du temps est celui de la purge de rétention : `created_at` puis `id`, l'ordre total d'ADR-0122, décision 9 ;
- le dépassement est borné par ce qu'un vidage peut apporter, soit 10 s de trafic (`FLUSH_INTERVAL`) plus la taille de file : à 49 requêtes/s supposées, environ 490 lignes, soit quelques Mio ;
- `capped` vaut vrai quand le dernier contrôle a trouvé le journal au plafond, et reste vrai jusqu'au démarrage suivant ; `trimmed` est le nombre de lignes supprimées par le plafond depuis le démarrage. Les deux sont des compteurs de processus, comme `debug_journal_dropped`, et ne sont pas persistés ;
- la suppression se fait par lots d'`id`, jamais par un balayage complet hors index : la table porte déjà l'index sur `created_at` (ADR-0121, décision 3) ;
- un trim qui ne tient pas dans le délai d'un vidage d'arrêt est abandonné avec ce vidage : le fichier reste lisible, le dépassement est borné, et le contrôle repart de la taille réelle au démarrage suivant.

### Décision 4 — contrôle à chaque vidage et une fois à l'ouverture

**Décision.** Le plafond est contrôlé à **chaque vidage**, dans la transaction, et **une fois à l'ouverture**, après la purge de rétention. Aucun autre moment n'est ajouté.

Précisions :

- chaque vidage est déjà le point où le journal est écrit : y mettre le contrôle ne crée ni minuteur ni tâche ;
- le contrôle à l'ouverture rattrape un fichier laissé au-dessus du plafond par un arrêt brutal, une baisse de configuration entre deux démarrages, ou un ancien binaire sans plafond ;
- un contrôle par ligne serait payé à chaque insertion pour aucun gain, la transaction du vidage bornant déjà le dépassement ;
- le tick de 24 h de la purge serait trop tard : un pic de trafic d'une heure peut dépasser le plafond de plusieurs jours de rétention.

### Décision 5 — pas de `VACUUM` complet ; `auto_vacuum = INCREMENTAL` à la création et récupération bornée

**Décision.** Aucun `VACUUM` complet n'est exécuté par le serveur. Un fichier **neuf** est créé avec `auto_vacuum = INCREMENTAL`, et une récupération `incremental_vacuum` **bornée** suit une purge ou un trim, dans la même tâche bloquante, hors de la transaction. Un fichier existant sans ce mode garde son plus haut niveau ; un `VACUUM` hors ligne, journal désactivé, est documenté pour l'opérateur.

Précisions :

- `auto_vacuum` ne s'active que sur un fichier neuf ou par un `VACUUM` : c'est pourquoi la décision ne l'impose pas aux fichiers existants, et pourquoi la documentation d'exploitation donne la procédure ;
- le `VACUUM` complet est écarté : il demande jusqu'au double de la place, bloque la connexion et échoue précisément quand le disque est plein, c'est-à-dire dans le cas où on en aurait besoin ;
- `auto_vacuum = FULL` est écarté : il déplace des pages à chaque transaction, donc sur le chemin de chaque vidage, pour un gain que la récupération bornée donne sans ce coût ;
- la récupération est bornée par un budget de pages par vidage, fixé à 256 pages (1 Mio à 4 Kio de page) : un vidage ne peut pas être monopolisé par la récupération, et le fichier redescend sur plusieurs vidages ;
- sans récupération, un trim réduit la taille utilisée mais jamais la taille physique : c'est suffisant pour ne pas dépasser le plafond logique, insuffisant pour rendre la place au système, d'où la récupération bornée ;
- la récupération n'a pas lieu dans la transaction : elle prend le verrou d'écriture pour elle seule, et un échec est journalisé en `warn` sans reprise, comme la purge.

### Décision 6 — visibilité : `warn` de comptage et quatre membres sur `GET /v1/debug/queries`

**Décision.** Le plafond se voit par un `warn` de comptage, au plus une fois par intervalle de purge, et par quatre membres ajoutés à `GET /v1/debug/queries` : `capped`, `rows`, `bytes` et `trimmed`. Ni `GET /v1/stats`, ni `/ready` n'en portent rien.

Précisions :

- le `warn` porte des nombres seulement — `rows`, `bytes`, `file_bytes`, `trimmed` — et aucun texte de requête, aucun filtre, aucune clé : la règle d'ADR-0122, décision 10, n'est pas entamée ;
- `rows` est le nombre de lignes du journal, `bytes` la taille utilisée de la décision 2, `trimmed` le compteur de la décision 3, `capped` le drapeau de la décision 3 ;
- la route reste un objet **fermé** (ADR-0111) : les quatre membres sont déclarés dans le composant `DebugQueryList` et dans `docs/05`, avec leur exemple ; un client d'avant les ignore ou les lit, mais le schéma dit qu'ils existent toujours ;
- `GET /v1/stats` est écarté : sa réponse est un objet fermé de seize décisions (ADR-0124, décision 1), et le journal n'est pas un analytics ; y ajouter un membre obligerait à rouvrir cet ADR pour un compteur de fichier ;
- `/ready` est écarté : un journal au plafond est un état normal de fonctionnement, pas une indisponibilité (ADR-0099) ;
- rien n'est ajouté à `crible stats` : la CLI lit `analytics.sqlite`, pas `debug.sqlite`.

### Décision 7 — `ttl_days` reste la borne de durée, le plafond ne peut que raccourcir la rétention

**Décision.** La rétention effective est le **minimum** de `ttl_days` et de ce que les plafonds laissent tenir : la purge de durée précède le trim, et le plafond peut raccourcir la rétention, jamais l'allonger.

Précisions :

- `ttl_days` reste la borne de **confidentialité** : c'est elle qui dit combien de temps une requête peut rester sur le disque, et le plafond ne la dépasse pas ;
- le plafond est la borne de **disque** : il ne s'exprime ni en jours ni en taux de tirage, donc il tient même si `sample_rate` est mal réglé ;
- une ligne plus récente que `ttl_days` peut être supprimée par le plafond : c'est le prix de la borne de disque, et `capped` le dit ;
- aucune clé ne permet de désactiver le plafond : une instance qui veut un journal non borné est un cas d'exploitation, pas un réglage de production.

### Ce que cet ADR ne décide pas

Le schéma de `debug.sqlite` et son `format_version = 1` ne changent pas : aucun membre nouveau n'est persisté, et les compteurs vivent en mémoire (INV-15 tenu sans migration). Le contenu du payload, la forme de la route et son rôle `admin` restent ceux d'ADR-0121 et d'ADR-0122. La question d'un plafond du même ordre pour `analytics.sqlite` n'est pas ouverte ici.

### Tests attendus

- `inv_17_debug_journal_cap_never_blocks`, sous la feature `failpoints` du serveur, sur le modèle d'`inv_17_debug_journal_never_blocks` (ADR-0122, décision 11) : un trim au plafond ne bloque et n'allonge aucune requête, et les réponses restent celles d'un journal vide (INV-17) ;
- plafond par lignes : au-delà de `max_rows`, les lignes les plus anciennes sont supprimées et les plus récentes gardées ;
- plafond par octets : même contrôle avec des lignes grosses, et une ligne plus grosse que `max_bytes`, supprimée par le trim ;
- les deux plafonds ensemble, et le cas où un trim pour l'un laisse l'autre loin de sa borne ;
- ordre du trim : les plus anciennes d'abord, `created_at` égal départagé par `id` ;
- purge et plafond dans le même vidage : la purge de durée passe d'abord, le trim ensuite, et `trimmed` ne compte que le second ;
- `auto_vacuum` incrémental sur un fichier neuf : le pragma vaut `2`, et `incremental_vacuum` fait baisser `freelist_count` sans changer le nombre de lignes ;
- compteurs et route : `capped`, `rows`, `bytes` et `trimmed` justes, et le schéma `DebugQueryList` reste fermé (ADR-0111) ;
- `format_version` reste `1` : un fichier écrit par la version bornée est relu par la version précédente, et inversement (INV-15) ;
- le `warn` de plafond ne porte aucun texte de requête, aucun filtre et aucune clé : des nombres seulement (ADR-0122, décision 10 ; INV-13 ne vise que `analytics.sqlite`).

## Décision du propriétaire du 2026-11-02

Le propriétaire a accepté l'ADR le 2026-11-02, avec les recommandations. Les trois points qui étaient laissés ouverts sont tranchés comme suit.

### (a) Valeurs par défaut

`max_rows = 500000` et `max_bytes = 2147483648` (2 Gio), retenus. Le tableau ci-dessous garde les deux autres couples comme référence de configuration : ils restent jouables sans changement de code.

| Couple | Taille utilisée à 3,5 Ko par ligne | Rétention à 1 requête/s, `sample_rate = 1.0` | Ce qu'il change |
|---|---|---|---|
| 250 000 lignes et 1 Gio | ≈ 0,85 Gio | ≈ 2,9 jours | marge disque plus large, rétention plus courte que `ttl_days` |
| **500 000 lignes et 2 Gio (retenu)** | ≈ 1,7 Gio, donc sous le plafond d'octets | ≈ 5,8 jours | proche des 7 jours de `ttl_days` à 1 requête/s ; le plafond mord vers 1,2 jour à 5 requêtes/s |
| 1 000 000 lignes et 4 Gio | ≈ 3,3 Gio | `ttl_days` mord d'abord | plafond rarement atteint, 4 Gio sur un hôte de 16 Go |

Le couple retenu laisse le plafond d'octets inerte tant que les lignes font 3,5 Ko, et le rend actif dès qu'elles grossissent : c'est le comportement voulu d'une borne de disque.

### (b) Les membres ajoutés à `GET /v1/debug/queries`

`capped`, `rows`, `bytes` et `trimmed` sont retenus, comme la décision 6. `file_bytes` reste hors de la route (décision 2) : il est écrit dans le `warn` de plafond et documenté pour l'opérateur, pour ne pas mêler taille logique et taille physique dans une réponse de lecture.

### (c) Le placement de la tâche d'implémentation

La tâche hors plan **`PLAFONNER-JOURNAL-DE-DEBUG`** est retenue, en P8, **avant `MESURER-EN-CONTENEUR-MAC`**, qui mesure le journal en conteneur et doit le faire sur une version déjà bornée. L'ampleur est petite à moyenne : environ 500 à 700 lignes de diff, un passage de `@dev`, un passage de `@testeur` pour INV-17, une relecture. Les deux autres places envisagées, `NETTOYER-P8` et une insertion dans le plan de P8, ne sont pas retenues.

## Conséquences

- **Positives** :
  - le journal a une borne de disque **indépendante** de `sample_rate` et de `ttl_days`, donc un tirage mal réglé ne remplit plus un disque ;
  - le dépassement est borné par un intervalle de vidage, soit quelques Mio au pire, au lieu de croître jusqu'au disque plein ;
  - le coût sur le chemin de requête reste **nul** : le contrôle est dans l'écrivain, comme le payload (ADR-0119, décision 8 ; INV-17) ;
  - l'état est visible sans lire un log, par quatre membres d'une route déjà réservée à `admin` ;
  - le schéma et `format_version` ne changent pas : aucun fichier existant n'est illisible, et INV-15 reste tenu sans migration ;
  - `ttl_days` garde son sens de borne de confidentialité, que le plafond ne peut que resserrer.
- **Négatives / coûts** :
  - une ligne diagnostique peut être supprimée plus tôt que `ttl_days` ne le promettait : la rétention devient « au plus 7 jours, et moins si le disque l'exige » ;
  - le vidage porte un travail de plus (comptage, trim, parfois récupération) : les vidages des journaux au plafond sont plus longs que les autres, sans effet sur les requêtes ;
  - un fichier existant ne redescend pas sous son plus haut niveau sans `VACUUM` hors ligne : l'opérateur qui baisse le plafond doit passer par la procédure documentée, journal désactivé ;
  - deux clés de configuration de plus, et deux surcharges `CRIBLE_*` de plus à documenter et à tester ;
  - le `warn` de plafond peut devenir régulier sur une instance très sollicitée : c'est un état normal, et un `warn` permanent use la vigilance qu'on lui demande.
- **À surveiller** :
  - la taille d'une ligne sur Legifacile : les 3,48 à 3,54 Ko de MES-16 sont synthétiques, et une ligne réelle plus grosse rend le couple 500 000 / 2 Gio effectivement plus court que 5,8 jours ;
  - le temps d'un vidage au plafond, avec trim et récupération : s'il approchait la seconde, le budget de pages de la décision 5 serait à revoir ;
  - `trimmed` qui croît sans interruption : il dit un plafond trop bas pour le trafic, ou un `sample_rate` laissé à 1.0 en production ;
  - la cohérence entre `bytes` et `file_bytes` : un écart durable dit des pages libres que la récupération ne rend pas, donc un `auto_vacuum` absent ou une récupération trop bornée ;
  - la place réelle de `debug.sqlite` dans les mesures de P8 : c'est `MESURER-EN-CONTENEUR-MAC` qui doit la chiffrer sur volumes réels, et non cet ADR.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Ne pas borner la taille, garder `ttl_days` seul | La durée ne borne pas les octets : à 49 requêtes/s supposées et `sample_rate = 1.0`, 7 jours valent environ 99 Gio, et le seul signal serait la perte silencieuse d'un disque plein |
| Un plafond en lignes seulement | Le nombre de lignes ne borne pas les octets : une ligne peut approcher la limite de corps de 32 Mio (ADR-0102), et quelques milliers de lignes suffiraient à remplir un disque |
| Un plafond en octets physiques du fichier seulement | Il ne redescend pas sans récupération, donc le plafond resterait dépassé après chaque trim sur un fichier existant ; et il mêle la place rendue au système à la place occupée par les données |
| Un plafond en octets logiques seuls | Il borne le disque utile mais pas le coût de la purge et de la route de lecture, qui croissent avec le nombre de lignes ; et il ne dit rien d'une ligne énorme |
| Des constantes dans le code, sans clé | La taille du disque et la politique de diagnostic varient d'un hôte à l'autre, comme `ttl_days` l'admet déjà ; une constante imposerait un binaire différent par déploiement |
| `0` valant « pas de plafond » | Cela recréerait le mode non borné que cet ADR supprime, sous une clé qui aurait l'air réglée ; refuser `0` au démarrage rend l'intention explicite |
| Refuser d'écrire au plafond et compter les refus | Le journal perdrait justement les requêtes du moment où le diagnostic compte, et le compteur de pertes existant ne distingue pas un disque plein d'une file pleine |
| Arrêter le journal au plafond | Il tuerait le diagnostic sans le dire, et il faudrait un mécanisme pour le rallumer, donc un état de plus dans l'instance |
| Un trim par ligne, à l'insertion | Il paierait un contrôle à chaque insertion pour un gain nul : la transaction du vidage borne déjà le dépassement à un intervalle |
| Un contrôle au tick de 24 h de la purge | Trop tard : un pic d'une heure peut dépasser le plafond de plusieurs jours de rétention |
| Un contrôle à l'ouverture seulement | Insuffisant : un processus qui tourne une semaine ne repasserait jamais sous le plafond |
| Un `VACUUM` complet après chaque purge | Il demande jusqu'au double de la place, bloque la connexion et échoue quand le disque est plein, soit exactement le cas à traiter |
| `auto_vacuum = FULL` | Il déplace des pages à chaque transaction, donc à chaque vidage, pour un gain que la récupération bornée donne sans ce coût sur le chemin d'écriture |
| Aucune récupération d'espace | Le plafond logique suffit à ne pas dépasser les octets, mais pas à rendre la place au système ; un opérateur qui baisse le plafond ne verrait jamais le fichier rétrécir |
| Exposer le plafond sur `GET /v1/stats` | La réponse est un objet fermé de seize décisions (ADR-0124, décision 1), et le journal n'est pas un analytics ; rouvrir cet ADR pour un compteur de fichier serait disproportionné |
| Exposer le plafond sur `/ready` | Un journal au plafond est un état normal, pas une indisponibilité (ADR-0099) : `/ready` sert au déploiement, pas au diagnostic de volumétrie |
| Un plafond exprimé en jours seulement | C'est `ttl_days`, qui existe déjà et ne borne pas les octets ; un second réglage de durée ferait deux vérités sur la rétention |

## Références

- Demande : propriétaire, 2026-11-02, « tu peux faire un adr pour le plafond de taille de debug.sqlite » ; analyse de `@architecte` du 2026-11-02, verdict `NON-COUVERT`, au contrôle de fin de P7.
- Conception : `docs/06-observabilite.md:91-109` (journal de debug, config, écriture, vidage, purge, logs) ; `docs/05-api.md:451-477` (`GET /v1/debug/queries`, composant `DebugQueryList`) ; `docs/07-stack-et-deploiement.md:89-92` (section `[debug_journal]`) ; `docs/08-invariants-et-tests.md:23` (INV-15) et `:25` (INV-17).
- Mesure : `docs/mesures/MES-16-charge-concurrente-analytics-et-journal.md:201-215` (tailles du fichier et lignes), `:267` (débit de repos), `:338-348` (limites de la mesure).
- Code : `crates/crible-store/src/debug_journal.rs:140-160` (`open_or_create`), `:170-209` (`write`, une transaction), `:220-241` (`purge`) ; `crates/crible-server/src/debug_journal.rs:76` (`FLUSH_INTERVAL`), `:79` (`STOP_BOUND`), `:82` (`PURGE_INTERVAL`), `:367-430` (écrivain), `:472-510` (`Flush::run`, `warn` d'échec) ; `crates/crible-server/src/config.rs:189-196` (`DebugJournalConfig`), `:329` (liste fermée des clés), `:338-347` (`ttl_days = 0` refusé) ; `crates/crible-store/src/records.rs:1296-1304` (`database_error`) ; `crates/crible-server/src/http.rs:632-641` (`/ready`) ; `crates/crible-store/src/snapshot.rs:1009` (seul `VACUUM` du dépôt).
- ADR : ADR-0014 (analytics sans contenu, journal à la demande), ADR-0031 (clé inconnue refusée), ADR-0038 (`SQLITE_BUSY` en `overloaded`), ADR-0097 (surcharges `CRIBLE_*`), ADR-0099 (`/ready`), ADR-0102 (limite de corps de 32 Mio), ADR-0111 (objets fermés, exemples OpenAPI), ADR-0119 (décision 8, capture sans coût), ADR-0121 (stockage, schéma, payload, protections), ADR-0122 (exécution, file, vidage, purge, route, preuves), ADR-0124 (réponse fermée de `GET /v1/stats`).

## Documents modifiés à l'acceptation

| Document | Modification |
|---|---|
| `docs/06-observabilite.md:91-109` | deux plafonds de `[debug_journal]`, trim au vidage, récupération bornée, visibilité (décisions 1 à 7) |
| `docs/07-stack-et-deploiement.md:89-92` | `max_rows` et `max_bytes` dans la section `[debug_journal]`, avec leurs surcharges `CRIBLE_*` et le refus de `0` |
| `docs/05-api.md:451-477` | membres `capped`, `rows`, `bytes` et `trimmed` de `GET /v1/debug/queries`, et l'exemple mis à jour |
| Composant OpenAPI `DebugQueryList` | quatre membres ajoutés, objet toujours fermé (ADR-0111) |
| `docs/phases/P8-docker-et-deploiement.md` | fiche `PLAFONNER-JOURNAL-DE-DEBUG`, hors plan en P8, avant `MESURER-EN-CONTENEUR-MAC` (point (c)) |
| `docs/adr/README.md` | ligne d'index de cet ADR |

Sont **non modifiés** : `docs/01` à `docs/04`, `docs/08`, le corps d'ADR-0121 et d'ADR-0122, et `docs/suivi.md` (tenu par l'orchestrateur). Le code n'est pas touché par cet ADR : il est livré par `PLAFONNER-JOURNAL-DE-DEBUG`, ouverte en P8.

## Note d'application du 2026-11-02

Aucune décision de cet ADR ne change. Sur décision du propriétaire du 2026-11-02, l'**exemple JSON** de `GET /v1/debug/queries` dans `docs/05-api.md` ne porte pas encore `capped`, `rows`, `bytes` ni `trimmed` : ces membres entrent dans l'exemple avec leur implémentation, par `PLAFONNER-JOURNAL-DE-DEBUG` (P8). La raison est le test `json_examples_conform_to_the_specification` du crate `crible-server`, qui compare chaque exemple de `docs/05` à la spécification OpenAPI produite par le code : un exemple en avance sur le composant `DebugQueryList` fait échouer le test. La prose de `docs/05` (description des membres, objet fermé) reste en place, la ligne du tableau ci-dessus vaut pour la fin de la tâche, et une note datée dans `docs/05` et dans la fiche de la tâche rappelle la remise en place.
