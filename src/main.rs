use actix_web::{web, App, HttpServer, middleware::Logger};
use actix_web::http::header::{AUTHORIZATION, CONTENT_TYPE};
use actix_cors::Cors;

mod config;
mod state;
mod db;
mod models;
mod auth;
mod routes;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // Logging
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("actix_web=info")
    ).init();

    // Config + DB
    let cfg = config::AppConfig::from_env();
    let pool = db::connect_and_migrate(&cfg.database_url).await;
    let state = web::Data::new(state::AppState { db: pool, jwt_secret: cfg.jwt_secret.clone() });

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
    })
    .bind(format!("{}:{}", cfg.host, cfg.port))?
    .run()
    .await
}

