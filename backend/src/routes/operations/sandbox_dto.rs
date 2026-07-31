//! The response contract of `GET /api/v1/operations/sandboxes`.
//!
//! [`SandboxItem`] is the public projection of #5674's
//! [`RuntimeInventoryItem`](crate::session_backend::inventory::RuntimeInventoryItem).
//! Three things about the shape are deliberate.
//!
//! **Every optional field is serialized as an explicit `null`, never omitted.** A
//! backend that cannot know something and a field that happens to be absent are
//! the same wire shape only if `null` is always present; omitting it would make a
//! client's "is this supported here?" question unanswerable. `restart_count` is
//! the clearest case: OpenSandbox exposes no such concept, and `null` says so
//! while `0` would be a lie.
//!
//! **Timestamps are RFC3339 UTC strings with millisecond precision** — the same
//! form the audit contract writes — so a client comparing an inventory timestamp
//! to a recorded one gets an equality rather than a formatting puzzle.
//!
//! **There is no count of anything the caller cannot see.** `item_count` is the
//! length of `items`, and `warning_codes` summarizes those same rows. No field on
//! this page is derived from a runtime the caller is not authorized to see (epic
//! `AUTH-06`).
//!
//! Nothing here can carry a credential: every value originates in #5674's typed,
//! bounded, secret-redacted projection, and `backend_location` is a namespace or
//! a bounded server label with userinfo, query, and path already stripped.

use k8s_openapi::chrono::{DateTime, SecondsFormat, Utc};
use serde::Serialize;
use utoipa::ToSchema;

use crate::operations::sandbox::{AuthorizedInventory, AuthorizedRuntime, SandboxWarningCode};
use crate::runtime_identity::{AttributionSource, RuntimeBackendKind};
use crate::session_backend::inventory::{
    RuntimeInventoryItem, RuntimeInventoryStatus, RuntimeMetadataState,
};

use super::sandbox_query::NormalizedSandboxRequest;

/// Which runtime backend produced a snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SandboxBackend {
    Kubernetes,
    Opensandbox,
}

impl SandboxBackend {
    fn of(kind: RuntimeBackendKind) -> Self {
        match kind {
            RuntimeBackendKind::Kubernetes => SandboxBackend::Kubernetes,
            RuntimeBackendKind::OpenSandbox => SandboxBackend::Opensandbox,
        }
    }
}

/// The effective scope a snapshot was produced under.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SandboxEffectiveScope {
    /// Only sessions this caller created or can observe through an explicit
    /// session tier — evaluated WITHOUT the deployment global-admin bypass.
    Accessible,
    /// Every FKST-managed runtime, including unattributable ones. Global
    /// administrators only.
    All,
}

/// The stable normalized runtime state.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SandboxStatus {
    Pending,
    Running,
    Paused,
    Transitioning,
    Succeeded,
    Failed,
    Terminating,
    Terminated,
    Unknown,
}

impl SandboxStatus {
    fn of(status: RuntimeInventoryStatus) -> Self {
        match status {
            RuntimeInventoryStatus::Pending => SandboxStatus::Pending,
            RuntimeInventoryStatus::Running => SandboxStatus::Running,
            RuntimeInventoryStatus::Paused => SandboxStatus::Paused,
            RuntimeInventoryStatus::Transitioning => SandboxStatus::Transitioning,
            RuntimeInventoryStatus::Succeeded => SandboxStatus::Succeeded,
            RuntimeInventoryStatus::Failed => SandboxStatus::Failed,
            RuntimeInventoryStatus::Terminating => SandboxStatus::Terminating,
            RuntimeInventoryStatus::Terminated => SandboxStatus::Terminated,
            RuntimeInventoryStatus::Unknown => SandboxStatus::Unknown,
        }
    }
}

/// How trustworthy one runtime's FKST metadata stamp is.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SandboxMetadataState {
    Complete,
    Partial,
    Malformed,
}

impl SandboxMetadataState {
    fn of(state: RuntimeMetadataState) -> Self {
        match state {
            RuntimeMetadataState::Complete => SandboxMetadataState::Complete,
            RuntimeMetadataState::Partial => SandboxMetadataState::Partial,
            RuntimeMetadataState::Malformed => SandboxMetadataState::Malformed,
        }
    }
}

/// How a runtime's displayed attribution was obtained. Display and correlation
/// only — never an authorization input.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SandboxAttributionSource {
    LaunchMetadata,
    BackfilledCurrentTrigger,
    PartialMetadata,
    UnknownLegacy,
    Conflict,
}

