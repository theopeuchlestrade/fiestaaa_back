use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use log::{debug, warn};
use redis::Client as RedisClient;
use serde::Deserialize;
use serde_json::Value;
use sqlx::{Pool, Postgres, Row};

use crate::observability;

const FCM_ENDPOINT: &str = "https://fcm.googleapis.com/fcm/send";
const FCM_V1_BASE: &str = "https://fcm.googleapis.com/v1";
const FCM_SCOPE: &str = "https://www.googleapis.com/auth/firebase.messaging";

#[derive(Debug, Deserialize)]
struct ServiceAccountProject {
    project_id: Option<String>,
}
#[derive(Clone)]
pub struct NotificationService {
    pub server_key: Option<String>,
    pub redis_client: Option<RedisClient>,
    pub http_client: reqwest::Client,
    pub default_dedup_ttl_seconds: u64,
    pub fcm_project_id: Option<String>,
    pub fcm_token_provider: Option<Arc<dyn gcp_auth::TokenProvider + Send + Sync>>,
}

pub struct NotificationMessage<'a> {
    pub title: &'a str,
    pub body: &'a str,
    pub data: Value,
}

pub struct NotificationRequest<'a> {
    pub title: &'a str,
    pub body: &'a str,
    pub data: Value,
    pub dedup_base_key: Option<&'a str>,
    pub dedup_ttl: Option<u64>,
}

pub struct NotificationTarget {
    pub tokens: Vec<String>,
    pub dedup_key: Option<String>,
    pub dedup_ttl: Option<u64>,
}

struct FcmV1Context<'a> {
    db: &'a Pool<Postgres>,
    project_id: &'a str,
    provider: Arc<dyn gcp_auth::TokenProvider + Send + Sync>,
}

