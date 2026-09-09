//! The closed, exact filter vocabulary of the sandbox inventory.
//!
//! Every filter here is a NARROWING predicate evaluated on rows the caller is
//! ALREADY authorized to see (see [`super::service::run`]). That ordering is what
//! makes the whole vocabulary safe: `creator_id`, `creator_login`,
//! `repo_full_name`, and `session_id` all name somebody else's property, and if
//! they ran before authorization every one of them would be a cross-user probe.
//! Running them after means the widest possible answer to any of them is still
//! the caller's own authorized set.
//!
//! ## What is deliberately not accepted
//!
//! No arbitrary field, no regex, no label selector, no namespace/project
//! override, no raw status-message search, no Kubernetes selector, no OpenSandbox
//! metadata filter, no session-access entry, and no caller identity. Each of
//! those is either a query language (unbounded cost, unbounded leakage) or an
//! attempt to name the authorization input the server owns.
//!
//! ## Exact means exact
//!
//! A filtered field that the runtime does not carry never matches. An item with
//! no `creator_id` is not returned by `creator_id=…`, and is not returned by
//! "everything except" either — there is no negation. Logins and `owner/name`
//! compare ASCII-case-insensitively because GitHub itself does; ids compare
//! numerically because the id is the only immutable identifier.

use crate::error::AppError;
use crate::runtime_identity::{AttributionSource, RuntimeBackendKind};
use crate::session_backend::inventory::{RuntimeInventoryItem, RuntimeInventoryStatus};

use crate::audit::arguments::bounds::{safe_owner, safe_repo, safe_session_id};

/// The longest GitHub login GitHub itself will issue.
const MAX_LOGIN_LEN: usize = 39;

/// One request's validated filters. Every field is optional; an all-`None` value
/// matches every authorized row.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SandboxFilters {
    pub status: Option<RuntimeInventoryStatus>,
    pub backend: Option<RuntimeBackendKind>,
    pub creator_id: Option<i64>,
    pub creator_login: Option<String>,
    pub repo_full_name: Option<String>,
    pub session_id: Option<String>,
    pub trigger_issue: Option<i64>,
    pub attribution_source: Option<AttributionSource>,
}

impl SandboxFilters {
    /// Whether one already-authorized runtime matches every stated filter.
    pub fn matches(&self, item: &RuntimeInventoryItem) -> bool {
        self.status.is_none_or(|status| item.status == status)
            && self.backend.is_none_or(|backend| item.backend == backend)
            && self.creator_id.is_none_or(|id| item.creator_id == Some(id))
            && self
                .creator_login
                .as_deref()
                .is_none_or(|login| matches_login(item.creator_login.as_deref(), login))
            && self
                .repo_full_name
                .as_deref()
                .is_none_or(|repo| matches_login(item.repo_full_name.as_deref(), repo))
            && self
                .session_id
                .as_deref()
                .is_none_or(|session| item.session_id.as_deref() == Some(session))
            && self
                .trigger_issue
                .is_none_or(|issue| item.trigger_issue == Some(issue))
            && self
                .attribution_source
                .is_none_or(|source| item.attribution_source == source)
    }

    /// The exact session id the caller named, if any.
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }
}

/// ASCII-case-insensitive equality against an optional runtime value. An absent
/// value never matches: "unknown" is not "equal to whatever you asked for".
fn matches_login(value: Option<&str>, expected: &str) -> bool {
    value.is_some_and(|value| value.eq_ignore_ascii_case(expected))
}

/// Accept a normalized runtime status from the inventory's own closed set.
pub fn parse_status(value: &str) -> Result<RuntimeInventoryStatus, AppError> {
    RuntimeInventoryStatus::parse(value.trim())
        .ok_or_else(|| AppError::Validation("status is not a known runtime status".to_string()))
}

/// Accept a runtime backend from the closed set. A value naming the backend this
/// deployment does NOT run is still valid syntax — it simply matches no row —
/// because answering `400` would make the endpoint a configuration oracle.
pub fn parse_backend(value: &str) -> Result<RuntimeBackendKind, AppError> {
    RuntimeBackendKind::parse(value.trim())
        .ok_or_else(|| AppError::Validation("backend is not a known runtime backend".to_string()))
}

/// Accept an attribution source from #5673's closed set.
pub fn parse_attribution_source(value: &str) -> Result<AttributionSource, AppError> {
    AttributionSource::parse(value.trim()).ok_or_else(|| {
        AppError::Validation("attribution_source is not a known attribution source".to_string())
    })
}

/// Accept a positive immutable GitHub numeric id.
pub fn parse_creator_id(value: i64) -> Result<i64, AppError> {
    (value > 0)
        .then_some(value)
        .ok_or_else(|| AppError::Validation("creator_id must be a positive integer".to_string()))
}

/// Accept a GitHub login snapshot, bounded and free of the characters that would
/// let one value forge a field in a structured log.
pub fn parse_creator_login(value: &str) -> Result<String, AppError> {
    let value = value.trim().trim_start_matches('@');
    let ok = !value.is_empty()
        && value.len() <= MAX_LOGIN_LEN
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    ok.then(|| value.to_string()).ok_or_else(|| {
        AppError::Validation("creator_login is not a valid GitHub login".to_string())
    })
}

/// Accept an exact `owner/name` pair in the validated form the rest of the audit
/// surface enforces.
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
    safe_session_id(value.trim())
        .ok_or_else(|| AppError::Validation("session_id is not a valid session id".to_string()))
}

/// Accept a positive trigger-issue number.
pub fn parse_trigger_issue(value: i64) -> Result<i64, AppError> {
    (value > 0)
        .then_some(value)
        .ok_or_else(|| AppError::Validation("trigger_issue must be a positive integer".to_string()))
}

#[cfg(test)]
#[path = "filters_tests.rs"]
mod tests;
