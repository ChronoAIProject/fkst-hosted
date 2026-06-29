//! `GET /openapi.json` integration tests driven via `tower::ServiceExt::oneshot`
//! against the REAL `build_router(AppState)`.
//!
//! These assert the headline contract of the dynamic OpenAPI feature: the spec
//! is generated from the LIVE routes/types (no static file), it covers the public
//! surface, it EXCLUDES the fleet-only `/internal/v1/*` protocol, and it tracks
//! configuration — the conditionally-mounted GitHub App webhook appears in the
//! spec only when a webhook secret is configured.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use fkst_control_plane::auth::AuthMode;
use fkst_control_plane::authz::Authorizer;
use fkst_control_plane::config::Config;
use fkst_control_plane::engine::EngineConfig;
use fkst_control_plane::goals::GoalIssueStore;
use fkst_control_plane::router::build_router;
use fkst_control_plane::sessions::{SessionRepo, SessionService};
use fkst_control_plane::state::AppState;
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

mod support;

/// Build the real router. `webhook_secret` toggles the conditionally-mounted
/// GitHub App webhook so a test can assert the spec reflects live configuration.
fn app(webhook_secret: bool) -> axum::Router {
    let github_app_webhook_secret = webhook_secret
        .then(|| secrecy::SecretString::new("dummy-webhook-secret".to_string().into()));
    build_router(AppState {
        config: Config::default(),
        sessions: SessionService::new(SessionRepo::new(), EngineConfig::default()),
        auth_mode: AuthMode::Disabled,
        authz: Authorizer::disabled(),
        github_app: None,
        github_app_webhook_secret,
        goals: GoalIssueStore::new(None),
        vault: support::test_vault(),
        ornn: None,
    })
    .expect("router builds")
}

/// Fetch and parse `/openapi.json`, asserting the transport-level contract.
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
    let content_type = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(
        content_type.starts_with("application/json"),
        "content-type must be application/json, got {content_type:?}"
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
    // The NyxID identity security scheme is documented in components.
    assert!(
        spec["components"]["securitySchemes"]["NyxIdIdentity"].is_object(),
        "NyxIdIdentity security scheme must be present"
    );
}

#[tokio::test]
async fn paths_cover_the_public_surface() {
    let spec = fetch_spec(app(false)).await;
    let paths = &spec["paths"];

    // A representative slice of every tag group, plus the two health paths and
    // metrics. Each must be present as a live path item.
    for expected in [
        "/api/v1/sessions/{id}",
        "/api/v1/sessions/{id}/stop",
        "/api/v1/goals",
        "/api/v1/goals/{id}",
        "/api/v1/goals/{id}/trigger",
        "/api/v1/goals/submit",
        "/api/v1/catalog/skills",
        "/api/v1/catalog/skills/{name}/versions",
        "/api/v1/github/accounts",
        "/api/v1/github/issues",
        "/api/v1/github/repos/{owner}/{repo}/issues",
        "/api/v1/github/repos/{owner}/{repo}/issues/{number}/comments",
        "/api/v1/repos/{owner}/{name}/fkst-setup",
        "/api/v1/admin/state",
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

    // `/api/v1/goals` carries BOTH list (GET) and create (POST).
    assert!(paths["/api/v1/goals"]["get"].is_object());
    assert!(paths["/api/v1/goals"]["post"].is_object());
    // `/api/v1/goals/{id}` carries GET, PATCH, and DELETE.
    assert!(paths["/api/v1/goals/{id}"]["get"].is_object());
    assert!(paths["/api/v1/goals/{id}"]["patch"].is_object());
    assert!(paths["/api/v1/goals/{id}"]["delete"].is_object());
}

#[tokio::test]
async fn components_include_key_schemas_from_all_crates() {
    let spec = fetch_spec(app(false)).await;
    let schemas = &spec["components"]["schemas"];

    for expected in [
        // Control-plane response/request DTOs.
        "GoalView",
        "SessionView",
        "TriggerRequest",
        "SubmitSessionRequest",
        "CatalogResponse",
        "AdminStateView",
        "AdminSessionView",
        "SetupRepoRef",
        "ErrorEnvelope",
        "RepoRefView",
        // Control-plane domain enum.
        "GoalStatus",
        // fkst-shared wire types (behind the `schema` feature).
        "SessionStatus",
        "TerminalCause",
        "OrnnSkillPin",
        "SearchRow",
        "VersionRow",
        "EnvKind",
    ] {
        assert!(
            schemas.get(expected).is_some(),
            "spec components must include {expected}; have {:?}",
            schemas.as_object().map(|m| m.keys().collect::<Vec<_>>())
        );
    }
}

#[tokio::test]
async fn protected_paths_require_identity_public_paths_do_not() {
    let spec = fetch_spec(app(false)).await;
    let paths = &spec["paths"];

    // A protected /api/v1 operation references the NyxIdIdentity scheme.
    let protected = &paths["/api/v1/sessions/{id}"]["get"]["security"];
    assert!(
        protected
            .as_array()
            .map(|reqs| reqs.iter().any(|r| r.get("NyxIdIdentity").is_some()))
            .unwrap_or(false),
        "GET /sessions/{{id}} must require NyxIdIdentity, got {protected:?}"
    );

    // Public operations carry no security requirement.
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
    // The /internal/v1/* routes are mounted AFTER build_router (in main.rs) and
    // are out of scope for the public contract: they must not appear.
    let spec = fetch_spec(app(true)).await;
    let paths = spec["paths"].as_object().expect("paths object");
    for key in paths.keys() {
        assert!(
            !key.starts_with("/internal/"),
            "internal route {key} leaked into the public spec"
        );
    }
}

#[tokio::test]
async fn webhook_path_tracks_configuration() {
    // No secret configured: the webhook route is not mounted, so it is absent
    // from the spec (the spec reflects what is actually served).
    let without = fetch_spec(app(false)).await;
    assert!(
        without["paths"].get("/api/v1/github/app/webhook").is_none(),
        "webhook must be absent when no secret is configured"
    );

    // Secret configured: the route is mounted, so it appears.
    let with = fetch_spec(app(true)).await;
    assert!(
        with["paths"]["/api/v1/github/app/webhook"]["post"].is_object(),
        "webhook must be documented when a secret is configured"
    );
}
