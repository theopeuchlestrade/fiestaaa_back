use actix_web::http::header::{CACHE_CONTROL, CONTENT_TYPE, PRAGMA};
use actix_web::{HttpRequest, HttpResponse, Responder, post, web};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use log::{error, warn};
use serde::Deserialize;
use serde_json::json;
use sqlx::{Error, PgPool, Row};
use uuid::Uuid;

use crate::{
    auth::{
        build_cleared_session_cookie, build_session_cookie, encode_jwt, fetch_user_auth,
        hash_password, now_ts, random_password_token, revoke_auth_token_from_request,
        validate_password_strength, verify_password,
    },
    handles::{generate_unique_handle, handle_available, is_valid_handle, normalize_handle},
    models::{
        AppleClaims, Claims, CompleteRegistrationPayload, ErrorResponse, LoginPayload,
        OAuthPayload, RegisterPayload, StatusResponse, TokenResponse, VerifyEmailPayload,
    },
    security::{normalize_email, sha256_hex},
    state::AppState,
};

const EMAIL_VERIFICATION_TTL_HOURS: i64 = 24;

#[derive(Debug, Clone, sqlx::FromRow)]
struct PendingRegistrationRow {
    email: String,
    verification_expires_at: DateTime<Utc>,
}

#[derive(Deserialize)]
pub struct OAuthPath {
    provider: String,
}

fn auth_cookie_is_secure(state: &AppState) -> bool {
    state.app_base_url.starts_with("https://")
}

fn auth_rate_limit_remote(req: &HttpRequest, state: &AppState) -> String {
    if state.trust_proxy_headers {
        return req
            .connection_info()
            .realip_remote_addr()
            .unwrap_or("unknown")
            .to_string();
    }

    req.peer_addr()
        .map(|addr| addr.ip().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

async fn enforce_auth_rate_limit(
    req: &HttpRequest,
    state: &AppState,
    scope: &str,
) -> Result<(), HttpResponse> {
    let remote = auth_rate_limit_remote(req, state);
    let key = format!("auth:{scope}:{remote}");
    if state.auth_rate_limiter.allow(&key).await {
        Ok(())
    } else {
        Err(HttpResponse::TooManyRequests().json(ErrorResponse {
            error: "rate_limited".into(),
            details: Some("too many authentication attempts".into()),
        }))
    }
}

fn token_response(
    token: String,
    public_id: String,
    email: String,
    handle: String,
    secure_cookie: bool,
    include_body_token: bool,
) -> HttpResponse {
    HttpResponse::Ok()
        .insert_header((CONTENT_TYPE, "application/json"))
        .insert_header((CACHE_CONTROL, "no-store"))
        .insert_header((PRAGMA, "no-cache"))
        .cookie(build_session_cookie(&token, secure_cookie))
        .json(TokenResponse {
            token: if include_body_token {
                token
            } else {
                String::new()
            },
            public_id,
            email,
            handle,
        })
}

fn auth_response_includes_body_token(req: &HttpRequest) -> bool {
    !req.headers().contains_key("Origin") && !req.headers().contains_key("Sec-Fetch-Site")
}

async fn cleanup_expired_pending_registrations(db: &PgPool) -> Result<(), HttpResponse> {
    sqlx::query("DELETE FROM pending_registrations WHERE verification_expires_at < NOW()")
        .execute(db)
        .await
        .map_err(|_| {
            HttpResponse::InternalServerError().json(ErrorResponse {
                error: "db_error".into(),
                details: None,
            })
        })?;
    Ok(())
}

async fn pending_handle_exists(db: &PgPool, handle: &str) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(
            SELECT 1
            FROM pending_registrations
            WHERE lower(handle) = lower($1)
        )",
    )
    .bind(handle)
    .fetch_one(db)
    .await
}

async fn registration_handle_available(db: &PgPool, handle: &str) -> Result<bool, sqlx::Error> {
    Ok(handle_available(db, handle).await? && !pending_handle_exists(db, handle).await?)
}

async fn generate_registration_handle(state: &AppState) -> Result<String, HttpResponse> {
    for _ in 0..32 {
        let handle = match generate_unique_handle(&state.db).await {
            Ok(value) => value,
            Err(_) => {
                return Err(HttpResponse::InternalServerError().json(ErrorResponse {
                    error: "handle_generation_failed".into(),
                    details: None,
                }));
            }
        };

        match registration_handle_available(&state.db, &handle).await {
            Ok(true) => return Ok(handle),
            Ok(false) => continue,
            Err(_) => {
                return Err(HttpResponse::InternalServerError().json(ErrorResponse {
                    error: "db_error".into(),
                    details: None,
                }));
            }
        }
    }

    Err(HttpResponse::InternalServerError().json(ErrorResponse {
        error: "handle_generation_failed".into(),
        details: None,
    }))
}

fn pending_placeholder_hash() -> Result<String, HttpResponse> {
    hash_password(&random_password_token()).map_err(|_| {
        HttpResponse::InternalServerError().json(ErrorResponse {
            error: "hash_failed".into(),
            details: None,
        })
    })
}

