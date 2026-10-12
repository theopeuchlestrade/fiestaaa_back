# Test coverage

## Reproducible baseline

Source: `281524f2765c99be2e38b4f013151ec2a257775b`. Full isolated suite on macOS arm64 with pinned dependencies; raw CI reference: 63.61%. Local raw: 63.61%; application: 63.61% (8199/12890). The baseline source inventory is in [coverage-baseline.json](coverage-baseline.json).

| Critical group | Baseline | Current floor | Final target |
|---|---:|---:|---:|
| authentication-sessions | 66.83% | 80% | 80% |
| permissions-participation | 53.25% | 80% | 80% |
| recovery-deletion | 72.82% | 80% | 80% |
| notifications-qr | 55.49% | 80% | 80% |

Final application target: **75%**. The versioned policy ratchets floors only after added tests are measured. Excluding generated sources is a denominator change, not an added-test gain.

## Run locally

Use Node 22 for tooling. Run `npm run coverage:test`, then collect with the repository's documented Flutter or cargo-llvm-cov command. Rust collector version is 0.9.1; database and Redis variables must target isolated services, with skip flags unset. Run `npm run coverage:report` to create `coverage/report/{lcov.info,summary.json,index.html,README.md,badge.svg}`. For patch coverage, append `-- --base <full-base-commit-sha>`.

The report uses collector LF/LH counters for raw/application/group totals. LLVM summaries count instantiations and can differ from unique DA lines. DA mappings power changed-line checks and source highlighting. No average of file percentages is used. No measured application lines, invalid counters, missing inputs or an eligible changed file absent from LCOV fail closed. Changes without added executable application lines are N/A, not 100%.

## Measurement boundary

The source inventory distinguishes generated exclusions, explicitly unsupported platform adapters, eligible files absent from LCOV, measured files with zero hits and covered files. Missing source lines are not invented. Browser-only frontend adapters are unmeasured by Dart VM and retain separate browser checks. Native provider, camera, push and store behavior cannot be certified by desktop coverage or plugin doubles. No screen, business handler, server binary or cleanup task is excluded merely because it is difficult to test.

## GitHub publication

CI runs read-only on PRs/forks and uploads one LCOV/HTML/JSON bundle retained for seven days. Its job summary shows source SHA, raw/application/group/patch metrics, missing inventory and failures.

After the complete main push CI succeeds, a separate guarded workflow downloads only that source run's report. It verifies source run repository/event/branch/conclusion/SHA, checks current main before publication, and updates only compact badge/summary/README files on `coverage-reports`. The publisher alone has contents:write; it never executes PR code. Ref updates are non-forced and serialized. A superseded, failed or fork run cannot replace a newer badge. No external account, Pages site, PAT or source-branch bot commit is required.

**Bootstrap:** the live README badge does not exist before this tooling is merged and main CI succeeds. During review, inspect the PR report artifact/HTML instead. No badge branch is created during preparation.

## Acceptance

Global floor 75%, each critical group 80%, changed executable lines 80%. Targeted behavior suites must pass three consecutive times and the full existing CI must pass. All API/contracts remain unchanged. Physical-device and browser checks are recorded separately from instrumented line coverage; no release or production action is part of this change.

## Behavior-test evidence

The fresh full isolated LLVM collection reaches **76.78%**. Critical groups: authentication/sessions 80.38%, permissions/participation 82.36%, recovery/deletion 90.29%, notifications/QR 81.82%. The enforced policy now requires 75% globally and 80% per group.

New integration tests exercise PostgreSQL/Redis on dedicated loopback services, independently generated provider signing keys, local Apple/FCM OAuth servers, refused CONNECT through a local proxy for email transport failures, database read-only failures, generic/throttled password recovery, JWT expiry and cookie/header precedence, Apple reauthorization and revocation retries, invitation deadlines and resource release on declining, report privacy and moderation, device rotation/outbox retries and QR expiry/duplicate denial. The local user command is verified in isolated subprocesses; configuration probes do not inherit developer credentials.

Only small internal transport injection helpers and the existing bounded outbox batch are exposed for testing; all runtime defaults, HTTP APIs and migrations remain unchanged. All new tests live outside application `src`. Historical inline Rust unit-test mappings and generic instantiations remain included in the collector's source LF/LH accounting, consistently with the baseline; they are not added to inflate coverage. This source-based metric is documented rather than presented as native-device certification.

Strict Clippy, formatting, 28 reporting/publication guard tests and strict OpenSpec validation pass. Complete CI, OpenAPI generation and builds are verified separately on the PR. No production services, emails or notifications are contacted. Physical phone validation is not performed for this test-only change.

Stability: all 195 tests pass in the fresh complete collection, followed by two consecutive successful runs of the 27 new targeted scenarios (account/permissions, provider protocols, isolated configuration and local operations). Added/modified executable application lines are 93.10% covered (27/29).

CI discovered five current RustSec advisories for the existing transitive `quinn-proto` 0.11.15 dependency. The minimal lockfile update to 0.11.18 addresses them; no runtime APIs/features are changed. The full suite is rerun after this update. Local `cargo audit -D warnings --no-yanked` passes. See [RUSTSEC-2026-0340](https://github.com/RustSec/advisory-db/blob/main/crates/quinn-proto/RUSTSEC-2026-0340.md).

The secret scanner flagged the public RFC 6455 handshake nonce in the WebSocket integration fixture. A narrow allowlist requires both that exact public value and that exact test path, preserving the default scan rules for all other content/history. A local scan verifies 289 commits and reports no leaks.

A fresh full collection after the security update confirms 76.78% on 12,914 mapped lines; all critical groups still exceed 80%. Parser guards also reject nonzero summaries without line mappings and non-finite thresholds; the 28 tooling tests pass.

## GitHub CI confirmation

[Complete backend CI](https://github.com/theopeuchlestrade/fiestaaa_back/actions/runs/38292209892) passes on implementation commit `da68788` (tested merge source `75fb2b9e3151b406db88b9f7ac7d93c333e68053`). Ubuntu LLVM application source coverage is **76.79% (9,917/12,914)**; critical groups are 80.38%, 82.36%, 90.29% and 81.82%. Changed executable application lines are 93.10% (27/29). The report artifact is available from that run for seven days. All tests, exact OpenAPI comparison, Clippy, formatting, container build/security scan, dependency audit and secret scan pass.

The coordinated changes are [backend PR #218](https://github.com/theopeuchlestrade/fiestaaa_back/pull/218) and [frontend PR #206](https://github.com/theopeuchlestrade/fiestaaa_front/pull/206). Final evidence edits affect documentation only. Temporary local PostgreSQL/Redis services have been stopped. No source code is merged, deployed or released by this preparation.
