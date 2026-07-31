//! Fail-closed validation of a completed audit record.
//!
//! Validation runs before an event is ever projected onto the wire. It exists for
//! three distinct reasons, all of them load-bearing:
//!
//! 1. **Authorization support.** The read side filters on the canonical top-level
//!    `actor_id`. A record whose canonical and nested ids disagree would let a
//!    row be attributed to the wrong person, so it is rejected outright rather
//!    than "fixed up" — silently rewriting an identity is exactly the failure a
//!    verified-actor contract must not have.
//! 2. **Redaction.** A route template containing `?` means the caller handed us a
//!    raw URI instead of the matched template, which would smuggle query values
//!    (potentially secrets) into the analytics store. Same for a free-text error
//!    code.
//! 3. **Delivery safety.** Bounded string lengths and a bounded serialized size
//!    keep one pathological record from poisoning a whole batch.
//!
//! Every failure is a typed [`EventError`] naming the field; the caller turns it
//! into a drop metric and a structured log, never a silent discard.

use super::event::{ActorKind, ApiRequestCompletedV1, AuditOutcome};

/// Explicitly documented bounds for the individual safe string fields. They are
/// deliberately generous relative to real values (a GitHub login is at most 39
/// characters) — their job is to stop unbounded growth, not to re-validate the
/// upstream format.
pub mod limits {
    pub const REQUEST_ID: usize = 128;
    pub const METHOD: usize = 16;
    pub const ROUTE_TEMPLATE: usize = 256;
    pub const OPERATION_ID: usize = 128;
    pub const LOGIN: usize = 64;
    pub const PRINCIPAL_ID: usize = 128;
    pub const ERROR_CODE: usize = 64;
    pub const SESSION_ID: usize = 128;
    pub const REPO_FULL_NAME: usize = 256;
    pub const WEBHOOK_DELIVERY_ID: usize = 64;
    pub const SERVICE_VERSION: usize = 64;
    pub const SERVICE_ENVIRONMENT: usize = 64;
}

/// Why a record cannot be sent.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum EventError {
    /// A field violates the contract. `reason` is developer-facing text about
    /// the FIELD, never the offending value (which may be sensitive).
    #[error("audit event field `{field}` is invalid: {reason}")]
    Invalid { field: &'static str, reason: String },
    /// The projected event exceeds the configured maximum serialized size.
    /// Never truncated: arbitrary JSON cannot be shortened without corrupting
    /// the record's meaning.
    #[error("audit event is {actual} bytes, over the {limit}-byte maximum")]
    TooLarge { actual: usize, limit: usize },
    /// The projection could not be serialized at all.
    #[error("audit event could not be serialized: {0}")]
    Unserializable(String),
}

impl EventError {
    fn invalid(field: &'static str, reason: impl Into<String>) -> Self {
        Self::Invalid {
            field,
            reason: reason.into(),
        }
    }
}

/// Validate a completed record. `Ok(())` means it is safe to project and send.
pub fn validate(event: &ApiRequestCompletedV1) -> Result<(), EventError> {
    if event.schema_version != super::event::SCHEMA_VERSION {
        return Err(EventError::invalid(
            "schema_version",
            format!("expected {}", super::event::SCHEMA_VERSION),
        ));
    }
    bounded("request_id", &event.request_id, limits::REQUEST_ID, true)?;
    validate_method(&event.method)?;
    validate_route_template(&event.route_template)?;
    bounded(
        "operation_id",
        &event.operation_id,
        limits::OPERATION_ID,
        true,
    )?;
    validate_timestamps(event)?;
    validate_status_and_outcome(event.status_code, event.outcome)?;
    validate_error_code(event.error_code.as_deref())?;
    validate_actor(event)?;
    validate_principal(event)?;
    validate_correlation(event)?;
    bounded(
        "service.version",
        &event.service.version,
        limits::SERVICE_VERSION,
        true,
    )?;
    bounded(
        "service.environment",
        &event.service.environment,
        limits::SERVICE_ENVIRONMENT,
        false,
    )?;
    Ok(())
}

