# ADR-0066 — Dérivation de l'instantané après commit sans balayage complet

- **Statut** : Accepté (décision du propriétaire le 2026-10-26)
- **Date** : 2026-10-26 (proposition et acceptation)
- **Décideurs** : propriétaire
- **Décision** : accepté le 2026-10-26 par le propriétaire, tel que proposé : décision 1 retenue (remède (b), `after_commit` reçoit le delta écrit par la transaction), décision 2 en recours seulement (les deux index SQLite ne sont pas ajoutés maintenant), décision 3 retenue (tâche hors plan `BORNER-COUT-DU-COMMIT` en P3, avant `NETTOYER-P3` et le rapport J3), décision 4 retenue (les chiffres du 2026-10-25 restent, le rejeu ajoute une section datée à MES-15 et à MES-08). Aucun changement de schéma, de `format_version` ni des index de `docs/02` ; INV-15 n'est pas en cause, l'ordre du commit est inchangé (INV-02, INV-03).
- **Proposé par** : agent greffier, sur demande de l'orchestrateur, à la suite de la tâche `MESURER-CYCLE-DE-VIE` (phase P3) et du verdict `NON-COUVERT` de `@architecte` du 2026-10-25
- **Remplace** : —

## Contexte

### Ce que la mesure du 2026-10-25 dit

Le rejeu de MES-15 (`docs/mesures/MES-15-cout-commit-generation.md`, § 10) compare le commit d'une génération avant et après la dérivation incrémentale de la bitmap des vivants (ADR-0048) :

| Passages vivants | `S` | Commit p50 du 2026-10-21 | Commit p50 du 2026-10-25 | Écart |
|---|---|---|---|---|
| 10 000 | 10 | 106,4 ms | 41,1 ms | −61,4 % |
| 100 000 | 10 | 110,1 ms | 71,9 ms | −34,7 % |
| 1 000 000 | 10 | 160,0 ms | 375,8 ms | +134,9 % |
| 8 580 000 | 10 | 598,3 ms | 3 085,1 ms | **+415,6 %** |
| 8 580 000 | 1 000 | 641,3 ms | 3 108,1 ms | +384,6 % |

Source : § 10.3. Le gain du compresseur zstd réutilisé (ADR-0048) explique les deux petits points ; les deux grands points sont dominés par le terme « reste » de la décomposition, qui valait entre +11,4 et −15,1 ms dans la campagne du 2026-10-21 et vaut 350 ms à 1 M et 3 059 ms à 8,58 M.

La dérivation **retire** la lecture de `live_iids` du commit — 517,8 ms à 8,58 M passages vivants, 534,2 ms à `S` = 1 000 — mais **ajoute deux balayages complets** de `passages` par commit, mesurés à **2,92 s** en p50 à 8,58 M (§ 10.5) :

| Requête de `after_commit` | Plan SQLite sur l'index de 8,58 M | p50 sans index | p50 avec index de diagnostic |
|---|---|---|---|
| passages créés (`WHERE p.created_generation = ?`) | `SCAN p` | 489,0 ms | 0,0 ms |
| documents modifiés (`WHERE d.updated_generation = ?`) | `SCAN p` puis `SEARCH d` | **2 432,6 ms** | 0,0 ms |
| pierres tombales (`WHERE dead_since_gen = ?`) | `SEARCH q USING INDEX passages_dead_since_gen` | 0,1 ms | 0,1 ms |
| documents retirés | `SEARCH p USING COVERING INDEX passages_doc_id` | 0,1 ms | 0,1 ms |

Deux des quatre requêtes sont bornées par les index de `docs/02` ; les deux autres ne le sont pas. Le § 10.6 en tire la conclusion : « la dérivation n'est pas en cause dans son principe, mais dans ses requêtes ».

MES-08 (`docs/mesures/MES-08-duree-attache-quotidienne.md`, campagne du 2026-10-25) retrouve le même coût par un autre chemin, celui d'une attache réelle : sur `synth-large` (7,74 M passages vivants), une attache de 1 000 à 3 002 passages dure 3 257 à 3 333 ms, dont **3 207 à 3 219 ms dans l'étape `snapshot`**, soit 96,6 à 98,5 %. Extrapolé à l'échelle de Legifacile, le commit de génération pèse donc l'essentiel d'une attache quotidienne (≈ 3,2 s). Le coût de l'attache est aujourd'hui une fonction du plan d'exécution de deux requêtes, pas du lot attaché : l'étape `snapshot` est plate en taille de lot (+0,4 % de 1 000 à 3 002 passages).

