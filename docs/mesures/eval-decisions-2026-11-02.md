# Éval — decisions — 2026-11-02

## But

Mesurer, sur l'index évalué, la fidélité de la recherche rapide contre la recherche exacte, la qualité des résultats contre les documents attendus, et les latences par étape. Campagne produite par `crible eval` (ADR-0125).

## Machine

- Système : macos aarch64
- Cœurs disponibles : 8
- Version de Crible : 0.1.0

## Données

- Index : `decisions`
- Génération : 1
- Passages vivants : 49998
- Jeu de requêtes : `decisions` (200 requêtes, dont 200 avec des documents attendus)

## Paramètres

- Commande : `crible eval --data data/j7-demo/data --index decisions --queries data/j7-demo/queries/decisions.jsonl --models data/models --modes dense,lexical,hybrid --k 10 --out docs/mesures --repeat 3`
- Modes : dense, hybrid, lexical
- `k` de la qualité : 10
- `exact_threshold` : 0 (0 force la stratégie binaire en mémoire, ADR-0076)
- `rescore_candidates` (N) : 5000 (valeur de `search.rescore_candidates` de l'index ; la campagne la remplace quand `--rescore-candidates` est donné). Source de N : `configuration`
- Répétitions comptées : 3 (une passe d'échauffement non comptée précède)

## Résultats

### Mode `dense`

- Stratégie dense du plan : binary × 200
- Fidélité : recall@10 au passage 0.9965, recouvrement@10 au document 0.9975 (n = 200)
- Qualité : recall@k 0.5063, MRR@k 0.5068 (n = 200)

| Étape | p50 (ms) | p95 (ms) | p99 (ms) |
|---|---|---|---|
| `dense` | 3.405 | 3.916 | 8.006 |
| `encode` | 0.001 | 0.001 | 0.001 |
| `fetch` | 0.184 | 0.353 | 0.373 |
| `filter` | 0.001 | 0.001 | 0.002 |
| `group` | 0.002 | 0.003 | 0.003 |
| `total` | 3.667 | 4.157 | 8.341 |
| `validate` | 0.000 | 0.000 | 0.000 |

### Mode `hybrid`

- Stratégie dense du plan : binary × 200
- Fidélité : recall@10 au passage 0.9890, recouvrement@10 au document 0.9891 (n = 200)
- Qualité : recall@k 0.5638, MRR@k 0.5701 (n = 200)

| Étape | p50 (ms) | p95 (ms) | p99 (ms) |
|---|---|---|---|
| `dense` | 3.447 | 3.867 | 9.305 |
| `encode` | 0.001 | 0.001 | 0.001 |
| `fetch` | 0.181 | 0.368 | 0.411 |
| `filter` | 0.001 | 0.001 | 0.001 |
| `fusion` | 0.017 | 0.019 | 0.019 |
| `group` | 0.005 | 0.005 | 0.006 |
| `lexical` | 0.888 | 1.279 | 1.683 |
| `total` | 3.713 | 4.179 | 10.032 |
| `validate` | 0.000 | 0.000 | 0.000 |

### Mode `lexical`

- Qualité : recall@k 0.5800, MRR@k 0.6119 (n = 200)

| Étape | p50 (ms) | p95 (ms) | p99 (ms) |
|---|---|---|---|
| `fetch` | 0.304 | 0.356 | 0.372 |
| `filter` | 0.000 | 0.001 | 0.001 |
| `group` | 0.002 | 0.003 | 0.003 |
| `lexical` | 0.703 | 1.093 | 1.475 |
| `total` | 0.971 | 1.398 | 1.730 |
| `validate` | 0.000 | 0.000 | 0.000 |

## Lecture

- La cible de fidélité est 0,99 de recall@10 au passage, **au N par défaut de 5000** et en moyenne sur les requêtes dont la liste exacte n'est pas vide (ADR-0076, ADR-0078, ADR-0125 complément C1). Une campagne à un autre N éclaire ce réglage : elle ne se compare pas directement à la cible. Le recouvrement@10 n'a pas de cible, il dépend du nombre de passages par document.
- Le dénominateur du recall@10 de fidélité est la taille du top-10 EXACT : il vaut 10 dès que le pool filtré compte au moins dix passages, et le rapport écrit sa valeur par requête (`fidelity_denominator`) quand elle diffère de 10. Une requête dont la liste exacte est vide est exclue de la moyenne et comptée à part, avec son `id` (ADR-0125, complément du 2026-11-01, point C1).
- La qualité ne porte que sur les requêtes qui portent des documents attendus : `n` la borne, et un `n` faible rend chaque requête plus lourde que le seuil de régression de 0,005. Un attendu absent du jeu mesuré compte simplement comme non trouvé
(ADR-0125, décision 4).
- Les latences sont **indicatives** : elles viennent de `debug.timings_ms`, ne valent pas une mesure MES-xx et ne changent jamais le code de sortie. La passe d'échauffement charge lecteurs, modèle et cache de requêtes (ADR-0051), donc `encode` mesure le chemin d'un cache déjà chaud, pas le premier encodage.
- La machine est décrite par son système, son architecture et ses cœurs : un rapport de mesure MES-xx, lui, nomme le processeur et la RAM.
- Les chiffres portent sur cette génération et ce jeu : ils ne valent pas preuve à 8,58 M passages, où la fidélité se confirme en P9 (ADR-0076).
- Une régression de `--baseline` est une baisse de plus de 0,005 en valeur absolue d'une moyenne de qualité ou de fidélité ; le code de sortie vaut alors 1. Les latences sont comparées et rapportées, jamais bloquantes.
