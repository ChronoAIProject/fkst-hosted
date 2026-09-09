//! Everything the staging smoke needs that is not an assertion.
//!
//! Split out so `acceptance_staging.rs` reads as the six numbered steps the
//! issue specifies and nothing else. The pieces here are deliberately the
//! PRODUCTION ones wherever a production one exists:
//!
//! - the record is produced by the real audit middleware;
//! - it is made durable through the real `AuditRelayClient`;
//! - it is captured by the real `PostHogClient`;
//! - and it is read back through the real `build_router`, so the query travels
//!   the whole product path — `AuthenticatedViewer`, the scope gate, the
//!   session-visibility projection, the fixed HogQL builder, the cursor binding,
//!   and the merge layer.
//!
//! That last one is the entire reason this tier exists. An earlier version of
//! this suite hand-wrote its own HogQL and posted it straight at PostHog, which
//! proved that PostHog can filter on a property — a fact nobody doubted — while
//! proving nothing at all about whether the PRODUCT's authorization holds
//! against a real project. Only the identity provider is mocked, because a
//! staging PostHog cannot mint GitHub tokens and GitHub identity is not what
//! this tier is testing.

#![allow(dead_code)]

#[path = "../acceptance_gate.rs"]
pub mod gate;

pub mod artifact;
pub mod probe;

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, Response};
use fkst_control_plane::audit::relay::{
    AuditDeliveryConfig, AuditDeliveryMode, AuditRelayClient, RelayClientMetrics,
};
use fkst_control_plane::audit::AuditConfig;
use fkst_control_plane::config::Config;
use fkst_control_plane::operations::posthog::{PosthogActivitySource, PosthogQueryClient};
use fkst_control_plane::operations::OperationsState;
use fkst_control_plane::router::build_router;
use fkst_control_plane::session_access::{SessionAccessRegistry, SessionAccessState};
use fkst_control_plane::state::{empty_self_router, AppState};
use http_body_util::BodyExt;
use secrecy::SecretString;
use serde_json::{json, Value};
use tower::ServiceExt;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The switch that opens this tier.
pub const SWITCH: &str = "FKST_ACCEPTANCE_POSTHOG_HOST";

/// Everything the switch implies.
///
/// The relay entries are required, not optional: the issue's step 2 is "relay
/// confirms durable start/completion", and a tier that silently skipped the
/// durable half would claim a round trip it never made.
pub const REQUIRED: [&str; 6] = [
    "FKST_ACCEPTANCE_POSTHOG_PROJECT_ID",
    "FKST_ACCEPTANCE_POSTHOG_TOKEN",
    "FKST_ACCEPTANCE_POSTHOG_QUERY_KEY",
    "FKST_ACCEPTANCE_RELAY_URL",
    "FKST_ACCEPTANCE_RELAY_WRITE_TOKEN",
    "FKST_ACCEPTANCE_RELAY_READ_TOKEN",
];

/// The originating regular user. A synthetic numeric id far outside GitHub's
/// allocated range, so nothing this suite writes can ever collide with — or be
/// mistaken for — a real person's record in a shared staging project.
pub const ALICE: (i64, &str) = (9_000_000_001, "fkst-acceptance-alice");
/// A second regular user who must NOT find Alice's record.
pub const BOB: (i64, &str) = (9_000_000_002, "fkst-acceptance-bob");
/// A deployment global administrator, who must find it in the `all` scope.
pub const GRACE: (i64, &str) = (9_000_000_003, "fkst-acceptance-grace");

/// The bearer token each fixture identity presents to the mocked GitHub.
pub fn token(who: (i64, &str)) -> String {
    format!("staging-token-{}", who.1)
}

/// The capture-side configuration, pointed at the staging project.
pub fn capture_config(environment: &gate::GateEnvironment) -> AuditConfig {
    AuditConfig {
        enabled: true,
        host: Some(host(environment).to_string()),
        project_token: SecretString::from(
            environment.get("FKST_ACCEPTANCE_POSTHOG_TOKEN").to_string(),
        ),
        ..AuditConfig::default()
    }
}

/// The staging PostHog host, without a trailing slash.
pub fn host(environment: &gate::GateEnvironment) -> &str {
    environment.get(SWITCH).trim_end_matches('/')
}

/// The staging project's numeric id.
pub fn project(environment: &gate::GateEnvironment) -> &str {
    environment.get("FKST_ACCEPTANCE_POSTHOG_PROJECT_ID")
}

