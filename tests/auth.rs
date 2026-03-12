mod common;

use std::error::Error;

use actix_web::{App, http::StatusCode, test};
use common::{DB_LOCK, build_state, obtain_pool, reset_tables};
use fiestaaa_back::{
    auth::{hash_password, session_cookie_name, verify_password},
    routes,
};
use serde_json::Value;

async fn pending_token_for(pool: &sqlx::PgPool, email: &str) -> sqlx::Result<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT verification_token::text
         FROM pending_registrations
         WHERE lower(email) = lower($1)",
    )
    .bind(email)
    .fetch_one(pool)
    .await
}

#[tokio::test]
async fn register_creates_pending_registration_and_completes_user() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping auth tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["pending_registrations", "users"]).await?;

    let secret = "secret";
    let state = build_state(pool.clone(), secret, &[]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let email = "Admin@Test.com";
    let password = "Sup3rSecurePass!";
    let handle = "admin_test";

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/register")
            .set_json(serde_json::json!({ "email": email, "password": password }))
            .to_request(),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::CREATED);
    let body: Value = test::read_body_json(resp).await;
    assert!(matches!(
        body.get("status").and_then(|value| value.as_str()),
        Some("verification_email_sent" | "verification_pending")
    ));

    let user_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
        .fetch_one(&pool)
        .await?;
    assert_eq!(user_count.0, 0);

    let (stored_email, stored_hash): (String, String) =
        sqlx::query_as("SELECT email, password_hash FROM pending_registrations")
            .fetch_one(&pool)
            .await?;
    assert_eq!(stored_email, email.to_lowercase());
    assert_ne!(stored_hash, password);
    assert!(stored_hash.starts_with("$argon2"));

    let verify_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/verify-email")
            .set_json(serde_json::json!({ "token": pending_token_for(&pool, email).await? }))
            .to_request(),
    )
    .await;
    assert_eq!(verify_resp.status(), StatusCode::OK);
    let verify_body: Value = test::read_body_json(verify_resp).await;
    assert_eq!(
        verify_body.get("status").and_then(|value| value.as_str()),
        Some("setup_required")
    );

    let user_count_after_verify: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
        .fetch_one(&pool)
        .await?;
    assert_eq!(user_count_after_verify.0, 0);

    let complete_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/complete-registration")
            .set_json(serde_json::json!({
                "token": pending_token_for(&pool, email).await?,
                "password": password,
                "handle": handle,
            }))
            .to_request(),
    )
    .await;
    assert_eq!(complete_resp.status(), StatusCode::OK);
    let complete_body: Value = test::read_body_json(complete_resp).await;
    assert_eq!(
        complete_body.get("email").and_then(|value| value.as_str()),
        Some(email.to_lowercase().as_str())
    );
    assert_eq!(
        complete_body.get("handle").and_then(|value| value.as_str()),
        Some(handle)
    );

    let (verified_email, verified_hash, verified_handle): (String, String, String) =
        sqlx::query_as("SELECT email, password_hash, handle FROM users")
            .fetch_one(&pool)
            .await?;
    assert_eq!(verified_email, email.to_lowercase());
    assert_ne!(verified_hash, stored_hash);
    assert!(verify_password(&verified_hash, password));
    assert_eq!(verified_handle, handle);

    let pending_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM pending_registrations")
        .fetch_one(&pool)
        .await?;
    assert_eq!(pending_count.0, 0);

    Ok(())
}

#[tokio::test]
async fn register_rejects_invalid_payload() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping auth tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["pending_registrations", "users"]).await?;

    let state = build_state(pool.clone(), "secret", &[]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/register")
            .set_json(serde_json::json!({ "email": "", "password": "short" }))
            .to_request(),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
        .fetch_one(&pool)
        .await?;
    assert_eq!(count.0, 0);
    let pending_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM pending_registrations")
        .fetch_one(&pool)
        .await?;
    assert_eq!(pending_count.0, 0);
    Ok(())
}

