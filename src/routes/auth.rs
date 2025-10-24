use actix_web::{post, web, HttpResponse, Responder};
use actix_web::http::header::CONTENT_TYPE;
use sqlx::Error;

use crate::{
    auth::{encode_jwt, hash_password, now_ts, verify_user_db},
    models::{
        Claims, ErrorResponse, LoginPayload, RegisterPayload, StatusResponse, TokenResponse,
    },
    state::AppState,
};

#[utoipa::path(
    post,
    path = "/auth/register",
    tag = "auth",
    request_body = RegisterPayload,
    responses(
        (status = 201, description = "User registered", body = StatusResponse),
        (status = 400, description = "Invalid payload", body = ErrorResponse),
        (status = 409, description = "Email already exists", body = ErrorResponse),
        (status = 500, description = "Database or hashing error", body = ErrorResponse)
    )
)]
#[post("/auth/register")]
pub async fn register(state: web::Data<AppState>, payload: web::Json<RegisterPayload>) -> impl Responder {
    let email = payload.email.trim().to_lowercase();
    let password = payload.password.trim();

    if email.is_empty() || password.len() < 8 {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("email required, password >= 8 chars".into()),
        });
    }

    let hash = match hash_password(password) {
        Ok(h) => h,
        Err(_) => {
            return HttpResponse::InternalServerError().json(ErrorResponse {
                error: "hash_failed".into(),
                details: None,
            })
        }
    };

    let res = sqlx::query("INSERT INTO users (email, password_hash) VALUES ($1, $2)")
        .bind(&email)
        .bind(&hash)
        .execute(&state.db)
        .await;

    match res {
        Ok(_) => HttpResponse::Created().json(StatusResponse { status: "ok".into() }),
        Err(e) => match e {
            Error::Database(db_err) if db_err.code().as_deref() == Some("23505") => {
                HttpResponse::Conflict().json(ErrorResponse { error: "email_taken".into(), details: None })
            }
            _ => HttpResponse::InternalServerError().json(ErrorResponse { error: "db_error".into(), details: None }),
        }
    }
}

#[utoipa::path(
    post,
    path = "/auth/login",
    tag = "auth",
    request_body = LoginPayload,
    responses(
        (status = 200, description = "Valid credentials", body = TokenResponse),
        (status = 401, description = "Invalid credentials", body = ErrorResponse),
        (status = 500, description = "Database or token creation error", body = ErrorResponse)
    )
)]
#[post("/auth/login")]
pub async fn login(state: web::Data<AppState>, payload: web::Json<LoginPayload>) -> impl Responder {
    let ok = match verify_user_db(&state.db, &payload.email, &payload.password).await {
        Ok(v) => v,
        Err(_) => return HttpResponse::InternalServerError().json(ErrorResponse { error: "db_error".into(), details: None }),
    };
    if !ok {
        return HttpResponse::Unauthorized().json(ErrorResponse { error: "invalid_credentials".into(), details: None });
    }

    let exp = (now_ts() + 24 * 3600) as usize;
    let claims = Claims { sub: payload.email.to_owned(), exp };

    match encode_jwt(&claims, &state.jwt_secret) {
        Ok(token) => HttpResponse::Ok()
            .insert_header((CONTENT_TYPE, "application/json"))
            .json(TokenResponse { token }),
        Err(_) => HttpResponse::InternalServerError().json(ErrorResponse { error: "token_creation_failed".into(), details: None }),
    }
}
