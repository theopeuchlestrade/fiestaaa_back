mod common;
#[path = "common/coverage_http.rs"]
mod coverage_http;

use std::error::Error;

use actix_web::{App, http::StatusCode, test};
use common::{DB_LOCK, build_state, obtain_pool, reset_tables};
use fiestaaa_back::{
    auth::{encode_jwt, hash_password, now_ts},
    models::{Claims, DeviceDeletePayload, DeviceRefreshPayload, DeviceRegisterPayload},
    notifications::{NotificationRequest, notify_users},
    routes,
};
use serde_json::Value;
use serde_json::json;
use sqlx::PgPool;

fn make_token(secret: &str, email: &str, handle: &str) -> Option<String> {
    let claims = Claims {
        session_version: 0,
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

#[tokio::test]
async fn device_registration_refresh_and_delete_flow() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping notifications tests: FIESTAAA_SKIP_DB_TESTS=1");
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
         WHERE user_id = $1
           AND fiestaaa_lookup_matches(fcm_token_lookup_hash, $2)",
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

    let old_disabled: (bool,) = sqlx::query_as(
        "SELECT disabled_at IS NOT NULL
             FROM user_devices
             WHERE fiestaaa_lookup_matches(fcm_token_lookup_hash, $1)",
    )
    .bind("device-token-1")
    .fetch_one(&pool)
    .await?;
    assert!(old_disabled.0);

    let refreshed: (String, Option<String>, Option<String>, bool) = sqlx::query_as(
        "SELECT platform, locale, app_version, disabled_at IS NULL
         FROM user_devices
         WHERE user_id = $1
           AND fiestaaa_lookup_matches(fcm_token_lookup_hash, $2)",
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
        test::TestRequest::post()
            .uri("/me/devices/revoke")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .set_json(&DeviceDeletePayload {
                token: "device-token-2".into(),
            })
            .to_request(),
    )
    .await;
    assert_eq!(delete_resp.status(), StatusCode::OK);

    let deleted: (bool,) = sqlx::query_as(
        "SELECT disabled_at IS NOT NULL
             FROM user_devices
             WHERE fiestaaa_lookup_matches(fcm_token_lookup_hash, $1)",
    )
    .bind("device-token-2")
    .fetch_one(&pool)
    .await?;
    assert!(deleted.0);
    Ok(())
}

#[tokio::test]
async fn device_registration_rejects_invalid_payloads() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping notifications tests: FIESTAAA_SKIP_DB_TESTS=1");
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

#[tokio::test]
async fn outbox_deduplication_expires_and_reuses_the_key() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping notifications tests: FIESTAAA_SKIP_DB_TESTS=1");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["notification_outbox", "users"]).await?;

    let user_id = seed_user(&pool, "owner@example.com", "owner_handle").await?;
    let state = build_state(pool.clone(), "secret", &[]);
    let enqueue = |title: &'static str| NotificationRequest {
        title,
        body: "body",
        data: json!({"type": "event_updated"}),
        dedup_base_key: Some("event_updated:42"),
        dedup_ttl: Some(60),
    };

    notify_users(&state.notifications, &pool, &[user_id], enqueue("first")).await;
    notify_users(
        &state.notifications,
        &pool,
        &[user_id],
        enqueue("duplicate"),
    )
    .await;

    let first: (i64, String, bool) = sqlx::query_as(
        "SELECT COUNT(*), MAX(title), bool_and(dedup_expires_at > NOW())
         FROM notification_outbox",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(first.0, 1);
    assert_eq!(first.1, "first");
    assert!(first.2);

    sqlx::query("UPDATE notification_outbox SET dedup_expires_at = NOW() - INTERVAL '1 second'")
        .execute(&pool)
        .await?;
    notify_users(
        &state.notifications,
        &pool,
        &[user_id],
        enqueue("after-expiry"),
    )
    .await;

    let after_expiry: (i64, String, String, i32) = sqlx::query_as(
        "SELECT COUNT(*), MAX(title), MAX(status), MAX(attempts)
         FROM notification_outbox",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(
        after_expiry,
        (1, "after-expiry".into(), "pending".into(), 0)
    );
    Ok(())
}

struct DiagnosticLogger(std::sync::Mutex<Vec<String>>);
impl log::Log for DiagnosticLogger {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }
    fn log(&self, record: &log::Record<'_>) {
        if record.target().starts_with("fiestaaa_back::notifications") {
            self.0.lock().unwrap().push(record.args().to_string());
        }
    }
    fn flush(&self) {}
}
static DIAGNOSTICS: DiagnosticLogger = DiagnosticLogger(std::sync::Mutex::new(Vec::new()));

