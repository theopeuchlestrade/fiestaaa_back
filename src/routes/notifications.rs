use actix_web::{HttpRequest, HttpResponse, Responder, post, web};

use crate::{
    auth::extract_active_claims_from_auth,
    models::{
        DeviceDeletePayload, DeviceRefreshPayload, DeviceRegisterPayload, ErrorResponse,
        StatusResponse,
    },
    notifications::find_user_id_by_email,
    state::AppState,
};

fn normalize_platform(raw: &str) -> Option<String> {
    let p = raw.trim().to_lowercase();
    match p.as_str() {
        "ios" | "android" | "web" => Some(p),
        _ => None,
    }
}

async fn current_user_id(req: &HttpRequest, state: &AppState) -> Result<i64, HttpResponse> {
    let claims = extract_active_claims_from_auth(req, &state.db, &state.jwt_secret).await?;
    match find_user_id_by_email(&state.db, &claims.sub).await {
        Ok(Some(id)) => Ok(id),
        Ok(None) => Err(HttpResponse::Unauthorized().json(ErrorResponse {
            error: "user_not_found".into(),
            details: None,
        })),
        Err(_) => Err(HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        })),
    }
}

#[utoipa::path(
    post,
    path = "/me/devices",
    tag = "notifications",
    request_body = DeviceRegisterPayload,
    responses(
        (status = 200, description = "Token saved", body = StatusResponse),
        (status = 400, description = "Invalid payload", body = ErrorResponse),
        (status = 401, description = "Authentication required", body = ErrorResponse),
        (status = 500, description = "Database error", body = ErrorResponse)
    )
)]
#[post("/me/devices")]
pub async fn register_device(
    state: web::Data<AppState>,
    req: HttpRequest,
    payload: web::Json<DeviceRegisterPayload>,
) -> impl Responder {
    let user_id = match current_user_id(&req, state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let payload = payload.into_inner();
    let token = payload.token.trim();
    if token.is_empty() {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_token".into(),
            details: Some("token requis".into()),
        });
    }

    let Some(platform) = normalize_platform(&payload.platform) else {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_platform".into(),
            details: Some("platform doit être ios, android ou web".into()),
        });
    };

    let res = sqlx::query(
        "INSERT INTO user_devices (
            user_id,
            fcm_token_ciphertext,
            fcm_token_lookup_hash,
            platform,
            locale,
            app_version,
            created_at,
            last_seen,
            disabled_at
         )
         VALUES ($1, fiestaaa_encrypt_text($2), fiestaaa_lookup_text($2), $3, $4, $5, NOW(), NOW(), NULL)
         ON CONFLICT (fcm_token_lookup_hash)
         DO UPDATE SET user_id = EXCLUDED.user_id,
                       platform = EXCLUDED.platform,
                       locale = EXCLUDED.locale,
                       app_version = EXCLUDED.app_version,
                       last_seen = NOW(),
                       disabled_at = NULL",
    )
    .bind(user_id)
    .bind(token)
    .bind(platform)
    .bind(payload.locale.as_deref())
    .bind(payload.app_version.as_deref())
    .execute(&state.db)
    .await;

    match res {
        Ok(_) => HttpResponse::Ok().json(StatusResponse {
            status: "saved".into(),
        }),
        Err(_) => HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        }),
    }
}

