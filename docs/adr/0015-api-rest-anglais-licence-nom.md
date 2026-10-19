# ADR-0015 — API REST v1 en anglais, licence Apache 2.0, nom « Crible »

- **Statut** : Accepté
- **Date** : 2026-10-18
- **Décideurs** : propriétaire
- **Proposé par** : brainstorm de conception

## Contexte

Crible est générique, réutilisable dans plusieurs projets, et publiable. Il doit être appelé par des clients en Python (le MCP de Legifacile) et par la CLI.

## Décision

- **API REST** versionnée sous `/v1`, en JSON, avec des noms **en anglais** et en `snake_case`. La spécification OpenAPI est générée depuis le code (Swagger UI).
- **Documentation de conception en français.** Le code, les commentaires et les messages de commit sont en anglais.
- **Licence Apache 2.0**, comme les outils LightOn dont Crible s'inspire.
- **Nom : Crible**, de « passer au crible » : filtrer d'abord, examiner chaque résultat. Le nom était libre sur crates.io au 18/10/2026.

## Conséquences

- **Positives** : une API standard et publiable, avec un client Python généré ou écrit facilement.
- **Négatives** : deux langues dans le projet (documentation en français, code en anglais).
- **À surveiller** : vérifier les collisions de nom (GitHub, produits existants) avant toute publication.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| gRPC | Plus lourd pour des clients simples |
| Noms d'API en français | Moins réutilisable et moins publiable |
| Autres noms (Tamis, Vigie, Phare…) | Crible porte à la fois le filtrage et l'examen |

## Références

`docs/05-api.md` ; `LICENSE`.
