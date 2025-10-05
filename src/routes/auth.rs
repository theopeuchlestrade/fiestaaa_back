use actix_web::{post, web, HttpResponse, Responder};
use actix_web::http::header::CONTENT_TYPE;
use sqlx::Error;

use crate::{state::AppState, models::{LoginPayload, RegisterPayload, Claims}, auth::{hash_password, verify_user_db, encode_jwt, now_ts}};

#[post("/auth/register")]
pub async fn register(state: web::Data<AppState>, payload: web::Json<RegisterPayload>) -> impl Responder {
    let email = payload.email.trim().to_lowercase();
    let password = payload.password.trim();

    if email.is_empty() || password.len() < 8 {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "error": "invalid_payload",
            "details": "email required, password >= 8 chars"
        }));
    }

    let hash = match hash_password(password) { Ok(h) => h, Err(_) => return HttpResponse::InternalServerError().json(serde_json::json!({"error":"hash_failed"})) };

    let res = sqlx::query("INSERT INTO users (email, password_hash) VALUES ($1, $2)")
        .bind(&email)
        .bind(&hash)
        .execute(&state.db)
        .await;

    match res {
        Ok(_) => HttpResponse::Created().json(serde_json::json!({"status":"ok"})),
        Err(e) => match e {
            Error::Database(db_err) if db_err.code().map(|c| c.as_ref()) == Some("23505") => {
                HttpResponse::Conflict().json(serde_json::json!({"error":"email_taken"}))
            }
            _ => HttpResponse::InternalServerError().json(serde_json::json!({"error":"db_error"})),
        }
    }
}

#[post("/auth/login")]
pub async fn login(state: web::Data<AppState>, payload: web::Json<LoginPayload>) -> impl Responder {
    let ok = match verify_user_db(&state.db, &payload.email, &payload.password).await {
        Ok(v) => v,
        Err(_) => return HttpResponse::InternalServerError().json(serde_json::json!({"error":"db_error"})),
    };
    if !ok {
        return HttpResponse::Unauthorized().json(serde_json::json!({"error":"invalid_credentials"}));
    }

    let exp = (now_ts() + 24 * 3600) as usize;
    let claims = Claims { sub: payload.email.to_owned(), exp };

    match encode_jwt(&claims, &state.jwt_secret) {
        Ok(token) => HttpResponse::Ok()
            .insert_header((CONTENT_TYPE, "application/json"))
            .json(serde_json::json!({ "token": token })),
        Err(_) => HttpResponse::InternalServerError().json(serde_json::json!({"error": "token_creation_failed"})),
    }
}

