# ADR-0059 — Robustesse du téléchargement de l'artefact du modèle en CI

- **Statut** : Proposé
- **Date** : 2026-10-25
- **Décideurs** : propriétaire
- **Proposé par** : agent greffier, sur demande du propriétaire, hors tâche (branche `phase/P3-cycle-de-vie`).
- **Remplace** : — (complète ADR-0057, qui reste en vigueur)

## Contexte

### Le sujet est préventif

Aucun échec du téléchargement de l'artefact du modèle n'a été observé à ce jour. Le run à froid `40118324771` et le run à cache chaud `40119158468` sont verts, et le run `40119640865` de la branche de phase l'est aussi. La décision porte donc sur un risque non réalisé, pas sur un incident.

### Ce que fait le code aujourd'hui

`tools/fetch_model_artifact.py` (ADR-0057) télécharge trois fichiers de `mattboll/mdenseon-onnx`, épinglés au commit `491bbcd6cb1b0a7a97995a5d767ab8f5270a7b84` du manifeste `ci/model-artifact.toml`, sans jeton parce que le dépôt est public. Il vérifie chaque fichier par blake3, puis le renomme en place.

**Il ne fait aucune nouvelle tentative.** `download_file` appelle `urllib.request.urlopen` une seule fois. La première erreur remonte en `ArtifactError`, `main` l'imprime et rend le code 1 : le job `rust-onnx` échoue aussitôt. Une coupure réseau passagère, un HTTP 429 de limite de débit ou un 5xx de `huggingface.co` suffit donc à faire échouer un run, et le propriétaire doit le relancer à la main.

Le seul garde-fou de durée est `TIMEOUT_SECONDS = 120`, qui borne une lecture bloquée du socket, et le `timeout-minutes: 20` du job.

### Ce que fait le cache GitHub Actions

Le job restaure `data/models/mdenseon` avec `actions/cache/restore@v4`, sous la clé `cache_key(manifest)` : dépôt, commit et blake3 de chaque fichier. Le contenu restauré est vérifié comme un téléchargement neuf (ADR-0057), et un cache incomplet échoue avec la clé à purger. Le cache n'est donc jamais une source de confiance. Son absence ne met pas la correction en danger : elle rend le job dépendant de `huggingface.co`, ce qui est précisément le sujet de cet ADR.

Les règles de la plateforme, vérifiées dans la documentation GitHub Actions le 2026-10-25 :

- un cache non accédé depuis plus de 7 jours est supprimé ;
- le total est limité à 10 Go par dépôt par défaut, et au-delà, GitHub évince les caches du plus ancien accédé au plus récent ;
- un run restaure les caches de sa propre branche et ceux de la branche par défaut ; un run de pull request restaure aussi ceux de la branche de base, y compris une base de fourche ;
- un run ne restaure jamais les caches d'une branche sœur ni d'une branche enfant ; un cache créé par un run de pull request est attaché à `refs/pull/…/merge` et n'est pas relisible ailleurs ;
- `schedule` et `workflow_dispatch` font partie des déclencheurs qui peuvent écrire dans la portée de la branche par défaut.

Le 2026-10-24, le propriétaire a supprimé les caches hors `main`. `ci.yml` enregistre pourtant le cache sur toutes les branches : chaque branche a donc reconstruit le sien à son premier run, et ce cache n'est relisible que par cette branche. Seul celui de `main` sert à toutes les branches, et seul un push sur `main` le crée ou le rafraîchit. Voir la section suivante pour ce que cela implique.

### Ce que disent les mesures

La ligne `AJOUTER-ORT-CI` de `docs/suivi.md` donne : run à froid `40118324771`, téléchargement vérifié en ≈ 8 s pour environ 1,1 Gio ; run à cache chaud `40119158468`, restauration en ≈ 9 s ; job `rust-onnx` à 4 min 20 s à froid et 3 min 16 s à cache chaud.

Sur l'axe de la **vitesse**, le cache chaud n'apporte donc rien : 9 s contre 8 s, un écart dans le bruit de mesure. L'écart de 64 s entre les deux runs vient d'ailleurs (compilation, charge du runner), pas du modèle. Un téléchargement à froid n'est pas un incident de durée.

**La vitesse n'est pas le bon axe.** L'option 3 n'a jamais été proposée pour gagner des secondes : elle a été proposée pour que le job ne dépende pas de `huggingface.co` au moment où il tourne. Une première version de cet ADR a écarté l'option 3 sur la seule comparaison 8 s contre 9 s ; c'était une erreur de raisonnement, corrigée ci-dessous.

### Le vrai enjeu : la disponibilité d'un tiers

Trois faits, tirés du code et de la documentation de la plateforme :

