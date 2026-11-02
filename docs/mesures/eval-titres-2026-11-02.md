# Éval — titres — 2026-11-02

## But

Mesurer, sur l'index évalué, la fidélité de la recherche rapide contre la recherche exacte, la qualité des résultats contre les documents attendus, et les latences par étape. Campagne produite par `crible eval` (ADR-0125).

## Machine

- Système : macos aarch64
- Cœurs disponibles : 8
- Version de Crible : 0.1.0

## Données

- Index : `titres`
- Génération : 1
- Passages vivants : 50000
- Jeu de requêtes : `titres` (8 requêtes, dont 8 avec des documents attendus)

## Paramètres

- Commande : `crible eval --data data/j7-demo/data --index titres --queries data/j7-demo/queries/titres.jsonl --models data/models --modes dense,lexical,hybrid --k 10 --out docs/mesures --repeat 3`
- Modes : dense, hybrid, lexical
- `k` de la qualité : 10
- `exact_threshold` : 0 (0 force la stratégie binaire en mémoire, ADR-0076)
- `rescore_candidates` (N) : 5000 (valeur de `search.rescore_candidates` de l'index ; la campagne la remplace quand `--rescore-candidates` est donné). Source de N : `configuration`
- Répétitions comptées : 3 (une passe d'échauffement non comptée précède)

## Résultats

### Mode `dense`

- Stratégie dense du plan : binary × 8
- Fidélité : recall@10 au passage 1.0000, recouvrement@10 au document 1.0000 (n = 8)
- Qualité : recall@k 1.0000, MRR@k 0.8333 (n = 8)

| Étape | p50 (ms) | p95 (ms) | p99 (ms) |
|---|---|---|---|
| `dense` | 3.264 | 5.477 | 8.135 |
| `encode` | 0.001 | 0.001 | 0.002 |
| `fetch` | 0.142 | 0.257 | 1.218 |
| `filter` | 0.001 | 0.001 | 0.001 |
| `group` | 0.003 | 0.003 | 0.003 |
| `total` | 3.417 | 5.658 | 8.273 |
| `validate` | 0.000 | 0.000 | 0.000 |

### Mode `hybrid`

- Stratégie dense du plan : binary × 8
- Fidélité : recall@10 au passage 1.0000, recouvrement@10 au document 1.0000 (n = 8)
- Qualité : recall@k 1.0000, MRR@k 0.8750 (n = 8)

| Étape | p50 (ms) | p95 (ms) | p99 (ms) |
|---|---|---|---|
| `dense` | 3.333 | 3.723 | 3.981 |
| `encode` | 0.001 | 0.001 | 0.001 |
| `fetch` | 0.147 | 0.204 | 0.204 |
| `filter` | 0.001 | 0.001 | 0.001 |
| `fusion` | 0.019 | 0.026 | 0.027 |
| `group` | 0.004 | 0.005 | 0.005 |
| `lexical` | 0.642 | 0.837 | 0.868 |
| `total` | 3.534 | 3.896 | 4.207 |
| `validate` | 0.000 | 0.000 | 0.000 |

### Mode `lexical`

- Qualité : recall@k 1.0000, MRR@k 0.9375 (n = 8)

| Étape | p50 (ms) | p95 (ms) | p99 (ms) |
|---|---|---|---|
| `fetch` | 0.196 | 0.208 | 0.214 |
| `filter` | 0.000 | 0.001 | 0.001 |
| `group` | 0.003 | 0.003 | 0.003 |
| `lexical` | 0.440 | 0.593 | 0.602 |
| `total` | 0.643 | 0.789 | 0.816 |
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
