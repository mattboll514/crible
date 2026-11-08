# Bilan de la v1 de Crible (J10)

| | |
|---|---|
| **Date** | 2026-11-08 |
| **Branche** | `tache/BILAN-V1` |
| **Commit de départ** | `8aa9097` (`merge: NETTOYER-P10`) |
| **Statut** | à valider par le propriétaire ; la v1 est taguée par lui (ADR-0129, décision 7 ; `docs/phases/P10-validation-a-l-echelle-reelle.md:205`) |
| **Périmètre** | tous les chiffres de mesure viennent de `docs/mesures/`, du suivi et des ADR ; aucun n'est recalculé ici, et les comptages (écarts, ADR, thèmes, écarts de rappel) sont explicités |

## 1. Résumé

La v1 de Crible est terminée sur le plan technique : les cinq premiers critères de J10 sont tenus,
et le sixième est ce bilan, soumis au propriétaire. Les deux limites connues sont acceptées par
ADR-0142. L'import à l'échelle réelle porte
**9 600 105 passages** et **6 660 239 documents** dans six index, avec des comptes égaux à pgvector
(`docs/mesures/import-legifacile-2026-11-06.md:35-36`). Le rappel du dense primaire vaut
**0,500000** au global, **0,322581** sur `juri_texte_integral` et **0,750000** sur
`textes_vigueur`, contre des cibles de 0,482, 0,290 et 0,750
(`docs/mesures/comparaison-pgvector-2026-11-06.md:543-545`). La fidélité binaire contre exact vaut
**0,995876** sur les 97 requêtes (`docs/mesures/comparaison-pgvector-2026-11-06.md:154`). La
latence contraignante, mesurée sur le Mac mini M1 en conteneur, donne **79,355 ms** au p95 (plus
grand p95 par index), avec une estimation prudente de **358,959 ms** et un recoupement API de
**418,425 ms** ; les trois lectures restent sous 698 ms
(`docs/mesures/latence-mac-mini-2026-11-07.md:129`, `:256`, `:258`).

Le projet a traversé **71 écarts à la conception**, dont **70** ont été tranchés par un ADR ou un
complément d'ADR et **1** par un erratum ; **27** l'ont été par anticipation avant confirmation du
propriétaire (suivi, section « Écarts à la conception », lignes 544-614). L'index des ADR compte
**142** décisions : **140** `Accepté`, **2** `Remplacé` (ADR-0049 par ADR-0050, ADR-0071 par
ADR-0081), aucune `Proposé` ni `Rejeté` (`docs/adr/README.md`, index des décisions ; les deux
`Remplacé` sont aux lignes 105 et 127).

Deux limites restent assumées et écrites : la reconstruction lexicale prend **71, 127 et 185 s**
sur `jorf`, `articles` et `decisions` contre un budget de 60 s, et un `replace_all` complet de
`decisions` demande **10,99 GiB** d'anonyme pour une marge de **0,58 GiB** sur la VM de 11,67 GiB
(ADR-0142:52, `:42` ; `docs/mesures/import-legifacile-2026-11-06.md:40`, `:43`). La v2 est donc
priorisée autour de la mémoire et du redémarrage, avant les gains de rappel ou de latence.

## 2. Mesures comparées aux cibles

### 2.1 Les critères de J10