1. **Un cache restauré dispense de tout appel réseau.** Le script ne télécharge que les fichiers absents ; un cache complet est vérifié en place (`verify_cache`), sans contacter `huggingface.co`. Une panne de Hugging Face, une rafale de 429 ou une coupure prolongée n'ont alors **aucun effet** sur le job.
2. **Sans cache, une indisponibilité durable fait échouer le job.** L'option 1 ne couvre qu'environ 65 s d'attente cumulée, bornée par l'échéance de 600 s. Une panne de trente minutes dépasse les deux, et `rust-onnx` échoue à chaque run à froid. Les autres jobs restent verts, mais la CI est rouge : la fusion est bloquée par un tiers.
3. **Le cache de `main` est la seule réserve lisible par toutes les branches.** Un run restaure les caches de sa propre branche et ceux de la branche par défaut. Il ne voit jamais ceux d'une branche sœur ni d'une branche parente.

Le point 3 doit être nuancé, parce que `ci.yml` enregistre aujourd'hui le cache sur **toutes** les branches : l'étape `Save the model artifact` ne s'exécute que sur un défaut de cache, sans condition de branche. Un run de `phase/P3-cycle-de-vie` crée donc un cache propre à cette branche, relisible par les runs suivants de la même branche de phase ; un run de `tache/X` crée le sien, relisible par les pushes suivants de `tache/X`. Un cache créé par un run de pull request est attaché à `refs/pull/…/merge` et n'est relisible que par ce pull request ; ce dépôt ne passe pas par des pull requests, le cas ne se présente donc pas. Le point 4 revient sur cet enregistrement par branche, qui gaspille de l'espace sans servir à personne.

La situation réelle est donc la suivante, et elle est moins mauvaise que « tout part à froid » :

- le **premier** run `rust-onnx` d'une branche de tâche part à froid, puis la branche est chaude pour ses pushes suivants ;
- idem pour une branche de phase, mais son premier run à froid suit généralement le premier run de sa première tâche ;
- en revanche, **aucune branche ne profite du cache d'une autre** : une tâche neuve repart à froid même si la phase a un cache chaud, et la phase repart à froid après un `main` fraîchement fusionné si son cache a été évincé ;
- le cache de `main` n'est rafraîchi que par un push sur `main`, c'est-à-dire une fusion de phase. Entre deux fusions, il est évincé au bout de 7 jours sans accès. Pendant la fin d'une phase, les runs de tâche ne le touchent pas : seuls ceux de `main` le rafraîchiraient, et `main` ne bouge pas.

Le seul cache qui serve à tout le monde est donc celui de `main`, et c'est précisément celui qui refroidit. Un déclencheur planifié sur `main` le restaure ou le recrée : pendant une panne de Hugging Face, toutes les branches trouvent alors un cache chaud et le job passe sans réseau.

### Contraintes

- `timeout-minutes: 20` sur le job, et le job consomme 4 min 20 s à froid : il reste environ 15 min de marge, ce qui borne toute politique de reprise.
- Aucun jeton en CI ni dans le dépôt Crible (ADR-0057), et le jeton d'envoi du propriétaire (`crible-upload`) a été révoqué le 2026-10-24.
- Aucune donnée de production (ADR-0032) : tous les fichiers téléchargés sont publics.
- Aucune dépendance nouvelle sans ADR, et `docs/07` ne cite `blake3` (ADR-0034) et `urllib` de la bibliothèque standard que pour ce script.
- Les 14 tests de `tools/tests/test_fetch_model_artifact.py` n'atteignent jamais le réseau : l'ouvreur est injecté.
- Le cache reste une **copie vérifiée**, jamais une source de confiance (ADR-0057). Aucune des options retenues ne relâche cette règle.

## Décision

**Deux mesures, pour deux propriétés distinctes. `tools/fetch_model_artifact.py` refait un petit nombre de fois les téléchargements qui échouent pour une raison passagère, en attendant de plus en plus longtemps, sous une échéance globale qui garde le job loin de son `timeout-minutes` (disponibilité pendant quelques minutes). Un workflow séparé, planifié deux fois par semaine et déclenché aussi quand le manifeste change, tient chaud le cache de `main` (disponibilité pendant une panne longue) ; il en est le seul écrivain, et `ci.yml` ne fait que restaurer. Aucun jeton n'entre en CI.**

La première mesure couvre la panne courte, la seconde la panne longue. Ni l'une ni l'autre ne relâche le contrôle des hash : le cache reste une copie vérifiée, et un contenu faux échoue du premier coup, cache ou pas (ADR-0057).

### 1. Politique de tentatives, fixée dans le script

