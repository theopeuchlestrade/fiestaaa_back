mod common;

use std::error::Error;

use actix_web::{App, http::StatusCode, test};
use chrono::{Duration, NaiveTime, Utc};
use common::{DB_LOCK, build_state, obtain_pool, reset_tables};
use fiestaaa_back::{
    auth::{encode_jwt, hash_password, now_ts},
    models::{Claims, Invitation, InvitationPatchPayload, InvitationPayload},
    routes,
    security::sha256_hex,
};
use serde::Deserialize;
use sqlx::PgPool;
use uuid::Uuid;

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

#[derive(Debug, Deserialize)]
struct TestErrorResponse {
    error: String,
}

async fn seed_user(pool: &PgPool, email: &str) -> sqlx::Result<i64> {
    let hash = hash_password("password").expect("hash");
    let handle = email
        .split('@')
        .next()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("user")
        .replace(|c: char| !c.is_ascii_alphanumeric(), "_");
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

async fn ensure_user(pool: &PgPool, email: &str) -> sqlx::Result<i64> {
    if let Some(user_id) = sqlx::query_scalar::<_, i64>(
        "SELECT id FROM users WHERE email_lookup_hash = fiestaaa_email_lookup($1)",
    )
    .bind(email)
    .fetch_optional(pool)
    .await?
    {
        return Ok(user_id);
    }

    seed_user(pool, email).await
}

async fn seed_event(pool: &PgPool, owner_email: &str) -> sqlx::Result<i64> {
    let future_date = Utc::now().date_naive() + Duration::days(30);
    let owner_user_id = ensure_user(pool, owner_email).await?;

    sqlx::query_scalar::<_, i64>(
        "INSERT INTO events (
            name_event,
            description,
            date_event,
            start_time,
            address_ciphertext,
            owner_user_id
         )
         VALUES ($1, $2, $3, $4, fiestaaa_encrypt_text($5), $6)
         RETURNING event_id",
    )
    .bind("Party")
    .bind("Description")
    .bind(future_date)
    .bind(NaiveTime::from_hms_opt(20, 0, 0).unwrap())
    .bind("123 Street")
    .bind(owner_user_id)
    .fetch_one(pool)
    .await
}

async fn seed_pending_share_token(
    pool: &PgPool,
    event_id: i64,
    owner_email: &str,
    target_email: &str,
) -> sqlx::Result<String> {
    let token = Uuid::new_v4().to_string();
    let owner_user_id = ensure_user(pool, owner_email).await?;
    sqlx::query(
        "INSERT INTO event_share_tokens (
            token_hash,
            event_id,
            created_by_user_id,
            target_email_ciphertext,
            target_email_lookup_hash,
            expires_at
         )
         VALUES (
            $1,
            $2,
            $3,
            fiestaaa_encrypt_text($4),
            fiestaaa_email_lookup($4),
            NOW() + INTERVAL '7 days'
         )",
    )
    .bind(sha256_hex(&token))
    .bind(event_id)
    .bind(owner_user_id)
    .bind(target_email)
    .execute(pool)
    .await?;
    Ok(token)
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
                identifier: "guest".into(),
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
    assert_eq!(listed.len(), 2);
    assert!(
        listed
            .iter()
            .any(|invitation| invitation.email.eq_ignore_ascii_case(owner_email))
    );
    assert!(
        listed
            .iter()
            .any(|invitation| invitation.email.eq_ignore_ascii_case(invitee_email))
    );

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
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].email, owner_email);

    Ok(())
}

#[tokio::test]
async fn waiting_invitee_cannot_list_event_invitations() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping invitations tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["invitations", "events", "users"]).await?;

    let secret = "secret";
    let owner_email = "owner@example.com";
    let guest_email = "guest@example.com";
    let state = build_state(pool.clone(), secret, &[]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let _owner_id = seed_user(&pool, owner_email).await?;
    let _guest_id = seed_user(&pool, guest_email).await?;
    let event_id = seed_event(&pool, owner_email).await?;
    let owner_token = admin_token(secret, owner_email).expect("token");
    let guest_token = admin_token(secret, guest_email).expect("token");

    let invite_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!("/events/{}/invitations", event_id))
            .insert_header(("Authorization", format!("Bearer {}", owner_token)))
            .set_json(&InvitationPayload {
                identifier: "guest".into(),
                status: None,
            })
            .to_request(),
    )
    .await;
    assert_eq!(invite_resp.status(), StatusCode::CREATED);

    let list_resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(&format!("/events/{}/invitations", event_id))
            .insert_header(("Authorization", format!("Bearer {}", guest_token)))
            .to_request(),
    )
    .await;

    assert_eq!(list_resp.status(), StatusCode::FORBIDDEN);
    Ok(())
}

#[tokio::test]
async fn owner_invitation_routes_keep_not_found_for_missing_events() -> Result<(), Box<dyn Error>> {
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

    let _owner_id = seed_user(&pool, owner_email).await?;
    let owner_token = admin_token(secret, owner_email).expect("token");

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/events/999999/invitations")
            .insert_header(("Authorization", format!("Bearer {}", owner_token)))
            .set_json(&InvitationPayload {
                identifier: "guest".into(),
                status: None,
            })
            .to_request(),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let body: TestErrorResponse = test::read_body_json(resp).await;
    assert_eq!(body.error, "event_not_found");
    Ok(())
}

