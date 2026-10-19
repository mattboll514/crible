# ADR-0027 — Ajouter `serde_json` comme dépendance

- **Statut** : Proposé
- **Date** : 2026-10-19
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, tâche DEFINIR-CONFIG-INDEX
- **Remplace** : —

## Contexte

La fiche `DEFINIR-CONFIG-INDEX` demande d'analyser et de valider des configurations d'index et des filtres écrits en JSON. `docs/05-api.md` fixe JSON comme format du corps des requêtes et des réponses de l'API. La tâche `DEFINIR-FORMAT-LOT` lit aussi `manifest.json`.

`docs/07-stack-et-deploiement.md` liste seulement `serde` et `toml` pour la configuration. La règle 12 des consignes du projet impose un ADR pour toute dépendance absente de `docs/07`. C'est un **écart à la conception** : le besoin est certain, mais la dépendance n'est pas autorisée aujourd'hui.

`serde_json` est le compagnon naturel de `serde` : mêmes traits `Serialize` et `Deserialize`, même modèle de données. Sa table `Map` est triée par défaut, ce qui sert au JSON canonique de l'ADR-0019 et à la forme canonique de l'empreinte du modèle (ADR-0028).

## Décision

Ajouter **`serde_json`** aux dépendances Rust de `docs/07-stack-et-deploiement.md`, dans la ligne « Config », aux côtés de `serde` et `toml`. Le périmètre couvre l'analyse et la sérialisation JSON de l'API, des configurations d'index, des filtres et des manifestes de lot.

Ce qui n'est pas décidé ici : aucune autre dépendance n'est ajoutée, et le choix de `serde_json` ne préjuge pas de la bibliothèque utilisée plus tard pour un JSON à très haut débit.

## Conséquences

- **Positives** :
  - une seule pile de sérialisation pour la configuration TOML, les corps JSON et les manifestes ;
  - les dérivations `serde` existantes fonctionnent sans adaptation ;
  - les clés sont triées par défaut, ce qui aide le JSON canonique.
- **Négatives / coûts** :
  - une dépendance de plus à compiler et à auditer ;
  - `serde_json` n'est pas le plus rapide sur de très gros documents JSON.
- **À surveiller** : si une mesure montre que l'analyse JSON devient un coût notable sur le chemin des requêtes, rouvrir la question avec une mesure à l'appui.

À l'acceptation, `docs/07-stack-et-deploiement.md` doit être mis à jour dans un commit qui cite `ADR-0027`.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| `simd-json` | Plus rapide, mais API et modèle de données différents de `serde`, dépendance à `unsafe` et emploi justifié seulement par une mesure. Aucun besoin mesuré aujourd'hui. |
| Analyseur JSON maison | Code à maintenir et à tester, risque d'écarts de conformité, aucune valeur ajoutée par rapport à une crate éprouvée. |

## Références

`docs/05-api.md` (format JSON de l'API) ; `docs/07-stack-et-deploiement.md` (dépendances) ; `docs/04-donnees-et-ingestion.md` (configuration d'index) ; ADR-0019 ; ADR-0028 ; fiche `DEFINIR-CONFIG-INDEX`.
