use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::api_error::{ApiError, ApiResult};

#[derive(Debug, Clone)]
pub struct EventTiming {
    pub starts_at: DateTime<Utc>,
    pub effective_ends_at: DateTime<Utc>,
}

pub async fn fetch_event_timing(db: &PgPool, event_id: i64) -> ApiResult<EventTiming> {
    let row = sqlx::query_as::<_, (DateTime<Utc>, DateTime<Utc>)>(
        "SELECT starts_at, effective_ends_at
         FROM events
         WHERE event_id = $1 AND deleted_at IS NULL",
    )
    .bind(event_id)
    .fetch_optional(db)
    .await
    .map_err(|_| ApiError::database())?;

    row.map(|(starts_at, effective_ends_at)| EventTiming {
        starts_at,
        effective_ends_at,
    })
    .ok_or_else(|| ApiError::not_found("event_not_found"))
}

pub async fn fetch_event_owner_id(db: &PgPool, event_id: i64) -> ApiResult<i64> {
    sqlx::query_scalar::<_, i64>(
        "SELECT owner_user_id FROM events WHERE event_id = $1 AND deleted_at IS NULL",
    )
    .bind(event_id)
    .fetch_optional(db)
    .await
    .map_err(|_| ApiError::database())?
    .ok_or_else(|| ApiError::not_found("event_not_found"))
}

pub async fn fetch_user_id_by_email(db: &PgPool, email: &str) -> ApiResult<Option<i64>> {
    sqlx::query_scalar::<_, i64>(
        "SELECT id FROM users WHERE fiestaaa_email_matches(email_lookup_hash, $1)",
    )
    .bind(email)
    .fetch_optional(db)
    .await
    .map_err(|_| ApiError::database())
}

pub async fn is_accepted_event_member(db: &PgPool, event_id: i64, email: &str) -> ApiResult<bool> {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(
            SELECT 1
            FROM invitations i
            WHERE i.event_id = $1
              AND i.status = 'Accepted'
              AND i.user_id = (
                  SELECT id FROM users WHERE fiestaaa_email_matches(email_lookup_hash, $2)
              )
        )",
    )
    .bind(event_id)
    .bind(email)
    .fetch_one(db)
    .await
    .map_err(|_| ApiError::database())
}
