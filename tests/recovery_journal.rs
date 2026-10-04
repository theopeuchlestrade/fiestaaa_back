mod common;
use actix_web::{App, http::StatusCode, test};
use fiestaaa_back::{
    auth::{encode_jwt, now_ts},
    models::Claims,
    routes,
};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

#[actix_web::test]
async fn migration_captures_existing_pending_apple_job_without_changing_its_ciphertext() {
    let pool = common::obtain_pool().await.unwrap();
    let _lock = common::DB_LOCK.lock().await;
    let mut tx = pool.begin().await.unwrap();
    // A disposable schema contains only the pre-migration shapes needed here.
    // Rollback removes it without altering the fully migrated test database.
    sqlx::raw_sql("CREATE SCHEMA recovery_backfill_test; SET LOCAL search_path TO recovery_backfill_test,public;
        CREATE TABLE users(dummy INT); CREATE TABLE events(dummy INT);
        CREATE TABLE abuse_reports(dummy INT); CREATE TABLE moderation_terms(term TEXT);
        CREATE TABLE apple_revocations(id BIGSERIAL,client_id TEXT,refresh_token_ciphertext BYTEA);
        INSERT INTO apple_revocations(client_id,refresh_token_ciphertext)
        VALUES('legacy.synthetic',fiestaaa_encrypt_text('synthetic-existing-token'));")
        .execute(&mut *tx).await.unwrap();
    let before: Vec<u8> =
        sqlx::query_scalar("SELECT refresh_token_ciphertext FROM apple_revocations")
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    sqlx::raw_sql(include_str!("../migrations/012_recovery_decisions.sql"))
        .execute(&mut *tx)
        .await
        .unwrap();
    let (key, after): (String, Vec<u8>) =
        sqlx::query_as("SELECT recovery_key,refresh_token_ciphertext FROM apple_revocations")
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    assert_eq!(key.len(), 64);
    assert_eq!(before, after);
    let (action, payload): (String, String) = sqlx::query_as(
        "SELECT action,fiestaaa_decrypt_text(payload_ciphertext) FROM recovery_decisions",
    )
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    assert_eq!(action, "apple_pending");
    let payload: Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(payload["recovery_key"], key);
    assert_eq!(payload["client_id"], "legacy.synthetic");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM recovery_journal_meta")
            .fetch_one(&mut *tx)
            .await
            .unwrap(),
        1
    );
    tx.rollback().await.unwrap();
}

async fn seed(pool: &PgPool) -> (i64, Uuid) {
    sqlx::query_as("INSERT INTO users(email_ciphertext,email_lookup_hash,password_hash,handle) VALUES(fiestaaa_encrypt_text('recovery@example.invalid'),fiestaaa_email_lookup('recovery@example.invalid'),'synthetic-hash','recoveryqa') RETURNING id,public_id")
        .fetch_one(pool).await.unwrap()
}
async fn records(pool: &PgPool) -> Vec<(String, Value)> {
    let rows: Vec<(String,String)> = sqlx::query_as("SELECT action,fiestaaa_decrypt_text(payload_ciphertext) FROM recovery_decisions ORDER BY sequence")
        .fetch_all(pool).await.unwrap();
    rows.into_iter()
        .map(|(action, payload)| (action, serde_json::from_str(&payload).unwrap()))
        .collect()
}