### Où le coût se trouve dans le code

`GenerationStore::commit_with_rows` (`crates/crible-store/src/generation.rs:623`) écrit la transaction, la valide (`transaction.commit()`, point de commit, ADR-0016), puis appelle `build_snapshot_after_commit` (`:1215`). Celui-ci appelle `FilterIndex::after_commit` (`crates/crible-store/src/filters_index.rs:854`), qui dérive les structures de la génération G+1 de celles de G en quatre étapes :

1. `insert_created_passages` (`:999`) — les passages créés, avec les champs de leur document ;
2. `apply_tombstones` (`:1048`) — les passages morts ;
3. `refresh_updated_documents` (`:1066`) — les passages des documents dont les champs ont été réécrits ;
4. `clear_removed_documents` (`:1107`) — les passages des documents retirés.

Les étapes 1 et 3 retrouvent leur ensemble en **relisant SQLite par génération** (`created_generation`, `updated_generation`), et c'est là que les deux `SCAN` naissent. Les étapes 2 et 4 passent par `passages(dead_since_gen)` et par la clé primaire `documents(doc_id)`, donc restent bornées.

Or ce delta est **déjà en mémoire** au moment du commit :

- `GenerationChange` (`crates/crible-store/src/generation.rs:223`) porte `segments`, `documents`, `passages` et `deletions` ; c'est l'entrée de `commit` (`:598`) ;
- `attach::Plan` (`crates/crible-store/src/attach.rs:345`) porte `refresh`, `rewrite`, `declared`, `tombstones`, `removed`, `retired` et `removing` ; c'est ce que la closure de `commit_with_rows` (`:537`) écrit dans la transaction.

Le chemin de l'attache connaît donc déjà les documents dont les passages sont à écrire (`refresh`), ceux dont la ligne `documents` est réécrite (`rewrite`) et ceux qui sont retirés (`removed`). `after_commit` va pourtant les redemander à SQLite, une génération après.

### Ce que la conception demande

- `docs/02-architecture.md:91-107` (« Tables SQLite ») ne prévoit que deux index : `passages(doc_id)` et `passages(dead_since_gen)`. Aucun index ne couvre `created_generation` ni `updated_generation`.
- `docs/02-architecture.md:109-127` (« Générations et point de commit »), **étape 6**, ne demande que de « construire le nouvel instantané de lecture (bitmaps en copie sur écriture) et le publier (`ArcSwap`) ». Rien n'impose de relire ce que la transaction vient d'écrire, ni sous quelle forme.
- **ADR-0016** fait du commit SQLite la **source de vérité de la génération** : il fixe *quand* une génération existe et devient visible. Il ne fait pas de SQLite un journal à rejouer pour retrouver un delta que l'appelant tient déjà en main.
- **ADR-0048** a introduit le rejeu de MES-15 en P3 pour chiffrer la dérivation incrémentale ; ce chiffrage est fait, et il est négatif à 1 M passages et au-delà.
- **INV-02** (une lecture n'est jamais bloquée par une écriture) et **INV-03** (une génération est visible entièrement ou pas du tout, `docs/08:10-11`) sont tenus par le fait que l'instantané est construit **avant** d'être publié ; le remède proposé ne change pas cet ordre.
- **INV-15** (`docs/08:23`) demande un format sur disque versionné. Un index SQLite n'ajoute aucune information au fichier : c'est le raisonnement déjà tenu par ADR-0039 et ADR-0040, qui laissent `format_version` à 1 quand la décision ne change ni le schéma, ni les colonnes, ni les types.

### Le verdict de `@architecte`

Le 2026-10-25, `@architecte` a rendu **NON-COUVERT** : rien dans `docs/01` à `docs/08` ni dans les ADR acceptés ne dit si `after_commit` doit relire l'état commité ou s'il peut recevoir le delta de son appelant ; et la conception ne prévoit pas les index qui rendraient cette relecture bornée. Le choix entre les deux remèdes engage le schéma de `records.sqlite` ou le contrat interne du point de commit : c'est une décision, pas un détail d'implémentation.

## Décisions

Aucune décision n'est prise ici : chacune est soumise au propriétaire. Les quatre décisions sont **séparables** ; le rejet de la décision 1 laisse la décision 2 ouverte en recours, et les décisions 3 et 4 tiennent dans tous les cas.

### Décision 1 — `after_commit` reçoit le delta écrit par la transaction (remède (b), recommandé)

**Proposition.** `after_commit` prend le delta que la transaction vient d'écrire, au lieu de le relire dans SQLite par génération :

- la closure de `commit_with_rows` (`generation.rs:623`) rend, en plus du premier iid libre, ce delta — `GenerationChange` sur le chemin de `commit` (`:598`), `attach::Plan` sur celui de `attach.rs:537`, et la forme équivalente dans le banc `crates/crible-store/benches/commit_cost.rs` (trois appelants) ;
- les quatre étapes de `after_commit` consomment ce delta : les passages créés viennent de `passages` et de `documents` de `GenerationChange`, les pierres tombales de `deletions` et de `tombstones`, les documents à rafraîchir de `refresh` (ou des documents de `GenerationChange`), les documents retirés de `removed` ;
- il reste des accès SQL, mais **bornés** : les passages d'un document par `passages(doc_id)`, les champs d'un document par sa clé primaire `documents(doc_id)`.

Aucun changement de schéma, aucune migration, aucune question INV-15, aucun coût d'ouverture ne sont introduits. Les chiffres de MES-13 (reconstruction à l'ouverture) ne bougent pas.

