//! The redaction-canary harness.
//!
//! Every value this module plants is unique and greppable, so an assertion can
//! search a WHOLE serialized surface rather than a field someone remembered to
//! check. That matters: a leak that reaches an observability surface almost
//! never arrives through the field you were watching.
//!
//! The harness drives the REAL router against a wiremock GitHub, so the
//! requests below take the same path a browser or the SPA would — extractors,
//! identity verification, the access policy, handlers, and `AppError`
//! conversion all run. What is asserted afterwards is what the audit pipeline
//! actually produced, projected exactly as PostHog would receive it.

#![allow(dead_code)]

pub mod log_bundle;
/// The requests that plant every canary.
mod plant;

// Re-exported so every suite keeps importing `audit_canary::plant_every_canary`
// after the split. Not every suite uses both, and each test binary compiles this
// module separately, so an unused re-export here is expected rather than a
// mistake.
#[allow(unused_imports)]
pub use plant::{create_session_body, plant_every_canary};

use axum::body::Body;
use axum::http::Request;
use fkst_control_plane::audit::projection::EventLimits;
use fkst_control_plane::audit::sink::RecordingSink;
use fkst_control_plane::audit::{ApiRequestCompletedV1, AuditHandle};
use fkst_control_plane::config::Config;
use fkst_control_plane::router::build_router;
use fkst_control_plane::state::{empty_self_router, AppState};
use hmac::{Hmac, Mac};
use http_body_util::BodyExt;
use sha2::Sha256;
use tower::ServiceExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The verified caller every authenticated canary request acts as.
pub const LOGIN: &str = "octocat";
/// That caller's immutable GitHub id, as the mocked `/user` reports it.
pub const USER_ID: i64 = 583_231;
/// The webhook secret the harness signs deliveries with.
pub const WEBHOOK_SECRET: &str = "canary-webhook-secret";
/// A signature that cannot verify against any secret.
pub const BOGUS_SIGNATURE: &str =
    "sha256=00000000000000000000000000000000000000000000000000000000000000ff";

/// The body every failing upstream call returns.
///
/// It deliberately carries three DIFFERENT hostile shapes at once — a free-text
/// message, a minted credential, and a stack string — because an upstream error
/// body is one value in the code and three separate leak classes in practice.
pub const UPSTREAM_ERROR_BODY: &str = concat!(
    r#"{"message":"canary-upstream-body","#,
    r#""stack":"canary-stack-frame at src/upstream.rs:1"}"#
);

/// Every hostile value planted by the suite, in one list so a single assertion
/// covers a whole surface.
pub const CANARIES: &[&str] = &[
    // credentials and headers
    "canary-bearer-token",
    "canary-cookie-value",
    "canary-custom-header",
    "canary-broader-token",
    // OAuth material
    "canary-oauth-code",
    "canary-oauth-state",
    "canary-oauth-error",
    // unrouted path and query
    "canary-unrouted-path",
    "canary-query-token",
    // repository and session free text
    "canary-repository-description",
    "canary-session-name",
    "canary-work-item-title",
    "canary-work-item-body",
    // environment contents
    "canary-install-command",
    "CANARY_VARIABLE_KEY",
    "canary-variable-value",
    "CANARY_SECRET_KEY",
    "canary-secret-value",
    // webhook payload
    "canary-webhook-signature",
    "canary-issue-title",
    "canary-issue-body",
    // logs
    "canary-log-path",
    "canary-log-content",
    // error text and upstream bodies
    "canary-invalid-branch",
    "canary-upstream-body",
    // package reference input, which is parsed and quoted back on failure
    "canary-invalid-package-ref",
    // OAuth session credentials: the one a caller presents, and the pair the
    // upstream mints in exchange. All three are credentials this deployment
    // handles but must never persist.
    "canary-refresh-token",
    "canary-access-token",
    "canary-rotated-refresh-token",
    // a stack string inside an upstream error body — a different leak class from
    // the message beside it, because a stack names internal paths
    "canary-stack-frame",
    // credentials the DEPLOYMENT holds in its own configuration. They never
    // travel on a request, so only a config-serializing bug could surface them —
    // which is exactly the bug a per-request corpus cannot find.
    "canary-llm-api-key",
    "canary-posthog-query-key",
    "canary-storage-client-secret",
    // userinfo and query material inside a configured BACKEND URL
    "canary-url-userinfo",
    "canary-url-query",
];