async fn resolve_final_handle(
    state: &AppState,
    requested_handle: Option<&str>,
) -> Result<String, HttpResponse> {
    if let Some(raw_handle) = requested_handle {
        let normalized = normalize_handle(raw_handle).normalized;
        if !is_valid_handle(&normalized) {
            return Err(HttpResponse::BadRequest().json(ErrorResponse {
                error: "invalid_handle".into(),
                details: Some("format attendu: 4-32 chars [a-z0-9._-]".into()),
            }));
        }
        return match handle_available(&state.db, &normalized).await {
            Ok(true) => Ok(normalized),
            Ok(false) => Err(HttpResponse::Conflict().json(ErrorResponse {
                error: "handle_taken".into(),
                details: None,
            })),
            Err(_) => Err(HttpResponse::InternalServerError().json(ErrorResponse {
                error: "db_error".into(),
                details: None,
            })),
        };
    }

    generate_unique_handle(&state.db).await.map_err(|_| {
        HttpResponse::InternalServerError().json(ErrorResponse {
            error: "handle_generation_failed".into(),
            details: None,
        })
    })
}

fn build_email_verification_link(base_url: &str, token: Uuid) -> String {
    let trimmed = base_url.trim_end_matches('/');
    if trimmed.contains('?') {
        format!("{trimmed}&verifyEmailToken={token}")
    } else {
        format!("{trimmed}?verifyEmailToken={token}")
    }
}

fn escape_html(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

async fn send_verification_email(
    state: &AppState,
    email: &str,
    verification_link: &str,
) -> Result<bool, HttpResponse> {
    let Some(sender) = state.invitation_email_sender.as_ref() else {
        warn!("Registration kept pending because INVITATION_EMAIL_SENDER is missing");
        return Ok(false);
    };
    let Some(api_key) = state.invitation_email_api_key.as_ref() else {
        warn!("Registration kept pending because RESEND_API_KEY is missing");
        return Ok(false);
    };

    let subject = "Verify your Fiestaaa email address";
    let text = format!(
        "Welcome to Fiestaaa.\n\nVerify your email address by opening this link:\n{verification_link}\n\nThis link expires in {EMAIL_VERIFICATION_TTL_HOURS} hours."
    );
    let html = format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <title>Verify your Fiestaaa email</title>
</head>
<body style="margin:0;padding:24px;background:#f8fafc;font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;color:#0f172a;">
  <div style="max-width:560px;margin:0 auto;background:#ffffff;border-radius:16px;padding:28px;border:1px solid #e2e8f0;">
    <h1 style="margin:0 0 12px;font-size:24px;">Verify your email address</h1>
    <p style="margin:0 0 16px;color:#475569;">Finish creating your Fiestaaa account by confirming that you control this email address.</p>
    <p style="margin:0 0 20px;">
      <a href="{verification_link_html}" style="display:inline-block;padding:14px 20px;background:#0f172a;color:#ffffff;text-decoration:none;border-radius:10px;font-weight:700;">Verify email</a>
    </p>
    <p style="margin:0;color:#64748b;font-size:13px;">If the button does not work, copy and paste this link into your browser:</p>
    <p style="margin:8px 0 0;font-size:13px;word-break:break-all;"><a href="{verification_link_html}" style="color:#0f172a;">{verification_link_html}</a></p>
    <p style="margin:20px 0 0;color:#64748b;font-size:13px;">This link expires in {ttl_hours} hours.</p>
  </div>
</body>
</html>"#,
        verification_link_html = escape_html(verification_link),
        ttl_hours = EMAIL_VERIFICATION_TTL_HOURS,
    );

    let payload = json!({
        "from": sender,
        "to": [email],
        "subject": subject,
        "text": text,
        "html": html
    });

    let response = state
        .http_client
        .post("https://api.resend.com/emails")
        .bearer_auth(api_key)
        .json(&payload)
        .send()
        .await;

    match response {
        Ok(resp) if resp.status().is_success() => Ok(true),
        Ok(resp) => {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            warn!("verification email provider failure: status {status}, body: {body}");
            Err(HttpResponse::BadGateway().json(ErrorResponse {
                error: "email_send_failed".into(),
                details: Some(format!("provider status {status}")),
            }))
        }
        Err(err) => {
            error!("verification email transport failure: {err}");
            Err(HttpResponse::BadGateway().json(ErrorResponse {
                error: "email_send_failed".into(),
                details: Some("transport_error".into()),
            }))
        }
    }
}

