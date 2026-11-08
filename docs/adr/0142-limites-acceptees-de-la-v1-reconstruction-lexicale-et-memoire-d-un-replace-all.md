# ADR-0142 — Limites acceptées de la v1 : reconstruction lexicale et mémoire d'un `replace_all` complet

- **Statut** : Accepté (décidé par le propriétaire le 2026-11-08, « ADR-0142 et complément d'ADR-0113 acceptés, ajoute la règle dans docs/07 »)
- **Date** : 2026-11-08
- **Décideurs** : propriétaire
- **Proposé par** : agent `@greffier`, à la demande de l'orchestrateur, hors pipeline de tâche, pour préparer la tâche `NETTOYER-P10` (phase P10). Le texte reprend la décision du propriétaire du 2026-11-08 — « ok je valide tes recos, tu peux mettre C de coté pour le moment on le garde en note pour plus tard » — et les faits mesurés de la campagne `IMPORTER-BASE-COMPLETE`.
- **Remplace** : — (le présent ADR ne remplace pas ADR-0139, dont la campagne est close)

## Contexte

La campagne `IMPORTER-BASE-COMPLETE` (P10) a importé les six index de Legifacile dans une instance Crible : 6 660 239 documents et 9 600 105 passages, égaux à pgvector (`docs/mesures/import-legifacile-2026-11-06.md:35-36`). Elle a laissé deux constats mesurés que rien de la conception ne tranche : le temps de reconstruction lexicale dépasse la cible de 60 s sur trois index, et l'attache complète d'un grand index laisse très peu de marge mémoire. Le propriétaire a décidé, le 2026-11-08, de les accepter comme limites de la v1. Les deux lignes du suivi qui portent cette décision sont `docs/suivi.md:789` (reconstruction lexicale) et `docs/suivi.md:790` (marge mémoire).

### Fait 1 — la reconstruction lexicale dépasse 60 s sur trois index

La cible de 60 s vient d'ADR-0047 : c'est un **budget de démarrage** écrit à l'échelle de Legifacile, et non une cible de reconstruction lexicale (`docs/adr/0047-phase-de-mes-13-et-ordre-des-remedes-au-temps-de-demarrage.md:25`). ADR-0047 le rappelle une seconde fois en classant les remèdes (`:44`), et sa conséquence « À surveiller » en fait un budget de service : « tout index dont le redémarrage dépasserait 60 s » (`:66`). ADR-0139 précise que ce budget se lit **par index**, jamais pour l'instance entière (`docs/adr/0139-protocole-de-la-campagne-d-import-a-l-echelle-reelle.md:27`).

La passe B a mesuré `crible rebuild-lexical` sur les six index, dans un conteneur d'une VM de 11,67 Gio (`docs/mesures/import-legifacile-2026-11-06.md:357-365`) :

| Index | Passages | Jetons analysés | Durée | Cible de 60 s |
|---|---:|---:|---:|---|
| `dole` | 7 653 | 1 788 111 | 1 s | tenue |
| `sarde` | 214 578 | 14 123 942 | 5 s | tenue |
| `titres` | 1 477 883 | 23 186 520 | 13 s | tenue |
| `jorf` | 1 833 500 | 177 669 539 | 71 s | dépassée (×1,18) |
| `articles` | 2 476 715 | 310 806 138 | 127 s | dépassée (×2,12) |
| `decisions` | 3 589 776 | 494 570 237 | 185 s | dépassée (×3,08) |

La somme, 402 s pour 1,02 milliard de jetons, n'est pas comparable à 60 s, puisque le budget est par index (`docs/mesures/import-legifacile-2026-11-06.md:372-373`). Le rapport conclut que trois index dépassent la cible, sans rien coder, et rappelle que `WRITER_THREADS` reste à 1 (`:374-377`).

Cette reconstruction ne se produit pas à chaque requête ni à chaque attache : elle survient à la **reprise** lexicale, à l'ouverture en écriture (ADR-0087, décision 1, `docs/adr/0087-reprise-lexicale-et-ouverture-en-lecture-seule.md:34`), pendant la **compaction**, et sur la commande explicite `crible rebuild-lexical`. C'est donc un coût de démarrage ou de maintenance, jamais un coût de service.

Le levier de correction est connu et déjà écrit : paralléliser l'écrivain lexical. ADR-0087 a noté le budget de l'`IndexWriter` de tantivy — 50 MiB et **un seul thread**, valeur qui est une constante du code — et l'a classé « NON-COUVERT mais gardé » (`docs/adr/0087-reprise-lexicale-et-ouverture-en-lecture-seule.md:28`). Sa conséquence « À surveiller » demande à MES-06 de mesurer ce budget, et n'ouvre un ADR que si la mesure le montre structurant (`:75`). La mesure de MES-06 ne l'a pas montré structurant (`docs/suivi.md:103`).

### Fait 2 — l'attache complète d'un grand index laisse 0,58 Gio de marge

L'attache de `decisions`, avec les cinq autres index ouverts, culmine à **10,99 Gio de mémoire anonyme** et 11,09 GiB de `memory.current`, soit **0,58 Gio de marge** sous les 11,67 Gio de la VM (`docs/mesures/import-legifacile-2026-11-06.md:40` et `:469-471`). Le propriétaire avait réglé cette VM à **12 Gio**, et ADR-0139, décision 3, la retient comme RAM de référence de la campagne (`docs/adr/0139-protocole-de-la-campagne-d-import-a-l-echelle-reelle.md:65` et `:69`).

`@mesureur` propose de porter la VM à **16 Gio** (`docs/mesures/import-legifacile-2026-11-06.md:469-471` et `:519-520`). Ce chiffre n'a pas été mesuré en 16 Gio, et il remplacerait ADR-0139, décision 3. Un autre levier existe, plus profond : retirer les textes de `CommitDelta.created`, axe (ii) du diagnostic de `CORRIGER-MEMOIRE-APRES-COMMIT`, pour environ **5 Go** sur `decisions` (`docs/suivi.md:790`). `@architecte` a jugé cet axe (ii) **NON-COUVERT** par ADR-0066, donc soumis à un ADR propre (`docs/suivi.md:1370`) ; seul l'axe (i) a été corrigé, et il a fait passer le pic d'`articles` de 8,24 à 6,54 Go (`docs/suivi.md:152`).

### Fait 3 — la machine de service est plus petite que celle des mesures

Le Mac mini, machine de référence de la latence contraignante de la v1 (ADR-0129, décision 2, `docs/adr/0129-replanification-de-la-fin-du-plan.md:36-40` et `:92`), porte une VM de **9,703 GiB** (`docs/mesures/mac-mini-2026-11-07.md:17`). Après la charge de la campagne, son instance a **2,275 GiB en swap** pour un `memory.current` resté sous 9,20 GiB : le rapport qualifie la marge de mince et demande qu'elle soit surveillée (`docs/mesures/mac-mini-2026-11-07.md:252` et `:338-349`, point 3). Un `replace_all` complet de `decisions`, qui demande 10,99 Gio d'anonyme sur la VM de 11,67 Gio, n'a donc pas sa place sur cette machine.

### Ce que la conception ne tranche pas

Aucun document accepté ne dit ce qu'on fait d'un dépassement de la cible de 60 s par la reconstruction lexicale, ni d'un `replace_all` complet qui ne tient pas dans la mémoire d'une machine de service. ADR-0047 réserve le cas à un ADR ; ADR-0087 renvoie la parallélisation de l'écrivain à un ADR si la mesure la justifie ; ADR-0139 fixe la RAM de sa propre campagne et ne dit rien de l'exploitation. La décision du propriétaire du 2026-11-08 comble ce vide.

## Décision

Les deux dépassements mesurés sont **acceptés comme limites de la v1**, et l'exploitation en tire une règle.

1. **Reconstruction lexicale : aucune correction en v1.** Le dépassement de 60 s sur `jorf`, `articles` et `decisions` est une limite connue, mesurée, de la v1 ; elle sera écrite dans le bilan de la v1. Le budget de 60 s d'ADR-0047 reste un budget de démarrage et **n'est pas relevé**. En v2, la reconstruction lexicale passe à **plus d'un thread d'écriture**, ce qui reviendra sur la constante notée par ADR-0087 (`:28` et `:75`).

2. **ADR-0139 n'est pas remplacé.** Sa campagne est close et ses chiffres restent ceux de la VM de 11,67 Gio. La proposition de 16 Gio n'est pas retenue ici : elle n'est pas mesurée, elle change le réglage d'une machine, et la marge de 0,58 Gio tient sur cette VM. Le présent ADR ne rouvre ni la décision 3 ni aucune autre décision d'ADR-0139.

3. **Règle d'exploitation.** Un `replace_all` complet d'un **grand index** — `decisions`, `articles` ou `jorf` — ne se fait pas sur le Mac mini. Il se fait sur la **machine de développement**, puis l'index rejoint la machine de service par **instantané** et **restauration** (ADR-0064, ADR-0065), comme le prévoit déjà ADR-0129 (`docs/adr/0129-replanification-de-la-fin-du-plan.md:42`) et comme l'a fait `MESURER-SUR-MAC-MINI` (ADR-0141, décision 2, `docs/adr/0141-protocole-de-mesurer-sur-mac-mini-et-installation-par-les-agents.md:60-67`). Les **lots quotidiens**, petits, s'attachent directement sur le Mac mini.

4. **En v2 : retirer les textes de `CommitDelta.created`** (axe (ii) du diagnostic de `CORRIGER-MEMOIRE-APRES-COMMIT`), sous un ADR propre. Le gain attendu est d'environ 5 Go sur `decisions` ; il n'est pas mesuré, et l'ADR de la v2 devra le mesurer avant de le retenir.

Ce qui n'est **pas** décidé ici : aucun seuil chiffré ne sépare un « grand index » d'un petit. La règle de la décision 3 nomme les trois index **mesurés** ; le cas d'un index futur de taille intermédiaire se tranchera sur mesure, et non par analogie. Le présent ADR ne décide pas non plus la forme de la parallélisation de l'écrivain lexical — nombre de threads, budget mémoire, nombre de segments produits : c'est un travail de v2, avec sa propre mesure.

## Conséquences

- **Positives** :
  - la v1 n'ouvre aucune tâche de performance sur un coût qui ne se paie qu'au démarrage, à la compaction ou sur une commande explicite ;
  - le budget de 60 s d'ADR-0047 garde son sens de budget de service, et la limite est écrite au lieu d'être contournée en silence ;
  - la règle d'exploitation évite un `replace_all` complet sur une machine qui a déjà 2,275 GiB de swap après une campagne de mesures, et elle réutilise un chemin déjà éprouvé, l'instantané et la restauration ;
  - ADR-0139 et sa campagne restent intacts : aucun chiffre accepté n'est réécrit, et aucune remesure n'est demandée ;
  - les deux points de v2 sont nommés et datés, donc ils ne dépendent plus de la mémoire d'une session.
- **Négatives / coûts** :
  - un redémarrage ou une compaction d'un grand index paie 71 à 185 s de reconstruction lexicale jusqu'à la v2 ;
  - un `replace_all` complet exige deux machines et un transfert d'instantané (`decisions` pèse 9,21 GiB, cité par `docs/adr/0141-protocole-de-mesurer-sur-mac-mini-et-installation-par-les-agents.md:41`), donc du temps et de l'espace sur le SSD externe ;
  - un dépassement futur, sur un index plus gros que `decisions`, ne sera pas couvert par cette décision : il faudra un nouvel ADR ;
  - la marge de 0,58 Gio reste mince pour un lot quotidien inattendu sur le Mac mini, puisque rien ne plafonne la mémoire du conteneur (ADR-0141, décision 3, `:72`).
- **À surveiller** :
  - **toute attache qui, sur le Mac mini, dépasse 9,20 GiB de `memory.current`** ou entre en swap de façon durable : le signal dirait que la règle de la décision 3 doit couvrir un index de plus, ou qu'un plafond de mémoire devient nécessaire ;
  - **le temps de reprise réel en exploitation** : si une reconstruction lexicale s'impose à un moment où le service doit répondre, la limite cesse d'être un coût de démarrage et la v2 doit être ouverte plus tôt ;
  - **le rejeu de la reconstruction après la parallélisation de la v2** : un gain très inférieur au rapport des threads indiquerait que le coût vient de l'analyse et du disque, et non de l'écriture tantivy ;
  - **la mémoire anonyme après un `replace_all` complet** : elle ne retombe qu'au redémarrage (10,55 Gio après l'attache, 3,57 Gio après redémarrage, `docs/suivi.md:787`). Un changement d'allocateur serait une dépendance nouvelle, donc un ADR.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Relever la cible de 60 s pour le cas de la reprise | Le budget d'ADR-0047 est un budget de service, écrit pour un redémarrage. Le relever effacerait le signal au lieu de le documenter, et ferait passer une limite mesurée pour une cible atteinte. La limite est écrite, la cible reste. |
