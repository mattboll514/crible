# ADR-0093 — Fusion par défaut : `rrf` confirmé par MES-10 (Q-02)

- **Statut** : Accepté (décision du propriétaire le 2026-10-29)
- **Date** : 2026-10-29 (proposition et acceptation)
- **Décideurs** : propriétaire
- **Décision** : le propriétaire retient la recommandation de MES-10, telle quelle : `rrf` reste la fusion par défaut, aucune valeur d'α par défaut n'est fixée, et α = 0,3 n'est pas retenu.
- **Proposé par** : agent greffier, sur demande du propriétaire, au point de fin de la série MESURER-LEXICAL et MESURER-HYBRIDE (phase P5), à partir du rapport MES-10 et de la recommandation de `@mesureur`
- **Remplace** : — (précise ADR-0008, ligne 18, et ADR-0090, qui ne fixait aucun défaut, sans réécrire leur corps)

## Contexte

### La question ouverte

- **Q-02** demande « Fusion par défaut (`rrf` ou `relative_score`, et α) », à trancher par MES-10 en P5 (`docs/adr/README.md`, section « Questions ouvertes »). Son protocole est fixé par ADR-0092, qui la laisse ouverte : « ni Q-02 ni Q-04 ne sont closes par le présent ADR » (`docs/adr/0092-protocole-de-mes-10-et-mes-11-qualite-de-la-fusion-et-analyse-francaise.md`, décision 6).
- **ADR-0008** fixe un défaut provisoire : « La méthode par défaut se règle par index, et chaque requête peut la changer. En attendant MES-10 (Q-02), le défaut est `rrf` » (`docs/adr/0008-hybride-sur-le-meme-pool.md:18`). Sa section « À surveiller » demande « MES-10 (qualité par méthode et par α) » (`:25`).
- **ADR-0090** borne α à `[0, 1]` et **ne fixe aucune valeur par défaut** : « aucune valeur par défaut ni changement du défaut `rrf` » (`docs/adr/0090-fusion-relative-score-branche-constante-et-domaine-de-alpha.md`).
- **ADR-0092, décision 6** fixe la règle de décision : la métrique principale est le recall@10 au document, la différence est calculée contre `rrf`, et une autre configuration n'est recommandée que si la **borne basse** de l'intervalle de confiance à 95 % **dépasse 0**, sans régression sur le jeu secondaire ni perte au-delà du bruit sur `juri_texte_integral` et `textes_vigueur`. En cas d'égalité ou de résultat non concluant, `rrf` reste le défaut, et c'est une conclusion valide.

### Ce que la mesure dit

Rapport `docs/mesures/MES-10-qualite-fusion.md`, campagne du 2026-10-28 sur `data/mes10/` (258 071 passages, jeu enrichi provisoire), `k` = 10, `group_by: document`, profondeur de branche 200.

