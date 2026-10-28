# ADR-0091 — Protocole de MES-06 (mesure de la branche lexicale)

- **Statut** : Accepté (accepté par anticipation par le propriétaire le 2026-10-28, à confirmer à la fin de la série MESURER-LEXICAL, MESURER-HYBRIDE)
- **Date** : 2026-10-28 (proposition et acceptation anticipée)
- **Décideurs** : propriétaire
- **Décision** : accepté par anticipation le 2026-10-28, par délégation du propriétaire pour la série MESURER-LEXICAL et MESURER-HYBRIDE, à confirmer à la fin de la série. Les six décisions sont retenues telles que proposées : taille et durée de construction mesurées sur l'échantillon Legifacile seulement, extrapolées aux 8,58 M passages linéairement en jetons analysés et données en fourchette, `synth-large` exclu de ces deux chiffres ; « avec et sans positions » vaut les deux valeurs du booléen `lexical.positions`, `false` en campagne principale et `true` en série secondaire sur un seul point de pool ; deux campagnes de pools et de requêtes, l'une réelle sur l'échantillon (1 000, 10 000 et 50 000 passages, 300 requêtes de `queries.jsonl`, `k = 10` principal et `k = 100` secondaire, 3 passes), l'autre de plomberie sur `synth-large` (537 k, 1 M, 2,15 M et 7,74 M) dont les résultats sont déclarés pire cas synthétique ; un seuil d'alerte unique par soustraction des mesures déjà faites au budget de 698 ms ; un périmètre obligatoire qui couvre le pic RSS du commit, le nombre de segments, la reconstruction à 8,58 M et le constat block-WAND ; un harnais hors production dans `crates/crible/examples/` et `tools/bench/mes06/`, sans changement de l'API publique.
- **Proposé par** : agent greffier, sur demande de l'orchestrateur, pour la tâche `MESURER-LEXICAL` (phase P5), à partir des verdicts de `@architecte` du 2026-10-28 (six points NON-COUVERTS, un ECART de prémisse)
- **Remplace** : —

## Contexte

### Ce que la fiche demande

La fiche `MESURER-LEXICAL` (`docs/phases/P5-lexical-et-hybride.md:95-103`) demande MES-06. Ses livrables sont « la taille de l'index, avec et sans positions » (`:99`), « la durée de construction, mesurée sur l'échantillon et extrapolée aux 8,58 M passages » (`:100`) et « la latence BM25 filtrée selon la taille du pool » (`:101`). Son critère est : « un rapport reproductible, avec une alerte si le filtrage du pool est trop lent ; dans ce cas, proposer le repli de ADR-0007 » (`:102`).

La fiche ne dit ni quel jeu sert de base à chaque chiffre, ni comment extrapoler d'un échantillon de 257 626 passages à 8,58 M, ni ce que signifie exactement « avec et sans positions », ni comment obtenir des tailles de pool, ni quel nombre de requêtes et quelle répétition rendre les chiffres comparables, ni à partir de quel seuil le filtrage est « trop lent ». Ce sont des décisions de protocole : sans elles, deux mesures portant le même nom ne seraient pas comparables.

### Ce que la conception fixe

