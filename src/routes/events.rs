use actix_web::{HttpRequest, HttpResponse, Responder, delete, get, patch, post, put, web};
use chrono::{Duration, NaiveDate, Utc};
use log::{info, warn};
use regex::Regex;
use serde::Deserialize;
use serde_json::json;
use sqlx::{Error, PgPool, Row};
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use uuid::Uuid;

use crate::{
    auth::extract_active_claims_from_auth,
    models::{
        AddressSuggestion, ErrorResponse, Event, EventCustomItemPayload, EventExpenseBalanceView,
        EventExpenseParticipantView, EventExpensePayload, EventExpenseSettlementView,
        EventExpenseView, EventExpensesSummaryView, EventItemAttachPayload,
        EventItemReservationPayload, EventItemView, EventPatchPayload, EventPayload,
        EventPollCreatePayload, EventPollVotePayload, ItemContribution, PollOptionView,
        PollOptionVoter, PollView, ShareClaimPayload, ShareClaimResponse, ShareTokenResponse,
        StatusResponse,
    },
    notifications::{NotificationRequest, event_member_user_ids, notify_users},
    realtime::{event_types, publish_event, publish_event_type, publish_global_type},
    routes::event_access::{ensure_event_writable, fetch_event_timing},
    security::sha256_hex,
    state::AppState,
};

const OWNER_SHARE_TOKEN_TTL_HOURS: i64 = 24;

async fn claims_email(req: &HttpRequest, state: &AppState) -> Result<String, HttpResponse> {
    let claims = extract_active_claims_from_auth(req, &state.db, &state.jwt_secret).await?;
    Ok(claims.sub.to_lowercase())
}

fn event_projection(event_alias: &str, owner_alias: &str) -> String {
    format!(
        "{event_alias}.event_id,
         {event_alias}.name_event,
         {event_alias}.description,
         {event_alias}.date_event,
         {event_alias}.start_time,
         {event_alias}.end_date,
         {event_alias}.end_time,
         {event_alias}.invitation_deadline,
         fiestaaa_decrypt_text({event_alias}.address_ciphertext) AS address,
         CAST(fiestaaa_decrypt_text({event_alias}.latitude_ciphertext) AS DOUBLE PRECISION) AS latitude,
         CAST(fiestaaa_decrypt_text({event_alias}.longitude_ciphertext) AS DOUBLE PRECISION) AS longitude,
         {event_alias}.payment_provider_id,
         fiestaaa_decrypt_text({event_alias}.payment_identifier_ciphertext) AS payment_identifier,
         {event_alias}.payment_requested_amount,
         {event_alias}.payment_per_person,
         {event_alias}.playlist_url,
         {event_alias}.playlist_provider,
         {event_alias}.enabled_features,
         fiestaaa_decrypt_text({owner_alias}.email_ciphertext) AS owner_email"
    )
}

fn select_events_sql(from_and_where: &str) -> String {
    format!(
        "SELECT {}
         {from_and_where}",
        event_projection("e", "owner"),
    )
}

fn upsert_event_returning_sql(body: &str) -> String {
    format!(
        "WITH saved_event AS (
            {body}
         )
         SELECT {}
         FROM saved_event e
         JOIN users owner ON owner.id = e.owner_user_id",
        event_projection("e", "owner"),
    )
}

#[derive(Debug)]
struct EventOwnerIdentity {
    user_id: i64,
}

fn normalize_item_kind(value: &str) -> Option<String> {
    let normalized = value.trim().to_lowercase();
    match normalized.as_str() {
        "need" | "bring" => Some(normalized),
        _ => None,
    }
}

fn invalid_item_kind_response() -> HttpResponse {
    HttpResponse::BadRequest().json(ErrorResponse {
        error: "invalid_payload".into(),
        details: Some("item_kind doit être 'need' ou 'bring'".into()),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EventItemsScope {
    All,
    Mine,
    ToCover,
    Completed,
}

#[derive(Debug, Deserialize)]
pub struct EventItemsQuery {
    #[serde(default)]
    pub scope: Option<String>,
}

fn normalize_event_items_scope(value: &str) -> Option<EventItemsScope> {
    let normalized = value.trim().to_lowercase();
    match normalized.as_str() {
        "all" => Some(EventItemsScope::All),
        "mine" => Some(EventItemsScope::Mine),
        "to_cover" => Some(EventItemsScope::ToCover),
        "completed" => Some(EventItemsScope::Completed),
        _ => None,
    }
}

fn invalid_items_scope_response() -> HttpResponse {
    HttpResponse::BadRequest().json(ErrorResponse {
        error: "invalid_payload".into(),
        details: Some("scope doit être 'all', 'mine', 'to_cover' ou 'completed'".into()),
    })
}

async fn fetch_event_owner_identity(
    db: &PgPool,
    event_id: i64,
) -> Result<EventOwnerIdentity, HttpResponse> {
    let owner = sqlx::query(
        "SELECT e.owner_user_id,
                fiestaaa_decrypt_text(u.email_ciphertext) AS owner_email
         FROM events e
         JOIN users u ON u.id = e.owner_user_id
         WHERE e.event_id = $1",
    )
    .bind(event_id)
    .fetch_optional(db)
    .await
    .map_err(|_| {
        HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        })
    })?;

    match owner {
        Some(row) => Ok(EventOwnerIdentity {
            user_id: row.get("owner_user_id"),
        }),
        None => Err(HttpResponse::NotFound().json(ErrorResponse {
            error: "event_not_found".into(),
            details: None,
        })),
    }
}

async fn fetch_event_owner_id(db: &PgPool, event_id: i64) -> Result<i64, HttpResponse> {
    fetch_event_owner_identity(db, event_id)
        .await
        .map(|owner| owner.user_id)
}

async fn ensure_event_owner(
    req: &HttpRequest,
    state: &AppState,
    event_id: i64,
) -> Result<(), HttpResponse> {
    let requester = claims_email(req, state).await?;
    if state.admin_emails.contains(&requester) {
        return Ok(());
    }
    let requester_id = fetch_user_id(&state.db, &requester).await?;
    let owner_id = fetch_event_owner_id(&state.db, event_id).await?;
    if owner_id == requester_id {
        Ok(())
    } else {
        Err(HttpResponse::Forbidden().json(ErrorResponse {
            error: "forbidden".into(),
            details: Some("only the creator can perform this action".into()),
        }))
    }
}

async fn ensure_event_member(
    req: &HttpRequest,
    state: &AppState,
    event_id: i64,
) -> Result<(), HttpResponse> {
    let requester = claims_email(req, state).await?;
    let requester_id = fetch_user_id(&state.db, &requester).await?;
    let owner_id = fetch_event_owner_id(&state.db, event_id).await?;
    if owner_id == requester_id {
        return Ok(());
    }

    let is_member = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(
            SELECT 1
            FROM invitations i
            WHERE i.event_id = $1
              AND i.user_id = $2
              AND i.status = 'Accepted'
        )",
    )
    .bind(event_id)
    .bind(requester_id)
    .fetch_one(&state.db)
    .await
    .map_err(|_| server_error())?;

    if is_member {
        Ok(())
    } else {
        Err(HttpResponse::Forbidden().json(ErrorResponse {
            error: "forbidden".into(),
            details: Some("membership required".into()),
        }))
    }
}

async fn notify_event_members(state: &AppState, event: &Event, updated_fields: &[&str]) {
    if updated_fields.is_empty() || !state.notifications.is_enabled() {
        return;
    }

    let members = match event_member_user_ids(&state.db, event.event_id).await {
        Ok(list) => list,
        Err(err) => {
            warn!("failed to load event members for notifications: {err}");
            return;
        }
    };

    if members.is_empty() {
        return;
    }

    let fields: Vec<String> = updated_fields.iter().map(|f| f.to_string()).collect();
    let body = format!("{} a été mis à jour", event.name_event);
    let dedup = format!("event_updated:{}", event.event_id);
    notify_users(
        &state.notifications,
        &state.db,
        &members,
        NotificationRequest {
            title: "Événement mis à jour",
            body: body.as_str(),
            data: json!({
                "type": "event_updated",
                "event_id": event.event_id,
                "event_name": event.name_event,
                "fields": fields
            }),
            dedup_base_key: Some(&dedup),
            dedup_ttl: Some(300),
        },
    )
    .await;
}

fn validate_invitation_deadline(
    deadline: Option<NaiveDate>,
    event_date: NaiveDate,
) -> Result<(), HttpResponse> {
    if let Some(limit) = deadline {
        let today = chrono::Utc::now().date_naive();
        if limit < today {
            return Err(HttpResponse::BadRequest().json(ErrorResponse {
                error: "invalid_invitation_deadline".into(),
                details: Some("La date limite de réponse ne peut pas être dans le passé".into()),
            }));
        }
        if limit > event_date {
            return Err(HttpResponse::BadRequest().json(ErrorResponse {
                error: "invalid_invitation_deadline".into(),
                details: Some(
                    "La date limite de réponse doit être avant ou au jour de l'événement".into(),
                ),
            }));
        }
    }

    Ok(())
}

fn validate_event_schedule(
    start_date: NaiveDate,
    start_time: chrono::NaiveTime,
    end_date: Option<NaiveDate>,
    end_time: Option<chrono::NaiveTime>,
) -> Result<(), HttpResponse> {
    match (end_date, end_time) {
        (None, None) => Ok(()),
        (Some(end_date), Some(end_time)) => {
            if (end_date, end_time) < (start_date, start_time) {
                Err(HttpResponse::BadRequest().json(ErrorResponse {
                    error: "invalid_event_schedule".into(),
                    details: Some(
                        "La date et heure de fin doivent être après le début de l'événement".into(),
                    ),
                }))
            } else {
                Ok(())
            }
        }
        _ => Err(HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_event_schedule".into(),
            details: Some("end_date et end_time doivent être renseignés ensemble".into()),
        })),
    }
}

const PLAYLIST_SPOTIFY_REGEX: &str = r"^https?://open\.spotify\.com/.+$";
const PLAYLIST_APPLE_REGEX: &str = r"^https?://music\.apple\.com/.+$";
const PLAYLIST_DEEZER_REGEX: &str = r"^https?://(www\.)?deezer\.com/.+$";
const FEATURE_CARPOOLS: &str = "carpools";
const FEATURE_POLLS: &str = "polls";
const FEATURE_ITEMS: &str = "items";
const FEATURE_TICKETING: &str = "ticketing";
const FEATURE_PLAYLIST: &str = "playlist";
const FEATURE_PAYMENT: &str = "payment";
const FEATURE_EXPENSES: &str = "expenses";
const DEFAULT_EVENT_FEATURES: [&str; 3] = [FEATURE_CARPOOLS, FEATURE_POLLS, FEATURE_ITEMS];
const ALLOWED_EVENT_FEATURES: [&str; 7] = [
    FEATURE_CARPOOLS,
    FEATURE_POLLS,
    FEATURE_ITEMS,
    FEATURE_TICKETING,
    FEATURE_PLAYLIST,
    FEATURE_PAYMENT,
    FEATURE_EXPENSES,
];

fn default_enabled_features() -> Vec<String> {
    DEFAULT_EVENT_FEATURES
        .iter()
        .map(|feature| (*feature).to_string())
        .collect()
}

fn append_feature(features: &mut Vec<String>, feature: &str) {
    if !features.iter().any(|value| value == feature) {
        features.push(feature.to_string());
    }
}

fn normalize_enabled_features(features: Vec<String>) -> Result<Vec<String>, HttpResponse> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();

    for feature in features {
        let value = feature.trim().to_lowercase();
        if value.is_empty() {
            continue;
        }
        if !ALLOWED_EVENT_FEATURES.contains(&value.as_str()) {
            return Err(HttpResponse::BadRequest().json(ErrorResponse {
                error: "invalid_enabled_features".into(),
                details: Some(format!(
                    "Feature invalide: '{value}'. Valeurs autorisées: {}",
                    ALLOWED_EVENT_FEATURES.join(", ")
                )),
            }));
        }
        if seen.insert(value.clone()) {
            normalized.push(value);
        }
    }

    Ok(normalized)
}

fn has_playlist_metadata(provider: Option<&str>, url: Option<&str>) -> bool {
    provider.is_some_and(|value| !value.trim().is_empty())
        && url.is_some_and(|value| !value.trim().is_empty())
}

fn has_payment_metadata(provider_id: Option<i32>, identifier: Option<&str>) -> bool {
    provider_id.is_some() && identifier.is_some_and(|value| !value.trim().is_empty())
}

fn resolve_enabled_features(
    requested: Option<Vec<String>>,
    payment_provider_id: Option<i32>,
    payment_identifier: Option<&str>,
    playlist_provider: Option<&str>,
    playlist_url: Option<&str>,
) -> Result<Vec<String>, HttpResponse> {
    let features = if let Some(requested_features) = requested {
        normalize_enabled_features(requested_features)?
    } else {
        let mut inferred = default_enabled_features();
        if has_playlist_metadata(playlist_provider, playlist_url) {
            append_feature(&mut inferred, FEATURE_PLAYLIST);
        }
        if has_payment_metadata(payment_provider_id, payment_identifier) {
            append_feature(&mut inferred, FEATURE_PAYMENT);
        }
        inferred
    };

    if features.iter().any(|value| value == FEATURE_PLAYLIST)
        && !has_playlist_metadata(playlist_provider, playlist_url)
    {
        return Err(HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_enabled_features".into(),
            details: Some("playlist nécessite playlist_provider et playlist_url renseignés".into()),
        }));
    }
    if features.iter().any(|value| value == FEATURE_PAYMENT)
        && !has_payment_metadata(payment_provider_id, payment_identifier)
    {
        return Err(HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_enabled_features".into(),
            details: Some(
                "payment nécessite payment_provider_id et payment_identifier renseignés".into(),
            ),
        }));
    }

    Ok(features)
}

