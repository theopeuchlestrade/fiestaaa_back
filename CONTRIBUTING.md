# Contributing to fiestaaa_back

Thanks for contributing to the backend.

## Prerequisites
- Rust (toolchain stable)
- Docker + Docker Compose (recommended for Postgres)

## Installation
1. Copy `.env.example` to `.env` and adjust if needed.
2. Start Postgres through Docker:
   - `docker compose up --build`

## Run the API
- Docker (recommended): `docker compose up --build`
- Local: `docker compose up -d db`, then `cargo run` with a local `DATABASE_URL`.

## Pre-commit (Local Hooks)
The hooks run `cargo fmt` + `cargo clippy -D warnings`.

Install the hooks:
- From this repo: `sh scripts/install-hooks.sh`
- From the mono-repo root: `sh scripts/install-hooks.sh`

For a one-off bypass if needed: `SKIP_LINT=1 git commit ...`

## Lint / Format
- `cargo fmt --all`
- `cargo clippy --all-targets --all-features -- -D warnings`

## Tests
- Docker: `docker compose run --rm api cargo test`
- Local: `cargo test` (requires `TEST_DATABASE_URL` or `DATABASE_URL`).
- Full CI suite: `cargo test --locked --all-targets --jobs 1 -- --test-threads=1`

## Migrations
SQL migrations live in `migrations/` and are applied on startup through `sqlx::migrate!`.

## PR / MR
- Describe the context, change, and impact.
- Add/update tests if applicable.
- Ensure `cargo fmt` and `cargo clippy -D warnings` pass.
- Update `CHANGELOG.md` for any notable releasable, user-facing, security, production infrastructure, or DX change.
- Security vulnerabilities must not be reported through a public issue; follow `SECURITY.md`.
