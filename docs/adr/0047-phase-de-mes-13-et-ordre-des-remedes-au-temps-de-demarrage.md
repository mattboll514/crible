# ADR-0047 — Phase de MES-13 et ordre des remèdes au temps de démarrage

- **Statut** : Proposé
- **Date** : 2026-10-22
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, sur demande du propriétaire. Décision « D2 » de `docs/suivi.md`, section « Décisions en attente », issue de MES-13 (tâche `CONSTRUIRE-BITMAPS`, P1)
- **Remplace** : —

## Contexte

MES-13 avait deux questions à trancher, et les deux se posaient en même temps. La première est la **phase** de la mesure : `docs/08-invariants-et-tests.md` classe MES-13 en « P3 », alors que la fiche `CONSTRUIRE-BITMAPS` de P1, désormais `FAIT`, demande « MES-13 (premier chiffre) ». La seconde est l'**ordre des remèdes** au temps de démarrage : la fiche `MESURER-CYCLE-DE-VIE` de P3 fait du cache des bitmaps le premier remède proposé, alors que la mesure désigne le décodage JSON.

**Ce que la mesure dit.** Le rapport `docs/mesures/MES-13-temps-de-demarrage.md` a mesuré une fixture de 8,58 M lignes au schéma de `synth-small`, à cache chaud (§ 3) :

| Grandeur | Valeur | Source |
|---|---|---|
| Ouverture complète (`open`, p50) | 20,12 s | § 3 |
| Ouverture avec 10 % de lignes mortes | ≈ 22,4 s | § 4, calculé par la règle linéaire, non mesuré |
| Décodage JSON | 46,8 % (9 412 ms, 1,10 µs par ligne) | § 4 |
| Lecture SQLite | 32,2 % (6 473 ms) | § 4 |
| Insertion dans les structures | 21,2 % (4 276 ms) | § 4 |

Le rapport établit aussi que `FilterIndex::from_records` décode le JSON d'un document à **chaque passage** qui l'hérite : la fixture porte un document pour cinq passages. Décoder chaque document une seule fois épargnerait au mieux environ **7,5 s** — calculé, pas mesuré (§ 4 et § 8).

**Comparaison au seul budget écrit.** Il n'existe pas de cible chiffrée du démarrage dans `docs/08`. Le seul budget est celui de la fiche `MESURER-CYCLE-DE-VIE` : un cache des bitmaps est proposé si le démarrage dépasse 60 s à l'échelle de Legifacile. Les 20,12 s mesurés valent **33,5 % de ce budget**, soit une marge de 2,98 ; avec 10 % de lignes mortes, ≈ 22,4 s, soit 37 % (§ 8). Le rapport conclut : « Ne pas ouvrir `AJOUTER-CACHE-BITMAPS` sur la foi de ce chiffre », et renvoie la décision à P3, sur l'échantillon Legifacile et le schéma réel.

**Ce que la mesure ne dit pas.** Le point à 8,58 M est une fixture au schéma de `synth-small`, **pas** l'échantillon Legifacile ; le nombre de champs filtrables, la cardinalité des `keyword` et le nombre de passages par document changent le coût, et aucun n'est mesuré (§ 8). Le gain du décodage unique par document n'est pas mesuré non plus. Enfin, le cache des bitmaps n'existe pas : la mesure ne dit rien de son gain, de son format ni de son invalidation.

**Le coût d'un remède n'est pas le même selon le remède.** Le décodage unique par document ne change ni le format sur disque ni l'invalidation : il réordonne un calcul à l'intérieur de la reconstruction. Le cache des bitmaps ajoute, lui, **un format sur disque versionné** et une invalidation à chaque génération nouvelle. Le second est donc le plus coûteux, alors que la fiche de P3 le proposait en premier.

**Précédent de rédaction.** `docs/08` classe déjà MES-14 en « P1, P4 » : premier chiffre dans la phase qui en a besoin, mesure complète dans la phase qui la consomme. MES-13 suit maintenant le même modèle.

## Décision

La colonne de phase de MES-13 devient « P1, P3 » ; **aucun cache de bitmaps et aucune tâche d'optimisation du démarrage n'est ouvert avant P3** ; et la fiche `MESURER-CYCLE-DE-VIE` reçoit l'ordre des remèdes, le décodage unique par document avant le cache.

1. **Phase de MES-13 (`docs/08`).** La colonne de phase passe de « P3 » à « **P1, P3** », sur le modèle de MES-14 : le premier chiffre est livré en P1, sans cache, et la mesure complète, avec comparaison au chargement d'un cache, reste en P3. Le titre de la ligne ne change pas ; MES-13 reste une seule mesure, en deux temps. La fiche P1 `CONSTRUIRE-BITMAPS`, déjà `FAIT` et qui demandait ce premier chiffre, **n'est pas modifiée** : elle est satisfaite.

