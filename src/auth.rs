use actix_web::http::header::AUTHORIZATION;
use actix_web::{HttpRequest, HttpResponse};
use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use jsonwebtoken::{
    Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode, errors::ErrorKind,
};
use rand_core::OsRng;
use sqlx::{Pool, Postgres};

use crate::{
    handles::{looks_like_email, normalize_handle},
    models::{Claims, ErrorResponse},
};

pub fn now_ts() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

pub fn hash_password(password: &str) -> Result<String, ()> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|_| ())
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

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct UserAuthRow {
    pub id: i64,
    pub email: String,
    pub handle: String,
    pub password_hash: String,
}

pub async fn fetch_user_auth(
    pool: &Pool<Postgres>,
    identifier: &str,
) -> sqlx::Result<Option<UserAuthRow>> {
    let trimmed = identifier.trim();
    if looks_like_email(trimmed) {
        sqlx::query_as::<_, UserAuthRow>(
            "SELECT id, email, handle, password_hash FROM users WHERE lower(email)=lower($1)",
        )
        .bind(trimmed)
        .fetch_optional(pool)
        .await
    } else {
        let normalized = normalize_handle(trimmed).normalized;
        sqlx::query_as::<_, UserAuthRow>(
            "SELECT id, email, handle, password_hash FROM users WHERE lower(handle)=lower($1)",
        )
        .bind(normalized)
        .fetch_optional(pool)
        .await
    }
}

pub fn encode_jwt(claims: &Claims, secret: &str) -> Result<String, ()> {
    encode(
        &Header::new(Algorithm::HS256),
        claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|_| ())
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

pub fn extract_claims_from_auth(req: &HttpRequest, secret: &str) -> Result<Claims, HttpResponse> {
    let header = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    let Some(header_val) = header else {
        return Err(HttpResponse::Unauthorized().json(ErrorResponse {
            error: "missing_authorization_header".into(),
            details: None,
        }));
    };
    let prefix = "Bearer ";
    if !header_val.starts_with(prefix) {
        return Err(HttpResponse::Unauthorized().json(ErrorResponse {
            error: "invalid_authorization_scheme".into(),
            details: None,
        }));
    }
    let token = &header_val[prefix.len()..];
    decode_jwt(token, secret)
}
