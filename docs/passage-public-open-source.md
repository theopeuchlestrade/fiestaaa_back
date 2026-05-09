# Moving Repositories from Private to Public

Runbook for the moment when `fiestaaa_back` and `fiestaaa_front` move from
`private + GitHub Free` to `public + GitHub Free`, during production launch and
source-code publication.

## Objective

Today, both repositories remain private. This lets us prepare production and
secret rotation, but GitHub Free limits several safeguards while repositories
are not public.

When the repositories become public, the goal is to switch without exposing
secrets and to immediately enable GitHub protections that are not available
today.

## What Is Already Ready in the Code

Workflows and documentation have already been prepared for the target state:

- Both repositories now use `main` as the default branch.
- `fiestaaa_back/.github/workflows/deploy.yml` references the GitHub
  `production` environment.
- `fiestaaa_front/.github/workflows/deploy.yml` also references `production`.
- Both repositories have a `Dependency Review` workflow, currently configured to
  skip cleanly while the repositories are `private + GitHub Free`.
- Deployment workflows are ready to publish provenance attestations.
- Both repositories have `CODEOWNERS`, a PR template, `SECURITY.md`,
  `CONTRIBUTING.md`, a `LICENSE`, a public-oriented `README.md`, and issue
  templates.
- Local or generated files that had been tracked by mistake were removed from
  version control:
  - backend: `.idea/`, `.docker-config`;
  - frontend: `.dart-tool/`.
- `fiestaaa_front` was rewritten before publication to remove old
  Firebase/GCP/VAPID values from Git history, then force-pushed with
  `--force-with-lease`.
- `gitleaks detect --source . --redact=100` passes on the complete history of
  both repositories.
- Flutter dependencies were updated to the latest versions resolvable with
  Flutter `3.41.5` / Dart `3.11.3`.

While the repositories remain `private + Free`, some of these protections are
not actually available on GitHub. They become useful once the repositories are
public.

## Current State Before Public Opening

State as of 2026-05-03:

- `fiestaaa_back`
  - GitHub visibility: private;
  - default branch: `main`;
  - `gitleaks` history: clean;
  - open-source readiness score: about `8.8/10`.
- `fiestaaa_front`
  - GitHub visibility: private;
  - default branch: `main`;
  - `gitleaks` history after rewrite: clean;
  - open-source readiness score: about `8.5/10`.

These scores do not mean "make it public now with no further action": they
indicate that the code and Git history are close to the target state. The final
important points mostly concern GitHub settings, external key rotation or
restriction, and operations decisions.

## Current Limitations in Private + Free

- No usable GitHub environment to cleanly separate production secrets.
- No deployment rules such as `required reviewers` or `wait timer`.
- No usable `dependency review` as a GitHub PR safeguard.
- No GitHub provenance attestations for private repositories.
- No branch protection available on a private Free repository.

Conclusion: today, security mostly relies on:

- classic `repo secrets`;
- Docker and VPS hardening;
- review and deployment discipline.

## Target Once the Repositories Are Public

When the code opens, immediately target the following state:

- `fiestaaa_back` and `fiestaaa_front` repositories visible as `public`;
- GitHub `production` environment configured on both repositories;
- production secrets moved from `repo secrets` to `environment secrets` where
  possible;
- branch protection on `main`;
- `Dependency Review` active on PRs;
- provenance attestations active on GHCR builds;
- GitHub security features enabled (`secret scanning`, `push protection`,
  `dependabot`, `dependency graph`);
- no secret or sensitive file in publicly visible Git history.

## Checklist Before Opening the Code

Do this a few days before making the repositories public.

### 1. Rerun a Secret Audit

Check that no secret is versioned or ready to be versioned:

- `.env`, `.env.prod`, `service-account.json`, Android keystores, APNs `.p8`
  keys, OAuth `client_secret_*.json` files, SSH keys;
