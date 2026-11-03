# ADR-0135 — Protocole de la mesure en conteneur sur la machine de développement

- **Statut** : Accepté (accepté par le propriétaire le 2026-11-03, tel que proposé)
- **Date** : 2026-11-03
- **Décideurs** : propriétaire
- **Proposé par** : `@greffier`, à la demande de l'orchestrateur, pour la tâche `MESURER-EN-CONTENEUR-MAC` (P8), d'après les verdicts de `@architecte` du 2026-11-03
- **Remplace** : —

## Contexte

La tâche `MESURER-EN-CONTENEUR-MAC` (P8) demande les chiffres en conteneur, à titre indicatif, sur la machine de développement. Sa fiche (`docs/phases/P8-docker-et-deploiement.md`, section `### MESURER-EN-CONTENEUR-MAC`) dit : « le froid sous Linux — `posix_fadvise(POSIX_FADV_DONTNEED)`, `drop_caches` sous root, comptabilité cgroup — n'est défini par aucun document ; il est tranché par un ADR au début de la tâche ».

Cet ADR est cet arbitrage. Il ne mesure rien : il fixe ce que la campagne a le droit d'affirmer et comment.

### Ce que les documents déjà acceptés laissent ouvert

- **ADR-0077** fixe le protocole de latence dense : N ∈ {1000, 2000, 5000}, 300 requêtes fp32 distinctes, 3 passes après échauffement, p50/p95/p99 au rang le plus proche, RSS en trois états, dont **(b)** après une passe de requêtes à vecteur fourni, sans session ONNX, et **(c)** avec la session fp32 chargée, rapportée à part.
- **ADR-0080, décision 7** signale le conteneur sans le décider : « Sous Linux x86_64, les pages font 4 Kio : il reste environ une page par candidat, avec quatre fois moins d'octets par page, et le coût froid suit les IOPS du volume. `F_NOCACHE` n'existe que sur macOS ; le froid Linux (`posix_fadvise(DONTNEED)`, cgroup) n'est défini nulle part. »
- **ADR-0084, décision 6** renvoie le froid vrai à P8 et demande un protocole défini **avant** de conclure. Sa décision 7 interdit toute affirmation de budget à froid tant que P8 n'a pas produit une série froide contrôlée, et impose le nom « froid partiel, résidence r » dans le cas contraire.
- **ADR-0105** range dans « À surveiller » le coût du parcours de `sizes.disk_bytes` : « s'il dépasse quelques millisecondes, la valeur sera portée par l'état plutôt que recalculée ». La route `GET /v1/indexes/{index}/documents/{doc_id}` ouvre en outre une connexion `ReadPool` (capacité 1) par requête.
- **ADR-0129, décision 4** ramène la tâche à la machine de développement, sous Docker Desktop, avec la RAM de la VM indiquée dans le rapport ; la mesure est indicative et ne porte plus la cible de latence contraignante. Sa **décision 6** place l'index réel de 8,58 M passages derrière `IMPORTER-BASE-COMPLETE`, en P10, et la latence contraignante derrière `MESURER-SUR-MAC-MINI`.
- **`docs/07`** impose `/data` en **volume nommé**, un utilisateur non-root, et rappelle que la RAM disponible est celle de la VM Docker, pas celle de la machine (`docs/07:138`).

### Environnement réel et verdicts du 2026-11-03

Machine : MacBook Air M3, `arm64`, 8 cœurs, 24 Gio. Docker Desktop 28.5.1. La VM Linux est `aarch64`, à 8 cœurs et 11,7 Gio.

`@architecte` a rendu, le 2026-11-03, les verdicts suivants.

- **NON-COUVERT** : le froid sous Linux, le seuil d'écart entre mesures, la base de la recommandation de RAM, le protocole et le seuil des deux routes, la taille de page, la portée de la purge, la comptabilité cgroup et l'échauffement.
- **ECART** : l'échelle du jeu. L'index réel à 8,58 M passages n'existe qu'avec `IMPORTER-BASE-COMPLETE`, en P10 (ADR-0129, décision 6).
- **CONFORME** : le montage `/data` en volume nommé, tel que `docs/07:138` l'impose.

### Ce que la fiche demande en plus d'ADR-0080

La décision 7 d'ADR-0080 ne citait que MES-02, MES-05 et MES-09 pour P8. La fiche y ajoute le rescoring fp16 (ADR-0077, ADR-0080), la recette de l'attache avec ses tailles de lot (ADR-0073) et les deux coûts de routes (ADR-0105). C'est la fiche qui fait foi pour le périmètre.

## Décision

