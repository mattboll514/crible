# ADR-0036 — `producer.host` réglable par le producteur

- **Statut** : Accepté
- **Date** : 2026-10-21 (proposition et acceptation)
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, sur demande de l'orchestrateur, tâche GENERER-JEU-SYNTHETIQUE (phase P1), après le verdict ECART de l'agent architecte
- **Remplace** : —

## Contexte

### L'exigence de reproductibilité

La fiche `GENERER-JEU-SYNTHETIQUE` (phase P1) exige que « deux générations avec la même graine donnent des fichiers identiques au bit près ». `docs/08-invariants-et-tests.md` (l. 45) précise que la somme de contrôle de `synth-small` est **versionnée**, et `docs/mesures/README.md` (l. 19) impose qu'un rapport cite « Jeu de données : <nom, taille, somme de contrôle> ». La même somme doit donc désigner le même jeu, quelle que soit la machine qui l'a produit.

### Ce qui empêche de tenir l'exigence

Le manifeste de chaque lot porte `producer.host`. `clients/python/src/crible/batch.py` (l. 262) le remplit avec `socket.gethostname()`. Deux machines produisent donc des manifestes aux octets différents, et la somme de contrôle du jeu change avec la machine : elle n'est reproductible ni en CI ni chez un autre développeur.

Le format décrit ce membre sans le contraindre. `docs/format-lot-v1.md` (l. 79) le déclare obligatoire, de type chaîne, avec la description « Machine qui a produit le lot » ; `crates/crible-core/src/batch.rs` (l. 86-87) porte le commentaire `/// Machine that ran the producer.` Aucun contrôle de l'attache ne vérifie la valeur, et le format n'impose pas qu'elle corresponde au nom d'hôte réel de la machine.

### Le précédent de `created_at`

`created_at` a déjà été rendu réglable : `BatchWriter` accepte un paramètre facultatif, l'horloge reste le défaut, et `gen_synth.py` fixe une date dérivée de la graine seule. Ce précédent ne s'applique pas tel quel à `host` : une date fixe reste une date RFC 3339 et respecte la spec (`format-lot-v1.md`, l. 76), alors qu'une valeur comme `"gen_synth"` n'est pas une machine.

### Ce que les règles imposent

`docs/adr/README.md` : un ADR est requis pour tout changement de format, lot compris. Il s'agit donc d'un **écart à la conception**, pas d'un détail d'implémentation.

## Décision

`producer.host` devient **réglable par le producteur**, au moyen d'un paramètre facultatif de `BatchWriter`, dont la valeur par défaut reste `socket.gethostname()`.

La description du membre dans le format devient : « identifiant de la machine, ou du producteur, qui a produit le lot ». `producer.name` continue de nommer le programme ; `host` dit d'où le lot est sorti, machine physique ou producteur identifiable.

`gen_synth.py` fixe une valeur stable, afin que la somme de contrôle de `synth-small` soit la même partout. Le format reste en **version 1** : aucun contrôle ne change, et aucun lecteur ne dépend de la valeur de `host`.

Ce qui n'est **pas** décidé ici :

- la valeur exacte choisie par `gen_synth.py` relève de la tâche, pas de cet ADR ;
- `created_at` n'est pas rouvert : il est déjà réglable ;
- aucun autre membre du manifeste ne devient réglable.

## Conséquences

- **Positives** :
  - la somme de contrôle de `synth-small` devient reproductible : CI et postes de développement calculent la même ;
  - la CI peut vérifier que la somme produite est égale à celle consignée dans `tools/README.md` ;
  - le format gagne une description exacte de ce que `host` porte réellement, au lieu d'une promesse que rien ne contrôle ;
  - aucune nouvelle version de format, aucun changement de lecteur.
- **Négatives / coûts** :
  - la valeur par défaut reste le nom d'hôte : un producteur qui ne règle rien garde un manifeste non reproductible, ce qui est voulu mais doit être su ;
  - un manifeste dont `host` vaut un identifiant de producteur est moins utile au diagnostic qu'un nom de machine ;
  - la somme consignée aujourd'hui dans `tools/README.md` change une fois, puis devient définitive ;
  - tant que l'ADR n'est pas accepté, la somme consignée dans `tools/README.md` n'est pas définitive.
- **À surveiller** : toute différence de somme entre deux machines après application ; un consommateur qui se mettrait à interpréter `host` comme un nom d'hôte résolvable, ce que le format ne promet pas.

En cas d'acceptation :

- `docs/format-lot-v1.md` (l. 79) et le commentaire de `crates/crible-core/src/batch.rs` (l. 86-87) sont reformulés ;
- `BatchWriter` reçoit un paramètre facultatif `host` ;
- `gen_synth.py` fixe une valeur stable et la somme de `tools/README.md` est recalculée ;
- un test vérifie que la somme produite est égale à celle de `tools/README.md`, ce qui donne à la CI un contrôle réel.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| B — exclure `manifest.json` de la somme de contrôle | Le format ne change pas, mais la somme ne couvre plus le schéma, le modèle ni les comptes, et les octets du manifeste diffèrent toujours d'une machine à l'autre : le « bit près » ne vaudrait que localement. |
| C — garder le nom d'hôte et consigner une somme par machine | `docs/08` parle de « la somme », au singulier, et une somme propre à une machine ne sert à rien dans un rapport de mesure. |
| D — retirer `host` du format, dans une version 2 | Le coût d'une nouvelle version de format pour un gain nul, alors qu'aucun contrôle ne dépend de ce membre. |

## Références

`docs/08-invariants-et-tests.md` (jeux de données, l. 45) ; `docs/format-lot-v1.md` (l. 76 et l. 79) ; `docs/mesures/README.md` (l. 19) ; `crates/crible-core/src/batch.rs` (l. 86-87) ; `clients/python/src/crible/batch.py` (l. 262) ; `tools/README.md` (somme de contrôle) ; `tools/gen_synth.py` ; ADR-0012 (format de lot public) ; fiche `GENERER-JEU-SYNTHETIQUE` (phase P1).
