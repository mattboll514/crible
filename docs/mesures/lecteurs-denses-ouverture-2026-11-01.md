# Mesure — coût d'ouverture des lecteurs denses à 8,58 M passages

Rapport attaché à la tâche `REUTILISER-LECTEURS-DENSES` (phase P7, livrable 1). Ce n'est pas une mesure numérotée `MES-xx` de `docs/08-invariants-et-tests.md` : elle répond à une question posée après `REPONSE-DEBUG`, et son protocole tient en une commande.

## Campagne du 2026-11-01

- Tâche : `REUTILISER-LECTEURS-DENSES`
- Commit : `789abfaec9ea49c6f8de990355a0621bfaf1ea6e` (branche `tache/REUTILISER-LECTEURS-DENSES`)
- Machine : MacBook Air M3, modèle `Mac15,12`, CPU `Apple M3`, 25 769 803 776 octets de RAM (24 Gio), macOS 15.7.7 (24G720), **natif** (pas de Docker), `cargo build --release`.
- Charge tierce : moyennes de charge `3,28 / 3,83 / 4,42` avant la campagne (1, 5 et 15 minutes) et `1,95 / 2,21 / 2,88` après. La machine n'était pas au repos complet, et aucune autre campagne n'a tourné pendant les mesures.
- Espace disque avant la campagne : 78 GiB libres. Rien de volumineux n'a été écrit hors de `data/`.

### Jeu de données

Point principal, celui sur lequel porte la décision : index **`synth-large`**, `data/synth-large/indexes/synth-large`.

| Élément | Valeur |
|---|---|
| Passages déclarés dans le segment | 8 600 000 |
| Passages vivants au moment de la mesure | 7 740 000 |
| Dimensions, précision | 768, fp16 |
| Segments | 1 (`seg-large-0001`, `iid_start` 0) |
| Taille du fichier de vecteurs | 13 209 600 000 octets |
| Somme de contrôle déclarée | `blake3:9087fe59d7036cde0bf26fa9f8498deaca699efb0549ff64f17ddcd8d160711a` |

Points secondaires, pour montrer la croissance avec le corpus :

| Index | Passages vivants | Octets de vecteurs | Segments | Somme de contrôle (`vectors.f16`) |
|---|---|---|---|---|
| `articles` (échantillon Legifacile) | 50 000 | 76 800 000 | 1 | `blake3:e76f45d96183a5fab52bc5f15c2451ec621061de143124499cda7a25d5f0abf4` |
| `synth-small` | 9 750 | 14 976 000 | 1 | — |

La somme de contrôle est celle que `segment.json` déclare, c'est-à-dire l'empreinte que `SegmentReader::open` recalcule à chaque ouverture (`crates/crible-store/src/segment.rs:983-1007`).

### Protocole

Harnais : `crates/crible/examples/dense_latency.rs`, mode `open` ajouté pour cette mesure. Il appelle la fonction publique `DenseSearcher::open(&segments_dir, &segments, &records_path)`, puis détruit le searcher avant l'ouverture suivante. Aucun code de `crates/*/src/` n'est modifié.

Commandes exactes (depuis la racine du dépôt, après `cargo build --release --example dense_latency -p crible`) :

```
./target/release/examples/dense_latency open --data data/synth-large --index synth-large \
  --out data/mes-lecteurs-denses/open-synth-large-2026-11-01.json --opens 200
./target/release/examples/dense_latency open --data data --index articles \
  --out data/mes-lecteurs-denses/open-articles-2026-11-01.json --opens 200
./target/release/examples/dense_latency open --data data --index synth-small \
  --out data/mes-lecteurs-denses/open-synth-small-2026-11-01.json --opens 200
```

Elles sont aussi dans `tools/bench/ouvertures-denses/run.sh`.

- **Préchauffage** : une ouverture par point, chronométrée mais **rapportée à part** (`first_ms`), puis **200 ouvertures mesurées** (`n = 200`).
- **Cache** : série **cache chaud**. Les fichiers sont dans le cache de pages après le préchauffage ; chaque ouverture les relit quand même en entier pour la somme de contrôle. Aucune série `F_NOCACHE` ni copie non cachée n'est produite : sur macOS, le froid contrôlé est renvoyé à P8 par ADR-0084, et le sceau blake3 de `SegmentReader::open` réchauffe le fichier avant toute requête.
- **Percentiles** : rang le plus proche, par la fonction `percentile` du harnais, la même que MES-02 et MES-03.
- **Durée** : 1 683 s au total pour le point `synth-large` (28 min), soit environ 8,27 s par ouverture.

