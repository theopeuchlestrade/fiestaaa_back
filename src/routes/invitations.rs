use actix_web::{HttpRequest, HttpResponse, Responder, delete, get, patch, post, web};
use chrono::{Duration, NaiveDate, NaiveTime, Utc};
use log::{error, warn};
use serde_json::json;
use sqlx::{FromRow, Row};
use uuid::Uuid;

use crate::{
    auth::extract_active_claims_from_auth,
    handles::{is_valid_handle, looks_like_email, normalize_handle},
    models::{
        ErrorResponse, Invitation, InvitationPatchPayload, InvitationPayload, StatusResponse,
    },
    notifications::{NotificationRequest, find_user_id_by_email, notify_users},
    realtime::{event_types, publish_event, publish_event_type, publish_global_type},
    routes::event_access::ensure_event_writable,
    security::sha256_hex,
    state::AppState,
};

async fn claims_email(req: &HttpRequest, state: &AppState) -> Result<String, HttpResponse> {
    let claims = extract_active_claims_from_auth(req, &state.db, &state.jwt_secret).await?;
    Ok(claims.sub.to_lowercase())
}

fn invitation_rate_limit_remote(req: &HttpRequest, state: &AppState) -> String {
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

async fn enforce_invitation_rate_limit(
    req: &HttpRequest,
    state: &AppState,
    owner_email: &str,
) -> Result<(), HttpResponse> {
    let remote = invitation_rate_limit_remote(req, state);
    let key = format!("invitation:email:{}:{remote}", owner_email.to_lowercase());
    if state.invitation_rate_limiter.allow(&key).await {
        Ok(())
    } else {
        Err(HttpResponse::TooManyRequests().json(ErrorResponse {
            error: "rate_limited".into(),
            details: Some("too many email invitations".into()),
        }))
    }
}

async fn fetch_event_owner_email(db: &sqlx::PgPool, event_id: i64) -> Result<String, HttpResponse> {
    let owner = sqlx::query_scalar::<_, String>(
        "SELECT fiestaaa_decrypt_text(u.email_ciphertext) AS owner_email
         FROM events e
         JOIN users u ON u.id = e.owner_user_id
         WHERE e.event_id = $1",
    )
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
    let requester = claims_email(req, state).await?;
    let requester = fetch_user_by_email(&state.db, &requester).await?;
    let owner = sqlx::query_as::<_, (String, i64)>(
        "SELECT fiestaaa_decrypt_text(u.email_ciphertext) AS owner_email,
                e.owner_user_id
         FROM events e
         JOIN users u ON u.id = e.owner_user_id
         WHERE e.event_id = $1",
    )
    .bind(event_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|_| {
        HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        })
    })?;

    let Some((owner_email, owner_user_id)) = owner else {
        return Err(HttpResponse::NotFound().json(ErrorResponse {
            error: "event_not_found".into(),
            details: None,
        }));
    };

    if owner_user_id != requester.id {
        Err(HttpResponse::Forbidden().json(ErrorResponse {
            error: "forbidden".into(),
            details: Some("only the creator can manage invitations".into()),
        }))
    } else {
        Ok(owner_email)
    }
}

async fn ensure_event_participant(
    req: &HttpRequest,
    state: &AppState,
    event_id: i64,
) -> Result<(), HttpResponse> {
    let requester = claims_email(req, state).await?;
    let requester = fetch_user_by_email(&state.db, &requester).await?;
    let owner_id =
        sqlx::query_scalar::<_, i64>("SELECT owner_user_id FROM events WHERE event_id = $1")
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
    if owner_id == requester.id {
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
    .bind(requester.id)
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

#[derive(Debug, FromRow)]
struct UserIdentity {
    id: i64,
    email: String,
    handle: String,
    avatar_url: Option<String>,
}

enum TargetIdentifier {
    Email(String),
    Handle(String),
}

async fn expire_overdue_invitations(db: &sqlx::PgPool) -> Result<(), HttpResponse> {
    sqlx::query(
        "UPDATE invitations i
         SET status = 'Expired'
         FROM events e
         WHERE i.event_id = e.event_id
           AND i.status = 'Waiting'
           AND e.invitation_deadline IS NOT NULL
           AND CURRENT_DATE > e.invitation_deadline",
    )
    .execute(db)
    .await
    .map(|_| ())
    .map_err(|_| {
        HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        })
    })
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

async fn fetch_user_by_email(db: &sqlx::PgPool, email: &str) -> Result<UserIdentity, HttpResponse> {
    let normalized = email.trim().to_lowercase();
    if normalized.is_empty() {
        return Err(HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_email".into(),
            details: Some("email is required".into()),
        }));
    }

    sqlx::query_as::<_, UserIdentity>(
        "SELECT id,
                fiestaaa_decrypt_text(email_ciphertext) AS email,
                handle,
                avatar_url
         FROM users
         WHERE fiestaaa_email_matches(email_lookup_hash, $1)",
    )
    .bind(&normalized)
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

