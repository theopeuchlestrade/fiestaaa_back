use actix_web::{HttpRequest, HttpResponse, Responder, get, web};
use sqlx::Row;

use crate::{
    auth::extract_claims_from_auth,
    models::{ErrorResponse, MeResponse},
    state::AppState,
};

#[utoipa::path(
    get,
    path = "/",
    tag = "root",
    responses(
        (status = 200, description = "API heartbeat information", body = String)
    )
)]
#[get("/")]
pub async fn hello() -> impl Responder {
    HttpResponse::Ok().body("Fiestaaa API is running ✨")
}

#[utoipa::path(
    get,
    path = "/me",
    tag = "root",
    responses(
        (status = 200, description = "Connected user information", body = MeResponse),
        (status = 401, description = "Missing or invalid token", body = crate::models::ErrorResponse)
    )
)]
#[get("/me")]
pub async fn me(state: web::Data<AppState>, req: HttpRequest) -> impl Responder {
    match extract_claims_from_auth(&req, &state.jwt_secret) {
        Ok(claims) => {
            let record =
                sqlx::query("SELECT email, handle FROM users WHERE lower(email)=lower($1)")
                    .bind(&claims.sub)
                    .fetch_optional(&state.db)
                    .await;

            match record {
                Ok(Some(user)) => {
                    let email: String =
                        user.try_get("email").unwrap_or_else(|_| claims.sub.clone());
                    let handle: String = user.try_get("handle").unwrap_or_else(|_| claims.handle);
                    HttpResponse::Ok().json(MeResponse {
                        email,
                        handle,
                        exp: claims.exp,
                    })
                }
                Ok(None) => HttpResponse::Unauthorized().json(ErrorResponse {
                    error: "user_not_found".into(),
                    details: None,
                }),
                Err(_) => HttpResponse::InternalServerError().json(ErrorResponse {
                    error: "db_error".into(),
                    details: None,
                }),
            }
        }
        Err(resp) => resp,
    }
}
