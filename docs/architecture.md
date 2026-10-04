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

## Realtime Recovery

The WebSocket sends `{"type":"realtime.ready"}` after all Redis subscriptions
are active. Clients should reload their subscribed resources on this message,
including on the first connection, because Redis PubSub does not replay changes
missed while disconnected. Existing change messages are unchanged.

Redis connection/subscription failures and ended streams close the WebSocket;
clients reconnect with backoff. Closing a socket cancels its Redis work. When
Redis is not configured, the existing `realtime_disabled` warning remains.
Deploy this backend before the frontend that consumes readiness messages.

Redis integration tests require an isolated `TEST_REDIS_URL` (CI supplies one).
These tests disconnect PubSub clients on that server; never point them at a
shared development or production Redis. Database tests continue to use the
isolated `TEST_DATABASE_URL`.

## Address-search provider safeguards

Address searches are explicit authenticated actions, never typing-driven
autocomplete. `GEOCODING_BASE_URL` remains configurable server-side without an
app update. The default provider is public Nominatim; review its usage policy
before rollout: https://operations.osmfoundation.org/policies/nominatim/.

A shared Redis lease serializes cache misses across API workers and instances.
An upstream request has a ten-second deadline, no automatic redirects, and a
64 KiB response limit. The lease lasts 30 seconds while work is in progress;
success or failure leaves a one-second cooldown. Cancellation leaves the longer
lease to expire. Busy callers receive HTTP 429 with Retry-After: 1; clients must
retry explicitly. Missing/unavailable Redis fails closed with HTTP 502, without
a local fallback or an upstream call. Redis is therefore required for address
searches even when the rest of the API can run without it.

The shared FIFO cache holds at most 128 response entries, each physically expiring
after 24 hours. Cache keys hash provider, country configuration, exact trimmed
query and limit; cache values contain place labels and coordinates. Query length
is bounded to 256 UTF-8 bytes. Existing request logging records URL paths rather
than query strings. Do not send confidential data or personal names to the
provider; the frontend displays privacy guidance and OSM/ODbL attribution.

Use the same Redis service for every API instance using this provider. Do not
flush/reset the gate during live searches; after a Redis restart, allow existing
upstream requests to drain before admitting traffic. A lease is a capacity guard,
not an availability guarantee; increased beta traffic may require a different
provider. No migration or deployment is part of this change.

## Private recovery decision journal

Migration 012 adds a private, encrypted journal written by PostgreSQL triggers in
the same transaction as account deletion, session/password/suspension changes,
avatar removal, event removal/hiding, report closure/retirement, moderation terms
and Apple revocation queue creation/completion. A failed journal write rolls back
the action. Journal rows have no foreign keys to deleted accounts and no public
API route. Payloads use the existing field encryption key; they include only
recovery state, never email bodies or report comments. Recovery of password state
uses encrypted password hashes; pending Apple jobs retain already-encrypted
credentials inside the encrypted payload.

Existing pending Apple jobs receive a stable recovery key derived from their
client ID and stored token ciphertext. Retries do not create completion records;
the successful worker's queue deletion does. Do not manually delete pending jobs
to suppress errors, as that would represent completion in the journal.

Exports must read the whole journal in one consistent database snapshot. Sequence
numbers can be allocated before another transaction commits; an incremental
`sequence > last_seen` export could miss an older transaction. The database
journal alone does not provide off-host durability. Private backup/recovery
procedures must preserve the latest independent export, verify its lineage and
freshness, reconcile later decisions against an older restored database, and
review retention against all recoverable snapshots before retiring metadata.
Migration 012 cannot reconstruct decisions made before it was installed and does
not itself activate exports, apply recovery changes or send provider requests.
