# ADR-0088 — Définition d'un index lexical à jour, et lexical illisible à la reprise

- **Statut** : Accepté (décidé le 2026-10-28, tel que proposé)
- **Date** : 2026-10-28 (proposition et décision)
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, sur demande de l'orchestrateur, à partir de la relecture du 1er tour de la tâche `INDEXER-TANTIVY-A-L-ATTACHE` (phase P5) et des verdicts de `@architecte` du 2026-10-28 (`ECART` sur le patch de métadonnées, `NON-COUVERT` sur le lexical illisible)
- **Remplace** : — (précise ADR-0087, dont le corps n'est pas réécrit : la décision 2 ci-dessous élargit sa règle d'ouverture « charge utile égale `G` » au critère `M ≤ Gt ≤ G`, et la décision 5 décline le message d'ADR-0087:38 selon l'état constaté)

## Contexte

### Ce que la conception et les ADR exigent

- **Le point de commit est le commit SQLite** (ADR-0016). L'étape 4 de l'attache est l'index tantivy : « ajouter les nouveaux passages, supprimer les iid tombés, faire un commit avec la charge utile `G+1`, puis recharger le lecteur » (`docs/02-architecture.md:118`).
- **Un patch de métadonnées saute l'étape 4.** « Modifier des métadonnées : étapes 3, 5 et 6 » (`docs/02:122`) et « l'étape 4 n'a pas lieu : le texte ne change pas, donc tantivy n'est pas touché » (`docs/02:127`). ADR-0060 le confirme : un patch ne touche « ni le texte, ni les vecteurs, ni les iid, ni le lexical » (ADR-0060:27, `:176`).
- **La reprise compare la charge utile du dernier commit tantivy `Gt` à `G`** (`docs/02:139-141`) : `Gt < G` rattrape, `Gt > G` reconstruit. Depuis ADR-0087, cette comparaison et la reprise n'ont lieu qu'à l'**ouverture en écriture** ; l'ouverture en lecture seule « n'ouvre un lecteur que si la charge utile du dernier commit égale `G` » (ADR-0087:34), et refuse sinon la branche lexicale en `overloaded` (503) avec le message « lexical index behind generation Gt of G; open the index for writing to repair it » (ADR-0087:38).
- **INV-04** (`docs/08:12`) : « au redémarrage, l'état est la dernière génération commitée ; tout le reste est reconstruit ou nettoyé ». **INV-02** et **INV-03** interdisent qu'une lecture bloque ou publie un état mixte.
- **Règle des fichiers dérivés** (ADR-0069:114, `docs/02:80`) : reconstruits à l'ouverture en écriture, en RAM seulement en lecture ; et « absent n'est pas une erreur, longueur fausse en est une » (ADR-0069, décision 3).

### Verdict de `@architecte` — partie 1, patch et charge utile : `ECART`

Le code de la tâche applique la lettre d'ADR-0087 : un lecteur n'est ouvert que si la charge utile vaut `G`. Or un patch est une **opération normale** — ADR-0060 lui donne une sous-commande, une route et un rapport, et `docs/02:131` crée une génération dès qu'un document change. Après un `crible patch` qui fait passer la génération de `G` à `G+1`, la charge utile reste `Gt = G` : une ouverture en lecture seule refuserait donc `lexical` et `hybrid` en `overloaded` sur un **index sain**, avec un message qui affirme un retard qui n'existe pas. Deux faits rendent l'écart net :

- dans le processus écrivain, le code fait déjà ce qu'il faut : quand une génération ne change aucun texte, `index_lexical` réutilise le lecteur de `G` sous l'instantané `G+1` (`crates/crible-store/src/generation.rs:1229-1231`, commentaire `:1208-1211`) ; seule l'ouverture en lecture seule ignore cette règle ;
- la propriété qui compte n'est pas « la charge utile vaut `G` », mais « aucun passage n'a été créé ni n'est mort depuis la charge utile ». Les textes et les iid sont immuables pour un iid donné (`docs/02:158`), et un patch ne change que la ligne `documents` (`docs/02:126`).

