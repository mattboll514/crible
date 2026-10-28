# ADR-0075 — Regroupement des résultats : ordre de `dedup` et `group_by`, sur-échantillonnage, forme de la réponse et `k` de la branche dense

- **Statut** : Accepté (accepté par anticipation le 2026-10-27, confirmé par le propriétaire le 2026-10-27)
- **Date** : 2026-10-27 (proposition et acceptation anticipée)
- **Décideurs** : propriétaire
- **Décision** : accepté par anticipation le 2026-10-27, par délégation du propriétaire pour la série REGROUPER-RESULTATS, MESURER-FIDELITE-BINAIRE, MESURER-LATENCE-DENSE, puis confirmé par le propriétaire le 2026-10-27. Les quatre décisions sont retenues telles que proposées par `@architecte` et choisies par l'orchestrateur : `dedup` s'applique avant `group_by`, sur toute la liste sur-échantillonnée, avec recomplètement ; la règle de sur-échantillonnage vit dans `crible-search` ; la réponse reste une liste plate d'entrées de passage où `k` compte des documents et où les entrées d'un document sont consécutives ; sous BINARY, le scan garde `max(N, candidates_per_branch)` candidats.
- **Proposé par** : agent greffier, sur demande de l'orchestrateur, pour la tâche `REGROUPER-RESULTATS` (phase P4), à partir des verdicts de `@architecte` sur l'implémentation de `@dev` ; les options sont retenues par l'orchestrateur par délégation
- **Remplace** : —

**Erratum du 2026-10-27 (ajouté à la confirmation, sans réécriture du corps).** Les renvois de code du corps sont périmés : les éléments cités sont désormais en `crates/crible-search/src/group.rs:158` (`candidates_per_branch`), `:185` (`group`), `:218` (`dedup_by_content`) et `:255` (`keep_documents`), et non en `:136`, `:157`, `:189` et `:213-240`. Le point « à surveiller » sur la contiguïté des entrées d'un document est **levé** : l'implémentation finale de `keep_documents` regroupe les candidats par document puis aplatit les groupes, et le test `grouped_entries_of_a_document_are_consecutive` (`crates/crible-search/tests/regrouper_resultats.rs`) couvre la contrainte de la décision 3. Le reste du corps est inchangé.

## Contexte

### Ce que la fiche demande

La fiche `REGROUPER-RESULTATS` (`docs/phases/P4-recherche-rapide.md:73-82`) demande `group_by: document`, `passages_per_document` et `dedup: content`. Ses livrables sont « le sur-échantillonnage (`clamp(k × 20, 200, 1000)` quand on regroupe), le regroupement, et la fusion des passages identiques par `passage_hash` » (`:77`). Ses critères sont : « on obtient k documents distincts », « le meilleur passage de chaque document est conservé », « les doublons de contenu sont fusionnés », et « des tests couvrent un document qui a plusieurs passages dans le top » (`:79-82`). La tâche ne cite aucun invariant et ne précise ni l'ordre des deux opérations, ni l'étage de la règle de sur-échantillonnage, ni la forme de la réponse quand `passages_per_document > 1`.

### Ce que la conception fixe

