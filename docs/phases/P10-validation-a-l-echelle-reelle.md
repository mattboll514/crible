# P10 — Validation à l'échelle réelle

**But** : valider Crible sur la vraie base de Legifacile — six configs d'index, export de migration, import complet, jeu de référence, comparaison avec pgvector, mesure sur le Mac mini — puis écrire le bilan. C'est la dernière phase du plan (ADR-0129).

**Jalon J10 — fin du plan, « v1 » de Crible** (ADR-0129, décision 7) :

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
- **Dépend de** : J9 validé (ADR-0129)
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

### IMPORTER-BASE-COMPLETE
- **Objectif** : charger les 8,58 M passages.
- **Dépend de** : EXPORTER-BASE-LEGIFACILE (ADR-0129)
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
