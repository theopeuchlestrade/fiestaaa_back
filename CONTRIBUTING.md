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

## Project Policies
- Follow `CODE_OF_CONDUCT.md` in project spaces.
- Read `SUPPORT.md` before opening support-style issues.
- Read `GOVERNANCE.md` for the maintainer-led decision model.
- Changes touching the Fiestaaa name, public copy, screenshots, icons, logos, or
  third-party marks must follow `TRADEMARKS.md`.

## PR / MR
- Describe the context, change, and impact.
- Add/update tests if applicable.
- Ensure `cargo fmt` and `cargo clippy -D warnings` pass.
- Use a PR title or squash commit message that can become a clear release note.
  Gitmoji is preferred for new work, for example:
  - `✨ (events): Add item reservations`
  - `🐛 (auth): Fix OAuth state refresh`
  - `🔒 (auth): Harden token validation`
  - `⬆️ (deps): Bump Rust dependencies`
- Conventional Commit titles such as `feat(events): add item reservations` and
  `fix(auth): refresh OAuth state` remain accepted during the transition.
- `CHANGELOG.md` is generated automatically during the release workflow from
  commits on `main`; only edit it manually for historical corrections.
- Security vulnerabilities must not be reported through a public issue; follow `SECURITY.md`.
