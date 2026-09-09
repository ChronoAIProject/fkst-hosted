//! Durable creator/trigger attribution for a session runtime (epic `SBOX-05`).
//!
//! A Kubernetes Pod or an OpenSandbox sandbox outlives the process that created
//! it, and the control plane keeps no session database — so "who does this
//! running sandbox belong to?" can only be answered from the runtime object
//! itself. This module owns that stamp end to end:
//!
//! ```text
//! SessionRegistration
//!   -> RuntimeIdentityMetadata      [mod]      normalized, credential-free facts
//!        -> stamp_pairs(keys, …)    [keys]     ONE renderer, two key sets
//!             -> Pod annotations / sandbox metadata
//!        <- read(keys, …)           [keys]     the exact inverse
//!             -> ObservedRuntimeIdentity
//!                  -> plan(…)       [merge]    complete | backfill | conflict
//!                       -> IdentityGate        [gate]  bounded retry suppression
//!                            -> RuntimeTelemetry [metrics]
//! ```
//!
//! Three rules shape everything here:
//!
//! - **The first complete stamp is authoritative for that runtime incarnation.**
//!   A later trigger edit, assignee change, or reconcile sweep may FILL a missing
//!   key but may never rewrite a differing one. A disagreement is surfaced as
//!   [`AttributionSource::Conflict`], never resolved by preferring the newest
//!   value — silently rewriting attribution is exactly the failure an audit trail
//!   must not have.
//! - **A missing creator id is a fact, not a gap to fill.** An App-authored
//!   trigger's effective creator comes from its sole assignee, and GitHub's issue
//!   metadata exposes no assignee id. The login is stamped, the id key is simply
//!   absent, and the trigger author's id is NEVER borrowed to stand in for it.
//! - **This is display and correlation data, never authorization evidence.** A
//!   regular caller's access to a runtime is decided by
//!   [`crate::session_access`], which projects GitHub's trigger issues. An
//!   annotation is writable by anyone with namespace access; it may corroborate
//!   attribution, it may never grant it.
//!
//! Nothing here ever carries a token, an issue body, an environment value, a
//! collaborator list, or a log-access entry. Only public GitHub ids and logins.

use crate::k8s::SessionPodSpec;

pub mod gate;
pub mod keys;
pub mod merge;
pub mod metrics;

pub use gate::IdentityGate;
pub use keys::{
    read, stamp_pairs, IdentityField, IdentityKeys, IDENTITY_SCHEMA_VERSION, K8S_IDENTITY_KEYS,
    OSB_IDENTITY_KEYS, SOURCE_BACKFILLED_CURRENT_TRIGGER, SOURCE_LAUNCH_METADATA,
};
pub use merge::{is_settled, plan, IdentityPlan};
pub use metrics::{
    IdentityOperationResult, LifecycleEmitResult, RuntimeTelemetry, RuntimeTelemetrySnapshot,
};

/// Which runtime a session lives in. A closed enum, so it is also the only value
/// ever safe to use as the `backend` metric label (epic `OPS-04`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeBackendKind {
    Kubernetes,
    OpenSandbox,
}

impl RuntimeBackendKind {
    /// Every variant, for exhaustive metric rendering.
    pub const ALL: [RuntimeBackendKind; 2] = [
        RuntimeBackendKind::Kubernetes,
        RuntimeBackendKind::OpenSandbox,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            RuntimeBackendKind::Kubernetes => "kubernetes",
            RuntimeBackendKind::OpenSandbox => "opensandbox",
        }
    }

    /// Parse the closed wire spelling back. `None` for anything else — used by
    /// the public filter layer, which must REJECT an unrecognized value rather
    /// than silently widening a query.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == value)
    }

    /// Dense index for the fixed-size metric counter arrays.
    pub(crate) fn index(self) -> usize {
        match self {
            RuntimeBackendKind::Kubernetes => 0,
            RuntimeBackendKind::OpenSandbox => 1,
        }
    }
}

impl std::fmt::Display for RuntimeBackendKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Which concrete runtime an effect concerns — the discriminator that keeps a
/// session's SECOND runtime from being mistaken for its first.
///
/// A session id is stable across kill/respawn by design (it is derived from the
/// trigger issue), and Kubernetes names its Pod from that session id, so neither
/// distinguishes incarnations on its own. The backend-assigned handle
/// (OpenSandbox's sandbox id) or the runtime's creation instant does. Both are
/// optional because neither is knowable before a runtime exists.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RuntimeIncarnation {
    /// A handle only this incarnation has, when the backend assigns one.
    pub runtime_id: Option<String>,
    /// When this incarnation came into existence.
    pub created_at: Option<k8s_openapi::chrono::DateTime<k8s_openapi::chrono::Utc>>,
}

