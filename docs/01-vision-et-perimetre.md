# 01 — Vision et périmètre

## En une phrase

Crible est un moteur de recherche **dense + BM25** pour données structurées. Il se déploie en **une instance par projet**, tourne vite sur **CPU**, calcule **exactement** quand le filtre réduit assez le pool, et il est **débuggable par construction**.

## Origine

Crible généralise le moteur de recherche de **Legifacile**, la recherche juridique sur l'open data de la DILA : 8,58 M passages encodés avec mDenseOn (768 dimensions), aujourd'hui servis par pgvector. L'objectif est de réutiliser la même architecture dans d'autres projets.

Le modèle de départ est **NextPlaid** (LightOn), mais en **vecteur dense uniquement**. Sur des données structurées, les filtres apportent l'essentiel de la précision, et le late interaction ne justifie pas son coût (5 à 25 fois le stockage du dense). Un rerank pourra venir plus tard comme étape optionnelle.

Le nom vient de « passer au crible » : **filtrer** avant de chercher, et **examiner** chaque résultat en détail.

## Ce qui distingue Crible

1. **Filtrer d'abord, chercher ensuite.** Le coût d'une requête suit la taille du pool, pas celle de la base. Sous le seuil exact, le résultat est exact par construction.
2. **Débuggable par construction.**
   - Plan d'exécution et temps de chaque étape.
   - Rang de chaque résultat dans chaque branche.
   - `explain_ids` pour savoir pourquoi un document n'est pas remonté.
   - Journal de debug, éval par rejeu.
3. **Analytics d'usage sans aucune donnée d'entrée** : sûres par construction.
4. **Sobre.** CPU seulement, un binaire, une RAM minimale (les codes binaires), tout le reste sur disque.
5. **Ingestion n'importe où.** Les lots sont produits sur un GPU loué ou en local, puis attachés après une recette automatique.

## Objectifs de la v1

- Plusieurs index par instance, chacun avec un schéma typé, et le modèle documents → passages.
- Recherche :
  - **dense** : scan binaire puis rescoring fp16, exacte sous le seuil ;
  - **lexicale** : tantivy, analyse française ;
  - **hybride** : RRF ou `relative_score` ;
  - **multi-index**.
- Filtres JSON validés contre le schéma, regroupement par document, dédoublonnage par contenu.
- Cycle de vie :
  - lots Parquet idempotents, avec recette ;
  - mises à jour quotidiennes ;
  - pierres tombales et compaction ;
  - export et instantanés.
- Interfaces : API REST `/v1` documentée en OpenAPI, CLI `crible`, client Python.
- Analytics, journal de debug, explain, éval.
- Image Docker CPU multi-architecture (arm64 et amd64).

## Hors périmètre de la v1

| Hors v1 | Pourquoi / quand |
|---|---|
| ANN (HNSW, IVF) | Le scan binaire suffit jusqu'à ~30 M passages ; IVF ensuite |
| Rerank, late interaction | Étape distante prévue plus tard (voir `03-recherche.md`) |
| Découpage dans le moteur | C'est un savoir métier : le producteur découpe |
| Serveur partagé entre projets, multi-tenant | Une instance par projet (ADR-0002) |
| Plusieurs nœuds, réplication | Un seul nœud suffit à l'échelle visée |
| Interface web, tableau de bord | Les stats passent par l'API et la CLI |
| Fraîcheur en temps réel (< 1 s) | Un lot est visible en quelques secondes à quelques minutes |
| Encodage GPU dans le serveur | Les producteurs encodent ; le serveur n'encode que les requêtes (et les petits lots en ligne) |
| Champs multivalués | Question ouverte Q-03 |

## Critère de réussite de la v1 (jalon J9)

La recherche de Legifacile est servie par Crible, sur le Mac mini M1 16 Go en Docker, avec 6 index : `articles`, `jorf`, `decisions`, `titres`, `sarde`, `dole`. Quatre conditions :

- **Rappel** au moins égal à celui de pgvector sur le jeu de référence. La base de comparaison est la mesure MES-00, faite en phase 0.
- **Latence p95** au plus égale à celle de pgvector.
- **Tâche quotidienne** branchée et stable pendant 14 jours consécutifs.
- **Retour arrière** vers pgvector possible pendant toute la période de bascule.

## Contraintes

| Contrainte | Valeur |
|---|---|
| Machine de production | Mac mini M1 16 Go. Docker tourne dans une VM Linux (~8 Go alloués aujourd'hui), sans accès au GPU |
| Base de référence | 8,58 M passages, 768 dimensions, mDenseOn. Prompts `query: ` et `document: `, pooling CLS + normalisation L2 |
| Bases futures | Plus grosses que Legifacile |
| Mises à jour | Quotidiennes : ~1 000 à 3 000 passages par nuit, ~1 % de passages morts par mois. Des campagnes de ré-encodage ponctuelles |
| Producteurs | Python : PyTorch sur MPS en local, CUDA sur OVH |
| Développement | Agents opencode (DeepSeek V4.1 Flash via Ollama). Le propriétaire est l'architecte et valide les jalons |

## Principes de conception

1. **Mesurer avant d'affirmer.** Toute affirmation de performance renvoie à une mesure MES-xx enregistrée.
2. **Ce qui a coûté du GPU est figé et conservé.** Tout le reste se reconstruit en local : codes binaires, bitmaps, index lexical.
3. **Une lecture n'est jamais bloquée par une écriture.**
4. **Sûr par construction.** Une donnée qui ne doit pas être stockée n'atteint jamais le composant qui stocke.
5. **Pas de fonctionnalité spéculative.** Chaque élément répond à un besoin de Legifacile ou protège un invariant.
6. **Vérifiable sans lire le code.** Le contrôle passe par les tests d'invariants, l'éval, les mesures et les rapports de jalon.
7. **Toute décision est tracée.** Chaque choix qui engage le projet est un ADR (`docs/adr/`). La mémoire du projet ne dépend ni d'un agent ni d'une conversation (ADR-0021).

## Rôles

- **Propriétaire.** Architecte du projet : il arbitre les écarts de conception et valide chaque jalon. Lui seul fusionne une phase dans `main`.
- **Agents opencode.** Ils implémentent les tâches, une à la fois, selon `AGENTS.md`.
