# ADR-0109 — Route de recherche : champs inconnus, périmètre de l'échéance et preuve « API = CLI »

- **Statut** : Accepté (accepté par anticipation par le propriétaire le 2026-10-29, confirmé par le propriétaire le 2026-10-30)
- **Date** : 2026-10-29
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, tâche ROUTE-RECHERCHE (phase P6), sur délégation du propriétaire pour la série des ADR-0103 à ADR-0109 et les verdicts de `@architecte` du 2026-10-29
- **Remplace** : — (ne remplace aucune décision : complète ADR-0102, et complète ADR-0057, ADR-0062, ADR-0063 et ADR-0085, sans modifier le corps d'aucun d'eux)

## Contexte

### La fiche et ce qu'elle demande

La fiche `ROUTE-RECHERCHE` (`docs/phases/P6-serveur-et-api.md:99-111`) crée `POST /v1/indexes/{index}/search`. Elle demande tous les champs de la requête « sauf `debug` et `explain_ids` (P7) », acceptés et validés (`:104`) ; la réponse de `docs/05` avec `generation`, `pool_size`, `took_ms` et `truncated_query` (`:105`) ; les calculs sur le pool rayon, jamais sur les threads tokio (`:106`). Ses critères d'acceptation sont au nombre de quatre : l'identité des résultats avec la CLI « sur les requêtes de l'échantillon » (`:108`), les trois rôles atteignant la route (`:109`), `timeout_ms` lu dans le corps avec `504 timeout` et jeton de concurrence tenu (`:110`), et `503 overloaded` au-delà de `max_concurrent_searches` (`:111`).

`docs/05-api.md:228-249` décrit le corps de la requête par un tableau fermé de quatorze champs ; `:251-303` décrit la réponse, dont les objets `debug` et `explain`. Le verdict de `@architecte` du 2026-10-29 relève deux **NON-COUVERT** et un **ECART**, traités ci-dessous.

### 1. Le champ inconnu du corps de recherche

`docs/05:79` tranche le cas d'une clé inconnue pour le `PATCH /v1/indexes/{index}` seulement, en application d'ADR-0031 : refus en `invalid_request` (400), message nommant la clé et son **chemin JSON**, à tous les niveaux du corps. Rien n'est écrit pour le corps d'une recherche.

Ce corps est un objet imbriqué : `filter` (grammaire de `:206-226`), `fusion` (`:239`), `return` (`:243`). Une faute de frappe — `fusion: { "method": "rrf", "alpah": 0.3 }`, `return: { "field": "all" }` — passerait donc sans effet, et le client croirait avoir réglé ce qu'il n'a pas réglé. C'est exactement le défaut qu'ADR-0031 a fermé pour le PATCH, et deux règles différentes pour deux corps voisins se contrediraient.

### 2. `debug` et `explain_ids` en P6

La fiche les exclut de P6 et les renvoie à P7 (`:104`) ; P7 les livre par `REPONSE-DEBUG` et `EXPLIQUER-DOCUMENTS` (`docs/phases/P7-observabilite.md:52-70`). Mais `docs/05:245-246` les décrit comme des champs de la requête, avec un défaut (`false`, `[]`), et `:273-302` décrit ce que la réponse y ajoute.

La fiche ne dit pas ce que la route fait d'un corps qui les porte : les refuser, ou les accepter sans effet. La seconde branche est une **ignorance silencieuse** : le client demanderait un diagnostic et recevrait une réponse sans `debug`, sans erreur, donc sans savoir si le champ est inconnu, ignoré ou vide. C'est le NON-COUVERT du verdict.

### 3. Ce que couvrent l'échéance de `timeout_ms` et le jeton

ADR-0102, décision 3, fixe la valeur (`docs/05:247`, 1 à 30 000 ms, `[limits].default_timeout_ms` par défaut), le `504 timeout` à l'échéance, et le fait que le calcul rayon n'est **pas** annulé et que « son jeton de sémaphore reste pris jusqu'à sa fin réelle » (`:49`). Sa section « Ce qui n'est décidé ici » laisse ouvert « le point exact où part le chronomètre de `timeout_ms` — validation et authentification comprises ou non », « que ROUTE-RECHERCHE fixe au plus tard » (`:63`). C'est le second NON-COUVERT.

