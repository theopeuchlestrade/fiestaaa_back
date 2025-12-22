use actix_web::{HttpRequest, HttpResponse, Responder, delete, get, patch, post, web};
use serde::Deserialize;
use serde_json::json;
use sqlx::{PgPool, Row};

use crate::{
    auth::extract_claims_from_auth,
    handles::{is_valid_handle, looks_like_email, normalize_handle},
    models::{
        ErrorResponse, Friend, FriendRequest, FriendRequestActionPayload, FriendRequestPayload,
        FriendSearchResult, StatusResponse,
    },
    notifications::notify_users,
    realtime::{event_types, publish_global},
    state::AppState,
};

#[derive(Debug, sqlx::FromRow)]
#[allow(dead_code)]
struct UserIdentity {
    id: i64,
    email: String,
    handle: String,
    avatar_url: Option<String>,
}

async fn ensure_avatar_column(db: &sqlx::PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("ALTER TABLE users ADD COLUMN IF NOT EXISTS avatar_url TEXT;")
        .execute(db)
        .await?;
    Ok(())
}

fn ordered_pair(a: i64, b: i64) -> (i64, i64) {
    if a < b { (a, b) } else { (b, a) }
}

async fn current_user(req: &HttpRequest, state: &AppState) -> Result<UserIdentity, HttpResponse> {
    let _ = sqlx::query("ALTER TABLE users ADD COLUMN IF NOT EXISTS avatar_url TEXT;")
        .execute(&state.db)
        .await;
    let claims = extract_claims_from_auth(req, &state.jwt_secret)?;
    match find_user_by_email(&state.db, &claims.sub).await? {
        Some(user) => Ok(user),
        None => Err(HttpResponse::Unauthorized().json(ErrorResponse {
            error: "user_not_found".into(),
            details: None,
        })),
    }
}

async fn find_user_by_email(
    db: &PgPool,
    email: &str,
) -> Result<Option<UserIdentity>, HttpResponse> {
    let _ = ensure_avatar_column(db).await;
    let normalized = email.trim().to_lowercase();
    if normalized.is_empty() {
        return Err(HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_email".into(),
            details: Some("email is required".into()),
        }));
    }

    sqlx::query_as::<_, UserIdentity>(
        "SELECT id, email, handle, avatar_url FROM users WHERE lower(email) = lower($1)",
    )
    .bind(&normalized)
    .fetch_optional(db)
    .await
    .map_err(|_| db_error())
}

async fn find_user_by_handle(
    db: &PgPool,
    handle: &str,
) -> Result<Option<UserIdentity>, HttpResponse> {
    let _ = ensure_avatar_column(db).await;
    sqlx::query_as::<_, UserIdentity>(
        "SELECT id, email, handle, avatar_url FROM users WHERE lower(handle) = lower($1)",
    )
    .bind(handle)
    .fetch_optional(db)
    .await
    .map_err(|_| db_error())
}

enum TargetIdentifier {
    Email(String),
    Handle(String),
}

fn parse_identifier(raw: &str) -> Result<TargetIdentifier, HttpResponse> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_identifier".into(),
            details: Some("valeur requise".into()),
        }));
    }

    if looks_like_email(trimmed) {
        return Ok(TargetIdentifier::Email(trimmed.to_lowercase()));
    }

    let normalized = normalize_handle(trimmed).normalized;
    if !is_valid_handle(&normalized) {
        return Err(HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_handle".into(),
            details: Some("format attendu: 4-32 chars [a-z0-9._-]".into()),
        }));
    }

    Ok(TargetIdentifier::Handle(normalized))
}

async fn resolve_identifier(db: &PgPool, raw: &str) -> Result<UserIdentity, HttpResponse> {
    match parse_identifier(raw)? {
        TargetIdentifier::Email(email) => match find_user_by_email(db, &email).await? {
            Some(u) => Ok(u),
            None => Err(HttpResponse::NotFound().json(ErrorResponse {
                error: "user_not_found".into(),
                details: None,
            })),
        },
        TargetIdentifier::Handle(handle) => match find_user_by_handle(db, &handle).await? {
            Some(u) => Ok(u),
            None => Err(HttpResponse::NotFound().json(ErrorResponse {
                error: "user_not_found".into(),
                details: Some("identifiant introuvable".into()),
            })),
        },
    }
}

