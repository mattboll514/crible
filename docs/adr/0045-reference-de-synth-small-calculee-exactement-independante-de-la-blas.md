# ADR-0045 — Référence de `synth-small` calculée exactement, indépendante de la bibliothèque BLAS

- **Statut** : Accepté
- **Date** : 2026-10-22 (proposition et acceptation)
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, sur demande de l'orchestrateur, d'après le diagnostic de l'agent dev et le verdict de l'agent architecte ; tâche d'origine : RECHERCHER-DENSE-EXACT (phase P1), après l'échec de la CI du run `39794175404`
- **Remplace** : —

## Contexte

### Ce que montre la CI

Le premier passage de la CI sur le code de la phase P1 échoue, alors que les mêmes tests passent sur un poste macOS arm64. L'échec vient du run GitHub `39794175404`, sur Linux x86_64, où numpy est lié à OpenBLAS ; le poste de développement utilise numpy lié à Accelerate. Les versions de numpy (2.5.4), de pyarrow (26.0.0) et de blake3 sont égales de part et d'autre.

Le générateur n'est pas en cause. `tools/gen_synth.py` produit des octets identiques sur les deux plateformes : c'est vérifié aussi avec CPython 3.12 et 3.13. La marge minimale à une frontière d'arrondi fp16 y vaut 2,9e-11, l'aléa est un PCG64 à graine fixe, et les métadonnées sont figées depuis ADR-0036.

Ce qui varie est `expected.jsonl`, écrit par `tools/reference_topk.py`. À la ligne 429, le calcul est `scores = matrix @ vector`, soit un sgemv confié à BLAS en f32. Selon la bibliothèque, l'ordre de sommation change, et 1 512 des 2 000 scores enregistrés changent avec lui, pour un écart maximal de 3,0e-7.

### Les deux conséquences

Première conséquence : la somme versionnée de `synth-small`, consignée dans `tools/README.md` (`blake3:2d3d8265a8df350d816bccb5732fe338921d275cae5d091a96d0497646e58805`), couvre `expected.jsonl`, car `SMALL_FILES` de `gen_synth.py` (ligne 423) le liste. Cette somme dépend donc de la bibliothèque BLAS de la machine. C'est exactement ce qu'ADR-0036 voulait éviter : il promettait que « CI et postes de développement calculent la même ».

Seconde conséquence : le test `expected_jsonl_holds_the_quasi_equalities_the_tolerance_allows` (`crates/crible-search/tests/inv_06_dense_exact_synth.rs`, vers la ligne 512) exige que 5 requêtes aient une paire de scores adjacents à moins de 1e-6. Avec Accelerate, le décompte vaut 5 ; avec tout autre ordre de sommation, il vaut 4. Le décompte à moins de 1e-5, lui, reste stable à 55.

### Ce que la conception exige

La fiche `RECHERCHER-DENSE-EXACT` (phase P1, ligne 159) décrit aujourd'hui l'écart comme voulu : « la référence somme en f32 via BLAS, le moteur dans un autre ordre ». La fiche `GENERER-JEU-SYNTHETIQUE` exige de son côté que deux générations avec la même graine donnent des fichiers identiques au bit près, et `docs/08-invariants-et-tests.md` (ligne 45) précise que la somme de contrôle de `synth-small` est versionnée. Une somme qui dépend d'une bibliothèque ne tient aucune des deux promesses.

INV-06 demande que, sous le seuil exact, le résultat soit identique au calcul exact. C'est la référence qui doit être exacte ; le moteur, lui, somme en f32 dans l'ordre des composantes (`crates/crible-search/src/dense/exact.rs`, fonction `dot`), ce qui est reproductible au bit près et ne dépend pas d'une largeur SIMD.

## Décision

La référence de `synth-small` est calculée **exactement**, et devient donc indépendante de la bibliothèque BLAS : les produits de composantes f32 sont faits en f64 (ils y sont exacts), chaque ligne est sommée avec `math.fsum`, et le résultat reçoit un seul arrondi final en f32.

Ce que compare INV-06 ne change pas : même expression mathématique (le produit scalaire de la requête normalisée et du passage), même comparateur à tolérance (`1e-5` sur les scores, départage stable par clé), même jeu de requêtes. Seul l'ordre de sommation cesse de dépendre de la machine, puisque la somme est arrondie correctement au lieu d'être accumulée dans un ordre qui varie.

Les valeurs attendues après application sont les suivantes :

- les décomptes de quasi-égalités deviennent **4** requêtes sous 1e-6 et **55** sous 1e-5, et le test les garde **exacts**, sans les transformer en propriété ;
- la somme consignée dans `tools/README.md` change une seconde fois. La valeur prédite par l'agent dev est `blake3:415b4c18e58cfec4a8ae4f4f658dd905ad48ba8e1b3868eedb5f39e3c702d232` ; elle est à confirmer à l'application, et c'est la valeur recalculée qui fait foi dans le document.

Le coût est mesuré : 14,2 s pour les 200 requêtes, soit environ 14 s de plus en CI. C'est le prix d'une somme exacte en Python pur.

Ce qui n'est **pas** décidé ici :

- la tolérance du test et la règle de départage ne changent pas ;
- le calcul du moteur ne change pas : il reste en f32, dans l'ordre des composantes ;
- ADR-0036 reste valide et n'est pas remplacé ;
- la valeur de la somme elle-même n'est pas une décision : elle est constatée à l'application.

## Conséquences

