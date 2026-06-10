use actix_web::cookie::{Cookie, SameSite, time::Duration};
use actix_web::http::header::AUTHORIZATION;
use actix_web::{HttpRequest, HttpResponse};
use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};
use chrono::{DateTime, Utc};
use jsonwebtoken::{
    Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode, errors::ErrorKind,
};
use sqlx::{Pool, Postgres};
use uuid::Uuid;

use crate::{
    handles::{looks_like_email, normalize_handle},
    models::{Claims, ErrorResponse},
    security::sha256_hex,
};

const SESSION_COOKIE_NAME: &str = "fiestaaa_session";

#[derive(Debug)]
pub enum AuthError {
    HashFailed,
    JwtFailed,
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::HashFailed => write!(f, "hashing failed"),
            Self::JwtFailed => write!(f, "jwt encoding failed"),
        }
    }
}

impl std::error::Error for AuthError {}

pub fn now_ts() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

pub fn hash_password(password: &str) -> Result<String, AuthError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|_| AuthError::HashFailed)
}

pub fn verify_password(hash: &str, password: &str) -> bool {
    if let Ok(parsed) = PasswordHash::new(hash) {
        Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok()
    } else {
        false
    }
}

pub fn validate_password_strength(password: &str) -> Result<(), &'static str> {
    if password.len() < 12 {
        return Err("le mot de passe doit contenir au moins 12 caractères");
    }
    let has_upper = password.chars().any(|c| c.is_uppercase());
    let has_lower = password.chars().any(|c| c.is_lowercase());
    let has_digit = password.chars().any(|c| c.is_ascii_digit());
    let has_special = password
        .chars()
        .any(|c| !c.is_ascii_alphanumeric() && !c.is_whitespace());
    if !(has_upper && has_lower && has_digit && has_special) {
        return Err("inclure une majuscule, une minuscule, un chiffre et un caractère spécial");
    }
    if password.chars().any(|c| c.is_control()) {
        return Err("les caractères de contrôle ne sont pas autorisés");
    }
    Ok(())
}

pub fn random_password_token() -> String {
    format!("oauth-{}", Uuid::new_v4())
}

pub fn session_cookie_name() -> &'static str {
    SESSION_COOKIE_NAME
}

fn header_first_value<'a>(req: &'a HttpRequest, name: &str) -> Option<&'a str> {
    req.headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn forwarded_proto_is_https(req: &HttpRequest) -> bool {
    if let Some(value) = header_first_value(req, "Forwarded") {
        for part in value.split(';') {
            let mut split = part.splitn(2, '=');
            let key = split.next().map(str::trim);
            let value = split
                .next()
                .map(str::trim)
                .map(|v| v.trim_matches('"'))
                .unwrap_or_default();
            if matches!(key, Some(k) if k.eq_ignore_ascii_case("proto"))
                && value.eq_ignore_ascii_case("https")
            {
                return true;
            }
        }
    }

    matches!(
        header_first_value(req, "X-Forwarded-Proto"),
        Some(value) if value.eq_ignore_ascii_case("https")
    )
}

pub fn should_secure_cookie(
    req: &HttpRequest,
    app_base_url: &str,
    trust_proxy_headers: bool,
) -> bool {
    if app_base_url.starts_with("https://") {
        return true;
    }

    if let Some(scheme) = req.uri().scheme_str()
        && scheme.eq_ignore_ascii_case("https")
    {
        return true;
    }

    trust_proxy_headers && forwarded_proto_is_https(req)
}

pub fn build_session_cookie(token: &str, secure: bool) -> Cookie<'static> {
    Cookie::build(SESSION_COOKIE_NAME, token.to_string())
        .http_only(true)
        .same_site(SameSite::Lax)
        .path("/")
        .secure(secure)
        .finish()
}