- **Jeu principal, 56 questions satisfiables.** `rrf` donne un recall@10 de 0,9107 et un MRR@10 de 0,7645. α = 0,3 donne 0,9464 et 0,7812, soit +0,0357 de recall, avec un intervalle de confiance de l'écart à `rrf` de **[+0,0000 ; +0,0893]** : la borne basse n'est pas supérieure à 0.
- **Jeu secondaire, 145 requêtes.** `rrf` donne 0,8414 et α = 0,3 donne 0,8690, avec un intervalle de [+0,0000 ; +0,0621]. L'écart reste non concluant.
- **Puissance.** Sur 56 questions, une question vaut 1/56 ≈ 0,0179 de recall, et l'écart observé pour α = 0,3 vaut deux questions. Il reste dans le bruit.
- **Aucune autre configuration ne se détache.** Dense BINARY et dense EXACT sont à 0,8571 (intervalle de l'écart [−0,1250 ; +0,0000]), le lexical seul à 0,9107, α = 0,5 à 0,9107 et α = 0,75 à 0,8929.
- **Contrôles de cohérence.** α = 0 reproduit le lexical seul sur 56/56 questions, α = 1 le dense seul sur 56/56, ce qui est conforme à ADR-0090.

## Décision

**`rrf` reste la fusion par défaut. Aucune valeur d'α par défaut n'est fixée, et α = 0,3 n'est pas retenu.**

Précisions :

1. **Le défaut ne change pas.** `search.fusion` continue de valoir `rrf` par défaut à la création d'un index, comme `docs/03-recherche.md:106` l'écrit déjà. ADR-0008 n'est pas modifié : sa ligne 18 cesse simplement d'être provisoire, et le présent ADR la confirme.
2. **Aucun α par défaut.** Le domaine d'ADR-0090 reste `alpha ∈ [0, 1]`, valeur obligatoire dès que `relative_score` est choisi. Un producteur ou une requête qui veut `relative_score` doit nommer α lui-même. Ni la configuration de gabarit, ni un défaut de code, ne porte de valeur.
3. **α = 0,3 n'est pas retenu.** Son point d'estimation est le plus haut du jeu principal et du jeu secondaire, mais la mesure ne permet pas d'exclure l'égalité. Le retenir serait un choix de produit, pas un résultat statistique ; le propriétaire ne le fait pas.
4. **Ce qui n'est pas décidé ici.** Aucune méthode n'est retirée : `relative_score` reste offerte par index et par requête (ADR-0008). Aucune valeur de `candidates_per_branch`, de `k` ni de `group_by` n'est revue. Le cas « article 1240 » n'est pas rouvert.
5. **La question se rouvre en P9.** Le jeu enrichi est provisoire : 55 attendus ont été ajoutés à l'échantillon, et les 8,58 M passages de production sont remplacés par environ 50 000 distracteurs par structure. Les chiffres ne sont pas comparables à la cible de 0,482 de MES-00. Q-02 se rejoue sur le vrai jeu de référence, dans `CONSTITUER-JEU-DE-REFERENCE` puis `COMPARER-AVEC-PGVECTOR` (P9).

## Conséquences

- **Positives** :
  - le défaut est celui qui n'a rien à régler : aucun α à choisir pour le producteur, et deux index ne peuvent pas diverger par un réglage implicite ;
  - la décision suit la règle fixée avant la mesure (ADR-0092, décision 6), donc elle n'est pas ajustée aux chiffres observés ;
  - Q-02 est close, et `docs/03-recherche.md:106` porte désormais la référence à la mesure.
- **Négatives / coûts** :
  - α = 0,3 est perdu comme réglage recommandé, alors que c'est le seul point d'estimation qui dépasse `rrf` sur les deux jeux. Un utilisateur qui veut l'essayer doit le nommer lui-même ;
  - la conclusion repose sur 56 questions, où une question vaut 0,018 : elle ne classe pas finement les α, elle constate seulement qu'aucun ne se détache ;
  - le coût de la fusion (`rrf` a besoin des deux rangs, `relative_score` des deux scores normalisés) reste inchangé : la décision ne porte pas sur la latence.
- **À surveiller** :
  - **le vrai jeu de référence en P9** (`CONSTITUER-JEU-DE-REFERENCE`, puis `COMPARER-AVEC-PGVECTOR`) : sur 8,58 M passages et des requêtes de production, un écart pourrait devenir détectable. Un résultat concluant rouvrirait un ADR qui remplacerait celui-ci ;
  - **le lexical seul égale `rrf` en recall@10 (0,9107) et dépasse le dense (0,8571) sur ce jeu enrichi.** Ce constat n'est pas une décision : il vient d'un jeu plus facile que la production. S'il se confirme en P9, il posera la question de la place du dense et du mode par défaut (`hybrid` ou `lexical`), qui demanderait son propre ADR ;
  - **la profondeur de branche.** La campagne a tourné avec la règle actuelle (200 candidats par branche). Si le plan change, la comparaison se rejoue.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Fixer α = 0,3 comme défaut de `relative_score` | La borne basse de l'intervalle de l'écart à `rrf` vaut 0,0000, donc la règle d'ADR-0092, décision 6, n'est pas satisfaite. Ce serait un choix de produit que la mesure ne justifie pas. |
| Fixer α = 0,5 comme défaut, par prudence | Rappel identique à `rrf` (0,9107) et MRR supérieur (0,7795 contre 0,7645), mais l'intervalle de l'écart est [+0,0000 ; +0,0000] : aucun gain de rappel, et α serait choisi pour une différence de MRR que la règle de décision ne place qu'en départage. |
| Passer le défaut à `relative_score` sans α par défaut | α serait obligatoire à chaque création d'index et à chaque requête, ce qui rend le défaut inutilisable sans réglage : contraire au sens d'un défaut. |
| Choisir `lexical` comme mode par défaut | Le constat du jeu enrichi ne vaut que pour un jeu provisoire et plus facile que la production, et le mode par défaut relève d'une autre question que Q-02 : le dense reste le socle de la v1 (ADR-0001, ADR-0008). |
| Attendre P9 avant de trancher Q-02 | La mesure de P5 a un protocole accepté et une règle de décision écrite ; attendre laisserait le défaut provisoire d'ADR-0008 en vigueur sans trace d'un examen, alors que la décision de P9 se rejouera de toute façon sur un autre jeu. |

## Références

- Q-02, `docs/adr/README.md` (questions ouvertes, puis questions tranchées).
- ADR-0008 (`:18`, défaut `rrf` en attendant MES-10 ; `:25`, « À surveiller »), ADR-0090 (domaine de α, aucun défaut), ADR-0092 décision 6 (règle de décision).
- `docs/mesures/MES-10-qualite-fusion.md` et son artefact daté `docs/mesures/MES-10-qualite-fusion.2026-10-28.json`.
- `docs/03-recherche.md:106` (défaut par index).
- Tâches `MESURER-HYBRIDE` (P5), `CONSTITUER-JEU-DE-REFERENCE` et `COMPARER-AVEC-PGVECTOR` (P9).
