mod common;

use std::{
    error::Error,
    io::Write,
    time::{SystemTime, UNIX_EPOCH},
};

use actix_web::{App, http::StatusCode, test};
use common::{DB_LOCK, build_state, build_state_with_avatar_storage, obtain_pool, reset_tables};
use fiestaaa_back::{
    auth::{encode_jwt, hash_password, now_ts},
    models::{Claims, HandleAvailabilityResponse, HandleUpdatePayload},
    observability, routes, user_metrics,
};
use serde::Deserialize;
use sqlx::PgPool;

#[derive(Debug, Deserialize)]
struct TestMeResponse {
    email: String,
    handle: String,
    avatar_url: Option<String>,
    exp: usize,
}

#[derive(Debug, Deserialize)]
struct TestHealthResponse {
    status: String,
    db: Option<String>,
}

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

fn test_png_avatar() -> Result<Vec<u8>, image::ImageError> {
    let img = image::RgbImage::from_pixel(2, 2, image::Rgb([240, 80, 120]));
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgb8(img).write_to(
        &mut std::io::Cursor::new(&mut bytes),
        image::ImageFormat::Png,
    )?;
    Ok(bytes)
}

fn multipart_avatar_body(boundary: &str, avatar_bytes: &[u8]) -> Result<Vec<u8>, std::io::Error> {
    let mut body = Vec::new();
    write!(body, "--{boundary}\r\n")?;
    write!(
        body,
        "Content-Disposition: form-data; name=\"avatar\"; filename=\"avatar.png\"\r\n"
    )?;
    write!(body, "Content-Type: image/png\r\n\r\n")?;
    body.extend_from_slice(avatar_bytes);
    write!(body, "\r\n--{boundary}--\r\n")?;
    Ok(body)
}

#[tokio::test]
async fn health_returns_ok_when_database_is_reachable() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping root/health/users tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["users"]).await?;

    let state = build_state(pool, "secret", &[]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let resp = test::call_service(&app, test::TestRequest::get().uri("/health").to_request()).await;

    assert_eq!(resp.status(), StatusCode::OK);
    let body: TestHealthResponse = test::read_body_json(resp).await;
    assert_eq!(body.status, "ok");
    assert_eq!(body.db, None);
    Ok(())
}

#[tokio::test]
async fn metrics_endpoint_requires_bearer_token() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping metrics test: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["users"]).await?;

    let state = build_state(pool, "secret", &[]);
    let app = test::init_service(
        App::new()
            .wrap(observability::MetricsMiddleware)
            .app_data(state)
            .configure(routes::configure),
    )
    .await;

    let unauthorized =
        test::call_service(&app, test::TestRequest::get().uri("/metrics").to_request()).await;
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let authorized = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/metrics")
            .insert_header(("Authorization", "Bearer test-metrics-token"))
            .to_request(),
    )
    .await;
    assert_eq!(authorized.status(), StatusCode::OK);
    let body = test::read_body(authorized).await;
    assert!(String::from_utf8_lossy(&body).contains("fiestaaa_http_requests_total"));
    Ok(())
}

#[tokio::test]
async fn metrics_endpoint_exposes_user_metrics() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping user metrics test: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(
        &pool,
        &[
            "user_devices",
            "oauth_identities",
            "pending_registrations",
            "users",
        ],
    )
    .await?;

    let user_id = seed_user(&pool, "metrics-owner@example.com", "metrics_owner").await?;
    sqlx::query(
        "INSERT INTO user_devices (user_id, fcm_token_ciphertext, fcm_token_lookup_hash, platform)
         VALUES ($1, fiestaaa_encrypt_text($2), fiestaaa_lookup_text($2), 'ios')",
    )
    .bind(user_id)
    .bind("metrics-device-token")
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO oauth_identities (
            provider,
            provider_subject_ciphertext,
            provider_subject_lookup_hash,
            user_id
         )
         VALUES ('google', fiestaaa_encrypt_text($1), fiestaaa_lookup_text($1), $2)",
    )
    .bind("metrics-google-subject")
    .bind(user_id)
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO pending_registrations (
            email_ciphertext,
            email_lookup_hash,
            password_hash,
            handle,
            verification_token_hash,
            verification_expires_at
         )
         VALUES (
            fiestaaa_encrypt_text($1),
            fiestaaa_email_lookup($1),
            $2,
            $3,
            $4,
            NOW() + INTERVAL '1 day'
         )",
    )
    .bind("metrics-pending@example.com")
    .bind(hash_password("StrongPassw0rd!").expect("hash"))
    .bind("metrics_pending")
    .bind("metrics-verification-token")
    .execute(&pool)
    .await?;

    user_metrics::refresh_user_metrics(&pool).await?;

    let state = build_state(pool, "secret", &[]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/metrics")
            .insert_header(("Authorization", "Bearer test-metrics-token"))
            .to_request(),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let body = String::from_utf8_lossy(&test::read_body(resp).await).into_owned();
    assert!(body.contains("fiestaaa_users_registered 1"));
    assert!(body.contains("fiestaaa_pending_registrations 1"));
    assert!(body.contains("fiestaaa_users_active_window"));
    assert!(body.contains("source=\"any\""));
    assert!(body.contains("fiestaaa_users_with_active_device"));
    assert!(body.contains("platform=\"ios\""));
    assert!(body.contains("fiestaaa_users_oauth_linked"));
    assert!(body.contains("provider=\"google\""));
    Ok(())
}