fn normalize_playlist_payload(
    provider: Option<String>,
    url: Option<String>,
    clear_provider: bool,
    clear_url: bool,
) -> Result<(Option<String>, Option<String>), HttpResponse> {
    let provider_value = if clear_provider {
        None
    } else {
        provider.map(|value| value.trim().to_lowercase())
    };
    let url_value = if clear_url {
        None
    } else {
        url.map(|value| value.trim().to_string())
    };

    let provider_value = match provider_value {
        Some(value) if value.is_empty() => None,
        other => other,
    };
    let url_value = match url_value {
        Some(value) if value.is_empty() => None,
        other => other,
    };

    match (&provider_value, &url_value) {
        (None, None) => return Ok((None, None)),
        (Some(_), None) | (None, Some(_)) => {
            return Err(HttpResponse::BadRequest().json(ErrorResponse {
                error: "invalid_playlist".into(),
                details: Some(
                    "playlist_url et playlist_provider doivent être renseignés ensemble".into(),
                ),
            }));
        }
        _ => {}
    }

    let provider = provider_value.unwrap();
    let playlist_url = url_value.unwrap();
    let allowed = ["spotify", "apple_music", "deezer"];
    if !allowed.contains(&provider.as_str()) {
        return Err(HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_playlist_provider".into(),
            details: Some("playlist_provider doit être spotify, apple_music ou deezer".into()),
        }));
    }
    let url_pattern = match provider.as_str() {
        "spotify" => PLAYLIST_SPOTIFY_REGEX,
        "apple_music" => PLAYLIST_APPLE_REGEX,
        "deezer" => PLAYLIST_DEEZER_REGEX,
        _ => PLAYLIST_SPOTIFY_REGEX,
    };
    let url_regex = match Regex::new(url_pattern) {
        Ok(regex) => regex,
        Err(_) => {
            return Err(HttpResponse::InternalServerError().json(ErrorResponse {
                error: "playlist_validation_error".into(),
                details: None,
            }));
        }
    };

    if !url_regex.is_match(&playlist_url) {
        return Err(HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_playlist_url".into(),
            details: Some("playlist_url doit correspondre au provider sélectionné".into()),
        }));
    }
    ensure_safe_absolute_http_url(
        &playlist_url,
        "invalid_playlist_url",
        "playlist_url doit utiliser http ou https",
    )?;

    Ok((Some(provider), Some(playlist_url)))
}

async fn expire_overdue_invitations(db: &PgPool) -> Result<(), HttpResponse> {
    sqlx::query(
        "UPDATE invitations i
         SET status = 'Expired'
         FROM events e
         WHERE i.event_id = e.event_id
           AND i.status = 'Waiting'
           AND e.invitation_deadline IS NOT NULL
           AND CURRENT_DATE > e.invitation_deadline",
    )
    .execute(db)
    .await
    .map(|_| ())
    .map_err(|_| server_error())
}

#[derive(Deserialize)]
pub struct AddressSearchQuery {
    pub q: String,
    pub limit: Option<u8>,
}

#[utoipa::path(
    get,
    path = "/geo/address-search",
    tag = "events",
    params(
        ("q" = String, Query, description = "Adresse ou lieu à rechercher"),
        ("limit" = u8, Query, description = "Nombre maximum de suggestions (1-10)")
    ),
    responses(
        (status = 200, description = "Suggestions géocodées", body = [AddressSuggestion]),
        (status = 400, description = "Requête trop courte", body = ErrorResponse),
        (status = 401, description = "Authentification requise", body = ErrorResponse),
        (status = 502, description = "Service de géocodage indisponible", body = ErrorResponse)
    )
)]
#[get("/geo/address-search")]
pub async fn search_address(
    state: web::Data<AppState>,
    req: HttpRequest,
    params: web::Query<AddressSearchQuery>,
) -> impl Responder {
    if let Err(resp) = claims_email(&req, state.get_ref()).await {
        return resp;
    }

    let query = params.q.trim();
    if query.len() < 3 {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "query_too_short".into(),
            details: Some("Au moins 3 caractères requis pour la recherche".into()),
        });
    }
    let limit = params.limit.unwrap_or(5).clamp(1, 10);

    match fetch_address_suggestions(
        &state.http_client,
        &state.geocoding_base_url,
        state.geocoding_country_codes.as_deref(),
        query,
        limit,
    )
    .await
    {
        Ok(results) => HttpResponse::Ok().json(results),
        Err(resp) => resp,
    }
}

async fn fetch_address_suggestions(
    client: &reqwest::Client,
    base_url: &str,
    country_codes: Option<&str>,
    query: &str,
    limit: u8,
) -> Result<Vec<AddressSuggestion>, HttpResponse> {
    let mut url = match reqwest::Url::parse(&format!("{}/search", base_url.trim_end_matches('/'))) {
        Ok(url) => url,
        Err(_) => {
            return Err(HttpResponse::InternalServerError().json(ErrorResponse {
                error: "geocoding_config_error".into(),
                details: None,
            }));
        }
    };

    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair("format", "jsonv2");
        pairs.append_pair("addressdetails", "0");
        pairs.append_pair("limit", &limit.to_string());
        pairs.append_pair("q", query);
        if let Some(cc) = country_codes {
            pairs.append_pair("countrycodes", cc);
        }
    }

    #[derive(Deserialize)]
    struct NominatimPlace {
        display_name: String,
        lat: String,
        lon: String,
    }

    let response = client.get(url).send().await.map_err(|_| {
        HttpResponse::BadGateway().json(ErrorResponse {
            error: "geocoding_unreachable".into(),
            details: None,
        })
    })?;

    if !response.status().is_success() {
        return Err(HttpResponse::BadGateway().json(ErrorResponse {
            error: "geocoding_error".into(),
            details: Some(format!("Status: {}", response.status())),
        }));
    }

    let places: Vec<NominatimPlace> = response.json().await.map_err(|_| {
        HttpResponse::BadGateway().json(ErrorResponse {
            error: "geocoding_parse_error".into(),
            details: None,
        })
    })?;

    let suggestions = places
        .into_iter()
        .filter_map(|place| {
            let lat = place.lat.parse::<f64>().ok()?;
            let lon = place.lon.parse::<f64>().ok()?;
            Some(AddressSuggestion {
                label: place.display_name,
                latitude: lat,
                longitude: lon,
            })
        })
        .collect();

    Ok(suggestions)
}

#[utoipa::path(
    get,
    path = "/events/{event_id}",
    tag = "events",
    responses(
        (status = 200, description = "Événement trouvé", body = Event),
        (status = 401, description = "Authentification requise", body = ErrorResponse),
        (status = 403, description = "Accès non autorisé", body = ErrorResponse),
        (status = 404, description = "Événement introuvable", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Identifiant de l'événement")
    )
)]
#[get("/events/{event_id}")]
pub async fn get_event(
    state: web::Data<AppState>,
    req: HttpRequest,
    event_id: web::Path<i64>,
) -> impl Responder {
    if let Err(resp) = ensure_event_member(&req, state.get_ref(), *event_id).await {
        return resp;
    }

    let sql = select_events_sql(
        "FROM events e
         JOIN users owner ON owner.id = e.owner_user_id
         WHERE e.event_id = $1",
    );

    match sqlx::query_as::<_, Event>(&sql)
    .bind(*event_id)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(event)) => HttpResponse::Ok().json(event),
        Ok(None) => HttpResponse::NotFound().json(ErrorResponse {
            error: "event_not_found".into(),
            details: None,
        }),
        Err(_) => server_error(),
    }
}

#[utoipa::path(
    get,
    path = "/events",
    tag = "events",
    responses(
        (status = 200, description = "Liste des événements", body = [Event]),
        (status = 401, description = "Authentification requise", body = ErrorResponse),
        (status = 500, description = "Erreur base de données", body = ErrorResponse)
    )
)]
#[get("/events")]
pub async fn list_events(state: web::Data<AppState>, req: HttpRequest) -> impl Responder {
    let email = match claims_email(&req, state.get_ref()).await {
        Ok(e) => e,
        Err(resp) => return resp,
    };
    let user_id = match fetch_user_id(&state.db, &email).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    if let Err(resp) = expire_overdue_invitations(&state.db).await {
        return resp;
    }

    let sql = select_events_sql(
        "FROM events e
         JOIN users owner ON owner.id = e.owner_user_id
         WHERE e.owner_user_id = $1
            OR EXISTS (
                SELECT 1
                FROM invitations i
                WHERE i.event_id = e.event_id
                  AND i.user_id = $1
                  AND i.status NOT IN ('Declined', 'Expired')
            )
         ORDER BY e.date_event, e.start_time",
    );

    let res = sqlx::query_as::<_, Event>(&sql)
    .bind(user_id)
    .fetch_all(&state.db)
    .await;

    match res {
        Ok(events) => HttpResponse::Ok().json(events),
        Err(_) => HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        }),
    }
}

#[utoipa::path(
    post,
    path = "/events",
    tag = "events",
    request_body = EventPayload,
    responses(
        (status = 201, description = "Événement créé", body = Event),
        (status = 400, description = "Payload invalide", body = ErrorResponse),
        (status = 403, description = "Non autorisé", body = ErrorResponse),
        (status = 500, description = "Erreur base de données", body = ErrorResponse)
    )
)]
#[post("/events")]
pub async fn create_event(
    state: web::Data<AppState>,
    req: HttpRequest,
    payload: web::Json<EventPayload>,
) -> impl Responder {
    let owner_email = match claims_email(&req, state.get_ref()).await {
        Ok(email) => email,
        Err(resp) => return resp,
    };
    let owner_user_id = match fetch_user_id(&state.db, &owner_email).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let payload = payload.into_inner();
    if payload.name_event.trim().is_empty()
        || payload.description.trim().is_empty()
        || payload.address.trim().is_empty()
    {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some(
                "Les champs name_event, description et address ne peuvent pas être vides".into(),
            ),
        });
    }
    if payload.latitude.is_some() ^ payload.longitude.is_some() {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("Latitude et longitude doivent être renseignées ensemble".into()),
        });
    }
    if let Err(resp) = validate_event_schedule(
        payload.date_event,
        payload.start_time,
        payload.end_date,
        payload.end_time,
    ) {
        return resp;
    }
    if let Err(resp) = validate_invitation_deadline(payload.invitation_deadline, payload.date_event)
    {
        return resp;
    }
    let (payment_provider_id, payment_identifier) = match normalize_payment_info(
        &state.db,
        payload.payment_provider_id,
        payload.payment_identifier,
    )
    .await
    {
        Ok(values) => values,
        Err(resp) => return resp,
    };
    let payment_per_person = if payment_provider_id.is_none() {
        false
    } else {
        payload.payment_per_person.unwrap_or(false)
    };
    let (playlist_provider, playlist_url) = match normalize_playlist_payload(
        payload.playlist_provider,
        payload.playlist_url,
        false,
        false,
    ) {
        Ok(values) => values,
        Err(resp) => return resp,
    };
    let enabled_features = match resolve_enabled_features(
        payload.enabled_features,
        payment_provider_id,
        payment_identifier.as_deref(),
        playlist_provider.as_deref(),
        playlist_url.as_deref(),
    ) {
        Ok(values) => values,
        Err(resp) => return resp,
    };

    let sql = upsert_event_returning_sql(
        "INSERT INTO events (
            name_event,
            description,
            date_event,
            start_time,
            end_date,
            end_time,
            invitation_deadline,
            address_ciphertext,
            latitude_ciphertext,
            longitude_ciphertext,
            payment_provider_id,
            payment_identifier_ciphertext,
            payment_requested_amount,
            payment_per_person,
            playlist_url,
            playlist_provider,
            enabled_features,
            owner_user_id
         )
         VALUES (
            $1,
            $2,
            $3,
            $4,
            $5,
            $6,
            $7,
            fiestaaa_encrypt_text($8),
            CASE WHEN $9 IS NULL THEN NULL ELSE fiestaaa_encrypt_text($9::TEXT) END,
            CASE WHEN $10 IS NULL THEN NULL ELSE fiestaaa_encrypt_text($10::TEXT) END,
            $11,
            CASE WHEN $12 IS NULL THEN NULL ELSE fiestaaa_encrypt_text($12) END,
            $13,
            $14,
            $15,
            $16,
            $17,
            $18
         )
         RETURNING *",
    );

    let res = sqlx::query_as::<_, Event>(&sql)
    .bind(payload.name_event.trim())
    .bind(payload.description.trim())
    .bind(payload.date_event)
    .bind(payload.start_time)
    .bind(payload.end_date)
    .bind(payload.end_time)
    .bind(payload.invitation_deadline)
    .bind(payload.address.trim())
    .bind(payload.latitude)
    .bind(payload.longitude)
    .bind(payment_provider_id)
    .bind(payment_identifier)
    .bind(payload.payment_requested_amount)
    .bind(payment_per_person)
    .bind(&playlist_url)
    .bind(&playlist_provider)
    .bind(&enabled_features)
    .bind(owner_user_id)
    .fetch_one(&state.db)
    .await;

    match res {
        Ok(event) => {
            publish_global_type(&state.redis_client, event_types::EVENTS_CHANGED).await;
            HttpResponse::Created().json(event)
        }
        Err(Error::Database(db_err)) if db_err.code().as_deref() == Some("23503") => {
            HttpResponse::BadRequest().json(ErrorResponse {
                error: "unknown_payment_provider".into(),
                details: None,
            })
        }
        Err(_) => HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        }),
    }
}

