# ADR-0006 — Segments par lot, pierres tombales et compaction occasionnelle

- **Statut** : Accepté
- **Date** : 2026-10-18
- **Décideurs** : propriétaire
- **Proposé par** : brainstorm (« option B »)

## Contexte

La base change tous les jours. Pour Legifacile, une nuit typique compte :
- ~930 passages ré-encodés ;
- ~220 documents supprimés ;
- ~1 230 republications identiques ;
- ~72 changements d'état liés à des dates.

Cela fait environ **1 % de passages morts par mois**. Les grosses vagues viennent des campagnes de ré-encodage, comme le re-découpage de la jurisprudence (de 2,0 M à 3,0 M passages).

Supprimer physiquement tout de suite, comme le font bm25x et NextPlaid, impose de réécrire les fichiers et de renuméroter les ids, et gêne les lectures concurrentes. Presque tous les moteurs (Lucene, tantivy, Qdrant, LanceDB, Postgres) font plutôt une suppression logique, et récupèrent la place plus tard.

## Décision

- Chaque lot crée un **segment immuable**, qui couvre une plage contiguë d'ids internes.
- Une suppression ou un remplacement pose une **pierre tombale** (`dead_since_gen`). Le passage sort de la bitmap des vivants.
- La place est récupérée par une **compaction complète**, qui construit une nouvelle époque (ids renumérotés, données re-triées). Elle se déclenche au-delà de `dead_ratio` (0,10 par défaut) ou à la main, sans GPU.

## Conséquences

- **Positives** :
  - le coût d'une écriture est proportionnel aux changements du jour ;
  - les ids restent stables d'une compaction à l'autre ;
  - les lectures se font sans verrou ;
  - le segment correspond au lot (ADR-0012) et facilite les sauvegardes incrémentales.
- **Négatives** :
  - les passages morts occupent de la place (RAM pour les codes, disque pour les vecteurs) ;
  - les statistiques BM25 sont légèrement faussées jusqu'à la compaction ;
  - la compaction demande deux fois l'espace disque pendant qu'elle tourne.
- **À surveiller** : la durée de la compaction (MES-07). Si elle devient trop longue, passer à des fusions progressives (option C, nouvel ADR).

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| A. Tout reconstruire chaque nuit | Le coût est proportionnel à toute la base, chaque nuit ; ça ne tient pas pour des bases plus grosses |
| C. Fusions progressives de segments (comme Lucene) | Politique de fusion et statistiques globales : trop complexe pour l'échelle actuelle |
| Suppression physique immédiate | Renumérotation, réécritures, conflits avec les lectures |

## Références

`docs/02-architecture.md` (suppressions, pierres tombales et compaction) ; ADR-0016, ADR-0017 ; MES-07, MES-08.
