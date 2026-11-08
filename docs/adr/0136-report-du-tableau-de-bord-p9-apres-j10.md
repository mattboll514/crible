# ADR-0136 — Report du tableau de bord (P9) après J10

- **Statut** : Accepté — décidé par le propriétaire le 2026-11-03 (« ok ça me vas »), avec les deux recommandations
- **Date** : 2026-11-03 (proposition et décision le même jour)
- **Décideurs** : propriétaire
- **Proposé par** : `@greffier`, à la demande de l'orchestrateur, sur demande du propriétaire du 2026-11-03 (conversation avec l'orchestrateur) ; travail **hors tâche**, aucune tâche du plan ne le porte
- **Remplace** : — (ne remplace aucun ADR ; touche l'**ordre d'exécution** fixé par ADR-0129, décision 5, et la **place** de P9 fixée par ADR-0130, décision 1 ; voir « Points laissés au propriétaire »)

## Contexte

### Ce qui a changé, et pourquoi un ADR

ADR-0129 a replanifié la fin du plan le 2026-11-02 : `P0 → … → P8 → P9 → P10`, P9 pour le tableau de bord, P10 pour la validation à l'échelle réelle, **J10 pour la v1** (ADR-0129, décisions 5 et 7). ADR-0130 a fixé le contenu du tableau de bord : lecture seule, intégré à `crible serve` sous `/ui`, HTML/CSS/JS sans framework ni étape de build, tests en Rust seulement (ADR-0130, décisions 1 à 3 et 10 à 12), avec sept tâches dont `MAQUETTE-TABLEAU-DE-BORD` en premier (ADR-0130, décision 13).

Le propriétaire a changé d'avis le 2026-11-03, en deux temps :

> « ok je suis desolé, on va mettre de coté l'interface pour le moment, on ferra ça une fois que J10 sera terminé »

> « J'aimerais faire un vrai dashboard, surement avec un framwork comme next.js et de visuel plus propre tout en restant epuré, mais en gardant le meme fond que ce qui a ete defini, donc je te laisse decider si ça vaut le cout de renumeroté ou non »

L'interface sort donc de la v1, et son contenu sera revu à la reprise. Aucun document ne prévoit cet ordre : ADR-0129, décision 5, écrit que « le principe « une seule phase ouverte » impose P9 avant P10 » et `docs/09-plan-de-developpement.md:84-90` trace la même chaîne. Un ADR est nécessaire, puisque l'ordre du plan et le périmètre de la v1 changent.

### L'état exact au 2026-11-03

- `MAQUETTE-TABLEAU-DE-BORD` (première tâche de P9) avait été **commencée** le 2026-11-03. Un brouillon de maquette existait, **jamais commité ni relu**. Le propriétaire a demandé sa suppression ; le brouillon a été supprimé et les branches vides `phase/P9-tableau-de-bord` et `tache/MAQUETTE-TABLEAU-DE-BORD` ont été retirées.
- **Aucune autre tâche de P9 n'a commencé.** Aucun livrable de P9 n'existe dans le dépôt : ni `/ui`, ni route de liste des documents, ni actif embarqué.
- `docs/suivi.md:142` porte déjà la note « Commencée puis **mise de côté le 2026-11-03** par le propriétaire : la phase P9 est reportée après J10 (ADR de report à proposer) ». Le présent ADR est cet ADR ; le suivi sera mis à jour à son application.
- P8 est en cours ; J8 n'est pas validé.

### Ce que P10 ou J10 attendent de P9

Une dépendance a été cherchée dans tout `docs/phases/P10-validation-a-l-echelle-reelle.md`, `docs/passation-legifacile.md` et ADR-0129 (motifs `P9`, `J9`, « tableau de bord », `/ui`, `ROUTE-LISTER-DOCUMENTS`). Le résultat est net :