#[utoipa::path(
    put,
    path = "/events/{event_id}",
    tag = "events",
    request_body = EventPayload,
    responses(
        (status = 200, description = "Événement mis à jour", body = Event),
        (status = 400, description = "Payload invalide", body = ErrorResponse),
        (status = 403, description = "Non autorisé", body = ErrorResponse),
        (status = 404, description = "Événement introuvable", body = ErrorResponse),
        (status = 500, description = "Erreur base de données", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Identifiant de l'événement")
    )
)]
#[put("/events/{event_id}")]
pub async fn replace_event(
    state: web::Data<AppState>,
    req: HttpRequest,
    event_id: web::Path<i64>,
    payload: web::Json<EventPayload>,
) -> impl Responder {
    if let Err(resp) = ensure_event_owner(&req, state.get_ref(), *event_id).await {
        return resp;
    }
    if let Err(resp) = ensure_event_writable(&state.db, *event_id).await {
        return resp;
    }

    let payload = payload.into_inner();
    let enabled_features_requested = payload.enabled_features.is_some();
    let mut updated_fields = vec!["name", "description", "date", "time", "location"];
    if payload.name_event.trim().is_empty()
        || payload.description.trim().is_empty()
        || payload.address.trim().is_empty()
    {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some(
                "Les champs name_event, description et address ne peuvent pas être vides".into(),
            ),
        });
    }
    if let Err(resp) = validate_event_schedule(
        payload.date_event,
        payload.start_time,
        payload.end_date,
        payload.end_time,
    ) {
        return resp;
    }
    if payload.end_date.is_some() || payload.end_time.is_some() {
        updated_fields.push("duration");
    }
    if let Err(resp) = validate_invitation_deadline(payload.invitation_deadline, payload.date_event)
    {
        return resp;
    }

    let (payment_provider_id, payment_identifier) = match normalize_payment_info(
        &state.db,
        payload.payment_provider_id,
        payload.payment_identifier,
    )
    .await
    {
        Ok(values) => values,
        Err(resp) => return resp,
    };
    let payment_per_person = if payment_provider_id.is_none() {
        false
    } else {
        payload.payment_per_person.unwrap_or(false)
    };
    let (playlist_provider, playlist_url) = match normalize_playlist_payload(
        payload.playlist_provider,
        payload.playlist_url,
        false,
        false,
    ) {
        Ok(values) => values,
        Err(resp) => return resp,
    };
    if playlist_provider.is_some() || playlist_url.is_some() {
        updated_fields.push("playlist");
    }
    let enabled_features = match resolve_enabled_features(
        payload.enabled_features,
        payment_provider_id,
        payment_identifier.as_deref(),
        playlist_provider.as_deref(),
        playlist_url.as_deref(),
    ) {
        Ok(values) => values,
        Err(resp) => return resp,
    };
    if enabled_features_requested {
        updated_fields.push("features");
    }

    let sql = upsert_event_returning_sql(
        "UPDATE events
         SET name_event = $1,
             description = $2,
             date_event = $3,
             start_time = $4,
             end_date = $5,
             end_time = $6,
             invitation_deadline = $7,
             address_ciphertext = fiestaaa_encrypt_text($8),
             latitude_ciphertext = CASE WHEN $9 IS NULL THEN NULL ELSE fiestaaa_encrypt_text($9::TEXT) END,
             longitude_ciphertext = CASE WHEN $10 IS NULL THEN NULL ELSE fiestaaa_encrypt_text($10::TEXT) END,
             payment_provider_id = $11,
             payment_identifier_ciphertext = CASE WHEN $12 IS NULL THEN NULL ELSE fiestaaa_encrypt_text($12) END,
             payment_requested_amount = $13,
             payment_per_person = $14,
             playlist_url = $15,
             playlist_provider = $16,
             enabled_features = $17
         WHERE event_id = $18
         RETURNING *",
    );

    let res = sqlx::query_as::<_, Event>(&sql)
    .bind(payload.name_event.trim())
    .bind(payload.description.trim())
    .bind(payload.date_event)
    .bind(payload.start_time)
    .bind(payload.end_date)
    .bind(payload.end_time)
    .bind(payload.invitation_deadline)
    .bind(payload.address.trim())
    .bind(payload.latitude)
    .bind(payload.longitude)
    .bind(payment_provider_id)
    .bind(payment_identifier)
    .bind(payload.payment_requested_amount)
    .bind(payment_per_person)
    .bind(&playlist_url)
    .bind(&playlist_provider)
    .bind(&enabled_features)
    .bind(*event_id)
    .fetch_optional(&state.db)
    .await;

    match res {
        Ok(Some(event)) => {
            if playlist_provider.is_some() || playlist_url.is_some() {
                info!(
                    "playlist updated event_id={} provider={}",
                    event.event_id,
                    event
                        .playlist_provider
                        .clone()
                        .unwrap_or_else(|| "none".into())
                );
            }
            notify_event_members(state.get_ref(), &event, &updated_fields).await;
            publish_event(
                &state.redis_client,
                event.event_id,
                &json!({"type": event_types::EVENT_UPDATED, "event_id": event.event_id}),
            )
            .await;
            publish_global_type(&state.redis_client, event_types::EVENTS_CHANGED).await;
            HttpResponse::Ok().json(event)
        }
        Ok(None) => HttpResponse::NotFound().json(ErrorResponse {
            error: "event_not_found".into(),
            details: None,
        }),
        Err(Error::Database(db_err)) if db_err.code().as_deref() == Some("23503") => {
            HttpResponse::BadRequest().json(ErrorResponse {
                error: "unknown_payment_provider".into(),
                details: None,
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
    path = "/events/{event_id}",
    tag = "events",
    request_body = EventPatchPayload,
    responses(
        (status = 200, description = "Événement modifié", body = Event),
        (status = 400, description = "Payload invalide", body = ErrorResponse),
        (status = 403, description = "Non autorisé", body = ErrorResponse),
        (status = 404, description = "Événement introuvable", body = ErrorResponse),
        (status = 500, description = "Erreur base de données", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Identifiant de l'événement")
    )
)]
#[patch("/events/{event_id}")]
pub async fn update_event(
    state: web::Data<AppState>,
    req: HttpRequest,
    event_id: web::Path<i64>,
    payload: web::Json<EventPatchPayload>,
) -> impl Responder {
    if let Err(resp) = ensure_event_owner(&req, state.get_ref(), *event_id).await {
        return resp;
    }
    if let Err(resp) = ensure_event_writable(&state.db, *event_id).await {
        return resp;
    }

    let payload = payload.into_inner();
    let mut updated_fields: Vec<&str> = Vec::new();
    if payload.name_event.is_some() {
        updated_fields.push("name");
    }
    if payload.description.is_some() {
        updated_fields.push("description");
    }
    if payload.date_event.is_some() {
        updated_fields.push("date");
    }
    if payload.start_time.is_some() {
        updated_fields.push("time");
    }
    if payload.end_date.is_some() || payload.end_time.is_some() {
        updated_fields.push("duration");
    }
    if payload.address.is_some() || payload.latitude.is_some() || payload.longitude.is_some() {
        updated_fields.push("location");
    }
    if payload.invitation_deadline.is_some() {
        updated_fields.push("deadline");
    }
    if payload.playlist_url.is_some() || payload.playlist_provider.is_some() {
        updated_fields.push("playlist");
    }
    if payload.enabled_features.is_some() {
        updated_fields.push("features");
    }
    if payload
        .name_event
        .as_ref()
        .is_some_and(|v| v.trim().is_empty())
        || payload
            .description
            .as_ref()
            .is_some_and(|v| v.trim().is_empty())
        || payload
            .address
            .as_ref()
            .is_some_and(|v| v.trim().is_empty())
    {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some(
                "Les champs name_event, description et address ne peuvent pas être vides".into(),
            ),
        });
    }
    if payload.latitude.is_some() ^ payload.longitude.is_some() {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("Latitude et longitude doivent être renseignées ensemble".into()),
        });
    }

    let (
        current_provider_id,
        current_identifier,
        current_payment_per_person,
        current_date_event,
        current_start_time,
        current_end_date,
        current_end_time,
        current_invitation_deadline,
        current_playlist_provider,
        current_playlist_url,
        current_enabled_features,
    ) = match fetch_event_payment_info(&state.db, *event_id).await {
        Ok(info) => info,
        Err(resp) => return resp,
    };

    let merged_provider = payload.payment_provider_id.or(current_provider_id);
    let merged_identifier = payload.payment_identifier.clone().or(current_identifier);
    let (payment_provider_id, payment_identifier) =
        match normalize_payment_info(&state.db, merged_provider, merged_identifier).await {
            Ok(values) => values,
            Err(resp) => return resp,
        };
    let payment_per_person = if payment_provider_id.is_none() {
        false
    } else {
        payload
            .payment_per_person
            .unwrap_or(current_payment_per_person)
    };
    let target_date_event = payload.date_event.unwrap_or(current_date_event);
    let target_start_time = payload.start_time.unwrap_or(current_start_time);
    let target_end_date = payload.end_date.unwrap_or(current_end_date);
    let target_end_time = payload.end_time.unwrap_or(current_end_time);
    if let Err(resp) = validate_event_schedule(
        target_date_event,
        target_start_time,
        target_end_date,
        target_end_time,
    ) {
        return resp;
    }
    let invitation_deadline_update = payload.invitation_deadline;
    let target_deadline = invitation_deadline_update.unwrap_or(current_invitation_deadline);
    if let Err(resp) = validate_invitation_deadline(target_deadline, target_date_event) {
        return resp;
    }

    let playlist_provider_set = payload.playlist_provider.is_some();
    let playlist_url_set = payload.playlist_url.is_some();
    let provider_cleared = payload
        .playlist_provider
        .as_ref()
        .is_some_and(|value| value.is_none());
    let url_cleared = payload
        .playlist_url
        .as_ref()
        .is_some_and(|value| value.is_none());
    if provider_cleared ^ url_cleared {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_playlist".into(),
            details: Some(
                "playlist_url et playlist_provider doivent être renseignés ensemble".into(),
            ),
        });
    }
    let merged_playlist_provider = payload
        .playlist_provider
        .clone()
        .unwrap_or(current_playlist_provider);
    let merged_playlist_url = payload.playlist_url.clone().unwrap_or(current_playlist_url);
    let clearing_playlist = provider_cleared && url_cleared;
    let (playlist_provider, playlist_url) = if clearing_playlist {
        (None, None)
    } else {
        match normalize_playlist_payload(
            merged_playlist_provider,
            merged_playlist_url,
            false,
            false,
        ) {
            Ok(values) => values,
            Err(resp) => return resp,
        }
    };
    let enabled_features_input = payload
        .enabled_features
        .clone()
        .unwrap_or(current_enabled_features);
    let enabled_features = match resolve_enabled_features(
        Some(enabled_features_input),
        payment_provider_id,
        payment_identifier.as_deref(),
        playlist_provider.as_deref(),
        playlist_url.as_deref(),
    ) {
        Ok(values) => values,
        Err(resp) => return resp,
    };

    let (invitation_deadline_set, invitation_deadline_value) = match invitation_deadline_update {
        Some(value) => (true, value),
        None => (false, None),
    };
    let (end_date_set, end_date_value) = match payload.end_date {
        Some(value) => (true, value),
        None => (false, None),
    };
    let (end_time_set, end_time_value) = match payload.end_time {
        Some(value) => (true, value),
        None => (false, None),
    };

    let sql = upsert_event_returning_sql(
        "UPDATE events
         SET name_event = COALESCE($1, name_event),
             description = COALESCE($2, description),
             date_event = COALESCE($3, date_event),
             start_time = COALESCE($4, start_time),
             end_date = CASE WHEN $5 THEN $6 ELSE end_date END,
             end_time = CASE WHEN $7 THEN $8 ELSE end_time END,
             invitation_deadline = CASE WHEN $9 THEN $10 ELSE invitation_deadline END,
             address_ciphertext = COALESCE(
                CASE WHEN $11 IS NULL THEN NULL ELSE fiestaaa_encrypt_text($11) END,
                address_ciphertext
             ),
             latitude_ciphertext = COALESCE(
                CASE WHEN $12 IS NULL THEN NULL ELSE fiestaaa_encrypt_text($12::TEXT) END,
                latitude_ciphertext
             ),
             longitude_ciphertext = COALESCE(
                CASE WHEN $13 IS NULL THEN NULL ELSE fiestaaa_encrypt_text($13::TEXT) END,
                longitude_ciphertext
             ),
             payment_provider_id = COALESCE($14, payment_provider_id),
             payment_identifier_ciphertext = COALESCE(
                CASE WHEN $15 IS NULL THEN NULL ELSE fiestaaa_encrypt_text($15) END,
                payment_identifier_ciphertext
             ),
             payment_requested_amount = COALESCE($16, payment_requested_amount),
             payment_per_person = $17,
             playlist_url = CASE WHEN $18 THEN $19 ELSE playlist_url END,
             playlist_provider = CASE WHEN $20 THEN $21 ELSE playlist_provider END,
             enabled_features = $22
         WHERE event_id = $23
         RETURNING *",
    );

    let res = sqlx::query_as::<_, Event>(&sql)
    .bind(payload.name_event.as_ref().map(|v| v.trim()).filter(|v| !v.is_empty()))
    .bind(payload.description.as_ref().map(|v| v.trim()).filter(|v| !v.is_empty()))
    .bind(payload.date_event)
    .bind(payload.start_time)
    .bind(end_date_set)
    .bind(end_date_value)
    .bind(end_time_set)
    .bind(end_time_value)
    .bind(invitation_deadline_set)
    .bind(invitation_deadline_value)
    .bind(payload.address.as_ref().map(|v| v.trim()).filter(|v| !v.is_empty()))
    .bind(payload.latitude)
    .bind(payload.longitude)
    .bind(payment_provider_id)
    .bind(payment_identifier)
    .bind(payload.payment_requested_amount)
    .bind(payment_per_person)
    .bind(playlist_url_set)
    .bind(playlist_url)
    .bind(playlist_provider_set)
    .bind(playlist_provider)
    .bind(&enabled_features)
    .bind(*event_id)
    .fetch_optional(&state.db)
    .await;

    match res {
        Ok(Some(event)) => {
            if updated_fields.contains(&"playlist") {
                info!(
                    "playlist updated event_id={} provider={}",
                    event.event_id,
                    event
                        .playlist_provider
                        .clone()
                        .unwrap_or_else(|| "none".into())
                );
            }
            notify_event_members(state.get_ref(), &event, &updated_fields).await;
            publish_event(
                &state.redis_client,
                event.event_id,
                &json!({"type": event_types::EVENT_UPDATED, "event_id": event.event_id}),
            )
            .await;
            publish_global_type(&state.redis_client, event_types::EVENTS_CHANGED).await;
            HttpResponse::Ok().json(event)
        }
        Ok(None) => HttpResponse::NotFound().json(ErrorResponse {
            error: "event_not_found".into(),
            details: None,
        }),
        Err(Error::Database(db_err)) if db_err.code().as_deref() == Some("23503") => {
            HttpResponse::BadRequest().json(ErrorResponse {
                error: "unknown_payment_provider".into(),
                details: None,
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
    path = "/events/{event_id}",
    tag = "events",
    responses(
        (status = 200, description = "Événement supprimé", body = StatusResponse),
        (status = 403, description = "Non autorisé", body = ErrorResponse),
        (status = 404, description = "Événement introuvable", body = ErrorResponse),
        (status = 500, description = "Erreur base de données", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Identifiant de l'événement")
    )
)]
#[delete("/events/{event_id}")]
pub async fn delete_event(
    state: web::Data<AppState>,
    req: HttpRequest,
    event_id: web::Path<i64>,
) -> impl Responder {
    if let Err(resp) = ensure_event_owner(&req, state.get_ref(), *event_id).await {
        return resp;
    }
    if let Err(resp) = ensure_event_writable(&state.db, *event_id).await {
        return resp;
    }

    let res = sqlx::query("DELETE FROM events WHERE event_id = $1")
        .bind(*event_id)
        .execute(&state.db)
        .await;

    match res {
        Ok(result) if result.rows_affected() == 0 => HttpResponse::NotFound().json(ErrorResponse {
            error: "event_not_found".into(),
            details: None,
        }),
        Ok(_) => {
            publish_event(
                &state.redis_client,
                *event_id,
                &json!({"type": event_types::EVENT_DELETED, "event_id": *event_id}),
            )
            .await;
            publish_global_type(&state.redis_client, event_types::EVENTS_CHANGED).await;
            HttpResponse::Ok().json(StatusResponse {
                status: "deleted".into(),
            })
        }
        Err(_) => HttpResponse::InternalServerError().json(ErrorResponse {
            error: "db_error".into(),
            details: None,
        }),
    }
}

#[utoipa::path(
    post,
    path = "/events/{event_id}/share",
    tag = "events",
    responses(
        (status = 201, description = "Lien de partage généré. C'est un bearer token: tout utilisateur authentifié qui l'obtient peut le réclamer jusqu'à expiration ou consommation.", body = ShareTokenResponse),
        (status = 403, description = "Non autorisé", body = ErrorResponse),
        (status = 404, description = "Événement introuvable", body = ErrorResponse),
        (status = 500, description = "Erreur base de données", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Identifiant de l'événement")
    )
)]
#[post("/events/{event_id}/share")]
pub async fn create_share_link(
    state: web::Data<AppState>,
    req: HttpRequest,
    event_id: web::Path<i64>,
) -> impl Responder {
    let owner_email = match claims_email(&req, state.get_ref()).await {
        Ok(email) => email,
        Err(resp) => return resp,
    };
    let owner_user_id = match fetch_user_id(&state.db, &owner_email).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    if let Err(resp) = ensure_event_owner(&req, state.get_ref(), *event_id).await {
        return resp;
    }
    if let Err(resp) = ensure_event_writable(&state.db, *event_id).await {
        return resp;
    }

    // also ensures event exists
    if let Err(resp) = ensure_event_exists(&state.db, *event_id).await {
        return resp;
    }

    let token = Uuid::new_v4();
    let token_hash = sha256_hex(&token.to_string());
    let expires_at = Utc::now() + Duration::hours(OWNER_SHARE_TOKEN_TTL_HOURS);

    let res = sqlx::query(
        "INSERT INTO event_share_tokens (token_hash, event_id, created_by_user_id, expires_at)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(&token_hash)
    .bind(*event_id)
    .bind(owner_user_id)
    .bind(expires_at)
    .execute(&state.db)
    .await;

    match res {
        Ok(_) => {
            let token_str = token.to_string();
            HttpResponse::Created().json(ShareTokenResponse {
                token: token_str,
                event_id: *event_id,
            })
        }
        Err(_) => server_error(),
    }
}

#[utoipa::path(
    post,
    path = "/share/claim",
    tag = "events",
    request_body = ShareClaimPayload,
    responses(
        (status = 200, description = "Lien consommé et événement accessible", body = ShareClaimResponse),
        (status = 400, description = "Token manquant", body = ErrorResponse),
        (status = 401, description = "Authentification requise", body = ErrorResponse),
        (status = 404, description = "Lien inexistant", body = ErrorResponse),
        (status = 410, description = "Lien déjà consommé", body = ErrorResponse),
        (status = 500, description = "Erreur base de données", body = ErrorResponse)
    )
)]
#[post("/share/claim")]
pub async fn claim_share_link(
    state: web::Data<AppState>,
    req: HttpRequest,
    payload: web::Json<ShareClaimPayload>,
) -> impl Responder {
    let claims = match extract_active_claims_from_auth(&req, &state.db, &state.jwt_secret).await {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let token = payload.token.trim();
    if token.is_empty() {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_token".into(),
            details: Some("Token manquant".into()),
        });
    }

    let parsed_token = match Uuid::parse_str(token) {
        Ok(t) => t,
        Err(_) => {
            return HttpResponse::BadRequest().json(ErrorResponse {
                error: "invalid_token".into(),
                details: Some("Format de token invalide".into()),
            });
        }
    };
    let token_hash = sha256_hex(&parsed_token.to_string());

    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(_) => return server_error(),
    };

    let token_row = match sqlx::query(
        "SELECT event_id,
                used_at,
                expires_at,
                fiestaaa_decrypt_text(target_email_ciphertext) AS target_email
         FROM event_share_tokens
         WHERE token_hash = $1
         FOR UPDATE",
    )
    .bind(&token_hash)
    .fetch_optional(&mut *tx)
    .await
    {
        Ok(Some(row)) => row,
        Ok(None) => {
            return HttpResponse::NotFound().json(ErrorResponse {
                error: "token_not_found".into(),
                details: None,
            });
        }
        Err(_) => return server_error(),
    };

    let used_at: Option<chrono::DateTime<chrono::Utc>> = match token_row.try_get("used_at") {
        Ok(v) => v,
        Err(_) => return server_error(),
    };
    if used_at.is_some() {
        return HttpResponse::Gone().json(ErrorResponse {
            error: "token_used".into(),
            details: Some("Ce lien a déjà été utilisé".into()),
        });
    }
    let expires_at: chrono::DateTime<chrono::Utc> = match token_row.try_get("expires_at") {
        Ok(v) => v,
        Err(_) => return server_error(),
    };
    if expires_at < chrono::Utc::now() {
        return HttpResponse::Gone().json(ErrorResponse {
            error: "token_expired".into(),
            details: Some("Ce lien a expiré".into()),
        });
    }
    let target_email: Option<String> = match token_row.try_get("target_email") {
        Ok(v) => v,
        Err(_) => return server_error(),
    };
    if let Some(expected_email) = target_email
        && !expected_email.eq_ignore_ascii_case(&claims.sub)
    {
        return HttpResponse::Forbidden().json(ErrorResponse {
            error: "token_recipient_mismatch".into(),
            details: Some("Ce lien est réservé à une autre adresse email".into()),
        });
    }
    let event_id: i64 = match token_row.try_get("event_id") {
        Ok(v) => v,
        Err(_) => return server_error(),
    };
    if let Err(resp) = ensure_event_writable(&state.db, event_id).await {
        return resp;
    }

    let event_sql = select_events_sql(
        "FROM events e
         JOIN users owner ON owner.id = e.owner_user_id
         WHERE e.event_id = $1",
    );
    let event = sqlx::query_as::<_, Event>(&event_sql)
    .bind(event_id)
    .fetch_optional(&mut *tx)
    .await;

    let event = match event {
        Ok(Some(e)) => e,
        Ok(None) => {
            return HttpResponse::NotFound().json(ErrorResponse {
                error: "event_not_found".into(),
                details: None,
            });
        }
        Err(_) => return server_error(),
    };

    if let Some(limit) = event.invitation_deadline
        && chrono::Utc::now().date_naive() > limit
    {
        return HttpResponse::Gone().json(ErrorResponse {
            error: "invitation_expired".into(),
            details: Some("La date limite pour répondre est dépassée".into()),
        });
    }

    let user_id = match fetch_user_id(&state.db, &claims.sub).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    // Ensure an invitation exists, but let the user choose their response later.
    let invitation_inserted = match sqlx::query(
        "INSERT INTO invitations (event_id, user_id, status)
         VALUES ($1, $2, 'Waiting')
         ON CONFLICT (event_id, user_id) DO NOTHING",
    )
    .bind(event.event_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    {
        Ok(result) => result.rows_affected() > 0,
        Err(_) => return server_error(),
    };

    let update_res = sqlx::query(
        "UPDATE event_share_tokens
         SET used_at = NOW(), used_by_user_id = $1
         WHERE token_hash = $2",
    )
    .bind(user_id)
    .bind(&token_hash)
    .execute(&mut *tx)
    .await;

    if update_res.is_err() {
        return server_error();
    }

    if tx.commit().await.is_err() {
        return server_error();
    }

    if invitation_inserted {
        publish_event_type(
            &state.redis_client,
            event.event_id,
            event_types::EVENT_INVITATIONS_CHANGED,
        )
        .await;
        publish_global_type(&state.redis_client, event_types::INVITATIONS_CHANGED).await;
    }

    HttpResponse::Ok().json(ShareClaimResponse { event })
}

