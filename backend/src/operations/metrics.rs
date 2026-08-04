//! Bounded telemetry for the activity query (`fkst_operations_activity_*`).
//!
//! Every label is a closed Rust enum, so the series count is decided at compile
//! time: scopes × record kinds × results, sources × results, row results, and
//! rejection reasons. No viewer, actor, filter, session, repository, request,
//! event, or cursor value is ever a label OR a value here (epic `OPS-04`).
//!
//! The counters live in fixed-size arrays indexed by those enums, which is what
//! makes "a future variant cannot introduce unbounded cardinality" a compile-time
//! property rather than a review note.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use super::filters::RecordKind;
use super::record::ActivitySourceKind;

/// The terminal result of one activity query. Mirrors the documented status set.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueryResult {
    Success,
    InvalidRequest,
    Forbidden,
    NotFound,
    RateLimited,
    NotConfigured,
    UpstreamError,
    Unavailable,
}

impl QueryResult {
    pub const COUNT: usize = 8;
    pub const ALL: [QueryResult; Self::COUNT] = [
        QueryResult::Success,
        QueryResult::InvalidRequest,
        QueryResult::Forbidden,
        QueryResult::NotFound,
        QueryResult::RateLimited,
        QueryResult::NotConfigured,
        QueryResult::UpstreamError,
        QueryResult::Unavailable,
    ];

    /// The stable wire string.
    pub fn as_str(self) -> &'static str {
        match self {
            QueryResult::Success => "success",
            QueryResult::InvalidRequest => "invalid_request",
            QueryResult::Forbidden => "forbidden",
            QueryResult::NotFound => "not_found",
            QueryResult::RateLimited => "rate_limited",
            QueryResult::NotConfigured => "not_configured",
            QueryResult::UpstreamError => "upstream_error",
            QueryResult::Unavailable => "unavailable",
        }
    }

    fn index(self) -> usize {
        match self {
            QueryResult::Success => 0,
            QueryResult::InvalidRequest => 1,
            QueryResult::Forbidden => 2,
            QueryResult::NotFound => 3,
            QueryResult::RateLimited => 4,
            QueryResult::NotConfigured => 5,
            QueryResult::UpstreamError => 6,
            QueryResult::Unavailable => 7,
        }
    }
}

/// The result of one SOURCE read, for the per-source duration summary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceResult {
    Success,
    UpstreamError,
    Unavailable,
}

impl SourceResult {
    pub const COUNT: usize = 3;
    pub const ALL: [SourceResult; Self::COUNT] = [
        SourceResult::Success,
        SourceResult::UpstreamError,
        SourceResult::Unavailable,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            SourceResult::Success => "success",
            SourceResult::UpstreamError => "upstream_error",
            SourceResult::Unavailable => "unavailable",
        }
    }

    fn index(self) -> usize {
        match self {
            SourceResult::Success => 0,
            SourceResult::UpstreamError => 1,
            SourceResult::Unavailable => 2,
        }
    }
}

/// What happened to one already-authorized candidate row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RowResult {
    /// Decoded and returned.
    Returned,
    /// Failed the typed row contract; counted, never returned.
    Invalid,
    /// A second copy of an event already held.
    Duplicate,
    /// Contradicted the personal visibility constraint. Operator-only telemetry:
    /// a non-zero value means a source predicate has regressed.
    ConstraintViolation,
}

impl RowResult {
    pub const COUNT: usize = 4;
    pub const ALL: [RowResult; Self::COUNT] = [
        RowResult::Returned,
        RowResult::Invalid,
        RowResult::Duplicate,
        RowResult::ConstraintViolation,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            RowResult::Returned => "returned",
            RowResult::Invalid => "invalid",
            RowResult::Duplicate => "duplicate",
            RowResult::ConstraintViolation => "constraint_violation",
        }
    }

    fn index(self) -> usize {
        match self {
            RowResult::Returned => 0,
            RowResult::Invalid => 1,
            RowResult::Duplicate => 2,
            RowResult::ConstraintViolation => 3,
        }
    }
}

/// Why a scope or session selection was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RejectionReason {
    GlobalScope,
    CrossActorFilter,
    /// A regular caller asked for lifecycle rows without an authorized session.
    LifecycleSession,
    /// A cursor failed its binding check.
    Cursor,
    /// Local query capacity was exhausted.
    Capacity,
}

