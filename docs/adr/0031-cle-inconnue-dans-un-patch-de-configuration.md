# ADR-0031 — Refuser une clé inconnue dans un `PATCH` de configuration

- **Statut** : Proposé
- **Date** : 2026-10-19
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, tâche DEFINIR-CONFIG-INDEX
- **Remplace** : —

## Contexte

`docs/04-donnees-et-ingestion.md` liste les réglages figés et les réglages modifiables d'un index. `docs/05-api.md` prévoit `PATCH /v1/indexes/{index}` : modifier un réglage figé renvoie `409 immutable_setting` (INV-16).

Le cas d'une clé qui n'existe dans aucune des deux listes n'est pas traité. Une faute de frappe (`rescore_candidate` au lieu de `rescore_candidates`) passerait donc sans effet, ou avec un effet dépendant de l'implémentation. Les tests déjà écrits supposent un refus explicite.

## Décision

Une clé **inconnue** dans le corps d'un `PATCH` de configuration est **refusée** :

- code d'erreur `invalid_request` (HTTP 400) ;
- le message **nomme la clé** et son **chemin** dans le corps JSON ;
- `immutable_setting` (409) reste réservé aux clés connues mais figées.

La règle s'applique à tous les niveaux du corps, y compris dans les objets imbriqués (`search`, `lexical`, `ingest`, `compaction`).

## Conséquences

- **Positives** :
  - une faute de frappe est signalée au lieu d'être ignorée ;
  - la distinction entre clé inconnue (400) et réglage figé (409) est nette ;
  - le client peut corriger sa requête à partir du message.
- **Négatives / coûts** :
  - compatibilité ascendante plus stricte : un client qui envoie un champ prévu pour une version future sera refusé au lieu d'être toléré ;
  - toute nouvelle clé de configuration devra être ajoutée au schéma du `PATCH` en même temps que sa prise en charge.
- **À surveiller** : un pic de `invalid_request` après l'ajout d'un client ou d'une version d'API indiquerait un besoin de versionner le corps plutôt que d'assouplir la règle.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Ignorer silencieusement la clé inconnue | Une faute de frappe passe inaperçue et le client croit avoir modifié un réglage ; le diagnostic devient coûteux. |
| Renvoyer `immutable_setting` pour toute clé non modifiable | Confond deux causes différentes (clé inexistante, clé figée) et rend le message d'erreur trompeur. |

## Références

`docs/04-donnees-et-ingestion.md` (réglages figés et modifiables) ; `docs/05-api.md` (`PATCH`, codes d'erreur) ; INV-16 ; fiche `DEFINIR-CONFIG-INDEX`.
