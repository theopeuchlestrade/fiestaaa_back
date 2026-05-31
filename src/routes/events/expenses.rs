use actix_web::{Responder, delete, get, post, web};
use chrono::Utc;
use std::collections::{HashMap, HashSet};

use super::*;

#[utoipa::path(
    get,
    path = "/events/{event_id}/expenses",
    tag = "events",
    responses(
        (status = 200, description = "Event shared expenses", body = [EventExpenseView]),
        (status = 403, description = "Unauthorized", body = ErrorResponse),
        (status = 404, description = "Event not found", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Event identifier")
    )
)]
#[get("/events/{event_id}/expenses")]
pub async fn list_event_expenses(
    state: web::Data<AppState>,
    req: HttpRequest,
    event_id: web::Path<i64>,
) -> impl Responder {
    if let Err(resp) = ensure_event_member(&req, state.get_ref(), *event_id).await {
        return resp;
    }

    match fetch_event_expenses(&state.db, *event_id).await {
        Ok(expenses) => HttpResponse::Ok().json(expenses),
        Err(resp) => resp,
    }
}

#[utoipa::path(
    post,
    path = "/events/{event_id}/expenses",
    tag = "events",
    request_body = EventExpensePayload,
    responses(
        (status = 201, description = "Expense created", body = EventExpenseView),
        (status = 400, description = "Invalid payload", body = ErrorResponse),
        (status = 403, description = "Unauthorized", body = ErrorResponse),
        (status = 404, description = "Event not found", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Event identifier")
    )
)]
#[post("/events/{event_id}/expenses")]
pub async fn create_event_expense(
    state: web::Data<AppState>,
    req: HttpRequest,
    event_id: web::Path<i64>,
    payload: web::Json<EventExpensePayload>,
) -> impl Responder {
    let claims = match extract_active_claims_from_auth(&req, &state.db, &state.jwt_secret).await {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    if let Err(resp) = ensure_event_member(&req, state.get_ref(), *event_id).await {
        return resp;
    }
    if let Err(resp) = ensure_event_writable(&state.db, *event_id).await {
        return resp;
    }

    let requester_id = match fetch_user_id(&state.db, &claims.sub).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let payload = payload.into_inner();
    let title = payload.title.trim().to_string();
    if title.is_empty() {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("Le titre de la dépense est requis".into()),
        });
    }
    if payload.amount_cents <= 0 {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("Le montant doit être supérieur à 0".into()),
        });
    }

    let mut participant_user_ids = payload.participant_user_ids;
    participant_user_ids.sort_unstable();
    participant_user_ids.dedup();
    if participant_user_ids.is_empty() {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("Au moins un participant est requis pour la dépense".into()),
        });
    }

    let members = match fetch_event_member_directory(&state.db, *event_id).await {
        Ok(values) => values,
        Err(resp) => return resp,
    };
    let member_ids: HashSet<i64> = members.keys().copied().collect();
    let paid_by_user_id = payload.paid_by_user_id.unwrap_or(requester_id);
    if !member_ids.contains(&paid_by_user_id) {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payer".into(),
            details: Some("Le payeur doit appartenir à la fiestaaa".into()),
        });
    }
    if !participant_user_ids
        .iter()
        .all(|user_id| member_ids.contains(user_id))
    {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_participants".into(),
            details: Some("Tous les participants doivent appartenir à la fiestaaa".into()),
        });
    }

    let note = payload
        .note
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let expense_date = payload.expense_date.unwrap_or_else(Utc::now);

    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(_) => return server_error(),
    };

    let expense_id = match sqlx::query_scalar::<_, i64>(
        "INSERT INTO event_expenses (event_id, paid_by_user_id, created_by_user_id, title, amount_cents, note, expense_date)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         RETURNING expense_id",
    )
    .bind(*event_id)
    .bind(paid_by_user_id)
    .bind(requester_id)
    .bind(&title)
    .bind(payload.amount_cents)
    .bind(&note)
    .bind(expense_date)
    .fetch_one(&mut *tx)
    .await
    {
        Ok(id) => id,
        Err(_) => {
            let _ = tx.rollback().await;
            return server_error();
        }
    };

    for participant_user_id in participant_user_ids {
        if (sqlx::query(
            "INSERT INTO event_expense_participants (expense_id, user_id, share_weight)
             VALUES ($1, $2, 1)",
        )
        .bind(expense_id)
        .bind(participant_user_id)
        .execute(&mut *tx)
        .await)
            .is_err()
        {
            let _ = tx.rollback().await;
            return server_error();
        }
    }

    if tx.commit().await.is_err() {
        return server_error();
    }

    publish_event(
        &state.redis_client,
        *event_id,
        &json!({
            "type": "event_expenses_changed",
            "event_id": *event_id,
            "expense_id": expense_id,
        }),
    )
    .await;

    match fetch_event_expense_view(&state.db, expense_id).await {
        Ok(expense) => HttpResponse::Created().json(expense),
        Err(resp) => resp,
    }
}

