use actix_web::http::header::CONTENT_TYPE;
use actix_web::{HttpResponse, Responder, post, web};
use sqlx::Error;

use crate::{
    auth::{
        encode_jwt, fetch_user_auth, hash_password, now_ts, random_password_token,
        validate_password_strength, verify_password,
    },
    handles::{generate_unique_handle, handle_available, is_valid_handle, normalize_handle},
    models::{
        Claims, ErrorResponse, LoginPayload, OAuthPayload, RegisterPayload, StatusResponse,
        TokenResponse,
    },
    state::AppState,
};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct OAuthPath {
    provider: String,
}

#[utoipa::path(
    post,
    path = "/auth/register",
    tag = "auth",
    request_body = RegisterPayload,
    responses(
        (status = 201, description = "User registered", body = StatusResponse),
        (status = 400, description = "Invalid payload", body = ErrorResponse),
        (status = 409, description = "Email already exists", body = ErrorResponse),
        (status = 500, description = "Database or hashing error", body = ErrorResponse)
    )
)]
#[post("/auth/register")]
pub async fn register(
    state: web::Data<AppState>,
    payload: web::Json<RegisterPayload>,
) -> impl Responder {
    let email = payload.email.trim().to_lowercase();
    let password = payload.password.trim();
    let requested_handle = payload
        .handle
        .as_ref()
        .map(|raw| normalize_handle(raw).normalized);

    if email.is_empty() {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("email requis".into()),
        });
    }
    if let Err(reason) = validate_password_strength(password) {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "weak_password".into(),
            details: Some(reason.into()),
        });
    }

    let handle = match requested_handle {
        Some(ref h) => {
            if !is_valid_handle(h) {
                return HttpResponse::BadRequest().json(ErrorResponse {
                    error: "invalid_handle".into(),
                    details: Some("format attendu: 4-32 chars [a-z0-9._-]".into()),
                });
            }
            match handle_available(&state.db, h).await {
                Ok(true) => h.clone(),
                Ok(false) => {
                    return HttpResponse::Conflict().json(ErrorResponse {
                        error: "handle_taken".into(),
                        details: None,
                    });
                }
                Err(_) => {
                    return HttpResponse::InternalServerError().json(ErrorResponse {
                        error: "db_error".into(),
                        details: None,
                    });
                }
            }
        }
        None => match generate_unique_handle(&state.db).await {
            Ok(h) => h,
            Err(_) => {
                return HttpResponse::InternalServerError().json(ErrorResponse {
                    error: "handle_generation_failed".into(),
                    details: None,
                });
            }
        },
    };

    let hash = match hash_password(password) {
        Ok(h) => h,
        Err(_) => {
            return HttpResponse::InternalServerError().json(ErrorResponse {
                error: "hash_failed".into(),
                details: None,
            });
        }
    };

    let res = sqlx::query("INSERT INTO users (email, password_hash, handle) VALUES ($1, $2, $3)")
        .bind(&email)
        .bind(&hash)
        .bind(&handle)
        .execute(&state.db)
        .await;

    match res {
        Ok(_) => HttpResponse::Created().json(StatusResponse {
            status: "ok".into(),
        }),
        Err(e) => match e {
            Error::Database(db_err) if db_err.code().as_deref() == Some("23505") => {
                let constraint = db_err.constraint().unwrap_or_default();
                let error = if constraint.contains("handle") {
                    "handle_taken"
                } else {
                    "email_taken"
                };
                HttpResponse::Conflict().json(ErrorResponse {
                    error: error.into(),
                    details: None,
                })
            }
            _ => HttpResponse::InternalServerError().json(ErrorResponse {
                error: "db_error".into(),
                details: None,
            }),
        },
    }
}

#[utoipa::path(
    post,
    path = "/auth/oauth/{provider}",
    tag = "auth",
    params(
        ("provider" = String, Path, description = "oauth provider (google)")
    ),
    request_body = OAuthPayload,
    responses(
        (status = 200, description = "Valid provider token", body = TokenResponse),
        (status = 400, description = "Invalid payload or provider", body = ErrorResponse),
        (status = 401, description = "Invalid provider token", body = ErrorResponse),
        (status = 500, description = "Database or token creation error", body = ErrorResponse)
    )
)]
#[post("/auth/oauth/{provider}")]
pub async fn oauth_login(
    state: web::Data<AppState>,
    path: web::Path<OAuthPath>,
    payload: web::Json<OAuthPayload>,
) -> HttpResponse {
    let provider = path.into_inner().provider.to_lowercase();
    match provider.as_str() {
        "google" => oauth_google(state, payload.into_inner()).await,
        "apple" => oauth_apple(state, payload.into_inner()).await,
        _ => HttpResponse::BadRequest().json(ErrorResponse {
            error: "unsupported_provider".into(),
            details: Some("provider must be 'google' ou 'apple'".into()),
        }),
    }
}

