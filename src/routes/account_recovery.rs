use crate::{
    auth::{hash_password, validate_password_strength},
    security::{normalize_email, sha256_hex},
    state::AppState,
};
use actix_web::{HttpRequest, HttpResponse, Responder, post, web};
use serde::Deserialize;
use serde_json::json;
use sqlx::Row;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Deserialize, ToSchema)]
pub struct ResetRequest {
    pub email: String,
}
#[derive(Deserialize, ToSchema)]
pub struct ResetConfirm {
    pub token: String,
    pub password: String,
}

#[utoipa::path(post, path="/auth/password-reset/request", tag="auth", request_body=ResetRequest,
 responses((status=202, description="If eligible, recovery email will be sent"), (status=429, description="Rate limited")))]
#[post("/auth/password-reset/request")]
pub async fn request_reset(
    req: HttpRequest,
    state: web::Data<AppState>,
    payload: web::Json<ResetRequest>,
) -> impl Responder {
    if let Err(r) = super::auth::enforce_auth_rate_limit(&req, &state, "password-reset").await {
        return r;
    }
    let email = normalize_email(&payload.email);
    if email.len() > 254 || !email.contains('@') {
        return accepted();
    }
    if !state
        .auth_rate_limiter
        .allow(&format!("reset-email:{}", sha256_hex(&email)))
        .await
    {
        return accepted();
    }
    let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let result = sqlx::query(
        "INSERT INTO password_resets(user_id, token_hash, expires_at) SELECT id, $2, NOW()+INTERVAL '30 minutes'
         FROM users WHERE fiestaaa_email_matches(email_lookup_hash,$1) AND password_login_enabled AND NOT suspended
         ON CONFLICT(user_id) DO UPDATE SET token_hash=EXCLUDED.token_hash, expires_at=EXCLUDED.expires_at, requested_at=NOW()
         WHERE password_resets.requested_at < NOW()-INTERVAL '60 seconds' RETURNING user_id")
        .bind(&email).bind(sha256_hex(&token)).fetch_optional(&state.db).await;
    match result {
        Err(_) => {
            return HttpResponse::ServiceUnavailable()
                .json(json!({"error":"recovery_unavailable"}));
        }
        Ok(Some(_)) => {
            // Identical HTTP response regardless of provider latency or account eligibility.
            actix_web::rt::spawn(async move {
                let Some(key) = &state.invitation_email_api_key else {
                    log::warn!("password reset email configuration missing");
                    return;
                };
                let Some(sender) = &state.invitation_email_sender else {
                    return;
                };
                let mut url = match reqwest::Url::parse(&state.app_base_url) {
                    Ok(u) => u,
                    Err(_) => return,
                };
                url.set_path("/reset-password");
                url.set_query(Some(&format!("token={token}")));
                url.set_fragment(None);
                let result = state.http_client.post("https://api.resend.com/emails").bearer_auth(key)
                    .json(&json!({"from":sender,"to":[email],"subject":"Fiestaaa — Réinitialiser / Reset password",
                    "text":format!("Réinitialisez votre mot de passe / Reset your password:\n{url}\n\nCe lien expire dans 30 minutes et ne peut être utilisé qu'une fois.\nThis link expires in 30 minutes and can only be used once.\nIgnorez ce message si vous n'êtes pas à l'origine de la demande / Ignore this message if you did not request it.")}))
                    .send().await;
                if !result.is_ok_and(|r| r.status().is_success()) {
                    crate::observability::record_email_error("password_reset_delivery_failed");
                }
            });
        }
        Ok(None) => {}
    }
    accepted()
}
fn accepted() -> HttpResponse {
    HttpResponse::Accepted()
        .insert_header(("Cache-Control", "no-store"))
        .json(json!({"status":"recovery_requested"}))
}

#[utoipa::path(post, path="/auth/password-reset/confirm", tag="auth", request_body=ResetConfirm,
 responses((status=200,description="Password changed, previous sessions invalidated"),(status=400,description="Invalid/expired token or password"),(status=429,description="Rate limited")))]
#[post("/auth/password-reset/confirm")]
pub async fn confirm_reset(
    req: HttpRequest,
    state: web::Data<AppState>,
    payload: web::Json<ResetConfirm>,
) -> impl Responder {
    if let Err(r) =
        super::auth::enforce_auth_rate_limit(&req, &state, "password-reset-confirm").await
    {
        return r;
    }
    if payload.token.len() != 64
        || payload.password.len() > 1024
        || validate_password_strength(&payload.password).is_err()
    {
        return HttpResponse::BadRequest().json(json!({"error":"invalid_reset","details":"12 characters minimum, uppercase, lowercase, digit and symbol required"}));
    }
    let hash = match hash_password(&payload.password) {
        Ok(h) => h,
        Err(_) => return HttpResponse::InternalServerError().finish(),
    };
    let mut tx = match state.db.begin().await {
        Ok(t) => t,
        Err(_) => return HttpResponse::ServiceUnavailable().finish(),
    };
    let row = sqlx::query(
        "DELETE FROM password_resets WHERE token_hash=$1 AND expires_at>NOW() RETURNING user_id",
    )
    .bind(sha256_hex(&payload.token))
    .fetch_optional(&mut *tx)
    .await;
    let user_id: i64 = match row {
        Ok(Some(r)) => r.get("user_id"),
        Ok(None) => {
            return HttpResponse::BadRequest().json(json!({"error":"invalid_or_expired_reset"}));
        }
        Err(_) => return HttpResponse::ServiceUnavailable().finish(),
    };
    let result = sqlx::query("UPDATE users SET password_hash=$2, session_version=session_version+1 WHERE id=$1 AND password_login_enabled AND NOT suspended")
        .bind(user_id).bind(hash).execute(&mut *tx).await;
    if !result.is_ok_and(|r| r.rows_affected() == 1) {
        return HttpResponse::BadRequest().json(json!({"error":"invalid_or_expired_reset"}));
    }
    if tx.commit().await.is_err() {
        return HttpResponse::ServiceUnavailable().finish();
    }
    HttpResponse::Ok()
        .insert_header(("Cache-Control", "no-store"))
        .json(json!({"status":"password_reset"}))
}
