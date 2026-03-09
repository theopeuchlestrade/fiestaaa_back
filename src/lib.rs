pub mod auth;
pub mod cleanup;
pub mod config;
pub mod db;
pub mod docs;
pub mod handles;
pub mod models;
pub mod notifications;
pub mod rate_limit;
pub mod realtime;
pub mod routes;
pub mod state;

use std::sync::OnceLock;

// reqwest and gcp_auth currently compile rustls with different crypto backends.
// Install one provider explicitly so runtime startup is deterministic.
pub fn install_rustls_crypto_provider() {
    static INIT: OnceLock<()> = OnceLock::new();

    INIT.get_or_init(|| {
        rustls::crypto::ring::default_provider()
            .install_default()
            .expect("failed to install rustls crypto provider");
    });
}

#[cfg(test)]
mod tests {
    use super::install_rustls_crypto_provider;

    #[test]
    fn rustls_provider_install_is_idempotent() {
        install_rustls_crypto_provider();
        install_rustls_crypto_provider();
    }
}
