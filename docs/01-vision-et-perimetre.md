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
- Interfaces : API REST `/v1` documentée en OpenAPI, CLI `crible`, client Python, **tableau de bord intégré** au serveur sous `/ui` (ADR-0130).
- Analytics, journal de debug, explain, éval.
- Image Docker CPU multi-architecture (arm64 et amd64).

## Hors périmètre de la v1 (J10, ADR-0129)

| Hors v1 | Pourquoi / quand |
|---|---|
| ANN (HNSW, IVF) | Le scan binaire suffit jusqu'à ~30 M passages ; IVF ensuite |
| Rerank, late interaction | Étape distante prévue plus tard (voir `03-recherche.md`) |
| Découpage dans le moteur | C'est un savoir métier : le producteur découpe |
| Serveur partagé entre projets, multi-tenant | Une instance par projet (ADR-0002) |
| Plusieurs nœuds, réplication | Un seul nœud suffit à l'échelle visée |
| Fraîcheur en temps réel (< 1 s) | Un lot est visible en quelques secondes à quelques minutes |
| Encodage GPU dans le serveur | Les producteurs encodent ; le serveur n'encode que les requêtes (et les petits lots en ligne) |
| Champs multivalués | Question ouverte Q-03 |

**Le tableau de bord n'est plus hors périmètre.** La ligne « Interface web, tableau de bord », qui renvoyait les statistiques à l'API et à la CLI, a quitté ce tableau le 2026-11-02 : ADR-0130 décide un tableau de bord visuel en lecture seule, intégré à `crible serve` et servi sous `/ui`, en phase P9. C'est un écart de périmètre assumé par le propriétaire.

## Critère de réussite de la v1 (jalon J10)

ADR-0129 sépare la validation de Crible de la mise en service de Legifacile. La recherche de Legifacile est servie par Crible, avec six index : `articles`, `jorf`, `decisions`, `titres`, `sarde`, `dole`. Six conditions, toutes portées par la phase P10 « Validation à l'échelle réelle » :

- **Configs** : les six configs d'index sont définies et validées par le propriétaire.
- **Import** : les 8,58 M passages sont importés ; les comptes par index sont égaux à ceux de pgvector ou les écarts sont expliqués ; MES-07 et MES-09 sont faites à l'échelle réelle.
- **Jeu de référence** : il est validé et identique pour pgvector et Crible.
- **Rappel et fidélité** : rappel d'au moins **0,482** sur les 56 questions satisfaisables (`Q30` à part) et cibles par corpus (`juri_texte_integral` ≥ 0,290, `textes_vigueur` ≥ 0,750) ; fidélité d'au moins **0,99** à 8,58 M passages, au N par défaut (5000, ADR-0078). La base de comparaison est la mesure MES-00, faite en phase 0.
- **Latence p95** : contraignante, au plus égale à celle de pgvector, **seulement** sur le Mac mini M1 16 Go en conteneur, par `MESURER-SUR-MAC-MINI` ; indicative ailleurs, machine de développement comprise.
- **Bilan** : le bilan de Crible est écrit et validé.

La **tâche quotidienne stable 14 jours** et le **retour arrière testé** ne conditionnent plus ce jalon : ils appartiennent au projet Legifacile et à son propre bilan (`docs/passation-legifacile.md`).

## Contraintes

| Contrainte | Valeur |
|---|---|
| Machines (ADR-0129) | **Développement** : MacBook Air M3 (`Mac15,12`, 8 cœurs, 24 Gio), sous Docker Desktop ; import de la base complète et comparaison avec pgvector, mesures indicatives. **Latence contraignante** : Mac mini M1 16 Go, en conteneur (VM Linux, ~8 Go alloués aujourd'hui, sans accès au GPU) ; le propriétaire y installe Crible après confirmation de l'absence de régression. **Production officielle** : non actée, probablement un serveur OVH, hors v1 |
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