async fn find_user_by_handle(
    db: &sqlx::PgPool,
    handle: &str,
) -> Result<Option<UserIdentity>, HttpResponse> {
    sqlx::query_as::<_, UserIdentity>(
        "SELECT id,
                fiestaaa_decrypt_text(email_ciphertext) AS email,
                handle,
                avatar_url
         FROM users
         WHERE lower(handle) = lower($1)",
    )
    .bind(handle)
    .fetch_optional(db)
    .await
    .map_err(|_| {
        HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        })
    })
}

struct EventEmailMetadata {
    name: String,
    date: NaiveDate,
    start_time: NaiveTime,
}

async fn fetch_event_email_metadata(
    db: &sqlx::PgPool,
    event_id: i64,
) -> Result<EventEmailMetadata, HttpResponse> {
    let row =
        sqlx::query("SELECT name_event, date_event, start_time FROM events WHERE event_id = $1")
            .bind(event_id)
            .fetch_optional(db)
            .await
            .map_err(|_| {
                HttpResponse::InternalServerError().json(ErrorResponse {
                    error: "db_error".into(),
                    details: None,
                })
            })?;

    match row {
        Some(row) => Ok(EventEmailMetadata {
            name: row
                .try_get("name_event")
                .unwrap_or_else(|_| "Événement".to_string()),
            date: row
                .try_get("date_event")
                .unwrap_or_else(|_| chrono::Utc::now().date_naive()),
            start_time: row
                .try_get("start_time")
                .unwrap_or_else(|_| chrono::NaiveTime::from_hms_opt(19, 0, 0).unwrap()),
        }),
        None => Err(HttpResponse::NotFound().json(ErrorResponse {
            error: "event_not_found".into(),
            details: None,
        })),
    }
}