/// A control-plane relay client for the staging relay, in REQUIRED mode.
///
/// Required mode is the point: best-effort delivery would let a relay outage
/// pass as a successful round trip, and the durability half of the smoke would
/// then assert nothing.
pub fn relay_client(environment: &gate::GateEnvironment) -> Arc<AuditRelayClient> {
    let config = AuditDeliveryConfig {
        mode: AuditDeliveryMode::Required,
        relay_url: Some(environment.get("FKST_ACCEPTANCE_RELAY_URL").to_string()),
        write_token: SecretString::from(
            environment
                .get("FKST_ACCEPTANCE_RELAY_WRITE_TOKEN")
                .to_string(),
        ),
        read_token: SecretString::from(
            environment
                .get("FKST_ACCEPTANCE_RELAY_READ_TOKEN")
                .to_string(),
        ),
        start_timeout_ms: 10_000,
        completion_timeout_ms: 10_000,
        incomplete_grace_secs: 60,
    };
    Arc::new(
        AuditRelayClient::from_config(&config, RelayClientMetrics::new())
            .expect("the relay client builds"),
    )
}

/// The product, wired to the staging project as its activity source.
pub struct Product {
    pub router: axum::Router,
    /// Kept alive for the harness's lifetime; dropping it stops the identity mock.
    _github: MockServer,
}

impl Product {
    /// Build the real router with the staging project behind the activity API.
    pub async fn start(environment: &gate::GateEnvironment) -> Self {
        let github = MockServer::start().await;
        for who in [ALICE, BOB, GRACE] {
            Mock::given(method("GET"))
                .and(path("/user"))
                .and(header(
                    "authorization",
                    format!("Bearer {}", token(who)).as_str(),
                ))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(json!({"login": who.1, "id": who.0})),
                )
                .mount(&github)
                .await;
        }
        Mock::given(method("GET"))
            .and(path("/user"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&github)
            .await;

        let query = PosthogQueryClient::new(
            format!(
                "{}/api/projects/{}/query/",
                host(environment),
                project(environment)
            ),
            SecretString::from(
                environment
                    .get("FKST_ACCEPTANCE_POSTHOG_QUERY_KEY")
                    .to_string(),
            ),
            Duration::from_secs(30),
        )
        .expect("the staging query client builds");

        let config = Config::from_vars([
            ("FKST_GLOBAL_ADMINS".to_string(), GRACE.1.to_string()),
            (
                "FKST_POSTHOG_PROJECT_ID".to_string(),
                project(environment).to_string(),
            ),
            (
                "FKST_POSTHOG_QUERY_API_KEY".to_string(),
                environment
                    .get("FKST_ACCEPTANCE_POSTHOG_QUERY_KEY")
                    .to_string(),
            ),
        ])
        .expect("the staging configuration parses");
        let config = Config {
            github_api_base_url: github.uri(),
            ..config
        };

        let router = build_router(AppState {
            config,
            recovery: Default::default(),
            github_app: None,
            github_app_webhook_secret: None,
            reconciler: None,
            session_backend: None,
            storage: None,
            // Dispatch off: this tier asserts API-row authorization, and an
            // authoritatively empty session projection is the honest shape for a
            // deployment with no reconciler attached.
            session_access: SessionAccessState::new(SessionAccessRegistry::new(false)),
            operations: OperationsState::with_sources(
                Some(Arc::new(PosthogActivitySource::new(query))),
                None,
            ),
            log_bundle_cache: Default::default(),
            disposable_environments: Default::default(),
            self_router: empty_self_router(),
            chat: None,
            audit: Default::default(),
        })
        .expect("the staging router builds");

        Self {
            router,
            _github: github,
        }
    }

    /// One authenticated activity query, exactly as the SPA issues it.
    pub async fn activity(&self, who: (i64, &str), query: &str) -> Response<Body> {
        self.router
            .clone()
            .oneshot(
                Request::get(format!("/api/v1/operations/activity{query}"))
                    .header("host", "acceptance")
                    .header("authorization", format!("Bearer {}", token(who)))
                    .body(Body::empty())
                    .expect("request builds"),
            )
            .await
            .expect("the product router responds")
    }

    /// The event ids a viewer's scoped page contains.
    pub async fn event_ids(&self, who: (i64, &str), query: &str) -> Vec<String> {
        let response = self.activity(who, query).await;
        let status = response.status();
        let body = body_json(response).await;
        assert!(
            status.is_success(),
            "the product refused a scoped read with {status}: {body}"
        );
        body["items"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|item| item["event_id"].as_str().map(str::to_string))
            .collect()
    }
}

pub async fn body_json(response: Response<Body>) -> Value {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| json!({ "raw": String::from_utf8_lossy(&bytes) }))
}

/// A per-run nonce so a shared staging project never mixes two runs.
pub fn run_nonce() -> String {
    format!(
        "{:x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or_default()
    )
}