| Élément | Dépendance à P9 | Référence |
|---|---|---|
| Fiche P10, une seule ligne | `DEFINIR-INDEX-LEGIFACILE` : « **Dépend de** : J9 validé (ADR-0129) » | `docs/phases/P10-validation-a-l-echelle-reelle.md:28` |
| Autres tâches de P10 | aucune : elles dépendent de tâches de P10 ou de l'installation par le propriétaire | `:43`, `:62`, `:79`, `:91`, `:105`, `:127` |
| Critères de J10 (ADR-0129, décision 7) | aucun : six conditions de validation de Crible | ADR-0129, décision 7 ; `docs/phases/P10…:5-14` |
| `docs/passation-legifacile.md` | aucune ; les mentions de l'ancienne P9 sont historiques | `:7`, `:24` |

**P10 ne dépend donc d'aucun livrable de P9** : sa seule attente est le jalon `J9`, c'est-à-dire une validation, non un code. La remplacer par `J8` suffit.

### Les deux sens du mot « fin »

ADR-0129 appelle J10 « la fin du plan » (décisions 5 et 7, `docs/09:78-80`). Après cet ADR, J10 reste **la fin de la v1**, mais **le plan continue** : P9 et J9 viennent après. Les deux formulations devront être distinguées à l'application, sans quoi un lecteur croira que le plan s'arrête à J10.

### La numérotation

Deux options existaient : garder P9 et P10, ou renuméroter pour que l'ordre des numéros suive l'ordre d'exécution (P9 = validation et J9 = v1 ; P10 = tableau de bord et J10). L'orchestrateur a tranché sur délégation du propriétaire : **la numérotation est conservée**. Les raisons sont au point 3 de la décision.

### La question

Comment sortir le tableau de bord de la v1, exécuter P10 avant P9, et ne rien casser dans les documents, les ADR et le suivi — sans décider maintenant le contenu du futur tableau de bord ?

## Décision

**L'ordre d'exécution devient `P8 → P10 → P9`. P10 dépend de J8 validé au lieu de J9, J10 reste le jalon de la v1, et P9 est reportée après J10, hors de la v1. Les numéros de phase et de jalon ne changent pas. ADR-0130 n'est ni modifié ni remplacé ; son application est suspendue, et un nouvel ADR, écrit à la reprise et avant toute tâche de P9, tranchera le contenu du futur tableau de bord.**

### Décision 1 — ordre d'exécution : P8, puis P10, puis P9

La chaîne des phases devient :

```
P0 → P1 → P2 → P3 → P4 → P5 → P6 → P7 → P8 → P10 → P9
```

- **P10 dépend de J8 validé**, et non plus de J9. La modification porte sur la seule fiche qui l'écrivait (`docs/phases/P10-validation-a-l-echelle-reelle.md:28`) ; aucune autre tâche de P10 ne cite J9.
- **J10 reste le jalon de la v1.** Ses six critères (ADR-0129, décision 7) sont **inchangés**, à une réserve près : ils ne dépendent pas du tableau de bord, ce que le tableau des dépendances ci-dessus établit.
- La règle « une seule phase ouverte » (`docs/09:6`, `:90`) est **respectée** : les phases restent séquentielles. C'est la phrase d'ADR-0129 qui tire de cette règle la conclusion « P9 avant P10 » qui tombe, pas la règle.

### Décision 2 — P9 reportée après J10, hors de la v1

Le tableau de bord **sort des interfaces de la v1** (`docs/01:41`) et **rejoint le tableau du hors-périmètre de la v1** (`docs/01:45-56`), avec la mention « après J10 (ADR-0136) ». Le paragraphe qui explique aujourd'hui que « le tableau de bord n'est plus hors périmètre » (`docs/01:58`) est à revoir en conséquence : il dira que le tableau de bord est reporté après J10 et que son contenu sera redécidé.

**P9 et J9 restent dans le plan**, exécutées après J10. Ils ne sont ni supprimés ni fusionnés dans une autre phase : le propriétaire veut ce tableau de bord, seulement plus tard.

