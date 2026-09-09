//! The closed filter vocabulary of the activity query, and its normalization.
//!
//! Every accepted filter is a FIXED field with a validated value. There is
//! deliberately no free-form property name, sort expression, text search, raw
//! JSON filter, event name, or HogQL fragment: the query text is server-owned
//! (see [`super::hogql`]), and a caller can only ever choose WHICH of the fixed
//! snippets are switched on and what value rides in the accompanying parameter.
//!
//! Normalization happens once, here, before anything is authorized or queried:
//!
//! - a value that is not in its validated form is a `400`, never a silently
//!   dropped predicate — a filter that quietly disappeared would show the caller
//!   a wider result set than they asked for;
//! - the normalized form is what goes into the audit record's safe arguments,
//!   into the source predicate's parameter, AND into the cursor's binding digest,
//!   so those three can never describe different queries.

use k8s_openapi::chrono::{DateTime, Duration, SecondsFormat, Utc};

use crate::audit::arguments::bounds::{safe_owner, safe_repo, safe_session_id};
use crate::audit::event::{AuditOutcome, UNMATCHED_OPERATION_ID};
use crate::audit::request::id::is_acceptable as is_acceptable_request_id;
use crate::audit::request::policy::{declared_operation_ids, RESERVED_ARGUMENT_POLICIES};
use crate::error::AppError;

/// The HTTP methods a recorded request can carry. Closed: the audit contract
/// records an uppercase method, and nothing else may reach the query.
pub const ALLOWED_METHODS: [&str; 5] = ["GET", "POST", "PUT", "PATCH", "DELETE"];

/// Which record kinds one query asks for.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RecordKind {
    /// Only API-request rows. The default: a personal timeline needs no session
    /// authorization, so the cheapest, least-privileged shape is what an omitted
    /// parameter resolves to.
    #[default]
    ApiRequest,
    /// Only system sandbox lifecycle rows.
    SandboxLifecycle,
    /// Both, merged into one timeline.
    All,
}

impl RecordKind {
    /// The stable wire string; safe as a closed-enum metric label.
    pub fn as_str(self) -> &'static str {
        match self {
            RecordKind::ApiRequest => "api_request",
            RecordKind::SandboxLifecycle => "sandbox_lifecycle",
            RecordKind::All => "all",
        }
    }

    /// Parse the query parameter.
    pub fn parse(value: &str) -> Result<Self, AppError> {
        match value.trim() {
            "api_request" => Ok(RecordKind::ApiRequest),
            "sandbox_lifecycle" => Ok(RecordKind::SandboxLifecycle),
            "all" => Ok(RecordKind::All),
            _ => Err(AppError::Validation(
                "record_kind must be api_request, sandbox_lifecycle, or all".to_string(),
            )),
        }
    }

    /// Whether this kind can return system lifecycle rows — the kinds that need
    /// an authorized exact session in a regular caller's scope.
    pub fn includes_lifecycle(self) -> bool {
        matches!(self, RecordKind::SandboxLifecycle | RecordKind::All)
    }

    /// Whether this kind can return API-request rows.
    pub fn includes_api_requests(self) -> bool {
        matches!(self, RecordKind::ApiRequest | RecordKind::All)
    }
}

/// A status-code family filter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StatusClass {
    Success,
    Redirect,
    ClientError,
    ServerError,
}

impl StatusClass {
    /// The stable wire string.
    pub fn as_str(self) -> &'static str {
        match self {
            StatusClass::Success => "2xx",
            StatusClass::Redirect => "3xx",
            StatusClass::ClientError => "4xx",
            StatusClass::ServerError => "5xx",
        }
    }

    /// Parse the query parameter.
    pub fn parse(value: &str) -> Result<Self, AppError> {
        match value.trim() {
            "2xx" => Ok(StatusClass::Success),
            "3xx" => Ok(StatusClass::Redirect),
            "4xx" => Ok(StatusClass::ClientError),
            "5xx" => Ok(StatusClass::ServerError),
            _ => Err(AppError::Validation(
                "status_class must be 2xx, 3xx, 4xx, or 5xx".to_string(),
            )),
        }
    }

    /// The half-open `[low, high)` status window this class covers.
    pub fn bounds(self) -> (u16, u16) {
        match self {
            StatusClass::Success => (200, 300),
            StatusClass::Redirect => (300, 400),
            StatusClass::ClientError => (400, 500),
            StatusClass::ServerError => (500, 600),
        }
    }
}

