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

#[derive(Deserialize, ToSchema)]
pub struct OAuthPayload {
    #[serde(rename = "idToken", alias = "id_token")]
    pub id_token: Option<String>,
    #[serde(rename = "accessToken", alias = "access_token")]
    pub access_token: Option<String>,
    pub email: Option<String>,
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AppleClaims {
    pub sub: String,
    pub email: Option<String>,
    pub email_verified: Option<serde_json::Value>,
    pub exp: usize,
    pub iss: String,
    pub aud: String,
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
    pub item_kind: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ItemPayload {
    pub type_id: i64,
    pub name_item: String,
    pub max_quantity: i32,
    pub unit_label: String,
    pub item_kind: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ItemPatchPayload {
    pub type_id: Option<i64>,
    pub name_item: Option<String>,
    pub max_quantity: Option<i32>,
    pub unit_label: Option<String>,
    pub item_kind: Option<String>,
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
    pub playlist_url: Option<String>,
    pub playlist_provider: Option<String>,
    #[serde(default)]
    pub enabled_features: Vec<String>,
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
    pub playlist_url: Option<String>,
    pub playlist_provider: Option<String>,
    pub enabled_features: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct EventPatchPayload {
    pub name_event: Option<String>,
    pub description: Option<String>,
    pub date_event: Option<chrono::NaiveDate>,
    pub start_time: Option<chrono::NaiveTime>,
    pub invitation_deadline: Option<Option<chrono::NaiveDate>>,
    pub address: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub payment_provider_id: Option<i32>,
    pub payment_identifier: Option<String>,
    pub payment_requested_amount: Option<f64>,
    pub payment_per_person: Option<bool>,
    pub playlist_url: Option<Option<String>>,
    pub playlist_provider: Option<Option<String>>,
    pub enabled_features: Option<Vec<String>>,
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
    pub item_kind: String,
    pub created_by_email: Option<String>,
    pub created_by_handle: Option<String>,
    pub created_by_avatar_url: Option<String>,
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
    pub item_kind: Option<String>,
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

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct EventPollCreatePayload {
    pub question: String,
    pub options: Vec<String>,
    pub duration_minutes: i64,
    pub allow_multiple: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct EventPollVotePayload {
    pub option_ids: Vec<i64>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PollOptionVoter {
    pub email: String,
    pub handle: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PollOptionView {
    pub option_id: i64,
    pub label: String,
    pub vote_count: i64,
    pub voters: Vec<PollOptionVoter>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PollView {
    pub poll_id: i64,
    pub event_id: i64,
    pub question: String,
    pub allow_multiple: bool,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub created_by_email: Option<String>,
    pub options: Vec<PollOptionView>,
    pub my_votes: Vec<i64>,
    pub total_votes: i64,
    pub has_expired: bool,
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
pub struct DeviceRegisterPayload {
    /// Jeton FCM renvoyé par firebase_messaging
    pub token: String,
    /// Plateforme de l'appareil: ios, android ou web
    pub platform: String,
    /// Langue préférée (ex: fr-FR)
    pub locale: Option<String>,
    /// Version applicative pour diagnostiques
    pub app_version: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct DeviceRefreshPayload {
    pub old_token: String,
    pub new_token: String,
    pub platform: Option<String>,
    pub locale: Option<String>,
    pub app_version: Option<String>,
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

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct QRCodeGenerateResponse {
    pub qr_token: String,
    pub event_id: i64,
    pub generated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct QRCodeScanPayload {
    pub token: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct QRCodeScanResponse {
    pub success: bool,
    pub status: String,
    pub user_email: Option<String>,
    pub user_handle: Option<String>,
    pub user_avatar_url: Option<String>,
    pub scanned_at: Option<chrono::DateTime<chrono::Utc>>,
    pub message: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct QRCodeStatsResponse {
    pub total_invited: i64,
    pub total_checked_in: i64,
    pub pending_checkins: i64,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CarpoolPayload {
    pub origin: String,
    pub origin_latitude: Option<f64>,
    pub origin_longitude: Option<f64>,
    pub depart_at: chrono::DateTime<chrono::Utc>,
    pub seats_total: i32,
    pub notes: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CarpoolPatchPayload {
    pub origin: Option<String>,
    pub origin_latitude: Option<f64>,
    pub origin_longitude: Option<f64>,
    pub depart_at: Option<chrono::DateTime<chrono::Utc>>,
    pub seats_total: Option<i32>,
    pub notes: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema, FromRow)]
pub struct Carpool {
    pub carpool_id: i64,
    pub event_id: i64,
    pub driver_id: i64,
    pub origin: String,
    pub origin_latitude: Option<f64>,
    pub origin_longitude: Option<f64>,
    pub depart_at: chrono::DateTime<chrono::Utc>,
    pub seats_total: i32,
    pub seats_taken: i32,
    pub notes: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema, FromRow)]
pub struct CarpoolPassenger {
    pub user_id: i64,
    pub handle: Option<String>,
    pub avatar_url: Option<String>,
    pub joined_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema, FromRow)]
pub struct CarpoolView {
    pub carpool_id: i64,
    pub event_id: i64,
    pub driver_id: i64,
    pub driver_handle: Option<String>,
    pub driver_avatar_url: Option<String>,
    pub origin: String,
    pub origin_latitude: Option<f64>,
    pub origin_longitude: Option<f64>,
    pub depart_at: chrono::DateTime<chrono::Utc>,
    pub seats_total: i32,
    pub seats_taken: i32,
    pub notes: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub passengers: Vec<CarpoolPassenger>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CarpoolJoinResponse {
    pub success: bool,
    pub seats_taken: i32,
    pub seats_total: i32,
    pub message: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CarpoolLeaveResponse {
    pub success: bool,
    pub seats_taken: i32,
    pub seats_total: i32,
    pub message: String,
}
