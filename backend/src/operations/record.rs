//! Source-neutral activity records.
//!
//! PostHog and (from issue #5678) the durable relay hold the same two audit
//! contracts in two different shapes. This module is the ONE shape the merge
//! layer, the page assembler, and the HTTP DTOs speak, so neither source's wire
//! format leaks past its adapter — and so a second source can be added without
//! touching anything above [`super::source`].
//!
//! Two invariants make the merge sound:
//!
//! - every record carries a `sort_timestamp` and an `event_id`, which together
//!   are the total order and the deduplication key;
//! - `delivery_state` is ORDERED by severity, so deduplicating two copies of one
//!   event keeps the more alarming state (a relay row that says `queued` must not
//!   be erased by a PostHog row that says `verified`, or a stuck delivery would
//!   look healthy the moment it was also captured).

use k8s_openapi::chrono::{DateTime, Utc};
use serde_json::{Map, Value};

/// Which source produced a record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActivitySourceKind {
    Posthog,
    Relay,
}

impl ActivitySourceKind {
    /// Every variant, for the closed-label metric exposition.
    pub const ALL: [ActivitySourceKind; 2] =
        [ActivitySourceKind::Posthog, ActivitySourceKind::Relay];

    /// The stable wire string; safe as a closed-enum metric label.
    pub fn as_str(self) -> &'static str {
        match self {
            ActivitySourceKind::Posthog => "posthog",
            ActivitySourceKind::Relay => "relay",
        }
    }
}

/// How far through delivery a record has got.
///
/// The ordering is deliberate and is what [`ActivityRecord::merge_delivery`]
/// relies on: a more severe state always wins a deduplication.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum DeliveryState {
    /// Read back from PostHog: the record is genuinely query-visible.
    VerifiedInPosthog,
    /// Capture returned `200`. Accepted is not the same as query-visible.
    AcceptedPendingVerification,
    /// Still in the relay's outbox.
    Queued,
    /// A start record whose completion never arrived.
    Incomplete,
    /// Delivery gave up permanently.
    DeadLetter,
}

impl DeliveryState {
    /// The stable wire string.
    pub fn as_str(self) -> &'static str {
        match self {
            DeliveryState::VerifiedInPosthog => "verified_in_posthog",
            DeliveryState::AcceptedPendingVerification => "accepted_pending_verification",
            DeliveryState::Queued => "queued",
            DeliveryState::Incomplete => "incomplete",
            DeliveryState::DeadLetter => "dead_letter",
        }
    }
}

/// The initiating identity, as recorded. `id` is the immutable GitHub numeric id;
/// `login` is a historical snapshot and is never an authorization input.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RecordActor {
    pub kind: Option<String>,
    pub id: Option<i64>,
    pub login: Option<String>,
}

/// The executing identity, as recorded.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RecordPrincipal {
    pub kind: Option<String>,
    pub id: Option<String>,
}

/// The correlation keys shared by both record kinds (epic `AUD-05`).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RecordCorrelation {
    pub session_id: Option<String>,
    pub repo_full_name: Option<String>,
    pub installation_id: Option<i64>,
    pub trigger_issue: Option<i64>,
    pub request_id: Option<String>,
    /// The GitHub delivery id a webhook-driven record was correlated to. Present
    /// on API-request rows only; `AUD-05` names it as a correlation key, so it
    /// travels all the way to the response rather than stopping at capture.
    pub webhook_delivery_id: Option<String>,
}

/// One recorded API request.
#[derive(Clone, Debug, PartialEq)]
pub struct ApiRequestRecord {
    pub event_id: String,
    pub request_id: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: DateTime<Utc>,
    pub method: String,
    pub route_template: String,
    pub operation_id: String,
    pub actor: RecordActor,
    pub principal: RecordPrincipal,
    /// The operation's own allowlisted safe arguments, verbatim. Nothing outside
    /// each operation's documented allowlist can be in here: the write side
    /// filtered it (see [`crate::audit::arguments`]).
    pub arguments: Map<String, Value>,
    pub arguments_parse_status: Option<String>,
    /// `None` for an incomplete record — no system fabricates a status it never
    /// returned.
    pub status_code: Option<u16>,
    pub outcome: String,
    pub error_code: Option<String>,
    pub duration_ms: Option<u64>,
    pub correlation: RecordCorrelation,
}

