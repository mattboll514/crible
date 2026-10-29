# ADR-0096 — Dépendances et features du serveur

- **Statut** : Accepté (accepté par anticipation le 2026-10-29, confirmé par le propriétaire le 2026-10-29, tel que rédigé)
- **Date** : 2026-10-29
- **Décideurs** : propriétaire (décision du 2026-10-24, appliquée en P6 le 2026-10-29)
- **Proposé par** : agent greffier, tâche DEMARRER-SERVEUR-AXUM (phase P6), sur la demande du propriétaire
- **Remplace** : —

## Contexte

`docs/07-stack-et-deploiement.md` nomme les crates du serveur : `axum`, `tokio`, `tower`, `tower-http`, `tracing`, `tracing-subscriber`. Il ne dit pas quelles **features** activer, et il ne nomme pas de crate pour générer un identifiant de requête.

Trois faits pèsent :

1. **`RUST_LOG` n'est pas honoré.** Le journal de `crible ingest` utilise `tracing-subscriber` avec la feature `json` et un niveau fixe `WARN` (NETTOYER-P2). La feature `env-filter` n'est pas dans `docs/07`. Le propriétaire a décidé le 2026-10-24 de traiter ce point en P6, avec `DEMARRER-SERVEUR-AXUM` (note de `docs/suivi.md`, « Idées hors périmètre », ligne « NETTOYER-P2 (décision du propriétaire) », et ligne 187 du contenu de NETTOYER-P2 : « `RUST_LOG` n'est pas honoré (niveau fixe `WARN`) : la feature `env-filter` de `tracing-subscriber` n'est pas dans `docs/07` (ADR si on la veut) »).
2. **`x-request-id` demande un identifiant.** ADR-0098 fixe la règle : reprendre l'en-tête entrant s'il est valide, sinon en générer un. `tower-http` sait propager un identifiant existant (feature `request-id`), mais ne sait pas en générer un UUID v4 conforme. Il faut donc une crate d'UUID.
3. **La confidentialité ne dépend pas du niveau de log.** INV-13 interdit le texte des requêtes, les valeurs de filtre et les ids dans les analytics (`docs/06-observabilite.md`). `docs/06:116` réservait ces contenus au niveau `trace` avant cet ADR. Avec `RUST_LOG`, ce niveau devient atteignable par simple variable d'environnement : la règle doit être tenue par le code, pas par la configuration.

## Décision

**Logs.** Activer la feature `env-filter` de `tracing-subscriber`, pour honorer `RUST_LOG`. Quand `RUST_LOG` est absent, le filtre par défaut vaut `info`. Quel que soit `RUST_LOG`, aucun log ne contient de texte de requête ni de valeur de filtre : aucun champ de ce genre n'est journalisé, à aucun niveau. `docs/06:116` est donc plus restrictif que la réalité après cette décision (voir « À surveiller »).

**Identifiant de requête.** Ajouter la crate `uuid` avec la feature `v4`, pour générer `x-request-id` (ADR-0098). La feature `request-id` de `tower-http` n'est pas activée : elle propage un identifiant, elle n'en crée pas.

**Features consignées sans décision nouvelle.** `docs/07` nomme déjà ces crates ; les features ci-dessous ne créent aucune dépendance :

- `tokio` : `rt-multi-thread`, `macros`, `signal`, `net`, `time` ;
- `tower-http` : `trace`, plus ce dont la tâche a besoin pour ses limites et ses en-têtes.

**Threads.** Les threads tokio restent aux défauts de la bibliothèque ; aucune clé de configuration nouvelle n'est ajoutée. Les calculs de recherche restent sur le pool rayon dédié, **jamais sur les threads tokio** (`docs/02-architecture.md:164`).

Ce qui n'est pas décidé ici : la version exacte des crates, épinglée par la tâche ; le format des lignes de log, déjà fixé par `docs/06` (JSON sur stdout) ; les réglages de fils d'ONNX Runtime, qui restent aux défauts (ADR-0052) et se décideront en conteneur en P8.

## Conséquences

- **Positives** :
  - `RUST_LOG` fonctionne, y compris pour cibler un module, sans reconstruire un binaire ;
  - le défaut `info` ne dépend plus d'une variable d'environnement présente dans l'image ;
  - la génération d'UUID v4 est locale, testable et sans service extérieur ;
  - INV-13 devient vrai à tous les niveaux de log, pas seulement au niveau par défaut.
- **Négatives / coûts** :
  - deux features et une crate de plus dans l'arbre de dépendances, à compiler et à auditer ;
  - `env-filter` ajoute une analyse d'expression au démarrage ;
  - `docs/06:116`, qui plaçait encore ces contenus au niveau `trace`, est corrigé par l'application de cet ADR.