#[actix_web::test]
async fn invitation_delivery_diagnostics_distinguish_no_device_retry_and_provider_ack()
-> Result<(), Box<dyn Error>> {
    use fiestaaa_back::notifications::{NotificationOutboxWorker, NotificationService};
    let pool = obtain_pool().await.expect("isolated database required");
    let _guard = DB_LOCK.lock().await;
    log::set_logger(&DIAGNOSTICS).expect("test logger");
    log::set_max_level(log::LevelFilter::Info);
    reset_tables(&pool, &["users", "events", "notification_outbox"]).await?;
    let owner = seed_user(&pool, "diag-owner@example.invalid", "diagowner").await?;
    let guest = seed_user(&pool, "diag-guest@example.invalid", "diagguest").await?;
    let event: i64 = sqlx::query_scalar("INSERT INTO events(name_event,description,date_event,start_time,address_ciphertext,owner_user_id) VALUES('Fixture','Fictional',CURRENT_DATE+1,'20:00',fiestaaa_encrypt_text('Fixture'),$1) RETURNING event_id").bind(owner).fetch_one(&pool).await?;
    sqlx::query("INSERT INTO invitations(event_id,user_id,status) VALUES($1,$2,'Waiting')")
        .bind(event)
        .bind(guest)
        .execute(&pool)
        .await?;
    let server = coverage_http::MockServer::start();
    let service = NotificationService::new(
        Some("fixture-key".into()),
        None,
        None,
        None,
        fiestaaa_back::build_http_client("notification-fixture"),
        300,
    )
    .with_test_endpoints(
        format!("{}/legacy", server.base),
        format!("{}/v1", server.base),
    );
    let worker = NotificationOutboxWorker::new(pool.clone(), service);
    let request = || NotificationRequest {
        title: "private-title-sentinel",
        body: "private-body-sentinel",
        data: json!({"type":"invite_received","event_id":event}),
        dedup_base_key: None,
        dedup_ttl: None,
    };
    notify_users(
        &NotificationService::new(
            None,
            None,
            None,
            None,
            fiestaaa_back::build_http_client("fixture"),
            300,
        ),
        &pool,
        &[guest],
        request(),
    )
    .await;
    worker.process_batch().await?;
    assert!(server.requests().is_empty());
    let initial: String =
        sqlx::query_scalar("SELECT status FROM notification_outbox ORDER BY id LIMIT 1")
            .fetch_one(&pool)
            .await?;
    assert_eq!(
        initial, "sent",
        "terminal status is retained but does not prove phone presentation"
    );
    sqlx::query("INSERT INTO user_devices(user_id,fcm_token_ciphertext,fcm_token_lookup_hash,platform) VALUES($1,fiestaaa_encrypt_text('private-token-sentinel'),fiestaaa_lookup_text('private-token-sentinel'),'ios')").bind(guest).execute(&pool).await?;
    server.response(
        "/legacy",
        503,
        json!({"error":"private-token-sentinel private-body-sentinel"}),
    );
    notify_users(
        &NotificationService::new(
            None,
            None,
            None,
            None,
            fiestaaa_back::build_http_client("fixture"),
            300,
        ),
        &pool,
        &[guest],
        request(),
    )
    .await;
    worker.process_batch().await?;
    let retry:(String,i32,bool)=sqlx::query_as("SELECT status,attempts,available_at>NOW() FROM notification_outbox ORDER BY id DESC LIMIT 1").fetch_one(&pool).await?;
    assert_eq!(retry, ("retry".into(), 1, true));
    server.response(
        "/legacy",
        200,
        json!({"success":1,"failure":0,"results":[{"message_id":"fixture-ack"}]}),
    );
    sqlx::query("UPDATE notification_outbox SET available_at=NOW() WHERE status='retry'")
        .execute(&pool)
        .await?;
    worker.process_batch().await?;
    let done: (String, i32) =
        sqlx::query_as("SELECT status,attempts FROM notification_outbox ORDER BY id DESC LIMIT 1")
            .fetch_one(&pool)
            .await?;
    assert_eq!(done, ("sent".into(), 2));
    assert_eq!(server.requests().len(), 2);
    // A malformed HTTP success is an acknowledgement, not evidence of display.
    server.response("/legacy", 200, json!("private-body-sentinel"));
    notify_users(
        &NotificationService::new(
            None,
            None,
            None,
            None,
            fiestaaa_back::build_http_client("fixture"),
            300,
        ),
        &pool,
        &[guest],
        NotificationRequest {
            data: json!({"type":"private-type-sentinel"}),
            ..request()
        },
    )
    .await;
    worker.process_batch().await?;
    let logs = DIAGNOSTICS.0.lock().unwrap().join("\n");
    assert!(logs.contains("outcome=no_active_device"));
    assert!(logs.contains("outcome=provider_http_acknowledged") && logs.contains("kind=other"));
    assert!(!logs.contains("private-type-sentinel"));
    assert!(logs.contains("outcome=provider_rejected"));
    assert!(logs.contains("push_retry") && logs.contains("attempt=1"));
    assert!(logs.contains("outcome=provider_accepted") && logs.contains("attempt=2"));
    for secret in [
        "private-token-sentinel",
        "private-body-sentinel",
        "private-title-sentinel",
    ] {
        assert!(
            !logs.contains(secret),
            "diagnostics must not expose tokens or message content"
        );
    }
    server.stop().await;
    Ok(())
}