async fn oauth_google(state: web::Data<AppState>, payload: OAuthPayload) -> HttpResponse {
    let id_token = payload
        .id_token
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());
    let access_token = payload
        .access_token
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());
    if id_token.is_none() && access_token.is_none() {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("idToken ou accessToken requis".into()),
        });
    }

    let mut allowed_aud = Vec::new();
    if let Some(web_id) = state.google_client_id.as_ref() {
        allowed_aud.push(web_id.as_str());
    }
    if let Some(android_id) = state.google_android_client_id.as_ref() {
        allowed_aud.push(android_id.as_str());
    }
    if let Some(ios_id) = state.google_ios_client_id.as_ref() {
        allowed_aud.push(ios_id.as_str());
    }
    if allowed_aud.is_empty() {
        return HttpResponse::InternalServerError().json(ErrorResponse {
            error: "oauth_not_configured".into(),
            details: Some("aucun FIESTAAA_GOOGLE_*_CLIENT_ID defini (WEB/ANDROID/IOS)".into()),
        });
    }

    let mut query = Vec::new();
    if let Some(idt) = id_token {
        query.push(("id_token", idt));
    }
    if let Some(at) = access_token {
        query.push(("access_token", at));
    }

    let token_info = match state
        .http_client
        .get("https://oauth2.googleapis.com/tokeninfo")
        .query(&query)
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => match resp.json::<serde_json::Value>().await {
            Ok(v) => v,
            Err(_) => {
                return HttpResponse::Unauthorized().json(ErrorResponse {
                    error: "invalid_token".into(),
                    details: Some("Réponse Google invalide".into()),
                });
            }
        },
        _ => {
            return HttpResponse::Unauthorized().json(ErrorResponse {
                error: "invalid_token".into(),
                details: Some("Échec vérification Google".into()),
            });
        }
    };

    // tokeninfo returns `aud` for id_tokens and `audience` for access_tokens.
    let aud = token_info
        .get("aud")
        .or_else(|| token_info.get("audience"))
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    if !allowed_aud.contains(&aud) {
        return HttpResponse::Unauthorized().json(ErrorResponse {
            error: "invalid_token".into(),
            details: Some("aud mismatch".into()),
        });
    }

    let email = token_info
        .get("email")
        .and_then(|v| v.as_str())
        .or(payload.email.as_deref());
    if email.is_none() {
        return HttpResponse::Unauthorized().json(ErrorResponse {
            error: "email_required".into(),
            details: Some("Email manquant dans le token".into()),
        });
    }
    let email = email.unwrap().to_lowercase();

    let handle = match sqlx::query_scalar::<_, String>(
        "SELECT handle FROM users WHERE lower(email)=lower($1)",
    )
    .bind(&email)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(h)) => h,
        Ok(None) => {
            let new_handle = match generate_unique_handle(&state.db).await {
                Ok(h) => h,
                Err(_) => {
                    return HttpResponse::InternalServerError().json(ErrorResponse {
                        error: "handle_generation_failed".into(),
                        details: None,
                    });
                }
            };
            let pwd = random_password_token();
            let hash = match hash_password(&pwd) {
                Ok(h) => h,
                Err(_) => {
                    return HttpResponse::InternalServerError().json(ErrorResponse {
                        error: "hash_failed".into(),
                        details: None,
                    });
                }
            };
            let res =
                sqlx::query("INSERT INTO users (email, handle, password_hash) VALUES ($1, $2, $3)")
                    .bind(&email)
                    .bind(&new_handle)
                    .bind(&hash)
                    .execute(&state.db)
                    .await;
            match res {
                Ok(_) => new_handle,
                Err(e) => match e {
                    Error::Database(db_err) if db_err.code().as_deref() == Some("23505") => {
                        // Race: user created between SELECT and INSERT
                        let existing = sqlx::query_scalar::<_, String>(
                            "SELECT handle FROM users WHERE lower(email)=lower($1)",
                        )
                        .bind(&email)
                        .fetch_optional(&state.db)
                        .await
                        .ok()
                        .flatten();
                        existing.unwrap_or(new_handle)
                    }
                    _ => {
                        return HttpResponse::InternalServerError().json(ErrorResponse {
                            error: "db_error".into(),
                            details: None,
                        });
                    }
                },
            }
        }
        Err(_) => {
            return HttpResponse::InternalServerError().json(ErrorResponse {
                error: "db_error".into(),
                details: None,
            });
        }
    };

    let exp = (now_ts() + 24 * 3600) as usize;
    let claims = Claims {
        sub: email.clone(),
        handle: handle.clone(),
        exp,
    };

    match encode_jwt(&claims, &state.jwt_secret) {
        Ok(token) => HttpResponse::Ok()
            .insert_header((CONTENT_TYPE, "application/json"))
            .json(TokenResponse {
                token,
                email,
                handle,
            }),
        Err(_) => HttpResponse::InternalServerError().json(ErrorResponse {
            error: "token_creation_failed".into(),
            details: None,
        }),
    }
}

