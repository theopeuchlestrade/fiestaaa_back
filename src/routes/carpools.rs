use actix_web::{HttpRequest, HttpResponse, Responder, delete, get, patch, post, web};
use chrono::Utc;
use log::{info, warn};
use serde::Deserialize;
use serde_json::json;
use sqlx::{AssertSqlSafe, PgPool, Row};
use std::{
    cmp::Reverse,
    collections::{HashMap, HashSet},
};

use crate::{
    auth::extract_active_claims_from_auth,
    models::{
        Carpool, CarpoolJoinResponse, CarpoolLeaveResponse, CarpoolPassenger, CarpoolPatchPayload,
        CarpoolPayload, CarpoolView, ErrorResponse, StatusResponse,
    },
    notifications::{NotificationRequest, notify_users},
    realtime::publish_event,
    routes::event_access::ensure_event_writable,
    state::AppState,
};

async fn claims_email(req: &HttpRequest, state: &AppState) -> Result<String, HttpResponse> {
    let claims = extract_active_claims_from_auth(req, &state.db, &state.jwt_secret).await?;
    Ok(claims.sub.to_lowercase())
}

async fn fetch_user_id(db: &PgPool, email: &str) -> Result<i64, HttpResponse> {
    sqlx::query_scalar::<_, i64>(
        "SELECT id FROM users WHERE fiestaaa_email_matches(email_lookup_hash, $1)",
    )
    .bind(email)
    .fetch_optional(db)
    .await
    .map_err(|_| {
        HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        })
    })?
    .ok_or_else(|| {
        HttpResponse::Unauthorized().json(ErrorResponse {
            error: "user_not_found".into(),
            details: None,
        })
    })
}

async fn ensure_event_member(
    req: &HttpRequest,
    state: &AppState,
    event_id: i64,
) -> Result<(), HttpResponse> {
    let requester = claims_email(req, state).await?;
    let requester_id = fetch_user_id(&state.db, &requester).await?;
    let owner_id = sqlx::query_scalar::<_, i64>(
        "SELECT owner_user_id
             FROM events
             WHERE event_id = $1 AND deleted_at IS NULL",
    )
    .bind(event_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|_| {
        HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        })
    })?
    .ok_or_else(|| {
        HttpResponse::NotFound().json(ErrorResponse {
            error: "event_not_found".into(),
            details: None,
        })
    })?;
    if owner_id == requester_id {
        return Ok(());
    }

    let is_member = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(
            SELECT 1
            FROM invitations i
            WHERE i.event_id = $1
              AND i.user_id = $2
              AND i.status = 'Accepted'
        )",
    )
    .bind(event_id)
    .bind(requester_id)
    .fetch_one(&state.db)
    .await
    .map_err(|_| {
        HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        })
    })?;

    if is_member {
        Ok(())
    } else {
        Err(HttpResponse::Forbidden().json(ErrorResponse {
            error: "forbidden".into(),
            details: Some("membership required".into()),
        }))
    }
}

async fn ensure_carpool_driver(
    req: &HttpRequest,
    state: &AppState,
    carpool_id: i64,
) -> Result<(), HttpResponse> {
    let requester_email = claims_email(req, state).await?;
    let requester_id = fetch_user_id(&state.db, &requester_email).await?;

    let driver_id =
        sqlx::query_scalar::<_, i64>("SELECT driver_id FROM carpools WHERE carpool_id = $1")
            .bind(carpool_id)
            .fetch_optional(&state.db)
            .await
            .map_err(|_| {
                HttpResponse::InternalServerError().json(ErrorResponse {
                    error: "db_error".into(),
                    details: None,
                })
            })?;

    match driver_id {
        Some(id) if id == requester_id => Ok(()),
        Some(_) => Err(HttpResponse::Forbidden().json(ErrorResponse {
            error: "forbidden".into(),
            details: Some("only the driver can perform this action".into()),
        })),
        None => Err(HttpResponse::NotFound().json(ErrorResponse {
            error: "carpool_not_found".into(),
            details: None,
        })),
    }
}

fn server_error() -> HttpResponse {
    HttpResponse::InternalServerError().json(ErrorResponse {
        error: "internal_error".into(),
        details: None,
    })
}

async fn lock_event_user(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    event_id: i64,
    user_id: i64,
) -> Result<(), HttpResponse> {
    let key = format!("carpool:{event_id}:{user_id}");
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(key)
        .execute(&mut **tx)
        .await
        .map(|_| ())
        .map_err(|_| server_error())
}

fn is_unique_violation(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .and_then(|db_error| db_error.code())
        .is_some_and(|code| code == "23505")
}

