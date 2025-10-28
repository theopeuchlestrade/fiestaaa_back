mod common;

use std::error::Error;

use actix_web::{test, App, http::StatusCode};
use chrono::{NaiveDate, NaiveTime};
use common::{DB_LOCK, build_state, obtain_pool, reset_tables};
use fiestaaa_back::{
    auth::{encode_jwt, now_ts},
    models::{Claims, Event, EventPatchPayload, EventPayload},
    routes,
};
use sqlx::PgPool;

fn admin_token(secret: &str, email: &str) -> Option<String> {
    let claims = Claims {
        sub: email.to_string(),
        exp: (now_ts() + 3600) as usize,
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
    let mut app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let resp = test::call_service(
        &mut app,
        test::TestRequest::get().uri("/events").to_request(),
    )
    .await;

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
    let mut app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let resp = test::call_service(
        &mut app,
        test::TestRequest::post()
            .uri("/events")
            .set_json(&EventPayload {
                name_event: "Summer Party".to_string(),
                description: "The best party of the summer".to_string(),
                date_event: NaiveDate::from_ymd_opt(2024, 7, 1).unwrap(),
                start_time: NaiveTime::from_hms_opt(20, 0, 0).unwrap(),
                address: "123 Party Street".to_string(),
                payment_provider_id: None,
                payment_identifier: None,
            })
            .to_request(),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}

#[tokio::test]
async fn create_event_rejects_non_admin() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping events tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["events", "payment_providers"]).await?;

    let secret = "secret";
    let state = build_state(pool.clone(), secret, &["admin@example.com"]);
    let mut app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let token = admin_token(secret, "user@example.com").expect("token");

    let resp = test::call_service(
        &mut app,
        test::TestRequest::post()
            .uri("/events")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .set_json(&EventPayload {
                name_event: "Summer Party".to_string(),
                description: "The best party of the summer".to_string(),
                date_event: NaiveDate::from_ymd_opt(2024, 7, 1).unwrap(),
                start_time: NaiveTime::from_hms_opt(20, 0, 0).unwrap(),
                address: "123 Party Street".to_string(),
                payment_provider_id: None,
                payment_identifier: None,
            })
            .to_request(),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
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
    let mut app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let provider_id = seed_payment_provider(&pool, "TestProvider").await?;
    let token = admin_token(secret, admin_email).expect("token");

    // Create an event
    let resp = test::call_service(
        &mut app,
        test::TestRequest::post()
            .uri("/events")
            .insert_header(("Authorization", format!("Bearer {}", token.clone())))
            .set_json(&EventPayload {
                name_event: "Summer Party".to_string(),
                description: "The best party of the summer".to_string(),
                date_event: NaiveDate::from_ymd_opt(2024, 7, 1).unwrap(),
                start_time: NaiveTime::from_hms_opt(20, 0, 0).unwrap(),
                address: "123 Party Street".to_string(),
                payment_provider_id: Some(provider_id),
                payment_identifier: Some("PARTY2024".to_string()),
            })
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let created: Event = test::read_body_json(resp).await;
    assert_eq!(created.name_event, "Summer Party");
    assert_eq!(created.payment_provider_id, Some(provider_id));

    // List events
    let resp = test::call_service(
        &mut app,
        test::TestRequest::get().uri("/events").to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let listed: Vec<Event> = test::read_body_json(resp).await;
    assert_eq!(listed.len(), 1);

    // Update event (PUT)
    let resp = test::call_service(
        &mut app,
        test::TestRequest::put()
            .uri(&format!("/events/{}", created.event_id))
            .insert_header(("Authorization", format!("Bearer {}", token.clone())))
            .set_json(&EventPayload {
                name_event: "Mega Summer Party".to_string(),
                description: "The BIGGEST party of the summer".to_string(),
                date_event: NaiveDate::from_ymd_opt(2024, 7, 2).unwrap(),
                start_time: NaiveTime::from_hms_opt(21, 0, 0).unwrap(),
                address: "456 Party Avenue".to_string(),
                payment_provider_id: Some(provider_id),
                payment_identifier: Some("MEGAPARTY2024".to_string()),
            })
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let replaced: Event = test::read_body_json(resp).await;
    assert_eq!(replaced.name_event, "Mega Summer Party");
    assert_eq!(replaced.address, "456 Party Avenue");

    // Partial update (PATCH)
    let resp = test::call_service(
        &mut app,
        test::TestRequest::patch()
            .uri(&format!("/events/{}", created.event_id))
            .insert_header(("Authorization", format!("Bearer {}", token.clone())))
            .set_json(&EventPatchPayload {
                name_event: Some("Super Mega Summer Party".to_string()),
                description: None,
                date_event: None,
                start_time: Some(NaiveTime::from_hms_opt(22, 0, 0).unwrap()),
                address: None,
                payment_provider_id: None,
                payment_identifier: None,
            })
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let patched: Event = test::read_body_json(resp).await;
    assert_eq!(patched.name_event, "Super Mega Summer Party");
    assert_eq!(patched.start_time.format("%H:%M").to_string(), "22:00");

    // Delete event
    let resp = test::call_service(
        &mut app,
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
    let mut app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let token = admin_token(secret, admin_email).expect("token");

    let resp = test::call_service(
        &mut app,
        test::TestRequest::post()
            .uri("/events")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .set_json(&EventPayload {
                name_event: "Summer Party".to_string(),
                description: "The best party of the summer".to_string(),
                date_event: NaiveDate::from_ymd_opt(2024, 7, 1).unwrap(),
                start_time: NaiveTime::from_hms_opt(20, 0, 0).unwrap(),
                address: "123 Party Street".to_string(),
                payment_provider_id: Some(9999),
                payment_identifier: Some("INVALID".to_string()),
            })
            .to_request(),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
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
    let mut app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let token = admin_token(secret, admin_email).expect("token");

    // Test with empty name
    let resp = test::call_service(
        &mut app,
        test::TestRequest::post()
            .uri("/events")
            .insert_header(("Authorization", format!("Bearer {}", token.clone())))
            .set_json(&EventPayload {
                name_event: "".to_string(),
                description: "Description".to_string(),
                date_event: NaiveDate::from_ymd_opt(2024, 7, 1).unwrap(),
                start_time: NaiveTime::from_hms_opt(20, 0, 0).unwrap(),
                address: "Address".to_string(),
                payment_provider_id: None,
                payment_identifier: None,
            })
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // Test with empty description
    let resp = test::call_service(
        &mut app,
        test::TestRequest::post()
            .uri("/events")
            .insert_header(("Authorization", format!("Bearer {}", token.clone())))
            .set_json(&EventPayload {
                name_event: "Event".to_string(),
                description: "".to_string(),
                date_event: NaiveDate::from_ymd_opt(2024, 7, 1).unwrap(),
                start_time: NaiveTime::from_hms_opt(20, 0, 0).unwrap(),
                address: "Address".to_string(),
                payment_provider_id: None,
                payment_identifier: None,
            })
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // Test with empty address
    let resp = test::call_service(
        &mut app,
        test::TestRequest::post()
            .uri("/events")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .set_json(&EventPayload {
                name_event: "Event".to_string(),
                description: "Description".to_string(),
                date_event: NaiveDate::from_ymd_opt(2024, 7, 1).unwrap(),
                start_time: NaiveTime::from_hms_opt(20, 0, 0).unwrap(),
                address: "".to_string(),
                payment_provider_id: None,
                payment_identifier: None,
            })
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    Ok(())
}
