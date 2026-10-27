# ADR-0076 — Protocole de mesure de fidélité du scan binaire : métriques, centrage, pools et requêtes

- **Statut** : Accepté (accepté par anticipation le 2026-10-27, par délégation du propriétaire pour la série REGROUPER-RESULTATS, MESURER-FIDELITE-BINAIRE, MESURER-LATENCE-DENSE ; confirmation du propriétaire attendue à la fin de la série)
- **Date** : 2026-10-27 (proposition et acceptation anticipée)
- **Décideurs** : propriétaire
- **Décision** : accepté par anticipation le 2026-10-27, par délégation du propriétaire pour la série REGROUPER-RESULTATS, MESURER-FIDELITE-BINAIRE, MESURER-LATENCE-DENSE ; la confirmation du propriétaire est attendue à la fin de la série. Les six décisions sont retenues telles que proposées par `@architecte` et choisies par l'orchestrateur : recall@10 au niveau du passage et recouvrement@10 au niveau du document, avec la cible de 0,99 en moyenne globale au N par défaut ; un tableau par structure plus la moyenne ; centrage mesuré hors moteur par un harnais contrôlé au bit près, sans changement du moteur ni du format ; mesure sur l'échantillon, BINARY forcé par `search.exact_threshold = 0` ; 300 requêtes fp32, campagne principale sans filtre et variante filtrée déclarée ; levier proposé, non codé, si la cible manque.
- **Proposé par** : agent greffier, sur demande de l'orchestrateur, pour la tâche `MESURER-FIDELITE-BINAIRE` (phase P4), à partir des verdicts de `@architecte` rendus avant de confier la mesure à `@mesureur` ; les options sont retenues par l'orchestrateur par délégation
- **Remplace** : —

## Contexte

### Ce que la fiche demande

La fiche `MESURER-FIDELITE-BINAIRE` (`docs/phases/P4-recherche-rapide.md:84-92`) demande MES-01 et MES-12, puis le choix de N. Son livrable est « le recall@10 et le recouvrement@10 de BINARY contre EXACT sur les requêtes réelles de l'échantillon, pour N ∈ {200, 500, 1000, 2000, 5000}, avec et sans centrage, et par structure d'index » (`:88`). Ses critères sont : « le rapport contient une recommandation de N et de centrage », et « la cible est atteinte, ou bien le rapport propose un levier (scoring asymétrique) à faire valider » (`:89-91`). La fiche ne définit ni les deux métriques, ni le protocole de mesure du centrage, ni la taille des pools, ni le traitement des filtres.

### Ce que la conception fixe

- **Les deux métriques sont nommées, pas définies.** `docs/06-observabilite.md:104` décrit la mesure de fidélité de l'éval comme « recall@k et recouvrement@k du pipeline normal contre le même pipeline en `exact: true` ». `docs/08-invariants-et-tests.md:85` ne fixe que la cible : « le recall@10 de l'approximation contre l'exact doit atteindre au moins 0,99 au N par défaut ». Il n'existe aucune définition normative du dénominateur, du niveau (passage ou document) ni de l'agrégation.
- **Le précédent mesuré existe.** `docs/mesures/MES-04-parite-encodeur.md` §3 rapporte un « Rappel@10 moyen » par structure et un « Accord document@10 », avec une ligne **Global** sur les 300 requêtes. C'est la forme que le propriétaire a déjà lue et validée.
- **P7 doit hériter de la définition.** `crible eval` n'arrive qu'en P7 (`docs/phases/P7-observabilite.md`, `docs/06-observabilite.md:92`). Une définition propre à la mesure de P4 ferait diverger le rapport et l'outil qui le rejouera.
- **Le centrage est une question ouverte de conception.** `docs/03-recherche.md:50` écrit que le bit vaut `x_i > 0` et qu'« un centrage préalable est à étudier (MES-12) » ; `docs/08:77` porte MES-12 comme « Codes binaires : signe brut contre signe après centrage ». Rien ne fixe ce qu'est « centrer », ni sur quels vecteurs.
- **`codes.bin` est dérivé et versionné par le segment.** Le bit `i` vaut `x_i > 0` et les bits de bourrage valent zéro ; le fichier n'a pas d'en-tête et sa version est `format_version` de `segment.json` (ADR-0069, décision 6). Un harnais hors moteur peut donc recalculer des codes en RAM sans toucher au format.
- **Un seul index par structure.** ADR-0003 fixe un index par structure de données ; `docs/08:56` décrit l'échantillon comme « ~50 000 passages réels pour chacune des structures ». Les six index de l'échantillon sont `articles`, `decisions`, `jorf`, `titres`, `sarde` et `dole`, où `decisions` agrège deux corpus du jeu de requêtes, `juri_sommaires` et `juri_texte_integral` (note de `MES-04` §3, tâche `NETTOYER-P2`).
- **La stratégie se force par le seuil.** ADR-0074 fixe `exact_threshold` optionnelle, matérialisée à la valeur de `rescore_candidates` à la création, un domaine de `0` à `u32::MAX`, et un pool de taille **égale** au seuil traité en EXACT. Avec le seuil par défaut égal à N, tout pool d'au plus N passages passe donc en EXACT et la comparaison vaudrait 1,0 par construction ; `PATCH search.exact_threshold = 0` ne fait passer par EXACT qu'un pool vide.
- **Les requêtes sont encodées en fp32.** ADR-0052 fixe `query_precision = fp32`.
- **La production filtre presque toujours.** ADR-0004 constate que « les requêtes sont presque toujours filtrées ».
- **`synth-large` est réservé à la latence.** `docs/08:55` le décrit comme « 8,6 M passages de vecteurs aléatoires normalisés : pour la latence seulement ».

