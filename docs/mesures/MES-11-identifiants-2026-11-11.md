# MES-11 — identifiants nus en lexical : rejeu de MES-11 sur les six index complets

## Campagne du 2026-11-11 (tâche `T14-IDENTIFIANTS-NUS-LEXICAL`, Crible 1.1)

Cette campagne est le **rejeu de Q-04** qu'ADR-0094 prévoyait et qu'ADR-0148 spécifie. Elle
mesure, dans le **moteur** et sur les **six index complets**, ce que le réglage optionnel
`lexical.identifiers` change : la passe 1.0 (`"none"`, réglage absent) puis la passe `"v1"`
(jeton canonique d'identifiant ajouté aux morceaux). Le protocole et la règle de décision sont
écrits **avant** la campagne dans
[ADR-0148](../adr/0148-jeton-canonique-des-identifiants-dans-l-analyse-lexicale.md), section
« Mesure » ; ce rapport les applique et ne les modifie pas.

- **Tâche** : `T14-IDENTIFIANTS-NUS-LEXICAL`. Branche `tache/T14-IDENTIFIANTS-NUS-LEXICAL`,
  commit `ca3072c5a6596d485f2260cf44e22cc84bc71a3f` (prototype relu `OK` d'ADR-0148).
- **Machine** : MacBook Air `Mac15,12` (Apple M3), 8 cœurs (4 performance, 4 efficacité), **24 Go**,
  macOS 15.7.7 (24G720), **natif** (pas de Docker ; la campagne P10 d'ADR-0142 avait tourné dans une
  VM Docker de 11,67 Gio sur la même machine, et cette différence compte pour le point 4).
- **Données** : `data/t14-mes11/data`, six index importés des lots `replace_all` de
  `data/legifacile-full/` (18 Go, non modifiés) : 6 660 239 documents et **9 600 105 passages**,
  0 mort, comptes égaux à ceux de `import-legifacile-2026-11-06.md`. Empreintes blake3 des six
  `manifest.json` de lot, qui identifient la version importée des données (les fichiers Parquet
  eux-mêmes ne sont pas hachés ici) :
  `articles` `5f4e8c9e…d47f0d4`, `decisions` `25519543…30652dd`, `dole` `d10c1134…a03996`,
  `jorf` `9efe657f…14e1645ac`, `sarde` `4d937e8d…69ded78a`, `titres` `5051e9a0…216cf3b`
  (détail complet dans `data/t14-mes11/raw/lot-manifests.blake3.json`). Modèle
  `data/models/mdenseon`, ONNX Runtime par `ORT_DYLIB_PATH`
  (`tools/.venv/lib/python3.13/site-packages/onnxruntime/capi/libonnxruntime.1.31.0.dylib`).
- **Configuration** : les six fichiers de `docs/legifacile/configs/`, copiés tels quels par
  `crible index create` : `reference_precision` `fp16`, `lexical.enabled` `true`,
  `lexical.positions` `false`, `ingest.acceptance_sample` 200, `acceptance_min_cosine` 0,999.
  La passe 1.0 n'écrit **pas** `lexical.identifiers` (vérifié : la clé est absente des six
  `index.json`).
- **Espace** : 90 GiB libres au départ, 59 GiB à la fin (l'import et les deux passes lexicales
  occupent environ 31 GiB) ; `data/t14-mes11/` pèse **25 Go** à la fin.
- **Durée** : 112,9 minutes de temps mural du début de la campagne à ce compte rendu, lecture,
  analyse et rédaction comprises.

### Protocole

Les commandes sont celles de la CLI, **sans serveur** : `crible index create`, `crible attach`,
`crible rebuild-lexical`, `crible search`, `crible eval`, `crible inspect`. Aucune commande n'a
tourné en arrière-plan et aucune n'a dépassé 30 minutes ; chaque attache et chaque reconstruction
est un appel séparé. Trois harnais nouveaux portent la mesure, sous `tools/bench/t14/`
(`rank.py`, `retokenized.py`, `aggregate.py`) et un exemple de lecture de l'index lexical
(`crates/crible-store/examples/lexical_terms.rs`, sur le modèle de `lexical_tokens.rs` de la
campagne P10). Aucun code de production ni aucun test n'a été modifié.

**Import.** Six attaches natives, dans l'ordre des tailles croissantes, avec le réglage absent :

```sh
./target/release/crible index create --data data/t14-mes11/data --config docs/legifacile/configs/<index>.json
./target/release/crible attach --data data/t14-mes11/data --models data/models \
    --index <index> data/legifacile-full/<index>/<lot>
```

**Reconstruction lexicale.** Une commande chronométrée par index et par passe, c'est la grandeur
d'ADR-0142 :

```sh
/usr/bin/time -l ./target/release/crible rebuild-lexical --data data/t14-mes11/data --index <index>
```

**Rangs.** `crible search --query <texte> --mode lexical|hybrid --k 1000`, puis recherche du
document attendu dans la réponse. Deux conventions sont rapportées, parce qu'elles diffèrent :
`flat` (une ligne par passage, la convention du ticket 14, qui donne 234 pour `10-13.625`) et
`grouped` (`--group-by document`). Un document absent des 1 000 premiers résultats est noté
`absent`, jamais extrapolé : c'est une borne, pas un rang. La CLI n'expose pas `explain_ids` ;
le harnais `rank.py` lit donc la liste des résultats.

**Jeu de 97 questions.** `docs/legifacile/reference/` **inchangé** : rappel et MRR par index, puis
agrégation sur les 97 :

```sh
./target/release/crible eval --data data/t14-mes11/data --index <index> \
    --queries docs/legifacile/reference/<index>.jsonl --models data/models \
    --modes lexical,hybrid --k 10 --out data/t14-mes11/out/<index>
```

Les deux lignes `APPLICABLE` du jeu portent le jour figé du **2026-11-03** ; elles sont rejouées
telles quelles, comme `jeu-de-reference.md` l'exige.

**Bootstrap.** ADR-0092, décision 6 : 10 000 rééchantillonnages des 97 questions, graine
`20261111`, différence `v1 − none`, intervalle de percentile à 95 %.

**Préchauffage, répétitions, cache.** Les rangs sont une requête unique par sonde et par mode,
sans préchauffage, avec le cache du système chaud depuis l'import et la reconstruction ; chaque
appel rouvre l'index et le premier appel d'un mode paie cette ouverture. Les durées de
reconstruction sont une passe chronométrée par index et par passe (`/usr/bin/time -l`), et un
second passage de `decisions` sous `"none"` donne la dispersion (écart au protocole 4).
`crible eval` fait une passe de préchauffage puis **3 répétitions comptées**, dont la première
seule paie les passages `exact` et groupé (ADR-0125, décision 9). Aucune latence de requête n'est
rapportée : la campagne mesure des rangs, un rappel et des durées de reconstruction.

**Mise en forme.** Les valeurs des tableaux sont celles des sorties brutes de
`data/t14-mes11/raw/`, résumées dans `MES-11-identifiants-2026-11-11.json`. Aucune valeur n'est
arrondie au-delà de la précision affichée, et aucune n'est filtrée.

### Résultats — 1 et 2, les deux points bloquants

Complément déclaré, hors des 97 questions (`MES-11-identifiants-complement.jsonl`), rang du
document attendu selon la convention :

| Sonde | Index | Requête | Attendu | `"none"` lexical plat | `"none"` lexical groupé | `"none"` hybride plat | `"v1"` lexical plat | `"v1"` lexical groupé | `"v1"` hybride plat |
|---|---|---|---|---|---|---|---|---|---|
| critère du ticket | `decisions` | `10-13.625` | `JURITEXT000023607955` | 234 | 222 | 468 | **1** | **1** | **3** |
| complément | `decisions` | `81-70.489` | `JURITEXT000007009951` | 9 | 9 | 18 | 1 | 1 | 1 |
| complément | `articles` | `L1132-1` | `LEGIARTI000045391841` | 79 | 79 | 3 | 38 | 38 | 2 |
| complément | `articles` | `L. 1132-1` | `LEGIARTI000045391841` | absent (> 1 000) | absent | 12 | absent (> 1 000) | absent | 12 |
| contrôle négatif | `articles` | `l1132-11` | l'article `L1132-1` hors du top 3 | 79 | 79 | 2 | 80 | 80 | 2 |
| complément | `sarde` | `SANP0422530D` | `SARDBLOC000007251655` | 2 | 2 | 15 | 2 | 2 | 15 |

Les quatre documents attendus ont été vérifiés présents dans leur index avant la mesure (lecture
seule de `records.sqlite` : `JURITEXT000023607955`, `JURITEXT000007009951`, `LEGIARTI000045391841`,
`SARDBLOC000007251655`, un document chacun ; `LEGIARTI000045391841` porte bien `num` `L1132-1`).

**La passe 1.0 reproduit les rangs du ticket au chiffre près** : 234 et 468 pour `10-13.625`,
79 et 3 pour `L1132-1`, 9 et 18 pour `81-70.489`, 2 et 15 pour le NOR, et `L. 1132-1` hors du
top 1 000. C'est l'ancrage de la campagne : les index complets régénérés rendent les rangs de
production du 2026-11-10, alors que ceux-ci n'étaient plus reproductibles depuis la destruction
des index de P10.

**Ce que le jeton canonique fait, et ce qu'il ne fait pas.** Sous `"v1"`, `explain_ids` n'existe
pas en CLI, mais l'index répond : la requête et le texte indexé traversent la même chaîne, et
l'exemple `lexical_terms` lit le sac de termes et la fréquence documentaire (`doc_freq`) de chaque
terme. `doc_freq` est le nombre de passages qui portent le terme, donc le nombre de compétiteurs
directs.

| Index | Requête | Termes de la chaîne `"v1"`, avec leur `doc_freq` |
|---|---|---|
| `decisions` | `10-13.625` | `10` 541 855, `13` 338 984, `625` 9 975, **`10-13-625` 1** |
| `decisions` | `81-70.489` | `81` 27 119, `70` 33 271, `489` 2 500, **`81-70-489` 1** |
| `articles` | `L1132-1` | `l1132` 88, `1` 1 192 672, **`l1132-1` 549** |
| `articles` | `L. 1132-1` | `1132` 1 210, `1` 1 192 672, **`l1132-1` 549** (la chaîne 1.0 ne donnait que `1132` et `1`) |
| `articles` | `l1132-11` | `l1132` 88, `11` 267 455, **`l1132-11` 0** |
| `sarde` | `SANP0422530D` | `sanp0422530d` 36, aucun jeton ajouté (déjà un jeton unique) |

Trois faits en découlent, tous mesurés. Le pourvoi du critère a **un seul porteur** dans
`decisions` : c'est ce qui explique le rang 1, et non un effet de score. Le contrôle négatif tient
par construction : `l1132-11` n'a **aucun** porteur, et les morceaux `l1132` et `11` n'apparient ni
`l1132` ni `1` de l'article. Enfin la **forme citée `L. 1132-1` reste noyée**, alors que la
décision 2 d'ADR-0148 attend qu'elle donne le même canonique que `L1132-1` : elle le donne bien
(le sac de termes le montre), mais elle **perd le morceau `l1132`** que la forme stockée partage,
gagne le morceau `1132` (1 210 passages) et se trouve donc en compétition avec les 549 passages
qui portent `l1132-1` sans la remise de score que `l1132` apportait (88 passages seulement). Le
rang 38 de `L1132-1` n'est donc pas atteint par la forme citée. Aucun point de la règle de
décision ne dépend de cette sonde ; c'est une **réserve** pour la note de version.

### Résultats — 3, non-régression sur les 97 questions

Jeu inchangé, 97 questions, `k` = 10, regroupement par document :

| Branche | Passe | recall@10 | MRR@10 | écart de recall@10 | IC 95 % de l'écart |
|---|---|---|---|---|---|
| lexical | `"none"` | 0,6392 | 0,4181 | — | — |
| lexical | `"v1"` | 0,6495 | 0,4345 | **+0,0103** | **[0,0000 ; +0,0309]** |
| hybride | `"none"` | 0,6289 | 0,3872 | — | — |
| hybride | `"v1"` | 0,6289 | 0,3876 | 0,0000 | [0,0000 ; 0,0000] |

Le rappel lexical de la passe 1.0 vaut **0,639**, exactement le chiffre que le ticket 14 relève sur
les 97 questions de production : second ancrage de la campagne. **Une seule question change de
résultat** : `MES00-q0007` (index `articles`) passe de manquée à trouvée, ce qui vaut
1/97 ≈ 0,0103. Les 96 autres sont identiques, d'où une borne basse de l'intervalle **exactement
nulle** : les rééchantillonnages qui omettent cette question donnent un écart nul, et 35,9 % des
10 000 tirages sont dans ce cas. L'intervalle n'est pas entièrement négatif, et l'écart de MRR est
positif (+0,0164) sans aucune question dégradée : sept questions gagnent en MRR (`Q21` et ses trois
variantes +0,0179 chacune, `MES00-q0004` +0,8571, `MES00-q0007` +0,5000, `Q56` +0,1667), aucune
n'en perd.

Par index, rappel@10 lexical (`"none"` → `"v1"`) : `dole` 1,0000 → 1,0000 ;
`sarde` 1,0000 → 1,0000 ; `titres` 1,0000 → 1,0000 ; `jorf` 0,8000 → 0,8000 ;
`articles` 0,4400 → 0,4800 ; `decisions` 0,5778 → 0,5778. L'hybride est inchangé partout
(`jorf` 0,7000, `articles` 0,5600, `decisions` 0,5111) ; trois questions y gagnent en MRR
(`Q02` +0,0139, `MES00-q0009` +0,0179, `Q49` +0,0111), ce qui donne l'écart de +0,00044.

### Résultats — 4, budget : durées et tailles des six

Durées de `crible rebuild-lexical`, une passe par index, contre les valeurs d'ADR-0142
(1, 5, 13, 71, 127 et 185 s) :

| Index | Passages | `"none"` | × ADR-0142 | `"v1"` | × ADR-0142 | `"v1"` ÷ `"none"` | `lexical/` `"none"` | `lexical/` `"v1"` | croissance | Passages retokenisés |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| `dole` | 7 653 | 0,83 s | ×0,83 | 1,96 s | ×1,96 | ×2,36 | 1 883 918 o | 1 992 074 o | +5,7 % | 22,4 % |
| `sarde` | 214 578 | 6,67 s | ×1,33 | 12,15 s | ×2,43 | ×1,82 | 16 398 964 o | 19 185 243 o | +17,0 % | 6,0 % |
| `titres` | 1 477 883 | 20,93 s | ×1,61 | 31,07 s | ×2,39 | ×1,48 | 46 616 644 o | 50 071 369 o | +7,4 % | 4,2 % |
| `jorf` | 1 833 500 | 92,10 s | ×1,30 | 161,27 s | ×2,27 | ×1,75 | 207 609 117 o | 216 462 435 o | +4,3 % | 8,4 % |
| `articles` | 2 476 715 | 156,98 s | ×1,24 | 306,76 s | ×2,42 | ×1,95 | 320 446 636 o | 340 882 492 o | +6,4 % | 10,6 % |
| `decisions` | 3 589 776 | 234,86 s | ×1,27 | 467,48 s | ×2,53 | ×1,99 | 466 513 253 o | 481 385 358 o | +3,2 % | 20,5 % |
| **somme** | **9 600 105** | **512,37 s** | **×1,27** | **980,69 s** | **×2,44** | **×1,91** | **1 059 468 532 o** | **1 109 978 971 o** | **+4,8 %** | **12,8 %** |

Les deux écarts se lisent séparément, et le rapport ne les confond pas.

1. **Même la passe 1.0 dépasse les références d'ADR-0142**, de +24 % à +61 % selon l'index. Trois
   causes sont nommées, aucune n'est mesurée isolément : la campagne d'ADR-0142 a tourné **dans un
   conteneur Docker** (VM de 11,67 Gio) sur la même machine, celle-ci tourne en **natif** ; le
   binaire est celui du **prototype 1.1**, dont le flux recopie le texte de chaque jeton dans un
   tampon réutilisé au lieu de l'emprunter au flux interne (`IdentifierTokenStream::advance`),
   alors que la campagne P10 tournait sur un binaire d'avant le prototype ; enfin les index de
   P10 avaient subi des compactions (`jorf` en 10 segments, `decisions` en 4) alors que ceux-ci
   portent **un seul segment** par index. La passe 1.0 ne mesure donc pas une régression du
   prototype, mais un autre point de comparaison. L'écart entre deux passes sur la même machine
   (colonne « `"v1"` ÷ `"none"` ») est, lui, directement attribuable au jeton ajouté.
2. **Le jeton canonique coûte ×1,91 en durée et +4,8 % en taille, sur la somme des six.** Le coût
   suit la part des passages retokenisés, pas la taille de l'index : `decisions` (20,5 % des
   passages retokenisés) double presque sa durée (×1,99) ; `sarde`, avec 6,0 % de passages
   retokenisés, gagne **+17,0 % de taille** parce que ses seuls 12 840 passages changés y sèment
   des termes rares et dispersés, qui se compriment mal ; `titres` est le moins touché (4,2 % des
   passages, ×1,48).

