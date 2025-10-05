use actix_web::{get, web, HttpRequest, HttpResponse, Responder};

use crate::{state::AppState, auth::extract_claims_from_auth};

#[get("/")]
pub async fn hello() -> impl Responder {
    HttpResponse::Ok().body("Fiestaaa API is running ✨")
}

#[get("/me")]
pub async fn me(state: web::Data<AppState>, req: HttpRequest) -> impl Responder {
    match extract_claims_from_auth(&req, &state.jwt_secret) {
        Ok(claims) => HttpResponse::Ok().json(serde_json::json!({
            "email": claims.sub,
            "exp": claims.exp
        })),
        Err(resp) => resp,
    }
}

