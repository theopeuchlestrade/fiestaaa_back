## Context

Describe the problem or goal addressed by this PR.

## Changes

-

## Verification

- [ ] `cargo fmt --all --check`
- [ ] `cargo clippy --all-targets --all-features -- -D warnings`
- [ ] `cargo test --locked --all-targets --jobs 1 -- --test-threads=1`
- [ ] `docker build --check .`

## Security

- [ ] No secret, token, `.env` file, service account, or private key is added.
- [ ] Changes affecting auth, permissions, personal data, or deployment are explained.
- [ ] Brand, screenshot, icon, logo, and third-party mark changes follow `TRADEMARKS.md`.

## Release Notes

- [ ] `CHANGELOG.md` updated, or this change is not user-facing/releasable.
- [ ] Documentation, configuration, and required secrets are updated if needed.