**Verdict du point 4.** Le budget est **dépassé sur les six index** : contre les références
d'ADR-0142, `"v1"` vaut ×1,96 à ×2,53. Chaque dépassement est expliqué ci-dessus index par index,
conformément à la règle ; aucun n'est imputé à une cause unique non mesurée. Les six durées et les
six tailles sont rapportées telles quelles, sans extrapolation.

### Part des passages retokenisés

**Mesurée**, à 12,8 % sur les 9 600 105 passages : 1 228 184 passages changent de jetons, avec
`dole` 22,4 %, `decisions` 20,5 %, `articles` 10,6 %, `jorf` 8,4 %, `sarde` 6,0 % et `titres` 4,2 %.
La méthode est une transcription fidèle des fonctions `canonical_tokens`, `article_at`,
`number_at`, `starts_a_word` et `ends_a_word` de
`crates/crible-store/src/lexical/analyzer.rs`, validée avant tout comptage sur les 16 chaînes que
les tests de ce fichier figent (`L. 1132-1`, `C100123`, `1240`, `15-02-2011`, `1.5`, `1 240`,
`N°1234`, `ECLI:…`, etc.) ; la comparaison ne rejoue donc pas deux chaînes complètes, mais le
prédicat exact « ce texte ajoute au moins un jeton canonique ». MES-11 mesurait 21,1 % pour la V2
de sa réplique, qui **remplaçait** des morceaux : les deux chiffres ne sont pas la même grandeur.

