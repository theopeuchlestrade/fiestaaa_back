use actix_web::{HttpRequest, HttpResponse, Responder, delete, get, patch, post, put, web};
use chrono::{Duration, NaiveDate, Utc};
use log::warn;
use regex::Regex;
use serde::Deserialize;
use serde_json::json;
use sqlx::{Error, PgPool, Row};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

use crate::{
    auth::extract_claims_from_auth,
    models::{
        AddressSuggestion, ErrorResponse, Event, EventCustomItemPayload, EventItemAttachPayload,
        EventItemReservationPayload, EventItemView, EventPatchPayload, EventPayload,
        EventPollCreatePayload, EventPollVotePayload, ItemContribution, PollOptionView,
        PollOptionVoter, PollView, ShareClaimPayload, ShareClaimResponse, ShareTokenResponse,
        StatusResponse,
    },
    notifications::{event_member_user_ids, notify_users},
    realtime::publish_event,
    state::AppState,
};

fn claims_email(req: &HttpRequest, state: &AppState) -> Result<String, HttpResponse> {
    let claims = extract_claims_from_auth(req, &state.jwt_secret)?;
    Ok(claims.sub.to_lowercase())
}

async fn fetch_event_owner_email(db: &PgPool, event_id: i64) -> Result<String, HttpResponse> {
    let owner =
        sqlx::query_scalar::<_, String>("SELECT owner_email FROM events WHERE event_id = $1")
            .bind(event_id)
            .fetch_optional(db)
            .await
            .map_err(|_| {
                HttpResponse::InternalServerError().json(ErrorResponse {
                    error: "db_error".into(),
                    details: None,
                })
            })?;

    match owner {
        Some(email) => Ok(email),
        None => Err(HttpResponse::NotFound().json(ErrorResponse {
            error: "event_not_found".into(),
            details: None,
        })),
    }
}

async fn ensure_event_owner(
    req: &HttpRequest,
    state: &AppState,
    event_id: i64,
) -> Result<(), HttpResponse> {
    let requester = claims_email(req, state)?;
    let owner = fetch_event_owner_email(&state.db, event_id).await?;
    if owner == requester || owner.is_empty() {
        Ok(())
    } else {
        Err(HttpResponse::Forbidden().json(ErrorResponse {
            error: "forbidden".into(),
            details: Some("only the creator can perform this action".into()),
        }))
    }
}

async fn ensure_event_member(
    req: &HttpRequest,
    state: &AppState,
    event_id: i64,
) -> Result<(), HttpResponse> {
    let requester = claims_email(req, state)?;
    let owner = fetch_event_owner_email(&state.db, event_id).await?;
    if owner.eq_ignore_ascii_case(&requester) || owner.is_empty() {
        return Ok(());
    }

    let is_member = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(
            SELECT 1
            FROM invitations i
            JOIN users u ON u.id = i.user_id
            WHERE i.event_id = $1
              AND lower(u.email) = lower($2)
              AND i.status = 'Accepted'
        )",
    )
    .bind(event_id)
    .bind(&requester)
    .fetch_one(&state.db)
    .await
    .map_err(|_| server_error())?;

    if is_member {
        Ok(())
    } else {
        Err(HttpResponse::Forbidden().json(ErrorResponse {
            error: "forbidden".into(),
            details: Some("membership required".into()),
        }))
    }
}

async fn notify_event_members(state: &AppState, event: &Event, updated_fields: &[&str]) {
    if updated_fields.is_empty() || !state.notifications.is_enabled() {
        return;
    }

    let members = match event_member_user_ids(&state.db, event.event_id).await {
        Ok(list) => list,
        Err(err) => {
            warn!("failed to load event members for notifications: {err}");
            return;
        }
    };

    if members.is_empty() {
        return;
    }

    let fields: Vec<String> = updated_fields.iter().map(|f| f.to_string()).collect();
    let dedup = format!("event_updated:{}", event.event_id);
    notify_users(
        &state.notifications,
        &state.db,
        &members,
        "Événement mis à jour",
        &format!("{} a été mis à jour", event.name_event),
        json!({
            "type": "event_updated",
            "event_id": event.event_id,
            "event_name": event.name_event,
            "fields": fields
        }),
        Some(&dedup),
        Some(300),
    )
    .await;
}

fn validate_invitation_deadline(
    deadline: Option<NaiveDate>,
    event_date: NaiveDate,
) -> Result<(), HttpResponse> {
    if let Some(limit) = deadline {
        let today = chrono::Utc::now().date_naive();
        if limit < today {
            return Err(HttpResponse::BadRequest().json(ErrorResponse {
                error: "invalid_invitation_deadline".into(),
                details: Some("La date limite de réponse ne peut pas être dans le passé".into()),
            }));
        }
        if limit > event_date {
            return Err(HttpResponse::BadRequest().json(ErrorResponse {
                error: "invalid_invitation_deadline".into(),
                details: Some(
                    "La date limite de réponse doit être avant ou au jour de l'événement".into(),
                ),
            }));
        }
    }

    Ok(())
}

async fn ensure_invitation_deadline_schema(db: &PgPool) -> Result<(), HttpResponse> {
    if let Err(_) =
        sqlx::query("ALTER TABLE events ADD COLUMN IF NOT EXISTS invitation_deadline DATE")
            .execute(db)
            .await
    {
        return Err(server_error());
    }

    let ensure_constraint = r#"
        DO $$
        BEGIN
            BEGIN
                ALTER TABLE events
                ADD CONSTRAINT invitation_deadline_before_event
                CHECK (invitation_deadline IS NULL OR invitation_deadline <= date_event);
            EXCEPTION
                WHEN duplicate_object THEN
                    NULL;
            END;
        END
        $$;
    "#;

    if let Err(_) = sqlx::query(ensure_constraint).execute(db).await {
        return Err(server_error());
    }

    Ok(())
}

async fn expire_overdue_invitations(db: &PgPool) -> Result<(), HttpResponse> {
    ensure_invitation_deadline_schema(db).await?;
    sqlx::query(
        "UPDATE invitations i
         SET status = 'Expired'
         FROM events e
         WHERE i.event_id = e.event_id
           AND i.status = 'Waiting'
           AND e.invitation_deadline IS NOT NULL
           AND CURRENT_DATE > e.invitation_deadline",
    )
    .execute(db)
    .await
    .map(|_| ())
    .map_err(|_| server_error())
}

#[derive(Deserialize)]
pub struct AddressSearchQuery {
    pub q: String,
    pub limit: Option<u8>,
}

#[utoipa::path(
    get,
    path = "/geo/address-search",
    tag = "events",
    params(
        ("q" = String, Query, description = "Adresse ou lieu à rechercher"),
        ("limit" = u8, Query, description = "Nombre maximum de suggestions (1-10)")
    ),
    responses(
        (status = 200, description = "Suggestions géocodées", body = [AddressSuggestion]),
        (status = 400, description = "Requête trop courte", body = ErrorResponse),
        (status = 401, description = "Authentification requise", body = ErrorResponse),
        (status = 502, description = "Service de géocodage indisponible", body = ErrorResponse)
    )
)]
#[get("/geo/address-search")]
pub async fn search_address(
    state: web::Data<AppState>,
    req: HttpRequest,
    params: web::Query<AddressSearchQuery>,
) -> impl Responder {
    if let Err(resp) = claims_email(&req, state.get_ref()) {
        return resp;
    }

    let query = params.q.trim();
    if query.len() < 3 {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "query_too_short".into(),
            details: Some("Au moins 3 caractères requis pour la recherche".into()),
        });
    }
    let limit = params.limit.unwrap_or(5).clamp(1, 10);

    match fetch_address_suggestions(
        &state.http_client,
        &state.geocoding_base_url,
        state.geocoding_country_codes.as_deref(),
        query,
        limit,
    )
    .await
    {
        Ok(results) => HttpResponse::Ok().json(results),
        Err(resp) => resp,
    }
}

