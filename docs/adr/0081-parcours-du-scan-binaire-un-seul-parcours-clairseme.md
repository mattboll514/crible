# ADR-0081 — Parcours du scan binaire : un seul parcours clairsemé

- **Statut** : Proposé
- **Date** : 2026-10-27
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, sur demande du propriétaire, à partir de la campagne MES-02 du 2026-10-27 (`docs/mesures/MES-02-parcours-dense-clairseme.md`, commit `17fc786`) et de l'analyse de `@architecte`
- **Remplace** : — (ADR-0071 devient caduc si cet ADR est accepté ; son statut n'est pas modifié tant que le propriétaire n'a pas tranché)

## Contexte

### La question

ADR-0071 choisit le parcours du scan binaire selon la densité du pool : **dense** (codes contigus, test de la bitmap) quand `2 × taille du pool ≥ plage d'iid occupée`, **clairsemé** (itération de la bitmap) sinon. Sa section « À surveiller » demandait à MES-02 deux choses : des chiffres de latence **par densité de pool**, et la comparaison des **deux parcours sur un même pool**. C'est cette mesure qui est arrivée, et qui instruit la question : **ce seuil doit-il rester, et s'il tombe, que devient le parcours dense ?**

### Ce que la mesure établit

La campagne du 2026-10-27 rejoue MES-02 sur `synth-large` (7 740 000 passages vivants sur 8 600 000 iid, 768 dimensions, un seul segment), avec **3 paires de passes alternées** après un échauffement séparé, 300 requêtes, N = 5000, et deux tailles de plage (1 M et 7,74 M iid) sur des densités de 0,10 à 0,90. Les deux parcours sont **forcés** par le harnais : sans cela, ADR-0071 n'en aurait fait tourner qu'un et il n'y aurait rien à comparer.

Résultat : **le clairsemé gagne sur 17 points et fait match nul sur le dix-huitième. Aucun point mesuré ne donne le dense vainqueur.**

| Campagne | Pool | Densité | écart p50 clairsemé / dense |
|---|---|---|---|
| N = 5000 | 774 000 | 0,10 | −53,1 % |
| N = 5000 | 3 870 000 | 0,50 | −35,8 % |
| N = 5000 | 900 000 | 0,90 | −4,5 % |
| N = 5000 | 7 740 000 (complet) | 0,90 | −19,2 % |
| N = 5000 | 1 000 000 (contigu) | 0,90 | −5,6 % |
| N = 1000 | 1 000 000 (contigu) | 0,90 | −0,0 % (match nul) |

Trois faits pèsent sur la lecture :

- **le gain n'est pas réservé aux pools dispersés.** C'est sur les pools **contigus**, ceux que la compaction produit avec `cluster_by` (`docs/03-recherche.md:54`), que la règle actuelle envoie le dense : 5 à 36 % du scan y sont perdus ;
- **le modèle de coût d'ADR-0071 est contredit.** Il attendait du dense un avantage d'ordre mémoire dès la moitié de densité ; la mesure n'en voit aucun, même à 0,90 ;
- **le −11,8 % de MES-02 sur 1 M contigu était du bruit.** À N = 1000, les deux parcours sont indiscernables (−0,0 %, dispersion de 34,2 %) ; à N = 5000, le gain est de −5,6 %.

Le biais d'ordre est d'environ **0,4 % par passe en faveur du dense** (l'ordre dense puis clairsemé ne s'inverse pas d'une paire à l'autre), donc le résultat est **conservateur** : il sous-estime le gain du clairsemé.

Les contrôles tiennent : `agree` compare les deux parcours sur **16 pools et 18 points** et rend les mêmes candidats, même iid, même distance, même ordre ; le test `dense_and_sparse_traversals_agree_with_a_full_sort` compare le moteur à un tri complet à chaque `cargo test` ; le budget de `docs/03-recherche.md:143` est tenu dans les deux cas, le choix du parcours ne faisant basculer aucun point hors budget.

### Ce que la mesure ne tranche pas