Ce sont des valeurs par défaut, donc elles relèvent de cet ADR. Elles sont des constantes du module, pas une option de ligne de commande (règle 12 d'`AGENTS.md`).

| Paramètre | Valeur | Justification |
|---|---|---|
| Nombre d'essais par fichier | **4** (1 essai initial et 3 reprises) | Couvre une rafale de trois échecs consécutifs, ce qui est déjà très improbable pour des erreurs indépendantes. Un cinquième essai ajouterait du temps sans changer la probabilité d'échec. |
| Attentes entre essais | **5 s, 15 s, 45 s** (65 s au total) | Une limite de débit ou une coupure passagère se résorbe en quelques dizaines de secondes. La progression laisse passer une fenêtre courte, puis une fenêtre large, sans jamais bloquer une minute entière sur un seul essai. |
| `Retry-After` | respecté, **borné à 60 s** | Le serveur connaît sa fenêtre de réinitialisation. Au-delà de 60 s, attendre coûterait plus que relancer le run : le script échoue tout de suite en nommant l'en-tête et sa valeur. |
| Échéance globale du téléchargement | **600 s** | Le nombre de fichiers peut croître (aujourd'hui trois). Une reprise par fichier, multipliée par trois, dépasserait le `timeout-minutes`. Une échéance commune garde la borne indépendante du manifeste. |
| Borne haute du job | environ 600 s de téléchargement + environ 200 s de tests ≈ **13 min 30 s** | Reste sous `timeout-minutes: 20`, marge comprise. |

L'attente effective entre deux essais est `Retry-After` s'il est présent et inférieur ou égal à 60 s, sinon le délai nominal. On ne cumule jamais les deux.

### 2. Ce qui est passager et ce qui ne l'est pas

Une nouvelle tentative n'est faite que si la cause est extérieure au contenu :

- `urllib.error.URLError` : résolution de nom, connexion refusée, coupure TLS, délai de socket dépassé ;
- `TimeoutError` et les autres `OSError` de la lecture ;
- `http.client.IncompleteRead` : réponse tronquée en cours de corps ;
- HTTP **429** et HTTP **5xx**.

Aucune reprise, en revanche, dans ces cas :

- **un hash différent** : le fichier est corrompu ou le dépôt a changé sous le sha épinglé, et refaire la requête donnera le même résultat. C'est le contrôle final d'ADR-0057, il doit rester immédiat ;
- **404, 401, 403 et tout autre 4xx** (429 excepté) : le commit, le dépôt ou le nom du fichier est en cause, pas le réseau ;
- **un manifeste invalide** : il est refusé avant tout téléchargement, comme aujourd'hui ;
- **un cache restauré divergent ou incomplet** : la règle d'ADR-0057 ne change pas, la clé est à purger, pas à réessayer.

Après le dernier essai, l'échec reste **franc et identique** à celui d'aujourd'hui : `error: cannot download '<fichier>' from '<URL>': <cause>`, code de sortie 1. Le message gagne seulement le nombre d'essais effectués, pour que le diagnostic distingue une panne franche d'une panne intermittente.

### 3. Les tentatives sont testées sans réseau

L'attente et l'horloge deviennent des paramètres injectables, comme l'ouvreur l'est déjà. Les tests ne dorment jamais réellement : ils passent une fonction d'attente qui enregistre les délais demandés, et vérifient :

- quatre tentatives au maximum sur un 5xx, avec les délais 5, 15 et 45 s ;
- quatre tentatives sur une `URLError`, une `TimeoutError` et un `IncompleteRead` ;
- une seule tentative sur un 404, un 403 et un hash faux ;
- le respect de `Retry-After`, et l'échec immédiat au-delà de 60 s ;
- l'échec à l'échéance globale, avec le nom du fichier en cours ;
- le succès à la deuxième tentative, fichier écrit et vérifié, aucun `.part` laissé derrière.

Les 14 tests existants restent valables : ils échouent sur une cause non passagère, ou tombent dans la limite d'essais. Ceux qui comptent les URL demandées doivent être ajustés si la politique change ce compte.

### 4. Un workflow séparé tient chaud le cache de `main`

Un fichier de workflow distinct, `.github/workflows/model-cache.yml`, porte trois déclencheurs : `schedule`, `workflow_dispatch` et `push` filtré par `paths`. Il n'est **pas** ajouté à `ci.yml` : un `schedule` déclencherait les quatre jobs existants, et chacun devrait porter une garde `if: github.event_name != 'schedule'`. Une garde mal écrite ferait taire un job sans bruit, pour un gain nul.

Le workflow contient un seul job, qui ne lance aucun test :

1. `actions/checkout@v5` ;
2. installation d'`uv` et `uv sync --group ci` dans `tools/`, comme dans `ci.yml` ;
3. calcul de la clé avec `fetch_model_artifact.py cache-key` ;
4. `actions/cache/restore@v4` sur `data/models/mdenseon` ;
5. `fetch_model_artifact.py`, qui vérifie un cache restauré en place et ne télécharge que ce qui manque ;
6. `actions/cache/save@v4`, **seulement** si `cache-hit != 'true'`.

L'étape 6 ne peut pas écraser une entrée existante : `actions/cache` interdit de modifier un cache. D'où le `if:` sur `cache-hit != 'true'`, la condition que `ci.yml` appliquait avant le point 4. Conséquence utile : le workflow **restaure ou recrée**, et il n'a pas besoin de savoir laquelle des deux, ni de supposer qu'une restauration rafraîchit la date d'accès d'un cache. Si GitHub a évincé l'entrée, la restauration rate, le téléchargement refait le travail en 8 s, et l'enregistrement repart pour sept jours.

Rythme du `schedule` : `17 3 * * 1,4`, soit le lundi et le jeudi à 03:17 UTC. Deux passages par semaine, et non un, parce que l'éviction tombe après 7 jours sans accès : un passage hebdomadaire jouerait à la limite, alors que deux passages laissent environ 3 jours 1/2 de marge, week-ends compris. L'heure est creuse et sans rapport avec les fusions.

#### Le déclencheur `push` filtré par `paths`, et pourquoi il est aussi la preuve

```yaml
on:
  schedule:
    - cron: "17 3 * * 1,4"
  push:
    paths:
      - ci/model-artifact.toml
      - .github/workflows/model-cache.yml
  workflow_dispatch:
```

Ce déclencheur a un intérêt permanent : quand le manifeste change — nouvel envoi de l'artefact, nouveau sha, nouveau hash — la clé de cache change, et le workflow recrée l'entrée sans attendre le prochain passage planifié. Sur `main`, c'est exactement le moment où le cache doit être réchauffé, puisque c'est la fusion de phase qui fait arriver le manifeste. Il déclenche aussi le workflow quand le fichier du workflow lui-même change, ce qui donne la preuve demandée plus bas.

**`workflow_dispatch` ne peut pas servir de preuve avant la fusion de P3 dans `main`.** La documentation est explicite : « This event will only trigger a workflow run if the workflow file exists on the default branch », et « To trigger the `workflow_dispatch` event, your workflow must be in the default branch ». L'option `--ref` de `gh workflow run`, ou le choix de branche dans l'interface, ne choisit que la version **exécutée** ; elle n'ouvre pas le déclencheur. Une phrase de la documentation suggère qu'après un premier run on peut dispatcher sur n'importe quelle branche — « Once a workflow has run at least once, you can dispatch it against any branch or tag » — mais ce premier run suppose précisément le fichier sur la branche par défaut. Avant la fusion, le déclencheur n'existe donc pas.

Et même une fois le fichier sur `main`, un dispatch sur une autre branche ne prouverait pas le réchauffement du cache de `main` : un run écrit dans la portée de la branche qu'il exécute, jamais dans celle de la branche par défaut. La preuve par `workflow_dispatch` sur `main` ne vaut donc que pour `main`.

La preuve sur la branche de tâche passe par le `push` : le commit qui ajoute `.github/workflows/model-cache.yml` correspond au filtre `paths` et déclenche le workflow. Le run montre la clé calculée, le téléchargement, la vérification et l'enregistrement, puis un second passage sur la même branche montre une restauration. Ce run écrit dans la portée de la branche de tâche, ce qui est sans effet sur `main` : il prouve le **fonctionnement** du workflow, pas le réchauffement du cache de `main`.

La preuve du réchauffement de `main` est donc constatée **après la fusion de la phase**, et l'orchestrateur l'inscrit dans le suivi au moment de `/valider J3` : soit le premier passage planifié, soit un `workflow_dispatch` sur `main` une fois le fichier présent, soit le `push` de la fusion lui-même s'il touche le manifeste. Ce n'est pas un critère de la tâche, parce qu'il dépend d'une fusion que le propriétaire seul autorise.

#### Un seul écrivain de cache

`ci.yml` perd son étape `Save the model artifact` et ne fait plus que **restaurer**. Sans cela, deux runs concurrents, `rust-onnx` et le workflow de cache, calculeraient la même clé sur le même push et tenteraient tous deux de l'enregistrer : le second échouerait sur une clé déjà réservée. Un seul écrivain supprime la course.

Le changement a deux effets secondaires, tous deux favorables :

- une branche ne crée plus de cache à chaque run : seul le workflow de cache en écrit un, et seulement quand il tourne sur cette branche — c'est-à-dire quand son manifeste ou son fichier change. Le total du dépôt reste donc à une entrée d'environ 1,1 Gio pour l'ordinaire, au lieu d'une par branche, ce qui éloigne le plafond de 10 Go et l'éviction de l'entrée de `main` ;
- le coût d'un run à froid sur une branche reste de 8 s, absorbé par la politique de reprises du point 1.

Coût du workflow, chiffré à partir des durées connues : démarrage du runner et `checkout` quelques secondes, `uv` et `uv sync` une dizaine de secondes, restauration ≈ 9 s (mesure `40119158468`), vérification blake3 de 1,1 Gio quelques secondes. Un passage tient donc dans la minute facturée, et deux passages par semaine coûtent environ **2 minutes par semaine**, soit 8 à 9 minutes par mois. Sans la vérification, le chiffre serait plus bas, mais elle est le prix du refus de faire confiance au cache (ADR-0057) ; elle n'est pas négociable.

Une réserve à connaître : le workflow qui enregistre depuis une branche écrit dans la portée de cette branche, donc une entrée de 1,1 Gio de plus, le temps que le propriétaire la supprime. C'est rare — seulement quand le manifeste ou le fichier du workflow change sur cette branche — mais c'est la raison pour laquelle l'entretien des caches reste un geste du propriétaire.

### 5. Ce qui change dans `ci.yml`, et rien d'autre

- `ci.yml` perd l'étape `Save the model artifact` (point 4). Il garde la restauration, le téléchargement vérifié, `timeout-minutes: 20` et tout le reste, à l'identique.
- Le manifeste, le commit épinglé, les blake3, les trois fichiers téléchargés et le chemin de destination sont inchangés.
- La clé de cache est inchangée : `cache_key(manifest)`, calculée par le même script. Seul l'écrivain change.
- Un job `rust-onnx` qui trouve le cache de `main` ne contacte pas Hugging Face du tout.
- Aucun secret, aucun jeton, aucune dépendance nouvelle : le workflow planifié n'installe que le groupe `ci` déjà décrit dans `docs/07`.
- ADR-0057 reste `Accepté` et vaut tel quel. Cet ADR ne le remplace pas : il ajoute une politique de reprise au téléchargement dont il fixait déjà le caractère franc, et il donne au cache — que son point 3 enregistre déjà — un moyen d'être chaud plus souvent qu'à l'occasion d'une fusion de phase.

### 6. Une tâche hors plan `DURCIR-ARTEFACT-MODELE-EN-CI`

La mise en œuvre demande une petite tâche hors plan, en P3, qui suit le pipeline complet (`@git debut-tache` → `@dev` → `@relecteur` → suivi → `@git commit` → `@git fin-tache`). Elle ne cite aucun invariant : `docs/08` n'en définit pas pour la CI, et les critères ci-dessous sont des tests ordinaires. Si un invariant devait être cité, `@testeur` passerait avant `@dev`.

- **Nom** : `DURCIR-ARTEFACT-MODELE-EN-CI`. Il remplace `DURCIR-TELECHARGEMENT-ARTEFACT-CI`, que la première version de cet ADR proposait : l'option 3 ajoute un livrable qui n'est pas un téléchargement. Aucune tâche n'ayant été créée, le renommage ne coûte rien.
- **Dépend de** : `AJOUTER-ORT-CI` (`FAIT`).
- **À lire** : cet ADR ; ADR-0057 ; `tools/fetch_model_artifact.py` ; `tools/tests/test_fetch_model_artifact.py` ; `.github/workflows/ci.yml`, job `rust-onnx` ; `docs/07-stack-et-deploiement.md`, « CI (GitHub Actions) ».
- **Objectif** : rendre le téléchargement de l'artefact tolérant à une erreur passagère, sans jamais tolérer un contenu faux, sans sortir du `timeout-minutes` du job, et faire en sorte qu'une panne prolongée de Hugging Face ne bloque pas la CI tant que le cache de `main` est chaud.
- **Livrables** :
  1. `tools/fetch_model_artifact.py` : les constantes du point 1, l'attente et l'horloge injectables, la classification du point 2, l'échéance globale, un message d'échec qui nomme les essais effectués. Les commentaires et docstrings sont mis à jour, en anglais.
  2. `tools/tests/test_fetch_model_artifact.py` : les tests du point 3, sans réseau et sans attente réelle.
  3. `.github/workflows/model-cache.yml` : le workflow du point 4, avec les trois déclencheurs (`schedule`, `push` filtré par `paths`, `workflow_dispatch`), un seul job sans test, et l'enregistrement seulement sur un défaut de cache.
  4. `.github/workflows/ci.yml` : suppression de l'étape `Save the model artifact`, rien d'autre. Un commentaire d'une ligne dit que le workflow de cache est le seul écrivain, pour que la suppression ne soit pas reprise plus tard par inadvertance.
  5. `docs/07-stack-et-deploiement.md`, « CI (GitHub Actions) » : une phrase sur les reprises et l'échéance globale dans le point du job `rust-onnx`, et un point court sur le workflow qui tient chaud le cache de `main` — déclencheurs, rythme, aucun test lancé, et le fait qu'il est le seul à enregistrer le cache.
  6. La ligne de suivi, la durée du run distant et le constat sur `main`, tenus par l'orchestrateur.
- **Critères d'acceptation** :
  - quatre essais au plus par fichier, attentes 5, 15 et 45 s, mesurées par un faux sommeil ;
  - `Retry-After` respecté jusqu'à 60 s, et dépassement refusé immédiatement avec la valeur dans le message ;
  - une seule tentative sur 404, 403 et hash faux ;
  - échec à l'échéance globale avec le fichier en cours ;
  - aucun `.part` laissé derrière dans tous les cas ;
  - `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` et `uv run pytest` verts dans `tools/`, sans réseau ;
  - un run distant vert sur la branche de tâche, avec le job `rust-onnx` à 420 passés, 0 échoué et 1 filtré, et la durée du job notée ;
  - une exécution verte du workflow de cache, déclenchée par le `push` du commit qui ajoute le fichier (filtre `paths`), dont le journal montre la clé calculée, le téléchargement, la vérification des blake3 et l'enregistrement ; un second `push` touchant le fichier montre une restauration, sans téléchargement. `workflow_dispatch` ne peut pas servir ici : il n'existe qu'une fois le fichier sur `main` ;
  - **après la fusion de la phase seulement** : le constat que le cache de `main` est chaud, à faire par l'orchestrateur au moment de `/valider J3` (premier passage planifié, `workflow_dispatch` sur `main`, ou le `push` de fusion s'il touche le manifeste), puis noté dans le suivi. Ce constat n'est pas un critère de la tâche : il dépend d'une fusion que le propriétaire seul autorise.
- **Clause d'arrêt** : si la politique ne peut pas être bornée sous `timeout-minutes: 20`, si un test ne peut pas être écrit sans réseau, ou si une cause passagère se révèle indiscernable d'un contenu faux, s'arrêter et remonter une `QUESTION-CONCEPTION`. De même si le `push` filtré par `paths` refusait de déclencher le workflow sur une branche de tâche.
- **Hors périmètre** : l'option 2 et tout jeton ; les autres jobs et les autres étapes de `ci.yml` ; les gardes `if:` sur `ci.yml` ; le manifeste, le commit épinglé et les hash ; le nettoyage des caches de branches, qui reste une opération du propriétaire ; une mesure MES.

## Conséquences

- **Positives** :
  - un 429, un 5xx ou une coupure passagère de `huggingface.co` ne fait plus échouer un run, alors que le téléchargement de 1,1 Gio est le seul point du job qui dépend d'un tiers ;
  - une panne **longue** de Hugging Face ne bloque plus la CI, tant que le cache de `main` est chaud : le job `rust-onnx` des branches de phase et de tâche ne contacte alors pas le tiers du tout ;
  - le pire cas de reprise est borné par une échéance explicite, compatible avec le `timeout-minutes` actuel, sans changer sa valeur ;
  - le contenu faux reste refusé du premier coup : ni les reprises ni le cache ne relâchent le contrôle des hash (ADR-0057) ;
  - la politique est testée sans réseau, comme les 14 tests existants, et le diagnostic gagne le nombre d'essais ;
  - **un seul écrivain de cache** : plus de course entre deux jobs sur la même clé, et plus d'entrée de 1,1 Gio créée par chaque branche. Le total du dépôt se rapproche d'une seule entrée, ce qui éloigne le plafond de 10 Go et l'éviction de celle de `main` ;
  - le déclencheur `push` filtré par `paths` réchauffe le cache au moment où le manifeste change, sans attendre le prochain passage planifié ;
  - aucun secret, aucune dépendance : la surface de maintenance reste faible, et le workflow de cache ne lance aucun test ;
  - le coût du maintien du cache est chiffré et faible : environ 2 minutes facturées par semaine, soit 8 à 9 minutes par mois.

- **Négatives / coûts** :
  - un second fichier de workflow à maintenir, avec son cron à ne pas laisser dériver au-delà des 7 jours d'éviction ;
  - environ 2 minutes de CI facturées par semaine, et une entrée de cache de 1,1 Gio sur le dépôt ;
  - `ci.yml` n'enregistre plus de cache : une branche dont le cache de `main` est froid paie 8 s de téléchargement à chaque run, au lieu de se dépanner elle-même après le premier. C'est le prix de l'écrivain unique, et ces 8 s sont couverts par les reprises du point 1 ;
  - un run réellement cassé (commit supprimé, dépôt déplacé sous un autre compte) attend désormais jusqu'à 65 s de plus avant d'échouer franchement, et un 5xx persistant jusqu'à l'échéance de 600 s ; le message doit rester lisible ;
  - une erreur passagère est masquée pendant quelques dizaines de secondes : le run est vert alors que le réseau a vacillé. C'est voulu, mais cela retarde le signal ;
  - la politique introduit de l'état (compteur d'essais, horloge monotone) dans un script jusqu'ici sans état, donc deux paramètres injectables de plus à maintenir ;
  - `Retry-After` au-delà de 60 s fait échouer le run alors qu'attendre aurait peut-être suffi : c'est le prix de la borne ;
  - les tentatives ne couvrent pas un serveur qui répond lentement sans dépasser le délai de socket : `TIMEOUT_SECONDS = 120` borne une lecture bloquée, pas la durée totale d'un essai. Seule l'échéance globale, vérifiée entre deux essais, borne l'ensemble ;
  - le workflow ne protège que si son cache porte la **même clé** que le manifeste courant : tout nouvel envoi de l'artefact change la clé, et le premier run qui suit repart à froid, panne ou pas. C'est inévitable et acceptable, mais il faut le savoir avant de conclure qu'un run à froid est anormal.