async fn fetch_address_suggestions(
    client: &reqwest::Client,
    base_url: &str,
    country_codes: Option<&str>,
    query: &str,
    limit: u8,
) -> Result<Vec<AddressSuggestion>, HttpResponse> {
    let mut url = match reqwest::Url::parse(&format!("{}/search", base_url.trim_end_matches('/'))) {
        Ok(url) => url,
        Err(_) => {
            return Err(HttpResponse::InternalServerError().json(ErrorResponse {
                error: "geocoding_config_error".into(),
                details: None,
            }));
        }
    };

    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair("format", "jsonv2");
        pairs.append_pair("addressdetails", "0");
        pairs.append_pair("limit", &limit.to_string());
        pairs.append_pair("q", query);
        if let Some(cc) = country_codes {
            pairs.append_pair("countrycodes", cc);
        }
    }

    #[derive(Deserialize)]
    struct NominatimPlace {
        display_name: String,
        lat: String,
        lon: String,
    }

    let response = client.get(url).send().await.map_err(|_| {
        HttpResponse::BadGateway().json(ErrorResponse {
            error: "geocoding_unreachable".into(),
            details: None,
        })
    })?;

    if !response.status().is_success() {
        return Err(HttpResponse::BadGateway().json(ErrorResponse {
            error: "geocoding_error".into(),
            details: Some(format!("Status: {}", response.status())),
        }));
    }

    let places: Vec<NominatimPlace> = response.json().await.map_err(|_| {
        HttpResponse::BadGateway().json(ErrorResponse {
            error: "geocoding_parse_error".into(),
            details: None,
        })
    })?;

    let suggestions = places
        .into_iter()
        .filter_map(|place| {
            let lat = place.lat.parse::<f64>().ok()?;
            let lon = place.lon.parse::<f64>().ok()?;
            Some(AddressSuggestion {
                label: place.display_name,
                latitude: lat,
                longitude: lon,
            })
        })
        .collect();

    Ok(suggestions)
}

#[utoipa::path(
    get,
    path = "/events/{event_id}",
    tag = "events",
    responses(
        (status = 200, description = "Événement trouvé", body = Event),
        (status = 401, description = "Authentification requise", body = ErrorResponse),
        (status = 403, description = "Accès non autorisé", body = ErrorResponse),
        (status = 404, description = "Événement introuvable", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Identifiant de l'événement")
    )
)]
#[get("/events/{event_id}")]
pub async fn get_event(
    state: web::Data<AppState>,
    req: HttpRequest,
    event_id: web::Path<i64>,
) -> impl Responder {
    if let Err(resp) = ensure_event_member(&req, state.get_ref(), *event_id).await {
        return resp;
    }

    match sqlx::query_as::<_, Event>(
        "SELECT event_id,
                name_event,
                description,
                date_event,
                start_time,
                invitation_deadline,
                address,
                latitude,
                longitude,
                payment_provider_id,
                payment_identifier,
                payment_requested_amount,
                payment_per_person,
                owner_email
         FROM events
         WHERE event_id = $1",
    )
    .bind(*event_id)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(event)) => HttpResponse::Ok().json(event),
        Ok(None) => HttpResponse::NotFound().json(ErrorResponse {
            error: "event_not_found".into(),
            details: None,
        }),
        Err(_) => server_error(),
    }
}

