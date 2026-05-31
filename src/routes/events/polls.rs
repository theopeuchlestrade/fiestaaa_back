use actix_web::{Responder, delete, get, post, web};
use chrono::{Duration, Utc};
use std::collections::{HashMap, HashSet};

use super::*;

#[utoipa::path(
    get,
    path = "/events/{event_id}/polls",
    tag = "events",
    responses(
        (status = 200, description = "Polls associated with the event", body = [PollView]),
        (status = 403, description = "Unauthorized", body = ErrorResponse),
        (status = 404, description = "Event not found", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Event identifier")
    )
)]
#[get("/events/{event_id}/polls")]
pub async fn list_event_polls(
    state: web::Data<AppState>,
    req: HttpRequest,
    event_id: web::Path<i64>,
) -> impl Responder {
    let claims = match extract_active_claims_from_auth(&req, &state.db, &state.jwt_secret).await {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    if let Err(resp) = ensure_event_member(&req, state.get_ref(), *event_id).await {
        return resp;
    }

    let user_id = match fetch_user_id(&state.db, &claims.sub).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    match fetch_poll_views(state.get_ref(), *event_id, user_id).await {
        Ok(list) => HttpResponse::Ok().json(list),
        Err(resp) => resp,
    }
}

#[utoipa::path(
    post,
    path = "/events/{event_id}/polls",
    tag = "events",
    request_body = EventPollCreatePayload,
    responses(
        (status = 201, description = "Poll created", body = PollView),
        (status = 400, description = "Invalid payload", body = ErrorResponse),
        (status = 403, description = "Unauthorized", body = ErrorResponse),
        (status = 404, description = "Event not found", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Event identifier")
    )
)]
#[post("/events/{event_id}/polls")]
pub async fn create_event_poll(
    state: web::Data<AppState>,
    req: HttpRequest,
    event_id: web::Path<i64>,
    payload: web::Json<EventPollCreatePayload>,
) -> impl Responder {
    let claims = match extract_active_claims_from_auth(&req, &state.db, &state.jwt_secret).await {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    if let Err(resp) = ensure_event_owner(&req, state.get_ref(), *event_id).await {
        return resp;
    }
    if let Err(resp) = ensure_event_writable(&state.db, *event_id).await {
        return resp;
    }

    let creator_id = match fetch_user_id(&state.db, &claims.sub).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let payload = payload.into_inner();
    let question = payload.question.trim().to_string();
    if question.is_empty() {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("La question du sondage est requise".into()),
        });
    }

    let mut seen = HashSet::new();
    let mut options: Vec<String> = Vec::new();
    for raw in payload.options.into_iter() {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let key = trimmed.to_lowercase();
        if seen.insert(key) {
            options.push(trimmed.to_string());
        }
    }

    if options.len() < 2 {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("Au moins deux options distinctes sont requises".into()),
        });
    }
    if options.len() > 12 {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("Maximum 12 options par sondage".into()),
        });
    }

    if payload.duration_minutes <= 0 {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("Durée d'expiration invalide".into()),
        });
    }

    if let Err(resp) = ensure_event_exists(&state.db, *event_id).await {
        return resp;
    }

    let duration_minutes = payload.duration_minutes.min(60 * 24 * 7);
    let expires_at = Utc::now() + Duration::minutes(duration_minutes);
    let allow_multiple = payload.allow_multiple.unwrap_or(true);

    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(_) => return server_error(),
    };

    let poll_row = sqlx::query(
        "INSERT INTO event_polls (event_id, question, allow_multiple, expires_at, created_by)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING poll_id",
    )
    .bind(*event_id)
    .bind(&question)
    .bind(allow_multiple)
    .bind(expires_at)
    .bind(creator_id)
    .fetch_one(&mut *tx)
    .await;

    let poll_id: i64 = match poll_row {
        Ok(row) => row.get("poll_id"),
        Err(_) => {
            let _ = tx.rollback().await;
            return server_error();
        }
    };

    for (idx, label) in options.iter().enumerate() {
        if (sqlx::query(
            "INSERT INTO event_poll_options (poll_id, label, position) VALUES ($1, $2, $3)",
        )
        .bind(poll_id)
        .bind(label)
        .bind(idx as i32)
        .execute(&mut *tx)
        .await)
            .is_err()
        {
            let _ = tx.rollback().await;
            return server_error();
        }
    }

    if (tx.commit().await).is_err() {
        return server_error();
    }

    let poll = match fetch_poll_views(state.get_ref(), *event_id, creator_id).await {
        Ok(list) => list.into_iter().find(|p| p.poll_id == poll_id),
        Err(resp) => return resp,
    };

    let poll = match poll {
        Some(p) => p,
        None => return server_error(),
    };

    publish_event(
        &state.redis_client,
        *event_id,
        &json!({
            "type": event_types::EVENT_POLLS_CHANGED,
            "event_id": *event_id,
            "poll_id": poll_id
        }),
    )
    .await;

    if state.notifications.is_enabled()
        && let Ok(members) = event_member_user_ids(&state.db, *event_id).await
    {
        let event_name =
            sqlx::query_scalar::<_, String>("SELECT name_event FROM events WHERE event_id = $1")
                .bind(*event_id)
                .fetch_optional(&state.db)
                .await
                .ok()
                .flatten()
                .unwrap_or_else(|| "Événement".into());
        let sender = if claims.handle.trim().is_empty() {
            claims.sub.as_str()
        } else {
            claims.handle.as_str()
        };
        let body = format!("{sender} a lancé un sondage dans {event_name}");
        let dedup = format!("poll_created:{poll_id}");
        notify_users(
            &state.notifications,
            &state.db,
            &members,
            NotificationRequest {
                title: "Nouveau sondage",
                body: body.as_str(),
                data: json!({
                    "type": "poll_created",
                    "event_id": *event_id,
                    "poll_id": poll_id,
                    "question": question
                }),
                dedup_base_key: Some(&dedup),
                dedup_ttl: Some(600),
            },
        )
        .await;
    }

    HttpResponse::Created().json(poll)
}

