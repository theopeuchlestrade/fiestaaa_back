mod common;

use std::error::Error;

use actix_web::{App, http::StatusCode, test};
use common::{DB_LOCK, build_state, obtain_pool, reset_tables};
use fiestaaa_back::{
    auth::{encode_jwt, now_ts},
    models::{Claims, Item, ItemPatchPayload, ItemPayload},
    routes,
};
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

async fn seed_item_type(pool: &PgPool, name: &str) -> sqlx::Result<i64> {
    sqlx::query_scalar::<_, i64>("INSERT INTO item_types (type) VALUES ($1) RETURNING type_id")
        .bind(name)
        .fetch_one(pool)
        .await
}

#[tokio::test]
async fn list_items_initially_empty() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping items tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["items", "item_types"]).await?;

    let secret = "secret";
    let state = build_state(pool.clone(), secret, &["admin@example.com"]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let resp = test::call_service(&app, test::TestRequest::get().uri("/items").to_request()).await;

    assert_eq!(resp.status(), StatusCode::OK);
    let payload: Vec<Item> = test::read_body_json(resp).await;
    assert!(payload.is_empty());
    Ok(())
}

#[tokio::test]
async fn create_item_requires_authentication() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping items tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["items", "item_types"]).await?;

    let secret = "secret";
    let state = build_state(pool.clone(), secret, &["admin@example.com"]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;
    let type_id = seed_item_type(&pool, "Drink").await?;

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/items")
            .set_json(&ItemPayload {
                type_id,
                name_item: "Soda".to_string(),
                max_quantity: 10,
                unit_label: "unités".to_string(),
                item_kind: None,
            })
            .to_request(),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}

#[tokio::test]
async fn create_item_rejects_non_admin() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping items tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["items", "item_types"]).await?;

    let secret = "secret";
    let state = build_state(pool.clone(), secret, &["admin@example.com"]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let type_id = seed_item_type(&pool, "Drink").await?;
    let token = admin_token(secret, "user@example.com").expect("token");

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/items")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .set_json(&ItemPayload {
                type_id,
                name_item: "Soda".to_string(),
                max_quantity: 10,
                unit_label: "unités".to_string(),
                item_kind: None,
            })
            .to_request(),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    Ok(())
}

#[tokio::test]
async fn create_item_rejects_requests_when_admins_are_unset() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping items tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["items", "item_types"]).await?;

    let secret = "secret";
    let state = build_state(pool.clone(), secret, &[]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let type_id = seed_item_type(&pool, "Drink").await?;
    let token = admin_token(secret, "user@example.com").expect("token");

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/items")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .set_json(&ItemPayload {
                type_id,
                name_item: "Soda".to_string(),
                max_quantity: 10,
                unit_label: "unités".to_string(),
                item_kind: None,
            })
            .to_request(),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    Ok(())
}

#[tokio::test]
async fn items_crud_flow() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping items tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["items", "item_types"]).await?;

    let secret = "secret";
    let admin_email = "admin@example.com";
    let state = build_state(pool.clone(), secret, &[admin_email]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let type_id = seed_item_type(&pool, "Drink").await?;
    let other_type_id = seed_item_type(&pool, "Food").await?;
    let token = admin_token(secret, admin_email).expect("token");

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/items")
            .insert_header(("Authorization", format!("Bearer {}", token.clone())))
            .set_json(&ItemPayload {
                type_id,
                name_item: "Soda".to_string(),
                max_quantity: 10,
                unit_label: "unités".to_string(),
                item_kind: None,
            })
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let created: Item = test::read_body_json(resp).await;
    assert_eq!(created.type_id, type_id);
    assert_eq!(created.name_item, "Soda");
    assert_eq!(created.max_quantity, 10);
    assert_eq!(created.item_kind, "need");

    let resp = test::call_service(&app, test::TestRequest::get().uri("/items").to_request()).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let listed: Vec<Item> = test::read_body_json(resp).await;
    assert_eq!(listed.len(), 1);

    let resp = test::call_service(
        &app,
        test::TestRequest::put()
            .uri(&format!("/items/{}", created.item_id))
            .insert_header(("Authorization", format!("Bearer {}", token.clone())))
            .set_json(&ItemPayload {
                type_id: other_type_id,
                name_item: "Burger".to_string(),
                max_quantity: 5,
                unit_label: "unités".to_string(),
                item_kind: None,
            })
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let replaced: Item = test::read_body_json(resp).await;
    assert_eq!(replaced.type_id, other_type_id);
    assert_eq!(replaced.name_item, "Burger");
    assert_eq!(replaced.max_quantity, 5);

    let resp = test::call_service(
        &app,
        test::TestRequest::patch()
            .uri(&format!("/items/{}", created.item_id))
            .insert_header(("Authorization", format!("Bearer {}", token.clone())))
            .set_json(&ItemPatchPayload {
                type_id: None,
                name_item: Some("Burger Deluxe".to_string()),
                max_quantity: Some(7),
                unit_label: None,
                item_kind: None,
            })
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let patched: Item = test::read_body_json(resp).await;
    assert_eq!(patched.type_id, other_type_id);
    assert_eq!(patched.name_item, "Burger Deluxe");
    assert_eq!(patched.max_quantity, 7);

    let resp = test::call_service(
        &app,
        test::TestRequest::delete()
            .uri(&format!("/items/{}", created.item_id))
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    let remaining: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM items")
        .fetch_one(&pool)
        .await?;
    assert_eq!(remaining.0, 0);

    Ok(())
}

#[tokio::test]
async fn create_item_rejects_unknown_type() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping items tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["items", "item_types"]).await?;

    let secret = "secret";
    let admin_email = "admin@example.com";
    let state = build_state(pool.clone(), secret, &[admin_email]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let token = admin_token(secret, admin_email).expect("token");

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/items")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .set_json(&ItemPayload {
                type_id: 9999,
                name_item: "Ghost".to_string(),
                max_quantity: 1,
                unit_label: "unités".to_string(),
                item_kind: None,
            })
            .to_request(),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    Ok(())
}
