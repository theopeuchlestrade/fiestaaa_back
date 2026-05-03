# Passage des dépôts privés à publics

Runbook pour le moment où `fiestaaa_back` et `fiestaaa_front` passeront de `privé + GitHub Free` à `public + GitHub Free`, au moment de la mise en prod et de l'ouverture du code.

## Objectif

Aujourd'hui, les deux repos restent privés. Cela permet de préparer la prod et la rotation des secrets, mais GitHub Free limite plusieurs garde-fous tant que les repos ne sont pas publics.

Quand les repos deviendront publics, l'objectif est de faire le basculement sans exposer de secrets et d'activer immédiatement les protections GitHub qui ne sont pas disponibles aujourd'hui.

## Ce qui est déjà prêt dans le code

Les workflows et la doc ont déjà été préparés pour l'état cible :

- `fiestaaa_back/.github/workflows/deploy.yml` référence l'environnement GitHub `production`.
- `fiestaaa_front/.github/workflows/deploy.yml` référence aussi `production`.
- Les deux repos ont un workflow `Dependency Review`, actuellement configuré pour skipper proprement tant que les repos sont `privés + GitHub Free`.
- Les workflows de déploiement sont prêts à publier des attestations de provenance.

Tant que les repos sont `privés + Free`, une partie de ces protections n'est pas réellement disponible côté GitHub. Elles deviendront utiles quand les repos seront publics.

## Limites actuelles en privé + Free

- Pas d'environnement GitHub exploitable pour séparer proprement les secrets de prod.
- Pas de règles de déploiement type `required reviewers` ou `wait timer`.
- Pas de `dependency review` exploitable comme garde-fou GitHub sur PR.
- Pas d'attestations de provenance GitHub pour des repos privés.
- Pas de protection de branche disponible sur repo privé Free.

Conclusion : aujourd'hui, la sécurité repose surtout sur :

- les `repo secrets` classiques ;
- le durcissement Docker et VPS ;
- la discipline de review et de déploiement.

## Cible une fois les repos publics

À l'ouverture du code, viser immédiatement l'état suivant :

- repos `fiestaaa_back` et `fiestaaa_front` en visibilité `public` ;
- environnement GitHub `production` configuré sur les deux repos ;
- secrets de prod déplacés des `repo secrets` vers les `environment secrets` quand c'est possible ;
- protection de branche sur `main` ;
- `Dependency Review` actif sur les PRs ;
- attestations de provenance actives sur les builds GHCR ;
- fonctionnalités GitHub de sécurité activées (`secret scanning`, `push protection`, `dependabot`, `dependency graph`) ;
- aucun secret ni fichier sensible dans l'historique Git visible publiquement.

## Checklist avant ouverture du code

À faire quelques jours avant de rendre les repos publics.

### 1. Refaire un audit des secrets

Vérifier qu'aucun secret n'est versionné ou prêt à être versionné :

- `.env`, `.env.prod`, `service-account.json`, keystores Android, clés APNs `.p8`, fichiers OAuth `client_secret_*.json`, clés SSH ;
- artefacts générés localement ;
- captures d'écran, exemples de config ou snippets dans la doc.

Vérifier aussi les fichiers d'exemple :

- `fiestaaa_back/.env.example`
- `fiestaaa_front/.env.example`

Ils doivent rester des placeholders, jamais des vraies valeurs.

### 2. Refaire un audit de l'historique Git

Le point critique avant un passage en public n'est pas seulement l'état courant du repo, mais aussi l'historique.

Si un secret a déjà été commité un jour, le simple fait de l'avoir supprimé d'un fichier ne suffit pas. Avant le passage en public :

- identifier tout secret historiquement commité ;
- le considérer comme compromis ;
- le régénérer si ce n'est pas déjà fait ;
- décider si l'historique doit être réécrit avant publication.

Après l'incident de sécurité, il faut partir du principe que tout secret collé dans un commit, un gist, un ticket, un chat ou une capture est potentiellement exposé.

### 3. Vérifier les fichiers et métadonnées open source

Avant publication, vérifier au minimum :

- licences confirmées et fichiers `LICENSE` présents :
  - `fiestaaa_back` sous `AGPL-3.0-only`
  - `fiestaaa_front` sous `MPL-2.0`
- politique de sécurité `SECURITY.md` ou document équivalent ;
- `CONTRIBUTING.md` cohérent avec la contribution externe ;
- `CODEOWNERS` et template de PR présents ;
- description de repo, topics, homepage, éventuellement templates d'issues ou PR ;
- revue des assets non open source : logos, visuels, fontes, captures, textes marketing, données d'exemple.

Point important :

- la politique `SECURITY.md` peut être préparée avant l'ouverture du code ;
- le choix de licence est maintenant acté ; s'il change un jour, il faudra le faire volontairement et documenter l'impact.

### 4. Vérifier les packages GHCR

Décider explicitement si les images GHCR restent privées ou deviennent publiques.

Option A, plus simple à court terme :

- garder les packages GHCR privés ;
- conserver `GHCR_TOKEN` sur le VPS pour `docker login`.

Option B, plus simple à long terme :

- rendre les packages GHCR publics ;
- supprimer ensuite le besoin de `GHCR_TOKEN` côté VPS si aucun pull privé n'est nécessaire.

