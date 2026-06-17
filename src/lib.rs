pub mod api_error;
pub mod auth;
pub mod cleanup;
pub mod config;
pub mod db;
pub mod docs;
pub mod handles;
pub mod models;
pub mod notifications;
pub mod observability;
pub mod rate_limit;
pub mod realtime;
pub mod repositories;
pub mod routes;
pub mod security;
pub mod state;
pub mod user_metrics;

use dotenvy::from_path;
use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

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

pub fn build_http_client(user_agent: &str) -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(user_agent)
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .build()
        .expect("http client")
}

pub fn load_dotenv_from_repo() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(".env");
    if let Err(err) = from_path(&path)
        && path.exists()
    {
        eprintln!("Warning: failed to load {}: {err}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::{install_rustls_crypto_provider, load_dotenv_from_repo};

    #[test]
    fn rustls_provider_install_is_idempotent() {
        install_rustls_crypto_provider();
        install_rustls_crypto_provider();
    }

    #[test]
    fn load_dotenv_from_repo_is_idempotent() {
        load_dotenv_from_repo();
        load_dotenv_from_repo();
    }
}
