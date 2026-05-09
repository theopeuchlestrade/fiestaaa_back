# [BACK] Separate bring vs need items

## Objective
Distinguish "what I bring" (everyone) items from "what I need" (creator) items.

## Context
The current model mixes items requested by the creator with participant offers.

## Scope
- Included: add an item type (bring/need) and associated access rules.
- Excluded: item categories (soft drinks/alcohol, etc.) handled in another issue.

## API / Endpoints
- Impacted routes (methods, paths): item endpoints (create/update/list) to accept item_kind.
- Payloads (inputs/outputs): item_kind enum (need|bring).
- Versioning / compatibility: optional field with default value.

## Data / Migrations
- Impacted tables/collections: items.
- Required migrations (yes/no): yes, add item_kind.
- Backward-compatible data (yes/no): yes, default value "need" for existing items.

## Business Rules / Validations
- Main rules: creator/admin can create/update "need"; participants can create "bring".
- Edge cases: converting an item from "need" to "bring" and the reverse; deletion.

## Security / Permissions
- Roles / rights: creator/admin vs participant.
- Sensitive data: none.

## Observability
- Logs / metrics / alerting: log item type changes.

## Tests
- Unit tests: item_kind and permission validation.
- Integration: creation of need vs bring items.
- E2E (if applicable): separation visible on the frontend.

## Definition of Done
- [ ] API matches the specs
- [ ] Migrations applied and documented
- [ ] Tests pass
- [ ] Docs updated (if needed)

## Notes / Risks
Possible impact on the UI and existing sorting.
