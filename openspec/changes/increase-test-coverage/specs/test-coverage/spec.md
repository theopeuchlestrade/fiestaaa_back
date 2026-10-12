# Spec Delta

## Purpose

Make application test coverage reproducible, enforce meaningful regression protection and expose current trusted results to repository readers without an external reporting service.

## ADDED Requirements

### Requirement: Transparent measurement
Coverage reports SHALL distinguish raw collector totals, application totals, excluded generated sources and eligible sources absent from measurement. Totals SHALL use collector LF/LH counters rather than averaging file percentages.

#### Scenario: Generated code and missing file
- **WHEN** generated localization code is excluded and an application file is absent from LCOV
- **THEN** the exclusion and absent file are visible separately and neither is presented as added test coverage

### Requirement: Application and critical thresholds
The completed change SHALL achieve at least 75% application line coverage and 80% in each declared critical group. Intermediate lots SHALL enforce a versioned baseline floor and preserve the final targets.

#### Scenario: A group misses its floor
- **WHEN** global coverage passes but a declared critical group misses its enforced floor
- **THEN** the coverage check fails and identifies that group

### Requirement: Changed-line protection
Pull requests SHALL require at least 80% coverage of added or modified executable application lines. An eligible changed file absent from measurement SHALL fail. Documentation-only changes SHALL be marked not applicable.

#### Scenario: Unmeasured new implementation
- **WHEN** a new eligible implementation file is missing from the report
- **THEN** the check fails rather than silently treating its changed lines as covered

#### Scenario: No executable changes
- **WHEN** only documentation or tests change
- **THEN** patch coverage is not applicable and ordinary suite/global checks still run

### Requirement: Invalid reports fail closed
Missing, empty, malformed or internally inconsistent coverage inputs SHALL fail the coverage check. Instrumented line mappings SHALL retain collector summary semantics, including Rust instantiation accounting.

#### Scenario: Empty report
- **WHEN** the input report contains no valid measured application records
- **THEN** no successful zero-line coverage result or green badge is generated

### Requirement: GitHub-visible evidence
Each measured run SHALL provide a GitHub job summary and LCOV, HTML and JSON artifacts retained for seven days. The README badge SHALL identify the measurement kind and link to trusted main results.

#### Scenario: Reader inspects a result
- **WHEN** a reader follows the badge
- **THEN** the result identifies its source commit, date, totals, group outcomes and measurement limits

### Requirement: Trusted publication
Only successful complete main CI SHALL publish compact badge/results to coverage-reports. PRs and forks SHALL have read-only reporting. An older run SHALL NOT replace a newer main result.

#### Scenario: Fork or failed run
- **WHEN** a fork PR or failed CI produces a report
- **THEN** its artifacts remain inspectable but it cannot write the main badge branch

#### Scenario: Main advances during publication
- **WHEN** another main commit supersedes the measured commit before publication
- **THEN** the superseded run leaves the existing published badge unchanged

### Requirement: Isolated behavior validation
Tests SHALL assert outcomes for account, permission, event, synchronization and QR failure scenarios using fictional data and isolated services. Database tests SHALL NOT be silently skipped in CI. Coverage SHALL NOT claim phone certification.

#### Scenario: Missing database service
- **WHEN** database-backed CI tests cannot obtain their dedicated test database
- **THEN** tests fail rather than report successful skipped integration coverage
