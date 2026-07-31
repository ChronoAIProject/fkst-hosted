//! The second audit contract: system sandbox lifecycle transitions.
//!
//! [`super::event::ApiRequestCompletedV1`] answers "who called what". This
//! answers "what did the control plane then DO to a runtime", which no HTTP
//! record can: a session pod is created by a level-triggered reconciler minutes
//! after — or with no relation at all to — the request that opened its trigger
//! issue.
//!
//! ## Transitions, not a polling log
//!
//! Only EFFECTS are recorded, at the effect boundary:
//!
//! - `*_requested` immediately before the backend call;
//! - `created` only once the backend confirmed a runtime;
//! - `deleted` only once absence is confirmed (including the idempotent
//!   already-gone case);
//! - `*_failed` on a bounded backend failure, with a closed reason code and
//!   never a raw error message;
//! - the two identity actions only when a real backfill/conflict DECISION was
//!   taken.
//!
//! There is deliberately no status event per inventory poll: live state comes
//! from the runtime backend, and a per-sweep row would drown the transitions it
//! is supposed to explain while costing one PostHog event per session per sweep.
//!
//! ## Deterministic ids, and the honest limit of incarnations
//!
//! Delivery is at-least-once, so the event id must be derived, not random:
//! PostHog deduplicates on the UUID, which is what stops a reconcile retry from
//! writing a second row for one transition (epic `AUD-07`).
//!
//! The key includes an INCARNATION discriminator so a session that is killed and
//! respawned does not collapse into one row. When a concrete runtime handle
//! exists it IS the incarnation, which is exact. Before one exists — a
//! `create_requested`, or a create that failed — there is nothing runtime-shaped
//! to key on, so the caller supplies the session's runtime config hash instead:
//! retries of one spawn dedupe, and a spawn of a changed configuration does not.
//! Two spawns of the SAME configuration therefore share a `create_requested`
//! row; the `created` rows that follow remain distinct, and those are what a
//! timeline reads. This is the strongest determinism available before a runtime
//! exists, and it is stated here rather than hidden.
//!
//! ## What may never appear on this event
//!
//! No token, no issue body or title, no environment value, no install command,
//! no collaborator or log-access list, no upstream error text. Attribution is
//! ids plus normalized login snapshots; failures are closed reason codes.

use k8s_openapi::chrono::{DateTime, SecondsFormat, Utc};
use serde_json::{json, Map, Value};
use uuid::Uuid;

use super::event::{Actor, Principal, ServiceIdentity};
use super::identity::AuditIdentity;
use super::lifecycle_validate::validate_lifecycle;
use super::projection::{CaptureEvent, EventLimits};
use super::validate::EventError;
use crate::runtime_identity::RuntimeBackendKind;

/// Schema version stamped on every lifecycle record.
pub const LIFECYCLE_SCHEMA_VERSION: u32 = 1;

/// The PostHog event name. Stable: HogQL, dashboards, and the scoped activity
/// query all key on it, and it is what separates lifecycle rows from request
/// rows in a merged timeline.
pub const LIFECYCLE_EVENT_NAME: &str = "fkst sandbox lifecycle";

/// Namespace for the deterministic UUIDv5 lifecycle event id. A fixed random
/// constant, distinct from the request-event namespace so the two contracts can
/// never collide on the same key material.
const LIFECYCLE_EVENT_ID_NAMESPACE: Uuid =
    Uuid::from_u128(0x6c31_28ad_4e7f_4f91_b0d2_5a44_9c17_63be);

/// The incarnation discriminator used before any runtime handle exists.
const NO_RUNTIME_INCARNATION: &str = "pending";

/// One recorded runtime transition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleAction {
    CreateRequested,
    Created,
    CreateFailed,
    DeleteRequested,
    Deleted,
    DeleteFailed,
    IdentityBackfilled,
    IdentityConflict,
}

