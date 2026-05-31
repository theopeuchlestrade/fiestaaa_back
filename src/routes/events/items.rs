use actix_web::{Responder, delete, get, post, web};
use sqlx::Error;
use std::collections::HashSet;

use super::*;

#[utoipa::path(
    get,
    path = "/events/{event_id}/items",
    tag = "events",
    responses(
        (status = 200, description = "Items configured for the event", body = [EventItemView]),
        (status = 400, description = "Invalid scope", body = ErrorResponse),
        (status = 401, description = "Authentication required", body = ErrorResponse),
        (status = 403, description = "Access restricted to event members", body = ErrorResponse),
        (status = 404, description = "Event not found", body = ErrorResponse),
        (status = 500, description = "Database error", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Event identifier"),
        ("scope" = Option<String>, Query, description = "Filter: all, mine, to_cover, completed")
    )
)]
#[get("/events/{event_id}/items")]
pub async fn list_event_items(
    state: web::Data<AppState>,
    req: HttpRequest,
    event_id: web::Path<i64>,
    query: web::Query<EventItemsQuery>,
) -> impl Responder {
    let scope = match query.scope.as_deref() {
        Some(raw) => match normalize_event_items_scope(raw) {
            Some(value) => value,
            None => return invalid_items_scope_response(),
        },
        None => EventItemsScope::All,
    };

    if let Err(resp) = ensure_event_exists(&state.db, *event_id).await {
        return resp;
    }
    if let Err(resp) = ensure_event_member(&req, state.get_ref(), *event_id).await {
        return resp;
    }

    let mine_user = if scope == EventItemsScope::Mine {
        let email = match claims_email(&req, state.get_ref()).await {
            Ok(value) => value,
            Err(resp) => return resp,
        };
        let user_id = match fetch_user_id(&state.db, &email).await {
            Ok(value) => value,
            Err(resp) => return resp,
        };
        Some((email, user_id))
    } else {
        None
    };

    let result = sqlx::query_as::<_, EventItemView>(
        "SELECT ei.event_id,
                ei.item_id,
                it.type_id,
                it.type AS type_name,
                i.name_item,
                ei.max_quantity,
                ei.quantity AS reserved_quantity,
                i.unit_label,
                i.item_kind,
                fiestaaa_decrypt_text(cu.email_ciphertext) AS created_by_email,
                cu.handle AS created_by_handle,
                cu.avatar_url AS created_by_avatar_url
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
        Ok(mut items) => {
            if let Some((mine_email, mine_user_id)) = mine_user {
                let contributed_ids = match sqlx::query_scalar::<_, i64>(
                    "SELECT item_id FROM user_items WHERE event_id = $1 AND user_id = $2",
                )
                .bind(*event_id)
                .bind(mine_user_id)
                .fetch_all(&state.db)
                .await
                {
                    Ok(ids) => ids.into_iter().collect::<HashSet<_>>(),
                    Err(_) => return server_error(),
                };

                items.retain(|item| {
                    item.created_by_email
                        .as_ref()
                        .is_some_and(|email| email.eq_ignore_ascii_case(&mine_email))
                        || contributed_ids.contains(&item.item_id)
                });
            }

            match scope {
                EventItemsScope::All | EventItemsScope::Mine => {}
                EventItemsScope::ToCover => {
                    items.retain(|item| {
                        item.item_kind == "need" && item.reserved_quantity < item.max_quantity
                    });
                }
                EventItemsScope::Completed => {
                    items.retain(|item| {
                        item.item_kind == "need" && item.reserved_quantity >= item.max_quantity
                    });
                }
            }

            HttpResponse::Ok().json(items)
        }
        Err(_) => server_error(),
    }
}

