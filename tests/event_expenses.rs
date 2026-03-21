mod common;

use std::error::Error;

use actix_web::{App, http::StatusCode, test};
use chrono::{NaiveDate, NaiveTime};
use common::{DB_LOCK, build_state, obtain_pool, reset_tables};
use fiestaaa_back::{
    auth::{encode_jwt, hash_password, now_ts},
    models::{Claims, EventExpenseView, EventExpensesSummaryView},
    routes,
};
use serde::Deserialize;
use serde_json::json;
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

async fn seed_event(pool: &PgPool, owner_email: &str, date: NaiveDate) -> sqlx::Result<i64> {
    sqlx::query_scalar::<_, i64>(
        "INSERT INTO events (name_event, description, date_event, start_time, address, owner_email)
         VALUES ($1, $2, $3, $4, $5, $6)
         RETURNING event_id",
    )
    .bind("Test Event")
    .bind("A shared expenses event")
    .bind(date)
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

fn balance_for_user(summary: &EventExpensesSummaryView, user_id: i64) -> Option<(i64, i64, i64)> {
    summary
        .balances
        .iter()
        .find(|balance| balance.user_id == user_id)
        .map(|balance| {
            (
                balance.paid_cents,
                balance.owed_cents,
                balance.balance_cents,
            )
        })
}

#[tokio::test]
async fn create_event_expense_supports_explicit_payer_and_summary() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping event_expenses tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(
        &pool,
        &[
            "event_expense_participants",
            "event_expenses",
            "invitations",
            "events",
            "users",
        ],
    )
    .await?;

    let secret = "secret";
    let owner_id = seed_user(&pool, "owner@example.com", "owner").await?;
    let alice_id = seed_user(&pool, "alice@example.com", "alice").await?;
    let bob_id = seed_user(&pool, "bob@example.com", "bob").await?;
    let event_id = seed_event(
        &pool,
        "owner@example.com",
        NaiveDate::from_ymd_opt(2099, 1, 1).unwrap(),
    )
    .await?;
    accept_invitation(&pool, event_id, alice_id).await?;
    accept_invitation(&pool, event_id, bob_id).await?;

    let state = build_state(pool.clone(), secret, &[]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;
    let alice_token = make_token(secret, "alice@example.com", "alice").expect("token");

    let create_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!("/events/{}/expenses", event_id))
            .insert_header(("Authorization", format!("Bearer {}", alice_token.clone())))
            .set_json(json!({
                "title": "Courses",
                "amount_cents": 900,
                "paid_by_user_id": bob_id,
                "participant_user_ids": [alice_id, bob_id],
                "note": "Apéro et petit-déj"
            }))
            .to_request(),
    )
    .await;

    assert_eq!(create_resp.status(), StatusCode::CREATED);
    let created: EventExpenseView = test::read_body_json(create_resp).await;
    assert_eq!(created.event_id, event_id);
    assert_eq!(created.paid_by_user_id, bob_id);
    assert_eq!(created.paid_by_handle.as_deref(), Some("bob"));
    assert_eq!(created.amount_cents, 900);
    assert_eq!(
        created
            .participants
            .iter()
            .map(|item| item.user_id)
            .collect::<Vec<_>>(),
        vec![alice_id, bob_id]
    );

    let summary_resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(&format!("/events/{}/expenses/summary", event_id))
            .insert_header(("Authorization", format!("Bearer {}", alice_token)))
            .to_request(),
    )
    .await;

    assert_eq!(summary_resp.status(), StatusCode::OK);
    let summary: EventExpensesSummaryView = test::read_body_json(summary_resp).await;
    assert_eq!(summary.total_expenses_cents, 900);
    assert_eq!(balance_for_user(&summary, owner_id), Some((0, 0, 0)));
    assert_eq!(balance_for_user(&summary, alice_id), Some((0, 450, -450)));
    assert_eq!(balance_for_user(&summary, bob_id), Some((900, 450, 450)));
    assert_eq!(summary.settlements.len(), 1);
    assert_eq!(summary.settlements[0].from_user_id, alice_id);
    assert_eq!(summary.settlements[0].to_user_id, bob_id);
    assert_eq!(summary.settlements[0].amount_cents, 450);
    Ok(())
}

