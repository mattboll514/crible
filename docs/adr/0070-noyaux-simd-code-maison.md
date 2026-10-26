# ADR-0070 — Noyaux SIMD : code maison dans `crible-search::kernels`

- **Statut** : Accepté (accepté par anticipation le 2026-10-26 par délégation du propriétaire pour les trois premières tâches de P4, à confirmer au point de fin)
- **Date** : 2026-10-26 (proposition et acceptation anticipée)
- **Décideurs** : propriétaire
- **Décision** : accepté par anticipation le 2026-10-26, par délégation du propriétaire pour les trois premières tâches de P4, à confirmer au point de fin de la série. La décision est retenue telle que proposée : code maison dans `crible-search::kernels` (modules `scalar`, `neon`, `x86`), intrinsèques NEON sur `aarch64`, AVX2 sur `x86_64`, repli scalaire et détection à l'exécution, sans aucune crate dédiée ; les micro-benchmarks sont consignés dans cet ADR.
- **Proposé par** : agent greffier, sur demande de l'orchestrateur, pour la tâche `NOYAUX-SIMD` (phase P4), à partir du micro-benchmark et du choix retenus par `@dev`
- **Remplace** : —

## Contexte

### Ce que la fiche demande

La fiche `NOYAUX-SIMD` (`docs/phases/P4-recherche-rapide.md:29-42`) demande deux noyaux dans `crible-search::kernels` — la distance de Hamming (popcount sur des `u64`) et le produit scalaire fp16 × f32 — avec des implémentations NEON (`aarch64`) et AVX2 (`x86_64`), un repli scalaire et une détection à l'exécution. Elle demande aussi des benchmarks `criterion`, et « le choix entre une crate dédiée et le code maison, justifié dans le rapport ». Elle fixe deux critères : un proptest compare chaque noyau SIMD au scalaire (égalité exacte pour Hamming, `1e-5` pour le produit scalaire), et chaque `unsafe` porte un commentaire `SAFETY`. Les benchmarks doivent être consignés : c'est le rôle de cet ADR.

La fiche ne tranche pas le choix. Elle ne fixe non plus aucune cible de performance : aucun MES ne porte sur les noyaux.

### Ce que la conception décrit

- `docs/07-stack-et-deploiement.md:37` laisse la ligne « Noyaux SIMD » ouverte : « à trancher (crate dédiée ou intrinsèques NEON/AVX2 avec repli scalaire) », avec « Micro-benchmark dans `NOYAUX-SIMD` ».
- `docs/07:36` liste déjà `half` pour les vecteurs ; `docs/07:48` liste `proptest` et `criterion` pour les tests. Ces trois crates sont donc dans la conception.
- `docs/03-recherche.md:46` et `:50` fixent ce que les noyaux calculent : la stratégie BINARY fait « un scan de Hamming sur le pool → top-N → rescoring fp16 → tri », et la distance de Hamming est « le popcount du XOR sur des mots `u64` ». La stratégie EXACT (`:45`) fait le produit scalaire fp16 sur tout le pool.
- `docs/08-invariants-et-tests.md:31` et `:37` placent les noyaux dans les tests unitaires et les micro-benchmarks `criterion`. Aucun invariant INV-xx ne porte sur les noyaux : la fidélité du résultat est prouvée par la comparaison au scalaire, pas par un invariant numéroté.
- **ADR-0069**, décision 6 (`docs/adr/0069-reconstruction-et-format-de-codes-bin.md:95`) fixe la forme d'un code : `ceil(dims/64)` mots `u64` little-endian, un bit par dimension. À 768 dimensions, cela fait 12 mots, la taille de production.

### Les faits qui pèsent