fn carpool_projection(alias: &str) -> String {
    format!(
        "{alias}.carpool_id,
         {alias}.event_id,
         {alias}.driver_id,
         fiestaaa_decrypt_text({alias}.origin_ciphertext) AS origin,
         CAST(fiestaaa_decrypt_text({alias}.origin_latitude_ciphertext) AS DOUBLE PRECISION) AS origin_latitude,
         CAST(fiestaaa_decrypt_text({alias}.origin_longitude_ciphertext) AS DOUBLE PRECISION) AS origin_longitude,
         {alias}.depart_at,
         {alias}.seats_total,
         {alias}.seats_taken,
         fiestaaa_decrypt_text({alias}.notes_ciphertext) AS notes,
         {alias}.created_at,
         {alias}.updated_at"
    )
}

fn select_carpools_sql(from_and_where: &str) -> String {
    format!("SELECT {} {from_and_where}", carpool_projection("c"))
}

async fn fetch_carpool(db: &PgPool, carpool_id: i64) -> Result<Carpool, HttpResponse> {
    let sql = select_carpools_sql("FROM carpools c WHERE c.carpool_id = $1");
    sqlx::query_as::<_, Carpool>(AssertSqlSafe(sql))
        .bind(carpool_id)
        .fetch_optional(db)
        .await
        .map_err(|_| server_error())?
        .ok_or_else(|| {
            HttpResponse::NotFound().json(ErrorResponse {
                error: "carpool_not_found".into(),
                details: None,
            })
        })
}

async fn fetch_carpool_views(
    db: &PgPool,
    event_id: i64,
    user_id: Option<i64>,
    sort_by: Option<String>,
) -> Result<Vec<CarpoolView>, HttpResponse> {
    let sql = select_carpools_sql("FROM carpools c WHERE c.event_id = $1");
    let carpools = sqlx::query_as::<_, Carpool>(AssertSqlSafe(sql))
        .bind(event_id)
        .fetch_all(db)
        .await
        .map_err(|_| {
            HttpResponse::InternalServerError().json(ErrorResponse {
                error: "db_error".into(),
                details: None,
            })
        })?;

    // Helper function to apply sorting to a list of carpools
    fn apply_sort(list: &mut [Carpool], sort_by: Option<&str>) {
        match sort_by {
            Some("departure_asc") => {
                list.sort_by_key(|carpool| carpool.depart_at);
            }
            Some("departure_desc") => {
                list.sort_by_key(|carpool| Reverse(carpool.depart_at));
            }
            Some("seats_asc") => {
                list.sort_by_key(|carpool| carpool.seats_total);
            }
            Some("seats_desc") => {
                list.sort_by_key(|carpool| Reverse(carpool.seats_total));
            }
            Some("available_seats_asc") => {
                list.sort_by(|a, b| {
                    (a.seats_total - a.seats_taken).cmp(&(b.seats_total - b.seats_taken))
                });
            }
            Some("available_seats_desc") => {
                list.sort_by(|a, b| {
                    (b.seats_total - b.seats_taken).cmp(&(a.seats_total - a.seats_taken))
                });
            }
            _ => {
                // Default sorting: by departure time ascending
                list.sort_by_key(|carpool| carpool.depart_at);
            }
        }
    }

    // Apply custom sorting based on user preferences and participation
    let mut carpools = carpools;

    if let Some(user_id) = user_id {
        let carpool_ids: Vec<i64> = carpools.iter().map(|carpool| carpool.carpool_id).collect();
        let passenger_carpool_ids: HashSet<i64> = if carpool_ids.is_empty() {
            HashSet::new()
        } else {
            sqlx::query_scalar::<_, i64>(
                "SELECT carpool_id
                 FROM carpool_passengers
                 WHERE user_id = $1 AND carpool_id = ANY($2)",
            )
            .bind(user_id)
            .bind(&carpool_ids)
            .fetch_all(db)
            .await
            .map_err(|_| server_error())?
            .into_iter()
            .collect()
        };

        // First, separate carpools where user is driver or passenger
        let mut user_driver_carpools = Vec::new();
        let mut user_passenger_carpools = Vec::new();
        let mut other_carpools = Vec::new();

        for carpool in carpools {
            // Check if user is driver
            if carpool.driver_id == user_id {
                user_driver_carpools.push(carpool);
            } else {
                if passenger_carpool_ids.contains(&carpool.carpool_id) {
                    user_passenger_carpools.push(carpool);
                } else {
                    other_carpools.push(carpool);
                }
            }
        }

        // Sort each category independently BEFORE combining
        apply_sort(&mut user_driver_carpools, sort_by.as_deref());
        apply_sort(&mut user_passenger_carpools, sort_by.as_deref());
        apply_sort(&mut other_carpools, sort_by.as_deref());

        // Recombine with priority: driver first, then passenger, then others
        // The sort order is preserved within each category
        carpools = Vec::new();
        carpools.extend(user_driver_carpools);
        carpools.extend(user_passenger_carpools);
        carpools.extend(other_carpools);
    } else {
        // No user context: just apply global sort
        apply_sort(&mut carpools, sort_by.as_deref());
    }

    let carpool_ids: Vec<i64> = carpools.iter().map(|carpool| carpool.carpool_id).collect();
    let driver_ids: Vec<i64> = carpools.iter().map(|carpool| carpool.driver_id).collect();
    let mut drivers = HashMap::<i64, (String, Option<String>)>::new();
    if !driver_ids.is_empty() {
        for row in sqlx::query(
            "SELECT id, handle, avatar_url
             FROM users
             WHERE id = ANY($1)",
        )
        .bind(&driver_ids)
        .fetch_all(db)
        .await
        .map_err(|_| server_error())?
        {
            drivers.insert(row.get("id"), (row.get("handle"), row.get("avatar_url")));
        }
    }

    let mut passengers_by_carpool = HashMap::<i64, Vec<CarpoolPassenger>>::new();
    if !carpool_ids.is_empty() {
        for row in sqlx::query(
            "SELECT cp.carpool_id, u.id AS user_id, u.handle, u.avatar_url, cp.joined_at
             FROM carpool_passengers cp
             JOIN users u ON u.id = cp.user_id
             WHERE cp.carpool_id = ANY($1)
             ORDER BY cp.carpool_id, cp.joined_at ASC",
        )
        .bind(&carpool_ids)
        .fetch_all(db)
        .await
        .map_err(|_| server_error())?
        {
            passengers_by_carpool
                .entry(row.get("carpool_id"))
                .or_default()
                .push(CarpoolPassenger {
                    user_id: row.get("user_id"),
                    handle: row.get("handle"),
                    avatar_url: row.get("avatar_url"),
                    joined_at: row.get("joined_at"),
                });
        }
    }

    let mut views = Vec::with_capacity(carpools.len());
    for carpool in carpools {
        let driver = drivers.get(&carpool.driver_id);
        let passengers = passengers_by_carpool
            .remove(&carpool.carpool_id)
            .unwrap_or_default();

        views.push(CarpoolView {
            carpool_id: carpool.carpool_id,
            event_id: carpool.event_id,
            driver_id: carpool.driver_id,
            driver_handle: driver.map(|(handle, _)| handle.clone()),
            driver_avatar_url: driver.and_then(|(_, avatar_url)| avatar_url.clone()),
            origin: carpool.origin,
            origin_latitude: carpool.origin_latitude,
            origin_longitude: carpool.origin_longitude,
            depart_at: carpool.depart_at,
            seats_total: carpool.seats_total,
            seats_taken: carpool.seats_taken,
            notes: carpool.notes,
            created_at: carpool.created_at,
            updated_at: carpool.updated_at,
            passengers,
        });
    }

    Ok(views)
}

