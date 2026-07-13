# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.1] - 2026-07-13

### Added
- Added per-request identifiers to API responses, access logs, and Sentry
  reports so production failures can be correlated without exposing database
  details to clients.

### Changed
- Bounded client-version Prometheus labels to a fixed set of buckets to prevent
  attacker-controlled metric cardinality growth.
- Validated positive cleanup, refresh, notification, pagination, and rate-limit
  settings at startup so invalid runtime configuration fails fast.
- Enforced a 58 percent backend line-coverage floor in CI.

### Dependencies
- Updated the Rust builder image, Cargo dependencies, and Docker build GitHub
  Actions.

## [0.2.0] - 2026-07-01

### Added
- Added a durable FCM push outbox with recipient-level deduplication, bounded
  delivery concurrency, retry backoff, terminal failure handling, and delivery
  metrics.
- Added timezone-aware event lifecycle fields, owner trash and restore
  endpoints, automatic archival, and delayed permanent deletion.
- Added cursor pagination for events, invitations, friends, friend requests,
  expenses, and carpools.

### Changed
- Centralized authenticated request identity, active-user validation, and event
  access checks.
- Applied keyset pagination defaults of 50 items, with a maximum of 100, and
  added client adoption metrics.
- Added an OpenAPI contract snapshot and a CI coverage regression guard.
- Moved avatar decoding and resizing off the async request executor.

### Fixed
- Made carpool creation, joining, leaving, seat accounting, and deletion
  transactional and safe under concurrent requests.
- Prevented a user from participating in multiple carpools for the same event
  and replaced carpool N+1 reads with grouped queries.
- Made event start and effective end instants safe across timezone and daylight
  saving transitions.

### Dependencies
- Updated the pinned Rust container image.

## [0.1.3] - 2026-06-24

### Dependencies
- *(deps)* Bump the cargo-dependencies group with 4 updates.
- *(deps)* Bump actions/checkout from 6.0.3 to 7.0.0.

### Fixed
- *(auth)* Require explicit bearer token responses (#133).

### Internal
- Use standard Apple OAuth session expiry (#134).
- Refresh pending registration verification links (#135).

## [0.1.2] - 2026-06-20

### Added
- Completed OpenAPI route coverage for the remaining backend endpoints.

### Changed
- Aligned the backend CI PostgreSQL service version with the supported test
  matrix.

### Fixed
- Returned structured backend configuration validation errors instead of
  discarding the underlying validation failure.

## [0.1.1] - 2026-06-17

### Added
- Added backend coverage reporting to CI.
- Added invitation lookup indexes for faster invitation queries.
- Added Docker image vulnerability scanning to the backend CI pipeline.

### Changed
- Made the database pool size configurable.
- Required an explicit test database URL for backend integration tests.
- Hardened backend maintenance paths and open-source maintenance automation.
- Updated backend dependencies, CI actions, Rust, and Debian base image pins.

### Fixed
- Removed QR route row unwraps so missing rows are handled without panics.

## [0.1.0] - 2026-06-05

Initial public-readiness baseline for the Fiestaaa backend.

### Added
- Added brand/assets, code of conduct, support, and governance policies for
  public contribution readiness.
- Added full email/password authentication with registration, email verification, registration completion, login, logout, token revocation, and account deletion.
- Added OAuth authentication support for configured Google and Apple providers.
- Added user APIs for `/me`, handle availability checks, handle updates, avatar uploads, and avatar media serving.
- Added event management APIs for creation, listing, retrieval, replacement, patch updates, deletion, address lookup, feature configuration, and share-link flows.
- Added event feature support for carpools, polls, item lists, ticketing, shared playlists, payment links, and shared expenses.
- Added invitation flows by handle or email, invitation responses, personal invitation listing, owner-side invitation management, and participant-list privacy protections.
- Added recipient-bound email invitation links for guests who are not registered yet.
- Added friend management APIs for search, friend requests, accept/decline actions, friend listing, and removal.
- Added carpool APIs for creation, update, deletion, joining, leaving, duplicate-participation prevention, seat tracking, and sorted listings.
- Added global item catalog management plus event item attachment, custom event items, reservations, and contribution tracking.
- Added event poll creation, voting, listing, and deletion.
- Added shared expense APIs for listing, creation, deletion, balance summaries, and settlement suggestions.
- Added QR check-in flows for guest QR generation, owner scanning, token validation, check-in recording, and scan statistics.
- Added admin CRUD APIs for payment providers and payment-link validation.
- Added device registration, refresh, and revocation endpoints for web/mobile push notifications.
- Added realtime ticket issuance and WebSocket support for event, item, and invitation updates.
- Added health and Prometheus metrics endpoints, including user activity metrics.
- Added OpenAPI/Swagger documentation wiring for the public API surface.
- Added a local user creation helper for development and support workflows.
- Added Docker Compose support for local development with PostgreSQL and Redis.
- Added Dockerfile support for public API image builds.
- Added backend CI jobs for formatting, linting, integration tests with PostgreSQL, dependency auditing, and production container builds.

### Changed
- Changed the package version baseline to `0.1.0` for the first public release.
- Changed backend CI to run the full suite with `cargo test --locked --all-targets --jobs 1 -- --test-threads=1`.
- Changed production configuration to support `TRUST_PROXY_HEADERS=true` behind Traefik or another reverse proxy.
- Pinned the Rust builder and Debian runtime base images used for production Docker builds.

### Security
- Protected emails, addresses, coordinates, and sensitive event identifiers with database encryption.
- Hardened payment-link and playlist validation to reject local, private, and otherwise unsafe targets.
- Enabled trusted proxy header handling in production so rate limiting, logs, and client metadata can use forwarded values safely.
- Fixed event patch handling so omitting a field and explicitly setting it to `null` are treated differently for clearable fields.
- Added configurable rate limits for authentication and invitation flows.
- Added bearer-token protection for Prometheus metrics.
- Added secret scanning, security policy, dependency review, provenance attestation, and public-opening documentation for open-source readiness.