| Corriger tout de suite, en v1, en parallélisant l'écrivain lexical | Revient sur la constante notée par ADR-0087 (`:28`), demande du code, un budget mémoire, un nombre de segments et une mesure ; MES-06 n'a pas montré ce budget structurant (`docs/suivi.md:103`). C'est un travail de v2, pas un correctif de fin de v1. |
| Remplacer ADR-0139 par une VM de 16 Gio | Le chiffre n'est pas mesuré en 16 Gio (`docs/mesures/import-legifacile-2026-11-06.md:519-520`) ; il change le réglage d'une machine de service ; et il traite un pic d'attache unique, alors que la règle d'exploitation le supprime à la source. |
| Retirer les textes de `CommitDelta.created` en v1 | `@architecte` juge l'axe (ii) NON-COUVERT par ADR-0066 (`docs/suivi.md:1370`) : il touche la construction du delta de commit, donc un ADR propre et une mesure. Le gain d'environ 5 Go est prédit, pas mesuré. |
| Ne pas écrire de règle et laisser l'exploitant décider au cas par cas | Les deux machines n'ont pas la même marge (11,67 Gio contre 9,703 GiB), et un `replace_all` complet est justement le geste qui ne se rejoue pas sans conséquence. Une règle écrite vaut mieux qu'un arbitrage de session, qui serait perdu. |
| Faire les `replace_all` complets sur le Mac mini avec un plafond `--memory` bas | Un plafond ne réduit pas le besoin de 10,99 Gio ; il transforme un dépassement mesuré en échec du conteneur, au milieu d'une attache. Le transfert par instantané est le chemin prévu par ADR-0064 et ADR-0065. |

