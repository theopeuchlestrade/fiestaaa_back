mod common;

use std::{error::Error, net::SocketAddr, thread::JoinHandle};

use actix_web::{
    App, HttpRequest, HttpResponse, HttpServer,
    dev::ServerHandle,
    http::{StatusCode, header::AUTHORIZATION},
    test, web,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use common::{
    DB_LOCK, TestOAuthConfig, build_state, build_state_with_oauth_config, obtain_pool, reset_tables,
};
use fiestaaa_back::{
    auth::{hash_password, now_ts, session_cookie_name, verify_password},
    routes,
    security::sha256_hex,
};
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

const TEST_GOOGLE_WEB_CLIENT_ID: &str = "fiestaaa-web-client.apps.googleusercontent.com";
const TEST_APPLE_SERVICE_ID: &str = "app.fiestaaa.web";
const TEST_APPLE_KID: &str = "fiestaaa-test-apple-key";
const TEST_APPLE_MODULUS: &str = "zp_AAIAxF-hYotR45X6z1ZFYEAUlCixB0VcRVTq13fSyqdtuxWskRkeZQ1N0DTHZFU88On5LF5syZs7IZcP49U9DhB7AXDs1IcrsRVtBCg28omstiWb-eTGjLEmvNQf52aI6t_2gMCDnQ__NOhMAFJWMFhnDZynve99VUXnsm1m1Q50XgxPo45JW1p7eyU7zvsAr8NnB4Tg6W-Q5zYuXAXMJoKYURuqKDE5SNky1ZFLDwoj9zd86sPxWUHwZ6auhgVY29KCbJRHycirPl8AmmxT4fSflXK_192mDqOZMD898_snXG3LrvaK5Q-ZNJWdfF3_3ehs25zyjHqASrTQIAQ";
const TEST_APPLE_PRIVATE_KEY: &str = r#"-----BEGIN RSA PRIVATE KEY-----
MIIEpAIBAAKCAQEAzp/AAIAxF+hYotR45X6z1ZFYEAUlCixB0VcRVTq13fSyqdtu
xWskRkeZQ1N0DTHZFU88On5LF5syZs7IZcP49U9DhB7AXDs1IcrsRVtBCg28omst
iWb+eTGjLEmvNQf52aI6t/2gMCDnQ//NOhMAFJWMFhnDZynve99VUXnsm1m1Q50X
gxPo45JW1p7eyU7zvsAr8NnB4Tg6W+Q5zYuXAXMJoKYURuqKDE5SNky1ZFLDwoj9
zd86sPxWUHwZ6auhgVY29KCbJRHycirPl8AmmxT4fSflXK/192mDqOZMD898/snX
G3LrvaK5Q+ZNJWdfF3/3ehs25zyjHqASrTQIAQIDAQABAoIBAAlMx+nN20fR8aFc
kld0ATig2teXv6/KR6kaM+HD63kiwSLjiUQR+zc9lECjQjMw1e4/W3zff9Y/akCV
2I+6BxvVdjq9XpeYI59SgJlrjs0ayq19yPYpAFWonglJhL0Mj5qT0nRDEmFwLbCS
FCTjw4ppo70/6htb2BdZeT/aTsO8LHgEO/Cx7Bu3D8wKC+1mHgjuROkOEZGZC6/m
JUXxZT8qHOgjIB4GRAvkiqUoOHuDPq8d+g8o99uzA43fjO3DDHWiZbHowe7GJSHc
hi2p+DbG2LLK15R00QCFEqy7tQefXnnbH6iMGat71f2hYw94cbhFrLg+zXmW6+Io
Kmu8utUCgYEA5vw5CEXmV7bQtPadiT7lUPd4lnvqnLyWWTv9JopJL5kZdkmXz3RD
DrY3TcetwzchccbzOVgH3uDbZlMuk/q4CgkGWmLtHrr6k70+Pr0zOAaNDhH/+mlG
MFcGocDGj6SXTBgIDM1lylCkBLaC4d8vaJNxX532hfvwkLUAv3wC6A0CgYEA5QAk
jo/KGsODKpPXaz3+J+fR76TMjLsLxzk3L7ifysB4/HuGcl9/fQ7qux+MQMN88CRy
oAUEB9qN4GY4ICuTAA3bwWDSjDqGdyiN4LQBFdR3TFLwV37Yc2NpcI7aIbbMTmOz
L+VdG49y6lHc8pv8R8XqcnYwNvMHusA7RkTxzsUCgYEA1zzetDvWaZPcJVTM9ZAb
RXhk8O0lcMo2244P1jL0AZuLY3MuOE0hE3tuS1cvLwKXcpsuGBhUtTYYm+AVPiVa
C1ffiKg4RvN6/eJRN0s8iA9qr1rMif5BPlhJwL6PCFkZ9vlJvwxCtuSwAghEK8+6
MJt8ANqEVtOulllkCgq39p0CgYEAzSGtnY6sSgEs8+zvIP+tNW3xnquPF9lNma5l
AvhtGyACwJiePMHS3+GG3wxJhJIYzry3eSRFEgvy3zpxuE+QJJJFchobQMYEQaUw
QkK8XiOuoc4BwT69Ac/hWZR9TYoDxYyFrLfXCaMcG04tj52vBVQCyXmZgv98wwsD
jdSgjskCgYBlG8tPgcx6uvK5M76yoW79twgsWXWafVUSlKqEvrrE7l5pxGwiiSzb
ul/DgmDWuyiZmPqXu0sY6DqWbT4RZZK888cK05BzA/bzyVFJJxPa2mC8QM4w9T2X
hrNGoIWum5S6bDLJo9GG+CV5wNYO5gGhWzm6W28SgkdoB1dmV8YwBQ==
-----END RSA PRIVATE KEY-----"#;

async fn overwrite_pending_token_for(pool: &sqlx::PgPool, email: &str) -> sqlx::Result<String> {
    let token = Uuid::new_v4().to_string();
    sqlx::query(
        "UPDATE pending_registrations
         SET verification_token_hash = $1
         WHERE fiestaaa_email_matches(email_lookup_hash, $2)",
    )
    .bind(sha256_hex(&token))
    .bind(email)
    .execute(pool)
    .await
    .map(|_| token)
}

async fn pending_token_for(pool: &sqlx::PgPool, email: &str) -> sqlx::Result<String> {
    overwrite_pending_token_for(pool, email).await
}

async fn pending_token_hash_for(pool: &sqlx::PgPool, email: &str) -> sqlx::Result<String> {
    sqlx::query_scalar(
        "SELECT verification_token_hash
         FROM pending_registrations
         WHERE fiestaaa_email_matches(email_lookup_hash, $1)",
    )
    .bind(email)
    .fetch_one(pool)
    .await
}

async fn mock_google_tokeninfo(req: HttpRequest) -> HttpResponse {
    let query = req.query_string();
    if query.contains("id_token=valid-google-id-token") {
        return HttpResponse::Ok().json(serde_json::json!({
            "aud": TEST_GOOGLE_WEB_CLIENT_ID,
            "iss": "https://accounts.google.com",
            "sub": "google-id-subject",
            "email": "Google.User@Example.com",
            "email_verified": "true"
        }));
    }
    if query.contains("id_token=wrong-aud-google-id-token") {
        return HttpResponse::Ok().json(serde_json::json!({
            "aud": "another-client.apps.googleusercontent.com",
            "iss": "https://accounts.google.com",
            "sub": "google-wrong-aud",
            "email": "wrong-aud@example.com",
            "email_verified": true
        }));
    }
    if query.contains("access_token=valid-google-access-token") {
        return HttpResponse::Ok().json(serde_json::json!({
            "audience": TEST_GOOGLE_WEB_CLIENT_ID,
            "sub": "google-access-subject"
        }));
    }

    HttpResponse::Unauthorized().finish()
}

async fn mock_google_userinfo(req: HttpRequest) -> HttpResponse {
    let auth = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    if auth == Some("Bearer valid-google-access-token") {
        return HttpResponse::Ok().json(serde_json::json!({
            "sub": "google-access-subject",
            "email": "Access.User@Example.com",
            "email_verified": true
        }));
    }

    HttpResponse::Unauthorized().finish()
}

async fn mock_apple_keys() -> HttpResponse {
    HttpResponse::Ok().json(serde_json::json!({
        "keys": [{
            "kid": TEST_APPLE_KID,
            "kty": "RSA",
            "alg": "RS256",
            "use": "sig",
            "n": TEST_APPLE_MODULUS,
            "e": "AQAB"
        }]
    }))
}

struct OAuthMockServer {
    base_url: String,
    handle: ServerHandle,
    thread: JoinHandle<()>,
}

impl OAuthMockServer {
    async fn stop(self) {
        self.handle.stop(true).await;
        let _ = self.thread.join();
    }
}

fn spawn_oauth_mock_server() -> Result<OAuthMockServer, Box<dyn Error>> {
    let (tx, rx) = std::sync::mpsc::channel::<std::io::Result<(SocketAddr, ServerHandle)>>();
    let thread = std::thread::spawn(move || {
        actix_web::rt::System::new().block_on(async move {
            let server = match HttpServer::new(|| {
                App::new()
                    .route("/google/tokeninfo", web::get().to(mock_google_tokeninfo))
                    .route("/google/userinfo", web::get().to(mock_google_userinfo))
                    .route("/apple/keys", web::get().to(mock_apple_keys))
            })
            .bind(("127.0.0.1", 0))
            {
                Ok(server) => server,
                Err(err) => {
                    let _ = tx.send(Err(err));
                    return;
                }
            };
            let Some(addr) = server.addrs().first().copied() else {
                let _ = tx.send(Err(std::io::Error::other("oauth mock server did not bind")));
                return;
            };
            let server = server.run();
            let handle = server.handle();
            let _ = tx.send(Ok((addr, handle)));
            let _ = server.await;
        });
    });
    let (addr, handle) = rx.recv()??;
    Ok(OAuthMockServer {
        base_url: format!("http://{addr}"),
        handle,
        thread,
    })
}

fn oauth_test_config(base_url: &str) -> TestOAuthConfig {
    TestOAuthConfig {
        google_client_id: Some(TEST_GOOGLE_WEB_CLIENT_ID.into()),
        apple_service_id: Some(TEST_APPLE_SERVICE_ID.into()),
        google_tokeninfo_url: Some(format!("{base_url}/google/tokeninfo")),
        google_userinfo_url: Some(format!("{base_url}/google/userinfo")),
        apple_jwks_url: Some(format!("{base_url}/apple/keys")),
        ..TestOAuthConfig::default()
    }
}

#[derive(Serialize)]
struct TestAppleClaims<'a> {
    sub: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    email: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    email_verified: Option<serde_json::Value>,
    exp: usize,
    iss: &'a str,
    aud: &'a str,
}

