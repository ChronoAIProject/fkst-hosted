//! Isolation, ordering, filters, warnings, capacity, and redaction for
//! `GET /api/v1/operations/sandboxes`.
//!
//! The central test in this file is
//! [`mutating_the_hidden_fleet_cannot_change_one_response_byte`]: the endpoint
//! reads the COMPLETE fleet into the process, so the only thing standing between
//! a caller and somebody else's runtime is the ORDER of the pipeline. Byte
//! equality across two wildly different hidden populations is the strongest
//! available statement that the order is right.

mod sandbox_harness;

use axum::http::StatusCode;
use fkst_control_plane::session_backend::inventory::{
    BoundedInventoryWarning, InventoryWarningCode, RuntimeInventoryStatus,
};
use sandbox_harness::fleet;
use sandbox_harness::{
    body_json, error_code, harness, harness_with, item_ids, HarnessSpec, ALICE, ERIN, GRACE,
    OTHER_SESSION, SESSION, UNKNOWN_SESSION,
};

/// Two runtimes Alice may see, in a fleet that also holds rows she may not.
fn visible_and_hidden() -> Vec<fleet::Item> {
    vec![
        fleet::with_status(
            "hidden-a",
            Some(OTHER_SESSION),
            RuntimeInventoryStatus::Failed,
        ),
        fleet::with_status(
            "mine-running",
            Some(SESSION),
            RuntimeInventoryStatus::Running,
        ),
        fleet::orphan("hidden-orphan"),
        fleet::with_status("mine-failed", Some(SESSION), RuntimeInventoryStatus::Failed),
        fleet::item("hidden-b", Some(UNKNOWN_SESSION)),
    ]
}

/// More warning-emitting hidden rows than one snapshot's shared warning budget
/// can hold. Deliberately above the ceiling: a budget consumed in fleet-list
/// order would otherwise strip Alice's own row of its codes, which is a hidden
/// runtime changing a regular caller's response.
const HIDDEN_OVER_THE_WARNING_CEILING: usize = 306;

/// A completely different hidden population: three hundred more rows, different
/// ids, different states, a different order, and a warning on every one of them.
fn mutated_hidden_fleet() -> Vec<fleet::Item> {
    let mut fleet: Vec<fleet::Item> = (0..HIDDEN_OVER_THE_WARNING_CEILING)
        .map(|index| fleet::Item {
            status: RuntimeInventoryStatus::Pending,
            raw_status: RuntimeInventoryStatus::Pending.as_str().to_string(),
            ..fleet::with_warnings(
                &format!("aaa-hidden-{index:03}"),
                Some(OTHER_SESSION),
                vec![InventoryWarningCode::MalformedIdentity],
            )
        })
        .collect();
    fleet.push(fleet::malformed("zzz-malformed"));
    fleet.push(fleet::with_status(
        "mine-running",
        Some(SESSION),
        RuntimeInventoryStatus::Running,
    ));
    fleet.push(fleet::with_status(
        "mine-failed",
        Some(SESSION),
        RuntimeInventoryStatus::Failed,
    ));
    fleet
}

/// What a real adapter reports once that many rows have warned: a clipped
/// snapshot-scope diagnostic, and nothing per-row left to say.
fn hidden_warnings() -> Vec<BoundedInventoryWarning> {
    vec![BoundedInventoryWarning::snapshot(
        InventoryWarningCode::WarningsTruncated,
    )]
}

/// The claim: authorization runs before filters, ordering, `item_count`, warning
/// projection, the result ceiling, and serialization — so nothing derived from a
/// hidden row can appear anywhere in an authorized response.
#[tokio::test]
async fn mutating_the_hidden_fleet_cannot_change_one_response_byte() {
    let baseline = harness_with(visible_and_hidden())
        .await
        .snapshot_bytes(ALICE, "")
        .await;

    let mutated = harness(HarnessSpec::new(fleet::snapshot_with_warnings(
        mutated_hidden_fleet(),
        hidden_warnings(),
    )))
    .await
    .snapshot_bytes(ALICE, "")
    .await;

    assert_eq!(
        String::from_utf8_lossy(&mutated),
        String::from_utf8_lossy(&baseline),
        "a hidden runtime must not change the body, the order, the count, or the \
         warning codes of an authorized response"
    );
}

