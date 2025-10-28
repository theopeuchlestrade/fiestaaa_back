use actix_web::{delete, get, patch, post, put, web, HttpRequest, HttpResponse, Responder};
use sqlx::Error;

use crate::{
    auth::extract_claims_from_auth,
    models::{ErrorResponse, Event, EventPatchPayload, EventPayload, StatusResponse},
    state::AppState,
};

fn ensure_admin(req: &HttpRequest, state: &AppState) -> Result<(), HttpResponse> {
    let claims = extract_claims_from_auth(req, &state.jwt_secret)?;
    if state.admin_emails.is_empty() {
        return Ok(());
    }

    if state.admin_emails.contains(&claims.sub.to_lowercase()) {
        Ok(())
    } else {
        Err(HttpResponse::Forbidden().json(ErrorResponse {
            error: "forbidden".into(),
            details: Some("admin privileges required".into()),
        }))
    }
}

#[utoipa::path(
    get,
    path = "/events",
    tag = "events",
    responses(
        (status = 200, description = "Liste des événements", body = [Event]),
        (status = 500, description = "Erreur base de données", body = ErrorResponse)
    )
)]
#[get("/events")]
pub async fn list_events(state: web::Data<AppState>) -> impl Responder {
    let res = sqlx::query_as::<_, Event>(
        "SELECT event_id, name_event, description, date_event, start_time, address, 
         payment_provider_id, payment_identifier 
         FROM events ORDER BY date_event, start_time",
    )
    .fetch_all(&state.db)
    .await;

    match res {
        Ok(events) => HttpResponse::Ok().json(events),
        Err(_) => HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        }),
    }
}

#[utoipa::path(
    post,
    path = "/events",
    tag = "events",
    request_body = EventPayload,
    responses(
        (status = 201, description = "Événement créé", body = Event),
        (status = 400, description = "Payload invalide", body = ErrorResponse),
        (status = 403, description = "Non autorisé", body = ErrorResponse),
        (status = 500, description = "Erreur base de données", body = ErrorResponse)
    )
)]
#[post("/events")]
pub async fn create_event(
    state: web::Data<AppState>,
    req: HttpRequest,
    payload: web::Json<EventPayload>,
) -> impl Responder {
    if let Err(resp) = ensure_admin(&req, state.get_ref()) {
        return resp;
    }

    let payload = payload.into_inner();
    if payload.name_event.trim().is_empty() || payload.description.trim().is_empty() || payload.address.trim().is_empty() {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("Les champs name_event, description et address ne peuvent pas être vides".into()),
        });
    }

    let res = sqlx::query_as::<_, Event>(
        "INSERT INTO events (name_event, description, date_event, start_time, address, payment_provider_id, payment_identifier)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         RETURNING event_id, name_event, description, date_event, start_time, address, payment_provider_id, payment_identifier",
    )
    .bind(payload.name_event.trim())
    .bind(payload.description.trim())
    .bind(payload.date_event)
    .bind(payload.start_time)
    .bind(payload.address.trim())
    .bind(payload.payment_provider_id)
    .bind(payload.payment_identifier)
    .fetch_one(&state.db)
    .await;

    match res {
        Ok(event) => HttpResponse::Created().json(event),
        Err(Error::Database(db_err)) if db_err.code().as_deref() == Some("23503") => {
            HttpResponse::BadRequest().json(ErrorResponse {
                error: "unknown_payment_provider".into(),
                details: None,
            })
        }
        Err(_) => HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        }),
    }
}

#[utoipa::path(
    put,
    path = "/events/{event_id}",
    tag = "events",
    request_body = EventPayload,
    responses(
        (status = 200, description = "Événement mis à jour", body = Event),
        (status = 400, description = "Payload invalide", body = ErrorResponse),
        (status = 403, description = "Non autorisé", body = ErrorResponse),
        (status = 404, description = "Événement introuvable", body = ErrorResponse),
        (status = 500, description = "Erreur base de données", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Identifiant de l'événement")
    )
)]
#[put("/events/{event_id}")]
pub async fn replace_event(
    state: web::Data<AppState>,
    req: HttpRequest,
    event_id: web::Path<i64>,
    payload: web::Json<EventPayload>,
) -> impl Responder {
    if let Err(resp) = ensure_admin(&req, state.get_ref()) {
        return resp;
    }

    let payload = payload.into_inner();
    if payload.name_event.trim().is_empty() || payload.description.trim().is_empty() || payload.address.trim().is_empty() {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("Les champs name_event, description et address ne peuvent pas être vides".into()),
        });
    }

    let res = sqlx::query_as::<_, Event>(
        "UPDATE events
         SET name_event = $1, description = $2, date_event = $3, start_time = $4, 
             address = $5, payment_provider_id = $6, payment_identifier = $7
         WHERE event_id = $8
         RETURNING event_id, name_event, description, date_event, start_time, address, payment_provider_id, payment_identifier",
    )
    .bind(payload.name_event.trim())
    .bind(payload.description.trim())
    .bind(payload.date_event)
    .bind(payload.start_time)
    .bind(payload.address.trim())
    .bind(payload.payment_provider_id)
    .bind(payload.payment_identifier)
    .bind(*event_id)
    .fetch_optional(&state.db)
    .await;

    match res {
        Ok(Some(event)) => HttpResponse::Ok().json(event),
        Ok(None) => HttpResponse::NotFound().json(ErrorResponse {
            error: "event_not_found".into(),
            details: None,
        }),
        Err(Error::Database(db_err)) if db_err.code().as_deref() == Some("23503") => {
            HttpResponse::BadRequest().json(ErrorResponse {
                error: "unknown_payment_provider".into(),
                details: None,
            })
        }
        Err(_) => HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        }),
    }
}

