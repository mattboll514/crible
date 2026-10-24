# ADR-0054 — Chargement différé du modèle à l'attache et bornes de la recette

- **Statut** : Accepté (décision du propriétaire le 2026-10-24)
- **Date** : 2026-10-24 (proposition et acceptation)
- **Décideurs** : propriétaire
- **Décision** : accepté le 2026-10-24 par le propriétaire, tel que proposé : option (a) sur chacun des deux points, donc chargeur de modèle différé dont l'attache choisit le moment d'appel, et bornes `acceptance_sample ≥ 1` et `acceptance_min_cosine ∈ ]0, 1]` refusées en `invalid_request`.
- **Proposé par** : agent greffier, sur demande du propriétaire. Origine : relecture du tour 1 de la tâche `RECETTE-IMPORT` (phase P2, fiche `docs/phases/P2-encodeur-et-recette.md`), puis verdicts de `@architecte` du 2026-10-24 : `ECART` sur la forme de l'API (fourniture du modèle à `attach`), `ECART` sur `acceptance_sample = 0`, `NON-COUVERT` sur le domaine d'`acceptance_min_cosine`.
- **Remplace** : —

## Contexte

### Pourquoi cet ADR existe

La tâche `RECETTE-IMPORT` implémente le contrôle 9 de `docs/04-donnees-et-ingestion.md` (recette fp32, **INV-12**) dans `crible-store::attach`. La relecture du tour 1 et les verdicts de `@architecte` du 2026-10-24 relèvent deux points que la conception ne tranche pas, et un troisième incomplètement couvert. Aucun des trois ne peut être clos par le code seul : ils fixent une forme d'API et un domaine de configuration, donc ils demandent une décision écrite.

### Point 1 — forme de la fourniture du modèle à `attach`

ADR-0053, « Décision », précision 2, dit : « Le modèle est chargé et fourni par l'appelant (CLI, serveur, tests) ; l'attache ne charge aucun modèle et ne décide pas de sa configuration. `crible-store` dépend des **types** de `crible-encode` (compteur de tokens, encodeur), pas de la politique de chargement. »

L'implémentation relue ne fournit pas un modèle déjà chargé : elle reçoit `Option<&dyn Fn() -> Result<Model, CribleError>>`, c'est-à-dire un **chargeur** différé. L'appelant garde le dossier du modèle, la précision fp32 et la vérification des hash (`crates/crible/src/commands.rs`) ; `attach` ne choisit que le **moment** de l'appel.

Ce moment est significatif : `attach` n'invoque le chargeur qu'après le contrôle 3 (`incompatible_model`) et le contrôle 8 (longueurs des passages, INV-11). Un lot refusé plus tôt est donc rejeté sans payer ONNX Runtime ni le graphe fp32. ADR-0052 chiffre ce coût : plus d'1 Gio de mémoire et environ 1,8 s au premier appel. Le chargement différé garde aussi les tests de refus (contrôles 3 et 8) exécutables en CI sans ONNX Runtime.

Le conflit est donc étroit. La précision 2 dit « fourni par l'appelant », ce qui reste vrai : l'appelant fournit bien le chargeur, et la politique (dossier, précision, hash) reste chez lui. Mais ADR-0053 a écarté en (c) une **fonction de comptage** fournie par l'appelant, au motif explicite qu'elle serait « une abstraction pour une seule implémentation, ce que la règle 12 d'`AGENTS.md` proscrit ». Le chargeur différé est une abstraction du même genre. La différence est qu'il sert ici à **retarder** un travail coûteux, et non à remplacer un type. Cette différence doit être écrite, sinon le prochain lecteur appliquera la règle 12 et supprimera le paramètre.

### Point 2 — domaine des réglages de recette

Deux valeurs de `ingest.*` commandent la recette :

- `acceptance_sample` : nombre de passages ré-encodés en ONNX fp32 et comparés aux vecteurs du lot ;
- `acceptance_min_cosine` : cosinus minimal accepté, valeur par défaut 0,999 (ADR-0052).

