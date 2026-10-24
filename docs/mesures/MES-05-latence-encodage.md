# MES-05 — Latence d'encodage d'une requête (natif, CPU du Mac)

## Campagne du 2026-10-24

- Tâche : `MESURER-PARITE-ENCODEUR` (phase P2, fiche
  `docs/phases/P2-encodeur-et-recette.md`). Mesure demandée avant le jalon J2.
- Références : `docs/08-invariants-et-tests.md` (MES-05), `docs/03-recherche.md`
  (« Budget de latence indicatif », encodage de la requête : 20 à 50 ms),
  `docs/07-stack-et-deploiement.md` (`[encoder]`), ADR-0051.
- Commit : `2fd3c29b64a9f5ce02d83cbdd9df49a5ce6d540a`, branche
  `tache/MESURER-PARITE-ENCODEUR`. **Aucun code de production n'a été modifié
  pendant la campagne.** Seuls ont été ajoutés l'exemple
  `crates/crible-encode/examples/measure_queries.rs`, les scripts
  `tools/bench/mes04/` et leur entrée pytest dans `tools/pyproject.toml`.
- Machine : MacBook Air Apple M3 (8 cœurs : 4 performance, 4 efficacité), 24 Go
  de RAM (25 769 803 776 octets), macOS 15.7.7 (24G720), APFS. Mesure **native**,
  aucun conteneur. Binaire compilé par `cargo build --release`.
- Charge ambiante : la machine **n'était pas au repos**. La moyenne de charge
  était de 2,95 à 4,45 sur 8 cœurs pendant les quatre passes chronométrées (Arc,
  opencode et Ollama actifs). Aucun autre travail lourd n'a été lancé par la
  campagne. Les p50 et p95 sont proches d'une passe à l'autre (§ 3), mais cette
  charge n'est pas isolée et peut expliquer une partie de la queue.
- Jeu de données : les **300 requêtes réelles** de
  `data/legifacile-sample/queries.jsonl` (SHA-256 `fdaa2794…`), de 8 à 92
  tokens, moyenne 26,17. La boucle parcourt ces 300 textes en cycle ; le cache
  des requêtes est **désactivé** (capacité 0), donc aucun appel n'est servi en
  mémoire. Les compteurs le prouvent : 0 succès et 1 301 manques par passe
  (1 premier appel + 300 de préchauffage + 1 000 mesurés).
- Modèle : `data/models/mdenseon/`, révision
  `a5fdb000f7a21da96c3bddde3a782ef777316df3` ; `model.int8.onnx` (293 Mio) pour
  l'INT8, `model.onnx` fp32 (1,1 Gio) pour le fp32 ; `tokenizer.json` commun.
  Export transformers 5.14.1, torch 2.14.1, onnxruntime 1.31.0 (ADR-0050).

## 1. Protocole

Le harnais est `crates/crible-encode/examples/measure_queries.rs`, mode
`latency`. Il appelle `QueryEncoder::encode_query`, donc il chronomètre ce que
le moteur fait par requête : lecture de `model.json` (empreinte), consultation
du cache, tokenisation avec le prompt `query: `, exécution ONNX, pooling CLS,
normalisation L2. Le cache est construit avec une capacité de 0 (désactivé).

```sh
cargo build --release --example measure_queries -p crible-encode
export ORT_DYLIB_PATH=$(cd tools && uv run python -c \
  'import onnxruntime, pathlib; print(next((pathlib.Path(onnxruntime.__file__).parent / "capi").glob("libonnxruntime*.dylib")))')

for precision in int8 fp32; do
  for run in 1 2; do
    ./target/release/examples/measure_queries latency \
      --model data/models/mdenseon --queries data/mes04/query-texts.jsonl \
      --precision "$precision" --warmup 300 --count 1000 \
      --out "data/mes04/latency-$precision-run$run.json"
  done
done
```

- **Préchauffage** : 300 encodages non chronométrés par passe, soit un cycle
  complet des 300 textes. Les 1 000 encodages mesurés suivent.
- **Répétitions** : deux passes complètes par précision, dans deux processus
  distincts, pour voir la reproductibilité.
- **Premier appel** : chronométré à part. Il porte la vérification du hash du
  graphe, le chargement du tokenizer, la création de la session ONNX et la
  première exécution. C'est le coût « à froid du processus », payé une fois par
  instance et par précision, pas par requête.
