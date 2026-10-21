# ADR-0032 — Accès des agents à la production Legifacile : SSH, lecture seule, informations hors git

- **Statut** : Accepté (décision du propriétaire le 2026-10-19)
- **Date** : 2026-10-19
- **Décideurs** : propriétaire
- **Proposé par** : propriétaire (mesurer directement sur le Mac mini, garder les accès confidentiels)

## Contexte

- **La mesure de référence MES-00 doit se faire sur la production.**
  - La copie locale de Legifacile est figée à la mi-août, sans le découpage des décisions en parties (2,02 M passages de jurisprudence contre 3,0 M en production). Elle a été mise de côté.
  - La production tourne sur un Mac mini, joignable en SSH par une clé déjà configurée.
  - C'est aussi la machine sur laquelle la v1 de Crible sera jugée (J9).
- **Les informations d'accès** (hôte, chemins, nom du conteneur) ne doivent pas être publiées. Le dépôt Crible est sur GitHub, sous licence Apache 2.0.
- **La production sert une utilisatrice réelle.** Les scripts de mesure existants peuvent y écrire : `eval_recherche.py` réécrit `docs/mcp/EVALUATION.md` par défaut.

## Décision

1. **Les accès vivent dans `.env.prod`**, à la racine du dépôt.
   - Ce fichier est **non versionné** : il est couvert par la règle `.env.*` du `.gitignore`.
   - Un modèle sans valeurs, `.env.prod.example`, est versionné.
   - Les documents, rapports, ADR et commits ne citent que les **noms** des variables (`$PROD_SSH`…), jamais leurs valeurs.
2. **Toute action d'un agent en production est en lecture seule** :
   - sessions Postgres avec `PGOPTIONS=-c default_transaction_read_only=on` (vérifié : `transaction_read_only = on`) ;
   - aucune modification de schéma, d'index, de réglage, de données ni de fichier dans le dépôt Legifacile ;
   - aucun redémarrage ni arrêt de Postgres, de Docker ou des services de production ;
   - les mesures se font hors des heures d'usage, jamais pendant la fenêtre de la tâche quotidienne.
3. **Garde-fous** :
   - `@git` vérifie avant chaque commit que le diff indexé ne contient aucune valeur de `.env.prod` ;
   - `@relecteur` en fait un point bloquant.

## Conséquences

- **Positives** : des mesures sur les vraies données et la vraie machine, sans exposer les accès.
- **Négatives** :
  - tout agent qui lit `.env.prod` envoie ces informations au fournisseur de son modèle (Ollama Cloud). L'hôte n'est joignable que par le réseau privé Tailscale, ce qui limite le risque ;
  - quelqu'un qui rejoint le projet doit obtenir le fichier auprès du propriétaire.
- **À surveiller** : toute fuite d'une valeur de `.env.prod` dans l'historique git. Si cela arrive, changer les accès concernés plutôt que réécrire l'historique (ADR-0023).

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| Mesurer sur la copie locale | Données figées à la mi-août, sans les parties des décisions ; matériel différent de la production |
| Écrire les accès dans la fiche ou dans `AGENTS.md` | Ils seraient publiés sur GitHub |
| Un rôle Postgres dédié en lecture seule | Préférable à terme, mais demande une modification de la production ; l'option de session suffit pour les mesures |

## Références

`.env.prod.example` ; `docs/phases/P0-fondations.md` (`MESURER-REFERENCE-PGVECTOR`) ; ADR-0023 ; ADR-0026.
