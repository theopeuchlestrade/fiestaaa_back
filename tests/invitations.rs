mod common;

use std::error::Error;

use actix_web::{App, http::StatusCode, test};
use chrono::{Duration, NaiveTime, Utc};
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
    let future_date = Utc::now().date_naive() + Duration::days(30);
    sqlx::query_scalar::<_, i64>(
        "INSERT INTO events (name_event, description, date_event, start_time, address, owner_email)
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING event_id",
    )
    .bind("Party")
    .bind("Description")
    .bind(future_date)
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
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let event_id = seed_event(&pool, owner_email).await?;
    let invitee_email = "guest@example.com";
    let _invitee_id = seed_user(&pool, invitee_email).await?;
    let owner_token = admin_token(secret, owner_email).expect("token");
    let invitee_token = admin_token(secret, invitee_email).expect("token");

    // Create invitation
    let resp = test::call_service(
        &app,
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
        &app,
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
        &app,
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
        &app,
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
        &app,
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
        &app,
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

#[tokio::test]
async fn email_invite_share_token_is_bound_to_target_email() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping invitations tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(
        &pool,
        &["event_share_tokens", "invitations", "events", "users"],
    )
    .await?;

    let secret = "secret";
    let owner_email = "owner@example.com";
    let target_email = "future-guest@example.com";
    let attacker_email = "attacker@example.com";
    let state = build_state(pool.clone(), secret, &[]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let event_id = seed_event(&pool, owner_email).await?;
    let owner_token = admin_token(secret, owner_email).expect("token");

    let invite_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!("/events/{}/invitations", event_id))
            .insert_header(("Authorization", format!("Bearer {}", owner_token)))
            .set_json(&InvitationPayload {
                identifier: target_email.into(),
                status: None,
            })
            .to_request(),
    )
    .await;
    assert_eq!(invite_resp.status(), StatusCode::ACCEPTED);

    let share_token: String = sqlx::query_scalar(
        "SELECT token::text FROM event_share_tokens WHERE lower(target_email) = lower($1)",
    )
    .bind(target_email)
    .fetch_one(&pool)
    .await?;

    let _target_id = seed_user(&pool, target_email).await?;
    let _attacker_id = seed_user(&pool, attacker_email).await?;
    let attacker_token = admin_token(secret, attacker_email).expect("token");
    let target_token = admin_token(secret, target_email).expect("token");

    let forbidden_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/share/claim")
            .insert_header(("Authorization", format!("Bearer {}", attacker_token)))
            .set_json(serde_json::json!({ "token": share_token }))
            .to_request(),
    )
    .await;
    assert_eq!(forbidden_resp.status(), StatusCode::FORBIDDEN);

    let success_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/share/claim")
            .insert_header(("Authorization", format!("Bearer {}", target_token)))
            .set_json(serde_json::json!({ "token": share_token }))
            .to_request(),
    )
    .await;
    assert_eq!(success_resp.status(), StatusCode::OK);

    Ok(())
}

#[tokio::test]
async fn share_token_claim_rejects_expired_token() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping invitations tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(
        &pool,
        &["event_share_tokens", "invitations", "events", "users"],
    )
    .await?;

    let secret = "secret";
    let owner_email = "owner@example.com";
    let guest_email = "guest@example.com";
    let state = build_state(pool.clone(), secret, &[]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let event_id = seed_event(&pool, owner_email).await?;
    let owner_token = admin_token(secret, owner_email).expect("token");
    let share_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!("/events/{}/share", event_id))
            .insert_header(("Authorization", format!("Bearer {}", owner_token)))
            .to_request(),
    )
    .await;
    assert_eq!(share_resp.status(), StatusCode::CREATED);
    let share_json: serde_json::Value = test::read_body_json(share_resp).await;
    let share_token = share_json
        .get("token")
        .and_then(|value| value.as_str())
        .expect("share token");

    let _guest_id = seed_user(&pool, guest_email).await?;
    sqlx::query("UPDATE event_share_tokens SET expires_at = NOW() - INTERVAL '1 hour' WHERE token = $1::uuid")
        .bind(share_token)
        .execute(&pool)
        .await?;

    let guest_token = admin_token(secret, guest_email).expect("token");
    let claim_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/share/claim")
            .insert_header(("Authorization", format!("Bearer {}", guest_token)))
            .set_json(serde_json::json!({ "token": share_token }))
            .to_request(),
    )
    .await;
    assert_eq!(claim_resp.status(), StatusCode::GONE);

    Ok(())
}
