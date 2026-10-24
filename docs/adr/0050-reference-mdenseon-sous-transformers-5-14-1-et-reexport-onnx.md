# ADR-0050 — Référence mDenseOn sous transformers 5.14.1 et ré-export de l'ONNX

- **Statut** : Accepté (décision du propriétaire le 2026-10-23)
- **Date** : 2026-10-23 (proposition et acceptation)
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, sur demande du propriétaire. Origine : `QUESTION-CONCEPTION` remontée par `@dev` pendant la tâche `CONSTITUER-ECHANTILLON-LEGIFACILE` (phase P2). Verdict de `@architecte` : **NON-COUVERT** par `docs/04` et les ADR acceptés ; les options « abaisser le seuil » et « accepter par exception » sont des **ECART** à `docs/04` et à INV-12.
- **Remplace** : ADR-0049 (seulement la liste des paquets du groupe `export` de `tools/`, voir point 3)

## Contexte

### Ce que produisent les vecteurs de production

Les vecteurs de production de Legifacile viennent de `lightonai/mDenseOn`, révision `a5fdb000f7a21da96c3bddde3a782ef777316df3`, avec le prompt `document: `, exécutés sous **transformers 5.14.1**. Un ré-encodage PyTorch sous cette même version retrouve ces vecteurs à un cosinus de **0,999996 à 0,999999**. Autrement dit, la référence est connue, elle est reproductible, et elle est fixée par la version de `transformers` employée.

### Ce que produit l'ONNX de Crible

`EXPORTER-MODELE-ONNX` (`tools/export_onnx.py`, commit `1669ee5`) a produit l'ONNX fp32 sous **transformers 4.57.6**, plafond imposé par `optimum-onnx`. Mesure faite sur l'échantillon réel, dans `data/legifacile-sample/README.md` (gitignoré), section « Limite connue » :

> l'ONNX fp32 exporté par `EXPORTER-MODELE-ONNX` (transformers 4.57.6) ne les retrouve qu'à 0,983–0,990 **avec** prompt (0,708–0,768 sans prompt) sur 10 passages par corpus.

Le critère de parité écrit dans la fiche d'`EXPORTER-MODELE-ONNX` (cosinus PyTorch contre ONNX fp32 ≥ 0,9999 sur 100 textes) a bien été tenu : minimum 0,9999999999259, moyenne 0,9999999999976. Mais il comparait **PyTorch et ONNX sous la même version 4.57.6**. Il est structurellement aveugle à une dérive de version : les deux côtés se trompent ensemble, et le cosinus reste à 1.

### Pourquoi un `optimum-onnx` compatible n'existe pas

Fait vérifié par l'orchestrateur sur PyPI le 2026-10-23 : la dernière version d'`optimum-onnx` est la **0.1.0** (publiée le 2025-12-23), qui exige `transformers>=4.36,<4.58.0`. Aucune version publiée ne permet `transformers` 5. Le groupe `export` de `tools/` est donc bloqué sous 4.57.6 tant qu'`optimum` reste dans la chaîne.

### Pourquoi Legifacile a déjà rencontré cet écart

Legifacile a heurté le même problème et l'a tranché : la version de `transformers` est entrée dans l'empreinte de définition de son corpus (`legifacile-sandbox/docs/MDENSEON.md`, début et §5), après un refus de recette à un cosinus de 0,988. `docs/04` cite déjà ce précédent : « la recette de Legifacile, qui a déjà détecté un écart de version de transformers (cosinus 0,988) ». Crible n'a pas le même besoin : sa recette re-mesure **chaque lot** au moment de l'attache (contrôle 9), alors que Legifacile encode 8,58 M passages une fois puis les réconcilie. C'est ce qui permet à Crible de fonder son garde-fou sur une mesure plutôt que sur un identifiant.

### Ce qui casse si rien ne change

