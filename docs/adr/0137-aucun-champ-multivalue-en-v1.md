# ADR-0137 — Aucun champ multivalué en v1 (réponse à Q-03)

- **Statut** : Accepté (par anticipation le 2026-11-03, sur délégation du propriétaire du 2026-11-03 ; à confirmer par le propriétaire le 2026-11-04 au matin)
- **Date** : 2026-11-03 (proposition et acceptation anticipée)
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, tâche `DEFINIR-INDEX-LEGIFACILE` (phase P10), à la demande de `@relecteur` au premier tour de relecture (point bloquant 1)
- **Remplace** : — (tranche Q-03 ; complète ADR-0030 et ADR-0035 sans modifier leur corps)

La délégation du propriétaire est citée telle quelle, le 2026-11-03 : « on part sur l'option B » (option qui déléguait par anticipation la validation des six configs, celle du jeu de référence et l'acceptation de l'ADR de Q-03) et « si tu doit ecrire des ADR considere qu'il sont accepter par anticipation, on les valide ensemble demain matin ». L'acceptation anticipée porte sur cet ADR seul ; le propriétaire le confirme ou le rejette le 2026-11-04.

## Contexte

### La question posée et ce qui l'a fait rouvrir

Q-03 figure depuis l'origine dans les questions ouvertes de `docs/adr/README.md` : « Champs multivalués (`keyword[]`) nécessaires pour Legifacile ? » Avant cet ADR, `docs/04-donnees-et-ingestion.md:41` la traduisait en règle — « Les champs multivalués sont hors v1 (Q-03) » —, et `docs/01-vision-et-perimetre.md:56` la rangeait au hors-périmètre sous la mention « Question ouverte Q-03 » ; ces deux lignes renvoient désormais à cet ADR (voir « Documents mis à jour par cet ADR »). Le schéma ne connaît donc qu'un `keyword` scalaire, et `docs/03-recherche.md:27` lui associe les opérateurs `eq`, `ne`, `in`, `nin`, `exists`.

La tâche `DEFINIR-INDEX-LEGIFACILE` devait répondre à la question dans son document de travail, `docs/legifacile/index-legifacile.md` (`:210-246`). `@relecteur`, au premier tour, a jugé le point bloquant : Q-03 y était tranchée sans ADR, alors que l'index des questions ouvertes annonce que « chacune deviendra un ADR quand elle sera tranchée ». Cet ADR est la réponse à ce point.

### Ce que disent les sources, vérifié pièce par pièce

La production Legifacile a été relue en lecture seule, dans `~/MattCorp/legifacile-sandbox`.

**Les filtres exposés par le MCP.** `serveur_mcp/outils.py` déclare, dans `DOMAINES` (`:244-263`), le type SQL de chaque filtre. Pour `articles`, `fonds`, `etat` et `texte_cid` sont des `text[]` ; pour `textes`, `fonds`, `nature` et `etat` ; pour `decisions`, `section`, `nature`, `juridiction` et `formation` ; pour `dole`, `type`. Seul `blocs_sarde.nature_texte` est un `text` scalaire. Un `text[]` reçu par `_valider_valeur_filtre` (`:352-398`) est passé tel quel, une chaîne seule est enveloppée, et une liste envoyée à un filtre `text` est refusée explicitement.

**Les colonnes lues par les vues `vec.src_*`.** Les huit vues de `supabase/migrations/20260818160000_vec_src_maj_publication.sql` ne lisent que des colonnes scalaires :

- `src_textes_vigueur` et `src_textes_hors_vigueur` : `a.fonds`, `a.etat`, `a.texte_cid`, `vec.annee(a.date_debut)` pour `annee`, et `a.contexte -> 'titre_txt' -> 0 ->> 'text'` pour `cle_domaine` (`:62`, `:72`). La flèche `-> 0` ne retient que le **premier** titre du tableau `titre_txt` : `cle_domaine` est donc scalaire, et un article ne peut pas porter plusieurs codes dans le corpus vectoriel actuel ;
- `src_jorf_normatif` : `tvp.nature` pour `cle_domaine` ; `src_titres_textes` : `tv.nature` ;
- `src_juri_sommaires` et `src_juri_texte_integral` : `d.section` pour `fonds`, `d.juridiction` pour `cle_domaine` ;
- `src_sarde` : `b.libelleparent` pour `cle_domaine` ;
- `src_dole` : `d.type` pour `cle_domaine`.