fn apple_id_token(aud: &str, sub: &str, email: Option<&str>) -> String {
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
    header.kid = Some(TEST_APPLE_KID.into());
    let claims = TestAppleClaims {
        sub,
        email,
        email_verified: email.map(|_| serde_json::json!("true")),
        exp: (now_ts() + 3600) as usize,
        iss: "https://appleid.apple.com",
        aud,
    };
    let key_body = TEST_APPLE_PRIVATE_KEY
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .collect::<String>();
    let key_der = STANDARD.decode(key_body).expect("test apple private key");
    let key = jsonwebtoken::EncodingKey::from_rsa_der(&key_der);
    jsonwebtoken::encode(&header, &claims, &key).expect("test apple id token")
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

    let (stored_email, stored_hash): (String, String) = sqlx::query_as(
        "SELECT fiestaaa_decrypt_text(email_ciphertext) AS email, password_hash
             FROM pending_registrations",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(stored_email, email.to_lowercase());
    assert_ne!(stored_hash, password);
    assert!(stored_hash.starts_with("$argon2"));

    let verify_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/verify-email")
            .set_json(
                serde_json::json!({ "token": overwrite_pending_token_for(&pool, email).await? }),
            )
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
                "token": overwrite_pending_token_for(&pool, email).await?,
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
        sqlx::query_as(
            "SELECT fiestaaa_decrypt_text(email_ciphertext) AS email, password_hash, handle
             FROM users",
        )
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
async fn oauth_google_id_token_creates_user_and_identity() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping auth tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(
        &pool,
        &["oauth_identities", "pending_registrations", "users"],
    )
    .await?;

    let oauth_server = spawn_oauth_mock_server()?;
    let state = build_state_with_oauth_config(
        pool.clone(),
        "secret",
        &[],
        oauth_test_config(&oauth_server.base_url),
    );
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/oauth/google")
            .set_json(serde_json::json!({ "idToken": "valid-google-id-token" }))
            .to_request(),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(
        body.get("email").and_then(|value| value.as_str()),
        Some("google.user@example.com")
    );
    assert!(
        body.get("token")
            .and_then(|value| value.as_str())
            .is_some_and(|token| !token.is_empty())
    );

    let user_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
        .fetch_one(&pool)
        .await?;
    assert_eq!(user_count.0, 1);
    let (provider, subject): (String, String) = sqlx::query_as(
        "SELECT provider, fiestaaa_decrypt_text(provider_subject_ciphertext)
         FROM oauth_identities",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(provider, "google");
    assert_eq!(subject, "google-id-subject");

    oauth_server.stop().await;
    Ok(())
}

#[tokio::test]
async fn oauth_google_access_token_uses_userinfo_email() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping auth tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(
        &pool,
        &["oauth_identities", "pending_registrations", "users"],
    )
    .await?;

    let oauth_server = spawn_oauth_mock_server()?;
    let state = build_state_with_oauth_config(
        pool.clone(),
        "secret",
        &[],
        oauth_test_config(&oauth_server.base_url),
    );
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/oauth/google")
            .set_json(serde_json::json!({
                "accessToken": "valid-google-access-token"
            }))
            .to_request(),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(
        body.get("email").and_then(|value| value.as_str()),
        Some("access.user@example.com")
    );

    let (provider, subject): (String, String) = sqlx::query_as(
        "SELECT provider, fiestaaa_decrypt_text(provider_subject_ciphertext)
         FROM oauth_identities",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(provider, "google");
    assert_eq!(subject, "google-access-subject");

    oauth_server.stop().await;
    Ok(())
}

#[tokio::test]
async fn oauth_google_rejects_audience_mismatch() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping auth tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(
        &pool,
        &["oauth_identities", "pending_registrations", "users"],
    )
    .await?;

    let oauth_server = spawn_oauth_mock_server()?;
    let state = build_state_with_oauth_config(
        pool.clone(),
        "secret",
        &[],
        oauth_test_config(&oauth_server.base_url),
    );
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/oauth/google")
            .set_json(serde_json::json!({ "idToken": "wrong-aud-google-id-token" }))
            .to_request(),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(
        body.get("error").and_then(|value| value.as_str()),
        Some("invalid_token")
    );
    let user_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
        .fetch_one(&pool)
        .await?;
    assert_eq!(user_count.0, 0);

    oauth_server.stop().await;
    Ok(())
}

#[tokio::test]
async fn oauth_apple_reuses_identity_without_email() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping auth tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(
        &pool,
        &["oauth_identities", "pending_registrations", "users"],
    )
    .await?;

    let oauth_server = spawn_oauth_mock_server()?;
    let state = build_state_with_oauth_config(
        pool.clone(),
        "secret",
        &[],
        oauth_test_config(&oauth_server.base_url),
    );
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let first_token = apple_id_token(
        TEST_APPLE_SERVICE_ID,
        "apple-stable-subject",
        Some("Apple.User@Example.com"),
    );
    let first_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/oauth/apple")
            .set_json(serde_json::json!({ "idToken": first_token }))
            .to_request(),
    )
    .await;
    assert_eq!(first_resp.status(), StatusCode::OK);
    let first_body: Value = test::read_body_json(first_resp).await;
    assert_eq!(
        first_body.get("email").and_then(|value| value.as_str()),
        Some("apple.user@example.com")
    );
    let public_id = first_body
        .get("public_id")
        .and_then(|value| value.as_str())
        .expect("public_id")
        .to_owned();

    let second_token = apple_id_token(TEST_APPLE_SERVICE_ID, "apple-stable-subject", None);
    let second_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/oauth/apple")
            .set_json(serde_json::json!({ "idToken": second_token }))
            .to_request(),
    )
    .await;
    assert_eq!(second_resp.status(), StatusCode::OK);
    let second_body: Value = test::read_body_json(second_resp).await;
    assert_eq!(
        second_body
            .get("public_id")
            .and_then(|value| value.as_str()),
        Some(public_id.as_str())
    );
    assert_eq!(
        second_body.get("email").and_then(|value| value.as_str()),
        Some("apple.user@example.com")
    );

    let user_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
        .fetch_one(&pool)
        .await?;
    assert_eq!(user_count.0, 1);
    let (provider, subject): (String, String) = sqlx::query_as(
        "SELECT provider, fiestaaa_decrypt_text(provider_subject_ciphertext)
         FROM oauth_identities",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(provider, "apple");
    assert_eq!(subject, "apple-stable-subject");

    oauth_server.stop().await;
    Ok(())
}