#[utoipa::path(
    post,
    path = "/events/{event_id}/polls/{poll_id}/vote",
    tag = "events",
    request_body = EventPollVotePayload,
    responses(
        (status = 200, description = "Vote saved", body = PollView),
        (status = 400, description = "Invalid payload", body = ErrorResponse),
        (status = 403, description = "Unauthorized", body = ErrorResponse),
        (status = 404, description = "Poll not found", body = ErrorResponse),
        (status = 410, description = "Poll expired", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Event identifier"),
        ("poll_id" = i64, Path, description = "Poll identifier")
    )
)]
#[post("/events/{event_id}/polls/{poll_id}/vote")]
pub async fn vote_event_poll(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<(i64, i64)>,
    payload: web::Json<EventPollVotePayload>,
) -> impl Responder {
    let claims = match extract_active_claims_from_auth(&req, &state.db, &state.jwt_secret).await {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let user_id = match fetch_user_id(&state.db, &claims.sub).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let (event_id, poll_id) = path.into_inner();

    if let Err(resp) = ensure_event_member(&req, state.get_ref(), event_id).await {
        return resp;
    }
    if let Err(resp) = ensure_event_writable(&state.db, event_id).await {
        return resp;
    }

    let poll_row = sqlx::query(
        "SELECT event_id, allow_multiple, expires_at FROM event_polls WHERE poll_id = $1",
    )
    .bind(poll_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|_| server_error());

    let poll_row = match poll_row {
        Ok(Some(row)) => row,
        Ok(None) => {
            return HttpResponse::NotFound().json(ErrorResponse {
                error: "poll_not_found".into(),
                details: None,
            });
        }
        Err(resp) => return resp,
    };

    let poll_event_id: i64 = match poll_row.try_get("event_id") {
        Ok(id) => id,
        Err(_) => return server_error(),
    };
    if poll_event_id != event_id {
        return HttpResponse::NotFound().json(ErrorResponse {
            error: "poll_not_found".into(),
            details: None,
        });
    }

    let expires_at: chrono::DateTime<chrono::Utc> = match poll_row.try_get("expires_at") {
        Ok(dt) => dt,
        Err(_) => return server_error(),
    };
    if expires_at < Utc::now() {
        return HttpResponse::Gone().json(ErrorResponse {
            error: "poll_expired".into(),
            details: Some("Ce sondage est expiré.".into()),
        });
    }

    let allow_multiple: bool = match poll_row.try_get("allow_multiple") {
        Ok(v) => v,
        Err(_) => return server_error(),
    };

    let mut option_ids: Vec<i64> = payload.option_ids.to_vec();
    option_ids.sort();
    option_ids.dedup();

    if !allow_multiple && option_ids.len() > 1 {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("Ce sondage n'autorise qu'un seul choix.".into()),
        });
    }

    let valid_option_ids =
        sqlx::query_scalar::<_, i64>("SELECT option_id FROM event_poll_options WHERE poll_id = $1")
            .bind(poll_id)
            .fetch_all(&state.db)
            .await;

    let valid_option_ids = match valid_option_ids {
        Ok(list) => list,
        Err(_) => return server_error(),
    };

    let valid_set: HashSet<i64> = valid_option_ids.iter().copied().collect();
    if !option_ids.iter().all(|id| valid_set.contains(id)) {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_option".into(),
            details: Some("Option inconnue pour ce sondage".into()),
        });
    }

    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(_) => return server_error(),
    };

    if (sqlx::query("DELETE FROM event_poll_votes WHERE poll_id = $1 AND user_id = $2")
        .bind(poll_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await)
        .is_err()
    {
        let _ = tx.rollback().await;
        return server_error();
    }

    if !option_ids.is_empty() {
        for opt_id in option_ids {
            if (sqlx::query(
                "INSERT INTO event_poll_votes (poll_id, option_id, user_id) VALUES ($1, $2, $3)",
            )
            .bind(poll_id)
            .bind(opt_id)
            .bind(user_id)
            .execute(&mut *tx)
            .await)
                .is_err()
            {
                let _ = tx.rollback().await;
                return server_error();
            }
        }
    }

    if (tx.commit().await).is_err() {
        return server_error();
    }

    let poll = match fetch_poll_views(state.get_ref(), event_id, user_id).await {
        Ok(list) => list.into_iter().find(|p| p.poll_id == poll_id),
        Err(resp) => return resp,
    };

    let poll = match poll {
        Some(p) => p,
        None => return server_error(),
    };

    publish_event(
        &state.redis_client,
        event_id,
        &json!({
            "type": event_types::EVENT_POLLS_CHANGED,
            "event_id": event_id,
            "poll_id": poll_id
        }),
    )
    .await;

    HttpResponse::Ok().json(poll)
}

