# ADR-0071 — Seuil entre parcours dense et parcours clairsemé du scan binaire

- **Statut** : Accepté (accepté par anticipation le 2026-10-26, confirmé par le propriétaire le 2026-10-26)
- **Date** : 2026-10-26 (proposition et acceptation anticipée)
- **Décideurs** : propriétaire
- **Décision** : accepté par anticipation le 2026-10-26, par délégation du propriétaire pour les trois premières tâches de P4, confirmé par le propriétaire le 2026-10-26. La décision est retenue telle que proposée : parcours dense quand le pool couvre au moins la moitié de la plage d'iid qu'il occupe, parcours clairsemé sinon ; règle fixe, sans réglage ni configuration.
- **Proposé par** : agent greffier, sur demande de l'orchestrateur, pour la tâche `SCANNER-POOL-BINAIRE` (phase P4), à partir du choix retenu par `@dev`
- **Remplace** : —

## Erratum du 2026-10-26

Les numéros de ligne de `crates/crible-search/src/dense/binary.rs` cités dans la décision et dans les références ont dérivé depuis la proposition de cet ADR. Ils sont remplacés par des noms stables, vérifiés dans le fichier le 2026-10-26 : la fonction `traversal` porte le seuil, `top_k` choisit le parcours une fois par scan, et le test `dense_and_sparse_traversals_agree_with_a_full_sort` compare les deux parcours à un tri complet. Aucune décision ne change ; la correction est autorisée par le propriétaire le 2026-10-26.

- **Avant** : `binary.rs:261-271` (le seuil) ; `binary.rs:126` (choix une fois par scan) ; `binary.rs:552` (test de comparaison au tri complet).
- **Après** : fonction `traversal` (le seuil) ; dans `top_k`, l'endroit qui choisit le parcours une fois par scan ; test `dense_and_sparse_traversals_agree_with_a_full_sort` (comparaison au tri complet).

Motif : ces renvois doivent rester valables quand le fichier évolue. Les numéros observés après dérive étaient 130, 266-274 et 576, ce qui montre que la citation par ligne ne tient pas d'une tâche à l'autre.

## Contexte

La fiche `SCANNER-POOL-BINAIRE` (`docs/phases/P4-recherche-rapide.md:44-51`) demande un parcours du pool par tranches d'iid, et précise : « Si le pool est dense, on itère sur les codes contigus en testant la bitmap. S'il est clairsemé, on itère sur la bitmap. » Elle ne fixe aucun seuil entre les deux parcours.

`docs/03-recherche.md:45-47` et `:51` ne le fixent pas non plus : la table des stratégies choisit entre EXACT et BINARY, et le paragraphe de parallélisme ne parle que du découpage en tranches d'iid. Aucun ADR ne tranche le seuil. L'`exact_threshold` ne peut pas servir : il sépare EXACT de BINARY (`docs/03:45-46`, `:62`), pas deux parcours du même scan binaire.

`AGENTS.md` (règle 4) impose un ADR pour tout seuil : c'est l'objet de celui-ci.

Les deux parcours diffèrent par leur coût, pas par leur résultat :

- le **dense** paie un test de bitmap par iid de la plage, vivant ou non, et lit les codes dans l'ordre de la mémoire ;
- le **clairsemé** paie un pas de bitmap, une recherche de segment et une lecture aléatoire de code par passage du pool.

À moins de la moitié, le nombre d'iid morts testés par le dense dépasserait celui des codes lus par le clairsemé.

## Décision

**Le parcours est dense quand le pool couvre au moins la moitié de la plage d'iid qu'il occupe, c'est-à-dire de son plus petit à son plus grand iid ; il est clairsemé sinon.** La règle est fixe : aucun réglage, aucune configuration, aucun paramètre de requête.

Précisions :

