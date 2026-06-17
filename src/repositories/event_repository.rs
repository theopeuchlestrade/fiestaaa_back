use chrono::{NaiveDate, NaiveTime};
use sqlx::PgPool;

use crate::api_error::{ApiError, ApiResult};

#[derive(Debug, Clone)]
pub struct EventTiming {
    pub date_event: NaiveDate,
    pub start_time: NaiveTime,
    pub end_date: Option<NaiveDate>,
    pub end_time: Option<NaiveTime>,
}

pub async fn fetch_event_timing(db: &PgPool, event_id: i64) -> ApiResult<EventTiming> {
    let row = sqlx::query_as::<_, (NaiveDate, NaiveTime, Option<NaiveDate>, Option<NaiveTime>)>(
        "SELECT date_event, start_time, end_date, end_time FROM events WHERE event_id = $1",
    )
    .bind(event_id)
    .fetch_optional(db)
    .await
    .map_err(|_| ApiError::database())?;

    row.map(|(date_event, start_time, end_date, end_time)| EventTiming {
        date_event,
        start_time,
        end_date,
        end_time,
    })
    .ok_or_else(|| ApiError::not_found("event_not_found"))
}

pub async fn fetch_event_owner_id(db: &PgPool, event_id: i64) -> ApiResult<i64> {
    sqlx::query_scalar::<_, i64>("SELECT owner_user_id FROM events WHERE event_id = $1")
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