/// The same claim for the caller who sees nothing at all: an empty snapshot must
/// not leak the fleet's size through its shape.
#[tokio::test]
async fn an_unrelated_callers_empty_snapshot_is_identical_whatever_the_fleet_holds() {
    let baseline = harness_with(visible_and_hidden())
        .await
        .snapshot_bytes(ERIN, "")
        .await;
    let mutated = harness(HarnessSpec::new(fleet::snapshot_with_warnings(
        mutated_hidden_fleet(),
        hidden_warnings(),
    )))
    .await
    .snapshot_bytes(ERIN, "")
    .await;
    assert_eq!(
        String::from_utf8_lossy(&mutated),
        String::from_utf8_lossy(&baseline)
    );
}

#[tokio::test]
async fn active_and_problem_states_sort_before_terminal_ones_then_newest_first() {
    let harness = harness_with(vec![
        fleet::with_status(
            "t-terminated",
            Some(SESSION),
            RuntimeInventoryStatus::Terminated,
        ),
        fleet::with_status("r-running", Some(SESSION), RuntimeInventoryStatus::Running),
        fleet::with_status("f-failed", Some(SESSION), RuntimeInventoryStatus::Failed),
        fleet::with_status("p-pending", Some(SESSION), RuntimeInventoryStatus::Pending),
        fleet::with_status(
            "s-succeeded",
            Some(SESSION),
            RuntimeInventoryStatus::Succeeded,
        ),
    ])
    .await;
    let snapshot = harness.snapshot(ALICE, "").await;
    assert_eq!(
        item_ids(&snapshot),
        vec![
            "f-failed",
            "p-pending",
            "r-running",
            "s-succeeded",
            "t-terminated"
        ]
    );
}

#[tokio::test]
async fn within_one_state_the_newest_runtime_comes_first_and_a_dateless_one_last() {
    let dateless = fleet::Item {
        created_at: None,
        ..fleet::item("aaa-dateless", Some(SESSION))
    };
    let harness = harness_with(vec![
        fleet::with_created("old", Some(SESSION), 600),
        dateless,
        fleet::with_created("new", Some(SESSION), 1),
    ])
    .await;
    let snapshot = harness.snapshot(ALICE, "").await;
    assert_eq!(item_ids(&snapshot), vec!["new", "old", "aaa-dateless"]);
}

/// Filters run AFTER authorization, so naming somebody else narrows to nothing
/// rather than widening to their rows.
#[tokio::test]
async fn no_filter_can_widen_a_callers_authorized_row_set() {
    let harness = harness_with(visible_and_hidden()).await;
    for query in [
        "?creator_id=707",
        "?creator_login=stranger",
        "?repo_full_name=acme/other",
    ] {
        let snapshot = harness.snapshot(ALICE, query).await;
        assert!(
            item_ids(&snapshot).is_empty(),
            "{query} must narrow, never widen"
        );
        assert_eq!(snapshot["item_count"], 0);
    }
}

