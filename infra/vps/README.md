# VPS provisioning

This directory makes the VPS reproducible without storing secrets in Git.

- `cloud-init.yml`: bootstraps a fresh VPS at creation time.
- `ansible/playbook.yml`: idempotently brings an existing VPS back into compliance.
- `ansible/inventory.example.yml`: inventory to copy locally as `inventory.yml`.

The application stack remains described by `../../docker-compose.prod.yml`.
Production secrets stay in the GitHub Actions `production` environment and are
materialized on the VPS by deployment workflows.

## Option 1: Fresh VPS with cloud-init

1. Copy `cloud-init.yml`.
2. Replace placeholders, especially the SSH public key.
3. Paste the content into the VPS provider cloud-init/user-data field.
4. Wait for bootstrap to complete, then verify:

```bash
ssh deploy@51.75.20.71
docker compose version
cd ~/apps/fiestaaa
ls -la
```

`cloud-init` prepares the production SSH port `1969`. Always keep the initial
session open until a new connection on `1969` has been validated.

## Option 2: Existing VPS with Ansible

Install Ansible locally:

```bash
brew install ansible
```

Copy and adapt the inventory:

```bash
cp infra/vps/ansible/inventory.example.yml infra/vps/ansible/inventory.yml
```

Apply:

```bash
ansible-playbook -i infra/vps/ansible/inventory.yml infra/vps/ansible/playbook.yml
```

Run a partial dry run when the server is already configured:

```bash
ansible-playbook -i infra/vps/ansible/inventory.yml infra/vps/ansible/playbook.yml --check
```

`inventory.yml` is ignored by Git and may contain the IP, initial SSH user, or
local paths. Application secrets must not be stored there.

## Option 3: Provision from GitHub Actions

The manual `Provision VPS` workflow runs the same playbook from GitHub Actions,
using secrets from the `production` environment.

Required secrets:

- `VPS_HOST`
- `VPS_SSH_KEY`
- `VPS_USER` if you do not set `connection_user`
- optional `VPS_PORT`; otherwise `22` for a freshly reinstalled VPS; the production target is `1969`

For a fresh VPS, run the workflow with `connection_user=root` if root SSH key
access is active. For an already configured VPS, leave `connection_user` empty
to reuse `VPS_USER`.

The workflow derives the public key from `VPS_SSH_KEY` and adds it to the
`deploy` account.

## First Full Deployment

Normal workflows update only one service at a time and expect both image tags to
already exist in `~/apps/fiestaaa/.env`. To initialize a blank machine:

1. Run `Provision VPS`.
2. In `fiestaaa_front`, run `Build and Deploy Front` with `skip_deploy=true`. Note the frontend commit SHA used as the image tag.
3. In `fiestaaa_back`, run `Bootstrap VPS Stack` with `front_image_tag=<sha_front>`. This workflow builds/pushes the API, then starts the whole Compose stack.
4. Then use the normal back/front deployment workflows.

If you use FCM HTTP v1, also store the Firebase service key as base64 in
`FCM_SERVICE_ACCOUNT_JSON_B64` on the backend GitHub `production` environment:

```bash
base64 < service-account.json | tr -d '\n' | gh secret set FCM_SERVICE_ACCOUNT_JSON_B64 --repo theopeuchlestrade/fiestaaa_back --env production
```

## What the Playbook Manages

- base system packages;
- Docker Engine + Compose V2 installation from the official Docker repository;
- `deploy` user;
- authorized SSH keys for `deploy`;
- UFW for SSH, HTTP, and HTTPS;
- Fail2ban for SSH;
- `~/apps/fiestaaa` directory tree;
- copy of `docker-compose.prod.yml` to `~/apps/fiestaaa/docker-compose.yml`;
- permissions for `traefik/letsencrypt/acme.json`, `.env`, and `data/service-account.json`.

## What Intentionally Stays Outside Git

- `.env` values;
- Firebase `service-account.json`;
- deploy SSH private key;
- GHCR token;
- Android keystores;
- database backups.

For a completely fresh VPS, the practical order is:

1. cloud-init or Ansible;
2. DNS `fiestaaa.app` and `api.fiestaaa.app` pointing to the public IP;
3. GitHub `production` secrets filled;
4. first back/front deployment from GitHub Actions;
5. `docker compose ps` verification and public smoke checks.