impl NotificationService {
    pub fn new(
        server_key: Option<String>,
        service_account_path: Option<String>,
        fcm_project_id: Option<String>,
        redis_client: Option<RedisClient>,
        http_client: reqwest::Client,
        default_dedup_ttl_seconds: u64,
    ) -> Self {
        let fcm_token_provider = service_account_path.as_ref().and_then(|path| {
            let pb = PathBuf::from(path);
            match gcp_auth::CustomServiceAccount::from_file(pb) {
                Ok(sa) => Some(Arc::new(sa) as Arc<dyn gcp_auth::TokenProvider + Send + Sync>),
                Err(err) => {
                    warn!("failed to load FCM service account: {err}");
                    observability::record_push_error("service_account_load_failed");
                    observability::capture_message(
                        sentry::Level::Error,
                        &format!("failed to load FCM service account: {err}"),
                    );
                    None
                }
            }
        });

        let derived_project_id = if fcm_project_id.is_none() {
            service_account_path
                .as_ref()
                .and_then(|path| fs::read_to_string(path).ok())
                .and_then(|txt| serde_json::from_str::<ServiceAccountProject>(&txt).ok())
                .and_then(|sa| sa.project_id)
        } else {
            fcm_project_id.clone()
        };

        Self {
            server_key,
            redis_client,
            http_client,
            default_dedup_ttl_seconds,
            fcm_project_id: derived_project_id,
            fcm_token_provider,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.server_key.is_some()
            || (self.fcm_token_provider.is_some() && self.fcm_project_id.is_some())
    }

    pub async fn send_to_tokens(
        &self,
        db: &Pool<Postgres>,
        target: NotificationTarget,
        message: &NotificationMessage<'_>,
    ) {
        if target.tokens.is_empty() {
            return;
        }

        let ttl = target.dedup_ttl.unwrap_or(self.default_dedup_ttl_seconds);
        if let Some(key) = target.dedup_key.as_ref()
            && self.is_throttled(key, ttl).await
        {
            debug!("notification skipped by throttle for key {key}");
            return;
        }

        if let (Some(project_id), Some(provider)) =
            (&self.fcm_project_id, self.fcm_token_provider.as_ref())
        {
            let ctx = FcmV1Context {
                db,
                project_id,
                provider: provider.clone(),
            };
            for tok in target.tokens {
                self.send_v1(&ctx, &tok, message).await;
            }
            return;
        }

        if self.server_key.is_none() {
            return;
        }

        let payload = legacy_payload(&target.tokens, message);

        let auth_header = format!("key={}", self.server_key.as_ref().unwrap());
        let response = self
            .http_client
            .post(FCM_ENDPOINT)
            .header("Authorization", auth_header)
            .json(&payload)
            .send()
            .await;

        let resp = match response {
            Ok(resp) => resp,
            Err(err) => {
                warn!("failed to send FCM request: {err}");
                observability::record_push_error("legacy_transport_failure");
                observability::capture_message(
                    sentry::Level::Error,
                    &format!("failed to send FCM request: {err}"),
                );
                return;
            }
        };

        let status = resp.status();
        let body_text = resp.text().await.unwrap_or_default();
        let parsed = serde_json::from_str::<FcmResponse>(&body_text);
        match parsed {
            Ok(body) => {
                if status.is_client_error() || status.is_server_error() {
                    warn!(
                        "FCM responded with status {} (success {:?}, failure {:?})",
                        status, body.success, body.failure
                    );
                    observability::record_push_error("legacy_provider_failure");
                } else {
                    debug!(
                        "FCM responded with status {} (success {:?}, failure {:?})",
                        status, body.success, body.failure
                    );
                }

                if let Err(err) = handle_invalid_tokens(db, &target.tokens, &body).await {
                    warn!("failed to prune invalid FCM tokens: {err}");
                    observability::record_push_error("legacy_invalid_token_prune_failed");
                }
            }
            Err(err) => {
                let snippet = &body_text.chars().take(200).collect::<String>();
                warn!(
                    "failed to parse FCM response (status {}): {} body_snippet='{}'",
                    status, err, snippet
                );
                observability::record_push_error("legacy_response_parse_failure");
            }
        }
    }

    async fn send_v1(
        &self,
        ctx: &FcmV1Context<'_>,
        token: &str,
        message: &NotificationMessage<'_>,
    ) {
        let url = format!("{FCM_V1_BASE}/projects/{}/messages:send", ctx.project_id);
        let access = match ctx.provider.token(&[FCM_SCOPE]).await {
            Ok(tok) => tok.as_str().to_string(),
            Err(err) => {
                warn!("fcm v1 auth token error: {err}");
                observability::record_push_error("v1_auth_token_failure");
                observability::capture_message(
                    sentry::Level::Error,
                    &format!("fcm v1 auth token error: {err}"),
                );
                return;
            }
        };

        let payload = fcm_v1_payload(token, message);

        let resp = match self
            .http_client
            .post(url)
            .bearer_auth(access)
            .json(&payload)
            .send()
            .await
        {
            Ok(r) => r,
            Err(err) => {
                warn!("failed to send FCM v1 request: {err}");
                observability::record_push_error("v1_transport_failure");
                observability::capture_message(
                    sentry::Level::Error,
                    &format!("failed to send FCM v1 request: {err}"),
                );
                return;
            }
        };

        let status = resp.status();
        let body_txt = resp.text().await.unwrap_or_default();
        if status.is_client_error() || status.is_server_error() {
            warn!(
                "FCM v1 status {} body_snippet='{}'",
                status,
                body_txt.chars().take(200).collect::<String>()
            );
            observability::record_push_error("v1_provider_failure");
        } else {
            debug!(
                "FCM v1 status {} body_snippet='{}'",
                status,
                body_txt.chars().take(120).collect::<String>()
            );
        }

        if status == reqwest::StatusCode::NOT_FOUND
            || body_txt.contains("UNREGISTERED")
            || body_txt.contains("INVALID_ARGUMENT")
        {
            let _ = sqlx::query(
                "UPDATE user_devices
                 SET disabled_at = NOW()
                 WHERE fiestaaa_lookup_matches(fcm_token_lookup_hash, $1)",
            )
            .bind(token)
            .execute(ctx.db)
            .await;
        }
    }

    async fn is_throttled(&self, key: &str, ttl_seconds: u64) -> bool {
        if let Some(client) = self.redis_client.as_ref()
            && let Ok(mut conn) = client.get_multiplexed_async_connection().await
        {
            let inserted: redis::RedisResult<i32> = redis::cmd("SETNX")
                .arg(key)
                .arg("1")
                .query_async(&mut conn)
                .await;
            match inserted {
                Ok(1) => {
                    let _ = redis::cmd("EXPIRE")
                        .arg(key)
                        .arg(ttl_seconds as i64)
                        .query_async::<()>(&mut conn)
                        .await;
                    return false;
                }
                Ok(_) => return true,
                Err(err) => {
                    warn!("redis throttle error: {err}");
                    observability::record_push_error("throttle_redis_error");
                    return false;
                }
            }
        }
        false
    }
}

fn data_to_string_map(data: &Value) -> HashMap<String, String> {
    match data {
        Value::Object(map) => map
            .iter()
            .map(|(k, v)| {
                let s = if let Some(str_val) = v.as_str() {
                    str_val.to_string()
                } else {
                    v.to_string()
                };
                (k.clone(), s)
            })
            .collect(),
        _ => HashMap::new(),
    }
}

fn legacy_payload(tokens: &[String], message: &NotificationMessage<'_>) -> Value {
    serde_json::json!({
        "registration_ids": tokens,
        "notification": {
            "title": message.title,
            "body": message.body,
            "sound": "default"
        },
        "data": message.data,
        "priority": "high"
    })
}

fn fcm_v1_payload(token: &str, message: &NotificationMessage<'_>) -> Value {
    serde_json::json!({
        "message": {
            "token": token,
            "notification": {
                "title": message.title,
                "body": message.body
            },
            "data": data_to_string_map(&message.data),
            "android": {
                "priority": "HIGH",
                "notification": {
                    "channel_id": "fiestaaa_fcm",
                    "sound": "default"
                }
            },
            "apns": {
                "headers": {
                    "apns-priority": "10",
                    "apns-push-type": "alert"
                },
                "payload": {
                    "aps": {
                        "alert": {
                            "title": message.title,
                            "body": message.body
                        },
                        "sound": "default"
                    }
                }
            }
        }
    })
}

#[derive(Debug, Deserialize)]
struct FcmResponse {
    pub success: Option<i32>,
    pub failure: Option<i32>,
    pub results: Option<Vec<FcmResult>>,
}

#[derive(Debug, Deserialize)]
struct FcmResult {
    pub error: Option<String>,
}

async fn handle_invalid_tokens(
    db: &Pool<Postgres>,
    sent_tokens: &[String],
    response: &FcmResponse,
) -> Result<(), sqlx::Error> {
    if let Some(results) = response.results.as_ref() {
        let mut invalid: Vec<String> = Vec::new();
        for (idx, result) in results.iter().enumerate() {
            let Some(error) = result.error.as_deref() else {
                continue;
            };

            if matches!(
                error,
                "NotRegistered" | "InvalidRegistration" | "MissingRegistration"
            ) && let Some(tok) = sent_tokens.get(idx)
            {
                invalid.push(tok.clone());
            }
        }

        if !invalid.is_empty() {
            sqlx::query(
                "UPDATE user_devices
                 SET disabled_at = NOW()
                 WHERE fcm_token_lookup_hash = ANY(
                    ARRAY(SELECT fiestaaa_lookup_text(value) FROM unnest($1::text[]) AS value)
                 )",
            )
            .bind(&invalid)
            .execute(db)
            .await?;
        }
    }
    Ok(())
}

pub async fn find_user_id_by_email(
    db: &Pool<Postgres>,
    email: &str,
) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar::<_, i64>(
        "SELECT id FROM users WHERE fiestaaa_email_matches(email_lookup_hash, $1)",
    )
    .bind(email)
    .fetch_optional(db)
    .await
}

