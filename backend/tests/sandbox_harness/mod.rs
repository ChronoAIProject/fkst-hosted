//! Shared harness for the `/api/v1/operations/sandboxes` integration tests.
//!
//! It drives the REAL `build_router` — extractors, GitHub identity verification,
//! the access policy, the scope gate, the session-visibility registry, the
//! handler, the audit middleware, and `AppError` conversion all run — against a
//! wiremock GitHub `/user` endpoint and a scripted runtime backend.
//!
//! The scripted backend is what makes the strongest claims testable: it counts
//! inventory reads (so "exactly one list per request" is asserted, not assumed)
//! and it counts every OTHER verb (so a per-runtime status read or a log/exec call
//! would fail a test that never mentions them).
//!
//! ## The fixture matrix
//!
//! One session, seven identities, each representing exactly one tier — or the
//! deliberate absence of one:
//!
//! | who | relationship | may see the session |
//! |---|---|---|
//! | Alice | effective creator | yes |
//! | Bob | `### Session Collaborators` | yes |
//! | Carol | `### FKST Contributors` log grant | yes |
//! | Dana | deployment `FKST_LOG_ADMINS` | yes |
//! | Erin | no relationship at all | no |
//! | Frank | repository owner (`acme`) | **no** — repository role is not a tier |
//! | Grace | deployment `FKST_GLOBAL_ADMINS` | yes, and everything else |

#![allow(dead_code)]

/// The scripted runtime backend.
pub mod backend;
/// The runtime fixtures every test's fleet is built from.
pub mod fleet;
/// An in-memory `tracing` sink, for the log half of the canary sweep.
pub mod logs;

pub use backend::{InventoryScript, ScriptedBackend};

use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, Response, StatusCode};
use fkst_control_plane::audit::sink::RecordingSink;
use fkst_control_plane::audit::AuditHandle;
use fkst_control_plane::config::Config;
use fkst_control_plane::models::RepoRef;
use fkst_control_plane::operations::{
    ActivitySource, ActivitySourceKind, OperationsState, SourceError, SourcePage, SourceQuery,
};
use fkst_control_plane::reconcile::creator::SessionCreator;
use fkst_control_plane::router::build_router;
use fkst_control_plane::session_access::{
    SessionAccessContext, SessionAccessRegistry, SessionAccessState,
};
use fkst_control_plane::state::{empty_self_router, AppState};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A stand-in historical-activity source.
///
/// It exists so the two operations surfaces can be failed INDEPENDENTLY. Proving
/// "a runtime outage does not change the activity answer" needs an activity
/// source that can actually succeed; with no source configured at all, both
/// directions collapse into the same `503 audit_query_not_configured`, and the
/// property is then asserted only in the degenerate case.
#[derive(Debug)]
pub struct ScriptedActivitySource {
    healthy: bool,
}

#[async_trait]
impl ActivitySource for ScriptedActivitySource {
    fn kind(&self) -> ActivitySourceKind {
        ActivitySourceKind::Posthog
    }

    async fn fetch(&self, _query: &SourceQuery) -> Result<SourcePage, SourceError> {
        if self.healthy {
            Ok(SourcePage::default())
        } else {
            Err(SourceError::Transient { kind: "timeout" })
        }
    }
}

/// The session Alice created.
pub const SESSION: &str = "sess-alice";
/// A second session, created by somebody outside this fixture entirely.
pub const OTHER_SESSION: &str = "sess-stranger";
/// A session id no registry generation has ever contained.
pub const UNKNOWN_SESSION: &str = "sess-nowhere";

pub const ALICE: (i64, &str) = (101, "alice");
pub const BOB: (i64, &str) = (102, "bob");
pub const CAROL: (i64, &str) = (103, "carol");
pub const DANA: (i64, &str) = (104, "dana");
pub const ERIN: (i64, &str) = (105, "erin");
/// The repository OWNER's login. Repository role is deliberately not a session
/// tier, so this identity must be refused exactly like an unrelated stranger.
pub const FRANK: (i64, &str) = (106, "acme");
pub const GRACE: (i64, &str) = (900, "grace");

/// The bearer token each fixture identity presents.
pub fn token(who: (i64, &str)) -> String {
    format!("token-{}", who.1)
}

/// The assembled harness.
pub struct Harness {
    pub router: axum::Router,
    /// Kept alive for the lifetime of the test; dropping it stops the server.
    _github: MockServer,
    pub backend: Option<Arc<ScriptedBackend>>,
    /// Every audit record the outer middleware produced, so a test can assert
    /// what a request — including a REFUSED one — actually recorded.
    pub audit: RecordingSink,
    /// The operations telemetry, for the closed-label metric assertions.
    pub operations: OperationsState,
}

/// How one harness is wired.
pub struct HarnessSpec {
    /// `None` builds a deployment with no runtime backend at all.
    pub script: Option<InventoryScript>,
    /// Whether the session-visibility projection has published a generation.
    pub registry_ready: bool,
    /// Whether the backend presents as OpenSandbox rather than Kubernetes.
    pub opensandbox: bool,
    /// `FKST_OPERATIONS_SANDBOX_MAX_RESULT_ITEMS`.
    pub max_result_items: usize,
    /// `FKST_OPERATIONS_SANDBOX_TIMEOUT_MS`.
    pub timeout_ms: u64,
    /// The historical-activity source, if any. `None` is the deployment that
    /// configured no read credentials; `Some(healthy)` scripts a source that
    /// answers or fails on its own, independently of the runtime backend.
    pub activity: Option<bool>,
}

impl HarnessSpec {
    /// A healthy Kubernetes deployment with a ready projection.
    pub fn new(script: InventoryScript) -> Self {
        Self {
            script: Some(script),
            registry_ready: true,
            opensandbox: false,
            max_result_items: 5_000,
            timeout_ms: 2_000,
            activity: None,
        }
    }