- **Froid du système** : non mesuré. macOS ne purge pas son cache de pages sans
  `root`, et aucune purge n'a été tentée. Le « premier appel » ci-dessus n'est
  donc pas un accès disque froid contrôlé.
- **Percentiles** : rang le plus proche (`ceil(p / 100 × n)`), comme MES-13,
  MES-15 et MES-04.
- **Réglages de session** : explorés **hors du code Rust**, avec
  `onnxruntime` 1.31.0 en Python (`tools/bench/mes04/threads.py`). Ce script
  chronomètre `session.run` seul, requêtes déjà tokenisées, 100 préchauffages et
  1 000 passes par réglage. Il compare des réglages entre eux ; ses valeurs
  absolues ne sont pas comparables à celles du harnais Rust, qui inclut le
  tokenizer et qui a tourné sous une charge différente.

## 2. Résultats — encodage complet, cache désactivé

En millisecondes, 1 000 encodages mesurés par passe après 300 préchauffages.

| Passe | Précision | Premier appel | Minimum | p50 | p95 | p99 | Maximum | Moyenne |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| 1 | INT8 | 1 013,8 | 4,138 | **8,646** | **22,490** | 28,159 | 52,002 | 10,234 |
| 2 | INT8 | 972,1 | 4,155 | **8,825** | **22,990** | 27,860 | 28,996 | 10,478 |
| 1 | fp32 | 1 776,7 | 10,253 | **20,978** | **55,763** | 70,212 | 74,683 | 24,872 |
| 2 | fp32 | 1 786,2 | 10,279 | **21,054** | **56,361** | 70,096 | 74,391 | 24,958 |

**Répétabilité.** Le p50 varie de +2,1 % en INT8 et de +0,4 % en fp32 entre les
deux passes ; le p95 de +2,2 % et +1,1 %. Les deux passes donnent donc la même
image à quelques pour cent.

**Effet de la longueur.** Les 300 requêtes font 8 à 92 tokens. La boucle les
parcourt en cycle, donc la série mélange les longueurs : le minimum (4,1 ms en
INT8, 10,3 ms en fp32) correspond aux requêtes courtes, et la queue aux requêtes
longues et aux accidents d'ordonnancement. Le p95 n'est pas une queue de
production isolée : c'est la queue de ce jeu de 300 requêtes réelles.

**Premier appel.** 1,01 s en INT8 et 1,78 s en fp32. Il domine largement une
requête, mais il est payé une fois par processus. La différence entre les deux
précisions suit la taille du graphe (293 Mio contre 1,1 Gio), donc la
vérification du hash et le chargement de la session, pas l'encodage.

## 3. Résultats — réglages de session explorés en Python

`session.run` seul, 1 000 passes par réglage, requêtes pré-tokenisées. Le réglage
« default » laisse `intra_op_num_threads` et le niveau d'optimisation aux
valeurs par défaut d'ONNX Runtime.

| Précision | Réglage | p50 | p95 | p99 |
|---|---|---:|---:|---:|
| INT8 | défaut | **8,881** | **23,460** | 28,948 |
| INT8 | 8 threads | 12,882 | 29,885 | 40,078 |
| INT8 | 4 threads | 9,605 | 25,246 | 31,257 |
| INT8 | 1 thread | 19,368 | 58,063 | 74,806 |
| INT8 | optimisation `basic` | 11,921 | 32,125 | 41,193 |
| fp32 | défaut | **24,187** | **60,474** | 75,219 |
| fp32 | 8 threads | 40,549 | 112,989 | 226,369 |
| fp32 | 4 threads | 28,631 | 73,331 | 91,318 |
| fp32 | 1 thread | 53,605 | 158,798 | 205,223 |
| fp32 | optimisation `basic` | 23,465 | 58,705 | 74,020 |

Le réglage par défaut est le meilleur dans les deux précisions. Forcer 8 threads
est nettement plus lent : le M3 a 8 cœurs, mais 4 seulement sont performance, et
la session sature la machine. Quatre threads restent au-dessus du défaut. Un
seul thread double presque le p50. Le niveau `basic` n'apporte rien : il est plus
lent en INT8 et à peine différent en fp32, dans le bruit.

