//! The shared fixture for the router-level audit suites.
//!
//! Every test here drives the REAL [`build_router`], because the property under
//! test is the middleware's *position* relative to every inner layer — CORS, the
//! route-scoped timeouts, the leader-readiness gate, the identity extractors,
//! `AppError` conversion, and axum's own routing answers. A purpose-built router
//! could only prove the middleware works when it is already outermost; this one
//! proves it *is*.
//!
//! Split into a shared module (rather than duplicated) so the two suites that
//! use it — request lifecycle and identity attribution — cannot drift into
//! testing subtly different routers.

// Each test binary compiles this module in full but uses a different subset of
// the constructors (only the identity suite needs GitHub/browser-login states),
// which would otherwise read as dead code in the other binary.
#![allow(dead_code)]

use axum::body::Body;
use axum::http::Request;
use fkst_control_plane::audit::sink::RecordingSink;
use fkst_control_plane::audit::{ApiRequestCompletedV1, AuditHandle};
use fkst_control_plane::config::Config;
use fkst_control_plane::recovery::RecoveryMonitor;
use fkst_control_plane::router::build_router;
use fkst_control_plane::state::{empty_self_router, AppState};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use tower::ServiceExt;

/// A signature that cannot verify against any secret.
pub const BOGUS_SIGNATURE: &str =
    "sha256=00000000000000000000000000000000000000000000000000000000000000ff";

/// The webhook secret every harness is built with.
pub const WEBHOOK_SECRET: &str = "audit-test-secret";

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

pub struct Harness {
    router: axum::Router,
    pub sink: RecordingSink,
}

impl Harness {
    pub fn new() -> Self {
        Self::build(Config::default(), RecoveryMonitor::new(false), true)
    }

    /// An election-enabled replica that has NOT completed its acquisition resync,
    /// so every gated route short-circuits before any handler.
    pub fn follower() -> Self {
        let mut config = Config::default();
        config.leader.enabled = true;
        config.leader.identity = Some("pod-follower".to_string());
        let recovery = RecoveryMonitor::new(true);
        recovery.enable_leader_election("pod-follower".to_string());
        Self::build(config, recovery, true)
    }

    /// A deployment whose token identity checks hit `github_api_base`, admitting
    /// only the logins in `allowlist`. Built through the real `from_vars` parse
    /// so the access gate under test is the one an operator's env produces.
    pub fn with_github(github_api_base: &str, allowlist: &str) -> Self {
        let config = Config::from_vars([
            (
                "FKST_GITHUB_API_BASE_URL".to_string(),
                github_api_base.to_string(),
            ),
            (
                "FKST_ACCESS_ALLOWED_USERS".to_string(),
                allowlist.to_string(),
            ),
        ])
        .expect("the test configuration parses");
        Self::build(config, RecoveryMonitor::new(false), true)
    }

    /// A deployment with browser login configured, so the browser OAuth surface
    /// renders its HTML pages instead of the "not configured" 503.
    pub fn with_browser_login() -> Self {
        let mut config = Config::default();
        config.log.oauth_client_id = Some("Iv1.audit-test".to_string());
        config.log.oauth_client_secret = Some(secrecy::SecretString::from("oauth-secret"));
        config.log.public_base_url = Some("https://api.example.test".to_string());
        Self::build(config, RecoveryMonitor::new(false), true)
    }

    fn build(config: Config, recovery: RecoveryMonitor, webhook: bool) -> Self {
        let (audit, sink) = AuditHandle::recording();
        let router = build_router(AppState {
            config,
            recovery,
            github_app: None,
            github_app_webhook_secret: webhook
                .then(|| secrecy::SecretString::from(WEBHOOK_SECRET.to_string())),
            reconciler: None,
            session_backend: None,
            storage: None,
            session_access: Default::default(),
            operations: Default::default(),
            log_bundle_cache: Default::default(),
            disposable_environments: Default::default(),
            self_router: empty_self_router(),
            chat: None,
            audit,
        })
        .expect("router builds");
        Self { router, sink }
    }

    pub async fn call(&self, request: Request<Body>) -> axum::response::Response {
        self.router
            .clone()
            .oneshot(request)
            .await
            .expect("router responds")
    }

    pub async fn get(&self, path: &str) -> axum::response::Response {
        self.call(
            Request::get(path)
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
    }

    pub async fn head(&self, path: &str) -> axum::response::Response {
        self.call(
            Request::head(path)
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
    }

    /// The single terminal record the request under test must have produced.
    pub fn only_event(&self) -> ApiRequestCompletedV1 {
        let events = self.sink.events();
        assert_eq!(
            events.len(),
            1,
            "exactly one terminal record per request, got {events:#?}"
        );
        events.into_iter().next().expect("one event")
    }
}
