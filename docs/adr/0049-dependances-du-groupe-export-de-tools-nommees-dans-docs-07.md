# ADR-0049 — Dépendances du groupe `export` de `tools/` nommées dans `docs/07`

- **Statut** : Remplacé par ADR-0050
- **Date** : 2026-10-23 (proposition et acceptation)
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, sur demande du propriétaire. Écart relevé par `@relecteur` au 2e tour de la tâche `EXPORTER-MODELE-ONNX` (phase P2, commit `1669ee5`)
- **Remplace** : —

## Contexte

`docs/07-stack-et-deploiement.md`, section « Côté Python », ne nomme aujourd'hui que deux paquets pour l'export ONNX :

> **Côté Python** : `uv`, `pyarrow`, `numpy`, `httpx`, `pytest`, `blake3` (ADR-0034). Les outils d'export ONNX (`optimum`, `onnxruntime`) restent dans `tools/` et ne sont pas des dépendances du paquet.

La tâche `EXPORTER-MODELE-ONNX` (statut `FAIT`, commit `1669ee5`) a déclaré dans `tools/pyproject.toml` un **groupe `uv` séparé**, `[dependency-groups] export`, qui contient six paquets, aux bornes suivantes :

| Paquet | Borne déclarée | Rôle |
|---|---|---|
| `huggingface-hub` | `>=0.36.2` | téléchargement du modèle à la révision épinglée |
| `onnxruntime` | `>=1.18` | exécution des graphes exportés |
| `optimum[onnxruntime]` | `>=2.1` | export PyTorch vers ONNX |
| `tokenizers` | `>=0.22.2` | copie du tokenizer rapide |
| `torch` | `>=2.14.1` | source de l'export |
| `transformers` | `>=4.57.6,<4.58` | définition du modèle ; borne haute imposée par `optimum-onnx` |

Trois précisions que porte ce fichier :

- **`torch` vient d'un index dédié** : `[[tool.uv.index]]` déclare `pytorch-cpu` (`https://download.pytorch.org/whl/cpu`, `explicit = true`), et `[tool.uv.sources]` associe `torch = { index = "pytorch-cpu" }`. Le projet ne fait aucun usage de GPU, et la wheel PyPI par défaut de `torch` entraîne la pile CUDA sous Linux, plusieurs gigaoctets.
- **Quatre de ces paquets sont des dépendances d'`optimum`** (`transformers`, `tokenizers`, `huggingface-hub`, `onnxruntime`), mais `tools/export_onnx.py` les importe directement, dans les fonctions qui en ont besoin. La relecture a demandé qu'ils soient déclarés explicitement plutôt que laissés au transit d'`optimum` : un import direct ne doit pas reposer sur une dépendance indirecte, qu'une version d'`optimum` peut retirer.
- **Le groupe n'est pas installé par défaut.** `uv sync` ne l'installe pas ; il faut `uv sync --group export`. C'est ce qui permet à la CI de tourner sans torch : le job `tools` de `.github/workflows/ci.yml` fait `uv sync` puis `uv run pytest`, et le job `rust` fait de même pour générer `synth-small`. Aucun des deux n'installe le groupe `export`, donc ni torch ni les bibliothèques CUDA ne sont téléchargés.

### Pourquoi cet écart demande un ADR

`docs/09-plan-de-developpement.md`, règle 5 bis : toute dépendance absente de `docs/07` demande un ADR. `docs/adr/README.md` : un ADR est requis pour toute décision qui « ajoute, retire ou remplace une dépendance ».

La question n'est pas la légitimité des paquets — ils sont installés, testés et la CI les exclut — mais **la fidélité de `docs/07` au dépôt**. Aujourd'hui, un lecteur de `docs/07` qui ouvre `tools/pyproject.toml` découvre quatre paquets de plus que la documentation ne prévoit, dont torch depuis un index non standard. ADR-0034 est le précédent de forme : une dépendance Python ajoutée par ADR, puis nommée dans `docs/07`.

Ce que le propriétaire a tranché, dans ses mots, le 2026-10-23 : « tu peux nommer les dépendances dans `docs/07` ». Le périmètre est donc la **documentation seule** : aucun paquet n'est ajouté, retiré ni changé de version.

## Décision

`docs/07-stack-et-deploiement.md`, section « Côté Python », **nomme les six paquets du groupe `export`** de `tools/`, avec quatre précisions : c'est un groupe `uv` séparé ; il est absent d'un `uv sync` par défaut et donc de la CI ; il s'installe avec `uv sync --group export` ; `torch` vient de l'index CPU `pytorch-cpu`. Ces paquets ne sont pas des dépendances du paquet client.

1. **Les six paquets sont nommés** : `optimum[onnxruntime]`, `onnxruntime`, `torch`, `transformers`, `tokenizers`, `huggingface-hub`.

2. **Les bornes de version restent dans `tools/pyproject.toml`**, seul endroit qui fait foi pour les versions. `docs/07` nomme les paquets, pas leurs bornes : recopier `>=4.57.6,<4.58` dans un document de conception créerait une seconde source de vérité, à corriger à chaque mise à jour du lock.

3. **Rien d'autre ne change.** Aucune dépendance n'est ajoutée ni retirée ; `tools/pyproject.toml`, `tools/uv.lock`, `tools/export_onnx.py` et la CI restent tels quels. Le paragraphe continue de dire que ces outils restent dans `tools/` et ne sont pas des dépendances du paquet.

