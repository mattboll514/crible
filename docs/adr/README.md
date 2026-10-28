# Architecture Decision Records (ADR)

Chaque décision qui engage le projet est écrite ici, **une par fichier**, avec son contexte, ce qui a été décidé, ses conséquences et les alternatives écartées. Ces fichiers sont la mémoire du projet : une décision n'existe pas tant qu'elle n'est pas dans un ADR. Ni la mémoire d'un agent, ni une conversation, ni un message de commit n'en tiennent lieu.

- **Les documents de conception** (`docs/01` à `docs/08`) décrivent **comment est** Crible.
- **Les ADR** expliquent **pourquoi** il est ainsi, et gardent l'historique des choix, y compris ceux qu'on a remplacés.

## Quand écrire un ADR

**Oui**, pour toute décision qui :
- change l'architecture, un format (disque, lot, API) ou un invariant ;
- ajoute, retire ou remplace une dépendance ;
- fixe un compromis de performance, de qualité ou de sécurité (seuil, valeur par défaut, algorithme) ;
- tranche une question ouverte (Q-xx) ou un écart à la conception ;
- change la façon de travailler (outils, processus, règles des agents).

**Non**, pour un détail d'implémentation local et réversible : nom d'une fonction privée, découpage interne d'un module.

## Qui fait quoi

- **N'importe qui propose** : un agent (via `@greffier` ou `/adr`) ou un humain. Un ADR proposé est en statut `Proposé`.
- **Seul le propriétaire décide** : il passe l'ADR en `Accepté` ou en `Rejeté`, avec la date, puis met à jour l'index ci-dessous.
- **Les agents ne s'appuient que sur les ADR `Accepté`.** Si une tâche dépend d'un ADR `Proposé`, elle attend qu'il soit décidé.

## Cycle de vie

```
Proposé ──► Accepté ──► Remplacé par ADR-NNNN
   │            └─────► Déprécié
   └──────► Rejeté
```

- **Un ADR accepté n'est jamais réécrit.** Pour changer d'avis, on écrit un **nouvel** ADR qui le remplace :
  - le nouveau indique `Remplace : ADR-XXXX` ;
  - l'ancien passe en `Remplacé par ADR-NNNN`, et c'est la seule modification permise sur un ADR accepté, avec la correction de coquilles.
- **Un ADR rejeté reste dans le dépôt** : il garde la trace de ce qui a été envisagé et refusé.

## Lien avec le reste du projet

- **Écart à la conception pendant une tâche.** L'agent :
  1. rédige un ADR `Proposé` (via `@greffier`) ;
  2. ajoute une ligne dans `docs/suivi.md`, section « Écarts », avec le numéro de l'ADR ;
  3. passe la tâche en `BLOQUE` et s'arrête.
- **Quand un ADR accepté change la conception**, les documents `docs/01` à `08` concernés sont mis à jour dans un commit qui cite l'ADR. C'est le propriétaire qui le demande, et `@greffier` qui le fait.
- **Le code et les docs citent les ADR** par leur numéro (`ADR-0007`).

## Fichiers et numérotation

- Nom : `NNNN-titre-en-kebab-case.md`. Le numéro a 4 chiffres, il est séquentiel et n'est **jamais réutilisé**.
- Modèle : [`modele.md`](modele.md).
- Après chaque nouvel ADR ou chaque changement de statut, mettre à jour l'index ci-dessous.

## Index

