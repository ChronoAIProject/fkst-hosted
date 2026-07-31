//! End-to-end authorization for `GET /api/v1/operations/sandboxes`.
//!
//! Every test here drives the REAL router: the GitHub identity extractor, the
//! deployment access policy, the scope gate, the session-visibility registry, the
//! handler, the audit middleware, and `AppError` conversion all run.
//!
//! The claims under test are the epic's: who may see which row, that an exact
//! unauthorized session is indistinguishable from a nonexistent one, that a
//! refused request costs the deployment nothing, and that a registry outage
//! blocks the personal scope ONLY.

mod sandbox_harness;

use axum::http::StatusCode;
use fkst_control_plane::session_backend::inventory::RuntimeInventoryStatus;
use sandbox_harness::fleet;
use sandbox_harness::{
    body_json, error_code, harness, harness_with, item_ids, HarnessSpec, InventoryScript, ALICE,
    BOB, CAROL, DANA, ERIN, FRANK, GRACE, OTHER_SESSION, SESSION, UNKNOWN_SESSION,
};

/// A fleet with one runtime for Alice's session, one for a stranger's, plus the
/// three shapes only a global administrator may ever see.
fn mixed_fleet() -> Vec<fleet::Item> {
    vec![
        fleet::item("mine", Some(SESSION)),
        fleet::item("theirs", Some(OTHER_SESSION)),
        fleet::orphan("orphan"),
        fleet::malformed("malformed"),
        fleet::item("unknown-ctx", Some(UNKNOWN_SESSION)),
    ]
}

#[tokio::test]
async fn a_request_without_an_identity_is_unauthorized() {
    let harness = harness_with(mixed_fleet()).await;
    let response = harness.request(None, "").await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response
            .headers()
            .get("www-authenticate")
            .and_then(|value| value.to_str().ok()),
        Some("Bearer")
    );
    assert_eq!(
        harness.inventory_calls(),
        0,
        "an unauthenticated request must cost the deployment nothing"
    );
}

/// The per-session tiers the epic names, end to end.
#[tokio::test]
async fn the_creator_collaborator_and_log_grantee_all_see_exactly_that_session() {
    let harness = harness_with(mixed_fleet()).await;
    for who in [ALICE, BOB, CAROL] {
        let snapshot = harness.snapshot(who, "").await;
        assert_eq!(item_ids(&snapshot), vec!["mine"], "{} sees it", who.1);
        assert_eq!(snapshot["item_count"], 1);
        assert_eq!(snapshot["effective_scope"], "accessible");
        assert_eq!(snapshot["can_view_all"], false);
    }
}

/// `FKST_LOG_ADMINS` is a deployment-wide CROSS-SESSION grant, and the milestone
/// preserves it verbatim: Dana therefore sees every session the registry knows.
/// She still sees no orphan, malformed, or unknown-context row — those have no
/// registry context at all, which is a global-admin-only condition.
#[tokio::test]
async fn a_legacy_log_admin_keeps_their_cross_session_grant_but_not_the_admin_view() {
    let harness = harness_with(mixed_fleet()).await;
    let snapshot = harness.snapshot(DANA, "").await;
    let mut ids = item_ids(&snapshot);
    ids.sort();
    assert_eq!(ids, vec!["mine", "theirs"]);
    assert_eq!(snapshot["item_count"], 2);
    assert_eq!(snapshot["effective_scope"], "accessible");
    assert_eq!(snapshot["can_view_all"], false);
}

/// Repository ownership is deliberately not a tier: Frank is refused exactly like
/// the unrelated Erin, and both get a complete, honest, empty snapshot.
#[tokio::test]
async fn an_unrelated_user_and_the_repository_owner_both_receive_an_empty_snapshot() {
    let harness = harness_with(mixed_fleet()).await;
    for who in [ERIN, FRANK] {
        let snapshot = harness.snapshot(who, "").await;
        assert!(item_ids(&snapshot).is_empty(), "{} sees nothing", who.1);
        assert_eq!(snapshot["item_count"], 0);
        assert!(snapshot["warning_codes"]
            .as_array()
            .expect("array")
            .is_empty());
    }
}

#[tokio::test]
async fn a_global_admin_defaults_to_the_complete_fleet_including_unattributable_rows() {
    let harness = harness_with(mixed_fleet()).await;
    let snapshot = harness.snapshot(GRACE, "").await;
    assert_eq!(snapshot["effective_scope"], "all");
    assert_eq!(snapshot["can_view_all"], true);
    assert_eq!(snapshot["item_count"], 5);
    let ids = item_ids(&snapshot);
    for expected in ["mine", "theirs", "orphan", "malformed", "unknown-ctx"] {
        assert!(ids.contains(&expected.to_string()), "{expected} missing");
    }
    // The unattributable rows carry their state EXPLICITLY rather than being
    // quietly normalized into something attributable.
    let items = snapshot["items"].as_array().expect("items");
    let orphan = items
        .iter()
        .find(|item| item["runtime_id"] == "orphan")
        .expect("the orphan row");
    assert!(orphan["session_id"].is_null());
    assert_eq!(orphan["attribution_source"], "unknown_legacy");
    assert_eq!(orphan["metadata_state"], "partial");
    let malformed = items
        .iter()
        .find(|item| item["runtime_id"] == "malformed")
        .expect("the malformed row");
    assert_eq!(malformed["metadata_state"], "malformed");
}

