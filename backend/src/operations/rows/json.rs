//! Presenting a stored relay body through the shared [`super::RowCells`] view.
//!
//! The relay stores the wire bodies verbatim
//! ([`crate::audit_relay::protocol::RequestCompletionV1`] /
//! [`crate::audit_relay::protocol::LifecycleEventV1`]), which are STRUCTURED —
//! `actor` and `correlation` are nested objects — while the PostHog capture
//! projection also flattens those onto first-level properties for its fixed
//! HogQL. Rather than store a second, flattened copy in the relay (which would be
//! two encodings of one fact, free to drift), this adapter maps the flat property
//! NAMES the decoder asks for onto the stored body's paths.
//!
//! The mapping is a fixed, exhaustive table of compile-time constants. Nothing a
//! caller supplies reaches it, and a name the table does not know resolves to
//! `None` — which the typed row contract turns into either an absent optional or
//! a rejected row, never a guess.
//!
//! Two fields are SYNTHESIZED because they do not exist in the stored body:
//!
//! - `event` — the decoder dispatches on the audit event NAME, and the relay
//!   knows the record kind instead. The two request event names are distinguished
//!   by the record's own `outcome`, so a synthesized `incomplete` record decodes
//!   under exactly the name it was shipped to PostHog under;
//! - `row_timestamp` — the fallback sort instant, supplied from the row's
//!   `sort_timestamp` (its `terminal_at`).

use serde_json::Value;

use crate::audit::event::{AuditOutcome, EVENT_NAME, INCOMPLETE_EVENT_NAME};
use crate::audit::lifecycle::LIFECYCLE_EVENT_NAME;

use super::RowCells;

/// The relay record kinds, as the read API spells them.
const KIND_API_REQUEST: &str = "api_request";

/// A stored relay body plus the two synthesized cells.
pub struct JsonRowView<'a> {
    body: &'a Value,
    event: Value,
    row_timestamp: Value,
}

impl<'a> JsonRowView<'a> {
    /// Bind a stored body to its record kind and sort instant.
    pub fn new(body: &'a Value, record_kind: &str, sort_timestamp: &str) -> Self {
        Self {
            body,
            event: Value::String(event_name(body, record_kind).to_string()),
            row_timestamp: Value::String(sort_timestamp.to_string()),
        }
    }

    /// Follow a `.`-separated path into the stored body.
    fn path(&self, path: &str) -> Option<&'a Value> {
        let mut current = self.body;
        for segment in path.split('.') {
            current = current.get(segment)?;
        }
        (!current.is_null()).then_some(current)
    }
}

impl RowCells for JsonRowView<'_> {
    fn cell(&self, column: &str) -> Option<&Value> {
        match column {
            "event" => Some(&self.event),
            "row_timestamp" => Some(&self.row_timestamp),
            // Nested on the wire, flat in the decoder's vocabulary.
            "actor_kind" => self.path("actor.kind"),
            "actor_login" => self.path("actor.login"),
            "principal_kind" => self.path("principal.kind"),
            "principal_id" => self.path("principal.id"),
            "repo_full_name" => self.path("correlation.repo_full_name"),
            "installation_id" => self.path("correlation.installation_id"),
            "trigger_issue" => self.path("correlation.trigger_issue"),
            "webhook_delivery_id" => self.path("correlation.webhook_delivery_id"),
            // The runtime's creation instant is `runtime_created_at` on the
            // lifecycle wire and `created_at` in the decoder's vocabulary.
            "created_at" => self.path("runtime_created_at"),
            // Canonical first, nested as the fallback: an API record carries the
            // top-level canonical id (null for every non-human actor), while a
            // lifecycle record only has the nested one.
            "actor_id" => self.path("actor_id").or_else(|| self.path("actor.id")),
            // Top-level on an API record, correlated on a lifecycle one.
            "request_id" => self
                .path("request_id")
                .or_else(|| self.path("correlation.request_id")),
            // Everything else is already a first-level field of the stored body.
            other => self.path(other),
        }
    }
}

/// Which audit event name a stored body decodes under.
///
/// A request record whose `outcome` is `incomplete` was shipped to PostHog as
/// [`INCOMPLETE_EVENT_NAME`], so it must decode under that name here too —
/// otherwise the relay copy and the PostHog copy of one event id would look like
/// two different contracts to the merge.
fn event_name(body: &Value, record_kind: &str) -> &'static str {
    if record_kind != KIND_API_REQUEST {
        return LIFECYCLE_EVENT_NAME;
    }
    let incomplete = body
        .get("outcome")
        .and_then(Value::as_str)
        .is_some_and(|outcome| outcome == AuditOutcome::Incomplete.as_str());
    if incomplete {
        INCOMPLETE_EVENT_NAME
    } else {
        EVENT_NAME
    }
}

#[cfg(test)]
#[path = "json_tests.rs"]
mod tests;