2. **Aucune optimisation du démarrage avant P3.** Ni le cache des bitmaps, ni une tâche de décodage unique par document, ni aucune autre tâche d'optimisation ne s'ouvre avant `MESURER-CYCLE-DE-VIE`. Trois raisons :
   - 20 s représentent **un tiers** du seuil de 60 s : la marge est de presque trois, et rien n'oblige à payer maintenant ;
   - le coût ne se paie qu'à **l'ouverture d'un index**, c'est-à-dire à un redémarrage ; il ne pèse ni sur la latence d'une requête, ni sur le débit d'attache ;
   - la fixture **n'a pas les proportions de Legifacile** (nombre de champs filtrables, cardinalités, passages par document). Optimiser sur ces proportions reviendrait à calibrer sur un schéma qui n'est pas celui du projet servi.

3. **Ordre des remèdes dans `MESURER-CYCLE-DE-VIE`.** Le critère est précisé, pas remplacé : le seuil reste 60 s à l'échelle de Legifacile.
   - Le rapport de mesure donne la **part du décodage JSON** dans le temps de démarrage et le **nombre moyen de passages par document** de l'échantillon Legifacile. Sans ces deux chiffres, l'ordre des remèdes ne peut pas être réévalué sur le schéma réel.
   - Si le démarrage dépasse 60 s, la mesure propose **d'abord** une tâche `DECODER-DOCUMENTS-UNE-FOIS`. Elle décode le JSON de chaque document une seule fois pendant la reconstruction des structures de filtrage, **sans changement de format et sans invalidation**, avec un test d'égalité des structures produites et un rejeu de MES-13.
   - La tâche `AJOUTER-CACHE-BITMAPS` n'est proposée que si le démarrage dépasse **encore** 60 s après cette première tâche, parce qu'elle ajoute un format sur disque versionné et une invalidation.

Ce qui n'est **pas** décidé ici : la forme exacte d'un futur cache des bitmaps, son format et sa règle d'invalidation ; le contenu détaillé de la fiche `DECODER-DOCUMENTS-UNE-FOIS` (elle sera écrite si le seuil de 60 s est dépassé en P3) ; et tout changement de seuil, de `N` ou de `exact_threshold`.

## Conséquences

- **Positives** :
  - `docs/08` et la fiche de P1 cessent de se contredire : MES-13 est en P1 pour le premier chiffre et en P3 pour la mesure complète, comme MES-14 ;
  - l'ordre des remèdes suit le coût : le décodage unique par document ne touche ni au format sur disque ni à l'invalidation, le cache des bitmaps touche aux deux ;
  - aucune tâche d'optimisation n'est ouverte sur un chiffre qui vaut un tiers du seuil, ni sur un schéma qui n'est pas celui de Legifacile ;
  - le rapport de P3 est obligé de livrer deux chiffres qui manquent aujourd'hui (part du décodage JSON, passages par document), sans quoi l'ordre des remèdes resterait une intuition.
- **Négatives / coûts** :
  - un démarrage de 20 à 22 s subsiste jusqu'en P3, et jusqu'à la tâche de décodage si elle est ouverte ensuite : chaque redémarrage paie ce temps ;
  - MES-13 porte désormais deux phases : la ligne de `docs/08` demande de lire `docs/mesures/MES-13-temps-de-demarrage.md` pour savoir ce qui est déjà fait ;
  - la mesure complète de MES-13, en P3, devra encore comparer la reconstruction à un chargement de cache — comparaison que rien ne permet de faire aujourd'hui, le cache n'existant pas ;
  - l'ordre des remèdes est écrit alors que le gain du décodage unique n'est pas mesuré : si le rejeu de MES-13 le dément, l'ordre devra être revu par un nouvel ADR.
- **À surveiller** :
  - le démarrage mesuré sur l'échantillon Legifacile en P3 : s'il dépasse 60 s, l'ordre « décodage puis cache » s'applique ; s'il reste sous 20 s, la question du cache se referme pour la v1 ;
  - le rejeu de MES-13 après `DECODER-DOCUMENTS-UNE-FOIS` : un gain très inférieur aux ≈ 7,5 s calculés indiquerait que la lecture SQLite ou le schéma réel dominent, et l'ordre des remèdes serait à corriger ;
  - tout index dont le redémarrage dépasserait 60 s en exploitation : le seuil est un budget de service, pas seulement un critère de mesure.

### Modifications à appliquer après acceptation

Aucune n'est appliquée par cet ADR : le propriétaire doit d'abord le passer en `Accepté`. Une fois accepté, `@greffier` applique les modifications suivantes, dans un commit qui cite `ADR-0047`.

1. **`docs/08-invariants-et-tests.md`**, ligne MES-13 du tableau des mesures. Remplacer

   ```
   | MES-13 | Temps de démarrage : reconstruction des bitmaps depuis SQLite contre chargement du cache | P3 |
   ```

   par

   ```
   | MES-13 | Temps de démarrage : reconstruction des bitmaps depuis SQLite contre chargement du cache | P1, P3 |
   ```

   Aucune autre ligne du tableau ne change ; la ligne MES-14 reste « P1, P4 ».