/// The whole reason `accessible` exists for an administrator: inspect only what
/// you directly own or were granted, without the global bypass.
#[tokio::test]
async fn a_global_admin_can_select_the_accessible_scope_without_their_bypass() {
    let harness = harness_with(mixed_fleet()).await;
    let snapshot = harness.snapshot(GRACE, "?scope=accessible").await;
    assert_eq!(snapshot["effective_scope"], "accessible");
    assert_eq!(
        snapshot["can_view_all"], true,
        "the capability is still reported; only the SELECTED scope narrowed"
    );
    assert!(item_ids(&snapshot).is_empty());
}

#[tokio::test]
async fn a_regular_caller_requesting_the_global_scope_is_refused_before_the_backend() {
    let harness = harness_with(mixed_fleet()).await;
    let response = harness.get(ALICE, "?scope=all").await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(error_code(response).await, "operations_scope_forbidden");
    assert_eq!(
        harness.inventory_calls(),
        0,
        "a refused scope must cost zero backend calls"
    );
}

#[tokio::test]
async fn an_administrator_may_state_the_global_scope_explicitly() {
    let harness = harness_with(mixed_fleet()).await;
    let snapshot = harness.snapshot(GRACE, "?scope=all").await;
    assert_eq!(snapshot["effective_scope"], "all");
    assert_eq!(snapshot["item_count"], 5);
}

/// The anti-enumeration contract: an exact session id that is unknown and one
/// that is merely somebody else's answer identically, and neither reaches the
/// backend.
#[tokio::test]
async fn an_unknown_and_an_unauthorized_session_id_are_indistinguishable() {
    let harness = harness_with(mixed_fleet()).await;
    let mut bodies = Vec::new();
    for session in [UNKNOWN_SESSION, OTHER_SESSION] {
        let response = harness.get(ALICE, &format!("?session_id={session}")).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        bodies.push(body_json(response).await);
    }
    assert_eq!(
        bodies[0], bodies[1],
        "the two answers must be byte-identical"
    );
    assert_eq!(bodies[0]["error"], "sandbox_not_found");
    assert_eq!(
        harness.inventory_calls(),
        0,
        "preauthorization happens BEFORE the backend is called"
    );
}

#[tokio::test]
async fn an_authorized_exact_session_id_is_served_normally() {
    let harness = harness_with(mixed_fleet()).await;
    let snapshot = harness
        .snapshot(ALICE, &format!("?session_id={SESSION}"))
        .await;
    assert_eq!(item_ids(&snapshot), vec!["mine"]);
    assert_eq!(snapshot["filters_applied"]["session_id"], SESSION);
}

/// A global administrator needs no preauthorization: they may inspect a session
/// the registry has never heard of, because the fleet contains rows with no
/// registry context at all.
#[tokio::test]
async fn a_global_admin_may_filter_by_a_session_the_registry_does_not_know() {
    let harness = harness_with(mixed_fleet()).await;
    let snapshot = harness
        .snapshot(GRACE, &format!("?session_id={UNKNOWN_SESSION}"))
        .await;
    assert_eq!(item_ids(&snapshot), vec!["unknown-ctx"]);
}

/// A cold projection cannot distinguish "you have none" from "I do not yet know
/// which are yours", so it must not answer an apparently complete empty snapshot.
#[tokio::test]
async fn a_cold_projection_blocks_the_accessible_scope_with_a_stable_503() {
    let harness = harness(HarnessSpec::new(fleet::snapshot(mixed_fleet())).cold_registry()).await;
    let response = harness.get(ALICE, "").await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(error_code(response).await, "session_visibility_unavailable");
    assert_eq!(harness.inventory_calls(), 0);
}

/// Registry health and runtime health are independent failures: a global
/// administrator's complete fleet view must survive a visibility outage.
#[tokio::test]
async fn a_cold_projection_does_not_block_the_global_fleet_view() {
    let harness = harness(HarnessSpec::new(fleet::snapshot(mixed_fleet())).cold_registry()).await;
    let snapshot = harness.snapshot(GRACE, "").await;
    assert_eq!(snapshot["item_count"], 5);
    assert_eq!(harness.inventory_calls(), 1);
}

/// A cold projection also refuses an exact session probe — with the RETRYABLE
/// code, because "retry shortly" and "no such session" need different remedies.
#[tokio::test]
async fn a_cold_projection_refuses_an_exact_session_probe_as_unavailable() {
    let harness = harness(HarnessSpec::new(fleet::snapshot(mixed_fleet())).cold_registry()).await;
    let response = harness.get(ALICE, &format!("?session_id={SESSION}")).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(error_code(response).await, "session_visibility_unavailable");
}