#[utoipa::path(
    delete,
    path = "/events/{event_id}/expenses/{expense_id}",
    tag = "events",
    responses(
        (status = 200, description = "Expense deleted", body = StatusResponse),
        (status = 403, description = "Unauthorized", body = ErrorResponse),
        (status = 404, description = "Expense not found", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Event identifier"),
        ("expense_id" = i64, Path, description = "Expense identifier")
    )
)]
#[delete("/events/{event_id}/expenses/{expense_id}")]
pub async fn delete_event_expense(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<(i64, i64)>,
) -> impl Responder {
    let claims = match extract_active_claims_from_auth(&req, &state.db, &state.jwt_secret).await {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let (event_id, expense_id) = path.into_inner();

    if let Err(resp) = ensure_event_member(&req, state.get_ref(), event_id).await {
        return resp;
    }
    if let Err(resp) = ensure_event_writable(&state.db, event_id).await {
        return resp;
    }

    let requester_id = match fetch_user_id(&state.db, &claims.sub).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let owner_id = match fetch_event_owner_id(&state.db, event_id).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let is_owner = owner_id == requester_id;

    let expense_row = match sqlx::query(
        "SELECT paid_by_user_id, created_by_user_id
         FROM event_expenses
         WHERE expense_id = $1 AND event_id = $2",
    )
    .bind(expense_id)
    .bind(event_id)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(row)) => row,
        Ok(None) => {
            return HttpResponse::NotFound().json(ErrorResponse {
                error: "expense_not_found".into(),
                details: None,
            });
        }
        Err(_) => return server_error(),
    };

    let paid_by_user_id: i64 = match expense_row.try_get("paid_by_user_id") {
        Ok(value) => value,
        Err(_) => return server_error(),
    };
    let created_by_user_id: i64 = match expense_row.try_get("created_by_user_id") {
        Ok(value) => value,
        Err(_) => return server_error(),
    };
    if !is_owner && requester_id != paid_by_user_id && requester_id != created_by_user_id {
        return HttpResponse::Forbidden().json(ErrorResponse {
            error: "forbidden".into(),
            details: Some(
                "Seuls le créateur, le payeur ou l'organisateur peuvent supprimer cette dépense"
                    .into(),
            ),
        });
    }

    match sqlx::query("DELETE FROM event_expenses WHERE expense_id = $1 AND event_id = $2")
        .bind(expense_id)
        .bind(event_id)
        .execute(&state.db)
        .await
    {
        Ok(result) if result.rows_affected() == 0 => HttpResponse::NotFound().json(ErrorResponse {
            error: "expense_not_found".into(),
            details: None,
        }),
        Ok(_) => {
            publish_event(
                &state.redis_client,
                event_id,
                &json!({
                    "type": "event_expenses_changed",
                    "event_id": event_id,
                    "expense_id": expense_id,
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

#[utoipa::path(
    get,
    path = "/events/{event_id}/expenses/summary",
    tag = "events",
    responses(
        (status = 200, description = "Shared expense summary", body = EventExpensesSummaryView),
        (status = 403, description = "Unauthorized", body = ErrorResponse),
        (status = 404, description = "Event not found", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Event identifier")
    )
)]
#[get("/events/{event_id}/expenses/summary")]
pub async fn get_event_expenses_summary(
    state: web::Data<AppState>,
    req: HttpRequest,
    event_id: web::Path<i64>,
) -> impl Responder {
    if let Err(resp) = ensure_event_member(&req, state.get_ref(), *event_id).await {
        return resp;
    }

    match build_event_expenses_summary(&state.db, *event_id).await {
        Ok(summary) => HttpResponse::Ok().json(summary),
        Err(resp) => resp,
    }
}

async fn fetch_event_member_directory(
    db: &PgPool,
    event_id: i64,
) -> Result<HashMap<i64, (Option<String>, Option<String>)>, HttpResponse> {
    let rows = sqlx::query(
        "SELECT DISTINCT u.id, u.handle, u.avatar_url
         FROM users u
         WHERE u.id = (SELECT owner_user_id FROM events WHERE event_id = $1)
         UNION
         SELECT DISTINCT u.id, u.handle, u.avatar_url
         FROM invitations i
         JOIN users u ON u.id = i.user_id
         WHERE i.event_id = $1
           AND i.status = 'Accepted'",
    )
    .bind(event_id)
    .fetch_all(db)
    .await
    .map_err(|_| server_error())?;

    let mut members = HashMap::new();
    for row in rows {
        let user_id: i64 = row.get("id");
        let handle: Option<String> = row.get("handle");
        let avatar_url: Option<String> = row.get("avatar_url");
        members.insert(user_id, (handle, avatar_url));
    }
    Ok(members)
}

async fn fetch_event_expense_view(
    db: &PgPool,
    expense_id: i64,
) -> Result<EventExpenseView, HttpResponse> {
    let expenses = fetch_event_expenses_by_ids(db, &[expense_id]).await?;
    expenses.into_iter().next().ok_or_else(|| {
        HttpResponse::NotFound().json(ErrorResponse {
            error: "expense_not_found".into(),
            details: None,
        })
    })
}

async fn fetch_event_expenses(
    db: &PgPool,
    event_id: i64,
) -> Result<Vec<EventExpenseView>, HttpResponse> {
    let expense_ids = sqlx::query_scalar::<_, i64>(
        "SELECT expense_id
         FROM event_expenses
         WHERE event_id = $1
         ORDER BY expense_date DESC, expense_id DESC",
    )
    .bind(event_id)
    .fetch_all(db)
    .await
    .map_err(|_| server_error())?;

    fetch_event_expenses_by_ids(db, &expense_ids).await
}

async fn fetch_event_expenses_by_ids(
    db: &PgPool,
    expense_ids: &[i64],
) -> Result<Vec<EventExpenseView>, HttpResponse> {
    if expense_ids.is_empty() {
        return Ok(Vec::new());
    }

    let expense_rows = sqlx::query(
        "SELECT ee.expense_id,
                ee.event_id,
                ee.paid_by_user_id,
                u.handle AS paid_by_handle,
                u.avatar_url AS paid_by_avatar_url,
                ee.title,
                ee.amount_cents,
                ee.note,
                ee.expense_date,
                ee.created_at
         FROM event_expenses ee
         JOIN users u ON u.id = ee.paid_by_user_id
         WHERE ee.expense_id = ANY($1)
         ORDER BY ee.expense_date DESC, ee.expense_id DESC",
    )
    .bind(expense_ids)
    .fetch_all(db)
    .await
    .map_err(|_| server_error())?;

    let participant_rows = sqlx::query(
        "SELECT ep.expense_id, ep.user_id, u.handle, u.avatar_url
         FROM event_expense_participants ep
         JOIN users u ON u.id = ep.user_id
         WHERE ep.expense_id = ANY($1)
         ORDER BY ep.expense_id, lower(u.handle), ep.user_id",
    )
    .bind(expense_ids)
    .fetch_all(db)
    .await
    .map_err(|_| server_error())?;

    let mut participants_by_expense: HashMap<i64, Vec<EventExpenseParticipantView>> =
        HashMap::new();
    for row in participant_rows {
        let expense_id: i64 = row.get("expense_id");
        participants_by_expense
            .entry(expense_id)
            .or_default()
            .push(EventExpenseParticipantView {
                user_id: row.get("user_id"),
                handle: row.get("handle"),
                avatar_url: row.get("avatar_url"),
            });
    }

    let mut expenses = Vec::new();
    for row in expense_rows {
        let expense_id: i64 = row.get("expense_id");
        expenses.push(EventExpenseView {
            expense_id,
            event_id: row.get("event_id"),
            paid_by_user_id: row.get("paid_by_user_id"),
            paid_by_handle: row.get("paid_by_handle"),
            paid_by_avatar_url: row.get("paid_by_avatar_url"),
            title: row.get("title"),
            amount_cents: row.get("amount_cents"),
            note: row.get("note"),
            expense_date: row.get("expense_date"),
            created_at: row.get("created_at"),
            participants: participants_by_expense
                .remove(&expense_id)
                .unwrap_or_default(),
        });
    }

    Ok(expenses)
}

async fn build_event_expenses_summary(
    db: &PgPool,
    event_id: i64,
) -> Result<EventExpensesSummaryView, HttpResponse> {
    let _ = fetch_event_timing(db, event_id).await?;
    let members = fetch_event_member_directory(db, event_id).await?;
    let expenses = fetch_event_expenses(db, event_id).await?;

    let mut balances_by_user: HashMap<i64, (i64, i64)> = members
        .keys()
        .copied()
        .map(|user_id| (user_id, (0, 0)))
        .collect();
    let mut total_expenses_cents = 0_i64;

    for expense in expenses {
        total_expenses_cents += expense.amount_cents;
        balances_by_user
            .entry(expense.paid_by_user_id)
            .or_insert((0, 0))
            .0 += expense.amount_cents;

        let mut participant_ids: Vec<i64> = expense
            .participants
            .into_iter()
            .map(|item| item.user_id)
            .collect();
        participant_ids.sort_unstable();
        participant_ids.dedup();
        if participant_ids.is_empty() {
            continue;
        }

        let participant_count = participant_ids.len() as i64;
        let share_cents = expense.amount_cents / participant_count;
        let remainder = expense.amount_cents % participant_count;

        for (index, participant_id) in participant_ids.into_iter().enumerate() {
            let extra_cent = if (index as i64) < remainder { 1 } else { 0 };
            balances_by_user.entry(participant_id).or_insert((0, 0)).1 += share_cents + extra_cent;
        }
    }

    let mut balances: Vec<EventExpenseBalanceView> = balances_by_user
        .into_iter()
        .map(|(user_id, (paid_cents, owed_cents))| {
            let (handle, avatar_url) = members.get(&user_id).cloned().unwrap_or((None, None));
            EventExpenseBalanceView {
                user_id,
                handle,
                avatar_url,
                paid_cents,
                owed_cents,
                balance_cents: paid_cents - owed_cents,
            }
        })
        .collect();

    balances.sort_by(|a, b| {
        b.balance_cents
            .cmp(&a.balance_cents)
            .then_with(|| a.handle.cmp(&b.handle))
            .then_with(|| a.user_id.cmp(&b.user_id))
    });

    let mut creditors: Vec<(usize, i64)> = balances
        .iter()
        .enumerate()
        .filter_map(|(index, item)| (item.balance_cents > 0).then_some((index, item.balance_cents)))
        .collect();
    let mut debtors: Vec<(usize, i64)> = balances
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            (item.balance_cents < 0).then_some((index, -item.balance_cents))
        })
        .collect();

    let mut settlements = Vec::new();
    let mut creditor_index = 0_usize;
    let mut debtor_index = 0_usize;
    while creditor_index < creditors.len() && debtor_index < debtors.len() {
        let (creditor_balance_index, creditor_amount) = creditors[creditor_index];
        let (debtor_balance_index, debtor_amount) = debtors[debtor_index];
        let transfer_amount = creditor_amount.min(debtor_amount);

        settlements.push(EventExpenseSettlementView {
            from_user_id: balances[debtor_balance_index].user_id,
            from_handle: balances[debtor_balance_index].handle.clone(),
            to_user_id: balances[creditor_balance_index].user_id,
            to_handle: balances[creditor_balance_index].handle.clone(),
            amount_cents: transfer_amount,
        });

        creditors[creditor_index].1 -= transfer_amount;
        debtors[debtor_index].1 -= transfer_amount;
        if creditors[creditor_index].1 == 0 {
            creditor_index += 1;
        }
        if debtors[debtor_index].1 == 0 {
            debtor_index += 1;
        }
    }

    Ok(EventExpensesSummaryView {
        currency: "EUR".into(),
        total_expenses_cents,
        balances,
        settlements,
    })
}