async fn oauth_apple(state: web::Data<AppState>, payload: OAuthPayload) -> HttpResponse {
    let mut allowed_aud = Vec::new();
    if let Some(service_id) = state.apple_service_id.as_ref() {
        allowed_aud.push(service_id.as_str());
    }
    if let Some(app_id) = state.apple_app_id.as_ref() {
        allowed_aud.push(app_id.as_str());
    }
    if allowed_aud.is_empty() {
        return HttpResponse::InternalServerError().json(ErrorResponse {
            error: "oauth_not_configured".into(),
            details: Some("FIESTAAA_APPLE_SERVICE_ID ou FIESTAAA_APPLE_APP_ID manquant".into()),
        });
    }
    let id_token = match payload
        .id_token
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        Some(v) => v,
        None => {
            return HttpResponse::BadRequest().json(ErrorResponse {
                error: "invalid_payload".into(),
                details: Some("idToken requis".into()),
            });
        }
    };

    let header = match jsonwebtoken::decode_header(id_token) {
        Ok(h) => h,
        Err(_) => {
            return HttpResponse::Unauthorized().json(ErrorResponse {
                error: "invalid_token".into(),
                details: Some("Header Apple invalide".into()),
            });
        }
    };
    let kid = match header.kid {
        Some(k) => k,
        None => {
            return HttpResponse::Unauthorized().json(ErrorResponse {
                error: "invalid_token".into(),
                details: Some("kid manquant".into()),
            });
        }
    };
    let alg = header.alg;
    if alg != jsonwebtoken::Algorithm::RS256 {
        return HttpResponse::Unauthorized().json(ErrorResponse {
            error: "invalid_token".into(),
            details: Some("alg non supporté".into()),
        });
    }

    let Some(decoding_key) = fetch_apple_decoding_key(&state, &kid).await else {
        return HttpResponse::Unauthorized().json(ErrorResponse {
            error: "invalid_token".into(),
            details: Some("clé Apple introuvable".into()),
        });
    };

    let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::RS256);
    validation.set_audience(&allowed_aud);
    validation.set_issuer(&["https://appleid.apple.com"]);

    let claims = match jsonwebtoken::decode::<crate::models::AppleClaims>(
        id_token,
        &decoding_key,
        &validation,
    ) {
        Ok(data) => data.claims,
        Err(_) => {
            return HttpResponse::Unauthorized().json(ErrorResponse {
                error: "invalid_token".into(),
                details: Some("JWT Apple invalide".into()),
            });
        }
    };

    let email = claims.email.or(payload.email).map(|e| e.to_lowercase());
    let Some(email) = email else {
        return HttpResponse::Unauthorized().json(ErrorResponse {
            error: "email_required".into(),
            details: Some("Email absent du token Apple".into()),
        });
    };

    let handle = match sqlx::query_scalar::<_, String>(
        "SELECT handle FROM users WHERE lower(email)=lower($1)",
    )
    .bind(&email)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(h)) => h,
        Ok(None) => {
            let new_handle = match generate_unique_handle(&state.db).await {
                Ok(h) => h,
                Err(_) => {
                    return HttpResponse::InternalServerError().json(ErrorResponse {
                        error: "handle_generation_failed".into(),
                        details: None,
                    });
                }
            };
            let pwd = random_password_token();
            let hash = match hash_password(&pwd) {
                Ok(h) => h,
                Err(_) => {
                    return HttpResponse::InternalServerError().json(ErrorResponse {
                        error: "hash_failed".into(),
                        details: None,
                    });
                }
            };
            let res =
                sqlx::query("INSERT INTO users (email, handle, password_hash) VALUES ($1, $2, $3)")
                    .bind(&email)
                    .bind(&new_handle)
                    .bind(&hash)
                    .execute(&state.db)
                    .await;
            match res {
                Ok(_) => new_handle,
                Err(e) => match e {
                    sqlx::Error::Database(db_err) if db_err.code().as_deref() == Some("23505") => {
                        let existing = sqlx::query_scalar::<_, String>(
                            "SELECT handle FROM users WHERE lower(email)=lower($1)",
                        )
                        .bind(&email)
                        .fetch_optional(&state.db)
                        .await
                        .ok()
                        .flatten();
                        existing.unwrap_or(new_handle)
                    }
                    _ => {
                        return HttpResponse::InternalServerError().json(ErrorResponse {
                            error: "db_error".into(),
                            details: None,
                        });
                    }
                },
            }
        }
        Err(_) => {
            return HttpResponse::InternalServerError().json(ErrorResponse {
                error: "db_error".into(),
                details: None,
            });
        }
    };

    let exp = claims.exp;
    let claims = Claims {
        sub: email.clone(),
        handle: handle.clone(),
        exp,
    };

    match encode_jwt(&claims, &state.jwt_secret) {
        Ok(token) => HttpResponse::Ok()
            .insert_header((CONTENT_TYPE, "application/json"))
            .json(TokenResponse {
                token,
                email,
                handle,
            }),
        Err(_) => HttpResponse::InternalServerError().json(ErrorResponse {
            error: "token_creation_failed".into(),
            details: None,
        }),
    }
}

