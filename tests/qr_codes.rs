mod common;

use std::error::Error;

use actix_web::{App, http::StatusCode, test};
use chrono::{NaiveDate, NaiveTime};
use common::{DB_LOCK, build_state, obtain_pool, reset_tables};
use fiestaaa_back::{
    auth::{encode_jwt, hash_password, now_ts},
    models::{
        Claims, QRCodeGenerateResponse, QRCodeScanPayload, QRCodeScanResponse, QRCodeStatsResponse,
    },
    routes,
};
use sqlx::PgPool;

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

async fn seed_event(pool: &PgPool, owner_email: &str) -> sqlx::Result<i64> {
    sqlx::query_scalar::<_, i64>(
        "INSERT INTO events (name_event, description, date_event, start_time, address, owner_email)
         VALUES ($1, $2, $3, $4, $5, $6)
         RETURNING event_id",
    )
    .bind("Test Event")
    .bind("A test event")
    .bind(NaiveDate::from_ymd_opt(2030, 1, 1).unwrap())
    .bind(NaiveTime::from_hms_opt(20, 0, 0).unwrap())
    .bind("123 Test Street")
    .bind(owner_email)
    .fetch_one(pool)
    .await
}

async fn seed_invitation(
    pool: &PgPool,
    event_id: i64,
    user_id: i64,
    status: &str,
) -> sqlx::Result<()> {
    sqlx::query("INSERT INTO invitations (event_id, user_id, status) VALUES ($1, $2, $3)")
        .bind(event_id)
        .bind(user_id)
        .bind(status)
        .execute(pool)
        .await?;
    Ok(())
}

#[tokio::test]
async fn generate_my_qr_code_reuses_existing_token() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping QR code tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["event_checkins", "invitations", "events", "users"]).await?;

    let secret = "secret";
    let guest_id = seed_user(&pool, "guest@example.com", "guest_handle").await?;
    seed_user(&pool, "owner@example.com", "owner_handle").await?;
    let event_id = seed_event(&pool, "owner@example.com").await?;
    seed_invitation(&pool, event_id, guest_id, "Accepted").await?;

    let state = build_state(pool, secret, &[]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;
    let guest_token = make_token(secret, "guest@example.com", "guest_handle").expect("token");

    let first_resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(&format!("/events/{}/my-qr-code", event_id))
            .insert_header(("Authorization", format!("Bearer {}", guest_token.clone())))
            .to_request(),
    )
    .await;
    assert_eq!(first_resp.status(), StatusCode::OK);
    let first: QRCodeGenerateResponse = test::read_body_json(first_resp).await;

    let second_resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(&format!("/events/{}/my-qr-code", event_id))
            .insert_header(("Authorization", format!("Bearer {}", guest_token)))
            .to_request(),
    )
    .await;
    assert_eq!(second_resp.status(), StatusCode::OK);
    let second: QRCodeGenerateResponse = test::read_body_json(second_resp).await;

    assert_eq!(first.event_id, event_id);
    assert_eq!(second.qr_token, first.qr_token);
    assert_eq!(second.generated_at, first.generated_at);
    Ok(())
}

#[tokio::test]
async fn scan_qr_code_updates_stats_and_rejects_duplicate_scans() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping QR code tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["event_checkins", "invitations", "events", "users"]).await?;

    let secret = "secret";
    let guest_id = seed_user(&pool, "guest@example.com", "guest_handle").await?;
    seed_user(&pool, "owner@example.com", "owner_handle").await?;
    let event_id = seed_event(&pool, "owner@example.com").await?;
    seed_invitation(&pool, event_id, guest_id, "Accepted").await?;

    let state = build_state(pool, secret, &[]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;
    let guest_token = make_token(secret, "guest@example.com", "guest_handle").expect("token");
    let owner_token = make_token(secret, "owner@example.com", "owner_handle").expect("token");

    let generate_resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(&format!("/events/{}/my-qr-code", event_id))
            .insert_header(("Authorization", format!("Bearer {}", guest_token)))
            .to_request(),
    )
    .await;
    let generated: QRCodeGenerateResponse = test::read_body_json(generate_resp).await;

    let scan_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!("/events/{}/scan-qr", event_id))
            .insert_header(("Authorization", format!("Bearer {}", owner_token.clone())))
            .set_json(&QRCodeScanPayload {
                token: generated.qr_token.clone(),
            })
            .to_request(),
    )
    .await;
    assert_eq!(scan_resp.status(), StatusCode::OK);
    let scanned: QRCodeScanResponse = test::read_body_json(scan_resp).await;
    assert!(scanned.success);
    assert_eq!(scanned.status, "scanned");
    assert_eq!(scanned.user_email.as_deref(), Some("guest@example.com"));
    assert!(scanned.scanned_at.is_some());

    let stats_resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(&format!("/events/{}/qr-scan-stats", event_id))
            .insert_header(("Authorization", format!("Bearer {}", owner_token.clone())))
            .to_request(),
    )
    .await;
    assert_eq!(stats_resp.status(), StatusCode::OK);
    let stats: QRCodeStatsResponse = test::read_body_json(stats_resp).await;
    assert_eq!(stats.total_invited, 1);
    assert_eq!(stats.total_checked_in, 1);
    assert_eq!(stats.pending_checkins, 0);

    let duplicate_scan_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!("/events/{}/scan-qr", event_id))
            .insert_header(("Authorization", format!("Bearer {}", owner_token)))
            .set_json(&QRCodeScanPayload {
                token: generated.qr_token,
            })
            .to_request(),
    )
    .await;
    assert_eq!(duplicate_scan_resp.status(), StatusCode::CONFLICT);
    let duplicate: QRCodeScanResponse = test::read_body_json(duplicate_scan_resp).await;
    assert!(!duplicate.success);
    assert_eq!(duplicate.status, "already_scanned");
    Ok(())
}
