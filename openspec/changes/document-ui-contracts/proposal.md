## Why
Fiestaaa screens use inconsistent layouts and repeat information. A shared visual language and explicit navigation will make the existing functions easier to find without changing the API.

## What Changes
- Introduce local OpenSpec 1.14.1 tooling under Node 22 and strict CI validation.
- Document the server permission contracts used by the interface.
- Provide authoritative existing contracts for the coordinated frontend harmonization PR.
- Keep violet/cyan, Manrope, French/English and all existing features.

## Capabilities
### New Capabilities
- `event-access`: authoritative event permissions (backend).

### Modified Capabilities
None; this is initial specification adoption.

## Impact
Frontend presentation and development tooling only. No API changes, migrations, production changes, tags or distribution. Existing local changes stay untouched. Phone checks are recorded separately from automated checks.