**`partie` est scalaire par passage.** La colonne est ajoutée à `vec.passages` par `supabase/migrations/20260820120000_vec_passages_partie.sql:67` (`ALTER TABLE vec.passages ADD COLUMN IF NOT EXISTS partie text`). Elle porte un vocabulaire fermé (`entete`, `faits`, `moyens`, `motifs`, `dispositif`, `indetermine`) et vaut `NULL` pour les corpus non segmentés.

**Les tableaux de la source qui ne sont pas des filtres.** `legi.articles.mots_cles` et `legi.texte_versions.mots_cles` sont des `text[]` ; aucun filtre du MCP ne les expose. `refs.dole_dossiers.legislature` est un `jsonb` et non un tableau (voir les corrections ci-dessous) ; aucun filtre du MCP ne l'expose non plus.

**La grammaire de Crible suffit.** `docs/05-api.md:224` donne à `keyword` les opérateurs `eq`, `ne`, `in`, `nin` et `exists`. Un `in` sur un champ scalaire exprime exactement un OU d'inclusion sur des valeurs, et un `nin` un OU d'exclusion. `fonds: ["LEGI", "KALI"]` s'écrit `{ "fonds": { "in": ["LEGI", "KALI"] } }`. `partie_exclue` demande la forme `$or` de la précision 5, parce qu'un `nin` seul écarte aussi les passages à `partie` null ; cette forme ne suffit pas pour autant, le garde-fou au niveau du document restant hors de portée de la grammaire.

### Corrections apportées aux affirmations transmises

Trois affirmations de la demande de rédaction étaient inexactes ou incomplètes. Aucune ne change la décision.

1. `legislature` de `refs.dole_dossiers` n'est **pas** un tableau : la colonne est un `jsonb` (`20260724170000_schema_refs.sql:69`) qui porte un objet `{numero, libelle, date_debut, date_fin}`. C'est donc une structure composée, non un `keyword[]`, et elle reste hors du schéma pour la même raison qu'un tableau : aucun filtre du MCP ne l'expose.
2. `mots_cles` existe **deux fois** : sur `legi.articles` (`20260723150100_schema_legi.sql:151`) et sur `legi.texte_versions` (`:37`), les deux en `text[]`. Le document de travail ne citait que la première.
3. `blocs_sarde.nature_texte` est scalaire jusque dans le MCP (`"text"` dans `DOMAINES`) : l'index `sarde` n'a même pas de filtre de type liste à couvrir. Cela renforce la conclusion au lieu de l'affaiblir.

## Décision

**Aucun champ multivalué (`keyword[]`) n'est nécessaire pour Legifacile en v1. Le schéma, le format de lot v1 et la grammaire de filtres ne changent pas.**

Précisions :

