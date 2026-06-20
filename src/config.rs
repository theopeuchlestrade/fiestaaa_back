use crate::load_dotenv_from_repo;
use log::warn;
use std::collections::HashSet;
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigValidationError(String);

impl fmt::Display for ConfigValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ConfigValidationError {}

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

fn read_bool_env(name: &str, default: bool) -> Result<bool, ConfigValidationError> {
    let Ok(value) = std::env::var(name) else {
        return Ok(default);
    };
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(ConfigValidationError(format!(
            "{name} doit être un booléen: true/false, yes/no, on/off ou 1/0"
        ))),
    }
}

fn read_parsed_env<T>(name: &str, default: T) -> Result<T, ConfigValidationError>
where
    T: FromStr,
{
    let Ok(value) = std::env::var(name) else {
        return Ok(default);
    };
    value
        .trim()
        .parse::<T>()
        .map_err(|_| ConfigValidationError(format!("{name} a une valeur invalide: {value}")))
}

fn read_positive_u32_env(name: &str, default: u32) -> Result<u32, ConfigValidationError> {
    let value = read_parsed_env(name, default)?;
    if value == 0 {
        return Err(ConfigValidationError(format!(
            "{name} doit être strictement positif"
        )));
    }
    Ok(value)
}

fn read_unit_f32_env(name: &str, default: f32) -> Result<f32, ConfigValidationError> {
    let value = read_parsed_env(name, default)?;
    if !(0.0..=1.0).contains(&value) {
        return Err(ConfigValidationError(format!(
            "{name} doit être compris entre 0 et 1"
        )));
    }
    Ok(value)
}

fn required_secret_env(name: &str, min_len: usize) -> Result<String, ConfigValidationError> {
    let value = std::env::var(name).unwrap_or_default();
    let trimmed = value.trim();
    if trimmed.len() < min_len {
        return Err(ConfigValidationError(format!(
            "{name} doit être défini et contenir au moins {min_len} caractères"
        )));
    }
    Ok(trimmed.to_string())
}

fn parse_cors_allowed_origins(raw: &str) -> Vec<String> {
    raw.split(',')
        .filter_map(|s| {
            let normalized = s.trim().trim_end_matches('/');
            if normalized.is_empty() {
                None
            } else {
                Some(normalized.to_string())
            }
        })
        .collect()
}

fn is_production_environment(environment: &str) -> bool {
    matches!(
        environment.trim().to_ascii_lowercase().as_str(),
        "prod" | "production"
    )
}

fn resolve_cors_allowed_origins(
    raw: Option<&str>,
    app_base_url: &str,
    production: bool,
) -> Result<Vec<String>, ConfigValidationError> {
    let explicit = raw.map(parse_cors_allowed_origins).unwrap_or_default();
    if !explicit.is_empty() {
        return Ok(explicit);
    }
    if production {
        return Err(ConfigValidationError(
            "CORS_ALLOWED_ORIGINS doit être défini explicitement en production".into(),
        ));
    }
    Ok(default_cors_allowed_origins(app_base_url))
}

pub struct AppConfig {
    pub app_environment: String,
    pub host: String,
    pub port: u16,
    pub database_url: String,
    pub database_max_connections: u32,
    pub jwt_secret: String,
    pub data_encryption_key: String,
    pub data_lookup_key: String,
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
    pub invitation_rate_limit_max_attempts: usize,
    pub invitation_rate_limit_window_seconds: u64,
    pub enable_swagger_ui: bool,
    pub metrics_bearer_token: Option<String>,
    pub user_metrics_refresh_seconds: u64,
    pub sentry_dsn: Option<String>,
    pub sentry_environment: String,
    pub sentry_traces_sample_rate: f32,
}

impl AppConfig {
    pub fn from_env() -> Self {
        Self::try_from_env().unwrap_or_else(|err| panic!("{err}"))
    }