- **Le tableau de `docs/03` laisse la branche lexicale vide.** `docs/03-recherche.md:154` porte la ligne « Branche lexicale | à mesurer | MES-06 ». Les autres lignes du même tableau sont désormais chiffrées par MES-02, MES-03 et MES-14, ce qui rend la soustraction possible.
- **Les positions sont un booléen, désactivé par défaut.** `docs/03:88` écrit « Positions désactivées par défaut (`lexical.positions`) ». `crates/crible-core/src/config.rs:423-424` déclare `pub positions: bool`, et `docs/04-donnees-et-ingestion.md:25` montre la configuration `"lexical": { "enabled": true, "language": "fr", "positions": false }`. `docs/04:49` place `lexical.positions` parmi les réglages `PATCH` qui « déclenchent un job de reconstruction du lexical ».
- **Aucune configuration ne nomme `freq` ni `position`.** Ces deux valeurs sont internes à tantivy et apparaissent dans les tests du job de reconstruction, pas dans `index.json`. La fiche, en écrivant « avec et sans positions », suppose donc une configuration qui n'existe pas : c'est l'ECART de prémisse relevé par `@architecte`, et la décision 2 le lève.
- **tantivy reste à mesurer pour sa part mémoire.** `docs/02-architecture.md:221` porte la ligne « tantivy | disque, mmap | à mesurer (MES-06) ».
- **ADR-0007 attend un chiffre.** Sa section « À surveiller » (`docs/adr/0007-tantivy-pour-bm25.md:27`) demande MES-06 et écrit : « Si le filtrage est trop lent, repli sur un index inversé maison par segment (nouvel ADR) ». Le mot « trop lent » n'a aucun chiffre associé.
- **ADR-0087 étend MES-06 au commit.** Sa section « À surveiller » (`docs/adr/0087-reprise-lexicale-et-ouverture-en-lecture-seule.md:75`) demande de mesurer « le pic RSS du commit et le nombre de segments produits », et précise qu'un ADR n'est justifié que si le budget de l'`IndexWriter` (50 MiB, un thread) se révèle structurant.
- **ADR-0088 étend MES-06 à la reconstruction.** Sa section « À surveiller » (`docs/adr/0088-index-lexical-a-jour-et-lexical-illisible-a-la-reprise.md:155`) demande « la durée de la reconstruction à 8,58 M passages, à relever à la première occurrence (MES-06), et la cible de 60 s d'ADR-0047 ».
- **Le block-WAND est douteux par construction.** `docs/03:91` écrit, à propos du collecteur qui filtre sur le pool : « Avec ce collecteur, l'élagage block-WAND ne s'applique probablement pas (à mesurer, MES-06) ».
- **`synth-large` est réservé à la latence.** `docs/08-invariants-et-tests.md:55` décrit ce jeu comme « 8,6 M passages de vecteurs aléatoires normalisés : pour la latence seulement ». Son texte n'est pas du français : il ne peut donc servir ni à la taille d'index ni à la durée de construction d'un index réel.
- **La cible de bout en bout est de 698 ms p95.** `docs/suivi.md`, section « Cibles », fixe « Latence chaude mono-corpus `k = 10` | p95 ≤ 698 ms » (`:31`), contraignante seulement sur le Mac mini en conteneur, indicative ailleurs.

### Les questions tranchées ici

`@architecte` a rendu ces verdicts le 2026-10-28, avant de confier la mesure à `@mesureur` :

1. **Jeu et extrapolation** : **NON-COUVERT**. La fiche demande une extrapolation de l'échantillon à 8,58 M sans dire selon quelle grandeur, et `docs/08:55` exclut `synth-large` de tout ce qui n'est pas latence.
2. **Positions** : **NON-COUVERT**, avec un **ECART de prémisse**. « Avec et sans positions » suppose une configuration qui nomme `freq` et `position` ; le moteur n'expose que le booléen `lexical.positions`.
3. **Pools et requêtes** : **NON-COUVERT**. Aucun jeu ne fournit les tailles de pool demandées, ni ne fixe le nombre de requêtes, les passes, les percentiles ou `k`.
4. **Seuil de l'alerte** : **NON-COUVERT**. « Trop lent » n'a pas de chiffre, et ADR-0007 ne propose aucun critère.
5. **Périmètre** : **NON-COUVERT**. ADR-0087 et ADR-0088 ajoutent quatre objets à MES-06 (RSS du commit, segments, reconstruction, block-WAND) que la fiche ne nomme pas ; il faut dire ce qui est obligatoire et ce qui sort du périmètre.
6. **Harnais** : **NON-COUVERT**. Les pools arbitraires et les tailles de l'échantillon demandent du code qui n'est pas dans l'API publique ; le précédent de MES-01 et MES-02 doit être suivi ou amendé.

Aucune de ces questions ne se tranche par la lecture seule : elles fixent un plan d'expérience, une définition et un seuil. Ce sont des décisions.

### L'état des données au moment de la décision