/// `sha256=<hex>` over `body`, exactly as GitHub signs a delivery.
pub fn sign(body: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(WEBHOOK_SECRET.as_bytes()).expect("hmac key");
    mac.update(body.as_bytes());
    let digest = mac.finalize().into_bytes();
    let mut out = String::from("sha256=");
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

pub struct Canary {
    router: axum::Router,
    sink: RecordingSink,
    /// Kept alive for the duration of the harness: dropping either stops that
    /// mock, and every canary request needs both.
    _github: MockServer,
    _storage: MockServer,
}

impl Canary {
    /// A deployment whose identity checks resolve `LOGIN`, whose access policy
    /// admits only that login, whose browser-login + webhook surfaces are
    /// configured so their canary paths reach real handlers, and whose log
    /// storage really serves a bundle to that caller (see [`log_bundle`]).
    ///
    /// Every OTHER GitHub call the handlers make lands on the same mock and gets
    /// a body carrying an upstream canary — proving an upstream response can
    /// never become an audit property either.
    pub async fn start() -> Self {
        let github = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/user"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "login": LOGIN,
                "id": USER_ID,
            })))
            .mount(&github)
            .await;
        // The OAuth token endpoint mints a REAL-shaped credential pair. A login
        // callback or a refresh therefore ends with this process holding an
        // access token and a rotated refresh token it never asked for by name —
        // the realistic way a credential ends up somewhere nobody planned for.
        Mock::given(method("POST"))
            .and(path("/login/oauth/access_token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "canary-access-token",
                "refresh_token": "canary-rotated-refresh-token",
                "expires_in": 28_800,
                "refresh_token_expires_in": 15_811_200,
                "token_type": "bearer",
                "scope": "",
            })))
            .mount(&github)
            .await;
        // The catch-all: any other GitHub call fails with a canary-bearing body
        // carrying an upstream message, a minted token, and a stack string.
        for verb in ["GET", "POST", "PATCH", "PUT", "DELETE"] {
            Mock::given(method(verb))
                .respond_with(ResponseTemplate::new(500).set_body_string(UPSTREAM_ERROR_BODY))
                .mount(&github)
                .await;
        }

        let mut config = Config::from_vars([
            ("FKST_GITHUB_API_BASE_URL".to_string(), github.uri()),
            ("FKST_ACCESS_ALLOWED_USERS".to_string(), LOGIN.to_string()),
            ("FKST_POSTHOG_PROJECT_ID".to_string(), "42".to_string()),
            (
                "FKST_POSTHOG_QUERY_API_KEY".to_string(),
                "canary-posthog-query-key".to_string(),
            ),
        ])
        .expect("the canary configuration parses");
        config.log.oauth_client_id = Some("Iv1.canary".to_string());
        config.log.oauth_client_secret = Some(secrecy::SecretString::from("canary-oauth-secret"));
        config.log.public_base_url = Some("https://api.example.test".to_string());
        config.log.frontend_url = Some("https://app.example.test".to_string());
        // The token exchange must reach the LOCAL mock, never github.com.
        config.log.oauth_base_url = github.uri();
        // A deployment credential that never travels on a request.
        config.llm_api_key = secrecy::SecretString::from("canary-llm-api-key");

        let (storage, storage_server) = log_bundle::storage().await;
        let (audit, sink) = AuditHandle::recording();
        let router = build_router(AppState {
            config,
            recovery: Default::default(),
            github_app: None,
            github_app_webhook_secret: Some(secrecy::SecretString::from(
                WEBHOOK_SECRET.to_string(),
            )),
            reconciler: None,
            session_backend: None,
            storage: Some(storage),
            session_access: log_bundle::access(LOGIN, USER_ID),
            operations: Default::default(),
            log_bundle_cache: Default::default(),
            disposable_environments: Default::default(),
            self_router: empty_self_router(),
            chat: None,
            audit,
        })
        .expect("the canary router builds");

        Self {
            router,
            sink,
            _github: github,
            _storage: storage_server,
        }
    }

    pub async fn call(&self, request: Request<Body>) -> axum::response::Response {
        self.router
            .clone()
            .oneshot(request)
            .await
            .expect("router responds")
    }

    /// A request carrying every credential-shaped header the suite plants.
    pub fn authenticated(&self, builder: axum::http::request::Builder) -> Request<Body> {
        builder
            .header("authorization", "Bearer canary-bearer-token")
            .header("cookie", "fkst_session=canary-cookie-value")
            .header("x-canary-header", "canary-custom-header")
            .header("x-github-broader-token", "canary-broader-token")
            .body(Body::empty())
            .expect("request builds")
    }

    /// The same, with a JSON body.
    pub fn authenticated_json(
        &self,
        builder: axum::http::request::Builder,
        body: serde_json::Value,
    ) -> Request<Body> {
        builder
            .header("authorization", "Bearer canary-bearer-token")
            .header("cookie", "fkst_session=canary-cookie-value")
            .header("x-canary-header", "canary-custom-header")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .expect("request builds")
    }

    /// Every terminal record produced so far.
    pub fn events(&self) -> Vec<ApiRequestCompletedV1> {
        self.sink.events()
    }

    /// The single record for `operation_id`.
    pub fn event(&self, operation_id: &str) -> ApiRequestCompletedV1 {
        let mut matching: Vec<_> = self
            .events()
            .into_iter()
            .filter(|event| event.operation_id == operation_id)
            .collect();
        assert_eq!(
            matching.len(),
            1,
            "expected exactly one {operation_id} record, got {}",
            matching.len()
        );
        matching.remove(0)
    }

    /// The response body of `GET /metrics`, for the metrics canary assertion.
    pub async fn metrics_text(&self) -> String {
        let response = self
            .call(
                Request::get("/metrics")
                    .body(Body::empty())
                    .expect("request builds"),
            )
            .await;
        body_text(response).await
    }
}

