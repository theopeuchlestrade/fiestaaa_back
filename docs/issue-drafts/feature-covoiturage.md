# [BACK] Carpools by event

## Objective
Add a carpool module by event (rides, seats, registrations).

## Context
Trip organization for participants (useful for student associations).

## Scope
- Included: carpool model, creation/update/deletion, join and leave.
- Excluded: chat, payment, navigation.

## API / Endpoints
- Impacted routes (methods, paths):
  - POST /events/{id}/carpools
  - GET /events/{id}/carpools
  - PATCH /carpools/{id}
  - DELETE /carpools/{id}
  - POST /carpools/{id}/join
  - DELETE /carpools/{id}/join
- Payloads (inputs/outputs): seats_total, seats_taken, driver_id, origin, depart_at, notes.
- Versioning / compatibility: new endpoints.

## Data / Migrations
- Impacted tables/collections: carpools, carpool_passengers.
- Required migrations (yes/no): yes.
- Backward-compatible data (yes/no): yes (new tables).

## Business Rules / Validations
- Main rules: seats_taken <= seats_total; a user cannot join twice; the driver cannot take extra seats.
- Edge cases: deleting a ride with passengers, cancellation by the driver.

## Security / Permissions
- Roles / rights: driver can edit/delete; participants can join/leave.
- Sensitive data: optional contact details to protect if added later.

## Observability
- Logs / metrics / alerting: log creation/deletion; join counter.

## Tests
- Unit tests: seat validation.
- Integration: join/leave.
- E2E (if applicable): complete carpool flow.

## Definition of Done
- [ ] API matches the specs
- [ ] Migrations applied and documented
- [ ] Tests pass
- [ ] Docs updated (if needed)

## Notes / Risks
Need a race-condition guard for remaining seats.
