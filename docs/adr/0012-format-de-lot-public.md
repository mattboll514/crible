# ADR-0012 — Format de lot public (Parquet + manifeste), format interne privé

- **Statut** : Accepté
- **Date** : 2026-10-18
- **Décideurs** : propriétaire
- **Proposé par** : brainstorm (besoins d'export, d'import et d'ingestion sur OVH)

## Contexte

On veut quatre choses :
- encoder sur un GPU loué (OVH) et rapatrier les vecteurs ;
- faire le quotidien en local ;
- sauvegarder ;
- migrer entre machines (x86 et ARM).

Exposer les fichiers internes figerait le format interne, et obligerait les producteurs à écrire du Rust.

## Décision

- **Format public, le « lot »** : un dossier qui contient `manifest.json` (modèle, découpeur, gabarit, comptes, checksums, `batch_id`, mode `upsert` ou `replace_all`) et des fichiers Parquet (`documents`, `passages` avec texte et vecteur, `deletions`).
  - Le vecteur est un `fixed_size_binary` en fp16 little-endian.
  - Le même format sert à l'ingestion, à l'import, à l'export et au transfert.
- **Format interne privé** : segments, SQLite, tantivy. Il peut évoluer d'une version à l'autre, et c'est le serveur qui convertit un lot à l'attache.
- Les lots sont **idempotents** par `batch_id` (INV-10).

## Conséquences

- **Positives** :
  - les producteurs peuvent être écrits en n'importe quel langage (Python avec pyarrow) ;
  - les lots s'inspectent avec DuckDB ;
  - l'export est portable d'une version de Crible à l'autre.
- **Négatives** : une conversion à l'attache, plus coûteuse qu'une simple copie de fichiers.
- **À surveiller** : la durée d'attache à grande échelle (MES-08, `IMPORTER-BASE-COMPLETE`).

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Exposer les segments internes | Fige le format interne, impose du Rust aux producteurs |
| JSONL + vecteurs en base64 | Volumineux, lent, pas de types |

## Références

`docs/04-donnees-et-ingestion.md` (format de lot v1, sémantique d'attache) ; `docs/format-lot-v1.md` (rédigé en P0).