async fn are_friends(db: &PgPool, a: i64, b: i64) -> Result<bool, HttpResponse> {
    let (u1, u2) = ordered_pair(a, b);
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(
            SELECT 1 FROM friendships WHERE user_a = $1 AND user_b = $2
        )",
    )
    .bind(u1)
    .bind(u2)
    .fetch_one(db)
    .await
    .map_err(|_| db_error())
}

async fn pending_request_exists(db: &PgPool, a: i64, b: i64) -> Result<bool, HttpResponse> {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(
            SELECT 1
            FROM friend_requests
            WHERE status = 'Pending'
              AND ((sender_id = $1 AND receiver_id = $2) OR (sender_id = $2 AND receiver_id = $1))
        )",
    )
    .bind(a)
    .bind(b)
    .fetch_one(db)
    .await
    .map_err(|_| db_error())
}

async fn fetch_request_view(db: &PgPool, id: i64) -> Result<FriendRequest, HttpResponse> {
    sqlx::query_as::<_, FriendRequest>(
        "SELECT fr.id,
                sender.email AS sender_email,
                sender.handle AS sender_handle,
                sender.avatar_url AS sender_avatar_url,
                receiver.email AS receiver_email,
                receiver.handle AS receiver_handle,
                receiver.avatar_url AS receiver_avatar_url,
                fr.status,
                fr.created_at
         FROM friend_requests fr
         JOIN users sender ON sender.id = fr.sender_id
         JOIN users receiver ON receiver.id = fr.receiver_id
         WHERE fr.id = $1",
    )
    .bind(id)
    .fetch_one(db)
    .await
    .map_err(|_| db_error())
}

fn db_error() -> HttpResponse {
    HttpResponse::InternalServerError().json(ErrorResponse {
        error: "db_error".into(),
        details: None,
    })
}

#[utoipa::path(
    get,
    path = "/me/friends",
    tag = "friends",
    responses(
        (status = 200, description = "Liste des amis", body = [Friend]),
        (status = 401, description = "Authentification requise", body = ErrorResponse)
    )
)]
#[get("/me/friends")]
pub async fn list_friends(state: web::Data<AppState>, req: HttpRequest) -> impl Responder {
    let user = match current_user(&req, state.get_ref()).await {
        Ok(u) => u,
        Err(resp) => return resp,
    };

    match sqlx::query_as::<_, Friend>(
        "SELECT
            CASE WHEN f.user_a = $1 THEN u2.email ELSE u1.email END AS email,
            CASE WHEN f.user_a = $1 THEN u2.handle ELSE u1.handle END AS handle,
            CASE WHEN f.user_a = $1 THEN u2.avatar_url ELSE u1.avatar_url END AS avatar_url,
            f.created_at AS since
         FROM friendships f
         JOIN users u1 ON u1.id = f.user_a
         JOIN users u2 ON u2.id = f.user_b
         WHERE f.user_a = $1 OR f.user_b = $1
         ORDER BY f.created_at DESC",
    )
    .bind(user.id)
    .fetch_all(&state.db)
    .await
    {
        Ok(list) => HttpResponse::Ok().json(list),
        Err(_) => db_error(),
    }
}

#[derive(Deserialize)]
pub struct FriendSearchQuery {
    pub q: Option<String>,
    pub limit: Option<i64>,
}

