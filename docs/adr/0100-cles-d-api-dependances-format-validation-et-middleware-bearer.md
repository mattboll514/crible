# ADR-0100 — Clés d'API : dépendances, format, validation et middleware `Bearer`

- **Statut** : Accepté (accepté par anticipation le 2026-10-29, confirmé par le propriétaire le 2026-10-29, tel que rédigé)
- **Date** : 2026-10-29
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, tâche AUTHENTIFIER-CLES (phase P6), sur demande du propriétaire et le verdict de `@architecte` du 2026-10-29
- **Remplace** : —

## Contexte

`docs/05-api.md:6` fixe la convention d'authentification en une phrase : en-tête `Authorization: Bearer <clé>`, clés déclarées dans `crible.toml` **sous forme de hash SHA-256**, avec un nom et un rôle. `docs/05-api.md:8-12` donne la hiérarchie des rôles (`read`, `write`, `admin`), et `docs/05-api.md:52-74` attribue à chaque route un rôle minimal, ou « — » pour `/health`, `/ready`, `/openapi.json` et `/docs`.

`docs/07-stack-et-deploiement.md:76-84` montre la forme de la configuration :

```toml
[[auth.keys]]
name = "legifacile-mcp"
role = "read"
sha256 = "…"                  # généré par `crible key generate`
```

La fiche `AUTHENTIFIER-CLES` (`docs/phases/P6-serveur-et-api.md:29-41`) demande un middleware `Bearer` qui hache la clé reçue en SHA-256, la compare en temps constant aux hachés de la configuration, applique les trois rôles, attache le nom de la clé à la requête, et livre `crible key generate`.

Six points ne sont tranchés nulle part, et la tâche ne peut pas les inventer :

1. **Les dépendances.** Le tableau des dépendances Rust de `docs/07` (`:29-50`) ne contient qu'une crate de hash, `blake3` (`:43`), dédiée aux empreintes, aux checksums et au `passage_hash`. Aucune crate ne fournit SHA-256, aucune ne fournit de comparaison en temps constant, aucune ne fournit d'aléa cryptographique. La règle 12 des `AGENTS.md` exige un ADR pour toute dépendance absente de `docs/07`.
2. **Le format de la clé et du haché.** `docs/07` dit seulement que `sha256` est « généré par `crible key generate` ». Ni la longueur, ni l'alphabet, ni le préfixe, ni l'objet exact du hachage ne sont écrits.
3. **La validation de `[[auth.keys]]` au démarrage.** `docs/07` ne dit pas ce qui se passe pour un `sha256` mal formé, deux noms identiques, deux hachés identiques, un rôle inconnu, ou une liste vide. `ADR-0097` établit le précédent : une faute de configuration se voit au démarrage, avec un message qui nomme l'élément fautif et un code de sortie non nul. Il exclut aussi ces tables de la surcharge par l'environnement.
4. **L'analyse de l'en-tête `Authorization`.** Casse du schéma, nombre d'espaces, clé vide, en-tête répété : rien n'est écrit.
5. **Les réponses `401` et `403`.** `docs/05-api.md:37-38` donne les codes (`unauthorized`, `forbidden`) mais pas l'en-tête `WWW-Authenticate`, ni le contenu du corps, ni la règle qui empêche de distinguer une clé absente d'une clé inconnue. Le format d'erreur général (`docs/05-api.md:26-30`) est livré par `FORMAT-ERREURS-ET-LIMITES`, tâche postérieure.
6. **Les logs.** La fiche exige que « la clé en clair n'apparaisse jamais dans les logs ». `docs/06-observabilite.md:17` agrège une dimension `client`, et `:68` enregistre le client dans le journal de debug. `INV-13` et `ADR-0096` interdisent le texte de requête et les valeurs de filtre à tout niveau de log. Ni la clé, ni son haché, ni l'en-tête `Authorization` ne sont nommés.

Deux contraintes de plan pèsent. `ADR-0095` reporte le test `403` de chaque famille de routes à la tâche qui la crée : `AUTHENTIFIER-CLES` teste donc le middleware et les trois rôles sur un routeur de test. `ADR-0099` fixe que `/health` et `/ready` ne demandent aucune authentification.

## Décision

**Décision 1 — dépendances.** Trois crates s'ajoutent à `docs/07` :

| Rôle | Crate | Emploi |
|---|---|---|
| SHA-256 | `sha2` | Hachage du jeton reçu dans l'en-tête |
| Comparaison en temps constant | `subtle` | `ConstantTimeEq` sur deux tableaux de 32 octets |
| Aléa | `getrandom` | Génération de la clé de `crible key generate` |

Aucune crate d'encodage hexadécimal n'est ajoutée : l'écriture et la lecture des 64 caractères hexadécimaux se font à la main.

