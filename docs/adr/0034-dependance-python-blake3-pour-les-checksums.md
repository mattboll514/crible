# ADR-0034 — Dépendance Python `blake3` pour les checksums des lots

- **Statut** : Proposé
- **Date** : 2026-10-21
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, sur demande de l'orchestrateur, tâche ECRIRE-LOTS-PYTHON (phase P1), après le verdict NON-COUVERT de l'agent architecte
- **Remplace** : —

## Contexte

La tâche `ECRIRE-LOTS-PYTHON` (phase P1) est bloquée sur une `QUESTION-CONCEPTION` : le producteur Python doit calculer un vrai digest **blake3** des fichiers Parquet du lot, et `docs/07-stack-et-deploiement.md` ne l'y autorise pas.

### Ce que le format exige

`docs/format-lot-v1.md`, section « Fichiers et checksums » :

> `files` associe chaque fichier Parquet à un checksum de la forme `blake3:<hex>`, où `<hex>` est le digest blake3 des octets du fichier en hexadécimal minuscule.

La vérification des valeurs a lieu **côté moteur Rust, à l'attache**, dès P1 (`ATTACHER-LOT`) : le contrôle 4, défini dans `docs/04-donnees-et-ingestion.md` et dans `docs/format-lot-v1.md` (section « Contrôles à l'attache »), compare le digest déclaré aux octets du fichier. Le producteur ne peut donc pas se contenter d'un checksum symbolique ni d'un autre algorithme : il lui faut blake3 pour **écrire** un manifeste dont les digests passeront ce contrôle.

### Ce que la conception autorise

`docs/07-stack-et-deploiement.md` liste, côté Python, exactement :

> `uv`, `pyarrow`, `numpy`, `httpx`, `pytest`

`blake3` n'y figure que dans le tableau Rust, sur la ligne « Hash », pour les empreintes, les checksums et `passage_hash`.

La bibliothèque standard Python ne fournit pas blake3 dans `hashlib`. Il n'existe donc aucun moyen de calculer ce digest avec les dépendances autorisées aujourd'hui.

### Ce que les règles imposent

- `docs/09-plan-de-developpement.md`, règle 5 bis : toute dépendance absente de `docs/07` demande un ADR.
- `docs/adr/README.md` : un ADR est requis pour toute décision qui « ajoute, retire ou remplace une dépendance ».

C'est donc un **écart à la conception**, pas un détail d'implémentation.

### Indice dans une décision antérieure

L'ADR-0028 (forme canonique de l'empreinte du modèle) prévoit que le producteur Python **puisse** reproduire l'empreinte s'il la calcule lui-même. Cette empreinte contient des hash, ce qui demanderait alors aussi blake3 en Python. L'ADR-0028 ne le décide pas et ne fixe pas le périmètre de cette dépendance. Le test croisé de l'empreinte, lui, existe depuis P0 dans `crates/crible-core/tests/model_config.rs` : il appelle `python3` pour produire le JSON canonique et laisse le hash côté Rust ; il n'a pas besoin de blake3 en Python.

### Question tranchée ici

Le périmètre de la dépendance : limitée au paquet `clients/python`, ou étendue à tout autre projet `uv` du dépôt ?

## Décision

Ajouter **`blake3`** (paquet PyPI) aux dépendances Python du projet dans `docs/07-stack-et-deploiement.md`, sur la ligne « Côté Python ».

La dépendance est déclarée comme **dépendance Python du projet** dans `docs/07-stack-et-deploiement.md`. Le premier utilisateur est le paquet `clients/python` (les producteurs, ADR-0013), qui la porte dans `clients/python/pyproject.toml`. Tout autre projet `uv` du dépôt ne l'ajoute qu'au besoin, et aucun projet ne la reçoit par défaut.

La dépendance est utilisée pour une seule chose : calculer les digests `blake3:<hex>` du format de lot. Elle ne remplace pas la crate Rust `blake3`, qui reste la référence côté serveur.

Ce qui n'est **pas** décidé ici :

- aucune autre dépendance Python n'est ajoutée ;
- la version minimale du paquet, le caractère optionnel de la dépendance et la politique de mise à jour ne sont pas fixés ; ils relèvent du fichier `pyproject.toml` au moment de l'implémentation ;
- `docs/07` n'est pas modifié tant que le propriétaire n'a pas accepté cet ADR.

## Conséquences

- **Positives** :
  - le producteur calcule un digest identique à celui de la crate Rust, puisque la wheel officielle est compilée depuis l'implémentation Rust de référence ;
  - le contrôle 4, exécuté par le moteur Rust à l'attache, peut valider des digests réels : le manifeste écrit par le producteur passe le contrôle sans contournement ;
  - le producteur peut, s'il le souhaite, reproduire l'empreinte du modèle en Python (ADR-0028), ce qui demande aussi blake3 ;
  - le débit reste compatible avec des fichiers Parquet de plusieurs centaines de Mo, contrairement à une implémentation en Python pur.
- **Négatives / coûts** :
  - une dépendance de plus à installer, à versionner et à auditer ; elle est compilée (extension native) et non purement Python ;
  - la disponibilité des wheels doit être vérifiée pour les deux cibles du projet : **macOS arm64** (poste de développement) et **Linux x86_64** (CI). Si une wheel manque, l'installation compile depuis les sources, avec un compilateur Rust requis ; c'est un point à constater, pas une hypothèse ;
  - deux projets `uv` peuvent diverger de version si la contrainte n'est pas tenue de façon cohérente.
- **À surveiller** : la présence effective des wheels pour macOS arm64 et Linux x86_64 au moment de l'implémentation ; toute divergence de digest entre Python et Rust, qui signalerait un problème de version ou d'usage de l'API.

En cas d'acceptation :

- `docs/07-stack-et-deploiement.md` ajoute `blake3` à la ligne Python, dans un commit qui cite `ADR-0034` ;
- `ECRIRE-LOTS-PYTHON` reprend, avec la dépendance déclarée dans `clients/python/pyproject.toml` ;
- tout autre projet `uv` du dépôt ne l'ajoute que s'il en a besoin.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Implémentation de blake3 en Python pur | Aucune dépendance, mais lente sur des fichiers Parquet de plusieurs centaines de Mo et risque de diverger de la crate Rust sur les cas limites ; le format exige un digest identique, pas seulement un digest plausible. |
| Appel à l'outil `b3sum` par sous-processus | Outil externe non géré par `uv`, absent des environnements de développement et de la CI ; un échec silencieux ou une version différente produirait des checksums faux sans que le format le détecte avant le contrôle 4. |
| Faire calculer les checksums par Rust à l'attache | Contredit le format : c'est le producteur qui écrit `manifest.json`, et le contrôle 4 compare le digest déclaré aux octets. Déplacer le calcul côté Rust viderait le contrôle de son sens et changerait le contrat du format de lot. |

## Références

`docs/format-lot-v1.md` (section « Fichiers et checksums ») ; `docs/07-stack-et-deploiement.md` (dépendances Python et Rust) ; `docs/09-plan-de-developpement.md` (règle 5 bis) ; `docs/adr/README.md` (quand écrire un ADR) ; ADR-0012 (format de lot public) ; ADR-0013 (producteurs Python, serveur Rust) ; ADR-0028 (forme canonique de l'empreinte du modèle) ; fiche `ECRIRE-LOTS-PYTHON` (phase P1).
