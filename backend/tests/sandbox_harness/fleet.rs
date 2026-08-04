//! The runtime fixtures every test's fleet is built from.
//!
//! Deliberately terse: a [`RuntimeInventoryItem`] has thirty-odd fields, and a
//! test about authorization should read as "this row belongs to that session",
//! not as a thirty-line literal.

#![allow(dead_code)]

use fkst_control_plane::runtime_identity::{AttributionSource, RuntimeBackendKind};
use fkst_control_plane::session_backend::inventory::{
    BoundedInventoryWarning, InventoryWarningCode, RuntimeInventoryItem, RuntimeInventoryStatus,
    RuntimeMetadataState,
};
use k8s_openapi::chrono::{DateTime, TimeZone, Utc};

use super::backend::InventoryScript;

/// The fixture alias, so tests read `fleet::Item`.
pub type Item = RuntimeInventoryItem;

/// The instant every fixture snapshot reports. Fixed so a test can assert the
/// backend's `observed_at` is returned VERBATIM rather than replaced by `now`.
pub fn observed_at() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 7, 31, 12, 0, 0)
        .single()
        .expect("a valid fixed instant")
}

/// An instant `minutes` before [`observed_at`].
pub fn created(minutes: i64) -> DateTime<Utc> {
    observed_at() - k8s_openapi::chrono::Duration::minutes(minutes)
}

/// A complete, plausible Kubernetes runtime belonging to `session`.
pub fn item(runtime_id: &str, session: Option<&str>) -> Item {
    RuntimeInventoryItem {
        backend: RuntimeBackendKind::Kubernetes,
        runtime_id: runtime_id.to_string(),
        runtime_name: Some(runtime_id.to_string()),
        runtime_uid: Some(format!("uid-{runtime_id}")),
        backend_location: Some("chronoai-fkst".to_string()),
        session_id: session.map(str::to_string),
        managed: true,
        metadata_state: RuntimeMetadataState::Complete,
        creator_id: Some(101),
        creator_login: Some("alice".to_string()),
        trigger_author_id: Some(101),
        trigger_author_login: Some("alice".to_string()),
        attribution_source: AttributionSource::LaunchMetadata,
        repo_full_name: Some("acme/site".to_string()),
        installation_id: Some(1),
        trigger_issue: Some(7),
        status: RuntimeInventoryStatus::Running,
        raw_status: "Running".to_string(),
        status_reason: None,
        status_message: None,
        created_at: Some(created(10)),
        age_seconds: Some(600),
        max_lifetime_seconds: None,
        expires_at: None,
        remaining_seconds: None,
        minimum_lifetime_seconds: 300,
        minimum_lifetime_remaining_seconds: None,
        idle_grace_seconds: 900,
        last_pending_at: None,
        idle_for_seconds: Some(600),
        restart_count: Some(0),
        last_transition_at: None,
        deletion_timestamp: None,
        warnings: Vec::new(),
    }
}

/// The same runtime carrying its OWN data-quality codes, which is what a real
/// adapter records on the row (see `RuntimeInventoryItem::warnings`).
pub fn with_warnings(
    runtime_id: &str,
    session: Option<&str>,
    warnings: Vec<InventoryWarningCode>,
) -> Item {
    RuntimeInventoryItem {
        warnings,
        ..item(runtime_id, session)
    }
}

/// The same runtime with an explicit normalized state.
pub fn with_status(
    runtime_id: &str,
    session: Option<&str>,
    status: RuntimeInventoryStatus,
) -> Item {
    RuntimeInventoryItem {
        status,
        raw_status: status.as_str().to_string(),
        ..item(runtime_id, session)
    }
}

/// The same runtime with an explicit creation instant.
pub fn with_created(runtime_id: &str, session: Option<&str>, minutes_ago: i64) -> Item {
    RuntimeInventoryItem {
        created_at: Some(created(minutes_ago)),
        ..item(runtime_id, session)
    }
}

/// An orphan: managed, but with no session-id stamp to attribute it by.
pub fn orphan(runtime_id: &str) -> Item {
    RuntimeInventoryItem {
        session_id: None,
        creator_id: None,
        creator_login: None,
        trigger_author_id: None,
        trigger_author_login: None,
        repo_full_name: None,
        installation_id: None,
        trigger_issue: None,
        metadata_state: RuntimeMetadataState::Partial,
        attribution_source: AttributionSource::UnknownLegacy,
        warnings: vec![InventoryWarningCode::MissingSessionId],
        ..item(runtime_id, None)
    }
}

/// A runtime whose session-id stamp is not a valid session id at all.
pub fn malformed(runtime_id: &str) -> Item {
    RuntimeInventoryItem {
        session_id: Some("not a session id".to_string()),
        metadata_state: RuntimeMetadataState::Malformed,
        attribution_source: AttributionSource::PartialMetadata,
        warnings: vec![InventoryWarningCode::MalformedIdentity],
        ..item(runtime_id, None)
    }
}

/// A runtime whose stamped attribution disagrees with its trigger. The stamp is
/// reported verbatim and the disagreement is carried as the row's own warning —
/// that pair is how a global administrator finds the disputed row.
pub fn conflicted(runtime_id: &str, session: Option<&str>) -> Item {
    RuntimeInventoryItem {
        attribution_source: AttributionSource::Conflict,
        warnings: vec![InventoryWarningCode::AttributionConflict],
        ..item(runtime_id, session)
    }
}

/// An OpenSandbox runtime: no restart count, no deletion window, no separate
/// name or uid.
pub fn opensandbox(runtime_id: &str, session: Option<&str>) -> Item {
    RuntimeInventoryItem {
        backend: RuntimeBackendKind::OpenSandbox,
        runtime_name: None,
        runtime_uid: None,
        backend_location: Some("opensandbox".to_string()),
        restart_count: None,
        deletion_timestamp: None,
        ..item(runtime_id, session)
    }
}

/// A healthy snapshot holding `items` and no warnings.
pub fn snapshot(items: Vec<Item>) -> InventoryScript {
    InventoryScript::Snapshot {
        items,
        warnings: Vec::new(),
        observed_at: observed_at(),
    }
}

/// A healthy snapshot holding `items` and `warnings`.
pub fn snapshot_with_warnings(
    items: Vec<Item>,
    warnings: Vec<BoundedInventoryWarning>,
) -> InventoryScript {
    InventoryScript::Snapshot {
        items,
        warnings,
        observed_at: observed_at(),
    }
}