    /// A deployment with no runtime backend configured.
    pub fn without_backend() -> Self {
        Self {
            script: None,
            registry_ready: true,
            opensandbox: false,
            max_result_items: 5_000,
            timeout_ms: 2_000,
            activity: None,
        }
    }

    /// Configure the historical-activity source: healthy, or failing on its own.
    pub fn activity(mut self, healthy: bool) -> Self {
        self.activity = Some(healthy);
        self
    }

    pub fn cold_registry(mut self) -> Self {
        self.registry_ready = false;
        self
    }

    pub fn opensandbox(mut self) -> Self {
        self.opensandbox = true;
        self
    }

    pub fn max_result_items(mut self, items: usize) -> Self {
        self.max_result_items = items;
        self
    }

    pub fn timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }
}

/// Build a harness with a healthy Kubernetes backend holding `items`.
pub async fn harness_with(items: Vec<fleet::Item>) -> Harness {
    harness(HarnessSpec::new(fleet::snapshot(items))).await
}

/// Build a harness from an explicit specification.
pub async fn harness(spec: HarnessSpec) -> Harness {
    let github = MockServer::start().await;
    for who in [ALICE, BOB, CAROL, DANA, ERIN, FRANK, GRACE] {
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

    // Built through the real `from_vars` rather than by mutating fields: the
    // access policy and the ceilings are exactly what an operator's environment
    // would produce.
    let config = Config::from_vars([
        ("FKST_GLOBAL_ADMINS".to_string(), GRACE.1.to_string()),
        // The legacy cross-session observability grant, so the route-level
        // matrix exercises that tier rather than only the pure policy's.
        ("FKST_LOG_ADMINS".to_string(), DANA.1.to_string()),
        (
            "FKST_OPERATIONS_SANDBOX_MAX_RESULT_ITEMS".to_string(),
            spec.max_result_items.to_string(),
        ),
        (
            "FKST_OPERATIONS_SANDBOX_TIMEOUT_MS".to_string(),
            spec.timeout_ms.to_string(),
        ),
    ])
    .expect("the fixture configuration parses");
    let config = Config {
        github_api_base_url: github.uri(),
        ..config
    };

    let backend = spec.script.map(|script| {
        if spec.opensandbox {
            ScriptedBackend::opensandbox(script)
        } else {
            ScriptedBackend::new(script)
        }
    });
    let operations = match spec.activity {
        None => OperationsState::default(),
        Some(healthy) => {
            OperationsState::with_sources(Some(Arc::new(ScriptedActivitySource { healthy })), None)
        }
    };
    let (audit, sink) = AuditHandle::recording();
    let router = build_router(AppState {
        config,
        recovery: Default::default(),
        github_app: None,
        github_app_webhook_secret: None,
        reconciler: None,
        session_backend: backend
            .clone()
            .map(|backend| backend as Arc<dyn fkst_control_plane::session_backend::SessionBackend>),
        storage: None,
        session_access: SessionAccessState::new(registry(spec.registry_ready)),
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
        backend,
        audit: sink,
        operations,
    }
}

/// The visibility projection: Alice created `SESSION`, Bob collaborates, Carol
/// holds a per-session log grant. `OTHER_SESSION` belongs to a stranger, so it is
/// KNOWN to the registry and authorized for nobody in this fixture — which is what
/// separates "unknown session" from "not yours" internally while keeping the two
/// indistinguishable on the wire.
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
            vec![
                (
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
                        log_access: vec![CAROL.1.to_string()],
                    },
                ),
                (
                    OTHER_SESSION.to_string(),
                    SessionAccessContext {
                        installation_id: 1,
                        repo: repo.clone(),
                        trigger_issue: 9,
                        creator: SessionCreator {
                            login: "stranger".to_string(),
                            id: Some(707),
                        },
                        collaborators: Vec::new(),
                        log_access: Vec::new(),
                    },
                ),
            ],
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
            Request::get(format!("/api/v1/operations/sandboxes{query}")).header("host", "test");
        if let Some(who) = who {
            builder = builder.header("authorization", format!("Bearer {}", token(who)));
        }
        self.router
            .clone()
            .oneshot(builder.body(Body::empty()).expect("request builds"))
            .await
            .expect("router responds")
    }

    /// Issue a request expected to succeed, returning the parsed snapshot.
    pub async fn snapshot(&self, who: (i64, &str), query: &str) -> Value {
        let response = self.get(who, query).await;
        let status = response.status();
        let body = body_json(response).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    /// The exact response BYTES of a successful request — the strongest form of
    /// "a hidden row changed nothing".
    pub async fn snapshot_bytes(&self, who: (i64, &str), query: &str) -> Vec<u8> {
        let response = self.get(who, query).await;
        assert_eq!(response.status(), StatusCode::OK);
        response
            .into_body()
            .collect()
            .await
            .expect("collect body")
            .to_bytes()
            .to_vec()
    }

    /// How many inventory reads the scripted backend received.
    pub fn inventory_calls(&self) -> usize {
        self.backend
            .as_ref()
            .map(|backend| backend.inventory_calls())
            .unwrap_or_default()
    }

    /// How many forbidden verbs the scripted backend received.
    pub fn forbidden_calls(&self) -> usize {
        self.backend
            .as_ref()
            .map(|backend| backend.forbidden_calls())
            .unwrap_or_default()
    }
}

/// The runtime ids of a snapshot's items, in order.
pub fn item_ids(snapshot: &Value) -> Vec<String> {
    snapshot["items"]
        .as_array()
        .expect("items array")
        .iter()
        .map(|item| item["runtime_id"].as_str().expect("runtime id").to_string())
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
