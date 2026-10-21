mod common;

use actix_web::{App, http::StatusCode, test, web};
use common::{DB_LOCK, build_state, obtain_pool, reset_tables};
use fiestaaa_back::{
    auth::{decode_jwt, hash_password, now_ts},
    routes,
};
use serde_json::{Value, json};

#[actix_web::test]
async fn mobile_sessions_renew_rotate_and_revoke_without_daily_login()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = obtain_pool().await else {
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["users"]).await?;
    sqlx::query(
        "INSERT INTO users (email_ciphertext, email_lookup_hash, password_hash, handle)
        VALUES (fiestaaa_encrypt_text($1), fiestaaa_email_lookup($1), $2, 'sessionuser')",
    )
    .bind("session@example.invalid")
    .bind(hash_password("TestPassword123!")?)
    .execute(&pool)
    .await?;
    let app = test::init_service(
        App::new()
            .app_data(build_state(pool.clone(), "session-test-secret", &[]))
            .app_data(web::JsonConfig::default())
            .configure(routes::configure),
    )
    .await;
    macro_rules! login {
        ($mode:expr) => {{
            let response = test::call_service(&app, test::TestRequest::post().uri("/auth/login")
                .insert_header(("X-Fiestaaa-Auth-Response", $mode))
                .set_json(json!({"identifier":"sessionuser", "password":"TestPassword123!"})).to_request()).await;
            assert_eq!(response.status(), StatusCode::OK);
            test::read_body_json::<Value, _>(response).await
        }};
    }
    macro_rules! refresh {
        ($token:expr) => {{
            test::call_service(&app, test::TestRequest::post().uri("/auth/refresh")
                .insert_header(("X-Fiestaaa-Auth-Response", "bearer-refresh"))
                .set_json(json!({"refresh_token": $token})).to_request()).await
        }};
    }
    let initial = login!("bearer-refresh");
    let access = initial["token"].as_str().unwrap();
    let original = initial["refresh_token"].as_str().unwrap();
    let claims = decode_jwt(access, "session-test-secret").unwrap();
    assert!((3590..=3600).contains(&(claims.exp as u64 - now_ts())));
    // Another device logging in in the same second gets an independent JWT.
    let other_device = login!("bearer-refresh");
    assert_ne!(initial["token"], other_device["token"]);
    assert_ne!(initial["refresh_token"], other_device["refresh_token"]);
    // Short-lived access alone cannot create a fresh long-lived session.
    let escalation = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/refresh")
            .insert_header(("X-Fiestaaa-Auth-Response", "bearer-refresh"))
            .insert_header(("Authorization", format!("Bearer {access}")))
            .set_json(json!({}))
            .to_request(),
    )
    .await;
    assert_eq!(escalation.status(), StatusCode::UNAUTHORIZED);
    let stored_hash: String =
        sqlx::query_scalar("SELECT token_hash FROM mobile_sessions WHERE token_hash = $1")
            .bind(fiestaaa_back::security::sha256_hex(original))
            .fetch_one(&pool)
            .await?;
    assert_ne!(stored_hash, original);
    // Refresh does not require a live access token, even after days of inactivity.
    let renewed = refresh!(original);
    assert_eq!(renewed.status(), StatusCode::OK);
    assert_eq!(renewed.headers().get("cache-control").unwrap(), "no-store");
    let renewed: Value = test::read_body_json(renewed).await;
    let rotated = renewed["refresh_token"].as_str().unwrap();
    assert_ne!(original, rotated);
    let retry = refresh!(original);
    assert_eq!(retry.status(), StatusCode::OK);
    let retry: Value = test::read_body_json(retry).await;
    assert_eq!(retry["refresh_token"], rotated);
    assert_eq!(refresh!("unknown").status(), StatusCode::UNAUTHORIZED);
    sqlx::query("UPDATE mobile_sessions SET previous_valid_until = NOW() - INTERVAL '1 second'")
        .execute(&pool)
        .await?;
    assert_eq!(refresh!(original).status(), StatusCode::UNAUTHORIZED);
    assert_eq!(refresh!(rotated).status(), StatusCode::OK);

    // A password reset / global revocation increments the existing version.
    sqlx::query("UPDATE users SET session_version = session_version + 1")
        .execute(&pool)
        .await?;
    assert_eq!(refresh!(rotated).status(), StatusCode::UNAUTHORIZED);
    let initial = login!("bearer-refresh");
    sqlx::query("UPDATE users SET suspended = TRUE")
        .execute(&pool)
        .await?;
    assert_eq!(
        refresh!(initial["refresh_token"].as_str().unwrap()).status(),
        StatusCode::UNAUTHORIZED
    );
    sqlx::query("UPDATE users SET suspended = FALSE")
        .execute(&pool)
        .await?;
    sqlx::query("UPDATE mobile_sessions SET expires_at = NOW() - INTERVAL '1 second'")
        .execute(&pool)
        .await?;
    assert_eq!(
        refresh!(initial["refresh_token"].as_str().unwrap()).status(),
        StatusCode::UNAUTHORIZED
    );

    let initial = login!("bearer-refresh");
    sqlx::query("UPDATE mobile_sessions SET created_at = NOW() - INTERVAL '366 days', expires_at = NOW() + INTERVAL '1 day'").execute(&pool).await?;
    assert_eq!(
        refresh!(initial["refresh_token"].as_str().unwrap()).status(),
        StatusCode::UNAUTHORIZED
    );
    let initial = login!("bearer-refresh");
    let logout = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/logout")
            .insert_header((
                "Authorization",
                format!("Bearer {}", initial["token"].as_str().unwrap()),
            ))
            .set_json(json!({"refresh_token":initial["refresh_token"]}))
            .to_request(),
    )
    .await;
    assert_eq!(logout.status(), StatusCode::OK);
    assert_eq!(
        refresh!(initial["refresh_token"].as_str().unwrap()).status(),
        StatusCode::UNAUTHORIZED
    );
    let me = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/me")
            .insert_header((
                "Authorization",
                format!("Bearer {}", initial["token"].as_str().unwrap()),
            ))
            .to_request(),
    )
    .await;
    assert_eq!(me.status(), StatusCode::UNAUTHORIZED);

    let legacy = login!("bearer");
    assert!(legacy.get("refresh_token").is_none());
    let upgrade = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/refresh")
            .insert_header(("X-Fiestaaa-Auth-Response", "bearer-refresh"))
            .insert_header((
                "Authorization",
                format!("Bearer {}", legacy["token"].as_str().unwrap()),
            ))
            .set_json(json!({}))
            .to_request(),
    )
    .await;
    assert_eq!(upgrade.status(), StatusCode::OK);
    let upgrade: Value = test::read_body_json(upgrade).await;
    assert!(upgrade["refresh_token"].as_str().is_some());
    // Cookie-only callers cannot obtain a native refresh credential.
    let web_attempt = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/refresh")
            .set_json(json!({"refresh_token": upgrade["refresh_token"]}))
            .to_request(),
    )
    .await;
    assert_eq!(web_attempt.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}