#[utoipa::path(
    get,
    path = "/friends/search",
    tag = "friends",
    params(
        ("q" = String, Query, description = "Recherche par email ou identifiant"),
        ("limit" = i64, Query, description = "Nombre de résultats (max 15)")
    ),
    responses(
        (status = 200, description = "Résultats de recherche", body = [FriendSearchResult]),
        (status = 401, description = "Authentification requise", body = ErrorResponse)
    )
)]
#[get("/friends/search")]
pub async fn search_friends(
    state: web::Data<AppState>,
    req: HttpRequest,
    query: web::Query<FriendSearchQuery>,
) -> impl Responder {
    let user = match current_user(&req, state.get_ref()).await {
        Ok(u) => u,
        Err(resp) => return resp,
    };

    let raw_q = query.q.as_deref().unwrap_or("").trim().to_string();
    if raw_q.len() < 2 {
        return HttpResponse::Ok().json(Vec::<FriendSearchResult>::new());
    }

    let pattern = format!("%{raw_q}%");
    let limit = query.limit.unwrap_or(8).clamp(1, 15);

    match sqlx::query_as::<_, FriendSearchResult>(
        "SELECT u.email, u.handle, u.avatar_url
         FROM users u
         WHERE (lower(u.email) LIKE lower($2) OR lower(u.handle) LIKE lower($2))
           AND u.id <> $1
           AND NOT EXISTS (
                SELECT 1 FROM friendships f
                WHERE (f.user_a = $1 AND f.user_b = u.id)
                   OR (f.user_b = $1 AND f.user_a = u.id)
           )
           AND NOT EXISTS (
                SELECT 1 FROM friend_requests fr
                WHERE fr.status = 'Pending'
                  AND ((fr.sender_id = $1 AND fr.receiver_id = u.id)
                       OR (fr.sender_id = u.id AND fr.receiver_id = $1))
           )
         ORDER BY lower(u.handle) ASC
         LIMIT $3",
    )
    .bind(user.id)
    .bind(&pattern)
    .bind(limit)
    .fetch_all(&state.db)
    .await
    {
        Ok(results) => HttpResponse::Ok().json(results),
        Err(_) => db_error(),
    }
}

#[utoipa::path(
    post,
    path = "/friends/requests",
    tag = "friends",
    request_body = FriendRequestPayload,
    responses(
        (status = 201, description = "Demande envoyée", body = FriendRequest),
        (status = 400, description = "Payload invalide", body = ErrorResponse),
        (status = 401, description = "Authentification requise", body = ErrorResponse),
        (status = 404, description = "Utilisateur introuvable", body = ErrorResponse),
        (status = 409, description = "Déjà amis ou demande existante", body = ErrorResponse)
    )
)]
#[post("/friends/requests")]
pub async fn create_friend_request(
    state: web::Data<AppState>,
    req: HttpRequest,
    payload: web::Json<FriendRequestPayload>,
) -> impl Responder {
    let requester = match current_user(&req, state.get_ref()).await {
        Ok(u) => u,
        Err(resp) => return resp,
    };

    let target = match resolve_identifier(&state.db, &payload.identifier).await {
        Ok(user) => user,
        Err(resp) => return resp,
    };

    if target.id == requester.id {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "cannot_friend_self".into(),
            details: None,
        });
    }

    match are_friends(&state.db, requester.id, target.id).await {
        Ok(true) => {
            return HttpResponse::Conflict().json(ErrorResponse {
                error: "already_friends".into(),
                details: None,
            });
        }
        Ok(false) => {}
        Err(resp) => return resp,
    }

    match pending_request_exists(&state.db, requester.id, target.id).await {
        Ok(true) => {
            return HttpResponse::Conflict().json(ErrorResponse {
                error: "request_exists".into(),
                details: None,
            });
        }
        Ok(false) => {}
        Err(resp) => return resp,
    }

    let insert = sqlx::query(
        "INSERT INTO friend_requests (sender_id, receiver_id, status)
         VALUES ($1, $2, 'Pending')
         RETURNING id",
    )
    .bind(requester.id)
    .bind(target.id)
    .fetch_one(&state.db)
    .await;

    let row = match insert {
        Ok(row) => row,
        Err(sqlx::Error::Database(db_err)) if db_err.code().as_deref() == Some("23505") => {
            return HttpResponse::Conflict().json(ErrorResponse {
                error: "request_exists".into(),
                details: None,
            });
        }
        Err(_) => {
            return db_error();
        }
    };

    match fetch_request_view(&state.db, row.get("id")).await {
        Ok(req) => {
            let title = "Nouvelle demande d'ami".to_string();
            let body = format!("{} souhaite t'ajouter", req.sender_handle);
            let dedup = format!("friend_request:{}", req.id);
            notify_users(
                &state.notifications,
                &state.db,
                &[target.id],
                &title,
                &body,
                json!({
                    "type": "friend_request",
                    "request_id": req.id,
                    "from_email": req.sender_email,
                    "from_handle": req.sender_handle
                }),
                Some(&dedup),
                Some(600),
            )
            .await;
            publish_global(
                &state.redis_client,
                &serde_json::json!({
                    "type": event_types::FRIEND_REQUESTS_CHANGED,
                    "action": "created",
                    "request_id": req.id,
                    "status": req.status.clone(),
                }),
            )
            .await;
            HttpResponse::Created().json(req)
        }
        Err(resp) => resp,
    }
}

