mod common;

use std::error::Error;

use actix_web::{App, http::StatusCode, test};
use common::{DB_LOCK, build_state, obtain_pool, reset_tables};
use fiestaaa_back::{auth::hash_password, routes};
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
async fn register_creates_pending_registration_and_verifies_user() -> Result<(), Box<dyn Error>> {
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

    let (verified_email, verified_hash): (String, String) =
        sqlx::query_as("SELECT email, password_hash FROM users")
            .fetch_one(&pool)
            .await?;
    assert_eq!(verified_email, email.to_lowercase());
    assert_eq!(verified_hash, stored_hash);

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
async fn register_rejects_duplicate_email() -> Result<(), Box<dyn Error>> {
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
    assert_eq!(resp.status(), StatusCode::CONFLICT);
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

    let register_payload = serde_json::json!({ "email": email, "password": password });
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

    let register_payload = serde_json::json!({ "email": email, "password": password });
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
            .set_json(serde_json::json!({ "email": email, "password": password }))
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
            .set_json(serde_json::json!({ "email": email, "password": password }))
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