impl SandboxAttributionSource {
    fn of(source: AttributionSource) -> Self {
        match source {
            AttributionSource::LaunchMetadata => SandboxAttributionSource::LaunchMetadata,
            AttributionSource::BackfilledCurrentTrigger => {
                SandboxAttributionSource::BackfilledCurrentTrigger
            }
            AttributionSource::PartialMetadata => SandboxAttributionSource::PartialMetadata,
            AttributionSource::UnknownLegacy => SandboxAttributionSource::UnknownLegacy,
            AttributionSource::Conflict => SandboxAttributionSource::Conflict,
        }
    }
}

/// A bounded, stable warning code.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SandboxWarning {
    MissingSessionId,
    MalformedCorrelation,
    MalformedIdentity,
    AttributionConflict,
    MissingCreatedAt,
    MalformedCreatedAt,
    MalformedLastPending,
    ClockSkew,
    LifetimeOverflow,
    UnknownStatus,
    WarningsIncomplete,
}

impl SandboxWarning {
    fn of(code: SandboxWarningCode) -> Self {
        match code {
            SandboxWarningCode::MissingSessionId => SandboxWarning::MissingSessionId,
            SandboxWarningCode::MalformedCorrelation => SandboxWarning::MalformedCorrelation,
            SandboxWarningCode::MalformedIdentity => SandboxWarning::MalformedIdentity,
            SandboxWarningCode::AttributionConflict => SandboxWarning::AttributionConflict,
            SandboxWarningCode::MissingCreatedAt => SandboxWarning::MissingCreatedAt,
            SandboxWarningCode::MalformedCreatedAt => SandboxWarning::MalformedCreatedAt,
            SandboxWarningCode::MalformedLastPending => SandboxWarning::MalformedLastPending,
            SandboxWarningCode::ClockSkew => SandboxWarning::ClockSkew,
            SandboxWarningCode::LifetimeOverflow => SandboxWarning::LifetimeOverflow,
            SandboxWarningCode::UnknownStatus => SandboxWarning::UnknownStatus,
            SandboxWarningCode::WarningsIncomplete => SandboxWarning::WarningsIncomplete,
        }
    }
}

/// The caller's own normalized filters, echoed back.
///
/// Echoing the NORMALIZED form (not the raw query) is what lets a client confirm
/// which query actually ran without the server ever repeating a value it refused.
#[derive(Clone, Debug, Default, Serialize, ToSchema)]
pub struct SandboxFiltersView {
    pub status: Option<SandboxStatus>,
    pub backend: Option<SandboxBackend>,
    pub creator_id: Option<i64>,
    pub creator_login: Option<String>,
    pub repo_full_name: Option<String>,
    pub session_id: Option<String>,
    pub trigger_issue: Option<i64>,
    pub attribution_source: Option<SandboxAttributionSource>,
}

/// One live FKST-managed runtime the caller is authorized to see.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct SandboxItem {
    pub backend: SandboxBackend,
    /// The backend's addressable handle: a Pod name, or a sandbox id.
    pub runtime_id: String,
    /// The human-facing name where the backend has one distinct from its id.
    pub runtime_name: Option<String>,
    /// The backend's own unique object identifier, when it assigns one.
    pub runtime_uid: Option<String>,
    /// The Kubernetes namespace, or the bounded OpenSandbox server label.
    pub backend_location: Option<String>,

    /// The FKST session this runtime belongs to. `null` for an orphan, which only
    /// a global administrator ever sees.
    pub session_id: Option<String>,
    /// Whether the runtime carries the FKST managed marker.
    pub managed: bool,
    pub metadata_state: SandboxMetadataState,

    pub creator_id: Option<i64>,
    pub creator_login: Option<String>,
    pub trigger_author_id: Option<i64>,
    pub trigger_author_login: Option<String>,
    pub attribution_source: SandboxAttributionSource,

    pub repo_full_name: Option<String>,
    pub installation_id: Option<i64>,
    pub trigger_issue: Option<i64>,

    pub status: SandboxStatus,
    /// The backend-native state string, preserved verbatim and bounded.
    pub raw_status: String,
    /// A bounded, redacted operational reason.
    pub status_reason: Option<String>,
    /// A bounded, redacted operational message. Never log output.
    pub status_message: Option<String>,

    /// RFC3339 UTC.
    pub created_at: Option<String>,
    pub age_seconds: Option<u64>,
    /// `null` when the deployment configured an unlimited session lifetime.
    pub max_lifetime_seconds: Option<u64>,
    pub expires_at: Option<String>,
    pub remaining_seconds: Option<u64>,
    pub minimum_lifetime_seconds: u64,
    pub minimum_lifetime_remaining_seconds: Option<u64>,
    pub idle_grace_seconds: u64,
    pub last_pending_at: Option<String>,
    pub idle_for_seconds: Option<u64>,

    /// `null` — never zero — when the backend has no restart concept.
    pub restart_count: Option<u32>,
    pub last_transition_at: Option<String>,
    pub deletion_timestamp: Option<String>,

    /// Bounded codes about THIS row's data quality.
    pub warning_codes: Vec<SandboxWarning>,
}

