use actix_web::{HttpRequest, HttpResponse, Responder, delete, get, patch, post, put, web};
use log::info;
use sqlx::Error;

use crate::{
    auth::extract_claims_from_auth,
    models::{ErrorResponse, Item, ItemPatchPayload, ItemPayload, StatusResponse},
    state::AppState,
};

fn ensure_admin(req: &HttpRequest, state: &AppState) -> Result<(), HttpResponse> {
    let claims = extract_claims_from_auth(req, &state.jwt_secret)?;
    if state.admin_emails.is_empty() {
        return Err(HttpResponse::Forbidden().json(ErrorResponse {
            error: "forbidden".into(),
            details: Some("admin privileges required".into()),
        }));
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

fn normalize_item_kind(value: &str) -> Option<String> {
    let normalized = value.trim().to_lowercase();
    match normalized.as_str() {
        "need" | "bring" => Some(normalized),
        _ => None,
    }
}

fn invalid_item_kind() -> HttpResponse {
    HttpResponse::BadRequest().json(ErrorResponse {
        error: "invalid_payload".into(),
        details: Some("item_kind doit être 'need' ou 'bring'".into()),
    })
}

#[utoipa::path(
    get,
    path = "/items",
    tag = "items",
    responses(
        (status = 200, description = "Liste des items", body = [Item]),
        (status = 500, description = "Erreur base de données", body = ErrorResponse)
    )
)]
#[get("/items")]
pub async fn list_items(state: web::Data<AppState>) -> impl Responder {
    let res = sqlx::query_as::<_, Item>(
        "SELECT item_id, type_id, name_item, max_quantity, unit_label, item_kind
         FROM items
         ORDER BY item_id",
    )
    .fetch_all(&state.db)
    .await;

    match res {
        Ok(items) => HttpResponse::Ok().json(items),
        Err(_) => HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        }),
    }
}

