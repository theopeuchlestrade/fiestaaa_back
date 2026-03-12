mod common;

use std::error::Error;

use actix_web::{App, http::StatusCode, test};
use chrono::{NaiveDate, NaiveTime};
use common::{DB_LOCK, build_state, obtain_pool, reset_tables};
use fiestaaa_back::{
    auth::{encode_jwt, hash_password, now_ts},
    models::{
        Claims, Event, EventItemReservationPayload, EventItemView, EventPatchPayload, EventPayload,
    },
    routes,
};
use serde_json::Value;
use sqlx::PgPool;

fn admin_token(secret: &str, email: &str) -> Option<String> {
    let handle = email
        .split('@')
        .next()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("user")
        .to_string();
    let claims = Claims {
        sub: email.to_string(),
        exp: (now_ts() + 3600) as usize,
        handle,
    };
    encode_jwt(&claims, secret).ok()
}

async fn seed_payment_provider(pool: &PgPool, name: &str) -> sqlx::Result<i32> {
    sqlx::query_scalar::<_, i32>(
        "INSERT INTO payment_providers (provider_name, url_template) VALUES ($1, 'https://example.com/{identifier}') RETURNING provider_id"
    )
    .bind(name)
    .fetch_one(pool)
    .await
}

async fn seed_payment_provider_with_regex(
    pool: &PgPool,
    name: &str,
    validation_regex: &str,
) -> sqlx::Result<i32> {
    sqlx::query_scalar::<_, i32>(
        "INSERT INTO payment_providers (provider_name, url_template, validation_regex)
         VALUES ($1, 'https://example.com/{identifier}', $2)
         RETURNING provider_id",
    )
    .bind(name)
    .bind(validation_regex)
    .fetch_one(pool)
    .await
}

async fn seed_item(
    pool: &PgPool,
    type_name: &str,
    item_name: &str,
    max_quantity: i32,
) -> sqlx::Result<i64> {
    let type_id =
        sqlx::query_scalar::<_, i64>("INSERT INTO item_types (type) VALUES ($1) RETURNING type_id")
            .bind(type_name)
            .fetch_one(pool)
            .await?;

    sqlx::query_scalar::<_, i64>(
        "INSERT INTO items (type_id, name_item, max_quantity) VALUES ($1, $2, $3) RETURNING item_id",
    )
    .bind(type_id)
    .bind(item_name)
    .bind(max_quantity)
    .fetch_one(pool)
    .await
}

async fn seed_item_with_kind(
    pool: &PgPool,
    type_name: &str,
    item_name: &str,
    max_quantity: i32,
    unit_label: &str,
    item_kind: &str,
) -> sqlx::Result<i64> {
    let type_id =
        sqlx::query_scalar::<_, i64>("INSERT INTO item_types (type) VALUES ($1) RETURNING type_id")
            .bind(type_name)
            .fetch_one(pool)
            .await?;

    sqlx::query_scalar::<_, i64>(
        "INSERT INTO items (type_id, name_item, max_quantity, unit_label, item_kind)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING item_id",
    )
    .bind(type_id)
    .bind(item_name)
    .bind(max_quantity)
    .bind(unit_label)
    .bind(item_kind)
    .fetch_one(pool)
    .await
}

async fn seed_user(pool: &PgPool, email: &str) -> sqlx::Result<i64> {
    let hash = hash_password("password").expect("hash");
    let handle = email
        .split('@')
        .next()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("user")
        .replace(|c: char| !c.is_ascii_alphanumeric(), "_");
    sqlx::query_scalar::<_, i64>(
        "INSERT INTO users (email, password_hash, handle) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(email)
    .bind(hash)
    .bind(handle)
    .fetch_one(pool)
    .await
}

async fn seed_invitation(
    pool: &PgPool,
    event_id: i64,
    user_id: i64,
    status: &str,
) -> sqlx::Result<()> {
    sqlx::query("INSERT INTO invitations (event_id, user_id, status) VALUES ($1, $2, $3)")
        .bind(event_id)
        .bind(user_id)
        .bind(status)
        .execute(pool)
        .await?;
    Ok(())
}

#[tokio::test]
async fn list_events_initially_empty() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping events tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["events", "payment_providers"]).await?;

    let secret = "secret";
    let state = build_state(pool.clone(), secret, &["admin@example.com"]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let resp = test::call_service(&app, test::TestRequest::get().uri("/events").to_request()).await;

    assert_eq!(resp.status(), StatusCode::OK);
    let payload: Vec<Event> = test::read_body_json(resp).await;
    assert!(payload.is_empty());
    Ok(())
}