#[utoipa::path(
    get,
    path = "/events/{event_id}/items",
    tag = "events",
    responses(
        (status = 200, description = "Items configurés pour l'événement", body = [EventItemView]),
        (status = 400, description = "Scope invalide", body = ErrorResponse),
        (status = 401, description = "Authentification requise", body = ErrorResponse),
        (status = 403, description = "Accès réservé aux membres de l'événement", body = ErrorResponse),
        (status = 404, description = "Événement introuvable", body = ErrorResponse),
        (status = 500, description = "Erreur base de données", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Identifiant de l'événement"),
        ("scope" = Option<String>, Query, description = "Filtre: all, mine, to_cover, completed")
    )
)]
#[get("/events/{event_id}/items")]
pub async fn list_event_items(
    state: web::Data<AppState>,
    req: HttpRequest,
    event_id: web::Path<i64>,
    query: web::Query<EventItemsQuery>,
) -> impl Responder {
    let scope = match query.scope.as_deref() {
        Some(raw) => match normalize_event_items_scope(raw) {
            Some(value) => value,
            None => return invalid_items_scope_response(),
        },
        None => EventItemsScope::All,
    };

    if let Err(resp) = ensure_event_exists(&state.db, *event_id).await {
        return resp;
    }
    if let Err(resp) = ensure_event_member(&req, state.get_ref(), *event_id).await {
        return resp;
    }

    let mine_user = if scope == EventItemsScope::Mine {
        let email = match claims_email(&req, state.get_ref()).await {
            Ok(value) => value,
            Err(resp) => return resp,
        };
        let user_id = match fetch_user_id(&state.db, &email).await {
            Ok(value) => value,
            Err(resp) => return resp,
        };
        Some((email, user_id))
    } else {
        None
    };

    let result = sqlx::query_as::<_, EventItemView>(
        "SELECT ei.event_id,
                ei.item_id,
                it.type_id,
                it.type AS type_name,
                i.name_item,
                ei.max_quantity,
                ei.quantity AS reserved_quantity,
                i.unit_label,
                i.item_kind,
                fiestaaa_decrypt_text(cu.email_ciphertext) AS created_by_email,
                cu.handle AS created_by_handle,
                cu.avatar_url AS created_by_avatar_url
         FROM events_items ei
         JOIN items i ON i.item_id = ei.item_id
         JOIN item_types it ON it.type_id = i.type_id
         LEFT JOIN users cu ON cu.id = ei.created_by
         WHERE ei.event_id = $1
         ORDER BY it.type, i.name_item",
    )
    .bind(*event_id)
    .fetch_all(&state.db)
    .await;

    match result {
        Ok(mut items) => {
            if let Some((mine_email, mine_user_id)) = mine_user {
                let contributed_ids = match sqlx::query_scalar::<_, i64>(
                    "SELECT item_id FROM user_items WHERE event_id = $1 AND user_id = $2",
                )
                .bind(*event_id)
                .bind(mine_user_id)
                .fetch_all(&state.db)
                .await
                {
                    Ok(ids) => ids.into_iter().collect::<HashSet<_>>(),
                    Err(_) => return server_error(),
                };

                items.retain(|item| {
                    item.created_by_email
                        .as_ref()
                        .is_some_and(|email| email.eq_ignore_ascii_case(&mine_email))
                        || contributed_ids.contains(&item.item_id)
                });
            }

            match scope {
                EventItemsScope::All | EventItemsScope::Mine => {}
                EventItemsScope::ToCover => {
                    items.retain(|item| {
                        item.item_kind == "need" && item.reserved_quantity < item.max_quantity
                    });
                }
                EventItemsScope::Completed => {
                    items.retain(|item| {
                        item.item_kind == "need" && item.reserved_quantity >= item.max_quantity
                    });
                }
            }

            HttpResponse::Ok().json(items)
        }
        Err(_) => server_error(),
    }
}

#[utoipa::path(
    get,
    path = "/events/{event_id}/items/contributions",
    tag = "events",
    responses(
        (status = 200, description = "Contributions des items de l'événement", body = [ItemContribution]),
        (status = 403, description = "Non autorisé", body = ErrorResponse),
        (status = 404, description = "Événement introuvable", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Identifiant de l'événement")
    )
)]
#[get("/events/{event_id}/items/contributions")]
pub async fn list_event_item_contributions(
    state: web::Data<AppState>,
    req: HttpRequest,
    event_id: web::Path<i64>,
) -> impl Responder {
    if let Err(resp) = ensure_event_member(&req, state.get_ref(), *event_id).await {
        return resp;
    }

    let result = sqlx::query_as::<_, ItemContribution>(
        "SELECT ui.item_id,
                ui.quantity,
                fiestaaa_decrypt_text(u.email_ciphertext) AS email,
                u.handle,
                u.avatar_url
         FROM user_items ui
         JOIN users u ON u.id = ui.user_id
         WHERE ui.event_id = $1",
    )
    .bind(*event_id)
    .fetch_all(&state.db)
    .await;

    match result {
        Ok(list) => HttpResponse::Ok().json(list),
        Err(_) => server_error(),
    }
}

#[utoipa::path(
    post,
    path = "/events/{event_id}/items",
    tag = "events",
    request_body = EventItemAttachPayload,
    responses(
        (status = 200, description = "Item attaché à l'événement", body = EventItemView),
        (status = 400, description = "Payload invalide", body = ErrorResponse),
        (status = 403, description = "Non autorisé", body = ErrorResponse),
        (status = 404, description = "Événement ou item introuvable", body = ErrorResponse),
        (status = 500, description = "Erreur base de données", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Identifiant de l'événement")
    )
)]
#[post("/events/{event_id}/items")]
pub async fn attach_event_item(
    state: web::Data<AppState>,
    req: HttpRequest,
    event_id: web::Path<i64>,
    payload: web::Json<EventItemAttachPayload>,
) -> impl Responder {
    if let Err(resp) = ensure_event_owner(&req, state.get_ref(), *event_id).await {
        return resp;
    }
    if let Err(resp) = ensure_event_writable(&state.db, *event_id).await {
        return resp;
    }

    let creator_email = match claims_email(&req, state.get_ref()).await {
        Ok(email) => email,
        Err(resp) => return resp,
    };

    let payload = payload.into_inner();
    if payload.max_quantity <= 0 {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("max_quantity doit être supérieur à 0".into()),
        });
    }

    if let Err(resp) = ensure_event_exists(&state.db, *event_id).await {
        return resp;
    }
    if let Err(resp) = ensure_item_exists(&state.db, payload.item_id).await {
        return resp;
    }

    let creator_id = match fetch_user_id(&state.db, &creator_email).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let res = sqlx::query(
        "INSERT INTO events_items (event_id, item_id, max_quantity, quantity, created_by)
         VALUES ($1, $2, $3, 0, $4)
         ON CONFLICT (event_id, item_id)
         DO UPDATE SET max_quantity = EXCLUDED.max_quantity
         RETURNING event_id, item_id",
    )
    .bind(*event_id)
    .bind(payload.item_id)
    .bind(payload.max_quantity)
    .bind(creator_id)
    .fetch_one(&state.db)
    .await;

    match res {
        Ok(row) => {
            let ev: i64 = row.get("event_id");
            let item: i64 = row.get("item_id");
            match fetch_event_item_view(&state.db, ev, item).await {
                Ok(view) => {
                    publish_event(
                        &state.redis_client,
                        ev,
                        &json!({
                            "type": event_types::EVENT_ITEMS_CHANGED,
                            "event_id": ev,
                            "item_id": item
                        }),
                    )
                    .await;
                    HttpResponse::Ok().json(view)
                }
                Err(resp) => resp,
            }
        }
        Err(Error::Database(db_err)) if db_err.code().as_deref() == Some("23503") => {
            HttpResponse::BadRequest().json(ErrorResponse {
                error: "unknown_reference".into(),
                details: None,
            })
        }
        Err(_) => server_error(),
    }
}

