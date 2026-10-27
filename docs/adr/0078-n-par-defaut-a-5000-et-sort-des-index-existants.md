# ADR-0078 — N par défaut à 5000 et sort des index existants

- **Statut** : Accepté (décision du propriétaire le 2026-10-27 pour la valeur 5000 ; mécanisme du défaut et sort des index existants retenus par l'orchestrateur sur recommandation de `@architecte`, à confirmer par le propriétaire)
- **Date** : 2026-10-27 (proposition et acceptation le 2026-10-27)
- **Décideurs** : propriétaire (valeur de N) ; orchestrateur sur recommandation de `@architecte` (mécanisme du défaut et sort des index existants, à confirmer)
- **Décision** : la valeur 5000 est décidée par le propriétaire le 2026-10-27. Le mécanisme du défaut (documentaire et de gabarit, sans constante dans le code) et le sort des index existants (aucune migration) sont retenus par l'orchestrateur sur recommandation de `@architecte`, et restent à confirmer par le propriétaire. Les sept décisions ci-dessous sont retenues telles que proposées.
- **Proposé par** : agent greffier, sur demande de l'orchestrateur, après MES-01, MES-02 et MES-03 (phase P4) et les verdicts de `@architecte`
- **Remplace** : —

## Contexte

### La question restée ouverte

Q-01, « N par défaut et cible de fidélité du scan binaire », était renvoyée à MES-01 (`docs/adr/README.md`, « Questions ouvertes »). MES-01, MES-02, MES-03 et MES-12 sont rendues. Il reste à fixer la valeur de N, le mécanisme par lequel elle devient un défaut, et ce qu'il advient des index déjà créés.

Aucun ADR ne fixe N. ADR-0074 (décision 2) fixe seulement que `search.exact_threshold` est optionnelle et prend à la création la valeur de `search.rescore_candidates`, valeur **matérialisée** dans `index.json`. ADR-0075 (décision 4) règle le nombre de candidats sous BINARY à `max(N, candidates_per_branch)`. ADR-0076 (décisions 1 et 4) définit les métriques de fidélité et renvoie la confirmation à l'échelle à P9. ADR-0077 fixe le protocole de latence, avec N ∈ {1000, 2000, 5000}.

### Ce que le code fait, et ce qu'il ne fait pas

- `search.rescore_candidates` est **obligatoire** dans `index.json` (`crates/crible-core/src/config.rs:428`).
- `SearchConfig` n'a **pas** de `Default` ; `index create` recopie le fichier fourni.
- **Aucune constante de production ne vaut 1000.** Le seul défaut calculé est `exact_threshold`, égal à N, matérialisé à la création (ADR-0074, décision 2).
- Le « défaut » de 1000 est donc **documentaire et de gabarit** : il vit dans `docs/03-recherche.md:62`, dans les exemples de `docs/04-donnees-et-ingestion.md:28-29` et de `docs/05-api.md:197`, et dans le gabarit de `tools/gen_synth.py`.

### Ce que les mesures disent

- **MES-01** (`docs/mesures/MES-01-fidelite-scan-binaire.md`) : recall@10 global de 0,9757 à N = 1000 et de 0,9977 à N = 5000 sur la campagne principale ; 0,9760 et 0,9989 sur la variante filtrée. N = 5000 est la **seule** valeur qui atteint la cible de 0,99 dans les deux campagnes et pour toutes les structures. N = 2000 donne 0,9887 (principale) et 0,9924 (filtrée) : sous la cible principale.
- **MES-02** (`docs/mesures/MES-02-latence-scan-binaire.md`) : le scan paie +11 % au pool complet en passant de N = 1000 à N = 5000 (40,0 → 44,6 ms p50).
- **MES-03** (`docs/mesures/MES-03-latence-rescoring.md`) : le rescoring à chaud passe de 29,8 à 61,1 ms p50 (p95 de 90,5 à 186,9 ms) ; à froid, de 179,0 ms à N = 1000 à 177,5 ms pour le produit scalaire seul à N = 5000, et jusqu'à 258,3 ms pour le moteur entier sur une série contaminée.

### Ce qu'on ne sait pas

La fidélité à l'échelle de Legifacile (8,58 M passages) n'est pas mesurée : MES-01 porte sur l'échantillon, dont les pools vont jusqu'à 50 000 passages. La confirmation est renvoyée à P9 (ADR-0076, décision 4).

## Décision

**N par défaut vaut 5000, sous forme d'un défaut documentaire et de gabarit, sans constante ni repli dans le code ; les index existants ne sont ni migrés ni renormalisés.**

Précisions :

1. **Valeur.** `search.rescore_candidates` prend la valeur **5000** par défaut. C'est la valeur décidée par le propriétaire.
2. **Mécanisme.** Le défaut est **documentaire et de gabarit** (option (i) de `@architecte`) : les documents, les exemples et le gabarit de `tools/gen_synth.py` passent à 5000. La clé reste **obligatoire** dans `index.json` : aucune constante `DEFAULT_RESCORE_CANDIDATES`, aucun repli à la lecture, aucun `Default` sur `SearchConfig`. Le contrat de configuration ne change pas.
3. **Conséquence sur le seuil.** `exact_threshold` suit N à la création (ADR-0074, décision 2). Les index créés ensuite portent 5000, donc tout pool d'au plus 5000 passages passe par EXACT. Pour 5000 produits, MES-03 mesure 2,04 ms à chaud et 177,5 ms à froid.
4. **Index existants.** Aucune migration, aucune renormalisation, ni génération ni époque : un index créé à 1000/1000 garde son comportement et sa configuration. Monter un index existant demande **deux** `PATCH` : `rescore_candidates = 5000` **et** `exact_threshold = 5000`. Un `PATCH` de N ne déplace pas le seuil (ADR-0074, décision 2). Crible n'est pas encore en production : l'index de Legifacile sera créé en P9 avec les valeurs de ce défaut.
5. **Regroupement.** Avec N = 5000, `max(N, candidates_per_branch)` vaut 5000 pour tout `k`, puisque le plafond de `candidates_per_branch` est 1000 (`docs/03-recherche.md:52`). Une requête groupée rescore donc 5000 candidats. La phrase d'ADR-0075 (conséquences, « avec N = 1000, le pire cas vaut 1000 ») ne décrit plus le défaut : le présent ADR le constate, sans modifier le corps d'ADR-0075.
6. **Budget.** La ligne « Rescoring de N = 1000 : < 1 ms à chaud, ~10 ms à froid » de `docs/03-recherche.md:144` est **remplacée** par les mesures de MES-03, à N = 1000 et à N = 5000, à chaud et à froid. Le dépassement du budget indicatif est **assumé** au profit de la fidélité. La cible réelle reste la latence de `docs/08-invariants-et-tests.md` (p95 au plus égal à pgvector), mesurée en P8 et P9. Le coût des clés au rescoring, distinct du coût des produits scalaires, relève d'un ADR séparé, en statut `Proposé`.
7. **Portée.** La cible de fidélité est atteinte sur l'échantillon, pour des pools d'au plus 50 000 passages. La confirmation à 8,58 M passages est renvoyée à P9 (ADR-0076, décision 4).

Ce qui n'est **pas** décidé ici : la valeur du centrage des codes binaires, que MES-12 instruit et qui demanderait un ADR remplaçant ADR-0069 décision 6 ; la parallélisation du rescoring ; le coût des clés, traité par un ADR séparé ; la forme du plan exposé, qui relève de P7.

## Conséquences

- **Positives** :
  - la cible de fidélité de 0,99 est atteinte à N = 5000 dans les deux campagnes et pour toutes les structures (MES-01), alors qu'aucune autre valeur mesurée ne l'atteint dans les deux ;
  - le défaut reste **lisible** : la valeur est écrite dans `index.json` à la création, et aucun code ne la recalcule ni ne la masque ;
  - aucun changement de contrat de configuration, aucune constante nouvelle, aucun repli : le comportement d'un `index.json` incomplet reste le refus d'ADR-0031 et de la validation de type ;
  - aucun index existant ne change de comportement sans `PATCH` explicite : pas de génération, pas de renormalisation, donc pas de reprise de mesure surprise en P4 ;
  - le gabarit et les exemples cessent de contredire la décision : un index neuf créé depuis la documentation porte la valeur retenue.
- **Négatives / coûts** :
  - le rescoring à chaud double (29,8 → 61,1 ms p50, p95 de 90,5 à 186,9 ms) et le budget indicatif de `docs/03-recherche.md` est dépassé sans être remplacé par une cible plus permissive ;
  - tout index créé ensuite avec les valeurs par défaut passe en EXACT jusqu'à 5000 passages : le coût de ces requêtes est celui du calcul exact, pas celui du scan binaire ;
  - une requête groupée rescore désormais 5000 candidats, quel que soit `k`, alors qu'ADR-0075 décrivait un pire cas à 1000 ;
  - monter un index existant est une opération en deux `PATCH` dont l'oubli du second laisse un index à N = 5000 et un seuil à 1000, donc un comportement différent de celui d'un index créé avec ce défaut ;
  - la valeur ne s'applique pas aux index déjà présents dans `data/` : les mesures à 1000 restent valides et deux régimes coexistent tant que les copies de mesure ne sont pas recréées.
- **À surveiller** :
  - MES-01 à 8,58 M passages en P9 : si le recall@10 au défaut tombe sous 0,99, N devra monter, ou le centrage devenir le levier, et cela demandera un nouvel ADR ;
  - MES-03 et MES-02 en conteneur en P8 : si le p95 dépasse celui de pgvector, la décision de fidélité devra être réarbitrée contre la cible de latence de `docs/08` ;
  - le coût des clés au rescoring (4,0 ms à N = 1000, 20,1 ms à N = 5000, MES-03), qui croît avec N et n'est pas couvert ici ;
  - toute tentative de matérialiser la valeur dans le code (constante ou repli) contredirait la décision 2 et demanderait un nouvel ADR.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Garder N = 1000 | recall@10 de 0,9757 (principale) et 0,9760 (filtrée), sous la cible de 0,99 de `docs/08` ; la fidélité est le motif premier du scan binaire |
| N = 2000, moins coûteux | 0,9887 sur la campagne principale, donc encore sous la cible ; seule la campagne filtrée atteint 0,99, et la cible porte sur la moyenne globale de la campagne principale |
| Constante `DEFAULT_RESCORE_CANDIDATES` dans le code (option (ii)) | Second défaut à côté de la configuration stockée : la valeur effective cesserait d'être lisible dans `index.json`, et deux sources divergeraient au premier changement |
| Rendre la clé optionnelle avec un repli à la lecture (option (iii)) | Change le contrat de configuration, crée un défaut matérialisé à la lecture au lieu de la création, et fait diverger deux index de même `index.json` selon la version du binaire |
| Migrer ou renormaliser les index existants vers 5000/5000 | Aucun index n'est en production, et une migration changerait le comportement sans action du propriétaire ; P9 crée l'index de Legifacile avec le défaut, ce qui suffit |
| Faire suivre le seuil à un `PATCH` de N | Déjà écarté par ADR-0074 : le seuil changerait sans `PATCH` du seuil, et la valeur effective ne serait plus celle écrite |
| Compenser le coût par le centrage des codes | MES-12 instruit le centrage, qui demande un ADR remplaçant ADR-0069 décision 6 ; le mêler ici ferait dépendre le défaut de N d'une décision non prise |
| Baisser `k` ou `candidates_per_branch` pour tenir le budget | Change le contrat de l'API et le rappel de l'étape de regroupement, sans lien avec la fidélité du scan binaire |

## Références

- Mesures : `docs/mesures/MES-01-fidelite-scan-binaire.md` (recall@10 par N et par structure), `docs/mesures/MES-02-latence-scan-binaire.md` (scan), `docs/mesures/MES-03-latence-rescoring.md` (rescoring à chaud, à froid, coût des clés), `docs/mesures/MES-12-centrage-codes-binaires.md`.
- Conception : `docs/03-recherche.md:52`, `:57-70`, `:136-150` ; `docs/04-donnees-et-ingestion.md:26-35` ; `docs/05-api.md:195-206` ; `docs/08-invariants-et-tests.md:80-90` ; `docs/00-glossaire.md:20-21` ; `docs/09-plan-de-developpement.md:74` ; `docs/phases/P4-recherche-rapide.md:84-92`, `:130-137`.
- Code : `crates/crible-core/src/config.rs:428` (`rescore_candidates` exigé) ; `crates/crible-core/src/config.rs` (`SearchConfig`, sans `Default`) ; `tools/gen_synth.py` (gabarit).
- ADR liés : ADR-0074 (décisions 2, 4 et 5), ADR-0075 (décision 4), ADR-0076 (décisions 1 et 4), ADR-0077 (protocole de latence), ADR-0031 (clé inconnue dans un `PATCH`), ADR-0069 (décision 6, codes binaires), ADR-0071 (seuil entre parcours du scan).
- Question tranchée : Q-01 (N par défaut et cible de fidélité du scan binaire).
- Note de traçabilité : ADR-0077 cite `docs/03-recherche.md:144` pour la ligne de budget « Rescoring de N = 1000 : < 1 ms à chaud, ~10 ms à froid ». Après application du présent ADR, cette ligne porte la mesure à N = 1000, et la ligne suivante la mesure à N = 5000. Le corps d'ADR-0077 n'est pas modifié.
