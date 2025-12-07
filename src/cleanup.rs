use chrono::{Duration, Utc};
use log::{error, info};
use sqlx::{Pool, Postgres};
use tokio::time::{Duration as TokioDuration, interval};

const DEFAULT_CLEANUP_DAYS: i64 = 7;
const DEFAULT_CLEANUP_INTERVAL_HOURS: u64 = 1;

pub struct CleanupService {
    pool: Pool<Postgres>,
    cleanup_days: i64,
    cleanup_interval_hours: u64,
}

impl CleanupService {
    pub fn new(pool: Pool<Postgres>) -> Self {
        Self {
            pool,
            cleanup_days: DEFAULT_CLEANUP_DAYS,
            cleanup_interval_hours: DEFAULT_CLEANUP_INTERVAL_HOURS,
        }
    }

    pub fn with_cleanup_days(mut self, days: i64) -> Self {
        self.cleanup_days = days;
        self
    }

    pub fn with_interval_hours(mut self, hours: u64) -> Self {
        self.cleanup_interval_hours = hours;
        self
    }

    pub fn start(self) {
        let pool = self.pool;
        let cleanup_days = self.cleanup_days;
        let interval_hours = self.cleanup_interval_hours;

        tokio::spawn(async move {
            info!(
                "Service de nettoyage démarré - Suppression des événements après {} jours, vérification toutes les {} heures",
                cleanup_days, interval_hours
            );

            if let Err(e) = cleanup_expired_events(&pool, cleanup_days).await {
                error!("Erreur lors du nettoyage initial des événements: {}", e);
            }

            let mut interval = interval(TokioDuration::from_secs(interval_hours * 60 * 60));

            loop {
                interval.tick().await;

                if let Err(e) = cleanup_expired_events(&pool, cleanup_days).await {
                    error!("Erreur lors du nettoyage des événements: {}", e);
                }
            }
        });
    }
}

async fn cleanup_expired_events(pool: &Pool<Postgres>, days: i64) -> Result<(), sqlx::Error> {
    let cutoff_date = Utc::now().naive_utc().date() - Duration::days(days);

    let result = sqlx::query(
        r#"
        DELETE FROM events
        WHERE date_event < $1
        RETURNING event_id, name_event, date_event
        "#,
    )
    .bind(cutoff_date)
    .fetch_all(pool)
    .await?;

    if !result.is_empty() {
        info!(
            "{} événement(s) supprimé(s) (plus vieux que {} jours)",
            result.len(),
            days
        );
    }

    Ok(())
}

pub async fn cleanup_now(pool: &Pool<Postgres>, days: i64) -> Result<u64, sqlx::Error> {
    let cutoff_date = Utc::now().naive_utc().date() - Duration::days(days);

    let result = sqlx::query(
        r#"
        DELETE FROM events
        WHERE date_event < $1
        "#,
    )
    .bind(cutoff_date)
    .execute(pool)
    .await?;

    let deleted = result.rows_affected();

    if deleted > 0 {
        info!(
            "Nettoyage manuel: {} événement(s) supprimé(s) (plus vieux que {} jours)",
            deleted, days
        );
    }

    Ok(deleted)
}
