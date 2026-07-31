//! Unit tests for the public projection.
//!
//! The claims under test are about the WIRE shape: every field survives the
//! projection, an unsupported field is an explicit `null` rather than an omitted
//! key, and no value that reached the item can be a credential.

use super::*;
use crate::operations::sandbox::test_support::{instant, item};
use crate::operations::sandbox::AuthorizedInventory;
use crate::session_backend::inventory::RuntimeInventoryStatus;

fn authorized(item: RuntimeInventoryItem, codes: Vec<SandboxWarningCode>) -> AuthorizedRuntime {
    AuthorizedRuntime {
        item,
        warning_codes: codes,
    }
}

fn response(items: Vec<AuthorizedRuntime>) -> SandboxInventoryResponse {
    response_from_inventory(
        &AuthorizedInventory {
            observed_at: instant(12, 30),
            backend: RuntimeBackendKind::Kubernetes,
            items,
            warning_codes: vec![SandboxWarningCode::WarningsIncomplete],
        },
        SandboxEffectiveScope::Accessible,
        false,
        SandboxFiltersView::default(),
    )
}

#[test]
fn the_snapshot_instant_is_rendered_in_the_audit_contracts_own_form() {
    let rendered = response(Vec::new());
    assert_eq!(rendered.observed_at, "2026-07-31T12:30:00.000Z");
    assert_eq!(rendered.item_count, 0);
}

#[test]
fn item_count_is_the_length_of_items_and_nothing_else() {
    let rendered = response(vec![
        authorized(item("a", Some("sess-a")), Vec::new()),
        authorized(item("b", Some("sess-a")), Vec::new()),
    ]);
    assert_eq!(rendered.item_count, 2);
    assert_eq!(rendered.items.len(), 2);
}

/// A backend that cannot know something and a field that happens to be absent
/// must be the SAME wire shape, or a client cannot ask "is this supported here?".
#[test]
fn an_unsupported_field_serializes_as_an_explicit_null() {
    let sparse = RuntimeInventoryItem {
        restart_count: None,
        runtime_name: None,
        runtime_uid: None,
        deletion_timestamp: None,
        created_at: None,
        age_seconds: None,
        ..item("osb-1", Some("sess-a"))
    };
    let rendered = response(vec![authorized(sparse, Vec::new())]);
    let json = serde_json::to_value(&rendered).expect("serializes");
    let item = &json["items"][0];
    for field in [
        "restart_count",
        "runtime_name",
        "runtime_uid",
        "deletion_timestamp",
        "created_at",
        "age_seconds",
        "expires_at",
        "remaining_seconds",
        "status_reason",
        "status_message",
    ] {
        assert!(
            item.get(field).is_some_and(serde_json::Value::is_null),
            "{field} must be present as null, not omitted: {item}"
        );
    }
}