impl LifecycleAction {
    pub const ALL: [LifecycleAction; 8] = [
        LifecycleAction::CreateRequested,
        LifecycleAction::Created,
        LifecycleAction::CreateFailed,
        LifecycleAction::DeleteRequested,
        LifecycleAction::Deleted,
        LifecycleAction::DeleteFailed,
        LifecycleAction::IdentityBackfilled,
        LifecycleAction::IdentityConflict,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            LifecycleAction::CreateRequested => "create_requested",
            LifecycleAction::Created => "created",
            LifecycleAction::CreateFailed => "create_failed",
            LifecycleAction::DeleteRequested => "delete_requested",
            LifecycleAction::Deleted => "deleted",
            LifecycleAction::DeleteFailed => "delete_failed",
            LifecycleAction::IdentityBackfilled => "identity_backfilled",
            LifecycleAction::IdentityConflict => "identity_conflict",
        }
    }

    /// Dense index for the fixed-size metric counter arrays.
    pub(crate) fn index(self) -> usize {
        match self {
            LifecycleAction::CreateRequested => 0,
            LifecycleAction::Created => 1,
            LifecycleAction::CreateFailed => 2,
            LifecycleAction::DeleteRequested => 3,
            LifecycleAction::Deleted => 4,
            LifecycleAction::DeleteFailed => 5,
            LifecycleAction::IdentityBackfilled => 6,
            LifecycleAction::IdentityConflict => 7,
        }
    }
}

impl std::fmt::Display for LifecycleAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why a transition happened or failed. A CLOSED enum on purpose: an upstream
/// error message may quote a URL, a header, or a rejected value, none of which
/// belong in an analytics store.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleReason {
    /// The session sat non-pending past its idle grace.
    Idle,
    /// The runtime's recorded config no longer matches its registration.
    ConfigChanged,
    /// The trigger issue closed, so the session is no longer desired.
    TriggerClosed,
    /// A terminal runtime was garbage-collected.
    TerminalCleanup,
    /// The deterministically identified runtime already existed.
    AlreadyLive,
    /// The runtime was already absent when the effect ran.
    RuntimeNotFound,
    /// The backend could not be reached or refused the call.
    BackendUnavailable,
    /// A value the runtime's metadata contract rejects.
    InvalidMetadata,
    /// A stamped attribution value disagrees with the current registration.
    AttributionConflict,
}

impl LifecycleReason {
    pub fn as_str(self) -> &'static str {
        match self {
            LifecycleReason::Idle => "idle",
            LifecycleReason::ConfigChanged => "config_changed",
            LifecycleReason::TriggerClosed => "trigger_closed",
            LifecycleReason::TerminalCleanup => "terminal_cleanup",
            LifecycleReason::AlreadyLive => "already_live",
            LifecycleReason::RuntimeNotFound => "runtime_not_found",
            LifecycleReason::BackendUnavailable => "backend_unavailable",
            LifecycleReason::InvalidMetadata => "invalid_metadata",
            LifecycleReason::AttributionConflict => "attribution_conflict",
        }
    }
}

impl std::fmt::Display for LifecycleReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The attribution carried for display and correlation. Never an authorization
/// input: the read side authorizes a session id against
/// [`crate::session_access`], never against these fields.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LifecycleAttribution {
    pub creator_id: Option<i64>,
    pub creator_login: Option<String>,
    pub trigger_author_id: Option<i64>,
    pub trigger_author_login: Option<String>,
}

/// Where the affected session lives, plus the audited request that caused the
/// effect when one did (epic `AUD-05`).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LifecycleCorrelation {
    /// `owner/name`.
    pub repo_full_name: Option<String>,
    pub installation_id: Option<i64>,
    pub trigger_issue: Option<i64>,
    /// The `X-Request-Id` of the API/webhook call that directly caused this
    /// effect, when one did. Autonomous reconcile effects carry `None` rather
    /// than a fabricated id.
    pub request_id: Option<String>,
}