| Critère (ADR-0129, décision 7) | Exigé | Mesuré | Verdict | Source |
|---|---|---|---|---|
| Six configs d'index définies et validées | oui | `DEFINIR-INDEX-LEGIFACILE` `FAIT` | tenu | suivi, « Tâches », P10, `DEFINIR-INDEX-LEGIFACILE` (ligne 149) |
| Import à l'échelle réelle | 8,58 M passages, comptes égaux à pgvector, MES-07 et MES-09 faites | 9 600 105 passages, 6 660 239 documents, comptes égaux ; MES-07 et MES-09 faites | tenu, avec la réserve de `docs/08:57` | `docs/mesures/import-legifacile-2026-11-06.md:35-36`, `:41-42` |
| Jeu de référence validé et identique | oui | 97 lignes, 97 `expected`, 35 filtres | tenu | `docs/mesures/comparaison-pgvector-2026-11-06.md:23-25` |
| Rappel global ≥ 0,482 | 0,482 sur 56 questions | 0,500000 | tenu (+0,018) | `docs/mesures/comparaison-pgvector-2026-11-06.md:543`, `:553-554` |
| Rappel `juri_texte_integral` ≥ 0,290 | 0,290 sur 31 questions | 0,322581 | tenu (+0,032581) | `docs/mesures/comparaison-pgvector-2026-11-06.md:544` |
| Rappel `textes_vigueur` ≥ 0,750 | 0,750 sur 12 questions | 0,750000 | tenu, sans marge | `docs/mesures/comparaison-pgvector-2026-11-06.md:545`, `:554-555` |
| Fidélité binaire contre exact ≥ 0,99 | 0,99 au N = 5000 | 0,995876 sur 97 requêtes | tenu | `docs/mesures/comparaison-pgvector-2026-11-06.md:154` |
| Latence p95 ≤ 698 ms sur le Mac mini en conteneur | 698 ms | 79,355 ms (critère) ; 358,959 ms (prudent) ; 418,425 ms (API) | tenu | `docs/mesures/latence-mac-mini-2026-11-07.md:129`, `:256`, `:258` |
| Bilan écrit et validé par le propriétaire | écrit et validé | écrit ; validation en attente | en attente | ADR-0129:160 ; `docs/phases/P10-validation-a-l-echelle-reelle.md:205` |

Le tableau détaille le critère 4 en rappel et fidélité, et ajoute le critère 6, qui est la
validation de ce document.

Le critère de latence se lit avec ADR-0141, décision 4 : le `total` chaud de `crible eval` sur les
97 requêtes en texte, plus grand p95 par index, vaut 79,355 ms
(`docs/mesures/latence-mac-mini-2026-11-07.md:135`). Les deux autres lectures ajoutent l'encodage :
358,959 ms est une estimation prudente (79,355 + 279,604, p95 fp32 sans cache mesuré sur 300
textes), et 418,425 ms est le recoupement par l'API à cache vide
(`docs/mesures/latence-mac-mini-2026-11-07.md:256-260`).

### 2.2 Les mesures MES-00 à MES-16