- **La recette refuserait un lot correct.** Le contrôle 9 (`docs/04`, `acceptance_min_cosine: 0.999`, INV-12) ré-encode 200 passages en ONNX fp32, avec `prompt_document`, et compare aux vecteurs du lot. Un minimum de 0,983 à 0,990 est sous le seuil : `422 acceptance_failed`. Or la fiche `RECETTE-IMPORT` exige d'accepter ce lot, qui est le lot de référence de l'échantillon Legifacile. J2 devient inatteignable.
- **Les requêtes ne seraient pas dans l'espace des documents.** Crible encoderait `query: …` avec un modèle qui diffère de celui des documents d'environ 0,988 de cosinus. Cela touche `ENCODER-REQUETES`, MES-04 et le rappel de toute la recherche dense.

Le coût de correction est aujourd'hui nul en données : **aucun index n'existe encore**, aucun vecteur n'a été encodé par Crible.

## Décision

**L'implémentation de référence de mDenseOn est `lightonai/mDenseOn`, révision `a5fdb000f7a21da96c3bddde3a782ef777316df3`, exécutée sous transformers 5.14.1, pooling CLS, normalisation L2, prompts `query: ` et `document: `. L'ONNX de Crible doit s'y conformer.** Pour y parvenir, la chaîne d'export quitte `optimum` et passe par `torch.onnx.export` sous `transformers==5.14.1`.

1. **La référence est une définition, pas un fichier.** Ce qui fait foi, c'est la révision du modèle **et** la version de `transformers` qui l'exécute : pooling CLS, normalisation L2, prompts `query: ` et `document: `. Un ONNX qui s'en écarte de plus de 0,001 de cosinus n'est pas conforme, quel que soit le critère de parité PyTorch-ONNX.

2. **La version de bibliothèque n'entre pas dans l'empreinte.** ADR-0028 reste inchangé, et la forme canonique de l'empreinte aussi : ni `transformers`, ni `torch`, ni `onnxruntime` n'y entrent. La version est **enregistrée** dans `model.json`, à la racine, dans un champ `export` qui donne les versions de `transformers`, `torch` et `onnxruntime` employées pour l'export. Ce champ est **hors du JSON canonique** hashé par ADR-0028 : l'ajouter ne change pas la forme de l'empreinte, et un `model.json` ancien, sans ce champ, reste lisible. Le garde-fou est la mesure (recette du contrôle 9, INV-12), pas l'identifiant : une empreinte qui contiendrait la version rendrait `422 incompatible_model` sur des lots valides dès qu'une bibliothèque monte, alors qu'aucun vecteur n'a changé. Ce champ sert au diagnostic — savoir sous quoi un ONNX a été produit —, jamais à refuser un lot.

