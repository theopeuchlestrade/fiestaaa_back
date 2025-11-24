use utoipa::OpenApi;

use crate::models::{
    AddressSuggestion, ErrorResponse, Event, EventCustomItemPayload, EventItemAttachPayload,
    EventItemReservationPayload, EventItemView, EventPatchPayload, EventPayload, HealthResponse,
    Invitation, InvitationPatchPayload, InvitationPayload, Item, ItemPatchPayload, ItemPayload,
    LoginPayload, MeResponse, PaymentProvider, PaymentProviderPatchPayload, PaymentProviderPayload,
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
        crate::routes::items::delete_item,
        crate::routes::events::list_events,
        crate::routes::events::create_event,
        crate::routes::events::replace_event,
        crate::routes::events::update_event,
        crate::routes::events::delete_event,
        crate::routes::events::search_address,
        crate::routes::events::list_event_items,
        crate::routes::events::attach_event_item,
        crate::routes::events::reserve_event_item,
        crate::routes::events::delete_event_item,
        crate::routes::events::create_custom_event_item,
        crate::routes::invitations::list_event_invitations,
        crate::routes::invitations::create_invitation,
        crate::routes::invitations::delete_invitation,
        crate::routes::invitations::list_my_invitations,
        crate::routes::invitations::respond_invitation,
        crate::routes::payment_providers::list_payment_providers,
        crate::routes::payment_providers::create_payment_provider,
        crate::routes::payment_providers::replace_payment_provider,
        crate::routes::payment_providers::update_payment_provider,
        crate::routes::payment_providers::delete_payment_provider
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
            EventPatchPayload,
            EventItemView,
            EventItemAttachPayload,
            EventItemReservationPayload,
            EventCustomItemPayload,
            Invitation,
            InvitationPayload,
            InvitationPatchPayload,
            PaymentProvider,
            PaymentProviderPayload,
            PaymentProviderPatchPayload,
            AddressSuggestion
        )
    ),
    tags(
        (name = "root", description = "Endpoints généraux"),
        (name = "auth", description = "Authentification"),
        (name = "health", description = "Surveillance de l'API"),
        (name = "items", description = "Catalogue des items référencés"),
        (name = "events", description = "Gestion des événements"),
        (name = "invitations", description = "Gestion des invitations aux événements"),
        (name = "payment-providers", description = "Configuration des fournisseurs de paiement (admin)")
    )
)]
pub struct ApiDoc;
