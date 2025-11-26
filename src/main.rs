use actix_cors::Cors;
use actix_web::http::header::{AUTHORIZATION, CONTENT_TYPE};
use actix_web::{App, HttpServer, middleware::Logger, web};
use fiestaaa_back::{config, db, docs, routes, state};
use std::collections::HashSet;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // Logging
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("actix_web=info"))
        .init();

    // Config + DB
    let cfg = config::AppConfig::from_env();
    let pool = db::connect_and_migrate(&cfg.database_url).await;
    let admin_emails = cfg.admin_emails.iter().cloned().collect::<HashSet<_>>();
    let http_client = reqwest::Client::builder()
        .user_agent(cfg.geocoding_user_agent.clone())
        .build()
        .expect("http client");
    let state = web::Data::new(state::AppState {
        db: pool,
        jwt_secret: cfg.jwt_secret.clone(),
        admin_emails,
        http_client,
        geocoding_base_url: cfg.geocoding_base_url.clone(),
        geocoding_country_codes: cfg.geocoding_country_codes.clone(),
        invitation_email_sender: cfg.invitation_email_sender.clone(),
        invitation_email_api_key: cfg.invitation_email_api_key.clone(),
        app_base_url: cfg.app_base_url.clone(),
    });

    // Server
    HttpServer::new(move || {
        let cors = Cors::default()
            .allow_any_origin()
            .allowed_methods(vec!["GET", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"])
            .allowed_headers(vec![AUTHORIZATION, CONTENT_TYPE])
            .max_age(3600);

        App::new()
            .app_data(state.clone())
            .wrap(Logger::default())
            .wrap(cors)
            .configure(routes::configure)
            .service(
                SwaggerUi::new("/docs/{_:.*}").url("/docs/openapi.json", docs::ApiDoc::openapi()),
            )
    })
    .bind(format!("{}:{}", cfg.host, cfg.port))?
    .run()
    .await
}