**Ce que le critère doit coûter.** L'ouverture en lecture seule ne peut rien écrire : ni index SQLite, ni fichier. Une requête d'existence sur `passages(created_generation)` balaie 8,58 M lignes, parce que `docs/02:101` ne déclare que `passages(doc_id)` et `passages(dead_since_gen)`. Il faut donc un critère qui se lit à coût constant, et trancher où l'information est gardée.

### Verdict de `@architecte` — partie 2, lexical illisible à l'ouverture : `NON-COUVERT`

Une charge utile tantivy illisible (non décimale) fait aujourd'hui échouer l'ouverture en écriture en `internal` (`crates/crible-store/src/lexical/mod.rs:452-462`), et un dossier `lexical/` que tantivy n'ouvre pas fait de même par le même chemin (`lexical/mod.rs:133-168`, `:380-393`). Or `lexical/` est **dérivé** (`docs/02:73,80`), et un `Gt > G` est déjà réparé par une reconstruction complète (`generation.rs:1167-1177`). Les précédents se contredisent :

- `docs/02:141` ne répare que `Gt > G` ;
- `docs/02:80` et ADR-0069:114 font du dérivé une chose reconstructible ; mais ADR-0069, décision 3, fait d'un `codes.bin` de **longueur fausse** une erreur `Internal` (corruption, pas dérivé manquant) ;
- INV-04 tranche dans le sens de la reconstruction : « tout le reste est reconstruit ou nettoyé » ;
- ADR-0069, alternative (i), reproche au refus de laisser « l'index **muet** » jusqu'à une intervention manuelle.

Le commentaire du code explique le `internal` actuel par une hypothèse : « a payload this build cannot read means the lexical index was written by another one » (`lexical/mod.rs:425-428`, `:447-451`). C'est plausible, mais un index écrit par une autre version est précisément ce qu'une version sait reconstruire : la charge utile ne sert qu'à décider quoi rattraper.

### Ce que cet ADR ne décide pas