/// Parse an `outcome` filter against the audit contract's closed set.
pub fn parse_outcome(value: &str) -> Result<AuditOutcome, AppError> {
    let candidates = [
        AuditOutcome::Success,
        AuditOutcome::Redirect,
        AuditOutcome::ClientError,
        AuditOutcome::ServerError,
        AuditOutcome::Timeout,
        AuditOutcome::Rejected,
        AuditOutcome::Incomplete,
    ];
    let value = value.trim();
    candidates
        .into_iter()
        .find(|candidate| candidate.as_str() == value)
        .ok_or_else(|| {
            AppError::Validation(
                "outcome must be success, redirect, client_error, server_error, timeout, \
                 rejected, or incomplete"
                    .to_string(),
            )
        })
}

/// Accept an `operation_id` only when the server's own audit catalog declares it.
///
/// A caller cannot invent an operation: the value goes into a parameterized
/// predicate either way, but refusing an unknown id turns a typo into an
/// immediate `400` instead of a confidently empty page, and it keeps the accepted
/// vocabulary identical to the published contract.
pub fn parse_operation_id(value: &str) -> Result<String, AppError> {
    let value = value.trim();
    let known = value == UNMATCHED_OPERATION_ID
        || declared_operation_ids().any(|id| id == value)
        || RESERVED_ARGUMENT_POLICIES
            .iter()
            .any(|operation| operation.operation_id == value);
    known.then(|| value.to_string()).ok_or_else(|| {
        AppError::Validation(
            "operation_id must be an operation id declared by this deployment's audit catalog"
                .to_string(),
        )
    })
}

/// The normalized, validated filter set for one query.
///
/// Only [`normalize`] builds one, so every field here has already passed its
/// validator. `actor_id`/`actor_login` are present only in a global-admin scope —
/// the scope gate refuses them for a regular caller before this struct is built.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ActivityFilters {
    pub actor_id: Option<i64>,
    pub actor_login: Option<String>,
    pub operation_id: Option<String>,
    pub method: Option<String>,
    pub status_code: Option<u16>,
    pub status_class: Option<StatusClass>,
    pub outcome: Option<AuditOutcome>,
    pub session_id: Option<String>,
    pub repo_full_name: Option<String>,
    pub trigger_issue: Option<i64>,
    pub request_id: Option<String>,
}

impl ActivityFilters {
    /// The canonical `field=value` pairs this filter set contributes to the
    /// cursor's binding digest.
    ///
    /// Emitted in a FIXED order with absent fields skipped, so two requests bind
    /// to the same digest exactly when they carry the same filters — which is what
    /// makes a cursor issued under one filter set unusable under another.
    pub fn binding_fields(&self) -> Vec<String> {
        let mut fields = Vec::new();
        let mut push = |key: &str, value: String| fields.push(format!("{key}={value}"));
        if let Some(actor_id) = self.actor_id {
            push("actor_id", actor_id.to_string());
        }
        if let Some(login) = &self.actor_login {
            push("actor_login", login.clone());
        }
        if let Some(operation_id) = &self.operation_id {
            push("operation_id", operation_id.clone());
        }
        if let Some(method) = &self.method {
            push("method", method.clone());
        }
        if let Some(status_code) = self.status_code {
            push("status_code", status_code.to_string());
        }
        if let Some(status_class) = self.status_class {
            push("status_class", status_class.as_str().to_string());
        }
        if let Some(outcome) = self.outcome {
            push("outcome", outcome.as_str().to_string());
        }
        if let Some(session_id) = &self.session_id {
            push("session_id", session_id.clone());
        }
        if let Some(repo) = &self.repo_full_name {
            push("repo_full_name", repo.clone());
        }
        if let Some(issue) = self.trigger_issue {
            push("trigger_issue", issue.to_string());
        }
        if let Some(request_id) = &self.request_id {
            push("request_id", request_id.clone());
        }
        fields
    }
}

/// Accept an uppercase HTTP method from the closed recorded set.
pub fn parse_method(value: &str) -> Result<String, AppError> {
    let upper = value.trim().to_ascii_uppercase();
    ALLOWED_METHODS
        .contains(&upper.as_str())
        .then_some(upper)
        .ok_or_else(|| {
            AppError::Validation("method must be GET, POST, PUT, PATCH, or DELETE".to_string())
        })
}

/// Accept an exact HTTP status code.
pub fn parse_status_code(value: u16) -> Result<u16, AppError> {
    (100..=599)
        .contains(&value)
        .then_some(value)
        .ok_or_else(|| AppError::Validation("status_code must be between 100 and 599".to_string()))
}

/// Accept an exact `owner/name` where BOTH halves pass the product's own
/// repository validators.
pub fn parse_repo_full_name(value: &str) -> Result<String, AppError> {
    let invalid =
        || AppError::Validation("repo_full_name must be an exact owner/name pair".to_string());
    let (owner, name) = value.trim().split_once('/').ok_or_else(invalid)?;
    let owner = safe_owner(owner).ok_or_else(invalid)?;
    let name = safe_repo(name).ok_or_else(invalid)?;
    Ok(format!("{owner}/{name}"))
}