Ne pas supposer qu'un package GHCR devient public automatiquement parce que le repo devient public.

## Séquence recommandée le jour du passage en public

### Étape 1. Geler les merges pendant l'opération

Pendant le basculement :

- éviter les merges simultanés sur `main` ;
- éviter les rotations de secrets en parallèle ;
- avoir une seule personne responsable du switch.

### Étape 2. Rendre les repos publics

Effectuer le changement de visibilité sur :

- `fiestaaa_back`
- `fiestaaa_front`

Une fois les repos publics, les options GitHub aujourd'hui absentes sur Free deviendront disponibles.

### Étape 3. Créer l'environnement `production`

Dans chaque repo :

1. `Settings` -> `Environments`
2. créer `production`
3. renseigner l'URL :
   - back : `https://api.fiestaaa.app`
   - front : `https://fiestaaa.app`

Configurer ensuite :

- `required reviewers` ;
- `prevent self-review` ;
- `wait timer` si souhaité ;
- restriction des branches et tags de déploiement à `main`.

### Étape 4. Déplacer les secrets de prod

Déplacer les secrets de prod utilisés par les workflows de déploiement depuis les `repo secrets` vers les `environment secrets` de `production`.

Conserver séparément, au besoin, certains secrets purement build ou release qui ne dépendent pas directement de l'environnement de prod, par exemple :

- signature Android ;
- `google-services.json` Android encodé ;
- autres secrets de build hors déploiement.

### Étape 5. Activer la protection de branche sur la branche par défaut

Dans chaque repo :

1. `Settings` -> `Branches`
2. ajouter une règle sur la branche par défaut

Aujourd'hui :

- `fiestaaa_front` utilise `main` ;
- `fiestaaa_back` utilise encore `master`.

Avant ou pendant l'ouverture publique, décider explicitement :

- soit renommer `fiestaaa_back` de `master` vers `main` ;
- soit protéger `master` immédiatement, puis renommer plus tard dans une opération séparée.

Réglages recommandés :

- `Require a pull request before merging`
- au moins 1 approbation
- `Dismiss stale pull request approvals when new commits are pushed`
- `Require approval of the most recent reviewable push`
- `Require conversation resolution before merging`
- `Require linear history`
- `Do not allow bypassing the above settings`
- pas de `force push`
- pas de suppression de branche protégée

Checks à rendre obligatoires quand ils existent :

- `Dependency Review`
- `Backend CI`
- `Frontend CI`

Ces workflows doivent déjà exister avant le passage en public pour que la protection de branche soit utile immédiatement.

### Étape 6. Activer les fonctionnalités GitHub de sécurité

Dans chaque repo public :

- `Dependency graph`
- `Dependabot alerts`
- `Dependabot security updates`
- `Secret scanning`
- `Push protection`

Vérifier dans l'UI GitHub que chaque option est bien activée ; certaines peuvent dépendre du type de compte ou des réglages d'organisation.

### Étape 7. Vérifier que les protections préparées deviennent actives

Après passage en public, vérifier que les éléments déjà committés deviennent réellement opérationnels :

- `environment: production` dans les workflows de déploiement ;
- `Dependency Review` sur les PRs ;
- attestations de provenance sur les builds GHCR ;
- règles de déploiement et de branche visibles dans GitHub.

## Vérifications à faire juste après l'ouverture

### Vérification GitHub

- ouvrir une PR de test et vérifier que `Dependency Review` s'exécute ;
- vérifier que les branches protégées empêchent un merge direct ;
- vérifier qu'un déploiement demande bien l'approbation et la configuration attendues via `production`.

### Vérification supply chain

- lancer un build de déploiement sur un commit sans changement fonctionnel ;
- vérifier dans GHCR que l'image publiée possède bien son attestation ;
- vérifier que le VPS peut toujours pull l'image selon le mode retenu, privé ou public.

### Vérification sécurité

- repasser un scan rapide du repo public pour confirmer qu'aucun secret n'apparaît ;
- vérifier les logs GitHub Actions pour s'assurer qu'aucune variable sensible n'est imprimée ;
- vérifier les téléchargements d'artefacts s'il y en a.

## Ce qu'il faudra probablement ajouter avant ou juste après

Le passage en public rendra les protections GitHub disponibles, mais pour atteindre un niveau plus sérieux il restera utile de compléter :

- éventuellement `CODEOWNERS` ;
- l'activation de GitHub Private Vulnerability Reporting une fois le repo public ;
- l'élargissement progressif de la CI backend au-delà du smoke test `auth`, une fois les suites d'intégration encore liées à l'ancien schéma remises à niveau ;
- éventuellement une politique séparée pour les marques, logos et autres assets non destinés à être librement réutilisés ;
- éventuellement une décision explicite sur la visibilité publique ou privée des packages GHCR.

## Décision recommandée

Le jour où l'app passe vraiment en prod et devient open source :

1. rendre les repos publics ;
2. activer immédiatement `production`, la protection de branche et les options GitHub de sécurité ;
3. vérifier que les secrets de prod ne vivent plus que dans l'environnement GitHub et sur le VPS ;
4. faire une PR de test pour valider la chaîne complète avant de reprendre un rythme normal de merge.
