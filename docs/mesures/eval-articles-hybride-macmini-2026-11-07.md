# Éval — articles-hybride-macmini — 2026-11-07

## But

Mesurer, sur l'index évalué, la fidélité de la recherche rapide contre la recherche exacte, la qualité des résultats contre les documents attendus, et les latences par étape.  Campagne produite par `crible eval` (ADR-0125, ADR-0131).

## Machine

- Système : linux aarch64
- Cœurs disponibles : 8
- Version de Crible : 0.1.0

## Données

- Index : `articles`
- Génération : 1
- Passages vivants : 2476715
- Jeu de requêtes : `articles-hybride-macmini` (25 requêtes, dont 25 avec des documents attendus)

## Paramètres

- Commande : `crible eval --data /data --index articles --queries /in/articles-hybride-macmini.jsonl --models /models --modes hybrid --k 10 --out /out --exact-threshold 5000 --repeat 3`
- Modes : hybrid
- `k` de la qualité : 10
- `exact_threshold` : 5000 (0 force la stratégie binaire en mémoire, ADR-0076)
- `rescore_candidates` (N) : 5000 (valeur de `search.rescore_candidates` de l'index ; la campagne la remplace quand `--rescore-candidates` est donné). Source de N : `configuration`
- Répétitions comptées : 3 (une passe d'échauffement non comptée précède)

## Résultats

### Mode `hybrid`

- Stratégie dense du plan : binary × 25
- Fidélité : recall@10 au passage 1.0000, recouvrement@10 au document 1.0000 (n = 25)
- Qualité : recall@k 0.5600, MRR@k 0.2906 (n = 25)

| Étape | p50 (ms) | p95 (ms) | p99 (ms) |
|---|---|---|---|
| `dense` | 22.485 | 28.897 | 30.236 |
| `encode` | 0.004 | 0.013 | 0.017 |
| `fetch` | 0.334 | 0.490 | 0.681 |
| `filter` | 0.423 | 59.072 | 65.113 |
| `fusion` | 0.044 | 0.058 | 0.062 |
| `group` | 0.002 | 0.002 | 0.003 |
| `lexical` | 54.728 | 100.432 | 103.755 |
| `total` | 57.960 | 105.144 | 128.620 |
| `validate` | 0.008 | 0.013 | 0.026 |

## Lecture

- La cible de fidélité est 0,99 de recall@10 au passage, **au N par défaut de 5000** et en moyenne sur les requêtes dont la liste exacte n'est pas vide (ADR-0076, ADR-0078, ADR-0125 complément C1). Une campagne à un autre N éclaire ce réglage : elle ne se compare pas directement à la cible. Le recouvrement@10 n'a pas de cible, il dépend du nombre de passages par document.
- Le dénominateur du recall@10 de fidélité est la taille du top-10 EXACT : il vaut 10 dès que le pool filtré compte au moins dix passages, et le rapport écrit sa valeur par requête (`fidelity_denominator`) quand elle diffère de 10. Une requête dont la liste exacte est vide est exclue de la moyenne et comptée à part, avec son `id` (ADR-0125, complément du 2026-11-01, point C1).
- La qualité ne porte que sur les requêtes qui portent des documents attendus : `n` la borne, et un `n` faible rend chaque requête plus lourde que le seuil de régression de 0,005. Un attendu absent du jeu mesuré compte simplement comme non trouvé
(ADR-0125, décision 4).
- Les latences sont **indicatives** : elles viennent de `debug.timings_ms`, ne valent pas une mesure MES-xx et ne changent jamais le code de sortie. La passe d'échauffement charge lecteurs, modèle et cache de requêtes (ADR-0051), donc `encode` mesure le chemin d'un cache déjà chaud, pas le premier encodage.
- La machine est décrite par son système, son architecture et ses cœurs : un rapport de mesure MES-xx, lui, nomme le processeur et la RAM.
- Les chiffres portent sur cette génération et ce jeu : ils ne valent pas preuve à 8,58 M passages, où la fidélité se confirme en P9 (ADR-0076).
- Une régression de `--baseline` est une baisse de plus de 0,005 en valeur absolue d'une moyenne de qualité ou de fidélité ; le code de sortie vaut alors 1. Les latences sont comparées et rapportées, jamais bloquantes.
