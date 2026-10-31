# ADR-0114 — Vérifier les liens intra-doc en intégration continue

- **Statut** : Accepté (accepté par anticipation par le propriétaire le 2026-10-30)
- **Date** : 2026-10-30
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, tâche hors plan NETTOYER-P6 (phase P6, branche `tache/NETTOYER-P6`), passage A, sur délégation du propriétaire du 2026-10-30
- **Remplace** : — (ne remplace aucune décision : complète ADR-0068 et ADR-0057, sans modifier le corps d'aucun d'eux)

## Contexte

### Le fait qui déclenche la décision

`docs/suivi.md`, section « Contenu de NETTOYER-P6 », point remonté par PUBLIER-OPENAPI : `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps -p crible-server` échoue sur **25 liens intra-doc cassés**, tous antérieurs à la tâche. Les liens se répartissent ainsi : `crates/crible-server/src/state.rs` (23 liens), `config.rs`, `jobs.rs`, `limits.rs` ; les cibles visées sont `ErrorCode::…`, `Notified::enable` et `error::response`.

Le même point demande de décider si `cargo doc -D warnings` devient une étape de la CI. Cette question demande un ADR pour deux raisons : `docs/07-stack-et-deploiement.md`, section « CI (GitHub Actions) », énumère à la ligne 143 les étapes de chaque push et chaque PR, et ADR-0068 a déjà fixé que cette liste ne change que par décision écrite, avec « rien d'autre ne change dans le workflow ».

### Le même défaut revient à chaque phase

Les liens cassés ne sont découverts qu'en fin de phase, parce qu'aucune vérification ne tourne à chaque push :

- **NETTOYER-P4** a réparé des liens rustdoc, et `cargo doc -D warnings` était vert sur le workspace à la fin de la phase (ligne NETTOYER-P4 du tableau des tâches de `docs/suivi.md`) ;
- **NETTOYER-P5** en a réparé **15** de plus, cassés pendant P5 dans `crible-core/src/config.rs` et dans `crible-search` et `crible-store` (ligne NETTOYER-P5 du même tableau) ;
- **NETTOYER-P6** en trouve **25** dans `crible-server`.

Trois phases de suite, le même nettoyage. Le coût est double : la réparation arrive tard, quand le contexte du commentaire est perdu, et elle est mêlée à une tâche de nettoyage qui avait d'autres objets.

### Ce que la CI fait aujourd'hui

Le job `Rust` de `.github/workflows/ci.yml` lance, dans cet ordre : `Check formatting`, `Lint` (`cargo clippy --all-targets --locked -- -D warnings`), `Install uv`, `Generate the synthetic dataset`, `Test` (`cargo test --locked`), les étapes `failpoints` d'ADR-0068, puis `Test the Python client against the binary`. Aucune étape ne construit la documentation : un lien intra-doc cassé est invisible pour la CI.

### Ce que la décision ne coûte pas

`ort` est déclaré en `load-dynamic` (workspace). Construire la documentation ne charge donc ni ONNX Runtime, ni modèle, ni donnée : `cargo doc --no-deps` lit le code et ses commentaires. L'étape n'ajoute aucune dépendance, aucun secret et aucun artefact de modèle.

## Décision

**1. Le job `Rust` reçoit une étape « Check the documentation links ».** Elle est placée **juste après** l'étape `Lint`, celle qui lance `cargo clippy --all-targets --locked -- -D warnings`. Sa commande est :

```
cargo doc --no-deps --workspace --locked
```

La variable d'environnement `RUSTDOCFLAGS: -D warnings` est posée **sur cette étape seulement**. L'étape tourne avec les features par défaut, et **sans** `--document-private-items`.

**2. Rien d'autre ne change dans le workflow.** Ni les autres jobs, ni le cache, ni `timeout-minutes`, ni l'ordre des autres étapes.

**3. Le passage A de `@dev` dans NETTOYER-P6 répare d'abord, ajoute l'étape ensuite.** L'ordre est : réparer tous les liens cassés du workspace, puis ajouter l'étape à `ci.yml`. C'est `@dev` qui modifie `ci.yml`, pas `@greffier`. Une étape rouge dès son ajout ne prouverait rien et bloquerait la branche de phase.