- Ni le sort de `lexical/` pendant la compaction et le job de reconstruction (`RECONSTRUIRE-LEXICAL`) ;
- ni un code d'erreur dédié (`lexical_not_ready`), écarté par ADR-0087:42 ;
- ni le budget de l'`IndexWriter` (ADR-0087, point 3) ;
- ni `ensure_manifest`, qui reste hors périmètre (ADR-0087, décision 4) ;
- ni la phrase de `docs/04-donnees-et-ingestion.md:229` (« aucune écriture n'a lieu après le point de commit »), corrigée par un erratum sur décision du propriétaire ;
- ni la durée d'un rattrapage attribuée à `lexical`, corrigée dans la tâche `INDEXER-TANTIVY-A-L-ATTACHE`.

## Décision

### 1. La charge utile d'un commit tantivy reste la génération dont le texte est indexé

La charge utile d'un commit tantivy est **la génération dont le texte est indexé** : `G+1` pour une attache ou une suppression qui touche des passages (`docs/02:118` inchangé), et **aucun commit tantivy pour un patch**, qui ne change aucun texte (`docs/02:127` inchangé). Cette décision ne change donc ni l'étape 4, ni le format de la charge utile : elle nomme ce que la charge utile signifie, pour que la règle d'ouverture ci-dessous puisse s'y appuyer.

### 2. « Lexical à jour » pour la génération `G` : `Gt ≤ G` et aucun passage créé ni mort après `Gt`

Un index lexical de l'époque courante est **à jour pour `G`** quand il existe un commit tantivy de charge utile `Gt` tel que :

```
M ≤ Gt ≤ G          où  M = meta.passage_change_generation
```

et où `M` est **la dernière génération qui a créé ou fait mourir un passage** dans l'époque (`0` si l'époque n'en a jamais porté). La charge utile seule ne suffit donc plus : `Gt < G` est légitime dès que le texte n'a pas bougé depuis `Gt`.

**Pourquoi c'est exact, et pas approché.** Si `M ≤ Gt`, aucune génération de `]Gt, G]` n'a créé de passage ni posé de pierre tombale. Le pool de `G` (passages vivants) est donc égal à celui de `Gt`, les textes sont immuables pour un iid donné (`docs/02:158`) et le nombre de documents tantivy — donc les statistiques BM25 — est celui de `G`. Un lecteur ouvert sur le commit `Gt` rend exactement ce qu'un lecteur de `G` rendrait : aucun état mixte (INV-03), le pool reste l'autorité (INV-05), et un passage mort après `Gt` ne peut pas apparaître puisqu'alors `M > Gt` et le lecteur n'est pas ouvert.

**Un `M > G` est incohérent** : aucune transaction de l'époque ne peut l'avoir écrit, puisque `M` est toujours une génération déjà commitée. Le marqueur est alors traité comme un état indécidable (décision 6) : reconstruction à l'ouverture en écriture, pas de lecteur en lecture seule.

**Conséquences directes de la règle, aux deux ouvertures.**

- **Lecture seule** : un lecteur est ouvert si `M ≤ Gt ≤ G`. Après un patch sur un index à jour, `Gt = G-1`, `M ≤ G-1`, le lecteur est présent et `lexical`/`hybrid` servent la génération `G`. C'est la correction de l'écart.
- **Écriture** : la reprise compare `(Gt, M)` à `G` au lieu de `Gt` seul. `Gt < G` avec `M ≤ Gt` ne déclenche **aucun** rattrapage, donc aucun commit tantivy inutile : aujourd'hui, chaque redémarrage suivant un patch rejoue un `catch_up_lexical` vide qui commite `G` sans contenu. `Gt < G` avec `M > Gt` rattrape comme avant ; `Gt > G` reconstruit comme avant.
- Le code en processus suit déjà cette règle (`generation.rs:1229-1231`) : la tâche n'a qu'à aligner l'ouverture en lecture seule et la comparaison de reprise sur la même définition.

### 3. Le marqueur `M` est une colonne de `meta`, écrite dans la transaction de l'étape 3

**Option retenue : (a1).** `meta` reçoit une colonne `passage_change_generation` (`INTEGER`, nullable), à côté de `generation`, `epoch`, `format_version` et `next_iid` (`docs/02:95`). Elle est écrite **dans la transaction de l'étape 3**, donc atomique avec le point de commit (ADR-0016) :

- une génération qui crée au moins un passage vivant ou pose au moins une pierre tombale y écrit `G+1` ;
- une génération qui n'en fait ni l'un ni l'autre — un patch, une attache qui ne réécrit que des lignes `documents` — **ne la modifie pas** ;
- la transaction qui crée l'époque (compaction, restauration, création) l'initialise à la génération qu'elle écrit ; une reconstruction complète du lexical commite ensuite avec une charge utile au moins égale.

Aucun index SQLite n'est ajouté : `docs/02:101` reste `passages(doc_id)` et `passages(dead_since_gen)`. Le critère se lit donc en **une ligne de `meta`** et un `meta.json` tantivy, aux deux ouvertures, y compris en lecture seule.

**Pourquoi (a1) plutôt que (a2).** `passages(created_generation)` coûterait une construction d'index mesurée à **6,4 s par époque existante** à la première ouverture en écriture, pendant laquelle l'index n'est pas servi (ADR-0066:98), une écriture d'index permanente sur la table de 8,58 M lignes — donc sur le chemin d'insertion de chaque attache, que `BORNER-COUT-DU-COMMIT` vient de ramener à 89,8 ms (MES-15) — et une structure de plus à tenir cohérente. Il ne dispenserait pas de traiter les index anciens : une ouverture en lecture seule sur une époque sans l'index retomberait sur un balayage ou sur la règle étroite d'ADR-0087. Une ligne de `meta` ne paie rien sur le chemin d'écriture des passages.

### 4. `format_version` reste 1 : la colonne est optionnelle et additive

La colonne est ajoutée **sans incrémenter `records::FORMAT_VERSION`**. Motif : `check_format_version` refuse toute version autre que la sienne (`crates/crible-store/src/records.rs:1166-1215`), donc passer à 2 rendrait **tous les index existants illisibles** au premier démarrage après mise à jour, en `invalid_request`, sans mécanisme de migration — un coût sans rapport avec le problème traité. La colonne est lue par nom (`SELECT generation, epoch, next_iid, passage_change_generation FROM meta`), une colonne absente se lit `NULL`, et un lecteur qui l'ignore lit l'époque exactement comme avant : l'évolution est compatible dans les deux sens, et aucun octet existant ne change de sens. La règle que cet ADR fixe, et que le propriétaire ratifie : **une colonne nullable ajoutée à `meta`, sans changement des colonnes existantes, ne change pas `format_version`**. Toute autre évolution du schéma reste soumise à la règle d'INV-15 et d'ADR-0066:64.

**Colonne absente = non certifié.** Une époque créée avant P5 n'a pas la colonne, ou l'a à `NULL`. La colonne n'est **jamais** ajoutée par une ouverture en lecture seule : une lecture n'écrit rien dans l'époque (ADR-0056:44, ADR-0087).

- **À l'ouverture en écriture** : la colonne est ajoutée si elle manque (`ALTER TABLE meta ADD COLUMN`, une instruction DDL sur une table d'une ligne), puis `M` est calculé une fois, par `max(created_generation)` et `max(dead_since_gen)` sur `passages` (`0` si la table est vide), et écrit. Cela ne crée ni génération, ni manifeste, ni changement de résultat, et ne modifie aucune colonne existante. Le coût est un balayage de `passages`, du même ordre que les 489 ms mesurés par ADR-0066:30 à 8,58 M passages (à confirmer) ; il est payé **une fois par époque**, avant la comparaison de reprise et avant la publication de l'instantané.
- **À l'ouverture en lecture seule** : la colonne absente **n'élargit jamais** le critère ; elle le réduit au cas d'ADR-0087, `Gt = G`. Un index ancien, jamais ouvert en écriture par cette version, garde donc le comportement actuel : un patch y fait refuser la branche (`Gt < G`, mais l'état n'est pas certifié), jusqu'à la première ouverture en écriture qui complète la colonne.