#[utoipa::path(
    post,
    path = "/me/devices/refresh",
    tag = "notifications",
    request_body = DeviceRefreshPayload,
    responses(
        (status = 200, description = "Token refreshed", body = StatusResponse),
        (status = 400, description = "Invalid payload", body = ErrorResponse),
        (status = 401, description = "Authentication required", body = ErrorResponse),
        (status = 500, description = "Database error", body = ErrorResponse)
    )
)]
#[post("/me/devices/refresh")]
pub async fn refresh_device(
    state: web::Data<AppState>,
    req: HttpRequest,
    payload: web::Json<DeviceRefreshPayload>,
) -> impl Responder {
    let user_id = match current_user_id(&req, state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let payload = payload.into_inner();
    let old_token = payload.old_token.trim();
    let new_token = payload.new_token.trim();
    if new_token.is_empty() {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_token".into(),
            details: Some("new_token requis".into()),
        });
    }

    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(_) => {
            return HttpResponse::InternalServerError().json(ErrorResponse {
                error: "db_error".into(),
                details: None,
            });
        }
    };

    if !old_token.is_empty() {
        let _ = sqlx::query(
            "UPDATE user_devices
             SET disabled_at = NOW()
             WHERE user_id = $1
               AND fiestaaa_lookup_matches(fcm_token_lookup_hash, $2)",
        )
        .bind(user_id)
        .bind(old_token)
        .execute(&mut *tx)
        .await;
    }

    let platform = match payload.platform.as_deref() {
        Some(raw) => match normalize_platform(raw) {
            Some(p) => Some(p),
            None => {
                let _ = tx.rollback().await;
                return HttpResponse::BadRequest().json(ErrorResponse {
                    error: "invalid_platform".into(),
                    details: Some("platform doit être ios, android ou web".into()),
                });
            }
        },
        None => None,
    };

    let final_platform = if let Some(p) = platform {
        p
    } else {
        match sqlx::query_scalar::<_, String>(
            "SELECT platform
             FROM user_devices
             WHERE user_id = $1
               AND fiestaaa_lookup_matches(fcm_token_lookup_hash, $2)",
        )
        .bind(user_id)
        .bind(old_token)
        .fetch_optional(&mut *tx)
        .await
        {
            Ok(Some(p)) => p,
            _ => "web".to_string(),
        }
    };

    let res = sqlx::query(
        "INSERT INTO user_devices (
            user_id,
            fcm_token_ciphertext,
            fcm_token_lookup_hash,
            platform,
            locale,
            app_version,
            created_at,
            last_seen,
            disabled_at
         )
         VALUES ($1, fiestaaa_encrypt_text($2), fiestaaa_lookup_text($2), $3, $4, $5, NOW(), NOW(), NULL)
         ON CONFLICT (fcm_token_lookup_hash)
         DO UPDATE SET user_id = EXCLUDED.user_id,
                       platform = EXCLUDED.platform,
                       locale = EXCLUDED.locale,
                       app_version = EXCLUDED.app_version,
                       last_seen = NOW(),
                       disabled_at = NULL",
    )
    .bind(user_id)
    .bind(new_token)
    .bind(final_platform)
    .bind(payload.locale.as_deref())
    .bind(payload.app_version.as_deref())
    .execute(&mut *tx)
    .await;

    let res = match res {
        Ok(_) => tx.commit().await,
        Err(err) => {
            let _ = tx.rollback().await;
            return HttpResponse::InternalServerError().json(ErrorResponse {
                error: "db_error".into(),
                details: Some(err.to_string()),
            });
        }
    };

    match res {
        Ok(_) => HttpResponse::Ok().json(StatusResponse {
            status: "refreshed".into(),
        }),
        Err(_) => HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        }),
    }
}

#[utoipa::path(
    post,
    path = "/me/devices/revoke",
    tag = "notifications",
    request_body = DeviceDeletePayload,
    responses(
        (status = 200, description = "Token deleted", body = StatusResponse),
        (status = 401, description = "Authentication required", body = ErrorResponse)
    )
)]
#[post("/me/devices/revoke")]
pub async fn delete_device(
    state: web::Data<AppState>,
    req: HttpRequest,
    payload: web::Json<DeviceDeletePayload>,
) -> impl Responder {
    let user_id = match current_user_id(&req, state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let token = payload.into_inner().token;

    let _ = sqlx::query(
        "UPDATE user_devices
         SET disabled_at = NOW()
         WHERE user_id = $1
           AND fiestaaa_lookup_matches(fcm_token_lookup_hash, $2)",
    )
    .bind(user_id)
    .bind(token.trim())
    .execute(&state.db)
    .await;

    HttpResponse::Ok().json(StatusResponse {
        status: "deleted".into(),
    })
}
