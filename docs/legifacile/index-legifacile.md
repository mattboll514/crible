# Index de Legifacile

Ce document fixe les **six configurations d'index** de Legifacile pour la v1 de Crible, et la
correspondance entre les filtres du serveur MCP actuel et ceux de Crible. Il répond à la tâche
`DEFINIR-INDEX-LEGIFACILE` (P10) et tranche la question **Q-03** (champs multivalués) par
ADR-0137.

Les fichiers sont dans `docs/legifacile/configs/` :

| Fichier | Index |
|---|---|
| `articles.json` | `articles` |
| `jorf.json` | `jorf` |
| `decisions.json` | `decisions` |
| `titres.json` | `titres` |
| `sarde.json` | `sarde` |
| `dole.json` | `dole` |

Chaque fichier est lu et validé par `IndexConfig::from_json_str`, le chemin réel de lecture d'un
`index.json` stocké, par le test `crates/crible-core/tests/legifacile_configs.rs`.

## Sources et méthode

Le découpage en six index est celui de `docs/02-architecture.md` : il est validé et n'est pas
rouvert ici. Les champs viennent des vues `vec.src_*` de Legifacile, du compositeur
`scripts/db/sync_vecteurs.py` et des filtres réellement exposés par `serveur_mcp/outils.py`. Les
types et les valeurs ont été vérifiés sur l'échantillon de P2 (`data/legifacile-sample`, exporté
par `scripts/export_crible_sample.py`).

Le dépôt Legifacile n'a été lu qu'en lecture seule. Aucune vue, aucun script et aucune base n'a
été modifié. Aucune valeur de `.env.prod` n'est citée.

## Vue d'ensemble

| Index | Corpus Legifacile | Unité | `cluster_by` | Gabarit | Découpeur |
|---|---|---|---|---|---|
| `articles` | `textes_vigueur` + `textes_hors_vigueur` | article | `["code"]` | `article_chemin_num_texte` v1 | `decouper` v1 |
| `jorf` | `jorf_normatif` | article publié | `["annee"]` | `article_chemin_num_texte` v1 | `decouper` v1 |
| `decisions` | `juri_texte_integral` + `juri_sommaires` | passage | `["juridiction"]` | `juri_titre_corps` v1 | `decouper_juri` v3 (intégral), `decouper` v1 (sommaire) |
| `titres` | `titres_textes` | titre | `["nature"]` | `titre_seul` v1 | `decouper` v1 |
| `sarde` | `sarde` | bloc | `["theme"]` | `titre_corps` v1 | `decouper` v1 |
| `dole` | `dole` | dossier | `["type"]` | `titre_expose_motifs` v1 | `decouper` v1 |

Le `doc_id` de chaque document est le `src_id` de la vue (id externe DILA). La clé de passage est
`doc_id#ord`. Le `chunker` et le `template` ne sont pas des réglages d'index : ils sont déclarés
dans le manifeste de chaque lot (ADR-0010), et `docs/format-lot-v1.md:88` n'admet qu'un `chunker`
par manifeste. La colonne « Découpeur » donne le découpeur réellement appliqué à chaque corpus,
celui de `sync_vecteurs.CORPUS` en production. Pour `decisions`, il y en a deux : `decouper_juri`
v3 sur `juri_texte_integral` et `decouper` v1 sur `juri_sommaires`. Le lot de P2 déclare pourtant
`decouper_juri` v3 dans son manifeste, sommaires compris, tout en appliquant le découpeur de
chaque corpus aux documents. `EXPORTER-BASE-LEGIFACILE` reprendra cette forme : un seul lot
`decisions`, un seul `chunker` déclaré (`decouper_juri` v3), et le découpeur réellement appliqué
par document reste celui de son corpus. Le manifeste décrit la structure du lot, pas le découpeur
de chaque document.

`model` vaut `mdenseon`, `reference_precision` vaut `fp16`, et les blocs `search`, `ingest` et
`compaction` reprennent les valeurs par défaut de `docs/04` (`scan: binary`, N = 5000,
`exact_threshold` = 5000, `candidates_per_branch` = 100, `fusion: rrf`, `acceptance_sample` =
200, `acceptance_min_cosine` = 0,999, `dead_ratio` = 0,10). Aucun de ces réglages n'est propre à
Legifacile : les changer demanderait une mesure, pas une préférence.

