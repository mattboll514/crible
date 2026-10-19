# ADR-0014 — Analytics sans contenu, journal de debug à la demande

- **Statut** : Accepté
- **Date** : 2026-10-17
- **Décideurs** : propriétaire
- **Proposé par** : propriétaire (analytics « sans garder les données d'entrée ») + brainstorm

## Contexte

Le propriétaire veut des chiffres concrets d'usage : temps de requête, erreurs, index les plus interrogés. Il veut aussi pouvoir débugger. Or les requêtes de Legifacile viennent d'une avocate et peuvent être sensibles. Un hash ne protège pas une requête courte, car « article 1240 » se retrouve avec un dictionnaire.

## Décision

On sépare deux composants :
- **Analytics** :
  - toujours actives, agrégées par minute (comptes, erreurs, histogrammes de latence par étape, taille du pool, forme du filtre réduite aux champs et opérateurs) ;
  - **sans aucun texte, valeur de filtre, hash ni id de résultat** ;
  - sûres **par construction** : le type d'événement n'a pas de champ pour ces informations (INV-13) ;
  - conservées 90 jours.
- **Journal de debug** :
  - contenu complet ;
  - **désactivé par défaut**, activé à la demande (ou par échantillonnage) ;
  - durée de vie de 7 jours, lisible par un admin seulement.

## Conséquences

- **Positives** : des chiffres utiles sans risque pour la confidentialité, et un debug complet quand il le faut.
- **Négatives** : les analytics ne permettent pas de retrouver une requête précise. Il faut pour cela activer le journal.
- **À surveiller** : le test INV-13, et les pertes d'écriture d'analytics (INV-17).

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Journal permanent de toutes les requêtes | Risque de confidentialité |
| Hasher les requêtes | Réversible par dictionnaire sur des requêtes courtes |

## Références

`docs/06-observabilite.md` ; INV-13, INV-17.
