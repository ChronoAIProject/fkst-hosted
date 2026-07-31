//! Backend-neutral live runtime inventory (issue #5674, epic `SBOX-01`..`SBOX-05`).
//!
//! [`crate::session_backend::SessionBackend::list_runtime_inventory`] answers one
//! question for the operations surface: *which FKST-managed runtimes exist right
//! now, who do they belong to, and how long may they live?* This module owns the
//! ANSWER's shape — deliberately outside `k8s/` and `opensandbox/`, so no
//! `kube::Pod` and no OpenSandbox wire DTO can leak into it.
//!
//! ```text
//! Pod LIST  ──┐                              ┌── RuntimeInventoryStatus [status]
//! sandbox     ├─> RawRuntimeFacts  ─ build ─>┤   bounded reason/message  [text]
//! page walk ──┘        [build]               ├── derived timings         [timing]
//!                                            └── BoundedInventoryWarning [warning]
//!                                                        │
//!                                             RuntimeInventorySnapshot
//! ```
//!
//! Four rules shape everything here:
//!
//! - **One list, one clock.** Each snapshot performs exactly one logical backend
//!   list and stamps ONE [`RuntimeInventorySnapshot::observed_at`]; every derived
//!   duration is computed against that instant with checked arithmetic, so two
//!   fields of the same item can never disagree about "now".
//! - **Missing data is represented, never invented.** A managed runtime with a
//!   missing label, an unparseable annotation, or no creation timestamp is
//!   returned with `None` fields, an explicit [`RuntimeMetadataState`], and a
//!   bounded warning. Substituting `now` for an absent creation time would make an
//!   ancient orphan look freshly launched, which is precisely the drift an
//!   operations view exists to surface.
//! - **Attribution is display data, never authorization.** The creator/trigger
//!   fields come from runtime metadata, which anyone with namespace access can
//!   write. Row authorization belongs to #5675 and [`crate::session_access`]; this
//!   method deliberately accepts no viewer, actor, access list, or selector.
//! - **Nothing operational is raw.** Reason/message text is byte-bounded, control-
//!   normalized, URI-stripped, and pushed through the central secret redactor
//!   ([`text`]). Container env, image-pull credentials, command output, and
//!   serialized Pod/Sandbox JSON never enter an inventory item.

use k8s_openapi::chrono::{DateTime, Utc};

use crate::reconcile_config::ReconcileConfig;
use crate::runtime_identity::{AttributionSource, RuntimeBackendKind};

pub mod build;
pub mod status;
pub mod text;
pub mod timing;
pub mod warning;

pub use build::{build_item, RawRuntimeFacts};
pub use status::RuntimeInventoryStatus;
pub use text::{
    bounded_operational_text, MAX_RAW_STATUS_BYTES, MAX_STATUS_MESSAGE_BYTES,
    MAX_STATUS_REASON_BYTES,
};
pub use timing::RuntimeTiming;
pub use warning::{
    BoundedInventoryWarning, InventoryWarningCode, WarningSink, DEFAULT_MAX_WARNINGS,
};

/// The lifetime/idle policy an inventory read renders each runtime against, plus
/// the defensive ceiling on how much a single snapshot may return.
///
/// This is a VALUE, not a config handle: the backends stay decoupled from
/// [`ReconcileConfig`] (and therefore testable with hand-built policies), and the
/// caller is forced to state which policy a snapshot was rendered under. The
/// reconciler's own knobs remain authoritative — inventory only DISPLAYS them and
/// never enforces a lifetime.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimeLifetimePolicy {
    /// `FKST_POD_SESSION_MAX_LIFETIME_SECS`. **Zero means unlimited**, which is a
    /// load-bearing distinction: an unlimited session must report a null maximum /
    /// expiry / remaining, never "0 seconds remaining".
    pub max_lifetime_seconds: u64,
    /// `FKST_POD_MIN_LIFETIME_SECS` — the idle-kill shield a fresh runtime enjoys.
    pub minimum_lifetime_seconds: u64,
    /// `FKST_SESSION_IDLE_GRACE_SECS` — how long a non-pending runtime may sit
    /// before the reconciler idle-kills it.
    pub idle_grace_seconds: u64,
    /// Defensive ceiling on the items ONE snapshot may carry
    /// (`FKST_SANDBOX_INVENTORY_MAX_SOURCE_ITEMS`). Exceeding it is an explicit
    /// [`crate::session_backend::BackendError::InventoryTooLarge`], never a
    /// silently shortened list that would read as a complete fleet.
    pub max_items: usize,
    /// Defensive ceiling on the warnings ONE snapshot may carry
    /// (`FKST_SANDBOX_INVENTORY_MAX_WARNINGS`). Configurable alongside
    /// [`Self::max_items`] so a deployment that raised the item ceiling can still
    /// see every affected runtime during a fleet-wide metadata regression.
    /// Overflow is announced with
    /// [`InventoryWarningCode::WarningsTruncated`], never silently dropped.
    pub max_warnings: usize,
}

impl RuntimeLifetimePolicy {
    /// The policy the deployment is actually running under.
    pub fn from_reconcile_config(config: &ReconcileConfig) -> Self {
        Self {
            max_lifetime_seconds: config.pod_session_max_lifetime_secs,
            minimum_lifetime_seconds: config.pod_min_lifetime_secs,
            idle_grace_seconds: config.session_idle_grace_secs,
            max_items: config.sandbox_inventory_max_source_items,
            max_warnings: config.sandbox_inventory_max_warnings,
        }
    }
}

