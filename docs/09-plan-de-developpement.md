# 09 — Plan de développement

## Règles du jeu

1. **Dix phases, de P0 à P9.** Chaque phase se termine par un **jalon** (J0 à J9) que **seul le propriétaire valide**.
2. **Une phase ne commence que si le jalon précédent est validé** (voir `docs/suivi.md`).
3. **Chaque phase se découpe en tâches** nommées `NOM-EN-MAJUSCULE`.
   - Une tâche est petite : idéalement moins de 500 lignes de diff.
   - Elle a des critères d'acceptation testables.
   - **Une session d'agent = une tâche.**
4. **L'ordre des tâches dans une phase suit leurs dépendances** (champ « Dépend de »). Sans dépendance entre elles, deux tâches peuvent se faire dans n'importe quel ordre.
5. **La fiche fait foi pour le périmètre.**
   - Une idée hors fiche va dans `docs/suivi.md` (section « Idées »).
   - Un désaccord avec la conception, ou un choix que rien ne tranche, devient un **ADR `Proposé`** (`docs/adr/`, via `@greffier`) et une ligne dans la section « Écarts ». L'agent s'arrête.
5 bis. **Toute décision qui engage le projet est tracée dans un ADR** (ADR-0021) : dépendance, format, seuil, valeur par défaut, compromis.
   - Seul le propriétaire accepte un ADR.
   - Une tâche qui dépend d'un ADR `Proposé` attend la décision.
6. **Une tâche est FAIT seulement si toutes ces conditions sont remplies** :
   - `cargo fmt`, `cargo clippy -D warnings` et les tests passent (`pytest` si du Python est touché) ;
   - le relecteur a validé ;
   - `docs/suivi.md` est à jour ;
   - le commit est fait.
7. **Une performance n'est jamais affirmée sans une mesure** consignée dans `docs/mesures/`.

## Git

- **Toutes les opérations git sont faites par l'agent `@git`** (ADR-0023). Les autres agents ne lisent git qu'en lecture.
- `main` ne contient que des phases validées. La fusion dans `main` se fait **uniquement sur instruction explicite du propriétaire** (`/valider J<n>`).
- Une **branche par phase** : `phase/P1-stockage-et-recherche-exacte`, créée depuis `main` au début de la phase.
- Une **branche par tâche** : `tache/NOM-DE-TACHE`, créée depuis la branche de phase.
  - Elle est fusionnée dans la branche de phase (`--no-ff`) quand la tâche est FAIT.
- **Commits** conventionnels, en anglais, avec la tâche en pied de message et les ADR concernés :

  ```
  feat(store): write fp16 segments with blake3 checksums

  Task: ECRIRE-SEGMENTS-VECTEURS
  Refs: ADR-0012
  ```
- Le **push** ne se fait que sur confirmation du propriétaire, et **jamais** en force.
- **Interdit** : toute réécriture d'historique (`reset`, `rebase`, `--amend`).

## Pipeline d'une tâche (ADR-0023)

L'orchestrateur fait passer chaque tâche par les mêmes étapes :

```
@git debut-tache → @testeur (si invariants) → @dev → @relecteur (≤ 3 tours) → suivi → @git commit → @git fin-tache
```

Les sous-agents ne s'appellent pas entre eux : c'est l'orchestrateur qui enchaîne. Les sessions et la gestion du contexte sont décrites dans `docs/guide-opencode.md`.

## Format d'une fiche de tâche

```
### NOM-DE-TACHE
- **Objectif** : une phrase.
- **Dépend de** : tâches à terminer avant (ou « — »).
- **À lire** : documents et sections nécessaires (seulement ceux-là).
- **Livrables** : fichiers, modules, commandes.
- **Critères d'acceptation** : liste testable.
- **Invariants / mesures** : INV-xx, MES-xx concernés.
- **Hors périmètre** : ce qu'il ne faut pas faire.
```

## Vue d'ensemble des phases