### Durées d'attache

Relevées à titre de repère, hors règle de décision (la CLI est synchrone, sans serveur) :

| Index | Documents | Passages | Durée d'attache | Pic RSS |
|---|---:|---:|---:|---:|
| `dole` | 3 600 | 7 653 | 158,16 s | 2,07 Go |
| `sarde` | 210 965 | 214 578 | non relevée (voir écarts) | 2,22 Go |
| `titres` | 1 477 883 | 1 477 883 | 97,24 s | 2,22 Go |
| `jorf` | 1 636 145 | 1 833 500 | 304,60 s | 2,66 Go |
| `articles` | 2 135 023 | 2 476 715 | 495,94 s | 4,49 Go |
| `decisions` | 1 196 623 | 3 589 776 | 913,97 s | 3,33 Go |

`dole` paie la session ONNX fp32 de la recette d'acceptation (158 s pour 7 653 passages), comme en
P10 ; `sarde` la réutilise et n'a pas été chronométrée. Le pic de RSS natif d'`articles` vaut
4,49 Go, contre 6,54 Go mesurés en natif par `CORRIGER-MEMOIRE-APRES-COMMIT` : la machine n'a
jamais approché l'OOM de la VM de P10.

### Conclusion

**Les quatre points de la règle de décision d'ADR-0148 sont satisfaits.**