#[tokio::test]
async fn a_deployment_without_a_runtime_backend_reports_the_feature_disabled() {
    let harness = harness(HarnessSpec::without_backend()).await;
    for who in [ALICE, GRACE] {
        let response = harness.get(who, "").await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(error_code(response).await, "sandbox_inventory_disabled");
    }
}

/// A backend failure, a timeout, and an oversize fleet are three distinct
/// operator stories — and none of them may leak an upstream detail.
#[tokio::test]
async fn every_backend_failure_answers_a_bounded_503_without_detail() {
    for (script, expected) in [
        (InventoryScript::Failure, "sandbox_inventory_unavailable"),
        (
            InventoryScript::TooLarge { limit: 4_242 },
            "sandbox_inventory_too_large",
        ),
    ] {
        let harness = harness(HarnessSpec::new(script)).await;
        let response = harness.get(ALICE, "").await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = body_json(response).await;
        assert_eq!(body["error"], expected);
        let rendered = body.to_string();
        for forbidden in ["10.0.0.1", "6443", "apiserver", "4242", "4,242"] {
            assert!(!rendered.contains(forbidden), "{rendered}");
        }
    }
}

#[tokio::test]
async fn a_backend_that_outlives_its_budget_answers_unavailable() {
    let harness = harness(
        HarnessSpec::new(InventoryScript::Slow(std::time::Duration::from_millis(
            1_500,
        )))
        .timeout_ms(200),
    )
    .await;
    let response = harness.get(ALICE, "").await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(error_code(response).await, "sandbox_inventory_unavailable");
}

/// Exactly one backend list per request, and not one of the verbs the endpoint is
/// forbidden to use.
#[tokio::test]
async fn one_inventory_read_happens_and_no_other_backend_verb_is_touched() {
    let harness = harness_with(mixed_fleet()).await;
    harness.snapshot(ALICE, "").await;
    assert_eq!(harness.inventory_calls(), 1);
    assert_eq!(
        harness.forbidden_calls(),
        0,
        "no list_fleet, no per-runtime status, no logs, no exec"
    );
}

/// A live inventory is true for exactly one instant, and it is authorization
/// dependent: any shared cache would hand it to the wrong reader.
#[tokio::test]
async fn every_snapshot_is_no_store_and_reports_the_backends_own_instant() {
    let harness = harness_with(mixed_fleet()).await;
    let response = harness.get(ALICE, "").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("cache-control")
            .and_then(|value| value.to_str().ok()),
        Some("no-store")
    );
    let body = body_json(response).await;
    assert_eq!(body["observed_at"], "2026-07-31T12:00:00.000Z");
}

/// A verified identity the deployment does not admit is refused before the scope
/// question even arises.
#[tokio::test]
async fn an_unverifiable_token_is_unauthorized() {
    let harness = harness_with(mixed_fleet()).await;
    let response = harness.request(Some((0, "nobody")), "").await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(harness.inventory_calls(), 0);
}

/// The OpenSandbox adapter's honest nulls must survive the projection: `null`
/// restart count means "not knowable here", never "never restarted".
#[tokio::test]
async fn an_opensandbox_snapshot_reports_its_unsupported_fields_as_null() {
    let harness = harness(
        HarnessSpec::new(fleet::snapshot(vec![fleet::opensandbox(
            "sbx-1",
            Some(SESSION),
        )]))
        .opensandbox(),
    )
    .await;
    let snapshot = harness.snapshot(ALICE, "").await;
    assert_eq!(snapshot["backend"], "opensandbox");
    let item = &snapshot["items"][0];
    assert!(item["restart_count"].is_null());
    assert!(item["runtime_name"].is_null());
    assert!(item["runtime_uid"].is_null());
    assert!(item["deletion_timestamp"].is_null());
    assert_eq!(item["backend"], "opensandbox");
}

/// Every timing, status, and identity field the epic requires is on the wire.
#[tokio::test]
async fn a_snapshot_item_carries_every_required_field() {
    let harness = harness_with(vec![fleet::with_status(
        "mine",
        Some(SESSION),
        RuntimeInventoryStatus::Failed,
    )])
    .await;
    let snapshot = harness.snapshot(ALICE, "").await;
    let item = snapshot["items"][0].as_object().expect("an item object");
    for field in [
        "backend",
        "runtime_id",
        "runtime_name",
        "runtime_uid",
        "backend_location",
        "session_id",
        "managed",
        "metadata_state",
        "creator_id",
        "creator_login",
        "trigger_author_id",
        "trigger_author_login",
        "attribution_source",
        "repo_full_name",
        "installation_id",
        "trigger_issue",
        "status",
        "raw_status",
        "status_reason",
        "status_message",
        "created_at",
        "age_seconds",
        "max_lifetime_seconds",
        "expires_at",
        "remaining_seconds",
        "minimum_lifetime_seconds",
        "minimum_lifetime_remaining_seconds",
        "idle_grace_seconds",
        "last_pending_at",
        "idle_for_seconds",
        "restart_count",
        "last_transition_at",
        "deletion_timestamp",
        "warning_codes",
    ] {
        assert!(item.contains_key(field), "{field} is missing from the item");
    }
    assert_eq!(item["status"], "failed");
    assert_eq!(item["raw_status"], "failed");
}
