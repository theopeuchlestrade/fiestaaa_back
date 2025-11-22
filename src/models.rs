use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use utoipa::ToSchema;

#[derive(Serialize, Deserialize, ToSchema)]
pub struct LoginPayload {
    pub email: String,
    pub password: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct RegisterPayload {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct Claims {
    pub sub: String,
    pub exp: usize,
}

#[derive(Serialize, ToSchema)]
pub struct TokenResponse {
    pub token: String,
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
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ItemPayload {
    pub type_id: i64,
    pub name_item: String,
    pub max_quantity: i32,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ItemPatchPayload {
    pub type_id: Option<i64>,
    pub name_item: Option<String>,
    pub max_quantity: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema, FromRow)]
pub struct Event {
    pub event_id: i64,
    pub name_event: String,
    pub description: String,
    pub date_event: chrono::NaiveDate,
    pub start_time: chrono::NaiveTime,
    pub address: String,
    pub payment_provider_id: Option<i32>,
    pub payment_identifier: Option<String>,
    pub payment_requested_amount: Option<f64>,
    pub owner_email: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct EventPayload {
    pub name_event: String,
    pub description: String,
    pub date_event: chrono::NaiveDate,
    pub start_time: chrono::NaiveTime,
    pub address: String,
    pub payment_provider_id: Option<i32>,
    pub payment_identifier: Option<String>,
    pub payment_requested_amount: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct EventPatchPayload {
    pub name_event: Option<String>,
    pub description: Option<String>,
    pub date_event: Option<chrono::NaiveDate>,
    pub start_time: Option<chrono::NaiveTime>,
    pub address: Option<String>,
    pub payment_provider_id: Option<i32>,
    pub payment_identifier: Option<String>,
    pub payment_requested_amount: Option<f64>,
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

#[derive(Debug, Serialize, Deserialize, ToSchema, FromRow)]
pub struct Invitation {
    pub event_id: i64,
    pub email: String,
    pub status: String,
    pub date_invi: chrono::DateTime<chrono::Utc>,
    pub event_name: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct InvitationPayload {
    pub email: String,
    pub status: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct InvitationPatchPayload {
    pub status: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema, FromRow)]
pub struct PaymentProvider {
    pub provider_id: i32,
    pub provider_name: String,
    pub url_template: String,
    pub is_active: bool,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PaymentProviderPayload {
    pub provider_name: String,
    pub url_template: String,
    pub is_active: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PaymentProviderPatchPayload {
    pub provider_name: Option<String>,
    pub url_template: Option<String>,
    pub is_active: Option<bool>,
}