/// How trustworthy one runtime's FKST metadata is.
///
/// Kept apart from [`AttributionSource`] on purpose: attribution answers "who does
/// this belong to", metadata state answers "did the correlation stamp survive".
/// A runtime can have complete attribution and a malformed installation id.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeMetadataState {
    /// Every correlation + attribution fact the contract stamps is present and
    /// parsed.
    Complete,
    /// Something the contract stamps is absent (a legacy runtime, or one stamped
    /// before a key existed). Nothing present failed to parse.
    Partial,
    /// A value IS present but does not parse (a non-integer id, an unparseable
    /// timestamp). Never collapsed into `Partial` — a corrupted stamp and an
    /// absent one call for different operator responses.
    Malformed,
}

impl RuntimeMetadataState {
    pub fn as_str(self) -> &'static str {
        match self {
            RuntimeMetadataState::Complete => "complete",
            RuntimeMetadataState::Partial => "partial",
            RuntimeMetadataState::Malformed => "malformed",
        }
    }
}

impl std::fmt::Display for RuntimeMetadataState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One complete inventory read: every FKST-managed runtime the configured backend
/// reports, observed at one instant.
///
/// This is the COMPLETE fleet — it deliberately carries no viewer filtering. It
/// exists only inside the trusted service process; #5675 authorizes each row
/// before anything is counted, sorted, or serialized.
#[derive(Clone, Debug, PartialEq)]
pub struct RuntimeInventorySnapshot {
    /// The single instant every derived duration in every item is measured from.
    pub observed_at: DateTime<Utc>,
    /// Which runtime backend produced this snapshot.
    pub backend: RuntimeBackendKind,
    pub items: Vec<RuntimeInventoryItem>,
    /// Bounded, closed-code notes about data that was missing, malformed, or
    /// clock-skewed. Never free text, never a backend error message.
    pub warnings: Vec<BoundedInventoryWarning>,
}

/// One FKST-managed runtime, projected into backend-neutral facts.
///
/// Every optional field is optional because the backend genuinely may not know it
/// — not because the projection gave up. See [`build_item`] for the exact rules.
#[derive(Clone, Debug, PartialEq)]
pub struct RuntimeInventoryItem {
    pub backend: RuntimeBackendKind,
    /// The backend's addressable handle: a Pod name, or a sandbox id.
    pub runtime_id: String,
    /// The human-facing runtime name where the backend has one distinct from its
    /// id (Kubernetes does; OpenSandbox does not).
    pub runtime_name: Option<String>,
    /// The backend's own unique object identifier, when it assigns one.
    pub runtime_uid: Option<String>,
    /// The Kubernetes namespace, or the bounded OpenSandbox server label. NEVER a
    /// credential-bearing URL — userinfo, query, and path are all stripped.
    pub backend_location: Option<String>,

    /// The FKST session this runtime belongs to. `None` for an orphan whose
    /// correlation stamp is gone; such a row is global-admin-only downstream.
    pub session_id: Option<String>,
    /// Whether the runtime carries the FKST managed marker. Normally true (the
    /// list selectors already filter to FKST objects); retained so a drifted or
    /// malformed marker stays visible instead of silently passing.
    pub managed: bool,
    pub metadata_state: RuntimeMetadataState,

    pub creator_id: Option<i64>,
    pub creator_login: Option<String>,
    pub trigger_author_id: Option<i64>,
    pub trigger_author_login: Option<String>,
    /// How the attribution above was obtained (#5673). Reused verbatim so a
    /// runtime's provenance means the same thing in the reconciler, the audit
    /// trail, and the operations view.
    pub attribution_source: AttributionSource,

    /// `owner/name`, when both halves are stamped.
    pub repo_full_name: Option<String>,
    pub installation_id: Option<i64>,
    /// The trigger issue number. A stamped `0` is the "unknown" sentinel the rest
    /// of the reconciler uses and is reported as `None`.
    pub trigger_issue: Option<i64>,

    /// The stable normalized state, comparable across backends.
    pub status: RuntimeInventoryStatus,
    /// The backend-native state string, preserved independently and bounded. Empty
    /// when the backend reported no state at all.
    pub raw_status: String,
    /// A bounded, redacted operational reason (a Kubernetes waiting/terminated
    /// reason, an OpenSandbox `status.reason`).
    pub status_reason: Option<String>,
    /// A bounded, redacted operational message. Never log output, never a
    /// serialized backend object.
    pub status_message: Option<String>,

    pub created_at: Option<DateTime<Utc>>,
    pub age_seconds: Option<u64>,
    /// The configured maximum lifetime, or `None` when the deployment configured
    /// unlimited (`FKST_POD_SESSION_MAX_LIFETIME_SECS=0`).
    pub max_lifetime_seconds: Option<u64>,
    pub expires_at: Option<DateTime<Utc>>,
    pub remaining_seconds: Option<u64>,
    pub minimum_lifetime_seconds: u64,
    pub minimum_lifetime_remaining_seconds: Option<u64>,
    pub idle_grace_seconds: u64,
    pub last_pending_at: Option<DateTime<Utc>>,
    /// How long the runtime has been idle, measured exactly as
    /// [`crate::reconcile::desired`] measures it: from `last_pending_at`, falling
    /// back to `created_at`.
    pub idle_for_seconds: Option<u64>,

    /// Summed container restarts where the backend reports them. `None` — never
    /// zero — when the backend has no such concept, so "never restarted" and "not
    /// knowable" stay distinguishable.
    pub restart_count: Option<u32>,
    pub last_transition_at: Option<DateTime<Utc>>,
    pub deletion_timestamp: Option<DateTime<Utc>>,
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "contract_tests.rs"]
mod contract_tests;
