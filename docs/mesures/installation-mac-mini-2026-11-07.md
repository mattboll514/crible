# Installation de Crible sur le Mac mini — transfert et restauration

Passe A de la tâche `MESURER-SUR-MAC-MINI` (P10), conforme à ADR-0141. Ce document n'est pas une
MES-xx : il ne mesure aucune latence. Il consigne l'installation, le transfert et la restauration
dont dépendent les passes B et C, et les preuves qui les rendent vérifiables.

## Campagne du 2026-11-07

- **Tâche** : `MESURER-SUR-MAC-MINI`, passe A seule (installation, transfert, restauration).
- **Commit** : `439d946bf28d63214e64aa7cb2e09e458f81a45d`, plus le harnais
  `tools/bench/p10-macmini/` de cette séance, pas encore commité au moment de la mesure.
- **Machine de développement** : Mac Apple Silicon, `linux/arm64` dans Docker Desktop.
- **Machine cible** : Mac mini M1 (`Macmini9,1`), 8 cœurs, 16 384 Mo (17 179 869 184 o), macOS 26.5.2,
  Docker Desktop 4.83.0, moteur 29.6.2. **VM mesurée : 8 CPU, 10 418 937 856 o, soit 9,70 GiB**. Disque
  de la VM sur le SSD externe `/Volumes/MattCorp-SSD` (APFS, 931 Go, 677 Go libres au départ).
- **État de Supabase** : les six conteneurs `supabase_*_legifacile` sont **arrêtés** avant la séance
  (`Exited (137)` ou `Exited (143)` depuis environ 18 h) et le restent. Aucun Crible n'est présent.
- **Jeu de données** : six instantanés hors ligne de `data/p10-exchange/snapshots/`, **23,73 GiB**,
  **282 fichiers**, **25 480 066 488 octets** ; blake3 des six conforme à `snapshot.json` avant
  départ (13,9 s de calcul local).
- **Fenêtre** : hors de `$PROD_DAILY_WINDOW`. Séance de 16:56 à 19:00 UTC.

## Protocole

Les commandes exactes sont celles du harnais `tools/bench/p10-macmini/` ; les valeurs de `.env.prod`
ne sont jamais affichées, la connexion passe par `$PROD_SSH`.

```sh
sh tools/bench/p10-macmini/host-before.sh
sh tools/bench/p10-macmini/copy-image.sh
sh tools/bench/p10-macmini/copy-models.sh
sh tools/bench/p10-macmini/copy-snapshots.sh
sh tools/bench/p10-macmini/restore-all.sh
sh tools/bench/p10-macmini/inspect-all.sh
sh tools/bench/p10-macmini/start-server.sh
```

La copie des instantanés se fait **un index à la fois, le plus gros d'abord**, par
`rsync -a --partial --progress`, et se découpe par sous-dossier de `segments/` : au débit mesuré,
l'index `decisions` seul dépasserait la limite de 25 minutes par commande. Aucune copie
n'a été interrompue ; aucun fichier n'a eu à être recopié.

## Résultats

### Débit et durées de transfert

| Objet | Octets | Durée | Débit |
|---|---:|---:|---:|
| Image `crible:p10` (`docker save \| ssh … docker load`) | 181 812 224 | 34,6 s | 5,3 Mo/s |
| Volume `crible-p10-models` (`tar` sur `ssh`) | 1,5 GiB | 421,3 s | 3,8 Mo/s |
| `dole-e1-g1` | 27 826 610 | 2,5 s | — |
| `sarde-e1-g1` | 498 833 036 | 105,4 s | 4,7 Mo/s |
| `titres-e1-g1` | 3 167 968 879 | 803,1 s | 3,9 Mo/s |
| `jorf-e3-g3` | 4 744 561 825 | 1 180,6 s | 4,0 Mo/s |
| `articles-e1-g1` | 7 156 322 218 | 1 833,4 s | 3,9 Mo/s |
| `decisions-e2-g2` | 9 884 596 466 | 1 946,3 s | 5,1 Mo/s |
| **Six instantanés** | **25 480 066 488** | **5 871,3 s (97,9 min)** | **4,34 Mo/s** |

