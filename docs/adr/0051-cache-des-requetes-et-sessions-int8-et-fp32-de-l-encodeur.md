# ADR-0051 — Cache des requêtes et sessions INT8 et fp32 de l'encodeur

- **Statut** : Accepté (décision du propriétaire le 2026-10-24) — point 3 (défaut `int8` de `[encoder].query_precision`) remplacé par ADR-0052
- **Date** : 2026-10-23 (proposition), 2026-10-24 (décision)
- **Décideurs** : propriétaire
- **Décision** : accepté le 2026-10-24 par le propriétaire, tel que proposé : les trois options recommandées sont retenues, y compris la session fp32 gardée jusqu'à l'arrêt de l'instance.
- **Proposé par** : agent greffier, sur demande du propriétaire. Origine : `QUESTION-CONCEPTION` remontée par `@dev` sur la tâche `ENCODER-REQUETES` (phase P2, fiche `docs/phases/P2-encodeur-et-recette.md`), avant implémentation. Verdicts de `@architecte` le 2026-10-23 : `NON-COUVERT` pour les points 1 et 3, `ECART` pour le point 2.
- **Remplace** : —

## Erratum du 2026-10-29

Dans « ADR liés », le renvoi « ADR-0027 (configuration `serde` et surcharges `CRIBLE_*`) » est erroné. ADR-0027 ne décide que l'ajout de `serde_json`. La forme des surcharges `CRIBLE_*`, le refus d'une variable inconnue ou invalide et la liste des variables exclues sont décidés par ADR-0097. La variable `CRIBLE_LIMITS__QUERY_CACHE_ENTRIES` citée dans la décision suit ADR-0097. Le texte d'origine est laissé tel quel. Rien ne change dans les décisions de cet ADR.

## Contexte

La tâche `ENCODER-REQUETES` doit livrer `encode_query(text)` : INT8 par défaut, fp32 en option, prompt `query: `, troncature à `max_tokens`, un cache LRU en RAM jamais persisté, et un repli vers fp32 si le modèle INT8 est absent. Trois choix nécessaires à cette implémentation ne sont tranchés ni par `docs/01` à `docs/08` ni par un ADR accepté. `@dev` s'est arrêté avant d'écrire le code, comme le veut la règle 3 des agents.

### 1. Capacité du cache LRU des requêtes — NON-COUVERT

Ce que dit la conception :

- la fiche `ENCODER-REQUETES` : « un cache LRU en RAM (taille configurable), jamais persisté » ;
- `docs/03` (pipeline, étape 2) : « Encode la requête en ONNX INT8, avec un cache LRU en RAM », et plus loin « jamais persisté » ;
- `docs/05` (« Limites par défaut (configurables) ») : le cache n'y figure pas ;
- `docs/07` (`crible.toml`) : le bloc `[limits]` ne contient que `writer_queue = 16` et `default_timeout_ms = 2000`.

La fiche exige donc une taille **configurable**, sans nommer la clé ni donner de défaut. Il faut choisir les deux.

Ordre de grandeur : une entrée de cache porte un vecteur de 768 composantes. En fp32, 1 024 entrées pèsent 1 024 × 768 × 4 octets = **3,0 Mio**, à comparer au minimum incompressible de **1,2 Go** de `docs/02` (section « Mémoire : ce qui est en RAM »). Le coût est négligeable. En INT8, le même cache pèse quatre fois moins.

### 2. Coexistence des sessions INT8 et fp32 — ECART

`docs/02` (Mémoire) ne compte qu'une ligne de modèle : « Modèle ONNX INT8 | RAM | ~0,3 Go », et affirme « Un même modèle n'est chargé qu'une fois par instance : il est partagé par tous les index qui ont la même empreinte ». ADR-0002 reprend « ~0,3 Go en INT8 ». Le budget de `docs/02` ne prévoit donc **qu'une** session.

Or `docs/04` (Le modèle) impose deux précisions à l'exécution :

- les **requêtes** sont encodées en INT8, réglable en `fp32` ;
- la **recette** (contrôle 9, INV-12) et l'**encodage en ligne de documents** se font en `fp32`.

Deux conséquences :

- charger la session fp32 en plus de l'INT8 ajoute **plus d'1 Go** de RAM au-dessus du budget de `docs/02` ;
- `docs/04` précise que « le fichier INT8 n'entre pas dans l'empreinte, car il ne sert qu'aux requêtes ». Un registre de sessions indexé par la **seule empreinte** ferait donc collision entre le graphe INT8 et le graphe fp32 du même modèle : le second chargement écraserait le premier, ou servirait le mauvais graphe. C'est un défaut de correction, pas seulement de budget.

La conception ne dit pas non plus **quand** la session fp32 est chargée, ni si elle est libérée après usage. `docs/02` ne peut pas rester muet sur une pièce qui peut peser plus d'1 Go.

### 3. Nom de la clé de précision des requêtes — NON-COUVERT