#[tokio::test]
async fn create_event_requires_authentication() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping events tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["events", "payment_providers"]).await?;

    let secret = "secret";
    let state = build_state(pool.clone(), secret, &["admin@example.com"]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/events")
            .set_json(&EventPayload {
                enabled_features: None,
                name_event: "Summer Party".to_string(),
                description: "The best party of the summer".to_string(),
                date_event: NaiveDate::from_ymd_opt(2024, 7, 1).unwrap(),
                start_time: NaiveTime::from_hms_opt(20, 0, 0).unwrap(),
                end_date: None,
                end_time: None,
                invitation_deadline: None,
                address: "123 Party Street".to_string(),
                latitude: None,
                longitude: None,
                payment_provider_id: None,
                payment_identifier: None,
                payment_requested_amount: None,
                payment_per_person: None,
                playlist_url: None,
                playlist_provider: None,
            })
            .to_request(),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}

#[tokio::test]
async fn create_event_allows_authenticated_user() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping events tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["events", "payment_providers"]).await?;

    let secret = "secret";
    let state = build_state(pool.clone(), secret, &["admin@example.com"]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let token = admin_token(secret, "user@example.com").expect("token");

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/events")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .set_json(&EventPayload {
                enabled_features: None,
                name_event: "Summer Party".to_string(),
                description: "The best party of the summer".to_string(),
                date_event: NaiveDate::from_ymd_opt(2024, 7, 1).unwrap(),
                start_time: NaiveTime::from_hms_opt(20, 0, 0).unwrap(),
                end_date: None,
                end_time: None,
                invitation_deadline: None,
                address: "123 Party Street".to_string(),
                latitude: None,
                longitude: None,
                payment_provider_id: None,
                payment_identifier: None,
                payment_requested_amount: None,
                payment_per_person: None,
                playlist_url: None,
                playlist_provider: None,
            })
            .to_request(),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::CREATED);
    Ok(())
}

