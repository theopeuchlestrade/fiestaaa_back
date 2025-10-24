use utoipa::OpenApi;

use crate::models::{
    ErrorResponse, HealthResponse, LoginPayload, MeResponse, RegisterPayload, StatusResponse,
    TokenResponse,
};

#[derive(OpenApi)]
#[openapi(
    paths(
        crate::routes::root::hello,
        crate::routes::root::me,
        crate::routes::auth::register,
        crate::routes::auth::login,
        crate::routes::health::health
    ),
    components(
        schemas(
            LoginPayload,
            RegisterPayload,
            StatusResponse,
            TokenResponse,
            ErrorResponse,
            MeResponse,
            HealthResponse
        )
    ),
    tags(
        (name = "root", description = "Endpoints généraux"),
        (name = "auth", description = "Authentification"),
        (name = "health", description = "Surveillance de l'API")
    )
)]
pub struct ApiDoc;
