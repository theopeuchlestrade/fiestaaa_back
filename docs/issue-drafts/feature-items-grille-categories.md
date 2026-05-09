# [BACK] Item grid + categories + quantity summary

## Objective
Add simple categories to items and provide a quantity summary by category.

## Context
The item list is long and hard to browse. We want to filter and quickly see quantities.

## Scope
- Included: category field on item, value validation, summary by category.
- Excluded: UI rendering (handled on the frontend).

## API / Endpoints
- Impacted routes (methods, paths): item endpoints to accept category; GET /events/{id}/items (or summary) returns the category.
- Payloads (inputs/outputs): category enum (soft|alcohol|savory|sweet|other), quantity if available.
- Versioning / compatibility: optional backward-compatible field.

## Data / Migrations
- Impacted tables/collections: items.
- Required migrations (yes/no): yes, add item_category.
- Backward-compatible data (yes/no): yes, default value "other".

## Business Rules / Validations
- Main rules: category required for new items; closed value set.
- Edge cases: existing items without category, conversion.

## Security / Permissions
- Roles / rights: according to current item rules.
- Sensitive data: none.

## Observability
- Logs / metrics / alerting: none.

## Tests
- Unit tests: category validation.
- Integration: item creation + summary reading.
- E2E (if applicable): grid + filter display on the frontend.

## Definition of Done
- [ ] API matches the specs
- [ ] Migrations applied and documented
- [ ] Tests pass
- [ ] Docs updated (if needed)

## Notes / Risks
Clarify the source of "quantity" (number of joined users vs explicit field).
