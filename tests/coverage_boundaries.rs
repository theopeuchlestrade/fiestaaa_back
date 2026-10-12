mod common;

use actix_web::{App, cookie::Cookie, http::StatusCode, test};
use chrono::Utc;
use fiestaaa_back::{
    auth::{self, encode_jwt, hash_password, now_ts},
    models::Claims,
    routes,
    security::sha256_hex,
};
use once_cell::sync::Lazy;
use serde_json::{Value, json};
use sqlx::PgPool;
use std::error::Error;
use uuid::Uuid;

const SECRET: &str = "coverage-fixture-secret";
static PASSWORD_HASH: Lazy<String> = Lazy::new(|| hash_password("CoverageFixture1!").unwrap());
struct User {
    id: i64,
    public_id: Uuid,
    email: String,
    token: String,
}
async fn user(pool: &PgPool, name: &str) -> User {
    let email = format!("{name}@example.invalid");
    let (id,public_id):(i64,Uuid)=sqlx::query_as("INSERT INTO users(email_ciphertext,email_lookup_hash,password_hash,handle) VALUES(fiestaaa_encrypt_text($1),fiestaaa_email_lookup($1),$2,$3) RETURNING id,public_id").bind(&email).bind(&*PASSWORD_HASH).bind(name).fetch_one(pool).await.unwrap();
    let token = encode_jwt(
        &Claims {
            sub: email.clone(),
            handle: name.into(),
            exp: (now_ts() + 3600) as usize,
            session_version: 0,
        },
        SECRET,
    )
    .unwrap();
    User {
        id,
        public_id,
        email,
        token,
    }
}
async fn event(pool: &PgPool, owner: &User) -> i64 {
    sqlx::query_scalar("INSERT INTO events(name_event,description,date_event,start_time,address_ciphertext,owner_user_id) VALUES('Coverage fixture','Fictional','2099-01-01','18:00',fiestaaa_encrypt_text('Fictional place'),$1) RETURNING event_id").bind(owner.id).fetch_one(pool).await.unwrap()
}
macro_rules! call {
    ($app:expr,$method:ident,$uri:expr,$token:expr,$body:expr) => {
        test::call_service(
            &$app,
            test::TestRequest::$method()
                .uri(&$uri)
                .insert_header(("Authorization", format!("Bearer {}", $token)))
                .set_json($body)
                .to_request(),
        )
        .await
    };
}

