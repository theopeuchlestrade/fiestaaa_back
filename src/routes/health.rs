use actix_web::{HttpResponse, Responder, get, web};

use crate::{models::HealthResponse, state::AppState};

#[utoipa::path(
    get,
    path = "/health",
    tag = "health",
    responses(
        (status = 200, description = "Database reachable", body = HealthResponse),
        (status = 503, description = "Database unreachable", body = HealthResponse)
    )
)]
#[get("/health")]
pub async fn health(state: web::Data<AppState>) -> impl Responder {
    let db_ok = sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&state.db)
        .await
        .map(|v| v == 1)
        .unwrap_or(false);

    if db_ok {
        HttpResponse::Ok().json(HealthResponse {
            status: "ok".into(),
            db: None,
        })
    } else {
        HttpResponse::ServiceUnavailable().json(HealthResponse {
            status: "degraded".into(),
            db: Some("unreachable".into()),
        })
    }
}