1. **Bloquant, critère du ticket — satisfait.** `10-13.625` en lexical sur `decisions` complet rend
   `JURITEXT000023607955` au **rang 1** (rang 234 sous `"none"`), plat comme groupé. Le jeton
   canonique `10-13-625` n'a **qu'un porteur** dans l'index : la décision est trouvée, pas
   seulement remontée.
2. **Bloquant, contrôle négatif — satisfait.** `l1132-11` laisse l'article `L1132-1` au rang
   lexical **80** (79 sous `"none"`), hors du top 3. Le canonique `l1132-11` n'a **aucun** porteur,
   et les morceaux `l1132` et `11` ne reforment pas l'article.
3. **Non-régression — satisfaite.** L'intervalle de la différence de recall@10 lexical
   (`"v1" − "none"`) vaut **[0,0000 ; +0,0309]** : il n'est pas entièrement négatif. L'écart de
   MRR@10 est positif (+0,0164), et la seule question qui bouge gagne son attendu.
4. **Budget — dépassé, et expliqué index par index.** `"v1"` vaut ×1,96 à ×2,53 des références
   d'ADR-0142 (1, 5, 13, 71, 127 et 185 s), pour ×1,91 en durée et +4,8 % en taille sur la somme
   des six. La passe 1.0 dépasse déjà ces références de +24 % à +61 %, et le rapport sépare les
   deux causes : le point de comparaison (natif contre conteneur, prototype 1.1, un segment au
   lieu de plusieurs) et le coût du jeton ajouté.

