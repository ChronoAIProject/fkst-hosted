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

/// The sandbox endpoint's documented contract: its closed filter vocabulary, its
/// item schema, and every status a client must be able to handle.
#[tokio::test]
async fn sandbox_operation_documents_its_filters_and_scope_statuses() {
    let spec = spec().await;
    let operation = &spec["paths"]["/api/v1/operations/sandboxes"]["get"];
    assert_eq!(operation["operationId"], "operations_list_sandboxes");
    assert_eq!(operation["tags"][0], "operations");

    let parameters: Vec<&str> = operation["parameters"]
        .as_array()
        .expect("sandbox parameters")
        .iter()
        .map(|parameter| parameter["name"].as_str().expect("parameter name"))
        .collect();
    for expected in [
        "scope",
        "status",
        "backend",
        "creator_id",
        "creator_login",
        "repo_full_name",
        "session_id",
        "trigger_issue",
        "attribution_source",
    ] {
        assert!(
            parameters.contains(&expected),
            "missing {expected}: {parameters:?}"
        );
    }
    // The filter vocabulary is CLOSED: a parameter nobody reviewed would be a
    // silent widening of the query surface.
    assert_eq!(parameters.len(), 9, "{parameters:?}");
    for parameter in operation["parameters"].as_array().expect("parameters") {
        assert_eq!(parameter["in"], "query", "{parameter:?}");
    }

    for status in ["200", "400", "401", "403", "404", "503"] {
        assert!(
            operation["responses"].get(status).is_some(),
            "sandboxes must document {status}: {:?}",
            operation["responses"]
        );
    }
    assert_eq!(
        operation["responses"]["200"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/SandboxInventoryResponse"
    );
    for (status, codes) in [
        ("403", vec!["operations_scope_forbidden"]),
        ("404", vec!["sandbox_not_found"]),
        (
            "503",
            vec![
                "session_visibility_unavailable",
                "sandbox_inventory_disabled",
                "sandbox_inventory_unavailable",
                "sandbox_inventory_too_large",
            ],
        ),
    ] {
        let description = operation["responses"][status]["description"]
            .as_str()
            .unwrap_or_default();
        for code in codes {
            assert!(
                description.contains(code),
                "the {status} description must name the stable code {code}: {description}"
            );
        }
    }
}

/// The snapshot schema must document every identity, timing, and status field the
/// epic requires — including the nullable ones a backend may not support.
#[tokio::test]
async fn the_sandbox_item_schema_documents_every_required_field() {
    let spec = spec().await;
    let schemas = &spec["components"]["schemas"];
    let item = &schemas["SandboxItem"]["properties"];
    for expected in [
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
        assert!(
            item.get(expected).is_some(),
            "the sandbox item must document {expected}"
        );
    }

    let response = &schemas["SandboxInventoryResponse"]["properties"];
    for expected in [
        "observed_at",
        "backend",
        "effective_scope",
        "can_view_all",
        "item_count",
        "filters_applied",
        "items",
        "warning_codes",
    ] {
        assert!(
            response.get(expected).is_some(),
            "the snapshot must document {expected}"
        );
    }
    // No fleet total, and no hidden-row statistic, anywhere in the contract.
    for forbidden in ["total", "total_count", "hidden_count", "fleet_size"] {
        assert!(
            response.get(forbidden).is_none(),
            "{forbidden} must not exist"
        );
    }

    // The scope vocabulary is closed and distinct from the activity endpoint's.
    let scope = serde_json::to_string(&schemas["SandboxEffectiveScope"]).expect("scope schema");
    assert!(scope.contains("accessible"), "{scope}");
    assert!(scope.contains("all"), "{scope}");
    assert!(!scope.contains("mine"), "{scope}");
}
