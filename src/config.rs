use dotenvy::dotenv;
use log::warn;
use std::collections::HashSet;

fn load_resend_api_key() -> Option<String> {
    if let Ok(value) = std::env::var("RESEND_API_KEY") {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return None;
        }
        if !trimmed.starts_with("re_") {
            warn!(
                "RESEND_API_KEY est defini mais ne ressemble pas a une cle Resend (prefixe re_) ; ignore"
            );
            return None;
        }
        return Some(trimmed.to_string());
    }
    None
}

fn default_cors_allowed_origins(app_base_url: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut seen = HashSet::new();

    for candidate in [
        app_base_url.trim().trim_end_matches('/'),
        "http://localhost:3000",
        "http://127.0.0.1:3000",
        "http://localhost:5001",
        "http://127.0.0.1:5001",
    ] {
        if !candidate.is_empty() && seen.insert(candidate.to_string()) {
            values.push(candidate.to_string());
        }
    }

    values
}

pub struct AppConfig {
    pub host: String,
    pub port: u16,
    pub database_url: String,
    pub jwt_secret: String,
    pub admin_emails: Vec<String>,
    pub trust_proxy_headers: bool,
    pub geocoding_base_url: String,
    pub geocoding_user_agent: String,
    pub geocoding_country_codes: Option<String>,
    pub invitation_email_sender: Option<String>,
    pub invitation_email_api_key: Option<String>,
    pub app_base_url: String,
    pub avatar_upload_dir: String,
    pub avatar_base_url: String,
    pub redis_url: Option<String>,
    pub fcm_server_key: Option<String>,
    pub fcm_vapid_key: Option<String>,
    pub notification_dedup_ttl_seconds: u64,
    pub fcm_service_account_path: Option<String>,
    pub fcm_project_id: Option<String>,
    pub event_cleanup_days: i64,
    pub event_cleanup_interval_hours: u64,
    pub google_client_id: Option<String>,
    pub google_android_client_id: Option<String>,
    pub google_ios_client_id: Option<String>,
    pub apple_app_id: Option<String>,
    pub apple_service_id: Option<String>,
    pub cors_allowed_origins: Vec<String>,
    pub auth_rate_limit_max_attempts: usize,
    pub auth_rate_limit_window_seconds: u64,
}