#[utoipa::path(
    get,
    path = "/events/{event_id}/carpools",
    tag = "carpools",
    params(
        ("event_id" = i64, Path, description = "Event identifier"),
        ("sort" = Option<String>, Query, description = "Sort criterion: departure_asc, departure_desc, seats_asc, seats_desc, available_seats_asc, available_seats_desc")
    ),
    responses(
        (status = 200, description = "Carpool list", body = Vec<CarpoolView>),
        (status = 403, description = "Unauthorized", body = ErrorResponse),
        (status = 404, description = "Event not found", body = ErrorResponse)
    )
)]
#[get("/events/{event_id}/carpools")]
pub async fn list_event_carpools(
    state: web::Data<AppState>,
    req: HttpRequest,
    event_id: web::Path<i64>,
    query: web::Query<CarpoolListQuery>,
) -> impl Responder {
    if let Err(resp) = ensure_event_member(&req, state.get_ref(), *event_id).await {
        return resp;
    }
    if let Err(resp) = ensure_event_writable(&state.db, *event_id).await {
        return resp;
    }

    let user_id = match claims_email(&req, &state).await {
        Ok(email) => fetch_user_id(&state.db, &email).await.ok(),
        Err(_) => None,
    };

    match fetch_carpool_views(&state.db, *event_id, user_id, query.sort.clone()).await {
        Ok(carpools) => HttpResponse::Ok().json(carpools),
        Err(resp) => resp,
    }
}

#[derive(Debug, Deserialize)]
pub struct CarpoolListQuery {
    #[serde(default)]
    pub sort: Option<String>,
}

