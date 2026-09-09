//! The relay's two PostHog conversations: capture, and the fixed verification
//! read that turns "accepted" into "proven query-visible".
//!
//! ## Capture is reused, not reimplemented
//!
//! [`crate::audit::posthog::PostHogClient`] already speaks the public capture API
//! with the exact retry classification, the exact `uuid` deduplication, and the
//! exact secret hygiene this relay needs. Writing a second client would be a
//! second place for "a `2xx` means delivered" to creep back in.
//!
//! ## Verification is a batched, fixed query — never one request per event
//!
//! ```text
//! SELECT properties.event_id FROM events
//!  WHERE timestamp >= {window_start}
//!    AND properties.event_id IN ({event_id_0}, {event_id_1}, …)
//!  LIMIT {row_limit}
//! ```
//!
//! The placeholder NAMES are generated from an index, so — exactly as in
//! [`crate::operations::hogql`] — the query TEXT is a function of how MANY ids
//! are being checked and of nothing a caller ever supplied. The ids themselves
//! travel in the `values` map.
//!
//! Batching is a requirement rather than an optimization: one HogQL request per
//! event would make verifying a backlog cost more than producing it, and the
//! spec is explicit that verification is batched.
//!
//! Absence is NOT proof of loss — ingestion lag is real — which is why absence
//! only triggers a re-capture (with the same uuid, so PostHog deduplicates) after
//! `FKST_AUDIT_RELAY_VERIFICATION_MAX_AGE_SECS`, and why nothing here ever marks
//! a record delivered.

use std::collections::HashSet;

use k8s_openapi::chrono::{DateTime, Utc};
use serde_json::{json, Map, Value};

use crate::audit::event::{EVENT_NAME, INCOMPLETE_EVENT_NAME};
use crate::audit::lifecycle::LIFECYCLE_EVENT_NAME;
use crate::operations::posthog::PosthogQueryClient;
use crate::operations::SourceError;

/// A rendered verification query and its parameter map.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerificationQuery {
    pub query: String,
    pub values: Map<String, Value>,
}

impl VerificationQuery {
    /// The `POST /api/projects/{id}/query/` request body.
    pub fn request_body(&self) -> Value {
        json!({
            "query": {
                "kind": "HogQLQuery",
                "query": self.query,
                "values": Value::Object(self.values.clone()),
            }
        })
    }
}

/// Build the fixed query that asks PostHog which of `event_ids` are visible.
///
/// `window_start` bounds the scan: an event captured now cannot be older than
/// the record's own terminal instant, so a window anchored on the oldest id in
/// the batch keeps the query cheap without ever hiding a hit.
pub fn build_verification_query(
    event_ids: &[String],
    window_start: DateTime<Utc>,
) -> VerificationQuery {
    let mut values = Map::new();
    values.insert(
        "window_start".to_string(),
        json!(window_start.to_rfc3339_opts(k8s_openapi::chrono::SecondsFormat::Millis, true)),
    );
    values.insert("event_completed".to_string(), json!(EVENT_NAME));
    values.insert("event_incomplete".to_string(), json!(INCOMPLETE_EVENT_NAME));
    values.insert("event_lifecycle".to_string(), json!(LIFECYCLE_EVENT_NAME));
    let mut placeholders = Vec::with_capacity(event_ids.len());
    for (index, event_id) in event_ids.iter().enumerate() {
        // The NAME is derived from the index; the VALUE is the only thing that
        // varies with content.
        let name = format!("event_id_{index}");
        placeholders.push(format!("{{{name}}}"));
        values.insert(name, json!(event_id));
    }
    values.insert(
        "row_limit".to_string(),
        json!(event_ids.len().max(1) as u64),
    );

    let query = format!(
        "SELECT properties.event_id AS event_id FROM events \
         WHERE timestamp >= {{window_start}} \
         AND event IN ({{event_completed}}, {{event_incomplete}}, {{event_lifecycle}}) \
         AND properties.event_id IN ({}) LIMIT {{row_limit}}",
        placeholders.join(", ")
    );
    VerificationQuery { query, values }
}

/// Ask PostHog which of `event_ids` are query-visible.
///
/// Returns the visible subset. A failure is a [`SourceError`], never a partial
/// answer: treating an unreachable query as "none visible" would re-capture the
/// whole backlog every sweep.
pub async fn verify_visible(
    client: &PosthogQueryClient,
    event_ids: &[String],
    window_start: DateTime<Utc>,
) -> Result<HashSet<String>, SourceError> {
    if event_ids.is_empty() {
        return Ok(HashSet::new());
    }
    let built = build_verification_query(event_ids, window_start);
    let response = client.query(&built.request_body()).await?;
    let column = response
        .columns
        .iter()
        .position(|name| name == "event_id")
        .unwrap_or(0);
    let mut visible = HashSet::with_capacity(response.results.len());
    for row in &response.results {
        if let Some(Value::String(event_id)) = row.get(column) {
            visible.insert(event_id.clone());
        }
    }
    Ok(visible)
}

#[cfg(test)]
#[path = "posthog_tests.rs"]
mod tests;