Le débit est **limité par le lien, pas par le chiffrement** : trois chiffrements essayés sur
200 Mo donnent 38,2 s (défaut), 40,2 s (chacha20) et 44,7 s (aes128-gcm). Une commande a dépassé
trois minutes sans progression — la copie du volume des modèles par `tar`, silencieuse ; c'est un
manquement au protocole de séance, sans effet sur les données.

### Contrôles d'empreinte

| Contrôle | Résultat |
|---|---|
| Image : identifiant local contre identifiant distant | `sha256:ff08d62b74d2…` **égal** des deux côtés |
| Modèles : SHA-256 de chaque fichier des deux côtés | **5 fichiers, listes identiques** |
| Instantanés : blake3 des sources contre `snapshot.json` | **282 fichiers conformes**, 6/6 index |
| Instantanés : taille de chaque fichier à l'arrivée | **282 fichiers, 25 480 066 488 octets, 0 écart** |
| Instantanés : blake3 à l'arrivée | non calculable sur le Mac mini (ni `b3sum`, ni module `blake3`) : le contrôle de contenu est celui de `crible restore`, qui refuse un fichier absent, de taille ou de blake3 différent (ADR-0141, décision 2) |

### Rapports de restauration

Volume `crible-p10-data` créé vide ; chaque restauration tourne dans un conteneur `docker run --rm`
au premier plan, le dossier d'échange monté en lecture seule.

| Index | Génération | Époque | Documents | Passages | Durée |
|---|---:|---:|---:|---:|---:|
| `dole` | 1 | 1 | 3 600 | 7 653 | 0,96 s |
| `sarde` | 1 | 1 | 210 965 | 214 578 | 4,03 s |
| `titres` | 1 | 1 | 1 477 883 | 1 477 883 | 27,79 s |
| `jorf` | 3 | 1 | 1 636 145 | 1 833 500 | 39,50 s |
| `articles` | 1 | 1 | 2 135 023 | 2 476 715 | 60,47 s |
| `decisions` | 2 | 1 | 1 196 623 | 3 589 776 | 81,52 s |
| **Total** | | | **6 660 239** | **9 600 105** | 214,3 s |

Les six rapports disent « aucune » époque remplacée, parce que le volume de destination était vide.
L'époque vaut donc **1** et non E+1 : E+1 suppose une époque antérieure dans le même volume
(ADR-0065, décision 2). La génération, elle, est conservée — 2 pour `decisions`, 3 pour `jorf` — et
c'est elle qui compte ; l'époque ne se compare pas.

### Comptes après restauration

`crible inspect --data /data`, serveur arrêté, six index :

| Index | Documents | Passages vivants | Morts | Segments |
|---|---:|---:|---:|---:|
| `articles` | 2 135 023 | 2 476 715 | 0 | 1 (`seg-articles-full-v1`) |
| `decisions` | 1 196 623 | 3 589 776 | 0 | 4 (`c2-0000…` à `c2-0003…`) |
| `dole` | 3 600 | 7 653 | 0 | 1 (`seg-dole-full-v1`) |
| `jorf` | 1 636 145 | 1 833 500 | 0 | 2 (`c3-0000…`, `c3-0001…`) |
| `sarde` | 210 965 | 214 578 | 0 | 1 |
| `titres` | 1 477 883 | 1 477 883 | 0 | 1 |

Total **6 660 239 documents** et **9 600 105 passages**, **égaux à la machine de développement**
(`docs/mesures/import-legifacile-2026-11-06.md:35-36`), zéro passage mort.

### Serveur

- Conteneur unique `crible-p10`, `crible:p10`, volumes `crible-p10-data` et `crible-p10-models`,
  configuration de la machine de développement copiée dans le dossier d'échange du SSD.
