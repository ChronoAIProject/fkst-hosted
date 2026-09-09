//! Orchestration: admit, query both sources concurrently, merge, observe.
//!
//! Everything authorization-shaped has already happened by the time this module
//! runs — the scope was resolved, the lifecycle session was authorized, and the
//! result is the sealed [`ActivityVisibilityConstraint`] it receives. Its job is
//! narrower and entirely mechanical:
//!
//! ```text
//! admit (bounded, per-principal)   -> 429 with a bounded Retry-After
//!   -> posthog.fetch ∥ relay.fetch (each applies the constraint at the SOURCE)
//!   -> merge / dedupe / keyset page
//!   -> bounded telemetry
//! ```
//!
//! The two source reads run CONCURRENTLY and only after authorization succeeded,
//! so a refused request costs the deployment no upstream call at all — and a
//! caller cannot learn anything from the timing difference between "denied" and
//! "denied after a query".

use std::time::Instant;

use crate::error::AppError;
use crate::session_access::ActivityVisibilityConstraint;

use super::cursor::CursorKey;
use super::filters::{ActivityFilters, RecordKind, TimeRange};
use super::limits::{AdmissionDenial, RETRY_AFTER_SECS};
use super::merge::{self, MergedPage};
use super::metrics::{QueryResult, RejectionReason, RowResult, SourceResult};
use super::record::ActivitySourceKind;
use super::source::{ActivitySource, SourceError, SourcePage, SourceQuery};
use super::OperationsState;

/// One fully-authorized activity query.
#[derive(Clone, Debug)]
pub struct ActivityQueryRequest {
    pub constraint: ActivityVisibilityConstraint,
    pub record_kind: RecordKind,
    pub range: TimeRange,
    pub filters: ActivityFilters,
    pub cursor: Option<CursorKey>,
    /// The page size the caller receives. The sources are asked for one more.
    pub limit: u32,
}

/// Execute one authorized activity query.
pub async fn run(
    state: &OperationsState,
    principal_id: i64,
    request: ActivityQueryRequest,
) -> Result<MergedPage, AppError> {
    let scope = request.constraint.as_str();
    let record_kind = request.record_kind;

    if state.posthog.is_none() && state.relay.is_none() {
        state
            .metrics
            .record_query(scope, record_kind, QueryResult::NotConfigured);
        return Err(AppError::AuditQueryNotConfigured(
            "historical activity is not configured on this deployment".to_string(),
        ));
    }

    let _permit = match state.concurrency.try_acquire(principal_id) {
        Ok(permit) => permit,
        Err(denial) => return Err(state.refuse_for_capacity(scope, record_kind, denial)),
    };

    // `limit + 1`: the extra row is how the page learns another exists without a
    // count, and it is never returned.
    let fetch_limit = request.limit.saturating_add(1);
    let query = SourceQuery {
        constraint: request.constraint.clone(),
        record_kind,
        range: request.range,
        filters: request.filters,
        cursor: request.cursor,
        fetch_limit,
    };

    let (posthog, relay) = tokio::join!(
        fetch(state, state.posthog.as_deref(), &query),
        fetch(state, state.relay.as_deref(), &query),
    );

    match merge::merge(
        &request.constraint,
        posthog,
        relay,
        request.limit,
        fetch_limit,
    ) {
        Ok(page) => {
            state.observe_page(&page);
            state
                .metrics
                .record_query(scope, record_kind, QueryResult::Success);
            Ok(page)
        }
        Err(error) => Err(state.refuse_for_source(scope, record_kind, error)),
    }
}

/// Read one optional source, recording its bounded duration and result.
async fn fetch(
    state: &OperationsState,
    source: Option<&dyn ActivitySource>,
    query: &SourceQuery,
) -> Option<Result<SourcePage, SourceError>> {
    let source = source?;
    let kind = source.kind();
    let started = Instant::now();
    let outcome = source.fetch(query).await;
    let result = match &outcome {
        Ok(_) => SourceResult::Success,
        Err(error) if error.is_upstream_fault() => SourceResult::UpstreamError,
        Err(_) => SourceResult::Unavailable,
    };
    let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    state.metrics.record_source(kind, result, elapsed);
    if let Err(error) = &outcome {
        tracing::warn!(
            source = kind.as_str(),
            reason = error.kind(),
            "operations: an activity source could not answer"
        );
    }
    Some(outcome)
}

impl OperationsState {
    /// Record the bounded row/partial telemetry of one assembled page.
    fn observe_page(&self, page: &MergedPage) {
        self.metrics
            .record_rows(RowResult::Returned, page.items.len() as u64);
        self.metrics
            .record_rows(RowResult::Invalid, page.row_errors as u64);
        self.metrics
            .record_rows(RowResult::Duplicate, page.duplicates as u64);
        self.metrics.record_rows(
            RowResult::ConstraintViolation,
            page.constraint_violations as u64,
        );
        if page.status.partial {
            for (source, health) in [
                (ActivitySourceKind::Posthog, page.status.posthog),
                (ActivitySourceKind::Relay, page.status.relay),
            ] {
                if matches!(
                    health,
                    merge::SourceHealth::Unavailable | merge::SourceHealth::Degraded
                ) {
                    self.metrics.record_partial(source);
                }
            }
        }
    }

    /// Map a local capacity refusal onto the documented `429`.
    fn refuse_for_capacity(
        &self,
        scope: &str,
        record_kind: RecordKind,
        denial: AdmissionDenial,
    ) -> AppError {
        self.metrics
            .record_query(scope, record_kind, QueryResult::RateLimited);
        self.metrics.record_rejection(RejectionReason::Capacity);
        tracing::info!(
            reason = denial.as_str(),
            "operations: activity query refused for local capacity"
        );
        AppError::RateLimited {
            message: "too many concurrent activity queries; retry shortly".to_string(),
            retry_after_secs: RETRY_AFTER_SECS,
        }
    }

    /// Map a total source outage onto the documented `502`/`503` split.
    fn refuse_for_source(
        &self,
        scope: &str,
        record_kind: RecordKind,
        error: SourceError,
    ) -> AppError {
        if error.is_upstream_fault() {
            self.metrics
                .record_query(scope, record_kind, QueryResult::UpstreamError);
            // A `502` here is deliberate: an auth or schema failure is a
            // deployment fault a retry cannot fix, and saying `503` would tell an
            // operator to wait for a recovery that will never come.
            AppError::Upstream("the activity source rejected the query".to_string())
        } else {
            self.metrics
                .record_query(scope, record_kind, QueryResult::Unavailable);
            AppError::Unavailable(
                "historical activity is temporarily unavailable; retry shortly".to_string(),
            )
        }
    }

    /// Count a request refused before any source was touched.
    pub fn record_rejected(
        &self,
        scope: &str,
        record_kind: RecordKind,
        result: QueryResult,
        reason: Option<RejectionReason>,
    ) {
        self.metrics.record_query(scope, record_kind, result);
        if let Some(reason) = reason {
            self.metrics.record_rejection(reason);
        }
    }
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