#[utoipa::path(
    post,
    path = "/events/{event_id}/carpools",
    tag = "carpools",
    request_body = CarpoolPayload,
    responses(
        (status = 201, description = "Carpool created", body = CarpoolView),
        (status = 400, description = "Invalid payload", body = ErrorResponse),
        (status = 403, description = "Unauthorized", body = ErrorResponse),
        (status = 404, description = "Event not found", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Event identifier")
    )
)]
#[post("/events/{event_id}/carpools")]
pub async fn create_carpool(
    state: web::Data<AppState>,
    req: HttpRequest,
    event_id: web::Path<i64>,
    payload: web::Json<CarpoolPayload>,
) -> impl Responder {
    let requester_email = match claims_email(&req, &state).await {
        Ok(email) => email,
        Err(resp) => return resp,
    };

    if let Err(resp) = ensure_event_member(&req, state.get_ref(), *event_id).await {
        return resp;
    }

    let user_id = match fetch_user_id(&state.db, &requester_email).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let payload = payload.into_inner();
    info!(
        "Received carpool create request for event {} by user {}",
        *event_id, user_id
    );

    let origin = payload.origin.trim().to_string();
    if origin.is_empty() {
        warn!("Origin is empty after trim");
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("L'origine est requise".into()),
        });
    }

    if payload.seats_total < 1 {
        warn!("Invalid seats_total: {}", payload.seats_total);
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("Au moins une place est requise".into()),
        });
    }

    let now = Utc::now();
    if payload.depart_at < now {
        warn!("Departure time is in past for event {}", *event_id);
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("La date de départ doit être dans le futur".into()),
        });
    }
    let notes = payload
        .notes
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty());

    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(_) => return server_error(),
    };
    if let Err(resp) = lock_event_user(&mut tx, *event_id, user_id).await {
        return resp;
    }

    let existing_carpool = match sqlx::query_scalar::<_, i64>(
        "SELECT carpool_id FROM carpool_passengers WHERE user_id = $1 AND carpool_id IN (SELECT carpool_id FROM carpools WHERE event_id = $2)
         UNION SELECT carpool_id FROM carpools WHERE driver_id = $1 AND event_id = $2
         LIMIT 1",
    )
    .bind(user_id)
    .bind(*event_id)
    .fetch_optional(&mut *tx)
    .await
    {
        Ok(opt) => opt,
        Err(e) => {
            warn!("Failed to check existing carpool: {}", e);
            return server_error();
        }
    };

    if existing_carpool.is_some() {
        let _ = tx.rollback().await;
        warn!(
            "User {} is already in a carpool for event {}",
            user_id, *event_id
        );
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "already_in_carpool".into(),
            details: Some("Vous êtes déjà dans un covoiturage pour cet événement".into()),
        });
    }

    let carpool_id = match sqlx::query_scalar::<_, i64>(
        r#"
        INSERT INTO carpools (
            event_id,
            driver_id,
            origin_ciphertext,
            origin_latitude_ciphertext,
            origin_longitude_ciphertext,
            depart_at,
            seats_total,
            seats_taken,
            notes_ciphertext
        )
        VALUES (
            $1,
            $2,
            fiestaaa_encrypt_text($3),
            CASE WHEN $4 IS NULL THEN NULL ELSE fiestaaa_encrypt_text($4::TEXT) END,
            CASE WHEN $5 IS NULL THEN NULL ELSE fiestaaa_encrypt_text($5::TEXT) END,
            $6,
            $7,
            0,
            CASE WHEN $8 IS NULL THEN NULL ELSE fiestaaa_encrypt_text($8) END
        )
        RETURNING carpool_id
        "#,
    )
    .bind(*event_id)
    .bind(user_id)
    .bind(&origin)
    .bind(payload.origin_latitude)
    .bind(payload.origin_longitude)
    .bind(payload.depart_at)
    .bind(payload.seats_total)
    .bind(&notes)
    .fetch_one(&mut *tx)
    .await
    {
        Ok(id) => id,
        Err(error) => {
            let _ = tx.rollback().await;
            if is_unique_violation(&error) {
                return HttpResponse::BadRequest().json(ErrorResponse {
                    error: "already_in_carpool".into(),
                    details: Some("Vous êtes déjà dans un covoiturage pour cet événement".into()),
                });
            }
            return server_error();
        }
    };
    if tx.commit().await.is_err() {
        return server_error();
    }

    info!(
        "Carpool {} created by user {} for event {}",
        carpool_id, user_id, event_id
    );

    publish_event(
        &state.redis_client,
        *event_id,
        &json!({"type": "carpool_created", "carpool_id": carpool_id}),
    )
    .await;

    match fetch_carpool_views(&state.db, *event_id, None, None).await {
        Ok(views) => {
            let view = views.iter().find(|v| v.carpool_id == carpool_id);
            match view {
                Some(v) => HttpResponse::Created().json(v),
                None => server_error(),
            }
        }
        Err(_) => server_error(),
    }
}