Faits :

- `ingest.*` est réglable par `PATCH` (`docs/04-donnees-et-ingestion.md`, « Ce qui est figé, ce qui est réglable »).
- `docs/05-api.md` définit `invalid_request` comme « paramètre hors bornes ».
- `crates/crible-core/src/config.rs` analyse les deux valeurs sans contrôle de domaine : `acceptance_sample = 0` et une valeur hors de `]0, 1]` passent l'analyse.
- ADR-0052 fixe la **valeur** 0,999, pas un **domaine**.
- `docs/04` est en tension avec elle-même : `ingest.*` est déclaré réglable, mais la même page note « 0,999 : valeur fixée par ADR-0052, elle n'est plus provisoire ».

Le cas `acceptance_sample = 0` est un écart, pas seulement un trou : il désactive silencieusement le contrôle 9. Le rapport d'attache porte alors `acceptance: null`, valeur **indistinguable** de celle d'un modèle `external`, alors que `docs/04` et la fiche n'exemptent que `external` et qu'INV-12 vaut pour tout modèle `onnx`. Une configuration valide produirait donc un rapport qui laisse croire qu'aucune recette n'était requise.

Le domaine d'`acceptance_min_cosine` est non couvert : rien n'interdit aujourd'hui une valeur nulle, négative ou supérieure à 1, qui rendrait la recette soit toujours satisfaite, soit impossible à satisfaire, sans que la configuration soit refusée.

Un cas voisin ne doit pas être confondu avec le précédent : un lot qui ne contient **aucun passage** (seulement des `deletions`) n'a rien à tirer au sort. Il garde `acceptance: null`, et ce comportement est légitime. Il doit être écrit pour qu'on ne le prenne pas pour le symptôme du réglage à zéro.

### Ce que les documents ne tranchent pas

- La précision 2 d'ADR-0053 ne dit pas si « fourni par l'appelant » autorise un chargeur différé, ni à quel moment du protocole le modèle peut être chargé.
- `docs/04` déclare `ingest.*` réglable sans donner de bornes.
- Aucun document ne décrit le rapport de recette d'un lot sans passage.

## Décision

**Proposition soumise au propriétaire : accepter l'option (a) sur chacun des deux points.**

### Point 1 — garder le chargeur différé et l'inscrire (option a)

L'API d'attache reçoit un **chargeur de modèle** fourni par l'appelant, et `attach` décide seulement du **moment** de l'invoquer : après le contrôle 3 (`incompatible_model`) et le contrôle 8 (longueurs), avant le contrôle 9 (recette). La configuration du modèle (dossier, précision fp32, vérification des hash) reste entièrement chez l'appelant.

Précisions :