#[actix_web::test]
async fn recovery_generic_validation_and_suspended_rollback_preserve_private_state()
-> Result<(), Box<dyn Error>> {
    let pool = common::obtain_pool()
        .await
        .expect("isolated database required");
    let _lock = common::DB_LOCK.lock().await;
    common::reset_tables(&pool, &["users"]).await?;
    let qa = user(&pool, "recoveryqa").await;
    let app = test::init_service(
        App::new()
            .app_data(common::build_state(pool.clone(), SECRET, &[]))
            .configure(routes::configure),
    )
    .await;
    for email in [
        "invalid".to_string(),
        "a".repeat(255),
        "missing@example.invalid".to_string(),
    ] {
        let response = call!(
            app,
            post,
            "/auth/password-reset/request",
            "",
            json!({"email":email})
        );
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
        let body: Value = test::read_body_json(response).await;
        assert_eq!(body, json!({"status":"recovery_requested"}));
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM password_resets")
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 0);
    for payload in [
        json!({"token":"short","password":"CoverageFixture1!"}),
        json!({"token":"a".repeat(64),"password":"weak"}),
        json!({"token":"a".repeat(64),"password":"A1!".repeat(400)}),
    ] {
        let response = call!(app, post, "/auth/password-reset/confirm", "", payload);
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body: Value = test::read_body_json(response).await;
        assert_eq!(body["error"], "invalid_reset");
    }
    let token = "b".repeat(64);
    sqlx::query("INSERT INTO password_resets(user_id,token_hash,expires_at) VALUES($1,$2,NOW()+INTERVAL '30 minutes')").bind(qa.id).bind(sha256_hex(&token)).execute(&pool).await?;
    sqlx::query("UPDATE users SET suspended=TRUE WHERE id=$1")
        .bind(qa.id)
        .execute(&pool)
        .await?;
    let response = call!(
        app,
        post,
        "/auth/password-reset/confirm",
        "",
        json!({"token":token,"password":"ChangedFixture2!"})
    );
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let retained: i64 = sqlx::query_scalar("SELECT count(*) FROM password_resets WHERE user_id=$1")
        .bind(qa.id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(retained, 1);
    let (hash, version): (String, i64) =
        sqlx::query_as("SELECT password_hash,session_version FROM users WHERE id=$1")
            .bind(qa.id)
            .fetch_one(&pool)
            .await?;
    assert!(auth::verify_password(&hash, "CoverageFixture1!"));
    assert_eq!(version, 0);
    Ok(())
}

#[actix_web::test]
async fn tokens_cookie_precedence_expiry_wrong_signature_and_logout_are_bounded()
-> Result<(), Box<dyn Error>> {
    let pool = common::obtain_pool()
        .await
        .expect("isolated database required");
    let _lock = common::DB_LOCK.lock().await;
    common::reset_tables(&pool, &["users"]).await?;
    let qa = user(&pool, "authqa").await;
    let app = test::init_service(
        App::new()
            .app_data(common::build_state(pool.clone(), SECRET, &[]))
            .configure(routes::configure),
    )
    .await;
    let expired = encode_jwt(
        &Claims {
            sub: qa.email.clone(),
            handle: "authqa".to_string(),
            exp: (now_ts() - 3600) as usize,
            session_version: 0,
        },
        SECRET,
    )
    .unwrap();
    let wrong = encode_jwt(
        &Claims {
            sub: qa.email.clone(),
            handle: "authqa".to_string(),
            exp: (now_ts() + 3600) as usize,
            session_version: 0,
        },
        "wrong-secret",
    )
    .unwrap();
    for token in ["malformed".to_string(), expired, wrong] {
        let response = call!(app, get, "/me", token, json!(null));
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
    for authorization in [None, Some("Basic fixture"), Some("Bearer ")] {
        let mut request = test::TestRequest::get().uri("/me");
        if let Some(value) = authorization {
            request = request.insert_header(("Authorization", value));
        }
        assert_eq!(
            test::call_service(&app, request.to_request())
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    let response = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/me")
            .cookie(Cookie::new(auth::session_cookie_name(), qa.token.clone()))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let response = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/me")
            .cookie(Cookie::new(auth::session_cookie_name(), qa.token.clone()))
            .insert_header(("Authorization", "Basic invalid"))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    for header in [None, Some("Basic invalid"), Some("Bearer malformed")] {
        let mut req = test::TestRequest::post().uri("/auth/logout");
        if let Some(value) = header {
            req = req.insert_header(("Authorization", value));
        }
        let response = test::call_service(&app, req.to_request()).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().get("set-cookie").is_some());
    }
    let response = call!(app, post, "/auth/logout", qa.token.clone(), json!(null));
    assert_eq!(response.status(), StatusCode::OK);
    let response = call!(app, get, "/me", qa.token, json!(null));
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}

#[actix_web::test]
async fn invitation_lists_and_mutations_keep_pending_outsiders_and_owners_distinct()
-> Result<(), Box<dyn Error>> {
    let pool = common::obtain_pool()
        .await
        .expect("isolated database required");
    let _lock = common::DB_LOCK.lock().await;
    common::reset_tables(&pool, &["users"]).await?;
    let owner = user(&pool, "ownerqa").await;
    let guest = user(&pool, "guestqa").await;
    let pending = user(&pool, "pendingqa").await;
    let outside = user(&pool, "outsideqa").await;
    let id = event(&pool, &owner).await;
    for (u, status) in [(&guest, "Accepted"), (&pending, "Waiting")] {
        sqlx::query("INSERT INTO invitations(event_id,user_id,status) VALUES($1,$2,$3)")
            .bind(id)
            .bind(u.id)
            .bind(status)
            .execute(&pool)
            .await?;
    }
    let app = test::init_service(
        App::new()
            .app_data(common::build_state(pool.clone(), SECRET, &[]))
            .configure(routes::configure),
    )
    .await;
    for (u, expected) in [
        (&owner, StatusCode::OK),
        (&guest, StatusCode::OK),
        (&pending, StatusCode::FORBIDDEN),
        (&outside, StatusCode::FORBIDDEN),
    ] {
        let response = call!(
            app,
            get,
            format!("/events/{id}/invitations?limit=1"),
            &u.token,
            json!(null)
        );
        assert_eq!(response.status(), expected, "list role {}", u.email);
    }
    for u in [&guest, &pending, &outside] {
        let response = call!(
            app,
            post,
            format!("/events/{id}/invitations"),
            &u.token,
            json!({"identifier":"outsideqa"})
        );
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
    for identifier in ["", "bad identifier", "a"] {
        let response = call!(
            app,
            post,
            format!("/events/{id}/invitations"),
            &owner.token,
            json!({"identifier":identifier})
        );
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "identifier {identifier}"
        );
    }
    let response = call!(
        app,
        post,
        format!("/events/{id}/invitations"),
        &owner.token,
        json!({"identifier":"missingqa"})
    );
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let response = call!(
        app,
        post,
        format!("/events/{id}/invitations"),
        &owner.token,
        json!({"identifier":"guestqa"})
    );
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let response = call!(
        app,
        post,
        format!("/events/{id}/invitations"),
        &owner.token,
        json!({"identifier":"outsideqa"})
    );
    assert_eq!(response.status(), StatusCode::CREATED);
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM invitations WHERE event_id=$1")
        .bind(id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(before, 3);
    for status in [
        json!({}),
        json!({"status":"Waiting"}),
        json!({"status":"accepted"}),
    ] {
        let response = call!(
            app,
            patch,
            format!("/my/invitations/{id}"),
            &pending.token,
            status
        );
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
    let response = call!(
        app,
        patch,
        format!("/my/invitations/{id}"),
        &pending.token,
        json!({"status":" Accepted "})
    );
    assert_eq!(response.status(), StatusCode::OK);
    let response = call!(
        app,
        get,
        format!("/events/{id}/invitations?limit=2"),
        &pending.token,
        json!(null)
    );
    assert_eq!(response.status(), StatusCode::OK);
    let response = call!(
        app,
        delete,
        format!("/events/{id}/invitations/{}", guest.email),
        &pending.token,
        json!(null)
    );
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = call!(
        app,
        delete,
        format!("/events/{id}/invitations/missing@example.invalid"),
        &owner.token,
        json!(null)
    );
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let response = call!(
        app,
        delete,
        format!("/events/{id}/invitations/{}", guest.email),
        &owner.token,
        json!(null)
    );
    assert_eq!(response.status(), StatusCode::OK);
    let remaining: i64 =
        sqlx::query_scalar("SELECT count(*) FROM invitations WHERE event_id=$1 AND user_id=$2")
            .bind(id)
            .bind(guest.id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(remaining, 0);
    Ok(())
}

#[actix_web::test]
async fn deadline_expiry_is_enforced_for_invitations_and_keeps_event_visible()
-> Result<(), Box<dyn Error>> {
    let pool = common::obtain_pool()
        .await
        .expect("isolated database required");
    let _lock = common::DB_LOCK.lock().await;
    common::reset_tables(&pool, &["users"]).await?;
    let owner = user(&pool, "ownerqa").await;
    let pending = user(&pool, "pendingqa").await;
    let id = event(&pool, &owner).await;
    sqlx::query("INSERT INTO invitations(event_id,user_id,status) VALUES($1,$2,'Waiting')")
        .bind(id)
        .bind(pending.id)
        .execute(&pool)
        .await?;
    sqlx::query("UPDATE events SET invitation_deadline=$2 WHERE event_id=$1")
        .bind(id)
        .bind(Utc::now().date_naive() - chrono::Duration::days(1))
        .execute(&pool)
        .await?;
    let app = test::init_service(
        App::new()
            .app_data(common::build_state(pool.clone(), SECRET, &[]))
            .configure(routes::configure),
    )
    .await;
    let response = call!(
        app,
        post,
        format!("/events/{id}/invitations"),
        &owner.token,
        json!({"identifier":"pendingqa"})
    );
    assert_eq!(response.status(), StatusCode::GONE);
    let response = call!(
        app,
        get,
        "/my/invitations?limit=1",
        &pending.token,
        json!(null)
    );
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = test::read_body_json(response).await;
    assert_eq!(body[0]["status"], "Expired");
    let response = call!(
        app,
        patch,
        format!("/my/invitations/{id}"),
        &pending.token,
        json!({"status":"Accepted"})
    );
    assert_eq!(response.status(), StatusCode::GONE);
    Ok(())
}

#[actix_web::test]
async fn blocking_missing_self_and_report_validation_do_not_create_private_records()
-> Result<(), Box<dyn Error>> {
    let pool = common::obtain_pool()
        .await
        .expect("isolated database required");
    let _lock = common::DB_LOCK.lock().await;
    common::reset_tables(&pool, &["users"]).await?;
    let qa = user(&pool, "safetyqa").await;
    let app = test::init_service(
        App::new()
            .app_data(common::build_state(pool.clone(), SECRET, &[]))
            .configure(routes::configure),
    )
    .await;
    for (id, expected) in [
        (qa.public_id, StatusCode::BAD_REQUEST),
        (Uuid::new_v4(), StatusCode::NOT_FOUND),
    ] {
        let response = call!(app, post, "/me/blocks", &qa.token, json!({"public_id":id}));
        assert_eq!(response.status(), expected);
    }
    for payload in [
        json!({"reason":"","comment":""}),
        json!({"reason":"spam","comment":"no target"}),
        json!({"reason":"x".repeat(81),"comment":"","public_id":qa.public_id}),
        json!({"reason":"spam","comment":"x".repeat(4001),"public_id":qa.public_id}),
    ] {
        let response = call!(app, post, "/reports", &qa.token, payload);
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM user_blocks")
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 0);
    Ok(())
}

#[path = "common/coverage_http.rs"]
mod coverage_http;

#[actix_web::test]
async fn outbox_rechecks_contact_and_retries_delivery_without_real_push_services()
-> Result<(), Box<dyn Error>> {
    use fiestaaa_back::notifications::{
        NotificationOutboxWorker, NotificationRequest, NotificationService,
        enqueue_notifications_tx,
    };
    let pool = common::obtain_pool()
        .await
        .expect("isolated database required");
    let _lock = common::DB_LOCK.lock().await;
    common::reset_tables(&pool, &["users"]).await?;
    let sender = user(&pool, "senderqa").await;
    let receiver = user(&pool, "receiverqa").await;
    let empty = user(&pool, "emptyqa").await;
    let server = coverage_http::MockServer::start();
    server.response(
        "/legacy",
        503,
        json!({"success":0,"failure":1,"results":[{"error":"Unavailable"}]}),
    );
    let service = NotificationService::new(
        Some("fixture-key".into()),
        None,
        None,
        None,
        fiestaaa_back::build_http_client("coverage-loopback"),
        30,
    )
    .with_test_endpoints(
        format!("{}/legacy", server.base),
        format!("{}/v1", server.base),
    );
    sqlx::query("INSERT INTO user_devices(user_id,fcm_token_ciphertext,fcm_token_lookup_hash,platform) VALUES($1,fiestaaa_encrypt_text('fixture-device'),fiestaaa_lookup_text('fixture-device'),'android')").bind(receiver.id).execute(&pool).await?;
    let request_id:i64=sqlx::query_scalar("INSERT INTO friend_requests(sender_id,receiver_id,status) VALUES($1,$2,'pending') RETURNING id").bind(sender.id).bind(receiver.id).fetch_one(&pool).await?;
    let mut tx = pool.begin().await?;
    enqueue_notifications_tx(
        &mut tx,
        &[receiver.id],
        NotificationRequest {
            title: "Fixture",
            body: "Fictional",
            data: json!({"type":"friend_request","request_id":request_id}),
            dedup_base_key: Some("fixture-blocked"),
            dedup_ttl: Some(30),
        },
    )
    .await?;
    enqueue_notifications_tx(
        &mut tx,
        &[receiver.id],
        NotificationRequest {
            title: "Fixture",
            body: "Fictional",
            data: json!({"type":"event_updated"}),
            dedup_base_key: Some("fixture-retry"),
            dedup_ttl: Some(30),
        },
    )
    .await?;
    enqueue_notifications_tx(
        &mut tx,
        &[empty.id],
        NotificationRequest {
            title: "Fixture",
            body: "Fictional",
            data: json!({"type":"event_updated"}),
            dedup_base_key: Some("fixture-empty"),
            dedup_ttl: None,
        },
    )
    .await?;
    tx.commit().await?;
    sqlx::query("INSERT INTO user_blocks(blocker_id,blocked_id) VALUES($1,$2)")
        .bind(sender.id)
        .bind(receiver.id)
        .execute(&pool)
        .await?;
    let worker = NotificationOutboxWorker::new(pool.clone(), service);
    worker.process_batch().await?;
    let rows: Vec<(String, String, i32, Option<String>)> = sqlx::query_as(
        "SELECT dedup_key,status,attempts,last_error FROM notification_outbox ORDER BY id",
    )
    .fetch_all(&pool)
    .await?;
    assert!(rows.iter().any(|r| r.0.starts_with("fixture-blocked")
        && r.1 == "dead"
        && r.3.as_deref() == Some("contact_unavailable")));
    assert!(
        rows.iter()
            .any(|r| r.0.starts_with("fixture-retry") && r.1 == "retry" && r.2 == 1)
    );
    assert!(
        rows.iter()
            .any(|r| r.0.starts_with("fixture-empty") && r.1 == "sent")
    );
    assert_eq!(
        server.requests().len(),
        1,
        "blocked recipient must not reach delivery"
    );
    sqlx::query(
        "UPDATE notification_outbox SET attempts=7,available_at=NOW() WHERE status='retry'",
    )
    .execute(&pool)
    .await?;
    worker.process_batch().await?;
    let dead:i64=sqlx::query_scalar("SELECT count(*) FROM notification_outbox WHERE dedup_key LIKE 'fixture-retry%' AND status='dead' AND attempts=8").fetch_one(&pool).await?;
    assert_eq!(dead, 1);
    assert_eq!(server.requests().len(), 2);
    server.stop().await;
    Ok(())
}

#[actix_web::test]
async fn device_rotation_validation_rolls_back_and_disabled_tokens_do_not_deliver()
-> Result<(), Box<dyn Error>> {
    use fiestaaa_back::notifications::tokens_by_user_ids;
    let pool = common::obtain_pool()
        .await
        .expect("isolated database required");
    let _lock = common::DB_LOCK.lock().await;
    common::reset_tables(&pool, &["users"]).await?;
    let qa = user(&pool, "deviceqa").await;
    let app = test::init_service(
        App::new()
            .app_data(common::build_state(pool.clone(), SECRET, &[]))
            .configure(routes::configure),
    )
    .await;
    let response = call!(
        app,
        post,
        "/me/devices",
        &qa.token,
        json!({"token":"fixture-old","platform":" IOS ","locale":"fr","app_version":"test"})
    );
    assert_eq!(response.status(), StatusCode::OK);
    for payload in [
        json!({"old_token":"fixture-old","new_token":" ","platform":"ios"}),
        json!({"old_token":"fixture-old","new_token":"fixture-new","platform":"unknown"}),
    ] {
        let response = call!(app, post, "/me/devices/refresh", &qa.token, payload);
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
    let active = tokens_by_user_ids(&pool, &[qa.id]).await?;
    assert_eq!(active[&qa.id], vec!["fixture-old"]);
    let response = call!(
        app,
        post,
        "/me/devices/refresh",
        &qa.token,
        json!({"old_token":"fixture-old","new_token":"fixture-new"})
    );
    assert_eq!(response.status(), StatusCode::OK);
    let active = tokens_by_user_ids(&pool, &[qa.id]).await?;
    assert_eq!(active[&qa.id], vec!["fixture-new"]);
    let platform:String=sqlx::query_scalar("SELECT platform FROM user_devices WHERE fiestaaa_lookup_matches(fcm_token_lookup_hash,'fixture-new')").fetch_one(&pool).await?;
    assert_eq!(platform, "ios");
    let response = call!(
        app,
        post,
        "/me/devices/refresh",
        &qa.token,
        json!({"old_token":"missing","new_token":"fixture-fallback"})
    );
    assert_eq!(response.status(), StatusCode::OK);
    let platform:String=sqlx::query_scalar("SELECT platform FROM user_devices WHERE fiestaaa_lookup_matches(fcm_token_lookup_hash,'fixture-fallback')").fetch_one(&pool).await?;
    assert_eq!(platform, "web");
    assert!(tokens_by_user_ids(&pool, &[]).await?.is_empty());
    Ok(())
}

#[actix_web::test]
async fn qr_generation_expiry_and_revoked_participation_never_check_in_an_invalid_ticket()
-> Result<(), Box<dyn Error>> {
    let pool = common::obtain_pool()
        .await
        .expect("isolated database required");
    let _lock = common::DB_LOCK.lock().await;
    common::reset_tables(&pool, &["users"]).await?;
    let owner = user(&pool, "ownerqa").await;
    let guest = user(&pool, "guestqa").await;
    let outside = user(&pool, "outsideqa").await;
    let id = event(&pool, &owner).await;
    sqlx::query("INSERT INTO invitations(event_id,user_id,status) VALUES($1,$2,'Waiting')")
        .bind(id)
        .bind(guest.id)
        .execute(&pool)
        .await?;
    let app = test::init_service(
        App::new()
            .app_data(common::build_state(pool.clone(), SECRET, &[]))
            .configure(routes::configure),
    )
    .await;
    for u in [&guest, &outside] {
        let response = call!(
            app,
            get,
            format!("/events/{id}/my-qr-code"),
            &u.token,
            json!(null)
        );
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
    sqlx::query("UPDATE invitations SET status='Accepted' WHERE event_id=$1")
        .bind(id)
        .execute(&pool)
        .await?;
    let response = call!(
        app,
        get,
        format!("/events/{id}/my-qr-code"),
        &guest.token,
        json!(null)
    );
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = test::read_body_json(response).await;
    let token = body["qr_token"].as_str().unwrap().to_string();
    let response = call!(
        app,
        post,
        format!("/events/{id}/scan-qr"),
        &guest.token,
        json!({"token":token})
    );
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = call!(
        app,
        post,
        format!("/events/{id}/scan-qr"),
        &owner.token,
        json!({"token":"invalid"})
    );
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let response = call!(
        app,
        post,
        format!("/events/{id}/scan-qr"),
        &owner.token,
        json!({"token":Uuid::new_v4()})
    );
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    sqlx::query(
        "UPDATE event_checkins SET generated_at=NOW()-INTERVAL '10 minutes' WHERE event_id=$1",
    )
    .bind(id)
    .execute(&pool)
    .await?;
    let response = call!(
        app,
        post,
        format!("/events/{id}/scan-qr"),
        &owner.token,
        json!({"token":token})
    );
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body: Value = test::read_body_json(response).await;
    assert_eq!(body["success"], false);
    let response = call!(
        app,
        get,
        format!("/events/{id}/my-qr-code"),
        &guest.token,
        json!(null)
    );
    assert_eq!(response.status(), StatusCode::OK);
    let next: Value = test::read_body_json(response).await;
    assert_ne!(next["qr_token"], token);
    sqlx::query("UPDATE invitations SET status='Declined' WHERE event_id=$1")
        .bind(id)
        .execute(&pool)
        .await?;
    let response = call!(
        app,
        post,
        format!("/events/{id}/scan-qr"),
        &owner.token,
        json!({"token":next["qr_token"]})
    );
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let checked: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM event_checkins WHERE event_id=$1 AND scanned_at IS NOT NULL",
    )
    .bind(id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(checked, 0);
    Ok(())
}

#[actix_web::test]
async fn private_moderation_actions_enforce_roles_and_record_their_effects()
-> Result<(), Box<dyn Error>> {
    let pool = common::obtain_pool()
        .await
        .expect("isolated database required");
    let _lock = common::DB_LOCK.lock().await;
    common::reset_tables(&pool, &["users", "moderation_terms"]).await?;
    let admin = user(&pool, "moderator").await;
    let guest = user(&pool, "reportedguest").await;
    let outsider = user(&pool, "outsideqa").await;
    let event_id = event(&pool, &guest).await;
    let app = test::init_service(
        App::new()
            .app_data(common::build_state(pool.clone(), SECRET, &[&admin.email]))
            .configure(routes::configure),
    )
    .await;
    let response = call!(
        app,
        post,
        "/reports",
        &outsider.token,
        json!({"event_id":event_id,"reason":"spam","comment":"private fixture"})
    );
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = call!(
        app,
        post,
        "/reports",
        &outsider.token,
        json!({"public_id":guest.public_id,"reason":"spam","comment":"private fixture"})
    );
    assert_eq!(response.status(), StatusCode::CREATED);
    let report: Value = test::read_body_json(response).await;
    let response = call!(app, get, "/admin/reports", &guest.token, json!({}));
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = call!(app, get, "/admin/reports", &admin.token, json!({}));
    assert_eq!(response.headers().get("Cache-Control").unwrap(), "no-store");
    let reports: Value = test::read_body_json(response).await;
    assert_eq!(reports[0]["comment"], "private fixture");
    assert_eq!(reports[0]["target"], guest.public_id.to_string());
    let response = call!(
        app,
        post,
        "/admin/moderation",
        &guest.token,
        json!({"action":"suspend","public_id":outsider.public_id})
    );
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    for payload in [
        json!({"action":"unknown"}),
        json!({"action":"hide_avatar"}),
        json!({"action":"suspend"}),
        json!({"action":"add_term","term":"ab"}),
        json!({"action":"add_term","term":"x".repeat(201)}),
    ] {
        assert_eq!(
            call!(app, post, "/admin/moderation", &admin.token, payload).status(),
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        call!(
            app,
            post,
            "/admin/moderation",
            &admin.token,
            json!({"action":"hide_avatar","public_id":Uuid::new_v4()})
        )
        .status(),
        StatusCode::NOT_FOUND
    );
    for action in ["suspend", "unsuspend"] {
        assert_eq!(
            call!(
                app,
                post,
                "/admin/moderation",
                &admin.token,
                json!({"action":action,"public_id":guest.public_id})
            )
            .status(),
            StatusCode::OK
        );
        let (suspended, version): (bool, i64) =
            sqlx::query_as("SELECT suspended,session_version FROM users WHERE id=$1")
                .bind(guest.id)
                .fetch_one(&pool)
                .await?;
        assert_eq!(suspended, action == "suspend");
        assert_eq!(version, if suspended { 1 } else { 2 });
    }
    assert_eq!(
        call!(
            app,
            post,
            "/admin/moderation",
            &admin.token,
            json!({"action":"hide_avatar","public_id":guest.public_id})
        )
        .status(),
        StatusCode::OK
    );
    for action in ["add_term", "add_term", "remove_term"] {
        assert_eq!(
            call!(
                app,
                post,
                "/admin/moderation",
                &admin.token,
                json!({"action":action,"term":"  Fixture forbidden  "})
            )
            .status(),
            StatusCode::OK
        );
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM moderation_terms WHERE term='fixture forbidden'",
        )
        .fetch_one(&pool)
        .await?;
        assert_eq!(count, if action == "remove_term" { 0 } else { 1 });
    }
    for action in ["resolve", "dismiss"] {
        assert_eq!(
            call!(
                app,
                post,
                "/admin/moderation",
                &admin.token,
                json!({"action":action,"public_id":report["public_id"]})
            )
            .status(),
            StatusCode::OK
        );
        let status: String =
            sqlx::query_scalar("SELECT status FROM abuse_reports WHERE public_id=$1")
                .bind(Uuid::parse_str(report["public_id"].as_str().unwrap())?)
                .fetch_one(&pool)
                .await?;
        assert_eq!(
            status,
            if action == "resolve" {
                "resolved"
            } else {
                "dismissed"
            }
        );
    }
    let response = call!(app, get, "/admin/reports", &admin.token, json!({}));
    assert_eq!(test::read_body_json::<Value, _>(response).await, json!([]));
    assert_eq!(
        call!(
            app,
            post,
            "/admin/moderation",
            &admin.token,
            json!({"action":"hide_event","event_id":event_id})
        )
        .status(),
        StatusCode::OK
    );
    let (hidden, reason, purge): (bool, String, Option<chrono::DateTime<Utc>>) = sqlx::query_as(
        "SELECT moderation_hidden,deletion_reason,purge_at FROM events WHERE event_id=$1",
    )
    .bind(event_id)
    .fetch_one(&pool)
    .await?;
    assert!(hidden);
    assert_eq!(reason, "moderation");
    assert!(purge.is_none());
    assert_eq!(
        call!(
            app,
            get,
            "/safety/user?handle=reportedguest",
            &admin.token,
            json!({})
        )
        .status(),
        StatusCode::OK
    );
    assert_eq!(
        call!(
            app,
            get,
            "/safety/user?handle=moderator",
            &admin.token,
            json!({})
        )
        .status(),
        StatusCode::NOT_FOUND
    );
    Ok(())
}

#[actix_web::test]
async fn recovery_delivery_preconditions_remain_generic_and_throttled() -> Result<(), Box<dyn Error>>
{
    let pool = common::obtain_pool()
        .await
        .expect("isolated database required");
    let _lock = common::DB_LOCK.lock().await;
    common::reset_tables(&pool, &["users"]).await?;
    let qa = user(&pool, "deliveryqa").await;
    for mode in 0..3 {
        sqlx::query("DELETE FROM password_resets")
            .execute(&pool)
            .await?;
        let mut shared = common::build_state(pool.clone(), SECRET, &[]).into_inner();
        let state = std::sync::Arc::get_mut(&mut shared).unwrap();
        if mode > 0 {
            state.invitation_email_api_key = Some("re_fixture".into());
        }
        if mode == 2 {
            state.invitation_email_sender = Some("fixture@example.invalid".into());
            state.app_base_url = "invalid URL".into();
        }
        let app = test::init_service(
            App::new()
                .app_data(actix_web::web::Data::from(shared))
                .configure(routes::configure),
        )
        .await;
        assert_eq!(
            call!(
                app,
                post,
                "/auth/password-reset/request",
                "",
                json!({"email":qa.email})
            )
            .status(),
            StatusCode::ACCEPTED
        );
        actix_web::rt::time::sleep(std::time::Duration::from_millis(20)).await;
        let hash: String =
            sqlx::query_scalar("SELECT token_hash FROM password_resets WHERE user_id=$1")
                .bind(qa.id)
                .fetch_one(&pool)
                .await?;
        assert_eq!(
            call!(
                app,
                post,
                "/auth/password-reset/request",
                "",
                json!({"email":qa.email})
            )
            .status(),
            StatusCode::ACCEPTED
        );
        let unchanged: String =
            sqlx::query_scalar("SELECT token_hash FROM password_resets WHERE user_id=$1")
                .bind(qa.id)
                .fetch_one(&pool)
                .await?;
        assert_eq!(hash, unchanged);
    }
    Ok(())
}

#[actix_web::test]
async fn email_invitations_hide_account_existence_and_roll_back_failed_delivery()
-> Result<(), Box<dyn Error>> {
    let pool = common::obtain_pool()
        .await
        .expect("isolated database required");
    let _lock = common::DB_LOCK.lock().await;
    common::reset_tables(&pool, &["users"]).await?;
    let owner = user(&pool, "emailowner").await;
    let guest = user(&pool, "emailguest").await;
    let id = event(&pool, &owner).await;
    sqlx::query(
        "INSERT INTO invitations(event_id,user_id,status,date_invi) VALUES($1,$2,'Waiting',NOW())",
    )
    .bind(id)
    .bind(guest.id)
    .execute(&pool)
    .await?;
    let app = test::init_service(
        App::new()
            .app_data(common::build_state(pool.clone(), SECRET, &[]))
            .configure(routes::configure),
    )
    .await;
    for email in [&owner.email, &guest.email] {
        let response = call!(
            app,
            post,
            format!("/events/{id}/invitations"),
            &owner.token,
            json!({"identifier":email})
        );
        assert_eq!(response.status(), StatusCode::ACCEPTED);
    }
    let response = call!(
        app,
        post,
        format!("/events/{id}/invitations"),
        &owner.token,
        json!({"identifier":"missing@example.invalid"})
    );
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM event_share_tokens WHERE event_id=$1")
            .bind(id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(count, 0);
    sqlx::query("INSERT INTO event_share_tokens(token_hash,event_id,created_by_user_id,target_email_ciphertext,target_email_lookup_hash,expires_at) VALUES($1,$2,$3,fiestaaa_encrypt_text('pending@example.invalid'),fiestaaa_email_lookup('pending@example.invalid'),NOW()+INTERVAL '7 days')").bind(sha256_hex("fictional-token")).bind(id).bind(owner.id).execute(&pool).await?;
    assert_eq!(
        call!(
            app,
            post,
            format!("/events/{id}/invitations"),
            &owner.token,
            json!({"identifier":"pending@example.invalid"})
        )
        .status(),
        StatusCode::ACCEPTED
    );
    sqlx::query("INSERT INTO user_blocks(blocker_id,blocked_id) VALUES($1,$2)")
        .bind(owner.id)
        .bind(guest.id)
        .execute(&pool)
        .await?;
    assert_eq!(
        call!(
            app,
            post,
            format!("/events/{id}/invitations"),
            &owner.token,
            json!({"identifier":guest.email})
        )
        .status(),
        StatusCode::ACCEPTED
    );
    assert_eq!(
        call!(
            app,
            post,
            format!("/events/{id}/invitations"),
            &owner.token,
            json!({"identifier":"emailguest"})
        )
        .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call!(
            app,
            post,
            format!("/events/{id}/invitations"),
            &guest.token,
            json!({"identifier":"pending@example.invalid"})
        )
        .status(),
        StatusCode::FORBIDDEN
    );
    Ok(())
}

#[actix_web::test]
async fn provider_transport_failure_cleans_email_invitation_but_keeps_reset_eligible()
-> Result<(), Box<dyn Error>> {
    let pool = common::obtain_pool()
        .await
        .expect("isolated database required");
    let _lock = common::DB_LOCK.lock().await;
    common::reset_tables(&pool, &["users"]).await?;
    let qa = user(&pool, "mailtransportqa").await;
    let id = event(&pool, &qa).await;
    let server = coverage_http::MockServer::start();
    let mut shared = common::build_state(pool.clone(), SECRET, &[]).into_inner();
    let state = std::sync::Arc::get_mut(&mut shared).unwrap();
    state.invitation_email_api_key = Some("re_fixture".into());
    state.invitation_email_sender = Some("qa@example.invalid".into());
    // All HTTPS connections terminate at this loopback proxy. CONNECT is rejected;
    // the provider is never contacted and fixture credentials never leave the host.
    state.http_client = reqwest::Client::builder()
        .proxy(reqwest::Proxy::all(&server.base)?)
        .timeout(std::time::Duration::from_secs(1))
        .build()?;
    let app = test::init_service(
        App::new()
            .app_data(actix_web::web::Data::from(shared))
            .configure(routes::configure),
    )
    .await;
    assert_eq!(
        call!(
            app,
            post,
            format!("/events/{id}/invitations"),
            &qa.token,
            json!({"identifier":"mailmissing@example.invalid"})
        )
        .status(),
        StatusCode::BAD_GATEWAY
    );
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM event_share_tokens WHERE event_id=$1")
            .bind(id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(count, 0);
    assert_eq!(
        call!(
            app,
            post,
            "/auth/password-reset/request",
            "",
            json!({"email":qa.email})
        )
        .status(),
        StatusCode::ACCEPTED
    );
    actix_web::rt::time::sleep(std::time::Duration::from_millis(200)).await;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM password_resets WHERE user_id=$1")
        .bind(qa.id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 1);
    server.stop().await;
    Ok(())
}

#[actix_web::test]
async fn websocket_identity_checks_origin_purpose_expiry_version_and_membership()
-> Result<(), Box<dyn Error>> {
    let pool = common::obtain_pool()
        .await
        .expect("isolated database required");
    let _lock = common::DB_LOCK.lock().await;
    common::reset_tables(&pool, &["users"]).await?;
    let owner = user(&pool, "socketowner").await;
    let pending = user(&pool, "socketpending").await;
    let id = event(&pool, &owner).await;
    let app = test::init_service(
        App::new()
            .app_data(common::build_state(pool.clone(), SECRET, &[]))
            .configure(routes::configure),
    )
    .await;
    assert_eq!(
        call!(app, get, "/ws", "", json!({})).status(),
        StatusCode::UNAUTHORIZED
    );
    for payload in [
        json!({"sub":owner.email,"exp":now_ts()+60,"event_id":null,"purpose":"wrong","nonce":"fixture"}),
        json!({"sub":owner.email,"exp":now_ts()-120,"event_id":null,"purpose":"realtime","nonce":"fixture"}),
        json!({"sub":owner.email,"exp":now_ts()+60,"session_version":99,"event_id":null,"purpose":"realtime","nonce":"fixture"}),
        json!({"sub":pending.email,"exp":now_ts()+60,"event_id":id,"purpose":"realtime","nonce":"fixture"}),
    ] {
        let ticket = jsonwebtoken::encode(
            &jsonwebtoken::Header::default(),
            &payload,
            &jsonwebtoken::EncodingKey::from_secret(SECRET.as_bytes()),
        )?;
        let response = call!(app, get, format!("/ws?ticket={ticket}"), "", json!({}));
        assert!(matches!(
            response.status(),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ));
    }
    assert_eq!(
        call!(app, get, "/ws?ticket=malformed", "", json!({})).status(),
        StatusCode::UNAUTHORIZED
    );
    let response = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/ws")
            .insert_header(("Origin", "https://outside.invalid"))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    for event_id in [None, Some(id)] {
        let url = event_id
            .map(|value| format!("/ws-ticket?event_id={value}"))
            .unwrap_or("/ws-ticket".into());
        let response = call!(app, get, url, &owner.token, json!({}));
        assert_eq!(response.status(), StatusCode::OK);
        let body: Value = test::read_body_json(response).await;
        let ticket = body["ticket"].as_str().unwrap();
        let response = test::call_service(
            &app,
            test::TestRequest::get()
                .uri(&format!("/ws?ticket={ticket}"))
                .insert_header(("Upgrade", "websocket"))
                .insert_header(("Connection", "upgrade"))
                .insert_header(("Sec-WebSocket-Version", "13"))
                .insert_header(("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ=="))
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
    }
    Ok(())
}

#[actix_web::test]
async fn friendship_validation_response_roles_and_pagination_are_explicit()
-> Result<(), Box<dyn Error>> {
    let pool = common::obtain_pool()
        .await
        .expect("isolated database required");
    let _lock = common::DB_LOCK.lock().await;
    common::reset_tables(&pool, &["users"]).await?;
    let alice = user(&pool, "friendalice").await;
    let bob = user(&pool, "friendbob").await;
    let app = test::init_service(
        App::new()
            .app_data(common::build_state(pool.clone(), SECRET, &[]))
            .configure(routes::configure),
    )
    .await;
    for (identifier, status) in [
        ("", StatusCode::BAD_REQUEST),
        ("bad handle!", StatusCode::BAD_REQUEST),
        ("missinghandle", StatusCode::NOT_FOUND),
        ("missing@example.invalid", StatusCode::NOT_FOUND),
        ("friendalice", StatusCode::BAD_REQUEST),
    ] {
        assert_eq!(
            call!(
                app,
                post,
                "/friends/requests",
                &alice.token,
                json!({"identifier":identifier})
            )
            .status(),
            status
        );
    }
    let response = call!(app, get, "/friends/search?q=a", &alice.token, json!({}));
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(test::read_body_json::<Value, _>(response).await, json!([]));
    let response = call!(
        app,
        post,
        "/friends/requests",
        &alice.token,
        json!({"identifier":"friendbob"})
    );
    assert_eq!(response.status(), StatusCode::CREATED);
    let body: Value = test::read_body_json(response).await;
    let id = body["id"].as_i64().unwrap();
    for (who, status, expected) in [
        (&alice, "Accepted", StatusCode::FORBIDDEN),
        (&bob, "Waiting", StatusCode::BAD_REQUEST),
        (&bob, "Declined", StatusCode::OK),
        (&bob, "Accepted", StatusCode::CONFLICT),
    ] {
        assert_eq!(
            call!(
                app,
                patch,
                format!("/friends/requests/{id}"),
                &who.token,
                json!({"status":status})
            )
            .status(),
            expected
        );
    }
    assert_eq!(
        call!(
            app,
            patch,
            "/friends/requests/99999",
            &bob.token,
            json!({"status":"Accepted"})
        )
        .status(),
        StatusCode::NOT_FOUND
    );
    for uri in [
        "/me/friends?limit=1",
        "/friends/requests?limit=1",
        "/me/friends?limit=1&cursor=0",
        "/friends/requests?limit=1&cursor=0",
    ] {
        assert_eq!(
            call!(app, get, uri, &alice.token, json!({})).status(),
            StatusCode::OK
        );
    }
    Ok(())
}

#[actix_web::test]
async fn registration_verification_expiry_collision_and_delivery_failure_preserve_state()
-> Result<(), Box<dyn Error>> {
    let pool = common::obtain_pool()
        .await
        .expect("isolated database required");
    let _lock = common::DB_LOCK.lock().await;
    common::reset_tables(&pool, &["users", "pending_registrations"]).await?;
    let existing = user(&pool, "registeredqa").await;
    let server = coverage_http::MockServer::start();
    let mut shared = common::build_state(pool.clone(), SECRET, &[]).into_inner();
    let state = std::sync::Arc::get_mut(&mut shared).unwrap();
    state.invitation_email_sender = Some("fixture@example.invalid".into());
    state.invitation_email_api_key = Some("re_fixture".into());
    state.app_base_url = "https://fixture.invalid/link?source=qa".into();
    state.http_client = reqwest::Client::builder()
        .proxy(reqwest::Proxy::all(&server.base)?)
        .timeout(std::time::Duration::from_secs(1))
        .build()?;
    let app = test::init_service(
        App::new()
            .app_data(actix_web::web::Data::from(shared))
            .configure(routes::configure),
    )
    .await;
    assert_eq!(
        call!(
            app,
            post,
            "/auth/register",
            "",
            json!({"email":"signup@example.invalid"})
        )
        .status(),
        StatusCode::BAD_GATEWAY
    );
    let token:String=sqlx::query_scalar("SELECT fiestaaa_decrypt_text(verification_token_ciphertext) FROM pending_registrations WHERE fiestaaa_email_matches(email_lookup_hash,'signup@example.invalid')").fetch_one(&pool).await?;
    for token in [
        "".to_string(),
        "invalid".to_string(),
        Uuid::new_v4().to_string(),
    ] {
        for uri in ["/auth/verify-email", "/auth/complete-registration"] {
            assert_eq!(
                call!(
                    app,
                    post,
                    uri,
                    "",
                    json!({"token":token,"password":"SignupFixture1!","handle":"availableqa"})
                )
                .status(),
                StatusCode::BAD_REQUEST
            );
        }
    }
    for (handle, password, status) in [
        ("bad handle!", "SignupFixture1!", StatusCode::BAD_REQUEST),
        ("registeredqa", "SignupFixture1!", StatusCode::CONFLICT),
        ("availableqa", "weak", StatusCode::BAD_REQUEST),
    ] {
        assert_eq!(
            call!(
                app,
                post,
                "/auth/complete-registration",
                "",
                json!({"token":token,"handle":handle,"password":password})
            )
            .status(),
            status
        );
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM pending_registrations")
            .fetch_one(&pool)
            .await?;
        assert_eq!(count, 1);
    }
    sqlx::query("UPDATE pending_registrations SET verification_expires_at=NOW()-INTERVAL '1 hour'")
        .execute(&pool)
        .await?;
    assert_eq!(
        call!(
            app,
            post,
            "/auth/complete-registration",
            "",
            json!({"token":token,"handle":"availableqa","password":"SignupFixture1!"})
        )
        .status(),
        StatusCode::GONE
    );
    for mode in ["expired", "registered", "complete-conflict"] {
        let token = Uuid::new_v4().to_string();
        sqlx::query("INSERT INTO pending_registrations(email_ciphertext,email_lookup_hash,password_hash,handle,verification_token_hash,verification_expires_at) VALUES(fiestaaa_encrypt_text($1),fiestaaa_email_lookup($1),$2,$3,$4,$5)").bind(&existing.email).bind(&*PASSWORD_HASH).bind(format!("pending-{mode}")).bind(sha256_hex(&token)).bind(if mode=="expired" {Utc::now()-chrono::Duration::hours(1)}else{Utc::now()+chrono::Duration::hours(1)}).execute(&pool).await?;
        let uri = if mode == "complete-conflict" {
            "/auth/complete-registration"
        } else {
            "/auth/verify-email"
        };
        let response = call!(
            app,
            post,
            uri,
            "",
            json!({"token":token,"password":"SignupFixture1!","handle":"availableqa"})
        );
        assert_eq!(
            response.status(),
            match mode {
                "expired" => StatusCode::GONE,
                "registered" => StatusCode::OK,
                _ => StatusCode::CONFLICT,
            }
        );
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM pending_registrations")
            .fetch_one(&pool)
            .await?;
        assert_eq!(count, 0);
    }
    server.stop().await;
    Ok(())
}

#[actix_web::test]
async fn read_only_database_refuses_mutations_without_partial_relationships()
-> Result<(), Box<dyn Error>> {
    let pool = common::obtain_pool()
        .await
        .expect("isolated database required");
    let _lock = common::DB_LOCK.lock().await;
    common::reset_tables(&pool, &["users"]).await?;
    let owner = user(&pool, "readonlyowner").await;
    let guest = user(&pool, "readonlyguest").await;
    let event_id = event(&pool, &owner).await;
    sqlx::query("INSERT INTO invitations(event_id,user_id,status) VALUES($1,$2,'Accepted')")
        .bind(event_id)
        .bind(guest.id)
        .execute(&pool)
        .await?;
    let request_id:i64=sqlx::query_scalar("INSERT INTO friend_requests(sender_id,receiver_id,status) VALUES($1,$2,'Pending') RETURNING id").bind(owner.id).bind(guest.id).fetch_one(&pool).await?;
    sqlx::query("INSERT INTO friendships(user_a,user_b) VALUES($1,$2)")
        .bind(owner.id.min(guest.id))
        .bind(owner.id.max(guest.id))
        .execute(&pool)
        .await?;
    let readonly=sqlx::postgres::PgPoolOptions::new().max_connections(2).after_connect(|connection,_|Box::pin(async move {
        sqlx::query("SELECT set_config('fiestaaa.data_encryption_key','test-data-encryption-key-32-chars!!',false),set_config('fiestaaa.data_lookup_key','test-data-lookup-key-32-chars!!!!!!',false),set_config('default_transaction_read_only','on',false)").execute(connection).await?;Ok(())
    })).connect(&std::env::var("TEST_DATABASE_URL")?).await?;
    let app = test::init_service(
        App::new()
            .app_data(common::build_state(
                readonly.clone(),
                SECRET,
                &[&owner.email],
            ))
            .configure(routes::configure),
    )
    .await;
    for identifier in ["readonlyguest", "newreadonly@example.invalid"] {
        assert_eq!(
            call!(
                app,
                post,
                format!("/events/{event_id}/invitations"),
                &owner.token,
                json!({"identifier":identifier})
            )
            .status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }
    assert_eq!(
        call!(
            app,
            delete,
            format!("/events/{event_id}/invitations/{}", guest.email),
            &owner.token,
            json!({})
        )
        .status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(
        call!(
            app,
            patch,
            format!("/friends/requests/{request_id}"),
            &guest.token,
            json!({"status":"Accepted"})
        )
        .status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(
        call!(
            app,
            post,
            "/me/blocks",
            &owner.token,
            json!({"public_id":guest.public_id})
        )
        .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        call!(
            app,
            delete,
            format!("/me/blocks/{}", guest.public_id),
            &owner.token,
            json!({})
        )
        .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        call!(
            app,
            post,
            "/reports",
            &owner.token,
            json!({"public_id":guest.public_id,"reason":"spam","comment":"fixture"})
        )
        .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        call!(
            app,
            post,
            "/admin/moderation",
            &owner.token,
            json!({"action":"suspend","public_id":guest.public_id})
        )
        .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        call!(
            app,
            post,
            "/admin/moderation",
            &owner.token,
            json!({"action":"hide_avatar","public_id":guest.public_id})
        )
        .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    let status: String = sqlx::query_scalar("SELECT status FROM friend_requests WHERE id=$1")
        .bind(request_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(status, "Pending");
    let active: bool = sqlx::query_scalar("SELECT NOT suspended FROM users WHERE id=$1")
        .bind(guest.id)
        .fetch_one(&pool)
        .await?;
    assert!(active);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM invitations WHERE event_id=$1")
        .bind(event_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 1);
    readonly.close().await;
    Ok(())
}

#[actix_web::test]
async fn apple_android_callback_is_fixed_bounded_and_never_creates_a_session() {
    let app = test::init_service(App::new().service(fiestaaa_back::apple::android_callback)).await;
    for form in [
        std::collections::HashMap::new(),
        std::collections::HashMap::from([("state", "x".repeat(16385))]),
        std::collections::HashMap::from([
            ("state", "fixture".into()),
            ("a", "1".into()),
            ("b", "1".into()),
            ("c", "1".into()),
            ("d", "1".into()),
            ("e", "1".into()),
            ("f", "1".into()),
        ]),
    ] {
        let response = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/auth/apple/android-callback")
                .set_form(&form)
                .to_request(),
        )
        .await;
        assert!(matches!(
            response.status(),
            StatusCode::BAD_REQUEST | StatusCode::PAYLOAD_TOO_LARGE
        ));
    }
    let form = std::collections::HashMap::from([
        ("state", "fixture & state"),
        ("code", "fixture-code"),
        ("id_token", "fixture-token"),
        ("error_description", "fixture error"),
        ("redirect", "https://outside.invalid"),
    ]);
    let response = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/apple/android-callback")
            .set_form(&form)
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers().get("Cache-Control").unwrap(), "no-store");
    assert_eq!(
        response.headers().get("Referrer-Policy").unwrap(),
        "no-referrer"
    );
    assert!(response.headers().get("Set-Cookie").is_none());
    let target = response
        .headers()
        .get("Location")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(target.starts_with("intent://callback?"));
    assert!(target.ends_with("#Intent;package=com.fiestaaa.fiestaaa;scheme=signinwithapple;end"));
    assert!(!target.contains("outside.invalid"));
    assert!(target.contains("state=fixture+%26+state"));
}

#[actix_web::test]
async fn registration_limits_identity_across_distinct_trusted_remotes() -> Result<(), Box<dyn Error>>
{
    let pool = common::obtain_pool()
        .await
        .expect("isolated database required");
    let _lock = common::DB_LOCK.lock().await;
    common::reset_tables(&pool, &["users", "pending_registrations"]).await?;
    let mut shared = common::build_state(pool.clone(), SECRET, &[]).into_inner();
    let state = std::sync::Arc::get_mut(&mut shared).unwrap();
    state.trust_proxy_headers = true;
    state.invitation_email_sender = Some("fixture@example.invalid".into());
    state.auth_rate_limiter = fiestaaa_back::rate_limit::AuthRateLimiter::new(
        1,
        std::time::Duration::from_secs(60),
        None,
    );
    let app = test::init_service(
        App::new()
            .app_data(actix_web::web::Data::from(shared))
            .configure(routes::configure),
    )
    .await;
    for (remote, status) in [
        ("192.0.2.1", StatusCode::CREATED),
        ("192.0.2.2", StatusCode::TOO_MANY_REQUESTS),
    ] {
        let response = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/auth/register")
                .insert_header(("X-Forwarded-For", remote))
                .set_json(json!({"email":"limited@example.invalid"}))
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), status);
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM pending_registrations")
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 1);
    Ok(())
}

#[actix_web::test]
async fn declining_participation_releases_contributions_driver_and_passenger_resources()
-> Result<(), Box<dyn Error>> {
    let pool = common::obtain_pool()
        .await
        .expect("isolated database required");
    let _lock = common::DB_LOCK.lock().await;
    common::reset_tables(&pool, &["users", "item_types"]).await?;
    let owner = user(&pool, "declineowner").await;
    let driver = user(&pool, "declinedriver").await;
    let passenger = user(&pool, "declinepassenger").await;
    let rider = user(&pool, "declinerider").await;
    let id = event(&pool, &owner).await;
    for user in [&driver, &passenger, &rider] {
        sqlx::query("INSERT INTO invitations(event_id,user_id,status) VALUES($1,$2,'Accepted')")
            .bind(id)
            .bind(user.id)
            .execute(&pool)
            .await?;
    }
    let item:i64=sqlx::query_scalar("WITH kind AS (INSERT INTO item_types(type) VALUES('Fixture food') RETURNING type_id) INSERT INTO items(type_id,name_item,max_quantity) SELECT type_id,'Fixture need',6 FROM kind RETURNING item_id").fetch_one(&pool).await?;
    sqlx::query("INSERT INTO events_items(event_id,item_id,max_quantity,quantity,created_by) VALUES($1,$2,6,2,$3)").bind(id).bind(item).bind(owner.id).execute(&pool).await?;
    sqlx::query("INSERT INTO user_items(user_id,event_id,item_id,quantity) VALUES($1,$2,$3,2)")
        .bind(driver.id)
        .bind(id)
        .bind(item)
        .execute(&pool)
        .await?;
    let owned:i64=sqlx::query_scalar("INSERT INTO carpools(event_id,driver_id,origin_ciphertext,depart_at,seats_total,seats_taken) VALUES($1,$2,fiestaaa_encrypt_text('Fictional station'),'2099-01-01 18:00Z',3,1) RETURNING carpool_id").bind(id).bind(driver.id).fetch_one(&pool).await?;
    let other:i64=sqlx::query_scalar("INSERT INTO carpools(event_id,driver_id,origin_ciphertext,depart_at,seats_total,seats_taken) VALUES($1,$2,fiestaaa_encrypt_text('Fictional station'),'2099-01-01 18:00Z',3,1) RETURNING carpool_id").bind(id).bind(owner.id).fetch_one(&pool).await?;
    for (carpool, user_id) in [(owned, rider.id), (other, passenger.id)] {
        sqlx::query("INSERT INTO carpool_passengers(carpool_id,user_id,event_id) VALUES($1,$2,$3)")
            .bind(carpool)
            .bind(user_id)
            .bind(id)
            .execute(&pool)
            .await?;
    }
    let app = test::init_service(
        App::new()
            .app_data(common::build_state(pool.clone(), SECRET, &[]))
            .configure(routes::configure),
    )
    .await;
    assert_eq!(
        call!(
            app,
            patch,
            format!("/my/invitations/{id}"),
            &driver.token,
            json!({"status":"Declined"})
        )
        .status(),
        StatusCode::OK
    );
    let quantity: i32 =
        sqlx::query_scalar("SELECT quantity FROM events_items WHERE event_id=$1 AND item_id=$2")
            .bind(id)
            .bind(item)
            .fetch_one(&pool)
            .await?;
    assert_eq!(quantity, 0);
    let removed: i64 = sqlx::query_scalar("SELECT count(*) FROM carpools WHERE carpool_id=$1")
        .bind(owned)
        .fetch_one(&pool)
        .await?;
    assert_eq!(removed, 0);
    assert_eq!(
        call!(
            app,
            patch,
            format!("/my/invitations/{id}"),
            &passenger.token,
            json!({"status":"Declined"})
        )
        .status(),
        StatusCode::OK
    );
    let seats: i32 = sqlx::query_scalar("SELECT seats_taken FROM carpools WHERE carpool_id=$1")
        .bind(other)
        .fetch_one(&pool)
        .await?;
    assert_eq!(seats, 0);
    let promises: i64 = sqlx::query_scalar("SELECT count(*) FROM user_items WHERE event_id=$1")
        .bind(id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(promises, 0);
    assert_eq!(
        call!(
            app,
            get,
            format!("/events/{id}/items"),
            &driver.token,
            json!({})
        )
        .status(),
        StatusCode::FORBIDDEN
    );
    Ok(())
}
