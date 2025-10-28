use utoipa::OpenApi;

use crate::models::{
    ErrorResponse, Event, EventPatchPayload, EventPayload, HealthResponse, Item, ItemPatchPayload, 
    ItemPayload, LoginPayload, MeResponse, RegisterPayload, StatusResponse, TokenResponse,
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
        crate::routes::items::delete_item,
        crate::routes::events::list_events,
        crate::routes::events::create_event,
        crate::routes::events::replace_event,
        crate::routes::events::update_event,
        crate::routes::events::delete_event
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
            ItemPatchPayload,
            Event,
            EventPayload,
            EventPatchPayload
        )
    ),
    tags(
        (name = "root", description = "Endpoints généraux"),
        (name = "auth", description = "Authentification"),
        (name = "health", description = "Surveillance de l'API"),
        (name = "items", description = "Catalogue des items référencés"),
        (name = "events", description = "Gestion des événements")
    )
)]
pub struct ApiDoc;