**Recommandation, à valider par le propriétaire.** La décision d'ADR-0148 peut s'appliquer : les
deux critères bloquants sont tenus sur les index complets, sans régression mesurable sur les 97
questions. Deux propositions accompagnent la décision, aucune n'est appliquée ici : (a) la note de
version 1.1.0 doit annoncer, par index, les durées **mesurées** de la colonne `"v1"` (1,96 ; 12,15 ;
31,07 ; 161,27 ; 306,76 ; 467,48 s) et non les références de 1.0, qui sous-estiment la
reconstruction d'environ 2 fois ; (b) la **forme citée `L. 1132-1` reste hors du top 1 000** alors
que la décision 2 d'ADR-0148 attend qu'elle rejoigne `L1132-1` : c'est un écart entre le texte de
l'ADR et la mesure, à trancher par un ADR ou par une ligne de la note de version, pas par cette
campagne.

### Écarts au protocole, déclarés

1. **Le réglage `"v1"` a été posé à la main, faute de commande.** Aucune commande de la CLI
   n'écrit `lexical.identifiers` ; `crible index create` copie le fichier tel quel et `PATCH` est
   une route de serveur. Les six `index.json` de **la copie de mesure** (`data/t14-mes11/`) ont donc
   été réécrits dans la forme que produit `IndexConfig::to_value` : la clé `"identifiers": "v1"`
   ajoutée **dans** l'objet `lexical`, rien d'autre. Aucun autre index n'a été touché, et
   `data/legifacile-full/` est intact. Le chemin `PATCH` + job d'ADR-0105 n'est donc **pas**
   exercé ; la reconstruction l'est par la même commande `crible rebuild-lexical` que la passe 1.0,
   et l'ouverture en écriture a bien signalé le changement de chaîne :
   « the lexical index was built with another analysis chain (ADR-0106, ADR-0148) », avec
   `recorded_identifiers: "none"` et `configured_identifiers: "v1"`.
