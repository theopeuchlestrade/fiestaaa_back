use actix_web::http::header::{CACHE_CONTROL, CONTENT_TYPE, PRAGMA};
use actix_web::{HttpRequest, HttpResponse, Responder, post, web};
use sqlx::{Error, PgPool};

use crate::{
    auth::{
        build_cleared_session_cookie, build_session_cookie, encode_jwt, fetch_user_auth,
        hash_password, now_ts, random_password_token, validate_password_strength, verify_password,
    },
    handles::{generate_unique_handle, handle_available, is_valid_handle, normalize_handle},
    models::{
        AppleClaims, Claims, ErrorResponse, LoginPayload, OAuthPayload, RegisterPayload,
        StatusResponse, TokenResponse,
    },
    state::AppState,
};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct OAuthPath {
    provider: String,
}

fn auth_cookie_is_secure(state: &AppState) -> bool {
    state.app_base_url.starts_with("https://")
}

async fn enforce_auth_rate_limit(
    req: &HttpRequest,
    state: &AppState,
    scope: &str,
) -> Result<(), HttpResponse> {
    let remote = req
        .connection_info()
        .realip_remote_addr()
        .unwrap_or("unknown")
        .to_string();
    let key = format!("auth:{scope}:{remote}");
    if state.auth_rate_limiter.allow(&key).await {
        Ok(())
    } else {
        Err(HttpResponse::TooManyRequests().json(ErrorResponse {
            error: "rate_limited".into(),
            details: Some("too many authentication attempts".into()),
        }))
    }
}

