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

## Campagne en conteneur (P8, ADR-0135) — 2026-11-03

Cette section **ajoute** une campagne en conteneur à la campagne native du 2026-10-24 ; elle ne
réécrit aucun de ses chiffres. Elle suit le protocole de MES-05 sans changement, et relève en plus le
nombre de fils que le processus voit, parce qu'une latence d'encodage mesurée en conteneur ne se
compare au natif que si le runtime y disposait de la même parallélie (ADR-0135, décision 5).

- **Tâche** : `MESURER-EN-CONTENEUR-MAC` (P8), passe B, mesure MES-05.
- **Commit** : `3fb33439aa3349a31593b8281920af7ce51938bb`, branche
  `tache/MESURER-EN-CONTENEUR-MAC`. **Aucun code de production n'a été modifié.** Le harnais
  `crates/crible-encode/examples/measure_queries.rs` gagne un relevé de fils (`threads` dans le
  document, une ligne à la console) ; c'est une **addition**, et elle est déclarée ici parce que le
  harnais est un fichier suivi.
- **Machine** : MacBook Air M3, 8 cœurs, 24 Go, macOS 15.7.7 (24G720), **en conteneur** : Docker
  Desktop 28.5.1, VM Linux `aarch64` de **12 529 836 032 octets (11,67 Gio)** de RAM et **8 vCPU**,
  **page de 4 096 octets**. Aucune limite `--memory`. La charge de l'hôte n'est pas relevée : limite
  déclarée.
- **Modèle** : `data/models/mdenseon`, copié dans le volume (`/data/models/mdenseon`), session
  rechargée depuis le même `model.json` ; ONNX Runtime **1.31.0** dans l'image (`/opt/ort`) comme sur
  l'hôte (`libonnxruntime.1.31.0.dylib` du wheel Python). Aucun changement de runtime ni d'export du
  graphe.
- **Requêtes** : les 300 textes de `data/mes04/query-texts.jsonl`, copiés dans le volume
  (`/data/staging/query-texts.jsonl`). Les textes ne sortent ni dans un journal ni dans un document.

### Protocole

Celui du § 1, sans écart : mode `latency` de `measure_queries`, cache de requêtes **désactivé**
(capacité 0), 300 appels de préchauffage, 1 000 appels mesurés, deux exécutions par précision, une
précision par conteneur. Le relevé de fils est pris **après le premier appel**, parce qu'ONNX Runtime
construit son pool intra-opération au premier `Run` : il donne `available_parallelism` (ce dont le
runtime dimensionne ses pools) et le nombre de tâches du processus (`/proc/self/task`), ou `null`
quand la plateforme n'a pas `/proc` — c'est le cas sur macOS.

```sh
# conteneur, image de harnais de la campagne (ENTRYPOINT remplacé)
docker run --rm -v crible-mes-p8-data:/data --entrypoint measure_queries \
    crible-mes-p8-harness:dev latency --model /data/models/mdenseon \
    --queries /data/staging/query-texts.jsonl --precision <int8|fp32> \
    --warmup 300 --count 1000 --out /data/raw/b-encode-<precision>-<n>.json

# contrôle natif, même harnais, même machine, juste avant
ORT_DYLIB_PATH=…/libonnxruntime.1.31.0.dylib \
    target/release/examples/measure_queries latency --model data/models/mdenseon \
    --queries data/mes04/query-texts.jsonl --precision <int8|fp32> \
    --warmup 300 --count 1000 --out data/mes-p8-conteneur/raw/native-encode-<precision>.json
```

Écarts et limites, déclarés :

- **Le contrôle natif n'est pas celui du 2026-10-24.** Il a été refait le 2026-11-03, sur la même
  machine mais dans l'état où elle est aujourd'hui : c'est la seule comparaison qui isole le
  conteneur, et elle est plus lente que la campagne publiée (voir plus bas).
- **Le nombre de fils n'est pas lu dans ONNX Runtime** : le runtime ne l'expose pas, et le harnais
  compte donc les tâches du processus. Il prouve que le processus n'a **pas** tourné sur un seul
  fil, il ne prouve pas que le pool intra-opération valait 8.
- **Le préchauffage et le comptage sont identiques** aux deux endroits, mais la première exécution
  par précision paie encore le chargement du graphe (voir « premier appel »).
- **Pas de mesure de la contention** : ni le scan binaire ni la recherche ne tournent pendant
  l'encodage.

### Résultats

Documents bruts : `data/mes-p8-conteneur/raw/b-encode-int8-{1,2}.json`,
`b-encode-fp32-{3,4}.json`, `native-encode-{int8,fp32}.json` (gitignorés) ; artefact versionné :
`docs/mesures/MES-05-latence-encodage.2026-11-03.conteneur.json`, écrit par
`tools/bench/mes-p8-conteneur/aggregate-b.py`, qui recalcule chaque percentile depuis les durées par
appel et **refuse** une série dont le nombre d'appels s'écarte de celui du harnais.

