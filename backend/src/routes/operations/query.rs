//! The activity endpoint's query parameters, and their normalization.
//!
//! Every parameter is declared as a plain scalar and validated by hand rather
//! than by a serde enum. That is deliberate: a serde rejection can only say "the
//! query did not parse", which would collapse every distinct mistake — an unknown
//! `record_kind`, an out-of-range `status_code`, a backwards time window — into
//! one opaque `400`. Validating here lets each failure name the parameter it is
//! about while still never echoing the value that failed.
//!
//! Normalization produces ONE value ([`NormalizedActivityRequest`]) that is then
//! used three times over: as the audit record's safe arguments, as the source
//! predicate's parameters, and as the cursor's binding. Deriving all three from
//! one normalized value is what makes it impossible for the record, the query,
//! and the cursor to describe different requests.

use k8s_openapi::chrono::{DateTime, Utc};
use serde::Deserialize;
use utoipa::IntoParams;

use crate::error::AppError;
use crate::operations::filters::{self, ActivityFilters, RecordKind, TimeRange};
use crate::session_access::RequestedScope;

/// Query parameters for `GET /api/v1/operations/activity`.
#[derive(Clone, Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ActivityQueryParams {
    /// Inclusive RFC3339 UTC lower bound. Default: 24 hours before `to`.
    pub from: Option<String>,
    /// Exclusive RFC3339 UTC upper bound. Default: now.
    pub to: Option<String>,
    /// `api_request` (default), `sandbox_lifecycle`, or `all`.
    pub record_kind: Option<String>,
    /// Exact immutable GitHub id. Global scope only.
    pub actor_id: Option<i64>,
    /// Exact historical login snapshot. Global scope only.
    pub actor_login: Option<String>,
    /// An `operationId` declared by this deployment's audit catalog.
    pub operation_id: Option<String>,
    /// `GET`, `POST`, `PUT`, `PATCH`, or `DELETE`.
    pub method: Option<String>,
    /// Exact HTTP status, 100..=599.
    pub status_code: Option<u16>,
    /// `2xx`, `3xx`, `4xx`, or `5xx`.
    pub status_class: Option<String>,
    /// `success`, `redirect`, `client_error`, `server_error`, `timeout`,
    /// `rejected`, or `incomplete`.
    pub outcome: Option<String>,
    /// Exact session id. Required (and authorized) for lifecycle records in a
    /// regular caller's scope.
    pub session_id: Option<String>,
    /// Exact `owner/name`.
    pub repo_full_name: Option<String>,
    /// Positive trigger-issue number.
    pub trigger_issue: Option<i64>,
    /// Exact propagated request id.
    pub request_id: Option<String>,
    /// The opaque cursor returned by the previous page.
    pub cursor: Option<String>,
    /// Page size, 1..=`FKST_POSTHOG_ACTIVITY_MAX_LIMIT` (default 200).
    pub limit: Option<u32>,
    /// `mine` or `all`. Omitted resolves to `mine` for a regular caller and
    /// `all` for a deployment global administrator.
    pub scope: Option<String>,
}

/// The validated form of one activity request, before authorization.
#[derive(Clone, Debug)]
pub struct NormalizedActivityRequest {
    /// `None` when the caller omitted `scope`.
    pub requested_scope: Option<RequestedScope>,
    /// Whether ANY cross-user actor filter was supplied. A regular caller may
    /// never do that, even in personal scope: the server owns the identity
    /// predicate, so a client-supplied one is always ambiguous authority.
    pub cross_actor_filter: bool,
    pub record_kind: RecordKind,
    pub range: TimeRange,
    /// Whether the caller STATED a bound. A resumed page uses the window its
    /// cursor was issued for; an explicitly stated window that disagrees with it
    /// is a refusal rather than a silent re-window.
    pub range_explicit: bool,
    pub limit: u32,
    pub filters: ActivityFilters,
    /// The raw cursor text. Decoded only after the scope and session are
    /// resolved, because its digest binds both.
    pub cursor: Option<String>,
}