## Champs par index

Tous les champs sont `filterable: true` et `returned: true` (les deux défauts de `docs/04`). Tous
acceptent null. Aucun nom n'est déclaré aux deux niveaux (ADR-0030).

### `articles`

| Champ | Type | Niveau | Source |
|---|---|---|---|
| `fonds` | `keyword` | document | `legi.articles.fonds` (`LEGI`, `KALI`) |
| `etat` | `keyword` | document | `legi.articles.etat` |
| `annee` | `integer` | document | `vec.annee(date_debut)` |
| `code` | `keyword` | document | `contexte -> titre_txt -> 0 ->> text` (nom du code, ou titre du texte consolidé) |
| `num` | `keyword` | document | `legi.articles.num` |
| `texte_cid` | `keyword` | document | `legi.articles.texte_cid` |
| `date_debut` | `date` | document | `legi.articles.date_debut` |
| `date_fin` | `date` | document | `legi.articles.date_fin` |

La hiérarchie complète (`contexte`) n'est pas un champ : elle n'est pas un filtre du MCP et reste
dans le texte composé par le gabarit. `code` porte le premier niveau de cette hiérarchie, celui
que la vue expose déjà sous `cle_domaine` et que le MCP appelle `cle_domaine`.

`date_debut` et `date_fin` servent l'équivalence « en vigueur à une date » : le MCP calcule
aujourd'hui `etat_applicable` sur les dates, jamais sur `etat`. Elles servent aussi à filtrer un
article à une date donnée (droit applicable à l'époque des faits).

### `jorf`

| Champ | Type | Niveau | Source |
|---|---|---|---|
| `fonds` | `keyword` | document | `legi.articles.fonds` (toujours `JORF`) |
| `etat` | `keyword` | document | `NULL::text` dans la vue (toujours null) |
| `annee` | `integer` | document | `vec.annee(COALESCE(CASE WHEN year(date_publi) BETWEEN 1700 AND 2030 THEN date_publi END, CASE WHEN year(date_texte) BETWEEN 1700 AND 2030 THEN date_texte END))` du texte parent |
| `nature` | `keyword` | document | `legi.texte_versions.nature` (`cle_domaine` de la vue) |
| `num` | `keyword` | document | `legi.articles.num` |
| `texte_cid` | `keyword` | document | `legi.articles.texte_cid` |

`etat` est déclaré bien qu'il soit toujours null : la recherche lexicale `articles` accepte ce
filtre sur tout `legi.articles`, JORF compris, et doit rendre zéro résultat plutôt qu'échouer sur
un champ inconnu. Les comparaisons ne sont jamais satisfaites par un null (`docs/05`), ce qui
reproduit le SQL actuel (`a.etat = ANY(p_etat)` sur une colonne nulle). La recherche sémantique,
elle, refuse `etat` sur `jorf_normatif` (`filtre_etat = false`) : l'adaptateur doit conserver ce
refus avant de construire le filtre.

### `decisions`

| Champ | Type | Niveau | Source |
|---|---|---|---|
| `section` | `keyword` | document | `juri.decisions.section` (`fonds` de la vue) |
| `juridiction` | `keyword` | document | `juri.decisions.juridiction` (`cle_domaine` de la vue) |
| `formation` | `keyword` | document | `juri.decisions.formation` |
| `nature` | `keyword` | document | `juri.decisions.nature` |
| `numero` | `keyword` | document | `juri.decisions.numero` (`num` de la vue) |
| `annee` | `integer` | document | `vec.annee(date_decision)` |
| `date_decision` | `date` | document | `juri.decisions.date_decision` |
| `partie` | `keyword` | **passage** | `partie` du découpage `decouper_juri` |

`partie` est le seul champ de passage des six index. Il porte les cinq valeurs filtrables
(`faits`, `moyens`, `motifs`, `dispositif`, `indetermine`). `entete` n'est jamais encodé et
n'apparaît donc pas.

Les deux corpus `juri_*` partagent ce schéma : une décision du sommaire n'a qu'un passage, sans
partie (`partie` null), et une décision du texte intégral a plusieurs passages, chacun avec sa
partie. Les deux sont dans le même index parce que `docs/02` les valide ensemble ; le découpeur
réellement appliqué diffère selon le corpus du document, tandis que le manifeste du lot ne déclare
qu'un `chunker` (voir « Vue d'ensemble »).

