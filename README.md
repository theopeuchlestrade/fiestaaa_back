# Rust lines coverage

Commit: `d265604b0859dcf242b3e7a705b13d666f4178c4` · 2026-10-12T20:39:18.557Z

| Measure | Covered / total | Coverage | Enforced / target |
|---|---:|---:|---:|
| Raw collector | 9917/12914 | 76.79% | — |
| Application | 9917/12914 | 76.79% | 75% / 75% |
| authentication-sessions | 1606/1998 | 80.38% | 80% / 80% |
| permissions-participation | 1522/1848 | 82.36% | 80% / 80% |
| recovery-deletion | 93/103 | 90.29% | 80% / 80% |
| notifications-qr | 1206/1474 | 81.82% | 80% / 80% |

Patch: **not-requested**.

Result: **passed**.


## Largest uncovered areas

- `src/routes/events/items.rs`: 304 uncovered collector lines (41.76%)
- `src/routes/auth.rs`: 297 uncovered collector lines (75.64%)
- `src/routes/events/polls.rs`: 285 uncovered collector lines (44.23%)
- `src/routes/carpools.rs`: 271 uncovered collector lines (70.70%)
- `src/routes/invitations.rs`: 252 uncovered collector lines (78.20%)
- `src/routes/events/mod.rs`: 165 uncovered collector lines (78.20%)
- `src/routes/events/core.rs`: 155 uncovered collector lines (81.70%)
- `src/routes/items.rs`: 118 uncovered collector lines (61.06%)
- `src/routes/qr_codes.rs`: 111 uncovered collector lines (71.54%)
- `src/cleanup.rs`: 96 uncovered collector lines (17.95%)
- `src/routes/users.rs`: 94 uncovered collector lines (73.14%)
- `src/routes/events/expenses.rs`: 88 uncovered collector lines (81.59%)
- `src/routes/events/address.rs`: 72 uncovered collector lines (75.09%)
- `src/notifications.rs`: 71 uncovered collector lines (87.88%)
- `src/routes/payment_providers.rs`: 61 uncovered collector lines (73.59%)

## Eligible sources absent from measurement

- `src/repositories/mod.rs`
- `src/routes/realtime.rs`
- `src/state.rs`

## Platform sources not measured

None.

## Documented exclusions

None.

Coverage does not certify native providers, physical devices or store delivery.

[Source CI](https://github.com/theopeuchlestrade/fiestaaa_back/actions/runs/38304925935)