**Garde-fou.** Le test d'égalité existant — `after_commit` comparé à `FilterIndex::from_records` sur le même état de référence (`filters_index.rs:851`, test d'acceptation de `CONSTRUIRE-BITMAPS` après `L1`, `L2` et `L3`) — reste la preuve que les deux chemins donnent le même instantané. Il est étendu par la décision 3.

**Justification.** L'étape 6 de `docs/02` ne demande que de construire et publier l'instantané ; ADR-0016 fait de SQLite la source de vérité de la génération, pas un journal à relire pour retrouver ce que l'appelant vient d'écrire. Le delta traverse déjà la transaction : le repasser à la construction de l'instantané supprime les deux `SCAN` sans changer ce que le commit garantit. C'est aussi la seule option qui ne fasse pas dépendre la **vitesse** du commit de la présence d'un index : un plan d'exécution peut régresser sans qu'aucun test ne le voie.

**Ce que cette décision ne tranche pas.** Le coût résiduel de la dérivation — copie sur écriture des structures, accès bornés par document, compression des textes — n'est pas chiffré ici : c'est la décision 3 qui le mesure. La décision ne dit pas non plus comment `refresh` et `rewrite` se traduisent en passages : ce détail reste dans `filters_index.rs` et `attach.rs`.

### Décision 2 — En recours seulement : deux index SQLite (remède (a))

**Proposition.** Si la décision 1 ne suffit pas à ramener le commit sous les chiffres du 2026-10-21, ajouter `passages(created_generation)` et `documents(updated_generation)` :

- créés par `CREATE INDEX IF NOT EXISTS` à l'ouverture en écriture (`RecordStore::open`, `crates/crible-store/src/records.rs:252`), et inscrits dans le `SCHEMA` des époques neuves (`records.rs:64`) — le `SCHEMA` emploie déjà cette forme pour les deux index existants (`:105-106`) ;
- sans montée de `format_version` : un index n'ajoute aucune information au fichier (même raisonnement qu'ADR-0039 et ADR-0040) ;
- coût : **≈ 6,4 s une fois par époque existante** (mesuré sur une copie de `data/mes15/i-8580000-s10`, § 10.5), une écriture d'index permanente sur une table de 8,58 M lignes, et une modification de `docs/02` › « Tables SQLite ».

**Pourquoi non retenue maintenant.** Elle laisse la relecture en place : le commit resterait une requête SQL dont le plan peut régresser, et la mesure du 2026-10-25 montre qu'un plan non surveillé suffit à multiplier le coût par cinq. Elle paie en plus une réécriture d'index à l'ouverture (6,4 s par époque, pendant laquelle l'index n'est pas servi) et une écriture permanente, pour une information que l'appelant tient déjà. Elle reste le recours naturel si la mesure de la décision 3 montre que les accès bornés coûtent encore trop.

### Décision 3 — Une tâche `BORNER-COUT-DU-COMMIT`, hors plan, en P3

**Proposition.** Ajouter la tâche `BORNER-COUT-DU-COMMIT` en **P3**, avant `NETTOYER-P3` et avant le rapport J3. Sans elle, J3 validerait une phase dont la mesure d'attache est à +416 % sur le commit, et dont le défaut est identifié mais non corrigé. Elle dépend de `MESURER-CYCLE-DE-VIE`, désormais `FAIT`.

**Critères d'acceptation proposés.**