#[utoipa::path(
    post,
    path = "/events/{event_id}/items/custom",
    tag = "events",
    request_body = EventCustomItemPayload,
    responses(
        (status = 200, description = "Item personnalisé ajouté ou mis à jour", body = EventItemView),
        (status = 400, description = "Payload invalide", body = ErrorResponse),
        (status = 403, description = "Non autorisé", body = ErrorResponse),
        (status = 404, description = "Événement introuvable", body = ErrorResponse),
        (status = 500, description = "Erreur base de données", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Identifiant de l'événement")
    )
)]
#[post("/events/{event_id}/items/custom")]
pub async fn create_custom_event_item(
    state: web::Data<AppState>,
    req: HttpRequest,
    event_id: web::Path<i64>,
    payload: web::Json<EventCustomItemPayload>,
) -> impl Responder {
    if let Err(resp) = ensure_event_member(&req, state.get_ref(), *event_id).await {
        return resp;
    }
    if let Err(resp) = ensure_event_writable(&state.db, *event_id).await {
        return resp;
    }

    let creator_email = match claims_email(&req, state.get_ref()).await {
        Ok(email) => email,
        Err(resp) => return resp,
    };

    let creator_id = match fetch_user_id(&state.db, &creator_email).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let owner_id = match fetch_event_owner_id(&state.db, *event_id).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let is_owner = owner_id == creator_id;

    let payload = payload.into_inner();
    let EventCustomItemPayload {
        name_item,
        max_quantity,
        unit_label,
        item_kind,
    } = payload;

    let name_trimmed = name_item.trim().to_string();
    if name_trimmed.is_empty() || max_quantity <= 0 {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("Nom non vide et quantité > 0 requis".into()),
        });
    }

    let unit_label = unit_label
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "pièce".to_string());

    let item_kind = match item_kind {
        Some(raw) => match normalize_item_kind(&raw) {
            Some(value) => value,
            None => return invalid_item_kind_response(),
        },
        None => {
            if is_owner {
                "need".to_string()
            } else {
                "bring".to_string()
            }
        }
    };

    if item_kind == "need" && !is_owner {
        return HttpResponse::Forbidden().json(ErrorResponse {
            error: "forbidden".into(),
            details: Some("only the creator can add need items".into()),
        });
    }

    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(_) => return server_error(),
    };

    let normalized_name = name_trimmed.to_lowercase();

    let existing_event_item = sqlx::query_scalar::<_, i64>(
        "SELECT ei.item_id
         FROM events_items ei
         JOIN items i ON i.item_id = ei.item_id
         WHERE ei.event_id = $1
           AND lower(i.name_item) = $2
           AND i.item_kind = $3
         FOR UPDATE",
    )
    .bind(*event_id)
    .bind(&normalized_name)
    .bind(item_kind.as_str())
    .fetch_optional(&mut *tx)
    .await
    .map_err(|_| server_error());

    let existing_event_item = match existing_event_item {
        Ok(value) => value,
        Err(resp) => {
            let _ = tx.rollback().await;
            return resp;
        }
    };

    let item_id = if let Some(item_id) = existing_event_item {
        item_id
    } else {
        // Always create a fresh item when not already attached to this event to avoid leaking a previous type.
        let default_type_id = match sqlx::query_scalar::<_, i64>(
            "SELECT type_id FROM item_types WHERE type = 'Autres' LIMIT 1",
        )
        .fetch_optional(&mut *tx)
        .await
        {
            Ok(Some(id)) => id,
            Ok(None) => {
                match sqlx::query_scalar::<_, i64>(
                    "INSERT INTO item_types (type)
                     VALUES ('Autres')
                     ON CONFLICT (type) DO UPDATE SET type = EXCLUDED.type
                     RETURNING type_id",
                )
                .fetch_one(&mut *tx)
                .await
                {
                    Ok(id) => id,
                    Err(_) => {
                        let _ = tx.rollback().await;
                        return server_error();
                    }
                }
            }
            Err(_) => {
                let _ = tx.rollback().await;
                return server_error();
            }
        };

        match sqlx::query_scalar::<_, i64>(
            "INSERT INTO items (type_id, name_item, max_quantity, unit_label, item_kind)
             VALUES ($1, $2, $3, $4, $5)
             RETURNING item_id",
        )
        .bind(default_type_id)
        .bind(name_trimmed.as_str())
        .bind(max_quantity)
        .bind(unit_label.as_str())
        .bind(item_kind.as_str())
        .fetch_one(&mut *tx)
        .await
        {
            Ok(id) => id,
            Err(_) => {
                let _ = tx.rollback().await;
                return server_error();
            }
        }
    };

    let insert_res = sqlx::query(
        "INSERT INTO events_items (event_id, item_id, max_quantity, quantity, created_by)
         VALUES ($1, $2, $3, 0, $4)
         ON CONFLICT (event_id, item_id)
         DO UPDATE SET max_quantity = events_items.max_quantity + EXCLUDED.max_quantity
         RETURNING event_id, item_id",
    )
    .bind(*event_id)
    .bind(item_id)
    .bind(max_quantity)
    .bind(creator_id)
    .fetch_one(&mut *tx)
    .await;

    let (ev_id, item_id) = match insert_res {
        Ok(row) => (row.get::<i64, _>("event_id"), row.get::<i64, _>("item_id")),
        Err(_) => {
            let _ = tx.rollback().await;
            return server_error();
        }
    };

    if (tx.commit().await).is_err() {
        return server_error();
    }

    match fetch_event_item_view(&state.db, ev_id, item_id).await {
        Ok(view) => {
            publish_event(
                &state.redis_client,
                ev_id,
                &json!({
                    "type": event_types::EVENT_ITEMS_CHANGED,
                    "event_id": ev_id,
                    "item_id": item_id
                }),
            )
            .await;
            HttpResponse::Ok().json(view)
        }
        Err(resp) => resp,
    }
}

#[utoipa::path(
    post,
    path = "/events/{event_id}/items/{item_id}/reserve",
    tag = "events",
    request_body = EventItemReservationPayload,
    responses(
        (status = 200, description = "Quantité réservée", body = EventItemView),
        (status = 400, description = "Quantité invalide ou dépassement du maximum", body = ErrorResponse),
        (status = 401, description = "Authentification requise", body = ErrorResponse),
        (status = 404, description = "Événement, item ou utilisateur introuvable", body = ErrorResponse),
        (status = 500, description = "Erreur base de données", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Identifiant de l'événement"),
        ("item_id" = i64, Path, description = "Identifiant de l'item référencé")
    )
)]
#[post("/events/{event_id}/items/{item_id}/reserve")]
pub async fn reserve_event_item(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<(i64, i64)>,
    payload: web::Json<EventItemReservationPayload>,
) -> impl Responder {
    let claims = match extract_active_claims_from_auth(&req, &state.db, &state.jwt_secret).await {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let (event_id, item_id) = path.into_inner();
    if let Err(resp) = ensure_event_member(&req, state.get_ref(), event_id).await {
        return resp;
    }

    let payload = payload.into_inner();
    if payload.quantity < 0 {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("La quantité doit être positive".into()),
        });
    }

    let user_id = match fetch_user_id(&state.db, &claims.sub).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    if let Err(resp) = ensure_event_exists(&state.db, event_id).await {
        return resp;
    }
    if let Err(resp) = ensure_event_writable(&state.db, event_id).await {
        return resp;
    }
    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(_) => return server_error(),
    };

    let event_item = match sqlx::query(
        "SELECT max_quantity, quantity FROM events_items WHERE event_id = $1 AND item_id = $2 FOR UPDATE",
    )
    .bind(event_id)
    .bind(item_id)
    .fetch_optional(&mut *tx)
    .await
    {
        Ok(Some(row)) => (
            row.get::<i32, _>("max_quantity"),
            row.get::<i32, _>("quantity"),
        ),
        Ok(None) => {
            let _ = tx.rollback().await;
            return HttpResponse::NotFound().json(ErrorResponse {
                error: "event_item_not_found".into(),
                details: None,
            });
        }
        Err(_) => {
            let _ = tx.rollback().await;
            return server_error();
        }
    };
    let (max_quantity, current_quantity) = event_item;

    let existing_user_qty = match sqlx::query_scalar::<_, i32>(
        "SELECT quantity FROM user_items WHERE user_id = $1 AND event_id = $2 AND item_id = $3 FOR UPDATE",
    )
    .bind(user_id)
    .bind(event_id)
    .bind(item_id)
    .fetch_optional(&mut *tx)
    .await
    {
        Ok(value) => value.unwrap_or(0),
        Err(_) => {
            let _ = tx.rollback().await;
            return server_error();
        }
    };

    let requested = payload.quantity;
    let new_total = current_quantity - existing_user_qty + requested;
    if new_total < 0 || new_total > max_quantity {
        let _ = tx.rollback().await;
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_quantity".into(),
            details: Some("La quantité dépasse la limite disponible pour cet item".into()),
        });
    }

    let result = if requested == 0 {
        sqlx::query("DELETE FROM user_items WHERE user_id = $1 AND event_id = $2 AND item_id = $3")
            .bind(user_id)
            .bind(event_id)
            .bind(item_id)
            .execute(&mut *tx)
            .await
    } else {
        sqlx::query(
            "INSERT INTO user_items (user_id, event_id, item_id, quantity)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (user_id, event_id, item_id)
             DO UPDATE SET quantity = EXCLUDED.quantity",
        )
        .bind(user_id)
        .bind(event_id)
        .bind(item_id)
        .bind(requested)
        .execute(&mut *tx)
        .await
    };
    if result.is_err() {
        let _ = tx.rollback().await;
        return server_error();
    }

    if (sqlx::query("UPDATE events_items SET quantity = $1 WHERE event_id = $2 AND item_id = $3")
        .bind(new_total)
        .bind(event_id)
        .bind(item_id)
        .execute(&mut *tx)
        .await)
        .is_err()
    {
        let _ = tx.rollback().await;
        return server_error();
    }

    if (tx.commit().await).is_err() {
        return server_error();
    }

    match fetch_event_item_view(&state.db, event_id, item_id).await {
        Ok(view) => {
            publish_event(
                &state.redis_client,
                event_id,
                &json!({
                    "type": event_types::EVENT_ITEMS_CHANGED,
                    "event_id": event_id,
                    "item_id": item_id
                }),
            )
            .await;
            HttpResponse::Ok().json(view)
        }
        Err(resp) => resp,
    }
}

#[utoipa::path(
    delete,
    path = "/events/{event_id}/items/{item_id}",
    tag = "events",
    responses(
        (status = 200, description = "Item supprimé", body = StatusResponse),
        (status = 400, description = "Suppression impossible", body = ErrorResponse),
        (status = 403, description = "Non autorisé", body = ErrorResponse),
        (status = 404, description = "Item introuvable", body = ErrorResponse),
        (status = 500, description = "Erreur base de données", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Identifiant de l'événement"),
        ("item_id" = i64, Path, description = "Identifiant de l'item")
    )
)]
#[delete("/events/{event_id}/items/{item_id}")]
pub async fn delete_event_item(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<(i64, i64)>,
) -> impl Responder {
    let claims = match extract_active_claims_from_auth(&req, &state.db, &state.jwt_secret).await {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let user_id = match fetch_user_id(&state.db, &claims.sub).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let (event_id, item_id) = path.into_inner();

    if let Err(resp) = ensure_event_exists(&state.db, event_id).await {
        return resp;
    }
    if let Err(resp) = ensure_event_writable(&state.db, event_id).await {
        return resp;
    }

    let owner_id = match fetch_event_owner_id(&state.db, event_id).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(_) => return server_error(),
    };

    let record = match sqlx::query(
        "SELECT created_by FROM events_items WHERE event_id = $1 AND item_id = $2 FOR UPDATE",
    )
    .bind(event_id)
    .bind(item_id)
    .fetch_optional(&mut *tx)
    .await
    {
        Ok(Some(row)) => row,
        Ok(None) => {
            let _ = tx.rollback().await;
            return HttpResponse::NotFound().json(ErrorResponse {
                error: "event_item_not_found".into(),
                details: None,
            });
        }
        Err(_) => {
            let _ = tx.rollback().await;
            return server_error();
        }
    };

    let created_by: Option<i64> = record.get("created_by");

    let is_owner = owner_id == user_id;
    let is_creator = created_by.is_some_and(|id| id == user_id);

    if !is_owner && !is_creator {
        let _ = tx.rollback().await;
        return HttpResponse::Forbidden().json(ErrorResponse {
            error: "forbidden".into(),
            details: Some("Seul le créateur ou l'organisateur peut supprimer cet item".into()),
        });
    }

    if (sqlx::query("DELETE FROM user_items WHERE event_id = $1 AND item_id = $2")
        .bind(event_id)
        .bind(item_id)
        .execute(&mut *tx)
        .await)
        .is_err()
    {
        let _ = tx.rollback().await;
        return server_error();
    }

    if (sqlx::query("DELETE FROM events_items WHERE event_id = $1 AND item_id = $2")
        .bind(event_id)
        .bind(item_id)
        .execute(&mut *tx)
        .await)
        .is_err()
    {
        let _ = tx.rollback().await;
        return server_error();
    }

    if (tx.commit().await).is_err() {
        return server_error();
    }

    publish_event(
        &state.redis_client,
        event_id,
        &json!({
            "type": event_types::EVENT_ITEMS_CHANGED,
            "event_id": event_id,
            "item_id": item_id
        }),
    )
    .await;

    HttpResponse::Ok().json(StatusResponse {
        status: "deleted".into(),
    })
}

