use log::{error, info};
use once_cell::sync::Lazy;
use prometheus::{
    IntCounter, IntGauge, IntGaugeVec, register_int_counter, register_int_gauge,
    register_int_gauge_vec,
};
use sqlx::{Pool, Postgres};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::time::{Duration as TokioDuration, interval};

const DEFAULT_REFRESH_SECONDS: u64 = 300;
const MIN_REFRESH_SECONDS: u64 = 30;
const WINDOWS: [&str; 3] = ["24h", "7d", "30d"];
const ACTIVE_SOURCES: [&str; 4] = ["oauth_login", "device_seen", "product_activity", "any"];
const PLATFORMS: [&str; 3] = ["ios", "android", "web"];
const OAUTH_PROVIDERS: [&str; 2] = ["google", "apple"];

static USERS_REGISTERED: Lazy<IntGauge> = Lazy::new(|| {
    register_int_gauge!(
        "fiestaaa_users_registered",
        "Total registered users in the database."
    )
    .expect("register fiestaaa_users_registered")
});

static PENDING_REGISTRATIONS: Lazy<IntGauge> = Lazy::new(|| {
    register_int_gauge!(
        "fiestaaa_pending_registrations",
        "Pending email registrations awaiting verification."
    )
    .expect("register fiestaaa_pending_registrations")
});

static USERS_CREATED_WINDOW: Lazy<IntGaugeVec> = Lazy::new(|| {
    register_int_gauge_vec!(
        "fiestaaa_users_created_window",
        "Users created within a rolling time window.",
        &["window"]
    )
    .expect("register fiestaaa_users_created_window")
});

static USERS_ACTIVE_WINDOW: Lazy<IntGaugeVec> = Lazy::new(|| {
    register_int_gauge_vec!(
        "fiestaaa_users_active_window",
        "Distinct users active within a rolling time window.",
        &["window", "source"]
    )
    .expect("register fiestaaa_users_active_window")
});

static USERS_WITH_ACTIVE_DEVICE: Lazy<IntGaugeVec> = Lazy::new(|| {
    register_int_gauge_vec!(
        "fiestaaa_users_with_active_device",
        "Distinct users with at least one active device.",
        &["platform"]
    )
    .expect("register fiestaaa_users_with_active_device")
});

static USER_DEVICES_ACTIVE: Lazy<IntGaugeVec> = Lazy::new(|| {
    register_int_gauge_vec!(
        "fiestaaa_user_devices_active",
        "Active user devices by platform.",
        &["platform"]
    )
    .expect("register fiestaaa_user_devices_active")
});

static USERS_OAUTH_LINKED: Lazy<IntGaugeVec> = Lazy::new(|| {
    register_int_gauge_vec!(
        "fiestaaa_users_oauth_linked",
        "Distinct users with a linked OAuth identity.",
        &["provider"]
    )
    .expect("register fiestaaa_users_oauth_linked")
});

static USER_METRICS_REFRESH_TIMESTAMP_SECONDS: Lazy<IntGauge> = Lazy::new(|| {
    register_int_gauge!(
        "fiestaaa_user_metrics_refresh_timestamp_seconds",
        "Unix timestamp of the last successful user metrics refresh."
    )
    .expect("register fiestaaa_user_metrics_refresh_timestamp_seconds")
});

static USER_METRICS_REFRESH_ERRORS_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "fiestaaa_user_metrics_refresh_errors_total",
        "Total user metrics refresh failures."
    )
    .expect("register fiestaaa_user_metrics_refresh_errors_total")
});

pub struct UserMetricsService {
    pool: Pool<Postgres>,
    refresh_seconds: u64,
}

impl UserMetricsService {
    pub fn new(pool: Pool<Postgres>) -> Self {
        Self {
            pool,
            refresh_seconds: DEFAULT_REFRESH_SECONDS,
        }
    }

    pub fn with_refresh_seconds(mut self, seconds: u64) -> Self {
        self.refresh_seconds = seconds;
        self
    }

    pub fn start(self) {
        force_registered();
        initialize_zero_labels();

        let pool = self.pool;
        let refresh_seconds = self.refresh_seconds.max(MIN_REFRESH_SECONDS);

        tokio::spawn(async move {
            info!(
                "User metrics service started - refresh every {} seconds",
                refresh_seconds
            );

            if let Err(e) = refresh_user_metrics(&pool).await {
                USER_METRICS_REFRESH_ERRORS_TOTAL.inc();
                error!("Initial user metrics refresh failed: {}", e);
            }

            let mut interval = interval(TokioDuration::from_secs(refresh_seconds));

            loop {
                interval.tick().await;

                if let Err(e) = refresh_user_metrics(&pool).await {
                    USER_METRICS_REFRESH_ERRORS_TOTAL.inc();
                    error!("User metrics refresh failed: {}", e);
                }
            }
        });
    }
}

