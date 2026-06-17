use actix_web::{
    HttpRequest, HttpResponse, get,
    http::header::{CACHE_CONTROL, PRAGMA},
    web::{self, Data},
};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use redis::Client as RedisClient;
use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

use crate::{
    auth::now_ts,
    models::{ErrorResponse, RealtimeTicketResponse},
    routes::event_access::ensure_event_member_email,
    state::AppState,
};

const GLOBAL_CHANNEL: &str = "fiestaaa:global";
const REALTIME_TICKET_PURPOSE: &str = "realtime";
const REALTIME_TICKET_TTL_SECONDS: u64 = 60;

mod actor_ws;

pub mod event_types {
    pub const EVENTS_CHANGED: &str = "events.changed";
    pub const EVENT_UPDATED: &str = "event.updated";
    pub const EVENT_DELETED: &str = "event.deleted";
    pub const EVENT_ITEMS_CHANGED: &str = "event.items.changed";
    pub const EVENT_POLLS_CHANGED: &str = "event.polls.changed";
    pub const EVENT_INVITATIONS_CHANGED: &str = "event.invitations.changed";
    pub const INVITATIONS_CHANGED: &str = "invitations.changed";
    pub const FRIEND_REQUESTS_CHANGED: &str = "friend_requests.changed";
    pub const FRIENDSHIPS_CHANGED: &str = "friendships.changed";
}

pub fn global_channel() -> &'static str {
    GLOBAL_CHANNEL
}

pub fn event_channel(event_id: i64) -> String {
    format!("fiestaaa:event:{event_id}")
}

pub async fn publish_json(redis: &Option<RedisClient>, channel: &str, payload: &impl Serialize) {
    if let Some(client) = redis
        && let Ok(mut conn) = client.get_multiplexed_async_connection().await
    {
        let _: redis::RedisResult<()> = redis::cmd("PUBLISH")
            .arg(channel)
            .arg(serde_json::to_string(payload).unwrap_or_default())
            .query_async(&mut conn)
            .await;
    }
}

pub async fn publish_global(redis: &Option<RedisClient>, payload: &impl Serialize) {
    publish_json(redis, GLOBAL_CHANNEL, payload).await;
}

pub async fn publish_global_type(redis: &Option<RedisClient>, event_type: &str) {
    publish_global(redis, &json!({ "type": event_type })).await;
}

pub async fn publish_event(redis: &Option<RedisClient>, event_id: i64, payload: &impl Serialize) {
    let ch = event_channel(event_id);
    publish_json(redis, &ch, payload).await;
}

pub async fn publish_event_type(redis: &Option<RedisClient>, event_id: i64, event_type: &str) {
    publish_event(
        redis,
        event_id,
        &json!({
            "type": event_type,
            "event_id": event_id,
        }),
    )
    .await;
}