1. **`keyword` reste scalaire, un champ porte une valeur ou rien.** Le comportement des valeurs nulles d'ADR-0035 s'applique sans exception : un `in` ou un `nin` n'est jamais satisfait par un null.
2. **Les listes du MCP deviennent des OU de scalaires.** `fonds`, `etat`, `cle_domaine`, `partie_exclue` et les listes de `recherche_texte` sont exprimées par `in` (inclusion) et `nin` (exclusion), sur le nom métier du champ dans chaque index. La correspondance complète appartient à `docs/legifacile/index-legifacile.md` et à la tâche `BRANCHER-MCP`.
3. **Les six configs d'index n'ont aucun champ multivalué**, et toutes les colonnes filtrées qu'elles déclarent sont scalaires dans les vues `vec.src_*` relues ci-dessus.
4. **Aucune extension du format de lot v1.** `docs/format-lot-v1.md` reste inchangé : un champ `keyword` est une colonne UTF-8, pas une liste.
5. **Ce qui n'est pas décidé ici** : la mise en œuvre de cette correspondance dans `BRANCHER-MCP`, et la mesure de l'écart de `dedupliquer: "contenu"`. Deux écarts de comportement portent sur `partie_exclue`, et le `$or` n'en règle qu'un.
   - **Les passages à `partie` null.** Un `nin` n'est jamais satisfait par un null (ADR-0035) : employé seul, il écarterait les sommaires, que le SQL du MCP sert par son terme `p.partie IS NULL` (`supabase/migrations/20260821110000_api_recherche_semantique_partie.sql:328`). Le document de travail retient donc `{ "$or": [ { "partie": { "nin": [...] } }, { "partie": { "exists": false } } ] }`, qui les conserve.
   - **Le garde-fou au niveau du document.** Quand **tous** les passages d'une décision tombent dans l'exclusion, un quatrième terme du `WHERE` laisse passer tous ses passages exclus, pour ne jamais retirer une décision entière : `OR NOT EXISTS (SELECT 1 FROM vec.passages p2 WHERE p2.corpus = p.corpus AND p2.src_id = p.src_id AND (p2.partie IS NULL OR NOT (p2.partie = ANY($8))))` (`:328-333` ; « RULING 5 » `:114-136`, §6.1bis de la spécification de segmentation). Le `$or` ne l'exprime pas : la grammaire des filtres n'a aucune condition au niveau du document. `BRANCHER-MCP` devra soit accepter cet écart documenté, soit faire un second appel sans le filtre pour les documents entièrement exclus.
   Enfin, l'écart de `dedupliquer: "contenu"` : le SQL déduplique sur `vec.sources.src_hash`, l'empreinte du **document** entier, alors que `dedup: "content"` de Crible fusionne les **passages** de contenu identique ; il est à mesurer dans `COMPARER-AVEC-PGVECTOR` avant de conclure. Aucune de ces questions n'est un type de schéma, et aucune ne rouvre Q-03.
6. **Le sujet se rouvre par un nouvel ADR**, jamais par un assouplissement silencieux du schéma, si un projet demande de vraies listes — par exemple une facette à valeurs multiples ou un filtrage sur `mots_cles`.

### Documents mis à jour par cet ADR

- `docs/adr/README.md` : ligne d'index d'ADR-0137 ; Q-03 retirée des questions ouvertes et ajoutée aux « Questions tranchées » avec renvoi à cet ADR.
- `docs/01-vision-et-perimetre.md:56` : la ligne « Champs multivalués » renvoie à ADR-0137 au lieu de « Question ouverte Q-03 ».
- `docs/04-donnees-et-ingestion.md:41` : le renvoi devient ADR-0137.
- `docs/legifacile/index-legifacile.md` (`:210-246`) : le renvoi à cet ADR est ajouté par `@dev`, dans le travail de `DEFINIR-INDEX-LEGIFACILE` ; `@greffier` n'y touche pas.

## Conséquences

- **Positives** :
  - Q-03 est tranchée là où l'index des questions ouvertes le demande, avec ses sources citées et vérifiées ; le document de travail de `DEFINIR-INDEX-LEGIFACILE` peut s'appuyer sur un ADR `Accepté` ;
  - aucun code, aucun format et aucune dépendance ne changent : la décision confirme l'état livré, elle ne demande pas de travail ;
  - la grammaire de filtres reste petite — cinq opérateurs pour `keyword` —, et les listes du MCP se traduisent sans mécanisme nouveau ;
  - la perte par rapport à pgvector est nulle sur le corpus actuel, puisque `cle_domaine` n'y retient que le premier titre.
- **Négatives / coûts** :
  - un besoin réel de listes — facettes à valeurs multiples, filtrage sur `mots_cles` — coûtera un nouvel ADR, une extension du format de lot v1, des bitmaps par valeur et par position, et une ré-indexation ;
  - deux écarts de `partie_exclue` restent à traiter dans `BRANCHER-MCP` (précision 5) : la forme `$or` règle les passages à `partie` null, mais le garde-fou au niveau du document n'est pas exprimable par la grammaire des filtres ; l'écart de `dedupliquer: "contenu"` reste à mesurer ;
  - `legislature` reste hors du schéma : un futur filtre « par législature » demandera une dérivation scalaire chez le producteur, comme `cle_domaine`.
