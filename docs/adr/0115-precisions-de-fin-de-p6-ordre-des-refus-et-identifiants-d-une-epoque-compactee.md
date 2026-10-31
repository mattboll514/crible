# ADR-0115 — Précisions de fin de P6 : ordre des refus de `POST documents` et identifiants d'une époque compactée

- **Statut** : Accepté (accepté par anticipation par le propriétaire le 2026-10-31)
- **Date** : 2026-10-31
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, tâche hors plan NETTOYER-P6 (phase P6, branche `tache/NETTOYER-P6`), passage B, sur délégation du propriétaire du 2026-10-31
- **Remplace** : — (ne remplace aucune décision : complète ADR-0107 et ADR-0108 sans modifier le corps d'aucun d'eux)

## Contexte

Les deux décisions ci-dessous écrivent un comportement **déjà livré**. Aucune ne change le code : la première fige par un test un ordre de refus que la documentation laisse implicite, la seconde fait entrer dans `docs/02` deux formes d'identifiant qui ne vivent aujourd'hui que dans un commentaire de module. Elles viennent du contrôle de fin de phase P6, qui a relevé deux points de relecture restés ouverts.

### Décision 1 : deux refus de `POST /v1/indexes/{index}/documents` et leur ordre

`docs/05-api.md` décrit deux refus distincts du corps d'un lot en ligne, avant toute mise en file :

- `docs/05-api.md:156` — **corps vide** : un corps qui ne porte ni `documents` ni `deletions` rend `400 invalid_request`, avec un message qui nomme les deux champs attendus (complément du 2026-10-29 à ADR-0107) ;
- `docs/05-api.md:159` — **mode** : `replace_all` est refusé avec `invalid_request`, et le message nomme le mode (ADR-0055).

Aucun texte ne dit lequel des deux l'emporte quand un corps porte **les deux fautes**. Par exemple, `{"batch_id":"x","mode":"replace_all"}` n'a ni `documents` ni `deletions`, et porte un mode refusé.

Le point vient de la relecture de ROUTES-DOCUMENTS-ET-LOTS (`docs/suivi.md:398`) : le refus du corps vide passait avant le contrôle du `mode`, donc ce corps ne nommait pas le mode, alors que `docs/05` le demande. Le passage B de NETTOYER-P6 (commit `37651cb`) a corrigé l'ordre et l'a figé par un test.

Ce que le code fait aujourd'hui :

- `crates/crible-server/src/documents.rs:326`, fonction `check_online`, contrôle le `mode` en premier (`:330`), puis le corps vide (`:341`), puis `batch_id` et le compte d'entrées (`:347` et suivantes) ;
- `crates/crible-server/tests/routes_documents.rs:411`, test `the_shape_of_an_online_body_is_refused_before_the_job`, couvre le cas `{"batch_id":"batch-shape","mode":"replace_all"}` (`:445`, cas listé `:453-454`) et vérifie que le message nomme le mode.

Un ordre implicite ne suffit pas. Un client ne peut pas s'y fier, et un test qui fige un comportement non écrit ne protège que contre une régression accidentelle : il n'autorise personne à décider si un futur changement d'ordre est une correction ou une rupture.

### Décision 2 : les identifiants d'une époque compactée

La compaction crée une époque E+1 dont les segments portent des identifiants qui n'existent dans aucune attache :

- `crates/crible-store/src/compact.rs:674`, fonction `compaction_batch_id`, rend un `batch_id` de la forme `compaction-e<E>-e<E+1>`, écrit dans `segment.json` et dans la colonne `segments.batch_id` de l'époque E+1 (`:408`) ;
- `crates/crible-store/src/compact.rs:666`, fonction `compacted_segment_id`, rend un `segment_id` de la forme `c<E>-<iid_start>`, écrit au même endroit (`:434`) et servant aussi de nom de dossier sous `segments/`.

Deux précisions que le commentaire de module ne donne pas, et qui comptent pour un lecteur :

- `iid_start` est écrit sur **dix chiffres, zéros de tête compris** : la forme exacte d'un `segment_id` est `c<E>-<iid_start:010>`, par exemple `c3-0000000000` (test `compacted_segment_ids_are_deterministic_and_safe`, `crates/crible-store/src/compact.rs:1027`, et `crates/crible-store/tests/inv_04_crash_recovery.rs:228`) ;
- `batch_id` ne correspond à **aucune ligne de `batches`** : une compaction n'est pas une attache, donc le contrôle de rejeu d'INV-10 ne s'y applique jamais. L'historique de `batches` est recopié à l'identique depuis E, ce qui garde le rejeu d'un ancien lot sans effet.

Ces formes ne vivent aujourd'hui que dans le commentaire de module de `compact.rs:48` (« Identifiers of a compacted epoch »), et `docs/02-architecture.md` n'en dit rien : la table `segments` (`docs/02-architecture.md:101`) ne liste que les noms de colonnes. Le point vient de la relecture de COMPACTER-INDEX (`docs/suivi.md:562`, 2026-10-25) et a été relevé au contrôle de fin de phase de P6 par `@architecte`. ADR-0061 les mentionne déjà (`docs/adr/0061-…:54`), preuve que le fait est connu mais non documenté à sa place.

