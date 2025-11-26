use dotenvy::dotenv;

pub struct AppConfig {
    pub host: String,
    pub port: u16,
    pub database_url: String,
    pub jwt_secret: String,
    pub admin_emails: Vec<String>,
    pub geocoding_base_url: String,
    pub geocoding_user_agent: String,
    pub geocoding_country_codes: Option<String>,
    pub invitation_email_sender: Option<String>,
    pub invitation_email_api_key: Option<String>,
    pub app_base_url: String,
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
        let jwt_secret =
            std::env::var("JWT_SECRET").unwrap_or_else(|_| "dev_secret_change_me".into());
        let admin_emails_raw = std::env::var("ADMIN_EMAILS").unwrap_or_default();
        let admin_emails = admin_emails_raw
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_lowercase())
            .collect::<Vec<_>>();
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
        let invitation_email_api_key = std::env::var("INVITATION_EMAIL_API_KEY")
            .or_else(|_| std::env::var("RESEND_API_KEY"))
            .ok()
            .filter(|v| !v.trim().is_empty());
        let app_base_url =
            std::env::var("APP_BASE_URL").unwrap_or_else(|_| "https://fiestaaa.app".into());
        Self {
            host,
            port,
            database_url,
            jwt_secret,
            admin_emails,
            geocoding_base_url,
            geocoding_user_agent,
            geocoding_country_codes,
            invitation_email_sender,
            invitation_email_api_key,
            app_base_url,
        }
    }
}
