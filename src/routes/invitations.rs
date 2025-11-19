use actix_web::{HttpRequest, HttpResponse, Responder, delete, get, patch, post, web};

use crate::{
    auth::extract_claims_from_auth,
    models::{
        ErrorResponse, Invitation, InvitationPatchPayload, InvitationPayload, StatusResponse,
    },
    state::AppState,
};

fn claims_email(req: &HttpRequest, state: &AppState) -> Result<String, HttpResponse> {
    let claims = extract_claims_from_auth(req, &state.jwt_secret)?;
    Ok(claims.sub.to_lowercase())
}

async fn fetch_event_owner_email(db: &sqlx::PgPool, event_id: i64) -> Result<String, HttpResponse> {
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

    owner.ok_or_else(|| {
        HttpResponse::NotFound().json(ErrorResponse {
            error: "event_not_found".into(),
            details: None,
        })
    })
}

async fn ensure_event_owner(
    req: &HttpRequest,
    state: &AppState,
    event_id: i64,
) -> Result<(), HttpResponse> {
    let requester = claims_email(req, state)?;
    let owner = fetch_event_owner_email(&state.db, event_id).await?;
    if owner == requester {
        Ok(())
    } else {
        Err(HttpResponse::Forbidden().json(ErrorResponse {
            error: "forbidden".into(),
            details: Some("only the creator can manage invitations".into()),
        }))
    }
}

fn validate_status(status: &str) -> bool {
    matches!(status, "Waiting" | "Accepted" | "Declined")
}

async fn fetch_user_id_by_email(db: &sqlx::PgPool, email: &str) -> Result<i64, HttpResponse> {
    let normalized = email.trim().to_lowercase();
    if normalized.is_empty() {
        return Err(HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_email".into(),
            details: Some("email is required".into()),
        }));
    }

    let record =
        sqlx::query_scalar::<_, i64>("SELECT id FROM users WHERE lower(email) = lower($1)")
            .bind(&normalized)
            .fetch_optional(db)
            .await
            .map_err(|_| {
                HttpResponse::InternalServerError().json(ErrorResponse {
                    error: "db_error".into(),
                    details: None,
                })
            })?;

    match record {
        Some(id) => Ok(id),
        None => Err(HttpResponse::NotFound().json(ErrorResponse {
            error: "user_not_found".into(),
            details: None,
        })),
    }
}

#[utoipa::path(
    get,
    path = "/events/{event_id}/invitations",
    tag = "invitations",
    responses(
        (status = 200, description = "Invitations de l'événement", body = [Invitation]),
        (status = 403, description = "Non autorisé", body = ErrorResponse),
        (status = 404, description = "Événement introuvable", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Identifiant de l'événement")
    )
)]
#[get("/events/{event_id}/invitations")]
pub async fn list_event_invitations(
    state: web::Data<AppState>,
    req: HttpRequest,
    event_id: web::Path<i64>,
) -> impl Responder {
    if let Err(resp) = ensure_event_owner(&req, state.get_ref(), *event_id).await {
        return resp;
    }

    match sqlx::query_as::<_, Invitation>(
        "SELECT i.event_id, u.email, i.status, i.date_invi, e.name_event AS event_name
         FROM invitations i
         JOIN users u ON u.id = i.user_id
         JOIN events e ON e.event_id = i.event_id
         WHERE i.event_id = $1
         ORDER BY i.date_invi DESC",
    )
    .bind(*event_id)
    .fetch_all(&state.db)
    .await
    {
        Ok(list) => HttpResponse::Ok().json(list),
        Err(_) => HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        }),
    }
}

