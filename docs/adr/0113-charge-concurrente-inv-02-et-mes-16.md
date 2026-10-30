# ADR-0113 — Charge concurrente : critère d'INV-02, mesure MES-16, protocole et démonstration J6

- **Statut** : Accepté (par anticipation, décision déléguée par le propriétaire le 2026-10-30 ; revue avec le propriétaire à la fin de la série)
- **Date** : 2026-10-30
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, tâche TESTER-CHARGE-CONCURRENTE (phase P6), sur délégation du propriétaire pour la troisième série de P6 et sur les verdicts de `@architecte` du 2026-10-30
- **Remplace** : — (ne remplace aucune décision : complète ADR-0077, ADR-0102, ADR-0068 et ADR-0067, sans modifier le corps d'aucun d'eux)

## Contexte

### La fiche et ce qu'elle demande

La fiche `TESTER-CHARGE-CONCURRENTE` (`docs/phases/P6-serveur-et-api.md:149-157`) demande trois livrables : un script de charge à N clients de recherche qui tourne pendant des attaches, puis pendant une compaction (`:152`) ; un rapport avec les percentiles (`:152`) ; `docs/jalons/J6-demo.md` (`:156`). Ses critères d'acceptation sont : « le p99 de recherche pendant les écritures reste sous 3 fois le p99 au repos » (`:154`), « aucune erreur `5xx` autre que les `503` attendus » (`:155`) et l'écriture de la démonstration J6 (`:156`). La fiche cite un seul invariant, INV-02 (`:157`).

### Ce que la conception et les ADR voisins posent

`docs/08-invariants-et-tests.md:10` définit INV-02 — « Une lecture n'est jamais bloquée par une écriture » — et le teste par « une charge de recherches concurrentes pendant une attache et une compaction : **aucune requête au-delà du p99 de référence × 3** ». Le critère de `docs/08` porte donc sur **chaque** requête, alors que celui de la fiche porte sur le **p99** de la phase d'écriture. Les deux ne mesurent pas la même chose : un p99 laisse passer jusqu'à 1 % des requêtes au-delà du seuil, c'est-à-dire, à 4 clients sur 60 s, des centaines de requêtes.