- **Deux noyaux seulement**, tous deux sans état : un XOR suivi d'un popcount, et une suite de produits sur des vecteurs normalisés. Une crate dédiée devrait couvrir ces deux calculs et rien de plus.
- **Le produit scalaire est le poste coûteux.** Il s'exécute sur tout le pool en EXACT et sur les N candidats en BINARY ; Hamming, lui, porte sur 12 mots par passage. Un facteur 4 sur le produit scalaire pèse donc plus qu'un facteur sur Hamming.
- **La machine de développement est `aarch64`** (Apple Silicon, NEON et extension fp16), avec un jeu de benchmarks `criterion` et le `rustc` épinglé. Le chemin AVX2 ne peut pas y être mesuré.
- **Le déploiement est `aarch64`** (`docs/07:133` : « le SIMD NEON est disponible »), et la CI tourne sur `ubuntu-latest`, donc `x86_64` : le chemin AVX2 n'est exercé qu'en CI.
- **Le contrat de robustesse du projet** interdit `unwrap()`, `expect()` et `panic!` hors des tests, et demande un commentaire `SAFETY` et un test pour chaque `unsafe` (`AGENTS.md`, règle 10). Un noyau écrit à la main est du code `unsafe` à relire ; une crate dédiée déplacerait ce risque chez un tiers.

### Ce qui n'est pas décidé ici

Cet ADR ne choisit ni N, ni `exact_threshold`, ni le parallélisme du scan (MES-01, Q-01, `SCANNER-POOL-BINAIRE`). Il ne traite ni l'int8, ni le scoring asymétrique, que la fiche met hors périmètre. Il ne fixe aucune cible de latence : il consigne un choix et des chiffres indicatifs. Il ne modifie ni le format de `codes.bin` (ADR-0069), ni la liste des dépendances de `docs/07`.

## Décision

**Le code maison est retenu : aucune crate dédiée.** Les noyaux vivent dans `crible-search::kernels`, en trois modules — `scalar` (référence portable), `neon` (`aarch64`), `x86` (`x86_64`) —, derrière deux fonctions publiques, `hamming(&[u64], &[u64]) -> u64` et `dot(&[f16], &[f32]) -> f32`.

### 1. Hamming : popcount du XOR sur des `u64`, exact sur tous les backends

Le noyau compte les bits du XOR des deux codes, mot par mot. Le résultat est exact quel que soit le backend : aucune tolérance n'est nécessaire, et le proptest exige l'égalité stricte avec le scalaire.

Les deux noyaux sont **totaux** : ils lisent `min(a.len(), b.len())` éléments et rendent la contribution du préfixe commun, au lieu de paniquer ou de lire hors bornes. Les appelants de production passent toujours des longueurs égales.

### 2. Produit scalaire fp16 × f32, accumulé en `f64`

Chaque composante fp16 est élargie en `f32`, multipliée par la composante `f32` de la requête, et la somme est accumulée en `f64` sur tous les backends. Seul l'ordre de sommation change d'un backend à l'autre : l'écart est un arrondi sur les derniers bits du `f64`, très en dessous de la tolérance `1e-5` de la fiche. Les vecteurs sont normalisés en amont, donc ce produit est le cosinus sur lequel le rescoring classe.

### 3. Détection à l'exécution, un backend par noyau

Le backend est choisi une fois, au premier appel, et gardé dans un `OnceLock`. Les deux noyaux sont sélectionnés **indépendamment**, parce que les jeux d'instructions ne coïncident pas :

- `aarch64` : NEON pour Hamming. Le produit scalaire NEON demande l'extension fp16 (conversion `vcvt_f32_f16` dans la boucle) ; sans elle, Hamming reste vectorisé et le produit scalaire prend le scalaire.
- `x86_64` : AVX2 pour Hamming. Le produit scalaire demande F16C pour la conversion fp16 → fp32 ; sans F16C, seul Hamming est vectorisé.
- Partout ailleurs, ou sans le jeu d'instructions : repli scalaire.

NEON fait partie de la base `aarch64`, mais la détection tourne quand même : un même binaire se comporte ainsi de la même façon sur tout processeur qui annonce l'architecture.

### 4. Chaque `unsafe` porte un commentaire `SAFETY` et un test

C'est le critère de la fiche. Le proptest compare chaque backend au scalaire — égalité exacte pour Hamming, `1e-5` pour le produit scalaire — et un test unitaire fixe la règle du préfixe commun. Le scalaire sert à la fois d'oracle des tests et de base du benchmark.

### 5. Aucune dépendance hors de `docs/07`

`half` est déjà une dépendance de `crible-search` et figure en `docs/07:36`. `criterion` et `proptest` passent en `dev-dependencies` et figurent en `docs/07:48`. Le noyau lui-même n'utilise que `std::arch` et `half` : aucune crate nouvelle, donc aucun ADR de dépendance.

