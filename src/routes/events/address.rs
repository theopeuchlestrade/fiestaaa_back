use actix_web::{Responder, get, web};
use redis::AsyncCommands;
use serde::Serialize;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::*;

#[derive(Deserialize)]
pub struct AddressSearchQuery {
    pub q: String,
    pub limit: Option<u8>,
}

#[utoipa::path(
    get,
    path = "/geo/address-search",
    tag = "events",
    params(
        ("q" = String, Query, description = "Address or place to search"),
        ("limit" = u8, Query, description = "Maximum number of suggestions (1-10)")
    ),
    responses(
        (status = 200, description = "Geocoded suggestions", body = [AddressSuggestion]),
        (status = 400, description = "Invalid query length", body = ErrorResponse),
        (status = 401, description = "Authentication required", body = ErrorResponse),
        (status = 429, description = "Shared geocoding capacity busy", body = ErrorResponse),
        (status = 502, description = "Geocoding service unavailable", body = ErrorResponse)
    )
)]
#[get("/geo/address-search")]
pub async fn search_address(
    state: web::Data<AppState>,
    req: HttpRequest,
    params: web::Query<AddressSearchQuery>,
) -> impl Responder {
    if let Err(resp) = claims_email(&req, state.get_ref()).await {
        return resp;
    }

    let query = params.q.trim();
    if query.len() < 3 {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "query_too_short".into(),
            details: Some("Au moins 3 caractères requis pour la recherche".into()),
        });
    }
    if query.len() > 256 {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "query_too_long".into(),
            details: None,
        });
    }
    let limit = params.limit.unwrap_or(5).clamp(1, 10);

    match cached_address_suggestions(
        state.redis_client.as_ref(),
        &state.geocoding_http_client,
        &state.geocoding_base_url,
        state.geocoding_country_codes.as_deref(),
        query,
        limit,
    )
    .await
    {
        Ok(results) => HttpResponse::Ok().json(results),
        Err(resp) => resp,
    }
}

const GATE_KEY: &str = "fiestaaa:geocoding:gate:v1";
const CACHE_KEY: &str = "fiestaaa:geocoding:cache:v1";
const CACHE_ORDER: &str = "fiestaaa:geocoding:order:v1";
const CACHE_SEQUENCE: &str = "fiestaaa:geocoding:sequence:v1";
const CACHE_SECONDS: u64 = 86400;

#[derive(Serialize, Deserialize)]
struct CachedSuggestions {
    created_at: u64,
    results: Vec<AddressSuggestion>,
}