1. Cet ADR **complète** la précision 2 d'ADR-0053 : « fourni par l'appelant » se lit désormais « fourni par l'appelant, éventuellement sous forme d'un chargeur différé dont l'attache choisit le moment d'appel ».
2. L'abstraction est **assumée et tracée** : c'est une abstraction pour une seule implémentation, au sens de la règle 12 d'`AGENTS.md`, et elle est acceptée pour deux raisons écrites ici : éviter le coût d'ONNX Runtime et du graphe fp32 (plus d'1 Gio, environ 1,8 s au premier appel, ADR-0052) quand le lot est refusé plus tôt, et garder les tests de refus exécutables en CI sans ONNX Runtime.
3. Le moment d'appel est **contractuel**, pas un détail d'implémentation : un futur remaniement ne peut pas charger le modèle avant le contrôle 3 sans rouvrir cet ADR.
4. L'appelant reste seul à décider de la précision de chargement : `attach` ne reçoit ni chemin, ni nom de fichier, ni précision.

### Point 2 — borner les deux réglages de recette (option a)

- `acceptance_sample ≥ 1` ;
- `acceptance_min_cosine ∈ ]0, 1]` ;
- les deux valeurs sont **refusées** en `invalid_request` à la validation de la configuration, à la création comme au `PATCH` ;
- `ingest.*` reste réglable par `PATCH` (aucune clé n'est retirée) ;
- la valeur par défaut `0,999` d'ADR-0052 ne change pas ;
- un lot **sans aucun passage** (seulement des `deletions`) garde `acceptance: null` : c'est un fait du lot, pas un réglage, et ce comportement est écrit.

Précisions :

1. Le contrôle de domaine vit dans `crible-core`, au même endroit que l'analyse de la configuration, et non dans `crible-store` : la validation d'une valeur de configuration ne dépend pas du stockage.
2. Le refus est un `invalid_request` à la validation, pas une erreur d'attache : la configuration est invalide en elle-même, quelle que soit la tâche qui la lit.
3. Le présent ADR ne fixe **pas** de nouvelle valeur par défaut : il ne fixe qu'un domaine.

### Ce que cet ADR ne décide pas

- Il ne rouvre pas ADR-0052 : la valeur `0,999` reste la valeur par défaut.
- Il ne tranche pas le cas de `compaction.dead_ratio`, qui présente la même absence de bornes mais reste **hors périmètre** de la tâche `RECETTE-IMPORT`. Ce point est signalé, pas décidé.
- Il ne change ni le format de lot, ni le protocole d'attache, ni le contenu du rapport d'attache.

## Conséquences

- **Positives** :
  - un lot refusé par les contrôles 3 ou 8 n'entraîne aucun chargement de modèle : ni mémoire du graphe fp32, ni latence du premier appel ;
  - les tests de refus restent exécutables en CI sans ONNX Runtime, donc sans dépendance de plateforme pour ces cas ;
  - la configuration ne peut plus désactiver la recette par une valeur valide en apparence : `acceptance_sample = 0` devient un `invalid_request` explicite ;
  - `acceptance: null` n'a plus qu'**une** signification dans le rapport, celle du lot sans passage, et elle sera écrite ;
  - la tension de `docs/04` sur `ingest.*` est levée par l'ajout d'une ligne de bornes, sans retirer le réglage.
- **Négatives / coûts** :
  - la règle 12 d'`AGENTS.md` reçoit une exception de plus, locale et motivée ; chaque exception de ce genre affaiblit la règle si elle n'est pas relue ;
  - l'API d'attache transporte une closure : signature plus lourde à lire pour un appelant qui a déjà le modèle en main ;
  - l'ordre des contrôles devient un contrat public de fait : le déplacer demande un ADR ;
  - le contrôle de domaine ajouté à `crible-core` peut refuser des configurations existantes, si l'une d'elles portait déjà `0` ou une valeur hors bornes. Aucune n'est connue à ce jour.
- **État des tâches** :
  - `RECETTE-IMPORT` reste **`BLOQUE`** jusqu'à la décision du propriétaire ;
  - le code de la tâche implémente **déjà** l'option (a) des deux points ; le contrôle de domaine est ajouté à `crible-core` dans la même tâche, en attente de la décision. L'ADR n'attend donc aucune modification de code supplémentaire ;
  - `docs/suivi.md`, section « Écarts à la conception », doit citer ADR-0054 ; cette ligne n'est pas ajoutée par cet ADR et reste à la charge de l'orchestrateur.
- **Documents à modifier à l'acceptation** :
  - ADR-0053 : mention sur la précision 2, complétée par ADR-0054. Le corps d'ADR-0053 n'est **pas** réécrit ;
  - `docs/04-donnees-et-ingestion.md` : domaine des deux réglages (`acceptance_sample ≥ 1`, `acceptance_min_cosine ∈ ]0, 1]`, refus en `invalid_request`), et comportement du lot sans passage (`acceptance: null`).
- **À surveiller** :
  - l'apparition d'autres valeurs de configuration sans contrôle de domaine (`compaction.dead_ratio` est la première) : si elles se multiplient, un ADR général sur les bornes de configuration sera préférable à une décision par valeur ;
  - un lot refusé qui chargerait malgré tout le modèle : signal que l'ordre des contrôles n'est plus tenu ;
  - un rapport d'attache qui porterait `acceptance: null` pour un lot qui contient des passages : signal que le contrôle 9 a été contourné, et que la décision doit être revue ;
  - l'usage du chargeur par un appelant qui aurait besoin du modèle **avant** l'appel (par exemple pour valider le lot lui-même) : ce besoin relèverait de l'option (d), déjà écartée.

## Alternatives écartées

### Point 1 — forme de la fourniture du modèle

| Alternative | Pourquoi écartée |
|---|---|
| **(b)** `Option<&Model>` déjà chargé, littéral à la précision 2 | Chaque attache charge le graphe fp32 même pour un lot refusé par les contrôles 3 ou 8. Les tests de refus exigeraient alors ONNX Runtime en CI, donc une plateforme et des fichiers de modèle pour vérifier un refus qui ne dépend pas du modèle. |
| **(c)** `attach` reçoit le dossier du modèle et le charge lui-même | Contredit la précision 2 d'ADR-0053 et déplace la politique de chargement (précision, hash, dossier) dans `crible-store`, qui n'a pas à la connaître. |
| **(d)** Passe de validation séparée chez l'appelant, puis attache | Le lot serait lu deux fois, ce qui contredit ADR-0039 (lecture en flux, une seule passe). La séquence validation → refus → écriture serait scindée entre deux modules, et ADR-0053 a déjà écarté cette forme en (b) pour le contrôle 8. |

### Point 2 — domaine des réglages de recette

| Alternative | Pourquoi écartée |
|---|---|
| **(b)** Figer `acceptance_min_cosine` à 0,999 et le retirer de `PATCH` | Ferme le trou du domaine, mais retire un réglage que `docs/04` déclare réglable. Un producteur qui change de précision ou de plateforme n'aurait plus de marge, alors qu'ADR-0052 a justement tranché la question du seuil et non celle de l'usage. Retirer un réglage est plus lourd à revenir qu'ajouter une borne. |
| **(c)** Statuer que `acceptance_sample = 0` désactive la recette | Viole INV-12 pour tout modèle `onnx`. Le rapport produit (`acceptance: null`) serait indistinguable d'un modèle `external`, donc l'extinction serait invisible dans les données. Une configuration valide ne peut pas désactiver un invariant. |

### Ce que les options écartées ont en commun

Les deux points partagent la même préférence : garder la souplesse de configuration et d'API, et encadrer par une **borne** ou un **moment** écrit plutôt que par un retrait. C'est ce qui rend les deux décisions cohérentes entre elles, et c'est aussi pourquoi elles sont proposées dans un seul ADR : le code de `RECETTE-IMPORT` les porte ensemble.

## Références

- Fiche : `docs/phases/P2-encodeur-et-recette.md`, `### RECETTE-IMPORT`.
- Conception : `docs/04-donnees-et-ingestion.md` (contrôles 3, 8 et 9 ; « Ce qui est figé, ce qui est réglable »), `docs/05-api.md` (`invalid_request`), `docs/08-invariants-et-tests.md` (INV-11, INV-12).
- Code relu : `crates/crible/src/commands.rs` (chargeur fourni par l'appelant), `crates/crible-core/src/config.rs` (analyse des deux valeurs).
- ADR liés : ADR-0052 (requêtes fp32 et seuil 0,999 ; coût du premier chargement), ADR-0053 (encodeur dans le chemin d'attache, précision 2 complétée ici), ADR-0039 (ordre des lignes, lecture en flux), ADR-0041 (sémantique d'attache et rapport), ADR-0028 (empreinte du modèle).
- Processus : `docs/adr/README.md` (« Quand écrire un ADR », cycle de vie, section « Écarts ») ; `AGENTS.md` (règle 12, simplicité).
