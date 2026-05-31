use actix_web::{Responder, delete, get, patch, post, put, web};
use log::info;
use sqlx::{AssertSqlSafe, Error};

use super::*;

#[utoipa::path(
    get,
    path = "/events/{event_id}",
    tag = "events",
    responses(
        (status = 200, description = "Event found", body = Event),
        (status = 401, description = "Authentication required", body = ErrorResponse),
        (status = 403, description = "Unauthorized access", body = ErrorResponse),
        (status = 404, description = "Event not found", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Event identifier")
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

    let sql = select_events_sql(
        "FROM events e
         JOIN users owner ON owner.id = e.owner_user_id
         WHERE e.event_id = $1",
    );

    match sqlx::query_as::<_, Event>(AssertSqlSafe(sql))
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
        (status = 200, description = "Event list", body = [Event]),
        (status = 401, description = "Authentication required", body = ErrorResponse),
        (status = 500, description = "Database error", body = ErrorResponse)
    )
)]
#[get("/events")]
pub async fn list_events(state: web::Data<AppState>, req: HttpRequest) -> impl Responder {
    let email = match claims_email(&req, state.get_ref()).await {
        Ok(e) => e,
        Err(resp) => return resp,
    };
    let user_id = match fetch_user_id(&state.db, &email).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    if let Err(resp) = expire_overdue_invitations(&state.db).await {
        return resp;
    }

    let sql = select_events_sql(
        "FROM events e
         JOIN users owner ON owner.id = e.owner_user_id
         WHERE e.owner_user_id = $1
            OR EXISTS (
                SELECT 1
                FROM invitations i
                WHERE i.event_id = e.event_id
                  AND i.user_id = $1
                  AND i.status NOT IN ('Declined', 'Expired')
            )
         ORDER BY e.date_event, e.start_time",
    );

    let res = sqlx::query_as::<_, Event>(AssertSqlSafe(sql))
        .bind(user_id)
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
        (status = 201, description = "Event created", body = Event),
        (status = 400, description = "Invalid payload", body = ErrorResponse),
        (status = 403, description = "Unauthorized", body = ErrorResponse),
        (status = 500, description = "Database error", body = ErrorResponse)
    )
)]
#[post("/events")]
pub async fn create_event(
    state: web::Data<AppState>,
    req: HttpRequest,
    payload: web::Json<EventPayload>,
) -> impl Responder {
    let owner_email = match claims_email(&req, state.get_ref()).await {
        Ok(email) => email,
        Err(resp) => return resp,
    };
    let owner_user_id = match fetch_user_id(&state.db, &owner_email).await {
        Ok(id) => id,
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
    if let Err(resp) = validate_event_schedule(
        payload.date_event,
        payload.start_time,
        payload.end_date,
        payload.end_time,
    ) {
        return resp;
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
    let (playlist_provider, playlist_url) = match normalize_playlist_payload(
        payload.playlist_provider,
        payload.playlist_url,
        false,
        false,
    ) {
        Ok(values) => values,
        Err(resp) => return resp,
    };
    let enabled_features = match resolve_enabled_features(
        payload.enabled_features,
        payment_provider_id,
        payment_identifier.as_deref(),
        playlist_provider.as_deref(),
        playlist_url.as_deref(),
    ) {
        Ok(values) => values,
        Err(resp) => return resp,
    };

    let sql = upsert_event_returning_sql(
        "INSERT INTO events (
            name_event,
            description,
            date_event,
            start_time,
            end_date,
            end_time,
            invitation_deadline,
            address_ciphertext,
            latitude_ciphertext,
            longitude_ciphertext,
            payment_provider_id,
            payment_identifier_ciphertext,
            payment_requested_amount,
            payment_per_person,
            playlist_url,
            playlist_provider,
            enabled_features,
            owner_user_id
         )
         VALUES (
            $1,
            $2,
            $3,
            $4,
            $5,
            $6,
            $7,
            fiestaaa_encrypt_text($8),
            CASE WHEN $9 IS NULL THEN NULL ELSE fiestaaa_encrypt_text($9::TEXT) END,
            CASE WHEN $10 IS NULL THEN NULL ELSE fiestaaa_encrypt_text($10::TEXT) END,
            $11,
            CASE WHEN $12 IS NULL THEN NULL ELSE fiestaaa_encrypt_text($12) END,
            $13,
            $14,
            $15,
            $16,
            $17,
            $18
         )
         RETURNING *",
    );

    let res = sqlx::query_as::<_, Event>(AssertSqlSafe(sql))
        .bind(payload.name_event.trim())
        .bind(payload.description.trim())
        .bind(payload.date_event)
        .bind(payload.start_time)
        .bind(payload.end_date)
        .bind(payload.end_time)
        .bind(payload.invitation_deadline)
        .bind(payload.address.trim())
        .bind(payload.latitude)
        .bind(payload.longitude)
        .bind(payment_provider_id)
        .bind(payment_identifier)
        .bind(payload.payment_requested_amount)
        .bind(payment_per_person)
        .bind(&playlist_url)
        .bind(&playlist_provider)
        .bind(&enabled_features)
        .bind(owner_user_id)
        .fetch_one(&state.db)
        .await;

    match res {
        Ok(event) => {
            publish_global_type(&state.redis_client, event_types::EVENTS_CHANGED).await;
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
        (status = 200, description = "Event updated", body = Event),
        (status = 400, description = "Invalid payload", body = ErrorResponse),
        (status = 403, description = "Unauthorized", body = ErrorResponse),
        (status = 404, description = "Event not found", body = ErrorResponse),
        (status = 500, description = "Database error", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Event identifier")
    )
)]
#[put("/events/{event_id}")]
pub async fn replace_event(
    state: web::Data<AppState>,
    req: HttpRequest,
    event_id: web::Path<i64>,
    payload: web::Json<EventPayload>,
) -> impl Responder {
    if let Err(resp) = ensure_event_owner(&req, state.get_ref(), *event_id).await {
        return resp;
    }
    if let Err(resp) = ensure_event_writable(&state.db, *event_id).await {
        return resp;
    }

    let payload = payload.into_inner();
    let enabled_features_requested = payload.enabled_features.is_some();
    let mut updated_fields = vec!["name", "description", "date", "time", "location"];
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
    if let Err(resp) = validate_event_schedule(
        payload.date_event,
        payload.start_time,
        payload.end_date,
        payload.end_time,
    ) {
        return resp;
    }
    if payload.end_date.is_some() || payload.end_time.is_some() {
        updated_fields.push("duration");
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
    let (playlist_provider, playlist_url) = match normalize_playlist_payload(
        payload.playlist_provider,
        payload.playlist_url,
        false,
        false,
    ) {
        Ok(values) => values,
        Err(resp) => return resp,
    };
    if playlist_provider.is_some() || playlist_url.is_some() {
        updated_fields.push("playlist");
    }
    let enabled_features = match resolve_enabled_features(
        payload.enabled_features,
        payment_provider_id,
        payment_identifier.as_deref(),
        playlist_provider.as_deref(),
        playlist_url.as_deref(),
    ) {
        Ok(values) => values,
        Err(resp) => return resp,
    };
    if enabled_features_requested {
        updated_fields.push("features");
    }

    let sql = upsert_event_returning_sql(
        "UPDATE events
         SET name_event = $1,
             description = $2,
             date_event = $3,
             start_time = $4,
             end_date = $5,
             end_time = $6,
             invitation_deadline = $7,
             address_ciphertext = fiestaaa_encrypt_text($8),
             latitude_ciphertext = CASE WHEN $9 IS NULL THEN NULL ELSE fiestaaa_encrypt_text($9::TEXT) END,
             longitude_ciphertext = CASE WHEN $10 IS NULL THEN NULL ELSE fiestaaa_encrypt_text($10::TEXT) END,
             payment_provider_id = $11,
             payment_identifier_ciphertext = CASE WHEN $12 IS NULL THEN NULL ELSE fiestaaa_encrypt_text($12) END,
             payment_requested_amount = $13,
             payment_per_person = $14,
             playlist_url = $15,
             playlist_provider = $16,
             enabled_features = $17
         WHERE event_id = $18
         RETURNING *",
    );

    let res = sqlx::query_as::<_, Event>(AssertSqlSafe(sql))
        .bind(payload.name_event.trim())
        .bind(payload.description.trim())
        .bind(payload.date_event)
        .bind(payload.start_time)
        .bind(payload.end_date)
        .bind(payload.end_time)
        .bind(payload.invitation_deadline)
        .bind(payload.address.trim())
        .bind(payload.latitude)
        .bind(payload.longitude)
        .bind(payment_provider_id)
        .bind(payment_identifier)
        .bind(payload.payment_requested_amount)
        .bind(payment_per_person)
        .bind(&playlist_url)
        .bind(&playlist_provider)
        .bind(&enabled_features)
        .bind(*event_id)
        .fetch_optional(&state.db)
        .await;

    match res {
        Ok(Some(event)) => {
            if playlist_provider.is_some() || playlist_url.is_some() {
                info!(
                    "playlist updated event_id={} provider={}",
                    event.event_id,
                    event
                        .playlist_provider
                        .clone()
                        .unwrap_or_else(|| "none".into())
                );
            }
            notify_event_members(state.get_ref(), &event, &updated_fields).await;
            publish_event(
                &state.redis_client,
                event.event_id,
                &json!({"type": event_types::EVENT_UPDATED, "event_id": event.event_id}),
            )
            .await;
            publish_global_type(&state.redis_client, event_types::EVENTS_CHANGED).await;
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
        (status = 200, description = "Event modified", body = Event),
        (status = 400, description = "Invalid payload", body = ErrorResponse),
        (status = 403, description = "Unauthorized", body = ErrorResponse),
        (status = 404, description = "Event not found", body = ErrorResponse),
        (status = 500, description = "Database error", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Event identifier")
    )
)]
#[patch("/events/{event_id}")]
pub async fn update_event(
    state: web::Data<AppState>,
    req: HttpRequest,
    event_id: web::Path<i64>,
    payload: web::Json<EventPatchPayload>,
) -> impl Responder {
    if let Err(resp) = ensure_event_owner(&req, state.get_ref(), *event_id).await {
        return resp;
    }
    if let Err(resp) = ensure_event_writable(&state.db, *event_id).await {
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
    if payload.end_date.is_some() || payload.end_time.is_some() {
        updated_fields.push("duration");
    }
    if payload.address.is_some() || payload.latitude.is_some() || payload.longitude.is_some() {
        updated_fields.push("location");
    }
    if payload.invitation_deadline.is_some() {
        updated_fields.push("deadline");
    }
    if payload.playlist_url.is_some() || payload.playlist_provider.is_some() {
        updated_fields.push("playlist");
    }
    if payload.enabled_features.is_some() {
        updated_fields.push("features");
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
        current_start_time,
        current_end_date,
        current_end_time,
        current_invitation_deadline,
        current_playlist_provider,
        current_playlist_url,
        current_enabled_features,
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
    let target_start_time = payload.start_time.unwrap_or(current_start_time);
    let target_end_date = payload.end_date.unwrap_or(current_end_date);
    let target_end_time = payload.end_time.unwrap_or(current_end_time);
    if let Err(resp) = validate_event_schedule(
        target_date_event,
        target_start_time,
        target_end_date,
        target_end_time,
    ) {
        return resp;
    }
    let invitation_deadline_update = payload.invitation_deadline;
    let target_deadline = invitation_deadline_update.unwrap_or(current_invitation_deadline);
    if let Err(resp) = validate_invitation_deadline(target_deadline, target_date_event) {
        return resp;
    }

    let playlist_provider_set = payload.playlist_provider.is_some();
    let playlist_url_set = payload.playlist_url.is_some();
    let provider_cleared = payload
        .playlist_provider
        .as_ref()
        .is_some_and(|value| value.is_none());
    let url_cleared = payload
        .playlist_url
        .as_ref()
        .is_some_and(|value| value.is_none());
    if provider_cleared ^ url_cleared {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_playlist".into(),
            details: Some(
                "playlist_url et playlist_provider doivent être renseignés ensemble".into(),
            ),
        });
    }
    let merged_playlist_provider = payload
        .playlist_provider
        .clone()
        .unwrap_or(current_playlist_provider);
    let merged_playlist_url = payload.playlist_url.clone().unwrap_or(current_playlist_url);
    let clearing_playlist = provider_cleared && url_cleared;
    let (playlist_provider, playlist_url) = if clearing_playlist {
        (None, None)
    } else {
        match normalize_playlist_payload(
            merged_playlist_provider,
            merged_playlist_url,
            false,
            false,
        ) {
            Ok(values) => values,
            Err(resp) => return resp,
        }
    };
    let enabled_features_input = if let Some(requested) = payload.enabled_features.clone() {
        requested
    } else {
        sync_optional_features(
            current_enabled_features,
            payment_provider_id,
            payment_identifier.as_deref(),
            playlist_provider.as_deref(),
            playlist_url.as_deref(),
        )
    };
    let enabled_features = match resolve_enabled_features(
        Some(enabled_features_input),
        payment_provider_id,
        payment_identifier.as_deref(),
        playlist_provider.as_deref(),
        playlist_url.as_deref(),
    ) {
        Ok(values) => values,
        Err(resp) => return resp,
    };

    let (invitation_deadline_set, invitation_deadline_value) = match invitation_deadline_update {
        Some(value) => (true, value),
        None => (false, None),
    };
    let (end_date_set, end_date_value) = match payload.end_date {
        Some(value) => (true, value),
        None => (false, None),
    };
    let (end_time_set, end_time_value) = match payload.end_time {
        Some(value) => (true, value),
        None => (false, None),
    };

    let sql = upsert_event_returning_sql(
        "UPDATE events
         SET name_event = COALESCE($1, name_event),
             description = COALESCE($2, description),
             date_event = COALESCE($3, date_event),
             start_time = COALESCE($4, start_time),
             end_date = CASE WHEN $5 THEN $6 ELSE end_date END,
             end_time = CASE WHEN $7 THEN $8 ELSE end_time END,
             invitation_deadline = CASE WHEN $9 THEN $10 ELSE invitation_deadline END,
             address_ciphertext = COALESCE(
                CASE WHEN $11 IS NULL THEN NULL ELSE fiestaaa_encrypt_text($11) END,
                address_ciphertext
             ),
             latitude_ciphertext = COALESCE(
                CASE WHEN $12 IS NULL THEN NULL ELSE fiestaaa_encrypt_text($12::TEXT) END,
                latitude_ciphertext
             ),
             longitude_ciphertext = COALESCE(
                CASE WHEN $13 IS NULL THEN NULL ELSE fiestaaa_encrypt_text($13::TEXT) END,
                longitude_ciphertext
             ),
             payment_provider_id = COALESCE($14, payment_provider_id),
             payment_identifier_ciphertext = COALESCE(
                CASE WHEN $15 IS NULL THEN NULL ELSE fiestaaa_encrypt_text($15) END,
                payment_identifier_ciphertext
             ),
             payment_requested_amount = COALESCE($16, payment_requested_amount),
             payment_per_person = $17,
             playlist_url = CASE WHEN $18 THEN $19 ELSE playlist_url END,
             playlist_provider = CASE WHEN $20 THEN $21 ELSE playlist_provider END,
             enabled_features = $22
         WHERE event_id = $23
         RETURNING *",
    );

    let res = sqlx::query_as::<_, Event>(AssertSqlSafe(sql))
        .bind(
            payload
                .name_event
                .as_ref()
                .map(|v| v.trim())
                .filter(|v| !v.is_empty()),
        )
        .bind(
            payload
                .description
                .as_ref()
                .map(|v| v.trim())
                .filter(|v| !v.is_empty()),
        )
        .bind(payload.date_event)
        .bind(payload.start_time)
        .bind(end_date_set)
        .bind(end_date_value)
        .bind(end_time_set)
        .bind(end_time_value)
        .bind(invitation_deadline_set)
        .bind(invitation_deadline_value)
        .bind(
            payload
                .address
                .as_ref()
                .map(|v| v.trim())
                .filter(|v| !v.is_empty()),
        )
        .bind(payload.latitude)
        .bind(payload.longitude)
        .bind(payment_provider_id)
        .bind(payment_identifier)
        .bind(payload.payment_requested_amount)
        .bind(payment_per_person)
        .bind(playlist_url_set)
        .bind(playlist_url)
        .bind(playlist_provider_set)
        .bind(playlist_provider)
        .bind(&enabled_features)
        .bind(*event_id)
        .fetch_optional(&state.db)
        .await;

    match res {
        Ok(Some(event)) => {
            if updated_fields.contains(&"playlist") {
                info!(
                    "playlist updated event_id={} provider={}",
                    event.event_id,
                    event
                        .playlist_provider
                        .clone()
                        .unwrap_or_else(|| "none".into())
                );
            }
            notify_event_members(state.get_ref(), &event, &updated_fields).await;
            publish_event(
                &state.redis_client,
                event.event_id,
                &json!({"type": event_types::EVENT_UPDATED, "event_id": event.event_id}),
            )
            .await;
            publish_global_type(&state.redis_client, event_types::EVENTS_CHANGED).await;
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
        (status = 200, description = "Event deleted", body = StatusResponse),
        (status = 403, description = "Unauthorized", body = ErrorResponse),
        (status = 404, description = "Event not found", body = ErrorResponse),
        (status = 500, description = "Database error", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Event identifier")
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
    if let Err(resp) = ensure_event_writable(&state.db, *event_id).await {
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
        Ok(_) => {
            publish_event(
                &state.redis_client,
                *event_id,
                &json!({"type": event_types::EVENT_DELETED, "event_id": *event_id}),
            )
            .await;
            publish_global_type(&state.redis_client, event_types::EVENTS_CHANGED).await;
            HttpResponse::Ok().json(StatusResponse {
                status: "deleted".into(),
            })
        }
        Err(_) => HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        }),
    }
}
