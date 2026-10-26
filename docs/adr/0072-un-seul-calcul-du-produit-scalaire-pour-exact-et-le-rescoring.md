# ADR-0072 — Un seul calcul du produit scalaire pour EXACT et le rescoring

- **Statut** : Accepté (accepté par anticipation le 2026-10-26, par délégation du propriétaire pour la série RESCORER-FP16, INSERER-POOL-EN-ORDRE, ACCELERER-RECETTE, PLANIFIER-STRATEGIE ; à confirmer par le propriétaire au point de fin)
- **Date** : 2026-10-26 (proposition et acceptation anticipée)
- **Décideurs** : propriétaire
- **Décision** : accepté par anticipation le 2026-10-26, par délégation du propriétaire pour la série RESCORER-FP16, INSERER-POOL-EN-ORDRE, ACCELERER-RECETTE, PLANIFIER-STRATEGIE, à confirmer par le propriétaire au point de fin. La décision est retenue telle que proposée, c'est-à-dire l'option A de `@dev`, recommandée par `@architecte` : un seul chemin de calcul du produit scalaire, partagé par EXACT et par le rescoring, avec `kernels::dot` pour une référence fp16 et une fonction scalaire unique accumulée en `f64` pour une référence fp32 ; la clause arithmétique d'ADR-0045 est remplacée, le reste d'ADR-0045 est inchangé.
- **Proposé par** : agent greffier, sur demande de l'orchestrateur, pour la tâche `RESCORER-FP16` (phase P4), à partir d'une question de conception de `@dev` et du verdict de `@architecte`
- **Remplace** : ADR-0045 (sa clause arithmétique seulement : « le calcul du moteur ne change pas : il reste en f32, dans l'ordre des composantes »)

## Contexte

### Ce que la fiche demande

La fiche `RESCORER-FP16` (`docs/phases/P4-recherche-rapide.md:53-57`) demande le score exact des N candidats. Son livrable est « la lecture des vecteurs de référence des N candidats (mmap), le produit scalaire via les noyaux, puis le tri final » (`:56`). Son unique critère d'acceptation est : « quand N est au moins égal à la taille du pool, le résultat est identique à la stratégie EXACT » (`:57`).

La fiche ne dit pas **ce que « identique » veut dire** : mêmes clés seulement, ou mêmes clés et mêmes scores ; ni si les scores doivent être égaux au bit près ou dans la tolérance de `1e-5` d'INV-06. Elle ne dit pas non plus quel calcul sert une référence `fp32`.

### Deux arithmétiques aujourd'hui

- **EXACT** somme en `f32`, dans l'ordre des composantes : `crates/crible-search/src/dense/exact.rs:426-432` (fonction `dot`), appelée en `:285`. ADR-0045 a figé cette arithmétique comme une clause de sa décision : « le calcul du moteur ne change pas : il reste en f32, dans l'ordre des composantes » (`docs/adr/0045-reference-de-synth-small-calculee-exactement-independante-de-la-blas.md:29`, `:47`, et `:97` en référence). Cette clause protège INV-06 d'une dépendance à une largeur SIMD.
- **Le noyau** `kernels::dot` (ADR-0070, décision 2, `docs/adr/0070-noyaux-simd-code-maison.md:55-57`) multiplie fp16 × f32 et accumule en `f64`, avec un seul arrondi final en `f32` ; il est implémenté en `crates/crible-search/src/kernels/mod.rs:119-122`, sur le repli scalaire `crates/crible-search/src/kernels/scalar.rs:33-40`.

Les deux calculs ne rendent pas les mêmes bits. Sur 100 vecteurs fp16 et une requête normalisée, relevé par `@dev` dans sa question de conception, 94 scores sur 100 diffèrent du calcul `f32` séquentiel au dernier bit, pour un écart au plus de 2,4e-7. Les deux valeurs sont correctes à la tolérance d'INV-06 près, mais elles ne sont pas égales au bit.

### Pourquoi le noyau gagne

ADR-0070 justifie déjà le noyau comme le poste dominant « du rescoring et de l'EXACT » (`docs/adr/0070-noyaux-simd-code-maison.md:108`), et écarte le scalaire seul pour cette raison (`:131`). La fiche `RESCORER-FP16` demande explicitement « le produit scalaire via les noyaux » (`docs/phases/P4-recherche-rapide.md:56`) : réimplémenter un second calcul pour EXACT reviendrait à payer deux fois le même poste, et à maintenir deux arithmétiques.

### Ce que la conception ne couvre pas

- **INV-06** (`docs/08-invariants-et-tests.md:14`) : « Pool ≤ seuil exact ⇒ résultat identique au calcul exact », comparé « avec la référence numpy sur le jeu synthétique ». Il compare donc le **moteur à la référence numpy**, pas deux stratégies du moteur entre elles. Le sens de « identique » entre EXACT et le rescoring n'y est pas défini.
- **`reference_precision: fp32`** : `docs/03-recherche.md:59` autorise `fp16` ou `fp32`, figé à la création. ADR-0070 ne couvre que fp16 × f32 : ni son noyau, ni ses benchmarks, ni ses tests ne traitent un produit fp32 × f32.

`@architecte` a rendu deux verdicts : **ECART**, parce que l'option retenue contredit la clause arithmétique d'ADR-0045, qui est `Accepté` ; et **NON-COUVERT**, pour le sens de « identique » du critère de la fiche et pour la référence `fp32`. Ces deux points sont tranchés ici, et c'est cet ADR qui autorise EXACT à abandonner la somme `f32` séquentielle.

## Décision

**Un seul chemin de calcul du produit scalaire, partagé par EXACT et par le rescoring.** Les deux stratégies appellent la même fonction, sur la même machine, avec la même arithmétique.

Précisions :

1. **Référence `fp16`.** EXACT et le rescoring appellent `kernels::dot` (`crates/crible-search/src/kernels/mod.rs:119-122`), avec le dispatch SIMD à l'exécution d'ADR-0070. EXACT abandonne la somme `f32` séquentielle de `crates/crible-search/src/dense/exact.rs:426-432`. Cette décision **remplace la clause arithmétique d'ADR-0045** ; le reste d'ADR-0045 est inchangé : la référence numpy reste exacte et indépendante de BLAS, le comparateur reste à `1e-5` avec départage stable par clé.
2. **Référence `fp32`.** La ligne fp32 est lue telle quelle, sans conversion. Le produit est calculé par une **fonction partagée unique** : produits et accumulation en `f64`, un seul arrondi en `f32` à la fin, comme le noyau fp16. Elle est scalaire : aucun noyau SIMD fp32 × f32 n'est écrit tant qu'aucune mesure ne le justifie. EXACT et le rescoring l'appellent tous deux. Le produit fp32 × fp32 est exact en `f64`, donc seule l'accumulation ordonne le résultat, et elle l'ordonne de la même façon pour les deux stratégies.
3. **Sens du critère de `RESCORER-FP16`.** Quand N est au moins égal à la taille du pool, le rescoring rend **les mêmes clés, dans le même ordre** (départage stable par clé `doc_id#ord`), avec des **scores identiques au bit près**. C'est une conséquence de la décision 1 : les deux stratégies passent par la même fonction sur la même machine. La comparaison à la référence numpy reste celle d'ADR-0045 : `1e-5`, départage par clé, quasi-égalités admises.
4. **Portée.** La décision porte sur le produit scalaire du moteur. Elle ne change ni la normalisation de la requête, ni le comparateur d'INV-06, ni le format des segments, ni la précision de référence, qui reste figée à la création de l'index (`docs/03-recherche.md:59`).

Ce qui n'est **pas** décidé ici :

- la valeur des décomptes de quasi-égalités, recalculée par la tâche (voir les conséquences) ;
- un noyau SIMD fp32 × f32, qui demanderait une mesure ;
- le rangement des scores, la fusion, le regrouping, et tout le reste de P4.

## Conséquences

- **Positives** :
  - une seule arithmétique à écrire, tester et maintenir, au lieu de deux : le noyau mesuré d'ADR-0070 sert les deux stratégies ;
  - le critère de `RESCORER-FP16` devient vérifiable au bit près (`mêmes clés, même ordre, mêmes scores`), au lieu d'une ressemblance à arbitrer ;
  - EXACT gagne la vitesse du noyau sur son poste dominant, sans changer de résultat observable au-delà du dernier bit ;
  - la référence `fp32`, jusqu'ici sans calcul défini, en reçoit un, commun aux deux stratégies.
- **Négatives / coûts** :
  - **les scores du moteur changent au dernier bit par rapport à P1** : les valeurs enregistrées en P1 ne sont plus reproduites à l'identique, seulement dans `1e-5` ;
  - **INV-06 se revérifie à `1e-5`** : la référence numpy ne bouge pas, le moteur s'en écarte d'un ou deux bits de plus ;
  - **les décomptes de quasi-égalités qu'ADR-0045 fige en assertions exactes sont à recalculer** (`docs/adr/0045-reference-de-synth-small-calculee-exactement-independante-de-la-blas.md:39`, valeurs **4** requêtes sous 1e-6 et **55** sous 1e-5) : `RESCORER-FP16` les recalcule avec la nouvelle arithmétique, et les nouvelles valeurs sont reportées **dans cet ADR par un erratum** si elles changent. L'assertion reste exacte, elle n'est pas transformée en propriété ;
  - les scores `fp32` restent calculés en scalaire : aucun gain de noyau sur cette voie, qui n'est pas celle de production par défaut (`docs/03-recherche.md:59`, défaut `fp16`).
- **À surveiller** :
  - **scalaire et SIMD peuvent différer au dernier bit d'une machine à l'autre** : ADR-0070 l'admet déjà. Cela ne touche pas l'égalité entre deux stratégies **sur une même machine**, puisque le backend est choisi une fois par processus (`crates/crible-search/src/kernels/mod.rs:119-122`, `OnceLock`) ; un même processus ne mélange donc pas deux backends ;
  - le premier rejeu d'INV-06 sur les deux plateformes de la CI : un écart supérieur à `1e-5` signalerait un défaut du noyau, pas une tolérance à élargir ;
  - les décomptes recalculés, s'ils s'éloignent fortement de 4 et 55 : ce serait le signe que la nouvelle arithmétique déplace les quasi‑égalités du jeu, et non seulement leur dernier bit ;
  - l'apparition d'une référence `fp32` en production, qui rendrait un noyau fp32 × f32 mesurable, donc la décision 2 révisable par un nouvel ADR.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| **B** — garder la somme `f32` séquentielle pour EXACT et n'utiliser le noyau que pour le rescoring | C'est l'état actuel, et il rend le critère de la fiche faux : à N ≥ taille du pool, les deux stratégies diffèrent au dernier bit (94 scores sur 100). Il faudrait alors affaiblir le critère à `1e-5` et renoncer au gain du noyau sur EXACT, contre la lettre de la fiche (`docs/phases/P4-recherche-rapide.md:56`) et l'argument d'ADR-0070 (`docs/adr/0070-noyaux-simd-code-maison.md:108`) |
| **C** — réécrire le noyau en `f32` séquentiel, pour que tout le moteur retrouve l'arithmétique d'ADR-0045 | Cela annule le gain mesuré d'ADR-0070 (environ 4 fois sur le produit scalaire, `:131`) et refait le travail de `NOYAUX-SIMD`. Cela ne résout rien : il suffit d'une largeur SIMD différente pour changer l'ordre de sommation, ce qu'ADR-0045 voulait précisément éviter de laisser au hasard |
| **D** — un second calcul explicite pour EXACT, égal au noyau, mais écrit à part | Deux implémentations de la même arithmétique divergent à la première correction. Le critère « identique au bit près » ne tiendrait que par convention, non par construction |
| **E** — définir « identique » comme `1e-5` seulement, sans toucher à ADR-0045 | Laisse deux arithmétiques, et le critère de la fiche devient plus faible que ce que la même fonction partagée permet. Une assertion à `1e-5` ne détecte plus une dérive de dernier bit, alors que la règle 5 d'`AGENTS.md` veut un test qui échoue quand le critère n'est plus tenu |
| **F** — écrire un noyau SIMD fp32 × f32 dans le même travail | Aucune mesure ne le justifie : `fp32` n'est pas la précision par défaut, et `NOYAUX-SIMD` n'a mesuré que fp16 × f32. La règle 12 d'`AGENTS.md` écarte l'abstraction sans besoin ; la décision 2 le réserve à une mesure |

## Documents modifiés après acceptation

Appliqué dans le même travail que cet ADR, sur la branche de la tâche `RESCORER-FP16` :

| Document | Modification |
|---|---|
| `docs/adr/README.md` | ajouter la ligne d'index de cet ADR ; signaler sur la ligne d'ADR-0045 que sa clause arithmétique est remplacée par ADR-0072 |
| `docs/adr/0045-reference-de-synth-small-calculee-exactement-independante-de-la-blas.md` | note en tête : la clause arithmétique est remplacée par ADR-0072 ; la décision d'ADR-0045 n'est pas réécrite |
| `docs/phases/P4-recherche-rapide.md:57` | préciser le critère de `RESCORER-FP16` selon la décision 3, et renvoyer à ADR-0072 |

`docs/suivi.md`, le code et les tests ne sont pas touchés par cet ADR : le suivi est tenu par l'orchestrateur, et la mise en œuvre appartient à `RESCORER-FP16`.

## Références

- Fiche `RESCORER-FP16` : `docs/phases/P4-recherche-rapide.md:53-57`
- `crates/crible-search/src/dense/exact.rs:426-432` (fonction `dot`, somme `f32` séquentielle), appel en `:285`
- `crates/crible-search/src/kernels/mod.rs:119-122` (`kernels::dot`, dispatch à l'exécution) ; `crates/crible-search/src/kernels/scalar.rs:33-40` (repli scalaire, accumulation `f64`)
- ADR-0045 : clause arithmétique (`docs/adr/0045-reference-de-synth-small-calculee-exactement-independante-de-la-blas.md:29`, `:47`, `:97`), comparateur et départage (`:35`), décomptes 4 et 55 (`:39`)
- ADR-0070 : décision 2 (produit scalaire fp16 × f32 accumulé en `f64`, `docs/adr/0070-noyaux-simd-code-maison.md:55-57`), poste dominant du rescoring et de l'EXACT (`:108`), alternative du scalaire seul (`:131`)
- `docs/03-recherche.md:44-46` (table EXACT / BINARY), `:59` (`reference_precision` `fp16` ou `fp32`, figé à la création)
- `docs/08-invariants-et-tests.md:14` (INV-06 : le moteur comparé à la référence numpy)
- `AGENTS.md`, règles 4 (ADR pour tout compromis), 5 (un test par critère) et 12 (pas d'abstraction sans besoin)
