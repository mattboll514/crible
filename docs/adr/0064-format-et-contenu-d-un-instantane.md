# ADR-0064 — Format et contenu d'un instantané (`snapshot.json`)

- **Statut** : Accepté (décision du propriétaire le 2026-10-25 ; accepté par anticipation le 2026-10-25, confirmé par le propriétaire le 2026-10-26)
- **Date** : 2026-10-25 (proposition et acceptation)
- **Décideurs** : propriétaire
- **Décision** : accepté le 2026-10-25 par le propriétaire, tel que proposé, lors d'une acceptation anticipée de sa part, confirmée par le propriétaire au point de fin de l'enchaînement de P3, le 2026-10-26. Les quatre décisions sont retenues telles quelles : `snapshot.json` versionné (`format_version: 1`) qui atteste la copie (1), « le manifeste » = `manifests/<G>.json` et seul G copié (2), destination occupée refusée et écriture sous dossier temporaire (3), contenu copié et exclusions (4). `format_version` vaut 1 ; INV-15 est satisfait sans dépendance nouvelle, `blake3` étant déjà au stack. Les modifications à appliquer sont celles de la section « Documents à modifier après acceptation ».
- **Proposé par** : agent greffier, sur demande de l'orchestrateur, pour la tâche `CREER-ET-RESTAURER-INSTANTANE` (phase P3), à la suite d'une `QUESTION-CONCEPTION` de `@dev` avant implémentation et des verdicts de `@architecte` du 2026-10-25
- **Remplace** : —

## Contexte

### Ce que la fiche demande

La fiche `CREER-ET-RESTAURER-INSTANTANE` (`docs/phases/P3-cycle-de-vie.md:121-130`) demande `crible snapshot` et `crible restore`. Elle liste ce que la copie contient — « un `VACUUM INTO`, puis on copie les segments, `index.json`, le manifeste et le dossier lexical s'il existe » — et fixe un critère sur l'intégrité : « un instantané incomplet (fichier manquant) est refusé à la restauration ».

### Ce que la conception décrit

`docs/04-donnees-et-ingestion.md:298-304` présente l'instantané comme une « copie cohérente de l'époque : `VACUUM INTO` de SQLite, segments, tantivy, `index.json`, manifeste », écrite dans `exchange/snapshots/<index>-e<E>-g<G>/`, et restaurée par la CLI comme « nouvelle époque, puis `CURRENT_EPOCH` ».

`docs/02-architecture.md:55-85` décrit le contenu d'une époque : `records.sqlite` en WAL, `segments/<segment_id>/{segment.json,vectors.f16|f32,codes.bin}`, `lexical/`, `manifests/<generation>.json` et `cache/`. Deux fichiers y ont un format propre : `CURRENT_EPOCH`, écrit par fichier temporaire puis renommage, avec `meta.epoch` qui en garde une copie vérifiée à l'ouverture ; et `manifests/<generation>.json`, JSON canonique versionné, **dérivé de SQLite et jamais source de vérité** (ADR-0016), reconstruit par la reprise s'il manque (`docs/02:138`).

Trois faits bornent la question :

- `records.sqlite` tourne en **WAL** (`docs/02:68`, `crates/crible-store/src/records.rs:1137`). Copier le seul fichier principal, sans son `-wal`, perd des pages commitées ; copier les trois fichiers d'un coup n'en fait pas un ensemble cohérent vis-à-vis d'un écrivain.
- `manifests/<G>.json` porte `epoch`, `format_version`, `generation` et la liste des segments avec leurs checksums (`docs/02:85`). Il ne couvre ni `records.sqlite` ni `index.json`, et il se reconstruit : il ne peut donc pas attester une copie.
- `segment.json` porte un objet `checksums` qui ne couvre que les fichiers de son segment (`crates/crible-store/src/segment.rs:12`, `:117`), écrit sous la forme `blake3:<hex>`.

**INV-15** (« Tout format sur disque est little-endian explicite et versionné », `docs/08-invariants-et-tests.md:23`) demande un numéro de version pour tout format nouveau, et le refus d'une version inconnue. `blake3` est déjà une dépendance du workspace (`Cargo.toml:53`, `docs/07-stack-et-deploiement.md:43`), employée pour les checksums des segments : aucun ajout n'est nécessaire.

