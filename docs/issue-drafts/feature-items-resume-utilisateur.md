# [BACK] User item summary by event

## Objective
Expose a summary of the items a user participates in for an event.

## Context
Today, users have to browse the full item list to find what they should bring.

## Scope
- Included: "my items" endpoint by event, sorting, and readable format.
- Excluded: notifications, exports.

## API / Endpoints
- Impacted routes (methods, paths): GET /events/{id}/my-items (auth) or my_items field in GET /events/{id}.
- Payloads (inputs/outputs): item list with quantity, status, category (if available).
- Versioning / compatibility: add an endpoint or optional field.

## Data / Migrations
- Impacted tables/collections: items + participation table.
- Required migrations (yes/no): no.
- Backward-compatible data (yes/no): yes.

## Business Rules / Validations
- Main rules: return only items associated with the current user.
- Edge cases: user is not a participant, empty list.

## Security / Permissions
- Roles / rights: authenticated user, event member.
- Sensitive data: none.

## Observability
- Logs / metrics / alerting: none, or debug log if needed.

## Tests
- Unit tests: item selection by user.
- Integration: endpoint returns the correct items.
- E2E (if applicable): summary display on the frontend.

## Definition of Done
- [ ] API matches the specs
- [ ] Tests pass
- [ ] Docs updated (if needed)

## Notes / Risks
Check performance if the item list is large.