#[utoipa::path(
    get,
    path = "/events/{event_id}/items/contributions",
    tag = "events",
    responses(
        (status = 200, description = "Event item contributions", body = [ItemContribution]),
        (status = 403, description = "Unauthorized", body = ErrorResponse),
        (status = 404, description = "Event not found", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Event identifier")
    )
)]
#[get("/events/{event_id}/items/contributions")]
pub async fn list_event_item_contributions(
    state: web::Data<AppState>,
    req: HttpRequest,
    event_id: web::Path<i64>,
) -> impl Responder {
    if let Err(resp) = ensure_event_member(&req, state.get_ref(), *event_id).await {
        return resp;
    }

    let result = sqlx::query_as::<_, ItemContribution>(
        "SELECT ui.item_id,
                ui.quantity,
                fiestaaa_decrypt_text(u.email_ciphertext) AS email,
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
        (status = 200, description = "Item attached to the event", body = EventItemView),
        (status = 400, description = "Invalid payload", body = ErrorResponse),
        (status = 403, description = "Unauthorized", body = ErrorResponse),
        (status = 404, description = "Event or item not found", body = ErrorResponse),
        (status = 500, description = "Database error", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Event identifier")
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
    if let Err(resp) = ensure_event_writable(&state.db, *event_id).await {
        return resp;
    }

    let creator_email = match claims_email(&req, state.get_ref()).await {
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
                        &json!({
                            "type": event_types::EVENT_ITEMS_CHANGED,
                            "event_id": ev,
                            "item_id": item
                        }),
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
        (status = 200, description = "Custom item added or updated", body = EventItemView),
        (status = 400, description = "Invalid payload", body = ErrorResponse),
        (status = 403, description = "Unauthorized", body = ErrorResponse),
        (status = 404, description = "Event not found", body = ErrorResponse),
        (status = 500, description = "Database error", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Event identifier")
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
    if let Err(resp) = ensure_event_writable(&state.db, *event_id).await {
        return resp;
    }

    let creator_email = match claims_email(&req, state.get_ref()).await {
        Ok(email) => email,
        Err(resp) => return resp,
    };

    let creator_id = match fetch_user_id(&state.db, &creator_email).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let owner_id = match fetch_event_owner_id(&state.db, *event_id).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let is_owner = owner_id == creator_id;

    let payload = payload.into_inner();
    let EventCustomItemPayload {
        name_item,
        max_quantity,
        unit_label,
        item_kind,
    } = payload;

    let name_trimmed = name_item.trim().to_string();
    if name_trimmed.is_empty() || max_quantity <= 0 {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("Nom non vide et quantité > 0 requis".into()),
        });
    }

    let unit_label = unit_label
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "pièce".to_string());

    let item_kind = match item_kind {
        Some(raw) => match normalize_item_kind(&raw) {
            Some(value) => value,
            None => return invalid_item_kind_response(),
        },
        None => {
            if is_owner {
                "need".to_string()
            } else {
                "bring".to_string()
            }
        }
    };

    if item_kind == "need" && !is_owner {
        return HttpResponse::Forbidden().json(ErrorResponse {
            error: "forbidden".into(),
            details: Some("only the creator can add need items".into()),
        });
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
           AND i.item_kind = $3
         FOR UPDATE",
    )
    .bind(*event_id)
    .bind(&normalized_name)
    .bind(item_kind.as_str())
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
            "INSERT INTO items (type_id, name_item, max_quantity, unit_label, item_kind)
             VALUES ($1, $2, $3, $4, $5)
             RETURNING item_id",
        )
        .bind(default_type_id)
        .bind(name_trimmed.as_str())
        .bind(max_quantity)
        .bind(unit_label.as_str())
        .bind(item_kind.as_str())
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
    .bind(max_quantity)
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

    if (tx.commit().await).is_err() {
        return server_error();
    }

    match fetch_event_item_view(&state.db, ev_id, item_id).await {
        Ok(view) => {
            publish_event(
                &state.redis_client,
                ev_id,
                &json!({
                    "type": event_types::EVENT_ITEMS_CHANGED,
                    "event_id": ev_id,
                    "item_id": item_id
                }),
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
        (status = 200, description = "Quantity reserved", body = EventItemView),
        (status = 400, description = "Invalid quantity or maximum exceeded", body = ErrorResponse),
        (status = 401, description = "Authentication required", body = ErrorResponse),
        (status = 404, description = "Event, item, or user not found", body = ErrorResponse),
        (status = 500, description = "Database error", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Event identifier"),
        ("item_id" = i64, Path, description = "Referenced item identifier")
    )
)]
#[post("/events/{event_id}/items/{item_id}/reserve")]
pub async fn reserve_event_item(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<(i64, i64)>,
    payload: web::Json<EventItemReservationPayload>,
) -> impl Responder {
    let claims = match extract_active_claims_from_auth(&req, &state.db, &state.jwt_secret).await {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let (event_id, item_id) = path.into_inner();
    if let Err(resp) = ensure_event_member(&req, state.get_ref(), event_id).await {
        return resp;
    }

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

    if let Err(resp) = ensure_event_exists(&state.db, event_id).await {
        return resp;
    }
    if let Err(resp) = ensure_event_writable(&state.db, event_id).await {
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

    if (sqlx::query("UPDATE events_items SET quantity = $1 WHERE event_id = $2 AND item_id = $3")
        .bind(new_total)
        .bind(event_id)
        .bind(item_id)
        .execute(&mut *tx)
        .await)
        .is_err()
    {
        let _ = tx.rollback().await;
        return server_error();
    }

    if (tx.commit().await).is_err() {
        return server_error();
    }

    match fetch_event_item_view(&state.db, event_id, item_id).await {
        Ok(view) => {
            publish_event(
                &state.redis_client,
                event_id,
                &json!({
                    "type": event_types::EVENT_ITEMS_CHANGED,
                    "event_id": event_id,
                    "item_id": item_id
                }),
            )
            .await;
            HttpResponse::Ok().json(view)
        }
        Err(resp) => resp,
    }
}

#[utoipa::path(
    delete,
    path = "/events/{event_id}/items/{item_id}",
    tag = "events",
    responses(
        (status = 200, description = "Item deleted", body = StatusResponse),
        (status = 400, description = "Deletion not possible", body = ErrorResponse),
        (status = 403, description = "Unauthorized", body = ErrorResponse),
        (status = 404, description = "Item not found", body = ErrorResponse),
        (status = 500, description = "Database error", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Event identifier"),
        ("item_id" = i64, Path, description = "Item identifier")
    )
)]
#[delete("/events/{event_id}/items/{item_id}")]
pub async fn delete_event_item(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<(i64, i64)>,
) -> impl Responder {
    let claims = match extract_active_claims_from_auth(&req, &state.db, &state.jwt_secret).await {
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
    if let Err(resp) = ensure_event_writable(&state.db, event_id).await {
        return resp;
    }

    let owner_id = match fetch_event_owner_id(&state.db, event_id).await {
        Ok(id) => id,
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

    let is_owner = owner_id == user_id;
    let is_creator = created_by.is_some_and(|id| id == user_id);

    if !is_owner && !is_creator {
        let _ = tx.rollback().await;
        return HttpResponse::Forbidden().json(ErrorResponse {
            error: "forbidden".into(),
            details: Some("Seul le créateur ou l'organisateur peut supprimer cet item".into()),
        });
    }

    if (sqlx::query("DELETE FROM user_items WHERE event_id = $1 AND item_id = $2")
        .bind(event_id)
        .bind(item_id)
        .execute(&mut *tx)
        .await)
        .is_err()
    {
        let _ = tx.rollback().await;
        return server_error();
    }

    if (sqlx::query("DELETE FROM events_items WHERE event_id = $1 AND item_id = $2")
        .bind(event_id)
        .bind(item_id)
        .execute(&mut *tx)
        .await)
        .is_err()
    {
        let _ = tx.rollback().await;
        return server_error();
    }

    if (tx.commit().await).is_err() {
        return server_error();
    }

    publish_event(
        &state.redis_client,
        event_id,
        &json!({
            "type": event_types::EVENT_ITEMS_CHANGED,
            "event_id": event_id,
            "item_id": item_id
        }),
    )
    .await;

    HttpResponse::Ok().json(StatusResponse {
        status: "deleted".into(),
    })
}
