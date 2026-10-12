use fiestaaa_back::config::AppConfig;
use std::process::Command;

// Configuration is evaluated in a fresh process: no environment mutation races
// with parallel tests, and no developer credentials are inherited.
fn probe(extra: &[(&str, &str)], expected: &str) {
    let mut child = Command::new(std::env::current_exe().unwrap());
    child
        .args(["--exact", "configuration_probe", "--nocapture"])
        .env_clear()
        .env("COVERAGE_CONFIG_EXPECT", expected)
        .env("JWT_SECRET", "unit-fixture-jwt-key-32-characters!!!!")
        .env(
            "DATA_ENCRYPTION_KEY",
            "unit-fixture-data-key-32-characters!!!",
        )
        .env("DATA_LOOKUP_KEY", "unit-fixture-lookup-key-32-characters!");
    if let Some(value) = std::env::var_os("LLVM_PROFILE_FILE") {
        child.env("LLVM_PROFILE_FILE", value);
    }
    for (key, value) in extra {
        child.env(key, value);
    }
    let output = child.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}
#[test]
fn configuration_probe() {
    let Ok(expected) = std::env::var("COVERAGE_CONFIG_EXPECT") else {
        return;
    };
    assert!(
        !std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(".env")
            .exists(),
        "configuration tests require a clean checkout without .env"
    );
    match expected.as_str() {
        "defaults" => {
            let config = AppConfig::from_env();
            assert_eq!(config.app_environment, "development");
            assert_eq!(config.port, 8080);
            assert_eq!(config.database_max_connections, 5);
            assert!(!config.trust_proxy_headers);
            assert_eq!(config.event_cleanup_days, 7);
            assert_eq!(config.event_purge_days, 30);
            assert_eq!(config.sentry_traces_sample_rate, 0.0);
            assert!(config.invitation_email_api_key.is_none());
        }
        "custom" => {
            let config = AppConfig::try_from_env().unwrap();
            assert_eq!(config.port, 9090);
            assert_eq!(
                config.admin_emails,
                ["qa@example.invalid", "other@example.invalid"]
            );
            assert!(config.trust_proxy_headers);
            assert!(config.enforce_pagination_defaults);
            assert_eq!(
                config.cors_allowed_origins,
                ["https://fixture.invalid", "http://localhost:9000"]
            );
            assert_eq!(
                config.invitation_email_api_key.as_deref(),
                Some("re_fictional")
            );
            assert_eq!(config.google_client_id.as_deref(), Some("fixture-google"));
            assert_eq!(config.apple_service_id.as_deref(), Some("fixture-apple"));
            assert_eq!(config.sentry_traces_sample_rate, 0.5);
        }
        "production" => {
            let config = AppConfig::try_from_env().unwrap();
            assert!(config.enforce_pagination_defaults);
            assert_eq!(config.cors_allowed_origins, ["https://fixture.invalid"]);
        }
        "no-email" => assert!(
            AppConfig::try_from_env()
                .unwrap()
                .invitation_email_api_key
                .is_none()
        ),
        error => match AppConfig::try_from_env() {
            Err(value) => assert!(
                value.to_string().contains(error),
                "expected {error}: {value}"
            ),
            Ok(_) => panic!("invalid configuration accepted: {error}"),
        },
    }
}
#[test]
fn isolated_environment_defaults_overrides_and_invalid_runtime_values() {
    probe(&[], "defaults");
    probe(
        &[
            ("PORT", "9090"),
            (
                "ADMIN_EMAILS",
                " QA@example.invalid, ,Other@example.invalid ",
            ),
            ("TRUST_PROXY_HEADERS", "on"),
            ("ENFORCE_PAGINATION_DEFAULTS", "yes"),
            (
                "CORS_ALLOWED_ORIGINS",
                " https://fixture.invalid/, http://localhost:9000/ ",
            ),
            ("RESEND_API_KEY", " re_fictional "),
            ("FIESTAAA_GOOGLE_WEB_CLIENT_ID", "fixture-google"),
            ("FIESTAAA_APPLE_SERVICE_ID", "fixture-apple"),
            ("SENTRY_TRACES_SAMPLE_RATE", "0.5"),
        ],
        "custom",
    );
    for value in ["", "invalid-prefix"] {
        probe(&[("RESEND_API_KEY", value)], "no-email");
    }
    for key in [
        "DATABASE_MAX_CONNECTIONS",
        "NOTIFICATION_DEDUP_TTL_SECONDS",
        "EVENT_CLEANUP_DAYS",
        "EVENT_PURGE_DAYS",
        "EVENT_CLEANUP_INTERVAL_HOURS",
        "AUTH_RATE_LIMIT_MAX_ATTEMPTS",
        "AUTH_RATE_LIMIT_WINDOW_SECONDS",
        "INVITATION_RATE_LIMIT_MAX_ATTEMPTS",
        "INVITATION_RATE_LIMIT_WINDOW_SECONDS",
        "USER_METRICS_REFRESH_SECONDS",
    ] {
        probe(&[(key, "0")], key);
        probe(&[(key, "invalid")], key);
    }
    for key in [
        "TRUST_PROXY_HEADERS",
        "ENFORCE_PAGINATION_DEFAULTS",
        "ENABLE_SWAGGER_UI",
    ] {
        probe(&[(key, "maybe")], key);
    }
    for value in ["-0.1", "1.1", "NaN", "invalid"] {
        probe(
            &[("SENTRY_TRACES_SAMPLE_RATE", value)],
            "SENTRY_TRACES_SAMPLE_RATE",
        );
    }
    probe(&[("PORT", "invalid")], "PORT");
    for key in ["JWT_SECRET", "DATA_ENCRYPTION_KEY", "DATA_LOOKUP_KEY"] {
        probe(&[(key, "short")], key);
    }
    let production = [
        ("APP_ENV", "production"),
        (
            "DATABASE_URL",
            "postgres://fixture:fixture@127.0.0.1:5432/fixture",
        ),
        ("INVITATION_EMAIL_SENDER", "qa@example.invalid"),
        ("RESEND_API_KEY", "re_fictional"),
        ("CORS_ALLOWED_ORIGINS", "https://fixture.invalid"),
    ];
    probe(&production, "production");
    for (key, value, error) in [
        (
            "DATABASE_URL",
            "postgres://postgres:postgres@localhost/fixture",
            "DATABASE_URL",
        ),
        ("RESEND_API_KEY", "", "RESEND_API_KEY"),
        ("APP_BASE_URL", "http://fixture.invalid", "APP_BASE_URL"),
        ("CORS_ALLOWED_ORIGINS", "", "CORS_ALLOWED_ORIGINS"),
        (
            "JWT_SECRET",
            "example-secret-example-secret-example-secret",
            "JWT_SECRET",
        ),
        ("METRICS_BEARER_TOKEN", "change_me", "METRICS_BEARER_TOKEN"),
    ] {
        let mut vars = production.to_vec();
        vars.push((key, value));
        probe(&vars, error);
    }
}