Tous les percentiles de ce tableau et de l'artefact sont pris au **rang le plus proche** (ADR-0077,
décision 4), jamais par interpolation ; l'artefact a été régénéré depuis les sorties brutes le
2026-11-03, après alignement de l'agrégateur sur cette méthode, et les deux concordent.

Millisecondes, 1 000 appels mesurés par exécution :

| Exécution | Précision | p50 | p95 | p99 | premier appel | fils vus |
|---|---|---:|---:|---:|---:|---|
| `b-encode-int8-1` | int8 | 47,457 | 132,103 | 176,773 | 3 701,1 | 8 / 8 |
| `b-encode-int8-2` | int8 | 41,510 | 116,966 | 156,241 | 1 905,6 | 8 / 8 |
| `b-encode-fp32-3` | fp32 | 100,347 | 258,465 | 316,956 | 6 596,4 | 8 / 8 |
| `b-encode-fp32-4` | fp32 | 103,593 | 260,640 | 317,729 | 3 571,9 | 8 / 8 |
| `native-encode-int8` | int8 | **18,262** | **52,710** | **65,833** | 2 570,5 | 8 / — |
| `native-encode-fp32` | fp32 | **42,616** | **117,720** | **146,124** | 3 065,4 | 8 / — |

« fils vus » : `available_parallelism` / tâches du processus ; le tiret signifie que la plateforme
n'a pas `/proc`. Les quatre exécutions en conteneur rapportent **8 et 8** : le processus n'a pas été
réduit à un fil.

Rapports. Pour le conteneur, chaque précision a **deux exécutions** : le rapport prend la **médiane
des deux**, soit la moyenne de deux valeurs, et le dit à chaque fois plutôt que de choisir une
exécution.

- **conteneur / natif du jour** : **×2,44** pour l'int8 (médiane 44,484 contre 18,262) et **×2,39**
  pour le fp32 (101,970 contre 42,616) ; en p95, **×2,36** (124,535 contre 52,710) et **×2,20**
  (259,553 contre 117,720). Prises séparément, les deux exécutions d'une précision donnent 2,27 et
  2,60 pour l'int8 et 2,36 et 2,43 pour le fp32 (p50) : la médiane est retenue, et la fourchette est
  dite ici plutôt que masquée.
- **natif du jour / campagne publiée du 2026-10-24** (8,646 int8, 20,978 fp32) : **×2,11** et **×2,03**.

Le second rapport est la part de l'état de la machine, le premier est celle du conteneur. Les deux
sont donnés séparément : aucun des deux ne peut être attribué au conteneur seul.

### Conclusion

- **Le conteneur coûte entre 2,2 et 2,6 fois le natif du même jour** (2,4 en médiane sur les p50), à
  précision égale, sur le même modèle, le même harnais, le même nombre de fils vus (8) et les mêmes
  300 requêtes. C'est le chiffre
  que la décision 5 d'ADR-0135 demande, et il ne dépend pas de la campagne publiée.
- **La machine était plus lente aujourd'hui qu'au 2026-10-24, et c'est déclaré** : le contrôle natif
  vaut ×2,0 à ×2,1 la campagne publiée. Comparer le conteneur directement aux 8,646 / 20,978 ms
  publiés donnerait ×5,1 et ×4,9, soit un peu plus du double du coût réel du conteneur ; ce rapport
  n'est donc **pas** retenu.
- **Aucune comparaison au budget n'est faite.** Le budget indicatif de `docs/03-recherche.md`
  (20 à 50 ms pour l'encodage d'une requête) se lit sur le Mac mini en conteneur (P10) ; ces chiffres
  sont **indicatifs** (ADR-0129). En conteneur, sur cette VM, le p50 int8 (41,5 à 47,5 ms) est dans le
  budget et le p50 fp32 (100,4 à 103,6 ms) le dépasse : c'est un constat sur cette VM, pas un verdict
  sur la cible.

Ce que la mesure **ne** permet pas d'affirmer :

- le nombre de fils du pool intra-opération d'ONNX Runtime : il n'est pas exposé par le runtime ;
- l'effet d'un pool réduit (`search_threads`) sur l'encodage : la configuration de l'instance laisse
  le runtime choisir ;
- la latence de l'encodage **de document**, qui n'est pas MES-05 ;
- la reproductibilité sur une machine au repos : la charge de l'hôte n'est pas relevée.

