# ADR-0097 — Configuration de l'instance : surcharges `CRIBLE_*`, chemin de `crible.toml`, options de `crible serve`

- **Statut** : Accepté (accepté par anticipation le 2026-10-29, confirmé par le propriétaire le 2026-10-29, tel que rédigé)
- **Date** : 2026-10-29
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, tâche DEMARRER-SERVEUR-AXUM (phase P6), sur la demande du propriétaire et le verdict de `@architecte` du 2026-10-29
- **Remplace** : —

## Contexte

`docs/07-stack-et-deploiement.md` décrit la configuration de l'instance : un fichier `crible.toml` avec les sections `[server]`, `[paths]`, `[[auth.keys]]`, `[analytics]`, `[debug_journal]`, `[encoder]` et `[limits]`. Deux phrases seulement règlent les surcharges : « Chaque clé peut être surchargée par une variable d'environnement, par exemple `CRIBLE_SERVER__BIND` » (section « Configuration de l'instance »), et le tableau des dépendances, à la ligne « Config », attribue ces surcharges à **ADR-0027**.

ADR-0027 ne décide que l'ajout de `serde_json` : son corps ne dit rien des variables d'environnement. Le renvoi de `docs/07` (ligne « Config » du tableau des dépendances, `:46` avant application du présent ADR) est donc faux, et ADR-0051 le reprend à son tour (`docs/adr/0051-…:139`, « ADR-0027 (configuration `serde` et surcharges `CRIBLE_*`) »).

Trois points restent ouverts, et `DEMARRER-SERVEUR-AXUM` ne peut pas les inventer :

1. la **forme** exacte d'une surcharge, au-delà de l'exemple ;
2. le **sort d'une faute** : clé inconnue, valeur invalide, ou tentative de surcharger une table de tableaux ;
3. le **chemin du fichier** et les **options de la commande** `crible serve`.

Le projet a déjà tranché un cas voisin : ADR-0031 refuse une clé inconnue dans un `PATCH` de configuration, pour qu'une faute de frappe ne passe pas sans effet.

## Décision

**Forme d'une surcharge.** `CRIBLE_` suivi du chemin de la clé en majuscules, les sections séparées par `__`. Exemples : `CRIBLE_SERVER__BIND`, `CRIBLE_LIMITS__QUERY_CACHE_ENTRIES`. La valeur est analysée comme un **scalaire** du type de la clé visée : entier, booléen, chaîne, chaîne d'énumération. Une valeur qui ne s'analyse pas dans ce type est refusée.

**Refus au démarrage.** Le serveur refuse de démarrer, avec un message qui **nomme la variable** fautive et un code de sortie non nul, dans trois cas :

- la clé visée n'existe pas dans `crible.toml` ;
- la valeur ne s'analyse pas dans le type de la clé ;
- la variable vise un **tableau de tables**, par exemple `[[auth.keys]]` : ces tables ne se surchargent pas par l'environnement en v1.

Le raisonnement est celui d'ADR-0031 : une faute de frappe ne doit pas passer sans effet. Un serveur qui ignore silencieusement `CRIBLE_SERVER__BND` démarre avec une adresse que personne n'attend.

**Liste fermée des variables exclues.** Certaines variables `CRIBLE_*` sont documentées **hors configuration** et ne désignent aucune clé de `crible.toml`. Elles échappent au contrôle précédent. La liste est fermée :

| Variable | Emploi |
|---|---|
| `CRIBLE_FAILPOINT` | Point d'injection des tests de panne, en build de test seulement (`docs/08-invariants-et-tests.md:35`, ADR-0068) |
| `CRIBLE_BIN` | Chemin du binaire `crible` pour les tests Python (`clients/python/tests/test_online_batch.py`) |
| `CRIBLE_PYTHON` | Interpréteur Python des tests de recette (`crates/crible/tests/recette_import.rs`) |
| `CRIBLE_EXPORT_REPRO` | Rejeu opt-in de la reproductibilité de l'export (`tools/README.md`, ADR-0050) |
| `CRIBLE_CRASH_CHILD` | Sélection du processus enfant des tests de panne (`crates/crible-store/tests/inv_04_crash_recovery.rs`) |
| `CRIBLE_CRASH_BATCH` | Identifiant du lot des tests de panne (même fichier) |
| `CRIBLE_CRASH_INDEX` | Index des tests de panne (même fichier) |

Toute autre variable `CRIBLE_*` inconnue est une faute et fait échouer le démarrage. Une variable de cette liste présente dans l'environnement du serveur est ignorée sans erreur. `CRIBLE_DEPOT`, cité par une note de `docs/suivi.md` (`CONSTITUER-ECHANTILLON-LEGIFACILE`), n'est lu par aucun script aujourd'hui : il n'est pas dans la liste.

**Commande et chemin du fichier.** `crible serve [--config <chemin>]`, chemin par défaut `/etc/crible/crible.toml`. Un fichier absent ou illisible fait échouer le démarrage, avec un code de sortie non nul : le serveur ne démarre jamais sur des valeurs par défaut implicites. Les chemins `data`, `models` et `exchange` viennent de la section `[paths]` ; ils ne sont pas déduits de l'emplacement du fichier, et il n'y a pas d'option de ligne de commande pour les remplacer.

Ce qui n'est pas décidé ici : le contenu de chaque clé, décrit par `docs/07` et les ADR qui les fixent ; la façon de recharger la configuration à chaud, qui n'existe pas en v1.

## Conséquences

- **Positives** :
  - un conteneur se règle par variables d'environnement sans éditer le fichier monté en lecture seule ;
  - une faute de frappe se voit au démarrage, pas trois semaines plus tard ;
  - la liste des variables hors configuration est écrite une fois, et le contrôle est mécanique ;
  - le renvoi de `docs/07` désigne enfin l'ADR qui décide.
- **Négatives / coûts** :
  - une variable `CRIBLE_*` nouvelle employée hors configuration demande la mise à jour de cette liste ;
  - l'interdiction de surcharger `[[auth.keys]]` oblige à éditer le fichier pour changer une clé d'API, ou à en ajouter une : c'est le prix d'un contrôle simple en v1 ;
  - un fichier de configuration absent n'a aucun repli : c'est voulu, mais l'image doit fournir le fichier, ce que le volume monté de `docs/07` fait déjà.
- **À surveiller** : si le besoin d'ajouter une clé d'API par l'environnement apparaît (par exemple pour un secret d'orchestration), il faudra un ADR qui rouvre la surcharge des tableaux de tables. **Point ouvert** : le renvoi d'ADR-0051 (`:139`) attribue encore les surcharges `CRIBLE_*` à ADR-0027 ; sa correction est un **erratum** qui demande une décision du propriétaire, et le corps d'ADR-0051 n'est pas modifié ici.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Séparateur `_` simple ou `.` | `CRIBLE_SERVER_BIND` est ambigu dès qu'une clé contient un souligné, cas fréquent (`query_cache_entries`) ; le point n'est pas un caractère d'environnement portable. |
| Accepter une clé inconnue en l'ignorant | Une faute de frappe passerait sans effet ; c'est exactement ce qu'ADR-0031 refuse pour un `PATCH`. |
| Surcharger `[[auth.keys]]` par un JSON dans une variable | Introduit un second format de configuration, avec son propre analyseur et ses propres erreurs, pour un besoin non exprimé. |
| Chercher `crible.toml` dans le dossier courant, puis dans `/etc` | Deux emplacements possibles rendent le diagnostic ambigu : le serveur lirait un fichier que l'exploitant ne regarde pas. |
| Exiger `--config` sans valeur par défaut | Le conteneur de `docs/07` monte le fichier à un emplacement fixe ; une valeur par défaut évite un drapeau dans chaque ligne de commande. |