pub fn build_cleared_session_cookie(secure: bool) -> Cookie<'static> {
    Cookie::build(SESSION_COOKIE_NAME, "")
        .http_only(true)
        .same_site(SameSite::Lax)
        .path("/")
        .secure(secure)
        .max_age(Duration::seconds(0))
        .finish()
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct UserAuthRow {
    pub id: i64,
    pub public_id: Uuid,
    pub email: String,
    pub handle: String,
    pub password_hash: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct ActiveUserIdentityRow {
    pub email: String,
    pub handle: String,
}

pub async fn fetch_user_auth(
    pool: &Pool<Postgres>,
    identifier: &str,
) -> sqlx::Result<Option<UserAuthRow>> {
    let trimmed = identifier.trim();
    if looks_like_email(trimmed) {
        sqlx::query_as::<_, UserAuthRow>(
            "SELECT id,
                    public_id,
                    fiestaaa_decrypt_text(email_ciphertext) AS email,
                    handle,
                    password_hash
             FROM users
             WHERE fiestaaa_email_matches(email_lookup_hash, $1)",
        )
        .bind(trimmed)
        .fetch_optional(pool)
        .await
    } else {
        let normalized = normalize_handle(trimmed).normalized;
        sqlx::query_as::<_, UserAuthRow>(
            "SELECT id,
                    public_id,
                    fiestaaa_decrypt_text(email_ciphertext) AS email,
                    handle,
                    password_hash
             FROM users
             WHERE lower(handle)=lower($1)",
        )
        .bind(normalized)
        .fetch_optional(pool)
        .await
    }
}

pub fn encode_jwt(claims: &Claims, secret: &str) -> Result<String, AuthError> {
    encode(
        &Header::new(Algorithm::HS256),
        claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|_| AuthError::JwtFailed)
}

pub fn decode_jwt(token: &str, secret: &str) -> Result<Claims, HttpResponse> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = true;
    match decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    ) {
        Ok(data) => Ok(data.claims),
        Err(e) => {
            let code = match e.kind() {
                ErrorKind::ExpiredSignature => "token_expired",
                ErrorKind::InvalidToken => "invalid_token",
                _ => "token_error",
            };
            Err(HttpResponse::Unauthorized().json(ErrorResponse {
                error: code.into(),
                details: None,
            }))
        }
    }
}

fn token_hash(token: &str) -> String {
    sha256_hex(token)
}

fn extract_token_from_auth(req: &HttpRequest) -> Result<String, HttpResponse> {
    let header = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    if let Some(header_val) = header {
        let prefix = "Bearer ";
        if !header_val.starts_with(prefix) {
            return Err(HttpResponse::Unauthorized().json(ErrorResponse {
                error: "invalid_authorization_scheme".into(),
                details: None,
            }));
        }
        Ok(header_val[prefix.len()..].to_string())
    } else if let Some(cookie) = req.cookie(SESSION_COOKIE_NAME) {
        Ok(cookie.value().to_string())
    } else {
        Err(HttpResponse::Unauthorized().json(ErrorResponse {
            error: "missing_authorization_header".into(),
            details: None,
        }))
    }
}

fn extract_token_from_auth_optional(req: &HttpRequest) -> Option<String> {
    req.headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(ToOwned::to_owned)
        .or_else(|| {
            req.cookie(SESSION_COOKIE_NAME)
                .map(|cookie| cookie.value().to_string())
        })
}

async fn cleanup_expired_revoked_tokens(db: &Pool<Postgres>) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM revoked_auth_tokens WHERE expires_at <= NOW()")
        .execute(db)
        .await?;
    Ok(())
}

async fn find_active_user_by_subject(
    db: &Pool<Postgres>,
    subject: &str,
) -> Result<Option<ActiveUserIdentityRow>, sqlx::Error> {
    let trimmed = subject.trim();
    if looks_like_email(trimmed) {
        return sqlx::query_as::<_, ActiveUserIdentityRow>(
            "SELECT fiestaaa_decrypt_text(email_ciphertext) AS email, handle
             FROM users
             WHERE fiestaaa_email_matches(email_lookup_hash, $1)",
        )
        .bind(trimmed)
        .fetch_optional(db)
        .await;
    }

    let Ok(public_id) = Uuid::parse_str(trimmed) else {
        return Ok(None);
    };

    sqlx::query_as::<_, ActiveUserIdentityRow>(
        "SELECT fiestaaa_decrypt_text(email_ciphertext) AS email, handle
         FROM users
         WHERE public_id = $1",
    )
    .bind(public_id)
    .fetch_optional(db)
    .await
}

fn normalize_claims_with_user(mut claims: Claims, user: &ActiveUserIdentityRow) -> Claims {
    claims.sub = user.email.clone();
    claims.handle = user.handle.clone();
    claims
}

async fn is_token_revoked(db: &Pool<Postgres>, token: &str) -> Result<bool, sqlx::Error> {
    cleanup_expired_revoked_tokens(db).await?;
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(
            SELECT 1
            FROM revoked_auth_tokens
            WHERE token_hash = $1
        )",
    )
    .bind(token_hash(token))
    .fetch_one(db)
    .await
}

fn expiration_timestamp_to_datetime(exp: usize) -> Option<DateTime<Utc>> {
    DateTime::<Utc>::from_timestamp(exp as i64, 0)
}

pub async fn revoke_auth_token(
    db: &Pool<Postgres>,
    token: &str,
    secret: &str,
) -> Result<(), HttpResponse> {
    let claims = match decode_jwt(token, secret) {
        Ok(value) => value,
        Err(_) => return Ok(()),
    };
    let Some(expires_at) = expiration_timestamp_to_datetime(claims.exp) else {
        return Ok(());
    };

    cleanup_expired_revoked_tokens(db).await.map_err(|_| {
        HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        })
    })?;

    sqlx::query(
        "INSERT INTO revoked_auth_tokens (token_hash, expires_at)
         VALUES ($1, $2)
         ON CONFLICT (token_hash) DO NOTHING",
    )
    .bind(token_hash(token))
    .bind(expires_at)
    .execute(db)
    .await
    .map_err(|_| {
        HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        })
    })?;

    Ok(())
}

