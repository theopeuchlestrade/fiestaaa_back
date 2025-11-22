use actix_web::{HttpRequest, HttpResponse, Responder, delete, get, patch, post, put, web};
use regex::Regex;
use sqlx::Error;

use crate::{
    auth::extract_claims_from_auth,
    models::{
        ErrorResponse, PaymentProvider, PaymentProviderPatchPayload, PaymentProviderPayload,
        StatusResponse,
    },
    state::AppState,
};

const DEFAULT_PAYMENT_URL_REGEX: &str = r"^https?://.+$";

fn ensure_admin(req: &HttpRequest, state: &AppState) -> Result<(), HttpResponse> {
    let claims = extract_claims_from_auth(req, &state.jwt_secret)?;
    if state.admin_emails.is_empty() {
        return Ok(());
    }

    if state.admin_emails.contains(&claims.sub.to_lowercase()) {
        Ok(())
    } else {
        Err(HttpResponse::Forbidden().json(ErrorResponse {
            error: "forbidden".into(),
            details: Some("admin privileges required".into()),
        }))
    }
}

#[utoipa::path(
    get,
    path = "/payment-providers",
    tag = "payment-providers",
    responses(
        (status = 200, description = "Liste des fournisseurs de paiement", body = [PaymentProvider]),
        (status = 500, description = "Erreur base de données", body = ErrorResponse)
    )
)]
#[get("/payment-providers")]
pub async fn list_payment_providers(state: web::Data<AppState>) -> impl Responder {
    let res = sqlx::query_as::<_, PaymentProvider>(
        "SELECT provider_id, provider_name, url_template, validation_regex, is_active 
         FROM payment_providers 
         ORDER BY provider_name",
    )
    .fetch_all(&state.db)
    .await;

    match res {
        Ok(providers) => HttpResponse::Ok().json(providers),
        Err(_) => HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        }),
    }
}

#[utoipa::path(
    post,
    path = "/payment-providers",
    tag = "payment-providers",
    request_body = PaymentProviderPayload,
    responses(
        (status = 201, description = "Fournisseur de paiement créé", body = PaymentProvider),
        (status = 400, description = "Payload invalide", body = ErrorResponse),
        (status = 403, description = "Non autorisé", body = ErrorResponse),
        (status = 500, description = "Erreur base de données", body = ErrorResponse)
    )
)]
#[post("/payment-providers")]
pub async fn create_payment_provider(
    state: web::Data<AppState>,
    req: HttpRequest,
    payload: web::Json<PaymentProviderPayload>,
) -> impl Responder {
    if let Err(resp) = ensure_admin(&req, state.get_ref()) {
        return resp;
    }

    create_or_replace_provider(state, payload.into_inner(), None).await
}

#[utoipa::path(
    put,
    path = "/payment-providers/{provider_id}",
    tag = "payment-providers",
    request_body = PaymentProviderPayload,
    responses(
        (status = 200, description = "Fournisseur de paiement mis à jour", body = PaymentProvider),
        (status = 400, description = "Payload invalide", body = ErrorResponse),
        (status = 403, description = "Non autorisé", body = ErrorResponse),
        (status = 404, description = "Fournisseur introuvable", body = ErrorResponse),
        (status = 500, description = "Erreur base de données", body = ErrorResponse)
    ),
    params(
        ("provider_id" = i32, Path, description = "Identifiant du fournisseur")
    )
)]
#[put("/payment-providers/{provider_id}")]
pub async fn replace_payment_provider(
    state: web::Data<AppState>,
    req: HttpRequest,
    provider_id: web::Path<i32>,
    payload: web::Json<PaymentProviderPayload>,
) -> impl Responder {
    if let Err(resp) = ensure_admin(&req, state.get_ref()) {
        return resp;
    }

    create_or_replace_provider(state, payload.into_inner(), Some(*provider_id)).await
}

async fn create_or_replace_provider(
    state: web::Data<AppState>,
    payload: PaymentProviderPayload,
    provider_id: Option<i32>,
) -> HttpResponse {
    if payload.provider_name.trim().is_empty() || payload.url_template.trim().is_empty() {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("provider_name et url_template ne peuvent pas être vides".into()),
        });
    }

    if !payload.url_template.contains("{identifier}") {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("url_template doit contenir le placeholder {identifier}".into()),
        });
    }

    let validation_pattern = payload
        .validation_regex
        .as_deref()
        .unwrap_or(DEFAULT_PAYMENT_URL_REGEX)
        .trim();
    if validation_pattern.is_empty() || Regex::new(validation_pattern).is_err() {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("validation_regex doit être un regex valide".into()),
        });
    }

    let query = match provider_id {
        Some(id) => sqlx::query_as::<_, PaymentProvider>(
            "UPDATE payment_providers
             SET provider_name = $1, url_template = $2, validation_regex = $3, is_active = $4
             WHERE provider_id = $5
             RETURNING provider_id, provider_name, url_template, validation_regex, is_active",
        )
        .bind(payload.provider_name.trim())
        .bind(payload.url_template.trim())
        .bind(validation_pattern)
        .bind(payload.is_active.unwrap_or(true))
        .bind(id),
        None => sqlx::query_as::<_, PaymentProvider>(
            "INSERT INTO payment_providers (provider_name, url_template, validation_regex, is_active)
             VALUES ($1, $2, $3, $4)
             RETURNING provider_id, provider_name, url_template, validation_regex, is_active",
        )
        .bind(payload.provider_name.trim())
        .bind(payload.url_template.trim())
        .bind(validation_pattern)
        .bind(payload.is_active.unwrap_or(true)),
    };

    let res = if provider_id.is_some() {
        query.fetch_optional(&state.db).await
    } else {
        query.fetch_one(&state.db).await.map(Some)
    };

    match res {
        Ok(Some(provider)) => {
            if provider_id.is_some() {
                HttpResponse::Ok().json(provider)
            } else {
                HttpResponse::Created().json(provider)
            }
        }
        Ok(None) => HttpResponse::NotFound().json(ErrorResponse {
            error: "provider_not_found".into(),
            details: None,
        }),
        Err(Error::Database(db_err)) if db_err.code().as_deref() == Some("23505") => {
            HttpResponse::BadRequest().json(ErrorResponse {
                error: "duplicate_provider_name".into(),
                details: Some("Ce nom de fournisseur existe déjà".into()),
            })
        }
        Err(_) => HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        }),
    }
}

