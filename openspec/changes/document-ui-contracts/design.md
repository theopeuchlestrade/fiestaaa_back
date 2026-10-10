## Context
The clean frontend base is 83df069 and backend base is eb49e14. The primary working directories contain unrelated uncommitted work and remain untouched. Current main already contains shared event forms and reusable async/realtime components.

## Goals / Non-Goals
Preserve every function and server permission while presenting one complete copy of information per page. No backend behavioral change, portal, publication or production deployment.

## Decisions
Use per-repository OpenSpec with simple relative cross references. Backend event-access is authoritative; frontend specs reference it. Use three named destinations and full routes for event modules. Keep controllers, validation, draft guards, realtime refresh queues and short input dialogs. Keep white/black tickets and camera surfaces as intentional exceptions. Public legal wording is unchanged.

Use the existing theme with neutral surfaces, consistent spacing, a 1200px page width and 760px form width. At 720px use a navigation rail. When available width or 200% text scaling requires it, stack actions instead of truncating essential text.

## Risks / Trade-offs
Direct module routes must check membership and enabled features before mounting module content. Authentication restoration must retain the destination without retaining sensitive token URLs. Existing visual snapshots will change intentionally. Development-only OpenSpec currently reports a braces stack-exhaustion advisory; inspect remediation without downgrading the requested tool or changing runtime dependencies.

## Migration Plan
Review coordinated PRs, then create a separate tested beta candidate. No migration or deployment in this change. Rollback is reverting frontend presentation changes; backend contracts are documentation only.

## Validation
Run strict OpenSpec validation, Flutter analysis, relevant functional tests and full CI. Compare light/dark FR/EN at 360/720/1024/1440 and 200% text. Check keyboard, semantic labels, contrast, touch targets, back/reload, notification destinations, offline/realtime behavior and role gates. Manual device results remain pending until actually performed.