La question est plus large qu'un point de départ, parce qu'une recherche n'est pas seulement le calcul rayon. Elle encode la requête ou la sert depuis le cache d'ADR-0051, planifie la stratégie dense (ADR-0074), évalue le pool, exécute les deux branches, fusionne (ADR-0008, ADR-0090), regroupe (ADR-0075) et lit les payloads. Si l'échéance ne couvrait que le rayon, une requête dont l'encodage ONNX prend l'essentiel du temps dépasserait `timeout_ms` sans `504` ; et le `503` d'ADR-0102 mentirait, puisqu'il annoncerait une capacité occupée par un calcul que l'échéance prétend borner.

`took_ms` mesure, dans la CLI, le **total depuis l'entrée de la commande** : le commentaire de `crible search` dit « `took_ms` is the total, from the entry of the command » et énumère la configuration lue, l'encodage, le filtre, l'évaluation du pool, l'ouverture des lecteurs et la recherche (`crates/crible/src/commands.rs:721-732`), et la valeur est `started.elapsed()` (`:843`). La route doit mesurer la même chose, sinon les deux nombres ne se comparent pas et le critère `:108` devient invérifiable.

### 4. La preuve « API = CLI »

Le critère `docs/phases/P6:108` demande que « les résultats de l'API sont identiques à ceux de la CLI sur les requêtes de l'échantillon ». L'échantillon est `data/legifacile-sample` : 300 questions étiquetées dans `queries.jsonl`, jeu gitignoré qui dérive de la production (ADR-0032). La CI ne peut donc pas l'exécuter, et le critère n'aurait aucune preuve continue.

Par ailleurs, la CI nommait, avant application de cet ADR, **quatre** tests locaux exclus un par un (`docs/07-stack-et-deploiement.md:147` et `:150`, `docs/08-invariants-et-tests.md:48`) : ils viennent d'ADR-0057, d'ADR-0062, d'ADR-0063 et d'ADR-0085. ADR-0062, décision 3, pose la règle : « Aucun autre test n'est exclu, et aucune règle générale d'exclusion n'est créée » (`docs/adr/0062-second-test-local-de-l-echantillon-legifacile-exclu-de-la-ci-par-son-nom.md:99`). ADR-0085 la reprend et prévoit ce cas : « si un cinquième test de l'échantillon apparaît, ne pas improviser : rouvrir cet ADR plutôt que d'ajouter une exclusion sans décision » (`docs/adr/0085-quatrieme-test-local-de-l-echantillon-legifacile-exclu-de-la-ci-par-son-nom.md:103`). Le cinquième test demandé par la fiche demande donc une décision : c'est l'ECART du verdict, et l'objet de la décision 3.

## Décision

**Décision 1 — champs inconnus.** Un champ du corps de `POST /v1/indexes/{index}/search` absent de `docs/05` (« Recherche : requête », `:232-247`) est refusé en **`invalid_request` (400)**, à tous les niveaux du corps, et le message nomme le **chemin JSON** du champ — même règle qu'ADR-0031 pour le `PATCH` de configuration.

Précisions :

- le contrôle descend dans les objets imbriqués : `fusion`, `return`, et les objets de `filter` ;
- dans `filter`, un nom de champ, un opérateur ou une valeur hors de la grammaire reste `invalid_filter` (400), avec `details.path` (`docs/05:34`, `:39`) : son code ne change pas ;
- la valeur d'un champ **connu** reste validée par sa propre règle, par exemple `alpha` hors de `[0, 1]` en `invalid_request` (ADR-0090) ou `k` hors de `1..=1000` ;
- en P6, `debug` et `explain_ids` ne font pas partie de ce que la route accepte : ils sont donc refusés de la même façon, avec un message qui dit que le champ n'est pas encore pris en charge. P7 les ouvrira par `REPONSE-DEBUG` et `EXPLIQUER-DOCUMENTS`. **Jamais d'ignorance silencieuse.**

**Décision 2 — échéance et jeton.** Une recherche prend son jeton de `[server].max_concurrent_searches` **avant tout travail** (ADR-0102, décision 2, inchangée). L'échéance de `timeout_ms` (ADR-0102, décision 3) couvre **tout le pipeline** : encodage de la requête, cache d'ADR-0051 compris, planification, branches dense et lexicale sur le pool rayon `search_threads` (`docs/07:68`), fusion, regroupement et lecture des payloads. Le jeton est tenu jusqu'à la **fin réelle** du calcul, même après un `504` : ADR-0102 ne change pas sur ce point.

`took_ms` mesure la même durée, **de la prise du jeton à la fin du calcul**, comme la CLI mesure le total depuis l'entrée de la commande (`crates/crible/src/commands.rs:721-732`, `:843`).