- **La place du regroupement** : étape 7 du pipeline, sous la clé de temps `group`, entre la fusion (étape 6) et la lecture des payloads (étape 8) (`docs/03-recherche.md:13`, `:107`).
- **Les deux opérations** (`docs/03-recherche.md:108-109`) : `group_by: "document"` garde le meilleur passage de chaque document, et `passages_per_document` (1 par défaut, 5 au maximum) ajoute d'autres passages du même document qui ont aussi matché ; `dedup: "content"` fusionne les passages de même `passage_hash` et garde le meilleur. Le glossaire les nomme sans ordre (`docs/00-glossaire.md:44`).
- **Le sur-échantillonnage** (`docs/03-recherche.md:52`, `:110-112`) : quand on regroupe, `candidates_per_branch = clamp(k × 20, 200, 1000)` ; sinon `max(k, 100)`. La branche dense « sort les `candidates_per_branch` meilleurs, avec leur cosinus », et « ce nombre est augmenté quand il faut regrouper ».
- **La forme de la réponse** (`docs/05-api.md:177-182`) : `results` est une liste plate d'entrées de niveau passage, avec `doc_id`, `passage`, `key`, `score`, `fields` et `text`. `k` va de 1 à 1000 et se nomme « Nombre de résultats » (`:162`) ; `group_by` vaut `"document"` ou null (`:164`) ; `passages_per_document` va de 1 à 5 (`:165`) ; `dedup` vaut `"content"` ou null (`:166`).
- **Sous BINARY** (`docs/03-recherche.md:68`, texte antérieur de `docs/05-api.md:206`, ADR-0074 décision 4) : le scan garde `max(N, k)` candidats, tous rescorés, et `k` y est « le nombre de résultats que demande la branche dense ».
- **Le debug et l'explain** (`docs/05-api.md:197-200`, `:216` ; `docs/phases/P7-observabilite.md:69`) : le plan expose `candidates_per_branch` et `group_by`, les temps exposent `group` ; `grouped_under` apparaît dans `explain`, et la fiche de P7 demande de dire si un document « a été absorbé par un regroupement ».

### Les questions tranchées ici

`@architecte` a rendu les verdicts suivants sur les choix que `@dev` a tranchés par inférence :

1. ordre entre `dedup: "content"` et `group_by: "document"` : **NON-COUVERT** ;
2. étage de la règle de sur-échantillonnage hors regroupement (`max(k, 100)`) : **NON-COUVERT** ;
3. forme de la réponse quand `group_by` est actif et `passages_per_document > 1` : **NON-COUVERT**, alors que `k` compte des documents, lecture plus précise de `P4:79` que de `05:162` ;
4. nombre de candidats gardés par le scan sous BINARY : **CONFORME** à ADR-0074 décision 4, mais divergent de la lettre de `docs/05:206`, qui écrivait `max(N, k)` avec le `k` de la requête (texte remplacé par cet ADR).

Sont **CONFORMES** et ne changent pas : le sur-échantillonnage vaut aussi pour `dedup` seul (glossaire `:44`, `03:13`, `:52`) ; `passages_per_document` est validé de 1 à 5 même sans `group_by`, où il est sans effet ; le plan de debug (`candidates_per_branch`, `group_by`, `timings_ms.group`) relève de `REPONSE-DEBUG` en P7 (ADR-0074 décision 3).

Aucune de ces questions ne se tranche par la lecture seule : elles fixent l'ordre d'une étape du pipeline, la couche qui porte une règle, la forme d'un contrat d'API et le sens d'un `k`, donc un ADR.

### L'état du code au moment du verdict

`@dev` a livré le regroupement dans `crates/crible-search/src/group.rs` : `candidates_per_branch` (`:136`), `group` (`:157`), `dedup_by_content` (`:189`) et `keep_documents` (`:213`), appelés par `dense::search_grouped` (`crates/crible-search/src/dense/planner.rs:226`). La CLI ne porte que les options de la requête (`crates/crible/src/main.rs:170-179`). Les tests d'intégration sont dans `crates/crible-search/tests/regrouper_resultats.rs`.

## Décision

**Le regroupement est une étape unique et entièrement déterminée : son ordre, son alimentation, la couche qui calcule le sur-échantillonnage et la forme de sa réponse sont fixés ici.**

Précisions :

