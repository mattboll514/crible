# ADR-0028 — Forme canonique de l'empreinte du modèle

- **Statut** : Accepté (décision du propriétaire le 2026-10-19)
- **Date** : 2026-10-19
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, tâche DEFINIR-CONFIG-INDEX
- **Remplace** : —

## Contexte

`docs/04-donnees-et-ingestion.md` définit l'**empreinte du modèle** comme le blake3 du JSON canonique de certains champs : `hf_id`, `revision`, `dims`, `pooling`, `normalize`, `prompts`, et des hash des fichiers `fp32` et `tokenizer`. Le fichier INT8 et `max_tokens` sont exclus, car ils ne changent pas les vecteurs de référence.

Le document ne dit pas comment ce JSON est construit : ordre des clés, imbrication des hash de fichiers, forme du hash final. Deux implémentations conformes au texte pourraient donc produire deux empreintes différentes pour le même modèle, et refuser à tort un lot compatible (`422 incompatible_model`).

L'empreinte est copiée dans `index.json` et compare les lots à l'index. Elle est donc un format persistant, pas un détail interne.

## Décision

L'empreinte est le **blake3 d'un JSON canonique** construit ainsi :

- encodage **UTF-8**, **JSON compact** (aucun espace, aucune indentation) ;
- **aucun échappement des caractères non ASCII** : `é` reste `é`, il n'est pas écrit `\u00e9` ; seuls les caractères que JSON impose d'échapper (`"`, `\`, et les caractères de contrôle) le sont ;
- **clés triées** par ordre lexicographique à chaque niveau ;
- les hash de fichiers sont regroupés dans un **objet `files` imbriqué**, avec seulement les entrées `fp32` et `tokenizer` :

```json
{
  "dims": 768,
  "files": {
    "fp32": "<hash blake3>",
    "tokenizer": "<hash blake3>"
  },
  "hf_id": "lightonai/mDenseOn",
  "normalize": true,
  "pooling": "cls",
  "prompts": {
    "document": "document: ",
    "query": "query: "
  },
  "revision": "<sha du commit HF>"
}
```

- chaque hash de fichier est écrit en **hexadécimal minuscule nu**, sans préfixe `blake3:` ;
- l'empreinte finale est elle aussi en **hexadécimal minuscule nu** ;
- **aucun objet enveloppe** : le JSON hashé est directement l'objet ci-dessus, sans clé `model`, `config` ou équivalent.

Côté Python, le JSON canonique est produit par exactement :

```python
json.dumps(obj, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
```

L'encodage du résultat en octets se fait en UTF-8. `ensure_ascii=True`, valeur par défaut, produirait des séquences `\u00e9` et une empreinte différente : c'est une erreur à éviter explicitement. Côté Rust, `serde_json` n'échappe pas les caractères non ASCII lors de la sérialisation, ce qui donne la même forme.

Ne sont pas inclus : le nom du modèle, `kind`, `max_tokens`, le fichier `int8`, les chemins de fichiers et tout autre champ absent de la liste de `docs/04`.

## Conséquences

- **Positives** :
  - deux implémentations honnêtes produisent la même empreinte ;
  - la forme est simple à reproduire dans un test qui fige une valeur connue ;
  - le JSON compact et trié se compare octet par octet ;
  - la règle sur les caractères non ASCII lève l'ambiguïté entre Rust et Python.
- **Négatives / coûts** :
  - toute évolution de la forme (ajout d'un champ, changement d'imbrication) rend les index existants incompatibles avec les nouveaux lots ;
  - la forme doit être reproduite par le producteur Python s'il calcule l'empreinte de son côté ;
  - le producteur Python doit passer `ensure_ascii=False` : la valeur par défaut de `json.dumps` est fausse pour cette forme.
- **Tests attendus** :
  - un test Rust fige une empreinte connue ;
  - un test croisé calcule le JSON canonique avec `json.dumps(obj, sort_keys=True, separators=(",", ":"), ensure_ascii=False)` et compare la même empreinte qu'une implémentation qui n'échappe pas les caractères non ASCII ;
  - un cas de test contient un caractère non ASCII (par exemple un prompt `« recherche : »`) et vérifie que les octets hashés contiennent cet UTF-8, sans séquence `\u`.
- **À surveiller** : un changement de forme ne se fait que par un nouvel ADR, avec une procédure de migration ou l'acceptation explicite d'une rupture (`incompatible_model`).

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Clés plates (`files.fp32`, `files.tokenizer`) | Moins lisible, et impose une convention d'échappement des points ; l'objet imbriqué correspond directement à la structure de `docs/04`. |
| RFC 8785 (JCS) | Norme complète et utile, mais elle impose des règles de nombre et de chaînes dont le besoin n'existe pas ici ; elle ajoute une dépendance et une surface de conformité pour un objet de configuration simple. |

## Références

`docs/04-donnees-et-ingestion.md` (le modèle et l'empreinte) ; `docs/02-architecture.md` (`index.json`) ; ADR-0027 ; fiche `DEFINIR-CONFIG-INDEX`.
