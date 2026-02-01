mod common;

use std::error::Error;

use actix_web::{App, http::StatusCode, test};
use common::{DB_LOCK, build_state, obtain_pool, reset_tables};
use fiestaaa_back::routes;
use serde_json::Value;

#[tokio::test]
async fn register_creates_user_and_hashes_password() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping auth tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["users"]).await?;

    let secret = "secret";
    let state = build_state(pool.clone(), secret, &[]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let email = "Admin@Test.com";
    let password = "supersafepw";

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/register")
            .set_json(serde_json::json!({ "email": email, "password": password }))
            .to_request(),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::CREATED);

    let (stored_email, stored_hash): (String, String) =
        sqlx::query_as("SELECT email, password_hash FROM users")
            .fetch_one(&pool)
            .await?;
    assert_eq!(stored_email, email.to_lowercase());
    assert_ne!(stored_hash, password);
    assert!(stored_hash.starts_with("$argon2"));

    Ok(())
}

#[tokio::test]
async fn register_rejects_invalid_payload() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping auth tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["users"]).await?;

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
    Ok(())
}

#[tokio::test]
async fn register_rejects_duplicate_email() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping auth tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["users"]).await?;

    let state = build_state(pool.clone(), "secret", &[]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let payload = serde_json::json!({ "email": "dup@example.com", "password": "strongpass" });

    let first = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/register")
            .set_json(&payload)
            .to_request(),
    )
    .await;
    assert_eq!(first.status(), StatusCode::CREATED);

    let second = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/register")
            .set_json(&payload)
            .to_request(),
    )
    .await;
    assert_eq!(second.status(), StatusCode::CONFLICT);
    Ok(())
}

#[tokio::test]
async fn login_returns_token_for_valid_credentials() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping auth tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["users"]).await?;

    let state = build_state(pool.clone(), "secret", &[]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let email = "user@example.com";
    let password = "mypassword";

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

    let login_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/login")
            .set_json(serde_json::json!({ "email": email, "password": password }))
            .to_request(),
    )
    .await;
    assert_eq!(login_resp.status(), StatusCode::OK);

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
    reset_tables(&pool, &["users"]).await?;

    let state = build_state(pool.clone(), "secret", &[]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let email = "user2@example.com";
    let password = "mypassword";

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

    let wrong_password_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/login")
            .set_json(serde_json::json!({ "email": email, "password": "wrongpass" }))
            .to_request(),
    )
    .await;
    assert_eq!(wrong_password_resp.status(), StatusCode::UNAUTHORIZED);

    let unknown_user_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/login")
            .set_json(serde_json::json!({ "email": "ghost@example.com", "password": "something" }))
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
    reset_tables(&pool, &["users"]).await?;

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

    let login_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/login")
            .set_json(serde_json::json!({ "email": email, "password": password }))
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
    reset_tables(&pool, &["users"]).await?;

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
    reset_tables(&pool, &["users"]).await?;

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

    let login_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/login")
            .set_json(serde_json::json!({ "email": email, "password": password }))
            .to_request(),
    )
    .await;
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
