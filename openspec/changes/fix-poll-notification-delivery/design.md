# Design

Select accepted non-author invitation rows for poll creation. Recheck poll membership in the outbox worker so a later refusal cannot expose private content. Keep the shared event-update recipient helper unchanged.

Expose an internal typed delivery outcome while preserving the existing boolean transport contract and outbox statuses. Log bounded notification kinds, outbox ID, attempt, active device count, outcome and retry scheduling. Never log tokens, titles, bodies or provider response snippets. `sent` means a terminal processing outcome, not proof that iOS displayed a notification. Legacy HTTP acknowledgement without a positive per-device result is distinct from provider acceptance.

Use isolated PostgreSQL/Redis and loopback Firebase/OAuth fixtures. Real APNs presentation requires a separately configured QA iOS app; the existing QA app has empty entitlements and Firebase disabled. Production replay is excluded.