### 5. Le refus en lecture seule nomme l'état trouvé ; `overloaded` reste le code

Le refus reste `overloaded` (503) pour les motifs d'ADR-0087, décision 3, qui ne changent pas. En revanche, le message ne peut plus affirmer « behind » dans tous les cas : il nomme l'état réellement constaté, et garde la phrase d'ADR-0087:38 pour le seul cas où elle est vraie.

| État constaté à l'ouverture en lecture seule | Message |
|---|---|
| `Gt < G` et `M > Gt` | « lexical index behind generation Gt of G; open the index for writing to repair it » (inchangé) |
| `Gt > G` | « lexical index ahead of generation Gt of G; open the index for writing to repair it » |
| `Gt < G` et `M` non certifié (colonne absente) | « lexical index state at generation Gt is not certified for generation G; open the index for writing to repair it » |
| `lexical/` illisible, ou `M` incohérent (`M > G`) | « lexical index is unreadable at generation G; open the index for writing to repair it » (voir décisions 2 et 6) |

`Gt`, `G` et l'état sont des faits visibles : aucun `iid` n'y figure (INV-01). Le mode `dense` continue de servir `G`, et `hybrid` n'est jamais dégradé vers le dense (ADR-0087, décision 2). ADR-0087 n'est **pas réécrit** : si le propriétaire veut que « behind » reste littéralement juste dans son corps, une note d'erratum sur ADR-0087:38 suffit, et relève de lui.

### 6. À l'ouverture en écriture, un `lexical/` illisible est reconstruit, pas refusé

**Option retenue : (1), pour les deux cas.** Toute impossibilité de lire l'état lexical de l'époque — charge utile non décimale (`lexical/mod.rs:452-467`), `meta.json` ou segment que tantivy n'ouvre pas, dossier que tantivy n'ouvre pas — est traitée comme **« état indécidable »**, exactement comme un dossier absent, et non comme une erreur. L'ouverture en écriture suit alors le chemin déjà écrit pour `Gt > G` : `remove_index` puis `rebuild_lexical` (`generation.rs:1167-1177`), avec une ligne de journal qui nomme l'époque, le cas et le message d'origine. Aucune génération nouvelle, aucun changement du manifeste, aucune intervention de l'exploitant : INV-04 est tenu par la reconstruction.