`docs/04` dit « les requêtes sont encodées en INT8 (réglable : `fp32`) » sans nommer la clé de configuration. `docs/07` ne la nomme pas non plus, et ne contient aucune table `[encoder]`. La tâche doit pourtant exposer ce réglage : MES-04 compare INT8 et fp32 sur les requêtes réelles, et le repli vers fp32 quand l'INT8 est absent doit être observable.

La question porte sur le **nom** et sur l'**emplacement** de la clé, pas sur le défaut : `docs/04` fixe déjà INT8 par défaut.

## Décision

Trois décisions, indépendantes l'une de l'autre. Elles sont **proposées** : seul le propriétaire les accepte.

### 1. Capacité du cache : `[limits].query_cache_entries`, défaut 1 024

Le cache LRU des requêtes est borné par la clé `query_cache_entries` du bloc `[limits]` de `crible.toml`, avec **1 024** entrées par défaut. La clé est surchargeable par la variable d'environnement habituelle (`CRIBLE_LIMITS__QUERY_CACHE_ENTRIES`), comme toute clé de `crible.toml` (`docs/07`).

- Le cache est **par instance** et **jamais persisté** : il disparaît à l'arrêt, et rien n'est écrit sur disque.
- Sa clé d'entrée est `(empreinte du modèle, précision, texte exact de la requête sans prompt)`. La précision entre dans la clé parce que les deux sessions ne produisent pas le même vecteur (voir point 2). Le texte stocké est celui d'avant l'ajout du prompt.
- La capacité compte des **entrées**, pas des octets. Une entrée est un vecteur de `dims` composantes dans la précision de la session.
- Valeur à documenter à l'acceptation : une ligne dans `docs/07` (bloc `[limits]` du `crible.toml` d'exemple) et une ligne dans le tableau de `docs/05`, section « Limites par défaut (configurables) » : `Cache des requêtes | 1 024 entrées par instance`.

### 2. Sessions : registre indexé par `(empreinte, précision)`, chargement paresseux

Le registre de sessions de `crible-encode` est indexé par le couple `(empreinte du modèle, précision)`, et non par l'empreinte seule. Chaque session est chargée **au premier usage** de sa précision :

- une instance qui ne sert que des requêtes ne charge que l'INT8 (~0,3 Go, budget de `docs/02` inchangé) ;
- la session fp32 n'apparaît qu'après un appel de recette ou un encodage en ligne de documents ;
- les deux sessions d'un même modèle restent distinctes, même si l'INT8 ne figure pas dans l'empreinte.