3. **La chaîne d'export n'utilise plus `optimum`.** `optimum` est **retiré** du groupe `export` de `tools/`, faute de version compatible avec transformers 5 :
   - l'export fp32 passe par `torch.onnx.export`, sous `transformers==5.14.1` **épinglé** (borne exacte, pas une plage) ;
   - la quantification INT8 dynamique passe par `onnxruntime.quantization` ;
   - le groupe `export` devient : `onnxruntime`, `onnx`, `torch` (index CPU `pytorch-cpu`, inchangé), `transformers==5.14.1`, `tokenizers`, `huggingface-hub`, et `onnxscript` **si** l'exportateur l'exige ;
   - cet ADR **remplace ADR-0049 sur la liste des paquets** : ADR-0049 reste valable pour le reste (groupe `uv` séparé, absent d'un `uv sync` par défaut, donc de la CI, index CPU pour `torch`, bornes dans `tools/pyproject.toml`). À l'acceptation, ADR-0049 passe en « Remplacé par ADR-0050 » et la ligne « Côté Python » de `docs/07` est mise à jour.

4. **Le critère de fidélité de l'export est resserré à deux conditions, toutes deux obligatoires.**
   - **(i) Parité PyTorch-ONNX** : PyTorch sous transformers 5.14.1 contre ONNX fp32, cosinus ≥ 0,9999 sur les 100 textes de parité existants. Ce critère reste celui de la fiche `EXPORTER-MODELE-ONNX` ; il contrôle la qualité de la conversion, pas la fidélité au modèle de production.
   - **(ii) Fidélité au modèle de production** : cosinus entre l'ONNX fp32, avec prompt de document, et les **vecteurs de production** de l'échantillon `data/legifacile-sample/`, ≥ **0,999** — le seuil de recette lui-même. Sur un tirage **déterministe** de 100 passages par structure, soit **600 passages**. C'est le critère qui n'existait pas et qui aurait détecté l'écart de 0,983–0,990.
   - Le rapport de la tâche donne **minimum, p1 et médiane** pour les deux critères, et reverifie **`max_tokens` = 8192** après le ré-export.

5. **Une nouvelle tâche hors plan porte ce travail** : `REEXPORTER-MODELE-TRANSFORMERS-5`, dans `docs/phases/P2-encodeur-et-recette.md`, placée juste avant `CHARGER-MODELE`. Elle dépend d'`EXPORTER-MODELE-ONNX` et de `CONSTITUER-ECHANTILLON-LEGIFACILE` ; `CHARGER-MODELE` dépend désormais d'elle, puisque le modèle chargé doit être celui de la référence. Si l'export par `torch.onnx.export` s'avère impossible pour ce modèle — opérateur non supporté, sortie instable, graphe non exécutable à 8 192 tokens —, la tâche **s'arrête** et remonte une `QUESTION-CONCEPTION` ; elle ne se rabat pas sur l'ONNX 4.57.6 en silence.

6. **Aucun seuil de recette n'est touché.** `acceptance_min_cosine: 0.999`, `acceptance_sample: 200`, le contrôle 9 et INV-12 restent tels quels. Le problème n'était pas le seuil, c'était le modèle qu'on lui présentait.

Ce qui n'est **pas** décidé ici : la politique de mise à jour de `transformers` en P3 et au-delà (une montée de version demandera une nouvelle mesure de fidélité, donc un nouvel ADR si elle change les vecteurs) ; le retour éventuel d'`optimum` si une version compatible transformers 5 paraît ; et le choix de précision INT8 (déjà tranché ailleurs).

## Conséquences

- **Positives** :
  - la recette peut accepter le lot correct de l'échantillon : `RECETTE-IMPORT` et J2 cessent d'être bloqués par un problème de version ;
  - les documents et les requêtes vivent enfin dans le **même** espace vectoriel : c'est la condition du rappel de la recherche dense ;
  - la chaîne d'export ne dépend plus d'un paquet dont le plafond de version bloque le projet ; `torch.onnx.export` et `onnxruntime.quantization` sont dans les paquets déjà présents ou explicitement déclarés, sans dépendance indirecte ;
  - le critère (ii) crée un garde-fou **mesuré contre la production**, que le critère (i) ne pouvait pas donner : un futur export ne pourra plus être « parfaitement paritaire » et pourtant faux ;
  - `model.json` porte désormais la trace des versions d'export (`export`), utile au diagnostic, sans peser sur l'empreinte ;
  - le coût en données est nul : aucun index n'existe encore.
- **Négatives / coûts** :
  - l'empreinte du modèle change, puisque le hash du fichier fp32 change ; aucun index n'étant créé, le coût est nul aujourd'hui ;
  - la fiche `EXPORTER-MODELE-ONNX` s'avère **incomplète** après coup : son critère de parité était aveugle à une dérive de version. Elle reste `FAIT` — son critère écrit était tenu —, mais la leçon est que tout critère de conversion doit s'ancrer sur une référence externe, pas sur une comparaison interne ;
  - la tâche `REEXPORTER-MODELE-TRANSFORMERS-5` est **hors plan** : elle allonge P2 d'une tâche et décale `CHARGER-MODELE`, `ENCODER-REQUETES`, MES-04 et `RECETTE-IMPORT` ;
  - l'épinglage exact `transformers==5.14.1` est rigide : il faudra le relever à la main, avec une nouvelle mesure ;
  - `torch.onnx.export` est un chemin moins outillé qu'`optimum` : il n'y a plus de `--task feature-extraction` ni de vérification de support d'opérateur toute faite. Le risque est réel, d'où la clause d'arrêt du point 5 ;
  - le champ `export` ajoute un endroit où une version peut vieillir sans que rien ne le signale.
- **À surveiller** :
  - **la mesure de fidélité (ii)** : si un futur export passe de 0,9996 à 0,9991, la décision tient encore mais la marge s'use ; un cosinus qui descend sous 0,999 signifie que la chaîne d'export a dérivé ;
  - **`max_tokens` = 8192** après le ré-export : c'est le seul endroit où `torch.onnx.export` peut produire un graphe plus fragile qu'`optimum` ;
  - **la parité INT8** (MES-04) : elle se mesure après le ré-export, et le modèle INT8 reste hors de l'empreinte, donc une dérive INT8 ne serait vue que par MES-04 ;
  - **la sortie d'un `optimum-onnx` compatible transformers 5** : elle rendrait possible un retour, sans obligation ;
  - **`docs/04`, bullet `max_tokens`**, qui renvoie à `EXPORTER-MODELE-ONNX` : après acceptation, la vérification de `max_tokens` est faite par `REEXPORTER-MODELE-TRANSFORMERS-5`. Le renvoi peut rester, ou être nommé plus précisément lors d'une prochaine mise à jour de `docs/04` ; ce n'est pas corrigé par cet ADR.
  - **Note ajoutée le 2026-10-24, après l'acceptation, avec l'accord du propriétaire (ADR-0057).** La CI exécute désormais l'artefact **déjà exporté** publié dans le dépôt de modèle Hugging Face, et non un ré-export : le job `rust-onnx` télécharge le graphe fp32 épinglé par un sha de commit Hugging Face et vérifie ses hash blake3 contre un manifeste versionné. Le champ `export` reste hors empreinte et les versions restent celles décrites ci-dessus. Toute modification d'un fichier de l'artefact ou de ses hash relève donc d'un **nouvel ADR** : ce n'est pas une mise à jour de manifeste.

### Modifications à appliquer après acceptation

Aucune n'est appliquée maintenant. Elles le seront dans un commit `docs: apply ADR-0050 …`, par `@greffier` pour les documents de conception, par le propriétaire pour le statut d'ADR-0049 et par l'orchestrateur pour `docs/suivi.md`.

#### 1. `docs/04-donnees-et-ingestion.md`, section « Le modèle »

Dans le bloc `models/<name>/model.json`, après la ligne `"max_tokens": 8192,`, insérer :

```json
  "export": {
    "transformers": "<version>",
    "torch": "<version>",
    "onnxruntime": "<version>"
  },
```

Après le bullet `- **`max_tokens`** : …`, insérer :

```
- **`export`** : versions de `transformers`, `torch` et `onnxruntime` qui ont produit le fichier fp32. Ce champ est **hors de l'empreinte** (ADR-0028, ADR-0050) : il sert au diagnostic, jamais à refuser un lot. La fidélité du modèle se mesure par la recette (contrôle 9, INV-12).
```

Les valeurs entre chevrons sont écrites par `tools/export_onnx.py` au moment de l'export ; `docs/04` ne les recopie pas, comme il ne recopie pas les bornes de `tools/pyproject.toml`.

#### 2. `docs/07-stack-et-deploiement.md`, section « Côté Python »

Remplacer la ligne actuelle

```
**Côté Python** : `uv`, `pyarrow`, `numpy`, `httpx`, `pytest`, `blake3` (ADR-0034). Les outils d'export ONNX restent dans `tools/` et ne sont pas des dépendances du paquet : ils forment le groupe `uv` séparé `export` de `tools/pyproject.toml` — `optimum[onnxruntime]`, `onnxruntime`, `torch`, `transformers`, `tokenizers`, `huggingface-hub` (ADR-0049). Ce groupe n'est pas installé par un `uv sync` par défaut, donc la CI ne l'installe pas ; il s'installe avec `uv sync --group export`. `torch` vient de l'index CPU `pytorch-cpu`, ce qui évite la pile CUDA sous Linux (plusieurs Go). Les bornes de version font foi dans `tools/pyproject.toml`.
```

par

```
**Côté Python** : `uv`, `pyarrow`, `numpy`, `httpx`, `pytest`, `blake3` (ADR-0034). Les outils d'export ONNX restent dans `tools/` et ne sont pas des dépendances du paquet : ils forment le groupe `uv` séparé `export` de `tools/pyproject.toml` — `onnx`, `onnxruntime`, `torch`, `transformers`, `tokenizers`, `huggingface-hub`, plus `onnxscript` si l'exportateur l'exige (ADR-0050). `optimum` n'en fait plus partie : aucune version publiée ne permet `transformers` 5 (ADR-0050). Ce groupe n'est pas installé par un `uv sync` par défaut, donc la CI ne l'installe pas ; il s'installe avec `uv sync --group export`. `torch` vient de l'index CPU `pytorch-cpu`, ce qui évite la pile CUDA sous Linux (plusieurs Go). Les bornes de version font foi dans `tools/pyproject.toml`.
```

#### 3. `docs/adr/0049-dependances-du-groupe-export-de-tools-nommees-dans-docs-07.md`

Changer la ligne de statut :

```
- **Statut** : Remplacé par ADR-0050
```

C'est la seule modification permise sur cet ADR accepté, avec la correction de coquilles. Dans l'index, la ligne d'ADR-0049 passe au statut `Remplacé par ADR-0050`, date inchangée.

#### 4. `docs/phases/P2-encodeur-et-recette.md`

**a.** Dans la fiche `CHARGER-MODELE`, remplacer `- **Dépend de** : EXPORTER-MODELE-ONNX` par :

```
- **Dépend de** : REEXPORTER-MODELE-TRANSFORMERS-5
```

**b.** Insérer la fiche suivante **juste avant** `### CHARGER-MODELE` :

```
### REEXPORTER-MODELE-TRANSFORMERS-5
- **Objectif** : ré-exporter l'ONNX de mDenseOn sous `transformers==5.14.1`, la version qui a produit les vecteurs de production de Legifacile, et prouver la fidélité contre ces vecteurs.
- **Dépend de** : EXPORTER-MODELE-ONNX, CONSTITUER-ECHANTILLON-LEGIFACILE
- **À lire** : `04-donnees-et-ingestion.md` (le modèle) ; `08-invariants-et-tests.md` (INV-12, MES-04) ; `adr/0050-reference-mdenseon-sous-transformers-5-14-1-et-reexport-onnx.md` ; `data/legifacile-sample/README.md` (« Limite connue »).
- **Livrables** :
  - `tools/pyproject.toml` et `tools/uv.lock` : `optimum` retiré du groupe `export`, `transformers==5.14.1` épinglé, `onnx` et `onnxscript` ajoutés si l'exportateur les exige ;
  - `tools/export_onnx.py` adapté : export fp32 par `torch.onnx.export`, quantification INT8 dynamique par `onnxruntime.quantization` ; `model.json` porte le champ `export` (versions de `transformers`, `torch`, `onnxruntime`) ;
  - `model.json` réécrit de façon atomique, `max_tokens` reverifié à 8192 ;
  - `tools/README.md` : la nouvelle procédure d'export, la disparition d'`optimum`, et la façon de reproduire les deux mesures ;
  - les chiffres des deux critères, dans le rapport de la tâche.
- **Critères d'acceptation** :
  - (i) sur les 100 textes de parité, cosinus PyTorch (transformers 5.14.1) contre ONNX fp32 ≥ 0,9999 ;
  - (ii) sur un tirage déterministe de 100 passages par structure, soit 600 passages de `data/legifacile-sample/`, cosinus ONNX fp32 avec prompt de document contre les vecteurs de production ≥ 0,999 ; minimum, p1 et médiane dans le rapport ;
  - `max_tokens` = 8192 vérifié sur le graphe ré-exporté ;
  - l'export est reproductible : même révision, mêmes hash fp32 ;
  - `tools/export_onnx.py` échoue proprement, sans `unwrap()` ni trace non interceptée, si le moteur d'export refuse le modèle.
- **Invariants / mesures** : INV-11 (aucune troncature silencieuse), INV-12 (seuil de recette) ; prépare MES-04.
- **Hors périmètre** : le code Rust ; les seuils de recette ; le modèle INT8 au-delà de sa quantification ; toute modification de `docs/`.

**Clause d'arrêt** : si `torch.onnx.export` ne produit pas un graphe exécutable pour ce modèle (opérateur non supporté, sortie instable, exécution à 8 192 tokens impossible), la tâche **s'arrête** et remonte une `QUESTION-CONCEPTION` ; elle ne réutilise pas l'ONNX exporté sous transformers 4.57.6, qui ne retrouve les vecteurs de production qu'à 0,983–0,990.
```

#### 5. `docs/suivi.md` — fait par l'orchestrateur, pas par `@greffier`

- **Section « Écarts à la conception »**, ajouter la ligne :

```
| 2026-10-23 | CONSTITUER-ECHANTILLON-LEGIFACILE (`QUESTION-CONCEPTION` de `@dev`, verdict `@architecte` NON-COUVERT) | L'ONNX fp32 exporté sous transformers 4.57.6 ne retrouve les vecteurs de production de l'échantillon (transformers 5.14.1) qu'à un cosinus de 0,983–0,990, sous `acceptance_min_cosine: 0.999` : la recette refuserait un lot correct, et les requêtes ne seraient pas dans l'espace des documents. `optimum-onnx` 0.1.0 plafonne `transformers` à `<4.58` | ADR-0050 | En attente |
```

- **Tableau des tâches** : passer `CONSTITUER-ECHANTILLON-LEGIFACILE` en `BLOQUE`, avec le renvoi à ADR-0050. Après acceptation, ajouter la ligne `REEXPORTER-MODELE-TRANSFORMERS-5` (P2, `A-FAIRE`, dépendances `EXPORTER-MODELE-ONNX` et `CONSTITUER-ECHANTILLON-LEGIFACILE`), et mettre à jour la fiche de `CHARGER-MODELE`.
- **Journal** : une ligne pour la question de conception, une pour l'acceptation éventuelle.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| (b) Abaisser `acceptance_min_cosine`, ou ajouter une exception pour cet échantillon | C'est un **ECART** à `docs/04` (contrôle 9) et à INV-12, donc un affaiblissement du seul contrôle qui détecte une dérive de modèle. Et cela ne corrige rien : les requêtes resteraient encodées par un modèle distant des documents d'environ 0,988 de cosinus, donc le rappel dense resterait faux. Le seuil n'était pas trop haut ; le modèle présenté au seuil était le mauvais. |
| (c) Accepter l'échantillon par décision explicite, sans changer le modèle | **ECART** à INV-12 et à la fiche `RECETTE-IMPORT`, qui exige que la recette accepte le lot correct. Une exception écrite pour un lot précis ne se généralise pas : le producteur Legifacile ré-encoderait ses 8,58 M passages sous 4.57.6, ou Crible resterait faux. |
| Faire entrer la version de `transformers` dans l'empreinte du modèle | Change un **format persistant** (ADR-0028), pour un garde-fou que la recette assure déjà lot par lot. Conséquence concrète : une montée de bibliothèque rendrait `422 incompatible_model` sur des lots valides alors qu'aucun vecteur n'a changé, et imposerait un ré-encodage complet pour un simple changement de paquet. Legifacile a fait ce choix dans un autre contexte (encodage massif unique, 8,58 M passages, puis réconciliation) ; Crible re-mesure à chaque attache. La version est donc **enregistrée** (champ `export`), pas **hachée**. |
| Attendre un `optimum-onnx` compatible transformers 5 | Rien n'est annoncé : la dernière version, 0.1.0 (2025-12-23), exige `transformers<4.58.0`. Attendre bloque P2 sans date, alors que le remède est connu et que le coût en données est nul aujourd'hui. |
| Ré-encoder les 8,58 M passages de production sous transformers 4.57.6, pour rejoindre l'ONNX existant | Les vecteurs de production sont **déjà** conformes à la référence (0,999996–0,999999 sous 5.14.1). Les refaire coûterait un passage GPU complet pour se rapprocher d'une version plus ancienne, et ferait de la production Legifacile la variable dépendante d'un plafond de paquet dans Crible. |
| Garder `optimum` et patcher sa borne de version à la main | Une borne forcée laisserait `optimum-onnx` s'installer contre une version qu'il déclare incompatible : l'échec serait différé à l'exécution, hors de tout contrôle, et la CI ne verrait rien. Un ADR ne peut pas promettre qu'un contournement de dépendance reste correct. |

## Références

- `docs/04-donnees-et-ingestion.md` : « Le modèle » (`model.json`, empreinte, `max_tokens`), « Contrôles et recette, dans l'ordre » (contrôle 3, contrôle 9, `acceptance_min_cosine: 0.999`) et la phrase sur la recette de Legifacile (cosinus 0,988).
- `docs/07-stack-et-deploiement.md`, section « Côté Python » (ligne à modifier).
- `docs/08-invariants-et-tests.md` : INV-11, INV-12 ; MES-04 (parité de l'encodeur).
- `docs/09-plan-de-developpement.md` : règles 3 à 5 bis, format d'une fiche de tâche.
- `docs/phases/P2-encodeur-et-recette.md` : fiches `EXPORTER-MODELE-ONNX`, `CONSTITUER-ECHANTILLON-LEGIFACILE`, `CHARGER-MODELE` (à modifier), `RECETTE-IMPORT`, `MESURER-PARITE-ENCODEUR`.
- `tools/export_onnx.py`, `tools/pyproject.toml` (groupe `export`), `tools/uv.lock`, `tools/README.md`.
- `data/legifacile-sample/README.md` (gitignoré), section « Limite connue : modèle ONNX et seuil de recette » : cosinus 0,999996–0,999999 sous 5.14.1 ; 0,983–0,990 avec prompt et 0,708–0,768 sans prompt pour l'ONNX 4.57.6, sur 10 passages par corpus.
- `legifacile-sandbox/docs/MDENSEON.md` : début (« L'import a eu lieu le 2026-08-06/07… », recette avant écriture) et §5 (« Ce que l'empreinte couvre — et ce qu'elle a laissé passer » : `transformers.__version__` dans l'empreinte, refus à 0,988, torch hors empreinte à 0,999976 minimum sur 500 passages).
- PyPI, `optimum-onnx` 0.1.0 (2025-12-23), `Requires-Dist: transformers<4.58.0,>=4.36` — vérifié le 2026-10-23.
- ADR-0028 (forme canonique de l'empreinte du modèle : champs hashés, format persistant) ; ADR-0021 (tracer les décisions par des ADR) ; ADR-0049 (liste des paquets du groupe `export`, remplacée ici) ; ADR-0034 (précédent de forme : dépendance Python nommée dans `docs/07`).
- Tâche `EXPORTER-MODELE-ONNX` (P2, `FAIT`, commit `1669ee5`) ; tâche `CONSTITUER-ECHANTILLON-LEGIFACILE` (P2, origine de la question).
