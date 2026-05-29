use actix_web::http::header::CONTENT_TYPE;
use actix_web::{HttpRequest, HttpResponse, Responder, get, web};

use crate::{models::HealthResponse, observability, state::AppState};

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
    let redis_status = if let Some(client) = state.redis_client.as_ref() {
        match client.get_multiplexed_async_connection().await {
            Ok(mut conn) => {
                let ping = redis::cmd("PING").query_async::<String>(&mut conn).await;
                if ping.map(|value| value == "PONG").unwrap_or(false) {
                    None
                } else {
                    Some("unreachable".into())
                }
            }
            Err(_) => Some("unreachable".into()),
        }
    } else {
        None
    };

    if db_ok && redis_status.is_none() {
        HttpResponse::Ok().json(HealthResponse {
            status: "ok".into(),
            db: None,
            redis: None,
        })
    } else {
        HttpResponse::ServiceUnavailable().json(HealthResponse {
            status: "degraded".into(),
            db: if db_ok {
                None
            } else {
                Some("unreachable".into())
            },
            redis: redis_status,
        })
    }
}

#[get("/metrics")]
pub async fn metrics(req: HttpRequest, state: web::Data<AppState>) -> impl Responder {
    if !observability::metrics_authorized(&req, state.metrics_bearer_token.as_deref()) {
        return HttpResponse::Unauthorized().finish();
    }

    match observability::render_prometheus() {
        Ok(body) => HttpResponse::Ok()
            .insert_header((CONTENT_TYPE, "text/plain; version=0.0.4"))
            .body(body),
        Err(err) => {
            observability::capture_message(
                sentry::Level::Error,
                &format!("failed to render prometheus metrics: {err}"),
            );
            HttpResponse::InternalServerError().finish()
        }
    }
}