Ce qui n'est **pas** décidé ici : la politique de mise à jour de ces paquets, le contenu du groupe `dev`, et la question de savoir si l'export ONNX doit un jour rejoindre le paquet client (il ne le doit pas aujourd'hui : c'est un outil hors ligne, ADR-0013).

## Conséquences

- **Positives** :
  - `docs/07` cesse d'être incomplet : les six paquets du groupe `export` y figurent, et un lecteur n'a plus à déduire torch et `transformers` d'`optimum` ;
  - la règle 5 bis de `docs/09` est respectée sans exception implicite : le groupe `export` a son ADR, comme `blake3` (ADR-0034) ;
  - la contrainte de coût de la CI est écrite noir sur blanc : le groupe n'est pas installé par défaut, donc pas de torch ni de CUDA dans les jobs ;
  - l'usage de l'index `pytorch-cpu` est documenté, alors qu'il n'était visible que dans `tools/pyproject.toml`, avec le commentaire qui l'explique.
- **Négatives / coûts** :
  - le paragraphe « Côté Python » s'allonge sensiblement : six noms de paquets, trois précisions d'installation et un renvoi d'ADR sur une seule ligne ;
  - le document nomme des paquets dont les versions peuvent changer sans lui : `docs/07` ne porte pas les bornes, donc il ne se désynchronise pas sur les versions, mais il pourrait citer un paquet que le groupe retire un jour ;
  - un lecteur pressé peut croire que ces paquets sont installés par `uv sync` : les précisions sont là pour l'éviter, pas pour rendre la ligne courte.
- **À surveiller** :
  - toute modification du groupe `export` dans `tools/pyproject.toml` (paquet ajouté ou retiré) : elle demande la mise à jour de la ligne de `docs/07`, sous un nouvel ADR si le périmètre change ;
  - le temps du job `tools` de la CI : s'il se met à installer torch, c'est que le groupe est devenu par défaut, et la promesse « ni torch ni CUDA en CI » ne tient plus ;
  - la borne `transformers < 4.58` : elle est imposée par `optimum-onnx` 0.1.0 et devra être révisée quand `optimum-onnx` relèvera son plafond. Ce point relève de `tools/pyproject.toml`, pas de `docs/07`.

### Modification appliquée

Appliquée dans le même commit que cet ADR, puisqu'il est accepté.

**`docs/07-stack-et-deploiement.md`**, section « Côté Python ». Remplacer

```
**Côté Python** : `uv`, `pyarrow`, `numpy`, `httpx`, `pytest`, `blake3` (ADR-0034). Les outils d'export ONNX (`optimum`, `onnxruntime`) restent dans `tools/` et ne sont pas des dépendances du paquet.
```

par

```
**Côté Python** : `uv`, `pyarrow`, `numpy`, `httpx`, `pytest`, `blake3` (ADR-0034). Les outils d'export ONNX restent dans `tools/` et ne sont pas des dépendances du paquet : ils forment le groupe `uv` séparé `export` de `tools/pyproject.toml` — `optimum[onnxruntime]`, `onnxruntime`, `torch`, `transformers`, `tokenizers`, `huggingface-hub` (ADR-0049). Ce groupe n'est pas installé par un `uv sync` par défaut, donc la CI ne l'installe pas ; il s'installe avec `uv sync --group export`. `torch` vient de l'index CPU `pytorch-cpu`, ce qui évite la pile CUDA sous Linux (plusieurs Go). Les bornes de version font foi dans `tools/pyproject.toml`.
```

`docs/suivi.md` n'est pas modifié par `@greffier` : c'est l'orchestrateur qui y ajoute le renvoi à cet ADR, sur la ligne d'écart du 2026-10-22.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Laisser `docs/07` tel quel, en considérant ces paquets comme couverts par `optimum` | C'est faux à deux titres. `torch` n'est pas une dépendance d'`optimum` : c'est la source de l'export, déclarée directement, et elle vient d'un index non standard que la documentation ne mentionne pas. Les quatre autres sont bien des dépendances d'`optimum`, mais `tools/export_onnx.py` les importe directement : les couvrir par transit laisserait un import direct sans déclaration, ce que la règle 5 bis de `docs/09` refuse. Le document resterait donc en retard sur le dépôt, et la question reviendrait à chaque relecture. |

## Références

- `docs/07-stack-et-deploiement.md`, section « Côté Python » (ligne modifiée) et section « Environnement de développement (Mac) » (`uv sync` dans `clients/python/` et `tools/`).
- `tools/pyproject.toml` : `[dependency-groups] export`, `[[tool.uv.index]]` (`pytorch-cpu`, `explicit`), `[tool.uv.sources]` (`torch`).
- `tools/export_onnx.py` : imports directs de `torch`, `transformers`, `tokenizers`, `huggingface_hub`, `onnxruntime` et `optimum`.
- `.github/workflows/ci.yml`, jobs `tools` et `rust` : `uv sync` sans `--group export`.
- `docs/09-plan-de-developpement.md`, règle 5 bis (dépendance absente de `docs/07`).
- `docs/suivi.md`, écart du 2026-10-22 sur `EXPORTER-MODELE-ONNX` (2e tour de relecture) ; tâche `EXPORTER-MODELE-ONNX` (P2, `FAIT`, commit `1669ee5`).
- ADR-0013 (producteurs et outils en Python, serveur Rust sur CPU) ; ADR-0034 (dépendance Python `blake3` ajoutée par ADR, précédent de forme).
