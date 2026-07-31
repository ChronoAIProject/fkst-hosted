//! Shared harness for the `/api/v1/operations/activity` integration tests.
//!
//! It drives the REAL `build_router` — extractors, identity verification, the
//! access policy, the scope gate, the session-visibility registry, the handler,
//! and `AppError` conversion all run — against two wiremock servers: GitHub
//! `/user` for identity, and a PostHog query endpoint that applies the request's
//! OWN predicate to a fixed dataset.
//!
//! That second detail is the point. A mock that returned rows regardless of the
//! predicate would let a broken source query pass every test; a mock that
//! HONOURS the predicate proves both that the predicate reached the source and
//! that rows hidden by it never affect the page.

#![allow(dead_code)]

/// The planted dataset and the predicate-applying PostHog stand-in.
pub mod dataset;

pub use dataset::Row;

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, Response, StatusCode};
use fkst_control_plane::audit::sink::RecordingSink;
use fkst_control_plane::audit::AuditHandle;
use fkst_control_plane::config::Config;
use fkst_control_plane::models::RepoRef;
use fkst_control_plane::operations::posthog::{PosthogActivitySource, PosthogQueryClient};
use fkst_control_plane::operations::{ActivitySource, OperationsState};
use fkst_control_plane::reconcile::creator::SessionCreator;
use fkst_control_plane::router::build_router;
use fkst_control_plane::session_access::{
    SessionAccessContext, SessionAccessRegistry, SessionAccessState,
};
use fkst_control_plane::state::{empty_self_router, AppState};
use http_body_util::BodyExt;
use secrecy::SecretString;
use serde_json::{json, Value};
use tower::ServiceExt;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The session Alice created; Bob is a collaborator on it.
pub const SESSION: &str = "sess-alice";
/// A session nobody in these fixtures may see.
pub const OTHER_SESSION: &str = "sess-stranger";

pub const ALICE: (i64, &str) = (101, "alice");
pub const BOB: (i64, &str) = (102, "bob");
/// A verified user with no relationship to the session at all.
pub const ERIN: (i64, &str) = (105, "erin");
/// A deployment global administrator.
pub const ROOT: (i64, &str) = (900, "root");

/// The bearer token each fixture identity presents.
pub fn token(who: (i64, &str)) -> String {
    format!("token-{}", who.1)
}

/// An RFC3339 UTC instant `minutes` before now.
///
/// Fixtures are anchored to the wall clock because the endpoint's default window
/// is "the last 24 hours": a fixed literal date would silently fall out of range
/// the day after it was written.
pub fn minutes_ago(minutes: i64) -> String {
    (k8s_openapi::chrono::Utc::now() - k8s_openapi::chrono::Duration::minutes(minutes))
        .to_rfc3339_opts(k8s_openapi::chrono::SecondsFormat::Millis, true)
}

/// The assembled harness: a router plus the two mock servers it talks to.
pub struct Harness {
    pub router: axum::Router,
    /// Kept alive for the lifetime of the test; dropping it stops the server.
    _github: MockServer,
    pub posthog: Option<MockServer>,
    /// Every audit record the outer middleware produced, so a test can assert
    /// what a request — including a REFUSED one — actually recorded.
    pub audit: RecordingSink,
    /// The activity telemetry, for the closed-label metric assertions.
    pub operations: OperationsState,
}

/// How the activity source is wired for one test.
pub enum Sources {
    /// A predicate-aware PostHog holding `rows`, and no relay.
    Posthog(Vec<Row>),
    /// A PostHog that always answers with `status`, and no relay.
    PosthogFailing(u16),
    /// No source at all (the unconfigured deployment).
    None,
    /// An explicit pair, for the relay-merge tests.
    Explicit {
        posthog: Option<Arc<dyn ActivitySource>>,
        relay: Option<Arc<dyn ActivitySource>>,
    },
}

/// Build a harness. `registry_ready` false leaves the visibility projection cold.
pub async fn harness(sources: Sources, registry_ready: bool) -> Harness {
    let github = MockServer::start().await;
    for who in [ALICE, BOB, ERIN, ROOT] {
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
    // Any other token is rejected, exactly as GitHub would.
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&github)
        .await;

    let (operations, posthog) = build_sources(sources).await;
    // Built through the real `from_vars` rather than by mutating fields: the
    // access policy and the query config are exactly what an operator's
    // environment would produce.
    let config = Config::from_vars([
        ("FKST_GLOBAL_ADMINS".to_string(), ROOT.1.to_string()),
        ("FKST_POSTHOG_PROJECT_ID".to_string(), "42".to_string()),
        (
            "FKST_POSTHOG_QUERY_API_KEY".to_string(),
            "phx_read_key".to_string(),
        ),
    ])
    .expect("the fixture configuration parses");
    let config = Config {
        github_api_base_url: github.uri(),
        ..config
    };

    let (audit, sink) = AuditHandle::recording();
    let router = build_router(AppState {
        config,
        recovery: Default::default(),
        github_app: None,
        github_app_webhook_secret: None,
        reconciler: None,
        session_backend: None,
        storage: None,
        session_access: SessionAccessState::new(registry(registry_ready)),
        operations: operations.clone(),
        log_bundle_cache: Default::default(),
        disposable_environments: Default::default(),
        self_router: empty_self_router(),
        chat: None,
        audit,
    })
    .expect("router builds");

    Harness {
        router,
        _github: github,
        posthog,
        audit: sink,
        operations,
    }
}