1. **Ordre de `dedup` et `group_by`.** `dedup: "content"` s'applique **d'abord**, puis `group_by: "document"`, sur toute la liste des candidats sur-échantillonnés. Le regroupement garde les `k` meilleurs documents de cette liste, et les prend dans toute la liste : il se **recomplète** donc depuis les candidats restants quand la fusion en a retiré. Pour `dedup`, le passage gardé est le meilleur selon l'ordre de tri déjà en place (score décroissant, puis clé textuelle croissante) ; aucun nouveau critère de tri n'est introduit. Quand les candidats sont épuisés avant `k` documents, la réponse contient moins de `k` documents, **sans erreur**.
2. **Étage de la règle de sur-échantillonnage.** La règle `candidates_per_branch` (`clamp(k × 20, 200, 1000)` quand la requête regroupe, `max(k, 100)` sinon) vit dans la **couche de recherche partagée** (`crible-search`), là où se construit la planification. La CLI ne passe que `k` et les options de la requête ; le serveur de P6 n'aura pas à réimplémenter la règle. `dense::search(k)` garde un `k` explicite, et INV-06 reste inchangé.
3. **Forme de la réponse sous regroupement.** La réponse reste une **liste plate** d'entrées de niveau passage, de la forme de `docs/05-api.md:177-182`. Sous `group_by: "document"`, `k` compte des **documents** : au plus `k` documents distincts et au plus `k × passages_per_document` entrées. Les entrées d'un même document sont **consécutives** ; les documents sont rangés selon leur meilleur passage (score décroissant, puis clé croissante), et les passages d'un document par score décroissant puis clé croissante. Aucun champ nouveau n'apparaît dans les résultats en P4 : `doc_id` suffit à rattacher une entrée à son document. Tout marqueur de diagnostic (`grouped_under` d'`explain`, `docs/05:216`) reste en P7.
4. **Nombre de candidats sous BINARY.** Le scan garde `max(N, candidates_per_branch)` candidats, et tous sont rescorés : c'est la lecture d'ADR-0074 décision 4, où `k` désigne le nombre de résultats demandés par la branche dense. Le nombre effectif publié dans le plan suit donc le sur-échantillonnage, sans seconde règle.

Ce qui n'est **pas** décidé ici : la valeur de N (MES-01, Q-01) ; la forme sérialisée du plan et le contenu d'`explain`, qui relèvent de P7 ; le rôle de la clé stockée `search.candidates_per_branch` (`docs/04:30`, défaut 100), que la règle calculée ne lit pas — la garder comme plancher du cas sans regroupement, la retirer ou la laisser inerte demande une décision distincte ; la fusion hybride, qui n'existe pas encore en P4.

## Conséquences

- **Positives** :
  - l'étape 7 a un ordre total et reproductible : un même ensemble de candidats donne toujours le même regroupement ;
  - l'ordre `dedup` puis `group_by` protège le critère « `k` documents distincts », que l'ordre inverse violerait en supprimant le représentant d'un document ;
  - le recomplètement évite de rendre moins de `k` documents alors que la liste sur-échantillonnée en contient encore ;
  - la règle dans `crible-search` donne une seule implémentation pour la CLI, le serveur de P6 et l'éval : `candidates_per_branch` et le `k` de la branche dense ne peuvent plus diverger ;
  - la réponse plate garde le client actuel : aucun changement de schéma JSON, `doc_id` suffit à regrouper côté appelant ;
  - `max(N, candidates_per_branch)` aligne `docs/05:206` sur ADR-0074 et rend le rescoring suffisant pour le sur-échantillonnage.
- **Négatives / coûts** :
  - la contiguïté des entrées d'un document est une contrainte de plus pour l'implémentation : l'ordre global « meilleur d'abord » ne la donne pas, il faut grouper les entrées retenues par document ;
  - le sur-échantillonnage porte aussi sur `dedup` seul, donc une requête qui ne regroupe pas par document paie des candidats supplémentaires sans bénéfice visible dans la réponse ;
  - `max(N, candidates_per_branch)` fait grossir le rescoring dès que `clamp(k × 20, 200, 1000)` dépasse N : avec N = 1000, le pire cas vaut 1000 candidats, mais avec un N plus faible réglé par MES-01 le coût suit le regroupement ;
  - la clé `search.candidates_per_branch` reste dans `index.json` sans piloter la règle : la configuration cesse d'être auto-descriptive sur ce point tant que son rôle n'est pas tranché.
- **À surveiller** :
  - **écart d'implémentation à vérifier** : `keep_documents` (`crates/crible-search/src/group.rs:213-240`) parcourt les candidats dans l'ordre global et garde un passage dès qu'il appartient à un document déjà sélectionné ; les entrées d'un même document ne sont donc pas garanties consécutives quand un passage d'un autre document s'intercale entre deux passages du premier. Le test `a_document_with_several_passages_in_the_top_fills_passages_per_document` ne distingue pas les deux ordres, car `A#1` y est aussi le deuxième candidat global ;
  - MES-02 et MES-03 : le sur-échantillonnage et le `max(N, candidates_per_branch)` déplacent le coût du scan et du rescoring ; les chiffres de la branche dense doivent être lus avec regroupement actif ;
  - toute reprise de l'ordre (`group_by` avant `dedup`, ou suppression du recomplètement) demanderait un nouvel ADR, puisque le critère de la fiche en dépend ;
  - la forme plate rend le rattachement à un document implicite : si un client a besoin d'un objet par document, ce sera un changement de contrat d'API, donc un ADR.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| `group_by` avant `dedup` | La fusion retire ensuite le représentant d'un document, qui disparaît de la réponse : le critère « `k` documents distincts » n'est plus tenu, et le recomplètement après coup coûterait un second parcours sans garantie |
| `group_by`, `dedup`, puis recomplètement depuis les candidats | Un document peut perdre **tous** ses passages à la fusion ; il faudrait alors le remplacer sans savoir s'il lui reste un passage, et l'ordre dépendrait du nombre de doublons, non du score |
| Règle de sur-échantillonnage dans la CLI | Le serveur de P6 et l'éval devraient la réimplémenter ; deux implémentations de la même règle divergent à la première évolution, et la CLI n'est qu'un client du moteur (`docs/07:15`) |
| Compter `k` en entrées de résultat sous `group_by` | Avec `passages_per_document > 1`, `k` entrées peuvent venir d'un seul document, ce qui contredit « on obtient `k` documents distincts » (`P4:79`) |
| Réponse imbriquée : un objet par document, avec la liste de ses passages | Change la forme de `docs/05:177-182`, rompt le client et les tests de P1, et duplique les champs de document à chaque entrée |
| Publier `grouped_under` dans chaque résultat de P4 | C'est un marqueur de diagnostic : il relève d'`explain` et de P7 (`docs/05:216`, `P7:69`), et `doc_id` suffit à regrouper en P4 |
| Garder `max(N, k)` avec le `k` de la requête (texte antérieur de `docs/05:206`) | Divergerait d'ADR-0074 dès que `candidates_per_branch > k` : le scan garderait moins de candidats que le regroupement n'en consomme, et `k` aurait deux sens dans le même document |
| Créer une seconde règle de rescoring pour le regroupement | Deux règles à maintenir pour un même nombre ; ADR-0074 décision 4 définit déjà `k` comme le nombre demandé par la branche dense |
| Rendre une erreur quand les candidats ne suffisent pas à `k` documents | Un petit pool ou un filtre sélectif rendrait une requête valide en erreur ; moins de `k` résultats est déjà le comportement admis d'une recherche ordinaire |

## Références

- Fiche : `docs/phases/P4-recherche-rapide.md:73-82` (`REGROUPER-RESULTATS`).
- Conception : `docs/03-recherche.md:13`, `:52`, `:68`, `:95`, `:107-112` ; `docs/05-api.md:162-166`, `:177-182`, `:197-200`, `:206`, `:216` ; `docs/00-glossaire.md:44` ; `docs/04-donnees-et-ingestion.md:30` ; `docs/07-stack-et-deploiement.md:15` ; `docs/phases/P7-observabilite.md:69`.
- Code : `crates/crible-search/src/group.rs:136` (`candidates_per_branch`), `:157` (`group`), `:189` (`dedup_by_content`), `:213-240` (`keep_documents`) ; `crates/crible-search/src/dense/planner.rs:226` (`search_grouped`) ; `crates/crible-search/tests/regrouper_resultats.rs` ; `crates/crible/src/main.rs:170-179`.
- ADR liés : ADR-0008 (hybride et fusion), ADR-0043 (payload d'un document supprimé), ADR-0074 (planification de la stratégie dense, décisions 3 et 4).
- Question ouverte liée : Q-01 (N par défaut et cible de fidélité du scan binaire, MES-01).
