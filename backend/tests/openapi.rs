//! `GET /openapi.json` integration tests against the REAL `build_router`.
//!
//! Assert the v1 contract: the spec is generated from the LIVE routes (no static
//! file), it documents only the trimmed v1 surface (sessions + the webhook +
//! health/metrics), it does NOT document the removed legacy API (goals / GitHub
//! proxy / catalog / admin / repo-scaffold), it registers NO security scheme
//! (the API is open, read-only, network-isolated — there is no NyxID auth), it
//! EXCLUDES any `/internal/*` path, and the conditionally-mounted webhook tracks
//! config.

use axum::body::Body;
use axum::http::{Request, StatusCode};
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
        config: Config::default(),
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
        spec["components"]["securitySchemes"]
            .get("NyxIdIdentity")
            .is_none(),
        "NyxIdIdentity security scheme must be ABSENT (no application-level auth)"
    );
}

#[tokio::test]
async fn paths_are_the_trimmed_v1_surface() {
    let spec = fetch_spec(app(false)).await;
    let paths = &spec["paths"];

    // Present: the v1 surface — the named-environment REST API (collection +
    // item) plus the top-level system routes.
    for expected in [
        "/api/v1/users/me/environments",
        "/api/v1/users/me/environments/{name}",
        "/health",
        "/metrics",
    ] {
        assert!(
            paths.get(expected).is_some(),
            "spec must document {expected}; paths = {:?}",
            paths.as_object().map(|m| m.keys().collect::<Vec<_>>())
        );
    }

    // Absent: the removed legacy API + the removed REST session query/stop
    // (a session is controlled solely through its GitHub issue).
    for gone in [
        // `/api/v1/health` collapsed into the single top-level `/health`.
        "/api/v1/health",
        "/api/v1/sessions/{owner}/{repo}/{issue}",
        "/api/v1/sessions/{owner}/{repo}/{issue}/stop",
        // The flat per-user env store was replaced by named environments.
        "/api/v1/users/me/env",
        "/api/v1/users/me/secrets",
        "/api/v1/users/me/env/{key}",
        "/api/v1/users/me/secrets/{key}",
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
async fn components_include_the_named_environment_schemas_and_not_the_removed_ones() {
    let spec = fetch_spec(app(false)).await;
    let schemas = &spec["components"]["schemas"];
    for expected in [
        "ErrorEnvelope",
        // The named-environment DTOs (issue #338 §2.2).
        "EnvironmentSpec",
        "EnvironmentView",
        "EnvironmentList",
        "EnvironmentSummary",
        "InstallValidationError",
    ] {
        assert!(
            schemas.get(expected).is_some(),
            "spec must include {expected}; have {:?}",
            schemas.as_object().map(|m| m.keys().collect::<Vec<_>>())
        );
    }
    for gone in [
        // The REST session DTOs went with the endpoints.
        "SessionView",
        "StopResponse",
        // The flat user-store DTOs were replaced by the named-environment ones.
        "EnvPatchRequest",
        "UserEnvView",
        "PutEnvRequest",
        "EnvVariablesResponse",
        "PutSecretsRequest",
        "SecretKeysResponse",
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
async fn no_operation_requires_security_the_whole_surface_is_open() {
    let spec = fetch_spec(app(true)).await;
    let paths = &spec["paths"];
    // The environment endpoints authenticate via the per-request GitHub token (the
    // `GithubUser` extractor), NOT a documented security scheme — so they carry
    // no `security` and no `NyxIdIdentity`.
    for (route, verb) in [
        ("/api/v1/users/me/environments", "get"),
        ("/api/v1/users/me/environments/{name}", "put"),
        ("/api/v1/users/me/environments/{name}", "get"),
        ("/api/v1/users/me/environments/{name}", "delete"),
    ] {
        assert!(
            paths[route][verb].get("security").is_none(),
            "environment {verb} {route} must NOT carry a security scheme"
        );
    }
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

/// The document-level `tags` must be exactly the live set — no phantom tag left
/// behind by a removed surface (the audit found stale `goals`/`catalog`/etc.;
/// dropping the REST session endpoints left a phantom `sessions` tag until this
/// guard). Guards against declared-but-unused tags in the served contract.
#[tokio::test]
async fn document_tags_are_exactly_the_live_surface() {
    let spec = fetch_spec(app(true)).await;
    let mut tags: Vec<String> = spec["tags"]
        .as_array()
        .expect("tags array")
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    tags.sort();
    assert_eq!(
        tags,
        vec![
            "system".to_string(),
            "users".to_string(),
            "webhooks".to_string()
        ],
        "root `tags` must equal the live operation tags (no phantom/removed tags)"
    );
}
