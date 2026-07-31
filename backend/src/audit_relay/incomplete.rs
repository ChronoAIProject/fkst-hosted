//! Synthesizing the terminal projection of a request that never finished.
//!
//! A process, Pod, or node can die between "start committed" and "response
//! produced". The durable start is what makes that visible at all; this module
//! decides what the resulting record SAYS, and the rules are entirely about what
//! it must NOT say:
//!
//! - **no fabricated status.** `status_code` stays `null` and `outcome` is
//!   `incomplete`. There was no response, so there is no status, and inventing
//!   one would make every dashboard and every scoped query lie;
//! - **no fabricated actor.** The start is registered BEFORE the handler runs, so
//!   no identity was ever verified for it. The synthesized record therefore
//!   carries the anonymous actor and no `actor_id` — which means it is
//!   global-admin-only, because ownership cannot be proven (epic `AUTH-03`);
//! - **no fabricated arguments.** The safe-argument contract never ran, so the
//!   bag is empty and its status is `unavailable`, not `parsed`;
//! - **the same event id.** It is shipped to PostHog as
//!   [`crate::audit::event::INCOMPLETE_EVENT_NAME`] under the id the start
//!   registered, so a late completion — or a replay — deduplicates against it
//!   instead of producing a second row for one invocation.
//!
//! The terminal instant is the record's own `completion_deadline_at`, not "now":
//! it is the last instant at which the request could still have completed, so the
//! row sorts where the invocation actually happened rather than where the sweep
//! noticed it.

use k8s_openapi::chrono::{DateTime, Utc};
use serde_json::Map;

use crate::audit::event::{
    ActorKind, ArgumentsParseStatus, AuditOutcome, AuthenticationMethod, PrincipalKind,
};

use super::protocol::{
    ActorV1, CorrelationV1, PrincipalV1, ProtocolError, RequestCompletionV1, RequestStartV1,
    PROTOCOL_SCHEMA_VERSION,
};

/// The stable error code every synthesized incomplete record carries.
pub const INCOMPLETE_ERROR_CODE: &str = "request_incomplete";

/// Build the terminal projection for an expired start.
pub fn synthesize(
    start: &RequestStartV1,
) -> Result<(RequestCompletionV1, DateTime<Utc>), ProtocolError> {
    let identity = start.to_identity()?;
    let deadline = start.deadline()?;
    // The deadline is validated to be at or after the start, so this cannot go
    // negative; the clamp is belt-and-braces against a future relaxation.
    let duration_ms = u64::try_from((deadline - identity.started_at).num_milliseconds().max(0))
        .unwrap_or(u64::MAX);

    Ok((
        RequestCompletionV1 {
            schema_version: PROTOCOL_SCHEMA_VERSION,
            event_id: start.event_id.clone(),
            request_id: start.request_id.clone(),
            started_at: start.started_at.clone(),
            completed_at: start.completion_deadline_at.clone(),
            method: start.method.clone(),
            route_template: start.route_template.clone(),
            operation_id: start.operation_id.clone(),
            arguments: Map::new(),
            arguments_parse_status: ArgumentsParseStatus::Unavailable.as_str().to_string(),
            actor_id: None,
            actor: ActorV1 {
                kind: ActorKind::Anonymous.as_str().to_string(),
                id: None,
                login: None,
                authentication: AuthenticationMethod::None.as_str().to_string(),
            },
            principal: PrincipalV1 {
                kind: PrincipalKind::None.as_str().to_string(),
                id: None,
            },
            status_code: None,
            outcome: AuditOutcome::Incomplete.as_str().to_string(),
            error_code: Some(INCOMPLETE_ERROR_CODE.to_string()),
            duration_ms,
            session_id: None,
            correlation: CorrelationV1::default(),
            service_version: start.service_version.clone(),
            deployment_environment: start.deployment_environment.clone(),
        },
        deadline,
    ))
}

#[cfg(test)]
#[path = "incomplete_tests.rs"]
mod tests;
