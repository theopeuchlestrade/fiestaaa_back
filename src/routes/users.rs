use actix_multipart::Multipart;
use actix_web::{HttpRequest, HttpResponse, Responder, delete, get, patch, post, web};
use futures_util::StreamExt;
use image::{ImageReader, Limits};
use log::warn;
use sqlx::Row;
use uuid::Uuid;

use crate::{
    auth::{
        build_cleared_session_cookie, extract_active_claims_from_auth,
        extract_verified_claims_from_auth, revoke_auth_token_from_request, should_secure_cookie,
    },
    handles::{handle_available, is_valid_handle, normalize_handle},
    models::{
        ErrorResponse, HandleAvailabilityResponse, HandleUpdatePayload, MeResponse, StatusResponse,
    },
    state::AppState,
};

const MAX_AVATAR_BYTES: usize = 8 * 1024 * 1024;
const MAX_AVATAR_DIM: u32 = 512;
const MAX_SOURCE_AVATAR_DIM: u32 = 4096;
const MAX_SOURCE_AVATAR_ALLOC_BYTES: u64 = 64 * 1024 * 1024;

fn avatar_storage_path(
    avatar_upload_dir: &str,
    avatar_base_url: &str,
    avatar_url: &str,
) -> Option<std::path::PathBuf> {
    let base = avatar_base_url.trim_end_matches('/');
    let filename = avatar_url.strip_prefix(base)?.strip_prefix('/')?;
    if filename.is_empty()
        || filename.contains('/')
        || filename.contains('\\')
        || filename.contains("..")
    {
        return None;
    }

    Some(std::path::Path::new(avatar_upload_dir).join(filename))
}

