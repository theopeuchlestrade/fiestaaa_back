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
use actix_web::{HttpRequest, HttpResponse};
use chrono::NaiveDate;
use log::warn;
use regex::Regex;
use serde::Deserialize;
use serde_json::json;
use sqlx::{PgPool, Row};
use std::collections::HashSet;
use std::net::IpAddr;

pub(crate) mod address;
pub(crate) mod core;
pub(crate) mod expenses;
pub(crate) mod items;
pub(crate) mod polls;
pub(crate) mod share_links;

pub use address::search_address;
pub use core::{create_event, delete_event, get_event, list_events, replace_event, update_event};
pub use expenses::{
    create_event_expense, delete_event_expense, get_event_expenses_summary, list_event_expenses,
};
pub use items::{
    attach_event_item, create_custom_event_item, delete_event_item, list_event_item_contributions,
    list_event_items, reserve_event_item,
};
pub use polls::{create_event_poll, delete_event_poll, list_event_polls, vote_event_poll};
pub use share_links::{claim_share_link, create_share_link};

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

fn sync_optional_features(
    mut features: Vec<String>,
    payment_provider_id: Option<i32>,
    payment_identifier: Option<&str>,
    playlist_provider: Option<&str>,
    playlist_url: Option<&str>,
) -> Vec<String> {
    if has_playlist_metadata(playlist_provider, playlist_url) {
        append_feature(&mut features, FEATURE_PLAYLIST);
    } else {
        features.retain(|value| value != FEATURE_PLAYLIST);
    }

    if has_payment_metadata(payment_provider_id, payment_identifier) {
        append_feature(&mut features, FEATURE_PAYMENT);
    } else {
        features.retain(|value| value != FEATURE_PAYMENT);
    }

    features
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

    let host_for_ip = host_normalized
        .trim_start_matches('[')
        .trim_end_matches(']');
    if let Ok(ip) = host_for_ip.parse::<IpAddr>() {
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
                let segments = addr.segments();
                let mapped_ipv4_is_private = addr.to_ipv4_mapped().is_some_and(|mapped| {
                    let octets = mapped.octets();
                    mapped.is_private()
                        || mapped.is_loopback()
                        || mapped.is_link_local()
                        || mapped.is_unspecified()
                        || (octets[0] == 100 && (64..=127).contains(&octets[1]))
                });
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
    let record = sqlx::query_scalar::<_, i64>(
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
