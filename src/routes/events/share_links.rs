use actix_web::{Responder, post, web};
use chrono::{Duration, Utc};
use sqlx::AssertSqlSafe;
use uuid::Uuid;

use super::*;

#[utoipa::path(
    post,
    path = "/events/{event_id}/share",
    tag = "events",
    responses(
        (status = 201, description = "Share link generated. This is a bearer token: any authenticated user who obtains it can claim it until expiration or consumption.", body = ShareTokenResponse),
        (status = 403, description = "Unauthorized", body = ErrorResponse),
        (status = 404, description = "Event not found", body = ErrorResponse),
        (status = 500, description = "Database error", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Event identifier")
    )
)]
#[post("/events/{event_id}/share")]
pub async fn create_share_link(
    state: web::Data<AppState>,
    req: HttpRequest,
    event_id: web::Path<i64>,
) -> impl Responder {
    let owner_email = match claims_email(&req, state.get_ref()).await {
        Ok(email) => email,
        Err(resp) => return resp,
    };
    let owner_user_id = match fetch_user_id(&state.db, &owner_email).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    if let Err(resp) = ensure_event_owner(&req, state.get_ref(), *event_id).await {
        return resp;
    }
    if let Err(resp) = ensure_event_writable(&state.db, *event_id).await {
        return resp;
    }

    // also ensures event exists
    if let Err(resp) = ensure_event_exists(&state.db, *event_id).await {
        return resp;
    }

    let token = Uuid::new_v4();
    let token_hash = sha256_hex(&token.to_string());
    let expires_at = Utc::now() + Duration::hours(OWNER_SHARE_TOKEN_TTL_HOURS);

    let res = sqlx::query(
        "INSERT INTO event_share_tokens (token_hash, event_id, created_by_user_id, expires_at)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(&token_hash)
    .bind(*event_id)
    .bind(owner_user_id)
    .bind(expires_at)
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
        (status = 200, description = "Link consumed and event accessible", body = ShareClaimResponse),
        (status = 400, description = "Missing token", body = ErrorResponse),
        (status = 401, description = "Authentication required", body = ErrorResponse),
        (status = 404, description = "Link not found", body = ErrorResponse),
        (status = 410, description = "Link already consumed", body = ErrorResponse),
        (status = 500, description = "Database error", body = ErrorResponse)
    )
)]
#[post("/share/claim")]
pub async fn claim_share_link(
    state: web::Data<AppState>,
    req: HttpRequest,
    payload: web::Json<ShareClaimPayload>,
) -> impl Responder {
    let claims = match extract_active_claims_from_auth(&req, &state.db, &state.jwt_secret).await {
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
    let token_hash = sha256_hex(&parsed_token.to_string());

    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(_) => return server_error(),
    };

    let token_row = match sqlx::query(
        "SELECT event_id,
                used_at,
                expires_at,
                fiestaaa_decrypt_text(target_email_ciphertext) AS target_email
         FROM event_share_tokens
         WHERE token_hash = $1
         FOR UPDATE",
    )
    .bind(&token_hash)
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
    let expires_at: chrono::DateTime<chrono::Utc> = match token_row.try_get("expires_at") {
        Ok(v) => v,
        Err(_) => return server_error(),
    };
    if expires_at < chrono::Utc::now() {
        return HttpResponse::Gone().json(ErrorResponse {
            error: "token_expired".into(),
            details: Some("Ce lien a expiré".into()),
        });
    }
    let target_email: Option<String> = match token_row.try_get("target_email") {
        Ok(v) => v,
        Err(_) => return server_error(),
    };
    if let Some(expected_email) = target_email
        && !expected_email.eq_ignore_ascii_case(&claims.sub)
    {
        return HttpResponse::Forbidden().json(ErrorResponse {
            error: "token_recipient_mismatch".into(),
            details: Some("Ce lien est réservé à une autre adresse email".into()),
        });
    }
    let event_id: i64 = match token_row.try_get("event_id") {
        Ok(v) => v,
        Err(_) => return server_error(),
    };
    if let Err(resp) = ensure_event_writable(&state.db, event_id).await {
        return resp;
    }

    let event_sql = select_events_sql(
        "FROM events e
         JOIN users owner ON owner.id = e.owner_user_id
         WHERE e.event_id = $1",
    );
    let event = sqlx::query_as::<_, Event>(AssertSqlSafe(event_sql))
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

    if let Some(limit) = event.invitation_deadline
        && chrono::Utc::now().date_naive() > limit
    {
        return HttpResponse::Gone().json(ErrorResponse {
            error: "invitation_expired".into(),
            details: Some("La date limite pour répondre est dépassée".into()),
        });
    }

    let user_id = match fetch_user_id(&state.db, &claims.sub).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    // Ensure an invitation exists, but let the user choose their response later.
    let invitation_inserted = match sqlx::query(
        "INSERT INTO invitations (event_id, user_id, status)
         VALUES ($1, $2, 'Waiting')
         ON CONFLICT (event_id, user_id) DO NOTHING",
    )
    .bind(event.event_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    {
        Ok(result) => result.rows_affected() > 0,
        Err(_) => return server_error(),
    };

    let update_res = sqlx::query(
        "UPDATE event_share_tokens
         SET used_at = NOW(), used_by_user_id = $1
         WHERE token_hash = $2",
    )
    .bind(user_id)
    .bind(&token_hash)
    .execute(&mut *tx)
    .await;

    if update_res.is_err() {
        return server_error();
    }

    if tx.commit().await.is_err() {
        return server_error();
    }

    if invitation_inserted {
        publish_event_type(
            &state.redis_client,
            event.event_id,
            event_types::EVENT_INVITATIONS_CHANGED,
        )
        .await;
        publish_global_type(&state.redis_client, event_types::INVITATIONS_CHANGED).await;
    }

    HttpResponse::Ok().json(ShareClaimResponse { event })
}