## Décision

**1. Dans `POST /v1/indexes/{index}/documents`, le contrôle du `mode` précède le contrôle du corps vide.** Un corps qui cumule les deux fautes rend le `400 invalid_request` qui nomme le mode. L'ordre complet des contrôles du corps reste celui de `check_online` : `mode`, corps vide, `batch_id`, compte des entrées, `doc_id` à la fois écrit et supprimé, membre inconnu.

`docs/05-api.md` reçoit une phrase, près des lignes 156 et 159, qui écrit cet ordre et cite ADR-0115.

**2. `docs/02-architecture.md` décrit les deux formes d'identifiant d'une époque compactée**, dans la description de la table `segments` : `segment_id` vaut `c<E>-<iid_start>` avec `iid_start` sur dix chiffres, et `batch_id` vaut `compaction-e<E>-e<E+1>`. Le texte dit aussi que ce `batch_id` ne correspond à aucune ligne de `batches` et sert de marqueur de provenance, non de clé étrangère.

**Ce qui n'est pas décidé ici.** Le présent ADR ne change ni le code, ni les tests, ni `docs/suivi.md`. Il ne touche pas les autres refus du corps (membre inconnu, `doc_id` écrit et supprimé, limite de 10 000 entrées) ni l'ordre des contrôles de la route de recherche (ADR-0109, ADR-0110). Il ne change aucun format sur disque : les deux formes d'identifiant étaient déjà écrites, elles sont seulement documentées.

## Conséquences

- **Positives** :
  - un client sait ce qu'il reçoit pour un corps qui cumule les deux fautes, et l'ordre est écrit là où il lit l'API ;
  - le test de `routes_documents.rs` cesse de figer une règle non écrite : il défend désormais un contrat, et un changement d'ordre devient une décision visible ;
  - un lecteur de `docs/02` reconnaît un segment compacté sans ouvrir `compact.rs`, et sait que son `batch_id` ne se joint pas à `batches` ;
  - la forme exacte du `segment_id` (dix chiffres, zéros de tête) entre dans la documentation, ce qui évite la lecture littérale de `c<E>-<iid_start>` de `docs/suivi.md:562`.
- **Négatives / coûts** :
  - deux paragraphes de documentation de plus, donc deux endroits de plus à tenir si l'un des comportements change ;
  - l'ordre des refus devient un engagement public : le changer demandera un nouvel ADR, alors qu'il s'agissait jusqu'ici d'un détail d'implémentation.
- **À surveiller** : si un futur refus du corps est ajouté à `check_online`, sa place dans l'ordre est une décision, pas un hasard de rédaction ; et si les formes d'identifiant de la compaction changent, `docs/02` et le commentaire de module de `compact.rs` doivent changer ensemble, avec un nouvel ADR.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Laisser l'ordre implicite | Un client ne peut pas s'y fier, et le test fige un comportement que rien n'écrit : la prochaine session ne saurait pas distinguer une correction d'une régression. |
| Contrôler le corps vide d'abord, et ne nommer le mode que si le corps en porte un | Contredit le point de relecture d'origine (`docs/suivi.md:398`) et `docs/05-api.md:159`, qui demandent que le refus nomme le mode : un client qui envoie `{"mode":"replace_all"}` lit « il manque des documents », alors que le mode suffit déjà à refuser la requête. |
| Laisser les identifiants dans le seul commentaire de module (`compact.rs:48`) | Un lecteur de `docs/02` ne saurait pas reconnaître un segment compacté, et lirait `segments.batch_id` comme une clé vers `batches` : la jointure est fausse après une compaction. |

## Références

- `docs/05-api.md:156` (corps vide), `docs/05-api.md:159` (mode), `docs/02-architecture.md:101` (table `segments`).
- `crates/crible-server/src/documents.rs:326` (`check_online`), `crates/crible-server/tests/routes_documents.rs:411` (test du corps en ligne).
- `crates/crible-store/src/compact.rs:48` (commentaire de module), `:666` (`compacted_segment_id`), `:674` (`compaction_batch_id`), `:1027` (test des identifiants) ; `crates/crible-store/tests/inv_04_crash_recovery.rs:228`.
- `docs/suivi.md:398` (relecture de ROUTES-DOCUMENTS-ET-LOTS), `docs/suivi.md:562` (relecture de COMPACTER-INDEX, 2026-10-25).
- ADR-0107 (écriture de documents et de lots par l'API, corps sans document ni suppression), ADR-0055 (lot en ligne et mode `upsert`), ADR-0108 (cohérence de l'époque après une compaction), ADR-0061 (manifeste d'un lot exporté, qui cite déjà `compaction-e<E>-e<E+1>`), ADR-0109 et ADR-0110 (ordre des validations des routes de recherche, hors périmètre ici).
