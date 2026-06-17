#![allow(deprecated)]

use std::time::{Duration, Instant};

use actix::prelude::*;
use actix_web::{HttpRequest, HttpResponse, web};
use actix_web_actors::ws;
use futures_util::StreamExt;
use log::warn;
use redis::Client as RedisClient;
use serde_json::json;

use crate::{auth::now_ts, realtime::event_channel};

use super::GLOBAL_CHANNEL;

#[derive(Message)]
#[rtype(result = "()")]
struct RedisMessage(String);

struct WsSession {
    redis_client: Option<RedisClient>,
    event_id: Option<i64>,
    auth_exp: usize,
    hb: Instant,
}

impl WsSession {
    fn hb(&self, ctx: &mut ws::WebsocketContext<Self>) {
        ctx.run_interval(Duration::from_secs(15), |act, ctx| {
            if now_ts() >= act.auth_exp as u64 {
                ctx.close(None);
                ctx.stop();
                return;
            }
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

pub(super) fn start_actor_websocket(
    redis_client: Option<RedisClient>,
    event_id: Option<i64>,
    auth_exp: usize,
    req: &HttpRequest,
    stream: web::Payload,
) -> Result<HttpResponse, actix_web::Error> {
    let ws = WsSession {
        redis_client,
        event_id,
        auth_exp,
        hb: Instant::now(),
    };

    ws::start(ws, req, stream)
}