## Complément du 2026-10-29 (relecture de DEMARRER-SERVEUR-AXUM)

Deux précisions, qui ne changent pas la décision.

**Les scalaires acceptés.** La décision parle de « scalaire du type de la clé visée ». La liste est fermée :

| Scalaire | Exemple de clé |
|---|---|
| Entier | `CRIBLE_LIMITS__QUERY_CACHE_ENTRIES` |
| Nombre flottant | `CRIBLE_DEBUG_JOURNAL__SAMPLE_RATE` (`docs/07`, valeur par défaut `0.0`) |
| Booléen | les clés de bascule de `crible.toml` |
| Chaîne | `CRIBLE_SERVER__BIND` |
| Chaîne d'énumération | les clés à valeurs nommées |

Le **nombre flottant** manquait à la liste. Il est nécessaire : `[debug_journal].sample_rate` vaut `0.0` par défaut (`docs/07`, section « Configuration de l'instance »), et une valeur telle que `0.25` doit pouvoir être surchargée sans éditer le fichier monté en lecture seule. L'analyse se fait en virgule flottante décimale, sans séparateur de milliers ni notation dépendant de la locale.

**Variables non UTF-8.** Une variable d'environnement de préfixe `CRIBLE_` dont le **nom** ou la **valeur** n'est pas en UTF-8 est refusée au démarrage comme une variable invalide, avec le même code de sortie non nul. Son nom est cité dans le message sous une forme approchée (substitution des octets non décodables), faute de pouvoir l'écrire exactement. Le but est qu'aucune variable `CRIBLE_*` ne soit ignorée en silence : le système d'exploitation permet des octets arbitraires dans l'environnement, et le contrôle de la décision, qui lit des chaînes, ne verrait pas ces variables sans cette règle.

## Références

`docs/07-stack-et-deploiement.md` (sections « Configuration de l'instance » et « Docker ») ; `docs/08-invariants-et-tests.md:35` (`CRIBLE_FAILPOINT`) ; `docs/suivi.md` (« Idées hors périmètre », `CRIBLE_DEPOT`) ; ADR-0031 (clé inconnue refusée) ; ADR-0027 (renvoi erroné à corriger) ; ADR-0051 (`:139`, renvoi erroné à corriger) ; ADR-0068 (failpoints) ; ADR-0050 (export) ; fiche `DEMARRER-SERVEUR-AXUM`.