| ADR | Titre | Statut | Date |
|---|---|---|---|
| [ADR-0001](0001-vecteur-dense-seul.md) | Vecteur dense seul, sans late interaction | Accepté | 2026-10-17 |
| [ADR-0002](0002-une-instance-par-projet.md) | Une instance par projet | Accepté | 2026-10-18 |
| [ADR-0003](0003-un-index-par-structure.md) | Un index par structure de données | Accepté | 2026-10-18 |
| [ADR-0004](0004-pas-d-ann-en-v1.md) | Pas d'ANN en v1 : codes binaires, scan du pool et rescoring | Accepté | 2026-10-17 |
| [ADR-0005](0005-precision-de-reference-figee.md) | Précision de référence figée, étages de recherche réglables | Accepté | 2026-10-17 |
| [ADR-0006](0006-segments-pierres-tombales-compaction.md) | Segments par lot, pierres tombales et compaction occasionnelle | Accepté | 2026-10-18 |
| [ADR-0007](0007-tantivy-pour-bm25.md) | tantivy pour la recherche BM25 | Accepté | 2026-10-18 |
| [ADR-0008](0008-hybride-sur-le-meme-pool.md) | Hybride sur le même pool ; fusion RRF ou relative_score | Accepté | 2026-10-17 |
| [ADR-0009](0009-recherche-multi-index-dans-le-moteur.md) | Recherche multi-index dans le moteur | Accepté | 2026-10-18 |
| [ADR-0010](0010-decoupage-chez-le-producteur.md) | Le découpage reste chez le producteur | Accepté | 2026-10-18 |
| [ADR-0011](0011-stocker-l-entree-encodeur.md) | Stocker le texte exact envoyé à l'encodeur | Accepté | 2026-10-18 |
| [ADR-0012](0012-format-de-lot-public.md) | Format de lot public (Parquet + manifeste), format interne privé | Accepté | 2026-10-18 |
| [ADR-0013](0013-producteurs-python-serveur-rust-cpu.md) | Producteurs en Python, serveur Rust sur CPU | Accepté | 2026-10-18 |
| [ADR-0014](0014-analytics-sans-contenu.md) | Analytics sans contenu, journal de debug à la demande | Accepté | 2026-10-17 |
| [ADR-0015](0015-api-rest-anglais-licence-nom.md) | API REST v1 en anglais, licence Apache 2.0, nom « Crible » | Accepté | 2026-10-18 |
| [ADR-0016](0016-point-de-commit-sqlite.md) | Le commit SQLite est l'unique point de commit | Accepté | 2026-10-18 (décidé le 2026-10-19) |
| [ADR-0017](0017-compaction-suspend-les-ecritures.md) | En v1, la compaction suspend les écritures | Accepté | 2026-10-18 (décidé le 2026-10-19) |
| [ADR-0018](0018-filtres-intervalle-par-colonnes.md) | Filtres d'intervalle par colonnes de valeurs en RAM | Accepté | 2026-10-18 (décidé le 2026-10-19) |
| [ADR-0019](0019-champs-en-json-canonique.md) | Champs stockés en JSON canonique dans SQLite | Accepté | 2026-10-18 (décidé le 2026-10-19) |
| [ADR-0020](0020-texte-bm25-egal-entree-encodeur.md) | Le texte indexé en BM25 est l'entrée encodeur | Accepté | 2026-10-18 |
| [ADR-0021](0021-tracer-les-decisions-par-des-adr.md) | Tracer les décisions par des ADR | Accepté | 2026-10-19 |
| [ADR-0022](0022-colgrep-pour-la-recherche-de-code.md) | ColGrep et ColGrep-Agent pour la recherche de code par les agents | Accepté | 2026-10-19 |
| [ADR-0023](0023-orchestration-des-agents-opencode.md) | Orchestration des agents dans opencode : orchestrateur, pipeline séquentiel, agent git unique | Accepté (point 5 remplacé par ADR-0025 ; points 1 et 7 par ADR-0033) | 2026-10-19 |
| [ADR-0024](0024-plugins-opencode.md) | Plugins opencode : caveman (avec garde-fous) et notifications | Accepté | 2026-10-19 |
| [ADR-0025](0025-opencode-v2.md) | Utiliser opencode v2, avec une configuration au format v1 | Accepté | 2026-10-19 |
| [ADR-0026](0026-permissions-et-mode-auto.md) | Permissions des agents : utilitaires autorisés, interdictions renforcées, mode --auto | Accepté | 2026-10-19 |
| [ADR-0027](0027-serde-json-comme-dependance.md) | Ajouter `serde_json` comme dépendance | Accepté | 2026-10-19 |
| [ADR-0028](0028-forme-canonique-de-l-empreinte-du-modele.md) | Forme canonique de l'empreinte du modèle | Accepté | 2026-10-19 |
| [ADR-0029](0029-limites-de-filtre-profondeur-et-clauses.md) | Limites de filtre : comptage de la profondeur et des clauses | Accepté | 2026-10-19 |
| [ADR-0030](0030-nom-de-champ-declare-aux-deux-niveaux.md) | Refuser un nom de champ déclaré aux deux niveaux | Accepté | 2026-10-19 |
| [ADR-0031](0031-cle-inconnue-dans-un-patch-de-configuration.md) | Refuser une clé inconnue dans un `PATCH` de configuration | Accepté | 2026-10-19 |
| [ADR-0032](0032-acces-a-la-production.md) | Accès des agents à la production Legifacile : SSH, lecture seule, informations hors git | Accepté | 2026-10-19 |
| [ADR-0033](0033-orchestrateur-dans-claude-code.md) | L'orchestrateur tourne dans Claude Code (Opus), les agents restent dans opencode (DeepSeek via Ollama) | Accepté | 2026-10-20 |
| [ADR-0034](0034-dependance-python-blake3-pour-les-checksums.md) | Dépendance Python `blake3` pour les checksums des lots | Accepté | 2026-10-21 |
| [ADR-0035](0035-semantique-des-valeurs-nulles-dans-les-filtres.md) | Sémantique des valeurs nulles dans les filtres | Accepté | 2026-10-21 |
| [ADR-0036](0036-producer-host-reglable-par-le-producteur.md) | `producer.host` réglable par le producteur | Accepté | 2026-10-21 |
| [ADR-0037](0037-permissions-de-lecture-du-relecteur.md) | Élargir les permissions de `@relecteur` aux utilitaires de vérification et à Python | Accepté | 2026-10-21 |
| [ADR-0038](0038-delai-d-attente-sqlite-busy-timeout.md) | Régler un délai d'attente sur les verrous SQLite (`busy_timeout`) | Accepté | 2026-10-21 |
| [ADR-0039](0039-ordre-des-lignes-dans-un-lot-v1.md) | Ordre des lignes dans un lot v1 | Accepté | 2026-10-21 |
| [ADR-0040](0040-unicite-des-documents-et-des-parties-dans-un-lot-v1.md) | Unicité des documents et des parties dans un lot v1 | Accepté | 2026-10-22 |
| [ADR-0041](0041-semantique-d-attache-document-supprime-digest-du-manifeste-et-rapport.md) | Sémantique d'attache : document supprimé, digest du manifeste et rapport | Accepté | 2026-10-22 |
| [ADR-0042](0042-durees-du-rapport-d-attache-telemetrie-d-appel-non-persistee.md) | Durées du rapport d'attache : télémétrie d'appel, non persistée | Accepté | 2026-10-22 |
| [ADR-0043](0043-payload-d-un-resultat-dont-le-document-est-supprime.md) | Payload d'un résultat dont le document a été supprimé après le snapshot | Accepté | 2026-10-22 |
| [ADR-0044](0044-regles-du-relecteur-verifications-executees.md) | Règles de `@relecteur` : faire passer les vérifications et ne rapporter que ce qui a été exécuté | Accepté | 2026-10-22 |
| [ADR-0045](0045-reference-de-synth-small-calculee-exactement-independante-de-la-blas.md) | Référence de `synth-small` calculée exactement, indépendante de la bibliothèque BLAS | Accepté (clause arithmétique remplacée par ADR-0072) | 2026-10-22 |
| [ADR-0046](0046-budget-des-filtres-d-intervalle-a-l-echelle-de-legifacile.md) | Budget des filtres d'intervalle à l'échelle de Legifacile | Accepté | 2026-10-22 |
| [ADR-0047](0047-phase-de-mes-13-et-ordre-des-remedes-au-temps-de-demarrage.md) | Phase de MES-13 et ordre des remèdes au temps de démarrage | Accepté | 2026-10-22 |
| [ADR-0048](0048-rejeu-de-mes-15-en-p3-et-compresseur-zstd-reutilise.md) | Rejeu de MES-15 en P3 et compresseur zstd réutilisé | Accepté | 2026-10-22 |
| [ADR-0049](0049-dependances-du-groupe-export-de-tools-nommees-dans-docs-07.md) | Dépendances du groupe `export` de `tools/` nommées dans `docs/07` | Remplacé par ADR-0050 | 2026-10-23 |
| [ADR-0050](0050-reference-mdenseon-sous-transformers-5-14-1-et-reexport-onnx.md) | Référence mDenseOn sous transformers 5.14.1 et ré-export de l'ONNX | Accepté | 2026-10-23 (proposition et acceptation) |
| [ADR-0051](0051-cache-des-requetes-et-sessions-int8-et-fp32-de-l-encodeur.md) | Cache des requêtes et sessions INT8 et fp32 de l'encodeur | Accepté (décidé le 2026-10-24 ; point 3 remplacé par ADR-0052) | 2026-10-23 |
| [ADR-0052](0052-requetes-encodees-en-fp32-et-seuil-de-recette-a-0-999.md) | Requêtes encodées en fp32 et seuil de recette à 0,999 | Accepté | 2026-10-24 (proposition et acceptation) |
| [ADR-0053](0053-encodeur-dans-le-chemin-d-attache-arete-crible-store-vers-crible-encode.md) | Encodeur dans le chemin d'attache : arête `crible-store` → `crible-encode` | Accepté (précision 2 complétée par ADR-0054) | 2026-10-24 (proposition et acceptation) |
| [ADR-0054](0054-chargement-differe-du-modele-a-l-attache-et-bornes-de-la-recette.md) | Chargement différé du modèle à l'attache et bornes de la recette | Accepté | 2026-10-24 (proposition et acceptation) |
| [ADR-0055](0055-lot-en-ligne-fichier-d-entree-identite-empreinte-et-lot-interne.md) | Lot en ligne : fichier d'entrée, identité, empreinte des documents et lot interne | Accepté (décision 4, nettoyage, remplacée par ADR-0056) | 2026-10-24 (proposition et acceptation) |
| [ADR-0056](0056-verrou-de-presence-des-lots-en-ligne.md) | Verrou de présence des lots en ligne (remplace la décision 4 d'ADR-0055 sur le nettoyage) | Accepté | 2026-10-24 (proposition et acceptation) |
| [ADR-0057](0057-onnx-runtime-et-vrai-modele-mdenseon-en-integration-continue.md) | ONNX Runtime et vrai modèle mDenseOn en intégration continue | Accepté (complété par ADR-0059, ADR-0062 et ADR-0063, sans modification du corps) | 2026-10-24 (proposition, révision et acceptation) |
| [ADR-0058](0058-controle-anti-fuite-de-git-par-script-versionne.md) | Contrôle anti-fuite de `@git` par un script versionné, sans fichier temporaire | Accepté (`.env.prod` absent = code 2 et arrêt ; pas de seuil de longueur ; pas de hook `pre-commit` pour l'instant ; refus `*.env*` accepté tel quel, précision du 2026-10-25) | 2026-10-25 (proposition et acceptation) |
| [ADR-0059](0059-robustesse-du-telechargement-de-l-artefact-du-modele-en-ci.md) | Robustesse du téléchargement de l'artefact du modèle en CI (complète ADR-0057) | Accepté (options 1 et 3 ; 4 essais, attentes 5/15/45 s, `Retry-After` ≤ 60 s, échéance 600 s ; workflow `model-cache.yml` seul écrivain du cache, `ci.yml` en restauration seule ; `HF_TOKEN` en réserve) | 2026-10-25 (proposition et acceptation) |
| [ADR-0060](0060-contrat-du-patch-de-metadonnees-des-documents.md) | Contrat du patch de métadonnées des documents | Accepté (toutes les options (a) : refus atomique `not_found`, noms réservés refusés par la règle du champ inconnu, champs non filtrables ou non rendus acceptés, 32 Mo et 10 000 entrées, rapport non persisté, aucune génération pour un patch sans effet) | 2026-10-25 (proposition et acceptation) |
| [ADR-0061](0061-manifeste-d-un-lot-exporte.md) | Manifeste d'un lot exporté | Accepté (toutes les options (a) sur les décisions 1 et 3 à 5, option (i) sur la décision 2 ; `format_version` reste 1) | 2026-10-25 (proposition et acceptation) |
| [ADR-0062](0062-second-test-local-de-l-echantillon-legifacile-exclu-de-la-ci-par-son-nom.md) | Second test local de l'échantillon Legifacile, exclu de la CI par son nom | Accepté (option (1a) : second `--skip` explicite ; option (2ii) : pas de borne par défaut sur la durée, durée relevée à J3 ; complété par ADR-0063, sans modification du corps) | 2026-10-25 (proposition et acceptation) |
| [ADR-0063](0063-troisieme-test-local-de-l-echantillon-legifacile-exclu-de-la-ci-par-son-nom.md) | Troisième test local de l'échantillon Legifacile, exclu de la CI par son nom | Accepté (option (1a) : troisième `--skip` explicite ; option (2ii) : pas de borne par défaut sur la durée, durée relevée à J3 ; accepté par anticipation le 2026-10-25, confirmé par le propriétaire le 2026-10-26 ; complété par ADR-0085, sans modification du corps) | 2026-10-25 (proposition et acceptation) |
| [ADR-0064](0064-format-et-contenu-d-un-instantane.md) | Format et contenu d'un instantané (`snapshot.json`) | Accepté (les quatre décisions telles quelles ; accepté par anticipation le 2026-10-25, confirmé par le propriétaire le 2026-10-26) | 2026-10-25 (proposition et acceptation) |
| [ADR-0065](0065-instantane-hors-ligne-epoque-de-restauration-et-retention.md) | Instantané hors ligne, époque de restauration, cible et rétention | Accepté (les six décisions telles quelles ; accepté par anticipation le 2026-10-25, confirmé par le propriétaire le 2026-10-26) | 2026-10-25 (proposition et acceptation) |
| [ADR-0066](0066-derivation-de-l-instantane-apres-commit-sans-balayage-complet.md) | Dérivation de l'instantané après commit sans balayage complet | Accepté (les quatre décisions telles que proposées : `after_commit` reçoit le delta écrit par la transaction, les deux index SQLite en recours seulement, tâche hors plan `BORNER-COUT-DU-COMMIT` en P3, chiffres du 2026-10-25 conservés et rejeu en section datée ; schéma et `format_version` inchangés) | 2026-10-26 (proposition et acceptation) |
| [ADR-0067](0067-duree-de-la-recette-a-l-attache-et-echeance-de-wait-true.md) | Durée de la recette à l'attache et échéance de `?wait=true` | Accepté (les quatre décisions telles que proposées : recette inchangée ; `200` pour un job terminé dans le délai, `202` avec le job en cours à l'échéance de `wait_timeout_ms`, jamais `504` ; `wait_timeout_ms` = 60 000 ms dans `docs/05` et `docs/07` ; tâche hors plan `ACCELERER-RECETTE` en P4 ; J3 n'attend rien) | 2026-10-26 (proposition et acceptation) |
| [ADR-0068](0068-construire-et-tester-crible-store-avec-la-feature-failpoints-en-ci.md) | Construire et tester `crible-store` avec la feature `failpoints` en intégration continue | Accepté (accepté par anticipation le 2026-10-26, confirmé par le propriétaire le 2026-10-26 ; les trois décisions telles que proposées : les deux commandes `failpoints` après `cargo test --locked`, périmètre `crible-store` seul sans `--workspace`, rien d'autre ne change dans le workflow ; complète ADR-0057 sans modifier son corps) | 2026-10-26 (proposition et acceptation anticipée) |
| [ADR-0069](0069-reconstruction-et-format-de-codes-bin.md) | Reconstruction et format de `codes.bin` | Accepté (accepté par anticipation le 2026-10-26, confirmé par le propriétaire le 2026-10-26 ; les six décisions telles que proposées : reconstruction à l'ouverture en écriture sans génération nouvelle, codes en RAM seulement en lecture seule, absence qui n'est pas une erreur et longueur fausse qui en est une, instantané et export inchangés, aucune sous-commande CLI, format sans en-tête de `count × ceil(dims/64) × 8` octets versionné par `format_version` de `segment.json`) | 2026-10-26 (proposition et acceptation anticipée) |
| [ADR-0070](0070-noyaux-simd-code-maison.md) | Noyaux SIMD : code maison dans `crible-search::kernels` | Accepté (accepté par anticipation le 2026-10-26, confirmé par le propriétaire le 2026-10-26 ; code maison retenu après micro-benchmark, sans crate dédiée : NEON sur `aarch64`, AVX2 sur `x86_64`, repli scalaire, détection à l'exécution ; benchmarks `criterion` consignés dans l'ADR) | 2026-10-26 (proposition et acceptation anticipée) |
| [ADR-0071](0071-seuil-entre-parcours-dense-et-clairseme-du-scan-binaire.md) | Seuil entre parcours dense et parcours clairsemé du scan binaire | Remplacé par ADR-0081 | 2026-10-26 |
| [ADR-0072](0072-un-seul-calcul-du-produit-scalaire-pour-exact-et-le-rescoring.md) | Un seul calcul du produit scalaire pour EXACT et le rescoring | Accepté (accepté par anticipation le 2026-10-26, par délégation du propriétaire pour la série RESCORER-FP16, INSERER-POOL-EN-ORDRE, ACCELERER-RECETTE, PLANIFIER-STRATEGIE, confirmé par le propriétaire le 2026-10-27 ; un seul chemin de calcul partagé par EXACT et le rescoring : `kernels::dot` pour fp16, fonction scalaire accumulée en `f64` pour fp32 ; remplace la clause arithmétique d'ADR-0045, le reste d'ADR-0045 est inchangé) | 2026-10-26 (proposition et acceptation anticipée) |
| [ADR-0073](0073-recette-sans-encodage-par-lots.md) | Recette sans encodage par lots : la mesure ne donne aucun gain | Accepté (accepté par anticipation le 2026-10-27, par délégation du propriétaire pour la série RESCORER-FP16, INSERER-POOL-EN-ORDRE, ACCELERER-RECETTE, PLANIFIER-STRATEGIE, confirmé par le propriétaire le 2026-10-27 ; le code du livrable (b) d'`ACCELERER-RECETTE` est retiré, la recette reste encodée un passage à la fois, aucune taille de lot dans le code ; le contrat de la recette ne change pas ; fils et réglages d'ONNX Runtime revus en conteneur en P8 avec la recette comme cas de mesure ; fournisseur d'exécution et ré-export du graphe par nouvel ADR, non planifiés) | 2026-10-27 (proposition et acceptation anticipée) |
| [ADR-0074](0074-planification-de-la-strategie-dense.md) | Planification de la stratégie dense : seuil exact, valeur par défaut, nom de la stratégie et k au-delà de N | Accepté (accepté par anticipation le 2026-10-27, par délégation du propriétaire pour la série RESCORER-FP16, INSERER-POOL-EN-ORDRE, ACCELERER-RECETTE, PLANIFIER-STRATEGIE, confirmé par le propriétaire le 2026-10-27 ; les cinq décisions telles que proposées : domaine `0` à `u32::MAX`, `exact_threshold` optionnelle et défaut matérialisé à la création depuis `rescore_candidates`, `dense_strategy` valant `"exact"` ou `"binary"`, `max(N, k)` candidats rescorés sous BINARY, égalité au seuil traitée en EXACT avec glossaire aligné) | 2026-10-27 (proposition et acceptation anticipée) |
| [ADR-0075](0075-regroupement-des-resultats-ordre-et-forme-de-la-reponse.md) | Regroupement des résultats : ordre de `dedup` et `group_by`, sur-échantillonnage, forme de la réponse et `k` de la branche dense | Accepté (accepté par anticipation le 2026-10-27, confirmé par le propriétaire le 2026-10-27 ; les quatre décisions telles que proposées : `dedup` avant `group_by` avec recomplètement, règle de sur-échantillonnage dans `crible-search`, réponse plate où `k` compte des documents et où les entrées d'un document sont consécutives, `max(N, candidates_per_branch)` candidats sous BINARY ; erratum du 2026-10-27 : les renvois de code sont désormais `group.rs:158/185/218/255`, et la contiguïté des entrées d'un document est levée par `keep_documents` et le test `grouped_entries_of_a_document_are_consecutive`) | 2026-10-27 (proposition et acceptation anticipée) |
| [ADR-0076](0076-protocole-de-mesure-de-fidelite-du-scan-binaire.md) | Protocole de mesure de fidélité du scan binaire : métriques, centrage, pools et requêtes | Accepté (accepté par anticipation le 2026-10-27, confirmé par le propriétaire le 2026-10-27 ; les six décisions telles que proposées : recall@10 au passage et recouvrement@10 au document, cible de 0,99 en moyenne globale au N par défaut avec signalement par structure, tableau par structure plus moyenne globale, centrage mesuré hors moteur par un harnais contrôlé au bit près sans changement du moteur ni du format, mesure sur l'échantillon avec `PATCH search.exact_threshold = 0` et confirmation renvoyée à P9, 300 requêtes fp32 sans filtre plus une variante filtrée déclarée, levier proposé et non codé) | 2026-10-27 (proposition et acceptation anticipée) |
| [ADR-0077](0077-protocole-de-mesure-de-latence-de-la-recherche-dense.md) | Protocole de mesure de latence de la recherche dense : N, froid, pools, requêtes et RSS | Accepté (accepté par anticipation le 2026-10-27, confirmé par le propriétaire le 2026-10-27 ; les cinq décisions telles que proposées : N ∈ {1000, 2000, 5000} avec rescoring et scan mesurés et une colonne EXACT au point de bascule d'ADR-0074, froid par copies non cachées (`F_NOCACHE`, `mincore`) pour le rescoring seul avec série secondaire après 10 min d'inactivité, pools de 1 k à 7,74 M contigus et dispersés avec contre-contrôle par la CLI, 300 requêtes fp32 distinctes et 3 passes avec p50/p95/p99 par rang le plus proche, RSS en trois états scindé en mémoire anonyme et adossée à des fichiers) | 2026-10-27 (proposition et acceptation anticipée) |
| [ADR-0078](0078-n-par-defaut-a-5000-et-sort-des-index-existants.md) | N par défaut à 5000 et sort des index existants | Accepté (décision du propriétaire le 2026-10-27 pour la valeur 5000 ; mécanisme du défaut et sort des index existants proposés par l'orchestrateur sur recommandation de `@architecte`, confirmés par le propriétaire le 2026-10-28 ; les sept décisions telles que proposées : N par défaut à 5000 par défaut documentaire et de gabarit, sans constante ni repli dans le code et avec la clé toujours obligatoire dans `index.json` ; `exact_threshold` suit N à la création ; aucune migration ni renormalisation des index existants, qu'un double `PATCH` fait monter ; `max(N, candidates_per_branch)` vaut 5000 pour tout `k` ; la ligne de budget de `docs/03-recherche.md:144` est remplacée par MES-03 et le dépassement assumé ; fidélité confirmée à 8,58 M en P9 ; complète ADR-0074 décision 2 et tranche Q-01) | 2026-10-27 (proposition et acceptation) |
| [ADR-0079](0079-cout-des-cles-au-rescoring.md) | Coût des clés au rescoring | Accepté (décision du propriétaire le 2026-10-27, tel que proposé ; séquence retenue : profil et mesures neutres d'abord, puis clés après le tri par score, clés par lot en repli ; clés en RAM et colonne de clés en mmap rejugées après le profil ; mise en œuvre par `REDUIRE-COUT-DES-CLES`, en P4 avant J4) | 2026-10-27 |
| [ADR-0080](0080-acces-aux-vecteurs-fp16-au-rescoring.md) | Accès aux vecteurs fp16 au rescoring | Accepté (décision du propriétaire le 2026-10-27, tel que proposé ; précisé par ADR-0084 ; lecture groupée et parallèle des N lignes avant le calcul, `MADV_RANDOM` mesuré comme contrôle, `pread` groupé en repli, vecteurs en RAM et `mlock` écartés ; mise en œuvre par `GROUPER-LECTURES-VECTEURS`, en P4 avant J4) | 2026-10-27 (proposition et acceptation) |
| [ADR-0081](0081-parcours-du-scan-binaire-un-seul-parcours-clairseme.md) | Parcours du scan binaire : un seul parcours clairsemé | Accepté (décision du propriétaire le 2026-10-27, tel que proposé ; le seuil d'ADR-0071 tombe, un seul parcours clairsemé, sous condition d'une mesure multi-segments ; remplace ADR-0071 ; mise en œuvre par `UNIFIER-PARCOURS-SCAN`, en P4 avant J4) | 2026-10-27 (proposition et acceptation) |
| [ADR-0082](0082-scan-binaire-multi-segments-curseur-de-segment-et-verification-a-b.md) | Scan binaire multi-segments : curseur de segment et vérification A/B du parcours unique | Accepté (accepté par anticipation le 2026-10-27, par délégation du propriétaire pour l'enchaînement `UNIFIER-PARCOURS-SCAN`, `REDUIRE-COUT-DES-CLES` et `GROUPER-LECTURES-VECTEURS`, à confirmer au point de fin des trois tâches, confirmé par le propriétaire le 2026-10-28 ; les cinq décisions telles que proposées : la question rouverte par ADR-0081 est tranchée par le curseur de segment, obligatoire dans la tâche, sans retour d'un second parcours ; A/B après le code entre le binaire de `e7d4e08` et le nouveau, sur les trois copies et tous les points ; règle de bruit par l'amplitude des écarts d'une paire à l'autre, un match nul satisfaisant le critère ; point perdu au-delà du bruit en `QUESTION-CONCEPTION` et `BLOQUE`, sans repli dans le code ; percentile du budget renvoyé à P8) | 2026-10-27 (proposition et acceptation anticipée) |
| [ADR-0083](0083-suites-de-reduire-cout-des-cles.md) | Suites de REDUIRE-COUT-DES-CLES : seule la lecture des clés après le tri est livrée | Accepté (accepté par anticipation le 2026-10-28, par délégation du propriétaire pour l'enchaînement `UNIFIER-PARCOURS-SCAN`, `REDUIRE-COUT-DES-CLES` et `GROUPER-LECTURES-VECTEURS`, à confirmer au point de fin des trois tâches, confirmé par le propriétaire le 2026-10-28 ; les six décisions telles que proposées : seule (b), la lecture des clés après le tri par score, est livrée, (2a) et (2b) retirées du code sans trace — (2a) perd 6,6 % et 10,2 %, (2b) reste dans le bruit pour un `unsafe` — et (a) non mise en œuvre, sa condition n'étant pas remplie ; la condition d'« À surveiller » d'ADR-0079 (`:100`) est infirmée, aucune règle d'ordre des iid n'est écrite dans `docs/03` ; mesure de confirmation de (b) seule contre `base` et `final`, N ∈ {1000, 2000, 5000}, règle de bruit d'ADR-0082 et contrôles d'égalité rejoués sur la variante livrée ; aucune mesure d'ordre ajoutée à `GROUPER-LECTURES-VECTEURS`) | 2026-10-28 (proposition et acceptation anticipée) |
| [ADR-0084](0084-quel-froid-fait-foi-pour-le-rescoring-jusqua-p8.md) | Quel froid fait foi pour le rescoring jusqu'à P8 | Accepté (décision du propriétaire le 2026-10-28, tel que proposé ; la série « searcher déjà ouvert, cache déplacé après l'ouverture » fait foi en P4, `F_NOCACHE` non rejouable sur le moteur, froid vrai renvoyé à P8 ; précise la décision 6 d'ADR-0080 et la ligne MES-03 de `docs/08` sans modifier leur corps) | 2026-10-28 (proposition et acceptation) |
| [ADR-0085](0085-quatrieme-test-local-de-l-echantillon-legifacile-exclu-de-la-ci-par-son-nom.md) | Quatrième test local de l'échantillon Legifacile, exclu de la CI par son nom | Accepté (option (1a) : quatrième `--skip` explicite, tel que proposé ; complète ADR-0057, ADR-0062 et ADR-0063 sans modification de leur corps) | 2026-10-28 (proposition et acceptation) |
| [ADR-0086](0086-domaine-de-lexical-language-et-chaine-sans-liste-de-mots-vides.md) | Domaine de `lexical.language` et chaîne d'analyse des langues sans liste de mots vides | Accepté (décision du propriétaire le 2026-10-28, tel que proposé ; dix-huit codes ISO 639-1 en minuscules sans normalisation, refus en `invalid_request` (400) à la validation de la configuration, `StopWordFilter` absent pour `ar`, `el`, `ro`, `ta` et `tr` ; précise `docs/03`, `docs/04` et la fiche `CONFIGURER-ANALYSEUR`, `docs/05` inchangé) | 2026-10-28 |
| [ADR-0087](0087-reprise-lexicale-et-ouverture-en-lecture-seule.md) | Reprise lexicale à l'ouverture en écriture et comportement en lecture seule | Accepté (décision du propriétaire le 2026-10-28, tel que proposé ; les cinq décisions telles que proposées : la reprise lexicale et la comparaison `Gt`/`G` n'ont lieu qu'à l'ouverture en écriture, une lecture ne compare rien, n'ouvre aucun `IndexWriter`, ne commite et ne supprime rien, et n'ouvre un lecteur que si la charge utile du dernier commit vaut `G` ; en lecture seule, `Gt ≠ G` laisse l'instantané sans lecteur lexical et la branche lexicale refuse en `overloaded` (503) avec le message « lexical index behind generation Gt of G; open the index for writing to repair it », sans dégradation silencieuse de `hybrid` vers le dense, le mode `dense` restant servi ; `internal` et `invalid_request` écartés ; la réécriture du manifeste manquant à la lecture (`ensure_manifest`) reste hors périmètre et ne sert pas de précédent ; `AttachDurations` reçoit un champ `lexical` entre `transaction` et `manifest`, télémétrie d'appel non persistée, `batches.report` inchangé ; `docs/08` inchangé, INV-02, INV-03 et INV-04 ne changent pas) | 2026-10-28 (proposition et acceptation) |
| [ADR-0088](0088-index-lexical-a-jour-et-lexical-illisible-a-la-reprise.md) | Définition d'un index lexical à jour, et lexical illisible à la reprise | Accepté (décision du propriétaire le 2026-10-28, tel que proposé ; les décisions 1 à 7 telles que proposées : la charge utile d'un commit tantivy reste la génération dont le texte est indexé, « lexical à jour pour `G` » vaut `M ≤ Gt ≤ G` avec `M = meta.passage_change_generation`, ce marqueur est une colonne nullable de `meta` écrite dans la transaction de l'étape 3, `format_version` reste 1 pour une colonne optionnelle et additive, une colonne absente n'est jamais complétée à la lecture et réduit le critère à `Gt = G`, le refus en lecture seule garde `overloaded` avec quatre messages nommant l'état constaté, un `lexical/` illisible est reconstruit à l'ouverture en écriture et ne fait pas échouer la lecture seule ; `docs/08` inchangé, INV-01 à INV-04 ne changent pas ; précise ADR-0087 sans réécrire son corps) | 2026-10-28 |
| [ADR-0089](0089-branche-lexicale-indisponible-ou-desactivee.md) | Branche lexicale indisponible ou désactivée : messages des états sans commit tantivy et code d'erreur quand `lexical.enabled` vaut `false` | Accepté (accepté par anticipation par le propriétaire le 2026-10-28, à confirmer au point de fin de l'enchaînement `RECONSTRUIRE-LEXICAL`, `RECHERCHER-BM25-DANS-LE-POOL` et `FUSIONNER-HYBRIDE` ; les quatre décisions telles que proposées : deux messages de plus pour `Behind { committed: None }` et `NotCertified { committed: None }`, en `overloaded` (503), sans `iid` ; `Missing` (`M = 0`) ne refuse pas et rend une réponse vide ; une branche désactivée est refusée en `invalid_request` (400) pour `lexical` comme pour `hybrid` ; une recherche multi-index dont une cible a le lexical désactivé est refusée en entier, le message nommant l'index fautif ; le défaut de `mode` ne change pas ; précise ADR-0088 décision 5 et tranche ce que sa décision 8 laissait ouvert) | 2026-10-28 (proposition et acceptation anticipée) |

## Questions ouvertes

Chacune deviendra un ADR quand elle sera tranchée.

| # | Question | À trancher dans |
|---|---|---|
| Q-02 | Fusion par défaut (`rrf` ou `relative_score`, et α) | MES-10 (P5) |
| Q-03 | Champs multivalués (`keyword[]`) nécessaires pour Legifacile ? | `DEFINIR-INDEX-LEGIFACILE` (P9) ; à remonter plus tôt si besoin |
| Q-04 | Gestion des élisions et des identifiants dans l'analyseur français | MES-11 (P5) |
| Q-05 | Répéter un en-tête court sur chaque passage (côté producteur) | Mesure en P9 |

Questions tranchées :

| # | Question | Tranchée par |
|---|---|---|
| Q-01 | N par défaut et cible de fidélité du scan binaire | ADR-0078 (2026-10-27) : N = 5000 par défaut, défaut documentaire et de gabarit ; cible de fidélité atteinte sur l'échantillon (MES-01), confirmation à 8,58 M en P9 |
| Q-06 | Seuil `acceptance_min_cosine` pour des vecteurs encodés sur MPS ou CUDA en fp16 | ADR-0052 (2026-10-24) : `0,999`. La question est rouverte par un nouvel ADR si un producteur MPS ou CUDA échoue à la recette |