/// Accept an exact session id in the audit contract's validated form.
pub fn parse_session_id(value: &str) -> Result<String, AppError> {
    safe_session_id(value)
        .ok_or_else(|| AppError::Validation("session_id is not a valid session id".to_string()))
}

/// Accept a positive issue number.
pub fn parse_trigger_issue(value: i64) -> Result<i64, AppError> {
    (value > 0)
        .then_some(value)
        .ok_or_else(|| AppError::Validation("trigger_issue must be a positive integer".to_string()))
}

/// Accept a request id in the exact bounded form the middleware propagates.
pub fn parse_request_id(value: &str) -> Result<String, AppError> {
    let value = value.trim();
    is_acceptable_request_id(value)
        .then(|| value.to_string())
        .ok_or_else(|| AppError::Validation("request_id is not a valid request id".to_string()))
}

/// A validated, half-open `[from, to)` UTC window.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimeRange {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
}

impl TimeRange {
    /// RFC3339 UTC with millisecond precision — the exact form the audit contract
    /// writes timestamps in, so a boundary compares the way a reader expects.
    pub fn from_rfc3339(&self) -> String {
        self.from.to_rfc3339_opts(SecondsFormat::Millis, true)
    }

    /// See [`TimeRange::from_rfc3339`].
    pub fn to_rfc3339(&self) -> String {
        self.to.to_rfc3339_opts(SecondsFormat::Millis, true)
    }
}

/// Default window when neither bound is supplied: the last 24 hours.
const DEFAULT_RANGE_HOURS: i64 = 24;

/// Resolve `from`/`to` against `now`, applying the default window and the
/// deployment's configured maximum span.
///
/// The bounds are half-open (`from` inclusive, `to` exclusive) so consecutive
/// windows tile without double-counting the instant they share.
pub fn resolve_range(
    from: Option<&str>,
    to: Option<&str>,
    now: DateTime<Utc>,
    max_range_days: u64,
) -> Result<TimeRange, AppError> {
    let to = match to {
        Some(raw) => parse_instant("to", raw)?,
        None => now,
    };
    let from = match from {
        Some(raw) => parse_instant("from", raw)?,
        None => to - Duration::hours(DEFAULT_RANGE_HOURS),
    };
    let range = TimeRange { from, to };
    check_range(&range, now, max_range_days)?;
    Ok(range)
}

/// Apply the deployment's window bounds to an ALREADY-ASSEMBLED range.
///
/// Split out of [`resolve_range`] because a range reaches the query by two
/// routes, and both must be bounded identically. The second route is a resumed
/// page: its window comes out of the caller's cursor payload, and the cursor's
/// digest is a plain SHA-256 over public data — explicitly not a MAC (see
/// [`super::cursor`]). Every other digest component is re-derived server-side
/// from the current request, which makes the window the one input a caller can
/// choose freely and still produce a matching digest. Re-checking it here is
/// what keeps `FKST_POSTHOG_ACTIVITY_MAX_RANGE_DAYS` a real bound instead of a
/// bound on the non-cursor path only (epic `OPS-02`).
pub fn check_range(
    range: &TimeRange,
    now: DateTime<Utc>,
    max_range_days: u64,
) -> Result<(), AppError> {
    if range.from >= range.to {
        return Err(AppError::Validation(
            "from must be strictly before to".to_string(),
        ));
    }
    let max = Duration::days(i64::try_from(max_range_days).unwrap_or(i64::MAX));
    if range.to - range.from > max {
        return Err(AppError::Validation(format!(
            "the requested range exceeds this deployment's maximum of {max_range_days} days"
        )));
    }
    // A window entirely in the future can only ever be empty, and asking for one
    // is a client bug worth naming rather than answering with a confident empty
    // page. A window that merely ENDS in the future is fine — "up to now-ish" is
    // how a live view is written.
    if range.from > now {
        return Err(AppError::Validation(
            "from must not be in the future".to_string(),
        ));
    }
    Ok(())
}

/// Parse one RFC3339 bound into UTC, naming the parameter that failed.
fn parse_instant(field: &str, raw: &str) -> Result<DateTime<Utc>, AppError> {
    DateTime::parse_from_rfc3339(raw.trim())
        .map(|parsed| parsed.with_timezone(&Utc))
        .map_err(|_| AppError::Validation(format!("{field} must be an RFC3339 UTC timestamp")))
}

#[cfg(test)]
#[path = "filters_tests.rs"]
mod tests;
