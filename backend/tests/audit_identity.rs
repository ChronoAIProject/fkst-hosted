//! Router-level tests for how the REAL router attributes and classifies the
//! answers that identity and authorization produce.
//!
//! Every case here is a *policy decision*: the caller could not be verified, was
//! verified and refused, or was verified and admitted. The epic's `rejected`
//! outcome is what makes those queryable as a class, so each surface — JSON
//! envelope, browser HTML page, signature-verified webhook — has to reach the
//! same classification for the same decision. Where they disagree, a security
//! query silently under-reports, which is why these are asserted end to end
//! against `build_router` rather than against the tagging helpers.
//!
//! The request-lifecycle cases (exclusions, route resolution, request ids) live
//! in the sibling `audit_middleware` suite.

mod audit_router;

use audit_router::{sign, Harness, BOGUS_SIGNATURE};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use fkst_control_plane::audit::AuditOutcome;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// An extractor rejection never reaches a handler, and must still be recorded.
#[tokio::test]
async fn a_missing_bearer_token_is_recorded_as_a_rejection() {
    let harness = Harness::new();
    let response = harness.get("/api/v1/users/me/environment-profiles").await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let event = harness.only_event();
    assert_eq!(event.operation_id, "list_user_environment_profiles");
    assert_eq!(
        event.route_template,
        "/api/v1/users/me/environment-profiles"
    );
    assert_eq!(event.outcome, AuditOutcome::Rejected);
    assert_eq!(event.error_code.as_deref(), Some("unauthorized"));
    assert_eq!(event.actor_id, None);
}

/// A token GitHub rejects is an authentication failure that never reaches a
/// handler, and it must never leave an actor — or the token — behind.
#[tokio::test]
async fn an_invalid_bearer_token_is_recorded_as_a_rejection() {
    let github = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&github)
        .await;
    let harness = Harness::with_github(&github.uri(), "octocat");

    let response = harness
        .call(
            Request::get("/api/v1/users/me/environment-profiles")
                .header("authorization", "Bearer ghu_canary-token")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let event = harness.only_event();
    assert_eq!(event.operation_id, "list_user_environment_profiles");
    assert_eq!(event.outcome, AuditOutcome::Rejected);
    assert_eq!(event.error_code.as_deref(), Some("unauthorized"));
    assert_eq!(event.actor_id, None, "an unverified token names nobody");
    assert!(
        !format!("{event:#?}").contains("ghu_canary-token"),
        "the bearer token must never reach a record"
    );
}

/// A token GitHub accepts but the deployment's access policy does not admit: the
/// record must say `rejected`/`forbidden`, distinguishing "we could not verify
/// you" from "we verified you and said no".
#[tokio::test]
async fn a_base_access_rejection_is_recorded_as_a_forbidden_rejection() {
    let github = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"login": "mallory", "id": 4242})),
        )
        .mount(&github)
        .await;
    // An allowlist that names someone else, so the VERIFIED identity is denied.
    let harness = Harness::with_github(&github.uri(), "octocat");

    let response = harness
        .call(
            Request::get("/api/v1/users/me/environment-profiles")
                .header("authorization", "Bearer ghu_valid-token")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let event = harness.only_event();
    assert_eq!(event.operation_id, "list_user_environment_profiles");
    assert_eq!(event.outcome, AuditOutcome::Rejected);
    assert_eq!(event.error_code.as_deref(), Some("forbidden"));
    // The extractor publishes the identity only AFTER the base-access gate, so a
    // caller this deployment does not admit never becomes an actor at all. Scope
    // denials (a caller the deployment DOES admit, refused one operation) are the
    // case that keeps its verified actor.
    assert_eq!(
        event.actor_id, None,
        "a caller the deployment does not admit is never attributed"
    );
}

/// A forged delivery is an identity rejection, and the sender its unverified
/// body claims must never become the record's actor.
#[tokio::test]
async fn a_webhook_signature_rejection_is_recorded_without_the_claimed_sender() {
    let harness = Harness::new();
    let body = r#"{"action":"opened","sender":{"login":"mallory-canary","id":9999}}"#;
    let response = harness
        .call(
            Request::post("/api/v1/github/app/webhook")
                .header("content-type", "application/json")
                .header("x-github-event", "issues")
                .header("x-hub-signature-256", BOGUS_SIGNATURE)
                .body(Body::from(body))
                .expect("request builds"),
        )
        .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let event = harness.only_event();
    assert_eq!(event.operation_id, "github_app_webhook");
    assert_eq!(event.outcome, AuditOutcome::Rejected);
    assert_eq!(
        event.error_code.as_deref(),
        Some("webhook_signature_invalid")
    );
    assert_eq!(event.actor_id, None);
    assert!(!format!("{event:#?}").contains("mallory-canary"));
}

