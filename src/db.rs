use sqlx::{Pool, Postgres};

pub async fn connect_and_migrate(database_url: &str) -> Pool<Postgres> {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(database_url)
        .await
        .expect("failed to connect to database");

    if let Err(e) = sqlx::migrate!("./migrations").run(&pool).await {
        eprintln!("Migration error: {e}");
        std::process::exit(1);
    }

    // Extra safety: ensure friend tables exist even if migration cache lags behind.
    if let Err(e) = ensure_friend_tables(&pool).await {
        eprintln!("Failed to ensure friend tables: {e}");
        std::process::exit(1);
    }

    if let Err(e) = ensure_avatar_column(&pool).await {
        eprintln!("Failed to ensure avatar column: {e}");
        std::process::exit(1);
    }

    pool
}

async fn ensure_friend_tables(pool: &Pool<Postgres>) -> Result<(), sqlx::Error> {
    // Split the DDL so it works on Postgres prepared statements.
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS friendships (
            user_a BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            user_b BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            CONSTRAINT friendships_user_order CHECK (user_a < user_b),
            CONSTRAINT friendships_no_self CHECK (user_a <> user_b),
            CONSTRAINT friendships_pk PRIMARY KEY (user_a, user_b)
        );
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS friendships_user_lookup_idx
            ON friendships (user_a, user_b);
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS friend_requests (
            id BIGSERIAL PRIMARY KEY,
            sender_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            receiver_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            status TEXT NOT NULL DEFAULT 'Pending',
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            responded_at TIMESTAMPTZ
        );
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE UNIQUE INDEX IF NOT EXISTS friend_requests_pending_pair_idx
            ON friend_requests (LEAST(sender_id, receiver_id), GREATEST(sender_id, receiver_id))
            WHERE status = 'Pending';
        "#,
    )
    .execute(pool)
    .await?;

    Ok(())
}

async fn ensure_avatar_column(pool: &Pool<Postgres>) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        ALTER TABLE users
            ADD COLUMN IF NOT EXISTS avatar_url TEXT;
        "#,
    )
    .execute(pool)
    .await?;
    Ok(())
}
