use actix_web::{HttpResponse, Responder, get};

use crate::{auth::AuthenticatedUser, models::MeResponse};

#[utoipa::path(
    get,
    path = "/",
    tag = "root",
    responses(
        (status = 200, description = "API heartbeat information", body = String)
    )
)]
#[get("/")]
pub async fn hello() -> impl Responder {
    HttpResponse::Ok().body("Fiestaaa API is running ✨")
}

#[utoipa::path(
    get,
    path = "/me",
    tag = "root",
    responses(
        (status = 200, description = "Connected user information", body = MeResponse),
        (status = 401, description = "Missing or invalid token", body = crate::models::ErrorResponse)
    )
)]
#[get("/me")]
pub async fn me(user: AuthenticatedUser) -> impl Responder {
    HttpResponse::Ok().json(MeResponse {
        public_id: user.public_id.to_string(),
        email: user.email,
        handle: user.handle,
        avatar_url: user.avatar_url,
        exp: user.exp,
    })
}