- **À surveiller** :
  - le nombre de reprises réellement observées dans les journaux de `rust-onnx` : une reprise occasionnelle est normale, une reprise à chaque run signale un problème de débit ou de disponibilité, et c'est le signal pour rouvrir l'option 2 ;
  - un HTTP 429 qui reviendrait malgré les quatre essais : il justifierait le jeton `HF_TOKEN` en lecture seule, c'est-à-dire un nouvel ADR qui modifierait ADR-0057 ;
  - une panne de Hugging Face qui toucherait aussi les runs à cache chaud : cela signifierait que le cache de `main` a été évincé, donc que le cron n'a pas tourné ou que le plafond de 10 Go a évincé l'entrée. Les deux se diagnostiquent dans la page des caches du dépôt ;
  - le succès du cron, qui peut être retardé ou ignoré par GitHub sur un dépôt inactif ; un `schedule` manqué se voit sur la page du workflow, et le run suivant rattrape ;
  - le total de cache du dépôt face aux 10 Go : une entrée de 1,1 Gio reste, plus celles qu'un `push` de branche peut créer quand le manifeste y change. Nettoyer les caches des branches terminées, comme le 2026-10-24, reste le geste d'entretien ;
  - la durée du job `rust-onnx` : si elle approche 15 min, l'échéance de 600 s et le `timeout-minutes: 20` sont à réexaminer ensemble ;
  - la supposition qu'une restauration rafraîchit la date d'accès n'est pas vérifiée ici. Le dispositif n'en dépend pas, puisqu'il restaure **ou** recrée, mais si elle était fausse, le cron téléchargerait 1,1 Gio à chaque passage au lieu de restaurer ;
  - le cas des pull requests venues d'une fourche : la documentation dit qu'un run de pull request restaure les caches de la branche de base, fourches comprises. Ce point n'a pas été vérifié en pratique ; s'il était faux, ces runs téléchargeraient à froid, soit 8 s de plus, sans autre conséquence.
  - le fait que le cache de `main` reste chaud **après** la fusion de P3 : c'est le point à constater au moment de `/valider J3`, et non un critère de la tâche.

