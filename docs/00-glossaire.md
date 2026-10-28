# Glossaire

Vocabulaire commun à toute la documentation, au code et aux agents. Utiliser ces termes tels quels.

| Terme | Définition |
|---|---|
| **Instance** | Un processus Crible (un conteneur) dédié à **un** projet. Un projet = une instance. |
| **Index** | Collection de passages qui partagent le même schéma, le même modèle, le même découpage et le même gabarit. Une instance contient plusieurs index. |
| **Document** | Unité métier fournie par le producteur (un article de code, une décision…). Identifiée par un id externe. C'est l'**unité de mise à jour**. |
| **Passage** | Morceau d'un document : l'unité encodée en vecteur et indexée en BM25. Clé `doc_id#ord`. |
| **Champ de document / de passage** | Métadonnée typée portée par le document (héritée par tous ses passages) ou par un passage précis (ex. `partie`). |
| **Schéma** | Liste des champs d'un index, avec leur type et leur niveau (document ou passage). |
| **Gabarit** | Règle de composition du texte encodé (ex. hiérarchie + numéro + corps). Appliqué par le producteur, tracé dans le lot. |
| **Entrée encodeur** | Texte exact passé au modèle, sans le prompt. Stocké par Crible ; c'est aussi le texte indexé en BM25. |
| **Prompt** | Préfixe ajouté au moment de l'encodage (`document: `, `query: `). Défini dans la config du modèle. |
| **Empreinte du modèle** | Hash canonique de la config du modèle (id, révision, dimensions, pooling, normalisation, prompts, hash des fichiers fp32 et tokenizer). |
| **Précision de référence** | Précision de stockage des vecteurs (fp16 ou fp32). Figée à la création de l'index. |
| **Codes binaires** | 1 bit par dimension (signe de la composante). Gardés en RAM, parcourus à chaque requête. |
| **Rescoring** | Recalcul exact (depuis les vecteurs de référence) du score des N meilleurs candidats du scan binaire. |
| **N** | Nombre de candidats re-scorés (`rescore_candidates`). Vaut 5000 par défaut (ADR-0078). |
| **Seuil exact** | Taille de pool jusqu'à laquelle (incluse) la recherche est exacte (`exact_threshold`) ; un pool de taille égale au seuil passe par EXACT (ADR-0074). |
| **Pool** | Ensemble des passages **vivants** qui satisfont le filtre. Calculé une fois par requête, partagé par les branches dense et lexicale. |
| **Bitmap** | Ensemble d'ids internes compressé (roaring). |
| **Bitmap des vivants** | Ensemble des passages non supprimés. |
| **Colonne de valeurs** | Tableau en RAM, indexé par iid, des valeurs d'un champ numérique ou date (filtres d'intervalle). |
| **Pierre tombale** | Suppression logique d'un passage (`dead_since_gen`). L'espace est récupéré à la compaction. |
| **Id externe** | Id fourni par le producteur (ex. `LEGIARTI…`). Le seul id visible hors du moteur. |
| **Id interne (iid)** | Entier `u32` dense attribué par Crible = position dans les tableaux. Ne sort **jamais** du moteur. Stable au sein d'une époque. |
| **Segment** | Fichiers immuables contenant les vecteurs d'un lot, pour une plage contiguë d'iid. |
| **Génération** | Numéro d'état visible d'un index, incrémenté à chaque changement validé. |
| **Époque** | Jeu de fichiers d'un index (SQLite, segments, lexical) entre deux compactions. Les iid sont renumérotés à chaque nouvelle époque. |
| **Point de commit** | Le commit de la transaction SQLite qui fait passer un index de la génération G à G+1. |
| **Manifeste** | Description JSON d'une génération (dérivée de SQLite), pour les instantanés et l'inspection. |
| **Instantané de lecture** | Vue immuable d'une génération (`Arc<Snapshot>`) tenue par une requête du début à la fin. |
| **Lot** | Format d'échange public : `manifest.json` + fichiers Parquet. Produit hors du moteur puis attaché. |
| **Producteur** | Programme (Python) qui découpe, encode et écrit des lots. |
| **Attacher** | Valider puis intégrer un lot dans un index (nouvelle génération). |
| **Recette** | Contrôle d'import : ré-encodage d'un échantillon en ONNX fp32 et comparaison cosinus avec les vecteurs du lot. |
| **Compaction** | Construction d'une nouvelle époque sans les passages morts, sans GPU. |
| **Instantané (sauvegarde)** | Copie cohérente d'une époque à une génération donnée. Restauration rapide, même version de Crible. |
| **Export** | Lot `replace_all` contenant toutes les données vivantes. Sauvegarde portable, lisible avec DuckDB. |
| **Branche dense / lexicale** | Les deux recherches lancées sur le même pool. |
| **Fusion** | Combinaison des classements des deux branches (`rrf` ou `relative_score`). |
| **Regroupement** | `group_by: document` (meilleur passage par document) et `dedup: content` (passages identiques fusionnés). |
| **Analytics** | Métriques d'usage agrégées, **sans aucun contenu** (ni texte, ni valeur de filtre, ni id). |
| **Journal de debug** | Enregistrement complet de requêtes, activé à la demande, à durée de vie courte. |
| **Explain** | Diagnostic d'un document précis pour une requête (`explain_ids`) : pourquoi il est (ou n'est pas) remonté. |
| **Éval** | Rejeu d'un jeu de requêtes de référence pour mesurer rappel et latence (`crible eval`). |
| **Tâche** | Unité de travail du plan, nommée `NOM-EN-MAJUSCULE`. |
| **Orchestrateur** | Agent opencode principal. Il discute avec le propriétaire, tient le suivi et fait exécuter chaque tâche par les sous-agents. Il ne code pas (ADR-0023). |
| **Pipeline d'une tâche** | `@git debut-tache` → `@testeur` (si invariants) → `@dev` → `@relecteur` → suivi → `@git commit` → `@git fin-tache`. |
| **Sous-agent / session enfant** | Agent lancé par l'orchestrateur, dans une session au contexte neuf. Il ne renvoie qu'un compte rendu. |
| **Jalon** | Point de validation par le propriétaire en fin de phase (J0 … J9). |
| **ADR** | *Architecture Decision Record* : une décision par fichier dans `docs/adr/`, avec son contexte, ses conséquences et les alternatives écartées. Statuts : `Proposé`, `Accepté`, `Rejeté`, `Remplacé par…`, `Déprécié`. Seul le propriétaire accepte. |
| **INV-xx / MES-xx / ADR-xxxx / Q-xx** | Invariant / mesure / décision / question ouverte numérotés (voir `08-invariants-et-tests.md` et `docs/adr/README.md`). |