**4. `docs/07-stack-et-deploiement.md` est mis à jour** : la liste des étapes de chaque push et PR, ligne 143, reçoit la commande et cite ADR-0114. Le texte reste ainsi exact, comme ADR-0068 l'avait exigé pour ses propres étapes.

**Ce qui n'est pas décidé ici.** Le présent ADR ne touche ni aux tests laissés locaux (ADR-0057, ADR-0062, ADR-0063, ADR-0085, ADR-0109), ni au job `rust-onnx`, ni à ses `--skip`, ni à la règle 6 d'`AGENTS.md` sur la définition de FAIT, ni aux commentaires de code eux-mêmes.

## Conséquences

- **Positives** :
  - un lien intra-doc cassé échoue à chaque push et chaque PR, donc au moment où il est écrit, et non trois phases plus tard ;
  - la réparation revient à l'auteur du commentaire, qui a le contexte ;
  - le nettoyage de fin de phase cesse de porter cette dette : NETTOYER-P4, NETTOYER-P5 et NETTOYER-P6 sont les trois derniers cas connus ;
  - la documentation publique du workspace reste construisible, ce qui est aussi une condition d'usage de `cargo doc` par un lecteur ;
  - aucun coût de dépendance, de secret ou de donnée : `ort` est en `load-dynamic`.
- **Négatives / coûts** :
  - le job `Rust` s'allonge du temps de construction de la documentation du workspace ;
  - `-D warnings` sur rustdoc est strict : un lien vers un élément privé, renommé ou déplacé fait échouer la CI, même si aucune ligne de code exécutable ne change. C'est le but, mais cela demande de tenir les commentaires ;
  - un `#[doc(hidden)]` ou un commentaire de test mal formé peut produire un avertissement rustdoc jusqu'ici ignoré.
- **À surveiller** : la **durée du job `Rust`**, relevée au premier run après l'ajout. Une étape instable se traite par un **nouvel ADR**, jamais par `continue-on-error` : une étape qui peut échouer sans conséquence ne prouve rien, et c'est exactement le défaut qu'ADR-0068 a corrigé pour les tests `failpoints`.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Statu quo : réparer les liens en fin de phase | Déjà arrivé trois fois (NETTOYER-P4, NETTOYER-P5, NETTOYER-P6) : la réparation arrive tard, hors du contexte de l'auteur, et le même défaut revient à chaque phase. |
| Ajouter `cargo doc` à la définition de FAIT locale seulement (règle 6 d'`AGENTS.md`) | Dépend de la discipline de chaque passage et de chaque agent ; rien ne le vérifie, et un oubli ne se voit qu'à la phase suivante. La CI est le seul contrôle qui ne s'oublie pas. |
| Ajouter `--document-private-items` | Multiplierait les liens à tenir (documents internes, champs privés, modules d'implémentation) sans aucun bénéfice pour l'API publique, et rendrait l'étape plus fragile pour un gain nul côté lecteur. |
| Poser `RUSTDOCFLAGS` globalement sur le job | Ferait échouer la CI pour un avertissement rustdoc pendant les étapes de test et de lint, hors du périmètre de la décision ; la variable est donc locale à l'étape. |

## Références

- `docs/suivi.md` : section « Contenu de NETTOYER-P6 », point de PUBLIER-OPENAPI (25 liens cassés de `crible-server`, question de la CI) ; lignes NETTOYER-P4, NETTOYER-P5 et NETTOYER-P6 du tableau des tâches.
- `docs/07-stack-et-deploiement.md` : `:143` (liste des étapes de chaque push et PR), `:144` (job `rust-onnx`, cinq `--skip`).
- `AGENTS.md` : règle 6 (définition de FAIT), règle 12 (simplicité).
- ADR-0068 (étapes `failpoints` du job `Rust`, « rien d'autre ne change dans le workflow »), ADR-0057 (job `rust-onnx`, tests locaux), ADR-0059 (cache du modèle), ADR-0112 (étape « Test the Python client against the binary »).
- `.github/workflows/ci.yml` (job `Rust`).