/// A verified delivery publishes the two handles that make it findable again:
/// the id on the App's *Recent Deliveries* page and the installation it belongs
/// to.
#[tokio::test]
async fn a_verified_webhook_records_its_delivery_correlation() {
    let harness = Harness::new();
    let body = r#"{"zen":"Non-blocking is better","sender":{"login":"octocat","id":583231},"installation":{"id":146704012}}"#;
    let response = harness
        .call(
            Request::post("/api/v1/github/app/webhook")
                .header("content-type", "application/json")
                .header("x-github-event", "ping")
                .header("x-github-delivery", "8f0a1c22-6b1e-11ee-9d0e-2f7a1b3c4d5e")
                .header("x-hub-signature-256", sign(body))
                .body(Body::from(body))
                .expect("request builds"),
        )
        .await;
    assert!(response.status().is_success(), "{}", response.status());

    let event = harness.only_event();
    assert_eq!(event.operation_id, "github_app_webhook");
    assert_eq!(
        event.correlation.webhook_delivery_id.as_deref(),
        Some("8f0a1c22-6b1e-11ee-9d0e-2f7a1b3c4d5e")
    );
    assert_eq!(event.correlation.installation_id, Some(146704012));
    assert_eq!(event.actor_id, Some(583231));
}

/// The browser surfaces render HTML instead of the JSON envelope, and a denial
/// there is the SAME policy decision a Bearer caller's `403` envelope carries —
/// so it must classify the same way, or the epic's `rejected` filtering silently
/// misses every browser-surface denial.
#[tokio::test]
async fn a_browser_surface_denial_is_recorded_as_a_rejection() {
    let harness = Harness::with_browser_login();
    let response = harness
        .get("/api/v1/logs/oauth/callback?error=access_denied")
        .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let event = harness.only_event();
    assert_eq!(event.operation_id, "session_logs_oauth_callback");
    assert_eq!(event.status_code, Some(403));
    assert_eq!(
        event.outcome,
        AuditOutcome::Rejected,
        "an HTML denial must not degrade to a plain client error"
    );
    assert_eq!(event.error_code.as_deref(), Some("oauth_forbidden"));
}

/// A browser failure that is NOT a policy decision (a missing/tampered OAuth
/// state) stays an ordinary client error, so `rejected` keeps meaning "denied".
#[tokio::test]
async fn a_browser_surface_bad_request_is_not_a_rejection() {
    let harness = Harness::with_browser_login();
    let response = harness.get("/api/v1/logs/oauth/callback").await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let event = harness.only_event();
    assert_eq!(event.outcome, AuditOutcome::ClientError);
    assert_eq!(event.error_code.as_deref(), Some("oauth_invalid_request"));
}

/// The browser OAuth callback is the highest-risk path for redaction: its query
/// carries the authorization `code` and CSRF `state`.
#[tokio::test]
async fn the_oauth_callback_records_its_template_and_never_its_query() {
    let harness = Harness::new();
    let response = harness
        .get("/api/v1/auth/github/callback?code=canary-code&state=canary-state")
        .await;
    // Login is unconfigured in this fixture, so the browser path renders its HTML
    // error page — the interesting part is what the RECORD contains.
    assert!(response.status().is_client_error() || response.status().is_server_error());

    let event = harness.only_event();
    assert_eq!(event.operation_id, "github_login_callback");
    assert_eq!(event.route_template, "/api/v1/auth/github/callback");
    assert!(
        event
            .error_code
            .as_deref()
            .is_some_and(|code| code.starts_with("oauth_")),
        "the HTML path must still carry a bounded code, got {:?}",
        event.error_code
    );
    let rendered = format!("{event:#?}");
    for canary in ["canary-code", "canary-state", "?"] {
        assert!(
            !rendered.contains(canary),
            "{canary} leaked into {rendered}"
        );
    }
}
