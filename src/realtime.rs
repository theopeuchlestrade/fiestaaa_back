use std::time::{Duration, Instant};

use actix::prelude::*;
use actix_web::{
    HttpRequest, HttpResponse, get,
    web::{self, Data},
};
use actix_web_actors::ws;
use futures_util::StreamExt;
use log::warn;
use redis::Client as RedisClient;
use serde::Serialize;
use serde_json::json;

use crate::{
    auth::{decode_jwt, extract_claims_from_auth},
    models::ErrorResponse,
    state::AppState,
};

const GLOBAL_CHANNEL: &str = "fiestaaa:global";

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

pub async fn publish_event(redis: &Option<RedisClient>, event_id: i64, payload: &impl Serialize) {
    let ch = event_channel(event_id);
    publish_json(redis, &ch, payload).await;
}

#[get("/ws")]
pub async fn websocket(
    state: Data<AppState>,
    req: HttpRequest,
    stream: web::Payload,
) -> Result<HttpResponse, actix_web::Error> {
    let params: EventWsQuery = serde_urlencoded::from_str(req.query_string()).unwrap_or_default();
    let claims = match extract_claims_from_auth(&req, &state.jwt_secret) {
        Ok(c) => c,
        Err(_) => {
            if let Some(token) = params.token.as_deref() {
                match decode_jwt(token, &state.jwt_secret) {
                    Ok(claims) => claims,
                    Err(resp) => return Ok(resp),
                }
            } else {
                return Ok(HttpResponse::Unauthorized().json(ErrorResponse {
                    error: "missing_authorization_header".into(),
                    details: None,
                }));
            }
        }
    };

    let event_id = params.event_id;
    let ws = WsSession {
        email: claims.sub.to_lowercase(),
        redis_client: state.redis_client.clone(),
        event_id,
        hb: Instant::now(),
    };

    ws::start(ws, &req, stream)
}

#[derive(Debug, serde::Deserialize, Clone, Default)]
#[serde(default)]
pub struct EventWsQuery {
    pub event_id: Option<i64>,
    pub token: Option<String>,
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
                // echo simple ping/pong or ignore
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