#[utoipa::path(
    get,
    path = "/events",
    tag = "events",
    responses(
        (status = 200, description = "Liste des événements", body = [Event]),
        (status = 401, description = "Authentification requise", body = ErrorResponse),
        (status = 500, description = "Erreur base de données", body = ErrorResponse)
    )
)]
#[get("/events")]
pub async fn list_events(state: web::Data<AppState>, req: HttpRequest) -> impl Responder {
    let email = match claims_email(&req, state.get_ref()) {
        Ok(e) => e,
        Err(resp) => return resp,
    };
    if let Err(resp) = expire_overdue_invitations(&state.db).await {
        return resp;
    }

    let res = sqlx::query_as::<_, Event>(
        "SELECT e.event_id,
                e.name_event,
                e.description,
                e.date_event,
                e.start_time,
                e.invitation_deadline,
                e.address,
                e.latitude,
                e.longitude,
                e.payment_provider_id,
                e.payment_identifier,
                e.payment_requested_amount,
                e.payment_per_person,
                e.owner_email
         FROM events e
         WHERE lower(e.owner_email) = lower($1)
            OR EXISTS (
                SELECT 1
                FROM invitations i
                JOIN users u ON u.id = i.user_id
                WHERE i.event_id = e.event_id
                  AND lower(u.email) = lower($1)
                  AND i.status NOT IN ('Declined', 'Expired')
            )
         ORDER BY e.date_event, e.start_time",
    )
    .bind(&email)
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
    if let Err(resp) = ensure_invitation_deadline_schema(&state.db).await {
        return resp;
    }
    let owner_email = match claims_email(&req, state.get_ref()) {
        Ok(email) => email,
        Err(resp) => return resp,
    };
    let payload = payload.into_inner();
    if payload.name_event.trim().is_empty()
        || payload.description.trim().is_empty()
        || payload.address.trim().is_empty()
    {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some(
                "Les champs name_event, description et address ne peuvent pas être vides".into(),
            ),
        });
    }
    if payload.latitude.is_some() ^ payload.longitude.is_some() {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("Latitude et longitude doivent être renseignées ensemble".into()),
        });
    }
    if let Err(resp) = validate_invitation_deadline(payload.invitation_deadline, payload.date_event)
    {
        return resp;
    }
    let (payment_provider_id, payment_identifier) = match normalize_payment_info(
        &state.db,
        payload.payment_provider_id,
        payload.payment_identifier,
    )
    .await
    {
        Ok(values) => values,
        Err(resp) => return resp,
    };
    let payment_per_person = if payment_provider_id.is_none() {
        false
    } else {
        payload.payment_per_person.unwrap_or(false)
    };

    let res = sqlx::query_as::<_, Event>(
        "INSERT INTO events (name_event, description, date_event, start_time, invitation_deadline, address, latitude, longitude, payment_provider_id, payment_identifier, payment_requested_amount, payment_per_person, owner_email)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
         RETURNING event_id, name_event, description, date_event, start_time, invitation_deadline, address, latitude, longitude, payment_provider_id, payment_identifier, payment_requested_amount, payment_per_person, owner_email",
    )
    .bind(payload.name_event.trim())
    .bind(payload.description.trim())
    .bind(payload.date_event)
    .bind(payload.start_time)
    .bind(payload.invitation_deadline)
    .bind(payload.address.trim())
    .bind(payload.latitude)
    .bind(payload.longitude)
    .bind(payment_provider_id)
    .bind(payment_identifier)
    .bind(payload.payment_requested_amount)
    .bind(payment_per_person)
    .bind(owner_email)
    .fetch_one(&state.db)
    .await;

    match res {
        Ok(event) => {
            publish_event(
                &state.redis_client,
                event.event_id,
                &json!({"type": "event_updated", "event_id": event.event_id, "event": &event}),
            )
            .await;
            HttpResponse::Created().json(event)
        }
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
    if let Err(resp) = ensure_invitation_deadline_schema(&state.db).await {
        return resp;
    }
    if let Err(resp) = ensure_event_owner(&req, state.get_ref(), *event_id).await {
        return resp;
    }

    let payload = payload.into_inner();
    let updated_fields = vec!["name", "description", "date", "time", "location"];
    if payload.name_event.trim().is_empty()
        || payload.description.trim().is_empty()
        || payload.address.trim().is_empty()
    {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some(
                "Les champs name_event, description et address ne peuvent pas être vides".into(),
            ),
        });
    }

    let (payment_provider_id, payment_identifier) = match normalize_payment_info(
        &state.db,
        payload.payment_provider_id,
        payload.payment_identifier,
    )
    .await
    {
        Ok(values) => values,
        Err(resp) => return resp,
    };
    let payment_per_person = if payment_provider_id.is_none() {
        false
    } else {
        payload.payment_per_person.unwrap_or(false)
    };

    let res = sqlx::query_as::<_, Event>(
        "UPDATE events
         SET name_event = $1, description = $2, date_event = $3, start_time = $4, 
             invitation_deadline = $5, address = $6, latitude = $7, longitude = $8, payment_provider_id = $9, payment_identifier = $10,
             payment_requested_amount = $11, payment_per_person = $12
         WHERE event_id = $13
         RETURNING event_id, name_event, description, date_event, start_time, invitation_deadline, address, latitude, longitude, payment_provider_id, payment_identifier, payment_requested_amount, payment_per_person, owner_email",
    )
    .bind(payload.name_event.trim())
    .bind(payload.description.trim())
    .bind(payload.date_event)
    .bind(payload.start_time)
    .bind(payload.invitation_deadline)
    .bind(payload.address.trim())
    .bind(payload.latitude)
    .bind(payload.longitude)
    .bind(payment_provider_id)
    .bind(payment_identifier)
    .bind(payload.payment_requested_amount)
    .bind(payment_per_person)
    .bind(*event_id)
    .fetch_optional(&state.db)
    .await;

    match res {
        Ok(Some(event)) => {
            notify_event_members(state.get_ref(), &event, &updated_fields).await;
            publish_event(
                &state.redis_client,
                event.event_id,
                &json!({"type": "event_updated", "event_id": event.event_id, "event": &event}),
            )
            .await;
            HttpResponse::Ok().json(event)
        }
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
    if let Err(resp) = ensure_invitation_deadline_schema(&state.db).await {
        return resp;
    }
    if let Err(resp) = ensure_event_owner(&req, state.get_ref(), *event_id).await {
        return resp;
    }

    let payload = payload.into_inner();
    let mut updated_fields: Vec<&str> = Vec::new();
    if payload.name_event.is_some() {
        updated_fields.push("name");
    }
    if payload.description.is_some() {
        updated_fields.push("description");
    }
    if payload.date_event.is_some() {
        updated_fields.push("date");
    }
    if payload.start_time.is_some() {
        updated_fields.push("time");
    }
    if payload.address.is_some() || payload.latitude.is_some() || payload.longitude.is_some() {
        updated_fields.push("location");
    }
    if payload
        .name_event
        .as_ref()
        .is_some_and(|v| v.trim().is_empty())
        || payload
            .description
            .as_ref()
            .is_some_and(|v| v.trim().is_empty())
        || payload
            .address
            .as_ref()
            .is_some_and(|v| v.trim().is_empty())
    {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some(
                "Les champs name_event, description et address ne peuvent pas être vides".into(),
            ),
        });
    }
    if payload.latitude.is_some() ^ payload.longitude.is_some() {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("Latitude et longitude doivent être renseignées ensemble".into()),
        });
    }

    let (
        current_provider_id,
        current_identifier,
        current_payment_per_person,
        current_date_event,
        current_invitation_deadline,
    ) = match fetch_event_payment_info(&state.db, *event_id).await {
        Ok(info) => info,
        Err(resp) => return resp,
    };

    let merged_provider = payload.payment_provider_id.or(current_provider_id);
    let merged_identifier = payload.payment_identifier.clone().or(current_identifier);
    let (payment_provider_id, payment_identifier) =
        match normalize_payment_info(&state.db, merged_provider, merged_identifier).await {
            Ok(values) => values,
            Err(resp) => return resp,
        };
    let payment_per_person = if payment_provider_id.is_none() {
        false
    } else {
        payload
            .payment_per_person
            .unwrap_or(current_payment_per_person)
    };
    let target_date_event = payload.date_event.unwrap_or(current_date_event);
    let target_deadline = payload.invitation_deadline.or(current_invitation_deadline);
    if let Err(resp) = validate_invitation_deadline(target_deadline, target_date_event) {
        return resp;
    }

    let res = sqlx::query_as::<_, Event>(
        "UPDATE events
         SET name_event = COALESCE($1, name_event),
             description = COALESCE($2, description),
             date_event = COALESCE($3, date_event),
             start_time = COALESCE($4, start_time),
             invitation_deadline = COALESCE($5, invitation_deadline),
             address = COALESCE($6, address),
             latitude = COALESCE($7, latitude),
             longitude = COALESCE($8, longitude),
             payment_provider_id = COALESCE($9, payment_provider_id),
             payment_identifier = COALESCE($10, payment_identifier),
             payment_requested_amount = COALESCE($11, payment_requested_amount),
             payment_per_person = $12
         WHERE event_id = $13
         RETURNING event_id, name_event, description, date_event, start_time, invitation_deadline, address, latitude, longitude, payment_provider_id, payment_identifier, payment_requested_amount, payment_per_person, owner_email",
    )
    .bind(payload.name_event.as_ref().map(|v| v.trim()).filter(|v| !v.is_empty()))
    .bind(payload.description.as_ref().map(|v| v.trim()).filter(|v| !v.is_empty()))
    .bind(payload.date_event)
    .bind(payload.start_time)
    .bind(payload.invitation_deadline)
    .bind(payload.address.as_ref().map(|v| v.trim()).filter(|v| !v.is_empty()))
    .bind(payload.latitude)
    .bind(payload.longitude)
    .bind(payment_provider_id)
    .bind(payment_identifier)
    .bind(payload.payment_requested_amount)
    .bind(payment_per_person)
    .bind(*event_id)
    .fetch_optional(&state.db)
    .await;

    match res {
        Ok(Some(event)) => {
            notify_event_members(state.get_ref(), &event, &updated_fields).await;
            HttpResponse::Ok().json(event)
        }
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
    if let Err(resp) = ensure_event_owner(&req, state.get_ref(), *event_id).await {
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

#[utoipa::path(
    post,
    path = "/events/{event_id}/share",
    tag = "events",
    responses(
        (status = 201, description = "Lien de partage généré", body = ShareTokenResponse),
        (status = 403, description = "Non autorisé", body = ErrorResponse),
        (status = 404, description = "Événement introuvable", body = ErrorResponse),
        (status = 500, description = "Erreur base de données", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Identifiant de l'événement")
    )
)]
#[post("/events/{event_id}/share")]
pub async fn create_share_link(
    state: web::Data<AppState>,
    req: HttpRequest,
    event_id: web::Path<i64>,
) -> impl Responder {
    let owner_email = match claims_email(&req, state.get_ref()) {
        Ok(email) => email,
        Err(resp) => return resp,
    };

    if let Err(resp) = ensure_event_owner(&req, state.get_ref(), *event_id).await {
        return resp;
    }

    // also ensures event exists
    if let Err(resp) = ensure_event_exists(&state.db, *event_id).await {
        return resp;
    }

    let token = Uuid::new_v4();

    let res = sqlx::query(
        "INSERT INTO event_share_tokens (token, event_id, created_by_email) VALUES ($1, $2, $3)",
    )
    .bind(token)
    .bind(*event_id)
    .bind(&owner_email)
    .execute(&state.db)
    .await;

    match res {
        Ok(_) => {
            let token_str = token.to_string();
            HttpResponse::Created().json(ShareTokenResponse {
                token: token_str,
                event_id: *event_id,
            })
        }
        Err(_) => server_error(),
    }
}

#[utoipa::path(
    post,
    path = "/share/claim",
    tag = "events",
    request_body = ShareClaimPayload,
    responses(
        (status = 200, description = "Lien consommé et événement accessible", body = ShareClaimResponse),
        (status = 400, description = "Token manquant", body = ErrorResponse),
        (status = 401, description = "Authentification requise", body = ErrorResponse),
        (status = 404, description = "Lien inexistant", body = ErrorResponse),
        (status = 410, description = "Lien déjà consommé", body = ErrorResponse),
        (status = 500, description = "Erreur base de données", body = ErrorResponse)
    )
)]
#[post("/share/claim")]
pub async fn claim_share_link(
    state: web::Data<AppState>,
    req: HttpRequest,
    payload: web::Json<ShareClaimPayload>,
) -> impl Responder {
    let claims = match extract_claims_from_auth(&req, &state.jwt_secret) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let token = payload.token.trim();
    if token.is_empty() {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_token".into(),
            details: Some("Token manquant".into()),
        });
    }

    let parsed_token = match Uuid::parse_str(token) {
        Ok(t) => t,
        Err(_) => {
            return HttpResponse::BadRequest().json(ErrorResponse {
                error: "invalid_token".into(),
                details: Some("Format de token invalide".into()),
            });
        }
    };

    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(_) => return server_error(),
    };

    let token_row = match sqlx::query(
        "SELECT event_id, used_at FROM event_share_tokens WHERE token = $1 FOR UPDATE",
    )
    .bind(parsed_token)
    .fetch_optional(&mut *tx)
    .await
    {
        Ok(Some(row)) => row,
        Ok(None) => {
            return HttpResponse::NotFound().json(ErrorResponse {
                error: "token_not_found".into(),
                details: None,
            });
        }
        Err(_) => return server_error(),
    };

    let used_at: Option<chrono::DateTime<chrono::Utc>> = match token_row.try_get("used_at") {
        Ok(v) => v,
        Err(_) => return server_error(),
    };
    if used_at.is_some() {
        return HttpResponse::Gone().json(ErrorResponse {
            error: "token_used".into(),
            details: Some("Ce lien a déjà été utilisé".into()),
        });
    }
    let event_id: i64 = match token_row.try_get("event_id") {
        Ok(v) => v,
        Err(_) => return server_error(),
    };

    let event = sqlx::query_as::<_, Event>(
        "SELECT event_id, name_event, description, date_event, start_time, invitation_deadline, address, latitude, longitude, payment_provider_id, payment_identifier, payment_requested_amount, payment_per_person, owner_email
         FROM events
         WHERE event_id = $1",
    )
    .bind(event_id)
    .fetch_optional(&mut *tx)
    .await;

    let event = match event {
        Ok(Some(e)) => e,
        Ok(None) => {
            return HttpResponse::NotFound().json(ErrorResponse {
                error: "event_not_found".into(),
                details: None,
            });
        }
        Err(_) => return server_error(),
    };

    if let Some(limit) = event.invitation_deadline {
        if chrono::Utc::now().date_naive() > limit {
            return HttpResponse::Gone().json(ErrorResponse {
                error: "invitation_expired".into(),
                details: Some("La date limite pour répondre est dépassée".into()),
            });
        }
    }

    let user_id = match fetch_user_id(&state.db, &claims.sub).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    // Ensure an invitation exists, but let the user choose their response later.
    if let Err(_) = sqlx::query(
        "INSERT INTO invitations (event_id, user_id, status)
         VALUES ($1, $2, 'Waiting')
         ON CONFLICT (event_id, user_id) DO NOTHING",
    )
    .bind(event.event_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    {
        return server_error();
    }

    let update_res = sqlx::query(
        "UPDATE event_share_tokens SET used_at = NOW(), used_by_email = $1 WHERE token = $2",
    )
    .bind(&claims.sub)
    .bind(parsed_token)
    .execute(&mut *tx)
    .await;

    if let Err(_) = update_res {
        return server_error();
    }

    if tx.commit().await.is_err() {
        return server_error();
    }

    HttpResponse::Ok().json(ShareClaimResponse { event })
}