### `titres`

| Champ | Type | Niveau | Source |
|---|---|---|---|
| `fonds` | `keyword` | document | `legi.texte_versions.fonds` |
| `nature` | `keyword` | document | `legi.texte_versions.nature` (`cle_domaine` de la vue) |
| `etat` | `keyword` | document | `legi.texte_versions.etat` |
| `annee` | `integer` | document | `vec.annee(COALESCE(date_texte, date_publi))` |
| `num` | `keyword` | document | `legi.texte_versions.num` |
| `texte_cid` | `keyword` | document | `legi.texte_versions.cid` |
| `date_publi` | `date` | document | `legi.texte_versions.date_publi` |

`date_publi` porte les filtres de dates de la recherche lexicale `textes` du MCP, qui borne
`t.date_publi`, et non `date_debut`/`date_fin`. Le SQL applique en plus une fenêtre de
plausibilité `1800-01-01` à `2999-01-01` exclus, mais seulement si une borne de date est fournie
(`NOT v_borne OR …`). L'adaptateur ajoute ces deux bornes au filtre dans ce cas ; elles ne sont
pas des champs.

Comme `jorf_normatif`, `titres_textes` a `filtre_etat = false` : la recherche sémantique refuse
`etat` sur cet index. Le champ est déclaré pour la recherche lexicale `textes`, qui le filtre.
L'adaptateur doit conserver ce refus avant de construire le filtre.

### `sarde`

| Champ | Type | Niveau | Source |
|---|---|---|---|
| `theme` | `keyword` | document | `refs.sarde_blocs.libelleparent` (`cle_domaine` de la vue) |
| `nature_texte` | `keyword` | document | `refs.sarde_blocs.nature_texte` |

`nature_texte` est un texte libre et bruyant dans la source, mais le MCP le filtre par égalité
exacte (`p_nature_texte text`) : un `keyword` est donc fidèle.

### `dole`

| Champ | Type | Niveau | Source |
|---|---|---|---|
| `type` | `keyword` | document | `refs.dole_dossiers.type` (`cle_domaine` de la vue) |
| `annee` | `integer` | document | `vec.annee(date_creation)` |
| `date_creation` | `date` | document | `refs.dole_dossiers.date_creation` |

Les filtres de dates du MCP bornent `date_creation`. Comme pour `textes`, le SQL applique une
fenêtre de plausibilité `1800-01-01` à `2999-01-01` exclus, mais seulement si une borne de date
est fournie (`NOT v_borne OR …`). L'adaptateur ajoute ces deux bornes au filtre dans ce cas ;
elles ne sont pas des champs.

## `cluster_by` et sa justification

`cluster_by` trie physiquement les passages à la compaction. Un filtre sur ce champ parcourt alors
une zone contiguë (`docs/03`). On choisit le filtre le plus sélectif et le plus fréquent de chaque
index, pas un filtre décoratif.

| Index | `cluster_by` | Justification |
|---|---|---|
| `articles` | `["code"]` | Le nom du code est le premier filtre d'un juriste (commentaire de la vue sur `cle_domaine`). `fonds` n'a que trois valeurs et ne gagnerait presque rien. |
| `jorf` | `["annee"]` | JORF n'a pas d'`etat` : la période est le filtre discriminant, et un intervalle d'années devient contigu. |
| `decisions` | `["juridiction"]` | Filtre principal de la jurisprudence, et exemple de `docs/04`. |
| `titres` | `["nature"]` | `fonds` n'a que trois valeurs et `etat` est rare sur les titres JORF ; `nature` porte le filtre de type de texte. |
| `sarde` | `["theme"]` | `theme` porte le filtre du périmètre `index_thematique` ; `nature_texte` est un texte libre très peu sélectif. |
| `dole` | `["type"]` | Filtre principal du corpus ; `annee` reste un intervalle peu sélectif. |