- **À surveiller** : un test qui cherche un marqueur de requête dans les logs produits avec `RUST_LOG=trace` doit rester vert. C'est le signal que la confidentialité ne dépend pas de la configuration. Le test d'INV-13 de `TESTER-ANALYTICS-SANS-CONTENU` cherche aujourd'hui les marqueurs dans les logs `info` ; l'étendre à `trace` est la preuve attendue.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Garder un niveau fixe et ne pas honorer `RUST_LOG` | Empêche de diagnostiquer une instance en production sans reconstruire une image ; la décision du 2026-10-24 demande le contraire. |
| Feature `request-id` de `tower-http` | Propage un identifiant fourni, n'en génère pas ; il faudrait de toute façon une source d'UUID. |
| Générer l'identifiant avec `rand` ou un compteur maison | `rand` n'est pas dans `docs/07` et une génération maison n'apporte rien face à `uuid`, éprouvée et sans état. |
| Un identifiant monotone par processus (compteur) | Fuit l'ordre et le volume de trafic, et se confond entre deux instances ou deux redémarrages, ce qui rend le rapprochement des logs faux. |
| Régler explicitement `worker_threads` de tokio | Aucun besoin mesuré ; la règle 12 interdit la configuration d'une valeur fixe, et les calculs restent sur rayon. |

## Complément du 2026-10-29 (relecture de DEMARRER-SERVEUR-AXUM)

La relecture de `DEMARRER-SERVEUR-AXUM` a relevé trois features employées par le code et absentes de la liste de la décision. Elles sont **consignées** ici, sans décision nouvelle de crate : les trois crates (`tokio`, `tower`, `tower-http`) figurent déjà dans `docs/07`. Les décisions ci-dessus ne changent pas.

| Crate | Feature | Dépendance | Emploi |
|---|---|---|---|
| `tokio` | `sync` | normale (`[workspace.dependencies]` de `Cargo.toml`, racine) | Canal `watch` du signal d'arrêt et de l'état prêt du serveur |
| `tokio` | `io-util` | **de développement seulement** (`[dev-dependencies]` de `crates/crible-server/Cargo.toml`) | `AsyncReadExt` / `AsyncWriteExt` : les tests du serveur parlent HTTP sur une socket |
| `tower` | `util` | **de développement seulement** (`[dev-dependencies]` de `crates/crible-server/Cargo.toml`) | `ServiceExt` (`oneshot`) : tests du routeur et du service sans socket |

Aucune de ces features n'ajoute de crate à l'arbre de dépendances, et aucune n'est active dans le binaire de production : `io-util` et `util` ne servent qu'aux tests. La liste de la décision se lit donc avec ce tableau pour être complète, et `docs/07` (ligne HTTP du tableau des dépendances) est complétée en conséquence.

### Suite du complément : relecture de ROUTES-DOCUMENTS-ET-LOTS

**Statut de cette suite** : acceptée par anticipation par le propriétaire le 2026-10-29, à confirmer à la revue de fin de série. Elle ne change aucune décision ci-dessus, ni le statut de l'ADR.

La relecture de `ROUTES-DOCUMENTS-ET-LOTS` ajoute **deux entrées** de même nature : une dépendance déjà nommée par `docs/07`, et une feature du paquet serveur.

| Crate | Feature | Dépendance | Emploi |
|---|---|---|---|
| `serde` | `derive` | normale (`[dependencies]` de `crates/crible-server/Cargo.toml`), même déclaration que `crible-core` et `crible-store` | Décoder les corps des routes d'écriture dans les types de `crible-store` qui dérivent `Deserialize` (ADR-0107) |
| `crible-server` | `failpoints` | **feature du paquet**, éteinte par défaut ; active `crible-store/failpoints` | Prouver la reprise d'ADR-0108 au niveau du serveur par le point de panne `after_epoch_switch` de la compaction ; les deux étapes de CI correspondantes sont fixées par le complément du 2026-10-29 à ADR-0068 |

`serde` avec `derive` ne crée aucune dépendance : la ligne « Config » de `docs/07:48` nomme déjà `serde` pour le serveur, `crible-core` et `crible-store`, avec la même déclaration. La feature `failpoints` du serveur n'ajoute pas non plus de crate : elle n'active celle de `crible-store` que sous la ligne de commande qui la demande, et le binaire de production ne la reçoit pas.

## Références

`docs/07-stack-et-deploiement.md` (tableau des dépendances, Docker) ; `docs/06-observabilite.md` (santé et logs, INV-13) ; `docs/02-architecture.md:164` (calculs sur le pool rayon) ; `docs/suivi.md`, « Idées hors périmètre » (2026-10-24, `RUST_LOG`) et contenu de NETTOYER-P2, ligne 187 ; ADR-0098 (`x-request-id`) ; ADR-0052 (défauts d'ONNX Runtime) ; ADR-0057 (job `rust-onnx`).