2. **Rangs lus par `crible search --k 1000`, pas par `explain_ids`.** La CLI n'expose pas
   `explain_ids`. Un document au-delà du rang 1 000 est noté « absent » : c'est une borne, et le
   rapport ne lui prête jamais le rang 498 425 du ticket.
3. **Une durée d'attache perdue.** La sortie de `/usr/bin/time` de `sarde` a été tronquée par le
   harnais ; la durée n'est pas relevée et n'est pas reconstituée. `dole` a été relevée à l'écran
   (158,16 s) sans fichier brut.
4. **Une seule répétition par index et par passe** pour la reconstruction. Un second passage de
   `decisions` sous `"none"` donne **250,67 s** contre 234,86 s, soit **+6,7 %** : les durées
   portent une dispersion de cet ordre, et le rapport ne présente pas les écarts de quelques
   pour cent comme significatifs.
5. **Les deux passes ne mesurent pas les mêmes statistiques de termes.** Passer de `"none"` à
   `"v1"` change l'`idf` de tous les termes, donc le classement de toutes les questions. C'est le
   sens de la mesure, mais cela interdit d'attribuer un déplacement de rang à la seule présence du
   jeton : la table `doc_freq` est là pour séparer les deux effets.
6. **Le binaire s'annonce `1.0.0`.** `Cargo.toml` n'a pas été relevé sur la branche ; les rapports
   `crible eval` portent donc `"version": "1.0.0"` alors que le prototype est celui de la 1.1.