## Alternatives écartées

| Alternative | Pourquoi écartée |
|---|---|
| **(1) Nouvelles tentatives avec attente croissante** — retenue | — |
| **(3) Garder chaud le cache de `main` par un déclencheur `schedule`** — retenue | — |
| **(2) Secret `HF_TOKEN` facultatif en lecture seule** | Il donne des limites de débit plus hautes, mais il contredit la phrase d'ADR-0057 « aucun jeton n'entre en CI ni dans le dépôt Crible », qui a été acceptée avec le reste. Il faudrait donc un ADR qui modifie ADR-0057, plus la création et la rotation d'un secret, et il serait absent des pull requests venues d'une fourche. Aucun 429 n'a été observé : le coût est réel, le bénéfice hypothétique. Gardé en réserve, à rouvrir si des 429 apparaissent malgré les quatre essais. |
| **Comparer l'option 3 au seul critère de la vitesse** | C'est l'erreur de la première version de cet ADR : 8 s à froid contre 9 s à chaud ne dit rien de la disponibilité, qui est le sujet. Sur ce critère, l'option 3 ne vaut rien ; sur celui de la disponibilité, elle évite tout appel réseau pendant une panne longue, pour environ 2 minutes de CI par semaine. Un raisonnement qui ne compare qu'un axe écarte une mesure qui sert l'autre. |
| **Prouver le workflow par `workflow_dispatch` sur une branche, avant la fusion** | Impossible : la documentation dit « This event will only trigger a workflow run if the workflow file exists on the default branch ». `--ref` ne choisit que la version exécutée. Et même une fois le fichier sur `main`, un dispatch sur une branche écrirait dans la portée de cette branche, sans rien prouver du cache de `main`. Le `push` filtré par `paths` remplace cette preuve, et garde un intérêt permanent. |
| **Garder deux écrivains de cache (`ci.yml` et le workflow)** | Deux runs concurrents sur le même push calculeraient la même clé et tenteraient tous deux de l'enregistrer ; le second échouerait sur une clé déjà réservée, et le dépôt accumulerait une entrée de 1,1 Gio par branche. Un seul écrivain supprime la course et l'espace perdu, pour un `ci.yml` qui perd une étape. |
| **Déclencheur `schedule` dans `ci.yml`, avec des gardes `if:` sur les quatre jobs existants** | Un `schedule` déclenche tout le workflow : chaque job existant devrait porter un `if: github.event_name != 'schedule'`, et une faute de frappe dans une garde ferait taire silencieusement un job. Un fichier de workflow séparé évite la question et laisse `ci.yml` intact, hors l'étape d'enregistrement retirée au point 4. |
| **Passage hebdomadaire du cron, au lieu de deux fois par semaine** | L'éviction tombe après 7 jours sans accès : un passage hebdomadaire joue à la limite, et un retard de GitHub ou une semaine chargée suffit à perdre le cache. Deux passages laissent environ 3 jours 1/2 de marge, pour une minute de plus par semaine. |
| **Workflow planifié qui lance aussi les tests** | Il doublerait le coût sans rien prouver de plus : un run de la branche de phase prouve déjà le job complet. Le workflow ne fait que restaurer, vérifier et enregistrer. |
| **Boucle de reprise en YAML dans `ci.yml`** | Elle serait invisible aux tests, écrite une fois par fichier, et incapable de distinguer un 404 d'un 503. Le script sait déjà le faire, et ses tests existent. |
| **Action tierce de reprise (`nick-fields/retry` ou équivalent)** | Dépendance absente de `docs/07`, donc un ADR, et une action de plus à auditer pour une boucle de vingt lignes. |
| **Bibliothèque de reprise (`tenacity` ou équivalent)** | Même reproche, en plus lourd : une dépendance Python pour trois constantes et une boucle. |
| **Reprendre aussi sur un hash faux** | Le hash faux est le signal d'un artefact modifié ou corrompu, pas d'un transport capricieux. Une reprise masquerait trente secondes d'un vrai problème, puis échouerait quand même, ou pire, tomberait sur un contenu différent. ADR-0057 exige l'échec immédiat sur écart de hash. |
| **Reprises illimitées, bornées par le seul `timeout-minutes`** | Un 5xx persistant consommerait tout le job, et l'échec arriverait après vingt minutes au lieu de treize, avec un message moins précis que celui du script. |
| **Monter `timeout-minutes` pour absorber les reprises** | Le job consomme 4 min 20 s à froid : la marge de 15 min suffit déjà. Augmenter le délai rendrait les pannes plus longues à constater, sans rien apporter. |
| **Statu quo** | Il tient tant que `huggingface.co` ne flanche pas. Le jour où un 429 tombe sur `model.onnx`, le job échoue, et le propriétaire relance à la main sans savoir si le contenu était en cause. Les quatre essais, le message enrichi et un cache de `main` tenu chaud suppriment cette ambiguïté pour quelques dizaines de lignes de code, six tests, un petit workflow et 2 minutes de CI par semaine. |