#[tokio::test]
async fn register_hides_duplicate_email_state() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping auth tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["pending_registrations", "users"]).await?;

    let state = build_state(pool.clone(), "secret", &[]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let hash = hash_password("Sup3rSecurePass!")?;
    sqlx::query("INSERT INTO users (email, password_hash, handle) VALUES ($1, $2, $3)")
        .bind("dup@example.com")
        .bind(hash)
        .bind("dupuser")
        .execute(&pool)
        .await?;

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/register")
            .set_json(serde_json::json!({
                "email": "dup@example.com",
                "password": "Sup3rSecurePass!"
            }))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(
        body.get("status").and_then(|value| value.as_str()),
        Some("verification_pending")
    );

    let pending_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM pending_registrations WHERE lower(email) = lower($1)")
            .bind("dup@example.com")
            .fetch_one(&pool)
            .await?;
    assert_eq!(pending_count.0, 0);
    Ok(())
}

#[tokio::test]
async fn register_keeps_existing_pending_registration_unchanged() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping auth tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["pending_registrations", "users"]).await?;

    let state = build_state(pool.clone(), "secret", &[]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let email = "dup-pending@example.com";
    let first_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/register")
            .set_json(serde_json::json!({ "email": email }))
            .to_request(),
    )
    .await;
    assert_eq!(first_resp.status(), StatusCode::CREATED);
    let first_token = pending_token_for(&pool, email).await?;

    let second_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/register")
            .set_json(serde_json::json!({
                "email": email,
                "password": "AnotherStr0ng!Pass",
                "handle": "attacker_handle"
            }))
            .to_request(),
    )
    .await;
    assert_eq!(second_resp.status(), StatusCode::CREATED);
    let body: Value = test::read_body_json(second_resp).await;
    assert_eq!(
        body.get("status").and_then(|value| value.as_str()),
        Some("verification_pending")
    );

    let second_token = pending_token_for(&pool, email).await?;
    assert_eq!(second_token, first_token);

    let pending_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM pending_registrations")
        .fetch_one(&pool)
        .await?;
    assert_eq!(pending_count.0, 1);

    Ok(())
}

#[tokio::test]
async fn login_returns_token_for_valid_credentials() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping auth tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["pending_registrations", "users"]).await?;

    let state = build_state(pool.clone(), "secret", &[]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let email = "user@example.com";
    let password = "MyStr0ng!Pass#2025";

    let register_payload = serde_json::json!({ "email": email });
    let register_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/register")
            .set_json(&register_payload)
            .to_request(),
    )
    .await;
    assert_eq!(register_resp.status(), StatusCode::CREATED);
    let verify_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/verify-email")
            .set_json(serde_json::json!({ "token": pending_token_for(&pool, email).await? }))
            .to_request(),
    )
    .await;
    assert_eq!(verify_resp.status(), StatusCode::OK);
    let complete_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/complete-registration")
            .set_json(serde_json::json!({
                "token": pending_token_for(&pool, email).await?,
                "password": password,
            }))
            .to_request(),
    )
    .await;
    assert_eq!(complete_resp.status(), StatusCode::OK);

    let login_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/login")
            .set_json(serde_json::json!({ "identifier": email, "password": password }))
            .to_request(),
    )
    .await;
    assert_eq!(login_resp.status(), StatusCode::OK);
    assert_eq!(
        login_resp
            .headers()
            .get("cache-control")
            .and_then(|value| value.to_str().ok()),
        Some("no-store")
    );
    assert_eq!(
        login_resp
            .headers()
            .get("pragma")
            .and_then(|value| value.to_str().ok()),
        Some("no-cache")
    );

    let json: Value = test::read_body_json(login_resp).await;
    let token = json
        .get("token")
        .and_then(|v| v.as_str())
        .expect("token field in JSON response");
    assert!(!token.is_empty());

    Ok(())
}

