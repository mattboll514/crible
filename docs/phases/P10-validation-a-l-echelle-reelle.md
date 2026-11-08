# P10 — Validation à l'échelle réelle

**But** : valider Crible sur la vraie base de Legifacile — six configs d'index, export de migration, import complet, jeu de référence, comparaison avec pgvector, mesure sur le Mac mini — puis écrire le bilan. C'est la dernière phase de la **v1** ; le plan continue ensuite avec P9, après J10 (ADR-0136).

**Jalon J10 — fin de la v1** (ADR-0129, décision 7 ; ADR-0136 distingue la fin de la v1 de la fin du plan, qui est J9) :

1. Les **six configs** d'index sont définies et validées par le propriétaire.
2. Les **8,58 M passages** sont importés ; les comptes par index sont égaux à pgvector ou les écarts sont expliqués ; MES-07 et MES-09 sont faites à l'échelle réelle.
3. Le **jeu de référence** est validé et identique pour pgvector et Crible.
4. **Rappel** : au moins **0,482** sur les 56 questions satisfaisables (`Q30` à part) et les cibles par corpus (`juri_texte_integral` ≥ 0,290, `textes_vigueur` ≥ 0,750). **Fidélité** : au moins **0,99** à 8,58 M passages, au N par défaut (5000, ADR-0078).
5. **Latence p95** : contraignante (≤ 698 ms, MES-00) seulement sur le Mac mini en conteneur, par `MESURER-SUR-MAC-MINI` ; indicative ailleurs.
6. Le **bilan de Crible** est écrit et validé.

La **tâche quotidienne stable 14 jours** et le **retour arrière testé** ne conditionnent plus J10 : ils partent au projet Legifacile (`docs/passation-legifacile.md`), qui a son propre bilan (ADR-0129, décision 7).

**Machines et rôles** (ADR-0129, décision 2) :

- l'import complet et la comparaison avec pgvector se font sur la **machine de développement** (MacBook Air M3, `Mac15,12`, 8 cœurs, 24 Gio), sous Docker Desktop ; leurs mesures de latence sont **indicatives** ;
- la **latence contraignante** se mesure sur le **Mac mini M1 16 Go**, en conteneur, après installation de Crible par le propriétaire ; la base y arrive par **instantané puis restauration** (ADR-0064, ADR-0065), sans réimport ni ré-encodage ;
- la **production officielle** n'est pas actée ; ce sera probablement un serveur OVH, hors v1.

**Lecture seule pendant les mesures** (ADR-0129, décision 9, complément à ADR-0032). Sur le Mac mini, les mesures n'exécutent que des **commandes de lecture** de Crible (`search`, `inspect`, `export`, `stats`, `eval` local). Aucune écriture sous le dossier de données de Crible ni dans le dépôt Legifacile ; les fichiers temporaires vont hors de ces deux arbres et les rapports sont rapatriés dans `docs/mesures/`. Une mesure qui exigerait une écriture d'index (attache) se fait sur un **dossier de données distinct**, jamais sur l'instance servie. Aucun agent n'arrête, ne redémarre ni ne lance `docker stop`, y compris pour libérer le `WRITER_LOCK`.

---

### DEFINIR-INDEX-LEGIFACILE
- **Objectif** : les six configs d'index définitives.
- **Dépend de** : J8 validé (ADR-0136 ; ADR-0129 écrivait J9 validé)
- **À lire** : `02-architecture.md` (découpage validé) ; `04-donnees-et-ingestion.md` (configuration). Dans Legifacile : les vues `vec.src_*`, `scripts/db/sync_vecteurs.py` et `serveur_mcp/outils.py` (filtres exposés).
- **Livrables** :
  - `articles`, `jorf`, `decisions`, `titres`, `sarde` et `dole`, avec leurs champs, leurs types et leur niveau ;
  - `cluster_by` pour chaque index ;
  - pour chaque index, le gabarit à garder ou à changer ;
  - la réponse à Q-03 (champs multivalués) ;
  - un tableau de correspondance entre les filtres actuels du MCP et les filtres Crible (`fonds`, `etat` → dates, `annee`, `cle_domaine`, `partie_exclue`, `dedupliquer`).
