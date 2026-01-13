use prometheus::{IntCounter, IntGauge, HistogramVec, Registry};
use std::time::Instant;

pub struct AppMetrics {
    pub registry: Registry,
    pub http_requests_total: IntCounter,
    pub http_request_duration_seconds: HistogramVec,
    pub active_connections: IntGauge,
    pub database_operations_total: IntCounter,
    pub database_operation_duration_seconds: HistogramVec,
    pub authentication_attempts_total: IntCounter,
    pub authentication_success_total: IntCounter,
    pub authentication_failure_total: IntCounter,
    pub event_creations_total: IntCounter,
    pub invitation_creations_total: IntCounter,
    pub item_creations_total: IntCounter,
    pub active_users: IntGauge,
}

impl AppMetrics {
    pub fn new() -> Self {
        let registry = Registry::new();
        
        let http_requests_total = IntCounter::new(
            "http_requests_total",
            "Total number of HTTP requests",
        )
        .expect("metric can be created");
        
        let http_request_duration_seconds = HistogramVec::new(
            prometheus::HistogramOpts::new(
                "http_request_duration_seconds",
                "HTTP request duration in seconds",
            )
            .buckets(vec![0.01, 0.05, 0.1, 0.5, 1.0, 2.5, 5.0, 10.0]),
            &["method", "path"],
        )
        .expect("metric can be created");
        
        let active_connections = IntGauge::new(
            "active_connections",
            "Number of active connections",
        )
        .expect("metric can be created");
        
        let database_operations_total = IntCounter::new(
            "database_operations_total",
            "Total number of database operations",
        )
        .expect("metric can be created");
        
        let database_operation_duration_seconds = HistogramVec::new(
            prometheus::HistogramOpts::new(
                "database_operation_duration_seconds",
                "Database operation duration in seconds",
            )
            .buckets(vec![0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0]),
            &["operation"],
        )
        .expect("metric can be created");
        
        let authentication_attempts_total = IntCounter::new(
            "authentication_attempts_total",
            "Total number of authentication attempts",
        )
        .expect("metric can be created");
        
        let authentication_success_total = IntCounter::new(
            "authentication_success_total",
            "Total number of successful authentications",
        )
        .expect("metric can be created");
        
        let authentication_failure_total = IntCounter::new(
            "authentication_failure_total",
            "Total number of failed authentications",
        )
        .expect("metric can be created");
        
        let event_creations_total = IntCounter::new(
            "event_creations_total",
            "Total number of event creations",
        )
        .expect("metric can be created");
        
        let invitation_creations_total = IntCounter::new(
            "invitation_creations_total",
            "Total number of invitation creations",
        )
        .expect("metric can be created");
        
        let item_creations_total = IntCounter::new(
            "item_creations_total",
            "Total number of item creations",
        )
        .expect("metric can be created");
        
        let active_users = IntGauge::new(
            "active_users",
            "Number of active users",
        )
        .expect("metric can be created");
        
        registry.register(Box::new(http_requests_total.clone())).expect("can register metric");
        registry.register(Box::new(http_request_duration_seconds.clone())).expect("can register metric");
        registry.register(Box::new(active_connections.clone())).expect("can register metric");
        registry.register(Box::new(database_operations_total.clone())).expect("can register metric");
        registry.register(Box::new(database_operation_duration_seconds.clone())).expect("can register metric");
        registry.register(Box::new(authentication_attempts_total.clone())).expect("can register metric");
        registry.register(Box::new(authentication_success_total.clone())).expect("can register metric");
        registry.register(Box::new(authentication_failure_total.clone())).expect("can register metric");
        registry.register(Box::new(event_creations_total.clone())).expect("can register metric");
        registry.register(Box::new(invitation_creations_total.clone())).expect("can register metric");
        registry.register(Box::new(item_creations_total.clone())).expect("can register metric");
        registry.register(Box::new(active_users.clone())).expect("can register metric");
        
        AppMetrics {
            registry,
            http_requests_total,
            http_request_duration_seconds,
            active_connections,
            database_operations_total,
            database_operation_duration_seconds,
            authentication_attempts_total,
            authentication_success_total,
            authentication_failure_total,
            event_creations_total,
            invitation_creations_total,
            item_creations_total,
            active_users,
        }
    }
    
    pub async fn track_http_request<F, Fut, R>(&self, method: &str, path: &str, f: F) -> R
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = R>,
    {
        self.http_requests_total.inc();
        let start = Instant::now();
        let result = f().await;
        let duration = start.elapsed().as_secs_f64();
        self.http_request_duration_seconds
            .with_label_values(&[method, path])
            .observe(duration);
        result
    }
    
    pub async fn track_database_operation<F, Fut, R>(&self, operation: &str, f: F) -> R
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = R>,
    {
        self.database_operations_total.inc();
        let start = Instant::now();
        let result = f().await;
        let duration = start.elapsed().as_secs_f64();
        self.database_operation_duration_seconds
            .with_label_values(&[operation])
            .observe(duration);
        result
    }
}
