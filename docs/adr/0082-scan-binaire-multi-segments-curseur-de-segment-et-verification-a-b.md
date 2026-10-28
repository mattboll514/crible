# ADR-0082 — Scan binaire multi-segments : curseur de segment et vérification A/B du parcours unique

- **Statut** : Accepté (accepté par anticipation le 2026-10-27, par délégation du propriétaire pour l'enchaînement `UNIFIER-PARCOURS-SCAN`, `REDUIRE-COUT-DES-CLES` et `GROUPER-LECTURES-VECTEURS`, à confirmer avec lui au point de fin des trois tâches, confirmé par le propriétaire le 2026-10-28)
- **Date** : 2026-10-27 (proposition et acceptation anticipée le même jour)
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, sur demande du propriétaire, à partir de la section « multi-segments » de `docs/mesures/MES-02-parcours-dense-clairseme.md` (mesure de `@mesureur` du 2026-10-27) et du verdict de `@architecte` sur la fiche `UNIFIER-PARCOURS-SCAN`
- **Remplace** : —

## Contexte

### La question

ADR-0081 a ramené le scan binaire à **un seul parcours, le clairsemé**, et il a rendu sa décision **conditionnelle à une mesure multi-segments** (décision 2, `docs/adr/0081-parcours-du-scan-binaire-un-seul-parcours-clairseme.md`). Cette mesure est faite. Elle remplit le **second** déclencheur qu'ADR-0081 avait écrit dans « À surveiller » : « un point de densité ou de taille où le dense repasserait devant … un seul suffirait à rouvrir la question » (`:82`). La question est donc rouverte, et elle arrive **pendant** `UNIFIER-PARCOURS-SCAN`, pas après.

Deux manques de texte s'y ajoutent, relevés par `@architecte` :

- la fiche ne dit pas ce que veut dire « au moins aussi bonne » : le critère « la latence du scan est au moins aussi bonne que celle de l'ancienne règle sur **tous** les points mesurés » (`docs/phases/P4-recherche-rapide.md:151`) n'a **aucune marge de bruit** ;
- le percentile du budget « quelques ms » n'est défini nulle part (`docs/03-recherche.md:144`, dans un tableau intitulé « Budget de latence indicatif », `:135`). ADR-0081 renvoie ce point à la mesure de P8.

### Ce que la mesure multi-segments établit

`@mesureur` a copié `synth-large` en trois versions — 1 segment, 8 segments (7 × 1 M + 740 000), 100 segments (100 × 86 000) — et rejoué la campagne sur les mêmes pools, à N = 5000, avec l'ordre des paires alterné d'ADR-0081. La section est ajoutée à la fin de `docs/mesures/MES-02-parcours-dense-clairseme.md` (`:594-804`) et n'est pas encore commitée.

| Copie | Points où le clairsemé est devant | Points perdus par le clairsemé |
|---|---|---|
| 1 segment | 16 sur 16 | aucun |
| 8 segments | 17 sur 18 | `density-1000000-100`, **+6,0 %** |
| 100 segments | 12 sur 16 | `contiguous-1000000` **+10,4 %**, `density-1000000-75` **+2,6 %**, `density-1000000-90` **+10,5 %**, `density-7740000-90` **+1,4 %** |

Quatre faits portent la lecture :

- **le clairsemé ralentit de façon monotone avec le nombre de segments.** Dans les 12 combinaisons tour × pool de la série entrelacée, p50(clairsemé) est strictement ordonné `1 < 8 < 100` ; l'écart « 100 vs 1 » vaut **+8,6 à +19,5 %** en médiane selon le pool et **+6,5 %** au minimum observé (`MES-02:701-705`). À 8 segments, l'écart comparable vaut **+3,2 % et +3,6 %** (`:744-748`), au-delà du seuil « quelques pour cent » qu'ADR-0081 avait écrit (`ADR-0081:80`) ;
- **le témoin dense ne bouge pas** entre 1 et 100 segments : +0,5 %, +0,2 %, −2,6 % et +1,5 %, tous dans **±2,6 %** (`MES-02:706-709`). Le découpage ne coûte donc rien au dense : le surcoût est propre au clairsemé ;
- **le coût est par passage, pas par requête.** Ramené au passage vivant, il vaut **0,93 à 1,03 ns** sur les quatre pools de 100 segments, constant (`:711-715`). Il est dans `segment_of` (`crates/crible-search/src/dense/binary.rs:419-435`), que `sparse_window` appelle pour **chaque** iid vivant (`:404`) et qui fait une recherche binaire `partition_point` dans la table des segments (`:422`). `dense_window` (`:351-388`) ne le paie pas : il boucle sur les segments à l'extérieur, donc le segment est déjà en main ;
- **l'attribution explique les points perdus.** À 1 segment, `segment_of` est quasi gratuit et le clairsemé gagne partout. Les quatre points que le dense reprend à 100 segments sont **tous des pools denses** (0,75 et 0,90), c'est-à-dire la zone où le dense avait déjà un avantage d'ordre mémoire (`MES-02:596-599`). Le surcoût par passage y pèse le plus lourd, parce que le clairsemé y parcourt un pool presque aussi large que la plage.

Les contrôles tiennent : `agree` rend 48 pools sur 48 identiques, mêmes iid, mêmes distances, même ordre (`:758-760`) ; le budget « 20 à 80 ms » est tenu partout (35,5 à 48,1 ms en p50, 39,0 à 59,9 ms en p95 sur le pool complet) ; le p50 du pool de 1 M reste entre 6,8 et 8,6 ms, mais son **p95 atteint 10,3 à 10,5 ms** sur les deux copies multi-segments (`:717-727`, `:761-766`).

### Ce que la mesure ne tranche pas

Le régime au-delà de 100 segments n'est pas mesuré (Legifacile compacté est attendu à environ 9 segments, `docs/02-architecture.md:179`). Les petits pools (1 k, 10 k, 100 k) ne sont pas mesurés. Trois tours ne suffisent pas à réduire le bruit sous quelques points : les amplitudes de la série entrelacée vont jusqu'à 21 points sur un pool (`MES-02:770-773`), et la même copie, à 43 minutes d'intervalle, varie de −10,1 % à +26,0 % (`:656-659`).

## Décision

**La question rouverte par ADR-0081 est tranchée : le remède est le curseur de segment, mis en œuvre dans `UNIFIER-PARCOURS-SCAN` avec le parcours unique. Aucun second parcours ne revient, ni comme filet ni sous condition. Le résultat est vérifié après le code par une mesure A/B, dont la règle de bruit est écrite ici.**

Précisions :

1. **La cause des points perdus est le coût par passage de `segment_of`.** Il est mesuré (0,93 à 1,03 ns par passage vivant à 100 segments) et il est absent à 1 segment, où le clairsemé gagne 16 points sur 16. La réponse est donc le **curseur de segment** que la décision 2 d'ADR-0081 nomme déjà : un curseur qui avance avec l'itérateur trié de la bitmap du pool, en **O(1) amorti**, au lieu d'une recherche binaire par iid. Il est livré dans `UNIFIER-PARCOURS-SCAN`, avec le test du curseur sur des segments de tailles inégales que la fiche demande déjà (`docs/phases/P4-recherche-rapide.md:147`). Un second parcours ne revient pas. La décision 2 d'ADR-0081 et sa phrase « Le remède n'est pas le retour d'un second parcours » (`ADR-0081:55`) restent la règle.
2. **Vérification après le code, avant de déclarer la tâche `FAIT`.** `@mesureur` fait une mesure **A/B** :

   - **binaire ancien** : la règle de densité d'ADR-0071, construite depuis le commit `e7d4e08` (dernier commit où elle vit), **hors de l'arbre de travail** — par exemple `git archive` vers `data/` — pour que le code de la tâche ne la contamine pas ;
   - **binaire nouveau** : parcours unique et curseur ;
   - **copies** : les trois de la campagne multi-segments (1, 8 et 100 segments) ;
   - **points** : **tous** ceux de la campagne multi-segments, sans exception ni échantillon ;
   - **protocole** : celui d'ADR-0077 (N = 5000, 300 requêtes, 3 passes, p50, p95, p99), avec l'ordre des paires **alterné** d'ADR-0081 ;
   - **contrôle** : les deux binaires rendent les **mêmes candidats**, mêmes iid, mêmes distances, même ordre.

   Le résultat est consigné en section datée du rapport MES-02. Il rapporte p50 et p95.
3. **Règle de bruit du critère « au moins aussi bonne ».** On compare le **p50 point par point**. Un point est **perdu** seulement si le nouveau binaire est **plus lent en p50** *et* que l'écart **dépasse l'amplitude de ses propres écarts d'une paire à l'autre sur ce point**. C'est la définition de « dans le bruit » que la campagne du 2026-10-27 emploie déjà : « 17 points sur 18 dépassent l'amplitude de leur propre écart d'une paire à l'autre. Le seul point dans le bruit est celui du match nul » (`docs/mesures/MES-02-parcours-dense-clairseme.md:266-267`). Un point dans cette amplitude est un **match nul**, et un match nul **satisfait** le critère de la fiche.
4. **Si un point est perdu au-delà du bruit.** La tâche remonte une `QUESTION-CONCEPTION`, passe en `BLOQUE` et s'arrête. Seul le propriétaire décide de la suite, par un ADR qui **remplacerait ADR-0081**. Aucun repli n'est écrit dans le code : ni rétablissement de la règle de densité, ni seuil conditionnel, ni second parcours derrière un drapeau.
5. **Le percentile du budget « quelques ms » n'est pas tranché ici.** La remesure A/B le **rapporte** (p50 et p95), et **P8 le juge en conteneur**, comme ADR-0081 le prévoit (`ADR-0081:83`, `:61`). Cet ADR ne fixe ni un percentile, ni une lecture du budget de `docs/03-recherche.md:144`.

Ce qui n'est **pas** décidé ici : la valeur du budget de `docs/03-recherche.md:144`, le nombre de segments réel de Legifacile, le régime au-delà de 100 segments, les petits pools, et le sort d'ADR-0081 si un point est perdu — ce sort appartient au propriétaire, par l'ADR de remplacement du point 4.

## Conséquences

- **Positives** :
  - la question qu'ADR-0081 avait laissée ouverte est **fermée avec un chiffre** : la cause est attribuée (0,93 à 1,03 ns par passage dans `segment_of`), et le remède est celui que l'ADR nommait déjà, pas une règle nouvelle ;
  - le critère de la fiche devient **exécutable** : sans la règle de bruit, « au moins aussi bonne sur tous les points » aurait fait échouer la tâche sur un écart de 1,4 % à 100 segments, alors que la même machine varie de −10,1 % à +26,0 % sur une même copie ;
  - le curseur supprime un coût **par passage**, donc il agit aussi sur les index à venir, dont le nombre de segments croît avec les attaches journalières ;
  - l'A/B compare le nouveau code à **ce qui tournait réellement**, pas à un chiffre de rapport lu dans un autre contexte ; il protège donc contre une régression silencieuse du parcours unique ;
  - le contrôle « mêmes candidats » garde INV-06 et l'égalité au bit près d'ADR-0072 pendant le changement de parcours.
- **Négatives / coûts** :
  - la mesure A/B demande **deux binaires** et un build depuis `e7d4e08` hors de l'arbre : c'est une manipulation de plus, et elle suppose que les copies de `data/mes02` restent disponibles ;
  - le curseur ajoute un **état** à `sparse_window` : il dépend de l'itérateur trié de la bitmap, et il faudra un test sur des segments de tailles inégales — le cas de `synth-large-8seg` (7 × 1 M + 740 000) et celui de `synth-large-multi` (100 × 86 000) sont disponibles ;
  - trois paires ne réduisent pas le bruit sous quelques points : la règle du point 3 rend le critère **franchissable**, elle ne rend pas la mesure précise ;
  - la mesure est faite sur une machine chargée (charge 3,9 à 4,4, `MES-02:288-289`), pas en conteneur ;
  - un point perdu au-delà du bruit **bloque tard**, après le code : la tâche aura coûté son implémentation avant que le propriétaire ne tranche ;
  - le p95 du pool de 1 M (10,3 à 10,5 ms) reste **au-dessus** de la lecture de 10 ms retenue par la campagne précédente, et cet ADR ne le corrige pas : il le renvoie à P8.
- **À surveiller** :
  - **le résultat de l'A/B** : si l'écart « 100 vs 1 » du clairsemé ne retombe pas dans le bruit du témoin dense (±2,6 %), le curseur n'a pas supprimé le coût, et un point perdu au-delà du bruit rouvre la question devant le propriétaire ;
  - **le nombre de segments réel de Legifacile** à `DEFINIR-INDEX-LEGIFACILE` : les ~9 segments de `docs/02:179` sont une déduction de la règle de compaction, pas un compte sur les données ;
  - **le p95 du pool de 1 M en conteneur** : P8 juge « quelques ms » avec ses propres chiffres (`MESURER-EN-CONTENEUR-MAC`) ;
  - **la constance du coût par passage au-delà de 100 segments** : elle est mesurée sur quatre pools à 100 segments, pas au-delà.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Rouvrir ADR-0081 maintenant et rétablir la règle de densité pour les index multi-segments | Deux parcours à maintenir, contre la décision 2 d'ADR-0081 et sa phrase « Le remède n'est pas le retour d'un second parcours » ; le dense ne gagne que 1 à 4 points denses (+1,4 à +10,5 %) là où le clairsemé rend −20 à −53 % sur les pools dispersés des mêmes copies |
| Comparer sans marge de bruit | Fragile : la dispersion de la machine atteint plusieurs pour cent d'une paire à l'autre, et jusqu'à 21 points d'amplitude sur un pool de la série entrelacée ; le critère « tous les points » ferait échouer la tâche sur un écart que la mesure ne distingue pas |
| Une marge fixe en pourcentage (par exemple 5 %) | Arbitraire et sans mesure : elle serait trop large sur les points stables et trop étroite sur les points à 21 points d'amplitude ; l'amplitude observée point par point est déjà dans le rapport |
| Le curseur **dès maintenant**, sans A/B après le code | Le curseur est décidé (point 1), mais rien ne prouverait ensuite qu'il a supprimé le coût ; l'A/B est le seul contrôle qui relie la cause mesurée à l'effet obtenu |
| Trancher ici le percentile du budget « quelques ms » | Aucune mesure en conteneur n'existe encore ; fixer un percentile sur des chiffres natifs d'une machine chargée serait une valeur par défaut sans observation (règle 12 d'`AGENTS.md`, `docs/03:135` budget indicatif) |

## Application du 2026-10-27

L'ADR est accepté par anticipation. Ce qui a été appliqué : la fiche `UNIFIER-PARCOURS-SCAN` (`docs/phases/P4-recherche-rapide.md`) reçoit le renvoi à cet ADR au critère de latence (`:151` : A/B et règle de bruit) et au livrable (2) (`:147` : le curseur est **obligatoire**, ADR-0081 « À surveiller » et ADR-0082), et la ligne d'index de `docs/adr/README.md`. ADR-0081 et `docs/03-recherche.md` ne sont **pas** modifiés : ADR-0081 est accepté et son corps ne se réécrit pas, et le budget de `docs/03:144` reste renvoyé à P8.

| Document | Modification |
|---|---|
| `docs/phases/P4-recherche-rapide.md:144` | la ligne « À lire » cite cet ADR et la section « multi-segments » de MES-02 |
| `docs/phases/P4-recherche-rapide.md:147` | le curseur de segment passe de conditionnel à obligatoire, avec renvoi à ADR-0081 (« À surveiller ») et à cet ADR |
| `docs/phases/P4-recherche-rapide.md:151` | le critère de latence renvoie à l'A/B et à la règle de bruit du point 3 |
| `docs/phases/P4-recherche-rapide.md:152` | la condition d'arrêt « avant le code » est close : la question a été tranchée, seule reste la perte d'un point au-delà du bruit |
| `docs/adr/README.md` | ligne d'index de cet ADR en `Accepté` |
| `docs/suivi.md` | ligne dans « Écarts à la conception », tâche `UNIFIER-PARCOURS-SCAN` |

Le code, les tests et le protocole de MES-02 ne sont pas touchés par cet ADR : la mesure A/B et le curseur appartiennent à la tâche.

## Références

- `docs/adr/0081-parcours-du-scan-binaire-un-seul-parcours-clairseme.md` — décision 2 (curseur de segment, jamais un second parcours), « Conséquences » (`:73`, le régime multi-segments comme seul argument qui pourrait renverser la décision), « À surveiller » (`:80`, `:82`)
- `docs/mesures/MES-02-parcours-dense-clairseme.md` — section « multi-segments » du 2026-10-27 (non commitée) : `:594-599` points gagnés et perdus par copie, `:601-629` campagne séquentielle écartée, `:661-709` série entrelacée et témoin dense, `:711-715` coût par passage, `:717-727` budget, `:775-784` question remontée ; `:266-267` définition du « dans le bruit »
- `crates/crible-search/src/dense/binary.rs` — `sparse_window` (`:404`, appel de `segment_of` par iid), `segment_of` (`:419-435`, `partition_point` `:422`), `dense_window` (`:351-388`)
- `docs/phases/P4-recherche-rapide.md:147` — livrable (2), curseur et test sur segments inégaux ; `:151` — critère de latence ; `:144` — protocole de mesure à relire
- `docs/03-recherche.md:135` — « Budget de latence indicatif » ; `:144` — « Scan binaire : 8,6 M / pool de 1 M | 20 à 80 ms / quelques ms »
- `docs/adr/0077-protocole-de-mesure-de-latence-de-la-recherche-dense.md` — N, passes alternées, p50/p95/p99, RSS
- `docs/adr/0071-seuil-entre-parcours-dense-et-clairseme-du-scan-binaire.md` — la règle de densité qui sert de binaire ancien dans l'A/B, remplacée par ADR-0081
- `docs/adr/0072-un-seul-calcul-du-produit-scalaire-pour-exact-et-le-rescoring.md`, décision 3 — égalité au bit près, que le contrôle « mêmes candidats » préserve
- `docs/adr/0006-segments-pierres-tombales-compaction.md` — un segment couvre une plage contiguë d'iid, ce dont le curseur dépend
- `docs/02-architecture.md:179` — segments d'au plus 1 M passages à la compaction
- `AGENTS.md`, règle 12 (pas de valeur par défaut sans mesure)