#[utoipa::path(
    get,
    path = "/events/{event_id}/items",
    tag = "events",
    responses(
        (status = 200, description = "Items configurés pour l'événement", body = [EventItemView]),
        (status = 404, description = "Événement introuvable", body = ErrorResponse),
        (status = 500, description = "Erreur base de données", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Identifiant de l'événement")
    )
)]
#[get("/events/{event_id}/items")]
pub async fn list_event_items(
    state: web::Data<AppState>,
    event_id: web::Path<i64>,
) -> impl Responder {
    if let Err(resp) = ensure_event_exists(&state.db, *event_id).await {
        return resp;
    }

    let result = sqlx::query_as::<_, EventItemView>(
        "SELECT ei.event_id,
                ei.item_id,
                it.type_id,
                it.type AS type_name,
                i.name_item,
                ei.max_quantity,
                ei.quantity AS reserved_quantity,
                i.unit_label,
                cu.email AS created_by_email
         FROM events_items ei
         JOIN items i ON i.item_id = ei.item_id
         JOIN item_types it ON it.type_id = i.type_id
         LEFT JOIN users cu ON cu.id = ei.created_by
         WHERE ei.event_id = $1
         ORDER BY it.type, i.name_item",
    )
    .bind(*event_id)
    .fetch_all(&state.db)
    .await;

    match result {
        Ok(items) => HttpResponse::Ok().json(items),
        Err(_) => server_error(),
    }
}

#[utoipa::path(
    get,
    path = "/events/{event_id}/items/contributions",
    tag = "events",
    responses(
        (status = 200, description = "Contributions des items de l'événement", body = [ItemContribution]),
        (status = 403, description = "Non autorisé", body = ErrorResponse),
        (status = 404, description = "Événement introuvable", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Identifiant de l'événement")
    )
)]
#[get("/events/{event_id}/items/contributions")]
pub async fn list_event_item_contributions(
    state: web::Data<AppState>,
    req: HttpRequest,
    event_id: web::Path<i64>,
) -> impl Responder {
    let _ = sqlx::query("ALTER TABLE users ADD COLUMN IF NOT EXISTS avatar_url TEXT;")
        .execute(&state.db)
        .await;
    if let Err(resp) = ensure_event_member(&req, state.get_ref(), *event_id).await {
        return resp;
    }

    let result = sqlx::query_as::<_, ItemContribution>(
        "SELECT ui.item_id,
                ui.quantity,
                u.email,
                u.handle,
                u.avatar_url
         FROM user_items ui
         JOIN users u ON u.id = ui.user_id
         WHERE ui.event_id = $1",
    )
    .bind(*event_id)
    .fetch_all(&state.db)
    .await;

    match result {
        Ok(list) => HttpResponse::Ok().json(list),
        Err(_) => server_error(),
    }
}

#[utoipa::path(
    post,
    path = "/events/{event_id}/items",
    tag = "events",
    request_body = EventItemAttachPayload,
    responses(
        (status = 200, description = "Item attaché à l'événement", body = EventItemView),
        (status = 400, description = "Payload invalide", body = ErrorResponse),
        (status = 403, description = "Non autorisé", body = ErrorResponse),
        (status = 404, description = "Événement ou item introuvable", body = ErrorResponse),
        (status = 500, description = "Erreur base de données", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Identifiant de l'événement")
    )
)]
#[post("/events/{event_id}/items")]
pub async fn attach_event_item(
    state: web::Data<AppState>,
    req: HttpRequest,
    event_id: web::Path<i64>,
    payload: web::Json<EventItemAttachPayload>,
) -> impl Responder {
    if let Err(resp) = ensure_event_owner(&req, state.get_ref(), *event_id).await {
        return resp;
    }

    let creator_email = match claims_email(&req, state.get_ref()) {
        Ok(email) => email,
        Err(resp) => return resp,
    };

    let payload = payload.into_inner();
    if payload.max_quantity <= 0 {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("max_quantity doit être supérieur à 0".into()),
        });
    }

    if let Err(resp) = ensure_event_exists(&state.db, *event_id).await {
        return resp;
    }
    if let Err(resp) = ensure_item_exists(&state.db, payload.item_id).await {
        return resp;
    }

    let creator_id = match fetch_user_id(&state.db, &creator_email).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let res = sqlx::query(
        "INSERT INTO events_items (event_id, item_id, max_quantity, quantity, created_by)
         VALUES ($1, $2, $3, 0, $4)
         ON CONFLICT (event_id, item_id)
         DO UPDATE SET max_quantity = EXCLUDED.max_quantity
         RETURNING event_id, item_id",
    )
    .bind(*event_id)
    .bind(payload.item_id)
    .bind(payload.max_quantity)
    .bind(creator_id)
    .fetch_one(&state.db)
    .await;

    match res {
        Ok(row) => {
            let ev: i64 = row.get("event_id");
            let item: i64 = row.get("item_id");
            match fetch_event_item_view(&state.db, ev, item).await {
                Ok(view) => {
                    publish_event(
                        &state.redis_client,
                        ev,
                        &json!({"type": "items_changed", "event_id": ev, "item_id": item}),
                    )
                    .await;
                    HttpResponse::Ok().json(view)
                }
                Err(resp) => resp,
            }
        }
        Err(Error::Database(db_err)) if db_err.code().as_deref() == Some("23503") => {
            HttpResponse::BadRequest().json(ErrorResponse {
                error: "unknown_reference".into(),
                details: None,
            })
        }
        Err(_) => server_error(),
    }
}