- `data/legifacile-sample/` porte **257 626 passages** répartis en **six structures** (`journal-articles`, `journal-decisions`, `journal-dole`, `journal-jorf`, `journal-sarde`, `journal-titres`), avec un pool maximum d'environ **50 000 passages par structure**. Le jeu fournit **300 requêtes réelles** dans `queries.jsonl` (300 lignes). Ses configurations portent `"lexical": {"enabled": true, "language": "fr", "positions": false}` et **aucun index n'est attaché** : la campagne construit donc ses index.
- `data/synth-large/` porte **7 740 000 passages vivants** sur 8 600 000 (MES-07). Son texte est synthétique : `"<doc_id> passage <rank> "` suivi de six mots tirés d'un vocabulaire de seize mots (`tools/gen_synth.py:355-357` pour le texte, `:134-151` pour le vocabulaire). Cela donne **environ 9 jetons par passage**, contre **331 jetons mesurés sur `articles`**.
- Un texte de neuf jetons ne représente pas un passage de 331 jetons : la taille d'index et la durée de construction tirées de `synth-large` n'auraient aucun sens. En revanche, sa longueur de listes d'inversion est un pire cas utile (`df` ≈ 1/16), et ses 7,74 M passages restent le seul jeu local de cette taille.

## Décision

**La mesure de la branche lexicale a un protocole unique : taille et durée de construction sur le seul échantillon Legifacile, extrapolées en jetons analysés et données en fourchette ; « avec et sans positions » vaut les deux valeurs du booléen `lexical.positions` ; deux campagnes de latence, l'une réelle sur l'échantillon, l'autre de plomberie sur `synth-large` et déclarée comme telle ; un seuil d'alerte unique obtenu par soustraction au budget de 698 ms ; un périmètre obligatoire fixé, et un harnais hors production.**

Précisions :

1. **Jeu et extrapolation.** La **taille de l'index** et la **durée de construction** sont mesurées sur l'**échantillon Legifacile seulement**, **par structure**. L'extrapolation aux **8,58 M passages** est **linéaire en jetons analysés** : le harnais **compte les jetons indexés par structure**, et le rapport donne la taille et la durée **par passage** et **par jeton**. L'extrapolation est présentée en **fourchette** : taux minimal et taux maximal observés par structure, multipliés par 8,58 M passages au nombre moyen de jetons de la structure, puisque la **composition de production par structure est inconnue**. Un **second contrôle** est donné par le simple **ratio de passages** (8,58 M / 257 626). `synth-large` est **exclu** de la taille et de la durée : son texte de neuf jetons n'est pas celui de Legifacile.
2. **Positions.** « Avec et sans positions » désigne les **deux valeurs du booléen `lexical.positions`**, mesurées sur **les mêmes données** : `false` (le défaut, `docs/03:88`) et `true`, obtenu par le job **`rebuild_lexical`** (`docs/04:49`, `docs/05-api.md:254`). La configuration ne nomme **jamais** `freq` ni `position` : ce sont des valeurs internes de tantivy, pas des réglages d'index. Pour la **latence**, la campagne **principale** est menée en `false`, et une **série secondaire** en `true` porte sur **un seul point de pool**, **déclarée comme telle** : elle donne l'ordre de grandeur du surcoût, pas une courbe.
3. **Pools et requêtes.** Deux campagnes distinctes.
   - **(a) Échantillon, réelle.** Les pools sont obtenus par de **vrais filtres de l'échantillon** et par l'**index entier**, aux tailles **1 000, 10 000 et 50 000 environ**, la **taille réelle** étant écrite à côté du chiffre rond. Les **300 requêtes** de `queries.jsonl` sont rejouées, **chacune sur l'index de sa structure**. `k = 10` en principal, `k = 100` en secondaire. Chaque point suit un **échauffement** puis **3 passes**, et le rapport donne **p50, p95 et p99 au rang le plus proche**, avec **`n` sur chaque ligne**. Une **seule requête à la fois**, **nombre de threads rayon au défaut**.
   - **(b) `synth-large`, plomberie.** Les pools sont de **537 k environ** (une valeur de `categorie`), **1 M et 2,15 M environ** (unions de valeurs **non adjacentes**, donc **dispersées**) et **7,74 M** (tous les vivants). Les requêtes sont **synthétiques**, tirées du **vocabulaire de seize mots**. Le rapport écrit que **`df` ≈ 1/16 est un pire cas de longueur de listes**, **pas une mesure de production**, et que la **latence réelle à grand pool est renvoyée à P9**.
   - **Froid.** Une **seule série déclarée** : copie lue avec `F_NOCACHE` ou 10 min d'inactivité (précédents ADR-0077 et ADR-0084), sur **un point**, donnée **en borne**.
