mod common;

use actix_web::{App, http::StatusCode, test};
use chrono::{NaiveDate, NaiveTime};
use common::{DB_LOCK, build_state, obtain_pool, reset_tables};
use fiestaaa_back::{
    auth::{encode_jwt, hash_password, now_ts},
    models::Claims,
    routes,
};
use serde_json::json;
use sqlx::PgPool;
use std::{error::Error, time::Duration};

fn make_token(secret: &str, email: &str, handle: &str) -> Option<String> {
    let claims = Claims {
        sub: email.to_string(),
        exp: (now_ts() + 3600) as usize,
        handle: handle.to_string(),
    };
    encode_jwt(&claims, secret).ok()
}

async fn seed_user(pool: &PgPool, email: &str, handle: &str) -> sqlx::Result<i64> {
    let hash = hash_password("StrongPassw0rd!").expect("hash");
    sqlx::query_scalar::<_, i64>(
        "INSERT INTO users (email_ciphertext, email_lookup_hash, password_hash, handle)
         VALUES (fiestaaa_encrypt_text($1), fiestaaa_email_lookup($1), $2, $3)
         RETURNING id",
    )
    .bind(email)
    .bind(hash)
    .bind(handle)
    .fetch_one(pool)
    .await
}

async fn ensure_user(pool: &PgPool, email: &str) -> sqlx::Result<i64> {
    if let Some(user_id) = sqlx::query_scalar::<_, i64>(
        "SELECT id FROM users WHERE email_lookup_hash = fiestaaa_email_lookup($1)",
    )
    .bind(email)
    .fetch_optional(pool)
    .await?
    {
        return Ok(user_id);
    }

    let handle = email.split('@').next().unwrap_or("user");
    seed_user(pool, email, handle).await
}

async fn seed_event(pool: &PgPool, owner_email: &str) -> sqlx::Result<i64> {
    let owner_user_id = ensure_user(pool, owner_email).await?;

    sqlx::query_scalar::<_, i64>(
        "INSERT INTO events (
            name_event,
            description,
            date_event,
            start_time,
            address_ciphertext,
            owner_user_id
         )
         VALUES ($1, $2, $3, $4, fiestaaa_encrypt_text($5), $6)
         RETURNING event_id",
    )
    .bind("Realtime Event")
    .bind("A realtime-secured event")
    .bind(NaiveDate::from_ymd_opt(2099, 1, 1).unwrap())
    .bind(NaiveTime::from_hms_opt(20, 0, 0).unwrap())
    .bind("123 Test Street")
    .bind(owner_user_id)
    .fetch_one(pool)
    .await
}

async fn seed_poll(pool: &PgPool, event_id: i64, multiple: bool) -> sqlx::Result<(i64, i64, i64)> {
    let poll = sqlx::query_scalar::<_, i64>(
        "INSERT INTO event_polls (event_id, question, allow_multiple, expires_at) VALUES ($1, 'Choose', $2, NOW() + INTERVAL '1 hour') RETURNING poll_id"
    ).bind(event_id).bind(multiple).fetch_one(pool).await?;
    let options = sqlx::query_scalar::<_, i64>(
        "INSERT INTO event_poll_options (poll_id, label, position) VALUES ($1, 'A', 0), ($1, 'B', 1) RETURNING option_id"
    ).bind(poll).fetch_all(pool).await?;
    Ok((poll, options[0], options[1]))
}

async fn wait_for_blocked_votes(pool: &PgPool, count: i64) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let blocked: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_stat_activity WHERE wait_event_type = 'Lock' AND query LIKE 'SELECT event_id, allow_multiple, expires_at FROM event_polls%'"
            ).fetch_one(pool).await.expect("inspect waiting votes");
            if blocked >= count { break; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("votes should wait for the poll lock");
}

#[actix_web::test]
async fn votes_are_serialized_and_expiration_is_checked_after_waiting() -> Result<(), Box<dyn Error>>
{
    let Some(pool) = obtain_pool().await else {
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["users", "events"]).await?;
    let email = "poll-owner@example.com";
    let event = seed_event(&pool, email).await?;
    let (poll, a, b) = seed_poll(&pool, event, false).await?;
    let secret = "poll-test-secret";
    let token = make_token(secret, email, "poll-owner").unwrap();
    let app = test::init_service(
        App::new()
            .app_data(build_state(pool.clone(), secret, &[]))
            .configure(routes::configure),
    )
    .await;
    let uri = format!("/events/{event}/polls/{poll}/vote");
    let request = |choices: Vec<i64>| {
        test::TestRequest::post()
            .uri(&uri)
            .insert_header(("Authorization", format!("Bearer {token}")))
            .set_json(json!({"option_ids": choices}))
            .to_request()
    };

    let mut lock = pool.begin().await?;
    sqlx::query("SELECT poll_id FROM event_polls WHERE poll_id=$1 FOR UPDATE")
        .bind(poll)
        .execute(&mut *lock)
        .await?;
    let release = async {
        wait_for_blocked_votes(&pool, 2).await;
        lock.commit().await.unwrap();
    };
    let (first, second, ()) = tokio::join!(
        test::call_service(&app, request(vec![a])),
        test::call_service(&app, request(vec![b])),
        release,
    );
    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(second.status(), StatusCode::OK);
    let choices: Vec<i64> =
        sqlx::query_scalar("SELECT option_id FROM event_poll_votes WHERE poll_id=$1")
            .bind(poll)
            .fetch_all(&pool)
            .await?;
    assert_eq!(choices.len(), 1);
    assert!(choices[0] == a || choices[0] == b);

    // Empty submissions withdraw a vote; multiple choices are rejected intact.
    assert_eq!(
        test::call_service(&app, request(vec![a, b])).await.status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        test::call_service(&app, request(vec![])).await.status(),
        StatusCode::OK
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM event_poll_votes WHERE poll_id=$1")
        .bind(poll)
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 0);

    sqlx::query("UPDATE event_polls SET allow_multiple=true WHERE poll_id=$1")
        .bind(poll)
        .execute(&pool)
        .await?;
    assert_eq!(
        test::call_service(&app, request(vec![a, b, a]))
            .await
            .status(),
        StatusCode::OK
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM event_poll_votes WHERE poll_id=$1")
        .bind(poll)
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 2);

    let mut lock = pool.begin().await?;
    sqlx::query("SELECT poll_id FROM event_polls WHERE poll_id=$1 FOR UPDATE")
        .bind(poll)
        .execute(&mut *lock)
        .await?;
    let expire = async {
        wait_for_blocked_votes(&pool, 1).await;
        sqlx::query(
            "UPDATE event_polls SET expires_at=NOW() - INTERVAL '1 second' WHERE poll_id=$1",
        )
        .bind(poll)
        .execute(&mut *lock)
        .await
        .unwrap();
        lock.commit().await.unwrap();
    };
    let (response, ()) = tokio::join!(test::call_service(&app, request(vec![])), expire);
    assert_eq!(response.status(), StatusCode::GONE);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM event_poll_votes WHERE poll_id=$1")
        .bind(poll)
        .fetch_one(&pool)
        .await?;
    assert_eq!(
        count, 2,
        "expired vote must leave the previous selection intact"
    );
    Ok(())
}
