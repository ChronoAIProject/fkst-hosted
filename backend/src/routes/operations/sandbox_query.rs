//! The sandbox endpoint's query parameters, and their normalization.
//!
//! Every parameter is declared as a plain scalar and validated by hand rather
//! than by a serde enum, for the same reason the activity endpoint does: a serde
//! rejection can only say "the query did not parse", which collapses an unknown
//! `status`, a negative `creator_id`, and a malformed `repo_full_name` into one
//! opaque `400`. Validating here lets each failure name the parameter it is about
//! while still never echoing the value that failed.
//!
//! Normalization is PURE: it makes no authorization decision, reads no registry,
//! and touches no runtime backend. That is what lets a malformed request be
//! refused before the deployment spends anything on it, and it is why a caller
//! learns nothing from the timing of a `400`.
//!
//! One normalized value feeds three consumers — the audit record's safe
//! arguments, the post-authorization filter predicate, and the response's
//! `filters_applied` echo — so the record, the query, and the echo can never
//! describe different requests.
//!
//! ## Why `scope` is parsed separately from the rest
//!
//! The issue's authorization pipeline is normative and puts "resolve the
//! requested scope" BEFORE "validate filter syntax". Splitting the two entry
//! points is what makes the route able to honour that: a regular caller sending
//! `?scope=all` alongside a malformed `status` is told they may not select the
//! global scope (`403`), rather than being handed a `400` that never mentions the
//! decision that actually stopped them. Both halves remain pure and both still
//! run before any registry or backend call.

use serde::Deserialize;
use utoipa::IntoParams;

use crate::error::AppError;
use crate::operations::sandbox::filters::{self, SandboxFilters};
use crate::session_access::RequestedScope;

/// Query parameters for `GET /api/v1/operations/sandboxes`.
#[derive(Clone, Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct SandboxQueryParams {
    /// `accessible` or `all`. Omitted resolves to `accessible` for a regular
    /// caller and `all` for a deployment global administrator.
    pub scope: Option<String>,
    /// A normalized runtime status: `pending`, `running`, `paused`,
    /// `transitioning`, `succeeded`, `failed`, `terminating`, `terminated`, or
    /// `unknown`.
    pub status: Option<String>,
    /// `kubernetes` or `opensandbox`. A backend this deployment does not run is
    /// valid syntax and simply matches no row.
    pub backend: Option<String>,
    /// The effective creator's immutable GitHub numeric id.
    pub creator_id: Option<i64>,
    /// The effective creator's login snapshot, matched ASCII-case-insensitively.
    pub creator_login: Option<String>,
    /// Exact `owner/name`.
    pub repo_full_name: Option<String>,
    /// Exact session id. In `accessible` scope it is authorized BEFORE the
    /// runtime backend is called, and an unknown or unauthorized id answers the
    /// same `404`.
    pub session_id: Option<String>,
    /// Positive trigger-issue number; most useful alongside `repo_full_name`.
    pub trigger_issue: Option<i64>,
    /// `launch_metadata`, `backfilled_current_trigger`, `partial_metadata`,
    /// `unknown_legacy`, or `conflict`.
    pub attribution_source: Option<String>,
}

/// Step 1 of the normative pipeline: the scope the caller asked for.
///
/// `Ok(None)` means the caller stated none, which the server resolves to their
/// natural default. Parsed on its own so the scope DECISION can be made before
/// any other parameter is looked at.
pub fn requested_scope(params: &SandboxQueryParams) -> Result<Option<RequestedScope>, AppError> {
    params.scope.as_deref().map(parse_scope).transpose()
}

/// Step 2 of the normative pipeline: every remaining filter, validated and
/// normalized into the single form the record, the predicate, and the echo share.
pub fn filters(params: &SandboxQueryParams) -> Result<SandboxFilters, AppError> {
    let filters = SandboxFilters {
        status: params
            .status
            .as_deref()
            .map(filters::parse_status)
            .transpose()?,
        backend: params
            .backend
            .as_deref()
            .map(filters::parse_backend)
            .transpose()?,
        creator_id: params
            .creator_id
            .map(filters::parse_creator_id)
            .transpose()?,
        creator_login: params
            .creator_login
            .as_deref()
            .map(filters::parse_creator_login)
            .transpose()?,
        repo_full_name: params
            .repo_full_name
            .as_deref()
            .map(filters::parse_repo_full_name)
            .transpose()?,
        session_id: params
            .session_id
            .as_deref()
            .map(filters::parse_session_id)
            .transpose()?,
        trigger_issue: params
            .trigger_issue
            .map(filters::parse_trigger_issue)
            .transpose()?,
        attribution_source: params
            .attribution_source
            .as_deref()
            .map(filters::parse_attribution_source)
            .transpose()?,
    };
    Ok(filters)
}

/// Map the route's `accessible`/`all` vocabulary onto the shared scope request.
fn parse_scope(value: &str) -> Result<RequestedScope, AppError> {
    match value.trim() {
        "accessible" => Ok(RequestedScope::Personal),
        "all" => Ok(RequestedScope::Global),
        _ => Err(AppError::Validation(
            "scope must be accessible or all".to_string(),
        )),
    }
}

#[cfg(test)]
#[path = "sandbox_query_tests.rs"]
mod tests;
