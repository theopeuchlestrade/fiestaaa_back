use super::*;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::AssertSqlSafe;

#[derive(Debug, Default, Deserialize, utoipa::IntoParams)]
pub struct EventListQuery {
    pub limit: Option<i64>,
    pub cursor: Option<String>,
    /// Literal case-insensitive substring of the event name.
    pub q: Option<String>,
    /// upcoming (includes ongoing), invitations, owned, past, or all.
    pub view: Option<String>,
    /// start_asc or start_desc. New criteria opt in to chronological cursors.
    pub sort: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct EventCursor {
    version: u8,
    start: DateTime<Utc>,
    id: i64,
    criteria: String,
}

#[derive(Debug)]
pub(super) struct EventSearch {
    q: String,
    view: String,
    sort: String,
    limit: i64,
    cursor: Option<EventCursor>,
}

fn invalid(code: &str) -> HttpResponse {
    HttpResponse::BadRequest().json(ErrorResponse {
        error: code.into(),
        details: None,
    })
}

impl EventSearch {
    pub fn parse(query: &EventListQuery) -> Result<Option<Self>, HttpResponse> {
        if query.q.is_none() && query.view.is_none() && query.sort.is_none() {
            return Ok(None);
        }
        let view = query.view.as_deref().unwrap_or("upcoming");
        let sort = query.sort.as_deref().unwrap_or(if view == "past" {
            "start_desc"
        } else {
            "start_asc"
        });
        let limit = query.limit.unwrap_or(50);
        if !["upcoming", "invitations", "owned", "past", "all"].contains(&view)
            || !["start_asc", "start_desc"].contains(&sort)
            || !(1..=100).contains(&limit)
        {
            return Err(invalid("invalid_event_query"));
        }
        let q = query.q.as_deref().unwrap_or("").trim().to_lowercase();
        if q.chars().count() > 200 {
            return Err(invalid("invalid_event_query"));
        }
        let mut search = Self {
            q,
            view: view.into(),
            sort: sort.into(),
            limit,
            cursor: None,
        };
        if let Some(raw) = &query.cursor {
            let decoded: EventCursor =
                serde_json::from_str(raw).map_err(|_| invalid("invalid_cursor"))?;
            if decoded.version != 1 || decoded.id <= 0 || decoded.criteria != search.criteria() {
                return Err(invalid("invalid_cursor"));
            }
            search.cursor = Some(decoded);
        }
        Ok(Some(search))
    }

    fn criteria(&self) -> String {
        sha256_hex(
            &serde_json::to_string(&(&self.q, &self.view, &self.sort)).expect("string tuple"),
        )
    }

    pub async fn fetch(&self, state: &AppState, user_id: i64, from: &str) -> HttpResponse {
        let direction = if self.sort == "start_desc" {
            "DESC"
        } else {
            "ASC"
        };
        let comparator = if self.sort == "start_desc" { "<" } else { ">" };
        let filter = match self.view.as_str() {
            "upcoming" => "AND e.effective_ends_at >= NOW()",
            "past" => "AND e.effective_ends_at < NOW()",
            "owned" => "AND e.owner_user_id = $1",
            "invitations" => {
                "AND e.effective_ends_at >= NOW() AND EXISTS (SELECT 1 FROM invitations pending WHERE pending.event_id = e.event_id AND pending.user_id = $1 AND pending.status = 'Waiting' AND (e.invitation_deadline IS NULL OR CURRENT_DATE <= e.invitation_deadline))"
            }
            _ => "",
        };
        // strpos treats %, _ and backslashes literally, unlike LIKE patterns.
        let suffix = format!(
            "{from} {filter}
            AND strpos(lower(e.name_event), $2) > 0
            AND ($3::timestamptz IS NULL OR (e.starts_at, e.event_id) {comparator} ($3, $4))
            ORDER BY e.starts_at {direction}, e.event_id {direction} LIMIT $5"
        );
        let sql = select_events_sql(&suffix);
        let result = sqlx::query_as::<_, Event>(AssertSqlSafe(sql))
            .bind(user_id)
            .bind(&self.q)
            .bind(self.cursor.as_ref().map(|c| c.start))
            .bind(self.cursor.as_ref().map(|c| c.id).unwrap_or(0))
            .bind(self.limit + 1)
            .fetch_all(&state.db)
            .await;
        match result {
            Ok(events) => json_page(events, self.limit, |event| {
                serde_json::to_string(&EventCursor {
                    version: 1,
                    start: event.start_at,
                    id: event.event_id,
                    criteria: self.criteria(),
                })
                .expect("event cursor")
            }),
            Err(_) => server_error(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_and_new_defaults_are_distinct() {
        assert!(
            EventSearch::parse(&EventListQuery::default())
                .unwrap()
                .is_none()
        );
        let search = EventSearch::parse(&EventListQuery {
            view: Some("past".into()),
            ..Default::default()
        })
        .unwrap()
        .unwrap();
        assert_eq!(search.sort, "start_desc");
        assert_eq!(search.limit, 50);
    }
    #[test]
    fn cursors_are_bound_to_normalized_criteria() {
        let mut query = EventListQuery {
            q: Some("  Party%_  ".into()),
            view: Some("upcoming".into()),
            ..Default::default()
        };
        let search = EventSearch::parse(&query).unwrap().unwrap();
        assert_eq!(search.q, "party%_");
        query.cursor = Some(
            serde_json::to_string(&EventCursor {
                version: 1,
                start: Utc::now(),
                id: 3,
                criteria: search.criteria(),
            })
            .unwrap(),
        );
        assert!(EventSearch::parse(&query).is_ok());
        query.view = Some("past".into());
        assert!(EventSearch::parse(&query).is_err());
        query.cursor = Some("3".into());
        assert!(EventSearch::parse(&query).is_err());
    }
    #[test]
    fn rejects_invalid_criteria() {
        for query in [
            EventListQuery {
                view: Some("unknown".into()),
                ..Default::default()
            },
            EventListQuery {
                sort: Some("name".into()),
                ..Default::default()
            },
            EventListQuery {
                view: Some("owned".into()),
                limit: Some(101),
                ..Default::default()
            },
        ] {
            assert!(EventSearch::parse(&query).is_err());
        }
    }
}