Ces valeurs sont réglables par `PATCH` et ne prennent effet qu'à la compaction suivante
(`docs/04`). Ce ne sont donc pas des réglages figés.

## Gabarits : gardés, aucun changé

Les six gabarits restent ceux de la production actuelle :

| Index | Gabarit | Contenu |
|---|---|---|
| `articles`, `jorf` | `article_chemin_num_texte` v1 | chemin hiérarchique, numéro, texte, joints par `\n\n` |
| `decisions` | `juri_titre_corps` v1 | titre, puis corps (sommaire ou texte intégral) |
| `titres` | `titre_seul` v1 | le titre du texte |
| `sarde` | `titre_corps` v1 | titre, puis corps |
| `dole` | `titre_expose_motifs` v1 | titre, puis exposé des motifs détagué |

**Raison de ne rien changer** : `EXPORTER-BASE-LEGIFACILE` reprend les vecteurs pgvector sans
ré-encodage et vérifie chaque texte recomposé contre le `src_hash` existant. Changer un gabarit
change le texte composé, donc le `src_hash`, donc les vecteurs : il faudrait ré-encoder, ce que la
tâche d'export interdit. Un gabarit ne changera qu'après une décision explicite et une
ré-vectorisation complète.

## Réponse à Q-03 : aucun champ multivalué n'est nécessaire

**Décision : ADR-0137** (« Aucun champ multivalué en v1 (réponse à Q-03) », accepté par
anticipation le 2026-11-03). `docs/01:56` et `docs/04:41` renvoient désormais à cet ADR : `keyword`
reste scalaire, et le format de lot v1 n'est pas étendu.

Aucun des filtres du MCP n'a besoin d'une liste. Tous les champs filtrés sont scalaires par
document ou par passage dans la production actuelle :

- `fonds`, `etat`, `section`, `type`, `theme`, `nature`, `nature_texte`, `juridiction`,
  `formation`, `code` et `texte_cid` viennent d'une colonne scalaire de la vue ou de la table
  source ;
- `cle_domaine` de la vue des articles vaut `a.contexte -> 'titre_txt' -> 0 ->> 'text'` : la
  flèche `-> 0` ne retient que le premier titre, donc la valeur est scalaire, et un article ne
  peut pas porter plusieurs codes dans le corpus vectoriel actuel ;
- `partie` est scalaire par passage ;
- `blocs_sarde.nature_texte` est scalaire jusque dans le MCP (`"text"` dans `DOMAINES`) : l'index
  `sarde` n'a pas de filtre de type liste à couvrir.

