//! Internal controller<->worker protocol (issue #134).
//!
//! Defines the versioned, shared-secret-authenticated message vocabulary both
//! roles speak: registration, heartbeat (with the `lifecycle_state` field), the
//! work-pull request/response, and the four drain-related types
//! ([`Draining`], [`Released`], [`ControlMessage::StopSession`], plus the
//! `lifecycle_state` on [`Heartbeat`]). The controller-side registry/router and
//! the worker-side agent/pull-loop consume these types; this module is the
//! authoritative wire contract every later database-free issue builds on.

pub mod types;

pub use types::{
    check_protocol_version, ControlMessage, Draining, Heartbeat, HeartbeatResponse, LifecycleState,
    ProtocolError, PullRequest, PullResponse, RegisterRequest, RegisterResponse, Released,
    WorkAssignment, INTERNAL_AUTH_HEADER, PROTOCOL_VERSION,
};
