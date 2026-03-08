use actix_web::{HttpRequest, HttpResponse, Responder, get, post, web};
use log::error;
use sqlx::Row;
use uuid::Uuid;

use crate::{
    auth::extract_claims_from_auth,
    models::{
        ErrorResponse, QRCodeGenerateResponse, QRCodeScanPayload, QRCodeScanResponse,
        QRCodeStatsResponse,
    },
    routes::event_access::ensure_event_writable,
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
) -> Result<String, HttpResponse> {
    let requester = claims_email(req, state)?;
    let owner = fetch_event_owner_email(&state.db, event_id).await?;
    if owner == requester {
        Ok(owner)
    } else {
        Err(HttpResponse::Forbidden().json(ErrorResponse {
            error: "forbidden".into(),
            details: Some("only the event owner can scan QR codes".into()),
        }))
    }
}

async fn get_user_id_from_email(db: &sqlx::PgPool, email: &str) -> Result<i64, HttpResponse> {
    sqlx::query_scalar::<_, i64>("SELECT id FROM users WHERE lower(email) = lower($1)")
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
            HttpResponse::NotFound().json(ErrorResponse {
                error: "user_not_found".into(),
                details: None,
            })
        })
}

#[utoipa::path(
    get,
    path = "/events/{event_id}/my-qr-code",
    tag = "qr_codes",
    responses(
        (status = 200, description = "QR code token généré ou récupéré", body = QRCodeGenerateResponse),
        (status = 403, description = "Non autorisé ou invitation invalide", body = ErrorResponse),
        (status = 404, description = "Événement introuvable", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Identifiant de l'événement")
    )
)]
#[get("/events/{event_id}/my-qr-code")]
pub async fn generate_my_qr_code(
    state: web::Data<AppState>,
    req: HttpRequest,
    event_id: web::Path<i64>,
) -> impl Responder {
    let requester_email = match claims_email(&req, state.get_ref()) {
        Ok(e) => e,
        Err(resp) => return resp,
    };
    if let Err(resp) = ensure_event_writable(&state.db, *event_id).await {
        return resp;
    }

    let user_id = match get_user_id_from_email(&state.db, &requester_email).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    // Check if user has a valid invitation (Accepted or Waiting)
    let invitation_status = sqlx::query_scalar::<_, String>(
        "SELECT status FROM invitations WHERE event_id = $1 AND user_id = $2",
    )
    .bind(*event_id)
    .bind(user_id)
    .fetch_optional(&state.db)
    .await;

    let invitation_status = match invitation_status {
        Ok(Some(status)) => status,
        Ok(None) => {
            return HttpResponse::Forbidden().json(ErrorResponse {
                error: "not_invited".into(),
                details: Some("Vous n'êtes pas invité à cet événement".into()),
            });
        }
        Err(_) => {
            return HttpResponse::InternalServerError().json(ErrorResponse {
                error: "db_error".into(),
                details: None,
            });
        }
    };

    // Reject if invitation is declined or expired
    if invitation_status == "Declined" || invitation_status == "Expired" {
        return HttpResponse::Forbidden().json(ErrorResponse {
            error: "invitation_invalid".into(),
            details: Some(format!("Votre invitation est: {}", invitation_status)),
        });
    }

    // Check if QR code already exists for this user-event combination
    let existing_qr = sqlx::query(
        "SELECT qr_token, generated_at FROM event_checkins WHERE event_id = $1 AND user_id = $2",
    )
    .bind(*event_id)
    .bind(user_id)
    .fetch_optional(&state.db)
    .await;

    match existing_qr {
        Ok(Some(row)) => {
            let token: Uuid = row.try_get("qr_token").unwrap();
            let generated_at: chrono::DateTime<chrono::Utc> = row.try_get("generated_at").unwrap();

            HttpResponse::Ok().json(QRCodeGenerateResponse {
                qr_token: token.to_string(),
                event_id: *event_id,
                generated_at,
            })
        }
        Ok(None) => {
            // Generate new QR code
            let new_token = Uuid::new_v4();
            let insert_result = sqlx::query(
                "INSERT INTO event_checkins (qr_token, event_id, user_id) VALUES ($1, $2, $3) RETURNING generated_at",
            )
            .bind(new_token)
            .bind(*event_id)
            .bind(user_id)
            .fetch_one(&state.db)
            .await;

            match insert_result {
                Ok(row) => {
                    let generated_at: chrono::DateTime<chrono::Utc> =
                        row.try_get("generated_at").unwrap();
                    HttpResponse::Ok().json(QRCodeGenerateResponse {
                        qr_token: new_token.to_string(),
                        event_id: *event_id,
                        generated_at,
                    })
                }
                Err(e) => {
                    error!("Failed to generate QR code: {}", e);
                    HttpResponse::InternalServerError().json(ErrorResponse {
                        error: "db_error".into(),
                        details: None,
                    })
                }
            }
        }
        Err(e) => {
            error!("Database error checking existing QR: {}", e);
            HttpResponse::InternalServerError().json(ErrorResponse {
                error: "db_error".into(),
                details: None,
            })
        }
    }
}

