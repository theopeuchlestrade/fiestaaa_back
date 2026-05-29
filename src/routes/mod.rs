use actix_web::web;

pub mod auth;
pub mod carpools;
pub mod event_access;
pub mod events;
pub mod friends;
pub mod health;
pub mod invitations;
pub mod items;
pub mod notifications;
pub mod payment_providers;
pub mod qr_codes;
pub mod realtime;
pub mod root;
pub mod users;

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(root::hello)
        .service(root::me)
        .service(health::health)
        .service(health::metrics)
        .service(auth::register)
        .service(auth::verify_email)
        .service(auth::complete_registration)
        .service(auth::login)
        .service(auth::oauth_login)
        .service(auth::logout)
        .service(items::list_items)
        .service(items::create_item)
        .service(items::replace_item)
        .service(items::update_item)
        .service(items::delete_item)
        .service(invitations::list_event_invitations)
        .service(invitations::create_invitation)
        .service(invitations::delete_invitation)
        .service(invitations::list_my_invitations)
        .service(invitations::respond_invitation)
        .service(qr_codes::generate_my_qr_code)
        .service(qr_codes::scan_qr_code)
        .service(qr_codes::get_qr_scan_stats)
        .service(users::check_handle_availability)
        .service(users::update_handle)
        .service(users::upload_avatar)
        .service(users::delete_account)
        .service(events::get_event)
        .service(events::list_events)
        .service(events::create_event)
        .service(events::replace_event)
        .service(events::update_event)
        .service(events::delete_event)
        .service(events::create_share_link)
        .service(events::claim_share_link)
        .service(events::list_event_polls)
        .service(events::create_event_poll)
        .service(events::vote_event_poll)
        .service(events::delete_event_poll)
        .service(events::list_event_expenses)
        .service(events::create_event_expense)
        .service(events::delete_event_expense)
        .service(events::get_event_expenses_summary)
        .service(carpools::list_event_carpools)
        .service(carpools::create_carpool)
        .service(carpools::update_carpool)
        .service(carpools::delete_carpool)
        .service(carpools::join_carpool)
        .service(carpools::leave_carpool)
        .service(events::search_address)
        .service(events::list_event_items)
        .service(events::list_event_item_contributions)
        .service(events::attach_event_item)
        .service(events::create_custom_event_item)
        .service(events::reserve_event_item)
        .service(events::delete_event_item)
        .service(realtime::issue_realtime_ticket)
        .service(realtime::websocket)
        .service(friends::list_friends)
        .service(friends::search_friends)
        .service(friends::create_friend_request)
        .service(friends::list_friend_requests)
        .service(friends::respond_friend_request)
        .service(friends::delete_friend)
        .service(notifications::register_device)
        .service(notifications::refresh_device)
        .service(notifications::delete_device)
        .service(payment_providers::list_payment_providers)
        .service(payment_providers::create_payment_provider)
        .service(payment_providers::replace_payment_provider)
        .service(payment_providers::update_payment_provider)
        .service(payment_providers::delete_payment_provider);
}
