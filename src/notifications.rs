use std::collections::HashMap;

use log::{debug, warn};
use redis::Client as RedisClient;
use serde::Deserialize;
use serde_json::Value;
use sqlx::{Pool, Postgres, Row};

const FCM_ENDPOINT: &str = "https://fcm.googleapis.com/fcm/send";
#[derive(Clone)]
pub struct NotificationService {
    pub server_key: Option<String>,
    pub redis_client: Option<RedisClient>,
    pub http_client: reqwest::Client,
    pub default_dedup_ttl_seconds: u64,
}

impl NotificationService {
    pub fn new(
        server_key: Option<String>,
        redis_client: Option<RedisClient>,
        http_client: reqwest::Client,
        default_dedup_ttl_seconds: u64,
    ) -> Self {
        Self {
            server_key,
            redis_client,
            http_client,
            default_dedup_ttl_seconds,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.server_key.is_some()
    }

    pub async fn send_to_tokens(
        &self,
        db: &Pool<Postgres>,
        tokens: Vec<String>,
        title: &str,
        body: &str,
        data: Value,
        dedup_key: Option<String>,
        dedup_ttl: Option<u64>,
    ) {
        if self.server_key.is_none() || tokens.is_empty() {
            return;
        }

        let ttl = dedup_ttl.unwrap_or(self.default_dedup_ttl_seconds);
        if let Some(key) = dedup_key.as_ref() {
            if self.is_throttled(key, ttl).await {
                debug!("notification skipped by throttle for key {key}");
                return;
            }
        }

        let payload = serde_json::json!({
            "registration_ids": tokens,
            "notification": {
                "title": title,
                "body": body,
            },
            "data": data,
            "priority": "high"
        });

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
                return;
            }
        };

        let status = resp.status();
        let parsed = resp.json::<FcmResponse>().await;
        match parsed {
            Ok(body) => {
                if status.is_client_error() || status.is_server_error() {
                    warn!(
                        "FCM responded with status {} (success {:?}, failure {:?})",
                        status, body.success, body.failure
                    );
                }

                if let Err(err) = handle_invalid_tokens(db, &tokens, &body).await {
                    warn!("failed to prune invalid FCM tokens: {err}");
                }
            }
            Err(err) => {
                warn!("failed to parse FCM response: {err}");
            }
        }
    }

    async fn is_throttled(&self, key: &str, ttl_seconds: u64) -> bool {
        if let Some(client) = self.redis_client.as_ref() {
            if let Ok(mut conn) = client.get_multiplexed_async_connection().await {
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
                        return false;
                    }
                }
            }
        }
        false
    }
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
            ) {
                if let Some(tok) = sent_tokens.get(idx) {
                    invalid.push(tok.clone());
                }
            }
        }

        if !invalid.is_empty() {
            sqlx::query("UPDATE user_devices SET disabled_at = NOW() WHERE fcm_token = ANY($1)")
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
    sqlx::query_scalar::<_, i64>("SELECT id FROM users WHERE lower(email) = lower($1)")
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
        "SELECT user_id, fcm_token FROM user_devices
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
    title: &str,
    body: &str,
    data: Value,
    dedup_base_key: Option<&str>,
    dedup_ttl: Option<u64>,
) {
    if !service.is_enabled() {
        return;
    }

    let tokens = match tokens_by_user_ids(db, user_ids).await {
        Ok(map) => map,
        Err(err) => {
            warn!("failed to load device tokens: {err}");
            return;
        }
    };

    for user_id in user_ids {
        if let Some(user_tokens) = tokens.get(user_id) {
            let dedup_key = dedup_base_key.map(|base| format!("{base}:{user_id}"));
            service
                .send_to_tokens(
                    db,
                    user_tokens.clone(),
                    title,
                    body,
                    data.clone(),
                    dedup_key,
                    dedup_ttl,
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
        "SELECT id FROM users WHERE lower(email) = (
            SELECT lower(owner_email) FROM events WHERE event_id = $1
        )",
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
