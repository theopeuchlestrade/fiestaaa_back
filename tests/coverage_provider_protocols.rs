mod common;
#[path = "common/coverage_http.rs"]
mod coverage_http;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use fiestaaa_back::{
    apple::{self, AppleConfig},
    auth::{hash_password, now_ts},
    models::AppleClaims,
    notifications::{
        DeliveryOutcome, NotificationMessage, NotificationService, NotificationTarget,
    },
};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use rsa::{
    RsaPrivateKey, RsaPublicKey,
    pkcs8::{EncodePrivateKey, LineEnding},
    rand_core::OsRng,
    traits::PublicKeyParts,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::{error::Error, path::PathBuf, sync::Arc};
use uuid::Uuid;
struct FixtureFiles(PathBuf);
impl FixtureFiles {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("fiestaaa-provider-fixture-{}", Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for FixtureFiles {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
async fn user(pool: &PgPool) -> i64 {
    sqlx::query_scalar("INSERT INTO users(email_ciphertext,email_lookup_hash,password_hash,handle) VALUES(fiestaaa_encrypt_text('provider@example.invalid'),fiestaaa_email_lookup('provider@example.invalid'),$1,'providerqa') RETURNING id").bind(hash_password("ProviderFixture1!").unwrap()).fetch_one(pool).await.unwrap()
}
async fn device(pool: &PgPool, id: i64, token: &str) {
    sqlx::query("INSERT INTO user_devices(user_id,fcm_token_ciphertext,fcm_token_lookup_hash,platform) VALUES($1,fiestaaa_encrypt_text($2),fiestaaa_lookup_text($2),'ios')").bind(id).bind(token).execute(pool).await.unwrap();
}
fn signing_key() -> (String, Value) {
    let private = RsaPrivateKey::new(&mut OsRng, 2048).unwrap();
    let public = RsaPublicKey::from(&private);
    let pem = private.to_pkcs8_pem(LineEnding::LF).unwrap().to_string();
    let jwks = json!({"keys":[{"kty":"RSA","kid":"fixture-kid","alg":"RS256","use":"sig","n":URL_SAFE_NO_PAD.encode(public.n().to_bytes_be()),"e":URL_SAFE_NO_PAD.encode(public.e().to_bytes_be())}]});
    (pem, jwks)
}
fn identity(pem: &str, sub: &str, aud: &str) -> String {
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some("fixture-kid".into());
    jsonwebtoken::encode(&header,&json!({"sub":sub,"aud":aud,"iss":"https://appleid.apple.com","exp":now_ts()+3600,"email":"provider@example.invalid","email_verified":true}),&EncodingKey::from_rsa_pem(pem.as_bytes()).unwrap()).unwrap()
}

#[actix_web::test]
async fn legacy_push_preserves_http_acknowledgement_and_retires_rejected_tokens()
-> Result<(), Box<dyn Error>> {
    let _ = env_logger::Builder::new()
        .filter_module("fiestaaa_back::notifications", log::LevelFilter::Info)
        .is_test(true)
        .try_init();
    let pool = common::obtain_pool()
        .await
        .expect("isolated database required");
    let _lock = common::DB_LOCK.lock().await;
    common::reset_tables(&pool, &["users"]).await?;
    let id = user(&pool).await;
    device(&pool, id, "fixture-device").await;
    let server = coverage_http::MockServer::start();
    let service = NotificationService::new(
        Some("fixture-key".into()),
        None,
        None,
        None,
        fiestaaa_back::build_http_client("loopback-fixture"),
        30,
    )
    .with_test_endpoints(
        format!("{}/legacy", server.base),
        format!("{}/v1", server.base),
    );
    let message = NotificationMessage {
        title: "Fixture",
        body: "Fictional",
        data: json!({"type":"event_updated","event_id":1}),
    };
    let target = || NotificationTarget {
        tokens: vec!["fixture-device".into()],
        dedup_key: None,
        dedup_ttl: None,
    };
    assert!(
        service
            .send_to_tokens(
                &pool,
                NotificationTarget {
                    tokens: vec![],
                    dedup_key: None,
                    dedup_ttl: None
                },
                &message
            )
            .await
    );
    assert!(server.requests().is_empty());
    assert_eq!(
        service
            .send_to_tokens_diagnosed(
                &pool,
                NotificationTarget {
                    tokens: vec![],
                    dedup_key: None,
                    dedup_ttl: None
                },
                &message
            )
            .await,
        DeliveryOutcome::NoActiveDevice
    );
    for (status, body) in [
        (
            200,
            json!({"success":1,"failure":0,"results":[{"message_id":"fixture-message"}]}),
        ),
        (
            503,
            json!({"success":0,"failure":1,"results":[{"error":"Unavailable"}]}),
        ),
        (200, json!("malformed-provider-body")),
        (
            200,
            json!({"success":0,"failure":1,"results":[{"error":"NotRegistered"}]}),
        ),
    ] {
        server.response("/legacy", status, body.clone());
        let outcome = service
            .send_to_tokens_diagnosed(&pool, target(), &message)
            .await;
        let expected = if status != 200 {
            DeliveryOutcome::ProviderRejected
        } else if body["success"] == 1 {
            DeliveryOutcome::ProviderAccepted
        } else {
            DeliveryOutcome::ProviderAcknowledged
        };
        assert_eq!(outcome, expected);
    }
    let disabled: bool =
        sqlx::query_scalar("SELECT disabled_at IS NOT NULL FROM user_devices WHERE user_id=$1")
            .bind(id)
            .fetch_one(&pool)
            .await?;
    assert!(disabled);
    let requests = server.requests();
    assert_eq!(requests.len(), 4);
    let payload: Value = serde_json::from_slice(&requests[0].1)?;
    assert_eq!(payload["registration_ids"], json!(["fixture-device"]));
    assert_eq!(payload["data"]["type"], "event_updated");
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let unavailable = listener.local_addr()?;
    drop(listener);
    let failed = service.clone().with_test_endpoints(
        format!("http://{unavailable}/legacy"),
        format!("{}/v1", server.base),
    );
    assert!(!failed.send_to_tokens(&pool, target(), &message).await);
    server.stop().await;
    Ok(())
}

#[actix_web::test]
async fn fcm_v1_uses_local_oauth_and_validates_provider_errors_and_token_retirement()
-> Result<(), Box<dyn Error>> {
    let _ = env_logger::Builder::new()
        .filter_module("fiestaaa_back::notifications", log::LevelFilter::Info)
        .is_test(true)
        .try_init();
    let pool = common::obtain_pool()
        .await
        .expect("isolated database required");
    let _lock = common::DB_LOCK.lock().await;
    common::reset_tables(&pool, &["users"]).await?;
    let id = user(&pool).await;
    device(&pool, id, "fixture-v1-device").await;
    let server = coverage_http::MockServer::start();
    server.response(
        "/oauth-token",
        200,
        json!({"access_token":"fixture-access-token","expires_in":3600,"token_type":"Bearer"}),
    );
    let (pem, _) = signing_key();
    let files = FixtureFiles::new();
    let path = files.0.join("service-account.json");
    std::fs::write(
        &path,
        serde_json::to_vec(
            &json!({"type":"service_account","project_id":"fixture-project","private_key_id":"fixture-key","private_key":pem,"client_email":"fixture@example.invalid","client_id":"fixture-client","token_uri":format!("{}/oauth-token",server.base)}),
        )?,
    )?;
    let service = NotificationService::new(
        None,
        Some(path.to_string_lossy().into_owned()),
        None,
        None,
        fiestaaa_back::build_http_client("loopback-fixture"),
        30,
    )
    .with_test_endpoints(
        format!("{}/legacy", server.base),
        format!("{}/v1", server.base),
    );
    assert!(service.is_enabled());
    let message = NotificationMessage {
        title: "Fixture",
        body: "Fictional",
        data: json!({"type":"invite_received","event_id":7}),
    };
    let target = || NotificationTarget {
        tokens: vec!["fixture-v1-device".into()],
        dedup_key: None,
        dedup_ttl: None,
    };
    let url = "/v1/projects/fixture-project/messages:send";
    server.response(
        url,
        200,
        json!({"name":"projects/fixture-project/messages/fixture"}),
    );
    assert_eq!(
        service
            .send_to_tokens_diagnosed(&pool, target(), &message)
            .await,
        DeliveryOutcome::ProviderAccepted
    );
    let calls = server.requests();
    assert!(calls.iter().any(|r| r.0 == "/oauth-token"));
    let body: Value = serde_json::from_slice(&calls.iter().find(|r| r.0 == url).unwrap().1)?;
    assert_eq!(body["message"]["token"], "fixture-v1-device");
    assert_eq!(body["message"]["data"]["event_id"], "7");
    server.response(url, 503, json!({"error":{"status":"UNAVAILABLE"}}));
    assert_eq!(
        service
            .send_to_tokens_diagnosed(&pool, target(), &message)
            .await,
        DeliveryOutcome::ProviderRejected
    );
    server.response(
        url,
        404,
        json!({"error":{"status":"NOT_FOUND","details":[{"errorCode":"UNREGISTERED"}]}}),
    );
    assert_eq!(
        service
            .send_to_tokens_diagnosed(&pool, target(), &message)
            .await,
        DeliveryOutcome::InvalidDevice,
        "permanent retirement is distinct from provider acceptance"
    );
    let disabled: bool =
        sqlx::query_scalar("SELECT disabled_at IS NOT NULL FROM user_devices WHERE user_id=$1")
            .bind(id)
            .fetch_one(&pool)
            .await?;
    assert!(disabled);
    let invalid = NotificationService::new(
        None,
        Some(files.0.join("missing.json").to_string_lossy().into_owned()),
        None,
        None,
        fiestaaa_back::build_http_client("loopback-fixture"),
        30,
    );
    assert!(!invalid.is_enabled());
    assert_eq!(
        invalid
            .send_to_tokens_diagnosed(&pool, target(), &message)
            .await,
        DeliveryOutcome::NotConfigured
    );
    server.stop().await;
    Ok(())
}

#[actix_web::test]
async fn apple_exchange_preserves_identity_and_stores_only_encrypted_refresh_credentials()
-> Result<(), Box<dyn Error>> {
    let _ = env_logger::Builder::new()
        .filter_module("fiestaaa_back::notifications", log::LevelFilter::Info)
        .is_test(true)
        .try_init();
    let pool = common::obtain_pool()
        .await
        .expect("isolated database required");
    let _lock = common::DB_LOCK.lock().await;
    common::reset_tables(&pool, &["users", "apple_revocations"]).await?;
    let id = user(&pool).await;
    let server = coverage_http::MockServer::start();
    let (pem, jwks) = signing_key();
    server.response("/keys", 200, jwks);
    let mut state =
        Arc::try_unwrap(common::build_state(pool.clone(), "fixture-secret", &[]).into_inner())
            .ok()
            .unwrap();
    state.apple_jwks_url = format!("{}/keys", server.base);
    let original = AppleClaims {
        sub: "fixture-apple-sub".into(),
        email: Some("provider@example.invalid".into()),
        email_verified: Some(json!(true)),
        aud: "fixture.apple.native".into(),
        iss: "https://appleid.apple.com".into(),
        exp: (now_ts() + 3600) as usize,
    };
    assert_eq!(
        apple::save_code(&state, id, &original, "", false).await,
        Err("invalid_apple_code")
    );
    assert_eq!(
        apple::save_code(&state, id, &original, "code", false).await,
        Err("apple_revocation_not_configured")
    );
    assert!(server.requests().is_empty());
    let ec = std::process::Command::new("openssl")
        .args([
            "genpkey",
            "-algorithm",
            "EC",
            "-pkeyopt",
            "ec_paramgen_curve:P-256",
        ])
        .output()?;
    assert!(ec.status.success(), "generate ephemeral test EC key");
    let mut config = AppleConfig::default().with_test_endpoints(
        format!("{}/token", server.base),
        format!("{}/revoke", server.base),
    );
    config.team_id = "fixture-team".into();
    config.key_id = "fixture-key".into();
    config.private_key = String::from_utf8(ec.stdout)?;
    state.apple_config = config;
    server.response("/token", 400, json!({"error":"invalid_grant"}));
    assert_eq!(
        apple::save_code(&state, id, &original, "code", false).await,
        Err("apple_exchange_failed")
    );
    for encoded in [
        "malformed".to_string(),
        identity(&pem, "wrong-sub", &original.aud),
        identity(&pem, &original.sub, "wrong-audience"),
    ] {
        server.response(
            "/token",
            200,
            json!({"id_token":encoded,"refresh_token":"fixture-refresh"}),
        );
        assert_eq!(
            apple::save_code(&state, id, &original, "code", false).await,
            Err("invalid_apple_code")
        );
    }
    let valid = identity(&pem, &original.sub, &original.aud);
    server.response("/token", 200, json!({"id_token":valid}));
    assert_eq!(
        apple::save_code(&state, id, &original, "code", false).await,
        Err("apple_exchange_failed")
    );
    server.response(
        "/token",
        200,
        json!({"id_token":valid,"refresh_token":"fixture-refresh"}),
    );
    assert_eq!(
        apple::save_code(&state, id, &original, "code", false).await,
        Ok(())
    );
    let(ciphertext,plain):(Vec<u8>,String)=sqlx::query_as("SELECT refresh_token_ciphertext,fiestaaa_decrypt_text(refresh_token_ciphertext) FROM apple_credentials WHERE user_id=$1").bind(id).fetch_one(&pool).await?;
    assert_ne!(ciphertext.as_slice(), b"fixture-refresh");
    assert_eq!(plain, "fixture-refresh");
    let body = server
        .requests()
        .into_iter()
        .find(|r| r.0 == "/token")
        .unwrap()
        .1;
    let form: std::collections::HashMap<String, String> = serde_urlencoded::from_bytes(&body)?;
    assert_eq!(form["grant_type"], "authorization_code");
    assert_eq!(form["client_id"], original.aud);
    assert!(!form.contains_key("redirect_uri"));
    state.apple_app_id = Some(original.aud.clone());
    let bearer = fiestaaa_back::auth::encode_jwt(
        &fiestaaa_back::models::Claims {
            sub: "provider@example.invalid".into(),
            handle: "providerqa".into(),
            exp: (now_ts() + 3600) as usize,
            session_version: 0,
        },
        "fixture-secret",
    )?;
    let app = actix_web::test::init_service(
        actix_web::App::new()
            .app_data(actix_web::web::Data::new(state))
            .configure(fiestaaa_back::routes::configure),
    )
    .await;
    for (payload, status) in [
        (json!({"provider":"apple"}), 400),
        (json!({"provider":"apple","id_token":"malformed"}), 400),
        (
            json!({"provider":"apple","id_token":"malformed","authorization_code":"code"}),
            401,
        ),
        (
            json!({"provider":"apple","id_token":valid,"authorization_code":"code"}),
            401,
        ),
    ] {
        let response = actix_web::test::call_service(
            &app,
            actix_web::test::TestRequest::post()
                .uri("/me/apple-reauthorize")
                .insert_header(("Authorization", format!("Bearer {bearer}")))
                .set_json(payload)
                .to_request(),
        )
        .await;
        assert_eq!(response.status().as_u16(), status);
    }
    sqlx::query("INSERT INTO oauth_identities(provider,provider_subject_ciphertext,provider_subject_lookup_hash,user_id) VALUES('apple',fiestaaa_encrypt_text($1),fiestaaa_lookup_text($1),$2)").bind(&original.sub).bind(id).execute(&pool).await?;
    let response = actix_web::test::call_service(
        &app,
        actix_web::test::TestRequest::post()
            .uri("/me/apple-reauthorize")
            .insert_header(("Authorization", format!("Bearer {bearer}")))
            .set_json(json!({"provider":"apple","id_token":valid,"authorization_code":"code"}))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), actix_web::http::StatusCode::OK);
    let body: Value = actix_web::test::read_body_json(response).await;
    assert_eq!(body["status"], "apple_reauthorized");
    server.stop().await;
    Ok(())
}

#[actix_web::test]
async fn apple_revocation_is_retryable_and_never_requires_the_deleted_user()
-> Result<(), Box<dyn Error>> {
    let _ = env_logger::Builder::new()
        .filter_module("fiestaaa_back::notifications", log::LevelFilter::Info)
        .is_test(true)
        .try_init();
    let pool = common::obtain_pool()
        .await
        .expect("isolated database required");
    let _lock = common::DB_LOCK.lock().await;
    common::reset_tables(&pool, &["users", "apple_revocations"]).await?;
    let server = coverage_http::MockServer::start();
    let mut state =
        Arc::try_unwrap(common::build_state(pool.clone(), "fixture-secret", &[]).into_inner())
            .ok()
            .unwrap();
    let ec = std::process::Command::new("openssl")
        .args([
            "genpkey",
            "-algorithm",
            "EC",
            "-pkeyopt",
            "ec_paramgen_curve:P-256",
        ])
        .output()?;
    assert!(ec.status.success());
    let mut config = AppleConfig::default().with_test_endpoints(
        format!("{}/token", server.base),
        format!("{}/revoke", server.base),
    );
    config.team_id = "fixture-team".into();
    config.key_id = "fixture-key".into();
    config.private_key = String::from_utf8(ec.stdout)?;
    state.apple_config = config;
    apple::revoke_next(&state).await?;
    assert!(server.requests().is_empty());
    sqlx::query("INSERT INTO apple_revocations(client_id,refresh_token_ciphertext) VALUES('fixture.apple.native',fiestaaa_encrypt_text('fixture-refresh'))").execute(&pool).await?;
    server.response("/revoke", 503, json!({"error":"unavailable"}));
    apple::revoke_next(&state).await?;
    let (attempts, future): (i32, bool) =
        sqlx::query_as("SELECT attempts,next_attempt_at>NOW() FROM apple_revocations")
            .fetch_one(&pool)
            .await?;
    assert_eq!(attempts, 1);
    assert!(future);
    sqlx::query("UPDATE apple_revocations SET next_attempt_at=NOW()")
        .execute(&pool)
        .await?;
    server.response("/revoke", 200, json!({}));
    apple::revoke_next(&state).await?;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM apple_revocations")
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 0);
    assert_eq!(server.requests().len(), 2);
    server.stop().await;
    Ok(())
}
