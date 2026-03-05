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
use uuid::Uuid;

use crate::{
    handles::{looks_like_email, normalize_handle},
    models::{Claims, ErrorResponse},
};

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

#[cfg(test)]
mod tests {
    use super::{hash_password, validate_password_strength, verify_password};

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
}