### Résultats

| Point | Ouvertures | Première ouverture | p50 | p95 | p99 | max |
|---|---|---|---|---|---|---|
| `synth-large` (7,74 M vivants, 13,2 Go) | 200 | 8 252,6 ms | **8 262,9 ms** | **8 406,9 ms** | 8 531,4 ms | 8 580,9 ms |
| `articles` (50 000 vivants, 76,8 Mo) | 200 | 46,3 ms | 43,8 ms | 44,9 ms | 45,9 ms | 46,5 ms |
| `synth-small` (9 750 vivants, 15,0 Mo) | 200 | 8,4 ms | 7,9 ms | 8,2 ms | 8,2 ms | 8,5 ms |

Le minimum de la série `synth-large` vaut 8 115,1 ms et sa moyenne 8 266,9 ms : la dispersion est faible (environ 5 % entre le minimum et le maximum), et la première ouverture n'est pas plus lente que la médiane.

Croissance avec le corpus, à partir des p50 : 14 976 000 octets en 7,9 ms (1,89 Go/s), 76 800 000 octets en 43,8 ms (1,75 Go/s), 13 209 600 000 octets en 8 262,9 ms (1,60 Go/s). Le temps suit le nombre d'octets du fichier de vecteurs, à débit presque constant. Cela désigne la relecture intégrale du fichier de vecteurs pour la somme de contrôle (`check_checksum`, `crates/crible-store/src/segment.rs:983-1007`). Ce n'est pas isolé par un test séparé : le prouver demanderait un chemin d'ouverture sans sceau, donc un changement de code, interdit dans cette session.

Sorties brutes (JSON avec chaque durée, et journal du harnais) :

- `data/mes-lecteurs-denses/open-synth-large-2026-11-01.json` et `.log` ;
- `data/mes-lecteurs-denses/open-articles-2026-11-01.json` et `.log` ;
- `data/mes-lecteurs-denses/open-synth-small-2026-11-01.json` et `.log` ;
- `data/mes-lecteurs-denses/uptime-before.txt` et `uptime-after.txt`.

### Lecture

À 8,58 M passages, le **p95 de l'ouverture des lecteurs denses vaut 8 406,9 ms**, et le p50 8 262,9 ms. Le p95 est donc **environ 8 400 fois le seuil de 1 ms** de la règle de décision de la fiche. La branche « négligeable (p95 sous 1 ms) » n'est pas la situation mesurée ; la fiche prévoit alors le cache par génération. La position par rapport au seuil est constatée ici, et la suite de la décision appartient au propriétaire.

Pour situer : `REPONSE-DEBUG` avait relevé 0,6 ms à 4 000 passages et 2,3 ms à 20 000, chiffres qui croissaient déjà avec le corpus. La série ci-dessus prolonge cette croissance et la chiffre au-delà de l'échantillon.

### Réserves

- **Machine de développement**, native, avec une charge tierce de 3 à 4 au début de la campagne. Les chiffres ne valent pas pour le conteneur de P8, et aucune comparaison à une cible de latence de service n'en est tirée.
- **Un seul segment** dans les trois index. Le coût d'un index multi-segments n'est pas mesuré : l'ouverture additionnerait la relecture de chaque fichier de vecteurs et ouvrirait la même réserve de connexions.
- **Froid vrai non mesuré** (ADR-0084), et la première ouverture ne s'en distingue pas, parce que la relecture du fichier pour le sceau domine dans les deux cas.
- **Le partage interne n'est pas mesuré** : la série comprend la vérification du sceau, la création des `mmap`, l'ouverture de la réserve SQLite et sa requête de vérification. Le poids relatif de ces postes n'est pas isolé.
- **Vecteurs synthétiques** : le coût dépend des octets, pas du contenu, donc la nature aléatoire des vecteurs ne devrait pas le changer ; ce n'est pas vérifié sur `legifacile-full`.
- **Aucune extrapolation** : les trois points sont mesurés, y compris celui de 8,58 M passages.
