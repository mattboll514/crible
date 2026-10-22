# P3 — Cycle de vie

**But** : une base qui vit au quotidien. On veut des mises à jour de métadonnées, la compaction, l'export et l'import, les instantanés et une reprise sûre après panne.

**Jalon J3** :
- Sur l'échantillon Legifacile et `synth-small` :
  - trois lots quotidiens simulés, puis une compaction, donnent des résultats identiques avant et après ;
  - l'aller-retour export → import donne des résultats identiques ;
  - un arrêt brutal à chaque point d'injection est suivi d'une reprise propre.
- Les invariants INV-03, 04, 08, 09 et 14 sont verts.
- Les mesures MES-07, MES-08 et MES-13 sont consignées.

---

### PATCHER-METADONNEES
- **Objectif** : modifier les champs d'un document sans ré-encodage.
- **Dépend de** : CONSTRUIRE-BITMAPS
- **À lire** : `02-architecture.md` (générations, suppressions) ; `05-api.md` (`PATCH documents`).
- **Livrables** :
  - `crible-store::patch` : mise à jour de `documents.fields`, passage à G+1, mise à jour des bitmaps en copie sur écriture ;
  - validation contre le schéma ;
  - commande `crible patch`.
- **Critères d'acceptation** :
  - après un patch, les filtres sur le champ modifié reflètent la nouvelle valeur ;
  - les vecteurs et les iid sont inchangés ;
  - une valeur mal typée est refusée sans aucun effet.