pub fn force_registered() {
    Lazy::force(&USERS_REGISTERED);
    Lazy::force(&PENDING_REGISTRATIONS);
    Lazy::force(&USERS_CREATED_WINDOW);
    Lazy::force(&USERS_ACTIVE_WINDOW);
    Lazy::force(&USERS_WITH_ACTIVE_DEVICE);
    Lazy::force(&USER_DEVICES_ACTIVE);
    Lazy::force(&USERS_OAUTH_LINKED);
    Lazy::force(&USER_METRICS_REFRESH_TIMESTAMP_SECONDS);
    Lazy::force(&USER_METRICS_REFRESH_ERRORS_TOTAL);
}

fn initialize_zero_labels() {
    for window in WINDOWS {
        USERS_CREATED_WINDOW.with_label_values(&[window]).set(0);
        for source in ACTIVE_SOURCES {
            USERS_ACTIVE_WINDOW
                .with_label_values(&[window, source])
                .set(0);
        }
    }

    for platform in PLATFORMS {
        USERS_WITH_ACTIVE_DEVICE
            .with_label_values(&[platform])
            .set(0);
        USER_DEVICES_ACTIVE.with_label_values(&[platform]).set(0);
    }

    for provider in OAUTH_PROVIDERS {
        USERS_OAUTH_LINKED.with_label_values(&[provider]).set(0);
    }
}