#[utoipa::path(
    get,
    path = "/events/{event_id}/polls",
    tag = "events",
    responses(
        (status = 200, description = "Sondages associés à l'événement", body = [PollView]),
        (status = 403, description = "Non autorisé", body = ErrorResponse),
        (status = 404, description = "Événement introuvable", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Identifiant de l'événement")
    )
)]
#[get("/events/{event_id}/polls")]
pub async fn list_event_polls(
    state: web::Data<AppState>,
    req: HttpRequest,
    event_id: web::Path<i64>,
) -> impl Responder {
    let claims = match extract_active_claims_from_auth(&req, &state.db, &state.jwt_secret).await {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    if let Err(resp) = ensure_event_member(&req, state.get_ref(), *event_id).await {
        return resp;
    }

    let user_id = match fetch_user_id(&state.db, &claims.sub).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    match fetch_poll_views(state.get_ref(), *event_id, user_id).await {
        Ok(list) => HttpResponse::Ok().json(list),
        Err(resp) => resp,
    }
}

#[utoipa::path(
    post,
    path = "/events/{event_id}/polls",
    tag = "events",
    request_body = EventPollCreatePayload,
    responses(
        (status = 201, description = "Sondage créé", body = PollView),
        (status = 400, description = "Payload invalide", body = ErrorResponse),
        (status = 403, description = "Non autorisé", body = ErrorResponse),
        (status = 404, description = "Événement introuvable", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Identifiant de l'événement")
    )
)]
#[post("/events/{event_id}/polls")]
pub async fn create_event_poll(
    state: web::Data<AppState>,
    req: HttpRequest,
    event_id: web::Path<i64>,
    payload: web::Json<EventPollCreatePayload>,
) -> impl Responder {
    let claims = match extract_active_claims_from_auth(&req, &state.db, &state.jwt_secret).await {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    if let Err(resp) = ensure_event_owner(&req, state.get_ref(), *event_id).await {
        return resp;
    }
    if let Err(resp) = ensure_event_writable(&state.db, *event_id).await {
        return resp;
    }

    let creator_id = match fetch_user_id(&state.db, &claims.sub).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let payload = payload.into_inner();
    let question = payload.question.trim().to_string();
    if question.is_empty() {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("La question du sondage est requise".into()),
        });
    }

    let mut seen = HashSet::new();
    let mut options: Vec<String> = Vec::new();
    for raw in payload.options.into_iter() {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let key = trimmed.to_lowercase();
        if seen.insert(key) {
            options.push(trimmed.to_string());
        }
    }

    if options.len() < 2 {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("Au moins deux options distinctes sont requises".into()),
        });
    }
    if options.len() > 12 {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("Maximum 12 options par sondage".into()),
        });
    }

    if payload.duration_minutes <= 0 {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("Durée d'expiration invalide".into()),
        });
    }

    if let Err(resp) = ensure_event_exists(&state.db, *event_id).await {
        return resp;
    }

    let duration_minutes = payload.duration_minutes.min(60 * 24 * 7);
    let expires_at = Utc::now() + Duration::minutes(duration_minutes);
    let allow_multiple = payload.allow_multiple.unwrap_or(true);

    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(_) => return server_error(),
    };

    let poll_row = sqlx::query(
        "INSERT INTO event_polls (event_id, question, allow_multiple, expires_at, created_by)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING poll_id",
    )
    .bind(*event_id)
    .bind(&question)
    .bind(allow_multiple)
    .bind(expires_at)
    .bind(creator_id)
    .fetch_one(&mut *tx)
    .await;

    let poll_id: i64 = match poll_row {
        Ok(row) => row.get("poll_id"),
        Err(_) => {
            let _ = tx.rollback().await;
            return server_error();
        }
    };

    for (idx, label) in options.iter().enumerate() {
        if (sqlx::query(
            "INSERT INTO event_poll_options (poll_id, label, position) VALUES ($1, $2, $3)",
        )
        .bind(poll_id)
        .bind(label)
        .bind(idx as i32)
        .execute(&mut *tx)
        .await)
            .is_err()
        {
            let _ = tx.rollback().await;
            return server_error();
        }
    }

    if (tx.commit().await).is_err() {
        return server_error();
    }

    let poll = match fetch_poll_views(state.get_ref(), *event_id, creator_id).await {
        Ok(list) => list.into_iter().find(|p| p.poll_id == poll_id),
        Err(resp) => return resp,
    };

    let poll = match poll {
        Some(p) => p,
        None => return server_error(),
    };

    publish_event(
        &state.redis_client,
        *event_id,
        &json!({
            "type": event_types::EVENT_POLLS_CHANGED,
            "event_id": *event_id,
            "poll_id": poll_id
        }),
    )
    .await;

    if state.notifications.is_enabled()
        && let Ok(members) = event_member_user_ids(&state.db, *event_id).await
    {
        let event_name =
            sqlx::query_scalar::<_, String>("SELECT name_event FROM events WHERE event_id = $1")
                .bind(*event_id)
                .fetch_optional(&state.db)
                .await
                .ok()
                .flatten()
                .unwrap_or_else(|| "Événement".into());
        let sender = if claims.handle.trim().is_empty() {
            claims.sub.as_str()
        } else {
            claims.handle.as_str()
        };
        let body = format!("{sender} a lancé un sondage dans {event_name}");
        let dedup = format!("poll_created:{poll_id}");
        notify_users(
            &state.notifications,
            &state.db,
            &members,
            NotificationRequest {
                title: "Nouveau sondage",
                body: body.as_str(),
                data: json!({
                    "type": "poll_created",
                    "event_id": *event_id,
                    "poll_id": poll_id,
                    "question": question
                }),
                dedup_base_key: Some(&dedup),
                dedup_ttl: Some(600),
            },
        )
        .await;
    }

    HttpResponse::Created().json(poll)
}

#[utoipa::path(
    post,
    path = "/events/{event_id}/polls/{poll_id}/vote",
    tag = "events",
    request_body = EventPollVotePayload,
    responses(
        (status = 200, description = "Vote enregistré", body = PollView),
        (status = 400, description = "Payload invalide", body = ErrorResponse),
        (status = 403, description = "Non autorisé", body = ErrorResponse),
        (status = 404, description = "Sondage introuvable", body = ErrorResponse),
        (status = 410, description = "Sondage expiré", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Identifiant de l'événement"),
        ("poll_id" = i64, Path, description = "Identifiant du sondage")
    )
)]
#[post("/events/{event_id}/polls/{poll_id}/vote")]
pub async fn vote_event_poll(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<(i64, i64)>,
    payload: web::Json<EventPollVotePayload>,
) -> impl Responder {
    let claims = match extract_active_claims_from_auth(&req, &state.db, &state.jwt_secret).await {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let user_id = match fetch_user_id(&state.db, &claims.sub).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let (event_id, poll_id) = path.into_inner();

    if let Err(resp) = ensure_event_member(&req, state.get_ref(), event_id).await {
        return resp;
    }
    if let Err(resp) = ensure_event_writable(&state.db, event_id).await {
        return resp;
    }

    let poll_row = sqlx::query(
        "SELECT event_id, allow_multiple, expires_at FROM event_polls WHERE poll_id = $1",
    )
    .bind(poll_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|_| server_error());

    let poll_row = match poll_row {
        Ok(Some(row)) => row,
        Ok(None) => {
            return HttpResponse::NotFound().json(ErrorResponse {
                error: "poll_not_found".into(),
                details: None,
            });
        }
        Err(resp) => return resp,
    };

    let poll_event_id: i64 = match poll_row.try_get("event_id") {
        Ok(id) => id,
        Err(_) => return server_error(),
    };
    if poll_event_id != event_id {
        return HttpResponse::NotFound().json(ErrorResponse {
            error: "poll_not_found".into(),
            details: None,
        });
    }

    let expires_at: chrono::DateTime<chrono::Utc> = match poll_row.try_get("expires_at") {
        Ok(dt) => dt,
        Err(_) => return server_error(),
    };
    if expires_at < Utc::now() {
        return HttpResponse::Gone().json(ErrorResponse {
            error: "poll_expired".into(),
            details: Some("Ce sondage est expiré.".into()),
        });
    }

    let allow_multiple: bool = match poll_row.try_get("allow_multiple") {
        Ok(v) => v,
        Err(_) => return server_error(),
    };

    let mut option_ids: Vec<i64> = payload.option_ids.to_vec();
    option_ids.sort();
    option_ids.dedup();

    if !allow_multiple && option_ids.len() > 1 {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("Ce sondage n'autorise qu'un seul choix.".into()),
        });
    }

    let valid_option_ids =
        sqlx::query_scalar::<_, i64>("SELECT option_id FROM event_poll_options WHERE poll_id = $1")
            .bind(poll_id)
            .fetch_all(&state.db)
            .await;

    let valid_option_ids = match valid_option_ids {
        Ok(list) => list,
        Err(_) => return server_error(),
    };

    let valid_set: HashSet<i64> = valid_option_ids.iter().copied().collect();
    if !option_ids.iter().all(|id| valid_set.contains(id)) {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_option".into(),
            details: Some("Option inconnue pour ce sondage".into()),
        });
    }

    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(_) => return server_error(),
    };

    if (sqlx::query("DELETE FROM event_poll_votes WHERE poll_id = $1 AND user_id = $2")
        .bind(poll_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await)
        .is_err()
    {
        let _ = tx.rollback().await;
        return server_error();
    }

    if !option_ids.is_empty() {
        for opt_id in option_ids {
            if (sqlx::query(
                "INSERT INTO event_poll_votes (poll_id, option_id, user_id) VALUES ($1, $2, $3)",
            )
            .bind(poll_id)
            .bind(opt_id)
            .bind(user_id)
            .execute(&mut *tx)
            .await)
                .is_err()
            {
                let _ = tx.rollback().await;
                return server_error();
            }
        }
    }

    if (tx.commit().await).is_err() {
        return server_error();
    }

    let poll = match fetch_poll_views(state.get_ref(), event_id, user_id).await {
        Ok(list) => list.into_iter().find(|p| p.poll_id == poll_id),
        Err(resp) => return resp,
    };

    let poll = match poll {
        Some(p) => p,
        None => return server_error(),
    };

    publish_event(
        &state.redis_client,
        event_id,
        &json!({
            "type": event_types::EVENT_POLLS_CHANGED,
            "event_id": event_id,
            "poll_id": poll_id
        }),
    )
    .await;

    HttpResponse::Ok().json(poll)
}

#[utoipa::path(
    delete,
    path = "/events/{event_id}/polls/{poll_id}",
    tag = "events",
    responses(
        (status = 200, description = "Sondage supprimé", body = StatusResponse),
        (status = 403, description = "Non autorisé", body = ErrorResponse),
        (status = 404, description = "Sondage introuvable", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Identifiant de l'événement"),
        ("poll_id" = i64, Path, description = "Identifiant du sondage")
    )
)]
#[delete("/events/{event_id}/polls/{poll_id}")]
pub async fn delete_event_poll(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<(i64, i64)>,
) -> impl Responder {
    let claims = match extract_active_claims_from_auth(&req, &state.db, &state.jwt_secret).await {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let (event_id, poll_id) = path.into_inner();

    if let Err(resp) = ensure_event_exists(&state.db, event_id).await {
        return resp;
    }
    if let Err(resp) = ensure_event_writable(&state.db, event_id).await {
        return resp;
    }

    let poll_row = sqlx::query("SELECT event_id, created_by FROM event_polls WHERE poll_id = $1")
        .bind(poll_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|_| server_error());

    let poll_row = match poll_row {
        Ok(Some(row)) => row,
        Ok(None) => {
            return HttpResponse::NotFound().json(ErrorResponse {
                error: "poll_not_found".into(),
                details: None,
            });
        }
        Err(resp) => return resp,
    };

    let poll_event_id: i64 = match poll_row.try_get("event_id") {
        Ok(id) => id,
        Err(_) => return server_error(),
    };
    if poll_event_id != event_id {
        return HttpResponse::NotFound().json(ErrorResponse {
            error: "poll_not_found".into(),
            details: None,
        });
    }

    let creator_id: Option<i64> = poll_row.try_get("created_by").ok();
    let user_id = match fetch_user_id(&state.db, &claims.sub).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let owner_id = match fetch_event_owner_id(&state.db, event_id).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let is_owner = owner_id == user_id;
    let is_creator = creator_id.is_some_and(|id| id == user_id);
    if !is_owner && !is_creator {
        return HttpResponse::Forbidden().json(ErrorResponse {
            error: "forbidden".into(),
            details: Some("Seul l'organisateur ou le créateur peut supprimer ce sondage".into()),
        });
    }

    let res = sqlx::query("DELETE FROM event_polls WHERE poll_id = $1")
        .bind(poll_id)
        .execute(&state.db)
        .await;

    match res {
        Ok(result) if result.rows_affected() == 0 => HttpResponse::NotFound().json(ErrorResponse {
            error: "poll_not_found".into(),
            details: None,
        }),
        Ok(_) => {
            publish_event(
                &state.redis_client,
                event_id,
                &json!({
                    "type": event_types::EVENT_POLLS_CHANGED,
                    "event_id": event_id,
                    "poll_id": poll_id,
                    "deleted": true
                }),
            )
            .await;
            HttpResponse::Ok().json(StatusResponse {
                status: "deleted".into(),
            })
        }
        Err(_) => server_error(),
    }
}

#[utoipa::path(
    get,
    path = "/events/{event_id}/expenses",
    tag = "events",
    responses(
        (status = 200, description = "Dépenses partagées de l'événement", body = [EventExpenseView]),
        (status = 403, description = "Non autorisé", body = ErrorResponse),
        (status = 404, description = "Événement introuvable", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Identifiant de l'événement")
    )
)]
#[get("/events/{event_id}/expenses")]
pub async fn list_event_expenses(
    state: web::Data<AppState>,
    req: HttpRequest,
    event_id: web::Path<i64>,
) -> impl Responder {
    if let Err(resp) = ensure_event_member(&req, state.get_ref(), *event_id).await {
        return resp;
    }

    match fetch_event_expenses(&state.db, *event_id).await {
        Ok(expenses) => HttpResponse::Ok().json(expenses),
        Err(resp) => resp,
    }
}