#[tokio::test]
async fn expense_creator_can_delete_even_if_someone_else_paid() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping event_expenses tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(
        &pool,
        &[
            "event_expense_participants",
            "event_expenses",
            "invitations",
            "events",
            "users",
        ],
    )
    .await?;

    let secret = "secret";
    seed_user(&pool, "owner@example.com", "owner").await?;
    let alice_id = seed_user(&pool, "alice@example.com", "alice").await?;
    let bob_id = seed_user(&pool, "bob@example.com", "bob").await?;
    let event_id = seed_event(
        &pool,
        "owner@example.com",
        NaiveDate::from_ymd_opt(2099, 6, 1).unwrap(),
    )
    .await?;
    accept_invitation(&pool, event_id, alice_id).await?;
    accept_invitation(&pool, event_id, bob_id).await?;

    let state = build_state(pool.clone(), secret, &[]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;
    let alice_token = make_token(secret, "alice@example.com", "alice").expect("token");

    let create_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!("/events/{}/expenses", event_id))
            .insert_header(("Authorization", format!("Bearer {}", alice_token.clone())))
            .set_json(json!({
                "title": "Maison",
                "amount_cents": 1200,
                "paid_by_user_id": bob_id,
                "participant_user_ids": [alice_id, bob_id]
            }))
            .to_request(),
    )
    .await;
    assert_eq!(create_resp.status(), StatusCode::CREATED);
    let created: EventExpenseView = test::read_body_json(create_resp).await;

    let delete_resp = test::call_service(
        &app,
        test::TestRequest::delete()
            .uri(&format!(
                "/events/{}/expenses/{}",
                event_id, created.expense_id
            ))
            .insert_header(("Authorization", format!("Bearer {}", alice_token.clone())))
            .to_request(),
    )
    .await;

    assert_eq!(delete_resp.status(), StatusCode::OK);

    let list_resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(&format!("/events/{}/expenses", event_id))
            .insert_header(("Authorization", format!("Bearer {}", alice_token)))
            .to_request(),
    )
    .await;

    assert_eq!(list_resp.status(), StatusCode::OK);
    let expenses: Vec<EventExpenseView> = test::read_body_json(list_resp).await;
    assert!(expenses.is_empty());
    Ok(())
}

#[tokio::test]
async fn finished_event_blocks_new_shared_expenses() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping event_expenses tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(
        &pool,
        &[
            "event_expense_participants",
            "event_expenses",
            "invitations",
            "events",
            "users",
        ],
    )
    .await?;

    let secret = "secret";
    seed_user(&pool, "owner@example.com", "owner").await?;
    let guest_id = seed_user(&pool, "guest@example.com", "guest").await?;
    let event_id = seed_event(
        &pool,
        "owner@example.com",
        NaiveDate::from_ymd_opt(2020, 1, 1).unwrap(),
    )
    .await?;
    accept_invitation(&pool, event_id, guest_id).await?;

    let state = build_state(pool.clone(), secret, &[]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;
    let guest_token = make_token(secret, "guest@example.com", "guest").expect("token");

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!("/events/{}/expenses", event_id))
            .insert_header(("Authorization", format!("Bearer {}", guest_token)))
            .set_json(json!({
                "title": "Taxi",
                "amount_cents": 2300,
                "participant_user_ids": [guest_id]
            }))
            .to_request(),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let payload: TestErrorResponse = test::read_body_json(resp).await;
    assert_eq!(payload.error, "event_finished");
    assert!(payload.details.unwrap_or_default().contains("terminée"));
    Ok(())
}