| Mesure | Résultat retenu | Machine | Cible ou budget | Tenue ? | Source |
|---|---|---|---|---|---|
| MES-00 | pgvector : p50 279 ms, p95 698 ms, p99 1 112 ms ; rappel@10 0,482 | Mac mini M1 16 Go, production pgvector en lecture seule | référence, pas de cible | — | `docs/mesures/MES-00-reference-pgvector.md:11`, `:177`, `:77` |
| MES-01 | fidélité 0,9977 sans filtre, 0,9989 en filtrée | MacBook Air M3, natif | ≥ 0,99 (`docs/08-invariants-et-tests.md:86` ; suivi, « Cibles », 2026-10-20) | oui | `docs/mesures/MES-01-fidelite-scan-binaire.md:26`, `:145`, `:178` |
| MES-02 | à 3,59 M passages, scan binaire 47,535 ms p95 à N = 5000 | Mac mini M1, conteneur | budget indicatif 20 à 80 ms pour l'index entier (`docs/03-recherche.md:154`) | — (indicatif) | `docs/mesures/MES-02-latence-scan-binaire.md:401`, `:447` |
| MES-03 | rescoring chaud à N = 5000 : 4,274 ms p95 | Mac mini M1, conteneur | pas de cible ; protocole fixé par ADR-0077 | — | `docs/mesures/MES-03-latence-rescoring.md:2070`, `:2112` |
| MES-04 | fp32 ONNX contre PyTorch : cosinus minimal 0,999999999989 ; INT8 : rappel 0,7920 | MacBook Air M3, natif | parité ≥ 0,9999 (`docs/mesures/MES-04-parite-encodeur.md:112`) ; recette ≥ 0,999, minimale 0,9999604 (`docs/mesures/MES-04-parite-encodeur.md:202-207` ; ADR-0052:6) | oui | `docs/mesures/MES-04-parite-encodeur.md:17`, `:109`, `:130` |
| MES-05 | encodage fp32 279,604 ms p95 ; INT8 84,147 ms p95 | Mac mini M1, conteneur | budget indicatif 20 à 50 ms, INT8 (`docs/03-recherche.md:151`) | non (indicatif) ; ne vise pas le fp32 par défaut | `docs/mesures/MES-05-latence-encodage.md:329`, `:358-359` |
| MES-06 | BM25 filtrée sous 1,2 ms p95 ; reconstruction extrapolée 325 s en P5, ramenée à 71–185 s à l'import | MacBook Air M3, natif | pas de cible (`docs/03-recherche.md:159`, « à mesurer ») | — | `docs/mesures/MES-06-lexical-bm25-filtre.md:23`, `:397`, `:602` ; `docs/mesures/import-legifacile-2026-11-06.md:43` |
| MES-07 | compaction de `decisions` 371,0 s ; `jorf` 150,5 s puis 142,1 s | MacBook Air M3, conteneur 11,67 Gio | pas de cible ; ADR-0006:37 surveille la durée | — | `docs/mesures/MES-07-duree-compaction.md:264-266`, `:302-304` |
| MES-08 | 1 000 passages attachés en 3,26 s p50 à 8,6 M, dont 96,6 % dans `snapshot` ; avec le vrai modèle, la recette prend 51,5 s | MacBook Air M3, natif | pas de cible (hors v1, `docs/08-invariants-et-tests.md:73`) | — | `docs/mesures/MES-08-duree-attache-quotidienne.md:11`, `:252-253`, `:262` |
| MES-09 | 3,54 GiB d'anonyme en régime ; minimum 4 Gio, confortable 12 Gio ; `memory.current` du Mac mini de 9,048 à 9,196 GiB | MacBook Air M3 en conteneur ; Mac mini M1 en conteneur | minimum de service 4 Gio, confort 12 Gio (`docs/mesures/MES-09-ram-reelle.md:527-531`) | dimensionnement, pas un critère | `docs/mesures/MES-09-ram-reelle.md:527-531`, `:603-604` |
| MES-10 | `rrf` reste la fusion par défaut ; α = 0,3 ne se distingue pas | MacBook Air M3, natif | pas de cible : choix de la fusion | — | `docs/mesures/MES-10-qualite-fusion.md:7`, `:157` |
| MES-11 | aucune variante (V1, V2, V3) ne se distingue de V0 | MacBook Air M3, natif | pas de cible : choix de l'analyse | — | `docs/mesures/MES-11-analyse-francaise.md:7`, `:113` |
| MES-12 | à N = 5000, le centrage gagne +0,0010 et perd 0,0004 ; signe brut gardé | MacBook Air M3, natif | pas de cible : choix du signe | — | `docs/mesures/MES-12-centrage-codes-binaires.md:19`, `:167-169` |
| MES-13 | démarrage reconstruit : 16,6 à 32,1 s en extrapolation | MacBook Air M3, natif | budget ≤ 60 s, déclencheur du cache (ADR-0047:25, `:66`) | oui (extrapolé) | `docs/mesures/MES-13-temps-de-demarrage.md:26`, `:530-531` |
| MES-14 | filtres d'intervalle : 101 à 194 ms à 8,58 M sur fixtures ; 41,5 à 68,2 ms sur l'index réel | MacBook Air M3, natif puis conteneur pour le rejeu réel | pas de cible : valeur mesurée (ADR-0046:35) | — | `docs/mesures/MES-14-cout-filtres-intervalles.md:28`, `:493`, `:495` ; `docs/mesures/comparaison-pgvector-2026-11-06.md:275-276` |
| MES-15 | commit de génération à N = 8,58 M passages : 89,8 ms p50 (S = 10) et 111,1 ms (S = 1 000) | MacBook Air M3, natif | pas de cible | — | `docs/mesures/MES-15-cout-commit-generation.md:11`, `:793-794`, `:852` |
| MES-16 | pire requête sous charge : 344,0 ms contre une limite de 465,34 ms | MacBook Air M3, natif | ≤ 3 × p99 du repos, soit 465,34 ms (ADR-0113:37) | oui | `docs/mesures/MES-16-charge-concurrente-analytics-et-journal.md:20`, `:298` |

Quatre lignes portent une cible ou un budget chiffré : MES-01, MES-04, MES-13 et MES-16. `docs/08`
ne fixe que le rappel, la fidélité et la latence (`docs/08-invariants-et-tests.md:83-88`) ; MES-09
porte un plancher de dimensionnement, les budgets de MES-02 et de MES-05 sont indicatifs, et les
autres mesures éclairent un choix ou un protocole, ce que la colonne « Cible ou budget » dit.