2. **`docs/phases/P3-cycle-de-vie.md`**, fiche `MESURER-CYCLE-DE-VIE`, section « Livrables ». Remplacer le troisième livrable

   ```
     - temps de démarrage, en reconstruisant les bitmaps puis en chargeant le cache.
   ```

   par

   ```
     - temps de démarrage, en reconstruisant les bitmaps puis en chargeant le cache, avec la part du décodage JSON dans ce temps et le nombre moyen de passages par document de l'échantillon Legifacile (ADR-0047) ;
     - l'ordre des remèdes à appliquer si le démarrage dépasse 60 s : d'abord `DECODER-DOCUMENTS-UNE-FOIS`, puis `AJOUTER-CACHE-BITMAPS` seulement si le seuil est encore dépassé (ADR-0047).
   ```

3. **`docs/phases/P3-cycle-de-vie.md`**, même fiche, section « Critères d'acceptation ». Remplacer le second critère

   ```
     - si le démarrage dépasse 60 s à l'échelle de Legifacile, un cache des bitmaps est proposé, sous forme de tâche `AJOUTER-CACHE-BITMAPS` à valider par le propriétaire.
   ```

   par

   ```
     - le rapport donne la part du décodage JSON dans le démarrage et le nombre moyen de passages par document de l'échantillon Legifacile ;
     - si le démarrage dépasse 60 s à l'échelle de Legifacile, la mesure propose **d'abord** une tâche `DECODER-DOCUMENTS-UNE-FOIS` : décoder le JSON de chaque document une seule fois pendant la reconstruction des structures de filtrage, sans changement de format ni invalidation, avec un test d'égalité des structures et un rejeu de MES-13 ; la tâche est à valider par le propriétaire ;
     - la tâche `AJOUTER-CACHE-BITMAPS`, qui ajoute un format sur disque versionné et une invalidation, n'est proposée que si le démarrage dépasse encore 60 s après `DECODER-DOCUMENTS-UNE-FOIS`, et reste à valider par le propriétaire (ADR-0047).
   ```

4. **`docs/phases/P1-stockage-et-recherche-exacte.md`** : aucune modification. La fiche `CONSTRUIRE-BITMAPS`, statut `FAIT`, demande « MES-13 (premier chiffre) » et ce premier chiffre existe.

5. **`docs/suivi.md`** : c'est l'**orchestrateur** qui tient ce fichier, pas `@greffier`. Il retire D2 de « Décisions en attente » quand l'ADR est accepté. `@greffier` ne touche ni au suivi des tâches ni au journal.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Ouvrir dès maintenant une tâche `DECODER-DOCUMENTS-UNE-FOIS` en P3, avant `MESURER-CYCLE-DE-VIE` | Le gain n'est pas mesuré : les ≈ 7,5 s se déduisent du coût du décodage (46,8 %) et de l'hypothèse d'un document pour cinq passages, qui est celle de la fixture. La marge est de presque trois sous le seuil de 60 s, et le coût ne se paie qu'à un redémarrage. Ouvrir la tâche maintenant reviendrait à optimiser avant de savoir si Legifacile en a besoin, et sur des proportions qui ne sont pas les siennes. |
| Aligner seulement `docs/08`, sans ordre des remèdes | La fiche de P3 continuerait de proposer le cache des bitmaps en premier, alors que c'est le remède le plus coûteux : un format sur disque versionné et une invalidation, pour un gain inconnu. Le défaut de phase serait corrigé et le défaut d'ordre conservé. |
| Modifier la fiche P1 `CONSTRUIRE-BITMAPS` pour retirer la référence à MES-13 | La fiche est `FAIT` et satisfaite : le premier chiffre existe et a servi à J1. Réécrire une fiche terminée effacerait la trace de ce qui a été demandé, et laisserait MES-13 sans premier chiffre dans `docs/08`. Aligner `docs/08` suffit. |
| Ouvrir `AJOUTER-CACHE-BITMAPS` maintenant, sur la foi des 20 s mesurés | Le seuil de 60 s n'est pas atteint, avec une marge de 2,98 ; la fixture n'a pas les proportions de Legifacile ; le cache ajoute un format sur disque versionné et une invalidation pour un gain non mesuré. C'est exactement ce que le § 8 de MES-13 recommande de ne pas faire. |

## Références

- `docs/mesures/MES-13-temps-de-demarrage.md`, § 3 (résultats à cache chaud), § 4 (décomposition, lignes mortes) et § 8 (conclusion, comparaison au budget, recommandations).
- `docs/08-invariants-et-tests.md`, tableau des mesures : MES-13 (P3, à passer en P1, P3) et MES-14 (P1, P4, précédent de rédaction).
- `docs/phases/P1-stockage-et-recherche-exacte.md`, fiche `CONSTRUIRE-BITMAPS` (« MES-13 (premier chiffre) », statut `FAIT`).
- `docs/phases/P3-cycle-de-vie.md`, fiche `MESURER-CYCLE-DE-VIE` (livrables et critère des 60 s).
- `docs/suivi.md`, « Décisions en attente » (D2).
- ADR-0018 (colonnes de valeurs en RAM), ADR-0019 (champs en JSON canonique), ADR-0006 (pierres tombales et compaction) : structures dont la reconstruction est mesurée.
- ADR-0046 : même méthode de rédaction, conséquences et modifications à appliquer après acceptation.