#[utoipa::path(
    delete,
    path = "/events/{event_id}/polls/{poll_id}",
    tag = "events",
    responses(
        (status = 200, description = "Poll deleted", body = StatusResponse),
        (status = 403, description = "Unauthorized", body = ErrorResponse),
        (status = 404, description = "Poll not found", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Event identifier"),
        ("poll_id" = i64, Path, description = "Poll identifier")
    )
)]
#[delete("/events/{event_id}/polls/{poll_id}")]
pub async fn delete_event_poll(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<(i64, i64)>,
) -> impl Responder {
    let claims = match extract_active_claims_from_auth(&req, &state.db, &state.jwt_secret).await {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let (event_id, poll_id) = path.into_inner();

    if let Err(resp) = ensure_event_exists(&state.db, event_id).await {
        return resp;
    }
    if let Err(resp) = ensure_event_writable(&state.db, event_id).await {
        return resp;
    }

    let poll_row = sqlx::query("SELECT event_id, created_by FROM event_polls WHERE poll_id = $1")
        .bind(poll_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|_| server_error());

    let poll_row = match poll_row {
        Ok(Some(row)) => row,
        Ok(None) => {
            return HttpResponse::NotFound().json(ErrorResponse {
                error: "poll_not_found".into(),
                details: None,
            });
        }
        Err(resp) => return resp,
    };

    let poll_event_id: i64 = match poll_row.try_get("event_id") {
        Ok(id) => id,
        Err(_) => return server_error(),
    };
    if poll_event_id != event_id {
        return HttpResponse::NotFound().json(ErrorResponse {
            error: "poll_not_found".into(),
            details: None,
        });
    }

    let creator_id: Option<i64> = poll_row.try_get("created_by").ok();
    let user_id = match fetch_user_id(&state.db, &claims.sub).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let owner_id = match fetch_event_owner_id(&state.db, event_id).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let is_owner = owner_id == user_id;
    let is_creator = creator_id.is_some_and(|id| id == user_id);
    if !is_owner && !is_creator {
        return HttpResponse::Forbidden().json(ErrorResponse {
            error: "forbidden".into(),
            details: Some("Seul l'organisateur ou le créateur peut supprimer ce sondage".into()),
        });
    }

    let res = sqlx::query("DELETE FROM event_polls WHERE poll_id = $1")
        .bind(poll_id)
        .execute(&state.db)
        .await;

    match res {
        Ok(result) if result.rows_affected() == 0 => HttpResponse::NotFound().json(ErrorResponse {
            error: "poll_not_found".into(),
            details: None,
        }),
        Ok(_) => {
            publish_event(
                &state.redis_client,
                event_id,
                &json!({
                    "type": event_types::EVENT_POLLS_CHANGED,
                    "event_id": event_id,
                    "poll_id": poll_id,
                    "deleted": true
                }),
            )
            .await;
            HttpResponse::Ok().json(StatusResponse {
                status: "deleted".into(),
            })
        }
        Err(_) => server_error(),
    }
}

async fn fetch_poll_views(
    state: &AppState,
    event_id: i64,
    user_id: i64,
) -> Result<Vec<PollView>, HttpResponse> {
    let poll_rows = sqlx::query(
        "SELECT p.poll_id,
                p.question,
                p.allow_multiple,
                p.expires_at,
                p.created_at,
                fiestaaa_decrypt_text(u.email_ciphertext) AS created_by_email
         FROM event_polls p
         LEFT JOIN users u ON u.id = p.created_by
         WHERE p.event_id = $1
         ORDER BY p.created_at DESC",
    )
    .bind(event_id)
    .fetch_all(&state.db)
    .await
    .map_err(|_| server_error())?;

    if poll_rows.is_empty() {
        return Ok(Vec::new());
    }

    let poll_ids: Vec<i64> = poll_rows
        .iter()
        .filter_map(|row| row.try_get("poll_id").ok())
        .collect();
    let option_rows = sqlx::query(
        "SELECT option_id, poll_id, label, position
         FROM event_poll_options
         WHERE poll_id = ANY($1)
         ORDER BY position, option_id",
    )
    .bind(&poll_ids)
    .fetch_all(&state.db)
    .await
    .map_err(|_| server_error())?;

    let vote_rows = sqlx::query(
        "SELECT v.poll_id,
                v.option_id,
                v.user_id,
                fiestaaa_decrypt_text(u.email_ciphertext) AS email,
                u.handle,
                u.avatar_url
         FROM event_poll_votes v
         JOIN users u ON u.id = v.user_id
         WHERE v.poll_id = ANY($1)",
    )
    .bind(&poll_ids)
    .fetch_all(&state.db)
    .await
    .map_err(|_| server_error())?;

    let mut polls: Vec<PollView> = poll_rows
        .into_iter()
        .filter_map(|row| {
            let expires_at: chrono::DateTime<chrono::Utc> = row.try_get("expires_at").ok()?;
            let created_at: chrono::DateTime<chrono::Utc> = row.try_get("created_at").ok()?;
            let poll_id: i64 = row.try_get("poll_id").ok()?;
            let question: String = row.try_get("question").ok()?;
            let allow_multiple: bool = row.try_get("allow_multiple").ok()?;
            let created_by_email: Option<String> = row.try_get("created_by_email").ok();
            Some(PollView {
                poll_id,
                event_id,
                question,
                allow_multiple,
                expires_at,
                created_at,
                created_by_email,
                options: Vec::new(),
                my_votes: Vec::new(),
                total_votes: 0,
                has_expired: expires_at < Utc::now(),
            })
        })
        .collect();

    let mut options_by_poll: HashMap<i64, Vec<(PollOptionView, i32)>> = HashMap::new();
    for row in option_rows {
        let poll_id: i64 = row.get("poll_id");
        let option_id: i64 = row.get("option_id");
        let label: String = row.get("label");
        let position: i32 = row.get("position");
        options_by_poll.entry(poll_id).or_default().push((
            PollOptionView {
                option_id,
                label,
                vote_count: 0,
                voters: Vec::new(),
            },
            position,
        ));
    }

    for vote_row in vote_rows {
        let poll_id: i64 = vote_row.get("poll_id");
        let option_id: i64 = vote_row.get("option_id");
        let voter_id: i64 = vote_row.get("user_id");
        if let Some(poll) = polls.iter_mut().find(|p| p.poll_id == poll_id) {
            poll.total_votes += 1;
            if voter_id == user_id {
                poll.my_votes.push(option_id);
            }
        }
        if let Some(options) = options_by_poll.get_mut(&poll_id)
            && let Some((opt, _)) = options
                .iter_mut()
                .find(|(opt, _)| opt.option_id == option_id)
        {
            opt.vote_count += 1;
            opt.voters.push(PollOptionVoter {
                email: vote_row.get("email"),
                handle: vote_row.get("handle"),
                avatar_url: vote_row.get("avatar_url"),
            });
        }
    }

    for poll in &mut polls {
        if let Some(mut opts) = options_by_poll.remove(&poll.poll_id) {
            opts.sort_by_key(|(_, pos)| *pos);
            poll.options = opts.into_iter().map(|(opt, _)| opt).collect();
        }
        poll.my_votes.sort_unstable();
        poll.my_votes.dedup();
    }

    Ok(polls)
}