### Les questions tranchées ici

`@architecte` a rendu ces verdicts avant la mesure :

1. **Définition des métriques** : **NON-COUVERT**. `docs/06:104` nomme les deux métriques, `docs/08:85` ne fixe que la cible, et le seul précédent est MES-04 §3.
2. **« Par structure d'index »** : **CONFORME**. C'est le corpus juridique, un index par structure (ADR-0003, `docs/08:56`), avec les six index de l'échantillon et `decisions` agrégeant deux corpus.
3. **Centrage** : protocole **NON-COUVERT** (`docs/03:50`, `docs/08:77`). La mesure hors moteur est possible sans toucher au format, puisque `codes.bin` est dérivé ; un centrage par défaut demanderait un ADR qui remplace ADR-0069, décision 6.
4. **Taille des pools** : l'échantillon est **CONFORME** (`docs/08:56`), l'extrapolation à 8,58 M **NON-COUVERT**. `docs/08:55` réserve `synth-large` à la latence, et le seuil par défaut rendrait la comparaison triviale sur un pool d'au plus N passages.
5. **Requêtes et filtres** : fp32 **CONFORME** (ADR-0052) ; le champ `structure` de chaque requête donne l'index à interroger ; les filtres sont **NON-COUVERT**, car aucun champ de filtre n'existe dans le jeu de référence et qu'ADR-0004 écrit que les requêtes de production sont presque toujours filtrées.

Aucune de ces questions ne se tranche par la lecture seule : elles fixent deux définitions de métrique, un protocole expérimental, un mode de forçage de stratégie et le périmètre d'un jeu de requêtes. Ce sont des décisions.

### L'état des données au moment de la décision

- `data/legifacile-sample/queries.jsonl` contient **300** requêtes, chacune portant un champ `structure` : 63 `articles`, 200 `decisions`, 12 `jorf`, 9 `sarde`, 8 `titres`, 8 `dole`. Le texte de la requête est le champ `question`. Aucune entrée ne porte de champ `filter`.
- Les pools de l'échantillon, relevés par MES-04 §3, vont de **7 628** passages (`dole`) à **50 000** passages (`articles`, `jorf`, `sarde`, `titres`), pour **257 626** passages au total. Le plus grand pool de la mesure vaut donc 50 000 passages, contre 8,58 M en production.
- Les configurations de `data/legifacile-sample/configs/` portent déjà `search.exact_threshold` explicite (par exemple 1000 pour `articles`), `rescore_candidates` 1000 et `scan: "binary"`. Le champ `etat` existe dans le schéma de `articles` et le champ `partie` dans celui de `decisions` : ce sont les deux seuls champs dérivables pour une variante filtrée sans modifier les lots.

## Décision

**La mesure de fidélité a un protocole unique : deux métriques définies au passage et au document, une agrégation en moyenne globale pour la cible, un centrage mesuré hors moteur, BINARY forcé par le seuil, et un jeu de requêtes figé.**

Précisions :

