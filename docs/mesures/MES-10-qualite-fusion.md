# MES-10 — quelle fusion par défaut, et à quel prix en qualité ?

## Campagne du 2026-10-28

- Tâche : `MESURER-HYBRIDE`
- Commit : `b9e0ea6` (le harnais et l'agrégateur de cette campagne ne sont pas encore commités ; le message proposé est en fin de compte rendu)
- Machine : MacBook Air M3, 8 cœurs, 24 Go, macOS 15.7.7, natif (pas de Docker)
- Jeu de données : `data/mes10/`, 1,8 Go, copie de l'échantillon Legifacile (257 626 passages, six structures) plus 55 documents attendus absents (445 passages), soit **258 071 passages** mesurés ; extrait de production en lecture seule (ADR-0032), `data/mes10/extraction.json` (SHA-256 `4aa4a1721c5047412ac0586a6c05b4cf39ddf54f09924384f45f2394e2b7eadb`)
- Vecteurs de requête : `data/mes10/query-vectors-2026-10-28.json`, fp32, 309 textes, 0 tronqué, SHA-256 `0a0348e670172fe1b800e53fec9af557434f7c4b507c64dc50510c38100ea482`
- Configuration : défauts de production sur les copies de mesure, `scan: binary`, `rescore_candidates` = 5000, `exact_threshold` = 5000, `k` = 10, `group_by: document`, `passages_per_document` = 1, profondeur de branche `clamp(k × 20, 200, 1000)` = **200**, `rrf` à 60, `lexical.positions` = `false`, α ∈ {0 ; 0,3 ; 0,5 ; 0,75 ; 1}. Colonne secondaire dense **EXACT** : clone de l'index avec `exact_threshold` = 4294967295. Ces quatre réglages (`k`, `grouping`, `passages_per_document`, `candidates_per_branch`) sont lus dans chacun des douze documents bruts, qui doivent s'accorder.

### Protocole

Le protocole est celui d'ADR-0092. Les commandes sont celles de `tools/bench/mes10/run.sh`, exécutées depuis la racine du dépôt : `extract`, `prepare`, `encode`, `eval`, `analyze`, `aggregate`. Les étapes utiles à cette mesure sont :

```sh
sh tools/bench/mes10/run.sh extract    # 55 documents attendus, production en lecture seule
sh tools/bench/mes10/run.sh prepare    # copies des lots, six index, six clones EXACT
sh tools/bench/mes10/run.sh encode     # 309 textes en fp32, vecteurs archivés
sh tools/bench/mes10/run.sh eval       # six campagnes binaires, puis six colonnes EXACT
```

`prepare` copie les lots de l'échantillon par clone APFS (`cp -cR`), copie les configurations puis les passe à 5000/5000 **sur les copies**, crée chaque index, attache le lot de l'échantillon (génération 1), puis le lot enrichi (génération 2) ; il clone enfin l'index et porte `exact_threshold` au maximum dans le clone. `data/legifacile-sample/` n'est jamais modifié.

Le harnais est `crates/crible/examples/hybrid_eval.rs` (mode `eval`). Il appelle `pipeline::search` avec la même configuration que le CLI ; aucune mesure ne passe par du code de production modifié.

**Répétitions.** Les métriques de qualité sont déterministes : une seule passe suffit, et une répétition rendrait le même document. Il n'y a donc ni préchauffage ni répétition pour le recall et le MRR ; le cache est chaud après le premier accès aux segments, et la mesure ne rapporte aucune latence. Le bootstrap, lui, est répété 10 000 fois avec la graine `20261028` (ADR-0092, décision 6).

**Métriques.** Recall@10 au document : moyenne, sur les 56 questions satisfiables, de |attendus ∩ top-10| / |attendus|. MRR@10 : moyenne de 1/rang du premier attendu, 0 s'il est absent du top 10. Rang médian des questions trouvées en ligne secondaire. `Q30` est comptée à part et n'entre dans aucune moyenne. La différence rapportée est **méthode − `rrf`**, dans la convention d'ADR-0092, décision 6 : une valeur positive signifie que la méthode fait mieux que `rrf`, et une recommandation exige une borne basse supérieure à zéro.

**Mise en forme.** Les tableaux publiés reprennent les valeurs de `tools/bench/mes10/run.sh aggregate`, avec une mise en forme française (virgule décimale, signes, espaces de milliers) ; aucune valeur n'est changée.

### Résultats

Campagne principale, 56 questions satisfiables, recall@10 au document :

| Mode | recall@10 | MRR@10 | rang médian | trouvées | écart à `rrf` | IC 95 % |
|---|---|---|---|---|---|---|
| dense BINARY | 0,8571 | 0,6839 | 1,0 | 48 | -0,0536 | [-0,1250 ; +0,0000] |
| dense EXACT | 0,8571 | 0,6839 | 1,0 | 48 | -0,0536 | [-0,1250 ; +0,0000] |
| lexical | 0,9107 | 0,7699 | 1 | 51 | 0,0000 | [-0,0536 ; +0,0536] |
| `rrf` | 0,9107 | 0,7645 | 1 | 51 | — | — |
| α = 0 | 0,9107 | 0,7699 | 1 | 51 | 0,0000 | [-0,0536 ; +0,0536] |
| α = 0,3 | 0,9464 | 0,7812 | 1 | 53 | +0,0357 | [+0,0000 ; +0,0893] |
| α = 0,5 | 0,9107 | 0,7795 | 1 | 51 | 0,0000 | [+0,0000 ; +0,0000] |
| α = 0,75 | 0,8929 | 0,7420 | 1,0 | 50 | -0,0179 | [-0,0536 ; +0,0000] |
| α = 1 | 0,8571 | 0,6839 | 1,0 | 48 | -0,0536 | [-0,1250 ; +0,0000] |

Par corpus, avec leur `n` :

| Corpus | n | Mode | recall@10 | MRR@10 |
|---|---|---|---|---|
| dole | 2 | dense BINARY | 1,0000 | 0,6250 |
| dole | 2 | dense EXACT | 1,0000 | 0,6250 |
| dole | 2 | lexical | 1,0000 | 1,0000 |
| dole | 2 | `rrf` | 1,0000 | 0,7500 |
| dole | 2 | α = 0 | 1,0000 | 1,0000 |
| dole | 2 | α = 0,3 | 1,0000 | 0,7500 |
| dole | 2 | α = 0,5 | 1,0000 | 0,7500 |
| dole | 2 | α = 0,75 | 1,0000 | 0,7500 |
| dole | 2 | α = 1 | 1,0000 | 0,6250 |
| jorf_normatif | 4 | dense BINARY | 1,0000 | 0,8000 |
| jorf_normatif | 4 | dense EXACT | 1,0000 | 0,8000 |
| jorf_normatif | 4 | lexical | 0,7500 | 0,5833 |
| jorf_normatif | 4 | `rrf` | 1,0000 | 0,8125 |
| jorf_normatif | 4 | α = 0 | 0,7500 | 0,5833 |
| jorf_normatif | 4 | α = 0,3 | 1,0000 | 0,8000 |
| jorf_normatif | 4 | α = 0,5 | 1,0000 | 0,8333 |
| jorf_normatif | 4 | α = 0,75 | 1,0000 | 0,8000 |
| jorf_normatif | 4 | α = 1 | 1,0000 | 0,8000 |
| juri_sommaires | 4 | dense BINARY | 0,7500 | 0,5312 |
| juri_sommaires | 4 | dense EXACT | 0,7500 | 0,5312 |
| juri_sommaires | 4 | lexical | 0,7500 | 0,6250 |
| juri_sommaires | 4 | `rrf` | 0,7500 | 0,5833 |
| juri_sommaires | 4 | α = 0 | 0,7500 | 0,6250 |
| juri_sommaires | 4 | α = 0,3 | 0,7500 | 0,5625 |
| juri_sommaires | 4 | α = 0,5 | 0,7500 | 0,5500 |
| juri_sommaires | 4 | α = 0,75 | 0,7500 | 0,5500 |
| juri_sommaires | 4 | α = 1 | 0,7500 | 0,5312 |
| juri_texte_integral | 31 | dense BINARY | 0,8387 | 0,6525 |
| juri_texte_integral | 31 | dense EXACT | 0,8387 | 0,6525 |
| juri_texte_integral | 31 | lexical | 0,9677 | 0,8478 |
| juri_texte_integral | 31 | `rrf` | 0,9355 | 0,7815 |
| juri_texte_integral | 31 | α = 0 | 0,9677 | 0,8478 |
| juri_texte_integral | 31 | α = 0,3 | 1,0000 | 0,8323 |
| juri_texte_integral | 31 | α = 0,5 | 0,9355 | 0,8102 |
| juri_texte_integral | 31 | α = 0,75 | 0,9032 | 0,7306 |
| juri_texte_integral | 31 | α = 1 | 0,8387 | 0,6525 |
| sarde | 3 | dense BINARY | 1,0000 | 1,0000 |
| sarde | 3 | dense EXACT | 1,0000 | 1,0000 |
| sarde | 3 | lexical | 1,0000 | 0,8333 |
| sarde | 3 | `rrf` | 1,0000 | 1,0000 |
| sarde | 3 | α = 0 | 1,0000 | 0,8333 |
| sarde | 3 | α = 0,3 | 1,0000 | 0,8333 |
| sarde | 3 | α = 0,5 | 1,0000 | 0,8333 |
| sarde | 3 | α = 0,75 | 1,0000 | 1,0000 |
| sarde | 3 | α = 1 | 1,0000 | 1,0000 |
| textes_vigueur | 12 | dense BINARY | 0,8333 | 0,7083 |
| textes_vigueur | 12 | dense EXACT | 0,8333 | 0,7083 |
| textes_vigueur | 12 | lexical | 0,8333 | 0,6250 |
| textes_vigueur | 12 | `rrf` | 0,8333 | 0,7083 |
| textes_vigueur | 12 | α = 0 | 0,8333 | 0,6250 |
| textes_vigueur | 12 | α = 0,3 | 0,8333 | 0,7083 |
| textes_vigueur | 12 | α = 0,5 | 0,8333 | 0,7500 |
| textes_vigueur | 12 | α = 0,75 | 0,8333 | 0,7500 |
| textes_vigueur | 12 | α = 1 | 0,8333 | 0,7083 |

Aucune moyenne inter-corpus n'est calculée : les `n` vont de 2 à 31 et ne se prêtent pas à une moyenne unique.

Jeu secondaire, **145 requêtes** à `expected_in_sample: true` (115 `decisions`, 8 `jorf`, 8 `dole`, 6 `sarde`, 8 `titres`, aucune `articles`). `Q28` et `Q29` sont labellisées **et** à `expected_in_sample: true` : elles comptent dans le jeu principal et dans le jeu secondaire, conformément à la décision 1 d'ADR-0092, qui parle des 145 requêtes :

| Mode | recall@10 | MRR@10 | rang médian | trouvées | écart à `rrf` | IC 95 % |
|---|---|---|---|---|---|---|
| dense BINARY | 0,7793 | 0,6463 | 1 | 113 | -0,0621 | [-0,1103 ; -0,0207] |
| dense EXACT | 0,7793 | 0,6463 | 1 | 113 | -0,0621 | [-0,1103 ; -0,0207] |
| lexical | 0,8621 | 0,7652 | 1 | 125 | +0,0207 | [-0,0138 ; +0,0552] |
| `rrf` | 0,8414 | 0,7149 | 1,0 | 122 | — | — |
| α = 0 | 0,8621 | 0,7652 | 1 | 125 | +0,0207 | [-0,0138 ; +0,0552] |
| α = 0,3 | 0,8690 | 0,7605 | 1,0 | 126 | +0,0276 | [+0,0000 ; +0,0621] |
| α = 0,5 | 0,8690 | 0,7460 | 1,0 | 126 | +0,0276 | [+0,0000 ; +0,0621] |
| α = 0,75 | 0,8138 | 0,6916 | 1,0 | 118 | -0,0276 | [-0,0552 ; -0,0069] |
| α = 1 | 0,7793 | 0,6463 | 1 | 113 | -0,0621 | [-0,1103 ; -0,0207] |

Cas « article 1240 », rang du document `LEGIARTI000032041571` sur l'index `articles` :

| Variante | Texte | dense | lexical | `rrf` | α = 0 | α = 0,3 | α = 0,5 | α = 0,75 | α = 1 |
|---|---|---|---|---|---|---|---|---|---|
| V1 | l'article 1240 du code civil | 1 | 1 | 1 | 1 | 1 | 1 | 1 | 1 |
| V2 | article 1240 du code civil | 1 | 1 | 1 | 1 | 1 | 1 | 1 | 1 |
| V3 | article 1240 | 1 | 5 | 2 | 5 | 2 | 1 | 1 | 1 |
| V4 | art. 1240 code civil | 1 | 1 | 1 | 1 | 1 | 1 | 1 | 1 |
| V5 | Code civil, article 1240 | 1 | 1 | 1 | 1 | 1 | 1 | 1 | 1 |

**Contrôles de cohérence.** α = 0 reproduit le lexical seul sur **56/56** questions ; α = 1 reproduit le dense seul sur **56/56** questions. Aucun écart : la mesure est cohérente avec ADR-0090.

**Colonne EXACT.** Le dense EXACT donne exactement les mêmes chiffres que le dense BINARY, question par question sur les 56 : l'écart à `rrf` est identique (-0,0536 [-0,1250 ; +0,0000]). Sur des pools de 50 000 passages, le scan binaire ne coûte donc rien de mesurable en qualité ; aucun écart de fusion ne peut lui être attribué dans cette campagne.

**Question non satisfiable.** `Q30` est hors de toute moyenne. Son attendu `LEGIARTI000029370748` est une version `ABROGE_DIFF`, hors du périmètre `textes_vigueur` ; il est présent comme distracteur parce que l'extraction a pris les 55 attendus absents, y compris celui-là. Il sort au rang 1 en dense comme en lexical, ce que la mesure ne compte pas : la question reste non satisfiable par construction.

**Règle de décision.** Aucune configuration n'a de borne basse au-dessus de zéro contre `rrf` :

| Mode | borne basse | borne haute | Motif |
|---|---|---|---|
| dense BINARY | -0,1250 | +0,0000 | borne basse non supérieure à zéro |
| dense EXACT | -0,1250 | +0,0000 | borne basse non supérieure à zéro |
| lexical | -0,0536 | +0,0536 | borne basse non supérieure à zéro |
| α = 0 | -0,0536 | +0,0536 | borne basse non supérieure à zéro |
| α = 0,3 | +0,0000 | +0,0893 | borne basse non supérieure à zéro |
| α = 0,5 | +0,0000 | +0,0000 | borne basse non supérieure à zéro |
| α = 0,75 | -0,0536 | +0,0000 | borne basse non supérieure à zéro |
| α = 1 | -0,1250 | +0,0000 | borne basse non supérieure à zéro |

Le signe du bootstrap a été corrigé après relecture : la différence est désormais `méthode − rrf`, comme l'exige la décision 6 d'ADR-0092. La conclusion ne change pas, mais elle repose maintenant sur le bon test : α = 0,3 a une borne basse exactement à zéro, et non un intervalle entièrement négatif.

### Conclusion

`rrf` reste la fusion par défaut. Le point d'estimation le plus haut est α = 0,3 (recall@10 0,9464 contre 0,9107 pour `rrf`, soit deux questions de plus sur 56), mais son intervalle va de 0,0000 à +0,0893 : il ne permet pas d'exclure l'égalité, et il n'atteint pas la borne stricte d'ADR-0092. Le jeu secondaire ne le contredit pas (0,8690 contre 0,8414 pour `rrf`, IC [0,0000 ; +0,0621]) et les deux garde-fous sont au niveau du bruit, voire au-dessus (`juri_texte_integral` 1,0000 contre 0,9355, IC [0,0000 ; +0,1613] ; `textes_vigueur` 0,8333, IC [0,0000 ; +0,0000]). La conclusion conforme à ADR-0092 est donc une **égalité statistique** : `rrf` reste le défaut, et c'est une conclusion valide, pas un échec.

Le critère du jalon J5 sur le cas « article 1240 » est satisfait : en `rrf` et pour α ≥ 0,3, les cinq variantes placent l'attendu dans le top 3, la moins bonne étant « article 1240 » au rang 2 en `rrf` et à α = 0,3 (rang 1 à partir de α = 0,5). Le contrôle α = 0, qui reproduit le lexical seul, laisse « article 1240 » au rang 5 pour la variante V3 : c'est un contrôle de cohérence, pas un mode hybride, et il ne compte pas pour le critère. Le dense le place toujours au rang 1 ; le lexical seul le laisse au rang 5 pour « article 1240 », ce qui montre l'apport de la fusion sur une requête courte.

**Puissance.** Avec 56 questions, une question vaut 1/56 ≈ 0,0179 de recall. Un écart sous environ 0,1 (six questions) n'est pas détectable avec cet intervalle ; l'écart observé pour α = 0,3 vaut deux questions et reste dans le bruit. Le rapport ne prétend donc pas classer finement les α : il constate qu'aucun ne se détache.

**Ce que la mesure ne permet pas d'affirmer.** Le jeu enrichi est plus facile que la production : 55 attendus ont été ajoutés et les 8,58 M passages de production sont remplacés par environ 50 000 distracteurs par structure (**7 628 pour `dole`**). Les chiffres ne sont pas comparables à la cible de 0,482 de MES-00 ; la comparaison est renvoyée à `CONSTITUER-JEU-DE-REFERENCE` (P9). Le jeu est provisoire jusqu'à cette tâche. `Q30` reste hors moyenne. La profondeur de branche est celle du plan actuel (200) ; si le plan change, la campagne se rejoue.

**Recommandation, à valider par le propriétaire (Q-02).** Conserver `rrf` comme fusion par défaut, sans valeur d'α par défaut. Si le propriétaire veut un réglage, α = 0,3 est le seul candidat dont le point d'estimation dépasse `rrf` sur le jeu principal et le jeu secondaire, mais la mesure ne permet pas de le recommander : la proposition serait alors un choix de produit, pas un résultat statistique.

### Écarts au protocole, déclarés

1. **Double `PATCH`.** ADR-0092 et ADR-0078 prévoient un double `PATCH` pour porter `rescore_candidates` et `exact_threshold` à 5000. Aucun point d'entrée CLI ou HTTP ne permet de patcher la configuration de recherche (le CLI `crible patch` ne met à jour que des champs de documents ; `IndexPatch` et `apply_patch` sont publics dans `crible_core` mais ne sont atteignables que depuis du code). Les deux valeurs ont donc été écrites à 5000 dans les **copies de mesure** de `configs/*.json` avant `index create`, et `exact_threshold` a été porté au maximum dans le `index.json` du clone EXACT. La configuration obtenue est identique à celle d'un double `PATCH` (5000/5000 relus dans chaque `eval-*.json`) ; le jeu versionné de l'échantillon n'est pas modifié. C'est le précédent `edit_positions` de MES-06, étendu à deux clés.
2. **Jeu secondaire.** Les 145 requêtes à `expected_in_sample: true` incluent `Q28` et `Q29`, labellisées et déjà comptées dans le jeu principal ; elles sont donc dans les deux jeux, comme le veut la décision 1.
3. **Sondes de MES-11.** Les quatre sondes sont mesurées sur l'index `articles`, seul index qui porte leurs attendus ; les mesurer sur les six index répétait la même tokenisation. La stabilité est vérifiée dans l'artefact : 96/96 couples (sonde, variante) identiques entre les six index.
4. **`Q30`.** Son attendu a été extrait avec les 55 autres, donc il est présent comme distracteur. La question reste exclue des moyennes, conformément à la décision 1.

L'artefact daté `docs/mesures/MES-10-qualite-fusion.2026-10-28.json` porte les 12 documents bruts et leur SHA-256, les 10 000 rééchantillonnages, les intervalles, les garde-fous, la décision et les contrôles.
