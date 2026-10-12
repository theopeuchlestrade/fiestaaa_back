#[allow(dead_code)]
mod common;
use fiestaaa_back::{auth, cleanup::cleanup_now};
use sqlx::PgPool;
use std::{error::Error, process::Command};

fn run_cli(args: &[&str]) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_create_local_user"));
    command
        .env_clear()
        .args(args)
        .env(
            "DATABASE_URL",
            std::env::var("TEST_DATABASE_URL").expect("isolated database required"),
        )
        .env("DATA_ENCRYPTION_KEY", "test-data-encryption-key-32-chars!!")
        .env("DATA_LOOKUP_KEY", "test-data-lookup-key-32-chars!!!!!!");
    if let Some(value) = std::env::var_os("LLVM_PROFILE_FILE") {
        command.env("LLVM_PROFILE_FILE", value);
    }
    command.output().unwrap()
}
async fn row(pool: &PgPool, email: &str) -> (i64, String, String) {
    sqlx::query_as("SELECT id,handle,password_hash FROM users WHERE fiestaaa_email_matches(email_lookup_hash,$1)").bind(email).fetch_one(pool).await.unwrap()
}
#[actix_web::test]
async fn local_user_command_normalizes_hashes_updates_and_rejects_invalid_handles()
-> Result<(), Box<dyn Error>> {
    let pool = common::obtain_pool()
        .await
        .expect("isolated database required");
    let _lock = common::DB_LOCK.lock().await;
    common::reset_tables(&pool, &["users"]).await?;
    for args in [
        vec![],
        vec!["--help"],
        vec!["--unknown"],
        vec!["--email", "qa@example.invalid"],
        vec!["--email", "", "--password", "fixture"],
        vec!["--email", "qa@example.invalid", "--password", ""],
    ] {
        assert_eq!(run_cli(&args).status.code(), Some(2));
    }
    let output = run_cli(&[
        "--email",
        " QA@example.invalid ",
        "--password",
        "LocalFixture1!",
        "--handle",
        "Local.QA",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let (id, handle, hash) = row(&pool, "qa@example.invalid").await;
    assert_eq!(handle, "local.qa");
    assert!(auth::verify_password(&hash, "LocalFixture1!"));
    assert_ne!(hash, "LocalFixture1!");
    let output = run_cli(&[
        "--email",
        "qa@example.invalid",
        "--password",
        "ChangedFixture2!",
    ]);
    assert!(output.status.success());
    let (same_id, same_handle, changed) = row(&pool, "qa@example.invalid").await;
    assert_eq!(same_id, id);
    assert_eq!(same_handle, handle);
    assert!(auth::verify_password(&changed, "ChangedFixture2!"));
    assert!(!auth::verify_password(&changed, "LocalFixture1!"));
    assert_eq!(
        run_cli(&[
            "--email",
            "other@example.invalid",
            "--password",
            "LocalFixture1!",
            "--handle",
            "bad handle!"
        ])
        .status
        .code(),
        Some(2)
    );
    assert_eq!(
        run_cli(&[
            "--email",
            "other@example.invalid",
            "--password",
            "LocalFixture1!",
            "--handle",
            "local.qa"
        ])
        .status
        .code(),
        Some(1)
    );
    assert!(
        run_cli(&[
            "--email",
            "generated@example.invalid",
            "--password",
            "LocalFixture1!"
        ])
        .status
        .success()
    );
    let (_, generated, _) = row(&pool, "generated@example.invalid").await;
    assert!(fiestaaa_back::handles::is_valid_handle(&generated));
    Ok(())
}
#[actix_web::test]
async fn manual_retention_moves_only_expired_events_to_trash_idempotently()
-> Result<(), Box<dyn Error>> {
    let pool = common::obtain_pool()
        .await
        .expect("isolated database required");
    let _lock = common::DB_LOCK.lock().await;
    common::reset_tables(&pool, &["users"]).await?;
    assert!(
        run_cli(&[
            "--email",
            "retention@example.invalid",
            "--password",
            "LocalFixture1!",
            "--handle",
            "retentionqa"
        ])
        .status
        .success()
    );
    let (owner, _, _) = row(&pool, "retention@example.invalid").await;
    for date in ["2000-01-01", "2099-01-01"] {
        sqlx::query("INSERT INTO events(name_event,description,date_event,start_time,address_ciphertext,owner_user_id) VALUES('Retention fixture','Fictional',$1::text::date,'20:00',fiestaaa_encrypt_text('Fictional'),$2)").bind(date).bind(owner).execute(&pool).await?;
    }
    assert_eq!(cleanup_now(&pool, 7).await?, 1);
    assert_eq!(cleanup_now(&pool, 7).await?, 0);
    let (deleted,reason,purge):(bool,Option<String>,bool)=sqlx::query_as("SELECT deleted_at IS NOT NULL,deletion_reason,purge_at IS NOT NULL FROM events WHERE date_event='2000-01-01'").fetch_one(&pool).await?;
    assert!(deleted && purge);
    assert_eq!(reason.as_deref(), Some("retention"));
    let active: bool =
        sqlx::query_scalar("SELECT deleted_at IS NULL FROM events WHERE date_event='2099-01-01'")
            .fetch_one(&pool)
            .await?;
    assert!(active);
    Ok(())
}
