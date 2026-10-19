---
description: Le propriétaire valide un jalon - écrit VALIDE, fusionne la phase dans main, tag, propose le push (ex. /valider J1)
agent: orchestrateur
---

**Je suis le propriétaire, et je valide le jalon $1.** C'est mon instruction explicite.

Procède dans cet ordre :
1. Vérifie que `docs/jalons/$1-rapport.md` existe et que le jalon est `PRET-POUR-VALIDATION` dans `docs/suivi.md`. Sinon, arrête-toi et dis-moi ce qui manque.
2. Dans `docs/suivi.md`, écris `VALIDE <date du jour>` pour $1, et ajoute une ligne de journal.
3. `@git` → `commit-docs "validate milestone $1"`.
4. `@git` → `fin-phase` de la phase correspondant à $1 (fusion dans `main` et tag `jalon-$1`).
5. Demande-moi si je veux pousser (`@git pousser main`, avec les tags). Ne pousse qu'après ma confirmation.
6. Rappelle-moi d'ouvrir une nouvelle session (`/new`) pour la phase suivante.

Jalons :
!`grep -n "^| J[0-9] |" docs/suivi.md`
