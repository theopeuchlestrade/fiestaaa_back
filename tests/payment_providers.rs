mod common;

use std::error::Error;

use actix_web::{App, http::StatusCode, test};
use common::{DB_LOCK, build_state, obtain_pool, reset_tables};
use fiestaaa_back::{
    auth::{encode_jwt, now_ts},
    models::{Claims, PaymentProvider, PaymentProviderPatchPayload, PaymentProviderPayload},
    routes,
};

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

#[tokio::test]
async fn list_payment_providers_initially_empty() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping payment providers tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["payment_providers"]).await?;

    let secret = "secret";
    let state = build_state(pool.clone(), secret, &["admin@example.com"]);
    let mut app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let resp = test::call_service(
        &mut app,
        test::TestRequest::get()
            .uri("/payment-providers")
            .to_request(),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let payload: Vec<PaymentProvider> = test::read_body_json(resp).await;
    assert!(payload.is_empty());
    Ok(())
}

#[tokio::test]
async fn create_payment_provider_requires_authentication() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping payment providers tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["payment_providers"]).await?;

    let secret = "secret";
    let state = build_state(pool.clone(), secret, &["admin@example.com"]);
    let mut app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let resp = test::call_service(
        &mut app,
        test::TestRequest::post()
            .uri("/payment-providers")
            .set_json(&PaymentProviderPayload {
                provider_name: "Stripe".to_string(),
                url_template: "https://checkout.stripe.com/{identifier}".to_string(),
                validation_regex: None,
                is_active: None,
            })
            .to_request(),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}

#[tokio::test]
async fn create_payment_provider_rejects_non_admin() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping payment providers tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["payment_providers"]).await?;

    let secret = "secret";
    let state = build_state(pool.clone(), secret, &["admin@example.com"]);
    let mut app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;

    let token = admin_token(secret, "user@example.com").expect("token");

    let resp = test::call_service(
        &mut app,
        test::TestRequest::post()
            .uri("/payment-providers")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .set_json(&PaymentProviderPayload {
                provider_name: "Stripe".to_string(),
                url_template: "https://checkout.stripe.com/{identifier}".to_string(),
                validation_regex: None,
                is_active: None,
            })
            .to_request(),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    Ok(())
}

#[tokio::test]
async fn payment_providers_crud_flow() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping payment providers tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["payment_providers"]).await?;

    let secret = "secret";
    let admin_email = "admin@example.com";
    let state = build_state(pool.clone(), secret, &[admin_email]);
    let mut app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;
    let token = admin_token(secret, admin_email).expect("token");

    // Create a payment provider
    let resp = test::call_service(
        &mut app,
        test::TestRequest::post()
            .uri("/payment-providers")
            .insert_header(("Authorization", format!("Bearer {}", token.clone())))
            .set_json(&PaymentProviderPayload {
                provider_name: "Stripe".to_string(),
                url_template: "https://checkout.stripe.com/{identifier}".to_string(),
                validation_regex: None,
                is_active: Some(true),
            })
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let created: PaymentProvider = test::read_body_json(resp).await;
    assert_eq!(created.provider_name, "Stripe");
    assert!(created.is_active);

    // List payment providers
    let resp = test::call_service(
        &mut app,
        test::TestRequest::get()
            .uri("/payment-providers")
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let listed: Vec<PaymentProvider> = test::read_body_json(resp).await;
    assert_eq!(listed.len(), 1);

    // Replace (PUT) payment provider
    let resp = test::call_service(
        &mut app,
        test::TestRequest::put()
            .uri(&format!("/payment-providers/{}", created.provider_id))
            .insert_header(("Authorization", format!("Bearer {}", token.clone())))
            .set_json(&PaymentProviderPayload {
                provider_name: "Stripe Pro".to_string(),
                url_template: "https://pro.stripe.com/{identifier}".to_string(),
                validation_regex: None,
                is_active: Some(true),
            })
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let replaced: PaymentProvider = test::read_body_json(resp).await;
    assert_eq!(replaced.provider_name, "Stripe Pro");
    assert_eq!(replaced.url_template, "https://pro.stripe.com/{identifier}");

    // Update (PATCH) payment provider partially
    let resp = test::call_service(
        &mut app,
        test::TestRequest::patch()
            .uri(&format!("/payment-providers/{}", created.provider_id))
            .insert_header(("Authorization", format!("Bearer {}", token.clone())))
            .set_json(&PaymentProviderPatchPayload {
                provider_name: Some("Stripe Premium".to_string()),
                url_template: None,
                validation_regex: None,
                is_active: Some(false),
            })
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let patched: PaymentProvider = test::read_body_json(resp).await;
    assert_eq!(patched.provider_name, "Stripe Premium");
    assert_eq!(patched.url_template, "https://pro.stripe.com/{identifier}");
    assert!(!patched.is_active);

    // Delete payment provider
    let resp = test::call_service(
        &mut app,
        test::TestRequest::delete()
            .uri(&format!("/payment-providers/{}", created.provider_id))
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    let remaining: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM payment_providers")
        .fetch_one(&pool)
        .await?;
    assert_eq!(remaining.0, 0);

    Ok(())
}

#[tokio::test]
async fn create_payment_provider_validates_url_template() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping payment providers tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["payment_providers"]).await?;

    let secret = "secret";
    let admin_email = "admin@example.com";
    let state = build_state(pool.clone(), secret, &[admin_email]);
    let mut app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;
    let token = admin_token(secret, admin_email).expect("token");

    // Try creating provider with invalid URL template
    let resp = test::call_service(
        &mut app,
        test::TestRequest::post()
            .uri("/payment-providers")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .set_json(&PaymentProviderPayload {
                provider_name: "Invalid".to_string(),
                url_template: "https://example.com/no-placeholder".to_string(),
                validation_regex: None,
                is_active: None,
            })
            .to_request(),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
async fn create_payment_provider_prevents_duplicates() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping payment providers tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    reset_tables(&pool, &["payment_providers"]).await?;

    let secret = "secret";
    let admin_email = "admin@example.com";
    let state = build_state(pool.clone(), secret, &[admin_email]);
    let mut app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;
    let token = admin_token(secret, admin_email).expect("token");

    // Create first provider
    let resp = test::call_service(
        &mut app,
        test::TestRequest::post()
            .uri("/payment-providers")
            .insert_header(("Authorization", format!("Bearer {}", token.clone())))
            .set_json(&PaymentProviderPayload {
                provider_name: "Stripe".to_string(),
                url_template: "https://checkout.stripe.com/{identifier}".to_string(),
                validation_regex: None,
                is_active: None,
            })
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);

    // Try creating provider with same name
    let resp = test::call_service(
        &mut app,
        test::TestRequest::post()
            .uri("/payment-providers")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .set_json(&PaymentProviderPayload {
                provider_name: "Stripe".to_string(),
                url_template: "https://different.url/{identifier}".to_string(),
                validation_regex: None,
                is_active: None,
            })
            .to_request(),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    Ok(())
}