- locally generated artifacts;
- screenshots, config examples, or documentation snippets.

Also check example files:

- `fiestaaa_back/.env.example`
- `fiestaaa_front/.env.example`

They must remain placeholders, never real values.

Current state:

- full `gitleaks` audit OK on both repositories;
- local `.env` files remain ignored;
- old frontend historical secrets were removed by Git rewrite;
- old Firebase/GCP/VAPID keys seen in the previous history must still be
  considered compromised and rotated or strictly restricted in Google/Firebase.

### 2. Rerun a Git History Audit

The critical point before going public is not only the current repository state,
but also its history.

If a secret was ever committed, simply deleting it from a file is not enough.
Before going public:

- identify any historically committed secret;
- treat it as compromised;
- regenerate it if that has not already been done;
- decide whether history must be rewritten before publication.

After the security incident, assume that any secret pasted into a commit, gist,
ticket, chat, or screenshot is potentially exposed.

Current state:

- backend: no leak detected in history;
- frontend: old history rewritten, fresh clone from GitHub checked with
  `gitleaks`, no leak detected;
- a local pre-rewrite frontend backup exists at
  `/private/tmp/fiestaaa_front_pre_rewrite_e13db82_20260503_231008.bundle`. Do
  not publish or push this backup.

### 3. Check Open-Source Files and Metadata

Before publication, check at least:

- licenses confirmed and `LICENSE` files present:
  - `fiestaaa_back` under `AGPL-3.0-only`
  - `fiestaaa_front` under `MPL-2.0`
- security policy `SECURITY.md` or equivalent document;
- `CONTRIBUTING.md` consistent with external contribution;
- `CODEOWNERS` and PR template present;
- repository description, topics, homepage, and optionally issue or PR
  templates;
- review of non-open-source assets: logos, visuals, fonts, screenshots,
  marketing copy, example data.

Important point:

- `SECURITY.md` can be prepared before opening the code;
- the license choice is now decided; if it changes someday, it must be done
  deliberately and the impact must be documented.

### 4. Check GHCR Packages

Explicitly decide whether GHCR images remain private or become public.

Option A, simpler in the short term:

- keep GHCR packages private;
- keep `GHCR_TOKEN` on the VPS for `docker login`.

Option B, simpler in the long term:

- make GHCR packages public;
- then remove the need for `GHCR_TOKEN` on the VPS if no private pull is needed.

Do not assume a GHCR package automatically becomes public because the repository
becomes public.

## What Can Still Be Done from the Command Line

Before the public switch, several actions can still be done from this machine:

- rerun secret scans:
  - `cd fiestaaa_back && gitleaks detect --source . --redact=100`
  - `cd fiestaaa_front && gitleaks detect --source . --redact=100`
- verify local suites:
  - backend: `cargo fmt --all --check`, `cargo clippy --all-targets --all-features -- -D warnings`, tests with Postgres;
  - frontend: `flutter gen-l10n`, `dart format --output=none --set-exit-if-changed lib test tool`, `flutter analyze`, `flutter test --dart-define-from-file=.env.example`, `flutter build web --release --dart-define-from-file=.env.example`.
- check GitHub state with `gh`:
  - default branch;
  - existing branches;
  - present workflows;
  - configured secrets, without displaying their values.
- create GitHub `production` environments and move secrets to environment
  secrets via `gh secret set --env production`, provided the real values are on
  hand.
- configure some GitHub metadata via `gh repo edit`: description, homepage,
  topics, wiki/discussions/projects according to product choice.
- trigger GitHub Actions builds or checks via `gh workflow run` or `gh run`.

What must not be done blindly in the CLI:

- make repositories public without a freeze and final verification;
- enable branch protections without checking the exact GitHub Actions check
  names available;
- delete or replace production secrets without an inventory of the workflows
  that consume them;
- make GHCR packages public without an explicit decision on the VPS pull mode.

What needs external consoles or a manual decision instead:

- rotation or restriction of old Firebase/GCP/VAPID keys;
- verification of OAuth origins, bundle IDs, Android SHA fingerprints, and
  Google/Firebase authorized domains;
- activation and validation of GitHub Private Vulnerability Reporting once the
  repositories are public;
- final decision on public or private GHCR package visibility;
- decision on brand/logo/asset policy.

## Recommended Sequence on Public-Opening Day

### Step 1. Freeze Merges During the Operation

During the switch:

- avoid simultaneous merges to `main`;
- avoid parallel secret rotations;
- have one person responsible for the switch.

### Step 2. Make the Repositories Public

Change visibility on:

- `fiestaaa_back`
- `fiestaaa_front`

Once the repositories are public, GitHub options currently missing on Free
become available.

### Step 3. Create the `production` Environment

In each repository:

1. `Settings` -> `Environments`
2. create `production`
3. fill the URL:
   - back: `https://api.fiestaaa.app`
   - front: `https://fiestaaa.app`

Then configure:

- `required reviewers`;
- `prevent self-review`;
- `wait timer` if desired;
- deployment branch and tag restriction to `main`.

### Step 4. Move Production Secrets

Move production secrets used by deployment workflows from `repo secrets` to
`production` `environment secrets`.

Keep separate, if needed, some pure build or release secrets that do not depend
directly on the production environment, for example:

- Android signing;
- encoded Android `google-services.json`;
- other non-deployment build secrets.

### Step 5. Enable Branch Protection on `main`

In each repository:

1. `Settings` -> `Branches`
2. add a rule on `main`

Recommended settings:

- `Require a pull request before merging`
- at least 1 approval
- `Dismiss stale pull request approvals when new commits are pushed`
- `Require approval of the most recent reviewable push`
- `Require conversation resolution before merging`
- `Require linear history`
- `Do not allow bypassing the above settings`
- no `force push`
- no protected branch deletion

Checks to make required when they exist:

- `Dependency Review`
- `Backend CI`
- `Frontend CI`

These workflows must already exist before going public so branch protection is
useful immediately.

### Step 6. Enable GitHub Security Features

In each public repository:

- `Dependency graph`
- `Dependabot alerts`
- `Dependabot security updates`
- `Secret scanning`
- `Push protection`

Check in the GitHub UI that every option is enabled; some may depend on account
type or organization settings.

### Step 7. Verify Prepared Protections Become Active

After going public, verify that already committed elements become operational:

- `environment: production` in deployment workflows;
- `Dependency Review` on PRs;
- provenance attestations on GHCR builds;
- deployment and branch rules visible in GitHub.

## Checks Right After Opening

### GitHub Check

- open a test PR and verify that `Dependency Review` runs;
- verify protected branches prevent direct merge;
- verify deployment requires the expected approval and configuration through
  `production`.

### Supply Chain Check

- run a deployment build on a commit with no functional change;
- verify in GHCR that the published image has its attestation;
- verify the VPS can still pull the image according to the retained mode,
  private or public.

### Security Check

- rerun a quick scan of the public repository to confirm no secret appears;
- check GitHub Actions logs to ensure no sensitive variable is printed;
- check artifact downloads if any exist.

## What Will Probably Need to Be Added Before or Right After

Going public will make GitHub protections available, but to reach a stronger
level it will remain useful to complete:

- GitHub Private Vulnerability Reporting activation once the repository is
  public;
- verification of the exact checks to require in branch protection;
- possibly a separate policy for brands, logos, and other assets not intended
  for free reuse;
- possibly an explicit decision on public or private GHCR package visibility.

## Recommended Decision

On the day the app really goes to production and becomes open source:

1. make the repositories public;
2. immediately enable `production`, branch protection, and GitHub security
   options;
3. verify production secrets live only in the GitHub environment and on the VPS;
4. create a test PR to validate the full chain before resuming normal merge
   rhythm.