#[utoipa::path(
    post,
    path = "/events/{event_id}/invitations",
    tag = "invitations",
    request_body = InvitationPayload,
    responses(
        (status = 201, description = "Invitation créée", body = Invitation),
        (status = 400, description = "Payload invalide", body = ErrorResponse),
        (status = 403, description = "Non autorisé", body = ErrorResponse),
        (status = 404, description = "Événement ou utilisateur introuvable", body = ErrorResponse),
        (status = 409, description = "Invitation existante", body = ErrorResponse)
    )
)]
#[post("/events/{event_id}/invitations")]
pub async fn create_invitation(
    state: web::Data<AppState>,
    req: HttpRequest,
    event_id: web::Path<i64>,
    payload: web::Json<InvitationPayload>,
) -> impl Responder {
    if let Err(resp) = ensure_event_owner(&req, state.get_ref(), *event_id).await {
        return resp;
    }

    let user_id = match fetch_user_id_by_email(&state.db, &payload.email).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let res = sqlx::query_as::<_, Invitation>(
        "INSERT INTO invitations (event_id, user_id, status)
         VALUES ($1, $2, 'Waiting')
         RETURNING event_id, $3 AS email, status, date_invi,
                   (SELECT name_event FROM events WHERE event_id = $1) AS event_name",
    )
    .bind(*event_id)
    .bind(user_id)
    .bind(payload.email.trim().to_lowercase())
    .fetch_one(&state.db)
    .await;

    match res {
        Ok(inv) => HttpResponse::Created().json(inv),
        Err(sqlx::Error::Database(db_err)) if db_err.code().as_deref() == Some("23505") => {
            HttpResponse::Conflict().json(ErrorResponse {
                error: "invitation_exists".into(),
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
    path = "/events/{event_id}/invitations/{email}",
    tag = "invitations",
    responses(
        (status = 200, description = "Invitation supprimée", body = StatusResponse),
        (status = 403, description = "Non autorisé", body = ErrorResponse),
        (status = 404, description = "Invitation introuvable", body = ErrorResponse)
    )
)]
#[delete("/events/{event_id}/invitations/{email}")]
pub async fn delete_invitation(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<(i64, String)>,
) -> impl Responder {
    let (event_id, email) = path.into_inner();
    if let Err(resp) = ensure_event_owner(&req, state.get_ref(), event_id).await {
        return resp;
    }
    let user_id = match fetch_user_id_by_email(&state.db, &email).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    match sqlx::query("DELETE FROM invitations WHERE event_id = $1 AND user_id = $2")
        .bind(event_id)
        .bind(user_id)
        .execute(&state.db)
        .await
    {
        Ok(result) if result.rows_affected() == 0 => HttpResponse::NotFound().json(ErrorResponse {
            error: "invitation_not_found".into(),
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
    path = "/my/invitations",
    tag = "invitations",
    responses(
        (status = 200, description = "Invitations de l'utilisateur", body = [Invitation]),
        (status = 401, description = "Authentification requise", body = ErrorResponse)
    )
)]
#[get("/my/invitations")]
pub async fn list_my_invitations(state: web::Data<AppState>, req: HttpRequest) -> impl Responder {
    let email = match claims_email(&req, state.get_ref()) {
        Ok(e) => e,
        Err(resp) => return resp,
    };

    match sqlx::query_as::<_, Invitation>(
        "SELECT i.event_id, u.email, i.status, i.date_invi, e.name_event AS event_name
         FROM invitations i
         JOIN users u ON u.id = i.user_id
         JOIN events e ON e.event_id = i.event_id
         WHERE lower(u.email) = lower($1)
         ORDER BY i.date_invi DESC",
    )
    .bind(&email)
    .fetch_all(&state.db)
    .await
    {
        Ok(list) => HttpResponse::Ok().json(list),
        Err(_) => HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        }),
    }
}

#[utoipa::path(
    patch,
    path = "/my/invitations/{event_id}",
    tag = "invitations",
    request_body = InvitationPatchPayload,
    responses(
        (status = 200, description = "Invitation mise à jour", body = Invitation),
        (status = 400, description = "Payload invalide", body = ErrorResponse),
        (status = 401, description = "Authentification requise", body = ErrorResponse),
        (status = 404, description = "Invitation introuvable", body = ErrorResponse)
    )
)]
#[patch("/my/invitations/{event_id}")]
pub async fn respond_invitation(
    state: web::Data<AppState>,
    req: HttpRequest,
    event_id: web::Path<i64>,
    payload: web::Json<InvitationPatchPayload>,
) -> impl Responder {
    let email = match claims_email(&req, state.get_ref()) {
        Ok(e) => e,
        Err(resp) => return resp,
    };

    let status = match payload.status.clone() {
        Some(s) if validate_status(s.trim()) => s.trim().to_string(),
        _ => {
            return HttpResponse::BadRequest().json(ErrorResponse {
                error: "invalid_status".into(),
                details: Some("status must be Accepted or Declined".into()),
            });
        }
    };

    let user_id = match fetch_user_id_by_email(&state.db, &email).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let target_status = status;
    let res = sqlx::query_as::<_, Invitation>(
        "UPDATE invitations
         SET status = $1
         WHERE event_id = $2 AND user_id = $3
         RETURNING event_id, $4 AS email, status, date_invi,
                   (SELECT name_event FROM events WHERE event_id = $2) AS event_name",
    )
    .bind(&target_status)
    .bind(*event_id)
    .bind(user_id)
    .bind(email)
    .fetch_optional(&state.db)
    .await;

    match res {
        Ok(Some(inv)) => HttpResponse::Ok().json(inv),
        Ok(None) => HttpResponse::NotFound().json(ErrorResponse {
            error: "invitation_not_found".into(),
            details: None,
        }),
        Err(_) => HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        }),
    }
}
