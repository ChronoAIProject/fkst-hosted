//! The audit-coverage guard: every operation in the generated OpenAPI document
//! must carry exactly one EXPLICIT audit policy.
//!
//! This is the test that makes a new endpoint fail CI until someone decides
//! whether it is audited. It reads the live document (with both conditionally
//! mounted operations enabled) rather than a checked-in list, so it tracks the
//! code the way the spec itself does.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use fkst_control_plane::audit::request::policy::{
    declared_operation_ids, policy_for, ExclusionReason, OperationPolicy,
};
use fkst_control_plane::config::Config;
use fkst_control_plane::router::build_router;
use fkst_control_plane::state::{empty_self_router, AppState};
use http_body_util::BodyExt;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use tower::ServiceExt;

/// The eight OpenAPI operation keys a path item can carry.
const METHODS: [&str; 8] = [
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];

/// Build the router with EVERY conditionally mounted operation present, so the
/// document is the complete surface rather than one deployment's subset.
fn full_surface_router() -> axum::Router {
    let chat_config = fkst_control_plane::chat::config::from_vars(&[
        ("FKST_CHAT_ENABLED".to_string(), "true".to_string()),
        (
            "FKST_LLM_BASE_URL".to_string(),
            "https://llm.example/v1".to_string(),
        ),
        ("FKST_LLM_API_KEY".to_string(), "dummy-chat-key".to_string()),
        ("FKST_LLM_MODEL".to_string(), "dummy-model".to_string()),
    ])
    .expect("chat config parses")
    .expect("chat config is enabled");

    build_router(AppState {
        config: Config::default(),
        recovery: Default::default(),
        github_app: None,
        github_app_webhook_secret: Some(secrecy::SecretString::from(
            "audit-policy-test-secret".to_string(),
        )),
        reconciler: None,
        session_backend: None,
        storage: None,
        session_access: Default::default(),
        log_bundle_cache: Default::default(),
        disposable_environments: Default::default(),
        self_router: empty_self_router(),
        chat: Some(std::sync::Arc::new(
            fkst_control_plane::chat::ChatRuntime::from_config(chat_config),
        )),
        audit: Default::default(),
    })
    .expect("the router must build; a build failure here IS the coverage guard firing")
}

/// `operationId -> "METHOD path"` for every operation in the served document.
async fn live_operations() -> BTreeMap<String, String> {
    let response = full_surface_router()
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
    let spec: Value = serde_json::from_slice(&bytes).expect("valid JSON");

    let mut operations = BTreeMap::new();
    for (path, item) in spec["paths"].as_object().expect("paths object") {
        for method in METHODS {
            let Some(operation) = item.get(method) else {
                continue;
            };
            let operation_id = operation["operationId"]
                .as_str()
                .unwrap_or_else(|| {
                    panic!("{} {path} declares no operationId", method.to_uppercase())
                })
                .to_string();
            let previous = operations.insert(
                operation_id.clone(),
                format!("{} {path}", method.to_uppercase()),
            );
            assert!(
                previous.is_none(),
                "operationId {operation_id} is declared twice"
            );
        }
    }
    operations
}

/// The guard itself: adding a product endpoint without an audit policy fails
/// here (and, because the catalog is built at router assembly, at startup too).
#[tokio::test]
async fn every_documented_operation_has_exactly_one_explicit_policy() {
    let operations = live_operations().await;
    let mut unpoliced = Vec::new();
    for (operation_id, route) in &operations {
        if policy_for(operation_id).is_none() {
            unpoliced.push(format!("{operation_id} ({route})"));
        }
    }
    assert!(
        unpoliced.is_empty(),
        "these operations have no explicit audit policy; add an Audited or \
         Excluded entry to audit::request::policy::OPERATION_POLICIES: {unpoliced:?}"
    );
}

/// Pin the surface size so a silently REMOVED operation is noticed too — the
/// policy table would otherwise keep a stale entry forever.
#[tokio::test]
async fn the_audited_surface_is_the_expected_size_and_shape() {
    let operations = live_operations().await;
    assert_eq!(
        operations.len(),
        29,
        "the full surface changed; update this baseline deliberately: {:?}",
        operations.keys().collect::<Vec<_>>()
    );

    let excluded: BTreeSet<&str> = operations
        .keys()
        .filter(|id| policy_for(id) != Some(OperationPolicy::Audited))
        .map(String::as_str)
        .collect();
    assert_eq!(
        excluded,
        BTreeSet::from(["health", "readiness", "metrics"]),
        "only probe and scrape traffic may be excluded from the audit trail"
    );
    for probe in ["health", "readiness"] {
        assert_eq!(
            policy_for(probe),
            Some(OperationPolicy::Excluded(ExclusionReason::Probe))
        );
    }
    assert_eq!(
        policy_for("metrics"),
        Some(OperationPolicy::Excluded(ExclusionReason::Scrape))
    );
}

/// The two conditionally mounted operations are audited like any other product
/// call, and must be present in the full-surface document.
#[tokio::test]
async fn the_conditionally_mounted_operations_are_audited() {
    let operations = live_operations().await;
    for (operation_id, expected_route) in [
        ("github_app_webhook", "POST /api/v1/github/app/webhook"),
        ("chat_turn", "POST /api/v1/chat"),
    ] {
        assert_eq!(
            operations.get(operation_id).map(String::as_str),
            Some(expected_route),
            "{operation_id} must be in the full-surface document"
        );
        assert_eq!(policy_for(operation_id), Some(OperationPolicy::Audited));
    }
}

/// A stale table entry is as much a bug as a missing one: it hides the fact that
/// an operation disappeared.
#[tokio::test]
async fn the_policy_table_names_no_operation_the_document_lacks() {
    let operations = live_operations().await;
    let stale: Vec<&str> = declared_operation_ids()
        .filter(|id| !operations.contains_key(*id))
        .collect();
    assert!(
        stale.is_empty(),
        "these policy entries no longer match any documented operation: {stale:?}"
    );
}

/// A deployment with the conditional features off must still build — the catalog
/// tolerates an absent operation, it only rejects an undeclared one.
#[tokio::test]
async fn a_minimal_deployment_still_builds_its_catalog() {
    let router = build_router(AppState {
        config: Config::default(),
        recovery: Default::default(),
        github_app: None,
        github_app_webhook_secret: None,
        reconciler: None,
        session_backend: None,
        storage: None,
        session_access: Default::default(),
        log_bundle_cache: Default::default(),
        disposable_environments: Default::default(),
        self_router: empty_self_router(),
        chat: None,
        audit: Default::default(),
    })
    .expect("a minimal deployment must still build");
    let response = router
        .oneshot(
            Request::get("/health")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("router responds");
    assert_eq!(response.status(), StatusCode::OK);
}