    pub fn try_from_env() -> Result<Self, ConfigValidationError> {
        load_dotenv_from_repo();
        let app_environment = std::env::var("APP_ENV")
            .or_else(|_| std::env::var("SENTRY_ENVIRONMENT"))
            .unwrap_or_else(|_| "development".into());
        let is_production = is_production_environment(&app_environment);
        let host = std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".into());
        let port = read_parsed_env("PORT", 8080)?;
        let database_url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/fiestaaa".into());
        let database_max_connections = read_positive_u32_env("DATABASE_MAX_CONNECTIONS", 5)?;
        let jwt_secret = required_secret_env("JWT_SECRET", 32)?;
        let data_encryption_key = required_secret_env("DATA_ENCRYPTION_KEY", 32)?;
        let data_lookup_key = required_secret_env("DATA_LOOKUP_KEY", 32)?;
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
        let trust_proxy_headers = read_bool_env("TRUST_PROXY_HEADERS", false)?;
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
        let notification_dedup_ttl_seconds =
            read_parsed_env("NOTIFICATION_DEDUP_TTL_SECONDS", 300)?;
        let fcm_service_account_path = std::env::var("FCM_SERVICE_ACCOUNT_PATH")
            .ok()
            .filter(|v| !v.trim().is_empty());
        let fcm_project_id = std::env::var("FCM_PROJECT_ID")
            .ok()
            .filter(|v| !v.trim().is_empty());
        let event_cleanup_days = read_parsed_env("EVENT_CLEANUP_DAYS", 7)?;
        let event_cleanup_interval_hours = read_parsed_env("EVENT_CLEANUP_INTERVAL_HOURS", 1)?;
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
        let cors_allowed_origins_raw = std::env::var("CORS_ALLOWED_ORIGINS").ok();
        let cors_allowed_origins = resolve_cors_allowed_origins(
            cors_allowed_origins_raw.as_deref(),
            &app_base_url,
            is_production,
        )?;
        let auth_rate_limit_max_attempts = read_parsed_env("AUTH_RATE_LIMIT_MAX_ATTEMPTS", 20)?;
        let auth_rate_limit_window_seconds = read_parsed_env("AUTH_RATE_LIMIT_WINDOW_SECONDS", 60)?;
        let invitation_rate_limit_max_attempts =
            read_parsed_env("INVITATION_RATE_LIMIT_MAX_ATTEMPTS", 10)?;
        let invitation_rate_limit_window_seconds =
            read_parsed_env("INVITATION_RATE_LIMIT_WINDOW_SECONDS", 300)?;
        let enable_swagger_ui = read_bool_env("ENABLE_SWAGGER_UI", false)?;
        let metrics_bearer_token = std::env::var("METRICS_BEARER_TOKEN")
            .ok()
            .filter(|v| !v.trim().is_empty());
        let user_metrics_refresh_seconds = read_parsed_env("USER_METRICS_REFRESH_SECONDS", 300)?;
        let sentry_dsn = std::env::var("SENTRY_DSN")
            .ok()
            .filter(|v| !v.trim().is_empty());
        let sentry_environment =
            std::env::var("SENTRY_ENVIRONMENT").unwrap_or_else(|_| app_environment.clone());
        let sentry_traces_sample_rate = read_unit_f32_env("SENTRY_TRACES_SAMPLE_RATE", 0.0)?;
        Ok(Self {
            app_environment,
            host,
            port,
            database_url,
            database_max_connections,
            jwt_secret,
            data_encryption_key,
            data_lookup_key,
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
            invitation_rate_limit_max_attempts,
            invitation_rate_limit_window_seconds,
            enable_swagger_ui,
            metrics_bearer_token,
            user_metrics_refresh_seconds,
            sentry_dsn,
            sentry_environment,
            sentry_traces_sample_rate,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{default_cors_allowed_origins, resolve_cors_allowed_origins};

    #[test]
    fn default_cors_allowed_origins_trims_and_deduplicates_app_base_url() {
        let origins = default_cors_allowed_origins("  http://localhost:5001/  ");

        assert_eq!(
            origins.first().map(String::as_str),
            Some("http://localhost:5001")
        );
        assert_eq!(
            origins
                .iter()
                .filter(|origin| origin.as_str() == "http://localhost:5001")
                .count(),
            1
        );
        assert!(origins.contains(&"http://127.0.0.1:5001".to_string()));
        assert!(origins.contains(&"http://localhost:3000".to_string()));
    }

    #[test]
    fn default_cors_allowed_origins_skips_blank_app_base_url() {
        let origins = default_cors_allowed_origins("   ");

        assert_eq!(
            origins.first().map(String::as_str),
            Some("http://localhost:3000")
        );
        assert!(!origins.iter().any(|origin| origin.is_empty()));
    }

    #[test]
    fn cors_origins_fail_closed_in_production_without_explicit_value() {
        let err = resolve_cors_allowed_origins(None, "https://fiestaaa.app", true)
            .expect_err("production CORS must be explicit");

        assert!(err.to_string().contains("CORS_ALLOWED_ORIGINS"));
    }

    #[test]
    fn cors_origins_use_safe_dev_defaults_outside_production() {
        let origins = resolve_cors_allowed_origins(None, "http://localhost:5001", false)
            .expect("development defaults");

        assert!(origins.contains(&"http://localhost:5001".to_string()));
    }
}
