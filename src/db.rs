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

    pool
}

