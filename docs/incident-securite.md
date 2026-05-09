# Security Incident / Suspected Compromise

Operational runbook to use together with `fiestaaa_back/docs/deploiement.md`
when a VPS, secret, or third-party account may have been compromised.

This document favors clean rebuilds. In the Fiestaaa context, if the data is not
critical, it is generally safer and faster to start from scratch than to try to
"clean" a questionable server.

## When to Trigger This Runbook

Trigger this runbook if at least one of the following signals appears:

- the VPS SSH host key changes without explicit action on your side;
- `fiestaaa.app` or `api.fiestaaa.app` responds with unexpected behavior, for
  example `404 page not found` instead of known API routes;
- secrets were exposed in a repository, Git history, chat, screenshot, or
  untrusted user workstation;
- you no longer recognize users, services, containers, or files on the server;
- a third party may have accessed the VPS or the GitHub, Google/Firebase, Resend,
  or Apple Developer accounts.

## Ground Rules

- Treat the current VPS as untrusted until it has been reset or verified beyond
  doubt.
- Do not reuse old secret values (`.env`, `.env.prod`, Firebase JSON, SSH keys,
  GitHub tokens, DB passwords, etc.).
- Never accept a new SSH host key blindly. Verify the machine through OVH, the
  KVM console, or rescue mode.
- If production data is critical, take a snapshot or backup before deleting
  anything. If the data is test data, prefer a complete rebuild.
- Do not copy the old `.env` to the new server. Start from a fresh file with
  regenerated values.

## Recommended Fiestaaa Strategy

### Case 1: Test Environment / Disposable Data

Recommended strategy:

1. Freeze the old environment.
2. Rotate every secret.
3. Fully reinstall the VPS.
4. Recreate third-party integrations (Google/Firebase, Resend, Apple if needed).
5. Redeploy cleanly from Git and `deploiement.md`.

### Case 2: Important Production Data

Recommended strategy:

1. Isolate the server.
2. Back up volumes and logs.
3. Rotate secrets.
4. Reinstall or migrate to a new VPS.
5. Restore only the necessary data after verification.

The rest of this document mostly details **Case 1**, which is the most suitable
option if the current environment contains only test data.

## Recommended Action Plan from Now

### 1. Freeze the Old Environment

- Stop deploying to the current VPS.
- Stop entering passwords on the old host until its identity is confirmed.
- Remove old local SSH host keys:

```bash
ssh-keygen -R 51.75.20.71
ssh-keygen -R fiestaaa.app
ssh-keygen -R "[2001:41d0:305:2100::d42f]:22"
```

- If needed, keep only a minimal backup of useful config files
  (`docker-compose.yml`, directory structure, etc.), never secrets for reuse.

### 2. Generate a New Administration SSH Key

Create a new key pair dedicated to the new VPS:

```bash
ssh-keygen -t ed25519 -a 64 -f ~/.ssh/fiestaaa_vps_2026 -C "theo@fiestaaa-vps-2026"
cat ~/.ssh/fiestaaa_vps_2026.pub
```

Add a dedicated entry in `~/.ssh/config`:

```sshconfig
Host fiestaaa-vps
  HostName fiestaaa.app
  User ubuntu
  IdentityFile ~/.ssh/fiestaaa_vps_2026
  IdentitiesOnly yes
  AddKeysToAgent yes
  UseKeychain yes
```

Do not delete the old local key until the new machine is operational.

### 3. Rotate Secrets and Third-Party Accounts

#### 3.1 GitHub

Objective:

- replace the GHCR token;
- replace the private key used by GitHub Actions to connect to the VPS;
- update all Actions secrets for `fiestaaa_back` and `fiestaaa_front`.

Actions:

1. Create a new `GHCR_TOKEN` dedicated to the VPS with the minimal
   `read:packages` scope.
2. Delete or revoke the old token.
3. Generate a new SSH private key for GitHub Actions deployment if you want it
   separate from your admin key.
4. Update GitHub Actions secrets, ideally in the GitHub `production` environment
   rather than at the global repository level.

Backend secrets to verify / regenerate if needed:

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
- `FCM_SERVER_KEY` (optional, only if you keep the legacy FCM fallback)
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

Frontend secrets to verify / regenerate if needed:

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
- `FIREBASE_AUTH_DOMAIN` (optional; defaults to `${FIREBASE_PROJECT_ID}.firebaseapp.com`)

Other sensitive values/files to update outside workflows:

- `service-account.json`
- `google-services.json`
- `GoogleService-Info.plist`
- `ANDROID_GOOGLE_SERVICES_JSON`
- `ANDROID_KEYSTORE_BASE64`
- `ANDROID_KEYSTORE_PASSWORD`
- `ANDROID_KEY_ALIAS`
- `ANDROID_KEY_PASSWORD`