#[tokio::test]
async fn oauth_apple_rejects_audience_mismatch() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping auth tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(
        &pool,
        &["oauth_identities", "pending_registrations", "users"],
    )
    .await?;

    let oauth_server = spawn_oauth_mock_server()?;
    let state = build_state_with_oauth_config(
        pool.clone(),
        "secret",
        &[],
        oauth_test_config(&oauth_server.base_url),
    );
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let token = apple_id_token(
        "wrong.service.id",
        "apple-wrong-audience",
        Some("wrong@example.com"),
    );
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/oauth/apple")
            .set_json(serde_json::json!({ "idToken": token }))
            .to_request(),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(
        body.get("error").and_then(|value| value.as_str()),
        Some("invalid_token")
    );
    let user_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
        .fetch_one(&pool)
        .await?;
    assert_eq!(user_count.0, 0);

    oauth_server.stop().await;
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
    sqlx::query(
        "INSERT INTO users (email_ciphertext, email_lookup_hash, password_hash, handle)
         VALUES (fiestaaa_encrypt_text($1), fiestaaa_email_lookup($1), $2, $3)",
    )
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

    let pending_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*)
             FROM pending_registrations
             WHERE fiestaaa_email_matches(email_lookup_hash, $1)",
    )
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
    let first_token_hash = pending_token_hash_for(&pool, email).await?;

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

    let second_token_hash = pending_token_hash_for(&pool, email).await?;
    assert_eq!(second_token_hash, first_token_hash);

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
async fn delete_account_returns_401_for_missing_user() -> Result<(), Box<dyn Error>> {
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

    sqlx::query("DELETE FROM users WHERE fiestaaa_email_matches(email_lookup_hash, $1)")
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
    assert_eq!(delete_resp.status(), StatusCode::UNAUTHORIZED);
    let delete_body: Value = test::read_body_json(delete_resp).await;
    assert_eq!(
        delete_body.get("error").and_then(|value| value.as_str()),
        Some("user_not_found")
    );

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

    sqlx::query("DELETE FROM users WHERE fiestaaa_email_matches(email_lookup_hash, $1)")
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
