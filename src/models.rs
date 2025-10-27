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