impl RuntimeIncarnation {
    /// The incarnation identified by a backend-assigned handle alone (the handle
    /// is unique per incarnation, so no timestamp is needed).
    pub fn from_handle(runtime_id: impl Into<String>) -> Self {
        Self {
            runtime_id: Some(runtime_id.into()),
            created_at: None,
        }
    }

    /// The incarnation identified by its creation instant — used where the
    /// handle is derived from the session id and therefore constant.
    pub fn at(created_at: k8s_openapi::chrono::DateTime<k8s_openapi::chrono::Utc>) -> Self {
        Self {
            runtime_id: None,
            created_at: Some(created_at),
        }
    }
}

/// How a runtime's displayed attribution was obtained.
///
/// `BackfilledCurrentTrigger` is deliberately not called "original" or
/// "historical": a legacy runtime's trigger issue may have been edited or
/// re-assigned since launch, so evidence recovered from the trigger *now* is
/// honest only about being current.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttributionSource {
    /// Stamped at launch by a control plane that knew the registration.
    LaunchMetadata,
    /// Filled in later from the currently parsed registration.
    BackfilledCurrentTrigger,
    /// Some identity keys present, others missing, with no current evidence.
    PartialMetadata,
    /// No identity stamp at all and no matching registration to recover one.
    UnknownLegacy,
    /// A stamped value disagrees with the current registration. Never silently
    /// replaced; visible to a global admin and to an authorized session viewer.
    Conflict,
}

impl AttributionSource {
    /// Every variant, so a downstream filter or renderer can enumerate the closed
    /// set without restating it.
    pub const ALL: [AttributionSource; 5] = [
        AttributionSource::LaunchMetadata,
        AttributionSource::BackfilledCurrentTrigger,
        AttributionSource::PartialMetadata,
        AttributionSource::UnknownLegacy,
        AttributionSource::Conflict,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            AttributionSource::LaunchMetadata => "launch_metadata",
            AttributionSource::BackfilledCurrentTrigger => "backfilled_current_trigger",
            AttributionSource::PartialMetadata => "partial_metadata",
            AttributionSource::UnknownLegacy => "unknown_legacy",
            AttributionSource::Conflict => "conflict",
        }
    }

    /// Parse the closed wire spelling back. `None` for anything else — used by
    /// the public filter layer, which must REJECT an unrecognized value rather
    /// than silently widening a query.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|source| source.as_str() == value)
    }
}

impl std::fmt::Display for AttributionSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What an idempotent identity patch did to one runtime.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeIdentityOutcome {
    /// Every key the registration can supply was already present and agreeing.
    Unchanged,
    /// One or more absent keys were filled; nothing was overwritten.
    Backfilled,
    /// A present value disagrees with the registration; nothing was written.
    Conflict,
    /// The runtime vanished between observation and patch — a benign no-op.
    NotFound,
}

impl RuntimeIdentityOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            RuntimeIdentityOutcome::Unchanged => "unchanged",
            RuntimeIdentityOutcome::Backfilled => "backfilled",
            RuntimeIdentityOutcome::Conflict => "conflict",
            RuntimeIdentityOutcome::NotFound => "not_found",
        }
    }
}

impl std::fmt::Display for RuntimeIdentityOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The attribution facts a session's runtime carries, already normalized.
///
/// Constructed from the reconciler's `SessionRegistration` (through
/// [`SessionPodSpec`]) and never from a runtime's own metadata, so a tampered
/// annotation can never become the desired state it is compared against.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RuntimeIdentityMetadata {
    /// The effective creator's immutable GitHub id. `None` is legitimate and
    /// load-bearing: an assignee-derived creator has no id in issue metadata.
    pub creator_id: Option<i64>,
    /// The effective creator's normalized GitHub login.
    pub creator_login: String,
    /// The trigger issue author's immutable GitHub id.
    pub trigger_author_id: i64,
    /// The trigger issue author's normalized GitHub login. Stays the historical
    /// author even when the effective creator is a different person.
    pub trigger_author_login: String,
}

impl RuntimeIdentityMetadata {
    /// Build from raw registration values, normalizing both logins.
    pub fn new(
        creator_id: Option<i64>,
        creator_login: &str,
        trigger_author_id: i64,
        trigger_author_login: &str,
    ) -> Self {
        Self {
            creator_id,
            creator_login: normalize_identity_login(creator_login),
            trigger_author_id,
            trigger_author_login: normalize_identity_login(trigger_author_login),
        }
    }

    /// The identity a launch spec carries.
    pub fn from_spec(spec: &SessionPodSpec) -> Self {
        Self::new(
            spec.creator_id,
            &spec.creator_login,
            spec.trigger_author_id,
            &spec.trigger_author_login,
        )
    }
}