**Pourquoi le lexical diffère de `codes.bin`.** Trois raisons, toutes documentées :

- **L'information perdue n'est pas comparable.** Pour `codes.bin`, la longueur attendue est connue (`count × ceil(dims/64) × 8`, ADR-0069, décision 6) : une longueur fausse est un **fait vérifiable** sur un fichier Crible, et le refus protège d'une réécriture par-dessus un fichier qu'on n'a pas su lire. Ici, la charge utile est l'unique état de l'index, et une charge utile illisible rend `Gt` **indécidable** : le moteur ne sait pas quelle génération l'index décrit, donc ne peut rien rattraper — c'est la définition même d'un dérivé à reconstruire.
- **Le format n'est pas le nôtre.** `lexical/` est un index tantivy, format d'une bibliothèque tierce, hors du champ d'INV-15 : aucune version Crible n'y est stockée, et une incompatibilité de version est un cas normal d'une mise à jour qui change de tantivy, que seule une reconstruction sait résoudre.
- **Le refus a un coût disproportionné.** `internal` à l'ouverture en écriture rend l'index **non ouvrable pour écrire** : plus d'attache, plus de patch, sur un index dont la seule pièce dérivée est illisible. C'est le reproche qu'ADR-0069, alternative (i), adresse au refus. La reconstruction coûte des minutes à 8,58 M passages, une fois.

**Contrepartie assumée** : une corruption réelle (secteur défaillant, copie partielle) n'est visible que par la ligne de journal, et non par un échec. Voir « À surveiller ».

### 7. À l'ouverture en lecture seule, un `lexical/` illisible ne fait pas échouer l'ouverture

Rien n'est réparé à la lecture (ADR-0087, décision 1). Un `lexical/` illisible ne doit donc pas non plus faire échouer l'ouverture : la conséquence serait qu'une pièce **dérivée** rende l'état de référence illisible, alors que `dense` n'en dépend pas. L'ouverture en lecture seule journalise le cas et publie l'instantané **sans lecteur lexical** ; la branche lexicale refuse ensuite en `overloaded` (décision 5), et `dense` sert `G`. Aucun `IndexWriter`, aucune suppression, aucune écriture sous l'époque.

### 8. Ce que cet ADR ne décide pas

Le sort de `lexical/` pendant la compaction et le job de reconstruction (`RECONSTRUIRE-LEXICAL`), qui devra commiter avec une charge utile au moins égale à `M` ; le code d'erreur d'une recherche lexicale quand `lexical.enabled` vaut `false` ; le budget de l'`IndexWriter` ; `ensure_manifest` ; toute mesure de durée, les chiffres cités venant d'ADR-0066 et restant à confirmer.

## Conséquences

- **Positives** :
  - un patch de métadonnées, opération normale, ne dégrade plus `hybrid` en lecture seule : la branche lexicale sert la génération `G` avec un lecteur `Gt` dont le contenu est identique (décision 2) ;
  - la règle d'ouverture devient une propriété vérifiable et bon marché, lue en une ligne aux deux ouvertures ;
  - plus de commit tantivy vide au redémarrage qui suit un patch, donc un fsync et un segment de moins par cycle attache-patch-redémarrage ;
  - un index dont le lexical est illisible ou écrit par une autre version repart seul, sans intervention, en cohérence avec INV-04 et `docs/02:80` ;
  - une lecture seule ne peut plus être empêchée par un fichier dérivé, ce qui renforce INV-02 ;
  - le message de refus cesse d'affirmer un retard inexistant.
- **Négatives / coûts** :
  - une colonne de plus dans `meta`, une règle de plus à tenir à chaque écriture de génération, et une complétion de schéma à l'ouverture en écriture des époques antérieures à P5 ;
  - une exception explicite à la règle « toute évolution du schéma incrémente `format_version` », qu'il faudra citer si un jour une colonne ajoutée casse réellement la lecture ;
  - une corruption du lexical n'échoue plus : elle se paie en minutes de reconstruction et en une ligne de journal, invisible pour qui ne lit pas les journaux ;
  - quatre messages de refus au lieu d'un, donc quatre cas à tester dans la branche lexicale ;
  - `overloaded` couvre un cas de plus (lexical illisible), dans un code déjà chargé (`docs/05:42`).