- Port publié sur **`127.0.0.1:55400`** seulement ; **aucun plafond `--memory`**.
- `/ready` répond `{"status":"ready"}` **60 s** après le lancement ; `GET /v1/indexes` liste les six
  index avec les comptes ci-dessus.
- Mémoire au repos : **3,674 GiB** pour une VM de **9,703 GiB** (37,87 %), CPU 0,00 %.
- `crible inspect` exige le serveur arrêté (verrou d'écriture). Le conteneur a donc été arrêté, les six
  index inspectés, puis relancé : prêt en **70 s**, `{"status":"ready"}`, **3,718 GiB** au repos. Ce
  cycle arrêt-relance du seul conteneur de Crible est le geste permis par ADR-0141, décision 1.

### Hôte avant et après

| Relevé | Avant (16:56 UTC) | Après (18:57 UTC) |
|---|---:|---:|
| Espace libre, disque interne | 40 Gi | 39 Gi |
| Espace libre, SSD externe | 677 Gi | 627 Gi |
| Swap utilisé / total | 642,81 Mo / 2 048 Mo | **2 461,38 Mo / 3 072 Mo** |
| Mémoire libre (pression) | 81 % | **57 %** |

Le SSD a perdu **50 Gi** : 23,73 GiB d'instantanés, l'image, les modèles et `crible-p10-data`
(25,5 Go restaurés). Le disque interne ne bouge pas : la VM et les données sont sur le SSD.

## Conclusion

**Ce que la passe A établit.** L'installation est faite et prouvée : image identique des deux côtés,
modèles identiques par SHA-256, 282 fichiers d'instantanés de tailles exactes et de blake3 conforme à
la source, six restaurations réussies sans époque remplacée, comptes de `crible inspect` **égaux** à
ceux de la machine de développement, serveur prêt en 60 s sur `127.0.0.1` sans plafond mémoire.

**Ce qu'elle n'établit pas.** Aucune latence : MES-02, MES-03, MES-05 et MES-09 et la passe
contraignante d'ADR-0141, décision 4, restent à faire. Les top-10 du jeu de référence ne sont pas
encore comparés à ceux de la passe B de `COMPARER-AVEC-PGVECTOR`, qui est la preuve de non-régression
du classement et **arrête la campagne en cas de différence** (ADR-0141, décision 2).

**Points à signaler, pour le propriétaire.**

1. **La VM mesure 9,70 GiB, non 11.** ADR-0141 et la fiche annoncent 11 Gio ; `docker info` donne
   10 418 937 856 o. Le protocole de la passe B doit déclarer la valeur mesurée, pas celle de l'ADR.
2. **La pression mémoire de l'hôte monte** : swap de 642 à 2 461 Mo et mémoire libre de 81 % à 57 %,
   alors que Legifacile est arrêté. C'est le signal que la décision 3 d'ADR-0141 demande de
   surveiller avant et après chaque série. Aucune action n'est proposée : toucher à Docker Desktop
   est interdit aux agents.
3. **Époque à 1 et non E+1.** Conséquence d'un volume de destination vide, conforme au message du
   CLI ; la génération est conservée. La comparaison des époques reste sans objet.
4. **Le blake3 d'arrivée n'est pas publié** : l'hôte n'a pas d'outil blake3. Le contrôle de contenu
   est celui de `crible restore` ; si le propriétaire veut un contrôle indépendant, il faut installer
   `b3sum` sur l'hôte, ce qu'aucun agent n'a le droit de faire dans cette tâche.
5. **L'installation reste en place** : image, volumes et instantanés du SSD sont conservés
   (ADR-0141, décision 6). Le serveur tourne sur `127.0.0.1:55400` et ne sert aucun utilisateur ;
   son arrêt est un geste de fin de campagne.

**Recommandations soumises au propriétaire** : aucune décision n'est prise ici. Trois propositions
attendent son accord — déclarer la VM à 9,70 GiB dans le protocole de la passe B, garder `b3sum`
hors de l'hôte (donc le contrôle par `restore`), et fixer le sort du conteneur `crible-p10` entre la
passe A et la passe B.
