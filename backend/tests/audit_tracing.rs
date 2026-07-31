//! Redaction canary for the request tracing surface.
//!
//! `tower_http`'s default HTTP span records the RAW request URI, query string
//! included, and a span's fields are attached to every event emitted inside it.
//! On this surface that would put OAuth authorization codes, CSRF state, and
//! presigned storage URLs into ordinary debug logs. The router therefore replaces
//! the default span; these tests prove the replacement holds at the most verbose
//! level anything is ever run at.

use std::io;
use std::sync::{Arc, Mutex, MutexGuard};

use axum::body::Body;
use axum::http::Request;
use fkst_control_plane::config::Config;
use fkst_control_plane::router::build_router;
use fkst_control_plane::state::{empty_self_router, AppState};
use tower::ServiceExt;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::fmt::MakeWriter;

/// An in-memory `tracing` writer.
#[derive(Clone, Default)]
struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

impl CapturedLogs {
    fn lock(&self) -> MutexGuard<'_, Vec<u8>> {
        self.0.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.lock()).into_owned()
    }
}

impl io::Write for CapturedLogs {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.lock().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for CapturedLogs {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

fn router() -> axum::Router {
    build_router(AppState {
        config: Config::default(),
        recovery: Default::default(),
        github_app: None,
        github_app_webhook_secret: Some(secrecy::SecretString::from(
            "trace-canary-secret".to_string(),
        )),
        reconciler: None,
        session_backend: None,
        storage: None,
        session_access: Default::default(),
        log_bundle_cache: Default::default(),
        disposable_environments: Default::default(),
        self_router: empty_self_router(),
        chat: None,
        audit: Default::default(),
    })
    .expect("router builds")
}

/// Drive `requests` through the real router with everything captured at TRACE.
async fn trace_requests(requests: Vec<Request<Body>>) -> String {
    let logs = CapturedLogs::default();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(LevelFilter::TRACE)
        .with_ansi(false)
        .with_target(true)
        .with_writer(logs.clone())
        .finish();
    // A thread-local default: `#[tokio::test]` uses the current-thread runtime,
    // so every poll of the router future happens on this thread and is covered.
    let guard = tracing::subscriber::set_default(subscriber);
    let router = router();
    for request in requests {
        let _ = router
            .clone()
            .oneshot(request)
            .await
            .expect("router responds");
    }
    drop(guard);
    logs.text()
}

fn get(uri: &str) -> Request<Body> {
    Request::get(uri)
        .body(Body::empty())
        .expect("request builds")
}

#[tokio::test]
async fn oauth_material_bearer_tokens_and_presigned_urls_never_reach_the_logs() {
    let captured = trace_requests(vec![
        // The browser OAuth callback: authorization code + CSRF state in the query.
        get("/api/v1/auth/github/callback?code=canary-oauth-code&state=canary-oauth-state"),
        // A bearer token on a token-authenticated route.
        Request::get("/api/v1/users/me/environment-profiles")
            .header("authorization", "Bearer canary-bearer-token")
            .body(Body::empty())
            .expect("request builds"),
        // A presigned-URL-shaped query value.
        get("/api/v1/logs/sess-canary?url=https://storage.example/o/b%3FX-Goog-Signature%3Dcanary-signature"),
        // An UNROUTED path: its raw value is exactly what must not be retained.
        get("/api/v1/canary-unrouted-path?token=canary-query-token"),
    ])
    .await;

    assert!(
        !captured.is_empty(),
        "the capture harness recorded nothing, so it proves nothing"
    );
    for canary in [
        "canary-oauth-code",
        "canary-oauth-state",
        "canary-bearer-token",
        "canary-signature",
        "canary-query-token",
        "canary-unrouted-path",
        "X-Goog-Signature",
        // The raw query separator would only appear if a URI were logged.
        "?code=",
    ] {
        assert!(
            !captured.contains(canary),
            "{canary} leaked into tracing output:\n{captured}"
        );
    }
}

/// The safe fields must actually be there — a span that carried nothing would
/// pass the canary test above for the wrong reason.
#[tokio::test]
async fn the_safe_correlation_fields_are_still_emitted() {
    let captured = trace_requests(vec![Request::get("/api/v1/overview")
        .header("x-request-id", "trace-canary-request-id")
        .body(Body::empty())
        .expect("request builds")])
    .await;
    assert!(
        captured.contains("trace-canary-request-id"),
        "the normalized request id must be correlatable:\n{captured}"
    );
    assert!(
        captured.contains("canvas_overview"),
        "the resolved operation id must be logged:\n{captured}"
    );
    assert!(
        captured.contains("/api/v1/overview"),
        "the normalized route template must be logged:\n{captured}"
    );
}
