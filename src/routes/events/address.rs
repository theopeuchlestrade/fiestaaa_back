use actix_web::{Responder, get, web};

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
        (status = 400, description = "Query too short", body = ErrorResponse),
        (status = 401, description = "Authentication required", body = ErrorResponse),
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
    let limit = params.limit.unwrap_or(5).clamp(1, 10);

    match fetch_address_suggestions(
        &state.http_client,
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

    let response = client.get(url).send().await.map_err(|_| {
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

    let places: Vec<NominatimPlace> = response.json().await.map_err(|_| {
        HttpResponse::BadGateway().json(ErrorResponse {
            error: "geocoding_parse_error".into(),
            details: None,
        })
    })?;

    let suggestions = places
        .into_iter()
        .filter_map(|place| {
            let lat = place.lat.parse::<f64>().ok()?;
            let lon = place.lon.parse::<f64>().ok()?;
            Some(AddressSuggestion {
                label: place.display_name,
                latitude: lat,
                longitude: lon,
            })
        })
        .collect();

    Ok(suggestions)
}