MES-08 n'est plus une cible de la v1 : `docs/08-invariants-et-tests.md:73` place l'attache
quotidienne dans le projet Legifacile. Le chiffre est conservé comme référence de plomberie, pas
comme critère de J10.

Une précaution de lecture pour MES-15 et les durées d'attache : depuis `NETTOYER-P10`, le pas
`Manifest` compte `segments_of`, qui était compté dans `snapshot` ; une durée par étape d'une
attache ne se compare donc plus telle quelle à celles de MES-15 et des campagnes de P10 (suivi,
« Idées hors périmètre », 2026-11-08, `NETTOYER-P10`, ligne 809).

### 2.3 Les limites acceptées par ADR-0142

ADR-0142 accepte deux dépassements mesurés et en tire une règle d'exploitation
(`docs/adr/0142-limites-acceptees-de-la-v1-reconstruction-lexicale-et-memoire-d-un-replace-all.md:48-62`).

| Limite | Mesure | Décision |
|---|---|---|
| Reconstruction lexicale | 71 s (`jorf`), 127 s (`articles`), 185 s (`decisions`) contre 60 s, pour 1 022 144 487 jetons | aucune correction en v1 ; le budget de 60 s n'est pas relevé ; la v2 passe à plus d'un thread d'écriture (ADR-0142:52 ; `docs/mesures/import-legifacile-2026-11-06.md:43`) |
| `replace_all` complet de `decisions` | 10,99 GiB d'anonyme, 0,58 GiB de marge sur une VM de 11,67 GiB | pas de correction en v1 ; un grand `replace_all` se fait sur la machine de développement, puis l'index rejoint le Mac mini par instantané et restauration (ADR-0142:56 ; `docs/mesures/import-legifacile-2026-11-06.md:40`) |
| Mémoire après attache | 10,55 Gio d'anonyme après l'attache, 3,57 Gio après redémarrage | en v2, retirer les textes de `CommitDelta.created`, gain attendu d'environ 5 Go, **non mesuré** (ADR-0142:58, `:79`) |

La règle d'exploitation est écrite dans `docs/07` : les lots quotidiens, petits, s'attachent
directement sur le Mac mini ; les grands index passent par un instantané
(ADR-0142:56). Le seuil chiffré entre « grand » et « petit » index n'est pas fixé : ADR-0142 nomme
les trois index mesurés et renvoie un index intermédiaire à une mesure (ADR-0142:60).

## 3. Écarts à la conception

Les **71** écarts de la section « Écarts à la conception » du suivi (lignes 544-614) se regroupent
en neuf thèmes. Les comptes ci-dessous portent sur les lignes du tableau, une ligne par écart.

| Thème | Nombre | Exemples représentatifs | Traitement |
|---|---:|---|---|
| Configuration, schéma et empreinte du modèle | 6 | `serde_json` requis, forme canonique de l'empreinte, champs déclarés aux deux niveaux | ADR-0027 à ADR-0031 ; ADR-0137 |
| Lots, stockage et cycle de vie | 16 | format des lots, identité d'un lot en ligne, patch, export, instantané, compaction | ADR-0034 à ADR-0066 ; 1 erratum, celui de `docs/04:229` (suivi, « Écarts à la conception », 2026-10-28, `INDEXER-TANTIVY-A-L-ATTACHE`, ligne 591) |
| Encodage et recette | 4 | empreinte des requêtes, longueurs, `acceptance_sample = 0`, recette sans lots | ADR-0053, ADR-0054, ADR-0067, ADR-0073 |
| Recherche dense, binaire, rescoring et fusion | 6 | seuil exact, stratégie, regroupement, calcul partagé EXACT/rescoring | ADR-0069, ADR-0072, ADR-0074, ADR-0075, ADR-0090 |
| Lexical et tantivy | 7 | langue, reprise à l'ouverture, état « à jour », branche désactivée, fusion constante | ADR-0086 à ADR-0090 |
| API et observabilité | 9 | télémétrie ONNX, analytics, journal de debug, `explain`, `stats`, `eval` | ADR-0117 à ADR-0128 ; complément ADR-0113 |
| Performance, mémoire et exploitation | 3 | coût du commit, cache des lecteurs denses | ADR-0066, ADR-0126 |
| Protocoles de mesure et CI | 16 | N, froid, pools, RSS, protocoles MES-02/03/06/09/10/11, comparaison pgvector, CI distante | ADR-0077, ADR-0091, ADR-0092, ADR-0140, ADR-0141 |
| Données Legifacile et comparaison | 4 | échantillon, jeu de référence versionné, import à l'échelle, mémoire du `replace_all` | ADR-0138, ADR-0139, ADR-0142 |

