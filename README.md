# Fiestaaa Back — Docker Dev

Quick dev workflow using Docker Compose for both Postgres and the Rust API.

## Prerequisites
- Docker CLI + Compose v2
- Colima or Docker Desktop running (on macOS, `colima start` + `docker context use colima`)

## Environment
- Copy `.env.example` to `.env` and adjust as needed. The compose file sets
  `DATABASE_URL=postgres://postgres:postgres@db:5432/fiestaaa` for the API container.
- Optionally define `ADMIN_EMAILS` (comma-separated, lower/upper case ignored) to restrict admin endpoints like `/items` to specific accounts.
- For invitation emails to unregistered guests, set `APP_BASE_URL` (used to build the share link) to your front URL
  (ex: `http://localhost:5001` in dev), plus `INVITATION_EMAIL_SENDER` and `RESEND_API_KEY`.
- Monitoring: set `METRICS_TOKEN` to protect `/metrics`, and `GRAFANA_ADMIN_PASSWORD` for Grafana (required by the monitoring compose).

## Run
- `docker compose up --build`
- API: http://127.0.0.1:8080
- Ctrl+C to stop; `docker compose down` to clean up.

### Clean database 
- `docker compose down -v`
- `docker compose up --build`

## Notes
- Migrations run automatically on API startup (via `sqlx::migrate!`).
- The API container mounts the project directory; code changes rebuild on next run.
- If you prefer local cargo run, start only DB: `docker compose up -d db`, and keep
  `DATABASE_URL=postgres://postgres:postgres@localhost:5432/fiestaaa` in `.env`.

## Monitoring (Prometheus/Grafana)
- Make sure the API is running on `:8080` and that `METRICS_TOKEN` matches the `bearer_token` in `prometheus.yml`.
- Start the stack: `docker compose -f docker-compose.monitoring.yml up -d`
- Prometheus: http://127.0.0.1:9090
- Grafana: http://127.0.0.1:3000 (user `admin`, password from `GRAFANA_ADMIN_PASSWORD`)
- `/metrics` requires a token when `METRICS_TOKEN` is set:
  ```bash
  curl -H "Authorization: Bearer $METRICS_TOKEN" http://127.0.0.1:8080/metrics
  ```
- If you change the token, update `prometheus.yml` accordingly.

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
