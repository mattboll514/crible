# 08 — Invariants, tests et mesures

## Invariants

Chaque invariant a au moins un test dédié, nommé `inv_xx_…`. Une tâche qui touche un invariant cite son numéro dans sa fiche.

| # | Invariant | Comment on le teste |
|---|---|---|
| INV-01 | Aucun iid ne sort du moteur : ni API, ni logs, ni analytics, ni exports | Recherche de motifs dans les réponses et les logs des tests d'intégration ; les types de l'API n'ont pas de champ iid |
| INV-02 | Une lecture n'est jamais bloquée par une écriture | Charge de recherches concurrentes pendant une attache et une compaction : aucune requête au-delà du p99 de référence × 3 |
| INV-03 | Une génération est visible entièrement ou pas du tout. Portée : pool, scores et classement ; les payloads n'en font pas partie (ADR-0043) | Lecteurs concurrents pendant des attaches : jamais d'état mixte (ancienne et nouvelle version d'un document, ou aucune des deux) |
| INV-04 | Au redémarrage, l'état est la dernière génération commitée ; tout le reste est reconstruit ou nettoyé | Injection de pannes : arrêt brutal à chaque étape du protocole d'attache, puis redémarrage et vérification |
| INV-05 | Les branches dense et lexicale utilisent le même pool | Test du pipeline : les candidats des deux branches sont inclus dans le pool |
| INV-06 | Pool ≤ seuil exact ⇒ résultat identique au calcul exact | Comparaison avec la référence numpy sur le jeu synthétique |
| INV-07 | Filtre → bitmap ≡ évaluation SQL du même filtre | Test de propriété (proptest) : filtres aléatoires, comparés à une évaluation SQLite JSON |
| INV-08 | Export puis import ⇒ mêmes résultats (mêmes clés, scores à 1e-6) | Aller-retour sur le jeu synthétique et sur l'échantillon Legifacile |
| INV-09 | Compaction ⇒ mêmes résultats avant et après | Mêmes requêtes avant et après la compaction |
| INV-10 | Rejouer un lot (même `batch_id`) ne fait rien | Double attache, génération et comptes inchangés |
| INV-11 | Aucun passage tronqué en silence | Un lot dont un passage dépasse `max_tokens` est rejeté |
| INV-12 | Un lot incompatible avec le modèle de l'index est refusé | Lot avec un mauvais prompt ou un modèle différent : `acceptance_failed` ou `incompatible_model` |
| INV-13 | Les analytics ne contiennent ni texte de requête, ni valeur de filtre, ni hash, ni id de résultat | Recherche des textes et valeurs utilisés dans le fichier `analytics.sqlite` |
| INV-14 | Rien de ce qui a coûté du GPU n'est détruit sans copie valide | La compaction garde l'époque E jusqu'à la bascule ; un échec de compaction laisse E intacte |
| INV-15 | Tout format sur disque est little-endian explicite et versionné | Relecture d'un segment écrit avec des valeurs connues ; refus d'une version inconnue, `snapshot.json` d'un instantané compris (ADR-0064) |
| INV-16 | Un réglage figé ne se modifie pas par l'API | `PATCH` → `409 immutable_setting` |
| INV-17 | L'écriture des analytics ou du journal ne bloque jamais une requête | File saturée artificiellement : latence de recherche inchangée, pertes comptées |

## Stratégie de tests

| Niveau | Outil | Contenu |
|---|---|---|
| Unitaire | `cargo test` | Fonctions pures : filtres, fusion, histogrammes, codes binaires, sérialisation |
| Propriété | `proptest` | INV-07 ; aller-retour des formats ; ordre de fusion |
| Intégration | `cargo test` (`tests/`), `tempfile` | Index réel dans un dossier temporaire : attache, recherche, compaction, export |
| Référence | `tools/reference_topk.py` (numpy) | Top-k exact calculé en Python sur le jeu synthétique ; Crible doit le reproduire |
| Pannes | Points d'injection `CRIBLE_FAILPOINT=<étape>` (en build de test uniquement) | INV-03, INV-04 |
| Charge | Script de charge (Rust ou Python), recherches concurrentes | INV-02, INV-17 |
| Micro-benchmarks | `criterion` | Noyaux SIMD, scans, fusion |
| Éval | `crible eval` | Rappel et latence sur des jeux de requêtes |
| Python | `pytest` | `BatchWriter`, client HTTP |

### Sauts de test

Règle normative (ADR-0057) : **`#[ignore]` est le seul mécanisme de saut autorisé.** Un test marqué `#[ignore]` qui est lancé explicitement (`--include-ignored`) et privé de son prérequis **échoue**, et son message d'échec donne la commande qui produit ce prérequis. Un `return` anticipé, un `if !path.exists() { return; }` ou toute sortie silencieuse équivalente sont interdits. Un test non lancé n'est pas un saut : il n'apparaît pas dans le compte des tests exécutés.

Deux conséquences pratiques :

- un test qui saute silencieusement ne prouve rien et se lit pourtant comme vert : c'est le défaut corrigé dans `crates/crible/tests/recette_import.rs` par `AJOUTER-ORT-CI` ; les tests de `crates/crible-encode/tests/charger_modele.rs` et de `crates/crible-core/tests/model_config.rs` sont le modèle à suivre, puisqu'ils échouent déjà avec la commande d'export à lancer ;
- une CI qui exécute `--include-ignored` doit nommer explicitement chaque test qu'elle laisse local, par son nom, et le justifier : c'est le cas de `inv_12_the_legifacile_sample_passes_the_acceptance_check`, de `inv_09_legifacile_sample_queries_return_the_same_results_after_compaction` et de `inv_08_legifacile_sample_queries_return_the_same_results_after_export_and_import`, dont les données dérivent de la production Legifacile (ADR-0032) et dont les deux derniers demandent en plus ONNX Runtime et le vrai modèle (ADR-0057, ADR-0062, ADR-0063).

## Jeux de données

| Jeu | Contenu | Où |
|---|---|---|
| `synth-small` | 10 000 passages, 2 000 documents, 768 dimensions, champs de tous types, vecteurs en grappes. Graine fixe | Généré par `tools/`, dans `data/` ; la somme de contrôle est versionnée |
| `synth-large` | 8,6 M passages de vecteurs aléatoires normalisés : pour la latence seulement | Généré à la demande, dans `data/` |
| `legifacile-sample` | ~50 000 passages réels pour chacune des structures, avec leurs vecteurs existants, et un jeu de requêtes de référence | Produit avec le propriétaire (`CONSTITUER-ECHANTILLON-LEGIFACILE`), dans `data/` |
| `legifacile-full` | Toute la base (8,58 M) | Phase 9 |

## Mesures

Une mesure n'est valable que si elle est consignée dans `docs/mesures/MES-xx-<slug>.md`, selon le modèle de `docs/mesures/README.md` : machine, commit, jeu de données, protocole, chiffres bruts, conclusion.

| # | Question | Phase |
|---|---|---|
| MES-00 | Référence pgvector actuelle : rappel et latence p50/p95 sur le jeu de requêtes de Legifacile | P0 |
| MES-01 | Rappel du scan binaire + rescoring contre l'exact (mDenseOn, 768 dimensions), selon N → choix du N par défaut. Métriques, agrégation et protocole fixés par ADR-0076 | P4 |
| MES-02 | Latence du scan binaire selon la taille du pool (1 k → 8,6 M). Protocole fixé par ADR-0077 : pools construits à 1 k, 10 k, 100 k, 1 M et 7,74 M passages vivants, contigus et dispersés, BINARY forcé par `search.exact_threshold = 0`, un seul parcours, le clairsemé, avec curseur de segment (ADR-0081, qui remplace ADR-0071 ; ADR-0082 ; corrigé le 2026-10-28) | P4, puis P8 en conteneur |
| MES-03 | Latence du rescoring fp16, à froid et à chaud. Protocole fixé par ADR-0077 : N ∈ {1000, 2000, 5000}, froid par copies non cachées (`F_NOCACHE`, `mincore`) ; le scan n'a pas d'état froid, ses codes étant en RAM. Depuis l'ADR-0084 (accepté le 2026-10-28), la série `F_NOCACHE` est déclarée non rejouable sur le moteur en P4, la vérification du sceau blake3 à l'ouverture réchauffant le fichier ; la série « searcher déjà ouvert, cache déplacé après l'ouverture » fait foi jusqu'à P8, et le froid vrai est renvoyé à P8. Rejouée en conteneur par la tâche `MESURER-EN-CONTENEUR-MAC` (P8) | P4, P8 |
| MES-04 | Parité de l'encodeur : ONNX fp32 et INT8 contre PyTorch (cosinus, rappel) | P2 |
| MES-05 | Latence d'encodage d'une requête sur CPU (natif, puis en conteneur) | P2, P8 |
| MES-06 | tantivy : taille d'index (avec ou sans positions), durée de construction, latence BM25 filtrée selon le pool | P5 |
| MES-07 | Durée d'une compaction à l'échelle de Legifacile | P3, P9 |
| MES-08 | Durée d'une attache quotidienne type (~1 000 à 3 000 passages) | P3, P9 |
| MES-09 | RAM réelle (RSS) en régime, par index. Protocole fixé par ADR-0077 : trois états (index ouvert, après une passe de requêtes, session fp32 à part), mémoire anonyme et adossée à des fichiers séparées | P4, P8 |
| MES-10 | Qualité : dense, lexical, hybride (`rrf` contre `relative_score`) sur le jeu de référence Legifacile, cas « article 1240 » compris → fusion par défaut | P5 |
| MES-11 | Analyse française : élisions, stopwords, identifiants (`L1132-1`, `1240`) | P5 |
| MES-12 | Codes binaires : signe brut contre signe après centrage. Centrage mesuré hors moteur, par un harnais contrôlé au bit près (ADR-0076) | P4 |
| MES-13 | Temps de démarrage : reconstruction des bitmaps depuis SQLite contre chargement du cache | P1, P3 |
| MES-14 | Coût des filtres d'intervalle (colonne de valeurs) selon le domaine | P1, P4 |
| MES-15 | Coût d'un commit de génération selon le nombre de passages vivants (matérialisation des iid vivants, relecture des `segment.json`) | P1, P3 |

## Cibles chiffrées

Les cibles de latence et de rappel sont fixées **après MES-00** et consignées dans `docs/suivi.md` (section « Cibles »). Règles :
- **Fidélité** : le recall@10 de l'approximation contre l'exact doit atteindre au moins 0,99 au N par défaut (N = 5000, ADR-0078), **en moyenne sur toutes les requêtes** et au niveau du passage. Les valeurs par structure sont rapportées et toute structure sous 0,99 est signalée et expliquée ; le recouvrement@10 au niveau du document est rapporté sans cible (ADR-0076). La cible est atteinte sur l'échantillon (MES-01) ; sa confirmation à 8,58 M passages est renvoyée à P9.
- **Rappel** : au moins égal à pgvector (MES-00) sur le jeu de référence.
- **Latence** : p95 au plus égal à pgvector (MES-00) à l'échelle de Legifacile, sur le Mac mini en conteneur.