#[utoipa::path(
    post,
    path = "/events/{event_id}/items/custom",
    tag = "events",
    request_body = EventCustomItemPayload,
    responses(
        (status = 200, description = "Item personnalisé ajouté ou mis à jour", body = EventItemView),
        (status = 400, description = "Payload invalide", body = ErrorResponse),
        (status = 403, description = "Non autorisé", body = ErrorResponse),
        (status = 404, description = "Événement introuvable", body = ErrorResponse),
        (status = 500, description = "Erreur base de données", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Identifiant de l'événement")
    )
)]
#[post("/events/{event_id}/items/custom")]
pub async fn create_custom_event_item(
    state: web::Data<AppState>,
    req: HttpRequest,
    event_id: web::Path<i64>,
    payload: web::Json<EventCustomItemPayload>,
) -> impl Responder {
    if let Err(resp) = ensure_event_member(&req, state.get_ref(), *event_id).await {
        return resp;
    }

    let creator_email = match claims_email(&req, state.get_ref()) {
        Ok(email) => email,
        Err(resp) => return resp,
    };

    let creator_id = match fetch_user_id(&state.db, &creator_email).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let payload = payload.into_inner();
    let name_trimmed = payload.name_item.trim().to_string();
    if name_trimmed.is_empty() || payload.max_quantity <= 0 {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("Nom non vide et quantité > 0 requis".into()),
        });
    }

    let unit_label = payload
        .unit_label
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "pièce".to_string());

    if let Err(resp) = ensure_event_exists(&state.db, *event_id).await {
        return resp;
    }

    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(_) => return server_error(),
    };

    let normalized_name = name_trimmed.to_lowercase();

    let existing_event_item = sqlx::query_scalar::<_, i64>(
        "SELECT ei.item_id
         FROM events_items ei
         JOIN items i ON i.item_id = ei.item_id
         WHERE ei.event_id = $1
           AND lower(i.name_item) = $2
         FOR UPDATE",
    )
    .bind(*event_id)
    .bind(&normalized_name)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|_| server_error());

    let existing_event_item = match existing_event_item {
        Ok(value) => value,
        Err(resp) => {
            let _ = tx.rollback().await;
            return resp;
        }
    };

    let item_id = if let Some(item_id) = existing_event_item {
        item_id
    } else {
        // Always create a fresh item when not already attached to this event to avoid leaking a previous type.
        let default_type_id = match sqlx::query_scalar::<_, i64>(
            "SELECT type_id FROM item_types WHERE type = 'Autres' LIMIT 1",
        )
        .fetch_optional(&mut *tx)
        .await
        {
            Ok(Some(id)) => id,
            Ok(None) => {
                match sqlx::query_scalar::<_, i64>(
                    "INSERT INTO item_types (type)
                     VALUES ('Autres')
                     ON CONFLICT (type) DO UPDATE SET type = EXCLUDED.type
                     RETURNING type_id",
                )
                .fetch_one(&mut *tx)
                .await
                {
                    Ok(id) => id,
                    Err(_) => {
                        let _ = tx.rollback().await;
                        return server_error();
                    }
                }
            }
            Err(_) => {
                let _ = tx.rollback().await;
                return server_error();
            }
        };

        match sqlx::query_scalar::<_, i64>(
            "INSERT INTO items (type_id, name_item, max_quantity, unit_label)
             VALUES ($1, $2, $3, $4)
             RETURNING item_id",
        )
        .bind(default_type_id)
        .bind(name_trimmed.as_str())
        .bind(payload.max_quantity)
        .bind(unit_label.as_str())
        .fetch_one(&mut *tx)
        .await
        {
            Ok(id) => id,
            Err(_) => {
                let _ = tx.rollback().await;
                return server_error();
            }
        }
    };

    let insert_res = sqlx::query(
        "INSERT INTO events_items (event_id, item_id, max_quantity, quantity, created_by)
         VALUES ($1, $2, $3, 0, $4)
         ON CONFLICT (event_id, item_id)
         DO UPDATE SET max_quantity = events_items.max_quantity + EXCLUDED.max_quantity
         RETURNING event_id, item_id",
    )
    .bind(*event_id)
    .bind(item_id)
    .bind(payload.max_quantity)
    .bind(creator_id)
    .fetch_one(&mut *tx)
    .await;

    let (ev_id, item_id) = match insert_res {
        Ok(row) => (row.get::<i64, _>("event_id"), row.get::<i64, _>("item_id")),
        Err(_) => {
            let _ = tx.rollback().await;
            return server_error();
        }
    };

    if let Err(_) = tx.commit().await {
        return server_error();
    }

    match fetch_event_item_view(&state.db, ev_id, item_id).await {
        Ok(view) => {
            publish_event(
                &state.redis_client,
                ev_id,
                &json!({"type": "items_changed", "event_id": ev_id, "item_id": item_id}),
            )
            .await;
            HttpResponse::Ok().json(view)
        }
        Err(resp) => resp,
    }
}

#[utoipa::path(
    post,
    path = "/events/{event_id}/items/{item_id}/reserve",
    tag = "events",
    request_body = EventItemReservationPayload,
    responses(
        (status = 200, description = "Quantité réservée", body = EventItemView),
        (status = 400, description = "Quantité invalide ou dépassement du maximum", body = ErrorResponse),
        (status = 401, description = "Authentification requise", body = ErrorResponse),
        (status = 404, description = "Événement, item ou utilisateur introuvable", body = ErrorResponse),
        (status = 500, description = "Erreur base de données", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Identifiant de l'événement"),
        ("item_id" = i64, Path, description = "Identifiant de l'item référencé")
    )
)]
#[post("/events/{event_id}/items/{item_id}/reserve")]
pub async fn reserve_event_item(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<(i64, i64)>,
    payload: web::Json<EventItemReservationPayload>,
) -> impl Responder {
    let claims = match extract_claims_from_auth(&req, &state.jwt_secret) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let payload = payload.into_inner();
    if payload.quantity < 0 {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("La quantité doit être positive".into()),
        });
    }

    let user_id = match fetch_user_id(&state.db, &claims.sub).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let (event_id, item_id) = path.into_inner();

    if let Err(resp) = ensure_event_exists(&state.db, event_id).await {
        return resp;
    }
    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(_) => return server_error(),
    };

    let event_item = match sqlx::query(
        "SELECT max_quantity, quantity FROM events_items WHERE event_id = $1 AND item_id = $2 FOR UPDATE",
    )
    .bind(event_id)
    .bind(item_id)
    .fetch_optional(&mut *tx)
    .await
    {
        Ok(Some(row)) => (
            row.get::<i32, _>("max_quantity"),
            row.get::<i32, _>("quantity"),
        ),
        Ok(None) => {
            let _ = tx.rollback().await;
            return HttpResponse::NotFound().json(ErrorResponse {
                error: "event_item_not_found".into(),
                details: None,
            });
        }
        Err(_) => {
            let _ = tx.rollback().await;
            return server_error();
        }
    };
    let (max_quantity, current_quantity) = event_item;

    let existing_user_qty = match sqlx::query_scalar::<_, i32>(
        "SELECT quantity FROM user_items WHERE user_id = $1 AND event_id = $2 AND item_id = $3 FOR UPDATE",
    )
    .bind(user_id)
    .bind(event_id)
    .bind(item_id)
    .fetch_optional(&mut *tx)
    .await
    {
        Ok(value) => value.unwrap_or(0),
        Err(_) => {
            let _ = tx.rollback().await;
            return server_error();
        }
    };

    let requested = payload.quantity;
    let new_total = current_quantity - existing_user_qty + requested;
    if new_total < 0 || new_total > max_quantity {
        let _ = tx.rollback().await;
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_quantity".into(),
            details: Some("La quantité dépasse la limite disponible pour cet item".into()),
        });
    }

    let result = if requested == 0 {
        sqlx::query("DELETE FROM user_items WHERE user_id = $1 AND event_id = $2 AND item_id = $3")
            .bind(user_id)
            .bind(event_id)
            .bind(item_id)
            .execute(&mut *tx)
            .await
    } else {
        sqlx::query(
            "INSERT INTO user_items (user_id, event_id, item_id, quantity)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (user_id, event_id, item_id)
             DO UPDATE SET quantity = EXCLUDED.quantity",
        )
        .bind(user_id)
        .bind(event_id)
        .bind(item_id)
        .bind(requested)
        .execute(&mut *tx)
        .await
    };
    if result.is_err() {
        let _ = tx.rollback().await;
        return server_error();
    }

    if let Err(_) =
        sqlx::query("UPDATE events_items SET quantity = $1 WHERE event_id = $2 AND item_id = $3")
            .bind(new_total)
            .bind(event_id)
            .bind(item_id)
            .execute(&mut *tx)
            .await
    {
        let _ = tx.rollback().await;
        return server_error();
    }

    if let Err(_) = tx.commit().await {
        return server_error();
    }

    match fetch_event_item_view(&state.db, event_id, item_id).await {
        Ok(view) => {
            publish_event(
                &state.redis_client,
                event_id,
                &json!({"type": "items_changed", "event_id": event_id, "item_id": item_id}),
            )
            .await;
            return HttpResponse::Ok().json(view);
        }
        Err(resp) => resp,
    }
}