#[utoipa::path(
    post,
    path = "/events/{event_id}/expenses",
    tag = "events",
    request_body = EventExpensePayload,
    responses(
        (status = 201, description = "Dépense créée", body = EventExpenseView),
        (status = 400, description = "Payload invalide", body = ErrorResponse),
        (status = 403, description = "Non autorisé", body = ErrorResponse),
        (status = 404, description = "Événement introuvable", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Identifiant de l'événement")
    )
)]
#[post("/events/{event_id}/expenses")]
pub async fn create_event_expense(
    state: web::Data<AppState>,
    req: HttpRequest,
    event_id: web::Path<i64>,
    payload: web::Json<EventExpensePayload>,
) -> impl Responder {
    let claims = match extract_active_claims_from_auth(&req, &state.db, &state.jwt_secret).await {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    if let Err(resp) = ensure_event_member(&req, state.get_ref(), *event_id).await {
        return resp;
    }
    if let Err(resp) = ensure_event_writable(&state.db, *event_id).await {
        return resp;
    }

    let requester_id = match fetch_user_id(&state.db, &claims.sub).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let payload = payload.into_inner();
    let title = payload.title.trim().to_string();
    if title.is_empty() {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("Le titre de la dépense est requis".into()),
        });
    }
    if payload.amount_cents <= 0 {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("Le montant doit être supérieur à 0".into()),
        });
    }

    let mut participant_user_ids = payload.participant_user_ids;
    participant_user_ids.sort_unstable();
    participant_user_ids.dedup();
    if participant_user_ids.is_empty() {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("Au moins un participant est requis pour la dépense".into()),
        });
    }

    let members = match fetch_event_member_directory(&state.db, *event_id).await {
        Ok(values) => values,
        Err(resp) => return resp,
    };
    let member_ids: HashSet<i64> = members.keys().copied().collect();
    let paid_by_user_id = payload.paid_by_user_id.unwrap_or(requester_id);
    if !member_ids.contains(&paid_by_user_id) {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payer".into(),
            details: Some("Le payeur doit appartenir à la fiestaaa".into()),
        });
    }
    if !participant_user_ids
        .iter()
        .all(|user_id| member_ids.contains(user_id))
    {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_participants".into(),
            details: Some("Tous les participants doivent appartenir à la fiestaaa".into()),
        });
    }

    let note = payload
        .note
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let expense_date = payload.expense_date.unwrap_or_else(Utc::now);

    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(_) => return server_error(),
    };

    let expense_id = match sqlx::query_scalar::<_, i64>(
        "INSERT INTO event_expenses (event_id, paid_by_user_id, created_by_user_id, title, amount_cents, note, expense_date)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         RETURNING expense_id",
    )
    .bind(*event_id)
    .bind(paid_by_user_id)
    .bind(requester_id)
    .bind(&title)
    .bind(payload.amount_cents)
    .bind(&note)
    .bind(expense_date)
    .fetch_one(&mut *tx)
    .await
    {
        Ok(id) => id,
        Err(_) => {
            let _ = tx.rollback().await;
            return server_error();
        }
    };

    for participant_user_id in participant_user_ids {
        if (sqlx::query(
            "INSERT INTO event_expense_participants (expense_id, user_id, share_weight)
             VALUES ($1, $2, 1)",
        )
        .bind(expense_id)
        .bind(participant_user_id)
        .execute(&mut *tx)
        .await)
            .is_err()
        {
            let _ = tx.rollback().await;
            return server_error();
        }
    }

    if tx.commit().await.is_err() {
        return server_error();
    }

    publish_event(
        &state.redis_client,
        *event_id,
        &json!({
            "type": "event_expenses_changed",
            "event_id": *event_id,
            "expense_id": expense_id,
        }),
    )
    .await;

    match fetch_event_expense_view(&state.db, expense_id).await {
        Ok(expense) => HttpResponse::Created().json(expense),
        Err(resp) => resp,
    }
}

#[utoipa::path(
    delete,
    path = "/events/{event_id}/expenses/{expense_id}",
    tag = "events",
    responses(
        (status = 200, description = "Dépense supprimée", body = StatusResponse),
        (status = 403, description = "Non autorisé", body = ErrorResponse),
        (status = 404, description = "Dépense introuvable", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Identifiant de l'événement"),
        ("expense_id" = i64, Path, description = "Identifiant de la dépense")
    )
)]
#[delete("/events/{event_id}/expenses/{expense_id}")]
pub async fn delete_event_expense(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<(i64, i64)>,
) -> impl Responder {
    let claims = match extract_active_claims_from_auth(&req, &state.db, &state.jwt_secret).await {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let (event_id, expense_id) = path.into_inner();

    if let Err(resp) = ensure_event_member(&req, state.get_ref(), event_id).await {
        return resp;
    }
    if let Err(resp) = ensure_event_writable(&state.db, event_id).await {
        return resp;
    }

    let requester_id = match fetch_user_id(&state.db, &claims.sub).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let owner_id = match fetch_event_owner_id(&state.db, event_id).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let is_owner = owner_id == requester_id;

    let expense_row = match sqlx::query(
        "SELECT paid_by_user_id, created_by_user_id
         FROM event_expenses
         WHERE expense_id = $1 AND event_id = $2",
    )
    .bind(expense_id)
    .bind(event_id)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(row)) => row,
        Ok(None) => {
            return HttpResponse::NotFound().json(ErrorResponse {
                error: "expense_not_found".into(),
                details: None,
            });
        }
        Err(_) => return server_error(),
    };

    let paid_by_user_id: i64 = match expense_row.try_get("paid_by_user_id") {
        Ok(value) => value,
        Err(_) => return server_error(),
    };
    let created_by_user_id: i64 = match expense_row.try_get("created_by_user_id") {
        Ok(value) => value,
        Err(_) => return server_error(),
    };
    if !is_owner && requester_id != paid_by_user_id && requester_id != created_by_user_id {
        return HttpResponse::Forbidden().json(ErrorResponse {
            error: "forbidden".into(),
            details: Some(
                "Seuls le créateur, le payeur ou l'organisateur peuvent supprimer cette dépense"
                    .into(),
            ),
        });
    }

    match sqlx::query("DELETE FROM event_expenses WHERE expense_id = $1 AND event_id = $2")
        .bind(expense_id)
        .bind(event_id)
        .execute(&state.db)
        .await
    {
        Ok(result) if result.rows_affected() == 0 => HttpResponse::NotFound().json(ErrorResponse {
            error: "expense_not_found".into(),
            details: None,
        }),
        Ok(_) => {
            publish_event(
                &state.redis_client,
                event_id,
                &json!({
                    "type": "event_expenses_changed",
                    "event_id": event_id,
                    "expense_id": expense_id,
                }),
            )
            .await;
            HttpResponse::Ok().json(StatusResponse {
                status: "deleted".into(),
            })
        }
        Err(_) => server_error(),
    }
}

#[utoipa::path(
    get,
    path = "/events/{event_id}/expenses/summary",
    tag = "events",
    responses(
        (status = 200, description = "Résumé des dépenses partagées", body = EventExpensesSummaryView),
        (status = 403, description = "Non autorisé", body = ErrorResponse),
        (status = 404, description = "Événement introuvable", body = ErrorResponse)
    ),
    params(
        ("event_id" = i64, Path, description = "Identifiant de l'événement")
    )
)]
#[get("/events/{event_id}/expenses/summary")]
pub async fn get_event_expenses_summary(
    state: web::Data<AppState>,
    req: HttpRequest,
    event_id: web::Path<i64>,
) -> impl Responder {
    if let Err(resp) = ensure_event_member(&req, state.get_ref(), *event_id).await {
        return resp;
    }

    match build_event_expenses_summary(&state.db, *event_id).await {
        Ok(summary) => HttpResponse::Ok().json(summary),
        Err(resp) => resp,
    }
}

async fn fetch_poll_views(
    state: &AppState,
    event_id: i64,
    user_id: i64,
) -> Result<Vec<PollView>, HttpResponse> {
    let poll_rows = sqlx::query(
        "SELECT p.poll_id,
                p.question,
                p.allow_multiple,
                p.expires_at,
                p.created_at,
                fiestaaa_decrypt_text(u.email_ciphertext) AS created_by_email
         FROM event_polls p
         LEFT JOIN users u ON u.id = p.created_by
         WHERE p.event_id = $1
         ORDER BY p.created_at DESC",
    )
    .bind(event_id)
    .fetch_all(&state.db)
    .await
    .map_err(|_| server_error())?;

    if poll_rows.is_empty() {
        return Ok(Vec::new());
    }

    let poll_ids: Vec<i64> = poll_rows
        .iter()
        .filter_map(|row| row.try_get("poll_id").ok())
        .collect();
    let option_rows = sqlx::query(
        "SELECT option_id, poll_id, label, position
         FROM event_poll_options
         WHERE poll_id = ANY($1)
         ORDER BY position, option_id",
    )
    .bind(&poll_ids)
    .fetch_all(&state.db)
    .await
    .map_err(|_| server_error())?;

    let vote_rows = sqlx::query(
        "SELECT v.poll_id,
                v.option_id,
                v.user_id,
                fiestaaa_decrypt_text(u.email_ciphertext) AS email,
                u.handle,
                u.avatar_url
         FROM event_poll_votes v
         JOIN users u ON u.id = v.user_id
         WHERE v.poll_id = ANY($1)",
    )
    .bind(&poll_ids)
    .fetch_all(&state.db)
    .await
    .map_err(|_| server_error())?;

    let mut polls: Vec<PollView> = poll_rows
        .into_iter()
        .filter_map(|row| {
            let expires_at: chrono::DateTime<chrono::Utc> = row.try_get("expires_at").ok()?;
            let created_at: chrono::DateTime<chrono::Utc> = row.try_get("created_at").ok()?;
            let poll_id: i64 = row.try_get("poll_id").ok()?;
            let question: String = row.try_get("question").ok()?;
            let allow_multiple: bool = row.try_get("allow_multiple").ok()?;
            let created_by_email: Option<String> = row.try_get("created_by_email").ok();
            Some(PollView {
                poll_id,
                event_id,
                question,
                allow_multiple,
                expires_at,
                created_at,
                created_by_email,
                options: Vec::new(),
                my_votes: Vec::new(),
                total_votes: 0,
                has_expired: expires_at < Utc::now(),
            })
        })
        .collect();

    let mut options_by_poll: HashMap<i64, Vec<(PollOptionView, i32)>> = HashMap::new();
    for row in option_rows {
        let poll_id: i64 = row.get("poll_id");
        let option_id: i64 = row.get("option_id");
        let label: String = row.get("label");
        let position: i32 = row.get("position");
        options_by_poll.entry(poll_id).or_default().push((
            PollOptionView {
                option_id,
                label,
                vote_count: 0,
                voters: Vec::new(),
            },
            position,
        ));
    }

    for vote_row in vote_rows {
        let poll_id: i64 = vote_row.get("poll_id");
        let option_id: i64 = vote_row.get("option_id");
        let voter_id: i64 = vote_row.get("user_id");
        if let Some(poll) = polls.iter_mut().find(|p| p.poll_id == poll_id) {
            poll.total_votes += 1;
            if voter_id == user_id {
                poll.my_votes.push(option_id);
            }
        }
        if let Some(options) = options_by_poll.get_mut(&poll_id)
            && let Some((opt, _)) = options
                .iter_mut()
                .find(|(opt, _)| opt.option_id == option_id)
        {
            opt.vote_count += 1;
            opt.voters.push(PollOptionVoter {
                email: vote_row.get("email"),
                handle: vote_row.get("handle"),
                avatar_url: vote_row.get("avatar_url"),
            });
        }
    }

    for poll in &mut polls {
        if let Some(mut opts) = options_by_poll.remove(&poll.poll_id) {
            opts.sort_by_key(|(_, pos)| *pos);
            poll.options = opts.into_iter().map(|(opt, _)| opt).collect();
        }
        poll.my_votes.sort_unstable();
        poll.my_votes.dedup();
    }

    Ok(polls)
}

async fn fetch_event_member_directory(
    db: &PgPool,
    event_id: i64,
) -> Result<HashMap<i64, (Option<String>, Option<String>)>, HttpResponse> {
    let rows = sqlx::query(
        "SELECT DISTINCT u.id, u.handle, u.avatar_url
         FROM users u
         WHERE u.id = (SELECT owner_user_id FROM events WHERE event_id = $1)
         UNION
         SELECT DISTINCT u.id, u.handle, u.avatar_url
         FROM invitations i
         JOIN users u ON u.id = i.user_id
         WHERE i.event_id = $1
           AND i.status = 'Accepted'",
    )
    .bind(event_id)
    .fetch_all(db)
    .await
    .map_err(|_| server_error())?;

    let mut members = HashMap::new();
    for row in rows {
        let user_id: i64 = row.get("id");
        let handle: Option<String> = row.get("handle");
        let avatar_url: Option<String> = row.get("avatar_url");
        members.insert(user_id, (handle, avatar_url));
    }
    Ok(members)
}

async fn fetch_event_expense_view(
    db: &PgPool,
    expense_id: i64,
) -> Result<EventExpenseView, HttpResponse> {
    let expenses = fetch_event_expenses_by_ids(db, &[expense_id]).await?;
    expenses.into_iter().next().ok_or_else(|| {
        HttpResponse::NotFound().json(ErrorResponse {
            error: "expense_not_found".into(),
            details: None,
        })
    })
}

async fn fetch_event_expenses(
    db: &PgPool,
    event_id: i64,
) -> Result<Vec<EventExpenseView>, HttpResponse> {
    let expense_ids = sqlx::query_scalar::<_, i64>(
        "SELECT expense_id
         FROM event_expenses
         WHERE event_id = $1
         ORDER BY expense_date DESC, expense_id DESC",
    )
    .bind(event_id)
    .fetch_all(db)
    .await
    .map_err(|_| server_error())?;

    fetch_event_expenses_by_ids(db, &expense_ids).await
}

