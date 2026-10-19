# ADR-0030 — Refuser un nom de champ déclaré aux deux niveaux

- **Statut** : Proposé
- **Date** : 2026-10-19
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, tâche DEFINIR-CONFIG-INDEX
- **Remplace** : —

## Contexte

`docs/04-donnees-et-ingestion.md` sépare le schéma en `document_fields` et `passage_fields`. `docs/03-recherche.md` précise que tous les champs sont matérialisés au niveau du passage et qu'un passage **hérite** des champs de son document. Un même nom peut donc être déclaré aux deux niveaux, et le schéma ne dit pas lequel gagne.

Rien n'interdit ce doublon aujourd'hui. Il rend ambiguë la résolution d'un nom dans un filtre, dans `apply_patch` et dans les messages d'erreur. Les tests déjà écrits supposent que ce cas est refusé à la validation.

## Décision

Un nom de champ déclaré **à la fois** dans `document_fields` et dans `passage_fields` est **refusé à la validation du schéma** :

- code d'erreur `invalid_request` (HTTP 400) ;
- le message **nomme le champ** et les **deux niveaux** (`document_fields` et `passage_fields`).

La validation s'applique à la création de l'index et à tout ajout de champ par `PATCH`.

Ce qui n'est pas décidé ici : le comportement d'un schéma déjà existant qui contiendrait un tel doublon. Aucun index n'est créé avant cette décision, donc le cas ne se présente pas en pratique ; s'il apparaît, il faudra un nouvel ADR.

## Conséquences

- **Positives** :
  - un nom de champ désigne **un seul niveau**, donc une seule source de valeur ;
  - la résolution dans les filtres et `apply_patch` reste déterministe ;
  - l'erreur est détectée tôt, à la configuration, et non à la première requête.
- **Négatives / coûts** :
  - un schéma qui voudrait surcharger un champ de document au niveau du passage doit choisir un autre nom ;
  - la règle est plus stricte que le texte actuel de `docs/04` et `docs/03`, qui ne l'énonce pas.
- **À surveiller** : si un besoin réel de surcharge apparaît (Q-03 ou le schéma de Legifacile), rouvrir la question avec un cas concret plutôt que d'assouplir la règle.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Le champ de passage prime | La valeur dépend alors du niveau, et une requête ne peut plus lire le champ sans connaître le schéma ; source d'erreurs silencieuses. |
| Le champ de document prime | Symétrique du précédent, et rend inutile une déclaration de passage que l'auteur croyait effective. |

## Références

`docs/03-recherche.md` (héritage des champs) ; `docs/04-donnees-et-ingestion.md` (schéma et champs) ; `docs/05-api.md` (`invalid_request`) ; fiche `DEFINIR-CONFIG-INDEX`.