/// A non-empty (when `required`) string within `max` bytes, with no control
/// characters — a newline in a value would corrupt structured logs and metric
/// exposition alike.
fn bounded(field: &'static str, value: &str, max: usize, required: bool) -> Result<(), EventError> {
    if required && value.trim().is_empty() {
        return Err(EventError::invalid(field, "must not be empty"));
    }
    if value.len() > max {
        return Err(EventError::invalid(
            field,
            format!("must be at most {max} bytes (got {})", value.len()),
        ));
    }
    if value.chars().any(char::is_control) {
        return Err(EventError::invalid(
            field,
            "must not contain control characters",
        ));
    }
    Ok(())
}

fn validate_method(method: &str) -> Result<(), EventError> {
    bounded("method", method, limits::METHOD, true)?;
    if !method
        .chars()
        .all(|c| c.is_ascii_uppercase() || c == '-' || c == '_')
    {
        return Err(EventError::invalid(
            "method",
            "must be an uppercase HTTP method",
        ));
    }
    Ok(())
}

fn validate_route_template(route: &str) -> Result<(), EventError> {
    bounded("route_template", route, limits::ROUTE_TEMPLATE, true)?;
    // A `?` (or `#`) means the caller passed a raw URI, not the matched
    // template: query values are forbidden data (epic `AUD-03`).
    if route.contains('?') || route.contains('#') {
        return Err(EventError::invalid(
            "route_template",
            "must be the normalized matched route, not a raw query-bearing URI",
        ));
    }
    if !route.starts_with('/') {
        return Err(EventError::invalid("route_template", "must start with `/`"));
    }
    Ok(())
}

fn validate_timestamps(event: &ApiRequestCompletedV1) -> Result<(), EventError> {
    if event.completed_at < event.started_at {
        return Err(EventError::invalid(
            "completed_at",
            "must not precede started_at",
        ));
    }
    let elapsed = (event.completed_at - event.started_at)
        .num_milliseconds()
        .max(0);
    if u64::try_from(elapsed).unwrap_or(u64::MAX) != event.duration_ms {
        return Err(EventError::invalid(
            "duration_ms",
            "must equal completed_at - started_at",
        ));
    }
    Ok(())
}

/// The status/outcome matrix. An outcome that contradicts the returned status
/// would make every dashboard and every scoped query lie.
fn validate_status_and_outcome(
    status_code: Option<u16>,
    outcome: AuditOutcome,
) -> Result<(), EventError> {
    let Some(status) = status_code else {
        // Only a record that genuinely never produced a response may omit the
        // status; a timeout that DID return 504 carries it.
        return match outcome {
            AuditOutcome::Incomplete | AuditOutcome::Timeout => Ok(()),
            other => Err(EventError::invalid(
                "status_code",
                format!("is required for outcome `{other}`"),
            )),
        };
    };
    if !(100..=599).contains(&status) {
        return Err(EventError::invalid(
            "status_code",
            "must be in the range 100..=599",
        ));
    }
    let class_ok = match outcome {
        AuditOutcome::Success => (200..300).contains(&status),
        AuditOutcome::Redirect => (300..400).contains(&status),
        AuditOutcome::ClientError => (400..500).contains(&status),
        AuditOutcome::ServerError => (500..600).contains(&status),
        // A timeout surfaces as 408 (request) or 504 (upstream/handler budget).
        AuditOutcome::Timeout => status == 408 || status == 504,
        // A pre-handler rejection is a 4xx auth/policy answer, or the 503 the
        // leader-readiness gate returns on a follower replica.
        AuditOutcome::Rejected => (400..500).contains(&status) || status == 503,
        // An incomplete record by definition has no status.
        AuditOutcome::Incomplete => false,
    };
    if !class_ok {
        return Err(EventError::invalid(
            "outcome",
            format!("`{outcome}` is not a valid outcome for status {status}"),
        ));
    }
    Ok(())
}

