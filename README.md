# Crible

> **Passer au crible** : filtrer d'abord, chercher ensuite, et pouvoir examiner chaque résultat.

Crible est un moteur de recherche **dense + BM25** pour données structurées.

- **Une instance par projet** : un binaire Rust, sur CPU uniquement.
- **Filtres d'abord** : le coût d'une requête suit la taille du pool filtré. Le résultat est exact quand ce pool est petit.
- **Recherche dense** : codes binaires en RAM, puis rescoring fp16. **Lexicale** : BM25 tantivy, avec analyse française. **Hybride** : RRF ou `relative_score`. Recherche **multi-index**.
- **Ingestion par lots Parquet** produits n'importe où (GPU loué, Mac local), avec une recette automatique et des mises à jour quotidiennes idempotentes.
- **Débuggable par construction** : plan d'exécution, temps par étape, rang de chaque résultat dans chaque branche, `explain_ids`, journal de debug, éval par rejeu.
- **Analytics d'usage sans aucune donnée d'entrée.**

Premier cas réel : **Legifacile**, une recherche juridique sur l'open data de la DILA (8,58 M passages, mDenseOn 768 dimensions). Sa migration depuis pgvector valide la v1.

## État

**Conception terminée, développement non commencé.** Le plan compte 10 phases (P0 → P9), chacune fermée par un jalon validé par le propriétaire, et 83 tâches. Voir [`docs/suivi.md`](docs/suivi.md).

## Documentation et décisions

Le point d'entrée est [`docs/README.md`](docs/README.md), avec un parcours de lecture d'environ 45 minutes.

Chaque décision du projet est un **Architecture Decision Record** dans [`docs/adr/`](docs/adr/README.md) : une décision par fichier, avec son contexte, ses conséquences et les alternatives écartées. Ces fichiers sont la mémoire du projet, accessible à tous sans dépendre d'un agent ni d'une conversation. N'importe qui propose (`/adr <sujet>`) ; le propriétaire accepte ou rejette.

## Développer avec opencode + Ollama

Le développement se fait avec [opencode](https://opencode.ai). La configuration est dans [`opencode.json`](opencode.json), les règles dans [`AGENTS.md`](AGENTS.md), les agents dans `.opencode/agents/` et les commandes dans `.opencode/commands/`.

**Le guide complet est dans [`docs/guide-opencode.md`](docs/guide-opencode.md)** : installation, Ollama, ColGrep, plugins, gestion des sessions et du contexte, dépannage. En résumé :

```bash
brew install anomalyco/tap/opencode-v2         # opencode v2 (ADR-0025) ; la config du dépôt reste au format v1, lu par la v2
ollama signin && ollama pull deepseek-v4.1-flash:1017-cloud
colgrep --install-agent                        # modèle local de ColGrep-Agent (~2,5 Go)
cd crible && opencode                          # l'agent par défaut est l'orchestrateur
```

Il te faut aussi Rust stable (`rustup`), `uv`, et ONNX Runtime à partir de la phase 2.

**L'orchestrateur** (agent par défaut) fait exécuter chaque tâche par des sous-agents, selon un pipeline fixe (ADR-0023) :

```
@git debut-tache → @testeur → @dev → @relecteur → suivi → @git commit → @git fin-tache
```

Les commandes : `/etat`, `/tache NOM`, `/jalon J<n>`, `/valider J<n>`, `/adr <sujet>`, `/mesurer MES-xx`, `/chercher <question>`, `/relire`, `/git <opération>`.

### Rôle du propriétaire

Le propriétaire :
- discute avec l'orchestrateur ;
- accepte ou rejette les ADR `Proposé` ;
- valide chaque jalon, après avoir lu le rapport et rejoué la démo (`/valider J<n>`) ;
- confirme les push.

L'agent `@git` fait toutes les opérations git, et ne fusionne dans `main` que sur ordre du propriétaire.

## Organisation du dépôt (cible)

```
crates/          crible-core, crible-store, crible-encode, crible-search, crible-server, crible (CLI)
clients/python/  paquet `crible` : BatchWriter + client HTTP
tools/           export ONNX, jeux synthétiques, référence numpy, bancs de mesure
docker/          Dockerfile, compose d'exemple, config d'exemple
docs/            conception, phases, suivi, jalons, mesures
```

## Licence

[Apache 2.0](LICENSE)