Sur ces 71 écarts, **70** sont tranchés par un ADR ou un complément d'ADR et **1** par un erratum
(celui de `docs/04:229` ; suivi, « Écarts à la conception », 2026-10-28,
`INDEXER-TANTIVY-A-L-ATTACHE`, ligne 591). **27** lignes portent une acceptation par anticipation,
dont certaines ont été confirmées ensuite par le propriétaire (suivi, section « Écarts à la
conception », lignes 544-614). Le volume ne signale pas un échec de la conception : il montre que la
conception laissait ouverts des choix d'implémentation que les mesures et les relectures ont dû
trancher un par un.

## 4. Leçons

1. **Le scan binaire et le rescoring ne sont pas le coût dominant.** À 3,59 M passages, le scan
   vaut 47,535 ms p95 et le rescoring chaud à N = 5000 vaut 4,274 ms p95
   (`docs/mesures/MES-02-latence-scan-binaire.md:447`,
   `docs/mesures/MES-03-latence-rescoring.md:2112`). Sur `articles`, le filtre pèse 58,610 ms des
   79,355 ms du p95 (`docs/mesures/latence-mac-mini-2026-11-07.md:129`). Le prochain levier de
   latence est donc le filtre d'intervalle, déjà mesuré à 101–194 ms sur fixtures et 41,5–68,2 ms
   sur l'index réel (`docs/mesures/comparaison-pgvector-2026-11-06.md:275-276`).

2. **L'encodage domine quand le cache est froid, et disparaît quand il est chaud.** Le p95 de
   `encode` vaut 0,005 à 0,024 ms dans la série contraignante, contre 279,604 ms pour le fp32 sans
   cache de MES-05 (`docs/mesures/latence-mac-mini-2026-11-07.md:147-150`,
   `docs/mesures/MES-05-latence-encodage.md:359`). La comparaison à 698 ms est favorable par
   construction sur ce poste, et c'est pourquoi le bilan donne aussi 358,959 ms et 418,425 ms
   (`docs/mesures/latence-mac-mini-2026-11-07.md:256-260`).

3. **La mémoire, et non le CPU, borne l'exploitation.** L'attache de `decisions` laisse 10,99 GiB
   d'anonyme et 0,58 GiB de marge, et la campagne laisse 2,275 GiB de swap
   (`docs/mesures/import-legifacile-2026-11-06.md:40` ; ADR-0142:42). Ouvrir les six index
   ensemble coûte 552,7 MiB de plus que la somme des ouvertures séparées, sans explication
   (`docs/mesures/MES-09-ram-reelle.md:509`). Un dimensionnement de service se fait donc sur la
   mémoire, pas sur la latence.

4. **Un coût quadratique invisible sur l'échantillon a dominé avant correction, et pèse encore.**
   `refresh_updated_documents` faisait une requête `passages(doc_id)` par document réécrit, soit
   1,6 M requêtes à la génération 1 de `jorf`, avec une prédiction d'environ 2 h 16 après le commit
   sur cet index (suivi, « Tâches », P10, `CORRIGER-BLOCAGE-APRES-COMMIT`, ligne 151). Après
   correction, `jorf` rend la main ≤ 92 s après le commit, contre un processus encore vivant à 600 s
   (même ligne). La lecture qui reste — « probablement l'essentiel des ≤ 92 s restantes » — n'est
   pas mesurée (suivi, « Idées hors périmètre », 2026-11-06, `CORRIGER-BLOCAGE-APRES-COMMIT`,
   ligne 783). Mesurer après le commit, et pas seulement le commit, était nécessaire.