**La mesure en conteneur suit un protocole unique : froid obtenu par éviction depuis le processus puis `fadvise`, résidence toujours publiée par point ; jeu `synth-large` à l'échelle déclarée, l'échelle réelle étant reportée en P10 ; `/data` en volume nommé seulement ; comparaison aux dernières campagnes natives avec un seuil d'écart explicite ; recommandation de RAM fondée sur les états (b) et (c) d'ADR-0077 ; seuil de 5 ms au p95 pour les deux routes ; taille de page, cgroup et échauffement déclarés.**

### Décision 1 — Froid sous Linux : éviction depuis le processus, puis `fadvise`, résidence vérifiée par point

Le mécanisme principal est `posix_fadvise(POSIX_FADV_DONTNEED)` sur `vectors.f16`, qui ne demande aucun privilège.

Une limite connue du noyau Linux est écrite dans chaque rapport : `fadvise(DONTNEED)`, comme `drop_caches`, n'évince que les pages du cache **non projetées**. Une page déjà projetée par un processus — typiquement le searcher ouvert qui a déjà servi des requêtes — reste résidente.

Donc, pour la série « searcher déjà ouvert, cache déplacé après l'ouverture » d'ADR-0080, le harnais évince d'abord **depuis le processus** : `madvise(MADV_PAGEOUT)` ou `MADV_DONTNEED` sur la projection des vecteurs, puis `fadvise`.

La résidence est **toujours** vérifiée, par `mincore` ou `fincore`, et publiée par point. Si elle reste au-dessus de 0, le chiffre s'appelle « froid partiel, résidence r » (ADR-0084, décision 7) et jamais « à froid ».

`drop_caches` dans un conteneur privilégié ne sert que de contre-contrôle, déclaré global à la VM.

### Décision 2 — Jeu et échelle : `synth-large` et une copie à ≈ 9 segments, l'échelle réelle en P10

Le jeu est `synth-large` : 8,6 M passages, 7,74 M vivants, régénéré par `tools/` selon `tools/README.md`. Une copie répartie sur ≈ 9 segments, à raison de segments d'au plus 1 M passages (`docs/02:202`), sert au cas « nombre de segments réel ».

L'échelle est déclarée dans chaque rapport. L'exigence « 8,58 M passages de la base réelle » est reportée à P10, avec `IMPORTER-BASE-COMPLETE` et `MESURER-SUR-MAC-MINI`. **C'est l'écart à la conception.**

### Décision 3 — Montage : volume Docker nommé seulement

`/data` est monté en volume Docker nommé, et rien d'autre (`docs/07:138`). Un chiffre pris en montage d'hôte, s'il est pris, va en annexe du rapport et n'est comparé à aucune mesure native.

### Décision 4 — Comparaison aux mesures natives : dernières campagnes publiées et seuil de 10 %

La comparaison se fait à la **dernière campagne** de chaque rapport de `docs/mesures/` : MES-02 ; MES-03 dans son dernier rejeu d'ADR-0080 ; MES-05 ; MES-08, campagne de la recette du 2026-10-27 ; MES-09.

Un **écart important**, qui déclenche un compte rendu, est un p50 ou un p95 en conteneur qui s'écarte de plus de **10 %** au-delà de la dispersion entre campagnes natives publiée.

Chaque écart important reçoit un mécanisme nommé — IOPS du volume, taille de page, cache cgroup, fils d'ONNX Runtime, etc. — ou la mention « non expliqué ».

### Décision 5 — Recommandation de RAM de la VM

La base de la recommandation est le pic de mémoire du conteneur aux états **(b)** et **(c)** d'ADR-0077, session fp32 de l'encodeur comprise, lu avec `memory.stat` et scindé entre mémoire anonyme et mémoire adossée à un fichier.

Le rapport dit que le cache de pages des vecteurs fp16 est évincable et ne fixe aucun plancher. Il donne deux valeurs : la **RAM minimale** (mémoire possédée plus une marge) et la **RAM confortable** (avec les vecteurs résidents).

Aucune limite `--memory` n'est posée sur le conteneur : la RAM de la VM fait la borne, et elle est déclarée.

### Décision 6 — Les deux routes : à chaud seulement, 5 ms au p95

Les deux routes mesurées sont `sizes.disk_bytes` de `GET /v1/indexes/{index}`, puis `GET /v1/indexes/{index}/documents/{doc_id}` avec la part de l'ouverture de sa connexion `ReadPool`.

Protocole : **à chaud seulement**, au moins **300 appels**, **3 passes** après une passe d'échauffement jetée, p50/p95/p99, client HTTP nommé dans le rapport, sur la copie à ≈ 9 segments. La part de l'ouverture est isolée par instrumentation du harnais ou par comparaison avec une connexion déjà ouverte, et la méthode est déclarée.

Le « quelques millisecondes » d'ADR-0105 est chiffré à **5 ms au p95** :