`docs/08-invariants-et-tests.md:36` range la charge sous le niveau « Charge », avec deux invariants : INV-02 **et INV-17** (« L'écriture des analytics ou du journal ne bloque jamais une requête »). Or les analytics et le journal de debug arrivent en P7 : en P6, rien ne les écrit, donc rien ne peut prouver INV-17.

`docs/08-invariants-et-tests.md:61` exige qu'une mesure soit consignée dans `docs/mesures/MES-xx-<slug>.md` selon le modèle de `docs/mesures/README.md` (machine, commit, jeu de données, protocole, chiffres bruts, conclusion ; données brutes en JSON à côté du rapport). La règle 7 d'`AGENTS.md` réserve l'exécution d'une mesure à l'agent `@mesureur` : `@dev` écrit l'outil, `@mesureur` produit le chiffre. Aucune mesure de charge n'existe à ce jour dans la liste de `docs/08` (MES-00 à MES-15).

Les ADR voisins fixent les éléments de protocole que la fiche ne donne pas : ADR-0077 fixe les percentiles au rang le plus proche ; ADR-0102 fixe le `503 overloaded` immédiat avec `Retry-After: 1` quand le sémaphore de `[server].max_concurrent_searches` est plein ; ADR-0103 généralise `Retry-After: 1` à toute réponse `overloaded` ; ADR-0067 fixe l'échéance des écritures rendues en `202` ; ADR-0017 rappelle que la compaction suspend les écritures en v1 ; ADR-0068 fixe les deux étapes `failpoints` de la CI pour `crible-store` et son complément pour `crible-server`. ADR-0091 a créé le précédent d'un harnais versionné dans `tools/bench/mes06/`.

### Les verdicts de `@architecte` du 2026-10-30

Le verdict relève **deux ECART** : le critère de la fiche (p99 contre p99) n'est pas celui de `docs/08` (chaque requête contre le p99 de référence × 3) ; INV-17 est associé au script par `docs/08:36` mais n'est pas testable avant les analytics de P7.

Il relève **quatorze points NON-COUVERT** : le protocole de la phase de repos, l'endroit où la latence est mesurée, la répartition des rôles entre agents, le test d'invariant exécuté en CI, le numéro de la mesure, les données utilisées, le mode de recherche, N et les durées, les écritures soumises pendant la charge, les `503` dits « attendus », la langue et l'emplacement du harnais, la place de la campagne en CI, et le contenu de `docs/jalons/J6-demo.md`.

## Décision

### Décision 1 — le critère bloquant est celui de `docs/08`, par requête

**Constat.** La fiche écrit « le p99 de recherche pendant les écritures reste sous 3 fois le p99 au repos » (`:154`) ; `docs/08:10` écrit « aucune requête au-delà du p99 de référence × 3 ». Un p99 agrégé ne voit pas une lecture bloquée si elle reste sous 1 % des requêtes.

**Décision.** Le critère bloquant de la tâche est celui de `docs/08` : **pendant chaque phase d'écriture, chaque recherche se termine en moins de 3 × le p99 de la phase de repos**. Le critère de la fiche — p99 de la phase d'écriture sous 3 × p99 du repos — est **impliqué** par le précédent et il est **aussi rapporté** dans le rapport MES-16, pour rester lisible par rapport à la fiche.

**Conséquences.** Une lecture bloquée par une écriture se voit comme une valeur extrême et fait échouer la campagne, même isolée. Coût : le critère est plus dur qu'un p99 et un seul incident suffit ; c'est voulu, puisque INV-02 dit « jamais ».

### Décision 2 — INV-17 reste hors de P6

**Constat.** `docs/08:36` associe INV-17 au script de charge ; les analytics et le journal arrivent en P7, et `docs/08:25` ne peut donc pas être prouvé en P6.

**Décision.** INV-17 **n'est pas** traité par cette tâche. Le harnais de MES-16 est écrit pour être réutilisable, et la tâche de P7 qui prouve INV-17 le reprend.

**Conséquences.** La fiche reste tenue sur son unique invariant cité, INV-02. Coût : une seconde campagne en P7, sur le même harnais étendu au journal et aux analytics.

### Décision 3 — rôles et ordre

**Décision.**

- `@testeur` écrit des tests `inv_02_*` **au niveau du serveur**, sous la feature `failpoints` de `crible-server` (ADR-0068, complément du 2026-10-29), exécutés par l'étape « Test the server failpoint build » de la CI. Ces tests sont **déterministes et sans seuil de latence** : une recherche HTTP se termine et rend la génération précédente pendant qu'un job d'attache est retenu par un point de panne dans son écriture, et de même pendant une compaction retenue.
- `@dev` écrit le harnais `tools/bench/mes16/` et `docs/jalons/J6-demo.md`.
- `@mesureur` exécute la campagne MES-16 et écrit le rapport (règle 7 d'`AGENTS.md`).
- `@relecteur` relit le code, puis le rapport.

**Conséquences.** L'invariant a une preuve en CI, reproductible, indépendante d'une machine ; la mesure de latence reste la seule partie locale. Coût : deux niveaux de preuve à maintenir sur le même comportement, le test de panne et la campagne.

### Décision 4 — nouvelle mesure MES-16

**Décision.** La mesure **MES-16** « Charge concurrente pendant les écritures » est créée, en phase **P6**, et ajoutée à la liste des mesures de `docs/08-invariants-et-tests.md` avec sa phase et son renvoi au présent ADR. Son rapport est `docs/mesures/MES-16-charge-concurrente.md`, avec les données brutes en JSON à côté, selon `docs/mesures/README.md`.

**Conséquences.** La campagne cesse d'être un chiffre de session : elle est rejouable par son numéro. Coût : une ligne de plus au tableau des mesures et une campagne locale à rejouer quand le chemin de lecture change.

### Décision 5 — protocole

**Décision.**

1. Build `--release`. Le serveur est lancé **par le harnais**, avec une configuration temporaire.
2. Un index à modèle `external`, 1 024 dimensions, lexical activé. Données fabriquées par `crible.BatchWriter` avec une graine fixe : **50 000 passages de base**, texte compris ; puis **5 lots de 2 000 passages** (documents nouveaux et documents remplacés), attachés l'un après l'autre par la route des lots déposés (`exchange/inbox`) ; puis **une compaction** de l'index.
3. Requêtes : un jeu fixe de **200 requêtes** (vecteur et texte), alternant `dense` (avec `vector`) et `hybrid` (avec `query` et `vector`, sans encodeur), `k = 10`.
4. Charge : **N = 4 clients en boucle fermée**, chacun envoyant sa requête suivante dès la réponse. N reste **sous le nombre de cœurs**, pour ne pas mesurer la saturation du processeur ; `max_concurrent_searches` reste à son défaut ; le nombre de cœurs est rapporté dans le rapport.
5. Phases : **échauffement de 30 s** (écarté des résultats), **repos de 60 s** (référence), **attaches** (de la première soumission à la fin du dernier job), **compaction** (de la soumission à la fin du job).
6. Latence mesurée à l'**horloge murale côté client** pour chaque requête ; `took_ms` du serveur rapporté en second. Percentiles au **rang le plus proche** (ADR-0077).
7. **Trois campagnes complètes**, chacune sur un dossier de données neuf ; le critère doit tenir sur chacune.

**Conséquences.** Le protocole est rejouable et ne dépend d'aucune donnée de production. Coût : environ trois fois la durée d'une campagne, et des données synthétiques qui ne représentent pas la distribution réelle des tailles de documents.

### Décision 6 — codes d'état : les « `503` attendus » sont un ensemble vide

**Constat.** ADR-0102 rend `503 overloaded` quand le sémaphore de `[server].max_concurrent_searches` est plein ; avec N = 4 sous la limite par défaut, aucune recherche ne le sature. La fiche parle de « `503` attendus » (`:155`) sans dire lesquels.

**Décision.**

- **Recherches** : aucun `503` n'est attendu. Toute réponse `5xx`, **`503` compris**, fait échouer la campagne.
- **Écritures** : elles sont soumises une à une, donc aucun `503` n'est attendu non plus. Tout `503` est **compté**, son `Retry-After` relevé (ADR-0103), et **expliqué** dans le rapport ; il ne fait pas échouer la campagne, mais il doit être justifié.
- Les « `503` attendus » de la fiche valent donc, dans ce protocole, **l'ensemble vide**.

**Conséquences.** Un `503` de recherche devient un signal d'alarme, pas une tolérance. Coût : le protocole ne couvre pas le comportement du serveur à saturation du sémaphore ; ce comportement est prouvé séparément par les tests d'ADR-0102.

### Décision 7 — CI

**Décision.** La campagne est **locale** : une latence de runner partagé n'est pas reproductible. Aucun nouveau test n'est exclu de la CI, et aucun `#[ignore]` n'est ajouté. En CI tournent :

- les tests `inv_02_*` du serveur (décision 3) ;
- les tests unitaires du harnais — calcul des percentiles, phases, critère — dans la suite de `tools/`.

Le harnais **n'ajoute aucune dépendance** : il utilise `crible.AsyncClient` et `crible.BatchWriter`, déjà disponibles dans `tools/` par la dépendance de chemin `crible`. L'environnement de `tools/` se met à jour par `uv sync --inexact` — jamais une synchronisation exacte, qui retirerait le groupe `export` — et `tools/uv.lock` suit le client.

**Conséquences.** La logique du harnais est testée sans machine dédiée, et la campagne reste rejouable à la main. Coût : aucune alerte automatique si une régression de concurrence apparaît ; elle ne se voit qu'à la campagne suivante.

### Décision 8 — `docs/jalons/J6-demo.md`

**Décision.** `@dev` écrit `docs/jalons/J6-demo.md` sur le modèle de `docs/jalons/J2-demo.md` :

- prérequis et build ;
- configuration et clés ;
- création des index de l'échantillon Legifacile (`data/legifacile-sample`, modèle ONNX réel) ;
- le scénario J6 via Swagger UI (`/docs`, bouton « Authorize ») et via le client Python : attacher, chercher en simple et en multi-index, patcher, compacter, exporter ;
- les commandes exactes ;
- un renvoi à MES-16 pour INV-02 sous charge.

`@jalon` rejoue la démonstration et la cite **sans la réécrire**.

**Conséquences.** Le jalon J6 a une démonstration exécutable et vérifiable, distincte de la mesure. Coût : la démonstration dépend de l'échantillon Legifacile et de l'environnement ONNX (ADR-0032, ADR-0057).

### Décision 9 — un échec ne relâche pas le critère

**Décision.** Si une campagne viole le critère, la tâche passe en `BLOQUE` et le rapport analyse la cause — attente d'un verrou ou d'un instantané, ou concurrence pour le processeur — **avant toute décision du propriétaire**. Le critère n'est jamais relâché pour faire passer la campagne.

**Conséquences.** Un échec produit une analyse utilisable et non un chiffre arrangé. Coût : une campagne ratée peut coûter un aller-retour avec le propriétaire.

## Conséquences

- **Positives** : INV-02 a une preuve déterministe en CI et une mesure locale nommée, rejouable ; le critère est plus strict que la fiche et couvre le pire cas ; les rôles sont fixés et la mesure reste chez `@mesureur` (règle 7) ; le jalon J6 dispose d'une démonstration écrite.
- **Négatives / coûts** : trois campagnes complètes par validation, sur des données synthétiques ; aucune détection automatique des régressions de concurrence ; une seconde campagne pour INV-17 en P7.
- **À surveiller** : la première campagne MES-16. Si un seul dépassement apparaît, la cause (verrou, instantané, processeur) décide de la suite : une cause de conception rouvre le présent ADR, une cause de mesure corrige le protocole par un nouvel ADR. Le coût de `GET /v1/indexes/{index}` signalé au suivi le 2026-10-29 est aussi à relever pendant cette campagne.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Garder le seul critère de la fiche (p99 contre p99) | Laisse passer jusqu'à 1 % des requêtes au-delà du seuil ; une lecture bloquée isolée ne se verrait pas, alors qu'INV-02 dit « jamais ». |
| Exiger les deux critères comme bloquants, à égalité | Le second est impliqué par le premier ; le rendre bloquant séparément n'ajoute aucune preuve et complique le verdict. |
| Prouver INV-17 en P6 par un journal factice | Les analytics et le journal n'existent pas avant P7 ; un journal factice ne prouverait pas le chemin réel. |
| Faire écrire la mesure par `@dev` dans la même session que le code | Viole la règle 7 d'`AGENTS.md` : `@dev` mesure ce qu'il vient d'écrire. |
| Mesurer la latence par `took_ms` du serveur seul | Ne voit ni l'attente dans la file HTTP, ni la sérialisation, ni le réseau local ; l'horloge murale côté client couvre la requête entière. |
| Boucle ouverte (débit fixe) au lieu de la boucle fermée | Une boucle ouverte mesure une file d'attente qui grossit, pas la latence de service ; la boucle fermée mesure ce que voit un client. |
| N égal au nombre de cœurs ou au-delà | Mesurerait la saturation du processeur, pas l'effet des écritures ; le critère deviendrait un test de débit. |
| Une seule campagne | Une campagne unique ne distingue pas un incident de mesure d'une régression ; trois campagnes sur données neuves donnent une marge de reproductibilité. |
| Tolérer les `503` de recherche comme « attendus » | Avec N = 4 sous la limite de concurrence, aucun `503` n'est attendu ; le tolérer masquerait une saturation réelle. |
| Lancer la campagne en CI | Une latence de runner partagé n'est pas reproductible ; le chiffre serait sans valeur et instable. |
| Ajouter des dépendances au harnais | `crible.AsyncClient` et `crible.BatchWriter` suffisent ; une dépendance de plus demanderait un ADR (règle 12). |
| Écrire `J6-demo.md` par `@jalon` | `@jalon` prépare la validation et ne décide pas du contenu de la démonstration ; il la rejoue et la cite. |
| Relâcher le critère après un échec | Ferait du critère une formalité ; l'échec doit d'abord être analysé. |

## Références

- `docs/phases/P6-serveur-et-api.md` : `:149-157` (fiche `TESTER-CHARGE-CONCURRENTE`), `:5-9` (jalon J6).
- `docs/08-invariants-et-tests.md` : `:10` (INV-02), `:25` (INV-17), `:36` (niveau « Charge »), `:43` (sauts de test), `:61-80` (mesures).
- `docs/mesures/README.md` (modèle de rapport, données brutes).
- `AGENTS.md`, règle 7 (mesure par `@mesureur`).
- ADR-0077 (percentiles au rang le plus proche), ADR-0102 (`503 overloaded`, `Retry-After`), ADR-0103 (`Retry-After: 1`), ADR-0067 (échéance des écritures), ADR-0017 (compaction et écritures), ADR-0068 (features `failpoints` en CI), ADR-0091 (harnais `tools/bench/mes06/`).
- `docs/jalons/J2-demo.md` (modèle de démonstration).
