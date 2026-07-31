//! Fail-closed validation of a sandbox lifecycle record.
//!
//! Split from [`super::lifecycle`] for the same reason
//! [`super::validate`] is split from [`super::event`]: "is this record legal"
//! is a redaction and authorization concern, while the contract next door is a
//! domain one, and mixing them is how a bound quietly stops being checked.
//!
//! Two of these checks are load-bearing rather than defensive:
//!
//! - the **session id** is what a scoped read authorizes on, so it is validated
//!   against the canonical derived form's shape rather than merely being
//!   non-empty. An arbitrary string must never be able to ride that field into
//!   the analytics store and become a selectable scope;
//! - an **empty optional string** is rejected rather than sent, because an empty
//!   value is a field that should have been absent, and the difference between
//!   "unknown" and "known to be blank" is exactly what an audit trail is for.

use super::lifecycle::{SandboxLifecycleV1, LIFECYCLE_SCHEMA_VERSION};
use super::validate::{limits, EventError};

/// Maximum length of the bounded runtime/incarnation identifiers.
const RUNTIME_ID_LIMIT: usize = 128;

/// Fail-closed validation of a lifecycle record.
///
/// The session id is checked against the canonical derived form's shape
/// (lowercase hyphenated, alphanumeric-bounded, bounded length) rather than just
/// "non-empty": this field is what a scoped read authorizes on, so an arbitrary
/// string must never be able to ride it into the analytics store.
pub fn validate_lifecycle(event: &SandboxLifecycleV1) -> Result<(), EventError> {
    if event.schema_version != LIFECYCLE_SCHEMA_VERSION {
        return Err(EventError::Invalid {
            field: "schema_version",
            reason: format!("expected {LIFECYCLE_SCHEMA_VERSION}"),
        });
    }
    validate_session_id(&event.session_id)?;
    bounded("runtime_id", &event.runtime.runtime_id, RUNTIME_ID_LIMIT)?;
    bounded(
        "incarnation_hint",
        &event.runtime.incarnation_hint,
        RUNTIME_ID_LIMIT,
    )?;
    bounded(
        "creator_login",
        &event.attribution.creator_login,
        limits::LOGIN,
    )?;
    bounded(
        "trigger_author_login",
        &event.attribution.trigger_author_login,
        limits::LOGIN,
    )?;
    bounded("actor_login", &event.actor.login, limits::LOGIN)?;
    bounded("principal_id", &event.principal.id, limits::PRINCIPAL_ID)?;
    bounded(
        "repo_full_name",
        &event.correlation.repo_full_name,
        limits::REPO_FULL_NAME,
    )?;
    bounded(
        "request_id",
        &event.correlation.request_id,
        limits::REQUEST_ID,
    )?;
    bounded(
        "service_version",
        &Some(event.service.version.clone()),
        limits::SERVICE_VERSION,
    )?;
    bounded(
        "service_environment",
        &Some(event.service.environment.clone()),
        limits::SERVICE_ENVIRONMENT,
    )?;
    Ok(())
}

/// The canonical session id is a lowercase hyphenated UUID
/// ([`crate::session_spec::derive_session_id`]); this admits that shape and
/// rejects anything that could smuggle free text, a path, or a query string.
fn validate_session_id(session_id: &str) -> Result<(), EventError> {
    let invalid = |reason: &str| EventError::Invalid {
        field: "session_id",
        reason: reason.to_string(),
    };
    let bytes = session_id.as_bytes();
    if bytes.is_empty() {
        return Err(invalid("must not be empty"));
    }
    if bytes.len() > limits::SESSION_ID {
        return Err(invalid("exceeds the maximum length"));
    }
    let alnum = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit();
    if !alnum(bytes[0]) || !alnum(bytes[bytes.len() - 1]) {
        return Err(invalid("must start and end with a lowercase alphanumeric"));
    }
    if !bytes.iter().all(|&b| alnum(b) || b == b'-') {
        return Err(invalid("must be lowercase alphanumerics and hyphens"));
    }
    Ok(())
}

/// Bound an optional string field, rejecting an empty one (an empty value is a
/// field that should have been absent).
fn bounded(field: &'static str, value: &Option<String>, limit: usize) -> Result<(), EventError> {
    let Some(value) = value else {
        return Ok(());
    };
    if value.is_empty() {
        return Err(EventError::Invalid {
            field,
            reason: "must be absent rather than empty".to_string(),
        });
    }
    if value.len() > limit {
        return Err(EventError::Invalid {
            field,
            reason: format!("exceeds {limit} bytes"),
        });
    }
    Ok(())
}