#[utoipa::path(
    get,
    path = "/friends/requests",
    tag = "friends",
    responses(
        (status = 200, description = "Demandes d'amis", body = [FriendRequest]),
        (status = 401, description = "Authentification requise", body = ErrorResponse)
    )
)]
#[get("/friends/requests")]
pub async fn list_friend_requests(state: web::Data<AppState>, req: HttpRequest) -> impl Responder {
    let _ = ensure_avatar_column(&state.db).await;
    let user = match current_user(&req, state.get_ref()).await {
        Ok(u) => u,
        Err(resp) => return resp,
    };

    match sqlx::query_as::<_, FriendRequest>(
        "SELECT fr.id,
                sender.email AS sender_email,
                sender.handle AS sender_handle,
                sender.avatar_url AS sender_avatar_url,
                receiver.email AS receiver_email,
                receiver.handle AS receiver_handle,
                receiver.avatar_url AS receiver_avatar_url,
                fr.status,
                fr.created_at
         FROM friend_requests fr
         JOIN users sender ON sender.id = fr.sender_id
         JOIN users receiver ON receiver.id = fr.receiver_id
         WHERE fr.sender_id = $1 OR fr.receiver_id = $1
         ORDER BY fr.created_at DESC",
    )
    .bind(user.id)
    .fetch_all(&state.db)
    .await
    {
        Ok(list) => HttpResponse::Ok().json(list),
        Err(_) => HttpResponse::Ok().json(Vec::<FriendRequest>::new()),
    }
}

