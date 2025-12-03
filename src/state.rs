use std::collections::HashSet;

use crate::notifications::NotificationService;
use redis::Client as RedisClient;
use sqlx::{Pool, Postgres};

pub struct AppState {
    pub db: Pool<Postgres>,
    pub jwt_secret: String,
    pub admin_emails: HashSet<String>,
    pub http_client: reqwest::Client,
    pub geocoding_base_url: String,
    pub geocoding_country_codes: Option<String>,
    pub invitation_email_sender: Option<String>,
    pub invitation_email_api_key: Option<String>,
    pub app_base_url: String,
    pub avatar_upload_dir: String,
    pub avatar_base_url: String,
    pub redis_client: Option<RedisClient>,
    pub notifications: NotificationService,
    pub fcm_project_id: Option<String>,
}
