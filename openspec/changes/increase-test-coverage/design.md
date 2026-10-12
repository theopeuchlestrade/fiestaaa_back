# Design

## Context

See proposal.md. Existing CI already collects LCOV. Flutter reports include generated localization code and omit browser-only/unused libraries; LLVM LF/LH include instantiation accounting that differs from unique DA lines. Backend PostgreSQL/Redis services and local HTTP OAuth fixtures already exist. Existing backend event-access is the permission authority.

## Goals / Non-Goals

**Goals:** meaningful behavior coverage, reproducible denominators, 75% application coverage, 80% critical groups, explicit changed-line enforcement, GitHub-only evidence.
**Non-Goals:** API/schema/design changes, private operations, production calls, provider/store certification or coverage inflation via exclusions.

## Decisions

- Keep existing collectors; pin cargo-llvm-cov 0.9.1, Flutter/Rust workflow versions, OpenSpec 1.14.1 and Node 22. New report tools use Node built-ins and node:test without runtime dependencies.
- Version the policy under tool/coverage-policy.json. Filter only generated/dependency/fixture records; inventory application paths and mark unsupported browser adapters separately. Track missing files without inventing executable-line counts. Raw and filtered LF/LH stay separate. Test output paths are relative to the checkout.
- Parse LCOV summary counters as emitted; use DA hit maps for changed-line checks and HTML links. Reject duplicate source records, invalid counters and out-of-checkout paths. LLVM summaries need not equal unique DA counts.
- Record baseline evidence before adding tests. Intermediate floors are the baseline rounded down to a tenth; final floors become 75% and 80% for each group after measured success. Keep raw baseline reference to distinguish changed denominators from added behavior coverage.
- Generate JSON, Markdown, escaped self-contained HTML and SVG from one validated model. HTML contains line highlights from tracked public source files only. Main output records source SHA and run URL; artifacts are seven-day CI outputs, not tracked reports.
- Read git diff --unified=0 against the PR base SHA. Only added executable DA lines count; missing eligible changed files fail, unknown platform adapters are explicitly not measured. No application diff is N/A, not 100%.
- Add a separate workflow_run publication workflow after successful complete main CI. Its guarded job alone has contents:write and actions:read; it checks the source run event/repository/SHA, downloads only that run's report, rechecks current main and updates coverage-reports with per-repository concurrency. PR and failed-run triggers are rejected before checkout. Badge branch holds only badge.svg, summary.json and README.md. Failed/PR runs never publish.
- Add scenario tests to existing suites with deterministic clocks, plugin doubles and loopback HTTP fixtures; PostgreSQL/Redis use dedicated coverage containers. Keep small behavior-preserving extractions only when a provider/clock seam is needed.

## Risks / Trade-offs

- VM coverage omits browser/native integrations → named inventory and separate functional checks; badge says Dart VM.
- Raising floors before adding tests breaks CI → measured baseline floor in the first lot, explicit final targets and task tracking.
- Broad coverage can hide a weak critical group → enforce separate groups and error/authorization assertions.
- Main publication races → SHA check plus Git compare-and-swap push; never force-push or publish a superseded run.
- External calls destabilize tests → local fake services; no production URLs or real credentials in scenarios.

## Delivery

Independent frontend/backend PRs in English, staged measurement/account/modules/final validation commits. No merge, release or deployment. Do not archive until all evidence and final targets pass.