4. **Seuil de l'alerte.** Une **règle unique, par soustraction**. Le budget restant de la branche lexicale vaut **698 ms** (cible p95 de bout en bout, `docs/suivi.md`, section « Cibles ») moins la somme de : **encodage fp32 p95 (MES-05)**, plus **filtre (MES-14)**, plus **scan p95 (MES-02)**, plus **rescoring p95 (MES-03)**, plus **payload**. Chaque terme est **cité depuis son rapport** dans le rapport de MES-06, avec sa source ; l'ordre de grandeur attendu du reste est d'**environ 560 ms**. Il y a **alerte si la p95 de la branche lexicale dépasse ce reste à une taille de pool mesurée**. Sur `synth-large`, l'alerte est étiquetée « **pire cas synthétique, à confirmer en P9** ». Les chiffres sont **natifs et indicatifs jusqu'à P8** : seule la mesure sur le Mac mini en conteneur est contraignante. En cas d'alerte, le rapport **propose** le repli d'ADR-0007 (index inversé maison par segment), qui demanderait un **nouvel ADR** ; **rien n'est codé**.
5. **Périmètre.** Sont **obligatoires** :
   - le **pic RSS du commit** et le **nombre de segments tantivy produits** (ADR-0087), avec un **jugement sur le budget `IndexWriter` de 50 MiB et un thread** — un ADR n'est écrit que si ce budget se révèle **structurant** ;
   - la **durée de la reconstruction** par `rebuild_lexical`, extrapolée selon la décision 1 et comparée aux **60 s d'ADR-0047**, avec la mention « **première occurrence réelle en P9** » (ADR-0088) ;
   - le **constat block-WAND** (`docs/03:91`) : **nombre de candidats visités** contre **taille du pool**.
   Sont **optionnels, sur une passe** : le **coût du groupe d'égalité à la frontière du top-k** (`synth-large`, scores très liés) et le **pic RSS d'attache** dû au texte des passages créés gardé jusqu'à la publication.
   Sont **hors périmètre** : `KEY_READ_CONNECTIONS = 1` sous concurrence, renvoyé à `TESTER-CHARGE-CONCURRENTE` (P6) ; la **reconstruction pendant la compaction**, renvoyée à P9 avec MES-07.
6. **Harnais.** **Hors production**, comme MES-01 et MES-02 : un exemple Rust dans `crates/crible/examples/` et des scripts dans `tools/bench/mes06/`. Les **pools réels passent par l'API publique** (`Pool::evaluate`, `crible_search::lexical::search`) ; les **tailles arbitraires se répliquent** comme en MES-02 (**ADR-0077, décision 3**), et l'**API publique ne change pas**. Le rapport est `docs/mesures/MES-06-<slug>.md`, au format de `docs/mesures/README.md`, avec les **artefacts JSON datés à côté**, des **chemins relatifs**, et **rien de `.env.prod`**.

Ce qui n'est **pas** décidé ici : la valeur des réglages de tantivy ou de l'`IndexWriter`, qui appartient à un ADR séparé si la mesure le justifie ; le remplacement de tantivy, qui demanderait un nouvel ADR et n'est que proposé en cas d'alerte ; la latence réelle aux grands pools, renvoyée à P9 ; la concurrence de lecture et la reconstruction pendant la compaction, hors périmètre ; la fusion et la qualité, qui appartiennent à MESURER-HYBRIDE.

