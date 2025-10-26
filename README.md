Fiestaaa Back — Docker Dev

Quick dev workflow using Docker Compose for both Postgres and the Rust API.

Prerequisites
- Docker CLI + Compose v2
- Colima or Docker Desktop running (on macOS, `colima start` + `docker context use colima`)

Environment
- Copy `.env.example` to `.env` and adjust as needed. The compose file sets
  `DATABASE_URL=postgres://postgres:postgres@db:5432/fiestaaa` for the API container.

Run
- ``docker compose up --build
- API: http://127.0.0.1:8080
- Ctrl+C to stop; `docker compose down` to clean up.

Notes
- Migrations run automatically on API startup (via `sqlx::migrate!`).
- The API container mounts the project directory; code changes rebuild on next run.
- If you prefer local cargo run, start only DB: `docker compose up -d db`, and keep
  `DATABASE_URL=postgres://postgres:postgres@localhost:5432/fiestaaa` in `.env`.