/// One complete authorized live snapshot.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct SandboxInventoryResponse {
    /// The backend snapshot's own instant, verbatim — never `now`, and never a
    /// cached value from an earlier read.
    pub observed_at: String,
    pub backend: SandboxBackend,
    pub effective_scope: SandboxEffectiveScope,
    /// Whether this caller may select the global scope. A server fact the UI uses
    /// to LABEL controls; it is never an authorization input, and changing it
    /// client-side widens nothing.
    pub can_view_all: bool,
    /// The length of `items`. Never a fleet total.
    pub item_count: u64,
    pub filters_applied: SandboxFiltersView,
    pub items: Vec<SandboxItem>,
    /// Codes summarizing the returned rows, plus deployment-scope inventory
    /// health for a global administrator.
    pub warning_codes: Vec<SandboxWarning>,
}

/// Project the caller's normalized filters.
pub fn filters_view(request: &NormalizedSandboxRequest) -> SandboxFiltersView {
    let filters = &request.filters;
    SandboxFiltersView {
        status: filters.status.map(SandboxStatus::of),
        backend: filters.backend.map(SandboxBackend::of),
        creator_id: filters.creator_id,
        creator_login: filters.creator_login.clone(),
        repo_full_name: filters.repo_full_name.clone(),
        session_id: filters.session_id.clone(),
        trigger_issue: filters.trigger_issue,
        attribution_source: filters.attribution_source.map(SandboxAttributionSource::of),
    }
}

/// Assemble the response body from an already-authorized inventory.
pub fn response_from_inventory(
    inventory: &AuthorizedInventory,
    effective_scope: SandboxEffectiveScope,
    can_view_all: bool,
    filters_applied: SandboxFiltersView,
) -> SandboxInventoryResponse {
    SandboxInventoryResponse {
        observed_at: rfc3339(inventory.observed_at),
        backend: SandboxBackend::of(inventory.backend),
        effective_scope,
        can_view_all,
        item_count: inventory.items.len() as u64,
        filters_applied,
        items: inventory.items.iter().map(item_from_runtime).collect(),
        warning_codes: inventory
            .warning_codes
            .iter()
            .map(|code| SandboxWarning::of(*code))
            .collect(),
    }
}

/// Project one already-authorized runtime onto its wire item.
fn item_from_runtime(runtime: &AuthorizedRuntime) -> SandboxItem {
    let item: &RuntimeInventoryItem = &runtime.item;
    SandboxItem {
        backend: SandboxBackend::of(item.backend),
        runtime_id: item.runtime_id.clone(),
        runtime_name: item.runtime_name.clone(),
        runtime_uid: item.runtime_uid.clone(),
        backend_location: item.backend_location.clone(),
        session_id: item.session_id.clone(),
        managed: item.managed,
        metadata_state: SandboxMetadataState::of(item.metadata_state),
        creator_id: item.creator_id,
        creator_login: item.creator_login.clone(),
        trigger_author_id: item.trigger_author_id,
        trigger_author_login: item.trigger_author_login.clone(),
        attribution_source: SandboxAttributionSource::of(item.attribution_source),
        repo_full_name: item.repo_full_name.clone(),
        installation_id: item.installation_id,
        trigger_issue: item.trigger_issue,
        status: SandboxStatus::of(item.status),
        raw_status: item.raw_status.clone(),
        status_reason: item.status_reason.clone(),
        status_message: item.status_message.clone(),
        created_at: item.created_at.map(rfc3339),
        age_seconds: item.age_seconds,
        max_lifetime_seconds: item.max_lifetime_seconds,
        expires_at: item.expires_at.map(rfc3339),
        remaining_seconds: item.remaining_seconds,
        minimum_lifetime_seconds: item.minimum_lifetime_seconds,
        minimum_lifetime_remaining_seconds: item.minimum_lifetime_remaining_seconds,
        idle_grace_seconds: item.idle_grace_seconds,
        last_pending_at: item.last_pending_at.map(rfc3339),
        idle_for_seconds: item.idle_for_seconds,
        restart_count: item.restart_count,
        last_transition_at: item.last_transition_at.map(rfc3339),
        deletion_timestamp: item.deletion_timestamp.map(rfc3339),
        warning_codes: runtime
            .warning_codes
            .iter()
            .map(|code| SandboxWarning::of(*code))
            .collect(),
    }
}

/// RFC3339 UTC with millisecond precision — the exact form the audit contract
/// writes, so an inventory timestamp and a recorded one compare as equals.
fn rfc3339(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Millis, true)
}

#[cfg(test)]
#[path = "sandbox_dto_tests.rs"]
mod tests;