pub async fn tokens_by_user_ids(
    db: &Pool<Postgres>,
    user_ids: &[i64],
) -> Result<HashMap<i64, Vec<String>>, sqlx::Error> {
    if user_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let rows = sqlx::query(
        "SELECT user_id, fiestaaa_decrypt_text(fcm_token_ciphertext) AS fcm_token
         FROM user_devices
         WHERE disabled_at IS NULL AND user_id = ANY($1)",
    )
    .bind(user_ids)
    .fetch_all(db)
    .await?;

    let mut map: HashMap<i64, Vec<String>> = HashMap::new();
    for row in rows {
        let user_id: i64 = row.get("user_id");
        let token: String = row.get("fcm_token");
        map.entry(user_id).or_default().push(token);
    }
    Ok(map)
}

pub async fn notify_users(
    service: &NotificationService,
    db: &Pool<Postgres>,
    user_ids: &[i64],
    request: NotificationRequest<'_>,
) {
    if !service.is_enabled() {
        return;
    }

    let NotificationRequest {
        title,
        body,
        data,
        dedup_base_key,
        dedup_ttl,
    } = request;
    let message = NotificationMessage { title, body, data };
    let tokens = match tokens_by_user_ids(db, user_ids).await {
        Ok(map) => map,
        Err(err) => {
            warn!("failed to load device tokens: {err}");
            observability::record_push_error("device_token_load_failed");
            return;
        }
    };

    for user_id in user_ids {
        if let Some(user_tokens) = tokens.get(user_id) {
            let dedup_key = dedup_base_key.map(|base| format!("{base}:{user_id}"));
            service
                .send_to_tokens(
                    db,
                    NotificationTarget {
                        tokens: user_tokens.clone(),
                        dedup_key,
                        dedup_ttl,
                    },
                    &NotificationMessage {
                        title: message.title,
                        body: message.body,
                        data: message.data.clone(),
                    },
                )
                .await;
        }
    }
}