#[tokio::test]
async fn events_crud_flow() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping events tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["events", "payment_providers"]).await?;

    let secret = "secret";
    let admin_email = "admin@example.com";
    let state = build_state(pool.clone(), secret, &[admin_email]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let provider_id = seed_payment_provider(&pool, "TestProvider").await?;
    let token = admin_token(secret, admin_email).expect("token");

    // Create an event
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/events")
            .insert_header(("Authorization", format!("Bearer {}", token.clone())))
            .set_json(&EventPayload {
                enabled_features: None,
                name_event: "Summer Party".to_string(),
                description: "The best party of the summer".to_string(),
                date_event: NaiveDate::from_ymd_opt(2024, 7, 1).unwrap(),
                start_time: NaiveTime::from_hms_opt(20, 0, 0).unwrap(),
                end_date: None,
                end_time: None,
                invitation_deadline: None,
                address: "123 Party Street".to_string(),
                latitude: None,
                longitude: None,
                payment_provider_id: Some(provider_id),
                payment_identifier: Some("PARTY2024".to_string()),
                payment_requested_amount: None,
                payment_per_person: None,
                playlist_url: Some("https://open.spotify.com/playlist/test".to_string()),
                playlist_provider: Some("spotify".to_string()),
            })
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let created: Event = test::read_body_json(resp).await;
    assert_eq!(created.name_event, "Summer Party");
    assert_eq!(created.payment_provider_id, Some(provider_id));
    assert_eq!(created.playlist_provider.as_deref(), Some("spotify"));
    assert_eq!(created.owner_email, admin_email);

    // List events
    let resp = test::call_service(&app, test::TestRequest::get().uri("/events").to_request()).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let listed: Vec<Event> = test::read_body_json(resp).await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].owner_email, admin_email);

    // Update event (PUT)
    let resp = test::call_service(
        &app,
        test::TestRequest::put()
            .uri(&format!("/events/{}", created.event_id))
            .insert_header(("Authorization", format!("Bearer {}", token.clone())))
            .set_json(&EventPayload {
                enabled_features: None,
                name_event: "Mega Summer Party".to_string(),
                description: "The BIGGEST party of the summer".to_string(),
                date_event: NaiveDate::from_ymd_opt(2024, 7, 2).unwrap(),
                start_time: NaiveTime::from_hms_opt(21, 0, 0).unwrap(),
                end_date: None,
                end_time: None,
                invitation_deadline: None,
                address: "456 Party Avenue".to_string(),
                latitude: None,
                longitude: None,
                payment_provider_id: Some(provider_id),
                payment_identifier: Some("MEGAPARTY2024".to_string()),
                payment_requested_amount: None,
                payment_per_person: None,
                playlist_url: None,
                playlist_provider: None,
            })
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let replaced: Event = test::read_body_json(resp).await;
    assert_eq!(replaced.name_event, "Mega Summer Party");
    assert_eq!(replaced.address, "456 Party Avenue");
    assert_eq!(replaced.owner_email, admin_email);

    // Partial update (PATCH)
    let resp = test::call_service(
        &app,
        test::TestRequest::patch()
            .uri(&format!("/events/{}", created.event_id))
            .insert_header(("Authorization", format!("Bearer {}", token.clone())))
            .set_json(&EventPatchPayload {
                enabled_features: None,
                name_event: Some("Super Mega Summer Party".to_string()),
                description: None,
                date_event: None,
                start_time: Some(NaiveTime::from_hms_opt(22, 0, 0).unwrap()),
                end_date: None,
                end_time: None,
                invitation_deadline: None,
                address: None,
                latitude: None,
                longitude: None,
                payment_provider_id: None,
                payment_identifier: None,
                payment_requested_amount: None,
                payment_per_person: None,
                playlist_url: None,
                playlist_provider: None,
            })
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let patched: Event = test::read_body_json(resp).await;
    assert_eq!(patched.name_event, "Super Mega Summer Party");
    assert_eq!(patched.start_time.format("%H:%M").to_string(), "22:00");
    assert_eq!(patched.owner_email, admin_email);

    // Delete event
    let resp = test::call_service(
        &app,
        test::TestRequest::delete()
            .uri(&format!("/events/{}", created.event_id))
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    let remaining: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM events")
        .fetch_one(&pool)
        .await?;
    assert_eq!(remaining.0, 0);

    Ok(())
}

#[tokio::test]
async fn update_event_playlist_requires_creator_or_admin() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping events tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["events", "payment_providers", "users"]).await?;

    let secret = "secret";
    let admin_email = "admin@example.com";
    let state = build_state(pool.clone(), secret, &[admin_email]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let creator_email = "creator@example.com";
    let other_email = "other@example.com";
    seed_user(&pool, creator_email).await?;
    seed_user(&pool, other_email).await?;

    let creator_token = admin_token(secret, creator_email).expect("token");
    let other_token = admin_token(secret, other_email).expect("token");

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/events")
            .insert_header(("Authorization", format!("Bearer {}", creator_token.clone())))
            .set_json(&EventPayload {
                enabled_features: None,
                name_event: "Playlist Party".to_string(),
                description: "Music matters".to_string(),
                date_event: NaiveDate::from_ymd_opt(2024, 7, 10).unwrap(),
                start_time: NaiveTime::from_hms_opt(20, 0, 0).unwrap(),
                end_date: None,
                end_time: None,
                invitation_deadline: None,
                address: "123 Party Street".to_string(),
                latitude: None,
                longitude: None,
                payment_provider_id: None,
                payment_identifier: None,
                payment_requested_amount: None,
                payment_per_person: None,
                playlist_url: None,
                playlist_provider: None,
            })
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let created: Event = test::read_body_json(resp).await;

    let resp = test::call_service(
        &app,
        test::TestRequest::patch()
            .uri(&format!("/events/{}", created.event_id))
            .insert_header(("Authorization", format!("Bearer {}", other_token)))
            .set_json(&EventPatchPayload {
                enabled_features: None,
                name_event: None,
                description: None,
                date_event: None,
                start_time: None,
                end_date: None,
                end_time: None,
                invitation_deadline: None,
                address: None,
                latitude: None,
                longitude: None,
                payment_provider_id: None,
                payment_identifier: None,
                payment_requested_amount: None,
                payment_per_person: None,
                playlist_url: Some(Some("https://open.spotify.com/playlist/test".to_string())),
                playlist_provider: Some(Some("spotify".to_string())),
            })
            .to_request(),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    Ok(())
}

#[tokio::test]
async fn update_event_playlist_rejects_non_owner_when_admins_are_unset()
-> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping events tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["events", "payment_providers", "users"]).await?;

    let secret = "secret";
    let state = build_state(pool.clone(), secret, &[]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let creator_email = "creator@example.com";
    let other_email = "other@example.com";
    seed_user(&pool, creator_email).await?;
    seed_user(&pool, other_email).await?;

    let creator_token = admin_token(secret, creator_email).expect("token");
    let other_token = admin_token(secret, other_email).expect("token");

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/events")
            .insert_header(("Authorization", format!("Bearer {}", creator_token)))
            .set_json(&EventPayload {
                enabled_features: None,
                name_event: "Playlist Party".to_string(),
                description: "Music matters".to_string(),
                date_event: NaiveDate::from_ymd_opt(2024, 7, 10).unwrap(),
                start_time: NaiveTime::from_hms_opt(20, 0, 0).unwrap(),
                end_date: None,
                end_time: None,
                invitation_deadline: None,
                address: "123 Party Street".to_string(),
                latitude: None,
                longitude: None,
                payment_provider_id: None,
                payment_identifier: None,
                payment_requested_amount: None,
                payment_per_person: None,
                playlist_url: None,
                playlist_provider: None,
            })
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let created: Event = test::read_body_json(resp).await;

    let resp = test::call_service(
        &app,
        test::TestRequest::patch()
            .uri(&format!("/events/{}", created.event_id))
            .insert_header(("Authorization", format!("Bearer {}", other_token)))
            .set_json(&EventPatchPayload {
                enabled_features: None,
                name_event: None,
                description: None,
                date_event: None,
                start_time: None,
                end_date: None,
                end_time: None,
                invitation_deadline: None,
                address: None,
                latitude: None,
                longitude: None,
                payment_provider_id: None,
                payment_identifier: None,
                payment_requested_amount: None,
                payment_per_person: None,
                playlist_url: Some(Some("https://open.spotify.com/playlist/test".to_string())),
                playlist_provider: Some(Some("spotify".to_string())),
            })
            .to_request(),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    Ok(())
}

#[tokio::test]
async fn update_event_playlist_requires_valid_provider() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping events tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["events", "payment_providers"]).await?;

    let secret = "secret";
    let admin_email = "admin@example.com";
    let state = build_state(pool.clone(), secret, &[admin_email]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let token = admin_token(secret, admin_email).expect("token");

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/events")
            .insert_header(("Authorization", format!("Bearer {}", token.clone())))
            .set_json(&EventPayload {
                enabled_features: None,
                name_event: "Playlist Party".to_string(),
                description: "Music matters".to_string(),
                date_event: NaiveDate::from_ymd_opt(2024, 7, 10).unwrap(),
                start_time: NaiveTime::from_hms_opt(20, 0, 0).unwrap(),
                end_date: None,
                end_time: None,
                invitation_deadline: None,
                address: "123 Party Street".to_string(),
                latitude: None,
                longitude: None,
                payment_provider_id: None,
                payment_identifier: None,
                payment_requested_amount: None,
                payment_per_person: None,
                playlist_url: None,
                playlist_provider: None,
            })
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let created: Event = test::read_body_json(resp).await;

    let resp = test::call_service(
        &app,
        test::TestRequest::patch()
            .uri(&format!("/events/{}", created.event_id))
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .set_json(&EventPatchPayload {
                enabled_features: None,
                name_event: None,
                description: None,
                date_event: None,
                start_time: None,
                end_date: None,
                end_time: None,
                invitation_deadline: None,
                address: None,
                latitude: None,
                longitude: None,
                payment_provider_id: None,
                payment_identifier: None,
                payment_requested_amount: None,
                payment_per_person: None,
                playlist_url: Some(Some("https://open.spotify.com/playlist/test".to_string())),
                playlist_provider: Some(Some("spotify".to_string())),
            })
            .to_request(),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
async fn update_event_playlist_requires_valid_url() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping events tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["events", "payment_providers"]).await?;

    let secret = "secret";
    let admin_email = "admin@example.com";
    let state = build_state(pool.clone(), secret, &[admin_email]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let token = admin_token(secret, admin_email).expect("token");

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/events")
            .insert_header(("Authorization", format!("Bearer {}", token.clone())))
            .set_json(&EventPayload {
                enabled_features: None,
                name_event: "Playlist Party".to_string(),
                description: "Music matters".to_string(),
                date_event: NaiveDate::from_ymd_opt(2024, 7, 10).unwrap(),
                start_time: NaiveTime::from_hms_opt(20, 0, 0).unwrap(),
                end_date: None,
                end_time: None,
                invitation_deadline: None,
                address: "123 Party Street".to_string(),
                latitude: None,
                longitude: None,
                payment_provider_id: None,
                payment_identifier: None,
                payment_requested_amount: None,
                payment_per_person: None,
                playlist_url: None,
                playlist_provider: None,
            })
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let created: Event = test::read_body_json(resp).await;

    let resp = test::call_service(
        &app,
        test::TestRequest::patch()
            .uri(&format!("/events/{}", created.event_id))
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .set_json(&EventPatchPayload {
                enabled_features: None,
                name_event: None,
                description: None,
                date_event: None,
                start_time: None,
                end_date: None,
                end_time: None,
                invitation_deadline: None,
                address: None,
                latitude: None,
                longitude: None,
                payment_provider_id: None,
                payment_identifier: None,
                payment_requested_amount: None,
                payment_per_person: None,
                playlist_url: Some(Some("https://example.com/playlist".to_string())),
                playlist_provider: Some(Some("spotify".to_string())),
            })
            .to_request(),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
async fn update_event_playlist_can_clear_fields() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping events tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["events", "payment_providers"]).await?;

    let secret = "secret";
    let admin_email = "admin@example.com";
    let state = build_state(pool.clone(), secret, &[admin_email]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let token = admin_token(secret, admin_email).expect("token");

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/events")
            .insert_header(("Authorization", format!("Bearer {}", token.clone())))
            .set_json(&EventPayload {
                enabled_features: None,
                name_event: "Playlist Party".to_string(),
                description: "Music matters".to_string(),
                date_event: NaiveDate::from_ymd_opt(2024, 7, 10).unwrap(),
                start_time: NaiveTime::from_hms_opt(20, 0, 0).unwrap(),
                end_date: None,
                end_time: None,
                invitation_deadline: None,
                address: "123 Party Street".to_string(),
                latitude: None,
                longitude: None,
                payment_provider_id: None,
                payment_identifier: None,
                payment_requested_amount: None,
                payment_per_person: None,
                playlist_url: Some("https://open.spotify.com/playlist/test".to_string()),
                playlist_provider: Some("spotify".to_string()),
            })
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let created: Event = test::read_body_json(resp).await;
    assert_eq!(created.playlist_provider.as_deref(), Some("spotify"));

    let resp = test::call_service(
        &app,
        test::TestRequest::patch()
            .uri(&format!("/events/{}", created.event_id))
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .set_json(&EventPatchPayload {
                enabled_features: None,
                name_event: None,
                description: None,
                date_event: None,
                start_time: None,
                end_date: None,
                end_time: None,
                invitation_deadline: None,
                address: None,
                latitude: None,
                longitude: None,
                payment_provider_id: None,
                payment_identifier: None,
                payment_requested_amount: None,
                payment_per_person: None,
                playlist_url: Some(None),
                playlist_provider: Some(None),
            })
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let updated: Event = test::read_body_json(resp).await;
    assert!(updated.playlist_url.is_none());
    assert!(updated.playlist_provider.is_none());
    Ok(())
}

#[tokio::test]
async fn event_items_reservation_flow() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping events tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(
        &pool,
        &[
            "events",
            "payment_providers",
            "item_types",
            "items",
            "events_items",
            "user_items",
            "users",
        ],
    )
    .await?;

    let secret = "secret";
    let admin_email = "admin@example.com";
    let state = build_state(pool.clone(), secret, &[admin_email]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;
    let admin_token_value = admin_token(secret, admin_email).expect("token");

    // Create event
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/events")
            .insert_header((
                "Authorization",
                format!("Bearer {}", admin_token_value.clone()),
            ))
            .set_json(&EventPayload {
                enabled_features: None,
                name_event: "Tasting Night".to_string(),
                description: "Bring your best drinks".to_string(),
                date_event: NaiveDate::from_ymd_opt(2024, 8, 1).unwrap(),
                start_time: NaiveTime::from_hms_opt(18, 30, 0).unwrap(),
                end_date: None,
                end_time: None,
                invitation_deadline: None,
                address: "Club House".to_string(),
                latitude: None,
                longitude: None,
                payment_provider_id: None,
                payment_identifier: None,
                payment_requested_amount: None,
                payment_per_person: None,
                playlist_url: None,
                playlist_provider: None,
            })
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let event: Event = test::read_body_json(resp).await;

    // Seed catalog item
    let item_id = seed_item(&pool, "Boisson", "Punch", 5).await?;

    // Listing automatically seeds catalog items for the event
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(&format!("/events/{}/items", event.event_id))
            .insert_header((
                "Authorization",
                format!("Bearer {}", admin_token_value.clone()),
            ))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let listed: Vec<EventItemView> = test::read_body_json(resp).await;
    assert!(listed.iter().any(|item| item.item_id == item_id));

    // Seed two users
    let user_one = "alice@example.com";
    let user_two = "bob@example.com";
    let user_one_id = seed_user(&pool, user_one).await?;
    let user_two_id = seed_user(&pool, user_two).await?;
    seed_invitation(&pool, event.event_id, user_one_id, "Accepted").await?;
    seed_invitation(&pool, event.event_id, user_two_id, "Accepted").await?;

    let user_one_token = admin_token(secret, user_one).expect("token");
    let user_two_token = admin_token(secret, user_two).expect("token");

    // User one reserves 2 units
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!(
                "/events/{}/items/{}/reserve",
                event.event_id, item_id
            ))
            .insert_header((
                "Authorization",
                format!("Bearer {}", user_one_token.clone()),
            ))
            .set_json(&EventItemReservationPayload { quantity: 2 })
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let reserved: EventItemView = test::read_body_json(resp).await;
    assert_eq!(reserved.reserved_quantity, 2);

    // User two reserves 2 units (total = 4)
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!(
                "/events/{}/items/{}/reserve",
                event.event_id, item_id
            ))
            .insert_header((
                "Authorization",
                format!("Bearer {}", user_two_token.clone()),
            ))
            .set_json(&EventItemReservationPayload { quantity: 2 })
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let reserved: EventItemView = test::read_body_json(resp).await;
    assert_eq!(reserved.reserved_quantity, 4);

    // User two attempts to exceed max quantity
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!(
                "/events/{}/items/{}/reserve",
                event.event_id, item_id
            ))
            .insert_header((
                "Authorization",
                format!("Bearer {}", user_two_token.clone()),
            ))
            .set_json(&EventItemReservationPayload { quantity: 5 })
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // User one adjusts contribution to 1 unit (total should become 3)
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!(
                "/events/{}/items/{}/reserve",
                event.event_id, item_id
            ))
            .insert_header((
                "Authorization",
                format!("Bearer {}", user_one_token.clone()),
            ))
            .set_json(&EventItemReservationPayload { quantity: 1 })
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let reserved: EventItemView = test::read_body_json(resp).await;
    assert_eq!(reserved.reserved_quantity, 3);

    // Listing reflects final quantity
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(&format!("/events/{}/items", event.event_id))
            .insert_header(("Authorization", format!("Bearer {}", admin_token_value)))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let listed: Vec<EventItemView> = test::read_body_json(resp).await;
    let punch = listed
        .iter()
        .find(|item| item.item_id == item_id)
        .expect("event item exists");
    assert_eq!(punch.reserved_quantity, 3);

    Ok(())
}