**Décision 2 — format de la clé et du haché.** Une clé générée est faite de **32 octets** tirés de `getrandom` (256 bits d'entropie), écrits en **64 caractères hexadécimaux minuscules**, sans préfixe. Le haché est le **SHA-256 des octets UTF-8 de la chaîne transmise dans l'en-tête**, c'est-à-dire les 64 caractères de la clé tels quels, écrit en 64 caractères hexadécimaux minuscules, sans préfixe.

**Décision 3 — `crible key generate`.** La commande n'a **aucune option**. Elle écrit sur `stdout` exactement deux lignes, dans cet ordre :

```
key: <clé>
sha256: <haché>
```

Elle ne lit ni n'écrit aucun fichier, et n'écrit rien d'autre sur `stdout`. La clé n'est jamais journalisée par la commande.

**Décision 4 — validation de `[[auth.keys]]` au démarrage.** Le serveur refuse de démarrer, avec un code de sortie non nul et un message qui nomme la clé fautive par son `name`, dans les cas suivants :

- `sha256` ne fait pas exactement 64 caractères de `[0-9a-f]` ;
- deux clés portent le même `name` ;
- deux clés portent le même `sha256` ;
- `role` n'est pas l'un de `read`, `write`, `admin`.

Le raisonnement est celui d'`ADR-0097` : une faute de configuration ne doit pas passer sans effet. Une **liste vide** est acceptée : le serveur démarre, journalise un avertissement, et toute route authentifiée répond `401`. C'est le fermé par défaut ; `/health` et `/ready` restent ouvertes (`ADR-0099`).

**Décision 5 — rôles.** `admin` contient `write`, qui contient `read` (`docs/05-api.md:8-12`). Chaque route déclare son rôle minimal, et le middleware compare ce minimum au rôle de la clé reconnue.

**Décision 6 — analyse de l'en-tête `Authorization`.** Le schéma `Bearer` est reconnu **sans tenir compte de la casse**, suivi d'**un seul espace**, puis de la clé, qui doit être **non vide et sans espace**. Un en-tête absent, mal formé, vide, ou présent **plusieurs fois** donne `401`.

**Décision 7 — comparaison.** Le haché du jeton reçu est comparé à **tous** les hachés de la configuration, **sans court-circuit**, avec `subtle` ; aucun cache n'est introduit. Le temps constant n'est pas mesuré en test : il est assuré par construction, et un test vérifie que **toutes** les clés de la configuration sont parcourues.

**Décision 8 — réponses.** Une clé absente, mal formée ou inconnue reçoit `401`, avec l'en-tête `WWW-Authenticate: Bearer` et **le même corps et le même message** dans les trois cas : aucun oracle ne distingue une clé inconnue d'une clé absente. Un rôle insuffisant reçoit `403`, **sans** `WWW-Authenticate`. Le corps suit le format d'erreur de `docs/05-api.md:26-30` (`error.code`, `error.message`, `error.details`, `error.request_id`) dès cette tâche ; `FORMAT-ERREURS-ET-LIMITES` le reprendra pour la conversion générale de `CribleError`, sans changer la forme fixée ici.

**Décision 9 — contexte de requête et logs.** Le nom et le rôle de la clé sont attachés à la requête, en extension, pour alimenter la dimension `client` des analytics (`docs/06-observabilite.md:17`, `:68`). Le nom de la clé peut apparaître dans les logs, dans le champ `client`. Ni l'en-tête `Authorization`, ni la clé, ni son haché n'apparaissent dans les logs, à aucun niveau et quel que soit `RUST_LOG` (`INV-13`, `ADR-0096`).

Ce qui n'est pas décidé ici : le nombre de clés d'une instance ; la rotation ou l'expiration d'une clé ; la limitation de débit par clé ; l'accès par en-tête autre que `Bearer` ; le libellé exact des messages d'erreur, qui reste celui de `docs/05` ; la façon dont les routes déclarent leur rôle minimal dans le code.

## Conséquences

- **Positives** :
  - SHA-256, temps constant et aléa viennent de crates spécialisées, épinglées et sans code cryptographique maison ;
  - le haché transmis dans `crible.toml` ne permet pas de retrouver la clé en clair, et la clé ne quitte jamais `stdout` de `crible key generate` ;
  - une faute de configuration échoue au démarrage, avec un message qui nomme la clé : elle ne produit pas une instance qui refuse silencieusement toutes les requêtes ;
  - la réponse `401` est identique dans les trois cas d'échec, donc elle ne sert pas d'oracle pour deviner une clé ;
  - la comparaison ne s'arrête pas à la première correspondance, ce qui évite de révéler la position de la clé reconnue ;
  - le fermé par défaut (`[[auth.keys]]` vide) est explicite, et les sondes restent joignables.
- **Négatives / coûts** :
  - trois crates de plus dans l'arbre de dépendances, à compiler et à auditer ;
  - chaque requête authentifiée paie un SHA-256 et une comparaison sur toutes les clés ; le coût est négligeable pour une poignée de clés, mais il croît linéairement avec leur nombre ;
  - l'absence de cache rend le coût prévisible et la comparaison uniforme, au prix de ce parcours complet ;
  - `docs/05` gagne une convention d'authentification plus détaillée, et sa ligne `401` doit couvrir le cas « mal formée » ;
  - une clé ne peut pas être ajoutée par l'environnement (`ADR-0097`) : il faut éditer le fichier monté en lecture seule.
- **À surveiller** :
  - un test qui cherche la clé ou son haché dans les logs produits avec `RUST_LOG=trace` doit rester vert ;
  - le nombre de clés d'une instance : s'il devient grand, la comparaison sur toutes les clés deviendra un coût mesurable, et un ADR devra rouvrir la question du cache ;
  - la fidélité de `FORMAT-ERREURS-ET-LIMITES` au corps d'erreur fixé ici : une divergence entre les deux tâches casserait la promesse d'un format unique ;
  - l'usage de `sha2` hors de l'authentification : le tableau des dépendances doit rester lisible, et `blake3` reste la crate des empreintes et des checksums.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| `argon2` ou `bcrypt` pour hacher les clés | Ces fonctions sont conçues pour des mots de passe à faible entropie et coûtent cher par vérification ; une clé de 256 bits n'en a pas besoin, et `docs/05` impose SHA-256. |
| Stocker les clés en clair dans `crible.toml` | Un fichier monté, lu par des outils et copié dans une sauvegarde, laisserait la clé lisible ; `docs/05` impose le haché. |
| Comparer avec `==` sur les hachés | La comparaison s'arrête au premier octet différent et fuit l'avancement de la correspondance ; `subtle` existe pour cela. |
| Un `HashMap<haché, (nom, rôle)>` comme cache ou comme index | Le parcours devient dépendant de la clé reçue et de la table, et la décision de temps constant perd sa preuve par construction ; le gain est nul pour quelques clés. |
| Ajouter une crate d'encodage hexadécimal (`hex`, `base16`) | 64 caractères se lisent et s'écrivent en quelques lignes ; une dépendance de plus pour un format trivial contredit la règle 12. |
| Tirer l'aléa avec `rand` ou `uuid` | `rand` n'est pas dans `docs/07` ; `uuid` sert à `x-request-id` (`ADR-0096`) et ne donne pas 32 octets bruts ; `getrandom` est la primitive système, sans état. |
| Un préfixe de clé (`crible_…`) ou une somme de contrôle dans la clé | Le format devient plus long, doit être validé, et n'ajoute rien : la reconnaissance se fait par le haché, pas par la forme. |
| Reconnaître le schéma `Bearer` en respectant la casse | La RFC 7235 déclare les schémas insensibles à la casse ; un client qui écrit `bearer` serait refusé sans raison. |
| Accepter plusieurs espaces après `Bearer` | Deux analyses concurrentes de la même chaîne ouvrent la porte à des divergences entre client et serveur ; un seul espace est la forme canonique. |
| `401` avec un message différent selon la cause | Distinguer « clé absente » de « clé inconnue » donne un oracle à un attaquant qui devine. |
| `WWW-Authenticate` sur `403` | Le client est authentifié, mais son rôle ne suffit pas : redemander des identifiants ne changerait rien. |
| Refuser le démarrage quand `[[auth.keys]]` est vide | Empêche de lancer une instance sans index ouverte pour diagnostic ; le fermé par défaut avec un avertissement rend le même service sans bloquer l'exploitation. |
| Mettre en cache le résultat d'authentification d'une clé | Le cache est une seconde source de vérité pour une valeur secrète, et la décision de temps constant ne s'y applique plus. |

## Références

`docs/05-api.md` (`:6` convention d'authentification, `:8-12` rôles, `:24-48` codes d'erreur, `:52-74` tableau des routes) ; `docs/06-observabilite.md` (`:17` dimension `client` des analytics, `:68` client du journal de debug, `:111-116` santé et logs, `INV-13`) ; `docs/07-stack-et-deploiement.md` (`:29-50` dépendances Rust, `:63-108` configuration de l'instance et `[[auth.keys]]`) ; `docs/08-invariants-et-tests.md` (`INV-01`, `INV-13`) ; fiche `AUTHENTIFIER-CLES` (`docs/phases/P6-serveur-et-api.md:29-41`) ; `ADR-0095` (report des critères, test sur routeur de test) ; `ADR-0096` (dépendances et features du serveur, logs sans contenu) ; `ADR-0097` (configuration, refus au démarrage, `[[auth.keys]]` non surchargeable) ; `ADR-0098` (`x-request-id`) ; `ADR-0099` (`/health` et `/ready` sans authentification) ; `ADR-0031` (clé inconnue refusée, précédent) ; `ADR-0023` (orchestration).