impl NormalizedActivityRequest {
    /// The exact session id the caller named, if any.
    pub fn session_id(&self) -> Option<&str> {
        self.filters.session_id.as_deref()
    }

    /// Whether a cursor was supplied. The cursor TEXT is never recorded.
    pub fn cursor_present(&self) -> bool {
        self.cursor.is_some()
    }
}

/// Validate and normalize the raw parameters.
///
/// Runs before any authorization decision and before any source call, so a
/// malformed request costs the deployment nothing and reveals nothing.
pub fn normalize(
    params: &ActivityQueryParams,
    now: DateTime<Utc>,
    default_limit: u32,
    max_limit: u32,
    max_range_days: u64,
) -> Result<NormalizedActivityRequest, AppError> {
    let requested_scope = params.scope.as_deref().map(parse_scope).transpose()?;
    let record_kind = params
        .record_kind
        .as_deref()
        .map(RecordKind::parse)
        .transpose()?
        .unwrap_or_default();
    let range = filters::resolve_range(
        params.from.as_deref(),
        params.to.as_deref(),
        now,
        max_range_days,
    )?;
    let limit = match params.limit {
        None => default_limit,
        Some(limit) if (1..=max_limit).contains(&limit) => limit,
        Some(_) => {
            return Err(AppError::Validation(format!(
                "limit must be between 1 and {max_limit}"
            )))
        }
    };
    let filters = ActivityFilters {
        actor_id: params.actor_id,
        actor_login: params
            .actor_login
            .as_deref()
            .map(parse_actor_login)
            .transpose()?,
        operation_id: params
            .operation_id
            .as_deref()
            .map(filters::parse_operation_id)
            .transpose()?,
        method: params
            .method
            .as_deref()
            .map(filters::parse_method)
            .transpose()?,
        status_code: params
            .status_code
            .map(filters::parse_status_code)
            .transpose()?,
        status_class: params
            .status_class
            .as_deref()
            .map(filters::StatusClass::parse)
            .transpose()?,
        outcome: params
            .outcome
            .as_deref()
            .map(filters::parse_outcome)
            .transpose()?,
        session_id: params
            .session_id
            .as_deref()
            .map(filters::parse_session_id)
            .transpose()?,
        repo_full_name: params
            .repo_full_name
            .as_deref()
            .map(filters::parse_repo_full_name)
            .transpose()?,
        trigger_issue: params
            .trigger_issue
            .map(filters::parse_trigger_issue)
            .transpose()?,
        request_id: params
            .request_id
            .as_deref()
            .map(filters::parse_request_id)
            .transpose()?,
    };
    Ok(NormalizedActivityRequest {
        requested_scope,
        cross_actor_filter: params.actor_id.is_some() || params.actor_login.is_some(),
        record_kind,
        range,
        range_explicit: params.from.is_some() || params.to.is_some(),
        limit,
        filters,
        cursor: params.cursor.clone(),
    })
}

/// Map the route's `mine`/`all` vocabulary onto the shared scope request.
fn parse_scope(value: &str) -> Result<RequestedScope, AppError> {
    match value.trim() {
        "mine" => Ok(RequestedScope::Personal),
        "all" => Ok(RequestedScope::Global),
        _ => Err(AppError::Validation(
            "scope must be mine or all".to_string(),
        )),
    }
}

/// A GitHub login snapshot, bounded and free of the characters that would let one
/// value forge a field in a structured log.
fn parse_actor_login(value: &str) -> Result<String, AppError> {
    let value = value.trim().trim_start_matches('@');
    let ok = !value.is_empty()
        && value.len() <= 39
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    ok.then(|| value.to_string())
        .ok_or_else(|| AppError::Validation("actor_login is not a valid GitHub login".to_string()))
}

#[cfg(test)]
#[path = "query_tests.rs"]
mod tests;