/// A response body as text, for the assertions that must prove a canary really
/// was served before proving it was not recorded.
pub async fn body_text(response: axum::response::Response) -> String {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect response body")
        .to_bytes();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Every observability rendering of one record, concatenated.
///
/// Includes the `Debug` form (what a panic or a structured log would print) AND
/// the exact PostHog capture payload (what actually leaves the process), so an
/// assertion covers both the internal and the external surface at once.
pub fn rendered(event: &ApiRequestCompletedV1) -> String {
    let capture = event
        .to_capture_event(EventLimits::new(usize::MAX))
        .expect("a recorded event must satisfy the contract");
    format!(
        "{event:#?}\n{}",
        serde_json::to_string(&capture).expect("the capture payload serializes")
    )
}

/// Assert that no canary reached any rendering of any recorded event.
pub fn assert_no_canaries(canary: &Canary) {
    let events = canary.events();
    assert!(!events.is_empty(), "the harness recorded nothing at all");
    for event in &events {
        let text = rendered(event);
        for planted in CANARIES {
            assert!(
                !text.contains(planted),
                "{planted} reached the {} record:\n{text}",
                event.operation_id
            );
        }
    }
}

/// The `arguments` of a record, as a readable JSON object.
pub fn arguments(event: &ApiRequestCompletedV1) -> serde_json::Value {
    serde_json::Value::Object(event.arguments.clone())
}