### Les quatre points que rien ne tranche

`@architecte` a rendu, le 2026-10-25, les verdicts suivants.

| Point | Sujet | Verdict |
|---|---|---|
| B1 | Ce qui prouve qu'un instantané est complet et intact | **NON-COUVERT** |
| B2 | Ce que la fiche appelle « le manifeste » | **CONFORME** |
| C1 | Un dossier de destination déjà présent | non tranché |
| C2 | Ce qui est copié et ce qui est exclu | **CONFORME** |

### Les précédents

L'export (ADR-0061) a fixé deux formes que cet ADR reprend : un dossier de destination déjà présent est refusé en `invalid_request`, jamais écrasé (`docs/04:312`), et la copie s'écrit sous un dossier temporaire puis se renomme (`docs/04:313`). Une panne y laisse un résidu `.tmp-*` que ni la reprise ni une commande ne supprime, parce que `exchange/` est hors de l'index (`docs/02:132-133`).

`snapshot.json` n'est **pas** un manifeste au sens d'ADR-0016 : il atteste une copie de fichiers, il ne décrit pas une génération. Le manifeste de génération reste dérivé de SQLite et fait autorité par SQLite seul.

## Décisions

**Les quatre décisions ci-dessous sont acceptées telles quelles : `snapshot.json` versionné qui atteste la copie, « le manifeste » égal à `manifests/<G>.json`, destination occupée refusée avec écriture sous dossier temporaire, contenu copié et exclusions telles que listées.**

Les quatre points étaient **séparables** : en rejeter un laissait les trois autres tenir, et la tâche reprenait sur le périmètre accepté. Chacun porte la recommandation de `@architecte`.

### 1. Un `snapshot.json` versionné qui atteste la copie (B1)

**Proposition.** Un instantané porte, à sa racine, un fichier `snapshot.json` versionné (`format_version: 1`), écrit **après** la copie, par fichier temporaire, fsync, renommage puis fsync du dossier. Il contient :

- le nom de l'index, l'époque source E et la génération G ;
- pour **chaque fichier copié** — `snapshot.json` excepté, qui ne peut pas se hasher lui-même — son chemin relatif, sa taille en octets et son blake3 sous la forme `blake3:<hex>`, la liste étant triée par chemin.

```json
{
  "format_version": 1,
  "index": "legifacile",
  "epoch": 3,
  "generation": 412,
  "files": [
    { "path": "index.json", "size": 1841, "blake3": "blake3:…" },
    { "path": "manifests/412.json", "size": 902, "blake3": "blake3:…" },
    { "path": "records.sqlite", "size": 521483264, "blake3": "blake3:…" },
    { "path": "segments/<id>/segment.json", "size": 412, "blake3": "blake3:…" },
    { "path": "segments/<id>/vectors.f16", "size": 1572864000, "blake3": "blake3:…" }
  ]
}
```

La restauration refuse en `invalid_request`, sans rien écrire, un instantané :

- sans `snapshot.json`, ou dont le `format_version` est inconnu ;
- auquel il manque un fichier listé ;
- dont un fichier listé a une taille ou un blake3 différents ;
- qui contient un fichier **non listé**.

**Pourquoi.** C'est le seul contrôle qui couvre `records.sqlite` et `index.json`, et il distingue une troncature d'un fichier complet. Le fichier est déterministe — aucun horodatage — donc deux prises de la même époque produisent les mêmes octets, ce qui rend deux copies comparables. Sa présence à la racine est aussi le signal que la copie est allée au bout : une panne avant son écriture laisse un dossier temporaire, jamais un dossier final incomplet.

**Portée.** `snapshot.json` atteste la copie, pas la génération. Il ne remplace ni `manifests/<G>.json`, ni `meta.generation`, et la restauration ne s'en sert pas comme source de vérité.

### 2. « Le manifeste » est `manifests/<G>.json`, et seul G est copié (B2, conforme)

**Proposition.** Le mot « le manifeste » de la fiche désigne `manifests/<G>.json` de l'époque copiée. Seul le manifeste de la génération G est copié ; ceux des générations antérieures restent l'histoire de l'index et ne font pas partie de l'instantané.