L'authentification et le décodage du corps précèdent la prise du jeton, donc l'échéance : elles restent bornées par leurs propres règles (comparaison de clé sans attente, ADR-0100 ; corps de 32 Mio, ADR-0102 décision 1). Le point laissé ouvert par ADR-0102 (`:63`) est ainsi fixé : le chronomètre part à la prise du jeton.

**Décision 3 — la preuve « API = CLI ».**

**(a) Un cinquième test local**, `#[ignore]`, nommé **`api_and_cli_return_the_same_results_on_the_legifacile_sample`** — nom à reprendre tel quel, comme les quatre autres. Sur `data/legifacile-sample`, pour **chaque question étiquetée** de `queries.jsonl` et pour les modes `dense`, `lexical` et `hybrid` à `k = 10`, il compare la réponse de la route et la sortie JSON de `crible search` (`--format json`, `crates/crible/src/commands.rs:855-875`) : mêmes clés, même ordre, mêmes scores **au bit près**, même `generation`, même `truncated_query`. `took_ms` et `request_id` sont hors de la comparaison : le premier est une durée par construction différente, le second n'existe que dans l'API (`docs/05:263-265`).

Le test demande ONNX Runtime et le vrai modèle. Il rejoint les quatre exclus par leur nom dans le job `rust-onnx`, qui porte alors **cinq** `--skip`. Il vit dans `crates/crible/tests/`, le seul paquet qui dépend à la fois de `crible-server` et de la CLI (`crates/crible/Cargo.toml`) ; il n'ajoute aucun crate (`docs/07`).