#[tokio::test]
async fn every_filter_narrows_within_the_authorized_set() {
    let harness = harness_with(visible_and_hidden()).await;
    for (query, expected) in [
        ("?status=running", vec!["mine-running"]),
        ("?status=failed", vec!["mine-failed"]),
        ("?backend=kubernetes", vec!["mine-failed", "mine-running"]),
        ("?backend=opensandbox", vec![]),
        ("?creator_id=101", vec!["mine-failed", "mine-running"]),
        ("?creator_login=ALICE", vec!["mine-failed", "mine-running"]),
        (
            "?repo_full_name=acme/site",
            vec!["mine-failed", "mine-running"],
        ),
        ("?trigger_issue=7", vec!["mine-failed", "mine-running"]),
        ("?trigger_issue=9", vec![]),
        (
            "?attribution_source=launch_metadata",
            vec!["mine-failed", "mine-running"],
        ),
        ("?attribution_source=unknown_legacy", vec![]),
        ("?attribution_source=conflict", vec![]),
        ("?attribution_source=partial_metadata", vec![]),
        (
            "?status=running&creator_id=101&repo_full_name=acme/site",
            vec!["mine-running"],
        ),
    ] {
        let snapshot = harness.snapshot(ALICE, query).await;
        assert_eq!(item_ids(&snapshot), expected, "{query}");
        assert_eq!(snapshot["item_count"], expected.len(), "{query}");
    }
}

#[tokio::test]
async fn every_invalid_filter_is_a_400_before_the_backend_is_touched() {
    let harness = harness_with(visible_and_hidden()).await;
    for query in [
        "?scope=mine",
        "?status=melted",
        "?backend=nomad",
        "?creator_id=0",
        "?creator_login=not%20a%20login",
        "?repo_full_name=acme",
        "?session_id=sess%2F..%2Fetc",
        "?trigger_issue=0",
        "?attribution_source=vibes",
    ] {
        let response = harness.get(ALICE, query).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{query}");
        assert_eq!(error_code(response).await, "invalid_request", "{query}");
    }
    assert_eq!(
        harness.inventory_calls(),
        0,
        "a malformed request must cost the deployment nothing"
    );
}