5. **La spécification a été le premier poste de travail.** Le constat est qualitatif, mais il se lit
   dans les chiffres du processus : 71 écarts, 142 ADR et 27 acceptations par anticipation (suivi,
   section « Écarts à la conception », lignes 544-614 ; `docs/adr/README.md`). Il a fallu réécrire
   des protocoles avant campagne (ADR-0077, ADR-0091, ADR-0140) et déclarer après coup qu'une série
   de MES-03 n'était pas rejouable sur le moteur (ADR-0084:22). Écrire le protocole, la machine et
   le régime mémoire avant la mesure reste le geste qui économise le plus de temps.

## 5. Liste priorisée pour la v2

L'ordre suit trois blocs, dans cet ordre : **risque pour l'exploitation de Legifacile** (points 1 à
6), **gains annoncés, non mesurés** (points 7 à 10), puis **coût** (point 11). Aucun point n'a de
gain mesuré : un gain annoncé est signalé comme tel et ne justifie pas, à lui seul, une tâche. Le
point 12 est une décision, pas un travail.

| # | Élément | Pourquoi | Gain ou risque | Source |
|---:|---|---|---|---|
| 1 | Allocateur qui ne rend pas la mémoire au système | 10,55 Gio d'anonyme restent après l'attache, contre 3,57 Gio après redémarrage | risque d'exploitation direct | suivi, « Idées hors périmètre », 2026-11-06, `IMPORTER-BASE-COMPLETE` (ligne 789) |
| 2 | Retirer les textes de `CommitDelta.created` | environ 5 Go attendus sur `decisions` | risque d'exploitation ; gain non mesuré | ADR-0142:58 |
| 3 | Reconstruction lexicale à plus d'un thread d'écriture | 71, 127 et 185 s contre 60 s, payés à chaque redémarrage ou compaction d'un grand index | risque de reprise ; gain non mesuré | ADR-0142:52 ; `docs/mesures/import-legifacile-2026-11-06.md:43` |
| 4 | `crible inspect` bloqué sur un volume ouvert en écriture ; fusion tantivy annulée après un changement d'époque | diagnostic et intégrité de l'index | risque d'exploitation | suivi, « Contenu de NETTOYER-P10 », passage C (lignes 526-528) |
| 5 | `refresh_updated_documents` : exclure du delta les documents entièrement créés | 1,6 M requêtes, ≤ 92 s après le commit sur `jorf`, ≤ 227 s sur `decisions` | risque de durée d'attache | suivi, « Idées hors périmètre », 2026-11-06, `CORRIGER-BLOCAGE-APRES-COMMIT` (ligne 783) |
| 6 | `KeywordField::clear` reste O(V) par passage | coût quadratique si un patch nettoie en masse un champ de forte cardinalité | risque futur, pas observé en v1 | suivi, « Idées hors périmètre », 2026-11-06, `CORRIGER-BLOCAGE-APRES-COMMIT` (ligne 784) |
| 7 | Observabilité d'exploitation : durées par étape dans la CLI, nombre de jetons analysés, écart de 552,7 MiB de MES-09 | trois angles morts de mesure et de capacité | gain de diagnostic | suivi, « Idées hors périmètre », 2026-11-06 : `CORRIGER-MEMOIRE-APRES-COMMIT` (ligne 787), `IMPORTER-BASE-COMPLETE` (lignes 794 et 796) |
| 8 | Rerank (late interaction) | étape distante après la fusion, avec repli `reranked: false` | gain non mesuré ; à chiffrer | `docs/03-recherche.md:138-142` ; `docs/01-vision-et-perimetre.md:50` |
| 9 | IVF par segment | prévu au-delà de ~30 M passages ; la v1 tient à 9,6 M | gain non mesuré à la taille actuelle | `docs/02-architecture.md:249` ; `docs/01-vision-et-perimetre.md:49` |
| 10 | Fusions progressives de segments | si la compaction complète devient trop longue ; 371,0 s sur `decisions` | gain non mesuré, dépend de la croissance | `docs/02-architecture.md:250` ; ADR-0006:37, `:44` |
| 11 | Découpeur par défaut dans le moteur | un projet sans pipeline doit découper lui-même | coût de conception ; hors v1 par ADR-0010 | ADR-0010:25 ; `docs/01-vision-et-perimetre.md:51` |
| 12 | Reprise de P9 et tableau de bord, dont Next.js | P9 est reportée après J10, hors v1 ; Next.js ajoute Node, un build et probablement CORS | **décision du propriétaire**, pas une tâche : nouvel ADR obligatoire avant toute tâche | ADR-0136:59-71, `:93-100`, `:172` ; voir 7.2 |

