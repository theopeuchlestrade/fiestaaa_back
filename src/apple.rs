//! Apple credentials are encrypted at rest. Revocation survives local account deletion.
use crate::{
    auth::{extract_authenticated_user, now_ts},
    models::{AppleClaims, OAuthPayload},
    state::AppState,
};
use actix_web::{HttpRequest, HttpResponse, Responder, post, web};
use jsonwebtoken::{Algorithm, EncodingKey, Header, Validation};
use serde::Serialize;
use serde_json::json;
use sqlx::Row;

#[derive(Default)]
pub struct AppleConfig {
    pub team_id: String,
    pub key_id: String,
    pub private_key: String,
    pub redirect_uri: String,
    pub android_redirect_uri: String,
}
impl AppleConfig {
    pub fn from_env() -> Self {
        Self {
            team_id: std::env::var("APPLE_TEAM_ID").unwrap_or_default(),
            key_id: std::env::var("APPLE_KEY_ID").unwrap_or_default(),
            private_key: std::env::var("APPLE_PRIVATE_KEY").unwrap_or_default(),
            redirect_uri: std::env::var("FIESTAAA_APPLE_REDIRECT_URI").unwrap_or_default(),
            android_redirect_uri: std::env::var("FIESTAAA_APPLE_ANDROID_REDIRECT_URI")
                .unwrap_or_default(),
        }
    }
    fn client_secret(&self, client_id: &str) -> Result<String, &'static str> {
        if self.team_id.is_empty() || self.key_id.is_empty() {
            return Err("apple_revocation_not_configured");
        }
        #[derive(Serialize)]
        struct Secret<'a> {
            iss: &'a str,
            iat: u64,
            exp: u64,
            aud: &'a str,
            sub: &'a str,
        }
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some(self.key_id.clone());
        let key = EncodingKey::from_ec_pem(self.private_key.as_bytes())
            .map_err(|_| "apple_revocation_not_configured")?;
        jsonwebtoken::encode(
            &header,
            &Secret {
                iss: &self.team_id,
                iat: now_ts(),
                exp: now_ts() + 300,
                aud: "https://appleid.apple.com",
                sub: client_id,
            },
            &key,
        )
        .map_err(|_| "apple_revocation_not_configured")
    }
}
pub async fn save_code(
    state: &AppState,
    user_id: i64,
    claims: &AppleClaims,
    code: &str,
    android: bool,
) -> Result<(), &'static str> {
    if code.is_empty() || code.len() > 4096 {
        return Err("invalid_apple_code");
    }
    let secret = state.apple_config.client_secret(&claims.aud)?;
    let mut form = vec![
        ("client_id", claims.aud.as_str()),
        ("client_secret", secret.as_str()),
        ("code", code),
        ("grant_type", "authorization_code"),
    ];
    if state.apple_service_id.as_deref() == Some(claims.aud.as_str()) {
        let redirect = if android {
            &state.apple_config.android_redirect_uri
        } else {
            &state.apple_config.redirect_uri
        };
        if redirect.is_empty() {
            return Err("apple_revocation_not_configured");
        }
        form.push(("redirect_uri", redirect.as_str()));
    }
    let response = state
        .http_client
        .post("https://appleid.apple.com/auth/token")
        .form(&form)
        .send()
        .await
        .map_err(|_| "apple_exchange_failed")?;
    if !response.status().is_success() {
        let status = response.status().as_u16();
        let body = response.json::<serde_json::Value>().await.ok();
        // Never log the provider response or any authorization credential.
        let reason = match body.as_ref().and_then(|v| v["error"].as_str()) {
            Some("invalid_client") => "invalid_client",
            Some("invalid_grant") => "invalid_grant",
            Some("invalid_request") => "invalid_request",
            Some("unauthorized_client") => "unauthorized_client",
            _ => "other",
        };
        log::warn!("Apple code exchange failed: status={status} reason={reason}");
        return Err("apple_exchange_failed");
    }
    let value: serde_json::Value = response.json().await.map_err(|_| "apple_exchange_failed")?;
    validate_exchanged_identity_token(
        &state.http_client,
        &state.apple_jwks_url,
        claims,
        value["id_token"].as_str().ok_or("apple_exchange_failed")?,
    )
    .await?;
    let refresh = value["refresh_token"]
        .as_str()
        .filter(|t| !t.is_empty())
        .ok_or("apple_exchange_failed")?;
    sqlx::query("INSERT INTO apple_credentials(user_id,client_id,refresh_token_ciphertext) VALUES($1,$2,fiestaaa_encrypt_text($3)) ON CONFLICT(user_id,client_id) DO UPDATE SET refresh_token_ciphertext=EXCLUDED.refresh_token_ciphertext")
        .bind(user_id).bind(&claims.aud).bind(refresh).execute(&state.db).await.map_err(|_|"db_error")?;
    Ok(())
}

