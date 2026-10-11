# Specification tooling

Use Node 22 (`.nvmrc`), `npm ci --ignore-scripts`, then `npm run spec:validate`. OpenSpec is pinned to 1.14.1 in package.json and package-lock.json. Codex workflows are generated in .agents/skills. Specifications and change artifacts are English; user-facing text remains French and English. Each change has proposal, design, scenarios and verifiable tasks. Only mark tasks complete when their acceptance criteria pass.

This repository uses its own openspec directory. No shared store or global OpenSpec installation is required. Backend permission rules are authoritative; frontend specifications reference the backend repository. No generated specification is a replacement for server enforcement or the OpenAPI contract.

## Development dependency advisory

The initial 2026-10-10 audit reports GHSA-vfj7-8cjw-p6xm in braces 3.0.3 through OpenSpec → fast-glob → micromatch (four affected package entries, one root advisory). The [official advisory](https://github.com/advisories/GHSA-vfj7-8cjw-p6xm) lists no patched version. Do not use npm audit fix --force: it proposes downgrading OpenSpec to 0.17.2 and would violate the pinned version. This dependency is development-only and excluded from container contexts. Specification CI has no secrets or write permissions and only validates repository files with a timeout. Revisit the dependency when an upstream fix is available; the audit is not clean and is not represented as such.
