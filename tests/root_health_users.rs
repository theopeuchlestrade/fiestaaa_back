mod common;

use std::error::Error;

use actix_web::{App, http::StatusCode, test};
use common::{DB_LOCK, build_state, obtain_pool, reset_tables};
use fiestaaa_back::{
    auth::{encode_jwt, hash_password, now_ts},
    models::{Claims, HandleAvailabilityResponse, HandleUpdatePayload},
    routes,
};
use serde::Deserialize;
use sqlx::PgPool;

#[derive(Debug, Deserialize)]
struct TestMeResponse {
    email: String,
    handle: String,
    avatar_url: Option<String>,
    exp: usize,
}

#[derive(Debug, Deserialize)]
struct TestHealthResponse {
    status: String,
    db: Option<String>,
}

fn make_token(secret: &str, email: &str, handle: &str) -> Option<String> {
    let claims = Claims {
        sub: email.to_string(),
        exp: (now_ts() + 3600) as usize,
        handle: handle.to_string(),
    };
    encode_jwt(&claims, secret).ok()
}

async fn seed_user(pool: &PgPool, email: &str, handle: &str) -> sqlx::Result<i64> {
    let hash = hash_password("StrongPassw0rd!").expect("hash");
    sqlx::query_scalar::<_, i64>(
        "INSERT INTO users (email, password_hash, handle)
         VALUES ($1, $2, $3)
         RETURNING id",
    )
    .bind(email)
    .bind(hash)
    .bind(handle)
    .fetch_one(pool)
    .await
}

#[tokio::test]
async fn health_returns_ok_when_database_is_reachable() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping root/health/users tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["users"]).await?;

    let state = build_state(pool, "secret", &[]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let resp = test::call_service(&app, test::TestRequest::get().uri("/health").to_request()).await;

    assert_eq!(resp.status(), StatusCode::OK);
    let body: TestHealthResponse = test::read_body_json(resp).await;
    assert_eq!(body.status, "ok");
    assert_eq!(body.db, None);
    Ok(())
}

#[tokio::test]
async fn me_returns_authenticated_profile() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping root/health/users tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["users"]).await?;

    let secret = "secret";
    seed_user(&pool, "owner@example.com", "owner_handle").await?;
    sqlx::query("UPDATE users SET avatar_url = $1 WHERE lower(email) = lower($2)")
        .bind("https://cdn.example.com/avatar.jpg")
        .bind("owner@example.com")
        .execute(&pool)
        .await?;

    let state = build_state(pool, secret, &[]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;
    let token = make_token(secret, "owner@example.com", "owner_handle").expect("token");

    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/me")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .to_request(),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let body: TestMeResponse = test::read_body_json(resp).await;
    assert_eq!(body.email, "owner@example.com");
    assert_eq!(body.handle, "owner_handle");
    assert_eq!(
        body.avatar_url.as_deref(),
        Some("https://cdn.example.com/avatar.jpg")
    );
    assert!(body.exp > now_ts() as usize);
    Ok(())
}

#[tokio::test]
async fn handle_availability_and_update_cover_validation_and_conflicts()
-> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping root/health/users tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["users"]).await?;

    let secret = "secret";
    seed_user(&pool, "owner@example.com", "owner_handle").await?;
    seed_user(&pool, "taken@example.com", "taken_handle").await?;

    let state = build_state(pool.clone(), secret, &[]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;
    let token = make_token(secret, "owner@example.com", "owner_handle").expect("token");

    let availability_resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/handles/availability?handle=Fresh_Handle")
            .to_request(),
    )
    .await;
    assert_eq!(availability_resp.status(), StatusCode::OK);
    let availability: HandleAvailabilityResponse = test::read_body_json(availability_resp).await;
    assert!(availability.available);

    let invalid_resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/handles/availability?handle=??")
            .to_request(),
    )
    .await;
    assert_eq!(invalid_resp.status(), StatusCode::BAD_REQUEST);

    let conflict_resp = test::call_service(
        &app,
        test::TestRequest::patch()
            .uri("/me/handle")
            .insert_header(("Authorization", format!("Bearer {}", token.clone())))
            .set_json(&HandleUpdatePayload {
                handle: "taken_handle".into(),
            })
            .to_request(),
    )
    .await;
    assert_eq!(conflict_resp.status(), StatusCode::CONFLICT);

    let updated_resp = test::call_service(
        &app,
        test::TestRequest::patch()
            .uri("/me/handle")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .set_json(&HandleUpdatePayload {
                handle: "Fresh_Handle".into(),
            })
            .to_request(),
    )
    .await;
    assert_eq!(updated_resp.status(), StatusCode::OK);
    let updated: TestMeResponse = test::read_body_json(updated_resp).await;
    assert_eq!(updated.handle, "fresh_handle");

    let stored: (String,) =
        sqlx::query_as("SELECT handle FROM users WHERE lower(email) = lower($1)")
            .bind("owner@example.com")
            .fetch_one(&pool)
            .await?;
    assert_eq!(stored.0, "fresh_handle");
    Ok(())
}