#[utoipa::path(
    patch,
    path = "/events/{event_id}",
    tag = "events",
    request_body = EventPatchPayload,
    responses(
        (status = 200, description = "Événement modifié", body = Event),
        (status = 400, description = "Payload invalide", body = ErrorResponse),
        (status = 403, description = "Non autorisé", body = ErrorResponse),
        (status = 404, description = "Événement introuvable", body = ErrorResponse),
        (status = 500, description = "Erreur base de données", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Identifiant de l'événement")
    )
)]
#[patch("/events/{event_id}")]
pub async fn update_event(
    state: web::Data<AppState>,
    req: HttpRequest,
    event_id: web::Path<i64>,
    payload: web::Json<EventPatchPayload>,
) -> impl Responder {
    if let Err(resp) = ensure_admin(&req, state.get_ref()) {
        return resp;
    }

    let payload = payload.into_inner();
    if payload.name_event.as_ref().is_some_and(|v| v.trim().is_empty())
        || payload.description.as_ref().is_some_and(|v| v.trim().is_empty())
        || payload.address.as_ref().is_some_and(|v| v.trim().is_empty())
    {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("Les champs name_event, description et address ne peuvent pas être vides".into()),
        });
    }

    let res = sqlx::query_as::<_, Event>(
        "UPDATE events
         SET name_event = COALESCE($1, name_event),
             description = COALESCE($2, description),
             date_event = COALESCE($3, date_event),
             start_time = COALESCE($4, start_time),
             address = COALESCE($5, address),
             payment_provider_id = COALESCE($6, payment_provider_id),
             payment_identifier = COALESCE($7, payment_identifier)
         WHERE event_id = $8
         RETURNING event_id, name_event, description, date_event, start_time, address, payment_provider_id, payment_identifier",
    )
    .bind(payload.name_event.as_ref().map(|v| v.trim()).filter(|v| !v.is_empty()))
    .bind(payload.description.as_ref().map(|v| v.trim()).filter(|v| !v.is_empty()))
    .bind(payload.date_event)
    .bind(payload.start_time)
    .bind(payload.address.as_ref().map(|v| v.trim()).filter(|v| !v.is_empty()))
    .bind(payload.payment_provider_id)
    .bind(payload.payment_identifier)
    .bind(*event_id)
    .fetch_optional(&state.db)
    .await;

    match res {
        Ok(Some(event)) => HttpResponse::Ok().json(event),
        Ok(None) => HttpResponse::NotFound().json(ErrorResponse {
            error: "event_not_found".into(),
            details: None,
        }),
        Err(Error::Database(db_err)) if db_err.code().as_deref() == Some("23503") => {
            HttpResponse::BadRequest().json(ErrorResponse {
                error: "unknown_payment_provider".into(),
                details: None,
            })
        }
        Err(_) => HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        }),
    }
}

#[utoipa::path(
    delete,
    path = "/events/{event_id}",
    tag = "events",
    responses(
        (status = 200, description = "Événement supprimé", body = StatusResponse),
        (status = 403, description = "Non autorisé", body = ErrorResponse),
        (status = 404, description = "Événement introuvable", body = ErrorResponse),
        (status = 500, description = "Erreur base de données", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Identifiant de l'événement")
    )
)]
#[delete("/events/{event_id}")]
pub async fn delete_event(
    state: web::Data<AppState>,
    req: HttpRequest,
    event_id: web::Path<i64>,
) -> impl Responder {
    if let Err(resp) = ensure_admin(&req, state.get_ref()) {
        return resp;
    }

    let res = sqlx::query("DELETE FROM events WHERE event_id = $1")
        .bind(*event_id)
        .execute(&state.db)
        .await;

    match res {
        Ok(result) if result.rows_affected() == 0 => HttpResponse::NotFound().json(ErrorResponse {
            error: "event_not_found".into(),
            details: None,
        }),
        Ok(_) => HttpResponse::Ok().json(StatusResponse {
            status: "deleted".into(),
        }),
        Err(_) => HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        }),
    }
}