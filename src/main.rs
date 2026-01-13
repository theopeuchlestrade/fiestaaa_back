use actix_cors::Cors;
use actix_files::Files;
use actix_web::http::header::{AUTHORIZATION, CONTENT_TYPE};
use actix_web::{App, HttpRequest, HttpServer, middleware::Logger, web};
use fiestaaa_back::{
    cleanup,
    config,
    db,
    docs,
    metrics::{AppMetrics, MetricsMiddleware},
    notifications,
    routes,
    state,
};
use prometheus::{Encoder, TextEncoder};
use redis::Client as RedisClient;
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

    cleanup::CleanupService::new(pool.clone())
        .with_cleanup_days(cfg.event_cleanup_days)
        .with_interval_hours(cfg.event_cleanup_interval_hours)
        .start();

    let admin_emails = cfg.admin_emails.iter().cloned().collect::<HashSet<_>>();
    let http_client = reqwest::Client::builder()
        .user_agent(cfg.geocoding_user_agent.clone())
        .build()
        .expect("http client");
    let redis_client = cfg
        .redis_url
        .as_ref()
        .and_then(|url| RedisClient::open(url.as_str()).ok());
    let notifications = notifications::NotificationService::new(
        cfg.fcm_server_key.clone(),
        cfg.fcm_service_account_path.clone(),
        cfg.fcm_project_id.clone(),
        redis_client.clone(),
        http_client.clone(),
        cfg.notification_dedup_ttl_seconds,
    );
    let metrics = web::Data::new(AppMetrics::new());
    
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
        avatar_upload_dir: cfg.avatar_upload_dir.clone(),
        avatar_base_url: cfg.avatar_base_url.clone(),
        redis_client,
        notifications,
        fcm_project_id: cfg.fcm_project_id.clone(),
        google_client_id: cfg.google_client_id.clone(),
        google_android_client_id: cfg.google_android_client_id.clone(),
        google_ios_client_id: cfg.google_ios_client_id.clone(),
        apple_app_id: cfg.apple_app_id.clone(),
        apple_service_id: cfg.apple_service_id.clone(),
    });

    // Server
    HttpServer::new(move || {
        let mut cors = Cors::default()
            .allowed_methods(vec!["GET", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"])
            .allowed_headers(vec![AUTHORIZATION, CONTENT_TYPE])
            .max_age(3600);
        if cfg.cors_allowed_origins.is_empty() {
            log::warn!("CORS_ALLOWED_ORIGINS non défini, toutes les origines sont autorisées");
            cors = cors.allow_any_origin();
        } else {
            for origin in &cfg.cors_allowed_origins {
                cors = cors.allowed_origin(origin);
            }
        }

        App::new()
            .app_data(state.clone())
            .app_data(metrics.clone())
            .wrap(Logger::default())
            .wrap(cors)
            .wrap(MetricsMiddleware)
            .configure(routes::configure)
            .service(Files::new("/media/avatars", &cfg.avatar_upload_dir).prefer_utf8(true))
            .service(
                SwaggerUi::new("/docs/{_:.*}").url("/docs/openapi.json", docs::ApiDoc::openapi()),
            )
            .service(
                web::resource("/metrics")
                    .to({
                        let metrics_token = cfg.metrics_token.clone();
                        move |req: HttpRequest, metrics: web::Data<AppMetrics>| {
                            let metrics_token = metrics_token.clone();
                            async move {
                                if let Some(token) = metrics_token.as_deref() {
                                    let header = req
                                        .headers()
                                        .get(AUTHORIZATION)
                                        .and_then(|value| value.to_str().ok());
                                    let authorized = header
                                        .and_then(|value| value.strip_prefix("Bearer "))
                                        .is_some_and(|value| value == token)
                                        || req
                                            .headers()
                                            .get("X-Metrics-Token")
                                            .and_then(|value| value.to_str().ok())
                                            .is_some_and(|value| value == token);
                                    if !authorized {
                                        return actix_web::HttpResponse::Unauthorized().finish();
                                    }
                                }
                                let encoder = TextEncoder::new();
                                let mut buffer = vec![];
                                encoder.encode(&metrics.registry.gather(), &mut buffer).unwrap();
                                actix_web::HttpResponse::Ok()
                                    .content_type(encoder.format_type())
                                    .body(String::from_utf8(buffer).unwrap())
                            }
                        }
                    }),
            )
    })
    .bind(format!("{}:{}", cfg.host, cfg.port))?
    .run()
    .await
}
