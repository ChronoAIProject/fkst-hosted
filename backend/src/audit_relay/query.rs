//! The scoped read protocol: `GET /internal/v1/audit/records`.
//!
//! This endpoint is the one place a caller's identity turns into rows, so its
//! contract is written defensively:
//!
//! - **The scope is server-constructed, not caller-asserted.** The control plane
//!   mints it from a sealed [`crate::session_access::ActivityVisibilityConstraint`]
//!   ([`RelayScopeV1::from_constraint`]), which can only exist after a verified
//!   identity and an allowing policy decision. There is no admin FLAG on this
//!   wire: `scope=all` is meaningful only because the read token is held solely
//!   by the control plane and the control plane only ever writes `all` after its
//!   own global-admin check.
//! - **There is no free-form field.** No SQL fragment, no property name, no sort
//!   expression, no raw actor expression — only the closed filter vocabulary
//!   [`crate::operations::filters`] already validated, carried as typed values
//!   that reach SQLite exclusively as bound parameters.
//! - **`started` rows are never returned.** A registered-but-unfinished request
//!   has no terminal projection; returning one would force the reader to invent
//!   an outcome. It becomes visible as `incomplete` once its deadline plus grace
//!   elapses — which is precisely the guarantee the durable start exists for.
//!
//! The response carries the stored, already-sanitized event JSON verbatim plus a
//! delivery state. Nothing is re-derived on the way out, so the read surface
//! cannot widen what the write surface accepted.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::session_access::ActivityVisibilityConstraint;

/// Scope wire spelling: only rows provably owned by one verified actor.
pub const SCOPE_MINE: &str = "mine";
/// Scope wire spelling: every actor and record kind (global administrators).
pub const SCOPE_ALL: &str = "all";

/// The server-constructed visibility scope.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RelayScopeV1 {
    /// [`SCOPE_MINE`] or [`SCOPE_ALL`].
    pub scope: String,
    /// The verified viewer id every API-request row must carry. Mandatory for
    /// [`SCOPE_MINE`]; a `mine` query without it is refused rather than widened.
    pub actor_id: Option<i64>,
    /// The ONE session whose system lifecycle rows a personal scope may also
    /// see. It never widens the actor predicate.
    pub lifecycle_session_id: Option<String>,
}

impl RelayScopeV1 {
    /// Project the sealed constraint onto the wire.
    pub fn from_constraint(constraint: &ActivityVisibilityConstraint) -> Self {
        match constraint {
            ActivityVisibilityConstraint::Mine(scope) => Self {
                scope: SCOPE_MINE.to_string(),
                actor_id: Some(scope.actor_id()),
                lifecycle_session_id: scope.lifecycle_session_id().map(str::to_string),
            },
            ActivityVisibilityConstraint::All(_) => Self {
                scope: SCOPE_ALL.to_string(),
                actor_id: None,
                lifecycle_session_id: None,
            },
        }
    }
}

/// The relay's internal, already-checked scope. Private fields: the only way to
/// obtain one is [`ResolvedScope::resolve`], which refuses a `mine` scope with no
/// actor id, so no SQL path can be reached with a half-specified predicate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResolvedScope {
    Mine {
        actor_id: i64,
        lifecycle_session_id: Option<String>,
    },
    All,
}

impl ResolvedScope {
    /// Check the wire scope. `None` when it is not a usable predicate — the
    /// handler then answers `400`, never a widened query.
    pub fn resolve(wire: &RelayScopeV1) -> Option<Self> {
        match wire.scope.as_str() {
            SCOPE_MINE => wire.actor_id.map(|actor_id| ResolvedScope::Mine {
                actor_id,
                lifecycle_session_id: wire.lifecycle_session_id.clone(),
            }),
            SCOPE_ALL => Some(ResolvedScope::All),
            _ => None,
        }
    }

    /// The bounded label for logs and metrics.
    pub fn as_str(&self) -> &'static str {
        match self {
            ResolvedScope::Mine { .. } => SCOPE_MINE,
            ResolvedScope::All => SCOPE_ALL,
        }
    }
}

/// One scoped read, as query parameters.
///
/// Flat rather than nested because it travels as a URL query string; every field
/// is optional except the scope, the window, and the limit, and every value is
/// bound as a SQL parameter.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct RecordsQueryV1 {
    // ---- the mandatory, server-constructed scope ----
    pub scope: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle_session_id: Option<String>,

    // ---- what to return ----
    /// `api_request`, `sandbox_lifecycle`, or `all`.
    pub record_kind: String,
    /// Inclusive lower bound, RFC3339 UTC.
    pub from: String,
    /// Exclusive upper bound, RFC3339 UTC.
    pub to: String,
    /// Rows to fetch (the caller already added its has-more probe).
    pub limit: u32,

    // ---- keyset resume point ----
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor_timestamp: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor_event_id: Option<String>,

    // ---- the closed filter vocabulary ----
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter_actor_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter_actor_login: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter_operation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter_method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter_status_code: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter_status_low: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter_status_high: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter_outcome: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter_repo_full_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter_trigger_issue: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter_request_id: Option<String>,
}

impl RecordsQueryV1 {
    /// The scope half of this query, as it arrived.
    pub fn scope_wire(&self) -> RelayScopeV1 {
        RelayScopeV1 {
            scope: self.scope.clone(),
            actor_id: self.actor_id,
            lifecycle_session_id: self.lifecycle_session_id.clone(),
        }
    }
}

/// One returned row.
///
/// `terminal` is the stored, already-sanitized wire event verbatim
/// ([`super::protocol::RequestCompletionV1`] or
/// [`super::protocol::LifecycleEventV1`]). It is echoed, never rebuilt: a read
/// path that re-derived content could disagree with what was committed.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RecordRowV1 {
    pub event_id: String,
    /// `api_request` or `sandbox_lifecycle`.
    pub record_kind: String,
    /// The storage state ([`super::record::RecordState`]).
    pub state: String,
    /// The activity-merge delivery state.
    pub delivery_state: String,
    /// The row's position in the total order: its terminal instant.
    pub sort_timestamp: String,
    pub terminal: Value,
}

/// One page of already-authorized rows, newest first.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct RecordsPageV1 {
    pub rows: Vec<RecordRowV1>,
}

#[cfg(test)]
#[path = "query_tests.rs"]
mod tests;