- **Positives** :
  - la somme de `synth-small` ne dépend plus de la plateforme : elle est identique par construction, sur toute machine et avec toute bibliothèque ;
  - la CI retrouve un contrôle réel de divergence : un écart de somme signale désormais un vrai changement du jeu, non un changement de BLAS ;
  - INV-06 garde sa force : la référence est la valeur correctement arrondie, donc la comparaison porte sur la seule différence qui reste, celle du moteur ;
  - le test des quasi-égalités reste un test de valeur exacte, et non une propriété plus faible.
- **Négatives / coûts** :
  - environ 14 s de plus en CI ;
  - la somme consignée change une seconde fois. ADR-0036 annonçait qu'elle changerait une fois puis deviendrait définitive ; cette seconde cause, inconnue alors, oblige à le redire ;
  - deux fichiers datés citent l'ancienne somme et restent tels quels : `docs/mesures/MES-14-cout-filtres-intervalles.md` et `docs/jalons/J1-rapport.md`. Leur somme est antérieure et l'ADR le dit, mais un lecteur peut s'y tromper ;
  - la valeur prédite de la nouvelle somme doit être confirmée ; si elle diffère, la décision tient, mais le document doit porter la valeur mesurée.
- **À surveiller** :
  - l'égalité de la somme entre au moins deux plateformes, Linux x86_64 avec OpenBLAS et macOS arm64 avec Accelerate, contrôlée par la CI ;
  - l'égalité des décomptes 4 et 55 sur ces deux plateformes ;
  - la durée du calcul de référence en CI, qui doit rester acceptable ;
  - le double arrondi f64 puis f32 reste théoriquement possible avec `math.fsum` ; il est déterministe, donc il ne compromet pas la reproductibilité, qui est l'objet de cet ADR ;
  - toute nouvelle différence de somme entre deux machines après application, qui signalerait une autre source de non-déterminisme.

En cas d'acceptation, les documents sont mis à jour dans cet ordre, et chaque commit cite **ADR-0045** :

1. `tools/reference_topk.py` : le calcul et sa docstring ;
2. `tools/README.md` : la nouvelle somme, la formulation « en f32 comme le moteur », la table des quasi-égalités (4 et 55) et le paragraphe sur BLAS ;
3. `crates/crible-search/tests/inv_06_dense_exact_synth.rs` : le décompte 5 devient 4, avec le commentaire correspondant ;
4. la fiche `RECHERCHER-DENSE-EXACT` (`docs/phases/P1-stockage-et-recherche-exacte.md`, vers la ligne 159) : « la référence somme en f32 via BLAS » devient « la référence est arrondie une fois en f32, le moteur somme en f32 scalaire », et 5 devient 4 ;
5. `docs/jalons/J1-demo.md`, si la somme y figure ;
6. `docs/suivi.md`, tenu par l'orchestrateur.

`docs/mesures/MES-14-cout-filtres-intervalles.md` et `docs/jalons/J1-rapport.md` restent inchangés, car datés. La mise en œuvre est une tâche dédiée hors plan, `REFERENCE-EXACTE-SANS-BLAS`, qui dépend de cet ADR.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| **A'** — l'option A, mais en remplaçant le décompte exact par une propriété (« au moins une paire à moins de 1e-6 ») | Sous A, 4 est une valeur du jeu de données, non une valeur de la plateforme : il n'y a plus de raison de relâcher l'assertion. Une propriété ne détecterait plus une dérive de 4 vers 1, ce qui affaiblit le test, contrairement à la règle 5 d'`AGENTS.md` (chaque critère a un test qui échoue quand il n'est plus tenu). |
| **B** — sortir `expected.jsonl` de la somme versionnée | C'est l'alternative B déjà rejetée par ADR-0036 : la CI perdrait le contrôle de divergence, et la somme ne couvrirait plus un fichier que J1 et INV-06 utilisent. De plus, `SMALL_FILES` sert aussi de liste de nettoyage à `--overwrite` : l'en retirer laisserait un `expected.jsonl` périmé survivre d'une génération à l'autre. |
| **C** — arrondir les scores, ou calculer en f64 avec BLAS | Arrondir les scores ne supprime pas la sensibilité à l'ordre de sommation : deux ordres peuvent retomber de part et d'autre d'une frontière d'arrondi. Calculer en f64 avec BLAS déplace le problème sans le supprimer, et surtout supprime les quasi-égalités que le test mesure, donc le comportement que J1 doit éprouver. |

## Références

- `docs/adr/0036-producer-host-reglable-par-le-producteur.md` (ADR-0036, reproductibilité de la somme de `synth-small`) ;
- `docs/08-invariants-et-tests.md` : INV-06 (ligne 14) et le jeu `synth-small` (ligne 45) ;
- fiche `GENERER-JEU-SYNTHETIQUE` et fiche `RECHERCHER-DENSE-EXACT`, dans `docs/phases/P1-stockage-et-recherche-exacte.md` (lignes 29 et 150, notamment la ligne 159) ;
- `tools/reference_topk.py` (ligne 429, `scores = matrix @ vector`) ; `tools/gen_synth.py` (`SMALL_FILES`, ligne 423) ;
- `tools/README.md` : somme de contrôle (ligne 129), scores (ligne 160), quasi-égalités de scores (ligne 167) ;
- `crates/crible-search/tests/inv_06_dense_exact_synth.rs` (test `expected_jsonl_holds_the_quasi_equalities_the_tolerance_allows`, vers la ligne 512) ;
- `crates/crible-search/src/dense/exact.rs` (fonction `dot`, somme f32 dans l'ordre des composantes) ;
- run GitHub `39794175404`, CI de la phase P1.