#[utoipa::path(
    delete,
    path = "/events/{event_id}/items/{item_id}",
    tag = "events",
    responses(
        (status = 200, description = "Item supprimé", body = StatusResponse),
        (status = 400, description = "Suppression impossible", body = ErrorResponse),
        (status = 403, description = "Non autorisé", body = ErrorResponse),
        (status = 404, description = "Item introuvable", body = ErrorResponse),
        (status = 500, description = "Erreur base de données", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Identifiant de l'événement"),
        ("item_id" = i64, Path, description = "Identifiant de l'item")
    )
)]
#[delete("/events/{event_id}/items/{item_id}")]
pub async fn delete_event_item(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<(i64, i64)>,
) -> impl Responder {
    let claims = match extract_claims_from_auth(&req, &state.jwt_secret) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let user_id = match fetch_user_id(&state.db, &claims.sub).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let (event_id, item_id) = path.into_inner();

    if let Err(resp) = ensure_event_exists(&state.db, event_id).await {
        return resp;
    }

    let owner_email = match fetch_event_owner_email(&state.db, event_id).await {
        Ok(owner) => owner,
        Err(resp) => return resp,
    };

    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(_) => return server_error(),
    };

    let record = match sqlx::query(
        "SELECT created_by FROM events_items WHERE event_id = $1 AND item_id = $2 FOR UPDATE",
    )
    .bind(event_id)
    .bind(item_id)
    .fetch_optional(&mut *tx)
    .await
    {
        Ok(Some(row)) => row,
        Ok(None) => {
            let _ = tx.rollback().await;
            return HttpResponse::NotFound().json(ErrorResponse {
                error: "event_item_not_found".into(),
                details: None,
            });
        }
        Err(_) => {
            let _ = tx.rollback().await;
            return server_error();
        }
    };

    let created_by: Option<i64> = record.get("created_by");

    let is_owner = owner_email.eq_ignore_ascii_case(&claims.sub);
    let is_creator = created_by.is_some_and(|id| id == user_id);

    if !is_owner && !is_creator {
        let _ = tx.rollback().await;
        return HttpResponse::Forbidden().json(ErrorResponse {
            error: "forbidden".into(),
            details: Some("Seul le créateur ou l'organisateur peut supprimer cet item".into()),
        });
    }

    if let Err(_) = sqlx::query("DELETE FROM user_items WHERE event_id = $1 AND item_id = $2")
        .bind(event_id)
        .bind(item_id)
        .execute(&mut *tx)
        .await
    {
        let _ = tx.rollback().await;
        return server_error();
    }

    if let Err(_) = sqlx::query("DELETE FROM events_items WHERE event_id = $1 AND item_id = $2")
        .bind(event_id)
        .bind(item_id)
        .execute(&mut *tx)
        .await
    {
        let _ = tx.rollback().await;
        return server_error();
    }

    if let Err(_) = tx.commit().await {
        return server_error();
    }

    publish_event(
        &state.redis_client,
        event_id,
        &json!({"type": "items_changed", "event_id": event_id, "item_id": item_id}),
    )
    .await;

    HttpResponse::Ok().json(StatusResponse {
        status: "deleted".into(),
    })
}

#[utoipa::path(
    get,
    path = "/events/{event_id}/polls",
    tag = "events",
    responses(
        (status = 200, description = "Sondages associés à l'événement", body = [PollView]),
        (status = 403, description = "Non autorisé", body = ErrorResponse),
        (status = 404, description = "Événement introuvable", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Identifiant de l'événement")
    )
)]
#[get("/events/{event_id}/polls")]
pub async fn list_event_polls(
    state: web::Data<AppState>,
    req: HttpRequest,
    event_id: web::Path<i64>,
) -> impl Responder {
    let claims = match extract_claims_from_auth(&req, &state.jwt_secret) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    if let Err(resp) = ensure_event_member(&req, state.get_ref(), *event_id).await {
        return resp;
    }

    let user_id = match fetch_user_id(&state.db, &claims.sub).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    match fetch_poll_views(state.get_ref(), *event_id, user_id).await {
        Ok(list) => HttpResponse::Ok().json(list),
        Err(resp) => resp,
    }
}

#[utoipa::path(
    post,
    path = "/events/{event_id}/polls",
    tag = "events",
    request_body = EventPollCreatePayload,
    responses(
        (status = 201, description = "Sondage créé", body = PollView),
        (status = 400, description = "Payload invalide", body = ErrorResponse),
        (status = 403, description = "Non autorisé", body = ErrorResponse),
        (status = 404, description = "Événement introuvable", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Identifiant de l'événement")
    )
)]
#[post("/events/{event_id}/polls")]
pub async fn create_event_poll(
    state: web::Data<AppState>,
    req: HttpRequest,
    event_id: web::Path<i64>,
    payload: web::Json<EventPollCreatePayload>,
) -> impl Responder {
    let claims = match extract_claims_from_auth(&req, &state.jwt_secret) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    if let Err(resp) = ensure_event_owner(&req, state.get_ref(), *event_id).await {
        return resp;
    }

    let creator_id = match fetch_user_id(&state.db, &claims.sub).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let payload = payload.into_inner();
    let question = payload.question.trim().to_string();
    if question.is_empty() {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("La question du sondage est requise".into()),
        });
    }

    let mut seen = HashSet::new();
    let mut options: Vec<String> = Vec::new();
    for raw in payload.options.into_iter() {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let key = trimmed.to_lowercase();
        if seen.insert(key) {
            options.push(trimmed.to_string());
        }
    }

    if options.len() < 2 {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("Au moins deux options distinctes sont requises".into()),
        });
    }
    if options.len() > 12 {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("Maximum 12 options par sondage".into()),
        });
    }

    if payload.duration_minutes <= 0 {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("Durée d'expiration invalide".into()),
        });
    }

    if let Err(resp) = ensure_event_exists(&state.db, *event_id).await {
        return resp;
    }

    let duration_minutes = payload.duration_minutes.min(60 * 24 * 7);
    let expires_at = Utc::now() + Duration::minutes(duration_minutes as i64);
    let allow_multiple = payload.allow_multiple.unwrap_or(true);

    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(_) => return server_error(),
    };

    let poll_row = sqlx::query(
        "INSERT INTO event_polls (event_id, question, allow_multiple, expires_at, created_by)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING poll_id",
    )
    .bind(*event_id)
    .bind(&question)
    .bind(allow_multiple)
    .bind(expires_at)
    .bind(creator_id)
    .fetch_one(&mut *tx)
    .await;

    let poll_id: i64 = match poll_row {
        Ok(row) => row.get("poll_id"),
        Err(_) => {
            let _ = tx.rollback().await;
            return server_error();
        }
    };

    for (idx, label) in options.iter().enumerate() {
        if let Err(_) = sqlx::query(
            "INSERT INTO event_poll_options (poll_id, label, position) VALUES ($1, $2, $3)",
        )
        .bind(poll_id)
        .bind(label)
        .bind(idx as i32)
        .execute(&mut *tx)
        .await
        {
            let _ = tx.rollback().await;
            return server_error();
        }
    }

    if let Err(_) = tx.commit().await {
        return server_error();
    }

    let poll = match fetch_poll_views(state.get_ref(), *event_id, creator_id).await {
        Ok(list) => list.into_iter().find(|p| p.poll_id == poll_id),
        Err(resp) => return resp,
    };

    let poll = match poll {
        Some(p) => p,
        None => return server_error(),
    };

    publish_event(
        &state.redis_client,
        *event_id,
        &json!({"type": "polls_changed", "event_id": *event_id, "poll_id": poll_id}),
    )
    .await;

    if state.notifications.is_enabled() {
        if let Ok(members) = event_member_user_ids(&state.db, *event_id).await {
            let event_name = sqlx::query_scalar::<_, String>(
                "SELECT name_event FROM events WHERE event_id = $1",
            )
            .bind(*event_id)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| "Événement".into());
            let sender = if claims.handle.trim().is_empty() {
                claims.sub.as_str()
            } else {
                claims.handle.as_str()
            };
            let dedup = format!("poll_created:{poll_id}");
            notify_users(
                &state.notifications,
                &state.db,
                &members,
                "Nouveau sondage",
                &format!("{sender} a lancé un sondage dans {event_name}"),
                json!({
                    "type": "poll_created",
                    "event_id": *event_id,
                    "poll_id": poll_id,
                    "question": question
                }),
                Some(&dedup),
                Some(600),
            )
            .await;
        }
    }

    HttpResponse::Created().json(poll)
}