1. **Métriques.** Le **recall@10** est la moyenne, sur les requêtes, de |top-10 BINARY ∩ top-10 EXACT| / 10, au niveau du **passage** (clé `doc_id#ord`), à requête identique : le pipeline normal contre le même pipeline en `exact: true` (`docs/06:104`). Le **recouvrement@10** est le même calcul après projection des deux top-10 sur `doc_id`, dédoublonnés, divisé par le nombre de `doc_id` distincts du top-10 EXACT : c'est l'accord documentaire, dans la forme du « Accord document@10 » de MES-04 §3. La cible de 0,99 (`docs/08:85`) porte sur le recall@10 au niveau du passage, au N par défaut, **en moyenne sur toutes les requêtes** ; les valeurs par structure sont rapportées, et toute structure sous 0,99 est signalée et expliquée — même règle que les cibles de rappel par corpus de `docs/suivi.md`, section « Cibles » : les petits corpus ne sont pas contraints un par un. Le recouvrement@10 est rapporté **sans cible**. P7 (`crible eval`) reprendra ces définitions telles quelles.
2. **Structures.** Le rapport donne un tableau **par structure** — les six index de l'échantillon, avec la taille du pool — plus une ligne de **moyenne globale** sur les 300 requêtes. `decisions` reste un seul index, ses deux corpus n'étant pas séparés, comme dans MES-04 §3.
3. **Centrage.** La variante **principale (a)** soustrait la moyenne des vecteurs de **tous les passages vivants de l'index** aux vecteurs des documents **et** à la requête, avant le signe, **sans renormalisation**. La variante **secondaire (c)**, documents centrés et requête non centrée, est rapportée à titre indicatif. La mesure est **hors moteur** : un harnais recalcule les codes en RAM depuis les vecteurs fp16. **Contrôle préalable obligatoire** : le harnais doit reproduire **au bit près** les codes lus depuis `codes.bin` (signe brut) et le top-10 BINARY du moteur ; sinon aucun chiffre n'est retenu. Aucun changement du moteur ni du format n'est fait : adopter le centrage par défaut demandera un ADR qui remplace ADR-0069, décision 6.
4. **Pools.** La mesure porte sur **l'échantillon seul**, avec en plus une variante filtrée (point 5) plus proche de la production ; le rapport recommande N **avec une marge**. `synth-large` n'est **pas** utilisé pour la fidélité. BINARY est forcé par `PATCH search.exact_threshold = 0` sur les **index de mesure**, copies dans `data/`, jamais sur les lots de l'échantillon eux-mêmes ; un chiffre n'est retenu que si le plan rend `dense_strategy = "binary"`. La limite — pools d'au plus 50 000 passages contre 8,58 M en production — est écrite dans la **conclusion du rapport**, et la confirmation à l'échelle est renvoyée à P9 (`legifacile-full`).
5. **Requêtes et filtres.** Les **300** requêtes de `queries.jsonl` sont utilisées, encodées en fp32 (ADR-0052), chacune sur l'index de sa structure. La campagne **principale** est **sans filtre** : pool maximal, donc pire cas de l'échantillon. Une **variante filtrée** est dérivée d'un champ présent dans l'échantillon — `etat` pour `articles`, `partie` pour `decisions` — et déclarée comme **requête synthétique** dans le protocole du rapport ; le jeu de référence n'est pas modifié.
6. **Levier.** Si la cible n'est pas atteinte au N recommandé, le rapport **propose** un levier (le scoring asymétrique de `docs/03-recherche.md`) à faire valider par le propriétaire, et ne le code pas.

Ce qui n'est **pas** décidé ici : la valeur de N et du centrage par défaut, qui sont les conclusions de la mesure elle-même (MES-01, Q-01) ; l'adoption du centrage dans le moteur, qui demande un ADR remplaçant ADR-0069 décision 6 ; la fidélité sur `legifacile-full`, renvoyée à P9 ; le codage du scoring asymétrique ; la valeur des cibles de rappel et de latence, qui restent celles de `docs/suivi.md`.

## Conséquences

- **Positives** :
  - les deux métriques ont une définition unique, avec un dénominateur explicite : le rapport de P4 et `crible eval` en P7 ne peuvent plus diverger ;
  - la moyenne globale pour la cible évite de contraindre des structures de 8 requêtes, où un seul passage manqué ferait tomber le recall@10 de 0,0125 ;
  - forcer BINARY par `search.exact_threshold = 0` rend la mesure réelle : sans cela, un pool d'au plus N passages passerait en EXACT et la comparaison vaudrait 1,0 par construction ;
  - le contrôle au bit près du harnais rend le chiffre de centrage vérifiable : sans lui, une erreur de signe ou d'ordre des dimensions produirait un résultat plausible et faux ;
  - le protocole ne touche ni au moteur ni au format : la mesure reste possible en P4, et l'adoption éventuelle du centrage garde son propre ADR ;
  - la variante filtrée rapproche la mesure de la production sans réécrire le jeu de référence.
- **Négatives / coûts** :
  - le rapport porte deux variantes de centrage et deux campagnes de filtre : plusieurs tableaux à lire, et un temps de mesure plus long ;
  - les 300 requêtes ont des tailles de pool très inégales (7 628 à 50 000 passages) : les valeurs par structure ne sont pas comparables entre elles sans lire la colonne du pool ;
  - la cible en moyenne globale masque une structure faible : d'où la règle de signalement, qui demande une explication rédigée par structure sous 0,99 ;
  - la mesure ne dit rien de la fidélité à 8,58 M : la conclusion du rapport doit le rappeler, et P9 devra rejouer la mesure ;
  - le harnais de centrage vit hors du dépôt de production : il doit être versionné avec le rapport pour que le chiffre soit reproductible.
