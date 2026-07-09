//! Wire DTOs for the OpenSandbox sandbox-lifecycle API, plus [`OsbError`].
//!
//! Field names + shapes are pinned to the authoritative upstream spec — see the
//! module-root doc ([`super`]) for the exact revision. The response types are
//! deliberately forward-compatible (NO `deny_unknown_fields`): fields the server
//! adds in a later version are ignored rather than breaking deserialization.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Request body for `POST /v1/sandboxes`.
///
/// `timeout` is a PLAIN `Option` with no `skip_serializing_if`: `None` serializes
/// as a literal JSON `null`, which the API reads as "no auto-expiry / manual
/// cleanup" — semantically distinct from omitting the field. `env` / `metadata` /
/// `extensions` always serialize (an empty map -> `{}`), matching the plan's
/// explicit wire shape.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CreateSandboxRequest {
    pub image: ImageSpec,
    pub entrypoint: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub resource_limits: ResourceLimits,
    pub timeout: Option<i64>,
    pub metadata: BTreeMap<String, String>,
    pub extensions: BTreeMap<String, String>,
}

/// Container image spec: a registry `uri` and optional pull `auth`.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ImageSpec {
    pub uri: String,
    /// Omitted from the wire when absent (public image); present carries the
    /// registry credentials for a private image.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<RegistryAuth>,
}

/// Private-registry pull credentials. The spec's image `auth` object is exactly
/// `{username, password}` (a token is passed as the password).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RegistryAuth {
    pub username: String,
    pub password: String,
}

/// Runtime resource constraints. The wire type is a free-form string map
/// (`{"cpu":"500m","memory":"512Mi","gpu":"1"}`), so this is a transparent newtype
/// over `BTreeMap<String,String>` rather than a fixed cpu/memory struct — a fixed
/// struct would silently drop any other resource key (e.g. `gpu`) the server
/// accepts. Callers insert whichever keys they need; it serializes to a plain
/// object.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(transparent)]
pub struct ResourceLimits(pub BTreeMap<String, String>);

/// A sandbox projected into the flat view the client exposes.
///
/// The wire nests lifecycle facts under `status.{state,reason,message}`; this type
/// lifts them to the top level via [`SandboxWire`] (see the `#[serde(from)]`),
/// keeping an ergonomic flat shape for callers. `extensions` is retained for
/// forward-compatibility — current server responses omit it, so it deserializes to
/// an empty map.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(from = "SandboxWire")]
pub struct SandboxView {
    pub id: String,
    pub state: SandboxState,
    pub reason: Option<String>,
    pub message: Option<String>,
    pub metadata: BTreeMap<String, String>,
    pub extensions: BTreeMap<String, String>,
}

/// The nested wire shape actually returned by create / get / list / patch. Private:
/// it exists only to feed [`SandboxView`]'s `#[serde(from)]`. Extra response fields
/// (image, platform, createdAt, entrypoint, …) are ignored (no `deny_unknown_fields`).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SandboxWire {
    id: String,
    status: SandboxStatusWire,
    #[serde(default)]
    metadata: BTreeMap<String, String>,
    #[serde(default)]
    extensions: BTreeMap<String, String>,
}

/// The `status` sub-object of the sandbox wire. `state` is required; `reason` /
/// `message` (and the ignored `lastTransitionAt`) are optional.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SandboxStatusWire {
    state: SandboxState,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    message: Option<String>,
}

impl From<SandboxWire> for SandboxView {
    fn from(wire: SandboxWire) -> Self {
        SandboxView {
            id: wire.id,
            state: wire.status.state,
            reason: wire.status.reason,
            message: wire.status.message,
            metadata: wire.metadata,
            extensions: wire.extensions,
        }
    }
}

/// High-level lifecycle state of a sandbox.
///
/// The documented values are matched exactly (they appear PascalCase on the wire);
/// the spec warns new values may be added, so any unrecognized string deserializes
/// to [`SandboxState::Unknown`] — the client never breaks on a future state.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(from = "String")]
pub enum SandboxState {
    Pending,
    Running,
    Pausing,
    Paused,
    Resuming,
    Stopping,
    Terminated,
    Failed,
    Unknown(String),
}

impl From<String> for SandboxState {
    fn from(value: String) -> Self {
        match value.as_str() {
            "Pending" => SandboxState::Pending,
            "Running" => SandboxState::Running,
            "Pausing" => SandboxState::Pausing,
            "Paused" => SandboxState::Paused,
            "Resuming" => SandboxState::Resuming,
            "Stopping" => SandboxState::Stopping,
            "Terminated" => SandboxState::Terminated,
            "Failed" => SandboxState::Failed,
            _ => SandboxState::Unknown(value),
        }
    }
}

/// A failure talking to the OpenSandbox lifecycle API.
///
/// [`NotFound`](OsbError::NotFound) is the 404-equivalent surfaced LITERALLY on
/// every verb (including delete) — the benign-ness of a missing sandbox is the
/// caller's concern, not this transport's. Every other non-2xx becomes
/// [`Api`](OsbError::Api) carrying the numeric status + the response body text;
/// reqwest transport failures fold into [`Transport`](OsbError::Transport).
#[derive(Debug, thiserror::Error)]
pub enum OsbError {
    #[error("opensandbox resource not found")]
    NotFound,
    #[error("opensandbox API error {status}: {message}")]
    Api { status: u16, message: String },
    #[error(transparent)]
    Transport(#[from] reqwest::Error),
}

#[cfg(test)]
#[path = "dto_tests.rs"]
mod tests;