- **Critères d'acceptation** :
  - chaque filtre actuel du MCP a son équivalent ;
  - chaque config est validée par `crible-core` ;
  - le propriétaire valide les six configs.

### EXPORTER-BASE-LEGIFACILE
- **Objectif** : exporter les six index de Legifacile en lots au format v1, depuis les vecteurs pgvector existants, sans ré-encodage.
- **Dépend de** : DEFINIR-INDEX-LEGIFACILE
- **À lire** : `04-donnees-et-ingestion.md` (producteurs, format de lot, export) ; `format-lot-v1.md` ; ADR-0010 (découpage chez le producteur), ADR-0012, ADR-0036 (`producer.host`), ADR-0039, ADR-0040, ADR-0032 (lecture seule), ADR-0129 (décision 6). Précédent : `CONSTITUER-ECHANTILLON-LEGIFACILE` (P2), dont la fiche décrit déjà la reprise des vecteurs et la vérification des textes.
- **Livrables** :
  - un script ponctuel dans `legifacile-sandbox`, écrit et exécuté **avec le propriétaire** ;
  - en **lecture seule** sur le Mac mini (ADR-0032, ADR-0129 décision 9) : aucune écriture dans le dépôt Legifacile, aucun geste d'exploitation ; les lots et les fichiers temporaires sont écrits **hors** du dossier de données de Crible et du dépôt Legifacile ;
  - des lots au format v1, un jeu `replace_all` par index, écrits avec `crible.BatchWriter` ;
  - des textes **recomposés** avec le compositeur de Legifacile et vérifiés contre le sha256 `src_hash` existant ;
  - des vecteurs repris de pgvector, sans GPU et sans nouvel encodage ;
  - `producer.name` et `producer.host` renseignés (ADR-0036), `chunker` et `template` déclarés, `doc_hash` cohérent avec `src_hash`.
- **Critères d'acceptation** :
  - chaque lot passe la validation du format et la recette ;
  - 100 % des textes recomposés correspondent à `src_hash`, ou les écarts sont comptés et expliqués ;
  - les six index ont leur jeu de lots `replace_all` ;
  - aucune écriture n'a eu lieu sous le dossier de données de Crible ni dans le dépôt Legifacile.
