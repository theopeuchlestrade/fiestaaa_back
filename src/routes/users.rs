use actix_multipart::Multipart;
use actix_web::{HttpRequest, HttpResponse, Responder, get, patch, post, web};
use futures_util::StreamExt;
use sqlx::Row;
use uuid::Uuid;

use crate::{
    auth::extract_claims_from_auth,
    handles::{handle_available, is_valid_handle, normalize_handle},
    models::{ErrorResponse, HandleAvailabilityResponse, HandleUpdatePayload, MeResponse},
    state::AppState,
};

const MAX_AVATAR_BYTES: usize = 1_000_000; // ~1MB
const MAX_AVATAR_DIM: u32 = 512;

async fn ensure_avatar_column(db: &sqlx::PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("ALTER TABLE users ADD COLUMN IF NOT EXISTS avatar_url TEXT;")
        .execute(db)
        .await?;
    Ok(())
}

#[derive(serde::Deserialize)]
pub struct HandleAvailabilityQuery {
    pub handle: String,
}

#[utoipa::path(
    get,
    path = "/handles/availability",
    tag = "users",
    responses(
        (status = 200, description = "Disponibilité d'un handle", body = HandleAvailabilityResponse),
        (status = 400, description = "Handle invalide", body = ErrorResponse)
    ),
    params(
        ("handle" = String, Query, description = "Handle à tester")
    )
)]
#[get("/handles/availability")]
pub async fn check_handle_availability(
    state: web::Data<AppState>,
    query: web::Query<HandleAvailabilityQuery>,
) -> impl Responder {
    let candidate = normalize_handle(&query.handle).normalized;
    if !is_valid_handle(&candidate) {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_handle".into(),
            details: Some("format attendu: 4-32 chars [a-z0-9._-]".into()),
        });
    }

    match handle_available(&state.db, &candidate).await {
        Ok(available) => HttpResponse::Ok().json(HandleAvailabilityResponse { available }),
        Err(_) => HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        }),
    }
}

#[utoipa::path(
    patch,
    path = "/me/handle",
    tag = "users",
    request_body = HandleUpdatePayload,
    responses(
        (status = 200, description = "Handle mis à jour", body = MeResponse),
        (status = 400, description = "Handle invalide", body = ErrorResponse),
        (status = 401, description = "Authentification requise", body = ErrorResponse),
        (status = 409, description = "Handle déjà pris", body = ErrorResponse)
    )
)]
#[patch("/me/handle")]
pub async fn update_handle(
    state: web::Data<AppState>,
    req: HttpRequest,
    payload: web::Json<HandleUpdatePayload>,
) -> impl Responder {
    let claims = match extract_claims_from_auth(&req, &state.jwt_secret) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let candidate = normalize_handle(&payload.handle).normalized;
    if !is_valid_handle(&candidate) {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_handle".into(),
            details: Some("format attendu: 4-32 chars [a-z0-9._-]".into()),
        });
    }

    let _ = ensure_avatar_column(&state.db).await;

    let res = sqlx::query(
        "UPDATE users
         SET handle = $1
         WHERE lower(email) = lower($2)
         RETURNING email, handle, avatar_url",
    )
    .bind(&candidate)
    .bind(&claims.sub)
    .fetch_one(&state.db)
    .await;

    match res {
        Ok(row) => HttpResponse::Ok().json(MeResponse {
            email: row.get("email"),
            handle: row.get("handle"),
            avatar_url: row.get::<Option<String>, _>("avatar_url"),
            exp: claims.exp,
        }),
        Err(sqlx::Error::Database(db_err)) if db_err.code().as_deref() == Some("23505") => {
            HttpResponse::Conflict().json(ErrorResponse {
                error: "handle_taken".into(),
                details: None,
            })
        }
        Err(sqlx::Error::RowNotFound) => HttpResponse::Unauthorized().json(ErrorResponse {
            error: "user_not_found".into(),
            details: None,
        }),
        Err(_) => HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        }),
    }
}

