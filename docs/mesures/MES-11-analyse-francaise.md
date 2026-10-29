# MES-11 — élisions et identifiants français : quel traitement pour l'analyse ?

## Campagne du 2026-10-28

- Tâche : `MESURER-HYBRIDE`
- Commit : `b9e0ea6` (le harnais et l'agrégateur de cette campagne ne sont pas encore commités ; le message proposé est en fin de compte rendu)
- Machine : MacBook Air M3, 8 cœurs, 24 Go, macOS 15.7.7, natif (pas de Docker)
- Jeu de données : `data/mes10/`, 1,8 Go, **258 071 passages** sur six index (257 626 passages de l'échantillon Legifacile plus 445 passages de 55 documents attendus extraits en lecture seule) ; `data/mes10/extraction.json` (SHA-256 `4aa4a1721c5047412ac0586a6c05b4cf39ddf54f09924384f45f2394e2b7eadb`)
- Vecteurs de requête : `data/mes10/query-vectors-2026-10-28.json`, fp32, SHA-256 `0a0348e670172fe1b800e53fec9af557434f7c4b507c64dc50510c38100ea482`
- Configuration : chaîne d'analyse **répliquée hors du moteur**, un index tantivy de mesure par variante et par structure, construit en mémoire depuis les mêmes textes de passages ; requêtes encodées en fp32 avec `mdenseon` ; `k` = 10, `group_by: document` ; `rrf` à 60.

### Protocole

Le protocole est celui d'ADR-0092, décision 5. Les commandes sont celles de `tools/bench/mes10/run.sh` :

```sh
sh tools/bench/mes10/run.sh encode     # 309 textes en fp32, vecteurs archivés
sh tools/bench/mes10/run.sh analyze    # six index, quatre variantes, une passe
sh tools/bench/mes10/run.sh aggregate  # recall, MRR, intervalles, artefact
```

`analyze` construit, pour chaque index, les quatre variantes sur les mêmes textes de passages, puis mesure les questions étiquetées de la structure et les quatre sondes déclarées. Aucun index ni code de production n'est modifié : l'API publique est seulement appelée (`lexical::search` pour l'ancrage, `pipeline::search` pour le classement dense de la série `rrf`). Les index de mesure sont en mémoire et disparaissent avec le processus.

**Questions jugées.** La décision 5 d'ADR-0092 définit la série de MES-11 comme « les questions contenant une élision et celles contenant un identifiant », c'est-à-dire leur **union**. Le harnais compte **47** questions à élision et **0** question à identifiant dans les 56 questions satisfiables ; la série principale est donc ces **47** questions. Les 9 questions satisfiables qui ne portent ni élision ni identifiant sont données en **complément déclaré** (56 questions), pour que le lecteur voie l'effet de leur dilution ; elles ne servent pas à la décision. `Q30` est exclue dans les deux séries.

**Répétitions.** Les métriques sont déterministes : une seule passe, pas de préchauffage, aucune latence rapportée. Le bootstrap emploie 10 000 rééchantillonnages avec la graine `20261028`.

**Mise en forme.** Les tableaux publiés reprennent les valeurs de `tools/bench/mes10/run.sh aggregate`, avec une mise en forme française (virgule décimale, signes, espaces de milliers) ; aucune valeur n'est changée.

**Durée observée du mode `analyze`** (construction des quatre index tantivy et mesure, une passe) : articles 27,1 s ; decisions 37,6 s ; jorf 30,3 s ; sarde 9,7 s ; dole 8,6 s ; titres 4,2 s. Ces durées servent de repère de reproductibilité, pas de mesure de performance.

**Ancrage au moteur.** La réplique V0 doit reproduire le top-10 document du moteur (`lexical::search`) question par question : **57/57** questions reproduites sur les six index. Sans cet ancrage, un écart entre variantes pourrait venir de la réplique et non de l'analyse.

**Variantes.**

- **V0** : chaîne actuelle, `SimpleTokenizer`, `RemoveLongFilter(40)`, `LowerCaser`, `AsciiFoldingFilter`, `StopWordFilter(fr)`, `Stemmer(fr)`.
- **V1** : V0 plus un filtre d'élision qui retire les formes élidées (`l`, `d`, `qu`, `jusqu`, `lorsqu`, `puisqu`, `quoiqu`). La liste de mots vides française contient déjà les lettres simples (`c d j l m n s t qu`) : V1 n'ajoute donc, en pratique, que les formes composées, et le rapport mesure cet effet au lieu de le supposer.
- **V2** : V0 avec les identifiants conservés en un seul jeton. `L. 1132-1` et `L1132-1` donnent `l1132-1` ; `L. 1132-11` donne `l1132-11`, donc reste distinct.
- **V3** : V1 + V2.

**Détection d'un identifiant.** Le harnais reconnaît, à une frontière de mot, une lettre ASCII, un point optionnel, des espaces optionnels, des chiffres, puis des groupes `-chiffres` optionnels ; le jeton normalisé garde la lettre, les chiffres et les tirets (`l1132-1`), et doit dépasser deux caractères (`crates/crible/examples/hybrid_eval.rs`, `identifier_at`). Ce motif est **étroit** : il ne reconnaît ni `article 1240`, ni `1240` seul. Sur les 300 questions de `queries.jsonl`, il donne 0 question ; un motif plus large « article + numéro » en donne 6 et un motif « chiffres-chiffres » en donne 3, toutes **non labellisées**. Les 57 questions labellisées n'en contiennent aucune. Le « 0 » du jeu jugé est donc robuste au motif étroit, et les 12 requêtes à identifiant relevées par `@architecte` avec un motif plus large sont hors du jeu labellisé ; je n'ai pas reproduit ce compte de 12, et le rapport ne s'appuie pas dessus.

### Résultats

Branche lexicale (série principale) et `rrf` (série secondaire), **47 questions** (union élision ∪ identifiant) :

| Variante | recall@10 lexical | MRR@10 lexical | rang médian | recall@10 `rrf` | écart à V0 | IC 95 % |
|---|---|---|---|---|---|---|
| V0 | 0,9149 | 0,7826 | 1 | 0,9149 | — | — |
| V1 | 0,9149 | 0,7826 | 1 | 0,9149 | 0,0000 | [+0,0000 ; +0,0000] |
| V2 | 0,9149 | 0,7826 | 1 | 0,9149 | 0,0000 | [+0,0000 ; +0,0000] |
| V3 | 0,9149 | 0,7826 | 1 | 0,9149 | 0,0000 | [+0,0000 ; +0,0000] |

Complément déclaré, **56 questions satisfiables** (les 9 questions sans élision ni identifiant sont incluses) :

| Variante | recall@10 lexical | MRR@10 lexical | rang médian | recall@10 `rrf` | écart à V0 | IC 95 % |
|---|---|---|---|---|---|---|
| V0 | 0,9107 | 0,7699 | 1 | 0,9107 | — | — |
| V1 | 0,9107 | 0,7699 | 1 | 0,9107 | 0,0000 | [+0,0000 ; +0,0000] |
| V2 | 0,9107 | 0,7699 | 1 | 0,9107 | 0,0000 | [+0,0000 ; +0,0000] |
| V3 | 0,9107 | 0,7699 | 1 | 0,9107 | 0,0000 | [+0,0000 ; +0,0000] |

La série `rrf` donne le même recall pour les quatre variantes, sur la série principale comme sur le complément ; son MRR vaut 0,7619 sur les 47 et 0,7645 sur les 56, pour toutes les variantes. Les quatre variantes classent donc les premiers documents attendus de la même façon : les changements de tokenisation ne déplacent aucun premier attendu dans le top 10.

Par corpus de la série principale, identiques pour les quatre variantes :

| Corpus | n | recall@10 lexical | MRR@10 lexical | recall@10 `rrf` | MRR@10 `rrf` |
|---|---|---|---|---|---|
| dole | 2 | 1,0000 | 1,0000 | 1,0000 | 0,7500 |
| jorf_normatif | 3 | 0,6667 | 0,6667 | 1,0000 | 0,7500 |
| juri_sommaires | 4 | 0,7500 | 0,6250 | 0,7500 | 0,5833 |
| juri_texte_integral | 28 | 0,9643 | 0,8494 | 0,9286 | 0,7760 |
| sarde | 2 | 1,0000 | 0,7500 | 1,0000 | 1,0000 |
| textes_vigueur | 8 | 0,8750 | 0,6250 | 0,8750 | 0,7500 |

Aucune moyenne inter-corpus n'est calculée : les `n` vont de 2 à 28.

Tokenisation des 258 071 passages des six index :

| Variante | Passages | Passages dont les jetons diffèrent de V0 | Jetons |
|---|---|---|---|
| V0 | 258 071 | 0 | 25 774 104 |
| V1 | 258 071 | 15 213 | 25 753 313 |
| V2 | 258 071 | 54 424 | 25 573 270 |
| V3 | 258 071 | 63 195 | 25 552 472 |

V1 change donc 5,9 % des passages, V2 21,1 %, V3 24,5 % — sans effet mesurable sur le recall des 47 questions.

Sondes déclarées, rang du document attendu en lexical (index `articles`, seul index qui porte les attendus). **P3 et P4 n'ont pas d'attendu** : `P3` est le contrôle négatif « L. 1132-11 » et `P4` le contrôle d'élision « qu'il ». `probes_top3` ne repose donc que sur **P1 et P2** :

| Sonde | Texte | Attendu | V0 | V1 | V2 | V3 |
|---|---|---|---|---|---|---|
| P1 | L. 1132-1 | `LEGIARTI000045391841` | — | — | 4 | 4 |
| P2 | L1132-1 | `LEGIARTI000045391841` | 1 | 1 | 4 | 4 |
| P3 | L. 1132-11 | aucun (contrôle négatif) | — | — | — | — |
| P4 | qu'il | aucun (élision) | — | — | — | — |

Sondes dans le top 3 en `rrf` : V0 non, V1 non, V2 oui, V3 oui. P1 sort au rang 5 en `rrf` chez V0/V1 et au rang 1 chez V2/V3 ; P2 sort au rang 1 en `rrf` chez les quatre variantes. P4 ne produit aucun jeton utile dans aucune variante, la liste de mots vides retirant déjà `qu` et `il`.

**Le contrôle négatif P3 contredit une lecture optimiste de V2.** P3 n'a pas d'attendu, donc son `lexical_rank` est nul par construction pour toutes les variantes ; on ne peut pas en conclure qu'« aucune variante ne remonte l'attendu ». Ce qui est mesuré est autre chose : la branche lexicale de V2/V3 ne renvoie **aucun** document pour `l1132-11`, alors que celle de V0/V1 en renvoie 10 (jetons `1132` et `11` séparés) ; la distinction des jetons est donc réelle. Ces listes et leurs longueurs sont portées par l'artefact (`probes[].lexical` et `probes[].lexical_count`), et non seulement par les documents bruts. Mais le top 3 `rrf` de P3 place `LEGIARTI000045391841`, le document de « L. 1132-1 », **au rang 1 chez V2/V3** (rang 7 chez V0/V1). Comme la branche lexicale de V2/V3 est vide sur P3, ce rang 1 vient uniquement du dense, qui ne distingue pas `1132-1` de `1132-11`. Le critère binaire « top 3 en `rrf` » est donc satisfait par V2/V3 sur P1 **et** sur le contrôle négatif P3 : il ne discrimine pas. Le seul élément qui distingue V2 est le rang lexical (P1 au rang 4, P3 absent), et il ne suffit pas à fonder un gain de qualité mesuré.

**Cas « article 1240 » (ADR-0092, décision 5).** Les cinq variantes sont mesurées dans MES-10, section « Cas article 1240 » ; rangs lexicaux de `LEGIARTI000032041571` sur l'index `articles` : V1 1, V2 1, V3 5, V4 1, V5 1. En `rrf`, les cinq sont dans le top 3 (V3 au rang 2), et à partir de α = 0,5 toutes sont au rang 1. Ces variantes ne sont pas rejouées ici : elles portent sur une requête unique et la table de MES-10 les couvre déjà.

**Exemples de différences de tokenisation.** L'artefact en consigne 72, tous effectivement différents de V0, avec une fenêtre de douze jetons autour du premier écart. Deux exemples :

- `LEGIARTI000006738746#0`, V1, premier écart au jeton 62 : V0 `… etre remplac jusqu a expir del depot candidatur`, V1 `… etre remplac a expir del depot candidatur` (la forme `jusqu` tombe) ;
- `LEGIARTI000006807225#0`, V2, premier écart au jeton 23 : V0 `… r233 45 arret ministeriel …`, V2 `… r233-45 arret ministeriel …` (l'identifiant devient un seul jeton, et les positions suivantes se décalent).

Le détail complet, structure par structure, est dans `docs/mesures/MES-11-analyse-francaise.2026-10-28.json`.

### Conclusion

Sur les **47 questions de la série principale**, comme sur le complément à 56, **aucune variante ne se distingue de V0** : le recall@10 et le MRR@10 sont identiques au chiffre près, et l'intervalle de la différence est [0 ; 0] pour V1, V2 et V3. La règle de décision d'ADR-0092 conclut donc à une égalité et laisse V0 en place. C'est cohérent avec le fait qu'aucune question jugée ne porte d'identifiant et que les élisions mesurées ne déplacent aucun premier attendu. Le signe du bootstrap a été corrigé après relecture (différence `variante − V0`, convention d'ADR-0092) ; la conclusion est inchangée.

Le critère des sondes, lui, ne sépare pas les variantes aussi nettement que le premier passage du rapport le disait. **V2 et V3 placent l'attendu de « L. 1132-1 » dans le top 3 en `rrf`**, V0 et V1 non (rang 5) ; mais le même critère place aussi le document de « L. 1132-1 » au rang 1 pour le contrôle négatif P3 chez V2/V3, par le dense seul. Le critère formel est donc satisfait, mais il n'est pas discriminant. Le seul fait solide est lexical : V2/V3 retrouvent P1 au rang 4 et ne renvoient rien pour P3, alors que V0/V1 retrouvent P2 au rang 1 et noient P3 dans 10 résultats. V1, seule, n'apporte rien de mesurable : elle change 15 213 passages et aucun résultat au niveau document, ce qui confirme que la liste de mots vides couvrait déjà les élisions simples.

Deux réserves doivent être lues avec la recommandation. D'une part, V2 fait passer le rang lexical de « L1132-1 » de 1 à 4 : le découpage de V0 (`l1132` + `1`) était plus permissif et remontait l'article en tête, alors que le jeton unique est plus sélectif ; le `rrf` conserve le rang 1 dans les deux cas, donc la fusion absorbe la perte. D'autre part, la réplique n'est pas le moteur : un gain mesuré ici ne vaudrait qu'après un ADR et une tâche qui modifient l'analyseur, et il devra être reconfirmé sur le vrai jeu de référence en P9.

**Recommandation, à valider par le propriétaire (Q-04).** Ne rien changer par défaut sur la seule foi du recall : la mesure ne montre aucun gain. Si le propriétaire veut clore Q-04, **V2, identifiants conservés en un seul jeton**, est la seule variante qui satisfait le critère formel des sondes sans régression mesurable ; mais ce critère ne discrimine pas le contrôle négatif, et l'appui réel est le rang lexical de P1 (4 contre absent) et l'absence de résultat lexical sur P3. C'est donc une proposition plus faible qu'au premier passage : un choix de produit à confirmer par un ADR et une tâche d'analyseur, pas une modification directe de la chaîne de production. V3 ajoute le filtre d'élision, dont l'effet mesuré est nul, et n'apporte rien de plus.

**Puissance et limites.** Avec 47 questions, une question vaut 1/47 ≈ 0,0213 ; un écart sous environ 0,1 n'est pas détectable. L'absence de différence sur V1/V2/V3 est nulle au chiffre près, mais elle ne dit rien de l'effet sur d'autres requêtes, notamment celles qui portent un identifiant. Les sondes compensent partiellement ce manque, avec un critère binaire (top 3) et non une moyenne, et ce critère est affaibli par P3. Le jeu enrichi est plus facile que la production et le jeu est provisoire jusqu'à P9. Enfin, V1 ne traite que les formes composées en minuscules ou majuscules ; les formes accentuées ou les élisions non listées ne sont pas couvertes, et le rapport ne prétend pas mesurer leur effet.

### Écarts au protocole, déclarés

1. **Série jugée à 47, complément à 56.** La décision 5 définit la série comme l'union élision ∪ identifiant, soit 47 questions. Le premier passage moyennait sur les 56 satisfiables et présentait ce chiffre comme le protocole ; c'est corrigé, et le complément à 56 est donné à côté, déclaré, sans servir à la décision.
2. **Sondes sur un seul index.** Les quatre sondes sont mesurées sur l'index `articles`, seul index qui porte leurs attendus ; les mesurer sur les six index répétait la même tokenisation. La stabilité est vérifiée dans l'artefact : 96/96 couples (sonde, variante) identiques entre les six index.
3. **P3 et P4 sans attendu.** `probes_top3` ne repose que sur P1 et P2 ; le rapport signale le rang 1 de P3 en `rrf` chez V2/V3 et son origine dense. Aucun attendu n'a été ajouté par une nouvelle extraction.
4. **Détection étroite des identifiants.** Le motif du harnais (lettre, point et espaces optionnels, chiffres, groupes `-chiffres`) ne reconnaît pas `article 1240` ni `1240` seul ; les comptes élargis sont donnés en protocole et ne changent pas le 0 du jeu labellisé.
5. **Variantes du cas 1240.** Elles sont citées depuis la table de MES-10 et non rejouées ici.
6. **Série `rrf` répliquée.** La branche dense vient du moteur (`pipeline::search`), la branche lexicale de la réplique ; la fusion `rrf` est recalculée par le harnais avec les mêmes paramètres. V2/V3 emploient un tokenizer et un filtre d'identifiants propres à la réplique, qui ne sont pas ceux du moteur.
7. **Liste d'élisions unifiée.** La détection des questions et le filtre V1/V3 dérivent désormais des mêmes constantes ; `quoiqu'` est couvert. Aucune question du jeu ne porte `quoiqu'`, et `qu` était déjà retiré par la liste de mots vides : les comptes de jetons et la série à 47 sont inchangés, et aucune campagne n'a été relancée.

L'artefact daté `docs/mesures/MES-11-analyse-francaise.2026-10-28.json` porte les six documents bruts et leur SHA-256, l'ancrage 57/57, les statistiques de jetons, les 72 exemples de différences, les sondes avec leurs listes lexicales, leurs longueurs et leurs attendus manquants, les séries à 47 et à 56, les intervalles et la décision.