Les points 8 à 11 sont les « rerank, IVF, fusions progressives, découpeur par défaut » demandés par
la fiche de `BILAN-V1` (`docs/phases/P10-validation-a-l-echelle-reelle.md:204`). Les points 2 et 3
sont les deux points de v2 d'ADR-0142 : le retrait des textes de `CommitDelta.created` (décision 4)
et la reconstruction lexicale à plus d'un thread (décision 1). L'allocateur du point 1 vient du
suivi, pas d'ADR-0142. Les points 1 et 4 à 7 viennent du suivi et de `NETTOYER-P10`. Le point 12 est
une décision du propriétaire (7.2), pas une tâche de v2.

## 6. Ce qui revient au projet Legifacile

ADR-0129, décision 7, sort de la v1 la **tâche quotidienne stable 14 jours** et le **retour arrière
testé** (interrupteur `backend` du MCP) ; ils ne conditionnent plus J10 et appartiennent au projet
Legifacile, qui a son propre bilan (`docs/adr/0129-replanification-de-la-fin-du-plan.md:162`).
Le document de passation porte les fiches mot pour mot, et Crible reste derrière le backend de
Legifacile, avec ses clés d'API (ADR-0129:164-172).

Trois conséquences pour la v2 :

- **MES-08** (attache quotidienne) n'est plus une cible de Crible : `docs/08-invariants-et-tests.md:73`
  la place dans le projet Legifacile. Le chiffre de 3,26 s pour 1 000 passages reste une référence
  de plomberie (`docs/mesures/MES-08-duree-attache-quotidienne.md:250`).
- **Le script d'export** (`legifacile-sandbox/scripts/export_crible_full.py`) et la règle « grand
  `replace_all` sur la machine de développement » sont des outils de migration, utiles au
  branchement de Legifacile, pas des fonctions du moteur
  (suivi, « Idées hors périmètre », 2026-11-06, `EXPORTER-BASE-LEGIFACILE`, ligne 782 ; ADR-0142:60).
  Le script n'est donc pas un point de la v2.
- **Les volumes `crible-p10-*` et l'installation du Mac mini** sont gardés jusqu'au branchement de
  Legifacile, sur décision du propriétaire (suivi, « Idées hors périmètre », 2026-11-08,
  `MESURER-SUR-MAC-MINI`, ligne 807).

## 7. Décisions qui restent au propriétaire

1. **Valider ce bilan et taguer la v1** : c'est le critère d'acceptation de `BILAN-V1`, et le tag
   est posé par le propriétaire seul (`docs/phases/P10-validation-a-l-echelle-reelle.md:205` ;
   ADR-0129, décision 7).
2. **Décider la reprise de P9** et le contenu du tableau de bord : ADR-0136 suspend l'application
   d'ADR-0130 et exige un nouvel ADR, écrit à la reprise et avant toute tâche de P9, pour trancher
   le framework, dont Next.js (ADR-0136:108-113, `:172`).
3. **Corriger `docs/08:57`** (8,58 M contre 9 600 105) : la ligne est signalée dans le suivi, mais
   aucune décision ne l'a encore visée (suivi, « Idées hors périmètre », 2026-11-08, `NETTOYER-P10`,
   ligne 808).
4. **Arbitrer la priorité de la v2** : la liste ci-dessus est ordonnée par risque, puis gains
   annoncés, puis coût ; aucun point n'a de gain mesuré. Ouvrir les tâches reste une décision du
   propriétaire.
5. **Décider si la marge de 0,58 GiB impose un plafond de mémoire** sur le Mac mini : ADR-0142
   laisse le point « à surveiller » et ne fixe aucun plafond (ADR-0142:75-76).