Les paramètres du MCP qui sont des **listes** (`fonds`, `etat`, `cle_domaine`, `partie_exclue`,
et les listes de `recherche_texte`) ne décrivent pas un document multivalué : ils décrivent un
**OU** (ou un **OU d'exclusion**) sur des valeurs scalaires. La grammaire de filtres de Crible
les exprime directement : `in` pour l'inclusion, `nin` pour l'exclusion (`docs/05`). `fonds:
["LEGI", "KALI"]` devient `{ "fonds": { "in": ["LEGI", "KALI"] } }`. `partie_exclue` demande un
`$or`, parce qu'un `nin` n'est jamais satisfait par un null (ADR-0035) et que les sommaires ont
`partie` null : `{ "$or": [ { "partie": { "nin": ["moyens", "faits"] } }, { "partie": { "exists": false } } ] }`.

Les tableaux de la source qui ne sont pas des filtres du MCP restent hors du schéma :

- `legi.articles.mots_cles` et `legi.texte_versions.mots_cles` sont des `text[]` qu'aucun filtre
  du MCP n'expose ;
- `refs.dole_dossiers.legislature` est un `jsonb` (un objet, pas un tableau), et aucun filtre ne
  l'expose.

Les ajouter rouvrirait Q-03 sans besoin exprimé (ADR-0137, précision 6).

**Conclusion** : Q-03 est tranchée par ADR-0137. Aucune extension du format n'est demandée.

## Correspondance des filtres du MCP

### Recherche sémantique (`recherche_semantique`)

| Filtre MCP | Équivalent Crible | Remarque |
|---|---|---|
| `perimetre`, `corpus` | choix de l'index (recherche multi-index) | Ce n'est pas un filtre de document : `droit_applicable` interroge `articles`, `jurisprudence` interroge `decisions`, etc. |
| `fonds` | `{ "fonds": { "in": [...] } }` ; `section` sur `decisions` | `fonds` de la vue des décisions porte la section. |
| `etat` | `{ "etat": { "in": [...] } }` | Égalité exacte, comme `p_etat text[]`. |
| `etat` → dates | `date_debut` / `date_fin` | Le MCP calcule `etat_applicable` sur les dates, jamais sur `etat` : `date_debut <= aujourd'hui < date_fin`. |
| `annee_min`, `annee_max` | `{ "annee": { "gte": n } }`, `{ "annee": { "lte": n } }` | Champ `integer`. |
| `cle_domaine` | `code` (`articles`), `nature` (`jorf`, `titres`), `juridiction` (`decisions`), `theme` (`sarde`), `type` (`dole`), avec `in` | La vue appelle `cle_domaine` une colonne dont le sens dépend du corpus ; le schéma Crible lui donne le nom métier de chaque index. |
| `dedupliquer: "aucune"` | aucun `group_by`, aucun `dedup` | |
| `dedupliquer: "document"` | `group_by: "document"` | Meilleur passage par document. |
| `dedupliquer: "contenu"` | `dedup: "content"` | Voir l'écart ci-dessous. |
| `partie_exclue` | `{ "$or": [ { "partie": { "nin": [...] } }, { "partie": { "exists": false } } ] }` | Un `nin` seul écarterait les passages à `partie` null (sommaires) ; voir ci-dessous. |
| `k` | `k` | |

`partie_exclue` mérite une précaution. Un `nin` n'est jamais satisfait par un null (`docs/05:227`,
ADR-0035) : un simple `nin` écarterait **tous** les passages de `juri_sommaires`, dont `partie`
vaut null, alors que le SQL les sert par son terme `p.partie IS NULL`. Le `$or` ci-dessus les
conserve. L'adaptateur doit en outre envoyer explicitement la liste par défaut du MCP (`moyens`,
`faits`) : le SQL ne la pose pas lui-même (`outils.py:232-234`).

Deux écarts de **comportement**, pas de filtre manquant, restent à traiter dans `BRANCHER-MCP` :

