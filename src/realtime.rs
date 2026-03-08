use std::time::{Duration, Instant};

use actix::prelude::*;
use actix_web::{
    HttpRequest, HttpResponse, get,
    web::{self, Data},
};
use actix_web_actors::ws;
use futures_util::StreamExt;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use log::warn;
use redis::Client as RedisClient;
use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

use crate::{
    auth::{extract_claims_from_auth, now_ts},
    models::{ErrorResponse, RealtimeTicketResponse},
    routes::event_access::ensure_event_member_email,
    state::AppState,
};

const GLOBAL_CHANNEL: &str = "fiestaaa:global";
const REALTIME_TICKET_PURPOSE: &str = "realtime";
const REALTIME_TICKET_TTL_SECONDS: u64 = 60;

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

#[utoipa::path(
    get,
    path = "/ws-ticket",
    tag = "notifications",
    responses(
        (status = 200, description = "Ticket realtime éphémère", body = RealtimeTicketResponse),
        (status = 401, description = "Authentification requise", body = ErrorResponse),
        (status = 403, description = "Accès refusé", body = ErrorResponse),
        (status = 404, description = "Événement introuvable", body = ErrorResponse)
    ),
    params(
        ("event_id" = Option<i64>, Query, description = "Identifiant d'événement à cibler")
    )
)]
#[get("/ws-ticket")]
pub async fn issue_realtime_ticket(
    state: Data<AppState>,
    req: HttpRequest,
    query: web::Query<RealtimeTicketQuery>,
) -> HttpResponse {
    let claims = match extract_claims_from_auth(&req, &state.jwt_secret) {
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
        Ok(ticket) => HttpResponse::Ok().json(RealtimeTicketResponse {
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
    let params: EventWsQuery = serde_urlencoded::from_str(req.query_string()).unwrap_or_default();
    let (email, event_id) = match resolve_ws_identity(&state, &req, &params).await {
        Ok(value) => value,
        Err(resp) => return Ok(resp),
    };

    let ws = WsSession {
        email,
        redis_client: state.redis_client.clone(),
        event_id,
        hb: Instant::now(),
    };

    ws::start(ws, &req, stream)
}

async fn resolve_ws_identity(
    state: &Data<AppState>,
    req: &HttpRequest,
    params: &EventWsQuery,
) -> Result<(String, Option<i64>), HttpResponse> {
    if let Ok(claims) = extract_claims_from_auth(req, &state.jwt_secret) {
        let email = claims.sub.to_lowercase();
        if let Some(event_id) = params.event_id {
            ensure_event_member_email(&state.db, event_id, &email).await?;
            return Ok((email, Some(event_id)));
        }
        return Ok((email, None));
    }

    let Some(ticket) = params.ticket.as_deref() else {
        return Err(HttpResponse::Unauthorized().json(ErrorResponse {
            error: "missing_authorization_header".into(),
            details: None,
        }));
    };

    let claims = decode_realtime_ticket(ticket, &state.jwt_secret)?;
    let email = claims.sub.to_lowercase();
    if let Some(event_id) = claims.event_id {
        ensure_event_member_email(&state.db, event_id, &email).await?;
        return Ok((email, Some(event_id)));
    }
    Ok((email, None))
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

#[derive(Message)]
#[rtype(result = "()")]
struct RedisMessage(String);

pub struct WsSession {
    pub email: String,
    pub redis_client: Option<RedisClient>,
    pub event_id: Option<i64>,
    hb: Instant,
}

impl WsSession {
    fn hb(&self, ctx: &mut ws::WebsocketContext<Self>) {
        ctx.run_interval(Duration::from_secs(15), |act, ctx| {
            if Instant::now().duration_since(act.hb) > Duration::from_secs(45) {
                ctx.close(None);
                ctx.stop();
                return;
            }
            ctx.ping(b"ping");
        });
    }
}

impl Actor for WsSession {
    type Context = ws::WebsocketContext<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        self.hb(ctx);

        if let Some(client) = self.redis_client.clone() {
            let addr = ctx.address();
            let channels = {
                let mut list = vec![GLOBAL_CHANNEL.to_string()];
                if let Some(eid) = self.event_id {
                    list.push(event_channel(eid));
                }
                list
            };

            actix_web::rt::spawn(async move {
                match client.get_async_pubsub().await {
                    Ok(mut pubsub) => {
                        for ch in &channels {
                            let _ = pubsub.subscribe(ch).await;
                        }
                        let mut on_msg = pubsub.on_message();
                        while let Some(msg) = on_msg.next().await {
                            if let Ok(payload) = msg.get_payload::<String>() {
                                let _ = addr.try_send(RedisMessage(payload));
                            }
                        }
                    }
                    Err(err) => {
                        warn!("ws redis connection error: {err}");
                    }
                }
            });
        } else {
            ctx.text(
                json!({"type": "warning", "payload": {"message": "realtime_disabled"}}).to_string(),
            );
        }
    }
}

impl Handler<RedisMessage> for WsSession {
    type Result = ();

    fn handle(&mut self, msg: RedisMessage, ctx: &mut Self::Context) -> Self::Result {
        ctx.text(msg.0);
    }
}

impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for WsSession {
    fn handle(&mut self, item: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        match item {
            Ok(ws::Message::Ping(msg)) => {
                self.hb = Instant::now();
                ctx.pong(&msg);
            }
            Ok(ws::Message::Pong(_)) => {
                self.hb = Instant::now();
            }
            Ok(ws::Message::Text(text)) => {
                if text.trim() == "ping" {
                    ctx.text("pong");
                }
            }
            Ok(ws::Message::Close(reason)) => {
                ctx.close(reason);
                ctx.stop();
            }
            _ => {}
        }
    }
}