### Réserves

1. **Le point 4 est le point faible de la campagne, pas les points bloquants.** Les deux critères
   bloquants sont tenus avec une marge nette (rang 1 sur un terme à porteur unique ; aucun porteur
   pour le contrôle négatif), mais la comparaison au budget d'ADR-0142 mêle deux changements : le
   passage du conteneur au natif et le jeton ajouté. La seule comparaison nette est
   `"v1" ÷ "none"` sur la même machine, **×1,91** au total.
2. **Le corpus n'est pas celui de la production du 2026-11-10.** Les comptes sont égaux à pgvector
   (9 600 105 passages) et les rangs de la passe 1.0 reproduisent ceux du ticket au chiffre près,
   ce qui est le meilleur ancrage disponible ; mais la campagne du ticket lisait des index de
   développement que le propriétaire a détruits depuis, et l'égalité des rangs ne prouve pas
   l'égalité des corpus document par document.
3. **La part des passages retokenisés repose sur une transcription, pas sur le moteur.** Le
   prédicat Python reproduit l'analyseur et il est validé sur les 16 chaînes figées par les tests,
   mais il n'a pas été confronté passage par passage aux deux flux de jetons du moteur : cette
   confrontation aurait demandé de re-tokeniser deux fois les 9 600 105 passages, et la campagne ne
   l'a pas faite.
4. **La forme citée `L. 1132-1` reste perdue** (absente du top 1 000 dans les deux passes, hybride
   rang 12 dans les deux passes) : la décision 2 d'ADR-0148 attend qu'elle donne le même canonique,
   ce qu'elle fait, mais le rang de `L1132-1` ne suit pas. Un utilisateur qui cite un article avec
   un point et une espace n'a toujours aucun signal.
5. **Un identifiant incident dans une longue question** n'est pas mesuré par cette campagne : le
   complément est fait d'identifiants nus, et les 97 questions ne portent qu'un seul déplacement
   (`MES00-q0007`, qui gagne son attendu). Le risque de domination du score, nommé par ADR-0148,
   reste ouvert.
6. **Le contrôle négatif passe à 79 puis 80** : il n'est pas dans le top 3, mais il est dans le top
   100 dans les deux passes. Le critère est binaire (top 3, comme l'ADR le formule) et il est
   tenu ; le rapport ne prétend pas que l'article soit absent du classement.

## Fichiers

Versionnés, à commiter par `@git` (7 fichiers) :

- `docs/mesures/MES-11-identifiants-2026-11-11.md` — ce rapport ;
- `docs/mesures/MES-11-identifiants-2026-11-11.json` — les sorties brutes résumées : machine,
  commit, durées d'attache, durées et tailles des deux passes, rangs du complément, `crible eval`
  des 97 avec l'intervalle bootstrap, part des passages retokenisés, conventions de mesure ;
- `docs/mesures/MES-11-identifiants-complement.jsonl` — le complément déclaré, au format de
  `crible eval` (`id`, `query`, `expected`), une ligne par sonde ;
- `tools/bench/t14/rank.py` — rangs `lexical` et `hybride`, plat et groupé, du complément ;
- `tools/bench/t14/aggregate.py` — agrégation des 97 questions et bootstrap apparié d'ADR-0092 ;
- `tools/bench/t14/retokenized.py` — part des passages retokenisés, prédicat validé sur les tests ;
- `crates/crible-store/examples/lexical_terms.rs` — sac de termes et `doc_freq` d'une requête, sur
  le modèle de `lexical_tokens.rs`.

Bruts, gitignorés, sous `data/t14-mes11/` (25 Go, laissés en place) : `data/` (les six index),
`out/` et `out-v1/` (les douze rapports `crible eval`), `raw/` (`attach-*.txt`,
`rebuild-none-*.txt`, `rebuild-v1-*.txt`, `lexical-sizes-*.json`, `ranks-none.json`,
`ranks-v1.json`, `eval-summary.json`, `retokenized.json`, `terms-*.txt`, `inspect-pass1.txt`,
`du-indexes-pass1.txt`, `lot-manifests.blake3.json`).