pub async fn event_member_user_ids(
    db: &Pool<Postgres>,
    event_id: i64,
) -> Result<Vec<i64>, sqlx::Error> {
    let owner_id = sqlx::query_scalar::<_, i64>(
        "SELECT owner_user_id
         FROM events
         WHERE event_id = $1",
    )
    .bind(event_id)
    .fetch_optional(db)
    .await?;

    let mut ids = sqlx::query_scalar::<_, i64>(
        "SELECT DISTINCT user_id FROM invitations WHERE event_id = $1 AND status NOT IN ('Declined', 'Expired')",
    )
    .bind(event_id)
    .fetch_all(db)
    .await?;

    if let Some(owner) = owner_id {
        ids.push(owner);
    }

    ids.sort();
    ids.dedup();
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn fcm_v1_payload_sets_apns_alert_delivery_options() {
        let message = NotificationMessage {
            title: "Nouvelle demande d'ami",
            body: "alice souhaite t'ajouter",
            data: json!({
                "type": "friend_request",
                "request_id": 42
            }),
        };

        let payload = fcm_v1_payload("device-token", &message);

        assert_eq!(
            payload
                .pointer("/message/apns/headers/apns-push-type")
                .and_then(Value::as_str),
            Some("alert")
        );
        assert_eq!(
            payload
                .pointer("/message/apns/headers/apns-priority")
                .and_then(Value::as_str),
            Some("10")
        );
        assert_eq!(
            payload
                .pointer("/message/apns/payload/aps/sound")
                .and_then(Value::as_str),
            Some("default")
        );
        assert_eq!(
            payload
                .pointer("/message/apns/payload/aps/alert/title")
                .and_then(Value::as_str),
            Some("Nouvelle demande d'ami")
        );
        assert_eq!(
            payload
                .pointer("/message/data/request_id")
                .and_then(Value::as_str),
            Some("42")
        );
    }

    #[test]
    fn legacy_payload_sets_default_notification_sound() {
        let message = NotificationMessage {
            title: "Titre",
            body: "Corps",
            data: json!({ "type": "friend_request" }),
        };
        let tokens = vec!["token-1".to_string()];

        let payload = legacy_payload(&tokens, &message);

        assert_eq!(
            payload
                .pointer("/notification/sound")
                .and_then(Value::as_str),
            Some("default")
        );
        assert_eq!(
            payload.pointer("/priority").and_then(Value::as_str),
            Some("high")
        );
    }
}