async fn fetch_event_expenses_by_ids(
    db: &PgPool,
    expense_ids: &[i64],
) -> Result<Vec<EventExpenseView>, HttpResponse> {
    if expense_ids.is_empty() {
        return Ok(Vec::new());
    }

    let expense_rows = sqlx::query(
        "SELECT ee.expense_id,
                ee.event_id,
                ee.paid_by_user_id,
                u.handle AS paid_by_handle,
                u.avatar_url AS paid_by_avatar_url,
                ee.title,
                ee.amount_cents,
                ee.note,
                ee.expense_date,
                ee.created_at
         FROM event_expenses ee
         JOIN users u ON u.id = ee.paid_by_user_id
         WHERE ee.expense_id = ANY($1)
         ORDER BY ee.expense_date DESC, ee.expense_id DESC",
    )
    .bind(expense_ids)
    .fetch_all(db)
    .await
    .map_err(|_| server_error())?;

    let participant_rows = sqlx::query(
        "SELECT ep.expense_id, ep.user_id, u.handle, u.avatar_url
         FROM event_expense_participants ep
         JOIN users u ON u.id = ep.user_id
         WHERE ep.expense_id = ANY($1)
         ORDER BY ep.expense_id, lower(u.handle), ep.user_id",
    )
    .bind(expense_ids)
    .fetch_all(db)
    .await
    .map_err(|_| server_error())?;

    let mut participants_by_expense: HashMap<i64, Vec<EventExpenseParticipantView>> =
        HashMap::new();
    for row in participant_rows {
        let expense_id: i64 = row.get("expense_id");
        participants_by_expense
            .entry(expense_id)
            .or_default()
            .push(EventExpenseParticipantView {
                user_id: row.get("user_id"),
                handle: row.get("handle"),
                avatar_url: row.get("avatar_url"),
            });
    }

    let mut expenses = Vec::new();
    for row in expense_rows {
        let expense_id: i64 = row.get("expense_id");
        expenses.push(EventExpenseView {
            expense_id,
            event_id: row.get("event_id"),
            paid_by_user_id: row.get("paid_by_user_id"),
            paid_by_handle: row.get("paid_by_handle"),
            paid_by_avatar_url: row.get("paid_by_avatar_url"),
            title: row.get("title"),
            amount_cents: row.get("amount_cents"),
            note: row.get("note"),
            expense_date: row.get("expense_date"),
            created_at: row.get("created_at"),
            participants: participants_by_expense
                .remove(&expense_id)
                .unwrap_or_default(),
        });
    }

    Ok(expenses)
}

async fn build_event_expenses_summary(
    db: &PgPool,
    event_id: i64,
) -> Result<EventExpensesSummaryView, HttpResponse> {
    let _ = fetch_event_timing(db, event_id).await?;
    let members = fetch_event_member_directory(db, event_id).await?;
    let expenses = fetch_event_expenses(db, event_id).await?;

    let mut balances_by_user: HashMap<i64, (i64, i64)> = members
        .keys()
        .copied()
        .map(|user_id| (user_id, (0, 0)))
        .collect();
    let mut total_expenses_cents = 0_i64;

    for expense in expenses {
        total_expenses_cents += expense.amount_cents;
        balances_by_user
            .entry(expense.paid_by_user_id)
            .or_insert((0, 0))
            .0 += expense.amount_cents;

        let mut participant_ids: Vec<i64> = expense
            .participants
            .into_iter()
            .map(|item| item.user_id)
            .collect();
        participant_ids.sort_unstable();
        participant_ids.dedup();
        if participant_ids.is_empty() {
            continue;
        }

        let participant_count = participant_ids.len() as i64;
        let share_cents = expense.amount_cents / participant_count;
        let remainder = expense.amount_cents % participant_count;

        for (index, participant_id) in participant_ids.into_iter().enumerate() {
            let extra_cent = if (index as i64) < remainder { 1 } else { 0 };
            balances_by_user.entry(participant_id).or_insert((0, 0)).1 += share_cents + extra_cent;
        }
    }

    let mut balances: Vec<EventExpenseBalanceView> = balances_by_user
        .into_iter()
        .map(|(user_id, (paid_cents, owed_cents))| {
            let (handle, avatar_url) = members.get(&user_id).cloned().unwrap_or((None, None));
            EventExpenseBalanceView {
                user_id,
                handle,
                avatar_url,
                paid_cents,
                owed_cents,
                balance_cents: paid_cents - owed_cents,
            }
        })
        .collect();

    balances.sort_by(|a, b| {
        b.balance_cents
            .cmp(&a.balance_cents)
            .then_with(|| a.handle.cmp(&b.handle))
            .then_with(|| a.user_id.cmp(&b.user_id))
    });

    let mut creditors: Vec<(usize, i64)> = balances
        .iter()
        .enumerate()
        .filter_map(|(index, item)| (item.balance_cents > 0).then_some((index, item.balance_cents)))
        .collect();
    let mut debtors: Vec<(usize, i64)> = balances
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            (item.balance_cents < 0).then_some((index, -item.balance_cents))
        })
        .collect();

    let mut settlements = Vec::new();
    let mut creditor_index = 0_usize;
    let mut debtor_index = 0_usize;
    while creditor_index < creditors.len() && debtor_index < debtors.len() {
        let (creditor_balance_index, creditor_amount) = creditors[creditor_index];
        let (debtor_balance_index, debtor_amount) = debtors[debtor_index];
        let transfer_amount = creditor_amount.min(debtor_amount);

        settlements.push(EventExpenseSettlementView {
            from_user_id: balances[debtor_balance_index].user_id,
            from_handle: balances[debtor_balance_index].handle.clone(),
            to_user_id: balances[creditor_balance_index].user_id,
            to_handle: balances[creditor_balance_index].handle.clone(),
            amount_cents: transfer_amount,
        });

        creditors[creditor_index].1 -= transfer_amount;
        debtors[debtor_index].1 -= transfer_amount;
        if creditors[creditor_index].1 == 0 {
            creditor_index += 1;
        }
        if debtors[debtor_index].1 == 0 {
            debtor_index += 1;
        }
    }

    Ok(EventExpensesSummaryView {
        currency: "EUR".into(),
        total_expenses_cents,
        balances,
        settlements,
    })
}

fn clean_payment_value(value: Option<String>) -> Option<String> {
    value.and_then(|v| {
        let trimmed = v.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn ensure_safe_absolute_http_url(
    value: &str,
    error: &str,
    details: &str,
) -> Result<(), HttpResponse> {
    let url = reqwest::Url::parse(value).map_err(|_| {
        HttpResponse::BadRequest().json(ErrorResponse {
            error: error.into(),
            details: Some(details.into()),
        })
    })?;

    let scheme = url.scheme().to_ascii_lowercase();
    let Some(host) = url.host_str() else {
        return Err(HttpResponse::BadRequest().json(ErrorResponse {
            error: error.into(),
            details: Some(details.into()),
        }));
    };

    if scheme != "http" && scheme != "https" {
        return Err(HttpResponse::BadRequest().json(ErrorResponse {
            error: error.into(),
            details: Some(details.into()),
        }));
    }

    let host_lower = host.to_ascii_lowercase();
    let host_normalized = host_lower.trim_end_matches('.');
    if host_normalized == "localhost" || host_normalized.ends_with(".localhost") {
        return Err(HttpResponse::BadRequest().json(ErrorResponse {
            error: error.into(),
            details: Some("Le lien doit cibler une adresse publique".into()),
        }));
    }

    if let Ok(ip) = host.parse::<IpAddr>() {
        let is_private_or_local = match ip {
            IpAddr::V4(addr) => {
                let octets = addr.octets();
                addr.is_private()
                    || addr.is_loopback()
                    || addr.is_link_local()
                    || addr.is_unspecified()
                    || (octets[0] == 100 && (64..=127).contains(&octets[1]))
            }
            IpAddr::V6(addr) => {
                let octets = addr.octets();
                let segments = addr.segments();
                let is_ipv4_mapped = octets[..10].iter().all(|byte| *byte == 0)
                    && octets[10] == 0xff
                    && octets[11] == 0xff;
                let mapped_ipv4_is_private = if is_ipv4_mapped {
                    let first = octets[12];
                    let second = octets[13];
                    first == 0
                        || first == 10
                        || first == 127
                        || (first == 169 && second == 254)
                        || (first == 172 && (16..=31).contains(&second))
                        || (first == 192 && second == 168)
                        || (first == 100 && (64..=127).contains(&second))
                } else {
                    false
                };
                addr.is_loopback()
                    || addr.is_unspecified()
                    || mapped_ipv4_is_private
                    || (segments[0] & 0xfe00) == 0xfc00
                    || (segments[0] & 0xffc0) == 0xfe80
            }
        };

        if is_private_or_local {
            return Err(HttpResponse::BadRequest().json(ErrorResponse {
                error: error.into(),
                details: Some("Le lien doit cibler une adresse publique".into()),
            }));
        }
    }

    Ok(())
}

async fn fetch_provider_validation(
    db: &PgPool,
    provider_id: i32,
) -> Result<(String, Regex), HttpResponse> {
    let provider = sqlx::query_as::<_, (String, String)>(
        "SELECT provider_name, validation_regex FROM payment_providers WHERE provider_id = $1",
    )
    .bind(provider_id)
    .fetch_optional(db)
    .await
    .map_err(|_| server_error())?;

    match provider {
        Some((name, pattern)) => {
            let regex = Regex::new(&pattern).map_err(|_| server_error())?;
            Ok((name, regex))
        }
        None => Err(HttpResponse::BadRequest().json(ErrorResponse {
            error: "unknown_payment_provider".into(),
            details: None,
        })),
    }
}

async fn normalize_payment_info(
    db: &PgPool,
    provider_id: Option<i32>,
    payment_link: Option<String>,
) -> Result<(Option<i32>, Option<String>), HttpResponse> {
    match (provider_id, clean_payment_value(payment_link)) {
        (None, None) => Ok((None, None)),
        (Some(id), Some(link)) => {
            let (name, regex) = fetch_provider_validation(db, id).await?;
            if !regex.is_match(&link) {
                return Err(HttpResponse::BadRequest().json(ErrorResponse {
                    error: "invalid_payment_link".into(),
                    details: Some(format!(
                        "Le lien ne correspond pas au format attendu pour {name}"
                    )),
                }));
            }
            ensure_safe_absolute_http_url(
                &link,
                "invalid_payment_link",
                "Le lien de paiement doit utiliser http ou https",
            )?;
            Ok((Some(id), Some(link)))
        }
        (Some(id), None) => Ok((Some(id), None)),
        (None, Some(_)) => Err(HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_payload".into(),
            details: Some("Choisissez d'abord un provider pour associer un lien".into()),
        })),
    }
}

async fn ensure_event_exists(db: &PgPool, event_id: i64) -> Result<(), HttpResponse> {
    let exists =
        sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM events WHERE event_id = $1)")
            .bind(event_id)
            .fetch_one(db)
            .await
            .map_err(|_| server_error())?;

    if exists {
        Ok(())
    } else {
        Err(HttpResponse::NotFound().json(ErrorResponse {
            error: "event_not_found".into(),
            details: None,
        }))
    }
}

async fn fetch_event_payment_info(
    db: &PgPool,
    event_id: i64,
) -> Result<
    (
        Option<i32>,
        Option<String>,
        bool,
        NaiveDate,
        chrono::NaiveTime,
        Option<NaiveDate>,
        Option<chrono::NaiveTime>,
        Option<NaiveDate>,
        Option<String>,
        Option<String>,
        Vec<String>,
    ),
    HttpResponse,
> {
    let row = sqlx::query_as::<
        _,
        (
            Option<i32>,
            Option<String>,
            bool,
            NaiveDate,
            chrono::NaiveTime,
            Option<NaiveDate>,
            Option<chrono::NaiveTime>,
            Option<NaiveDate>,
            Option<String>,
            Option<String>,
            Vec<String>,
        ),
    >(
        "SELECT payment_provider_id,
                fiestaaa_decrypt_text(payment_identifier_ciphertext) AS payment_identifier,
                payment_per_person,
                date_event,
                start_time,
                end_date,
                end_time,
                invitation_deadline,
                playlist_provider,
                playlist_url,
                enabled_features
         FROM events
         WHERE event_id = $1",
    )
    .bind(event_id)
    .fetch_optional(db)
    .await
    .map_err(|_| server_error())?;

    match row {
        Some(info) => Ok(info),
        None => Err(HttpResponse::NotFound().json(ErrorResponse {
            error: "event_not_found".into(),
            details: None,
        })),
    }
}

async fn ensure_item_exists(db: &PgPool, item_id: i64) -> Result<(), HttpResponse> {
    let exists =
        sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM items WHERE item_id = $1)")
            .bind(item_id)
            .fetch_one(db)
            .await
            .map_err(|_| server_error())?;

    if exists {
        Ok(())
    } else {
        Err(HttpResponse::NotFound().json(ErrorResponse {
            error: "item_not_found".into(),
            details: None,
        }))
    }
}

async fn fetch_event_item_view(
    db: &PgPool,
    event_id: i64,
    item_id: i64,
) -> Result<EventItemView, HttpResponse> {
    let record = sqlx::query_as::<_, EventItemView>(
        "SELECT ei.event_id,
                ei.item_id,
                it.type_id,
                it.type AS type_name,
                i.name_item,
                ei.max_quantity,
                ei.quantity AS reserved_quantity,
                i.unit_label,
                i.item_kind,
                fiestaaa_decrypt_text(cu.email_ciphertext) AS created_by_email,
                cu.handle AS created_by_handle,
                cu.avatar_url AS created_by_avatar_url
         FROM events_items ei
         JOIN items i ON i.item_id = ei.item_id
         JOIN item_types it ON it.type_id = i.type_id
         LEFT JOIN users cu ON cu.id = ei.created_by
         WHERE ei.event_id = $1 AND ei.item_id = $2",
    )
    .bind(event_id)
    .bind(item_id)
    .fetch_optional(db)
    .await
    .map_err(|_| server_error())?;

    match record {
        Some(view) => Ok(view),
        None => Err(HttpResponse::NotFound().json(ErrorResponse {
            error: "event_item_not_found".into(),
            details: None,
        })),
    }
}

async fn fetch_user_id(db: &PgPool, email: &str) -> Result<i64, HttpResponse> {
    let record =
        sqlx::query_scalar::<_, i64>(
            "SELECT id FROM users WHERE fiestaaa_email_matches(email_lookup_hash, $1)",
        )
            .bind(email)
            .fetch_optional(db)
            .await
            .map_err(|_| server_error())?;

    match record {
        Some(id) => Ok(id),
        None => Err(HttpResponse::NotFound().json(ErrorResponse {
            error: "user_not_found".into(),
            details: None,
        })),
    }
}

fn server_error() -> HttpResponse {
    HttpResponse::InternalServerError().json(ErrorResponse {
        error: "db_error".into(),
        details: None,
    })
}