- **À surveiller** :
  - un filtre du MCP qui viendrait à porter sur un tableau de la source sans pouvoir s'exprimer en `in` ou `nin` — c'est le signal exact qui rouvrirait Q-03 ;
  - une évolution de la vue des articles qui remplirait `titre_txt` de plusieurs titres : `cle_domaine` resterait scalaire dans Crible, donc le second code serait perdu, et le document de travail devrait le dire ;
  - le garde-fou document de `partie_exclue`, non exprimable par la grammaire, et l'écart de `dedupliquer: "contenu"` de la précision 5, mesurés dans `COMPARER-AVEC-PGVECTOR`.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| **Ajouter `keyword[]` au schéma et au format de lot** | Aucun filtre du MCP ne le demande : les huit vues ne produisent que des scalaires, et les listes d'entrée sont des OU. Ce serait un type, un encodage Parquet, un stockage de bitmaps multi-valeurs et une validation de plus, pour zéro usage identifié. |
| **Ajouter un champ multivalué seulement pour `mots_cles`** | `legi.articles.mots_cles` et `legi.texte_versions.mots_cles` ne sont exposés par aucun filtre du MCP. Ajouter un type pour une colonne que personne n'interroge rouvrirait Q-03 sans besoin exprimé, et figerait le choix dans le format de lot v1. |
| **Exprimer les listes du MCP par `$or` de `eq`** | `in` et `nin` existent déjà (`docs/05:224`) et disent la même chose en une clause. Un `$or` de `eq` coûterait une profondeur de filtre (ADR-0029) pour un résultat identique. |
| **Dériver `cle_domaine` en tableau de codes pour préserver les titres suivants** | La vue ne retient que `titre_txt[0]` ; Crible ne perd donc rien par rapport à pgvector. Changer la dérivation serait une décision de production Legifacile, hors de ce moteur, et exigerait de toute façon un type multivalué. |
| **Répondre à Q-03 dans le seul document de travail de la tâche** | C'est l'état que `@relecteur` a jugé bloquant : l'index des questions ouvertes promet un ADR par question tranchée. Une décision qui ne vit que dans une fiche de tâche est perdue à la session suivante. |
| **Laisser Q-03 ouverte et ne rien décider** | Le document de travail devait y répondre pour livrer six configs sans champ multivalué, et `docs/01:56` et `docs/04:41` renvoient déjà à la question. Laisser ouvert laisserait la fiche dépendre d'une question non tranchée, ce que la règle des ADR interdit. |

## Références

- Question : `docs/adr/README.md:196-214` (questions ouvertes et questions tranchées).
- Conception : `docs/01-vision-et-perimetre.md:56` (hors-périmètre de la v1), `docs/04-donnees-et-ingestion.md:38-41` (types de champ, multivalué hors v1), `docs/03-recherche.md:27` (opérateurs de `keyword`), `docs/05-api.md:224` (grammaire des filtres), `docs/format-lot-v1.md:274` (colonne `keyword` en `BYTE_ARRAY` UTF-8).
- Travail d'origine : `docs/legifacile/index-legifacile.md:210-246` (réponse à Q-03) et `:272-289` (les deux écarts de comportement), et la tâche `DEFINIR-INDEX-LEGIFACILE` de `docs/phases/P10-validation-a-l-echelle-reelle.md`.
- Sources relues en lecture seule dans `~/MattCorp/legifacile-sandbox` : `serveur_mcp/outils.py:244-263` (`DOMAINES`) et `:352-398` (`_valider_valeur_filtre`) ; `supabase/migrations/20260818160000_vec_src_maj_publication.sql:59-162` (les huit vues `vec.src_*`) ; `supabase/migrations/20260820120000_vec_passages_partie.sql:67` (`partie` scalaire) ; `supabase/migrations/20260821110000_api_recherche_semantique_partie.sql:328-333` (terme du garde-fou document) et `:114-136` (« RULING 5 ») ; `supabase/migrations/20260723150100_schema_legi.sql:37` et `:151` (`mots_cles`) ; `supabase/migrations/20260724170000_schema_refs.sql:69` (`legislature` en `jsonb`).
- ADR : ADR-0030 (un nom de champ déclaré à un seul niveau), ADR-0035 (sémantique des valeurs nulles), ADR-0129 (replanification de la fin du plan, qui place `DEFINIR-INDEX-LEGIFACILE` en P10).