Le sort de l'installation du Mac mini n'est plus ouvert : il a été tranché par le propriétaire le
2026-11-08. L'image et les volumes `crible-p10-*` sont gardés jusqu'au branchement de Legifacile ;
les instantanés du dossier d'échange du SSD ont été supprimés par `@mesureur` ce jour-là (23,9 Gio) ;
`docker-backup` est gardé une semaine, puis retiré par le propriétaire ; le branchement de
Legifacile suit le tag de la v1 et reste un geste du propriétaire (suivi, « Idées hors périmètre »,
2026-11-08, `MESURER-SUR-MAC-MINI`, ligne 807).

## 8. Contrôle des citations

Dix citations ont été tirées au hasard et relues dans les fichiers, après rédaction :

| Citation | Contenu retrouvé |
|---|---|
| `docs/mesures/latence-mac-mini-2026-11-07.md:129` | `articles` : p50 21,862 ms, p95 **79,355** ms, p99 82,060 ms |
| `docs/mesures/latence-mac-mini-2026-11-07.md:256` | **358,959 ms**, estimation prudente (79,355 + 279,604) |
| `docs/mesures/comparaison-pgvector-2026-11-06.md:543` | global satisfaisable **0,500000**, cible **0,482** |
| `docs/mesures/comparaison-pgvector-2026-11-06.md:154` | global 97 requêtes, fidélité **0,995876** |
| `docs/mesures/import-legifacile-2026-11-06.md:40` | `decisions` : **10,99 GiB** d'`anon`, **marge 0,58 GiB** |
| `docs/mesures/MES-09-ram-reelle.md:527` | plancher **3,54 GiB**, minimum **4 Gio** |
| `docs/adr/0142-limites-acceptees-de-la-v1-reconstruction-lexicale-et-memoire-d-un-replace-all.md:52` | reconstruction lexicale à **plus d'un thread** en v2, budget de 60 s non relevé |
| suivi, section « Écarts à la conception », lignes 544-614 | **71** lignes d'écart (comptage des lignes `\| 2026`) |
| suivi, « Idées hors périmètre », 2026-11-08, `NETTOYER-P10` (ligne 808) | `docs/08:57` donne encore **8,58 M** contre **9 600 105** |
| `docs/08-invariants-et-tests.md:57` | `legifacile-full` : « Toute la base (**8,58 M**) » |

Les dix concordent. `git status --short` ne montre que `docs/suivi.md` modifié par l'orchestrateur
et ce fichier ajouté ; aucun autre fichier n'a été touché.

Trois incohérences restent à signaler, sans correction dans cette tâche :

- **`docs/08:57`** garde 8,58 M passages là où l'import en compte 9 600 105
  (`docs/08-invariants-et-tests.md:57` ; suivi, « Idées hors périmètre », 2026-11-08, `NETTOYER-P10`,
  ligne 808) ;
- **le rapport MES-16 de P6** nomme la machine « Mac mini `Mac15,12` (Apple M4) »
  (`docs/mesures/MES-16-charge-concurrente.md:12`), alors que `Mac15,12` est le MacBook Air M3 dans
  les autres rapports (`docs/mesures/MES-01-fidelite-scan-binaire.md:26`) et que la campagne de
  référence de MES-16 pour P7 tourne sur le MacBook Air M3
  (`docs/mesures/MES-16-charge-concurrente-analytics-et-journal.md:20`). Le libellé de P6 est à
  corriger ou à expliquer par le propriétaire ;
- **les renvois d'ADR-0142 au suivi ont dérivé** : la décision sur la reconstruction lexicale y est
  citée à la ligne qui porte aujourd'hui la mémoire après attache — suivi, « Idées hors périmètre »,
  2026-11-06, `IMPORTER-BASE-COMPLETE` (passe A), ligne 789 — alors qu'elle est à suivi, « Idées
  hors périmètre », 2026-11-06, `IMPORTER-BASE-COMPLETE` (passe B), ligne 791 ; et la mémoire après
  attache y est citée à suivi, « Idées hors périmètre », 2026-11-06,
  `CORRIGER-MEMOIRE-APRES-COMMIT`, ligne 787, qui n'est pas la bonne
  (`docs/adr/0142-limites-acceptees-de-la-v1-reconstruction-lexicale-et-memoire-d-un-replace-all.md:11`,
  `:79`, `:98`, `:103`). Les citations de l'ADR sont à réaligner.
