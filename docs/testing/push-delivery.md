# Push delivery diagnostics

`notification_outbox.status = sent` is a terminal processing state retained for compatibility. It does not prove that a phone displayed an alert. A no-device attempt, retired invalid token or legacy HTTP acknowledgement may also complete processing.

## Reading evidence

- `push_attempt`: outbox ID, bounded kind, attempt number, active device count and outcome.
- `no_active_device`: no provider call occurred.
- `provider_accepted`: provider acceptance was observed; phone presentation remains a separate check.
- `provider_http_acknowledged`: legacy HTTP success without a positive per-device result.
- `invalid_device`: the provider retired a token; no display is claimed.
- `provider_rejected`: this attempt failed and follows the existing retry policy.
- `push_retry`: retry/dead state and attempt number; no notification text or tokens.
- `not_configured` and `throttled`: transport skipped without claiming delivery.

Never record tokens, notification contents, provider response snippets or account credentials when collecting diagnostic evidence. Mixed device outcomes can require retry under the existing policy; individual provider acceptance is not a display receipt.

## Isolated iPhone reproduction

Use an isolated database/API, two fictional accounts and a separately provisioned QA app. Do not replay production notifications. Enable the QA app's APNs capability, install its matching Firebase client configuration and use authorized test push credentials. The existing email-only QA build with empty entitlements cannot validate real APNs presentation.

For each trial, create a fresh event to avoid the existing ten-minute invitation deduplication window. Send an invitation by the guest's handle from the creator account on a second client. Record app version/build, notification permission and app state, then correlate the outbox attempt with the provider result and actual phone alert. Never record the device token.

| Trial | Guest app state | Checks |
| --- | --- | --- |
| 1 | Foreground | Single alert, tap opens the pending invitation, creator receives no invitation alert |
| 2 | Background | System alert, tap opens the pending invitation |
| 3 | Terminated | System alert, cold-start tap resumes the pending invitation |

After acceptance, create a short-lived poll: the guest receives its alert, the creator receives none and authorized realtime clients still refresh. Repeat the invitation trials with permissions denied to distinguish system suppression from provider failure. Preserve the existing app unless an explicit installation choice authorizes replacement.

The unit/integration fixtures validate outbox behavior and Firebase requests, not iOS presentation. Keep this matrix unverified until the physical trials are actually observed.
