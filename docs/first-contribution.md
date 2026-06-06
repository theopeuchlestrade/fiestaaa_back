# First Contribution

This guide gets a fresh backend checkout to a useful local state for a small
API, documentation, migration, or test contribution.

## 1. Start Local Services

```bash
git clone https://github.com/theopeuchlestrade/fiestaaa_back.git
cd fiestaaa_back
cp .env.example .env
docker compose up --build
```

This starts the API, PostgreSQL, and Redis. The API is available at
`http://127.0.0.1:8080`.

## 2. Create a Local User

In another terminal:

```bash
cargo run --bin create_local_user -- --email test@local.dev --password changeme --handle test_local
```

Use this account when pairing the backend with the frontend during local
manual checks.

## 3. Inspect API Docs

To review the generated API surface locally:

```bash
ENABLE_SWAGGER_UI=true cargo run
```

Then open:

- `http://127.0.0.1:8080/docs/`
- `http://127.0.0.1:8080/docs/openapi.json`

## 4. Verify One Small Change

Before opening a pull request, run the checks relevant to the change:

```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --jobs 1 -- --test-threads=1
```

For docs-only changes, a careful local read-through is enough unless the change
touches commands, configuration, or generated files.