#[test]
fn every_epic_required_field_survives_the_projection() {
    let complete = RuntimeInventoryItem {
        max_lifetime_seconds: Some(3_600),
        expires_at: Some(instant(13, 0)),
        remaining_seconds: Some(1_800),
        minimum_lifetime_remaining_seconds: Some(60),
        last_pending_at: Some(instant(12, 15)),
        last_transition_at: Some(instant(12, 20)),
        deletion_timestamp: Some(instant(12, 25)),
        status_reason: Some("CrashLoopBackOff".to_string()),
        status_message: Some("back-off restarting".to_string()),
        restart_count: Some(4),
        ..item("k8s-1", Some("sess-a"))
    };
    let rendered = response(vec![authorized(
        complete,
        vec![SandboxWarningCode::ClockSkew],
    )]);
    let json = serde_json::to_value(&rendered).expect("serializes");
    let item = &json["items"][0];
    assert_eq!(item["runtime_id"], "k8s-1");
    assert_eq!(item["session_id"], "sess-a");
    assert_eq!(item["backend"], "kubernetes");
    assert_eq!(item["metadata_state"], "complete");
    assert_eq!(item["attribution_source"], "launch_metadata");
    assert_eq!(item["creator_id"], 101);
    assert_eq!(item["creator_login"], "alice");
    assert_eq!(item["trigger_author_id"], 101);
    assert_eq!(item["repo_full_name"], "acme/site");
    assert_eq!(item["installation_id"], 1);
    assert_eq!(item["trigger_issue"], 7);
    assert_eq!(item["status"], "running");
    assert_eq!(item["raw_status"], "Running");
    assert_eq!(item["status_reason"], "CrashLoopBackOff");
    assert_eq!(item["max_lifetime_seconds"], 3_600);
    assert_eq!(item["expires_at"], "2026-07-31T13:00:00.000Z");
    assert_eq!(item["remaining_seconds"], 1_800);
    assert_eq!(item["minimum_lifetime_seconds"], 300);
    assert_eq!(item["minimum_lifetime_remaining_seconds"], 60);
    assert_eq!(item["idle_grace_seconds"], 900);
    assert_eq!(item["last_pending_at"], "2026-07-31T12:15:00.000Z");
    assert_eq!(item["idle_for_seconds"], 600);
    assert_eq!(item["restart_count"], 4);
    assert_eq!(item["last_transition_at"], "2026-07-31T12:20:00.000Z");
    assert_eq!(item["deletion_timestamp"], "2026-07-31T12:25:00.000Z");
    assert_eq!(item["managed"], true);
    assert_eq!(item["warning_codes"][0], "clock_skew");
    assert_eq!(json["warning_codes"][0], "warnings_incomplete");
}

/// An unlimited configured lifetime must report NULL, not "0 seconds remaining".
#[test]
fn an_unlimited_lifetime_reports_null_rather_than_zero() {
    let rendered = response(vec![authorized(item("k8s-1", Some("sess-a")), Vec::new())]);
    let json = serde_json::to_value(&rendered).expect("serializes");
    assert!(json["items"][0]["max_lifetime_seconds"].is_null());
    assert!(json["items"][0]["remaining_seconds"].is_null());
}

#[test]
fn every_closed_enum_renders_its_snake_case_wire_value() {
    for (status, expected) in [
        (RuntimeInventoryStatus::Pending, "pending"),
        (RuntimeInventoryStatus::Running, "running"),
        (RuntimeInventoryStatus::Paused, "paused"),
        (RuntimeInventoryStatus::Transitioning, "transitioning"),
        (RuntimeInventoryStatus::Succeeded, "succeeded"),
        (RuntimeInventoryStatus::Failed, "failed"),
        (RuntimeInventoryStatus::Terminating, "terminating"),
        (RuntimeInventoryStatus::Terminated, "terminated"),
        (RuntimeInventoryStatus::Unknown, "unknown"),
    ] {
        let value = serde_json::to_value(SandboxStatus::of(status)).expect("serializes");
        assert_eq!(value, expected);
    }
    for code in SandboxWarningCode::ALL {
        let value = serde_json::to_value(SandboxWarning::of(code)).expect("serializes");
        assert_eq!(value, code.as_str());
    }
    for source in AttributionSource::ALL {
        let value = serde_json::to_value(SandboxAttributionSource::of(source)).expect("serializes");
        assert_eq!(value, source.as_str());
    }
    assert_eq!(
        serde_json::to_value(SandboxBackend::of(RuntimeBackendKind::OpenSandbox))
            .expect("serializes"),
        "opensandbox"
    );
}

/// The echo is the NORMALIZED filter set, so a client can confirm which query ran
/// without the server ever repeating a value it refused.
#[test]
fn the_filter_echo_reports_the_normalized_values() {
    let filters =
        super::super::sandbox_query::filters(&super::super::sandbox_query::SandboxQueryParams {
            creator_login: Some("@Alice".to_string()),
            status: Some("failed".to_string()),
            ..Default::default()
        })
        .expect("normalizes");
    let view = filters_view(&filters);
    assert_eq!(view.creator_login.as_deref(), Some("Alice"));
    assert_eq!(view.status, Some(SandboxStatus::Failed));
    assert_eq!(view.session_id, None);
}