## Application

Cet ADR est accepté par le propriétaire le 2026-11-08. L'application de ses décisions est la suivante :

1. **`docs/jalons/J10-bilan.md`** (tâche `BILAN-V1`, `docs/phases/P10-validation-a-l-echelle-reelle.md:197-205`, livrable nommé `:200`) reçoit les deux limites mesurées — reconstruction lexicale de 71, 127 et 185 s sur `jorf`, `articles` et `decisions` ; 0,58 Gio de marge à l'attache complète de `decisions` sur la VM de 11,67 Gio — et les **deux points de v2** des décisions 1 et 4 : paralléliser l'écrivain lexical, et retirer les textes de `CommitDelta.created`. Cette écriture n'est pas faite à ce jour : elle appartient à la tâche `BILAN-V1`, qui n'a pas encore été exécutée.
2. **La règle d'exploitation de la décision 3 est écrite dans `docs/07-stack-et-deploiement.md`**, section « Docker », sous-section « Contraintes propres au Mac », où elle prend la forme d'une puce de plus après la puce de la RAM de la VM (`:146`). C'est l'endroit de `docs/01` à `docs/08` qui décrit déjà les contraintes du Mac de service. La puce écrit qu'un `replace_all` complet d'un grand index se fait sur la machine de développement, puis que l'index passe par instantané et restauration (ADR-0064, ADR-0065) ; les lots quotidiens s'attachent sur le Mac mini.
3. **L'orchestrateur cite `ADR-0142`** dans les deux lignes du suivi qui portent la décision, `docs/suivi.md:789` et `:790`, et dans la section « Écarts à la conception » si le propriétaire y voit un écart. `@greffier` ne touche pas à `docs/suivi.md`.
4. **Aucune autre modification.** Le corps d'ADR-0139, celui d'ADR-0047, celui d'ADR-0087 et les fiches de tâches ne changent pas ; aucune mesure n'est refaite.