pub async fn refresh_user_metrics(pool: &Pool<Postgres>) -> Result<(), sqlx::Error> {
    force_registered();

    let registered = sqlx::query_scalar::<_, i64>("SELECT COUNT(*)::BIGINT FROM users")
        .fetch_one(pool)
        .await?;
    USERS_REGISTERED.set(registered);

    let pending =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*)::BIGINT FROM pending_registrations")
            .fetch_one(pool)
            .await?;
    PENDING_REGISTRATIONS.set(pending);

    let created_windows = sqlx::query_as::<_, (String, i64)>(
        r#"
        WITH windows(label, since_at) AS (
            VALUES
                ('24h', NOW() - INTERVAL '24 hours'),
                ('7d', NOW() - INTERVAL '7 days'),
                ('30d', NOW() - INTERVAL '30 days')
        )
        SELECT w.label, COUNT(u.id)::BIGINT
        FROM windows w
        LEFT JOIN users u ON u.created_at >= w.since_at
        GROUP BY w.label
        "#,
    )
    .fetch_all(pool)
    .await?;
    for window in WINDOWS {
        USERS_CREATED_WINDOW.with_label_values(&[window]).set(0);
    }
    for (window, users) in created_windows {
        USERS_CREATED_WINDOW
            .with_label_values(&[&window])
            .set(users);
    }

    let active_windows = sqlx::query_as::<_, (String, String, i64)>(
        r#"
        WITH windows(label, since_at) AS (
            VALUES
                ('24h', NOW() - INTERVAL '24 hours'),
                ('7d', NOW() - INTERVAL '7 days'),
                ('30d', NOW() - INTERVAL '30 days')
        ),
        activity AS (
            SELECT user_id, last_login_at AS occurred_at, 'oauth_login' AS source
            FROM oauth_identities
            WHERE last_login_at >= NOW() - INTERVAL '30 days'

            UNION ALL
            SELECT user_id, last_seen, 'device_seen'
            FROM user_devices
            WHERE disabled_at IS NULL
              AND last_seen >= NOW() - INTERVAL '30 days'

            UNION ALL
            SELECT sender_id, created_at, 'product_activity'
            FROM friend_requests
            WHERE created_at >= NOW() - INTERVAL '30 days'

            UNION ALL
            SELECT receiver_id, responded_at, 'product_activity'
            FROM friend_requests
            WHERE responded_at >= NOW() - INTERVAL '30 days'

            UNION ALL
            SELECT created_by_user_id, created_at, 'product_activity'
            FROM event_share_tokens
            WHERE created_by_user_id IS NOT NULL
              AND created_at >= NOW() - INTERVAL '30 days'

            UNION ALL
            SELECT used_by_user_id, used_at, 'product_activity'
            FROM event_share_tokens
            WHERE used_by_user_id IS NOT NULL
              AND used_at >= NOW() - INTERVAL '30 days'

            UNION ALL
            SELECT user_id, generated_at, 'product_activity'
            FROM event_checkins
            WHERE generated_at >= NOW() - INTERVAL '30 days'

            UNION ALL
            SELECT scanned_by_user_id, scanned_at, 'product_activity'
            FROM event_checkins
            WHERE scanned_by_user_id IS NOT NULL
              AND scanned_at >= NOW() - INTERVAL '30 days'

            UNION ALL
            SELECT created_by, created_at, 'product_activity'
            FROM event_polls
            WHERE created_by IS NOT NULL
              AND created_at >= NOW() - INTERVAL '30 days'

            UNION ALL
            SELECT user_id, created_at, 'product_activity'
            FROM event_poll_votes
            WHERE created_at >= NOW() - INTERVAL '30 days'

            UNION ALL
            SELECT driver_id, created_at, 'product_activity'
            FROM carpools
            WHERE created_at >= NOW() - INTERVAL '30 days'

            UNION ALL
            SELECT user_id, joined_at, 'product_activity'
            FROM carpool_passengers
            WHERE joined_at >= NOW() - INTERVAL '30 days'

            UNION ALL
            SELECT created_by_user_id, created_at, 'product_activity'
            FROM event_expenses
            WHERE created_at >= NOW() - INTERVAL '30 days'

            UNION ALL
            SELECT user_a, created_at, 'product_activity'
            FROM friendships
            WHERE created_at >= NOW() - INTERVAL '30 days'

            UNION ALL
            SELECT user_b, created_at, 'product_activity'
            FROM friendships
            WHERE created_at >= NOW() - INTERVAL '30 days'
        ),
        per_source AS (
            SELECT w.label AS window, a.source, COUNT(DISTINCT a.user_id)::BIGINT AS users
            FROM windows w
            JOIN activity a ON a.occurred_at >= w.since_at
            GROUP BY w.label, a.source
        ),
        any_source AS (
            SELECT w.label AS window, 'any' AS source, COUNT(DISTINCT a.user_id)::BIGINT AS users
            FROM windows w
            JOIN activity a ON a.occurred_at >= w.since_at
            GROUP BY w.label
        )
        SELECT window, source, users FROM per_source
        UNION ALL
        SELECT window, source, users FROM any_source
        "#,
    )
    .fetch_all(pool)
    .await?;
    for window in WINDOWS {
        for source in ACTIVE_SOURCES {
            USERS_ACTIVE_WINDOW
                .with_label_values(&[window, source])
                .set(0);
        }
    }
    for (window, source, users) in active_windows {
        USERS_ACTIVE_WINDOW
            .with_label_values(&[&window, &source])
            .set(users);
    }

    let active_devices = sqlx::query_as::<_, (String, i64, i64)>(
        r#"
        SELECT platform, COUNT(*)::BIGINT, COUNT(DISTINCT user_id)::BIGINT
        FROM user_devices
        WHERE disabled_at IS NULL
        GROUP BY platform
        "#,
    )
    .fetch_all(pool)
    .await?;
    for platform in PLATFORMS {
        USERS_WITH_ACTIVE_DEVICE
            .with_label_values(&[platform])
            .set(0);
        USER_DEVICES_ACTIVE.with_label_values(&[platform]).set(0);
    }
    for (platform, devices, users) in active_devices {
        USER_DEVICES_ACTIVE
            .with_label_values(&[&platform])
            .set(devices);
        USERS_WITH_ACTIVE_DEVICE
            .with_label_values(&[&platform])
            .set(users);
    }

    let oauth_linked = sqlx::query_as::<_, (String, i64)>(
        r#"
        SELECT provider, COUNT(DISTINCT user_id)::BIGINT
        FROM oauth_identities
        GROUP BY provider
        "#,
    )
    .fetch_all(pool)
    .await?;
    for provider in OAUTH_PROVIDERS {
        USERS_OAUTH_LINKED.with_label_values(&[provider]).set(0);
    }
    for (provider, users) in oauth_linked {
        USERS_OAUTH_LINKED
            .with_label_values(&[&provider])
            .set(users);
    }

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0);
    USER_METRICS_REFRESH_TIMESTAMP_SECONDS.set(timestamp);

    Ok(())
}
