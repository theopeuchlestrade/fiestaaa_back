# Proposal

## Why

The back CI reports 63.61% raw line coverage, but the enforced floor is outdated and repository readers cannot inspect a current badge or actionable coverage gaps. High-risk account, membership and synchronization behavior needs direct regression tests before further beta releases.

## What Changes

- Define a reproducible, versioned application coverage policy with inventory, generated-source exclusions and explicit unmeasured platform adapters.
- Add behavior/error/authorization tests, ratcheting application line coverage to 75% and each critical group to 80%.
- Require 80% coverage on added/modified executable application lines; absent eligible files and invalid reports fail closed.
- Publish GitHub job summaries and seven-day LCOV/HTML/JSON artifacts, with a small main-only badge/summary on coverage-reports after full CI success.
- Validate report generation, thresholds and publication race/fork safety; preserve API, storage and product behavior.

## Capabilities

### New Capabilities

- `test-coverage`: Reproducible coverage measurement, meaningful scenario coverage, enforcement and GitHub-only publication.

### Modified Capabilities

None. Existing application behavior remains unchanged. Frontend permissions refer to backend `event-access` rather than redefining its rules.

## Impact

Development tests, coverage tools, CI, README and OpenSpec documentation only. Node 22 and OpenSpec 1.14.1 remain pinned; the Rust collector is pinned to 0.9.1. No migration, API change, private-repository work, third-party reporting, production call or release.
