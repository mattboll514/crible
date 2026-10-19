# Documentation de Crible

## Parcours d'arrivée sur le projet

À lire dans cet ordre, une fois. Comptez ~45 minutes.

1. [`00-glossaire.md`](00-glossaire.md) : le vocabulaire, à employer tel quel.
2. [`01-vision-et-perimetre.md`](01-vision-et-perimetre.md) : pourquoi Crible, ce qui est dans la v1 et ce qui n'y est pas, le critère de réussite.
3. [`02-architecture.md`](02-architecture.md) : instance, index, disque, générations, concurrence, compaction, mémoire.
4. [`03-recherche.md`](03-recherche.md) : pool, branche dense, branche lexicale, fusion, regroupement, multi-index.
5. [`04-donnees-et-ingestion.md`](04-donnees-et-ingestion.md) : config d'index, modèle, documents → passages, format de lot, recette.
6. [`05-api.md`](05-api.md) : l'API REST.
7. [`06-observabilite.md`](06-observabilite.md) : analytics, journal, debug, explain, éval.
8. [`07-stack-et-deploiement.md`](07-stack-et-deploiement.md) : dépôt, dépendances, config, Docker, CI.
9. [`08-invariants-et-tests.md`](08-invariants-et-tests.md) : invariants INV-xx, stratégie de tests, mesures MES-xx.
10. [`adr/README.md`](adr/README.md) : les **décisions** (Architecture Decision Records) et leur processus, avec l'index des ADR et les questions ouvertes Q-xx. Lire au moins l'index et les ADR `Proposé`.
11. [`09-plan-de-developpement.md`](09-plan-de-developpement.md) : phases, jalons, règles, git.

> Les documents 01 à 08 décrivent **comment est** Crible ; les ADR expliquent **pourquoi**. En cas de doute sur une décision, c'est l'ADR accepté qui fait foi.

## Travail au quotidien

- [`guide-opencode.md`](guide-opencode.md) : installer opencode, Ollama, ColGrep et les plugins ; travailler avec l'orchestrateur ; quand ouvrir une nouvelle session ou compacter.

- **Agents** : pour une tâche, on lit **seulement** sa fiche dans [`phases/`](phases/) et les documents listés dans son champ « À lire ». Inutile de relire toute la doc.
- [`suivi.md`](suivi.md) : état des tâches et des jalons, cibles, écarts, idées, journal.
- [`adr/`](adr/) : une décision par fichier ; nouveau sujet → `/adr <sujet>` (statut `Proposé`), décision → le propriétaire.
- [`jalons/`](jalons/) : rapports et démos de jalon (`_modele-rapport.md`).
- [`mesures/`](mesures/) : rapports de mesure (`README.md` pour le modèle).

## Phases

| Phase | Fiche |
|---|---|
| P0 | [Fondations](phases/P0-fondations.md) |
| P1 | [Stockage, lots et recherche exacte](phases/P1-stockage-et-recherche-exacte.md) |
| P2 | [Encodeur et recette](phases/P2-encodeur-et-recette.md) |
| P3 | [Cycle de vie](phases/P3-cycle-de-vie.md) |
| P4 | [Recherche rapide](phases/P4-recherche-rapide.md) |
| P5 | [Lexical et hybride](phases/P5-lexical-et-hybride.md) |
| P6 | [Serveur et API](phases/P6-serveur-et-api.md) |
| P7 | [Observabilité](phases/P7-observabilite.md) |
| P8 | [Docker et déploiement](phases/P8-docker-et-deploiement.md) |
| P9 | [Migration Legifacile](phases/P9-migration-legifacile.md) |
