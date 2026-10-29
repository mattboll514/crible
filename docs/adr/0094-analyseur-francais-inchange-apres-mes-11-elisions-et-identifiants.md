# ADR-0094 — Analyseur français inchangé après MES-11 : élisions et identifiants (Q-04)

- **Statut** : Accepté (décision du propriétaire le 2026-10-29)
- **Date** : 2026-10-29 (proposition et acceptation)
- **Décideurs** : propriétaire
- **Décision** : le propriétaire retient la recommandation de MES-11, dans sa forme prudente : la chaîne d'analyse d'ADR-0086 reste inchangée, aucun filtre d'élision ni traitement d'identifiant n'est ajouté, et V2 (identifiants en un seul jeton) reste une option de produit non retenue.
- **Proposé par** : agent greffier, sur demande du propriétaire, au point de fin de la série MESURER-LEXICAL et MESURER-HYBRIDE (phase P5), à partir du rapport MES-11 et de la recommandation de `@mesureur`
- **Remplace** : — (précise ADR-0086, ligne 68, qui laissait les élisions et les identifiants non tranchés, sans réécrire son corps)

## Contexte

### La question ouverte

- **Q-04** demande la « Gestion des élisions et des identifiants dans l'analyseur français », à trancher par MES-11 en P5 (`docs/adr/README.md`, section « Questions ouvertes »). Son protocole est fixé par ADR-0092, qui la laisse ouverte (`docs/adr/0092-protocole-de-mes-10-et-mes-11-qualite-de-la-fusion-et-analyse-francaise.md`, décision 6).
- **`docs/03-recherche.md:87`** écrivait : « Le traitement des élisions (l', d', qu') et des identifiants comme `L1132-1` est à mesurer (MES-11). »
- **ADR-0086** fixe la chaîne actuelle pour le français : `SimpleTokenizer`, `RemoveLongFilter(40)`, `LowerCaser`, `AsciiFoldingFilter`, `StopWordFilter(fr)`, `Stemmer(fr)` (`docs/03-recherche.md:84-85`). Sa ligne 68 écrit explicitement ce qu'il ne décide pas : « le traitement des élisions et des identifiants (`L1132-1`, `1240`), qui reste ouvert (Q-04, MES-11) » (`docs/adr/0086-domaine-de-lexical-language-et-chaine-sans-liste-de-mots-vides.md:68`).
- **ADR-0092, décision 5** fixe les variantes mesurées et l'instrument : chaîne répliquée dans le harnais, **V0** la chaîne actuelle, **V1** V0 plus un filtre d'élision, **V2** V0 avec les identifiants en un seul jeton, **V3** = V1 + V2. La série jugée est l'union des questions à élision et des questions à identifiant, avec des sondes déclarées. Un résultat positif ne justifie qu'un ADR puis une tâche : rien n'est codé dans la campagne.
- **ADR-0092, décision 6** applique à Q-04 la même règle de décision qu'à Q-02, avec en plus un **critère explicite sur les sondes** : l'attendu doit être dans le **top 3**.

### Ce que la mesure dit

Rapport `docs/mesures/MES-11-analyse-francaise.md`, campagne du 2026-10-28 sur `data/mes10/` (258 071 passages, six index), même jeu provisoire que MES-10.

- **Série jugée : 47 questions à élision, aucune question étiquetée à identifiant.** Les 57 questions labellisées n'en contiennent aucune, quel que soit le motif de détection retenu par le harnais.
- **V0 à V3 sont à égalité stricte.** Sur les 47 questions : recall@10 de 0,9149 et MRR@10 de 0,7826 pour les quatre variantes, écart à V0 [0 ; 0]. Le complément à 56 questions donne la même égalité, à 0,9107 et 0,7699.
- **La tokenisation change pourtant.** V1 diffère de V0 sur 5,9 % des passages, V2 sur 21,1 % et V3 sur 24,5 %, sans déplacer aucun premier attendu dans le top 10.
- **Sondes.** « L. 1132-1 » entre dans le top 3 en `rrf` seulement avec V2 et V3, alors que V0 et V1 la laissent au rang 5. Mais « L. 1132-11 », contrôle négatif sans attendu, y place l'article L. 1132-1 **au rang 1** avec V2 et V3, par la branche dense seule : le critère formel est satisfait, il ne discrimine pas.
- **Ce qui distingue réellement V2 est lexical et faible.** V2 retrouve « L. 1132-1 » au rang 4 et ne renvoie aucun document pour « L. 1132-11 », là où V0 retrouve « L1132-1 » au rang 1 et noie le contrôle négatif dans 10 résultats. V2 fait donc passer le rang lexical de « L1132-1 » de 1 à 4 : le découpage de V0 était plus permissif, et `rrf` absorbe la perte.
- **V1 n'apporte rien de mesurable** : la liste de mots vides française retirait déjà les élisions simples.

## Décision

**La chaîne d'analyse d'ADR-0086 reste inchangée. Aucun filtre d'élision, aucun traitement d'identifiant n'est ajouté. V2 (identifiants en un seul jeton) reste une option de produit, non retenue.**

Précisions :

1. **Rien ne change dans le moteur.** `lexical.language: "fr"` applique toujours `SimpleTokenizer`, `RemoveLongFilter(40)`, `LowerCaser`, `AsciiFoldingFilter`, `StopWordFilter(fr)`, `Stemmer(fr)`. Aucun code de production n'est modifié, et aucune reconstruction d'index n'est déclenchée.
2. **V1 est écartée sans réserve.** Son effet mesuré est nul en recall comme en MRR, alors qu'elle change 15 213 passages sur 258 071. Un changement de tokenisation sans gain ajoute du risque et un coût de reconstruction.
3. **V2 n'est pas retenue, mais reste ouverte comme option de produit.** Elle est la seule variante qui satisfait le critère formel des sondes sans régression mesurable, mais ce critère ne discrimine pas son contrôle négatif, et l'appui réel est le rang lexical d'une sonde unique. Ce n'est pas un gain de qualité mesuré : la mesure ne fonde pas une modification de la chaîne.
4. **Aucune configuration nouvelle.** Il n'existe ni clé de filtre d'élision, ni mode d'identifiant : la décision ne crée aucune valeur dans `index.json` et ne change pas le domaine de `lexical.language` (ADR-0086).
5. **Ce qui n'est pas décidé ici.** Le cas « article 1240 » n'est pas rouvert : il est couvert par MES-10 et par la décision sur la fusion (ADR-0093). `docs/05` et `docs/04` ne changent pas.
6. **La question se rejoue en P9.** La mesure manque de puissance : aucune question étiquetée ne porte d'identifiant, et le jeu enrichi est provisoire. Q-04 se rejoue avec des questions à identifiant sur le vrai jeu de référence (`CONSTITUER-JEU-DE-REFERENCE`, P9).

## Conséquences

- **Positives** :
  - la chaîne d'analyse est celle qui a servi à toutes les mesures de P5 : les chiffres de MES-06, MES-10 et MES-11 restent valables sur les index existants ;
  - aucun index à reconstruire, et aucune clé de configuration nouvelle à documenter ou à valider ;
  - la décision suit la règle fixée avant la mesure (ADR-0092, décisions 5 et 6) et laisse une trace du fait que V2 a été examinée, avec son appui et sa faiblesse.
- **Négatives / coûts** :
  - une requête qui porte un identifiant reste tokenisée en morceaux (`l1132` + `1`), ce qui est plus permissif : la recherche exacte d'une référence dépend alors de la fusion, non du lexical ;
  - Q-04 est close sur une égalité, pas sur une réponse positive : la question des identifiants reste un manque de connaissance, et le vrai jeu de référence devra la porter ;
  - si V2 est un jour retenue, elle changera la tokenisation de 21,1 % des passages et déclenchera une reconstruction de l'index lexical (`docs/03-recherche.md:94`).
- **À surveiller** :
  - **le vrai jeu de référence en P9**, avec des questions à identifiant : un gain mesuré sur `CONSTITUER-JEU-DE-REFERENCE` ouvrirait un **nouvel ADR**, puis une tâche d'analyseur. La mesure de P5 ne permet ni de l'affirmer ni de l'exclure ;
  - **les sondes.** Le critère du top 3 ne sépare pas V2 de V0 sur le contrôle négatif. Une sonde qui distingue vraiment `L. 1132-1` de `L. 1132-11` en `rrf` devra être écrite avant tout nouveau jugement ;
  - **la liste de mots vides.** V1 est sans effet parce que cette liste retire déjà les élisions simples : tout changement de liste (nouvelle version de tantivy ou de Snowball) modifierait ce constat et demanderait une remesure, comme le prévoit ADR-0086.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Adopter V1, le filtre d'élision | Effet mesuré nul (0,9149 et 0,7826, identiques à V0, intervalle [0 ; 0]) pour 15 213 passages retokenisés : un coût de reconstruction sans gain. |
| Adopter V2, identifiants en un seul jeton | Seul appui : une sonde au rang lexical 4 et un contrôle négatif que le critère du top 3 ne discrimine pas. La mesure ne montre aucun gain de recall ni de MRR. C'est un choix de produit, et le propriétaire ne le retient pas à ce stade. |
| Adopter V3 (élisions et identifiants) | V3 cumule V1, sans effet, et V2, non retenue : 24,5 % des passages changent pour aucun gain mesuré. |
| Laisser Q-04 ouverte sans décision | La mesure a un protocole accepté et une règle de décision écrite ; ne rien décider laisserait `docs/03:87` en « à mesurer » alors que la mesure a eu lieu. La décision de ne rien changer est un résultat, et elle est tracée. |
| Modifier l'analyseur directement, sans ADR | Contraire à ADR-0092, décision 5 : un résultat positif ne justifie qu'un ADR puis une tâche. Aucune campagne ne code de changement. |

## Références

- Q-04, `docs/adr/README.md` (questions ouvertes, puis questions tranchées).
- `docs/03-recherche.md:87` (élisions et identifiants), ADR-0086 (`:68`, ce qu'il ne tranche pas), ADR-0092 décisions 5 et 6.
- `docs/mesures/MES-11-analyse-francaise.md` et son artefact daté `docs/mesures/MES-11-analyse-francaise.2026-10-28.json`.
- Tâches `MESURER-HYBRIDE` (P5), `CONSTITUER-JEU-DE-REFERENCE` et `COMPARER-AVEC-PGVECTOR` (P9).