fn validate_error_code(error_code: Option<&str>) -> Result<(), EventError> {
    let Some(code) = error_code else {
        return Ok(());
    };
    bounded("error_code", code, limits::ERROR_CODE, true)?;
    // Stable machine codes only (`invalid_request`, `session_visibility_unavailable`).
    // Anything with spaces or punctuation is error TEXT, which is forbidden data.
    if !code
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        return Err(EventError::invalid(
            "error_code",
            "must be a stable snake_case application code, never error text",
        ));
    }
    Ok(())
}

/// Actor invariants, including the canonical/nested agreement that makes the
/// flat `actor_id` trustworthy for source-level authorization.
fn validate_actor(event: &ApiRequestCompletedV1) -> Result<(), EventError> {
    if let Some(login) = &event.actor.login {
        bounded("actor.login", login, limits::LOGIN, true)?;
    }
    if event.actor.kind.is_human() {
        // The canonical field is what source-level authorization filters on, so
        // a disagreement is rejected rather than reconciled: silently choosing
        // one of two identities is how a row ends up attributed to the wrong
        // person.
        if event.actor_id != event.actor.id {
            return Err(EventError::invalid(
                "actor_id",
                "canonical actor id must equal actor.id for a human actor",
            ));
        }
        match event.actor.id {
            // A verified user always has an id; a record claiming otherwise is
            // an identity bug, not an anonymous request.
            None if event.actor.kind == ActorKind::GithubUser => {
                return Err(EventError::invalid(
                    "actor.id",
                    "a verified GitHub user must carry its immutable id",
                ))
            }
            // A webhook whose sender GitHub did not name stays unattributed; it
            // is then global-admin-only, exactly like anonymous traffic.
            None => {}
            Some(id) if id <= 0 => {
                return Err(EventError::invalid(
                    "actor.id",
                    "a GitHub user id is a positive integer",
                ))
            }
            Some(_) => {}
        }
    } else {
        // A service/system/anonymous record must never appear to belong to a
        // person: that is what keeps unattributed rows global-admin-only.
        if event.actor_id.is_some() || event.actor.id.is_some() {
            return Err(EventError::invalid(
                "actor_id",
                "a non-human actor must not carry a GitHub user id",
            ));
        }
        if event.actor.kind == ActorKind::Anonymous && event.actor.login.is_some() {
            return Err(EventError::invalid(
                "actor.login",
                "an anonymous actor must not carry a login",
            ));
        }
    }
    Ok(())
}

fn validate_principal(event: &ApiRequestCompletedV1) -> Result<(), EventError> {
    if let Some(id) = &event.principal.id {
        bounded("principal.id", id, limits::PRINCIPAL_ID, true)?;
    }
    Ok(())
}

fn validate_correlation(event: &ApiRequestCompletedV1) -> Result<(), EventError> {
    if event.session_id != event.correlation.session_id {
        return Err(EventError::invalid(
            "session_id",
            "canonical session id must equal correlation.session_id",
        ));
    }
    if let Some(session_id) = &event.session_id {
        bounded("session_id", session_id, limits::SESSION_ID, true)?;
    }
    if let Some(repo) = &event.correlation.repo_full_name {
        bounded(
            "correlation.repo_full_name",
            repo,
            limits::REPO_FULL_NAME,
            true,
        )?;
        let mut parts = repo.split('/');
        let owner = parts.next().unwrap_or_default();
        let name = parts.next().unwrap_or_default();
        if owner.is_empty() || name.is_empty() || parts.next().is_some() {
            return Err(EventError::invalid(
                "correlation.repo_full_name",
                "must be `owner/name`",
            ));
        }
    }
    if let Some(delivery) = &event.correlation.webhook_delivery_id {
        bounded(
            "correlation.webhook_delivery_id",
            delivery,
            limits::WEBHOOK_DELIVERY_ID,
            true,
        )?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "validate_tests.rs"]
mod tests;