impl RejectionReason {
    pub const COUNT: usize = 5;
    pub const ALL: [RejectionReason; Self::COUNT] = [
        RejectionReason::GlobalScope,
        RejectionReason::CrossActorFilter,
        RejectionReason::LifecycleSession,
        RejectionReason::Cursor,
        RejectionReason::Capacity,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            RejectionReason::GlobalScope => "global_scope_forbidden",
            RejectionReason::CrossActorFilter => "cross_actor_forbidden",
            RejectionReason::LifecycleSession => "lifecycle_session_forbidden",
            RejectionReason::Cursor => "invalid_cursor",
            RejectionReason::Capacity => "capacity_exhausted",
        }
    }

    fn index(self) -> usize {
        match self {
            RejectionReason::GlobalScope => 0,
            RejectionReason::CrossActorFilter => 1,
            RejectionReason::LifecycleSession => 2,
            RejectionReason::Cursor => 3,
            RejectionReason::Capacity => 4,
        }
    }
}

/// The two effective scopes, as a dense metric index.
const SCOPES: [&str; 2] = ["mine", "all"];
const RECORD_KINDS: [RecordKind; 3] = [
    RecordKind::ApiRequest,
    RecordKind::SandboxLifecycle,
    RecordKind::All,
];

const QUERY_SERIES: usize = SCOPES.len() * RECORD_KINDS.len() * QueryResult::COUNT;
const SOURCE_SERIES: usize = ActivitySourceKind::ALL.len() * SourceResult::COUNT;

/// Process-local activity-query counters. Cheap to clone; every clone shares one
/// backing store.
#[derive(Clone)]
pub struct ActivityMetrics {
    queries: Arc<[AtomicU64; QUERY_SERIES]>,
    source_duration_sum: Arc<[AtomicU64; SOURCE_SERIES]>,
    source_duration_count: Arc<[AtomicU64; SOURCE_SERIES]>,
    rows: Arc<[AtomicU64; RowResult::COUNT]>,
    partial: Arc<[AtomicU64; 2]>,
    rejections: Arc<[AtomicU64; RejectionReason::COUNT]>,
}

/// `[AtomicU64; N]` has no blanket `Default` (std stops at 32 elements and the
/// query family has 48 series), so the arrays are built explicitly.
fn zeroed<const N: usize>() -> Arc<[AtomicU64; N]> {
    Arc::new(std::array::from_fn(|_| AtomicU64::new(0)))
}

impl Default for ActivityMetrics {
    fn default() -> Self {
        Self {
            queries: zeroed(),
            source_duration_sum: zeroed(),
            source_duration_count: zeroed(),
            rows: zeroed(),
            partial: zeroed(),
            rejections: zeroed(),
        }
    }
}

impl std::fmt::Debug for ActivityMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActivityMetrics")
            .field("query_series", &QUERY_SERIES)
            .finish()
    }
}

/// The dense index of one `(scope, record_kind, result)` triple.
fn query_index(scope: &str, record_kind: RecordKind, result: QueryResult) -> usize {
    let scope_index = usize::from(scope == "all");
    let kind_index = match record_kind {
        RecordKind::ApiRequest => 0,
        RecordKind::SandboxLifecycle => 1,
        RecordKind::All => 2,
    };
    (scope_index * RECORD_KINDS.len() + kind_index) * QueryResult::COUNT + result.index()
}

fn source_index(source: ActivitySourceKind, result: SourceResult) -> usize {
    let source_index = match source {
        ActivitySourceKind::Posthog => 0,
        ActivitySourceKind::Relay => 1,
    };
    source_index * SourceResult::COUNT + result.index()
}

impl ActivityMetrics {
    /// Fresh counters.
    pub fn new() -> Self {
        Self::default()
    }

    /// Count one terminal query outcome.
    pub fn record_query(&self, scope: &str, record_kind: RecordKind, result: QueryResult) {
        self.queries[query_index(scope, record_kind, result)].fetch_add(1, Ordering::Relaxed);
    }

    /// Observe one source read's duration and result.
    pub fn record_source(
        &self,
        source: ActivitySourceKind,
        result: SourceResult,
        duration_millis: u64,
    ) {
        let index = source_index(source, result);
        self.source_duration_sum[index].fetch_add(duration_millis, Ordering::Relaxed);
        self.source_duration_count[index].fetch_add(1, Ordering::Relaxed);
    }