#[utoipa::path(
    patch,
    path = "/carpools/{carpool_id}",
    tag = "carpools",
    request_body = CarpoolPatchPayload,
    responses(
        (status = 200, description = "Carpool updated", body = CarpoolView),
        (status = 400, description = "Invalid payload", body = ErrorResponse),
        (status = 403, description = "Unauthorized", body = ErrorResponse),
        (status = 404, description = "Carpool not found", body = ErrorResponse)
    ),
    params(
        ("carpool_id" = i64, Path, description = "Carpool identifier")
    )
)]
#[patch("/carpools/{carpool_id}")]
pub async fn update_carpool(
    state: web::Data<AppState>,
    req: HttpRequest,
    carpool_id: web::Path<i64>,
    payload: web::Json<CarpoolPatchPayload>,
) -> impl Responder {
    if let Err(resp) = ensure_carpool_driver(&req, state.get_ref(), *carpool_id).await {
        return resp;
    }

    let payload = payload.into_inner();

    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(_) => return server_error(),
    };
    let sql = select_carpools_sql("FROM carpools c WHERE c.carpool_id = $1 FOR UPDATE");
    let current = match sqlx::query_as::<_, Carpool>(AssertSqlSafe(sql))
        .bind(*carpool_id)
        .fetch_optional(&mut *tx)
        .await
    {
        Ok(Some(carpool)) => carpool,
        Ok(None) => {
            return HttpResponse::NotFound().json(ErrorResponse {
                error: "carpool_not_found".into(),
                details: None,
            });
        }
        Err(_) => return server_error(),
    };
    if let Err(resp) = ensure_event_writable(&state.db, current.event_id).await {
        return resp;
    }

    let origin = payload.origin.unwrap_or(current.origin);
    if origin.trim().is_empty() {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("L'origine est requise".into()),
        });
    }

    let seats_total = payload.seats_total.unwrap_or(current.seats_total);
    if seats_total < 1 {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("Au moins une place est requise".into()),
        });
    }

    if seats_total < current.seats_taken {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some(
                "Impossible de réduire le nombre de places sous le nombre de passagers actuels"
                    .into(),
            ),
        });
    }

    let depart_at = payload.depart_at.unwrap_or(current.depart_at);
    if depart_at < Utc::now() {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("La date de départ doit être dans le futur".into()),
        });
    }

    let notes = match payload.notes {
        Some(notes) => Some(notes.trim().to_string()).filter(|n| !n.is_empty()),
        None => current.notes.clone(),
    };

    let origin_latitude = payload.origin_latitude.or(current.origin_latitude);
    let origin_longitude = payload.origin_longitude.or(current.origin_longitude);

    match sqlx::query(
        r#"
        UPDATE carpools
        SET origin_ciphertext = fiestaaa_encrypt_text($1),
            origin_latitude_ciphertext = CASE WHEN $2 IS NULL THEN NULL ELSE fiestaaa_encrypt_text($2::TEXT) END,
            origin_longitude_ciphertext = CASE WHEN $3 IS NULL THEN NULL ELSE fiestaaa_encrypt_text($3::TEXT) END,
            depart_at = $4,
            seats_total = $5,
            notes_ciphertext = CASE WHEN $6 IS NULL THEN NULL ELSE fiestaaa_encrypt_text($6) END,
            updated_at = now()
        WHERE carpool_id = $7
        "#,
    )
    .bind(origin)
    .bind(origin_latitude)
    .bind(origin_longitude)
    .bind(depart_at)
    .bind(seats_total)
    .bind(notes)
    .bind(*carpool_id)
    .execute(&mut *tx)
    .await
    {
        Ok(_) => (),
        Err(e) => {
            warn!("Failed to update carpool: {}", e);
            return server_error();
        }
    }
    if tx.commit().await.is_err() {
        return server_error();
    }

    info!("Carpool {} updated", carpool_id);

    publish_event(
        &state.redis_client,
        current.event_id,
        &json!({"type": "carpool_updated", "carpool_id": *carpool_id}),
    )
    .await;

    match fetch_carpool_views(&state.db, current.event_id, None, None).await {
        Ok(views) => {
            let view = views.iter().find(|v| v.carpool_id == *carpool_id);
            match view {
                Some(v) => HttpResponse::Ok().json(v),
                None => server_error(),
            }
        }
        Err(_) => server_error(),
    }
}