### COMPACTER-INDEX
- **Objectif** : construire une nouvelle époque sans les passages morts.
- **Dépend de** : ATTACHER-LOT, PATCHER-METADONNEES
- **À lire** : `02-architecture.md` (compaction, 6 étapes) ; `docs/adr/` (ADR-0006, ADR-0017, ADR-0043 pour la rétention de l'époque tenue par une recherche).
- **Livrables** :
  - `crible-store::compact` :
    - suspend l'écrivain ;
    - lit les passages vivants triés par (`cluster_by`, `doc_id`, `ord`) ;
    - écrit l'époque E+1 : segments d'au plus 1 M passages, SQLite neuf avec l'historique `batches` conservé ;
    - vérifie les comptes et les checksums ;
    - bascule `CURRENT_EPOCH` ;
    - supprime E quand plus aucun lecteur ne la tient ;
  - le déclenchement par `dead_ratio` et la commande `crible compact` ;
  - un point d'accroche pour reconstruire le lexical, laissé vide en P3 et rempli en P5.
- **Critères d'acceptation** :
  - après la compaction, il ne reste plus aucun passage mort ;
  - les iid sont contigus et triés selon `cluster_by` ;
  - une panne en cours de compaction laisse E intacte et utilisable (**INV-14**) ;
  - les lectures pendant la compaction ne sont pas bloquées.
- **Invariants** : INV-02 (partiel), INV-14.

### VERIFIER-EQUIVALENCE-COMPACTION
- **Objectif** : prouver INV-09.
- **Dépend de** : COMPACTER-INDEX
- **Livrables** : un test d'intégration qui lance les 200 requêtes `synth-small`, et les requêtes de l'échantillon Legifacile en exact, avant et après la compaction.
- **Critères d'acceptation** : mêmes clés et mêmes scores à 1e-6 près, sur toutes les requêtes.
- **Invariants** : INV-09.

### EXPORTER-LOT
- **Objectif** : un export portable des données vivantes.
- **Dépend de** : ATTACHER-LOT
- **À lire** : `04-donnees-et-ingestion.md` (export, instantanés).
- **Livrables** :
  - `crible-store::export` : un lot `replace_all` à la génération G, en parties d'au plus 1 M passages, écrit dans `exchange/outbox/<index>-g<G>/`, sans bloquer l'écrivain (lecture sur l'instantané) ;
  - commande `crible export`.
- **Critères d'acceptation** :
  - le lot exporté passe la validation de `LIRE-LOT-PARQUET` ;
  - il se lit avec DuckDB (commande citée dans le rapport) ;
  - il ne contient aucun iid (**INV-01**).
- **Invariants** : INV-01.

### VERIFIER-ALLER-RETOUR-EXPORT
- **Objectif** : prouver INV-08.
- **Dépend de** : EXPORTER-LOT
- **Livrables** : un test qui exporte, crée un index vide de même config, importe, puis compare les recherches.
- **Critères d'acceptation** : résultats identiques, avec les mêmes clés et les mêmes scores à 1e-6 près.
- **Invariants** : INV-08.

### CREER-ET-RESTAURER-INSTANTANE
- **Objectif** : une sauvegarde rapide et cohérente.
- **Dépend de** : COMPACTER-INDEX
- **À lire** : `04-donnees-et-ingestion.md` (instantanés).
- **Livrables** :
  - `crible snapshot` : l'écrivain est suspendu pendant la copie ; on fait un `VACUUM INTO`, puis on copie les segments, `index.json`, le manifeste et le dossier lexical s'il existe ;
  - `crible restore`, hors ligne : installe l'instantané comme nouvelle époque et met à jour `CURRENT_EPOCH`.
- **Critères d'acceptation** :
  - une restauration redonne la génération et des résultats identiques ;
  - un instantané incomplet (fichier manquant) est refusé à la restauration.

### TESTER-REPRISE-APRES-PANNE
- **Objectif** : prouver INV-03 et INV-04 à chaque étape du protocole.
- **Dépend de** : GERER-GENERATIONS, ATTACHER-LOT, COMPACTER-INDEX
- **À lire** : `02-architecture.md` (reprise au démarrage).
- **Livrables** :
  - des points d'injection à chaque étape de l'attache (1 à 6) et de la compaction (1 à 6) ;
  - un test qui, pour chaque point, tue le processus enfant (`SIGKILL`), redémarre, vérifie l'état et rejoue le lot.
- **Critères d'acceptation** :
  - pour chaque point, l'état est la génération G ou G+1 complète ;
  - rejouer le lot converge vers G+1 ;
  - aucun fichier orphelin ne reste.
- **Invariants** : INV-03, INV-04.

### REUTILISER-COMPRESSEUR-ZSTD
- **Objectif** : compresser les textes de passages avec un compresseur zstd réutilisé, au même niveau de compression, au lieu de construire un contexte par passage.
- **Dépend de** : STOCKER-ENREGISTREMENTS-SQLITE
- **À lire** : `02-architecture.md` (table `passages`, colonne `text` en zstd) ; `docs/mesures/MES-15-cout-commit-generation.md`, § 4 (tableau de la compression des textes) et § 6 (recommandation 3).
- **Livrables** :
  - `crible-store::records` : `encode_text` n'appelle plus `zstd::stream::encode_all` ; la compression passe par un compresseur zstd réutilisé (`zstd::bulk::Compressor`), au niveau `TEXT_COMPRESSION_LEVEL` inchangé ;
  - le compresseur est créé une fois pour l'écriture d'un lot de passages, et non une fois par passage ;
  - aucun changement de schéma, de `format_version` ni de nature de la trame stockée.
- **Critères d'acceptation** :
  - test d'aller-retour : le texte compressé puis décompressé est égal à l'original, y compris pour un texte vide et pour un texte long ;
  - test de relecture : une trame produite par l'ancienne méthode (`zstd::stream::encode_all`) se décompresse sans changement ;
  - les trames ne sont pas comparées octet pour octet : l'en-tête zstd peut porter la taille du contenu, et le critère porte sur la décompression ;
  - le gain sur une attache de 2 000 passages est chiffré par MES-08, dans `MESURER-CYCLE-DE-VIE`.
- **Invariants / mesures** : MES-08, MES-15.
- **Hors périmètre** : changer `TEXT_COMPRESSION_LEVEL` ; changer le schéma, `format_version` ou la nature de la trame ; compresser autre chose que `passages.text` ; ajouter une dépendance ; réécrire les textes déjà stockés.

### MESURER-CYCLE-DE-VIE
- **Objectif** : MES-07, MES-08, MES-13 et MES-15.
- **Dépend de** : COMPACTER-INDEX, CREER-ET-RESTAURER-INSTANTANE
- **À lire** : `docs/mesures/README.md`.
- **Livrables** : les rapports de mesure sur l'échantillon Legifacile et sur `synth-large`, avec une extrapolation à l'échelle de Legifacile :
  - compaction ;
  - attache d'un lot quotidien type de 1 000 à 3 000 passages, avec le gain du compresseur zstd réutilisé chiffré (ADR-0048) ;
  - temps de démarrage, en reconstruisant les bitmaps puis en chargeant le cache, avec la part du décodage JSON dans ce temps et le nombre moyen de passages par document de l'échantillon Legifacile (ADR-0047) ;
  - l'ordre des remèdes à appliquer si le démarrage dépasse 60 s : d'abord `DECODER-DOCUMENTS-UNE-FOIS`, puis `AJOUTER-CACHE-BITMAPS` seulement si le seuil est encore dépassé (ADR-0047) ;
  - rejeu de MES-15 avec le même banc (`crates/crible-store/benches/commit_cost.rs`) et les mêmes points `N` et `S` que la campagne du 2026-10-21, comparé à cette campagne, pour chiffrer la dérivation incrémentale de la bitmap des vivants (ADR-0048).
- **Critères d'acceptation** :
  - les protocoles sont reproductibles ;
  - le rapport donne la part du décodage JSON dans le démarrage et le nombre moyen de passages par document de l'échantillon Legifacile ;
  - si le démarrage dépasse 60 s à l'échelle de Legifacile, la mesure propose **d'abord** une tâche `DECODER-DOCUMENTS-UNE-FOIS` : décoder le JSON de chaque document une seule fois pendant la reconstruction des structures de filtrage, sans changement de format ni invalidation, avec un test d'égalité des structures et un rejeu de MES-13 ; la tâche est à valider par le propriétaire ;
  - la tâche `AJOUTER-CACHE-BITMAPS`, qui ajoute un format sur disque versionné et une invalidation, n'est proposée que si le démarrage dépasse encore 60 s après `DECODER-DOCUMENTS-UNE-FOIS`, et reste à valider par le propriétaire (ADR-0047) ;
  - le rapport donne, aux points `N` et `S` de la campagne du 2026-10-21, le commit p50 du rejeu de MES-15 et son écart avec cette campagne, et il chiffre ainsi la dérivation incrémentale de la bitmap des vivants (ADR-0048) ;
  - le rapport chiffre le gain de `REUTILISER-COMPRESSEUR-ZSTD` sur l'attache d'un lot quotidien type, par comparaison avec la compression mesurée par MES-15 (ADR-0048).
- **Invariants / mesures** : MES-07, MES-08, MES-13, MES-15.