async fn validate_exchanged_identity_token(
    client: &reqwest::Client,
    jwks_url: &str,
    original: &AppleClaims,
    encoded: &str,
) -> Result<(), &'static str> {
    let header = jsonwebtoken::decode_header(encoded).map_err(|_| "invalid_apple_code")?;
    if header.alg != Algorithm::RS256 {
        return Err("invalid_apple_code");
    }
    let kid = header
        .kid
        .as_deref()
        .filter(|kid| !kid.is_empty())
        .ok_or("invalid_apple_code")?;
    // Apple can use a different signing key for this newly issued token.
    // Resolve its own key from Apple's trusted JWKS, never from the token URL.
    let key = crate::routes::auth::fetch_apple_decoding_key_from(client, jwks_url, kid)
        .await
        .ok_or_else(|| {
            log::warn!("Apple exchanged signing key unavailable");
            "apple_exchange_failed"
        })?;
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_audience(&[&original.aud]);
    validation.set_issuer(&["https://appleid.apple.com"]);
    let exchanged =
        jsonwebtoken::decode::<AppleClaims>(encoded, &key, &validation).map_err(|error| {
            use jsonwebtoken::errors::ErrorKind;
            let reason = match error.kind() {
                ErrorKind::InvalidSignature => "signature",
                ErrorKind::ExpiredSignature => "expiry",
                ErrorKind::InvalidAudience => "audience",
                ErrorKind::InvalidIssuer => "issuer",
                ErrorKind::InvalidAlgorithm => "algorithm",
                _ => "other",
            };
            // Log only a fixed category, never the token, claims or library error.
            log::warn!("Apple exchange verification failed: category={reason}");
            "invalid_apple_code"
        })?;
    if exchanged.claims.sub != original.sub {
        log::warn!("Apple exchange verification failed: subject mismatch");
        return Err("invalid_apple_code");
    }
    Ok(())
}
#[utoipa::path(post,path="/me/apple-reauthorize",tag="users",request_body=OAuthPayload,
 responses((status=200,description="Apple deletion credential refreshed"),(status=401,description="Different Apple account or invalid credential"),(status=503,description="Apple unavailable")))]
#[post("/me/apple-reauthorize")]
pub async fn reauthorize(
    req: HttpRequest,
    state: web::Data<AppState>,
    payload: web::Json<OAuthPayload>,
) -> impl Responder {
    let user = match extract_authenticated_user(&req, &state.db, &state.jwt_secret).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    if let Err(r) =
        crate::routes::auth::enforce_auth_rate_limit(&req, &state, "apple-reauthorize").await
    {
        return r;
    }
    let Some(token) = payload.id_token.as_deref() else {
        return HttpResponse::BadRequest().finish();
    };
    let Some(code) = payload.authorization_code.as_deref() else {
        return HttpResponse::BadRequest().finish();
    };
    let kid = match jsonwebtoken::decode_header(token).ok().and_then(|h| h.kid) {
        Some(k) => k,
        None => return HttpResponse::Unauthorized().finish(),
    };
    let Some(key) = crate::routes::auth::fetch_apple_decoding_key(&state, &kid).await else {
        return HttpResponse::ServiceUnavailable().finish();
    };
    let allowed: Vec<&str> = [
        state.apple_app_id.as_deref(),
        state.apple_service_id.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect();
    if allowed.is_empty() {
        return HttpResponse::ServiceUnavailable().finish();
    }
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_audience(&allowed);
    validation.set_issuer(&["https://appleid.apple.com"]);
    let claims = match jsonwebtoken::decode::<AppleClaims>(token, &key, &validation) {
        Ok(c) => c.claims,
        Err(_) => return HttpResponse::Unauthorized().finish(),
    };
    let matches=sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM oauth_identities WHERE user_id=$1 AND provider='apple' AND fiestaaa_lookup_matches(provider_subject_lookup_hash,$2))")
        .bind(user.id).bind(&claims.sub).fetch_one(&state.db).await;
    if !matches.unwrap_or(false) {
        return HttpResponse::Unauthorized().json(json!({"error":"different_apple_account"}));
    }
    match save_code(&state, user.id, &claims, code, payload.android).await {
        Ok(()) => HttpResponse::Ok().json(json!({"status":"apple_reauthorized"})),
        Err(e) => HttpResponse::ServiceUnavailable().json(json!({"error":e})),
    }
}