#[derive(Debug, serde::Deserialize, Clone, Default)]
#[serde(default)]
pub struct RealtimeTicketQuery {
    pub event_id: Option<i64>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
struct RealtimeTicketClaims {
    sub: String,
    exp: usize,
    event_id: Option<i64>,
    purpose: String,
    nonce: String,
}

#[derive(Debug, serde::Deserialize, Clone, Default)]
#[serde(default)]
pub struct EventWsQuery {
    pub event_id: Option<i64>,
    pub ticket: Option<String>,
}

fn normalize_origin(value: &str) -> Option<String> {
    let trimmed = value.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn request_origin_allowed(state: &AppState, req: &HttpRequest) -> bool {
    let Some(origin) = req
        .headers()
        .get("Origin")
        .and_then(|value| value.to_str().ok())
    else {
        return true;
    };
    let Some(normalized) = normalize_origin(origin) else {
        return false;
    };
    state.cors_allowed_origins.contains(&normalized)
}

#[utoipa::path(
    get,
    path = "/ws-ticket",
    tag = "notifications",
    responses(
        (status = 200, description = "Ephemeral realtime ticket", body = RealtimeTicketResponse),
        (status = 401, description = "Authentication required", body = ErrorResponse),
        (status = 403, description = "Access denied", body = ErrorResponse),
        (status = 404, description = "Event not found", body = ErrorResponse)
    ),
    params(
        ("event_id" = Option<i64>, Query, description = "Target event identifier")
    )
)]
#[get("/ws-ticket")]
pub async fn issue_realtime_ticket(
    state: Data<AppState>,
    req: HttpRequest,
    query: web::Query<RealtimeTicketQuery>,
) -> HttpResponse {
    if !request_origin_allowed(state.get_ref(), &req) {
        return HttpResponse::Forbidden().json(ErrorResponse {
            error: "forbidden_origin".into(),
            details: Some("Origin non autorisee".into()),
        });
    }

    let claims = match crate::auth::extract_active_claims_from_auth(
        &req,
        &state.db,
        &state.jwt_secret,
    )
    .await
    {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    if let Some(event_id) = query.event_id
        && let Err(resp) = ensure_event_member_email(&state.db, event_id, &claims.sub).await
    {
        return resp;
    }

    let expires_at =
        chrono::Utc::now() + chrono::Duration::seconds(REALTIME_TICKET_TTL_SECONDS as i64);
    let ticket_claims = RealtimeTicketClaims {
        sub: claims.sub.to_lowercase(),
        exp: (now_ts() + REALTIME_TICKET_TTL_SECONDS) as usize,
        event_id: query.event_id,
        purpose: REALTIME_TICKET_PURPOSE.into(),
        nonce: Uuid::new_v4().to_string(),
    };

    match encode(
        &Header::new(Algorithm::HS256),
        &ticket_claims,
        &EncodingKey::from_secret(state.jwt_secret.as_bytes()),
    ) {
        Ok(ticket) => HttpResponse::Ok()
            .insert_header((CACHE_CONTROL, "no-store"))
            .insert_header((PRAGMA, "no-cache"))
            .json(RealtimeTicketResponse {
                ticket,
                expires_at,
                event_id: query.event_id,
            }),
        Err(_) => HttpResponse::InternalServerError().json(ErrorResponse {
            error: "ticket_creation_failed".into(),
            details: None,
        }),
    }
}

#[get("/ws")]
pub async fn websocket(
    state: Data<AppState>,
    req: HttpRequest,
    stream: web::Payload,
) -> Result<HttpResponse, actix_web::Error> {
    if !request_origin_allowed(state.get_ref(), &req) {
        return Ok(HttpResponse::Forbidden().json(ErrorResponse {
            error: "forbidden_origin".into(),
            details: Some("Origin non autorisee".into()),
        }));
    }

    let params: EventWsQuery = serde_urlencoded::from_str(req.query_string()).unwrap_or_default();
    let (_email, event_id, auth_exp) = match resolve_ws_identity(&state, &params).await {
        Ok(value) => value,
        Err(resp) => return Ok(resp),
    };

    actor_ws::start_actor_websocket(state.redis_client.clone(), event_id, auth_exp, &req, stream)
}

async fn resolve_ws_identity(
    state: &Data<AppState>,
    params: &EventWsQuery,
) -> Result<(String, Option<i64>, usize), HttpResponse> {
    let Some(ticket) = params.ticket.as_deref() else {
        return Err(HttpResponse::Unauthorized().json(ErrorResponse {
            error: "missing_ticket".into(),
            details: None,
        }));
    };

    let claims = decode_realtime_ticket(ticket, &state.jwt_secret)?;
    let email = claims.sub.to_lowercase();
    if let Some(event_id) = claims.event_id {
        ensure_event_member_email(&state.db, event_id, &email).await?;
        return Ok((email, Some(event_id), claims.exp));
    }
    Ok((email, None, claims.exp))
}

fn decode_realtime_ticket(
    ticket: &str,
    secret: &str,
) -> Result<RealtimeTicketClaims, HttpResponse> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = true;
    match decode::<RealtimeTicketClaims>(
        ticket,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    ) {
        Ok(data) if data.claims.purpose == REALTIME_TICKET_PURPOSE => Ok(data.claims),
        Ok(_) => Err(HttpResponse::Unauthorized().json(ErrorResponse {
            error: "invalid_ticket".into(),
            details: None,
        })),
        Err(_) => Err(HttpResponse::Unauthorized().json(ErrorResponse {
            error: "invalid_ticket".into(),
            details: None,
        })),
    }
}
