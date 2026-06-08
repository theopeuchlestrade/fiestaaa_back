# 🎉 Fiestaaa Back

<img src=".github/assets/fiestaaa_logo.png" alt="Fiestaaa Logo" width="120">

[![CI](https://github.com/theopeuchlestrade/fiestaaa_back/actions/workflows/ci.yml/badge.svg)](https://github.com/theopeuchlestrade/fiestaaa_back/actions/workflows/ci.yml)
[![Rust 1.96+](https://img.shields.io/badge/rust-1.96+-000000.svg?logo=rust)](https://www.rust-lang.org)
[![AGPL-3.0 License](https://img.shields.io/badge/license-AGPL--3.0-blue.svg)](LICENSE)
[![Docker](https://img.shields.io/badge/docker-ready-2496ED.svg?logo=docker)](https://www.docker.com)
[![PostgreSQL](https://img.shields.io/badge/postgresql-ready-336791.svg?logo=postgresql)](https://www.postgresql.org)

**Fiestaaa Backend** — The Rust-powered API for organizing private events with friends and family.

---

## 📖 Table of Contents

- [✨ Features](#-features)
- [🚀 Getting Started](#-getting-started)
- [🔧 Development](#-development)
- [📦 Build & Deployment](#-build--deployment)
- [🔒 Security](#-security)
- [📜 License](#-license)
- [🤝 Contributing](#-contributing)

---

## ✨ Features

- **Authentication**: JWT-based sessions with Argon2 password hashing
- **Event Management**: Create, update, and manage private events
- **Invitations**: Email-based invites with customizable messages
- **Item Lists**: Shared shopping lists for events
- **Carpools**: Coordinate rides with participants
- **Shared Expenses**: Track and split costs among attendees
- **Access Control**: QR code-based entry management
- **Notifications**: Push notifications for important updates
- **Realtime Streams**: Live updates via SSE (Server-Sent Events)

---

## 🚀 Getting Started

### Prerequisites

- Docker CLI + Docker Compose v2
- Rust 1.96+ (if running the API outside Docker)

### Quick Start

1. Clone the repository and copy the environment file:
   ```bash
   git clone https://github.com/theopeuchlestrade/fiestaaa_back.git
   cd fiestaaa_back
   cp .env.example .env
   ```

2. Configure your environment variables in `.env`. Required variables include:
   - `DATABASE_URL`: PostgreSQL connection string
   - `JWT_SECRET`: Session signing secret
   - `DATA_ENCRYPTION_KEY`: Application encryption key (at least 32 characters)
   - `DATA_LOOKUP_KEY`: Application lookup key (at least 32 characters)
   - `CORS_ALLOWED_ORIGINS`: Allowed frontend origins
   - `APP_BASE_URL`: Frontend URL for invitation links

3. Start the full stack with Docker:
   ```bash
   docker compose up --build
   ```

4. Access the API at:
   ```
   http://127.0.0.1:8080
   ```

---

## 🔧 Development

### Local Development

To run the API with `cargo`:

1. Start only the database:
   ```bash
   docker compose up -d db
   ```

2. Run the API:
   ```bash
   cargo run
   ```

Use a local database URL like:
```bash
DATABASE_URL=postgres://postgres:postgres@localhost:5432/fiestaaa
```

### Local User Creation

To create or update a local user directly in the database:
```bash
cargo run --bin create_local_user -- --email test@local.dev --password changeme --handle test_local
```

The command hashes the password with Argon2 and removes any pending registration for the same email.

### Database

SQL migrations live in `migrations/` and are applied on startup through `sqlx::migrate!`.

**Local reset:**
```bash
docker compose down -v
docker compose up --build
```

**Rebuild from current schema:**
```bash
./scripts/rebuild_db_from_schema.sh
```

### Quality and Tests

**Format:**
```bash
cargo fmt --all --check
```

**Lint:**
```bash
cargo clippy --all-targets --all-features -- -D warnings
```

**Tests with Docker:**
```bash
cargo test
```

**CI suite with test database:**
```bash
TEST_DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5432/fiestaaa_test \
  cargo test --locked --all-targets --jobs 1 -- --test-threads=1
```

**Coverage with test database:**
```bash
mkdir -p coverage
TEST_DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5432/fiestaaa_test \
  cargo llvm-cov --locked --all-targets --lcov --output-path coverage/lcov.info -- --test-threads=1
```

CI uploads the generated `coverage/lcov.info` report as a workflow artifact.

### OpenAPI Documentation

The API can expose Swagger UI and the generated OpenAPI document locally:

```bash
ENABLE_SWAGGER_UI=true cargo run
```

Then open:

- Swagger UI: `http://127.0.0.1:8080/docs/`
- OpenAPI JSON: `http://127.0.0.1:8080/docs/openapi.json`

---

## 📦 Build & Operations

### Local Container Build

The public repository includes a production-style Dockerfile that can be built
locally:

```bash
docker build -t fiestaaa_back:local .
```

Official production operations, release, backup, observability, and incident
runbooks are maintained outside the public source distribution.

### Public Releases

Public releases and container image metadata may be published from the private
operations repository. The application source, tests, and public CI remain here.

---

## 🔒 Security

⚠️ **Do not report vulnerabilities through public issues.**

See [`SECURITY.md`](SECURITY.md) for the reporting channel and disclosure expectations.

### Security Scans

CI runs:
- Workflow linting
- Dockerfile checks
- Production image vulnerability scanning with Trivy
- Rust dependency auditing with cargo-audit
- Full-history Gitleaks scan on pull requests and pushes to `main`

---

## 📜 License

`fiestaaa_back` is distributed under the **[AGPL-3.0-only](LICENSE)** license.

This license covers the backend source code. Fiestaaa brand assets and third-party marks are handled separately in [`TRADEMARKS.md`](TRADEMARKS.md).

---

## 🤝 Contributing

We welcome contributions! Please see:

- **Contributions**: [`CONTRIBUTING.md`](CONTRIBUTING.md)
- **Code of Conduct**: [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md)
- **Support**: [`SUPPORT.md`](SUPPORT.md)
- **Governance**: [`GOVERNANCE.md`](GOVERNANCE.md)
- **Brand & Assets**: [`TRADEMARKS.md`](TRADEMARKS.md)
- **Architecture**: [`docs/architecture.md`](docs/architecture.md)
- **First contribution**: [`docs/first-contribution.md`](docs/first-contribution.md)
- **Roadmap**: [`ROADMAP.md`](ROADMAP.md)
- **Third-party notices**: [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md)

### Companion Repository

- 🔗 [Fiestaaa Frontend](https://github.com/theopeuchlestrade/fiestaaa_front) — Flutter mobile/web application