#[utoipa::path(
    post,
    path = "/auth/register",
    tag = "auth",
    request_body = RegisterPayload,
    responses(
        (status = 201, description = "Registration pending email verification", body = StatusResponse),
        (status = 400, description = "Invalid payload", body = ErrorResponse),
        (status = 429, description = "Too many attempts", body = ErrorResponse),
        (status = 500, description = "Database or hashing error", body = ErrorResponse)
    )
)]
#[post("/auth/register")]
pub async fn register(
    req: HttpRequest,
    state: web::Data<AppState>,
    payload: web::Json<RegisterPayload>,
) -> impl Responder {
    if let Err(resp) = enforce_auth_rate_limit(&req, state.get_ref(), "register").await {
        return resp;
    }

    let email = normalize_email(&payload.email);

    if email.is_empty() {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("email requis".into()),
        });
    }
    if let Err(resp) = cleanup_expired_pending_registrations(&state.db).await {
        return resp;
    }
    match fetch_user_by_email(&state.db, &email).await {
        Ok(Some(_)) => {
            return HttpResponse::Created().json(StatusResponse {
                status: "verification_pending".into(),
            });
        }
        Ok(None) => {}
        Err(_) => {
            return HttpResponse::InternalServerError().json(ErrorResponse {
                error: "db_error".into(),
                details: None,
            });
        }
    }

    let verification_token = Uuid::new_v4();
    let verification_token_hash = sha256_hex(&verification_token.to_string());
    let verification_expires_at = Utc::now() + ChronoDuration::hours(EMAIL_VERIFICATION_TTL_HOURS);
    let verification_link = build_email_verification_link(&state.app_base_url, verification_token);

    let mut tx = match state.db.begin().await {
        Ok(value) => value,
        Err(_) => {
            return HttpResponse::InternalServerError().json(ErrorResponse {
                error: "db_error".into(),
                details: None,
            });
        }
    };

    let pending_exists = match sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(
            SELECT 1
            FROM pending_registrations
            WHERE fiestaaa_email_matches(email_lookup_hash, $1)
        )",
    )
    .bind(&email)
    .fetch_one(&mut *tx)
    .await
    {
        Ok(value) => value,
        Err(_) => {
            let _ = tx.rollback().await;
            return HttpResponse::InternalServerError().json(ErrorResponse {
                error: "db_error".into(),
                details: None,
            });
        }
    };

    if pending_exists {
        if tx.rollback().await.is_err() {
            return HttpResponse::InternalServerError().json(ErrorResponse {
                error: "db_error".into(),
                details: None,
            });
        }
        return HttpResponse::Created().json(StatusResponse {
            status: "verification_pending".into(),
        });
    }

    let handle = match generate_registration_handle(state.get_ref()).await {
        Ok(handle) => handle,
        Err(resp) => {
            let _ = tx.rollback().await;
            return resp;
        }
    };
    let hash = match pending_placeholder_hash() {
        Ok(hash) => hash,
        Err(resp) => {
            let _ = tx.rollback().await;
            return resp;
        }
    };

    let res = sqlx::query(
        "INSERT INTO pending_registrations (
            email_ciphertext,
            email_lookup_hash,
            password_hash,
            handle,
            verification_token_hash,
            verification_expires_at
         ) VALUES (
            fiestaaa_encrypt_text($1),
            fiestaaa_email_lookup($1),
            $2,
            $3,
            $4,
            $5
         )",
    )
    .bind(&email)
    .bind(&hash)
    .bind(&handle)
    .bind(&verification_token_hash)
    .bind(verification_expires_at)
    .execute(&mut *tx)
    .await;

    match res {
        Ok(_) => {
            if tx.commit().await.is_err() {
                return HttpResponse::InternalServerError().json(ErrorResponse {
                    error: "db_error".into(),
                    details: None,
                });
            }

            match send_verification_email(state.get_ref(), &email, &verification_link).await {
                Ok(email_sent) => HttpResponse::Created().json(StatusResponse {
                    status: if email_sent {
                        "verification_email_sent".into()
                    } else {
                        "verification_pending".into()
                    },
                }),
                Err(resp) => resp,
            }
        }
        Err(e) => {
            let _ = tx.rollback().await;
            match e {
                Error::Database(db_err) if db_err.code().as_deref() == Some("23505") => {
                    let constraint = db_err.constraint().unwrap_or_default();
                    let error = if constraint.contains("handle") {
                        "handle_taken"
                    } else {
                        "email_taken"
                    };
                    HttpResponse::Conflict().json(ErrorResponse {
                        error: error.into(),
                        details: None,
                    })
                }
                _ => HttpResponse::InternalServerError().json(ErrorResponse {
                    error: "db_error".into(),
                    details: None,
                }),
            }
        }
    }
}

#[utoipa::path(
    post,
    path = "/auth/verify-email",
    tag = "auth",
    request_body = VerifyEmailPayload,
    responses(
        (status = 200, description = "Email verified and registration can be completed", body = StatusResponse),
        (status = 400, description = "Invalid token", body = ErrorResponse),
        (status = 410, description = "Expired token", body = ErrorResponse),
        (status = 500, description = "Database error", body = ErrorResponse)
    )
)]
#[post("/auth/verify-email")]
pub async fn verify_email(
    state: web::Data<AppState>,
    payload: web::Json<VerifyEmailPayload>,
) -> impl Responder {
    let token = payload.token.trim();
    if token.is_empty() {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_token".into(),
            details: Some("token requis".into()),
        });
    }

    let verification_token = match Uuid::parse_str(token) {
        Ok(value) => value,
        Err(_) => {
            return HttpResponse::BadRequest().json(ErrorResponse {
                error: "invalid_token".into(),
                details: Some("format de token invalide".into()),
            });
        }
    };
    let verification_token_hash = sha256_hex(&verification_token.to_string());

    let mut tx = match state.db.begin().await {
        Ok(value) => value,
        Err(_) => {
            return HttpResponse::InternalServerError().json(ErrorResponse {
                error: "db_error".into(),
                details: None,
            });
        }
    };

    let pending = match sqlx::query_as::<_, PendingRegistrationRow>(
        "SELECT fiestaaa_decrypt_text(email_ciphertext) AS email, verification_expires_at
         FROM pending_registrations
         WHERE verification_token_hash = $1
         FOR UPDATE",
    )
    .bind(&verification_token_hash)
    .fetch_optional(&mut *tx)
    .await
    {
        Ok(row) => row,
        Err(_) => {
            let _ = tx.rollback().await;
            return HttpResponse::InternalServerError().json(ErrorResponse {
                error: "db_error".into(),
                details: None,
            });
        }
    };

    let Some(pending) = pending else {
        let _ = tx.rollback().await;
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_token".into(),
            details: Some("token inconnu".into()),
        });
    };

    if pending.verification_expires_at < Utc::now() {
        let _ = sqlx::query(
            "DELETE FROM pending_registrations WHERE fiestaaa_email_matches(email_lookup_hash, $1)",
        )
        .bind(&pending.email)
        .execute(&mut *tx)
        .await;
        let _ = tx.commit().await;
        return HttpResponse::Gone().json(ErrorResponse {
            error: "expired_token".into(),
            details: Some("ce lien de verification a expire".into()),
        });
    }

    match fetch_user_by_email(&state.db, &pending.email).await {
        Ok(Some(_)) => {
            let _ = sqlx::query(
                "DELETE FROM pending_registrations WHERE fiestaaa_email_matches(email_lookup_hash, $1)",
            )
                .bind(&pending.email)
                .execute(&mut *tx)
                .await;
            if tx.commit().await.is_err() {
                return HttpResponse::InternalServerError().json(ErrorResponse {
                    error: "db_error".into(),
                    details: None,
                });
            }
            return HttpResponse::Ok().json(StatusResponse {
                status: "already_verified".into(),
            });
        }
        Ok(None) => {}
        Err(_) => {
            let _ = tx.rollback().await;
            return HttpResponse::InternalServerError().json(ErrorResponse {
                error: "db_error".into(),
                details: None,
            });
        }
    }

    if tx.rollback().await.is_err() {
        return HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        });
    }

    HttpResponse::Ok().json(StatusResponse {
        status: "setup_required".into(),
    })
}