#[utoipa::path(
    post,
    path = "/events/{event_id}/polls/{poll_id}/vote",
    tag = "events",
    request_body = EventPollVotePayload,
    responses(
        (status = 200, description = "Vote enregistré", body = PollView),
        (status = 400, description = "Payload invalide", body = ErrorResponse),
        (status = 403, description = "Non autorisé", body = ErrorResponse),
        (status = 404, description = "Sondage introuvable", body = ErrorResponse),
        (status = 410, description = "Sondage expiré", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Identifiant de l'événement"),
        ("poll_id" = i64, Path, description = "Identifiant du sondage")
    )
)]
#[post("/events/{event_id}/polls/{poll_id}/vote")]
pub async fn vote_event_poll(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<(i64, i64)>,
    payload: web::Json<EventPollVotePayload>,
) -> impl Responder {
    let claims = match extract_claims_from_auth(&req, &state.jwt_secret) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let user_id = match fetch_user_id(&state.db, &claims.sub).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let (event_id, poll_id) = path.into_inner();

    if let Err(resp) = ensure_event_member(&req, state.get_ref(), event_id).await {
        return resp;
    }

    let poll_row = sqlx::query(
        "SELECT event_id, allow_multiple, expires_at FROM event_polls WHERE poll_id = $1",
    )
    .bind(poll_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|_| server_error());

    let poll_row = match poll_row {
        Ok(Some(row)) => row,
        Ok(None) => {
            return HttpResponse::NotFound().json(ErrorResponse {
                error: "poll_not_found".into(),
                details: None,
            });
        }
        Err(resp) => return resp,
    };

    let poll_event_id: i64 = match poll_row.try_get("event_id") {
        Ok(id) => id,
        Err(_) => return server_error(),
    };
    if poll_event_id != event_id {
        return HttpResponse::NotFound().json(ErrorResponse {
            error: "poll_not_found".into(),
            details: None,
        });
    }

    let expires_at: chrono::DateTime<chrono::Utc> = match poll_row.try_get("expires_at") {
        Ok(dt) => dt,
        Err(_) => return server_error(),
    };
    if expires_at < Utc::now() {
        return HttpResponse::Gone().json(ErrorResponse {
            error: "poll_expired".into(),
            details: Some("Ce sondage est expiré.".into()),
        });
    }

    let allow_multiple: bool = match poll_row.try_get("allow_multiple") {
        Ok(v) => v,
        Err(_) => return server_error(),
    };

    let mut option_ids: Vec<i64> = payload.option_ids.iter().copied().collect();
    option_ids.sort();
    option_ids.dedup();

    if !allow_multiple && option_ids.len() > 1 {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("Ce sondage n'autorise qu'un seul choix.".into()),
        });
    }

    let valid_option_ids =
        sqlx::query_scalar::<_, i64>("SELECT option_id FROM event_poll_options WHERE poll_id = $1")
            .bind(poll_id)
            .fetch_all(&state.db)
            .await;

    let valid_option_ids = match valid_option_ids {
        Ok(list) => list,
        Err(_) => return server_error(),
    };

    let valid_set: HashSet<i64> = valid_option_ids.iter().copied().collect();
    if !option_ids.iter().all(|id| valid_set.contains(id)) {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_option".into(),
            details: Some("Option inconnue pour ce sondage".into()),
        });
    }

    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(_) => return server_error(),
    };

    if let Err(_) = sqlx::query("DELETE FROM event_poll_votes WHERE poll_id = $1 AND user_id = $2")
        .bind(poll_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await
    {
        let _ = tx.rollback().await;
        return server_error();
    }

    if !option_ids.is_empty() {
        for opt_id in option_ids {
            if let Err(_) = sqlx::query(
                "INSERT INTO event_poll_votes (poll_id, option_id, user_id) VALUES ($1, $2, $3)",
            )
            .bind(poll_id)
            .bind(opt_id)
            .bind(user_id)
            .execute(&mut *tx)
            .await
            {
                let _ = tx.rollback().await;
                return server_error();
            }
        }
    }

    if let Err(_) = tx.commit().await {
        return server_error();
    }

    let poll = match fetch_poll_views(state.get_ref(), event_id, user_id).await {
        Ok(list) => list.into_iter().find(|p| p.poll_id == poll_id),
        Err(resp) => return resp,
    };

    let poll = match poll {
        Some(p) => p,
        None => return server_error(),
    };

    publish_event(
        &state.redis_client,
        event_id,
        &json!({"type": "polls_changed", "event_id": event_id, "poll_id": poll_id}),
    )
    .await;

    HttpResponse::Ok().json(poll)
}

#[utoipa::path(
    delete,
    path = "/events/{event_id}/polls/{poll_id}",
    tag = "events",
    responses(
        (status = 200, description = "Sondage supprimé", body = StatusResponse),
        (status = 403, description = "Non autorisé", body = ErrorResponse),
        (status = 404, description = "Sondage introuvable", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Identifiant de l'événement"),
        ("poll_id" = i64, Path, description = "Identifiant du sondage")
    )
)]
#[delete("/events/{event_id}/polls/{poll_id}")]
pub async fn delete_event_poll(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<(i64, i64)>,
) -> impl Responder {
    let claims = match extract_claims_from_auth(&req, &state.jwt_secret) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let (event_id, poll_id) = path.into_inner();

    if let Err(resp) = ensure_event_exists(&state.db, event_id).await {
        return resp;
    }

    let owner_email = match fetch_event_owner_email(&state.db, event_id).await {
        Ok(email) => email,
        Err(resp) => return resp,
    };

    let poll_row = sqlx::query("SELECT event_id, created_by FROM event_polls WHERE poll_id = $1")
        .bind(poll_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|_| server_error());

    let poll_row = match poll_row {
        Ok(Some(row)) => row,
        Ok(None) => {
            return HttpResponse::NotFound().json(ErrorResponse {
                error: "poll_not_found".into(),
                details: None,
            });
        }
        Err(resp) => return resp,
    };

    let poll_event_id: i64 = match poll_row.try_get("event_id") {
        Ok(id) => id,
        Err(_) => return server_error(),
    };
    if poll_event_id != event_id {
        return HttpResponse::NotFound().json(ErrorResponse {
            error: "poll_not_found".into(),
            details: None,
        });
    }

    let creator_id: Option<i64> = poll_row.try_get("created_by").ok();
    let user_id = match fetch_user_id(&state.db, &claims.sub).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let is_owner = owner_email.eq_ignore_ascii_case(&claims.sub);
    let is_creator = creator_id.is_some_and(|id| id == user_id);
    if !is_owner && !is_creator {
        return HttpResponse::Forbidden().json(ErrorResponse {
            error: "forbidden".into(),
            details: Some("Seul l'organisateur ou le créateur peut supprimer ce sondage".into()),
        });
    }

    let res = sqlx::query("DELETE FROM event_polls WHERE poll_id = $1")
        .bind(poll_id)
        .execute(&state.db)
        .await;

    match res {
        Ok(result) if result.rows_affected() == 0 => HttpResponse::NotFound().json(ErrorResponse {
            error: "poll_not_found".into(),
            details: None,
        }),
        Ok(_) => {
            publish_event(
                &state.redis_client,
                event_id,
                &json!({"type": "polls_changed", "event_id": event_id, "poll_id": poll_id, "deleted": true}),
            )
            .await;
            HttpResponse::Ok().json(StatusResponse {
                status: "deleted".into(),
            })
        }
        Err(_) => server_error(),
    }
}

