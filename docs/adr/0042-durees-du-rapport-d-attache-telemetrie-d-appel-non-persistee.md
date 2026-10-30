# ADR-0042 — Durées du rapport d'attache : télémétrie d'appel, non persistée

- **Statut** : Accepté
- **Date** : 2026-10-22 (proposition et acceptation)
- **Décideurs** : propriétaire
- **Décision** : accepté le 2026-10-22 par le propriétaire, option B telle que proposée, sans modification.
- **Proposé par** : agent greffier, sur demande de l'orchestrateur, au 3e tour de relecture de la tâche ATTACHER-LOT (phase P1), après le verdict **ECART** de `@architecte`
- **Remplace** : —

## Contexte

`docs/04-donnees-et-ingestion.md:197-200` demande que le rapport d'attache porte « la durée de chaque étape ». Le même document exige, dix lignes plus haut, qu'un `batch_id` déjà appliqué « renvoie le rapport d'origine » (`:162`, INV-10), et rappelle qu'il n'y a « pas de seconde écriture après le point de commit (ADR-0016) » (`:159`). Les deux exigences ne peuvent pas tenir ensemble telles quelles.

La raison est dans l'ordre des étapes. `docs/02-architecture.md:106-113` décrit l'attache en six étapes, dont cinq sont mesurées en P1 : `validation` (étape 1, contrôles et plan), `segment` (étape 2, écriture et renommage du segment), `transaction` (étape 3, qui se termine au COMMIT), `manifest` (étape 5, écriture de `manifests/<G+1>.json`) et `snapshot` (étape 6, construction et publication de l'instantané de lecture). L'étape 4 (tantivy) n'existe qu'à partir de P5. Or la ligne `batches`, donc `batches.report`, est écrite dans la transaction, avant le COMMIT. À cet instant, seules `validation` et `segment` sont terminées ; `transaction` ne se termine qu'au COMMIT, `manifest` et `snapshot` après. Trois des cinq durées sont donc inconnaissables quand le rapport est figé.

Deux décisions `Accepté` bornent le problème. L'ADR-0016 fait de la transaction SQLite le seul point de commit, sans écriture visible après lui. La fiche ATTACHER-LOT (`docs/phases/P1-stockage-et-recherche-exacte.md:113`) exige que toutes les écritures SQLite d'une attache, `batches` compris, passent dans une seule transaction, sans autocommit. L'ADR-0041 a explicitement laissé les durées hors de son périmètre (`:85`).

Une implémentation intermédiaire de la branche `tache/ATTACHER-LOT` complète `batches.report` par un `UPDATE` après le COMMIT (`crates/crible-store/src/attach.rs:513-527`, `update_batch_report`). Cette écriture contredit l'ADR-0016, la fiche et `docs/04:159`. Elle n'est pas réparée par la reprise : `recover()` ne touche pas `batches`. Un arrêt entre le COMMIT et l'`UPDATE` laisserait donc un rapport persisté sans durées, et un rejeu rendrait un rapport différent de celui de l'attache d'origine. Le commentaire de module qui la justifie (`attach.rs:32-35`) affirme pourtant que le rejeu rend le rapport d'origine « avec ses durées ». Cette écriture disparaît dans tous les cas, quelle que soit l'option retenue.

Enfin, `docs/05-api.md:215-220` montre déjà un rapport persisté sans aucune durée : les six comptes d'ADR-0041, `acceptance` et `generation`. C'est le seul exemple complet du projet, et il est compatible avec la décision recommandée.

## Décision

**Le rapport persisté, `batches.report`, porte les six comptes d'ADR-0041, les statistiques de recette (P2) et la génération. Les durées par étape sont de la télémétrie d'appel : elles sont rendues au seul appelant d'origine de l'attache et ne sont jamais persistées.**

Précisions :

- un rejeu (INV-10) rend le rapport persisté de l'attache d'origine, sans durées ; le rapport rejoué est identique, clé pour clé, à celui qu'un rejeu précédent aurait rendu ;
- les cinq durées de P1 (`validation`, `segment`, `transaction`, `manifest`, `snapshot`) restent mesurées, comme aujourd'hui, et restent disponibles pour l'appelant qui vient de lancer l'attache ainsi que pour la journalisation et les mesures ;
- aucune durée n'entre dans `batches.report` : ni les deux connues avant le COMMIT, ni les trois autres. Aucune écriture n'a lieu après le COMMIT ;
- `result.report` d'un job d'attache est le rapport persisté (`docs/05-api.md:215-220`), donc sans durées. L'ADR ne tranche pas la forme exacte sous laquelle un appelant reçoit ses durées : c'est un point d'API, hors du périmètre de cette décision ;
- INV-10 (`docs/08-invariants-et-tests.md:18`) n'est pas reformulé : il ne parle que de génération et de comptes. Son test vérifie que le rapport rejoué est identique au rapport persisté, ce qui couvre l'absence de durées ;
- la durée d'une attache reste observable après coup par la différence `finished_at - started_at` du job, qui n'est pas un rapport et ne prétend pas découper les étapes.

Sont hors du périmètre de cet ADR :

- le contenu des statistiques de recette, fixé par `docs/04:176-180` et implémenté en P2 ;
- les durées des autres opérations (compaction, export, instantané), qui n'ont pas de ligne `batches` à écrire ;
- la forme de la télémétrie côté API et côté journal, au-delà de l'interdiction de persistance.

## Conséquences

- **Positives** :
  - `batches.report` devient une valeur écrite une fois, dans la transaction du commit de génération : plus de seconde écriture, donc plus d'écart possible entre le rapport d'origine et le rapport rejoué ;
  - l'ADR-0016, la fiche ATTACHER-LOT et `docs/04:159` disent enfin la même chose, et `docs/04` cesse de se contredire ;
  - le rapport persisté a un schéma unique, que l'appel direct et le rejeu rendent à l'identique, ce qu'un test peut vérifier sans cas particulier ;
  - `docs/05-api.md:215-220` n'a plus besoin d'être corrigé sur ce point : l'exemple montre déjà le bon rapport ;
  - l'`UPDATE` post-commit disparaît, avec le risque d'arrêt qu'il portait : un rapport persisté est toujours complet.
- **Négatives / coûts** :
  - les durées par étape ne sont plus consultables après coup : après un redémarrage, ou pour une attache passée, il ne reste que la durée totale du job. Diagnostiquer une attache lente demande alors de rejouer la mesure ou d'activer la journalisation ;
  - un appelant qui veut les durées doit les recevoir de son propre appel ; un client qui lit un job plus tard n'y trouvera pas les durées ;
  - `docs/04:197-200` doit être réécrit, et `docs/05` gagne une phrase, au prix d'un écart de moins mais d'une information de moins dans le rapport.
- **À surveiller** :
  - une demande d'exploitation qui voudrait expliquer une attache passée par ses étapes, ou comparer les durées de deux attaches dans le temps : elle rouvrirait la décision, et la réponse serait probablement une mesure ou une table de télémétrie séparée, pas un rapport persisté ;
  - le volume d'un `batches.report` s'il grossit encore (recette, comptes) : le rapport reste une petite valeur JSON, mais c'est une ligne de la transaction de commit ;
  - la première mesure de latence d'attache en P1, si `transaction` domine le coût : elle montrerait que le découpage des durées a une valeur d'analyse, sans changer la décision sur leur persistance.

## Application

Après acceptation seulement, et dans un commit qui cite `ADR-0042` :

- `docs/04-donnees-et-ingestion.md:197-200` : le rapport persisté porte les comptes, les statistiques de recette et la génération ; les durées par étape sont rendues au seul appelant, jamais persistées ;
- `docs/04-donnees-et-ingestion.md:162` : « renvoie le rapport persisté de l'attache d'origine » ;
- `docs/05-api.md:215-225` : mention que `result.report` est le rapport persisté et ne porte pas de durées ;
- `docs/phases/P1-stockage-et-recherche-exacte.md` (tâche ATTACHER-LOT) : la fiche ne demande pas de durées persistées ; le livrable « rapport d'attache » vise le rapport persisté ;
- `docs/suivi.md`, section « Écarts à la conception » : ligne citant `ADR-0042`, à ajouter par l'orchestrateur ;
- code, dans la tâche ATTACHER-LOT : supprimer `store.update_batch_report` et son appel (`crates/crible-store/src/attach.rs:513-527`), retirer `durations` de la sérialisation du rapport et de sa relecture (`attach.rs:130-249`), corriger le commentaire de module (`attach.rs:32-35`), et adapter les tests qui comparent le rapport d'un rejeu à celui de l'attache d'origine.

ATTACHER-LOT reste bloquée jusqu'à la décision.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| **A.** Persister les seules durées connues avant le COMMIT, `validation` et `segment`. | Le rapport ne porterait qu'un sous-ensemble arbitraire de deux étapes sur cinq, choisi par la position du COMMIT et non par l'intérêt de la mesure. Le schéma du rapport dépendrait du chemin : un appel direct pourrait rendre les cinq durées, un rejeu seulement deux, ou un rapport à trous. Un client ne saurait pas si une clé absente signifie « non mesurée » ou « pas encore connue ». Le gain d'information est faible, et il ne supprime pas la différence entre le rapport rendu en direct et celui rendu au rejeu. |
| **C.** Faire écrire la ligne `batches` par le commit juste avant le COMMIT, pour y inclure la durée de la transaction. | La durée mesurée exclurait le COMMIT lui-même et l'essentiel du coût `fsync` : elle serait trompeuse, et d'autant plus courte que le commit est cher. `manifest` et `snapshot` resteraient dehors, donc le problème ne serait que déplacé. La variante qui écrit un fichier de rapport hors SQLite après le COMMIT échangerait un `UPDATE` contre un nouveau format sur disque, non reconstruit par la reprise et non couvert par la transaction : une seconde source de vérité, contraire à l'esprit de l'ADR-0016 et au principe « point de commit unique ». |

## Complément du 2026-10-29

Un renvoi, sans changement de la décision ci-dessus.

**Statut du complément** : confirmé par le propriétaire le 2026-10-30, sans changement de la décision ci-dessus.

**Constat.** La décision laisse ouverte « la forme exacte sous laquelle un appelant reçoit ses durées » (`:31`), en la qualifiant de point d'API.

**Décision.** ADR-0107, décision 4, la tranche pour le serveur : aucune réponse de l'API ne rend les durées, `?wait=true` compris ; le job les journalise une fois, à sa fin, au niveau `info`, avec `job_id`, `kind` et `index`, sans texte client ; elles ne sont ni persistées ni portées par le job. La CLI continue de les afficher à son appelant.

**Raison.** Un job est asynchrone et peut être relu par d'autres que son auteur ; la télémétrie d'exploitation est le journal, et les métriques de P7.

## Références

- `docs/04-donnees-et-ingestion.md` : `:159` (pas de seconde écriture après le point de commit), `:162` (idempotence, rapport d'origine, INV-10), `:176-180` (recette), `:184-200` (rapport d'attache).
- `docs/02-architecture.md:104-113` : générations, point de commit et six étapes de l'attache.
- `docs/05-api.md:212-225` : exemple de job d'attache et de `result.report`.
- `docs/08-invariants-et-tests.md:18` (INV-10).
- `docs/phases/P1-stockage-et-recherche-exacte.md:104-122` : fiche ATTACHER-LOT, `:113` pour la transaction unique.
- `crates/crible-store/src/attach.rs:32-35` (commentaire de module sur les durées persistées), `:130-249` (`AttachDurations`, sérialisation et relecture), `:513-527` (`UPDATE` post-commit).
- `crates/crible-store/src/generation.rs:418-450` : mesure de `transaction`, `manifest` et `snapshot`, disponible seulement après le COMMIT.
- ADR-0016 : le commit SQLite est l'unique point de commit.
- ADR-0041 : sémantique d'attache, six comptes du rapport ; durées explicitement hors de son périmètre (`:85`).
