# ADR-0068 — Construire et tester `crible-store` avec la feature `failpoints` en intégration continue

- **Statut** : Accepté (accepté par anticipation le 2026-10-26, confirmé par le propriétaire le 2026-10-26)
- **Date** : 2026-10-26 (proposition et acceptation anticipée)
- **Décideurs** : propriétaire
- **Décision** : accepté par anticipation le 2026-10-26, confirmé par le propriétaire le 2026-10-26, tel que proposé. Le job Rust d'`.github/workflows/ci.yml` construit et teste aussi `crible-store` avec la feature `failpoints`, à chaque push et chaque PR, avec les deux commandes exactes de la décision 1, sans ONNX Runtime.
- **Proposé par** : agent greffier, sur demande de l'orchestrateur, pour la tâche `NETTOYER-P3` (phase P3, branche `tache/NETTOYER-P3`), passage A. Origine : le point remonté par `@relecteur` pendant `COMPACTER-INDEX`, déjà consigné au suivi
- **Remplace** : — (complète ADR-0057, qui reste en vigueur et `Accepté` ; son corps n'est pas modifié)

## Contexte

### Les tests de panne et de concurrence n'existent que sous une feature

Les tests de `crible-store` qui vérifient INV-02, INV-03, INV-04 et INV-14, ainsi que les tests de panne d'`attacher_lot`, ne sont compilés et exécutés qu'avec la feature cargo `failpoints` (`crates/crible-store/Cargo.toml`, section `[features]`). Sans elle, ces tests **n'existent pas** : ils ne sont ni sautés ni ignorés, ils ne sont jamais construits.

### La CI ne construisait jamais cette feature

Le job Rust de `.github/workflows/ci.yml` lançait `cargo fmt`, `cargo clippy --all-targets` et `cargo test --locked`, sans `--features failpoints`. Aucun job ne construisait donc ces tests. Un test y était resté rouge sans que personne le voie : `attacher_lot::no_sqlite_write_follows_the_commit_point`, relevé par `@relecteur` pendant la tâche `COMPACTER-INDEX` (rapport d'attache persisté portant `"acceptance": null`). C'est l'argument de ce point de `NETTOYER-P3` : un test que la CI ne construit pas ne prouve rien, et son défaut reste invisible.

### L'état local

Les deux commandes de la décision, lancées en local, donnent **266 tests verts et 1 ignoré** pour `crible-store` sous `failpoints`.

### Ce qui a déjà changé

Le passage A de `NETTOYER-P3` a ajouté au job Rust, juste après `cargo test --locked` :

- une étape `Lint the failpoint build` : `cargo clippy -p crible-store --all-targets --locked --features failpoints -- -D warnings` ;
- une étape `Test the failpoint build` : `cargo test -p crible-store --locked --features failpoints`.

Ces étapes n'ont d'effet sur aucun autre job, et elles ne demandent ni ONNX Runtime ni donnée de production : `ort` est déclaré en `load-dynamic` (`Cargo.toml`, workspace) et les tests de panne travaillent dans des dossiers temporaires.

### La conception et le texte incomplet

`docs/07-stack-et-deploiement.md`, section « CI (GitHub Actions) », énumère les étapes de chaque push et PR : « `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, puis `uv run pytest` pour `clients/python` ». Au point `rust-onnx`, le même document dit : « Le job Rust actuel reste inchangé : `cargo test --locked`, sans ONNX Runtime ». Aucune de ces phrases ne contredit l'ajout : elles sont devenues **incomplètes**, et un lecteur ne saurait pas que la CI construit les tests de panne.

### Ce qui n'est pas décidé ici

Cet ADR ne touche ni à la liste des tests laissés locaux (ADR-0057, ADR-0062, ADR-0063), ni au job `rust-onnx`, ni au cache du modèle (ADR-0059), ni au seuil de recette (ADR-0052), ni aux invariants eux-mêmes. Il ne change aucun format, aucun schéma et aucune valeur par défaut du moteur.

## Décision

**1. Le job Rust de la CI construit et teste `crible-store` avec la feature `failpoints`, à chaque push et chaque PR, sans ONNX Runtime.** Les deux étapes, dans cet ordre et après `cargo test --locked`, sont :

```
cargo clippy -p crible-store --all-targets --locked --features failpoints -- -D warnings
cargo test -p crible-store --locked --features failpoints
```

**2. Le périmètre est le paquet `crible-store` seulement.** La feature reste désactivée partout ailleurs : `--workspace` n'est pas ajouté, et aucune autre crate ne la reçoit. La feature est un outil de test (`CRIBLE_FAILPOINT`), jamais activée dans une construction de version.

**3. Rien d'autre ne change dans le workflow.** Ni `timeout-minutes`, ni le cache, ni les autres étapes, ni les tests laissés locaux.

`docs/07-stack-et-deploiement.md` est mis à jour pour que le texte reste exact : la liste des étapes de chaque push et PR reçoit les deux commandes, et la phrase du point `rust-onnx` reste vraie en nommant les étapes `failpoints` d'ADR-0068.

## Conséquences

- **Positives** :
  - les tests de panne et de concurrence (INV-02, INV-03, INV-04, INV-14, tests de panne d'`attacher_lot`) tournent à chaque push et chaque PR : un défaut comme `no_sqlite_write_follows_the_commit_point` ne peut plus rester invisible pendant plusieurs tâches ;
  - le lint `--features failpoints` couvre aussi les cibles de test construites avec la feature, donc un avertissement propre à cette configuration échoue la CI ;
  - aucune dépendance, aucun service et aucune donnée supplémentaire ne sont nécessaires.
- **Négatives / coûts** :
  - le job Rust dure un peu plus : une compilation de plus de `crible-store` et de ses cibles de test, sans cache de compilateur séparé ;
  - les tests de panne tuent des processus enfants (`inv_04_crash_recovery`) et prennent plus de temps que les autres ; une instabilité propre à cette configuration deviendrait une CI rouge.
- **À surveiller** : la durée du job Rust avant et après l'ajout, et toute instabilité des tests de panne en CI. Si l'un d'eux devient instable sans défaut réel, la réponse passe par un nouvel ADR, jamais par un `continue-on-error` ni par le retrait silencieux d'une étape.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| (b) Laisser ces tests locaux, hors CI | C'est le défaut même que ce point corrige : la CI ne les construisait jamais, et un test rouge est resté invisible pendant plusieurs tâches. Un test qu'aucune CI ne construit ne protège rien. |
| (c) Un job séparé pour la feature `failpoints` | Rien à isoler : aucune dépendance supplémentaire (`ort` reste en `load-dynamic`), aucune donnée de production, aucun service. Un job de plus coûterait un runner et un démarrage pour la même preuve. |
| Ajouter `--workspace` aux commandes `failpoints` | La feature n'a de sens que pour `crible-store` : l'activer ailleurs ferait entrer des points d'injection dans des paquets qui ne les utilisent pas, et ralentirait la compilation de tout le graphe. |

## Documents à modifier après acceptation

Appliqué dans le même travail que cet ADR, sur la branche `tache/NETTOYER-P3`, dans un commit qui cite `ADR-0068` :

1. **`docs/07-stack-et-deploiement.md`, section « CI (GitHub Actions) », première puce** (ligne 137) : ajouter les deux commandes `failpoints` à la liste des étapes de chaque push et PR, après `cargo test`.
2. **`docs/07-stack-et-deploiement.md`, même section, puce `rust-onnx`** (ligne 138) : la phrase « Le job Rust actuel reste inchangé : `cargo test --locked`, sans ONNX Runtime » reste exacte en nommant les étapes `failpoints` de cet ADR.
3. **`docs/adr/README.md`** : ajouter la ligne d'index de cet ADR.

Rien d'autre dans `docs/07` n'est modifié.

## Références

- `docs/07-stack-et-deploiement.md`, section « CI (GitHub Actions) ».
- `docs/08-invariants-et-tests.md` : INV-02, INV-03, INV-04, INV-14, et les règles de saut de test.
- `.github/workflows/ci.yml`, job Rust, étapes `Lint the failpoint build` et `Test the failpoint build`.
- ADR-0057 (job `rust-onnx`, ONNX Runtime en CI ; corps inchangé), ADR-0059 (cache et téléchargement de l'artefact), ADR-0032 (aucun contenu de production en CI), ADR-0062 et ADR-0063 (tests laissés locaux, nommés un par un).
- `crates/crible-store/Cargo.toml`, section `[features]` : feature `failpoints`, `CRIBLE_FAILPOINT`.
