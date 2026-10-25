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
| [ADR-0045](0045-reference-de-synth-small-calculee-exactement-independante-de-la-blas.md) | Référence de `synth-small` calculée exactement, indépendante de la bibliothèque BLAS | Accepté | 2026-10-22 |
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
| [ADR-0063](0063-troisieme-test-local-de-l-echantillon-legifacile-exclu-de-la-ci-par-son-nom.md) | Troisième test local de l'échantillon Legifacile, exclu de la CI par son nom | Accepté (option (1a) : troisième `--skip` explicite ; option (2ii) : pas de borne par défaut sur la durée, durée relevée à J3 ; accepté par anticipation le 2026-10-25, confirmé par le propriétaire le 2026-10-26) | 2026-10-25 (proposition et acceptation) |
| [ADR-0064](0064-format-et-contenu-d-un-instantane.md) | Format et contenu d'un instantané (`snapshot.json`) | Accepté (les quatre décisions telles quelles ; accepté par anticipation le 2026-10-25, confirmé par le propriétaire le 2026-10-26) | 2026-10-25 (proposition et acceptation) |
| [ADR-0065](0065-instantane-hors-ligne-epoque-de-restauration-et-retention.md) | Instantané hors ligne, époque de restauration, cible et rétention | Accepté (les six décisions telles quelles ; accepté par anticipation le 2026-10-25, confirmé par le propriétaire le 2026-10-26) | 2026-10-25 (proposition et acceptation) |
| [ADR-0066](0066-derivation-de-l-instantane-apres-commit-sans-balayage-complet.md) | Dérivation de l'instantané après commit sans balayage complet | Accepté (les quatre décisions telles que proposées : `after_commit` reçoit le delta écrit par la transaction, les deux index SQLite en recours seulement, tâche hors plan `BORNER-COUT-DU-COMMIT` en P3, chiffres du 2026-10-25 conservés et rejeu en section datée ; schéma et `format_version` inchangés) | 2026-10-26 (proposition et acceptation) |

## Questions ouvertes

Chacune deviendra un ADR quand elle sera tranchée.

| # | Question | À trancher dans |
|---|---|---|
| Q-01 | N par défaut et cible de fidélité du scan binaire | MES-01 (P4) |
| Q-02 | Fusion par défaut (`rrf` ou `relative_score`, et α) | MES-10 (P5) |
| Q-03 | Champs multivalués (`keyword[]`) nécessaires pour Legifacile ? | `DEFINIR-INDEX-LEGIFACILE` (P9) ; à remonter plus tôt si besoin |
| Q-04 | Gestion des élisions et des identifiants dans l'analyseur français | MES-11 (P5) |
| Q-05 | Répéter un en-tête court sur chaque passage (côté producteur) | Mesure en P9 |

Questions tranchées :

| # | Question | Tranchée par |
|---|---|---|
| Q-06 | Seuil `acceptance_min_cosine` pour des vecteurs encodés sur MPS ou CUDA en fp16 | ADR-0052 (2026-10-24) : `0,999`. La question est rouverte par un nouvel ADR si un producteur MPS ou CUDA échoue à la recette |