## Références

- `tools/fetch_model_artifact.py` : `download_file` (aucune reprise aujourd'hui), `verify_file`, `verify_cache`, `ensure_artifact`, `cache_key`, `TIMEOUT_SECONDS = 120`.
- `tools/tests/test_fetch_model_artifact.py` : 14 tests, ouvreur injecté, aucun réseau.
- `ci/model-artifact.toml` : dépôt `mattboll/mdenseon-onnx`, commit `491bbcd6cb1b0a7a97995a5d767ab8f5270a7b84`, blake3 de `model.onnx`, `tokenizer.json` et `model.json`.
- `.github/workflows/ci.yml`, job `rust-onnx` : `timeout-minutes: 20`, `actions/cache/restore@v4` sur `data/models/mdenseon`, `Save the model artifact` (`actions/cache/save@v4`, sur défaut de cache seulement) que cet ADR retire, clé issue de `fetch_model_artifact.py cache-key`.
- `.github/workflows/model-cache.yml` : fichier à créer par la tâche `DURCIR-ARTEFACT-MODELE-EN-CI`, `schedule`, `push` filtré par `paths` et `workflow_dispatch`, un seul job sans test, seul écrivain du cache.
- `docs/07-stack-et-deploiement.md`, « CI (GitHub Actions) » : job `rust-onnx`, cache indexé sur les hash et jamais source de confiance, échec franc sans repli silencieux.
- `docs/suivi.md`, ligne `AJOUTER-ORT-CI` : runs `40118324771` (à froid, ≈ 8 s, 4 min 20 s) et `40119158468` (cache chaud, ≈ 9 s, 3 min 16 s), 1,1 Gio par téléchargement à froid ; run `40119640865` sur la branche de phase ; suppression des caches hors `main` le 2026-10-24.
- ADR-0032 (accès à la production, aucun contenu de production en CI) ; ADR-0034 (`blake3` en Python) ; ADR-0057 (artefact Hugging Face épinglé par sha, vérifié par blake3, cache jamais source de confiance, aucun jeton en CI) ; ADR-0052 (seuil de recette) sans rapport avec cet ADR.
- Documentation GitHub Actions, « Dependency caching reference » (consultée le 2026-10-25) : éviction après 7 jours sans accès, plafond de 10 Go par dépôt avec éviction du plus ancien accédé, restauration depuis la branche courante et la branche par défaut, cache d'un run de pull request attaché à `refs/pull/…/merge`, `schedule`, `push` et `workflow_dispatch` autorisés à écrire dans la portée de la branche par défaut, interdiction de modifier un cache existant.
- Documentation GitHub Actions, « Events that trigger workflows », section `workflow_dispatch` (consultée le 2026-10-25) : « This event will only trigger a workflow run if the workflow file exists on the default branch », et « Once a workflow has run at least once, you can dispatch it against any branch or tag via the GitHub API or GitHub CLI ».
- Documentation GitHub Actions, « Manually running a workflow » (consultée le 2026-10-25) : « To trigger the `workflow_dispatch` event, your workflow must be in the default branch » ; `--ref` choisit la branche exécutée, pas la présence du déclencheur.
