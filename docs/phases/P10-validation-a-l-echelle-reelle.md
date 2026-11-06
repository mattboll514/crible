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

### IMPORTER-BASE-COMPLETE
- **Objectif** : charger les 8,58 M passages.
- **Dépend de** : EXPORTER-BASE-LEGIFACILE (ADR-0129), CORRIGER-BLOCAGE-APRES-COMMIT
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

### MESURER-SUR-MAC-MINI
- **Objectif** : la mesure **contraignante** de latence, en conteneur, sur le Mac mini, sur l'instantané restauré.
- **Dépend de** : IMPORTER-BASE-COMPLETE (l'instantané de départ) et COMPARER-AVEC-PGVECTOR (le rapport de comparaison) ; l'**installation de Crible sur le Mac mini par le propriétaire**, qui est hors du champ des agents (ADR-0129, décisions 2 et 9). Placée **avant** `BILAN-V1`.
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
  - aucun geste d'exploitation n'a été fait par un agent.
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
