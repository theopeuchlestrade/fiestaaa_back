use actix_web::http::header::CONTENT_TYPE;
use actix_web::{HttpResponse, Responder, post, web};
use sqlx::Error;

use crate::{
    auth::{encode_jwt, fetch_user_auth, hash_password, now_ts, verify_password},
    handles::{generate_unique_handle, handle_available, is_valid_handle, normalize_handle},
    models::{Claims, ErrorResponse, LoginPayload, RegisterPayload, StatusResponse, TokenResponse},
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
pub async fn register(
    state: web::Data<AppState>,
    payload: web::Json<RegisterPayload>,
) -> impl Responder {
    let email = payload.email.trim().to_lowercase();
    let password = payload.password.trim();
    let requested_handle = payload
        .handle
        .as_ref()
        .map(|raw| normalize_handle(raw).normalized);

    if email.is_empty() || password.len() < 8 {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("email required, password >= 8 chars".into()),
        });
    }

    let handle = match requested_handle {
        Some(ref h) => {
            if !is_valid_handle(h) {
                return HttpResponse::BadRequest().json(ErrorResponse {
                    error: "invalid_handle".into(),
                    details: Some("format attendu: 4-32 chars [a-z0-9._-]".into()),
                });
            }
            match handle_available(&state.db, h).await {
                Ok(true) => h.clone(),
                Ok(false) => {
                    return HttpResponse::Conflict().json(ErrorResponse {
                        error: "handle_taken".into(),
                        details: None,
                    });
                }
                Err(_) => {
                    return HttpResponse::InternalServerError().json(ErrorResponse {
                        error: "db_error".into(),
                        details: None,
                    });
                }
            }
        }
        None => match generate_unique_handle(&state.db).await {
            Ok(h) => h,
            Err(_) => {
                return HttpResponse::InternalServerError().json(ErrorResponse {
                    error: "handle_generation_failed".into(),
                    details: None,
                });
            }
        },
    };

    let hash = match hash_password(password) {
        Ok(h) => h,
        Err(_) => {
            return HttpResponse::InternalServerError().json(ErrorResponse {
                error: "hash_failed".into(),
                details: None,
            });
        }
    };

    let res = sqlx::query("INSERT INTO users (email, password_hash, handle) VALUES ($1, $2, $3)")
        .bind(&email)
        .bind(&hash)
        .bind(&handle)
        .execute(&state.db)
        .await;

    match res {
        Ok(_) => HttpResponse::Created().json(StatusResponse {
            status: "ok".into(),
        }),
        Err(e) => match e {
            Error::Database(db_err) if db_err.code().as_deref() == Some("23505") => {
                let constraint = db_err.constraint().unwrap_or_default();
                let error = if constraint.contains("handle") {
                    "handle_taken"
                } else {
                    "email_taken"
                };
                HttpResponse::Conflict().json(ErrorResponse {
                    error: error.into(),
                    details: None,
                })
            }
            _ => HttpResponse::InternalServerError().json(ErrorResponse {
                error: "db_error".into(),
                details: None,
            }),
        },
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
    let auth_row = match fetch_user_auth(&state.db, &payload.identifier).await {
        Ok(Some(row)) => row,
        Ok(None) => {
            return HttpResponse::Unauthorized().json(ErrorResponse {
                error: "invalid_credentials".into(),
                details: None,
            });
        }
        Err(_) => {
            return HttpResponse::InternalServerError().json(ErrorResponse {
                error: "db_error".into(),
                details: None,
            });
        }
    };

    if !verify_password(&auth_row.password_hash, &payload.password) {
        return HttpResponse::Unauthorized().json(ErrorResponse {
            error: "invalid_credentials".into(),
            details: None,
        });
    }

    let exp = (now_ts() + 24 * 3600) as usize;
    let claims = Claims {
        sub: auth_row.email.clone(),
        handle: auth_row.handle.clone(),
        exp,
    };

    match encode_jwt(&claims, &state.jwt_secret) {
        Ok(token) => HttpResponse::Ok()
            .insert_header((CONTENT_TYPE, "application/json"))
            .json(TokenResponse {
                token,
                email: auth_row.email,
                handle: auth_row.handle,
            }),
        Err(_) => HttpResponse::InternalServerError().json(ErrorResponse {
            error: "token_creation_failed".into(),
            details: None,
        }),
    }
}
