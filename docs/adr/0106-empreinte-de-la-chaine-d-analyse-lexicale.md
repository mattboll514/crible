# ADR-0106 — Empreinte de la chaîne d'analyse lexicale

- **Statut** : Accepté (accepté par anticipation par le propriétaire le 2026-10-29, confirmé par le propriétaire le 2026-10-30)
- **Date** : 2026-10-29
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, tâche ROUTES-INDEX (phase P6), sur demande de l'orchestrateur et les verdicts de `@architecte` du 2026-10-29
- **Remplace** : — (complète ADR-0088, dont le corps n'est pas réécrit : sa charge utile reste la génération dont le texte est indexé, et la chaîne d'analyse s'y ajoute)

## Contexte

L'index lexical est construit avec une **chaîne d'analyse** choisie par index : `lexical.language` sélectionne le tokenizer, les filtres et le stemmer (`docs/03-recherche.md:82-87`, table d'ADR-0086), et `lexical.positions` commande l'indexation des positions (`docs/03:88`). ADR-0088 a donné à l'index lexical un état vérifiable à l'ouverture : la charge utile du dernier commit tantivy porte `Gt`, et le critère `M ≤ Gt ≤ G` dit si le lecteur est à jour pour la génération `G`.

Cet état ne dit **rien de la chaîne d'analyse**. Or trois faits rendent cette information nécessaire.

**Premier fait : un changement de chaîne est une opération normale, et elle est asynchrone.** La table `docs/04:49` fait de `lexical.enabled`, `lexical.language` et `lexical.positions` des réglages modifiables par `PATCH`, qui déclenchent un job de reconstruction du lexical. Depuis ADR-0105, ce job est exécuté par l'écrivain de l'index : la mise en file est un `202`, et rien ne garantit que le job s'exécute avant un arrêt.

**Second fait : l'écriture d'`index.json` et la reconstruction ne sont pas atomiques.** Le job écrit `index.json` de façon atomique, puis reconstruit l'index lexical. Un arrêt entre les deux laisse un `index.json` qui nomme la nouvelle chaîne et un `lexical/` construit avec l'ancienne. La charge utile vaut `G`, `M ≤ Gt ≤ G` tient, et **aucune reprise ne voit le problème** : les requêtes suivantes passent par une analyse différente de celle qui a construit l'index, donc des termes qui ne correspondent plus aux documents. Le risque est écrit dans `docs/suivi.md`, section « Idées hors périmètre », entrée `RECONSTRUIRE-LEXICAL` du 2026-10-28. Il ne s'agit pas d'un incident : c'est le résultat normal d'un arrêt au mauvais moment.

**Troisième fait : le format doit changer.** Aucun fichier existant ne porte la chaîne. ADR-0088 décision 6 traite déjà l'illisible et l'indécidable en reconstruisant ; il ne peut pas distinguer « construit avec la même chaîne » de « construit avec une autre », faute de l'avoir écrit.

**Ce que rien ne permet de deviner.** Aucune voie ne changeait la chaîne sans reconstruction avant `ROUTES-INDEX` : le seul chemin était le job `rebuild_lexical`. Écrire la chaîne et refuser ou reconstruire sur différence est donc sans coût pour les index existants, et c'est ce qui rend l'absence de chaîne dans une charge utile ancienne interprétable.

## Décision

**Décision 1 — la chaîne est enregistrée dans la charge utile du commit tantivy.** La chaîne d'analyse avec laquelle l'index lexical a été construit — `lexical.language` et `lexical.positions` — est enregistrée **dans la charge utile du commit tantivy**, à côté de `Gt` (ADR-0088, décision 1). Le format de la charge utile est **versionné** ; la version augmente, et une charge utile de la version précédente, qui ne porte que `Gt`, reste lisible.

**Décision 2 — à l'ouverture en écriture, une chaîne différente fait reconstruire.** Si la chaîne enregistrée diffère de celle d'`index.json`, l'index lexical est **reconstruit** par la même voie que la reprise (ADR-0088, décision 6), **avant que l'index ne soit servi**. Aucune génération nouvelle, aucun changement du manifeste, une ligne de journal qui nomme l'époque et les deux chaînes.

**Décision 3 — une charge utile ancienne n'est pas une différence.** Une charge utile d'une version antérieure, sans chaîne, est lue comme **construite avec la chaîne d'`index.json`** à cette ouverture. Aucun index existant n'est reconstruit du seul fait de cette décision : avant `ROUTES-INDEX`, aucune voie ne changeait la chaîne sans reconstruction, donc l'hypothèse est vraie pour tout index que cette version rencontre.

**Décision 4 — à l'ouverture en lecture seule, la branche lexicale est refusée.** Une chaîne différente fait refuser la branche lexicale en `overloaded` (503), comme les autres états d'ADR-0088 et d'ADR-0089, avec un message qui nomme l'état constaté : « the lexical index was built with another analysis chain; open the index for writing to rebuild it ». Rien n'est écrit, aucun `IndexWriter` n'est ouvert (ADR-0087). Le mode `dense` reste servi, et `hybrid` n'est pas dégradé silencieusement vers le dense. `Retry-After: 1` suit la règle d'ADR-0103.

**Décision 5 — les deux voies de reconstruction écrivent la chaîne courante.** Le job `rebuild_lexical` et la compaction, qui reconstruit le lexical de l'époque `E+1`, écrivent la chaîne courante dans la charge utile de leur commit. Aucune autre voie n'écrit la charge utile.

Ce qui n'est pas décidé ici : l'encodage exact de la charge utile versionnée, qui appartient à la fiche de la tâche ; la conduite à tenir si la chaîne enregistrée est lisible mais hors du domaine d'ADR-0086, qui est un état indécidable traité par ADR-0088, décision 6 ; la durée d'une reconstruction due à une chaîne différente, qui se mesure comme celle d'ADR-0088.

## Conséquences

- **Positives** :
  - l'état « avec quelle chaîne cet index a été construit » devient lisible à l'ouverture, en une lecture de `meta.json` tantivy, sans fichier de plus ni ordre d'écriture à tenir ;
  - un arrêt entre l'écriture d'`index.json` et la fin du job cesse d'être un état invisible : la reprise le détecte et le répare, ce qui ferme le risque de `RECONSTRUIRE-LEXICAL` ;
  - l'ouverture en écriture reste le seul endroit qui répare, comme pour tout le lexical (ADR-0087, ADR-0088) ;
  - le mode `dense` ne dépend jamais de cette information : une chaîne inattendue en lecture seule coûte la branche lexicale, pas l'index.
- **Négatives / coûts** :
  - le format de la charge utile change de version : il faut lire deux versions, et une version plus ancienne de Crible qui rencontrerait la nouvelle charge utile la tiendrait pour illisible, donc reconstruirait le lexical — un retour arrière coûte une reconstruction, pas une erreur ;
  - toute voie qui écrit la charge utile doit écrire la chaîne : un oubli dans une future voie certifie à tort un index construit avec une autre, exactement comme un `M` oublié dans ADR-0088 ;
  - la charge utile grossit de deux valeurs, et son analyse devient plus stricte qu'un entier décimal ;
  - `overloaded` couvre un état de plus, dans un code déjà chargé (`docs/05:49`) ;
  - une chaîne changée puis un arrêt avant la reconstruction laisse un index dont la branche lexicale est indisponible jusqu'à la prochaine ouverture en écriture : c'est le comportement voulu, mais il doit être dit à l'exploitant.
- **À surveiller** :
  - un test qui écrit avec `lexical.positions: true`, interrompt le job avant la reconstruction, rouvre en écriture, et vérifie que le lexical est reconstruit **avant** d'être servi ;
  - un test de charge utile ancienne (sans chaîne) : aucune reconstruction n'est forcée, la branche lexicale sert ;
  - un test d'ouverture en lecture seule avec une chaîne différente : `dense` sert la génération, `lexical` et `hybrid` refusent en `overloaded` avec le message de la décision 4, rien n'est écrit ;
  - un test qui vérifie que le job `rebuild_lexical` et la compaction écrivent tous deux la chaîne courante ;
  - un test de propriété sur la charge utile : si la chaîne enregistrée diffère de celle d'`index.json`, aucune lecture seule ne sert la branche lexicale sans refus ;
  - la fréquence des reconstructions pour cause de chaîne : une reconstruction à chaque démarrage signalerait une voie qui oublie d'écrire la chaîne, pas un réglage changé.
- **Documents mis à jour par cet ADR** :

| Document | Modification |
|---|---|
| `docs/02-architecture.md:152-153` | comparer la chaîne d'analyse enregistrée à celle d'`index.json` à l'ouverture en écriture ; en lecture seule, refus de la branche avec le nouveau message |
| `docs/03-recherche.md:88` | la chaîne est enregistrée dans la charge utile du commit, et comparée à l'ouverture |
| `docs/03-recherche.md:94` | ajouter « chaîne d'analyse enregistrée différente » aux cas de reconstruction, avec le refus en lecture seule |
| `docs/05-api.md:49` | ligne `overloaded` : le nouvel état et son message |
| `docs/adr/0088-…md:195` | note « Complément du 2026-10-29 » renvoyant à cet ADR, sans modification du corps |
| `docs/adr/README.md:162` | ligne d'index de cet ADR |

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Un fichier séparé à côté de `lexical/`, par exemple `lexical/analysis.json` | Deux écritures à ordonner : il faudrait décider laquelle fait foi en cas de désaccord, et le désaccord est précisément le cas qu'on veut détecter. La charge utile est déjà atomique avec le commit tantivy. |
| Reconstruire tous les index existants à la première ouverture | Des minutes par index à 8,58 M passages, pour une information qu'aucune voie n'avait changée avant `ROUTES-INDEX` : la décision 3 est exacte et gratuite. |
| Ne rien enregistrer et compter sur l'ordre des écritures du job | C'est l'état actuel : une panne entre les deux écritures laisse un index dont l'analyse ne correspond plus aux documents, sans aucun signe. C'est le risque que cet ADR ferme. |
| Refuser `internal` à l'ouverture en écriture sur une chaîne différente | Rendrait l'index non ouvrable pour écrire à cause d'une pièce dérivée, ce que `docs/02:84` et INV-04 contredisent, et ce qu'ADR-0088, décision 6, a déjà écarté pour l'illisible. |
| Refuser `invalid_request` plutôt que `overloaded` en lecture seule | `invalid_request` dit que la requête est fautive ; ici l'index est dans un état temporaire que la prochaine ouverture en écriture répare. C'est la même famille que le retard et l'absence de commit (ADR-0088, ADR-0089). |
| Dégrader `hybrid` vers le dense quand la chaîne diffère | Rend un `score_type` et un classement que le client n'a pas demandés, et masque un index à réparer (ADR-0087, alternative (e)). |
| Porter la chaîne dans `manifests/<G>.json` | Le manifeste est un dérivé écrit après le point de commit (ADR-0016, ADR-0088, alternative correspondante) : il ne peut pas certifier atomiquement l'état du lexical. |
| Une empreinte des fichiers du tokenizer dans la charge utile | Enregistrerait une valeur que rien ne compare à l'ouverture, puisque le tokenizer vient de tantivy et de la table d'ADR-0086 ; `lexical.language` et `lexical.positions` suffisent à décrire la chaîne, et se comparent directement à `index.json`. |

## Références

- `docs/suivi.md`, « Idées hors périmètre », entrée `RECONSTRUIRE-LEXICAL` du 2026-10-28 (risque d'un arrêt entre l'écriture d'`index.json` et la fin du job).
- `docs/03-recherche.md:78-96` — branche lexicale, chaîne d'analyse, `lexical.positions`, reconstruction, états sans commit.
- `docs/02-architecture.md:84` (dérivés reconstruits à l'ouverture en écriture), `:140-159` (reprise au démarrage), `:171` (calcul hors tokio).
- `docs/04-donnees-et-ingestion.md:46-55` (réglages modifiables et job de reconstruction), `:66-72` (domaine de `lexical.language`).
- `docs/05-api.md:49` (`overloaded`).
- `docs/08-invariants-et-tests.md` — INV-02, INV-03, INV-04.
- ADR-0086 (domaine de `lexical.language`), ADR-0087 (reprise lexicale et ouverture en lecture seule), ADR-0088 (charge utile, critère `M ≤ Gt ≤ G`, état indécidable), ADR-0089 (branche indisponible ou désactivée), ADR-0094 (analyseur français inchangé), ADR-0105 (job `rebuild_lexical` déclenché par un `PATCH`).