### Décision 3 — numérotation conservée

**P9 et P10 gardent leurs numéros et leurs noms**, et J9 et J10 gardent les leurs. L'ordre d'exécution est écrit explicitement dans `docs/09` (vue d'ensemble, section « Dépendances entre phases » et notes de jalon), ce qui suffit à lever l'ambiguïté entre numéro et ordre.

Les raisons :

- **Le contenu de P9 sera réécrit à la reprise** (décision 4). Renuméroter maintenant ferait porter un travail de réécriture à ADR-0129, aux fiches de P9 et de P10, au suivi, aux rapports de jalon et à toutes les mentions « J10 = v1 » — pour une phase dont le contenu va changer.
- **J10 est déjà employé partout comme le jalon de la v1**, dans ADR-0129 et dans les documents appliqués le 2026-11-02. Renuméroter rendrait J9 le jalon de la v1 et ferait relire tout le corpus, exactement ce qu'ADR-0129 a évité avec sa règle de correspondance bornée.
- **Les noms de jalons sont cités par des ADR acceptés** (`docs/suivi.md:21-22`, rapports `docs/jalons/`, notes de phase) : renuméroter rouvrirait des citations que rien n'oblige à toucher.

### Décision 4 — ADR-0130 : ni modifié, ni remplacé ; application suspendue

ADR-0130 reste **`Accepté`** et son corps n'est pas touché. Ce qui est suspendu, c'est son **application** : les fiches de P9 et les critères de J9 qu'il décrit ne seront pas exécutés dans l'état.

Le souhait du propriétaire pour la reprise est consigné ici comme une **intention, non décidée** :

- un **vrai tableau de bord**, plus soigné visuellement et toujours épuré, probablement bâti sur un **framework comme Next.js** ;
- sur **le même fond** que celui défini par ADR-0130 : lecture seule ; parcourir les données, tester une recherche, voir les analytics (décision 1) ; confidentialité de l'écran, aucun id interne, pas de journal de debug (décision 7) ; périmètre des informations affichées (décision 8) ; rôles `read` pour parcourir et chercher, `admin` pour les analytics (décision 5).

Ce souhait **contredirait** les décisions suivantes d'ADR-0130, et c'est pourquoi il n'est pas décidé ici :

| Décision d'ADR-0130 | Ce qu'elle impose | Ce que le souhait implique |
|---|---|---|
| 2 | Intégré à `crible serve`, sous `/ui`, dans la même image, sans second processus ni port | Une application Next.js est un second processus, avec son propre serveur et son propre port |
| 3 | Actifs embarqués par `include_bytes!`, handler axum, CSP stricte, aucun hôte externe | Un build de framework produit des actifs générés et un serveur Node à l'exécution |
| 10 | HTML/CSS/JS sans framework, sans étape de build, sans Node ; `cargo build` suffit | Une chaîne Node et une étape de build entrent dans la CI |
| 12 | Tests en Rust seulement, aucun testeur JavaScript | Un framework demande des tests JavaScript et un runner |
| 4 (probablement) | Même origine, aucune couche CORS | Une application servie sur une autre origine demande CORS, ou un proxy |

**Un nouvel ADR, écrit à la reprise et avant toute tâche de P9**, tranchera ces points, dira si ADR-0130 est remplacé (statut `Remplacé par ADR-NNNN`) ou seulement amendé, et réécrira les fiches et les critères de J9. Tant que cet ADR n'est pas accepté, **aucune tâche de P9 ne démarre** : la règle des ADR `Proposé` s'applique (`docs/adr/README.md:23`).

### Décision 5 — ce qui n'est pas décidé ici

- Le **contenu** du futur tableau de bord, son framework, son mode de service, sa place dans le dépôt et ses tests.
- Le **statut d'ADR-0130** à la reprise : amendement ou remplacement.
- Le **sort des tâches de P9** listées par ADR-0130, décision 13 (`SERVIR-UI-STATIQUE`, `ROUTE-LISTER-DOCUMENTS`, `PAGE-DONNEES`, `PAGE-RECHERCHE`, `PAGE-ANALYTICS`, `EPROUVER-UI`) : elles restent au plan, à réécrire.
- Toute **dépendance nouvelle**, Rust ou JavaScript, et toute **étape de CI** nouvelle.

## Application

Rien n'est appliqué tant que le propriétaire n'a pas accepté cet ADR. À l'acceptation, `@greffier` met à jour les documents suivants, en citant `ADR-0136`. **Application faite le 2026-11-03**, le jour de l'acceptation ; `docs/suivi.md` reste tenu par l'orchestrateur.

**Documents de conception**

- `docs/09-plan-de-developpement.md` :
  - `:79-80` (lignes du tableau des phases : ordre et but de P9, note « après J10 ») ;
  - `:84-90` (chaîne des dépendances : `… → P8 → P10 → P9`, et le texte qui dit que P9 passe avant P10, à remplacer) ;
  - `:5` si la formule « onze phases de P0 à P10 » doit préciser l'ordre d'exécution.
- `docs/01-vision-et-perimetre.md` :
  - `:41` (retirer le tableau de bord des interfaces de la v1) ;
  - `:45-56` (ajouter la ligne au tableau du hors-périmètre de la v1, avec « après J10 (ADR-0136) ») ;
  - `:58` (revoir le paragraphe « Le tableau de bord n'est plus hors périmètre »).

**Fiches de phase**

- `docs/phases/P10-validation-a-l-echelle-reelle.md` : `:28` (« **Dépend de** : J8 validé ») ; vérifier le reste de la fiche, qui ne cite P9 nulle part.
- `docs/phases/P9-tableau-de-bord.md` : en-tête, ajouter « Reportée après J10 (ADR-0136) ; fiches à réécrire à la reprise ». Le corps n'est pas réécrit à l'application de cet ADR.

**Suivi, tenu par l'orchestrateur**

- `docs/suivi.md` : `:21-22` (ordre et notes des jalons J9 et J10) ; `:142-148` (notes des tâches de P9) ; la ligne de `MAQUETTE-TABLEAU-DE-BORD` perd la mention « ADR de report à proposer », le présent ADR l'ayant remplacée.

**Non modifiés** : les corps d'ADR-0129 et d'ADR-0130 ; `docs/05-api.md` (aucune route de P9 n'existe) ; `docs/07-stack-et-deploiement.md` ; `docs/08-invariants-et-tests.md` (aucun invariant ne vise `/ui`) ; `docs/passation-legifacile.md`.

### Points laissés au propriétaire

Deux questions que le contexte fourni ne tranche pas, et que cet ADR ne tranchait pas non plus. **Le propriétaire les a tranchées le 2026-11-03**, en répondant « ok ça me vas » aux recommandations de l'orchestrateur : les deux recommandations sont suivies.

1. **ADR-0129.** Sa décision 5 fixe l'ordre `P8 → P9 → P10` et en tire une justification. Faut-il, à l'acceptation d'ADR-0136, inscrire dans l'index une note de renvoi sur ADR-0129 (comme pour les ADR partiellement remplacés), ou laisser ADR-0129 en l'état et ne compter que sur le présent ADR pour l'ordre ? L'application ci-dessus n'écrit aucune note de ce type sans décision.

   **Réponse (2026-11-03)** : note de renvoi dans l'index. La ligne d'ADR-0129 de `docs/adr/README.md` porte désormais « ordre des phases (décision 5) modifié par ADR-0136 : P8 → P10 → P9 ». Le fichier d'ADR-0129 lui-même n'est pas modifié.

2. **ADR-0130.** Restera-t-il `Accepté` jusqu'au nouvel ADR de reprise, avec une simple note « application suspendue », ou passe-t-il dès maintenant en `Déprécié` ? Cet ADR propose de le laisser `Accepté` : ses décisions de fond — lecture seule, confidentialité, périmètre, rôles — sont reprises par l'intention du propriétaire.

   **Réponse (2026-11-03)** : ADR-0130 reste `Accepté`, avec une note dans l'index. Sa ligne porte « application suspendue par ADR-0136 jusqu'à la reprise de P9 après J10 ». Son fichier n'est pas modifié.

## Conséquences

- **Positives** :
  - la v1 n'attend plus une interface : J10 peut être atteint dès que P10 est faite ;
  - P10 ne dépend d'aucun livrable de P9, et une seule ligne de fiche portait la dépendance ;
  - les fiches de P9 seront réécrites **une fois**, quand le contenu sera décidé, au lieu d'être écrites puis jetées ;
  - aucun numéro de phase, de jalon ou de tâche ne change : les citations d'ADR-0129, d'ADR-0130 et du suivi restent valides ;
  - aucun code, aucune dépendance et aucune étape de CI ne bougent : les tâches de P9 n'avaient rien livré.
- **Négatives / coûts** :
  - **le numéro de phase ne suit plus l'ordre d'exécution** : il faut lire la note d'ordre de `docs/09` pour savoir que P10 vient avant P9 ;
  - **« J10 = fin du plan » devient faux** : J10 est la fin de la v1, et J9 est le dernier jalon chronologique. Toute phrase qui dit « dernière phase du plan » ou « fin du plan » devra être relue ;
  - **ADR-0130 reste `Accepté` mais inappliqué** : le dépôt porte une décision valide dont les fiches ne seront pas exécutées, ce qui est une contradiction latente jusqu'au nouvel ADR ;
  - **un ADR de plus à écrire** à la reprise, et un chantier de réécriture des fiches de P9 ;
  - la **validation à l'échelle réelle se fait sans interface** : tout diagnostic passe par la CLI, l'API et Swagger UI ;
  - le tableau de bord, s'il arrive, arrivera sur un plan déjà clos, donc sans jalon de phase pour le porter.
- **À surveiller** :
  - **le besoin réel d'une interface pendant P10** : si l'inspection de l'index de 8,58 M passages devient pénible sans tableau de bord, l'ordre sera à rouvrir par un nouvel ADR. Les moyens actuels — CLI, API, `/docs` — couvrent la lecture, et ADR-0130 n'ajoutait qu'une route de liste ;
  - **le sort d'ADR-0130** : un ADR `Accepté` inappliqué pendant des mois se confond avec un ADR appliqué ; le nouvel ADR de reprise doit arriver avant toute tâche de P9, pas après ;
  - **la dérive documentaire** : toute mention de P9 dans un document écrit entre-temps doit porter la note de report, sans quoi un lecteur croira que P9 suit P8 ;
  - **le coût du framework à la reprise** : Next.js ajouterait Node à la CI, une étape de build, des dépendances à auditer et, sans doute, une couche CORS. C'est un coût à chiffrer dans le nouvel ADR, pas à découvrir en cours de tâche ;
  - **l'usage effectif** : si le tableau de bord n'est toujours pas commencé longtemps après J10, la question de le retirer du plan se posera.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| **Renuméroter** : la validation devient P9/J9 et la v1 J9, le tableau de bord devient P10/J10 | Ferait toucher ADR-0129, les deux fiches, le suivi, les rapports de jalon et toutes les mentions « J10 = v1 », pour une phase dont le contenu sera réécrit à la reprise. L'ordre écrit dans `docs/09` suffit à lever l'ambiguïté. |
| **Garder P9 avant P10** (statu quo d'ADR-0129) | Contredit la demande du propriétaire du 2026-11-03 : l'interface est mise de côté jusqu'à J10. |
| **Supprimer P9 et J9 du plan** | Le propriétaire veut ce tableau de bord, après J10. Le retirer du plan perdrait la trace de l'intention et de son contenu de fond. |
| **Fusionner P9 dans P10** | Mélangerait la validation de Crible et une interface, exactement ce qu'ADR-0129 a séparé le 2026-11-02 ; et J10 perdrait sa définition. |
| **Écrire dès maintenant les fiches du futur tableau de bord** | Le contenu n'est pas décidé : framework, mode de service, tests et place dans le dépôt sont ouverts. Les écrire maintenant produirait des fiches à jeter, comme la maquette du 2026-11-03. |
| **Remplacer ADR-0130 dès maintenant** | Un ADR accepté ne se réécrit pas, et le remplaçant ne peut pas être écrit sans décision sur le framework et le mode de service. Un remplacement se fera à la reprise. |
| **Adopter Next.js sur-le-champ, sans nouvel ADR** | Contredirait les décisions 2, 3, 10 et 12 d'ADR-0130 — intégration à `crible serve`, actifs embarqués, absence de framework, de build et de Node, tests en Rust seulement — et demanderait sans doute la décision 4 (même origine, pas de CORS). Chacun de ces points engage la CI, les dépendances et l'image : c'est un ADR, pas une conséquence du report. |
| **Rejouer `MAQUETTE-TABLEAU-DE-BORD` tout de suite, à blanc** | La maquette dépend du contenu, qui sera redécidé. Le brouillon du 2026-11-03 a déjà été supprimé à la demande du propriétaire ; aucune branche ne subsiste. |
| **Décaler J10 pour y inclure P9** | Fait dépendre la fin de la v1 d'une interface non décidée, et repousse la validation à l'échelle réelle, qui ne demande aucun livrable de P9. |

## Références

**Documents de conception**

- `docs/01-vision-et-perimetre.md` `:41` (interfaces de la v1), `:45-58` (hors-périmètre et paragraphe du tableau de bord), `:60-69` (critère de réussite de la v1)
- `docs/09-plan-de-developpement.md` `:5` (règle des phases et des jalons), `:6` (une seule phase ouverte), `:68-80` (vue d'ensemble des phases), `:82-90` (dépendances entre phases), `:92` (lecture des documents antérieurs à ADR-0129)
- `docs/suivi.md` `:21-22` (jalons J9 et J10), `:142-148` (tâches de P9), `:590` (note de lecture P9 → P10)
- `docs/phases/P10-validation-a-l-echelle-reelle.md` `:3-22` (but, J10, machines), `:28` (dépendance à J9), `:43`, `:62`, `:79`, `:91`, `:105`, `:127`
- `docs/phases/P9-tableau-de-bord.md` `:1-13` (but et J9), `:15-33` (`MAQUETTE-TABLEAU-DE-BORD`)
- `docs/passation-legifacile.md` `:7`, `:24`

**ADR**

- **ADR-0129** (replanification de la fin du plan) — décisions 3, 5 et 7 : règle de correspondance, table des phases et ordre `P8 → P9 → P10`, critères de J10
- **ADR-0130** (tableau de bord intégré) — décisions 1 à 3, 5, 7, 8, 10 à 14 : place en P9, intégration sous `/ui`, actifs embarqués, rôles, confidentialité, périmètre, absence de framework et de build, tests en Rust, tâches et critères de J9
- ADR-0014, ADR-0117, ADR-0118 (analytics sans contenu), ADR-0121, ADR-0122 (journal de debug), ADR-0124 (route `/v1/stats`, décisions 4 et 16 : rôles et pas de percentiles de pool)
- ADR-0100 (clés d'API), ADR-0101 (corps d'erreur), ADR-0111 (`/docs` servi sans clé, exception OpenAPI), ADR-0119 (`debug`), ADR-0123 (`explain_ids`)
- ADR-0133 (publication de l'image) — contexte de la même journée

**Code**

- `crates/crible-server/src/http.rs` — aucune route `/ui` n'existe au 2026-11-03 ; la lecture de P9 au moment du report est donc un état vide, pas un retrait.

**Conversation**

- Demandes du propriétaire du 2026-11-03, citées mot pour mot dans le contexte.