#[tokio::test]
async fn event_items_scope_filters() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping events tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(
        &pool,
        &[
            "events",
            "payment_providers",
            "item_types",
            "items",
            "events_items",
            "user_items",
            "users",
        ],
    )
    .await?;

    let secret = "secret";
    let admin_email = "admin@example.com";
    let state = build_state(pool.clone(), secret, &[admin_email]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;
    let admin_token_value = admin_token(secret, admin_email).expect("token");

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/events")
            .insert_header((
                "Authorization",
                format!("Bearer {}", admin_token_value.clone()),
            ))
            .set_json(&EventPayload {
                enabled_features: None,
                name_event: "Scope Party".to_string(),
                description: "Scope filters".to_string(),
                date_event: NaiveDate::from_ymd_opt(2024, 8, 1).unwrap(),
                start_time: NaiveTime::from_hms_opt(18, 30, 0).unwrap(),
                end_date: None,
                end_time: None,
                invitation_deadline: None,
                address: "Club House".to_string(),
                latitude: None,
                longitude: None,
                payment_provider_id: None,
                payment_identifier: None,
                payment_requested_amount: None,
                payment_per_person: None,
                playlist_url: None,
                playlist_provider: None,
            })
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let event: Event = test::read_body_json(resp).await;

    let owner_id = seed_user(&pool, admin_email).await?;
    let user_email = "alice@example.com";
    let user_id = seed_user(&pool, user_email).await?;
    let user_token = admin_token(secret, user_email).expect("token");
    seed_invitation(&pool, event.event_id, user_id, "Accepted").await?;

    let need_open_item =
        seed_item_with_kind(&pool, "ScopeNeedOpen", "Glacons", 5, "pieces", "need").await?;
    let need_completed_item =
        seed_item_with_kind(&pool, "ScopeNeedDone", "Sodas", 3, "pieces", "need").await?;
    let bring_item = seed_item_with_kind(&pool, "ScopeBring", "Chips", 1, "sac", "bring").await?;

    sqlx::query(
        "INSERT INTO events_items (event_id, item_id, max_quantity, quantity, created_by)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(event.event_id)
    .bind(need_open_item)
    .bind(5)
    .bind(2)
    .bind(owner_id)
    .execute(&pool)
    .await?;

    sqlx::query(
        "INSERT INTO events_items (event_id, item_id, max_quantity, quantity, created_by)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(event.event_id)
    .bind(need_completed_item)
    .bind(3)
    .bind(3)
    .bind(owner_id)
    .execute(&pool)
    .await?;

    sqlx::query(
        "INSERT INTO events_items (event_id, item_id, max_quantity, quantity, created_by)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(event.event_id)
    .bind(bring_item)
    .bind(1)
    .bind(0)
    .bind(user_id)
    .execute(&pool)
    .await?;

    sqlx::query(
        "INSERT INTO user_items (user_id, event_id, item_id, quantity)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(user_id)
    .bind(event.event_id)
    .bind(need_open_item)
    .bind(2)
    .execute(&pool)
    .await?;

    let all_resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(&format!("/events/{}/items?scope=all", event.event_id))
            .insert_header((
                "Authorization",
                format!("Bearer {}", admin_token_value.clone()),
            ))
            .to_request(),
    )
    .await;
    assert_eq!(all_resp.status(), StatusCode::OK);
    let all_items: Vec<EventItemView> = test::read_body_json(all_resp).await;
    assert_eq!(all_items.len(), 3);

    let to_cover_resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(&format!("/events/{}/items?scope=to_cover", event.event_id))
            .insert_header((
                "Authorization",
                format!("Bearer {}", admin_token_value.clone()),
            ))
            .to_request(),
    )
    .await;
    assert_eq!(to_cover_resp.status(), StatusCode::OK);
    let to_cover_items: Vec<EventItemView> = test::read_body_json(to_cover_resp).await;
    assert_eq!(to_cover_items.len(), 1);
    assert_eq!(to_cover_items[0].item_id, need_open_item);

    let completed_resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(&format!("/events/{}/items?scope=completed", event.event_id))
            .insert_header((
                "Authorization",
                format!("Bearer {}", admin_token_value.clone()),
            ))
            .to_request(),
    )
    .await;
    assert_eq!(completed_resp.status(), StatusCode::OK);
    let completed_items: Vec<EventItemView> = test::read_body_json(completed_resp).await;
    assert_eq!(completed_items.len(), 1);
    assert_eq!(completed_items[0].item_id, need_completed_item);

    let mine_unauthorized = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(&format!("/events/{}/items?scope=mine", event.event_id))
            .to_request(),
    )
    .await;
    assert_eq!(mine_unauthorized.status(), StatusCode::UNAUTHORIZED);

    let outsider_email = "outsider@example.com";
    seed_user(&pool, outsider_email).await?;
    let outsider_token = admin_token(secret, outsider_email).expect("token");
    let outsider_all = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(&format!("/events/{}/items?scope=all", event.event_id))
            .insert_header((
                "Authorization",
                format!("Bearer {}", outsider_token.clone()),
            ))
            .to_request(),
    )
    .await;
    assert_eq!(outsider_all.status(), StatusCode::FORBIDDEN);

    let mine_resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(&format!("/events/{}/items?scope=mine", event.event_id))
            .insert_header(("Authorization", format!("Bearer {}", user_token)))
            .to_request(),
    )
    .await;
    assert_eq!(mine_resp.status(), StatusCode::OK);
    let mine_items: Vec<EventItemView> = test::read_body_json(mine_resp).await;
    assert_eq!(mine_items.len(), 2);
    assert!(mine_items.iter().any(|item| item.item_id == need_open_item));
    assert!(mine_items.iter().any(|item| item.item_id == bring_item));

    let invalid_scope_resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(&format!("/events/{}/items?scope=unknown", event.event_id))
            .insert_header(("Authorization", format!("Bearer {}", admin_token_value)))
            .to_request(),
    )
    .await;
    assert_eq!(invalid_scope_resp.status(), StatusCode::BAD_REQUEST);
    let invalid_body: Value = test::read_body_json(invalid_scope_resp).await;
    assert_eq!(invalid_body["error"], "invalid_payload");

    Ok(())
}

