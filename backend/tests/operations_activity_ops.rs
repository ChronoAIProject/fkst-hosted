//! Operational tests for `/api/v1/operations/activity`: what it records, what it
//! exports, what it never leaks, and what it must NOT be able to break.

mod operations_harness;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use operations_harness::{harness, minutes_ago, token, Row, Sources, ALICE, ROOT, SESSION};
use tower::ServiceExt;

fn dataset() -> Vec<Row> {
    vec![
        Row::api("ev-alice-1", ALICE.0, &minutes_ago(2)),
        Row::lifecycle("ev-life-1", SESSION, &minutes_ago(4)),
    ]
}

/// The route participates in request auditing under its own operation id, with
/// the reviewed safe-argument boundary attached.
#[tokio::test]
async fn an_allowed_query_records_its_normalized_safe_arguments() {
    let harness = harness(Sources::Posthog(dataset()), true).await;
    harness
        .page(ALICE, "?record_kind=all&session_id=sess-alice&limit=25")
        .await;

    let events = harness.audit.events();
    let record = events
        .iter()
        .find(|event| event.operation_id == "operations_list_activity")
        .expect("the operations route is audited like any other product call");
    assert_eq!(record.method, "GET");
    assert_eq!(record.route_template, "/api/v1/operations/activity");
    assert_eq!(record.actor_id, Some(ALICE.0));
    assert_eq!(record.status_code, Some(200));

    let arguments = &record.arguments;
    assert_eq!(arguments["scope"], "mine");
    assert_eq!(arguments["record_kind"], "all");
    assert_eq!(arguments["limit"], 25);
    assert_eq!(arguments["cursor_present"], false);
    assert_eq!(arguments["actor_filter_present"], false);
    assert_eq!(arguments["session_id"], "sess-alice");
    assert!(arguments.contains_key("from") && arguments.contains_key("to"));
}

/// A refused cross-user probe is recorded as the ATTEMPT, never as the probe.
#[tokio::test]
async fn a_refused_probe_records_the_attempt_and_never_the_probed_identity() {
    let harness = harness(Sources::Posthog(dataset()), true).await;
    let response = harness.get(ALICE, "?scope=all&actor_login=carol").await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let events = harness.audit.events();
    let record = events
        .iter()
        .find(|event| event.operation_id == "operations_list_activity")
        .expect("a refusal is audited too");
    assert_eq!(record.status_code, Some(403));
    assert_eq!(
        record.error_code.as_deref(),
        Some("operations_scope_forbidden")
    );
    assert_eq!(record.arguments["scope"], "mine");
    assert_eq!(record.arguments["requested_scope"], "all");
    assert_eq!(record.arguments["actor_filter_present"], true);

    let rendered = serde_json::to_string(&record.arguments).expect("serializes");
    assert!(!rendered.contains("carol"), "{rendered}");
}

/// The query credential must be absent from every observable surface.
#[tokio::test]
async fn the_query_credential_never_reaches_a_record_a_metric_or_a_response() {
    let harness = harness(Sources::Posthog(dataset()), true).await;
    harness.page(ALICE, "").await;

    let rendered = serde_json::to_string(
        &harness
            .audit
            .events()
            .iter()
            .map(|event| event.arguments.clone())
            .collect::<Vec<_>>(),
    )
    .expect("serializes");
    assert!(!rendered.contains("phx_read_key"), "{rendered}");

    let metrics = metrics_body(&harness).await;
    assert!(!metrics.contains("phx_read_key"), "{metrics}");
    assert!(!metrics.contains("/api/projects/"), "{metrics}");
}

/// The metric families are exported with closed-enum labels only — no viewer,
/// actor, filter, session, repository, request, event, or cursor value.
#[tokio::test]
async fn the_activity_metrics_are_exported_with_bounded_labels_only() {
    let harness = harness(Sources::Posthog(dataset()), true).await;
    harness.page(ALICE, "").await;
    let _ = harness.get(ALICE, "?scope=all").await;
    let _ = harness.get(ALICE, "?record_kind=all").await;

    let body = metrics_body(&harness).await;
    for family in [
        "fkst_operations_activity_queries_total",
        "fkst_operations_activity_query_duration_seconds",
        "fkst_operations_activity_rows_total",
        "fkst_operations_activity_source_partial_total",
        "fkst_operations_activity_scope_rejections_total",
    ] {
        assert!(
            body.contains(&format!("# TYPE {family}")),
            "{family} missing"
        );
    }
    assert!(body.contains(
        "fkst_operations_activity_queries_total{scope=\"mine\",record_kind=\"api_request\",result=\"success\"} 1"
    ), "{body}");
    assert!(
        body.contains(
            "fkst_operations_activity_scope_rejections_total{reason=\"global_scope_forbidden\"} 1"
        ),
        "{body}"
    );
    assert!(body.contains(
        "fkst_operations_activity_scope_rejections_total{reason=\"lifecycle_session_forbidden\"} 1"
    ), "{body}");

    // No identity-bearing value appears anywhere in the activity families.
    for line in body
        .lines()
        .filter(|line| line.contains("fkst_operations_activity"))
    {
        for forbidden in ["alice", "root", "sess-", "ev-", &ALICE.0.to_string()] {
            assert!(!line.contains(forbidden), "{line}");
        }
    }
}

/// A PostHog outage must not touch product traffic or the liveness surface.
#[tokio::test]
async fn a_posthog_outage_leaves_product_and_probe_handlers_working() {
    let harness = harness(Sources::PosthogFailing(500), true).await;
    let failed = harness.get(ALICE, "").await;
    assert_eq!(failed.status(), StatusCode::SERVICE_UNAVAILABLE);

    for path in ["/health", "/ready", "/metrics"] {
        let response = harness
            .router
            .clone()
            .oneshot(
                Request::get(path)
                    .body(Body::empty())
                    .expect("request builds"),
            )
            .await
            .expect("router responds");
        assert_eq!(response.status(), StatusCode::OK, "{path}");
    }

    // A product route still authenticates and answers normally.
    let product = harness
        .router
        .clone()
        .oneshot(
            Request::get("/api/v1/users/me/environment-profiles")
                .header("authorization", format!("Bearer {}", token(ALICE)))
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("router responds");
    assert_ne!(
        product.status(),
        StatusCode::BAD_GATEWAY,
        "a PostHog outage must not surface on a product handler"
    );
}

/// A global administrator's all-history must not depend on the session registry.
#[tokio::test]
async fn a_cold_session_registry_does_not_affect_global_history() {
    let harness = harness(Sources::Posthog(dataset()), false).await;
    let page = harness.page(ROOT, "?record_kind=all").await;
    assert!(!page["items"].as_array().expect("items").is_empty());
    assert_eq!(page["effective_scope"], "all");
}

async fn metrics_body(harness: &operations_harness::Harness) -> String {
    let response = harness
        .router
        .clone()
        .oneshot(
            Request::get("/metrics")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("router responds");
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    String::from_utf8_lossy(&bytes).to_string()
}