- **À surveiller** :
  - si le recall@10 global tombe sous 0,99 au N recommandé, la fiche demande un levier : le scoring asymétrique devra être décrit puis validé par un ADR avant tout code ;
  - si le centrage (a) ne dépasse pas le signe brut de façon nette, il ne justifie pas un ADR remplaçant ADR-0069 décision 6 ;
  - la fidélité peut se dégrader quand le pool grandit : un recall@10 de 0,99 sur 50 000 passages ne vaut pas preuve pour 8,58 M, et P9 devra confirmer ;
  - si P7 définit le recall@k autrement (par exemple au niveau du document), les deux définitions devront être réconciliées par un nouvel ADR, pas par une retouche du rapport.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Cible de 0,99 imposée **par structure** | Les structures `dole`, `sarde` et `titres` ne portent que 8 ou 9 requêtes : un passage manqué fait perdre 0,0125 de recall@10, et la contrainte deviendrait un test de bruit statistique |
| Métrique au niveau du **document** seulement | `docs/08:85` parle du recall@10 de l'approximation, et le passage est l'unité encodée et rescorée ; l'accord documentaire est utile, mais il ne peut pas porter la cible |
| Définition propre à MES-01, laissée à P7 | Le rapport et `crible eval` mesureraient deux choses différentes sous le même nom ; le propriétaire ne pourrait pas comparer le chiffre de P4 à celui de P7 |
| Recouvrement@10 avec cible | Aucune référence ne fixe sa valeur ; il dépend du nombre de passages par document, donc du corpus, et non de la seule fidélité du scan |
| Mesurer le centrage **dans le moteur** | Demanderait de changer le calcul de `codes.bin`, donc le format et ADR-0069 décision 6, pour une variante non retenue : la mesure se fait hors moteur, sans risque pour les index |
| Centrer la requête et les documents sur des moyennes différentes (variantes (b) et (d)) | Ces variantes n'ont pas été retenues par l'orchestrateur ; la variante (c) suffit à montrer l'effet du centrage asymétrique, et multiplier les variantes rendrait le rapport illisible |
| Renormaliser après centrage | Change le produit scalaire, donc la comparabilité avec EXACT et avec les vecteurs fp16 stockés ; ce serait une autre décision, non mesurée ici |
| Mesurer sur `synth-large` | `docs/08:55` le réserve à la latence ; ses vecteurs aléatoires normalisés ne représentent pas les grappes réelles du corpus |
| Extrapoler les chiffres à 8,58 M | Aucune mesure de fidélité n'existe à cette taille ; P9 est la phase prévue, et une extrapolation serait une affirmation sans mesure |
| Mesurer sur les index de l'échantillon avec `PATCH` | Les lots de l'échantillon sont la référence partagée ; une copie dans `data/` garde les lots intacts et rend la manipulation réversible |
| Utiliser le seuil par défaut (= N) | Un pool d'au plus N passages passerait en EXACT : le recall@10 vaudrait 1,0 par construction, et la mesure ne dirait rien |
| Variante filtrée tirée d'un champ absent de l'échantillon | Aucun champ `filter` n'existe dans `queries.jsonl` ; inventer un filtre non déclaré rendrait la mesure non reproductible |
| Coder le scoring asymétrique dans la foulée | Le levier est une proposition à valider ; le coder avant décision ferait entrer dans le moteur une option non choisie |

## Références

- Fiche : `docs/phases/P4-recherche-rapide.md:84-92` (`MESURER-FIDELITE-BINAIRE`).
- Conception : `docs/06-observabilite.md:92`, `:104` ; `docs/08-invariants-et-tests.md:55-56`, `:66`, `:77`, `:85` ; `docs/03-recherche.md:43-47`, `:50`, `:62-63` ; `docs/00-glossaire.md` (seuil exact, rescoring, N, pool, codes binaires).
- Mesures : `docs/mesures/MES-04-parite-encodeur.md` §3 (Rappel@10 moyen, Accord document@10, note `NETTOYER-P2`) ; `docs/mesures/README.md` (modèle de rapport).
- Données : `data/legifacile-sample/queries.jsonl` (300 requêtes, champ `structure`) ; `data/legifacile-sample/configs/` (`articles.json`, `decisions.json`).
- ADR liés : ADR-0003 (un index par structure), ADR-0004 (codes binaires et scan du pool), ADR-0052 (requêtes fp32), ADR-0069 décision 6 (format de `codes.bin`, bit `x_i > 0`), ADR-0074 (planification de la stratégie dense, seuil exact et forçage), ADR-0075 (regroupement des résultats).
- Question ouverte liée : Q-01 (N par défaut et cible de fidélité du scan binaire, MES-01).
