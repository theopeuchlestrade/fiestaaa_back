use actix_web::{HttpRequest, HttpResponse, Responder, get, web};
use sqlx::Row;

use crate::{
    auth::extract_claims_from_auth,
    metrics::AppMetrics,
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
pub async fn me(
    state: web::Data<AppState>,
    req: HttpRequest,
    metrics: web::Data<AppMetrics>,
) -> impl Responder {
    let _ = sqlx::query("ALTER TABLE users ADD COLUMN IF NOT EXISTS avatar_url TEXT;")
        .execute(&state.db)
        .await;
    match extract_claims_from_auth(&req, &state.jwt_secret) {
        Ok(claims) => {
            let record = metrics
                .track_database_operation("select_user", || async {
                    sqlx::query(
                        "SELECT email, handle, avatar_url FROM users WHERE lower(email)=lower($1)",
                    )
                    .bind(&claims.sub)
                    .fetch_optional(&state.db)
                    .await
                })
                .await;

            match record {
                Ok(Some(user)) => {
                    let email: String =
                        user.try_get::<String, _>("email").unwrap_or_else(|_| claims.sub.clone());
                    let handle: String =
                        user.try_get::<String, _>("handle").unwrap_or_else(|_| claims.handle);
                    let avatar_url: Option<String> =
                        user.try_get::<Option<String>, _>("avatar_url")
                            .ok()
                            .flatten();
                    HttpResponse::Ok().json(MeResponse {
                        email,
                        handle,
                        avatar_url,
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