async fn cleanup_avatar_file(state: &AppState, avatar_url: Option<&str>) {
    let Some(avatar_url) = avatar_url else {
        return;
    };
    let Some(path) =
        avatar_storage_path(&state.avatar_upload_dir, &state.avatar_base_url, avatar_url)
    else {
        return;
    };

    match tokio::fs::remove_file(&path).await {
        Ok(_) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => warn!("failed to remove avatar file {}: {err}", path.display()),
    }
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
        (status = 200, description = "Handle availability", body = HandleAvailabilityResponse),
        (status = 400, description = "Invalid handle", body = ErrorResponse)
    ),
    params(
        ("handle" = String, Query, description = "Handle to check")
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
        (status = 200, description = "Handle updated", body = MeResponse),
        (status = 400, description = "Invalid handle", body = ErrorResponse),
        (status = 401, description = "Authentication required", body = ErrorResponse),
        (status = 409, description = "Handle already taken", body = ErrorResponse)
    )
)]
#[patch("/me/handle")]
pub async fn update_handle(
    state: web::Data<AppState>,
    req: HttpRequest,
    payload: web::Json<HandleUpdatePayload>,
) -> impl Responder {
    let claims = match extract_active_claims_from_auth(&req, &state.db, &state.jwt_secret).await {
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

    let res = sqlx::query(
        "UPDATE users
         SET handle = $1
         WHERE fiestaaa_email_matches(email_lookup_hash, $2)
         RETURNING public_id::text AS public_id,
                   fiestaaa_decrypt_text(email_ciphertext) AS email,
                   handle,
                   avatar_url",
    )
    .bind(&candidate)
    .bind(&claims.sub)
    .fetch_one(&state.db)
    .await;

    match res {
        Ok(row) => HttpResponse::Ok().json(MeResponse {
            public_id: row.get("public_id"),
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
        (status = 200, description = "Avatar updated", body = MeResponse),
        (status = 400, description = "Invalid file", body = ErrorResponse),
        (status = 401, description = "Authentication required", body = ErrorResponse),
        (status = 413, description = "File too large", body = ErrorResponse),
    )
)]
#[post("/me/avatar")]
pub async fn upload_avatar(
    state: web::Data<AppState>,
    req: HttpRequest,
    mut payload: Multipart,
) -> impl Responder {
    let claims = match extract_active_claims_from_auth(&req, &state.db, &state.jwt_secret).await {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    let previous_avatar_url = match sqlx::query_scalar::<_, Option<String>>(
        "SELECT avatar_url
         FROM users
         WHERE fiestaaa_email_matches(email_lookup_hash, $1)",
    )
    .bind(&claims.sub)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(value)) => value,
        Ok(None) => {
            return HttpResponse::Unauthorized().json(ErrorResponse {
                error: "user_not_found".into(),
                details: None,
            });
        }
        Err(_) => {
            return HttpResponse::InternalServerError().json(ErrorResponse {
                error: "db_error".into(),
                details: None,
            });
        }
    };

    let mut bytes: Vec<u8> = Vec::new();
    if let Some(field) = payload.next().await {
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
                    details: Some("limite 8 Mo".into()),
                });
            }
            bytes.extend_from_slice(&chunk);
        }
    }

    if bytes.is_empty() {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "file_missing".into(),
            details: Some("aucun fichier reçu".into()),
        });
    }

    let mut reader = match ImageReader::new(std::io::Cursor::new(&bytes)).with_guessed_format() {
        Ok(value) => value,
        Err(_) => {
            return HttpResponse::BadRequest().json(ErrorResponse {
                error: "invalid_image".into(),
                details: Some("format image non supporté".into()),
            });
        }
    };
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SOURCE_AVATAR_DIM);
    limits.max_image_height = Some(MAX_SOURCE_AVATAR_DIM);
    limits.max_alloc = Some(MAX_SOURCE_AVATAR_ALLOC_BYTES);
    reader.limits(limits);

    let img = match reader.decode() {
        Ok(img) => img,
        Err(_) => {
            return HttpResponse::BadRequest().json(ErrorResponse {
                error: "invalid_image".into(),
                details: Some("image trop grande ou format non supporté".into()),
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

    let res = sqlx::query(
        "UPDATE users
         SET avatar_url = $1
         WHERE fiestaaa_email_matches(email_lookup_hash, $2)
         RETURNING public_id::text AS public_id,
                   fiestaaa_decrypt_text(email_ciphertext) AS email,
                   handle,
                   avatar_url",
    )
    .bind(&public_url)
    .bind(&claims.sub)
    .fetch_one(&state.db)
    .await;

    match res {
        Ok(row) => {
            cleanup_avatar_file(
                state.get_ref(),
                previous_avatar_url
                    .as_deref()
                    .filter(|url| *url != public_url),
            )
            .await;
            HttpResponse::Ok().json(MeResponse {
                public_id: row.get("public_id"),
                email: row.get("email"),
                handle: row.get("handle"),
                avatar_url: row.get::<Option<String>, _>("avatar_url"),
                exp: claims.exp,
            })
        }
        Err(sqlx::Error::RowNotFound) => {
            cleanup_avatar_file(state.get_ref(), Some(&public_url)).await;
            HttpResponse::Unauthorized().json(ErrorResponse {
                error: "user_not_found".into(),
                details: None,
            })
        }
        Err(_) => {
            cleanup_avatar_file(state.get_ref(), Some(&public_url)).await;
            HttpResponse::InternalServerError().json(ErrorResponse {
                error: "db_error".into(),
                details: None,
            })
        }
    }
}

#[utoipa::path(
    delete,
    path = "/me",
    tag = "users",
    responses(
        (status = 200, description = "Account deleted successfully", body = StatusResponse),
        (status = 401, description = "Authentication required", body = ErrorResponse),
        (status = 404, description = "User not found", body = ErrorResponse),
        (status = 500, description = "Server error", body = ErrorResponse)
    )
)]
#[delete("/me")]
pub async fn delete_account(state: web::Data<AppState>, req: HttpRequest) -> impl Responder {
    let claims = match extract_verified_claims_from_auth(&req, &state.db, &state.jwt_secret).await {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    if let Err(resp) = revoke_auth_token_from_request(&req, &state.db, &state.jwt_secret).await {
        return resp;
    }

    let res = sqlx::query(
        "DELETE FROM users
         WHERE fiestaaa_email_matches(email_lookup_hash, $1)
         RETURNING avatar_url",
    )
    .bind(&claims.sub)
    .fetch_optional(&state.db)
    .await;

    match res {
        Ok(Some(row)) => {
            let avatar_url: Option<String> = row.get("avatar_url");
            cleanup_avatar_file(state.get_ref(), avatar_url.as_deref()).await;
            HttpResponse::Ok()
                .cookie(build_cleared_session_cookie(should_secure_cookie(
                    &req,
                    &state.app_base_url,
                    state.trust_proxy_headers,
                )))
                .json(StatusResponse {
                    status: "account_deleted".into(),
                })
        }
        Ok(_) => HttpResponse::NotFound().json(ErrorResponse {
            error: "user_not_found".into(),
            details: None,
        }),
        Err(_) => HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: Some("Impossible de supprimer le compte".into()),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::avatar_storage_path;
    use std::path::PathBuf;

    #[test]
    fn avatar_storage_path_accepts_expected_public_url() {
        let path = avatar_storage_path(
            "/tmp/uploads/avatars",
            "https://api.fiestaaa.app/media/avatars",
            "https://api.fiestaaa.app/media/avatars/avatar.jpg",
        );

        assert_eq!(path, Some(PathBuf::from("/tmp/uploads/avatars/avatar.jpg")));
    }

    #[test]
    fn avatar_storage_path_rejects_nested_or_external_paths() {
        assert!(
            avatar_storage_path(
                "/tmp/uploads/avatars",
                "https://api.fiestaaa.app/media/avatars",
                "https://api.fiestaaa.app/media/avatars/../secret.txt",
            )
            .is_none()
        );
        assert!(
            avatar_storage_path(
                "/tmp/uploads/avatars",
                "https://api.fiestaaa.app/media/avatars",
                "https://other.example/media/avatars/avatar.jpg",
            )
            .is_none()
        );
    }
}