**Pourquoi.** C'est le seul manifeste qu'une époque porte pour l'état courant (`docs/02:85`), et c'est celui que la restauration doit réécrire avec sa nouvelle époque (ADR-0065, décision 2). Le copier n'est pas ce qui rend la restauration possible — la reprise le reconstruit depuis SQLite et les `segment.json` (`docs/02:138`) — mais il est listé par la fiche, il rend l'instantané lisible sans Crible, et le contrôle de la décision 1 le vérifie comme les autres fichiers.

### 3. Un dossier de destination occupé est refusé, la copie passe par un dossier temporaire (C1)

**Proposition.** La destination est `exchange/snapshots/<index>-e<E>-g<G>/`. Si elle existe déjà, la commande s'arrête en `invalid_request` et nomme le dossier : un instantané n'écrase jamais un instantané existant. La copie s'écrit dans `exchange/snapshots/.tmp-<index>-e<E>-g<G>-<pid>-<n>-<nanos>/`, puis le dossier est renommé.

**Pourquoi.** C'est la règle de l'export (ADR-0061, décision 2 ; `docs/04:312-313`), et elle vaut ici pour la même raison : une restauration, ou un opérateur, peut être en train de lire le dossier. Le nom temporaire, avec pid et nanos, garantit que deux prises simultanées n'écrivent jamais au même endroit.

**Conséquence assumée.** Une panne laisse un résidu `.tmp-*` dans `exchange/snapshots/` : la reprise ne couvre pas `exchange/` (`docs/02:132-133`) et aucune commande ne le supprime. Il se nettoie à la main, comme celui de l'export.

### 4. Contenu copié et exclusions (C2, conforme)

**Proposition.** Sont copiés :

- le résultat de `VACUUM INTO` de `records.sqlite`, posé à la racine de l'époque sous le nom `records.sqlite` ;
- pour chaque segment de l'époque, `segment.json` et `vectors.f16` ou `vectors.f32` ;
- `index.json` ;
- `manifests/<G>.json` ;
- `lexical/` s'il existe — absent en P3, présent avec tantivy en P5.

Sont exclus : `cache/`, `codes.bin` et tout fichier dérivé, reconstructibles (`docs/02:80`), ainsi que `CURRENT_EPOCH`, que la restauration pose elle-même.

**Pourquoi.** Le `VACUUM INTO` rend un fichier unique et cohérent, sans `-wal` ni `-shm` : la copie n'a donc pas à embarquer les fichiers annexes de WAL, et le blake3 de la décision 1 porte sur un état logique et non sur un assemblage de trois fichiers. `codes.bin` est dérivé des vecteurs : le copier ajouterait des octets sans rien rendre possible. `CURRENT_EPOCH` nomme l'époque de la **source** : le copier ferait pointer l'index cible vers une époque qui peut déjà exister chez lui.

## Conséquences

- **Positives** :
  - l'instantané est autodescriptif : la restauration sait ce qu'elle doit trouver, et refuse avant d'avoir écrit quoi que ce soit ;
  - le contrôle couvre `records.sqlite` et `index.json`, que ni `manifests/<G>.json` ni les `checksums` des `segment.json` ne couvrent ;
  - `format_version` satisfait INV-15, sans dépendance nouvelle : `blake3` est déjà au stack (`docs/07:43`) ;
  - le contenu est déterministe, donc deux prises de la même époque sont comparables octet pour octet ;
  - un dossier final n'existe que complet : la copie n'est visible qu'après le renommage, comme pour l'export.
- **Négatives / coûts** :
  - le hachage relit toute l'époque une seconde fois, vecteurs compris : à l'échelle de Legifacile, les vecteurs de référence fp16 pèsent à eux seuls environ 13,2 Go (`docs/02:195`). La durée n'est pas mesurée, et cet ADR n'avance aucun chiffre ;
  - le refus d'un fichier en trop est strict : une note ajoutée à la main, ou un `.DS_Store` déposé par macOS, rend l'instantané inutilisable ;
  - tout fichier ajouté un jour à une époque — tantivy en P5, un futur dérivé qui devrait survivre — doit entrer à la fois dans la copie et dans `snapshot.json`, sinon des instantanés légitimes deviennent irrestaurables ;
  - `snapshot.json` ne porte pas d'horodatage, et le nom du dossier n'en porte pas non plus : rien ne dit quand la copie a été prise ;
  - un format de plus à versionner (INV-15) et une cause de plus pour `invalid_request`.