fn token_response(
    token: String,
    email: String,
    handle: String,
    secure_cookie: bool,
) -> HttpResponse {
    HttpResponse::Ok()
        .insert_header((CONTENT_TYPE, "application/json"))
        .insert_header((CACHE_CONTROL, "no-store"))
        .insert_header((PRAGMA, "no-cache"))
        .cookie(build_session_cookie(&token, secure_cookie))
        .json(TokenResponse {
            token,
            email,
            handle,
        })
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
    req: HttpRequest,
    state: web::Data<AppState>,
    path: web::Path<OAuthPath>,
    payload: web::Json<OAuthPayload>,
) -> HttpResponse {
    let provider = path.into_inner().provider.to_lowercase();
    if let Err(resp) =
        enforce_auth_rate_limit(&req, state.get_ref(), &format!("oauth:{provider}")).await
    {
        return resp;
    }
    match provider.as_str() {
        "google" => oauth_google(state, payload.into_inner()).await,
        "apple" => oauth_apple(state, payload.into_inner()).await,
        _ => HttpResponse::BadRequest().json(ErrorResponse {
            error: "unsupported_provider".into(),
            details: Some("provider must be 'google' ou 'apple'".into()),
        }),
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct OAuthUserRow {
    id: i64,
    email: String,
    handle: String,
}

fn normalize_provider_email(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_lowercase())
}

fn json_truthy(value: Option<&serde_json::Value>) -> bool {
    match value {
        Some(serde_json::Value::Bool(flag)) => *flag,
        Some(serde_json::Value::String(flag)) => flag.eq_ignore_ascii_case("true"),
        _ => false,
    }
}

fn google_subject(token_info: &serde_json::Value) -> Option<String> {
    token_info
        .get("sub")
        .or_else(|| token_info.get("user_id"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

async fn fetch_google_userinfo(
    state: &web::Data<AppState>,
    access_token: &str,
) -> Option<serde_json::Value> {
    let response = state
        .http_client
        .get("https://www.googleapis.com/oauth2/v3/userinfo")
        .bearer_auth(access_token)
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.json::<serde_json::Value>().await.ok()
}

async fn fetch_oauth_user_by_identity(
    db: &PgPool,
    provider: &str,
    provider_subject: &str,
) -> Result<Option<OAuthUserRow>, sqlx::Error> {
    sqlx::query_as::<_, OAuthUserRow>(
        "SELECT u.id, u.email, u.handle
         FROM oauth_identities oi
         JOIN users u ON u.id = oi.user_id
         WHERE oi.provider = $1 AND oi.provider_subject = $2",
    )
    .bind(provider)
    .bind(provider_subject)
    .fetch_optional(db)
    .await
}

async fn touch_oauth_identity(
    db: &PgPool,
    provider: &str,
    provider_subject: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE oauth_identities
         SET last_login_at = NOW()
         WHERE provider = $1 AND provider_subject = $2",
    )
    .bind(provider)
    .bind(provider_subject)
    .execute(db)
    .await?;
    Ok(())
}

async fn fetch_user_by_email(
    db: &PgPool,
    email: &str,
) -> Result<Option<OAuthUserRow>, sqlx::Error> {
    sqlx::query_as::<_, OAuthUserRow>(
        "SELECT id, email, handle FROM users WHERE lower(email) = lower($1)",
    )
    .bind(email)
    .fetch_optional(db)
    .await
}

async fn create_oauth_user(
    state: &web::Data<AppState>,
    email: &str,
) -> Result<OAuthUserRow, HttpResponse> {
    let new_handle = match generate_unique_handle(&state.db).await {
        Ok(handle) => handle,
        Err(_) => {
            return Err(HttpResponse::InternalServerError().json(ErrorResponse {
                error: "handle_generation_failed".into(),
                details: None,
            }));
        }
    };
    let password = random_password_token();
    let hash = match hash_password(&password) {
        Ok(hash) => hash,
        Err(_) => {
            return Err(HttpResponse::InternalServerError().json(ErrorResponse {
                error: "hash_failed".into(),
                details: None,
            }));
        }
    };

    let inserted = sqlx::query_as::<_, OAuthUserRow>(
        "INSERT INTO users (email, handle, password_hash)
         VALUES ($1, $2, $3)
         RETURNING id, email, handle",
    )
    .bind(email)
    .bind(&new_handle)
    .bind(&hash)
    .fetch_one(&state.db)
    .await;

    match inserted {
        Ok(user) => Ok(user),
        Err(Error::Database(db_err)) if db_err.code().as_deref() == Some("23505") => {
            match fetch_user_by_email(&state.db, email).await {
                Ok(Some(user)) => Ok(user),
                _ => Err(HttpResponse::InternalServerError().json(ErrorResponse {
                    error: "db_error".into(),
                    details: None,
                })),
            }
        }
        Err(_) => Err(HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        })),
    }
}

async fn resolve_oauth_user(
    state: &web::Data<AppState>,
    provider: &str,
    provider_subject: &str,
    email: Option<String>,
) -> Result<OAuthUserRow, HttpResponse> {
    match fetch_oauth_user_by_identity(&state.db, provider, provider_subject).await {
        Ok(Some(user)) => {
            let _ = touch_oauth_identity(&state.db, provider, provider_subject).await;
            return Ok(user);
        }
        Ok(None) => {}
        Err(_) => {
            return Err(HttpResponse::InternalServerError().json(ErrorResponse {
                error: "db_error".into(),
                details: None,
            }));
        }
    }

    let Some(email) = email else {
        return Err(HttpResponse::Unauthorized().json(ErrorResponse {
            error: "email_required".into(),
            details: Some("Email absent de l'identite OAuth verifiee".into()),
        }));
    };

    let user = match fetch_user_by_email(&state.db, &email).await {
        Ok(Some(user)) => user,
        Ok(None) => create_oauth_user(state, &email).await?,
        Err(_) => {
            return Err(HttpResponse::InternalServerError().json(ErrorResponse {
                error: "db_error".into(),
                details: None,
            }));
        }
    };

    let insert_identity = sqlx::query(
        "INSERT INTO oauth_identities (provider, provider_subject, user_id)
         VALUES ($1, $2, $3)
         ON CONFLICT (provider, provider_subject) DO NOTHING",
    )
    .bind(provider)
    .bind(provider_subject)
    .bind(user.id)
    .execute(&state.db)
    .await;
    if insert_identity.is_err() {
        return Err(HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        }));
    }

    match fetch_oauth_user_by_identity(&state.db, provider, provider_subject).await {
        Ok(Some(user)) => {
            let _ = touch_oauth_identity(&state.db, provider, provider_subject).await;
            Ok(user)
        }
        _ => Err(HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        })),
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
    if id_token.is_some() {
        let issuer = token_info.get("iss").and_then(|value| value.as_str());
        if !matches!(
            issuer,
            Some("accounts.google.com") | Some("https://accounts.google.com")
        ) {
            return HttpResponse::Unauthorized().json(ErrorResponse {
                error: "invalid_token".into(),
                details: Some("issuer Google invalide".into()),
            });
        }
    }

    let mut userinfo = None;
    let mut provider_subject = google_subject(&token_info);
    if provider_subject.is_none()
        && let Some(token) = access_token
    {
        userinfo = fetch_google_userinfo(&state, token).await;
        provider_subject = userinfo.as_ref().and_then(google_subject);
    }
    let Some(provider_subject) = provider_subject else {
        return HttpResponse::Unauthorized().json(ErrorResponse {
            error: "invalid_token".into(),
            details: Some("subject Google manquant".into()),
        });
    };

    let mut verified_email =
        normalize_provider_email(token_info.get("email").and_then(|value| value.as_str()))
            .filter(|_| json_truthy(token_info.get("email_verified")));
    if verified_email.is_none()
        && let Some(token) = access_token
    {
        if userinfo.is_none() {
            userinfo = fetch_google_userinfo(&state, token).await;
        }
        verified_email = userinfo.as_ref().and_then(|profile| {
            normalize_provider_email(profile.get("email").and_then(|value| value.as_str()))
                .filter(|_| json_truthy(profile.get("email_verified")))
        });
    }

    let user = match resolve_oauth_user(&state, "google", &provider_subject, verified_email).await {
        Ok(user) => user,
        Err(resp) => return resp,
    };

    let exp = (now_ts() + 24 * 3600) as usize;
    let claims = Claims {
        sub: user.email.clone(),
        handle: user.handle.clone(),
        exp,
    };
    let secure_cookie = auth_cookie_is_secure(state.get_ref());

    match encode_jwt(&claims, &state.jwt_secret) {
        Ok(token) => token_response(token, user.email, user.handle, secure_cookie),
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

    let claims = match jsonwebtoken::decode::<AppleClaims>(id_token, &decoding_key, &validation) {
        Ok(data) => data.claims,
        Err(_) => {
            return HttpResponse::Unauthorized().json(ErrorResponse {
                error: "invalid_token".into(),
                details: Some("JWT Apple invalide".into()),
            });
        }
    };

    let verified_email = normalize_provider_email(claims.email.as_deref())
        .filter(|_| json_truthy(claims.email_verified.as_ref()));
    let user = match resolve_oauth_user(&state, "apple", &claims.sub, verified_email).await {
        Ok(user) => user,
        Err(resp) => return resp,
    };

    let exp = claims.exp;
    let claims = Claims {
        sub: user.email.clone(),
        handle: user.handle.clone(),
        exp,
    };
    let secure_cookie = auth_cookie_is_secure(state.get_ref());

    match encode_jwt(&claims, &state.jwt_secret) {
        Ok(token) => token_response(token, user.email, user.handle, secure_cookie),
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
    path = "/auth/logout",
    tag = "auth",
    responses(
        (status = 200, description = "Session invalidated", body = StatusResponse)
    )
)]
#[post("/auth/logout")]
pub async fn logout(state: web::Data<AppState>) -> impl Responder {
    HttpResponse::Ok()
        .cookie(build_cleared_session_cookie(auth_cookie_is_secure(
            state.get_ref(),
        )))
        .json(StatusResponse {
            status: "logged_out".into(),
        })
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
pub async fn login(
    req: HttpRequest,
    state: web::Data<AppState>,
    payload: web::Json<LoginPayload>,
) -> impl Responder {
    if let Err(resp) = enforce_auth_rate_limit(&req, state.get_ref(), "login").await {
        return resp;
    }
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
    let secure_cookie = auth_cookie_is_secure(state.get_ref());

    match encode_jwt(&claims, &state.jwt_secret) {
        Ok(token) => token_response(token, auth_row.email, auth_row.handle, secure_cookie),
        Err(_) => HttpResponse::InternalServerError().json(ErrorResponse {
            error: "token_creation_failed".into(),
            details: None,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::{google_subject, json_truthy, normalize_provider_email};

    #[test]
    fn normalize_provider_email_trims_and_lowercases() {
        assert_eq!(
            normalize_provider_email(Some("  USER@Example.COM ")).as_deref(),
            Some("user@example.com")
        );
        assert_eq!(normalize_provider_email(Some("   ")), None);
        assert_eq!(normalize_provider_email(None), None);
    }

    #[test]
    fn json_truthy_accepts_boolean_and_string_true() {
        assert!(json_truthy(Some(&serde_json::json!(true))));
        assert!(json_truthy(Some(&serde_json::json!("true"))));
        assert!(!json_truthy(Some(&serde_json::json!(false))));
        assert!(!json_truthy(Some(&serde_json::json!("false"))));
        assert!(!json_truthy(None));
    }

    #[test]
    fn google_subject_prefers_sub_and_rejects_blank_values() {
        assert_eq!(
            google_subject(&serde_json::json!({ "sub": "google-user-123" })).as_deref(),
            Some("google-user-123")
        );
        assert_eq!(
            google_subject(&serde_json::json!({ "user_id": "legacy-user-id" })).as_deref(),
            Some("legacy-user-id")
        );
        assert_eq!(google_subject(&serde_json::json!({ "sub": "   " })), None);
    }
}
