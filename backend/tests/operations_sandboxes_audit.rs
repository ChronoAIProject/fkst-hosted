//! The audit half of `GET /api/v1/operations/sandboxes`.
//!
//! The operations UI polls this route, and capture is never allowed to skip its
//! own traffic (epic `AUD-01`) — so every request here, allowed or refused,
//! produces exactly one terminal record carrying only the reviewed safe
//! arguments.

mod sandbox_harness;

use axum::http::StatusCode;
use fkst_control_plane::audit::event::ApiRequestCompletedV1;
use sandbox_harness::fleet;
use sandbox_harness::{harness, harness_with, HarnessSpec, ALICE, GRACE, OTHER_SESSION, SESSION};

fn fleet() -> Vec<fleet::Item> {
    vec![
        fleet::item("mine", Some(SESSION)),
        fleet::item("theirs", Some(OTHER_SESSION)),
        fleet::orphan("orphan"),
    ]
}

/// The one record for a request, or a panic naming what was recorded instead.
fn sandbox_record(events: &[ApiRequestCompletedV1]) -> &ApiRequestCompletedV1 {
    let matching: Vec<&ApiRequestCompletedV1> = events
        .iter()
        .filter(|event| event.operation_id == "operations_list_sandboxes")
        .collect();
    assert_eq!(
        matching.len(),
        1,
        "exactly one terminal record per request; got {:?}",
        events
            .iter()
            .map(|event| event.operation_id.as_str())
            .collect::<Vec<_>>()
    );
    matching[0]
}

#[tokio::test]
async fn an_allowed_read_records_its_normalized_safe_arguments() {
    let harness = harness_with(fleet()).await;
    harness
        .snapshot(
            ALICE,
            &format!(
                "?session_id={SESSION}&repo_full_name=acme/site&trigger_issue=7\
                 &status=running&backend=kubernetes&creator_id=101&creator_login=@Alice\
                 &attribution_source=launch_metadata"
            ),
        )
        .await;

    let events = harness.audit.events();
    let record = sandbox_record(&events);
    assert_eq!(record.method, "GET");
    assert_eq!(record.route_template, "/api/v1/operations/sandboxes");
    assert_eq!(record.actor_id, Some(ALICE.0));
    assert_eq!(record.status_code, Some(200));

    let arguments = &record.arguments;
    assert_eq!(arguments["scope"], "accessible");
    assert_eq!(arguments["session_id"], SESSION);
    assert_eq!(arguments["repo_full_name"], "acme/site");
    assert_eq!(arguments["trigger_issue"], 7);
    assert_eq!(arguments["status"], "running");
    assert_eq!(arguments["backend"], "kubernetes");
    assert_eq!(arguments["creator_id"], 101);
    assert_eq!(
        arguments["creator_login"], "Alice",
        "the NORMALIZED value the query actually ran with"
    );
    assert_eq!(arguments["attribution_source"], "launch_metadata");
    assert!(
        !arguments.contains_key("requested_scope"),
        "an unstated scope adds nothing to the record"
    );
}

/// A refused scope selection is exactly the thing an audit trail must record.
#[tokio::test]
async fn a_refused_scope_selection_records_the_attempt() {
    let harness = harness_with(fleet()).await;
    let response = harness.get(ALICE, "?scope=all").await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let events = harness.audit.events();
    let record = sandbox_record(&events);
    assert_eq!(record.status_code, Some(403));
    assert_eq!(
        record.error_code.as_deref(),
        Some("operations_scope_forbidden")
    );
    assert_eq!(record.arguments["scope"], "accessible");
    assert_eq!(record.arguments["requested_scope"], "all");
}

/// An exact unauthorized session probe is recorded — but the record must not
/// become the oracle the response refuses to be, so it carries only the caller's
/// own validated filter and a stable code.
#[tokio::test]
async fn an_unauthorized_session_probe_is_recorded_with_a_stable_code() {
    let harness = harness_with(fleet()).await;
    let response = harness
        .get(ALICE, &format!("?session_id={OTHER_SESSION}"))
        .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let events = harness.audit.events();
    let record = sandbox_record(&events);
    assert_eq!(record.status_code, Some(404));
    assert_eq!(record.error_code.as_deref(), Some("sandbox_not_found"));
    assert_eq!(record.arguments["scope"], "accessible");
}

/// A rejected filter never becomes a property: the record says the input was
/// invalid, and keeps only bounded transport metadata.
#[tokio::test]
async fn an_invalid_filter_is_recorded_without_the_value_that_failed() {
    let harness = harness_with(fleet()).await;
    let response = harness.get(ALICE, "?status=canary-status-value").await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let events = harness.audit.events();
    let record = sandbox_record(&events);
    assert_eq!(record.status_code, Some(400));
    let rendered = serde_json::to_string(&record.arguments).expect("serializes");
    assert!(!rendered.contains("canary-status-value"), "{rendered}");
}

/// A backend failure must not carry an upstream message, host, or status into the
/// trail any more than into the response.
#[tokio::test]
async fn a_backend_failure_records_a_bounded_code_and_no_upstream_detail() {
    let harness = harness(HarnessSpec::new(sandbox_harness::InventoryScript::Failure)).await;
    let response = harness.get(GRACE, "").await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

    let events = harness.audit.events();
    let record = sandbox_record(&events);
    assert_eq!(record.status_code, Some(503));
    assert_eq!(
        record.error_code.as_deref(),
        Some("sandbox_inventory_unavailable")
    );
    // `Debug` is the widest projection the record has, so it is the strictest
    // place to look for an upstream detail that should never have been kept.
    let rendered = format!("{record:?}");
    for forbidden in ["10.0.0.1", "6443", "apiserver"] {
        assert!(!rendered.contains(forbidden), "{rendered}");
    }
}

/// Nothing about the fleet — not a hidden runtime id, not a session, not a count
/// — may reach the trail through the arguments.
#[tokio::test]
async fn no_record_carries_a_hidden_runtime_or_a_row_count() {
    let harness = harness_with(fleet()).await;
    harness.snapshot(ALICE, "").await;
    let events = harness.audit.events();
    let record = sandbox_record(&events);
    let rendered = serde_json::to_string(&record.arguments).expect("serializes");
    for forbidden in ["theirs", "orphan", OTHER_SESSION, "item_count"] {
        assert!(!rendered.contains(forbidden), "{rendered}");
    }
}