1. **`partie_exclue` et le garde-fou document.** Le SQL ne se contente pas de servir les passages
   à `partie` null : quand **tous** les passages d'une décision tombent dans l'exclusion, un
   quatrième terme du `WHERE` laisse passer tous ses passages exclus, pour ne jamais retirer une
   décision entière. C'est `OR NOT EXISTS (SELECT 1 FROM vec.passages p2 WHERE p2.corpus =
   p.corpus AND p2.src_id = p.src_id AND (p2.partie IS NULL OR NOT (p2.partie = ANY($8))))`
   (`20260821110000_api_recherche_semantique_partie.sql:328-333`, Ruling 5 et §6.1bis). Le
   garde-fou agit au niveau du **document** ; il ne range pas la décision sous `indetermine`. Un
   `nin` sur `partie`, même enveloppé du `$or exists: false` ci-dessus, ne sait pas l'exprimer :
   le `$or` règle les sommaires, pas ce cas, et la grammaire des filtres n'a aucune condition au
   niveau du document. L'adaptateur devra soit accepter cet écart documenté, soit faire un second
   appel sans le filtre pour les documents entièrement exclus.
2. **`dedupliquer: "contenu"`.** Le SQL déduplique sur `vec.sources.src_hash`, l'empreinte du
   **document** entier. `dedup: "content"` de Crible fusionne les **passages** de contenu
   identique. C'est équivalent sur `titres` (un titre = un passage) et approchant ailleurs. À
   mesurer dans `COMPARER-AVEC-PGVECTOR` avant de conclure.

### Recherche lexicale (`recherche_texte`)

| Domaine MCP | Filtre MCP | Équivalent Crible |
|---|---|---|
| `articles` | `fonds`, `etat`, `texte_cid` | index `articles` (`fonds` LEGI/KALI) ou `jorf` (`fonds` JORF), mêmes champs |
| `textes` | `fonds`, `nature`, `etat` | index `titres`, champs de même nom |
| `textes` | `date_debut`, `date_fin` | `{ "date_publi": { "gte": … } }`, `{ "date_publi": { "lte": … } }`, plus les bornes de plausibilité 1800–2999 si une borne est fournie |
| `decisions` | `section`, `nature`, `juridiction`, `formation` | index `decisions`, champs de même nom |
| `decisions` | `date_debut`, `date_fin` | `{ "date_decision": { "gte": … } }`, `{ "date_decision": { "lte": … } }` |
| `blocs_sarde` | `nature_texte` | index `sarde`, champ `nature_texte` |
| `dole` | `type` | index `dole`, champ `type` |
| `dole` | `date_debut`, `date_fin` | `{ "date_creation": { "gte": … } }`, `{ "date_creation": { "lte": … } }`, plus les bornes de plausibilité 1800–2999 si une borne est fournie |

Le domaine MCP `articles` couvre tous les `legi.articles`, JORF compris. L'adaptateur route donc
selon `fonds` : `LEGI`/`KALI` vers l'index `articles`, `JORF` vers l'index `jorf`. Sans filtre
`fonds`, il interroge les deux.

**Filtres sans équivalent** : aucun, pour les filtres de recherche. Les outils `lire`, `versions`
et `liens` ne sont pas des filtres de recherche : `lire` et `versions` sont des lectures par
identifiant, et `liens` (sens et types d'arêtes) est une navigation sur `legi.liens` et
`juri.decision_liens`, hors du modèle d'index de la v1. Ils ne relèvent pas de cette tâche.

### Périmètres

| Périmètre MCP | Index Crible | Filtre ajouté |
|---|---|---|
| `droit_applicable` | `articles` | `date_debut <= aujourd'hui` et `date_fin > aujourd'hui` |
| `droit_en_vigueur` | `articles` | `etat in` les valeurs documentées `VIGUEUR`, `VIGUEUR_DIFF`, `VIGUEUR_ETEN`, `VIGUEUR_NON_ETEN` (`etat LIKE 'VIGUEUR%'`) |
| `droit_historique` | `articles` | `$or` de `etat nin` ces quatre valeurs et de `etat exists: false` |
| `jurisprudence` | `decisions` | aucun |
| `journal_officiel` | `jorf` | aucun |
| `index_thematique` | `sarde` | aucun |
| `dossiers_legislatifs` | `dole` | aucun |
| `titres` | `titres` | aucun |

Le périmètre `droit_en_vigueur` s'appuie sur la liste fermée des valeurs `VIGUEUR*` documentées
par les DTD LEGI et KALI, puisque Crible n'a pas d'opérateur de préfixe. L'adaptateur doit porter
cette liste, et non la deviner.

## Champs absents des vues actuelles

Les six configurations sont valides pour `crible-core`. Certains champs ne sont pas encore exposés
par les vues `vec.src_*` : `EXPORTER-BASE-LEGIFACILE` devra les ajouter à la vue (ou les lire sur
la table source) avant d'écrire les lots.

| Index | Champ | Colonne source | Vue actuelle |
|---|---|---|---|
| `articles` | `date_debut`, `date_fin` | `legi.articles.date_debut`, `date_fin` | absents |
| `decisions` | `formation`, `nature`, `date_decision` | `juri.decisions` | absents |
| `titres` | `date_publi` | `legi.texte_versions.date_publi` | absent |
| `sarde` | `nature_texte` | `refs.sarde_blocs.nature_texte` | absent |
| `dole` | `date_creation` | `refs.dole_dossiers.date_creation` | absent |

Tous les autres champs sont déjà exposés par la vue correspondante. Ces ajouts sont des colonnes
supplémentaires dans `vec.src_*` ; ils ne changent ni le périmètre des vues ni les textes composés,
donc ni les `src_hash` ni les vecteurs.

## Validation du propriétaire

Déléguée par anticipation le 2026-11-03, à confirmer.
