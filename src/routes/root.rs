use actix_web::{HttpRequest, HttpResponse, Responder, get, web};

use crate::{auth::extract_claims_from_auth, models::MeResponse, state::AppState};

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
pub async fn me(state: web::Data<AppState>, req: HttpRequest) -> impl Responder {
    match extract_claims_from_auth(&req, &state.jwt_secret) {
        Ok(claims) => HttpResponse::Ok().json(MeResponse {
            email: claims.sub,
            exp: claims.exp,
        }),
        Err(resp) => resp,
    }
}
