//! The relay implementation of [`ActivitySource`] — the slot issue #5672
//! reserved.
//!
//! ```text
//! SourceQuery (typed ActivityVisibilityConstraint)
//!   -> RecordsQueryV1              the same constraint, on the internal wire
//!        -> relay SQL              predicate BEFORE LIMIT, on a scoped index
//!   -> already-authorized ActivityRecord candidates
//! ```
//!
//! ## The constraint is carried, never re-derived
//!
//! [`crate::audit_relay::query::RelayScopeV1::from_constraint`] is the ONLY way
//! this module produces a scope, and it consumes the sealed
//! [`crate::session_access::ActivityVisibilityConstraint`]. There is no field on
//! this path a caller could set to widen the query, and the relay refuses a
//! `mine` scope that arrives without its actor id — so a regression here fails
//! closed rather than open.
//!
//! ## What the relay contributes to a merged page
//!
//! Rows PostHog cannot answer for yet or at all: `complete` (captured but not
//! accepted), `posthog_accepted` (accepted, not proven visible), `incomplete`
//! (the request never finished), and `dead_letter` (delivery gave up). Verified
//! rows may also come back inside the retention overlap; the merge prefers
//! PostHog's content for those and keeps the more severe delivery state, so a
//! stuck delivery is never hidden by a healthy copy.
//!
//! A `started` row is never returned by the relay at all — it has no terminal
//! projection, and no source may invent an outcome.

use std::time::Duration;

use async_trait::async_trait;

use crate::audit::relay::AuditRelayClient;
use crate::audit_relay::query::{RecordRowV1, RecordsQueryV1, RelayScopeV1};

use super::record::{ActivityRecord, ActivitySourceKind, DeliveryState};
use super::rows::{self, RowError};
use super::source::{ActivitySource, SourceError, SourcePage, SourceQuery};

/// The relay-backed activity source.
#[derive(Debug)]
pub struct RelayActivitySource {
    client: std::sync::Arc<AuditRelayClient>,
    timeout: Duration,
}

impl RelayActivitySource {
    pub fn new(client: std::sync::Arc<AuditRelayClient>, timeout: Duration) -> Self {
        Self { client, timeout }
    }

    /// Project one source query onto the internal read wire.
    ///
    /// Split out so a test can assert the mandatory predicate travels, without
    /// standing up an HTTP server to observe it.
    pub fn build_query(query: &SourceQuery) -> RecordsQueryV1 {
        let scope = RelayScopeV1::from_constraint(&query.constraint);
        let filters = &query.filters;
        let status_bounds = filters.status_class.map(|class| class.bounds());
        RecordsQueryV1 {
            scope: scope.scope,
            actor_id: scope.actor_id,
            lifecycle_session_id: scope.lifecycle_session_id,
            record_kind: query.record_kind.as_str().to_string(),
            from: query.range.from_rfc3339(),
            to: query.range.to_rfc3339(),
            limit: query.fetch_limit,
            cursor_timestamp: query.cursor.as_ref().map(|key| key.timestamp_rfc3339()),
            cursor_event_id: query.cursor.as_ref().map(|key| key.event_id.clone()),
            filter_actor_id: filters.actor_id,
            filter_actor_login: filters.actor_login.clone(),
            filter_operation_id: filters.operation_id.clone(),
            filter_method: filters.method.clone(),
            filter_status_code: filters.status_code,
            filter_status_low: status_bounds.map(|(low, _)| low),
            filter_status_high: status_bounds.map(|(_, high)| high),
            filter_outcome: filters.outcome.map(|outcome| outcome.as_str().to_string()),
            filter_session_id: filters.session_id.clone(),
            filter_repo_full_name: filters.repo_full_name.clone(),
            filter_trigger_issue: filters.trigger_issue,
            filter_request_id: filters.request_id.clone(),
        }
    }
}

#[async_trait]
impl ActivitySource for RelayActivitySource {
    fn kind(&self) -> ActivitySourceKind {
        ActivitySourceKind::Relay
    }

    async fn fetch(&self, query: &SourceQuery) -> Result<SourcePage, SourceError> {
        let wire = Self::build_query(query);
        let page = self
            .client
            .read_records(&wire, self.timeout)
            .await
            .map_err(map_error)?;
        let mut source_page = SourcePage {
            records: Vec::with_capacity(page.rows.len()),
            raw_rows: page.rows.len(),
            row_errors: 0,
        };
        for row in &page.rows {
            match decode(row) {
                Ok(record) => source_page.records.push(record),
                // One undecodable row is dropped with its bounded reason and
                // counted; failing the page would let a single malformed record
                // hide every well-formed one.
                Err(error) => {
                    source_page.row_errors += 1;
                    tracing::warn!(
                        source = ActivitySourceKind::Relay.as_str(),
                        reason = %error,
                        "operations: dropping an undecodable relay activity row"
                    );
                }
            }
        }
        Ok(source_page)
    }
}

/// Decode one relay row into the source-neutral record shape.
///
/// The stored body is the SAME flattened property set the PostHog projection
/// writes, so it is decoded through the SAME [`super::rows`] contract rather than
/// a second, parallel decoder that could diverge.
fn decode(row: &RecordRowV1) -> Result<ActivityRecord, RowError> {
    let delivery_state = delivery_state(&row.delivery_state)?;
    let view = rows::json::JsonRowView::new(&row.terminal, &row.record_kind, &row.sort_timestamp);
    rows::decode(&view, ActivitySourceKind::Relay, delivery_state)
}

/// Map the relay's delivery wire spelling onto the merge's enum.
fn delivery_state(value: &str) -> Result<DeliveryState, RowError> {
    match value {
        "verified_in_posthog" => Ok(DeliveryState::VerifiedInPosthog),
        "accepted_pending_verification" => Ok(DeliveryState::AcceptedPendingVerification),
        "queued" => Ok(DeliveryState::Queued),
        "incomplete" => Ok(DeliveryState::Incomplete),
        "dead_letter" => Ok(DeliveryState::DeadLetter),
        _ => Err(RowError::WrongType {
            column: "delivery_state",
        }),
    }
}

/// Map a relay client failure onto the source error the endpoint documents.
fn map_error(error: crate::audit::relay::RelayClientError) -> SourceError {
    use crate::audit::relay::RelayClientError;
    match error {
        // A refused credential or a rejected query shape is a deployment fault:
        // retrying cannot fix it, so it must not read as a transient outage.
        RelayClientError::Rejected { kind } => SourceError::Upstream { kind },
        RelayClientError::Conflict => SourceError::Upstream { kind: "conflict" },
        RelayClientError::Unavailable { kind } => SourceError::Transient { kind },
    }
}

#[cfg(test)]
#[path = "relay_tests.rs"]
mod tests;