#[derive(serde::Deserialize)]
struct AppleJwk {
    kid: String,
    n: String,
    e: String,
}

#[derive(serde::Deserialize)]
struct AppleJwkSet {
    keys: Vec<AppleJwk>,
}

async fn fetch_apple_decoding_key(
    state: &web::Data<AppState>,
    kid: &str,
) -> Option<jsonwebtoken::DecodingKey> {
    let resp = state
        .http_client
        .get("https://appleid.apple.com/auth/keys")
        .send()
        .await
        .ok()?;
    let jwks: AppleJwkSet = resp.json().await.ok()?;
    let key = jwks.keys.into_iter().find(|k| k.kid == kid)?;
    jsonwebtoken::DecodingKey::from_rsa_components(&key.n, &key.e).ok()
}
#[utoipa::path(
    post,
    path = "/auth/login",
    tag = "auth",
    request_body = LoginPayload,
    responses(
        (status = 200, description = "Valid credentials", body = TokenResponse),
        (status = 401, description = "Invalid credentials", body = ErrorResponse),
        (status = 500, description = "Database or token creation error", body = ErrorResponse)
    )
)]
#[post("/auth/login")]
pub async fn login(state: web::Data<AppState>, payload: web::Json<LoginPayload>) -> impl Responder {
    let auth_row = match fetch_user_auth(&state.db, &payload.identifier).await {
        Ok(Some(row)) => row,
        Ok(None) => {
            return HttpResponse::Unauthorized().json(ErrorResponse {
                error: "invalid_credentials".into(),
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

    if !verify_password(&auth_row.password_hash, &payload.password) {
        return HttpResponse::Unauthorized().json(ErrorResponse {
            error: "invalid_credentials".into(),
            details: None,
        });
    }

    let exp = (now_ts() + 24 * 3600) as usize;
    let claims = Claims {
        sub: auth_row.email.clone(),
        handle: auth_row.handle.clone(),
        exp,
    };

    match encode_jwt(&claims, &state.jwt_secret) {
        Ok(token) => HttpResponse::Ok()
            .insert_header((CONTENT_TYPE, "application/json"))
            .json(TokenResponse {
                token,
                email: auth_row.email,
                handle: auth_row.handle,
            }),
        Err(_) => HttpResponse::InternalServerError().json(ErrorResponse {
            error: "token_creation_failed".into(),
            details: None,
        }),
    }
}