- la comparaison est `2 × taille du pool ≥ dernier iid − premier iid + 1` (`crates/crible-search/src/dense/binary.rs`, fonction `traversal`) ;
- la décision est prise **une fois par scan, sur le pool entier** (`crates/crible-search/src/dense/binary.rs`, dans `top_k`), puis appliquée à toutes les tranches d'iid ; elle n'est pas recalculée par tranche ;
- un pool vide ne choisit pas de parcours : le scan rend une liste vide avant la décision ;
- le seuil ne change que le coût, jamais le résultat. Les deux parcours sont comparés à un tri complet par Hamming, sur des pools de toutes densités, par le test `dense_and_sparse_traversals_agree_with_a_full_sort`.

Cet ADR ne fixe ni N, ni `exact_threshold`, ni le parallélisme du scan : ces choix relèvent de MES-01, Q-01 et de la fiche elle-même.

## Conséquences

- **Positives** :
  - le comportement est déterministe et ne dépend d'aucune configuration : même requête, même parcours ;
  - chaque pool prend le parcours qui paie le moins de lectures, sur le critère qui les distingue (iid morts testés contre lectures aléatoires) ;
  - une seule comparaison entière par scan : le choix ne coûte rien de mesurable.
- **Négatives / coûts** :
  - c'est une heuristique de coût, **pas un optimum mesuré** : la moitié est le seuil le plus simple qui s'énonce sans réglage ;
  - le parcours dense paie un test par iid mort : un pool juste au-dessus de la moitié avec un grand trou central reste sur le parcours le plus coûteux ;
  - la décision est globale au pool : une tranche très clairsemée suit le parcours dense choisi pour l'ensemble.
- **À surveiller** :
  - **MES-02** (latence du scan binaire selon la taille du pool, `docs/08-invariants-et-tests.md:67`, `docs/03:138`) : c'est la mesure qui pourra remplacer ce seuil, par un nouvel ADR, avec des chiffres par densité de pool ;
  - un écart de latence entre les deux parcours sur un même pool, relevé par MES-02, dirait que la moitié n'est pas le bon point de bascule.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Un seuil configurable | Configuration pour une valeur fixe, que `AGENTS.md` (règle 12) écarte ; aucun besoin exprimé ne demande de régler ce ratio |
| Un seul parcours, dense partout | Le dense paie un test de bitmap par iid mort : sur un pool clairsemé, il lit bien plus que les passages vivants |
| Un seul parcours, clairsemé partout | Le clairsemé paie une lecture aléatoire de code par passage : sur un grand pool dense, il perd l'ordre mémoire du dense |
| Réutiliser `exact_threshold` comme seuil | Il sépare EXACT de BINARY (`docs/03:45-46`) ; il ne dit rien de la densité du pool dans sa plage d'iid |

## Documents modifiés après acceptation

Appliqué dans le même travail que cet ADR, sur la branche de la tâche `SCANNER-POOL-BINAIRE` :

| Document | Modification |
|---|---|
| `docs/adr/README.md` | ajouter la ligne d'index de cet ADR |

Aucun document de conception n'est modifié : `docs/03-recherche.md:51` traite du parallélisme (découpage en tranches d'iid, tas par thread, fusion) et n'appelle pas de renvoi vers cet ADR, puisque la décision est prise une fois sur le pool entier. `docs/suivi.md`, le code et les tests ne sont pas touchés par cet ADR.

## Références

- Fiche `SCANNER-POOL-BINAIRE` : `docs/phases/P4-recherche-rapide.md:44-51`
- `docs/03-recherche.md:45-47` : table des stratégies EXACT / BINARY ; `:51` : parallélisme ; `:138` : budget du scan binaire, renvoyant à MES-02
- `docs/08-invariants-et-tests.md:67` : MES-02, latence du scan binaire selon la taille du pool
- `crates/crible-search/src/dense/binary.rs` : fonction `traversal` (le seuil) ; choix du parcours une fois par scan dans `top_k` ; test `dense_and_sparse_traversals_agree_with_a_full_sort` (comparaison au tri complet)
- `AGENTS.md`, règles 4 (ADR pour tout seuil) et 12 (pas de configuration pour une valeur fixe)
- ADR-0004 (codes binaires, scan du pool et rescoring), ADR-0069 (format de `codes.bin`), ADR-0070 (noyaux SIMD)
