This is the crates.io source of utoipa-swagger-ui 10.0.1, with its original
MIT/Apache licenses. The only source changes are the ZIP build dependency 8.6
to 9.0 and propagation of the now-fallible file.name() result in build.rs.
These follow the upstream fix visible at:
https://github.com/juhaku/utoipa/tree/master/utoipa-swagger-ui

They remove RUSTSEC-2026-0367, RUSTSEC-2026-0368 and RUSTSEC-2026-0369 from
the build dependency tree. Remove this override when a published upstream
release includes ZIP 9 or later. No application runtime API is changed.