#[tokio::test]
async fn participant_list_hides_other_emails_and_pending_entries() -> Result<(), Box<dyn Error>> {
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
    let alice_email = "alice@example.com";
    let bob_email = "bobby@example.com";
    let pending_email = "future-guest@example.com";
    let state = build_state(pool.clone(), secret, &[]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let _owner_id = seed_user(&pool, owner_email).await?;
    let _alice_id = seed_user(&pool, alice_email).await?;
    let _bob_id = seed_user(&pool, bob_email).await?;
    let event_id = seed_event(&pool, owner_email).await?;
    let owner_token = admin_token(secret, owner_email).expect("token");
    let alice_token = admin_token(secret, alice_email).expect("token");
    let bob_token = admin_token(secret, bob_email).expect("token");

    for identifier in ["alice", "bobby"] {
        let invite_resp = test::call_service(
            &app,
            test::TestRequest::post()
                .uri(&format!("/events/{}/invitations", event_id))
                .insert_header(("Authorization", format!("Bearer {}", owner_token.clone())))
                .set_json(&InvitationPayload {
                    identifier: identifier.into(),
                    status: None,
                })
                .to_request(),
        )
        .await;
        assert_eq!(invite_resp.status(), StatusCode::CREATED);
    }

    let _pending_share_token =
        seed_pending_share_token(&pool, event_id, owner_email, pending_email).await?;

    for token in [&alice_token, &bob_token] {
        let accept_resp = test::call_service(
            &app,
            test::TestRequest::patch()
                .uri(&format!("/my/invitations/{}", event_id))
                .insert_header(("Authorization", format!("Bearer {}", token)))
                .set_json(&InvitationPatchPayload {
                    status: Some("Accepted".into()),
                })
                .to_request(),
        )
        .await;
        assert_eq!(accept_resp.status(), StatusCode::OK);
    }

    let list_resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(&format!("/events/{}/invitations", event_id))
            .insert_header(("Authorization", format!("Bearer {}", alice_token)))
            .to_request(),
    )
    .await;
    assert_eq!(list_resp.status(), StatusCode::OK);
    let listed: Vec<Invitation> = test::read_body_json(list_resp).await;

    assert_eq!(listed.len(), 3);
    assert!(
        listed
            .iter()
            .all(|invitation| !invitation.email.eq_ignore_ascii_case(pending_email))
    );

    let owner = listed
        .iter()
        .find(|invitation| invitation.handle.as_deref() == Some("owner"))
        .expect("owner row");
    assert_eq!(owner.email, owner_email);

    let alice = listed
        .iter()
        .find(|invitation| invitation.handle.as_deref() == Some("alice"))
        .expect("self row");
    assert_eq!(alice.email, alice_email);

    let bob = listed
        .iter()
        .find(|invitation| invitation.handle.as_deref() == Some("bobby"))
        .expect("other participant row");
    assert!(bob.email.is_empty());

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
    let share_token = seed_pending_share_token(&pool, event_id, owner_email, target_email).await?;

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
async fn email_invites_hide_registered_state_and_appear_as_pending_entries()
-> Result<(), Box<dyn Error>> {
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
    let registered_email = "registered@example.com";
    let unregistered_email = "future-guest@example.com";
    let state = build_state(pool.clone(), secret, &[]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let event_id = seed_event(&pool, owner_email).await?;
    let _registered_id = seed_user(&pool, registered_email).await?;

    let _registered_share_token =
        seed_pending_share_token(&pool, event_id, owner_email, registered_email).await?;
    let _unregistered_share_token =
        seed_pending_share_token(&pool, event_id, owner_email, unregistered_email).await?;

    let owner_token = admin_token(secret, owner_email).expect("token");

    let invitations_resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(&format!("/events/{}/invitations", event_id))
            .insert_header(("Authorization", format!("Bearer {}", owner_token)))
            .to_request(),
    )
    .await;
    assert_eq!(invitations_resp.status(), StatusCode::OK);
    let invitations: Vec<Invitation> = test::read_body_json(invitations_resp).await;

    let pending_registered = invitations
        .iter()
        .find(|inv| inv.email.eq_ignore_ascii_case(registered_email))
        .expect("registered email pending entry");
    assert!(pending_registered.user_id.is_none());
    assert!(pending_registered.handle.is_none());

    let pending_unregistered = invitations
        .iter()
        .find(|inv| inv.email.eq_ignore_ascii_case(unregistered_email))
        .expect("unregistered email pending entry");
    assert!(pending_unregistered.user_id.is_none());
    assert!(pending_unregistered.handle.is_none());

    let share_rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM event_share_tokens
         WHERE event_id = $1
           AND target_email_lookup_hash IS NOT NULL
           AND used_at IS NULL",
    )
    .bind(event_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(share_rows, 2);

    let invitation_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM invitations WHERE event_id = $1")
            .bind(event_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(invitation_rows, 0);

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
    sqlx::query(
        "UPDATE event_share_tokens
         SET expires_at = NOW() - INTERVAL '1 hour'
         WHERE token_hash = $1",
    )
    .bind(sha256_hex(share_token))
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
