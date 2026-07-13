use actix_web::HttpResponse;
use serde::Deserialize;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::models::ErrorResponse;

pub const MAX_PAGE_SIZE: i64 = 100;
static ENFORCE_DEFAULTS: AtomicBool = AtomicBool::new(false);

pub fn configure_defaults(enforce: bool) {
    ENFORCE_DEFAULTS.store(enforce, Ordering::Relaxed);
}

#[derive(Debug, Default, Deserialize, utoipa::IntoParams)]
pub struct PaginationQuery {
    /// Number of items to return. When omitted with cursor, the default is 50.
    pub limit: Option<i64>,
    /// Opaque cursor returned by the previous response.
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct PageRequest {
    pub limit: i64,
    pub after_id: Option<i64>,
}

pub fn page_request(query: &PaginationQuery) -> Result<Option<PageRequest>, HttpResponse> {
    if query.limit.is_none() && query.cursor.is_none() && !ENFORCE_DEFAULTS.load(Ordering::Relaxed)
    {
        return Ok(None);
    }
    let limit = query.limit.unwrap_or(50);
    if !(1..=MAX_PAGE_SIZE).contains(&limit) {
        return Err(HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_pagination".into(),
            details: None,
        }));
    }
    let after_id = match query.cursor.as_deref() {
        Some(value) => Some(value.parse::<i64>().map_err(|_| {
            HttpResponse::BadRequest().json(ErrorResponse {
                error: "invalid_cursor".into(),
                details: None,
            })
        })?),
        None => None,
    };
    Ok(Some(PageRequest { limit, after_id }))
}

pub fn json_page<T: serde::Serialize>(
    items: Vec<T>,
    limit: i64,
    next_cursor: impl FnOnce(&T) -> String,
) -> HttpResponse {
    let has_more = items.len() as i64 == limit;
    let mut response = HttpResponse::Ok();
    if has_more && let Some(last) = items.last() {
        response.insert_header(("X-Next-Cursor", next_cursor(last)));
    }
    response.json(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_request_stays_unpaginated() {
        assert!(page_request(&PaginationQuery::default()).unwrap().is_none());
    }

    #[test]
    fn rejects_invalid_cursor_and_limit() {
        assert!(
            page_request(&PaginationQuery {
                limit: Some(101),
                cursor: None,
            })
            .is_err()
        );
        assert!(
            page_request(&PaginationQuery {
                limit: Some(10),
                cursor: Some("not-a-cursor".into()),
            })
            .is_err()
        );
    }
}