- **Tests exigés** :
  - **patch puis ouverture en lecture seule : lecteur présent** (le cas de l'écart), et `lexical`/`hybrid` servis à `G` ;
  - patch puis ouverture en écriture : aucun commit tantivy, charge utile inchangée ;
  - arrêt entre l'étape 3 et l'étape 4 avec `M = G` : la reprise en écriture rattrape et INV-04 tient ;
  - colonne absente : en lecture seule, la règle étroite d'ADR-0087 s'applique ; en écriture, `M` est calculé puis écrit, une fois, sans génération ;
  - charge utile non décimale : l'ouverture en écriture reconstruit et journalise, sans erreur ; en lecture seule, `dense` réussit et `lexical` refuse en `overloaded` avec le message de la décision 5 ;
  - dossier `lexical/` illisible : mêmes deux comportements ;
  - `Gt > G` en lecture seule : pas de lecteur, message « ahead », `dense` servi.
- **À surveiller** :
  - la fréquence des reconstructions complètes du lexical : une reconstruction à chaque démarrage signalerait autre chose qu'un dérivé manquant — disque, copie partielle, deux versions de tantivy en alternance — et justifierait de revenir sur la décision 6 ;
  - le coût réel du balayage qui complète la colonne sur une époque ancienne, à relever à la première ouverture en écriture de l'index Legifacile ;
  - la tenue de `M` par tout nouveau chemin qui crée une époque ou écrit des passages : un `M` oublié certifie à tort un index en retard, ce qui est le risque le plus grave de cet ADR. Un test de propriété sur les transactions de génération (le marqueur avance si et seulement si des passages sont créés ou tués) est le garde-fou ;
  - la durée de la reconstruction à 8,58 M passages, à relever à la première occurrence (MES-06), et la cible de 60 s d'ADR-0047 pour le temps de démarrage ;
  - un index ancien resté en lecture seule : il garde la règle étroite jusqu'à sa première ouverture en écriture. Si l'exploitation finit par ne plus jamais ouvrir en écriture, il faudra une commande de réparation décidée, comme le note ADR-0087.
- **Documents à modifier à l'acceptation** :

| Document | Modification |
|---|---|
| `docs/02-architecture.md:95` | ajouter `passage_change_generation` à la ligne `meta`, avec sa définition (dernière génération qui a créé ou fait mourir un passage, `0` sinon) |
| `docs/02:101` | **inchangé** : aucun index SQLite n'est ajouté ; le dire si la phrase prête à confusion |
| `docs/02:117` | à l'étape 3, nommer l'écriture de `meta.passage_change_generation = G+1` quand la transaction crée ou tue un passage |
| `docs/02:118` | écrire que la charge utile d'un commit tantivy est « la génération dont le texte est indexé », et qu'un patch n'en produit aucun |
| `docs/02:127` | **inchangé** ; ajouter au besoin que le lecteur de `G` reste servi sous l'instantané `G+1` |
| `docs/02:139-142` | remplacer « un lecteur n'est ouvert que si la charge utile du dernier commit égale `G` » par le critère `M ≤ Gt ≤ G` ; ajouter la complétion de la colonne absente à l'ouverture en écriture, et le traitement d'un `lexical/` illisible (reconstruction en écriture, pas de lecteur en lecture seule) |
| `docs/02:80` | préciser que, pour `lexical/`, « absent » et « illisible » ont la même conséquence : reconstruit à l'ouverture en écriture |
| `docs/02:144` | mentionner, à côté de la règle de `codes.bin`, que `lexical/` suit la règle inverse (état indécidable ⇒ reconstruction) et pourquoi |
| `docs/03-recherche.md:78-93` | définir « lexical à jour » (`M ≤ Gt ≤ G`), ajouter « charge utile ou dossier illisible » aux cas de reconstruction, et dire qu'un patch ne rend pas la branche indisponible |
| `docs/05-api.md:42` | `overloaded` : ajouter « index lexical non certifié ou illisible » à côté du retard, et renvoyer aux quatre messages |
| `docs/phases/P5-lexical-et-hybride.md` (fiche `INDEXER-TANTIVY-A-L-ATTACHE`) | livrable de reprise et critères : critère `M ≤ Gt ≤ G`, complétion de la colonne, reconstruction d'un lexical illisible, et le test « patch puis ouverture en lecture seule : lecteur présent » |
| `docs/phases/P5-lexical-et-hybride.md` (fiche `RECHERCHER-BM25-DANS-LE-POOL`, `:65`, `:70`) | remplacer le message unique par les quatre états de la décision 5, et élargir le critère d'acceptation du refus |
| `docs/adr/README.md` | ligne d'index de cet ADR |
| `docs/08-invariants-et-tests.md` | **aucune modification nécessaire** : INV-01, INV-02, INV-03 et INV-04 ne changent pas ; les nouveaux comportements sont des tests, pas des invariants |

`docs/suivi.md` n'est pas modifié par cet ADR : les lignes « Écarts à la conception » de `INDEXER-TANTIVY-A-L-ATTACHE` sont tenues par l'orchestrateur et doivent citer ADR-0088 pour les deux parties.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| (a2) Index SQLite `passages(created_generation)` et deux recherches indexées | Construction mesurée à 6,4 s par époque à la première ouverture en écriture, index non servi pendant ce temps (ADR-0066:98) ; écriture d'index permanente sur la table de 8,58 M lignes, donc sur le chemin d'insertion que MES-15 vient d'optimiser ; et une époque ancienne sans l'index laisse l'ouverture en lecture seule devant un balayage ou la règle étroite. Ne retire pas la question du cas ancien, contrairement à ce qu'elle laisse croire |
| Calculer le critère par un balayage à chaque ouverture, sans état | Coût non budgété à chaque ouverture, et le seul cas où il est interdit d'écrire — la lecture seule — est aussi celui où il serait payé à chaque invocation de `crible search` |
| Traiter une colonne absente comme `M = 0` | Certifierait à tort un index en retard : `0 ≤ Gt` est toujours vrai. La décision 4 retient l'inverse, « non certifié », qui réduit le critère au lieu de l'élargir |
| (b) Un commit tantivy vide portant `G+1` à chaque patch | Contredit `docs/02:127` ; un fsync et un commit sans contenu par patch ; ajoute un point de panne après le point de commit pour une information déjà portée par SQLite |
| (c) Acter le refus après un patch jusqu'à la prochaine ouverture en écriture | Dégrade `hybrid` sur une opération normale et rend faux un message d'ADR-0087 qui affirme un retard inexistant ; le coût est payé par le client, pas par le moteur |
| Porter l'information dans `manifests/<G+1>.json` | Le manifeste est écrit à l'étape 5, **après** le point de commit, et `docs/02:85` en fait un dérivé, jamais une source de vérité : il ne peut pas certifier atomiquement l'état du lexical |
| Ajouter la colonne en passant `FORMAT_VERSION` à 2, avec migration à l'ouverture | `check_format_version` refuse toute autre version avant toute modification (`records.rs:1166-1215`) : tous les index existants deviendraient illisibles, en `invalid_request`, et le mécanisme de migration n'existe pas. Coût sans rapport avec une colonne optionnelle |
| (2) Refuser en `internal` un lexical illisible à l'ouverture en écriture | Rend l'index non ouvrable pour écrire à cause d'une pièce dérivée : plus d'attache ni de patch jusqu'à intervention, ce qu'ADR-0069, alternative (i), reproche déjà au refus, et ce que `docs/02:80` et INV-04 contredisent |
| Faire échouer l'ouverture en lecture seule sur un lexical illisible | Une pièce dérivée empêcherait de servir l'état de référence, y compris en `dense` : contraire à INV-02 et au principe d'ADR-0069:114 |
| Reconstruire le lexical en RAM à la lecture seule | Écarté par ADR-0087, alternative (b) : des minutes et des gigaoctets à chaque invocation de la CLI, pour un état que la prochaine ouverture en écriture répare seule |
| Dégrader `hybrid` en `dense` quand le lexical est illisible, sans le dire | Rend un `score_type` et un classement que le client n'a pas demandés (ADR-0087, alternative (e)) |
| Considérer une charge utile illisible comme une corruption à signaler, et non comme un dérivé manquant | Vaut pour `codes.bin`, dont la longueur attendue est vérifiable et le format le nôtre (ADR-0069, décision 3) ; ici l'état est indécidable et le format appartient à tantivy : la seule réponse possible au moment de l'ouverture est la reconstruction |

## Complément du 2026-10-29

Une précision sur le contenu de la charge utile. Elle ne change ni le statut ni les décisions ci-dessus.

**Statut du complément** : confirmé par le propriétaire le 2026-10-30, sans changement du statut ni des décisions ci-dessus.

**Constat.** La charge utile d'un commit tantivy porte `Gt`, et le critère `M ≤ Gt ≤ G` dit si le lecteur est à jour. Elle ne dit pas avec quelle **chaîne d'analyse** l'index a été construit, alors que `lexical.language` et `lexical.positions` sont modifiables par `PATCH` et que l'écriture d'`index.json` et la reconstruction ne sont pas atomiques.

**Décision.** ADR-0106 ajoute la chaîne d'analyse à la charge utile, dont le format passe à une version qui reste lisible pour une charge utile de la version précédente. Il en tire trois conséquences : reconstruction à l'ouverture en écriture sur chaîne différente, lecture d'une charge utile ancienne comme construite avec la chaîne courante, refus de la branche lexicale en lecture seule sur chaîne différente.

**Raison.** Sans cette information, un arrêt entre l'écriture d'`index.json` et la fin du job `rebuild_lexical` laisse un index lexical construit avec l'ancienne chaîne qu'aucune reprise ne détecte (risque de `RECONSTRUIRE-LEXICAL`, `docs/suivi.md`, 2026-10-28).

## Références

- `docs/02-architecture.md:73,80` — `lexical/` dérivé, reconstruction à l'ouverture en écriture ; `:95,101` — tables `meta` et `passages`, index SQLite ; `:117-131` — étapes de l'attache, patch de métadonnées ; `:133-148` — reprise au démarrage ; `:151-158` — concurrence, immuabilité des textes pour un iid.
- `docs/03-recherche.md:78-93` — branche lexicale, reconstruction.
- `docs/05-api.md:29-44` — codes d'erreur, `overloaded`.
- `docs/08-invariants-et-tests.md:9-12` — INV-01 à INV-04 ; `:23` — INV-15.
- `docs/phases/P5-lexical-et-hybride.md:27-42` — `INDEXER-TANTIVY-A-L-ATTACHE` ; `:56-71` — `RECHERCHER-BM25-DANS-LE-POOL`.
- `crates/crible-store/src/generation.rs:1125-1177` — reprise, `catch_up_lexical`, `rebuild_lexical` ; `:1204-1264` — étape 4 en processus, réutilisation du lecteur, `changes_passage_text` ; `:704-719` — ouverture partagée, `recover_lexical`, `open_current`.
- `crates/crible-store/src/lexical/mod.rs:124-168` — `read_state` ; `:180-208` — `open_current` ; `:375-393` — `existing_index` ; `:425-467` — charge utile et `parse_payload`.
- `crates/crible-store/src/records.rs:64-107` — schéma de l'époque ; `:1162-1215` — `check_format_version`.
- ADR-0007 (tantivy), ADR-0016 (point de commit SQLite), ADR-0047 (mesurer avant d'ajouter un coût de démarrage), ADR-0056:44 (une lecture ne modifie plus le disque), ADR-0060 (contrat du patch), ADR-0066:64, `:98` (index SQLite et `format_version` ; coût d'un index), ADR-0069 (fichiers dérivés, `codes.bin`), ADR-0087 (reprise lexicale et lecture seule), ADR-0086 (`lexical.language`).
