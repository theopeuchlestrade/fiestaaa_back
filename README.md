# Fiestaaa Back — Docker Dev

Quick dev workflow using Docker Compose for both Postgres and the Rust API.

## Prerequisites
- Docker CLI + Compose v2
- Colima or Docker Desktop running (on macOS, `colima start` + `docker context use colima`)

## Environment
- Copy `.env.example` to `.env` and adjust as needed. The compose file sets
  `DATABASE_URL=postgres://postgres:postgres@db:5432/fiestaaa` for the API container.
- `DATA_ENCRYPTION_KEY` and `DATA_LOOKUP_KEY` are now required. Keep them outside the database and use secrets of at least 32 characters.
- Optionally define `ADMIN_EMAILS` (comma-separated, lower/upper case ignored) to restrict admin endpoints like `/items` to specific accounts.
- For invitation emails to unregistered guests, set `APP_BASE_URL` (used to build the share link) to your front URL
  (ex: `http://localhost:5001` in dev), plus `INVITATION_EMAIL_SENDER` and `RESEND_API_KEY`.

## Run
- `docker compose up --build`
- API: http://127.0.0.1:8080
- Ctrl+C to stop; `docker compose down` to clean up.

## Local test user
- To create or update a local user directly in Postgres, use:
  `cargo run --manifest-path Cargo.toml --bin create_local_user -- --email test@local.dev --password changeme --handle test_local`
- The command hashes the password with Argon2 and removes any pending registration for the same email.
- If `--handle` is omitted, a unique handle is generated automatically.

### Clean database 
- `docker compose down -v`
- `docker compose up --build`
- Or rebuild directly from the single initial migration with
  `./scripts/rebuild_db_from_schema.sh`
  using `migrations/001_initial_schema.sql`.

## Notes
- Migrations run automatically on API startup (via `sqlx::migrate!`).
- The project now assumes a clean-slate migration history: `migrations/001_initial_schema.sql`
  is the full current schema.
- The API container mounts the project directory; code changes rebuild on next run.
- If you prefer local cargo run, start only DB: `docker compose up -d db`, and keep
  `DATABASE_URL=postgres://postgres:postgres@localhost:5432/fiestaaa` in `.env`.
- Owner share links created via `/events/{event_id}/share` intentionally use a bearer-capability model:
  any authenticated user who gets the token can claim the event until the link expires or is used.
  Use email invitations when you need recipient binding.

## Tests
- Run tests with Docker (recommended): `docker compose run --rm api cargo test`
- Alternatively, provide a Postgres instance and set `TEST_DATABASE_URL` (or reuse `DATABASE_URL`), then run `cargo test`.
- For local coverage, leverage LLVM instrumentation:
  ```bash
  rustup component add llvm-tools-preview
  rm -rf coverage && mkdir -p coverage
  export LLVM_PROFILE_FILE="coverage/fiestaaa-%p-%m.profraw"
  export RUSTFLAGS="-Cinstrument-coverage -Clink-dead-code"
  export RUSTDOCFLAGS="-Cinstrument-coverage -Clink-dead-code"
  cargo test
  llvm-profdata merge -sparse coverage/fiestaaa-*.profraw -o coverage/fiestaaa.profdata
  llvm-cov report --use-color --ignore-filename-regex='/.cargo/registry' \
    --instr-profile=coverage/fiestaaa.profdata \
    $(find target/debug/deps -maxdepth 1 -type f \( -name 'fiestaaa_back-*' -o -name 'items-*' \))
  ```
  The `cargo test` run generates `.profraw` files; `llvm-profdata`/`llvm-cov` summarize coverage locally without impacting CI.

## VPS

Connect to the VPS with the following command :

```bash
ssh <username>@fiestaaa.app
```

Available users :
- ubuntu (default/admin)
- theo (admin)
- deploy

## Docs

- Deployment and VPS ops: `docs/deploiement.md`
- Security incident runbook: `docs/incident-securite.md`
- Future switch from private repos to public open source: `docs/passage-public-open-source.md`
