use actix_web::{get, web, HttpResponse, Responder};
use crate::state::AppState;

#[get("/health")]
pub async fn health(state: web::Data<AppState>) -> impl Responder {
    let db_ok = sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&state.db)
        .await
        .map(|v| v == 1)
        .unwrap_or(false);

    if db_ok { HttpResponse::Ok().json(serde_json::json!({"status":"ok"})) }
    else { HttpResponse::ServiceUnavailable().json(serde_json::json!({"status":"degraded","db":"unreachable"})) }
}

