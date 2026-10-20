# Ligne de base -- `api.recherche_semantique`

Mesuré le 2026-10-20 par `scripts/db/eval_recherche.py` (`--k 10 --dedupliquer document`), jeu de questions `$PROD_REPO/docs/mcp/evaluation/questions.yaml`, log complet : `/private/tmp/mes00/logs/eval_recherche_20261020-094146.log`.
Backend : SQL direct (`api.recherche_semantique`, comportement du lot 1 inchangé)

**Ce que cette mesure ne dit pas** : les questions de `questions.yaml` sont écrites en interne, par l'équipe technique du projet, PAS par une juriste (cf. l'en-tête de ce fichier). Cette ligne de base sert à comparer le lot 3 au lot 1 -- pas à juger de la pertinence pour un usage professionnel réel. Voir la section « Limites » en fin de rapport.

## Contexte de la mesure

- Révision du modèle : `a5fdb000f7a21da96c3bddde3a782ef777316df3` (`lightonai/mDenseOn`)
- Version de `transformers` : `5.14.1`
- `ef_search` rendu par la fonction : 200 (une seule valeur observée)
- Mode de déduplication : `document` -- `p_limite` (= k) = 10
- Filtre `--fonds` (défaut, jamais un écrasement d'un filtre propre à une question) : aucun -- comportement historique inchangé
- Questions portant un filtre `fonds` propre dans `questions.yaml` (qui aurait gagné sur `--fonds`) : 0 -- aucune à ce jour, cf. `docs/mcp/evaluation/questions.yaml` : ce run mesure uniquement l'effet du défaut `--fonds`, jamais une surcharge par question
- Fraîcheur des corpus interrogés (`api.fraicheur_vecteurs`) :
  - `dole` : etat=`a_jour` documents_en_derive=0 derniere_sync=2026-10-20T03:38:09.312525+00:00 derniere_verification=2026-10-20T03:38:09.312525+00:00
  - `jorf_normatif` : etat=`a_jour` documents_en_derive=0 derniere_sync=2026-10-20T03:39:46.248543+00:00 derniere_verification=2026-10-20T03:39:46.248543+00:00
  - `juri_sommaires` : etat=`a_jour` documents_en_derive=0 derniere_sync=2026-10-20T03:39:46.296361+00:00 derniere_verification=2026-10-20T03:39:46.296361+00:00
  - `juri_texte_integral` : etat=`a_jour` documents_en_derive=0 derniere_sync=2026-10-20T03:41:54.30588+00:00 derniere_verification=2026-10-20T03:41:54.30588+00:00
  - `sarde` : etat=`a_jour` documents_en_derive=0 derniere_sync=2026-10-20T03:41:54.346023+00:00 derniere_verification=2026-10-20T03:41:54.346023+00:00
  - `textes_vigueur` : etat=`a_jour` documents_en_derive=0 derniere_sync=2026-10-20T03:46:22.695639+00:00 derniere_verification=2026-10-20T03:46:22.695639+00:00
- Avertissements rencontrés, par corpus :
  - `dole` : qualite_non_mesuree
  - `juri_sommaires` : qualite_non_mesuree
  - `sarde` : qualite_non_mesuree
  - `textes_vigueur` : perimetre_approximatif

## Métriques agrégées (questions `satisfiable: true` seulement)

- Questions évaluées : 57 (dont 56 satisfiable, 1 non satisfiable -- comptées à part ci-dessous, jamais mêlées à ces moyennes) ; 0 erreur(s) d'appel.
- **rappel@10 moyen** : 0.482 sur 56 question(s) évaluée(s) sans erreur (27 à rappel plein, 29 à rappel nul).
- **rang du premier document pertinent** : médiane 2 sur 27 question(s) où un attendu a été trouvé ; 29 question(s) satisfiable sans aucun attendu trouvé (rang `null`).
- **taux de hors-vigueur non signalés** (résultats à `etat_applicable=true`, dates connues) : global 6/130 (4.6%).
  - `textes_vigueur` : 6/130 (4.6%)
- **taux de passages d'argumentation servis** (résultats `moyens`/`entete`/`faits` parmi les résultats d'un corpus segmenté, cf. `sync_vecteurs.SEGMENTEURS`) : 108/310 (34.8%).
  - répartition par partie des résultats servis (corpus segmentés, 310 résultat(s) — `null` = non localisé) : `motifs` 189 (61.0%), `moyens` 101 (32.6%), `indetermine` 13 (4.2%), `faits` 7 (2.3%).