**(b) La même comparaison tourne en CI**, sur un petit index construit dans le test, sans l'échantillon, dans le job qui peut l'exécuter : le job Rust si un index à modèle `external` avec un `vector` dans la requête suffit (`docs/04-donnees-et-ingestion.md:119` : pas d'encodeur, pas de recette, `vector` obligatoire par requête ; `docs/05:235`), le job `rust-onnx` avec le modèle de test d'ADR-0057 sinon.

C'est la **preuve continue** du critère ; (a) en est la preuve sur données réelles, rejouée par `@jalon`. Le job Rust reste sans ONNX Runtime (`docs/07:144`) : un test qui en a besoin appartient à `rust-onnx`, jamais au job Rust.

Ce qui n'est pas décidé ici :

- le corps de `POST /v1/search` (fiche `RECHERCHE-MULTI-INDEX`, `docs/phases/P6:113-127`, `docs/05:304-318`) n'est pas couvert par la décision 1 : une divergence demanderait un ADR ;
- le texte exact du message d'un champ non pris en charge ;
- la borne de durée du test (a), laissée ouverte comme ADR-0062 et ADR-0063 l'ont fait pour leurs tests ;
- la mise à jour de `.github/workflows/ci.yml` : la cinquième ligne `--skip` arrive **avec** le test, dans la tâche ROUTE-RECHERCHE (voir « Documents mis à jour par cet ADR »).

## Conséquences

- **Positives** :
  - un client ne peut plus croire avoir réglé ce qu'il n'a pas réglé : la faute de frappe dans `fusion`, `return` ou un objet de `filter` est nommée, avec son chemin, comme dans un `PATCH` (ADR-0031) ;
  - `debug` et `explain_ids` refusés en P6 disent la vérité ; P7 les ouvrira sans changer la règle, donc sans rupture pour un client qui suit la documentation ;
  - l'échéance ne ment pas : ce que `timeout_ms` borne est exactement ce que le jeton occupe, encodage compris ;
  - `took_ms` et le temps de la CLI mesurent la même chose, ce qui rend le critère de la fiche comparable et le diagnostic d'un écart possible ;
  - le critère « API = CLI » a une preuve continue (b) sur un index de test, et une preuve sur les données réelles (a) rejouée à chaque jalon ;
  - l'exclusion du cinquième test reste nommée et justifiable une par une, comme les quatre autres (`docs/08:43`, `:48`).
- **Négatives / coûts** :
  - un client P6 qui envoie `debug: true` reçoit `400` : c'est un refus, pas une réponse dégradée ;
  - le contrôle des champs inconnus ajoute une validation par niveau, à tenir à jour quand `docs/05` gagne un champ ;
  - la commande du job `rust-onnx` s'allonge d'un cinquième `--skip`, et elle est dupliquée entre `.github/workflows/ci.yml` et `docs/07` (ADR-0085) ;
  - le test (b) ajoute sa durée au job qui l'accueille, et un index à construire par exécution ;
  - (a) n'est pas prouvé à la fusion : une régression de l'égalité API/CLI ne sera vue qu'au passage local ou à `@jalon` ;
  - les phrases qui comptent les tests exclus vieillissent encore une fois ; un ADR accepté n'est pas réécrit, et c'est celui-ci qui fait foi pour les cinq noms.
- **À surveiller** :
  - `docs/07:147` décrit la cible à cinq `--skip` ; `ci.yml:140` en porte encore quatre tant que le test n'existe pas. La cinquième ligne doit être ajoutée **dans le même commit** que le test, sinon la documentation décrit autre chose que ce que la CI exécute ;
  - le nom `api_and_cli_return_the_same_results_on_the_legifacile_sample` est un **contrat** : le renommer sans son `--skip` le ferait exécuter en CI, où `data/legifacile-sample` manque ;
  - si (b) finit par demander ONNX Runtime alors qu'il a été placé dans le job Rust, c'est le test qui change de job, pas le job qui gagne ONNX ;
  - la part des `504` due à l'encodage : si elle domine, une échéance séparée pour l'encodage redevient une question, à trancher par un ADR ;
  - la fréquence des `400` pour `debug` en P6 : un client légitime qui en envoie dirait que P7 est à avancer, pas que la règle est à changer ;
  - toute phrase qui cite un nombre de tests `#[ignore]`, exclus ou exécutés doit être relue (ADR-0085, « À surveiller ») : `docs/07:150`, `docs/08:48` et la note d'ADR-0085 en font partie.

- **Documents mis à jour par cet ADR** :

| Document | Modification |
|---|---|
| `docs/05-api.md:228-249` | « Recherche : requête » : paragraphe sur le champ inconnu et sur `debug`/`explain_ids` refusés en P6 |
| `docs/05-api.md:263`, `:269` | « Recherche : réponse » : ce que mesure `took_ms` |
| `docs/05-api.md:352`, `:356` | « Limites » : ce que couvre l'échéance, et le point de départ du chronomètre |
| `docs/07-stack-et-deploiement.md:147` | commande du job `rust-onnx` avec cinq `--skip` |
| `docs/07-stack-et-deploiement.md:150` | « les cinq exclus », nommés un par un |
| `docs/08-invariants-et-tests.md:48` | même liste, à cinq noms ; la règle de saut ne change pas |
| `docs/adr/0085-…md` | note « Complément du 2026-10-29 » de renvoi, corps inchangé |
| `docs/adr/README.md` | ligne d'index de cet ADR |

Sont **non modifiés** : `.github/workflows/ci.yml` (la cinquième ligne `--skip` arrive avec le test, dans ROUTE-RECHERCHE), `docs/suivi.md` (tenu par l'orchestrateur), et tout fichier de code.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Ignorer les champs inconnus du corps de recherche | Une faute de frappe passerait sans effet et le client croirait avoir réglé ce qu'il n'a pas réglé ; le diagnostic deviendrait coûteux. C'est le cas qu'ADR-0031 a fermé pour le `PATCH`, et le corps de recherche a les mêmes objets imbriqués. |
| Refuser les champs inconnus au seul premier niveau | Les fautes visées vivent dans les objets imbriqués (`alpah` dans `fusion`, `field` dans `return`), donc un contrôle du seul premier niveau laisserait passer exactement les cas que la décision vise. |
| Accepter `debug` et `explain_ids` sans effet en P6 | Le client demanderait un diagnostic et recevrait une réponse muette : champ inconnu, champ ignoré et champ vide seraient indistinguables, ce que la décision 1 refuse par ailleurs. |
| Répondre `debug: {}` ou `explain: []` quand le champ est présent | Une réponse vide se lit comme un résultat : elle ferait croire que le plan a été calculé et qu'aucun document demandé n'existe, ce qui est faux dans les deux cas. |
| Échéance limitée au calcul rayon | Le `504` ne couvrirait ni l'encodage ONNX ni le cache ; une requête lente à encoder rendrait `200` après son échéance, et le `503` d'ADR-0102 annoncerait une capacité occupée par un travail que l'échéance prétend borner. |
| Faire partir l'échéance avant l'authentification et le décodage du corps | Ces deux étapes sont bornées par leurs propres règles (ADR-0100, ADR-0102 décision 1) et le jeton n'est pas encore pris : un `504` rendu à ce moment ne garantirait rien sur la tenue du jeton, et un `401` ou un `400` dépendrait d'un délai de recherche. |
| Réduire le critère à `synth-small` | La fiche nomme « les requêtes de l'échantillon » (`docs/phases/P6:108`) ; `synth-small` n'a ni questions étiquetées ni les structures de production. Réduire le critère serait un écart à la fiche, pas une simplification. |
| Un test local seulement, sans preuve en CI | L'échantillon ne peut pas entrer en CI (ADR-0032) : le critère n'aurait aucune preuve continue, et une régression de l'égalité API/CLI ne serait vue qu'au passage local, ou jamais. |
| Un test en CI seulement, sans le test local | Il ne prouverait l'égalité que sur un index construit par le test, pas sur les données réelles que la fiche cite ; la comparaison sur l'échantillon est ce que `@jalon` rejoue. |
| Un motif commun d'exclusion pour les tests de l'échantillon | Déjà écarté par ADR-0062, ADR-0063 et ADR-0085 : `--skip` filtre par sous-chaîne, un tel motif exclurait aussi `inv_12_the_legifacile_sample_test_fails_without_its_prerequisites` et ferait un saut silencieux (`docs/08:43`, `:48`). |

## Références

- `docs/phases/P6-serveur-et-api.md:99-111` — fiche `ROUTE-RECHERCHE` ; `:104` (`debug` et `explain_ids` renvoyés à P7), `:105` (réponse), `:108` (critère « API = CLI »), `:110` (`timeout_ms`), `:111` (`503 overloaded`) ; `:113-127` — fiche `RECHERCHE-MULTI-INDEX`.
- `docs/phases/P7-observabilite.md:52-70` — `REPONSE-DEBUG` et `EXPLIQUER-DOCUMENTS`.
- `docs/05-api.md` : `:34` et `:39` (corps d'erreur, `details.path`, `invalid_filter`), `:79` (clé inconnue d'un `PATCH`, ADR-0031), `:206-226` (grammaire de `filter`), `:228-249` (« Recherche : requête »), `:235` (`vector` obligatoire pour un modèle `external`), `:251-303` (« Recherche : réponse », `took_ms` à `:263` et `:269`, `request_id` à `:265`), `:273-302` (`debug`, `explain`), `:304-318` (multi-index), `:342-356` (« Limites par défaut »).
- `docs/07-stack-et-deploiement.md` : `:68` (`[server].search_threads`), `:144` (job Rust sans ONNX Runtime), `:146-148` (commande du job `rust-onnx`), `:150` (les cinq exclus nommés un par un).
- `docs/08-invariants-et-tests.md:43` (`#[ignore]` seul mécanisme de saut, échec franc sans prérequis), `:48` (exclusion nommée un par un).
- `docs/04-donnees-et-ingestion.md:119` — modèle `kind: "external"` : pas d'encodeur, pas de recette, `vector` obligatoire par requête.
- `crates/crible/src/commands.rs:721-732` (commentaire de `crible search`, « `took_ms` is the total, from the entry of the command »), `:843` (`started.elapsed()`), `:855-875` (forme JSON de la réponse de la CLI) ; `crates/crible/Cargo.toml` (dépendances vers `crible-server` et la CLI) ; `crates/crible-server/Cargo.toml` (pas de client HTTP en dépendance).
- `data/legifacile-sample/queries.jsonl` — 300 questions étiquetées, jeu local (ADR-0032, ADR-0062).
- ADR-0031 (clé inconnue d'un `PATCH`, `invalid_request` nommant le chemin) ; ADR-0051 (cache des requêtes) ; ADR-0057 (ONNX Runtime et vrai modèle en CI, modèle de test) ; ADR-0062 (second `--skip` explicite, décision 3 : aucun autre test exclu) ; ADR-0063 (troisième `--skip`) ; ADR-0074 (planification de la stratégie dense) ; ADR-0075 (regroupement) ; ADR-0085 (quatrième `--skip`, note de renvoi) ; ADR-0090 (`alpha` et fusion `relative_score`) ; ADR-0100 (clés d'API) ; ADR-0101 (corps d'erreur) ; ADR-0102 (limites, échéance, jeton) ; ADR-0108 (forme des décisions de la même série).
- Verdicts de `@architecte` du 2026-10-29 sur la fiche `ROUTE-RECHERCHE` : deux `NON-COUVERT` (champ inconnu du corps de recherche, `debug`/`explain_ids` en P6 ; périmètre de `timeout_ms` et du jeton) et un `ECART` (critère « API = CLI » et cinquième test local).