#[utoipa::path(
    delete,
    path = "/carpools/{carpool_id}",
    tag = "carpools",
    responses(
        (status = 200, description = "Carpool deleted", body = StatusResponse),
        (status = 403, description = "Unauthorized", body = ErrorResponse),
        (status = 404, description = "Carpool not found", body = ErrorResponse)
    ),
    params(
        ("carpool_id" = i64, Path, description = "Carpool identifier")
    )
)]
#[delete("/carpools/{carpool_id}")]
pub async fn delete_carpool(
    state: web::Data<AppState>,
    req: HttpRequest,
    carpool_id: web::Path<i64>,
) -> impl Responder {
    let current = match fetch_carpool(&state.db, *carpool_id).await {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    if let Err(resp) = ensure_carpool_driver(&req, state.get_ref(), *carpool_id).await {
        return resp;
    }
    if let Err(resp) = ensure_event_writable(&state.db, current.event_id).await {
        return resp;
    }

    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(_) => return server_error(),
    };

    let passenger_ids = match sqlx::query_scalar::<_, i64>(
        "SELECT user_id FROM carpool_passengers WHERE carpool_id = $1",
    )
    .bind(*carpool_id)
    .fetch_all(&mut *tx)
    .await
    {
        Ok(ids) => ids,
        Err(_) => return server_error(),
    };

    match sqlx::query("DELETE FROM carpools WHERE carpool_id = $1")
        .bind(*carpool_id)
        .execute(&mut *tx)
        .await
    {
        Ok(_) => (),
        Err(e) => {
            warn!("Failed to delete carpool: {}", e);
            return server_error();
        }
    }
    if tx.commit().await.is_err() {
        return server_error();
    }

    info!("Carpool {} deleted", carpool_id);

    publish_event(
        &state.redis_client,
        current.event_id,
        &json!({"type": "carpool_deleted", "carpool_id": *carpool_id}),
    )
    .await;

    if !passenger_ids.is_empty() && state.notifications.is_enabled() {
        let body = format!(
            "Le covoiturage au départ de {} a été annulé par le conducteur",
            current.origin
        );
        let dedup = format!("carpool_cancelled:{}", *carpool_id);
        notify_users(
            &state.notifications,
            &state.db,
            &passenger_ids,
            NotificationRequest {
                title: "Covoiturage annulé",
                body: body.as_str(),
                data: json!({
                    "type": "carpool_cancelled",
                    "event_id": current.event_id,
                    "carpool_id": *carpool_id
                }),
                dedup_base_key: Some(dedup.as_str()),
                dedup_ttl: Some(300),
            },
        )
        .await;
    }

    HttpResponse::Ok().json(StatusResponse {
        status: "deleted".into(),
    })
}

