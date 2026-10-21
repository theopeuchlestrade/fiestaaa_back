# Supported build versions

Use a maintained LTS branch where upstream provides one, and the latest
supported stable otherwise. Reviewed on 2026-10-21:

- Rust has no official LTS: compile, format, lint and test with 1.99.1.
  Update `rust-toolchain.toml`, Docker's `RUST_VERSION`, the builder image and
  CI compiler references together. `rust-version` is the compatibility minimum,
  not the compiler selected for builds.
- Tokio 1.53.x is LTS through September 2027. The `~1.53` requirement preserves
  that branch while allowing patch updates; commit the resolved lockfile.
- Node 22 is LTS through April 2027. Keep npm engine requirements and CI aligned.
- PostgreSQL 18 is supported through November 2030. Use the same PostgreSQL and
  Redis patch versions in development and CI as the reviewed infrastructure.
- Debian 12 receives LTS security support through June 2028 for covered packages.

Policies: [Rust](https://rust-lang.org/policies/security/),
[Tokio](https://github.com/tokio-rs/tokio/blob/master/tokio/README.md#bug-patching-policy),
[Node](https://github.com/nodejs/Release),
[PostgreSQL](https://www.postgresql.org/support/versioning/).
