use actix_cors::Cors;
use actix_files::Files;
use actix_web::http::header::{AUTHORIZATION, CONTENT_TYPE};
use actix_web::{
    App, HttpServer,
    middleware::{DefaultHeaders, Logger},
    web,
};
use fiestaaa_back::{
    cleanup, config, db, docs, notifications, observability, rate_limit, routes, state,
    user_metrics,
};
use redis::Client as RedisClient;
use std::collections::HashSet;
use std::time::Duration;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    fiestaaa_back::install_rustls_crypto_provider();

    // Logging
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("actix_web=info"))
        .init();

    // Config + DB
    let cfg = config::AppConfig::from_env();
    let _sentry_guard = cfg.sentry_dsn.as_ref().map(|dsn| {
        sentry::init((
            dsn.as_str(),
            sentry::ClientOptions {
                release: sentry::release_name!(),
                environment: Some(cfg.sentry_environment.clone().into()),
                traces_sample_rate: cfg.sentry_traces_sample_rate,
                ..Default::default()
            },
        ))
    });
    let pool = db::connect_and_migrate(
        &cfg.database_url,
        cfg.database_max_connections,
        &cfg.data_encryption_key,
        &cfg.data_lookup_key,
    )
    .await;

    cleanup::CleanupService::new(pool.clone())
        .with_cleanup_days(cfg.event_cleanup_days)
        .with_interval_hours(cfg.event_cleanup_interval_hours)
        .start();

    user_metrics::UserMetricsService::new(pool.clone())
        .with_refresh_seconds(cfg.user_metrics_refresh_seconds)
        .start();

    let admin_emails = cfg.admin_emails.iter().cloned().collect::<HashSet<_>>();
    let http_client = fiestaaa_back::build_http_client(&cfg.geocoding_user_agent);
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
    let state = web::Data::new(state::AppState {
        db: pool,
        jwt_secret: cfg.jwt_secret.clone(),
        admin_emails,
        trust_proxy_headers: cfg.trust_proxy_headers,
        http_client,
        geocoding_base_url: cfg.geocoding_base_url.clone(),
        geocoding_country_codes: cfg.geocoding_country_codes.clone(),
        invitation_email_sender: cfg.invitation_email_sender.clone(),
        invitation_email_api_key: cfg.invitation_email_api_key.clone(),
        app_base_url: cfg.app_base_url.clone(),
        cors_allowed_origins: cfg.cors_allowed_origins.iter().cloned().collect(),
        avatar_upload_dir: cfg.avatar_upload_dir.clone(),
        avatar_base_url: cfg.avatar_base_url.clone(),
        redis_client: redis_client.clone(),
        notifications,
        fcm_project_id: cfg.fcm_project_id.clone(),
        google_client_id: cfg.google_client_id.clone(),
        google_android_client_id: cfg.google_android_client_id.clone(),
        google_ios_client_id: cfg.google_ios_client_id.clone(),
        apple_app_id: cfg.apple_app_id.clone(),
        apple_service_id: cfg.apple_service_id.clone(),
        google_tokeninfo_url: "https://oauth2.googleapis.com/tokeninfo".into(),
        google_userinfo_url: "https://www.googleapis.com/oauth2/v3/userinfo".into(),
        apple_jwks_url: "https://appleid.apple.com/auth/keys".into(),
        auth_rate_limiter: rate_limit::AuthRateLimiter::new(
            cfg.auth_rate_limit_max_attempts,
            Duration::from_secs(cfg.auth_rate_limit_window_seconds),
            redis_client.clone(),
        ),
        invitation_rate_limiter: rate_limit::AuthRateLimiter::new(
            cfg.invitation_rate_limit_max_attempts,
            Duration::from_secs(cfg.invitation_rate_limit_window_seconds),
            redis_client.clone(),
        ),
        metrics_bearer_token: cfg.metrics_bearer_token.clone(),
    });

    // Server
    let enable_swagger_ui = cfg.enable_swagger_ui;
    let enable_hsts = cfg.app_base_url.starts_with("https://");
    HttpServer::new(move || {
        let mut cors = Cors::default()
            .allowed_methods(vec!["GET", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"])
            .allowed_headers(vec![AUTHORIZATION, CONTENT_TYPE])
            .supports_credentials()
            .max_age(3600);
        for origin in &cfg.cors_allowed_origins {
            cors = cors.allowed_origin(origin);
        }

        let mut default_headers = DefaultHeaders::new()
            .add(("X-Content-Type-Options", "nosniff"))
            .add(("X-Frame-Options", "DENY"))
            .add(("Referrer-Policy", "no-referrer"))
            .add((
                "Permissions-Policy",
                "camera=(), microphone=(), geolocation=(), payment=(), usb=()",
            ));
        if enable_hsts {
            default_headers = default_headers.add((
                "Strict-Transport-Security",
                "max-age=31536000; includeSubDomains",
            ));
        }

        App::new()
            .app_data(state.clone())
            .wrap(observability::MetricsMiddleware)
            .wrap(Logger::new(r#"%a "%m %U" %s %b %T"#))
            .wrap(default_headers)
            .wrap(cors)
            .configure(routes::configure)
            .service(Files::new("/media/avatars", &cfg.avatar_upload_dir).prefer_utf8(true))
            .configure(|cfg| {
                if enable_swagger_ui {
                    cfg.service(
                        SwaggerUi::new("/docs/{_:.*}")
                            .url("/docs/openapi.json", docs::ApiDoc::openapi()),
                    );
                }
            })
    })
    .bind(format!("{}:{}", cfg.host, cfg.port))?
    .run()
    .await
}