## Questions `satisfiable: false` (comptées à part)

Vérité terrain structurellement absente du corpus visé (ex. Q30 : article `ABROGE_DIFF` en vigueur aujourd'hui, encodé dans `textes_hors_vigueur`, jamais dans `textes_vigueur`). Un rappel non nul ici serait une ANOMALIE à investiguer, pas un succès.

| id | corpus | rappel obtenu | attendu | résultat |
|---|---|---|---|---|
| Q30 | `textes_vigueur` | 1.0 | rappel = 0 | ⚠️ ANOMALIE : trouvé alors que non satisfiable |

## Répartition des résultats par fonds

Ce que chaque question a réellement reçu comme filtre `p_fonds`, et à quel `fonds` appartiennent les résultats rendus (top-k, après déduplication).

| id | corpus | fonds demandé (source) | fonds appliqué | répartition des résultats |
|---|---|---|---|---|
| Q01 | `textes_vigueur` | — | — | KALI=8, LEGI=2 |
| Q02 | `textes_vigueur` | — | — | LEGI=10 |
| Q03 | `textes_vigueur` | — | — | LEGI=10 |
| Q04 | `textes_vigueur` | — | — | LEGI=10 |
| Q05 | `textes_vigueur` | — | — | LEGI=10 |
| Q06 | `textes_vigueur` | — | — | LEGI=10 |
| Q07 | `textes_vigueur` | — | — | LEGI=10 |
| Q08 | `textes_vigueur` | — | — | LEGI=10 |
| Q09 | `textes_vigueur` | — | — | LEGI=10 |
| Q10 | `textes_vigueur` | — | — | LEGI=10 |
| Q11 | `textes_vigueur` | — | — | LEGI=10 |
| Q12 | `textes_vigueur` | — | — | LEGI=10 |
| Q13 | `juri_sommaires` | — | — | CASS=10 |
| Q14 | `juri_texte_integral` | — | — | INCA=8, CAPP=2 |
| Q15 | `juri_texte_integral` | — | — | JADE=10 |
| Q16 | `juri_texte_integral` | — | — | CNIL=9, JADE=1 |
| Q17 | `juri_texte_integral` | — | — | INCA=7, CONSTIT=3 |
| Q18 | `juri_sommaires` | — | — | CASS=10 |
| Q19 | `juri_sommaires` | — | — | JADE=10 |
| Q20 | `juri_sommaires` | — | — | CASS=8, CAPP=2 |
| Q21 | `jorf_normatif` | — | — | JORF=10 |
| Q22 | `jorf_normatif` | — | — | JORF=10 |
| Q23 | `jorf_normatif` | — | — | JORF=10 |
| Q24 | `jorf_normatif` | — | — | JORF=10 |
| Q25 | `sarde` | — | — | NULL=10 |
| Q26 | `sarde` | — | — | NULL=10 |
| Q27 | `sarde` | — | — | NULL=10 |
| Q28 | `dole` | — | — | NULL=10 |
| Q29 | `dole` | — | — | NULL=10 |
| Q30 | `textes_vigueur` | — | — | LEGI=10 |
| Q31 | `juri_texte_integral` | — | — | INCA=9, CAPP=1 |
| Q32 | `juri_texte_integral` | — | — | INCA=9, CAPP=1 |
| Q33 | `juri_texte_integral` | — | — | INCA=8, CAPP=1, JADE=1 |
| Q34 | `juri_texte_integral` | — | — | CAPP=5, INCA=5 |
| Q35 | `juri_texte_integral` | — | — | INCA=10 |
| Q36 | `juri_texte_integral` | — | — | INCA=9, CAPP=1 |
| Q37 | `juri_texte_integral` | — | — | INCA=7, CAPP=3 |
| Q38 | `juri_texte_integral` | — | — | INCA=10 |
| Q39 | `juri_texte_integral` | — | — | INCA=10 |
| Q41 | `juri_texte_integral` | — | — | INCA=10 |
| Q42 | `juri_texte_integral` | — | — | INCA=10 |
| Q43 | `juri_texte_integral` | — | — | INCA=9, CONSTIT=1 |
| Q44 | `juri_texte_integral` | — | — | INCA=10 |
| Q45 | `juri_texte_integral` | — | — | INCA=9, JADE=1 |
| Q46 | `juri_texte_integral` | — | — | INCA=10 |
| Q47 | `juri_texte_integral` | — | — | INCA=9, CAPP=1 |
| Q48 | `juri_texte_integral` | — | — | INCA=4, JADE=4, CAPP=2 |
| Q49 | `juri_texte_integral` | — | — | INCA=10 |
| Q50 | `juri_texte_integral` | — | — | CAPP=7, INCA=3 |
| Q51 | `juri_texte_integral` | — | — | INCA=10 |
| Q52 | `juri_texte_integral` | — | — | INCA=10 |
| Q53 | `juri_texte_integral` | — | — | INCA=10 |
| Q54 | `juri_texte_integral` | — | — | INCA=10 |
| Q55 | `juri_texte_integral` | — | — | INCA=10 |
| Q56 | `juri_texte_integral` | — | — | INCA=10 |
| Q57 | `juri_texte_integral` | — | — | INCA=10 |
| Q58 | `juri_texte_integral` | — | — | JADE=7, INCA=3 |

Total toutes questions confondues : INCA=249, LEGI=122, NULL=50, JORF=40, JADE=34, CASS=28, CAPP=26, CNIL=9, KALI=8, CONSTIT=4

## Détail par question

| id | domaine | corpus | rappel | rang | trouvés | manqués | interdits présents | avertissements | argumentation | erreur |
|---|---|---|---|---|---|---|---|---|---|---|
| Q01 | droit du travail | `textes_vigueur` | 0.0 | null | — | LEGIARTI000045391841 | — | perimetre_approximatif | — | — |
| Q02 | droit civil | `textes_vigueur` | 0.0 | null | — | LEGIARTI000032041571 | — | perimetre_approximatif | — | — |
| Q03 | sécurité sociale | `textes_vigueur` | 1.0 | 2 | LEGIARTI000006743112 | — | — | perimetre_approximatif | — | — |
| Q04 | droit commercial | `textes_vigueur` | 1.0 | 3 | LEGIARTI000047381704 | — | — | perimetre_approximatif | — | — |
| Q05 | droit de la consommation | `textes_vigueur` | 1.0 | 2 | LEGIARTI000044142579 | — | — | perimetre_approximatif | — | — |
| Q06 | droit de l'environnement | `textes_vigueur` | 0.0 | null | — | LEGIARTI000043978078 | — | perimetre_approximatif | — | — |
| Q07 | droit de l'urbanisme | `textes_vigueur` | 1.0 | 4 | LEGIARTI000006815663 | — | — | perimetre_approximatif | — | — |
| Q08 | droit pénal | `textes_vigueur` | 1.0 | 1 | LEGIARTI000006418127 | — | — | perimetre_approximatif | — | — |
| Q09 | droit fiscal | `textes_vigueur` | 1.0 | 6 | LEGIARTI000053542636 | — | — | perimetre_approximatif | — | — |
| Q10 | santé publique | `textes_vigueur` | 1.0 | 2 | LEGIARTI000054137430 | — | — | perimetre_approximatif | — | — |
| Q11 | éducation | `textes_vigueur` | 1.0 | 1 | LEGIARTI000038901859 | — | — | perimetre_approximatif | — | — |
| Q12 | transports | `textes_vigueur` | 1.0 | 3 | LEGIARTI000036749888 | — | — | perimetre_approximatif | — | — |
| Q13 | jurisprudence sociale — judiciaire (CASS) | `juri_sommaires` | 1.0 | 1 | JURITEXT000054392399 | — | — | qualite_non_mesuree | — | — |
| Q14 | jurisprudence civile/commerciale — judiciaire (CAPP) | `juri_texte_integral` | 0.0 | null | — | JURITEXT000046990794 | — | — | 7/10 (70%) | — |
| Q15 | jurisprudence administrative — JADE (Conseil d'État) | `juri_texte_integral` | 0.0 | null | — | CETATEXT000054616043 | — | — | 0/10 (0%) | — |
| Q16 | jurisprudence — CNIL | `juri_texte_integral` | 0.0 | null | — | CNILTEXT000053391342 | — | — | 0/10 (0%) | — |
| Q17 | jurisprudence — CONSTIT (QPC) | `juri_texte_integral` | 1.0 | 2 | CONSTEXT000052372192 | — | — | — | 2/10 (20%) | — |
| Q18 | jurisprudence pénale — judiciaire (CASS, chambre criminelle) | `juri_sommaires` | 1.0 | 1 | JURITEXT000054110110 | — | — | qualite_non_mesuree | — | — |
| Q19 | jurisprudence administrative — JADE (Conseil d'État) | `juri_sommaires` | 0.0 | null | — | CETATEXT000008221541 | — | qualite_non_mesuree | — | — |
| Q20 | jurisprudence commerciale — judiciaire (CASS) | `juri_sommaires` | 0.0 | null | — | JURITEXT000054422719 | — | qualite_non_mesuree | — | — |
| Q21 | fonction publique — JORF (arrêté) | `jorf_normatif` | 1.0 | 3 | JORFARTI000042845467 | — | — | — | — | — |
| Q22 | droit du travail — JORF (arrêté) | `jorf_normatif` | 0.0 | null | — | JORFARTI000054408921 | — | — | — | — |
| Q23 | sécurité sociale — JORF (arrêté) | `jorf_normatif` | 0.0 | null | — | JORFARTI000046221547 | — | — | — | — |
| Q24 | agriculture — JORF (arrêté) | `jorf_normatif` | 1.0 | 1 | JORFARTI000044793391 | — | — | — | — | — |
| Q25 | énergie — SARDE (recensement thématique) | `sarde` | 1.0 | 1 | SARDBLOC000007204874 | — | — | qualite_non_mesuree | — | — |
| Q26 | environnement — SARDE (recensement thématique) | `sarde` | 1.0 | 1 | SARDBLOC000018568777 | — | — | qualite_non_mesuree | — | — |
| Q27 | fonction publique hospitalière — SARDE (recensement thématique) | `sarde` | 1.0 | 3 | SARDBLOC000018576254 | — | — | qualite_non_mesuree | — | — |
| Q28 | environnement — DOLE (dossier législatif) | `dole` | 1.0 | 4 | JORFDOLE000020078758 | — | — | qualite_non_mesuree | — | — |
| Q29 | droit des étrangers — DOLE (dossier législatif) | `dole` | 1.0 | 1 | JORFDOLE000047079143 | — | — | qualite_non_mesuree | — | — |
| Q30 | droit pénal — limite de périmètre du corpus (ABROGE_DIFF en vigueur aujourd'hui) | `textes_vigueur` | 1.0 | 1 | LEGIARTI000029370748 | — | — | perimetre_approximatif | — | — |
| Q31 | droit du travail — liberté d'expression du salarié | `juri_texte_integral` | 0.0 | null | — | JURITEXT000053493190 | — | — | 6/10 (60%) | — |
| Q32 | droit du travail — preuve de la discrimination salariale | `juri_texte_integral` | 1.0 | 3 | JURITEXT000053764828 | — | — | — | 4/10 (40%) | — |
| Q33 | profession d'avocat — portée d'une interdiction temporaire d'exercer | `juri_texte_integral` | 1.0 | 5 | JURITEXT000019003917 | — | — | — | 9/10 (90%) | — |
| Q34 | droit du travail — licenciement pour faute grave, usage d'entreprise invoqué en défense | `juri_texte_integral` | 0.0 | null | — | JURITEXT000030209224 | — | — | 4/10 (40%) | — |
| Q35 | droit des contrats — dol dans la cession d'un fonds de commerce, charge de la preuve | `juri_texte_integral` | 0.0 | null | — | JURITEXT000030388564 | — | — | 10/10 (100%) | — |
| Q36 | sécurité sociale — expertise médicale et autorité de la chose jugée | `juri_texte_integral` | 1.0 | 1 | JURITEXT000045904904 | — | — | — | 5/10 (50%) | — |
| Q37 | droit immobilier — devoir d'information du vendeur et du notaire lors d'une vente | `juri_texte_integral` | 1.0 | 1 | JURITEXT000039285432 | — | — | — | 3/10 (30%) | — |
| Q38 | droit de la concurrence — déséquilibre significatif entre partenaires commerciaux | `juri_texte_integral` | 0.0 | null | — | JURITEXT000030328766 | — | — | 5/10 (50%) | — |
| Q39 | cautionnement — validité de la mention manuscrite légale (art. L. 341-2 et L. 341-3 du code de la consommation) | `juri_texte_integral` | 0.0 | null | — | JURITEXT000045470181 | — | — | 5/10 (50%) | — |
| Q41 | droit international privé — compétence judiciaire en matière de contrats de consommation, règlement Bruxelles I bis | `juri_texte_integral` | 1.0 | 1 | JURITEXT000050290428 | — | — | — | 6/10 (60%) | — |
| Q42 | procédure pénale — droits de la défense devant le juge des libertés et de la détention | `juri_texte_integral` | 0.0 | null | — | JURITEXT000045167330 | — | — | 2/10 (20%) | — |
| Q43 | droit pénal des affaires — caractérisation de la circonstance aggravante de bande organisée | `juri_texte_integral` | 0.0 | null | — | JURITEXT000041481976 | — | — | 6/10 (60%) | — |
| Q44 | droit du travail — contrat de travail intermittent | `juri_texte_integral` | 1.0 | 8 | JURITEXT000036803358 | — | — | — | 3/10 (30%) | — |
| Q45 | droit du travail — licenciement économique dans un groupe | `juri_texte_integral` | 0.0 | null | — | JURITEXT000007489778 | — | — | 1/10 (10%) | — |
| Q46 | procédure prud'homale — unicité de l'instance | `juri_texte_integral` | 1.0 | 5 | JURITEXT000023669056 | — | — | — | 6/10 (60%) | — |
| Q47 | sécurité sociale — faute inexcusable de l'employeur | `juri_texte_integral` | 0.0 | null | — | JURITEXT000032056066 | — | — | 2/10 (20%) | — |
| Q48 | procédure civile — autorité de la chose jugée | `juri_texte_integral` | 0.0 | null | — | JURITEXT000017963756 | — | — | 1/10 (10%) | — |
| Q49 | procédure civile — pourvoi contre une décision par défaut | `juri_texte_integral` | 0.0 | null | — | JURITEXT000031151572 | — | — | 5/10 (50%) | — |
| Q50 | procédure civile — péremption d'instance après incompétence | `juri_texte_integral` | 0.0 | null | — | JURITEXT000021857785 | — | — | 1/10 (10%) | — |
| Q51 | procédure civile — déchéance du pourvoi et aide juridictionnelle | `juri_texte_integral` | 0.0 | null | — | JURITEXT000007407460 | — | — | 1/10 (10%) | — |
| Q52 | droit civil — exception de nullité et acte exécuté | `juri_texte_integral` | 1.0 | 3 | JURITEXT000032636457 | — | — | — | 5/10 (50%) | — |
| Q53 | droit des contrats — assiette contractuelle du prix | `juri_texte_integral` | 0.0 | null | — | JURITEXT000028577377 | — | — | 0/10 (0%) | — |
| Q54 | procédure pénale — appel d'assises et intérêts civils | `juri_texte_integral` | 0.0 | null | — | JURITEXT000007641200 | — | — | 3/10 (30%) | — |
| Q55 | procédure pénale — contrôle de la mise en accusation | `juri_texte_integral` | 0.0 | null | — | JURITEXT000007640826 | — | — | 0/10 (0%) | — |
| Q56 | procédure pénale — détention provisoire après annulation du renvoi | `juri_texte_integral` | 0.0 | null | — | JURITEXT000029364231 | — | — | 1/10 (10%) | — |
| Q57 | droit de la concurrence — visites et saisies | `juri_texte_integral` | 0.0 | null | — | JURITEXT000007591636 | — | — | 2/10 (20%) | — |
| Q58 | droit de la presse — provocation à la discrimination | `juri_texte_integral` | 0.0 | null | — | JURITEXT000038194394 | — | — | 3/10 (30%) | — |

## Limites

- **Ces 30 questions sont écrites en interne, par l'équipe technique du projet, PAS par une juriste** (décision actée le 2026-08-07, cf. `docs/superpowers/plans/2026-08-07-lot1-fondations-sql.md` §F). L'avocate ne testera qu'après le lot 4, une fois le serveur MCP accessible. Ce jeu sert à comparer le lot 3 au lot 1 -- **pas** à juger de la pertinence pour un usage professionnel réel. Il sera enrichi des questions réelles de l'avocate et la ligne de base recalculée à ce moment-là.
- Le rappel n'est comparé à AUCUN balayage exhaustif ici (contrairement à T0, qui l'a fait sur 4 corpus sur 8) : cette mesure porte uniquement sur `api.recherche_semantique` telle qu'exposée, avec son `ef_search` figé et sa déduplication -- pas sur la capacité théorique de l'index HNSW.
- Le taux de hors-vigueur non signalés ne porte QUE sur les résultats effectivement renvoyés par ce run (top-k des questions évaluées), pas sur l'ensemble du corpus.