#[utoipa::path(
    post,
    path = "/auth/complete-registration",
    tag = "auth",
    request_body = CompleteRegistrationPayload,
    responses(
        (status = 200, description = "Registration completed", body = TokenResponse),
        (status = 400, description = "Invalid payload or token", body = ErrorResponse),
        (status = 409, description = "Email or handle already taken", body = ErrorResponse),
        (status = 410, description = "Expired token", body = ErrorResponse),
        (status = 500, description = "Database or token creation error", body = ErrorResponse)
    )
)]
#[post("/auth/complete-registration")]
pub async fn complete_registration(
    req: HttpRequest,
    state: web::Data<AppState>,
    payload: web::Json<CompleteRegistrationPayload>,
) -> impl Responder {
    let token = payload.token.trim();
    if token.is_empty() {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_token".into(),
            details: Some("token requis".into()),
        });
    }
    if let Err(reason) = validate_password_strength(&payload.password) {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "weak_password".into(),
            details: Some(reason.into()),
        });
    }

    let verification_token = match Uuid::parse_str(token) {
        Ok(value) => value,
        Err(_) => {
            return HttpResponse::BadRequest().json(ErrorResponse {
                error: "invalid_token".into(),
                details: Some("format de token invalide".into()),
            });
        }
    };
    let verification_token_hash = sha256_hex(&verification_token.to_string());

    let mut tx = match state.db.begin().await {
        Ok(value) => value,
        Err(_) => {
            return HttpResponse::InternalServerError().json(ErrorResponse {
                error: "db_error".into(),
                details: None,
            });
        }
    };

    let pending = match sqlx::query_as::<_, PendingRegistrationRow>(
        "SELECT fiestaaa_decrypt_text(email_ciphertext) AS email, verification_expires_at
         FROM pending_registrations
         WHERE verification_token_hash = $1
         FOR UPDATE",
    )
    .bind(&verification_token_hash)
    .fetch_optional(&mut *tx)
    .await
    {
        Ok(row) => row,
        Err(_) => {
            let _ = tx.rollback().await;
            return HttpResponse::InternalServerError().json(ErrorResponse {
                error: "db_error".into(),
                details: None,
            });
        }
    };

    let Some(pending) = pending else {
        let _ = tx.rollback().await;
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_token".into(),
            details: Some("token inconnu".into()),
        });
    };

    if pending.verification_expires_at < Utc::now() {
        let _ = sqlx::query(
            "DELETE FROM pending_registrations WHERE fiestaaa_email_matches(email_lookup_hash, $1)",
        )
        .bind(&pending.email)
        .execute(&mut *tx)
        .await;
        let _ = tx.commit().await;
        return HttpResponse::Gone().json(ErrorResponse {
            error: "expired_token".into(),
            details: Some("ce lien de verification a expire".into()),
        });
    }

    match fetch_user_by_email(&state.db, &pending.email).await {
        Ok(Some(_)) => {
            let _ = sqlx::query(
                "DELETE FROM pending_registrations WHERE fiestaaa_email_matches(email_lookup_hash, $1)",
            )
                .bind(&pending.email)
                .execute(&mut *tx)
                .await;
            let _ = tx.commit().await;
            return HttpResponse::Conflict().json(ErrorResponse {
                error: "email_taken".into(),
                details: None,
            });
        }
        Ok(None) => {}
        Err(_) => {
            let _ = tx.rollback().await;
            return HttpResponse::InternalServerError().json(ErrorResponse {
                error: "db_error".into(),
                details: None,
            });
        }
    }

    let final_handle = match resolve_final_handle(
        state.get_ref(),
        payload
            .handle
            .as_deref()
            .filter(|value| !value.trim().is_empty()),
    )
    .await
    {
        Ok(handle) => handle,
        Err(resp) => {
            let _ = tx.rollback().await;
            return resp;
        }
    };
    let password_hash = match hash_password(&payload.password) {
        Ok(hash) => hash,
        Err(_) => {
            let _ = tx.rollback().await;
            return HttpResponse::InternalServerError().json(ErrorResponse {
                error: "hash_failed".into(),
                details: None,
            });
        }
    };

    let inserted_user = sqlx::query(
        "INSERT INTO users (email_ciphertext, email_lookup_hash, password_hash, handle)
         VALUES (fiestaaa_encrypt_text($1), fiestaaa_email_lookup($1), $2, $3)
         RETURNING public_id::text AS public_id",
    )
    .bind(&pending.email)
    .bind(&password_hash)
    .bind(&final_handle)
    .fetch_one(&mut *tx)
    .await;

    let inserted_user = match inserted_user {
        Ok(row) => row,
        Err(err) => {
            let _ = tx.rollback().await;
            return match err {
                Error::Database(db_err) if db_err.code().as_deref() == Some("23505") => {
                    let constraint = db_err.constraint().unwrap_or_default();
                    HttpResponse::Conflict().json(ErrorResponse {
                        error: if constraint.contains("handle") {
                            "handle_taken".into()
                        } else {
                            "email_taken".into()
                        },
                        details: None,
                    })
                }
                _ => HttpResponse::InternalServerError().json(ErrorResponse {
                    error: "db_error".into(),
                    details: None,
                }),
            };
        }
    };

    if sqlx::query(
        "DELETE FROM pending_registrations WHERE fiestaaa_email_matches(email_lookup_hash, $1)",
    )
    .bind(&pending.email)
    .execute(&mut *tx)
    .await
    .is_err()
    {
        let _ = tx.rollback().await;
        return HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        });
    }

    if tx.commit().await.is_err() {
        return HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        });
    }

    let exp = (now_ts() + 24 * 3600) as usize;
    let claims = Claims {
        sub: inserted_user.get::<String, _>("public_id"),
        handle: final_handle.clone(),
        exp,
    };
    let secure_cookie = auth_cookie_is_secure(state.get_ref());

    match encode_jwt(&claims, &state.jwt_secret) {
        Ok(token) => token_response(
            token,
            claims.sub.clone(),
            pending.email,
            final_handle,
            secure_cookie,
            auth_response_includes_body_token(&req),
        ),
        Err(_) => HttpResponse::InternalServerError().json(ErrorResponse {
            error: "token_creation_failed".into(),
            details: None,
        }),
    }
}

