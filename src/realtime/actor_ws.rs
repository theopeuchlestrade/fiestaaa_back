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

            // Actor-owned futures are dropped when the socket stops, including
            // the Redis connection and any pending subscription/read.
            ctx.spawn(
                async move {
                    let setup = async {
                        let mut pubsub = client.get_async_pubsub().await?;
                        for channel in &channels {
                            pubsub.subscribe(channel).await?;
                        }
                        Ok::<_, redis::RedisError>(pubsub)
                    };
                    let mut pubsub =
                        match tokio::time::timeout(Duration::from_secs(15), setup).await {
                            Ok(Ok(pubsub)) => pubsub,
                            _ => return "subscription_failed",
                        };
                    if addr
                        .send(RedisMessage(json!({"type": "realtime.ready"}).to_string()))
                        .await
                        .is_err()
                    {
                        return "socket_closed";
                    }
                    let mut messages = pubsub.on_message();
                    while let Some(message) = messages.next().await {
                        if let Ok(payload) = message.get_payload::<String>()
                            && addr.send(RedisMessage(payload)).await.is_err()
                        {
                            return "socket_closed";
                        }
                    }
                    "redis_stream_ended"
                }
                .into_actor(self)
                .map(|reason, _, ctx| {
                    warn!("ws realtime interrupted: {reason}");
                    ctx.close(Some(ws::CloseReason {
                        code: ws::CloseCode::Error,
                        description: Some("realtime_unavailable".into()),
                    }));
                    ctx.stop();
                }),
            );
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

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::{Error, web::Bytes};
    use futures_util::{Stream, stream};

    fn socket(client: Option<RedisClient>) -> impl Stream<Item = Result<Bytes, Error>> {
        ws::WebsocketContext::create(
            WsSession {
                redis_client: client,
                event_id: Some(987654321),
                auth_exp: (now_ts() + 3600) as usize,
                hb: Instant::now(),
            },
            stream::pending(),
        )
    }

    async fn next_frame(stream: &mut (impl Stream<Item = Result<Bytes, Error>> + Unpin)) -> Bytes {
        tokio::time::timeout(Duration::from_secs(5), stream.next())
            .await
            .expect("socket should respond")
            .expect("frame")
            .expect("valid frame")
    }

    #[actix_web::test]
    async fn disabled_and_failed_redis_are_not_reported_as_ready() {
        let mut disabled = Box::pin(socket(None));
        let frame = next_frame(&mut disabled).await;
        assert!(String::from_utf8_lossy(&frame).contains("realtime_disabled"));

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let mut unavailable = Box::pin(socket(Some(
            RedisClient::open(format!("redis://127.0.0.1:{port}")).unwrap(),
        )));
        let frame = next_frame(&mut unavailable).await;
        assert_eq!(frame[0] & 0x0f, 8, "failed setup closes the websocket");
    }

    #[actix_web::test]
    async fn redis_disconnect_closes_socket_and_socket_drop_releases_subscription() {
        let Ok(url) = std::env::var("TEST_REDIS_URL") else {
            assert!(
                std::env::var("CI").is_err(),
                "CI requires isolated TEST_REDIS_URL"
            );
            eprintln!("Skipping Redis integration test: set isolated TEST_REDIS_URL");
            return;
        };
        let client = RedisClient::open(url).unwrap();
        let mut connection = client.get_multiplexed_async_connection().await.unwrap();
        let mut first = Box::pin(socket(Some(client.clone())));
        assert!(String::from_utf8_lossy(&next_frame(&mut first).await).contains("realtime.ready"));
        // This URL is exclusively for tests: kill only PubSub clients, leaving
        // the command connection alive to check cleanup and recovery.
        let _: i64 = redis::cmd("CLIENT")
            .arg("KILL")
            .arg("TYPE")
            .arg("pubsub")
            .query_async(&mut connection)
            .await
            .unwrap();
        let frame = next_frame(&mut first).await;
        assert_eq!(frame[0] & 0x0f, 8);
        drop(first);

        let mut recovered = Box::pin(socket(Some(client)));
        assert!(
            String::from_utf8_lossy(&next_frame(&mut recovered).await).contains("realtime.ready")
        );
        drop(recovered);
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let values: Vec<(String, i64)> = redis::cmd("PUBSUB")
                    .arg("NUMSUB")
                    .arg(event_channel(987654321))
                    .query_async(&mut connection)
                    .await
                    .unwrap();
                if values[0].1 == 0 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("dropping socket must release its Redis subscription");
    }
}