pub fn start_worker(state: web::Data<AppState>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            if let Err(e) = revoke_next(&state).await {
                log::warn!("Apple revocation retry pending: {e}");
            }
        }
    });
}
pub async fn revoke_next(state: &AppState) -> Result<(), sqlx::Error> {
    let mut tx = state.db.begin().await?;
    let Some(row)=sqlx::query("SELECT id,client_id,fiestaaa_decrypt_text(refresh_token_ciphertext) AS token FROM apple_revocations WHERE next_attempt_at<=NOW() ORDER BY next_attempt_at LIMIT 1 FOR UPDATE SKIP LOCKED")
        .fetch_optional(&mut *tx).await? else {return Ok(())};
    let id: i64 = row.get("id");
    let client: String = row.get("client_id");
    let token: String = row.get("token");
    let success = match state.apple_config.client_secret(&client) {
        Ok(secret) => state
            .http_client
            .post("https://appleid.apple.com/auth/revoke")
            .form(&[
                ("client_id", client.as_str()),
                ("client_secret", secret.as_str()),
                ("token", token.as_str()),
                ("token_type_hint", "refresh_token"),
            ])
            .send()
            .await
            .is_ok_and(|r| r.status().is_success()),
        Err(_) => false,
    };
    if success {
        sqlx::query("DELETE FROM apple_revocations WHERE id=$1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    } else {
        sqlx::query("UPDATE apple_revocations SET attempts=attempts+1,next_attempt_at=NOW()+INTERVAL '15 minutes' WHERE id=$1").bind(id).execute(&mut *tx).await?;
        crate::observability::capture_message(sentry::Level::Warning, "apple_revocation_pending");
    }
    tx.commit().await
}

/// Apple form callback: fixed application destination, never issues a session.
#[utoipa::path(post,path="/auth/apple/android-callback",tag="auth",
 responses((status=303,description="Return Apple credentials to Android"),(status=400,description="Invalid form")))]
#[post("/auth/apple/android-callback")]
pub async fn android_callback(
    form: web::Form<std::collections::HashMap<String, String>>,
) -> HttpResponse {
    if form.len() > 6 || form.values().any(|v| v.len() > 16384) || !form.contains_key("state") {
        return HttpResponse::BadRequest().finish();
    }
    let mut query = reqwest::Url::parse("https://localhost/").expect("static URL");
    for key in [
        "code",
        "id_token",
        "state",
        "user",
        "error",
        "error_description",
    ] {
        if let Some(value) = form.get(key) {
            query.query_pairs_mut().append_pair(key, value);
        }
    }
    HttpResponse::SeeOther()
        .insert_header(("Location",format!("intent://callback?{}#Intent;package=com.fiestaaa.fiestaaa;scheme=signinwithapple;end",query.query().unwrap_or_default())))
        .insert_header(("Cache-Control","no-store"))
        .insert_header(("Referrer-Policy","no-referrer"))
        .finish()
}

#[cfg(test)]
mod tests {
    use super::validate_exchanged_identity_token;
    use crate::{auth::now_ts, models::AppleClaims};
    use actix_web::{App, HttpResponse, HttpServer, web};
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
    use rsa::{
        RsaPrivateKey, RsaPublicKey, pkcs1::EncodeRsaPrivateKey, rand_core::OsRng,
        traits::PublicKeyParts,
    };

    struct SigningKey {
        private_der: Vec<u8>,
        modulus: String,
        exponent: String,
    }

    impl SigningKey {
        fn generate() -> Self {
            let private = RsaPrivateKey::new(&mut OsRng, 2048).unwrap();
            let public = RsaPublicKey::from(&private);
            Self {
                private_der: private.to_pkcs1_der().unwrap().as_bytes().to_vec(),
                modulus: URL_SAFE_NO_PAD.encode(public.n().to_bytes_be()),
                exponent: URL_SAFE_NO_PAD.encode(public.e().to_bytes_be()),
            }
        }

