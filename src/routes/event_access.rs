use actix_web::HttpResponse;
use chrono::{NaiveDate, NaiveDateTime, NaiveTime, Utc};
use serde::Serialize;
use sqlx::{PgPool, Row};

use crate::models::ErrorResponse;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventLifecycleStatus {
    Upcoming,
    Ongoing,
    Finished,
}

#[derive(Debug, Clone)]
pub struct EventTiming {
    pub date_event: NaiveDate,
    pub start_time: NaiveTime,
    pub end_date: Option<NaiveDate>,
    pub end_time: Option<NaiveTime>,
}

impl EventTiming {
    pub fn start_at(&self) -> NaiveDateTime {
        self.date_event.and_time(self.start_time)
    }

    pub fn end_at(&self) -> Option<NaiveDateTime> {
        self.end_date
            .zip(self.end_time)
            .map(|(date, time)| date.and_time(time))
    }
}

pub fn status_from_timing(timing: &EventTiming) -> EventLifecycleStatus {
    let now = Utc::now().naive_utc();
    let start_at = timing.start_at();
    if let Some(end_at) = timing.end_at() {
        if now < start_at {
            EventLifecycleStatus::Upcoming
        } else if now <= end_at {
            EventLifecycleStatus::Ongoing
        } else {
            EventLifecycleStatus::Finished
        }
    } else if now < start_at {
        EventLifecycleStatus::Upcoming
    } else if now.date() == timing.date_event {
        EventLifecycleStatus::Ongoing
    } else {
        EventLifecycleStatus::Finished
    }
}

pub async fn fetch_event_timing(db: &PgPool, event_id: i64) -> Result<EventTiming, HttpResponse> {
    let row = sqlx::query_as::<_, (NaiveDate, NaiveTime, Option<NaiveDate>, Option<NaiveTime>)>(
        "SELECT date_event, start_time, end_date, end_time FROM events WHERE event_id = $1",
    )
    .bind(event_id)
    .fetch_optional(db)
    .await
    .map_err(|_| server_error())?;

    match row {
        Some((date_event, start_time, end_date, end_time)) => Ok(EventTiming {
            date_event,
            start_time,
            end_date,
            end_time,
        }),
        None => Err(HttpResponse::NotFound().json(ErrorResponse {
            error: "event_not_found".into(),
            details: None,
        })),
    }
}

pub async fn ensure_event_writable(db: &PgPool, event_id: i64) -> Result<(), HttpResponse> {
    let timing = fetch_event_timing(db, event_id).await?;
    if status_from_timing(&timing) == EventLifecycleStatus::Finished {
        Err(HttpResponse::Forbidden().json(ErrorResponse {
            error: "event_finished".into(),
            details: Some(
                "Cette fiestaaa est terminée. Les modifications ne sont plus autorisées.".into(),
            ),
        }))
    } else {
        Ok(())
    }
}

pub async fn ensure_event_member_email(
    db: &PgPool,
    event_id: i64,
    email: &str,
) -> Result<(), HttpResponse> {
    let owner_row = sqlx::query("SELECT owner_email FROM events WHERE event_id = $1")
        .bind(event_id)
        .fetch_optional(db)
        .await
        .map_err(|_| server_error())?;

    let Some(owner_row) = owner_row else {
        return Err(HttpResponse::NotFound().json(ErrorResponse {
            error: "event_not_found".into(),
            details: None,
        }));
    };

    let owner_email: String = owner_row.get("owner_email");
    if owner_email.eq_ignore_ascii_case(email) {
        return Ok(());
    }

    let is_member = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(
            SELECT 1
            FROM invitations i
            JOIN users u ON u.id = i.user_id
            WHERE i.event_id = $1
              AND i.status = 'Accepted'
              AND lower(u.email) = lower($2)
        )",
    )
    .bind(event_id)
    .bind(email)
    .fetch_one(db)
    .await
    .map_err(|_| server_error())?;

    if is_member {
        Ok(())
    } else {
        Err(HttpResponse::Forbidden().json(ErrorResponse {
            error: "forbidden".into(),
            details: Some("Accès refusé à cette fiestaaa".into()),
        }))
    }
}

fn server_error() -> HttpResponse {
    HttpResponse::InternalServerError().json(ErrorResponse {
        error: "db_error".into(),
        details: None,
    })
}
