mod common;

use std::error::Error;

use actix_web::{App, http::StatusCode, test};
use chrono::{NaiveDate, NaiveTime};
use common::{DB_LOCK, build_state, obtain_pool, reset_tables};
use fiestaaa_back::{
    auth::{encode_jwt, hash_password, now_ts},
    models::{Claims, Invitation, InvitationPatchPayload, InvitationPayload},
    routes,
};
use sqlx::PgPool;

fn admin_token(secret: &str, email: &str) -> Option<String> {
    let handle = email
        .split('@')
        .next()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("user")
        .to_string();
    let claims = Claims {
        sub: email.to_string(),
        exp: (now_ts() + 3600) as usize,
        handle,
    };
    encode_jwt(&claims, secret).ok()
}

async fn seed_user(pool: &PgPool, email: &str) -> sqlx::Result<i64> {
    let hash = hash_password("password").expect("hash");
    sqlx::query_scalar::<_, i64>(
        "INSERT INTO users (email, password_hash) VALUES ($1, $2) RETURNING id",
    )
    .bind(email)
    .bind(hash)
    .fetch_one(pool)
    .await
}

async fn seed_event(pool: &PgPool, owner_email: &str) -> sqlx::Result<i64> {
    sqlx::query_scalar::<_, i64>(
        "INSERT INTO events (name_event, description, date_event, start_time, address, owner_email)
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING event_id",
    )
    .bind("Party")
    .bind("Description")
    .bind(NaiveDate::from_ymd_opt(2024, 7, 1).unwrap())
    .bind(NaiveTime::from_hms_opt(20, 0, 0).unwrap())
    .bind("123 Street")
    .bind(owner_email)
    .fetch_one(pool)
    .await
}

#[tokio::test]
async fn invitations_crud_flow() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping invitations tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["invitations", "events", "users"]).await?;

    let secret = "secret";
    let owner_email = "owner@example.com";
    let state = build_state(pool.clone(), secret, &[]);
    let mut app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let event_id = seed_event(&pool, owner_email).await?;
    let invitee_email = "guest@example.com";
    let _invitee_id = seed_user(&pool, invitee_email).await?;
    let owner_token = admin_token(secret, owner_email).expect("token");
    let invitee_token = admin_token(secret, invitee_email).expect("token");

    // Create invitation
    let resp = test::call_service(
        &mut app,
        test::TestRequest::post()
            .uri(&format!("/events/{}/invitations", event_id))
            .insert_header(("Authorization", format!("Bearer {}", owner_token.clone())))
            .set_json(&InvitationPayload {
                identifier: invitee_email.into(),
                status: None,
            })
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let created: Invitation = test::read_body_json(resp).await;
    assert_eq!(created.status, "Waiting");

    // List invitations
    let resp = test::call_service(
        &mut app,
        test::TestRequest::get()
            .uri(&format!("/events/{}/invitations", event_id))
            .insert_header(("Authorization", format!("Bearer {}", owner_token.clone())))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let listed: Vec<Invitation> = test::read_body_json(resp).await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].email, invitee_email);

    // Invitee fetches their invitations
    let resp = test::call_service(
        &mut app,
        test::TestRequest::get()
            .uri("/my/invitations")
            .insert_header(("Authorization", format!("Bearer {}", invitee_token.clone())))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let mine: Vec<Invitation> = test::read_body_json(resp).await;
    assert_eq!(mine.len(), 1);

    // Invitee accepts
    let resp = test::call_service(
        &mut app,
        test::TestRequest::patch()
            .uri(&format!("/my/invitations/{}", event_id))
            .insert_header(("Authorization", format!("Bearer {}", invitee_token.clone())))
            .set_json(&InvitationPatchPayload {
                status: Some("Accepted".into()),
            })
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let updated: Invitation = test::read_body_json(resp).await;
    assert_eq!(updated.status, "Accepted");

    // Delete invitation
    let resp = test::call_service(
        &mut app,
        test::TestRequest::delete()
            .uri(&format!(
                "/events/{}/invitations/{}",
                event_id, invitee_email
            ))
            .insert_header(("Authorization", format!("Bearer {}", owner_token.clone())))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    // Ensure list is empty
    let resp = test::call_service(
        &mut app,
        test::TestRequest::get()
            .uri(&format!("/events/{}/invitations", event_id))
            .insert_header(("Authorization", format!("Bearer {}", owner_token)))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let listed: Vec<Invitation> = test::read_body_json(resp).await;
    assert!(listed.is_empty());

    Ok(())
}
