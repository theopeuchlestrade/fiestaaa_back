mod common;
use actix_web::{App, http::StatusCode, test};
use fiestaaa_back::{
    auth::{encode_jwt, hash_password, now_ts, verify_password},
    models::Claims,
    routes,
    security::sha256_hex,
};
use serde_json::json;
use uuid::Uuid;
async fn seed(pool: &sqlx::PgPool, email: &str, handle: &str) -> (i64, Uuid, String) {
    let (id,public):(i64,Uuid)=sqlx::query_as("INSERT INTO users(email_ciphertext,email_lookup_hash,password_hash,handle) VALUES(fiestaaa_encrypt_text($1),fiestaaa_email_lookup($1),$2,$3) RETURNING id,public_id").bind(email).bind(hash_password("OriginalPassword1!").unwrap()).bind(handle).fetch_one(pool).await.unwrap();
    let token = encode_jwt(
        &Claims {
            sub: public.to_string(),
            handle: handle.into(),
            exp: (now_ts() + 3600) as usize,
            session_version: 0,
        },
        "test-secret",
    )
    .unwrap();
    (id, public, token)
}
#[actix_web::test]
async fn recovery_consumes_once_and_revokes_all_sessions() {
    let Some(pool) = common::obtain_pool().await else {
        return;
    };
    let _lock = common::DB_LOCK.lock().await;
    common::reset_tables(&pool, &["users"]).await.unwrap();
    let (id, _, token) = seed(&pool, "reset@example.test", "reset_test").await;
    let app = test::init_service(
        App::new()
            .app_data(common::build_state(pool.clone(), "test-secret", &[]))
            .configure(routes::configure),
    )
    .await;
    for email in ["reset@example.test", "missing@example.test"] {
        let response = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/auth/password-reset/request")
                .set_json(json!({"email":email}))
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        assert_eq!(
            test::read_body_json::<serde_json::Value, _>(response).await,
            json!({"status":"recovery_requested"})
        );
    }
    let raw = "a".repeat(64);
    sqlx::query("UPDATE password_resets SET token_hash=$1,expires_at=NOW()+INTERVAL '30 minutes' WHERE user_id=$2").bind(sha256_hex(&raw)).bind(id).execute(&pool).await.unwrap();
    for expected in [StatusCode::OK, StatusCode::BAD_REQUEST] {
        let response = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/auth/password-reset/confirm")
                .set_json(json!({"token":raw,"password":"ChangedPassword2!"}))
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), expected);
    }
    let response = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/me")
            .insert_header(("Authorization", format!("Bearer {token}")))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let hash: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id=$1")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(verify_password(&hash, "ChangedPassword2!"));
    assert!(!verify_password(&hash, "OriginalPassword1!"));
    let response = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/login")
            .insert_header(("X-Fiestaaa-Auth-Response", "bearer"))
            .set_json(json!({"identifier":"reset@example.test","password":"ChangedPassword2!"}))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let value: serde_json::Value = test::read_body_json(response).await;
    let response = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/me")
            .insert_header((
                "Authorization",
                format!("Bearer {}", value["token"].as_str().unwrap()),
            ))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
}
#[actix_web::test]
async fn expired_and_social_only_recovery_are_rejected() {
    let Some(pool) = common::obtain_pool().await else {
        return;
    };
    let _lock = common::DB_LOCK.lock().await;
    common::reset_tables(&pool, &["users"]).await.unwrap();
    let (id, _, _) = seed(&pool, "social@example.test", "social_test").await;
    sqlx::query("UPDATE users SET password_login_enabled=FALSE WHERE id=$1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    let app = test::init_service(
        App::new()
            .app_data(common::build_state(pool.clone(), "test-secret", &[]))
            .configure(routes::configure),
    )
    .await;
    let response = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/password-reset/request")
            .set_json(json!({"email":"social@example.test"}))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM password_resets")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
    let raw = "b".repeat(64);
    sqlx::query("INSERT INTO password_resets(user_id,token_hash,expires_at) VALUES($1,$2,NOW()-INTERVAL '1 minute')").bind(id).bind(sha256_hex(&raw)).execute(&pool).await.unwrap();
    let response = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/password-reset/confirm")
            .set_json(json!({"token":raw,"password":"ChangedPassword2!"}))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
#[actix_web::test]
async fn block_cancels_friendship_and_denies_both_directions_and_private_reports() {
    let Some(pool) = common::obtain_pool().await else {
        return;
    };
    let _lock = common::DB_LOCK.lock().await;
    common::reset_tables(&pool, &["users"]).await.unwrap();
    let (a, _, ta) = seed(&pool, "a@example.test", "alice_test").await;
    let (b, pb, tb) = seed(&pool, "b@example.test", "bob_test").await;
    sqlx::query("INSERT INTO friendships(user_a,user_b) VALUES($1,$2)")
        .bind(a)
        .bind(b)
        .execute(&pool)
        .await
        .unwrap();
    let event:i64=sqlx::query_scalar("INSERT INTO events(name_event,description,owner_user_id,date_event,start_time,address_ciphertext) VALUES('Shared','test',$1,CURRENT_DATE,'12:00',fiestaaa_encrypt_text('test')) RETURNING event_id").bind(a).fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO invitations(event_id,user_id,status) VALUES($1,$2,'Accepted')")
        .bind(event)
        .bind(b)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO event_share_tokens(token_hash,event_id,expires_at,target_email_lookup_hash) SELECT 'pending', $1,NOW()+INTERVAL '1 day',email_lookup_hash FROM users WHERE id=$2").bind(event).bind(b).execute(&pool).await.unwrap();
    let app = test::init_service(
        App::new()
            .app_data(common::build_state(pool.clone(), "test-secret", &[]))
            .configure(routes::configure),
    )
    .await;
    let response = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/me/blocks")
            .insert_header(("Authorization", format!("Bearer {ta}")))
            .set_json(json!({"public_id":pb}))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM friendships")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
    let accepted: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM invitations WHERE event_id=$1 AND user_id=$2 AND status='Accepted'",
    )
    .bind(event)
    .bind(b)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        accepted, 1,
        "Shared participation must remain after blocking"
    );
    let links: i64 =
        sqlx::query_scalar("SELECT count(*) FROM event_share_tokens WHERE event_id=$1")
            .bind(event)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(links, 0, "Pending targeted links must be cancelled");
    assert!(sqlx::query("INSERT INTO event_share_tokens(token_hash,event_id,expires_at,target_email_lookup_hash) SELECT 'new-targeted', $1,NOW()+INTERVAL '1 day',email_lookup_hash FROM users WHERE id=$2").bind(event).bind(b).execute(&pool).await.is_err());
    assert!(
        sqlx::query("UPDATE invitations SET status='Waiting' WHERE event_id=$1 AND user_id=$2")
            .bind(event)
            .bind(b)
            .execute(&pool)
            .await
            .is_err()
    );
    for (token, target) in [(&ta, "bob_test"), (&tb, "alice_test")] {
        let response = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/friends/requests")
                .insert_header(("Authorization", format!("Bearer {token}")))
                .set_json(json!({"identifier":target}))
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
    let response = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/reports")
            .insert_header(("Authorization", format!("Bearer {ta}")))
            .set_json(json!({"public_id":pb,"reason":"harassment","comment":"Private report"}))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let response = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/admin/reports")
            .insert_header(("Authorization", format!("Bearer {tb}")))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/reports")
            .insert_header(("Authorization", format!("Bearer {ta}")))
            .set_json(json!({"event_id":1234567,"reason":"spam","comment":""}))
            .to_request(),
    )
    .await;
    assert!(!response.status().is_success());
}
#[actix_web::test]
async fn apple_deletion_queues_revocation_without_network_dependency() {
    let Some(pool) = common::obtain_pool().await else {
        return;
    };
    let _lock = common::DB_LOCK.lock().await;
    common::reset_tables(&pool, &["users", "apple_revocations"])
        .await
        .unwrap();
    let (id, _, token) = seed(&pool, "apple@example.test", "apple_test").await;
    sqlx::query("INSERT INTO oauth_identities(provider,provider_subject_ciphertext,provider_subject_lookup_hash,user_id) VALUES('apple',fiestaaa_encrypt_text('subject'),fiestaaa_lookup_text('subject'),$1)").bind(id).execute(&pool).await.unwrap();
    let state = common::build_state(pool.clone(), "test-secret", &[]);
    let app = test::init_service(
        App::new()
            .app_data(state.clone())
            .configure(routes::configure),
    )
    .await;
    let response = test::call_service(
        &app,
        test::TestRequest::delete()
            .uri("/me")
            .insert_header(("Authorization", format!("Bearer {token}")))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    sqlx::query("INSERT INTO apple_credentials VALUES($1,'com.fiestaaa.fiestaaa',fiestaaa_encrypt_text('test-refresh'))").bind(id).execute(&pool).await.unwrap();
    let response = test::call_service(
        &app,
        test::TestRequest::delete()
            .uri("/me")
            .insert_header(("Authorization", format!("Bearer {token}")))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    fiestaaa_back::apple::revoke_next(&state).await.unwrap();
    let attempts: i32 = sqlx::query_scalar("SELECT attempts FROM apple_revocations")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(attempts, 1);
    let users: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(users, 0);
}

#[actix_web::test]
async fn recovery_rate_limit_is_applied_before_account_lookup() {
    let Some(pool) = common::obtain_pool().await else {
        return;
    };
    let _lock = common::DB_LOCK.lock().await;
    let state = common::build_state(pool, "test-secret", &[]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;
    for _ in 0..1000 {
        let response = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/auth/password-reset/request")
                .set_json(json!({"email":"missing@example.test"}))
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::ACCEPTED);
    }
    let response = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/password-reset/request")
            .set_json(json!({"email":"missing@example.test"}))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[actix_web::test]
async fn moderation_can_hide_existing_filtered_text_and_owner_cannot_restore() {
    let Some(pool) = common::obtain_pool().await else {
        return;
    };
    let _lock = common::DB_LOCK.lock().await;
    common::reset_tables(&pool, &["users", "moderation_terms"])
        .await
        .unwrap();
    let (owner, _, _) = seed(&pool, "owner@example.test", "moderation_owner").await;
    let (_, _, token) = seed(&pool, "moderator@example.test", "moderation_admin").await;
    let event:i64 = sqlx::query_scalar("INSERT INTO events(name_event,description,owner_user_id,date_event,start_time,address_ciphertext) VALUES('badword','existing content',$1,CURRENT_DATE,'12:00',fiestaaa_encrypt_text('test')) RETURNING event_id").bind(owner).fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO moderation_terms VALUES('badword')")
        .execute(&pool)
        .await
        .unwrap();
    let app = test::init_service(
        App::new()
            .app_data(common::build_state(
                pool.clone(),
                "test-secret",
                &["moderator@example.test"],
            ))
            .configure(routes::configure),
    )
    .await;
    let response = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/admin/moderation")
            .insert_header(("Authorization", format!("Bearer {token}")))
            .set_json(json!({"action":"hide_event","event_id":event}))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        sqlx::query("UPDATE events SET deleted_at=NULL WHERE event_id=$1")
            .bind(event)
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(sqlx::query("INSERT INTO events(name_event,description,owner_user_id,date_event,start_time,address_ciphertext) VALUES('badword','new content',$1,CURRENT_DATE,'12:00',fiestaaa_encrypt_text('test'))").bind(owner).execute(&pool).await.is_err());
    sqlx::query("TRUNCATE moderation_terms")
        .execute(&pool)
        .await
        .unwrap();
}

#[actix_web::test]
async fn apple_callback_encodes_credentials_and_has_a_fixed_destination() {
    let app = test::init_service(App::new().configure(routes::configure)).await;
    let response = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/apple/android-callback")
            .set_form([("state", "one#Intent;package=other"), ("code", "a&b")])
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let location = response
        .headers()
        .get("Location")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(location.contains("code=a%26b"));
    assert_eq!(location.matches("#Intent;").count(), 1);
    assert!(location.ends_with("package=com.fiestaaa.fiestaaa;scheme=signinwithapple;end"));
    assert!(response.headers().get("Set-Cookie").is_none());
}