#[utoipa::path(
    post,
    path = "/me/avatar",
    tag = "users",
    responses(
        (status = 200, description = "Avatar mis à jour", body = MeResponse),
        (status = 400, description = "Fichier invalide", body = ErrorResponse),
        (status = 401, description = "Authentification requise", body = ErrorResponse),
        (status = 413, description = "Fichier trop volumineux", body = ErrorResponse),
    )
)]
#[post("/me/avatar")]
pub async fn upload_avatar(
    state: web::Data<AppState>,
    req: HttpRequest,
    mut payload: Multipart,
) -> impl Responder {
    let claims = match extract_claims_from_auth(&req, &state.jwt_secret) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let mut bytes: Vec<u8> = Vec::new();
    while let Some(field) = payload.next().await {
        let mut field = match field {
            Ok(f) => f,
            Err(_) => {
                return HttpResponse::BadRequest().json(ErrorResponse {
                    error: "invalid_upload".into(),
                    details: Some("champ multipart invalide".into()),
                });
            }
        };
        while let Some(chunk) = field.next().await {
            let chunk = match chunk {
                Ok(c) => c,
                Err(_) => {
                    return HttpResponse::BadRequest().json(ErrorResponse {
                        error: "invalid_upload".into(),
                        details: Some("lecture impossible".into()),
                    });
                }
            };
            if bytes.len() + chunk.len() > MAX_AVATAR_BYTES {
                return HttpResponse::PayloadTooLarge().json(ErrorResponse {
                    error: "file_too_large".into(),
                    details: Some("limite 1 Mo".into()),
                });
            }
            bytes.extend_from_slice(&chunk);
        }
        break;
    }

    if bytes.is_empty() {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "file_missing".into(),
            details: Some("aucun fichier reçu".into()),
        });
    }

    let img = match image::load_from_memory(&bytes) {
        Ok(img) => img,
        Err(_) => {
            return HttpResponse::BadRequest().json(ErrorResponse {
                error: "invalid_image".into(),
                details: Some("format image non supporté".into()),
            });
        }
    };

    let resized = img.resize(
        MAX_AVATAR_DIM,
        MAX_AVATAR_DIM,
        image::imageops::FilterType::Triangle,
    );
    let mut encoded: Vec<u8> = Vec::new();
    if resized
        .write_to(
            &mut std::io::Cursor::new(&mut encoded),
            image::ImageFormat::Jpeg,
        )
        .is_err()
    {
        return HttpResponse::InternalServerError().json(ErrorResponse {
            error: "encode_error".into(),
            details: None,
        });
    }

    let filename = format!("{}.jpg", Uuid::new_v4());
    let path = std::path::Path::new(&state.avatar_upload_dir).join(&filename);
    if let Err(e) = tokio::fs::create_dir_all(path.parent().unwrap()).await {
        return HttpResponse::InternalServerError().json(ErrorResponse {
            error: "storage_error".into(),
            details: Some(format!("impossible de préparer le dossier: {e}")),
        });
    }
    if let Err(e) = tokio::fs::write(&path, encoded).await {
        return HttpResponse::InternalServerError().json(ErrorResponse {
            error: "storage_error".into(),
            details: Some(format!("écriture impossible: {e}")),
        });
    }

    let public_url = format!(
        "{}/{}",
        state.avatar_base_url.trim_end_matches('/'),
        filename
    );

    let _ = ensure_avatar_column(&state.db).await;

    let res = sqlx::query(
        "UPDATE users
         SET avatar_url = $1
         WHERE lower(email) = lower($2)
         RETURNING email, handle, avatar_url",
    )
    .bind(&public_url)
    .bind(&claims.sub)
    .fetch_one(&state.db)
    .await;

    match res {
        Ok(row) => HttpResponse::Ok().json(MeResponse {
            email: row.get("email"),
            handle: row.get("handle"),
            avatar_url: row.get::<Option<String>, _>("avatar_url"),
            exp: claims.exp,
        }),
        Err(sqlx::Error::RowNotFound) => HttpResponse::Unauthorized().json(ErrorResponse {
            error: "user_not_found".into(),
            details: None,
        }),
        Err(_) => HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        }),
    }
}