#[utoipa::path(
    post,
    path = "/items",
    tag = "items",
    request_body = ItemPayload,
    responses(
        (status = 201, description = "Item créé", body = Item),
        (status = 400, description = "Payload invalide ou type inconnu", body = ErrorResponse),
        (status = 403, description = "Non autorisé", body = ErrorResponse),
        (status = 500, description = "Erreur base de données", body = ErrorResponse)
    )
)]
#[post("/items")]
pub async fn create_item(
    state: web::Data<AppState>,
    req: HttpRequest,
    payload: web::Json<ItemPayload>,
) -> impl Responder {
    if let Err(resp) = ensure_admin(&req, state.get_ref()) {
        return resp;
    }

    let payload = payload.into_inner();
    let ItemPayload {
        type_id,
        name_item,
        max_quantity,
        unit_label,
        item_kind,
    } = payload;

    if name_item.trim().is_empty() || max_quantity <= 0 || unit_label.trim().is_empty() {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("name_item, unit_label non vides et max_quantity > 0".into()),
        });
    }

    let item_kind = match item_kind {
        Some(raw) => match normalize_item_kind(&raw) {
            Some(value) => value,
            None => return invalid_item_kind(),
        },
        None => "need".to_string(),
    };

    let res = sqlx::query_as::<_, Item>(
        "INSERT INTO items (type_id, name_item, max_quantity, unit_label, item_kind)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING item_id, type_id, name_item, max_quantity, unit_label, item_kind",
    )
    .bind(type_id)
    .bind(name_item.trim())
    .bind(max_quantity)
    .bind(unit_label.trim())
    .bind(item_kind)
    .fetch_one(&state.db)
    .await;

    match res {
        Ok(item) => HttpResponse::Created().json(item),
        Err(Error::Database(db_err)) if db_err.code().as_deref() == Some("23503") => {
            HttpResponse::BadRequest().json(ErrorResponse {
                error: "unknown_type_id".into(),
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
    path = "/items/{item_id}",
    tag = "items",
    request_body = ItemPayload,
    responses(
        (status = 200, description = "Item mis à jour", body = Item),
        (status = 400, description = "Payload invalide ou type inconnu", body = ErrorResponse),
        (status = 403, description = "Non autorisé", body = ErrorResponse),
        (status = 404, description = "Item introuvable", body = ErrorResponse),
        (status = 500, description = "Erreur base de données", body = ErrorResponse)
    ),
    params(
        ("item_id" = i64, Path, description = "Identifiant de l'item")
    )
)]
#[put("/items/{item_id}")]
pub async fn replace_item(
    state: web::Data<AppState>,
    req: HttpRequest,
    item_id: web::Path<i64>,
    payload: web::Json<ItemPayload>,
) -> impl Responder {
    if let Err(resp) = ensure_admin(&req, state.get_ref()) {
        return resp;
    }

    let payload = payload.into_inner();
    let ItemPayload {
        type_id,
        name_item,
        max_quantity,
        unit_label,
        item_kind,
    } = payload;

    if name_item.trim().is_empty() || max_quantity <= 0 || unit_label.trim().is_empty() {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("name_item, unit_label non vides et max_quantity > 0".into()),
        });
    }

    let item_kind = match item_kind {
        Some(raw) => match normalize_item_kind(&raw) {
            Some(value) => Some(value),
            None => return invalid_item_kind(),
        },
        None => None,
    };

    let previous_kind = if item_kind.is_some() {
        match sqlx::query_scalar::<_, String>("SELECT item_kind FROM items WHERE item_id = $1")
            .bind(*item_id)
            .fetch_optional(&state.db)
            .await
        {
            Ok(Some(value)) => Some(value),
            Ok(None) => {
                return HttpResponse::NotFound().json(ErrorResponse {
                    error: "item_not_found".into(),
                    details: None,
                });
            }
            Err(_) => {
                return HttpResponse::InternalServerError().json(ErrorResponse {
                    error: "db_error".into(),
                    details: None,
                });
            }
        }
    } else {
        None
    };

    let res = sqlx::query_as::<_, Item>(
        "UPDATE items
         SET type_id = $1,
             name_item = $2,
             max_quantity = $3,
             unit_label = $4,
             item_kind = COALESCE($5, item_kind)
         WHERE item_id = $6
         RETURNING item_id, type_id, name_item, max_quantity, unit_label, item_kind",
    )
    .bind(type_id)
    .bind(name_item.trim())
    .bind(max_quantity)
    .bind(unit_label.trim())
    .bind(item_kind.as_deref())
    .bind(*item_id)
    .fetch_optional(&state.db)
    .await;

    match res {
        Ok(Some(item)) => {
            if let Some(previous) = previous_kind
                && previous != item.item_kind
            {
                info!(
                    "item_kind_changed item_id={} from={} to={}",
                    item.item_id, previous, item.item_kind
                );
            }
            HttpResponse::Ok().json(item)
        }
        Ok(None) => HttpResponse::NotFound().json(ErrorResponse {
            error: "item_not_found".into(),
            details: None,
        }),
        Err(Error::Database(db_err)) if db_err.code().as_deref() == Some("23503") => {
            HttpResponse::BadRequest().json(ErrorResponse {
                error: "unknown_type_id".into(),
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
    path = "/items/{item_id}",
    tag = "items",
    request_body = ItemPatchPayload,
    responses(
        (status = 200, description = "Item modifié", body = Item),
        (status = 400, description = "Payload invalide ou type inconnu", body = ErrorResponse),
        (status = 403, description = "Non autorisé", body = ErrorResponse),
        (status = 404, description = "Item introuvable", body = ErrorResponse),
        (status = 500, description = "Erreur base de données", body = ErrorResponse)
    ),
    params(
        ("item_id" = i64, Path, description = "Identifiant de l'item")
    )
)]
#[patch("/items/{item_id}")]
pub async fn update_item(
    state: web::Data<AppState>,
    req: HttpRequest,
    item_id: web::Path<i64>,
    payload: web::Json<ItemPatchPayload>,
) -> impl Responder {
    if let Err(resp) = ensure_admin(&req, state.get_ref()) {
        return resp;
    }

    let payload = payload.into_inner();
    let ItemPatchPayload {
        type_id,
        name_item,
        max_quantity,
        unit_label,
        item_kind,
    } = payload;

    if name_item.as_ref().is_some_and(|v| v.trim().is_empty()) {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("name_item ne peut pas être vide".into()),
        });
    }
    if max_quantity.is_some_and(|qty| qty <= 0) {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("max_quantity doit être > 0".into()),
        });
    }
    if unit_label.as_ref().is_some_and(|v| v.trim().is_empty()) {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("unit_label ne peut pas être vide".into()),
        });
    }

    let item_kind = match item_kind {
        Some(raw) => match normalize_item_kind(&raw) {
            Some(value) => Some(value),
            None => return invalid_item_kind(),
        },
        None => None,
    };

    let previous_kind = if item_kind.is_some() {
        match sqlx::query_scalar::<_, String>("SELECT item_kind FROM items WHERE item_id = $1")
            .bind(*item_id)
            .fetch_optional(&state.db)
            .await
        {
            Ok(Some(value)) => Some(value),
            Ok(None) => {
                return HttpResponse::NotFound().json(ErrorResponse {
                    error: "item_not_found".into(),
                    details: None,
                });
            }
            Err(_) => {
                return HttpResponse::InternalServerError().json(ErrorResponse {
                    error: "db_error".into(),
                    details: None,
                });
            }
        }
    } else {
        None
    };

    let res = sqlx::query_as::<_, Item>(
        "UPDATE items
         SET type_id = COALESCE($1, type_id),
             name_item = COALESCE($2, name_item),
             max_quantity = COALESCE($3, max_quantity),
             unit_label = COALESCE($4, unit_label),
             item_kind = COALESCE($5, item_kind)
         WHERE item_id = $6
         RETURNING item_id, type_id, name_item, max_quantity, unit_label, item_kind",
    )
    .bind(type_id)
    .bind(
        name_item
            .as_ref()
            .map(|v| v.trim())
            .filter(|v| !v.is_empty()),
    )
    .bind(max_quantity)
    .bind(
        unit_label
            .as_ref()
            .map(|v| v.trim())
            .filter(|v| !v.is_empty()),
    )
    .bind(item_kind.as_deref())
    .bind(*item_id)
    .fetch_optional(&state.db)
    .await;

    match res {
        Ok(Some(item)) => {
            if let Some(previous) = previous_kind
                && previous != item.item_kind
            {
                info!(
                    "item_kind_changed item_id={} from={} to={}",
                    item.item_id, previous, item.item_kind
                );
            }
            HttpResponse::Ok().json(item)
        }
        Ok(None) => HttpResponse::NotFound().json(ErrorResponse {
            error: "item_not_found".into(),
            details: None,
        }),
        Err(Error::Database(db_err)) if db_err.code().as_deref() == Some("23503") => {
            HttpResponse::BadRequest().json(ErrorResponse {
                error: "unknown_type_id".into(),
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
    path = "/items/{item_id}",
    tag = "items",
    responses(
        (status = 200, description = "Item supprimé", body = StatusResponse),
        (status = 403, description = "Non autorisé", body = ErrorResponse),
        (status = 404, description = "Item introuvable", body = ErrorResponse),
        (status = 500, description = "Erreur base de données", body = ErrorResponse)
    ),
    params(
        ("item_id" = i64, Path, description = "Identifiant de l'item")
    )
)]
#[delete("/items/{item_id}")]
pub async fn delete_item(
    state: web::Data<AppState>,
    req: HttpRequest,
    item_id: web::Path<i64>,
) -> impl Responder {
    if let Err(resp) = ensure_admin(&req, state.get_ref()) {
        return resp;
    }

    let res = sqlx::query("DELETE FROM items WHERE item_id = $1")
        .bind(*item_id)
        .execute(&state.db)
        .await;

    match res {
        Ok(result) if result.rows_affected() == 0 => HttpResponse::NotFound().json(ErrorResponse {
            error: "item_not_found".into(),
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