fn build_share_link(base_url: &str, token: &Uuid) -> String {
    let trimmed = base_url.trim_end_matches('/');
    if trimmed.contains('?') {
        format!("{trimmed}&shareToken={token}")
    } else {
        format!("{trimmed}?shareToken={token}")
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

async fn send_invitation_email(
    state: &AppState,
    to_email: &str,
    owner_email: &str,
    share_link: &str,
    event: &EventEmailMetadata,
) -> Result<(), HttpResponse> {
    let sender = match &state.invitation_email_sender {
        Some(value) => value,
        None => {
            warn!("Invitation email not sent: INVITATION_EMAIL_SENDER missing");
            return Err(HttpResponse::ServiceUnavailable().json(ErrorResponse {
                error: "email_not_configured".into(),
                details: Some("INVITATION_EMAIL_SENDER manquant".into()),
            }));
        }
    };

    let api_key = match &state.invitation_email_api_key {
        Some(value) => value,
        None => {
            warn!("Invitation email not sent: RESEND_API_KEY missing");
            return Err(HttpResponse::ServiceUnavailable().json(ErrorResponse {
                error: "email_not_configured".into(),
                details: Some("RESEND_API_KEY manquant".into()),
            }));
        }
    };

    let date_label = event.date.format("%d/%m/%Y").to_string();
    let time_label = event.start_time.format("%H:%M").to_string();
    let subject = format!("{owner_email} t'invite à \"{}\" sur Fiestaaa", event.name);
    let body = format!(
        "Salut !\n\n{owner_email} t'invite à rejoindre \"{event_name}\" sur Fiestaaa.\nFiestaaa rassemble tes invités, les infos pratiques et le suivi des réponses en un seul endroit.\n\nÉvénement : {event_name}\nDate : {date_label}\nHeure : {time_label}\nLien unique : {share_link}\n\nCe lien te permet de créer un compte si besoin et de confirmer ta présence.\n\nÀ très vite,\nL'équipe Fiestaaa",
        owner_email = owner_email,
        event_name = event.name,
        date_label = date_label,
        time_label = time_label,
        share_link = share_link
    );
    let html_body = format!(
        r#"<!doctype html>
<html lang="fr">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <title>Invitation à {event_name} — Fiestaaa</title>
</head>
<body style="margin:0;padding:0;background:#f1f5f9;font-family:'Inter',-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;color:#0f172a;">
  <table role="presentation" width="100%" cellpadding="0" cellspacing="0" style="padding:30px 0;background:linear-gradient(135deg,#fef3c7,#e0f2fe);">
    <tr>
      <td align="center">
        <table role="presentation" width="640" cellpadding="0" cellspacing="0" style="background:#ffffff;border-radius:18px;overflow:hidden;box-shadow:0 20px 40px rgba(15,23,42,0.15);text-align:left;">
          <tr>
            <td style="background:linear-gradient(120deg,#0ea5e9,#6366f1);padding:18px 26px;color:#0b1224;font-weight:700;font-size:16px;letter-spacing:0.4px;">
              Fiestaaa
              <div style="font-size:13px;font-weight:500;color:rgba(11,18,36,0.85);margin-top:4px;">Organise tes soirées, centralise les infos, garde la liste des invités.</div>
            </td>
          </tr>
          <tr>
            <td style="padding:26px 28px 18px;">
              <p style="margin:0 0 10px;font-size:22px;font-weight:700;">Invitation à <span style="color:#0ea5e9;">{event_name}</span></p>
              <p style="margin:0 0 18px;color:#334155;font-size:15px;">{owner_email_html} t'a envoyé cette invitation via Fiestaaa, l'app qui garde toutes les infos de l'événement au même endroit.</p>
              <div style="padding:14px 16px;background:#f8fafc;border:1px solid #e2e8f0;border-radius:14px;margin-bottom:18px;">
                <div style="font-size:16px;font-weight:700;color:#0f172a;">{event_name}</div>
                <div style="font-size:14px;color:#475569;margin-top:4px;">{date_label} • {time_label}</div>
                <div style="font-size:14px;color:#475569;margin-top:2px;">Organisé par {owner_email_html}</div>
              </div>
              <p style="margin:0 0 12px;color:#334155;font-size:15px;">Rejoins l'événement, confirme ta présence et retrouve toutes les infos utiles.</p>
              <a href="{share_link_html}" style="display:inline-block;padding:14px 22px;background:linear-gradient(120deg,#0ea5e9,#6366f1);color:#ffffff;text-decoration:none;font-weight:700;border-radius:12px;">Rejoindre l'événement</a>
              <p style="margin:12px 0 0;color:#475569;font-size:13px;">Lien unique : crée un compte si nécessaire puis accepte l'invitation.</p>
              <p style="margin:16px 0 6px;font-size:14px;font-weight:700;color:#0f172a;">Fiestaaa en 3 points :</p>
              <ul style="margin:0 0 10px;padding-left:18px;color:#475569;line-height:1.6;font-size:14px;">
                <li>Centralise les infos pratiques de la soirée et les dernières updates.</li>
                <li>Invite tout le monde via un lien unique, même sans compte.</li>
                <li>Suis qui vient et garde les échanges au même endroit.</li>
              </ul>
              <p style="margin:12px 0 0;font-size:13px;color:#64748b;">Si le bouton ne s'affiche pas, copie-colle ce lien : <a href="{share_link_html}" style="color:#0ea5e9;font-weight:600;">{share_link_html}</a></p>
            </td>
          </tr>
          <tr>
            <td style="padding:16px 28px 22px;border-top:1px solid #e2e8f0;background:#f8fafc;color:#64748b;font-size:12px;">
              À bientôt sur Fiestaaa.
            </td>
          </tr>
        </table>
      </td>
    </tr>
  </table>
</body>
</html>"#,
        event_name = escape_html(&event.name),
        owner_email_html = escape_html(owner_email),
        date_label = date_label,
        time_label = time_label,
        share_link_html = escape_html(share_link),
    );

    let payload = json!({
        "from": sender,
        "to": [to_email],
        "subject": subject,
        "text": body,
        "html": html_body
    });

    let res = state
        .http_client
        .post("https://api.resend.com/emails")
        .bearer_auth(api_key)
        .json(&payload)
        .send()
        .await;

    match res {
        Ok(resp) if resp.status().is_success() => Ok(()),
        Ok(resp) => {
            let status = resp.status();
            warn!("Invitation email provider failure: status {status}");
            Err(HttpResponse::BadGateway().json(ErrorResponse {
                error: "email_send_failed".into(),
                details: Some(format!("provider status {status}")),
            }))
        }
        Err(e) => {
            error!("Invitation email send failed: transport error: {e}");
            Err(HttpResponse::BadGateway().json(ErrorResponse {
                error: "email_send_failed".into(),
                details: Some("transport_error".into()),
            }))
        }
    }
}

fn accepted_email_invitation_response() -> HttpResponse {
    HttpResponse::Accepted().json(StatusResponse {
        status: "email_sent".into(),
    })
}

async fn invite_email_target(
    state: &AppState,
    event_id: i64,
    invitee_email: &str,
    owner_email: &str,
) -> Result<HttpResponse, HttpResponse> {
    let normalized_invitee = invitee_email.trim().to_lowercase();
    if normalized_invitee.is_empty() {
        return Err(HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_email".into(),
            details: Some("email is required".into()),
        }));
    }
    if normalized_invitee.eq_ignore_ascii_case(owner_email) {
        return Ok(accepted_email_invitation_response());
    }

    let event = fetch_event_email_metadata(&state.db, event_id).await?;

    let already_invited = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(
            SELECT 1
            FROM invitations i
            JOIN users u ON u.id = i.user_id
            WHERE i.event_id = $1
              AND fiestaaa_email_matches(u.email_lookup_hash, $2)
              AND i.status <> 'Expired'
        )",
    )
    .bind(event_id)
    .bind(&normalized_invitee)
    .fetch_one(&state.db)
    .await
    .map_err(|_| {
        HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        })
    })?;
    if already_invited {
        return Ok(accepted_email_invitation_response());
    }

    let existing_pending = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(
            SELECT 1
            FROM event_share_tokens
            WHERE event_id = $1
              AND target_email_lookup_hash = fiestaaa_email_lookup($2)
              AND used_at IS NULL
              AND expires_at >= NOW()
        )",
    )
    .bind(event_id)
    .bind(&normalized_invitee)
    .fetch_one(&state.db)
    .await
    .map_err(|_| {
        HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        })
    })?;
    if existing_pending {
        return Ok(accepted_email_invitation_response());
    }

    let mut tx = state.db.begin().await.map_err(|_| {
        HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        })
    })?;

    let token = Uuid::new_v4();
    let token_hash = sha256_hex(&token.to_string());
    let expires_at = Utc::now() + Duration::days(7);
    let owner_user_id = fetch_user_by_email(&state.db, owner_email).await?.id;

    if (sqlx::query(
        "INSERT INTO event_share_tokens (
            token_hash,
            event_id,
            created_by_user_id,
            target_email_ciphertext,
            target_email_lookup_hash,
            expires_at
         )
         VALUES (
            $1,
            $2,
            $3,
            fiestaaa_encrypt_text($4),
            fiestaaa_email_lookup($4),
            $5
         )",
    )
    .bind(&token_hash)
    .bind(event_id)
    .bind(owner_user_id)
    .bind(&normalized_invitee)
    .bind(expires_at)
    .execute(&mut *tx)
    .await)
        .is_err()
    {
        let _ = tx.rollback().await;
        return Err(HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        }));
    }

    if (tx.commit().await).is_err() {
        return Err(HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        }));
    }

    let share_link = build_share_link(&state.app_base_url, &token);
    if let Err(resp) =
        send_invitation_email(state, &normalized_invitee, owner_email, &share_link, &event).await
    {
        let _ = sqlx::query("DELETE FROM event_share_tokens WHERE token_hash = $1")
            .bind(&token_hash)
            .execute(&state.db)
            .await;
        return Err(resp);
    }

    publish_event_type(
        &state.redis_client,
        event_id,
        event_types::EVENT_INVITATIONS_CHANGED,
    )
    .await;
    publish_global_type(&state.redis_client, event_types::INVITATIONS_CHANGED).await;

    Ok(accepted_email_invitation_response())
}

