//! Wire types for the internal controller<->worker protocol.
//!
//! Every request is authenticated by the shared secret in the
//! [`INTERNAL_AUTH_HEADER`] and carries an inline `protocol_version` checked
//! against [`PROTOCOL_VERSION`]. The lifecycle state machine is
//! `Active -> Draining -> Terminated`; the drain message vocabulary
//! ([`Draining`], [`Released`], [`ControlMessage::StopSession`], and the
//! `lifecycle_state` on [`Heartbeat`]) is defined here so the elasticity flow
//! (#140) only adds behaviour, never new wire types.
//!
//! Input validation: every request struct is `#[serde(deny_unknown_fields)]`
//! so malformed/extra-field input is rejected at the trust boundary.

use serde::{Deserialize, Serialize};

/// Header carrying the shared internal-auth secret on every internal request.
pub const INTERNAL_AUTH_HEADER: &str = "x-fkst-internal-auth";

/// Current internal protocol version. Bumped on any breaking wire change.
pub const PROTOCOL_VERSION: u32 = 1;

/// Worker lifecycle state reported on every heartbeat. `Terminated` is not a
/// wire-reported state (a terminated worker simply stops heartbeating and is
/// expired by the controller), so only the two reportable states exist here.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleState {
    /// Accepting and running work.
    #[default]
    Active,
    /// Draining: no longer pulling new work, flushing/handing off live work.
    Draining,
}

/// Worker registration request (worker -> controller). `worker_id` defaults to
/// the k8s pod name (`FKST_POD_ID`) and must be non-empty. `capacity` is the
/// max concurrent engine sessions the worker accepts (0 = derive later).
/// `engine_temp_root` is the worker's engine temp dir (for later re-adopt
/// reasoning; unused by this issue).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RegisterRequest {
    pub worker_id: String,
    pub protocol_version: u32,
    pub capacity: u32,
    pub engine_temp_root: String,
}

/// Controller's answer to a registration. The controller is authoritative for
/// the heartbeat cadence the worker must use.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RegisterResponse {
    pub accepted: bool,
    pub heartbeat_interval_secs: u64,
    pub controller_protocol_version: u32,
}

/// Periodic worker liveness + state report (worker -> controller).
/// `running_sessions` is the set of session ids the worker currently runs
/// (empty until later issues populate it). `lifecycle_state` is required by the
/// elasticity flow (#140).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Heartbeat {
    pub worker_id: String,
    pub protocol_version: u32,
    pub lifecycle_state: LifecycleState,
    pub running_sessions: Vec<String>,
    pub timestamp_unix_ms: i64,
}

/// Controller's heartbeat answer. The controller piggybacks control messages on
/// the heartbeat response (pull model: the worker asks, the controller answers).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HeartbeatResponse {
    pub acknowledged: bool,
    pub control: Vec<ControlMessage>,
}

/// Controller -> worker control message, piggybacked on a heartbeat response.
/// Extensible: future variants add here. Tagged by `type` (snake_case).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ControlMessage {
    /// Stop a running session. The worker stops the engine and replies with a
    /// [`Released`] so the controller can safely reassign without a double-run.
    StopSession { session_id: String, reason: String },
}

/// Worker -> controller, sent when the worker begins draining. The controller
/// receives and logs it; no reassignment logic lands until #140.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Draining {
    pub worker_id: String,
    pub sessions: Vec<String>,
    pub checkpoint_done: bool,
}

/// Worker -> controller drain/handoff acknowledgement confirming an engine is
/// actually stopped, so the controller can later reassign without a double-run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Released {
    pub worker_id: String,
    pub session_id: String,
}

/// Worker -> controller work-pull request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PullRequest {
    pub worker_id: String,
    pub protocol_version: u32,
    pub available_capacity: u32,
}

/// Controller's answer to a pull. Empty until claim authority lands (#135).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PullResponse {
    pub assignments: Vec<WorkAssignment>,
}

/// A single unit of work the controller assigns to a worker.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkAssignment {
    pub session_id: String,
    pub goal_ref: String,
}

/// Errors at the protocol boundary.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProtocolError {
    /// The peer's `protocol_version` does not match ours.
    #[error("protocol version mismatch: expected {expected}, got {got}")]
    VersionMismatch { expected: u32, got: u32 },
    /// The internal-auth secret was missing or wrong.
    #[error("unauthorized internal request")]
    Unauthorized,
}

/// Reject a request whose inline `protocol_version` does not match ours.
pub fn check_protocol_version(theirs: u32) -> Result<(), ProtocolError> {
    if theirs == PROTOCOL_VERSION {
        Ok(())
    } else {
        Err(ProtocolError::VersionMismatch {
            expected: PROTOCOL_VERSION,
            got: theirs,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_state_defaults_to_active_and_serializes_snake_case() {
        assert_eq!(LifecycleState::default(), LifecycleState::Active);
        assert_eq!(
            serde_json::to_string(&LifecycleState::Draining).unwrap(),
            "\"draining\""
        );
    }

    #[test]
    fn check_protocol_version_accepts_current_and_rejects_other() {
        assert!(check_protocol_version(PROTOCOL_VERSION).is_ok());
        assert_eq!(
            check_protocol_version(999),
            Err(ProtocolError::VersionMismatch {
                expected: PROTOCOL_VERSION,
                got: 999,
            })
        );
    }

    #[test]
    fn control_message_is_tag_typed_snake_case() {
        let msg = ControlMessage::StopSession {
            session_id: "s1".into(),
            reason: "drain".into(),
        };
        let json = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["type"], "stop_session");
        assert_eq!(json["session_id"], "s1");
    }

    #[test]
    fn register_request_rejects_unknown_fields() {
        let json = r#"{"worker_id":"w1","protocol_version":1,"capacity":4,"engine_temp_root":"/tmp/e","extra":true}"#;
        assert!(serde_json::from_str::<RegisterRequest>(json).is_err());
    }

    #[test]
    fn every_protocol_type_round_trips_through_serde() {
        macro_rules! round_trip {
            ($val:expr) => {{
                let v = $val;
                let s = serde_json::to_string(&v).unwrap();
                let back = serde_json::from_str(&s).unwrap();
                assert_eq!(v, back);
            }};
        }
        round_trip!(RegisterRequest {
            worker_id: "w1".into(),
            protocol_version: PROTOCOL_VERSION,
            capacity: 4,
            engine_temp_root: "/tmp/e".into(),
        });
        round_trip!(RegisterResponse {
            accepted: true,
            heartbeat_interval_secs: 10,
            controller_protocol_version: PROTOCOL_VERSION,
        });
        round_trip!(Heartbeat {
            worker_id: "w1".into(),
            protocol_version: PROTOCOL_VERSION,
            lifecycle_state: LifecycleState::Active,
            running_sessions: vec!["s1".into()],
            timestamp_unix_ms: 1_700_000_000_000,
        });
        round_trip!(HeartbeatResponse {
            acknowledged: true,
            control: vec![ControlMessage::StopSession {
                session_id: "s1".into(),
                reason: "drain".into(),
            }],
        });
        round_trip!(Draining {
            worker_id: "w1".into(),
            sessions: vec!["s1".into()],
            checkpoint_done: false,
        });
        round_trip!(Released {
            worker_id: "w1".into(),
            session_id: "s1".into(),
        });
        round_trip!(PullRequest {
            worker_id: "w1".into(),
            protocol_version: PROTOCOL_VERSION,
            available_capacity: 2,
        });
        round_trip!(PullResponse {
            assignments: vec![WorkAssignment {
                session_id: "s1".into(),
                goal_ref: "owner/repo#1".into(),
            }],
        });
    }
}