#[utoipa::path(
    post,
    path = "/auth/oauth/{provider}",
    tag = "auth",
    params(
        ("provider" = String, Path, description = "oauth provider (google)")
    ),
    request_body = OAuthPayload,
    responses(
        (status = 200, description = "Valid provider token", body = TokenResponse),
        (status = 400, description = "Invalid payload or provider", body = ErrorResponse),
        (status = 401, description = "Invalid provider token", body = ErrorResponse),
        (status = 500, description = "Database or token creation error", body = ErrorResponse)
    )
)]
#[post("/auth/oauth/{provider}")]
pub async fn oauth_login(
    req: HttpRequest,
    state: web::Data<AppState>,
    path: web::Path<OAuthPath>,
    payload: web::Json<OAuthPayload>,
) -> HttpResponse {
    let provider = path.into_inner().provider.to_lowercase();
    if let Err(resp) =
        enforce_auth_rate_limit(&req, state.get_ref(), &format!("oauth:{provider}")).await
    {
        return resp;
    }
    match provider.as_str() {
        "google" => oauth_google(req, state, payload.into_inner()).await,
        "apple" => oauth_apple(req, state, payload.into_inner()).await,
        _ => HttpResponse::BadRequest().json(ErrorResponse {
            error: "unsupported_provider".into(),
            details: Some("provider must be 'google' ou 'apple'".into()),
        }),
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct OAuthUserRow {
    id: i64,
    public_id: Uuid,
    email: String,
    handle: String,
}

fn normalize_provider_email(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_lowercase())
}

fn json_truthy(value: Option<&serde_json::Value>) -> bool {
    match value {
        Some(serde_json::Value::Bool(flag)) => *flag,
        Some(serde_json::Value::String(flag)) => flag.eq_ignore_ascii_case("true"),
        _ => false,
    }
}

fn google_subject(token_info: &serde_json::Value) -> Option<String> {
    token_info
        .get("sub")
        .or_else(|| token_info.get("user_id"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

async fn fetch_google_userinfo(
    state: &web::Data<AppState>,
    access_token: &str,
) -> Option<serde_json::Value> {
    let response = state
        .http_client
        .get("https://www.googleapis.com/oauth2/v3/userinfo")
        .bearer_auth(access_token)
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.json::<serde_json::Value>().await.ok()
}

async fn fetch_oauth_user_by_identity(
    db: &PgPool,
    provider: &str,
    provider_subject: &str,
) -> Result<Option<OAuthUserRow>, sqlx::Error> {
    sqlx::query_as::<_, OAuthUserRow>(
        "SELECT u.id,
                u.public_id,
                fiestaaa_decrypt_text(u.email_ciphertext) AS email,
                u.handle
         FROM oauth_identities oi
         JOIN users u ON u.id = oi.user_id
         WHERE oi.provider = $1
           AND fiestaaa_lookup_matches(oi.provider_subject_lookup_hash, $2)",
    )
    .bind(provider)
    .bind(provider_subject)
    .fetch_optional(db)
    .await
}

async fn touch_oauth_identity(
    db: &PgPool,
    provider: &str,
    provider_subject: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE oauth_identities
         SET last_login_at = NOW()
         WHERE provider = $1
           AND fiestaaa_lookup_matches(provider_subject_lookup_hash, $2)",
    )
    .bind(provider)
    .bind(provider_subject)
    .execute(db)
    .await?;
    Ok(())
}

async fn fetch_user_by_email(
    db: &PgPool,
    email: &str,
) -> Result<Option<OAuthUserRow>, sqlx::Error> {
    sqlx::query_as::<_, OAuthUserRow>(
        "SELECT id,
                public_id,
                fiestaaa_decrypt_text(email_ciphertext) AS email,
                handle
         FROM users
         WHERE fiestaaa_email_matches(email_lookup_hash, $1)",
    )
    .bind(email)
    .fetch_optional(db)
    .await
}

async fn create_oauth_user(
    state: &web::Data<AppState>,
    email: &str,
) -> Result<OAuthUserRow, HttpResponse> {
    let new_handle = match generate_unique_handle(&state.db).await {
        Ok(handle) => handle,
        Err(_) => {
            return Err(HttpResponse::InternalServerError().json(ErrorResponse {
                error: "handle_generation_failed".into(),
                details: None,
            }));
        }
    };
    let password = random_password_token();
    let hash = match hash_password(&password) {
        Ok(hash) => hash,
        Err(_) => {
            return Err(HttpResponse::InternalServerError().json(ErrorResponse {
                error: "hash_failed".into(),
                details: None,
            }));
        }
    };

    let inserted = sqlx::query_as::<_, OAuthUserRow>(
        "INSERT INTO users (email_ciphertext, email_lookup_hash, handle, password_hash)
         VALUES (fiestaaa_encrypt_text($1), fiestaaa_email_lookup($1), $2, $3)
         RETURNING id,
                   public_id,
                   fiestaaa_decrypt_text(email_ciphertext) AS email,
                   handle",
    )
    .bind(email)
    .bind(&new_handle)
    .bind(&hash)
    .fetch_one(&state.db)
    .await;

    match inserted {
        Ok(user) => Ok(user),
        Err(Error::Database(db_err)) if db_err.code().as_deref() == Some("23505") => {
            match fetch_user_by_email(&state.db, email).await {
                Ok(Some(user)) => Ok(user),
                _ => Err(HttpResponse::InternalServerError().json(ErrorResponse {
                    error: "db_error".into(),
                    details: None,
                })),
            }
        }
        Err(_) => Err(HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        })),
    }
}

async fn resolve_oauth_user(
    state: &web::Data<AppState>,
    provider: &str,
    provider_subject: &str,
    email: Option<String>,
) -> Result<OAuthUserRow, HttpResponse> {
    match fetch_oauth_user_by_identity(&state.db, provider, provider_subject).await {
        Ok(Some(user)) => {
            let _ = touch_oauth_identity(&state.db, provider, provider_subject).await;
            return Ok(user);
        }
        Ok(None) => {}
        Err(_) => {
            return Err(HttpResponse::InternalServerError().json(ErrorResponse {
                error: "db_error".into(),
                details: None,
            }));
        }
    }

    let Some(email) = email else {
        return Err(HttpResponse::Unauthorized().json(ErrorResponse {
            error: "email_required".into(),
            details: Some("Email absent de l'identite OAuth verifiee".into()),
        }));
    };

    let user = match fetch_user_by_email(&state.db, &email).await {
        Ok(Some(user)) => user,
        Ok(None) => create_oauth_user(state, &email).await?,
        Err(_) => {
            return Err(HttpResponse::InternalServerError().json(ErrorResponse {
                error: "db_error".into(),
                details: None,
            }));
        }
    };

    let insert_identity = sqlx::query(
        "INSERT INTO oauth_identities (
            provider,
            provider_subject_ciphertext,
            provider_subject_lookup_hash,
            user_id
         )
         VALUES ($1, fiestaaa_encrypt_text($2), fiestaaa_lookup_text($2), $3)
         ON CONFLICT (provider, provider_subject_lookup_hash) DO NOTHING",
    )
    .bind(provider)
    .bind(provider_subject)
    .bind(user.id)
    .execute(&state.db)
    .await;
    if insert_identity.is_err() {
        return Err(HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        }));
    }
    let _ = sqlx::query(
        "DELETE FROM pending_registrations WHERE fiestaaa_email_matches(email_lookup_hash, $1)",
    )
    .bind(&email)
    .execute(&state.db)
    .await;

    match fetch_oauth_user_by_identity(&state.db, provider, provider_subject).await {
        Ok(Some(user)) => {
            let _ = touch_oauth_identity(&state.db, provider, provider_subject).await;
            Ok(user)
        }
        _ => Err(HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        })),
    }
}

