//! `GET /openapi.json` integration tests against the REAL `build_router`.
//!
//! Assert the v1 contract: the spec is generated from the LIVE routes (no static
//! file), it documents only the trimmed v1 surface (sessions + nyxid-connect +
//! the webhook + health/metrics), it does NOT document the removed legacy API
//! (goals / GitHub proxy / catalog / admin / repo-scaffold), it EXCLUDES any
//! `/internal/*` path, and the conditionally-mounted webhook tracks config.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use fkst_control_plane::auth::AuthMode;
use fkst_control_plane::authz::Authorizer;
use fkst_control_plane::config::Config;
use fkst_control_plane::router::build_router;
use fkst_control_plane::state::AppState;
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

/// Build the real router. `webhook_secret` toggles the conditionally-mounted
/// GitHub App webhook so a test can assert the spec reflects live configuration.
fn app(webhook_secret: bool) -> axum::Router {
    let github_app_webhook_secret = webhook_secret
        .then(|| secrecy::SecretString::new("dummy-webhook-secret".to_string().into()));
    build_router(AppState {
        binding_store: fkst_control_plane::nyxid_connect::BrokerBindingStore::new(),
        config: Config::default(),
        auth_mode: AuthMode::Disabled,
        authz: Authorizer::disabled(),
        github_app: None,
        github_app_webhook_secret,
    })
    .expect("router builds")
}

async fn fetch_spec(router: axum::Router) -> Value {
    let response = router
        .oneshot(
            Request::get("/openapi.json")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("router responds");
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "/openapi.json must be 200"
    );
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    serde_json::from_slice(&bytes).expect("body is valid JSON")
}

#[tokio::test]
async fn serves_a_valid_openapi3_document_with_metadata() {
    let spec = fetch_spec(app(false)).await;
    let version = spec["openapi"].as_str().expect("openapi version string");
    assert!(
        version.starts_with("3."),
        "must be OpenAPI 3.x, got {version}"
    );
    assert_eq!(spec["info"]["title"], "fkst-hosted control plane API");
    assert_eq!(spec["info"]["version"], env!("CARGO_PKG_VERSION"));
    assert!(
        spec["components"]["securitySchemes"]["NyxIdIdentity"].is_object(),
        "NyxIdIdentity security scheme must be present"
    );
}

#[tokio::test]
async fn paths_are_the_trimmed_v1_surface() {
    let spec = fetch_spec(app(false)).await;
    let paths = &spec["paths"];

    // Present: the v1 surface.
    for expected in [
        "/api/v1/sessions/{owner}/{repo}/{issue}",
        "/api/v1/sessions/{owner}/{repo}/{issue}/stop",
        "/health",
        "/api/v1/health",
        "/metrics",
    ] {
        assert!(
            paths.get(expected).is_some(),
            "spec must document {expected}; paths = {:?}",
            paths.as_object().map(|m| m.keys().collect::<Vec<_>>())
        );
    }

    // Absent: the removed legacy API.
    for gone in [
        "/api/v1/goals",
        "/api/v1/goals/{id}",
        "/api/v1/goals/submit",
        "/api/v1/catalog/skills",
        "/api/v1/github/accounts",
        "/api/v1/admin/state",
        "/api/v1/repos/{owner}/{name}/fkst-setup",
    ] {
        assert!(
            paths.get(gone).is_none(),
            "removed path {gone} must NOT be in the spec"
        );
    }
}

#[tokio::test]
async fn components_include_the_session_schemas_and_not_the_removed_ones() {
    let spec = fetch_spec(app(false)).await;
    let schemas = &spec["components"]["schemas"];
    for expected in ["SessionView", "StopResponse", "ErrorEnvelope"] {
        assert!(
            schemas.get(expected).is_some(),
            "spec must include {expected}; have {:?}",
            schemas.as_object().map(|m| m.keys().collect::<Vec<_>>())
        );
    }
    for gone in [
        "GoalView",
        "CatalogResponse",
        "AdminStateView",
        "SetupRepoRef",
    ] {
        assert!(
            schemas.get(gone).is_none(),
            "removed schema {gone} must be absent"
        );
    }
}

#[tokio::test]
async fn protected_paths_require_identity_public_paths_do_not() {
    let spec = fetch_spec(app(false)).await;
    let paths = &spec["paths"];
    let protected = &paths["/api/v1/sessions/{owner}/{repo}/{issue}"]["get"]["security"];
    assert!(
        protected
            .as_array()
            .map(|reqs| reqs.iter().any(|r| r.get("NyxIdIdentity").is_some()))
            .unwrap_or(false),
        "GET session must require NyxIdIdentity, got {protected:?}"
    );
    assert!(
        paths["/health"]["get"].get("security").is_none(),
        "/health must not require security"
    );
    assert!(
        paths["/metrics"]["get"].get("security").is_none(),
        "/metrics must not require security"
    );
}

#[tokio::test]
async fn internal_worker_protocol_is_never_in_the_spec() {
    let spec = fetch_spec(app(true)).await;
    let paths = spec["paths"].as_object().expect("paths object");
    for key in paths.keys() {
        assert!(
            !key.starts_with("/internal/"),
            "internal route {key} leaked"
        );
    }
}

#[tokio::test]
async fn webhook_path_tracks_configuration() {
    let without = fetch_spec(app(false)).await;
    assert!(
        without["paths"].get("/api/v1/github/app/webhook").is_none(),
        "webhook must be absent when no secret is configured"
    );
    let with = fetch_spec(app(true)).await;
    assert!(
        with["paths"]["/api/v1/github/app/webhook"]["post"].is_object(),
        "webhook must be documented when a secret is configured"
    );
}
