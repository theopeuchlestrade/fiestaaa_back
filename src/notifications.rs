use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use log::{debug, warn};
use redis::Client as RedisClient;
use serde::Deserialize;
use serde_json::Value;
use sqlx::{Pool, Postgres, Row};

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
        tokens: Vec<String>,
        title: &str,
        body: &str,
        data: Value,
        dedup_key: Option<String>,
        dedup_ttl: Option<u64>,
    ) {
        if tokens.is_empty() {
            return;
        }

        let ttl = dedup_ttl.unwrap_or(self.default_dedup_ttl_seconds);
        if let Some(key) = dedup_key.as_ref() {
            if self.is_throttled(key, ttl).await {
                debug!("notification skipped by throttle for key {key}");
                return;
            }
        }

        if let (Some(project_id), Some(provider)) =
            (&self.fcm_project_id, self.fcm_token_provider.as_ref())
        {
            for tok in tokens {
                self.send_v1(
                    db,
                    project_id,
                    provider.clone(),
                    &tok,
                    title,
                    body,
                    data.clone(),
                )
                .await;
            }
            return;
        }

        if self.server_key.is_none() {
            return;
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
        let body_text = resp.text().await.unwrap_or_default();
        let parsed = serde_json::from_str::<FcmResponse>(&body_text);
        match parsed {
            Ok(body) => {
                if status.is_client_error() || status.is_server_error() {
                    warn!(
                        "FCM responded with status {} (success {:?}, failure {:?})",
                        status, body.success, body.failure
                    );
                } else {
                    debug!(
                        "FCM responded with status {} (success {:?}, failure {:?})",
                        status, body.success, body.failure
                    );
                }

                if let Err(err) = handle_invalid_tokens(db, &tokens, &body).await {
                    warn!("failed to prune invalid FCM tokens: {err}");
                }
            }
            Err(err) => {
                let snippet = &body_text.chars().take(200).collect::<String>();
                warn!(
                    "failed to parse FCM response (status {}): {} body_snippet='{}'",
                    status, err, snippet
                );
            }
        }
    }

    async fn send_v1(
        &self,
        db: &Pool<Postgres>,
        project_id: &str,
        provider: Arc<dyn gcp_auth::TokenProvider + Send + Sync>,
        token: &str,
        title: &str,
        body: &str,
        data: Value,
    ) {
        let url = format!("{FCM_V1_BASE}/projects/{project_id}/messages:send");
        let access = match provider.token(&[FCM_SCOPE]).await {
            Ok(tok) => tok.as_str().to_string(),
            Err(err) => {
                warn!("fcm v1 auth token error: {err}");
                return;
            }
        };

        let data_map = data_to_string_map(&data);

        let payload = serde_json::json!({
            "message": {
                "token": token,
                "notification": { "title": title, "body": body },
                "data": data_map
            }
        });

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
            let _ = sqlx::query("UPDATE user_devices SET disabled_at = NOW() WHERE fcm_token = $1")
                .bind(token)
                .execute(db)
                .await;
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