### 6. Les benchmarks sont consignés ici

Protocole : `cargo bench -p crible-search --bench kernels` en `--release`, réglages par défaut de `criterion` (3 s de chauffe, 5 s de mesure, 100 échantillons), entrées en cache, un seul thread, un benchmark à la fois. Machine : Apple Silicon (`aarch64`, NEON et fp16), `rustc 1.99.1`, macOS, le 2026-10-26, machine de développement non au repos. Valeurs en nanosecondes par appel.

| Noyau | Taille | Scalaire | SIMD maison (dispatch inclus) | Crate `wide` 1.x |
|---|---|---|---|---|
| Hamming | 128 bits (2 mots) | 1,36 | 1,86 | — |
| Hamming | 768 bits (12 mots) | 3,60 | 3,55 | — |
| Hamming | 4 096 bits (64 mots) | 9,15 | 9,29 | — |
| Produit scalaire | d = 128 | 58,2 | 15,2 | 31,4 |
| Produit scalaire | d = 768 | 491,6 | 119,3 | 183,7 |
| Produit scalaire | d = 4 096 | 2 814 | 758 | 971 |
| Dispatch seul | — | — | 1,33 (Hamming), 1,59 (produit scalaire) | — |

**Lecture des chiffres.**

- **Produit scalaire** : le noyau maison va environ 4 fois plus vite que le scalaire et 1,3 à 2,1 fois plus vite que `wide`. `wide` n'a pas de type fp16 : il élargit par `half` avant le produit `f32x8`, et cette passe de conversion scalaire lui coûte la différence. Le noyau NEON convertit dans la boucle vectorielle.
- **Hamming sur `aarch64`** : le SIMD maison ne gagne rien, parce que LLVM vectorise déjà la boucle `count_ones` en `cnt` sur un registre de 128 bits. À 12 mots, les deux se valent (3,55 ns contre 3,60 ns, dispatch de 1,33 ns inclus dans le chiffre SIMD) ; à 4 096 bits aussi, dans le bruit. Le chemin AVX2 est gardé pour `x86_64`, où un `popcnt` scalaire traite 64 bits par cycle là où AVX2 en compte 256.
- **Dispatch** : 1,33 à 1,59 ns par appel, soit environ 40 % d'un Hamming à 12 mots. Son amortissement n'est pas décidé ici : il revient à `SCANNER-POOL-BINAIRE`, qui peut résoudre le noyau une fois par scan ou exposer un point d'entrée par lot.

### 7. Réserves sur les mesures

- La colonne `wide` a été mesurée **hors du dépôt**, sur la même machine et les mêmes données, avec `wide` 1.x. Elle n'est pas reproductible par la commande du dépôt.
- Le **chemin AVX2 n'est pas mesuré** : la machine est `aarch64`. Il n'est exercé qu'en CI Linux `x86_64` (`ubuntu-latest`), par les tests de comparaison au scalaire, pas par un benchmark.
- La machine de développement n'était pas au repos : les chiffres sont **indicatifs**. Ils tranchent un choix d'implémentation, ils ne sont pas une cible. C'est la règle de `docs/suivi.md` (section « Cibles ») : « le rappel prime, la latence est indicative ».
- Aucun MES ne couvre les noyaux : ces chiffres ne se comparent à aucun budget de `docs/03-recherche.md`.

## Conséquences

- **Positives** :
  - aucune dépendance nouvelle : `half` était déjà là, `criterion` et `proptest` aussi ;
  - le produit scalaire, poste dominant du rescoring et de l'EXACT, va environ 4 fois plus vite que le scalaire sur la machine de développement ;
  - une seule implémentation par architecture, avec un repli scalaire qui garantit que le moteur tourne partout ;
  - le contrat de fidélité est explicite et testé : Hamming exact, produit scalaire à `1e-5`, sur chaque backend ;
  - le code reste dans le dépôt : il se lit, se teste et se profile avec le reste, sans dépendre de la politique de version d'un tiers.
