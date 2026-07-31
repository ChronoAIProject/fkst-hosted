//! Router-level tests for the outer audit middleware, against the REAL
//! [`build_router`].
//!
//! The point of driving the real router is that the middleware's position
//! relative to every inner layer — CORS, the route-scoped timeouts, the
//! leader-readiness gate, the identity extractors, `AppError` conversion, and
//! axum's own routing answers — is *proven* rather than inferred from the order
//! the layers happen to be declared in.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use fkst_control_plane::audit::sink::RecordingSink;
use fkst_control_plane::audit::{ApiRequestCompletedV1, AuditHandle, AuditOutcome};
use fkst_control_plane::config::Config;
use fkst_control_plane::recovery::RecoveryMonitor;
use fkst_control_plane::router::build_router;
use fkst_control_plane::state::{empty_self_router, AppState};
use tower::ServiceExt;

/// A signature that cannot verify against any secret.
const BOGUS_SIGNATURE: &str =
    "sha256=00000000000000000000000000000000000000000000000000000000000000ff";

struct Harness {
    router: axum::Router,
    sink: RecordingSink,
}

impl Harness {
    fn new() -> Self {
        Self::build(Config::default(), RecoveryMonitor::new(false), true)
    }

    /// An election-enabled replica that has NOT completed its acquisition resync,
    /// so every gated route short-circuits before any handler.
    fn follower() -> Self {
        let mut config = Config::default();
        config.leader.enabled = true;
        config.leader.identity = Some("pod-follower".to_string());
        let recovery = RecoveryMonitor::new(true);
        recovery.enable_leader_election("pod-follower".to_string());
        Self::build(config, recovery, true)
    }

    fn build(config: Config, recovery: RecoveryMonitor, webhook: bool) -> Self {
        let (audit, sink) = AuditHandle::recording();
        let router = build_router(AppState {
            config,
            recovery,
            github_app: None,
            github_app_webhook_secret: webhook
                .then(|| secrecy::SecretString::from("audit-test-secret".to_string())),
            reconciler: None,
            session_backend: None,
            storage: None,
            session_access: Default::default(),
            log_bundle_cache: Default::default(),
            disposable_environments: Default::default(),
            self_router: empty_self_router(),
            chat: None,
            audit,
        })
        .expect("router builds");
        Self { router, sink }
    }

    async fn call(&self, request: Request<Body>) -> axum::response::Response {
        self.router
            .clone()
            .oneshot(request)
            .await
            .expect("router responds")
    }

    async fn get(&self, path: &str) -> axum::response::Response {
        self.call(
            Request::get(path)
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
    }

    fn only_event(&self) -> ApiRequestCompletedV1 {
        let events = self.sink.events();
        assert_eq!(
            events.len(),
            1,
            "exactly one terminal record per request, got {events:#?}"
        );
        events.into_iter().next().expect("one event")
    }
}

/// Probe, scrape, contract, and preflight traffic must never reach the sink —
/// and must still be answered exactly as before.
#[tokio::test]
async fn excluded_traffic_produces_no_records() {
    let harness = Harness::new();
    for path in ["/health", "/ready", "/metrics", "/openapi.json"] {
        let response = harness.get(path).await;
        assert!(
            response.status().is_success(),
            "{path} must still answer: {}",
            response.status()
        );
    }
    let preflight = harness
        .call(
            Request::builder()
                .method(Method::OPTIONS)
                .uri("/api/v1/overview")
                .header("origin", "https://app.example")
                .header("access-control-request-method", "GET")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await;
    assert!(preflight.status().is_success());
    assert!(
        harness.sink.is_empty(),
        "probe/scrape/contract/preflight traffic must be excluded, got {:#?}",
        harness.sink.events()
    );
}

/// An unrouted `/api/v1` path may carry OAuth material in its query, so neither
/// the path nor the query may survive into the record.
#[tokio::test]
async fn an_unmatched_api_path_records_sentinels_only() {
    let harness = Harness::new();
    let response = harness
        .get("/api/v1/not-a-real-route?code=canary-code&state=canary-state")
        .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let event = harness.only_event();
    assert_eq!(event.operation_id, "<unmatched>");
    assert_eq!(event.route_template, "<unmatched>");
    assert_eq!(event.outcome, AuditOutcome::ClientError);
    assert_eq!(event.error_code.as_deref(), Some("route_not_found"));
    let rendered = format!("{event:#?}");
    for canary in ["canary-code", "canary-state", "not-a-real-route"] {
        assert!(
            !rendered.contains(canary),
            "{canary} leaked into {rendered}"
        );
    }
}

/// A matched path with an unserved method keeps its documented template (a
/// public constant) but never invents an operation id.
#[tokio::test]
async fn a_method_not_allowed_answer_is_recorded_under_the_matched_template() {
    let harness = Harness::new();
    let response = harness
        .call(
            Request::post("/api/v1/overview")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await;
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);

    let event = harness.only_event();
    assert_eq!(event.operation_id, "<unmatched>");
    assert_eq!(event.route_template, "/api/v1/overview");
    assert_eq!(event.error_code.as_deref(), Some("method_not_allowed"));
}

/// The leader gate answers before any handler; the record must say so.
#[tokio::test]
async fn a_leader_gate_rejection_is_recorded_as_rejected_with_its_real_status() {
    let harness = Harness::follower();
    let response = harness.get("/api/v1/overview").await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

    let event = harness.only_event();
    assert_eq!(event.operation_id, "canvas_overview");
    assert_eq!(event.status_code, Some(503));
    assert_eq!(event.outcome, AuditOutcome::Rejected);
    assert_eq!(event.error_code.as_deref(), Some("leader_not_ready"));
    assert_eq!(
        event.actor_id, None,
        "a gated request has no verified actor"
    );
}

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

#[tokio::test]
async fn a_well_formed_request_id_is_accepted_and_echoed_and_recorded() {
    let harness = Harness::new();
    let response = harness
        .call(
            Request::get("/api/v1/overview")
                .header("x-request-id", "client-supplied-1")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await;
    assert_eq!(
        response
            .headers()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok()),
        Some("client-supplied-1")
    );
    assert_eq!(harness.only_event().request_id, "client-supplied-1");
}

#[tokio::test]
async fn a_malformed_request_id_is_replaced_in_the_response_and_the_record() {
    let harness = Harness::new();
    let response = harness
        .call(
            Request::get("/api/v1/overview")
                .header("x-request-id", "forged id; with separators")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await;
    let echoed = response
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .expect("a normalized id is always echoed")
        .to_string();
    assert_ne!(echoed, "forged id; with separators");
    assert_eq!(harness.only_event().request_id, echoed);
}

#[tokio::test]
async fn a_request_without_a_request_id_gets_a_generated_one() {
    let harness = Harness::new();
    let response = harness.get("/api/v1/overview").await;
    let echoed = response
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .expect("a request id is always generated")
        .to_string();
    assert_eq!(echoed.len(), 36, "a generated id is a hyphenated UUID");
    assert_eq!(harness.only_event().request_id, echoed);
}

/// Different requests must never share a delivery id, or PostHog's UUID
/// deduplication would silently collapse them into one row.
#[tokio::test]
async fn every_request_gets_its_own_event_id() {
    let harness = Harness::new();
    for path in [
        "/api/v1/overview",
        "/api/v1/users/me/environment-profiles",
        "/api/v1/nope",
    ] {
        let _ = harness.get(path).await;
    }
    let events = harness.sink.events();
    assert_eq!(events.len(), 3);
    let mut ids: Vec<_> = events.iter().map(|event| event.event_id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 3, "every request needs a distinct event id");
}