async fn oauth_google(
    req: HttpRequest,
    state: web::Data<AppState>,
    payload: OAuthPayload,
) -> HttpResponse {
    let id_token = payload
        .id_token
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());
    let access_token = payload
        .access_token
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());
    if id_token.is_none() && access_token.is_none() {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("idToken ou accessToken requis".into()),
        });
    }

    let mut allowed_aud = Vec::new();
    if let Some(web_id) = state.google_client_id.as_ref() {
        allowed_aud.push(web_id.as_str());
    }
    if let Some(android_id) = state.google_android_client_id.as_ref() {
        allowed_aud.push(android_id.as_str());
    }
    if let Some(ios_id) = state.google_ios_client_id.as_ref() {
        allowed_aud.push(ios_id.as_str());
    }
    if allowed_aud.is_empty() {
        return HttpResponse::InternalServerError().json(ErrorResponse {
            error: "oauth_not_configured".into(),
            details: Some("aucun FIESTAAA_GOOGLE_*_CLIENT_ID defini (WEB/ANDROID/IOS)".into()),
        });
    }

    let mut query = Vec::new();
    if let Some(idt) = id_token {
        query.push(("id_token", idt));
    }
    if let Some(at) = access_token {
        query.push(("access_token", at));
    }

    let token_info = match state
        .http_client
        .get("https://oauth2.googleapis.com/tokeninfo")
        .query(&query)
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => match resp.json::<serde_json::Value>().await {
            Ok(v) => v,
            Err(_) => {
                return HttpResponse::Unauthorized().json(ErrorResponse {
                    error: "invalid_token".into(),
                    details: Some("Réponse Google invalide".into()),
                });
            }
        },
        _ => {
            return HttpResponse::Unauthorized().json(ErrorResponse {
                error: "invalid_token".into(),
                details: Some("Échec vérification Google".into()),
            });
        }
    };

    // tokeninfo returns `aud` for id_tokens and `audience` for access_tokens.
    let aud = token_info
        .get("aud")
        .or_else(|| token_info.get("audience"))
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    if !allowed_aud.contains(&aud) {
        return HttpResponse::Unauthorized().json(ErrorResponse {
            error: "invalid_token".into(),
            details: Some("aud mismatch".into()),
        });
    }
    if id_token.is_some() {
        let issuer = token_info.get("iss").and_then(|value| value.as_str());
        if !matches!(
            issuer,
            Some("accounts.google.com") | Some("https://accounts.google.com")
        ) {
            return HttpResponse::Unauthorized().json(ErrorResponse {
                error: "invalid_token".into(),
                details: Some("issuer Google invalide".into()),
            });
        }
    }

    let mut userinfo = None;
    let mut provider_subject = google_subject(&token_info);
    if provider_subject.is_none()
        && let Some(token) = access_token
    {
        userinfo = fetch_google_userinfo(&state, token).await;
        provider_subject = userinfo.as_ref().and_then(google_subject);
    }
    let Some(provider_subject) = provider_subject else {
        return HttpResponse::Unauthorized().json(ErrorResponse {
            error: "invalid_token".into(),
            details: Some("subject Google manquant".into()),
        });
    };

    let mut verified_email =
        normalize_provider_email(token_info.get("email").and_then(|value| value.as_str()))
            .filter(|_| json_truthy(token_info.get("email_verified")));
    if verified_email.is_none()
        && let Some(token) = access_token
    {
        if userinfo.is_none() {
            userinfo = fetch_google_userinfo(&state, token).await;
        }
        verified_email = userinfo.as_ref().and_then(|profile| {
            normalize_provider_email(profile.get("email").and_then(|value| value.as_str()))
                .filter(|_| json_truthy(profile.get("email_verified")))
        });
    }

    let user = match resolve_oauth_user(&state, "google", &provider_subject, verified_email).await {
        Ok(user) => user,
        Err(resp) => return resp,
    };

    let exp = (now_ts() + 24 * 3600) as usize;
    let claims = Claims {
        sub: user.public_id.to_string(),
        handle: user.handle.clone(),
        exp,
    };
    let secure_cookie = auth_cookie_is_secure(state.get_ref());

    match encode_jwt(&claims, &state.jwt_secret) {
        Ok(token) => token_response(
            token,
            user.public_id.to_string(),
            user.email,
            user.handle,
            secure_cookie,
            auth_response_includes_body_token(&req),
        ),
        Err(_) => HttpResponse::InternalServerError().json(ErrorResponse {
            error: "token_creation_failed".into(),
            details: None,
        }),
    }
}

