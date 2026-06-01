# Fiestaaa Back

Fiestaaa's Rust backend, an app for organizing private events.

The API handles authentication, events, invitations, item lists, carpools,
shared expenses, access QR codes, notifications, and realtime streams.

## Stack

- Rust 1.96
- Actix Web
- PostgreSQL via SQLx
- Redis for some ephemeral state
- Docker Compose for local development

## Prerequisites

- Docker CLI + Docker Compose v2
- Rust, if you run the API outside Docker
- A local copy of `.env.example` as `.env`

## Configuration

```bash
cp .env.example .env
```

The values in `.env.example` are placeholders or local development values. Real
secrets must never be committed.

Important variables:

- `DATABASE_URL`: PostgreSQL connection
- `JWT_SECRET`: session signing secret
- `DATA_ENCRYPTION_KEY` and `DATA_LOOKUP_KEY`: application keys, at least 32 characters
- `CORS_ALLOWED_ORIGINS`: allowed frontend origins
- `APP_BASE_URL`: frontend URL for invitation links
- `RESEND_API_KEY` and `INVITATION_EMAIL_SENDER`: invitation email sending
- `FCM_*` and `FIESTAAA_FCM_VAPID_KEY`: push notifications
- `METRICS_BEARER_TOKEN` and `SENTRY_DSN`: production observability

## Local Development

Full startup with Postgres:

```bash
docker compose up --build
```

Local API:

```text
http://127.0.0.1:8080
```

To run the API with `cargo`, start only the database:

```bash
docker compose up -d db
cargo run
```

In this mode, keep a local URL like:

```bash
DATABASE_URL=postgres://postgres:postgres@localhost:5432/fiestaaa
```

## Local User

To create or update a local user directly in the database:

```bash
cargo run --bin create_local_user -- --email test@local.dev --password changeme --handle test_local
```

The command hashes the password with Argon2 and removes any pending registration
for the same email.

## Database

SQL migrations live in `migrations/` and are applied on startup through
`sqlx::migrate!`.

Local reset:

```bash
docker compose down -v
docker compose up --build
```

Or rebuild directly from the current schema:

```bash
./scripts/rebuild_db_from_schema.sh
```

## Quality and Tests

Format:

```bash
cargo fmt --all --check
```

Lint:

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

Tests with Docker:

```bash
docker compose run --rm api cargo test
```

Equivalent CI suite, with an available test database:

```bash
TEST_DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5432/fiestaaa_test cargo test --locked --all-targets --jobs 1 -- --test-threads=1
```

## Deployment

Deployment and operations documentation is in
`docs/deploiement.md`.

The manual `Backend Release` GitHub Actions workflow verifies the release
candidate, bumps the Cargo package version from a `patch`, `minor`, `major`, or
custom version choice, creates the `vX.Y.Z` tag, publishes the GHCR image,
creates the GitHub Release, and can deploy the API to the VPS.

The production compose stack includes Prometheus/Grafana/Loki observability,
external uptime checks, and automated backup/restore-drill scripts.

The transition from private to public repositories is documented in
`docs/passage-public-open-source.md`.

## Security

Do not report vulnerabilities through a public issue. See `SECURITY.md` for the
reporting channel and disclosure expectations.

Before any public release of the repository, rerun a secret scan on the current
state and the full Git history.

## License

`fiestaaa_back` is distributed under the `AGPL-3.0-only` license. See `LICENSE`.
