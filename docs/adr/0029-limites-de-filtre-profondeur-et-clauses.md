# ADR-0029 — Limites de filtre : comptage de la profondeur et des clauses

- **Statut** : Accepté (décision du propriétaire le 2026-10-19)
- **Date** : 2026-10-19
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, tâche DEFINIR-CONFIG-INDEX
- **Remplace** : —

## Contexte

`docs/05-api.md` fixe deux limites pour un filtre : **profondeur 8 au plus** et **64 clauses au plus**. Il ne définit pas ce qu'on compte. Sans définition, un filtre peut être accepté par un client et refusé par le serveur, ou l'inverse, et le message d'erreur ne peut pas désigner la limite atteinte.

La validation du filtre est un livrable de `DEFINIR-CONFIG-INDEX`. Les tests déjà écrits supposent une convention précise : neuf `$not` imbriqués sont refusés, quatre sont acceptés. Il faut donc figer cette convention.

## Décision

**Profondeur** :

- on compte les **niveaux d'objets et de tableaux** du filtre ;
- la **racine vaut 1** ;
- un objet d'opérateurs de champ compte pour **un niveau** ;
- un tableau `$and` ou `$or` compte pour un niveau, et chacun de ses éléments pour le niveau suivant ;
- dépasser 8 donne `invalid_filter`, avec le chemin JSON de l'élément fautif.

Conséquence concrète : neuf `$not` imbriqués sont refusés, quatre sont acceptés.

**Clauses** :

- une **clause** est une **condition de champ**, c'est-à-dire une entrée dont la clé est un nom de champ ;
- les nœuds `$and`, `$or` et `$not` **ne comptent pas** ;
- 64 conditions sont acceptées ; la 65e donne `invalid_filter`, avec le chemin JSON.

Exemple : `{ "a": 1, "$or": [ { "b": 2 }, { "c": 3 } ] }` compte **3 clauses** et une profondeur de **3** (racine, tableau `$or`, objet élément).

## Conséquences

- **Positives** :
  - la borne est effective et testable, indépendamment du style d'écriture du client ;
  - le message d'erreur peut nommer le chemin exact et la limite ;
  - un client peut calculer la même valeur avant l'envoi.
- **Négatives / coûts** :
  - la règle doit être documentée côté client, sinon un filtre refusé surprend ;
  - certaines formes très plates atteignent la limite de clauses sans atteindre celle de profondeur, et inversement.
- **À surveiller** : si des filtres légitimes de Legifacile dépassent 64 clauses, rouvrir la limite avec un exemple réel à l'appui plutôt que de la relever sans mesure.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Racine = 0 | Décale toutes les bornes d'un niveau et rend le calcul moins intuitif pour un client ; la racine est un niveau comme un autre. |
| Ne compter que les nœuds logiques (`$and`, `$or`, `$not`) | Un filtre plat avec beaucoup de champs échapperait à la limite de profondeur, alors que le coût d'évaluation vient aussi du nombre de conditions. |

## Références

`docs/05-api.md` (grammaire et limites des filtres) ; `docs/03-recherche.md` (évaluation des filtres) ; fiche `DEFINIR-CONFIG-INDEX`.