async fn oauth_apple(
    req: HttpRequest,
    state: web::Data<AppState>,
    payload: OAuthPayload,
) -> HttpResponse {
    let mut allowed_aud = Vec::new();
    if let Some(service_id) = state.apple_service_id.as_ref() {
        allowed_aud.push(service_id.as_str());
    }
    if let Some(app_id) = state.apple_app_id.as_ref() {
        allowed_aud.push(app_id.as_str());
    }
    if allowed_aud.is_empty() {
        return HttpResponse::InternalServerError().json(ErrorResponse {
            error: "oauth_not_configured".into(),
            details: Some("FIESTAAA_APPLE_SERVICE_ID ou FIESTAAA_APPLE_APP_ID manquant".into()),
        });
    }
    let id_token = match payload
        .id_token
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        Some(v) => v,
        None => {
            return HttpResponse::BadRequest().json(ErrorResponse {
                error: "invalid_payload".into(),
                details: Some("idToken requis".into()),
            });
        }
    };

    let header = match jsonwebtoken::decode_header(id_token) {
        Ok(h) => h,
        Err(_) => {
            return HttpResponse::Unauthorized().json(ErrorResponse {
                error: "invalid_token".into(),
                details: Some("Header Apple invalide".into()),
            });
        }
    };
    let kid = match header.kid {
        Some(k) => k,
        None => {
            return HttpResponse::Unauthorized().json(ErrorResponse {
                error: "invalid_token".into(),
                details: Some("kid manquant".into()),
            });
        }
    };
    let alg = header.alg;
    if alg != jsonwebtoken::Algorithm::RS256 {
        return HttpResponse::Unauthorized().json(ErrorResponse {
            error: "invalid_token".into(),
            details: Some("alg non supporté".into()),
        });
    }

    let Some(decoding_key) = fetch_apple_decoding_key(&state, &kid).await else {
        return HttpResponse::Unauthorized().json(ErrorResponse {
            error: "invalid_token".into(),
            details: Some("clé Apple introuvable".into()),
        });
    };

    let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::RS256);
    validation.set_audience(&allowed_aud);
    validation.set_issuer(&["https://appleid.apple.com"]);

    let claims = match jsonwebtoken::decode::<AppleClaims>(id_token, &decoding_key, &validation) {
        Ok(data) => data.claims,
        Err(_) => {
            return HttpResponse::Unauthorized().json(ErrorResponse {
                error: "invalid_token".into(),
                details: Some("JWT Apple invalide".into()),
            });
        }
    };

    let verified_email = normalize_provider_email(claims.email.as_deref())
        .filter(|_| json_truthy(claims.email_verified.as_ref()));
    let user = match resolve_oauth_user(&state, "apple", &claims.sub, verified_email).await {
        Ok(user) => user,
        Err(resp) => return resp,
    };

    let exp = claims.exp;
    let claims = Claims {
        sub: user.public_id.to_string(),
        handle: user.handle.clone(),
        exp,
    };
    let secure_cookie = auth_cookie_is_secure(state.get_ref());

    match encode_jwt(&claims, &state.jwt_secret) {
        Ok(token) => token_response(
            token,
            user.public_id.to_string(),
            user.email,
            user.handle,
            secure_cookie,
            auth_response_includes_body_token(&req),
        ),
        Err(_) => HttpResponse::InternalServerError().json(ErrorResponse {
            error: "token_creation_failed".into(),
            details: None,
        }),
    }
}

