use std::error::Error;

use dotenvy::dotenv;
use fiestaaa_back::{
    auth::{hash_password, validate_password_strength},
    db,
    handles::{generate_unique_handle, is_valid_handle, normalize_handle},
    security::normalize_email,
};
use sqlx::PgPool;

#[derive(sqlx::FromRow)]
struct ExistingUser {
    handle: String,
}

struct CliArgs {
    email: String,
    password: String,
    handle: Option<String>,
}

fn required_secret_env(name: &str) -> String {
    let value = std::env::var(name).unwrap_or_default();
    let trimmed = value.trim();
    if trimmed.len() < 32 {
        eprintln!("{name} must be defined and contain at least 32 characters");
        std::process::exit(2);
    }
    trimmed.to_string()
}

fn usage(binary: &str) -> String {
    format!(
        "Usage: {binary} --email <email> --password <password> [--handle <handle>]\n\
\n\
Creates or updates a local user directly in the database.\n\
Passwords are hashed with Argon2 before storage.\n\
\n\
Examples:\n\
  {binary} --email test@local.dev --password changeme --handle test_local\n\
  {binary} --email test@local.dev --password changeme"
    )
}

fn parse_args() -> Result<CliArgs, String> {
    let mut args = std::env::args().skip(1);
    let binary = std::env::args()
        .next()
        .unwrap_or_else(|| "create_local_user".to_string());
    let mut email = None;
    let mut password = None;
    let mut handle = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--email" => email = args.next(),
            "--password" => password = args.next(),
            "--handle" => handle = args.next(),
            "--help" | "-h" => return Err(usage(&binary)),
            other => {
                return Err(format!("Unknown argument: {other}\n\n{}", usage(&binary)));
            }
        }
    }

    let email = email
        .map(|value| normalize_email(&value))
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("--email is required\n\n{}", usage(&binary)))?;
    let password = password
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("--password is required\n\n{}", usage(&binary)))?;

    Ok(CliArgs {
        email,
        password,
        handle,
    })
}

async fn fetch_existing_user(
    pool: &PgPool,
    email: &str,
) -> Result<Option<ExistingUser>, sqlx::Error> {
    sqlx::query_as::<_, ExistingUser>(
        "SELECT handle FROM users WHERE fiestaaa_email_matches(email_lookup_hash, $1)",
    )
        .bind(email)
        .fetch_optional(pool)
        .await
}

async fn resolve_handle(
    pool: &PgPool,
    raw_handle: Option<String>,
    existing_handle: Option<&str>,
) -> Result<String, String> {
    match raw_handle {
        Some(value) => {
            let normalized = normalize_handle(&value).normalized;
            if !is_valid_handle(&normalized) {
                return Err(
                    "Invalid handle. Expected 4-32 chars using only a-z, 0-9, '.', '_' or '-'"
                        .to_string(),
                );
            }
            Ok(normalized)
        }
        None => match existing_handle {
            Some(value) => Ok(value.to_string()),
            None => generate_unique_handle(pool)
                .await
                .map_err(|err| format!("failed to generate handle: {err}")),
        },
    }
}

async fn upsert_user(
    pool: &PgPool,
    email: &str,
    password_hash: &str,
    handle: &str,
) -> Result<(i64, bool), sqlx::Error> {
    let mut tx = pool.begin().await?;

    sqlx::query("DELETE FROM pending_registrations WHERE fiestaaa_email_matches(email_lookup_hash, $1)")
        .bind(email)
        .execute(&mut *tx)
        .await?;

    let existing_id =
        sqlx::query_scalar::<_, i64>(
            "SELECT id FROM users WHERE fiestaaa_email_matches(email_lookup_hash, $1)",
        )
            .bind(email)
            .fetch_optional(&mut *tx)
            .await?;

    let result = if let Some(id) = existing_id {
        sqlx::query("UPDATE users SET password_hash = $1, handle = $2 WHERE id = $3")
            .bind(password_hash)
            .bind(handle)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        (id, false)
    } else {
        let id = sqlx::query_scalar::<_, i64>(
            "INSERT INTO users (email_ciphertext, email_lookup_hash, password_hash, handle)
             VALUES (fiestaaa_encrypt_text($1), fiestaaa_email_lookup($1), $2, $3)
             RETURNING id",
        )
        .bind(email)
        .bind(password_hash)
        .bind(handle)
        .fetch_one(&mut *tx)
        .await?;
        (id, true)
    };

    tx.commit().await?;
    Ok(result)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    fiestaaa_back::install_rustls_crypto_provider();
    let _ = dotenv();

    let args = match parse_args() {
        Ok(value) => value,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };

    if let Err(reason) = validate_password_strength(&args.password) {
        eprintln!("Warning: weak password accepted for local dev only ({reason}).");
    }

    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/fiestaaa".to_string());
    let data_encryption_key = required_secret_env("DATA_ENCRYPTION_KEY");
    let data_lookup_key = required_secret_env("DATA_LOOKUP_KEY");
    let pool = db::connect_and_migrate(&database_url, &data_encryption_key, &data_lookup_key).await;
    let existing_user = match fetch_existing_user(&pool, &args.email).await {
        Ok(value) => value,
        Err(err) => {
            eprintln!("Failed to look up local user: {err}");
            std::process::exit(1);
        }
    };
    let handle = match resolve_handle(
        &pool,
        args.handle,
        existing_user.as_ref().map(|user| user.handle.as_str()),
    )
    .await
    {
        Ok(value) => value,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };
    let password_hash = hash_password(&args.password)?;

    match upsert_user(&pool, &args.email, &password_hash, &handle).await {
        Ok((id, created)) => {
            let status = if created { "created" } else { "updated" };
            println!(
                "Local user {status}: id={id} email={} handle={handle}",
                args.email
            );
            Ok(())
        }
        Err(err) => {
            eprintln!("Failed to create local user: {err}");
            std::process::exit(1);
        }
    }
}
