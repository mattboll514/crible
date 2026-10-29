# ADR-0069 — Reconstruction et format de `codes.bin`

- **Statut** : Accepté (accepté par anticipation le 2026-10-26, confirmé par le propriétaire le 2026-10-26)
- **Date** : 2026-10-26 (proposition et acceptation anticipée)
- **Décideurs** : propriétaire
- **Décision** : accepté par anticipation le 2026-10-26, par délégation du propriétaire (« considère qu'il est auto-accepté, on regarde ensemble à la fin des 3 tâches »), confirmé par le propriétaire le 2026-10-26. Les six décisions sont retenues telles que proposées : reconstruction automatique à l'ouverture en écriture, construction en RAM seule à l'ouverture en lecture seule, absence de fichier qui n'est pas une erreur mais longueur fausse qui en est une, exclusion maintenue de l'instantané et de l'export, aucune sous-commande CLI en P4, format sans en-tête versionné par `format_version` de `segment.json`.
- **Proposé par** : agent greffier, sur demande de l'orchestrateur, pour la tâche `QUANTIFIER-BINAIRE` (phase P4), à la suite d'une `QUESTION-CONCEPTION` remontée par `@testeur` avant `@dev` et des verdicts de `@architecte` du 2026-10-26
- **Remplace** : —

## Erratum du 2026-10-29

La citation de `docs/07-stack-et-deploiement.md:15`, dans la section « Contexte », décrit l'inventaire des sous-commandes à la date de cet ADR. Depuis la tâche `NETTOYER-P5` (commit `7ca3e63`), cette ligne compte aussi `rebuild-lexical`, juste après `compact` : la commande `crible rebuild-lexical` est exigée par la fiche `RECONSTRUIRE-LEXICAL` (`docs/phases/P5-lexical-et-hybride.md:53`) et livrée en P5. Cette citation est laissée telle quelle, comme un état daté. Rien ne change dans les décisions de cet ADR : aucune sous-commande n'est ajoutée pour `codes.bin`, et la décision 5 reste en vigueur. ADR-0087 range « une sous-commande de réparation explicite » parmi ce qu'il ne décide pas. La note est demandée par le propriétaire le 2026-10-29, après un constat de `@relecteur` à la fin de `NETTOYER-P5`.

## Contexte

### Ce que la fiche demande

La fiche `QUANTIFIER-BINAIRE` (`docs/phases/P4-recherche-rapide.md:14-26`) demande trois livrables : l'écriture de `codes.bin` — « bit i = x_i > 0, mots `u64`, little-endian » — à l'attache et à la compaction ; « un job de reconstruction des codes depuis les vecteurs de référence, pour les index existants » ; le chargement des codes en RAM, hors mmap, dans le `Snapshot`. Elle cite **INV-15** et demande que les codes reconstruits soient identiques à ceux écrits à l'attache.

La fiche ne dit ni **quand** ce job se lance, ni **qui** le déclenche, ni ce que devient un index dont le fichier manque. La question a été remontée par `@testeur` avant l'implémentation, parce qu'elle décide de la forme des tests autant que du code.

### Ce que la conception décrit

- `docs/02-architecture.md:72` : « `codes.bin` — n × d/8 octets, codes binaires (dérivé) ».
- `docs/02:80` : « Tout ce qui est marqué « dérivé » se reconstruit à partir de `records.sqlite` et des fichiers `vectors.*`. »
- `docs/02:84` : précédent du format sans en-tête. `CURRENT_EPOCH` est « un nombre décimal ASCII suivi d'un saut de ligne, sans version propre » ; sa version vient de `meta.format_version` de `records.sqlite` et des `format_version` des fichiers de l'époque.
- `docs/02:133-145` : la reprise supprime les résidus, reconstruit le manifeste s'il manque, construit les bitmaps ou charge le cache estampillé G. Elle ne nomme pas `codes.bin` avant cet ADR.
- `docs/03-recherche.md:86` : le lexical se reconstruit depuis le texte SQLite « dans trois cas : compaction, reprise après incident, changement d'analyseur ».
- **INV-04** (`docs/08-invariants-et-tests.md:12`) : « Au redémarrage, l'état est la dernière génération commitée ; tout le reste est reconstruit ou nettoyé ».
- **INV-15** (`docs/08:23`) : tout format sur disque est little-endian explicite et versionné.
- `docs/04-donnees-et-ingestion.md:321` et **ADR-0064**, décision 4 (`:111`, `:113`) : `codes.bin` est exclu de l'instantané, comme dérivé reconstructible. `docs/04:300` et **ADR-0061** : l'export est un lot de données vivantes et ne porte pas `codes.bin`.
- `docs/05-api.md:15` : les mutations asynchrones rendent un `job`. Cette notion n'existe pas en P4 : le serveur et l'API arrivent en P6.
- `docs/07-stack-et-deploiement.md:15` : la liste des sous-commandes du binaire est fixée — `serve, index, attach, ingest, patch, search, compact, export, snapshot, restore, inspect, eval, stats, key`.

### Les faits qui pèsent

- **Aucun index restauré ou importé n'a de `codes.bin`.** L'instantané et l'export l'excluent tous deux : la reconstruction n'est pas un cas de panne rare, c'est le cas normal de toute restauration et de tout import.
- **Le coût de la reconstruction est connu en ordre de grandeur.** Les vecteurs de référence fp16 pèsent environ 13,2 Go à l'échelle de Legifacile (8,58 M passages × 768, `docs/02:213`) ; les codes, eux, valent 0,82 Go en RAM (`docs/02:208`). Reconstruire les codes relit donc tous les vecteurs, une fois.
- **ADR-0047** a fixé l'ordre des remèdes au temps de démarrage : mesurer avant d'optimiser, et n'ajouter un format sur disque que si la cible de 60 s reste dépassée. Un coût de démarrage supplémentaire, même assumé, s'inscrit dans cette règle.
- **Un segment porte `count`, `dims` et `format_version`** dans son `segment.json` (`docs/02:70`), et ses `checksums` ne couvrent aujourd'hui que `vectors.f16` ou `vectors.f32` (`crates/crible-store/src/segment.rs:12`, `:305`).

### Les quatre points que rien ne tranche

`@architecte` a rendu, le 2026-10-26, les verdicts suivants.

| Point | Sujet | Verdict |
|---|---|---|
| A1 | Un en-tête propre à `codes.bin` | **CONFORME** sans en-tête : `docs/02:84` et le précédent de `CURRENT_EPOCH` font porter la version par le fichier voisin, ici `format_version` de `segment.json` |
| A2 | Une taille arrondie au mot `u64` | **CONFORME** pour `dims` multiple de 64 ; **NON-COUVERT** au-delà |
| A3 | `codes.bin` absent à l'ouverture | **ÉCART** : rendu en erreur `Internal`, ce qui contredit `docs/02:80` et INV-04 |
| A4 | Le déclencheur de la reconstruction | **NON-COUVERT** |

Le point A2 n'est pas théorique : `dims` est aujourd'hui 768, multiple de 64, mais rien dans la conception ne l'impose. Une formule qui ne vaut que pour 768 laisserait un piège au premier modèle de 384 ou 1 024 dimensions.

### Ce qui n'est pas décidé ici

Cet ADR ne modifie ni le contenu de l'instantané et de l'export (ADR-0061, ADR-0064, qui restent en vigueur), ni la liste des sous-commandes (`docs/07`), ni le chargement en RAM ou la structure du `Snapshot`, ni le calcul des bitmaps. Il ne fixe **aucun chiffre de durée** pour la reconstruction : le coût est à mesurer.

## Décision

**Les six décisions ci-dessous sont acceptées telles quelles.** Elles restent séparables : la 1 et la 2 règlent le déclencheur (A4), la 3 la conséquence d'un fichier absent (A3), la 6 le format (A1, A2).

### 1. `codes.bin` absent à l'ouverture en écriture est reconstruit, sans nouvelle génération

À l'ouverture d'un segment par **l'écrivain** de l'index, si `codes.bin` est absent, Crible le reconstruit depuis `vectors.f16` ou `vectors.f32` du même segment, puis le persiste **atomiquement** : fichier temporaire dans le dossier du segment, fsync, renommage, fsync du dossier.

La reconstruction **ne crée ni génération ni époque**. `codes.bin` est un fichier dérivé, invisible : il n'apparaît ni dans `meta.generation`, ni dans le manifeste, ni dans `CURRENT_EPOCH`, et il ne change aucun résultat de recherche, puisqu'il est la forme quantifiée des mêmes vecteurs.

C'est le « job de reconstruction » de la fiche. Ce n'est pas un job d'API : c'est une étape de l'ouverture, synchrone, sur le chemin de la reprise.

### 2. À l'ouverture en lecture seule, les codes manquants sont construits en RAM

Quand le segment est ouvert **en lecture seule** — une autre instance, un outil, la restauration, une recherche pendant que l'écrivain est suspendu — les codes manquants sont construits en mémoire et **jamais écrits dans l'époque**. Le calcul est le même, la persistance n'a pas lieu.

**Pourquoi.** Une ouverture en lecture ne doit rien écrire sous `epochs/` : c'est la règle qui rend une lecture sûre à côté d'un écrivain (INV-02), et elle vaut pour un fichier dérivé comme pour les autres. Un lecteur qui écrirait `codes.bin` modifierait l'époque sans passer par le point de commit (ADR-0016).

### 3. Un fichier absent n'est pas une erreur ; une longueur fausse en est une

- `codes.bin` **absent** n'est pas une erreur : il se reconstruit (décision 1 ou 2). C'est ce qu'exigent `docs/02:80` et INV-04.
- `codes.bin` de **longueur fausse** — tronqué, ou plus long que la taille de la décision 6 — est une erreur `Internal` qui nomme le fichier et le segment : c'est une corruption, pas un dérivé manquant. On ne réécrit pas par-dessus un fichier qu'on n'a pas su lire, et on ne fait pas semblant de le reconstruire.

### 4. L'instantané et l'export continuent d'exclure `codes.bin`

La décision 4 d'ADR-0064 et `docs/04:321` restent en vigueur sans changement, de même que le manifeste d'export (ADR-0061, `docs/04:300`).

**Conséquence assumée.** Toute restauration et tout import reconstruisent les codes au premier démarrage en écriture, ce qui relit **tous** les vecteurs du segment : environ 13,2 Go à l'échelle de Legifacile. C'est un coût de démarrage, pas un coût de requête, et il est cohérent avec ADR-0047 : on mesure avant d'optimiser. Aucune optimisation n'est décidée ici, ni un cache, ni un format qui voyagerait dans l'instantané.

### 5. Aucune sous-commande CLI en P4

La liste de `docs/07:15` reste inchangée : pas de `crible codes rebuild` ni d'équivalent. Le déclencheur est l'ouverture (décision 1), pas une commande.

**Pourquoi.** La conception ne nomme aucune sous-commande pour cette opération, et `docs/07:15` fixe la liste : en ajouter une serait une surface publique nouvelle, à décider, pour un geste qu'aucune situation ne rend nécessaire — un index dont les codes manquent les reconstruit tout seul.

### 6. Format : octets nus, taille arrondie au mot `u64`, version portée par `segment.json`

- **Pas d'en-tête.** `codes.bin` est une suite d'octets nus, comme `vectors.*`. Sa version est `format_version` de `segment.json` du même segment : c'est le précédent de `CURRENT_EPOCH` (`docs/02:84`), et c'est ce qui satisfait INV-15 sans inventer un second numéro de version à synchroniser.
- **Toute modification de la disposition de `codes.bin` incrémente `format_version` de `segment.json`.** Une version inconnue est refusée, comme pour tout fichier de segment.
- **Taille** : `count × ceil(dims/64) × 8` octets. `count` et `dims` viennent de `segment.json`.
- **Code d'un passage** : `ceil(dims/64)` mots `u64` little-endian. Le bit `i` vaut `x_i > 0`. Les bits de bourrage au-dessus de `dims` valent zéro.
- **Cas de `dims` = 768** : `ceil(768/64) = 12` mots, soit **96 octets**, c'est-à-dire le `d/8` de `docs/02:72`. La formule de cet ADR généralise cette écriture ; elle ne la contredit pas.
- **`codes.bin` n'entre pas dans les `checksums` de `segment.json`.** Ces checksums couvrent les fichiers dont l'absence ou l'atteinte est une panne ; `codes.bin` est dérivé et se reconstruit. Un checksum en ferait un fichier obligatoire, en contradiction avec la décision 3.

## Conséquences

- **Positives** :
  - INV-04 est tenu pour `codes.bin` : après un redémarrage, un arrêt brutal ou une restauration, l'index repart sans intervention, et sans état mixte ;
  - `docs/02:80` devient vrai pour tous les fichiers marqués « dérivé », pas seulement pour le manifeste et le lexical ;
  - la restauration et l'import restent utilisables tels quels, sans nouvelle commande ni étape d'exploitation ;
  - le format vaut pour tout `dims`, pas seulement pour les multiples de 64 ;
  - un seul numéro de version par segment, et aucune synchronisation à maintenir entre deux fichiers.
- **Négatives / coûts** :
  - le premier démarrage en écriture après une restauration ou un import relit tous les vecteurs, soit environ 13,2 Go à l'échelle de Legifacile : le démarrage est plus lent, une fois par segment ;
  - l'écrivain écrit un fichier de plus au premier usage d'un segment importé, ce qui ajoute un cas à la reprise et à ses tests de panne (INV-04) ;
  - un `codes.bin` tronqué n'est pas réparable automatiquement : c'est une erreur `Internal` qui demande une intervention, là où l'absence est transparente ;
  - le format reste implicite : un lecteur qui ouvrirait `codes.bin` sans `segment.json` ne connaîtrait ni `count`, ni `dims`, ni la version.
- **À surveiller** :
  - le temps de reconstruction des codes au démarrage, à relever à la première restauration et au premier import sur l'échantillon Legifacile, puis à comparer à la cible de 60 s d'ADR-0047 ; s'il domine, la réponse est un ADR, pas un contournement ;
  - tout futur fichier dérivé doit suivre la même règle : reconstruit à l'ouverture en écriture, en RAM seulement en lecture, absent n'est pas une erreur, longueur fausse en est une ;
  - un modèle de dimensions non multiples de 64, qui est le cas que la conception n'avait pas couvert : il est désormais couvert par la formule, et un test doit le vérifier.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| (i) Une sous-commande `crible codes rebuild` explicite, avec refus à l'ouverture si `codes.bin` manque | Surface CLI nouvelle, absente de `docs/07:15`, pour un geste que l'ouverture peut faire seule. Et un refus à l'ouverture laisse l'index **muet** après toute restauration ou tout import, jusqu'à une intervention manuelle : c'est exactement l'écart A3 que cet ADR corrige, déplacé d'un cran |
| (ii) Un en-tête propre à `codes.bin`, avec son numéro de version | `docs/02:72` décrit le fichier comme `n × d/8` octets, et `docs/02:84` a déjà tranché la question pour `CURRENT_EPOCH` : la version vient du fichier voisin. Un second numéro de version devrait être tenu cohérent avec `format_version` de `segment.json` sans que rien ne l'y oblige |
| Reconstruction à chaque ouverture, sans persistance | Relit tous les vecteurs à chaque démarrage, y compris quand rien n'a changé : c'est le coût de la décision 4 payé indéfiniment, et non une fois |
| Refuser l'index et demander une reconstruction manuelle | Contredit `docs/02:80` et INV-04, et fait de tout import un index hors service |
| Inscrire `codes.bin` dans les `checksums` de `segment.json` | En ferait un fichier obligatoire : le fichier absent deviendrait une erreur, ce que la décision 3 refuse |
| Inclure `codes.bin` dans l'instantané et l'export | Rouvre ADR-0064 et ADR-0061 pour un fichier dérivé : des octets de plus à copier, à hasher et à invalider, sans rien rendre possible |

## Documents modifiés après acceptation

Appliqué dans le même travail que cet ADR, sur la branche `tache/QUANTIFIER-BINAIRE`, dans un commit qui cite `ADR-0069` :

| Document | Modification |
|---|---|
| `docs/02-architecture.md:72` | remplacer « n × d/8 octets » par la formule de la décision 6 : `count × ceil(dims/64) × 8` octets, version portée par `segment.json` |
| `docs/02-architecture.md:133-144` | ajouter `codes.bin` à la liste de la reprise, en renvoyant à ADR-0069 : reconstruit à l'ouverture en écriture s'il manque, construit en RAM en lecture seule |
| `docs/phases/P4-recherche-rapide.md` | ajouter à `QUANTIFIER-BINAIRE` une note « Précisé par ADR-0069 » sur le déclencheur de la reconstruction, le format et la taille |
| `docs/adr/README.md` | ajouter la ligne d'index de cet ADR |

`docs/suivi.md` n'est pas touché par cet ADR. Aucun autre document de conception n'est modifié.

## Références

- Fiche `QUANTIFIER-BINAIRE` : `docs/phases/P4-recherche-rapide.md:14-26`
- `docs/02-architecture.md` : organisation sur disque (`:55-80`), `CURRENT_EPOCH` et `manifests/<generation>.json` (`:84-85`), reprise au démarrage (`:133-145`), mémoire et tailles (`:202-219`)
- `docs/03-recherche.md:86` : reconstruction du lexical
- `docs/04-donnees-et-ingestion.md` : export, instantanés, restauration (`:296-340`), exclusions (`:321`)
- `docs/05-api.md:15-16` : mutations asynchrones et objet `job`
- `docs/07-stack-et-deploiement.md:15` : sous-commandes du binaire `crible`
- `docs/08-invariants-et-tests.md` : INV-02 (`:10`), INV-04 (`:12`), INV-15 (`:23`)
- `crates/crible-store/src/segment.rs:12`, `:305` : `checksums` de `segment.json`
- ADR-0004 (codes binaires et scan du pool), ADR-0016 (point de commit SQLite), ADR-0047 (phase de mesures et ordre des remèdes au démarrage), ADR-0061 (manifeste d'un lot exporté), ADR-0064 (format et contenu d'un instantané, décision 4)
