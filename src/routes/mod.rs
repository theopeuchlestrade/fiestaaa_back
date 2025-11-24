use actix_web::web;

pub mod auth;
pub mod events;
pub mod health;
pub mod invitations;
pub mod items;
pub mod payment_providers;
pub mod root;

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(root::hello)
        .service(root::me)
        .service(health::health)
        .service(auth::register)
        .service(auth::login)
        .service(items::list_items)
        .service(items::create_item)
        .service(items::replace_item)
        .service(items::update_item)
        .service(items::delete_item)
        .service(invitations::list_event_invitations)
        .service(invitations::suggest_invitations)
        .service(invitations::create_invitation)
        .service(invitations::delete_invitation)
        .service(invitations::list_my_invitations)
        .service(invitations::respond_invitation)
        .service(events::list_events)
        .service(events::create_event)
        .service(events::replace_event)
        .service(events::update_event)
        .service(events::delete_event)
        .service(events::create_share_link)
        .service(events::claim_share_link)
        .service(events::search_address)
        .service(events::list_event_items)
        .service(events::attach_event_item)
        .service(events::create_custom_event_item)
        .service(events::reserve_event_item)
        .service(events::delete_event_item)
        .service(payment_providers::list_payment_providers)
        .service(payment_providers::create_payment_provider)
        .service(payment_providers::replace_payment_provider)
        .service(payment_providers::update_payment_provider)
        .service(payment_providers::delete_payment_provider);
}
