//! `/openapi.json` contract tests for the operations surface (milestone #22).
//!
//! Kept in their own file so `tests/openapi.rs` — already the largest suite in
//! the tree — does not keep growing with every new surface.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use fkst_control_plane::config::Config;
use fkst_control_plane::router::build_router;
use fkst_control_plane::state::{empty_self_router, AppState};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

/// The real router with no conditional feature enabled: the operations surface
/// is unconditional, so it must be documented in the minimal deployment too.
async fn spec() -> Value {
    let router = build_router(AppState {
        config: Config::default(),
        recovery: Default::default(),
        github_app: None,
        github_app_webhook_secret: None,
        reconciler: None,
        session_backend: None,
        storage: None,
        session_access: Default::default(),
        operations: Default::default(),
        log_bundle_cache: Default::default(),
        disposable_environments: Default::default(),
        self_router: empty_self_router(),
        chat: None,
        audit: Default::default(),
    })
    .expect("router builds");
    let response = router
        .oneshot(
            Request::get("/openapi.json")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("router responds");
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    serde_json::from_slice(&bytes).expect("body is valid JSON")
}

/// The activity endpoint's documented contract: its parameters, its tagged
/// record union, and every status a client must be able to handle.
#[tokio::test]
async fn activity_operation_documents_its_parameters_and_scope_statuses() {
    let spec = spec().await;
    let operation = &spec["paths"]["/api/v1/operations/activity"]["get"];
    assert_eq!(operation["operationId"], "operations_list_activity");
    assert_eq!(operation["tags"][0], "operations");

    let parameters: Vec<&str> = operation["parameters"]
        .as_array()
        .expect("activity parameters")
        .iter()
        .map(|parameter| parameter["name"].as_str().expect("parameter name"))
        .collect();
    for expected in [
        "from",
        "to",
        "record_kind",
        "actor_id",
        "actor_login",
        "operation_id",
        "method",
        "status_code",
        "status_class",
        "outcome",
        "session_id",
        "repo_full_name",
        "trigger_issue",
        "request_id",
        "cursor",
        "limit",
        "scope",
    ] {
        assert!(
            parameters.contains(&expected),
            "missing {expected}: {parameters:?}"
        );
    }
    for parameter in operation["parameters"].as_array().expect("parameters") {
        assert_eq!(parameter["in"], "query", "{parameter:?}");
    }

    // The canonical status set, including the three scope/session behaviours the
    // milestone's definition of done requires the document to carry.
    for status in ["200", "400", "401", "403", "404", "429", "502", "503"] {
        assert!(
            operation["responses"].get(status).is_some(),
            "activity must document {status}: {:?}",
            operation["responses"]
        );
    }
    assert_eq!(
        operation["responses"]["200"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/ActivityPage"
    );
    for (status, code) in [
        ("403", "operations_scope_forbidden"),
        ("404", "activity_session_not_found"),
        ("503", "audit_query_not_configured"),
    ] {
        let description = operation["responses"][status]["description"]
            .as_str()
            .unwrap_or_default();
        assert!(
            description.contains(code),
            "the {status} description must name the stable code {code}: {description}"
        );
    }
}

/// A lifecycle row must not be documented with fabricated HTTP fields — the
/// union is what keeps "no status" and "status unknown" distinguishable.
#[tokio::test]
async fn the_activity_record_union_is_tagged_and_carries_no_fake_http_fields() {
    let spec = spec().await;
    let schemas = &spec["components"]["schemas"];
    let union = serde_json::to_string(&schemas["ActivityItem"]).expect("union schema");
    assert!(union.contains("record_kind"), "{union}");
    assert!(union.contains("ApiRequestActivityItem"), "{union}");
    assert!(union.contains("SandboxLifecycleActivityItem"), "{union}");

    let lifecycle = &schemas["SandboxLifecycleActivityItem"]["properties"];
    for absent in ["method", "status_code", "route_template", "duration_ms"] {
        assert!(
            lifecycle.get(absent).is_none(),
            "a lifecycle row must not document {absent}"
        );
    }
    let api = &schemas["ApiRequestActivityItem"]["properties"];
    for expected in [
        "method",
        "status_code",
        "outcome",
        "delivery_state",
        "source",
    ] {
        assert!(
            api.get(expected).is_some(),
            "api row must document {expected}"
        );
    }

    // No total count anywhere in the page contract.
    let page = &schemas["ActivityPage"]["properties"];
    for forbidden in ["total", "total_count", "count"] {
        assert!(page.get(forbidden).is_none(), "{forbidden} must not exist");
    }
}