#[tokio::test]
async fn login_rejects_invalid_credentials() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping auth tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["pending_registrations", "users"]).await?;

    let state = build_state(pool.clone(), "secret", &[]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let email = "user2@example.com";
    let password = "MyStr0ng!Pass#2025";

    let register_payload = serde_json::json!({ "email": email });
    let register_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/register")
            .set_json(&register_payload)
            .to_request(),
    )
    .await;
    assert_eq!(register_resp.status(), StatusCode::CREATED);
    let verify_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/verify-email")
            .set_json(serde_json::json!({ "token": pending_token_for(&pool, email).await? }))
            .to_request(),
    )
    .await;
    assert_eq!(verify_resp.status(), StatusCode::OK);
    let complete_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/complete-registration")
            .set_json(serde_json::json!({
                "token": pending_token_for(&pool, email).await?,
                "password": password,
            }))
            .to_request(),
    )
    .await;
    assert_eq!(complete_resp.status(), StatusCode::OK);

    let wrong_password_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/login")
            .set_json(serde_json::json!({ "identifier": email, "password": "wrongpass" }))
            .to_request(),
    )
    .await;
    assert_eq!(wrong_password_resp.status(), StatusCode::UNAUTHORIZED);

    let unknown_user_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/login")
            .set_json(
                serde_json::json!({ "identifier": "ghost@example.com", "password": "something" }),
            )
            .to_request(),
    )
    .await;
    assert_eq!(unknown_user_resp.status(), StatusCode::UNAUTHORIZED);

    Ok(())
}

#[tokio::test]
async fn login_keeps_pending_registrations_indistinguishable() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping auth tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["pending_registrations", "users"]).await?;

    let state = build_state(pool.clone(), "secret", &[]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let email = "pending@example.com";
    let password = "MyStr0ng!Pass#2025";

    let register_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/register")
            .set_json(serde_json::json!({ "email": email }))
            .to_request(),
    )
    .await;
    assert_eq!(register_resp.status(), StatusCode::CREATED);

    let login_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/login")
            .set_json(serde_json::json!({ "identifier": email, "password": password }))
            .to_request(),
    )
    .await;
    assert_eq!(login_resp.status(), StatusCode::UNAUTHORIZED);

    let body: Value = test::read_body_json(login_resp).await;
    assert_eq!(
        body.get("error").and_then(|value| value.as_str()),
        Some("invalid_credentials")
    );

    Ok(())
}

#[tokio::test]
async fn delete_account_removes_user() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping auth tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["pending_registrations", "users"]).await?;

    let secret = "secret";
    let state = build_state(pool.clone(), secret, &[]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let email = "delete-me@example.com";
    let password = "MyStr0ng!Pass#2025";

    let register_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/register")
            .set_json(serde_json::json!({ "email": email }))
            .to_request(),
    )
    .await;
    assert_eq!(register_resp.status(), StatusCode::CREATED);
    let verify_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/verify-email")
            .set_json(serde_json::json!({ "token": pending_token_for(&pool, email).await? }))
            .to_request(),
    )
    .await;
    assert_eq!(verify_resp.status(), StatusCode::OK);
    let complete_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/complete-registration")
            .set_json(serde_json::json!({
                "token": pending_token_for(&pool, email).await?,
                "password": password,
            }))
            .to_request(),
    )
    .await;
    assert_eq!(complete_resp.status(), StatusCode::OK);

    let login_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/login")
            .set_json(serde_json::json!({ "identifier": email, "password": password }))
            .to_request(),
    )
    .await;
    assert_eq!(login_resp.status(), StatusCode::OK);

    let login_json: Value = test::read_body_json(login_resp).await;
    let token = login_json
        .get("token")
        .and_then(|v| v.as_str())
        .expect("token in response");

    let count_before: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
        .fetch_one(&pool)
        .await?;
    assert_eq!(count_before.0, 1);

    let delete_resp = test::call_service(
        &app,
        test::TestRequest::delete()
            .uri("/me")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .to_request(),
    )
    .await;
    assert_eq!(delete_resp.status(), StatusCode::OK);
    let cleared_cookie = delete_resp
        .headers()
        .get("set-cookie")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(cleared_cookie.contains(&format!("{}=", session_cookie_name())));
    assert!(cleared_cookie.contains("Max-Age=0"));

    let delete_json: Value = test::read_body_json(delete_resp).await;
    assert_eq!(
        delete_json.get("status").and_then(|v| v.as_str()),
        Some("account_deleted")
    );

    let count_after: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
        .fetch_one(&pool)
        .await?;
    assert_eq!(count_after.0, 0);

    Ok(())
}

