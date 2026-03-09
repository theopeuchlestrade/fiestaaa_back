use std::collections::HashSet;

use actix_web::web;
use fiestaaa_back::{
    notifications::NotificationService, rate_limit::AuthRateLimiter, state::AppState,
};
use once_cell::sync::Lazy;
use sqlx::{PgPool, postgres::PgPoolOptions};
use tokio::sync::Mutex;

pub static DB_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

pub async fn obtain_pool() -> Option<PgPool> {
    let url = std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .ok()?;

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .ok()?;

    if let Err(e) = sqlx::migrate!("./migrations").run(&pool).await {
        eprintln!("Skipping tests: failed to run migrations: {e}");
        return None;
    }

    Some(pool)
}

pub async fn reset_tables(pool: &PgPool, tables: &[&str]) -> sqlx::Result<()> {
    if tables.is_empty() {
        return Ok(());
    }
    let names = tables.join(", ");
    let query = format!("TRUNCATE {} RESTART IDENTITY CASCADE", names);
    sqlx::query(&query).execute(pool).await?;
    Ok(())
}

pub fn build_state(pool: PgPool, secret: &str, admin_emails: &[&str]) -> web::Data<AppState> {
    fiestaaa_back::install_rustls_crypto_provider();

    let admins = admin_emails
        .iter()
        .map(|email| email.to_lowercase())
        .collect::<HashSet<_>>();
    let http_client = reqwest::Client::builder()
        .user_agent("fiestaaa-backend-tests")
        .build()
        .expect("http client");
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
        avatar_upload_dir: "./uploads/avatars".into(),
        avatar_base_url: "http://localhost:8080/media/avatars".into(),
        redis_client: None,
        notifications,
        fcm_project_id: None,
        google_client_id: None,
        google_android_client_id: None,
        apple_app_id: None,
        apple_service_id: None,
        google_ios_client_id: None,
        auth_rate_limiter: AuthRateLimiter::new(1000, std::time::Duration::from_secs(60)),
    })
}