- au-delà pour `disk_bytes`, porter la taille dans l'état publié devient une **tâche à part** ;
- un pool par époque est justifié si l'ouverture dépasse 5 ms au p95 **ou** la moitié du p50 de la route, et reste une **tâche à part**.

### Décision 7 — Points de méthode : page, cgroup, purge et échauffement

- La taille de page de la VM est **mesurée** (`getconf PAGESIZE` dans le conteneur) et déclarée. L'interprétation d'ADR-0080 — environ une page par candidat, chiffre écrit pour `x86_64` à 4 Kio — suit la valeur mesurée, la VM étant `aarch64`.
- Les séries froides tournent sur un **conteneur neuf** après toute purge globale.
- `memory.current` et `memory.stat` du cgroup sont publiés avec le RSS.
- Échauffement : une **passe complète** du jeu de requêtes, jetée, avant les 3 passes mesurées.

### Décision 8 — Portée : la fiche fait foi, le rapport le dit

La décision 7 d'ADR-0080 ne citait que MES-02, MES-05 et MES-09 pour P8 ; la fiche ajoute le rescoring, la recette et les routes. **La fiche fait foi**, et le rapport le dit.

Ce qui n'est **pas** décidé ici :

- les cibles de latence en conteneur, qui appartiennent à P10 (`MESURER-SUR-MAC-MINI`) ;
- toute correction des routes, qui serait une tâche à part ;
- tout changement de fournisseur d'exécution ONNX ou ré-export du graphe, qui demande un nouvel ADR et une fiche.

## Conséquences

- **Positives** : le froid est enfin contrôlé et reproductible sous Linux, et chaque chiffre porte sa résidence. Le seuil de 10 % et celui de 5 ms rendent les conclusions vérifiables. La recommandation de RAM s'appuie sur des états déjà normalisés par ADR-0077.
- **Négatives / coûts** : la série « searcher déjà ouvert » ne donne souvent qu'un froid partiel, et le rapport doit le dire. L'échelle réelle de 8,58 M passages manque jusqu'en P10 : les chiffres de P8 restent indicatifs et synthétiques. La mesure coûte deux campagnes de plus (routes, recette).
- **À surveiller** : la résidence publiée point par point ; la dispersion entre campagnes natives, qui peut rendre le seuil de 10 % trop serré ou trop lâche ; les deux routes au voisinage de 5 ms au p95 ; la mémoire du cgroup, qui peut invalider la recommandation de RAM si le cache de pages n'est pas évincé comme prévu.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| `drop_caches` comme série principale du froid | Global à la VM, et évince aussi SQLite et tantivy : le chiffre ne décrit plus le rescoring seul. |
| Limite mémoire cgroup ou `memory.reclaim` comme mécanisme de froid | `cgroupfs` n'est pas exposé par Docker Desktop, et cela change le reclaim de toutes les séries. |
| Montage d'hôte pour `/data` | Le mmap à travers le partage de fichiers macOS est lent (`docs/07:138`) ; le chiffre ne serait comparable à rien. |
| Seuil d'écart fixe, sans dispersion entre campagnes natives | Un seuil fixe ignore la variabilité déjà publiée et déclenche des comptes rendus sans cause. |
| Précédent de 3 × p99 d'ADR-0113 | Cet ADR juge une dérive de service sous charge ; il ne compare pas deux campagnes de mesure. |
| Seuil de 10 ms pour les deux routes | Trop lâche pour trancher « quelques millisecondes » : les deux corrections resteraient indécises. |
| Renvoi au budget de `docs/03` pour les routes | Ce budget porte la recherche, pas l'administration ni la lecture d'un document. |
| Exiger 8,58 M passages en P8 | L'index réel n'existe qu'après `IMPORTER-BASE-COMPLETE` (P10, ADR-0129 décision 6) ; la campagne serait bloquée. |

## Références

- `docs/phases/P8-docker-et-deploiement.md`, section `### MESURER-EN-CONTENEUR-MAC`
- `docs/07-stack-et-deploiement.md` (volumes, RAM de la VM, `:138`)
- `docs/02-architecture.md:202` (segments d'au plus 1 M passages)
- `docs/08-invariants-et-tests.md` (MES-02, MES-03, MES-05, MES-08, MES-09)
- `tools/README.md` (génération de `synth-large`)
- ADR-0073 (recette sans encodage par lots), ADR-0077 (protocole de latence dense), ADR-0080 (accès aux vecteurs fp16, décision 7), ADR-0084 (quel froid fait foi, décisions 6 et 7), ADR-0105 (administration des index, « À surveiller »), ADR-0129 (replanification de la fin du plan, décisions 4 et 6)
- Verdicts de `@architecte` du 2026-11-03 (NON-COUVERT, ECART, CONFORME)
