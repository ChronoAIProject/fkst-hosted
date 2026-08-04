//! Drives a REAL request through every audited route and checks what was
//! recorded — rather than inspecting a string map.
//!
//! The coverage guard in `audit_operation_policy.rs` proves the policy TABLE is
//! complete. That is a static claim about a `const` array: it holds even if the
//! middleware were mounted on half the router, if an operation's route were
//! shadowed by an earlier matcher, or if a handler returned before the layer
//! that records. The issue calls that out explicitly — "drive representative real
//! requests through every audited route, not just inspect a string map" — and
//! this suite is the dynamic half.
//!
//! ## How a request is chosen
//!
//! One request per audited operation, built from the OPERATION'S OWN path
//! template in the generated document: the method the document declares, path
//! parameters filled with benign literals, and an empty JSON body where a body is
//! expected. Almost every one of them is refused — unauthenticated, unsigned, or
//! semantically invalid — and that is the point: a refusal is a terminal outcome
//! the audit trail must still contain, and it exercises the extractor and
//! middleware path without needing a credential, a cluster, or a GitHub App.
//!
//! Any outbound GitHub call the surviving handlers make lands on a local mock, so
//! the suite never reaches the network.
//!
//! ## What it asserts
//!
//! The set of `operation_id`s the sink OBSERVED equals the set the policy table
//! declares audited. A missing observation means a route that cannot record; an
//! unexpected observation means a request landed somewhere other than the
//! operation whose template produced it.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use fkst_control_plane::audit::request::policy::{policy_for, OperationPolicy};
use fkst_control_plane::audit::sink::RecordingSink;
use fkst_control_plane::audit::AuditHandle;
use fkst_control_plane::config::Config;
use fkst_control_plane::router::build_router;
use fkst_control_plane::state::{empty_self_router, AppState};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;
use wiremock::matchers::method as mock_method;
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The eight OpenAPI operation keys a path item can carry.
const METHODS: [&str; 8] = [
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];

/// How long one driven request may take before the suite calls it a hang.
///
/// Generous relative to a refusal (microseconds) and short enough that a route
/// which blocks on an unreachable dependency fails loudly instead of stalling
/// the run.
const PER_REQUEST_BUDGET: Duration = Duration::from_secs(20);

/// Benign substitutions for the path parameters the surface declares.
///
/// Values are chosen to be *plausible* so a route that validates its path before
/// authenticating still reaches its own handler rather than a shared 400 — which
/// would record the right operation but exercise less of it.
fn substitute(template: &str) -> String {
    let mut path = template.to_string();
    for (parameter, value) in [
        ("{owner}", "acme"),
        ("{repo}", "site"),
        ("{name}", "site"),
        ("{session_id}", "sess-acceptance"),
        ("{issue_number}", "7"),
        ("{issue}", "7"),
        ("{number}", "7"),
        ("{profile}", "node-20"),
        ("{id}", "7"),
    ] {
        path = path.replace(parameter, value);
    }
    // Anything left is filled with a safe literal rather than left as a brace,
    // which axum would never match.
    while let Some(open) = path.find('{') {
        let Some(close) = path[open..].find('}') else {
            break;
        };
        path.replace_range(open..open + close + 1, "acceptance");
    }
    path
}

