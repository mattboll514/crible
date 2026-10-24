# ADR-0052 — Requêtes encodées en fp32 et seuil de recette à 0,999

- **Statut** : Proposé
- **Date** : 2026-10-24
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, sur demande du propriétaire. Origine : tâche `MESURER-PARITE-ENCODEUR` (phase P2, fiche `docs/phases/P2-encodeur-et-recette.md`), mesures MES-04 et MES-05, relues OK.
- **Remplace** : — (le point 3 d'ADR-0051, qui fixe le défaut `int8` de `[encoder].query_precision`, devient caduc si cet ADR est accepté)

## Contexte

La fiche de `MESURER-PARITE-ENCODEUR` demande deux choses avant le jalon J2 : « une recommandation (INT8 ou fp32 pour les requêtes) et une valeur de seuil de recette proposée (Q-06) ». Les deux campagnes ont été conduites en natif sur un MacBook Air M3 (8 cœurs, 24 Go, macOS 15.7.7), binaire `--release`, commit `2fd3c29`. Aucun code de production n'a été modifié pendant les campagnes.

### Ce que MES-04 mesure

Sur 300 requêtes réelles de Legifacile, avec le `QueryEncoder` du moteur (prompt `query: `, aucune troncature) :

| Comparaison | Cosinus minimum | Médiane | Moyenne |
|---|---:|---:|---:|
| fp32 ONNX contre PyTorch fp32 | 0,999999999989 | 0,999999999998 | 0,999999999998 |
| INT8 contre PyTorch fp32 | **0,920358** | 0,949328 | 0,948764 |

- La chaîne du moteur est fidèle à PyTorch pour le fp32 : l'écart est de l'ordre de 1e-12.
- L'INT8 dynamique dégrade fortement les vecteurs de requête : meilleure requête 0,9668, pire 0,9204.
- Sur le top-10 exact, l'INT8 ne conserve que **79,2 %** du top-10 du fp32 (rappel@10 moyen 0,7920), avec 31 requêtes sur 300 dont le top-10 est identique. Aucune des six structures n'atteint 0,90.
- Sur les 145 requêtes à attendus dans l'échantillon, l'INT8 place l'attendu dans le top-10 pour 110 requêtes, le fp32 pour 113. Cet écart de trois requêtes reste dans le bruit ; c'est le rappel@10 de 0,7920 qui est le signal robuste.
- L'écart INT8 n'est pas un défaut du harnais : il a été recontrôlé hors du code Rust, avec `onnxruntime` en Python (cosinus 0,948 à 0,963 sur les dix premières requêtes, cosinus Rust/Python de 1,0). C'est une propriété du graphe quantifié livré.

### Ce que MES-05 mesure

Mille encodages complets par passe, cache des requêtes désactivé, après 300 préchauffages, deux passes par précision :

| Précision | Premier appel | p50 | p95 | p99 |
|---|---:|---:|---:|---:|
| INT8 | 1,01 s | 8,6 à 8,8 ms | 22,5 à 23,0 ms | 27,9 à 28,2 ms |
| fp32 | 1,78 s | 21,0 ms | 55,8 à 56,4 ms | 70,1 à 70,2 ms |

Les réglages de session ont été explorés hors du code Rust, par le script Python `tools/bench/mes04/threads.py` (`session.run` seul) : les **défauts d'ONNX Runtime** sont les meilleurs. Forcer 8 threads, 4 threads ou 1 thread est plus lent, et le niveau d'optimisation `basic` est plus lent en INT8 et sans gain mesurable en fp32.

### Le principe du propriétaire et le statut de la latence

Le 2026-10-20, la section « Cibles » de `docs/suivi.md` a fixé le principe : **« le rappel prime, la latence est indicative »**. La règle de latence de `docs/08` ne vaut que mesurée sur le Mac mini en conteneur ; ailleurs, l'écart est rapporté, sans bloquer un jalon. MES-05 est native, sur un MacBook Air, sous charge ambiante : elle informe le choix, elle ne juge pas la cible.

La ligne de budget de `docs/03` (« Encodage de la requête (CPU, INT8) : 20 à 50 ms ») est écrite pour l'INT8. En fp32, le p50 tient dans le budget et le p95 le dépasse d'environ 13 %.

### Ce que la mesure de fidélité couvre, et ce qu'elle ne couvre pas

La recette de fidélité (`tools/bench/mes04/fidelity.py`, même révision, même graphe fp32 blake3 `7b6eb680…`) a été rejouée : 600 passages, 100 par structure, prompt `document: `, ONNX fp32 contre les vecteurs fp16 de production. Résultat : **minimum 0,9999604, p1 0,9999935, médiane 0,9999983**. Ces valeurs sont identiques à celles de `data/models/mdenseon/parity.json` au chiffre près : la recette est reproductible. L'écart au pire cas est `1 − 0,9999604 = 3,96e-5`, soit environ 25 fois la tolérance `1 − 0,999`.

Ce que la distribution couvre : l'arrondi fp16 des vecteurs de production tels qu'ils sont stockés et relus, et la chaîne documentaire complète (gabarit, tokenizer, révision, export) sur les six structures de l'échantillon. Le seuil refuse encore les vecteurs d'un mauvais export (cosinus 0,983 à 0,990, ADR-0050) et un lot encodé sans le prompt.

Ce qu'elle **ne couvre pas**, et qui est justement l'objet de Q-06 : aucun encodage sur MPS ou CUDA n'a été exécuté ici (la machine n'a pas de GPU exploitable par la campagne), aucun graphe ONNX fp16 n'a été mesuré, et aucun autre GPU ni autre révision n'a été essayé. La provenance des vecteurs de l'échantillon (pgvector `halfvec`, produit en natif sur MPS et en campagne sur CUDA) est documentée, pas réexécutée. Une valeur de 0,999 est donc un défaut **conservateur**, pas une borne prouvée pour tout matériel.

