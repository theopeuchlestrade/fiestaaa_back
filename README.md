# Rust lines coverage

Commit: `7eadfc4bf79d85c1d74ca8f6aa7e6920633cc9be` · 2026-11-18T16:01:11.339Z

| Measure | Covered / total | Coverage | Enforced / target |
|---|---:|---:|---:|
| Raw collector | 10472/13351 | 78.44% | — |
| Application | 10472/13351 | 78.44% | 75% / 75% |
| authentication-sessions | 1781/2161 | 82.42% | 80% / 80% |
| permissions-participation | 1530/1860 | 82.26% | 80% / 80% |
| recovery-deletion | 93/103 | 90.29% | 80% / 80% |
| notifications-qr | 1285/1547 | 83.06% | 80% / 80% |

Patch: **not-requested**.

Result: **passed**.


## Largest uncovered areas

- `src/routes/events/items.rs`: 304 uncovered collector lines (41.76%)
- `src/routes/auth.rs`: 297 uncovered collector lines (77.79%)
- `src/routes/carpools.rs`: 271 uncovered collector lines (70.70%)
- `src/routes/invitations.rs`: 251 uncovered collector lines (78.44%)
- `src/routes/events/polls.rs`: 167 uncovered collector lines (67.38%)
- `src/routes/events/mod.rs`: 165 uncovered collector lines (78.79%)
- `src/routes/events/core.rs`: 155 uncovered collector lines (81.70%)
- `src/routes/items.rs`: 118 uncovered collector lines (61.06%)
- `src/routes/qr_codes.rs`: 111 uncovered collector lines (71.39%)
- `src/cleanup.rs`: 98 uncovered collector lines (17.65%)
- `src/routes/users.rs`: 94 uncovered collector lines (73.14%)
- `src/routes/events/expenses.rs`: 88 uncovered collector lines (81.67%)
- `src/routes/events/address.rs`: 72 uncovered collector lines (75.59%)
- `src/notifications.rs`: 65 uncovered collector lines (90.17%)
- `src/main.rs`: 62 uncovered collector lines (0.00%)

## Eligible sources absent from measurement

- `src/repositories/mod.rs`
- `src/routes/realtime.rs`
- `src/state.rs`

## Platform sources not measured

None.

## Documented exclusions

None.

Coverage does not certify native providers, physical devices or store delivery.

[Source CI](https://github.com/theopeuchlestrade/fiestaaa_back/actions/runs/44341181099)
