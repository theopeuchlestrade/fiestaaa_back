use actix_web::body::MessageBody;
use actix_web::dev::{Service, ServiceRequest, ServiceResponse, Transform};
use actix_web::http::header::{HeaderName, HeaderValue};
use actix_web::{Error, HttpRequest};
use futures_util::future::{LocalBoxFuture, Ready, ready};
use once_cell::sync::Lazy;
use prometheus::{
    Encoder, HistogramOpts, HistogramVec, IntCounterVec, TextEncoder, register_histogram_vec,
    register_int_counter_vec,
};
use std::rc::Rc;
use std::task::{Context, Poll};
use std::time::Instant;
use uuid::Uuid;

const REQUEST_ID_HEADER: HeaderName = HeaderName::from_static("x-request-id");

const UNKNOWN_CLIENT_VERSION: &str = "unknown";
const INVALID_CLIENT_VERSION: &str = "invalid";
const OTHER_FLUTTER_CLIENT_VERSION: &str = "flutter/other";

fn client_version_bucket(raw: Option<&str>) -> &'static str {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return UNKNOWN_CLIENT_VERSION;
    };
    let Some(version) = raw.strip_prefix("flutter/") else {
        return INVALID_CLIENT_VERSION;
    };
    let mut parts = version.split('.');
    let (Some(major), Some(minor), Some(patch), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return INVALID_CLIENT_VERSION;
    };
    if [major, minor, patch]
        .iter()
        .any(|part| part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return INVALID_CLIENT_VERSION;
    }

    match (major, minor) {
        ("0", "1") => "flutter/0.1.x",
        ("0", "2") => "flutter/0.2.x",
        _ => OTHER_FLUTTER_CLIENT_VERSION,
    }
}

static HTTP_REQUESTS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "fiestaaa_http_requests_total",
        "Total HTTP requests handled by the API.",
        &["method", "route", "status"]
    )
    .expect("register fiestaaa_http_requests_total")
});

static HTTP_REQUEST_DURATION_SECONDS: Lazy<HistogramVec> = Lazy::new(|| {
    register_histogram_vec!(
        HistogramOpts::new(
            "fiestaaa_http_request_duration_seconds",
            "HTTP request duration in seconds."
        )
        .buckets(vec![
            0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
        ]),
        &["method", "route", "status"]
    )
    .expect("register fiestaaa_http_request_duration_seconds")
});

static AUTH_ERRORS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "fiestaaa_auth_errors_total",
        "Authentication and account flow errors.",
        &["kind"]
    )
    .expect("register fiestaaa_auth_errors_total")
});

static INVITATION_ERRORS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "fiestaaa_invitation_errors_total",
        "Invitation flow errors.",
        &["kind"]
    )
    .expect("register fiestaaa_invitation_errors_total")
});

static EMAIL_ERRORS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "fiestaaa_email_errors_total",
        "Outbound email errors.",
        &["kind"]
    )
    .expect("register fiestaaa_email_errors_total")
});

static PUSH_ERRORS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "fiestaaa_push_errors_total",
        "Outbound push notification errors.",
        &["kind"]
    )
    .expect("register fiestaaa_push_errors_total")
});

static API_LIST_CLIENTS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "fiestaaa_api_list_clients_total",
        "GET requests split by pagination adoption and client version.",
        &["mode", "client_version"]
    )
    .expect("register fiestaaa_api_list_clients_total")
});

pub struct MetricsMiddleware;

impl<S, B> Transform<S, ServiceRequest> for MetricsMiddleware
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: MessageBody + 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type InitError = ();
    type Transform = MetricsMiddlewareService<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(MetricsMiddlewareService {
            service: Rc::new(service),
        }))
    }
}

pub struct MetricsMiddlewareService<S> {
    service: Rc<S>,
}

impl<S, B> Service<ServiceRequest> for MetricsMiddlewareService<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: MessageBody + 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&self, ctx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(ctx)
    }

    fn call(&self, mut req: ServiceRequest) -> Self::Future {
        let service = Rc::clone(&self.service);
        let start = Instant::now();
        let request_id = Uuid::new_v4().to_string();
        req.headers_mut().insert(
            REQUEST_ID_HEADER,
            HeaderValue::from_str(&request_id).expect("UUID is a valid header value"),
        );
        let pagination_mode = if req
            .query_string()
            .split('&')
            .any(|part| part.starts_with("limit=") || part.starts_with("cursor="))
        {
            "paginated"
        } else {
            "legacy"
        };
        let client_version = client_version_bucket(
            req.headers()
                .get("x-fiestaaa-client-version")
                .and_then(|value| value.to_str().ok()),
        );

        Box::pin(async move {
            let mut res = service.call(req).await?;
            let method = res.request().method().as_str().to_owned();
            let route = res
                .request()
                .match_pattern()
                .unwrap_or_else(|| "unmatched".to_string());
            let status = res.status().as_u16().to_string();
            let elapsed = start.elapsed().as_secs_f64();

            HTTP_REQUESTS_TOTAL
                .with_label_values(&[&method, &route, &status])
                .inc();
            HTTP_REQUEST_DURATION_SECONDS
                .with_label_values(&[&method, &route, &status])
                .observe(elapsed);
            if method == "GET" {
                API_LIST_CLIENTS_TOTAL
                    .with_label_values(&[pagination_mode, client_version])
                    .inc();
            }

            if res.status().is_server_error() {
                capture_message(
                    sentry::Level::Error,
                    &format!(
                        "HTTP {} {} returned {} (request_id={})",
                        method, route, status, request_id
                    ),
                );
            }

            res.headers_mut().insert(
                REQUEST_ID_HEADER,
                HeaderValue::from_str(&request_id).expect("UUID is a valid header value"),
            );

            Ok(res)
        })
    }
}

