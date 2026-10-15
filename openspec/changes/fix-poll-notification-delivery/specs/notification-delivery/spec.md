## ADDED Requirements

### Requirement: Private poll push recipients
The server SHALL enqueue poll-created alerts only for accepted guests other than the author, and SHALL recheck this access before dispatch. Authorized realtime updates SHALL remain unchanged.

#### Scenario: Poll creation
- **WHEN** an organizer creates a poll with accepted, waiting, declined and expired invitees
- **THEN** only accepted guests receive queued push alerts and the organizer does not

#### Scenario: Participation revoked before dispatch
- **WHEN** a queued poll recipient declines before dispatch
- **THEN** the worker does not send private poll content to that recipient

### Requirement: Redacted delivery evidence
Push diagnostics SHALL distinguish no active device, provider acceptance, rejected attempt, invalid device, unavailable configuration, throttling and retry scheduling without tokens or notification contents. Existing statuses SHALL NOT be interpreted as confirmed phone presentation.

#### Scenario: No registered device
- **WHEN** an invitation is processed without an active device
- **THEN** no provider request occurs and the trace explicitly reports no active device

#### Scenario: Provider rejection and recovery
- **WHEN** Firebase rejects an attempt and accepts a later retry
- **THEN** traces identify both outcomes and attempts while existing retry rules remain intact

#### Scenario: Provider response contains sensitive strings
- **WHEN** a provider error contains token or message content
- **THEN** the diagnostic trace does not include that content