/// Normalize a GitHub login to the one form both runtimes stamp.
///
/// Two reasons this is not the raw login:
///
/// 1. **OpenSandbox metadata values must be valid Kubernetes label values**, and
///    an App author's login is rendered `slug[bot]` (or `app/slug`) by GitHub's
///    various surfaces — brackets and slashes that the label-value contract
///    rejects, which would make every App-seeded session fail to create.
/// 2. **The two backends must agree byte-for-byte.** Normalizing once, here,
///    is what makes a Kubernetes annotation and an OpenSandbox metadata value
///    the same string for the same session.
///
/// This mirrors [`crate::reconcile::creator`]'s login normalization, which is
/// already the repository's canonical identity-comparison form; the immutable
/// numeric id remains the authoritative identifier either way.
///
/// **Accepted consequence:** an App-authored trigger stamps `fkst-cloud`, not
/// `fkst-cloud[bot]`, so the stamped LOGIN of a bot-seeded session no longer
/// reads as visibly bot-authored. That is a deliberate trade: the alternative is
/// an OpenSandbox create that the server rejects outright for every seeded
/// session. The distinction is not lost — `trigger_author_id` is the App's
/// immutable numeric id, which is what any caller must compare on anyway, and
/// the trigger issue itself remains the authority on who wrote it.
pub fn normalize_identity_login(login: &str) -> String {
    crate::reconcile::creator::normalize_login(login.trim())
}

/// One runtime's identity stamp as READ BACK from its metadata.
///
/// Every field is optional because a legacy runtime predates the stamp entirely,
/// and because `creator_id` is legitimately absent for an assignee-derived
/// creator. `conflicting` comes from the runtime's DURABLE conflict marker, so a
/// reader holding only the runtime still learns of a disagreement an earlier
/// pass detected; a caller that has just compared the stamp against a current
/// registration may additionally set it. The read itself never decides authority.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ObservedRuntimeIdentity {
    /// The stamped schema version, verbatim. Its PRESENCE is what distinguishes
    /// "written by a control plane that knew the contract" from "legacy".
    pub schema_version: Option<String>,
    pub creator_id: Option<i64>,
    pub creator_login: Option<String>,
    pub trigger_author_id: Option<i64>,
    pub trigger_author_login: Option<String>,
    /// The stamped provenance marker, verbatim
    /// ([`SOURCE_LAUNCH_METADATA`] / [`SOURCE_BACKFILLED_CURRENT_TRIGGER`]).
    /// Absent on a runtime stamped before the marker existed, and on a legacy
    /// runtime with no stamp at all.
    pub source: Option<String>,
    /// A stamped value was observed to disagree with the trigger — read from the
    /// runtime's durable conflict marker ([`IdentityKeys::conflict`]), so it
    /// survives the process that detected it.
    pub conflicting: bool,
    /// A stamped id key held a value that is not a decimal integer. Kept
    /// distinct from "absent" so a corrupted stamp is never mistaken for the
    /// legitimate assignee-derived missing id.
    pub malformed: bool,
}

impl ObservedRuntimeIdentity {
    /// Whether anything at all was stamped.
    pub fn is_empty(&self) -> bool {
        self.schema_version.is_none()
            && self.creator_id.is_none()
            && self.creator_login.is_none()
            && self.trigger_author_id.is_none()
            && self.trigger_author_login.is_none()
    }

    /// How this runtime's attribution should be labelled for display.
    ///
    /// The stamped provenance marker is what separates a launch stamp from a
    /// later backfill — the two write identical attribution keys, so without the
    /// marker a backfilled runtime would claim launch provenance forever, and
    /// the reconciler's in-memory knowledge that it backfilled does not survive
    /// a restart.
    pub fn attribution_source(&self) -> AttributionSource {
        if self.conflicting {
            return AttributionSource::Conflict;
        }
        if self.is_empty() {
            return AttributionSource::UnknownLegacy;
        }
        let stamped_by_contract = self.schema_version.is_some()
            && self.creator_login.is_some()
            && self.trigger_author_id.is_some()
            && self.trigger_author_login.is_some()
            && !self.malformed;
        if !stamped_by_contract {
            return AttributionSource::PartialMetadata;
        }
        match self.source.as_deref() {
            Some(SOURCE_BACKFILLED_CURRENT_TRIGGER) => AttributionSource::BackfilledCurrentTrigger,
            // An absent marker means the stamp predates it, which only a launch
            // writer could have produced: the backfill path has always written
            // one.
            _ => AttributionSource::LaunchMetadata,
        }
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
