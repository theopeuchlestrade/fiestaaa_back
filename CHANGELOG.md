# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Added full email/password authentication with registration, email verification, registration completion, login, logout, and account deletion.
- Added API support for configured OAuth providers.
- Added user endpoints for `/me`, handle availability checks, handle updates, and avatar uploads.
- Added event management APIs for creation, retrieval, update, deletion, address lookup, and share-link flows.
- Added invitation flows by handle, invitation responses, personal invitation listing, and participant-list privacy protections.
- Added email invitation flows with recipient-bound share links for guests who are not registered yet.
- Added friend management APIs for search, friend requests, accept/decline, and removal.
- Added carpool APIs for creation, updates, deletion, participation, leaving, and sorted listings.
- Added global item management plus event item attachment, custom items, reservations, and contribution tracking.
- Added event poll creation, voting, listing, and deletion.
- Added shared expense APIs for listing, creation, deletion, and balance/settlement summaries.
- Added QR check-in flows for guest QR generation, owner scanning, and scan statistics.
- Added admin CRUD for payment providers and payment-link validation.
- Added device registration, refresh, and revocation endpoints for notifications.

### Changed
- Changed backend CI to run the full suite with `cargo test --locked --all-targets --jobs 1 -- --test-threads=1`.
- Changed integration test fixtures to match the current encrypted database schema.
- Changed production deployment to use immutable image tags instead of relying on `latest`.
- Changed deployment workflows to run public smoke checks after rollout.
- Changed production configuration to support `TRUST_PROXY_HEADERS=true` behind Traefik or another reverse proxy.
- Pinned the Rust builder and Debian runtime base images used for production Docker builds.
- Removed the `latest` image publication path from the backend deployment workflow in favor of commit-SHA tags only.
- Made `API_IMAGE_TAG` and `FRONT_IMAGE_TAG` explicit deployment prerequisites for production compose/bootstrap flows.

### Security
- Protected emails, addresses, coordinates, and sensitive event identifiers with database encryption.
- Hardened payment-link and playlist validation to reject local, private, and otherwise unsafe targets.
- Enabled trusted proxy header handling in production so rate limiting, logs, and client metadata can use forwarded values safely.
- Fixed event patch handling so omitting a field and explicitly setting it to `null` are treated differently for clearable fields.
