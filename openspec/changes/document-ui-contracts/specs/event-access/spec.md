## ADDED Requirements

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
