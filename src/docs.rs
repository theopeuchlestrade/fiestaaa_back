use utoipa::OpenApi;

use crate::models::{
    ErrorResponse, HealthResponse, Item, ItemPatchPayload, ItemPayload, LoginPayload, MeResponse,
    RegisterPayload, StatusResponse, TokenResponse,
};

#[derive(OpenApi)]
#[openapi(
    paths(
        crate::routes::root::hello,
        crate::routes::root::me,
        crate::routes::auth::register,
        crate::routes::auth::login,
        crate::routes::health::health,
        crate::routes::items::list_items,
        crate::routes::items::create_item,
        crate::routes::items::replace_item,
        crate::routes::items::update_item,
        crate::routes::items::delete_item
    ),
    components(
        schemas(
            LoginPayload,
            RegisterPayload,
            StatusResponse,
            TokenResponse,
            ErrorResponse,
            MeResponse,
            HealthResponse,
            Item,
            ItemPayload,
            ItemPatchPayload
        )
    ),
    tags(
        (name = "root", description = "Endpoints généraux"),
        (name = "auth", description = "Authentification"),
        (name = "health", description = "Surveillance de l'API"),
        (name = "items", description = "Catalogue des items référencés")
    )
)]
pub struct ApiDoc;
