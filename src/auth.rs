use argon2::{Argon2, password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString}};
use jsonwebtoken::{encode, decode, Header, Algorithm, Validation, EncodingKey, DecodingKey, errors::ErrorKind};
use rand_core::OsRng;
use sqlx::{Pool, Postgres, Row};
use actix_web::{HttpRequest, HttpResponse};
use actix_web::http::header::AUTHORIZATION;

use crate::models::Claims;

pub fn now_ts() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

pub fn hash_password(password: &str) -> Result<String, ()> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default().hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|_| ())
}

pub fn verify_password(hash: &str, password: &str) -> bool {
    if let Ok(parsed) = PasswordHash::new(hash) { Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok() } else { false }
}

pub async fn verify_user_db(pool: &Pool<Postgres>, email: &str, password: &str) -> sqlx::Result<bool> {
    let row = sqlx::query("SELECT password_hash FROM users WHERE lower(email)=lower($1)")
        .bind(email)
        .fetch_optional(pool)
        .await?;
    let Some(row) = row else { return Ok(false) };
    let stored: String = row.get("password_hash");
    Ok(verify_password(&stored, password))
}

pub fn encode_jwt(claims: &Claims, secret: &str) -> Result<String, ()> {
    encode(&Header::new(Algorithm::HS256), claims, &EncodingKey::from_secret(secret.as_bytes())).map_err(|_| ())
}

pub fn decode_jwt(token: &str, secret: &str) -> Result<Claims, HttpResponse> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = true;
    match decode::<Claims>(token, &DecodingKey::from_secret(secret.as_bytes()), &validation) {
        Ok(data) => Ok(data.claims),
        Err(e) => {
            let code = match e.kind() {
                ErrorKind::ExpiredSignature => "token_expired",
                ErrorKind::InvalidToken => "invalid_token",
                _ => "token_error",
            };
            Err(HttpResponse::Unauthorized().json(serde_json::json!({ "error": code })))
        }
    }
}

pub fn extract_claims_from_auth(req: &HttpRequest, secret: &str) -> Result<Claims, HttpResponse> {
    let header = req.headers().get(AUTHORIZATION).and_then(|v| v.to_str().ok());
    let Some(header_val) = header else { return Err(HttpResponse::Unauthorized().json(serde_json::json!({"error":"missing_authorization_header"}))) };
    let prefix = "Bearer ";
    if !header_val.starts_with(prefix) {
        return Err(HttpResponse::Unauthorized().json(serde_json::json!({"error":"invalid_authorization_scheme"})));
    }
    let token = &header_val[prefix.len()..];
    decode_jwt(token, secret)
}

