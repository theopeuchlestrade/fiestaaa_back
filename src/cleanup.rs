use log::{error, info};
use sqlx::{Pool, Postgres};
use tokio::time::{Duration as TokioDuration, interval};

use crate::auth;

const DEFAULT_CLEANUP_DAYS: i64 = 7;
const DEFAULT_PURGE_DAYS: i64 = 30;
const DEFAULT_CLEANUP_INTERVAL_HOURS: u64 = 1;

pub struct CleanupService {
    pool: Pool<Postgres>,
    cleanup_days: i64,
    purge_days: i64,
    cleanup_interval_hours: u64,
}

impl CleanupService {
    pub fn new(pool: Pool<Postgres>) -> Self {
        Self {
            pool,
            cleanup_days: DEFAULT_CLEANUP_DAYS,
            purge_days: DEFAULT_PURGE_DAYS,
            cleanup_interval_hours: DEFAULT_CLEANUP_INTERVAL_HOURS,
        }
    }

    pub fn with_cleanup_days(mut self, days: i64) -> Self {
        self.cleanup_days = days;
        self
    }

    pub fn with_purge_days(mut self, days: i64) -> Self {
        self.purge_days = days;
        self
    }

    pub fn with_interval_hours(mut self, hours: u64) -> Self {
        self.cleanup_interval_hours = hours;
        self
    }

    pub fn start(self) {
        let pool = self.pool;
        let cleanup_days = self.cleanup_days;
        let purge_days = self.purge_days;
        let interval_hours = self.cleanup_interval_hours;

        tokio::spawn(async move {
            info!(
                "Service de nettoyage démarré - Suppression des événements après {} jours, vérification toutes les {} heures",
                cleanup_days, interval_hours
            );

            if let Err(e) = cleanup_expired_events(&pool, cleanup_days, purge_days).await {
                error!("Erreur lors du nettoyage initial des événements: {}", e);
            }
            if let Err(e) = cleanup_auth_state(&pool).await {
                error!("Erreur lors du nettoyage initial des sessions: {}", e);
            }

            let mut interval = interval(TokioDuration::from_secs(interval_hours * 60 * 60));

            loop {
                interval.tick().await;

                if let Err(e) = cleanup_expired_events(&pool, cleanup_days, purge_days).await {
                    error!("Erreur lors du nettoyage des événements: {}", e);
                }
                if let Err(e) = cleanup_auth_state(&pool).await {
                    error!("Erreur lors du nettoyage des sessions: {}", e);
                }
            }
        });
    }
}

async fn cleanup_auth_state(pool: &Pool<Postgres>) -> Result<(), sqlx::Error> {
    auth::cleanup_expired_revoked_tokens(pool).await?;
    sqlx::query(
        "UPDATE invitations i
         SET status = 'Expired'
         FROM events e
         WHERE i.event_id = e.event_id
           AND i.status = 'Waiting'
           AND e.invitation_deadline IS NOT NULL
           AND CURRENT_DATE > e.invitation_deadline",
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn cleanup_expired_events(
    pool: &Pool<Postgres>,
    days: i64,
    purge_days: i64,
) -> Result<(), sqlx::Error> {
    let result = sqlx::query(
        r#"
        UPDATE events
        SET deleted_at = NOW(),
            purge_at = NOW() + make_interval(days => $2),
            deletion_reason = 'retention'
        WHERE deleted_at IS NULL
          AND effective_ends_at < NOW() - make_interval(days => $1)
        RETURNING event_id
        "#,
    )
    .bind(days as i32)
    .bind(purge_days as i32)
    .fetch_all(pool)
    .await?;

    if !result.is_empty() {
        info!(
            "{} événement(s) placé(s) dans la corbeille après {} jours",
            result.len(),
            days
        );
    }

    let purged = sqlx::query("DELETE FROM events WHERE purge_at <= NOW()")
        .execute(pool)
        .await?
        .rows_affected();
    if purged > 0 {
        info!("{purged} événement(s) purgé(s) définitivement");
    }

    Ok(())
}

pub async fn cleanup_now(pool: &Pool<Postgres>, days: i64) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        r#"
        UPDATE events
        SET deleted_at = NOW(),
            purge_at = NOW() + make_interval(days => 30),
            deletion_reason = 'retention'
        WHERE deleted_at IS NULL
          AND effective_ends_at < NOW() - make_interval(days => $1)
        "#,
    )
    .bind(days as i32)
    .execute(pool)
    .await?;

    let deleted = result.rows_affected();

    if deleted > 0 {
        info!(
            "Nettoyage manuel: {} événement(s) placé(s) dans la corbeille après {} jours",
            deleted, days
        );
    }

    Ok(deleted)
}