/// The runtime an effect concerns, to the extent it is known.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LifecycleRuntime {
    /// The backend's own identifier (pod name, sandbox id). `None` before a
    /// runtime exists.
    pub runtime_id: Option<String>,
    /// When the runtime was created, when the backend reports it.
    pub created_at: Option<DateTime<Utc>>,
    /// Discriminator used when no `runtime_id` exists yet — the session's
    /// runtime config hash. See the module docs for why.
    pub incarnation_hint: Option<String>,
}

/// One sandbox lifecycle transition, schema version 1.
#[derive(Clone, Debug, PartialEq)]
pub struct SandboxLifecycleV1 {
    pub schema_version: u32,
    /// Deterministic UUIDv5 over `(schema, action, backend, session, incarnation)`.
    pub event_id: Uuid,
    pub occurred_at: DateTime<Utc>,
    pub action: LifecycleAction,
    /// The initiating identity. Autonomous effects use the `system` actor.
    pub actor: Actor,
    /// The executing identity: the App installation, or the reconciler itself.
    pub principal: Principal,
    /// The canonical session id. Validated before delivery; a lifecycle row
    /// without a trustworthy session id can never be scoped to a regular user.
    pub session_id: String,
    pub backend: RuntimeBackendKind,
    pub runtime: LifecycleRuntime,
    pub attribution: LifecycleAttribution,
    pub correlation: LifecycleCorrelation,
    pub reason_code: Option<LifecycleReason>,
    pub service: ServiceIdentity,
}

impl SandboxLifecycleV1 {
    /// Build a transition record with a deterministic event id.
    pub fn new(
        action: LifecycleAction,
        backend: RuntimeBackendKind,
        session_id: impl Into<String>,
        identity: AuditIdentity,
        service: ServiceIdentity,
    ) -> Self {
        let session_id = session_id.into();
        let runtime = LifecycleRuntime::default();
        Self {
            schema_version: LIFECYCLE_SCHEMA_VERSION,
            event_id: derive_lifecycle_event_id(action, backend, &session_id, &runtime),
            occurred_at: Utc::now(),
            action,
            actor: identity.actor,
            principal: identity.principal,
            session_id,
            backend,
            runtime,
            attribution: LifecycleAttribution::default(),
            correlation: LifecycleCorrelation::default(),
            reason_code: None,
            service,
        }
    }

    /// Attach the runtime handle, re-deriving the event id so the incarnation is
    /// part of it. Always call this before the record is submitted.
    pub fn with_runtime(mut self, runtime: LifecycleRuntime) -> Self {
        self.event_id =
            derive_lifecycle_event_id(self.action, self.backend, &self.session_id, &runtime);
        self.runtime = runtime;
        self
    }

    pub fn with_attribution(mut self, attribution: LifecycleAttribution) -> Self {
        self.attribution = attribution;
        self
    }

    pub fn with_correlation(mut self, correlation: LifecycleCorrelation) -> Self {
        self.correlation = correlation;
        self
    }

    pub fn with_reason(mut self, reason: LifecycleReason) -> Self {
        self.reason_code = Some(reason);
        self
    }

    /// Override the occurrence instant (tests and replayed effects).
    pub fn at(mut self, occurred_at: DateTime<Utc>) -> Self {
        self.occurred_at = occurred_at;
        self
    }

    /// RFC3339 UTC with millisecond precision, the exact form sent to PostHog.
    pub fn occurred_at_rfc3339(&self) -> String {
        self.occurred_at
            .to_rfc3339_opts(SecondsFormat::Millis, true)
    }