#[tokio::test]
async fn reserve_event_item_requires_membership() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping events tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(
        &pool,
        &[
            "events",
            "payment_providers",
            "item_types",
            "items",
            "events_items",
            "user_items",
            "invitations",
            "users",
        ],
    )
    .await?;

    let secret = "secret";
    let admin_email = "admin@example.com";
    let state = build_state(pool.clone(), secret, &[admin_email]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;
    let admin_token_value = admin_token(secret, admin_email).expect("token");

    let create_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/events")
            .insert_header((
                "Authorization",
                format!("Bearer {}", admin_token_value.clone()),
            ))
            .set_json(&EventPayload {
                enabled_features: None,
                name_event: "Private Items".to_string(),
                description: "Reservation access control".to_string(),
                date_event: NaiveDate::from_ymd_opt(2030, 1, 1).unwrap(),
                start_time: NaiveTime::from_hms_opt(18, 30, 0).unwrap(),
                end_date: None,
                end_time: None,
                invitation_deadline: None,
                address: "Club House".to_string(),
                latitude: None,
                longitude: None,
                payment_provider_id: None,
                payment_identifier: None,
                payment_requested_amount: None,
                payment_per_person: None,
                playlist_url: None,
                playlist_provider: None,
            })
            .to_request(),
    )
    .await;
    assert_eq!(create_resp.status(), StatusCode::CREATED);
    let event: Event = test::read_body_json(create_resp).await;

    let item_id = seed_item(&pool, "Boisson", "Punch", 5).await?;
    sqlx::query(
        "INSERT INTO events_items (event_id, item_id, max_quantity, quantity, created_by)
         VALUES ($1, $2, $3, 0, NULL)",
    )
    .bind(event.event_id)
    .bind(item_id)
    .bind(5)
    .execute(&pool)
    .await?;

    let outsider_email = "outsider@example.com";
    seed_user(&pool, outsider_email).await?;
    let outsider_token = admin_token(secret, outsider_email).expect("token");
    let reserve_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!(
                "/events/{}/items/{}/reserve",
                event.event_id, item_id
            ))
            .insert_header(("Authorization", format!("Bearer {}", outsider_token)))
            .set_json(&EventItemReservationPayload { quantity: 1 })
            .to_request(),
    )
    .await;
    assert_eq!(reserve_resp.status(), StatusCode::FORBIDDEN);

    Ok(())
}

