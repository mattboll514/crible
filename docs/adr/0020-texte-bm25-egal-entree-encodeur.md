# ADR-0020 — Le texte indexé en BM25 est l'entrée encodeur

- **Statut** : Accepté
- **Date** : 2026-10-18
- **Décideurs** : propriétaire
- **Proposé par** : brainstorm (cas « article 1240 »)

## Contexte

Le corps de l'art. 1240 du Code civil ne contient pas « 1240 » : le numéro est une métadonnée. Un BM25 qui n'indexerait que le corps ne le trouverait pas, alors que le gabarit de Legifacile ajoute déjà la hiérarchie et le numéro au texte encodé.

## Décision

Le texte indexé par tantivy est **l'entrée encodeur stockée** (ADR-0011), gabarit et en-tête compris : par exemple « Code civil > … > Article 1240 » suivi du corps. Les branches dense et lexicale voient exactement le même texte.

## Conséquences

- **Positives** : les numéros et les titres deviennent trouvables en BM25, et les deux branches sont cohérentes.
- **Négatives** : les en-têtes répétés pèsent un peu dans les statistiques BM25.
- **À surveiller** : MES-10 et MES-11, en particulier le cas « article 1240 » et ses variantes.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Indexer seulement le corps | Le numéro et le titre seraient introuvables, alors que c'est le cas qui motive l'hybride |
| Un champ BM25 séparé pour les métadonnées | Complexité (BM25F) ; les filtres exacts couvrent déjà les identifiants |

## Références

`docs/03-recherche.md` (branche lexicale) ; ADR-0007, ADR-0008, ADR-0011.