- **À surveiller** :
  - le coût du hachage, à relever à la première prise sur l'échantillon Legifacile ; s'il domine la copie, la question d'un contrôle plus léger pour les vecteurs demandera une décision, pas un contournement ;
  - P5 : le dossier `lexical/` doit entrer dans la liste, sinon la restauration d'une époque tantivy sera refusée ;
  - si un opérateur a besoin de la date de prise, l'absence d'`created_at` se rouvre ; le champ n'est pas ajouté ici.

## Alternatives écartées

| Décision | Alternative | Pourquoi écartée |
|---|---|---|
| 1 | (b) s'appuyer sur `manifests/<G>.json` et les `checksums` des `segment.json` | Ces deux sources ne couvrent ni `records.sqlite` ni `index.json` : une troncature de la base passe sans être vue. Le manifeste est de plus dérivé et reconstruit s'il manque (`docs/02:85`, `:138`), donc il ne peut pas attester une copie |
| 1 | (c) ne contrôler que l'absence des fichiers | Un fichier tronqué, vide ou écrasé passe : le critère « un instantané incomplet est refusé » ne serait tenu qu'à moitié, et un vecteur corrompu ne se verrait qu'à la première recherche |
| 1 | Copier `records.sqlite` avec ses `-wal` et `-shm` au lieu d'un `VACUUM INTO` | `docs/04:300` prescrit déjà le `VACUUM INTO` ; trois fichiers à copier ensemble ne font pas un état cohérent vis-à-vis d'un écrivain, et le checksum ne dirait plus rien de l'état logique |
| 3 | Écraser un dossier existant | Un instantané peut être en cours de lecture par une restauration ou par un opérateur ; c'est le précédent de l'export (ADR-0061, décision 2) |
| 4 | Copier `cache/` et `codes.bin` | Dérivés (`docs/02:80`), reconstructibles ; les copier ajoute des octets et fait voyager un cache à invalider, sans rien rendre possible |
| 4 | Copier `CURRENT_EPOCH` | Il nomme l'époque de la source ; l'installer ferait pointer l'index cible vers une époque qui peut déjà exister chez lui, et la restauration doit poser ce pointeur elle-même (ADR-0065) |

## Documents à modifier après acceptation

| Document | Modification, selon les décisions retenues |
|---|---|
| `docs/04-donnees-et-ingestion.md` | section « Export, instantanés, restauration » : contenu exact de l'instantané et exclusions (4), `snapshot.json` et ses contrôles (1), refus d'un dossier de destination occupé et écriture sous dossier temporaire (3), « le manifeste » = `manifests/<G>.json` (2) |
| `docs/02-architecture.md` | seulement si l'organisation sur disque décrit les fichiers d'échange : ajouter `snapshot.json` et son rôle (attester une copie, pas décrire une génération) |
| `docs/08-invariants-et-tests.md` | mentionner `snapshot.json` là où les formats versionnés sont listés, au titre d'INV-15 |

Les lignes d'erreur de `docs/05-api.md` relèvent d'ADR-0065, décision 6, qui les traite pour les deux commandes.

## Références

- Fiche `CREER-ET-RESTAURER-INSTANTANE` : `docs/phases/P3-cycle-de-vie.md:121-130`
- `docs/04-donnees-et-ingestion.md:296-314` : tableau export/instantané, règles de l'export
- `docs/02-architecture.md` : organisation sur disque (`:55-80`), `CURRENT_EPOCH` (`:84`), `manifests/<generation>.json` (`:85`), reprise (`:129-142`), compaction (`:155-181`), mémoire et tailles (`:188-199`)
- `docs/08-invariants-et-tests.md` : INV-14 (`:22`), INV-15 (`:23`)
- `docs/07-stack-et-deploiement.md:43` : `blake3` au stack
- `Cargo.toml:53`, `crates/crible-store/src/segment.rs` (checksums, forme `blake3:<hex>`), `crates/crible-store/src/export.rs` (refus d'écraser, dossier de transit), `crates/crible-store/src/records.rs:1137` (WAL)
- ADR-0016 (commit SQLite, manifeste dérivé), ADR-0061 (manifeste d'un lot exporté, refus d'écraser et transit), ADR-0065 (instantané hors ligne, époque de restauration, cible et rétention)