    /// Validate and project onto the PostHog capture wire format.
    pub fn to_capture_event(&self, limits: EventLimits) -> Result<CaptureEvent, EventError> {
        validate_lifecycle(self)?;
        let projected = CaptureEvent {
            event: LIFECYCLE_EVENT_NAME,
            // System effects are grouped under the control plane's own distinct
            // id, never under the creator's: attributing a reconciler action to
            // a person's PostHog profile would make an autonomous effect look
            // like something they did.
            distinct_id: super::projection::SYSTEM_DISTINCT_ID.to_string(),
            uuid: self.event_id.to_string(),
            timestamp: self.occurred_at_rfc3339(),
            properties: self.properties(),
        };
        let encoded = serde_json::to_vec(&projected)
            .map_err(|e| EventError::Unserializable(e.to_string()))?;
        if encoded.len() > limits.max_event_bytes {
            return Err(EventError::TooLarge {
                actual: encoded.len(),
                limit: limits.max_event_bytes,
            });
        }
        Ok(projected)
    }

    /// The flattened property bag. Flat scalars because the scoped activity
    /// query filters on `session_id` without digging through nested JSON.
    fn properties(&self) -> Map<String, Value> {
        let mut properties = Map::new();
        let mut put = |key: &str, value: Value| {
            properties.insert(key.to_string(), value);
        };
        put("schema_version", json!(self.schema_version));
        put("event_id", json!(self.event_id.to_string()));
        put("occurred_at", json!(self.occurred_at_rfc3339()));
        put("lifecycle_action", json!(self.action.as_str()));
        put("actor_kind", json!(self.actor.kind.as_str()));
        put("actor_id", json!(self.actor.id));
        put("actor_login", json!(self.actor.login));
        put("principal_kind", json!(self.principal.kind.as_str()));
        put("principal_id", json!(self.principal.id));
        put("session_id", json!(self.session_id));
        put("backend", json!(self.backend.as_str()));
        put("runtime_id", json!(self.runtime.runtime_id));
        put("creator_id", json!(self.attribution.creator_id));
        put("creator_login", json!(self.attribution.creator_login));
        put(
            "trigger_author_id",
            json!(self.attribution.trigger_author_id),
        );
        put(
            "trigger_author_login",
            json!(self.attribution.trigger_author_login),
        );
        put("repo_full_name", json!(self.correlation.repo_full_name));
        put("installation_id", json!(self.correlation.installation_id));
        put("trigger_issue", json!(self.correlation.trigger_issue));
        put("request_id", json!(self.correlation.request_id));
        put(
            "created_at",
            json!(self
                .runtime
                .created_at
                .map(|at| at.to_rfc3339_opts(SecondsFormat::Millis, true))),
        );
        put(
            "reason_code",
            json!(self.reason_code.map(LifecycleReason::as_str)),
        );
        put("service_version", json!(self.service.version));
        put("service_environment", json!(self.service.environment));
        // A system effect must never create a PostHog person profile.
        put("$process_person_profile", json!(false));
        properties
    }
}

/// Derive the deterministic lifecycle event id.
///
/// The incarnation component is the runtime handle when one exists (exact), and
/// otherwise the caller's hint — see the module docs for what that trades away.
pub fn derive_lifecycle_event_id(
    action: LifecycleAction,
    backend: RuntimeBackendKind,
    session_id: &str,
    runtime: &LifecycleRuntime,
) -> Uuid {
    let incarnation = match (&runtime.runtime_id, &runtime.incarnation_hint) {
        (Some(runtime_id), _) => match runtime.created_at {
            Some(created_at) => format!(
                "{runtime_id}@{}",
                created_at.to_rfc3339_opts(SecondsFormat::Millis, true)
            ),
            None => runtime_id.clone(),
        },
        (None, Some(hint)) => hint.clone(),
        (None, None) => NO_RUNTIME_INCARNATION.to_string(),
    };
    let key = format!(
        "{LIFECYCLE_SCHEMA_VERSION}|{}|{}|{session_id}|{incarnation}",
        action.as_str(),
        backend.as_str(),
    );
    Uuid::new_v5(&LIFECYCLE_EVENT_ID_NAMESPACE, key.as_bytes())
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;
