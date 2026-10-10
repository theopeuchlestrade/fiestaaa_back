# Event access

## Purpose
Document the existing permission contract used by frontend event routes.

## Requirements

### Requirement: Server authority
The existing server SHALL remain the authority for event access and mutations. The interface SHALL NOT grant access to a disabled or restricted module by opening a route. This adoption SHALL NOT change endpoints or migrations.

#### Scenario: Restricted module access
- **WHEN** a user without the required existing membership accesses a module
- **THEN** the existing server authorization is applied regardless of the frontend destination

### Requirement: Distinct contribution concepts
Needs and personal brings SHALL retain their existing API semantics and authorization. UI specifications SHALL reference this contract rather than redefining permissions.

#### Scenario: Participant contribution
- **WHEN** an authorized participant contributes to an organizer need
- **THEN** the contribution is attached to that need and does not become an organizer-created need

### Requirement: Regression boundaries
Existing invitation, voting, expense, carpool, blocking, password-reset and account-deletion rules SHALL remain unchanged by this presentation work.

#### Scenario: Contract compatibility
- **WHEN** the frontend is built against the documented backend
- **THEN** the OpenAPI contract remains unchanged

### Requirement: Event membership and lifecycle
Protected event lists and mutations SHALL require the owner or an accepted invitation. Waiting invitations SHALL NOT grant protected module access. Finished events SHALL remain readable to authorized members but SHALL reject writes with event_finished.

#### Scenario: Pending invitation
- **WHEN** a waiting guest requests items, polls, expenses or carpools
- **THEN** the server denies access until the invitation is accepted

#### Scenario: Finished event mutation
- **WHEN** an authorized member attempts a protected mutation after the event has finished
- **THEN** the server rejects the mutation and does not change existing content

### Requirement: Item roles
Only the owner SHALL create needs. Accepted members SHALL create personal brings and contribute under the existing quantity limits. Deletion SHALL retain the existing owner-or-creator authorization.

#### Scenario: Non-owner creates a need
- **WHEN** an accepted participant submits item_kind need
- **THEN** the server denies the creation

### Requirement: Poll and expense roles
Only the owner SHALL create polls. Accepted members SHALL vote in active polls under the existing single/multiple choice rules. Expense participants SHALL belong to the event; deletion SHALL remain restricted to the owner, payer or creator.

#### Scenario: Outsider in expense
- **WHEN** a member submits an expense with an unrelated participant
- **THEN** the server rejects invalid_participants

### Requirement: Ticket roles
Only the owner SHALL scan event tickets. Ticket generation and duplicate-scan responses SHALL preserve their existing invitation and lifecycle rules.

#### Scenario: Replayed scan
- **WHEN** the owner scans an already consumed ticket
- **THEN** the server returns the existing already-scanned response

## Source references

- [Membership and lifecycle](../../../src/routes/event_access.rs)
- [Items](../../../src/routes/events/items.rs)
- [Polls](../../../src/routes/events/polls.rs)
- [Expenses](../../../src/routes/events/expenses.rs)
- [Carpools](../../../src/routes/carpools.rs)
- [Invitations](../../../src/routes/invitations.rs)
- [Tickets](../../../src/routes/qr_codes.rs)
- [Account recovery](../../../src/routes/account_recovery.rs)
- [Safety](../../../src/routes/safety.rs)