#[actix_web::test]
async fn account_api_deletion_records_private_tombstone_and_cascade_atomically() {
    let pool = common::obtain_pool().await.unwrap();
    let _lock = common::DB_LOCK.lock().await;
    common::reset_tables(&pool, &["users", "recovery_decisions"])
        .await
        .unwrap();
    let (id, public) = seed(&pool).await;
    sqlx::query("INSERT INTO events(name_event,description,date_event,start_time,address_ciphertext,owner_user_id) VALUES('synthetic','synthetic',CURRENT_DATE+30,'12:00',fiestaaa_encrypt_text('synthetic'),$1)")
        .bind(id).execute(&pool).await.unwrap();
    let credential = encode_jwt(
        &Claims {
            sub: public.to_string(),
            handle: "recoveryqa".into(),
            exp: (now_ts() + 3600) as usize,
            session_version: 0,
        },
        "test-secret",
    )
    .unwrap();
    let app = test::init_service(
        App::new()
            .app_data(common::build_state(pool.clone(), "test-secret", &[]))
            .configure(routes::configure),
    )
    .await;
    let response = test::call_service(
        &app,
        test::TestRequest::delete()
            .uri("/me")
            .insert_header(("Authorization", format!("Bearer {credential}")))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let rows = records(&pool).await;
    assert!(rows.iter().any(|(action, _)| action == "event_deleted"));
    let payload = &rows
        .iter()
        .find(|(action, _)| action == "account_deleted")
        .unwrap()
        .1;
    assert_eq!(payload["public_id"], public.to_string());
    assert!(payload.get("email").is_none());
    let ciphertext: Vec<u8> = sqlx::query_scalar(
        "SELECT payload_ciphertext FROM recovery_decisions WHERE action='account_deleted'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        !ciphertext
            .windows(public.to_string().len())
            .any(|window| window == public.to_string().as_bytes())
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM users")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
}

#[actix_web::test]
async fn rolled_back_action_leaves_no_record_and_failed_record_prevents_deletion() {
    let pool = common::obtain_pool().await.unwrap();
    let _lock = common::DB_LOCK.lock().await;
    common::reset_tables(&pool, &["users", "recovery_decisions"])
        .await
        .unwrap();
    let (id, _) = seed(&pool).await;
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("DELETE FROM users WHERE id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    assert!(records(&pool).await.is_empty());
    sqlx::query(
        "ALTER TABLE recovery_decisions ADD CONSTRAINT test_refuse_journal CHECK(false) NOT VALID",
    )
    .execute(&pool)
    .await
    .unwrap();
    let result = sqlx::query("DELETE FROM users WHERE id=$1")
        .bind(id)
        .execute(&pool)
        .await;
    sqlx::query("ALTER TABLE recovery_decisions DROP CONSTRAINT test_refuse_journal")
        .execute(&pool)
        .await
        .unwrap();
    assert!(result.is_err());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM users WHERE id=$1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    assert!(records(&pool).await.is_empty());
}

#[actix_web::test]
async fn latest_user_state_preserves_session_password_and_avatar_removal_without_profile_noise() {
    let pool = common::obtain_pool().await.unwrap();
    let _lock = common::DB_LOCK.lock().await;
    common::reset_tables(&pool, &["users", "recovery_decisions"])
        .await
        .unwrap();
    let (id, _) = seed(&pool).await;
    sqlx::query(
        "UPDATE users SET handle='renamed',avatar_url='/media/avatars/old.png' WHERE id=$1",
    )
    .bind(id)
    .execute(&pool)
    .await
    .unwrap();
    assert!(records(&pool).await.is_empty());
    sqlx::query("UPDATE users SET suspended=TRUE,session_version=session_version+1 WHERE id=$1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE users SET suspended=FALSE,password_hash='new-synthetic-hash',session_version=session_version+1,avatar_url=NULL WHERE id=$1").bind(id).execute(&pool).await.unwrap();
    let rows = records(&pool).await;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].0, "user_state");
    assert_eq!(rows[1].1["session_version"], 2);
    assert_eq!(rows[1].1["suspended"], false);
    assert_eq!(rows[1].1["password_hash"], "new-synthetic-hash");
    assert_eq!(rows[1].1["removed_avatar"], "/media/avatars/old.png");
}

#[actix_web::test]
async fn apple_queue_has_stable_recovery_identity_and_retry_does_not_claim_completion() {
    let pool = common::obtain_pool().await.unwrap();
    let _lock = common::DB_LOCK.lock().await;
    common::reset_tables(&pool, &["apple_revocations", "recovery_decisions"])
        .await
        .unwrap();
    let (id,key):(i64,String)=sqlx::query_as("INSERT INTO apple_revocations(client_id,refresh_token_ciphertext) VALUES('synthetic.client',fiestaaa_encrypt_text('synthetic-token')) RETURNING id,recovery_key").fetch_one(&pool).await.unwrap();
    assert_eq!(key.len(), 64);
    sqlx::query("UPDATE apple_revocations SET attempts=1,next_attempt_at=NOW()+INTERVAL '15 minutes' WHERE id=$1").bind(id).execute(&pool).await.unwrap();
    let rows = records(&pool).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, "apple_pending");
    assert_eq!(rows[0].1["recovery_key"], key);
    assert!(rows[0].1.get("refresh_token").is_none());
    sqlx::query("DELETE FROM apple_revocations WHERE id=$1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    let rows = records(&pool).await;
    assert_eq!(rows[1].0, "apple_completed");
    assert_eq!(rows[1].1, serde_json::json!({"recovery_key":key}));
}

#[actix_web::test]
async fn moderation_closure_purge_event_hiding_and_term_changes_are_captured() {
    let pool = common::obtain_pool().await.unwrap();
    let _lock = common::DB_LOCK.lock().await;
    common::reset_tables(
        &pool,
        &[
            "users",
            "recovery_decisions",
            "moderation_terms",
            "abuse_reports",
        ],
    )
    .await
    .unwrap();
    let (id, _) = seed(&pool).await;
    let event:i64=sqlx::query_scalar("INSERT INTO events(name_event,description,date_event,start_time,address_ciphertext,owner_user_id) VALUES('synthetic','synthetic',CURRENT_DATE+30,'12:00',fiestaaa_encrypt_text('synthetic'),$1) RETURNING event_id").bind(id).fetch_one(&pool).await.unwrap();
    sqlx::query("UPDATE events SET moderation_hidden=TRUE,deleted_at=NOW(),deletion_reason='moderation',purge_at=NULL WHERE event_id=$1").bind(event).execute(&pool).await.unwrap();
    let report:Uuid=sqlx::query_scalar("INSERT INTO abuse_reports(reason,comment_ciphertext) VALUES('other',fiestaaa_encrypt_text('synthetic comment')) RETURNING public_id").fetch_one(&pool).await.unwrap();
    sqlx::query("UPDATE abuse_reports SET status='resolved',resolved_at=NOW() WHERE public_id=$1")
        .bind(report)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM abuse_reports WHERE public_id=$1")
        .bind(report)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO moderation_terms VALUES('synthetic filtered term')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM moderation_terms WHERE term='synthetic filtered term'")
        .execute(&pool)
        .await
        .unwrap();
    let rows = records(&pool).await;
    let actions: Vec<_> = rows.iter().map(|row| row.0.as_str()).collect();
    assert_eq!(
        actions,
        vec![
            "event_state",
            "report_state",
            "report_deleted",
            "term_state",
            "term_state"
        ]
    );
    assert_eq!(rows[0].1["moderation_hidden"], true);
    assert_eq!(rows[1].1["status"], "resolved");
    assert_eq!(rows[4].1["present"], false);
    assert!(rows.iter().all(|row| row.1.get("comment").is_none()));
}