1. Les deux balayages complets disparaissent : les plans d'exécution du chemin du commit ne portent plus de `SCAN` sur `passages` ni sur `documents`.
2. L'instantané après commit reste égal à celui que reconstruit `from_records` : les tests d'égalité existants, plus un test sur une attache qui mêle mises à jour, suppressions et patch de métadonnées (ADR-0060).
3. Rejeu de MES-15 aux points `N` et `S` des campagnes du 2026-10-21 et du 2026-10-25, et de MES-08 sur `synth-large`, comparés aux deux campagnes.

**Pourquoi une tâche et pas une ligne de plus dans `NETTOYER-P3`.** `NETTOYER-P3` ramasse des points de rédaction et de finition ; celui-ci change la signature du point de commit, touche trois appelants et demande un rejeu de mesure avec un banc. C'est un travail de développement, pas un nettoyage.

### Décision 4 — Les chiffres datés restent, le rejeu ajoute une section datée

**Proposition.** Le § 10 de MES-15 et MES-08 restent tels quels, avec leur date. Le rejeu de la décision 3 ajoute une section datée à chacun des deux rapports, qui cite les campagnes précédentes.

**Pourquoi.** Un rapport de mesure est une pièce datée : réécrire les chiffres du 2026-10-25 effacerait la preuve de la régression et rendrait incomparable la campagne du 2026-10-21. Les trois campagnes se lisent en séquence, chacune disant l'état du code à sa date.

## Conséquences

- **Positives** :
  - le commit cesse de payer la taille de l'index : les 2,92 s du § 10.5 disparaissent du chemin, soit l'essentiel des 3 085 ms mesurés à 8,58 M ;
  - l'attache quotidienne à l'échelle de Legifacile redevient bornée par le lot et par la recette, non par un plan d'exécution ;
  - aucun changement de schéma, aucune migration, aucun `format_version`, aucun coût d'ouverture : INV-15 n'est pas en cause et les index de `docs/02` restent ceux de la conception ;
  - une seule description du delta circule — celle que l'appelant écrit — au lieu d'une seconde, reconstruite par estampille de génération ; une divergence entre les deux ne peut plus exister ;
  - l'ordre du commit ne change pas : transaction, puis instantané, puis manifeste, puis publication (INV-02, INV-03).
