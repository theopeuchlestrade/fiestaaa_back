## Context

Describe the problem or goal addressed by this PR.

## Changes

-

## Verification

- [ ] `cargo fmt --all --check`
- [ ] `cargo clippy --all-targets --all-features -- -D warnings`
- [ ] `cargo test --locked --all-targets --jobs 1 -- --test-threads=1`
- [ ] `docker build --check .`
- [ ] `ruby scripts/check_markdown_links.rb`
- [ ] CI `Workflow Lint`, `Secret Scan`, `Dockerfile Check`, and `Container Build & Scan` passed.

## Security

- [ ] No secret, token, `.env` file, service account, or private key is added.
- [ ] Changes affecting auth, permissions, personal data, or deployment are explained.
- [ ] Brand, screenshot, icon, logo, and third-party mark changes follow `TRADEMARKS.md`.

## Release Notes

- [ ] PR title or squash commit is suitable for generated release notes.
- [ ] Prefer Gitmoji style such as `✨ (events): Add item reservations`; Conventional Commit style remains accepted.
- [ ] Documentation, configuration, and required secrets are updated if needed.