## Décision

Trois décisions, proposées au propriétaire.

### 1. Les requêtes sont encodées en fp32 par défaut : `[encoder].query_precision = "fp32"`

Le défaut de `[encoder].query_precision` passe de `int8` à `fp32`. Une instance de production encode donc ses requêtes avec le graphe fp32, et le rappel de recherche reste celui mesuré pour le fp32 (rappel@10 de 1,0 contre lui-même, 113/145 d'attendus).

Coûts assumés :

- la session fp32 (plus d'1 Gio) devient résidente dès la première requête, au lieu de l'INT8 (~0,3 Go). Une instance qui ne sert que des requêtes ne charge plus que le fp32 : le minimum de `docs/02` passe d'environ 1,2 Go à environ 2,0 Go, et une instance qui fait aussi des appels INT8 porte les deux sessions ;
- le p95 d'encodage mesuré (≈ 56 ms) dépasse la ligne de budget de `docs/03` ; le p50 (21 ms) y reste. Ce dépassement est accepté au nom du principe « le rappel prime », et devra être re-mesuré en conteneur (MES-05, P8) ;
- le premier appel coûte environ 1,8 s au lieu de 1,0 s (chargement du graphe, une fois par processus).

Le fichier `model.int8.onnx` **reste exporté et livré** dans le répertoire modèle, et `int8` reste une valeur acceptée de `query_precision`. Il sert aux comparaisons (MES-04), au repli et au diagnostic ; il ne disparaît pas du format du modèle, et l'empreinte continue de l'exclure (ADR-0051, ADR-0050).

Ce que cet ADR ne décide pas : une éventuelle requantification du graphe. Elle est notée comme piste future, sans engagement de tâche ni de calendrier (voir « Alternatives écartées »).

**Textes contredits, à modifier à l'acceptation** (le propriétaire demande, `@greffier` applique) :

| Document | Ce qui change |
|---|---|
| `docs/03-recherche.md` | étape 2 du pipeline (« Encodage en ONNX INT8 ») ; ligne du budget « Encodage de la requête (CPU, INT8) 20 à 50 ms » ; tableau « Réglages de précision » et son paragraphe (défaut `fp32`, `int8` en option) |
| `docs/04-donnees-et-ingestion.md` | « les requêtes sont encodées en INT8 (réglable par `[encoder].query_precision`…) » devient fp32 par défaut, INT8 en option |
| `docs/02-architecture.md` | tableau « Mémoire : ce qui est en RAM » et le paragraphe qui suit : le fp32 est chargé dès la première requête, le minimum passe à environ 2,0 Go, la session INT8 n'apparaît que si on l'utilise |
| `docs/07-stack-et-deploiement.md` | valeur par défaut de `[encoder].query_precision` dans le `crible.toml` d'exemple |
| ADR-0002 | la mention « ~0,3 Go en INT8 » devient fausse pour une instance qui sert des requêtes : **amender**, pas réécrire (une note datée renvoyant à cet ADR) |
| ADR-0051 | point 3 : le défaut `int8` est remplacé. Comme un ADR accepté ne se réécrit pas, la forme proposée est celle déjà employée pour ADR-0023 : la ligne d'index d'ADR-0051 porte « point 3 remplacé par ADR-0052 », et le propriétaire ajoute la même mention à son statut. C'est au propriétaire de le faire, pas au greffier. |

### 2. `acceptance_min_cosine` vaut **0,999**, et Q-06 est close avec cette valeur

Le seuil de recette de l'attache (contrôle 9 de `docs/04`, INV-12) est fixé à **0,999**. C'est déjà la valeur provisoire de la config d'ingestion de `docs/04` (`"acceptance_min_cosine": 0.999`) ; cet ADR la confirme au lieu de la laisser provisoire.

La valeur s'appuie sur la distribution mesurée : minimum 0,9999604, soit une marge d'environ 25 fois la tolérance. Elle reste assez large pour refuser un mauvais export (0,983 à 0,990) et un lot sans prompt.

**Q-06 est close avec 0,999.** La question portait sur des vecteurs encodés sur MPS ou CUDA en fp16 ; la campagne ne peut pas en produire, donc la valeur retenue est un défaut conservateur. La règle proposée est simple : si un producteur MPS ou CUDA échoue à la recette avec 0,999, la question est **rouverte** par un nouvel ADR, avec la distribution de ce producteur. On ne baisse pas le seuil pour faire passer un lot sans instruction explicite du propriétaire.

À documenter à l'acceptation : `docs/04` perd le caractère provisoire de la valeur, et la ligne Q-06 de `docs/adr/README.md` est retirée (ou marquée close) vers cet ADR.

### 3. Les réglages de session ONNX restent les défauts d'ONNX Runtime

`crible-encode` ne règle ni `intra_op_num_threads` ni le niveau d'optimisation de la session. MES-05 montre que les défauts sont les meilleurs des réglages essayés, dans les deux précisions. Le report de réglage laissé par `CHARGER-MODELE` se referme donc sans changement de code.

Si un réglage devait être ajouté plus tard, ce serait pour **brider** la consommation (4 threads), pas pour accélérer. La question est à revoir en conteneur avec MES-05 en P8, où les cœurs disponibles et le voisinage de charge changent.

## Conséquences

- **Positives** :
  - le rappel de recherche n'est plus amputé par l'approximation INT8 : 20,8 % de churn du top-10 disparaît ;
  - la chaîne d'encodage de requête est prouvée fidèle au modèle PyTorch à 1e-12, ce qui rend le diagnostic simple : un écart de résultat ne vient plus de l'encodeur ;
  - le seuil de recette quitte l'état provisoire, et `RECETTE-IMPORT` peut l'utiliser tel quel ;
  - aucune tâche de quantification n'est ouverte : le plan de P2 continue sans dépendance nouvelle ;
  - le réglage de session est tranché par la mesure : ni code, ni clé de configuration supplémentaire.
- **Négatives / coûts** :
  - plus d'1 Gio de RAM résident dès la première requête, contre ~0,3 Go : le budget de `docs/02` doit être réécrit, sinon il devient faux ;
  - p95 d'encodage à ≈ 56 ms sur le Mac de développement, au-dessus de la ligne de 20 à 50 ms de `docs/03` ; cette ligne doit être réécrite ou datée ;
  - premier appel à ≈ 1,8 s au lieu de 1,0 s par processus ;
  - six documents de conception à mettre à jour, plus une note sur ADR-0002 et une mention sur ADR-0051 ;
  - l'INT8 reste livré et maintenu comme option : deux chemins d'encodage à ne pas laisser diverger.
- **Le code à changer** : une seule valeur par défaut, `query_precision` (la clé est déjà réglable depuis `ENCODER-REQUETES`). À faire dans `NETTOYER-P2` ou dans une petite tâche dédiée. À noter aussi : `crible search --query` encode toujours en INT8 et n'a pas de chargeur `crible.toml` (constat hors mesure de MES-04) ; le changement de défaut ne l'atteindra pas tant que ce chargeur manque.
- **Tâches** : aucune tâche n'est bloquée. `CONTROLER-LONGUEURS` et la suite de P2 ne dépendent pas de cette décision ; `RECETTE-IMPORT` utilisera le seuil de 0,999.
- **À surveiller** :
  - la **résidence mémoire** en conteneur (P8) : si le budget de la VM Docker devient serré, la session fp32 permanente est le premier levier à réexaminer ;
  - la **latence d'encodage en conteneur** (MES-05, P8) : si le p95 fp32 y dépasse durablement la cible, la requantification redevient d'actualité ;
  - les **échecs de recette d'un producteur MPS ou CUDA** : signal de réouverture de Q-06 ;
  - le **taux de succès du cache de requêtes** (ADR-0051) : avec un encodage fp32 plus cher, un cache qui ne sert pas coûte plus qu'avant.

## Alternatives écartées

### Point 1 — Précision par défaut des requêtes

| Alternative | Pourquoi écartée |
|---|---|
| (b) Garder l'INT8 par défaut et accepter la perte | Le rappel@10 perd 20,8 % contre le fp32, et aucune structure ne dépasse 0,90. Cela contredit le principe « le rappel prime, la latence est indicative » : on paierait un rappel mesurablement dégradé pour gagner environ 12 ms de p50 et 800 Mo de RAM. |
| (c) Garder l'INT8, requantifier d'abord (statique, par canal, calibration sur des requêtes, couches sensibles en fp32), puis remesurer | C'est la bonne piste si le coût fp32 devient gênant, mais elle demande une campagne entière (export, MES-04 rejouée, validation) avant de savoir si elle retrouve le fp32, et rien ne garantit qu'elle y arrive. Elle retarderait J2 et le défaut actuel resterait mauvais entre-temps. Notée comme piste future, sans engagement. |
| (d) Choisir la précision par index ou par requête HTTP | Aucun besoin exprimé : `docs/05` n'a pas de champ de précision, et le choix est une propriété d'instance (ADR-0051). À instruire si un usage réel apparaît. |
| (e) Supprimer le fichier INT8 | Il sert aux comparaisons et au repli, et l'empreinte le référence déjà par son absence (ADR-0051). Le supprimer retirerait un moyen de diagnostic sans rien gagner en RAM, puisque le fichier n'est pas chargé quand le fp32 est seul. |

### Point 2 — Seuil de recette et sort de Q-06

| Alternative | Pourquoi écartée |
|---|---|
| Garder Q-06 **ouverte** pour le cas MPS/CUDA, avec 0,999 seulement pour le CPU | Laisse une question ouverte sans échéance, alors que la valeur est déjà la provisoire de `docs/04` et que `RECETTE-IMPORT` doit l'employer. La réouverture explicite, en cas d'échec d'un producteur GPU, couvre le même risque sans bloquer le plan. |
| Resserrer à 0,9995 | Le minimum observé (0,9999604) passerait encore, mais la mesure ne contient aucun cas MPS ou CUDA contrôlé : resserrer donnerait une fausse impression de borne prouvée, au risque de refuser des lots légitimes. |
| Baisser à 0,99 (cible de fidélité de `docs/08` pour le scan binaire) | Laisserait passer un export dégradé (0,983 à 0,990, ADR-0050), c'est-à-dire précisément ce que la recette doit détecter. La règle de 0,99 vise le scan binaire, pas l'encodage. |
| Ne rien décider et laisser la valeur provisoire | Une valeur provisoire dans un document de conception n'engage rien : `RECETTE-IMPORT` doit savoir quel seuil appliquer, et un changement ultérieur passerait sans trace. |

### Point 3 — Réglages de session ONNX

| Alternative | Pourquoi écartée |
|---|---|
| `intra_op_num_threads = 4` pour brider la consommation | Plus lent que le défaut (p50 9,6 ms contre 8,9 ms en INT8, 28,6 contre 24,2 ms en fp32) sans preuve qu'il protège utilement la machine. À réexaminer en conteneur (P8), où le partage des cœurs est réel. |
| Niveau d'optimisation `basic` | Plus lent en INT8 (11,9 ms de p50 contre 8,9 ms) et sans gain en fp32. |
| Nombre de threads déduit des cœurs physiques (8) | Nettement plus lent : le M3 a 8 cœurs mais 4 performance, et la session sature la machine (p95 113 ms en fp32 contre 60 ms au défaut). |

## Références

- Mesures : `docs/mesures/MES-04-parite-encodeur.md` et `.2026-10-24.json`, `docs/mesures/MES-05-latence-encodage.md` et `.2026-10-24.json` (relues OK).
- Fiche : `docs/phases/P2-encodeur-et-recette.md`, `### MESURER-PARITE-ENCODEUR`.
- Conception : `docs/02-architecture.md` (« Mémoire : ce qui est en RAM »), `docs/03-recherche.md` (pipeline, « Réglages de précision », budget de latence), `docs/04-donnees-et-ingestion.md` (« Le modèle », config d'ingestion, contrôle 9), `docs/05-api.md` (limites par défaut), `docs/07-stack-et-deploiement.md` (`crible.toml`), `docs/08-invariants-et-tests.md` (MES-04, MES-05, INV-12).
- ADR liés : ADR-0002 (budget mémoire d'une instance), ADR-0050 (référence mDenseOn et export ONNX), ADR-0051 (cache des requêtes, sessions INT8 et fp32, nom et défaut de `query_precision`), ADR-0028 (empreinte du modèle).
- Question ouverte tranchée : Q-06 (`acceptance_min_cosine` pour des vecteurs MPS ou CUDA en fp16), `docs/adr/README.md`.
- Principe : `docs/suivi.md`, section « Cibles » (2026-10-20) — « le rappel prime, la latence est indicative ».