## Conséquences

- **Positives** :
  - la mesure de MES-06 devient reproductible : jeu, tailles, requêtes, passes, percentiles et `k` sont fixés ;
  - l'extrapolation est donnée en fourchette et doublée d'un contrôle par le ratio de passages, ce qui évite de présenter une projection comme une mesure ;
  - `synth-large` reste employé pour ce qu'il sait faire — la longueur des listes d'inversion — sans contaminer la taille ni la durée de construction ;
  - le seuil d'alerte se calcule à partir de mesures déjà faites, donc il ne s'invente pas, et chaque terme reste traçable jusqu'à son rapport ;
  - le périmètre obligatoire couvre les quatre demandes d'ADR-0087 et d'ADR-0088, et le constat block-WAND ferme la réserve de `docs/03:91` ;
  - le harnais reste hors production et l'API publique ne bouge pas.
- **Négatives / coûts** :
  - la campagne a deux jeux, deux valeurs de positions et plusieurs tailles de pool : le rapport est long et son harnais doit être versionné pour rester vérifiable ;
  - l'échantillon ne fournit que trois tailles de pool utiles, et sa taille maximale par structure est de 50 000 passages : aucune mesure réelle ne dépasse ce point ;
  - `synth-large` produit des chiffres de latence qui ne valent pas pour Legifacile, et le rapport doit le répéter à chaque tableau ;
  - le comptage de jetons analysés par structure ajoute une instrumentation au harnais, avec le risque d'un comptage qui ne suive pas exactement l'analyseur ;
  - le froid n'est mesuré que sur un point, en borne : il ne donne pas de courbe.
- **À surveiller** :
  - **la latence réelle aux grands pools reste inconnue jusqu'à P9** : les points de `synth-large` ne la remplacent pas, et la confirmation est due avec MES-07 ;
  - **le seuil se recalcule si une des mesures soustraites change** (MES-05, MES-14, MES-02, MES-03, payload) : un rapport publié avec un reste de 560 ms ne vaut que pour les chiffres de son époque ;
  - si l'écart entre la fourchette haute et la fourchette basse de l'extrapolation dépasse un facteur notable, la composition de production par structure devient une donnée à obtenir avant P9 ;
  - si le comptage de jetons du harnais s'écarte du comptage réel de l'analyseur, l'extrapolation en jetons perd sa base et la mesure devra être reprise ;
  - un pic RSS de commit qui approche le budget de l'`IndexWriter`, ou un nombre de segments qui croît avec les attaches, justifierait l'ADR que l'ADR-0087 mentionne ;
  - si la reconstruction extrapolée dépasse les 60 s d'ADR-0047, l'ordre des remèdes au temps de démarrage sera à revoir ; la confirmation vient de P9.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Mesurer la taille et la durée sur `synth-large` | Son texte fait neuf jetons contre 331 sur `articles` : la taille et la durée ne décriraient pas un index réel, et `docs/08:55` réserve ce jeu à la latence |
