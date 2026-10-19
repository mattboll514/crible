# ADR-0013 — Producteurs en Python, serveur Rust sur CPU

- **Statut** : Accepté
- **Date** : 2026-10-18
- **Décideurs** : propriétaire
- **Proposé par** : brainstorm de conception

## Contexte

L'encodage de Legifacile existe déjà en Python (PyTorch) : sur MPS pour le quotidien, en natif sur le Mac, et sur CUDA pour les campagnes OVH. Docker sur Mac tourne dans une VM Linux, sans accès au GPU Apple. Le serveur doit être rapide, sobre et livré en un seul binaire. Le propriétaire ne code pas en Rust : ce sont les agents qui écrivent le code.

## Décision

- **Serveur en Rust**, sur **CPU uniquement** : une image Docker multi-arch, sans image CUDA. Il n'encode que les requêtes (ONNX INT8) et les petits lots en ligne (fp32).
- **Producteurs en Python** : ils réutilisent le code d'encodage existant et écrivent des lots avec le paquet `crible` (`BatchWriter`).

## Conséquences

- **Positives** : réutilisation du code de Legifacile, pas de GPU à gérer côté serveur, une seule image.
- **Négatives** : deux langages dans le dépôt. Les gros encodages passent forcément par un producteur.
- **À surveiller** : la latence d'encodage des requêtes sur CPU dans le conteneur (MES-05).

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Encodage GPU dans le serveur | Pas de GPU dans Docker sur Mac ; image CUDA à maintenir |
| Serveur en Python | Moins performant pour les scans SIMD et le mmap ; empreinte plus lourde |

## Références

`docs/07-stack-et-deploiement.md` ; `docs/04-donnees-et-ingestion.md` (producteurs).
