//! RESERVED safe arguments for the `/api/v1/operations/*` surface.
//!
//! Neither route exists yet: issue #5672 adds `operations_list_activity` and
//! #5675 adds `operations_list_sandboxes`. Their DTOs live here now so those
//! issues attach an already-reviewed argument boundary to their routes rather
//! than inventing one under delivery pressure — the DTO, its allowlist, and its
//! unit tests land with the contract, not with the handler.
//!
//! Attaching one is two lines: add the operation to
//! [`crate::audit::request::policy::OPERATION_POLICIES`] with the matching
//! [`SafeArgumentSpec`], and call [`super::record`] from the handler once the
//! scope decision is made. Until then the ids are deliberately absent from that
//! table, so the coverage guard keeps reporting them as undocumented if a route
//! ever appears without its policy.
//!
//! [`SafeArgumentSpec`]: super::SafeArgumentSpec
//!
//! ## The two rules these DTOs already encode
//!
//! - **The verified actor is not an argument.** A regular caller's own id is
//!   already the record's `actor_id`; duplicating it inside `arguments` would
//!   invite a reader to filter on the copy, which is not the authorization
//!   field.
//! - **A denied cross-user probe records the ATTEMPT, not the probe.** A regular
//!   user asking for the global scope, or naming another actor, records the
//!   closed `requested_scope` and a boolean `actor_filter_present` — never the
//!   login or id they guessed at. The denial itself is already the record's
//!   stable error code.
//!
//! Cursor text, HogQL, PostHog/relay credentials, configured access lists,
//! session access entries, policy-decision internals, and hidden-row counts are
//! forbidden here exactly as they are everywhere else.

use serde::Serialize;

use super::bounds::{safe_repo_full_name, safe_session_id};
use super::catalog;
use super::BoundedAuditArguments;

/// The scope an activity query ran under.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityScope {
    /// Only records whose verified actor id equals the caller's.
    Mine,
    /// Every record; a global-admin-only scope.
    All,
}

impl ActivityScope {
    /// The stable wire string. Shared with the closed-enum metric label so the
    /// record and the counter can never disagree about which scope ran.
    pub fn as_str(self) -> &'static str {
        match self {
            ActivityScope::Mine => "mine",
            ActivityScope::All => "all",
        }
    }
}

/// The scope a sandbox inventory query ran under.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxScope {
    /// Only sessions the caller passes session-visibility authorization for.
    Accessible,
    /// Every FKST-managed runtime; a global-admin-only scope.
    All,
}

/// Which record kinds an activity query asked for.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityRecordKind {
    ApiRequest,
    SandboxLifecycle,
    All,
}

/// `operations_list_activity` — the scoped historical activity query.
#[derive(Clone, Debug, Serialize)]
pub struct SafeOperationsListActivity {
    /// The EFFECTIVE scope the query ran under.
    pub scope: ActivityScope,
    /// The scope the caller asked for, when it differed from the effective one —
    /// this is what makes a denied global request legible without recording who
    /// they tried to impersonate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requested_scope: Option<ActivityScope>,
    pub record_kind: ActivityRecordKind,
    /// Normalized RFC3339 UTC lower bound.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// Normalized RFC3339 UTC upper bound.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    /// The clamped page size the query executed with.
    pub limit: u32,
    /// Whether a keyset cursor was supplied. Never the cursor itself.
    pub cursor_present: bool,
    /// Whether a cross-user actor filter was supplied. Never its value.
    pub actor_filter_present: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo_full_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_issue: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// An uppercase HTTP method filter.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    /// An `operationId` filter, valid only when the catalog declares it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    /// An exact status-code filter.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    /// A status-FAMILY filter (`2xx`..`5xx`). Recorded separately from `status`
    /// because both are accepted and both become source predicates: folding one
    /// into the other would leave an audit reader unable to reconstruct which
    /// constraint actually ran.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_class: Option<String>,
    /// An audit-outcome filter from the contract's closed set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
}

impl BoundedAuditArguments for SafeOperationsListActivity {
    const OPERATION_ID: &'static str = "operations_list_activity";
    const ALLOWED_FIELDS: &'static [&'static str] = catalog::OPERATIONS_LIST_ACTIVITY_FIELDS;
}

/// `operations_list_sandboxes` — the scoped live runtime inventory.
#[derive(Clone, Debug, Serialize)]
pub struct SafeOperationsListSandboxes {
    pub scope: SandboxScope,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requested_scope: Option<SandboxScope>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo_full_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_issue: Option<i64>,
    /// A normalized runtime-status filter (the inventory's own closed set).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    pub limit: u32,
}

impl BoundedAuditArguments for SafeOperationsListSandboxes {
    const OPERATION_ID: &'static str = "operations_list_sandboxes";
    const ALLOWED_FIELDS: &'static [&'static str] = catalog::OPERATIONS_LIST_SANDBOXES_FIELDS;
}

/// Validate a session-id filter before it becomes a property.
///
/// Exposed so the sibling issues cannot accidentally record the raw value: an
/// exact unauthorized/nonexistent session probe must be indistinguishable, and
/// the safest way to keep it that way is to never echo an unvalidated one.
pub fn filter_session_id(value: &str) -> Option<String> {
    safe_session_id(value)
}

/// Validate a repository filter before it becomes a property.
pub fn filter_repo_full_name(owner: &str, name: &str) -> Option<String> {
    safe_repo_full_name(owner, name)
}

#[cfg(test)]
#[path = "operations_tests.rs"]
mod tests;