## Références

- `docs/suivi.md:789` et `:790` : les deux lignes datées du 2026-11-06 qui portent la décision du propriétaire ; `:787` (mémoire anonyme après attache), `:103` (MES-06), `:1370` (verdict d'`@architecte` sur l'axe (ii)).
- `docs/mesures/import-legifacile-2026-11-06.md` : `:35-36` (comptes), `:40` (pic d'attache), `:357-377` (reconstruction lexicale), `:465-471` et `:519-520` (points ouverts et proposition de 16 Gio).
- `docs/mesures/mac-mini-2026-11-07.md:17`, `:252` et `:338-349` (VM de 9,703 GiB, swap de 2,275 GiB, points à signaler).
- `docs/phases/P10-validation-a-l-echelle-reelle.md:197-205` (fiche `BILAN-V1`, livrable `docs/jalons/J10-bilan.md`).
- `docs/07-stack-et-deploiement.md:145-148` (contraintes propres au Mac, lieu proposé pour la règle d'exploitation).
- ADR-0047 (`docs/adr/0047-phase-de-mes-13-et-ordre-des-remedes-au-temps-de-demarrage.md:25`, `:44`, `:66`) : le budget de 60 s comme budget de démarrage, par index.
- ADR-0087 (`docs/adr/0087-reprise-lexicale-et-ouverture-en-lecture-seule.md:28`, `:34`, `:75`) : reprise lexicale à l'ouverture en écriture, budget de l'`IndexWriter` de 50 MiB sur un thread.
- ADR-0139 (`docs/adr/0139-protocole-de-la-campagne-d-import-a-l-echelle-reelle.md:27`, `:65`, `:69`) : budget par index, RAM de référence de la campagne.
- ADR-0064 (format et contenu d'un instantané) et ADR-0065 (instantané hors ligne, époque de restauration) : le mécanisme de transfert de la décision 3, déjà retenu par ADR-0129 (`docs/adr/0129-replanification-de-la-fin-du-plan.md:42`).
- ADR-0141 (`docs/adr/0141-protocole-de-mesurer-sur-mac-mini-et-installation-par-les-agents.md:41`, `:60-67`, `:72`) : précédent d'un transfert par instantané puis restauration.
