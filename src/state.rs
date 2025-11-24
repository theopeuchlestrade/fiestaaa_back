use std::collections::HashSet;

use sqlx::{Pool, Postgres};

pub struct AppState {
    pub db: Pool<Postgres>,
    pub jwt_secret: String,
    pub admin_emails: HashSet<String>,
    pub http_client: reqwest::Client,
    pub geocoding_base_url: String,
    pub geocoding_country_codes: Option<String>,
}
