mod common;

use fiestaaa_back::db::verify_schema;
use sqlx::AssertSqlSafe;

#[actix_web::test]
async fn runtime_verification_needs_only_ledger_select_and_rejects_drift() {
    let _guard = common::DB_LOCK.lock().unwrap();
    let Some(pool) = common::obtain_pool().await else {
        return;
    };
    let role = format!("schema_test_{}", uuid::Uuid::new_v4().simple());
    sqlx::raw_sql(AssertSqlSafe(format!(
        "CREATE ROLE {role}; GRANT USAGE ON SCHEMA public TO {role}; \
         GRANT SELECT ON public._sqlx_migrations TO {role};"
    )))
    .execute(&pool)
    .await
    .unwrap();
    let url = std::env::var("TEST_DATABASE_URL").unwrap();
    let role_clone = role.clone();
    let limited = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .after_connect(move |connection, _| {
            let statement = format!("SET ROLE {role_clone}");
            Box::pin(async move {
                sqlx::query(AssertSqlSafe(statement))
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect(&url)
        .await
        .unwrap();
    let can_create: bool =
        sqlx::query_scalar("SELECT has_schema_privilege(current_user,'public','CREATE')")
            .fetch_one(&limited)
            .await
            .unwrap();
    assert!(!can_create);
    verify_schema(&limited).await.unwrap();
    let mut transaction = pool.begin().await.unwrap();
    sqlx::query("UPDATE public._sqlx_migrations SET success=false WHERE version=(SELECT max(version) FROM public._sqlx_migrations)")
        .execute(&mut *transaction).await.unwrap();
    transaction.commit().await.unwrap();
    let rejected = verify_schema(&limited).await.is_err();
    sqlx::query("UPDATE public._sqlx_migrations SET success=true WHERE NOT success")
        .execute(&pool)
        .await
        .unwrap();
    limited.close().await;
    sqlx::raw_sql(AssertSqlSafe(format!(
        "DROP OWNED BY {role}; DROP ROLE {role};"
    )))
    .execute(&pool)
    .await
    .unwrap();
    assert!(
        rejected,
        "An incomplete migration must prevent runtime startup"
    );
}
