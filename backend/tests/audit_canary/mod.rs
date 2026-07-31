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
/// The webhook secret the harness signs deliveries with.
pub const WEBHOOK_SECRET: &str = "canary-webhook-secret";
/// A signature that cannot verify against any secret.
pub const BOGUS_SIGNATURE: &str =
    "sha256=00000000000000000000000000000000000000000000000000000000000000ff";

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
    /// Kept alive for the duration of the harness: dropping it stops the mock.
    _github: MockServer,
}

impl Canary {
    /// A deployment whose identity checks resolve `LOGIN`, whose access policy
    /// admits only that login, and whose browser-login + webhook surfaces are
    /// configured so their canary paths reach real handlers.
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
                "id": 583_231,
            })))
            .mount(&github)
            .await;
        // The catch-all: any other GitHub call fails with a canary-bearing body.
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(500).set_body_string(r#"{"message":"canary-upstream-body"}"#),
            )
            .mount(&github)
            .await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(500).set_body_string(r#"{"message":"canary-upstream-body"}"#),
            )
            .mount(&github)
            .await;

        let mut config = Config::from_vars([
            ("FKST_GITHUB_API_BASE_URL".to_string(), github.uri()),
            ("FKST_ACCESS_ALLOWED_USERS".to_string(), LOGIN.to_string()),
        ])
        .expect("the canary configuration parses");
        config.log.oauth_client_id = Some("Iv1.canary".to_string());
        config.log.oauth_client_secret = Some(secrecy::SecretString::from("canary-oauth-secret"));
        config.log.public_base_url = Some("https://api.example.test".to_string());
        config.log.frontend_url = Some("https://app.example.test".to_string());

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
            storage: None,
            session_access: Default::default(),
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
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("collect metrics body")
            .to_bytes();
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

/// A create-session body carrying a canary in every free-text location.
pub fn create_session_body() -> serde_json::Value {
    serde_json::json!({
        "name": "canary-session-name",
        "packages": ["acme/pkgs@main:packages/devloop"],
        "manifests": ["acme/pkgs@main:manifests/default.json"],
        "work_label": "fkst:work",
        "source_branch": "main",
        "target_branch": "fkst-hosted-default",
        "auto_merge": true,
        "log_access": ["grantee-one", "grantee-two"],
        "collaborators": ["collab-one"],
        "output_lang": "zh-CN",
        "disposable_environment": {
            "install": ["canary-install-command"],
            "variables": { "CANARY_VARIABLE_KEY": "canary-variable-value" },
            "secrets": { "CANARY_SECRET_KEY": "canary-secret-value" },
        },
    })
}

/// Drive every canary-bearing request through the real router once.
pub async fn plant_every_canary(canary: &Canary) {
    // Credentials in a header, a cookie, and a custom header, on a route whose
    // arguments are recorded before anything else runs.
    canary
        .call(canary.authenticated(Request::get("/api/v1/overview")))
        .await;

    // OAuth code, state, and GitHub's own error slug, all in the query.
    canary
        .call(
            Request::get(
                "/api/v1/auth/github/callback\
                 ?code=canary-oauth-code&state=canary-oauth-state&error=canary-oauth-error",
            )
            .body(Body::empty())
            .expect("request builds"),
        )
        .await;

    // An unrouted path with a credential-shaped query value.
    canary
        .call(
            Request::get("/api/v1/canary-unrouted-path?token=canary-query-token")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await;

    // A repository description.
    canary
        .call(canary.authenticated_json(
            Request::post("/api/v1/repos"),
            serde_json::json!({
                "owner": null,
                "name": "site",
                "private": true,
                "description": "canary-repository-description",
            }),
        ))
        .await;

    // A session name plus disposable-environment keys, values, and commands.
    canary
        .call(canary.authenticated_json(
            Request::post("/api/v1/repos/acme/site/sessions"),
            create_session_body(),
        ))
        .await;

    // A branch value that only the ERROR MESSAGE quotes back.
    let mut invalid_branch = create_session_body();
    invalid_branch["source_branch"] = serde_json::json!("canary-invalid-branch name");
    canary
        .call(canary.authenticated_json(
            Request::post("/api/v1/repos/acme/site/sessions"),
            invalid_branch,
        ))
        .await;

    // A work item's title and body.
    canary
        .call(canary.authenticated_json(
            Request::post("/api/v1/repos/acme/site/sessions/42/work-items"),
            serde_json::json!({
                "title": "canary-work-item-title",
                "body": "canary-work-item-body",
                "label": "fkst:work",
            }),
        ))
        .await;

    // Install commands, variable keys/values, and secret keys/values.
    canary
        .call(canary.authenticated_json(
            Request::put("/api/v1/users/me/environment-profiles/node-20"),
            serde_json::json!({
                "install": ["canary-install-command"],
                "variables": { "CANARY_VARIABLE_KEY": "canary-variable-value" },
                "secrets": { "CANARY_SECRET_KEY": "canary-secret-value" },
            }),
        ))
        .await;

    // A requested log PATH (the archive is matched on it, so an unmatched one is
    // a probe string).
    canary
        .call(canary.authenticated(Request::get(
            "/api/v1/logs/8f0a1c22-6b1e-11ee-9d0e-2f7a1b3c4d5e/file\
             ?path=canary-log-path&tail_bytes=4096",
        )))
        .await;

    // A rejected webhook delivery: everything it claims is attacker-controlled.
    canary
        .call(
            Request::post("/api/v1/github/app/webhook")
                .header("x-hub-signature-256", BOGUS_SIGNATURE)
                .header("x-github-event", "issues")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "action": "opened",
                        "issue": { "number": 9, "title": "canary-issue-title",
                                   "body": "canary-issue-body" },
                        "repository": { "owner": { "login": "acme" }, "name": "site" },
                        "installation": { "id": 7 },
                        "sender": { "login": "canary-webhook-signature", "id": 1 },
                    })
                    .to_string(),
                ))
                .expect("request builds"),
        )
        .await;

    // A VERIFIED delivery whose payload still carries issue free text.
    let verified = serde_json::json!({
        "action": "opened",
        "issue": { "number": 9, "title": "canary-issue-title", "body": "canary-issue-body" },
        "repository": { "owner": { "login": "acme" }, "name": "site" },
        "installation": { "id": 7 },
        "sender": { "login": "octocat", "id": 583_231 },
    })
    .to_string();
    canary
        .call(
            Request::post("/api/v1/github/app/webhook")
                .header("x-hub-signature-256", sign(&verified))
                .header("x-github-event", "issues")
                .header("content-type", "application/json")
                .body(Body::from(verified))
                .expect("request builds"),
        )
        .await;

    // A malformed body whose bytes are themselves a canary.
    canary
        .call(
            Request::post("/api/v1/repos")
                .header("authorization", "Bearer canary-bearer-token")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"name": canary-log-content}"#))
                .expect("request builds"),
        )
        .await;
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