| Extrapoler par le seul ratio de passages | Le ratio ignore la longueur des passages ; deux structures de longueurs différentes ne coûtent pas le même nombre de jetons |
| Extrapoler par une droite de régression sur les jetons | Trois tailles de pool par structure et six structures ne donnent pas de quoi ajuster une droite stable ; une fourchette dit ce qu'on sait |
| Donner une extrapolation ponctuelle | La composition de production par structure est inconnue ; un seul chiffre se lirait comme une mesure |
| Lire « avec et sans positions » comme `freq` et `position` de la configuration | Ces deux valeurs sont internes à tantivy et n'existent dans aucun `index.json` : la configuration ne porte que le booléen `lexical.positions` |
| Mesurer la latence dans les deux valeurs de positions sur toute la campagne | Le surcoût des positions est un ordre de grandeur, pas une courbe ; une série secondaire sur un point suffit et garde le rapport lisible |
| Deux index distincts pour comparer les positions | Deux jeux de fichiers à maintenir et à expliquer ; la même donnée avec deux réglages et un `rebuild_lexical` donne la comparaison voulue |
| Prendre le seuil d'alerte dans `docs/03` | Le tableau de `docs/03` ne porte aucun chiffre pour la branche lexicale (`:154`) : le seuil ne peut venir que d'une soustraction au budget |
| Alerter sur une valeur absolue fixée d'avance | Une valeur absolue ignorerait ce que les autres étapes coûtent déjà ; la soustraction rattache l'alerte au budget de bout en bout |
| Mesurer le froid sur toute la campagne | Le froid coûte du temps et ne sert qu'à borner ; un point suffit à dire si la branche lexicale a un état froid coûteux |
| Traiter les points de `synth-large` comme des mesures de production | `df` ≈ 1/16 est un pire cas de longueur de listes, et le texte n'est pas du français : ce sont des points de plomberie |
| Étendre la campagne à la concurrence de lecture | `KEY_READ_CONNECTIONS = 1` relève d'une tâche de charge en P6, avec un protocole de concurrence qui n'est pas celui-ci |
| Mesurer la reconstruction pendant une compaction | La compaction suspend les écritures (ADR-0017) et son coût appartient à MES-07 ; la mesure conjointe viendra en P9 |
| Modifier l'API publique pour construire les pools | Une mesure ne change pas le contrat du moteur ; le harnais réplique le code, comme MES-01 et MES-02 (ADR-0077, décision 3) |
| Coder le repli d'ADR-0007 dans la même tâche | Le repli n'est qu'une proposition en cas d'alerte, et il demanderait un nouvel ADR : le coder d'avance trancherait une décision qui n'est pas prise |

## Références

- Fiche : `docs/phases/P5-lexical-et-hybride.md:95-103` (`MESURER-LEXICAL`).
- Conception : `docs/03-recherche.md:84-95` (chaîne d'analyse, positions, collecteur, block-WAND), `:154` (ligne « Branche lexicale : à mesurer ») ; `docs/04-donnees-et-ingestion.md:25` (configuration lexicale), `:49` (`lexical.positions` et reconstruction) ; `docs/05-api.md:254` (job `rebuild_lexical`) ; `docs/02-architecture.md:221` (tantivy, à mesurer) ; `docs/08-invariants-et-tests.md:55` (`synth-large` réservé à la latence), `:71` (ligne MES-06) ; `docs/suivi.md`, section « Cibles », `:31` (p95 ≤ 698 ms) ; `docs/00-glossaire.md` (index, passage, pool, empreinte du modèle).
- Mesures : `docs/mesures/README.md` (modèle de rapport) ; MES-02 (scan), MES-03 (rescoring), MES-05 (encodage), MES-14 (filtres) pour les termes de la soustraction ; MES-07 (7,74 M passages vivants).
- Données : `data/legifacile-sample/` (257 626 passages, six structures, 300 requêtes, aucun index attaché) ; `data/synth-large/` (7,74 M vivants, texte synthétique) ; `tools/gen_synth.py:134-151`, `:355-357`.
- Code : `crates/crible-core/src/config.rs:423-424` (`pub positions: bool`) ; `crates/crible-search/src/pool.rs:82` (`Pool::evaluate`) ; `crates/crible-search/src/lexical.rs:89` (`lexical::search`).
- ADR liés : ADR-0007 (tantivy pour BM25, attend MES-06 et son seuil), ADR-0047 (cible de 60 s au démarrage), ADR-0076 et ADR-0077 (protocoles de mesure de P4, forme reprise ici), ADR-0084 (quel froid fait foi), ADR-0087 (pic RSS du commit et segments), ADR-0088 (reconstruction à 8,58 M).

## Documents modifiés à l'acceptation

| Document | Modification |
|---|---|
| `docs/adr/README.md` | ligne d'index de l'ADR-0091 |
| `docs/phases/P5-lexical-et-hybride.md` | section `MESURER-LEXICAL` : renvoi au protocole d'ADR-0091, livrables inchangés |
| `docs/08-invariants-et-tests.md:71` | ligne MES-06 : référence à ADR-0091 |