#[tokio::test]
async fn delete_account_requires_auth() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping auth tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["pending_registrations", "users"]).await?;

    let state = build_state(pool.clone(), "secret", &[]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let no_header_resp =
        test::call_service(&app, test::TestRequest::delete().uri("/me").to_request()).await;
    assert_eq!(no_header_resp.status(), StatusCode::UNAUTHORIZED);

    let invalid_token_resp = test::call_service(
        &app,
        test::TestRequest::delete()
            .uri("/me")
            .insert_header(("Authorization", "Bearer invalid.token.here"))
            .to_request(),
    )
    .await;
    assert_eq!(invalid_token_resp.status(), StatusCode::UNAUTHORIZED);

    Ok(())
}

#[tokio::test]
async fn delete_account_returns_404_for_missing_user() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping auth tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["pending_registrations", "users"]).await?;

    let secret = "secret";
    let state = build_state(pool.clone(), secret, &[]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let email = "ghost@example.com";
    let password = "MyStr0ng!Pass#2025";

    let register_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/register")
            .set_json(serde_json::json!({ "email": email }))
            .to_request(),
    )
    .await;
    assert_eq!(register_resp.status(), StatusCode::CREATED);
    let verify_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/verify-email")
            .set_json(serde_json::json!({ "token": pending_token_for(&pool, email).await? }))
            .to_request(),
    )
    .await;
    assert_eq!(verify_resp.status(), StatusCode::OK);
    let complete_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/complete-registration")
            .set_json(serde_json::json!({
                "token": pending_token_for(&pool, email).await?,
                "password": password,
            }))
            .to_request(),
    )
    .await;
    assert_eq!(complete_resp.status(), StatusCode::OK);

    let login_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/login")
            .set_json(serde_json::json!({ "identifier": email, "password": password }))
            .to_request(),
    )
    .await;
    assert_eq!(login_resp.status(), StatusCode::OK);
    let login_json: Value = test::read_body_json(login_resp).await;
    let token = login_json
        .get("token")
        .and_then(|v| v.as_str())
        .expect("token");

    sqlx::query("DELETE FROM users WHERE lower(email) = lower($1)")
        .bind(email)
        .execute(&pool)
        .await?;

    let delete_resp = test::call_service(
        &app,
        test::TestRequest::delete()
            .uri("/me")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .to_request(),
    )
    .await;
    assert_eq!(delete_resp.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[tokio::test]
async fn browser_login_uses_cookie_without_exposing_token() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping auth tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(
        &pool,
        &["revoked_auth_tokens", "pending_registrations", "users"],
    )
    .await?;

    let state = build_state(pool.clone(), "secret", &[]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let email = "web@example.com";
    let password = "MyStr0ng!Pass#2025";

    let register_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/register")
            .set_json(serde_json::json!({ "email": email }))
            .to_request(),
    )
    .await;
    assert_eq!(register_resp.status(), StatusCode::CREATED);

    let verify_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/verify-email")
            .set_json(serde_json::json!({ "token": pending_token_for(&pool, email).await? }))
            .to_request(),
    )
    .await;
    assert_eq!(verify_resp.status(), StatusCode::OK);

    let complete_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/complete-registration")
            .set_json(serde_json::json!({
                "token": pending_token_for(&pool, email).await?,
                "password": password,
            }))
            .to_request(),
    )
    .await;
    assert_eq!(complete_resp.status(), StatusCode::OK);

    let login_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/login")
            .insert_header(("Origin", "https://fiestaaa.app"))
            .set_json(serde_json::json!({ "identifier": email, "password": password }))
            .to_request(),
    )
    .await;
    assert_eq!(login_resp.status(), StatusCode::OK);
    let session_cookie = login_resp
        .headers()
        .get("set-cookie")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(session_cookie.contains(&format!("{}=", session_cookie_name())));

    let login_body: Value = test::read_body_json(login_resp).await;
    assert_eq!(
        login_body.get("token").and_then(|value| value.as_str()),
        Some("")
    );
    assert_eq!(
        login_body.get("email").and_then(|value| value.as_str()),
        Some(email)
    );

    Ok(())
}