#[tokio::test]
async fn me_returns_authenticated_profile() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping root/health/users tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["users"]).await?;

    let secret = "secret";
    seed_user(&pool, "owner@example.com", "owner_handle").await?;
    sqlx::query(
        "UPDATE users SET avatar_url = $1 WHERE fiestaaa_email_matches(email_lookup_hash, $2)",
    )
    .bind("https://cdn.example.com/avatar.jpg")
    .bind("owner@example.com")
    .execute(&pool)
    .await?;

    let state = build_state(pool, secret, &[]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;
    let token = make_token(secret, "owner@example.com", "owner_handle").expect("token");

    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/me")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .to_request(),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let body: TestMeResponse = test::read_body_json(resp).await;
    assert_eq!(body.email, "owner@example.com");
    assert_eq!(body.handle, "owner_handle");
    assert_eq!(
        body.avatar_url.as_deref(),
        Some("https://cdn.example.com/avatar.jpg")
    );
    assert!(body.exp > now_ts() as usize);
    Ok(())
}

#[tokio::test]
async fn handle_availability_and_update_cover_validation_and_conflicts()
-> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping root/health/users tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["users"]).await?;

    let secret = "secret";
    seed_user(&pool, "owner@example.com", "owner_handle").await?;
    seed_user(&pool, "taken@example.com", "taken_handle").await?;

    let state = build_state(pool.clone(), secret, &[]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;
    let token = make_token(secret, "owner@example.com", "owner_handle").expect("token");

    let availability_resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/handles/availability?handle=Fresh_Handle")
            .to_request(),
    )
    .await;
    assert_eq!(availability_resp.status(), StatusCode::OK);
    let availability: HandleAvailabilityResponse = test::read_body_json(availability_resp).await;
    assert!(availability.available);

    let invalid_resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/handles/availability?handle=??")
            .to_request(),
    )
    .await;
    assert_eq!(invalid_resp.status(), StatusCode::BAD_REQUEST);

    let conflict_resp = test::call_service(
        &app,
        test::TestRequest::patch()
            .uri("/me/handle")
            .insert_header(("Authorization", format!("Bearer {}", token.clone())))
            .set_json(&HandleUpdatePayload {
                handle: "taken_handle".into(),
            })
            .to_request(),
    )
    .await;
    assert_eq!(conflict_resp.status(), StatusCode::CONFLICT);

    let updated_resp = test::call_service(
        &app,
        test::TestRequest::patch()
            .uri("/me/handle")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .set_json(&HandleUpdatePayload {
                handle: "Fresh_Handle".into(),
            })
            .to_request(),
    )
    .await;
    assert_eq!(updated_resp.status(), StatusCode::OK);
    let updated: TestMeResponse = test::read_body_json(updated_resp).await;
    assert_eq!(updated.handle, "fresh_handle");

    let stored: (String,) = sqlx::query_as(
        "SELECT handle FROM users WHERE fiestaaa_email_matches(email_lookup_hash, $1)",
    )
    .bind("owner@example.com")
    .fetch_one(&pool)
    .await?;
    assert_eq!(stored.0, "fresh_handle");
    Ok(())
}

#[tokio::test]
async fn upload_avatar_accepts_multipart_image_and_returns_public_url() -> Result<(), Box<dyn Error>>
{
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping root/health/users tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["users"]).await?;

    let secret = "secret";
    seed_user(&pool, "owner@example.com", "owner_handle").await?;
    let token = make_token(secret, "owner@example.com", "owner_handle").expect("token");

    let suffix = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let avatar_root = std::env::temp_dir().join(format!("fiestaaa-avatar-test-{suffix}"));
    let avatar_upload_dir = avatar_root.join("avatars");
    let avatar_base_url = "https://api.example.test/media/avatars";
    let state = build_state_with_avatar_storage(
        pool,
        secret,
        &[],
        avatar_upload_dir.to_string_lossy().into_owned(),
        avatar_base_url.into(),
    );
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let boundary = "fiestaaa-test-boundary";
    let body = multipart_avatar_body(boundary, &test_png_avatar()?)?;
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/me/avatar")
            .insert_header(("Authorization", format!("Bearer {token}")))
            .insert_header((
                "Content-Type",
                format!("multipart/form-data; boundary={boundary}"),
            ))
            .set_payload(body)
            .to_request(),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let body: TestMeResponse = test::read_body_json(resp).await;
    let avatar_url = body.avatar_url.expect("avatar url");
    let expected_prefix = format!("{avatar_base_url}/");
    assert!(avatar_url.starts_with(&expected_prefix));
    assert!(avatar_url.ends_with(".jpg"));

    let filename = avatar_url
        .strip_prefix(&expected_prefix)
        .expect("avatar filename");
    assert!(avatar_upload_dir.join(filename).is_file());
    let _ = tokio::fs::remove_dir_all(avatar_root).await;
    Ok(())
}