#[utoipa::path(
    post,
    path = "/carpools/{carpool_id}/join",
    tag = "carpools",
    responses(
        (status = 200, description = "Joined successfully", body = CarpoolJoinResponse),
        (status = 400, description = "No seats available", body = ErrorResponse),
        (status = 403, description = "Unauthorized", body = ErrorResponse),
        (status = 404, description = "Carpool not found", body = ErrorResponse),
        (status = 409, description = "Already joined", body = ErrorResponse)
    ),
    params(
        ("carpool_id" = i64, Path, description = "Carpool identifier")
    )
)]
#[post("/carpools/{carpool_id}/join")]
pub async fn join_carpool(
    state: web::Data<AppState>,
    req: HttpRequest,
    carpool_id: web::Path<i64>,
) -> impl Responder {
    let requester_email = match claims_email(&req, &state).await {
        Ok(email) => email,
        Err(resp) => return resp,
    };

    let user_id = match fetch_user_id(&state.db, &requester_email).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let carpool = match fetch_carpool(&state.db, *carpool_id).await {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    if let Err(resp) = ensure_event_member(&req, state.get_ref(), carpool.event_id).await {
        return resp;
    }
    if let Err(resp) = ensure_event_writable(&state.db, carpool.event_id).await {
        return resp;
    }

    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(_) => return server_error(),
    };
    if let Err(resp) = lock_event_user(&mut tx, carpool.event_id, user_id).await {
        return resp;
    }
    let sql = select_carpools_sql("FROM carpools c WHERE c.carpool_id = $1 FOR UPDATE");
    let carpool = match sqlx::query_as::<_, Carpool>(AssertSqlSafe(sql))
        .bind(*carpool_id)
        .fetch_optional(&mut *tx)
        .await
    {
        Ok(Some(carpool)) => carpool,
        Ok(None) => {
            return HttpResponse::NotFound().json(ErrorResponse {
                error: "carpool_not_found".into(),
                details: None,
            });
        }
        Err(_) => return server_error(),
    };

    if carpool.driver_id == user_id {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "cannot_join_own_carpool".into(),
            details: Some("Le conducteur ne peut pas rejoindre son propre covoiturage".into()),
        });
    }

    let is_driver_elsewhere = match sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM carpools WHERE driver_id = $1 AND carpool_id != $2 AND event_id = $3)",
    )
    .bind(user_id)
    .bind(*carpool_id)
    .bind(carpool.event_id)
    .fetch_one(&mut *tx)
    .await
    {
        Ok(is_driver) => is_driver,
        Err(e) => {
            warn!("Failed to check if user is driver of another carpool: {}", e);
            return server_error();
        }
    };

    if is_driver_elsewhere {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "cannot_join_another_carpool".into(),
            details: Some("Vous êtes déjà conducteur d'un autre covoiturage".into()),
        });
    }

    // Check if user is already a passenger in another carpool of the same event
    let is_passenger_elsewhere = match sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(
            SELECT 1 FROM carpool_passengers cp
            JOIN carpools c ON c.carpool_id = cp.carpool_id
            WHERE cp.user_id = $1 AND c.event_id = $2 AND cp.carpool_id != $3
        )",
    )
    .bind(user_id)
    .bind(carpool.event_id)
    .bind(*carpool_id)
    .fetch_one(&mut *tx)
    .await
    {
        Ok(is_passenger) => is_passenger,
        Err(e) => {
            warn!(
                "Failed to check if user is passenger in another carpool: {}",
                e
            );
            return server_error();
        }
    };

    if is_passenger_elsewhere {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "already_in_another_carpool".into(),
            details: Some(
                "Vous êtes déjà passager dans un autre covoiturage pour cet événement".into(),
            ),
        });
    }

    let already_joined = match sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM carpool_passengers WHERE carpool_id = $1 AND user_id = $2)",
    )
    .bind(*carpool_id)
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await
    {
        Ok(joined) => joined,
        Err(_) => return server_error(),
    };

    if already_joined {
        return HttpResponse::Conflict().json(ErrorResponse {
            error: "already_joined".into(),
            details: Some("Vous êtes déjà inscrit à ce covoiturage".into()),
        });
    }

    let new_seats_taken = match sqlx::query_scalar::<_, i32>(
        "UPDATE carpools
         SET seats_taken = seats_taken + 1, updated_at = NOW()
         WHERE carpool_id = $1 AND seats_taken < seats_total
         RETURNING seats_taken",
    )
    .bind(*carpool_id)
    .fetch_optional(&mut *tx)
    .await
    {
        Ok(Some(value)) => value,
        Ok(None) => {
            let _ = tx.rollback().await;
            return HttpResponse::BadRequest().json(ErrorResponse {
                error: "carpool_full".into(),
                details: Some("Plus de places disponibles".into()),
            });
        }
        Err(e) => {
            warn!("Failed to reserve carpool seat: {}", e);
            return server_error();
        }
    };

    match sqlx::query(
        "INSERT INTO carpool_passengers (carpool_id, event_id, user_id)
         VALUES ($1, $2, $3)",
    )
    .bind(*carpool_id)
    .bind(carpool.event_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    {
        Ok(_) => (),
        Err(e) => {
            let unique_violation = is_unique_violation(&e);
            let _ = tx.rollback().await;
            if unique_violation {
                return HttpResponse::Conflict().json(ErrorResponse {
                    error: "already_in_carpool".into(),
                    details: Some("Vous êtes déjà dans un covoiturage pour cet événement".into()),
                });
            }
            warn!("Failed to join carpool: {}", e);
            return server_error();
        }
    }
    if tx.commit().await.is_err() {
        return server_error();
    }

    info!("User {} joined carpool {}", user_id, carpool_id);

    publish_event(
        &state.redis_client,
        carpool.event_id,
        &json!({"type": "carpool_joined", "carpool_id": *carpool_id, "user_id": user_id}),
    )
    .await;

    if state.notifications.is_enabled() {
        let passenger_handle =
            match sqlx::query_scalar::<_, String>("SELECT handle FROM users WHERE id = $1")
                .bind(user_id)
                .fetch_optional(&state.db)
                .await
            {
                Ok(Some(handle)) => handle,
                _ => "Un utilisateur".to_string(),
            };
        let body = format!("{} a rejoint votre covoiturage", passenger_handle);
        let dedup = format!("carpool_join:{}:{}", *carpool_id, user_id);
        notify_users(
            &state.notifications,
            &state.db,
            &[carpool.driver_id],
            NotificationRequest {
                title: "Nouveau passager",
                body: body.as_str(),
                data: json!({
                    "type": "carpool_joined",
                    "event_id": carpool.event_id,
                    "carpool_id": *carpool_id
                }),
                dedup_base_key: Some(dedup.as_str()),
                dedup_ttl: Some(300),
            },
        )
        .await;
    }

    HttpResponse::Ok().json(CarpoolJoinResponse {
        success: true,
        seats_taken: new_seats_taken,
        seats_total: carpool.seats_total,
        message: "Inscription réussie".into(),
    })
}