Note:

- Actions secrets are the source of truth for production; local `.env.prod`
  files must not be copied as-is to the new server.

#### 3.2 Google Cloud / Firebase

Objective:

- start from a new clean project;
- recreate OAuth credentials, Firebase config, and service key.

Recommended strategy:

1. Treat the old project as lost.
2. If the Firebase project has already been deleted, verify in Google Cloud that
   it is being deleted or has already been deleted.
3. Create a **new Google Cloud project**.
4. Add Firebase to this project.
5. Recreate:
   - the Web OAuth client;
   - the Android OAuth client;
   - the iOS OAuth client;
   - the `google-services.json` file;
   - the `GoogleService-Info.plist` file;
   - the new Firebase/Admin service key;
   - the Web configuration (`FIREBASE_*`, `FIESTAAA_GOOGLE_WEB_CLIENT_ID`).
6. Configure web domains / origins:
   - `https://fiestaaa.app`
   - if useful: `https://www.fiestaaa.app`

Watch points:

- never reuse the old `service-account.json`;
- `FIREBASE_*` and `FIESTAAA_GOOGLE_*` values must be consistent between backend
  and frontend;
- if you still use the legacy FCM fallback, regenerate `FCM_SERVER_KEY`;
  otherwise, if you moved to FCM HTTP v1 with `service-account.json`, leave
  `FCM_SERVER_KEY` empty;
- also update mobile values linked to the new project if you use them:
  `FIREBASE_ANDROID_API_KEY`, `FIREBASE_ANDROID_APP_ID`,
  `ANDROID_GOOGLE_SERVICES_JSON`, `google-services.json`,
  `GoogleService-Info.plist`.

#### 3.3 Resend

Objective:

- invalidate any old sending key;
- recreate a clean sending domain.

Recommended strategy:

1. Create a new Resend API key.
2. Configure a new sending domain or subdomain.
3. Verify DNS records.
4. Update `INVITATION_EMAIL_SENDER` and `RESEND_API_KEY`.
5. Delete the old API key once the new one is operational.

Tip:

- use a dedicated email subdomain (`mail.fiestaaa.app` or `notify.fiestaaa.app`)
  rather than the root domain.

#### 3.4 Apple Developer

Objective:

- restart from a clean Sign in with Apple web configuration.

Recommended strategy:

1. Revoke old Apple keys used for auth.
2. Delete or recreate the web `Services ID` if you have any doubt.
3. Reconfigure Sign in with Apple for the web.
4. Regenerate values:
   - `FIESTAAA_APPLE_SERVICE_ID`
   - `FIESTAAA_APPLE_REDIRECT_URI`
5. If you use `.p8` keys, do not reuse a potentially exposed key.

Watch point:

- do not blindly delete the mobile `App ID` if the app is already known to Apple
  / App Store Connect;
- when in doubt, recreate the web part first (Services ID, redirect URL, key)
  before touching the mobile bundle ID.

#### 3.5 Android / App Signing

Objective:

- verify whether the local Android key must be replaced.

Recommended strategy:

- if the app is not published or has no value, regenerate a fresh release
  keystore;
- if the app is published on Google Play with Play App Signing, **do not change
  the app signing key lightly**;
- if only the upload key is compromised and Play App Signing is enabled, use the
  upload key reset procedure in Play Console.

Relevant variables/files:

- `ANDROID_KEYSTORE_BASE64`
- `ANDROID_KEYSTORE_PASSWORD`
- `ANDROID_KEY_ALIAS`
- `ANDROID_KEY_PASSWORD`
- `google-services.json`

### 4. Fully Reinstall the OVH VPS

Recommended strategy:

1. Start a full VPS reinstallation from OVH.
2. Inject the **new SSH public key** during reinstallation.
3. Start from a clean image.
4. Do not restore the old `/etc`, old `~/.ssh`, or old `.env`.

Once the new VPS is reachable, follow `fiestaaa_back/docs/deploiement.md`:

- section "1) Prepare the VPS"
- section "2) Prepare the VPS Directory Tree"
- section "3) First Manual Startup"
- section "4) GitHub Actions CI/CD (backend)"
- section "5) Frontend (fiestaaa_front)"
- section "6) Runtime Checks"

### 5. Recreate a Clean Runtime Configuration

Start from scratch:

```bash
mkdir -p ~/apps/fiestaaa/{backend,frontend,data/uploads,traefik/letsencrypt}
cp fiestaaa_back/docker-compose.prod.yml ~/apps/fiestaaa/docker-compose.yml
touch ~/apps/fiestaaa/traefik/letsencrypt/acme.json
chmod 600 ~/apps/fiestaaa/traefik/letsencrypt/acme.json
```