- **Négatives / coûts** :
  - du code `unsafe` à relire et à maintenir, sur deux architectures, pour deux noyaux seulement ;
  - le chemin AVX2 n'est pas mesuré localement : sa performance réelle sur `x86_64` reste inconnue tant qu'aucun benchmark n'y tourne ;
  - sur `aarch64`, le noyau Hamming n'apporte rien face à LLVM : il ajoute un dispatch et du code à maintenir pour un gain nul sur cette architecture ;
  - le dispatch coûte 1,33 à 1,59 ns par appel, soit environ 40 % d'un Hamming de production : un scan qui appelle noyau par noyau en paie le prix à chaque passage ;
  - la comparaison avec `wide` n'est pas rejouable depuis le dépôt ;
  - deux détections distinctes (fp16, F16C) ajoutent des combinaisons à couvrir : un processeur peut vectoriser Hamming sans vectoriser le produit scalaire.
- **À surveiller** :
  - la latence réelle du scan et du rescoring, relevée par `SCANNER-POOL-BINAIRE` puis par MES-02, MES-03 et MES-09 : c'est là que le gain du noyau se voit, ou pas ;
  - le coût du dispatch par passage : s'il pèse, la réponse est un point d'entrée par lot ou un noyau résolu une fois par scan, décidé dans `SCANNER-POOL-BINAIRE`, pas ici ;
  - le premier résultat du benchmark ou des tests AVX2 sur `x86_64` : s'il révèle un écart de fidélité ou une absence de gain, l'ADR est à revoir ;
  - l'arrivée d'un troisième noyau (int8, scoring asymétrique, autre modèle de dimensions) : c'est le signal pour rouvrir la question de la crate dédiée, aujourd'hui écartée pour deux noyaux.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| La crate `wide` | Plus lente que le noyau maison sur les trois tailles mesurées (31,4 contre 15,2 ns à d = 128 ; 183,7 contre 119,3 à d = 768 ; 971 contre 758 à d = 4 096), et sans type fp16 : la conversion fp16 → f32 reste scalaire, ce qui annule l'avantage du produit vectoriel |
| Une crate dédiée de distances | Non mesurée, et elle ajouterait une dépendance pour deux noyaux seulement. Le noyau maison couvre déjà les deux calculs avec `std::arch` et `half`, tous deux dans `docs/07` |
| Le scalaire seul | Le plus simple, mais environ 4 fois plus lent sur le produit scalaire (491,6 contre 119,3 ns à d = 768), qui est le poste dominant du rescoring et de l'EXACT. Le repli scalaire reste présent pour les architectures sans jeu d'instructions, mais il n'est pas le chemin par défaut sur `aarch64` et `x86_64` |

## Documents modifiés après acceptation

Appliqué dans le même travail que cet ADR, sur la branche de la tâche `NOYAUX-SIMD` :

| Document | Modification |
|---|---|
| `docs/07-stack-et-deploiement.md:37` | remplacer « à trancher (crate dédiée ou intrinsèques NEON/AVX2 avec repli scalaire) » par le choix retenu : code maison `std::arch` dans `crible-search::kernels`, NEON sur `aarch64`, AVX2 sur `x86_64`, repli scalaire, détection à l'exécution, renvoi à ADR-0070 ; la colonne « Remarque » renvoie aux benchmarks consignés dans cet ADR |
| `docs/adr/README.md` | ajouter la ligne d'index de cet ADR |

`docs/suivi.md`, le code et les tests ne sont pas touchés par cet ADR. Aucun autre document de conception n'est modifié.

## Références

- Fiche `NOYAUX-SIMD` : `docs/phases/P4-recherche-rapide.md:29-42`
- `docs/07-stack-et-deploiement.md:36-37` : `half`, ligne « Noyaux SIMD » (tranchée ici) ; `:48` : `proptest`, `criterion` ; `:133` : NEON disponible sur le Mac ; `:135-147` : CI
- `docs/03-recherche.md:45-50` : stratégies EXACT et BINARY, code binaire et distance de Hamming
- `docs/08-invariants-et-tests.md:31`, `:37` : tests unitaires et micro-benchmarks `criterion`
- `crates/crible-search/src/kernels/mod.rs`, `scalar.rs`, `neon.rs`, `x86.rs` : implémentation
- `crates/crible-search/benches/kernels.rs` : protocole et chiffres du micro-benchmark
- ADR-0004 (codes binaires, scan du pool et rescoring), ADR-0069 (format de `codes.bin`)