pub fn metrics_authorized(req: &HttpRequest, bearer_token: Option<&str>) -> bool {
    let Some(expected) = bearer_token.filter(|value| !value.trim().is_empty()) else {
        return false;
    };
    let Some(header) = req.headers().get("authorization") else {
        return false;
    };
    let Ok(value) = header.to_str() else {
        return false;
    };
    value.trim() == format!("Bearer {expected}")
}

pub fn render_prometheus() -> Result<String, prometheus::Error> {
    Lazy::force(&HTTP_REQUESTS_TOTAL);
    Lazy::force(&HTTP_REQUEST_DURATION_SECONDS);
    Lazy::force(&AUTH_ERRORS_TOTAL);
    Lazy::force(&INVITATION_ERRORS_TOTAL);
    Lazy::force(&EMAIL_ERRORS_TOTAL);
    Lazy::force(&PUSH_ERRORS_TOTAL);
    Lazy::force(&API_LIST_CLIENTS_TOTAL);
    crate::user_metrics::force_registered();

    let metric_families = prometheus::gather();
    let encoder = TextEncoder::new();
    let mut buffer = Vec::new();
    encoder.encode(&metric_families, &mut buffer)?;
    Ok(String::from_utf8_lossy(&buffer).into_owned())
}

pub fn record_auth_error(kind: &str) {
    AUTH_ERRORS_TOTAL.with_label_values(&[kind]).inc();
}

pub fn record_invitation_error(kind: &str) {
    INVITATION_ERRORS_TOTAL.with_label_values(&[kind]).inc();
}

pub fn record_email_error(kind: &str) {
    EMAIL_ERRORS_TOTAL.with_label_values(&[kind]).inc();
}

pub fn record_push_error(kind: &str) {
    PUSH_ERRORS_TOTAL.with_label_values(&[kind]).inc();
}

pub fn capture_message(level: sentry::Level, message: &str) {
    sentry::capture_message(message, level);
}

pub fn capture_internal_error(context: &str, error: &dyn std::fmt::Display) {
    let message = format!("{context}: {error}");
    log::error!("{message}");
    capture_message(sentry::Level::Error, &message);
}

#[track_caller]
pub fn capture_internal_failure(context: &str) {
    let caller = std::panic::Location::caller();
    let message = format!(
        "{context} at {}:{}:{}",
        caller.file(),
        caller.line(),
        caller.column()
    );
    log::error!("{message}");
    capture_message(sentry::Level::Error, &message);
}

#[cfg(test)]
mod tests {
    use super::{client_version_bucket, metrics_authorized, record_auth_error, render_prometheus};
    use actix_web::{App, HttpResponse, test as actix_test, web};
    use uuid::Uuid;

    #[test]
    fn metrics_require_configured_bearer_token() {
        let req = actix_test::TestRequest::default()
            .insert_header(("Authorization", "Bearer secret"))
            .to_http_request();

        assert!(metrics_authorized(&req, Some("secret")));
        assert!(!metrics_authorized(&req, Some("other")));
        assert!(!metrics_authorized(&req, None));
    }

    #[actix_web::test]
    async fn middleware_adds_a_request_id_to_responses() {
        let app = actix_test::init_service(App::new().wrap(super::MetricsMiddleware).route(
            "/test",
            web::get().to(|| async { HttpResponse::NoContent().finish() }),
        ))
        .await;
        let response = actix_test::call_service(
            &app,
            actix_test::TestRequest::get().uri("/test").to_request(),
        )
        .await;

        let request_id = response
            .headers()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .expect("request ID response header");
        assert!(Uuid::parse_str(request_id).is_ok());
    }

    #[test]
    fn client_versions_are_reduced_to_bounded_buckets() {
        assert_eq!(
            client_version_bucket(Some("flutter/0.2.0")),
            "flutter/0.2.x"
        );
        assert_eq!(
            client_version_bucket(Some("flutter/0.2.99")),
            "flutter/0.2.x"
        );
        assert_eq!(
            client_version_bucket(Some("flutter/12.4.1")),
            "flutter/other"
        );
        assert_eq!(client_version_bucket(Some("custom/1.0.0")), "invalid");
        assert_eq!(client_version_bucket(Some("flutter/not-semver")), "invalid");
        assert_eq!(client_version_bucket(None), "unknown");
    }

    #[test]
    fn prometheus_render_contains_fiestaaa_metrics() {
        record_auth_error("test");
        let rendered = render_prometheus().expect("render metrics");
        assert!(rendered.contains("fiestaaa_auth_errors_total"));
    }
}