/// One recorded sandbox lifecycle transition.
#[derive(Clone, Debug, PartialEq)]
pub struct SandboxLifecycleRecord {
    pub event_id: String,
    pub occurred_at: DateTime<Utc>,
    pub lifecycle_action: String,
    pub actor: RecordActor,
    pub principal: RecordPrincipal,
    pub session_id: String,
    pub backend: Option<String>,
    pub runtime_id: Option<String>,
    pub creator_id: Option<i64>,
    pub creator_login: Option<String>,
    pub trigger_author_id: Option<i64>,
    pub trigger_author_login: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub reason_code: Option<String>,
    pub correlation: RecordCorrelation,
}

/// One row of a merged timeline.
///
/// A tagged union rather than one flat struct: a lifecycle transition has no HTTP
/// method, status, or duration, and inventing null ones would invite every reader
/// to treat "no status" and "status unknown" as the same thing.
#[derive(Clone, Debug, PartialEq)]
pub enum ActivityRecord {
    ApiRequest {
        record: Box<ApiRequestRecord>,
        delivery_state: DeliveryState,
        source: ActivitySourceKind,
    },
    SandboxLifecycle {
        record: Box<SandboxLifecycleRecord>,
        delivery_state: DeliveryState,
        source: ActivitySourceKind,
    },
}

impl ActivityRecord {
    /// The deduplication key.
    pub fn event_id(&self) -> &str {
        match self {
            ActivityRecord::ApiRequest { record, .. } => &record.event_id,
            ActivityRecord::SandboxLifecycle { record, .. } => &record.event_id,
        }
    }

    /// The primary sort key: the terminal/deadline instant of the record.
    pub fn sort_timestamp(&self) -> DateTime<Utc> {
        match self {
            ActivityRecord::ApiRequest { record, .. } => record.completed_at,
            ActivityRecord::SandboxLifecycle { record, .. } => record.occurred_at,
        }
    }

    /// The verified actor id, when the record carries one. `None` covers
    /// anonymous, unattributed, and system rows — none of which a regular caller
    /// may ever see.
    pub fn actor_id(&self) -> Option<i64> {
        match self {
            ActivityRecord::ApiRequest { record, .. } => record.actor.id,
            ActivityRecord::SandboxLifecycle { record, .. } => record.actor.id,
        }
    }

    /// The correlated session id, when the record carries one.
    pub fn session_id(&self) -> Option<&str> {
        match self {
            ActivityRecord::ApiRequest { record, .. } => record.correlation.session_id.as_deref(),
            ActivityRecord::SandboxLifecycle { record, .. } => Some(&record.session_id),
        }
    }

    /// Whether this is a system lifecycle row.
    pub fn is_lifecycle(&self) -> bool {
        matches!(self, ActivityRecord::SandboxLifecycle { .. })
    }

    /// Which source produced it.
    pub fn source(&self) -> ActivitySourceKind {
        match self {
            ActivityRecord::ApiRequest { source, .. } => *source,
            ActivityRecord::SandboxLifecycle { source, .. } => *source,
        }
    }

    /// The delivery state currently attached.
    pub fn delivery_state(&self) -> DeliveryState {
        match self {
            ActivityRecord::ApiRequest { delivery_state, .. } => *delivery_state,
            ActivityRecord::SandboxLifecycle { delivery_state, .. } => *delivery_state,
        }
    }

    /// Raise this record's delivery state to `other` when `other` is more severe.
    ///
    /// Called when two sources report the same event id: PostHog's CONTENT is
    /// preferred (it is the verified projection), but a relay copy saying the
    /// delivery is stuck or dead is the fact an operator needs, so the more severe
    /// state survives the merge.
    pub fn merge_delivery(&mut self, other: DeliveryState) {
        let current = self.delivery_state();
        if other <= current {
            return;
        }
        match self {
            ActivityRecord::ApiRequest { delivery_state, .. } => *delivery_state = other,
            ActivityRecord::SandboxLifecycle { delivery_state, .. } => *delivery_state = other,
        }
    }
}

#[cfg(test)]
#[path = "record_tests.rs"]
mod tests;