#[derive(serde::Deserialize)]
struct AppleJwk {
    kid: String,
    n: String,
    e: String,
}

#[derive(serde::Deserialize)]
struct AppleJwkSet {
    keys: Vec<AppleJwk>,
}

async fn fetch_apple_decoding_key(
    state: &web::Data<AppState>,
    kid: &str,
) -> Option<jsonwebtoken::DecodingKey> {
    let resp = state
        .http_client
        .get("https://appleid.apple.com/auth/keys")
        .send()
        .await
        .ok()?;
    let jwks: AppleJwkSet = resp.json().await.ok()?;
    let key = jwks.keys.into_iter().find(|k| k.kid == kid)?;
    jsonwebtoken::DecodingKey::from_rsa_components(&key.n, &key.e).ok()
}

#[utoipa::path(
    post,
    path = "/auth/logout",
    tag = "auth",
    responses(
        (status = 200, description = "Session invalidated", body = StatusResponse)
    )
)]
#[post("/auth/logout")]
pub async fn logout(state: web::Data<AppState>, req: HttpRequest) -> impl Responder {
    if let Err(resp) = revoke_auth_token_from_request(&req, &state.db, &state.jwt_secret).await {
        return resp;
    }
    HttpResponse::Ok()
        .cookie(build_cleared_session_cookie(auth_cookie_is_secure(
            state.get_ref(),
        )))
        .json(StatusResponse {
            status: "logged_out".into(),
        })
}

#[utoipa::path(
    post,
    path = "/auth/login",
    tag = "auth",
    request_body = LoginPayload,
    responses(
        (status = 200, description = "Valid credentials", body = TokenResponse),
        (status = 401, description = "Invalid credentials", body = ErrorResponse),
        (status = 500, description = "Database or token creation error", body = ErrorResponse)
    )
)]
#[post("/auth/login")]
pub async fn login(
    req: HttpRequest,
    state: web::Data<AppState>,
    payload: web::Json<LoginPayload>,
) -> impl Responder {
    if let Err(resp) = enforce_auth_rate_limit(&req, state.get_ref(), "login").await {
        return resp;
    }
    let auth_row = match fetch_user_auth(&state.db, &payload.identifier).await {
        Ok(Some(row)) => row,
        Ok(None) => {
            return HttpResponse::Unauthorized().json(ErrorResponse {
                error: "invalid_credentials".into(),
                details: None,
            });
        }
        Err(_) => {
            return HttpResponse::InternalServerError().json(ErrorResponse {
                error: "db_error".into(),
                details: None,
            });
        }
    };

    if !verify_password(&auth_row.password_hash, &payload.password) {
        return HttpResponse::Unauthorized().json(ErrorResponse {
            error: "invalid_credentials".into(),
            details: None,
        });
    }

    let exp = (now_ts() + 24 * 3600) as usize;
    let claims = Claims {
        sub: auth_row.public_id.to_string(),
        handle: auth_row.handle.clone(),
        exp,
    };
    let secure_cookie = auth_cookie_is_secure(state.get_ref());

    match encode_jwt(&claims, &state.jwt_secret) {
        Ok(token) => token_response(
            token,
            auth_row.public_id.to_string(),
            auth_row.email,
            auth_row.handle,
            secure_cookie,
            auth_response_includes_body_token(&req),
        ),
        Err(_) => HttpResponse::InternalServerError().json(ErrorResponse {
            error: "token_creation_failed".into(),
            details: None,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::{google_subject, json_truthy, normalize_provider_email};

    #[test]
    fn normalize_provider_email_trims_and_lowercases() {
        assert_eq!(
            normalize_provider_email(Some("  USER@Example.COM ")).as_deref(),
            Some("user@example.com")
        );
        assert_eq!(normalize_provider_email(Some("   ")), None);
        assert_eq!(normalize_provider_email(None), None);
    }

    #[test]
    fn json_truthy_accepts_boolean_and_string_true() {
        assert!(json_truthy(Some(&serde_json::json!(true))));
        assert!(json_truthy(Some(&serde_json::json!("true"))));
        assert!(!json_truthy(Some(&serde_json::json!(false))));
        assert!(!json_truthy(Some(&serde_json::json!("false"))));
        assert!(!json_truthy(None));
    }

    #[test]
    fn google_subject_prefers_sub_and_rejects_blank_values() {
        assert_eq!(
            google_subject(&serde_json::json!({ "sub": "google-user-123" })).as_deref(),
            Some("google-user-123")
        );
        assert_eq!(
            google_subject(&serde_json::json!({ "user_id": "legacy-user-id" })).as_deref(),
            Some("legacy-user-id")
        );
        assert_eq!(google_subject(&serde_json::json!({ "sub": "   " })), None);
    }
}