#[utoipa::path(
    delete,
    path = "/carpools/{carpool_id}/join",
    tag = "carpools",
    responses(
        (status = 200, description = "Left successfully", body = CarpoolLeaveResponse),
        (status = 403, description = "Unauthorized", body = ErrorResponse),
        (status = 404, description = "Carpool not found", body = ErrorResponse),
        (status = 400, description = "Not joined", body = ErrorResponse)
    ),
    params(
        ("carpool_id" = i64, Path, description = "Carpool identifier")
    )
)]
#[delete("/carpools/{carpool_id}/join")]
pub async fn leave_carpool(
    state: web::Data<AppState>,
    req: HttpRequest,
    carpool_id: web::Path<i64>,
) -> impl Responder {
    let requester_email = match claims_email(&req, &state).await {
        Ok(email) => email,
        Err(resp) => return resp,
    };

    let user_id = match fetch_user_id(&state.db, &requester_email).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let carpool = match fetch_carpool(&state.db, *carpool_id).await {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    if let Err(resp) = ensure_event_writable(&state.db, carpool.event_id).await {
        return resp;
    }

    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(_) => return server_error(),
    };
    if let Err(resp) = lock_event_user(&mut tx, carpool.event_id, user_id).await {
        return resp;
    }
    let sql = select_carpools_sql("FROM carpools c WHERE c.carpool_id = $1 FOR UPDATE");
    let carpool = match sqlx::query_as::<_, Carpool>(AssertSqlSafe(sql))
        .bind(*carpool_id)
        .fetch_optional(&mut *tx)
        .await
    {
        Ok(Some(carpool)) => carpool,
        Ok(None) => {
            return HttpResponse::NotFound().json(ErrorResponse {
                error: "carpool_not_found".into(),
                details: None,
            });
        }
        Err(_) => return server_error(),
    };

    let was_joined = match sqlx::query(
        "DELETE FROM carpool_passengers WHERE carpool_id = $1 AND user_id = $2 RETURNING user_id",
    )
    .bind(*carpool_id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await
    {
        Ok(Some(_)) => true,
        Ok(None) => false,
        Err(_) => return server_error(),
    };

    if !was_joined {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "not_joined".into(),
            details: Some("Vous n'êtes pas inscrit à ce covoiturage".into()),
        });
    }

    let new_seats_taken = match sqlx::query_scalar::<_, i32>(
        "UPDATE carpools
         SET seats_taken = GREATEST(seats_taken - 1, 0), updated_at = NOW()
         WHERE carpool_id = $1
         RETURNING seats_taken",
    )
    .bind(*carpool_id)
    .fetch_one(&mut *tx)
    .await
    {
        Ok(value) => value,
        Err(e) => {
            warn!("Failed to update seats_taken: {}", e);
            return server_error();
        }
    };
    if tx.commit().await.is_err() {
        return server_error();
    }

    info!("User {} left carpool {}", user_id, carpool_id);

    publish_event(
        &state.redis_client,
        carpool.event_id,
        &json!({"type": "carpool_left", "carpool_id": *carpool_id, "user_id": user_id}),
    )
    .await;

    // Notify driver
    if state.notifications.is_enabled() {
        let passenger_handle =
            match sqlx::query_scalar::<_, String>("SELECT handle FROM users WHERE id = $1")
                .bind(user_id)
                .fetch_optional(&state.db)
                .await
            {
                Ok(Some(h)) => h,
                _ => "Un utilisateur".to_string(),
            };

        let body = format!("{} a quitté votre covoiturage", passenger_handle);
        let dedup = format!("carpool_leave:{}:{}", *carpool_id, user_id);
        notify_users(
            &state.notifications,
            &state.db,
            &[carpool.driver_id],
            NotificationRequest {
                title: "Un passager est parti",
                body: body.as_str(),
                data: json!({
                    "type": "carpool_left",
                    "event_id": carpool.event_id,
                    "carpool_id": *carpool_id
                }),
                dedup_base_key: Some(dedup.as_str()),
                dedup_ttl: Some(300),
            },
        )
        .await;
    }

    HttpResponse::Ok().json(CarpoolLeaveResponse {
        success: true,
        seats_taken: new_seats_taken,
        seats_total: carpool.seats_total,
        message: "Désinscription réussie".into(),
    })
}
