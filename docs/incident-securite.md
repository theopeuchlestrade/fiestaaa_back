# Incident de securite / compromission suspectee

Runbook operationnel a utiliser en complement de `fiestaaa_back/docs/deploiement.md` lorsqu'un VPS, un secret ou un compte tiers peut avoir ete compromis.

Ce document privilegie la reconstruction propre. Dans le contexte Fiestaaa, si les donnees ne sont pas critiques, il est generalement plus sur et plus rapide de repartir de zero que d'essayer de "nettoyer" un serveur douteux.

## Quand declencher ce runbook

Declencher ce runbook si au moins un des signaux suivants apparait :

- la cle hote SSH du VPS change sans action explicite de votre part ;
- `fiestaaa.app` ou `api.fiestaaa.app` repond avec un comportement inattendu (ex. `404 page not found` a la place des routes connues de l'API) ;
- des secrets ont ete exposes dans un repo, dans un historique Git, dans un chat, dans un screenshot ou dans un poste utilisateur non fiable ;
- vous ne reconnaissez plus les utilisateurs, services, conteneurs ou fichiers presents sur le serveur ;
- un tiers a potentiellement eu acces au VPS, aux comptes GitHub, Google/Firebase, Resend ou Apple Developer.

## Regles de base

- Considerer le VPS courant comme non fiable tant qu'il n'a pas ete reinitialise ou verifie hors de tout doute.
- Ne plus reutiliser les anciennes valeurs secretes (`.env`, `.env.prod`, JSON Firebase, cles SSH, tokens GitHub, mots de passe DB, etc.).
- Ne jamais accepter une nouvelle host key SSH "a l'aveugle". Verifier la machine via OVH, la console KVM ou le mode rescue.
- Si des donnees de prod sont critiques, faire un snapshot ou une sauvegarde avant toute suppression. Si les donnees sont de test, preferer la reconstruction complete.
- Ne pas recopier l'ancien `.env` sur le nouveau serveur. Repartir d'un fichier neuf avec des valeurs regenerees.

## Strategie recommandee pour Fiestaaa

### Cas 1 : environnement de test / donnees jetables

Strategie recommandee :

1. Geler l'ancien environnement.
2. Faire tourner tous les secrets.
3. Reinstaller completement le VPS.
4. Recreer les integrations tierces (Google/Firebase, Resend, Apple si necessaire).
5. Redeployer proprement depuis Git et `deploiement.md`.

### Cas 2 : donnees de prod importantes

Strategie recommandee :

1. Isoler le serveur.
2. Sauvegarder les volumes et les journaux.
3. Faire la rotation des secrets.
4. Reinstaller ou migrer vers un nouveau VPS.
5. Restaurer uniquement les donnees necessaires apres verification.

Le reste de ce document detaille surtout le **Cas 1**, qui est l'option la plus adaptee si l'environnement actuel ne contient que de la donnee de test.

## Plan d'action recommande a partir de maintenant

### 1. Geler l'ancien environnement

- Ne plus deployer vers le VPS actuel.
- Ne plus saisir de mot de passe sur l'ancien hote tant que son identite n'est pas confirmee.
- Supprimer les anciennes host keys SSH locales :

```bash
ssh-keygen -R 51.75.20.71
ssh-keygen -R fiestaaa.app
ssh-keygen -R "[2001:41d0:305:2100::d42f]:22"
```

- Si besoin, conserver uniquement une sauvegarde minimale des fichiers de config utiles (`docker-compose.yml`, structure des dossiers, etc.), jamais des secrets pour reutilisation.

### 2. Generer une nouvelle cle SSH d'administration

Creer une nouvelle paire de cles dediee au nouveau VPS :

```bash
ssh-keygen -t ed25519 -a 64 -f ~/.ssh/fiestaaa_vps_2026 -C "theo@fiestaaa-vps-2026"
cat ~/.ssh/fiestaaa_vps_2026.pub
```

Ajouter une entree dediee dans `~/.ssh/config` :

```sshconfig
Host fiestaaa-vps
  HostName fiestaaa.app
  User ubuntu
  IdentityFile ~/.ssh/fiestaaa_vps_2026
  IdentitiesOnly yes
  AddKeysToAgent yes
  UseKeychain yes
```

Ne pas supprimer l'ancienne cle locale tant que la nouvelle machine n'est pas operationnelle.

### 3. Faire tourner les secrets et comptes tiers

#### 3.1 GitHub

Objectif :

- remplacer le token GHCR ;
- remplacer la cle privee utilisee par les GitHub Actions pour se connecter au VPS ;
- mettre a jour tous les secrets Actions des repos `fiestaaa_back` et `fiestaaa_front`.

Actions :

1. Creer un nouveau `GHCR_TOKEN` dedie au VPS avec le scope minimal `read:packages`.
2. Supprimer ou revoquer l'ancien token.
3. Generer une nouvelle cle privee SSH pour le deploiement GitHub Actions si vous souhaitez la separer de votre cle admin.
4. Mettre a jour les secrets GitHub Actions.

Secrets backend a verifier / regenerer si necessaire :

- `VPS_HOST`
- `VPS_PORT`
- `VPS_USER`
- `VPS_SSH_KEY`
- `GHCR_TOKEN`
- `JWT_SECRET`
- `DATABASE_URL`
- `POSTGRES_USER`
- `POSTGRES_PASSWORD`
- `POSTGRES_DB`
- `REDIS_URL`
- `APP_BASE_URL`
- `CORS_ALLOWED_ORIGINS`
- `AVATAR_BASE_URL`
- `AVATAR_UPLOAD_DIR`
- `INVITATION_EMAIL_SENDER`
- `RESEND_API_KEY`
- `FCM_SERVER_KEY` (optionnel, uniquement si vous gardez le fallback FCM legacy)
- `FIESTAAA_FCM_VAPID_KEY`
- `FCM_SERVICE_ACCOUNT_PATH`
- `FCM_PROJECT_ID`
- `FIESTAAA_GOOGLE_WEB_CLIENT_ID`
- `FIESTAAA_GOOGLE_ANDROID_CLIENT_ID`
- `FIESTAAA_GOOGLE_IOS_CLIENT_ID`
- `FIESTAAA_APPLE_APP_ID`
- `FIESTAAA_APPLE_SERVICE_ID`
- `FIESTAAA_APPLE_REDIRECT_URI`
- `NOTIFICATION_DEDUP_TTL_SECONDS`

Secrets frontend a verifier / regenerer si necessaire :

- `VPS_HOST`
- `VPS_PORT`
- `VPS_USER`
- `VPS_SSH_KEY`
- `GHCR_TOKEN`
- `FIESTAAA_API_BASE_URL`
- `FIESTAAA_GOOGLE_WEB_CLIENT_ID`
- `FIESTAAA_APPLE_SERVICE_ID`
- `FIESTAAA_APPLE_REDIRECT_URI`
- `FIESTAAA_FCM_VAPID_KEY`
- `FIREBASE_PROJECT_ID`
- `FIREBASE_STORAGE_BUCKET`
- `FIREBASE_MESSAGING_SENDER_ID`
- `FIREBASE_WEB_API_KEY`
- `FIREBASE_WEB_APP_ID`
- `FIREBASE_WEB_MEASUREMENT_ID`
- `FIREBASE_AUTH_DOMAIN` (optionnel ; par defaut `${FIREBASE_PROJECT_ID}.firebaseapp.com`)

Autres valeurs/fichiers sensibles ou a mettre a jour hors workflows :

- `service-account.json`
- `google-services.json`
- `GoogleService-Info.plist`
- `ANDROID_GOOGLE_SERVICES_JSON`
- `ANDROID_KEYSTORE_BASE64`
- `ANDROID_KEYSTORE_PASSWORD`
- `ANDROID_KEY_ALIAS`
- `ANDROID_KEY_PASSWORD`

Remarque :

- les secrets Actions sont la source de verite de la prod ; les `.env.prod` locaux ne doivent pas etre recopies tels quels sur le nouveau serveur.

#### 3.2 Google Cloud / Firebase

Objectif :

- repartir d'un nouveau projet propre ;
- recreer les identifiants OAuth, la config Firebase et la cle de service.

Strategie recommandee :

1. Considerer l'ancien projet comme perdu.
2. Si le projet Firebase a deja ete supprime, verifier dans Google Cloud qu'il est bien en suppression ou deja supprime.
3. Creer un **nouveau projet Google Cloud**.
4. Ajouter Firebase a ce projet.
5. Recreer :
   - le client OAuth Web ;
   - le client OAuth Android ;
   - le client OAuth iOS ;
   - le fichier `google-services.json` ;
   - le fichier `GoogleService-Info.plist` ;
   - la nouvelle cle de service Firebase/Admin ;
   - la configuration Web (`FIREBASE_*`, `FIESTAAA_GOOGLE_WEB_CLIENT_ID`).
6. Configurer les domaines / origines web :
   - `https://fiestaaa.app`
   - si utile : `https://www.fiestaaa.app`

Points de vigilance :

- ne jamais reutiliser l'ancien `service-account.json` ;
- les valeurs `FIREBASE_*` et `FIESTAAA_GOOGLE_*` doivent etre coherentes entre backend et frontend ;
- si vous utilisez encore le fallback FCM legacy, regenerer `FCM_SERVER_KEY` ; sinon, si vous etes passes a FCM HTTP v1 avec `service-account.json`, laissez `FCM_SERVER_KEY` vide ;
- mettre a jour aussi les valeurs mobile liees au nouveau projet si vous les utilisez : `FIREBASE_ANDROID_API_KEY`, `FIREBASE_ANDROID_APP_ID`, `ANDROID_GOOGLE_SERVICES_JSON`, `google-services.json`, `GoogleService-Info.plist`.

#### 3.3 Resend

Objectif :

- invalider toute ancienne cle d'envoi ;
- recreer un domaine d'envoi propre.

Strategie recommandee :

1. Creer une nouvelle API key Resend.
2. Configurer un nouveau domaine ou sous-domaine d'envoi.
3. Verifier les enregistrements DNS.
4. Mettre a jour `INVITATION_EMAIL_SENDER` et `RESEND_API_KEY`.
5. Supprimer l'ancienne API key une fois la nouvelle operationnelle.

Conseil :

- utiliser un sous-domaine dedie a l'email (`mail.fiestaaa.app` ou `notify.fiestaaa.app`) plutot que le domaine racine.

#### 3.4 Apple Developer

Objectif :

- repartir d'une configuration Sign in with Apple web propre.

Strategie recommandee :

1. Revoquer les anciennes cles Apple utilisees pour l'auth.
2. Supprimer ou recreer le `Services ID` web si vous avez un doute.
3. Reconfigurer Sign in with Apple for the web.
4. Regenerer les valeurs :
   - `FIESTAAA_APPLE_SERVICE_ID`
   - `FIESTAAA_APPLE_REDIRECT_URI`
5. Si vous utilisez des cles `.p8`, ne reutilisez pas une cle potentiellement exposee.

Point de vigilance :

- ne supprimez pas aveuglement l'`App ID` mobile si l'app est deja connue d'Apple / App Store Connect ;
- en cas de doute, recreer d'abord la partie web (Services ID, redirect URL, key) avant de toucher au bundle ID mobile.

#### 3.5 Android / signature de l'application

Objectif :

- verifier si la cle Android locale doit etre remplacee.

Strategie recommandee :

- si l'application n'est pas publiee ou n'a aucune valeur, regenerer un keystore de release neuf ;
- si l'application est publiee sur Google Play avec Play App Signing, **ne changez pas l'app signing key a la legere** ;
- si seule l'upload key est compromise et que Play App Signing est active, utilisez la procedure de reset d'upload key dans Play Console.

Variables/fichiers concernes :

- `ANDROID_KEYSTORE_BASE64`
- `ANDROID_KEYSTORE_PASSWORD`
- `ANDROID_KEY_ALIAS`
- `ANDROID_KEY_PASSWORD`
- `google-services.json`

### 4. Reinstaller completement le VPS OVH

Strategie recommandee :

1. Lancer une reinstallation complete du VPS depuis OVH.
2. Injecter la **nouvelle cle publique SSH** lors de la reinstallation.
3. Repartir sur une image propre.
4. Ne pas restaurer l'ancien `/etc`, l'ancien `~/.ssh` ou l'ancien `.env`.

Une fois le nouveau VPS accessible, suivre ensuite `fiestaaa_back/docs/deploiement.md` :

- section "1) Preparer le VPS"
- section "2) Preparer l'arborescence sur le VPS"
- section "3) Premier demarrage manuel"
- section "4) CI/CD GitHub Actions (backend)"
- section "5) Frontend (fiestaaa_front)"
- section "6) Verifications runtime"

### 5. Recreer une configuration runtime propre

Repartir de zero :

```bash
mkdir -p ~/apps/fiestaaa/{backend,frontend,data/uploads,traefik/letsencrypt}
cp fiestaaa_back/docker-compose.prod.yml ~/apps/fiestaaa/docker-compose.yml
touch ~/apps/fiestaaa/traefik/letsencrypt/acme.json
chmod 600 ~/apps/fiestaaa/traefik/letsencrypt/acme.json
```

Puis :

- creer un nouveau `.env` a partir des placeholders de `deploiement.md` ;
- deposer le nouveau `service-account.json` dans `~/apps/fiestaaa/backend/service-account.json` ;
- verifier que `CORS_ALLOWED_ORIGINS` contient bien `APP_BASE_URL`.

Ne pas reutiliser :

- l'ancien `.env` ;
- l'ancien `.env.prod` ;
- les anciens tokens ;
- les anciens fichiers Firebase / Apple / Android potentiellement exposes.

### 6. Redeployer dans le bon ordre

Ordre recommande :

1. backend ;
2. smoke tests API ;
3. frontend ;
4. smoke tests front + OAuth.

Checks minimaux :

```bash
curl -vk https://api.fiestaaa.app/health
curl -I -X OPTIONS https://api.fiestaaa.app/auth/oauth/google \
  -H 'Origin: https://fiestaaa.app' \
  -H 'Access-Control-Request-Method: POST' \
  -H 'Access-Control-Request-Headers: content-type'
```

Verifier ensuite :

- la presence de `Access-Control-Allow-Origin: https://fiestaaa.app` sur le preflight ;
- l'ouverture du front `https://fiestaaa.app` ;
- le login Google ;
- le login Apple si utilise ;
- l'envoi d'emails via Resend ;
- la sante du backend et de la DB.

## Que faire des fichiers `.env`, `.env.prod` et des secrets locaux

### Backend

- `fiestaaa_back/.env` : fichier **dev local**. Le recreer avec des valeurs de dev uniquement.
- `fiestaaa_back/.env.prod` : ne pas le considerer comme source de verite. Si vous le gardez localement, regenez toutes les valeurs secretes.
- `fiestaaa_back/.env.example` : garder uniquement des placeholders non sensibles.

### Frontend

- `fiestaaa_front/.env` : config locale / dev, sans secret reutilisable en prod.
- `fiestaaa_front/.env.prod` : regenirer toutes les valeurs sensibles, surtout celles liees a Firebase, au keystore Android et aux integrations.

### Regle simple

Si une valeur etait presente sur l'ancien VPS, dans un ancien `.env`, dans un historique Git, dans un export de build ou dans un chat public/prive douteux, considerer cette valeur comme compromise et en regenerer une nouvelle.

## Hygiene Git et historique

Le repo ignore deja les fichiers sensibles (`.env`, `.env.*`, `service-account.json`, keystores, certificats). Cela ne supprime pas un secret s'il a deja ete pousse dans l'historique.

Procedure recommandee :

1. Faire tourner les secrets d'abord.
2. Ensuite seulement, nettoyer l'historique Git si necessaire.
3. Verifier les actions suivantes :
   - repository prive ;
   - 2FA active sur GitHub ;
   - acces des collaborateurs revus ;
   - suppression des anciens tokens PAT ;
   - suppression des anciennes cles SSH non utilisees.

## Checklist courte

### Immediate

- [ ] Geler l'ancien VPS
- [ ] Generer une nouvelle cle SSH admin
- [ ] Regenerer `GHCR_TOKEN`
- [ ] Regenerer `JWT_SECRET`
- [ ] Regenerer credentials Postgres
- [ ] Regenerer `RESEND_API_KEY`
- [ ] Regenerer la configuration Firebase / Google Cloud
- [ ] Regenerer la configuration Apple web
- [ ] Evaluer la rotation du keystore Android

### Infra

- [ ] Reinstaller le VPS depuis OVH
- [ ] Injecter la nouvelle cle publique SSH
- [ ] Refaire l'installation Docker / Compose / UFW / Fail2ban
- [ ] Recreer `~/apps/fiestaaa`
- [ ] Refaire un `.env` neuf
- [ ] Reposer un nouveau `service-account.json`

### CI/CD

- [ ] Mettre a jour les secrets GitHub backend
- [ ] Mettre a jour les secrets GitHub frontend
- [ ] Redeployer `fiestaaa_back`
- [ ] Verifier `/health` et le preflight CORS
- [ ] Redeployer `fiestaaa_front`
- [ ] Verifier `https://fiestaaa.app`
- [ ] Verifier les logins Google / Apple

## Liens utiles

- OVHcloud VPS rescue : https://help.ovhcloud.com/csm/fr-ca-vps-rescue?id=kb_article_view&sysparm_article=KB0047660
- Google Cloud project lifecycle : https://cloud.google.com/resource-manager/docs/creating-managing-projects
- Firebase project / Google Cloud project relation : https://firebase.google.com/docs/projects/use-firebase-with-existing-cloud-project
- Google OAuth Web : https://developers.google.com/identity/protocols/oauth2/javascript-implicit-flow
- Firebase Admin setup : https://firebase.google.com/docs/admin/setup
- Resend API keys : https://resend.com/docs/dashboard/api-keys/introduction
- Resend domains : https://resend.com/docs/dashboard/domains/introduction
- Apple Sign in with Apple for the web : https://developer.apple.com/help/account/capabilities/configure-sign-in-with-apple-for-the-web/
- Apple revoke keys : https://developer.apple.com/fr/help/account/keys/revoke-edit-and-download-keys
- Android app signing : https://developer.android.com/guide/publishing/app-signing.html
