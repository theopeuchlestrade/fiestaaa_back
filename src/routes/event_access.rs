use actix_web::HttpResponse;
use chrono::Utc;
use serde::Serialize;
use sqlx::PgPool;

use crate::{
    api_error::ApiError,
    models::ErrorResponse,
    repositories::event_repository::{self, EventTiming},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventLifecycleStatus {
    Upcoming,
    Ongoing,
    Finished,
}

pub fn status_from_timing(timing: &EventTiming) -> EventLifecycleStatus {
    let now = Utc::now();
    if now < timing.starts_at {
        EventLifecycleStatus::Upcoming
    } else if now <= timing.effective_ends_at {
        EventLifecycleStatus::Ongoing
    } else {
        EventLifecycleStatus::Finished
    }
}

pub async fn fetch_event_timing(db: &PgPool, event_id: i64) -> Result<EventTiming, HttpResponse> {
    event_repository::fetch_event_timing(db, event_id)
        .await
        .map_err(HttpResponse::from)
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
    let owner_user_id = event_repository::fetch_event_owner_id(db, event_id)
        .await
        .map_err(HttpResponse::from)?;
    let requester_id = event_repository::fetch_user_id_by_email(db, email)
        .await
        .map_err(HttpResponse::from)?;

    if requester_id.is_some_and(|id| id == owner_user_id) {
        return Ok(());
    }

    let is_member = event_repository::is_accepted_event_member(db, event_id, email)
        .await
        .map_err(HttpResponse::from)?;

    if is_member {
        Ok(())
    } else {
        Err(HttpResponse::from(ApiError::forbidden(
            "forbidden",
            "Accès refusé à cette fiestaaa",
        )))
    }
}
