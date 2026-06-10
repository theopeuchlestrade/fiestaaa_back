use std::collections::HashSet;

use actix_web::web;
use fiestaaa_back::{
    db, notifications::NotificationService, rate_limit::AuthRateLimiter, state::AppState,
};
use once_cell::sync::Lazy;
use sqlx::{AssertSqlSafe, PgPool};
use tokio::sync::Mutex;

pub static DB_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));
const TEST_DATA_ENCRYPTION_KEY: &str = "test-data-encryption-key-32-chars!!";
const TEST_DATA_LOOKUP_KEY: &str = "test-data-lookup-key-32-chars!!!!!!";

#[derive(Default)]
pub struct TestOAuthConfig {
    pub google_client_id: Option<String>,
    pub google_android_client_id: Option<String>,
    pub google_ios_client_id: Option<String>,
    pub apple_app_id: Option<String>,
    pub apple_service_id: Option<String>,
    pub google_tokeninfo_url: Option<String>,
    pub google_userinfo_url: Option<String>,
    pub apple_jwks_url: Option<String>,
}

pub async fn obtain_pool() -> Option<PgPool> {
    let url = std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .ok()?;

    Some(db::connect_and_migrate(&url, 5, TEST_DATA_ENCRYPTION_KEY, TEST_DATA_LOOKUP_KEY).await)
}

pub async fn reset_tables(pool: &PgPool, tables: &[&str]) -> sqlx::Result<()> {
    if tables.is_empty() {
        return Ok(());
    }
    let names = tables.join(", ");
    let query = format!("TRUNCATE {} RESTART IDENTITY CASCADE", names);
    sqlx::query(AssertSqlSafe(query)).execute(pool).await?;
    Ok(())
}

pub fn build_state(pool: PgPool, secret: &str, admin_emails: &[&str]) -> web::Data<AppState> {
    build_state_with_avatar_storage(
        pool,
        secret,
        admin_emails,
        "./uploads/avatars".into(),
        "http://localhost:8080/media/avatars".into(),
    )
}

pub fn build_state_with_avatar_storage(
    pool: PgPool,
    secret: &str,
    admin_emails: &[&str],
    avatar_upload_dir: String,
    avatar_base_url: String,
) -> web::Data<AppState> {
    build_state_with_avatar_storage_and_oauth_config(
        pool,
        secret,
        admin_emails,
        avatar_upload_dir,
        avatar_base_url,
        TestOAuthConfig::default(),
    )
}

#[allow(dead_code)]
pub fn build_state_with_oauth_config(
    pool: PgPool,
    secret: &str,
    admin_emails: &[&str],
    oauth: TestOAuthConfig,
) -> web::Data<AppState> {
    build_state_with_avatar_storage_and_oauth_config(
        pool,
        secret,
        admin_emails,
        "./uploads/avatars".into(),
        "http://localhost:8080/media/avatars".into(),
        oauth,
    )
}

fn build_state_with_avatar_storage_and_oauth_config(
    pool: PgPool,
    secret: &str,
    admin_emails: &[&str],
    avatar_upload_dir: String,
    avatar_base_url: String,
    oauth: TestOAuthConfig,
) -> web::Data<AppState> {
    fiestaaa_back::install_rustls_crypto_provider();

    let admins = admin_emails
        .iter()
        .map(|email| email.to_lowercase())
        .collect::<HashSet<_>>();
    let http_client = fiestaaa_back::build_http_client("fiestaaa-backend-tests");
    let notifications = NotificationService::new(None, None, None, None, http_client.clone(), 300);

    web::Data::new(AppState {
        db: pool,
        jwt_secret: secret.to_string(),
        admin_emails: admins,
        trust_proxy_headers: false,
        http_client,
        geocoding_base_url: "https://nominatim.openstreetmap.org".into(),
        geocoding_country_codes: None,
        invitation_email_sender: None,
        invitation_email_api_key: None,
        app_base_url: "http://localhost:3000".into(),
        cors_allowed_origins: HashSet::from(["http://localhost:3000".to_string()]),
        avatar_upload_dir,
        avatar_base_url,
        redis_client: None,
        notifications,
        fcm_project_id: None,
        google_client_id: oauth.google_client_id,
        google_android_client_id: oauth.google_android_client_id,
        google_ios_client_id: oauth.google_ios_client_id,
        apple_app_id: oauth.apple_app_id,
        apple_service_id: oauth.apple_service_id,
        google_tokeninfo_url: oauth
            .google_tokeninfo_url
            .unwrap_or_else(|| "https://oauth2.googleapis.com/tokeninfo".into()),
        google_userinfo_url: oauth
            .google_userinfo_url
            .unwrap_or_else(|| "https://www.googleapis.com/oauth2/v3/userinfo".into()),
        apple_jwks_url: oauth
            .apple_jwks_url
            .unwrap_or_else(|| "https://appleid.apple.com/auth/keys".into()),
        auth_rate_limiter: AuthRateLimiter::new(1000, std::time::Duration::from_secs(60), None),
        invitation_rate_limiter: AuthRateLimiter::new(
            1000,
            std::time::Duration::from_secs(60),
            None,
        ),
        metrics_bearer_token: Some("test-metrics-token".into()),
    })
}