#[tokio::test]
async fn create_event_rejects_unknown_payment_provider() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping events tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["events", "payment_providers"]).await?;

    let secret = "secret";
    let admin_email = "admin@example.com";
    let state = build_state(pool.clone(), secret, &[admin_email]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let token = admin_token(secret, admin_email).expect("token");

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/events")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .set_json(&EventPayload {
                enabled_features: None,
                name_event: "Summer Party".to_string(),
                description: "The best party of the summer".to_string(),
                date_event: NaiveDate::from_ymd_opt(2024, 7, 1).unwrap(),
                start_time: NaiveTime::from_hms_opt(20, 0, 0).unwrap(),
                end_date: None,
                end_time: None,
                invitation_deadline: None,
                address: "123 Party Street".to_string(),
                latitude: None,
                longitude: None,
                payment_provider_id: Some(9999),
                payment_identifier: Some("INVALID".to_string()),
                payment_requested_amount: None,
                payment_per_person: None,
                playlist_url: None,
                playlist_provider: None,
            })
            .to_request(),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
async fn create_event_rejects_unsafe_absolute_payment_link() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping events tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["events", "payment_providers"]).await?;

    let secret = "secret";
    let admin_email = "admin@example.com";
    let state = build_state(pool.clone(), secret, &[admin_email]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let provider_id = seed_payment_provider_with_regex(&pool, "LooseProvider", ".*").await?;
    let token = admin_token(secret, admin_email).expect("token");

    for link in [
        "javascript:alert(1)",
        "https://localhost:8080/mock-pay",
        "https://localhost./mock-pay",
        "http://127.0.0.1:8080/mock-pay",
        "https://10.0.0.42/mock-pay",
        "https://[::ffff:127.0.0.1]/mock-pay",
        "not a valid url",
    ] {
        let resp = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/events")
                .insert_header(("Authorization", format!("Bearer {}", token)))
                .set_json(&EventPayload {
                    enabled_features: None,
                    name_event: "Unsafe payment".to_string(),
                    description: "Should be rejected".to_string(),
                    date_event: NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
                    start_time: NaiveTime::from_hms_opt(20, 0, 0).unwrap(),
                    end_date: None,
                    end_time: None,
                    invitation_deadline: None,
                    address: "123 Party Street".to_string(),
                    latitude: None,
                    longitude: None,
                    payment_provider_id: Some(provider_id),
                    payment_identifier: Some(link.to_string()),
                    payment_requested_amount: None,
                    payment_per_person: None,
                    playlist_url: None,
                    playlist_provider: None,
                })
                .to_request(),
        )
        .await;

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "link={link}");
        let body: Value = test::read_body_json(resp).await;
        assert_eq!(body["error"], "invalid_payment_link", "link={link}");
    }

    Ok(())
}