- **Négatives / coûts** :
  - **signature interne** : `after_commit`, la closure de `commit_with_rows` et ses trois appelants changent ; le banc `commit_cost.rs` suit, sans quoi la mesure cesserait de mesurer le vrai chemin ;
  - **contrat plus exigeant** : un appelant qui oublierait un document dans le delta produirait un instantané faux **en silence**. Le seul garde-fou est le test d'égalité contre `from_records` ; il faut donc l'étendre aux cas mêlés (décision 3, critère 2) et le tenir à jour à chaque nouveau chemin d'écriture. La décision 1 réduit ce risque en gardant un point d'entrée unique, mais ne le supprime pas ;
  - **coût résiduel non chiffré** : la copie sur écriture des structures et les accès bornés par document ne sont pas mesurés ; le commit après remède n'est pas prédit ici ;
  - **la mesure reste à faire** : tant que MES-15 et MES-08 ne sont pas rejouées, la correction n'est pas prouvée, seulement argumentée ;
  - le remède ne touche ni la recette (≈ 44 s sur l'échantillon, au-delà de `wait_timeout_ms`, proposition distincte de `MESURER-CYCLE-DE-VIE`) ni le reste de l'attache.
- **À surveiller** :
  - le rejeu de MES-15 aux points `N` et `S`, et de MES-08 sur `synth-large` : si le commit à 8,58 M reste au-dessus des 598,3 ms du 2026-10-21, la décision 2 (index) devient le recours à instruire ;
  - l'apparition d'un `SCAN` dans `EXPLAIN QUERY PLAN` sur le chemin du commit : ce serait la régression que la décision 1 vise à rendre impossible, et elle doit être un test, pas une revue ;
  - un nouveau chemin d'écriture qui produirait une génération sans passer par `commit_with_rows` : il n'aurait pas de delta à fournir, et `after_commit` devrait alors être repensé ;
  - le coût d'ouverture d'une époque existante : si la décision 2 était un jour retenue, les 6,4 s par époque se paieraient à chaque ouverture en écriture, donc à chaque démarrage du serveur qui doit écrire.

### Documents à appliquer après acceptation

| Document | Modification, selon les décisions retenues |
|---|---|
| `docs/phases/P3-cycle-de-vie.md` | fiche de la tâche hors plan `BORNER-COUT-DU-COMMIT`, même forme que les tâches hors plan `DURCIR-…` (`:15`, `:35`), placée avant `NETTOYER-P3` : objectif, livrables, critères de la décision 3, mesure de rattachement (`MESURER-CYCLE-DE-VIE`) |
| `docs/02-architecture.md` | seulement si la décision 2 est un jour retenue : § « Tables SQLite » (`:101`), ajouter `passages(created_generation)` et `documents(updated_generation)` |
| `docs/mesures/MES-15-cout-commit-generation.md` | section datée du rejeu, sans toucher au § 10 (décision 4) |
| `docs/mesures/MES-08-duree-attache-quotidienne.md` | section datée du rejeu sur `synth-large`, sans toucher aux chiffres du 2026-10-25 (décision 4) |

Rien n'est appliqué tant que l'ADR est `Proposé`. La proposition d'origine est consignée dans `docs/suivi.md` (« Idées hors périmètre », ligne du 2026-10-25, `MESURER-CYCLE-DE-VIE`).

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| (a) Index `passages(created_generation)` et `documents(updated_generation)` | Garde la relecture et son plan, paie ≈ 6,4 s par époque existante à la première ouverture en écriture, une écriture d'index permanente sur une table de 8,58 M lignes et une modification de `docs/02`, pour une information que l'appelant tient déjà. Retenue seulement en recours (décision 2) |
| Ne rien faire, valider J3 en l'état | J3 validerait une phase dont l'attache quotidienne est à +416 % sur le commit, défaut identifié et documenté : la mesure de P3 servirait de constat, pas de garde |
| Écrire le delta dans une table de journal, puis le relire | Un format sur disque de plus (INV-15, reprise, compaction), et une relecture de plus, pour un delta qui n'existe que le temps d'un commit |
| Reconstruire l'instantané par `from_records` à chaque commit | Plus coûteux encore : 20,12 s p50 à 8,58 M passages vivants mesurés par MES-13 (`docs/mesures/MES-13-temps-de-demarrage.md`, § 3), et un balayage complet de plus, pas de moins |
| Décaler la dérivation hors du chemin du commit (asynchrone) | Soit l'instantané est publié avant d'être complet, ce qui viole INV-03, soit la génération attend, ce qui revient au même coût en différé et déplace le problème vers la file d'écriture |
| Deux index partiels (`WHERE created_generation IS NOT NULL`) | Même coût et mêmes effets que (a), avec une condition à maintenir dans chaque requête ; aucun gain mesuré |
| Modifier la conception (`docs/02`) pour prescrire la relecture par génération | La conception décrit un point de commit, pas la façon de dériver l'instantané ; figer la relecture transformerait un détail d'implémentation en contrat et rendrait toute optimisation ultérieure contraire à la conception |

## Références

- `docs/mesures/MES-15-cout-commit-generation.md` : campagne du 2026-10-21 (§ 1 à 6), rejeu du 2026-10-25 (§ 10, résultats § 10.3, relevé des quatre requêtes § 10.5, conclusion § 10.6)
- `docs/mesures/MES-08-duree-attache-quotidienne.md` : part du commit dans l'attache (`:150-160`), conclusion (`:240-256`)
- `docs/02-architecture.md` : tables SQLite et index (`:91-107`), générations et point de commit (`:109-127`), patch de métadonnées (ADR-0060)
- `docs/08-invariants-et-tests.md` : INV-02, INV-03 (`:10-11`), INV-15 (`:23`), MES-08 (`:73`), MES-15 (`:80`)
- `crates/crible-store/src/filters_index.rs` : dérivation (`:854`), `insert_created_passages` (`:999`), `apply_tombstones` (`:1048`), `refresh_updated_documents` (`:1066`), `clear_removed_documents` (`:1107`), égalité avec `from_records` (`:851`)
- `crates/crible-store/src/generation.rs` : `GenerationChange` (`:223`), `commit` (`:598`), `commit_with_rows` (`:623`), `build_snapshot_after_commit` (`:1215`)
- `crates/crible-store/src/attach.rs` : `Plan` (`:345`), appel de `commit_with_rows` (`:537`)
- `crates/crible-store/src/records.rs` : `SCHEMA` (`:64`), index existants (`:105-106`), `open` (`:252`)
- `crates/crible-store/benches/commit_cost.rs` : banc de MES-15
- ADR-0016 (point de commit SQLite), ADR-0018 et ADR-0019 (structures de filtrage dérivées), ADR-0039 et ADR-0040 (`format_version` inchangé quand la décision n'ajoute aucune information), ADR-0048 (rejeu de MES-15 en P3 et compresseur zstd réutilisé, qui porte la dérivation incrémentale), ADR-0060 (patch de métadonnées)