#[tokio::test]
async fn logout_revokes_current_token() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping auth tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(
        &pool,
        &["revoked_auth_tokens", "pending_registrations", "users"],
    )
    .await?;

    let state = build_state(pool.clone(), "secret", &[]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let email = "logout@example.com";
    let password = "MyStr0ng!Pass#2025";

    let register_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/register")
            .set_json(serde_json::json!({ "email": email }))
            .to_request(),
    )
    .await;
    assert_eq!(register_resp.status(), StatusCode::CREATED);

    let verify_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/verify-email")
            .set_json(serde_json::json!({ "token": pending_token_for(&pool, email).await? }))
            .to_request(),
    )
    .await;
    assert_eq!(verify_resp.status(), StatusCode::OK);

    let complete_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/complete-registration")
            .set_json(serde_json::json!({
                "token": pending_token_for(&pool, email).await?,
                "password": password,
            }))
            .to_request(),
    )
    .await;
    assert_eq!(complete_resp.status(), StatusCode::OK);

    let login_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/login")
            .set_json(serde_json::json!({ "identifier": email, "password": password }))
            .to_request(),
    )
    .await;
    assert_eq!(login_resp.status(), StatusCode::OK);
    let login_body: Value = test::read_body_json(login_resp).await;
    let token = login_body
        .get("token")
        .and_then(|value| value.as_str())
        .expect("token in response")
        .to_string();

    let logout_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/logout")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .to_request(),
    )
    .await;
    assert_eq!(logout_resp.status(), StatusCode::OK);

    let me_resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/me")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .to_request(),
    )
    .await;
    assert_eq!(me_resp.status(), StatusCode::UNAUTHORIZED);
    let me_body: Value = test::read_body_json(me_resp).await;
    assert_eq!(
        me_body.get("error").and_then(|value| value.as_str()),
        Some("revoked_token")
    );

    Ok(())
}

#[tokio::test]
async fn deleted_user_token_cannot_access_events() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping auth tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(
        &pool,
        &[
            "revoked_auth_tokens",
            "pending_registrations",
            "invitations",
            "events",
            "users",
        ],
    )
    .await?;

    let state = build_state(pool.clone(), "secret", &[]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let email = "deleted-owner@example.com";
    let password = "MyStr0ng!Pass#2025";

    let register_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/register")
            .set_json(serde_json::json!({ "email": email }))
            .to_request(),
    )
    .await;
    assert_eq!(register_resp.status(), StatusCode::CREATED);

    let verify_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/verify-email")
            .set_json(serde_json::json!({ "token": pending_token_for(&pool, email).await? }))
            .to_request(),
    )
    .await;
    assert_eq!(verify_resp.status(), StatusCode::OK);

    let complete_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/complete-registration")
            .set_json(serde_json::json!({
                "token": pending_token_for(&pool, email).await?,
                "password": password,
            }))
            .to_request(),
    )
    .await;
    assert_eq!(complete_resp.status(), StatusCode::OK);

    let login_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/login")
            .set_json(serde_json::json!({ "identifier": email, "password": password }))
            .to_request(),
    )
    .await;
    assert_eq!(login_resp.status(), StatusCode::OK);
    let login_body: Value = test::read_body_json(login_resp).await;
    let token = login_body
        .get("token")
        .and_then(|value| value.as_str())
        .expect("token in response")
        .to_string();

    sqlx::query("DELETE FROM users WHERE lower(email) = lower($1)")
        .bind(email)
        .execute(&pool)
        .await?;

    let events_resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/events")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .to_request(),
    )
    .await;
    assert_eq!(events_resp.status(), StatusCode::UNAUTHORIZED);
    let events_body: Value = test::read_body_json(events_resp).await;
    assert_eq!(
        events_body.get("error").and_then(|value| value.as_str()),
        Some("user_not_found")
    );

    Ok(())
}