impl AppConfig {
    pub fn from_env() -> Self {
        let _ = dotenv();
        let host = std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".into());
        let port = std::env::var("PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(8080);
        let database_url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/fiestaaa".into());
        let jwt_secret = std::env::var("JWT_SECRET").unwrap_or_default();
        if jwt_secret.len() < 32 {
            panic!("JWT_SECRET doit être défini et contenir au moins 32 caractères");
        }
        let admin_emails_raw = std::env::var("ADMIN_EMAILS").unwrap_or_default();
        let admin_emails = admin_emails_raw
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_lowercase())
            .collect::<Vec<_>>();
        if admin_emails.is_empty() {
            warn!("ADMIN_EMAILS n'est pas defini ; les endpoints admin seront desactives");
        }
        let trust_proxy_headers = std::env::var("TRUST_PROXY_HEADERS")
            .ok()
            .map(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or(false);
        let geocoding_base_url = std::env::var("GEOCODING_BASE_URL")
            .unwrap_or_else(|_| "https://nominatim.openstreetmap.org".into());
        let geocoding_user_agent =
            std::env::var("GEOCODING_USER_AGENT").unwrap_or_else(|_| "fiestaaa-backend/0.1".into());
        let geocoding_country_codes = std::env::var("GEOCODING_COUNTRY_CODES")
            .ok()
            .filter(|v| !v.trim().is_empty());
        let invitation_email_sender = std::env::var("INVITATION_EMAIL_SENDER")
            .ok()
            .filter(|v| !v.trim().is_empty());
        let invitation_email_api_key = load_resend_api_key();
        let app_base_url =
            std::env::var("APP_BASE_URL").unwrap_or_else(|_| "https://fiestaaa.app".into());
        let avatar_upload_dir =
            std::env::var("AVATAR_UPLOAD_DIR").unwrap_or_else(|_| "./uploads/avatars".into());
        let avatar_base_url = std::env::var("AVATAR_BASE_URL")
            .unwrap_or_else(|_| "http://localhost:8080/media/avatars".into());
        let redis_url = std::env::var("REDIS_URL")
            .ok()
            .filter(|v| !v.trim().is_empty());
        let fcm_server_key = std::env::var("FCM_SERVER_KEY")
            .ok()
            .filter(|v| !v.trim().is_empty());
        let fcm_vapid_key = std::env::var("FIESTAAA_FCM_VAPID_KEY")
            .ok()
            .filter(|v| !v.trim().is_empty());
        let notification_dedup_ttl_seconds = std::env::var("NOTIFICATION_DEDUP_TTL_SECONDS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(300);
        let fcm_service_account_path = std::env::var("FCM_SERVICE_ACCOUNT_PATH")
            .ok()
            .filter(|v| !v.trim().is_empty());
        let fcm_project_id = std::env::var("FCM_PROJECT_ID")
            .ok()
            .filter(|v| !v.trim().is_empty());
        let event_cleanup_days = std::env::var("EVENT_CLEANUP_DAYS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(7);
        let event_cleanup_interval_hours = std::env::var("EVENT_CLEANUP_INTERVAL_HOURS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1);
        let google_client_id = std::env::var("FIESTAAA_GOOGLE_WEB_CLIENT_ID")
            .ok()
            .filter(|v| !v.trim().is_empty());
        let google_android_client_id = std::env::var("FIESTAAA_GOOGLE_ANDROID_CLIENT_ID")
            .ok()
            .filter(|v| !v.trim().is_empty());
        let google_ios_client_id = std::env::var("FIESTAAA_GOOGLE_IOS_CLIENT_ID")
            .ok()
            .filter(|v| !v.trim().is_empty());
        let apple_app_id = std::env::var("FIESTAAA_APPLE_APP_ID")
            .ok()
            .filter(|v| !v.trim().is_empty());
        let apple_service_id = std::env::var("FIESTAAA_APPLE_SERVICE_ID")
            .ok()
            .filter(|v| !v.trim().is_empty());
        let cors_allowed_origins = std::env::var("CORS_ALLOWED_ORIGINS")
            .ok()
            .map(|v| {
                v.split(',')
                    .filter_map(|s| {
                        let normalized = s.trim().trim_end_matches('/');
                        if normalized.is_empty() {
                            None
                        } else {
                            Some(normalized.to_string())
                        }
                    })
                    .collect()
            })
            .filter(|origins: &Vec<String>| !origins.is_empty())
            .unwrap_or_else(|| default_cors_allowed_origins(&app_base_url));
        let auth_rate_limit_max_attempts = std::env::var("AUTH_RATE_LIMIT_MAX_ATTEMPTS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(20);
        let auth_rate_limit_window_seconds = std::env::var("AUTH_RATE_LIMIT_WINDOW_SECONDS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(60);
        Self {
            host,
            port,
            database_url,
            jwt_secret,
            admin_emails,
            trust_proxy_headers,
            geocoding_base_url,
            geocoding_user_agent,
            geocoding_country_codes,
            invitation_email_sender,
            invitation_email_api_key,
            app_base_url,
            avatar_upload_dir,
            avatar_base_url,
            redis_url,
            fcm_server_key,
            fcm_vapid_key,
            notification_dedup_ttl_seconds,
            fcm_service_account_path,
            fcm_project_id,
            event_cleanup_days,
            event_cleanup_interval_hours,
            google_client_id,
            google_android_client_id,
            google_ios_client_id,
            apple_app_id,
            apple_service_id,
            cors_allowed_origins,
            auth_rate_limit_max_attempts,
            auth_rate_limit_window_seconds,
        }
    }
}