- **Le nombre de segments.** `synth-large` est **mono-segment**. Or `sparse_window` paie un `segment_of` par passage — une recherche binaire (`partition_point`) dans la table des segments — que `dense_window` ne paie pas. Legifacile compacté aura environ **9 segments** de 1 M passages (`docs/02-architecture.md:179`), et un index nourri par attaches bien davantage. « Segments non contigus » n'existe pas : chaque segment couvre une plage contiguë d'iid (`docs/02:50`, ADR-0006) ; la variable est le **nombre** de segments. C'est l'explication probable de l'avantage du clairsemé, et elle n'est pas mesurée.
- **La densité 1,0.** Elle n'est pas constructible sur `synth-large`, qui porte 90 % de vivants : la série monte à 0,90.
- **Les pools de moins de 100 000 passages.** La campagne ne descend pas en dessous. Les seuls points disponibles (MES-02, à N = 1000) donnent le dense devant de 0,3 à 2,0 % sur 1 k, 10 k et 100 k — dans la dispersion de 16,1 % de cette campagne, donc ils ne concluent rien.
- **La neutralité du harnais.** La réplique ne fait pas le comptage des vivants ni la comparaison de rang que fait `dense_window` en production : elle est **favorable au dense**, et ce biais va contre la conclusion.

## Décision

**Le scan binaire n'a plus qu'un parcours : le parcours clairsemé.** Le seuil de densité d'ADR-0071 tombe, et `traversal` comme `dense_window` sont supprimés. La décision est **sous condition** d'une mesure multi-segments, décrite au point 2 ci-dessous.

Précisions :

1. **Un seul parcours, à toute densité et toute taille de pool.** Ce qui n'est pas mesuré n'est pas un motif de garder du code mort : maintenir deux parcours pour 0,3 à 2,0 % sur des scans de 0,05 à 1 ms serait de la complexité sans chiffre.
2. **La mesure multi-segments est un préalable à la mise en œuvre, pas un complément.** Elle se fait par passes alternées sur **une copie compactée** (environ 8 segments de 1 M) et sur **une copie multi-attaches** (nombreux petits segments), même pool, même plage, N = 5000, 300 requêtes, 3 paires. Si elle révèle un coût lié au nombre de segments, le remède est un **curseur de segment** qui avance avec l'itérateur trié de la bitmap — O(1) amorti — au lieu d'une recherche binaire par iid. **Le remède n'est pas le retour d'un second parcours.**
3. **Le reste du protocole est celui de la campagne du 2026-10-27**, avec trois corrections : l'**ordre des paires est alterné** (dense → clairsemé, puis clairsemé → dense) pour retirer le biais de 0,4 % ; les petits pools (1 k, 10 k, 100 k) ne sont mesurés **que si le propriétaire préfère un seuil de taille**, sinon ils sont déclarés non mesurés ; la densité 1,0 est obtenue par un index **sans morts**, ou déclarée hors d'atteinte.
4. **Les contrôles sont conservés, sans exception** : le mode `agree` (les deux parcours rendent les mêmes candidats, sur les mêmes pools), le test `dense_and_sparse_traversals_agree_with_a_full_sort` tant que les deux parcours coexistent, et le budget de `docs/03-recherche.md:143`. Ce dernier n'est pas réécrit : la mesure montre qu'il est tenu par le clairsemé.
5. **Le résultat ne change pas.** Un seul parcours rend les mêmes candidats que deux : c'était déjà la propriété qu'ADR-0071 exigeait et que le test compare à un tri complet. INV-06 et l'égalité au bit près d'ADR-0072, décision 3, ne sont pas touchés.
6. **La fiche `SCANNER-POOL-BINAIRE` doit être mise à jour**, par le propriétaire à l'acceptation : son livrable (`docs/phases/P4-recherche-rapide.md:48`) demande encore les deux parcours. Tant que la fiche n'est pas corrigée, elle et cet ADR se contredisent.
7. **Dépendance croisée, à écrire et non à décider ici.** Un scan clairsemé **trié** rend les candidats déjà dans l'**ordre croissant des iid**, puisqu'il suit l'itérateur d'un bitmap. Cela sert le parcours ordonné qu'ADR-0079 demande en **décision 2** (localité des pages de `records.sqlite` et des vecteurs) et le préchargement `MADV_WILLNEED` qu'ADR-0080 teste en **option 3**. Ces deux ADR ne sont pas modifiés ; la dépendance est notée dans leur section « À surveiller » par le propriétaire.
8. **Le conteneur de P8 n'est pas bloqué.** Le scan ne lit que la RAM (ADR-0069, décision 3) : ni page fault, ni `F_NOCACHE`, ni cgroup ne changent le choix du parcours.