- **Invariants / mesures** : INV-11, INV-12 (contrôles à l'attache des lots exportés) ; MES-04.
- **Hors périmètre** : un mode « import pgvector » dans Crible (ADR-0129, décision 6) ; toute revectorisation sur GPU ; le producteur quotidien, qui appartient au projet Legifacile.

### CORRIGER-BLOCAGE-APRES-COMMIT
- **Origine** : ajoutée par le propriétaire le 2026-11-06 (« oui ajoute la tâche de correction »), hors plan initial. Elle vient de `EXPORTER-BASE-LEGIFACILE` : journal de `docs/suivi.md` des 2026-11-05 et 2026-11-06, et « Idées hors périmètre » du 2026-11-06. `crible attach` d'un gros lot `replace_all` dans un index vide atteint son **point de commit** — 57 s pour `titres` (1 477 883 passages), 174 s pour `jorf` (1 833 500), 297 s pour `articles` (2 476 715), 448 s pour `decisions` (3 589 776) — puis **ne rend pas la main** : le processus est encore vivant 10 minutes après, et il est arrêté par `SIGTERM`. Les piles relevées par `sample` placent 96 à 98 % des échantillons en temps propre dans `FieldData::clear`, en partie dans `roaring::Container::contains`, sous `FilterIndex::after_commit`, appelé par `build_snapshot_after_commit` (`crible-store`). Le lot reste cohérent : à la réouverture, Crible reconstruit le manifeste et rejoue le lot (`already applied`) ; une attache en génération 2 sans changement rend la main normalement. L'hypothèse d'un coût quadratique du nettoyage des champs au premier commit de nombreux documents est **non prouvée**. À 9,6 M passages, l'import complet resterait bloqué des heures : cette tâche passe donc avant `IMPORTER-BASE-COMPLETE`.
- **Objectif** : supprimer le blocage qui suit le point de commit, en corrigeant sa **cause**, sans changer le résultat des filtres ni le protocole de commit.
- **Dépend de** : EXPORTER-BASE-LEGIFACILE (les six lots de `data/legifacile-full/` et les piles relevées existent)
- **À lire** : `adr/0048-rejeu-de-mes-15-en-p3-et-compresseur-zstd-reutilise.md` (coût d'un commit de génération, rejeu de MES-15) ; `adr/0066-derivation-de-l-instantane-apres-commit-sans-balayage-complet.md` (`after_commit` reçoit le delta écrit par la transaction, les accès SQL restants sont bornés) ; `02-architecture.md` (« Générations et point de commit », INV-03 et INV-04) ; `03-recherche.md` (filtres et structures dérivées) ; `08-invariants-et-tests.md` (INV-04, INV-07) ; `crates/crible-store/src/filters_index.rs` (`impl FieldData` vers `:221`, `fn clear` vers `:301`, `fn after_commit` vers `:920`, `clear_document_fields` vers `:1053`, `clear_removed_documents` vers `:1136`) ; les piles `data/legifacile-full/verif/attach-{jorf,articles,decisions}-sample.txt`, gitignorées et conservées.
- **Livrables** :
  1. un **diagnostic, avant toute correction, écrit dans le compte rendu de `@dev`**, qui dit :
     - la cause exacte : laquelle des quatre étapes d'`after_commit` porte le coût, quel appel à `FieldData::clear` la produit, et pourquoi le temps propre se concentre dans `roaring::Container::contains` ;
     - la **complexité** de ce nettoyage en fonction du nombre de passages et du nombre de valeurs distinctes des champs de document, avec le facteur qui la rend quadratique aux tailles observées ;
     - pourquoi la génération 1 d'un gros `replace_all` déclenche le coût alors qu'une attache en génération 2 sans changement rend la main normalement ;
     - ce qui est mesuré et ce qui reste supposé ; si la preuve demande une micro-mesure, son protocole et ses chiffres bruts sont dans le compte rendu.
  2. la correction, à l'endroit de la cause :
     - sans changer le **résultat des filtres** : les structures dérivées restent égales à celles de `FilterIndex::from_records` sur le même état de référence ;
     - sans changer le **protocole de commit** de `02-architecture.md` : ordre des étapes, point de commit unique, publication de l'instantané avant qu'il ne soit servi (INV-03, INV-04) ;
     - sans changer le schéma de `records.sqlite`, `format_version`, ni les index de `docs/02` ; sans dépendance nouvelle.
  3. un test de non-régression **non flaky** : aucun seuil de temps en CI ; par exemple un test qui borne le nombre d'opérations, ou un test de la complexité sur des tailles croissantes, au choix et à la main de `@dev`. Le test doit échouer sur le code d'avant la correction et passer après. Les tests **existants** des filtres et du commit restent verts et inchangés.
  4. une **preuve à l'échelle** : rejouer `crible attach` d'un lot réel de `data/legifacile-full/` dans un index jetable, au moins `jorf`, et `decisions` si le temps le permet, avec le **temps entre le point de commit et la fin du processus**. Comparer aux 174 s et 448 s relevés, et vérifier que le processus rend la main sans `SIGTERM` et que l'index reste cohérent à la réouverture.
  5. si le diagnostic révèle un comportement que la conception ne tranche pas, une `QUESTION-CONCEPTION` : ADR-0066 fixe la dérivation après le commit, `docs/02` et `docs/03` fixent les structures dérivées et l'ordre du commit ; toute autre voie demande une décision.
- **Critères d'acceptation** :
  - le diagnostic précède la correction dans le compte rendu, nomme la cause exacte et sa complexité, et ne présente pas l'hypothèse quadratique comme prouvée avant de l'avoir établie ;
  - l'attache de `jorf` dans un index jetable se termine seule : le processus rend la main, et le temps entre le point de commit et la sortie est rapporté ; `decisions` est rejoué si le temps le permet ;
  - les filtres rendent les mêmes résultats qu'avant la correction, et les structures dérivées restent égales à celles de `FilterIndex::from_records` ;
  - le protocole de commit est inchangé : aucun balayage complet ajouté, aucun schéma ni format modifié, aucun index SQLite ajouté ;
  - le test de non-régression est déterministe, sans seuil de temps, vert sur 50 exécutions consécutives, et échoue sans la correction ;
  - `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` et `cargo test` passent ; aucun test n'est désactivé ni marqué `#[ignore]`.
- **Invariants / mesures** : INV-07 (filtre → bitmap ≡ évaluation SQL du même filtre : la correction touche les structures de filtres et leur nettoyage, et les tests de propriété existants doivent rester verts) ; INV-04 (au redémarrage, l'état est la dernière génération commitée : le blocage se produit après le point de commit, et la correction ne doit ni déplacer ce point ni changer l'ordre du protocole). MES-15 mesure le coût d'un commit de génération et sert de référence ; le temps entre le point de commit et la fin du processus est mesuré pour cette tâche et rapporté dans le compte rendu de `@dev`, sans nouvelle campagne de mesure.
- **Hors périmètre** : les autres coûts de l'attache et du commit déjà mesurés (compression zstd, matérialisation des iid vivants, relecture des `segment.json`) ; la compaction, la reconstruction lexicale et le cache ; le script d'export, les lots exportés et l'écart de volume de 9,6 M passages, qui appartient à `IMPORTER-BASE-COMPLETE` ; l'import complet et les mesures MES-07 et MES-09 à l'échelle réelle ; toute modification d'un ADR accepté ou des documents de conception sans décision du propriétaire.

### CORRIGER-MEMOIRE-APRES-COMMIT
- **Origine** : ajoutée par le propriétaire le 2026-11-06 (« oui, ajoute la tâche »), hors plan initial. Elle vient de la campagne d'`IMPORTER-BASE-COMPLETE` (ADR-0139, passe A du 2026-11-06, rapport partiel `docs/mesures/import-legifacile-2026-11-06.md`) : l'attache d'`articles` y a été tuée par l'OOM de la VM Docker (11,67 Gio) **après son point de commit**, alors que cinq index sur six étaient importés. Le diagnostic natif de `@dev`, mesuré par `/usr/bin/time -l`, `ps` toutes les 5 s, `vmmap` et `sample` (bruts gitignorés sous `data/p10-import/diag/`), donne un pic RSS de **8,24 Go** pour `articles` (3,079 Go de texte, 2 476 715 passages, 2 135 023 documents ; reproduit à 8,23 Go) et de **5,34 Go** pour `jorf` (1,704 Go de texte, 1 833 500 passages, 1 636 145 documents). La loi mesurée sur ces deux points est : pic ≈ 1,1 à 1,5 Go + 1,0 × octets de texte + 1,2 ko × documents. Deux structures gardent tout le lot jusqu'à la fin du commit : le texte de tous les passages dans `CommitDelta.created`, pour l'étape lexicale, et un cache d'un `serde_json::Value` par **document** dans un `HashMap<&str, Option<Value>>` de `FilterIndex::insert_created_passages` (`crates/crible-store/src/filters_index.rs`, vers `:1101`), soit environ 1,2 ko par document et +2,7 Go pour `articles`. Tantivy n'est pas en cause : son budget est de 50 MiB et ses fusions retombent avant le pic. Le verdict de `@architecte` du 2026-11-06 juge l'axe retenu, borner ou supprimer ce cache, **`CONFORME`** à ADR-0066 (décision 1 : les accès SQL restants sont bornés, dont « les champs d'un document par sa clé primaire `documents(doc_id)` » ; le détail de `refresh` et `rewrite` reste dans `filters_index.rs` et `attach.rs`). C'est donc un raffinement sans ADR, comme `CORRIGER-BLOCAGE-APRES-COMMIT`. L'effet attendu sur `decisions` (1 196 623 documents, 5,04 Go de texte) est **supposé** : environ −1,44 Go, soit un pic de 6,2 à 6,8 Go, et de 9,4 à 10,1 Go avec les cinq autres index ouverts dans le serveur, sous les 11,67 Gio de la VM.
- **Objectif** : ne plus garder en mémoire, pendant la dérivation des filtres après le commit, une valeur par document pour tout le lot. Le résultat des filtres et le protocole de commit ne changent pas.
- **Dépend de** : CORRIGER-BLOCAGE-APRES-COMMIT (même chemin `after_commit`, déjà corrigé)
- **À lire** : ADR-0066 (le delta reçu par `after_commit`, les accès SQL restants et leur caractère borné) ; ADR-0048 (coût d'un commit de génération) ; `02-architecture.md` (« Générations et point de commit », « Mémoire : ce qui est en RAM ») ; `03-recherche.md` (filtres) ; `08-invariants-et-tests.md` (INV-04, INV-07) ; `crates/crible-store/src/filters_index.rs` (`insert_created_passages` vers `:1101`, `after_commit` vers `:947`) ; les bruts gitignorés de `data/p10-import/diag/`.
- **Livrables** :
  1. la correction du cache de `insert_created_passages`, à l'endroit de la cause : la mémoire retenue ne dépend plus du nombre de documents du lot. Par exemple, le cache d'un seul document à la fois si les passages arrivent groupés par document, ou une relecture par clé primaire ; `@dev` choisit et justifie son choix. Les structures dérivées restent égales à `FilterIndex::from_records`. Ne changent pas : le protocole de commit (INV-03, INV-04), le schéma de `records.sqlite`, `format_version`, les index SQLite et les dépendances. Aucun balayage complet n'est ajouté.
  2. un test de non-régression **non flaky**, sans seuil de temps ni de mémoire en CI : par exemple une sonde `#[cfg(test)]` qui borne la taille du cache. Il échoue sur le code d'avant et passe après, sur 50 exécutions consécutives. Les tests existants restent verts et inchangés.
  3. **la mesure avant et après**, en natif et en release : pic RSS par `/usr/bin/time -l` de `crible attach` d'`articles`, et de `jorf` si le temps le permet, dans un index jetable, comparé aux 8,24 Go et 5,34 Go relevés. Le **coût en temps** des éventuelles requêtes par clé primaire est mesuré, pas supposé (`AGENTS.md`, règle 7) : durée de l'étape `snapshot` avant et après. La prédiction recalculée pour `decisions` est marquée comme supposée.
  4. si la correction demande un choix que la conception ne tranche pas, une `QUESTION-CONCEPTION`.
- **Critères d'acceptation** :
  - le pic d'`articles` baisse d'environ la part du cache de documents, et le chiffre mesuré est rapporté ;
  - les structures dérivées restent égales à `FilterIndex::from_records`, et les tests d'INV-07 et d'INV-04 sont verts ;
  - le protocole de commit est inchangé : aucun balayage complet, aucun schéma ni format modifié, aucun index SQLite ni dépendance ajoutés ;
  - le test de non-régression est déterministe, échoue sans la correction et passe 50 fois sur 50 ;
  - `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` et `cargo doc --no-deps --workspace --locked`, avec `RUSTDOCFLAGS="-D warnings"`, passent ; aucun test n'est désactivé ni marqué `#[ignore]`.
- **Invariants / mesures** : INV-07 (filtre → bitmap ≡ évaluation SQL du même filtre : la correction touche la dérivation des filtres) et INV-04 (au redémarrage, l'état est la dernière génération commitée). Aucune MES-xx nouvelle : le pic est rapporté dans le compte rendu de `@dev` et repris dans le rapport d'import.
- **Hors périmètre** :
  - le texte des passages dans `CommitDelta.created`. C'est l'axe (ii), `NON-COUVERT` selon `@architecte`, et il demande un ADR. Il ne sera ouvert que si le pic de `decisions` dépasse encore environ 9 Go après cette tâche ;
  - le découpage des lots et l'agrandissement de la VM, qui seraient un écart à ADR-0139 ;
  - un plafond de mémoire transitoire du commit, jugé prématuré ;
  - le pas `progress` qui reste à `lexical` pendant `build_snapshot_after_commit`, noté dans « Idées hors périmètre » ;
  - toute modification d'un ADR accepté ou des documents de conception.

### IMPORTER-BASE-COMPLETE
- **Objectif** : charger les 8,58 M passages.
- **Dépend de** : EXPORTER-BASE-LEGIFACILE (ADR-0129), CORRIGER-BLOCAGE-APRES-COMMIT, CORRIGER-MEMOIRE-APRES-COMMIT
- **Livrables** :
  - un **contrôle d'espace disque avant la campagne** : le pic extrapolé est d'environ **105 Go** contre **151 Gio** libres sur la machine de développement, et il n'est pas mesuré (ADR-0129) ;
  - des lots `replace_all` attachés dans une instance Crible sur la **machine de développement**, sous Docker Desktop, avec la RAM de la VM indiquée (ADR-0129) ;
  - la durée et l'espace disque mesurés ;
  - le résultat de `crible inspect` ;
  - un instantané de départ, destiné au transfert vers le Mac mini (ADR-0064).
- **Critères d'acceptation** :
  - les comptes par index sont égaux à ceux de pgvector, ou les écarts sont expliqués ;
  - MES-07 et MES-09 sont refaites à l'échelle réelle ;
  - l'espace disque libre reste suffisant pendant l'import et une compaction simultanée.
- **Invariants / mesures** : MES-07, MES-09.
- **Hors périmètre** : la latence contraignante ; elle se mesure sur le Mac mini, par `MESURER-SUR-MAC-MINI` (ADR-0129).
- **À relever** : mesurer la durée de la reconstruction lexicale à 8,58 M passages, par `crible rebuild-lexical` ou à la première reconstruction, et la comparer à la cible de 60 s d'ADR-0047. MES-06 (§ 8.3.1) l'extrapole à 325 s, de 49 à 762 s selon la composition du corpus, et ADR-0088 le rappelle en « À surveiller ». Le propriétaire a décidé le 2026-10-29 d'attendre cette première mesure réelle avant tout ADR : si la cible n'est pas tenue, faire rédiger un ADR qui relève la cible pour ce cas de reprise, ou qui parallélise l'écriture (`WRITER_THREADS` vaut 1 par ADR-0087).

### CONSTITUER-JEU-DE-REFERENCE
- **Objectif** : le jeu de requêtes qui tranchera la v1.
- **Dépend de** : DEFINIR-INDEX-LEGIFACILE
- **Livrables** :
  - un jeu JSONL de requêtes réelles, avec leurs périmètres et `expected` quand il est connu (en repartant de `docs/mcp/EVALUATION.md` de Legifacile et du jeu de MES-00) ;
  - les cas « article 1240 » et leurs variantes ;
  - les requêtes filtrées les plus fréquentes.
- **Critères d'acceptation** :
  - le jeu est validé par le propriétaire ;
  - il est le même pour pgvector et pour Crible.
- **Note (ADR-0129)** : le jeu de P10 sert aussi au projet Legifacile (nouveau dépôt), qui en garde la propriété après la passation.

### COMPARER-AVEC-PGVECTOR
- **Objectif** : la mesure décisive.
- **Dépend de** : IMPORTER-BASE-COMPLETE, CONSTITUER-JEU-DE-REFERENCE
- **Livrables** : `crible eval` (modes dense et hybride) et la mesure pgvector, sur le même jeu, la même machine et dans des conditions comparables. Le rapport donne :
  - le rappel par périmètre ;
  - p50, p95 et p99 ;
  - les requêtes gagnantes et perdantes ;
  - la latence p50/p95 des requêtes **avec filtre d'intervalle** sur l'index réel de Legifacile, par rejeu de MES-14 sur cet index (mêmes cas : domaine plein, domaine réduit par une clause en bitmap), comparée à la cible de latence v1.
- **Critères d'acceptation** :
  - le **rappel** et la **fidélité** de la v1 sont atteints : ce sont eux qui sont **contraignants** ici ;
  - la **latence** est rapportée comme **indicative** sur la machine de développement (ADR-0129, décision 6) ; elle ne tranche pas J10 ;
  - sinon, une liste d'actions est soumise au propriétaire ;
  - un ADR d'index de plage n'est ouvert que si la p95 **de bout en bout** mesurée sur le Mac mini par `MESURER-SUR-MAC-MINI` dépasse la cible (ADR-0046, qui prolonge ADR-0018 ; ADR-0129, « À surveiller »).

### ACTUALISER-JEU-DE-REFERENCE
- **Origine** : ajoutée par le propriétaire le 2026-11-06 (« pour la cible non atteinte de textes_vigueur tu peux investiguer les résultats et essayer de comprendre l'écart, si les questions attendent des textes plus en vigueur je veux bien que tu mettes à jour les tests stp »), hors plan initial. Elle vient de `COMPARER-AVEC-PGVECTOR` (ADR-0140, rapport `docs/mesures/comparaison-pgvector-2026-11-06.md`). Le dense primaire atteint le seuil global — 0,482143 sur 56 questions — et la cible de `juri_texte_integral` — 0,322581 sur 31 questions —, mais pas celle de `textes_vigueur` : 0,666667 sur 12 questions contre une cible de 0,750 (`:108-110`). pgvector, rejoué le même jour, donne aussi 0,666667 (`:110`). Le rapport attribue l'écart à l'`expected` de `Q04` : une version antérieure de l'article, exclue par le filtre d'état de sa propre ligne, alors que la version courante est au rang 3 (`:305-312`). `Q01`, `Q02` et `Q06` sont manquées des deux côtés, aux rangs de Crible au-delà de 968, 14 et 171 (`:320-322`).
- **Objectif** : comprendre l'écart de `textes_vigueur` question par question, puis, quand un `expected` du jeu de référence désigne un texte qui n'est **plus en vigueur** — version antérieure, abrogé ou périmé — alors que sa requête vise le droit en vigueur, le remplacer par le document en vigueur qui lui correspond.
- **Dépend de** : COMPARER-AVEC-PGVECTOR.
- **À lire** : `docs/legifacile/jeu-de-reference.md` (en particulier « Lecture du jeu », `:245-261`, et la date figée au 2026-11-03, `:112-120`) ; `docs/legifacile/reference/*.jsonl` et `mcp.jsonl` ; ADR-0138 (`:40-46`) ; ADR-0125, décisions 4 et 16 (`:80-90` et `:226-235`) ; ADR-0140 et son complément du 2026-11-06 (`:144-168`) ; `docs/mesures/comparaison-pgvector-2026-11-06.md` (section sur les populations, `:100-112` ; `Q04`, `:305-312` ; `Q30`, `:313-318` ; « Actions soumises au propriétaire », `:409-424`) ; les données brutes de `data/p10-compare/` (gitignoré) ; le module de test `reference_game` de `crates/crible/src/eval.rs:2000`.
- **Livrables** :
  1. une analyse, pour chacune des 12 questions de `textes_vigueur` et pour toute ligne du jeu dont un `expected` n'est pas en vigueur ou n'est pas admis par le filtre de sa propre ligne : l'état et les dates de version de l'`expected`, l'existence d'une version en vigueur et son rang dans Crible et dans pgvector, et la cause du non-rappel. Les preuves viennent de l'index Crible importé et, au besoin, de pgvector en lecture seule, hors de la fenêtre quotidienne (`$PROD_DAILY_WINDOW`, `docs/mesures/MES-00-reference-pgvector.md:22` ; ADR-0032, point 2, `:23-27`) ;
  2. la mise à jour des `expected` concernés dans `docs/legifacile/reference/<index>.jsonl` : seuls les `expected` changent ; les `id`, les textes, les filtres et `mcp.jsonl` ne changent pas. Chaque changement est justifié dans une **note datée** de `docs/legifacile/jeu-de-reference.md`, qui donne l'ancien et le nouveau `doc_id` et la raison ;
  3. le rappel recalculé sur le jeu actualisé, pour le dense primaire, l'hybride et pgvector, avec les mêmes populations et les mêmes cibles : depuis les top-10 déjà enregistrés s'ils suffisent, sinon par un rejeu de `crible eval` sur les index touchés. Il est publié dans une section datée ajoutée au rapport de comparaison ; les chiffres d'origine du rapport restent tels quels ;
  4. si un cas demande un choix que rien ne tranche, par exemple comparer par article plutôt que par version, une `QUESTION-CONCEPTION`.
- **Critères d'acceptation** :
  - chaque `expected` modifié désigne un document en vigueur à la date du jeu, et le changement est justifié par une preuve ;
  - un `expected` en vigueur n'est jamais remplacé pour améliorer un score ;
  - `Q30`, non satisfaisable par construction, n'est pas modifiée (`docs/mesures/comparaison-pgvector-2026-11-06.md:313-318` ; `docs/legifacile/jeu-de-reference.md:252-255`) ;
  - la population des 56 questions satisfaisables et les cibles (0,482 ; 0,290 ; 0,750) sont inchangées (`docs/suivi.md:30-31`) ;
  - les tests `reference_game` de `crates/crible/src/eval.rs` sont verts (`cargo test -p crible reference_game`) ;
  - aucun texte de requête ni valeur de filtre n'apparaît dans un rapport, seulement des `id` (ADR-0125, décision 16, `:226-235`) ; aucune valeur de `.env.prod` n'est citée (ADR-0032, point 1, `:19-22`).
- **Invariants / mesures** : aucun invariant. La mesure est un complément de la comparaison d'ADR-0140, sans MES-xx nouvelle.
- **Hors périmètre** : toute modification de Crible (code, configurations d'index) ; le changement de la règle d'identité, c'est-à-dire comparer par article plutôt que par `doc_id`, qui demanderait un ADR ; la forme du périmètre `jurisprudence` (`Q52`) ; les autres actions du rapport ; toute modification des cibles.

### MESURER-SUR-MAC-MINI
- **Objectif** : la mesure **contraignante** de latence, en conteneur, sur le Mac mini, sur l'instantané restauré.
- **Dépend de** : IMPORTER-BASE-COMPLETE (l'instantané de départ) et COMPARER-AVEC-PGVECTOR (le rapport de comparaison) ; l'**installation de Crible sur le Mac mini par le propriétaire**, qui est hors du champ des agents (ADR-0129, décisions 2 et 9 ; pour MESURER-SUR-MAC-MINI, voir ADR-0141). Placée **avant** `BILAN-V1`.
- **À lire** : `08-invariants-et-tests.md` (cibles), `docs/mesures/MES-00-reference-pgvector.md`, `docs/phases/P8-docker-et-deploiement.md` (`MESURER-EN-CONTENEUR-MAC`, protocoles repris), ADR-0077, ADR-0084, ADR-0064, ADR-0065, ADR-0032, ADR-0129.
- **Livrables** :
  - le transfert de l'index comparé vers le Mac mini, par **instantané puis restauration** (`crible snapshot`, `crible restore`), sans réimport ni ré-encodage : iid, segments et comptes sont conservés ;
  - MES-02, MES-03, MES-05 et MES-09 refaites en conteneur sur le Mac mini, avec la RAM de la VM Docker indiquée ;
  - le rejeu de la recette ;
  - un rapport dans `docs/mesures/`, comparé à MES-00 du 2026-10-20 (pgvector, mesurée sur le même Mac mini).
- **Contraintes de lecture seule** (ADR-0129, décision 9 ; ADR-0032) :
  - seules les commandes de **lecture** de Crible sont lancées sur l'instance servie ;
  - aucune écriture sous le dossier de données de Crible ni dans le dépôt Legifacile ; temporaires hors des deux arbres ;
  - toute mesure qui demanderait une écriture d'index se fait sur un **dossier de données distinct** ;
  - aucun **arrêt, redémarrage ni `docker stop`** par un agent, même pour libérer le `WRITER_LOCK`.
- **Critères d'acceptation** :
  - la **latence p95** est au plus **698 ms** (MES-00) sur le jeu de référence, mesurée en conteneur sur le Mac mini : c'est le critère de J10 ;
  - les rapports sont consignés et comparés à MES-00 ;
  - si la cible est dépassée, J10 n'est pas tenu : la question de l'index de plage (ADR-0046) ou de la cible est rouverte par un ADR ;
  - aucun geste d'exploitation n'a été fait par un agent (pour MESURER-SUR-MAC-MINI, voir ADR-0141).
- **Invariants / mesures** : MES-02, MES-03, MES-05, MES-09 ; INV-12 (recette).
- **Hors périmètre** : toute correction de performance dans cette tâche ; l'installation, l'arrêt et le redémarrage des services, réservés au propriétaire.

### BILAN-V1
- **Objectif** : clore la v1.
- **Dépend de** : MESURER-SUR-MAC-MINI, COMPARER-AVEC-PGVECTOR (ADR-0129)
- **Livrables** : `docs/jalons/J10-bilan.md`, qui contient :
  - toutes les mesures comparées aux cibles ;
  - les écarts à la conception et la façon dont ils ont été tranchés ;
  - les leçons ;
  - la liste priorisée pour la v2 (rerank, IVF, fusions progressives, découpeur par défaut…).
- **Critères d'acceptation** : le bilan est validé par le propriétaire. La v1 est taguée **par le propriétaire**.
- **Note (ADR-0129, décision 7)** : la tâche quotidienne stable 14 jours et le retour arrière testé ne conditionnent plus ce bilan ; ils appartiennent au projet Legifacile.
