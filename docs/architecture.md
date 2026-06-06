# Architecture

Fiestaaa Back is the Rust API for the public Fiestaaa application. It owns
server-side authentication, event data, invitations, realtime streams,
notifications, media serving, and permission checks.

## Runtime Shape

- The API is built with Actix Web and exposes JSON HTTP routes plus realtime
  streams for selected event workflows.
- PostgreSQL stores users, events, invitations, encrypted personal data, item
  lists, carpools, polls, expenses, QR access records, notification devices,
  and admin-managed payment providers.
- Redis supports runtime coordination such as notification deduplication.
- Firebase Cloud Messaging delivers push notifications to registered devices.
- Google and Apple OAuth are optional runtime integrations enabled through
  environment variables.
- The Flutter frontend consumes the API and never connects directly to
  PostgreSQL or Redis.

## Configuration

Local configuration is copied from `.env.example` to `.env`. Do not commit
`.env`, service-account files, private keys, API keys, production inventory, or
generated upload data.

OpenAPI documentation is generated in-process. Set `ENABLE_SWAGGER_UI=true`
when running locally to expose `/docs/` and `/docs/openapi.json`.

## Public vs Private Operations

This repository contains source code, migrations, local Docker Compose,
production-style container builds, public CI, and security checks. Official
production deployment, backups, observability, secret rotation, incident
response, and rollback runbooks are maintained outside this public source
repository.