        fn jwk(&self, kid: &str) -> serde_json::Value {
            serde_json::json!({"kid":kid,"kty":"RSA","alg":"RS256","use":"sig",
                "n":self.modulus,"e":self.exponent})
        }

        fn token(&self, kid: Option<&str>, claims: &AppleClaims) -> String {
            let mut header = Header::new(Algorithm::RS256);
            header.kid = kid.map(str::to_owned);
            jsonwebtoken::encode(
                &header,
                &signed_claims(claims),
                &EncodingKey::from_rsa_der(&self.private_der),
            )
            .unwrap()
        }
    }

    fn signed_claims(claims: &AppleClaims) -> serde_json::Value {
        serde_json::json!({"sub":claims.sub,"aud":claims.aud,"iss":claims.iss,"exp":claims.exp})
    }

    fn claims() -> AppleClaims {
        AppleClaims {
            sub: "synthetic-apple-subject".into(),
            email: None,
            email_verified: None,
            exp: (now_ts() + 3600) as usize,
            iss: "https://appleid.apple.com".into(),
            aud: "com.fiestaaa.web".into(),
        }
    }

    #[actix_web::test]
    async fn exchanged_token_resolves_rotated_key_and_preserves_identity_checks() {
        crate::install_rustls_crypto_provider();
        let first = SigningKey::generate();
        let second = SigningKey::generate();
        let keys = serde_json::json!({"keys":[first.jwk("initial"),second.jwk("exchanged")]});
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let url = format!("http://{}/keys", listener.local_addr().unwrap());
        let server = HttpServer::new(move || {
            let keys = web::Data::new(keys.clone());
            App::new().app_data(keys).route(
                "/keys",
                web::get().to(|keys: web::Data<serde_json::Value>| async move {
                    HttpResponse::Ok().json(keys.get_ref())
                }),
            )
        })
        .workers(1)
        .listen(listener)
        .unwrap()
        .run();
        let handle = server.handle();
        let task = actix_web::rt::spawn(server);
        let client = reqwest::Client::new();
        let original = claims();
        let token = second.token(Some("exchanged"), &original);
        let old_key = DecodingKey::from_rsa_components(&first.modulus, &first.exponent).unwrap();
        let mut old_validation = Validation::new(Algorithm::RS256);
        old_validation.set_audience(&[&original.aud]);
        // Reproduce the old failure when Apple's two responses use different keys.
        assert!(jsonwebtoken::decode::<AppleClaims>(&token, &old_key, &old_validation).is_err());
        assert_eq!(
            validate_exchanged_identity_token(&client, &url, &original, &token).await,
            Ok(())
        );
        // The same signing key continues to work as well.
        let same_key_token = first.token(Some("initial"), &original);
        assert_eq!(
            validate_exchanged_identity_token(&client, &url, &original, &same_key_token).await,
            Ok(())
        );

        let mut invalid = Vec::new();
        let mut wrong = claims();
        wrong.sub = "another-subject".into();
        invalid.push(second.token(Some("exchanged"), &wrong));
        wrong = claims();
        wrong.aud = "another-service".into();
        invalid.push(second.token(Some("exchanged"), &wrong));
        wrong = claims();
        wrong.iss = "https://untrusted.example.invalid".into();
        invalid.push(second.token(Some("exchanged"), &wrong));
        wrong = claims();
        wrong.exp = (now_ts() - 300) as usize;
        invalid.push(second.token(Some("exchanged"), &wrong));
        invalid.push(second.token(None, &original));
        invalid.push(first.token(Some("exchanged"), &original));
        let mut header = Header::new(Algorithm::HS256);
        header.kid = Some("exchanged".into());
        invalid.push(
            jsonwebtoken::encode(
                &header,
                &signed_claims(&original),
                &EncodingKey::from_secret(b"test-only-secret"),
            )
            .unwrap(),
        );
        for token in invalid {
            assert_eq!(
                validate_exchanged_identity_token(&client, &url, &original, &token).await,
                Err("invalid_apple_code")
            );
        }
        let unknown = second.token(Some("unpublished-key"), &original);
        assert_eq!(
            validate_exchanged_identity_token(&client, &url, &original, &unknown).await,
            Err("apple_exchange_failed")
        );
        handle.stop(true).await;
        task.await.unwrap().unwrap();
    }
}