**Recommandation, à valider par le propriétaire** : pour un budget d'encodage en conteneur à 8,58 M
passages, ne retenir que l'int8 (`query_precision = "int8"`), dont le p50 en conteneur reste sous les
50 ms du budget indicatif ; le fp32, mesuré à 100 ms sur cette VM, ne le tient pas.

## Campagne en conteneur sur le Mac mini (P10, ADR-0141) — 2026-11-07

- **Tâche** : `MESURER-SUR-MAC-MINI`, passe C. Commit
  `439d946bf28d63214e64aa7cb2e09e458f81a45d` plus le harnais `tools/bench/p10-macmini/`, aucun code
  de production modifié.
- **Machine** : Mac mini M1, 8 cœurs, 16 Gio, macOS 26.5.2 ; **VM Docker mesurée 9,703 GiB**, 8 vCPU.
  Le conteneur voit **8 fils** (`available_parallelism` et fils du processus), page 4 096 o.
- **Modèle** : `/models/mdenseon`, le modèle de l'instance ; **300 textes de requête** (entrée
  d'encodage), sans écriture de texte.
- **Configuration** : `measure_queries latency`, **cache de requêtes à capacité 0**, 300 appels
  d'échauffement, **1 000 appels comptés par exécution**, **deux exécutions par précision**.

### Protocole

```sh
sh tools/bench/p10-macmini/run_pass_c.sh encode int8 1
sh tools/bench/p10-macmini/run_pass_c.sh encode int8 2
sh tools/bench/p10-macmini/run_pass_c.sh encode fp32 3
sh tools/bench/p10-macmini/run_pass_c.sh encode fp32 4
```

Chaque exécution tourne dans un conteneur `--rm` au premier plan, le volume des modèles monté en
lecture seule. `measure_queries` **n'ouvre aucun index** : le conteneur ne monte aucun dossier d'index
(`/models` et `/in` en lecture seule, `/out`)
(`tools/bench/p10-macmini/run_pass_c.sh:96`). Les 2 000 appels d'une précision sont regroupés pour
les percentiles ; les deux exécutions sont aussi données séparément. Percentiles au rang le plus
proche ; l'agrégateur recalcule et refuse tout désaccord.

### Résultats

En millisecondes, 300 requêtes, cache désactivé :

| Précision | n | p50 | p95 | p99 | min | max | Premier appel |
|---|---:|---:|---:|---:|---:|---:|---:|
| int8 | 2 000 | 32,028 | 84,147 | 106,070 | 10,574 | 129,326 | 1 375,7 à 1 849,3 ms |
| fp32 | 2 000 | 107,870 | 279,604 | 427,015 | 30,919 | 855,431 | 4 348,5 à 4 364,1 ms |

Les deux exécutions sont cohérentes : p50 de 31,431 et 32,532 ms pour l'int8, 114,020 et 102,501 ms
pour le fp32. Le premier appel paie l'ouverture de la session ONNX et n'est pas une requête.

### Conclusion

- **L'int8 tient son p50 dans le budget indicatif de 20 à 50 ms, estimé pour lui** (32,0 ms), mais
  **pas son p95** (84,1 ms). Le fp32 dépasse le budget dès le p50 (107,9 ms).
- **L'instance servie emploie le fp32** (`query_precision = "fp32"`) : c'est cette précision qui pèse
  sur le total de bout en bout. La passe C mesure ce poids : 231,9 ms p95 d'encodage à froid sur les
  82 requêtes uniques, dans `mac-mini-2026-11-07.md`.
- **La campagne ne fournit aucun contrôle natif contemporain.** Elle ne permet donc pas d'attribuer
  l'écart au conteneur seul, contrairement à la campagne du 2026-11-03 qui avait ce contrôle.

Ce que la mesure **ne** permet pas d'affirmer :

- le coût du conteneur par rapport au natif du même jour ;
- le nombre de fils internes d'ONNX Runtime : il n'est pas exposé ;
- la latence d'encodage d'un document, qui n'est pas MES-05 ;
- l'effet d'un changement de `query_precision` sur la fidélité : aucune bascule n'est faite ici.

**Recommandation, à valider par le propriétaire** : garder `query_precision = "fp32"` tant que la
latence avec encodage reste sous 698 ms. Le critère de J10 est le `total` chaud d'ADR-0141,
décision 4 : **79,355 ms**, plus grand p95 par index ; l'encodage y est presque nul parce que le
cache de requêtes sert les textes répétés. L'**estimation prudente** vaut **358,959 ms**
(79,355 + 279,604, le p95 fp32 sans cache mesuré sur **300 textes**, donc pas un majorant strict
pour les 97), et le **recoupement API** vaut **418,425 ms p95** à cache vide. Les trois lectures
sont sous 698 ms. Une bascule vers l'int8 ne tiendrait pas non plus le p95 du budget indicatif et
demanderait un ADR pour la fidélité.
