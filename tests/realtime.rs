mod common;

use std::error::Error;

use actix_web::{App, http::StatusCode, test};
use chrono::{NaiveDate, NaiveTime};
use common::{DB_LOCK, build_state, obtain_pool, reset_tables};
use fiestaaa_back::{
    auth::{encode_jwt, hash_password, now_ts},
    models::{Claims, RealtimeTicketResponse},
    routes,
};
use serde::Deserialize;
use sqlx::PgPool;

#[derive(Debug, Deserialize)]
struct TestErrorResponse {
    error: String,
    details: Option<String>,
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
        "INSERT INTO users (email_ciphertext, email_lookup_hash, password_hash, handle)
         VALUES (fiestaaa_encrypt_text($1), fiestaaa_email_lookup($1), $2, $3)
         RETURNING id",
    )
    .bind(email)
    .bind(hash)
    .bind(handle)
    .fetch_one(pool)
    .await
}

async fn seed_event(pool: &PgPool, owner_email: &str) -> sqlx::Result<i64> {
    sqlx::query_scalar::<_, i64>(
        "INSERT INTO events (name_event, description, date_event, start_time, address, owner_email)
         VALUES ($1, $2, $3, $4, $5, $6)
         RETURNING event_id",
    )
    .bind("Realtime Event")
    .bind("A realtime-secured event")
    .bind(NaiveDate::from_ymd_opt(2099, 1, 1).unwrap())
    .bind(NaiveTime::from_hms_opt(20, 0, 0).unwrap())
    .bind("123 Test Street")
    .bind(owner_email)
    .fetch_one(pool)
    .await
}

async fn accept_invitation(pool: &PgPool, event_id: i64, user_id: i64) -> sqlx::Result<()> {
    sqlx::query("INSERT INTO invitations (event_id, user_id, status) VALUES ($1, $2, 'Accepted')")
        .bind(event_id)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

#[tokio::test]
async fn issue_realtime_ticket_requires_authentication() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping realtime tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["invitations", "events", "users"]).await?;

    let secret = "secret";
    let state = build_state(pool.clone(), secret, &[]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let resp = test::call_service(
        &app,
        test::TestRequest::get().uri("/ws-ticket").to_request(),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let body: TestErrorResponse = test::read_body_json(resp).await;
    assert_eq!(body.error, "missing_authorization_header");
    assert!(body.details.is_none());
    Ok(())
}

#[tokio::test]
async fn issue_realtime_ticket_requires_event_membership() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping realtime tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["invitations", "events", "users"]).await?;

    let secret = "secret";
    let owner_email = "owner@example.com";
    seed_user(&pool, owner_email, "owner").await?;
    let member_id = seed_user(&pool, "member@example.com", "member").await?;
    seed_user(&pool, "outsider@example.com", "outsider").await?;
    let event_id = seed_event(&pool, owner_email).await?;
    accept_invitation(&pool, event_id, member_id).await?;

    let state = build_state(pool.clone(), secret, &[]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let member_token = make_token(secret, "member@example.com", "member").expect("token");
    let member_resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(&format!("/ws-ticket?event_id={event_id}"))
            .insert_header(("Authorization", format!("Bearer {member_token}")))
            .to_request(),
    )
    .await;

    assert_eq!(member_resp.status(), StatusCode::OK);
    assert_eq!(
        member_resp
            .headers()
            .get("cache-control")
            .and_then(|value| value.to_str().ok()),
        Some("no-store")
    );
    assert_eq!(
        member_resp
            .headers()
            .get("pragma")
            .and_then(|value| value.to_str().ok()),
        Some("no-cache")
    );
    let ticket: RealtimeTicketResponse = test::read_body_json(member_resp).await;
    assert_eq!(ticket.event_id, Some(event_id));
    assert!(!ticket.ticket.is_empty());

    let outsider_token = make_token(secret, "outsider@example.com", "outsider").expect("token");
    let outsider_resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(&format!("/ws-ticket?event_id={event_id}"))
            .insert_header(("Authorization", format!("Bearer {outsider_token}")))
            .to_request(),
    )
    .await;

    assert_eq!(outsider_resp.status(), StatusCode::FORBIDDEN);
    let body: TestErrorResponse = test::read_body_json(outsider_resp).await;
    assert_eq!(body.error, "forbidden");
    assert_eq!(
        body.details.as_deref(),
        Some("Accès refusé à cette fiestaaa")
    );
    Ok(())
}

#[tokio::test]
async fn issue_realtime_ticket_rejects_untrusted_origin() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping realtime tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["users"]).await?;

    let secret = "secret";
    seed_user(&pool, "member@example.com", "member").await?;

    let state = build_state(pool.clone(), secret, &[]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;
    let token = make_token(secret, "member@example.com", "member").expect("token");

    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/ws-ticket")
            .insert_header(("Origin", "https://evil.example"))
            .insert_header(("Authorization", format!("Bearer {token}")))
            .to_request(),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let body: TestErrorResponse = test::read_body_json(resp).await;
    assert_eq!(body.error, "forbidden_origin");
    assert_eq!(body.details.as_deref(), Some("Origin non autorisee"));
    Ok(())
}
