use sqlx::{Pool, Postgres};

pub async fn connect_and_migrate(
    database_url: &str,
    max_connections: u32,
    data_encryption_key: &str,
    data_lookup_key: &str,
) -> Pool<Postgres> {
    let data_encryption_key = data_encryption_key.to_string();
    let data_lookup_key = data_lookup_key.to_string();

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(max_connections)
        .after_connect(move |conn, _meta| {
            let data_encryption_key = data_encryption_key.clone();
            let data_lookup_key = data_lookup_key.clone();
            Box::pin(async move {
                sqlx::query(
                    r#"
                    SELECT
                        set_config('fiestaaa.data_encryption_key', $1, false),
                        set_config('fiestaaa.data_lookup_key', $2, false)
                    "#,
                )
                .bind(&data_encryption_key)
                .bind(&data_lookup_key)
                .execute(conn)
                .await?;
                Ok(())
            })
        })
        .connect(database_url)
        .await
        .expect("failed to connect to database");

    if let Err(e) = sqlx::migrate!("./migrations").run(&pool).await {
        eprintln!("Migration error: {e}");
        std::process::exit(1);
    }

    pool
}
