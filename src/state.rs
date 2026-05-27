use std::collections::HashSet;

use crate::notifications::NotificationService;
use crate::rate_limit::AuthRateLimiter;
use redis::Client as RedisClient;
use sqlx::{Pool, Postgres};

pub struct AppState {
    pub db: Pool<Postgres>,
    pub jwt_secret: String,
    pub admin_emails: HashSet<String>,
    pub trust_proxy_headers: bool,
    pub http_client: reqwest::Client,
    pub geocoding_base_url: String,
    pub geocoding_country_codes: Option<String>,
    pub invitation_email_sender: Option<String>,
    pub invitation_email_api_key: Option<String>,
    pub app_base_url: String,
    pub cors_allowed_origins: HashSet<String>,
    pub avatar_upload_dir: String,
    pub avatar_base_url: String,
    pub redis_client: Option<RedisClient>,
    pub notifications: NotificationService,
    pub fcm_project_id: Option<String>,
    pub google_client_id: Option<String>,
    pub google_android_client_id: Option<String>,
    pub google_ios_client_id: Option<String>,
    pub apple_app_id: Option<String>,
    pub apple_service_id: Option<String>,
    pub google_tokeninfo_url: String,
    pub google_userinfo_url: String,
    pub apple_jwks_url: String,
    pub auth_rate_limiter: AuthRateLimiter,
    pub invitation_rate_limiter: AuthRateLimiter,
}