**Recommandation, à valider par le propriétaire** : ne pas régler
`intra_op_num_threads` ni le niveau d'optimisation dans `crible-encode`, et
laisser les défauts d'ONNX Runtime. C'était l'objet du report de réglage de
`CHARGER-MODELE` : la mesure dit que le défaut est déjà le bon. Si un réglage
devait être ajouté plus tard, ce serait pour **brider** la consommation (4
threads), pas pour accélérer.

## 4. Conclusion

Ce que la mesure permet d'affirmer :

1. **En INT8, l'encodage complet d'une requête coûte 8,6 à 8,8 ms en p50 et 22,5
   à 23,0 ms en p95** sur ce Mac M3, cache désactivé, en natif. C'est dans le
   budget indicatif de 20 à 50 ms de `docs/03` (p95 sous la borne haute).
2. **En fp32, il coûte 21,0 ms en p50 et 55,8 à 56,4 ms en p95.** Le p50 tient
   dans le budget, le p95 le dépasse d'environ 13 %. Le budget de `docs/03` est
   écrit pour l'INT8 ; le fp32 est un réglage d'évaluation et de diagnostic
   (ADR-0051), pas le mode de production prévu.
3. **Le premier appel coûte environ 1 s en INT8 et 1,8 s en fp32**, une fois par
   processus et par précision : hash du graphe, tokenizer et création de la
   session. Ce n'est pas une latence de requête.
4. **Les réglages par défaut d'ONNX Runtime sont les meilleurs** parmi ceux
   essayés : 8 threads, 4 threads, 1 thread et l'optimisation `basic` sont tous
   plus lents ou sans gain. La note « threads et niveau d'optimisation non
   réglés, reportés à MES-05 » se referme donc sans changement de code.

Ce que la mesure ne permet pas d'affirmer :

- **rien sur le conteneur** : Docker et la VM de `docs/07` ne sont pas mesurés
  (P8). Le budget final est « sur le Mac mini en conteneur » (`docs/08`), pas sur
  ce MacBook Air natif ;
- **rien sur la charge concurrente** : une seule requête à la fois, aucun serveur
  HTTP, aucun autre index actif ;
- **rien sur un froid système contrôlé** : le cache de pages n'a pas été purgé ;
- **rien sur les requêtes de plus de 92 tokens** : la campagne emploie les 300
  requêtes réelles, dont la plus longue fait 92 tokens. Le chemin de troncature
  à `max_tokens` n'est pas chronométré ;
- **rien sur un autre CPU** : un seul modèle de machine.

**Comparaison au budget et recommandation.** L'INT8 tient le budget ; le fp32 le
dépasse en p95. Or MES-04 montre que l'INT8 perd 20,8 % du top-10 exact. Le choix
n'est donc pas un arbitrage latence seule : si le propriétaire retient le fp32
pour les requêtes (recommandation de MES-04), il faut soit accepter un p95
d'environ 56 ms, soit rouvrir une tâche de quantification ou d'optimisation du
graphe fp32. Aucune modification n'a été faite ici pour améliorer un chiffre.

## 5. Données brutes

`docs/mesures/MES-05-latence-encodage.2026-10-24.json` : les quatre passes avec
leur série complète de 1 000 durées en millisecondes, les percentiles, le premier
appel, les compteurs de cache, et l'exploration des réglages de session. Les
fichiers produits par le harnais restent dans `data/mes04/` (gitignoré), ainsi
que `conditions.txt` (date, charge, `vm_stat`).

## 6. Constats hors mesure

- `measure_queries` lit `model.json` à chaque appel pour calculer l'empreinte.
  C'est le comportement de `QueryEncoder::encode_query`, donc la mesure le
  compte ; le coût est inclus dans les 8,6 ms et n'a pas été isolé.
- Le cache LRU n'est pas mesuré en régime : capacité 0 pour voir l'encodage. Un
  cache de 1 024 entrées servirait les répétitions en mémoire, et MES-05 ne dit
  rien de son taux de succès réel (ADR-0051 le cite comme signal à surveiller).
- L'exploration Python a tourné pendant que la machine était chargée (moyenne
  de charge supérieure à 5), après les quatre passes Rust. Ses valeurs absolues
  sont donc plus hautes que celles du § 2 ; seule la comparaison entre réglages
  est retenue.