    /// Count already-authorized candidate rows by fate.
    pub fn record_rows(&self, result: RowResult, count: u64) {
        if count > 0 {
            self.rows[result.index()].fetch_add(count, Ordering::Relaxed);
        }
    }

    /// Count one page marked partial because of `source`.
    pub fn record_partial(&self, source: ActivitySourceKind) {
        let index = match source {
            ActivitySourceKind::Posthog => 0,
            ActivitySourceKind::Relay => 1,
        };
        self.partial[index].fetch_add(1, Ordering::Relaxed);
    }

    /// Count one refused scope/session/cursor/capacity selection.
    pub fn record_rejection(&self, reason: RejectionReason) {
        self.rejections[reason.index()].fetch_add(1, Ordering::Relaxed);
    }

    /// A consistent read projection for `/metrics`.
    pub fn snapshot(&self) -> ActivityMetricsSnapshot {
        let load = |slots: &[AtomicU64]| -> Vec<u64> {
            slots
                .iter()
                .map(|slot| slot.load(Ordering::Relaxed))
                .collect()
        };
        ActivityMetricsSnapshot {
            queries: load(&*self.queries),
            source_duration_sum: load(&*self.source_duration_sum),
            source_duration_count: load(&*self.source_duration_count),
            rows: load(&*self.rows),
            partial: load(&*self.partial),
            rejections: load(&*self.rejections),
        }
    }
}

/// An immutable copy of the activity counters.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ActivityMetricsSnapshot {
    queries: Vec<u64>,
    source_duration_sum: Vec<u64>,
    source_duration_count: Vec<u64>,
    rows: Vec<u64>,
    partial: Vec<u64>,
    rejections: Vec<u64>,
}

impl ActivityMetricsSnapshot {
    /// Every `(scope, record_kind, result)` series, in exposition order.
    pub fn queries(
        &self,
    ) -> impl Iterator<Item = (&'static str, &'static str, &'static str, u64)> + '_ {
        SCOPES.into_iter().flat_map(move |scope| {
            RECORD_KINDS.into_iter().flat_map(move |kind| {
                QueryResult::ALL.into_iter().map(move |result| {
                    let count = self
                        .queries
                        .get(query_index(scope, kind, result))
                        .copied()
                        .unwrap_or_default();
                    (scope, kind.as_str(), result.as_str(), count)
                })
            })
        })
    }

    /// Every `(source, result)` duration summary, in exposition order.
    pub fn source_durations(
        &self,
    ) -> impl Iterator<Item = (&'static str, &'static str, u64, u64)> + '_ {
        ActivitySourceKind::ALL.into_iter().flat_map(move |source| {
            SourceResult::ALL.into_iter().map(move |result| {
                let index = source_index(source, result);
                (
                    source.as_str(),
                    result.as_str(),
                    self.source_duration_sum
                        .get(index)
                        .copied()
                        .unwrap_or_default(),
                    self.source_duration_count
                        .get(index)
                        .copied()
                        .unwrap_or_default(),
                )
            })
        })
    }

    /// Row fates, in exposition order.
    pub fn rows(&self) -> impl Iterator<Item = (&'static str, u64)> + '_ {
        RowResult::ALL.into_iter().map(move |result| {
            (
                result.as_str(),
                self.rows.get(result.index()).copied().unwrap_or_default(),
            )
        })
    }

    /// Partial-page counts by source, in exposition order.
    pub fn partial(&self) -> impl Iterator<Item = (&'static str, u64)> + '_ {
        ActivitySourceKind::ALL
            .into_iter()
            .enumerate()
            .map(move |(index, source)| {
                (
                    source.as_str(),
                    self.partial.get(index).copied().unwrap_or_default(),
                )
            })
    }

    /// Rejection counts by bounded reason, in exposition order.
    pub fn rejections(&self) -> impl Iterator<Item = (&'static str, u64)> + '_ {
        RejectionReason::ALL.into_iter().map(move |reason| {
            (
                reason.as_str(),
                self.rejections
                    .get(reason.index())
                    .copied()
                    .unwrap_or_default(),
            )
        })
    }
}

#[cfg(test)]
#[path = "metrics_tests.rs"]
mod tests;