Then:

- create a new `.env` from the placeholders in `deploiement.md`;
- place the new `service-account.json` in
  `~/apps/fiestaaa/backend/service-account.json`;
- verify `CORS_ALLOWED_ORIGINS` contains `APP_BASE_URL`.

Do not reuse:

- the old `.env`;
- the old `.env.prod`;
- old tokens;
- old potentially exposed Firebase / Apple / Android files.

### 6. Redeploy in the Right Order

Recommended order:

1. backend;
2. API smoke tests;
3. frontend;
4. frontend + OAuth smoke tests.

Minimum checks:

```bash
curl -vk https://api.fiestaaa.app/health
curl -I -X OPTIONS https://api.fiestaaa.app/auth/oauth/google \
  -H 'Origin: https://fiestaaa.app' \
  -H 'Access-Control-Request-Method: POST' \
  -H 'Access-Control-Request-Headers: content-type'
```

Then verify:

- `Access-Control-Allow-Origin: https://fiestaaa.app` is present on the
  preflight;
- `https://fiestaaa.app` opens;
- Google login;
- Apple login if used;
- email sending through Resend;
- backend and DB health.

## What to Do with `.env`, `.env.prod`, and Local Secrets

### Backend

- `fiestaaa_back/.env`: **local dev** file. Recreate it with dev-only values.
- `fiestaaa_back/.env.prod`: do not treat it as the source of truth. If you keep
  it locally, regenerate every secret value.
- `fiestaaa_back/.env.example`: keep only non-sensitive placeholders.

### Frontend

- `fiestaaa_front/.env`: local / dev config, without reusable production
  secrets.
- `fiestaaa_front/.env.prod`: regenerate every sensitive value, especially those
  linked to Firebase, the Android keystore, and integrations.

### Simple Rule

If a value was present on the old VPS, in an old `.env`, in Git history, in a
build export, or in a questionable public/private chat, consider it compromised
and regenerate a new one.

## Git Hygiene and History

The repository already ignores sensitive files (`.env`, `.env.*`,
`service-account.json`, keystores, certificates). This does not remove a secret
if it was already pushed into history.

Recommended procedure:

1. Rotate secrets first.
2. Only then clean Git history if needed.
3. Verify the following:
   - repository private;
   - 2FA active on GitHub;
   - collaborator access reviewed;
   - old PAT tokens deleted;
   - old unused SSH keys deleted.

## Short Checklist

### Immediate

- [ ] Freeze the old VPS
- [ ] Generate a new admin SSH key
- [ ] Regenerate `GHCR_TOKEN`
- [ ] Regenerate `JWT_SECRET`
- [ ] Regenerate Postgres credentials
- [ ] Regenerate `RESEND_API_KEY`
- [ ] Regenerate Firebase / Google Cloud configuration
- [ ] Regenerate Apple web configuration
- [ ] Evaluate Android keystore rotation

### Infra

- [ ] Reinstall the VPS from OVH
- [ ] Inject the new SSH public key
- [ ] Redo Docker / Compose / UFW / Fail2ban installation
- [ ] Recreate `~/apps/fiestaaa`
- [ ] Recreate a fresh `.env`
- [ ] Put a new `service-account.json`

### CI/CD

- [ ] Update backend GitHub secrets
- [ ] Update frontend GitHub secrets
- [ ] Redeploy `fiestaaa_back`
- [ ] Verify `/health` and CORS preflight
- [ ] Redeploy `fiestaaa_front`
- [ ] Verify `https://fiestaaa.app`
- [ ] Verify Google / Apple logins

## Useful Links

- OVHcloud VPS rescue: https://help.ovhcloud.com/csm/fr-ca-vps-rescue?id=kb_article_view&sysparm_article=KB0047660
- Google Cloud project lifecycle: https://cloud.google.com/resource-manager/docs/creating-managing-projects
- Firebase project / Google Cloud project relation: https://firebase.google.com/docs/projects/use-firebase-with-existing-cloud-project
- Google OAuth Web: https://developers.google.com/identity/protocols/oauth2/javascript-implicit-flow
- Firebase Admin setup: https://firebase.google.com/docs/admin/setup
- Resend API keys: https://resend.com/docs/dashboard/api-keys/introduction
- Resend domains: https://resend.com/docs/dashboard/domains/introduction
- Apple Sign in with Apple for the web: https://developer.apple.com/help/account/capabilities/configure-sign-in-with-apple-for-the-web/
- Apple revoke keys: https://developer.apple.com/fr/help/account/keys/revoke-edit-and-download-keys
- Android app signing: https://developer.android.com/guide/publishing/app-signing.html
