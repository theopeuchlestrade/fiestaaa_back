use sqlx::{Pool, Postgres};

/// Check the complete embedded migration ledger without attempting any DDL.
pub async fn verify_schema(pool: &Pool<Postgres>) -> Result<(), sqlx::Error> {
    let rows: Vec<(i64, Vec<u8>, bool)> = sqlx::query_as(
        "SELECT version, checksum, success FROM public._sqlx_migrations ORDER BY version",
    )
    .fetch_all(pool)
    .await?;
    let migrator = sqlx::migrate!("./migrations");
    let expected: Vec<_> = migrator
        .iter()
        .filter(|migration| migration.migration_type.is_up_migration())
        .collect();
    if rows.len() != expected.len()
        || rows.iter().zip(expected).any(|(row, migration)| {
            !row.2 || row.0 != migration.version || row.1.as_slice() != migration.checksum.as_ref()
        })
    {
        return Err(sqlx::Error::Protocol(
            "Schema does not match this image; run the approved maintenance migration first".into(),
        ));
    }
    Ok(())
}

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

    let mode = std::env::var("DATABASE_MIGRATIONS").unwrap_or_else(|_| {
        if std::env::var("APP_ENV").as_deref() == Ok("production") {
            "verify".into()
        } else {
            "run".into()
        }
    });
    let result = match mode.as_str() {
        "verify" => verify_schema(&pool)
            .await
            .map_err(|error| error.to_string()),
        "run" => sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .map_err(|error| error.to_string()),
        _ => Err("DATABASE_MIGRATIONS must be run or verify".into()),
    };
    if let Err(e) = result {
        eprintln!("Migration error: {e}");
        std::process::exit(1);
    }

    pool
}
