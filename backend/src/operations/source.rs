//! The activity-source boundary: one trait, two implementations, one contract.
//!
//! ```text
//! SourceQuery  (typed ActivityVisibilityConstraint + fixed filters + keyset)
//!   -> ActivitySource::fetch
//!        -> PostHog   [super::posthog]  fixed HogQL, predicate before LIMIT
//!        -> relay     (issue #5678)     scoped SQL, predicate before LIMIT
//!   -> already-authorized ActivityRecord candidates
//! ```
//!
//! ## The contract every implementation owes
//!
//! 1. **Apply the constraint in the source's own query language, before its own
//!    `LIMIT`.** A source may not fetch broadly and filter afterwards: the page
//!    boundary would then be decided by rows the caller may not see, and page
//!    fullness alone would leak their existence (epic `AUTH-06`).
//! 2. **Return only already-authorized candidates.** The merge layer performs no
//!    authorization; it assumes every record it receives passed the constraint.
//! 3. **Order by `(sort_timestamp, event_id)` descending**, and honour the keyset
//!    cursor as a strictly-after-in-sort-order predicate, so pages tile with no
//!    overlap and no gap.
//! 4. **Fetch `limit + 1`.** The extra row is how the page learns there is a next
//!    one without a count, and it is never returned.
//!
//! Taking the SAME typed [`ActivityVisibilityConstraint`] in both sources is the
//! point: the value can only be minted by [`crate::session_access`] from a
//! verified identity and an allowing policy decision, so "did this source get the
//! right predicate" is a type question rather than a review question.
//!
//! ## Why the error type is this small
//!
//! It maps onto exactly the public statuses the endpoint documents: an
//! authentication or schema failure is a deployment fault (`502`, retrying will
//! not help), everything transient is `503`. Nothing here carries upstream error
//! text, a URL, a rendered query, or a credential — the variants are structurally
//! incapable of it.

use async_trait::async_trait;

use crate::session_access::ActivityVisibilityConstraint;

use super::cursor::CursorKey;
use super::filters::{ActivityFilters, RecordKind, TimeRange};
use super::record::{ActivityRecord, ActivitySourceKind};

/// One source read, fully described.
#[derive(Clone, Debug)]
pub struct SourceQuery {
    /// The mandatory row-visibility predicate. A source that ignores it is a
    /// row-authorization bug.
    pub constraint: ActivityVisibilityConstraint,
    pub record_kind: RecordKind,
    pub range: TimeRange,
    pub filters: ActivityFilters,
    /// Resume point: return only rows strictly after this key in the descending
    /// `(timestamp, event_id)` order.
    pub cursor: Option<CursorKey>,
    /// `limit + 1`: the extra row is the has-more probe.
    pub fetch_limit: u32,
}

impl SourceQuery {
    /// The exact session id whose lifecycle rows this query may return, in
    /// personal scope. `None` in global scope, where an administrator sees every
    /// session's rows.
    pub fn authorized_session_id(&self) -> Option<&str> {
        match &self.constraint {
            ActivityVisibilityConstraint::Mine(scope) => scope.lifecycle_session_id(),
            ActivityVisibilityConstraint::All(_) => None,
        }
    }
}

/// Why a source could not answer.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum SourceError {
    /// The source is not configured for this deployment.
    #[error("activity source is not configured")]
    NotConfigured,
    /// The credential was refused, or the source rejected the fixed query's
    /// shape. Retrying cannot help — this is a deployment fault.
    #[error("activity source rejected the request ({kind})")]
    Upstream { kind: &'static str },
    /// A transport, timeout, or capacity failure that another attempt could get
    /// past.
    #[error("activity source is temporarily unavailable ({kind})")]
    Transient { kind: &'static str },
}

impl SourceError {
    /// Whether the failure is the deployment's fault rather than a blip. Drives
    /// the `502` vs `503` split the endpoint documents.
    pub fn is_upstream_fault(&self) -> bool {
        matches!(self, SourceError::Upstream { .. })
    }

    /// The bounded reason label. Closed strings only.
    pub fn kind(&self) -> &'static str {
        match self {
            SourceError::NotConfigured => "not_configured",
            SourceError::Upstream { kind } => kind,
            SourceError::Transient { kind } => kind,
        }
    }
}

/// One source's answer.
///
/// `raw_rows` is separate from `records.len()` on purpose: a row the source
/// returned but this build could not decode still consumed one of the
/// `fetch_limit` slots, so it is what decides whether the source was saturated —
/// and therefore whether a next page can exist. Deriving that from the decoded
/// count would make one malformed record silently truncate a timeline.
#[derive(Debug, Default)]
pub struct SourcePage {
    /// Already-authorized candidates, newest first.
    pub records: Vec<ActivityRecord>,
    /// How many rows the source returned in total, decodable or not.
    pub raw_rows: usize,
    /// Rows that failed the typed row contract. Counted, never returned.
    pub row_errors: usize,
}

impl SourcePage {
    /// Whether the source filled its requested page, meaning more rows may exist
    /// beyond it.
    pub fn saturated(&self, fetch_limit: u32) -> bool {
        self.raw_rows >= fetch_limit as usize
    }
}

/// A source of already-authorized activity records.
#[async_trait]
pub trait ActivitySource: Send + Sync + std::fmt::Debug {
    /// Which source this is, for bounded telemetry and the response's
    /// per-source health block.
    fn kind(&self) -> ActivitySourceKind;

    /// Fetch at most `query.fetch_limit` already-authorized candidates.
    async fn fetch(&self, query: &SourceQuery) -> Result<SourcePage, SourceError>;
}

#[cfg(test)]
#[path = "source_tests.rs"]
mod tests;
