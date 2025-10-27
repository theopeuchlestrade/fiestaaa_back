use dotenvy::dotenv;

pub struct AppConfig {
    pub host: String,
    pub port: u16,
    pub database_url: String,
    pub jwt_secret: String,
    pub admin_emails: Vec<String>,
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
        Self {
            host,
            port,
            database_url,
            jwt_secret,
            admin_emails,
        }
    }
}