async fn build_sources(sources: Sources) -> (OperationsState, Option<MockServer>) {
    match sources {
        Sources::None => (OperationsState::default(), None),
        Sources::Explicit { posthog, relay } => {
            (OperationsState::with_sources(posthog, relay), None)
        }
        Sources::Posthog(rows) => {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/api/projects/42/query/"))
                .respond_with(dataset::PredicateAwareQuery::new(rows))
                .mount(&server)
                .await;
            let state = OperationsState::with_sources(Some(posthog_source(&server)), None);
            (state, Some(server))
        }
        Sources::PosthogFailing(status) => {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(status))
                .mount(&server)
                .await;
            let state = OperationsState::with_sources(Some(posthog_source(&server)), None);
            (state, Some(server))
        }
    }
}

fn posthog_source(server: &MockServer) -> Arc<dyn ActivitySource> {
    let client = PosthogQueryClient::new(
        format!("{}/api/projects/42/query/", server.uri()),
        SecretString::from("phx_read_key".to_string()),
        Duration::from_millis(2_000),
    )
    .expect("query client builds");
    Arc::new(PosthogActivitySource::new(client))
}

/// The visibility projection: Alice created `SESSION`, Bob is a collaborator.
fn registry(ready: bool) -> SessionAccessRegistry {
    let repo = RepoRef {
        owner: "acme".to_string(),
        name: "site".to_string(),
    };
    // `new(true)` means dispatch is on, so the projection starts COLD and fails
    // closed until a generation is published.
    let registry = SessionAccessRegistry::new(!ready);
    if ready {
        registry.replace_repo(
            1,
            &repo,
            vec![(
                SESSION.to_string(),
                SessionAccessContext {
                    installation_id: 1,
                    repo: repo.clone(),
                    trigger_issue: 7,
                    creator: SessionCreator {
                        login: ALICE.1.to_string(),
                        id: Some(ALICE.0),
                    },
                    collaborators: vec![BOB.1.to_string()],
                    log_access: Vec::new(),
                },
            )],
        );
    }
    registry
}

impl Harness {
    /// Issue one authenticated request and return the raw response.
    pub async fn get(&self, who: (i64, &str), query: &str) -> Response<Body> {
        self.request(Some(who), query).await
    }

    /// Issue one request, optionally without an identity.
    pub async fn request(&self, who: Option<(i64, &str)>, query: &str) -> Response<Body> {
        let mut builder =
            Request::get(format!("/api/v1/operations/activity{query}")).header("host", "test");
        if let Some(who) = who {
            builder = builder.header("authorization", format!("Bearer {}", token(who)));
        }
        self.router
            .clone()
            .oneshot(builder.body(Body::empty()).expect("request builds"))
            .await
            .expect("router responds")
    }

    /// Issue a request expected to succeed, returning the parsed page.
    pub async fn page(&self, who: (i64, &str), query: &str) -> Value {
        let response = self.get(who, query).await;
        let status = response.status();
        let body = body_json(response).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    /// How many queries the PostHog stand-in received. `0` proves a refusal cost
    /// the deployment no upstream call.
    pub async fn source_calls(&self) -> usize {
        match &self.posthog {
            Some(server) => server
                .received_requests()
                .await
                .map(|requests| requests.len())
                .unwrap_or_default(),
            None => 0,
        }
    }

    /// The last outbound query text, for the source-predicate assertions.
    pub async fn last_query_text(&self) -> String {
        let requests = self
            .posthog
            .as_ref()
            .expect("this harness has a posthog stand-in")
            .received_requests()
            .await
            .expect("recorded");
        let body: Value =
            serde_json::from_slice(&requests.last().expect("at least one query").body)
                .expect("a JSON body");
        body["query"]["query"]
            .as_str()
            .expect("query text")
            .to_string()
    }
}

/// The event ids of a page's items, in order.
pub fn item_ids(page: &Value) -> Vec<String> {
    page["items"]
        .as_array()
        .expect("items array")
        .iter()
        .map(|item| item["event_id"].as_str().expect("event id").to_string())
        .collect()
}

/// The stable error code of an error response.
pub async fn error_code(response: Response<Body>) -> String {
    body_json(response).await["error"]
        .as_str()
        .expect("a stable error code")
        .to_string()
}

pub async fn body_json(response: Response<Body>) -> Value {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| json!({"raw": String::from_utf8_lossy(&bytes)}))
}