fn unavailable() -> HttpResponse {
    HttpResponse::BadGateway().json(ErrorResponse {
        error: "geocoding_unreachable".into(),
        details: None,
    })
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

async fn cached_address_suggestions(
    redis_client: Option<&redis::Client>,
    client: &reqwest::Client,
    base_url: &str,
    country_codes: Option<&str>,
    query: &str,
    limit: u8,
) -> Result<Vec<AddressSuggestion>, HttpResponse> {
    // A shared gate is mandatory: never fall back to one limiter per API worker.
    let client_redis = redis_client.ok_or_else(unavailable)?;
    let mut conn = tokio::time::timeout(
        Duration::from_secs(2),
        client_redis.get_multiplexed_async_connection(),
    )
    .await
    .map_err(|_| unavailable())?
    .map_err(|_| unavailable())?;
    // Only the digest reaches Redis keys; search terms are not logged.
    let digest = sha256_hex(
        &serde_json::to_string(&(base_url, country_codes, query, limit))
            .map_err(|_| unavailable())?,
    );
    let key = format!("{CACHE_KEY}:{digest}");
    let cached: Option<String> = tokio::time::timeout(Duration::from_secs(2), conn.get(&key))
        .await
        .map_err(|_| unavailable())?
        .map_err(|_| unavailable())?;
    let now = unix_seconds();
    if let Some(cached) = cached
        && let Ok(entry) = serde_json::from_str::<CachedSuggestions>(&cached)
        && now >= entry.created_at
        && now - entry.created_at < CACHE_SECONDS
    {
        return Ok(entry.results);
    }
    let token = uuid::Uuid::new_v4().to_string();
    let acquired: Option<String> = tokio::time::timeout(
        Duration::from_secs(2),
        redis::cmd("SET")
            .arg(GATE_KEY)
            .arg(&token)
            .arg("NX")
            .arg("PX")
            .arg(30000)
            .query_async(&mut conn),
    )
    .await
    .map_err(|_| unavailable())?
    .map_err(|_| unavailable())?;
    if acquired.is_none() {
        return Err(HttpResponse::TooManyRequests()
            .insert_header(("Retry-After", "1"))
            .json(ErrorResponse {
                error: "geocoding_busy".into(),
                details: None,
            }));
    }
    // Hold the lease while the bounded request runs, then retain a full second
    // of cooldown. Cancellation leaves the longer lease intact and fails closed.
    let result = fetch_address_suggestions(client, base_url, country_codes, query, limit).await;
    let release = redis::Script::new(
        "if redis.call('GET', KEYS[1]) == ARGV[1] then
        redis.call('PEXPIRE', KEYS[1], 1000); return 1 end; return 0",
    );
    let renewed: i32 = tokio::time::timeout(
        Duration::from_secs(2),
        release.key(GATE_KEY).arg(&token).invoke_async(&mut conn),
    )
    .await
    .map_err(|_| unavailable())?
    .map_err(|_| unavailable())?;
    if renewed != 1 {
        return Err(unavailable());
    }
    let results = result?;
    let value = serde_json::to_string(&CachedSuggestions {
        created_at: unix_seconds(),
        results,
    })
    .map_err(|_| unavailable())?;
    store_cache(&mut conn, &key, &value).await?;
    Ok(serde_json::from_str::<CachedSuggestions>(&value)
        .map_err(|_| unavailable())?
        .results)
}

async fn store_cache(
    conn: &mut redis::aio::MultiplexedConnection,
    key: &str,
    value: &str,
) -> Result<(), HttpResponse> {
    // At most 128 response entries, each physically expires after 24 hours.
    let cache = redis::Script::new(
        "local seq = redis.call('INCR', KEYS[3])
        redis.call('SET', KEYS[1], ARGV[1], 'EX', 86400)
        redis.call('ZADD', KEYS[2], seq, KEYS[1])
        local old = redis.call('ZRANGE', KEYS[2], 0, -129)
        for _, field in ipairs(old) do
            redis.call('DEL', field); redis.call('ZREM', KEYS[2], field)
        end
        redis.call('EXPIRE', KEYS[2], 86400); redis.call('EXPIRE', KEYS[3], 86400)
        return 1",
    );
    let _: i32 = tokio::time::timeout(
        Duration::from_secs(2),
        cache
            .key(key)
            .key(CACHE_ORDER)
            .key(CACHE_SEQUENCE)
            .arg(value)
            .invoke_async(conn),
    )
    .await
    .map_err(|_| unavailable())?
    .map_err(|_| unavailable())?;
    Ok(())
}

async fn fetch_address_suggestions(
    client: &reqwest::Client,
    base_url: &str,
    country_codes: Option<&str>,
    query: &str,
    limit: u8,
) -> Result<Vec<AddressSuggestion>, HttpResponse> {
    let mut url = match reqwest::Url::parse(&format!("{}/search", base_url.trim_end_matches('/'))) {
        Ok(url) => url,
        Err(_) => {
            return Err(HttpResponse::InternalServerError().json(ErrorResponse {
                error: "geocoding_config_error".into(),
                details: None,
            }));
        }
    };

    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair("format", "jsonv2");
        pairs.append_pair("addressdetails", "0");
        pairs.append_pair("limit", &limit.to_string());
        pairs.append_pair("q", query);
        if let Some(cc) = country_codes {
            pairs.append_pair("countrycodes", cc);
        }
    }

    #[derive(Deserialize)]
    struct NominatimPlace {
        display_name: String,
        lat: String,
        lon: String,
    }

    let mut response = client
        .get(url)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|_| {
            HttpResponse::BadGateway().json(ErrorResponse {
                error: "geocoding_unreachable".into(),
                details: None,
            })
        })?;

    if !response.status().is_success() {
        return Err(HttpResponse::BadGateway().json(ErrorResponse {
            error: "geocoding_error".into(),
            details: Some(format!("Status: {}", response.status())),
        }));
    }

    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| unavailable())? {
        if body.len() + chunk.len() > 65536 {
            return Err(unavailable());
        }
        body.extend_from_slice(&chunk);
    }
    let places: Vec<NominatimPlace> = serde_json::from_slice(&body).map_err(|_| {
        HttpResponse::BadGateway().json(ErrorResponse {
            error: "geocoding_parse_error".into(),
            details: None,
        })
    })?;

    let suggestions = places
        .into_iter()
        .take(usize::from(limit))
        .filter_map(|place| {
            let lat = place.lat.parse::<f64>().ok()?;
            let lon = place.lon.parse::<f64>().ok()?;
            if !lat.is_finite()
                || !lon.is_finite()
                || !(-90.0..=90.0).contains(&lat)
                || !(-180.0..=180.0).contains(&lon)
            {
                return None;
            }
            Some(AddressSuggestion {
                label: place.display_name,
                latitude: lat,
                longitude: lon,
            })
        })
        .collect();

    Ok(suggestions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::{App, HttpServer};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    static REDIS_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    async fn fixture() -> (String, Arc<AtomicUsize>, actix_web::dev::ServerHandle) {
        let count = Arc::new(AtomicUsize::new(0));
        let inner = count.clone();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = HttpServer::new(move || {
            let count = inner.clone();
            App::new().app_data(web::Data::new(count)).route("/search", web::get().to(
                |count: web::Data<Arc<AtomicUsize>>, q: web::Query<AddressSearchQuery>| async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    if q.q == "slow" { tokio::time::sleep(Duration::from_millis(100)).await; }
                    if q.q == "redirect" { return HttpResponse::Found().insert_header(("Location", "/search?q=Paris")).finish(); }
                    if q.q == "huge" { return HttpResponse::Ok().body("x".repeat(70000)); }
                    if q.q == "failure" { return HttpResponse::ServiceUnavailable().finish(); }
                    HttpResponse::Ok().json(serde_json::json!([
                        {"display_name":"Synthetic test place","lat":"48.0","lon":"2.0"}
                    ]))
                }))
        })
        .workers(1)
        .listen(listener)
        .unwrap()
        .run();
        let handle = server.handle();
        actix_web::rt::spawn(server);
        (url, count, handle)
    }

    async fn redis_fixture() -> Option<redis::Client> {
        let Ok(url) = std::env::var("TEST_REDIS_URL") else {
            assert!(
                std::env::var("CI").is_err(),
                "CI requires isolated TEST_REDIS_URL"
            );
            eprintln!("Skipping Redis geocoding test: set isolated TEST_REDIS_URL");
            return None;
        };
        let client = redis::Client::open(url).unwrap();
        let mut conn = client.get_multiplexed_async_connection().await.unwrap();
        // TEST_REDIS_URL is exclusively a disposable test service, never production.
        let keys: Vec<String> = conn.keys("fiestaaa:geocoding:*").await.unwrap();
        if !keys.is_empty() {
            let _: usize = conn.del(keys).await.unwrap();
        }
        Some(client)
    }

    #[actix_web::test]
    async fn redirect_is_not_followed_and_oversized_body_is_rejected() {
        let (url, count, server) = fixture().await;
        let http = crate::build_geocoding_http_client("Fiestaaa-tests");
        assert_eq!(
            fetch_address_suggestions(&http, &url, None, "redirect", 5)
                .await
                .unwrap_err()
                .status(),
            502
        );
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert_eq!(
            fetch_address_suggestions(&http, &url, None, "huge", 5)
                .await
                .unwrap_err()
                .status(),
            502
        );
        server.stop(true).await;
    }

    #[actix_web::test]
    async fn missing_or_unavailable_redis_never_contacts_provider() {
        let (url, count, server) = fixture().await;
        let http = crate::build_geocoding_http_client("Fiestaaa-tests");
        assert_eq!(
            cached_address_suggestions(None, &http, &url, None, "Paris", 5)
                .await
                .unwrap_err()
                .status(),
            502
        );
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let client = redis::Client::open(format!("redis://127.0.0.1:{port}")).unwrap();
        assert_eq!(
            cached_address_suggestions(Some(&client), &http, &url, None, "Paris", 5)
                .await
                .unwrap_err()
                .status(),
            502
        );
        assert_eq!(count.load(Ordering::SeqCst), 0);
        server.stop(true).await;
    }

    #[actix_web::test]
    async fn shared_gate_cache_and_failure_cooldown() {
        let _guard = REDIS_LOCK.lock().await;
        let Some(redis) = redis_fixture().await else {
            return;
        };
        let other_worker = redis.clone();
        let (url, count, server) = fixture().await;
        let http = crate::build_geocoding_http_client("Fiestaaa-tests");
        let (first, second) = tokio::join!(
            cached_address_suggestions(Some(&redis), &http, &url, None, "slow", 5),
            cached_address_suggestions(Some(&other_worker), &http, &url, None, "other", 5)
        );
        assert!(first.is_ok() ^ second.is_ok());
        let busy = if first.is_ok() {
            second.unwrap_err()
        } else {
            first.unwrap_err()
        };
        assert_eq!(busy.status(), 429);
        assert_eq!(busy.headers().get("Retry-After").unwrap(), "1");
        assert_eq!(count.load(Ordering::SeqCst), 1);
        let winning_query =
            if cached_address_suggestions(Some(&redis), &http, &url, None, "slow", 5)
                .await
                .is_ok()
            {
                "slow"
            } else {
                "other"
            };
        assert!(
            cached_address_suggestions(Some(&redis), &http, &url, None, winning_query, 5)
                .await
                .is_ok()
        );
        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "cached response bypasses provider even during cooldown"
        );
        tokio::time::sleep(Duration::from_millis(1100)).await;
        assert_eq!(
            cached_address_suggestions(Some(&redis), &http, &url, None, "failure", 5)
                .await
                .unwrap_err()
                .status(),
            502
        );
        assert_eq!(
            cached_address_suggestions(Some(&redis), &http, &url, None, "new", 5)
                .await
                .unwrap_err()
                .status(),
            429
        );
        assert_eq!(count.load(Ordering::SeqCst), 2);
        server.stop(true).await;
    }

    #[actix_web::test]
    async fn cache_is_bounded_and_entries_have_individual_expiry() {
        let _guard = REDIS_LOCK.lock().await;
        let Some(redis) = redis_fixture().await else {
            return;
        };
        let mut conn = redis.get_multiplexed_async_connection().await.unwrap();
        for i in 0..129 {
            store_cache(&mut conn, &format!("{CACHE_KEY}:synthetic-{i}"), "{}")
                .await
                .unwrap();
        }
        let count: usize = conn.zcard(CACHE_ORDER).await.unwrap();
        assert_eq!(count, 128);
        let oldest: bool = conn
            .exists(format!("{CACHE_KEY}:synthetic-0"))
            .await
            .unwrap();
        assert!(!oldest);
        let ttl: i64 = conn
            .ttl(format!("{CACHE_KEY}:synthetic-128"))
            .await
            .unwrap();
        assert!((86390..=86400).contains(&ttl));
    }
}