#[utoipa::path(
    post,
    path = "/events/{event_id}/scan-qr",
    tag = "qr_codes",
    request_body = QRCodeScanPayload,
    responses(
        (status = 200, description = "QR code scanné avec succès", body = QRCodeScanResponse),
        (status = 400, description = "Token invalide", body = ErrorResponse),
        (status = 403, description = "Non autorisé", body = ErrorResponse),
        (status = 404, description = "QR code introuvable", body = ErrorResponse),
        (status = 409, description = "QR code déjà scanné", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Identifiant de l'événement")
    )
)]
#[post("/events/{event_id}/scan-qr")]
pub async fn scan_qr_code(
    state: web::Data<AppState>,
    req: HttpRequest,
    event_id: web::Path<i64>,
    payload: web::Json<QRCodeScanPayload>,
) -> impl Responder {
    let scanner_email = match ensure_event_owner(&req, state.get_ref(), *event_id).await {
        Ok(email) => email,
        Err(resp) => return resp,
    };
    if let Err(resp) = ensure_event_writable(&state.db, *event_id).await {
        return resp;
    }

    // Parse the UUID token
    let qr_token = match Uuid::parse_str(&payload.token) {
        Ok(uuid) => uuid,
        Err(_) => {
            return HttpResponse::BadRequest().json(ErrorResponse {
                error: "invalid_token".into(),
                details: Some("Format de token invalide".into()),
            });
        }
    };

    // Start a transaction to ensure atomicity
    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(_) => {
            return HttpResponse::InternalServerError().json(ErrorResponse {
                error: "db_error".into(),
                details: None,
            });
        }
    };

    // Fetch the QR code details with user info
    let qr_details = sqlx::query(
        "SELECT ec.event_id, ec.user_id, ec.scanned_at, ec.is_valid,
                u.email, u.handle, u.avatar_url,
                i.status as invitation_status
         FROM event_checkins ec
         JOIN users u ON u.id = ec.user_id
         LEFT JOIN invitations i ON i.event_id = ec.event_id AND i.user_id = ec.user_id
         WHERE ec.qr_token = $1",
    )
    .bind(qr_token)
    .fetch_optional(&mut *tx)
    .await;

    let row = match qr_details {
        Ok(Some(row)) => row,
        Ok(None) => {
            let _ = tx.rollback().await;
            return HttpResponse::NotFound().json(ErrorResponse {
                error: "qr_not_found".into(),
                details: Some("QR code introuvable".into()),
            });
        }
        Err(e) => {
            error!("Database error fetching QR details: {}", e);
            let _ = tx.rollback().await;
            return HttpResponse::InternalServerError().json(ErrorResponse {
                error: "db_error".into(),
                details: None,
            });
        }
    };

    let qr_event_id: i64 = row.try_get("event_id").unwrap();
    let is_valid: bool = row.try_get("is_valid").unwrap();
    let scanned_at: Option<chrono::DateTime<chrono::Utc>> = row.try_get("scanned_at").ok();
    let user_email: String = row.try_get("email").unwrap();
    let user_handle: String = row.try_get("handle").unwrap();
    let user_avatar_url: Option<String> = row.try_get("avatar_url").ok();
    let invitation_status: Option<String> = row.try_get("invitation_status").ok();

    // Verify the QR code belongs to this event
    if qr_event_id != *event_id {
        let _ = tx.rollback().await;
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "wrong_event".into(),
            details: Some("Ce QR code n'appartient pas à cet événement".into()),
        });
    }

    // Check if invitation is still valid
    if let Some(ref status) = invitation_status
        && (status == "Declined" || status == "Expired")
    {
        let _ = tx.rollback().await;
        return HttpResponse::Forbidden().json(QRCodeScanResponse {
            success: false,
            status: "invitation_invalid".into(),
            user_email: Some(user_email),
            user_handle: Some(user_handle),
            user_avatar_url,
            scanned_at: None,
            message: format!("L'invitation de cet utilisateur est: {}", status),
        });
    }

    // Check if manually invalidated
    if !is_valid {
        let _ = tx.rollback().await;
        return HttpResponse::Forbidden().json(QRCodeScanResponse {
            success: false,
            status: "qr_invalidated".into(),
            user_email: Some(user_email),
            user_handle: Some(user_handle),
            user_avatar_url,
            scanned_at: None,
            message: "Ce QR code a été invalidé".into(),
        });
    }

    // Check if already scanned
    if scanned_at.is_some() {
        let _ = tx.rollback().await;
        return HttpResponse::Conflict().json(QRCodeScanResponse {
            success: false,
            status: "already_scanned".into(),
            user_email: Some(user_email.clone()),
            user_handle: Some(user_handle.clone()),
            user_avatar_url: user_avatar_url.clone(),
            scanned_at,
            message: format!("{} a déjà été enregistré", user_email),
        });
    }

    // Mark as scanned
    let now = chrono::Utc::now();
    let update_result = sqlx::query(
        "UPDATE event_checkins SET scanned_at = $1, scanned_by_email = $2 WHERE qr_token = $3",
    )
    .bind(now)
    .bind(&scanner_email)
    .bind(qr_token)
    .execute(&mut *tx)
    .await;

    if let Err(e) = update_result {
        error!("Failed to update scan status: {}", e);
        let _ = tx.rollback().await;
        return HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        });
    }

    if (tx.commit().await).is_err() {
        return HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        });
    }

    HttpResponse::Ok().json(QRCodeScanResponse {
        success: true,
        status: "scanned".into(),
        user_email: Some(user_email.clone()),
        user_handle: Some(user_handle),
        user_avatar_url,
        scanned_at: Some(now),
        message: format!("✓ {} enregistré avec succès", user_email),
    })
}

