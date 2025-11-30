use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use utoipa::ToSchema;

#[derive(Serialize, Deserialize, ToSchema)]
pub struct LoginPayload {
    /// Email ou identifiant (handle)
    #[serde(alias = "email", alias = "handle")]
    pub identifier: String,
    pub password: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct RegisterPayload {
    pub email: String,
    pub password: String,
    pub handle: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct Claims {
    pub sub: String,
    pub exp: usize,
    pub handle: String,
}

#[derive(Serialize, ToSchema)]
pub struct TokenResponse {
    pub token: String,
    pub email: String,
    pub handle: String,
}

#[derive(Serialize, ToSchema)]
pub struct StatusResponse {
    pub status: String,
}

#[derive(Serialize, ToSchema)]
pub struct ErrorResponse {
    pub error: String,
    pub details: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct MeResponse {
    pub email: String,
    pub exp: usize,
    pub handle: String,
    pub avatar_url: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct HealthResponse {
    pub status: String,
    pub db: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema, FromRow)]
pub struct Item {
    pub item_id: i64,
    pub type_id: i64,
    pub name_item: String,
    pub max_quantity: i32,
    pub unit_label: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ItemPayload {
    pub type_id: i64,
    pub name_item: String,
    pub max_quantity: i32,
    pub unit_label: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ItemPatchPayload {
    pub type_id: Option<i64>,
    pub name_item: Option<String>,
    pub max_quantity: Option<i32>,
    pub unit_label: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema, FromRow)]
pub struct Event {
    pub event_id: i64,
    pub name_event: String,
    pub description: String,
    pub date_event: chrono::NaiveDate,
    pub start_time: chrono::NaiveTime,
    pub invitation_deadline: Option<chrono::NaiveDate>,
    pub address: String,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub payment_provider_id: Option<i32>,
    pub payment_identifier: Option<String>,
    pub payment_requested_amount: Option<f64>,
    #[serde(default)]
    pub payment_per_person: bool,
    pub owner_email: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct EventPayload {
    pub name_event: String,
    pub description: String,
    pub date_event: chrono::NaiveDate,
    pub start_time: chrono::NaiveTime,
    pub invitation_deadline: Option<chrono::NaiveDate>,
    pub address: String,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub payment_provider_id: Option<i32>,
    pub payment_identifier: Option<String>,
    pub payment_requested_amount: Option<f64>,
    pub payment_per_person: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct EventPatchPayload {
    pub name_event: Option<String>,
    pub description: Option<String>,
    pub date_event: Option<chrono::NaiveDate>,
    pub start_time: Option<chrono::NaiveTime>,
    pub invitation_deadline: Option<chrono::NaiveDate>,
    pub address: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub payment_provider_id: Option<i32>,
    pub payment_identifier: Option<String>,
    pub payment_requested_amount: Option<f64>,
    pub payment_per_person: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema, FromRow)]
pub struct EventItemView {
    pub event_id: i64,
    pub item_id: i64,
    pub type_id: i64,
    pub type_name: String,
    pub name_item: String,
    pub max_quantity: i32,
    pub reserved_quantity: i32,
    pub unit_label: String,
    pub created_by_email: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct EventItemAttachPayload {
    pub item_id: i64,
    pub max_quantity: i32,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct EventItemReservationPayload {
    pub quantity: i32,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct EventCustomItemPayload {
    pub name_item: String,
    pub max_quantity: i32,
    pub unit_label: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema, FromRow)]
pub struct Invitation {
    pub event_id: i64,
    pub email: String,
    pub handle: Option<String>,
    pub avatar_url: Option<String>,
    pub status: String,
    pub date_invi: chrono::DateTime<chrono::Utc>,
    pub event_name: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema, FromRow)]
pub struct InvitationSuggestion {
    pub email: String,
    pub handle: String,
    pub last_invited_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct InvitationPayload {
    #[serde(alias = "email", alias = "handle")]
    pub identifier: String,
    pub status: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct InvitationPatchPayload {
    pub status: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema, FromRow)]
pub struct Friend {
    pub email: String,
    pub handle: String,
    pub avatar_url: Option<String>,
    pub since: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema, FromRow)]
pub struct FriendSearchResult {
    pub email: String,
    pub handle: String,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct FriendRequestPayload {
    #[serde(alias = "email", alias = "handle")]
    pub identifier: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct FriendRequestActionPayload {
    pub status: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema, FromRow)]
pub struct FriendRequest {
    pub id: i64,
    pub sender_email: String,
    pub sender_handle: String,
    pub sender_avatar_url: Option<String>,
    pub receiver_email: String,
    pub receiver_handle: String,
    pub receiver_avatar_url: Option<String>,
    pub status: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema, FromRow)]
pub struct ItemContribution {
    pub item_id: i64,
    pub quantity: i32,
    pub email: String,
    pub handle: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema, FromRow)]
pub struct PaymentProvider {
    pub provider_id: i32,
    pub provider_name: String,
    pub url_template: String,
    pub validation_regex: String,
    pub is_active: bool,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PaymentProviderPayload {
    pub provider_name: String,
    pub url_template: String,
    pub validation_regex: Option<String>,
    pub is_active: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PaymentProviderPatchPayload {
    pub provider_name: Option<String>,
    pub url_template: Option<String>,
    pub validation_regex: Option<String>,
    pub is_active: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ShareTokenResponse {
    pub token: String,
    pub event_id: i64,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ShareClaimPayload {
    pub token: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ShareClaimResponse {
    pub event: Event,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AddressSuggestion {
    pub label: String,
    pub latitude: f64,
    pub longitude: f64,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct HandleAvailabilityResponse {
    pub available: bool,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct HandleUpdatePayload {
    pub handle: String,
}