#[utoipa::path(
    patch,
    path = "/friends/requests/{id}",
    tag = "friends",
    request_body = FriendRequestActionPayload,
    responses(
        (status = 200, description = "Demande mise à jour", body = FriendRequest),
        (status = 400, description = "Payload invalide", body = ErrorResponse),
        (status = 401, description = "Authentification requise", body = ErrorResponse),
        (status = 403, description = "Non autorisé", body = ErrorResponse),
        (status = 404, description = "Demande introuvable", body = ErrorResponse),
        (status = 409, description = "Déjà traitée", body = ErrorResponse)
    )
)]
#[patch("/friends/requests/{id}")]
pub async fn respond_friend_request(
    state: web::Data<AppState>,
    req: HttpRequest,
    id: web::Path<i64>,
    payload: web::Json<FriendRequestActionPayload>,
) -> impl Responder {
    let user = match current_user(&req, state.get_ref()).await {
        Ok(u) => u,
        Err(resp) => return resp,
    };

    let target_status = match payload.status.trim() {
        "Accepted" => "Accepted",
        "Declined" => "Declined",
        _ => {
            return HttpResponse::BadRequest().json(ErrorResponse {
                error: "invalid_status".into(),
                details: Some("Status must be Accepted or Declined".into()),
            });
        }
    };

    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(_) => return db_error(),
    };

    let request_row = sqlx::query(
        "SELECT sender_id, receiver_id, status
         FROM friend_requests
         WHERE id = $1
         FOR UPDATE",
    )
    .bind(*id)
    .fetch_optional(&mut *tx)
    .await;

    let request_row = match request_row {
        Ok(row) => row,
        Err(_) => {
            let _ = tx.rollback().await;
            return db_error();
        }
    };

    let Some(row) = request_row else {
        let _ = tx.rollback().await;
        return HttpResponse::NotFound().json(ErrorResponse {
            error: "request_not_found".into(),
            details: None,
        });
    };

    let sender_id: i64 = row.get("sender_id");
    let receiver_id: i64 = row.get("receiver_id");
    let current_status: String = row.get("status");

    if receiver_id != user.id {
        let _ = tx.rollback().await;
        return HttpResponse::Forbidden().json(ErrorResponse {
            error: "forbidden".into(),
            details: Some("Seul le destinataire peut répondre à la demande".into()),
        });
    }

    if current_status != "Pending" {
        let _ = tx.rollback().await;
        return HttpResponse::Conflict().json(ErrorResponse {
            error: "request_already_processed".into(),
            details: None,
        });
    }

    let update = sqlx::query(
        "UPDATE friend_requests
         SET status = $1, responded_at = NOW()
         WHERE id = $2",
    )
    .bind(target_status)
    .bind(*id)
    .execute(&mut *tx)
    .await;

    if update.is_err() {
        let _ = tx.rollback().await;
        return db_error();
    }

    if target_status == "Accepted" {
        let (a, b) = ordered_pair(sender_id, receiver_id);
        if let Err(_) = sqlx::query(
            "INSERT INTO friendships (user_a, user_b)
             VALUES ($1, $2)
             ON CONFLICT DO NOTHING",
        )
        .bind(a)
        .bind(b)
        .execute(&mut *tx)
        .await
        {
            let _ = tx.rollback().await;
            return db_error();
        }
    }

    if tx.commit().await.is_err() {
        return db_error();
    }

    match fetch_request_view(&state.db, *id).await {
        Ok(req) => {
            let status_label = if target_status == "Accepted" {
                "acceptée"
            } else {
                "refusée"
            };
            let title = format!("Demande d'ami {status_label}");
            let body = format!("{} a {status_label} votre demande", req.receiver_handle);
            let dedup = format!("friend_response:{}:{target_status}", req.id);
            notify_users(
                &state.notifications,
                &state.db,
                &[sender_id],
                &title,
                &body,
                json!({
                    "type": "friend_response",
                    "request_id": req.id,
                    "status": target_status,
                    "from_email": req.receiver_email,
                    "from_handle": req.receiver_handle
                }),
                Some(&dedup),
                Some(300),
            )
            .await;
            publish_global(
                &state.redis_client,
                &serde_json::json!({
                    "type": event_types::FRIEND_REQUESTS_CHANGED,
                    "action": "updated",
                    "request_id": req.id,
                    "status": target_status,
                }),
            )
            .await;
            if target_status == "Accepted" {
                publish_global(
                    &state.redis_client,
                    &serde_json::json!({
                        "type": event_types::FRIENDSHIPS_CHANGED,
                        "action": "created",
                        "user_emails": [req.sender_email.clone(), req.receiver_email.clone()],
                    }),
                )
                .await;
            }
            HttpResponse::Ok().json(req)
        }
        Err(resp) => resp,
    }
}

#[utoipa::path(
    delete,
    path = "/friends/{identifier}",
    tag = "friends",
    responses(
        (status = 200, description = "Ami supprimé", body = StatusResponse),
        (status = 401, description = "Authentification requise", body = ErrorResponse),
        (status = 404, description = "Ami introuvable", body = ErrorResponse)
    )
)]
#[delete("/friends/{identifier}")]
pub async fn delete_friend(
    state: web::Data<AppState>,
    req: HttpRequest,
    identifier: web::Path<String>,
) -> impl Responder {
    let user = match current_user(&req, state.get_ref()).await {
        Ok(u) => u,
        Err(resp) => return resp,
    };

    let target = match resolve_identifier(&state.db, &identifier).await {
        Ok(u) => u,
        Err(resp) => return resp,
    };

    let (a, b) = ordered_pair(user.id, target.id);
    match sqlx::query("DELETE FROM friendships WHERE user_a = $1 AND user_b = $2")
        .bind(a)
        .bind(b)
        .execute(&state.db)
        .await
    {
        Ok(result) if result.rows_affected() == 0 => HttpResponse::NotFound().json(ErrorResponse {
            error: "friend_not_found".into(),
            details: None,
        }),
        Ok(_) => {
            publish_global(
                &state.redis_client,
                &serde_json::json!({
                    "type": event_types::FRIENDSHIPS_CHANGED,
                    "action": "deleted",
                    "user_emails": [user.email, target.email],
                }),
            )
            .await;
            HttpResponse::Ok().json(StatusResponse {
                status: "deleted".into(),
            })
        }
        Err(_) => db_error(),
    }
}