#[utoipa::path(
    get,
    path = "/events/{event_id}/qr-scan-stats",
    tag = "qr_codes",
    responses(
        (status = 200, description = "Statistiques de scan", body = QRCodeStatsResponse),
        (status = 403, description = "Non autorisé", body = ErrorResponse),
        (status = 404, description = "Événement introuvable", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Identifiant de l'événement")
    )
)]
#[get("/events/{event_id}/qr-scan-stats")]
pub async fn get_qr_scan_stats(
    state: web::Data<AppState>,
    req: HttpRequest,
    event_id: web::Path<i64>,
) -> impl Responder {
    if let Err(resp) = ensure_event_owner(&req, state.get_ref(), *event_id).await {
        return resp;
    }

    // Count total invitations (excluding Declined and Expired)
    let total_invited = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM invitations 
         WHERE event_id = $1 AND status NOT IN ('Declined', 'Expired')",
    )
    .bind(*event_id)
    .fetch_one(&state.db)
    .await;

    // Count checked-in users
    let total_checked_in = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM event_checkins 
         WHERE event_id = $1 AND scanned_at IS NOT NULL",
    )
    .bind(*event_id)
    .fetch_one(&state.db)
    .await;

    match (total_invited, total_checked_in) {
        (Ok(invited), Ok(checked_in)) => {
            let pending = invited - checked_in;
            HttpResponse::Ok().json(QRCodeStatsResponse {
                total_invited: invited,
                total_checked_in: checked_in,
                pending_checkins: pending.max(0),
            })
        }
        _ => HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        }),
    }
}
