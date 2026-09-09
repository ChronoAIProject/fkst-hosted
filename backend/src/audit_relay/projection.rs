//! Turning a stored row back into the exact PostHog capture event.
//!
//! The stored body is replayed through the SAME domain types and the SAME
//! [`crate::audit::projection`] / [`crate::audit::lifecycle`] projections the
//! control plane's direct sink uses. That is deliberate: a relay with its own
//! projection would be a second definition of the wire format, and the first time
//! the two disagreed the difference would show up as a silently different history
//! rather than as a failing build.
//!
//! One event name is chosen here rather than by the projection: a synthesized
//! incomplete record ships as [`INCOMPLETE_EVENT_NAME`] under the SAME event id
//! the start registered, so a timeline shows one invocation with an honest
//! "no response was ever produced" instead of a second, invented row.

use crate::audit::event::INCOMPLETE_EVENT_NAME;
use crate::audit::projection::{CaptureEvent, EventLimits};
use crate::audit::validate::EventError;

use super::db::row::StoredRecord;
use super::protocol::{LifecycleEventV1, RequestCompletionV1};
use super::record::RelayRecordKind;

/// Why a stored row could not be projected. Bounded: it names the STAGE, never
/// the content.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ProjectionError {
    #[error("stored record has no terminal projection")]
    NoTerminal,
    #[error("stored record body could not be decoded")]
    Undecodable,
    #[error("stored record violates the audit event contract: {0}")]
    Contract(#[from] EventError),
}

impl ProjectionError {
    /// The bounded metric/log label.
    pub fn as_str(&self) -> &'static str {
        match self {
            ProjectionError::NoTerminal => "no_terminal",
            ProjectionError::Undecodable => "undecodable",
            ProjectionError::Contract(_) => "contract",
        }
    }
}

/// Project one stored row onto the capture wire format.
pub fn capture_event(
    record: &StoredRecord,
    limits: EventLimits,
) -> Result<CaptureEvent, ProjectionError> {
    let terminal = record
        .terminal_json
        .as_deref()
        .ok_or(ProjectionError::NoTerminal)?;
    match record.record_kind {
        RelayRecordKind::ApiRequest => {
            let wire: RequestCompletionV1 =
                serde_json::from_slice(terminal).map_err(|_| ProjectionError::Undecodable)?;
            let domain = wire.to_domain().map_err(|_| ProjectionError::Undecodable)?;
            let incomplete = domain.outcome == crate::audit::event::AuditOutcome::Incomplete;
            let mut projected = domain.to_capture_event(limits)?;
            if incomplete {
                projected.event = INCOMPLETE_EVENT_NAME;
            }
            Ok(projected)
        }
        RelayRecordKind::SandboxLifecycle => {
            let wire: LifecycleEventV1 =
                serde_json::from_slice(terminal).map_err(|_| ProjectionError::Undecodable)?;
            let domain = wire.to_domain().map_err(|_| ProjectionError::Undecodable)?;
            Ok(domain.to_capture_event(limits)?)
        }
    }
}

#[cfg(test)]
#[path = "projection_tests.rs"]
mod tests;