Ce qui reste à trancher par le propriétaire : **la politique de libération de la session fp32**. La proposition est de la **garder jusqu'à l'arrêt de l'instance**, comme la session INT8 : c'est la politique la plus simple, elle évite un chargement répété (plus d'1 Go relu à chaque recette) et elle rend le comportement prévisible. L'alternative, évincer la session fp32 après une période d'inactivité, rend la RAM au système mais fait payer le rechargement à chaque attache ; elle n'est retenue que si la RAM de la VM Docker devient serrée (aujourd'hui ~8 Go, `docs/07`).

À documenter à l'acceptation : une ligne dans le tableau de `docs/02` (Mémoire), par exemple `Modèle ONNX fp32 | RAM, seulement après recette ou encodage en ligne | plus d'1 Go`, avec la mention que cette session est paresseuse.

### 3. Précision des requêtes : `[encoder].query_precision`, défaut `int8`

La précision des requêtes se règle par une clé de `crible.toml` :

```toml
[encoder]
query_precision = "int8"   # "int8" | "fp32"
```

- Défaut : `int8`, conformément à `docs/04`.
- Une table `[encoder]` est créée dans `docs/07`, section « Configuration de l'instance : `crible.toml` ». C'est l'endroit naturel : la clé règle le moteur d'encodage, pas une limite de requête, pas l'API. Le bloc `[limits]` ne doit pas l'accueillir, car il regroupe des plafonds de charge, pas des choix de qualité.
- `docs/05` n'a **pas** de section de configuration : la clé n'y est donc pas décrite. En revanche, la valeur par défaut et le rôle du réglage trouvent leur place dans le tableau « Réglages de précision » de `docs/03`, à côté de `reference_precision` et de `search.scan`.
- Ce réglage ne crée pas de champ de requête HTTP : `docs/05` (Recherche : requête) n'a pas de champ de précision, et cet ADR n'en ajoute pas. `encode_query(text, precision)` reste une fonction interne, utilisée par MES-04 pour comparer INT8 et fp32, et par le repli automatique quand le fichier INT8 est absent.
- `fp32` pour les requêtes coûte la session fp32 décrite au point 2 : c'est un réglage d'évaluation et de diagnostic, pas un mode de production.

## Conséquences

- **Positives** :
  - `ENCODER-REQUETES` peut être implémentée sans choix implicite : les trois points sont tranchés et cités par le code ;
  - la taille du cache est réglable, comme la fiche l'exige, avec un défaut mesurable et négligeable (3,0 Mio en fp32, 0,75 Mio en INT8) ;
  - le registre `(empreinte, précision)` corrige une collision réelle entre deux graphes du même modèle ; c'est un défaut de correction évité, pas un confort ;
  - le budget mémoire de `docs/02` reste tenu au démarrage : la session fp32 n'est chargée que si on l'utilise ;
  - la clé de précision a un nom unique, cohérent avec les autres tables de `crible.toml`, et surchargeable par l'environnement.
- **Négatives / coûts** :
  - la session fp32 gardée après usage ajoute plus d'1 Go à la résidence mémoire d'une instance qui a fait une recette ou un encodage en ligne ; le budget de `docs/02` doit le dire, sinon il devient faux ;
  - le cache LRU est par instance, pas par index : deux index du même modèle partagent leurs entrées, ce qui est voulu mais doit être dit dans le code ;
  - une clé de configuration de plus (`query_cache_entries`) et une table de plus (`[encoder]`) à maintenir dans `docs/07`, avec leurs surcharges d'environnement ;
  - la politique de libération du fp32 reste une question ouverte tant que le propriétaire ne l'a pas tranchée ; la proposition « gardé jusqu'à l'arrêt » est la moins chère en code, pas la moins chère en RAM.
- **À surveiller** :
  - la **résidence mémoire** d'une instance après une recette : si elle dépasse durablement le budget de `docs/02`, la politique de libération du fp32 doit être revue (nouvel ADR) ;
  - le **taux de succès du cache** et son coût : MES-05 (latence d'encodage) est le signal ; un cache de 1 024 entrées inutile se verrait à un taux de succès proche de zéro ;
  - le **repli fp32** : s'il se déclenche souvent alors que le fichier INT8 est présent, c'est un défaut de chargement, pas un cas normal ;
  - la **dérive de MES-04** : la comparaison INT8/fp32 sur les requêtes réelles dira si INT8 reste le bon défaut (Q-06).
- **Ce que cet ADR ne décide pas** : la politique de libération du fp32 (proposée, à confirmer par le propriétaire) ; une éventuelle précision par requête dans l'API HTTP ; la taille du cache des documents (il n'y en a pas).

## Alternatives écartées

### Point 1 — Capacité du cache

| Alternative | Pourquoi écartée |
|---|---|
| (a) Constante de code à 1 024 entrées, sans configuration | Ne livre pas la « taille configurable » écrite dans la fiche. Un chiffre en dur oblige à recompiler pour changer un réglage d'exploitation. |
| (c) Capacité obligatoire, sans défaut | Une instance qui omet la clé refuserait de démarrer, alors que 1 024 entrées coûtent 3,0 Mio : le défaut est sans risque, l'obligation est une friction inutile. |

### Point 2 — Coexistence des sessions

| Alternative | Pourquoi écartée |
|---|---|
| (b) Un `Model` qui porte les deux sessions, chacune paresseuse | Même mémoire que (a), mais lie les durées de vie des deux sessions : on ne peut pas libérer l'une sans l'autre, et un `Model` devient responsable de deux graphes de précisions différentes. Le registre `(empreinte, précision)` garde cette séparation au bon endroit. |
| (c) Charger les deux sessions au démarrage | Contredit le budget de `docs/02` : toute instance paierait plus d'1 Go même sans recette ni encodage en ligne. |
| Libérer la session fp32 dès la fin de l'appel qui l'a chargée | Écartée comme **défaut** : une recette par lot rechargerait plus d'1 Go à chaque attache. Reste l'alternative à considérer si la RAM devient serrée. |

### Point 3 — Nom et emplacement de la clé

| Alternative | Pourquoi écartée |
|---|---|
| `[limits].query_precision` | Le bloc `[limits]` regroupe des plafonds de charge (file, délais) ; la précision est un choix de qualité, pas une limite. |
| `[search].query_precision` | La précision est une propriété de l'encodeur, pas de la stratégie de recherche ; `docs/03` et `docs/07` séparent déjà l'encodage de la recherche. |
| Clé par index, dans le `model.json` ou la config d'index | La précision de requête peut changer sans réindexer, et MES-04 doit comparer les deux valeurs sur le même index. Une clé d'instance suffit. |
| Champ `precision` dans la requête HTTP | Élargit l'API sans besoin exprimé : `docs/05` ne l'a pas, et la production reste en INT8. À instruire plus tard si un usage réel le demande. |

## Références

- Fiche : `docs/phases/P2-encodeur-et-recette.md`, `### ENCODER-REQUETES`.
- Conception : `docs/02-architecture.md` (« Mémoire : ce qui est en RAM »), `docs/03-recherche.md` (pipeline étape 2, « Réglages de précision »), `docs/04-donnees-et-ingestion.md` (« Le modèle », usage des précisions, empreinte), `docs/05-api.md` (« Limites par défaut (configurables) », « Recherche : requête »), `docs/07-stack-et-deploiement.md` (`crible.toml`).
- ADR liés : ADR-0002 (une instance par projet, budget mémoire), ADR-0028 (forme canonique de l'empreinte du modèle), ADR-0050 (référence du modèle et export ONNX), ADR-0027 (configuration `serde` et surcharges `CRIBLE_*`).
- Mesures et invariants : MES-04 (parité encodeur), MES-05 (latence d'encodage), INV-12 (recette à l'attache, en fp32), Q-06 (seuil de recette selon la précision d'encodage).