async fn insert_invitation_for_user(
    db: &sqlx::PgPool,
    event_id: i64,
    user: &UserIdentity,
) -> Result<Option<Invitation>, sqlx::Error> {
    sqlx::query_as::<_, Invitation>(
        "INSERT INTO invitations (event_id, user_id, status)
         VALUES ($1, $2, 'Waiting')
         ON CONFLICT (event_id, user_id)
         DO UPDATE SET status = 'Waiting', date_invi = NOW()
         WHERE invitations.status = 'Expired'
         RETURNING event_id, user_id, $3 AS email, $4 AS handle, $5 AS avatar_url, status, date_invi,
                   (SELECT name_event FROM events WHERE event_id = $1) AS event_name",
    )
    .bind(event_id)
    .bind(user.id)
    .bind(&user.email)
    .bind(&user.handle)
    .bind(&user.avatar_url)
    .fetch_optional(db)
    .await
}

#[utoipa::path(
    get,
    path = "/events/{event_id}/invitations",
    tag = "invitations",
    responses(
        (status = 200, description = "Event invitations", body = [Invitation]),
        (status = 403, description = "Unauthorized", body = ErrorResponse),
        (status = 404, description = "Event not found", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Event identifier")
    )
)]
#[get("/events/{event_id}/invitations")]
pub async fn list_event_invitations(
    state: web::Data<AppState>,
    req: HttpRequest,
    event_id: web::Path<i64>,
) -> impl Responder {
    let requester = match claims_email(&req, state.get_ref()).await {
        Ok(email) => email,
        Err(resp) => return resp,
    };
    let owner_email = match fetch_event_owner_email(&state.db, *event_id).await {
        Ok(email) => email,
        Err(resp) => return resp,
    };
    if let Err(resp) = ensure_event_participant(&req, state.get_ref(), *event_id).await {
        return resp;
    }
    if let Err(resp) = expire_overdue_invitations(&state.db).await {
        return resp;
    }

    match sqlx::query_as::<_, Invitation>(
        "SELECT e.event_id,
                u_owner.id AS user_id,
                fiestaaa_decrypt_text(u_owner.email_ciphertext) AS email,
                u_owner.handle AS handle,
                u_owner.avatar_url AS avatar_url,
                'Accepted'::text AS status,
                NOW() AS date_invi,
                e.name_event AS event_name
         FROM events e
         LEFT JOIN users u_owner ON u_owner.id = e.owner_user_id
         WHERE e.event_id = $1
         UNION ALL
         SELECT i.event_id,
                u.id AS user_id,
                fiestaaa_decrypt_text(u.email_ciphertext) AS email,
                u.handle,
                u.avatar_url,
                i.status,
                i.date_invi,
                e.name_event AS event_name
         FROM invitations i
         JOIN users u ON u.id = i.user_id
         JOIN events e ON e.event_id = i.event_id
         WHERE i.event_id = $1
           AND u.id <> e.owner_user_id
         UNION ALL
         SELECT pending.event_id,
                NULL::BIGINT AS user_id,
                pending.email AS email,
                NULL::TEXT AS handle,
                NULL::TEXT AS avatar_url,
                'Waiting'::text AS status,
                pending.created_at AS date_invi,
                e.name_event AS event_name
         FROM (
             SELECT DISTINCT ON (target_email_lookup_hash)
                    event_id,
                    fiestaaa_decrypt_text(target_email_ciphertext) AS email,
                    created_at,
                    target_email_lookup_hash
             FROM event_share_tokens
             WHERE event_id = $1
               AND target_email_lookup_hash IS NOT NULL
               AND used_at IS NULL
               AND expires_at >= NOW()
             ORDER BY target_email_lookup_hash, created_at DESC
         ) pending
         JOIN events e ON e.event_id = pending.event_id
         ORDER BY date_invi DESC",
    )
    .bind(*event_id)
    .fetch_all(&state.db)
    .await
    {
        Ok(mut list) => {
            if !owner_email.eq_ignore_ascii_case(&requester) {
                list.retain(|invitation| invitation.user_id.is_some());
                for invitation in &mut list {
                    if !invitation.email.eq_ignore_ascii_case(&requester)
                        && !invitation.email.eq_ignore_ascii_case(&owner_email)
                    {
                        invitation.email.clear();
                    }
                }
            }
            HttpResponse::Ok().json(list)
        }
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
        (status = 201, description = "Invitation created", body = Invitation),
        (status = 202, description = "Email invitation accepted for processing", body = StatusResponse),
        (status = 400, description = "Invalid payload", body = ErrorResponse),
        (status = 403, description = "Unauthorized", body = ErrorResponse),
        (status = 404, description = "Event or user not found", body = ErrorResponse),
        (status = 409, description = "Existing invitation", body = ErrorResponse),
        (status = 429, description = "Too many email invitations", body = ErrorResponse)
    )
)]
#[post("/events/{event_id}/invitations")]
pub async fn create_invitation(
    state: web::Data<AppState>,
    req: HttpRequest,
    event_id: web::Path<i64>,
    payload: web::Json<InvitationPayload>,
) -> impl Responder {
    let owner_email = match ensure_event_owner(&req, state.get_ref(), *event_id).await {
        Ok(owner) => owner,
        Err(resp) => return resp,
    };
    if let Err(resp) = ensure_event_writable(&state.db, *event_id).await {
        return resp;
    }
    if let Err(resp) = expire_overdue_invitations(&state.db).await {
        return resp;
    }

    let invitation_deadline = sqlx::query_scalar::<_, Option<NaiveDate>>(
        "SELECT invitation_deadline FROM events WHERE event_id = $1",
    )
    .bind(*event_id)
    .fetch_optional(&state.db)
    .await;

    let invitation_deadline = match invitation_deadline {
        Ok(value) => value,
        Err(_) => {
            return HttpResponse::InternalServerError().json(ErrorResponse {
                error: "db_error".into(),
                details: None,
            });
        }
    };

    if let Some(Some(limit)) = invitation_deadline
        && chrono::Utc::now().date_naive() > limit
    {
        return HttpResponse::Gone().json(ErrorResponse {
            error: "invitation_expired".into(),
            details: Some("La date limite pour répondre est dépassée".into()),
        });
    }

    let identifier = match parse_identifier(&payload.identifier) {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    match identifier {
        TargetIdentifier::Email(email) => {
            if let Err(resp) =
                enforce_invitation_rate_limit(&req, state.get_ref(), &owner_email).await
            {
                return resp;
            }
            match invite_email_target(state.get_ref(), *event_id, &email, &owner_email).await {
                Ok(resp) => resp,
                Err(resp) => resp,
            }
        }
        TargetIdentifier::Handle(handle) => {
            let user = match find_user_by_handle(&state.db, &handle).await {
                Ok(u) => u,
                Err(resp) => return resp,
            };

            match user {
                Some(user) => match insert_invitation_for_user(&state.db, *event_id, &user).await {
                    Ok(Some(inv)) => {
                        publish_event_type(
                            &state.redis_client,
                            *event_id,
                            event_types::EVENT_INVITATIONS_CHANGED,
                        )
                        .await;
                        publish_global_type(&state.redis_client, event_types::INVITATIONS_CHANGED)
                            .await;
                        let event_name =
                            inv.event_name.clone().unwrap_or("un événement".to_string());
                        let title = format!("Invitation à {event_name}");
                        let body = format!("{} t'a invité(e) à {event_name}", owner_email);
                        let dedup = format!("invite_received:{}", *event_id);
                        notify_users(
                            &state.notifications,
                            &state.db,
                            &[user.id],
                            NotificationRequest {
                                title: &title,
                                body: &body,
                                data: json!({
                                    "type": "invite_received",
                                    "event_id": *event_id,
                                    "event_name": inv.event_name.clone()
                                }),
                                dedup_base_key: Some(dedup.as_str()),
                                dedup_ttl: Some(600),
                            },
                        )
                        .await;
                        HttpResponse::Created().json(inv)
                    }
                    Ok(None) => HttpResponse::Conflict().json(ErrorResponse {
                        error: "invitation_exists".into(),
                        details: None,
                    }),
                    Err(sqlx::Error::Database(db_err))
                        if db_err.code().as_deref() == Some("23505") =>
                    {
                        HttpResponse::Conflict().json(ErrorResponse {
                            error: "invitation_exists".into(),
                            details: None,
                        })
                    }
                    Err(_) => HttpResponse::InternalServerError().json(ErrorResponse {
                        error: "db_error".into(),
                        details: None,
                    }),
                },
                None => HttpResponse::NotFound().json(ErrorResponse {
                    error: "user_not_found".into(),
                    details: Some("identifiant introuvable".into()),
                }),
            }
        }
    }
}

#[utoipa::path(
    delete,
    path = "/events/{event_id}/invitations/{email}",
    tag = "invitations",
    responses(
        (status = 200, description = "Invitation deleted", body = StatusResponse),
        (status = 403, description = "Unauthorized", body = ErrorResponse),
        (status = 404, description = "Invitation not found", body = ErrorResponse)
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
    if let Err(resp) = ensure_event_writable(&state.db, event_id).await {
        return resp;
    }
    let owner_email = match fetch_event_owner_email(&state.db, event_id).await {
        Ok(owner) => owner,
        Err(resp) => return resp,
    };
    if owner_email.eq_ignore_ascii_case(email.trim()) {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "cannot_remove_owner".into(),
            details: Some("Le créateur ne peut pas être retiré de l'événement".into()),
        });
    }
    let user = match fetch_user_by_email(&state.db, &email).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    match sqlx::query("DELETE FROM invitations WHERE event_id = $1 AND user_id = $2")
        .bind(event_id)
        .bind(user.id)
        .execute(&state.db)
        .await
    {
        Ok(result) if result.rows_affected() == 0 => HttpResponse::NotFound().json(ErrorResponse {
            error: "invitation_not_found".into(),
            details: None,
        }),
        Ok(_) => {
            publish_event_type(
                &state.redis_client,
                event_id,
                event_types::EVENT_INVITATIONS_CHANGED,
            )
            .await;
            publish_global_type(&state.redis_client, event_types::INVITATIONS_CHANGED).await;
            HttpResponse::Ok().json(StatusResponse {
                status: "deleted".into(),
            })
        }
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
        (status = 200, description = "User invitations", body = [Invitation]),
        (status = 401, description = "Authentication required", body = ErrorResponse)
    )
)]
#[get("/my/invitations")]
pub async fn list_my_invitations(state: web::Data<AppState>, req: HttpRequest) -> impl Responder {
    let email = match claims_email(&req, state.get_ref()).await {
        Ok(e) => e,
        Err(resp) => return resp,
    };
    if let Err(resp) = expire_overdue_invitations(&state.db).await {
        return resp;
    }

    match sqlx::query_as::<_, Invitation>(
        "SELECT i.event_id,
                u.id AS user_id,
                fiestaaa_decrypt_text(u.email_ciphertext) AS email,
                u.handle,
                u.avatar_url,
                i.status,
                i.date_invi,
                e.name_event AS event_name
         FROM invitations i
         JOIN users u ON u.id = i.user_id
         JOIN events e ON e.event_id = i.event_id
         WHERE fiestaaa_email_matches(u.email_lookup_hash, $1)
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
        (status = 200, description = "Invitation updated", body = Invitation),
        (status = 400, description = "Invalid payload", body = ErrorResponse),
        (status = 401, description = "Authentication required", body = ErrorResponse),
        (status = 404, description = "Invitation not found", body = ErrorResponse)
    )
)]
#[patch("/my/invitations/{event_id}")]
pub async fn respond_invitation(
    state: web::Data<AppState>,
    req: HttpRequest,
    event_id: web::Path<i64>,
    payload: web::Json<InvitationPatchPayload>,
) -> impl Responder {
    let email = match claims_email(&req, state.get_ref()).await {
        Ok(e) => e,
        Err(resp) => return resp,
    };
    if let Err(resp) = ensure_event_writable(&state.db, *event_id).await {
        return resp;
    }

    let status = match payload.status.clone() {
        Some(s) if matches!(s.trim(), "Accepted" | "Declined") => s.trim().to_string(),
        _ => {
            return HttpResponse::BadRequest().json(ErrorResponse {
                error: "invalid_status".into(),
                details: Some("status must be Accepted or Declined".into()),
            });
        }
    };

    let user = match fetch_user_by_email(&state.db, &email).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let owner_email = match fetch_event_owner_email(&state.db, *event_id).await {
        Ok(email) => email,
        Err(resp) => return resp,
    };
    if let Err(resp) = expire_overdue_invitations(&state.db).await {
        return resp;
    }

    let target_status = status;
    let is_declined = target_status == "Declined";
    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(_) => {
            return HttpResponse::InternalServerError().json(ErrorResponse {
                error: "db_error".into(),
                details: None,
            });
        }
    };
    let current_status = sqlx::query_scalar::<_, String>(
        "SELECT status FROM invitations WHERE event_id = $1 AND user_id = $2 FOR UPDATE",
    )
    .bind(*event_id)
    .bind(user.id)
    .fetch_optional(&mut *tx)
    .await;

    let current_status = match current_status {
        Ok(Some(s)) => s,
        Ok(None) => {
            let _ = tx.rollback().await;
            return HttpResponse::NotFound().json(ErrorResponse {
                error: "invitation_not_found".into(),
                details: None,
            });
        }
        Err(_) => {
            let _ = tx.rollback().await;
            return HttpResponse::InternalServerError().json(ErrorResponse {
                error: "db_error".into(),
                details: None,
            });
        }
    };

    if current_status == "Expired" {
        let _ = tx.rollback().await;
        return HttpResponse::Gone().json(ErrorResponse {
            error: "invitation_expired".into(),
            details: Some("La date limite pour répondre est dépassée".into()),
        });
    }

    let res = sqlx::query_as::<_, Invitation>(
        "WITH updated_invitation AS (
            UPDATE invitations 
            SET status = $1 
            WHERE event_id = $2 AND user_id = $3
            RETURNING event_id, user_id, status, date_invi
         )
         SELECT
            ui.event_id,
            ui.user_id,
            fiestaaa_decrypt_text(u.email_ciphertext) AS email,
            u.handle,
            u.avatar_url,
            ui.status,
            ui.date_invi,
            e.name_event AS event_name
         FROM updated_invitation ui
         JOIN users u ON u.id = ui.user_id
         JOIN events e ON e.event_id = ui.event_id",
    )
    .bind(&target_status)
    .bind(*event_id)
    .bind(user.id)
    .fetch_optional(&mut *tx)
    .await;

    let updated = match res {
        Ok(Some(inv)) => inv,
        Ok(None) => {
            let _ = tx.rollback().await;
            return HttpResponse::NotFound().json(ErrorResponse {
                error: "invitation_not_found".into(),
                details: None,
            });
        }
        Err(_) => {
            let _ = tx.rollback().await;
            return HttpResponse::InternalServerError().json(ErrorResponse {
                error: "db_error".into(),
                details: None,
            });
        }
    };

    if is_declined {
        let reservations = sqlx::query(
            "SELECT item_id, quantity
             FROM user_items
             WHERE user_id = $1 AND event_id = $2
             FOR UPDATE",
        )
        .bind(user.id)
        .bind(*event_id)
        .fetch_all(&mut *tx)
        .await;

        let reservations = match reservations {
            Ok(rows) => rows,
            Err(_) => {
                let _ = tx.rollback().await;
                return HttpResponse::InternalServerError().json(ErrorResponse {
                    error: "db_error".into(),
                    details: None,
                });
            }
        };

        for row in reservations {
            let item_id: i64 = row.get("item_id");
            let qty: i32 = row.get("quantity");
            if (sqlx::query(
                "UPDATE events_items
                 SET quantity = GREATEST(quantity - $1, 0)
                 WHERE event_id = $2 AND item_id = $3",
            )
            .bind(qty)
            .bind(*event_id)
            .bind(item_id)
            .execute(&mut *tx)
            .await)
                .is_err()
            {
                let _ = tx.rollback().await;
                return HttpResponse::InternalServerError().json(ErrorResponse {
                    error: "db_error".into(),
                    details: None,
                });
            }
        }

        if (sqlx::query("DELETE FROM user_items WHERE user_id = $1 AND event_id = $2")
            .bind(user.id)
            .bind(*event_id)
            .execute(&mut *tx)
            .await)
            .is_err()
        {
            let _ = tx.rollback().await;
            return HttpResponse::InternalServerError().json(ErrorResponse {
                error: "db_error".into(),
                details: None,
            });
        }

        let driver_carpool_id = sqlx::query_scalar::<_, i64>(
            "SELECT carpool_id FROM carpools WHERE driver_id = $1 AND event_id = $2",
        )
        .bind(user.id)
        .bind(*event_id)
        .fetch_optional(&mut *tx)
        .await;

        if let Ok(Some(carpool_id)) = driver_carpool_id {
            if (sqlx::query("DELETE FROM carpool_passengers WHERE carpool_id = $1")
                .bind(carpool_id)
                .execute(&mut *tx)
                .await)
                .is_err()
            {
                let _ = tx.rollback().await;
                return HttpResponse::InternalServerError().json(ErrorResponse {
                    error: "db_error".into(),
                    details: None,
                });
            }
            if (sqlx::query("DELETE FROM carpools WHERE carpool_id = $1")
                .bind(carpool_id)
                .execute(&mut *tx)
                .await)
                .is_err()
            {
                let _ = tx.rollback().await;
                return HttpResponse::InternalServerError().json(ErrorResponse {
                    error: "db_error".into(),
                    details: None,
                });
            }

            publish_event(
                &state.redis_client,
                *event_id,
                &json!({"type": "carpool_deleted", "carpool_id": carpool_id}),
            )
            .await;
        }

        let passenger_carpool = sqlx::query_scalar::<_, i64>(
            "SELECT cp.carpool_id FROM carpool_passengers cp
             JOIN carpools c ON c.carpool_id = cp.carpool_id
             WHERE cp.user_id = $1 AND c.event_id = $2",
        )
        .bind(user.id)
        .bind(*event_id)
        .fetch_optional(&mut *tx)
        .await;

        if let Ok(Some(carpool_id)) = passenger_carpool {
            if (sqlx::query(
                "DELETE FROM carpool_passengers WHERE carpool_id = $1 AND user_id = $2",
            )
            .bind(carpool_id)
            .bind(user.id)
            .execute(&mut *tx)
            .await)
                .is_err()
            {
                let _ = tx.rollback().await;
                return HttpResponse::InternalServerError().json(ErrorResponse {
                    error: "db_error".into(),
                    details: None,
                });
            }

            if (sqlx::query(
                "UPDATE carpools SET seats_taken = GREATEST(seats_taken - 1, 0) WHERE carpool_id = $1",
            )
            .bind(carpool_id)
            .execute(&mut *tx)
            .await)
                .is_err()
            {
                let _ = tx.rollback().await;
                return HttpResponse::InternalServerError().json(ErrorResponse {
                    error: "db_error".into(),
                    details: None,
                });
            }

            publish_event(
                &state.redis_client,
                *event_id,
                &json!({"type": "carpool_left", "carpool_id": carpool_id, "user_id": user.id}),
            )
            .await;
        }
    }

    if (tx.commit().await).is_err() {
        return HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        });
    }

    publish_event_type(
        &state.redis_client,
        *event_id,
        event_types::EVENT_INVITATIONS_CHANGED,
    )
    .await;
    publish_global_type(&state.redis_client, event_types::INVITATIONS_CHANGED).await;
    if is_declined {
        publish_event(
            &state.redis_client,
            *event_id,
            &json!({
                "type": event_types::EVENT_ITEMS_CHANGED,
                "event_id": *event_id,
                "reason": "invitation_declined",
            }),
        )
        .await;
    }

    if !owner_email.eq_ignore_ascii_case(&email)
        && let Ok(Some(owner_id)) = find_user_id_by_email(&state.db, &owner_email).await
    {
        let status_label = if target_status == "Accepted" {
            "accepté"
        } else {
            "refusé"
        };
        let event_name = updated
            .event_name
            .clone()
            .unwrap_or("un événement".to_string());
        let author = updated.handle.as_deref().unwrap_or(updated.email.as_str());
        let title = "Réponse à ton invitation".to_string();
        let body = format!("{author} a {status_label} l'invitation à {event_name}");
        let dedup = format!("invite_response:{}:{}", *event_id, user.id);
        notify_users(
            &state.notifications,
            &state.db,
            &[owner_id],
            NotificationRequest {
                title: &title,
                body: &body,
                data: json!({
                    "type": "invite_response",
                    "event_id": *event_id,
                    "status": target_status,
                    "user_email": updated.email.clone(),
                    "user_handle": updated.handle.clone()
                }),
                dedup_base_key: Some(dedup.as_str()),
                dedup_ttl: Some(300),
            },
        )
        .await;
    }

    HttpResponse::Ok().json(updated)
}
