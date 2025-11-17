use actix_web::{HttpRequest, HttpResponse, Responder, delete, get, patch, post, put, web};
use sqlx::{Error, PgPool, Row};

use crate::{
    auth::extract_claims_from_auth,
    models::{
        ErrorResponse, Event, EventItemAttachPayload, EventItemReservationPayload, EventItemView,
        EventPatchPayload, EventPayload, StatusResponse,
    },
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
         payment_provider_id, payment_identifier, owner_email
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

    let res = sqlx::query_as::<_, Event>(
        "INSERT INTO events (name_event, description, date_event, start_time, address, payment_provider_id, payment_identifier, owner_email)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         RETURNING event_id, name_event, description, date_event, start_time, address, payment_provider_id, payment_identifier, owner_email",
    )
    .bind(payload.name_event.trim())
    .bind(payload.description.trim())
    .bind(payload.date_event)
    .bind(payload.start_time)
    .bind(payload.address.trim())
    .bind(payload.payment_provider_id)
    .bind(payload.payment_identifier)
    .bind(owner_email)
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
    if let Err(resp) = ensure_event_owner(&req, state.get_ref(), *event_id).await {
        return resp;
    }

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

    let res = sqlx::query_as::<_, Event>(
        "UPDATE events
         SET name_event = $1, description = $2, date_event = $3, start_time = $4, 
             address = $5, payment_provider_id = $6, payment_identifier = $7
         WHERE event_id = $8
         RETURNING event_id, name_event, description, date_event, start_time, address, payment_provider_id, payment_identifier, owner_email",
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
    if let Err(resp) = ensure_event_owner(&req, state.get_ref(), *event_id).await {
        return resp;
    }

    let payload = payload.into_inner();
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
         RETURNING event_id, name_event, description, date_event, start_time, address, payment_provider_id, payment_identifier, owner_email",
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

    if let Err(resp) = ensure_event_has_all_items(&state.db, *event_id).await {
        return resp;
    }

    let result = sqlx::query_as::<_, EventItemView>(
        "SELECT ei.event_id,
                ei.item_id,
                it.type_id,
                it.type AS type_name,
                i.name_item,
                ei.max_quantity,
                ei.quantity AS reserved_quantity
         FROM events_items ei
         JOIN items i ON i.item_id = ei.item_id
         JOIN item_types it ON it.type_id = i.type_id
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

    let res = sqlx::query(
        "INSERT INTO events_items (event_id, item_id, max_quantity, quantity)
         VALUES ($1, $2, $3, 0)
         ON CONFLICT (event_id, item_id)
         DO UPDATE SET max_quantity = EXCLUDED.max_quantity
         RETURNING event_id, item_id",
    )
    .bind(*event_id)
    .bind(payload.item_id)
    .bind(payload.max_quantity)
    .fetch_one(&state.db)
    .await;

    match res {
        Ok(row) => {
            let ev: i64 = row.get("event_id");
            let item: i64 = row.get("item_id");
            match fetch_event_item_view(&state.db, ev, item).await {
                Ok(view) => HttpResponse::Ok().json(view),
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
    if let Err(resp) = ensure_event_has_all_items(&state.db, event_id).await {
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
        Ok(view) => HttpResponse::Ok().json(view),
        Err(resp) => resp,
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

async fn ensure_event_has_all_items(db: &PgPool, event_id: i64) -> Result<(), HttpResponse> {
    sqlx::query(
        "INSERT INTO events_items (event_id, item_id, max_quantity, quantity)
         SELECT $1, i.item_id, i.max_quantity, 0
         FROM items i
         WHERE NOT EXISTS (
             SELECT 1 FROM events_items ei
             WHERE ei.event_id = $1
               AND ei.item_id = i.item_id
         )",
    )
    .bind(event_id)
    .execute(db)
    .await
    .map(|_| ())
    .map_err(|_| server_error())
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
                ei.quantity AS reserved_quantity
         FROM events_items ei
         JOIN items i ON i.item_id = ei.item_id
         JOIN item_types it ON it.type_id = i.type_id
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