/// A warning belonging to a hidden runtime must not reach a regular caller — not
/// on an item, and not on the response.
#[tokio::test]
async fn warnings_reach_a_caller_only_for_rows_that_caller_receives() {
    let harness = harness(HarnessSpec::new(fleet::snapshot_with_warnings(
        vec![
            fleet::with_warnings(
                "hidden-a",
                Some(OTHER_SESSION),
                vec![InventoryWarningCode::MalformedIdentity],
            ),
            fleet::with_warnings(
                "mine-running",
                Some(SESSION),
                vec![InventoryWarningCode::AttributionConflict],
            ),
            fleet::orphan("hidden-orphan"),
        ],
        vec![BoundedInventoryWarning::snapshot(
            InventoryWarningCode::WarningsTruncated,
        )],
    )))
    .await;

    let snapshot = harness.snapshot(ALICE, "").await;
    let rendered = snapshot.to_string();
    assert!(!rendered.contains("malformed_identity"), "{rendered}");
    assert!(!rendered.contains("hidden-a"), "{rendered}");
    assert!(
        !rendered.contains("warnings_incomplete"),
        "a snapshot-scope warning is deployment health, and letting it through \
         would let a hidden runtime change a regular caller's response: {rendered}"
    );
    let item = snapshot["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|item| item["runtime_id"] == "mine-running")
        .expect("the visible row");
    assert_eq!(item["warning_codes"][0], "attribution_conflict");
    assert_eq!(snapshot["warning_codes"][0], "attribution_conflict");

    // A global administrator gets the deployment-scope code as well.
    let admin = harness.snapshot(GRACE, "").await;
    let codes = admin["warning_codes"].to_string();
    assert!(codes.contains("warnings_incomplete"), "{codes}");
    assert!(codes.contains("malformed_identity"), "{codes}");
}

/// The sharpest form of the warning half: a hidden population big enough to
/// exhaust the snapshot's shared warning budget must not cost the ONE row Alice
/// can see its own codes.
#[tokio::test]
async fn a_warning_flood_from_hidden_rows_cannot_strip_a_visible_rows_codes() {
    let mut hidden = mutated_hidden_fleet();
    hidden.retain(|item| !item.runtime_id.starts_with("mine-"));
    hidden.push(fleet::with_warnings(
        "mine-running",
        Some(SESSION),
        vec![InventoryWarningCode::ClockSkew],
    ));

    let harness = harness(HarnessSpec::new(fleet::snapshot_with_warnings(
        hidden,
        hidden_warnings(),
    )))
    .await;
    let snapshot = harness.snapshot(ALICE, "").await;
    assert_eq!(item_ids(&snapshot), vec!["mine-running"]);
    assert_eq!(snapshot["items"][0]["warning_codes"][0], "clock_skew");
    assert_eq!(snapshot["warning_codes"][0], "clock_skew");
    assert_eq!(
        snapshot["warning_codes"].as_array().expect("array").len(),
        1,
        "the clipped fleet-wide diagnostic stays out of a regular response"
    );
}

/// A clipped page walk means the fleet read was INCOMPLETE, so no answer derived
/// from it may claim to be the complete matching set.
#[tokio::test]
async fn a_truncated_source_walk_is_a_capacity_failure_for_every_caller() {
    let harness = harness(HarnessSpec::new(fleet::snapshot_with_warnings(
        visible_and_hidden(),
        vec![BoundedInventoryWarning::snapshot(
            InventoryWarningCode::SourceTruncated,
        )],
    )))
    .await;
    for who in [ALICE, GRACE] {
        let response = harness.get(who, "").await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(error_code(response).await, "sandbox_inventory_too_large");
    }
}

/// The public ceiling counts AUTHORIZED rows, so a large hidden fleet cannot fail
/// a caller whose own result is small — while the same ceiling does bound the
/// administrator whose result IS the fleet.
#[tokio::test]
async fn the_result_ceiling_never_fails_a_caller_for_rows_they_cannot_see() {
    let mut fleet: Vec<fleet::Item> = (0..30)
        .map(|index| fleet::item(&format!("hidden-{index:02}"), Some(OTHER_SESSION)))
        .collect();
    fleet.push(fleet::item("mine-1", Some(SESSION)));
    fleet.push(fleet::item("mine-2", Some(SESSION)));

    let harness = harness(HarnessSpec::new(fleet::snapshot(fleet)).max_result_items(3)).await;
    let snapshot = harness.snapshot(ALICE, "").await;
    assert_eq!(snapshot["item_count"], 2);

    let response = harness.get(GRACE, "").await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = body_json(response).await;
    assert_eq!(body["error"], "sandbox_inventory_too_large");
    let rendered = body.to_string();
    for count in ["32", "30", "3"] {
        assert!(
            !rendered.contains(count),
            "a capacity failure must carry no count: {rendered}"
        );
    }
}

/// Nothing that reaches the wire may be a credential, an access-list entry, or a
/// raw backend object.
#[tokio::test]
async fn no_response_carries_a_secret_or_an_access_list_canary() {
    let harness = harness_with(visible_and_hidden()).await;
    for who in [ALICE, GRACE] {
        let rendered = harness.snapshot(who, "").await.to_string();
        for canary in [
            "token",
            "Bearer",
            "secret",
            "password",
            "collaborator",
            "log_access",
            "allowlist",
            "bob",
            "carol",
            "dana",
            "kubeconfig",
            "apiVersion",
        ] {
            assert!(
                !rendered.contains(canary),
                "{who:?} response leaked {canary}: {rendered}"
            );
        }
    }
}

/// The bounded, closed-label telemetry: no viewer, session, runtime, or filter
/// value may ever become a Prometheus label.
#[tokio::test]
async fn the_metric_families_are_rendered_with_closed_labels_only() {
    let harness = harness_with(visible_and_hidden()).await;
    harness.snapshot(ALICE, "").await;
    let response = harness.get(ALICE, "?scope=all").await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let metrics = harness
        .router
        .clone()
        .oneshot(
            axum::http::Request::get("/metrics")
                .body(axum::body::Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("router responds");
    let body = http_body_util::BodyExt::collect(metrics.into_body())
        .await
        .expect("collect body")
        .to_bytes();
    let body = String::from_utf8(body.to_vec()).expect("utf-8 exposition");

    assert!(body.contains(
        "fkst_operations_sandbox_inventory_requests_total{backend=\"kubernetes\",scope=\"accessible\",result=\"success\"} 1"
    ), "{body}");
    assert!(body.contains(
        "fkst_operations_sandbox_inventory_requests_total{backend=\"kubernetes\",scope=\"accessible\",result=\"forbidden\"} 1"
    ), "{body}");
    assert!(
        body.contains(
            "fkst_operations_sandbox_scope_rejections_total{reason=\"global_scope_forbidden\"} 1"
        ),
        "{body}"
    );
    assert!(body.contains(
        "fkst_operations_sandbox_inventory_items{backend=\"kubernetes\",scope=\"accessible\"} 2"
    ), "{body}");
    assert!(
        body.contains("fkst_operations_sandbox_inventory_duration_seconds_count{backend=\"kubernetes\",result=\"success\"} 1"),
        "{body}"
    );
    for forbidden in ["alice", "sess-alice", "mine-running", "acme/site", "101"] {
        assert!(
            !body.contains(&format!("=\"{forbidden}\"")),
            "{forbidden} became a metric label: {body}"
        );
    }
}

use tower::ServiceExt;

/// One activity request through the same router the sandbox tests drive.
async fn activity(harness: &sandbox_harness::Harness) -> axum::http::Response<axum::body::Body> {
    harness
        .router
        .clone()
        .oneshot(
            axum::http::Request::get("/api/v1/operations/activity")
                .header("host", "test")
                .header(
                    "authorization",
                    format!("Bearer {}", sandbox_harness::token(ALICE)),
                )
                .body(axum::body::Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("router responds")
}

/// A PostHog outage must never hide live runtime state: the activity source is
/// configured here and FAILING, while the runtime backend is healthy.
#[tokio::test]
async fn a_failing_activity_source_does_not_affect_the_live_inventory() {
    let harness = harness(
        HarnessSpec::new(fleet::snapshot(visible_and_hidden())).activity(/* healthy */ false),
    )
    .await;

    let response = activity(&harness).await;
    assert_eq!(
        response.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "the fixture's activity source really is down"
    );
    assert_eq!(error_code(response).await, "unavailable");

    let snapshot = harness.snapshot(ALICE, "").await;
    assert_eq!(snapshot["item_count"], 2);
}

/// And the converse, in the non-degenerate direction: the runtime backend is down
/// while the activity source is healthy, and the activity surface still answers
/// `200`. Neither endpoint may become the other's dependency.
#[tokio::test]
async fn a_failing_runtime_backend_does_not_change_the_activity_answer() {
    let harness = harness(
        HarnessSpec::new(sandbox_harness::InventoryScript::Failure)
            .activity(/* healthy */ true),
    )
    .await;

    let sandboxes = harness.get(ALICE, "").await;
    assert_eq!(sandboxes.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(error_code(sandboxes).await, "sandbox_inventory_unavailable");

    let response = activity(&harness).await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the activity surface reports ITS own health, never the runtime's: {}",
        String::from_utf8_lossy(
            &http_body_util::BodyExt::collect(response.into_body())
                .await
                .expect("collect body")
                .to_bytes()
        )
    );
}

/// The deployment that configured no read credentials at all still says so, and
/// still serves live inventory.
#[tokio::test]
async fn an_unconfigured_activity_source_does_not_affect_the_live_inventory() {
    let harness = harness_with(visible_and_hidden()).await;
    let snapshot = harness.snapshot(ALICE, "").await;
    assert_eq!(snapshot["item_count"], 2);

    let response = activity(&harness).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(error_code(response).await, "audit_query_not_configured");
}