#[utoipa::path(
    patch,
    path = "/payment-providers/{provider_id}",
    tag = "payment-providers",
    request_body = PaymentProviderPatchPayload,
    responses(
        (status = 200, description = "Fournisseur de paiement modifié", body = PaymentProvider),
        (status = 400, description = "Payload invalide", body = ErrorResponse),
        (status = 403, description = "Non autorisé", body = ErrorResponse),
        (status = 404, description = "Fournisseur introuvable", body = ErrorResponse),
        (status = 500, description = "Erreur base de données", body = ErrorResponse)
    ),
    params(
        ("provider_id" = i32, Path, description = "Identifiant du fournisseur")
    )
)]
#[patch("/payment-providers/{provider_id}")]
pub async fn update_payment_provider(
    state: web::Data<AppState>,
    req: HttpRequest,
    provider_id: web::Path<i32>,
    payload: web::Json<PaymentProviderPatchPayload>,
) -> impl Responder {
    if let Err(resp) = ensure_admin(&req, state.get_ref()) {
        return resp;
    }

    let payload = payload.into_inner();
    if payload
        .provider_name
        .as_ref()
        .is_some_and(|v| v.trim().is_empty())
        || payload
            .url_template
            .as_ref()
            .is_some_and(|v| v.trim().is_empty())
    {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("provider_name et url_template ne peuvent pas être vides".into()),
        });
    }

    if payload
        .url_template
        .as_ref()
        .is_some_and(|v| !v.contains("{identifier}"))
    {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("url_template doit contenir le placeholder {identifier}".into()),
        });
    }

    let validation_pattern = payload
        .validation_regex
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty());
    if let Some(pattern) = validation_pattern {
        if Regex::new(pattern).is_err() {
            return HttpResponse::BadRequest().json(ErrorResponse {
                error: "invalid_payload".into(),
                details: Some("validation_regex doit être un regex valide".into()),
            });
        }
    }

    let res = sqlx::query_as::<_, PaymentProvider>(
        "UPDATE payment_providers
         SET provider_name = COALESCE($1, provider_name),
             url_template = COALESCE($2, url_template),
             validation_regex = COALESCE($3, validation_regex),
             is_active = COALESCE($4, is_active)
         WHERE provider_id = $5
         RETURNING provider_id, provider_name, url_template, validation_regex, is_active",
    )
    .bind(
        payload
            .provider_name
            .as_ref()
            .map(|v| v.trim())
            .filter(|v| !v.is_empty()),
    )
    .bind(
        payload
            .url_template
            .as_ref()
            .map(|v| v.trim())
            .filter(|v| !v.is_empty()),
    )
    .bind(validation_pattern)
    .bind(payload.is_active)
    .bind(*provider_id)
    .fetch_optional(&state.db)
    .await;

    match res {
        Ok(Some(provider)) => HttpResponse::Ok().json(provider),
        Ok(None) => HttpResponse::NotFound().json(ErrorResponse {
            error: "provider_not_found".into(),
            details: None,
        }),
        Err(Error::Database(db_err)) if db_err.code().as_deref() == Some("23505") => {
            HttpResponse::BadRequest().json(ErrorResponse {
                error: "duplicate_provider_name".into(),
                details: Some("Ce nom de fournisseur existe déjà".into()),
            })
        }
        Err(_) => HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        }),
    }
}

#[utoipa::path(
    delete,
    path = "/payment-providers/{provider_id}",
    tag = "payment-providers",
    responses(
        (status = 200, description = "Fournisseur supprimé", body = StatusResponse),
        (status = 403, description = "Non autorisé", body = ErrorResponse),
        (status = 404, description = "Fournisseur introuvable", body = ErrorResponse),
        (status = 500, description = "Erreur base de données", body = ErrorResponse)
    ),
    params(
        ("provider_id" = i32, Path, description = "Identifiant du fournisseur")
    )
)]
#[delete("/payment-providers/{provider_id}")]
pub async fn delete_payment_provider(
    state: web::Data<AppState>,
    req: HttpRequest,
    provider_id: web::Path<i32>,
) -> impl Responder {
    if let Err(resp) = ensure_admin(&req, state.get_ref()) {
        return resp;
    }

    let res = sqlx::query("DELETE FROM payment_providers WHERE provider_id = $1")
        .bind(*provider_id)
        .execute(&state.db)
        .await;

    match res {
        Ok(result) if result.rows_affected() == 0 => HttpResponse::NotFound().json(ErrorResponse {
            error: "provider_not_found".into(),
            details: None,
        }),
        Ok(_) => HttpResponse::Ok().json(StatusResponse {
            status: "deleted".into(),
        }),
        Err(_) => HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        }),
    }
}
