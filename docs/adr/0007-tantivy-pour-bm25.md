# ADR-0007 — tantivy pour la recherche BM25

- **Statut** : Accepté
- **Date** : 2026-10-18
- **Décideurs** : propriétaire
- **Proposé par** : brainstorm (après lecture du code de bm25x)

## Contexte

Il faut une branche lexicale BM25 : pour l'hybride, et pour les identifiants et les articles courts que le dense rate. Le premier candidat était bm25x (LightOn, v0.1). Sa lecture a révélé trois problèmes :
- `delete` **renumérote** tous les documents suivants, ce qui est incompatible avec des ids partagés ;
- le stemmer Snowball et les stopwords sont **anglais et codés en dur**, et le tokenizer n'est pas personnalisable ;
- aucun élagage (WAND) : la recherche parcourt toutes les listes.

## Décision

**tantivy** (≥ 0.27). Il apporte l'analyse française intégrée (stemmer et stopwords), un tokenizer personnalisable, des ids stables avec suppression logique, l'élagage block-WAND et `explain()`.

tantivy n'est qu'**un index de texte indexé par notre id interne** (champ `iid`). C'est **notre bitmap des vivants qui décide de ce qui est visible**. Le commit tantivy porte notre numéro de génération, et l'index se reconstruit à partir du texte stocké (compaction, reprise après incident).

## Conséquences

- **Positives** : français pris en charge, maturité (base de Quickwit et de pg_search), requêtes de phrase possibles plus tard.
- **Négatives** :
  - tantivy a son propre cycle de vie (segments, commits), à garder synchronisé avec le nôtre ;
  - avec notre collecteur filtré par le pool, l'élagage s'applique probablement mal (à mesurer).
- **À surveiller** : MES-06 (taille, durée de construction, latence filtrée). Si le filtrage est trop lent, repli sur un index inversé maison par segment (nouvel ADR).

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| bm25x | Ids renumérotés à chaque suppression, analyse anglaise seulement, v0.1. Les manques pourraient faire l'objet de PR chez LightOn |
| SQLite FTS5 | Pas d'élagage, filtrage par le pool malcommode, pas de racinisation française |
| Index maison | Réécrit ce que tantivy fait déjà ; ce n'est pas le cœur de Crible |

## Références

`docs/03-recherche.md` (branche lexicale) ; `docs/02-architecture.md` (étape 4 du commit) ; ADR-0020 ; MES-06, MES-11.
