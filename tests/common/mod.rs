use std::collections::HashSet;

use actix_web::web;
use fiestaaa_back::state::AppState;
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
    let admins = admin_emails
        .iter()
        .map(|email| email.to_lowercase())
        .collect::<HashSet<_>>();

    web::Data::new(AppState {
        db: pool,
        jwt_secret: secret.to_string(),
        admin_emails: admins,
    })
}