/// The whole audited surface, driven once each.
#[tokio::test]
async fn every_audited_operation_records_a_real_driven_request() {
    let github = MockServer::start().await;
    // Any GitHub call a handler makes gets a harmless refusal from a LOCAL
    // server: the suite must never depend on, or reach, the network.
    for verb in ["GET", "POST", "PATCH", "PUT", "DELETE"] {
        Mock::given(mock_method(verb))
            .respond_with(ResponseTemplate::new(401).set_body_string(r#"{"message":"no"}"#))
            .mount(&github)
            .await;
    }

    let (router, sink) = full_surface_router(&github).await;
    let surface = live_operations(&router).await;

    let audited: BTreeSet<&str> = surface
        .iter()
        .filter(|(id, _)| policy_for(id) == Some(OperationPolicy::Audited))
        .map(|(id, _)| id.as_str())
        .collect();
    assert!(
        audited.len() > 20,
        "the audited surface collapsed to {} operations; this suite would prove \
         almost nothing",
        audited.len()
    );

    let mut refused_to_answer = Vec::new();
    for operation_id in &audited {
        let route = surface
            .get(*operation_id)
            .expect("the operation came from this map");
        let (verb, template) = route
            .split_once(' ')
            .unwrap_or_else(|| panic!("{operation_id} has no method in {route:?}"));
        let method = Method::from_bytes(verb.as_bytes()).expect("a documented HTTP method");
        let path = substitute(template);

        let mut builder = Request::builder().method(method.clone()).uri(&path);
        let body = if matches!(method, Method::POST | Method::PUT | Method::PATCH) {
            builder = builder.header("content-type", "application/json");
            Body::from("{}")
        } else {
            Body::empty()
        };
        let request = builder.body(body).expect("request builds");

        match tokio::time::timeout(PER_REQUEST_BUDGET, drive(&router, request)).await {
            Ok(status) => {
                // Nothing here should reach a panic-shaped 500 from an absent
                // dependency; a fail-closed refusal is the expected answer.
                assert_ne!(
                    status,
                    StatusCode::NOT_IMPLEMENTED,
                    "{operation_id} ({verb} {path}) is not implemented at all"
                );
            }
            Err(_) => refused_to_answer.push(format!("{operation_id} ({verb} {path})")),
        }
    }
    assert!(
        refused_to_answer.is_empty(),
        "these audited routes did not answer within the per-request budget: \
         {refused_to_answer:#?}"
    );

    let observed: BTreeSet<String> = sink
        .events()
        .into_iter()
        .map(|event| event.operation_id)
        .collect();
    let observed: BTreeSet<&str> = observed.iter().map(String::as_str).collect();

    let never_recorded: Vec<&&str> = audited.difference(&observed).collect();
    assert!(
        never_recorded.is_empty(),
        "these audited operations were driven but produced NO audit record; the \
         middleware cannot see them: {never_recorded:#?}"
    );
    let unexpected: Vec<&&str> = observed.difference(&audited).collect();
    assert!(
        unexpected.is_empty(),
        "driving the audited surface also recorded operations nobody asked for; a \
         request landed on the wrong route: {unexpected:#?}"
    );
}

/// Issue one request and drain its body, so a streaming response is terminal by
/// the time the sink is read.
async fn drive(router: &axum::Router, request: Request<Body>) -> StatusCode {
    let response = router
        .clone()
        .oneshot(request)
        .await
        .expect("the router responds");
    let status = response.status();
    let _ = response.into_body().collect().await;
    status
}

/// Build the router with EVERY conditionally mounted operation present, plus a
/// recording sink so the driven requests can be read back.
async fn full_surface_router(github: &MockServer) -> (axum::Router, RecordingSink) {
    let chat_config = fkst_control_plane::chat::config::from_vars(&[
        ("FKST_CHAT_ENABLED".to_string(), "true".to_string()),
        (
            "FKST_LLM_BASE_URL".to_string(),
            "https://llm.invalid/v1".to_string(),
        ),
        ("FKST_LLM_API_KEY".to_string(), "dummy-chat-key".to_string()),
        ("FKST_LLM_MODEL".to_string(), "dummy-model".to_string()),
    ])
    .expect("chat config parses")
    .expect("chat config is enabled");

    let config = Config {
        github_api_base_url: github.uri(),
        ..Config::default()
    };
    let (audit, sink) = AuditHandle::recording();
    let router = build_router(AppState {
        config,
        recovery: Default::default(),
        github_app: None,
        github_app_webhook_secret: Some(secrecy::SecretString::from(
            "route-drive-secret".to_string(),
        )),
        reconciler: None,
        session_backend: None,
        storage: None,
        session_access: Default::default(),
        operations: Default::default(),
        log_bundle_cache: Default::default(),
        disposable_environments: Default::default(),
        self_router: empty_self_router(),
        chat: Some(std::sync::Arc::new(
            fkst_control_plane::chat::ChatRuntime::from_config(chat_config),
        )),
        audit,
    })
    .expect("the router builds");
    (router, sink)
}

/// `operationId -> "METHOD path"` for every operation in the served document.
async fn live_operations(router: &axum::Router) -> BTreeMap<String, String> {
    let response = router
        .clone()
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
                .unwrap_or_else(|| panic!("{} {path} declares no operationId", method))
                .to_string();
            operations.insert(operation_id, format!("{} {path}", method.to_uppercase()));
        }
    }
    operations
}
