use actix_web::{HttpRequest, HttpResponse, Responder, get, patch, web};
use sqlx::Row;

use crate::{
    auth::extract_claims_from_auth,
    handles::{handle_available, is_valid_handle, normalize_handle},
    models::{ErrorResponse, HandleAvailabilityResponse, HandleUpdatePayload, MeResponse},
    state::AppState,
};

#[derive(serde::Deserialize)]
pub struct HandleAvailabilityQuery {
    pub handle: String,
}

#[utoipa::path(
    get,
    path = "/handles/availability",
    tag = "users",
    responses(
        (status = 200, description = "Disponibilité d'un handle", body = HandleAvailabilityResponse),
        (status = 400, description = "Handle invalide", body = ErrorResponse)
    ),
    params(
        ("handle" = String, Query, description = "Handle à tester")
    )
)]
#[get("/handles/availability")]
pub async fn check_handle_availability(
    state: web::Data<AppState>,
    query: web::Query<HandleAvailabilityQuery>,
) -> impl Responder {
    let candidate = normalize_handle(&query.handle).normalized;
    if !is_valid_handle(&candidate) {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_handle".into(),
            details: Some("format attendu: 4-32 chars [a-z0-9._-]".into()),
        });
    }

    match handle_available(&state.db, &candidate).await {
        Ok(available) => HttpResponse::Ok().json(HandleAvailabilityResponse { available }),
        Err(_) => HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        }),
    }
}

#[utoipa::path(
    patch,
    path = "/me/handle",
    tag = "users",
    request_body = HandleUpdatePayload,
    responses(
        (status = 200, description = "Handle mis à jour", body = MeResponse),
        (status = 400, description = "Handle invalide", body = ErrorResponse),
        (status = 401, description = "Authentification requise", body = ErrorResponse),
        (status = 409, description = "Handle déjà pris", body = ErrorResponse)
    )
)]
#[patch("/me/handle")]
pub async fn update_handle(
    state: web::Data<AppState>,
    req: HttpRequest,
    payload: web::Json<HandleUpdatePayload>,
) -> impl Responder {
    let claims = match extract_claims_from_auth(&req, &state.jwt_secret) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let candidate = normalize_handle(&payload.handle).normalized;
    if !is_valid_handle(&candidate) {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_handle".into(),
            details: Some("format attendu: 4-32 chars [a-z0-9._-]".into()),
        });
    }

    let res = sqlx::query(
        "UPDATE users
         SET handle = $1
         WHERE lower(email) = lower($2)
         RETURNING email, handle",
    )
    .bind(&candidate)
    .bind(&claims.sub)
    .fetch_one(&state.db)
    .await;

    match res {
        Ok(row) => HttpResponse::Ok().json(MeResponse {
            email: row.get("email"),
            handle: row.get("handle"),
            exp: claims.exp,
        }),
        Err(sqlx::Error::Database(db_err)) if db_err.code().as_deref() == Some("23505") => {
            HttpResponse::Conflict().json(ErrorResponse {
                error: "handle_taken".into(),
                details: None,
            })
        }
        Err(sqlx::Error::RowNotFound) => HttpResponse::Unauthorized().json(ErrorResponse {
            error: "user_not_found".into(),
            details: None,
        }),
        Err(_) => HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        }),
    }
}