#[tokio::test]
async fn event_validates_empty_fields() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping events tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["events", "payment_providers"]).await?;

    let secret = "secret";
    let admin_email = "admin@example.com";
    let state = build_state(pool.clone(), secret, &[admin_email]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let token = admin_token(secret, admin_email).expect("token");

    // Test with empty name
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/events")
            .insert_header(("Authorization", format!("Bearer {}", token.clone())))
            .set_json(&EventPayload {
                enabled_features: None,
                name_event: "".to_string(),
                description: "Description".to_string(),
                date_event: NaiveDate::from_ymd_opt(2024, 7, 1).unwrap(),
                start_time: NaiveTime::from_hms_opt(20, 0, 0).unwrap(),
                end_date: None,
                end_time: None,
                invitation_deadline: None,
                address: "Address".to_string(),
                latitude: None,
                longitude: None,
                payment_provider_id: None,
                payment_identifier: None,
                payment_requested_amount: None,
                payment_per_person: None,
                playlist_url: None,
                playlist_provider: None,
            })
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // Test with empty description
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/events")
            .insert_header(("Authorization", format!("Bearer {}", token.clone())))
            .set_json(&EventPayload {
                enabled_features: None,
                name_event: "Event".to_string(),
                description: "".to_string(),
                date_event: NaiveDate::from_ymd_opt(2024, 7, 1).unwrap(),
                start_time: NaiveTime::from_hms_opt(20, 0, 0).unwrap(),
                end_date: None,
                end_time: None,
                invitation_deadline: None,
                address: "Address".to_string(),
                latitude: None,
                longitude: None,
                payment_provider_id: None,
                payment_identifier: None,
                payment_requested_amount: None,
                payment_per_person: None,
                playlist_url: None,
                playlist_provider: None,
            })
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // Test with empty address
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/events")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .set_json(&EventPayload {
                enabled_features: None,
                name_event: "Event".to_string(),
                description: "Description".to_string(),
                date_event: NaiveDate::from_ymd_opt(2024, 7, 1).unwrap(),
                start_time: NaiveTime::from_hms_opt(20, 0, 0).unwrap(),
                end_date: None,
                end_time: None,
                invitation_deadline: None,
                address: "".to_string(),
                latitude: None,
                longitude: None,
                payment_provider_id: None,
                payment_identifier: None,
                payment_requested_amount: None,
                payment_per_person: None,
                playlist_url: None,
                playlist_provider: None,
            })
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    Ok(())
}
