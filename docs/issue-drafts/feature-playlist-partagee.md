# [BACK] Shared playlist for event

## Objective
Add support for a shared playlist by event (Spotify/Apple Music/Deezer) with server-side validation.

## Context
Need a shared music space for participants.

## Scope
- Included: link storage, service type, reading in GET event, writing by creator.
- Excluded: realtime synchronization, title import, playlist editing.

## API / Endpoints
- Impacted routes (methods, paths): GET /events/{id}, PATCH /events/{id} (or PATCH /events/{id}/playlist).
- Payloads (inputs/outputs): playlist_url (nullable), playlist_provider (spotify|apple_music|deezer|other).
- Versioning / compatibility: add optional backward-compatible fields.

## Data / Migrations
- Impacted tables/collections: events.
- Required migrations (yes/no): yes, add playlist_url + playlist_provider.
- Backward-compatible data (yes/no): yes (default null values).

## Business Rules / Validations
- Main rules: only creator/admin can modify; known provider; valid URL; removal possible.
- Edge cases: unknown provider, invalid URL, empty URL.

## Security / Permissions
- Roles / rights: creator, admin.
- Sensitive data: none.

## Observability
- Logs / metrics / alerting: log playlist update with event_id + provider.

## Tests
- Unit tests: provider/URL validation.
- Integration: event update + reading.
- E2E (if applicable): event creation with playlist.

## Definition of Done
- [ ] API matches the specs
- [ ] Migrations applied and documented
- [ ] Tests pass
- [ ] Docs updated (if needed)

## Notes / Risks
Provider-specific URL validation (regex or parsing).
