mod common;

use std::error::Error;

use actix_web::{App, http::StatusCode, test};
use common::{DB_LOCK, build_state, obtain_pool, reset_tables};
use fiestaaa_back::{
    auth::{encode_jwt, hash_password, now_ts},
    models::{Claims, Friend, FriendRequest, FriendRequestActionPayload, FriendRequestPayload},
    routes,
};
use sqlx::PgPool;

fn make_token(secret: &str, email: &str, handle: &str) -> Option<String> {
    let claims = Claims {
        sub: email.to_string(),
        exp: (now_ts() + 3600) as usize,
        handle: handle.to_string(),
    };
    encode_jwt(&claims, secret).ok()
}

async fn ensure_friend_tables(pool: &PgPool) -> sqlx::Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS friendships (
            user_a BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            user_b BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            CONSTRAINT friendships_user_order CHECK (user_a < user_b),
            CONSTRAINT friendships_no_self CHECK (user_a <> user_b),
            CONSTRAINT friendships_pk PRIMARY KEY (user_a, user_b)
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS friend_requests (
            id BIGSERIAL PRIMARY KEY,
            sender_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            receiver_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            status TEXT NOT NULL DEFAULT 'Pending',
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            responded_at TIMESTAMPTZ
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE UNIQUE INDEX IF NOT EXISTS friend_requests_pending_pair_idx
            ON friend_requests (LEAST(sender_id, receiver_id), GREATEST(sender_id, receiver_id))
            WHERE status = 'Pending'
        "#,
    )
    .execute(pool)
    .await?;

    Ok(())
}

async fn seed_user(pool: &PgPool, email: &str, handle: &str) -> sqlx::Result<i64> {
    let hash = hash_password("StrongPassw0rd!").expect("hash");
    sqlx::query_scalar::<_, i64>(
        "INSERT INTO users (email, password_hash, handle)
         VALUES ($1, $2, $3)
         RETURNING id",
    )
    .bind(email)
    .bind(hash)
    .bind(handle)
    .fetch_one(pool)
    .await
}

#[tokio::test]
async fn friend_request_flow_covers_search_acceptance_and_deletion() -> Result<(), Box<dyn Error>> {
    let Some(pool) = obtain_pool().await else {
        eprintln!("Skipping friends tests: DATABASE_URL or TEST_DATABASE_URL not set");
        return Ok(());
    };
    let _guard = DB_LOCK.lock().await;
    ensure_friend_tables(&pool).await?;
    reset_tables(
        &pool,
        &["friend_requests", "friendships", "user_devices", "users"],
    )
    .await?;

    let secret = "secret";
    seed_user(&pool, "alice@example.com", "alice_handle").await?;
    seed_user(&pool, "bob@example.com", "bob_handle").await?;
    seed_user(&pool, "charlie@example.com", "charlie_handle").await?;

    let state = build_state(pool.clone(), secret, &[]);
    let app = test::init_service(App::new().app_data(state).configure(routes::configure)).await;
    let alice_token = make_token(secret, "alice@example.com", "alice_handle").expect("token");
    let bob_token = make_token(secret, "bob@example.com", "bob_handle").expect("token");

    let search_resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/friends/search?q=bo&limit=10")
            .insert_header(("Authorization", format!("Bearer {}", alice_token.clone())))
            .to_request(),
    )
    .await;
    assert_eq!(search_resp.status(), StatusCode::OK);
    let search_results: Vec<fiestaaa_back::models::FriendSearchResult> =
        test::read_body_json(search_resp).await;
    assert_eq!(search_results.len(), 1);
    assert_eq!(search_results[0].email, "bob@example.com");

    let create_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/friends/requests")
            .insert_header(("Authorization", format!("Bearer {}", alice_token.clone())))
            .set_json(&FriendRequestPayload {
                identifier: "bob_handle".into(),
            })
            .to_request(),
    )
    .await;
    assert_eq!(create_resp.status(), StatusCode::CREATED);
    let created: FriendRequest = test::read_body_json(create_resp).await;
    assert_eq!(created.status, "Pending");
    assert_eq!(created.sender_email, "alice@example.com");
    assert_eq!(created.receiver_email, "bob@example.com");

    let duplicate_resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/friends/requests")
            .insert_header(("Authorization", format!("Bearer {}", alice_token.clone())))
            .set_json(&FriendRequestPayload {
                identifier: "bob@example.com".into(),
            })
            .to_request(),
    )
    .await;
    assert_eq!(duplicate_resp.status(), StatusCode::CONFLICT);

    let requests_resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/friends/requests")
            .insert_header(("Authorization", format!("Bearer {}", bob_token.clone())))
            .to_request(),
    )
    .await;
    assert_eq!(requests_resp.status(), StatusCode::OK);
    let requests: Vec<FriendRequest> = test::read_body_json(requests_resp).await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].id, created.id);

    let accept_resp = test::call_service(
        &app,
        test::TestRequest::patch()
            .uri(&format!("/friends/requests/{}", created.id))
            .insert_header(("Authorization", format!("Bearer {}", bob_token.clone())))
            .set_json(&FriendRequestActionPayload {
                status: "Accepted".into(),
            })
            .to_request(),
    )
    .await;
    assert_eq!(accept_resp.status(), StatusCode::OK);
    let accepted: FriendRequest = test::read_body_json(accept_resp).await;
    assert_eq!(accepted.status, "Accepted");

    let friends_resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/me/friends")
            .insert_header(("Authorization", format!("Bearer {}", alice_token.clone())))
            .to_request(),
    )
    .await;
    assert_eq!(friends_resp.status(), StatusCode::OK);
    let friends: Vec<Friend> = test::read_body_json(friends_resp).await;
    assert_eq!(friends.len(), 1);
    assert_eq!(friends[0].email, "bob@example.com");

    let post_accept_search_resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/friends/search?q=bo&limit=10")
            .insert_header(("Authorization", format!("Bearer {}", alice_token.clone())))
            .to_request(),
    )
    .await;
    assert_eq!(post_accept_search_resp.status(), StatusCode::OK);
    let post_accept_results: Vec<fiestaaa_back::models::FriendSearchResult> =
        test::read_body_json(post_accept_search_resp).await;
    assert!(post_accept_results.is_empty());

    let delete_resp = test::call_service(
        &app,
        test::TestRequest::delete()
            .uri("/friends/bob_handle")
            .insert_header(("Authorization", format!("Bearer {}", alice_token)))
            .to_request(),
    )
    .await;
    assert_eq!(delete_resp.status(), StatusCode::OK);

    let remaining_friendships: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM friendships")
        .fetch_one(&pool)
        .await?;
    assert_eq!(remaining_friendships.0, 0);
    Ok(())
}
