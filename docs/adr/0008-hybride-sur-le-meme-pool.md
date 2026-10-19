# ADR-0008 — Hybride sur le même pool ; fusion RRF ou relative_score

- **Statut** : Accepté
- **Date** : 2026-10-17
- **Décideurs** : propriétaire
- **Proposé par** : brainstorm de conception

## Contexte

Le dense rate les références exactes : l'art. 1240 du Code civil sort au rang 70. Le BM25 les trouve si le numéro figure dans le texte indexé (ADR-0020). Aujourd'hui, Legifacile a bien un outil plein texte, mais il n'est pas fusionné avec le dense. Dans le sandbox, `relative_score` bat RRF quand un des deux signaux est faible.

## Décision

- Le mode `hybrid` lance les branches dense et lexicale **sur le même pool** (même filtre, mêmes vivants, INV-05), en parallèle.
- Deux méthodes de fusion sont proposées :
  - `rrf` : Σ 1/(60+rang), rien à régler ;
  - `relative_score` : min-max par branche, pondéré par α.
- La méthode par défaut se règle par index, et chaque requête peut la changer. En attendant MES-10 (Q-02), le défaut est `rrf`.
- On conserve le rang et le score de chaque branche, pour le debug.

## Conséquences

- **Positives** : filtrage cohérent entre les deux branches, réponse au cas « article 1240 », debug lisible (« dense 70, BM25 2 → fusionné 1 »).
- **Négatives** : deux branches à faire tourner, donc plus de latence et de CPU.
- **À surveiller** : MES-10 (qualité par méthode et par α).

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Filtrer chaque branche séparément | Risque de fusionner deux ensembles différents |
| Une seule méthode de fusion | Les mesures du sandbox montrent qu'aucune ne gagne partout |

## Références

`docs/03-recherche.md` (fusion) ; INV-05 ; MES-10.