| Phase | Fichier | But | Jalon : ce que le propriétaire valide |
|---|---|---|---|
| P0 | `phases/P0-fondations.md` | Workspace, CI, types de base, config d'index, format de lot, référence pgvector | **J0** : CI verte ; configs et manifestes validés par des tests ; MES-00 consignée ; cibles fixées |
| P1 | `phases/P1-stockage-et-recherche-exacte.md` | Écrire des lots en Python, les attacher, filtrer, chercher en exact (vecteur fourni) | **J1** : sur `synth-small`, la CLI crée un index, attache des lots, filtre et cherche ; résultats identiques à la référence numpy ; INV-07 et INV-10 verts |
| P2 | `phases/P2-encodeur-et-recette.md` | Modèle ONNX, encodage des requêtes, recette, longueurs, mode en ligne, échantillon Legifacile | **J2** : recherche par texte sur l'échantillon Legifacile ; recette qui refuse un lot corrompu ; MES-04 et MES-05 |
| P3 | `phases/P3-cycle-de-vie.md` | Métadonnées, compaction, export, instantanés, reprise après panne | **J3** : trois lots quotidiens, compaction, aller-retour par export, arrêt brutal et reprise ; INV-03, 04, 08, 09, 14 verts |
| P4 | `phases/P4-recherche-rapide.md` | Codes binaires, SIMD, scan du pool, rescoring, stratégie, regroupement | **J4** : MES-01 (fidélité ≥ cible), MES-02, MES-03, MES-09 ; N par défaut choisi |
| P5 | `phases/P5-lexical-et-hybride.md` | tantivy, analyse française, BM25 dans le pool, fusion | **J5** : hybride sur l'échantillon (cas « article 1240 ») ; MES-06, 10, 11 ; fusion par défaut choisie |
| P6 | `phases/P6-serveur-et-api.md` | Serveur axum, auth, erreurs, écrivain et jobs, routes, multi-index, OpenAPI, client Python | **J6** : démo par Swagger et le client Python ; INV-02 sous charge ; INV-16 |
| P7 | `phases/P7-observabilite.md` | Analytics, journal, debug, explain, stats, éval | **J7** : `crible eval` produit un rapport ; `explain_ids` explique un document absent ; INV-13 et INV-17 verts |
| P8 | `phases/P8-docker-et-deploiement.md` | Image multi-arch, volumes, arrêt propre, compose, mesures en conteneur | **J8** : l'image tourne sur le Mac mini ; MES-02, 05 et 09 refaites en conteneur |
| P9 | `phases/P9-migration-legifacile.md` | Six index Legifacile, producteur, import complet, comparaison, MCP, mode ombre, quotidien, bascule | **J9 = v1** : critères de `01-vision-et-perimetre.md` remplis |

## Dépendances entre phases

```
P0 → P1 → P2 → P3 → P4 → P5 → P6 → P7 → P8 → P9
              └── l'échantillon Legifacile (P2) sert à P4, P5 et P7
```

L'ordre est strictement séquentiel en v1, pour garder une seule phase ouverte à la fois. Le serveur (P6) arrive tard exprès : jusqu'à P5, tout se valide par la CLI et les tests, sur un cœur stable.

## Valider un jalon (propriétaire)

1. Dans opencode : `/jalon J<n>`. L'agent `jalon` vérifie les tâches, lance les tests et la démo, rassemble les mesures et les ADR de la phase, puis écrit `docs/jalons/J<n>-rapport.md`.
2. Lire le rapport, puis lancer soi-même les commandes de la section « Démo ».
3. Si tout est bon : `/valider J<n>`. L'orchestrateur :
   - écrit `VALIDE <date>` dans le suivi ;
   - fait fusionner la phase dans `main` par `@git`, avec le tag `jalon-J<n>` ;
   - propose le push.
   Ouvrir ensuite une nouvelle session (`/new`) pour la phase suivante.
4. Sinon : lister les corrections. Elles deviennent des tâches `CORRIGER-…` de la même phase.