Ce qui n'est **pas** décidé ici : une règle de seuil par taille de pool (option (b) ci-dessous), un point de bascule de densité autre que la moitié (option (c)), la réécriture d'ADR-0071, le protocole de mesure de MES-02 lui-même, et le budget de `docs/03-recherche.md:143`.

## Conséquences

- **Positives** :
  - un seul parcours à écrire, tester, mesurer et expliquer : moins de code, un seul cas de repli, une seule réplique de harnais (règle 12 d'`AGENTS.md`) ;
  - le coût mesuré du seuil disparaît : 5 à 36 % du scan sur les pools contigus que `cluster_by` produit, jusqu'à −19,2 % sur le pool complet ;
  - les candidats sortent triés par iid, ce qui profite à ADR-0079 (décision 2) et ADR-0080 (option 3) ;
  - le comportement ne dépend d'aucune configuration et d'aucune densité : un seul chemin, donc un seul endroit où chercher une erreur.
- **Négatives / coûts** :
  - **le régime multi-segments n'est pas mesuré**, et c'est le seul argument qui pourrait renverser la décision : le clairsemé paie un `segment_of` par passage, coût qui croît avec le nombre de segments ;
  - **les petits pools ne sont pas tranchés** : aucune donnée au-dessous de 100 000 passages, et les trois points existants sont dans le bruit ;
  - la densité 1,0 reste non mesurée tant qu'un index sans morts n'est pas construit ;
  - supprimer `dense_window` supprime un parcours dont l'ordre mémoire reste favorable sur un index mono-segment très dense (0,90 sur 1 M contigu : −4,5 %) : on renonce à ce petit gain ;
  - la fiche `SCANNER-POOL-BINAIRE` reste fausse jusqu'à sa mise à jour, et une revue qui la lit seule conclura à un manquement ;
  - le remède éventuel (curseur de segment) ajoute un état à `sparse_window` : il faudra un test qui vérifie le curseur sur des segments de tailles inégales.
- **À surveiller** :
  - **la mesure multi-segments** : si le clairsemé perd du terrain à mesure que le nombre de segments croît — par exemple au-delà de quelques pour cent sur une copie à 8 segments —, le curseur de segment devient la mise en œuvre à faire, pas une option ;
  - **le nombre de segments réels de Legifacile** au moment de `DEFINIR-INDEX-LEGIFACILE` : les ~9 segments de `docs/02:179` sont une déduction de la règle de compaction, pas un compte sur les données ;
  - **un point de densité ou de taille où le dense repasserait devant** dans une campagne à ordre alterné : aucun n'existe aujourd'hui, et un seul suffirait à rouvrir la question ;
  - **le budget de `docs/03-recherche.md:143`** : il est tenu par le clairsemé, mais si un index multi-segments ou un pool de 1 M en conteneur le dépassait, la mesure de P8 le dirait avant la conclusion de cet ADR.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| (a) Garder le seuil d'ADR-0071 jusqu'à la mesure multi-segments | Risque nul en décision, mais conserve un coût mesuré de 5 à 36 % du scan sur les pools contigus, et deux parcours à maintenir pour un gain nul sur 18 points mesurés |
| (b) Clairsemé au-dessus de 100 000 passages, dense en dessous | 100 000 est la **plus petite taille mesurée** : ce serait une règle sans aucun chiffre dans la zone qu'elle gouverne, pour préserver 0,3 à 2,0 % sur des scans de 0,05 à 1 ms, tous dans la dispersion |
| (c) Règle de densité avec un autre point de bascule | Aucun point mesuré ne donne le dense gagnant : déplacer la bascule revient à choisir une densité sans observation, et garde deux parcours |
| Un seul parcours, dense partout | Contredit la mesure sur 17 points sur 18, dont −53,1 % à densité 0,10 ; le dense paie un test de bitmap sur chaque iid de la plage, morts compris |
| Le curseur de segment **dès maintenant** | Optimise un coût que la mesure mono-segment ne voit pas et dont l'ampleur multi-segments est inconnue ; il se justifie après la mesure, pas avant |
| Un seuil configurable entre les deux parcours | Configuration pour une valeur fixe (règle 12 d'`AGENTS.md`), et aucun besoin exprimé ne demande de régler ce ratio |
| Réutiliser `exact_threshold` comme seuil de parcours | Il sépare EXACT de BINARY (`docs/03:45-46`), pas deux parcours du même scan (ADR-0071, déjà écarté) |

## Documents à modifier après acceptation

Rien n'est appliqué ici : cet ADR est `Proposé` et le propriétaire seul tranche. Les modifications ci-dessous sont **listées**, pas faites.

| Document | Modification prévue |
|---|---|
| `docs/adr/0071-seuil-entre-parcours-dense-et-clairseme-du-scan-binaire.md` | passer son statut en `Remplacé par ADR-0081` ; son corps n'est pas réécrit |
| `docs/phases/P4-recherche-rapide.md` (`SCANNER-POOL-BINAIRE`, ligne 48) | remplacer les deux parcours du livrable par le seul parcours clairsemé |
| `docs/03-recherche.md` | une phrase sur le parcours unique du scan binaire, à côté de la description du scan |
| `docs/adr/README.md` | ligne d'index de cet ADR en `Accepté`, et statut d'ADR-0071 aligné |
| `docs/adr/0077-protocole-de-mesure-de-la-latence-de-la-recherche-dense.md` | renvoi dans « À surveiller » vers la mesure multi-segments ; son corps n'est pas réécrit |
| `docs/adr/0079` et `docs/adr/0080` | renvoi dans « À surveiller » vers l'ordre par iid que le scan rend désormais ; leurs corps ne sont pas réécrits |

`docs/suivi.md`, le code et les tests ne sont **pas** touchés par cet ADR : la mise en œuvre demande une tâche, et la mesure multi-segments précède le code.

## Références

- `docs/mesures/MES-02-parcours-dense-clairseme.md` — campagne du 2026-10-27 (commit `17fc786`) : passes alternées, densités 0,10 à 0,90, plages de 1 M et 7,74 M iid, contrôle `agree`
- `docs/adr/0071-seuil-entre-parcours-dense-et-clairseme-du-scan-binaire.md` — le seuil retiré, et sa section « À surveiller », qui a commandé la mesure
- `docs/adr/0077-protocole-de-mesure-de-la-latence-de-la-recherche-dense.md` — protocole de mesure (N, passes, percentiles) ; « À surveiller » sur le parcours clairsemé
- `docs/adr/0079-cout-des-cles-au-rescoring.md`, décision 2 — parcours des candidats dans l'ordre croissant des iid
- `docs/adr/0080-acces-aux-vecteurs-fp16-au-rescoring.md`, option 3 — préchargement `MADV_WILLNEED`, qui suppose le même ordre
- `docs/adr/0069-reconstruction-et-format-de-codes-bin.md`, décision 3 — codes en RAM, en lecture seule : le scan ne dépend pas du stockage
- `docs/adr/0006-segments-pierres-tombales-compaction.md` — un segment couvre une plage contiguë d'iid
- `docs/02-architecture.md:50` — plage contiguë d'iid par segment ; `:179` — segments d'au plus 1 M passages à la compaction
- `docs/03-recherche.md:54` — rangement physique selon `cluster_by` ; `:143` — budget du scan binaire, renvoyant à MES-02
- `docs/phases/P4-recherche-rapide.md:48` — livrable actuel de `SCANNER-POOL-BINAIRE`, qui demande les deux parcours
- `crates/crible-search/src/dense/binary.rs` — `top_k`, `traversal`, `dense_window`, `sparse_window`, `segment_of`, test `dense_and_sparse_traversals_agree_with_a_full_sort`
- `AGENTS.md`, règle 12 (pas d'abstraction ni de configuration pour une valeur fixe)
