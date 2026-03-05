mod common;

use std::error::Error;

use actix_web::{App, http::StatusCode, test};
use common::{DB_LOCK, build_state, obtain_pool, reset_tables};
use fiestaaa_back::{
    auth::{encode_jwt, hash_password, now_ts},
    models::{Claims, DeviceRefreshPayload, DeviceRegisterPayload},
    routes,
};
use serde_json::Value;
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

#[tokio::test]
async fn device_registration_refresh_and_delete_flow() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping notifications tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["user_devices", "users"]).await?;

    let secret = "secret";
    let user_id = seed_user(&pool, "owner@example.com", "owner_handle").await?;
    let state = build_state(pool.clone(), secret, &[]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;
    let token = make_token(secret, "owner@example.com", "owner_handle").expect("token");

    let register_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/me/devices")
            .insert_header(("Authorization", format!("Bearer {}", token.clone())))
            .set_json(&DeviceRegisterPayload {
                token: "device-token-1".into(),
                platform: "web".into(),
                locale: Some("fr-FR".into()),
                app_version: Some("1.0.0".into()),
            })
            .to_request(),
    )
    .await;
    assert_eq!(register_resp.status(), StatusCode::OK);
    let register_body: Value = test::read_body_json(register_resp).await;
    assert_eq!(
        register_body.get("status").and_then(Value::as_str),
        Some("saved")
    );

    let registered: (String, Option<String>, Option<String>, bool) = sqlx::query_as(
        "SELECT platform, locale, app_version, disabled_at IS NULL
         FROM user_devices
         WHERE user_id = $1 AND fcm_token = $2",
    )
    .bind(user_id)
    .bind("device-token-1")
    .fetch_one(&pool)
    .await?;
    assert_eq!(registered.0, "web");
    assert_eq!(registered.1.as_deref(), Some("fr-FR"));
    assert_eq!(registered.2.as_deref(), Some("1.0.0"));
    assert!(registered.3);

    let refresh_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/me/devices/refresh")
            .insert_header(("Authorization", format!("Bearer {}", token.clone())))
            .set_json(&DeviceRefreshPayload {
                old_token: "device-token-1".into(),
                new_token: "device-token-2".into(),
                platform: None,
                locale: Some("en-US".into()),
                app_version: Some("2.0.0".into()),
            })
            .to_request(),
    )
    .await;
    assert_eq!(refresh_resp.status(), StatusCode::OK);
    let refresh_body: Value = test::read_body_json(refresh_resp).await;
    assert_eq!(
        refresh_body.get("status").and_then(Value::as_str),
        Some("refreshed")
    );

    let old_disabled: (bool,) =
        sqlx::query_as("SELECT disabled_at IS NOT NULL FROM user_devices WHERE fcm_token = $1")
            .bind("device-token-1")
            .fetch_one(&pool)
            .await?;
    assert!(old_disabled.0);

    let refreshed: (String, Option<String>, Option<String>, bool) = sqlx::query_as(
        "SELECT platform, locale, app_version, disabled_at IS NULL
         FROM user_devices
         WHERE user_id = $1 AND fcm_token = $2",
    )
    .bind(user_id)
    .bind("device-token-2")
    .fetch_one(&pool)
    .await?;
    assert_eq!(refreshed.0, "web");
    assert_eq!(refreshed.1.as_deref(), Some("en-US"));
    assert_eq!(refreshed.2.as_deref(), Some("2.0.0"));
    assert!(refreshed.3);

    let delete_resp = test::call_service(
        &app,
        test::TestRequest::delete()
            .uri("/me/devices/device-token-2")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .to_request(),
    )
    .await;
    assert_eq!(delete_resp.status(), StatusCode::OK);

    let deleted: (bool,) =
        sqlx::query_as("SELECT disabled_at IS NOT NULL FROM user_devices WHERE fcm_token = $1")
            .bind("device-token-2")
            .fetch_one(&pool)
            .await?;
    assert!(deleted.0);
    Ok(())
}

#[tokio::test]
async fn device_registration_rejects_invalid_payloads() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping notifications tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["user_devices", "users"]).await?;

    let secret = "secret";
    seed_user(&pool, "owner@example.com", "owner_handle").await?;
    let state = build_state(pool, secret, &[]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;
    let token = make_token(secret, "owner@example.com", "owner_handle").expect("token");

    let empty_token_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/me/devices")
            .insert_header(("Authorization", format!("Bearer {}", token.clone())))
            .set_json(&DeviceRegisterPayload {
                token: "   ".into(),
                platform: "web".into(),
                locale: None,
                app_version: None,
            })
            .to_request(),
    )
    .await;
    assert_eq!(empty_token_resp.status(), StatusCode::BAD_REQUEST);

    let invalid_platform_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/me/devices")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .set_json(&DeviceRegisterPayload {
                token: "device-token".into(),
                platform: "desktop".into(),
                locale: None,
                app_version: None,
            })
            .to_request(),
    )
    .await;
    assert_eq!(invalid_platform_resp.status(), StatusCode::BAD_REQUEST);
    Ok(())
}
