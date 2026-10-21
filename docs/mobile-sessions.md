# Persistent mobile sessions

## Diagnosis

Password login, registration completion, Google login and Apple login issued JWTs
expiring after exactly 24 hours. The Flutter app stored that JWT securely but had
no refresh credential or refresh endpoint. At expiry, `/me` or another protected
request returned 401 and the app cleared its session. The observed 24–48-hour
window depended on when the user next opened the app or made a request.

## Behavior

Native clients opt in with `X-Fiestaaa-Auth-Response: bearer-refresh`. They receive
an access JWT lasting one hour and an opaque `refresh_token`. The latter renews the
session without a password using `POST /auth/refresh`. Sessions expire after
90 days without renewal and have a maximum lifetime of 365 days. Each renewal
extends the inactivity deadline, up to that maximum.

The server rotates refresh credentials under a database row lock. A five-minute
retry window for the previous credential returns the current credential, allowing
recovery from a lost response. Credentials are hashed for lookup and encrypted at
rest using the existing database encryption functions. Password reset/global
revocation (`session_version`), suspension, account deletion and expiry prevent
renewal. Logout accepts an optional JSON `refresh_token` and removes the matching
device session in addition to revoking the presented access JWT. Each issued JWT
has a random `jti` so concurrent logins cannot share a revocation entry.

The mobile app stores access and refresh credentials together in secure storage.
All API clients share one renewal operation. Before sending a protected request,
they renew an expiring access token and persist the rotated credentials; the
original request is sent once, including mutations and uploads. Existing pages
holding the previous access token use the current token of the same session.
Network errors, timeouts and temporary server failures keep credentials for a
later retry. Only an explicit authentication refusal clears the session.

## Rollout

1. Deploy the backend first. Migration `013_mobile_sessions.sql` is additive and
   applied by the existing startup migration runner.
2. Build and distribute the updated mobile app. Password, registration, Google
   and Apple login all opt in through the same native request header.
3. A still-valid legacy mobile JWT upgrades on the first protected request. New
   mobile access JWTs are marked and cannot create a long-lived session without
   their refresh credential. Cookie-only requests cannot perform this upgrade. An
   already-expired legacy JWT cannot securely be upgraded: those users need to
   log in once to obtain their first refresh credential.

Old native versions (`bearer`) retain 24-hour JWTs. Browser cookie sessions keep
their existing behavior and never receive refresh credentials in JSON. This
change does not deploy the server or distribute an app build by itself.

## Verification

`tests/mobile_sessions.rs` checks issuance, rotation/retry, expiry, revocation,
suspension, logout, legacy upgrade and native-only credential delivery against an
isolated PostgreSQL database. The Flutter `mobile_session_manager_test.dart`
covers 48-hour inactivity, simultaneous API calls, preserved request bodies,
network/server failures, revocation, logout during renewal and origin isolation.
