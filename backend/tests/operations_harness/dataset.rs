//! The planted dataset, and the PostHog stand-in that applies the request's own
//! predicates to it.
//!
//! A mock that returned rows regardless of the predicate would let a broken
//! source query pass every test. This one HONOURS the predicate — the viewer id,
//! the authorized session, the event allowlist, the keyset cursor, and the page
//! limit — so a page assembled from a dataset containing hidden rows is
//! byte-identical to one assembled from a dataset that never had them.

use serde_json::{json, Value};
use wiremock::{Request as MockRequest, Respond, ResponseTemplate};

/// One planted event, as the dataset holds it.
#[derive(Clone, Debug)]
pub struct Row {
    pub event: &'static str,
    pub event_id: String,
    pub timestamp: String,
    /// `None` for a system/anonymous row.
    pub actor_id: Option<i64>,
    pub session_id: Option<String>,
    /// Set to make the row fail the typed row contract.
    pub malformed: bool,
}

impl Row {
    /// An API-request row owned by `actor_id`.
    pub fn api(event_id: &str, actor_id: i64, timestamp: &str) -> Self {
        Self {
            event: fkst_control_plane::audit::event::EVENT_NAME,
            event_id: event_id.to_string(),
            timestamp: timestamp.to_string(),
            actor_id: Some(actor_id),
            session_id: None,
            malformed: false,
        }
    }

    /// An unattributed row: no verified actor, so no regular caller owns it.
    pub fn anonymous(event_id: &str, timestamp: &str) -> Self {
        Self {
            actor_id: None,
            ..Self::api(event_id, 0, timestamp)
        }
    }

    /// A system sandbox lifecycle row for `session_id`.
    pub fn lifecycle(event_id: &str, session_id: &str, timestamp: &str) -> Self {
        Self {
            event: fkst_control_plane::audit::lifecycle::LIFECYCLE_EVENT_NAME,
            event_id: event_id.to_string(),
            timestamp: timestamp.to_string(),
            actor_id: None,
            session_id: Some(session_id.to_string()),
            malformed: false,
        }
    }

    /// The same row, correlated to a session.
    pub fn in_session(mut self, session_id: &str) -> Self {
        self.session_id = Some(session_id.to_string());
        self
    }

    /// The same row, with a value the typed row contract must reject.
    pub fn malformed(mut self) -> Self {
        self.malformed = true;
        self
    }
}

/// The column list the responder answers with, in a deliberately NON-select
/// order so the decoder's name addressing is exercised on every request.
pub const COLUMNS: [&str; 11] = [
    "operation_id",
    "event",
    "event_id",
    "row_timestamp",
    "actor_id",
    "session_id",
    "method",
    "route_template",
    "outcome",
    "lifecycle_action",
    "backend",
];

/// A PostHog stand-in that applies the request's own predicates.
pub struct PredicateAwareQuery {
    rows: Vec<Row>,
}

impl PredicateAwareQuery {
    /// A stand-in holding `rows`.
    pub fn new(rows: Vec<Row>) -> Self {
        Self { rows }
    }
}

impl Respond for PredicateAwareQuery {
    fn respond(&self, request: &MockRequest) -> ResponseTemplate {
        let body: Value = match serde_json::from_slice(&request.body) {
            Ok(body) => body,
            Err(_) => return ResponseTemplate::new(400),
        };
        let values = &body["query"]["values"];
        let viewer = values["viewer_actor_id"].as_i64();
        let authorized_session = values["authorized_session_id"].as_str();
        let lifecycle_event = fkst_control_plane::audit::lifecycle::LIFECYCLE_EVENT_NAME;
        let kind_is_lifecycle_only = body["query"]["query"]
            .as_str()
            .is_some_and(|text| !text.contains("event IN ("));
        let kind_is_api_only = body["query"]["query"]
            .as_str()
            .is_some_and(|text| !text.contains("event = {event_sandbox_lifecycle}"));

        let cursor_timestamp = values["cursor_timestamp"].as_str();
        let cursor_event_id = values["cursor_event_id"].as_str();
        let page_limit = values["page_limit"].as_u64().unwrap_or(u64::MAX) as usize;

        let mut matched: Vec<&Row> = self
            .rows
            .iter()
            .filter(|row| {
                let is_lifecycle = row.event == lifecycle_event;
                if is_lifecycle && kind_is_api_only {
                    return false;
                }
                if !is_lifecycle && kind_is_lifecycle_only {
                    return false;
                }
                if is_lifecycle {
                    // The lifecycle branch is reachable only through the
                    // authorized session (or, for an admin, not at all bound).
                    return authorized_session
                        .is_none_or(|session| row.session_id.as_deref() == Some(session));
                }
                // The API branch carries the viewer predicate and, when the query
                // asked for both kinds, the authorized session too.
                if let Some(viewer) = viewer {
                    if row.actor_id != Some(viewer) {
                        return false;
                    }
                    if let Some(session) = authorized_session {
                        if row.session_id.as_deref() != Some(session) {
                            return false;
                        }
                    }
                }
                true
            })
            .collect();
        // The source's OWN ordering, keyset predicate, and limit — applied here
        // so the fixture behaves like a real source rather than handing the
        // process a page it never asked for.
        matched.sort_by(|left, right| {
            right
                .timestamp
                .cmp(&left.timestamp)
                .then_with(|| right.event_id.cmp(&left.event_id))
        });
        if let (Some(timestamp), Some(event_id)) = (cursor_timestamp, cursor_event_id) {
            matched.retain(|row| {
                row.timestamp.as_str() < timestamp
                    || (row.timestamp == timestamp && row.event_id.as_str() < event_id)
            });
        }
        matched.truncate(page_limit);
        let visible: Vec<Value> = matched.into_iter().map(render_row).collect();
        ResponseTemplate::new(200).set_body_json(json!({
            "columns": COLUMNS,
            "results": visible,
        }))
    }
}

fn render_row(row: &Row) -> Value {
    json!([
        "canvas_overview",
        row.event,
        row.event_id,
        row.timestamp,
        row.actor_id,
        row.session_id,
        // A malformed row carries a numeric `method`, which the typed row
        // contract rejects — and which no repair may invent a value for.
        if row.malformed {
            json!(42)
        } else {
            json!("GET")
        },
        "/api/v1/overview",
        "success",
        // Only a lifecycle row carries these; a `null` on an API row is exactly
        // what the tagged union exists to keep out of the response.
        if row.event == fkst_control_plane::audit::lifecycle::LIFECYCLE_EVENT_NAME {
            json!("created")
        } else {
            Value::Null
        },
        if row.event == fkst_control_plane::audit::lifecycle::LIFECYCLE_EVENT_NAME {
            json!("kubernetes")
        } else {
            Value::Null
        },
    ])
}