async fn fetch_poll_views(
    state: &AppState,
    event_id: i64,
    user_id: i64,
) -> Result<Vec<PollView>, HttpResponse> {
    let poll_rows = sqlx::query(
        "SELECT p.poll_id,
                p.question,
                p.allow_multiple,
                p.expires_at,
                p.created_at,
                u.email AS created_by_email
         FROM event_polls p
         LEFT JOIN users u ON u.id = p.created_by
         WHERE p.event_id = $1
         ORDER BY p.created_at DESC",
    )
    .bind(event_id)
    .fetch_all(&state.db)
    .await
    .map_err(|_| server_error())?;

    if poll_rows.is_empty() {
        return Ok(Vec::new());
    }

    let poll_ids: Vec<i64> = poll_rows
        .iter()
        .filter_map(|row| row.try_get("poll_id").ok())
        .collect();
    let option_rows = sqlx::query(
        "SELECT option_id, poll_id, label, position
         FROM event_poll_options
         WHERE poll_id = ANY($1)
         ORDER BY position, option_id",
    )
    .bind(&poll_ids)
    .fetch_all(&state.db)
    .await
    .map_err(|_| server_error())?;

    let vote_rows = sqlx::query(
        "SELECT v.poll_id,
                v.option_id,
                v.user_id,
                u.email,
                u.handle,
                u.avatar_url
         FROM event_poll_votes v
         JOIN users u ON u.id = v.user_id
         WHERE v.poll_id = ANY($1)",
    )
    .bind(&poll_ids)
    .fetch_all(&state.db)
    .await
    .map_err(|_| server_error())?;

    let mut polls: Vec<PollView> = poll_rows
        .into_iter()
        .filter_map(|row| {
            let expires_at: chrono::DateTime<chrono::Utc> = row.try_get("expires_at").ok()?;
            let created_at: chrono::DateTime<chrono::Utc> = row.try_get("created_at").ok()?;
            let poll_id: i64 = row.try_get("poll_id").ok()?;
            let question: String = row.try_get("question").ok()?;
            let allow_multiple: bool = row.try_get("allow_multiple").ok()?;
            let created_by_email: Option<String> = row.try_get("created_by_email").ok();
            Some(PollView {
                poll_id,
                event_id,
                question,
                allow_multiple,
                expires_at,
                created_at,
                created_by_email,
                options: Vec::new(),
                my_votes: Vec::new(),
                total_votes: 0,
                has_expired: expires_at < Utc::now(),
            })
        })
        .collect();

    let mut options_by_poll: HashMap<i64, Vec<(PollOptionView, i32)>> = HashMap::new();
    for row in option_rows {
        let poll_id: i64 = row.get("poll_id");
        let option_id: i64 = row.get("option_id");
        let label: String = row.get("label");
        let position: i32 = row.get("position");
        options_by_poll.entry(poll_id).or_default().push((
            PollOptionView {
                option_id,
                label,
                vote_count: 0,
                voters: Vec::new(),
            },
            position,
        ));
    }

    for vote_row in vote_rows {
        let poll_id: i64 = vote_row.get("poll_id");
        let option_id: i64 = vote_row.get("option_id");
        let voter_id: i64 = vote_row.get("user_id");
        if let Some(poll) = polls.iter_mut().find(|p| p.poll_id == poll_id) {
            poll.total_votes += 1;
            if voter_id == user_id {
                poll.my_votes.push(option_id);
            }
        }
        if let Some(options) = options_by_poll.get_mut(&poll_id) {
            if let Some((opt, _)) = options
                .iter_mut()
                .find(|(opt, _)| opt.option_id == option_id)
            {
                opt.vote_count += 1;
                opt.voters.push(PollOptionVoter {
                    email: vote_row.get("email"),
                    handle: vote_row.get("handle"),
                    avatar_url: vote_row.get("avatar_url"),
                });
            }
        }
    }

    for poll in &mut polls {
        if let Some(mut opts) = options_by_poll.remove(&poll.poll_id) {
            opts.sort_by_key(|(_, pos)| *pos);
            poll.options = opts.into_iter().map(|(opt, _)| opt).collect();
        }
        poll.my_votes.sort_unstable();
        poll.my_votes.dedup();
    }

    Ok(polls)
}

fn clean_payment_value(value: Option<String>) -> Option<String> {
    value.and_then(|v| {
        let trimmed = v.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

async fn fetch_provider_validation(
    db: &PgPool,
    provider_id: i32,
) -> Result<(String, Regex), HttpResponse> {
    let provider = sqlx::query_as::<_, (String, String)>(
        "SELECT provider_name, validation_regex FROM payment_providers WHERE provider_id = $1",
    )
    .bind(provider_id)
    .fetch_optional(db)
    .await
    .map_err(|_| server_error())?;

    match provider {
        Some((name, pattern)) => {
            let regex = Regex::new(&pattern).map_err(|_| server_error())?;
            Ok((name, regex))
        }
        None => Err(HttpResponse::BadRequest().json(ErrorResponse {
            error: "unknown_payment_provider".into(),
            details: None,
        })),
    }
}

async fn normalize_payment_info(
    db: &PgPool,
    provider_id: Option<i32>,
    payment_link: Option<String>,
) -> Result<(Option<i32>, Option<String>), HttpResponse> {
    match (provider_id, clean_payment_value(payment_link)) {
        (None, None) => Ok((None, None)),
        (Some(id), Some(link)) => {
            let (name, regex) = fetch_provider_validation(db, id).await?;
            if !regex.is_match(&link) {
                return Err(HttpResponse::BadRequest().json(ErrorResponse {
                    error: "invalid_payment_link".into(),
                    details: Some(format!(
                        "Le lien ne correspond pas au format attendu pour {name}"
                    )),
                }));
            }
            Ok((Some(id), Some(link)))
        }
        (Some(id), None) => Ok((Some(id), None)),
        (None, Some(_)) => Err(HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("Choisissez d'abord un provider pour associer un lien".into()),
        })),
    }
}

async fn ensure_event_exists(db: &PgPool, event_id: i64) -> Result<(), HttpResponse> {
    let exists =
        sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM events WHERE event_id = $1)")
            .bind(event_id)
            .fetch_one(db)
            .await
            .map_err(|_| server_error())?;

    if exists {
        Ok(())
    } else {
        Err(HttpResponse::NotFound().json(ErrorResponse {
            error: "event_not_found".into(),
            details: None,
        }))
    }
}

async fn fetch_event_payment_info(
    db: &PgPool,
    event_id: i64,
) -> Result<
    (
        Option<i32>,
        Option<String>,
        bool,
        NaiveDate,
        Option<NaiveDate>,
    ),
    HttpResponse,
> {
    let row = sqlx::query_as::<_, (Option<i32>, Option<String>, bool, NaiveDate, Option<NaiveDate>)>(
        "SELECT payment_provider_id, payment_identifier, payment_per_person, date_event, invitation_deadline FROM events WHERE event_id = $1",
    )
    .bind(event_id)
    .fetch_optional(db)
    .await
    .map_err(|_| server_error())?;

    match row {
        Some(info) => Ok(info),
        None => Err(HttpResponse::NotFound().json(ErrorResponse {
            error: "event_not_found".into(),
            details: None,
        })),
    }
}

async fn ensure_item_exists(db: &PgPool, item_id: i64) -> Result<(), HttpResponse> {
    let exists =
        sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM items WHERE item_id = $1)")
            .bind(item_id)
            .fetch_one(db)
            .await
            .map_err(|_| server_error())?;

    if exists {
        Ok(())
    } else {
        Err(HttpResponse::NotFound().json(ErrorResponse {
            error: "item_not_found".into(),
            details: None,
        }))
    }
}

async fn fetch_event_item_view(
    db: &PgPool,
    event_id: i64,
    item_id: i64,
) -> Result<EventItemView, HttpResponse> {
    let record = sqlx::query_as::<_, EventItemView>(
        "SELECT ei.event_id,
                ei.item_id,
                it.type_id,
                it.type AS type_name,
                i.name_item,
                ei.max_quantity,
                ei.quantity AS reserved_quantity,
                i.unit_label,
                cu.email AS created_by_email
         FROM events_items ei
         JOIN items i ON i.item_id = ei.item_id
         JOIN item_types it ON it.type_id = i.type_id
         LEFT JOIN users cu ON cu.id = ei.created_by
         WHERE ei.event_id = $1 AND ei.item_id = $2",
    )
    .bind(event_id)
    .bind(item_id)
    .fetch_optional(db)
    .await
    .map_err(|_| server_error())?;

    match record {
        Some(view) => Ok(view),
        None => Err(HttpResponse::NotFound().json(ErrorResponse {
            error: "event_item_not_found".into(),
            details: None,
        })),
    }
}

async fn fetch_user_id(db: &PgPool, email: &str) -> Result<i64, HttpResponse> {
    let record =
        sqlx::query_scalar::<_, i64>("SELECT id FROM users WHERE lower(email) = lower($1)")
            .bind(email)
            .fetch_optional(db)
            .await
            .map_err(|_| server_error())?;

    match record {
        Some(id) => Ok(id),
        None => Err(HttpResponse::NotFound().json(ErrorResponse {
            error: "user_not_found".into(),
            details: None,
        })),
    }
}

fn server_error() -> HttpResponse {
    HttpResponse::InternalServerError().json(ErrorResponse {
        error: "db_error".into(),
        details: None,
    })
}