pub async fn revoke_auth_token_from_request(
    req: &HttpRequest,
    db: &Pool<Postgres>,
    secret: &str,
) -> Result<(), HttpResponse> {
    let Some(token) = extract_token_from_auth_optional(req) else {
        return Ok(());
    };
    revoke_auth_token(db, &token, secret).await
}

pub fn extract_claims_from_auth(req: &HttpRequest, secret: &str) -> Result<Claims, HttpResponse> {
    let token = extract_token_from_auth(req)?;
    decode_jwt(&token, secret)
}

pub async fn extract_verified_claims_from_auth(
    req: &HttpRequest,
    db: &Pool<Postgres>,
    secret: &str,
) -> Result<Claims, HttpResponse> {
    let token = extract_token_from_auth(req)?;
    let revoked = is_token_revoked(db, &token).await.map_err(|_| {
        HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        })
    })?;
    if revoked {
        return Err(HttpResponse::Unauthorized().json(ErrorResponse {
            error: "revoked_token".into(),
            details: None,
        }));
    }
    let claims = decode_jwt(&token, secret)?;
    let Some(user) = find_active_user_by_subject(db, &claims.sub)
        .await
        .map_err(|_| {
            HttpResponse::InternalServerError().json(ErrorResponse {
                error: "db_error".into(),
                details: None,
            })
        })?
    else {
        return Err(HttpResponse::Unauthorized().json(ErrorResponse {
            error: "user_not_found".into(),
            details: None,
        }));
    };
    Ok(normalize_claims_with_user(claims, &user))
}

pub async fn extract_active_claims_from_auth(
    req: &HttpRequest,
    db: &Pool<Postgres>,
    secret: &str,
) -> Result<Claims, HttpResponse> {
    extract_verified_claims_from_auth(req, db, secret).await
}

#[cfg(test)]
mod tests {
    use super::{
        build_cleared_session_cookie, build_session_cookie, hash_password, session_cookie_name,
        should_secure_cookie, validate_password_strength, verify_password,
    };
    use actix_web::{cookie::SameSite, test::TestRequest};

    #[test]
    fn password_hash_round_trip_verifies_plaintext() {
        let password = "Sup3rSecurePass!";
        let hash = hash_password(password).expect("hash");

        assert!(verify_password(&hash, password));
        assert!(!verify_password(&hash, "wrong-password"));
    }

    #[test]
    fn password_strength_requires_length_and_character_diversity() {
        assert!(validate_password_strength("Sup3rSecurePass!").is_ok());
        assert!(validate_password_strength("short").is_err());
        assert!(validate_password_strength("longbutnouppercase1!").is_err());
        assert!(validate_password_strength("LongButNoDigit!").is_err());
    }

    #[test]
    fn secure_cookie_detection_respects_app_url_and_trusted_proxy_headers() {
        let plain_req = TestRequest::default().to_http_request();
        assert!(should_secure_cookie(
            &plain_req,
            "https://fiestaaa.app",
            false
        ));
        assert!(!should_secure_cookie(
            &plain_req,
            "http://localhost:8080",
            false
        ));

        let forwarded_req = TestRequest::default()
            .insert_header(("X-Forwarded-Proto", "https"))
            .to_http_request();
        assert!(!should_secure_cookie(
            &forwarded_req,
            "http://localhost:8080",
            false
        ));
        assert!(should_secure_cookie(
            &forwarded_req,
            "http://localhost:8080",
            true
        ));

        let forwarded_header_req = TestRequest::default()
            .insert_header(("Forwarded", r#"for="127.0.0.1"; proto="https""#))
            .to_http_request();
        assert!(should_secure_cookie(
            &forwarded_header_req,
            "http://localhost:8080",
            true
        ));
    }

    #[test]
    fn session_cookies_use_http_only_lax_site_defaults() {
        let cookie = build_session_cookie("session-token", true);

        assert_eq!(cookie.name(), session_cookie_name());
        assert_eq!(cookie.value(), "session-token");
        assert_eq!(cookie.http_only(), Some(true));
        assert_eq!(cookie.secure(), Some(true));
        assert_eq!(cookie.same_site(), Some(SameSite::Lax));
        assert_eq!(cookie.path(), Some("/"));

        let cleared = build_cleared_session_cookie(false);
        assert_eq!(cleared.name(), session_cookie_name());
        assert_eq!(cleared.value(), "");
        assert_eq!(cleared.http_only(), Some(true));
        assert_eq!(cleared.secure(), Some(false));
        assert!(cleared.max_age().is_some());
    }
}
